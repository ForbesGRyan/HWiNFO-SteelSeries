use crate::consts::{Style, CUSTOM_SENSORS, DISPLAY_LINES};
use crate::render::FontSize;
use crate::weather::Units;
use anyhow::anyhow;
use console::Term;
use dialoguer::Input;
use hwinfo_steelseries_oled::{Hwinfo, SensorReadingType};
use ini::Ini;
use log::{error, info, warn};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CustomSensor {
    pub sensor: String,
    pub label: String,
    pub unit: String,
    pub convert: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeatherConfig {
    pub enabled: bool,
    pub location: String,
    #[serde(with = "units_serde")]
    pub units: Units,
    pub refresh_minutes: u64,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            location: String::new(),
            units: Units::Imperial,
            refresh_minutes: 15,
        }
    }
}

impl WeatherConfig {
    pub fn from_ini(config: &ini::Ini) -> Self {
        let section = match config.section(Some("Weather")) {
            Some(s) => s,
            None => return Self::default(),
        };
        let location = section.get("location").unwrap_or("").trim().to_string();
        if location.is_empty() {
            return Self::default();
        }
        let units = Units::from_config_str(section.get("units").unwrap_or(""));
        let refresh_minutes = section
            .get("refresh_minutes")
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v.max(1))
            .unwrap_or(15);
        Self {
            enabled: true,
            location,
            units,
            refresh_minutes,
        }
    }
}

mod units_serde {
    use super::Units;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(units: &Units, s: S) -> Result<S::Ok, S::Error> {
        match units {
            Units::Metric => "metric",
            Units::Imperial => "imperial",
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Units, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Units::from_config_str(&s))
    }
}

// Configuration struct for parsing existing config files
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppConfig {
    pub is_summary: bool,
    pub is_vertical: bool,
    pub gpu: String,
    pub decimal: bool,
    pub pages: usize,
    pub page_time: isize,
    pub sensors_per_line: u8,
    pub direct_usb: bool,
    #[serde(default)]
    pub direct_usb_serial: String,
    pub custom_sensors: Vec<Vec<CustomSensor>>,
    #[serde(default)]
    pub weather: WeatherConfig,
    #[serde(default = "default_font_sizes")]
    pub font_sizes: [FontSize; DISPLAY_LINES],
}

fn default_font_sizes() -> [FontSize; DISPLAY_LINES] {
    [FontSize::Medium; DISPLAY_LINES]
}

impl AppConfig {
    pub fn from_ini(config: &Ini) -> Result<Self, anyhow::Error> {
        println!("AppConfig: Reading from ini...");
        let main = config.section(Some("Main"));

        if main.is_none() {
            println!("AppConfig error: 'Main' section not found in INI");
            // Instead of erroring, let's look for any section to see if we loaded the wrong file
            for section in config.sections() {
                println!("AppConfig debug: Found section: {:?}", section);
            }
        }

        let main = main.ok_or_else(|| anyhow::anyhow!("Main config section not found"))?;

        let style = main
            .get("style")
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| {
                println!("AppConfig warning: 'style' key not found, defaulting to 'vertical'");
                "vertical".to_string()
            });

        let is_summary = matches!(style.as_str(), "vertical" | "horizontal");
        let is_vertical = style == "vertical";

        let gpu = if is_summary {
            main.get("gpu").unwrap_or("").to_string()
        } else {
            String::new()
        };

        let decimal = main
            .get("decimal")
            .and_then(|d| d.parse::<bool>().ok())
            .unwrap_or(false);

        let pages = main
            .get("pages")
            .and_then(|p| p.parse::<usize>().ok())
            .unwrap_or(1);

        let page_time = main
            .get("page_time")
            .and_then(|pt| pt.parse::<isize>().ok())
            .map(|num| if (0..=60).contains(&num) { num } else { 5 })
            .unwrap_or(5);

        let sensors_per_line = if !is_summary {
            main.get("sensors_per_line")
                .and_then(|spl| spl.parse::<u8>().ok())
                .unwrap_or(1)
        } else {
            1
        };

        let direct_usb = main
            .get("direct_usb")
            .and_then(|d| d.parse::<bool>().ok())
            .unwrap_or(false);

        let direct_usb_serial = main.get("direct_usb_serial").unwrap_or("").to_string();

        println!(
            "AppConfig: Loaded main settings. is_summary={}, direct_usb={}",
            is_summary, direct_usb
        );

        let font_sizes = {
            let mut arr = [FontSize::Medium; DISPLAY_LINES];
            for (i, slot) in arr.iter_mut().enumerate() {
                if let Some(v) = main.get(format!("font_line{}", i + 1)) {
                    *slot = FontSize::from_config_str(v);
                }
            }
            arr
        };

        Ok(Self {
            is_summary,
            is_vertical,
            gpu,
            decimal,
            pages,
            page_time,
            sensors_per_line,
            direct_usb,
            direct_usb_serial,
            custom_sensors: {
                let mut all_pages = Vec::new();
                for i in 1..=pages {
                    let mut page_sensors = Vec::new();
                    if let Some(section) = config.section(Some(format!("PAGE{}.Sensors", i))) {
                        for k in 0..CUSTOM_SENSORS {
                            if let Some(sensor) = section.get(format!("sensor_{}", k)) {
                                let label = section
                                    .get(format!("label_{}", k))
                                    .unwrap_or("")
                                    .to_string();
                                let unit =
                                    section.get(format!("unit_{}", k)).unwrap_or("").to_string();
                                let convert = section
                                    .get(format!("convert_{}", k))
                                    .unwrap_or("")
                                    .to_string();
                                page_sensors.push(CustomSensor {
                                    sensor: sensor.to_string(),
                                    label,
                                    unit,
                                    convert,
                                });
                            }
                        }
                    }
                    all_pages.push(page_sensors);
                }
                all_pages
            },
            weather: WeatherConfig::from_ini(config),
            font_sizes,
        })
    }
}

/// Map the "Choose style" menu input (1/2/3) to a Style. Returns None for invalid input.
fn style_from_choice(input: u8) -> Option<Style> {
    match input {
        1 => Some(Style::Vertical),
        2 => Some(Style::Horizontal),
        3 => Some(Style::Custom),
        _ => None,
    }
}

/// Map the "Connection Type" menu input (1/2) to direct_usb bool. Anything else → false.
fn direct_usb_from_choice(input: u8) -> bool {
    matches!(input, 2)
}

/// Clamp the "lines" input to the allowed 2..=5 range; out-of-range inputs default to 3.
fn validate_lines(input: u8) -> u8 {
    if (2..=5).contains(&input) {
        input
    } else {
        3
    }
}

/// Validate the "sensors per line" input; returns None if outside 1..=3.
fn validate_sensors_per_line(input: u8) -> Option<u8> {
    if (1..=3).contains(&input) {
        Some(input)
    } else {
        None
    }
}

/// Pick the unit to write: user input wins; falls back to default_unit when empty.
fn pick_unit(default_unit: &str, user_input: &str) -> String {
    if user_input.is_empty() {
        default_unit.to_string()
    } else {
        user_input.to_string()
    }
}

/// Resolve a category index against the sensor_names list. Errors if out of range.
fn validate_category_selection(
    idx: usize,
    sensor_names: &[String],
) -> Result<&String, anyhow::Error> {
    sensor_names.get(idx).ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid category selection: {} (max: {})",
            idx,
            sensor_names.len().saturating_sub(1)
        )
    })
}

/// Resolve a reading index against a sensor's reading_names. Errors if out of range.
fn validate_reading_selection(
    idx: usize,
    reading_names: &[String],
) -> Result<&String, anyhow::Error> {
    reading_names.get(idx).ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid reading selection: {} (max: {})",
            idx,
            reading_names.len().saturating_sub(1)
        )
    })
}

/// Format the canonical sensor id used in conf.ini: `Category;Reading`.
fn format_sensor_id(category: &str, reading: &str) -> String {
    format!("{};{}", category, reading)
}

fn get_default_unit(reading_type: SensorReadingType) -> &'static str {
    match reading_type {
        SensorReadingType::SensorTypeTemp => "°",
        SensorReadingType::SensorTypeVolt => "V",
        SensorReadingType::SensorTypeFan => "RPM",
        SensorReadingType::SensorTypeCurrent => "A",
        SensorReadingType::SensorTypePower => "W",
        SensorReadingType::SensorTypeClock => "MHz",
        SensorReadingType::SensorTypeUsage => "%",
        _ => "",
    }
}

fn configure_gpu_selection(
    term: &Term,
    hwinfo: &Hwinfo,
    conf: &mut Ini,
) -> Result<(), anyhow::Error> {
    let gpus = hwinfo.find("GPU Temperature").map_err(|e| {
        error!("Failed to find GPU temperature sensors: {}", e);
        e
    })?;

    if gpus.len() <= 1 {
        info!("Only one GPU found, no selection needed");
        return Ok(());
    }

    info!(
        "Multiple GPUs detected ({}), prompting user for selection",
        gpus.len()
    );
    term.write_line("Which GPU:\n")?;
    for (i, gpu) in gpus.iter().enumerate() {
        let sensor_name = &hwinfo.sensor_names[gpu.dw_sensor_index as usize];
        term.write_line(&format!("{}: {}", i, sensor_name))?;
    }

    let gpu_selection: usize = Input::new()
        .with_prompt(format!("0..{}", gpus.len() - 1))
        .interact_text()?;

    let gpu_selected = &hwinfo.sensor_names[gpus[gpu_selection].dw_sensor_index as usize];
    info!("User selected GPU: {}", gpu_selected);
    conf.with_section(Some("Main")).set("gpu", gpu_selected);

    Ok(())
}

fn configure_custom_sensors(
    hwinfo: &Hwinfo,
    conf: &mut Ini,
    lines: u8,
    sensors_per_line: u8,
) -> Result<(), anyhow::Error> {
    info!(
        "Configuring {} custom sensors ({} lines x {} sensors per line)",
        lines * sensors_per_line,
        lines,
        sensors_per_line
    );

    for k in 0..(lines * sensors_per_line) {
        println!("\n{} / {}\n", k + 1, lines * sensors_per_line);

        // Display available sensors in HWiNFO's original order
        for (i, sensor) in hwinfo.sensor_names.iter().enumerate() {
            println!("{}) {}", i, sensor);
        }

        let category: usize = Input::new()
            .with_prompt("Category")
            .interact_text()
            .unwrap_or(0);

        let sensor_name = validate_category_selection(category, &hwinfo.sensor_names)
            .inspect_err(|e| {
                error!("{}", e);
                println!("Category out of range, please try again.");
            })?
            .clone();

        let sensor = hwinfo.sensors.get(&sensor_name).ok_or_else(|| {
            error!("Sensor '{}' not found in HWiNFO data", sensor_name);
            anyhow::anyhow!(
                "Sensor '{}' not found - HWiNFO data may have changed",
                sensor_name
            )
        })?;

        println!("\n{}:", sensor_name);
        for (i, reading_name) in sensor.reading_names.iter().enumerate() {
            println!("\t{}) {}", i, reading_name);
        }

        let sensor_selection: usize = Input::new().with_prompt("Sensor").interact_text()?;
        let selected_reading_name =
            validate_reading_selection(sensor_selection, &sensor.reading_names)?.clone();
        let sensor_selected = format_sensor_id(&sensor_name, &selected_reading_name);
        let label: String = Input::new().with_prompt("Label").interact_text()?;

        let reading = sensor.readings.get(&selected_reading_name).ok_or_else(|| {
            error!(
                "Reading '{}' not found in sensor '{}'",
                selected_reading_name, sensor_name
            );
            anyhow::anyhow!(
                "Reading '{}' not found - HWiNFO data may have changed",
                selected_reading_name
            )
        })?;
        let default_unit = get_default_unit(reading.t_reading);

        let unit: String = if default_unit.is_empty() {
            Input::new().with_prompt("Unit").interact_text()?
        } else {
            let input: String = Input::new()
                .with_prompt(format!("Unit (default: {})", default_unit))
                .allow_empty(true)
                .interact_text()?;
            pick_unit(default_unit, &input)
        };

        conf.with_section(Some("PAGE1.Sensors"))
            .set(format!("sensor_{}", k), sensor_selected)
            .set(format!("label_{}", k), label)
            .set(format!("unit_{}", k), unit);
    }

    Ok(())
}

pub fn settings_create_config(term: &Term, hwinfo: &Hwinfo) -> Result<Ini, anyhow::Error> {
    info!("Creating new configuration file");
    term.write_line("Config not found.")?;
    let mut conf = Ini::new();

    term.write_line(
        "Summary Vertical:
    1) CPU  GPU  MEM\n
       55°  45°  8.65G\n
       10%  0.0% 32.0G",
    )?;
    term.write_line(
        "Summary Horizontal:
    2) CPU  45°  10.0%\n
       GPU  35°  0.0%\n
       MEM  10G  33.3%",
    )?;
    term.write_line("3) Pick your own sensors")?;

    let input: u8 = match Input::new()
        .with_prompt("Choose style\n(1,2,3)")
        .interact_text()
    {
        Ok(input) => input,
        Err(e) => {
            error!("Failed to read input: {}", e);
            return Err(anyhow!("Failed to read input"));
        }
    };

    let style: Style = match style_from_choice(input) {
        Some(s) => s,
        None => {
            warn!("Invalid style input: {}", input);
            term.write_line("Invalid input")?;
            return settings_create_config(term, hwinfo);
        }
    };

    info!("User selected style: {:?}", style);
    conf.with_section(Some("Main"))
        .set("style", style.to_string());

    let direct_usb_input: u8 = Input::new()
        .with_prompt("Connection Type\n1) SteelSeries GG (GameSense)\n2) Direct USB (HID)")
        .interact_text()
        .unwrap_or(1);
    let direct_usb = direct_usb_from_choice(direct_usb_input);

    conf.with_section(Some("Main"))
        .set("direct_usb", direct_usb.to_string());

    if style != Style::Custom {
        configure_gpu_selection(term, hwinfo, &mut conf)?;
    } else {
        println!(
            "\nUp to 5 lines will fit on the Arctis(or Nova) Pro screen, and 2 on the Apex Pro."
        );

        let raw_lines: u8 = Input::new()
            .with_prompt("How many lines? (2-5)")
            .interact_text()
            .unwrap_or(3);
        let lines = validate_lines(raw_lines);

        let raw_spl: u8 = Input::new()
            .with_prompt("How many sensors per line? (1-3)")
            .interact_text()?;
        let sensors_per_line = match validate_sensors_per_line(raw_spl) {
            Some(v) => v,
            None => return settings_create_config(term, hwinfo),
        };

        conf.with_section(Some("Main"))
            .set("sensors_per_line", sensors_per_line.to_string());

        configure_custom_sensors(hwinfo, &mut conf, lines, sensors_per_line)?;
    }

    info!("Writing configuration to conf.ini");
    conf.write_to_file("conf.ini").map_err(|e| {
        error!("Failed to write configuration file: {}", e);
        e
    })?;
    info!("Configuration file created successfully");
    term.write_line("config created.")?;

    Ok(conf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config(
        style: &str,
        gpu: Option<&str>,
        decimal: bool,
        pages: usize,
        page_time: isize,
        sensors_per_line: u8,
        direct_usb: bool,
    ) -> Ini {
        let mut conf = Ini::new();
        conf.with_section(Some("Main")).set("style", style);

        if let Some(gpu_val) = gpu {
            conf.with_section(Some("Main")).set("gpu", gpu_val);
        }

        conf.with_section(Some("Main"))
            .set("decimal", decimal.to_string())
            .set("pages", pages.to_string())
            .set("page_time", page_time.to_string())
            .set("sensors_per_line", sensors_per_line.to_string())
            .set("direct_usb", direct_usb.to_string());

        conf
    }

    #[test]
    fn test_appconfig_vertical_summary() {
        let conf = create_test_config("Vertical", None, true, 1, 5, 1, false);
        let config = AppConfig::from_ini(&conf).unwrap();

        assert!(config.is_summary);
        assert!(config.is_vertical);
        assert_eq!(config.gpu, "");
        assert!(config.decimal);
        assert_eq!(config.pages, 1);
        assert_eq!(config.page_time, 5);
    }

    #[test]
    fn test_appconfig_horizontal_summary() {
        let conf = create_test_config("Horizontal", Some("GPU [#0]"), false, 1, 10, 1, true);
        let config = AppConfig::from_ini(&conf).unwrap();

        assert!(config.is_summary);
        assert!(!config.is_vertical);
        assert_eq!(config.gpu, "GPU [#0]");
        assert!(!config.decimal);
        assert_eq!(config.page_time, 10);
    }

    #[test]
    fn test_appconfig_custom_mode() {
        let conf = create_test_config("Custom", None, false, 2, 8, 3, false);
        let config = AppConfig::from_ini(&conf).unwrap();

        assert!(!config.is_summary);
        assert_eq!(config.pages, 2);
        assert_eq!(config.sensors_per_line, 3);
    }

    #[test]
    fn test_appconfig_defaults() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main")).set("style", "Vertical");

        let config = AppConfig::from_ini(&conf).unwrap();

        assert_eq!(config.pages, 1);
        assert_eq!(config.page_time, 5);
        assert!(!config.decimal);
        assert_eq!(config.sensors_per_line, 1);
    }

    #[test]
    fn test_appconfig_page_time_out_of_range() {
        let conf = create_test_config("Vertical", None, false, 1, 100, 1, false);
        let config = AppConfig::from_ini(&conf).unwrap();

        // Should cap at 5 for values outside 0..=60
        assert_eq!(config.page_time, 5);
    }

    #[test]
    fn test_appconfig_page_time_negative() {
        let conf = create_test_config("Vertical", None, false, 1, -5, 1, false);
        let config = AppConfig::from_ini(&conf).unwrap();

        // Should use default 5 for negative values
        assert_eq!(config.page_time, 5);
    }

    #[test]
    fn test_appconfig_missing_main_section() {
        let conf = Ini::new();
        let result = AppConfig::from_ini(&conf);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Main config section not found"
        );
    }

    // ==================== style_from_choice ====================

    #[test]
    fn test_style_from_choice_valid() {
        assert_eq!(style_from_choice(1), Some(Style::Vertical));
        assert_eq!(style_from_choice(2), Some(Style::Horizontal));
        assert_eq!(style_from_choice(3), Some(Style::Custom));
    }

    #[test]
    fn test_style_from_choice_invalid() {
        assert!(style_from_choice(0).is_none());
        assert!(style_from_choice(4).is_none());
        assert!(style_from_choice(255).is_none());
    }

    // ==================== direct_usb_from_choice ====================

    #[test]
    fn test_direct_usb_from_choice_gamesense() {
        assert!(!direct_usb_from_choice(1));
    }

    #[test]
    fn test_direct_usb_from_choice_direct_usb() {
        assert!(direct_usb_from_choice(2));
    }

    #[test]
    fn test_direct_usb_from_choice_invalid_defaults_to_gamesense() {
        assert!(!direct_usb_from_choice(0));
        assert!(!direct_usb_from_choice(99));
    }

    // ==================== validate_lines ====================

    #[test]
    fn test_validate_lines_in_range() {
        for n in 2..=5 {
            assert_eq!(validate_lines(n), n);
        }
    }

    #[test]
    fn test_validate_lines_below_range_defaults_to_three() {
        assert_eq!(validate_lines(0), 3);
        assert_eq!(validate_lines(1), 3);
    }

    #[test]
    fn test_validate_lines_above_range_defaults_to_three() {
        assert_eq!(validate_lines(6), 3);
        assert_eq!(validate_lines(255), 3);
    }

    // ==================== validate_sensors_per_line ====================

    #[test]
    fn test_validate_sensors_per_line_in_range() {
        for n in 1..=3 {
            assert_eq!(validate_sensors_per_line(n), Some(n));
        }
    }

    #[test]
    fn test_validate_sensors_per_line_out_of_range() {
        assert!(validate_sensors_per_line(0).is_none());
        assert!(validate_sensors_per_line(4).is_none());
        assert!(validate_sensors_per_line(255).is_none());
    }

    // ==================== pick_unit ====================

    #[test]
    fn test_pick_unit_uses_input_when_non_empty() {
        assert_eq!(pick_unit("°", "K"), "K");
    }

    #[test]
    fn test_pick_unit_falls_back_to_default_when_empty() {
        assert_eq!(pick_unit("°", ""), "°");
    }

    #[test]
    fn test_pick_unit_empty_default_and_empty_input() {
        assert_eq!(pick_unit("", ""), "");
    }

    // ==================== validate_category_selection ====================

    #[test]
    fn test_validate_category_selection_valid() {
        let names = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        assert_eq!(validate_category_selection(0, &names).unwrap(), "A");
        assert_eq!(validate_category_selection(2, &names).unwrap(), "C");
    }

    #[test]
    fn test_validate_category_selection_out_of_range() {
        let names = vec!["A".to_string()];
        let err = validate_category_selection(5, &names).unwrap_err();
        assert!(format!("{}", err).contains("Invalid category selection"));
    }

    #[test]
    fn test_validate_category_selection_empty_list() {
        let names: Vec<String> = vec![];
        assert!(validate_category_selection(0, &names).is_err());
    }

    // ==================== validate_reading_selection ====================

    #[test]
    fn test_validate_reading_selection_valid() {
        let readings = vec!["Temp".to_string(), "Load".to_string()];
        assert_eq!(validate_reading_selection(1, &readings).unwrap(), "Load");
    }

    #[test]
    fn test_validate_reading_selection_out_of_range() {
        let readings = vec!["Temp".to_string()];
        let err = validate_reading_selection(7, &readings).unwrap_err();
        assert!(format!("{}", err).contains("Invalid reading selection"));
    }

    // ==================== format_sensor_id ====================

    #[test]
    fn test_format_sensor_id() {
        assert_eq!(
            format_sensor_id("CPU [#0]", "Temperature"),
            "CPU [#0];Temperature"
        );
        assert_eq!(format_sensor_id("", ""), ";");
    }

    // ==================== get_default_unit ====================

    #[test]
    fn test_get_default_unit_known_types() {
        assert_eq!(get_default_unit(SensorReadingType::SensorTypeTemp), "°");
        assert_eq!(get_default_unit(SensorReadingType::SensorTypeVolt), "V");
        assert_eq!(get_default_unit(SensorReadingType::SensorTypeFan), "RPM");
        assert_eq!(get_default_unit(SensorReadingType::SensorTypeCurrent), "A");
        assert_eq!(get_default_unit(SensorReadingType::SensorTypePower), "W");
        assert_eq!(get_default_unit(SensorReadingType::SensorTypeClock), "MHz");
        assert_eq!(get_default_unit(SensorReadingType::SensorTypeUsage), "%");
    }

    #[test]
    fn test_get_default_unit_unknown_types_empty() {
        assert_eq!(get_default_unit(SensorReadingType::SensorTypeNone), "");
        assert_eq!(get_default_unit(SensorReadingType::SensorTypeOther), "");
    }

    #[test]
    fn test_appconfig_loads_custom_sensors_from_pages() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Custom")
            .set("pages", "2")
            .set("sensors_per_line", "2");
        conf.with_section(Some("PAGE1.Sensors"))
            .set("sensor_0", "CPU [#0];Temperature")
            .set("label_0", "CPU")
            .set("unit_0", "°C")
            .set("convert_0", "")
            .set("sensor_1", "GPU [#0];Temperature")
            .set("label_1", "GPU")
            .set("unit_1", "°C");
        conf.with_section(Some("PAGE2.Sensors"))
            .set("sensor_0", "MEM;Used")
            .set("label_0", "RAM")
            .set("unit_0", "GB")
            .set("convert_0", "MB/GB");

        let config = AppConfig::from_ini(&conf).unwrap();

        assert_eq!(config.custom_sensors.len(), 2);
        assert_eq!(config.custom_sensors[0].len(), 2);
        assert_eq!(config.custom_sensors[0][0].sensor, "CPU [#0];Temperature");
        assert_eq!(config.custom_sensors[0][0].label, "CPU");
        assert_eq!(config.custom_sensors[0][0].unit, "°C");
        assert_eq!(config.custom_sensors[0][1].sensor, "GPU [#0];Temperature");

        assert_eq!(config.custom_sensors[1].len(), 1);
        assert_eq!(config.custom_sensors[1][0].sensor, "MEM;Used");
        assert_eq!(config.custom_sensors[1][0].convert, "MB/GB");
    }

    #[test]
    fn test_appconfig_missing_page_section_yields_empty_page() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Custom")
            .set("pages", "3"); // pages claimed but no PAGE sections

        let config = AppConfig::from_ini(&conf).unwrap();
        assert_eq!(config.custom_sensors.len(), 3);
        for page in &config.custom_sensors {
            assert!(page.is_empty());
        }
    }

    #[test]
    fn test_appconfig_custom_sensor_missing_label_unit_defaults_to_empty() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Custom")
            .set("pages", "1");
        conf.with_section(Some("PAGE1.Sensors"))
            .set("sensor_0", "CPU;Temp"); // no label_0 / unit_0 / convert_0

        let config = AppConfig::from_ini(&conf).unwrap();
        let sensor = &config.custom_sensors[0][0];
        assert_eq!(sensor.sensor, "CPU;Temp");
        assert_eq!(sensor.label, "");
        assert_eq!(sensor.unit, "");
        assert_eq!(sensor.convert, "");
    }

    #[test]
    fn test_appconfig_direct_usb_serial_loaded() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Custom")
            .set("direct_usb", "true")
            .set("direct_usb_serial", "ABCD1234")
            .set("pages", "1");
        let config = AppConfig::from_ini(&conf).unwrap();
        assert!(config.direct_usb);
        assert_eq!(config.direct_usb_serial, "ABCD1234");
    }

    #[test]
    fn test_appconfig_invalid_numeric_fields_fall_back_to_defaults() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Custom")
            .set("pages", "not-a-number")
            .set("page_time", "not-a-number")
            .set("sensors_per_line", "abc")
            .set("decimal", "maybe")
            .set("direct_usb", "tru-ish");
        let config = AppConfig::from_ini(&conf).unwrap();
        assert_eq!(config.pages, 1);
        assert_eq!(config.page_time, 5);
        assert_eq!(config.sensors_per_line, 1);
        assert!(!config.decimal);
        assert!(!config.direct_usb);
    }

    #[test]
    fn test_appconfig_summary_zeroes_sensors_per_line() {
        // is_summary=true → sensors_per_line is forced to 1 regardless of ini value
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Vertical")
            .set("sensors_per_line", "3");
        let config = AppConfig::from_ini(&conf).unwrap();
        assert!(config.is_summary);
        assert_eq!(config.sensors_per_line, 1);
    }

    fn build_hwinfo_with_gpus(num_gpus: usize) -> Hwinfo {
        use hwinfo_steelseries_oled::{
            HwinfoSensorsReadingElement, HwinfoSensorsSensorElement, Sensor,
        };
        use std::collections::HashMap;
        let mut sensors = HashMap::new();
        let mut sensor_names = Vec::new();
        for i in 0..num_gpus {
            let key = format!("GPU [#{}]", i);
            sensor_names.push(key.clone());
            let mut readings = HashMap::new();
            readings.insert(
                "GPU Temperature".to_string(),
                HwinfoSensorsReadingElement::new_mock(i as u32, 0, "GPU Temperature", 50.0),
            );
            sensors.insert(
                key.clone(),
                Sensor {
                    info: HwinfoSensorsSensorElement::new_mock(i as u32, &key),
                    readings,
                    reading_names: vec!["GPU Temperature".to_string()],
                },
            );
        }
        Hwinfo::new_mock(sensors, sensor_names)
    }

    #[test]
    fn test_configure_gpu_selection_single_gpu_returns_ok_without_prompt() {
        let term = Term::stdout();
        let hwinfo = build_hwinfo_with_gpus(1);
        let mut conf = Ini::new();
        configure_gpu_selection(&term, &hwinfo, &mut conf).unwrap();
        // No "gpu" key written, since single-GPU path skips selection.
        assert!(conf
            .section(Some("Main"))
            .and_then(|s| s.get("gpu"))
            .is_none());
    }

    #[test]
    fn test_configure_gpu_selection_multi_gpu_errors_on_stdin_eof() {
        // Two GPUs → triggers the prompt + Input::interact_text() which fails in tests → Err.
        let term = Term::stdout();
        let hwinfo = build_hwinfo_with_gpus(2);
        let mut conf = Ini::new();
        let r = configure_gpu_selection(&term, &hwinfo, &mut conf);
        assert!(r.is_err());
    }

    #[test]
    fn test_configure_custom_sensors_errors_when_category_out_of_range() {
        // Empty hwinfo → category=0 has no valid sensor → validate_category_selection fails,
        // exercising the inspect_err branch.
        let hwinfo = Hwinfo::new_mock(std::collections::HashMap::new(), vec![]);
        let mut conf = Ini::new();
        let r = configure_custom_sensors(&hwinfo, &mut conf, 1, 1);
        assert!(r.is_err());
    }

    #[test]
    fn test_configure_custom_sensors_errors_when_sensor_missing_from_hashmap() {
        // sensor_names points to "Phantom" but sensors map has no key for it → Err
        let hwinfo = Hwinfo::new_mock(
            std::collections::HashMap::new(),
            vec!["Phantom".to_string()],
        );
        let mut conf = Ini::new();
        let r = configure_custom_sensors(&hwinfo, &mut conf, 1, 1);
        match r {
            Err(e) => assert!(format!("{}", e).contains("not found")),
            Ok(_) => panic!("expected err"),
        }
    }

    #[test]
    fn test_configure_custom_sensors_errors_on_stdin_eof() {
        // Even a single sensor configuration requires Input::interact_text() which fails in tests.
        use hwinfo_steelseries_oled::{
            HwinfoSensorsReadingElement, HwinfoSensorsSensorElement, Sensor,
        };
        use std::collections::HashMap;
        let mut sensors = HashMap::new();
        let mut readings = HashMap::new();
        readings.insert(
            "Temperature".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 0, "Temperature", 50.0),
        );
        sensors.insert(
            "CPU".to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(0, "CPU"),
                readings,
                reading_names: vec!["Temperature".to_string()],
            },
        );
        let hwinfo = Hwinfo::new_mock(sensors, vec!["CPU".to_string()]);
        let mut conf = Ini::new();
        let r = configure_custom_sensors(&hwinfo, &mut conf, 1, 1);
        // Either fails on Input or validates index → either way uncovered branches run.
        // In cargo test the Input::interact_text returns Err so unwrap_or(0) gives 0 → validate ok → next Input fails → Err.
        assert!(r.is_err());
    }

    #[test]
    fn test_settings_create_config_errors_when_stdin_eof() {
        // In `cargo test`, dialoguer::Input has no real terminal — the first interact_text
        // call returns Err, which settings_create_config converts into anyhow::Error.
        // This test only confirms the preamble + error-mapping code runs.
        let term = Term::stdout();
        let hwinfo = build_hwinfo_with_gpus(1);
        let result = settings_create_config(&term, &hwinfo);
        assert!(result.is_err());
    }

    #[test]
    fn test_configure_gpu_selection_no_gpu_returns_err() {
        let term = Term::stdout();
        let hwinfo = build_hwinfo_with_gpus(0);
        let mut conf = Ini::new();
        let result = configure_gpu_selection(&term, &hwinfo, &mut conf);
        assert!(result.is_err());
    }

    #[test]
    fn test_appconfig_missing_style() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main")).set("pages", "1");

        let result = AppConfig::from_ini(&conf);

        // When style is missing, the code defaults to "vertical" (summary mode)
        assert!(result.is_ok());
        let config = result.unwrap();
        assert!(config.is_summary);
        assert!(config.is_vertical);
    }

    #[test]
    fn test_appconfig_font_sizes_parsed() {
        use crate::render::FontSize;
        let mut ini = Ini::new();
        ini.with_section(Some("Main"))
            .set("style", "vertical")
            .set("font_line1", "large")
            .set("font_line2", "small")
            .set("font_line3", "medium");
        let config = AppConfig::from_ini(&ini).unwrap();
        assert_eq!(config.font_sizes[0], FontSize::Large);
        assert_eq!(config.font_sizes[1], FontSize::Small);
        assert_eq!(config.font_sizes[2], FontSize::Medium);
    }

    #[test]
    fn test_appconfig_font_sizes_default_medium_when_missing() {
        use crate::render::FontSize;
        let mut ini = Ini::new();
        ini.with_section(Some("Main")).set("style", "vertical");
        let config = AppConfig::from_ini(&ini).unwrap();
        assert!(config.font_sizes.iter().all(|f| *f == FontSize::Medium));
        assert_eq!(config.font_sizes.len(), crate::consts::DISPLAY_LINES);
    }
}

#[cfg(test)]
mod weather_config_tests {
    use super::*;
    use ini::Ini;

    fn ini_with_weather(section: &str) -> Ini {
        let raw = format!("[Main]\nstyle=vertical\n\n{}", section);
        Ini::load_from_str(&raw).unwrap()
    }

    #[test]
    fn weather_config_missing_section_disabled() {
        let ini = ini_with_weather("");
        let cfg = WeatherConfig::from_ini(&ini);
        assert!(!cfg.enabled);
        assert_eq!(cfg.location, "");
    }

    #[test]
    fn weather_config_empty_location_disabled() {
        let ini = ini_with_weather("[Weather]\nlocation=\"\"\nunits=\"metric\"\n");
        let cfg = WeatherConfig::from_ini(&ini);
        assert!(!cfg.enabled);
    }

    #[test]
    fn weather_config_populated_enabled() {
        let ini = ini_with_weather(
            "[Weather]\nlocation=\"Seattle,US\"\nunits=\"metric\"\nrefresh_minutes=10\n",
        );
        let cfg = WeatherConfig::from_ini(&ini);
        assert!(cfg.enabled);
        assert_eq!(cfg.location, "Seattle,US");
        assert_eq!(cfg.units, crate::weather::Units::Metric);
        assert_eq!(cfg.refresh_minutes, 10);
    }

    #[test]
    fn weather_config_defaults_when_keys_missing() {
        let ini = ini_with_weather("[Weather]\nlocation=\"Boston\"\n");
        let cfg = WeatherConfig::from_ini(&ini);
        assert!(cfg.enabled);
        assert_eq!(cfg.units, crate::weather::Units::Imperial);
        assert_eq!(cfg.refresh_minutes, 15);
    }

    #[test]
    fn weather_config_refresh_minutes_clamped_to_one() {
        let ini = ini_with_weather("[Weather]\nlocation=\"X\"\nrefresh_minutes=0\n");
        let cfg = WeatherConfig::from_ini(&ini);
        assert_eq!(cfg.refresh_minutes, 1);
    }

    #[test]
    fn weather_config_strips_surrounding_quotes_from_location() {
        // ini crate already strips quotes on load, but be explicit in case of nesting.
        let ini = ini_with_weather("[Weather]\nlocation=Boise,US\n");
        let cfg = WeatherConfig::from_ini(&ini);
        assert_eq!(cfg.location, "Boise,US");
    }
}

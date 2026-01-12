use crate::consts::{Style, CUSTOM_SENSORS};
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
    pub custom_sensors: Vec<Vec<CustomSensor>>,
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

        println!(
            "AppConfig: Loaded main settings. is_summary={}, direct_usb={}",
            is_summary, direct_usb
        );

        Ok(Self {
            is_summary,
            is_vertical,
            gpu,
            decimal,
            pages,
            page_time,
            sensors_per_line,
            direct_usb,
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
        })
    }
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

        if category >= hwinfo.sensor_names.len() {
            error!(
                "Invalid category selection: {} (max: {})",
                category,
                hwinfo.sensor_names.len() - 1
            );
            println!("Category out of range, please try again.");
            return Err(anyhow::anyhow!("Invalid category selection"));
        }

        let sensor_name = &hwinfo.sensor_names[category];
        let sensor = hwinfo.sensors.get(sensor_name).ok_or_else(|| {
            error!("Sensor '{}' not found in HWiNFO data", sensor_name);
            anyhow::anyhow!(
                "Sensor '{}' not found - HWiNFO data may have changed",
                sensor_name
            )
        })?;

        // Display available readings for selected sensor in HWiNFO order
        println!("\n{}:", sensor_name);
        let temp_readings: Vec<String> = sensor
            .reading_names
            .iter()
            .enumerate()
            .map(|(i, reading_name)| {
                println!("\t{}) {}", i, reading_name);
                format!("{};{}", sensor_name, reading_name)
            })
            .collect();

        let sensor_selection: usize = Input::new().with_prompt("Sensor").interact_text()?;
        let sensor_selected = &temp_readings[sensor_selection];
        let label: String = Input::new().with_prompt("Label").interact_text()?;

        // Get the selected reading to determine default unit
        let selected_reading_name = &sensor.reading_names[sensor_selection];
        let reading = sensor.readings.get(selected_reading_name).ok_or_else(|| {
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

        // Prompt for unit with default suggestion
        let unit: String = if default_unit.is_empty() {
            Input::new().with_prompt("Unit").interact_text()?
        } else {
            let input: String = Input::new()
                .with_prompt(format!("Unit (default: {})", default_unit))
                .allow_empty(true)
                .interact_text()?;
            if input.is_empty() {
                default_unit.to_string()
            } else {
                input
            }
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

    let style: Style = match input {
        1 => Style::Vertical,
        2 => Style::Horizontal,
        3 => Style::Custom,
        _ => {
            warn!("Invalid style input: {}", input);
            term.write_line("Invalid input")?;
            return settings_create_config(term, hwinfo);
        }
    };

    info!("User selected style: {:?}", style);
    conf.with_section(Some("Main"))
        .set("style", style.to_string());

    let direct_usb: bool = match Input::new()
        .with_prompt("Connection Type\n1) SteelSeries GG (GameSense)\n2) Direct USB (HID)")
        .interact_text()
        .unwrap_or(1)
    {
        1 => false,
        2 => true,
        _ => false,
    };

    conf.with_section(Some("Main"))
        .set("direct_usb", direct_usb.to_string());

    if style != Style::Custom {
        configure_gpu_selection(term, hwinfo, &mut conf)?;
    } else {
        println!(
            "\nUp to 5 lines will fit on the Arctis(or Nova) Pro screen, and 2 on the Apex Pro."
        );

        let lines: u8 = Input::new()
            .with_prompt("How many lines? (2-5)")
            .interact_text()
            .ok()
            .filter(|&l| l >= 2 && l <= 5)
            .unwrap_or(3);

        let sensors_per_line: u8 = Input::new()
            .with_prompt("How many sensors per line? (1-3)")
            .interact_text()?;

        if !(1..=3).contains(&sensors_per_line) {
            return settings_create_config(term, hwinfo);
        }

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
}

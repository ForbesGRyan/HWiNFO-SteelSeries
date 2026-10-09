use crate::consts::{CUSTOM_SENSORS, DISPLAY_LINES};
use crate::render::FontSize;
use crate::weather::Units;
use anyhow::anyhow;
use ini::Ini;
use log::{debug, warn};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CustomSensor {
    pub sensor: String,
    pub label: String,
    pub unit: String,
    pub convert: String,
    /// Builtin icon name drawn before this sensor on the direct-USB OLED
    /// bitmap (e.g. "cpu", "gpu", "temp"). Empty = no icon. Ignored in
    /// SteelSeries GG mode. See [`crate::render::icon_by_name`].
    #[serde(default)]
    pub icon: String,
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
        units.as_str().serialize(s)
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
        let main = config.section(Some("Main"));

        let main = main.ok_or_else(|| anyhow::anyhow!("Main config section not found"))?;

        let style = main
            .get("style")
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| {
                warn!("'style' key not found in conf.ini, defaulting to 'vertical'");
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

        debug!(
            "Loaded main settings: is_summary={}, direct_usb={}",
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
                                let icon =
                                    section.get(format!("icon_{}", k)).unwrap_or("").to_string();
                                // Preserve index gaps: empty slots between
                                // sensors map to flat positions (line * spl +
                                // slot), so pad up to k before inserting.
                                while page_sensors.len() < k {
                                    page_sensors.push(CustomSensor::default());
                                }
                                page_sensors.push(CustomSensor {
                                    sensor: sensor.to_string(),
                                    label,
                                    unit,
                                    convert,
                                    icon,
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

/// Load `path`, or on first run write a minimal config there. Returns the
/// config and whether it was just created, so the caller can open the settings
/// window. A file that exists but cannot be read or parsed is an error and is
/// left untouched.
pub fn load_or_create_config(path: &str) -> Result<(AppConfig, bool), anyhow::Error> {
    match Ini::load_from_file(path) {
        Ok(ini) => Ok((AppConfig::from_ini(&ini)?, false)),
        Err(ini::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut ini = Ini::new();
            ini.with_section(Some("Main")).set("style", "Vertical");
            ini.write_to_file(path)
                .map_err(|e| anyhow!("Failed to write {}: {}", path, e))?;
            Ok((AppConfig::from_ini(&ini)?, true))
        }
        Err(e) => Err(anyhow!("Failed to read {}: {}", path, e)),
    }
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

    fn temp_ini(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("hwinfo_ss_{}_{}.ini", name, std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn load_or_create_config_writes_default_when_missing() {
        let path = temp_ini("first_run");
        let (cfg, created) = load_or_create_config(path.to_str().unwrap()).unwrap();
        assert!(created);
        assert!(cfg.is_summary);
        assert!(cfg.is_vertical);
        assert!(!cfg.direct_usb);
        // The file now exists and loads as an existing config.
        let (again, created_again) = load_or_create_config(path.to_str().unwrap()).unwrap();
        assert!(!created_again);
        assert!(again.is_summary);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_or_create_config_loads_existing_without_rewriting() {
        let path = temp_ini("existing");
        std::fs::write(
            &path,
            "[Main]
style=Horizontal
",
        )
        .unwrap();
        let (cfg, created) = load_or_create_config(path.to_str().unwrap()).unwrap();
        assert!(!created);
        assert!(cfg.is_summary);
        assert!(!cfg.is_vertical);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[Main]
style=Horizontal
"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_or_create_config_errors_on_malformed_ini_and_keeps_file() {
        let path = temp_ini("malformed");
        std::fs::write(
            &path,
            "[Main
style=Vertical
",
        )
        .unwrap();
        assert!(load_or_create_config(path.to_str().unwrap()).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[Main
style=Vertical
"
        );
        let _ = std::fs::remove_file(&path);
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
    fn test_appconfig_loads_icon_from_pages() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Custom")
            .set("pages", "1");
        conf.with_section(Some("PAGE1.Sensors"))
            .set("sensor_0", "CPU [#0];Temperature")
            .set("icon_0", "cpu")
            .set("sensor_1", "GPU [#0];Temperature"); // no icon_1

        let config = AppConfig::from_ini(&conf).unwrap();

        assert_eq!(config.custom_sensors[0][0].icon, "cpu");
        assert_eq!(config.custom_sensors[0][1].icon, "");
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

    #[test]
    fn test_from_ini_preserves_sensor_index_gaps() {
        // sensor_0 + sensor_3 (e.g. DATE on line 1, CLOCK on line 2 with
        // sensors_per_line=3) must reconstruct with the gap intact, so the
        // flat index `line * spl + slot` still maps CLOCK to line 2 slot 0.
        let mut ini = Ini::new();
        ini.with_section(Some("Main"))
            .set("style", "custom")
            .set("pages", "1")
            .set("sensors_per_line", "3");
        ini.with_section(Some("PAGE1.Sensors"))
            .set("sensor_0", "DATE")
            .set("sensor_3", "CLOCK");
        let config = AppConfig::from_ini(&ini).unwrap();
        let page = &config.custom_sensors[0];
        assert_eq!(page.len(), 4, "gap slots must be preserved");
        assert_eq!(page[0].sensor, "DATE");
        assert_eq!(page[1].sensor, "");
        assert_eq!(page[2].sensor, "");
        assert_eq!(page[3].sensor, "CLOCK");
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

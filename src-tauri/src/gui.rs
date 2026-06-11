use crate::connect::list_oled_devices;
use crate::consts::CUSTOM_SENSORS;
use crate::media::MediaReader;
use crate::mouse_battery::MouseBatteryReader;
use crate::render::render_text_to_oled;
use crate::settings::AppConfig;
use crate::state::{Shared, SleepCommand, StatusPayload};
use crate::utils::{format_custom_value, run_sensors};
use ini::Ini;
use log::{error, info};
use serde::Serialize;
use serde_json::Value;
use tauri::{command, State};

const SPECIAL_SENSORS: &[(&str, &str)] = &[
    (
        "CLOCK",
        "Current time (append ;FORMAT for a custom strftime, e.g. CLOCK;%H:%M)",
    ),
    (
        "DATE",
        "Current date (append ;FORMAT for a custom strftime, e.g. DATE;%m/%d/%Y)",
    ),
    ("BLANK", "Empty spacer"),
    ("MEDIA_TITLE", "Now-playing track title"),
    ("MEDIA_ARTIST", "Now-playing artist"),
    ("MEDIA_ALBUM", "Now-playing album"),
    ("MEDIA_APP", "Now-playing app source"),
];

/// Weather special sensors, backed by the [Weather] config + wttr.in.
/// Listed separately so they group under their own picker category.
/// The `_D1`/`_D2` suffixes are tomorrow / day-after; the parser also accepts
/// `_D3` if typed manually.
const WEATHER_SENSORS: &[(&str, &str)] = &[
    ("WEATHER_TEMP", "Current temperature"),
    ("WEATHER_FEELS", "Feels-like temperature"),
    ("WEATHER_HI", "Today's high"),
    ("WEATHER_LO", "Today's low"),
    ("WEATHER_CONDITION", "Condition (full text)"),
    ("WEATHER_CONDITION_SHORT", "Condition (abbreviated)"),
    ("WEATHER_HUMIDITY", "Humidity %"),
    ("WEATHER_WIND_SPEED", "Wind speed"),
    ("WEATHER_WIND_DIR", "Wind direction"),
    ("WEATHER_WIND_GUST", "Wind gust"),
    ("WEATHER_PRECIP_CHANCE", "Precipitation chance %"),
    ("WEATHER_PRECIP_AMOUNT", "Precipitation amount"),
    ("WEATHER_UV", "UV index"),
    ("WEATHER_PRESSURE", "Pressure"),
    ("WEATHER_CLOUDS", "Cloud cover %"),
    ("WEATHER_VISIBILITY", "Visibility"),
    ("WEATHER_SUNRISE", "Sunrise time"),
    ("WEATHER_SUNSET", "Sunset time"),
    ("WEATHER_HI_D1", "High, tomorrow"),
    ("WEATHER_LO_D1", "Low, tomorrow"),
    ("WEATHER_CONDITION_D1", "Condition tomorrow (full)"),
    ("WEATHER_CONDITION_SHORT_D1", "Condition tomorrow (short)"),
    ("WEATHER_PRECIP_CHANCE_D1", "Precip chance tomorrow %"),
    ("WEATHER_HI_D2", "High, in 2 days"),
    ("WEATHER_LO_D2", "Low, in 2 days"),
    ("WEATHER_CONDITION_D2", "Condition in 2 days (full)"),
    ("WEATHER_CONDITION_SHORT_D2", "Condition in 2 days (short)"),
    ("WEATHER_PRECIP_CHANCE_D2", "Precip chance in 2 days %"),
];

/// Pick the INI "style" string for a config.
fn style_from_config(config: &AppConfig) -> &'static str {
    if !config.is_summary {
        "Custom"
    } else if config.is_vertical {
        "Vertical"
    } else {
        "Horizontal"
    }
}

/// Static summary placeholder for the preview pane.
fn summary_preview_value(is_vertical: bool) -> Value {
    if is_vertical {
        serde_json::json!({
            "line1": "CPU   GPU   MEM",
            "line2": "00°   00°   00G",
            "line3": "00%   00%   00G",
        })
    } else {
        serde_json::json!({
            "line1": "CPU 00° 00%",
            "line2": "GPU 00° 00%",
            "line3": "MEM 00G 00%",
        })
    }
}

/// Build the `ini::Properties` block run_sensors needs from a slice of CustomSensor.
fn build_preview_props(page_sensors: &[crate::settings::CustomSensor]) -> ini::Properties {
    let mut props = ini::Properties::new();
    for (k, sensor) in page_sensors.iter().enumerate() {
        if k >= CUSTOM_SENSORS {
            break;
        }
        if sensor.sensor.trim().is_empty() {
            continue;
        }
        props.insert(format!("sensor_{}", k), sensor.sensor.clone());
        props.insert(format!("label_{}", k), sensor.label.clone());
        props.insert(format!("unit_{}", k), sensor.unit.clone());
        props.insert(format!("convert_{}", k), sensor.convert.clone());
        props.insert(format!("icon_{}", k), sensor.icon.clone());
    }
    props
}

/// Convert a line1/line2/.../lineN object back into newline-separated text for rendering.
/// Keys are sorted lexicographically for deterministic ordering.
fn value_to_preview_text(value: &Value) -> String {
    let mut text = String::new();
    if let Some(obj) = value.as_object() {
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        let mut first = true;
        for key in keys {
            if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                if !first {
                    text.push('\n');
                }
                text.push_str(s);
                first = false;
            }
        }
    }
    text
}

/// Built-in special sensors: CLOCK/BLANK/MEDIA_* grouped under "Special",
/// WEATHER_* grouped under "Weather".
fn special_sensor_options() -> Vec<SensorOption> {
    let special = SPECIAL_SENSORS.iter().map(|(name, desc)| SensorOption {
        category: "Special".to_string(),
        reading: name.to_string(),
        full_id: name.to_string(),
        is_special: true,
        description: Some(desc.to_string()),
    });
    let weather = WEATHER_SENSORS.iter().map(|(name, desc)| SensorOption {
        category: "Weather".to_string(),
        reading: name.to_string(),
        full_id: name.to_string(),
        is_special: true,
        description: Some(desc.to_string()),
    });
    special.chain(weather).collect()
}

/// Flatten an Hwinfo snapshot into a sorted list of SensorOption rows.
fn sensors_from_hwinfo(hw: &hwinfo_steelseries_oled::Hwinfo) -> Vec<SensorOption> {
    let mut out = Vec::new();
    let mut categories: Vec<&String> = hw.sensors.keys().collect();
    categories.sort();
    for cat in categories {
        if let Some(sensor) = hw.sensors.get(cat) {
            let mut readings: Vec<&String> = sensor.readings.keys().collect();
            readings.sort();
            for reading in readings {
                out.push(SensorOption {
                    category: cat.clone(),
                    reading: reading.clone(),
                    full_id: format!("{};{}", cat, reading),
                    is_special: false,
                    description: None,
                });
            }
        }
    }
    out
}

/// Write the [Main] section keys from an AppConfig into an Ini.
fn apply_main_section(ini: &mut Ini, config: &AppConfig) {
    let style = style_from_config(config);
    ini.with_section(Some("Main"))
        .set("style", style)
        .set("direct_usb", config.direct_usb.to_string())
        .set("direct_usb_serial", &config.direct_usb_serial)
        .set("decimal", config.decimal.to_string())
        .set("pages", config.pages.to_string())
        .set("page_time", config.page_time.to_string())
        .set("sensors_per_line", config.sensors_per_line.to_string())
        .set("gpu", &config.gpu);
    {
        let mut sec = ini.with_section(Some("Main"));
        for (i, fs) in config.font_sizes.iter().enumerate() {
            sec.set(format!("font_line{}", i + 1), fs.as_str());
        }
    }
}

/// Delete existing PAGE*.Sensors sections and rewrite from the config's custom_sensors.
fn apply_pages_sections(ini: &mut Ini, config: &AppConfig) {
    let existing_pages: Vec<String> = ini
        .sections()
        .filter_map(|s| s.map(|s| s.to_string()))
        .filter(|s| s.starts_with("PAGE") && s.ends_with(".Sensors"))
        .collect();
    for sec in existing_pages {
        ini.delete(Some(sec.as_str()));
    }

    for (i, page) in config.custom_sensors.iter().enumerate() {
        let section_name = format!("PAGE{}.Sensors", i + 1);
        let mut section = ini.with_section(Some(section_name));
        for (k, sensor) in page.iter().enumerate() {
            if k >= CUSTOM_SENSORS {
                break;
            }
            if sensor.sensor.trim().is_empty() {
                continue;
            }
            section.set(format!("sensor_{}", k), &sensor.sensor);
            section.set(format!("label_{}", k), &sensor.label);
            section.set(format!("unit_{}", k), &sensor.unit);
            section.set(format!("convert_{}", k), &sensor.convert);
            section.set(format!("icon_{}", k), &sensor.icon);
        }
    }
}

/// Write the [Weather] section from an AppConfig into an Ini.
/// When weather is disabled, the location is blanked (the backend treats an
/// empty location as disabled) while units/refresh are preserved for re-enable.
fn apply_weather_section(ini: &mut Ini, config: &AppConfig) {
    let w = &config.weather;
    let location = if w.enabled { w.location.trim() } else { "" };
    ini.with_section(Some("Weather"))
        .set("location", location)
        .set("units", w.units.as_str())
        .set("refresh_minutes", w.refresh_minutes.to_string());
}

#[derive(Debug, Clone, Serialize)]
pub struct HidDeviceInfo {
    pub serial: String,
    pub product: String,
    pub manufacturer: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub interface_number: i32,
    /// Platform HID device path. Stable identifier used to target a
    /// specific device interface when no USB serial is exposed.
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorOption {
    pub category: String,
    pub reading: String,
    pub full_id: String,
    pub is_special: bool,
    pub description: Option<String>,
}

fn get_status_impl(shared: &Shared) -> Result<StatusPayload, String> {
    let g = shared.lock().map_err(|e| e.to_string())?;
    Ok(g.status_payload())
}

fn get_config_impl(shared: &Shared) -> Result<AppConfig, String> {
    let g = shared.lock().map_err(|e| e.to_string())?;
    Ok(g.config.clone())
}

fn save_config_impl(path: &str, shared: &Shared, config: AppConfig) -> Result<(), String> {
    let mut ini = Ini::load_from_file(path).unwrap_or_else(|_| Ini::new());
    apply_main_section(&mut ini, &config);
    apply_weather_section(&mut ini, &config);
    if !config.is_summary {
        apply_pages_sections(&mut ini, &config);
    }

    ini.write_to_file(path).map_err(|e| {
        error!("Failed to write {}: {}", path, e);
        e.to_string()
    })?;
    info!("Config saved to {}", path);

    let mut g = shared.lock().map_err(|e| e.to_string())?;
    g.config = config;
    g.reload_requested = true;
    g.sleep_requested = Some(SleepCommand::Wake);
    Ok(())
}

fn get_live_preview_impl(shared: &Shared) -> Result<Vec<u8>, String> {
    let g = shared.lock().map_err(|e| e.to_string())?;
    Ok(buffer_to_pixels(&g.oled_buffer))
}

#[command]
pub fn get_status(state: State<Shared>) -> Result<StatusPayload, String> {
    get_status_impl(&state)
}

#[command]
pub fn get_config(state: State<Shared>) -> Result<AppConfig, String> {
    get_config_impl(&state)
}

#[command]
pub fn save_config(state: State<Shared>, config: AppConfig) -> Result<(), String> {
    save_config_impl("conf.ini", &state, config)
}

#[command]
pub fn get_live_preview(state: State<Shared>) -> Result<Vec<u8>, String> {
    get_live_preview_impl(&state)
}

#[command]
pub fn list_hid_devices() -> Result<Vec<HidDeviceInfo>, String> {
    let api = hidapi::HidApi::new().map_err(|e| format!("HID API init failed: {}", e))?;
    let devices = list_oled_devices(&api);
    Ok(devices
        .iter()
        .map(|d| HidDeviceInfo {
            serial: d.serial_number().unwrap_or("").to_string(),
            product: d.product_string().unwrap_or("").to_string(),
            manufacturer: d.manufacturer_string().unwrap_or("").to_string(),
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            interface_number: d.interface_number(),
            path: d.path().to_string_lossy().into_owned(),
        })
        .collect())
}

fn list_sensors_impl(shared: &Shared) -> Result<Vec<SensorOption>, String> {
    let g = shared.lock().map_err(|e| e.to_string())?;
    let mut out = special_sensor_options();
    if let Some(hw) = g.hwinfo_snapshot.as_ref() {
        out.extend(sensors_from_hwinfo(hw));
    }
    Ok(out)
}

#[command]
pub fn list_sensors(state: State<Shared>) -> Result<Vec<SensorOption>, String> {
    list_sensors_impl(&state)
}

fn preview_config_impl(shared: &Shared, config: AppConfig, page: usize) -> Result<Vec<u8>, String> {
    let (hwinfo_opt, media_info, weather_info) = {
        let g = shared.lock().map_err(|e| e.to_string())?;
        (
            g.hwinfo_snapshot.clone(),
            g.media_info.clone(),
            g.weather_info.clone(),
        )
    };

    let value: Value = if config.is_summary {
        summary_preview_value(config.is_vertical)
    } else {
        let hwinfo = match hwinfo_opt.as_ref() {
            Some(h) => h,
            None => {
                let buf = render_text_to_oled("HWiNFO not\nconnected", 0, &[], 128, 64);
                return Ok(buffer_to_pixels(&buf));
            }
        };

        let page_sensors = config.custom_sensors.get(page).cloned().unwrap_or_default();
        let props = build_preview_props(&page_sensors);

        let mut labels = vec![""; CUSTOM_SENSORS];
        let mut units = vec![""; CUSTOM_SENSORS];
        let mut values = vec![String::new(); CUSTOM_SENSORS];

        let mut mb = MouseBatteryReader::new();
        // Seed the preview readers with the daemon's live snapshots so MEDIA_*/
        // WEATHER_* sensors render real data instead of always-blank placeholders.
        let mut media = MediaReader::with_cached_info(media_info);
        let weather = match weather_info {
            Some(info) => crate::weather::WeatherReader::with_cached_info(info),
            None => crate::weather::WeatherReader::disabled(),
        };

        if let Err(e) = run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            hwinfo,
            config.decimal,
            &mut mb,
            &mut media,
            &weather,
            None,
        ) {
            let buf = render_text_to_oled(&format!("Preview error:\n{}", e), 0, &[], 128, 64);
            return Ok(buffer_to_pixels(&buf));
        }

        let icons: Vec<&str> = (0..CUSTOM_SENSORS)
            .map(|k| props.get(format!("icon_{}", k)).unwrap_or_default())
            .collect();

        format_custom_value(config.sensors_per_line, labels, values, units, icons)
    };

    let buf = render_text_to_oled(
        &value_to_preview_text(&value),
        0,
        &config.font_sizes,
        128,
        64,
    );
    Ok(buffer_to_pixels(&buf))
}

#[command]
pub fn preview_config(
    state: State<Shared>,
    config: AppConfig,
    page: usize,
) -> Result<Vec<u8>, String> {
    preview_config_impl(&state, config, page)
}

fn buffer_to_pixels(buf: &crate::render::OledBuffer) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((buf.width * buf.height) as usize);
    let pages = (buf.height / 8) as usize;
    for y in 0..buf.height {
        for x in 0..buf.width {
            let idx = x as usize * pages + (y / 8) as usize;
            let on = (buf.data[idx] & (1 << (y % 8))) != 0;
            pixels.push(if on { 255 } else { 0 });
        }
    }
    pixels
}

fn set_sleep_command(shared: &Shared, cmd: SleepCommand) -> Result<(), String> {
    let mut g = shared.lock().map_err(|e| e.to_string())?;
    g.sleep_requested = Some(cmd);
    Ok(())
}

#[command]
pub fn request_sleep(state: State<Shared>) -> Result<(), String> {
    set_sleep_command(&state, SleepCommand::Sleep)
}

#[command]
pub fn request_wake(state: State<Shared>) -> Result<(), String> {
    set_sleep_command(&state, SleepCommand::Wake)
}

#[command]
pub fn request_white_screen(state: State<Shared>) -> Result<(), String> {
    set_sleep_command(&state, SleepCommand::White)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::OledBuffer;
    use crate::settings::{CustomSensor, WeatherConfig};
    use hwinfo_steelseries_oled::{
        Hwinfo, HwinfoSensorsReadingElement, HwinfoSensorsSensorElement, Sensor,
    };
    use std::collections::HashMap;

    fn base_config() -> AppConfig {
        AppConfig {
            is_summary: true,
            is_vertical: true,
            gpu: "GPU [#0]".to_string(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            direct_usb_serial: String::new(),
            custom_sensors: Vec::new(),
            weather: WeatherConfig::default(),
            font_sizes: [crate::render::FontSize::Medium; crate::consts::DISPLAY_LINES],
        }
    }

    fn sensor(name: &str) -> CustomSensor {
        CustomSensor {
            sensor: name.to_string(),
            label: format!("L_{}", name),
            unit: "U".to_string(),
            convert: String::new(),
            icon: String::new(),
        }
    }

    // ==================== style_from_config ====================

    #[test]
    fn test_style_from_config_custom() {
        let mut c = base_config();
        c.is_summary = false;
        assert_eq!(style_from_config(&c), "Custom");
    }

    #[test]
    fn test_style_from_config_vertical() {
        let c = base_config();
        assert_eq!(style_from_config(&c), "Vertical");
    }

    #[test]
    fn test_style_from_config_horizontal() {
        let mut c = base_config();
        c.is_vertical = false;
        assert_eq!(style_from_config(&c), "Horizontal");
    }

    // ==================== summary_preview_value ====================

    #[test]
    fn test_summary_preview_value_vertical() {
        let v = summary_preview_value(true);
        assert_eq!(v["line1"], "CPU   GPU   MEM");
        assert!(v["line2"].as_str().unwrap().contains("00°"));
    }

    #[test]
    fn test_summary_preview_value_horizontal() {
        let v = summary_preview_value(false);
        assert_eq!(v["line1"], "CPU 00° 00%");
        assert_eq!(v["line3"], "MEM 00G 00%");
    }

    // ==================== build_preview_props ====================

    #[test]
    fn test_build_preview_props_writes_all_fields() {
        let page = vec![sensor("CPU;Temp")];
        let p = build_preview_props(&page);
        assert_eq!(p.get("sensor_0"), Some("CPU;Temp"));
        assert_eq!(p.get("label_0"), Some("L_CPU;Temp"));
        assert_eq!(p.get("unit_0"), Some("U"));
        assert_eq!(p.get("convert_0"), Some(""));
        assert_eq!(p.get("icon_0"), Some(""));
    }

    #[test]
    fn test_build_preview_props_writes_icon() {
        let page = vec![CustomSensor {
            sensor: "CPU;Temp".to_string(),
            label: String::new(),
            unit: String::new(),
            convert: String::new(),
            icon: "cpu".to_string(),
        }];
        let p = build_preview_props(&page);
        assert_eq!(p.get("icon_0"), Some("cpu"));
    }

    #[test]
    fn test_build_preview_props_skips_empty_sensor_string() {
        let page = vec![sensor("   "), sensor("S;R")];
        let p = build_preview_props(&page);
        assert!(p.get("sensor_0").is_none());
        assert_eq!(p.get("sensor_1"), Some("S;R"));
    }

    #[test]
    fn test_build_preview_props_caps_at_custom_sensors() {
        let page: Vec<CustomSensor> = (0..(CUSTOM_SENSORS + 5))
            .map(|i| sensor(&format!("S{};R", i)))
            .collect();
        let p = build_preview_props(&page);
        // Last in-range index should exist, first out-of-range should not
        let last = format!("sensor_{}", CUSTOM_SENSORS - 1);
        let over = format!("sensor_{}", CUSTOM_SENSORS);
        assert!(p.get(&last).is_some());
        assert!(p.get(&over).is_none());
    }

    // ==================== value_to_preview_text ====================

    #[test]
    fn test_value_to_preview_text_orders_lines() {
        let v = serde_json::json!({ "line3": "C", "line1": "A", "line2": "B" });
        assert_eq!(value_to_preview_text(&v), "A\nB\nC");
    }

    #[test]
    fn test_value_to_preview_text_preserves_blank_slots() {
        // Empty lines keep their position instead of shifting later lines up.
        let v = serde_json::json!({ "line1": "", "line2": "CPU", "line3": "GPU" });
        assert_eq!(value_to_preview_text(&v), "\nCPU\nGPU");
        let v2 = serde_json::json!({ "line1": "A", "line2": "", "line3": "C" });
        assert_eq!(value_to_preview_text(&v2), "A\n\nC");
    }

    #[test]
    fn test_value_to_preview_text_skips_non_string() {
        let v = serde_json::json!({ "line1": "A", "line2": 42, "line3": "C" });
        assert_eq!(value_to_preview_text(&v), "A\nC");
    }

    #[test]
    fn test_value_to_preview_text_empty_object() {
        let v = serde_json::json!({});
        assert_eq!(value_to_preview_text(&v), "");
    }

    #[test]
    fn test_value_to_preview_text_non_object_returns_empty() {
        let v = serde_json::json!("not an object");
        assert_eq!(value_to_preview_text(&v), "");
    }

    // ==================== special_sensor_options ====================

    #[test]
    fn test_special_sensor_options_count_and_categories() {
        let opts = special_sensor_options();
        assert_eq!(opts.len(), SPECIAL_SENSORS.len() + WEATHER_SENSORS.len());
        for opt in &opts {
            assert!(opt.category == "Special" || opt.category == "Weather");
            assert!(opt.is_special);
            assert!(opt.description.is_some());
            assert_eq!(opt.reading, opt.full_id);
        }
    }

    #[test]
    fn test_special_sensor_options_includes_clock_and_media() {
        let opts = special_sensor_options();
        let names: Vec<&str> = opts.iter().map(|o| o.reading.as_str()).collect();
        assert!(names.contains(&"CLOCK"));
        assert!(names.contains(&"DATE"));
        assert!(names.contains(&"BLANK"));
        assert!(names.contains(&"MEDIA_TITLE"));
    }

    #[test]
    fn test_special_sensor_options_includes_weather() {
        let opts = special_sensor_options();
        let weather: Vec<&str> = opts
            .iter()
            .filter(|o| o.category == "Weather")
            .map(|o| o.reading.as_str())
            .collect();
        assert!(weather.contains(&"WEATHER_TEMP"));
        assert!(weather.contains(&"WEATHER_CONDITION_SHORT"));
        assert!(weather.contains(&"WEATHER_HI_D1"));
        // Every weather option parses as a real weather field.
        for name in &weather {
            assert!(
                crate::weather::WeatherField::from_sensor_name(name).is_some(),
                "{} is not a recognized weather sensor",
                name
            );
        }
    }

    // ==================== sensors_from_hwinfo ====================

    fn build_hwinfo(entries: &[(&str, &str)]) -> Hwinfo {
        let mut sensors: HashMap<String, Sensor> = HashMap::new();
        let mut sensor_names: Vec<String> = Vec::new();
        for (sk, rk) in entries {
            let entry = sensors.entry(sk.to_string()).or_insert_with(|| {
                sensor_names.push(sk.to_string());
                Sensor {
                    info: HwinfoSensorsSensorElement::new_mock(0, sk),
                    readings: HashMap::new(),
                    reading_names: Vec::new(),
                }
            });
            entry.readings.insert(
                rk.to_string(),
                HwinfoSensorsReadingElement::new_mock(0, 0, rk, 1.0),
            );
            entry.reading_names.push(rk.to_string());
        }
        Hwinfo::new_mock(sensors, sensor_names)
    }

    #[test]
    fn test_sensors_from_hwinfo_sorts_categories_and_readings() {
        let hw = build_hwinfo(&[
            ("ZSensor", "Beta"),
            ("ASensor", "Beta"),
            ("ASensor", "Alpha"),
        ]);
        let opts = sensors_from_hwinfo(&hw);
        assert_eq!(opts.len(), 3);
        // Categories sorted: ASensor first
        assert_eq!(opts[0].category, "ASensor");
        // Readings within ASensor sorted: Alpha first
        assert_eq!(opts[0].reading, "Alpha");
        assert_eq!(opts[1].category, "ASensor");
        assert_eq!(opts[1].reading, "Beta");
        assert_eq!(opts[2].category, "ZSensor");
    }

    #[test]
    fn test_sensors_from_hwinfo_full_id_format() {
        let hw = build_hwinfo(&[("CPU", "Temp")]);
        let opts = sensors_from_hwinfo(&hw);
        assert_eq!(opts[0].full_id, "CPU;Temp");
        assert!(!opts[0].is_special);
        assert!(opts[0].description.is_none());
    }

    #[test]
    fn test_sensors_from_hwinfo_empty() {
        let hw = build_hwinfo(&[]);
        assert!(sensors_from_hwinfo(&hw).is_empty());
    }

    // ==================== apply_main_section ====================

    #[test]
    fn test_apply_main_section_writes_all_keys() {
        let mut ini = Ini::new();
        let mut c = base_config();
        c.decimal = true;
        c.direct_usb = true;
        c.direct_usb_serial = "ABC123".to_string();
        c.pages = 3;
        c.page_time = 7;
        c.sensors_per_line = 2;
        apply_main_section(&mut ini, &c);
        let main = ini.section(Some("Main")).unwrap();
        assert_eq!(main.get("style"), Some("Vertical"));
        assert_eq!(main.get("direct_usb"), Some("true"));
        assert_eq!(main.get("direct_usb_serial"), Some("ABC123"));
        assert_eq!(main.get("decimal"), Some("true"));
        assert_eq!(main.get("pages"), Some("3"));
        assert_eq!(main.get("page_time"), Some("7"));
        assert_eq!(main.get("sensors_per_line"), Some("2"));
        assert_eq!(main.get("gpu"), Some("GPU [#0]"));
    }

    // ==================== apply_pages_sections ====================

    #[test]
    fn test_apply_pages_sections_writes_sensors() {
        let mut ini = Ini::new();
        let mut c = base_config();
        c.is_summary = false;
        c.custom_sensors = vec![vec![sensor("CPU;Temp"), sensor("GPU;Temp")]];
        apply_pages_sections(&mut ini, &c);
        let sec = ini.section(Some("PAGE1.Sensors")).unwrap();
        assert_eq!(sec.get("sensor_0"), Some("CPU;Temp"));
        assert_eq!(sec.get("sensor_1"), Some("GPU;Temp"));
    }

    #[test]
    fn test_apply_pages_sections_writes_icon() {
        let mut ini = Ini::new();
        let mut c = base_config();
        c.is_summary = false;
        c.custom_sensors = vec![vec![CustomSensor {
            sensor: "CPU;Temp".to_string(),
            label: String::new(),
            unit: String::new(),
            convert: String::new(),
            icon: "cpu".to_string(),
        }]];
        apply_pages_sections(&mut ini, &c);
        let sec = ini.section(Some("PAGE1.Sensors")).unwrap();
        assert_eq!(sec.get("icon_0"), Some("cpu"));
    }

    #[test]
    fn test_apply_pages_sections_deletes_stale_pages() {
        let mut ini = Ini::new();
        ini.with_section(Some("PAGE5.Sensors"))
            .set("sensor_0", "OLD");
        ini.with_section(Some("PAGE6.Sensors"))
            .set("sensor_0", "OLD");
        ini.with_section(Some("UnrelatedSection"))
            .set("key", "keep");

        let mut c = base_config();
        c.is_summary = false;
        c.custom_sensors = vec![vec![sensor("CPU;Temp")]];
        apply_pages_sections(&mut ini, &c);

        assert!(ini.section(Some("PAGE5.Sensors")).is_none());
        assert!(ini.section(Some("PAGE6.Sensors")).is_none());
        assert_eq!(
            ini.section(Some("UnrelatedSection")).unwrap().get("key"),
            Some("keep")
        );
        assert!(ini.section(Some("PAGE1.Sensors")).is_some());
    }

    #[test]
    fn test_apply_pages_sections_skips_empty_sensor_string() {
        let mut ini = Ini::new();
        let mut c = base_config();
        c.is_summary = false;
        c.custom_sensors = vec![vec![sensor("   "), sensor("S;R")]];
        apply_pages_sections(&mut ini, &c);
        let sec = ini.section(Some("PAGE1.Sensors")).unwrap();
        assert!(sec.get("sensor_0").is_none());
        assert_eq!(sec.get("sensor_1"), Some("S;R"));
    }

    #[test]
    fn test_apply_pages_sections_break_when_page_exceeds_custom_sensors_limit() {
        let mut ini = Ini::new();
        let mut c = base_config();
        c.is_summary = false;
        // CUSTOM_SENSORS is 9; create more to exercise the break path.
        let page: Vec<CustomSensor> = (0..(CUSTOM_SENSORS + 3))
            .map(|i| sensor(&format!("S{};R{}", i, i)))
            .collect();
        c.custom_sensors = vec![page];
        apply_pages_sections(&mut ini, &c);
        let sec = ini.section(Some("PAGE1.Sensors")).unwrap();
        // sensor_0..sensor_8 set, sensor_9 NOT set
        assert!(sec
            .get(format!("sensor_{}", CUSTOM_SENSORS - 1).as_str())
            .is_some());
        assert!(sec
            .get(format!("sensor_{}", CUSTOM_SENSORS).as_str())
            .is_none());
    }

    #[test]
    fn test_apply_pages_sections_creates_one_section_per_page() {
        let mut ini = Ini::new();
        let mut c = base_config();
        c.is_summary = false;
        c.custom_sensors = vec![
            vec![sensor("A;1")],
            vec![sensor("B;2")],
            vec![sensor("C;3")],
        ];
        apply_pages_sections(&mut ini, &c);
        assert_eq!(
            ini.section(Some("PAGE1.Sensors")).unwrap().get("sensor_0"),
            Some("A;1")
        );
        assert_eq!(
            ini.section(Some("PAGE2.Sensors")).unwrap().get("sensor_0"),
            Some("B;2")
        );
        assert_eq!(
            ini.section(Some("PAGE3.Sensors")).unwrap().get("sensor_0"),
            Some("C;3")
        );
    }

    // ==================== buffer_to_pixels ====================

    use crate::state::{ActiveMode, SharedState};
    use std::sync::{Arc, Mutex};

    fn mock_shared(cfg: AppConfig) -> Shared {
        Arc::new(Mutex::new(SharedState::new(cfg)))
    }

    #[test]
    fn test_get_status_impl_returns_payload() {
        let shared = mock_shared(base_config());
        let r = get_status_impl(&shared).unwrap();
        assert!(!r.hwinfo_connected);
        assert_eq!(r.active_mode, ActiveMode::Disconnected);
    }

    #[test]
    fn test_get_config_impl_returns_clone() {
        let shared = mock_shared(base_config());
        let r = get_config_impl(&shared).unwrap();
        assert!(r.is_summary);
        assert_eq!(r.gpu, "GPU [#0]");
    }

    #[test]
    fn test_get_live_preview_impl_returns_pixels() {
        let shared = mock_shared(base_config());
        let pixels = get_live_preview_impl(&shared).unwrap();
        assert_eq!(pixels.len(), 128 * 64);
        // Empty buffer → all zero
        assert!(pixels.iter().all(|p| *p == 0));
    }

    #[test]
    fn test_set_sleep_command_writes_state() {
        let shared = mock_shared(base_config());
        set_sleep_command(&shared, SleepCommand::Sleep).unwrap();
        assert_eq!(
            shared.lock().unwrap().sleep_requested,
            Some(SleepCommand::Sleep)
        );
        set_sleep_command(&shared, SleepCommand::Wake).unwrap();
        assert_eq!(
            shared.lock().unwrap().sleep_requested,
            Some(SleepCommand::Wake)
        );
        set_sleep_command(&shared, SleepCommand::White).unwrap();
        assert_eq!(
            shared.lock().unwrap().sleep_requested,
            Some(SleepCommand::White)
        );
    }

    #[test]
    fn test_list_sensors_impl_without_hwinfo_returns_specials_only() {
        let shared = mock_shared(base_config());
        let opts = list_sensors_impl(&shared).unwrap();
        assert_eq!(opts.len(), SPECIAL_SENSORS.len() + WEATHER_SENSORS.len());
        assert!(opts.iter().all(|o| o.is_special));
    }

    #[test]
    fn test_list_sensors_impl_with_hwinfo_extends_specials() {
        let shared = mock_shared(base_config());
        {
            let mut g = shared.lock().unwrap();
            g.hwinfo_snapshot = Some(build_hwinfo(&[("CPU", "Temp")]));
        }
        let opts = list_sensors_impl(&shared).unwrap();
        assert_eq!(
            opts.len(),
            SPECIAL_SENSORS.len() + WEATHER_SENSORS.len() + 1
        );
        assert!(opts
            .iter()
            .any(|o| !o.is_special && o.full_id == "CPU;Temp"));
    }

    #[test]
    fn test_save_config_impl_writes_file_and_updates_state() {
        let tmp =
            std::env::temp_dir().join(format!("hwinfo_ss_gui_save_{}.ini", std::process::id()));
        let _ = std::fs::remove_file(&tmp);

        let shared = mock_shared(base_config());
        let mut new_cfg = base_config();
        new_cfg.decimal = true;
        new_cfg.gpu = "GPU [#1]".to_string();

        save_config_impl(tmp.to_str().unwrap(), &shared, new_cfg).unwrap();

        // File exists and contains key
        let contents = std::fs::read_to_string(&tmp).unwrap();
        assert!(contents.contains("decimal=true"));
        assert!(contents.contains("gpu=GPU [#1]"));

        // State updated
        let g = shared.lock().unwrap();
        assert!(g.config.decimal);
        assert!(g.reload_requested);
        assert_eq!(g.sleep_requested, Some(SleepCommand::Wake));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_save_config_impl_writes_weather_section() {
        let tmp =
            std::env::temp_dir().join(format!("hwinfo_ss_gui_weather_{}.ini", std::process::id()));
        let _ = std::fs::remove_file(&tmp);

        let shared = mock_shared(base_config());
        let mut cfg = base_config();
        cfg.weather.enabled = true;
        cfg.weather.location = "Seattle,US".to_string();
        cfg.weather.units = crate::weather::Units::Metric;
        cfg.weather.refresh_minutes = 30;

        save_config_impl(tmp.to_str().unwrap(), &shared, cfg).unwrap();

        let contents = std::fs::read_to_string(&tmp).unwrap();
        assert!(contents.contains("location=Seattle,US"));
        assert!(contents.contains("units=metric"));
        assert!(contents.contains("refresh_minutes=30"));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_save_config_impl_disabled_weather_blanks_location() {
        let tmp = std::env::temp_dir().join(format!(
            "hwinfo_ss_gui_weather_off_{}.ini",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp);

        let shared = mock_shared(base_config());
        let mut cfg = base_config();
        cfg.weather.enabled = false;
        cfg.weather.location = "Seattle,US".to_string();

        save_config_impl(tmp.to_str().unwrap(), &shared, cfg).unwrap();

        // Round-trip: a disabled save must produce a config that parses back as disabled.
        let ini = Ini::load_from_file(&tmp).unwrap();
        let parsed = crate::settings::WeatherConfig::from_ini(&ini);
        assert!(!parsed.enabled);
        assert!(parsed.location.is_empty());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_command_wrappers_via_tauri_state() {
        use tauri::Manager;
        let app = tauri::test::mock_app();
        let shared = mock_shared(base_config());
        app.manage(shared.clone());

        // Each #[command] wrapper delegates to its `_impl` fn. Call them directly via app.state().
        let state: tauri::State<Shared> = app.state();
        assert!(get_status(state.clone()).is_ok());
        assert!(get_config(state.clone()).is_ok());
        assert!(get_live_preview(state.clone()).is_ok());
        assert!(list_sensors(state.clone()).is_ok());
        assert!(request_sleep(state.clone()).is_ok());
        assert!(request_wake(state.clone()).is_ok());
        assert!(request_white_screen(state.clone()).is_ok());
        // preview_config calls render — works without HWiNFO snapshot
        assert!(preview_config(state.clone(), base_config(), 0).is_ok());
        // save_config writes to conf.ini in cwd. Run it in a temp cwd so it doesn't pollute
        // the test workspace (a stale conf.ini lets a parallel daemon test reach connect_all
        // and loop on HWiNFO retry).
        let tmp =
            std::env::temp_dir().join(format!("hwinfo_ss_save_cfg_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let prev_cwd = std::env::current_dir().unwrap();
        let _ = std::env::set_current_dir(&tmp);
        let _ = save_config(state.clone(), base_config());
        let _ = std::fs::remove_file(tmp.join("conf.ini"));
        let _ = std::env::set_current_dir(&prev_cwd);
    }

    #[test]
    fn test_save_config_impl_errors_on_unwritable_path() {
        let shared = mock_shared(base_config());
        // Path inside a non-existent directory → write_to_file fails
        let bad_path = "C:\\nope-this-dir-does-not-exist-xyz\\config.ini";
        let r = save_config_impl(bad_path, &shared, base_config());
        assert!(r.is_err());
    }

    #[test]
    fn test_save_config_impl_custom_writes_pages() {
        let tmp = std::env::temp_dir().join(format!(
            "hwinfo_ss_gui_save_custom_{}.ini",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp);

        let shared = mock_shared(base_config());
        let mut new_cfg = base_config();
        new_cfg.is_summary = false;
        new_cfg.custom_sensors = vec![vec![sensor("CPU;Temp")]];

        save_config_impl(tmp.to_str().unwrap(), &shared, new_cfg).unwrap();
        let contents = std::fs::read_to_string(&tmp).unwrap();
        assert!(contents.contains("[PAGE1.Sensors]"));
        assert!(contents.contains("CPU;Temp"));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_preview_config_impl_summary_renders_pixels() {
        let shared = mock_shared(base_config());
        let cfg = base_config();
        let pixels = preview_config_impl(&shared, cfg, 0).unwrap();
        assert_eq!(pixels.len(), 128 * 64);
        // Summary preview text "CPU GPU MEM" should light some pixels.
        assert!(pixels.iter().any(|p| *p != 0));
    }

    #[test]
    fn test_preview_config_impl_custom_no_hwinfo_returns_error_text() {
        let shared = mock_shared(base_config());
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.custom_sensors = vec![vec![sensor("CPU;Temp")]];

        let pixels = preview_config_impl(&shared, cfg, 0).unwrap();
        // "HWiNFO not connected" rendered → some pixels lit
        assert_eq!(pixels.len(), 128 * 64);
        assert!(pixels.iter().any(|p| *p != 0));
    }

    #[test]
    fn test_preview_config_impl_custom_with_hwinfo_blank_page() {
        let shared = mock_shared(base_config());
        {
            let mut g = shared.lock().unwrap();
            g.hwinfo_snapshot = Some(build_hwinfo(&[("CPU", "Temp")]));
        }
        let mut cfg = base_config();
        cfg.is_summary = false;
        // No custom_sensors for page 0 → empty page → renders blank text
        let pixels = preview_config_impl(&shared, cfg, 0).unwrap();
        assert_eq!(pixels.len(), 128 * 64);
    }

    #[test]
    fn test_preview_config_impl_custom_run_sensors_error_path() {
        let shared = mock_shared(base_config());
        {
            let mut g = shared.lock().unwrap();
            g.hwinfo_snapshot = Some(build_hwinfo(&[("CPU", "Temp")])); // doesn't have requested sensor
        }
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.custom_sensors = vec![vec![sensor("Nonexistent;Sensor")]];

        let pixels = preview_config_impl(&shared, cfg, 0).unwrap();
        // Preview error rendered to pixels
        assert!(pixels.iter().any(|p| *p != 0));
    }

    #[test]
    fn test_preview_config_impl_renders_live_media() {
        let shared = mock_shared(base_config());
        {
            let mut g = shared.lock().unwrap();
            g.hwinfo_snapshot = Some(build_hwinfo(&[("CPU", "Temp")]));
            g.media_info = crate::media::MediaInfo {
                title: "NowPlaying".to_string(),
                is_playing: true,
                ..Default::default()
            };
        }
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.custom_sensors = vec![vec![sensor("MEDIA_TITLE")]];

        let pixels = preview_config_impl(&shared, cfg, 0).unwrap();
        // The live media title must render → some pixels lit. With the old
        // throwaway MediaReader, nothing was playing → blank frame.
        assert!(
            pixels.iter().any(|p| *p != 0),
            "live MEDIA_TITLE should render in preview"
        );
    }

    #[test]
    fn test_preview_config_impl_renders_live_weather() {
        let shared = mock_shared(base_config());
        {
            let mut g = shared.lock().unwrap();
            g.hwinfo_snapshot = Some(build_hwinfo(&[("CPU", "Temp")]));
            g.weather_info = Some(crate::weather::WeatherInfo {
                temp: Some("72".to_string()),
                ..Default::default()
            });
        }
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.custom_sensors = vec![vec![sensor("WEATHER_TEMP")]];

        let pixels = preview_config_impl(&shared, cfg, 0).unwrap();
        // The live weather temp must render → some pixels lit. With the old
        // disabled WeatherReader, the field was None → blank frame.
        assert!(
            pixels.iter().any(|p| *p != 0),
            "live WEATHER_TEMP should render in preview"
        );
    }

    #[test]
    fn test_apply_main_section_writes_font_sizes() {
        use crate::render::FontSize;
        let mut config = base_config();
        config.font_sizes[0] = FontSize::Large;
        config.font_sizes[1] = FontSize::Small;
        let mut ini = Ini::new();
        apply_main_section(&mut ini, &config);
        let main = ini.section(Some("Main")).unwrap();
        assert_eq!(main.get("font_line1"), Some("large"));
        assert_eq!(main.get("font_line2"), Some("small"));
        assert_eq!(main.get("font_line3"), Some("medium"));
    }

    #[test]
    fn test_list_hid_devices_returns_vec() {
        // Real HidApi enumeration — may be empty in CI, but should not error
        if let Ok(v) = list_hid_devices() {
            for d in &v {
                assert_eq!(d.vendor_id, crate::connect::HID_VENDOR_ID);
            }
        }
    }

    #[test]
    fn test_buffer_to_pixels_size_and_mapping() {
        let mut buf = OledBuffer::new(128, 64);
        buf.set_pixel(0, 0, true);
        buf.set_pixel(127, 63, true);
        let px = buffer_to_pixels(&buf);
        assert_eq!(px.len(), 128 * 64);
        assert_eq!(px[0], 255);
        assert_eq!(px[63 * 128 + 127], 255);
        assert_eq!(px[1], 0);
    }
}

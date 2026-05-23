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
    ("CLOCK", "Current time (12-hour)"),
    ("BLANK", "Empty spacer"),
    ("MEDIA_TITLE", "Now-playing track title"),
    ("MEDIA_ARTIST", "Now-playing artist"),
    ("MEDIA_ALBUM", "Now-playing album"),
    ("MEDIA_APP", "Now-playing app source"),
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
        for key in keys {
            if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(s);
            }
        }
    }
    text
}

/// Built-in special sensors (CLOCK, BLANK, MEDIA_*).
fn special_sensor_options() -> Vec<SensorOption> {
    SPECIAL_SENSORS
        .iter()
        .map(|(name, desc)| SensorOption {
            category: "Special".to_string(),
            reading: name.to_string(),
            full_id: name.to_string(),
            is_special: true,
            description: Some(desc.to_string()),
        })
        .collect()
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
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HidDeviceInfo {
    pub serial: String,
    pub product: String,
    pub manufacturer: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub interface_number: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorOption {
    pub category: String,
    pub reading: String,
    pub full_id: String,
    pub is_special: bool,
    pub description: Option<String>,
}

#[command]
pub fn get_status(state: State<Shared>) -> Result<StatusPayload, String> {
    let g = state.lock().map_err(|e| e.to_string())?;
    Ok(g.status_payload())
}

#[command]
pub fn get_config(state: State<Shared>) -> Result<AppConfig, String> {
    let g = state.lock().map_err(|e| e.to_string())?;
    Ok(g.config.clone())
}

#[command]
pub fn save_config(state: State<Shared>, config: AppConfig) -> Result<(), String> {
    let mut ini = Ini::load_from_file("conf.ini").unwrap_or_else(|_| Ini::new());
    apply_main_section(&mut ini, &config);
    if !config.is_summary {
        apply_pages_sections(&mut ini, &config);
    }

    ini.write_to_file("conf.ini").map_err(|e| {
        error!("Failed to write conf.ini: {}", e);
        e.to_string()
    })?;
    info!("Config saved to conf.ini");

    let mut g = state.lock().map_err(|e| e.to_string())?;
    g.config = config;
    g.reload_requested = true;
    g.sleep_requested = Some(SleepCommand::Wake);
    Ok(())
}

#[command]
pub fn get_live_preview(state: State<Shared>) -> Result<Vec<u8>, String> {
    let g = state.lock().map_err(|e| e.to_string())?;
    let buf = &g.oled_buffer;
    let mut pixels = Vec::with_capacity(128 * 64);
    for y in 0..64u32 {
        for x in 0..128u32 {
            let col = x as usize;
            let byte_row = (y / 8) as usize;
            let bit = (y % 8) as u8;
            let idx = col * 8 + byte_row;
            let on = (buf.data[idx] & (1 << bit)) != 0;
            pixels.push(if on { 255 } else { 0 });
        }
    }
    Ok(pixels)
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
        })
        .collect())
}

#[command]
pub fn list_sensors(state: State<Shared>) -> Result<Vec<SensorOption>, String> {
    let g = state.lock().map_err(|e| e.to_string())?;
    let mut out = special_sensor_options();
    if let Some(hw) = g.hwinfo_snapshot.as_ref() {
        out.extend(sensors_from_hwinfo(hw));
    }
    Ok(out)
}

#[command]
pub fn preview_config(state: State<Shared>, config: AppConfig, page: usize) -> Result<Vec<u8>, String> {
    let hwinfo_opt = {
        let g = state.lock().map_err(|e| e.to_string())?;
        g.hwinfo_snapshot.clone()
    };

    let value: Value = if config.is_summary {
        summary_preview_value(config.is_vertical)
    } else {
        let hwinfo = match hwinfo_opt.as_ref() {
            Some(h) => h,
            None => {
                let buf = render_text_to_oled("HWiNFO not\nconnected", 0);
                return Ok(buffer_to_pixels(&buf));
            }
        };

        let page_sensors = config
            .custom_sensors
            .get(page)
            .cloned()
            .unwrap_or_default();
        let props = build_preview_props(&page_sensors);

        let mut labels = vec![""; CUSTOM_SENSORS];
        let mut units = vec![""; CUSTOM_SENSORS];
        let mut values = vec![String::new(); CUSTOM_SENSORS];

        let mut mb = MouseBatteryReader::new();
        let mut media = MediaReader::new();

        if let Err(e) = run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            hwinfo,
            config.decimal,
            &mut mb,
            &mut media,
            None,
        ) {
            let buf = render_text_to_oled(&format!("Preview error:\n{}", e), 0);
            return Ok(buffer_to_pixels(&buf));
        }

        format_custom_value(config.sensors_per_line, labels, values, units)
    };

    let buf = render_text_to_oled(&value_to_preview_text(&value), 0);
    Ok(buffer_to_pixels(&buf))
}

fn buffer_to_pixels(buf: &crate::render::OledBuffer) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(128 * 64);
    for y in 0..64u32 {
        for x in 0..128u32 {
            let col = x as usize;
            let byte_row = (y / 8) as usize;
            let bit = (y % 8) as u8;
            let idx = col * 8 + byte_row;
            let on = (buf.data[idx] & (1 << bit)) != 0;
            pixels.push(if on { 255 } else { 0 });
        }
    }
    pixels
}

#[command]
pub fn request_sleep(state: State<Shared>) -> Result<(), String> {
    let mut g = state.lock().map_err(|e| e.to_string())?;
    g.sleep_requested = Some(SleepCommand::Sleep);
    Ok(())
}

#[command]
pub fn request_wake(state: State<Shared>) -> Result<(), String> {
    let mut g = state.lock().map_err(|e| e.to_string())?;
    g.sleep_requested = Some(SleepCommand::Wake);
    Ok(())
}

#[command]
pub fn request_white_screen(state: State<Shared>) -> Result<(), String> {
    let mut g = state.lock().map_err(|e| e.to_string())?;
    g.sleep_requested = Some(SleepCommand::White);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::OledBuffer;
    use crate::settings::CustomSensor;
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
        }
    }

    fn sensor(name: &str) -> CustomSensor {
        CustomSensor {
            sensor: name.to_string(),
            label: format!("L_{}", name),
            unit: "U".to_string(),
            convert: String::new(),
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
        assert_eq!(opts.len(), SPECIAL_SENSORS.len());
        for opt in &opts {
            assert_eq!(opt.category, "Special");
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
        assert!(names.contains(&"BLANK"));
        assert!(names.contains(&"MEDIA_TITLE"));
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
    fn test_apply_pages_sections_deletes_stale_pages() {
        let mut ini = Ini::new();
        ini.with_section(Some("PAGE5.Sensors")).set("sensor_0", "OLD");
        ini.with_section(Some("PAGE6.Sensors")).set("sensor_0", "OLD");
        ini.with_section(Some("UnrelatedSection")).set("key", "keep");

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
        assert_eq!(ini.section(Some("PAGE1.Sensors")).unwrap().get("sensor_0"), Some("A;1"));
        assert_eq!(ini.section(Some("PAGE2.Sensors")).unwrap().get("sensor_0"), Some("B;2"));
        assert_eq!(ini.section(Some("PAGE3.Sensors")).unwrap().get("sensor_0"), Some("C;3"));
    }

    // ==================== buffer_to_pixels ====================

    #[test]
    fn test_buffer_to_pixels_size_and_mapping() {
        let mut buf = OledBuffer::new();
        buf.set_pixel(0, 0, true);
        buf.set_pixel(127, 63, true);
        let px = buffer_to_pixels(&buf);
        assert_eq!(px.len(), 128 * 64);
        assert_eq!(px[0], 255);
        assert_eq!(px[63 * 128 + 127], 255);
        assert_eq!(px[1], 0);
    }
}

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
    ("MOUSE_BATTERY", "Wireless mouse battery %"),
    ("MEDIA_TITLE", "Now-playing track title"),
    ("MEDIA_ARTIST", "Now-playing artist"),
    ("MEDIA_ALBUM", "Now-playing album"),
    ("MEDIA_APP", "Now-playing app source"),
];

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

    let style = if !config.is_summary {
        "Custom"
    } else if config.is_vertical {
        "Vertical"
    } else {
        "Horizontal"
    };

    ini.with_section(Some("Main"))
        .set("style", style)
        .set("direct_usb", config.direct_usb.to_string())
        .set("decimal", config.decimal.to_string())
        .set("pages", config.pages.to_string())
        .set("page_time", config.page_time.to_string())
        .set("sensors_per_line", config.sensors_per_line.to_string())
        .set("gpu", &config.gpu);

    if !config.is_summary {
        // Clear stale page sections to avoid leftover sensors
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
pub fn list_sensors(state: State<Shared>) -> Result<Vec<SensorOption>, String> {
    let g = state.lock().map_err(|e| e.to_string())?;
    let mut out: Vec<SensorOption> = SPECIAL_SENSORS
        .iter()
        .map(|(name, desc)| SensorOption {
            category: "Special".to_string(),
            reading: name.to_string(),
            full_id: name.to_string(),
            is_special: true,
            description: Some(desc.to_string()),
        })
        .collect();

    if let Some(hw) = g.hwinfo_snapshot.as_ref() {
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
        // Summary mode reuses static placeholder; daemon's preview already covers live summary
        if config.is_vertical {
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

        // Build an in-memory Ini Properties for run_sensors
        let mut tmp_ini = Ini::new();
        {
            let mut section = tmp_ini.with_section(Some("PREVIEW".to_string()));
            for (k, sensor) in page_sensors.iter().enumerate() {
                if k >= CUSTOM_SENSORS { break; }
                if sensor.sensor.trim().is_empty() { continue; }
                section.set(format!("sensor_{}", k), &sensor.sensor);
                section.set(format!("label_{}", k), &sensor.label);
                section.set(format!("unit_{}", k), &sensor.unit);
                section.set(format!("convert_{}", k), &sensor.convert);
            }
        }
        let props = tmp_ini.section(Some("PREVIEW")).cloned().unwrap_or_default();

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

    let mut text = String::new();
    if let Some(obj) = value.as_object() {
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                if !text.is_empty() { text.push('\n'); }
                text.push_str(s);
            }
        }
    }
    let buf = render_text_to_oled(&text, 0);
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

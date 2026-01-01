use crate::consts::CUSTOM_SENSORS;
use crate::render::render_text_to_oled;
use crate::settings::AppConfig;
use hwinfo_steelseries_oled::Hwinfo;
use ini::Ini;
use std::sync::Mutex;
use tauri::{command, State};

pub struct GuiState {
    pub ini: Mutex<Ini>,
    pub hwinfo: Mutex<Option<Hwinfo>>,
}

#[command]
fn get_config(state: State<GuiState>) -> Result<AppConfig, String> {
    println!("Tauri: get_config called");
    let mut ini = state.ini.lock().map_err(|e| {
        println!("Tauri error: failed to lock ini: {}", e);
        e.to_string()
    })?;

    // Reload from disk to ensure we have the latest
    if let Ok(new_ini) = Ini::load_from_file("conf.ini") {
        println!("Tauri: successfully reloaded conf.ini from disk");
        *ini = new_ini;
    } else {
        println!("Tauri warning: could not reload conf.ini from disk, using current memory state");
    }

    match AppConfig::from_ini(&ini) {
        Ok(config) => {
            println!("Tauri: config successfully loaded: {:?}", config);
            Ok(config)
        }
        Err(e) => {
            println!(
                "Tauri warning: AppConfig::from_ini failed ({}), returning default",
                e
            );
            // Return a default config if parsing failed (e.g. empty/missing ini)
            Ok(AppConfig {
                is_summary: true,
                is_vertical: true,
                gpu: String::new(),
                decimal: false,
                pages: 1,
                page_time: 5,
                sensors_per_line: 1,
                direct_usb: false,
                custom_sensors: vec![vec![]],
            })
        }
    }
}

#[command]
fn save_config(state: State<GuiState>, config: AppConfig) -> Result<(), String> {
    let mut ini = state.ini.lock().map_err(|e| e.to_string())?;

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

    // Save custom sensors
    if !config.is_summary {
        for (i, page) in config.custom_sensors.iter().enumerate() {
            let section_name = format!("PAGE{}.Sensors", i + 1);
            let mut section = ini.with_section(Some(section_name));
            for (k, sensor) in page.iter().enumerate() {
                if k < CUSTOM_SENSORS {
                    section.set(format!("sensor_{}", k), &sensor.sensor);
                    section.set(format!("label_{}", k), &sensor.label);
                    section.set(format!("unit_{}", k), &sensor.unit);
                    section.set(format!("convert_{}", k), &sensor.convert);
                }
            }
        }
    }

    ini.write_to_file("conf.ini").map_err(|e| e.to_string())?;
    println!("Config saved successfully via Tauri");
    Ok(())
}

#[command]
fn get_preview(state: State<GuiState>) -> Result<Vec<u8>, String> {
    let ini = state.ini.lock().map_err(|e| e.to_string())?;

    let preview_text = match AppConfig::from_ini(&ini) {
        Ok(config) => {
            // Generate preview text based on config (reusing existing logic)
            if config.is_summary {
                if config.is_vertical {
                    "CPU   GPU   MEM\n65.5° 72.8° 16.3G\n45.2% 88.9% 15.7G".to_string()
                } else {
                    "CPU 65.5° 45.2%\nGPU 72.8° 88.9%\nMEM 16.3G 50.9%".to_string()
                }
            } else {
                // Preview first page sensors
                let mut lines = Vec::new();
                lines.push("Custom Sensors".to_string());
                lines.push("Page 1".to_string());

                if let Some(page) = config.custom_sensors.get(0) {
                    for (i, sensor) in page.iter().enumerate() {
                        if i < 3 {
                            lines.push(format!("{}: {}", sensor.label, "Value"));
                        }
                    }
                }
                lines.join("\n")
            }
        }
        Err(e) => {
            println!("Tauri: get_preview failing back to error message: {}", e);
            format!("Error loading config for preview:\n{}", e)
        }
    };

    let buffer = render_text_to_oled(&preview_text, 0);
    let mut pixels = Vec::with_capacity(128 * 64);

    for y in 0..64 {
        for x in 0..128 {
            let col = x as usize;
            let byte_row = (y / 8) as usize;
            let bit = (y % 8) as u8;
            let idx = col * 8 + byte_row;
            let on = (buffer.data[idx] & (1 << bit)) != 0;
            pixels.push(if on { 255 } else { 0 });
        }
    }

    Ok(pixels)
}

pub fn run_settings(ini: Ini, hwinfo: Option<Hwinfo>) {
    tauri::Builder::default()
        .manage(GuiState {
            ini: Mutex::new(ini),
            hwinfo: Mutex::new(hwinfo),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_preview
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

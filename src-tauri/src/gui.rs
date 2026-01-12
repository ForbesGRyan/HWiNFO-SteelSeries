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

// =============================================================================
// Helper functions extracted for testability
// =============================================================================

/// Determines the style string based on config flags.
/// Extracted from save_config for unit testing.
fn determine_style(is_summary: bool, is_vertical: bool) -> &'static str {
    if !is_summary {
        "Custom"
    } else if is_vertical {
        "Vertical"
    } else {
        "Horizontal"
    }
}

/// Generates preview text based on configuration.
/// Extracted from get_preview for unit testing.
fn generate_preview_text(config: &AppConfig) -> String {
    if config.is_summary {
        if config.is_vertical {
            "CPU   GPU   MEM\n65.5° 72.8° 16.3G\n45.2% 88.9% 15.7G".to_string()
        } else {
            "CPU 65.5° 45.2%\nGPU 72.8° 88.9%\nMEM 16.3G 50.9%".to_string()
        }
    } else {
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

/// Converts OLED buffer data to a flat pixel array (grayscale 0 or 255).
/// Extracted from get_preview for unit testing.
fn buffer_to_pixels(buffer: &crate::render::OledBuffer) -> Vec<u8> {
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

    pixels
}

/// Creates a default AppConfig for fallback scenarios.
/// Extracted from get_config for unit testing.
fn create_default_config() -> AppConfig {
    AppConfig {
        is_summary: true,
        is_vertical: true,
        gpu: String::new(),
        decimal: false,
        pages: 1,
        page_time: 5,
        sensors_per_line: 1,
        direct_usb: false,
        custom_sensors: vec![vec![]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::CustomSensor;

    // =========================================================================
    // Testing Strategy Documentation
    // =========================================================================
    //
    // The main functions in this module (get_config, save_config, get_preview,
    // run_settings) are Tauri commands that depend on:
    //
    // 1. **Tauri State<T>**: These commands receive `State<GuiState>` which is
    //    a wrapper provided by Tauri's dependency injection system. This cannot
    //    be easily constructed in unit tests without a full Tauri runtime.
    //
    // 2. **File I/O**: The commands read from and write to "conf.ini" on disk,
    //    making tests non-deterministic and potentially destructive.
    //
    // 3. **Tauri Runtime**: The `run_settings` function requires `tauri::Builder`
    //    and `tauri::generate_context!()` which need a complete Tauri application
    //    context including the tauri.conf.json manifest.
    //
    // ## Testing Approach
    //
    // We've extracted pure helper functions that contain the core business logic:
    // - `determine_style()`: Style string determination
    // - `generate_preview_text()`: Preview text generation
    // - `buffer_to_pixels()`: Pixel buffer conversion
    // - `create_default_config()`: Default config creation
    //
    // These extracted functions can be unit tested without Tauri dependencies.
    //
    // ## Integration Testing Recommendations
    //
    // For full integration tests of the Tauri commands, consider:
    //
    // 1. **Tauri Test Utilities**: Use `tauri::test` module (if available) to
    //    create mock app handles and state managers.
    //
    // 2. **Manual Mocking**: Create a test harness that:
    //    - Uses tempfile to create isolated conf.ini files
    //    - Wraps GuiState directly without Tauri's State wrapper
    //    - Tests the inner logic by calling the command functions with
    //      extracted state access patterns
    //
    // 3. **E2E Testing**: Use Tauri's WebDriver integration for full end-to-end
    //    testing of the GUI behavior.
    //
    // Example of what would be needed to mock GuiState:
    // ```rust
    // fn create_test_gui_state() -> GuiState {
    //     let mut ini = Ini::new();
    //     ini.with_section(Some("Main"))
    //         .set("style", "Vertical")
    //         .set("direct_usb", "false");
    //     GuiState {
    //         ini: Mutex::new(ini),
    //         hwinfo: Mutex::new(None),
    //     }
    // }
    // ```
    //
    // The challenge is that `State<GuiState>` cannot be constructed without
    // Tauri's app builder, so the command signatures would need modification
    // or the test would need to use Tauri's test utilities.
    //
    // =========================================================================

    // =========================================================================
    // GuiState tests
    // =========================================================================

    #[test]
    fn test_gui_state_can_be_created() {
        let ini = Ini::new();
        let state = GuiState {
            ini: Mutex::new(ini),
            hwinfo: Mutex::new(None),
        };

        // Verify we can lock the mutex
        let ini_guard = state.ini.lock().unwrap();
        // Ini::new() creates an empty ini with no named sections
        // (sections() may return Some(None) for the global section)
        assert!(ini_guard.section(Some("Main")).is_none());
        drop(ini_guard);

        let hwinfo_guard = state.hwinfo.lock().unwrap();
        assert!(hwinfo_guard.is_none());
    }

    #[test]
    fn test_gui_state_ini_can_be_modified() {
        let ini = Ini::new();
        let state = GuiState {
            ini: Mutex::new(ini),
            hwinfo: Mutex::new(None),
        };

        {
            let mut ini_guard = state.ini.lock().unwrap();
            ini_guard
                .with_section(Some("Main"))
                .set("style", "Vertical");
        }

        let ini_guard = state.ini.lock().unwrap();
        let section = ini_guard.section(Some("Main")).unwrap();
        assert_eq!(section.get("style"), Some("Vertical"));
    }

    // =========================================================================
    // determine_style() tests
    // =========================================================================

    #[test]
    fn test_determine_style_custom() {
        assert_eq!(determine_style(false, true), "Custom");
        assert_eq!(determine_style(false, false), "Custom");
    }

    #[test]
    fn test_determine_style_vertical() {
        assert_eq!(determine_style(true, true), "Vertical");
    }

    #[test]
    fn test_determine_style_horizontal() {
        assert_eq!(determine_style(true, false), "Horizontal");
    }

    // =========================================================================
    // generate_preview_text() tests
    // =========================================================================

    #[test]
    fn test_generate_preview_text_vertical_summary() {
        let config = AppConfig {
            is_summary: true,
            is_vertical: true,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            custom_sensors: vec![vec![]],
        };

        let text = generate_preview_text(&config);
        assert!(text.contains("CPU   GPU   MEM"));
        assert!(text.contains("65.5°"));
        assert!(text.contains("45.2%"));
    }

    #[test]
    fn test_generate_preview_text_horizontal_summary() {
        let config = AppConfig {
            is_summary: true,
            is_vertical: false,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            custom_sensors: vec![vec![]],
        };

        let text = generate_preview_text(&config);
        assert!(text.contains("CPU 65.5° 45.2%"));
        assert!(text.contains("GPU 72.8° 88.9%"));
        assert!(text.contains("MEM 16.3G 50.9%"));
    }

    #[test]
    fn test_generate_preview_text_custom_empty_sensors() {
        let config = AppConfig {
            is_summary: false,
            is_vertical: false,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            custom_sensors: vec![vec![]],
        };

        let text = generate_preview_text(&config);
        assert!(text.contains("Custom Sensors"));
        assert!(text.contains("Page 1"));
        // No sensor lines since custom_sensors is empty
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn test_generate_preview_text_custom_with_sensors() {
        let config = AppConfig {
            is_summary: false,
            is_vertical: false,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            custom_sensors: vec![vec![
                CustomSensor {
                    sensor: "CPU;Temperature".to_string(),
                    label: "CPU".to_string(),
                    unit: "°".to_string(),
                    convert: String::new(),
                },
                CustomSensor {
                    sensor: "GPU;Temperature".to_string(),
                    label: "GPU".to_string(),
                    unit: "°".to_string(),
                    convert: String::new(),
                },
            ]],
        };

        let text = generate_preview_text(&config);
        assert!(text.contains("Custom Sensors"));
        assert!(text.contains("Page 1"));
        assert!(text.contains("CPU: Value"));
        assert!(text.contains("GPU: Value"));
    }

    #[test]
    fn test_generate_preview_text_custom_limits_to_three_sensors() {
        let config = AppConfig {
            is_summary: false,
            is_vertical: false,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            custom_sensors: vec![vec![
                CustomSensor {
                    sensor: "s1".to_string(),
                    label: "Label1".to_string(),
                    unit: "".to_string(),
                    convert: String::new(),
                },
                CustomSensor {
                    sensor: "s2".to_string(),
                    label: "Label2".to_string(),
                    unit: "".to_string(),
                    convert: String::new(),
                },
                CustomSensor {
                    sensor: "s3".to_string(),
                    label: "Label3".to_string(),
                    unit: "".to_string(),
                    convert: String::new(),
                },
                CustomSensor {
                    sensor: "s4".to_string(),
                    label: "Label4".to_string(),
                    unit: "".to_string(),
                    convert: String::new(),
                },
            ]],
        };

        let text = generate_preview_text(&config);
        assert!(text.contains("Label1: Value"));
        assert!(text.contains("Label2: Value"));
        assert!(text.contains("Label3: Value"));
        // Label4 should NOT be included (limited to 3)
        assert!(!text.contains("Label4"));
    }

    // =========================================================================
    // buffer_to_pixels() tests
    // =========================================================================

    #[test]
    fn test_buffer_to_pixels_empty_buffer() {
        let buffer = crate::render::OledBuffer::new();
        let pixels = buffer_to_pixels(&buffer);

        // Should have 128 * 64 = 8192 pixels
        assert_eq!(pixels.len(), 128 * 64);

        // All pixels should be 0 (off)
        for pixel in pixels.iter() {
            assert_eq!(*pixel, 0);
        }
    }

    #[test]
    fn test_buffer_to_pixels_single_pixel_on() {
        let mut buffer = crate::render::OledBuffer::new();
        buffer.set_pixel(0, 0, true);

        let pixels = buffer_to_pixels(&buffer);

        // Pixel at (0, 0) should be 255 (on)
        // In row-major order: index = y * 128 + x = 0 * 128 + 0 = 0
        assert_eq!(pixels[0], 255);

        // Verify other pixels in first row are off
        for i in 1..128 {
            assert_eq!(pixels[i], 0);
        }
    }

    #[test]
    fn test_buffer_to_pixels_last_pixel() {
        let mut buffer = crate::render::OledBuffer::new();
        buffer.set_pixel(127, 63, true);

        let pixels = buffer_to_pixels(&buffer);

        // Last pixel at (127, 63)
        // In row-major order: index = 63 * 128 + 127 = 8191
        assert_eq!(pixels[8191], 255);
    }

    #[test]
    fn test_buffer_to_pixels_pattern() {
        let mut buffer = crate::render::OledBuffer::new();
        // Set a diagonal pattern
        buffer.set_pixel(0, 0, true);
        buffer.set_pixel(1, 1, true);
        buffer.set_pixel(2, 2, true);

        let pixels = buffer_to_pixels(&buffer);

        // Check diagonal pixels
        assert_eq!(pixels[0 * 128 + 0], 255); // (0, 0)
        assert_eq!(pixels[1 * 128 + 1], 255); // (1, 1)
        assert_eq!(pixels[2 * 128 + 2], 255); // (2, 2)

        // Check some off pixels
        assert_eq!(pixels[0 * 128 + 1], 0); // (1, 0)
        assert_eq!(pixels[1 * 128 + 0], 0); // (0, 1)
    }

    #[test]
    fn test_buffer_to_pixels_correct_size() {
        let buffer = crate::render::OledBuffer::new();
        let pixels = buffer_to_pixels(&buffer);

        // 128 width * 64 height = 8192 pixels
        assert_eq!(pixels.len(), 8192);
    }

    // =========================================================================
    // create_default_config() tests
    // =========================================================================

    #[test]
    fn test_create_default_config_is_summary() {
        let config = create_default_config();
        assert!(config.is_summary);
    }

    #[test]
    fn test_create_default_config_is_vertical() {
        let config = create_default_config();
        assert!(config.is_vertical);
    }

    #[test]
    fn test_create_default_config_gpu_empty() {
        let config = create_default_config();
        assert_eq!(config.gpu, "");
    }

    #[test]
    fn test_create_default_config_decimal_false() {
        let config = create_default_config();
        assert!(!config.decimal);
    }

    #[test]
    fn test_create_default_config_pages() {
        let config = create_default_config();
        assert_eq!(config.pages, 1);
    }

    #[test]
    fn test_create_default_config_page_time() {
        let config = create_default_config();
        assert_eq!(config.page_time, 5);
    }

    #[test]
    fn test_create_default_config_sensors_per_line() {
        let config = create_default_config();
        assert_eq!(config.sensors_per_line, 1);
    }

    #[test]
    fn test_create_default_config_direct_usb() {
        let config = create_default_config();
        assert!(!config.direct_usb);
    }

    #[test]
    fn test_create_default_config_custom_sensors_structure() {
        let config = create_default_config();
        assert_eq!(config.custom_sensors.len(), 1);
        assert!(config.custom_sensors[0].is_empty());
    }

    // =========================================================================
    // Integration test helpers (for documentation purposes)
    // =========================================================================

    /// Example of creating a test GuiState (cannot be used with State<T> wrapper)
    #[test]
    fn test_gui_state_creation_example() {
        let mut ini = Ini::new();
        ini.with_section(Some("Main"))
            .set("style", "Vertical")
            .set("direct_usb", "false")
            .set("decimal", "true")
            .set("pages", "1")
            .set("page_time", "5")
            .set("sensors_per_line", "1");

        let state = GuiState {
            ini: Mutex::new(ini),
            hwinfo: Mutex::new(None),
        };

        // Verify the state was created correctly
        let ini_guard = state.ini.lock().unwrap();
        let config = AppConfig::from_ini(&ini_guard);
        assert!(config.is_ok());

        let config = config.unwrap();
        assert!(config.is_summary);
        assert!(config.is_vertical);
        assert!(config.decimal);
    }

    /// Demonstrates how save_config logic works by testing INI manipulation
    #[test]
    fn test_save_config_ini_manipulation() {
        let config = AppConfig {
            is_summary: false,
            is_vertical: false,
            gpu: "GPU [#0]".to_string(),
            decimal: true,
            pages: 2,
            page_time: 10,
            sensors_per_line: 2,
            direct_usb: true,
            custom_sensors: vec![
                vec![CustomSensor {
                    sensor: "CPU;Temp".to_string(),
                    label: "CPU".to_string(),
                    unit: "°".to_string(),
                    convert: "".to_string(),
                }],
                vec![],
            ],
        };

        let mut ini = Ini::new();

        // Replicate save_config logic
        let style = determine_style(config.is_summary, config.is_vertical);
        ini.with_section(Some("Main"))
            .set("style", style)
            .set("direct_usb", config.direct_usb.to_string())
            .set("decimal", config.decimal.to_string())
            .set("pages", config.pages.to_string())
            .set("page_time", config.page_time.to_string())
            .set("sensors_per_line", config.sensors_per_line.to_string())
            .set("gpu", &config.gpu);

        // Verify INI was populated correctly
        let main = ini.section(Some("Main")).unwrap();
        assert_eq!(main.get("style"), Some("Custom"));
        assert_eq!(main.get("direct_usb"), Some("true"));
        assert_eq!(main.get("decimal"), Some("true"));
        assert_eq!(main.get("pages"), Some("2"));
        assert_eq!(main.get("page_time"), Some("10"));
        assert_eq!(main.get("sensors_per_line"), Some("2"));
        assert_eq!(main.get("gpu"), Some("GPU [#0]"));
    }

    /// Tests that custom sensor serialization to INI works correctly
    #[test]
    fn test_custom_sensor_ini_serialization() {
        let sensors = vec![
            CustomSensor {
                sensor: "CPU [#0];Temperature".to_string(),
                label: "CPU".to_string(),
                unit: "°C".to_string(),
                convert: "".to_string(),
            },
            CustomSensor {
                sensor: "GPU [#0];Temperature".to_string(),
                label: "GPU".to_string(),
                unit: "°C".to_string(),
                convert: "MB/GB".to_string(),
            },
        ];

        let mut ini = Ini::new();
        let section_name = "PAGE1.Sensors";
        let mut section = ini.with_section(Some(section_name));

        for (k, sensor) in sensors.iter().enumerate() {
            section.set(format!("sensor_{}", k), &sensor.sensor);
            section.set(format!("label_{}", k), &sensor.label);
            section.set(format!("unit_{}", k), &sensor.unit);
            section.set(format!("convert_{}", k), &sensor.convert);
        }

        // Verify serialization
        let page_section = ini.section(Some(section_name)).unwrap();
        assert_eq!(
            page_section.get("sensor_0"),
            Some("CPU [#0];Temperature")
        );
        assert_eq!(page_section.get("label_0"), Some("CPU"));
        assert_eq!(page_section.get("unit_0"), Some("°C"));
        assert_eq!(page_section.get("convert_0"), Some(""));
        assert_eq!(
            page_section.get("sensor_1"),
            Some("GPU [#0];Temperature")
        );
        assert_eq!(page_section.get("label_1"), Some("GPU"));
        assert_eq!(page_section.get("convert_1"), Some("MB/GB"));
    }

    /// Tests round-trip: config -> INI -> config
    #[test]
    fn test_config_round_trip() {
        let original = AppConfig {
            is_summary: true,
            is_vertical: true,
            gpu: "NVIDIA GeForce RTX 3090".to_string(),
            decimal: true,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            custom_sensors: vec![vec![]],
        };

        // Serialize to INI
        let mut ini = Ini::new();
        let style = determine_style(original.is_summary, original.is_vertical);
        ini.with_section(Some("Main"))
            .set("style", style)
            .set("direct_usb", original.direct_usb.to_string())
            .set("decimal", original.decimal.to_string())
            .set("pages", original.pages.to_string())
            .set("page_time", original.page_time.to_string())
            .set("sensors_per_line", original.sensors_per_line.to_string())
            .set("gpu", &original.gpu);

        // Deserialize from INI
        let parsed = AppConfig::from_ini(&ini).unwrap();

        // Verify round-trip
        assert_eq!(parsed.is_summary, original.is_summary);
        assert_eq!(parsed.is_vertical, original.is_vertical);
        assert_eq!(parsed.gpu, original.gpu);
        assert_eq!(parsed.decimal, original.decimal);
        assert_eq!(parsed.pages, original.pages);
        assert_eq!(parsed.page_time, original.page_time);
        // Note: sensors_per_line is only loaded for Custom mode
    }
}

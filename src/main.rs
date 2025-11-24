mod settings;
use ini::Ini;
use settings::{settings_create_config, AppConfig};

mod consts;
use consts::*;

mod connect;
use connect::{connect_hwinfo, connect_steelseries};

mod console_utils;
use console_utils::{console_window, display_value_in_console, Console};

mod steelseries;
use steelseries::page_handler;

mod utils;
use utils::{format_custom_value, run_sensors};

use anyhow;
use console::Term;
use hwinfo_steelseries_oled::Hwinfo;
use image::ImageReader;
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use std::io::Cursor;
use std::num::Wrapping;
use tray_icon::{Icon, TrayIconBuilder};

// Summary sensors data
struct SummarySensors {
    cpu_temp: f64,
    cpu_usage: f64,
    gpu_temp: f64,
    gpu_usage: f64,
    mem_used: f64,
    mem_free: f64,
    mem_load: f64,
}

fn fetch_summary_sensors(hwinfo: &Hwinfo, gpu_name: &str) -> Result<SummarySensors, anyhow::Error> {
    let sensor_cpu_usage = hwinfo.find_first("Total CPU Usage")?;
    let sensor_cpu_temp = hwinfo.find_first("CPU (Tctl/Tdie)")?;
    let sensor_gpu_usage = hwinfo.find_first("GPU Core Load")?;

    let sensor_gpu_temp = if gpu_name.is_empty() {
        hwinfo.find_first("GPU Temperature")?
    } else {
        hwinfo
            .get(gpu_name, "GPU Temperature")
            .ok_or_else(|| anyhow::anyhow!("GPU Temperature not found"))?
    };

    let sensor_mem_used = hwinfo.find_first("Physical Memory Used")?;
    let sensor_mem_free = hwinfo.find_first("Physical Memory Available")?;
    let sensor_mem_load = hwinfo.find_first("Physical Memory Load")?;

    Ok(SummarySensors {
        cpu_temp: sensor_cpu_temp.value,
        cpu_usage: sensor_cpu_usage.value,
        gpu_temp: sensor_gpu_temp.value,
        gpu_usage: sensor_gpu_usage.value,
        mem_used: sensor_mem_used.value / 1024.0,
        mem_free: sensor_mem_free.value / 1024.0,
        mem_load: sensor_mem_load.value,
    })
}

fn format_vertical_summary(sensors: &SummarySensors, decimal: bool) -> Value {
    let precision = if decimal { 1 } else { 0 };
    let spacing = if decimal { " " } else { "   " };
    let spacing2 = if decimal { " " } else { "    " };

    json!({
        "line1": "CPU   GPU   MEM",
        "line2": format!("{:.prec$}°{}{:.prec$}°{}{:.prec$}G",
            sensors.cpu_temp,
            spacing,
            sensors.gpu_temp,
            spacing,
            sensors.mem_used,
            prec = precision),
        "line3": format!("{:.prec$}%{}{:.prec$}%{}{:.prec$}G",
            sensors.cpu_usage,
            spacing2,
            sensors.gpu_usage,
            spacing2,
            sensors.mem_free,
            prec = precision),
    })
}

fn format_horizontal_summary(sensors: &SummarySensors, decimal: bool) -> Value {
    let precision = if decimal { 1 } else { 0 };

    json!({
        "line1": format!("CPU {:.prec$}° {:.prec$}%",
            sensors.cpu_temp,
            sensors.cpu_usage,
            prec = precision),
        "line2": format!("GPU {:.prec$}° {:.prec$}%",
            sensors.gpu_temp,
            sensors.gpu_usage,
            prec = precision),
        "line3": format!("MEM {:.prec$}G {:.prec$}%",
            sensors.mem_used,
            sensors.mem_load,
            prec = precision),
    })
}

fn check_hwinfo_connection(
    old: &Hwinfo,
    new: &Hwinfo,
    disconnect_count: &mut usize,
    limit: usize,
) -> bool {
    if old == new {
        *disconnect_count = (*disconnect_count + 1).min(limit);
    } else {
        *disconnect_count = 0;
    }
    *disconnect_count >= limit
}

fn handle_fatal_error(term: &Term, err: anyhow::Error) -> anyhow::Error {
    error!("Fatal error occurred: {}", err);

    // Show console window to ensure user can see the error
    console_window(Console::SHOW);

    // Display error message
    let _ = term.write_line("");
    let _ = term.write_line("=================================");
    let _ = term.write_line("ERROR: Application stopped");
    let _ = term.write_line("=================================");
    let _ = term.write_line(&format!("{}", err));

    // Show the cause chain if available
    let mut source = err.source();
    if source.is_some() {
        let _ = term.write_line("");
        let _ = term.write_line("Caused by:");
    }
    while let Some(cause) = source {
        let _ = term.write_line(&format!("  {}", cause));
        source = cause.source();
    }

    let _ = term.write_line("");
    let _ = term.write_line("Press Enter to exit...");

    // Wait for user input
    let _ = std::io::stdin().read_line(&mut String::new());

    err
}

#[allow(unreachable_code)]
fn main() -> Result<(), anyhow::Error> {
    // Initialize logger - set RUST_LOG environment variable to control log level
    // e.g., RUST_LOG=debug, RUST_LOG=info, RUST_LOG=warn, RUST_LOG=error
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("Starting HWiNFO-SteelSeries application");

    let term = Term::stdout();

    // Run the application and handle fatal errors
    if let Err(e) = run_application(&term) {
        return Err(handle_fatal_error(&term, e));
    }

    Ok(())
}

#[allow(unreachable_code)]
fn run_application(term: &Term) -> Result<(), anyhow::Error> {
    // Embed the icon at compile time
    const ICON_DATA: &[u8] = include_bytes!("../assets/hwinfo-steelseries-icon.ico");

    let mut tray_builder = TrayIconBuilder::new().with_tooltip("HWiNFO-SteelSeries");

    // Decode the embedded ICO file
    let icon_result = ImageReader::new(Cursor::new(ICON_DATA))
        .with_guessed_format()
        .map_err(|e| format!("Format error: {}", e))
        .and_then(|reader| reader.decode().map_err(|e| format!("Decode error: {}", e)));

    match icon_result {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            match Icon::from_rgba(rgba.into_raw(), width, height) {
                Ok(icon) => {
                    info!(
                        "Successfully loaded embedded tray icon ({}x{})",
                        width, height
                    );
                    tray_builder = tray_builder.with_icon(icon);
                }
                Err(e) => {
                    warn!("Failed to create icon from RGBA data: {}", e);
                }
            }
        }
        Err(e) => {
            warn!(
                "Failed to decode embedded icon (continuing without icon): {}",
                e
            );
        }
    }

    let _tray = tray_builder
        .build()
        .map_err(|e| {
            warn!("Failed to build tray icon (continuing without icon): {}", e);
        })
        .ok();

    let mut client = connect_steelseries(term)?;

    let mut hwinfo = connect_hwinfo(term)?;
    hwinfo.pull().map_err(|e| {
        error!("Failed to pull initial HWiNFO data: {}", e);
        e
    })?;

    info!("Loading configuration from conf.ini");
    let config_file = match Ini::load_from_file("conf.ini") {
        Ok(conf) => {
            info!("Configuration file loaded successfully");
            conf
        }
        Err(err) => {
            warn!("Configuration file not found: {}. Creating new config", err);
            settings_create_config(term, &hwinfo)?
        }
    };

    let config = AppConfig::from_ini(&config_file).map_err(|e| {
        error!("Failed to parse configuration: {}", e);
        e
    })?;
    info!("Configuration parsed successfully");

    #[cfg(debug_assertions)]
    let display_in_console = true;
    #[cfg(not(debug_assertions))]
    let display_in_console = false;

    info!("Setting up {} page(s)", config.pages);
    let mut pages_vec = Vec::new();
    for i in 1..=config.pages {
        // Bind the event handler regardless of whether we're in summary or custom mode
        let handler = page_handler(3, "line1", "line2", "line3", None);
        client
            .bind_event(
                format!("PAGE{}", i).as_str(),
                None,
                None,
                None,
                None,
                vec![handler],
            )
            .map_err(|e| {
                error!("Failed to bind event for PAGE{}: {}", i, e);
                e
            })?;
        info!("Successfully bound event for PAGE{}", i);

        // For custom mode, store the sensor configuration
        if !config.is_summary {
            match config_file.section(Some(format!("PAGE{}.Sensors", i))) {
                Some(page) => {
                    pages_vec.push(page);
                }
                None => {
                    warn!("PAGE{}.Sensors section not found in config", i);
                    continue;
                }
            };
        }
    }

    info!("Starting heartbeat");
    client.start_heartbeat();
    info!("Entering main loop");

    // Hide console window in release mode after successful startup
    #[cfg(not(debug_assertions))]
    {
        std::thread::sleep(std::time::Duration::from_millis(500));
        console_window(Console::HIDE);
    }

    let mut i = Wrapping(0isize);
    let mut disconnect_count: usize = 0;
    let mut page_counter: usize = 0;
    let mut was_disconnected = false;
    loop {
        let old = hwinfo.clone();
        if let Err(e) = hwinfo.pull() {
            error!("Error pulling HWiNFO data: {}", e);
            return Err(e);
        }

        let disconnected = check_hwinfo_connection(&old, &hwinfo, &mut disconnect_count, 5);
        drop(old);

        // Hide console when reconnected (transitioned from disconnected to connected)
        if was_disconnected && !disconnected {
            #[cfg(not(debug_assertions))]
            console_window(Console::HIDE);
        }
        was_disconnected = disconnected;

        if disconnected {
            warn!("Disconnected from HWiNFO (no data updates for 5 cycles)");
            console_window(Console::SHOW);
            term.clear_line()?;
            term.write_line("Disconnected from HWiNFO")?;
            let value = json!({
                "line1": "Disconnected",
                "line2": "FROM",
                "line3": "HWiNFO"
            });
            if let Err(e) = client.trigger_event_frame("ERROR", i.0, value) {
                error!("Failed to trigger error frame: {}", e);
            }
            i += 1;
            std::thread::sleep(std::time::Duration::from_millis(TICK_RATE));
            continue;
        }

        let value = if config.is_summary {
            match fetch_summary_sensors(&hwinfo, config.gpu) {
                Ok(sensors) => {
                    if config.is_vertical {
                        format_vertical_summary(&sensors, config.decimal)
                    } else {
                        format_horizontal_summary(&sensors, config.decimal)
                    }
                }
                Err(e) => {
                    error!("Failed to fetch summary sensors: {}", e);
                    return Err(e);
                }
            }
        } else {
            // Custom Sensors - Logic to alternate between pages
            if i.0 % config.page_time == 0 && i.0 != 0 {
                page_counter = (page_counter + 1) % config.pages;
                debug!("Switching to page {}", page_counter + 1);
            }
            let pages_sensors = pages_vec[page_counter];

            let mut labels = vec![""; CUSTOM_SENSORS];
            let mut units = vec![""; CUSTOM_SENSORS];
            let mut values = vec![String::new(); CUSTOM_SENSORS];

            if let Err(e) = run_sensors(
                pages_sensors,
                &mut labels,
                &mut units,
                &mut values,
                &hwinfo,
                config.decimal,
            ) {
                error!("Failed to run sensors for page {}: {}", page_counter + 1, e);
                return Err(e);
            }
            format_custom_value(config.sensors_per_line, labels, values, units)
        };
        if display_in_console {
            display_value_in_console(term, &value)?;
        }
        if let Err(e) =
            client.trigger_event_frame(format!("PAGE{}", page_counter + 1).as_str(), i.0, value)
        {
            error!(
                "Failed to trigger event frame for PAGE{}: {}",
                page_counter + 1,
                e
            );
            return Err(e);
        }
        i += 1;
        std::thread::sleep(std::time::Duration::from_millis(TICK_RATE));
    }
    client.stop_heartbeat()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_sensors() -> SummarySensors {
        SummarySensors {
            cpu_temp: 65.5,
            cpu_usage: 45.2,
            gpu_temp: 72.8,
            gpu_usage: 88.9,
            mem_used: 16.3,
            mem_free: 15.7,
            mem_load: 50.9,
        }
    }

    #[test]
    fn test_format_vertical_summary_with_decimal() {
        let sensors = create_test_sensors();
        let result = format_vertical_summary(&sensors, true);

        assert_eq!(result["line1"], "CPU   GPU   MEM");
        assert_eq!(result["line2"], "65.5° 72.8° 16.3G");
        assert_eq!(result["line3"], "45.2% 88.9% 15.7G");
    }

    #[test]
    fn test_format_vertical_summary_without_decimal() {
        let sensors = create_test_sensors();
        let result = format_vertical_summary(&sensors, false);

        assert_eq!(result["line1"], "CPU   GPU   MEM");
        assert_eq!(result["line2"], "66°   73°   16G");
        assert_eq!(result["line3"], "45%    89%    16G");
    }

    #[test]
    fn test_format_horizontal_summary_with_decimal() {
        let sensors = create_test_sensors();
        let result = format_horizontal_summary(&sensors, true);

        assert_eq!(result["line1"], "CPU 65.5° 45.2%");
        assert_eq!(result["line2"], "GPU 72.8° 88.9%");
        assert_eq!(result["line3"], "MEM 16.3G 50.9%");
    }

    #[test]
    fn test_format_horizontal_summary_without_decimal() {
        let sensors = create_test_sensors();
        let result = format_horizontal_summary(&sensors, false);

        assert_eq!(result["line1"], "CPU 66° 45%");
        assert_eq!(result["line2"], "GPU 73° 89%");
        assert_eq!(result["line3"], "MEM 16G 51%");
    }

    // Note: check_hwinfo_connection is tested indirectly through the equality
    // implementation tested in lib.rs. The logic is simple: if hwinfo structs
    // are equal (unchanged readings), increment disconnect_count, else reset to 0.
    // Since we can't easily create mock Hwinfo in main.rs tests due to private
    // fields, we rely on lib.rs tests for Hwinfo equality verification.
}

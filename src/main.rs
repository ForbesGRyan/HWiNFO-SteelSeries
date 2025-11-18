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

use console::Term;
use hwinfo_steelseries_oled::Hwinfo;
use serde_json::{json, Value};
use std::num::Wrapping;
use tray_icon::{Icon, TrayIconBuilder};
use anyhow;

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

fn fetch_summary_sensors(
    hwinfo: &Hwinfo,
    gpu_name: &str,
) -> Result<SummarySensors, anyhow::Error> {
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

fn check_hwinfo_connection(old: &Hwinfo, new: &Hwinfo, disconnect_count: &mut usize, limit: usize) -> bool {
    if old == new {
        *disconnect_count = (*disconnect_count + 1).min(limit);
    } else {
        *disconnect_count = 0;
    }
    *disconnect_count >= limit
}

#[allow(unreachable_code)]
fn main() -> Result<(), anyhow::Error> {
    let icon = Icon::from_path("assets/hwinfo-steelseries-icon.ico", Some((64, 64)))?;
    let _tray = TrayIconBuilder::new()
        .with_tooltip("HWiNFO-SteelSeries")
        .with_icon(icon)
        .build()?;

    let term = Term::stdout();

    let mut client = connect_steelseries(&term)?;

    let mut hwinfo = connect_hwinfo(&term)?;
    hwinfo.pull()?;

    let config_file = match Ini::load_from_file("conf.ini") {
        Ok(conf) => conf,
        Err(_err) => settings_create_config(&term, &hwinfo)?,
    };

    let config = AppConfig::from_ini(&config_file)?;

    #[cfg(debug_assertions)]
    let display_in_console = true;
    #[cfg(not(debug_assertions))]
    let display_in_console = false;

    let mut pages_vec = Vec::new();
    for i in 1..=config.pages {
        match config_file.section(Some(format!("PAGE{}.Sensors", i))) {
            Some(page) => {
                let handler = page_handler(3, "line1", "line2", "line3", None);
                client.bind_event(
                    format!("PAGE{}", i).as_str(),
                    None,
                    None,
                    None,
                    None,
                    vec![handler],
                )?;
                pages_vec.push(page);
            }
            None => continue,
        };
    }

    client.start_heartbeat();
    let mut i = Wrapping(0isize);
    let mut disconnect_count: usize = 0;
    let mut page_counter: usize = 0;
    loop {
        // Logic to alternate between pages
        if i.0 % config.page_time == 0 && i.0 != 0 {
            page_counter = (page_counter + 1) % config.pages;
        }
        let pages_sensors = pages_vec[page_counter];

        let old = hwinfo.clone();
        hwinfo.pull()?;

        let disconnected = check_hwinfo_connection(&old, &hwinfo, &mut disconnect_count, 5);
        drop(old);

        if disconnected {
            console_window(Console::SHOW);
            term.clear_line()?;
            term.write_line("Disconnected from HWiNFO")?;
            let value = json!({
                "line1": "Disconnected",
                "line2": "FROM",
                "line3": "HWiNFO"
            });
            client.trigger_event_frame("ERROR", i.0, value)?;
            i += 1;
            std::thread::sleep(std::time::Duration::from_millis(TICK_RATE));
            continue;
        }

        let value = if config.is_summary {
            let sensors = fetch_summary_sensors(&hwinfo, config.gpu)?;
            if config.is_vertical {
                format_vertical_summary(&sensors, config.decimal)
            } else {
                format_horizontal_summary(&sensors, config.decimal)
            }
        } else {
            // Custom Sensors
            let mut labels = vec![""; CUSTOM_SENSORS];
            let mut units = vec![""; CUSTOM_SENSORS];
            let mut values = vec![String::new(); CUSTOM_SENSORS];

            run_sensors(
                pages_sensors,
                &mut labels,
                &mut units,
                &mut values,
                &hwinfo,
                config.decimal,
            )?;
            format_custom_value(config.sensors_per_line, labels, values, units)
        };
        if display_in_console {
            display_value_in_console(&term, &value)?;
        }
        client.trigger_event_frame(format!("PAGE{}", page_counter + 1).as_str(), i.0, value)?;
        i += 1;
        std::thread::sleep(std::time::Duration::from_millis(TICK_RATE));
    }
    client.stop_heartbeat()?;

    Ok(())
}

mod settings;
use ini::Ini;
use settings::settings_create_config;

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
use hwinfo_steelseries_oled::{Hwinfo, HwinfoSensorsReadingElement};
use serde_json::json;
use std::num::Wrapping;
use tray_icon::{Icon, TrayIconBuilder};

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

    let config = match Ini::load_from_file("conf.ini") {
        Ok(conf) => conf,
        Err(_err) => settings_create_config(&term, &hwinfo)?,
    };

    let config_main = match config.section(Some("Main")) {
        Some(main) => main,
        None => {
            return Err(anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Config Not found",
            )))
        }
    };
    // TODO: will error when using summary without a section for sensors
    // let config_sensors = match config.section(Some("PAGE1.Sensors")) {
    //     Some(sensors) => sensors,
    //     None => {
    //         return Err(anyhow::Error::new(std::io::Error::new(
    //             std::io::ErrorKind::NotFound,
    //             "Sensors Config Not found",
    //         )))
    //     }
    // };

    // std::thread::sleep(std::time::Duration::from_secs(1));
    // console_window(Console::HIDE);

    let style = match config_main.get("style") {
        Some(style) => style.to_lowercase(),
        None => {
            return Err(anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Style not found",
            )))
        }
    };
    let vertical = match style.as_str() {
        "vertical" => Some(true),
        "horizontal" => Some(false),
        _ => None,
    };
    let summary = match style.as_str() {
        "vertical" | "horizontal" => true,
        _ => false,
    };

    let mut gpu: &str = "";
    if summary {
        gpu = match config_main.get("gpu") {
            Some(gpu) => gpu,
            None => "",
        };
    }

    let decimal = match config_main.get("decimal") {
        Some(decimal) => decimal.parse::<bool>()?,
        None => false,
    };

    #[cfg(debug_assertions)]
    let display_in_console = true;
    #[cfg(not(debug_assertions))]
    let display_in_console = false;

    let pages = match config_main.get("pages") {
        Some(pages) => pages.parse::<usize>()?,
        None => 1,
    };
    let mut pages_vec = Vec::new();
    for i in 1..=pages {
        match config.section(Some(format!("PAGE{}.Sensors", i))) {
            Some(page) => {
                // client.register_event(format!("PAGE{}", i).as_str())?;
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

    // let handler = page_handler(3, "line1", "line2", "line3", None);

    // client.register_event_full("MAIN", None, None, None, Some(true))?;

    // client.bind_event("MAIN", None, None, None, None, vec![handler])?;
    client.start_heartbeat();
    let mut i = Wrapping(0isize);
    let mut count: usize = 0;
    let mut page_counter: usize = 0;
    let page_time = match config_main.get("page_time") {
        Some(second) => {
            let num = second.parse::<isize>()?;
            match num {
                0..=60 => num,
                _ => 5,
            }
        }
        None => 5,
    };
    loop {
        // Logic to alternate between pages
        if i.0 % page_time == 0 && i.0 != 0 {
            if page_counter >= pages - 1 {
                page_counter = 0;
            } else {
                page_counter += 1;
            }
        }
        let pages_sensors = pages_vec[page_counter];

        let limit = 5;
        let old = hwinfo.clone();
        hwinfo.pull()?;
        if old == hwinfo {
            if count < limit {
                count += 1;
            }
        } else {
            count = 0;
            // console_window(Console::HIDE);
        }
        drop(old);
        #[allow(unused_assignments)]
        let mut value = json!("");
        if count >= limit {
            console_window(Console::SHOW);
            term.clear_line()?;
            term.write_line("Disconnected from HWiNFO")?;
            value = json!({"line1":"Disconnected",
                           "line2":"FROM",
                           "line3":"HWiNFO"});
            client.trigger_event_frame("ERROR", i.0, value)?;
            i += 1;
            std::thread::sleep(std::time::Duration::from_millis(TICK_RATE));
            continue;
        }

        if summary {
            let mut labels: Vec<&str> = vec!["CPU", "", "GPU", "", "MEM", ""];
            let mut units: Vec<&str> = vec!["°", "%", "°", "%", "MB", "MB"];
            let mut values: Vec<String> = vec![String::new(); CUSTOM_SENSORS];
            let sensors_per_line: u8 = 2;

            let sensor_cpu_usage = hwinfo.find_first("Total CPU Usage")?;
            let sensor_cpu_temp = hwinfo.find_first("CPU (Tctl/Tdie)")?;

            let sensor_gpu_usage = hwinfo.find_first("GPU Core Load")?;
            let sensor_gpu_temp: &HwinfoSensorsReadingElement;
            if gpu == "" {
                sensor_gpu_temp = hwinfo.find_first("GPU Temperature")?;
            } else {
                sensor_gpu_temp = match hwinfo.get(gpu, "GPU Temperature") {
                    Some(sensor) => sensor,
                    None => {
                        return Err(anyhow::Error::new(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "GPU Temperature not found",
                        )))
                    }
                };
            }

            let sensor_mem_used = hwinfo.find_first("Physical Memory Used")?;
            let sensor_mem_free = hwinfo.find_first("Physical Memory Available")?;
            let sensor_mem_load = hwinfo.find_first("Physical Memory Load")?;
            let cpu_temp_cur_value = sensor_cpu_temp.value;
            let cpu_usage_cur_value = sensor_cpu_usage.value;
            let temp_unit = "°"; 
            let usage_unit = "%";
            let gpu_temp_cur_value = sensor_gpu_temp.value;
            let gpu_usage_cur_value = sensor_gpu_usage.value;
            let mem_unit = "G";
            let mem_used = sensor_mem_used.value / 1024.0;
            let mem_free = sensor_mem_free.value / 1024.0;
            let mem_load = sensor_mem_load.value;
            let line1_spaces = " ";
            let line2_spaces = " ";

            if vertical.unwrap_or(true) {
                if decimal {
                    value = json!({
                        "line1": "CPU   GPU   MEM",
                        "line2": format!("{:.1}{}{}{:.1}{}{}{:.1}{}",
                            cpu_temp_cur_value, temp_unit,
                            line1_spaces,
                            gpu_temp_cur_value, temp_unit,
                            line1_spaces,
                            mem_used, mem_unit),
                        "line3": format!("{:.1}{}{}{:.1}{}{}{:.1}{}",
                            cpu_usage_cur_value, usage_unit,
                            line2_spaces,
                            gpu_usage_cur_value, usage_unit,
                            line2_spaces,
                            mem_free, mem_unit),
                    });
                } else {
                    value = json!({
                        "line1": "CPU   GPU   MEM",
                        "line2": format!("{:.0}{}{}{:.0}{}{}{:.0}{}",
                            cpu_temp_cur_value, temp_unit,
                            "   ",
                            gpu_temp_cur_value, temp_unit,
                            "   ",
                            mem_used, mem_unit),
                        "line3": format!("{:.0}{}{}{:.0}{}{}{:.0}{}",
                            cpu_usage_cur_value, usage_unit,
                            "    ",
                            gpu_usage_cur_value, usage_unit,
                            "    ",
                            mem_free, mem_unit),
                    });
                }
            } else {
                // Horizontal
                if decimal {
                    value = json!({
                        "line1": format!("CPU {:.1}{} {:.1}{}",
                            cpu_temp_cur_value, temp_unit,
                            cpu_usage_cur_value, usage_unit),
                        "line2": format!("GPU {:.1}{} {:.1}{}",
                            gpu_temp_cur_value, temp_unit,
                            gpu_usage_cur_value, usage_unit),
                        "line3": format!("MEM {:.1}{} {:.1}{}",
                            mem_used, mem_unit,
                            mem_load, usage_unit,
                            // mem_free, mem_unit.to_lowercase()
                        ),
                    });
                } else {
                    value = json!({
                        "line1": format!("CPU {:.0}{} {:.0}{}",
                            cpu_temp_cur_value, temp_unit,
                            cpu_usage_cur_value, usage_unit),
                        "line2": format!("GPU {:.0}{} {:.0}{}",
                            gpu_temp_cur_value, temp_unit,
                            gpu_usage_cur_value, usage_unit),
                        "line3": format!("MEM {:.0}{} {:.0}{}",
                            mem_used, mem_unit,
                            mem_load, usage_unit,
                            // mem_free, mem_unit.to_lowercase()
                        ),
                    });
                }
            }
        } else {
            // Custom Senors
            let mut labels = vec![""; CUSTOM_SENSORS];
            let mut units = vec![""; CUSTOM_SENSORS];
            let mut values = vec![String::new(); CUSTOM_SENSORS];

            let sensors_per_line = match config_main.get("sensors_per_line") {
                Some(spl) => spl.parse::<u8>()?,
                None => 1,
            };
            run_sensors(
                pages_sensors,
                &mut labels,
                &mut units,
                &mut values,
                &hwinfo,
                decimal,
            )?;
            value = format_custom_value(sensors_per_line, labels, values, units);
        }
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

use console::Term;
use dialoguer::Input;
use gamesense::{
    client::GameSenseClient,
    handler::screen::{self, ScreenHandler},
};
use hwinfo_steelseries_oled::{Hwinfo, HwinfoSensorsReadingElement};
use ini::Ini;
use serde_json::{json, Value};
use std::num::Wrapping;
use tray_icon::{Icon, TrayIconBuilder};

#[derive(PartialEq)]
enum STYLE {
    VERTICAL,
    HORIZONTAL,
    CUSTOM,
}

impl std::fmt::Display for STYLE {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            STYLE::VERTICAL => write!(f, "Vertical"),
            STYLE::HORIZONTAL => write!(f, "Horizontal"),
            STYLE::CUSTOM => write!(f, "Custom"),
        }
    }
}

const CUSTOM_SENSORS: usize = 6;
const DISPLAY_LINES: usize = 3;

#[allow(unreachable_code)]
fn main() -> Result<(), anyhow::Error> {
    let icon = Icon::from_path("assets/hwinfo-steelseries-icon.ico", Some((64, 64)))?;
    let _tray_icon = TrayIconBuilder::new()
        .with_tooltip("HWiNFO-SteelSeries")
        .with_icon(icon)
        .build()
        .unwrap();

    let term = Term::stdout();

    let mut client = connect_steelseries(&term)?;

    let mut hwinfo = connect_hwinfo(&term)?;
    hwinfo.pull()?;

    let config = match Ini::load_from_file("conf.ini") {
        Ok(conf) => conf,
        Err(_err) => create_config(&term, &hwinfo)?,
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
    let config_sensors = match config.section(Some("Sensors")) {
        Some(sensors) => sensors,
        None => {
            return Err(anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Sensors Config Not found",
            )))
        }
    };

    // std::thread::sleep(std::time::Duration::from_secs(1));
    // console_window(Console::HIDE);

    let style = match config_main.get("style") {
        Some(style) => style,
        None => {
            return Err(anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Style not found",
            )))
        }
    };
    let vertical = match style {
        "Vertical" => Some(true),
        "Horizontal" => Some(false),
        _ => None,
    };
    let summary = match style {
        "Vertical" | "Horizontal" => true,
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

    let page1_handler = page_handler(3, "line1", "line2", "line3", false, None);

    client.bind_event("MAIN", None, None, None, None, vec![page1_handler])?;
    client.start_heartbeat();
    let mut i = Wrapping(0isize);
    let mut count: usize = 0;
    loop {
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
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }

        if summary {
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
            let temp_unit = "°"; //String::from_utf8(sensor_cpu_temp.utf_unit.to_vec())?;
            let usage_unit = "%"; //String::from_utf8(sensor_cpu_usage.utf_unit.to_vec())?;
            let gpu_temp_cur_value = sensor_gpu_temp.value;
            let gpu_usage_cur_value = sensor_gpu_usage.value;
            let mem_unit = "G";
            let mem_used = sensor_mem_used.value / 1024.0;
            let mem_free = sensor_mem_free.value / 1024.0;
            let mem_load = sensor_mem_load.value;
            let line1_spaces = " ";
            let line2_spaces = " ";

            if vertical.unwrap_or(false) {
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
            // Custom
            // let mut labels: Vec<&str> = Vec::with_capacity(CUSTOM_SENSORS);
            // let mut units: Vec<&str> = Vec::with_capacity(CUSTOM_SENSORS);
            // let mut values: Vec<f64> = Vec::with_capacity(CUSTOM_SENSORS);
            let mut labels = vec![""; CUSTOM_SENSORS];
            let mut units = vec![""; CUSTOM_SENSORS];
            let mut values = vec![String::new(); CUSTOM_SENSORS];

            let two_sensors_per_line = match config_main.get("two_sensors_per_line") {
                Some(tspl) => tspl.parse::<bool>()?,
                None => false,
            };
            for i in 0..CUSTOM_SENSORS {
                let sensor = match config
                    .section(Some("Sensors"))
                    .unwrap()
                    .get(format!("sensor_{}", i))
                {
                    Some(sensor) => sensor,
                    None => continue,
                }
                .split(";")
                .collect::<Vec<&str>>();
                let label = match config_sensors.get(format!("label_{}", i)) {
                    Some(label) => label,
                    None => sensor[1],
                };
                let unit = match config_sensors.get(format!("unit_{}", i)) {
                    Some(unit) => unit,
                    None => "",
                };
                let value = match hwinfo.get(sensor[0], sensor[1]) {
                    Some(value) => value,
                    None => {
                        return Err(anyhow::Error::new(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("Sensor not found:\n\t{}\n\t{}", sensor[0], sensor[1]),
                        )))
                    }
                }
                .value;
                let value_string: String;
                if decimal {
                    value_string = format!("{:.1}", &value);
                } else {
                    value_string = format!("{:.0}", &value);
                }
                labels[i] = label;
                units[i] = unit;
                values[i] = value_string;
            }
            value = format_custom_value(two_sensors_per_line, labels, values, units);
        }
        client.trigger_event_frame("MAIN", i.0, value)?;
        i += 1;
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    client.stop_heartbeat()?;

    Ok(())
}

fn format_custom_value(
    two_sensors_per_line: bool,
    labels: Vec<&str>,
    values: Vec<String>,
    units: Vec<&str>,
) -> Value {
    let mut value = json!({});
    if !two_sensors_per_line {
        for i in 0..DISPLAY_LINES {
            value[format!("line{}", i + 1)] =
                json!(format!("{} {}{}", labels[i], values[i], units[i]));
        }
    } else {
        value["line1"] = json!(format!(
            "{} {}{} {} {}{}",
            labels[0], values[0], units[0], labels[1], values[1], units[1]
        ));
        value["line2"] = json!(format!(
            "{} {}{} {} {}{}",
            labels[2], values[2], units[2], labels[3], values[3], units[3]
        ));
        value["line3"] = json!(format!(
            "{} {}{} {} {}{}",
            labels[4], values[4], units[4], labels[5], values[5], units[5]
        ));
    }
    value
}

fn connect_hwinfo(term: &Term) -> Result<Hwinfo, anyhow::Error> {
    match Hwinfo::new() {
        Ok(hwinfo) => {
            term.write_line("Connected to HWiNFO")?;
            Ok(hwinfo)
        }
        Err(_err) => {
            // println!("Can't connect to HWiNFO. Trying again in 1 second.");
            for i in (1..=3).rev() {
                term.clear_line()?;
                term.write_line(
                    format!("Can't connect to HWiNFO. Trying again in {} second.", i).as_str(),
                )?;
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            connect_hwinfo(term)
        }
    }
}

fn connect_steelseries(term: &Term) -> Result<GameSenseClient, anyhow::Error> {
    match GameSenseClient::new("HWINFO", "HWiNFO_Stats", "Ryan", Some(10000)) {
        Ok(c) => {
            term.write_line("Connected to SteelSeries GG")?;
            Ok(c)
        }
        Err(_e) => {
            for i in (1..=3).rev() {
                term.clear_line()?;
                term.write_line(
                    format!(
                        "Can't connect to SteelSeries GG. Trying again in {} second.",
                        i
                    )
                    .as_str(),
                )?;
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            connect_steelseries(term)
        }
    }
}

enum Console {
    SHOW,
    #[allow(dead_code)]
    HIDE,
}

fn console_window(action: Console) {
    use std::ptr;
    use winapi::um::wincon::GetConsoleWindow;
    use winapi::um::winuser::{ShowWindow, SW_HIDE, SW_SHOW};
    let window = unsafe { GetConsoleWindow() };
    let sw = match action {
        Console::HIDE => SW_HIDE,
        Console::SHOW => SW_SHOW,
    };
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow
    if window != ptr::null_mut() {
        unsafe {
            ShowWindow(window, sw);
        }
    }
}

fn page_handler(
    ttl: isize,
    label_1: &str,
    label_2: &str,
    label_3: &str,
    bold: bool,
    zone: Option<&str>,
) -> ScreenHandler {
    screen::ScreenHandler::new(
        "screened",
        match zone {
            Some(zone) => zone,
            None => "one",
        },
        screen::ScreenDataDefinition::StaticScreenDataDefinition(
            screen::StaticScreenDataDefinition(vec![screen::ScreenFrameData::MultiLineFrameData(
                screen::MultiLineFrameData {
                    frame_modifiers_data: Some(screen::FrameModifiersData {
                        length_millis: Some(ttl * 1000),
                        icon_id: Some(screen::Icon::None),
                        repeats: Some(screen::Repeat::Bool(false)),
                    }),
                    lines: vec![
                        screen::LineData {
                            type_options: screen::LineDataType::TextModifiersData(
                                screen::TextModifiersData {
                                    has_text: true,
                                    prefix: None,
                                    suffix: None,
                                    bold: Some(bold),
                                    wrap: None,
                                },
                            ),
                            data_accessor_data: Some(screen::DataAccessorData {
                                arg: None,
                                context_frame_key: Some(String::from(label_1)),
                            }),
                        },
                        screen::LineData {
                            type_options: screen::LineDataType::TextModifiersData(
                                screen::TextModifiersData {
                                    has_text: true,
                                    prefix: None,
                                    suffix: None,
                                    bold: Some(bold),
                                    wrap: None,
                                },
                            ),
                            data_accessor_data: Some(screen::DataAccessorData {
                                arg: None,
                                context_frame_key: Some(String::from(label_2)),
                            }),
                        },
                        screen::LineData {
                            type_options: screen::LineDataType::TextModifiersData(
                                screen::TextModifiersData {
                                    has_text: true,
                                    prefix: None,
                                    suffix: None,
                                    bold: Some(bold),
                                    wrap: None,
                                },
                            ),
                            data_accessor_data: Some(screen::DataAccessorData {
                                arg: None,
                                context_frame_key: Some(String::from(label_3)),
                            }),
                        },
                    ],
                },
            )]),
        ),
    )
}

fn create_config(term: &Term, hwinfo: &Hwinfo) -> Result<Ini, anyhow::Error> {
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
    let input: u8 = Input::new()
        .with_prompt("Choose style\n[1,2,3]")
        .interact_text()?;
    let style = match input {
        1 => STYLE::VERTICAL,
        3 => STYLE::CUSTOM,
        2 | _ => STYLE::HORIZONTAL,
    };
    conf.with_section(Some("Main"))
        .set("style", style.to_string());

    if style != STYLE::CUSTOM {
        let gpus = hwinfo.find("GPU Temperature")?;
        let len_gpus = gpus.len();
        if len_gpus > 1 {
            term.write_line("Which GPU:\n")?;
            for (i, gpu) in gpus.iter().enumerate() {
                let sensor = &hwinfo.master_sensor_names[gpu.dw_sensor_index as usize];
                let setup = format!("{}: {}", i, sensor);
                term.write_line(&setup)?;
            }
            let gpu_selection: usize = Input::new()
                .with_prompt(format!("0..{}", len_gpus - 1))
                .interact_text()?;

            let gpu_selected =
                &hwinfo.master_sensor_names[gpus[gpu_selection].dw_sensor_index as usize];
            conf.with_section(Some("Main")).set("gpu", gpu_selected);
        }
    } else {
        println!("\n3 lines will fit on the Arctis(or Nova) Pro screen, and 2 on the Apex Pro.");
        let lines: u8 = match Input::new()
            .with_prompt("How many lines? (2-3): ")
            .interact_text()
        {
            Ok(lines) => match lines {
                2 | 3 => lines,
                _ => 3,
            },
            Err(_) => 3,
        };
        let sensors_per_line: u8 = match Input::new()
            .with_prompt("How many sensors per line? (1-2): ")
            .interact_text()
        {
            Ok(sensors) => match sensors {
                1 => {
                    conf.with_section(Some("Main"))
                        .set("two_sensors_per_line", "false");
                    sensors
                }
                2 => {
                    conf.with_section(Some("Main"))
                        .set("two_sensors_per_line", "true");
                    sensors
                }
                _ => 2,
            },
            Err(_) => 2,
        };

        for k in 0..(lines * sensors_per_line) {
            println!("\n{} / {}\n", k + 1, (lines * sensors_per_line));
            for (i, sensor) in hwinfo.master_sensor_names.iter().enumerate() {
                println!("{}) {}", i, sensor);
            }
            let length = hwinfo.master_sensor_names.len();
            let category: usize = match Input::new().with_prompt("Category").interact_text() {
                Ok(category) => {
                    if category >= length {
                        println!("Category out of range, please try again.");
                        return create_config(term, hwinfo);
                    } else {
                        category
                    }
                }
                Err(_) => 0,
            };
            let sensor_name = &hwinfo.master_sensor_names[category];
            let sensor = hwinfo.master_readings.sensors.get(sensor_name).unwrap();
            println!("\n{}:", sensor_name);
            let mut temp_readings = Vec::new();
            for (i, reading) in sensor.reading.iter().enumerate() {
                println!("\t{}) {}", i, reading.0);
                let sensor_key = format!("{};{}", sensor_name, reading.0);
                temp_readings.push(sensor_key.to_owned());
            }
            let sensor_selection: usize = Input::new().with_prompt("Sensor").interact_text()?;
            let sensor_selected = format!("\"{}\"", &temp_readings[sensor_selection]);
            let key = format!("sensor_{}", k);
            conf.with_section(Some("Sensors")).set(key, sensor_selected);
        }
    }
    conf.write_to_file("conf.ini")?;

    term.write_line("config created.")?;
    Ok(conf)
}

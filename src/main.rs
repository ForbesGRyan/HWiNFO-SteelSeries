use gamesense::{
    client::GameSenseClient,
    handler::screen::{self, ScreenHandler},
};
use hwinfo_steelseries_oled::{Hwinfo, HwinfoSensorsReadingElement};
use serde_json::json;
use std::{num::Wrapping, io::Write};
use console::Term;
use ini::Ini;
use dialoguer::Input;

struct Screen {
    width: usize,
    height: usize,
}

const NOVA_PRO: Screen = Screen {
    width: 128,
    height: 52,
};
// const ARCTIS_PRO: Screen = Screen{width: 128, height: 48};

fn create_config(term: &Term) -> Result<Ini, anyhow::Error> {
    term.write_line("Config not found.")?;
    let mut conf = Ini::new();
    term.write_line("
    1) CPU  GPU  MEM\n
       55°  45°  8.65G\n
       10%  0.0% 32.0G\n
    ")?;
    term.write_line("
    2) CPU  45°  10.0%\n
       GPU  35°  0.0%\n
       MEM  10G  33.3%\n
    ")?;
    let input: u8 = Input::new()
        .with_prompt("Choose style\n1 or 2")
        .interact_text()?;
    let vertical = match input {
        1 => true,
        _ => false,
    };
    conf.with_section(Some("Main"))
        .set("vertical", vertical.to_string());

    conf.write_to_file("conf.ini")?;

    term.write_line("config created.")?;
    Ok(conf)
}

#[allow(unreachable_code)]
fn main() -> Result<(), anyhow::Error> {
    let term = Term::stdout();

    let mut client = connect_steelseries(&term)?;

    let mut hwinfo= connect_hwinfo(&term)?;
    hwinfo.pull()?;

    let config = match Ini::load_from_file("conf.ini") {
        Ok(conf) => conf,
        Err(_err) => {
            create_config(&term)?
        }
    };

    std::thread::sleep(std::time::Duration::from_secs(1));
    hide_console_window();

    let vertical = match config
        .section(Some("Main")).unwrap()
        .get("vertical").unwrap() {
            "true" => true,
            "false" => false,
            _ => panic!("invalid input")
        };

    let screen = NOVA_PRO;
    let _width = screen.width;
    let _height = screen.height;

    let page1_handler = page_handler(3, "line1", "line2", "line3", false);
    let page2_handler = page_handler(3, "line1", "line2", "line3", false);
    let error_handler = page_handler(30, "line1", "line2", "line3", true);

    client.bind_event("MAIN", None, None, None, None, vec![page1_handler])?;
    client.bind_event("ERROR", None, None, None, None, vec![error_handler])?;
    client.bind_event("EVENT2", None, None, None, None, vec![page2_handler])?;
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
        }
        drop(old);
        let mut value = json!("");
        if count >= limit {
            value = json!({"line1":"Disconnected",
                           "line2":"FROM",
                           "line3":"HWiNFO"});
            client.trigger_event_frame("ERROR", i.0, value)?;
            i += 1;
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
    
        let sensor_cpu_usage = hwinfo.find("Total CPU Usage").unwrap();
        let sensor_cpu_temp = hwinfo.find("CPU (Tctl/Tdie)").unwrap();
        let sensor_gpu_usage = hwinfo.find("GPU Core Load").unwrap();
        // let sensor_gpu_temp = hwinfo.find("GPU Temperature").unwrap();
        let sensor_gpu_temp = hwinfo.get("GPU [#0]: NVIDIA GeForce RTX 3090", "GPU Temperature").unwrap();
        let sensor_mem_used = hwinfo.find("Physical Memory Used").unwrap();
        let sensor_mem_free = hwinfo.find("Physical Memory Available").unwrap();
        let sensor_mem_load = hwinfo.find("Physical Memory Load").unwrap();
        // hwinfo.get("System: ASUS ", "Physical Memory Used").unwrap();

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

        let line1_spaces = "  ";
        let line2_spaces = "  ";

        if vertical {
            value = json!({
                "line1": "CPU    GPU    MEM",
                "line2": format!("{:.1}{}{}{:.1}{}{}{:.1}{}",
                    cpu_temp_cur_value, temp_unit,
                    line1_spaces,
                    gpu_temp_cur_value, temp_unit,
                    line1_spaces,
                    mem_used, mem_unit),
                "line3": format!("{:.1}{}{}{:02.1}{}{}{:.1}{}",
                    cpu_usage_cur_value, usage_unit,
                    line2_spaces,
                    gpu_usage_cur_value, usage_unit,
                    line2_spaces,
                    mem_free, mem_unit),
            });
        } else {
            value = json!({
                "line1": format!("CPU   {:.1}{} {:.1}{}",
                    cpu_temp_cur_value, temp_unit,
                    cpu_usage_cur_value, usage_unit),
                "line2": format!("GPU   {:.1}{} {:.1}{}",
                    gpu_temp_cur_value, temp_unit,
                    gpu_usage_cur_value, usage_unit),
                "line3": format!("MEM  {:.1}{} {:.1}{}",
                    mem_used, mem_unit,
                    mem_load, usage_unit,
                    // mem_free, mem_unit.to_lowercase()
                ),
            });
        }
        // if i.0 % 3 == 0 {
        //     client.trigger_event_frame("EVENT2", i.0, json!({
        //         "line1":"Hello!",
        //         "line2":"Hello!",
        //         "line3":"Hello!",
        //     }))?;
        // }
        // else {
            client.trigger_event_frame("MAIN", i.0, value)?;
        // }

        i += 1;
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    client.stop_heartbeat()?;

    Ok(())
}


fn connect_hwinfo(term: &Term) -> Result<Hwinfo, anyhow::Error>{
    match Hwinfo::new() {
        Ok(hwinfo) => {
            term.write_line("Connected to HWiNFO")?;
            Ok(hwinfo)
        },
        Err(_err) => {
            // println!("Can't connect to HWiNFO. Trying again in 1 second.");
            for i in (1..=3).rev() {
                term.write_line(format!("Can't connect to HWiNFO. Trying again in {} second.", i).as_str())?;
                std::thread::sleep(std::time::Duration::from_secs(1));
                term.clear_line()?;
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
        },
        Err(_e) => {
            for i in (1..=3).rev() {
                term.write_line(format!("Can't connect to SteelSeries GG. Trying again in {} second.", i).as_str())?;
                std::thread::sleep(std::time::Duration::from_secs(1));
                term.clear_line()?;
            }
            connect_steelseries(term)
        }
    }
}


fn hide_console_window() {
    use std::ptr;
    use winapi::um::wincon::GetConsoleWindow;
    use winapi::um::winuser::{ShowWindow, SW_HIDE};
    let window = unsafe { GetConsoleWindow() };
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow
    if window != ptr::null_mut() {
        unsafe {
            ShowWindow(window, SW_HIDE);
        }
    }
}

fn page_handler(
    ttl: isize,
    line1_label: &str,
    line2_label: &str,
    line3_label: &str,
    bold: bool,
) -> ScreenHandler {
    screen::ScreenHandler::new(
        "screened",
        "one",
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
                                context_frame_key: Some(String::from(line1_label)),
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
                                context_frame_key: Some(String::from(line2_label)),
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
                                context_frame_key: Some(String::from(line3_label)),
                            }),
                        },
                    ],
                },
            )]),
        ),
    )
}

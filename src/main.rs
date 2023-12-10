use gamesense::{
    client::GameSenseClient,
    handler::screen::{self, ScreenHandler},
};
use hwinfo_steelseries_oled::{Hwinfo, HwinfoSensorsReadingElement};
use serde_json::json;
use std::num::Wrapping;
use std::io;

struct Screen {
    width: usize,
    height: usize,
}

const NOVA_PRO: Screen = Screen {
    width: 128,
    height: 52,
};
// const ARCTIS_PRO: Screen = Screen{width: 128, height: 48};

#[allow(unreachable_code)]
fn main() -> Result<(), anyhow::Error> {
    let mut client = match GameSenseClient::new("HWINFO", "HWiNFO_Stats", "Ryan", Some(10000)) {
        Ok(c) => Ok(c),
        Err(_e) => {
            println!("cannot connect to SteelSeries GG");
            Err(anyhow::Error::new(io::Error::new(io::ErrorKind::NotFound, "Cannot Connect to SteelSeries GG")))
        }
    }?;

    let hwinfo_connected = match Hwinfo::new() {
        Ok(hwinfo) => Some(hwinfo),
        Err(_e) => None, // let handler = page_handler(10, "error1", "error2", "", false);
                         // client.bind_event("ERROR", None, None, None, None, vec![handler])?;
                         // client.start_heartbeat();
                         // client.trigger_event_frame(
                         //     "ERROR",
                         //     0,
                         //     json!(
                         //     {
                         //         "error1": "CAN'T CONNECT",
                         //         "error2": "TO HWiNFO"
                         //     }),
                         // )?;
                         // client.stop_heartbeat()?;
                         // None
    };
    let mut hwinfo = match hwinfo_connected {
        Some(hw) => hw,
        None => panic!("Cannot connect to HWiNFO"),
    };
    hwinfo.pull()?;

    let screen = NOVA_PRO;
    let width = screen.width;
    let height = screen.height;

    let page1_handler = page_handler(3, "line1", "line2", "line3", false);
    let page2_handler = page_handler(3, "line1", "line2", "line3", false);

    client.bind_event("MAIN", None, None, None, None, vec![page1_handler])?;
    client.bind_event("EVENT2", None, None, None, None, vec![page2_handler])?;
    client.start_heartbeat();
    let mut i = Wrapping(0isize);
    loop {
        hwinfo.pull()?;
        let mut value = json!("");

        let sensor_cpu_usage = hwinfo.find("Total CPU Usage").unwrap();
        let sensor_cpu_temp = hwinfo.find("CPU (Tctl/Tdie)").unwrap();
        let sensor_gpu_usage = hwinfo.find("GPU Core Load").unwrap();
        let sensor_gpu_temp = hwinfo.find("GPU Temperature").unwrap();
        let sensor_mem_used = hwinfo.find("Physical Memory Used").unwrap();
        let sensor_mem_free = hwinfo.find("Physical Memory Available").unwrap();

        let cpu_temp_cur_value = sensor_cpu_temp.value;
        let cpu_temp_unit = "°"; //String::from_utf8(sensor_cpu_temp.utf_unit.to_vec())?;
        let cpu_usage_cur_value = sensor_cpu_usage.value;
        let cpu_usage_unit = "%"; //String::from_utf8(sensor_cpu_usage.utf_unit.to_vec())?;

        let gpu_temp_cur_value = sensor_gpu_temp.value;
        let gpu_temp_unit = "°"; //String::from_utf8(sensor_gpu_temp.utf_unit.to_vec())?;
        let gpu_usage_cur_value = sensor_gpu_usage.value;
        let gpu_usage_unit = "%"; //String::from_utf8(sensor_gpu_usage.utf_unit.to_vec())?;

        let mem_unit = "GB";
        let mem_used = sensor_mem_used.value / 1024.0;
        let mem_free = sensor_mem_free.value / 1024.0;

        let line1_spaces = "  ";
        let line2_spaces = "  ";

        value = json!({
            "line1": "CPU    GPU    MEM",
            "line2": format!("{:.1}{}{}{:.1}{}{}{:.1}{}",
                cpu_temp_cur_value, cpu_temp_unit,
                line1_spaces,
                gpu_temp_cur_value, gpu_temp_unit,
                line1_spaces,
                mem_used, mem_unit.to_lowercase()),
            "line3": format!("{:.1}{}{}{:02.1}{}{}{:.1}{}",
                cpu_usage_cur_value, cpu_usage_unit,
                line2_spaces,
                gpu_usage_cur_value, gpu_usage_unit,
                line2_spaces,
                mem_free, mem_unit.to_lowercase()),
        });

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

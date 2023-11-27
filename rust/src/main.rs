use hwinfo_steelseries_oled::Hwinfo;
use serde_json::json;
use gamesense::{
    client::GameSenseClient,
    handler::screen};

fn get_inner_outer_keys(hwinfo: &Hwinfo, value: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let mut outer_key = String::new();
    let mut inner_key = String::new();
    // let _hwinfo = hwinfo.clone();
    'outer: for (outer_k, outer_v) in hwinfo.master_readings.iter() {
        for inner_k in outer_v.keys() {
            if inner_k.contains(value) {
                outer_key = outer_k.to_string();
                inner_key = inner_k.to_string();
                break 'outer;
            }
        }
    }
    Ok((outer_key, inner_key))
}

#[allow(unreachable_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut hwinfo = Hwinfo::new()?;
    hwinfo.pull()?;
    let (cpu_usage_outer_key, cpu_usage_inner_key) = get_inner_outer_keys(&hwinfo, "Total CPU Usage")?;
    let (cpu_temp_outer_key, cpu_temp_inner_key) = get_inner_outer_keys(&hwinfo, "CPU (Tctl/Tdie)")?;
    let (gpu_usage_outer_key, gpu_usage_inner_key) = get_inner_outer_keys(&hwinfo, "GPU Core Load")?;
    let (gpu_temp_outer_key, gpu_temp_inner_key) = get_inner_outer_keys(&hwinfo, "GPU Temperature")?;
    
    let (mem_used_outer_key, mem_used_inner_key) = get_inner_outer_keys(&hwinfo, "Physical Memory Used")?;
    let (mem_free_outer_key, mem_free_inner_key) = get_inner_outer_keys(&hwinfo, "Physical Memory Available")?;


    let mut client: GameSenseClient = GameSenseClient::new("HWINFO", "HWiNFO_Stats", "Ryan", None)?;
    let handler = screen::ScreenHandler::new("screened", "one",
        screen::ScreenDataDefinition::StaticScreenDataDefinition(screen::StaticScreenDataDefinition(
            vec!(
                screen::ScreenFrameData::MultiLineFrameData(screen::MultiLineFrameData {
                    frame_modifiers_data: Some(screen::FrameModifiersData {
                        length_millis: Some(3000),
                        icon_id: Some(screen::Icon::None),
                        repeats: None
                    }),
                    lines: vec![
                        screen::LineData {
                            type_options: screen::LineDataType::TextModifiersData(screen::TextModifiersData {
                                has_text: true,
                                prefix: None,
                                suffix: None,
                                bold: None,
                                wrap: None
                            }),
                            data_accessor_data: Some(screen::DataAccessorData {
                                arg: None,
                                context_frame_key: Some(String::from("line1"))
                            })
                        },
                        screen::LineData {
                            type_options: screen::LineDataType::TextModifiersData(screen::TextModifiersData {
                                has_text: true,
                                prefix: None,
                                suffix: None,
                                bold: None,
                                wrap: None
                            }),
                            data_accessor_data: Some(screen::DataAccessorData {
                                arg: None,
                                context_frame_key: Some(String::from("line2"))
                            })
                        },
                        screen::LineData {
                            type_options: screen::LineDataType::TextModifiersData(screen::TextModifiersData {
                                has_text: true,
                                prefix: None,
                                suffix: None,
                                bold: None,
                                wrap: None
                            }),
                            data_accessor_data: Some(screen::DataAccessorData {
                                arg: None,
                                context_frame_key: Some(String::from("line3"))
                            })
                        },
                    ]
                })
            )
        ))
    );

    client.bind_event("EVENT", None, None, None, None, vec![handler])?;
    client.start_heartbeat();
    let mut i = 0;
    loop {
        hwinfo.pull()?; 
        let (_, cpu_temp) = hwinfo.master_readings
            .get(&cpu_temp_outer_key).unwrap()
            .get_key_value(&cpu_temp_inner_key).unwrap();
        let cpu_temp_unit  = &cpu_temp.0[0..2];
        let cpu_temp_cur_value = &cpu_temp.1[0];
        
        let (_, cpu_usage) = hwinfo.master_readings
            .get(&cpu_usage_outer_key).unwrap()
            .get_key_value(&cpu_usage_inner_key).unwrap();
        let cpu_usage_unit = &cpu_usage.0;
        let cpu_usage_cur_value = &cpu_usage.1[0];

        let (_, gpu_usage) = hwinfo.master_readings
            .get(&gpu_usage_outer_key).unwrap()
            .get_key_value(&gpu_usage_inner_key).unwrap();
        let gpu_usage_unit = &gpu_usage.0;
        let gpu_usage_cur_value = &gpu_usage.1[0];

        let (_, gpu_temp) = hwinfo.master_readings
            .get(&gpu_temp_outer_key).unwrap()
            .get_key_value(&gpu_temp_inner_key).unwrap();
        let gpu_temp_unit  = &gpu_temp.0[0..2];
        let gpu_temp_cur_value = &gpu_temp.1[0];

        let (_ , mem_used) = hwinfo.master_readings
            .get(&mem_used_outer_key).unwrap()
            .get_key_value(&mem_used_inner_key).unwrap();
        let mem_used_unit = "GB";//&mem_used.0;
        let mem_used_curr = &mem_used.1[0] / 1024.0;

        let (_ , mem_free)= hwinfo.master_readings
            .get(&mem_free_outer_key).unwrap()
            .get_key_value(&mem_free_inner_key).unwrap();
        let mem_free_unit = "GB"; //&mem_free.0;
        let mem_free_curr = &mem_free.1[0] / 1024.0;
    
        client.trigger_event_frame("EVENT", i, json!({
            "line1": "CPU  GPU  MEM",
            // "line1":"12345678901234567",
            "line2": format!("{:.1}{} {:.1}{} {:.1}{}",
                cpu_temp_cur_value, cpu_temp_unit, gpu_temp_cur_value, gpu_temp_unit, mem_used_curr, mem_used_unit),
            "line3": format!("{:.1}{} {:.1}{} {:.1}{}",
                cpu_usage_cur_value, cpu_usage_unit, gpu_usage_cur_value, gpu_usage_unit, mem_free_curr, mem_free_unit),
        }))?;
        i += 1;
    }
    client.stop_heartbeat()?;

    Ok(())
}
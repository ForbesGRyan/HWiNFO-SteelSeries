use rust::update_hwinfo;
use serde_json::json;
use gamesense::{
    client::GameSenseClient,
    handler::screen};
use std::{thread, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>>{
    let mut _info = update_hwinfo().unwrap();
    // let mut outer_key: String = String::new();
    // let mut inner_key: String = String::new();

    // 'outer: for (k, v) in _info.master_readings.into_iter() {
    //     for (_k, _v) in v.into_iter() {
    //         if _k.contains("CPU (Tctl/Tdie)") {
    //             outer_key = k;
    //             inner_key = _k;
    //             break 'outer;
    //         }
    //     }
    // }
    // for i in 0..10 {
    //    _info = update_hwinfo()?; 
    //    let (label, cpu_temp) = _info.master_readings
    //     .get(&outer_key).unwrap()
    //     .get_key_value(&inner_key).unwrap();
    //     println!("{}: {}{}", label, cpu_temp[1], cpu_temp[0]);
    //     thread::sleep(Duration::from_secs(1))
    // }
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
                                context_frame_key: Some(String::from("artist"))
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
                                context_frame_key: Some(String::from("album"))
                            })
                        },
                        screen::LineData {
                            type_options: screen::LineDataType::ProgressBarData(screen::ProgressBarData {
                                has_progress_bar: true
                            }),
                            data_accessor_data: Some(screen::DataAccessorData {
                                arg: None,
                                context_frame_key: None //Some(String::from("song"))
                            })
                        },
                    ]
                })
            )
        ))
    );

    client.bind_event("EVENT", None, None, None, None, vec![handler])?;
    client.start_heartbeat();
    for i in 0..100 {
        client.trigger_event_frame("EVENT", i, json!({
            "artist": "Three Days Grace",
            "album": "One-X",
            "song": "Gone Forever"
        }))?;
    }
    client.stop_heartbeat()?;

    Ok(())
}
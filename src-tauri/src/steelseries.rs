use gamesense::handler::screen::{self, ScreenHandler};

pub fn page_handler(ttl: isize, labels: &[&str], bold: Option<bool>) -> ScreenHandler {
    let lines = labels
        .iter()
        .map(|label| screen::LineData {
            type_options: screen::LineDataType::TextModifiersData(screen::TextModifiersData {
                has_text: true,
                prefix: None,
                suffix: None,
                bold,
                wrap: None,
            }),
            data_accessor_data: Some(screen::DataAccessorData {
                arg: None,
                context_frame_key: Some(String::from(*label)),
            }),
        })
        .collect();

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
                    lines,
                },
            )]),
        ),
    )
}

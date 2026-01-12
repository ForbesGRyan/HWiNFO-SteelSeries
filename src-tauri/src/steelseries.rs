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

#[cfg(test)]
mod tests {
    use super::*;
    use gamesense::handler::screen::{
        Icon, LineDataType, MultiLineFrameData, Repeat, ScreenDataDefinition, ScreenFrameData,
        TextModifiersData,
    };

    /// Helper function to extract MultiLineFrameData from a ScreenHandler
    fn extract_multi_line_frame_data(handler: &ScreenHandler) -> &MultiLineFrameData {
        match &handler.datas {
            ScreenDataDefinition::StaticScreenDataDefinition(static_def) => {
                match &static_def.0[0] {
                    ScreenFrameData::MultiLineFrameData(data) => data,
                    _ => panic!("Expected MultiLineFrameData"),
                }
            }
            _ => panic!("Expected StaticScreenDataDefinition"),
        }
    }

    /// Helper function to extract TextModifiersData from a LineData
    fn extract_text_modifiers(line_data: &screen::LineData) -> &TextModifiersData {
        match &line_data.type_options {
            LineDataType::TextModifiersData(data) => data,
            _ => panic!("Expected TextModifiersData"),
        }
    }

    #[test]
    fn test_page_handler_default_parameters_no_bold() {
        let labels = &["line1", "line2", "line3"];
        let handler = page_handler(1, labels, None);

        // Verify basic handler properties
        assert_eq!(handler.device_type, "screened");
        assert_eq!(handler.zone, "one");

        // Extract and verify MultiLineFrameData
        let frame_data = extract_multi_line_frame_data(&handler);

        // Verify frame modifiers
        let modifiers = frame_data.frame_modifiers_data.as_ref().unwrap();
        assert_eq!(modifiers.length_millis, Some(1000)); // ttl * 1000
        assert!(matches!(modifiers.icon_id, Some(Icon::None)));
        assert!(matches!(modifiers.repeats, Some(Repeat::Bool(false))));

        // Verify lines count
        assert_eq!(frame_data.lines.len(), 3);

        // Verify each line has bold=None
        for (i, line) in frame_data.lines.iter().enumerate() {
            let text_modifiers = extract_text_modifiers(line);
            assert!(text_modifiers.has_text);
            assert_eq!(text_modifiers.bold, None);
            assert_eq!(text_modifiers.prefix, None);
            assert_eq!(text_modifiers.suffix, None);

            // Verify context_frame_key matches the label
            let accessor = line.data_accessor_data.as_ref().unwrap();
            assert_eq!(accessor.context_frame_key, Some(labels[i].to_string()));
            assert_eq!(accessor.arg, None);
        }
    }

    #[test]
    fn test_page_handler_with_bold_true() {
        let labels = &["label1", "label2"];
        let handler = page_handler(2, labels, Some(true));

        let frame_data = extract_multi_line_frame_data(&handler);

        // Verify all lines have bold=Some(true)
        for line in &frame_data.lines {
            let text_modifiers = extract_text_modifiers(line);
            assert_eq!(text_modifiers.bold, Some(true));
        }
    }

    #[test]
    fn test_page_handler_with_bold_false() {
        let labels = &["label1", "label2"];
        let handler = page_handler(2, labels, Some(false));

        let frame_data = extract_multi_line_frame_data(&handler);

        // Verify all lines have bold=Some(false)
        for line in &frame_data.lines {
            let text_modifiers = extract_text_modifiers(line);
            assert_eq!(text_modifiers.bold, Some(false));
        }
    }

    #[test]
    fn test_page_handler_with_different_ttl_values() {
        // Test TTL = 0
        let handler_0 = page_handler(0, &["line"], None);
        let frame_data_0 = extract_multi_line_frame_data(&handler_0);
        let modifiers_0 = frame_data_0.frame_modifiers_data.as_ref().unwrap();
        assert_eq!(modifiers_0.length_millis, Some(0));

        // Test TTL = 5
        let handler_5 = page_handler(5, &["line"], None);
        let frame_data_5 = extract_multi_line_frame_data(&handler_5);
        let modifiers_5 = frame_data_5.frame_modifiers_data.as_ref().unwrap();
        assert_eq!(modifiers_5.length_millis, Some(5000));

        // Test TTL = 10
        let handler_10 = page_handler(10, &["line"], None);
        let frame_data_10 = extract_multi_line_frame_data(&handler_10);
        let modifiers_10 = frame_data_10.frame_modifiers_data.as_ref().unwrap();
        assert_eq!(modifiers_10.length_millis, Some(10000));

        // Test negative TTL
        let handler_neg = page_handler(-1, &["line"], None);
        let frame_data_neg = extract_multi_line_frame_data(&handler_neg);
        let modifiers_neg = frame_data_neg.frame_modifiers_data.as_ref().unwrap();
        assert_eq!(modifiers_neg.length_millis, Some(-1000));
    }

    #[test]
    fn test_page_handler_with_single_label() {
        let labels = &["single_line"];
        let handler = page_handler(1, labels, None);

        let frame_data = extract_multi_line_frame_data(&handler);
        assert_eq!(frame_data.lines.len(), 1);

        let accessor = frame_data.lines[0].data_accessor_data.as_ref().unwrap();
        assert_eq!(accessor.context_frame_key, Some("single_line".to_string()));
    }

    #[test]
    fn test_page_handler_with_three_labels() {
        let labels = &["line1", "line2", "line3"];
        let handler = page_handler(1, labels, None);

        let frame_data = extract_multi_line_frame_data(&handler);
        assert_eq!(frame_data.lines.len(), 3);

        for (i, line) in frame_data.lines.iter().enumerate() {
            let accessor = line.data_accessor_data.as_ref().unwrap();
            assert_eq!(
                accessor.context_frame_key,
                Some(format!("line{}", i + 1))
            );
        }
    }

    #[test]
    fn test_page_handler_with_five_labels() {
        let labels = &["a", "b", "c", "d", "e"];
        let handler = page_handler(1, labels, None);

        let frame_data = extract_multi_line_frame_data(&handler);
        assert_eq!(frame_data.lines.len(), 5);

        let expected_labels = ["a", "b", "c", "d", "e"];
        for (i, line) in frame_data.lines.iter().enumerate() {
            let accessor = line.data_accessor_data.as_ref().unwrap();
            assert_eq!(
                accessor.context_frame_key,
                Some(expected_labels[i].to_string())
            );
        }
    }

    #[test]
    fn test_page_handler_with_empty_labels() {
        let labels: &[&str] = &[];
        let handler = page_handler(1, labels, None);

        let frame_data = extract_multi_line_frame_data(&handler);
        assert_eq!(frame_data.lines.len(), 0);
    }

    #[test]
    fn test_page_handler_with_empty_string_labels() {
        let labels = &["", "", ""];
        let handler = page_handler(1, labels, None);

        let frame_data = extract_multi_line_frame_data(&handler);
        assert_eq!(frame_data.lines.len(), 3);

        for line in &frame_data.lines {
            let accessor = line.data_accessor_data.as_ref().unwrap();
            assert_eq!(accessor.context_frame_key, Some(String::new()));
        }
    }

    #[test]
    fn test_page_handler_with_special_characters_in_labels() {
        let labels = &["CPU: 50%", "GPU Temp", "RAM Usage (GB)"];
        let handler = page_handler(1, labels, None);

        let frame_data = extract_multi_line_frame_data(&handler);
        assert_eq!(frame_data.lines.len(), 3);

        let accessor_0 = frame_data.lines[0].data_accessor_data.as_ref().unwrap();
        assert_eq!(accessor_0.context_frame_key, Some("CPU: 50%".to_string()));

        let accessor_1 = frame_data.lines[1].data_accessor_data.as_ref().unwrap();
        assert_eq!(accessor_1.context_frame_key, Some("GPU Temp".to_string()));

        let accessor_2 = frame_data.lines[2].data_accessor_data.as_ref().unwrap();
        assert_eq!(
            accessor_2.context_frame_key,
            Some("RAM Usage (GB)".to_string())
        );
    }

    #[test]
    fn test_page_handler_consistent_text_modifiers_structure() {
        let labels = &["test1", "test2"];
        let handler = page_handler(3, labels, Some(true));

        let frame_data = extract_multi_line_frame_data(&handler);

        for line in &frame_data.lines {
            let text_modifiers = extract_text_modifiers(line);

            // All text modifiers should have consistent structure
            assert!(text_modifiers.has_text);
            assert_eq!(text_modifiers.prefix, None);
            assert_eq!(text_modifiers.suffix, None);
            assert_eq!(text_modifiers.wrap, None);
        }
    }

    #[test]
    fn test_page_handler_frame_structure() {
        let labels = &["line1"];
        let handler = page_handler(1, labels, None);

        // Verify the entire structure is correct
        match &handler.datas {
            ScreenDataDefinition::StaticScreenDataDefinition(static_def) => {
                assert_eq!(static_def.0.len(), 1); // Only one frame
                match &static_def.0[0] {
                    ScreenFrameData::MultiLineFrameData(_) => {
                        // Expected variant
                    }
                    _ => panic!("Expected MultiLineFrameData variant"),
                }
            }
            ScreenDataDefinition::RangeScreenDataDefintion(_) => {
                panic!("Expected StaticScreenDataDefinition variant");
            }
        }
    }
}

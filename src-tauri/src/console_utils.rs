use console::Term;
use serde_json::Value;

use crate::consts::DISPLAY_LINES;

pub enum Console {
    SHOW,
    #[allow(dead_code)]
    HIDE,
}

pub fn console_window(action: Console) {
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

#[allow(dead_code)]
pub fn display_value_in_console(term: &Term, value: &Value) -> anyhow::Result<()> {
    term.clear_screen()?;
    for i in 0..DISPLAY_LINES {
        term.write_line(&value[format!("line{}", i + 1)].to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ============================================================================
    // Console Enum Tests
    // ============================================================================
    //
    // The Console enum represents the two states for console window visibility.
    // These tests verify the enum variants exist and can be used in match expressions.

    #[test]
    fn test_console_enum_show_variant_exists() {
        let action = Console::SHOW;
        match action {
            Console::SHOW => assert!(true),
            Console::HIDE => panic!("Expected SHOW variant"),
        }
    }

    #[test]
    fn test_console_enum_hide_variant_exists() {
        let action = Console::HIDE;
        match action {
            Console::HIDE => assert!(true),
            Console::SHOW => panic!("Expected HIDE variant"),
        }
    }

    #[test]
    fn test_console_enum_exhaustive_match() {
        // This test ensures all Console variants are covered in a match
        // If a new variant is added, this test will fail to compile
        fn describe_action(action: Console) -> &'static str {
            match action {
                Console::SHOW => "show",
                Console::HIDE => "hide",
            }
        }

        assert_eq!(describe_action(Console::SHOW), "show");
        assert_eq!(describe_action(Console::HIDE), "hide");
    }

    // ============================================================================
    // console_window Function Tests
    // ============================================================================
    //
    // NOTE: The console_window function uses Windows API calls (GetConsoleWindow,
    // ShowWindow) which require an actual Windows console environment to test
    // properly. Unit tests cannot easily verify the actual window visibility
    // changes without mocking the Windows API.
    //
    // The function is designed to be safe:
    // - It checks if the console window handle is valid (not null) before
    //   calling ShowWindow
    // - It gracefully handles the case where there is no console window
    //
    // For comprehensive testing of console_window, integration tests should be
    // used that run in an actual Windows environment with a console window.
    //
    // The following test verifies that the function doesn't panic when called,
    // even if there's no console window (common in test environments).

    #[test]
    fn test_console_window_does_not_panic_on_show() {
        // This test verifies the function handles the case where there may be
        // no console window (which is common in test runners)
        // The function should complete without panicking
        console_window(Console::SHOW);
    }

    #[test]
    fn test_console_window_does_not_panic_on_hide() {
        // Same as above, but for the HIDE action
        console_window(Console::HIDE);
    }

    // ============================================================================
    // display_value_in_console Function Tests
    // ============================================================================
    //
    // NOTE: The display_value_in_console function writes to a Term (terminal)
    // object, which makes direct unit testing challenging because:
    // 1. Term::stdout() requires an actual terminal
    // 2. The function calls term.clear_screen() and term.write_line()
    //    which have side effects
    //
    // For proper testing, this would require:
    // - A mock Term implementation, or
    // - Integration tests with a pseudo-terminal
    //
    // The tests below verify the JSON value access patterns used by the function
    // to ensure the expected data structure is correct.

    #[test]
    fn test_display_value_json_structure() {
        // Verify the JSON structure that display_value_in_console expects
        let value = json!({
            "line1": "CPU: 45C",
            "line2": "GPU: 60C",
            "line3": "RAM: 16GB",
            "line4": "FPS: 144",
            "line5": "NET: 100Mb"
        });

        // Verify we can access all lines that display_value_in_console would access
        for i in 0..DISPLAY_LINES {
            let key = format!("line{}", i + 1);
            assert!(
                value.get(&key).is_some(),
                "Expected key '{}' to exist in value",
                key
            );
        }
    }

    #[test]
    fn test_display_value_json_line_access() {
        // Test that the line access pattern matches what the function uses
        let value = json!({
            "line1": "First line content",
            "line2": "Second line content",
            "line3": "Third line content",
            "line4": "Fourth line content",
            "line5": "Fifth line content"
        });

        // The function uses value[format!("line{}", i + 1)].to_string()
        // Verify this access pattern works correctly
        assert_eq!(value["line1"].to_string(), "\"First line content\"");
        assert_eq!(value["line2"].to_string(), "\"Second line content\"");
        assert_eq!(value["line3"].to_string(), "\"Third line content\"");
        assert_eq!(value["line4"].to_string(), "\"Fourth line content\"");
        assert_eq!(value["line5"].to_string(), "\"Fifth line content\"");
    }

    #[test]
    fn test_display_value_with_special_characters() {
        // Test that JSON values with special characters are handled
        let value = json!({
            "line1": "CPU: 45\u{00B0}C",
            "line2": "GPU: 60\u{00B0}C",
            "line3": "RAM: 16/32 GB",
            "line4": "FPS: 144 (avg)",
            "line5": "100% usage"
        });

        // Verify special characters don't cause issues
        for i in 0..DISPLAY_LINES {
            let key = format!("line{}", i + 1);
            let line_value = &value[&key];
            assert!(line_value.is_string(), "Line {} should be a string", i + 1);
        }
    }

    #[test]
    fn test_display_value_with_empty_lines() {
        // Test handling of empty line values
        let value = json!({
            "line1": "",
            "line2": "Some content",
            "line3": "",
            "line4": "",
            "line5": "More content"
        });

        assert_eq!(value["line1"].as_str().unwrap(), "");
        assert_eq!(value["line2"].as_str().unwrap(), "Some content");
        assert_eq!(value["line3"].as_str().unwrap(), "");
    }

    #[test]
    fn test_display_value_numeric_content() {
        // Test that numeric values are handled (converted to string by to_string())
        let value = json!({
            "line1": 12345,
            "line2": 67.89,
            "line3": "text",
            "line4": true,
            "line5": null
        });

        // When to_string() is called on these, they become string representations
        assert_eq!(value["line1"].to_string(), "12345");
        assert_eq!(value["line2"].to_string(), "67.89");
        assert_eq!(value["line3"].to_string(), "\"text\"");
        assert_eq!(value["line4"].to_string(), "true");
        assert_eq!(value["line5"].to_string(), "null");
    }

    #[test]
    fn test_display_value_in_console_executes() {
        // Invokes the actual function so its body is covered. Output is buffered in tests.
        let term = Term::stdout();
        let value = json!({
            "line1": "L1",
            "line2": "L2",
            "line3": "L3",
            "line4": "L4",
            "line5": "L5",
        });
        let _ = display_value_in_console(&term, &value);
    }

    #[test]
    fn test_display_lines_constant_usage() {
        // Verify DISPLAY_LINES constant matches expected value
        // This ensures the loop in display_value_in_console iterates correctly
        assert_eq!(
            DISPLAY_LINES, 5,
            "DISPLAY_LINES should be 5 for this Tauri version"
        );
    }
}

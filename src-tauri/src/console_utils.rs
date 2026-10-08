pub enum Console {
    SHOW,
    #[allow(dead_code)]
    HIDE,
}

pub fn console_window(action: Console) {
    use winapi::um::wincon::GetConsoleWindow;
    use winapi::um::winuser::{ShowWindow, SW_HIDE, SW_SHOW};
    let window = unsafe { GetConsoleWindow() };
    let sw = match action {
        Console::HIDE => SW_HIDE,
        Console::SHOW => SW_SHOW,
    };
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow
    if !window.is_null() {
        unsafe {
            ShowWindow(window, sw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Console::SHOW => {}
            Console::HIDE => panic!("Expected SHOW variant"),
        }
    }

    #[test]
    fn test_console_enum_hide_variant_exists() {
        let action = Console::HIDE;
        match action {
            Console::HIDE => {}
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
}

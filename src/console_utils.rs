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

pub fn display_value_in_console(term: &Term, value: &Value) -> anyhow::Result<()> {
    term.clear_screen()?;
    for i in 0..DISPLAY_LINES {
        term.write_line(&value[format!("line{}", i + 1)].to_string())?;
    }
    Ok(())
}

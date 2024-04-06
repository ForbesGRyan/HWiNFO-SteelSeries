#[derive(PartialEq)]
pub enum STYLE {
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

pub const CUSTOM_SENSORS: usize = 9;
pub const DISPLAY_LINES: usize = 3;

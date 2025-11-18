#[derive(PartialEq)]
pub enum Style {
    Vertical,
    Horizontal,
    Custom,
}

impl std::fmt::Display for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Style::Vertical => write!(f, "Vertical"),
            Style::Horizontal => write!(f, "Horizontal"),
            Style::Custom => write!(f, "Custom"),
        }
    }
}

pub const CUSTOM_SENSORS: usize = 9;
pub const DISPLAY_LINES: usize = 3;
pub const TICK_RATE: u64 = 1000;
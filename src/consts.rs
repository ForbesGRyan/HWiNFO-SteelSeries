#[derive(Debug, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_style_display_vertical() {
        let style = Style::Vertical;
        assert_eq!(format!("{}", style), "Vertical");
    }

    #[test]
    fn test_style_display_horizontal() {
        let style = Style::Horizontal;
        assert_eq!(format!("{}", style), "Horizontal");
    }

    #[test]
    fn test_style_display_custom() {
        let style = Style::Custom;
        assert_eq!(format!("{}", style), "Custom");
    }

    #[test]
    fn test_style_equality() {
        assert_eq!(Style::Vertical, Style::Vertical);
        assert_eq!(Style::Horizontal, Style::Horizontal);
        assert_eq!(Style::Custom, Style::Custom);
        assert_ne!(Style::Vertical, Style::Horizontal);
        assert_ne!(Style::Vertical, Style::Custom);
        assert_ne!(Style::Horizontal, Style::Custom);
    }

    #[test]
    fn test_constants() {
        assert_eq!(CUSTOM_SENSORS, 9);
        assert_eq!(DISPLAY_LINES, 3);
        assert_eq!(TICK_RATE, 1000);
    }
}

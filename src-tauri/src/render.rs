use embedded_graphics::mono_font::MonoFont;
use embedded_graphics::{
    image::{Image, ImageRaw},
    mono_font::MonoTextStyle,
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};
use image::ImageReader;
use profont::{PROFONT_12_POINT, PROFONT_18_POINT, PROFONT_9_POINT};

/// Font size preset for a single OLED display line (direct-USB render path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl FontSize {
    /// Parse a config string; unknown/empty → Medium.
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "small" => FontSize::Small,
            "large" => FontSize::Large,
            _ => FontSize::Medium,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FontSize::Small => "small",
            FontSize::Medium => "medium",
            FontSize::Large => "large",
        }
    }

    fn font(&self) -> &'static MonoFont<'static> {
        match self {
            FontSize::Small => &PROFONT_9_POINT,
            FontSize::Medium => &PROFONT_12_POINT,
            FontSize::Large => &PROFONT_18_POINT,
        }
    }

    /// Baseline y for the first line at this size. Medium=10 reproduces the
    /// original fixed layout so existing configs render identically.
    fn first_baseline(&self) -> i32 {
        match self {
            FontSize::Small => 8,
            FontSize::Medium => 10,
            FontSize::Large => 16,
        }
    }

    /// Vertical step added before drawing each subsequent line. Medium=12
    /// reproduces the original `y += 12` spacing.
    fn line_advance(&self) -> i32 {
        match self {
            FontSize::Small => 10,
            FontSize::Medium => 12,
            FontSize::Large => 20,
        }
    }
}

/// A buffer for the SteelSeries OLED screen (128x64)
pub struct OledBuffer {
    // 128 columns, 8 bytes per column (64 pixels / 8)
    pub data: [u8; 128 * 8],
}

impl OledBuffer {
    pub fn new() -> Self {
        Self {
            data: [0u8; 128 * 8],
        }
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, on: bool) {
        if x >= 128 || y >= 64 {
            return;
        }
        let col = x as usize;
        let byte_row = (y / 8) as usize;
        let bit = (y % 8) as u8;
        let idx = col * 8 + byte_row;

        if on {
            self.data[idx] |= 1 << bit;
        } else {
            self.data[idx] &= !(1 << bit);
        }
    }

    // clear is omitted as it is currently unused

    pub fn get_chunk(&self, x_offset: u8, width: u8) -> Vec<u8> {
        let mut chunk = Vec::with_capacity(width as usize * 8);
        for x in x_offset..(x_offset + width) {
            let start = x as usize * 8;
            chunk.extend_from_slice(&self.data[start..start + 8]);
        }
        chunk
    }
}

impl DrawTarget for OledBuffer {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels.into_iter() {
            if point.x >= 0 && point.x < 128 && point.y >= 0 && point.y < 64 {
                self.set_pixel(point.x as u32, point.y as u32, color.is_on());
            }
        }
        Ok(())
    }
}

impl OriginDimensions for OledBuffer {
    fn size(&self) -> Size {
        Size::new(128, 64)
    }
}

// Icon Bitmaps (8x8)
const ICON_FIRE: [u8; 8] = [0x18, 0x3c, 0x7e, 0xdb, 0xff, 0xff, 0x7e, 0x3c];
const ICON_FAN: [u8; 8] = [0x66, 0x66, 0x3c, 0xff, 0xff, 0x3c, 0x66, 0x66];
const ICON_BOLT: [u8; 8] = [0x08, 0x1c, 0x3e, 0x7f, 0x1e, 0x1c, 0x08, 0x00];
const ICON_CHART: [u8; 8] = [0x01, 0x03, 0x05, 0x09, 0x11, 0x21, 0x41, 0xff];
const ICON_DISK: [u8; 8] = [0x7e, 0x81, 0xbd, 0xa5, 0xa5, 0xbd, 0x81, 0x7e];

fn get_emoji_icon(emoji: &str) -> Option<&'static [u8; 8]> {
    if emoji.contains('🔥') {
        return Some(&ICON_FIRE);
    }
    if emoji.contains('❄') {
        return Some(&ICON_FAN);
    }
    if emoji.contains('⚡') {
        return Some(&ICON_BOLT);
    }
    if emoji.contains('📈') {
        return Some(&ICON_CHART);
    }
    if emoji.contains('💾') {
        return Some(&ICON_DISK);
    }
    None
}

pub fn render_text_to_oled(text: &str, x: i32, line_fonts: &[FontSize]) -> OledBuffer {
    let mut buffer = OledBuffer::new();

    let mut y = 0;
    for (i, line) in text.lines().enumerate() {
        let fs = line_fonts.get(i).copied().unwrap_or(FontSize::Medium);
        if i == 0 {
            y = fs.first_baseline();
        } else {
            y += fs.line_advance();
        }

        let style = MonoTextStyle::new(fs.font(), BinaryColor::On);
        let mut current_x = x;

        if let Some(rest) = line.strip_prefix("IMG:") {
            let path = rest.trim();
            let img_y = (y - 10).max(0) as u32;
            let _ = load_image_to_buffer(path, &mut buffer, current_x as u32, img_y);
        } else {
            if let Some(icon_data) = get_emoji_icon(line) {
                let raw_image = ImageRaw::<BinaryColor>::new(icon_data, 8);
                let image = Image::new(&raw_image, Point::new(current_x, y - 8));
                let _ = image.draw(&mut buffer);
                current_x += 10;
            }

            let clean_line: String = line.chars().filter(|c| !c.is_emoji()).collect();
            let _ = Text::new(clean_line.trim(), Point::new(current_x, y), style).draw(&mut buffer);
        }
    }

    buffer
}

pub fn load_image_to_buffer(
    path: &str,
    buffer: &mut OledBuffer,
    x_off: u32,
    y_off: u32,
) -> Result<(), anyhow::Error> {
    let img = ImageReader::open(path)
        .map_err(|e| anyhow::anyhow!("Failed to open image {}: {}", path, e))?
        .decode()
        .map_err(|e| anyhow::anyhow!("Failed to decode image {}: {}", path, e))?;

    let gray = img.to_luma8();
    let (width, height) = gray.dimensions();

    for y in 0..height {
        if y + y_off >= 64 {
            break;
        }
        for x in 0..width {
            if x + x_off >= 128 {
                break;
            }
            let pixel = gray.get_pixel(x, y);
            if pixel[0] > 128 {
                buffer.set_pixel(x + x_off, y + y_off, true);
            }
        }
    }
    Ok(())
}

pub trait UnicodeEmoji {
    fn is_emoji(&self) -> bool;
}

impl UnicodeEmoji for char {
    fn is_emoji(&self) -> bool {
        let u = *self as u32;
        (0x1F300..=0x1F9FF).contains(&u)
            || (0x2600..=0x26FF).contains(&u)
            || (0x2700..=0x27BF).contains(&u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===========================================
    // OledBuffer::new() tests
    // ===========================================

    #[test]
    fn test_oled_buffer_new_creates_correct_size() {
        let buffer = OledBuffer::new();
        // 128 columns * 8 bytes per column = 1024 bytes
        assert_eq!(buffer.data.len(), 128 * 8);
    }

    #[test]
    fn test_oled_buffer_new_initialized_to_zeros() {
        let buffer = OledBuffer::new();
        // All bytes should be zero (black/off)
        for byte in buffer.data.iter() {
            assert_eq!(*byte, 0);
        }
    }

    // ===========================================
    // OledBuffer::set_pixel() tests
    // ===========================================

    #[test]
    fn test_set_pixel_at_origin() {
        let mut buffer = OledBuffer::new();
        buffer.set_pixel(0, 0, true);

        // Pixel at (0,0) should set bit 0 of byte at index 0
        assert_eq!(buffer.data[0] & 1, 1);
    }

    #[test]
    fn test_set_pixel_at_max_bounds() {
        let mut buffer = OledBuffer::new();
        // Max valid coordinates are (127, 63) for 128x64 display
        buffer.set_pixel(127, 63, true);

        // Column 127, row 63 -> byte_row = 63/8 = 7, bit = 63%8 = 7
        // idx = 127 * 8 + 7 = 1023
        let idx = 127 * 8 + 7;
        assert_eq!(buffer.data[idx] & (1 << 7), 1 << 7);
    }

    #[test]
    fn test_set_pixel_out_of_bounds_x_does_not_panic() {
        let mut buffer = OledBuffer::new();
        // Should not panic, just be ignored
        buffer.set_pixel(128, 0, true);
        buffer.set_pixel(200, 0, true);

        // Buffer should remain all zeros
        for byte in buffer.data.iter() {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn test_set_pixel_out_of_bounds_y_does_not_panic() {
        let mut buffer = OledBuffer::new();
        // Should not panic, just be ignored
        buffer.set_pixel(0, 64, true);
        buffer.set_pixel(0, 100, true);

        // Buffer should remain all zeros
        for byte in buffer.data.iter() {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn test_set_pixel_can_turn_off() {
        let mut buffer = OledBuffer::new();
        buffer.set_pixel(5, 5, true);

        // Verify pixel is on
        let idx = 5 * 8; // byte_row = 5/8 = 0
        let bit = 5;
        assert_ne!(buffer.data[idx] & (1 << bit), 0);

        // Turn it off
        buffer.set_pixel(5, 5, false);
        assert_eq!(buffer.data[idx] & (1 << bit), 0);
    }

    #[test]
    fn test_set_pixel_different_y_positions() {
        let mut buffer = OledBuffer::new();

        // Set pixels at different y positions in same column
        buffer.set_pixel(0, 0, true); // bit 0 of byte 0
        buffer.set_pixel(0, 7, true); // bit 7 of byte 0
        buffer.set_pixel(0, 8, true); // bit 0 of byte 1

        assert_eq!(buffer.data[0], 0b10000001); // bits 0 and 7
        assert_eq!(buffer.data[1], 0b00000001); // bit 0
    }

    // ===========================================
    // OledBuffer::get_chunk() tests
    // ===========================================

    #[test]
    fn test_get_chunk_at_offset_zero() {
        let mut buffer = OledBuffer::new();
        buffer.set_pixel(0, 0, true);

        let chunk = buffer.get_chunk(0, 1);
        // Should get 8 bytes for 1 column
        assert_eq!(chunk.len(), 8);
        assert_eq!(chunk[0], 1); // First byte should have bit 0 set
    }

    #[test]
    fn test_get_chunk_multiple_columns() {
        let buffer = OledBuffer::new();

        let chunk = buffer.get_chunk(0, 10);
        // 10 columns * 8 bytes = 80 bytes
        assert_eq!(chunk.len(), 80);
    }

    #[test]
    fn test_get_chunk_at_middle_offset() {
        let mut buffer = OledBuffer::new();
        buffer.set_pixel(64, 0, true);

        let chunk = buffer.get_chunk(64, 1);
        assert_eq!(chunk.len(), 8);
        assert_eq!(chunk[0], 1); // First byte of this column should have bit 0 set
    }

    #[test]
    fn test_get_chunk_preserves_data() {
        let mut buffer = OledBuffer::new();
        // Set a pattern in column 5
        buffer.set_pixel(5, 0, true);
        buffer.set_pixel(5, 1, true);
        buffer.set_pixel(5, 2, true);

        let chunk = buffer.get_chunk(5, 1);
        assert_eq!(chunk[0], 0b00000111); // bits 0, 1, 2 set
    }

    // ===========================================
    // get_emoji_icon() tests
    // ===========================================

    #[test]
    fn test_get_emoji_icon_fire() {
        let result = get_emoji_icon("🔥");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &ICON_FIRE);
    }

    #[test]
    fn test_get_emoji_icon_snowflake() {
        let result = get_emoji_icon("❄");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &ICON_FAN);
    }

    #[test]
    fn test_get_emoji_icon_bolt() {
        let result = get_emoji_icon("⚡");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &ICON_BOLT);
    }

    #[test]
    fn test_get_emoji_icon_chart() {
        let result = get_emoji_icon("📈");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &ICON_CHART);
    }

    #[test]
    fn test_get_emoji_icon_disk() {
        let result = get_emoji_icon("💾");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &ICON_DISK);
    }

    #[test]
    fn test_get_emoji_icon_unknown_returns_none() {
        let result = get_emoji_icon("hello");
        assert!(result.is_none());
    }

    #[test]
    fn test_get_emoji_icon_empty_string_returns_none() {
        let result = get_emoji_icon("");
        assert!(result.is_none());
    }

    #[test]
    fn test_get_emoji_icon_with_text_containing_emoji() {
        // The function uses contains(), so text with emoji should match
        let result = get_emoji_icon("Temperature 🔥");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &ICON_FIRE);
    }

    // ===========================================
    // render_text_to_oled() tests
    // ===========================================

    #[test]
    fn test_render_text_empty_string() {
        let buffer = render_text_to_oled("", 0, &[]);
        // Should return a valid buffer
        assert_eq!(buffer.data.len(), 128 * 8);
    }

    #[test]
    fn test_render_text_simple_ascii() {
        let buffer = render_text_to_oled("Hello", 0, &[]);
        // Buffer should have some pixels set (not all zeros)
        let has_pixels = buffer.data.iter().any(|&b| b != 0);
        assert!(has_pixels, "Text should render some pixels");
    }

    #[test]
    fn test_render_text_multiple_lines() {
        let buffer = render_text_to_oled("Line1\nLine2\nLine3", 0, &[]);
        let has_pixels = buffer.data.iter().any(|&b| b != 0);
        assert!(has_pixels, "Multi-line text should render some pixels");
    }

    #[test]
    fn test_render_text_with_emoji() {
        let buffer = render_text_to_oled("🔥 Hot", 0, &[]);
        let has_pixels = buffer.data.iter().any(|&b| b != 0);
        assert!(has_pixels, "Text with emoji should render some pixels");
    }

    #[test]
    fn test_render_text_correct_buffer_dimensions() {
        let buffer = render_text_to_oled("Test", 0, &[]);
        // OriginDimensions should report 128x64
        assert_eq!(buffer.size(), Size::new(128, 64));
    }

    #[test]
    fn test_render_text_with_x_offset() {
        let buffer_no_offset = render_text_to_oled("A", 0, &[]);
        let buffer_with_offset = render_text_to_oled("A", 50, &[]);

        // Both should have pixels, but in different positions
        let has_pixels_no_offset = buffer_no_offset.data.iter().any(|&b| b != 0);
        let has_pixels_with_offset = buffer_with_offset.data.iter().any(|&b| b != 0);

        assert!(has_pixels_no_offset);
        assert!(has_pixels_with_offset);

        // The buffers should be different due to different x positions
        assert_ne!(buffer_no_offset.data, buffer_with_offset.data);
    }

    // ===========================================
    // UnicodeEmoji trait tests
    // ===========================================

    #[test]
    fn test_is_emoji_fire() {
        assert!('🔥'.is_emoji());
    }

    #[test]
    fn test_is_emoji_snowflake() {
        assert!('❄'.is_emoji());
    }

    #[test]
    fn test_is_emoji_bolt() {
        assert!('⚡'.is_emoji());
    }

    #[test]
    fn test_is_emoji_chart() {
        assert!('📈'.is_emoji());
    }

    #[test]
    fn test_is_emoji_disk() {
        assert!('💾'.is_emoji());
    }

    #[test]
    fn test_is_emoji_ascii_letter_false() {
        assert!(!('A'.is_emoji()));
        assert!(!('z'.is_emoji()));
    }

    #[test]
    fn test_is_emoji_digit_false() {
        assert!(!('0'.is_emoji()));
        assert!(!('9'.is_emoji()));
    }

    #[test]
    fn test_is_emoji_space_false() {
        assert!(!(' '.is_emoji()));
    }

    #[test]
    fn test_is_emoji_punctuation_false() {
        assert!(!('.'.is_emoji()));
        assert!(!('!'.is_emoji()));
        assert!(!('?'.is_emoji()));
    }

    #[test]
    fn test_is_emoji_various_emojis() {
        // Test emojis in different unicode ranges
        assert!('☀'.is_emoji()); // U+2600 range (Miscellaneous Symbols)
        assert!('✂'.is_emoji()); // U+2700 range (Dingbats)
        assert!('🌀'.is_emoji()); // U+1F300 range (Miscellaneous Symbols and Pictographs)
    }

    // ===========================================
    // DrawTarget implementation tests
    // ===========================================

    #[test]
    fn test_draw_target_size() {
        let buffer = OledBuffer::new();
        let size = buffer.size();
        assert_eq!(size.width, 128);
        assert_eq!(size.height, 64);
    }

    #[test]
    fn test_draw_target_draw_iter() {
        use embedded_graphics::prelude::*;

        let mut buffer = OledBuffer::new();
        let pixels = vec![
            Pixel(Point::new(10, 10), BinaryColor::On),
            Pixel(Point::new(20, 20), BinaryColor::On),
        ];

        let result = buffer.draw_iter(pixels);
        assert!(result.is_ok());

        // Verify pixels were set
        // For (10, 10): col=10, byte_row=10/8=1, bit=10%8=2, idx=10*8+1=81
        let idx1 = 10 * 8 + 1;
        assert_ne!(buffer.data[idx1] & (1 << 2), 0);

        // For (20, 20): col=20, byte_row=20/8=2, bit=20%8=4, idx=20*8+2=162
        let idx2 = 20 * 8 + 2;
        assert_ne!(buffer.data[idx2] & (1 << 4), 0);
    }

    #[test]
    fn test_draw_target_ignores_negative_coordinates() {
        let mut buffer = OledBuffer::new();
        let pixels = vec![
            Pixel(Point::new(-1, 0), BinaryColor::On),
            Pixel(Point::new(0, -1), BinaryColor::On),
        ];

        let result = buffer.draw_iter(pixels);
        assert!(result.is_ok());

        // Buffer should remain all zeros
        for byte in buffer.data.iter() {
            assert_eq!(*byte, 0);
        }
    }

    fn write_test_image(path: &std::path::Path, w: u32, h: u32, brightness: u8) {
        let mut img = image::GrayImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.put_pixel(x, y, image::Luma([brightness]));
            }
        }
        img.save(path).unwrap();
    }

    #[test]
    fn test_load_image_to_buffer_lights_pixels_when_bright() {
        let dir = std::env::temp_dir();
        let path = dir.join("hwinfo_ss_render_bright.png");
        write_test_image(&path, 4, 4, 255);

        let mut buffer = OledBuffer::new();
        load_image_to_buffer(&path.to_string_lossy(), &mut buffer, 0, 0).unwrap();

        // Pixels (0,0)..(3,3) should be on
        let any_lit = buffer.data.iter().any(|b| *b != 0);
        assert!(any_lit, "Bright pixels should light the buffer");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_image_to_buffer_dark_pixels_left_blank() {
        let dir = std::env::temp_dir();
        let path = dir.join("hwinfo_ss_render_dark.png");
        write_test_image(&path, 4, 4, 50);

        let mut buffer = OledBuffer::new();
        load_image_to_buffer(&path.to_string_lossy(), &mut buffer, 0, 0).unwrap();

        assert!(buffer.data.iter().all(|b| *b == 0));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_image_to_buffer_clips_to_bounds() {
        // 200x100 image at offset 0,0 — must clip to 128x64 without OOB writes
        let dir = std::env::temp_dir();
        let path = dir.join("hwinfo_ss_render_clip.png");
        write_test_image(&path, 200, 100, 255);

        let mut buffer = OledBuffer::new();
        load_image_to_buffer(&path.to_string_lossy(), &mut buffer, 0, 0).unwrap();

        // Buffer length unchanged (no panic from clipping)
        assert_eq!(buffer.data.len(), 128 * 8);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_image_to_buffer_missing_file_returns_err() {
        let mut buffer = OledBuffer::new();
        let r = load_image_to_buffer("/no/such/path/xyz.png", &mut buffer, 0, 0);
        assert!(r.is_err());
    }

    #[test]
    fn test_render_text_img_directive_invokes_loader() {
        let dir = std::env::temp_dir();
        let path = dir.join("hwinfo_ss_render_img.png");
        write_test_image(&path, 4, 4, 255);

        let text = format!("IMG:{}", path.to_string_lossy());
        let buffer = render_text_to_oled(&text, 0, &[]);
        let any_lit = buffer.data.iter().any(|b| *b != 0);
        assert!(any_lit);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_draw_target_ignores_out_of_bounds() {
        let mut buffer = OledBuffer::new();
        let pixels = vec![
            Pixel(Point::new(128, 0), BinaryColor::On),
            Pixel(Point::new(0, 64), BinaryColor::On),
        ];

        let result = buffer.draw_iter(pixels);
        assert!(result.is_ok());

        // Buffer should remain all zeros
        for byte in buffer.data.iter() {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn test_fontsize_from_config_str_known_values() {
        assert_eq!(FontSize::from_config_str("small"), FontSize::Small);
        assert_eq!(FontSize::from_config_str("medium"), FontSize::Medium);
        assert_eq!(FontSize::from_config_str("large"), FontSize::Large);
    }

    #[test]
    fn test_fontsize_from_config_str_case_and_whitespace() {
        assert_eq!(FontSize::from_config_str("  LARGE "), FontSize::Large);
    }

    #[test]
    fn test_fontsize_from_config_str_unknown_defaults_medium() {
        assert_eq!(FontSize::from_config_str("huge"), FontSize::Medium);
        assert_eq!(FontSize::from_config_str(""), FontSize::Medium);
    }

    #[test]
    fn test_fontsize_as_str_roundtrips() {
        for fs in [FontSize::Small, FontSize::Medium, FontSize::Large] {
            assert_eq!(FontSize::from_config_str(fs.as_str()), fs);
        }
    }

    #[test]
    fn test_fontsize_default_is_medium() {
        assert_eq!(FontSize::default(), FontSize::Medium);
    }

    #[test]
    fn test_render_small_vs_large_differ() {
        let small = render_text_to_oled("Hello", 0, &[FontSize::Small]);
        let large = render_text_to_oled("Hello", 0, &[FontSize::Large]);
        assert_ne!(small.data, large.data);
    }

    #[test]
    fn test_render_mixed_line_fonts_no_panic_and_lit() {
        let buf = render_text_to_oled(
            "Big\nsmall\nmed",
            0,
            &[FontSize::Large, FontSize::Small, FontSize::Medium],
        );
        assert!(buf.data.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_render_fewer_fonts_than_lines_falls_back_medium() {
        let buf = render_text_to_oled("a\nb\nc", 0, &[FontSize::Small]);
        assert!(buf.data.iter().any(|&b| b != 0));
    }
}

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

/// A buffer for a SteelSeries OLED screen. Layout is column-major pages:
/// for each column, `height/8` bytes; within a byte, bit 0 is the topmost pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct OledBuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl OledBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        debug_assert!(
            height.is_multiple_of(8),
            "OLED height must be a multiple of 8"
        );
        Self {
            width,
            height,
            data: vec![0u8; (width * height / 8) as usize],
        }
    }

    /// Bytes per column (= height / 8).
    fn pages(&self) -> usize {
        (self.height / 8) as usize
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, on: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        let pages = self.pages();
        let idx = x as usize * pages + (y / 8) as usize;
        let bit = (y % 8) as u8;

        if on {
            self.data[idx] |= 1 << bit;
        } else {
            self.data[idx] &= !(1 << bit);
        }
    }

    // clear is omitted as it is currently unused

    /// Serialize to SSD1306 page-major order: all columns of page 0, then
    /// page 1, etc. (The internal layout is column-major pages.) Used by the
    /// Apex legacy protocol.
    pub fn to_page_major(&self) -> Vec<u8> {
        let pages = self.pages();
        let mut out = Vec::with_capacity(self.data.len());
        for page in 0..pages {
            for x in 0..self.width as usize {
                out.push(self.data[x * pages + page]);
            }
        }
        out
    }

    pub fn get_chunk(&self, x_offset: u8, width: u8) -> Vec<u8> {
        let pages = self.pages();
        let mut chunk = Vec::with_capacity(width as usize * pages);
        for x in x_offset..(x_offset + width) {
            let start = x as usize * pages;
            chunk.extend_from_slice(&self.data[start..start + pages]);
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
            if point.x >= 0
                && point.x < self.width as i32
                && point.y >= 0
                && point.y < self.height as i32
            {
                self.set_pixel(point.x as u32, point.y as u32, color.is_on());
            }
        }
        Ok(())
    }
}

impl OriginDimensions for OledBuffer {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}

// Icon Bitmaps (8x8). Each byte is one row, MSB = leftmost pixel.
const ICON_FIRE: [u8; 8] = [0x18, 0x3c, 0x7e, 0xdb, 0xff, 0xff, 0x7e, 0x3c];
const ICON_FAN: [u8; 8] = [0x66, 0x66, 0x3c, 0xff, 0xff, 0x3c, 0x66, 0x66];
const ICON_BOLT: [u8; 8] = [0x08, 0x1c, 0x3e, 0x7f, 0x1e, 0x1c, 0x08, 0x00];
const ICON_CHART: [u8; 8] = [0x01, 0x03, 0x05, 0x09, 0x11, 0x21, 0x41, 0xff];
const ICON_DISK: [u8; 8] = [0x7e, 0x81, 0xbd, 0xa5, 0xa5, 0xbd, 0x81, 0x7e];
const ICON_CPU: [u8; 8] = [0x18, 0x7e, 0x42, 0x5a, 0x5a, 0x42, 0x7e, 0x18];
const ICON_GPU: [u8; 8] = [0xff, 0x81, 0xbd, 0xa5, 0xbd, 0x81, 0xff, 0x24];
const ICON_MEM: [u8; 8] = [0x7e, 0xff, 0xdb, 0xdb, 0xdb, 0xff, 0x66, 0x66];
const ICON_TEMP: [u8; 8] = [0x18, 0x24, 0x24, 0x24, 0x3c, 0x7e, 0x7e, 0x3c];
const ICON_CLOCK: [u8; 8] = [0x3c, 0x42, 0x89, 0x89, 0x8f, 0x81, 0x42, 0x3c];
const ICON_NET: [u8; 8] = [0x18, 0x3c, 0x7e, 0x18, 0x18, 0x7e, 0x3c, 0x18];

/// Resolve a builtin icon name (from the per-sensor `icon` config field) to its
/// 8x8 bitmap. Case-insensitive; surrounding whitespace ignored. Unknown or
/// empty names return None (no icon drawn). Several names alias the legacy
/// emoji glyphs so existing artwork is reused.
pub fn icon_by_name(name: &str) -> Option<&'static [u8; 8]> {
    match name.trim().to_lowercase().as_str() {
        "cpu" => Some(&ICON_CPU),
        "gpu" => Some(&ICON_GPU),
        "mem" | "ram" => Some(&ICON_MEM),
        "temp" => Some(&ICON_TEMP),
        "clock" => Some(&ICON_CLOCK),
        "net" => Some(&ICON_NET),
        "fan" => Some(&ICON_FAN),
        "disk" => Some(&ICON_DISK),
        "power" | "bolt" => Some(&ICON_BOLT),
        "usage" | "chart" => Some(&ICON_CHART),
        "fire" => Some(&ICON_FIRE),
        _ => None,
    }
}

/// Delimiter wrapping an inline icon name inside a display line, e.g.
/// `"\u{1}cpu\u{1}42°C"`. Uses SOH (U+0001) so it can never collide with a
/// user label, sensor value, or unit. `format_custom_value` emits these; the
/// renderer parses them out.
pub const ICON_DELIM: char = '\u{1}';

/// Wrap a builtin icon name in [`ICON_DELIM`] so it survives line joining and
/// is recognised by [`render_text_to_oled`].
pub fn icon_token(name: &str) -> String {
    format!("{d}{n}{d}", d = ICON_DELIM, n = name)
}

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

pub fn render_text_to_oled(
    text: &str,
    x: i32,
    line_fonts: &[FontSize],
    width: u32,
    height: u32,
) -> OledBuffer {
    let mut buffer = OledBuffer::new(width, height);

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
        } else if line.contains(ICON_DELIM) {
            // Inline icon tokens: text and `\u{1}name\u{1}` icons interleave.
            // Splitting on the delimiter yields alternating segments — even
            // indices are text, odd indices are icon names.
            for (seg_idx, seg) in line.split(ICON_DELIM).enumerate() {
                if seg_idx % 2 == 1 {
                    if let Some(icon_data) = icon_by_name(seg) {
                        let raw_image = ImageRaw::<BinaryColor>::new(icon_data, 8);
                        let image = Image::new(&raw_image, Point::new(current_x, y - 8));
                        let _ = image.draw(&mut buffer);
                        current_x += 10;
                    }
                } else if !seg.is_empty() {
                    current_x = Text::new(seg, Point::new(current_x, y), style)
                        .draw(&mut buffer)
                        .map(|next| next.x)
                        .unwrap_or(current_x);
                }
            }
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
        if y + y_off >= buffer.height {
            break;
        }
        for x in 0..width {
            if x + x_off >= buffer.width {
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
        let buffer = OledBuffer::new(128, 64);
        // 128 columns * 8 bytes per column = 1024 bytes
        assert_eq!(buffer.data.len(), 128 * 8);
    }

    #[test]
    fn test_oled_buffer_new_initialized_to_zeros() {
        let buffer = OledBuffer::new(128, 64);
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
        let mut buffer = OledBuffer::new(128, 64);
        buffer.set_pixel(0, 0, true);

        // Pixel at (0,0) should set bit 0 of byte at index 0
        assert_eq!(buffer.data[0] & 1, 1);
    }

    #[test]
    fn test_set_pixel_at_max_bounds() {
        let mut buffer = OledBuffer::new(128, 64);
        // Max valid coordinates are (127, 63) for 128x64 display
        buffer.set_pixel(127, 63, true);

        // Column 127, row 63 -> byte_row = 63/8 = 7, bit = 63%8 = 7
        // idx = 127 * 8 + 7 = 1023
        let idx = 127 * 8 + 7;
        assert_eq!(buffer.data[idx] & (1 << 7), 1 << 7);
    }

    #[test]
    fn test_set_pixel_out_of_bounds_x_does_not_panic() {
        let mut buffer = OledBuffer::new(128, 64);
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
        let mut buffer = OledBuffer::new(128, 64);
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
        let mut buffer = OledBuffer::new(128, 64);
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
        let mut buffer = OledBuffer::new(128, 64);

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
        let mut buffer = OledBuffer::new(128, 64);
        buffer.set_pixel(0, 0, true);

        let chunk = buffer.get_chunk(0, 1);
        // Should get 8 bytes for 1 column
        assert_eq!(chunk.len(), 8);
        assert_eq!(chunk[0], 1); // First byte should have bit 0 set
    }

    #[test]
    fn test_get_chunk_multiple_columns() {
        let buffer = OledBuffer::new(128, 64);

        let chunk = buffer.get_chunk(0, 10);
        // 10 columns * 8 bytes = 80 bytes
        assert_eq!(chunk.len(), 80);
    }

    #[test]
    fn test_get_chunk_at_middle_offset() {
        let mut buffer = OledBuffer::new(128, 64);
        buffer.set_pixel(64, 0, true);

        let chunk = buffer.get_chunk(64, 1);
        assert_eq!(chunk.len(), 8);
        assert_eq!(chunk[0], 1); // First byte of this column should have bit 0 set
    }

    #[test]
    fn test_get_chunk_preserves_data() {
        let mut buffer = OledBuffer::new(128, 64);
        // Set a pattern in column 5
        buffer.set_pixel(5, 0, true);
        buffer.set_pixel(5, 1, true);
        buffer.set_pixel(5, 2, true);

        let chunk = buffer.get_chunk(5, 1);
        assert_eq!(chunk[0], 0b00000111); // bits 0, 1, 2 set
    }

    // ===========================================
    // Resolution-aware OledBuffer tests
    // ===========================================

    #[test]
    fn test_new_sizes_buffer_for_dimensions() {
        let b64 = OledBuffer::new(128, 64);
        assert_eq!(b64.width, 128);
        assert_eq!(b64.height, 64);
        assert_eq!(b64.data.len(), 128 * 64 / 8); // 1024

        let b40 = OledBuffer::new(128, 40);
        assert_eq!(b40.data.len(), 128 * 40 / 8); // 640
    }

    #[test]
    fn test_set_pixel_respects_instance_bounds() {
        let mut b40 = OledBuffer::new(128, 40);
        b40.set_pixel(0, 39, true); // in bounds
        b40.set_pixel(0, 40, true); // out of bounds for 40-tall: no-op, no panic
        b40.set_pixel(127, 0, true);
        b40.set_pixel(128, 0, true); // no-op

        // (0, 39): column 0, page 4, bit 7
        assert_eq!(b40.data[4], 0x80);
        // (127, 0): column 127, page 0, bit 0; 5 pages per column
        assert_eq!(b40.data[127 * 5], 0x01);
    }

    #[test]
    fn test_get_chunk_uses_instance_pages() {
        let mut b40 = OledBuffer::new(128, 40);
        b40.set_pixel(2, 0, true);
        let chunk = b40.get_chunk(0, 4); // 4 columns × 5 pages
        assert_eq!(chunk.len(), 20);
        assert_eq!(chunk[2 * 5], 0x01);
    }

    #[test]
    fn test_buffer_clone_is_deep() {
        let mut a = OledBuffer::new(128, 64);
        a.set_pixel(1, 1, true);
        let b = a.clone();
        assert_eq!(a, b);
        a.set_pixel(2, 2, true);
        assert_ne!(a, b);
    }

    // ===========================================
    // OledBuffer::to_page_major() tests
    // ===========================================

    #[test]
    fn test_to_page_major_length_and_ordering() {
        let mut b = OledBuffer::new(128, 40);
        // (0,0) → page 0, column 0, bit 0 → out[0] = 0x01
        b.set_pixel(0, 0, true);
        // (5,9) → page 1, column 5, bit 1 → out[1*128 + 5] = 0x02
        b.set_pixel(5, 9, true);
        // (127,39) → page 4, column 127, bit 7 → out[4*128 + 127] = 0x80
        b.set_pixel(127, 39, true);

        let out = b.to_page_major();
        assert_eq!(out.len(), 640);
        assert_eq!(out[0], 0x01);
        assert_eq!(out[128 + 5], 0x02);
        assert_eq!(out[4 * 128 + 127], 0x80);
        // Exactly the three set pixels are non-zero — no smearing/duplication.
        assert_eq!(out.iter().filter(|&&b| b != 0).count(), 3);
    }

    #[test]
    fn test_to_page_major_empty_buffer_is_zeroes() {
        let b = OledBuffer::new(128, 64);
        let out = b.to_page_major();
        assert_eq!(out.len(), 1024);
        assert!(out.iter().all(|&byte| byte == 0));
    }

    // ===========================================
    // icon_by_name() tests
    // ===========================================

    #[test]
    fn test_icon_by_name_known_builtins_resolve() {
        for name in [
            "cpu", "gpu", "mem", "temp", "fan", "clock", "disk", "power", "usage", "net",
        ] {
            assert!(
                icon_by_name(name).is_some(),
                "builtin icon '{}' should resolve",
                name
            );
        }
    }

    #[test]
    fn test_icon_by_name_is_case_insensitive_and_trims() {
        assert_eq!(icon_by_name("  CPU "), icon_by_name("cpu"));
    }

    #[test]
    fn test_icon_by_name_unknown_returns_none() {
        assert!(icon_by_name("not-an-icon").is_none());
        assert!(icon_by_name("").is_none());
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
    // inline icon token tests
    // ===========================================

    #[test]
    fn test_icon_token_roundtrips_with_icon_by_name() {
        let tok = icon_token("cpu");
        // The token wraps the name in delimiters so it survives line joining.
        assert!(tok.starts_with(ICON_DELIM));
        assert!(tok.ends_with(ICON_DELIM));
        assert!(tok.contains("cpu"));
    }

    #[test]
    fn test_render_inline_icon_token_lights_pixels() {
        let text = format!("{}Temp", icon_token("cpu"));
        let buf = render_text_to_oled(&text, 0, &[], 128, 64);
        assert!(buf.data.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_render_inline_icon_differs_from_plain_text() {
        let with_icon = render_text_to_oled(&format!("{}42", icon_token("cpu")), 0, &[], 128, 64);
        let plain = render_text_to_oled("42", 0, &[], 128, 64);
        assert_ne!(with_icon.data, plain.data);
    }

    #[test]
    fn test_render_unknown_icon_token_renders_following_text() {
        // Unknown icon name: no glyph, but the trailing text still renders and
        // the delimiter chars must not appear as literal glyphs.
        let with_bogus =
            render_text_to_oled(&format!("{}Hi", icon_token("bogus")), 0, &[], 128, 64);
        let plain = render_text_to_oled("Hi", 0, &[], 128, 64);
        assert!(with_bogus.data.iter().any(|&b| b != 0));
        assert_eq!(with_bogus.data, plain.data);
    }

    // ===========================================
    // render_text_to_oled() tests
    // ===========================================

    #[test]
    fn test_render_text_empty_string() {
        let buffer = render_text_to_oled("", 0, &[], 128, 64);
        // Should return a valid buffer
        assert_eq!(buffer.data.len(), 128 * 8);
    }

    #[test]
    fn test_render_text_simple_ascii() {
        let buffer = render_text_to_oled("Hello", 0, &[], 128, 64);
        // Buffer should have some pixels set (not all zeros)
        let has_pixels = buffer.data.iter().any(|&b| b != 0);
        assert!(has_pixels, "Text should render some pixels");
    }

    #[test]
    fn test_render_text_multiple_lines() {
        let buffer = render_text_to_oled("Line1\nLine2\nLine3", 0, &[], 128, 64);
        let has_pixels = buffer.data.iter().any(|&b| b != 0);
        assert!(has_pixels, "Multi-line text should render some pixels");
    }

    #[test]
    fn test_render_text_with_emoji() {
        let buffer = render_text_to_oled("🔥 Hot", 0, &[], 128, 64);
        let has_pixels = buffer.data.iter().any(|&b| b != 0);
        assert!(has_pixels, "Text with emoji should render some pixels");
    }

    #[test]
    fn test_render_text_correct_buffer_dimensions() {
        let buffer = render_text_to_oled("Test", 0, &[], 128, 64);
        // OriginDimensions should report 128x64
        assert_eq!(buffer.size(), Size::new(128, 64));
    }

    #[test]
    fn test_render_text_with_x_offset() {
        let buffer_no_offset = render_text_to_oled("A", 0, &[], 128, 64);
        let buffer_with_offset = render_text_to_oled("A", 50, &[], 128, 64);

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
        let buffer = OledBuffer::new(128, 64);
        let size = buffer.size();
        assert_eq!(size.width, 128);
        assert_eq!(size.height, 64);
    }

    #[test]
    fn test_draw_target_draw_iter() {
        use embedded_graphics::prelude::*;

        let mut buffer = OledBuffer::new(128, 64);
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
        let mut buffer = OledBuffer::new(128, 64);
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

        let mut buffer = OledBuffer::new(128, 64);
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

        let mut buffer = OledBuffer::new(128, 64);
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

        let mut buffer = OledBuffer::new(128, 64);
        load_image_to_buffer(&path.to_string_lossy(), &mut buffer, 0, 0).unwrap();

        // Buffer length unchanged (no panic from clipping)
        assert_eq!(buffer.data.len(), 128 * 8);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_image_to_buffer_missing_file_returns_err() {
        let mut buffer = OledBuffer::new(128, 64);
        let r = load_image_to_buffer("/no/such/path/xyz.png", &mut buffer, 0, 0);
        assert!(r.is_err());
    }

    #[test]
    fn test_render_text_img_directive_invokes_loader() {
        let dir = std::env::temp_dir();
        let path = dir.join("hwinfo_ss_render_img.png");
        write_test_image(&path, 4, 4, 255);

        let text = format!("IMG:{}", path.to_string_lossy());
        let buffer = render_text_to_oled(&text, 0, &[], 128, 64);
        let any_lit = buffer.data.iter().any(|b| *b != 0);
        assert!(any_lit);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_draw_target_ignores_out_of_bounds() {
        let mut buffer = OledBuffer::new(128, 64);
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
        let small = render_text_to_oled("Hello", 0, &[FontSize::Small], 128, 64);
        let large = render_text_to_oled("Hello", 0, &[FontSize::Large], 128, 64);
        assert_ne!(small.data, large.data);
    }

    #[test]
    fn test_render_mixed_line_fonts_no_panic_and_lit() {
        let buf = render_text_to_oled(
            "Big\nsmall\nmed",
            0,
            &[FontSize::Large, FontSize::Small, FontSize::Medium],
            128,
            64,
        );
        assert!(buf.data.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_render_text_at_explicit_dimensions() {
        let buf = render_text_to_oled("Hi", 0, &[], 128, 40);
        assert_eq!(buf.width, 128);
        assert_eq!(buf.height, 40);
        assert_eq!(buf.data.len(), 640);
        // Something was drawn
        assert!(buf.data.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_render_clips_safely_on_short_screen() {
        // 5 lines of Large font massively overflow 40px; must not panic.
        let buf = render_text_to_oled("A\nB\nC\nD\nE", 0, &[FontSize::Large; 5], 128, 40);
        assert_eq!(buf.data.len(), 640);
    }

    #[test]
    fn test_render_fewer_fonts_than_lines_falls_back_medium() {
        let buf = render_text_to_oled("a\nb\nc", 0, &[FontSize::Small], 128, 64);
        assert!(buf.data.iter().any(|&b| b != 0));
    }
}

use embedded_graphics::{
    image::{Image, ImageRaw},
    mono_font::MonoTextStyle,
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};
use image::ImageReader;
use profont::PROFONT_12_POINT;

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

pub fn render_text_to_oled(text: &str, x: i32) -> OledBuffer {
    let mut buffer = OledBuffer::new();
    let style = MonoTextStyle::new(&PROFONT_12_POINT, BinaryColor::On);

    let mut y = 10;
    for line in text.lines() {
        let mut current_x = x;

        if line.starts_with("IMG:") {
            let path = line[4..].trim();
            let _ = load_image_to_buffer(path, &mut buffer, current_x as u32, (y - 10) as u32);
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
        y += 12;
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

//! `fonts.mul` (ASCII) + `unifont*.mul` (Unicode). ClassicUO `FontsLoader`.
//!
//! ASCII: up to ~10 fonts, each a header byte then 224 glyphs (codes 32..255)
//! of `width, height, unk` + `width*height` little-endian ARGB1555 pixels.
//! Unicode: a 0x10000-entry little-endian lookup table of file offsets; each
//! glyph is `xoff, yoff, w, h` (i8) then 1-bit packed rows.

use std::path::Path;

use crate::art::Image;

/// One ASCII font's glyphs, indexed by `ch.wrapping_sub(32)` for printable
/// characters (ClassicUO walks 224 slots starting at space).
pub struct AsciiFont {
    pub glyphs: Vec<Option<Image>>,
}

pub struct UnicodeFont {
    data: Vec<u8>,
}

pub struct Fonts {
    pub ascii: Vec<AsciiFont>,
    unicode: Vec<Option<UnicodeFont>>,
}

impl Fonts {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = resource_dir.as_ref();
        let ascii = parse_ascii(&std::fs::read(dir.join("fonts.mul"))?);
        let mut unicode = Vec::with_capacity(20);
        for i in 0..20 {
            let name = if i == 0 {
                "unifont.mul".to_string()
            } else {
                format!("unifont{i}.mul")
            };
            unicode.push(
                std::fs::read(dir.join(&name))
                    .ok()
                    .map(|data| UnicodeFont { data }),
            );
        }
        if unicode.get(1).is_some_and(|u| u.is_none()) {
            unicode[1] = unicode.first().cloned().flatten();
        }
        Ok(Self { ascii, unicode })
    }

    pub fn ascii_glyph(&self, font: usize, ch: u8) -> Option<&Image> {
        let slot = ch.saturating_sub(32) as usize;
        self.ascii.get(font)?.glyphs.get(slot)?.as_ref()
    }

    pub fn unicode_glyph(&self, font: usize, ch: u32) -> Option<Image> {
        self.unicode.get(font)?.as_ref()?.glyph(ch)
    }

    /// Rasterize `text` with the given font. Unicode fonts are used when
    /// `unicode` is true (ClassicUO's overhead/journal path); ASCII otherwise.
    /// Caps at 128 characters so a runaway journal line cannot allocate a
    /// megapixel strip.
    pub fn render_text(&self, font: usize, unicode: bool, text: &str) -> Option<Image> {
        let text: String = text.chars().take(128).collect();
        if text.is_empty() {
            return None;
        }
        let mut glyphs: Vec<Image> = Vec::new();
        if unicode {
            for ch in text.chars() {
                if let Some(g) = self.unicode_glyph(font, ch as u32) {
                    glyphs.push(g);
                } else if ch == ' ' {
                    glyphs.push(Image {
                        width: 4,
                        height: 12,
                        rgba: vec![0; 4 * 12 * 4],
                    });
                }
            }
        } else {
            for b in text.bytes() {
                if let Some(g) = self.ascii_glyph(font, b) {
                    glyphs.push(g.clone());
                }
            }
        }
        if glyphs.is_empty() {
            return None;
        }
        let height = glyphs.iter().map(|g| g.height).max().unwrap_or(1);
        let width: u32 = glyphs
            .iter()
            .map(|g| g.width.saturating_add(1))
            .sum::<u32>()
            .saturating_sub(1)
            .max(1);
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        let mut x = 0u32;
        for g in &glyphs {
            let gy = height.saturating_sub(g.height);
            for yy in 0..g.height {
                for xx in 0..g.width {
                    let si = ((yy * g.width + xx) * 4) as usize;
                    if g.rgba.get(si + 3).copied().unwrap_or(0) == 0 {
                        continue;
                    }
                    let dx = x + xx;
                    let dy = gy + yy;
                    if dx < width && dy < height {
                        let di = ((dy * width + dx) * 4) as usize;
                        rgba[di..di + 4].copy_from_slice(&g.rgba[si..si + 4]);
                    }
                }
            }
            x = x.saturating_add(g.width.saturating_add(1));
        }
        Some(Image {
            width,
            height,
            rgba,
        })
    }
}

impl Clone for UnicodeFont {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}

fn parse_ascii(data: &[u8]) -> Vec<AsciiFont> {
    let mut fonts = Vec::new();
    let mut p = 0usize;
    while p < data.len() {
        p += 1; // per-font header byte
        let mut glyphs = Vec::with_capacity(224);
        let mut ok = true;
        for _ in 0..224 {
            if p + 3 > data.len() {
                ok = false;
                break;
            }
            let w = data[p] as usize;
            let h = data[p + 1] as usize;
            p += 3;
            let n = w.saturating_mul(h).saturating_mul(2);
            if p + n > data.len() {
                ok = false;
                break;
            }
            glyphs.push(if w == 0 || h == 0 {
                None
            } else {
                Some(decode_ascii_glyph(w, h, &data[p..p + n]))
            });
            p += n;
        }
        if !ok {
            break;
        }
        fonts.push(AsciiFont { glyphs });
    }
    fonts
}

fn decode_ascii_glyph(w: usize, h: usize, px: &[u8]) -> Image {
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let c = u16::from_le_bytes([px[i * 2], px[i * 2 + 1]]);
        let pix = argb1555(c);
        rgba[i * 4..i * 4 + 4].copy_from_slice(&pix);
    }
    Image {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

fn argb1555(c: u16) -> [u8; 4] {
    if c == 0 {
        return [0, 0, 0, 0];
    }
    let r = ((c >> 10) & 0x1F) as u8;
    let g = ((c >> 5) & 0x1F) as u8;
    let b = (c & 0x1F) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 3) | (g >> 2),
        (b << 3) | (b >> 2),
        255,
    ]
}

impl UnicodeFont {
    fn glyph(&self, ch: u32) -> Option<Image> {
        let index = ch as usize;
        if index >= 0x10000 {
            return None;
        }
        let look = index * 4;
        let data = &self.data;
        if look + 4 > data.len() {
            return None;
        }
        let lookup =
            i32::from_le_bytes([data[look], data[look + 1], data[look + 2], data[look + 3]]);
        if lookup <= 0 {
            return None;
        }
        let p = lookup as usize;
        if p + 4 > data.len() {
            return None;
        }
        let w = data[p + 2] as i8 as i32;
        let h = data[p + 3] as i8 as i32;
        if w <= 0 || h <= 0 {
            return None;
        }
        let row_bytes = ((w as usize - 1) / 8) + 1;
        let bytes = row_bytes * h as usize;
        let src = data.get(p + 4..p + 4 + bytes)?;
        let mut rgba = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let byte = src[y * row_bytes + x / 8];
                let bit = 7 - (x % 8);
                if byte & (1 << bit) != 0 {
                    let o = (y * w as usize + x) * 4;
                    rgba[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        Some(Image {
            width: w as u32,
            height: h as u32,
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_empty_file_yields_no_fonts() {
        assert!(parse_ascii(&[]).is_empty());
    }

    #[test]
    fn ascii_one_space_glyph() {
        // header + 224 glyphs of 0×0 (3 bytes each) = 1 + 672.
        let mut buf = vec![0u8; 1 + 224 * 3];
        buf[0] = 0; // font header
                    // first glyph (space): 1×1 black pixel
        buf[1] = 1;
        buf[2] = 1;
        buf[3] = 0;
        // insert 2 pixel bytes after the first header — rebuild a tiny valid font
        // instead: one font, one 1×1 glyph, 223 empty 0×0 glyphs.
        let mut buf = vec![0u8]; // font header
        buf.extend_from_slice(&[1, 1, 0, 0, 0]); // 1×1, unk, pixel 0
        for _ in 1..224 {
            buf.extend_from_slice(&[0, 0, 0]);
        }
        let fonts = parse_ascii(&buf);
        assert_eq!(fonts.len(), 1);
        assert!(fonts[0].glyphs[0].is_some());
        assert_eq!(fonts[0].glyphs[0].as_ref().unwrap().width, 1);
    }
}

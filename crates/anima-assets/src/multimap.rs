//! `Multimap.rle` — the classic client's own facet bitmap (ClassicUO
//! `MultiMapLoader`). The play server's world map already rasters from
//! map+radarcol; this decoder exists so that file is not a silent gap, and so
//! `GET /multimap.png` can serve the original RLE when the file is present.

use std::path::Path;

use crate::art::Image;

/// Decoded `Multimap.rle` (width×height occupancy counts, ClassicUO's first
/// pass before it hues them).
pub struct MultiMap {
    pub width: i32,
    pub height: i32,
    /// Run-length occupancy: 0 = empty, 1+ = land density.
    pub density: Vec<u8>,
}

impl MultiMap {
    /// Open `Multimap.rle` if present. Missing file → `None`, matching
    /// ClassicUO's warn-and-skip.
    pub fn open(resource_dir: impl AsRef<Path>) -> Option<Self> {
        let data = std::fs::read(resource_dir.as_ref().join("Multimap.rle")).ok()?;
        Self::parse(&data)
    }

    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }
        let w = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if w < 1 || h < 1 || w > 8192 || h > 8192 {
            return None;
        }
        let mut density = vec![0u8; (w as usize) * (h as usize)];
        let mut x = 0i32;
        let mut y = 0i32;
        let mut p = 8usize;
        while p < data.len() && y < h {
            let pic = data[p];
            p += 1;
            let size = (pic & 0x7F) as i32;
            let colored = pic & 0x80 != 0;
            for _ in 0..size {
                if colored && x >= 0 && x < w && y >= 0 && y < h {
                    let i = (y * w + x) as usize;
                    if density[i] < 0xFF {
                        density[i] = density[i].saturating_add(1);
                    }
                }
                x += 1;
                if x >= w {
                    x = 0;
                    y += 1;
                    if y >= h {
                        break;
                    }
                }
            }
        }
        Some(Self {
            width: w,
            height: h,
            density,
        })
    }

    /// Greyscale PNG-ready image: empty cells transparent, occupied cells
    /// white at density-scaled alpha. Enough to prove the file decoded; the
    /// live world map still uses radarcol.
    pub fn to_image(&self) -> Image {
        let w = self.width.max(1) as u32;
        let h = self.height.max(1) as u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for (i, &d) in self.density.iter().enumerate() {
            if d == 0 {
                continue;
            }
            let a = (d as u16 * 16).min(255) as u8;
            let o = i * 4;
            rgba[o] = 220;
            rgba[o + 1] = 200;
            rgba[o + 2] = 160;
            rgba[o + 3] = a;
        }
        Image {
            width: w,
            height: h,
            rgba,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_is_none() {
        assert!(MultiMap::parse(&[]).is_none());
    }

    #[test]
    fn parses_a_one_pixel_colored_run() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&1i32.to_le_bytes());
        buf.extend_from_slice(&1i32.to_le_bytes());
        buf.push(0x81); // colored run of 1
        let m = MultiMap::parse(&buf).expect("parse");
        assert_eq!(m.width, 1);
        assert_eq!(m.height, 1);
        assert_eq!(m.density[0], 1);
        let img = m.to_image();
        assert_eq!(img.width, 1);
        assert!(img.rgba[3] > 0);
    }
}

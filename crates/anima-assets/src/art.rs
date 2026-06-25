//! UO tile art reader: land tiles (44×44 diamond) and static tiles (RLE),
//! decoding ARGB1555 → RGBA8. From `artLegacyMUL.uop`.
//!
//! Land art index = land graphic (0..0x3FFF); static art index = 0x4000 + graphic.

use std::collections::HashMap;
use std::path::Path;

use crate::uop::UopReader;

const LAND_DIM: usize = 44;

/// A decoded RGBA8 image.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// Encode as a PNG (for serving to the web renderer).
    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width.max(1), self.height.max(1));
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().expect("png header");
            w.write_image_data(&self.rgba).expect("png data");
        }
        out
    }

    /// Any non-transparent pixel?
    pub fn is_empty(&self) -> bool {
        self.rgba.chunks_exact(4).all(|p| p[3] == 0)
    }
}

/// ARGB1555 (UO 16-bit) → RGBA8. Color 0 is transparent.
fn argb1555(c: u16) -> [u8; 4] {
    if c == 0 {
        return [0, 0, 0, 0];
    }
    let r = ((c >> 10) & 0x1F) as u8;
    let g = ((c >> 5) & 0x1F) as u8;
    let b = (c & 0x1F) as u8;
    // 5→8 bit expansion (replicate high bits into low).
    [(r << 3) | (r >> 2), (g << 3) | (g >> 2), (b << 3) | (b >> 2), 255]
}

fn rd_u16(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}

pub struct Art {
    uop: UopReader,
    avg_cache: HashMap<u16, [u8; 4]>,
}

impl Art {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Art> {
        let path = resource_dir.as_ref().join("artLegacyMUL.uop");
        Ok(Art {
            uop: UopReader::open(&path)?,
            avg_cache: HashMap::new(),
        })
    }


    /// Decode a land tile (graphic 0..0x3FFF) to a 44×44 RGBA image.
    pub fn land(&self, graphic: u16) -> Option<Image> {
        let data = self.uop.by_art((graphic & 0x3FFF) as usize)?;
        let mut rgba = vec![0u8; LAND_DIM * LAND_DIM * 4];
        let mut p = 0usize;
        let put = |x: usize, y: usize, c: u16, rgba: &mut [u8]| {
            let px = argb1555(c);
            let o = (y * LAND_DIM + x) * 4;
            rgba[o..o + 4].copy_from_slice(&px);
        };
        // Top half: rows widen 2,4,…,44.
        for i in 0..22 {
            let count = (i + 1) * 2;
            let start = 22 - (i + 1);
            for j in 0..count {
                if p + 2 > data.len() {
                    return Some(Image { width: 44, height: 44, rgba });
                }
                put(start + j, i, rd_u16(&data, p), &mut rgba);
                p += 2;
            }
        }
        // Bottom half: rows narrow 44,…,2.
        for i in 0..22 {
            let count = (22 - i) * 2;
            let start = i;
            for j in 0..count {
                if p + 2 > data.len() {
                    break;
                }
                put(start + j, 22 + i, rd_u16(&data, p), &mut rgba);
                p += 2;
            }
        }
        Some(Image { width: 44, height: 44, rgba })
    }

    /// Average opaque color of a land tile, `[r, g, b, 255]` (cached).
    /// Falls back to mid-gray when the tile can't be decoded.
    pub fn land_avg_color(&mut self, graphic: u16) -> [u8; 4] {
        if let Some(c) = self.avg_cache.get(&graphic) {
            return *c;
        }
        let color = self
            .land(graphic)
            .map(|img| average_opaque(&img.rgba))
            .unwrap_or([90, 90, 90, 255]);
        self.avg_cache.insert(graphic, color);
        color
    }

    /// Decode a static tile (graphic, art index 0x4000+graphic) to RGBA.
    pub fn static_tile(&self, graphic: u16) -> Option<Image> {
        let data = self.uop.by_art(0x4000 + graphic as usize)?;
        if data.len() < 8 {
            return None;
        }
        // Header: u32 flags/unknown, then width u16, height u16.
        let width = rd_u16(&data, 4) as usize;
        let height = rd_u16(&data, 6) as usize;
        if width == 0 || height == 0 || width > 1024 || height > 1024 {
            return None;
        }
        let lookup_start = 8;
        let data_start = lookup_start + height * 2;
        if data_start > data.len() {
            return None;
        }
        let mut rgba = vec![0u8; width * height * 4];
        for y in 0..height {
            let off = rd_u16(&data, lookup_start + y * 2) as usize;
            let mut pos = data_start + off * 2;
            let mut x = 0usize;
            loop {
                if pos + 4 > data.len() {
                    break;
                }
                let x_offset = rd_u16(&data, pos) as usize;
                let x_run = rd_u16(&data, pos + 2) as usize;
                pos += 4;
                if x_offset == 0 && x_run == 0 {
                    break; // end of scanline
                }
                x += x_offset;
                for _ in 0..x_run {
                    if pos + 2 > data.len() || x >= width {
                        break;
                    }
                    let c = rd_u16(&data, pos);
                    pos += 2;
                    if y < height && x < width {
                        let o = (y * width + x) * 4;
                        rgba[o..o + 4].copy_from_slice(&argb1555(c | 0x8000));
                    }
                    x += 1;
                }
            }
        }
        Some(Image {
            width: width as u32,
            height: height as u32,
            rgba,
        })
    }
}

fn average_opaque(rgba: &[u8]) -> [u8; 4] {
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for px in rgba.chunks_exact(4) {
        if px[3] != 0 {
            r += px[0] as u64;
            g += px[1] as u64;
            b += px[2] as u64;
            n += 1;
        }
    }
    if n == 0 {
        return [90, 90, 90, 255];
    }
    [(r / n) as u8, (g / n) as u8, (b / n) as u8, 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn land_avg_is_greenish_for_grass() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut art = Art::open(&dir).expect("open art");
        for g in [0x0003u16, 0x0006, 0x00A8] {
            println!("land 0x{:04X}: avg={:?}", g, art.land_avg_color(g));
        }
        // Grass (0x0003/0x0006): green channel should dominate red and blue.
        let c = art.land_avg_color(0x0006);
        assert_ne!(c, [90, 90, 90, 255], "land 0x0006 should decode, not fall back");
        assert!(c[1] >= c[0] && c[1] >= c[2], "expected greenish, got {c:?}");

        // Land tile encodes to a valid PNG (signature check).
        let png = art.land(0x0003).unwrap().to_png();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

        // Static decode works on real data: scan a range, expect several
        // non-empty sprites with sane dimensions.
        let mut decoded = 0;
        for g in 0u16..400 {
            if let Some(img) = art.static_tile(g) {
                if !img.is_empty() && img.width <= 256 && img.height <= 256 {
                    decoded += 1;
                }
            }
        }
        println!("static tiles decoded non-empty in 0..400: {decoded}");
        assert!(decoded > 20, "expected many decodable statics, got {decoded}");
    }
}

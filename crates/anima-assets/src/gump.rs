//! Gump art reader (`gumpartLegacyMUL.uop`). Gumps are the UI bitmaps: the
//! paperdoll doll body, each worn item's paperdoll piece, container backgrounds,
//! buttons, etc. Each gump is a width×height image stored as a per-row lookup
//! table (one u32 offset per row, in u32 units) followed by `(color16, run)`
//! RLE pairs. Ported from ClassicUO `GumpsLoader`.

use std::collections::HashMap;
use std::path::Path;

use crate::art::Image;
use crate::def::parse_alias_def;
use crate::idxmul::IdxMul;
use crate::uop::UopReader;

enum GumpSource {
    Uop(UopReader),
    Mul(IdxMul),
}

pub struct Gumps {
    source: GumpSource,
    /// `gump.def`: missing index → first present group member + hue.
    aliases: HashMap<u32, (Vec<u32>, u16)>,
}

impl Gumps {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Gumps> {
        let dir = resource_dir.as_ref();
        let uop = dir.join("gumpartLegacyMUL.uop");
        let source = if uop.is_file() {
            GumpSource::Uop(UopReader::open(&uop)?)
        } else {
            let mul = ["gumpart.mul", "Gumpart.mul"]
                .iter()
                .map(|n| dir.join(n))
                .find(|p| p.is_file())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "gumpart.mul"))?;
            let idx = ["gumpidx.mul", "Gumpidx.mul"]
                .iter()
                .map(|n| dir.join(n))
                .find(|p| p.is_file())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "gumpidx.mul"))?;
            GumpSource::Mul(IdxMul::open(idx, mul)?)
        };
        Ok(Gumps {
            source,
            aliases: std::fs::read_to_string(dir.join("gump.def"))
                .map(|t| parse_alias_def(&t))
                .unwrap_or_default(),
        })
    }

    fn raw(&self, index: usize) -> Option<(Vec<u8>, u32, u32)> {
        match &self.source {
            GumpSource::Uop(u) => u.by_gump(index),
            GumpSource::Mul(m) => {
                let (data, extra) = m.read(index)?;
                let w = extra & 0xFFFF;
                let h = extra >> 16;
                Some((data, w, h))
            }
        }
    }

    fn resolve(&self, index: usize) -> Option<(Vec<u8>, u32, u32, u16)> {
        if let Some((data, w, h)) = self.raw(index) {
            return Some((data, w, h, 0));
        }
        let (group, hue) = self.aliases.get(&(index as u32))?;
        for &alt in group {
            if let Some((data, w, h)) = self.raw(alt as usize) {
                return Some((data, w, h, *hue));
            }
        }
        None
    }

    fn has(&self, index: usize) -> bool {
        match &self.source {
            GumpSource::Uop(u) => u.has_gump(index),
            GumpSource::Mul(m) => m.entry(index).is_some(),
        }
    }

    /// Hue `gump.def` wants applied when this index was missing and aliased.
    pub fn def_hue(&self, index: usize) -> u16 {
        if self.has(index) {
            return 0;
        }
        self.aliases
            .get(&(index as u32))
            .map(|(_, h)| *h)
            .unwrap_or(0)
    }

    /// Decode gump `index` to RGBA, or `None` if absent/empty.
    pub fn get(&self, index: usize) -> Option<Image> {
        let (data, w, h, _) = self.resolve(index)?;
        let (w, h) = (w as usize, h as usize);
        if data.len() < h * 4 {
            return None;
        }
        let u16le = |o: usize| u16::from_le_bytes([data[o], data[o + 1]]);
        let u32le = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);

        // The payload starts with the row-lookup table: `h` u32 offsets, each in
        // u32 units from the payload start. The pixel runs for row y span from
        // lookup[y] to lookup[y+1] (last row → end of data).
        let total_u32 = data.len() >> 2;
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            let row_off = u32le(y * 4) as usize;
            let next = if y + 1 < h {
                u32le((y + 1) * 4) as usize
            } else {
                total_u32
            };
            let pairs = next.saturating_sub(row_off);
            let mut p = row_off * 4;
            let mut x = 0usize;
            for _ in 0..pairs {
                if p + 4 > data.len() {
                    break;
                }
                let value = u16le(p);
                let run = u16le(p + 2) as usize;
                p += 4;
                if value != 0 {
                    let r = ((value >> 10) & 0x1F) as u8;
                    let g = ((value >> 5) & 0x1F) as u8;
                    let b = (value & 0x1F) as u8;
                    let (r, g, b) = (
                        (r << 3) | (r >> 2),
                        (g << 3) | (g >> 2),
                        (b << 3) | (b >> 2),
                    );
                    for k in 0..run {
                        let px = x + k;
                        if px < w {
                            let o = (y * w + px) * 4;
                            rgba[o] = r;
                            rgba[o + 1] = g;
                            rgba[o + 2] = b;
                            rgba[o + 3] = 255;
                        }
                    }
                }
                x += run;
            }
        }
        Some(Image {
            width: w as u32,
            height: h as u32,
            rgba,
        })
    }
}

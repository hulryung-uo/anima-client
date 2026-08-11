//! Light-shape reader (`lightidx.mul` + `light.mul`) — the per-light bitmaps UO
//! uses for the glow a torch, lamp, campfire or spell casts at night. From
//! ClassicUO `LightsLoader`.
//!
//! A light is **not** a circle: `light.mul` holds ~100 hand-drawn greyscale
//! masks of different sizes and shapes, and each light-emitting graphic points
//! at one of them through its tiledata `Quality`/layer byte (ClassicUO
//! `AddLight`: `light.ID = data.Layer`). The index is the usual 12-byte
//! `(offset, length, extra)` triple, with the shape's width in the high half of
//! `extra` and its height in the low half.
//!
//! Each pixel is one byte of intensity. ClassicUO:
//!
//! ```text
//! if (val > 0x1F) val = ~val & 0x1F;   // negative lights are bit-inverted
//! rgb24 = (val << 19) | (val << 11) | (val << 3);
//! ```
//!
//! — i.e. `val * 8` in all three channels, a grey, and fully transparent where
//! `val == 0`. It draws those greys **additively** into a light buffer. This
//! renderer instead *subtracts* from a darkness overlay, so [`Lights::light`]
//! hands back white pixels with the intensity in the **alpha** channel: same
//! shape, same falloff, expressed for a compositor that erases rather than adds.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use crate::art::Image;

/// ClassicUO `LightsLoader.MAX_LIGHTS_DATA_INDEX_COUNT` — ids beyond this are
/// refused before the file is even consulted.
pub const MAX_LIGHTS: u32 = 100;

pub struct Lights {
    idx: Vec<u8>,
    mul: Mutex<File>,
}

impl Lights {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Lights> {
        let dir = resource_dir.as_ref();
        Ok(Lights {
            idx: std::fs::read(dir.join("lightidx.mul"))?,
            mul: Mutex::new(File::open(dir.join("light.mul"))?),
        })
    }

    /// Decode light shape `id` to a white RGBA image whose alpha is the light's
    /// intensity. `None` for an out-of-range id, an empty index entry, or a
    /// zero-sized shape (`light.mul` has plenty of both).
    pub fn light(&self, id: u32) -> Option<Image> {
        if id >= MAX_LIGHTS {
            return None;
        }
        let o = id as usize * 12;
        if o + 12 > self.idx.len() {
            return None;
        }
        let u32_at = |i: usize| {
            u32::from_le_bytes([
                self.idx[i],
                self.idx[i + 1],
                self.idx[i + 2],
                self.idx[i + 3],
            ])
        };
        let pos = u32_at(o);
        let len = u32_at(o + 4);
        let extra = u32_at(o + 8);
        if pos == 0xFFFF_FFFF || len == 0 {
            return None;
        }
        // `extra` packs the shape's dimensions: width in the high 16 bits,
        // height in the low 16 (ClassicUO `UOFileMul.FillEntries`).
        let w = (extra >> 16) as usize;
        let h = (extra & 0xFFFF) as usize;
        if w == 0 || h == 0 || w * h > len as usize {
            return None;
        }

        let buf = {
            let mut f = self.mul.lock().ok()?;
            f.seek(SeekFrom::Start(pos as u64)).ok()?;
            let mut buf = vec![0u8; w * h];
            f.read_exact(&mut buf).ok()?;
            buf
        };

        let mut rgba = vec![0u8; w * h * 4];
        for (i, &raw) in buf.iter().enumerate() {
            // A light runs -31..31; the negative half arrives bit-inverted.
            let val = if raw > 0x1F { !raw & 0x1F } else { raw };
            if val == 0 {
                continue; // leave it fully transparent
            }
            let a = val << 3; // 0..31 -> 0..248, ClassicUO's `val << 3` per channel
            rgba[i * 4] = 0xFF;
            rgba[i * 4 + 1] = 0xFF;
            rgba[i * 4 + 2] = 0xFF;
            rgba[i * 4 + 3] = a;
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
    fn ids_past_the_table_are_refused_without_touching_the_file() {
        // The id bound comes before any file access, which is what keeps a
        // bogus tiledata layer byte from seeking into light.mul. Proved by
        // asking a Lights whose files do not exist: it never gets that far.
        let lights = Lights {
            idx: Vec::new(),
            mul: Mutex::new(File::open("/dev/null").unwrap()),
        };
        assert!(lights.light(MAX_LIGHTS).is_none());
        assert!(lights.light(u32::MAX).is_none());
        // And an in-range id with no index behind it is simply absent.
        assert!(lights.light(0).is_none());
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource/light.mul
    fn real_light_mul_decodes_shapes_with_graded_alpha() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let lights = Lights::open(dir).expect("open light.mul");
        let mut found = 0;
        for id in 0..MAX_LIGHTS {
            let Some(img) = lights.light(id) else {
                continue;
            };
            found += 1;
            assert!(img.width > 0 && img.height > 0);
            assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
            // Whatever is lit is white; the shape lives entirely in alpha.
            for px in img.rgba.chunks_exact(4) {
                if px[3] != 0 {
                    assert_eq!([px[0], px[1], px[2]], [0xFF, 0xFF, 0xFF]);
                }
            }
        }
        assert!(found > 20, "expected a table of light shapes, got {found}");
        // A real light is a gradient, not a disc: some pixel must be partly lit.
        let img = lights.light(1).expect("light 1");
        assert!(img.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 248));
    }
}

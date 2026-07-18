//! Texmap reader (`texidx.mul` + `texmaps.mul`) — seamless square terrain
//! textures used for **stretched/sloped land tiles** (indexed by a land tile's
//! `tex_id` from tiledata). 64×64 when the entry is 0x2000 bytes, else 128×128.
//! Pixels are opaque ARGB1555. From ClassicUO `TexmapsLoader`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use crate::art::Image;

pub struct Texmaps {
    idx: Vec<u8>,
    mul: Mutex<File>,
}

impl Texmaps {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Texmaps> {
        let dir = resource_dir.as_ref();
        Ok(Texmaps {
            idx: std::fs::read(dir.join("texidx.mul"))?,
            mul: Mutex::new(File::open(dir.join("texmaps.mul"))?),
        })
    }

    /// Decode texmap `id` to an opaque RGBA square image.
    pub fn texmap(&self, id: u16) -> Option<Image> {
        let o = id as usize * 12;
        if o + 8 > self.idx.len() {
            return None;
        }
        let pos = u32::from_le_bytes([
            self.idx[o],
            self.idx[o + 1],
            self.idx[o + 2],
            self.idx[o + 3],
        ]);
        let len = u32::from_le_bytes([
            self.idx[o + 4],
            self.idx[o + 5],
            self.idx[o + 6],
            self.idx[o + 7],
        ]);
        if pos == 0xFFFF_FFFF || len == 0 {
            return None;
        }
        let size: usize = if len == 0x2000 { 64 } else { 128 };

        let buf = {
            let mut f = self.mul.lock().ok()?;
            f.seek(SeekFrom::Start(pos as u64)).ok()?;
            let mut buf = vec![0u8; (size * size * 2).min(len as usize)];
            f.read_exact(&mut buf).ok()?;
            buf
        };

        let mut rgba = vec![0u8; size * size * 4];
        for i in 0..(size * size) {
            if i * 2 + 1 >= buf.len() {
                break;
            }
            let c = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]);
            let r = ((c >> 10) & 0x1F) as u8;
            let g = ((c >> 5) & 0x1F) as u8;
            let b = (c & 0x1F) as u8;
            let o = i * 4;
            rgba[o] = (r << 3) | (r >> 2);
            rgba[o + 1] = (g << 3) | (g >> 2);
            rgba[o + 2] = (b << 3) | (b >> 2);
            rgba[o + 3] = 255; // terrain texmaps are opaque
        }
        Some(Image {
            width: size as u32,
            height: size as u32,
            rgba,
        })
    }
}

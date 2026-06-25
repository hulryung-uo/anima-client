//! Legacy mobile animation reader (`anim.idx` + `anim.mul`).
//!
//! UO bodies animate via groups (Walk=0, Stand=4 for people; Walk=0, Stand=2 for
//! monsters) × 5 stored directions (the other 3 are mirrored). Each (group,dir)
//! is one idx entry → a palette + frames; each frame is RLE over a 256-color
//! palette. Ported from ClassicUO `AnimationsLoader` (legacy MUL path).
//!
//! Body coverage: people bodies (human 400/401, elf, gargoyle) use the high
//! formula; everything else uses the monster formula. `body.def` remapping is
//! not applied yet, so exotic bodies may not resolve (caller falls back).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use crate::art::Image;

/// People animation groups (35 groups × 5 dirs = 175 entries/body).
pub const PEOPLE_WALK: u8 = 0;
pub const PEOPLE_STAND: u8 = 4;
/// Monster groups (22 groups × 5 dirs = 110 entries/body).
pub const MONSTER_STAND: u8 = 2;

pub struct Anim {
    idx: Vec<u8>,
    mul: Mutex<File>,
}

impl Anim {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Anim> {
        let dir = resource_dir.as_ref();
        Ok(Anim {
            idx: std::fs::read(dir.join("anim.idx"))?,
            mul: Mutex::new(File::open(dir.join("anim.mul"))?),
        })
    }

    /// Bodies that use the high/people group layout (175 entries).
    fn is_people(body: u16) -> bool {
        matches!(body, 400 | 401 | 402 | 403 | 605 | 606 | 666 | 667 | 694 | 695)
    }

    /// idx block for (body, group, animDir 0..4).
    fn block(body: u16, group: u8, dir: u8) -> usize {
        let base = if Self::is_people(body) {
            ((body as i64 - 400) * 175 + 35000) as usize
        } else {
            body as usize * 110
        };
        base + group as usize * 5 + dir as usize
    }

    fn entry(&self, block: usize) -> Option<(u32, u32)> {
        let o = block * 12;
        if o + 8 > self.idx.len() {
            return None;
        }
        let pos = u32le(&self.idx, o);
        let size = u32le(&self.idx, o + 4);
        if pos == 0xFFFF_FFFF || size == 0 || size == 0xFFFF_FFFF {
            return None;
        }
        Some((pos, size))
    }

    /// The default standing group for a body.
    pub fn stand_group(body: u16) -> u8 {
        if Self::is_people(body) {
            PEOPLE_STAND
        } else {
            MONSTER_STAND
        }
    }

    /// Number of frames in (body, group, UO-direction 0..7), or `None` if absent.
    pub fn frame_count(&self, body: u16, group: u8, dir8: u8) -> Option<usize> {
        let (dir, _) = map_dir(dir8);
        let (pos, size) = self.entry(Self::block(body, group, dir))?;
        if size < 516 {
            return None;
        }
        let mut f = self.mul.lock().ok()?;
        f.seek(SeekFrom::Start(pos as u64 + 512)).ok()?; // skip palette
        let mut b = [0u8; 4];
        f.read_exact(&mut b).ok()?;
        Some(u32::from_le_bytes(b) as usize)
    }

    /// Decode one frame of (body, group, UO-direction 0..7, frame_idx).
    /// Returns the RGBA image already horizontally mirrored when the direction
    /// requires it (so the caller just draws it). `None` if the body/group is
    /// absent or the frame can't be decoded.
    pub fn frame(&self, body: u16, group: u8, dir8: u8, frame_idx: usize) -> Option<Image> {
        let (dir, mirror) = map_dir(dir8);
        let (pos, size) = self.entry(Self::block(body, group, dir))?;

        let buf = {
            let mut f = self.mul.lock().ok()?;
            f.seek(SeekFrom::Start(pos as u64)).ok()?;
            let mut buf = vec![0u8; size as usize];
            f.read_exact(&mut buf).ok()?;
            buf
        };
        if buf.len() < 516 {
            return None;
        }

        // palette: 256 × u16 ARGB1555
        let pal = |i: u8| u16le(&buf, i as usize * 2);
        let data_start = 512usize;
        let frame_count = u32le(&buf, 512) as usize;
        if frame_idx >= frame_count {
            return None;
        }
        let foff = u32le(&buf, 516 + frame_idx * 4) as usize;
        let mut p = data_start + foff;
        if p + 8 > buf.len() {
            return None;
        }

        let center_x = i16le(&buf, p);
        let center_y = i16le(&buf, p + 2);
        let width = i16le(&buf, p + 4) as i32;
        let height = i16le(&buf, p + 6) as i32;
        p += 8;
        if width <= 0 || height <= 0 || width > 512 || height > 512 {
            return None;
        }
        let (w, h) = (width as usize, height as usize);
        let mut rgba = vec![0u8; w * h * 4];

        loop {
            if p + 4 > buf.len() {
                break;
            }
            let header = u32le(&buf, p);
            p += 4;
            if header == 0x7FFF_7FFF {
                break;
            }
            let run = (header & 0x0FFF) as usize;
            let mut x = ((header >> 22) & 0x3FF) as i32;
            if x & 0x200 != 0 {
                x |= !0x3FF; // sign-extend 10-bit
            }
            let mut y = ((header >> 12) & 0x3FF) as i32;
            if y & 0x200 != 0 {
                y |= !0x3FF;
            }
            x += center_x as i32;
            y += center_y as i32 + height;

            for k in 0..run {
                if p >= buf.len() {
                    break;
                }
                let c = pal(buf[p]);
                p += 1;
                let px = x + k as i32;
                if px >= 0 && px < width && y >= 0 && y < height {
                    let o = (y as usize * w + px as usize) * 4;
                    let r = ((c >> 10) & 0x1F) as u8;
                    let g = ((c >> 5) & 0x1F) as u8;
                    let b = (c & 0x1F) as u8;
                    rgba[o] = (r << 3) | (r >> 2);
                    rgba[o + 1] = (g << 3) | (g >> 2);
                    rgba[o + 2] = (b << 3) | (b >> 2);
                    rgba[o + 3] = 255;
                }
            }
        }

        let img = Image {
            width: w as u32,
            height: h as u32,
            rgba,
        };
        Some(if mirror { flip_h(&img) } else { img })
    }
}

/// 8-direction (UO) → (stored anim dir 0..4, mirror). From ClassicUO GetAnimDirection.
fn map_dir(dir8: u8) -> (u8, bool) {
    match dir8 & 7 {
        2 => (1, true),
        4 => (1, false),
        1 => (2, true),
        5 => (2, false),
        0 => (3, true),
        6 => (3, false),
        3 => (0, false),
        7 => (4, false),
        _ => (0, false),
    }
}

fn flip_h(img: &Image) -> Image {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let s = (y * w + x) * 4;
            let d = (y * w + (w - 1 - x)) * 4;
            rgba[d..d + 4].copy_from_slice(&img.rgba[s..s + 4]);
        }
    }
    Image {
        width: img.width,
        height: img.height,
        rgba,
    }
}

fn u32le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn u16le(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn i16le(d: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([d[o], d[o + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn decodes_human_stand_frame() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        // Human male (0x190 = 400), standing, facing south (dir 4), frame 0.
        let img = anim.frame(400, PEOPLE_STAND, 4, 0).expect("human stand frame");
        println!("human stand frame: {}x{}", img.width, img.height);
        assert!(img.width > 0 && img.height > 0);
        assert!(!img.is_empty(), "frame should have opaque pixels");
    }
}

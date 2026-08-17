//! Generic `*.idx` + `*.mul` pair. Used when a UOP container is absent
//! (ClassicUO's MUL fallback for art and gumps).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

pub struct IdxMul {
    idx: Vec<u8>,
    mul: Mutex<File>,
}

impl IdxMul {
    pub fn open(idx_path: impl AsRef<Path>, mul_path: impl AsRef<Path>) -> std::io::Result<IdxMul> {
        Ok(IdxMul {
            idx: std::fs::read(idx_path)?,
            mul: Mutex::new(File::open(mul_path)?),
        })
    }

    /// `(offset, length, extra)` for `index`, or `None` if the slot is empty.
    pub fn entry(&self, index: usize) -> Option<(u32, u32, u32)> {
        let o = index.checked_mul(12)?;
        if o + 12 > self.idx.len() {
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
        let extra = u32::from_le_bytes([
            self.idx[o + 8],
            self.idx[o + 9],
            self.idx[o + 10],
            self.idx[o + 11],
        ]);
        if pos == 0xFFFF_FFFF || len == 0 || len == 0xFFFF_FFFF {
            return None;
        }
        Some((pos, len, extra))
    }

    pub fn read(&self, index: usize) -> Option<(Vec<u8>, u32)> {
        let (pos, len, extra) = self.entry(index)?;
        let mut f = self.mul.lock().ok()?;
        f.seek(SeekFrom::Start(pos as u64)).ok()?;
        let mut buf = vec![0u8; len as usize];
        f.read_exact(&mut buf).ok()?;
        Some((buf, extra))
    }
}

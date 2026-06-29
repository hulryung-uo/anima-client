//! UOP (Ultima Online Patch) container reader.
//!
//! UOP files store entries keyed by a Jenkins `HashLittle2` of their virtual
//! path. Ported from `anima/anima/uop.py`.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;

/// Jenkins HashLittle2 of a UOP entry path, returned as a 64-bit value.
pub fn uop_hash(s: &str) -> u64 {
    let b = s.as_bytes();
    let length = b.len();
    let seed = (length as u32).wrapping_add(0xDEAD_BEEF);
    let (mut ebx, mut edi, mut esi) = (seed, seed, seed);
    let w = |x: &[u8], i: usize| -> u32 {
        (x[i] as u32) | ((x[i + 1] as u32) << 8) | ((x[i + 2] as u32) << 16) | ((x[i + 3] as u32) << 24)
    };

    let mut i = 0usize;
    while i + 12 < length {
        edi = edi.wrapping_add(w(b, i + 4));
        esi = esi.wrapping_add(w(b, i + 8));
        let mut edx = w(b, i).wrapping_sub(esi);
        edx = edx.wrapping_add(ebx) ^ esi.rotate_left(4);
        esi = esi.wrapping_add(edi);
        edi = edi.wrapping_sub(edx) ^ edx.rotate_left(6);
        edx = edx.wrapping_add(esi);
        esi = esi.wrapping_sub(edi) ^ edi.rotate_left(8);
        edi = edi.wrapping_add(edx);
        ebx = edx.wrapping_sub(esi) ^ esi.rotate_left(16);
        esi = esi.wrapping_add(edi);
        edi = edi.wrapping_sub(ebx) ^ ebx.rotate_left(19);
        ebx = ebx.wrapping_add(esi);
        esi = esi.wrapping_sub(edi) ^ edi.rotate_left(4);
        edi = edi.wrapping_add(ebx);
        i += 12;
    }

    let remaining = length - i;
    if remaining > 0 {
        let byte = |k: usize| b[i + k] as u32;
        if remaining >= 12 { esi = esi.wrapping_add(byte(11) << 24); }
        if remaining >= 11 { esi = esi.wrapping_add(byte(10) << 16); }
        if remaining >= 10 { esi = esi.wrapping_add(byte(9) << 8); }
        if remaining >= 9 { esi = esi.wrapping_add(byte(8)); }
        if remaining >= 8 { edi = edi.wrapping_add(byte(7) << 24); }
        if remaining >= 7 { edi = edi.wrapping_add(byte(6) << 16); }
        if remaining >= 6 { edi = edi.wrapping_add(byte(5) << 8); }
        if remaining >= 5 { edi = edi.wrapping_add(byte(4)); }
        if remaining >= 4 { ebx = ebx.wrapping_add(byte(3) << 24); }
        if remaining >= 3 { ebx = ebx.wrapping_add(byte(2) << 16); }
        if remaining >= 2 { ebx = ebx.wrapping_add(byte(1) << 8); }
        if remaining >= 1 { ebx = ebx.wrapping_add(byte(0)); }

        esi = (esi ^ edi).wrapping_sub(edi.rotate_left(14));
        let ecx = (esi ^ ebx).wrapping_sub(esi.rotate_left(11));
        edi = (edi ^ ecx).wrapping_sub(ecx.rotate_left(25));
        esi = (esi ^ edi).wrapping_sub(edi.rotate_left(16));
        let edx = (esi ^ ecx).wrapping_sub(esi.rotate_left(4));
        edi = (edi ^ edx).wrapping_sub(edx.rotate_left(14));
        let eax = (esi ^ edi).wrapping_sub(edi.rotate_left(24));
        ((edi as u64) << 32) | eax as u64
    } else {
        (esi as u64) << 32
    }
}

struct Entry {
    offset: usize,
    compressed_size: usize,
    decompressed_size: usize,
    compression: i16,
}

/// A parsed UOP container.
pub struct UopReader {
    data: Vec<u8>,
    entries: HashMap<u64, Entry>,
}

impl UopReader {
    pub fn open(path: &Path) -> std::io::Result<UopReader> {
        let data = std::fs::read(path)?;
        let mut reader = UopReader {
            data,
            entries: HashMap::new(),
        };
        reader.parse()?;
        Ok(reader)
    }

    fn parse(&mut self) -> std::io::Result<()> {
        let data = &self.data;
        let le32 = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
        let le64 = |o: usize| -> i64 {
            i64::from_le_bytes([
                data[o], data[o + 1], data[o + 2], data[o + 3], data[o + 4], data[o + 5], data[o + 6],
                data[o + 7],
            ])
        };

        let magic = le32(0);
        if magic != 0x0050_594D {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("not a UOP file: bad magic 0x{magic:X}"),
            ));
        }

        let mut next_block = le64(12);
        while next_block != 0 && (next_block as usize) < data.len() {
            let mut pos = next_block as usize;
            let count = le32(pos) as i32;
            pos += 4;
            next_block = le64(pos);
            pos += 8;

            for _ in 0..count {
                if pos + 34 > data.len() {
                    break;
                }
                let offset = le64(pos) as usize;
                let header_len = le32(pos + 8) as usize;
                let compressed = le32(pos + 12) as usize;
                let decompressed = le32(pos + 16) as usize;
                let file_hash = u64::from_le_bytes([
                    data[pos + 20], data[pos + 21], data[pos + 22], data[pos + 23], data[pos + 24],
                    data[pos + 25], data[pos + 26], data[pos + 27],
                ]);
                let compression = i16::from_le_bytes([data[pos + 32], data[pos + 33]]);
                pos += 34;

                if offset == 0 || compressed == 0 {
                    continue;
                }
                self.entries.insert(
                    file_hash,
                    Entry {
                        offset: offset + header_len,
                        compressed_size: compressed,
                        decompressed_size: decompressed,
                        compression,
                    },
                );
            }
        }
        Ok(())
    }

    /// Number of parsed entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Decompressed bytes for an entry identified by its path hash.
    pub fn by_hash(&self, hash: u64) -> Option<Vec<u8>> {
        let e = self.entries.get(&hash)?;
        let raw = &self.data[e.offset..e.offset + e.compressed_size];
        if e.compression == 1 {
            let mut out = Vec::new();
            ZlibDecoder::new(raw).read_to_end(&mut out).ok()?;
            Some(out)
        } else {
            Some(raw.to_vec())
        }
    }

    /// Decompressed bytes for `pattern.format(index)`, e.g.
    /// `"build/map0legacymul/{:08}.dat"` with `index`.
    pub fn by_map_chunk(&self, index: usize) -> Option<Vec<u8>> {
        let path = format!("build/map0legacymul/{index:08}.dat");
        self.by_hash(uop_hash(&path))
    }

    /// Decompressed bytes for an art entry (`build/artlegacymul/{:08}.tga` — the
    /// art UOP uses a legacy `.tga` extension in its virtual paths, unlike the
    /// map's `.dat`). `index` = land graphic (0..0x3FFF) or `0x4000 + static`.
    pub fn by_art(&self, index: usize) -> Option<Vec<u8>> {
        let path = format!("build/artlegacymul/{index:08}.tga");
        self.by_hash(uop_hash(&path))
    }

    /// Decompressed bytes for a sound entry (`build/soundlegacymul/{:08}.dat`).
    /// `index` = the sound id used by the 0x54 PlaySoundEffect packet.
    pub fn by_sound(&self, index: usize) -> Option<Vec<u8>> {
        let path = format!("build/soundlegacymul/{index:08}.dat");
        self.by_hash(uop_hash(&path))
    }

    /// Gump entry (`build/gumpartlegacymul/{:08}.tga`). Gump UOP entries carry an
    /// 8-byte "extra" (width:i32, height:i32) ahead of the payload (ClassicUO opens
    /// this file with `hasExtra=true`). Returns `(decompressed payload, width, height)`
    /// where the payload is the row-lookup table + RLE pixel runs.
    pub fn by_gump(&self, index: usize) -> Option<(Vec<u8>, u32, u32)> {
        let path = format!("build/gumpartlegacymul/{index:08}.tga");
        let e = self.entries.get(&uop_hash(&path))?;
        let d = &self.data;
        if e.offset + 8 > d.len() {
            return None;
        }
        let le32 = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        let mut w = le32(e.offset);
        let mut h = le32(e.offset + 4);
        let end = (e.offset + 8 + e.compressed_size).min(d.len());
        let payload = &d[e.offset + 8..end];
        let mut out = if e.compression == 1 {
            let mut out = Vec::with_capacity(e.decompressed_size);
            ZlibDecoder::new(payload).read_to_end(&mut out).ok()?;
            out
        } else {
            payload.to_vec()
        };
        // Fallback: some entries store width/height as the first two u32 of the
        // (decompressed) payload instead of the extra field.
        if (w == 0 || h == 0 || w > 4096 || h > 4096) && out.len() >= 8 {
            w = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
            h = u32::from_le_bytes([out[4], out[5], out[6], out[7]]);
            out.drain(0..8);
        }
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return None;
        }
        Some((out, w, h))
    }
}

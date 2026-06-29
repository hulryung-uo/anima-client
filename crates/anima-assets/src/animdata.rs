//! `animdata.mul` reader — per-tile effect/animation framing.
//!
//! animdata.mul is a flat table of fixed-size records describing how an animated
//! ART tile (spell effects, flames, water, …) cycles through neighbouring tile
//! ids. A UO graphical effect (0x70/0xC0/0xC7) plays its `graphic` tile for
//! several frames; the frame count, interval, and per-frame tile-id offsets all
//! come from here. Ported from ClassicUO `AnimDataLoader`.
//!
//! The file is grouped into blocks of 8 tiles. Each block is a 4-byte header
//! followed by 8 records of 68 bytes each (block = 4 + 8*68 = 548 bytes). A
//! record is `[i8 frameData[64]][u8 unknown][u8 frameCount][u8 frameInterval]
//! [u8 frameStart]`. The record for tile `g` therefore starts at byte offset
//! `g*68 + 4*(g/8) + 4` — equivalently `(g/8)*548 + 4 + (g%8)*68` (the ClassicUO
//! `graphic*68 + 4*((graphic>>3)+1)` form). `frameData[i]` is a signed offset
//! from `graphic`, so frame `i` shows tile `graphic + frameData[i]`.

use std::path::Path;

/// One animdata record is 64 frame-offset bytes + 4 trailing fields.
const RECORD: usize = 68;

/// `animdata.mul` reader.
pub struct AnimData {
    data: Vec<u8>,
}

impl AnimData {
    /// Open `animdata.mul` under `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>) -> std::io::Result<AnimData> {
        let data = std::fs::read(data_dir.as_ref().join("animdata.mul"))?;
        Ok(AnimData { data })
    }

    /// Byte offset of the record for tile `graphic` (ClassicUO `AnimDataLoader`).
    fn record_off(graphic: u16) -> usize {
        let g = graphic as usize;
        g * RECORD + 4 * (g / 8) + 4
    }

    /// `(frameCount, frameInterval, frameStart)` for an animated tile. `(0, 0, 0)`
    /// when the tile is out of range or has no animdata record — treat count 0 as
    /// a single static frame (`graphic` itself).
    pub fn frames(&self, graphic: u16) -> (u8, u8, u8) {
        let off = Self::record_off(graphic);
        if off + RECORD > self.data.len() {
            return (0, 0, 0);
        }
        (self.data[off + 65], self.data[off + 66], self.data[off + 67])
    }

    /// The per-frame tile-id offsets (the `i8[64]` frameData). Frame `i` shows tile
    /// `graphic + frame_offsets(graphic)[i]`; only the first `frameCount` are used.
    pub fn frame_offsets(&self, graphic: u16) -> [i8; 64] {
        let mut out = [0i8; 64];
        let off = Self::record_off(graphic);
        if off + 64 <= self.data.len() {
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = self.data[off + i] as i8;
            }
        }
        out
    }

    /// Resolve the sequence of ART tile ids this graphic animates through, applying
    /// the per-frame offsets. A graphic with `frameCount == 0` (no record) is a
    /// single static frame, so this returns `[graphic]`.
    pub fn frame_sequence(&self, graphic: u16) -> Vec<u16> {
        let (count, _interval, _start) = self.frames(graphic);
        if count == 0 {
            return vec![graphic];
        }
        let offs = self.frame_offsets(graphic);
        let count = (count as usize).min(64);
        (0..count).map(|i| graphic.wrapping_add(offs[i] as u16)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_offsets_match_classicuo() {
        // graphic*68 + 4*((graphic>>3)+1): block headers fall every 8 tiles.
        assert_eq!(AnimData::record_off(0), 4);
        assert_eq!(AnimData::record_off(7), 7 * 68 + 4);
        assert_eq!(AnimData::record_off(8), 8 * 68 + 4 + 4);
    }

    #[test]
    fn missing_record_is_single_frame() {
        let ad = AnimData { data: Vec::new() };
        assert_eq!(ad.frames(0x36D4), (0, 0, 0));
        assert_eq!(ad.frame_sequence(0x36D4), vec![0x36D4]);
    }

    #[test]
    fn frame_sequence_applies_offsets() {
        // Build a one-block file: header(4) + record for tile 0 with frameData
        // [0,1,2,...], frameCount 3, interval 5. Tiles 1..7 left zeroed.
        let mut data = vec![0u8; 548];
        let off = AnimData::record_off(0); // 4
        for i in 0..64u8 {
            data[off + i as usize] = i; // frameData[i] = i
        }
        data[off + 65] = 3; // frameCount
        data[off + 66] = 5; // frameInterval
        let ad = AnimData { data };
        assert_eq!(ad.frames(0), (3, 5, 0));
        // graphic 0 + offsets 0,1,2 → tiles 0,1,2.
        assert_eq!(ad.frame_sequence(0), vec![0, 1, 2]);
    }

    /// Requires local UO data at ~/dev/uo/uo-resource. Ignored by default.
    #[test]
    #[ignore]
    fn reads_real_animdata() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        if !Path::new(&dir).join("animdata.mul").exists() {
            return;
        }
        let ad = AnimData::open(&dir).expect("open animdata");
        // Scan for any tile with a real animation record.
        let mut animated = 0;
        for g in 0u16..0x4000 {
            if ad.frames(g).0 > 1 {
                animated += 1;
            }
        }
        println!("found {animated} animated tiles");
        assert!(animated > 0, "expected some animated tiles in animdata.mul");
    }
}

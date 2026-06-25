//! `tiledata.mul` reader (High Seas format, 64-bit flags).
//!
//! Layout (HS): a land section of 512 groups, then an item/static section.
//! - Land group:  `[header u32][32 × (flags u64, texID u16, name[20])]`  (30 B/entry)
//! - Item group:  `[header u32][32 × (flags u64, weight u8, quality u8, unk u16,
//!   unk1 u8, quantity u8, animID u16, unk2 u8, hue u8, unk3 u16, height u8,
//!   name[20])]`  (41 B/entry)
//!
//! We only need each tile's flags (and item height) for walkability.

/// Tiledata flag bits we care about (low 32 of the 64-bit flags field).
pub mod flags {
    pub const IMPASSABLE: u64 = 0x0000_0040;
    pub const SURFACE: u64 = 0x0000_0200;
    pub const BRIDGE: u64 = 0x0000_0400;
    pub const WET: u64 = 0x0000_0008;
}

const LAND_ENTRY: usize = 30;
const LAND_GROUP: usize = 4 + 32 * LAND_ENTRY; // 964
const LAND_GROUPS: usize = 512;
const LAND_SECTION: usize = LAND_GROUPS * LAND_GROUP; // 493_568

const ITEM_ENTRY: usize = 41;
const ITEM_GROUP: usize = 4 + 32 * ITEM_ENTRY; // 1316

pub struct TileData {
    data: Vec<u8>,
}

impl TileData {
    pub fn open(path: &std::path::Path) -> std::io::Result<TileData> {
        Ok(TileData {
            data: std::fs::read(path)?,
        })
    }

    fn u64_at(&self, off: usize) -> u64 {
        let d = &self.data;
        u64::from_le_bytes([
            d[off], d[off + 1], d[off + 2], d[off + 3], d[off + 4], d[off + 5], d[off + 6], d[off + 7],
        ])
    }

    /// Flags for a land tile graphic (0..0x4000).
    pub fn land_flags(&self, graphic: u16) -> u64 {
        let g = (graphic & 0x3FFF) as usize;
        let group = g / 32;
        let within = g % 32;
        let off = group * LAND_GROUP + 4 + within * LAND_ENTRY;
        if off + 8 <= self.data.len() {
            self.u64_at(off)
        } else {
            0
        }
    }

    fn item_entry_off(&self, graphic: u16) -> Option<usize> {
        let g = graphic as usize;
        let group = g / 32;
        let within = g % 32;
        let off = LAND_SECTION + group * ITEM_GROUP + 4 + within * ITEM_ENTRY;
        if off + ITEM_ENTRY <= self.data.len() {
            Some(off)
        } else {
            None
        }
    }

    /// Flags for a static/item graphic.
    pub fn item_flags(&self, graphic: u16) -> u64 {
        self.item_entry_off(graphic)
            .map(|off| self.u64_at(off))
            .unwrap_or(0)
    }

    /// Height of a static/item graphic (used for Z stacking/walkability).
    pub fn item_height(&self, graphic: u16) -> u8 {
        // height is the byte at entry offset +20 (after flags+the fixed fields).
        self.item_entry_off(graphic)
            .map(|off| self.data[off + 20])
            .unwrap_or(0)
    }
}

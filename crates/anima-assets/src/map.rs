//! Map reader: land tiles (from the UOP map) + statics, with Z-aware
//! walkability. Ported from `anima/anima/map.py`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::tiledata::{flags, TileData};
use crate::uop::UopReader;

// Map0 (Felucca/Trammel) dimensions.
pub const MAP_WIDTH: u32 = 7168;
pub const MAP_HEIGHT: u32 = 4096;
const BLOCK_SIZE: u32 = 8;
const BLOCKS_PER_UOP_CHUNK: usize = 4096;
const MAP_BLOCK_BYTES: usize = 196; // 4 header + 64 × 3
const BLOCKS_Y: u32 = MAP_HEIGHT / BLOCK_SIZE; // 512

/// Character body height and max single-step climb (ClassicUO defaults).
const CHAR_HEIGHT: i32 = 16;
const MAX_STEP: i32 = 16;

#[derive(Clone, Copy)]
pub struct LandTile {
    pub graphic: u16,
    pub z: i8,
    pub flags: u64,
}

impl LandTile {
    pub fn impassable(&self) -> bool {
        self.flags & flags::IMPASSABLE != 0
    }
}

#[derive(Clone, Copy)]
pub struct StaticTile {
    pub graphic: u16,
    pub z: i8,
    pub height: u8,
    pub flags: u64,
}

impl StaticTile {
    pub fn impassable(&self) -> bool {
        self.flags & flags::IMPASSABLE != 0
    }
    pub fn surface(&self) -> bool {
        self.flags & (flags::SURFACE | flags::BRIDGE) != 0
    }
}

/// Reads UO map data on demand with per-block caching.
pub struct MapData {
    uop: UopReader,
    staidx: Vec<u8>,
    statics: Vec<u8>,
    tiledata: TileData,
    land_cache: HashMap<u32, Vec<(u16, i8)>>,
    statics_cache: HashMap<u32, Vec<(u8, u8, StaticTile)>>, // (cx, cy, tile)
}

impl MapData {
    /// Open the map from a UO data directory (containing `map0LegacyMUL.uop`,
    /// `staidx0.mul`, `statics0.mul`, `tiledata.mul`).
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<MapData> {
        let dir: PathBuf = resource_dir.as_ref().to_path_buf();
        Ok(MapData {
            uop: UopReader::open(&dir.join("map0LegacyMUL.uop"))?,
            staidx: std::fs::read(dir.join("staidx0.mul"))?,
            statics: std::fs::read(dir.join("statics0.mul"))?,
            tiledata: TileData::open(&dir.join("tiledata.mul"))?,
            land_cache: HashMap::new(),
            statics_cache: HashMap::new(),
        })
    }

    fn load_land_block(&mut self, bx: u32, by: u32) -> Vec<(u16, i8)> {
        let key = (bx << 16) | by;
        if let Some(c) = self.land_cache.get(&key) {
            return c.clone();
        }
        let block_num = (bx * BLOCKS_Y + by) as usize;
        let chunk_idx = block_num / BLOCKS_PER_UOP_CHUNK;
        let block_in_chunk = block_num % BLOCKS_PER_UOP_CHUNK;

        let mut cells = vec![(0u16, 0i8); 64];
        if let Some(chunk) = self.uop.by_map_chunk(chunk_idx) {
            let base = block_in_chunk * MAP_BLOCK_BYTES + 4; // skip 4-byte header
            for (i, cell) in cells.iter_mut().enumerate() {
                let pos = base + i * 3;
                if pos + 3 <= chunk.len() {
                    let tile = u16::from_le_bytes([chunk[pos], chunk[pos + 1]]) & 0x3FFF;
                    let z = chunk[pos + 2] as i8;
                    *cell = (tile, z);
                }
            }
        }
        self.land_cache.insert(key, cells.clone());
        cells
    }

    fn load_statics_block(&mut self, bx: u32, by: u32) -> Vec<(u8, u8, StaticTile)> {
        let key = (bx << 16) | by;
        if let Some(c) = self.statics_cache.get(&key) {
            return c.clone();
        }
        let block_num = (bx * BLOCKS_Y + by) as usize;
        let idx_off = block_num * 12;
        let mut out = Vec::new();
        if idx_off + 12 <= self.staidx.len() {
            let data_off = u32::from_le_bytes([
                self.staidx[idx_off], self.staidx[idx_off + 1], self.staidx[idx_off + 2],
                self.staidx[idx_off + 3],
            ]) as usize;
            let data_len = u32::from_le_bytes([
                self.staidx[idx_off + 4], self.staidx[idx_off + 5], self.staidx[idx_off + 6],
                self.staidx[idx_off + 7],
            ]) as usize;
            if data_off != 0xFFFF_FFFF && data_len != 0 {
                let mut pos = data_off;
                let end = (data_off + data_len).min(self.statics.len());
                while pos + 7 <= end {
                    let graphic = u16::from_le_bytes([self.statics[pos], self.statics[pos + 1]]);
                    let cx = self.statics[pos + 2];
                    let cy = self.statics[pos + 3];
                    let z = self.statics[pos + 4] as i8;
                    pos += 7;
                    let tile = StaticTile {
                        graphic,
                        z,
                        height: self.tiledata.item_height(graphic),
                        flags: self.tiledata.item_flags(graphic),
                    };
                    out.push((cx, cy, tile));
                }
            }
        }
        self.statics_cache.insert(key, out.clone());
        out
    }

    /// Land tile at world (x, y). Off-map returns an impassable void.
    pub fn land(&mut self, x: u32, y: u32) -> LandTile {
        if x >= MAP_WIDTH || y >= MAP_HEIGHT {
            return LandTile {
                graphic: 0,
                z: 0,
                flags: flags::IMPASSABLE,
            };
        }
        let (bx, by) = (x / BLOCK_SIZE, y / BLOCK_SIZE);
        let (cx, cy) = (x % BLOCK_SIZE, y % BLOCK_SIZE);
        let cells = self.load_land_block(bx, by);
        let (graphic, z) = cells[(cy * BLOCK_SIZE + cx) as usize];
        LandTile {
            graphic,
            z,
            flags: self.tiledata.land_flags(graphic),
        }
    }

    /// Statics at world (x, y).
    pub fn statics(&mut self, x: u32, y: u32) -> Vec<StaticTile> {
        if x >= MAP_WIDTH || y >= MAP_HEIGHT {
            return Vec::new();
        }
        let (bx, by) = (x / BLOCK_SIZE, y / BLOCK_SIZE);
        let (cx, cy) = ((x % BLOCK_SIZE) as u8, (y % BLOCK_SIZE) as u8);
        self.load_statics_block(bx, by)
            .into_iter()
            .filter(|(sx, sy, _)| *sx == cx && *sy == cy)
            .map(|(_, _, t)| t)
            .collect()
    }

    /// Z-aware walkability following ClassicUO's algorithm: can an entity
    /// currently at `current_z` step onto (x, y), and at what Z would it stand?
    /// Returns `Some(new_z)` if walkable, else `None`.
    pub fn walkable_z(&mut self, x: u32, y: u32, current_z: i32) -> Option<i32> {
        let land = self.land(x, y);
        let statics = self.statics(x, y);

        let land_ok = !land.impassable();
        let mut best_z = if land_ok { land.z as i32 } else { current_z };
        let mut has_surface = land_ok;

        for s in &statics {
            let sz = s.z as i32;
            let h = s.height as i32;
            let standing_z = if s.flags & flags::BRIDGE != 0 {
                sz + h / 2
            } else {
                sz + h
            };

            if s.impassable() && !s.surface() {
                let blocker_top = sz + h.max(1);
                if blocker_top > current_z && sz < current_z + CHAR_HEIGHT {
                    return None; // body overlaps a blocker
                }
            } else if s.surface()
                && (standing_z - current_z).abs() <= MAX_STEP
                && (!has_surface || (standing_z - current_z).abs() < (best_z - current_z).abs())
            {
                best_z = standing_z;
                has_surface = true;
            }
        }

        if !has_surface || (best_z - current_z).abs() > MAX_STEP {
            return None;
        }
        Some(best_z)
    }
}

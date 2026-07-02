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

/// ServUO `ItemData.CalcHeight`: a bridge (stairs/ramp) counts as half height.
fn calc_height(flags: u64, height: u8) -> i32 {
    let h = height as i32;
    if flags & flags::BRIDGE != 0 {
        h / 2
    } else {
        h
    }
}

#[derive(Clone, Copy)]
pub struct LandTile {
    pub graphic: u16,
    pub z: i8,
    pub flags: u64,
    /// Texmap id (seamless texture for stretched/sloped rendering); 0 = none.
    pub tex_id: u16,
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
    // Statics bucketed by cell (index = cy*BLOCK_SIZE + cx, 0..64) so a per-tile
    // lookup is O(statics-on-this-cell), not O(whole-block). The whole-block
    // linear filter on every statics() call dominated scene-build time.
    statics_cache: HashMap<u32, Vec<Vec<StaticTile>>>,
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

    // Returns a *reference* into the block cache (no clone): callers copy out the
    // one cell / filter the few statics they need. Cloning the whole 8×8 block on
    // every land()/statics() call dominated scene-build time in dense areas.
    fn load_land_block(&mut self, bx: u32, by: u32) -> &Vec<(u16, i8)> {
        let key = (bx << 16) | by;
        if !self.land_cache.contains_key(&key) {
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
            self.land_cache.insert(key, cells);
        }
        &self.land_cache[&key]
    }

    fn load_statics_block(&mut self, bx: u32, by: u32) -> &Vec<Vec<StaticTile>> {
        let key = (bx << 16) | by;
        if self.statics_cache.contains_key(&key) {
            return &self.statics_cache[&key];
        }
        let block_num = (bx * BLOCKS_Y + by) as usize;
        let idx_off = block_num * 12;
        // One bucket per cell (cy*BLOCK_SIZE + cx), so statics(x,y) is a direct index.
        let mut out: Vec<Vec<StaticTile>> = vec![Vec::new(); (BLOCK_SIZE * BLOCK_SIZE) as usize];
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
                    let cx = self.statics[pos + 2] as u32;
                    let cy = self.statics[pos + 3] as u32;
                    let z = self.statics[pos + 4] as i8;
                    pos += 7;
                    let tile = StaticTile {
                        graphic,
                        z,
                        height: self.tiledata.item_height(graphic),
                        flags: self.tiledata.item_flags(graphic),
                    };
                    if cx < BLOCK_SIZE && cy < BLOCK_SIZE {
                        out[(cy * BLOCK_SIZE + cx) as usize].push(tile);
                    }
                }
            }
        }
        self.statics_cache.insert(key, out);
        &self.statics_cache[&key]
    }

    /// Land tile at world (x, y). Off-map returns an impassable void.
    pub fn land(&mut self, x: u32, y: u32) -> LandTile {
        if x >= MAP_WIDTH || y >= MAP_HEIGHT {
            return LandTile {
                graphic: 0,
                z: 0,
                flags: flags::IMPASSABLE,
                tex_id: 0,
            };
        }
        let (bx, by) = (x / BLOCK_SIZE, y / BLOCK_SIZE);
        let (cx, cy) = (x % BLOCK_SIZE, y % BLOCK_SIZE);
        // Copy the one cell out so the block borrow ends before we touch tiledata.
        let (graphic, z) = self.load_land_block(bx, by)[(cy * BLOCK_SIZE + cx) as usize];
        LandTile {
            graphic,
            z,
            flags: self.tiledata.land_flags(graphic),
            tex_id: self.tiledata.land_tex_id(graphic),
        }
    }

    /// All 64 land cells `(graphic, z)` and the per-cell static lists for an 8×8
    /// block `(bx, by)` (block coords; `0..MAP_WIDTH/8 × 0..MAP_HEIGHT/8`). Both
    /// returned vecs are 64 entries indexed `cell = (y & 7) * 8 + (x & 7)`. This
    /// is the efficient whole-map traversal path (each block read/decoded once via
    /// the per-block caches) used by the world-map renderer — far cheaper than
    /// calling `land()`/`statics()` per tile across all 29M cells.
    pub fn block_cells(&mut self, bx: u32, by: u32) -> (Vec<(u16, i8)>, Vec<Vec<StaticTile>>) {
        let land = self.load_land_block(bx, by).clone();
        let statics = self.load_statics_block(bx, by).clone();
        (land, statics)
    }

    /// Statics at world (x, y).
    pub fn statics(&mut self, x: u32, y: u32) -> Vec<StaticTile> {
        if x >= MAP_WIDTH || y >= MAP_HEIGHT {
            return Vec::new();
        }
        let (bx, by) = (x / BLOCK_SIZE, y / BLOCK_SIZE);
        let (cx, cy) = (x % BLOCK_SIZE, y % BLOCK_SIZE);
        self.load_statics_block(bx, by)[(cy * BLOCK_SIZE + cx) as usize].clone()
    }

    /// Z-aware walkability following ClassicUO's algorithm: can an entity
    /// currently at `current_z` step onto (x, y), and at what Z would it stand?
    /// Returns `Some(new_z)` if walkable, else `None`.
    /// Does a *dynamic* world item (graphic at `item_z`) block a body standing at
    /// `current_z`? Mirrors the static-blocker rule in [`Self::walkable_z`]:
    /// impassable, non-surface, and the body's height span overlaps the item.
    /// Used so we don't try to walk through an impassable placed object.
    /// Equipment animation id (`AnimID`) for an item graphic — used to draw worn
    /// equipment (clothes/hair/beard) by animating this id as a "body". 0 = none.
    pub fn item_anim(&self, graphic: u16) -> u16 {
        self.tiledata.item_anim(graphic)
    }

    /// Tiledata height of an item graphic (for draw-sort priority).
    pub fn item_height(&self, graphic: u16) -> u8 {
        self.tiledata.item_height(graphic)
    }

    /// Tiledata flags of an item graphic (e.g. Background 0x1 for draw-sort).
    pub fn item_flags(&self, graphic: u16) -> u64 {
        self.tiledata.item_flags(graphic)
    }

    /// Does an item graphic emit light (torches/lamps/braziers)? Drives the
    /// per-object night glow in the renderer.
    pub fn item_is_light(&self, graphic: u16) -> bool {
        self.tiledata.item_is_light(graphic)
    }

    /// Is an item graphic a container (chest/bag/corpse)? Lets the client open a
    /// loot window only for real containers (not doors/other double-clickables).
    pub fn item_is_container(&self, graphic: u16) -> bool {
        self.tiledata.item_is_container(graphic)
    }

    /// Does a static/item graphic cycle through frames (flames/fountains/water
    /// wheels)? The frame sequence comes from `animdata.mul`. Drives animated
    /// statics in the renderer.
    pub fn item_is_animated(&self, graphic: u16) -> bool {
        self.tiledata.item_is_animated(graphic)
    }

    pub fn item_blocks(&self, graphic: u16, item_z: i32, current_z: i32) -> bool {
        let f = self.tiledata.item_flags(graphic);
        // Only impassable items block. A table is Impassable+Surface — you cannot
        // stand on it, and it occupies the space above the tile (ServUO IsOk), so
        // it blocks even though it's also a Surface. (Previously we wrongly skipped
        // any Surface item, letting the player try to walk through tables.)
        if f & flags::IMPASSABLE == 0 {
            return false;
        }
        let h = self.tiledata.item_height(graphic) as i32;
        let top = item_z + h.max(1);
        top > current_z && item_z < current_z + CHAR_HEIGHT
    }

    pub fn walkable_z(&mut self, x: u32, y: u32, current_z: i32) -> Option<i32> {
        let land = self.land(x, y);
        let statics = self.statics(x, y);

        // Candidate standing heights: the land surface, plus any *standable* static
        // surface. ServUO: a standable surface is `Surface && !Impassable` — a
        // table is Impassable+Surface, so it is NOT standable.
        let mut candidates: Vec<i32> = Vec::new();
        if !land.impassable() {
            candidates.push(land.z as i32);
        }
        for s in &statics {
            if s.surface() && !s.impassable() {
                candidates.push(s.z as i32 + calc_height(s.flags, s.height));
            }
        }

        // Pick the candidate nearest current_z, within one step, with head room:
        // ServUO MovementImpl.IsOk — nothing that occupies space (Impassable OR
        // Surface) may overlap the body span [z, z+CHAR_HEIGHT). The surface we
        // stand on has its top exactly at z, so it never self-blocks. This is why
        // a table (Impassable+Surface) over the land blocks the tile.
        let mut best: Option<i32> = None;
        for &z in &candidates {
            if (z - current_z).abs() > MAX_STEP {
                continue;
            }
            let our_top = z + CHAR_HEIGHT;
            let blocked = statics.iter().any(|s| {
                (s.impassable() || s.surface()) && {
                    let cz = s.z as i32;
                    cz + calc_height(s.flags, s.height) > z && our_top > cz
                }
            });
            if !blocked && best.is_none_or(|b| (z - current_z).abs() < (b - current_z).abs()) {
                best = Some(z);
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TileFlag.Generic` (0x800, ClassicUO `ItemData.IsStackable`) distinguishes
    /// a real stack (gold coins) from an `amount`-bearing-but-unstackable item
    /// (a backpack's `amount` is unused/1); `scene.rs`'s split-stack dialog needs
    /// this so it doesn't offer to split something the server would reject.
    #[test]
    #[ignore]
    fn item_flags_stackable_bit_matches_known_items() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let map = MapData::open(&dir).expect("open map data");
        assert_ne!(map.item_flags(0x0EED) & 0x800, 0, "gold coins should be stackable");
        assert_eq!(map.item_flags(0x0E75) & 0x800, 0, "a backpack should not be stackable");
    }
}

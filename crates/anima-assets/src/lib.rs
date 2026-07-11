//! UO client data-file readers: UOP container, `tiledata.mul`, and the map
//! (land + statics) with Z-aware walkability.
//!
//! Point this at a local UO installation's data files (they are copyrighted and
//! cannot be redistributed). Provides terrain for `anima-core`'s pathfinder via
//! the [`Terrain`](anima_core::path::Terrain) trait.

pub mod anim;
pub mod animdata;
pub mod art;
pub mod cliloc;
pub mod gump;
pub mod hues;
pub mod map;
pub mod mounts;
pub mod multis;
pub mod radarcol;
pub mod sound;
pub mod texmap;
pub mod tiledata;
pub mod uop;

pub use anim::{Anim, EquipConv};
pub use animdata::AnimData;
pub use art::{Art, Image};
pub use cliloc::Cliloc;
pub use gump::Gumps;
pub use hues::{apply_hue, Hues};
pub use multis::{MultiComponent, Multis};
pub use radarcol::RadarCol;
pub use sound::Sounds;
pub use texmap::Texmaps;
pub use tiledata::TileData;
pub use map::{LandTile, MapData, StaticTile, ZReason, MAP_HEIGHT, MAP_WIDTH};

use anima_core::path::Terrain;

/// Bridge `MapData` to the core pathfinder's terrain interface.
impl Terrain for MapData {
    fn walkable_step(&mut self, x: u32, y: u32, from_z: i32) -> Option<i32> {
        self.walkable_z(x, y, from_z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires local UO data at ~/dev/uo/uo-resource. Ignored by default so the
    /// suite runs without game files; run with `--ignored` to validate.
    #[test]
    #[ignore]
    fn reads_real_spawn_tile() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        // The New Haven spawn the server gave us during login testing.
        let land = map.land(3503, 2574);
        println!("land graphic=0x{:04X} z={}", land.graphic, land.z);
        // It must be walkable from its own Z (the avatar stood here).
        let wz = map.walkable_z(3503, 2574, land.z as i32);
        assert!(wz.is_some(), "spawn tile should be walkable");
    }
}

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
    /// ClassicUO `TileFlag.Container` — the item is a container (chest/bag/corpse);
    /// double-clicking it opens a loot/content window (doors etc. are NOT this).
    pub const CONTAINER: u64 = 0x0020_0000;
    /// ClassicUO `TileFlag.LightSource` (Game/Data/TileFlag.cs) — the item emits
    /// light (torches, lamps, braziers, candles). Used for per-object night glow.
    pub const LIGHT_SOURCE: u64 = 0x0080_0000;
    /// ClassicUO `TileFlag.Animation` (Game/Data/TileFlag.cs) — the static cycles
    /// through frames from `animdata.mul` (flames, fountains, water wheels, magic
    /// flames, …). Used to drive animated-statics frame swapping in the renderer.
    pub const ANIMATION: u64 = 0x0100_0000;
    /// ClassicUO `TileFlag.Door` (Game/Data/TileFlag.cs) — the item is a door.
    /// A closed door is also `IMPASSABLE` (it really does block a live step),
    /// but unlike a wall it can be *opened* — the click-to-walk planner treats
    /// it specially (see `anima_net::scene::tile_walkable_for_planning`),
    /// mirroring ClassicUO's `Pathfinder`'s `SmoothDoors`-style handling and
    /// its `PlayerMobile.TryOpenDoors` auto-open convenience. Ghosts also use
    /// this to phase through closed doors.
    pub const DOOR: u64 = 0x2000_0000;
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
            d[off],
            d[off + 1],
            d[off + 2],
            d[off + 3],
            d[off + 4],
            d[off + 5],
            d[off + 6],
            d[off + 7],
        ])
    }

    fn land_off(&self, graphic: u16) -> usize {
        let g = (graphic & 0x3FFF) as usize;
        (g / 32) * LAND_GROUP + 4 + (g % 32) * LAND_ENTRY
    }

    /// Flags for a land tile graphic (0..0x4000).
    pub fn land_flags(&self, graphic: u16) -> u64 {
        let off = self.land_off(graphic);
        if off + 8 <= self.data.len() {
            self.u64_at(off)
        } else {
            0
        }
    }

    /// Texmap id for a land tile graphic (the seamless texture used when the
    /// tile is stretched/sloped). 0 = none. Lies right after the 8-byte flags.
    pub fn land_tex_id(&self, graphic: u16) -> u16 {
        let off = self.land_off(graphic) + 8;
        if off + 2 <= self.data.len() {
            u16::from_le_bytes([self.data[off], self.data[off + 1]])
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

    /// Equipment animation id (`AnimID`) for a static/item graphic. Worn
    /// equipment (clothes/hair/beard) is drawn by animating this id as if it
    /// were a body in the same `anim.mul` index space. 0 = none.
    ///
    /// In the 41-byte HS item record `animID` is a u16 at offset +14
    /// (flags u64=8, weight u8, quality u8, unk u16, unk1 u8, quantity u8).
    pub fn item_anim(&self, graphic: u16) -> u16 {
        self.item_entry_off(graphic)
            .map(|off| u16::from_le_bytes([self.data[off + 14], self.data[off + 15]]))
            .unwrap_or(0)
    }

    /// Worn `Layer` for an equippable item graphic. In the HS item record the
    /// `quality` byte at offset +9 doubles as the equipment layer (ClassicUO maps
    /// `Quality` → `Layer`). 0 = not normally wearable.
    pub fn item_layer(&self, graphic: u16) -> u8 {
        self.item_entry_off(graphic)
            .map(|off| self.data[off + 9])
            .unwrap_or(0)
    }

    /// Does a static/item graphic emit light (ClassicUO `TileFlag.LightSource`)?
    /// True for torches, lamps, braziers, candles, etc.
    pub fn item_is_light(&self, graphic: u16) -> bool {
        self.item_flags(graphic) & flags::LIGHT_SOURCE != 0
    }

    /// Is the item a container (chest/bag/corpse)? Double-clicking opens its
    /// contents window — used so doors/other items don't spawn an empty window.
    pub fn item_is_container(&self, graphic: u16) -> bool {
        self.item_flags(graphic) & flags::CONTAINER != 0
    }

    /// Does a static/item graphic cycle through frames (ClassicUO
    /// `TileFlag.Animation`)? True for flames, fountains, water wheels, magic
    /// flames, etc.; the frame sequence comes from `animdata.mul`.
    pub fn item_is_animated(&self, graphic: u16) -> bool {
        self.item_flags(graphic) & flags::ANIMATION != 0
    }

    /// Is a static/item graphic a door? See [`flags::DOOR`].
    pub fn item_is_door(&self, graphic: u16) -> bool {
        self.item_flags(graphic) & flags::DOOR != 0
    }

    /// Does the tile's name start with "nodraw"? ClassicUO's "hacky way" to cull
    /// the void/placeholder tiles (`GameObject.cs`:
    /// `data.Name.StartsWith("nodraw", OrdinalIgnoreCase)`) — e.g. static graphic
    /// 8600, whose art is the literal "NO DRAW" bitmap. The 20-byte name field sits
    /// at item-entry offset +21 (after flags..height); we compare the leading 6
    /// bytes case-insensitively, matching ClassicUO's `StartsWith`.
    pub fn item_is_nodraw(&self, graphic: u16) -> bool {
        self.item_entry_off(graphic)
            .is_some_and(|off| self.data[off + 21..off + 27].eq_ignore_ascii_case(b"nodraw"))
    }

    /// Height of a static/item graphic (used for Z stacking/walkability).
    pub fn item_height(&self, graphic: u16) -> u8 {
        // height is the byte at entry offset +20 (after flags+the fixed fields).
        self.item_entry_off(graphic)
            .map(|off| self.data[off + 20])
            .unwrap_or(0)
    }
}

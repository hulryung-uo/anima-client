//! `tiledata.mul` reader (High Seas format, 64-bit flags).
//!
//! Layout (HS): a land section of 512 groups, then an item/static section.
//! - Land group:  `[header u32][32 × (flags u64, texID u16, name[20])]`  (30 B/entry)
//! - Item group:  `[header u32][32 × (flags u64, weight u8, layer u8, count i32,
//!   animID u16, hue u16, lightIndex u16, height u8, name[20])]`  (41 B/entry)
//!
//! Field names and order are ClassicUO's `TileDataLoader`
//! (`src/ClassicUO.Assets/TileDataLoader.cs`, the `staticTiles.Add` loop), which
//! is the only thing that pins them — the file itself carries no schema. Only the
//! FLAGS field changes width between the two layouts (4 vs 8 bytes); everything
//! after it keeps the same relative offsets, which is why the constants below are
//! measured from the END of flags rather than from the start of the entry. Getting
//! that base wrong is not a theoretical risk: it is the bug this comment replaces
//! (see `ITEM_HEIGHT_AFTER_FLAGS`).
//!
//! We only need each tile's flags (and item height) for walkability.

/// Tiledata flag bits we care about (low 32 of the 64-bit flags field).
pub mod flags {
    pub const IMPASSABLE: u64 = 0x0000_0040;
    pub const SURFACE: u64 = 0x0000_0200;
    pub const BRIDGE: u64 = 0x0000_0400;
    /// ClassicUO `TileFlag.Translucent` (`TileDataLoader.cs:425`) — the art is
    /// drawn at partial alpha: spiderwebs, blood, curtains, energy fields, ocean
    /// waves. ClassicUO eases such an object's `AlphaHue` to **178/255**
    /// (`GameSceneDrawingSorting.ProcessAlpha`), not to a half.
    ///
    /// This constant was named `WET` and had no readers, so nothing was ever
    /// wrong at runtime — but the name was: ClassicUO's `Wet` is **0x80**
    /// (`TileDataLoader.cs:441`), one nibble away. Worth recording rather than
    /// deleting, because we will genuinely need `Wet` later: ServUO gates
    /// *movement onto water* on it (`Server/Map.cs:1480`, `:1505`:
    /// `id.Surface || (id.Flags & TileFlag.Wet) != 0`), and 109 of the 272
    /// translucent graphics carry it too, so the two are easy to confuse in
    /// exactly the places it matters.
    pub const TRANSLUCENT: u64 = 0x0000_0008;
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

/// HS (client ≥ 7.0.9.0 / `CV_7090`): 64-bit flags. Pre-HS: 32-bit flags.
/// Detected from file length, not a version string — a HS `tiledata.mul` is
/// always at least the 493_568-byte land section.
const LAND_ENTRY_HS: usize = 30;
const LAND_ENTRY_OLD: usize = 26;
const LAND_GROUP_HS: usize = 4 + 32 * LAND_ENTRY_HS; // 964
const LAND_GROUP_OLD: usize = 4 + 32 * LAND_ENTRY_OLD; // 836
const LAND_GROUPS: usize = 512;
const LAND_SECTION_HS: usize = LAND_GROUPS * LAND_GROUP_HS; // 493_568

/// Byte offset of an item entry's `height`, measured from the END of the flags
/// field: `weight u8` + `layer u8` + `count i32` + `animID u16` + `hue u16` +
/// `lightIndex u16` = 12.
///
/// Measured from the end of flags, not the start of the entry, because that is
/// the only part whose width differs between the two layouts. Both readers used
/// to add 16/17 to `flags_size()` instead — the correct *absolute* HS offsets
/// (20/21) applied to the wrong base, so every name came back four characters
/// short ("cedar tree" → "r tree", "nodraw" → "aw") and every height came back as
/// the fourth byte of the NAME ("cave floor" → 101, i.e. `'e'`). That made
/// `item_is_nodraw` match nothing at all, and fed ASCII codes to every
/// walkability calculation downstream. Named constants now, so the two call
/// sites cannot drift apart again.
const ITEM_HEIGHT_AFTER_FLAGS: usize = 12;
/// Byte offset of an item entry's 20-byte NUL-padded name, from the end of the
/// flags field — the `height` byte and then the name. See
/// [`ITEM_HEIGHT_AFTER_FLAGS`].
const ITEM_NAME_AFTER_FLAGS: usize = ITEM_HEIGHT_AFTER_FLAGS + 1;

const ITEM_ENTRY_HS: usize = 41;
const ITEM_ENTRY_OLD: usize = 37;
const ITEM_GROUP_HS: usize = 4 + 32 * ITEM_ENTRY_HS; // 1316
const ITEM_GROUP_OLD: usize = 4 + 32 * ITEM_ENTRY_OLD; // 1188

pub struct TileData {
    data: Vec<u8>,
    /// True when this is the High Seas 64-bit-flags layout.
    hs: bool,
}

impl TileData {
    pub fn open(path: &std::path::Path) -> std::io::Result<TileData> {
        let mut td = Self::from_bytes(std::fs::read(path)?);
        if let Some(dir) = path.parent() {
            td.apply_verdata(&crate::verdata::Verdata::open(dir));
        }
        Ok(td)
    }

    /// Parse a buffer, picking HS vs pre-HS from its length (ClassicUO
    /// `TileDataLoader`: `Version < CV_7090` is the same split; a real HS file
    /// is never shorter than the HS land section).
    pub fn from_bytes(data: Vec<u8>) -> TileData {
        let hs = data.len() >= LAND_SECTION_HS;
        TileData { data, hs }
    }

    pub fn is_high_seas(&self) -> bool {
        self.hs
    }

    fn flags_size(&self) -> usize {
        if self.hs {
            8
        } else {
            4
        }
    }

    fn land_entry(&self) -> usize {
        if self.hs {
            LAND_ENTRY_HS
        } else {
            LAND_ENTRY_OLD
        }
    }

    fn land_group(&self) -> usize {
        if self.hs {
            LAND_GROUP_HS
        } else {
            LAND_GROUP_OLD
        }
    }

    fn land_section(&self) -> usize {
        LAND_GROUPS * self.land_group()
    }

    fn item_entry(&self) -> usize {
        if self.hs {
            ITEM_ENTRY_HS
        } else {
            ITEM_ENTRY_OLD
        }
    }

    fn item_group(&self) -> usize {
        if self.hs {
            ITEM_GROUP_HS
        } else {
            ITEM_GROUP_OLD
        }
    }

    fn u32_at(&self, off: usize) -> u32 {
        let d = &self.data;
        u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
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

    fn flags_at(&self, off: usize) -> u64 {
        if self.hs {
            if off + 8 <= self.data.len() {
                self.u64_at(off)
            } else {
                0
            }
        } else if off + 4 <= self.data.len() {
            self.u32_at(off) as u64
        } else {
            0
        }
    }

    fn land_off(&self, graphic: u16) -> usize {
        let g = (graphic & 0x3FFF) as usize;
        (g / 32) * self.land_group() + 4 + (g % 32) * self.land_entry()
    }

    /// Flags for a land tile graphic (0..0x4000).
    pub fn land_flags(&self, graphic: u16) -> u64 {
        self.flags_at(self.land_off(graphic))
    }

    /// Texmap id for a land tile graphic (the seamless texture used when the
    /// tile is stretched/sloped). 0 = none. Lies right after the flags field.
    pub fn land_tex_id(&self, graphic: u16) -> u16 {
        let off = self.land_off(graphic) + self.flags_size();
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
        let off = self.land_section() + group * self.item_group() + 4 + within * self.item_entry();
        if off + self.item_entry() <= self.data.len() {
            Some(off)
        } else {
            None
        }
    }

    /// Overlay `verdata.mul` file-id 30 patches (ClassicUO `UOFileManager`).
    /// Land groups are 836 (old) or 964 (HS) bytes; static groups 1188 / 1316.
    /// An empty table is a no-op — modern clients ship without the file.
    pub fn apply_verdata(&mut self, verdata: &crate::verdata::Verdata) {
        let land_group = self.land_group();
        let item_group = self.item_group();
        let land_section = self.land_section();
        for patch in verdata.patches() {
            if patch.file_id != 30 {
                continue;
            }
            let Some(bytes) = verdata.bytes(*patch) else {
                continue;
            };
            let len = bytes.len();
            let dest = if len == land_group {
                let off = (patch.block_id as usize).saturating_mul(land_group);
                if off + land_group <= self.data.len() {
                    Some(off)
                } else {
                    None
                }
            } else if len == item_group {
                let group = patch.block_id.saturating_sub(0x0200) as usize;
                let off = land_section + group.saturating_mul(item_group);
                if off + item_group <= self.data.len() {
                    Some(off)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(off) = dest {
                self.data[off..off + len].copy_from_slice(bytes);
            }
        }
    }

    /// Flags for a static/item graphic.
    pub fn item_flags(&self, graphic: u16) -> u64 {
        self.item_entry_off(graphic)
            .map(|off| self.flags_at(off))
            .unwrap_or(0)
    }

    /// Equipment animation id (`AnimID`) for a static/item graphic. Worn
    /// equipment (clothes/hair/beard) is drawn by animating this id as if it
    /// were a body in the same `anim.mul` index space. 0 = none.
    ///
    /// After the flags field: weight u8, layer u8, count i32, then animID u16
    /// — so `animID` is at `flags_size + 6`.
    pub fn item_anim(&self, graphic: u16) -> u16 {
        let f = self.flags_size();
        self.item_entry_off(graphic)
            .map(|off| u16::from_le_bytes([self.data[off + f + 6], self.data[off + f + 7]]))
            .unwrap_or(0)
    }

    /// Worn `Layer` for an equippable item graphic. The `quality`/`layer` byte
    /// sits immediately after flags+weight (ClassicUO maps `Quality` → `Layer`).
    /// 0 = not normally wearable.
    pub fn item_layer(&self, graphic: u16) -> u8 {
        let f = self.flags_size();
        self.item_entry_off(graphic)
            .map(|off| self.data[off + f + 1])
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
    /// at [`ITEM_NAME_AFTER_FLAGS`] past the flags (item-entry offset +21 in the HS
    /// layout); we compare the leading 6 bytes case-insensitively, matching
    /// ClassicUO's `StartsWith`.
    pub fn item_is_nodraw(&self, graphic: u16) -> bool {
        let name_off = self.flags_size() + ITEM_NAME_AFTER_FLAGS;
        self.item_entry_off(graphic).is_some_and(|off| {
            self.data[off + name_off..off + name_off + 6].eq_ignore_ascii_case(b"nodraw")
        })
    }

    /// The tile's own name — the 20-byte ASCII field after flags..height,
    /// NUL-padded (the same field [`Self::item_is_nodraw`] sniffs). This is UO's
    /// built-in English name for a graphic ("kryss", "war fork"), which is what
    /// ClassicUO's combat book lists under each weapon ability
    /// (`TileData.StaticData[id].Name`). Empty when the graphic is out of range.
    ///
    /// Not a substitute for an OPL name: this one is per-GRAPHIC and knows
    /// nothing about a particular item's magical prefix, dye or crafter.
    pub fn item_name(&self, graphic: u16) -> String {
        let name_off = self.flags_size() + ITEM_NAME_AFTER_FLAGS;
        self.item_entry_off(graphic)
            .map(|off| {
                let raw = &self.data[off + name_off..off + name_off + 20];
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                String::from_utf8_lossy(&raw[..end]).trim().to_string()
            })
            .unwrap_or_default()
    }

    /// Height of a static/item graphic (used for Z stacking/walkability).
    pub fn item_height(&self, graphic: u16) -> u8 {
        let h_off = self.flags_size() + ITEM_HEIGHT_AFTER_FLAGS;
        self.item_entry_off(graphic)
            .map(|off| self.data[off + h_off])
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_format_land_flags_from_short_file() {
        let mut data = vec![0u8; LAND_GROUP_OLD];
        data[4..8].copy_from_slice(&0x40u32.to_le_bytes());
        data[8..10].copy_from_slice(&0x1234u16.to_le_bytes());
        let td = TileData::from_bytes(data);
        assert!(!td.is_high_seas());
        assert_eq!(td.land_flags(0), 0x40);
        assert_eq!(td.land_tex_id(0), 0x1234);
    }

    #[test]
    fn hs_format_from_full_land_section() {
        let mut data = vec![0u8; LAND_SECTION_HS];
        data[4..12].copy_from_slice(&0x200u64.to_le_bytes());
        let td = TileData::from_bytes(data);
        assert!(td.is_high_seas());
        assert_eq!(td.land_flags(0), 0x200);
    }

    /// Build a one-group HS item section whose single entry carries a known
    /// height and name, so the two field offsets are pinned without a data file.
    ///
    /// This is the shape of test that was missing: the reader had `item_flags`,
    /// `item_layer` and `item_anim` right and only `height`/`name` wrong, and
    /// nothing exercised those two — so a four-byte slip in both survived every
    /// gate, and the whole world's static heights were ASCII characters.
    fn hs_item_entry(height: u8, name: &[u8]) -> TileData {
        let mut data = vec![0u8; LAND_SECTION_HS + ITEM_GROUP_HS];
        let off = LAND_SECTION_HS + 4; // past the group header
        data[off..off + 8].copy_from_slice(&0x200u64.to_le_bytes()); // flags: SURFACE
        data[off + 8] = 7; // weight
        data[off + 9] = 21; // layer
        data[off + 14..off + 16].copy_from_slice(&0x1234u16.to_le_bytes()); // animID

        // Absolute offsets, deliberately NOT `ITEM_*_AFTER_FLAGS`: a fixture
        // written through the same constants it is meant to pin moves with them,
        // so it passes for any value of them. (Verified — with the old +16/+17
        // this test still went green; only the real-file test and the last-entry
        // bounds test caught the bug.) 20/21 are ClassicUO's layout: flags 8 +
        // weight 1 + layer 1 + count 4 + animID 2 + hue 2 + lightIndex 2.
        data[off + 20] = height;
        data[off + 21..off + 21 + name.len()].copy_from_slice(name);
        TileData::from_bytes(data)
    }

    #[test]
    fn hs_item_height_and_name_read_their_own_fields() {
        let td = hs_item_entry(20, b"cedar tree");
        assert!(td.is_high_seas());
        // The neighbouring fields must stay where they were — the bug was in the
        // BASE these two are measured from, so a test that only checked height
        // and name could be satisfied by moving everything.
        assert_eq!(td.item_flags(0), 0x200);
        assert_eq!(td.item_layer(0), 21);
        assert_eq!(td.item_anim(0), 0x1234);
        assert_eq!(td.item_height(0), 20);
        assert_eq!(td.item_name(0), "cedar tree");
        // Height 0 is a real value (every floor tile has it), not "absent": the
        // old reader could never return it, because it was reading a name byte.
        assert_eq!(hs_item_entry(0, b"cave floor").item_height(0), 0);
        assert_eq!(hs_item_entry(0, b"cave floor").item_name(0), "cave floor");
        // The nodraw sniff reads the same field, and used to match nothing at all.
        assert!(hs_item_entry(0, b"nodraw").item_is_nodraw(0));
        assert!(!hs_item_entry(0, b"cedar tree").item_is_nodraw(0));
    }

    #[test]
    fn item_name_stays_inside_the_last_entry() {
        // The name is the entry's final 20 bytes, so reading it four bytes late
        // ran off the end of the buffer on the very last graphic — a panic that
        // `/abilities.json?g=65535` reached from the network.
        let td = hs_item_entry(3, b"x");
        // Entry 31 is the last one the buffer holds; 32 is the first past it, so
        // this covers both the in-range boundary and the out-of-range guard.
        let past_end = ((ITEM_GROUP_HS - 4) / ITEM_ENTRY_HS) as u16;
        assert_eq!(past_end, 32);
        for g in 0..=past_end {
            let _ = td.item_name(g);
            let _ = td.item_height(g);
            let _ = td.item_is_nodraw(g);
        }
    }

    /// The five names and two heights this install actually holds — the exact
    /// values the buggy reader got wrong, pinned against the real file so a
    /// future offset slip is caught with a diff a reader can recognise.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real tiledata.mul)
    fn real_tiledata_names_and_heights() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let td = TileData::open(std::path::Path::new(&dir).join("tiledata.mul").as_path())
            .expect("open tiledata.mul");
        for (graphic, name) in [
            (1339u16, "cave floor"), // was "floor"
            (3288, "cedar tree"),    // was "r tree"
            (2960, "table"),         // was "e"
            (8600, "nodraw"),        // was "aw" — so nothing was ever culled
            (1873, "stone stairs"),  // was "e stairs"
        ] {
            assert_eq!(td.item_name(graphic), name, "name of {graphic}");
        }
        // Heights, not just names: the height field is a DIFFERENT offset, and
        // names alone would not have caught it reading the name's fourth byte.
        assert_eq!(td.item_height(1339), 0, "cave floor is flat (was 101, 'e')");
        assert_eq!(td.item_height(3288), 20, "cedar tree (was 97, 'a')");
        assert!(td.item_is_nodraw(8600));
        assert!(!td.item_is_nodraw(3288));
        // Reachable from the network via /tilename and /abilities.json?g=.
        let _ = td.item_name(u16::MAX);
    }

    #[test]
    fn apply_verdata_overwrites_a_land_group() {
        let data = vec![0u8; LAND_GROUP_OLD];
        let td_empty = TileData::from_bytes(data.clone());
        assert_eq!(td_empty.land_flags(0), 0);

        let mut patch = Vec::new();
        patch.extend_from_slice(&1u32.to_le_bytes());
        patch.extend_from_slice(&30u32.to_le_bytes());
        patch.extend_from_slice(&0u32.to_le_bytes());
        patch.extend_from_slice(&24u32.to_le_bytes());
        patch.extend_from_slice(&(LAND_GROUP_OLD as u32).to_le_bytes());
        patch.extend_from_slice(&0u32.to_le_bytes());
        let mut group = vec![0u8; LAND_GROUP_OLD];
        group[4..8].copy_from_slice(&0x40u32.to_le_bytes());
        patch.extend_from_slice(&group);
        let v = crate::verdata::Verdata::parse(patch);

        let mut td = TileData::from_bytes(data);
        td.apply_verdata(&v);
        assert_eq!(td.land_flags(0), 0x40);
    }
}

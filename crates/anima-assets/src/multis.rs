//! Multi (house/boat) reader — `multi.idx` + `multi.mul`: each multi id (a
//! placed boat or house) resolves to a fixed list of static **components**, each
//! at a `(dx, dy, dz)` offset from the multi's own world position. Ported from
//! ClassicUO `MultiLoader` (`ClassicUO.Assets/MultiLoader.cs`), the authoritative
//! reference for the on-disk shapes below.
//!
//! ## Format decision (MUL vs UOP)
//! ClassicUO reads multis from `MultiCollection.uop` (a zlib-compressed container,
//! `CompressionType.Zlib`+) when the install is UOP-native, else falls back to the
//! plain `multi.idx`/`multi.mul` pair. This reader implements **MUL only**: the
//! `~/dev/uo/uo-resource` data directory ships both files, and every id this
//! client actually needs — `SmallBoat` (ids 0-3, ServUO `Boats/SmallBoat.cs`
//! `NorthID..WestID`) and the classic `SmallOldHouse` (id `0x64`, ServUO
//! `Multis/Houses.cs`) — resolves a plausible component list straight out of the
//! MUL (see the `#[ignore]`d real-data test below: 38 components per boat facing,
//! 148 for the house). `MultiCollection.uop` support is left for a future session
//! if a shard ever ships a multi id the MUL doesn't cover.
//!
//! ## On-disk record shape (two sizes — this reader supports both)
//! `multi.idx` is the standard classic idx shape: 12-byte records
//! `[offset:u32][length:u32][extra:u32]` (LE), one per multi id.
//!
//! Each `multi.mul` **component** record starts with the same fixed 12-byte core
//! regardless of client version — `[graphic:u16][x:i16][y:i16][z:i16][flags:u32]`
//! (LE) — but pre-HS (legacy T2A-era) clients pack components back-to-back at a
//! **12-byte stride**, while HS+ (7.0.9.0+) clients pad each record to a
//! **16-byte stride** with 4 trailing reserved bytes (ClassicUO reads the same
//! `MultiBlock` struct either way and just `Skip`s the difference — see
//! `MultiLoader.GetMultis`). `flags != 0` marks a component visible
//! (`MultiInfo.IsVisible`) in both layouts; an invisible component still exists
//! (matters for placement/impassable checks) but is never drawn.
//!
//! Which stride a given `multi.mul` uses isn't recorded anywhere in the file
//! itself (ClassicUO picks it from the *client version* it's emulating, not from
//! file contents) — so this reader detects it once at [`Multis::open`]: if every
//! non-empty idx entry's byte length divides evenly by 16, the file is HS-strided;
//! otherwise it falls back to the legacy 12-byte stride. Verified against the real
//! `uo-resource` data: interpreting `SmallBoat`/`SmallOldHouse` at a 12-byte stride
//! produces nonsense (huge x/y deltas, garbage flags like `0xFFFF_FFFF`-ish
//! values); at 16 bytes it produces small, plausible `dx/dy/dz` and real static
//! graphics (hull/deck/wall ids) — and every one of the file's 800 non-empty
//! entries divides evenly by 16, confirming this install is HS-strided.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

/// One static component of a placed multi (house/boat), at a fixed offset from
/// the multi's own world position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiComponent {
    pub graphic: u16,
    pub dx: i16,
    pub dy: i16,
    pub dz: i16,
    /// `flags != 0` in the raw record (ClassicUO `MultiInfo.IsVisible`). Drives
    /// RENDERING only — ClassicUO never draws an invisible component. Do NOT
    /// use this alone to decide walkability; see [`Self::is_origin`], which a
    /// previous version of this doc wrongly claimed was unnecessary.
    pub visible: bool,
    /// Is this the multi's index-0 component (the first record in its
    /// `multi.mul` list)? ServUO's own live-server loader
    /// (`Server/MultiData.cs::MultiComponentList`, every constructor —
    /// verified directly in source, all four: `if (i == 0 ||
    /// allTiles[i].m_Flags != 0)`) force-includes index 0 in its
    /// collision/placement tile grid **regardless of `m_Flags`** — an earlier
    /// version of this doc assumed the origin tile was always separately
    /// covered by some other visible component and this reader didn't need to
    /// special-case it; live testing contradicted that (see FIX 7 in the
    /// review that added this field). So: an invisible index-0 component must
    /// still count for WALKABILITY (blocking/standing-surface/step-Z — see
    /// `anima_net::scene`'s multi-component fold) even though [`Self::visible`]
    /// says don't draw it — client prediction must not disagree with a
    /// server-side deny on the origin tile just because that tile's component
    /// happens to be flagged invisible.
    pub is_origin: bool,
}

/// `multi.idx` record size: `[offset:u32][length:u32][extra:u32]` LE.
const IDX_ENTRY: usize = 12;
/// Every on-disk component record starts with this fixed core, however it's
/// strided (see the module doc).
const CORE_RECORD: usize = 12;
/// idx sentinel for "no entry" (mirrors every other classic idx/mul pair).
const NO_ENTRY: u32 = 0xFFFF_FFFF;

/// Detect the per-component byte stride (12 legacy / 16 HS+) from `multi.idx`
/// alone: if every non-empty entry's length divides evenly by 16, it's HS+
/// (16-byte stride); otherwise legacy (12-byte). See the module doc for why this
/// can't be read directly from the file format and must be inferred.
fn detect_stride(idx: &[u8]) -> usize {
    let mut saw_nonempty = false;
    let mut all16 = true;
    for chunk in idx.chunks_exact(IDX_ENTRY) {
        let pos = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let len = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        if pos == NO_ENTRY || len == 0 {
            continue;
        }
        saw_nonempty = true;
        if len % 16 != 0 {
            all16 = false;
        }
    }
    if saw_nonempty && all16 {
        16
    } else {
        12
    }
}

/// Parse every multi id's component list out of `idx`/`mul` at the given stride
/// (12 or 16). Pure (no I/O) so it can be exercised directly with synthetic bytes
/// for both stride hypotheses — see the tests below.
fn parse_all(idx: &[u8], mul: &[u8], stride: usize) -> HashMap<u32, Vec<MultiComponent>> {
    let mut out = HashMap::new();
    if stride == 0 {
        return out;
    }
    for (i, chunk) in idx.chunks_exact(IDX_ENTRY).enumerate() {
        let pos = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let len = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        if pos == NO_ENTRY || len == 0 {
            continue;
        }
        let (pos, len) = (pos as usize, len as usize);
        let count = len / stride;
        let mut comps = Vec::with_capacity(count);
        for k in 0..count {
            let rp = pos + k * stride;
            if rp + CORE_RECORD > mul.len() {
                break;
            }
            let graphic = u16::from_le_bytes([mul[rp], mul[rp + 1]]);
            let dx = i16::from_le_bytes([mul[rp + 2], mul[rp + 3]]);
            let dy = i16::from_le_bytes([mul[rp + 4], mul[rp + 5]]);
            let dz = i16::from_le_bytes([mul[rp + 6], mul[rp + 7]]);
            let flags = u32::from_le_bytes([mul[rp + 8], mul[rp + 9], mul[rp + 10], mul[rp + 11]]);
            comps.push(MultiComponent {
                graphic,
                dx,
                dy,
                dz,
                visible: flags != 0,
                is_origin: k == 0,
            });
        }
        if !comps.is_empty() {
            out.insert(i as u32, comps);
        }
    }
    out
}

/// Per-multi-id `(dx, dy) -> components at that tile` grouping (see
/// [`Multis::tile_index`]'s doc).
type TileIndex = HashMap<(i16, i16), Vec<MultiComponent>>;

pub struct Multis {
    /// Every multi id's component list, parsed eagerly at `open()` — the whole
    /// dataset is ~1MB, small enough to just parse up front like the other small
    /// `anima-assets` readers (`Texmaps`, `RadarCol`, ...).
    components: HashMap<u32, Vec<MultiComponent>>,
    /// Per-multi `(dx, dy) -> components at that tile` index, built lazily (once
    /// per multi id, on first request) and cached — so a hot path (walkability
    /// checked per A* node, or every scene build) costs O(components on the ONE
    /// tile asked about), not O(components on the whole multi). `RefCell` so
    /// [`Self::components_at`] only needs `&self`: every walkability call site
    /// threads a plain shared reference instead of `&mut Multis`.
    tile_index: RefCell<HashMap<u32, TileIndex>>,
}

impl Multis {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Multis> {
        let dir = resource_dir.as_ref();
        let idx = std::fs::read(dir.join("multi.idx"))?;
        let mul = std::fs::read(dir.join("multi.mul"))?;
        let stride = detect_stride(&idx);
        let components = parse_all(&idx, &mul, stride);
        Ok(Multis {
            components,
            tile_index: RefCell::new(HashMap::new()),
        })
    }

    /// The full (unindexed) component list for a multi id, e.g. for placement
    /// preview or bulk iteration. `None` if the id has no entry.
    pub fn components(&self, multi_id: u32) -> Option<&[MultiComponent]> {
        self.components.get(&multi_id).map(Vec::as_slice)
    }

    /// Components sitting at tile offset `(dx, dy)` from a placed multi's origin
    /// (e.g. for world tile `(multi.x + dx, multi.y + dy)`). Empty if none (most
    /// tiles near a multi have none). See [`Self::tile_index`]'s doc for why this
    /// is cheap even on a hot per-node/per-scene-build path.
    pub fn components_at(&self, multi_id: u32, dx: i16, dy: i16) -> Vec<MultiComponent> {
        let mut cache = self.tile_index.borrow_mut();
        let grouped = cache.entry(multi_id).or_insert_with(|| {
            let mut g: TileIndex = HashMap::new();
            if let Some(comps) = self.components.get(&multi_id) {
                for c in comps {
                    g.entry((c.dx, c.dy)).or_default().push(*c);
                }
            }
            g
        });
        grouped.get(&(dx, dy)).cloned().unwrap_or_default()
    }

    /// Build a `Multis` directly from an already-parsed component map, with no
    /// file I/O — for synthetic tests (e.g. `anima_net::scene`'s multi-aware
    /// walkability/rendering tests) that need a controlled multi id/component
    /// list without a real `~/dev/uo/uo-resource` install. Real callers always
    /// go through [`Self::open`].
    pub fn from_components(components: HashMap<u32, Vec<MultiComponent>>) -> Multis {
        Multis {
            components,
            tile_index: RefCell::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic `multi.idx` (12-byte records: offset, length, extra) for
    /// a single id 0 pointing at `mul_offset..mul_offset+mul_len`, with every
    /// other slot marked empty (`NO_ENTRY`).
    fn synth_idx(n_ids: usize, id: usize, mul_offset: u32, mul_len: u32) -> Vec<u8> {
        let mut idx = vec![0xFFu8; n_ids * IDX_ENTRY]; // 0xFFFFFFFF = NO_ENTRY for offset+length+extra
        let o = id * IDX_ENTRY;
        idx[o..o + 4].copy_from_slice(&mul_offset.to_le_bytes());
        idx[o + 4..o + 8].copy_from_slice(&mul_len.to_le_bytes());
        idx[o + 8..o + 12].copy_from_slice(&0u32.to_le_bytes());
        idx
    }

    fn push_component(
        mul: &mut Vec<u8>,
        stride: usize,
        graphic: u16,
        dx: i16,
        dy: i16,
        dz: i16,
        flags: u32,
    ) {
        mul.extend_from_slice(&graphic.to_le_bytes());
        mul.extend_from_slice(&dx.to_le_bytes());
        mul.extend_from_slice(&dy.to_le_bytes());
        mul.extend_from_slice(&dz.to_le_bytes());
        mul.extend_from_slice(&flags.to_le_bytes());
        // Pad to the requested stride (0 for legacy 12-byte, 4 reserved bytes for HS 16-byte).
        mul.resize(mul.len() + (stride - CORE_RECORD), 0xAB);
    }

    #[test]
    fn parses_legacy_12_byte_stride() {
        let mut mul = Vec::new();
        push_component(&mut mul, 12, 0x1234, -1, 0, 0, 1); // visible
        push_component(&mut mul, 12, 0x5678, 0, 1, 3, 0); // invisible (flags == 0)
        let idx = synth_idx(4, 0, 0, mul.len() as u32);

        assert_eq!(detect_stride(&idx), 12);
        let all = parse_all(&idx, &mul, 12);
        let comps = all.get(&0).expect("id 0 present");
        assert_eq!(comps.len(), 2);
        assert_eq!(
            comps[0],
            MultiComponent {
                graphic: 0x1234,
                dx: -1,
                dy: 0,
                dz: 0,
                visible: true,
                is_origin: true
            }
        );
        assert_eq!(
            comps[1],
            MultiComponent {
                graphic: 0x5678,
                dx: 0,
                dy: 1,
                dz: 3,
                visible: false,
                is_origin: false
            }
        );
    }

    #[test]
    fn parses_hs_16_byte_stride() {
        let mut mul = Vec::new();
        push_component(&mut mul, 16, 0x0FA0, 2, -2, 5, 0x100); // visible
        push_component(&mut mul, 16, 0x0FA1, -2, 2, 0, 0); // invisible
        push_component(&mut mul, 16, 0x0FA2, 0, 0, 0, 1); // visible
        let idx = synth_idx(4, 0, 0, mul.len() as u32);

        // This entry's length (48 bytes = 3 × 16) also happens to divide evenly
        // by 12, so a single-entry file is ambiguous — `detect_stride`'s global
        // heuristic needs several entries to disambiguate (exercised by
        // `detect_stride_falls_back_to_12_when_not_uniformly_16` below). Here we
        // pin the stride directly to prove the at-16 *parse* path on its own.
        let comps = parse_all(&idx, &mul, 16);
        let comps = comps.get(&0).expect("id 0 present");
        assert_eq!(comps.len(), 3);
        assert_eq!(
            comps[0],
            MultiComponent {
                graphic: 0x0FA0,
                dx: 2,
                dy: -2,
                dz: 5,
                visible: true,
                is_origin: true
            }
        );
        assert_eq!(
            comps[1],
            MultiComponent {
                graphic: 0x0FA1,
                dx: -2,
                dy: 2,
                dz: 0,
                visible: false,
                is_origin: false
            }
        );
        assert_eq!(
            comps[2],
            MultiComponent {
                graphic: 0x0FA2,
                dx: 0,
                dy: 0,
                dz: 0,
                visible: true,
                is_origin: false
            }
        );
    }

    /// ServUO force-includes index 0 in its collision/placement grid
    /// regardless of `m_Flags` (`Server/MultiData.cs::MultiComponentList`,
    /// every constructor: `if (i == 0 || allTiles[i].m_Flags != 0)`) — index 0
    /// must report `is_origin: true` even when it's invisible (`flags == 0`),
    /// and every later index must report `is_origin: false` even when visible.
    #[test]
    fn index_0_is_origin_regardless_of_visibility() {
        let mut mul = Vec::new();
        push_component(&mut mul, 16, 0x2000, 0, 0, 0, 0); // index 0, INVISIBLE
        push_component(&mut mul, 16, 0x2001, 1, 0, 0, 1); // index 1, visible
        let idx = synth_idx(4, 0, 0, mul.len() as u32);

        let comps = parse_all(&idx, &mul, 16);
        let comps = comps.get(&0).expect("id 0 present");
        assert!(
            !comps[0].visible,
            "index 0 in this fixture is invisible (flags == 0)"
        );
        assert!(
            comps[0].is_origin,
            "index 0 must be is_origin regardless of its own visibility"
        );
        assert!(
            !comps[1].is_origin,
            "a later index is never is_origin even when visible"
        );
    }

    /// `detect_stride` picks 16 only when EVERY non-empty entry's length divides
    /// evenly by 16; a file whose entries are only 12-byte-clean must fall back
    /// to legacy.
    #[test]
    fn detect_stride_falls_back_to_12_when_not_uniformly_16() {
        // One entry of 3 12-byte records (36 bytes — divisible by 12, not by 16).
        let idx = synth_idx(2, 0, 0, 36);
        assert_eq!(detect_stride(&idx), 12);
    }

    #[test]
    fn components_at_groups_by_tile_offset_and_skips_other_tiles() {
        let mut mul = Vec::new();
        push_component(&mut mul, 16, 0x1000, 0, 0, 0, 1); // deck at origin
        push_component(&mut mul, 16, 0x1001, 0, 0, 4, 1); // wall over the same tile
        push_component(&mut mul, 16, 0x1002, 1, 0, 0, 1); // a neighboring tile
        let idx = synth_idx(4, 2, 0, mul.len() as u32); // multi id 2

        let m = Multis {
            components: parse_all(&idx, &mul, 16),
            tile_index: RefCell::new(HashMap::new()),
        };
        let at_origin = m.components_at(2, 0, 0);
        assert_eq!(at_origin.len(), 2);
        assert!(at_origin.iter().any(|c| c.graphic == 0x1000));
        assert!(at_origin.iter().any(|c| c.graphic == 0x1001));

        let at_neighbor = m.components_at(2, 1, 0);
        assert_eq!(at_neighbor.len(), 1);
        assert_eq!(at_neighbor[0].graphic, 0x1002);

        // Calling again reuses the cached per-multi index (same result).
        assert_eq!(m.components_at(2, 0, 0).len(), 2);
        // A tile with nothing on it is an empty Vec, not a panic/None-shaped surprise.
        assert!(m.components_at(2, 5, 5).is_empty());
        // An id with no entry at all behaves the same way.
        assert!(m.components_at(999, 0, 0).is_empty());
    }

    #[test]
    fn unknown_multi_id_has_no_components() {
        let idx = synth_idx(4, 0, 0, 0); // id 0 present but empty length
        let m = Multis {
            components: parse_all(&idx, &[], 16),
            tile_index: RefCell::new(HashMap::new()),
        };
        assert!(m.components(0).is_none());
        assert!(m.components(3).is_none());
    }

    /// Requires local UO data at ~/dev/uo/uo-resource. Ignored by default so the
    /// suite runs without game files; run with `--ignored` to validate.
    #[test]
    #[ignore]
    fn reads_real_boat_and_house_multis() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let m = Multis::open(&dir).expect("open multi data");

        // SmallBoat (ServUO `Scripts/Multis/Boats/SmallBoat.cs`): NorthID..WestID
        // are multi ids 0..3, one per facing. Verified directly against the real
        // multi.idx/multi.mul: each resolves 38 components at the HS 16-byte
        // stride (hull/deck/mast pieces) — the 12-byte hypothesis instead yields
        // ~50 records of nonsense (huge x/y, garbage flags), confirming this
        // install's records are HS-strided (see the module doc).
        for id in 0..4u32 {
            let comps = m
                .components(id)
                .unwrap_or_else(|| panic!("SmallBoat id {id} should resolve"));
            assert_eq!(comps.len(), 38, "SmallBoat id {id} component count");
            assert!(
                comps.iter().any(|c| c.visible),
                "id {id} should have at least one visible component"
            );
        }

        // SmallOldHouse / StonePlasterHouse (ServUO `Scripts/Multis/Houses.cs`,
        // `Scripts/Multis/Deeds.cs` `StonePlasterHouseDeed`): multi id 0x64.
        let house = m
            .components(0x64)
            .expect("StonePlasterHouse (0x64) should resolve");
        assert_eq!(house.len(), 148, "StonePlasterHouse component count");
    }
}

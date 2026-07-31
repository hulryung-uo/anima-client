//! Multi (house/boat) reader — `multi.idx` + `multi.mul` + `MultiCollection.uop`:
//! each multi id (a placed boat or house) resolves to a fixed list of static
//! **components**, each at a `(dx, dy, dz)` offset from the multi's own world
//! position. Ported from ClassicUO `MultiLoader`
//! (`ClassicUO.Assets/MultiLoader.cs`), the authoritative reference for the
//! on-disk shapes below.
//!
//! ## Two containers, merged (MUL is the base, UOP fills the gaps)
//! Multi data ships twice on a modern install: the classic `multi.idx`/`multi.mul`
//! pair AND `MultiCollection.uop`. ClassicUO picks exactly ONE of them —
//! `MultiLoader.Load` opens `MultiCollection.uop` when the install is UOP-native
//! (`UOFileManager.IsUOPInstallation`: client >= 7.0.0.0 **and** `MainMisc.uop`
//! present) and then never opens `multi.mul` at all, else the MUL pair.
//!
//! This reader opens **both** and merges, with **`multi.mul` winning every id the
//! two share**; the UOP only supplies ids the MUL has no record for (and, for the
//! shared ids, the one field the MUL cannot answer — see the keep overlay below).
//! Measured on the real `~/dev/uo/uo-resource` set (`multi.idx`: 8480 records,
//! 800 non-empty; `MultiCollection.uop`: 862 ids), the UOP is a superset *by id*
//! — 62 ids exist only there (`0x50-0x53`, `0x147C-0x14A1`, `0x177C`,
//! `0x1DF4-0x1DFB`, `0x2120-0x212A`) and none only in the MUL — but it is **not**
//! a superset by *content*: for the 476 addon multis `0x1F41-0x211F` (forge,
//! anvil, bellows, training dummy, ...) it keeps every component's offsets and
//! zeroes its `graphic`. Taking ClassicUO's wholesale swap would therefore blank
//! 476 multis that render correctly today in order to gain 62.
//!
//! ServUO's pre-built keeps and castles are `0x147C-0x148F`
//! (`Scripts/Multis/HousePlacementTool.cs`: `0x147C` "23x23 3-Story Customizable
//! Keep", `0x147D` "32x32 3-Story Customizable Castle", then `0x147E-0x148F` for
//! the contest designs — `Spires`, `FeudalCastle`, `GrimswindSisters`, ...), all
//! of them UOP-only here; that whole block is why the merge exists. `0x2120-
//! 0x212A` is a *different* group of 11 UOP-only ids that ServUO references
//! nowhere at all (its only `0x212x` mention is `0x2122` as a `MonsterStatuette`
//! *item* graphic, unrelated to multis) — they matter anyway, for the reason in
//! the next section.
//!
//! Neither container is individually required — a pre-7.0 install has no
//! `MultiCollection.uop` at all — so [`Multis::open`] only fails when *neither*
//! loads.
//!
//! ## On-disk record shape — `multi.mul` (two sizes; this reader supports both)
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
//!
//! ## On-disk record shape — `MultiCollection.uop` (a different struct)
//! Each entry is one multi's whole component list, stored (zlib-compressed) under
//! the virtual path `build/multicollection/{id:06}.bin`, and its payload is NOT
//! the MUL's record stream: an `[id:u32][count:u32]` header, then `count` ×
//! `[graphic:u16][dx:i16][dy:i16][dz:i16][flags:u16][cliloc_count:u32]` — 14
//! bytes, no padding — each followed by `cliloc_count` u32 clilocs to skip.
//! ClassicUO reads exactly this (`MultiLoader.GetMultis`'s zlib branch:
//! `MultiBlockNew` + `Skip(Unknown * sizeof(uint))`), as does ServUO's own
//! `Server/MultiData.cs::UOPLoad`. The header's leading u32 is the multi id
//! itself — ClassicUO skips it; verified against the real file, all 862 entries'
//! header id equals the id its virtual path encodes.
//!
//! ## Two questions, not one: "draw it?" and "does the server collide it?"
//! A component's `flags` answers both, and the answers genuinely differ, so
//! every component carries both [`MultiComponent::visible`] (rendering) and
//! [`MultiComponent::server_keeps`] (walkability).
//!
//! In the MUL the two coincide: ClassicUO draws when `flags != 0`
//! (`MultiLoader.GetMultis`, non-zlib branch) and ServUO puts the tile in
//! `MultiComponentList`'s grid on the very same test (`Server/MultiData.cs:753`
//! and its three sibling constructors, all `if (i == 0 || e.m_Flags != 0)`).
//!
//! In the UOP they do not, because that container's `flags` is an enum, not a
//! bitfield. ClassicUO draws when `flags == 0 || flags == 0x100`; ServUO first
//! maps the value to a `TileFlag` (`MultiData.cs:181-187`) and then applies the
//! same non-zero keep test:
//!
//! | UOP flag | ServUO `TileFlag`        | server keeps | ClassicUO draws |
//! |----------|--------------------------|--------------|-----------------|
//! | `0`      | `Background` (0x1)       | yes          | yes             |
//! | `1`      | `None` (0)               | no           | no              |
//! | `0x100`  | `Background` (`default:`)| yes          | yes             |
//! | `0x101`  | `Generic` (0x800)        | yes          | **no**          |
//!
//! `0x101` is therefore not a contradiction between the two references: it is a
//! **nodraw but solid** component. Both readings are right, they just answer
//! different questions — which is exactly why one `visible` flag can't carry
//! them both, and why walkability now folds on `server_keeps`
//! (`anima_net::scene`'s `multi_components_at`) while rendering still folds on
//! `visible`. The real container uses no other values: over all 862 ids the flag
//! is `0` ×174330, `1` ×3119, `0x100` ×452, `0x101` ×907 — the switch's four
//! arms and nothing else.
//!
//! `0x2120-0x212A` are where that bites hardest. On nine of them (`0x2120-
//! 0x2127`, `0x2129`) **not one** component is drawable, while 3-74 components
//! per id are `Impassable` in tiledata: under the old single-flag reading those
//! multis drew nothing AND let A* path straight through, while ServUO blocked
//! every step (live-verified — see the walk test in the review that added
//! `server_keeps`).
//!
//! ### The same split matters on ordinary, MUL-sourced ids: the keep overlay
//! The server on the other end of the socket loads the UOP, not the MUL: ServUO
//! prefers `MultiCollection.uop` whenever the file exists (`MultiData.cs`'s
//! static ctor, `if (File.Exists(multicollectionPath))`), and it looks for it in
//! the *same* directory this reader opens (`Config/DataPath.cfg`'s
//! `CustomPath`). So for a shared id, `multi.mul` is the right source for what to
//! draw but has no standing at all on what the server collides. Measured: 281
//! components across 22 shared ids are `0x101` in the UOP where the MUL flag is
//! `0` — every one of them a tile the server keeps and the MUL-derived flag
//! would have called open. That is the client-open/server-blocked direction, the
//! rubber-band this codebase works hardest to avoid.
//!
//! So the merge overlays the UOP's keep answer onto the MUL's components
//! ([`apply_uop_keep_overlay`]) for every id both containers carry, keyed by
//! `(graphic, dx, dy, dz)` rather than by list position. Position would be wrong:
//! 798 of the 800 shared lists are 1:1, but `0x7E`/`0x7F` are not (3667 MUL
//! records vs 3461 UOP, and the two diverge after index 0), and the UOP list
//! isn't even a subsequence of the MUL's. Under the geometry key the effect is
//! exactly those 281 components and nothing else — the 412 MUL records with no
//! UOP counterpart (all of them `0x7E`/`0x7F`'s extras) keep the MUL's own
//! answer, and no shared id has two UOP components sharing a key but disagreeing
//! on it. Net effect in tiledata terms: 10 of the 281 are `Impassable` and 14
//! `Surface`/`Bridge`, adding blocked tiles to six real, placeable multis
//! (`0xC8`, `0xC9`, `0x1771`, `0x1777`, `0x1779`, `0x177B` — 1-2 tiles each).
//! The 476 addon ids are untouched: their UOP graphics are zeroed, so no key
//! matches (and see the graphic-0 guard in [`parse_uop_components`]).
//!
//! ### Why ClassicUO's *visibility* rule on the 62 UOP-only ids
//! Those ids are the only place the UOP visibility rule is ever used (the MUL
//! wins everywhere else), so the shared-id evidence above says nothing about it.
//! What does: this install is UOP-native (`MainMisc.uop` is present), so
//! ClassicUO would open `MultiCollection.uop` and never `multi.mul` — the rule is
//! being applied to precisely the entries it was written for. On those 62 ids the
//! flag is `0` ×115236, `1` ×1000, `0x100` ×181, `0x101` ×626, so the rule hides
//! 1626 of 117043 components (1.4%). ServUO independently corroborates the
//! `flags == 1` half — it drops exactly those from its grid — and the `0x101`
//! half is the disagreement this module now models rather than resolves: 248 of
//! those 626 are `Impassable`, real collision that must not be dropped merely
//! because it is never drawn.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use crate::uop::UopReader;

/// One static component of a placed multi (house/boat), at a fixed offset from
/// the multi's own world position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiComponent {
    pub graphic: u16,
    pub dx: i16,
    pub dy: i16,
    pub dz: i16,
    /// ClassicUO `MultiInfo.IsVisible`, already normalized across the two
    /// containers' different flag conventions (`flags != 0` in `multi.mul`,
    /// `flags == 0 || flags == 0x100` in `MultiCollection.uop` — see the module
    /// doc). Drives RENDERING only: ClassicUO never draws an invisible
    /// component. Do NOT use this to decide walkability; that's
    /// [`Self::server_keeps`] (plus [`Self::is_origin`]).
    pub visible: bool,
    /// Does the SERVER hold this component in its collision/placement grid?
    /// ServUO's `MultiComponentList` keeps a tile when its `TileFlag` is
    /// non-zero, and the two containers reach that flag differently:
    /// `multi.mul` stores it directly (`m_Flags != 0`,
    /// `Server/MultiData.cs:753`), while `MultiCollection.uop` stores an enum
    /// ServUO maps first (`MultiData.cs:181-187`), where only the value `1`
    /// becomes `TileFlag.None`. So this is NOT [`Self::visible`]: a UOP `0x101`
    /// component is `TileFlag.Generic` — kept by the server — and at the same
    /// time invisible to ClassicUO. Every walkability decision
    /// (blocking/standing-surface/step-Z, `anima_net::scene`'s
    /// `multi_components_at`) folds on this field, so client prediction can't
    /// call a tile open that the server denies. See the module doc for the
    /// table, the UOP-over-MUL keep overlay, and the measured counts.
    pub server_keeps: bool,
    /// Is this the multi's index-0 component (the first record in its component
    /// list, whichever container it came from)? ServUO's own live-server loader
    /// (`Server/MultiData.cs::MultiComponentList`, every constructor —
    /// verified directly in source, all four: `if (i == 0 ||
    /// allTiles[i].m_Flags != 0)`) force-includes index 0 in its
    /// collision/placement tile grid **regardless of `m_Flags`** — an earlier
    /// version of this doc assumed the origin tile was always separately
    /// covered by some other visible component and this reader didn't need to
    /// special-case it; live testing contradicted that (see FIX 7 in the
    /// review that added this field). So an index-0 component counts for
    /// WALKABILITY even when [`Self::server_keeps`] is false — the one case
    /// where the keep flag alone is not the whole answer — and even when
    /// [`Self::visible`] says don't draw it.
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
            // A graphic of 0 can't be drawn or collided (tiledata item 0 carries
            // no flags and no height) — same guard as `parse_uop_components`,
            // where the real data actually produces them; `k` still comes from
            // the raw index so a dropped record can't promote a later one to
            // `is_origin`.
            if graphic == 0 {
                continue;
            }
            comps.push(MultiComponent {
                graphic,
                dx,
                dy,
                dz,
                visible: flags != 0,
                // ServUO reads this container's `m_Flags` straight into its
                // `TileFlag` and keeps the tile when it's non-zero
                // (`Server/MultiData.cs:753`) — the same test ClassicUO draws
                // on, so here the two answers coincide. They don't in the UOP.
                server_keeps: flags != 0,
                is_origin: k == 0,
            });
        }
        if !comps.is_empty() {
            out.insert(i as u32, comps);
        }
    }
    out
}

/// Fixed part of a `MultiCollection.uop` component record (ClassicUO's
/// `MultiBlockNew`, `Pack = 1`): `[graphic:u16][dx:i16][dy:i16][dz:i16]
/// [flags:u16][cliloc_count:u32]`. Unrelated to [`CORE_RECORD`] — the two
/// containers do not share a record layout (see the module doc).
const UOP_RECORD: usize = 14;

/// `MultiCollection.uop` carries no id list: ClassicUO reaches an entry by
/// hashing the virtual path `build/multicollection/{id:06}.bin`
/// (`MultiLoader.Load`), so the only way to learn which ids a container holds
/// is to hash every candidate — which is what ClassicUO's own
/// `UOFileUop.FillEntries` does. This is the candidate range: the full `u16`
/// domain.
///
/// The width isn't derived from any *item* graphic — both `World::items` paths
/// mask a multi's graphic to `& 0x3FFF` (`anima_core::net::game`, 0xF3 and the
/// legacy 0x1A), so the fold in `anima_net::scene` can only ever ask for
/// 14 bits. It's the 0x99 placement tail that needs the rest: `multi_placement_tail`
/// stores the packet's `multi_id` unmasked and `anima_net::scene`'s
/// `placement_json` hands it straight to [`Multis::components`]. Scanning the
/// whole `u16` domain rather than stopping past the largest real id (`0x212A`
/// here; ClassicUO sizes this file's index at `MAX_MULTI_DATA_INDEX_COUNT =
/// 0x2200`) costs one hash per candidate — ~5.5ms once, in [`Multis::open`] —
/// and removes the need to re-tune a bound when a shard ships new ids.
const UOP_ID_SCAN: u32 = u16::MAX as u32 + 1;

/// Parse one already-decompressed `MultiCollection.uop` entry payload into
/// components (layout + the inverted visibility rule are in the module doc).
/// Pure (no I/O) so the UOP record shape can be exercised with synthetic bytes
/// exactly like [`parse_all`] is for the MUL. `None` when the payload is too
/// short to even hold its header; a payload that runs out mid-stream keeps the
/// components it did parse, matching [`parse_all`]'s `break`.
fn parse_uop_components(buf: &[u8]) -> Option<Vec<MultiComponent>> {
    let count = u32::from_le_bytes(buf.get(4..8)?.try_into().ok()?) as usize;
    // Don't pre-allocate on a claimed count: a corrupt/truncated entry can name
    // far more components than the payload can hold (same reasoning as
    // `uop::parse_uop_table`'s record-count bound).
    let mut comps = Vec::new();
    let mut p = 8usize;
    for k in 0..count {
        // Every offset arithmetic step below is `checked_`: a corrupt entry can
        // name a cliloc count that overflows `usize` on a 32-bit target (wasm32)
        // and rewinds `p` into already-parsed bytes.
        let Some(end) = p.checked_add(UOP_RECORD) else {
            break;
        };
        let Some(rec) = buf.get(p..end) else {
            break;
        };
        let le = |o: usize| [rec[o], rec[o + 1]];
        // Each record is trailed by `cliloc_count` u32s (per-component name
        // overrides) carrying no geometry — skipped, as ClassicUO does. Advanced
        // before the record is judged, so the `graphic == 0` skip below can't
        // desynchronize the stream.
        let clilocs = u32::from_le_bytes([rec[10], rec[11], rec[12], rec[13]]) as usize;
        let Some(next) = clilocs
            .checked_mul(4)
            .and_then(|skip| end.checked_add(skip))
        else {
            break;
        };
        p = next;

        let flags = u16::from_le_bytes(le(8));
        let graphic = u16::from_le_bytes(le(0));
        // This container zeroes the graphic of every component of the 476 addon
        // multis `0x1F41-0x211F` (`0x1F42` is `[(0,0,0,0,0), (0,0,1,0,0), ...]`
        // — 1582 such records in the real file, and the MUL has none), so a
        // UOP-only install would otherwise hand the renderer and the collision
        // fold components whose art doesn't exist. Drop them here, the one
        // choke point both paths share. `k` is still the raw record index, so
        // dropping index 0 can't promote index 1 to `is_origin`.
        if graphic == 0 {
            continue;
        }
        comps.push(MultiComponent {
            graphic,
            dx: i16::from_le_bytes(le(2)),
            dy: i16::from_le_bytes(le(4)),
            dz: i16::from_le_bytes(le(6)),
            // ClassicUO `MultiLoader.GetMultis`, zlib branch: `IsVisible =
            // block.Flags == 0 || block.Flags == 0x100` — NOT the MUL's
            // `Flags != 0`.
            visible: flags == 0 || flags == 0x100,
            // ServUO's `UOPLoad` switch (`MultiData.cs:181-187`) maps only the
            // value `1` to `TileFlag.None`; `0`/`0x100` become `Background` and
            // `0x101` becomes `Generic`, all non-zero, all kept by
            // `MultiComponentList`. So `0x101` is nodraw AND solid — see the
            // module doc's table.
            server_keeps: flags != 1,
            is_origin: k == 0,
        });
    }
    Some(comps)
}

/// Replace `mul`'s [`MultiComponent::server_keeps`] with the UOP's answer for
/// every component the two containers both describe, matched on
/// `(graphic, dx, dy, dz)`.
///
/// Why the UOP wins this ONE field on an id `multi.mul` otherwise owns: the
/// server is what `server_keeps` predicts, and ServUO loads
/// `MultiCollection.uop` whenever the file exists (`Server/MultiData.cs`'s
/// static ctor) out of the very directory this reader reads. Everything else
/// still comes from the MUL, which is the only container with usable graphics
/// for the addon range.
///
/// Matched on geometry, not on list position, because `0x7E`/`0x7F` carry 206
/// more MUL records than UOP ones and diverge right after index 0 — see the
/// module doc, which also records the measured effect (281 components, all of
/// them UOP `0x101`) and that no shared id has two UOP components sharing a key
/// but disagreeing. `|=` rather than "last wins" so a duplicated key that DID
/// disagree would resolve toward the server's grid: over-blocking merely refuses
/// a legal step, while under-blocking is the rubber-band.
fn apply_uop_keep_overlay(mul: &mut [MultiComponent], uop: &[MultiComponent]) {
    let mut keep: HashMap<(u16, i16, i16, i16), bool> = HashMap::new();
    for c in uop {
        *keep.entry((c.graphic, c.dx, c.dy, c.dz)).or_insert(false) |= c.server_keeps;
    }
    for c in mul {
        if let Some(&k) = keep.get(&(c.graphic, c.dx, c.dy, c.dz)) {
            c.server_keeps = k;
        }
    }
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
        let mul = std::fs::read(dir.join("multi.idx"))
            .and_then(|idx| std::fs::read(dir.join("multi.mul")).map(|mul| (idx, mul)));
        let uop = UopReader::open(&dir.join("MultiCollection.uop"));

        let mut components = match &mul {
            Ok((idx, mul)) => parse_all(idx, mul, detect_stride(idx)),
            // A UOP-native install may ship no `multi.mul` at all; only
            // "neither container loaded" is fatal (reported as the MUL's own
            // error, which is the one a user is most likely to be missing).
            Err(_) if uop.is_err() => return Err(mul.unwrap_err()),
            Err(_) => HashMap::new(),
        };

        if let Ok(uop) = uop {
            for id in 0..UOP_ID_SCAN {
                if !uop.has_multi(id as usize) {
                    continue;
                }
                let Some(comps) = uop
                    .by_multi(id as usize)
                    .as_deref()
                    .and_then(parse_uop_components)
                else {
                    continue;
                };
                // `multi.mul` wins every id both containers carry — see the
                // module doc's merge rule — except for `server_keeps`, which
                // only the container the SERVER loads can answer.
                match components.get_mut(&id) {
                    Some(mul) => apply_uop_keep_overlay(mul, &comps),
                    None if !comps.is_empty() => {
                        components.insert(id, comps);
                    }
                    None => {}
                }
            }
        }

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
                server_keeps: true,
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
                server_keeps: false,
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
                server_keeps: true,
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
                server_keeps: false,
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
                server_keeps: true,
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

    /// One synthetic UOP record: `(graphic, dx, dy, dz, flags, clilocs)`.
    type UopRec<'a> = (u16, i16, i16, i16, u16, &'a [u32]);

    /// Build a synthetic `MultiCollection.uop` entry payload: `[id][count]`
    /// then 14-byte records, each optionally trailed by `clilocs` u32s.
    fn synth_uop_entry(id: u32, recs: &[UopRec]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&(recs.len() as u32).to_le_bytes());
        for &(graphic, dx, dy, dz, flags, clilocs) in recs {
            buf.extend_from_slice(&graphic.to_le_bytes());
            buf.extend_from_slice(&dx.to_le_bytes());
            buf.extend_from_slice(&dy.to_le_bytes());
            buf.extend_from_slice(&dz.to_le_bytes());
            buf.extend_from_slice(&flags.to_le_bytes());
            buf.extend_from_slice(&(clilocs.len() as u32).to_le_bytes());
            for c in clilocs {
                buf.extend_from_slice(&c.to_le_bytes());
            }
        }
        buf
    }

    /// The UOP record is a DIFFERENT struct from the MUL one — 14 bytes, a
    /// `u16` (not `u32`) flags field, a trailing cliloc list to skip, and the
    /// opposite reading of the flags (see the module doc).
    #[test]
    fn parses_uop_record_layout_with_inverted_visibility() {
        let buf = synth_uop_entry(
            0x147C,
            &[
                (0x0001, 0, 0, 0, 1, &[1061005]), // index 0, INVISIBLE, carries a cliloc
                (0x0FA0, -3, 4, 7, 0, &[]),       // flags 0 => visible
                (0x0FA1, 2, -2, 0, 0x100, &[]),   // flags 0x100 => visible too
                (0x0FA2, 1, 1, 3, 0x101, &[7, 8]), // flags 0x101 => invisible
            ],
        );

        let comps = parse_uop_components(&buf).expect("payload parses");
        assert_eq!(comps.len(), 4);
        assert_eq!(
            comps[0],
            MultiComponent {
                graphic: 0x0001,
                dx: 0,
                dy: 0,
                dz: 0,
                visible: false,
                server_keeps: false,
                is_origin: true
            }
        );
        // The cliloc list on comps[0] must be skipped, not read as geometry —
        // if it weren't, comps[1] would decode as garbage.
        assert_eq!(
            comps[1],
            MultiComponent {
                graphic: 0x0FA0,
                dx: -3,
                dy: 4,
                dz: 7,
                visible: true,
                server_keeps: true,
                is_origin: false
            }
        );
        assert!(comps[2].visible, "0x100 is visible in the UOP layout");
        assert!(comps[2].server_keeps, "0x100 maps to Background => kept");
        assert_eq!((comps[2].dx, comps[2].dy), (2, -2));
        // The whole point of the split: `0x101` is nodraw AND solid.
        assert!(!comps[3].visible, "0x101 is INVISIBLE in the UOP layout");
        assert!(
            comps[3].server_keeps,
            "0x101 maps to TileFlag.Generic — ServUO keeps it in its grid"
        );
        assert_eq!((comps[3].graphic, comps[3].dz), (0x0FA2, 3));
    }

    /// A `graphic` of 0 is the UOP's marker for "this multi's art was moved
    /// elsewhere" (all 476 addon multis `0x1F41-0x211F` are stored that way),
    /// and nothing downstream can draw or collide it — so the parser drops it
    /// without letting the drop shift `is_origin` onto a later record.
    #[test]
    fn uop_graphic_zero_components_are_dropped_without_shifting_is_origin() {
        let buf = synth_uop_entry(
            0x1F42,
            &[
                (0, 0, 0, 0, 0, &[]),        // index 0, zeroed graphic
                (0, 0, 1, 0, 0, &[9]),       // zeroed, and carries a cliloc to skip
                (0x0FA0, 0, 2, 0, 0, &[]),   // a real one
                (0, 0, 3, 0, 0x101, &[]),    // zeroed and "solid" — still dropped
            ],
        );

        let comps = parse_uop_components(&buf).expect("payload parses");
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].graphic, 0x0FA0);
        assert_eq!((comps[0].dx, comps[0].dy), (0, 2));
        assert!(
            !comps[0].is_origin,
            "index 2 must not inherit is_origin from the dropped index 0"
        );
    }

    /// The keep overlay: `multi.mul` keeps the geometry and the drawn set, but
    /// the UOP — the container the server loads — decides `server_keeps`, matched
    /// on `(graphic, dx, dy, dz)` so an unaligned list can't mis-assign it.
    #[test]
    fn uop_keep_overlay_replaces_only_the_keep_flag_and_only_where_keyed() {
        let mut mul = Vec::new();
        push_component(&mut mul, 16, 0x1000, 0, 0, 0, 0); // MUL: nodraw, not kept
        push_component(&mut mul, 16, 0x1001, 1, 0, 0, 0); // MUL: nodraw, not kept
        push_component(&mut mul, 16, 0x1002, 2, 0, 0, 1); // MUL: drawn and kept
        let idx = synth_idx(2, 0, 0, mul.len() as u32);
        let mut comps = parse_all(&idx, &mul, 16).remove(&0).expect("id 0");

        // Deliberately in a different order, one record short, and with the
        // 0x101 flag on the component the MUL called invisible.
        let uop = parse_uop_components(&synth_uop_entry(
            0,
            &[
                (0x1002, 2, 0, 0, 0, &[]),
                (0x1000, 0, 0, 0, 0x101, &[]),
            ],
        ))
        .expect("uop parses");
        apply_uop_keep_overlay(&mut comps, &uop);

        assert!(!comps[0].visible, "rendering still follows multi.mul");
        assert!(comps[0].server_keeps, "0x101 in the UOP => the server keeps it");
        assert!(
            !comps[1].server_keeps,
            "no UOP record keys this one — multi.mul's own answer stands"
        );
        assert!(comps[2].visible && comps[2].server_keeps);
    }

    /// A payload whose declared count outruns its bytes (truncated download,
    /// corrupt entry) keeps what it could parse instead of panicking — same
    /// degradation as `parse_all`'s `break`.
    #[test]
    fn parse_uop_components_survives_a_truncated_payload() {
        let mut buf = synth_uop_entry(1, &[(0x1000, 0, 0, 0, 0, &[]), (0x1001, 1, 0, 0, 0, &[])]);
        buf.truncate(buf.len() - 6); // chop the tail off the second record
        let comps = parse_uop_components(&buf).expect("header still parses");
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].graphic, 0x1000);

        // Too short to even hold the header.
        assert!(parse_uop_components(&[0u8; 4]).is_none());
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

    /// The 62 multi ids that live ONLY in `MultiCollection.uop` — ServUO's
    /// pre-built keeps/castles among them — and the merge rule that lets them
    /// in without displacing anything `multi.mul` already answers for.
    /// Requires local UO data at ~/dev/uo/uo-resource; `--ignored` to run.
    #[test]
    #[ignore]
    fn uop_supplies_the_multi_ids_the_mul_lacks() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let m = Multis::open(&dir).expect("open multi data");

        // What the MUL alone answers for, re-parsed here so the "before" side
        // of the merge is measured rather than assumed.
        let idx = std::fs::read(format!("{dir}/multi.idx")).expect("multi.idx");
        let mul = std::fs::read(format!("{dir}/multi.mul")).expect("multi.mul");
        let mul_only = parse_all(&idx, &mul, detect_stride(&idx));
        assert_eq!(mul_only.len(), 800, "non-empty multi.idx records");
        assert_eq!(m.components.len(), 862, "merged multi id count");

        let missing: Vec<u32> = (0x50..=0x53)
            .chain(0x147C..=0x14A1)
            .chain(0x177C..=0x177C)
            .chain(0x1DF4..=0x1DFB)
            .chain(0x2120..=0x212A)
            .collect();
        assert_eq!(missing.len(), 62);
        for id in &missing {
            assert!(
                !mul_only.contains_key(id),
                "id 0x{id:04X} was supposed to be MUL-less"
            );
            let comps = m
                .components(*id)
                .unwrap_or_else(|| panic!("UOP-only id 0x{id:04X} should resolve"));
            assert!(!comps.is_empty());
            assert!(comps[0].is_origin);
        }

        // 0x147C is the 23x23 3-Story Customizable Keep foundation ServUO
        // places (`Scripts/Multis/HousePlacementTool.cs`): the id whose
        // absence made the placement preview blank and the keep walk-through.
        let keep = m.components(0x147C).expect("keep foundation");
        assert_eq!(keep.len(), 622);
        assert_eq!(keep.iter().filter(|c| c.visible).count(), 621);
        assert!(keep.iter().any(|c| c.dx == -11 && c.dy == -11));

        // Every id the MUL already answered for keeps the MUL's components —
        // the UOP must not win the merge on geometry or on what to draw. The one
        // field it DOES get to overwrite is `server_keeps`, because the server
        // reads the UOP (see the keep-overlay section of the module doc), so
        // compare everything except that. 0x1F42 below is the sharp case for the
        // geometry half: the UOP keeps that addon multi's offsets and zeroes its
        // graphics.
        let mut overlaid = 0usize;
        for (id, comps) in &mul_only {
            let merged = m.components(*id).unwrap_or_else(|| panic!("id 0x{id:04X}"));
            assert_eq!(merged.len(), comps.len(), "id 0x{id:04X} component count");
            for (got, want) in merged.iter().zip(comps) {
                assert_eq!(
                    (got.graphic, got.dx, got.dy, got.dz, got.visible, got.is_origin),
                    (
                        want.graphic,
                        want.dx,
                        want.dy,
                        want.dz,
                        want.visible,
                        want.is_origin
                    ),
                    "id 0x{id:04X}"
                );
                if got.server_keeps != want.server_keeps {
                    overlaid += 1;
                }
            }
        }
        // The overlay has to actually fire, or this test would pass just as well
        // against the pre-overlay reader: 281 components across 22 shared ids are
        // `0x101` in the UOP where the MUL flag is `0` (module doc).
        assert_eq!(overlaid, 281, "components whose keep answer the UOP overrode");
        let bellows = m.components(0x1F42).expect("addon multi 0x1F42");
        assert!(
            bellows.iter().all(|c| c.graphic != 0),
            "0x1F42 must keep multi.mul's graphics, not the UOP's zeroed ones: {bellows:?}"
        );
    }
}

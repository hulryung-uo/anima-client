//! Legacy mobile animation reader (`anim.idx` + `anim.mul`).
//!
//! UO bodies animate via groups (Walk=0, Stand=4 for people; Walk=0, Stand=2 for
//! monsters) × 5 stored directions (the other 3 are mirrored). Each (group,dir)
//! is one idx entry → a palette + frames; each frame is RLE over a 256-color
//! palette. Ported from ClassicUO `AnimationsLoader` (legacy MUL path).
//!
//! Body coverage: people bodies (human 400/401, elf, gargoyle) use the high
//! formula; everything else uses the monster formula. `Body.def` remapping
//! ([`Anim::remap`]) redirects exotic body ids to a real animation body (+ a
//! fallback hue) so they resolve instead of falling back to a marker. `Corpse.def`
//! ([`Anim::remap_corpse`]) is the same idea applied to a dead creature's corpse
//! body (which travels in the corpse item's `amount` field), used with
//! [`Anim::death_group`] to pick the death-pose sprite for a corpse on the ground.
//!
//! Some worn equipment also looks different per wearer: `Equipconv.def`
//! ([`Anim::equip_conv`]) maps (wearer body, item's tiledata AnimID) → a
//! replacement anim graphic + explicit paperdoll gump id + fallback hue — e.g. a
//! human female wearing a "male-cut" robe graphic actually animates/paperdolls as
//! the female graphic. Ported from ClassicUO `ProcessEquipConvDef`.
//!
//! ~300 bodies (`mobtypes.txt` flags bit `0x10000`, `UseUopAnimation`) don't
//! animate from `anim*.mul` at all — their frames live in
//! `AnimationFrame{1..4}.uop`, one `.bin` entry per (body, group) holding ALL 5
//! directions' frames, keyed by `hash("build/animationlegacyframe/{body:D6}/
//! {action:D2}.bin")` where `action` is `group` after `AnimationSequence.uop`'s
//! per-body replace table ([`parse_anim_sequence`]). Ported from ClassicUO
//! `AnimationsLoader.GetIndices`'s UOP branch + `LoadUop` +
//! `ReadUOPAnimationFrames`. [`Anim::frame`]/[`Anim::frame_count`]/
//! [`Anim::frame_centers`] try this path first for a flagged body and fall
//! back to the legacy path only if the UOP container has no data at all for
//! that (body, group) — a deliberate small divergence from ClassicUO (which
//! never falls back once a body is flagged) for extra robustness/coverage.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::art::Image;
use crate::uop::{uop_hash, LazyUopReader, UopReader};

/// People animation groups (35 groups × 5 dirs = 175 entries/body): Stand=4.
pub const PEOPLE_WALK: u8 = 0;
pub const PEOPLE_STAND: u8 = 4;
/// Animal groups (13 groups × 5 = 65 entries/body): Walk=0, Run=1, Stand=2.
pub const ANIMAL_STAND: u8 = 2;
/// Monster/"high" groups (22 groups × 5 = 110 entries/body): Walk=0, Stand=1.
pub const MONSTER_STAND: u8 = 1;
/// Primary death-pose group per kind (ClassicUO `HighAnimationGroup`/
/// `LowAnimationGroup`/`PeopleAnimationGroup`'s `Die1`), used by [`Anim::death_group`].
pub const MONSTER_DIE1: u8 = 2;
pub const ANIMAL_DIE1: u8 = 8;
pub const PEOPLE_DIE1: u8 = 21;

/// ClassicUO `AnimationsLoader.MAX_ACTIONS`: the UOP per-body group-replace
/// table size (gargoyle uses close to all of these). Also the exclusive upper
/// bound a resolved UOP "action" (group after `AnimationSequence.uop`
/// replacement) must fall in to be looked up at all.
const MAX_ACTIONS: usize = 80;

/// Bound on how many decompressed UOP `.bin` payloads [`Anim::uop_cache`]
/// keeps around at once. See that field's doc comment for the eviction policy.
const UOP_CACHE_CAP: usize = 16;

/// One legacy animation file pair (`animN.idx` + `animN.mul`).
struct AnimFile {
    idx: Vec<u8>,
    mul: Mutex<File>,
}

/// One `Equipconv.def` conversion (ClassicUO `EquipConvData`): the replacement
/// anim graphic to draw instead of the item's own tiledata AnimID, an explicit
/// paperdoll gump id, and a fallback hue. See [`Anim::equip_conv`] for the lookup
/// contract and how `gump`'s 0/-1 special cases are resolved at parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquipConv {
    pub graphic: u16,
    pub gump: u16,
    pub hue: u16,
}

pub struct Anim {
    /// Animation files indexed by ClassicUO file index: `[0]` = `anim.mul`, `[1]`
    /// = `anim2.mul`, … `[4]` = `anim5.mul`. `Bodyconv.def` redirects a body into
    /// one of `[1..]`; `None` when that expansion's file isn't installed.
    files: Vec<Option<AnimFile>>,
    /// `Body.def` remap: exotic body id → (real animation body, fallback hue).
    bodydef: HashMap<u16, (u16, u16)>,
    /// `Corpse.def` remap: same idea as `bodydef` but applied to a corpse item's
    /// body (ClassicUO `ReplaceCorpse`) — a SEPARATE table, since a creature's
    /// corpse doesn't always animate from the same body id as the living creature.
    corpsedef: HashMap<u16, (u16, u16)>,
    /// `Bodyconv.def` redirect: body id → (file index 1..=4, graphic in that file).
    bodyconv: HashMap<u16, (u8, u16)>,
    /// `mobtypes.txt`: body id → group kind + UOP-path facts. Authoritative
    /// over the graphic-range heuristic when present. See [`MobTypeEntry`].
    mobtypes: HashMap<u16, MobTypeEntry>,
    /// `Equipconv.def`: (wearer body, item's tiledata AnimID) → conversion. See
    /// [`Self::equip_conv`].
    equipconv: HashMap<(u16, u16), EquipConv>,
    /// `AnimationFrame{1..4}.uop` — the UOP-format per-body/group animation
    /// container set, indexed exactly like `files` (ClassicUO probes them in
    /// order, first hit wins). `None` when that numbered file isn't installed
    /// (older/legacy-only resource sets have none of these at all).
    uop_files: Vec<Option<LazyUopReader>>,
    /// `AnimationSequence.uop`'s per-body group-replace table: `(body, old
    /// group) -> new group` (ClassicUO `UopInfo.ReplacedAnimations`). Sparse —
    /// an unlisted `(body, group)` pair keeps its own group (identity), which
    /// is why this is a plain map rather than a per-body fixed-size array.
    uop_replace: HashMap<(u16, u8), i32>,
    /// Bounded cache of decompressed `.bin` payloads for the UOP path, keyed
    /// by `(body, resolved action)`. One `.bin` holds ALL 5 directions × every
    /// frame of a single group, and a renderer/play-server burst-requests many
    /// frames from the very same one (a whole walk cycle, say) — caching the
    /// decompressed bytes avoids re-inflating the same zlib stream per frame.
    /// Bounded to avoid unbounded growth: once full, the WHOLE cache is
    /// cleared before the next insert. That's the simplest possible bounded
    /// policy (no LRU bookkeeping) and is cheap in the expected access
    /// pattern — a request burst reuses one key many times before moving on
    /// to a different body/group, so a full clear only costs one extra
    /// decompression at the boundary, not a thrash.
    uop_cache: Mutex<UopCache>,
}

/// Key = `(body, resolved UOP action)`, value = that action's decompressed
/// `.bin` bytes. See `Anim::uop_cache`'s doc comment.
type UopCache = HashMap<(u16, u8), Arc<Vec<u8>>>;

/// Parsed `mobtypes.txt` facts about one body: the group `kind` (0 =
/// monster/high, 1 = animal/low, 2 = people — same 3-way split
/// `Anim::anim_type` already exposed) plus two more bits the UOP animation
/// path needs that the old `HashMap<u16, u8>` discarded:
#[derive(Debug, Clone, Copy)]
struct MobTypeEntry {
    kind: u8,
    /// `flags & 0x10000` (ClassicUO `AnimationFlags.UseUopAnimation`): this
    /// body's animation must be read from `AnimationFrame*.uop`, not
    /// `anim*.mul`.
    uop: bool,
    /// The TYPE column was literally `equipment` (ClassicUO
    /// `AnimationGroupsType.Equipment`) — needed ONLY for the UOP path's
    /// `realFrameCount` "min 10 frames" rule; an equipment body still uses the
    /// ordinary people `kind` (2) for group semantics everywhere else.
    equipment: bool,
}

impl Anim {
    pub fn open(resource_dir: impl AsRef<Path>) -> std::io::Result<Anim> {
        let dir = resource_dir.as_ref();
        // File 0 (anim.mul) is mandatory; anim2..anim5 are optional expansion files.
        let open_file = |i: usize| -> Option<AnimFile> {
            let suffix = if i == 0 { String::new() } else { (i + 1).to_string() };
            let idx = std::fs::read(dir.join(format!("anim{suffix}.idx"))).ok()?;
            let mul = File::open(dir.join(format!("anim{suffix}.mul"))).ok()?;
            Some(AnimFile { idx, mul: Mutex::new(mul) })
        };
        if open_file(0).is_none() {
            // Force the same error surface as before when anim.mul/idx are missing.
            std::fs::read(dir.join("anim.idx"))?;
            File::open(dir.join("anim.mul"))?;
        }
        let files: Vec<Option<AnimFile>> = (0..5).map(open_file).collect();
        Ok(Anim {
            files,
            // Body.def is optional (older shards lack it); absent → empty map (no remap).
            bodydef: std::fs::read_to_string(dir.join("Body.def"))
                .map(|t| parse_body_def(&t))
                .unwrap_or_default(),
            // Corpse.def is the exact same line format as Body.def (ClassicUO
            // `ProcessCorpseDef` reuses the identical parse); also optional.
            corpsedef: std::fs::read_to_string(dir.join("Corpse.def"))
                .map(|t| parse_body_def(&t))
                .unwrap_or_default(),
            bodyconv: std::fs::read_to_string(dir.join("Bodyconv.def"))
                .map(|t| parse_body_conv(&t))
                .unwrap_or_default(),
            mobtypes: std::fs::read_to_string(dir.join("mobtypes.txt"))
                .map(|t| parse_mob_types(&t))
                .unwrap_or_default(),
            // Equipconv.def is optional (older shards lack it); absent → no conversions.
            equipconv: std::fs::read_to_string(dir.join("Equipconv.def"))
                .map(|t| parse_equip_conv(&t))
                .unwrap_or_default(),
            // AnimationFrame1..4.uop are optional (legacy-only resource sets have
            // none); a missing/unreadable file is just `None`, same convention as
            // the legacy `files` slots above.
            uop_files: (1..=4)
                .map(|i| LazyUopReader::open(&dir.join(format!("AnimationFrame{i}.uop"))).ok())
                .collect(),
            // AnimationSequence.uop is optional too; absent → every (body, group)
            // keeps its own group (identity replace table).
            uop_replace: UopReader::open(&dir.join("AnimationSequence.uop"))
                .ok()
                .map(|r| parse_anim_sequence(&r))
                .unwrap_or_default(),
            uop_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Apply `Body.def` remapping: return the real animation `(body, hue)` to draw
    /// for `body`. Faithful to ClassicUO `AnimationsLoader.ReplaceBody`: an exotic
    /// body is redirected to a base creature plus a fallback hue; the caller uses
    /// that hue only when the mobile has none of its own. Unmapped → `(body, 0)`.
    pub fn remap(&self, body: u16) -> (u16, u16) {
        self.bodydef.get(&body).copied().unwrap_or((body, 0))
    }

    /// Apply `Corpse.def` remapping (ClassicUO `ReplaceCorpse`): return the real
    /// animation `(body, hue)` for a *corpse's* body (which travels in the ground
    /// item's `amount` field — see `World::Item::amount`'s doc comment). Same
    /// caller contract as [`Self::remap`]: the corpse's own hue wins; this hue is
    /// only a fallback. Unmapped → `(body, 0)`.
    pub fn remap_corpse(&self, body: u16) -> (u16, u16) {
        self.corpsedef.get(&body).copied().unwrap_or((body, 0))
    }

    /// Look up `Equipconv.def`'s conversion for `(wearer body, item's tiledata
    /// AnimID)` — ClassicUO `EquipConversions[body][item.AnimID]`, the SAME two
    /// keys `MobileView`/`ItemView`/`PaperDollInteractable.GetAnimID` use. `body`
    /// must already be [`Self::remap`]-ed (ClassicUO looks this up AFTER
    /// `ConvertBodyIfNeeded`). `None` when this pair has no conversion.
    pub fn equip_conv(&self, body: u16, item_anim: u16) -> Option<EquipConv> {
        self.equipconv.get(&(body, item_anim)).copied()
    }

    /// The primary ("first") death-pose animation group for `body` (already
    /// Corpse.def-remapped — see [`Self::remap_corpse`]), following ClassicUO
    /// `GetDeathAction`: monster/high Die1 = 2, animal/low Die1 = 8, people Die1 =
    /// 21. ClassicUO also offers a "second" Die2 variant selected by a running-flag
    /// bit in the corpse's direction byte; we always draw the primary pose. It also
    /// gives `SeaMonster` its own fixed group (8) instead of Die1/Die2 — our 3-way
    /// `anim_type` collapses `sea_monster` into the monster kind (same as
    /// `stand_group`/`resolveActionGroup` do elsewhere), so a sea monster's corpse
    /// plays the ordinary monster Die1 pose instead; a small, deliberate loss of
    /// fidelity consistent with that existing collapse.
    pub fn death_group(&self, body: u16) -> u8 {
        match self.anim_type(body) {
            0 => MONSTER_DIE1,
            1 => ANIMAL_DIE1,
            _ => PEOPLE_DIE1,
        }
    }

    /// Resolve `Bodyconv.def`: return the `(file index, graphic)` to read `body`
    /// from. Bodies in an expansion live in `anim2..anim5`; everything else stays
    /// in `anim.mul` as `(0, body)`. A redirect whose file isn't installed is
    /// ignored (falls back to the base file), matching ClassicUO's null-file skip.
    fn resolve(&self, body: u16) -> (usize, u16) {
        match self.bodyconv.get(&body) {
            Some(&(fi, g)) if self.files.get(fi as usize).is_some_and(Option::is_some) => {
                (fi as usize, g)
            }
            _ => (0, body),
        }
    }

    /// Byte-independent idx block for (graphic, group, animDir 0..4) given the group
    /// `kind` (0 = monster/high, 1 = animal/low, 2 = people). Each kind has a fixed
    /// group count and section offset (ClassicUO `Calculate*GroupOffset`):
    ///   monster/high: 22 groups × 5 = 110/body, base = g*110
    ///   animal/low:   13 groups × 5 =  65/body, base = 22000+(g-200)*65
    ///   people:       35 groups × 5 = 175/body, base = 35000+(g-400)*175
    /// Returns `None` if the computed block is negative (never valid).
    fn block(graphic: u16, group: u8, dir: u8, kind: u8) -> Option<usize> {
        let g = graphic as i64;
        let base = match kind {
            0 => g * 110,
            1 => (g - 200) * 65 + 22000,
            _ => (g - 400) * 175 + 35000,
        };
        let block = base + group as i64 * 5 + dir as i64;
        (block >= 0).then_some(block as usize)
    }

    /// Locate the idx entry for (body, group, animDir 0..4): resolve `Bodyconv.def`
    /// (file + graphic), pick the group `kind` (`mobtypes.txt` or graphic range),
    /// compute the block, and read `(file index, pos, size)` from that file's idx.
    fn entry(&self, body: u16, group: u8, dir: u8) -> Option<(usize, u32, u32)> {
        let (fi, graphic) = self.resolve(body);
        let kind = self.offset_kind(body, graphic, fi);
        let file = self.files.get(fi)?.as_ref()?;
        let o = Self::block(graphic, group, dir, kind)?.checked_mul(12)?;
        if o + 8 > file.idx.len() {
            return None;
        }
        let pos = u32le(&file.idx, o);
        let size = u32le(&file.idx, o + 4);
        if pos == 0xFFFF_FFFF || size == 0 || size == 0xFFFF_FFFF {
            return None;
        }
        Some((fi, pos, size))
    }

    /// Read `size` bytes at `pos` from animation file `fi`.
    fn read_block(&self, fi: usize, pos: u32, size: u32) -> Option<Vec<u8>> {
        let file = self.files.get(fi)?.as_ref()?;
        let mut f = file.mul.lock().ok()?;
        f.seek(SeekFrom::Start(pos as u64)).ok()?;
        let mut buf = vec![0u8; size as usize];
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    /// Animation group kind for `body`: 0 = monster (high), 1 = animal (low), 2 =
    /// people. Uses `mobtypes.txt` when it covers the body (authoritative), else the
    /// graphic-range heuristic of the file the body resolves into. This is the SAME
    /// kind the reader uses to pick the idx offset, so a renderer that fetches
    /// animations by group number should derive its group numbers from this value
    /// (not from the raw body range) to stay consistent with the file layout.
    pub fn anim_type(&self, body: u16) -> u8 {
        let (fi, graphic) = self.resolve(body);
        self.offset_kind(body, graphic, fi)
    }

    /// The default standing group for a body (varies by kind: monster 1, animal 2,
    /// people 4). Walk is group 0 for every kind.
    pub fn stand_group(&self, body: u16) -> u8 {
        match self.anim_type(body) {
            0 => MONSTER_STAND,
            1 => ANIMAL_STAND,
            _ => PEOPLE_STAND,
        }
    }

    /// Group kind (0/1/2) used to compute the idx offset: `mobtypes.txt` if it
    /// covers the (Body.def-remapped) `body`, else the per-file graphic-range
    /// heuristic. `graphic`/`file_index` come from [`Self::resolve`] (Bodyconv).
    fn offset_kind(&self, body: u16, graphic: u16, file_index: usize) -> u8 {
        match self.mobtypes.get(&body) {
            Some(e) => e.kind,
            None => type_by_graphic(graphic, file_index),
        }
    }

    /// Whether `body` is flagged `UseUopAnimation` in `mobtypes.txt` —
    /// [`Self::frame`]/[`Self::frame_count`]/[`Self::frame_centers`] try the
    /// UOP path first for such a body (see [`Self::uop_bin`]).
    fn is_uop(&self, body: u16) -> bool {
        self.mobtypes.get(&body).is_some_and(|e| e.uop)
    }

    /// Whether `body`'s mobtypes.txt TYPE column is literally `equipment` —
    /// see [`MobTypeEntry::equipment`].
    fn is_equipment(&self, body: u16) -> bool {
        self.mobtypes.get(&body).is_some_and(|e| e.equipment)
    }

    /// Resolve (body, group) to its UOP "action" id via `AnimationSequence.uop`'s
    /// per-body replace table (ClassicUO `ReplacedAnimations[group]`; absent →
    /// identity, i.e. a slot replaces itself). `None` when the FINAL resolved
    /// action is out of range — real data uses this to mean "no animation for
    /// this group on this body" (ClassicUO stores a negative `newGroup` for
    /// exactly this case; the resulting hash simply never matches a real
    /// `.bin` entry).
    ///
    /// The table is applied TWICE, not once — this is the load-bearing (and
    /// non-obvious) part. ClassicUO's actual pipeline is two separate steps
    /// that compose into a double replace:
    ///   1. `Animations.GetIndexAnim` calls `AnimationsLoader.GetIndices`,
    ///      which for EVERY slot `i` in `0..MAX_ACTIONS` probes the hash for
    ///      `ReplacedAnimations[i]` and stores whatever it finds at
    ///      `index.UopGroups[i]` (`AnimationsLoader.cs` ~line 295). So table
    ///      slot `i` holds body/group `replaced[i]`'s `.bin` data — the table
    ///      is indexed by the ORIGINAL slot but contains the REPLACED group's
    ///      data.
    ///   2. `Animations.GetAnimationFrames` (`Animation.cs` ~line 265) calls
    ///      `ReplaceUopGroup(id, ref action)` — i.e. `action = replaced[G]` —
    ///      and THEN indexes `index.UopGroups[action]`, i.e. slot
    ///      `replaced[G]`, which (per step 1) holds `replaced[replaced[G]]`'s
    ///      data.
    ///
    /// Net effect: `bin(replaced[replaced[G]])`, not `bin(replaced[G])`.
    /// Verified against real data: 78 (body, group) pairs across 9 UOP bodies
    /// (1401 Turanchula_Mount, 1417-1422 dragons, 1431, 1434) exist ONLY at
    /// `bin(replaced[replaced[G]])` — `bin(replaced[G])` is absent for every
    /// one of them, zero counter-examples.
    fn uop_action(&self, body: u16, group: u8) -> Option<u8> {
        let replace = |g: u8| self.uop_replace.get(&(body, g)).copied().unwrap_or(g as i32);
        let stage1 = replace(group);
        // Re-apply only if `stage1` is itself a valid table SLOT (0..MAX_ACTIONS)
        // — the only range `uop_replace`'s keys ever occupy (mirrors
        // ClassicUO's fixed-size `ReplacedAnimations` array, which the real
        // pipeline re-indexes with `stage1` in step 2 above).
        let action = u8::try_from(stage1)
            .ok()
            .filter(|&g| (g as usize) < MAX_ACTIONS)
            .map(replace)
            .unwrap_or(stage1);
        (0..MAX_ACTIONS as i32).contains(&action).then_some(action as u8)
    }

    /// Decompressed `.bin` payload for (body, group) — ALL 5 directions' worth
    /// of frames for one UOP animation group. `AnimationFrame{1..4}.uop` are
    /// probed in order, first hit wins (ClassicUO `GetIndices`'s UOP branch).
    /// `None` if `body` isn't UOP-flagged, the replace table maps this group to
    /// nothing (see [`Self::uop_action`]), or no installed UOP file has this
    /// entry. Cached — see `uop_cache`'s doc comment on the eviction policy.
    fn uop_bin(&self, body: u16, group: u8) -> Option<Arc<Vec<u8>>> {
        let action = self.uop_action(body, group)?;
        let key = (body, action);
        if let Ok(cache) = self.uop_cache.lock() {
            if let Some(buf) = cache.get(&key) {
                return Some(buf.clone());
            }
        }
        let path = format!("build/animationlegacyframe/{body:06}/{action:02}.bin");
        let hash = uop_hash(&path);
        let buf = Arc::new(self.uop_files.iter().flatten().find_map(|f| f.by_hash(hash))?);
        if let Ok(mut cache) = self.uop_cache.lock() {
            if cache.len() >= UOP_CACHE_CAP {
                cache.clear(); // simplest bounded policy — see field doc comment
            }
            cache.insert(key, buf.clone());
        }
        Some(buf)
    }

    /// Number of frames in (body, group, UO-direction 0..7), or `None` if absent.
    /// For a UOP-flagged body this is `realFrameCount` — the same for every
    /// direction of a group, so `dir8` only matters for the legacy fallback.
    pub fn frame_count(&self, body: u16, group: u8, dir8: u8) -> Option<usize> {
        if self.is_uop(body) {
            if let Some(buf) = self.uop_bin(body, group) {
                return parse_uop_bin(&buf, self.is_equipment(body)).map(|b| b.real_frame_count);
            }
        }
        let (dir, _) = map_dir(dir8);
        let (fi, pos, size) = self.entry(body, group, dir)?;
        if size < 516 {
            return None;
        }
        let file = self.files.get(fi)?.as_ref()?;
        let mut f = file.mul.lock().ok()?;
        f.seek(SeekFrom::Start(pos as u64 + 512)).ok()?; // skip palette
        let mut b = [0u8; 4];
        f.read_exact(&mut b).ok()?;
        Some(u32::from_le_bytes(b) as usize)
    }

    /// Draw-center `(cx, cy)` for every frame of (body, group, dir) — the cheap
    /// header-only read (no pixel decode) the renderer needs to *position* each
    /// part. Mirror-adjusted to match [`Self::frame`]'s already-flipped image.
    /// A UOP frame with no stored pixels (a gap-filled placeholder — see
    /// [`parse_uop_bin`]) contributes `(0, 0)`.
    pub fn frame_centers(&self, body: u16, group: u8, dir8: u8) -> Option<Vec<(i16, i16)>> {
        let (dir, mirror) = map_dir(dir8);

        if self.is_uop(body) {
            if let Some(buf) = self.uop_bin(body, group) {
                let bin = parse_uop_bin(&buf, self.is_equipment(body))?;
                let mut out = Vec::with_capacity(bin.real_frame_count);
                for idx in 0..bin.real_frame_count {
                    let slot = bin.slot(dir, idx);
                    let Some(slot) = slot.filter(|s| !s.empty) else {
                        out.push((0, 0));
                        continue;
                    };
                    let p = slot.start + slot.pixel_offset as usize + 512; // skip the palette
                    if p + 6 > buf.len() {
                        out.push((0, 0));
                        continue;
                    }
                    let cx = i16le(&buf, p);
                    let cy = i16le(&buf, p + 2);
                    let w = i16le(&buf, p + 4);
                    out.push((if mirror { w - cx } else { cx }, cy));
                }
                return Some(out);
            }
        }

        let (fi, pos, size) = self.entry(body, group, dir)?;
        if size < 516 {
            return None;
        }
        let buf = self.read_block(fi, pos, size)?;
        let frame_count = u32le(&buf, 512) as usize;
        let mut out = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            if 516 + i * 4 + 4 > buf.len() {
                break;
            }
            let foff = u32le(&buf, 516 + i * 4) as usize;
            let p = 512 + foff;
            if p + 8 > buf.len() {
                out.push((0, 0));
                continue;
            }
            let cx = i16le(&buf, p);
            let cy = i16le(&buf, p + 2);
            let w = i16le(&buf, p + 4);
            out.push((if mirror { w - cx } else { cx }, cy));
        }
        Some(out)
    }

    /// Decode one frame of (body, group, UO-direction 0..7, frame_idx).
    /// Returns the RGBA image (already horizontally mirrored when the direction
    /// requires it) plus the frame's draw-center `(cx, cy)`: ClassicUO draws the
    /// `width×height` bitmap with its top-left at `(screenX - cx, screenY - height
    /// - cy)`, so the caller MUST honor the center to align multi-part mobiles
    /// (body + equipment) and especially a rider onto a mount. `cx` is already
    /// adjusted for the mirror (`width - cx`). `None` if absent/undecodable — for
    /// a UOP-flagged body this also covers a legitimately empty gap-filled frame
    /// (see [`parse_uop_bin`]): nothing to draw this tick, same as any other `None`.
    pub fn frame(&self, body: u16, group: u8, dir8: u8, frame_idx: usize) -> Option<(Image, i16, i16)> {
        let (dir, mirror) = map_dir(dir8);

        if self.is_uop(body) {
            if let Some(buf) = self.uop_bin(body, group) {
                let bin = parse_uop_bin(&buf, self.is_equipment(body))?;
                if frame_idx >= bin.real_frame_count {
                    return None;
                }
                let slot = bin.slot(dir, frame_idx)?;
                if slot.empty {
                    return None;
                }
                let p = slot.start + slot.pixel_offset as usize;
                if p + 512 > buf.len() {
                    return None;
                }
                let mut palette = [0u16; 256];
                for (i, c) in palette.iter_mut().enumerate() {
                    *c = u16le(&buf, p + i * 2);
                }
                let (img, cx, cy) = decode_sprite_frame(&buf, p + 512, &palette, true)?;
                return Some(apply_mirror(img, cx, cy, mirror));
            }
            // UOP-flagged but this (body, group) has no UOP data at all (see
            // module docs) — fall through to the legacy path below.
        }

        let (fi, pos, size) = self.entry(body, group, dir)?;
        let buf = self.read_block(fi, pos, size)?;
        if buf.len() < 516 {
            return None;
        }

        let mut palette = [0u16; 256];
        for (i, c) in palette.iter_mut().enumerate() {
            *c = u16le(&buf, i * 2);
        }
        let frame_count = u32le(&buf, 512) as usize;
        if frame_idx >= frame_count {
            return None;
        }
        let foff = u32le(&buf, 516 + frame_idx * 4) as usize;
        let p = 512 + foff;
        let (img, cx, cy) = decode_sprite_frame(&buf, p, &palette, false)?;
        Some(apply_mirror(img, cx, cy, mirror))
    }
}

/// One gap-filled UOP `.bin` frame slot (ClassicUO `UOPFrameData`, after the
/// `ReadUOPAnimationFrames` gap-fill loop): `frame_id` is 1-based and runs
/// contiguously across all 5 directions' worth of frames.
struct UopFrameSlot {
    /// Absolute offset of this slot's own 16-byte record within the `.bin`
    /// buffer. Meaningless (`0`) for a gap-filled placeholder.
    start: usize,
    /// Offset of the pixel payload (palette + sprite) RELATIVE to `start`.
    /// Meaningless for a gap-filled placeholder.
    pixel_offset: u32,
    frame_id: i32,
    /// A gap-filled placeholder with no stored pixels (ClassicUO `Position ==
    /// 0`) — the frame legitimately has nothing to draw for this slot.
    empty: bool,
}

/// Parsed `.bin` frame table: every gap-filled slot plus the derived
/// per-direction frame count.
struct UopBin {
    slots: Vec<UopFrameSlot>,
    real_frame_count: usize,
}

impl UopBin {
    /// The slot for (direction 0..4, frame_idx 0..real_frame_count), if any
    /// stored record maps there. `frameDirection = (frameId-1) /
    /// realFrameCount`, `idx = (frameId-1) % realFrameCount` — ClassicUO
    /// `ReadUOPAnimationFrames`'s direction slicing math, applied as a filter
    /// rather than relying on `slots` being contiguously ordered.
    fn slot(&self, dir: u8, frame_idx: usize) -> Option<&UopFrameSlot> {
        let real = self.real_frame_count as i32;
        self.slots.iter().find(|s| {
            let n = s.frame_id - 1;
            n.div_euclid(real) == dir as i32 && n.rem_euclid(real) as usize == frame_idx
        })
    }
}

/// Parse a decompressed `AnimationFrame*.uop` `.bin` payload's frame table:
/// skip the 32-byte header, read `frameCount`/`dataStart`, read each stored
/// frame's `(group, frameId, pixelOffset)` 16-byte record, gap-fill missing
/// frameIds as empty placeholders, and derive `realFrameCount` (frames per
/// direction: `round(n / 5)`, except Equipment-type bodies use `max(10,
/// round(n / 5))`). Ported from ClassicUO `AnimationsLoader.ReadUOPAnimationFrames`
/// — everything before its per-frame pixel decode (that part is
/// [`decode_sprite_frame`], called once per requested frame instead of eagerly
/// for the whole `.bin`).
fn parse_uop_bin(buf: &[u8], equipment: bool) -> Option<UopBin> {
    if buf.len() < 40 {
        return None;
    }
    let frame_count = i32le(buf, 32);
    if frame_count <= 0 {
        return None;
    }
    let data_start = u32le(buf, 36) as usize;

    let mut raw = Vec::with_capacity(frame_count as usize);
    let mut p = data_start;
    for _ in 0..frame_count {
        if p + 16 > buf.len() {
            break;
        }
        // Record layout: u16 group (unused — see module docs), u16 frameId,
        // u64 unknown (skipped), u32 pixelOffset.
        let frame_id = u16le(buf, p + 2) as i32;
        let pixel_offset = u32le(buf, p + 12);
        raw.push((p, frame_id, pixel_offset));
        p += 16;
    }

    // Gap-fill: ClassicUO's `while (frameData[i].FrameID - lastFrameId > 1)`.
    let mut slots = Vec::with_capacity(raw.len());
    let mut last_frame_id = 1i32;
    for &(start, frame_id, pixel_offset) in &raw {
        while frame_id - last_frame_id > 1 {
            last_frame_id += 1;
            slots.push(UopFrameSlot { start: 0, pixel_offset: 0, frame_id: last_frame_id, empty: true });
        }
        slots.push(UopFrameSlot { start, pixel_offset, frame_id, empty: false });
        last_frame_id = frame_id;
    }

    let max_frame_count = slots.len();
    let real = (max_frame_count as f64 / 5.0).round() as usize;
    let real_frame_count = if equipment { real.max(10) } else { real };
    if real_frame_count == 0 {
        return None;
    }
    Some(UopBin { slots, real_frame_count })
}

/// Decode one legacy-format animation sprite: `i16 centerX, centerY, width,
/// height` at `p` in `buf`, then `0x7FFF7FFF`-terminated RLE runs indexing a
/// 256-color `palette` (ARGB1555). Shared by the legacy MUL path and the UOP
/// path ([`Anim::frame`], both branches) — ClassicUO `ReadSpriteData`, the
/// exact same sprite encoding stored at a different offset in each container.
/// Returns the UNMIRRORED image + center; the caller applies [`apply_mirror`].
/// `alpha_check`: when true (the UOP path only), a palette color that resolves
/// to raw `0` is a transparent hole rather than opaque black (ClassicUO
/// `ReadSpriteData(..., alphaCheck: true)` — only for UOP frames).
fn decode_sprite_frame(buf: &[u8], p: usize, palette: &[u16; 256], alpha_check: bool) -> Option<(Image, i16, i16)> {
    if p + 8 > buf.len() {
        return None;
    }
    let center_x = i16le(buf, p);
    let center_y = i16le(buf, p + 2);
    let width = i16le(buf, p + 4) as i32;
    let height = i16le(buf, p + 6) as i32;
    // ClassicUO's `ReadSpriteData` has NO upper bound at all here (only the
    // `<= 0` rejection below) — real UOP frames run up to 640x480 (body 826
    // Stygian Dragon groups 19-23; bodies 1531/1532 pirate shields), which a
    // 512 cap silently dropped (frame() -> None, mobile blinks out mid-anim).
    // 4096 is our OWN corrupt-data guard (CUO has none), chosen to comfortably
    // clear every real frame while still bounding a malformed frame's
    // allocation to ~4096*4096*4 = 64MB worst case.
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![0u8; w * h * 4];

    let mut q = p + 8;
    loop {
        if q + 4 > buf.len() {
            break;
        }
        let header = u32le(buf, q);
        q += 4;
        if header == 0x7FFF_7FFF {
            break;
        }
        let run = (header & 0x0FFF) as usize;
        let mut x = ((header >> 22) & 0x3FF) as i32;
        if x & 0x200 != 0 {
            x |= !0x3FF; // sign-extend 10-bit
        }
        let mut y = ((header >> 12) & 0x3FF) as i32;
        if y & 0x200 != 0 {
            y |= !0x3FF;
        }
        x += center_x as i32;
        y += center_y as i32 + height;

        for k in 0..run {
            if q >= buf.len() {
                break;
            }
            let c = palette[buf[q] as usize];
            q += 1;
            let px = x + k as i32;
            if px >= 0 && px < width && y >= 0 && y < height {
                if alpha_check && c == 0 {
                    continue; // transparent hole — leave the (already-zero) pixel alone
                }
                let o = (y as usize * w + px as usize) * 4;
                let r = ((c >> 10) & 0x1F) as u8;
                let g = ((c >> 5) & 0x1F) as u8;
                let b = (c & 0x1F) as u8;
                rgba[o] = (r << 3) | (r >> 2);
                rgba[o + 1] = (g << 3) | (g >> 2);
                rgba[o + 2] = (b << 3) | (b >> 2);
                rgba[o + 3] = 255;
            }
        }
    }

    Some((Image { width: w as u32, height: h as u32, rgba }, center_x, center_y))
}

/// Apply direction-mirroring to a decoded frame: flip the image horizontally
/// and flip the draw-center's X (`width - cx`) — same rule for legacy and UOP
/// frames ([`Anim::frame`]'s two branches both end with this).
fn apply_mirror(img: Image, center_x: i16, center_y: i16, mirror: bool) -> (Image, i16, i16) {
    if mirror {
        let cx = img.width as i16 - center_x;
        (flip_h(&img), cx, center_y)
    } else {
        (img, center_x, center_y)
    }
}

/// Parse `Body.def`: each data line is `index {a, b, c…} hue`. The real animation
/// body is `group[2]` when the group lists ≥3 ids, else `group[0]` (ClassicUO
/// `ProcessBodyDef`: "Yes, this is actually how this is supposed to work"). The
/// first entry for an index wins; `#` starts a comment. Malformed lines are skipped.
fn parse_body_def(text: &str) -> HashMap<u16, (u16, u16)> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // Split around the `{ … }` group list.
        let (Some(open), Some(close)) = (line.find('{'), line.find('}')) else { continue };
        if close < open {
            continue;
        }
        let Some(index) = parse_def_int(line[..open].trim()) else { continue };
        let group: Vec<i64> = line[open + 1..close]
            .split(',')
            .filter_map(|t| parse_def_int(t.trim()))
            .collect();
        if group.is_empty() {
            continue;
        }
        let Some(hue) = parse_def_int(line[close + 1..].trim()) else { continue };
        let check = if group.len() >= 3 { group[2] } else { group[0] };
        if !(0..=0xFFFF).contains(&check) || !(0..=0xFFFF).contains(&index) {
            continue;
        }
        // First entry for an index wins (ClassicUO keeps the already-present graphic).
        map.entry(index as u16).or_insert((check as u16, hue.clamp(0, 0xFFFF) as u16));
    }
    map
}

/// Parse `Bodyconv.def`: each line is `index c1 c2 c3 c4 …` where column `i`
/// (1-based) holds this body's graphic in `anim{i+1}.mul` (`-1` = not in that
/// file). ClassicUO writes `_bodyConvInfos[index]` once per non-negative column,
/// so the highest-numbered valid column wins. We store `(file index i, graphic)`;
/// the caller drops redirects whose file isn't installed. `#` starts a comment.
fn parse_body_conv(text: &str) -> HashMap<u16, (u8, u16)> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut toks = line.split_whitespace();
        let Some(index) = toks.next().and_then(parse_def_int) else { continue };
        if !(0..=0xFFFF).contains(&index) {
            continue;
        }
        // Columns i = 1.. → anim{i+1}.mul. Later valid columns overwrite earlier
        // ones (ClassicUO order); only files 1..=4 (anim2..anim5) can ever exist.
        for (i, tok) in toks.enumerate() {
            let col = i + 1;
            if col > 4 {
                break;
            }
            let Some(graphic) = parse_def_int(tok) else { continue };
            if (0..=0xFFFF).contains(&graphic) {
                map.insert(index as u16, (col as u8, graphic as u16));
            }
        }
    }
    map
}

/// Parse `Equipconv.def`: each data line is `body graphic newGraphic gump hue`
/// (ClassicUO `ProcessEquipConvDef`, a `DefReader` with `minsize = 5`). `gump`'s
/// special cases are resolved right here, at parse time, exactly like ClassicUO:
/// a value over `u16::MAX` drops the WHOLE line (the entry is never inserted);
/// `0` means "use the item's own graphic"; `0xFFFF`/`-1` means "use newGraphic";
/// any other value is used as-is (it may already be a fully-baked gump id, e.g.
/// `61250` — see [`Anim::equip_conv`]'s caller for how that's turned into an
/// absolute paperdoll gump). `#` starts a comment; malformed lines are skipped.
fn parse_equip_conv(text: &str) -> HashMap<(u16, u16), EquipConv> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut toks = line.split_whitespace();
        let (Some(body), Some(graphic), Some(new_graphic), Some(gump)) = (
            toks.next().and_then(parse_def_int),
            toks.next().and_then(parse_def_int),
            toks.next().and_then(parse_def_int),
            toks.next().and_then(parse_def_int),
        ) else {
            continue;
        };
        if gump > 0xFFFF {
            continue; // ClassicUO: out-of-range gump silently drops the whole entry.
        }
        let gump = if gump == 0 {
            graphic
        } else if gump == 0xFFFF || gump == -1 {
            new_graphic
        } else {
            gump
        };
        let Some(hue) = toks.next().and_then(parse_def_int) else { continue };
        if !(0..=0xFFFF).contains(&body) || !(0..=0xFFFF).contains(&graphic) || !(0..=0xFFFF).contains(&new_graphic) {
            continue;
        }
        map.insert(
            (body as u16, graphic as u16),
            EquipConv {
                graphic: new_graphic as u16,
                // `gump` was already range-checked (<= 0xFFFF) or replaced above with
                // `graphic`/`new_graphic`, both already validated — always in range.
                gump: gump as u16,
                hue: hue.clamp(0, 0xFFFF) as u16,
            },
        );
    }
    map
}

/// Graphic-range group kind (0 = monster/high, 1 = animal/low, 2 = people) for a
/// file — ClassicUO `CalculateTypeByGraphic`. `anim2` splits monster/animal at 200;
/// `anim3` is animal <300, monster 300..400, people ≥400; the base file and
/// `anim4`/`anim5` use monster <200, animal 200..400, people ≥400. Used only when
/// `mobtypes.txt` doesn't cover the body.
fn type_by_graphic(graphic: u16, file_index: usize) -> u8 {
    match file_index {
        1 => (graphic >= 200) as u8, // anim2: <200 monster, else animal
        2 => {
            if graphic < 300 {
                1
            } else if graphic < 400 {
                0
            } else {
                2
            }
        }
        _ => {
            if graphic < 200 {
                0
            } else if graphic < 400 {
                1
            } else {
                2
            }
        }
    }
}

/// Parse `mobtypes.txt`: each data line is `id TYPE flags`, where TYPE is one of
/// `monster`/`sea_monster`/`animal`/`human`/`equipment` and `flags` is hex. We map
/// TYPE → group kind (0 monster/high, 1 animal/low, 2 people); `sea_monster` uses
/// the high group like a monster, and `equipment` uses the people group like a
/// human. For a monster, the offset-override flags apply (ClassicUO `CalculateOffset`):
/// `CalculateOffsetByPeopleGroup` (0x400) → people, `CalculateOffsetByLowGroup`
/// (0x40) → animal. `#` starts a comment; lines not beginning with a digit are skipped.
/// Also records two more per-body facts the UOP path needs (see [`MobTypeEntry`]):
/// the `UseUopAnimation` flag bit (`0x10000`) and whether TYPE was literally `equipment`.
fn parse_mob_types(text: &str) -> HashMap<u16, MobTypeEntry> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let mut toks = line.split_whitespace();
        let Some(id) = toks.next().and_then(|t| t.parse::<u16>().ok()) else { continue };
        let Some(ty) = toks.next() else { continue };
        let equipment = ty.eq_ignore_ascii_case("equipment");
        let mut kind = match ty.to_ascii_lowercase().as_str() {
            "monster" | "sea_monster" => 0u8,
            "animal" => 1,
            "human" | "equipment" => 2,
            _ => continue,
        };
        // Flags column: hex, possibly with a trailing `# comment`. Absent/`#` → 0.
        let flags = toks
            .next()
            .and_then(|t| t.split('#').next())
            .filter(|t| !t.is_empty())
            .and_then(|t| i64::from_str_radix(t, 16).ok())
            .unwrap_or(0);
        if kind == 0 {
            if flags & 0x400 != 0 {
                kind = 2; // CalculateOffsetByPeopleGroup
            } else if flags & 0x40 != 0 {
                kind = 1; // CalculateOffsetByLowGroup
            }
        }
        let uop = flags & 0x1_0000 != 0; // AnimationFlags.UseUopAnimation
        map.insert(id, MobTypeEntry { kind, uop, equipment });
    }
    map
}

/// Parse a `.def` integer token (decimal, or `0x`-prefixed hex). `None` if unparseable.
fn parse_def_int(t: &str) -> Option<i64> {
    let t = t.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()
    } else {
        t.parse().ok()
    }
}

/// 8-direction (UO) → (stored anim dir 0..4, mirror). From ClassicUO GetAnimDirection.
fn map_dir(dir8: u8) -> (u8, bool) {
    match dir8 & 7 {
        2 => (1, true),
        4 => (1, false),
        1 => (2, true),
        5 => (2, false),
        0 => (3, true),
        6 => (3, false),
        3 => (0, false),
        7 => (4, false),
        _ => (0, false),
    }
}

fn flip_h(img: &Image) -> Image {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let s = (y * w + x) * 4;
            let d = (y * w + (w - 1 - x)) * 4;
            rgba[d..d + 4].copy_from_slice(&img.rgba[s..s + 4]);
        }
    }
    Image {
        width: img.width,
        height: img.height,
        rgba,
    }
}

fn u32le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn u16le(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn i16le(d: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([d[o], d[o + 1]])
}
fn i32le(d: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// Parse `AnimationSequence.uop`'s per-body group-replace table by scanning
/// every entry, in FILE ORDER ([`UopReader::all_entries`] — the body id lives
/// inside the (decompressed) payload, not in a path we could hash up front).
/// See [`parse_anim_sequence_entry`] for the per-entry format.
///
/// Real data has duplicate animID entries for a handful of bodies (29, 704,
/// 1254). ClassicUO's loader is a plain `_uopInfos[animID] = uopInfo`
/// assignment per entry (`AnimationsLoader.cs` ~line 865): a LATER entry
/// (in file order) for the same body REPLACES the earlier `UopInfo` — a whole
/// fresh 80-slot table, not a merge — so any group the earlier entry touched
/// but the later one doesn't reverts to identity, it does NOT keep the
/// earlier entry's value. We collect each body's FILE-ORDER-LAST entry first
/// (last `HashMap::insert` for a body wins, same as ClassicUO's assignment)
/// and only flatten those into the sparse `(body, group) -> new_group` map —
/// never merging two different entries' replacements for one body.
fn parse_anim_sequence(uop: &UopReader) -> HashMap<(u16, u8), i32> {
    let mut last_by_body: HashMap<u16, Vec<(u8, i32)>> = HashMap::new();
    for buf in uop.all_entries() {
        if let Some((body, replacements)) = parse_anim_sequence_entry(&buf) {
            last_by_body.insert(body, replacements); // later entry replaces the WHOLE prior table
        }
    }
    let mut map = HashMap::new();
    for (body, replacements) in last_by_body {
        for (old_group, new_group) in replacements {
            map.insert((body, old_group), new_group);
        }
    }
    map
}

/// Parse one decompressed `AnimationSequence.uop` entry: `u32 animId`, skip 48
/// bytes (credited-to-`@tristran` unknown u32s), `u32 replaceCount`, then
/// `replaceCount × { i32 oldGroup, u32 frameCount, i32 newGroup, skip 60 }` —
/// `frameCount == 0` means `oldGroup` is replaced by `newGroup` (ClassicUO
/// `LoadUop`; the fixed 60-byte skip assumes each replace record's optional
/// trailing `num1`/`num2` arrays are empty, which holds for real data EXCEPT
/// a handful of `replaceCount` values ClassicUO special-cases below). A
/// trailing "xtra" (mode-dependent fallback) section follows but isn't parsed
/// — we don't need per-mode animation selection.
///
/// `replaceCount == 48 || == 68` skips the WHOLE replace list, exactly like
/// ClassicUO — a guard against real bodies (400 human, 666 gargoyle, 1253)
/// whose replace records actually DO carry non-empty `num1`/`num2` arrays,
/// which would misalign this fixed-skip parse; neither we nor ClassicUO parse
/// those variable-length arrays, so those bodies simply get an identity
/// replace table (no override) instead of a misdecoded one.
///
/// Returns `(body, [(old_group, new_group)])` for the entries actually
/// replaced; `None` if the buffer is too short to hold the fixed header.
fn parse_anim_sequence_entry(buf: &[u8]) -> Option<(u16, Vec<(u8, i32)>)> {
    const HEADER: usize = 4 + 48; // animId + unknown u32s
    if buf.len() < HEADER + 4 {
        return None;
    }
    let anim_id = u32le(buf, 0);
    let mut p = HEADER;
    let replaces = i32le(buf, p);
    p += 4;

    let mut out = Vec::new();
    if replaces > 0 && replaces != 48 && replaces != 68 {
        for _ in 0..replaces {
            if p + 12 > buf.len() {
                break;
            }
            let old_group = i32le(buf, p);
            let frame_count = u32le(buf, p + 4);
            let new_group = i32le(buf, p + 8);
            p += 12 + 60;
            if frame_count == 0 && (0..MAX_ACTIONS as i32).contains(&old_group) {
                out.push((old_group as u8, new_group));
            }
            if p > buf.len() {
                break;
            }
        }
    }
    (anim_id <= u16::MAX as u32).then_some((anim_id as u16, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn decodes_human_stand_frame() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        // Human male (0x190 = 400), standing, facing south (dir 4), frame 0.
        let (img, _cx, _cy) = anim.frame(400, PEOPLE_STAND, 4, 0).expect("human stand frame");
        println!("human stand frame: {}x{}", img.width, img.height);
        assert!(img.width > 0 && img.height > 0);
        assert!(!img.is_empty(), "frame should have opaque pixels");
    }

    #[test]
    fn body_def_remaps_and_picks_third_or_first() {
        let def = "\
# comment line
11 {28} 1401
46 {12, 59} 1106
100 {7, 8, 9} 0     # 3+ ids → use the third (9)
11 {99} 42         # duplicate index: first entry wins
bad line without braces
";
        let map = parse_body_def(def);
        // 2-id (and 1-id) groups use group[0]; hue carried through.
        assert_eq!(map.get(&11), Some(&(28, 1401)));
        assert_eq!(map.get(&46), Some(&(12, 1106)));
        // 3+ ids → group[2].
        assert_eq!(map.get(&100), Some(&(9, 0)));
        // Duplicate index 11 kept its first mapping (28), not the later 99.
        assert_eq!(map.get(&11).unwrap().0, 28);
    }

    /// Build an `Anim` with no backing files (`entry`/`frame` calls would fail),
    /// but enough of the def/mobtypes tables populated to exercise the pure
    /// remap/kind logic below (`remap_corpse`, `anim_type`, `death_group`).
    fn test_anim(corpsedef: HashMap<u16, (u16, u16)>, mobtypes: HashMap<u16, u8>) -> Anim {
        Anim {
            files: Vec::new(),
            bodydef: HashMap::new(),
            bodyconv: HashMap::new(),
            mobtypes: mobtypes
                .into_iter()
                .map(|(id, kind)| (id, MobTypeEntry { kind, uop: false, equipment: false }))
                .collect(),
            corpsedef,
            equipconv: HashMap::new(),
            uop_files: Vec::new(),
            uop_replace: HashMap::new(),
            uop_cache: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn corpse_def_reuses_body_def_format_and_remaps() {
        // Corpse.def is parsed with the exact same reader as Body.def.
        let def = "\
# a single-id group → group[0], hue carried through
99 {77} 0
# 3+ ids → group[2]
5 {1, 2, 3} 555
";
        let anim = test_anim(parse_body_def(def), HashMap::new());
        assert_eq!(anim.remap_corpse(99), (77, 0));
        assert_eq!(anim.remap_corpse(5), (3, 555));
        // Unmapped body passes through unchanged with hue 0 (caller's own hue wins).
        assert_eq!(anim.remap_corpse(12345), (12345, 0));
    }

    #[test]
    fn death_group_picks_primary_group_by_kind() {
        // No mobtypes entries → falls back to the graphic-range heuristic (monster
        // <200, animal 200..400, people >=400) for the base file.
        let anim = test_anim(HashMap::new(), HashMap::new());
        assert_eq!(anim.death_group(100), MONSTER_DIE1);
        assert_eq!(anim.death_group(250), ANIMAL_DIE1);
        assert_eq!(anim.death_group(400), PEOPLE_DIE1);

        // mobtypes.txt overrides the range heuristic, same as anim_type/stand_group.
        let mut mobtypes = HashMap::new();
        mobtypes.insert(50, 2); // a "monster" id remapped to people by flags upstream
        let anim = test_anim(HashMap::new(), mobtypes);
        assert_eq!(anim.death_group(50), PEOPLE_DIE1);
    }

    #[test]
    fn body_conv_parses_columns_and_last_valid_wins() {
        let def = "\
# object type → file index table
157\t1\t-1\t-1\t-1\t-1
11\t3\t-1\t-1\t-1\t-1
28\t-1\t-1\t-1\t-1\t-1
300\t-1\t5\t-1\t-1\t-1
900\t-1\t-1\t2\t7\t-1
bad
";
        let map = parse_body_conv(def);
        // Column 1 → anim2 (file index 1).
        assert_eq!(map.get(&157), Some(&(1, 1)));
        assert_eq!(map.get(&11), Some(&(1, 3)));
        // All -1 → no redirect.
        assert_eq!(map.get(&28), None);
        // Column 2 → anim3 (file index 2).
        assert_eq!(map.get(&300), Some(&(2, 5)));
        // Columns 3 and 4 both valid → the later (higher) column wins: anim5 (4).
        assert_eq!(map.get(&900), Some(&(4, 7)));
    }

    #[test]
    fn equip_conv_parses_gump_special_cases() {
        let def = "\
# 15th Anniversary Robes: gump given explicitly (already a baked female gump id)
401\t1249\t1250\t61250\t0\t#\tHuman M to F
# gump 0 → use the item's own graphic (female chain substitution)
401\t538\t986\t0\t0\t#\tfemale chain substitution
# gump -1 → use newGraphic
606\t968\t977\t-1\t0\t#\thide chest
# gump 0xFFFF (equivalent to -1) → use newGraphic
605\t1\t2\t65535\t7\t#\tsynthetic 0xFFFF case
# out-of-range gump (> u16::MAX) → the WHOLE line is dropped
700\t9\t10\t100000\t0
bad line
";
        let map = parse_equip_conv(def);
        assert_eq!(map.get(&(401, 1249)), Some(&EquipConv { graphic: 1250, gump: 61250, hue: 0 }));
        assert_eq!(map.get(&(401, 538)), Some(&EquipConv { graphic: 986, gump: 538, hue: 0 }));
        assert_eq!(map.get(&(606, 968)), Some(&EquipConv { graphic: 977, gump: 977, hue: 0 }));
        assert_eq!(map.get(&(605, 1)), Some(&EquipConv { graphic: 2, gump: 2, hue: 7 }));
        // Dropped entirely — never inserted under any key.
        assert_eq!(map.get(&(700, 9)), None);
        assert_eq!(map.len(), 4);
    }

    #[test]
    fn equip_conv_lookup_is_keyed_by_wearer_body_and_item_anim() {
        let def = "401\t1249\t1250\t61250\t0\n606\t1249\t1252\t61252\t0\n";
        let mut anim = test_anim(HashMap::new(), HashMap::new());
        anim.equipconv = parse_equip_conv(def);
        // Same item AnimID (1249), different wearer body → different conversion.
        assert_eq!(anim.equip_conv(401, 1249), Some(EquipConv { graphic: 1250, gump: 61250, hue: 0 }));
        assert_eq!(anim.equip_conv(606, 1249), Some(EquipConv { graphic: 1252, gump: 61252, hue: 0 }));
        // Unmapped (body, item) pair → None.
        assert_eq!(anim.equip_conv(400, 1249), None);
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real Equipconv.def)
    fn equip_conv_real_data_sanity() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        assert!(!anim.equipconv.is_empty(), "Equipconv.def should have loaded entries");
        // Every stored gump is a valid u16 (parse-time range checks held) and every
        // entry is reachable through the public lookup under its own keys.
        for (&(body, graphic), ec) in &anim.equipconv {
            assert_eq!(anim.equip_conv(body, graphic), Some(*ec));
        }
        println!("Equipconv.def: {} entries", anim.equipconv.len());
    }

    #[test]
    fn type_by_graphic_matches_classicuo_per_file() {
        // Base file (0) / anim4 / anim5: monster<200, animal 200..400, people ≥400.
        assert_eq!(type_by_graphic(100, 0), 0);
        assert_eq!(type_by_graphic(250, 0), 1);
        assert_eq!(type_by_graphic(400, 0), 2);
        assert_eq!(type_by_graphic(250, 3), 1);
        // anim2: monster<200, else animal.
        assert_eq!(type_by_graphic(150, 1), 0);
        assert_eq!(type_by_graphic(250, 1), 1);
        // anim3: animal <300, monster 300..400, people ≥400.
        assert_eq!(type_by_graphic(250, 2), 1);
        assert_eq!(type_by_graphic(350, 2), 0);
        assert_eq!(type_by_graphic(400, 2), 2);
    }

    #[test]
    fn block_offset_by_kind() {
        // Section base by kind, plus group*5 + dir.
        assert_eq!(Anim::block(100, 0, 0, 0), Some(100 * 110)); // monster/high
        assert_eq!(Anim::block(250, 0, 0, 1), Some((250 - 200) * 65 + 22000)); // animal/low
        assert_eq!(Anim::block(400, 0, 0, 2), Some(35000)); // people
        assert_eq!(Anim::block(100, 4, 3, 0), Some(100 * 110 + 4 * 5 + 3));
    }

    #[test]
    fn mob_types_parses_and_applies_offset_flags() {
        let def = "\
# id  type  flags
5\tANIMAL\t2A
200\tMONSTER\t0
400\tHUMAN\t0
9\tMONSTER\t1008
50\tMONSTER\t440    # ByPeopleGroup (0x400) → people
60\tMONSTER\t48     # ByLowGroup (0x40) → animal
70\tSEA_MONSTER\t0
not a data line
";
        let map = parse_mob_types(def);
        let kind = |id: u16| map.get(&id).map(|e| e.kind);
        assert_eq!(kind(5), Some(1)); // animal
        assert_eq!(kind(200), Some(0)); // monster (range would say animal — mobtypes wins)
        assert_eq!(kind(400), Some(2)); // human → people
        assert_eq!(kind(9), Some(0)); // monster, unrelated flags
        assert_eq!(kind(50), Some(2)); // monster + 0x400 → people
        assert_eq!(kind(60), Some(1)); // monster + 0x40 → animal
        assert_eq!(kind(70), Some(0)); // sea_monster → high/monster
        // Only body 400's TYPE column is literally `equipment`... except none of
        // these are — spot check that an ordinary HUMAN/MONSTER line is not
        // mistaken for one, and that none of them set the UOP flag (all flags
        // here are far below 0x10000).
        assert!(!map.get(&400).unwrap().equipment);
        assert!(map.values().all(|e| !e.uop));
    }

    #[test]
    fn mob_types_parses_equipment_type_and_uop_flag() {
        let def = "\
10\tEQUIPMENT\t0
11\tMONSTER\t10000    # UseUopAnimation (0x10000) bit set
12\tMONSTER\t10400    # UseUopAnimation (0x10000) + ByPeopleGroup (0x400), both set
";
        let map = parse_mob_types(def);
        // `equipment` TYPE → people kind (2) for group semantics, but flagged
        // `equipment` for the UOP min-10 rule.
        let e10 = map.get(&10).unwrap();
        assert_eq!(e10.kind, 2);
        assert!(e10.equipment);
        assert!(!e10.uop);
        // A MONSTER line with the UOP bit set: kind unaffected, `equipment` false.
        let e11 = map.get(&11).unwrap();
        assert_eq!(e11.kind, 0);
        assert!(!e11.equipment);
        assert!(e11.uop);
        // The UOP bit and an offset-override bit can be set at once: kind is
        // still overridden to people (2) by 0x400, independent of the UOP flag.
        let e12 = map.get(&12).unwrap();
        assert_eq!(e12.kind, 2);
        assert!(e12.uop);
        assert!(!e12.equipment);
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real Body.def + anim.mul)
    fn body_def_remap_adds_real_coverage() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        assert!(!anim.bodydef.is_empty(), "Body.def should have loaded entries");
        // For every remap, the *target* body's stand frame should resolve, and count
        // how many exotic bodies gain a sprite they lacked at their original id.
        let mut gained = 0;
        for (&orig, &(target, _)) in &anim.bodydef {
            if orig == target {
                continue;
            }
            let orig_ok = anim.frame_count(orig, anim.stand_group(orig), 4).unwrap_or(0) > 0;
            let target_ok = anim.frame_count(target, anim.stand_group(target), 4).unwrap_or(0) > 0;
            if !orig_ok && target_ok {
                gained += 1;
            }
        }
        println!("Body.def: {} entries, {} bodies gained a sprite via remap", anim.bodydef.len(), gained);
        assert!(gained > 0, "remap should resolve sprites that the raw body id could not");
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real Corpse.def + anim.mul)
    fn corpse_def_remap_and_death_group_resolve_real_frames() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        assert!(!anim.corpsedef.is_empty(), "Corpse.def should have loaded entries");
        // For every remap, the death-pose frame of the *target* body should resolve
        // (facing south, dir 4) — the same sprite the renderer will draw for a
        // corpse of that creature.
        let mut resolved = 0;
        for &(target, _) in anim.corpsedef.values() {
            let dg = anim.death_group(target);
            if anim.frame_count(target, dg, 4).unwrap_or(0) > 0 {
                resolved += 1;
            }
        }
        println!("Corpse.def: {} entries, {} death poses resolved", anim.corpsedef.len(), resolved);
        assert!(resolved > 0, "death_group should resolve real death-pose frames via Corpse.def");
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real mobtypes.txt)
    fn mob_types_overrides_range_heuristic() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        assert!(!anim.mobtypes.is_empty(), "mobtypes.txt should have loaded entries");
        let mut overridden = 0;
        for (&body, e) in &anim.mobtypes {
            let range = if body < 200 {
                0
            } else if body < 400 {
                1
            } else {
                2
            };
            if e.kind != range {
                overridden += 1;
            }
        }
        println!("mobtypes: {} entries, {overridden} override the range heuristic", anim.mobtypes.len());
        assert!(overridden > 0, "mobtypes should correct some bodies the range heuristic gets wrong");
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real Bodyconv.def + anim2..anim5.mul)
    fn body_conv_resolves_expansion_sprites() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");
        assert!(!anim.bodyconv.is_empty(), "Bodyconv.def should have loaded entries");
        let installed = anim.files.iter().skip(1).filter(|f| f.is_some()).count();
        println!("bodyconv: {} entries, {} expansion files (anim2..anim5) installed", anim.bodyconv.len(), installed);
        // Count bodies whose stand frame resolves ONLY through a bodyconv redirect
        // into an expansion file (the raw id has no entry in the base anim.mul).
        let mut via_conv = 0;
        for (&body, &(fi, _)) in &anim.bodyconv {
            if fi == 0 || anim.files.get(fi as usize).and_then(Option::as_ref).is_none() {
                continue;
            }
            let (rfi, _, _) = match anim.entry(body, anim.stand_group(body), 4) {
                Some(e) => e,
                None => continue,
            };
            if rfi != 0 && anim.frame_count(body, anim.stand_group(body), 4).unwrap_or(0) > 0 {
                via_conv += 1;
            }
        }
        println!("bodyconv: {via_conv} bodies render from an expansion file");
        if installed > 0 {
            assert!(via_conv > 0, "expansion redirects should resolve real sprites");
        }
    }

    /// Build a synthetic `.bin` payload with a GAP in frameIds and a second
    /// direction: direction 0 = frames [1 (real), 2 (gap)], direction 1 =
    /// frames [3 (gap), 4 (real)], plus one more raw record at frameId 10
    /// (direction 4) purely to push `maxFrameCount` to 10 so `realFrameCount`
    /// rounds to a clean 2. Returns `(buf, [record_start; 3])`.
    fn build_uop_bin() -> (Vec<u8>, [usize; 3]) {
        let mut buf = vec![0u8; 32]; // header (ClassicUO skips this — unused)
        buf.extend_from_slice(&3i32.to_le_bytes()); // frameCount: 3 STORED records
        buf.extend_from_slice(&40u32.to_le_bytes()); // dataStart
        assert_eq!(buf.len(), 40);

        let rec_starts = [40usize, 56, 72];
        buf.resize(88, 0); // reserve the 3×16-byte record table

        let put_u16 = |b: &mut [u8], o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_le_bytes());
        let put_u32 = |b: &mut [u8], o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put_u16(&mut buf, rec_starts[0] + 2, 1); // frameId=1 → dir0/idx0 (real)
        put_u16(&mut buf, rec_starts[1] + 2, 4); // frameId=4 → dir1/idx1 (real)
        put_u16(&mut buf, rec_starts[2] + 2, 10); // frameId=10 → forces maxFrameCount=10

        // Payload 0: 2×2 sprite, center (0,-2), palette[1] = white, both top-row
        // pixels opaque (run=2, x=0,y=0).
        let pa0 = buf.len();
        buf.extend(std::iter::repeat_n(0u8, 512));
        put_u16(&mut buf, pa0 + 2, 0x7FFF); // palette[1] = white
        buf.extend_from_slice(&0i16.to_le_bytes()); // center_x
        buf.extend_from_slice(&(-2i16).to_le_bytes()); // center_y
        buf.extend_from_slice(&2i16.to_le_bytes()); // width
        buf.extend_from_slice(&2i16.to_le_bytes()); // height
        buf.extend_from_slice(&2u32.to_le_bytes()); // RLE header: run=2, x=0, y=0
        buf.push(1);
        buf.push(1); // both pixels → palette[1]
        buf.extend_from_slice(&0x7FFF_7FFFu32.to_le_bytes()); // terminator
        put_u32(&mut buf, rec_starts[0] + 12, (pa0 - rec_starts[0]) as u32);

        // Payload 1: 1×1 sprite, center (0,-1), palette[2] = green, one opaque pixel.
        let pa1 = buf.len();
        buf.extend(std::iter::repeat_n(0u8, 512));
        put_u16(&mut buf, pa1 + 4, 0x03E0); // palette[2] = green
        buf.extend_from_slice(&0i16.to_le_bytes());
        buf.extend_from_slice(&(-1i16).to_le_bytes());
        buf.extend_from_slice(&1i16.to_le_bytes());
        buf.extend_from_slice(&1i16.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes()); // run=1, x=0, y=0
        buf.push(2);
        buf.extend_from_slice(&0x7FFF_7FFFu32.to_le_bytes());
        put_u32(&mut buf, rec_starts[1] + 12, (pa1 - rec_starts[1]) as u32);

        // Payload 2: dummy — never decoded in these tests, width=0 so a decode
        // attempt would bail out cleanly instead of reading garbage.
        let pa2 = buf.len();
        buf.extend(std::iter::repeat_n(0u8, 512 + 8));
        put_u32(&mut buf, rec_starts[2] + 12, (pa2 - rec_starts[2]) as u32);

        (buf, rec_starts)
    }

    #[test]
    fn parse_uop_bin_gap_fills_slices_directions_and_decodes_pixels() {
        let (buf, _) = build_uop_bin();

        let bin = parse_uop_bin(&buf, false).expect("parse");
        assert_eq!(bin.real_frame_count, 2); // round(10 / 5) = 2

        // Direction 0: idx0 real (frameId 1), idx1 gap-filled empty (frameId 2).
        let s00 = bin.slot(0, 0).expect("dir0/idx0 present");
        assert!(!s00.empty);
        let s01 = bin.slot(0, 1).expect("dir0/idx1 present (gap-filled)");
        assert!(s01.empty);

        // Direction 1: idx0 gap-filled empty (frameId 3), idx1 real (frameId 4).
        let s10 = bin.slot(1, 0).expect("dir1/idx0 present (gap-filled)");
        assert!(s10.empty);
        let s11 = bin.slot(1, 1).expect("dir1/idx1 present");
        assert!(!s11.empty);
        // Out of range for this real_frame_count.
        assert!(bin.slot(0, 2).is_none());

        // Decode direction 0's real frame: 2×2, center (0,-2), top row opaque white.
        let p0 = s00.start + s00.pixel_offset as usize;
        let mut pal0 = [0u16; 256];
        for (i, c) in pal0.iter_mut().enumerate() {
            *c = u16le(&buf, p0 + i * 2);
        }
        let (img0, cx0, cy0) = decode_sprite_frame(&buf, p0 + 512, &pal0, true).expect("decode dir0/idx0");
        assert_eq!((img0.width, img0.height), (2, 2));
        assert_eq!((cx0, cy0), (0, -2));
        assert_eq!(&img0.rgba[0..4], &[255, 255, 255, 255]);
        assert_eq!(&img0.rgba[4..8], &[255, 255, 255, 255]);
        // Bottom row never written — stays fully transparent.
        assert_eq!(&img0.rgba[8..16], &[0, 0, 0, 0, 0, 0, 0, 0]);

        // Mirroring flips both the image and the draw-center's X.
        let (mirrored, mcx, mcy) = apply_mirror(img0, cx0, cy0, true);
        assert_eq!((mcx, mcy), (2, -2)); // width(2) - center_x(0)
        assert_eq!((mirrored.width, mirrored.height), (2, 2));

        // Decode direction 1's real frame too: 1×1, center (0,-1), opaque green.
        let p1 = s11.start + s11.pixel_offset as usize;
        let mut pal1 = [0u16; 256];
        for (i, c) in pal1.iter_mut().enumerate() {
            *c = u16le(&buf, p1 + i * 2);
        }
        let (img1, cx1, cy1) = decode_sprite_frame(&buf, p1 + 512, &pal1, true).expect("decode dir1/idx1");
        assert_eq!((img1.width, img1.height), (1, 1));
        assert_eq!((cx1, cy1), (0, -1));
        assert_eq!(&img1.rgba[0..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn parse_uop_bin_equipment_forces_minimum_ten_frames() {
        // A single stored record at frameId=6 → maxFrameCount=6 after gap-fill
        // (frames 1..5 gap-filled, 6 real). round(6/5) = 1 normally, but
        // Equipment-type bodies floor at 10 (ClassicUO: "min amount of frames
        // is 10 for equipment").
        let mut buf = vec![0u8; 56];
        buf[32..36].copy_from_slice(&1i32.to_le_bytes()); // frameCount = 1 stored record
        buf[36..40].copy_from_slice(&40u32.to_le_bytes()); // dataStart
        buf[42..44].copy_from_slice(&6u16.to_le_bytes()); // frameId = 6

        let normal = parse_uop_bin(&buf, false).expect("non-equipment parse");
        assert_eq!(normal.real_frame_count, 1);

        let equipment = parse_uop_bin(&buf, true).expect("equipment parse");
        assert_eq!(equipment.real_frame_count, 10);
    }

    #[test]
    fn parse_anim_sequence_entry_marks_zero_frame_count_as_replaced() {
        let mut buf = vec![0u8; 4 + 48]; // animId + 48 unknown bytes
        buf[0..4].copy_from_slice(&777u32.to_le_bytes());
        buf.extend_from_slice(&2i32.to_le_bytes()); // replaceCount = 2

        // record 0: oldGroup=3, frameCount=0 → replaced by newGroup=9.
        buf.extend_from_slice(&3i32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&9i32.to_le_bytes());
        buf.extend(std::iter::repeat_n(0u8, 60));

        // record 1: oldGroup=5, frameCount=4 (nonzero) → NOT replaced.
        buf.extend_from_slice(&5i32.to_le_bytes());
        buf.extend_from_slice(&4u32.to_le_bytes());
        buf.extend_from_slice(&99i32.to_le_bytes());
        buf.extend(std::iter::repeat_n(0u8, 60));

        let (body, reps) = parse_anim_sequence_entry(&buf).expect("parse");
        assert_eq!(body, 777);
        assert_eq!(reps, vec![(3u8, 9i32)]);
    }

    #[test]
    fn parse_anim_sequence_entry_skips_whole_list_for_guarded_replace_counts() {
        // ClassicUO (and we) skip the replace list entirely when replaceCount
        // is 48 or 68 — real bodies (400 human, 666 gargoyle, 1253) whose
        // extra trailing fields the fixed 60-byte skip can't handle, so they
        // deliberately stay at identity instead of being misdecoded.
        let mut buf = vec![0u8; 4 + 48];
        buf[0..4].copy_from_slice(&400u32.to_le_bytes());
        buf.extend_from_slice(&48i32.to_le_bytes()); // guarded replaceCount

        // A well-formed, definitely-replaced record right after — must still
        // be ignored because the guard skips the whole list.
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&1i32.to_le_bytes());
        buf.extend(std::iter::repeat_n(0u8, 60));

        let (body, reps) = parse_anim_sequence_entry(&buf).expect("parse");
        assert_eq!(body, 400);
        assert!(reps.is_empty());
    }

    /// One `parse_anim_sequence_entry`-shaped payload: `animId` + 48 unknown
    /// bytes + `replaceCount` + that many `{oldGroup, frameCount=0, newGroup,
    /// skip 60}` records (all "replaced", per
    /// `parse_anim_sequence_entry_marks_zero_frame_count_as_replaced` above).
    fn sequence_entry_payload(anim_id: u32, reps: &[(i32, i32)]) -> Vec<u8> {
        let mut buf = vec![0u8; 4 + 48];
        buf[0..4].copy_from_slice(&anim_id.to_le_bytes());
        buf.extend_from_slice(&(reps.len() as i32).to_le_bytes());
        for &(old_group, new_group) in reps {
            buf.extend_from_slice(&old_group.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes()); // frameCount = 0 → replaced
            buf.extend_from_slice(&new_group.to_le_bytes());
            buf.extend(std::iter::repeat_n(0u8, 60));
        }
        buf
    }

    /// Build a synthetic UOP container (same block layout as
    /// `uop::tests::build_single_entry_uop`, generalized to N entries in one
    /// directory block, in the given FILE order) holding raw (uncompressed)
    /// payloads — used to exercise `parse_anim_sequence` against a real
    /// `UopReader`/`all_entries` file-order walk, not just
    /// `parse_anim_sequence_entry` in isolation.
    fn build_multi_entry_uop(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = vec![0u8; 20]; // magic(4) + version(4) + timestamp(4) + next_block(8)
        buf[0..4].copy_from_slice(&0x0050_594Du32.to_le_bytes());
        buf[12..20].copy_from_slice(&20i64.to_le_bytes()); // first (only) block starts right after

        // Block header: count=N, next_block=0 (end of chain).
        buf.extend_from_slice(&(entries.len() as i32).to_le_bytes());
        buf.extend_from_slice(&0i64.to_le_bytes());

        let table_start = buf.len();
        buf.resize(table_start + entries.len() * 34, 0); // reserve the N×34-byte record table

        let mut payload_off = buf.len();
        for (i, (path, payload)) in entries.iter().enumerate() {
            let o = table_start + i * 34;
            let hash = uop_hash(path);
            buf[o..o + 8].copy_from_slice(&(payload_off as i64).to_le_bytes()); // offset
            buf[o + 8..o + 12].copy_from_slice(&0u32.to_le_bytes()); // header_len
            buf[o + 12..o + 16].copy_from_slice(&(payload.len() as u32).to_le_bytes()); // compressed_size
            buf[o + 16..o + 20].copy_from_slice(&(payload.len() as u32).to_le_bytes()); // decompressed_size
            buf[o + 20..o + 28].copy_from_slice(&hash.to_le_bytes()); // file_hash
            buf[o + 28..o + 32].copy_from_slice(&0u32.to_le_bytes()); // data_hash (unused)
            buf[o + 32..o + 34].copy_from_slice(&0i16.to_le_bytes()); // compression flag: None
            payload_off += payload.len();
        }
        for (_, payload) in entries {
            buf.extend_from_slice(payload);
        }
        buf
    }

    #[test]
    fn parse_anim_sequence_last_file_order_entry_replaces_whole_body_table() {
        // Two AnimationSequence.uop entries for the SAME animID (777): the
        // FIRST replaces group 3 -> 9; the SECOND (later in file order)
        // replaces a DIFFERENT group (5 -> 20) and says nothing about group 3.
        // ClassicUO's `_uopInfos[animID] = uopInfo` means the second entry
        // replaces the body's WHOLE table: group 5 becomes 20, and group 3
        // must go back to identity (absent from the map), NOT stay merged
        // at 9 from the first entry.
        let first = sequence_entry_payload(777, &[(3, 9)]);
        let second = sequence_entry_payload(777, &[(5, 20)]);
        let data = build_multi_entry_uop(&[
            ("build/animationsequence/00000000.bin", &first),
            ("build/animationsequence/00000001.bin", &second),
        ]);

        let dir = std::env::temp_dir();
        let file_path = dir.join(format!("anima_seq_test_{}.uop", std::process::id()));
        std::fs::write(&file_path, &data).expect("write temp uop");
        let reader = UopReader::open(&file_path).expect("open");
        std::fs::remove_file(&file_path).ok();

        let map = parse_anim_sequence(&reader);
        assert_eq!(map.get(&(777, 5)), Some(&20));
        assert_eq!(map.get(&(777, 3)), None, "earlier entry's group must NOT survive a later whole-table replace");
    }

    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource (real AnimationFrame*.uop + AnimationSequence.uop + mobtypes.txt)
    fn uop_animation_real_data_resolves_frames() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let anim = Anim::open(&dir).expect("open anim");

        let uop_bodies: Vec<u16> =
            anim.mobtypes.iter().filter(|(_, e)| e.uop).map(|(&body, _)| body).collect();
        println!("mobtypes: {} bodies flagged UseUopAnimation", uop_bodies.len());
        assert!(uop_bodies.len() > 100);

        let mut resolved = 0;
        let mut sample: Option<(u16, Image)> = None;
        for &body in &uop_bodies {
            let group = anim.stand_group(body);
            if let Some((img, _cx, _cy)) = anim.frame(body, group, 4, 0) {
                resolved += 1;
                if sample.is_none() {
                    sample = Some((body, img));
                }
            }
        }
        println!("{resolved} of {} UOP-flagged bodies resolve a stand-group frame via the UOP path", uop_bodies.len());
        assert!(resolved > 100);

        let (sample_body, img) = sample.expect("at least one UOP body should decode a frame");
        println!("sample UOP body {sample_body}: {}x{}", img.width, img.height);
        assert!(img.width > 0 && img.width < 1024);
        assert!(img.height > 0 && img.height < 1024);

        let human_is_uop = anim.mobtypes.get(&400).is_some_and(|e| e.uop);
        let human_resolves = anim.frame(400, anim.stand_group(400), 4, 0).is_some();
        println!("body 400 (human): UOP-flagged={human_is_uop}, resolves a stand frame either way={human_resolves}");
        assert!(human_resolves, "human body should resolve a stand frame via either the UOP or legacy path");

        // FIX 1 regression check: body 1401 (Turanchula_Mount) is one of the 9
        // real bodies whose `AnimationSequence.uop` replace chain only
        // resolves through the DOUBLE apply (`bin(replaced[replaced[G]])`) —
        // `uop_action` applying the table once left every one of its groups
        // (including walk=0 and its own stand_group) with no `.bin` match at
        // all. Both must now resolve via the UOP path.
        assert!(anim.mobtypes.get(&1401).is_some_and(|e| e.uop), "body 1401 should be UOP-flagged");
        let walk = anim.frame(1401, PEOPLE_WALK, 4, 0);
        assert!(walk.is_some(), "body 1401 walk (group 0) should resolve via the double-replace UOP path");
        let stand = anim.frame(1401, anim.stand_group(1401), 4, 0);
        assert!(stand.is_some(), "body 1401 stand_group should resolve via the double-replace UOP path");

        // FIX 2 regression check: body 826 (Stygian Dragon) group 19 has
        // frames well past the old 512px legacy cap (up to 523x407 on this
        // direction) — every frame must now decode instead of silently
        // going missing mid-animation.
        assert!(anim.mobtypes.get(&826).is_some_and(|e| e.uop), "body 826 should be UOP-flagged");
        let count = anim.frame_count(826, 19, 4).expect("body 826 group 19 dir 4 should have a frame count");
        assert!(count > 0);
        for idx in 0..count {
            let (img, _cx, _cy) = anim
                .frame(826, 19, 4, idx)
                .unwrap_or_else(|| panic!("body 826 group 19 dir 4 frame {idx}/{count} should decode"));
            assert!(img.width > 0 && img.width <= 4096, "sane width for frame {idx}: {}", img.width);
            assert!(img.height > 0 && img.height <= 4096, "sane height for frame {idx}: {}", img.height);
        }
    }
}

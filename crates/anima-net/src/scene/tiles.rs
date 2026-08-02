//! Tile-flag lookups and the per-tile art/animation suffixes.
//!
//! The `TileFlag` bits this renderer actually consults, plus the small emitters
//! that turn a graphic into what the browser needs to draw it — hue (including
//! `PartialHue`), the `animdata.mul` frame cycle, and the pathfinding bits the
//! browser's own `calculate_new_z` port reads back.

use super::*;

/// Resolve a layer-25 mount item's *graphic* to the animal body to draw under the
/// rider. UO mounts map item graphic → creature body via a fixed table; the
/// item's own tiledata AnimID is a tiny equipment overlay, not the mount, so the
/// table wins. Falls back to the tiledata AnimID for anything not in the table.
pub(super) fn mount_anim_for(graphic: u16, item_anim: &impl Fn(u16) -> u16) -> u16 {
    match anima_assets::mounts::mount_body(graphic) {
        Some((body, _off)) => body,
        None => item_anim(graphic),
    }
}

/// Paperdoll gender-gump offsets (ClassicUO `Constants.MALE_GUMP_OFFSET` /
/// `FEMALE_GUMP_OFFSET`): a worn item's paperdoll art lives at `animID + offset`,
/// one offset per gender.
pub(super) const MALE_GUMP_OFFSET: u32 = 50_000;

pub(super) const FEMALE_GUMP_OFFSET: u32 = 60_000;

/// Turn an `Equipconv.def` gump column ([`anima_assets::EquipConv::gump`], already
/// 0/-1-substituted by the parser) into an absolute paperdoll gump id for
/// `wearer_body`. Mirrors ClassicUO `PaperDollInteractable.GetAnimID`: a value
/// above [`MALE_GUMP_OFFSET`] is already a baked gump id for SOME gender — strip
/// whichever offset it carries and re-add the offset for the wearer's ACTUAL
/// gender; a bare graphic id (below the offset) just gets that offset added.
/// UO's female people bodies are exactly 401/403 (human), 606 (elf), 667
/// (gargoyle) — 606 is even, so this is NOT a parity test (ClassicUO
/// `Mobile.CheckGraphicChange`).
pub(super) fn equip_conv_gump(wearer_body: u16, gump: u16) -> u16 {
    let gump = gump as u32;
    let base = if gump > MALE_GUMP_OFFSET {
        if gump >= FEMALE_GUMP_OFFSET {
            gump - FEMALE_GUMP_OFFSET
        } else {
            gump - MALE_GUMP_OFFSET
        }
    } else {
        gump
    };
    let female = matches!(wearer_body, 401 | 403 | 606 | 667);
    let offset = if female {
        FEMALE_GUMP_OFFSET
    } else {
        MALE_GUMP_OFFSET
    };
    (base + offset) as u16
}

/// Half-size of the square map window emitted around the player. A bit larger
/// than the visible area so new tiles are created off-screen (no edge pop-in).
/// Covers a ~1600px-wide viewport (`88*RADIUS` px). 18 keeps the rendered sprite
/// count (~1500 vs ~2800 at 22) low enough to not peg the CPU; the renderer reads
/// the radius from the scene so it adapts automatically.
pub const RADIUS: i64 = 18;

/// Half-size of the LAND-only window emitted beyond `RADIUS`, rendered by the
/// client as a desaturated flat ring for spatial context (no statics/objects
/// out there) — the "grayed-out terrain past your view range" ClassicUO look.
/// Must stay bigger than `RADIUS`; land tiles out to here are cheap (no
/// per-tile static fetch), so this can be pushed further than the view range
/// without the cost `RADIUS` itself is capped by (see doc above).
pub const LAND_RADIUS: i64 = 24;

/// Statics/land within this Chebyshev distance of the player also carry the
/// tiledata bits the browser needs to run `calculate_new_z` itself (see
/// web/main.js). Bounded on purpose: prediction only ever steps a few tiles
/// ahead, and shipping these for the whole window would bloat every poll.
pub const PATH_RADIUS: i64 = 10;

/// Static tiledata flag bits we need for roof/floor hiding (see [`max_draw_z`])
/// and step-Z resolution (see [`calculate_new_z`]).
pub(super) const FLAG_IMPASSABLE: u64 = 0x40;

pub(super) const FLAG_SURFACE: u64 = 0x200;

pub(super) const FLAG_BRIDGE: u64 = 0x400;

pub(super) const FLAG_ROOF: u64 = 0x1000_0000;

/// Foliage flag (trees/bushes): the renderer fades these when they'd hide the
/// player, like ClassicUO's foliage transparency.
pub(super) const FLAG_FOLIAGE: u64 = 0x2_0000;

/// Stackable flag (`TileFlag.Generic`, ClassicUO `ItemData.IsStackable`): drives
/// whether a dragged stack (amount > 1) offers the split-stack dialog, mirroring
/// ClassicUO `GameActions.PickUp` (`item.Amount > 1 && item.ItemData.IsStackable`).
pub(super) const FLAG_STACKABLE: u64 = 0x800;

/// Partial-hue flag (`TileFlag.PartialHue` = 0x0004_0000, ClassicUO
/// TileDataLoader.cs; `ItemData.IsPartialHue`): only the item art's GRAY pixels
/// take the dye. See [`item_art_hue`].
pub(super) const FLAG_PARTIAL_HUE: u64 = 0x4_0000;

/// The hue to ship for an ITEM's art, in the packet form the play server's
/// `?hue=` — and so `anima_assets::apply_hue` — reads: bit `0x8000` means
/// *partial* hue, i.e. recolor only the gray pixels so a dyed hatchet tints its
/// metal head and keeps its wooden handle. ClassicUO takes that bit from
/// tiledata per item (`ItemView.DrawInternal`: `bool partial =
/// ItemData.IsPartialHue`) and folds it into the very same `0x8000` bit inside
/// `ShaderHueTranslator.GetHueVector`, so encoding it here lets every consumer
/// of the scene (world sprite, container grid, vendor row, drag ghost) just pass
/// the value straight through as `?hue=`. An unhued item stays 0 = "no hue": the
/// flag alone must never turn into a hue request.
pub(super) fn item_art_hue(hue: u16, flags: u64) -> u16 {
    if hue == 0 {
        return 0;
    }
    if flags & FLAG_PARTIAL_HUE != 0 {
        hue | 0x8000
    } else {
        hue
    }
}

/// Per-frame interval (ms) for an animated static, from animdata's `frameInterval`
/// tick count. The raw value is a small tick count (often 0–3); we scale it into a
/// lively-but-not-frantic range so flames flicker and fountains/wheels turn at a
/// believable pace (mirrors the effects path, which scales interval ×50ms).
pub(super) fn anim_interval_ms(interval: u8) -> u32 {
    ((interval as u32).max(1) * 100).clamp(100, 1000)
}

/// The `,"a":[frame,frame,...],"ai":N` JSON suffix for an animated static/multi
/// component graphic (flames/fountains/water wheels — `TileFlag.Animation`,
/// frame sequence from `animdata.mul`), or `""` when the graphic isn't
/// animated / animdata gives fewer than 2 frames. Shared by the real-statics
/// render loop and the multi-component one (FIX 6) so an animated component
/// (mill wheel, pennant) cycles frames exactly like the identical graphic
/// would as a real static, instead of freezing on frame 0. Pure given
/// `map`/`animdata`, so this is unit-testable against real tiledata/animdata
/// without a live `Session` (see the `#[ignore]`d test below).
pub(super) fn anim_suffix(map: &MapData, animdata: Option<&AnimData>, graphic: u16) -> String {
    let mut anim = String::new();
    if let Some((seq, ai)) = anim_frames(map, animdata, graphic) {
        anim.push_str(",\"a\":[");
        for (i, g) in seq.iter().enumerate() {
            if i > 0 {
                anim.push(',');
            }
            let _ = write!(anim, "{g}");
        }
        let _ = write!(anim, "],\"ai\":{ai}");
    }
    anim
}

/// The `(frame sequence, per-frame interval ms)` behind [`anim_suffix`], or
/// `None` when the graphic isn't animated / animdata gives fewer than 2 frames.
/// Split out for the same reason [`path_bits`] was split out of [`path_suffix`]:
/// the dynamic-items loop in [`build_scene`] builds a `serde_json::Value`, not a
/// hand-written JSON string, and must emit the same `a`/`ai` a static with that
/// graphic gets — otherwise a server-spawned campfire or spell field freezes on
/// frame 0 while an identical static flickers.
pub(super) fn anim_frames(
    map: &MapData,
    animdata: Option<&AnimData>,
    graphic: u16,
) -> Option<(Vec<u16>, u32)> {
    if !map.item_is_animated(graphic) {
        return None;
    }
    let ad = animdata?;
    let seq = ad.frame_sequence(graphic);
    (seq.len() > 1).then(|| (seq, anim_interval_ms(ad.frames(graphic).1)))
}

/// The `h`/`pf` VALUES a graphic within [`PATH_RADIUS`] of the player would
/// carry: `h` is the tiledata HEIGHT, `pf` packs the three flag bits the
/// browser's `calculate_new_z` port needs (bit 0 impassable, bit 1 surface,
/// bit 2 bridge). Each is `None` when out-of-radius or zero. Split out from
/// [`path_suffix`] (which formats these straight into a hand-written JSON
/// string, for the real-statics/multi-component loops) so the dynamic-items
/// loop in [`build_scene`] — which builds a `serde_json::Value`, not a raw
/// string — can share the exact same bits instead of re-deriving them.
pub(super) fn path_bits(in_radius: bool, height: u8, flags: u64) -> (Option<u8>, Option<u8>) {
    if !in_radius {
        return (None, None);
    }
    let h = (height != 0).then_some(height);
    let mut bits = 0u8;
    if flags & FLAG_IMPASSABLE != 0 {
        bits |= 1;
    }
    if flags & FLAG_SURFACE != 0 {
        bits |= 2;
    }
    if flags & FLAG_BRIDGE != 0 {
        bits |= 4;
    }
    (h, (bits != 0).then_some(bits))
}

/// Optional `h`/`pf` suffix a static's JSON entry gets when it's within
/// [`PATH_RADIUS`] of the player (see [`path_bits`] for the values). Each
/// field is omitted when zero, so out-of-radius (or flag/height-less) statics
/// serialize identically to before this existed. Shared by the real-statics
/// loop and `emit_multi_component` so the two paths can't drift.
pub(super) fn path_suffix(in_radius: bool, height: u8, flags: u64) -> String {
    let mut s = String::new();
    let (h, pf) = path_bits(in_radius, height, flags);
    if let Some(h) = h {
        let _ = write!(s, ",\"h\":{}", h as i32);
    }
    if let Some(pf) = pf {
        let _ = write!(s, ",\"pf\":{pf}");
    }
    s
}

/// Optional `,"dr":<serial>` land-tile suffix naming the closed door sealing
/// it — present only when the tile FAILS strict [`tile_walkable`] but PASSES
/// [`tile_walkable_for_planning`] because every impassable dynamic item on it
/// is that one openable door (the serial comes from
/// [`explain_tile_walkable_for_planning`], reusing its exact "every blocker
/// is a door" rule rather than re-deriving it here). `None` (empty string)
/// for every other tile — an ordinary wall, an already-open doorway, or a
/// doorway also blocked by something else (e.g. a dropped crate) — so a tile
/// without this serializes byte-identically to before this field existed.
/// ClassicUO walks a player INTO a closed door and opens it for them
/// (`PlayerMobile.TryOpenDoors`); without this, the browser's manual
/// (keyboard) walking has no way to learn a doorway is openable — the strict
/// `w` flag alone just reads as a wall, so a player could build a house and
/// then be unable to walk into it (see this field's origin bug).
pub(super) fn door_suffix(door: Option<u32>) -> String {
    match door {
        Some(serial) => format!(",\"dr\":{serial}"),
        None => String::new(),
    }
}

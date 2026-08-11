//! Placed multis: houses, boats, and the custom-house designer.
//!
//! Resolving a multi id into components (`multi.mul` + `MultiCollection.uop`),
//! the placement preview the server asks for, and the 0xD8 custom-house design
//! whose decoded tiles replace a foundation's stock components in both the scene
//! and the walkability fold.

use super::*;

/// Cap on the deduped `(dx, dy)` footprint tiles emitted for a pending 0x99
/// placement preview — a house's raw component list runs into the hundreds,
/// but the outline only needs each distinct offset once, so this keeps the
/// field a few KB instead of dumping every component/graphic/z.
pub(super) const PLACEMENT_TILE_CAP: usize = 4096;

/// Cap on the raw (non-deduped) `parts` component list emitted alongside
/// `tiles` (see [`placement_json`]) — unlike `tiles`, every component is kept
/// (walls/roof/floors all stack on the same footprint tile), so a castle's
/// full multi.mul list is the realistic worst case rather than the tile
/// count. 2000 comfortably covers that while still bounding the payload.
pub(super) const PLACEMENT_PART_CAP: usize = 2000;

/// The pending 0x99 house/multi placement footprint
/// (`World::pending_multi_placement`), as a `(dx, dy)`-deduped outline the
/// browser draws under the cursor while it's waiting for the click that
/// places the multi (the real client shows this; clicking blind is the bug
/// this exists to fix). `None` when there's nothing pending, `multis` wasn't
/// loaded (no `multi.mul` on disk), or the packet's multi id has no
/// component list — the caller omits the field entirely in that case, so an
/// idle/multis-less scene stays byte-identical to before this field existed.
///
/// Also carries `parts`: the SAME component list `tiles` is deduped from, but
/// left raw — `(dx, dy, dz, graphic)` per component, in multi.mul order — so
/// the browser can draw the actual house (walls/roof/doors), not just a flat
/// footprint outline, while the player is choosing where to place it. This
/// only ever runs while a placement target is pending (see the gating below),
/// so shipping every component (a house is typically 150-400 of them) is not
/// a steady-state cost — it never touches the normal per-tick scene build.
///
/// Gated on `pending_target` too, not just `pending_multi_placement`:
/// answering or cancelling the target cursor (`respond_target`/
/// `cancel_target` in `lib.rs`) clears `pending_target` the moment the reply
/// is sent, but that driver-side code can't also reach into
/// `pending_multi_placement` (a `net::game` concept). Requiring both here is
/// what keeps a stale footprint from lingering after the target it belonged
/// to is gone, without needing a third crate to touch `net::game::game.rs`.
pub(super) fn placement_json(world: &World, multis: Option<&Multis>) -> Option<Value> {
    world.pending_target?;
    let mp = world.pending_multi_placement?;
    let comps = multis?.components(mp.multi_id as u32)?;
    let mut seen: HashSet<(i16, i16)> = HashSet::new();
    let mut tiles: Vec<Value> = Vec::new();
    for c in comps {
        if tiles.len() >= PLACEMENT_TILE_CAP {
            break;
        }
        if seen.insert((c.dx, c.dy)) {
            tiles.push(json!([c.dx, c.dy]));
        }
    }
    // Same `comps`, same order (multi.mul's own listing — NOT sorted), just
    // without the (dx, dy) dedup: one entry per real component so the
    // browser can draw each one's actual art instead of a blank outline.
    let parts: Vec<Value> = comps
        .iter()
        .take(PLACEMENT_PART_CAP)
        .map(|c| json!([c.dx, c.dy, c.dz, c.graphic]))
        .collect();
    Some(json!({
        "multiId": mp.multi_id, "hue": mp.hue,
        "xOff": mp.x_off, "yOff": mp.y_off, "zOff": mp.z_off,
        "tiles": tiles,
        "parts": parts,
    }))
}

/// The story count a customizable foundation supports (ServUO
/// `HouseFoundation.MaxLevels`, `Scripts/Multis/HouseFoundation.cs`): 3
/// normally, 4 when the plot is 14+ tiles in either dimension. Folded from the
/// SAME `multi.mul` component bounds [`ensure_house_tiles`] uses to decode
/// design planes — just also tracking `max_x`, which decoding itself never
/// needs (only `min_x`/`min_y`/`max_y` feed [`crate::world::decode_house_planes`]).
/// Defaults to the conservative 3-story baseline when `multis` isn't loaded or
/// the foundation's component list can't be found.
pub(super) fn house_design_max_levels(multis: Option<&Multis>, multi_id: u32) -> u8 {
    let Some(comps) = multis.and_then(|m| m.components(multi_id)) else {
        return 3;
    };
    if comps.is_empty() {
        return 3;
    }
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (0i16, 0i16, 0i16, 0i16);
    for c in comps {
        min_x = min_x.min(c.dx);
        max_x = max_x.max(c.dx);
        min_y = min_y.min(c.dy);
        max_y = max_y.max(c.dy);
    }
    let plot = (max_x - min_x + 1).max(max_y - min_y + 1);
    if plot >= 14 {
        4
    } else {
        3
    }
}

/// The house-designer editor snapshot (`houseDesign` scene key), present only
/// while [`World::customizing_house`] is `Some` — omitted entirely otherwise
/// (same "no key, not a null" convention as [`placement_json`]) so an ordinary
/// scene stays byte-identical to before this field existed. Carries what a
/// browser editor needs to place pieces:
/// - `serial`/`revision`: the foundation and the design's revision counter
///   (from [`World::house_designs`]; `0` if the 0xD8 itself hasn't arrived
///   yet — 0x20 can beat that round-trip).
/// - `floors`: see [`house_design_max_levels`].
/// - `x`/`y`/`z`: the foundation's WORLD position. Every outgoing 0xD7
///   designer command (`build_house_design_add_item` etc.) takes coordinates
///   relative to the foundation's own multi center (ServUO
///   `HouseFoundation.Designer_Build`: `mcl.Add(itemID, x, y, z)` where `x`/`y`
///   are read straight off the wire), but the browser only knows which WORLD
///   tile got clicked — it needs this to compute `dx = worldX - x`,
///   `dy = worldY - y` itself.
///
/// Requires the foundation to already be a known [`World::items`] entry (the
/// only place its world position lives) — omits the whole field rather than
/// guessing a position if it isn't, which can only happen for a moment right
/// after entering design mode before the foundation's own item state has
/// arrived.
pub(super) fn house_design_json(world: &World, multis: Option<&Multis>) -> Option<Value> {
    let serial = world.customizing_house?;
    let item = world.items.get(&serial)?;
    let revision = world
        .house_designs
        .get(&serial)
        .map(|d| d.revision)
        .unwrap_or(0);
    let floors = house_design_max_levels(multis, item.graphic as u32);
    Some(json!({
        "serial": serial,
        "revision": revision,
        "floors": floors,
        "x": item.pos.x,
        "y": item.pos.y,
        "z": item.pos.z,
    }))
}

/// Decode any pending custom-house designs (0xD8) whose foundation item is
/// already in `world` and whose bounds we can now resolve. `anima-core` can't
/// do this itself — mode-2 grid planes need the foundation multi's `multi.mul`
/// bounds ([`Multis`] has no bounds API `anima-core` could depend on anyway,
/// keeping the near-zero-dep core free of asset-format knowledge). Folded over
/// ALL of the multi's components (including invisible ones), matching
/// ServUO's `MultiComponentList.Min/Max` and ClassicUO's `MultiInfo` — a
/// partial fold would misplace whole floors.
pub(super) fn ensure_house_tiles(world: &mut World, multis: &Multis) {
    let pending: Vec<(u32, u32)> = world
        .items
        .iter()
        .filter(|(serial, it)| {
            it.is_multi
                && world
                    .house_designs
                    .get(*serial)
                    .is_some_and(|d| !d.tiles_ready)
        })
        .map(|(serial, it)| (*serial, it.graphic as u32))
        .collect();
    for (serial, multi_id) in pending {
        let Some(comps) = multis.components(multi_id) else {
            continue;
        };
        if comps.is_empty() {
            continue; // nothing to bound
        }
        // Bounds start at the ORIGIN, not at extremes: both authorities implicitly
        // include (0,0) in the fold (ServUO `MultiComponentList` inits Min/Max to
        // Point2D.Zero; ClassicUO `Item.LoadMulti` inits to 0), and the packed
        // mode-2 grids were sized against those origin-clamped bounds.
        let (mut min_x, mut min_y, mut max_y) = (0i16, 0i16, 0i16);
        for c in comps {
            min_x = min_x.min(c.dx);
            min_y = min_y.min(c.dy);
            max_y = max_y.max(c.dy);
        }
        let d = world.house_designs.get_mut(&serial).unwrap();
        d.tiles = anima_core::world::decode_house_planes(&d.planes, min_x, min_y, max_y);
        d.tiles_ready = true;
    }
}

/// Emit ONE resolved multi/design tile — `graphic` at absolute `cz` on world
/// `(x, y)` — into the statics JSON stream, exactly like a real static:
/// nodraw skip, roof/`max_z` cull (so standing under cover hides it), the
/// same `pz` draw-sort rule (background sinks, height rises), the foliage
/// flag, the animdata frame sequence, and light-source accounting. Shared by
/// the multi.mul branch and the custom-house design branch below it so a
/// design tile can never drift from a real component's treatment — before
/// this helper existed the two were separate, hand-duplicated bodies.
/// `visible`/nodraw gating for design tiles happens in the CALLER (design
/// tiles have no `visible` flag — a graphic-0 entry was already dropped at
/// decode) — this only applies the nodraw *tiledata* check both share.
/// `px`/`py` are the player's position, used only to gate the [`PATH_RADIUS`]
/// `h`/`pf` suffix (see [`path_suffix`]) — everything else here is unrelated
/// to where the player stands.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_multi_component(
    map: &MapData,
    animdata: Option<&AnimData>,
    max_z: i32,
    under_cover: bool,
    x: i64,
    y: i64,
    graphic: u16,
    cz: i32,
    multi_serial: u32,
    statics: &mut String,
    n_statics: &mut usize,
    lights: &mut Vec<Value>,
    light_cap: usize,
    px: i64,
    py: i64,
) {
    if map.item_is_nodraw(graphic) {
        return;
    }
    // Fetched once and reused below (is_roof/background/height/foliage/path)
    // instead of re-querying tiledata per check.
    let flags = map.item_flags(graphic);
    let height = map.item_height(graphic);
    let is_roof = flags & FLAG_ROOF != 0;
    if cz >= max_z || (under_cover && is_roof) {
        return;
    }
    let mut spz = cz;
    if flags & 0x1 != 0 {
        spz -= 1; // Background
    }
    if height != 0 {
        spz += 1; // has height (wall/solid)
    }
    let foliage = if flags & FLAG_FOLIAGE != 0 {
        ",\"f\":1"
    } else {
        ""
    };
    // Same animdata frame-sequence lookup the real-statics loop does (via the
    // shared `anim_suffix`) — an animated component (mill wheel, pennant) or
    // design tile must cycle frames exactly like the identical graphic would
    // as a real static, not freeze on frame 0.
    let anim = anim_suffix(map, animdata, graphic);
    let path = path_suffix(
        (x - px).abs() <= PATH_RADIUS && (y - py).abs() <= PATH_RADIUS,
        height,
        flags,
    );
    let _ = write!(
        statics,
        "{{\"x\":{},\"y\":{},\"z\":{},\"g\":{},\"pz\":{},\"ms\":{}{}{}{}}},",
        x, y, cz, graphic, spz, multi_serial, foliage, anim, path
    );
    *n_statics += 1;
    if lights.len() < light_cap && map.item_is_light(graphic) {
        lights.push(json!({ "x": x, "y": y, "z": cz, "r": 3,
                            "id": map.item_light_id(graphic) }));
    }
}

//! Builds the renderer scene JSON from a live [`Session`] + map/art data.
//! Shared by the `scene` (AI patrol) and `play` (human-controlled) bins.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use anima_assets::{
    Anim, AnimData, Art, Cliloc, Image, MapData, Multis, RadarCol, StaticTile, ZReason, MAP_HEIGHT,
    MAP_WIDTH,
};
use anima_core::gump_layout::{self, GumpElement, HtmlText};
use anima_core::path::Terrain;
use anima_core::World;
use serde_json::{json, Value};

use crate::Session;

/// Resolve a layer-25 mount item's *graphic* to the animal body to draw under the
/// rider. UO mounts map item graphic → creature body via a fixed table; the
/// item's own tiledata AnimID is a tiny equipment overlay, not the mount, so the
/// table wins. Falls back to the tiledata AnimID for anything not in the table.
fn mount_anim_for(graphic: u16, item_anim: &impl Fn(u16) -> u16) -> u16 {
    match anima_assets::mounts::mount_body(graphic) {
        Some((body, _off)) => body,
        None => item_anim(graphic),
    }
}

/// Paperdoll gender-gump offsets (ClassicUO `Constants.MALE_GUMP_OFFSET` /
/// `FEMALE_GUMP_OFFSET`): a worn item's paperdoll art lives at `animID + offset`,
/// one offset per gender.
const MALE_GUMP_OFFSET: u32 = 50_000;
const FEMALE_GUMP_OFFSET: u32 = 60_000;

/// Turn an `Equipconv.def` gump column ([`anima_assets::EquipConv::gump`], already
/// 0/-1-substituted by the parser) into an absolute paperdoll gump id for
/// `wearer_body`. Mirrors ClassicUO `PaperDollInteractable.GetAnimID`: a value
/// above [`MALE_GUMP_OFFSET`] is already a baked gump id for SOME gender — strip
/// whichever offset it carries and re-add the offset for the wearer's ACTUAL
/// gender; a bare graphic id (below the offset) just gets that offset added.
/// UO's female people bodies are exactly 401/403 (human), 606 (elf), 667
/// (gargoyle) — 606 is even, so this is NOT a parity test (ClassicUO
/// `Mobile.CheckGraphicChange`).
fn equip_conv_gump(wearer_body: u16, gump: u16) -> u16 {
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

/// Why [`explain_tile_walkable`] (or [`can_step_to`] — the movement-gate
/// twin, scored via [`calculate_new_z`] instead of `walkable_z_explain`)
/// would allow/deny a step onto `(x, y)` from `current_z` — the exact same
/// checks, decomposed for `[pathdbg]` diagnostics. [`tile_walkable`] is now a
/// thin wrapper over `explain_tile_walkable` (`.is_ok()`), and [`step_ok`]/
/// [`can_walk`] are now a thin wrapper over `can_step_to` the same way, so
/// none of the four can ever disagree about what this enum names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepDeny {
    OffMap,
    Terrain(ZReason),
    DynamicItem { graphic: u16, item_z: i32 },
    /// [`can_step_to`]'s equivalent of "nothing to land on at all":
    /// [`calculate_new_z`] returned `None` — no candidate surface/bridge
    /// within the StepHeight climb limit, or the nearest one lacked
    /// headroom. Unlike [`StepDeny::Terrain`] (`explain_tile_walkable`'s
    /// `walkable_z_explain` scorer, which tells NoSurface/OutOfReach/Blocked
    /// apart), `calculate_new_z`'s single `None` doesn't discriminate those
    /// cases — this is deliberately its own flat variant rather than reusing
    /// (and thereby misreporting) a [`ZReason`] that implies detail
    /// `can_step_to` doesn't actually have.
    NoLanding,
}

/// Every multi component (boat hull/deck, house wall/floor — never the
/// multi's own `World::items` entry, which carries a multi id in `graphic`,
/// not an ART graphic) sitting at world tile `(x, y)`, as `(graphic,
/// absolute_z)` pairs. The ONE shared fold every multi-aware walkability check
/// (blocking, standing-surface contribution, step-Z resolution) builds on, so
/// they can never disagree about which components are even in play on a tile.
/// Resolved through each in-view multi's own tile-indexed component list
/// ([`Multis::components_at`]) instead of a direct `World::items` hit — a
/// component isn't its own `World::items` entry. Cheap even per A* node:
/// `World::items` only ever holds the handful of multis actually in view
/// (pruned by 0x1D like any item), and `components_at` is O(components on
/// this ONE tile) after the first call per multi id.
///
/// Includes an invisible component when it's the multi's index-0 record
/// (`MultiComponent::is_origin`) — ServUO's own collision/placement grid force
/// -includes index 0 regardless of its flags (`Server/MultiData.cs`
/// `MultiComponentList`, every constructor: `if (i == 0 ||
/// allTiles[i].m_Flags != 0)`), so client-side walkability prediction must
/// match that or risk disagreeing with a server-side deny/allow on the
/// origin tile. This is the WALKABILITY rule only — rendering (the tile loop
/// in [`build_scene`]) still checks `visible` on its own, since ClassicUO
/// only ever *draws* visible components.
fn multi_components_at(world: &World, multis: &Multis, x: i64, y: i64) -> Vec<(u16, i32)> {
    let mut out = Vec::new();
    for (serial, it) in world.items.iter().filter(|(_, it)| it.is_multi) {
        let (dx, dy) = (x - it.pos.x as i64, y - it.pos.y as i64);
        if dx < i16::MIN as i64
            || dx > i16::MAX as i64
            || dy < i16::MIN as i64
            || dy > i16::MAX as i64
        {
            continue; // absurdly far from this multi's origin — never one of its tiles
        }
        // A decoded custom-house design (0xD8) REPLACES the foundation's
        // multi.mul components entirely for THIS multi — it's the
        // authoritative piece list once ready (ServUO's own server-side
        // collision is the design, not the stock foundation shape), so once
        // `tiles_ready` this is the ONLY source, never a fallback/merge. Keyed
        // i8 (house footprints are tiny — 18x18 max), so a dx/dy outside i8
        // range is never a design tile either way. This is the WALKABILITY
        // fold — see `ensure_house_tiles`'s and this fn's own doc; the
        // rendering loop in `build_scene` performs the identical swap.
        if let Some(d) = world.house_designs.get(serial).filter(|d| d.tiles_ready) {
            if dx >= i8::MIN as i64
                && dx <= i8::MAX as i64
                && dy >= i8::MIN as i64
                && dy <= i8::MAX as i64
            {
                if let Some(tiles) = d.tiles.get(&(dx as i8, dy as i8)) {
                    for &(g, dz) in tiles {
                        out.push((g, it.pos.z as i32 + dz as i32));
                    }
                }
            }
            continue;
        }
        for c in multis.components_at(it.graphic as u32, dx as i16, dy as i16) {
            if c.visible || c.is_origin {
                out.push((c.graphic, it.pos.z as i32 + c.dz as i32));
            }
        }
    }
    out
}

/// [`multi_components_at`]'s tuples, reshaped into synthetic [`StaticTile`]s
/// so they can be folded straight into [`MapData::walkable_z_explain`]'s own
/// candidate scoring ([`crate::scene`]'s only consumer of `extra`) — a boat
/// deck then contributes a standing surface, and a hull wall genuinely
/// blocks, using the EXACT SAME impassable/surface/bridge rules a real static
/// gets (`StaticTile::impassable`/`::surface`), not a parallel ad hoc check.
fn multi_statics_at(
    world: &World,
    multis: &Multis,
    map: &MapData,
    x: i64,
    y: i64,
) -> Vec<StaticTile> {
    multi_components_at(world, multis, x, y)
        .into_iter()
        .map(|(graphic, z)| StaticTile {
            graphic,
            z: z.clamp(i8::MIN as i32, i8::MAX as i32) as i8,
            height: map.item_height(graphic),
            flags: map.item_flags(graphic),
        })
        .collect()
}

/// `multi_statics_at`'s counterpart for **ordinary dynamic items**: on this
/// shard a boat's deck arrives as perfectly ordinary [`World::items`]
/// entries, not as a multi at all — live-verified graphics
/// 0x3EA1/0x3EAC/0x3EB0 sitting at `z=-5` with tiledata `height` 3 and
/// `SURFACE` (not `Impassable`), giving a standing Z of `-5 + 3 = -2`, with
/// the hull/mast (0x3E9F/0x3EB1) as plain `Impassable` items right alongside,
/// and the gangplank (0x3EC0/0x3ED4) as `SURFACE|BRIDGE`. Before this existed,
/// `explain_tile_walkable`'s scoring only ever folded in **multi** components
/// (via [`multi_statics_at`]) — an ordinary item was only ever consulted as a
/// *blocker* (`MapData::item_blocks`), never as a surface contributor — so a
/// deck shaped like this was entirely invisible to `walkable_z_explain`, and
/// every step onto it read as unwalkable with `sz` falling back to the water
/// underneath. ClassicUO's own `Pathfinder.CreateItemList` has an explicit
/// `case Item item2:` arm alongside Land/Static/Multi for exactly this
/// reason.
///
/// Same shape as [`multi_statics_at`] — every matching item, unfiltered (a
/// hull piece becomes a synthetic `StaticTile` here too, same as a real
/// static would) — so a single call collects everything
/// [`explain_tile_walkable`] needs from `World::items`: it derives BOTH the
/// standing-surface fold AND the blocker check from this ONE scan instead of
/// two (see that function's doc for why the surface fold still has to filter
/// what it takes from here). Excludes multis (`is_multi` — `multi_statics_at`'s
/// job) and contained items (inventory, not standing on the tile). `z` is
/// copied straight from `Item::pos.z`, which is already the wire's `i8` —
/// unlike a multi component's synthesized `(origin_z + dz)` sum, there's no
/// offset arithmetic here that could overflow, so no clamp is needed.
fn dynamic_statics_at(world: &World, map: &MapData, x: i64, y: i64) -> Vec<StaticTile> {
    world
        .items
        .values()
        .filter(|it| {
            !it.is_multi
                && it.container.is_none()
                && it.pos.x as i64 == x
                && it.pos.y as i64 == y
        })
        .map(|it| StaticTile {
            graphic: it.graphic,
            z: it.pos.z,
            height: map.item_height(it.graphic),
            flags: map.item_flags(it.graphic),
        })
        .collect()
}

/// Does a **multi component** block a body at `current_z` stepping onto `(x,
/// y)`? A real interactive door is never baked into a multi's component list
/// (ServUO places it as its own separate door `Item`, e.g.
/// `BaseHouse.AddSouthDoor`), so no door exception is needed here — that
/// already flows through the ordinary dynamic-item path above.
fn multi_blocker_at(
    world: &World,
    multis: &Multis,
    map: &mut MapData,
    x: i64,
    y: i64,
    current_z: i32,
    ghost: bool,
) -> Option<(u16, i32)> {
    multi_components_at(world, multis, x, y)
        .into_iter()
        .find(|&(graphic, comp_z)| {
            map.item_blocks(graphic, comp_z, current_z) && !(ghost && map.item_is_door(graphic))
        })
}

/// The **blocker** half of [`explain_tile_walkable`]: does an impassable
/// dynamic item (crate, closed door — ghost exception applies) or multi
/// component (hull/house wall) deny a body from occupying `(x, y)` while
/// standing at `current_z`? Split out so [`build_scene`]'s per-tile `w` flag
/// can reuse these EXACT rules (never re-derive/duplicate them) while pairing
/// them with the authoritative `sz_chain` answer instead of
/// [`MapData::walkable_z_explain`]'s simpler scorer.
///
/// Why that pairing is needed — a real bug, diagnosed live: standing on a
/// house foundation's stairs, a body sits on a **bridge** (`0x0751`, height
/// 5, z 0 — a bridge's stand Z is `z + height/2`, giving stand z=2). The next
/// tile north carries an **impassable** riser (`0x0063`, z 0..5, the
/// foundation's edge) topped by the empty plot's own **surface**
/// (`0x31F4`, z 7). Stepping there is a +5 climb — exactly what a bridge is
/// for — and [`calculate_new_z`] (ClassicUO's `Pathfinder.CalculateNewZ`,
/// which its `CanWalk` uses directly) resolves it correctly to `sz=7`. But
/// `tile_walkable`, checked at the OLD `w=0`: `walkable_z_explain`'s scorer
/// doesn't model the bridge-widened max Z, AND (verified directly against
/// this exact riser/floor pair) THIS fn's own blocker rules deny it too —
/// `item_blocks`'s span test (`item_z < current_z + CHAR_HEIGHT`) genuinely
/// overlaps the riser's `0..5` span against a body still down at `current_z=2`
/// (the pre-step Z `tile_walkable` always checks against, matching ClassicUO
/// `CanWalk`/our own [`step_ok`]) — even though that exact riser is well clear
/// of a body actually AT the resolved z=7. So the fix is two-part: `sz_chain`
/// already carries the authoritative (`calculate_new_z`-derived) landing Z;
/// [`build_scene`] passes THAT resolved Z as `current_z` here instead of the
/// player's pre-step `pz` — asking "does this blocker overlap the body once
/// the step actually lands" instead of "…while it's still down where it
/// started" — which is what turns this exact case walkable without weakening
/// a single blocker rule (a closed door, judged at its OWN — usually
/// unchanged — tile, still denies; see the tests). `dyn_items` is passed in
/// rather than re-fetched so a caller that already ran
/// [`dynamic_statics_at`] (as [`explain_tile_walkable`] does, for its surface
/// fold) never scans `World::items` twice for the same tile.
fn blocking_item_at(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
    current_z: i32,
    dyn_items: &[StaticTile],
    ghost: bool,
) -> Option<StepDeny> {
    if let Some(it) = dyn_items.iter().find(|st| {
        map.item_blocks(st.graphic, st.z as i32, current_z) && !(ghost && map.item_is_door(st.graphic))
    }) {
        return Some(StepDeny::DynamicItem {
            graphic: it.graphic,
            item_z: it.z as i32,
        });
    }
    if let Some(multis) = multis {
        if let Some((graphic, item_z)) = multi_blocker_at(world, multis, map, x, y, current_z, ghost) {
            return Some(StepDeny::DynamicItem { graphic, item_z });
        }
    }
    None
}

/// Is tile (x, y) walkable for a body at `current_z`, and if so what Z would it
/// stand at? Combines the static map (land + statics, via
/// [`MapData::walkable_z_explain`]) — widened, when `multis` is given, with
/// any **in-view multi component** (boat deck/hull, house floor/wall) folded
/// in via [`multi_statics_at`] so a component can CONTRIBUTE a standing
/// surface (a boat deck) exactly like a real static, not just block one —
/// and widened again with any **ordinary dynamic item** that is itself a
/// genuine surface (see [`dynamic_statics_at`] — the real shape of a boat
/// deck on this shard) — with **dynamic world items** on top — an impassable
/// placed object (e.g. a crate) blocks too — and then, same as
/// [`can_walk`]/[`step_ok`]'s own two-layer shape, a final
/// [`multi_blocker_at`] pass for a multi component occupying this exact body
/// span.
///
/// The surface fold and the blocker check both come from ONE call to
/// [`dynamic_statics_at`] (a single `World::items` scan) instead of two
/// separate passes — this runs per tile of the ~49×49 scene window on every
/// build, so a second full scan here is real, measurable cost. Only a
/// synthetic tile that is a genuine surface/bridge and NOT impassable is
/// folded into the scoring `extra`: `walkable_z_explain`'s own
/// candidate-blocking loop tests every `extra` entry, impassable ones
/// included, so folding in an impassable item too (a closed door, a crate)
/// would let it silently reclassify an otherwise-fine land candidate as
/// `ZReason::Blocked` — stealing the denial from the dedicated
/// `StepDeny::DynamicItem` path below (with its ghost/door exception), which
/// already denies it correctly and which click-to-walk planning depends on
/// to tell "wall" from "door" apart. The blocker check itself still walks
/// every item [`dynamic_statics_at`] returned (unfiltered), so an impassable
/// item denies exactly as before.
pub fn explain_tile_walkable(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
    current_z: i32,
) -> Result<i32, StepDeny> {
    if x < 0 || y < 0 {
        return Err(StepDeny::OffMap);
    }
    let ghost = player_is_ghost(world);
    let dyn_items = dynamic_statics_at(world, map, x, y);
    let mut extra = multis
        .map(|m| multi_statics_at(world, m, map, x, y))
        .unwrap_or_default();
    extra.extend(
        dyn_items
            .iter()
            .filter(|st| st.surface() && !st.impassable())
            .copied(),
    );
    let z = map
        .walkable_z_explain(x as u32, y as u32, current_z, &extra)
        .map_err(StepDeny::Terrain)?;
    if let Some(deny) = blocking_item_at(world, map, multis, x, y, current_z, &dyn_items, ghost) {
        return Err(deny);
    }
    Ok(z)
}

/// Is tile (x, y) walkable for a body at `current_z`? Combines the static map
/// (land + statics, via [`MapData::walkable_z`]) with **dynamic world items** —
/// an impassable placed object (e.g. a crate) blocks too. Both the renderer's
/// `w` flag and the play-server's pacing use this so we never try to step into
/// an impassable object (it would just DenyWalk → snap back). Thin wrapper over
/// [`explain_tile_walkable`] so the two can never drift apart.
pub fn tile_walkable(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
    current_z: i32,
) -> bool {
    explain_tile_walkable(world, map, multis, x, y, current_z).is_ok()
}

/// Is tile (x, y) walkable for **click-to-walk route planning**, at
/// `current_z`? Like [`explain_tile_walkable`], except a closed door never
/// blocks: a closed door isn't a wall, it's a wall we're allowed to open, and
/// ClassicUO's own pathfinder treats it the same way (`Pathfinder.CanWalk`'s
/// `SmoothDoors`-style `dropFlags` for door items, plus its
/// `PlayerMobile.TryOpenDoors` auto-open-as-you-approach convenience). The
/// A* terrain adapter (`play_server::MapTerrain`) uses this so a route can be
/// planned *through* a closed door; the executor then really opens it (see
/// `play_server`'s auto-walk loop) before stepping onto its tile — so what
/// gets planned and what gets walked never disagree about the real world.
/// Manual walking (`can_walk`/`step_ok`, via [`can_step_to`]) and the debug
/// minimap overlay (`tile_walkable`, beyond `build_scene`'s `CHAIN_RADIUS`)
/// both keep this same strict door semantics: a closed door genuinely blocks
/// a single committed step until something has actually opened it. (Only the
/// SURFACE half of that strictness now differs between the two — `can_walk`/
/// `step_ok` score it via `calculate_new_z`, `tile_walkable` via
/// `walkable_z_explain` — see [`can_step_to`]'s doc for why.)
pub fn tile_walkable_for_planning(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
    current_z: i32,
) -> Option<i32> {
    explain_tile_walkable_for_planning(world, map, multis, x, y, current_z).0
}

/// [`tile_walkable_for_planning`]'s full answer: the resolved standing Z, PLUS
/// — when the tile is walkable-for-planning only BECAUSE every impassable
/// dynamic item on it is an openable closed door — that door's serial. Split
/// out the same way [`explain_tile_walkable`]/[`tile_walkable`] already are,
/// so a caller that needs to actually ACT on the door (not just know the tile
/// is plannable) — [`build_scene`]'s `dr` field, which names it for the
/// browser to auto-open before a manual step, mirroring ClassicUO's
/// `PlayerMobile.TryOpenDoors` — gets the serial from this EXACT "every
/// blocker is a door" walk instead of a second, possibly-diverging copy of
/// it. A `Some` serial only ever comes back alongside a `Some` Z; a tile
/// that's walkable outright (no door involved, e.g. an already-open doorway)
/// reports a Z with no serial.
fn explain_tile_walkable_for_planning(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
    current_z: i32,
) -> (Option<i32>, Option<u32>) {
    match explain_tile_walkable(world, map, multis, x, y, current_z) {
        Ok(z) => (Some(z), None),
        Err(StepDeny::DynamicItem { .. }) => {
            // `explain_tile_walkable`'s `.find()` only reports the FIRST
            // blocking dynamic item it happens to hit (`World::items` is a
            // `HashMap` — iteration order isn't the same as "the" blocker).
            // A door on the tile only makes it plannable-through if EVERY
            // impassable dynamic item there is a door — a crate someone
            // dropped in the same doorway must still deny, in either
            // find-order (see the FIX 4 regression test). Multi components are
            // never doors themselves (see `multi_blocker_at`'s doc), so any
            // multi blocker on this tile always fails the "every blocker is a
            // door" test — a wall/hull never becomes plannable-through.
            let ghost = player_is_ghost(world);
            // Remembered in passing as the `.all()` below walks every item
            // anyway — if every blocker turns out to be a door, this is one
            // of them (any one will do: see this fn's doc and the `dr`
            // field's doc for why a double-leaf doorway doesn't need a
            // specific leaf named).
            let mut a_door = None;
            let all_blockers_are_doors = world.items.values().all(|it| {
                let is_door = map.item_is_door(it.graphic);
                let blocks = !it.is_multi
                    && it.container.is_none()
                    && it.pos.x as i64 == x
                    && it.pos.y as i64 == y
                    && map.item_blocks(it.graphic, it.pos.z as i32, current_z)
                    && !(ghost && is_door);
                if blocks && is_door {
                    a_door = Some(it.serial);
                }
                !blocks || is_door
            }) && multis
                .is_none_or(|m| multi_blocker_at(world, m, map, x, y, current_z, ghost).is_none());
            if all_blockers_are_doors {
                // Every blocker on this tile is an openable door — recompute
                // without dynamic items (the static base — real statics AND
                // any multi-contributed surface — still applies).
                let extra = multis
                    .map(|m| multi_statics_at(world, m, map, x, y))
                    .unwrap_or_default();
                let z = map
                    .walkable_z_explain(x as u32, y as u32, current_z, &extra)
                    .ok();
                // Only pair the door with a Z that actually resolved — a
                // `None` Z here (rare: the static base itself denies once the
                // dynamic items are dropped) must report no door either.
                (z, z.and(a_door))
            } else {
                (None, None)
            }
        }
        Err(_) => (None, None),
    }
}

/// Serial of a **closed door** item sitting on (x, y) that's currently
/// blocking a body at `current_z`, if any — used by the click-to-walk
/// executor to know when it should open a door instead of giving up (see
/// [`tile_walkable_for_planning`]'s doc for why this is safe to treat as
/// "walkable, given we act on it"). Multi components are never doors
/// themselves (see [`multi_blocker_at`]'s doc), so this only ever needs to
/// look at `World::items`.
pub fn door_blocking_at(
    world: &World,
    map: &MapData,
    x: i64,
    y: i64,
    current_z: i32,
) -> Option<u32> {
    world
        .items
        .values()
        .find(|it| {
            !it.is_multi
                && it.container.is_none()
                && it.pos.x as i64 == x
                && it.pos.y as i64 == y
                && map.item_is_door(it.graphic)
                && map.item_blocks(it.graphic, it.pos.z as i32, current_z)
        })
        .map(|it| it.serial)
}

/// [`Terrain`] over the live map + dynamic world items, with a blacklist of tiles
/// the server has *denied* (static map says walkable, a building/blocker disagrees)
/// so re-paths route around them. Mirrors `Session::navigate_to`'s `Avoiding`.
/// Shared by `play_server`'s click-to-walk executor and `lib.rs`'s `Route` (the
/// headless `anima-agent`/`anima2` driver) — both plan a route the same way, so
/// this is the ONE place that combines the static map with `World`'s dynamic
/// items for A* planning; see [`tile_walkable_for_planning`]'s doc.
pub(crate) struct MapTerrain<'a> {
    pub(crate) world: &'a World,
    pub(crate) map: &'a mut MapData,
    pub(crate) blocked: &'a HashSet<(u32, u32)>,
    pub(crate) multis: Option<&'a Multis>,
}

impl Terrain for MapTerrain<'_> {
    fn walkable_step(&mut self, x: u32, y: u32, from_z: i32) -> Option<i32> {
        if self.blocked.contains(&(x, y)) {
            return None;
        }
        // Planning, not a real committed step: a closed door doesn't block a
        // *route* (see `tile_walkable_for_planning`'s doc) — the auto-walk
        // executor opens any door it actually needs to step through.
        tile_walkable_for_planning(
            self.world,
            self.map,
            self.multis,
            x as i64,
            y as i64,
            from_z,
        )
    }

    /// `Terrain::door_at`'s real (dynamic-item-aware) implementation — reuses
    /// [`door_blocking_at`] so this can never drift from `play_server`'s own
    /// door check. Shared by `play_server`'s click-to-walk executor (which
    /// still calls `door_blocking_at` directly, having `world`/`map` in hand
    /// already) and `lib.rs`'s `Route` (via this generic `Terrain` hook,
    /// which is all a headless driver has).
    fn door_at(&mut self, x: u32, y: u32, current_z: i32) -> Option<u32> {
        door_blocking_at(self.world, self.map, x as i64, y as i64, current_z)
    }
}

/// A closed door blocking the next hop gets this many `Use` (open) attempts,
/// one per cadence tick, before we give up and treat its tile like any other
/// wall (blacklist + re-path around it). Bounds the case a real UO player
/// would also fail at — a *locked* door — so that still ends in "boxed in"
/// instead of hammering `Use` on it forever. Shared by `play_server`'s
/// click-to-walk executor and `lib.rs`'s `Route` — both open a door the same
/// way (see [`decide_blocked_step`]).
pub(crate) const MAX_DOOR_OPEN_ATTEMPTS: u32 = 3;

/// How long to wait after sending `Use` on a door before resending it, if the
/// door's own state hasn't visibly changed in the meantime — comfortably
/// above a realistic RTT + server processing time, so a slow (but eventually
/// successful) round trip doesn't get its own toggle undone by an impatient
/// resend (see [`BlockedStepAction::AwaitDoor`]'s doc). Well above the 400ms
/// cadence tick both `play_server` and `Route` check this on.
pub(crate) const DOOR_USE_COOLDOWN: Duration = Duration::from_millis(1200);

/// What the auto-walk executor should do about a next-hop tile that the
/// *strict* (real-movement) check just denied. Pulled out as a pure function
/// so the door-vs-wall decision is unit-testable without a live map/session.
/// Shared by `play_server`'s click-to-walk loop and `lib.rs`'s `Route`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockedStepAction {
    /// Send `Use` on this door serial instead of walking — it may just be
    /// closed, not locked, and this tick hasn't tried (enough), and either
    /// we've never sent one yet or the previous one has had a full
    /// [`DOOR_USE_COOLDOWN`] to land with no visible effect.
    OpenDoor(u32),
    /// A `Use` for this door was sent recently and hasn't had time to show
    /// an effect yet — do nothing this tick. ServUO's `Use` TOGGLES a door,
    /// so blindly resending every cadence tick (400ms) would close what a
    /// slower-than-usual round trip (RTT > 400ms) had *just* opened.
    AwaitDoor,
    /// Give up on this tile like any other wall: blacklist it so the next
    /// re-path routes around (or "boxed in" if that was the only way).
    Blacklist,
}

/// Per-blocked-tile bookkeeping for the door-open retry loop (see
/// [`decide_blocked_step`]): how many `Use` attempts have been sent so far,
/// when the most recent one was sent, and the door's own graphic at that
/// moment. The graphic is ServUO's usual tell that a door's state actually
/// changed (open/closed swap the item's graphic — a `0x1A`/delta update
/// arrives for it), so comparing it against the door's CURRENT graphic is
/// how the executor knows a sent `Use` already landed instead of guessing
/// off a fixed timer alone.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DoorUseAttempt {
    pub(crate) count: u32,
    pub(crate) sent_at: Instant,
    pub(crate) graphic_at_send: u16,
}

/// Decide what to do about a blocked next-hop tile. A closed door gets up to
/// [`MAX_DOOR_OPEN_ATTEMPTS`] `Use` attempts (it might open); anything else —
/// including a door we've already tried enough times (probably locked) — is
/// treated like a wall. Among those attempts, a fresh `Use` is only sent once
/// either the door's state has visibly changed since the last one we sent
/// (`door_state_changed`) or [`DOOR_USE_COOLDOWN`] has elapsed with no such
/// change (`pending_use_sent_at`, `now`) — otherwise we're still waiting on
/// the previous `Use` and must not resend (see [`BlockedStepAction::AwaitDoor`]).
pub(crate) fn decide_blocked_step(
    door: Option<u32>,
    attempts_so_far: u32,
    pending_use_sent_at: Option<Instant>,
    door_state_changed: bool,
    now: Instant,
) -> BlockedStepAction {
    match door {
        Some(serial) if attempts_so_far < MAX_DOOR_OPEN_ATTEMPTS => match pending_use_sent_at {
            Some(sent_at)
                if !door_state_changed && now.duration_since(sent_at) < DOOR_USE_COOLDOWN =>
            {
                BlockedStepAction::AwaitDoor
            }
            _ => BlockedStepAction::OpenDoor(serial),
        },
        _ => BlockedStepAction::Blacklist,
    }
}

/// A dead player uses one of ServUO's race-specific ghost bodies. Ghosts walk
/// through doors.
fn player_is_ghost(world: &World) -> bool {
    world
        .player_mobile()
        .is_some_and(|m| anima_core::world::is_ghost_body(m.body))
}

/// UO direction (0=N..7=NW) → (dx, dy) tile delta.
fn delta(d: u8) -> (i64, i64) {
    match d & 7 {
        0 => (0, -1),
        1 => (1, -1),
        2 => (1, 0),
        3 => (1, 1),
        4 => (0, 1),
        5 => (-1, 1),
        6 => (-1, 0),
        _ => (-1, -1),
    }
}

/// Inverse of [`delta`]: a one-tile (dx, dy) step → its UO direction, or `None` if
/// not a unit step. Used to pick the approach direction for [`calculate_new_z`].
fn dir_from_delta(dx: i64, dy: i64) -> Option<u8> {
    match (dx, dy) {
        (0, -1) => Some(0),
        (1, -1) => Some(1),
        (1, 0) => Some(2),
        (1, 1) => Some(3),
        (0, 1) => Some(4),
        (-1, 1) => Some(5),
        (-1, 0) => Some(6),
        (-1, -1) => Some(7),
        _ => None,
    }
}

/// The server's real movement gate: can a body at `(px, py, pz)` step in
/// direction `dir`, and if so what Z would it land at? Faithful to ClassicUO
/// `CanWalk`'s per-tile test — `CanWalk` IS `CalculateNewZ`, there is no
/// separate scoring pass in the client — so, unlike [`explain_tile_walkable`]
/// (which scores via the coarser `MapData::walkable_z_explain`), this
/// resolves the landing Z with the full [`calculate_new_z`] (surfaces,
/// bridges, headroom, the StepHeight climb limit) and then judges
/// [`blocking_item_at`]'s SAME dynamic-item/multi-component blocker rules
/// against THAT resolved landing Z, not the pre-step one — exactly the
/// pairing `build_scene`'s per-tile `w` already uses inside `CHAIN_RADIUS`
/// (see `blocking_item_at`'s doc for the live foundation-stairs bug this
/// fixes: bridge `0x0751` h=5 at z=0 → stand z=2; impassable riser `0x0063`
/// z=0..5; plot surface `0x31F4` at z=7 → landing z=7 — the riser is well
/// clear of a body actually AT z=7, even though it overlaps one still down at
/// z=2). Before this existed, [`step_ok`] re-derived its own blocker check
/// inline, checked at `fz` (the pre-step Z) — the same bug `blocking_item_at`
/// was split out to fix for `build_scene`, just not yet applied to the real
/// movement gate; this is that fix, applied once, for every caller that asks
/// "can this body take this step" (see [`step_ok`]/[`can_walk`], now thin
/// wrappers over this).
pub fn can_step_to(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    px: i64,
    py: i64,
    pz: i32,
    dir: u8,
) -> Result<i32, StepDeny> {
    let (dx, dy) = delta(dir);
    let (nx, ny) = (px + dx, py + dy);
    if nx < 0 || ny < 0 {
        return Err(StepDeny::OffMap);
    }
    let z = calculate_new_z(world, map, multis, nx, ny, pz, dir).ok_or(StepDeny::NoLanding)?;
    let ghost = player_is_ghost(world);
    let dyn_items = dynamic_statics_at(world, map, nx, ny);
    match blocking_item_at(world, map, multis, nx, ny, z, &dyn_items, ghost) {
        Some(deny) => Err(deny),
        None => Ok(z),
    }
}

/// Can a body at (fx, fy, fz) step in direction `dir`? Thin wrapper over
/// [`can_step_to`] (`.is_ok()`) — see its doc for what changed and why.
fn step_ok(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    fx: i64,
    fy: i64,
    fz: i32,
    dir: u8,
) -> bool {
    can_step_to(world, map, multis, fx, fy, fz, dir).is_ok()
}

/// ClassicUO `Pathfinder.CanWalk`: resolve a requested step from (x, y, z).
/// Returns the (possibly redirected) direction and destination tile, or `None`
/// if fully blocked. A **diagonal** step (1) forbids corner-cutting — both
/// adjacent cardinal tiles must be free — and (2) if blocked, redirects to the
/// first free adjacent cardinal, so you *slide along a wall* instead of stopping.
/// A blocked **cardinal** step just fails (no redirect), matching ClassicUO.
pub fn can_walk(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
    z: i32,
    dir: u8,
) -> Option<(u8, i64, i64)> {
    let dir = dir & 7;
    let (dx, dy) = delta(dir);
    let (mut nx, mut ny, mut ndir) = (x + dx, y + dy, dir);
    let mut passed = step_ok(world, map, multis, x, y, z, dir);

    if dir % 2 == 1 {
        // Diagonal: no corner-cutting — both flanking cardinals must be open too.
        if passed {
            for off in [1i32, -1] {
                let cd = (dir as i32 + off).rem_euclid(8) as u8;
                if !step_ok(world, map, multis, x, y, z, cd) {
                    passed = false;
                    break;
                }
            }
        }
        // Blocked diagonal → slide: redirect to the first open flanking cardinal.
        if !passed {
            for off in [1i32, -1] {
                let cd = (dir as i32 + off).rem_euclid(8) as u8;
                if step_ok(world, map, multis, x, y, z, cd) {
                    let (cx, cy) = delta(cd);
                    ndir = cd;
                    nx = x + cx;
                    ny = y + cy;
                    passed = true;
                    break;
                }
            }
        }
    }

    if passed {
        Some((ndir, nx, ny))
    } else {
        None
    }
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
const FLAG_IMPASSABLE: u64 = 0x40;
const FLAG_SURFACE: u64 = 0x200;
const FLAG_BRIDGE: u64 = 0x400;
const FLAG_ROOF: u64 = 0x1000_0000;
/// Foliage flag (trees/bushes): the renderer fades these when they'd hide the
/// player, like ClassicUO's foliage transparency.
const FLAG_FOLIAGE: u64 = 0x2_0000;
/// Stackable flag (`TileFlag.Generic`, ClassicUO `ItemData.IsStackable`): drives
/// whether a dragged stack (amount > 1) offers the split-stack dialog, mirroring
/// ClassicUO `GameActions.PickUp` (`item.Amount > 1 && item.ItemData.IsStackable`).
const FLAG_STACKABLE: u64 = 0x800;

/// Per-frame interval (ms) for an animated static, from animdata's `frameInterval`
/// tick count. The raw value is a small tick count (often 0–3); we scale it into a
/// lively-but-not-frantic range so flames flicker and fountains/wheels turn at a
/// believable pace (mirrors the effects path, which scales interval ×50ms).
fn anim_interval_ms(interval: u8) -> u32 {
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
fn anim_suffix(map: &MapData, animdata: Option<&AnimData>, graphic: u16) -> String {
    let mut anim = String::new();
    if map.item_is_animated(graphic) {
        if let Some(ad) = animdata {
            let seq = ad.frame_sequence(graphic);
            if seq.len() > 1 {
                let ai = anim_interval_ms(ad.frames(graphic).1);
                anim.push_str(",\"a\":[");
                for (i, g) in seq.iter().enumerate() {
                    if i > 0 {
                        anim.push(',');
                    }
                    let _ = write!(anim, "{g}");
                }
                let _ = write!(anim, "],\"ai\":{ai}");
            }
        }
    }
    anim
}

/// The `h`/`pf` VALUES a graphic within [`PATH_RADIUS`] of the player would
/// carry: `h` is the tiledata HEIGHT, `pf` packs the three flag bits the
/// browser's `calculate_new_z` port needs (bit 0 impassable, bit 1 surface,
/// bit 2 bridge). Each is `None` when out-of-radius or zero. Split out from
/// [`path_suffix`] (which formats these straight into a hand-written JSON
/// string, for the real-statics/multi-component loops) so the dynamic-items
/// loop in [`build_scene`] — which builds a `serde_json::Value`, not a raw
/// string — can share the exact same bits instead of re-deriving them.
fn path_bits(in_radius: bool, height: u8, flags: u64) -> (Option<u8>, Option<u8>) {
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
fn path_suffix(in_radius: bool, height: u8, flags: u64) -> String {
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
fn door_suffix(door: Option<u32>) -> String {
    match door {
        Some(serial) => format!(",\"dr\":{serial}"),
        None => String::new(),
    }
}

/// Real statics at `(x, y)` PLUS, when `multis` is given, any in-view multi
/// component there — as `(z, flags)` pairs, everything [`max_draw_z`] /
/// [`calculate_near_z`]'s roof-culling needs. A placed multi's own roof (a
/// house has real `FLAG_ROOF` components) must lift off exactly like a real
/// static roof does when the player is inside it — the static map alone has
/// no idea a multi is even there (see [`multi_components_at`]'s doc), so
/// without this a boat/house roof would never cull and the interior would
/// never show.
fn roof_scan_tiles(
    world: &World,
    multis: Option<&Multis>,
    map: &mut MapData,
    x: i64,
    y: i64,
) -> Vec<(i32, u64)> {
    let mut out: Vec<(i32, u64)> = map
        .statics(x as u32, y as u32)
        .into_iter()
        .map(|s| (s.z as i32, s.flags))
        .collect();
    if let Some(multis) = multis {
        out.extend(
            multi_components_at(world, multis, x, y)
                .into_iter()
                .map(|(graphic, z)| (z, map.item_flags(graphic))),
        );
    }
    out
}

/// ClassicUO `UpdateMaxDrawZ`: the Z at/above which statics are hidden so a roof
/// or upper floor over the player vanishes and the interior shows. 127 = draw all.
/// `multis` widens both scans below to in-view multi components (a house roof
/// is no different from a real static one) via [`roof_scan_tiles`].
fn max_draw_z(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    px: i64,
    py: i64,
    pz: i32,
) -> i32 {
    if px < 0 || py < 0 {
        return 127;
    }
    let mut max_z = 127i32;
    let pz14 = pz + 14;
    let pz16 = pz + 16;

    // Ground above the player (terrain overhang/cave) → cap at pz+16.
    if pz16 <= map.land(px as u32, py as u32).z as i32 {
        return pz16;
    }
    // Statics (+ multi components) over the player's own tile: an upper floor
    // / non-roof blocker.
    for (tz, flags) in roof_scan_tiles(world, multis, map, px, py) {
        if tz > pz14 && tz < max_z {
            let is_roof = flags & FLAG_ROOF != 0;
            let is_surface = flags & FLAG_SURFACE != 0;
            if (flags & 0x2_0004) == 0 && (!is_roof || is_surface) {
                max_z = tz;
            }
        }
    }
    // Roofs over the tile the player faces into (x+1, y+1). A roof collapses the
    // ceiling to the *near-Z* of its whole connected span (CalculateNearZ), so a
    // pitched roof lifts off cleanly instead of just its peak band.
    let mut roof_found = false;
    for (tz, flags) in roof_scan_tiles(world, multis, map, px + 1, py + 1) {
        if tz > pz14 && tz < max_z {
            let is_roof = flags & FLAG_ROOF != 0;
            if (flags & 0x204) == 0 && is_roof {
                let mut visited = HashSet::new();
                max_z = calculate_near_z(world, multis, map, px + 1, py + 1, tz, tz, &mut visited);
                roof_found = true;
            }
        }
    }

    // ClassicUO clamps the ceiling to at least pz+16 (you always see ~16 above
    // your head). Only when something was actually found over the player.
    if max_z != 127 || roof_found {
        max_z = max_z.max(pz16);
    }
    max_z
}

/// Flood-fill the lowest connected roof Z within ±6 of `z`, starting at (x, y).
/// Ported from ClassicUO `Map.CalculateNearZ`. `visited` prevents revisits.
/// `multis` (see [`roof_scan_tiles`]) lets a house's own roof components join
/// the flood, so a multi roof spanning several tiles lifts off as one
/// connected span instead of stopping dead at the first non-static tile.
#[allow(clippy::too_many_arguments)]
fn calculate_near_z(
    world: &World,
    multis: Option<&Multis>,
    map: &mut MapData,
    x: i64,
    y: i64,
    z: i32,
    default_z: i32,
    visited: &mut HashSet<(i64, i64)>,
) -> i32 {
    if x < 0 || y < 0 || !visited.insert((x, y)) {
        return default_z;
    }
    let roof = roof_scan_tiles(world, multis, map, x, y)
        .into_iter()
        .find(|&(tz, flags)| flags & FLAG_ROOF != 0 && (z - tz).abs() <= 6);
    let Some((tz, _)) = roof else {
        return default_z;
    };
    let mut near = default_z.min(tz);
    near = calculate_near_z(world, multis, map, x - 1, y, tz, near, visited);
    near = calculate_near_z(world, multis, map, x + 1, y, tz, near, visited);
    near = calculate_near_z(world, multis, map, x, y - 1, tz, near, visited);
    near = calculate_near_z(world, multis, map, x, y + 1, tz, near, visited);
    near
}

// ----------------------------------------------------------------------------
// Step-Z resolution — a faithful port of ClassicUO `Pathfinder.CalculateNewZ`
// (+ `CalculateMinMaxZ`, `CreateItemList`). The server's ConfirmWalk carries no
// Z, so when the player steps onto a tile we resolve the standing Z exactly as
// the client does: build the tile's object list, bound the step by the tile we
// came from, and pick the surface/bridge closest to our current Z with headroom.
// This is what makes stairs (bridge tiles, avg Z = z + height/2) climb correctly.
// ----------------------------------------------------------------------------

/// ClassicUO `PATH_OBJECT_FLAGS` (we only model the NORMAL step state).
const POF_IMPASS: u32 = 0x1; // POF_IMPASSABLE_OR_SURFACE
const POF_SURFACE: u32 = 0x2;
const POF_BRIDGE: u32 = 0x4;
/// `Constants.DEFAULT_BLOCK_HEIGHT` — head/body clearance needed to stand.
const BLOCK_HEIGHT: i32 = 16;
/// 8-direction deltas (`Pathfinder._offsetX/_offsetY`), dir 0=N..7=NW.
const OFF_X: [i64; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
const OFF_Y: [i64; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];

/// One walkable/blocking surface on a tile (ClassicUO `PathObject`). Plain data
/// (all `Copy` fields) — derived so tests can build small synthetic tile lists
/// (e.g. a staircase) without fighting the borrow checker over reused literals.
#[derive(Clone, Copy)]
struct PathObj {
    flags: u32,
    z: i32,
    avg_z: i32,
    height: i32,
    land_stretched: bool,
}

/// Land Z at (x, y), or a deep floor for out-of-bounds (ClassicUO uses -125).
fn land_z(map: &mut MapData, x: i64, y: i64) -> i32 {
    if x < 0 || y < 0 {
        return -125;
    }
    map.land(x as u32, y as u32).z as i32
}

/// Land `AverageZ` / `MinZ` from the 4 corners (ClassicUO `Land.ApplyStretch`),
/// plus whether the tile is sloped (corners differ → "stretched").
fn land_avg_min(map: &mut MapData, x: i64, y: i64) -> (i32, i32, bool) {
    let z_top = land_z(map, x, y);
    let z_right = land_z(map, x + 1, y);
    let z_left = land_z(map, x, y + 1);
    let z_bottom = land_z(map, x + 1, y + 1);
    let avg = if (z_top - z_bottom).abs() <= (z_left - z_right).abs() {
        (z_top + z_bottom) >> 1
    } else {
        (z_left + z_right) >> 1
    };
    let min = z_top.min(z_right).min(z_left).min(z_bottom);
    let stretched = !(z_top == z_right && z_right == z_left && z_left == z_bottom);
    (avg, min, stretched)
}

/// ClassicUO `Land.CalculateCurrentAverageZ` — the slope Z toward `direction`.
fn calc_current_average_z(map: &mut MapData, x: i64, y: i64, direction: i32) -> i32 {
    let z_top = land_z(map, x, y);
    let z_right = land_z(map, x + 1, y);
    let z_bottom = land_z(map, x + 1, y + 1);
    let z_left = land_z(map, x, y + 1);
    let gdz = |d: i32| match d & 3 {
        1 => z_right,
        2 => z_bottom,
        3 => z_left,
        _ => z_top,
    };
    let result = gdz(((direction >> 1) + 1) & 3);
    if direction & 1 != 0 {
        result
    } else {
        (result + gdz(direction >> 1)) >> 1
    }
}

/// Turn a tiledata-flags-bearing object at world Z `z` with tiledata `height`
/// into a [`PathObj`] the same way ClassicUO's `CreateItemList` treats a real
/// static, or `None` if it contributes nothing to standing (`flags == 0` —
/// neither impassable nor surface/bridge). Shared by [`create_item_list`]'s
/// real-statics loop and its multi-component loop (a boat deck plank or house
/// floor tile) so the two can never derive the impassable/surface/bridge bits
/// differently.
fn tiledata_path_obj(z: i32, height: i32, tile_flags: u64) -> Option<PathObj> {
    let impassable = tile_flags & FLAG_IMPASSABLE != 0;
    let is_surface = tile_flags & FLAG_SURFACE != 0;
    let is_bridge = tile_flags & FLAG_BRIDGE != 0;
    let mut flags = 0u32;
    if impassable || is_surface {
        flags = POF_IMPASS;
    }
    if !impassable {
        if is_surface {
            flags |= POF_SURFACE;
        }
        if is_bridge {
            flags |= POF_BRIDGE;
        }
    }
    if flags == 0 {
        return None;
    }
    // Bridges (stairs/ramps) stand at half height; surfaces at full.
    let avg = if is_bridge { height / 2 } else { height } + z;
    Some(PathObj {
        flags,
        z,
        avg_z: avg,
        height,
        land_stretched: false,
    })
}

/// ClassicUO `Pathfinder.CreateItemList`: land + statics + **ordinary dynamic
/// items** (see [`dynamic_statics_at`]'s doc — a boat deck on this shard is
/// exactly this shape) **and, when `multis` is given, in-view multi
/// components** (a boat deck/hull, house floor/wall) on a tile, as `PathObj`s
/// (mobiles are not modelled here — they rarely change the standing Z).
/// Dynamic items and multi components matter here for the SAME reason they
/// matter for blocking (see `multi_blocker_at`'s doc): without them, stepping
/// onto/around a boat whose deck sits at a Z the static map alone knows
/// nothing about would resolve the wrong standing Z (or none at all) and every
/// step would look like a deny. Every dynamic item is folded in through
/// [`tiledata_path_obj`] exactly like a real static or multi component — this
/// list-building loop, unlike [`explain_tile_walkable`]'s scoring, has no
/// impassable/surface split to worry about (its only caller, [`step_ok`], has
/// its own separate `blocked_by_item` check regardless of what
/// `calculate_new_z` decides).
fn create_item_list(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
) -> Vec<PathObj> {
    let mut list = Vec::new();
    if x < 0 || y < 0 {
        return list;
    }
    let land = map.land(x as u32, y as u32);
    let g = land.graphic;
    // Skip the "no-draw" land graphics (void/cave markers), like ClassicUO.
    if (g < 0x01AE && g != 2) || (g > 0x01B5 && g != 0x01DB) {
        let mut flags = POF_IMPASS;
        if !land.impassable() {
            flags |= POF_SURFACE | POF_BRIDGE;
        }
        let (avg, min, stretched) = land_avg_min(map, x, y);
        list.push(PathObj {
            flags,
            z: min,
            avg_z: avg,
            height: avg - min,
            land_stretched: stretched,
        });
    }
    for s in map.statics(x as u32, y as u32) {
        if let Some(obj) = tiledata_path_obj(s.z as i32, s.height as i32, s.flags) {
            list.push(obj);
        }
    }
    for it in dynamic_statics_at(world, map, x, y) {
        if let Some(obj) = tiledata_path_obj(it.z as i32, it.height as i32, it.flags) {
            list.push(obj);
        }
    }
    if let Some(multis) = multis {
        for (graphic, cz) in multi_components_at(world, multis, x, y) {
            let h = map.item_height(graphic) as i32;
            if let Some(obj) = tiledata_path_obj(cz, h, map.item_flags(graphic)) {
                list.push(obj);
            }
        }
    }
    list
}

/// Pure core of [`calc_min_max_z`] (ClassicUO `Pathfinder.CalculateMinMaxZ`'s
/// scoring loop): given the tile-behind's already-built [`PathObj`] list and
/// (for a stretched/sloped land tile) its direction-biased average Z, compute
/// the step's `(min_z, max_z)` bound. Split out — like
/// `anima_assets::map::score_walkable_z` — so a synthetic staircase (no real
/// `MapData`) can unit-test the standing-Z math directly; see
/// `resolve_standing_z` for the matching destination-tile half.
fn bound_min_max_z(source: &[PathObj], current_z: i32, stretched_avg: i32) -> (i32, i32) {
    let mut min_z = -128i32;
    let mut max_z = current_z;
    for obj in source {
        let avg = obj.avg_z;
        if avg <= current_z && obj.land_stretched {
            min_z = min_z.max(stretched_avg);
            max_z = max_z.max(stretched_avg);
        } else {
            if obj.flags & POF_IMPASS != 0 && avg <= current_z && min_z < avg {
                min_z = avg;
            }
            if obj.flags & POF_BRIDGE != 0 && current_z == avg {
                max_z = max_z.max(obj.z + obj.height);
                min_z = min_z.min(obj.z);
            }
        }
    }
    (min_z, max_z + 2)
}

/// ClassicUO `Pathfinder.CalculateMinMaxZ`: bound the step using the tile we
/// came *from* (opposite of `direction`). Returns `(min_z, max_z)`.
fn calc_min_max_z(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
    current_z: i32,
    direction: u8,
) -> (i32, i32) {
    let back = (direction ^ 4) & 7;
    let sx = x + OFF_X[back as usize];
    let sy = y + OFF_Y[back as usize];
    let source = create_item_list(world, map, multis, sx, sy);
    // Only land can be "stretched" (sloped) — at most one land entry per tile,
    // so this is computed at most once, matching the original inline call site.
    let stretched_avg = if source.iter().any(|o| o.land_stretched) {
        calc_current_average_z(map, sx, sy, direction as i32)
    } else {
        0
    };
    bound_min_max_z(&source, current_z, stretched_avg)
}

/// Pure core of [`calculate_new_z`] (ClassicUO `Pathfinder.CalculateNewZ`'s
/// surface/bridge/headroom scoring loop): given the destination tile's
/// already-built (unsorted) [`PathObj`] list and the step's `(min_z, max_z)`
/// bound from [`bound_min_max_z`], resolve the standing Z. `None` when nothing
/// in the list has clearance to stand on (a real DenyWalk situation). Split out
/// so a synthetic staircase can unit-test this without a real `MapData`.
fn resolve_standing_z(
    mut list: Vec<PathObj>,
    min_z: i32,
    max_z: i32,
    current_z: i32,
) -> Option<i32> {
    if list.is_empty() {
        return None;
    }
    // Sort by Z then height (PathObject.CompareTo), then add the "sky" sentinel.
    list.sort_by(|a, b| a.z.cmp(&b.z).then(a.height.cmp(&b.height)));
    list.push(PathObj {
        flags: POF_IMPASS,
        z: 128,
        avg_z: 128,
        height: 128,
        land_stretched: false,
    });

    let mut z = current_z;
    if z < min_z {
        z = min_z;
    }
    let mut min_z = min_z;
    let mut result_z = -128i32;
    let mut best_delta = i32::MAX;
    let mut cur_z = -128i32;

    for i in 0..list.len() {
        if list[i].flags & POF_IMPASS == 0 {
            continue;
        }
        let obj_z = list[i].z;
        // A ceiling object with clearance above the floor below it: find the
        // best surface/bridge under it that we can actually stand on.
        if obj_z - min_z >= BLOCK_HEIGHT {
            for j in (0..i).rev() {
                let t = &list[j];
                if t.flags & (POF_SURFACE | POF_BRIDGE) == 0 {
                    continue;
                }
                let tavg = t.avg_z;
                let fits = (tavg <= max_z && t.flags & POF_SURFACE != 0)
                    || (t.flags & POF_BRIDGE != 0 && t.z <= max_z);
                if tavg >= cur_z && obj_z - tavg >= BLOCK_HEIGHT && fits {
                    let delta = (z - tavg).abs();
                    if delta < best_delta {
                        best_delta = delta;
                        result_z = tavg;
                    }
                }
            }
        }
        let avg = list[i].avg_z;
        min_z = min_z.max(avg);
        cur_z = cur_z.max(avg);
    }

    if result_z == -128 {
        None
    } else {
        Some(result_z)
    }
}

/// ClassicUO `Pathfinder.CalculateNewZ`: the standing Z when stepping onto
/// `(x, y)` from `current_z` heading `direction`. `None` when the tile has no
/// valid surface to stand on (a real DenyWalk situation). `multis` (when given)
/// lets a boat deck/house floor contribute a standing Z the static map alone
/// wouldn't know about — see [`create_item_list`]'s doc.
pub fn calculate_new_z(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    x: i64,
    y: i64,
    current_z: i32,
    direction: u8,
) -> Option<i32> {
    if x < 0 || y < 0 {
        return None;
    }
    let (min_z, max_z) = calc_min_max_z(world, map, multis, x, y, current_z, direction);
    let list = create_item_list(world, map, multis, x, y);
    resolve_standing_z(list, min_z, max_z, current_z)
}

/// Tiles per output pixel when rendering the full-world map. 1 = full resolution
/// (one pixel per tile), so the client maps world tile (x, y) → image pixel 1:1.
/// Must match the JS `WORLDMAP_STEP` in `web/main.js`.
pub const WORLDMAP_STEP: u32 = 1;

/// Render the whole facet to a full-resolution RGBA PNG using ClassicUO's exact
/// world-map algorithm (`WorldMapGump.LoadMap`): per tile take the radar LAND
/// color, then overlay each STATIC top-most-by-Z with its radar STATIC color, then
/// a Z-relief shading pass that embosses slopes. This makes buildings, roads,
/// water and walls visible (the old land-average path showed only blurry terrain).
///
/// Traversal is block-by-block (8×8 cells) via [`MapData::block_cells`] so each
/// map/statics block is decoded exactly once — the per-pixel `land()`/`statics()`
/// path would be far too slow across the ~29M cells. `step` is accepted for API
/// symmetry but full resolution (1) is used. The caller renders this once and
/// caches the PNG.
pub fn render_worldmap(map: &mut MapData, radar: &RadarCol, _step: u32) -> Vec<u8> {
    let w = MAP_WIDTH as usize;
    let h = MAP_HEIGHT as usize;
    let mut rgba = vec![0u8; w * h * 4];
    // Parallel per-pixel Z buffer (ClassicUO `allZ`): land Z, raised by the
    // top-most static, then read by the relief pass.
    let mut allz = vec![0i8; w * h];

    let blocks_x = MAP_WIDTH / 8;
    let blocks_y = MAP_HEIGHT / 8;
    for bx in 0..blocks_x {
        let base_x = (bx * 8) as usize;
        for by in 0..blocks_y {
            let (land, statics) = map.block_cells(bx, by);
            let base_y = (by * 8) as usize;
            for cy in 0..8usize {
                for cx in 0..8usize {
                    let cell = cy * 8 + cx;
                    let (g, z) = land[cell];
                    let idx = (base_y + cy) * w + (base_x + cx);
                    let o = idx * 4;
                    let c = radar.land_color(g);
                    rgba[o] = c[0];
                    rgba[o + 1] = c[1];
                    rgba[o + 2] = c[2];
                    rgba[o + 3] = 255;
                    allz[idx] = z;
                    // Statics in file order; the top-most by Z wins (>= so a later
                    // equal-Z static overrides), giving roads/water/buildings.
                    for s in &statics[cell] {
                        if s.graphic == 0 || s.graphic == 0xFFFF {
                            continue;
                        }
                        if s.z >= allz[idx] {
                            let sc = radar.static_color(s.graphic);
                            rgba[o] = sc[0];
                            rgba[o + 1] = sc[1];
                            rgba[o + 2] = sc[2];
                            rgba[o + 3] = 255;
                            allz[idx] = s.z;
                        }
                    }
                }
            }
        }
    }

    // Z-relief shading (ClassicUO): compare each pixel's Z to the pixel one row
    // SOUTH. Lower-than-south → darken ×0.8; higher-than-south → brighten ×1.25
    // (clamped). Equal → unchanged. This is the embossed terrain look.
    const MAG_DARK: f32 = 80.0 / 100.0;
    const MAG_LIGHT: f32 = 100.0 / 80.0;
    for y in 0..h - 1 {
        let row = y * w;
        for x in 0..w {
            let idx = row + x;
            let z0 = allz[idx];
            let z1 = allz[idx + w];
            if z0 == z1 {
                continue;
            }
            let o = idx * 4;
            // Leave pure-black/empty pixels untouched (ClassicUO skips PackedValue 0).
            if rgba[o] == 0 && rgba[o + 1] == 0 && rgba[o + 2] == 0 {
                continue;
            }
            let mag = if z0 < z1 { MAG_DARK } else { MAG_LIGHT };
            for k in 0..3 {
                rgba[o + k] = (rgba[o + k] as f32 * mag).min(255.0) as u8;
            }
        }
    }

    Image {
        width: MAP_WIDTH,
        height: MAP_HEIGHT,
        rgba,
    }
    .to_png()
}

/// Convert a core-parsed [`GumpElement`] into the renderer's positioned JSON
/// shape (`t`/`x`/`y`/…). The grammar itself now lives in
/// [`anima_core::gump_layout`] (it's protocol data, not rendering); this is
/// just the JSON shaping plus cliloc resolution (which needs
/// `anima_assets::Cliloc`, unavailable to the zero-dep core) — ported
/// unchanged from the old inline `parse_gump_layout` so the scene JSON this
/// produces is byte-for-byte identical to before the split.
fn gump_element_json(e: &GumpElement, cliloc: Option<&Cliloc>) -> Value {
    match e {
        GumpElement::Background { x, y, w, h, page } => {
            json!({"t":"bg","x":x,"y":y,"w":w,"h":h,"page":page})
        }
        // Decorative art — we draw a plain marker, so the gump id isn't needed.
        GumpElement::Image { x, y, page, .. } => json!({"t":"bg","x":x,"y":y,"page":page}),
        // `graphic` (the normal-state art) lets the client draw the real button
        // art (a small gump) instead of the raw reply id as text.
        GumpElement::Button {
            x,
            y,
            graphic,
            reply_id,
            pageflag,
            param,
            page,
        } => json!({
            "t":"button","x":x,"y":y,"g":graphic,"id":reply_id,"page":page,
            "pageflag":pageflag,"param":param,
        }),
        GumpElement::Text {
            x,
            y,
            w: None,
            s,
            page,
        } => {
            json!({"t":"text","x":x,"y":y,"s":s,"page":page})
        }
        GumpElement::Text {
            x,
            y,
            w: Some(w),
            s,
            page,
        } => {
            json!({"t":"text","x":x,"y":y,"w":w,"s":s,"page":page})
        }
        // Resolve against the Cliloc table so NPC dialogs show real text, not
        // #ids. Shaped as the SAME "t":"text" JSON as a plain Text element
        // (deliberately — `w` is always `Some` for an html block, so the
        // client's one `e.t === "text"` branch in `renderGumpHtml` handles
        // both). Any UO gump-HTML tags/entities in `s` (`<CENTER>`, `&amp;`,
        // …) are left as-is for the client to interpret — see
        // `GumpElement::Html`'s doc.
        GumpElement::Html {
            x,
            y,
            w,
            text,
            page,
            ..
        } => {
            let s = match text {
                HtmlText::Literal(s) => s.clone(),
                HtmlText::Cliloc {
                    id,
                    args: Some(args),
                } => cliloc
                    .and_then(|c| c.format(*id, args))
                    .unwrap_or_else(|| format!("#{id}")),
                HtmlText::Cliloc { id, args: None } => cliloc
                    .and_then(|c| c.get(*id).map(str::to_string))
                    .unwrap_or_else(|| format!("#{id}")),
            };
            json!({"t":"text","x":x,"y":y,"w":w,"s":s,"page":page})
        }
        GumpElement::Check { x, y, id, on, page } => {
            json!({"t":"check","x":x,"y":y,"id":id,"on":on,"page":page})
        }
        GumpElement::Radio { x, y, id, on, page } => {
            json!({"t":"radio","x":x,"y":y,"id":id,"on":on,"page":page})
        }
        GumpElement::Entry {
            x,
            y,
            w,
            id,
            s,
            page,
        } => {
            json!({"t":"entry","x":x,"y":y,"w":w,"id":id,"s":s,"page":page})
        }
    }
}

/// Build the `gumps` array for the scene: each open server gump (0xB0/0xDD),
/// its layout parsed by [`gump_layout::parse`] into positioned elements (see
/// [`gump_element_json`]).
fn gumps_json(world: &World, cliloc: Option<&Cliloc>) -> String {
    let gumps: Vec<Value> = world
        .gumps
        .iter()
        .map(|g| {
            let layout = gump_layout::parse(&g.layout, &g.text);
            let elements: Vec<Value> = layout
                .elements
                .iter()
                .map(|e| gump_element_json(e, cliloc))
                .collect();
            json!({
                "serial": g.serial, "gumpId": g.gump_id,
                "x": g.x, "y": g.y, "w": layout.width, "h": layout.height,
                "elements": elements,
            })
        })
        .collect();
    serde_json::to_string(&gumps).unwrap_or_else(|_| "[]".into())
}

/// Build the `popup` object for the scene: the open context menu (0xBF/0x14), or
/// `null` when none. Each entry's `text` is resolved from the Cliloc table (falls
/// back to `#<id>` when the table is missing or the id is unknown).
fn popup_json(world: &World, cliloc: Option<&Cliloc>) -> Value {
    match &world.popup {
        None => Value::Null,
        Some(menu) => {
            let entries: Vec<Value> = menu
                .entries
                .iter()
                .map(|e| {
                    let text = cliloc
                        .and_then(|c| c.get(e.cliloc))
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("#{}", e.cliloc));
                    // `hl`: ClassicUO shows entries flagged 0x01 in a highlight hue
                    // (0x0386) — the menu's default/primary action. Pass it through so
                    // the renderer can accent it. (0x02 = submenu arrow; unsupported.)
                    json!({ "index": e.index, "text": text, "hl": e.flags & 0x01 != 0 })
                })
                .collect();
            json!({ "serial": menu.serial, "entries": entries })
        }
    }
}

/// Build every open legacy 0x7C menu in stable serial order. Entry indices are
/// one-based because that is what the matching 0x7D response echoes; zero is
/// reserved for cancel.
fn legacy_menus_json(world: &World) -> Value {
    let mut menus: Vec<_> = world.legacy_menus.iter().collect();
    menus.sort_by_key(|menu| menu.serial);
    Value::Array(
        menus
            .into_iter()
            .map(|menu| {
                let kind = match menu.kind {
                    anima_core::world::LegacyMenuKind::Items => "items",
                    anima_core::world::LegacyMenuKind::Question => "question",
                };
                let entries: Vec<Value> = menu
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        json!({
                            "index": index + 1,
                            "graphic": entry.graphic,
                            "hue": entry.hue,
                            "text": entry.text,
                        })
                    })
                    .collect();
                json!({
                    "serial": menu.serial,
                    "menuId": menu.menu_id,
                    "question": menu.question,
                    "kind": kind,
                    "entries": entries,
                })
            })
            .collect(),
    )
}

/// Build pending server 0x95 hue pickers in stable callback-serial order.
fn hue_pickers_json(world: &World) -> Value {
    let mut pickers = world.hue_pickers.clone();
    pickers.sort_by_key(|picker| picker.serial);
    Value::Array(
        pickers
            .into_iter()
            .map(|picker| json!({ "serial": picker.serial, "graphic": picker.graphic }))
            .collect(),
    )
}

/// Open 0xA6 Tip/Notice windows. Arrival order and `seq` are preserved because
/// repeated packets with the same wire tip id are distinct ClassicUO gumps.
fn tips_json(world: &World) -> Value {
    Value::Array(
        world
            .tips
            .iter()
            .map(|tip| {
                json!({
                    "seq": tip.seq,
                    "tip": tip.tip,
                    "kind": tip.kind.as_str(),
                    "text": tip.text,
                })
            })
            .collect(),
    )
}

/// Open 0xAB modal text-entry dialogs. Arrival order and local `seq` preserve
/// repeated callback tuples as distinct ClassicUO windows.
fn text_entry_dialogs_json(world: &World) -> Value {
    Value::Array(
        world
            .text_entry_dialogs
            .iter()
            .map(|dialog| {
                json!({
                    "seq": dialog.seq,
                    "serial": dialog.serial,
                    "parentId": dialog.parent_id,
                    "buttonId": dialog.button_id,
                    "text": dialog.text,
                    "canClose": dialog.can_close,
                    "variant": dialog.variant,
                    "maxLength": dialog.max_length,
                    "description": dialog.description,
                })
            })
            .collect(),
    )
}

/// Open 0xB8 character profiles. These are persistent windows rather than an
/// event ring: an exact `seq` remains present until the user closes/saves it,
/// and a newer response for the same serial replaces it with a fresh identity.
fn character_profiles_json(world: &World) -> Value {
    Value::Array(
        world
            .character_profiles
            .iter()
            .map(|profile| {
                json!({
                    "seq": profile.seq,
                    "serial": profile.serial,
                    "header": profile.header,
                    "footer": profile.footer,
                    "body": profile.body,
                    "canEdit": profile.can_edit,
                })
            })
            .collect(),
    )
}

/// Latest 0xD1 logout permission reply. The play loop, not the renderer,
/// performs an accepted disconnect; a denied reply remains visible long enough
/// for the browser to restore its logout button and explain the refusal.
fn logout_ack_json(world: &World) -> Value {
    match world.logout_ack {
        Some(ack) => json!({ "seq": ack.seq, "allowed": ack.allowed }),
        None => Value::Null,
    }
}

/// Bounded 0xF6 history used by the browser to interpolate every member of a
/// boat step from the same source, destination, and server speed.
fn boat_movements_json(world: &World) -> Value {
    Value::Array(
        world
            .recent_boat_movements
            .iter()
            .map(|movement| {
                let mut entities = Vec::with_capacity(movement.entities.len() + 1);
                entities.push(json!({
                    "serial": movement.boat_serial,
                    "from": { "x": movement.from.x, "y": movement.from.y, "z": movement.from.z },
                    "to": { "x": movement.to.x, "y": movement.to.y, "z": movement.to.z },
                }));
                entities.extend(movement.entities.iter().map(|entity| {
                    json!({
                        "serial": entity.serial,
                        "from": { "x": entity.from.x, "y": entity.from.y, "z": entity.from.z },
                        "to": { "x": entity.to.x, "y": entity.to.y, "z": entity.to.z },
                    })
                }));
                json!({
                    "seq": movement.seq,
                    "boat": movement.boat_serial,
                    "speed": movement.speed,
                    "dir": movement.moving_direction,
                    "facing": movement.facing_direction,
                    "entities": entities,
                })
            })
            .collect(),
    )
}

/// Build the `party` object for the scene (0xBF/0x06). `leader` is the party
/// leader's serial (0 = none), `members` lists each member `{serial, name, hits,
/// hitsMax}`, and `invite` is the serial of a leader who invited us (0 = none).
/// Member name/hits are resolved from the [`Mobile`] in view — falling back to
/// "Member"/0 when that member isn't currently in range. Always emitted; an empty
/// `members` means we're not in a party.
fn party_json(world: &World) -> Value {
    let members: Vec<Value> = world
        .party
        .members
        .iter()
        .map(|&serial| {
            let m = world.mobiles.get(&serial);
            let name = m
                .map(|m| m.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Member".to_string());
            json!({
                "serial": serial,
                "name": name,
                "hits": m.map_or(0, |m| m.hits),
                "hitsMax": m.map_or(0, |m| m.hits_max),
            })
        })
        .collect();
    json!({
        "leader": world.party.leader,
        "members": members,
        "invite": world.party.pending_invite.unwrap_or(0),
    })
}

/// Maximum number of serials whose OPL (tooltip) lines are emitted per scene, to
/// keep the payload bounded.
const OPL_CAP: usize = 64;

/// Build the `opl` object for the scene: each entity's resolved Object Property
/// List (0xD6 MegaCliloc) as an array of display lines `{ "<serial>": ["name",
/// "mod1", …], … }`. Each line is `cliloc.format(id, args)` (falls back to `#<id>`
/// when the table is missing or the id is unknown); empty lines are skipped.
/// Resolved here because the scene has the Cliloc table (the core stores raw ids).
/// Capped at [`OPL_CAP`] serials to keep the scene bounded — preferring serials
/// currently in view (mobiles/ground items near the player).
fn opl_json(world: &World, cliloc: Option<&Cliloc>) -> Value {
    let mut map = serde_json::Map::new();
    // Prefer entities the player can actually see: mobiles and ground items.
    let in_view = |s: u32| {
        world.mobiles.contains_key(&s)
            || world.items.get(&s).is_some_and(|it| it.container.is_none())
    };
    let resolve = |lines: &Vec<(u32, String)>| -> Vec<Value> {
        lines
            .iter()
            .filter_map(|(id, args)| {
                let text = cliloc
                    .and_then(|c| c.format(*id, args))
                    .unwrap_or_else(|| format!("#{id}"));
                let text = text.trim();
                if text.is_empty() {
                    None
                } else {
                    Some(Value::String(text.to_string()))
                }
            })
            .collect()
    };
    // Visible serials first, then any remaining, until the cap.
    for (&serial, lines) in world.opl.iter().filter(|(&s, _)| in_view(s)) {
        if map.len() >= OPL_CAP {
            break;
        }
        let resolved = resolve(lines);
        if !resolved.is_empty() {
            map.insert(serial.to_string(), Value::Array(resolved));
        }
    }
    for (&serial, lines) in world.opl.iter().filter(|(&s, _)| !in_view(s)) {
        if map.len() >= OPL_CAP {
            break;
        }
        let resolved = resolve(lines);
        if !resolved.is_empty() {
            map.insert(serial.to_string(), Value::Array(resolved));
        }
    }
    Value::Object(map)
}

/// Build the `trades` array for the scene: every open secure-trade session
/// (0x6F), or `[]` when none — see [`World::trades`]'s doc for why more than
/// one can be open at once (concurrent sessions with different opponents).
/// Items on each side are NOT duplicated here — the client already gets them
/// from `contItems`, filtered by `myCont`/`theirCont` (the trade containers
/// are ordinary container serials), exactly like a normal container window.
/// `myOfferGold`/`myOfferPlat` is what we've offered, `theirOfferGold`/
/// `theirOfferPlat` is the opponent's offer, and `balanceGold`/`balancePlat`
/// is our account balance (an input cap for our own offer, not a trade
/// amount) — see [`crate::world::TradeState`]'s doc for why these three are
/// distinct.
fn trades_json(world: &World) -> Value {
    let trades: Vec<Value> = world
        .trades
        .iter()
        .map(|t| {
            json!({
                "opponent": t.opponent_name,
                "opponentSerial": t.opponent_serial,
                "myCont": t.my_container,
                "theirCont": t.their_container,
                "myAccept": t.my_accept,
                "theirAccept": t.their_accept,
                "myOfferGold": t.my_offer_gold,
                "myOfferPlat": t.my_offer_platinum,
                "theirOfferGold": t.their_offer_gold,
                "theirOfferPlat": t.their_offer_platinum,
                "balanceGold": t.balance_gold,
                "balancePlat": t.balance_platinum,
            })
        })
        .collect();
    Value::Array(trades)
}

/// Build the `book` object for the scene: the open book (0x93/0xD4 header + 0x66
/// pages), or `null` when none. `pages` is an array of pages, each an array of its
/// text lines (empty arrays until the page content arrives).
fn book_json(world: &World) -> Value {
    match &world.book {
        None => Value::Null,
        Some(b) => json!({
            "serial": b.serial,
            "title": b.title,
            "author": b.author,
            "writable": b.writable,
            "pageCount": b.page_count,
            "pages": b.pages,
        }),
    }
}

/// Shallow-merge `fields`'s keys into `v` (both must be JSON objects). Used to
/// splice a pure per-item helper's output into the item loop's `json!` value
/// below without duplicating field names on both sides.
fn merge_obj(v: &mut Value, fields: Value) {
    if let (Value::Object(vm), Value::Object(fm)) = (v, fields) {
        vm.extend(fm);
    }
}

/// Stack/amount fields for a non-corpse ground item's scene JSON entry (see the
/// item loop in [`build_scene`]): `amount` always, `st:1` only when the tile is
/// Stackable — so the renderer's drag-split dialog only offers items the server
/// would actually accept a partial lift from (ClassicUO `GameActions.PickUp`'s
/// `IsStackable` gate). Pure (no Session/MapData), so it's unit-testable directly.
fn stack_fields(amount: u16, stackable: bool) -> Value {
    let mut v = json!({ "amount": amount });
    if stackable {
        v["st"] = json!(1);
    }
    v
}

/// Corpse (graphic 0x2006) scene fields: the dead creature's BODY id rides in the
/// item's `amount` (see `Item::amount`'s doc) and its facing in `direction`;
/// `body`/`hue` here are already Corpse.def-remapped and `death_group` is the
/// primary death-pose animation group. Pure (no Session/MapData), so it's
/// unit-testable directly — see [`build_scene`]'s item loop for the remap/
/// death-group resolution that feeds it.
fn corpse_fields(body: u16, hue: u16, direction: u8, death_group: u8) -> Value {
    json!({ "body": body, "dir": direction, "dg": death_group, "hue": hue })
}

/// `hidden` scene field for a mobile or the player (mobile-update status-flags
/// 0x80 bit — see [`anima_core::world::Mobile::hidden`]'s doc). Only emitted
/// when true (same small-payload convention as the item foliage `"f"` flag),
/// so the renderer's default (not hidden) needs no key at all. Pure, so it's
/// unit-testable directly.
fn hidden_field(hidden: bool) -> Value {
    if hidden {
        json!({ "hidden": true })
    } else {
        json!({})
    }
}

/// `poisoned` scene field for a mobile or the player (mobile-update status-flags
/// 0x04 bit — see [`anima_core::world::Mobile::poisoned`]'s doc). Only emitted
/// when true, same convention as [`hidden_field`], so the renderer's default
/// (health bar colored by HP fraction alone) needs no key at all. Pure, so
/// it's unit-testable directly.
fn poisoned_field(poisoned: bool) -> Value {
    if poisoned {
        json!({ "poisoned": true })
    } else {
        json!({})
    }
}

/// Build the `prompt` object for the scene: an outstanding 0x9A ASCII or 0xC2
/// Unicode server text prompt, or `{"active":0}` when none. The question text itself
/// already arrived as a journal line (see `World::prompt`'s doc) — the client
/// just needs to know a response is due. `promptId` is included alongside
/// `serial` so the client can tell a fresh, server-chained prompt (ServUO
/// commonly sets the next `Prompt` right inside `OnResponse`) apart from a
/// re-poll of the one it's already showing — the two ids together are the
/// prompt's identity, not just `active`'s edge. Pure (no Session), so it's
/// unit-testable directly.
fn prompt_json(world: &World) -> Value {
    match world.prompt {
        Some(p) => json!({
            "active": 1,
            "serial": p.sender_serial,
            "promptId": p.prompt_id,
            "kind": p.kind.as_str(),
        }),
        None => json!({ "active": 0 }),
    }
}

/// Resolve a vendor shop-list name that may be a literal display string OR a
/// bare cliloc id rendered as ASCII digits. ServUO's stock-item naming
/// (`IShopSellInfo.GetNameFor`, `Scripts/VendorInfo/GenericSell.cs`) falls
/// back to `item.LabelNumber.ToString()` — a plain decimal string, no `#` —
/// whenever the item has no explicit `Item.Name`, which is the common case
/// for ordinary stock; a leading `#` is stripped too before parsing, in case
/// some other server variant writes it that way. Cliloc ids for item names
/// run well above any count a real display name could plausibly be (`>=
/// 500_000` — the same heuristic the 0x74 buy-list side already used), so a
/// small numeric string (a name that just happens to be digits) is left
/// alone, and anything at/above it is looked up in the Cliloc table, falling
/// back to `name` **exactly as given** (any leading `#` included) if the
/// table doesn't know it (no table loaded, or an id it doesn't have) — the
/// `#`-stripped form is only used for the numeric parse, never invented into
/// the display text we can't actually confirm.
fn resolve_shop_name(name: &str, cliloc: Option<&Cliloc>) -> String {
    name.strip_prefix('#')
        .unwrap_or(name)
        .parse::<u32>()
        .ok()
        .filter(|&id| id >= 500_000)
        .and_then(|id| cliloc.and_then(|c| c.get(id).map(str::to_string)))
        .unwrap_or_else(|| name.to_string())
}

/// Build the `paperdoll` object for the scene: the latest server-initiated
/// paperdoll open/refresh (0x88), or `null` when none has arrived this
/// session. `seq` lets the renderer treat a fresh request as a "please open"
/// event even for a `serial` it already has a (possibly since-closed) window
/// for — see [`crate::world::Paperdoll`]'s doc for why a repeat matters.
fn paperdoll_json(world: &World) -> Value {
    match &world.paperdoll {
        None => Value::Null,
        Some(p) => json!({
            "seq": p.seq, "serial": p.serial, "title": p.title,
            "warmode": p.warmode, "canLift": p.can_lift,
        }),
    }
}

/// Validated 0xA5 HTTP(S) navigation requests. This bridge only transports the
/// request and its monotonic identity; the browser presents a consent dialog
/// and never navigates merely because an entry appears here.
fn open_urls_json(world: &World) -> Value {
    Value::Array(
        world
            .recent_open_urls
            .iter()
            .map(|request| json!({ "seq": request.seq, "url": request.url }))
            .collect(),
    )
}

/// ServUO 0x24 `gumpId`s that are NOT a container (see
/// [`anima_core::net::game::draw_container`]'s doc for the ServUO/ClassicUO
/// cites): `DisplayBuyList`/`DisplayBuyListHS` (vendor "Buy" window) always
/// writes `0x30`; `DisplaySpellbook`/`DisplaySpellbookHS` always writes
/// `0xFFFF` (`-1` as the wire i16).
const GUMP_ID_VENDOR_BUY: u16 = 0x0030;
const GUMP_ID_SPELLBOOK: u16 = 0xFFFF;

/// Build the `dragCompletions` event ring consumed by the browser's held-item
/// cursor. `token` is null for payload-free 0x29 and the raw four-byte 0x28
/// value otherwise; keeping it raw lets the UI correlate serial-bearing legacy
/// packets without teaching the protocol layer an unverified interpretation.
fn drag_completions_json(world: &World) -> Value {
    Value::Array(
        world
            .recent_drag_completions
            .iter()
            .map(|event| {
                json!({
                    "seq": event.seq,
                    "packet": event.packet,
                    "token": event.token,
                })
            })
            .collect(),
    )
}

/// Latest 0x2C death-screen trigger, separate from `player.dead` (which remains
/// body-derived). The browser uses `seq` to run ClassicUO's 1.5-second banner
/// once; action 2 therefore cannot be misread as a resurrection flag.
fn death_screen_json(world: &World) -> Value {
    match world.death_screen {
        Some(event) => json!({ "seq": event.seq, "action": event.action }),
        None => Value::Null,
    }
}

/// Build the `containerOpens` array: [`World::recent_container_opens`] filtered
/// down to events that are actually a container window. `World` keeps that ring
/// as raw, unfiltered 0x24 data (every `gump_id` ServUO ever sent); deciding
/// which of those ids should make the web client pop a window is a renderer
/// policy call (D3: core = data, renderer = policy), so it happens here, not in
/// `World`. We skip `GUMP_ID_VENDOR_BUY`/`GUMP_ID_SPELLBOOK` — a vendor's Buy
/// list is already surfaced via `shop`/0x74/0x3B, and a spellbook via
/// `spellbooks`/0xBF/0x1B, so re-showing either as a bare generic Container
/// window here would be a spurious empty duplicate (live-reproduced: opening a
/// vendor's Buy list otherwise pushed the vendor's own MOBILE serial into this
/// ring as if it were a container).
fn container_opens_json(world: &World) -> Value {
    let opens: Vec<Value> = world
        .recent_container_opens
        .iter()
        .filter(|&&(_, _, gump_id)| gump_id != GUMP_ID_VENDOR_BUY && gump_id != GUMP_ID_SPELLBOOK)
        .map(|&(seq, serial, _)| json!({ "seq": seq, "serial": serial }))
        .collect();
    Value::Array(opens)
}

/// Build the `maps` array: every open treasure/decoration map window
/// (0x90/0xF5 DisplayMap(New) + 0x56 MapCommand — [`anima_core::world::
/// MapView`]), sorted by serial for a stable order (the source is a
/// `HashMap`). `openSeq` bumps on every 0x90/0xF5 for that serial, even a
/// byte-identical resend (see [`anima_core::world::MapView::open_seq`]'s
/// doc) — the web client opens a window only when it sees a NEW `openSeq`
/// for a serial (mirrors `paperdoll`'s `seq`/`containerOpens`' ring), so a
/// user-closed map window doesn't pop back open on every poll. `pins` are
/// `[x, y]` pairs already in `w`×`h` PIXEL space (ServUO converts
/// bounds↔pixel server-side before a pin ever hits the wire — see
/// `MapView`'s doc) — the renderer draws each one straight onto the map art
/// with no rescale.
fn maps_json(world: &World) -> Value {
    let mut maps: Vec<(&u32, &anima_core::world::MapView)> = world.map_gumps.iter().collect();
    maps.sort_by_key(|&(serial, _)| *serial);
    let maps: Vec<Value> = maps
        .iter()
        .map(|&(serial, mv)| {
            json!({
                "serial": serial, "openSeq": mv.open_seq, "gumpArt": mv.gump_art, "facet": mv.facet,
                "bounds": { "minX": mv.min_x, "minY": mv.min_y, "maxX": mv.max_x, "maxY": mv.max_y },
                "w": mv.width, "h": mv.height,
                "pins": mv.pins.iter().map(|&(x, y)| json!([x, y])).collect::<Vec<_>>(),
                "editable": mv.editable,
            })
        })
        .collect();
    Value::Array(maps)
}

/// Cap on the deduped `(dx, dy)` footprint tiles emitted for a pending 0x99
/// placement preview — a house's raw component list runs into the hundreds,
/// but the outline only needs each distinct offset once, so this keeps the
/// field a few KB instead of dumping every component/graphic/z.
const PLACEMENT_TILE_CAP: usize = 4096;

/// Cap on the raw (non-deduped) `parts` component list emitted alongside
/// `tiles` (see [`placement_json`]) — unlike `tiles`, every component is kept
/// (walls/roof/floors all stack on the same footprint tile), so a castle's
/// full multi.mul list is the realistic worst case rather than the tile
/// count. 2000 comfortably covers that while still bounding the payload.
const PLACEMENT_PART_CAP: usize = 2000;

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
fn placement_json(world: &World, multis: Option<&Multis>) -> Option<Value> {
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
fn house_design_max_levels(multis: Option<&Multis>, multi_id: u32) -> u8 {
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
fn house_design_json(world: &World, multis: Option<&Multis>) -> Option<Value> {
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
fn ensure_house_tiles(world: &mut World, multis: &Multis) {
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
fn emit_multi_component(
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
        lights.push(json!({ "x": x, "y": y, "z": cz, "r": 3 }));
    }
}

/// Serialize the current world + a map window (walkability/Z + real terrain
/// color) + entities + journal to the JSON the web renderer consumes.
#[allow(clippy::too_many_arguments)]
pub fn build_scene(
    s: &mut Session,
    map: Option<&mut MapData>,
    mut art: Option<&mut Art>,
    cliloc: Option<&Cliloc>,
    animdata: Option<&AnimData>,
    anim: Option<&Anim>,
    multis: Option<&Multis>,
    journal: &[Value],
) -> String {
    // Decode any newly-arrived custom-house designs before anything below reads
    // `s.world` — needs `multis` for the foundation's `multi.mul` bounds, so it's
    // a no-op (and every design stays `tiles_ready == false`, rendering as the
    // stock foundation) when the caller has no `Multis` loaded at all.
    if let Some(m) = multis {
        ensure_house_tiles(&mut s.world, m);
    }
    // `Body.def` remap (ClassicUO ReplaceBody): redirect an exotic body to its real
    // animation body so the renderer picks the right group + resolves a sprite. The
    // mobile's own hue wins; Body.def's hue is only a fallback for base creatures.
    let remap = |body: u16, hue: u16| -> (u16, u16) {
        let (rbody, rhue) = anim.map_or((body, 0), |a| a.remap(body));
        (rbody, if hue != 0 { hue } else { rhue })
    };
    // Authoritative animation group kind (0 monster, 1 animal, 2 people) for the
    // (already Body.def-remapped) body: `mobtypes.txt` via `Anim`, else the raw range
    // heuristic. Sent as `at` so the renderer picks group numbers that match the file
    // layout the reader uses (an animal's stand is group 2, a monster's is group 1).
    let atype = |body: u16| -> u8 {
        anim.map_or_else(
            || (body >= 200) as u8 + (body >= 400) as u8,
            |a| a.anim_type(body),
        )
    };
    // `Corpse.def` remap (ClassicUO ReplaceCorpse): the SAME idea as `remap` above,
    // but a separate table applied to a corpse item's body (which travels in the
    // item's `amount` field — ClassicUO `Item.GetGraphicForAnimation`'s `IsCorpse`
    // special case). The corpse's own hue still wins over Corpse.def's fallback.
    let remap_corpse = |body: u16, hue: u16| -> (u16, u16) {
        let (rbody, rhue) = anim.map_or((body, 0), |a| a.remap_corpse(body));
        (rbody, if hue != 0 { hue } else { rhue })
    };
    let p = s.world.player_mobile().cloned().unwrap_or_default();
    let st = &s.world.player_stats;
    let mounted = s.world.player_mounted();
    let (px, py, pz) = (p.pos.x as i64, p.pos.y as i64, p.pos.z as i32);

    // Roof/ceiling cull bound (ClassicUO UpdateMaxDrawZ), computed up front so BOTH
    // the ground-items and statics emissions can hide anything at/above it. Without
    // this on items, a field/object sitting on the mountain surface above a cave
    // (or furniture on a hidden upper floor) renders floating over the black void.
    let mut map = map;
    let max_z = match map {
        Some(ref mut m) => max_draw_z(&s.world, m, multis, px, py, pz),
        None => 127i32,
    };

    // Worn equipment's AnimID (the sprite to fetch via `/anim`) comes from tiledata
    // on the map. `map` is consumed by the tile loop below, so resolve anims through
    // this shared-borrow helper while it's still available (0 when there's no map).
    let item_anim = |g: u16| map.as_deref().map_or(0u16, |m| m.item_anim(g));
    // `Equipconv.def` override (ClassicUO `EquipConversions[body][item.AnimID]`,
    // consulted by `MobileView`/`ItemView` for the world sprite and
    // `PaperDollInteractable.GetAnimID` for the paperdoll): given the wearer's
    // REMAPPED `body` and an item's tiledata `base_anim`, return the replacement
    // `(anim graphic, paperdoll gump id, hue)`. `anim` is always overridden when a
    // conversion exists (ClassicUO's unconditional `graphic = data.Graphic`);
    // `gump` is `Some` only then (`None` ⇒ caller keeps its own `anim + gender
    // offset` paperdoll convention); `hue` is the item's own hue, falling back to
    // the conversion's hue only when the item has none (ClassicUO:
    // `if (hue == 0 && _equipConvData.HasValue) hue = _equipConvData.Value.Color`).
    let equip_conv = |body: u16, base_anim: u16, item_hue: u16| -> (u16, Option<u16>, u16) {
        match anim.and_then(|a| a.equip_conv(body, base_anim)) {
            Some(ec) => (
                ec.graphic,
                Some(equip_conv_gump(body, ec.gump)),
                if item_hue != 0 { item_hue } else { ec.hue },
            ),
            None => (base_anim, None, item_hue),
        }
    };
    // Does an item graphic emit light (torch/lamp/brazier)? Resolved through the
    // shared borrow before `map` is consumed by the tile loop below.
    let item_is_light = |g: u16| map.as_deref().is_some_and(|m| m.item_is_light(g));
    // Does an item graphic carry the Foliage flag (tree/bush)? Used so the renderer
    // can fade it when it would occlude the player. Resolved through the shared
    // borrow before `map` is consumed by the tile loop below.
    let item_foliage = |g: u16| {
        map.as_deref()
            .is_some_and(|m| m.item_flags(g) & FLAG_FOLIAGE != 0)
    };
    // "nodraw" void-placeholder items (name starts "nodraw", e.g. graphic 0x1 staff
    // spawner/markers): ClassicUO culls these for items just like statics — without
    // this the "NO DRAW" placeholder bitmap shows on the ground for GM characters.
    let item_nodraw = |g: u16| map.as_deref().is_some_and(|m| m.item_is_nodraw(g));
    // Container (chest/bag/corpse 0x2006) → the client opens a loot window on
    // double-click; non-containers (doors, etc.) must NOT spawn an empty window.
    let item_is_cont =
        |g: u16| g == 0x2006 || map.as_deref().is_some_and(|m| m.item_is_container(g));
    // STACKABLE tiledata — the split-stack dialog should only ever offer to split
    // an item the server would actually accept a partial amount from.
    let item_stackable = |g: u16| {
        map.as_deref()
            .is_some_and(|m| m.item_flags(g) & FLAG_STACKABLE != 0)
    };
    // Draw-sort priority for a dynamic item (same scheme as statics): base z, with
    // a background tile under, and a tile with height (a wall/door) over, same-tile flats.
    let item_pz = |g: u16, z: i32| -> i32 {
        map.as_deref().map_or(z, |m| {
            let mut pz = z;
            if m.item_flags(g) & 0x1 != 0 {
                pz -= 1; // Background
            }
            if m.item_height(g) != 0 {
                pz += 1; // has height (door/wall/solid)
            }
            pz
        })
    };
    // `h`/`pf` for a dynamic item within `PATH_RADIUS` of the player: now that
    // a dynamic item can contribute a standing surface (a boat deck — see
    // `dynamic_statics_at`'s doc), the browser's own `calculateNewZ` port
    // needs the SAME tiledata bits a nearby static already carries (see
    // `path_suffix`'s doc), or it keeps resolving the water's Z under a deck
    // instead of the deck's own. Shares `path_bits` with `path_suffix` so the
    // two can never derive different bits for the same graphic.
    let item_path_bits = |g: u16, ix: i64, iy: i64| -> (Option<u8>, Option<u8>) {
        let in_radius = (ix - px).abs() <= PATH_RADIUS && (iy - py).abs() <= PATH_RADIUS;
        map.as_deref()
            .map_or((None, None), |m| path_bits(in_radius, m.item_height(g), m.item_flags(g)))
    };

    let mobiles: Vec<Value> = s
        .world
        .mobiles
        .values()
        .filter(|m| m.serial != p.serial)
        .map(|m| {
            let (body, hue) = remap(m.body, m.hue);
            // Only "people" bodies (>= 400) wear clothes/hair/beard; animals and
            // monsters carry nothing, so skip the per-item work for them.
            let equip: Vec<Value> = if body >= 400 {
                s.world
                    .items
                    .values()
                    .filter(|it| it.container == Some(m.serial) && it.layer != 0)
                    .map(|it| {
                        let (a, gump, hue) = equip_conv(body, item_anim(it.graphic), it.hue);
                        let mut v = json!({
                            "serial": it.serial, "layer": it.layer, "g": it.graphic,
                            "anim": a, "hue": hue
                        });
                        if let Some(g) = gump {
                            v["gump"] = json!(g);
                        }
                        v
                    })
                    .collect()
            } else {
                Vec::new()
            };
            // A mounted mobile wears a "mount item" on layer 25 (0x19); its tiledata
            // AnimID IS the mount's animal body. Resolve it (0 = not mounted) so the
            // renderer can draw the mount under the rider with the ONMOUNT groups.
            let mount = s
                .world
                .items
                .values()
                .find(|it| it.container == Some(m.serial) && it.layer == 25);
            let mount_anim = mount.map_or(0u16, |it| mount_anim_for(it.graphic, &item_anim));
            let mut v = json!({
                "serial": m.serial,
                "x": m.pos.x, "y": m.pos.y, "z": m.pos.z, "dir": m.direction,
                "body": body, "at": atype(body), "noto": m.notoriety, "name": m.name,
                "hits": m.hits, "hitsMax": m.hits_max,
                "hue": hue, "equip": equip,
                "mounted": mount.is_some() as u8, "mountAnim": mount_anim
            });
            merge_obj(&mut v, hidden_field(m.hidden));
            merge_obj(&mut v, poisoned_field(m.poisoned));
            v
        })
        .collect();
    let items: Vec<Value> = s
        .world
        .items
        .values()
        .filter(|it| {
            // Same z-ceiling rule the statics loop applies: at/above max_z is
            // hidden (roof lifted / cave ceiling), so no floating items. A multi
            // (`is_multi`) isn't a drawable item at all — its `graphic` is a
            // multi id, not an ART graphic; it's expanded into the statics
            // stream (see the tile loop below) instead of drawn directly here.
            !it.is_multi
                && it.container.is_none()
                && !item_nodraw(it.graphic)
                && (it.pos.z as i32) < max_z
        })
        .map(|it| {
            let mut v = json!({
                "x": it.pos.x, "y": it.pos.y, "z": it.pos.z, "g": it.graphic,
                "serial": it.serial, "pz": item_pz(it.graphic, it.pos.z as i32)
            });
            // Mark foliage so the renderer can fade it (only when true, small payload).
            if item_foliage(it.graphic) {
                v["f"] = json!(1);
            }
            // Mark containers so double-click opens a loot window (doors don't).
            if item_is_cont(it.graphic) {
                v["c"] = json!(1);
            }
            // `h`/`pf` (PATH_RADIUS-gated, see `item_path_bits`'s doc): omitted
            // whenever out of radius or zero, so this is purely additive —
            // an item outside PATH_RADIUS serializes exactly as before.
            let (h, pf) = item_path_bits(it.graphic, it.pos.x as i64, it.pos.y as i64);
            if let Some(h) = h {
                v["h"] = json!(h);
            }
            if let Some(pf) = pf {
                v["pf"] = json!(pf);
            }
            // Stack count, so the renderer's pointer-drag can offer a stack-split
            // dialog when lifting amount > 1 (ClassicUO SplitMenuGump). Omitted for
            // a corpse (graphic 0x2006): its `amount` is overloaded with the dead
            // creature's BODY id below, not a real stack size, and a corpse can't
            // be picked up/split like an ordinary item anyway.
            if it.graphic != 0x2006 {
                merge_obj(&mut v, stack_fields(it.amount, item_stackable(it.graphic)));
            }
            // A corpse (graphic 0x2006): the dead creature's BODY id rides in
            // `amount` (see `Item::amount`'s doc comment) and its facing in
            // `direction`. Remap through Corpse.def, resolve the primary death-pose
            // group, and hand the renderer everything it needs to draw the real
            // death-pose sprite instead of the generic corpse art.
            if it.graphic == 0x2006 {
                let (body, hue) = remap_corpse(it.amount, it.hue);
                let dg = anim.map_or(0, |a| a.death_group(body));
                merge_obj(&mut v, corpse_fields(body, hue, it.direction, dg));
            }
            v
        })
        .collect();
    // Per-object light sources for the renderer's night glow. The player always
    // carries a personal/held light (r:5) so the avatar stays visible at night;
    // each dynamic world item with the LightSource tile flag adds a smaller glow
    // (r:3). Static light sources (wall torches, lamps) are appended in the tile
    // loop below. Capped (~64) to keep the glow pass cheap.
    const LIGHT_CAP: usize = 64;
    let mut lights: Vec<Value> = Vec::new();
    lights.push(json!({ "x": px, "y": py, "z": pz, "r": 5 }));
    for it in s.world.items.values() {
        if lights.len() >= LIGHT_CAP {
            break;
        }
        // A multi's own entry carries a multi id in `graphic`, not an ART
        // graphic — skip it here (any light-emitting components are handled
        // per-component in the tile loop below, alongside static lights).
        if !it.is_multi && it.container.is_none() && item_is_light(it.graphic) {
            lights.push(json!({ "x": it.pos.x, "y": it.pos.y, "z": it.pos.z, "r": 3 }));
        }
    }
    // The player's worn items (container == us, on a real layer) drive the
    // paperdoll. Layer 0 = not equipped; the backpack itself is layer 0x15.
    // `Equipconv.def` is keyed by the wearer's REMAPPED body (same as the mobiles
    // loop above), computed once here for every worn item.
    let (equip_body, _) = remap(p.body, p.hue);
    let equip: Vec<Value> = s
        .world
        .items
        .values()
        .filter(|it| it.container == Some(p.serial) && it.layer != 0)
        .map(|it| {
            let (a, gump, hue) = equip_conv(equip_body, item_anim(it.graphic), it.hue);
            let mut v = json!({
                "serial": it.serial, "g": it.graphic, "layer": it.layer,
                "anim": a, "hue": hue
            });
            if let Some(g) = gump {
                v["gump"] = json!(g);
            }
            v
        })
        .collect();
    // The player's mount item (layer 25) AnimID — the animal body to draw under the
    // rider when mounted (0 = on foot). Resolved here (before `map` is consumed by
    // the tile loop) like the per-mobile mounts.
    let player_mount_anim = s
        .world
        .items
        .values()
        .find(|it| it.container == Some(p.serial) && it.layer == 25)
        .map_or(0u16, |it| mount_anim_for(it.graphic, &item_anim));
    // Every contained item (in any container), so the client can open a
    // backpack/container window by filtering on `cont`. x/y are grid coords
    // inside the container, not world tiles. Capped to keep the scene bounded.
    let cont_items: Vec<Value> = s
        .world
        .items
        .values()
        .filter(|it| it.container.is_some())
        .take(400)
        .map(|it| {
            let mut v = json!({
                "serial": it.serial, "cont": it.container,
                "g": it.graphic, "amount": it.amount,
                "x": it.pos.x, "y": it.pos.y, "hue": it.hue,
                // Is this nested item itself a container? Only then should a
                // double-click open a container window (bandages/potions/etc. must not).
                "c": item_is_cont(it.graphic) as u8
            });
            // Mark stackable so a dragged stack only offers the split dialog when
            // the server would actually accept a partial amount (only when true).
            if item_stackable(it.graphic) {
                v["st"] = json!(1);
            }
            v
        })
        .collect();
    // Vendor shop windows. `buy` (0x74) lists the vendor's for-sale prices in
    // packet order — the renderer matches them to that container's `contItems` by
    // index. `sell` (0x9E) lists the items in our pack the vendor will buy. Either
    // may be present; `shop` is null when no vendor window is open.
    let shop_buy = s.world.shop_buy.as_ref().map(|b| {
        let prices: Vec<Value> = b
            .entries
            .iter()
            .map(|e| {
                // ServUO sends cliloc-named stock as the bare numeric cliloc id; resolve
                // it to the real item name (e.g. 1060834 → "a hatchet"). The item fields
                // (serial/graphic/amount) are paired in by 0x3C arrival order in
                // anima-core's `open_buy_window`.
                json!({
                    "price": e.price, "name": resolve_shop_name(&e.name, cliloc),
                    "serial": e.serial, "graphic": e.graphic, "amount": e.amount,
                })
            })
            .collect();
        json!({ "vendor": b.vendor, "cont": b.container, "prices": prices })
    });
    let shop_sell = s.world.shop_sell.as_ref().map(|sl| {
        let items: Vec<Value> = sl
            .items
            .iter()
            .map(|it| {
                // Same cliloc-shaped-name resolution as the buy side above — ServUO's
                // `IShopSellInfo.GetNameFor` falls back to a bare numeric LabelNumber
                // string for stock with no explicit `Item.Name` (see
                // `resolve_shop_name`'s doc), which otherwise showed as a raw id.
                json!({
                    "serial": it.serial, "g": it.graphic, "amount": it.amount,
                    "price": it.price, "name": resolve_shop_name(&it.name, cliloc)
                })
            })
            .collect();
        json!({ "vendor": sl.vendor, "items": items })
    });
    let shop = if shop_buy.is_none() && shop_sell.is_none() {
        Value::Null
    } else {
        json!({ "buy": shop_buy, "sell": shop_sell })
    };
    let shop = serde_json::to_string(&shop).unwrap_or_else(|_| "null".into());

    // Targeting UI state: is the server waiting for a target, and is it an
    // object (kind 0) or ground (kind 1) cursor?
    let target = match s.world.pending_target {
        Some(tc) => json!({ "active": 1, "kind": tc.target_type }),
        None => json!({ "active": 0, "kind": 0 }),
    };
    // Pending 0x99 house/multi placement footprint, if any — see
    // `placement_json`'s doc. Formatted below into an optional (possibly
    // empty) fragment so an idle scene keeps serializing byte-identical to
    // before this field existed.
    let placement = placement_json(&s.world, multis);
    // House-designer editor snapshot, if we're currently customizing a house —
    // see `house_design_json`'s doc. Same "omit, don't null" convention as
    // `placement` right above.
    let house_design = house_design_json(&s.world, multis);

    // tiles/statics are the bulk (≈1225 + hundreds): serialize them straight into
    // String buffers instead of building serde_json::Value trees + re-walking them
    // in to_string(). That `Value` round-trip was ~31ms/build and blocked the game
    // loop (movement pacing + net pump) → periodic stutter. No string fields here,
    // so manual JSON is safe; the small parts below still go through serde.
    let mut tiles = String::with_capacity(64 * 1024);
    let mut statics = String::with_capacity(16 * 1024);
    let mut n_statics = 0usize;
    let mut dbg: Vec<Value> = Vec::new();
    if let Some(map) = map {
        // `max_z` (computed up front, see the top of this fn) hides the roof /
        // upper floors when the player is under cover (ClassicUO UpdateMaxDrawZ):
        // statics at/above it aren't sent, revealing the interior.
        // Under cover? Then (like ClassicUO `_noDrawRoofs`) hide *every* roof tile
        // in view, not only those above max_z — so the whole roof lifts off.
        let under_cover = max_z < 127;
        // Authoritative sz for a WIDER neighbourhood than just the 8 immediate
        // neighbours, resolved by chaining `calculate_new_z` hop-by-hop outward
        // (BFS by Chebyshev shell) from the player's own confirmed Z, instead of
        // a single hop from `pz`. Fixes a real misprediction: the cheap fallback
        // below (`best` nearest-to-`pz` within ±16) picks the candidate closest
        // to the CURRENT (already-passed) player Z, which on genuinely varied
        // terrain silently prefers the wrong static/land Z once you're a few
        // tiles further along a climb — verified live at (1420,1702): standing
        // at (1428,1702,2), the cheap hint for (1432,1702) was `9`; actually
        // walking there (ServUO-confirmed) lands at `12`. That 3-unit miss is
        // what the browser predicted, committed to, and only found out about
        // after the fact — the reported "flat, then pops up" bug. Chaining the
        // real per-hop calc from each already-resolved neighbour (its Chebyshev
        // predecessor, walking straight back toward the player) instead of a
        // single far hop keeps every step of the chain as accurate as the
        // existing radius-1 case. `CHAIN_RADIUS` must cover the browser's
        // `LEAD_CAP` (3.5 — how far prediction runs ahead of the confirmed
        // server position, web/main.js) PLUS the glide's own destination tile
        // and diagonal slack, so a mid-glide `tileSZ()` re-read NEVER lands on
        // a cheap-hint tile: the two calculators disagree by ±1 on slopes, and
        // a destination tile flipping between them across polls (as the chain
        // origin follows the player) re-targets the browser's Z ease every
        // poll — the "height keeps re-adjusting" bobbing verified live around
        // (1485,1600), where tiles at Chebyshev distance 4↔5 toggled sz 10↔11.
        // Radius 6 = ceil(LEAD_CAP) + 1 (in-flight step) + 1 margin. Cost:
        // ~168 `calculate_new_z` calls per build (vs 80 at radius 4) over
        // tiles already fetched for rendering — still no extra map I/O.
        const CHAIN_RADIUS: i64 = 6;
        const CHAIN_SPAN: usize = (2 * CHAIN_RADIUS as usize) + 1;
        let chain_idx = |ddx: i64, ddy: i64| -> usize {
            ((ddy + CHAIN_RADIUS) as usize) * CHAIN_SPAN + (ddx + CHAIN_RADIUS) as usize
        };
        let mut sz_chain: Vec<Option<i32>> = vec![None; CHAIN_SPAN * CHAIN_SPAN];
        sz_chain[chain_idx(0, 0)] = Some(pz);
        for shell in 1..=CHAIN_RADIUS {
            for ddy in -shell..=shell {
                for ddx in -shell..=shell {
                    if ddx.abs().max(ddy.abs()) != shell {
                        continue; // this shell's ring only
                    }
                    // Predecessor: one Chebyshev shell closer to the player,
                    // walking straight back toward them on both axes — already
                    // resolved by an earlier (smaller) shell iteration.
                    let (pdx, pdy) = (ddx - ddx.signum(), ddy - ddy.signum());
                    let Some(Some(pz_hop)) = sz_chain.get(chain_idx(pdx, pdy)).copied() else {
                        continue;
                    };
                    let Some(dir) = dir_from_delta(ddx - pdx, ddy - pdy) else {
                        continue;
                    };
                    if let Some(z) =
                        calculate_new_z(&s.world, map, multis, px + ddx, py + ddy, pz_hop, dir)
                    {
                        sz_chain[chain_idx(ddx, ddy)] = Some(z);
                    }
                }
            }
        }
        // Multis (boats/houses) within the window + a margin big enough for the
        // furthest real component (26 tiles, verified against the real
        // multi.mul — see `anima_assets::multis`'s module doc) so a multi whose
        // ORIGIN sits just outside the window can still have components drawn
        // over/walked over just inside it. Resolved once per scene build (not
        // per tile) as `(x, y, z, multi_id, serial)` — the serial rides along so
        // the per-tile loop below can look up a decoded custom-house design and
        // swap it in for `multis.components_at`; `Multis::components_at`'s own
        // per-multi cache then makes each tile's lookup below O(components on
        // that ONE tile), not O(components on the whole multi).
        const MULTI_MARGIN: i64 = 32;
        let near_multis: Vec<(i64, i64, i32, u32, u32)> = if multis.is_some() {
            s.world
                .items
                .iter()
                .filter(|(_, it)| {
                    it.is_multi
                        && (it.pos.x as i64 - px).abs() <= RADIUS + MULTI_MARGIN
                        && (it.pos.y as i64 - py).abs() <= RADIUS + MULTI_MARGIN
                })
                .map(|(serial, it)| {
                    (
                        it.pos.x as i64,
                        it.pos.y as i64,
                        it.pos.z as i32,
                        it.graphic as u32,
                        *serial,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        // DEBUG: statics above the player on this tile (to diagnose roof hiding).
        if px >= 0 && py >= 0 {
            for s in map.statics(px as u32, py as u32) {
                if (s.z as i32) > pz {
                    dbg.push(json!({
                        "z": s.z, "g": s.graphic,
                        "roof": s.flags & 0x1000_0000 != 0,
                        "surf": s.flags & 0x200 != 0,
                    }));
                }
            }
        }
        // Computed once (doesn't vary per tile) for the `walk` chain-radius
        // fast path below — see `blocking_item_at`'s doc.
        let ghost = player_is_ghost(&s.world);
        for dy in -LAND_RADIUS..=LAND_RADIUS {
            for dx in -LAND_RADIUS..=LAND_RADIUS {
                let (x, y) = (px + dx, py + dy);
                if x < 0 || y < 0 {
                    tiles.push_str(
                        "{\"w\":0,\"z\":0,\"g\":0,\"tx\":0,\"c\":[10,10,12],\"h\":0,\"sz\":0},",
                    );
                    continue;
                }
                // Past the actual view range this tile is land-only context (the
                // grayed-out ring the client renders) — no static fetch, no static
                // emission, no multi components. Cheaper AND land-only by
                // construction.
                //
                // Inside `CHAIN_RADIUS`, `w` must agree with `sz` instead of
                // asking `tile_walkable` (`walkable_z_explain`'s scorer) again:
                // the two calculators disagree on a bridge-widened climb (a
                // house foundation's stairs is a live, verified case — see
                // `blocking_item_at`'s doc), and ClassicUO has no such split —
                // `Pathfinder.CanWalk` *is* `CalculateNewZ`. `sz_chain` already
                // holds that authoritative answer (`Some` iff `calculate_new_z`
                // found a standing surface); blockers (crates, closed doors,
                // hull/house walls) still deny exactly as `tile_walkable` would,
                // via the SAME rules `explain_tile_walkable` uses (factored out
                // as `blocking_item_at` so they can't drift apart) — but checked
                // against the tile's OWN resolved `sz_chain` Z, not `pz`: a
                // dynamic/multi blocker's span is judged against the body's
                // ACTUAL height once the step completes, exactly the height
                // `calculate_new_z` just resolved, not the height it's leaving
                // behind. This matters for the very climb this fix is about — a
                // foundation's stairs at `pz=2` (a Bridge) stepping onto the
                // plot at `sz=7`: the plot's own impassable riser (`0x0063`, z
                // 0..5) genuinely overlaps a body still down at z=2 (verified:
                // `item_blocks` says so), which is exactly why the OLD `w=0`
                // check (using `pz`) also denied through `multi_blocker_at`, not
                // just `walkable_z_explain`'s scoring — but that same riser is
                // well clear of a body actually standing at z=7, matching the
                // real, live-confirmed outcome. Outside the chain radius there's
                // no authoritative answer to defer to, so this keeps exactly
                // today's `tile_walkable` result.
                let walk = if dx.abs() <= CHAIN_RADIUS && dy.abs() <= CHAIN_RADIUS {
                    match sz_chain[chain_idx(dx, dy)] {
                        Some(z) => {
                            let dyn_items = dynamic_statics_at(&s.world, map, x, y);
                            blocking_item_at(&s.world, map, multis, x, y, z, &dyn_items, ghost)
                                .is_none()
                        }
                        None => false,
                    }
                } else {
                    tile_walkable(&s.world, map, multis, x, y, pz)
                };
                let land = map.land(x as u32, y as u32);
                let c = art
                    .as_mut()
                    .map(|a| a.land_avg_color(land.graphic))
                    .unwrap_or([60, 90, 50, 255]);
                // ClassicUO Land rule: hide terrain above the ceiling so the floor
                // below shows — e.g. the surface (z=0) over a basement. We keep z so
                // the renderer can still use it for neighbour slope corners.
                let hidden = (land.z as i32) > max_z;
                // Statics (trees/walls/etc.) are emitted across the whole land
                // window, including the beyond-view gray ring — the client grays
                // them there (they're part of "the map you can see", just no live
                // detail).
                let tstatics = map.statics(x as u32, y as u32);
                // Standing Z hint if the player steps onto this tile — the surface
                // or bridge (stair) nearest the current Z within one step. This is
                // a *cheap* approximation of CalculateNewZ (the faithful version in
                // play.rs is authoritative); it only reads tiles we already fetched
                // (no per-tile map re-clone, which made the full flood ~40ms/build).
                // The renderer uses it to raise/lower Z in lock-step with X/Y so a
                // stair climbs *during* the glide instead of popping a poll later.
                let sz = if dx == 0 && dy == 0 {
                    pz // the tile we're already standing on
                } else if walk {
                    let g = land.graphic;
                    // Land counts as a surface unless it's a "no-draw" hole graphic.
                    let land_surface = !land.impassable()
                        && ((g < 0x01AE && g != 2) || (g > 0x01B5 && g != 0x01DB));
                    let mut best = if land_surface {
                        Some(land.z as i32)
                    } else {
                        None
                    };
                    for st in &tstatics {
                        let bridge = st.flags & FLAG_BRIDGE != 0;
                        let surface = st.flags & FLAG_SURFACE != 0;
                        if (surface || bridge) && st.flags & FLAG_IMPASSABLE == 0 {
                            let h = st.height as i32;
                            let stand = st.z as i32 + if bridge { h / 2 } else { h };
                            if (stand - pz).abs() <= 16
                                && best.is_none_or(|b| (stand - pz).abs() < (b - pz).abs())
                            {
                                best = Some(stand);
                            }
                        }
                    }
                    best.unwrap_or(land.z as i32)
                } else {
                    land.z as i32 // unwalkable → terrain baseline
                };
                // For tiles within CHAIN_RADIUS of the player, replace the cheap
                // hint with the AUTHORITATIVE chained CalculateNewZ computed above
                // (the same math the server uses, hop-by-hop from the player's own
                // Z). This makes the climb prediction exact well past the
                // immediate neighbours, so a stair/ramp/slope rises *during* the
                // glide instead of the avatar sliding flat then popping up a poll
                // later (see `sz_chain`'s doc above for the live-verified miss this
                // fixes).
                let sz = if dx.abs() <= CHAIN_RADIUS && dy.abs() <= CHAIN_RADIUS {
                    sz_chain[chain_idx(dx, dy)].unwrap_or(sz)
                } else {
                    sz
                };
                // `li` (land impassable): PATH_RADIUS-gated, like the static `h`/`pf`
                // above — the browser's `calculate_new_z` port needs to know the LAND
                // itself can't be stood on before it'll fall back to a static surface.
                let land_impassable = if dx.abs() <= PATH_RADIUS
                    && dy.abs() <= PATH_RADIUS
                    && land.impassable()
                {
                    ",\"li\":1"
                } else {
                    ""
                };
                // `dr` (door to open): PATH_RADIUS-gated like `li` above, and
                // only even worth computing for a tile that ISN'T strictly
                // walkable (`walk == false` — a walkable tile has no door to
                // open). Reuses `tile_walkable_for_planning`'s exact "every
                // blocker is a door" rule (via
                // `explain_tile_walkable_for_planning`) instead of
                // re-deriving it, so this can never disagree with what the
                // click-to-walk route planner already treats as passable.
                // See `door_suffix`'s doc for why this field exists:
                // ClassicUO opens a door you walk INTO (`TryOpenDoors`) — the
                // browser's manual (keyboard) walking needs this serial to do
                // the same, instead of just stopping at the strict `w=0`.
                let door_to_open = if !walk && dx.abs() <= PATH_RADIUS && dy.abs() <= PATH_RADIUS {
                    explain_tile_walkable_for_planning(&s.world, map, multis, x, y, pz).1
                } else {
                    None
                };
                let _ = write!(
                    tiles,
                    "{{\"w\":{},\"z\":{},\"g\":{},\"tx\":{},\"c\":[{},{},{}],\"h\":{},\"sz\":{}{}{}}},",
                    walk as u8,
                    land.z,
                    land.graphic,
                    land.tex_id,
                    c[0],
                    c[1],
                    c[2],
                    hidden as u8,
                    sz,
                    land_impassable,
                    door_suffix(door_to_open)
                );
                // Static objects on this tile (walls/trees/deco). Skip anything at
                // or above max_z so a roof/upper floor over the player vanishes.
                // Emitted across the whole land window; the client grays the ones
                // beyond the view range.
                if n_statics < 4000 {
                    for s in &tstatics {
                        // "nodraw" void placeholders (tiledata name starts "nodraw",
                        // e.g. graphic 8600 whose art is a literal "NO DRAW" bitmap):
                        // ClassicUO culls them (GameObject.cs) — if we drew them the
                        // placeholder would show on the terrain. Detected by tiledata
                        // NAME, not a flag (8600 carries no NoDraw flag bit).
                        if map.item_is_nodraw(s.graphic) {
                            continue;
                        }
                        let is_roof = s.flags & 0x1000_0000 != 0;
                        if (s.z as i32) >= max_z || (under_cover && is_roof) {
                            continue;
                        }
                        // Draw-sort priority (ClassicUO Chunk.AddGameObject): a tall
                        // object (height != 0, e.g. a wall) sorts above same-tile
                        // flats (floors); a background tile sorts below. Renderer
                        // uses `pz` so a floor draws under the wall on its tile.
                        let mut spz = s.z as i32;
                        if s.flags & 0x1 != 0 {
                            spz -= 1; // Background
                        }
                        if s.height != 0 {
                            spz += 1; // has height (wall/solid)
                        }
                        // Foliage (trees/bushes) get an `f` flag so the renderer fades
                        // them when they'd hide the player. Only emit when true.
                        let foliage = if s.flags & FLAG_FOLIAGE != 0 {
                            ",\"f\":1"
                        } else {
                            ""
                        };
                        // Animated statics (flames/fountains/water wheels) flagged
                        // `TileFlag.Animation` cycle through ART tiles from animdata.mul.
                        // Bake the frame tile-id sequence (`a`) + per-frame interval in
                        // ms (`ai`) so the renderer just swaps textures. Only emit when
                        // the tile is animated AND animdata gives more than one frame.
                        let anim = anim_suffix(map, animdata, s.graphic);
                        // `h`/`pf` (PATH_RADIUS-gated): `s.height`/`s.flags` are already
                        // fetched by `map.statics()` above, so this is free — see
                        // `path_suffix`'s doc.
                        let path = path_suffix(
                            dx.abs() <= PATH_RADIUS && dy.abs() <= PATH_RADIUS,
                            s.height,
                            s.flags,
                        );
                        let _ = write!(
                            statics,
                            "{{\"x\":{},\"y\":{},\"z\":{},\"g\":{},\"pz\":{}{}{}{}}},",
                            x, y, s.z, s.graphic, spz, foliage, anim, path
                        );
                        n_statics += 1;
                        // A static light source (wall torch, lamp, brazier) glows
                        // at night — same shape as dynamic-item lights (r:3).
                        if lights.len() < LIGHT_CAP && map.item_is_light(s.graphic) {
                            lights.push(json!({ "x": x, "y": y, "z": s.z, "r": 3 }));
                        }
                    }
                    // Multi components (boat hull/deck, house walls) whose tile
                    // falls on this world (x, y) — expanded into the SAME statics
                    // stream so the renderer needs no new drawing path (a
                    // component looks and sorts exactly like a static). Respects
                    // the same roof/max_z cull and nodraw skip as real statics
                    // (see the c9db52b commit that applied the rule to items too)
                    // so standing inside a boat/house still shows its own deck
                    // instead of the roof floating over nothing.
                    if let Some(multis) = multis {
                        for &(mx, my, mz, multi_id, mserial) in &near_multis {
                            let (cdx, cdy) = (x - mx, y - my);
                            if !(i16::MIN as i64..=i16::MAX as i64).contains(&cdx)
                                || !(i16::MIN as i64..=i16::MAX as i64).contains(&cdy)
                            {
                                continue;
                            }
                            // A decoded custom-house design REPLACES this multi's
                            // multi.mul components entirely — the identical swap
                            // `multi_components_at` makes for walkability; see that
                            // fn's doc for why the two must never merge. Design
                            // tiles carry no `visible` flag (a graphic-0 entry was
                            // already dropped at decode), so unlike the multi.mul
                            // branch below there's no `!visible` check here.
                            if let Some(d) = s
                                .world
                                .house_designs
                                .get(&mserial)
                                .filter(|d| d.tiles_ready)
                            {
                                if (i8::MIN as i64..=i8::MAX as i64).contains(&cdx)
                                    && (i8::MIN as i64..=i8::MAX as i64).contains(&cdy)
                                {
                                    if let Some(comp_tiles) = d.tiles.get(&(cdx as i8, cdy as i8)) {
                                        for &(g, dz) in comp_tiles {
                                            emit_multi_component(
                                                map,
                                                animdata,
                                                max_z,
                                                under_cover,
                                                x,
                                                y,
                                                g,
                                                mz + dz as i32,
                                                mserial,
                                                &mut statics,
                                                &mut n_statics,
                                                &mut lights,
                                                LIGHT_CAP,
                                                px,
                                                py,
                                            );
                                        }
                                    }
                                }
                                continue;
                            }
                            for c in multis.components_at(multi_id, cdx as i16, cdy as i16) {
                                if !c.visible {
                                    continue;
                                }
                                emit_multi_component(
                                    map,
                                    animdata,
                                    max_z,
                                    under_cover,
                                    x,
                                    y,
                                    c.graphic,
                                    mz + c.dz as i32,
                                    mserial,
                                    &mut statics,
                                    &mut n_statics,
                                    &mut lights,
                                    LIGHT_CAP,
                                    px,
                                    py,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    // Drop the trailing commas left by the per-entry writes.
    if tiles.ends_with(',') {
        tiles.pop();
    }
    if statics.ends_with(',') {
        statics.pop();
    }

    // Small parts go through serde (cheap + handles string escaping for names).
    let (p_body, p_hue) = remap(p.body, p.hue);
    let mut player = json!({
        "serial": p.serial,
        "x": p.pos.x, "y": p.pos.y, "z": p.pos.z, "dir": p.direction,
        "body": p_body, "dead": player_is_ghost(&s.world), "at": atype(p_body), "name": p.name,
        "noto": p.notoriety,  // self notoriety (innocent/criminal/murderer…) → name-overhead colour
        "hue": p_hue,
        "mounted": mounted, "mountAnim": player_mount_anim,
        "hits": p.hits, "hitsMax": p.hits_max, "mana": p.mana, "manaMax": p.mana_max,
        "stam": p.stam, "stamMax": p.stam_max,
        "str": st.strength, "dex": st.dexterity, "int": st.intelligence, "gold": st.gold,
        "equip": equip,
    });
    merge_obj(&mut player, hidden_field(p.hidden));
    merge_obj(&mut player, poisoned_field(p.poisoned));
    // Recent sound events (the client plays only seqs newer than its last) and the
    // current background music id. Both are read-only views of world audio state.
    let sounds: Vec<Value> = s
        .world
        .recent_sounds
        .iter()
        .map(|&(seq, id, x, y)| json!({ "seq": seq, "id": id, "x": x, "y": y }))
        .collect();
    let sounds = serde_json::to_string(&sounds).unwrap_or_else(|_| "[]".into());
    // Recent character-animation events (0x6E): play group `act` on `serial` once
    // (combat swings, bows, get-hit). The client plays each `seq` newer than its last.
    let anims: Vec<Value> = s
        .world
        .recent_anims
        .iter()
        .map(|&(seq, serial, act, frames, fwd, delay)| {
            json!({ "seq": seq, "serial": serial, "act": act, "frames": frames, "fwd": fwd, "delay": delay })
        })
        .collect();
    let anims = serde_json::to_string(&anims).unwrap_or_else(|_| "[]".into());
    // Recent *typed* animation events (0xE2): `serial` was told to play
    // `AnimationType` `typ`'s `act` (an emote/gesture/alert/…), with `mode` (the
    // wire "delay" byte) available for the client to pick a cosmetic variant. Unlike
    // 0x6E's `act`, `typ`/`act` here are NOT a raw animation group — the client
    // converts them per body (ClassicUO `GetObjectNewAnimation`), since only it
    // knows each body's animation-group layout.
    let tanims: Vec<Value> = s
        .world
        .recent_typed_anims
        .iter()
        .map(|&(seq, serial, typ, act, mode)| {
            json!({ "seq": seq, "serial": serial, "typ": typ, "act": act, "mode": mode })
        })
        .collect();
    let tanims = serde_json::to_string(&tanims).unwrap_or_else(|_| "[]".into());
    // Recent damage events (0x0B): `serial` took `amt` HP. The client floats a
    // number over the target for each `seq` newer than the last it showed.
    let damage: Vec<Value> = s
        .world
        .recent_damage
        .iter()
        .map(|&(seq, serial, amt)| json!({ "seq": seq, "serial": serial, "amt": amt }))
        .collect();
    let damage = serde_json::to_string(&damage).unwrap_or_else(|_| "[]".into());
    // Recent graphical effects (0x70/0xC0/0xC7): spell bolts, hit sparkles,
    // explosions, fields. The client spawns a visual for each `seq` newer than the
    // last it saw. We resolve the ART tile-id animation sequence + per-frame
    // interval server-side from animdata.mul so the client just cycles `frames`.
    let effects: Vec<Value> = s
        .world
        .recent_effects
        .iter()
        .map(|e| {
            let (frames, interval) = match animdata {
                Some(ad) => (ad.frame_sequence(e.graphic), ad.frames(e.graphic).1),
                None => (vec![e.graphic], 0u8),
            };
            json!({
                "seq": e.seq, "kind": e.kind, "src": e.src_serial, "tgt": e.tgt_serial,
                "sx": e.sx, "sy": e.sy, "sz": e.sz, "tx": e.tx, "ty": e.ty, "tz": e.tz,
                "g": e.graphic, "hue": e.hue, "speed": e.speed, "dur": e.duration,
                "frames": frames, "interval": interval
            })
        })
        .collect();
    let effects = serde_json::to_string(&effects).unwrap_or_else(|_| "[]".into());
    let music = match s.world.current_music {
        Some(id) => id.to_string(),
        None => "null".to_string(),
    };
    // Day/night + weather: the renderer darkens the scene by `light` and animates
    // rain/snow particles for the matching `weather` kind (`weatherN` = intensity).
    let light = s.world.effective_light();
    let weather = s.world.weather.kind;
    let weather_n = s.world.weather.intensity;
    // Current season (0xBC): the renderer may tint the scene per season. We do not
    // remap tree/foliage graphics (a much larger change).
    let season = s.world.season;
    // Active buffs/debuffs (0xDF): icon (upsert key), short name, duration secs.
    let buffs: Vec<Value> = s
        .world
        .buffs
        .iter()
        .map(|b| json!({ "icon": b.icon, "name": b.name, "dur": b.dur }))
        .collect();
    let buffs = serde_json::to_string(&buffs).unwrap_or_else(|_| "[]".into());
    // The player's skills (0x3A), sorted by id. Values stay in tenths (wire units):
    // 500 == 50.0; the client divides by 10 for display. `lock`: 0=up,1=down,2=locked.
    let mut skills: Vec<&anima_core::world::Skill> = s.world.skills.values().collect();
    skills.sort_by_key(|sk| sk.id);
    let skills: Vec<Value> = skills
        .iter()
        .map(|sk| json!({ "id": sk.id, "v": sk.value, "b": sk.base, "c": sk.cap, "lock": sk.lock }))
        .collect();
    let skills = serde_json::to_string(&skills).unwrap_or_else(|_| "[]".into());
    let lights = serde_json::to_string(&lights).unwrap_or_else(|_| "[]".into());
    let mobiles = serde_json::to_string(&mobiles).unwrap_or_else(|_| "[]".into());
    let items = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
    let cont_items = serde_json::to_string(&cont_items).unwrap_or_else(|_| "[]".into());
    let target = serde_json::to_string(&target).unwrap_or_else(|_| "{}".into());
    let dbg = serde_json::to_string(&dbg).unwrap_or_else(|_| "[]".into());
    let journal = serde_json::to_string(journal).unwrap_or_else(|_| "[]".into());
    // Open server gumps/dialogs (0xB0/0xDD), each parsed into positioned elements.
    let gumps = gumps_json(&s.world, cliloc);
    // The open right-click context menu (0xBF/0x14), with cliloc labels resolved.
    let popup =
        serde_json::to_string(&popup_json(&s.world, cliloc)).unwrap_or_else(|_| "null".into());
    // Legacy item/question menus (0x7C), potentially several at once.
    let legacy_menus =
        serde_json::to_string(&legacy_menus_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // Server dye hue pickers (0x95), potentially several callback serials.
    let hue_pickers =
        serde_json::to_string(&hue_pickers_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // Concurrent 0xA6 Tip/Notice windows. Only kind "tip" has prev/next.
    let tips = serde_json::to_string(&tips_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // Concurrent modal 0xAB text-entry dialogs, keyed in the browser by seq.
    let text_entry_dialogs =
        serde_json::to_string(&text_entry_dialogs_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // Persistent 0xB8 character profile windows, keyed by exact response seq.
    let profiles =
        serde_json::to_string(&character_profiles_json(&s.world)).unwrap_or_else(|_| "[]".into());
    let logout_ack =
        serde_json::to_string(&logout_ack_json(&s.world)).unwrap_or_else(|_| "null".into());
    let boat_moves =
        serde_json::to_string(&boat_movements_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // The open book (0x93/0xD4 + 0x66), or null.
    let book = serde_json::to_string(&book_json(&s.world)).unwrap_or_else(|_| "null".into());
    // Known spellbook contents (0xBF/0x1B), one entry per book we've been told
    // about this session (see `World::spellbooks`'s doc — populated only once a
    // book is actually opened). `content` is a 64-bit mask; split into two u32
    // halves (`lo` = bits 0..31, `hi` = bits 32..63) rather than sent whole,
    // because a JS `Number` only carries 53 bits of integer precision and a full
    // 64-spell Magery book can set bits past that — the renderer tests a bit
    // with plain 32-bit ops on whichever half it falls in, no BigInt needed.
    // Sorted by serial for a stable order (the source is a HashMap).
    let mut spellbooks: Vec<(&u32, &anima_core::world::SpellbookContent)> =
        s.world.spellbooks.iter().collect();
    spellbooks.sort_by_key(|&(serial, _)| *serial);
    let spellbooks: Vec<Value> = spellbooks
        .iter()
        .map(|&(serial, sb)| {
            json!({
                "serial": serial, "graphic": sb.graphic, "offset": sb.offset,
                "lo": (sb.content & 0xFFFF_FFFF) as u32, "hi": (sb.content >> 32) as u32
            })
        })
        .collect();
    let spellbooks = serde_json::to_string(&spellbooks).unwrap_or_else(|_| "[]".into());
    // Object Property Lists / tooltips (0xD6), resolved to display lines, capped.
    let opl = serde_json::to_string(&opl_json(&s.world, cliloc)).unwrap_or_else(|_| "{}".into());
    // The on-screen quest arrow target tile (0xBA), or null.
    let quest_arrow = match s.world.quest_arrow {
        Some((x, y)) => format!("{{\"x\":{x},\"y\":{y}}}"),
        None => "null".to_string(),
    };
    // The player's party (0xBF/0x06): leader, members (name/hits from view), invite.
    let party = serde_json::to_string(&party_json(&s.world)).unwrap_or_else(|_| "{}".into());
    // Combat state: war mode (0x72) and the current "last target" serial (0 = none)
    // so the client can show a war indicator and highlight the attacked mobile.
    let war = s.world.war;
    let last_attack = s.world.last_attack.unwrap_or(0);
    // The server's authoritative combat opponent (0xAA ChangeCombatant, 0 = none)
    // — distinct from `lastAttack` (the serial WE last sent an Attack request
    // for): the server can retarget on its own.
    let combatant = s.world.combatant.unwrap_or(0);
    // AOS expansion (SupportedFeatures 0xB9): gates AOS-only UI like the weapon
    // special-ability bar. T2A servers don't advertise it → the client hides it.
    let aos = s.world.aos;
    // An outstanding 0x9A ASCII / 0xC2 Unicode server prompt, or `{"active":0}`.
    // See [`prompt_json`]'s doc.
    let prompt =
        serde_json::to_string(&prompt_json(&s.world)).unwrap_or_else(|_| "{\"active\":0}".into());
    // Recent lift-rejection events (0x27 LiftRej): the client clears the drag-ghost
    // (without sending a drop — the item never left its source) and shows `reason`
    // as a system journal line, for each `seq` newer than the last it handled.
    let lift_rejects: Vec<Value> = s
        .world
        .recent_lift_rejects
        .iter()
        .map(|&(seq, reason)| json!({ "seq": seq, "reason": reason }))
        .collect();
    let lift_rejects = serde_json::to_string(&lift_rejects).unwrap_or_else(|_| "[]".into());
    // Item-drag completion acknowledgements (0x28 EndDraggingItem / 0x29
    // DropItemAccepted). The browser correlates these with pending placements
    // before clearing its cursor, protecting a newer lift from a delayed ack.
    let drag_completions =
        serde_json::to_string(&drag_completions_json(&s.world)).unwrap_or_else(|_| "[]".into());
    let death_screen =
        serde_json::to_string(&death_screen_json(&s.world)).unwrap_or_else(|_| "null".into());
    // Recent server-initiated container opens (0x24 DrawContainer): a window we
    // did NOT ourselves double-click for (banker "bank" speech, GM `[bank`, a
    // snoop, …). The client opens a window for each `seq` newer than the last it
    // handled (reusing the same `openContainer` it uses for its own double-clicks).
    // Filtered by `container_opens_json` to real container gumpIds — see its doc.
    let container_opens =
        serde_json::to_string(&container_opens_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // Recent Swing events (0x2F): `attacker` just swung at `defender`. Purely
    // cosmetic — the client briefly faces the attacker toward the defender.
    let swings: Vec<Value> = s
        .world
        .recent_swings
        .iter()
        .map(|&(seq, attacker, defender)| json!({ "seq": seq, "attacker": attacker, "defender": defender }))
        .collect();
    let swings = serde_json::to_string(&swings).unwrap_or_else(|_| "[]".into());
    // The latest server-initiated paperdoll open/refresh (0x88), or null. See
    // `paperdoll_json`'s doc for the `seq` "fresh request" semantics.
    let paperdoll =
        serde_json::to_string(&paperdoll_json(&s.world)).unwrap_or_else(|_| "null".into());
    // Validated 0xA5 external-page requests. The browser seq-gates these and
    // requires an explicit click before opening a new tab.
    let open_urls =
        serde_json::to_string(&open_urls_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // Current facet/map index (0xBF/0x08 MapChange); see `World::map_index`'s doc
    // for what a real per-facet `MapData` reload would additionally require.
    let facet = s.world.map_index;
    // Every open secure-trade session (0x6F), or []. See `trades_json`'s doc.
    let trades = serde_json::to_string(&trades_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // Open treasure/decoration map windows (0x90/0xF5 + 0x56), or []. See `maps_json`'s doc.
    let maps = serde_json::to_string(&maps_json(&s.world)).unwrap_or_else(|_| "[]".into());
    // Purely additive: omit the key entirely when nothing is pending (see
    // `placement_json`'s doc), rather than a `"placement":null`, so an idle
    // scene's JSON is unchanged from before this field existed.
    let placement_field = match placement {
        Some(v) => format!(
            ",\"placement\":{}",
            serde_json::to_string(&v).unwrap_or_else(|_| "null".into())
        ),
        None => String::new(),
    };
    // Purely additive, same convention as `placement_field` right above: omit
    // the key entirely when nobody is customizing a house.
    let house_design_field = match house_design {
        Some(v) => format!(
            ",\"houseDesign\":{}",
            serde_json::to_string(&v).unwrap_or_else(|_| "null".into())
        ),
        None => String::new(),
    };
    format!(
        "{{\"player\":{player},\
         \"map\":{{\"cx\":{px},\"cy\":{py},\"radius\":{LAND_RADIUS},\"viewRange\":{RADIUS},\"tiles\":[{tiles}],\"maxZ\":{max_z},\"dbg\":{dbg}}},\
         \"statics\":[{statics}],\"mobiles\":{mobiles},\"items\":{items},\"contItems\":{cont_items},\
         \"target\":{target},\"shop\":{shop},\"journal\":{journal},\"sounds\":{sounds},\"anims\":{anims},\"tanims\":{tanims},\"damage\":{damage},\"effects\":{effects},\"music\":{music},\
         \"light\":{light},\"weather\":{weather},\"weatherN\":{weather_n},\"season\":{season},\"lights\":{lights},\"buffs\":{buffs},\"skills\":{skills},\"gumps\":{gumps},\
         \"popup\":{popup},\"legacyMenus\":{legacy_menus},\"huePickers\":{hue_pickers},\"tips\":{tips},\"textEntryDialogs\":{text_entry_dialogs},\"profiles\":{profiles},\"logoutAck\":{logout_ack},\"boatMoves\":{boat_moves},\"book\":{book},\"spellbooks\":{spellbooks},\"opl\":{opl},\"questArrow\":{quest_arrow},\"party\":{party},\
         \"war\":{war},\"lastAttack\":{last_attack},\"combatant\":{combatant},\"aos\":{aos},\
         \"prompt\":{prompt},\"liftRejects\":{lift_rejects},\"dragCompletions\":{drag_completions},\"deathScreen\":{death_screen},\"containerOpens\":{container_opens},\"swings\":{swings},\
         \"paperdoll\":{paperdoll},\"openUrls\":{open_urls},\"facet\":{facet},\"trades\":{trades},\"maps\":{maps}{placement_field}{house_design_field},\
         \"stats\":{{\"confirms\":{},\"denies\":{}}}}}",
        s.confirms, s.denies
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gump_layout_parses_common_commands() {
        let layout = "{ resizepic 0 0 5054 200 120 }{ button 20 90 247 248 1 0 7 }\
                      { text 20 20 0 0 }{ checkbox 20 50 210 211 1 3 }\
                      { textentry 20 65 120 18 0 4 1 }";
        let text = vec!["Accept the quest?".to_string(), "Name".to_string()];
        let parsed = gump_layout::parse(layout, &text);
        let els: Vec<Value> = parsed
            .elements
            .iter()
            .map(|e| gump_element_json(e, None))
            .collect();
        // Width comes straight from the resizepic; height grows to fit elements
        // that extend below it (the button at y=90 + padding).
        assert_eq!(parsed.width, 200);
        assert!(parsed.height >= 120, "h={}", parsed.height);

        // bg, button(id 7), text("Accept…"), check(id 3,on), entry(id 4,"Name").
        let kinds: Vec<&str> = els.iter().map(|e| e["t"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["bg", "button", "text", "check", "entry"]);
        assert_eq!(els[1]["id"], 7);
        // pageflag 1 (reply) — this is what makes the button send a GumpResponse
        // instead of jumping pages locally.
        assert_eq!(els[1]["pageflag"], 1);
        assert_eq!(els[2]["s"], "Accept the quest?");
        assert_eq!(
            (els[3]["id"].as_i64(), els[3]["on"].as_i64()),
            (Some(3), Some(1))
        );
        assert_eq!(els[4]["id"], 4);
        assert_eq!(els[4]["s"], "Name");
    }

    #[test]
    fn gump_layout_tracks_pages_and_button_pageflag() {
        // Elements before the first "page" token are page 0 (always visible,
        // e.g. the background + a "next"/"prev" nav button that must show no
        // matter which page is active). "page 1" then "page 2" bracket the two
        // navigable sections; the pageflag-0 button on page 1 jumps to page 2
        // locally (no server round-trip), while the pageflag-1 button on page 2
        // is a real reply button.
        let layout = "{ resizepic 0 0 5054 200 200 }\
                      { page 1 }{ text 10 10 0 0 }\
                      { button 10 30 4005 4007 0 2 0 }\
                      { page 2 }{ text 10 10 0 1 }\
                      { button 10 30 247 248 1 0 99 }";
        let text = vec!["Page one".to_string(), "Page two".to_string()];
        let parsed = gump_layout::parse(layout, &text);
        let els: Vec<Value> = parsed
            .elements
            .iter()
            .map(|e| gump_element_json(e, None))
            .collect();

        // bg(page0), text(page1), button(page1, pageflag0→page2), text(page2), button(page2, pageflag1, id99)
        let pages: Vec<i64> = els.iter().map(|e| e["page"].as_i64().unwrap()).collect();
        assert_eq!(pages, [0, 1, 1, 2, 2]);

        let jump_btn = &els[2];
        assert_eq!(jump_btn["t"], "button");
        assert_eq!(jump_btn["pageflag"], 0);
        assert_eq!(jump_btn["param"], 2); // switches to page 2, contacts no server

        let reply_btn = &els[4];
        assert_eq!(reply_btn["t"], "button");
        assert_eq!(reply_btn["pageflag"], 1);
        assert_eq!(reply_btn["id"], 99); // reply id sent to the server on click
    }

    #[test]
    fn gump_layout_preserves_html_tags_and_handles_cliloc() {
        // Tags are no longer stripped here — the client's `renderGumpHtml`
        // interprets them (CENTER/BASEFONT/etc.) for display; the scene JSON
        // just carries the resolved string through unchanged, same as it
        // always has for a cliloc-driven `html` element.
        let layout = "{ htmlgump 5 5 180 40 0 0 0 }{ xmfhtmlgump 5 50 180 20 1015313 0 0 }";
        let text = vec!["<basefont color=#fff>Hello <b>world</b>".to_string()];
        let parsed = gump_layout::parse(layout, &text);
        let els: Vec<Value> = parsed
            .elements
            .iter()
            .map(|e| gump_element_json(e, None))
            .collect();
        assert_eq!(els[0]["s"], "<basefont color=#fff>Hello <b>world</b>");
        assert_eq!(els[1]["s"], "#1015313"); // cliloc placeholder (no table)
    }

    #[test]
    fn equip_conv_gump_resolves_bare_and_baked_ids() {
        // A bare graphic id (below MALE_GUMP_OFFSET) just gets the wearer's gender
        // offset added — e.g. Equipconv.def's "0 → equipmentID" / "-1 → newGraphic"
        // cases store a plain item graphic like 538 or 977.
        assert_eq!(equip_conv_gump(400, 538), 50_538); // male wearer
        assert_eq!(equip_conv_gump(401, 538), 60_538); // female wearer (401)
                                                       // A value already baked with SOME gender's offset gets that offset
                                                       // stripped and the wearer's ACTUAL gender's offset re-added (ClassicUO
                                                       // `GetAnimID`) — here the def literally stores the female-baked 61250 for
                                                       // a female (401) wearer, so it round-trips unchanged...
        assert_eq!(equip_conv_gump(401, 61_250), 61_250);
        // ...but a MALE wearer (400) re-bases it onto the male offset instead.
        assert_eq!(equip_conv_gump(400, 61_250), 51_250);
        // A male-baked id (50xxx) re-based onto a female wearer.
        assert_eq!(equip_conv_gump(401, 50_684), 60_684);
        // Elf female body (606) is EVEN — must not fall out via a parity test.
        assert_eq!(equip_conv_gump(606, 538), 60_538);
    }

    // ---- build_scene coverage -------------------------------------------------
    //
    // `build_scene` itself takes `&mut Session`, and `Session` can only be built
    // via `connect_and_login` (a live `TcpStream`) — per this crate's testing
    // convention (see `route_tests`'s doc in `lib.rs`), unit tests don't spin up
    // a live Session/socket. `tile_walkable`/`can_walk`/`can_step_to` similarly
    // need a real `anima_assets::MapData`, which only opens actual UO data files
    // (no in-memory constructor) — coverage for these needs either a `MapData`
    // test constructor (a real seam, not attempted here) or an `#[ignore]`d test
    // gated on a local UO install; `can_step_to` (which `can_walk`/`step_ok` now
    // wrap) has exactly that below (`can_step_to_allows_the_stairs_climb_...`).
    // `calculate_new_z` avoids this by testing its `bound_min_max_z`/
    // `resolve_standing_z` pure cores directly with synthetic `PathObj` literals
    // (see the staircase tests below), plus one `#[ignore]`d real-data test
    // against an actual staircase for end-to-end confidence.
    //
    // What *is* both pure (`&World`/primitives in, `Value`/`bool` out) and where
    // most of the shaping logic actually lives has been tested directly below:
    // the `*_json` helpers `build_scene` calls, plus the two little pieces
    // (`stack_fields`/`corpse_fields`) pulled out of its item loop so the
    // corpse/stackable shaping is unit-testable without a live Session.

    use anima_assets::MultiComponent;
    use anima_core::types::{Position, Serial};
    use anima_core::world::{
        Book, HuePicker, LegacyMenu, LegacyMenuEntry, LegacyMenuKind, MultiPlacement, PopupEntry,
        PopupMenu, PromptKind, PromptState, TargetCursor, TipKind, TradeState,
    };

    #[test]
    fn player_is_ghost_recognizes_all_servuo_ghost_bodies() {
        let mut w = World::default();
        assert!(!player_is_ghost(&w), "no player yet");

        w.player = Some(Serial(1));
        w.mobile_mut(1).body = 400; // ordinary human male
        assert!(!player_is_ghost(&w));

        for body in [402, 403, 607, 608, 694, 695, 970] {
            w.mobile_mut(1).body = body;
            assert!(player_is_ghost(&w), "body {body} must be a ghost");
        }
    }

    #[test]
    fn trades_json_empty_when_no_sessions_reflects_when_open() {
        let mut w = World::default();
        assert_eq!(trades_json(&w), json!([]), "no trades → empty array");

        w.open_trade(TradeState {
            opponent_serial: 0x1001,
            opponent_name: "Bob".to_string(),
            my_container: 0x2001,
            their_container: 0x2002,
            my_accept: true,
            their_accept: false,
            my_offer_gold: 50,
            my_offer_platinum: 0,
            their_offer_gold: 0,
            their_offer_platinum: 1,
            balance_gold: 500,
            balance_platinum: 2,
        });
        let v = trades_json(&w);
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        let t = &arr[0];
        assert_eq!(t["opponent"], "Bob");
        assert_eq!(t["opponentSerial"], 0x1001);
        assert_eq!(t["myCont"], 0x2001);
        assert_eq!(t["theirCont"], 0x2002);
        assert_eq!(t["myAccept"], true);
        assert_eq!(t["theirAccept"], false);
        assert_eq!(t["myOfferGold"], 50);
        assert_eq!(t["theirOfferPlat"], 1);
        assert_eq!(t["balanceGold"], 500);
    }

    #[test]
    fn prompt_json_reports_active_and_ids_or_inactive() {
        let mut w = World::default();
        assert_eq!(prompt_json(&w), json!({ "active": 0 }), "no prompt pending");

        w.prompt = Some(PromptState {
            sender_serial: 0x77,
            prompt_id: 42,
            kind: PromptKind::Ascii,
        });
        assert_eq!(
            prompt_json(&w),
            json!({ "active": 1, "serial": 0x77, "promptId": 42, "kind": "ascii" })
        );
    }

    #[test]
    fn open_urls_json_preserves_seq_and_validated_url() {
        let mut w = World::default();
        assert_eq!(open_urls_json(&w), json!([]));
        w.push_open_url("https://uo.com/news".into());
        w.push_open_url("http://localhost:8080/help".into());
        assert_eq!(
            open_urls_json(&w),
            json!([
                { "seq": 1, "url": "https://uo.com/news" },
                { "seq": 2, "url": "http://localhost:8080/help" },
            ])
        );
    }

    #[test]
    fn container_opens_json_skips_vendor_buy_and_spellbook_gump_ids() {
        let mut w = World::default();
        // DisplayBuyList (vendor "Buy" window): gumpId 0x30 — must NOT open a
        // (spurious, empty) container window; the vendor's shop already opens
        // via `shop`/0x74/0x3B.
        w.push_container_open(0x1000_0055, 0x0030);
        assert_eq!(
            container_opens_json(&w),
            json!([]),
            "vendor buy gumpId must not signal an open"
        );

        // DisplaySpellbook: gumpId 0xFFFF — must NOT open one either; the book
        // already opens via `spellbooks`/0xBF/0x1B.
        w.push_container_open(0x4000_0066, 0xFFFF);
        assert_eq!(
            container_opens_json(&w),
            json!([]),
            "spellbook gumpId must not signal an open"
        );

        // A normal container gumpId (e.g. a bank box) DOES open.
        w.push_container_open(0x4000_0077, 0x0048);
        assert_eq!(
            container_opens_json(&w),
            json!([{ "seq": 3, "serial": 0x4000_0077u32 }]),
            "a real container gumpId must still signal an open"
        );
    }

    #[test]
    fn drag_completions_json_preserves_packet_and_optional_token() {
        let mut w = World::default();
        assert_eq!(drag_completions_json(&w), json!([]));

        w.push_drag_completion(0x28, Some(0x4000_1234));
        w.push_drag_completion(0x29, None);
        assert_eq!(
            drag_completions_json(&w),
            json!([
                { "seq": 1, "packet": 0x28, "token": 0x4000_1234u32 },
                { "seq": 2, "packet": 0x29, "token": null }
            ])
        );
    }

    #[test]
    fn death_screen_json_is_a_seq_gated_event_not_alive_state() {
        let mut w = World::default();
        assert_eq!(death_screen_json(&w), Value::Null);

        w.on_death_screen(0);
        assert_eq!(death_screen_json(&w), json!({ "seq": 1, "action": 0 }));
        w.on_death_screen(2);
        assert_eq!(death_screen_json(&w), json!({ "seq": 2, "action": 2 }));

        w.on_death_screen(1);
        assert_eq!(death_screen_json(&w), json!({ "seq": 2, "action": 2 }));
    }

    #[test]
    fn paperdoll_json_null_when_none_reports_when_set() {
        let mut w = World::default();
        assert_eq!(paperdoll_json(&w), Value::Null, "no paperdoll signal yet");

        w.set_paperdoll(
            0xDEAD_BEEFu32,
            "Anima the Adventurer".to_string(),
            true,
            false,
        );
        assert_eq!(
            paperdoll_json(&w),
            json!({
                "seq": 1, "serial": 0xDEAD_BEEFu32, "title": "Anima the Adventurer",
                "warmode": true, "canLift": false,
            })
        );
    }

    #[test]
    fn maps_json_empty_then_reports_bounds_and_pins() {
        let mut w = World::default();
        assert_eq!(maps_json(&w), json!([]), "no map window open yet");

        w.set_map_view(0x4000_1234, 0x139D, 3, 520, 0, 2580, 2050, 400, 400);
        w.apply_map_command(0x4000_1234, 1, 0, 100, 120); // chest pin, index 0
        assert_eq!(
            maps_json(&w),
            json!([{
                "serial": 0x4000_1234u32, "openSeq": 1, "gumpArt": 0x139D, "facet": 3,
                "bounds": { "minX": 520, "minY": 0, "maxX": 2580, "maxY": 2050 },
                "w": 400, "h": 400,
                "pins": [[100, 120]],
                "editable": false,
            }])
        );

        // A re-decode/re-click bumps `openSeq` — the web client's seq-gate is
        // what tells it to reopen a closed window.
        w.set_map_view(0x4000_1234, 0x139D, 3, 520, 0, 2580, 2050, 400, 400);
        let v = maps_json(&w);
        assert_eq!(v[0]["openSeq"], 2);
        assert_eq!(
            v[0]["pins"].as_array().unwrap().len(),
            0,
            "a resend resets pins"
        );
    }

    #[test]
    fn resolve_shop_name_leaves_plain_names_alone() {
        assert_eq!(resolve_shop_name("a hatchet", None), "a hatchet");
        // A small numeric-looking name (below the cliloc-id floor) is left as-is.
        assert_eq!(resolve_shop_name("123", None), "123");
    }

    #[test]
    fn resolve_shop_name_resolves_cliloc_shaped_ids_bare_and_hashed() {
        // No table loaded → falls back to the raw string exactly as given
        // (the '#' is only stripped for the numeric *parse*, not the fallback).
        assert_eq!(resolve_shop_name("1060834", None), "1060834");
        assert_eq!(resolve_shop_name("#1060834", None), "#1060834");

        // With a real (synthetic, on-disk) Cliloc table, both ServUO's actual
        // bare-numeric shape (`IShopSellInfo.GetNameFor`'s `LabelNumber.ToString()`)
        // and a hypothetical '#'-prefixed one resolve to the same real name.
        // `Cliloc` only loads from a directory (`Cliloc::open`), so this writes a
        // minimal synthetic `Cliloc.enu` (6-byte header + one record — same shape
        // `anima_assets::cliloc`'s own tests build) to a scratch dir.
        let dir = std::env::temp_dir().join(format!(
            "anima_scene_test_cliloc_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let id: u32 = 1_060_834;
        let text = "a hatchet";
        let mut buf = vec![0u8; 6]; // 6-byte header, contents unused by the parser
        buf.extend_from_slice(&id.to_le_bytes());
        buf.push(0); // flag
        buf.extend_from_slice(&(text.len() as u16).to_le_bytes());
        buf.extend_from_slice(text.as_bytes());
        std::fs::write(dir.join("Cliloc.enu"), &buf).expect("write synthetic Cliloc.enu");
        let cliloc = Cliloc::open(&dir).expect("open synthetic Cliloc.enu");

        assert_eq!(resolve_shop_name("1060834", Some(&cliloc)), "a hatchet");
        assert_eq!(resolve_shop_name("#1060834", Some(&cliloc)), "a hatchet");

        let _ = std::fs::remove_dir_all(&dir); // best-effort cleanup
    }

    #[test]
    fn stack_fields_marks_stackable_only_when_flagged() {
        // A stack of reagents (Stackable tiledata flag set): "amount" + "st":1 so
        // the renderer offers the split-stack dialog.
        assert_eq!(stack_fields(40, true), json!({ "amount": 40, "st": 1 }));
        // A non-stackable item (e.g. a sword) never gets "st", even with
        // amount > 1 (shouldn't normally happen, but the field must still be
        // omitted so the renderer doesn't offer to split it).
        assert_eq!(stack_fields(1, false), json!({ "amount": 1 }));
        assert_eq!(stack_fields(5, false), json!({ "amount": 5 }));
    }

    #[test]
    fn hidden_field_present_only_when_true() {
        assert_eq!(hidden_field(true), json!({ "hidden": true }));
        // Not hidden → no key at all (not `"hidden": false`), so the renderer's
        // default (fully opaque) needs no per-mobile check.
        assert_eq!(hidden_field(false), json!({}));
    }

    #[test]
    fn poisoned_field_present_only_when_true() {
        assert_eq!(poisoned_field(true), json!({ "poisoned": true }));
        // Not poisoned → no key at all (not `"poisoned": false`), so the
        // renderer's default (HP-fraction-only bar color) needs no per-mobile
        // check.
        assert_eq!(poisoned_field(false), json!({}));
    }

    #[test]
    fn corpse_fields_carries_remapped_body_dir_and_death_group() {
        // Values here are already Corpse.def-remapped/resolved by the caller
        // (`build_scene`'s item loop) — this just checks the shaping.
        let v = corpse_fields(
            /* body */ 26, /* hue */ 1102, /* dir */ 3, /* dg */ 8,
        );
        assert_eq!(v, json!({ "body": 26, "dir": 3, "dg": 8, "hue": 1102 }));
    }

    #[test]
    fn party_json_reports_members_leader_and_pending_invite() {
        let mut w = World::default();
        // Not in a party: empty members, leader 0, no invite.
        assert_eq!(
            party_json(&w),
            json!({ "leader": 0, "members": [], "invite": 0 })
        );

        w.party.leader = 0x100;
        w.party.members = vec![0x100, 0x101];
        w.party.pending_invite = Some(0x200);
        // Member 0x101 is in view (has a Mobile); 0x100 (the leader) isn't, so it
        // falls back to the "Member"/0/0 placeholder.
        w.mobile_mut(0x101).name = "Alice".to_string();
        w.mobile_mut(0x101).hits = 80;
        w.mobile_mut(0x101).hits_max = 100;
        let v = party_json(&w);
        assert_eq!(v["leader"], 0x100);
        assert_eq!(v["invite"], 0x200);
        let members = v["members"].as_array().unwrap();
        assert_eq!(members[0]["name"], "Member"); // 0x100 not in view
        assert_eq!(members[1]["name"], "Alice");
        assert_eq!(members[1]["hits"], 80);
        assert_eq!(members[1]["hitsMax"], 100);
    }

    #[test]
    fn popup_json_null_when_absent_resolves_entries_when_open() {
        let mut w = World::default();
        assert_eq!(popup_json(&w, None), Value::Null);

        w.popup = Some(PopupMenu {
            serial: 0x555,
            entries: vec![PopupEntry {
                index: 0,
                cliloc: 3000123,
                flags: 0,
            }],
        });
        let v = popup_json(&w, None);
        assert_eq!(v["serial"], 0x555);
        // No Cliloc table available → falls back to "#<id>".
        assert_eq!(v["entries"][0]["text"], "#3000123");
        assert_eq!(v["entries"][0]["index"], 0);
    }

    #[test]
    fn legacy_menus_json_is_sorted_and_preserves_item_metadata() {
        let mut w = World::default();
        w.legacy_menus = vec![
            LegacyMenu {
                serial: 20,
                menu_id: 0,
                question: "Continue?".into(),
                kind: LegacyMenuKind::Question,
                entries: vec![LegacyMenuEntry {
                    text: "Yes".into(),
                    ..LegacyMenuEntry::default()
                }],
            },
            LegacyMenu {
                serial: 10,
                menu_id: 7,
                question: "Choose".into(),
                kind: LegacyMenuKind::Items,
                entries: vec![LegacyMenuEntry {
                    graphic: 0x0F5E,
                    hue: 0x0481,
                    text: "Sword".into(),
                }],
            },
        ];
        let v = legacy_menus_json(&w);
        assert_eq!(v[0]["serial"], 10);
        assert_eq!(v[0]["kind"], "items");
        assert_eq!(v[0]["entries"][0]["index"], 1);
        assert_eq!(v[0]["entries"][0]["graphic"], 0x0F5E);
        assert_eq!(v[0]["entries"][0]["hue"], 0x0481);
        assert_eq!(v[1]["serial"], 20);
        assert_eq!(v[1]["kind"], "question");
    }

    #[test]
    fn hue_pickers_json_is_sorted_and_exact() {
        let mut w = World::default();
        w.hue_pickers = vec![
            HuePicker {
                serial: 20,
                graphic: 0x2006,
            },
            HuePicker {
                serial: 10,
                graphic: 0x0FAB,
            },
        ];
        assert_eq!(
            hue_pickers_json(&w),
            json!([
                { "serial": 10, "graphic": 0x0FAB },
                { "serial": 20, "graphic": 0x2006 },
            ])
        );
    }

    #[test]
    fn tips_json_preserves_concurrent_window_identity_and_kind() {
        let mut w = World::default();
        assert_eq!(tips_json(&w), json!([]));
        w.push_tip(0x1234_5678, TipKind::Tip, "First\nSecond".into());
        w.push_tip(9, TipKind::Notice, "Maintenance".into());
        assert_eq!(
            tips_json(&w),
            json!([
                { "seq": 1, "tip": 0x1234_5678u32, "kind": "tip", "text": "First\nSecond" },
                { "seq": 2, "tip": 9, "kind": "notice", "text": "Maintenance" },
            ])
        );
    }

    #[test]
    fn text_entry_dialogs_json_preserves_callbacks_and_constraints() {
        let mut w = World::default();
        assert_eq!(text_entry_dialogs_json(&w), json!([]));
        w.push_text_entry_dialog(
            0x0102_0304,
            5,
            6,
            "Account €".into(),
            true,
            2,
            12,
            "Digits only".into(),
        );
        assert_eq!(
            text_entry_dialogs_json(&w),
            json!([{
                "seq": 1,
                "serial": 0x0102_0304u32,
                "parentId": 5,
                "buttonId": 6,
                "text": "Account €",
                "canClose": true,
                "variant": 2,
                "maxLength": 12,
                "description": "Digits only",
            }])
        );
    }

    #[test]
    fn character_profiles_json_preserves_order_text_and_editability() {
        let mut w = World::default();
        w.character_profiles = vec![
            anima_core::world::CharacterProfile {
                seq: 7,
                serial: 0x0102_0304,
                header: "Header €".into(),
                footer: "Footer 😀".into(),
                body: "Biography".into(),
                can_edit: true,
            },
            anima_core::world::CharacterProfile {
                seq: 8,
                serial: 0,
                header: "Locked".into(),
                footer: String::new(),
                body: "Read only".into(),
                can_edit: false,
            },
        ];
        assert_eq!(
            character_profiles_json(&w),
            json!([
                {
                    "seq": 7,
                    "serial": 0x0102_0304u32,
                    "header": "Header €",
                    "footer": "Footer 😀",
                    "body": "Biography",
                    "canEdit": true,
                },
                {
                    "seq": 8,
                    "serial": 0,
                    "header": "Locked",
                    "footer": "",
                    "body": "Read only",
                    "canEdit": false,
                },
            ])
        );
    }

    #[test]
    fn logout_ack_json_is_null_then_preserves_permission_identity() {
        let mut w = World::default();
        assert_eq!(logout_ack_json(&w), Value::Null);
        w.set_logout_ack(false);
        assert_eq!(logout_ack_json(&w), json!({ "seq": 1, "allowed": false }));
        w.set_logout_ack(true);
        assert_eq!(logout_ack_json(&w), json!({ "seq": 2, "allowed": true }));
    }

    #[test]
    fn boat_movements_json_preserves_rigid_group_and_speed() {
        let mut w = World::default();
        w.push_boat_movement(anima_core::world::BoatMovement {
            seq: 0,
            boat_serial: 0x4000_1000,
            speed: 4,
            moving_direction: 2,
            facing_direction: 6,
            from: anima_core::types::Position {
                x: 10,
                y: 20,
                z: -5,
            },
            to: anima_core::types::Position {
                x: 11,
                y: 20,
                z: -5,
            },
            entities: vec![anima_core::world::BoatMovedEntity {
                serial: 0x42,
                from: anima_core::types::Position {
                    x: 10,
                    y: 21,
                    z: -4,
                },
                to: anima_core::types::Position {
                    x: 11,
                    y: 21,
                    z: -4,
                },
            }],
        });
        assert_eq!(
            boat_movements_json(&w),
            json!([{
                "seq": 1, "boat": 0x4000_1000u32, "speed": 4, "dir": 2, "facing": 6,
                "entities": [
                    { "serial": 0x4000_1000u32, "from": { "x": 10, "y": 20, "z": -5 }, "to": { "x": 11, "y": 20, "z": -5 } },
                    { "serial": 0x42, "from": { "x": 10, "y": 21, "z": -4 }, "to": { "x": 11, "y": 21, "z": -4 } },
                ]
            }])
        );
    }

    #[test]
    fn book_json_null_when_absent_full_when_open() {
        let mut w = World::default();
        assert_eq!(book_json(&w), Value::Null);

        w.book = Some(Book {
            serial: 0x900,
            title: "Notes".to_string(),
            author: "Anon".to_string(),
            writable: true,
            page_count: 2,
            pages: vec![vec!["hello".to_string()], vec![]],
        });
        let v = book_json(&w);
        assert_eq!(v["title"], "Notes");
        assert_eq!(v["writable"], true);
        assert_eq!(v["pageCount"], 2);
        assert_eq!(v["pages"][0][0], "hello");
    }

    #[test]
    fn map_index_defaults_to_felucca_and_updates_via_on_map_change() {
        // Feeds `build_scene`'s "facet" field directly (`s.world.map_index`, no
        // further shaping) — see `World::map_index`'s doc.
        let mut w = World::default();
        assert_eq!(w.map_index, 0, "facet defaults to Felucca (0)");
        w.player = Some(Serial(1));
        w.mobile_mut(1).pos = Position {
            x: 100,
            y: 100,
            z: 0,
        };
        w.on_map_change(2); // Ilshenar
        assert_eq!(w.map_index, 2);
    }

    // ---- synthetic staircase tests for calculate_new_z's pure cores ----------
    //
    // A Bridge-flagged static (ClassicUO `ItemData.IsBridge`, ServUO
    // `ItemData.Bridge`) stands at HALF height — `avg_z = z + height/2` — not
    // its full top surface (`z + height`). This is intentional on BOTH
    // references (ClassicUO `CreateItemList`'s `staticAverageZ /= 2`; ServUO
    // `TileData.CalcHeight` halves for `Bridge` too), and it's what makes a
    // staircase built from stacked Bridge tiles climb in the first place — a
    // synthetic run of 5-tall stair statics based at z=0,5,10,15,20 (as this
    // test was originally going to assert should read as its FULL top surface)
    // would have been asserting the wrong behavior; these tests assert the
    // *correct*, half-height one instead, and that a UNIFORMLY-built staircase
    // (each tile based exactly at the half-height of the one before) climbs by
    // an EVEN delta per tile — proving the unevenness on the real Britain-bank
    // stair (+2, +5, +3) comes from THAT staircase's non-uniform geometry
    // (mixed static heights/bases), not from the algorithm.
    fn bridge_tile(z: i32, height: i32) -> PathObj {
        PathObj {
            flags: POF_IMPASS | POF_SURFACE | POF_BRIDGE,
            z,
            avg_z: z + height / 2,
            height,
            land_stretched: false,
        }
    }

    #[test]
    fn bridge_tile_stands_at_half_height_not_top_surface() {
        // A single 5-tall stair static at z=0 (top surface = 5): standing Z must
        // be the half-height average (0 + 5/2 = 2), not the top (5).
        let list = vec![bridge_tile(0, 5)];
        let (min_z, max_z) = bound_min_max_z(&[bridge_tile(0, 5)], 0, 0);
        let z = resolve_standing_z(list, min_z, max_z, 0).expect("stands on the bridge tile");
        assert_eq!(
            z, 2,
            "Bridge standing Z is z + height/2, not the top surface (5)"
        );
    }

    #[test]
    fn synthetic_staircase_climbs_and_descends_evenly() {
        // 5 tiles, each an 8-tall Bridge riser based exactly at the HALF-height
        // (avg) of the tile before: bases 0,4,8,12,16 -> avgs 4,8,12,16,20. If
        // this geometry is uniform, `calculate_new_z` (via its two pure cores)
        // should climb by the SAME +4 delta every tile.
        let tiles: Vec<PathObj> = (0..5).map(|i| bridge_tile(4 * i, 8)).collect();

        // Start already standing on tile 0 (avg 4), then climb through 1..4.
        let mut z = tiles[0].avg_z; // 4
        let mut seq = vec![z];
        for i in 1..tiles.len() {
            let (min_z, max_z) = bound_min_max_z(&[tiles[i - 1]], z, 0);
            z = resolve_standing_z(vec![tiles[i]], min_z, max_z, z).expect("climbs the next riser");
            seq.push(z);
        }
        assert_eq!(
            seq,
            vec![4, 8, 12, 16, 20],
            "uniform risers climb by an even +4 delta each tile"
        );

        // Descend back down through 3..0 — must mirror the climb exactly.
        let mut z = tiles[4].avg_z; // 20
        let mut seq = vec![z];
        for i in (0..4).rev() {
            let (min_z, max_z) = bound_min_max_z(&[tiles[i + 1]], z, 0);
            z = resolve_standing_z(vec![tiles[i]], min_z, max_z, z)
                .expect("descends the next riser down");
            seq.push(z);
        }
        assert_eq!(
            seq,
            vec![20, 16, 12, 8, 4],
            "descent mirrors the climb exactly"
        );
    }

    // Real-data regression for the Britain West Bank staircase (facet 0, x=1495,
    // y=1625..1629) — the tiles a live ANIMA_DEBUG capture flagged as "janky":
    // climbing north the resolved Z went 10 -> 12 -> 17 -> 20 (deltas +2, +5,
    // +3), and the first stair static's *top* surface (z+height) is 15 while the
    // resolved standing Z is only 12 — 3 below it. That looked like a bug (stand
    // ON the stair, not 3 below), so this test hand-derives what
    // `calculate_new_z` + the REAL tile data (dumped via `MapData::land`/
    // `statics`) should produce, to check whether 10,12,17,20 is actually right.
    //
    // Dumped real data (facet 0):
    //   (1495,1627) land g=0x03eb z=10 flags=0            — flat, walkable
    //     static g=0x0739 z=10 h=5  flags surf+bridge      (avg = z + h/2 = 12)
    //   (1495,1626) land g=0x03ec z=10 flags=0
    //     static g=0x0738 z=10 h=10 flags surf+bridge      (avg = 10 + 5 = 15)
    //     static g=0x0739 z=15 h=5  flags surf+bridge      (avg = 15 + 2 = 17)
    //   (1495,1625) land g=0x0401 z=10 flags=0
    //     static g=0x04ab z=20 h=0  flags surf (not bridge) (avg = z + h = 20)
    //     static g=0x04ba z=40 h=0  flags surf              (avg = 40)
    //     static g=0x013a z=40 h=20 impassable only          (a wall, not standable)
    //     (+ other impassable-only wall statics — none are candidate surfaces)
    //   (1495,1628) land g=0x0401 z=10 flags=0, no statics  — flat, walkable
    //   (1495,1629) land g=0x03ec z=10 flags=0, no statics  — flat, walkable (start)
    //
    // `Bridge` (stair) tiles stand at HALF height (ClassicUO
    // `staticAverageZ /= 2` in `CreateItemList`; ServUO `ItemData.CalcHeight`
    // does the identical halving) — by design, NOT the tile's raw top surface.
    // Hand-running `calculate_new_z` (`CalculateMinMaxZ` bounds the step by the
    // tile left behind, then the candidate nearest current Z with BLOCK_HEIGHT
    // clearance wins):
    //   1629(z10) -> 1628: flat both sides -> 10 (unchanged, trivial)
    //   1628(z10) -> 1627: bound from 1628 (flat) gives min=10,max=12; land(10)
    //     and static 0x0739(avg12) are candidates under the z=128 sky sentinel;
    //     nearest to current_z=10 with clearance is avg=12 -> **12**
    //   1627(z12) -> 1626: bound from 1627 (bridge avg12==current_z -> max
    //     bumped to z+height=15) gives min=12,max=17; candidates land(10),
    //     0x0738(avg15), 0x0739(avg17); nearest to 12 with clearance is 0x0739
    //     avg=17 (0x0738's avg 15 fails the `tavg >= cur_z` ordering test) ->
    //     **17**
    //   1626(z17) -> 1625: bound from 1626 (bridge avg17==current_z -> max
    //     bumped to z+height=20) gives min=15,max=22; only 0x04ab (avg20) has
    //     clearance and fits within max=22 -> **20**
    // So the captured sequence 10,12,17,20 IS the correct output of the ported
    // algorithm on the real data — not a bug. The "3 below the top" the capture
    // flagged is the Bridge half-height rule working as intended (see
    // `calculate_new_z`'s doc); the real jank is client-side easing (fixed in
    // `web/main.js`: see `RZ_CATCHUP`), not this Z resolution.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn britain_bank_stair_z_sequence_matches_captured_climb() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        let world = anima_core::World::new();
        const X: i64 = 1495;
        const NORTH: u8 = 0;
        const SOUTH: u8 = 4;

        // Climb north (y decreasing): 1629 -> 1628 -> 1627 -> 1626 -> 1625.
        let mut z = 10i32;
        let mut seq = vec![z];
        for y in [1628i64, 1627, 1626, 1625] {
            z = calculate_new_z(&world, &mut map, None, X, y, z, NORTH)
                .expect("stair climbs north");
            seq.push(z);
        }
        assert_eq!(
            seq,
            vec![10, 10, 12, 17, 20],
            "climbing-north Z sequence (trivial 10->10 step included)"
        );

        // Descend south (y increasing), mirroring the climb exactly.
        let mut z = 20i32;
        let mut seq = vec![z];
        for y in [1626i64, 1627, 1628, 1629] {
            z = calculate_new_z(&world, &mut map, None, X, y, z, SOUTH)
                .expect("stair descends south");
            seq.push(z);
        }
        assert_eq!(
            seq,
            vec![20, 17, 12, 10, 10],
            "descending-south Z sequence (trivial 10->10 step included)"
        );
    }

    /// Root-cause regression for the live `walkto (1621,1588) rejected: no
    /// path from (1620,1595,5)` bug: (1621,1588) sits behind a real, closed
    /// "wooden door" (graphic 0x06A5/0x06A7, tiledata Door+Impassable) at
    /// (1611,1591)/(1612,1591) — a genuine ServUO gate a live probe walked up
    /// to (confirmed live: opening it with `use:<serial>` made the very same
    /// `walkto` succeed). The strict check must still deny it (a closed door
    /// really does block a live step); the planning check must not (so click-
    /// to-walk can route through, and the executor can open it) — and
    /// `door_blocking_at` must find its serial so the executor knows to.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn closed_door_blocks_strictly_but_not_for_planning() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        assert!(
            map.item_is_door(0x06A5),
            "0x06A5 should be a real door graphic"
        );

        let mut world = anima_core::World::new();
        let door_serial = 1_073_751_127;
        world.items.insert(
            door_serial,
            anima_core::world::Item {
                serial: door_serial,
                graphic: 0x06A5,
                amount: 1,
                pos: anima_core::types::Position {
                    x: 1611,
                    y: 1591,
                    z: 0,
                },
                container: None,
                layer: 0,
                hue: 0,
                name: String::new(),
                direction: 0,
                is_multi: false,
            },
        );

        // Strict (manual-walk / minimap) check: the closed door really blocks.
        match explain_tile_walkable(&world, &mut map, None, 1611, 1591, 5) {
            Err(StepDeny::DynamicItem { graphic, .. }) => assert_eq!(graphic, 0x06A5),
            other => panic!("expected a closed-door deny, got {other:?}"),
        }
        assert!(!tile_walkable(&world, &mut map, None, 1611, 1591, 5));

        // Planning check: the same closed door does not block.
        assert!(
            tile_walkable_for_planning(&world, &mut map, None, 1611, 1591, 5).is_some(),
            "click-to-walk planning must route through a closed (openable) door"
        );

        // The executor can find the door to open.
        assert_eq!(
            door_blocking_at(&world, &map, 1611, 1591, 5),
            Some(door_serial)
        );
        assert_eq!(
            door_blocking_at(&world, &map, 1611, 1591 /* unrelated tile */ + 1, 5),
            None
        );
    }

    /// FIX 4 regression: a door AND a non-door blocker (e.g. a crate someone
    /// dropped in the doorway) sitting on the SAME tile must still deny
    /// planning. Before this fix, the door-recovery branch fired the moment
    /// `explain_tile_walkable`'s `.find()` reported ANY door on the tile,
    /// then recomputed with the STATIC-only `walkable_z` — silently ignoring
    /// every OTHER dynamic item there too. Since `World::items` is a
    /// `HashMap`, which blocker `.find()` hits first is iteration-order
    /// dependent, not a real answer — this asserts under two different
    /// serial-number arrangements for the pair (a `HashMap`'s iteration
    /// order is a function of its keys, not insertion sequence) so the
    /// fixed "every blocker must be a door" check can't quietly regress back
    /// to a `.find()`-shaped bug that just happens to pass for one layout.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn tile_walkable_for_planning_denies_a_door_tile_with_a_non_door_blocker_too() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        assert!(
            map.item_is_door(0x06A5),
            "0x06A5 should be a real door graphic"
        );
        assert!(
            !map.item_is_door(0x0E3D),
            "0x0E3D (crate) should not be a door"
        );
        assert!(
            map.item_blocks(0x0E3D, 5, 5),
            "0x0E3D (crate) should be an impassable blocker at these Zs"
        );

        for (door_serial, crate_serial) in [(1u32, 2u32), (2u32, 1u32)] {
            let mut world = anima_core::World::new();
            world.items.insert(
                door_serial,
                anima_core::world::Item {
                    serial: door_serial,
                    graphic: 0x06A5,
                    amount: 1,
                    pos: anima_core::types::Position {
                        x: 1611,
                        y: 1591,
                        z: 0,
                    },
                    container: None,
                    layer: 0,
                    hue: 0,
                    name: String::new(),
                    direction: 0,
                    is_multi: false,
                },
            );
            world.items.insert(
                crate_serial,
                anima_core::world::Item {
                    serial: crate_serial,
                    graphic: 0x0E3D,
                    amount: 1,
                    pos: anima_core::types::Position {
                        x: 1611,
                        y: 1591,
                        z: 5,
                    },
                    container: None,
                    layer: 0,
                    hue: 0,
                    name: String::new(),
                    direction: 0,
                    is_multi: false,
                },
            );
            assert!(
                tile_walkable_for_planning(&world, &mut map, None, 1611, 1591, 5).is_none(),
                "a crate blocking the same tile as an openable door must still deny planning \
                 (door_serial={door_serial}, crate_serial={crate_serial})"
            );
        }
    }

    /// The `dr` land-tile field's source of truth: `explain_tile_walkable_for_planning`
    /// must name a door's serial exactly when the tile is walkable-for-planning ONLY
    /// because of that door — never when the tile is either strictly walkable outright
    /// (no door involved) or, per the FIX 4 rule, blocked by something a door can't
    /// excuse (e.g. a crate sharing the doorway). Exercises the fn directly rather than
    /// `build_scene`'s JSON output, since assembling a full `Session`/scene just to read
    /// one optional field would be needless overhead — and this is the exact function
    /// `build_scene`'s `dr` suffix calls.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn explain_tile_walkable_for_planning_names_the_door_only_when_it_alone_blocks() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        assert!(
            map.item_is_door(0x06A5),
            "0x06A5 should be a real door graphic"
        );
        assert!(
            !map.item_is_door(0x0E3D),
            "0x0E3D (crate) should not be a door"
        );

        let door_serial = 1_073_751_127;
        let mut world = anima_core::World::new();
        world.items.insert(
            door_serial,
            anima_core::world::Item {
                serial: door_serial,
                graphic: 0x06A5,
                amount: 1,
                pos: anima_core::types::Position {
                    x: 1611,
                    y: 1591,
                    z: 0,
                },
                container: None,
                layer: 0,
                hue: 0,
                name: String::new(),
                direction: 0,
                is_multi: false,
            },
        );

        // A tile with ONLY a closed door: walkable-for-planning, and the door's
        // own serial comes back with it.
        let (z, door) = explain_tile_walkable_for_planning(&world, &mut map, None, 1611, 1591, 5);
        assert!(z.is_some(), "a door-only tile must be walkable for planning");
        assert_eq!(
            door,
            Some(door_serial),
            "must name the door blocking this tile"
        );

        // The SAME tile, plus a non-door impassable crate: no longer plannable at
        // all (FIX 4 — every blocker must be a door, not just one of them), so no
        // door serial either.
        let crate_serial = 2u32;
        world.items.insert(
            crate_serial,
            anima_core::world::Item {
                serial: crate_serial,
                graphic: 0x0E3D,
                amount: 1,
                pos: anima_core::types::Position {
                    x: 1611,
                    y: 1591,
                    z: 5,
                },
                container: None,
                layer: 0,
                hue: 0,
                name: String::new(),
                direction: 0,
                is_multi: false,
            },
        );
        let (z, door) = explain_tile_walkable_for_planning(&world, &mut map, None, 1611, 1591, 5);
        assert!(
            z.is_none(),
            "a crate alongside the door must still deny planning"
        );
        assert_eq!(
            door, None,
            "no door serial when the tile isn't even plannable"
        );

        // An ordinary walkable tile with no items at all: walkable outright, and
        // (since no door was ever involved) no door serial.
        let empty_world = anima_core::World::new();
        let (z, door) =
            explain_tile_walkable_for_planning(&empty_world, &mut map, None, 1611, 1591, 5);
        assert!(z.is_some(), "an ordinary tile with no items must be walkable");
        assert_eq!(
            door, None,
            "an ordinary walkable tile must not get a door serial"
        );
    }

    // ---- FIX 1/2/6/7: multi-component walkability + rendering -----------------

    fn synth_item(
        serial: u32,
        graphic: u16,
        x: u16,
        y: u16,
        z: i8,
        is_multi: bool,
    ) -> anima_core::world::Item {
        anima_core::world::Item {
            serial,
            graphic,
            amount: 1,
            pos: anima_core::types::Position { x, y, z },
            container: None,
            layer: 0,
            hue: 0,
            name: String::new(),
            direction: 0,
            is_multi,
        }
    }

    /// FIX 7 (pure, no map/file data needed): [`multi_components_at`] must
    /// force-include an invisible index-0 (`is_origin`) component — matching
    /// ServUO's own collision grid (`Server/MultiData.cs::MultiComponentList`:
    /// `if (i == 0 || allTiles[i].m_Flags != 0)`) — while still dropping any
    /// OTHER invisible component, and never returning a component from a
    /// different tile or a different (non-multi) `World::items` entry.
    #[test]
    fn multi_components_at_includes_invisible_origin_but_not_invisible_others() {
        let mut world = anima_core::World::new();
        world
            .items
            .insert(1, synth_item(1, 42, 1000, 1000, -15, true)); // graphic 42 = multi id
        world
            .items
            .insert(2, synth_item(2, 0x0BB8, 1000, 1000, 0, false)); // an ordinary item, ignored

        let multis = Multis::from_components(std::collections::HashMap::from([(
            42,
            vec![
                MultiComponent {
                    graphic: 0x1000,
                    dx: 0,
                    dy: 0,
                    dz: 0,
                    visible: false,
                    is_origin: true,
                },
                MultiComponent {
                    graphic: 0x1001,
                    dx: 0,
                    dy: 0,
                    dz: 4,
                    visible: false,
                    is_origin: false,
                },
                MultiComponent {
                    graphic: 0x1002,
                    dx: 0,
                    dy: 0,
                    dz: 8,
                    visible: true,
                    is_origin: false,
                },
                MultiComponent {
                    graphic: 0x2000,
                    dx: 1,
                    dy: 0,
                    dz: 0,
                    visible: true,
                    is_origin: false,
                },
            ],
        )]));

        let here = multi_components_at(&world, &multis, 1000, 1000);
        assert_eq!(
            here.len(),
            2,
            "invisible origin + visible component only: {here:?}"
        );
        assert!(
            here.contains(&(0x1000, -15)),
            "invisible index-0 origin must still count for walkability"
        );
        assert!(here.contains(&(0x1002, -7)), "visible component must count");
        assert!(
            !here.iter().any(|&(g, _)| g == 0x1001),
            "invisible NON-origin component must be excluded"
        );

        assert_eq!(
            multi_components_at(&world, &multis, 1001, 1000),
            vec![(0x2000, -15)]
        );
        assert!(
            multi_components_at(&world, &multis, 50_000, 50_000).is_empty(),
            "way outside any multi's footprint must return nothing"
        );
    }

    #[test]
    fn placement_json_none_when_nothing_pending() {
        let world = anima_core::World::new();
        let multis = Multis::from_components(std::collections::HashMap::new());
        assert!(placement_json(&world, Some(&multis)).is_none());
    }

    #[test]
    fn placement_json_none_without_multis_even_if_pending() {
        let mut world = anima_core::World::new();
        world.pending_target = Some(TargetCursor {
            target_type: 1,
            cursor_id: 1,
            cursor_flag: 0,
        });
        world.pending_multi_placement = Some(MultiPlacement {
            multi_id: 0x64,
            x_off: 0,
            y_off: 0,
            z_off: 0,
            hue: 0,
        });
        assert!(
            placement_json(&world, None).is_none(),
            "no Multis loaded -> omit rather than error"
        );
    }

    #[test]
    fn placement_json_none_for_unknown_multi_id() {
        let mut world = anima_core::World::new();
        world.pending_target = Some(TargetCursor {
            target_type: 1,
            cursor_id: 1,
            cursor_flag: 0,
        });
        world.pending_multi_placement = Some(MultiPlacement {
            multi_id: 0xFFFF, // no entry in `multis` below
            x_off: 0,
            y_off: 0,
            z_off: 0,
            hue: 0,
        });
        let multis = Multis::from_components(std::collections::HashMap::new());
        assert!(placement_json(&world, Some(&multis)).is_none());
    }

    /// A stale `pending_multi_placement` (the browser task's real bug target)
    /// must not render once the target cursor it belonged to is gone —
    /// `respond_target`/`cancel_target` (`lib.rs`, outside this crate's game
    /// packet handlers) clear `pending_target` the instant the reply is sent
    /// but can't also reach `pending_multi_placement`, so `placement_json`
    /// gates on BOTH (see its doc).
    #[test]
    fn placement_json_none_once_target_answered_or_cancelled() {
        let mut world = anima_core::World::new();
        world.pending_target = None; // already answered/cancelled
        world.pending_multi_placement = Some(MultiPlacement {
            multi_id: 0x64,
            x_off: 0,
            y_off: 0,
            z_off: 0,
            hue: 0,
        });
        let multis = Multis::from_components(std::collections::HashMap::from([(
            0x64,
            vec![MultiComponent {
                graphic: 0x1,
                dx: 0,
                dy: 0,
                dz: 0,
                visible: true,
                is_origin: true,
            }],
        )]));
        assert!(placement_json(&world, Some(&multis)).is_none());
    }

    #[test]
    fn placement_json_dedupes_footprint_and_carries_offsets_hue() {
        let mut world = anima_core::World::new();
        world.pending_target = Some(TargetCursor {
            target_type: 1,
            cursor_id: 0xBEEF,
            cursor_flag: 0,
        });
        world.pending_multi_placement = Some(MultiPlacement {
            multi_id: 0x64,
            x_off: 3,
            y_off: 4,
            z_off: 1,
            hue: 0x21,
        });
        let multis = Multis::from_components(std::collections::HashMap::from([(
            0x64,
            vec![
                MultiComponent {
                    graphic: 0x1,
                    dx: 0,
                    dy: 0,
                    dz: 0,
                    visible: true,
                    is_origin: true,
                },
                // A second-floor component stacked on the SAME (dx, dy) as the
                // origin — the outline only needs the tile once.
                MultiComponent {
                    graphic: 0x2,
                    dx: 0,
                    dy: 0,
                    dz: 4,
                    visible: true,
                    is_origin: false,
                },
                MultiComponent {
                    graphic: 0x3,
                    dx: 1,
                    dy: 0,
                    dz: 0,
                    visible: true,
                    is_origin: false,
                },
                MultiComponent {
                    graphic: 0x4,
                    dx: 0,
                    dy: 1,
                    dz: 0,
                    visible: false,
                    is_origin: false,
                },
            ],
        )]));

        let v = placement_json(&world, Some(&multis)).expect("placement pending");
        assert_eq!(v["multiId"], 0x64);
        assert_eq!(v["hue"], 0x21);
        assert_eq!(v["xOff"], 3);
        assert_eq!(v["yOff"], 4);
        assert_eq!(v["zOff"], 1);
        let mut tiles: Vec<(i64, i64)> = v["tiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| (t[0].as_i64().unwrap(), t[1].as_i64().unwrap()))
            .collect();
        tiles.sort();
        assert_eq!(
            tiles,
            vec![(0, 0), (0, 1), (1, 0)],
            "deduped by (dx, dy): {tiles:?}"
        );
    }

    /// End-to-end against REAL `SmallOldHouse` (multi id `0x64`, ServUO
    /// `Multis/Houses.cs`) component data — confirms `placement_json` resolves
    /// a genuine multi (not just the synthetic lists the tests above use) and
    /// actually dedupes: the module doc records ~148 raw components for this
    /// id, most of them floors/roof stacked over a much smaller set of
    /// distinct `(dx, dy)` footprint tiles.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn placement_json_real_house_multi_produces_deduped_footprint() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let multis = Multis::open(&dir).expect("open multi data");

        let mut world = anima_core::World::new();
        world.pending_target = Some(TargetCursor {
            target_type: 1,
            cursor_id: 1,
            cursor_flag: 0,
        });
        world.pending_multi_placement = Some(MultiPlacement {
            multi_id: 0x64,
            x_off: 0,
            y_off: 0,
            z_off: 0,
            hue: 0,
        });

        let v = placement_json(&world, Some(&multis)).expect("SmallOldHouse must resolve");
        let raw_len = multis.components(0x64).expect("id 0x64 must exist").len();
        let tiles = v["tiles"].as_array().unwrap();
        assert!(!tiles.is_empty());
        assert!(
            tiles.len() < raw_len,
            "deduped outline ({}) must be smaller than the raw component list ({raw_len})",
            tiles.len()
        );
        // (0, 0) is always the origin tile — every multi's index-0 component
        // sits at its own origin.
        assert!(
            tiles.iter().any(|t| t[0] == 0 && t[1] == 0),
            "origin tile (0,0) must be part of the footprint: {v}"
        );
        // `parts` carries the real (non-deduped) component list — the browser
        // draws the actual house shape from these, not just the footprint.
        let parts = v["parts"].as_array().unwrap();
        assert!(!parts.is_empty(), "parts must be non-empty: {v}");
        assert!(
            parts.len() >= tiles.len(),
            "raw component list ({}) must be at least as long as the deduped footprint ({})",
            parts.len(),
            tiles.len()
        );
        for p in parts {
            let entry = p.as_array().expect("each part is a 4-element array");
            assert_eq!(entry.len(), 4, "each part is [dx, dy, dz, graphic]: {p}");
        }
    }

    #[test]
    fn placement_json_caps_tile_count() {
        let mut world = anima_core::World::new();
        world.pending_target = Some(TargetCursor {
            target_type: 1,
            cursor_id: 1,
            cursor_flag: 0,
        });
        world.pending_multi_placement = Some(MultiPlacement {
            multi_id: 1,
            x_off: 0,
            y_off: 0,
            z_off: 0,
            hue: 0,
        });
        // 100 * 50 = 5000 distinct (dx, dy) offsets, comfortably over the cap.
        let mut comps = Vec::new();
        for dx in 0..100i16 {
            for dy in 0..50i16 {
                comps.push(MultiComponent {
                    graphic: 1,
                    dx,
                    dy,
                    dz: 0,
                    visible: true,
                    is_origin: dx == 0 && dy == 0,
                });
            }
        }
        let multis = Multis::from_components(std::collections::HashMap::from([(1, comps)]));
        let v = placement_json(&world, Some(&multis)).unwrap();
        assert_eq!(
            v["tiles"].as_array().unwrap().len(),
            PLACEMENT_TILE_CAP,
            "must cap rather than dump every distinct offset"
        );
    }

    #[test]
    fn house_design_json_none_when_not_customizing() {
        let world = anima_core::World::new();
        assert!(house_design_json(&world, None).is_none());
    }

    #[test]
    fn house_design_json_none_without_a_known_foundation_item() {
        let mut world = anima_core::World::new();
        // The 0xBF/0x20 notice arrived, but the foundation's own item state hasn't (yet).
        world.customizing_house = Some(0x4003_0001);
        assert!(
            house_design_json(&world, None).is_none(),
            "no known world position -> omit rather than guess"
        );
    }

    #[test]
    fn house_design_json_carries_serial_revision_and_world_position() {
        let mut world = anima_core::World::new();
        let serial = 0x4003_0002u32;
        world.customizing_house = Some(serial);
        world.item_mut(serial).graphic = 0x64; // arbitrary multi id, unknown to `multis` below
        world.item_mut(serial).pos = Position {
            x: 100,
            y: 200,
            z: 5,
        };

        // No 0xD8 design details have arrived yet -> revision defaults to 0.
        let v = house_design_json(&world, None).expect("customizing a known foundation");
        assert_eq!(v["serial"], serial);
        assert_eq!(v["revision"], 0);
        assert_eq!(v["floors"], 3, "no multis loaded -> conservative default");
        assert_eq!(v["x"], 100);
        assert_eq!(v["y"], 200);
        assert_eq!(v["z"], 5);

        let mut design = anima_core::world::HouseDesign::default();
        design.revision = 7;
        world.house_designs.insert(serial, design);
        let v = house_design_json(&world, None).unwrap();
        assert_eq!(v["revision"], 7, "once the 0xD8 arrives, its revision wins");
    }

    #[test]
    fn house_design_max_levels_uses_the_14_tile_rule() {
        let mut comps = std::collections::HashMap::new();
        comps.insert(
            1,
            vec![
                MultiComponent {
                    graphic: 1,
                    dx: -3,
                    dy: -3,
                    dz: 0,
                    visible: true,
                    is_origin: true,
                },
                MultiComponent {
                    graphic: 1,
                    dx: 3,
                    dy: 3,
                    dz: 0,
                    visible: true,
                    is_origin: false,
                },
            ], // 7x7 footprint -> below the 14-tile threshold
        );
        comps.insert(
            2,
            vec![
                MultiComponent {
                    graphic: 1,
                    dx: -7,
                    dy: -7,
                    dz: 0,
                    visible: true,
                    is_origin: true,
                },
                MultiComponent {
                    graphic: 1,
                    dx: 6,
                    dy: 6,
                    dz: 0,
                    visible: true,
                    is_origin: false,
                },
            ], // exactly 14x14 -> at the threshold
        );
        let multis = Multis::from_components(comps);
        assert_eq!(house_design_max_levels(Some(&multis), 1), 3);
        assert_eq!(house_design_max_levels(Some(&multis), 2), 4);
        assert_eq!(
            house_design_max_levels(Some(&multis), 999),
            3,
            "unknown multi id -> conservative default"
        );
        assert_eq!(
            house_design_max_levels(None, 1),
            3,
            "no multis loaded -> conservative default"
        );
    }

    /// T4: once a custom-house design is decoded (`tiles_ready`), it must
    /// REPLACE the foundation's multi.mul components for that multi entirely
    /// — never merge with them — matching the identical swap the emission
    /// loop in `build_scene` makes (§3c/§3d of the housing plan).
    #[test]
    fn multi_components_at_design_replaces_multi_mul_components() {
        let mut world = anima_core::World::new();
        world
            .items
            .insert(1, synth_item(1, 42, 1000, 1000, 0, true)); // graphic 42 = multi id

        let multis = Multis::from_components(std::collections::HashMap::from([(
            42,
            vec![MultiComponent {
                graphic: 0x1000,
                dx: 0,
                dy: 0,
                dz: 0,
                visible: true,
                is_origin: true,
            }],
        )]));

        // No design yet (or not decoded) → the stock multi.mul component wins.
        assert_eq!(
            multi_components_at(&world, &multis, 1000, 1000),
            vec![(0x1000, 0)]
        );

        let mut design = anima_core::world::HouseDesign::default();
        design.tiles.insert((0, 0), vec![(0x4001, 5)]);
        world.house_designs.insert(1, design); // tiles_ready still false — must be ignored
        assert_eq!(
            multi_components_at(&world, &multis, 1000, 1000),
            vec![(0x1000, 0)],
            "a design that isn't tiles_ready yet must not replace anything"
        );

        world.house_designs.get_mut(&1).unwrap().tiles_ready = true;
        assert_eq!(
            multi_components_at(&world, &multis, 1000, 1000),
            vec![(0x4001, 5)],
            "a tiles_ready design must replace the multi.mul component entirely, not merge"
        );
    }

    /// FIX 1 + FIX 7, end-to-end against REAL SmallBoat multi data (id 0,
    /// ServUO `SmallBoat.NorthID`) and a REAL clear-open-water spot: probed
    /// directly (see the `anima-assets` `probe_water`-style scan this test's
    /// coordinates came from) — every tile in a 7×7 box around (1459,1767) is
    /// impassable deep water with zero real statics, so any walkability there
    /// comes ONLY from the synthetic boat placed by this test, not dock
    /// clutter. Component offsets/flags below were read directly off this
    /// boat id's real component list (`Multis::components(0)`):
    /// `(dx=0,dy=-2)` is graphic `0x3EAC` (Surface, visible, height 3 — a deck
    /// plank); `(dx=-2,dy=-1)` is graphic `0x3EB1` (Impassable, visible — a
    /// hull side piece). Both at `dz=0`: every SmallBoat component sits
    /// coplanar with the multi's own Z (verified: every one of the 38
    /// components for every facing has `dz == 0`).
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn fix1_smallboat_deck_walkable_hull_blocks_using_real_boat_data() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        let multis = Multis::open(&dir).expect("open multi data");

        // Confirm the fixture assumption: a clear 7x7 open-water box, no statics.
        for oy in -3i64..=3 {
            for ox in -3i64..=3 {
                let (tx, ty) = ((1459 + ox) as u32, (1767 + oy) as u32);
                assert!(
                    map.land(tx, ty).impassable(),
                    "({tx},{ty}) should be deep water"
                );
                assert!(
                    map.statics(tx, ty).is_empty(),
                    "({tx},{ty}) should have no real statics"
                );
            }
        }

        // A SmallBoat (multi id 0) "placed" at (1459,1767); this shard snaps a
        // placed boat's Z to -15 (see FIX 3's live verification) — the exact
        // value doesn't matter to this check (only that the deck resolves
        // within one climb-step of it), so it's used as-observed.
        let (boat_x, boat_y, boat_z): (i64, i64, i32) = (1459, 1767, -15);
        let mut world = anima_core::World::new();
        world.items.insert(
            1,
            synth_item(1, 0, boat_x as u16, boat_y as u16, boat_z as i8, true),
        );

        // Deck tile: must be walkable, `tile_walkable` (the renderer's `w`
        // flag) and `tile_walkable_for_planning` (the click-to-walk A*
        // terrain adapter) must agree, and the standing Z must be the deck's
        // own top (boat_z + height 3).
        let (deck_x, deck_y) = (boat_x, boat_y - 2);
        assert_eq!(
            explain_tile_walkable(&world, &mut map, Some(&multis), deck_x, deck_y, boat_z),
            Ok(boat_z + 3),
            "the deck component must contribute a standing surface over open water"
        );
        assert!(tile_walkable(
            &world,
            &mut map,
            Some(&multis),
            deck_x,
            deck_y,
            boat_z
        ));
        assert!(tile_walkable_for_planning(
            &world,
            &mut map,
            Some(&multis),
            deck_x,
            deck_y,
            boat_z
        )
        .is_some());
        // Without the boat, the SAME tile is unwalkable open water — proves
        // the deck (not some coincidental real static) is what's carrying it.
        assert!(!tile_walkable(
            &world, &mut map, None, deck_x, deck_y, boat_z
        ));

        // Hull tile: must deny. HONEST finding (this is exactly the FIX 3
        // re-verification the review asked for): the hull piece (0x3EB1) is
        // Impassable-only, contributing NO standing candidate of its own
        // (`score_walkable_z`'s candidate rule is `surface() && !impassable()`)
        // — and this exact (dx,dy) has no OTHER (deck) component sharing it.
        // So the deny reason is `NoSurface` (nothing to stand on at all), NOT
        // `Blocked` (an overlapping impassable object stepping on an
        // otherwise-valid candidate) — `multi_blocker_at`'s DynamicItem path
        // never even fires here, because `walkable_z_explain` already denies
        // first. Either way the OBSERVABLE result is the same: the hull tile
        // is unwalkable, matching what a real player sees at the ship's rail.
        let (hull_x, hull_y) = (boat_x - 2, boat_y - 1);
        match explain_tile_walkable(&world, &mut map, Some(&multis), hull_x, hull_y, boat_z) {
            Err(StepDeny::Terrain(ZReason::NoSurface)) => {}
            other => panic!("expected a NoSurface deny at the hull tile, got {other:?}"),
        }
        assert!(!tile_walkable(
            &world,
            &mut map,
            Some(&multis),
            hull_x,
            hull_y,
            boat_z
        ));
    }

    /// Real-bug regression (the actual root cause this fix addresses, distinct
    /// from FIX 1's boat-as-multi case above): on THIS shard, a boat's deck
    /// arrives as an ORDINARY DYNAMIC WORLD ITEM, not as a multi component at
    /// all — live-verified graphics 0x3EA1/0x3EAC/0x3EB0 at `z=-5`, tiledata
    /// `height` 3, `SURFACE` (not `Impassable`), giving a standing Z of
    /// `-5 + 3 = -2`. Before `dynamic_statics_at` existed,
    /// `explain_tile_walkable` only ever folded MULTI components into its
    /// scoring — an ordinary item was consulted ONLY as a blocker — so this
    /// exact tile shape denied every step, with `sz` falling back to the water
    /// underneath. Reuses the same known-clear open-water box (no real
    /// statics, all impassable deep water) `fix1_smallboat_...` verified above,
    /// so the standing surface is provably coming from the synthetic dynamic
    /// item, not some coincidental real static.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn dynamic_item_deck_over_water_contributes_a_standing_surface() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        for (tx, ty) in [(1459u32, 1765u32), (1460u32, 1765u32)] {
            assert!(map.land(tx, ty).impassable(), "({tx},{ty}) should be deep water");
            assert!(map.statics(tx, ty).is_empty(), "({tx},{ty}) should have no real statics");
        }

        // Deck alone, as a plain (non-multi) dynamic item: must contribute a
        // standing surface exactly like a real static would.
        let mut world = anima_core::World::new();
        world
            .items
            .insert(1, synth_item(1, 0x3EAC, 1459, 1765, -5, false));
        assert_eq!(
            explain_tile_walkable(&world, &mut map, None, 1459, 1765, -5),
            Ok(-2),
            "a dynamic deck item must contribute a standing surface over open water"
        );
        assert!(tile_walkable(&world, &mut map, None, 1459, 1765, -5));

        // Without it, the SAME tile is unwalkable open water.
        world.items.remove(&1);
        assert!(!tile_walkable(&world, &mut map, None, 1459, 1765, -5));

        // Companion check: a hull piece (impassable, no surface) as a plain
        // dynamic item on a DIFFERENT clear-water tile must still fully deny —
        // it must never itself become a standing candidate (see
        // `dynamic_statics_at`'s doc: impassable items are excluded from the
        // surface fold, so this keeps denying through the unchanged
        // `StepDeny::DynamicItem`/blocker path, not by accident).
        world
            .items
            .insert(2, synth_item(2, 0x3EB1, 1460, 1765, -5, false));
        assert!(!tile_walkable(&world, &mut map, None, 1460, 1765, -5));
    }

    /// FIX 2: a multi's own roof component must lift `max_draw_z`'s ceiling
    /// exactly like a real static roof would — the static map alone has no
    /// idea a multi is even there, so without this a boat/house roof would
    /// never cull and the interior would never show. Uses a real ROOF-flagged
    /// graphic (0x0586, `FLAG_ROOF` set, non-surface, height 3 — probed
    /// directly off tiledata.mul) so the tiledata half of the check is real,
    /// not synthetic.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn max_draw_z_culls_a_multi_roof_component_above_the_player() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        assert_ne!(
            map.item_flags(0x0586) & FLAG_ROOF,
            0,
            "0x0586 should be a real roof graphic"
        );

        // New Haven spawn: outdoors, open sky — baseline with no multi at all.
        let (px, py, pz) = (3503i64, 2574, 0i32);
        let baseline = max_draw_z(&anima_core::World::new(), &mut map, None, px, py, pz);
        assert_eq!(
            baseline, 127,
            "open field with no roof over the player: draw everything"
        );

        // A synthetic multi whose one component (the real roof graphic) sits
        // directly over the tile the player faces into (px+1, py+1) — the
        // same tile real statics use for `max_draw_z`'s roof-flood check.
        let mut world = anima_core::World::new();
        world.items.insert(
            1,
            synth_item(
                1,
                999,
                (px + 1) as u16,
                (py + 1) as u16,
                (pz + 15) as i8,
                true,
            ),
        );
        let multis = Multis::from_components(std::collections::HashMap::from([(
            999,
            vec![MultiComponent {
                graphic: 0x0586,
                dx: 0,
                dy: 0,
                dz: 0,
                visible: true,
                is_origin: true,
            }],
        )]));

        let culled = max_draw_z(&world, &mut map, Some(&multis), px, py, pz);
        assert!(
            culled < 127,
            "the multi's roof component must cull max_draw_z, got {culled}"
        );
    }

    /// FIX 6 (pure given real tiledata/animdata, no `Session` needed): an
    /// animated multi component (mill wheel, pennant) must get the SAME
    /// frame-sequence treatment [`anim_suffix`] gives a real static — not
    /// freeze on frame 0. Uses a real animated graphic (0x03AE, 3 frames,
    /// probed directly off animdata.mul).
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn anim_suffix_emits_frame_sequence_for_a_real_animated_graphic() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let map = MapData::open(&dir).expect("open map data");
        let animdata = AnimData::open(&dir).expect("open animdata");
        assert!(
            map.item_is_animated(0x03AE),
            "0x03AE should be a real animated graphic"
        );

        let suffix = anim_suffix(&map, Some(&animdata), 0x03AE);
        assert!(suffix.contains("\"a\":[942,943,944]"), "suffix={suffix}");
        assert!(suffix.contains("\"ai\":"), "suffix={suffix}");

        // A non-animated graphic (an ordinary wall) emits nothing.
        assert_eq!(anim_suffix(&map, Some(&animdata), 0x0001), "");
        // No animdata table at all → nothing, even for an animated graphic.
        assert_eq!(anim_suffix(&map, None, 0x03AE), "");
    }

    /// Real bug regression (see `blocking_item_at`'s doc for the full live
    /// diagnosis): a house foundation's stairs put a body on a **bridge**
    /// (`0x0751`, height 5, z 0 → half-height stand z=2), and the tile one
    /// step further carries the foundation's own **impassable** riser
    /// (`0x0063`, z 0..5) topped by the empty plot's own **surface**
    /// (`0x31F4`, z 7) — a +5 climb exactly like a bridge is for.
    /// `calculate_new_z` resolves this correctly to z=7 (verified below), but
    /// checked against a body still down at the pre-step z=2 — exactly what
    /// `tile_walkable` always checks against — the SAME riser genuinely
    /// overlaps that lower body (`item_blocks`'s span test), which is why
    /// `tile_walkable` denies the climb outright (also verified below): the
    /// two calculators don't just differ in their surface scoring, the
    /// blocker check disagrees too depending on WHICH Z it's asked about.
    /// `build_scene`'s fix is to trust `calculate_new_z`'s landing Z and ask
    /// `blocking_item_at` about THAT Z, not the pre-step one. Reuses the
    /// known-clear open-deep-water box around (1459,1767) (see
    /// `fix1_smallboat_...`'s doc) so every candidate below is provably the
    /// synthetic multi, not real terrain/statics.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn foundation_stairs_bridge_climb_walkable_via_chain_and_resolved_z_blocker_check() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");

        // Confirm the live-diagnosed tiledata facts this fix leans on.
        assert_eq!(
            map.item_flags(0x0751) & (FLAG_SURFACE | FLAG_BRIDGE),
            FLAG_SURFACE | FLAG_BRIDGE,
            "0x0751 must be a surface+bridge"
        );
        assert_eq!(map.item_height(0x0751), 5);
        assert_eq!(
            map.item_flags(0x0063) & (FLAG_IMPASSABLE | FLAG_SURFACE),
            FLAG_IMPASSABLE,
            "0x0063 must be impassable and NOT itself a surface"
        );
        assert_eq!(map.item_height(0x0063), 5);
        assert_ne!(map.item_flags(0x31F4) & FLAG_SURFACE, 0, "0x31F4 must be a surface");

        let (bridge_x, bridge_y) = (1459i64, 1766i64);
        let (plot_x, plot_y) = (bridge_x, bridge_y - 1);
        for (tx, ty) in [(bridge_x, bridge_y), (plot_x, plot_y)] {
            assert!(
                map.land(tx as u32, ty as u32).impassable(),
                "({tx},{ty}) should be deep water"
            );
            assert!(
                map.statics(tx as u32, ty as u32).is_empty(),
                "({tx},{ty}) should have no real statics"
            );
        }

        let mut world = anima_core::World::new();
        world.items.insert(
            1,
            synth_item(1, 0, bridge_x as u16, bridge_y as u16, 0, true), // multi id 0, origin = the bridge tile
        );
        let multis = Multis::from_components(std::collections::HashMap::from([(
            0u32,
            vec![
                MultiComponent {
                    graphic: 0x0751,
                    dx: 0,
                    dy: 0,
                    dz: 0,
                    visible: true,
                    is_origin: true,
                },
                MultiComponent {
                    graphic: 0x0063,
                    dx: 0,
                    dy: -1,
                    dz: 0,
                    visible: true,
                    is_origin: false,
                },
                MultiComponent {
                    graphic: 0x31F4,
                    dx: 0,
                    dy: -1,
                    dz: 7,
                    visible: true,
                    is_origin: false,
                },
            ],
        )]));

        // `sz_chain`'s authoritative half: stepping north from the bridge
        // (stand z=2) resolves the climb to z=7.
        let landing_z =
            calculate_new_z(&world, &mut map, Some(&multis), plot_x, plot_y, 2, 0 /* north */)
                .expect("the bridge-widened climb must resolve a landing Z");
        assert_eq!(landing_z, 7);

        // `blocking_item_at`'s other half, checked against the body's ACTUAL
        // landing height: the riser does NOT block.
        let dyn_items = dynamic_statics_at(&world, &map, plot_x, plot_y);
        assert!(
            blocking_item_at(
                &world,
                &mut map,
                Some(&multis),
                plot_x,
                plot_y,
                landing_z,
                &dyn_items,
                false
            )
            .is_none(),
            "the riser must not block a body actually standing at the resolved landing Z"
        );

        // Combined: this is EXACTLY `build_scene`'s new per-tile `w` derivation.
        let walk = calculate_new_z(&world, &mut map, Some(&multis), plot_x, plot_y, 2, 0).is_some()
            && blocking_item_at(
                &world,
                &mut map,
                Some(&multis),
                plot_x,
                plot_y,
                landing_z,
                &dyn_items,
                false,
            )
            .is_none();
        assert!(walk, "the foundation's stairs must be walkable onto the plot");

        // The SAME riser, checked against the OLD pre-step Z (2) instead,
        // DOES block — proving why `build_scene` must pass the RESOLVED z,
        // not `pz`, into `blocking_item_at`.
        assert!(
            blocking_item_at(&world, &mut map, Some(&multis), plot_x, plot_y, 2, &dyn_items, false)
                .is_some(),
            "checked against the pre-step Z the riser genuinely overlaps the body"
        );

        // And the OLD/unchanged `tile_walkable` (which always checks against
        // the pre-step Z, matching `step_ok`) still disagrees with the chain
        // answer — the exact split `build_scene`'s per-tile `w` now resolves
        // in favor of the chain, inside `CHAIN_RADIUS`.
        assert!(
            !tile_walkable(&world, &mut map, Some(&multis), plot_x, plot_y, 2),
            "tile_walkable's own (unchanged, pre-step-Z) semantics must still deny this climb"
        );
    }

    /// Companion to the above: with NO surface at all over the impassable
    /// riser (the plot's own floor removed), the climb must genuinely stay
    /// unwalkable — `calculate_new_z` must return `None` (nothing to land
    /// on) — proving the fix isn't "the riser never blocks", it depends on
    /// there being a real destination surface to land on.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn foundation_riser_with_no_surface_above_is_genuinely_unwalkable() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");

        let (bridge_x, bridge_y) = (1459i64, 1766i64);
        let (plot_x, plot_y) = (bridge_x, bridge_y - 1);
        for (tx, ty) in [(bridge_x, bridge_y), (plot_x, plot_y)] {
            assert!(
                map.land(tx as u32, ty as u32).impassable(),
                "({tx},{ty}) should be deep water"
            );
            assert!(
                map.statics(tx as u32, ty as u32).is_empty(),
                "({tx},{ty}) should have no real statics"
            );
        }

        let mut world = anima_core::World::new();
        world.items.insert(
            1,
            synth_item(1, 0, bridge_x as u16, bridge_y as u16, 0, true),
        );
        let multis = Multis::from_components(std::collections::HashMap::from([(
            0u32,
            vec![
                MultiComponent {
                    graphic: 0x0751,
                    dx: 0,
                    dy: 0,
                    dz: 0,
                    visible: true,
                    is_origin: true,
                },
                MultiComponent {
                    graphic: 0x0063,
                    dx: 0,
                    dy: -1,
                    dz: 0,
                    visible: true,
                    is_origin: false,
                },
                // no floor component this time — nothing to stand on.
            ],
        )]));

        assert_eq!(
            calculate_new_z(&world, &mut map, Some(&multis), plot_x, plot_y, 2, 0),
            None,
            "an impassable riser with nothing to stand on above it must not resolve a landing Z"
        );
        assert!(
            !tile_walkable(&world, &mut map, Some(&multis), plot_x, plot_y, 2),
            "and must still be unwalkable through the ordinary (unchanged) check too"
        );
    }

    /// Root-cause regression for the SERVER'S OWN movement gate (`can_walk`/
    /// `step_ok`, now [`can_step_to`]): before this fix, a real committed step
    /// — the browser's keyboard walk, resolved server-side to decide which
    /// direction to actually send — used the same pre-step-Z blocker bug
    /// `blocking_item_at` was split out to fix for `build_scene`'s `w`, just
    /// never applied here. Live-reproduced: a character climbed the ground →
    /// stairs (landing on the bridge at z=2) and then could not step onto the
    /// foundation, even though `build_scene` already reported that tile
    /// `w=1`. Exercises the exact same bridge/riser/plot geometry as
    /// `foundation_stairs_bridge_climb_walkable_via_chain_and_resolved_z_blocker_check`
    /// above, but through `can_step_to` itself (the real gate), and proves
    /// the fix didn't weaken anything a real step must still refuse: a
    /// closed door and a genuinely unclimbable wall — both still deny (as
    /// `StepDeny::NoLanding`; see the door assertion's own doc for the
    /// honest reason they share that variant).
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn can_step_to_allows_the_stairs_climb_and_still_blocks_a_door_and_a_wall() {
        let dir_path = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir_path).expect("open map data");

        // --- 1. the foundation's stairs must now be ALLOWED, landing at z=7,
        // through the REAL gate — not just `calculate_new_z` + `blocking_item_at`
        // called by hand (already proven above).
        let (bridge_x, bridge_y) = (1459i64, 1766i64);
        let (plot_x, plot_y) = (bridge_x, bridge_y - 1);
        let mut climb_world = anima_core::World::new();
        climb_world.items.insert(
            1,
            synth_item(1, 0, bridge_x as u16, bridge_y as u16, 0, true),
        );
        let climb_multis = Multis::from_components(std::collections::HashMap::from([(
            0u32,
            vec![
                MultiComponent {
                    graphic: 0x0751,
                    dx: 0,
                    dy: 0,
                    dz: 0,
                    visible: true,
                    is_origin: true,
                },
                MultiComponent {
                    graphic: 0x0063,
                    dx: 0,
                    dy: -1,
                    dz: 0,
                    visible: true,
                    is_origin: false,
                },
                MultiComponent {
                    graphic: 0x31F4,
                    dx: 0,
                    dy: -1,
                    dz: 7,
                    visible: true,
                    is_origin: false,
                },
            ],
        )]));
        match can_step_to(
            &climb_world,
            &mut map,
            Some(&climb_multis),
            bridge_x,
            bridge_y,
            2,
            0, /* north: bridge -> plot */
        ) {
            Ok(z) => assert_eq!(z, 7, "must resolve the same landing Z calculate_new_z does"),
            other => panic!("expected the foundation stairs climb to be allowed, got {other:?}"),
        }
        // Sanity: (plot_x, plot_y) really is the tile the step above landed on.
        assert_eq!((plot_x, plot_y), (bridge_x, bridge_y - 1));

        // --- 2. a closed door must STILL refuse — same door/geometry as
        // `closed_door_blocks_strictly_but_not_for_planning`, walked through
        // `can_step_to` instead of `explain_tile_walkable` directly.
        //
        // HONEST finding (same spirit as `fix1_smallboat_..._using_real_boat_data`'s
        // hull-tile note above): the deny comes back as `NoLanding`, not
        // `DynamicItem`. ClassicUO's `CreateItemList` (`Pathfinder.cs`) only
        // drops a door's own Impassable flag from Z-scoring when the
        // `SmoothDoors` profile option is on — off by default (a plain `bool`
        // field, so `false` unless a player opts in) — so by default a closed
        // door's tiledata (Impassable, height ~20) poisons `calculate_new_z`'s
        // scoring the same way an actual wall would, and it returns `None`
        // before `blocking_item_at` is ever consulted. This is faithful, not a
        // regression: the OLD (pre-fix) `step_ok` ALSO called
        // `calculate_new_z(..).is_none()` as its very first check, before its
        // own inline blocker logic — so a closed door already denied a real
        // step this exact way before this fix, and still does; this fix only
        // changed how the tile SURFACE is scored and at which Z the blocker
        // rules apply, not this early exit.
        assert!(
            map.item_is_door(0x06A5),
            "0x06A5 should be a real door graphic"
        );
        let mut door_world = anima_core::World::new();
        let door_serial = 1_073_751_127;
        door_world.items.insert(
            door_serial,
            anima_core::world::Item {
                serial: door_serial,
                graphic: 0x06A5,
                amount: 1,
                pos: anima_core::types::Position {
                    x: 1611,
                    y: 1591,
                    z: 0,
                },
                container: None,
                layer: 0,
                hue: 0,
                name: String::new(),
                direction: 0,
                is_multi: false,
            },
        );
        match can_step_to(&door_world, &mut map, None, 1611, 1592, 5, 0 /* north, into the door */) {
            Err(StepDeny::NoLanding) => {}
            other => panic!("expected the closed door to still deny the step, got {other:?}"),
        }

        // --- 3. a plain (unclimbable) wall must still refuse: the SAME
        // impassable riser as the stairs climb above, but with no surface
        // above it at all this time — nothing for `calculate_new_z` to land
        // on, matching `foundation_riser_with_no_surface_above_is_genuinely_unwalkable`.
        let mut wall_world = anima_core::World::new();
        wall_world.items.insert(
            1,
            synth_item(1, 0, bridge_x as u16, bridge_y as u16, 0, true),
        );
        let wall_multis = Multis::from_components(std::collections::HashMap::from([(
            0u32,
            vec![
                MultiComponent {
                    graphic: 0x0751,
                    dx: 0,
                    dy: 0,
                    dz: 0,
                    visible: true,
                    is_origin: true,
                },
                MultiComponent {
                    graphic: 0x0063,
                    dx: 0,
                    dy: -1,
                    dz: 0,
                    visible: true,
                    is_origin: false,
                },
                // no floor component this time — nothing to stand on.
            ],
        )]));
        match can_step_to(&wall_world, &mut map, Some(&wall_multis), bridge_x, bridge_y, 2, 0) {
            Err(StepDeny::NoLanding) => {}
            other => panic!("expected the unclimbable riser to refuse with NoLanding, got {other:?}"),
        }
    }
}

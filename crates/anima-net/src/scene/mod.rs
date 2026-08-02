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

// What this module is assembled from. `mod.rs` keeps `build_scene` itself —
// the one function that holds every borrow (session, map, art, anim) at once
// and walks the visible window; the pieces it calls live next door. Each
// submodule opens with `use super::*`, reaching this module's imports and its
// siblings alike.
//
// `pub use` for the four that carry the crate-facing API (`lib.rs` and
// `play_server` path into them); plain `use` for the three that are purely
// internal to the scene build.
mod height;
mod tiles;
mod walk;
mod worldmap;
pub use height::*;
pub use tiles::*;
pub use walk::*;
pub use worldmap::*;

mod dialogs;
mod feeds;
mod multis;
use dialogs::*;
use feeds::*;
use multis::*;

// ----------------------------------------------------------------------------
// Step-Z resolution — a faithful port of ClassicUO `Pathfinder.CalculateNewZ`
// (+ `CalculateMinMaxZ`, `CreateItemList`). The server's ConfirmWalk carries no
// Z, so when the player steps onto a tile we resolve the standing Z exactly as
// the client does: build the tile's object list, bound the step by the tile we
// came from, and pick the surface/bridge closest to our current Z with headroom.
// This is what makes stairs (bridge tiles, avg Z = z + height/2) climb correctly.
// ----------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Replay feeds.
//
// Each of these is a *seq-stamped event log*, not state: the renderer keeps the
// highest `seq` it has acted on and plays only what is newer, so the same
// entries reappear across polls until they age out of `World`'s capped buffer.
// That is why they are `World`-only (no map/art/session), and why dropping one
// frame is harmless.
// ---------------------------------------------------------------------------

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
    // A dyed item's art hue, PartialHue-aware (see [`item_art_hue`]). Every
    // surface that draws item art — ground sprite, container/trade grid, vendor
    // row, drag ghost — asks the art endpoint for this exact value.
    let item_hue =
        |g: u16, hue: u16| item_art_hue(hue, map.as_deref().map_or(0, |m| m.item_flags(g)));
    // Animated dynamic item (campfire, spell field, brazier): the same animdata
    // frame sequence a static with that graphic gets, resolved through the shared
    // borrow before `map` is consumed by the tile loop below.
    let item_frames = |g: u16| map.as_deref().and_then(|m| anim_frames(m, animdata, g));
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
        map.as_deref().map_or((None, None), |m| {
            path_bits(in_radius, m.item_height(g), m.item_flags(g))
        })
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
                        // PartialHue-encoded like every other item-art surface: the
                        // /anim, /gump and /art endpoints all recolor through the
                        // same `apply_hue`, and ClassicUO honors PartialHue on worn
                        // sprites too (`Game/GameObjects/Views/ItemView.cs:278`).
                        // Without this a dyed partial-hue weapon was fully recolored
                        // on the character while correct in the backpack.
                        let hue = item_hue(it.graphic, hue);
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
                // Dye hue (omitted when the item is undyed). A corpse is excluded
                // because its `hue` below is the dead creature's Corpse.def-remapped
                // BODY hue, applied to an anim frame — not this item's art dye.
                let hue = item_hue(it.graphic, it.hue);
                if hue != 0 {
                    v["hue"] = json!(hue);
                }
                // Animated (`TileFlag.Animation`): same baked frame list the statics
                // loop emits, so a server-spawned campfire/spell field cycles instead
                // of freezing on frame 0.
                if let Some((seq, ai)) = item_frames(it.graphic) {
                    v["a"] = json!(seq);
                    v["ai"] = json!(ai);
                }
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
            // Same PartialHue encoding as the mobile-equip arm above; this one also
            // feeds the paperdoll's icon list and doll gump layers, which ClassicUO
            // hues partially as well (`PaperDollInteractable.cs:269`).
            let hue = item_hue(it.graphic, hue);
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
                "x": it.pos.x, "y": it.pos.y,
                // Is this nested item itself a container? Only then should a
                // double-click open a container window (bandages/potions/etc. must not).
                "c": item_is_cont(it.graphic) as u8
            });
            // Dye hue for the grid icon (and, via the vendor buy list's pairing to
            // these entries, its shop rows). Omitted when undyed — this array is
            // rebuilt every poll for up to 400 items.
            let hue = item_hue(it.graphic, it.hue);
            if hue != 0 {
                v["hue"] = json!(hue);
            }
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
                let mut v = json!({
                    "serial": it.serial, "g": it.graphic, "amount": it.amount,
                    "price": it.price, "name": resolve_shop_name(&it.name, cliloc)
                });
                // The sell list (0x9E) carries its own hue, so the row icon shows the
                // dyed item we're actually offering. (The buy side needs nothing here:
                // its rows pair to the vendor container's `contItems`, which are hued.)
                let hue = item_hue(it.graphic, it.hue);
                if hue != 0 {
                    v["hue"] = json!(hue);
                }
                v
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
                let land_impassable =
                    if dx.abs() <= PATH_RADIUS && dy.abs() <= PATH_RADIUS && land.impassable() {
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
    let sounds = json_array(&sounds_json(&s.world));
    let anims = json_array(&anims_json(&s.world));
    let tanims = json_array(&typed_anims_json(&s.world));
    let damage = json_array(&damage_json(&s.world));
    let effects = json_array(&effects_json(&s.world, animdata));
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
    let buffs = json_array(&buffs_json(&s.world));
    let skills = json_array(&skills_json(&s.world));
    let lights = json_array(&lights);
    let mobiles = json_array(&mobiles);
    let items = json_array(&items);
    let cont_items = json_array(&cont_items);
    let target = serde_json::to_string(&target).unwrap_or_else(|_| "{}".into());
    let dbg = json_array(&dbg);
    let journal = json_array(journal);
    // Open server gumps/dialogs (0xB0/0xDD), each parsed into positioned elements.
    let gumps = gumps_json(&s.world, cliloc);
    // The open right-click context menu (0xBF/0x14), with cliloc labels resolved.
    let popup =
        serde_json::to_string(&popup_json(&s.world, cliloc)).unwrap_or_else(|_| "null".into());
    // Legacy item/question menus (0x7C), potentially several at once.
    let legacy_menus = json_array_value(&legacy_menus_json(&s.world));
    // Server dye hue pickers (0x95), potentially several callback serials.
    let hue_pickers = json_array_value(&hue_pickers_json(&s.world));
    // Concurrent 0xA6 Tip/Notice windows. Only kind "tip" has prev/next.
    let tips = json_array_value(&tips_json(&s.world));
    // Concurrent modal 0xAB text-entry dialogs, keyed in the browser by seq.
    let text_entry_dialogs = json_array_value(&text_entry_dialogs_json(&s.world));
    // Persistent 0xB8 character profile windows, keyed by exact response seq.
    let profiles = json_array_value(&character_profiles_json(&s.world));
    let logout_ack =
        serde_json::to_string(&logout_ack_json(&s.world)).unwrap_or_else(|_| "null".into());
    let boat_moves = json_array_value(&boat_movements_json(&s.world));
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
    let spellbooks = json_array(&spellbooks);
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
    let lift_rejects = json_array(&lift_rejects);
    // Item-drag completion acknowledgements (0x28 EndDraggingItem / 0x29
    // DropItemAccepted). The browser correlates these with pending placements
    // before clearing its cursor, protecting a newer lift from a delayed ack.
    let drag_completions = json_array_value(&drag_completions_json(&s.world));
    let death_screen =
        serde_json::to_string(&death_screen_json(&s.world)).unwrap_or_else(|_| "null".into());
    // Recent server-initiated container opens (0x24 DrawContainer): a window we
    // did NOT ourselves double-click for (banker "bank" speech, GM `[bank`, a
    // snoop, …). The client opens a window for each `seq` newer than the last it
    // handled (reusing the same `openContainer` it uses for its own double-clicks).
    // Filtered by `container_opens_json` to real container gumpIds — see its doc.
    let container_opens = json_array_value(&container_opens_json(&s.world));
    // Recent Swing events (0x2F): `attacker` just swung at `defender`. Purely
    // cosmetic — the client briefly faces the attacker toward the defender.
    let swings: Vec<Value> = s
        .world
        .recent_swings
        .iter()
        .map(|&(seq, attacker, defender)| json!({ "seq": seq, "attacker": attacker, "defender": defender }))
        .collect();
    let swings = json_array(&swings);
    // The latest server-initiated paperdoll open/refresh (0x88), or null. See
    // `paperdoll_json`'s doc for the `seq` "fresh request" semantics.
    let paperdoll =
        serde_json::to_string(&paperdoll_json(&s.world)).unwrap_or_else(|_| "null".into());
    // Validated 0xA5 external-page requests. The browser seq-gates these and
    // requires an explicit click before opening a new tab.
    let open_urls = json_array_value(&open_urls_json(&s.world));
    // Current facet/map index (0xBF/0x08 MapChange); see `World::map_index`'s doc
    // for what a real per-facet `MapData` reload would additionally require.
    let facet = s.world.map_index;
    // Every open secure-trade session (0x6F), or []. See `trades_json`'s doc.
    let trades = json_array_value(&trades_json(&s.world));
    // Open treasure/decoration map windows (0x90/0xF5 + 0x56), or []. See `maps_json`'s doc.
    let maps = json_array_value(&maps_json(&s.world));
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
mod tests;

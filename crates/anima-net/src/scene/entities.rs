//! The entities in a scene: mobiles, ground items, worn equipment, and the
//! contents of every open container.
//!
//! Each of these was an inline `let x: Vec<Value> = …` chain inside
//! `build_scene`, several dozen lines long, reading the same handful of outer
//! bindings. They are all pure projections of [`World`] through a [`Look`],
//! with no borrow of the mutable map — which is why they can be plain
//! functions, and why `build_scene` calls every one of them before it walks
//! the tiles.

use anima_core::world::Mobile;

use super::look::Look;
use super::*;

/// Every mobile except the player: resolved animation body, worn equipment,
/// and health/notoriety state.
pub(super) fn mobiles_json(world: &World, look: &Look, player: &Mobile) -> Vec<Value> {
    world
        .mobiles
        .values()
        .filter(|m| m.serial != player.serial)
        .map(|m| {
            let (body, hue) = look.remap(m.body, m.hue);
            // Only "people" bodies (>= 400) wear clothes/hair/beard; animals and
            // monsters carry nothing, so skip the per-item work for them.
            let equip: Vec<Value> = if body >= 400 {
                world
                    .items
                    .values()
                    .filter(|it| it.container == Some(m.serial) && it.layer != 0)
                    .map(|it| {
                        let (a, gump, hue) =
                            look.equip_conv(body, look.item_anim(it.graphic), it.hue);
                        // PartialHue-encoded like every other item-art surface: the
                        // /anim, /gump and /art endpoints all recolor through the
                        // same `apply_hue`, and ClassicUO honors PartialHue on worn
                        // sprites too (`Game/GameObjects/Views/ItemView.cs:278`).
                        // Without this a dyed partial-hue weapon was fully recolored
                        // on the character while correct in the backpack.
                        let hue = look.item_hue(it.graphic, hue);
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
            let mount = world
                .items
                .values()
                .find(|it| it.container == Some(m.serial) && it.layer == 25);
            let (mount_anim, mount_off) = mount.map_or((0u16, 0i8), |it| {
                mount_info_for(it.graphic, &|g| look.item_anim(g))
            });
            let mut v = json!({
                "serial": m.serial,
                "x": m.pos.x, "y": m.pos.y, "z": m.pos.z, "dir": m.direction,
                "body": body, "at": look.atype(body), "noto": m.notoriety, "name": m.name,
                "hits": m.hits, "hitsMax": m.hits_max,
                "hue": hue, "equip": equip,
                "mounted": mount.is_some() as u8, "mountAnim": mount_anim,
                "mountOff": mount_off
            });
            merge_obj(&mut v, hidden_field(m.hidden));
            merge_obj(&mut v, running_field(m.running));
            merge_obj(&mut v, flying_field(m.flying));
            merge_obj(&mut v, poisoned_field(m.poisoned));
            merge_obj(&mut v, paralyzed_field(m.paralyzed));
            merge_obj(&mut v, yellow_field(m.yellow_health));
            v
        })
        .collect()
}

/// Flag mobiles ClassicUO would not draw (`AllowedToDraw = !HasSurfaceOverhead`).
/// `"so":1` is omitted when false, like [`hidden_field`]. The player is not in
/// this list. Needs `&mut MapData` for the statics cache, so it runs after
/// `Look` is dropped.
pub(super) fn apply_surface_overhead(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    max_z: i32,
    mobiles: &mut [Value],
) {
    let scan = TileScan::build(world, map);
    let mut cache = std::collections::HashMap::new();
    for v in mobiles {
        let (Some(x), Some(y), Some(z)) = (v.get("x"), v.get("y"), v.get("z")) else {
            continue;
        };
        let x = x.as_i64().unwrap_or(0);
        let y = y.as_i64().unwrap_or(0);
        let z = z.as_i64().unwrap_or(0) as i32;
        if has_surface_overhead(&scan, map, multis, x, y, z, max_z, &mut cache) {
            v["so"] = json!(1);
        }
    }
}

/// Ground items in view: the sprite, its draw-sort priority, and the
/// pathfinding bits a nearby one contributes to the browser's own Z resolution.
pub(super) fn items_json(
    world: &World,
    look: &Look,
    px: i64,
    py: i64,
    max_z: i32,
    no_draw_roofs: bool,
) -> Vec<Value> {
    world
        .items
        .values()
        .filter(|it| {
            // No z-ceiling filter here any more: an item hidden by the ceiling is
            // EMITTED with `hz` and eased to alpha 0 by the renderer, like every
            // other ceiling-hidden object (see terrain.rs's split-predicate note).
            // A multi (`is_multi`) isn't a drawable item at all — its `graphic` is
            // a multi id, not an ART graphic; it's expanded into the statics
            // stream (see the tile loop below) instead of drawn directly here.
            // `item_nodraw` is NOT in this filter any more: a nodraw item is
            // emitted with `nd` and simply never drawn, because the authoritative
            // walk path buckets EVERY uncontained non-multi item with no nodraw
            // test at all (`walk.rs`). ServUO really does ship path-bearing ones
            // as ordinary items — `HouseLadder` places graphic 0x3F28
            // (SURFACE|BRIDGE, height 3: the climbable rung), and `InvisibleTile`
            // /`ShipCannon` place 0x2198 (SURFACE) — so filtering them here made
            // the browser blind to a ladder it is standing on.
            !it.is_multi && it.container.is_none()
        })
        .map(|it| {
            let mut v = json!({
                "x": it.pos.x, "y": it.pos.y, "z": it.pos.z,
                "g": if it.graphic == 0x2006 { it.graphic } else { look.stack_graphic(it.graphic, it.amount) },
                "serial": it.serial, "pz": look.item_pz(it.graphic, it.pos.z as i32)
            });
            // Mark foliage so the renderer can fade it (only when true, small payload).
            if look.item_foliage(it.graphic) {
                v["f"] = json!(1);
            }
            // ClassicUO `Item.IsLocked`: `(Flags & Movable) == 0 && Weight > 90`.
            // Both halves are needed and they live in different places — the flag
            // is on the wire, the weight is in tiledata, which the core cannot
            // see — so the judgement is made here and shipped as one bit. The
            // renderer refuses to drag these, as ClassicUO does
            // (`GameActions.cs:457`), instead of sending a 0x07 that can only
            // come back as a 0x27 reject.
            if !it.flag_movable() && look.item_weight(it.graphic) > 90 {
                v["lk"] = json!(1);
            }
            // ServUO sends invisible items only to staff; ClassicUO draws them
            // translucent rather than dropping them, so a GM can see what they
            // are standing on.
            if it.flag_hidden() {
                v["hid"] = json!(1);
            }
            // Ceiling hiding — purely a DRAW decision, as in terrain.rs. The roof
            // clause matches ClassicUO, which runs items through the same
            // `_noDrawRoofs && IsRoof` branch as statics, so an item lying on a roof
            // fades with it. Pathing fields are unaffected: an invisible item still
            // blocks and still feeds the browser's `calculate_new_z`.
            let nd_item = look.item_nodraw(it.graphic);
            if nd_item {
                v["nd"] = json!(1);
            }
            let hz_item = ceil_hz(
                it.pos.z as i32,
                max_z,
                no_draw_roofs,
                look.item_roof(it.graphic),
            );
            if hz_item {
                v["hz"] = json!(1);
            }
            if look.item_roof(it.graphic) {
                v["rf"] = json!(1);
            }
            // Translucent (blood, spiderweb, curtain): ClassicUO runs dynamic
            // items through the same alpha chain as statics (`case Item item:`
            // → ProcessAlpha, GameSceneDrawingSorting.cs:901). ServUO's `Blood`
            // (Scripts/Items/Decorative/Blood.cs) is an Item, so skipping this
            // site would draw the most common translucent object on a live
            // shard opaque while identical map statics faded beside it.
            if look.item_translucent(it.graphic) {
                v["tr"] = json!(1);
            }
            // …and in winter/desolation, hide it outright — but not for a multi
            // (ClassicUO's `!item.IsMulti` guard); a placed house keeps its shrubs.
            if !it.is_multi
                && look.item_foliage_hidden(season::scene_season(world.season), it.graphic)
            {
                v["fh"] = json!(1);
            }
            // Mark containers so double-click opens a loot window (doors don't).
            if look.item_is_cont(it.graphic) {
                v["c"] = json!(1);
            }
            // `h`/`pf` (PATH_RADIUS-gated, see `item_path_bits`'s doc): omitted
            // whenever out of radius or zero, so this is purely additive —
            // an item outside PATH_RADIUS serializes exactly as before.
            let in_radius = (it.pos.x as i64 - px).abs() <= PATH_RADIUS
                && (it.pos.y as i64 - py).abs() <= PATH_RADIUS;
            let (h, pf) = look.item_path_bits(it.graphic, in_radius);
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
                merge_obj(
                    &mut v,
                    stack_fields(it.amount, look.item_stackable(it.graphic)),
                );
                // Dye hue (omitted when the item is undyed). A corpse is excluded
                // because its `hue` below is the dead creature's Corpse.def-remapped
                // BODY hue, applied to an anim frame — not this item's art dye.
                let hue = look.item_hue(it.graphic, it.hue);
                if hue != 0 {
                    v["hue"] = json!(hue);
                }
                // Animated (`TileFlag.Animation`): same baked frame list the statics
                // loop emits, so a server-spawned campfire/spell field cycles instead
                // of freezing on frame 0.
                if let Some((seq, ai)) = look.item_frames(it.graphic) {
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
                let (body, hue) = look.remap_corpse(it.amount, it.hue);
                let dg = look.anim.map_or(0, |a| a.death_group(body));
                merge_obj(&mut v, corpse_fields(body, hue, it.direction, dg));
                // What it died wearing. ClassicUO draws these over the death
                // pose exactly as it draws a living mobile's worn layers
                // (`ItemView.DrawCorpse` walks `LayerOrder.UsedLayers`), and
                // the data has been sitting in `World::corpse_equip` — parsed
                // from 0x89 — with nowhere to go.
                let equip = corpse_equip_json(world, look, it.serial, body);
                if !equip.is_empty() {
                    v["equip"] = json!(equip);
                }
            }
            v
        })
        .collect()
}

/// Cap on the glow pass — shared with the tile loop, which appends static
/// light sources (wall torches, lamps) to the same list.
pub(super) const LIGHT_CAP: usize = 64;

/// Per-object light sources for the renderer's night glow. Returns a list the
/// caller keeps appending to, not a finished one.
pub(super) fn lights_json(world: &World, look: &Look, px: i64, py: i64, pz: i32) -> Vec<Value> {
    let mut lights: Vec<Value> = Vec::new();
    // The player's own glow. ClassicUO gives a mobile light shape 1
    // (`AddLight`'s `obj is Mobile _` arm), which is its large soft one.
    lights.push(json!({ "x": px, "y": py, "z": pz, "r": 5, "id": 1 }));
    for it in world.items.values() {
        if lights.len() >= LIGHT_CAP {
            break;
        }
        // A multi's own entry carries a multi id in `graphic`, not an ART
        // graphic — skip it here (any light-emitting components are handled
        // per-component in the tile loop below, alongside static lights).
        if !it.is_multi && it.container.is_none() && look.item_is_light(it.graphic) {
            // `id` is the light.mul shape (tiledata Quality/layer, ClassicUO
            // `AddLight`); `r` stays as the fallback radius for a renderer that
            // has no shape for it.
            let (lid, lcolor) = look.item_light(it.graphic);
            lights.push(json!({ "x": it.pos.x, "y": it.pos.y, "z": it.pos.z, "r": 3,
                                "id": lid, "c": lcolor }));
        }
    }
    lights
}

/// The player's worn items (container == us, on a real layer), which drive the
/// paperdoll. Layer 0 = not equipped; the backpack itself is layer 0x15.
/// `equip_body` is the wearer's REMAPPED body, which is what `Equipconv.def`
/// is keyed by.
pub(super) fn equip_json(
    world: &World,
    look: &Look,
    player: &Mobile,
    equip_body: u16,
) -> Vec<Value> {
    world
        .items
        .values()
        .filter(|it| it.container == Some(player.serial) && it.layer != 0)
        .map(|it| {
            let (a, gump, hue) = look.paperdoll_gump(equip_body, it.graphic, it.hue);
            // Same PartialHue encoding as the mobile-equip arm above; this one also
            // feeds the paperdoll's icon list and doll gump layers, which ClassicUO
            // hues partially as well (`PaperDollInteractable.cs:269`).
            let hue = look.item_hue(it.graphic, hue);
            let mut v = json!({
                "serial": it.serial, "g": it.graphic, "layer": it.layer,
                "anim": a, "hue": hue
            });
            if let Some(g) = gump {
                v["gump"] = json!(g);
            }
            v
        })
        .collect()
}

/// A corpse's worn layers, resolved for drawing: 0x89 CorpseEquip gives
/// `(layer, item serial)` pairs and the items themselves arrive with it —
/// ServUO's `Corpse.SendInfoTo` sends `CorpseContent` and `CorpseEquip`
/// together, unprompted, for any human corpse — so everything needed is already
/// in the world.
///
/// Same resolution a living mobile's equipment gets: tiledata AnimID, then the
/// `Equipconv.def` override keyed by the wearer's body, then the PartialHue
/// encoding. `body` is the Corpse.def-remapped body the death frame is drawn
/// with, so a conversion keyed to it lines up with the frame underneath.
///
/// Layers with no AnimID (a backpack, a container of loot) are dropped: they
/// have no world sprite, which is also why ClassicUO's per-direction layer
/// order has no slot for them.
pub(super) fn corpse_equip_json(world: &World, look: &Look, corpse: u32, body: u16) -> Vec<Value> {
    let Some(entries) = world.corpse_equip.get(&corpse) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|&(layer, serial)| {
            let it = world.items.get(&serial)?;
            // Still *on* the corpse. 0x89 is a one-shot snapshot of what it died
            // wearing and nothing on the wire retracts it, so looting the sword
            // would otherwise leave the corpse holding it forever. ClassicUO
            // never has to think about this because it stores the layer on the
            // item and asks `FindItemByLayer` — a search of the corpse's *live*
            // contents — which is the same question this line asks.
            if it.container != Some(corpse) {
                return None;
            }
            let (anim, _, hue) = look.equip_conv(body, look.item_anim(it.graphic), it.hue);
            if anim == 0 {
                return None;
            }
            Some(json!({
                "layer": layer, "anim": anim,
                "hue": look.item_hue(it.graphic, hue),
                "serial": serial, "g": it.graphic,
            }))
        })
        .collect()
}

/// The contents of every open container/corpse/trade window, flattened — the
/// browser keys each entry by its own `cont` serial.
pub(super) fn container_items_json(world: &World, look: &Look) -> Vec<Value> {
    world
        .items
        .values()
        .filter(|it| it.container.is_some())
        .take(400)
        .map(|it| {
            let mut v = json!({
                "serial": it.serial, "cont": it.container,
                "g": if it.graphic == 0x2006 { it.graphic } else { look.stack_graphic(it.graphic, it.amount) },
                "amount": it.amount,
                "x": it.pos.x, "y": it.pos.y,
                // Is this nested item itself a container? Only then should a
                // double-click open a container window (bandages/potions/etc. must not).
                "c": look.item_is_cont(it.graphic) as u8
            });
            // Dye hue for the grid icon (and, via the vendor buy list's pairing to
            // these entries, its shop rows). Omitted when undyed — this array is
            // rebuilt every poll for up to 400 items.
            let hue = look.item_hue(it.graphic, it.hue);
            if hue != 0 {
                v["hue"] = json!(hue);
            }
            // Mark stackable so a dragged stack only offers the split dialog when
            // the server would actually accept a partial amount (only when true).
            if look.item_stackable(it.graphic) {
                v["st"] = json!(1);
            }
            v
        })
        .collect()
}

/// Per-open-container metadata the renderer needs to TITLE and ICON a container
/// window, keyed by container serial. Emitted beside (not folded into)
/// `contGumps` so that map's two readers stay untouched.
///
/// The crux this exists for: ServUO gives a pouch (item 0x0E79) and a backpack
/// (0x0E75) the SAME 0x24 gump id 0x3C (neither is in `Data/containers.cfg`, so
/// both fall to the default — `Container.cs`), so a window keyed on the gump id
/// alone literally cannot tell them apart. Only the container ITEM's graphic and
/// its tiledata name can, and those live in `World` (the single source of truth),
/// not scattered across the browser's equip/items/contItems arrays — so the
/// resolution belongs here.
pub(super) fn container_info_json(
    world: &World,
    look: &Look,
) -> std::collections::BTreeMap<String, Value> {
    world
        .container_gumps
        .keys()
        .map(|&serial| {
            // The container is an ordinary item tracked by serial whether it is
            // worn (the backpack), on the ground, or nested in another bag. A
            // container opened purely by a server 0x24 we never saw as an item
            // (a bank box) falls back to g=0 / no name — the window still draws
            // from its gump id, just without the icon/title refinement.
            let v = match world.items.get(&serial) {
                Some(it) => {
                    let mut m = json!({ "g": it.graphic, "name": look.item_name(it.graphic) });
                    let hue = look.item_hue(it.graphic, it.hue);
                    if hue != 0 {
                        m["hue"] = json!(hue);
                    }
                    m
                }
                None => json!({ "g": 0, "name": "" }),
            };
            (serial.to_string(), v)
        })
        .collect()
}

/// The player's mount item (layer 25): animal body to draw under the rider,
/// plus the table's vertical rider offset. `(0, 0)` = on foot.
pub(super) fn player_mount_info(world: &World, look: &Look, player: &Mobile) -> (u16, i8) {
    world
        .items
        .values()
        .find(|it| it.container == Some(player.serial) && it.layer == 25)
        .map_or((0, 0), |it| {
            mount_info_for(it.graphic, &|g| look.item_anim(g))
        })
}

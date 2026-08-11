//! Emitting the visible tile field: land, statics, and placed-multi components.
//!
//! The largest single thing `build_scene` does, and the only part that needs
//! the map **mutably** (block caching) — which is why it runs after every
//! asset lookup is finished with the shared borrow.
//!
//! The output is written as JSON text rather than built as `Value` trees:
//! tiles and statics are the bulk of a scene (≈1225 tiles plus hundreds of
//! statics), and the `Value` round-trip cost ~31ms per build, which blocked
//! the game loop (movement pacing + net pump) into a periodic stutter. There
//! are no string fields here, so hand-written JSON is safe; everything small
//! still goes through serde.

use super::*;

/// What one pass over the visible field produced. `tiles` and `statics` are
/// JSON array *bodies* — no brackets, and a trailing comma the caller trims.
/// (The static count stays internal: it only exists to cap the walk.)
#[derive(Default)]
pub(super) struct TileEmission {
    pub(super) tiles: String,
    pub(super) statics: String,
    pub(super) dbg: Vec<Value>,
}

/// Walk the field around `center` (the player's tile), emitting land and
/// statics up to `max_z`, and appending any static light sources to `lights`.
#[allow(clippy::too_many_arguments)] // each is a distinct input the walk needs
pub(super) fn emit_tiles(
    world: &World,
    map: &mut MapData,
    multis: Option<&Multis>,
    animdata: Option<&AnimData>,
    art: &mut Option<&mut Art>,
    center: (i64, i64, i32),
    max_z: i32,
    lights: &mut Vec<Value>,
) -> TileEmission {
    let (px, py, pz) = center;
    let scan = TileScan::build(world, map);
    let mut tiles = String::with_capacity(64 * 1024);
    let mut statics = String::with_capacity(16 * 1024);
    let mut n_statics = 0usize;
    let mut dbg: Vec<Value> = Vec::new();
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
                    calculate_new_z_scanned(&scan, map, multis, px + ddx, py + ddy, pz_hop, dir)
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
        world
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
    let ghost = player_is_ghost(world);
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
                        let dyn_items = dynamic_statics_at_scanned(&scan, x, y);
                        blocking_item_at_scanned(&scan, map, multis, x, y, z, dyn_items, ghost)
                            .is_none()
                    }
                    None => false,
                }
            } else {
                explain_tile_walkable_scanned(&scan, map, multis, x, y, pz).is_ok()
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
                let land_surface =
                    !land.impassable() && ((g < 0x01AE && g != 2) || (g > 0x01B5 && g != 0x01DB));
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
                explain_tile_walkable_for_planning_scanned(&scan, map, multis, x, y, pz).1
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
                        lights.push(json!({ "x": x, "y": y, "z": s.z, "r": 3,
                                            "id": map.item_light_id(s.graphic) }));
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
                        if let Some(d) = world.house_designs.get(&mserial).filter(|d| d.tiles_ready)
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
                                            lights,
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
                                lights,
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
    TileEmission {
        tiles,
        statics,
        dbg,
    }
}

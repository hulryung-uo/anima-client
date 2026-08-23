//! Replay feeds and the small field helpers the scene is assembled from.
//!
//! Each feed is a seq-stamped event log rather than state: the renderer keeps the
//! highest seq it acted on and plays only what is newer, so entries reappear
//! across polls until they age out of `World`'s capped buffer. Dropping one frame
//! is therefore harmless, which is why [`json_array`] falls back to `[]` instead
//! of failing the whole scene.

use super::*;

/// Shallow-merge `fields`'s keys into `v` (both must be JSON objects). Used to
/// splice a pure per-item helper's output into the item loop's `json!` value
/// below without duplicating field names on both sides.
pub(super) fn merge_obj(v: &mut Value, fields: Value) {
    if let (Value::Object(vm), Value::Object(fm)) = (v, fields) {
        vm.extend(fm);
    }
}

/// Stack/amount fields for a non-corpse ground item's scene JSON entry (see the
/// item loop in [`build_scene`]): `amount` always, `st:1` only when the tile is
/// Stackable — so the renderer's drag-split dialog only offers items the server
/// would actually accept a partial lift from (ClassicUO `GameActions.PickUp`'s
/// `IsStackable` gate). Pure (no Session/MapData), so it's unit-testable directly.
pub(super) fn stack_fields(amount: u16, stackable: bool) -> Value {
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
pub(super) fn corpse_fields(body: u16, hue: u16, direction: u8, death_group: u8) -> Value {
    json!({ "body": body, "dir": direction, "dg": death_group, "hue": hue })
}

/// `hidden` scene field for a mobile or the player (mobile-update status-flags
/// 0x80 bit — see [`anima_core::world::Mobile::hidden`]'s doc). Only emitted
/// when true (same small-payload convention as the item foliage `"f"` flag),
/// so the renderer's default (not hidden) needs no key at all. Pure, so it's
/// unit-testable directly.
pub(super) fn hidden_field(hidden: bool) -> Value {
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
pub(super) fn poisoned_field(poisoned: bool) -> Value {
    if poisoned {
        json!({ "poisoned": true })
    } else {
        json!({})
    }
}

/// `yellow` scene field for a mobile — the yellow/blessed health bar (0x16/0x17
/// type 2; see [`anima_core::world::Mobile::yellow_health`]'s doc). Only emitted
/// when true, same convention as [`hidden_field`].
///
/// ServUO raises it for `Blessed || YellowHealthbar` (`Mobile.GetPacketFlags`
/// bit 0x08 carries the same fact for a pre-SA client), i.e. a mobile that
/// cannot be killed. The renderer needs it because ClassicUO's ignore list
/// refuses to add one: `IgnoreManager.AddIgnoredTarget` tests `!m.IsYellowHits`
/// before taking a name.
pub(super) fn yellow_field(yellow: bool) -> Value {
    if yellow {
        json!({ "yellow": true })
    } else {
        json!({})
    }
}

/// Serialize a scene array, falling back to `[]` rather than failing the whole
/// frame. Every one of these fields is either replayable next poll or re-sent
/// every poll, so a frame the renderer can still parse beats no frame at all.
pub(super) fn json_array(v: &[Value]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

/// [`json_array`] for the fields this file builds as a finished array `Value`
/// rather than a `Vec<Value>`. (Two helpers rather than one generic one: naming
/// the bound would mean depending on `serde` directly, which nothing else here
/// needs.)
pub(super) fn json_array_value(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

/// Sound events (0x54): `id` at world `(x, y)` for positional panning.
pub(super) fn sounds_json(world: &World) -> Vec<Value> {
    world
        .recent_sounds
        .iter()
        .map(|&(seq, id, x, y)| json!({ "seq": seq, "id": id, "x": x, "y": y }))
        .collect()
}

/// Character-animation events (0x6E): play group `act` on `serial` once
/// (combat swings, bows, get-hit).
pub(super) fn anims_json(world: &World) -> Vec<Value> {
    world
        .recent_anims
        .iter()
        .map(|&(seq, serial, act, frames, fwd, delay)| {
            json!({ "seq": seq, "serial": serial, "act": act, "frames": frames, "fwd": fwd, "delay": delay })
        })
        .collect()
}

/// *Typed* animation events (0xE2): `serial` was told to play `AnimationType`
/// `typ`'s `act` (an emote/gesture/alert/…), with `mode` (the wire "delay"
/// byte) available for the client to pick a cosmetic variant. Unlike 0x6E's
/// `act`, `typ`/`act` here are NOT a raw animation group — the client converts
/// them per body (ClassicUO `GetObjectNewAnimation`), since only it knows each
/// body's animation-group layout.
pub(super) fn typed_anims_json(world: &World) -> Vec<Value> {
    world
        .recent_typed_anims
        .iter()
        .map(|&(seq, serial, typ, act, mode)| {
            json!({ "seq": seq, "serial": serial, "typ": typ, "act": act, "mode": mode })
        })
        .collect()
}

/// Damage events (0x0B): `serial` took `amt` HP. The client floats a number
/// over the target for each `seq` newer than the last it showed.
pub(super) fn damage_json(world: &World) -> Vec<Value> {
    world
        .recent_damage
        .iter()
        .map(|&(seq, serial, amt)| json!({ "seq": seq, "serial": serial, "amt": amt }))
        .collect()
}

/// Graphical effects (0x70/0xC0/0xC7): spell bolts, hit sparkles, explosions,
/// fields. The ART tile-id animation sequence + per-frame interval are resolved
/// here from `animdata.mul` so the client just cycles `frames`.
///
/// An effect whose `explode` byte was set bursts when it lands, which ClassicUO
/// does client-side by spawning a second `FixedEffect(0x36CB)` at the target
/// (`MovingEffect::RemoveMe`). The browser owns that spawn but has no
/// `animdata.mul`, so the burst's own frame list rides along on the effect that
/// causes it — `exFrames`/`exInterval`, present only when `explodes` is true.
/// The burst ClassicUO spawns at a moving effect's impact point
/// (`GameEffect::CreateExplosionEffect`), for 400 ms at speed 0.
const EXPLOSION_GRAPHIC: u16 = 0x36CB;

pub(super) fn effects_json(world: &World, animdata: Option<&AnimData>) -> Vec<Value> {
    world
        .recent_effects
        .iter()
        .map(|e| {
            let (frames, interval) = match animdata {
                Some(ad) => (ad.frame_sequence(e.graphic), ad.frames(e.graphic).1),
                None => (vec![e.graphic], 0u8),
            };
            let mut v = json!({
                "seq": e.seq, "kind": e.kind, "src": e.src_serial, "tgt": e.tgt_serial,
                "sx": e.sx, "sy": e.sy, "sz": e.sz, "tx": e.tx, "ty": e.ty, "tz": e.tz,
                "g": e.graphic, "hue": e.hue, "blend": e.blend,
                "speed": e.speed, "dur": e.duration,
                "frames": frames, "interval": interval
            });
            if e.explodes {
                let (ex_frames, ex_interval) = match animdata {
                    Some(ad) => (
                        ad.frame_sequence(EXPLOSION_GRAPHIC),
                        ad.frames(EXPLOSION_GRAPHIC).1,
                    ),
                    None => (vec![EXPLOSION_GRAPHIC], 0u8),
                };
                v["explodes"] = json!(true);
                v["exFrames"] = json!(ex_frames);
                v["exInterval"] = json!(ex_interval);
            }
            v
        })
        .collect()
}

/// Active buffs/debuffs (0xDF): icon (the upsert key), name, duration in
/// seconds, and the longer description when the server sent one.
///
/// The name is the server's own `title_cliloc` resolved here, falling back to
/// the short English table keyed off the icon — which is all the client had
/// before, and which covers 35 of the 189 icons, so most buffs used to show as
/// `#1234`.
pub(super) fn buffs_json(world: &World, cliloc: Option<&Cliloc>) -> Vec<Value> {
    world
        .buffs
        .iter()
        .map(|b| {
            let name = match b.title_cliloc {
                0 => b.name.clone(),
                id => cliloc
                    .and_then(|c| c.format(id, &b.title_args))
                    .unwrap_or_else(|| b.name.clone()),
            };
            let desc = match b.desc_cliloc {
                0 => String::new(),
                id => cliloc
                    .and_then(|c| c.format(id, &b.desc_args))
                    .unwrap_or_default(),
            };
            json!({ "icon": b.icon, "name": name, "dur": b.dur, "desc": desc })
        })
        .collect()
}

/// 0x23 DragAnimation: an item graphic flying source→dest (stack split, loot
/// pull). Same seq-stamped ring as [`effects_json`]; the renderer interpolates
/// in world tiles the way a kind-0 moving effect does.
pub(super) fn drag_anims_json(world: &World) -> Vec<Value> {
    world
        .recent_drag_anims
        .iter()
        .map(|a| {
            json!({
                "seq": a.seq, "g": a.graphic, "hue": a.hue, "n": a.count,
                "src": a.source, "tgt": a.dest,
                "sx": a.source_x, "sy": a.source_y, "sz": a.source_z,
                "tx": a.dest_x, "ty": a.dest_y, "tz": a.dest_z
            })
        })
        .collect()
}

/// The player's skills (0x3A), sorted by id. Values stay in tenths (wire)
/// units): 500 == 50.0, and the client divides by 10 for display.
/// `lock`: 0 = up, 1 = down, 2 = locked.
pub(super) fn skills_json(world: &World) -> Vec<Value> {
    let mut skills: Vec<&anima_core::world::Skill> = world.skills.values().collect();
    skills.sort_by_key(|sk| sk.id);
    skills
        .iter()
        .map(|sk| json!({ "id": sk.id, "v": sk.value, "b": sk.base, "c": sk.cap, "lock": sk.lock }))
        .collect()
}

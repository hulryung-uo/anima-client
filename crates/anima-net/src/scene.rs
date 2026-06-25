//! Builds the renderer scene JSON from a live [`Session`] + map/art data.
//! Shared by the `scene` (AI patrol) and `play` (human-controlled) bins.

use anima_assets::{Art, MapData};
use serde_json::{json, Value};

use crate::Session;

/// Half-size of the square map window emitted around the player (29×29 at 14).
pub const RADIUS: i64 = 14;

/// Serialize the current world + a map window (walkability/Z + real terrain
/// color) + entities + journal to the JSON the web renderer consumes.
pub fn build_scene(
    s: &mut Session,
    map: Option<&mut MapData>,
    mut art: Option<&mut Art>,
    journal: &[Value],
) -> String {
    let p = s.world.player_mobile().cloned().unwrap_or_default();
    let st = &s.world.player_stats;
    let (px, py, pz) = (p.pos.x as i64, p.pos.y as i64, p.pos.z as i32);

    let mobiles: Vec<Value> = s
        .world
        .mobiles
        .values()
        .filter(|m| m.serial != p.serial)
        .map(|m| {
            json!({
                "serial": m.serial,
                "x": m.pos.x, "y": m.pos.y, "z": m.pos.z, "dir": m.direction,
                "body": m.body, "noto": m.notoriety, "name": m.name
            })
        })
        .collect();
    let items: Vec<Value> = s
        .world
        .items
        .values()
        .filter(|it| it.container.is_none())
        .map(|it| json!({ "x": it.pos.x, "y": it.pos.y, "g": it.graphic, "serial": it.serial }))
        .collect();

    let mut tiles = Vec::new();
    let mut statics = Vec::new();
    if let Some(map) = map {
        for dy in -RADIUS..=RADIUS {
            for dx in -RADIUS..=RADIUS {
                let (x, y) = (px + dx, py + dy);
                if x < 0 || y < 0 {
                    tiles.push(json!({ "w": 0, "z": 0, "g": 0, "c": [10, 10, 12] }));
                    continue;
                }
                let walk = map.walkable_z(x as u32, y as u32, pz).is_some();
                let land = map.land(x as u32, y as u32);
                let c = art
                    .as_mut()
                    .map(|a| a.land_avg_color(land.graphic))
                    .unwrap_or([60, 90, 50, 255]);
                tiles.push(json!({
                    "w": walk as u8, "z": land.z, "g": land.graphic, "c": [c[0], c[1], c[2]]
                }));
                // Static objects on this tile (walls/trees/deco) for iso drawing.
                if statics.len() < 4000 {
                    for s in map.statics(x as u32, y as u32) {
                        statics.push(json!({ "x": x, "y": y, "z": s.z, "g": s.graphic }));
                    }
                }
            }
        }
    }

    json!({
        "player": {
            "x": p.pos.x, "y": p.pos.y, "z": p.pos.z, "dir": p.direction, "body": p.body, "name": p.name,
            "hits": p.hits, "hitsMax": p.hits_max, "mana": p.mana, "manaMax": p.mana_max,
            "stam": p.stam, "stamMax": p.stam_max,
            "str": st.strength, "dex": st.dexterity, "int": st.intelligence, "gold": st.gold,
        },
        "map": { "cx": px, "cy": py, "radius": RADIUS, "tiles": tiles },
        "statics": statics,
        "mobiles": mobiles,
        "items": items,
        "journal": journal,
        "stats": { "confirms": s.confirms, "denies": s.denies },
    })
    .to_string()
}

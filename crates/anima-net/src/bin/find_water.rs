//! `find-water` — locate shore tiles (dry, walkable land with water in casting
//! range) from the UO terrain map, for staging fishers. Reuses `anima-assets`'
//! map reader (water is *terrain*, not statics, so the tree-finder doesn't apply).
//!
//! Trammel (map 1) terrain mirrors Felucca (map 0) for the standard continent, so
//! map0 shore spots are valid for the map-1 agents.
//!
//! Usage: `find-water [cx] [cy] [radius]` → prints `x y z` lines (spread out).

use anima_assets::map::MapData;

const WATER: &[u16] = &[0x00A8, 0x00A9, 0x00AA, 0x00AB, 0x0136, 0x0137];
const CAST: i32 = 4; // stand within this many tiles of water

fn is_water(g: u16) -> bool {
    WATER.contains(&g)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let cx: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2899);
    let cy: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(676);
    let r: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(40);

    let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
    let mut map = MapData::open(&dir).expect("open map data");

    // Each entry: (stand_x, stand_y, stand_z, water_x, water_y, water_z).
    let mut chosen: Vec<(u32, u32, i32, u32, u32, i32)> = Vec::new();
    let mut n_water = 0u32;
    for x in cx.saturating_sub(r)..=cx + r {
        for y in cy.saturating_sub(r)..=cy + r {
            let lt = map.land(x, y);
            if is_water(lt.graphic) {
                n_water += 1;
                continue;
            }
            // Find the nearest water tile within casting range.
            let mut water: Option<(u32, u32, i32)> = None;
            'scan: for d in 1..=CAST {
                for dx in -d..=d {
                    for dy in -d..=d {
                        if dx.abs().max(dy.abs()) != d {
                            continue;
                        }
                        let (wx, wy) = (x as i32 + dx, y as i32 + dy);
                        if wx >= 0 && wy >= 0 {
                            let w = map.land(wx as u32, wy as u32);
                            if is_water(w.graphic) {
                                water = Some((wx as u32, wy as u32, w.z as i32));
                                break 'scan;
                            }
                        }
                    }
                }
            }
            let Some((wx, wy, wz)) = water else { continue };
            // A fisher is GM-placed (teleport ignores walkability) and doesn't move,
            // so any dry tile in range works. Keep spots apart so fishers don't crowd.
            if chosen.iter().all(|c| {
                (c.0 as i32 - x as i32).abs().max((c.1 as i32 - y as i32).abs()) > 6
            }) {
                chosen.push((x, y, lt.z as i32, wx, wy, wz));
            }
        }
    }
    // `stand_x stand_y stand_z  water_x water_y water_z` per line.
    for (x, y, z, wx, wy, wz) in chosen.iter().take(15) {
        println!("{x} {y} {z} {wx} {wy} {wz}");
    }
    eprintln!("[find-water] {} shore spots ({n_water} water tiles) near ({cx},{cy}) r{r}",
              chosen.len());
}

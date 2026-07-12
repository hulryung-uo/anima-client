//! Pathfinding over the UO map: A* with Z-aware, diagonal-safe steps.
//!
//! The algorithm is pure and terrain-agnostic — it queries a [`Terrain`]
//! implementation for walkability. `anima-assets` implements `Terrain` over real
//! `.mul`/`.uop` data; tests use a simple in-memory grid. The result is a list
//! of UO directions (0..7) ready to feed [`crate::net::movement::Walker`].

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::net::movement::direction_delta;

/// Walkability oracle. `&mut self` allows block caching in the implementation.
pub trait Terrain {
    /// If an entity at `from_z` can step onto `(x, y)`, return the Z it would
    /// stand at; otherwise `None`.
    fn walkable_step(&mut self, x: u32, y: u32, from_z: i32) -> Option<i32>;

    /// Serial of a **closed door** item genuinely blocking a real (non-planning)
    /// step onto `(x, y)` at `current_z`, if this terrain distinguishes "a wall"
    /// from "a closed door we could open". Route PLANNING already treats a
    /// closed door as passable (a `walkable_step` implementor that models
    /// dynamic items — e.g. `anima-net`'s `MapTerrain` — returns `Some` for a
    /// closed-door tile, same as `anima-net::scene::tile_walkable_for_planning`),
    /// so `find_path`/`find_path_near` may legitimately route straight through
    /// one; a route EXECUTOR calls this on the chosen next hop to decide
    /// whether to `Use` the door first instead of walking into what the real
    /// server would just deny. Default: no door awareness — a bare grid
    /// (tests) or a static-only terrain (`anima_assets::MapData` alone) has no
    /// dynamic items layered on top of it, so every tile it allows really is
    /// just walkable.
    fn door_at(&mut self, _x: u32, _y: u32, _current_z: i32) -> Option<u32> {
        None
    }
}

/// A* back-pointer: for a node, the (previous node, direction taken, Z reached).
type CameFrom = HashMap<(u32, u32), ((u32, u32), u8, i32)>;

/// One step of a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub dir: u8,
    pub x: u32,
    pub y: u32,
    pub z: i32,
}

/// Maximum A* node expansions before giving up (keeps a hopeless search bounded).
pub const DEFAULT_MAX_EXPANSIONS: usize = 20_000;

/// The UO direction (0..7) from a unit delta, if it is one.
#[cfg(test)]
fn delta_to_dir(dx: i32, dy: i32) -> Option<u8> {
    (0u8..8).find(|&d| direction_delta(d) == (dx, dy))
}

#[derive(Copy, Clone)]
struct Open {
    f: u32,
    g: u32,
    x: u32,
    y: u32,
    z: i32,
}

impl PartialEq for Open {
    fn eq(&self, o: &Self) -> bool {
        self.f == o.f
    }
}
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        // Min-heap on f (reverse), tie-break on lower g.
        o.f.cmp(&self.f).then_with(|| o.g.cmp(&self.g))
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn chebyshev(x0: u32, y0: u32, x1: u32, y1: u32) -> u32 {
    let dx = x0.abs_diff(x1);
    let dy = y0.abs_diff(y1);
    dx.max(dy)
}

/// Find a path from `(sx, sy, sz)` to `(gx, gy)`. Returns the sequence of steps,
/// or `None` if unreachable within `max_expansions`. Diagonal moves require both
/// flanking orthogonal tiles to be walkable (no corner cutting).
pub fn find_path(
    terrain: &mut dyn Terrain,
    (sx, sy, sz): (u32, u32, i32),
    (gx, gy): (u32, u32),
    max_expansions: usize,
) -> Option<Vec<Step>> {
    if (sx, sy) == (gx, gy) {
        return Some(Vec::new());
    }

    let mut open = BinaryHeap::new();
    // came_from: node -> (prev node, dir, z)
    let mut came_from: CameFrom = HashMap::new();
    let mut g_score: HashMap<(u32, u32), u32> = HashMap::new();

    g_score.insert((sx, sy), 0);
    open.push(Open {
        f: chebyshev(sx, sy, gx, gy),
        g: 0,
        x: sx,
        y: sy,
        z: sz,
    });

    let mut expansions = 0;
    while let Some(cur) = open.pop() {
        if (cur.x, cur.y) == (gx, gy) {
            return Some(reconstruct(&came_from, (gx, gy)));
        }
        // Skip stale heap entries (a better g was found after this was queued).
        if cur.g > *g_score.get(&(cur.x, cur.y)).unwrap_or(&u32::MAX) {
            continue;
        }
        expansions += 1;
        if expansions > max_expansions {
            return None;
        }

        for dir in 0u8..8 {
            let (dx, dy) = direction_delta(dir);
            let nx = cur.x as i64 + dx as i64;
            let ny = cur.y as i64 + dy as i64;
            if nx < 0 || ny < 0 {
                continue;
            }
            let (nx, ny) = (nx as u32, ny as u32);

            let nz = match terrain.walkable_step(nx, ny, cur.z) {
                Some(z) => z,
                None => continue,
            };
            // Anti corner-cut: a diagonal needs both orthogonal neighbours open.
            if dx != 0 && dy != 0 {
                let side_a = terrain.walkable_step((cur.x as i64 + dx as i64) as u32, cur.y, cur.z);
                let side_b = terrain.walkable_step(cur.x, (cur.y as i64 + dy as i64) as u32, cur.z);
                if side_a.is_none() || side_b.is_none() {
                    continue;
                }
            }

            let tentative = cur.g + 1;
            if tentative < *g_score.get(&(nx, ny)).unwrap_or(&u32::MAX) {
                g_score.insert((nx, ny), tentative);
                came_from.insert((nx, ny), ((cur.x, cur.y), dir, nz));
                open.push(Open {
                    f: tentative + chebyshev(nx, ny, gx, gy),
                    g: tentative,
                    x: nx,
                    y: ny,
                    z: nz,
                });
            }
        }
    }
    None
}

fn reconstruct(came_from: &CameFrom, goal: (u32, u32)) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut node = goal;
    while let Some(&(prev, dir, z)) = came_from.get(&node) {
        steps.push(Step {
            dir,
            x: node.0,
            y: node.1,
            z,
        });
        node = prev;
    }
    steps.reverse();
    steps
}

/// Like [`find_path`], but if the exact `goal` tile can't be reached, finds
/// whichever tile IS reachable that's *genuinely closest* to it (Chebyshev
/// distance), within `max_radius` — instead of giving up outright. Mirrors
/// ClassicUO's own `Pathfinder.WalkTo`, which relaxes its arrival distance to
/// 1 tile the moment the exact clicked tile is blocked (a tree, a wall
/// decoration, a crate, …) instead of refusing to move the character at all —
/// clicking *near* an obstacle should walk you up to it, not reject the
/// click, and to the CLOSEST standable tile, not just whichever one a fixed
/// probing order happens to try first (a player already standing adjacent to
/// the blocked click must resolve to their own tile, not a walk-around to
/// some farther tile that only "won" by being tried earlier).
///
/// Cost: the fast path above costs one [`find_path`] (cheap in the common
/// case — most clicks land on the exact tile, and `find_path` stops the
/// instant it dequeues the goal). Only when that fails does this run a
/// SECOND, single bounded flood from `start` — same `max_expansions` budget
/// as one `find_path` call, not a multiple of it — tracking, for every node
/// it actually expands, that node's Chebyshev distance to `goal`. Once the
/// flood exhausts its budget (or the whole reachable region, whichever comes
/// first), the winner is the expanded node with (min Chebyshev-to-goal, tie-
/// break min path cost, tie-break earliest-expanded) — a full, deterministic
/// ordering, so the result never depends on `HashMap`/search-order
/// happenstance. So an unreachable goal costs at most ~2 full budgets total,
/// not up to 25 independent ones (the old ring-probe design's real, measured
/// stall: 396ms release / 9.5s debug on a single-threaded session loop).
///
/// Returns the resolved `(x, y)` actually routed to (equal to `goal` when the
/// exact tile worked) alongside the steps (empty when `(x, y) == start` — the
/// caller is already as close as it's possible to get); `None` if nothing
/// within `max_radius` Chebyshev tiles of `goal` was reached either — a
/// genuine "no path", not just an unstandable exact tile.
pub fn find_path_near(
    terrain: &mut dyn Terrain,
    start: (u32, u32, i32),
    goal: (u32, u32),
    max_radius: u32,
    max_expansions: usize,
) -> Option<((u32, u32), Vec<Step>)> {
    if let Some(path) = find_path(terrain, start, goal, max_expansions) {
        return Some((goal, path));
    }

    let (sx, sy, sz) = start;
    let (gx, gy) = goal;

    let mut open = BinaryHeap::new();
    let mut came_from: CameFrom = HashMap::new();
    let mut g_score: HashMap<(u32, u32), u32> = HashMap::new();
    g_score.insert((sx, sy), 0);
    open.push(Open {
        f: chebyshev(sx, sy, gx, gy),
        g: 0,
        x: sx,
        y: sy,
        z: sz,
    });

    // The winner so far, ordered (Chebyshev-to-goal, path cost, discovery
    // index) — `Ord` on the tuple gives exactly the selection rule this
    // function promises: closest first, ties broken by cost, remaining ties
    // broken by whichever was expanded earliest (a total order, so the
    // result is fully deterministic regardless of the heap's internal tie
    // resolution). `node` rides alongside, not inside the ordering key.
    let mut best: Option<(u32, u32, u64)> = None;
    let mut best_node = (sx, sy);
    let mut seq: u64 = 0;
    let mut expansions = 0usize;

    while let Some(cur) = open.pop() {
        // Skip stale heap entries (a better g was found after this was queued).
        if cur.g > *g_score.get(&(cur.x, cur.y)).unwrap_or(&u32::MAX) {
            continue;
        }
        expansions += 1;
        if expansions > max_expansions {
            break;
        }

        let key = (chebyshev(cur.x, cur.y, gx, gy), cur.g, seq);
        seq += 1;
        if best.is_none_or(|b| key < b) {
            best = Some(key);
            best_node = (cur.x, cur.y);
        }

        for dir in 0u8..8 {
            let (dx, dy) = direction_delta(dir);
            let nx = cur.x as i64 + dx as i64;
            let ny = cur.y as i64 + dy as i64;
            if nx < 0 || ny < 0 {
                continue;
            }
            let (nx, ny) = (nx as u32, ny as u32);

            let nz = match terrain.walkable_step(nx, ny, cur.z) {
                Some(z) => z,
                None => continue,
            };
            // Anti corner-cut: a diagonal needs both orthogonal neighbours open.
            if dx != 0 && dy != 0 {
                let side_a = terrain.walkable_step((cur.x as i64 + dx as i64) as u32, cur.y, cur.z);
                let side_b = terrain.walkable_step(cur.x, (cur.y as i64 + dy as i64) as u32, cur.z);
                if side_a.is_none() || side_b.is_none() {
                    continue;
                }
            }

            let tentative = cur.g + 1;
            if tentative < *g_score.get(&(nx, ny)).unwrap_or(&u32::MAX) {
                g_score.insert((nx, ny), tentative);
                came_from.insert((nx, ny), ((cur.x, cur.y), dir, nz));
                open.push(Open {
                    f: tentative + chebyshev(nx, ny, gx, gy),
                    g: tentative,
                    x: nx,
                    y: ny,
                    z: nz,
                });
            }
        }
    }

    let (best_cheb, ..) = best?;
    if best_cheb > max_radius {
        return None;
    }
    let path = if best_node == (sx, sy) { Vec::new() } else { reconstruct(&came_from, best_node) };
    Some((best_node, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat grid where a set of cells is blocked.
    struct Grid {
        w: u32,
        h: u32,
        blocked: std::collections::HashSet<(u32, u32)>,
    }
    impl Terrain for Grid {
        fn walkable_step(&mut self, x: u32, y: u32, _from_z: i32) -> Option<i32> {
            if x < self.w && y < self.h && !self.blocked.contains(&(x, y)) {
                Some(0)
            } else {
                None
            }
        }
    }

    #[test]
    fn straight_line_uses_diagonals() {
        let mut g = Grid {
            w: 10,
            h: 10,
            blocked: Default::default(),
        };
        let path = find_path(&mut g, (0, 0, 0), (5, 5), 10_000).unwrap();
        // Pure diagonal: 5 steps, all direction 3 (SE = +1,+1).
        assert_eq!(path.len(), 5);
        assert!(path.iter().all(|s| s.dir == 3));
        assert_eq!(path.last().unwrap().x, 5);
        assert_eq!(path.last().unwrap().y, 5);
    }

    #[test]
    fn routes_around_a_wall() {
        // Vertical wall at x=2 for y=0..=4, leaving a gap at y=5.
        let blocked = (0..=4).map(|y| (2u32, y)).collect();
        let mut g = Grid {
            w: 10,
            h: 10,
            blocked,
        };
        let path = find_path(&mut g, (0, 0, 0), (4, 0), 10_000).unwrap();
        // Must detour around the wall (longer than the 4-step direct route).
        assert!(path.len() > 4);
        assert_eq!((path.last().unwrap().x, path.last().unwrap().y), (4, 0));
    }

    #[test]
    fn straight_open_path_cardinal() {
        // Open grid: a pure-east goal is reached in the minimal 5 steps (Chebyshev
        // distance), ending exactly on the target tile.
        let mut g = Grid {
            w: 10,
            h: 10,
            blocked: Default::default(),
        };
        let path = find_path(&mut g, (1, 1, 0), (6, 1), 10_000).unwrap();
        assert_eq!(path.len(), 5);
        assert_eq!((path.last().unwrap().x, path.last().unwrap().y), (6, 1));
    }

    #[test]
    fn same_tile_is_empty_path() {
        // Clicking your own tile: a valid, already-arrived route (empty steps).
        let mut g = Grid {
            w: 10,
            h: 10,
            blocked: Default::default(),
        };
        assert_eq!(find_path(&mut g, (3, 3, 0), (3, 3), 10_000), Some(Vec::new()));
    }

    #[test]
    fn unreachable_returns_none() {
        // Fully enclose the goal.
        let blocked = [(4, 4), (4, 5), (4, 6), (5, 4), (5, 6), (6, 4), (6, 5), (6, 6)]
            .into_iter()
            .collect();
        let mut g = Grid {
            w: 10,
            h: 10,
            blocked,
        };
        assert!(find_path(&mut g, (0, 0, 0), (5, 5), 10_000).is_none());
    }

    #[test]
    fn terrain_default_door_at_is_no_door_awareness() {
        // A plain `Terrain` impl (like `Grid` here, or `anima_assets::MapData`
        // alone) only ever models the static map — it has no concept of a
        // dynamic door item sitting on top, so the default must say "no door".
        let mut g = Grid { w: 5, h: 5, blocked: Default::default() };
        assert_eq!(Terrain::door_at(&mut g, 0, 0, 0), None);
    }

    #[test]
    fn delta_dir_roundtrip() {
        for d in 0u8..8 {
            let (dx, dy) = direction_delta(d);
            assert_eq!(delta_to_dir(dx, dy), Some(d));
        }
    }

    /// Every tile at exactly Chebyshev distance `r` from `(cx, cy)` — test-only
    /// fixture builder (production code no longer enumerates rings; see
    /// [`find_path_near`]'s doc for why a single flood replaced that).
    fn chebyshev_ring(cx: u32, cy: u32, r: i64) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() == r || dy.abs() == r {
                    let (x, y) = (cx as i64 + dx, cy as i64 + dy);
                    if x >= 0 && y >= 0 {
                        out.push((x as u32, y as u32));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn find_path_near_uses_exact_goal_when_walkable() {
        let mut g = Grid {
            w: 10,
            h: 10,
            blocked: Default::default(),
        };
        let (resolved, path) = find_path_near(&mut g, (0, 0, 0), (5, 5), 2, 10_000).unwrap();
        assert_eq!(resolved, (5, 5), "an already-walkable goal must not be adjusted");
        assert_eq!(path.last().map(|s| (s.x, s.y)), Some((5, 5)));
    }

    #[test]
    fn find_path_near_falls_back_to_nearest_walkable_tile() {
        // The exact goal (5,5) is blocked (a wall decoration, a tree, …), but
        // its neighbors are open — mirrors ClassicUO's own "click near an
        // obstacle still walks you up to it" `distance = 1` relaxation.
        let mut g = Grid {
            w: 10,
            h: 10,
            blocked: [(5, 5)].into_iter().collect(),
        };
        let (resolved, path) = find_path_near(&mut g, (0, 0, 0), (5, 5), 2, 10_000).unwrap();
        assert_ne!(resolved, (5, 5));
        assert_eq!(chebyshev(resolved.0, resolved.1, 5, 5), 1, "nearest ring (radius 1) must win");
        assert_eq!(path.last().map(|s| (s.x, s.y)), Some(resolved));
    }

    #[test]
    fn find_path_near_adjacent_south_of_blocked_goal_stays_put_not_a_far_corner() {
        // Root-cause regression (live repro): a player already standing
        // directly south of the blocked click must resolve to ITS OWN tile
        // (Chebyshev 1 from the goal, cost 0) — not walk around to some
        // farther candidate. The OLD ring-probe design tried candidates
        // around the goal in a fixed row-major order (NW, N, NE, W, E, SW,
        // S, SE) and returned the FIRST one it could reach a full path to;
        // since "S" (the player's own position here) was second-to-last in
        // that order, an open "NW" tile — reachable, but strictly farther —
        // won instead, a needless walk-around across the obstacle. The new
        // single-flood selection picks the true minimum (Chebyshev, then
        // cost), so the player's own zero-cost tile always wins any tie it's
        // party to.
        let mut g = Grid {
            w: 10,
            h: 10,
            blocked: [(5, 5)].into_iter().collect(),
        };
        let (resolved, path) = find_path_near(&mut g, (5, 6, 0), (5, 5), 2, 10_000).unwrap();
        assert_eq!(resolved, (5, 6), "already adjacent to the blocked goal — must not walk anywhere");
        assert!(path.is_empty());
    }

    #[test]
    fn find_path_near_reaches_past_a_blocked_radius_1_ring() {
        // Goal (5,5) and its entire radius-1 ring are blocked (everything
        // else on the grid is open); only radius 2 can possibly resolve this.
        // A radius-1-only search would find nothing (see the `None` case
        // below for the fully-enclosed variant of this).
        let blocked: std::collections::HashSet<(u32, u32)> = chebyshev_ring(5, 5, 0)
            .into_iter()
            .chain(chebyshev_ring(5, 5, 1))
            .collect();
        let mut g = Grid { w: 10, h: 10, blocked };
        let (resolved, _path) = find_path_near(&mut g, (0, 0, 0), (5, 5), 2, 10_000).unwrap();
        assert_eq!(chebyshev(resolved.0, resolved.1, 5, 5), 2, "radius-1 is fully blocked, so this must come from radius 2");
    }

    #[test]
    fn find_path_near_none_when_nothing_in_radius_is_reachable() {
        // A 2-tile-thick wall (radius 1 AND 2 both blocked) seals (5,5) off
        // from the outside entirely — every radius up to `max_radius` (2) is
        // either blocked outright or unreachable, so even the fallback must
        // still report a genuine "no path", not silently teleport somewhere.
        let blocked: std::collections::HashSet<(u32, u32)> = chebyshev_ring(5, 5, 1)
            .into_iter()
            .chain(chebyshev_ring(5, 5, 2))
            .collect();
        let mut g = Grid { w: 10, h: 10, blocked };
        assert!(find_path_near(&mut g, (0, 0, 0), (5, 5), 2, 10_000).is_none());
    }

    #[test]
    fn find_path_near_unreachable_goal_costs_about_one_extra_bounded_flood() {
        // The old ring-probe design ran up to 25 INDEPENDENT full-budget
        // floods for an unreachable goal — measured live: 396ms release /
        // 9.5s debug, stalling the single-threaded session loop. The new
        // design costs the fast exact-goal attempt (which, when the goal
        // turns out unreachable as here, must also run to completion) plus
        // exactly ONE fallback flood — bounded terrain-query volume that
        // stays a small multiple of `max_expansions`, not 25×. There's no
        // public expansion counter to assert on directly, so this wraps the
        // `Terrain` in a query-counting adapter as a proxy for "expansions".
        struct Counting<'a> {
            inner: &'a mut dyn Terrain,
            calls: std::cell::Cell<usize>,
        }
        impl Terrain for Counting<'_> {
            fn walkable_step(&mut self, x: u32, y: u32, from_z: i32) -> Option<i32> {
                self.calls.set(self.calls.get() + 1);
                self.inner.walkable_step(x, y, from_z)
            }
        }

        let max_expansions = 200usize;
        let blocked: std::collections::HashSet<(u32, u32)> = chebyshev_ring(5, 5, 1)
            .into_iter()
            .chain(chebyshev_ring(5, 5, 2))
            .collect();
        let mut g = Grid { w: 10, h: 10, blocked };
        let mut counting = Counting { inner: &mut g, calls: std::cell::Cell::new(0) };

        assert!(find_path_near(&mut counting, (0, 0, 0), (5, 5), 2, max_expansions).is_none());

        // Per expanded node: at most 8 directions, each up to 3 terrain
        // calls (the step itself + 2 corner-cut side checks on a diagonal) —
        // a deliberately generous per-expansion bound. Two phases (the
        // exact-goal attempt + one fallback flood) must together stay
        // within 2× that; 25 independent ring floods would have cost
        // roughly 25× as much again.
        let per_phase_bound = max_expansions * 8 * 3;
        assert!(
            counting.calls.get() <= 2 * per_phase_bound,
            "expected at most ~2 bounded floods worth of terrain queries (<= {}), got {} — total work must not scale with a ring count",
            2 * per_phase_bound,
            counting.calls.get()
        );
    }
}

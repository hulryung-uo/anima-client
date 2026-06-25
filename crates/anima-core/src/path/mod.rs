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
    fn delta_dir_roundtrip() {
        for d in 0u8..8 {
            let (dx, dy) = direction_delta(d);
            assert_eq!(delta_to_dir(dx, dy), Some(d));
        }
    }
}

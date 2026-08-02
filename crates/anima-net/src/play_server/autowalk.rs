//! Click-to-walk: turning a clicked tile into a route the game loop can step.
//!
//! The tuning constants and the pre-flight that runs *before* a route is accepted
//! — range, expansion budget, and the "is the exact clicked tile even reachable"
//! slop that decides whether to path to it or to its neighbour.

use super::*;

/// (dx, dy) tile delta → UO direction (0=N..7=NW). Inverse of `dir_delta`.
pub(super) fn delta_dir(dx: i64, dy: i64) -> u8 {
    match (dx.signum(), dy.signum()) {
        (0, -1) => 0,
        (1, -1) => 1,
        (1, 0) => 2,
        (1, 1) => 3,
        (0, 1) => 4,
        (-1, 1) => 5,
        (-1, 0) => 6,
        (-1, -1) => 7,
        _ => 0,
    }
}

/// Auto-walk (click-to-walk) tuning.
/// Slowest step cadence (ms) — ClassicUO's unmounted walk. The cadence actually
/// used per step comes from `anima_core::net::walk_pacing`, which picks the
/// right one of the four tiers from live world state; this value is only the
/// "already due" seed, so the first step of a route never waits.
pub(super) const AUTO_WALK_STEP_MS: u64 = 400;

/// Reject a click farther than this (Chebyshev tiles) so a distant/cross-map
/// click fails fast instead of churning the pathfinder.
pub(super) const AUTO_WALK_MAX_RANGE: u32 = 32;

/// Hard cap on A* node expansions per re-path (bounded, fast-fail).
pub(super) const AUTO_WALK_MAX_EXPANSIONS: usize = 4_000;

/// Give up after this many issued steps (prevents a runaway route).
pub(super) const AUTO_WALK_MAX_STEPS: u32 = 200;

/// ClassicUO bounds a server-requested 0x38 path at 10,000 explored nodes. It
/// does not impose the browser click's 32-tile range/200-step convenience cap.
pub(super) const SERVER_PATHFIND_MAX_EXPANSIONS: usize = 10_000;

pub(super) const SERVER_PATHFIND_MAX_STEPS: u32 = 10_000;

/// A `WalkTo` whose *exact* clicked tile isn't reachable (a wall decoration,
/// a tree, a crate someone dropped on it) falls back to the nearest tile
/// within this many Chebyshev tiles instead of rejecting outright — see
/// `anima_core::path::find_path_near`'s doc for why (ClassicUO parity).
pub(super) const WALKTO_GOAL_SLOP: u32 = 2;

/// ANIMA_DEBUG-only: probe the player's 8 neighbor tiles and print one compact
/// ALLOW/DENY line per direction, explaining exactly why a denied tile is
/// denied (no landing Z at all, blocked by a static/dynamic item/multi
/// component, or already blacklisted by a previous auto-walk deny —
/// `blocked` is play_server-local state, not part of the terrain check
/// itself). Called from the WalkTo arm's no-path rejection so a silent "no
/// path" has something to look at. Reuses [`can_step_to`] — the SAME gate
/// `can_walk`/`step_ok` use to decide a real committed step — so this can
/// never drift from, or explain a different question than, what actually
/// decided the walk (it used to reuse `explain_tile_walkable` instead, which
/// is a genuinely different — coarser, `walkable_z_explain`-scored —
/// question; see `can_step_to`'s doc).
pub(super) fn debug_probe_neighbors(
    world: &anima_core::World,
    map: &mut MapData,
    multis: Option<&Multis>,
    blocked: &std::collections::HashSet<(u32, u32)>,
    px: u32,
    py: u32,
    pz: i32,
) {
    for dir in 0u8..8 {
        let (dx, dy) = anima_core::net::movement::direction_delta(dir);
        let (nx, ny) = (px as i64 + dx as i64, py as i64 + dy as i64);
        if nx < 0 || ny < 0 {
            eprintln!("[pathdbg] dir={dir} ({nx},{ny}): DENY off-map");
            continue;
        }
        let (ux, uy) = (nx as u32, ny as u32);
        if blocked.contains(&(ux, uy)) {
            eprintln!("[pathdbg] dir={dir} ({ux},{uy}): DENY blacklisted");
            continue;
        }
        match can_step_to(world, map, multis, px as i64, py as i64, pz, dir) {
            Ok(z) => eprintln!("[pathdbg] dir={dir} ({ux},{uy}): ALLOW z {pz}->{z}"),
            Err(StepDeny::OffMap) => eprintln!("[pathdbg] dir={dir} ({ux},{uy}): DENY off-map"),
            Err(StepDeny::NoLanding) => {
                eprintln!(
                    "[pathdbg] dir={dir} ({ux},{uy}): DENY no-landing-z (no surface within climb range of z={pz})"
                );
            }
            Err(StepDeny::Terrain(reason)) => {
                // `can_step_to` never actually produces this variant — that's
                // `explain_tile_walkable`'s (see `StepDeny`'s doc) — kept only
                // so this match stays exhaustive if that ever changes.
                eprintln!("[pathdbg] dir={dir} ({ux},{uy}): DENY terrain {reason:?}");
            }
            Err(StepDeny::DynamicItem { graphic, item_z }) => {
                eprintln!("[pathdbg] dir={dir} ({ux},{uy}): DENY dynamic g=0x{graphic:04X} item_z={item_z}");
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WalkToStart {
    Start((u32, u32)),
    Stop,
}

/// Validate and resolve a browser- or server-requested WalkTo through the same
/// pathfinder policy. A blocked exact goal falls back to a nearby reachable
/// tile, matching ClassicUO's distance-1 relaxation; malformed/far/no-path
/// requests stop the current route and leave an explanatory system note.
pub(super) fn prepare_walkto(
    world: &mut anima_core::World,
    map: Option<&mut MapData>,
    multis: Option<&Multis>,
    x: u16,
    y: u16,
    max_range: Option<u32>,
    max_expansions: usize,
) -> WalkToStart {
    let Some((px, py, pz)) = world
        .player_mobile()
        .map(|p| (p.pos.x as u32, p.pos.y as u32, p.pos.z as i32))
    else {
        return WalkToStart::Stop;
    };
    let Some(map) = map else {
        return WalkToStart::Stop;
    };

    let (gx, gy) = (x as u32, y as u32);
    let dist = px.abs_diff(gx).max(py.abs_diff(gy));
    if let Some(max_range) = max_range {
        if dist > max_range {
            eprintln!("play: walkto ({gx},{gy}) rejected: out-of-range dist={dist}");
            world.push_system_note(format!(
                "walkto ({gx},{gy}) rejected: out of range ({dist} tiles, max {max_range})"
            ));
            return WalkToStart::Stop;
        }
    }

    let empty = std::collections::HashSet::new();
    let result = {
        let mut terrain = MapTerrain {
            world: &*world,
            map: &mut *map,
            blocked: &empty,
            multis,
        };
        find_path_near(
            &mut terrain,
            (px, py, pz),
            (gx, gy),
            WALKTO_GOAL_SLOP,
            max_expansions,
        )
    };

    match result {
        Some((goal, path)) => {
            if goal != (gx, gy) {
                eprintln!("play: walkto ({gx},{gy}) adjusted to nearest reachable tile {goal:?}");
                world.push_system_note(format!(
                    "walkto ({gx},{gy}): exact tile blocked, walking to {goal:?} instead"
                ));
            }
            if path.is_empty() {
                if goal == (gx, gy) {
                    world.push_system_note(format!("walkto ({gx},{gy}): already there"));
                }
                WalkToStart::Stop
            } else {
                WalkToStart::Start(goal)
            }
        }
        None => {
            eprintln!("play: walkto ({gx},{gy}) rejected: no path from ({px},{py},{pz})");
            world.push_system_note(format!("walkto ({gx},{gy}) rejected: no path found"));
            if std::env::var("ANIMA_DEBUG").is_ok() {
                debug_probe_neighbors(&*world, map, multis, &empty, px, py, pz);
            }
            WalkToStart::Stop
        }
    }
}

#[cfg(test)]
mod walkto_pathing_tests {
    use super::*;
    // Test-only: the door pacing constants live in `scene` and the `Terrain` trait
    // is only implemented by a test double below — neither is used by this module's
    // non-test code, so importing them here (not at the top) keeps the lib build warning-free.
    use crate::scene::{DOOR_USE_COOLDOWN, MAX_DOOR_OPEN_ATTEMPTS};
    use anima_core::path::Terrain;

    #[test]
    fn decide_blocked_step_opens_a_fresh_door() {
        // Never tried before (`pending_use_sent_at: None`) — nothing to wait
        // on, so it opens immediately regardless of `door_state_changed`.
        let now = Instant::now();
        assert_eq!(
            decide_blocked_step(Some(1234), 0, None, false, now),
            BlockedStepAction::OpenDoor(1234)
        );
    }

    #[test]
    fn decide_blocked_step_keeps_opening_up_to_the_cap_once_cooldown_elapses() {
        let now = Instant::now();
        let sent_at = now - DOOR_USE_COOLDOWN; // cooldown just fully elapsed
        for attempts in 0..MAX_DOOR_OPEN_ATTEMPTS {
            assert_eq!(
                decide_blocked_step(Some(1234), attempts, Some(sent_at), false, now),
                BlockedStepAction::OpenDoor(1234)
            );
        }
    }

    #[test]
    fn decide_blocked_step_gives_up_on_a_door_past_the_cap() {
        // A door that hasn't opened after `MAX_DOOR_OPEN_ATTEMPTS` `Use`s is
        // presumed locked — stop hammering it and treat it like a wall, so a
        // route with no other way through still ends in "boxed in" instead of
        // an infinite retry loop.
        let now = Instant::now();
        assert_eq!(
            decide_blocked_step(Some(1234), MAX_DOOR_OPEN_ATTEMPTS, None, false, now),
            BlockedStepAction::Blacklist
        );
    }

    #[test]
    fn decide_blocked_step_blacklists_a_non_door_blocker() {
        let now = Instant::now();
        assert_eq!(
            decide_blocked_step(None, 0, None, false, now),
            BlockedStepAction::Blacklist
        );
    }

    /// FIX 5 regression: a `Use` sent recently (well within
    /// [`DOOR_USE_COOLDOWN`]) with no visible door-state change yet must NOT
    /// be resent — this is exactly the >400ms-RTT race that would otherwise
    /// toggle shut a door the first `Use` was about to open.
    #[test]
    fn decide_blocked_step_awaits_a_recent_use_with_no_visible_change() {
        let now = Instant::now();
        let sent_at = now - Duration::from_millis(300);
        assert_eq!(
            decide_blocked_step(Some(1234), 1, Some(sent_at), false, now),
            BlockedStepAction::AwaitDoor
        );
    }

    /// The door's graphic changed since our last `Use` (it landed and
    /// toggled the door) — safe, and necessary (e.g. it toggled back
    /// closed), to act again immediately even though the cooldown hasn't
    /// elapsed.
    #[test]
    fn decide_blocked_step_resends_once_the_door_state_changes() {
        let now = Instant::now();
        let sent_at = now - Duration::from_millis(50);
        assert_eq!(
            decide_blocked_step(Some(1234), 1, Some(sent_at), true, now),
            BlockedStepAction::OpenDoor(1234)
        );
    }

    /// No visible state change, but the cooldown has fully elapsed — presume
    /// the previous `Use` was lost (or simply didn't take) and try again.
    #[test]
    fn decide_blocked_step_resends_once_the_cooldown_elapses() {
        let now = Instant::now();
        let sent_at = now - DOOR_USE_COOLDOWN - Duration::from_millis(1);
        assert_eq!(
            decide_blocked_step(Some(1234), 1, Some(sent_at), false, now),
            BlockedStepAction::OpenDoor(1234)
        );
    }

    /// Root-cause regression, exercised through the *real* A* adapter this
    /// bug lives in: from the live repro's exact start tile, a closed real
    /// double "wooden door" (0x06A5/0x06A7, two adjoining leaves at
    /// (1611,1591) and (1612,1591)) must not make `MapTerrain`/`find_path`
    /// report "no path" — this is what `[srv] walkto (1621,1588) rejected:
    /// no path from (1620,1595,5)` was.
    ///
    /// FIX 6: the original version of this test modeled only ONE leaf
    /// (0x06A5), leaving (1612,1591) — the second leaf's tile — completely
    /// undefended: a 1-tile gap right next to "the door" that made the goal
    /// trivially reachable regardless of whether planning ever treated the
    /// door specially. Worse, even with BOTH leaves modeled, the live map
    /// has a genuine ~29-tile detour around the east end of this building
    /// (verified with the real data via `find_path` against the STRICT,
    /// non-planning predicate at a generous expansion budget) — so even a
    /// fully-modeled door left this test passing for the wrong reason: it
    /// would have passed against the OLD, buggy strict-only planning
    /// predicate too, via that detour. `sealed` closes it off (on top of the
    /// real map, not replacing it) so the door becomes the ONLY connection;
    /// the companion assertion below proves that seal is real by checking
    /// the strict predicate finds NO path at all through it.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn find_path_routes_through_a_closed_door() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        let mut world = anima_core::World::new();
        world.items.insert(
            1_073_751_127,
            anima_core::world::Item {
                serial: 1_073_751_127,
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
            1_073_751_128,
            anima_core::world::Item {
                serial: 1_073_751_128,
                graphic: 0x06A7,
                amount: 1,
                pos: anima_core::types::Position {
                    x: 1612,
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
        // Seal the real ~29-tile detour around the east end of this building
        // (verified live against the real map data) so the double door above
        // is the ONLY connection left between start and goal — see the
        // companion strict-predicate assertion below, and this test's doc.
        let sealed: std::collections::HashSet<(u32, u32)> = (1583u32..=1599)
            .flat_map(|y| (1625u32..=1640).map(move |x| (x, y)))
            .collect();

        let path = {
            let mut terrain = MapTerrain {
                world: &world,
                map: &mut map,
                blocked: &sealed,
                multis: None,
            };
            find_path(
                &mut terrain,
                (1620, 1595, 5),
                (1621, 1588),
                AUTO_WALK_MAX_EXPANSIONS,
            )
        };
        assert!(
            path.is_some_and(|p| !p.is_empty()),
            "a closed door must not make the goal unreachable"
        );

        // Companion assertion: with the SAME seal, the STRICT predicate (a
        // real committed step — `tile_walkable`, where a closed door
        // genuinely blocks) must find NO path at all. If it found one, the
        // seal above wouldn't really make the door the sole connection, and
        // the assertion above would pass for the wrong reason — exactly the
        // bug this test exists to catch (see this test's doc).
        struct StrictTerrain<'a> {
            world: &'a anima_core::World,
            map: &'a mut MapData,
            blocked: &'a std::collections::HashSet<(u32, u32)>,
        }
        impl Terrain for StrictTerrain<'_> {
            fn walkable_step(&mut self, x: u32, y: u32, from_z: i32) -> Option<i32> {
                if self.blocked.contains(&(x, y)) {
                    return None;
                }
                if crate::scene::tile_walkable(
                    self.world, self.map, None, x as i64, y as i64, from_z,
                ) {
                    self.map.walkable_z(x, y, from_z)
                } else {
                    None
                }
            }
        }
        let mut strict = StrictTerrain {
            world: &world,
            map: &mut map,
            blocked: &sealed,
        };
        assert!(
            find_path(&mut strict, (1620, 1595, 5), (1621, 1588), 200_000).is_none(),
            "the seal must make the closed door the ONLY connection — a strict path here would mean \
             this test isn't really pinning planning-vs-strict"
        );
    }

    /// Second root-cause regression, found live while verifying the door fix:
    /// a `walkto` clicked exactly on an unstandable static (graphic 0x0A7F,
    /// `Blocked { candidate_z: 20, .. }`) at (1503,1618) got the same hard
    /// "no path" rejection from (1500,1620,20) — even though the tile right
    /// next to it is fine. `find_path_near` (mirroring ClassicUO's own
    /// `distance = 1` relaxation) must resolve to a nearby reachable tile
    /// instead of rejecting.
    #[test]
    #[ignore] // needs ~/dev/uo/uo-resource
    fn find_path_near_resolves_a_walkto_clicked_on_an_unstandable_static() {
        let dir = format!("{}/dev/uo/uo-resource", std::env::var("HOME").unwrap());
        let mut map = MapData::open(&dir).expect("open map data");
        let world = anima_core::World::new();
        let empty = std::collections::HashSet::new();

        // Confirm the premise against the real data: the exact tile really is
        // unstandable (this isn't a dynamic-item artifact of a live session).
        assert!(
            map.walkable_z(1503, 1618, 20).is_none(),
            "(1503,1618) from z=20 should be blocked by the real static in this repro"
        );

        let mut terrain = MapTerrain {
            world: &world,
            map: &mut map,
            blocked: &empty,
            multis: None,
        };
        let resolved = find_path_near(
            &mut terrain,
            (1500, 1620, 20),
            (1503, 1618),
            WALKTO_GOAL_SLOP,
            AUTO_WALK_MAX_EXPANSIONS,
        );
        let (goal, path) =
            resolved.expect("a nearby tile must be reachable even though the exact click wasn't");
        assert_ne!(
            goal,
            (1503, 1618),
            "the exact tile is unstandable, so the resolved goal must differ"
        );
        assert!(!path.is_empty());
    }
}

//! `play` — a human-controlled UO client served over HTTP.
//!
//! Holds one live [`Session`], serves the `web/` renderer + `/scene.json`, and
//! accepts `POST /input` commands (walk/say/use/attack/pickup/war) which it
//! executes on the live session. Open the page, use the keyboard, and your
//! character moves/talks on the real server.
//!
//! Usage: `play [host] [port] [user] [pass] [http_port] [web_dir] [data_dir]`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anima_assets::{
    Anim, AnimData, Art, Cliloc, Gumps, Hues, MapData, RadarCol, Sounds, Texmaps, TileData,
};
use anima_core::net::LoginConfig;
use anima_core::path::{find_path, Terrain};
use anima_core::Action;
use anima_net::scene::{
    build_scene, calculate_new_z, can_walk, render_worldmap, tile_walkable, WORLDMAP_STEP,
};
use anima_net::{Endpoint, Session};
use tiny_http::{Header, Method, Response, Server};

/// (dx, dy) tile delta → UO direction (0=N..7=NW). Inverse of [`dir_delta`].
fn delta_dir(dx: i64, dy: i64) -> u8 {
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
/// Walking step cadence (ms). ClassicUO unmounted-walk is 400ms; we don't run yet.
const AUTO_WALK_STEP_MS: u64 = 400;
/// Reject a click farther than this (Chebyshev tiles) so a distant/cross-map
/// click fails fast instead of churning the pathfinder.
const AUTO_WALK_MAX_RANGE: u32 = 32;
/// Hard cap on A* node expansions per re-path (bounded, fast-fail).
const AUTO_WALK_MAX_EXPANSIONS: usize = 4_000;
/// Give up after this many issued steps (prevents a runaway route).
const AUTO_WALK_MAX_STEPS: u32 = 200;

/// [`Terrain`] over the live map + dynamic world items, with a blacklist of tiles
/// the server has *denied* (static map says walkable, a building/blocker disagrees)
/// so re-paths route around them. Mirrors `Session::navigate_to`'s `Avoiding`.
struct MapTerrain<'a> {
    world: &'a anima_core::World,
    map: &'a mut MapData,
    blocked: &'a std::collections::HashSet<(u32, u32)>,
}

impl Terrain for MapTerrain<'_> {
    fn walkable_step(&mut self, x: u32, y: u32, from_z: i32) -> Option<i32> {
        if self.blocked.contains(&(x, y)) {
            return None;
        }
        if !tile_walkable(self.world, self.map, x as i64, y as i64, from_z) {
            return None;
        }
        self.map.walkable_z(x, y, from_z)
    }
}

fn main() {
    let mut a = std::env::args().skip(1);
    let host = a.next().unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(2594);
    let user = a.next().unwrap_or_else(|| "animaplay".into());
    let pass = a.next().unwrap_or_else(|| "animaplay".into());
    let http_port: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(8090);
    let web_dir = PathBuf::from(a.next().unwrap_or_else(|| "web".into()));
    let home = std::env::var("HOME").unwrap_or_default();
    let data_dir = a.next().unwrap_or_else(|| format!("{home}/dev/uo/uo-resource"));

    let mut map = MapData::open(&data_dir).ok();
    // Art is shared: the game loop reads avg colors, the HTTP thread encodes PNGs.
    let art: Option<Arc<Mutex<Art>>> = Art::open(&data_dir).ok().map(|a| Arc::new(Mutex::new(a)));
    let anim: Option<Arc<Anim>> = Anim::open(&data_dir).ok().map(Arc::new);
    // Gump art (gumpartLegacyMUL.uop) for the paperdoll (doll body + worn pieces).
    let gumps: Option<Arc<Gumps>> = Gumps::open(&data_dir).ok().map(Arc::new);
    // Hue table (hues.mul) for recoloring sprites (skin/clothes/hair); standalone
    // TileData for the /iteminfo route (item graphic → equipment AnimID).
    let hues: Option<Arc<Hues>> = Hues::open(&data_dir).ok().map(Arc::new);
    let tiledata: Option<Arc<TileData>> =
        TileData::open(&Path::new(&data_dir).join("tiledata.mul")).ok().map(Arc::new);
    let texmaps: Option<Arc<Texmaps>> = Texmaps::open(&data_dir).ok().map(Arc::new);
    // Cliloc table (Cliloc.enu): localized text for context-menu labels (and reusable
    // for gump/system-message clilocs). Resolved into the scene when present.
    let cliloc: Option<Arc<Cliloc>> = Cliloc::open(&data_dir).ok().map(Arc::new);
    println!("play: cliloc {}", cliloc.as_ref().map_or("not loaded".into(), |c| format!("loaded ({} entries)", c.len())));
    // animdata.mul: resolves a graphical effect's ART tile-id animation sequence +
    // frame interval (used by build_scene to bake `effects[].frames`/`interval`).
    // Read in the game-loop thread only, so a plain Option (no Arc) is enough.
    let animdata: Option<AnimData> = AnimData::open(&data_dir).ok();
    println!("play: animdata {}", if animdata.is_some() { "loaded" } else { "not loaded" });
    // Sound effects (soundLegacyMUL.uop → WAV) and the music id → mp3 path map.
    let sounds: Option<Arc<Sounds>> = Sounds::open(&data_dir).ok().map(Arc::new);
    let music: Arc<HashMap<u16, PathBuf>> = Arc::new(load_music_map(Path::new(&data_dir)));
    println!("play: {} sound assets, {} music tracks", if sounds.is_some() { "loaded" } else { "no" }, music.len());

    // Full-world map PNG, rendered once in a background thread with its *own*
    // MapData+Art so it never contends with the game loop. Served at /worldmap.png.
    let worldmap: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    {
        let (slot, ddir) = (worldmap.clone(), data_dir.clone());
        // Cache the rendered PNG to disk so the (multi-second) render only happens
        // once ever, not on every restart. Step is in the name → bumping it rebuilds.
        let cache = std::env::temp_dir().join(format!("anima-worldmap0-s{WORLDMAP_STEP}.png"));
        thread::spawn(move || {
            if let Ok(bytes) = std::fs::read(&cache) {
                println!("play: worldmap from cache ({} KB)", bytes.len() / 1024);
                *slot.lock().unwrap() = Some(bytes);
                return;
            }
            if let (Ok(mut m), Ok(rc)) = (MapData::open(&ddir), RadarCol::open(&ddir)) {
                let png = render_worldmap(&mut m, &rc, WORLDMAP_STEP);
                println!("play: worldmap ready ({} KB)", png.len() / 1024);
                let _ = std::fs::write(&cache, &png);
                *slot.lock().unwrap() = Some(png);
            }
        });
    }

    // Starting city for a newly-created character (ServUO honors the selection):
    // 0=Magincia/New Haven list-dependent, 3=Britain, ... Override via ANIMA_CITY.
    let city_index: u16 = std::env::var("ANIMA_CITY").ok().and_then(|s| s.parse().ok()).unwrap_or(3);

    // Shared scene JSON (HTTP thread reads, game loop writes) + input channel.
    let scene = Arc::new(Mutex::new(String::from("{}")));
    // `Some(action)` = do it; `None` = stop walking now (key released). The
    // explicit stop clears `desired` immediately so the server doesn't keep pacing
    // for the desired_until window and overshoot past where the player stopped
    // (which made the prediction snap forward → "jump" on stop).
    let (tx, rx) = mpsc::channel::<Option<Action>>();

    // Connected sound-SSE clients; the game loop pushes sound frames to these.
    let sse_hub: SseHub = Arc::new(Mutex::new(Vec::new()));
    // World-map POIs (towns/shops/dungeons/…), parsed once from the embedded data.
    let pois: Arc<String> = Arc::new(parse_pois());
    // Login credentials submitted by the web login page (host, port, user, pass).
    let (login_tx, login_rx) = mpsc::channel::<(String, u16, String, String)>();

    // The HTTP server comes up FIRST so the login page is reachable before we've
    // connected to any game server.
    spawn_http(SpawnHttp {
        port: http_port,
        web_dir,
        scene: scene.clone(),
        tx,
        login: login_tx,
        art: art.clone(),
        anim: anim.clone(),
        gumps: gumps.clone(),
        hues: hues.clone(),
        tiledata: tiledata.clone(),
        texmaps: texmaps.clone(),
        worldmap: worldmap.clone(),
        sounds: sounds.clone(),
        music: music.clone(),
        sse_hub: sse_hub.clone(),
        pois: pois.clone(),
    });

    // Connect to the game server. With ANIMA_LOGIN set we serve the web login page
    // and wait for the browser to POST a server + account; otherwise we auto-login
    // with the CLI host/port/user/pass (backward compatible with scripts/agents).
    let login_page = std::env::var("ANIMA_LOGIN").is_ok();
    let connect = |h: String, p: u16, u: String, pw: String| {
        let mut cfg = LoginConfig { username: u, password: pw, ..Default::default() };
        cfg.appearance.city_index = city_index;
        Session::connect_and_login(&Endpoint::new(h, p), cfg)
    };
    let mut session = if !login_page {
        println!("play: connecting to {host}:{port} as {user} ...");
        match connect(host.clone(), port, user.clone(), pass.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("login failed: {e}");
                std::process::exit(1);
            }
        }
    } else {
        *scene.lock().unwrap() = r#"{"auth":"login"}"#.into();
        println!("play: login page at http://127.0.0.1:{http_port}/  (enter server + account)");
        loop {
            let (lh, lp, lu, lpw) = match login_rx.recv() {
                Ok(v) => v,
                Err(_) => std::process::exit(1),
            };
            *scene.lock().unwrap() = r#"{"auth":"connecting"}"#.into();
            println!("play: connecting to {lh}:{lp} as {lu} ...");
            match connect(lh, lp, lu, lpw) {
                Ok(s) => break s,
                Err(e) => {
                    eprintln!("login failed: {e}");
                    let msg = format!("{e}").replace(['"', '\\', '\n'], " ");
                    *scene.lock().unwrap() = format!(r#"{{"auth":"error","msg":"{msg}"}}"#);
                }
            }
        }
    };
    println!("play: in world. open http://127.0.0.1:{http_port}/  (WASD/arrows move, T to talk)");

    let mut journal: Vec<serde_json::Value> = Vec::new();
    let mut journal_seq: u64 = 0; // monotonic id so the client floats each line once
    let mut cursor = 0usize;
    let mut last_ping = std::time::Instant::now();
    let mut last_build = Instant::now() - Duration::from_secs(1);
    // Seed from the live spawn position so the first step's Z is resolved from
    // the right current_z (not a phantom 0).
    let mut last_pos = session
        .world
        .player_mobile()
        .map(|p| (p.pos.x, p.pos.y, p.pos.z))
        .unwrap_or((0u16, 0u16, 0i8));
    let mut dirty = true;
    // Last seen seqs for the time-sensitive event queues (sound 0x54, damage 0x0B,
    // effects 0x70/0xC0/0xC7). These don't move the player or add journal lines, so
    // without this the scene would only rebuild on the 250ms timer → audible/visible
    // lag (a sound could sit up to ~250ms before it even reaches the served scene).
    // Bump `dirty` the instant any advances so the next poll (≤150ms) plays it.
    let mut last_event_seqs = (0u64, 0u64, 0u64); // (sound, damage, effect)
    let mut last_heartbeat = Instant::now(); // SSE keepalive + dead-connection reaper
    // Click-to-walk (server-paced auto-walk) state. Unlike manual walk (browser-
    // paced), the server owns the route: it re-paths to `auto_goal` each cadence,
    // issues one step, and blacklists denied tiles so it routes around them.
    let mut auto_goal: Option<(u32, u32)> = None;
    let mut auto_blocked: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    let mut auto_steps: u32 = 0;
    let mut last_step = Instant::now() - Duration::from_millis(AUTO_WALK_STEP_MS);
    // Whether the last issued step was a real move (vs a turn) and where we were
    // when we issued it — so we can detect a server deny (position didn't change).
    let mut auto_pending_move = false;
    let mut auto_from = (0u16, 0u16);
    let mut auto_target = (0u32, 0u32);
    // Movement (ClassicUO model): the *browser* is the pacer. Its prediction commits
    // one step per UO cadence (ClassicUO `Walker.LastStepRequestTime`) and sends one
    // `walk` per committed step; we just execute each step once. There is no
    // server-side pacing/`desired` window, so a key tap = exactly one step and a
    // release stops immediately — no "한 발자국 더" overshoot.
    // diagnostics
    let mut diag_since = Instant::now();
    let mut builds = 0u32;
    let mut build_max_us = 0u128;
    let mut build_sum_us = 0u128;
    let mut last_reqs = 0u64;
    let trace_t0 = Instant::now(); // ANIMA_DEBUG movement trace clock
    loop {
        // Drain input. The browser paces (ClassicUO model): each `walk` is one step
        // it already committed, so we execute it once — no `desired`/cadence here.
        // `None` (old stop signal) is now a no-op. We still resolve CanWalk so a
        // blocked diagonal slides along the wall, matching the browser's prediction.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                None => {}
                Some(Action::Walk { dir, run }) => {
                    // A manual movement key cancels any active auto-walk route.
                    auto_goal = None;
                    let (facing, px, py, pz) = session
                        .world
                        .player_mobile()
                        .map(|p| (p.direction, p.pos.x as i64, p.pos.y as i64, p.pos.z as i32))
                        .unwrap_or((dir & 7, 0, 0, 0));
                    let req = dir & 7;
                    let resolved = map
                        .as_mut()
                        .and_then(|m| can_walk(&session.world, m, px, py, pz, req));
                    let send = if facing == req {
                        resolved.map(|(nd, _, _)| nd)
                    } else {
                        Some(resolved.map(|(nd, _, _)| nd).unwrap_or(req))
                    };
                    if std::env::var("ANIMA_DEBUG").is_ok() {
                        eprintln!(
                            "[srv {}] rx walk req={req} run={} facing={facing} -> send={:?} pos=({px},{py})",
                            trace_t0.elapsed().as_millis(),
                            run as u8,
                            send
                        );
                    }
                    if let Some(sd) = send {
                        let _ = session.walk(sd, run);
                    }
                }
                // Click-to-walk: set the goal. The actual stepping happens below in
                // the loop body at the walk cadence. A far/out-of-range click is
                // rejected up front so it fails fast. A new WalkTo replaces any
                // active route (and clears the denied-tile blacklist).
                Some(Action::WalkTo { x, y }) => {
                    let here = session
                        .world
                        .player_mobile()
                        .map(|p| (p.pos.x as u32, p.pos.y as u32, p.pos.z as i32));
                    if let (Some((px, py, pz)), Some(m)) = (here, map.as_mut()) {
                        let (gx, gy) = (x as u32, y as u32);
                        let dist = px.abs_diff(gx).max(py.abs_diff(gy));
                        // Verify a route exists before committing (fail fast).
                        let reachable = dist <= AUTO_WALK_MAX_RANGE && {
                            let empty = std::collections::HashSet::new();
                            let mut terrain = MapTerrain { world: &session.world, map: m, blocked: &empty };
                            find_path(&mut terrain, (px, py, pz), (gx, gy), AUTO_WALK_MAX_EXPANSIONS)
                                .is_some_and(|p| !p.is_empty())
                        };
                        if reachable {
                            auto_goal = Some((gx, gy));
                            auto_blocked.clear();
                            auto_steps = 0;
                            auto_pending_move = false;
                            last_step = Instant::now() - Duration::from_millis(AUTO_WALK_STEP_MS);
                        } else {
                            auto_goal = None;
                        }
                    }
                }
                // Equip with layer 0 means "figure out the layer for me": look up the
                // item's graphic in the world and resolve its tiledata wear layer.
                Some(Action::Equip { serial, layer: 0 }) => {
                    let layer = session
                        .world
                        .items
                        .get(&serial)
                        .map(|it| it.graphic)
                        .and_then(|g| tiledata.as_ref().map(|t| t.item_layer(g)))
                        .unwrap_or(0);
                    let _ = session.apply_action(&Action::Equip { serial, layer });
                }
                Some(other) => {
                    let _ = session.apply_action(&other);
                }
            }
        }
        if last_ping.elapsed().as_secs() >= 15 {
            let _ = session.send(&[0x73, 0x00]);
            last_ping = std::time::Instant::now();
        }
        // Pump the network briefly (keeps input responsive).
        // Short pump so the loop ticks fast → the movement cadence gate fires near
        // its exact UO step time (low jitter). Confirms are still processed every
        // loop. (A long pump made the loop coarse → uneven step timing.)
        if session.observe(Duration::from_millis(20)).is_err() {
            eprintln!("play: connection closed");
            break;
        }

        // --- Click-to-walk advance: re-path to the goal and issue one step per
        // walk cadence (server-paced, unlike manual browser-paced walk). Confirms
        // have been processed by observe() above, so the player tile here is
        // current. Cancelled by a manual Walk / new WalkTo (handled above). ---
        if let Some((gx, gy)) = auto_goal {
            let here = session
                .world
                .player_mobile()
                .map(|p| (p.pos.x, p.pos.y, p.pos.z, p.direction));
            match here {
                Some((px, py, _, _)) if (px as u32, py as u32) == (gx, gy) => {
                    auto_goal = None; // arrived
                }
                Some((px, py, pz, facing))
                    if last_step.elapsed() >= Duration::from_millis(AUTO_WALK_STEP_MS) =>
                {
                    // Did the previous *move* land? If our tile didn't change, the
                    // server denied that tile → blacklist it so the re-path detours.
                    if auto_pending_move && (px, py) == auto_from {
                        auto_blocked.insert(auto_target);
                    }
                    auto_pending_move = false;

                    let path = map.as_mut().and_then(|m| {
                        let mut terrain =
                            MapTerrain { world: &session.world, map: m, blocked: &auto_blocked };
                        find_path(
                            &mut terrain,
                            (px as u32, py as u32, pz as i32),
                            (gx, gy),
                            AUTO_WALK_MAX_EXPANSIONS,
                        )
                    });
                    match path {
                        Some(p) if !p.is_empty() => {
                            let want = p[0].dir;
                            // Resolve like a manual key: a blocked diagonal slides to
                            // a free cardinal; a turn precedes a move on a new facing.
                            let resolved = map.as_mut().and_then(|m| {
                                can_walk(&session.world, m, px as i64, py as i64, pz as i32, want)
                            });
                            let send = if facing == want {
                                resolved.map(|(nd, _, _)| nd)
                            } else {
                                Some(resolved.map(|(nd, _, _)| nd).unwrap_or(want))
                            };
                            if let Some(sd) = send {
                                if session.walk(sd, false).unwrap_or(false) {
                                    auto_from = (px, py);
                                    // Same-facing = a real tile move; a facing change
                                    // is a turn (no move) and must not count as a deny.
                                    auto_pending_move = facing == sd;
                                    auto_target = resolved
                                        .map(|(_, nx, ny)| (nx as u32, ny as u32))
                                        .unwrap_or((px as u32, py as u32));
                                    auto_steps += 1;
                                    if auto_steps > AUTO_WALK_MAX_STEPS {
                                        auto_goal = None; // runaway guard
                                    }
                                }
                            } else {
                                // Fully blocked here → blacklist the intended tile;
                                // if the next re-path finds nothing we give up.
                                auto_blocked.insert((p[0].x, p[0].y));
                            }
                            last_step = Instant::now();
                        }
                        // No route given what we've learned → stop.
                        _ => auto_goal = None,
                    }
                }
                _ => {}
            }
        }

        let obs = session.world.observe(&mut cursor);
        for j in &obs.new_journal {
            journal_seq += 1;
            // For a localized (cliloc) line, `j.text` holds the raw tab-separated
            // args; resolve them against the Cliloc table into display text so the
            // journal + overhead show real words instead of a blank line. Fall back
            // to `#<id>` when the id isn't in the table.
            let text = if j.cliloc != 0 {
                cliloc
                    .as_deref()
                    .and_then(|c| c.format(j.cliloc, &j.text))
                    .unwrap_or_else(|| format!("#{}", j.cliloc))
            } else {
                j.text.clone()
            };
            journal.push(serde_json::json!({
                "seq": journal_seq, "serial": j.serial, "name": j.name,
                "text": text, "type": j.msg_type, "hue": j.hue, "cliloc": j.cliloc
            }));
            dirty = true;
        }
        while journal.len() > 50 {
            journal.remove(0);
        }
        // Rebuild the (expensive) scene only when the player moved, the journal
        // changed, or ~250ms passed — not on every 100ms loop iteration.
        // Include Z so climbing stairs (Z changes, maybe same X/Y) rebuilds the
        // scene → maxDrawZ recomputes and the visible floor switches with you.
        let pos = session
            .world
            .player_mobile()
            .map(|p| (p.pos.x, p.pos.y, p.pos.z))
            .unwrap_or(last_pos);
        if (pos.0, pos.1) != (last_pos.0, last_pos.1) {
            dirty = true;
            if std::env::var("ANIMA_DEBUG").is_ok() {
                eprintln!(
                    "[srv {}] MOVED ({},{}) -> ({},{})  confirms={} denies={}",
                    trace_t0.elapsed().as_millis(),
                    last_pos.0, last_pos.1, pos.0, pos.1,
                    session.confirms, session.denies
                );
            }
            // The server's ConfirmWalk (0x22) carries no Z; like ClassicUO
            // (Pathfinder.CalculateNewZ) the client resolves the standing Z of the
            // tile it stepped onto from the map — bounded by the tile it came from
            // and the step's direction, picking the surface/bridge nearest the
            // current Z with clearance. This is what makes stairs/ramps climb.
            let mut nz = pos.2;
            if let Some(m) = map.as_mut() {
                let dir = delta_dir(pos.0 as i64 - last_pos.0 as i64, pos.1 as i64 - last_pos.1 as i64);
                if let Some(z) = calculate_new_z(m, pos.0 as i64, pos.1 as i64, last_pos.2 as i32, dir) {
                    nz = z as i8;
                    if let Some(p) = session.world.player_mobile_mut() {
                        p.pos.z = nz;
                    }
                }
            }
            last_pos = (pos.0, pos.1, nz);
        }
        // A new sound/damage/effect event must be reflected immediately (not on the
        // 250ms timer), or it plays/shows late. Rebuild the scene the moment any of
        // these monotonic seqs advances.
        let seqs = (
            session.world.sound_seq,
            session.world.damage_seq,
            session.world.effect_seq,
        );
        if seqs != last_event_seqs {
            // Push each newly-arrived sound to the SSE clients immediately (no poll
            // wait). Damage/effects still ride the scene poll — only sound is pushed.
            let prev_sound = last_event_seqs.0;
            if session.world.sound_seq > prev_sound {
                for &(seq, id, x, y) in &session.world.recent_sounds {
                    if seq > prev_sound {
                        sse_broadcast(
                            &sse_hub,
                            format!("data: {{\"seq\":{seq},\"id\":{id},\"x\":{x},\"y\":{y}}}\n\n")
                                .as_bytes(),
                        );
                    }
                }
            }
            last_event_seqs = seqs;
            dirty = true;
        }
        // SSE keepalive: a periodic comment frame both keeps proxies from closing
        // the stream and lets a write to a vanished client fail → that worker thread
        // unblocks and the dead sender is reaped on the next broadcast.
        if last_heartbeat.elapsed() >= Duration::from_secs(15) {
            sse_broadcast(&sse_hub, b": ping\n\n");
            last_heartbeat = Instant::now();
        }
        if dirty || last_build.elapsed() >= Duration::from_millis(250) {
            let t0 = Instant::now();
            let mut art_guard = art.as_ref().map(|a| a.lock().unwrap());
            let json = build_scene(&mut session, map.as_mut(), art_guard.as_deref_mut(), cliloc.as_deref(), animdata.as_ref(), &journal);
            drop(art_guard);
            *scene.lock().unwrap() = json;
            last_build = Instant::now();
            dirty = false;

            let us = t0.elapsed().as_micros();
            builds += 1;
            build_sum_us += us;
            build_max_us = build_max_us.max(us);
            if us > 30_000 {
                eprintln!("[diag] slow scene build: {:.1}ms", us as f64 / 1000.0);
            }
        }

        // Periodic diagnostics line.
        if diag_since.elapsed() >= Duration::from_secs(5) {
            let reqs = REQ_COUNT.load(Ordering::Relaxed);
            let avg = if builds > 0 { build_sum_us / builds as u128 } else { 0 };
            eprintln!(
                "[diag] 5s: scene builds={builds} avg={:.1}ms max={:.1}ms | http reqs={}",
                avg as f64 / 1000.0,
                build_max_us as f64 / 1000.0,
                reqs - last_reqs,
            );
            diag_since = Instant::now();
            builds = 0;
            build_sum_us = 0;
            build_max_us = 0;
            last_reqs = reqs;
        }
    }
}

// Keyed by (is_static, graphic, hue) so hued effect frames don't collide with the
// plain terrain/static art.
type TileCache = Arc<Mutex<HashMap<(bool, u16, u16), Vec<u8>>>>;
// Keyed by (body, group, dir, frame, hue) so hued + un-hued frames don't collide.
// Cached anim frame: (PNG bytes, draw-center cx, cy). The center is sent to the
// client as headers so it can position each part (body/equipment/mount) correctly.
type AnimCache = Arc<Mutex<HashMap<(u16, u8, u8, u16, u16), (Vec<u8>, i16, i16)>>>;
type TexmapCache = Arc<Mutex<HashMap<u16, Vec<u8>>>>;
type GumpCache = Arc<Mutex<HashMap<(u32, u16), Vec<u8>>>>;

/// HTTP requests served (for the periodic diagnostics line).
static REQ_COUNT: AtomicU64 = AtomicU64::new(0);

/// Startup args for [`spawn_http`] (grouped to dodge the arg-count lint).
struct SpawnHttp {
    port: u16,
    web_dir: PathBuf,
    scene: Arc<Mutex<String>>,
    tx: mpsc::Sender<Option<Action>>,
    login: mpsc::Sender<(String, u16, String, String)>,
    art: Option<Arc<Mutex<Art>>>,
    anim: Option<Arc<Anim>>,
    gumps: Option<Arc<Gumps>>,
    hues: Option<Arc<Hues>>,
    tiledata: Option<Arc<TileData>>,
    texmaps: Option<Arc<Texmaps>>,
    worldmap: Arc<Mutex<Option<Vec<u8>>>>,
    sounds: Option<Arc<Sounds>>,
    music: Arc<HashMap<u16, PathBuf>>,
    sse_hub: SseHub,
    pois: Arc<String>,
}

fn spawn_http(args: SpawnHttp) {
    let SpawnHttp { port, web_dir, scene, tx, login, art, anim, gumps, hues, tiledata, texmaps, worldmap, sounds, music, sse_hub, pois } = args;
    let server = match Server::http(("0.0.0.0", port)) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("play: http server failed: {e}");
            return;
        }
    };
    let tile_cache: TileCache = Arc::new(Mutex::new(HashMap::new()));
    let anim_cache: AnimCache = Arc::new(Mutex::new(HashMap::new()));
    let texmap_cache: TexmapCache = Arc::new(Mutex::new(HashMap::new()));
    let gump_cache: GumpCache = Arc::new(Mutex::new(HashMap::new()));
    // Worker threads: a burst of tile/sprite PNG requests must never block the
    // frequent /scene.json polls (tiny_http's Server is shareable across threads).
    for _ in 0..6 {
        let server = server.clone();
        let web_dir = web_dir.clone();
        let scene = scene.clone();
        let tx = tx.clone();
        let login = login.clone();
        let art = art.clone();
        let anim = anim.clone();
        let gumps = gumps.clone();
        let hues = hues.clone();
        let tiledata = tiledata.clone();
        let texmaps = texmaps.clone();
        let tile_cache = tile_cache.clone();
        let anim_cache = anim_cache.clone();
        let texmap_cache = texmap_cache.clone();
        let gump_cache = gump_cache.clone();
        let worldmap = worldmap.clone();
        let sounds = sounds.clone();
        let music = music.clone();
        let sse_hub = sse_hub.clone();
        let pois = pois.clone();
        thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle_request(Ctx {
                    req,
                    web_dir: &web_dir,
                    scene: &scene,
                    tx: &tx,
                    login: &login,
                    art: &art,
                    anim: &anim,
                    gumps: &gumps,
                    hues: &hues,
                    tiledata: &tiledata,
                    texmaps: &texmaps,
                    tile_cache: &tile_cache,
                    anim_cache: &anim_cache,
                    texmap_cache: &texmap_cache,
                    gump_cache: &gump_cache,
                    worldmap: &worldmap,
                    sounds: &sounds,
                    music: &music,
                    sse_hub: &sse_hub,
                    pois: &pois,
                });
            }
        });
    }
}

/// Everything a request handler needs (groups args to dodge the arg-count lint).
struct Ctx<'a> {
    req: tiny_http::Request,
    web_dir: &'a Path,
    scene: &'a Arc<Mutex<String>>,
    tx: &'a mpsc::Sender<Option<Action>>,
    login: &'a mpsc::Sender<(String, u16, String, String)>,
    art: &'a Option<Arc<Mutex<Art>>>,
    anim: &'a Option<Arc<Anim>>,
    gumps: &'a Option<Arc<Gumps>>,
    hues: &'a Option<Arc<Hues>>,
    tiledata: &'a Option<Arc<TileData>>,
    texmaps: &'a Option<Arc<Texmaps>>,
    tile_cache: &'a TileCache,
    anim_cache: &'a AnimCache,
    texmap_cache: &'a TexmapCache,
    gump_cache: &'a GumpCache,
    worldmap: &'a Arc<Mutex<Option<Vec<u8>>>>,
    sounds: &'a Option<Arc<Sounds>>,
    music: &'a Arc<HashMap<u16, PathBuf>>,
    sse_hub: &'a SseHub,
    pois: &'a Arc<String>,
}

fn handle_request(ctx: Ctx) {
    REQ_COUNT.fetch_add(1, Ordering::Relaxed);
    let Ctx {
        mut req, web_dir, scene, tx, login, art, anim, gumps, hues, tiledata, texmaps, tile_cache,
        anim_cache, texmap_cache, gump_cache, worldmap, sounds, music, sse_hub, pois,
    } = ctx;
    let raw_url = req.url().to_string();
    // Parse the optional `?hue=<n>` query before stripping it. 0 = no hue.
    let hue = parse_hue_query(&raw_url);
    let url = raw_url.split('?').next().unwrap_or("/").to_string();
    let is_post = *req.method() == Method::Post;

    if is_post && url == "/log" {
        // Diagnostic trace from the browser: print verbatim so client + server
        // events interleave in one log (only when ANIMA_DEBUG is set).
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        if std::env::var("ANIMA_DEBUG").is_ok() {
            eprintln!("[cli] {}", body.trim());
        }
        let _ = req.respond(Response::from_string("ok"));
    } else if is_post && url == "/input" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        if body.trim() == "stop" {
            let _ = tx.send(None); // key released → stop pacing now
        } else if let Some(action) = parse_command(&body) {
            let _ = tx.send(Some(action));
        }
        let _ = req.respond(Response::from_string("ok"));
    } else if is_post && url == "/login" {
        // Web login page submitted a server + account: "host:port:user:pass" (the
        // password is the remainder, so it may itself contain ':'). Hand it to the
        // connect loop in main(); ignored if we're already in-world.
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let mut it = body.trim().splitn(4, ':');
        let h = it.next().unwrap_or("").to_string();
        let p: u16 = it.next().and_then(|s| s.parse().ok()).unwrap_or(2593);
        let u = it.next().unwrap_or("").to_string();
        let pw = it.next().unwrap_or("").to_string();
        if h.is_empty() || u.is_empty() {
            let _ = req.respond(Response::from_string("bad").with_status_code(400));
        } else {
            let _ = login.send((h, p, u, pw));
            let _ = req.respond(Response::from_string("ok"));
        }
    } else if url == "/scene.json" {
        let body = scene.lock().unwrap().clone();
        let mut r = Response::from_string(body);
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else if url == "/sounds" {
        // SSE stream. tiny_http's Response buffers the socket writer and only flushes
        // when the body completes — useless for a never-ending stream (headers never
        // reach the client). So we take the raw socket via into_writer() and write +
        // FLUSH each frame ourselves. This blocks the worker thread for the
        // connection's lifetime (one of 6 — fine for a single renderer); it ends when
        // a write fails (client gone — a heartbeat triggers this) or the hub drops us.
        let (s, rx) = mpsc::channel::<Vec<u8>>();
        sse_hub.lock().unwrap().push(s);
        let mut w = req.into_writer();
        let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
            Cache-Control: no-cache\r\nConnection: keep-alive\r\n\
            Access-Control-Allow-Origin: *\r\n\r\n: ok\n\n";
        if w.write_all(head).and_then(|_| w.flush()).is_ok() {
            while let Ok(frame) = rx.recv() {
                if w.write_all(&frame).and_then(|_| w.flush()).is_err() {
                    break;
                }
            }
        }
    } else if url == "/worldmap.png" {
        // Ready once the background render finishes; 503 (retry) until then.
        let bytes = worldmap.lock().unwrap().clone();
        match bytes {
            Some(b) => respond_png(req, b),
            None => {
                let _ = req.respond(Response::from_string("building").with_status_code(503));
            }
        }
    } else if url == "/pois.json" {
        // World-map points of interest (towns/banks/shops/dungeons/…). Static — built
        // once at startup; the client fetches it once when the world map opens.
        let mut r = Response::from_string(pois.as_str());
        r.add_header(ctype("application/json"));
        r.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"max-age=3600"[..]).unwrap());
        let _ = req.respond(r);
    } else if let Some(id) = parse_sound_url(&url) {
        serve_sound(sounds, id, req);
    } else if let Some(id) = parse_music_url(&url) {
        serve_music(music, id, req);
    } else if let Some((is_static, g)) = parse_art_url(&url) {
        serve_art(art, hues, tile_cache, is_static, g, hue, req);
    } else if let Some(id) = parse_texmap_url(&url) {
        serve_texmap(texmaps, texmap_cache, id, req);
    } else if let Some((body, group, dir)) = parse_animinfo_url(&url) {
        // Per-frame draw-centers let the renderer position each part (body, worn
        // equipment, rider on mount) correctly instead of foot-anchoring them all.
        let centers = anim.as_ref().and_then(|a| a.frame_centers(body, group, dir)).unwrap_or_default();
        let frames = centers.len();
        let c = centers.iter().map(|(cx, cy)| format!("[{cx},{cy}]")).collect::<Vec<_>>().join(",");
        let mut r = Response::from_string(format!("{{\"frames\":{frames},\"c\":[{c}]}}"));
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else if let Some(graphic) = parse_iteminfo_url(&url) {
        let anim_id = tiledata.as_ref().map(|t| t.item_anim(graphic)).unwrap_or(0);
        let mut r = Response::from_string(format!("{{\"anim\":{anim_id}}}"));
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else if let Some((body, group, dir, frame)) = parse_anim_url(&url) {
        serve_anim(anim, hues, anim_cache, body, group, dir, frame, hue, req);
    } else if let Some(id) = parse_gump_url(&url) {
        serve_gump(gumps, hues, gump_cache, id, hue, req);
    } else if let Some(hid) = url.strip_prefix("/hue/").and_then(|s| s.strip_suffix(".json")).and_then(|s| s.parse::<u16>().ok()) {
        // Resolve a hue id → a representative swatch colour (mid-bright ramp), so the
        // paperdoll can show the dye colour of hair/beard/clothing on hover.
        let c = hues.as_ref().map(|h| h.color(hid, 24)).unwrap_or([0, 0, 0, 0]);
        let mut r = Response::from_string(format!("{{\"rgb\":\"#{:02x}{:02x}{:02x}\"}}", c[0], c[1], c[2]));
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else {
        serve_static(web_dir, &url, req);
    }
}

fn respond_png(req: tiny_http::Request, bytes: Vec<u8>) {
    let mut r = Response::from_data(bytes);
    r.add_header(ctype("image/png"));
    r.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"max-age=86400"[..]).unwrap());
    let _ = req.respond(r);
}

/// Like [`respond_png`] but also sends the anim frame's draw-center as `X-Cx`/`X-Cy`
/// headers, so the renderer can place each part at `(screenX - cx, screenY - h - cy)`
/// (ClassicUO positioning) instead of a naïve foot anchor — which is what aligns
/// held items, hair, armor and a rider on a mount.
fn respond_png_center(req: tiny_http::Request, bytes: Vec<u8>, cx: i16, cy: i16) {
    let mut r = Response::from_data(bytes);
    r.add_header(ctype("image/png"));
    r.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"max-age=86400"[..]).unwrap());
    r.add_header(Header::from_bytes(&b"X-Cx"[..], cx.to_string().as_bytes()).unwrap());
    r.add_header(Header::from_bytes(&b"X-Cy"[..], cy.to_string().as_bytes()).unwrap());
    let _ = req.respond(r);
}

/// Serve audio bytes with a content type and a long cache (assets never change).
fn respond_audio(req: tiny_http::Request, bytes: Vec<u8>, mime: &str) {
    let mut r = Response::from_data(bytes);
    r.add_header(ctype(mime));
    r.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"max-age=86400"[..]).unwrap());
    let _ = req.respond(r);
}

/// Match `/sound/<id>.wav` → sound id.
fn parse_sound_url(url: &str) -> Option<u16> {
    url.strip_prefix("/sound/")?.strip_suffix(".wav")?.parse().ok()
}

fn serve_sound(sounds: &Option<Arc<Sounds>>, id: u16, req: tiny_http::Request) {
    match sounds.as_ref().and_then(|s| s.wav(id)) {
        Some(b) => respond_audio(req, b, "audio/wav"),
        None => {
            let _ = req.respond(Response::from_string("no sound").with_status_code(404));
        }
    }
}

/// Match `/music/<id>.mp3` → music id.
fn parse_music_url(url: &str) -> Option<u16> {
    url.strip_prefix("/music/")?.strip_suffix(".mp3")?.parse().ok()
}

fn serve_music(music: &Arc<HashMap<u16, PathBuf>>, id: u16, req: tiny_http::Request) {
    let bytes = music.get(&id).and_then(|p| std::fs::read(p).ok());
    match bytes {
        Some(b) => respond_audio(req, b, "audio/mpeg"),
        None => {
            let _ = req.respond(Response::from_string("no music").with_status_code(404));
        }
    }
}

/// Parse `Music/Digital/Config.txt` → music id → resolved `.mp3` path. Each line
/// is `<id> <name>[,loop]`; filenames omit the extension and UO is inconsistent
/// about case, so we resolve names case-insensitively against the actual files
/// found under `Music/` (mirrors ClassicUO `SoundsLoader.GetTrueFileName`).
fn load_music_map(data_dir: &Path) -> HashMap<u16, PathBuf> {
    let music_dir = data_dir.join("Music");
    // lowercase file stem → actual path, for all .mp3 under Music/ (recursively).
    let mut by_stem: HashMap<String, PathBuf> = HashMap::new();
    let mut stack = vec![music_dir.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("mp3")) {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    by_stem.insert(stem.to_ascii_lowercase(), p.clone());
                }
            }
        }
    }

    let mut map = HashMap::new();
    let config = music_dir.join("Digital").join("Config.txt");
    let Ok(text) = std::fs::read_to_string(&config) else {
        return map;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Tokens split on space/comma/tab (e.g. "9 britainpos,loop").
        let mut toks = line.split([' ', ',', '\t']).filter(|s| !s.is_empty());
        let Some(id) = toks.next().and_then(|t| t.parse::<u16>().ok()) else { continue };
        let Some(name) = toks.next() else { continue };
        // Strip any extension, then resolve case-insensitively to a real file.
        let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name);
        if let Some(path) = by_stem.get(&stem.to_ascii_lowercase()) {
            map.insert(id, path.clone());
        }
    }
    map
}

/// Match `/anim/<body>/<group>/<dir>/<frame>.png` → (body, group, dir, frame).
fn parse_anim_url(url: &str) -> Option<(u16, u8, u8, u16)> {
    let mut p = url.strip_prefix("/anim/")?.split('/');
    let body = p.next()?.parse().ok()?;
    let group = p.next()?.parse().ok()?;
    let dir = p.next()?.parse().ok()?;
    let frame = p.next()?.strip_suffix(".png")?.parse().ok()?;
    Some((body, group, dir, frame))
}

/// Match `/gump/<id>.png` → gump id.
fn parse_gump_url(url: &str) -> Option<u32> {
    url.strip_prefix("/gump/")?.strip_suffix(".png")?.parse().ok()
}

/// Match `/animinfo/<body>/<group>/<dir>` → (body, group, dir).
fn parse_animinfo_url(url: &str) -> Option<(u16, u8, u8)> {
    let mut p = url.strip_prefix("/animinfo/")?.split('/');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

/// Match `/iteminfo/<graphic>` → graphic. Resolves a worn item's AnimID.
fn parse_iteminfo_url(url: &str) -> Option<u16> {
    url.strip_prefix("/iteminfo/")?.parse().ok()
}

/// Extract `hue=<n>` from a raw URL query string (`...?hue=123`). 0 if absent.
fn parse_hue_query(raw_url: &str) -> u16 {
    let Some(q) = raw_url.split('?').nth(1) else { return 0 };
    for kv in q.split('&') {
        if let Some(v) = kv.strip_prefix("hue=") {
            return v.parse().unwrap_or(0);
        }
    }
    0
}

#[allow(clippy::too_many_arguments)]
fn serve_anim(
    anim: &Option<Arc<Anim>>,
    hues: &Option<Arc<Hues>>,
    cache: &AnimCache,
    body: u16,
    group: u8,
    dir: u8,
    frame: u16,
    hue: u16,
    req: tiny_http::Request,
) {
    let key = (body, group, dir, frame, hue);
    if let Some((b, cx, cy)) = cache.lock().unwrap().get(&key).cloned() {
        return respond_png_center(req, b, cx, cy);
    }
    // Decode outside the cache lock so concurrent requests don't serialize.
    // Apply the hue (skin/clothes/hair/equipment recolor) before PNG-encoding.
    let out = anim
        .as_ref()
        .and_then(|a| a.frame(body, group, dir, frame as usize))
        .map(|(mut i, cx, cy)| {
            if hue != 0 {
                if let Some(h) = hues.as_ref() {
                    anima_assets::apply_hue(&mut i, h, hue);
                }
            }
            (i.to_png(), cx, cy)
        });
    match out {
        Some((b, cx, cy)) => {
            cache.lock().unwrap().insert(key, (b.clone(), cx, cy));
            respond_png_center(req, b, cx, cy);
        }
        None => {
            let _ = req.respond(Response::from_string("no anim").with_status_code(404));
        }
    }
}

fn serve_gump(
    gumps: &Option<Arc<Gumps>>,
    hues: &Option<Arc<Hues>>,
    cache: &GumpCache,
    id: u32,
    hue: u16,
    req: tiny_http::Request,
) {
    let key = (id, hue);
    if let Some(b) = cache.lock().unwrap().get(&key).cloned() {
        return respond_png(req, b);
    }
    let bytes = gumps
        .as_ref()
        .and_then(|g| g.get(id as usize))
        .map(|mut i| {
            if hue != 0 {
                if let Some(h) = hues.as_ref() {
                    anima_assets::apply_hue(&mut i, h, hue);
                }
            }
            i.to_png()
        });
    match bytes {
        Some(b) => {
            cache.lock().unwrap().insert(key, b.clone());
            respond_png(req, b);
        }
        None => {
            let _ = req.respond(Response::from_string("no gump").with_status_code(404));
        }
    }
}

/// Match `/texmap/<id>.png` → texmap id.
fn parse_texmap_url(url: &str) -> Option<u16> {
    url.strip_prefix("/texmap/")?.strip_suffix(".png")?.parse().ok()
}

fn serve_texmap(texmaps: &Option<Arc<Texmaps>>, cache: &TexmapCache, id: u16, req: tiny_http::Request) {
    if let Some(b) = cache.lock().unwrap().get(&id).cloned() {
        return respond_png(req, b);
    }
    let bytes = texmaps.as_ref().and_then(|t| t.texmap(id)).map(|i| i.to_png());
    match bytes {
        Some(b) => {
            cache.lock().unwrap().insert(id, b.clone());
            respond_png(req, b);
        }
        None => {
            let _ = req.respond(Response::from_string("no texmap").with_status_code(404));
        }
    }
}

/// Match `/art/land/<g>.png` or `/art/static/<g>.png` → (is_static, graphic).
fn parse_art_url(url: &str) -> Option<(bool, u16)> {
    let rest = url.strip_prefix("/art/")?;
    let (kind, file) = rest.split_once('/')?;
    let g: u16 = file.strip_suffix(".png")?.parse().ok()?;
    match kind {
        "land" => Some((false, g)),
        "static" => Some((true, g)),
        _ => None,
    }
}

fn serve_art(
    art: &Option<Arc<Mutex<Art>>>,
    hues: &Option<Arc<Hues>>,
    cache: &TileCache,
    is_static: bool,
    g: u16,
    hue: u16,
    req: tiny_http::Request,
) {
    let key = (is_static, g, hue);
    if let Some(b) = cache.lock().unwrap().get(&key).cloned() {
        return respond_png(req, b);
    }
    // Hold the Art lock only for the raw decode, not the PNG encode. A nonzero hue
    // (graphical effects pass `?hue=`) recolors the tile like /anim and /gump do.
    let bytes = art
        .as_ref()
        .and_then(|a| {
            let guard = a.lock().unwrap();
            if is_static {
                guard.static_tile(g)
            } else {
                guard.land(g)
            }
        })
        .map(|mut i| {
            if hue != 0 {
                if let Some(h) = hues.as_ref() {
                    anima_assets::apply_hue(&mut i, h, hue);
                }
            }
            i.to_png()
        });
    match bytes {
        Some(b) => {
            cache.lock().unwrap().insert(key, b.clone());
            respond_png(req, b);
        }
        None => {
            let _ = req.respond(Response::from_string("no art").with_status_code(404));
        }
    }
}

/// Points of interest (towns, banks, shops, dungeons, moongates, shrines, …) for
/// the world map, parsed from ServUO's UOAM-style `Data/Common.map` (embedded at
/// build time). Each non-header line is `[+|-]<category>: <x> <y> <z> [name]`,
/// where the category may contain spaces (e.g. `weapons guild`). Returns a JSON
/// array string `[{"x":..,"y":..,"cat":"..","name":".."}, …]` built once at startup.
fn parse_pois() -> String {
    const RAW: &str = include_str!("../../data/Common.map");
    let mut out: Vec<serde_json::Value> = Vec::new();
    for line in RAW.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Header is a bare count (e.g. "3"); every POI line has a "category:" head.
        let Some(colon) = line.find(':') else { continue };
        let cat = line[..colon].trim_start_matches(['+', '-']).trim().to_ascii_lowercase();
        if cat.is_empty() {
            continue;
        }
        let mut rest = line[colon + 1..].split_whitespace();
        let (Some(xs), Some(ys), Some(_zs)) = (rest.next(), rest.next(), rest.next()) else {
            continue;
        };
        let (Ok(x), Ok(y)) = (xs.parse::<i32>(), ys.parse::<i32>()) else { continue };
        let name = rest.collect::<Vec<_>>().join(" ");
        out.push(serde_json::json!({ "x": x, "y": y, "cat": cat, "name": name }));
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

fn serve_static(web_dir: &Path, url: &str, req: tiny_http::Request) {
    let rel = if url == "/" { "index.html" } else { url.trim_start_matches('/') };
    // Prevent path traversal.
    if rel.contains("..") {
        let _ = req.respond(Response::from_string("bad path").with_status_code(400));
        return;
    }
    let path = web_dir.join(rel);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let mut r = Response::from_data(bytes);
            r.add_header(ctype(content_type(rel)));
            // Never cache the app shell (index.html / main.js / css) — Safari caches
            // it aggressively without this, so code changes never reached the page.
            r.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store, must-revalidate"[..]).unwrap());
            let _ = req.respond(r);
        }
        Err(_) => {
            let _ = req.respond(Response::from_string("404").with_status_code(404));
        }
    }
}

fn ctype(v: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], v.as_bytes()).unwrap()
}

// ── Sound push channel (Server-Sent Events) ────────────────────────────────
// Sounds used to ride the 150ms scene poll, so a hit could play up to a poll late.
// Instead the game loop pushes each sound the instant it arrives over an SSE stream
// (`GET /sounds`). The hub is the set of connected clients' senders; the loop
// broadcasts `data: {"seq":..,"id":..}\n\n` frames (plus a periodic heartbeat that
// also reaps dead connections, since a blocked reader only unblocks on a failed write).
type SseHub = Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>;

/// Send a raw SSE frame to every connected client; drop any whose receiver is gone.
fn sse_broadcast(hub: &SseHub, frame: &[u8]) {
    let mut g = hub.lock().unwrap();
    g.retain(|s| s.send(frame.to_vec()).is_ok());
}

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") {
        "text/javascript"
    } else if path.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}

/// Parse a `cmd:arg` input line into an [`Action`]. Supported:
/// `walk:<dir>:<run>` · `run:<dir>` · `say:<text>` · `use:<serial>` ·
/// `click:<serial>` · `attack:<serial>` · `pickup:<serial>[:<amount>]` ·
/// `drop:<serial>:<x>:<y>:<z>[:<container>]` (container default 0xFFFFFFFF =
/// ground) · `equip:<serial>[:<layer>]` (layer 0 = derive from tiledata) ·
/// `war:<0|1>` · `cast:<spellId>` · `target:<serial>` · `targetxy:<x>:<y>:<z>:<graphic>` ·
/// `gump:<serial>:<gumpId>:<button>[:sw=1,2][:e=<id>=<text>,…]` (gump reply; text
/// entries can't contain `:`, `,`, or `=`).
fn parse_command(body: &str) -> Option<Action> {
    let body = body.trim();
    let (cmd, arg) = body.split_once(':').unwrap_or((body, ""));
    match cmd {
        "walk" => {
            let mut p = arg.split(':');
            let dir: u8 = p.next()?.parse().ok()?;
            let run = p.next() == Some("1");
            Some(Action::Walk { dir: dir & 7, run })
        }
        "run" => Some(Action::Walk { dir: arg.parse::<u8>().ok()? & 7, run: true }),
        // walkto:<x>,<y> — click-to-walk: pathfind to a ground tile and auto-walk.
        "walkto" => {
            let (x, y) = arg.split_once(',')?;
            Some(Action::WalkTo { x: x.trim().parse().ok()?, y: y.trim().parse().ok()? })
        }
        "say" => Some(Action::Say { text: arg.to_string() }),
        "party" => Some(Action::PartySay { text: arg.to_string() }),
        "use" => Some(Action::Use { serial: parse_serial(arg)? }),
        "click" => Some(Action::Click { serial: parse_serial(arg)? }),
        "attack" => Some(Action::Attack { serial: parse_serial(arg)? }),
        // Auto-attack the best in-view hostile (last target, else nearest hostile).
        "autoattack" => Some(Action::AutoAttack),
        // Re-attack the remembered "last target".
        "attacklast" => Some(Action::AttackLast),
        "pickup" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let amount = p.next().and_then(|s| s.parse().ok()).unwrap_or(1);
            Some(Action::PickUp { serial, amount })
        }
        "drop" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            Some(Action::Drop {
                serial,
                x: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                y: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                z: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                container: p.next().and_then(parse_serial).unwrap_or(0xFFFF_FFFF),
            })
        }
        "equip" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            // layer 0 = "derive from the item's tiledata layer" (done in the loop).
            let layer = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            Some(Action::Equip { serial, layer })
        }
        "war" => Some(Action::WarMode { on: arg == "1" || arg == "on" }),
        "cast" => Some(Action::CastSpell { spell: arg.parse().ok()? }),
        // ability:<id> — arm a weapon special move (0 disarms). 0xD7 UseCombatAbility.
        "ability" => Some(Action::UseAbility { ability: arg.parse().ok()? }),
        // buy:<vendor>:<serial>x<amt>,<serial>x<amt>,…  (amount defaults to 1)
        "buy" => {
            let (vendor, list) = arg.split_once(':')?;
            Some(Action::BuyItems {
                vendor: parse_serial(vendor)?,
                items: parse_shop_items(list),
            })
        }
        // sell:<vendor>:<serial>x<amt>,…
        "sell" => {
            let (vendor, list) = arg.split_once(':')?;
            Some(Action::SellItems {
                vendor: parse_serial(vendor)?,
                items: parse_shop_items(list),
            })
        }
        // gump:<serial>:<gumpId>:<button>[:sw=1,2,3][:e=<id>=<text>,<id>=<text>]
        // Answer a server gump (0xB0/0xDD). `button` 0 = close/cancel. The optional
        // `sw=` group lists checked switch ids; the optional `e=` group lists text
        // entries as `<id>=<text>` (text may contain anything except a comma).
        "gump" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let gump_id = parse_serial(p.next()?)?;
            let button = p.next().and_then(parse_serial).unwrap_or(0);
            let mut switches = Vec::new();
            let mut entries = Vec::new();
            for seg in p {
                if let Some(sw) = seg.strip_prefix("sw=") {
                    switches = sw
                        .split(',')
                        .filter_map(|s| if s.is_empty() { None } else { s.parse().ok() })
                        .collect();
                } else if let Some(es) = seg.strip_prefix("e=") {
                    for pair in es.split(',') {
                        if let Some((id, text)) = pair.split_once('=') {
                            if let Ok(id) = id.parse::<u16>() {
                                entries.push((id, text.to_string()));
                            }
                        }
                    }
                }
            }
            Some(Action::GumpResponse { serial, gump_id, button, switches, entries })
        }
        // oplreq:<serial> — request an entity's Object Property List / tooltip (0xD6).
        "oplreq" => Some(Action::OplRequest { serial: parse_serial(arg)? }),
        // partyinvite — invite a player (0xBF/0x06/0x01); the server opens a target cursor.
        "partyinvite" => Some(Action::PartyInvite),
        // partyleave — leave the party (0xBF/0x06/0x02, self serial filled by the driver).
        "partyleave" => Some(Action::PartyLeave),
        // partyaccept[:<leader>] — accept an invite (0xBF/0x06/0x08). Defaults to the
        // pending inviter when no serial is given (the UI omits it).
        "partyaccept" => Some(Action::PartyAccept { leader: parse_serial(arg).unwrap_or(0) }),
        // partydecline[:<leader>] — decline an invite (0xBF/0x06/0x09).
        "partydecline" => Some(Action::PartyDecline { leader: parse_serial(arg).unwrap_or(0) }),
        // popupreq:<serial> — request the right-click context menu (0xBF/0x13).
        "popupreq" => Some(Action::PopupRequest { serial: parse_serial(arg)? }),
        // popupsel:<serial>:<index> — choose an entry from the open menu (0xBF/0x15).
        "popupsel" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let index = p.next()?.parse().ok()?;
            Some(Action::PopupSelect { serial, index })
        }
        // bookreq:<serial>:<count> — request all pages of the open book (0x66).
        "bookreq" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let pages = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            Some(Action::BookRequest { serial, pages })
        }
        // skilllock:<id>:<lock> — set a skill's lock (0=up,1=down,2=locked). 0x3A.
        "skilllock" => {
            let mut p = arg.split(':');
            let skill = p.next()?.parse().ok()?;
            let lock = p.next()?.parse().ok()?;
            Some(Action::SkillLock { skill, lock })
        }
        // useskill:<id> — invoke an active skill (0x12 ActionRequest type 0x24).
        "useskill" => Some(Action::UseSkill { skill: arg.parse().ok()? }),
        "target" => Some(Action::TargetObject { serial: parse_serial(arg)? }),
        "targetcancel" => Some(Action::TargetCancel),
        "targetxy" => {
            let mut p = arg.split(':');
            Some(Action::TargetGround {
                x: p.next()?.parse().ok()?,
                y: p.next()?.parse().ok()?,
                z: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                graphic: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            })
        }
        _ => None,
    }
}

/// Parse a comma-separated `<serial>x<amt>` list (amount defaults to 1) into
/// `(serial, amount)` pairs, skipping any malformed entry. e.g.
/// `0x4000001x3,0x4000002` → `[(0x4000001, 3), (0x4000002, 1)]`.
fn parse_shop_items(list: &str) -> Vec<(u32, u16)> {
    list.split(',')
        .filter_map(|e| {
            let e = e.trim();
            if e.is_empty() {
                return None;
            }
            let (s, a) = e.split_once('x').unwrap_or((e, "1"));
            let serial = parse_serial(s)?;
            let amount = a.trim().parse().unwrap_or(1);
            Some((serial, amount))
        })
        .collect()
}

fn parse_serial(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

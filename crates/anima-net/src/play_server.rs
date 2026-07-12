//! Library form of the `play` bin: a human-controlled UO client served over
//! HTTP. Holds one live [`Session`], serves the web renderer + `/scene.json`,
//! and accepts `POST /input` commands (walk/say/use/attack/pickup/war) which
//! it executes on the live session.
//!
//! Split in two so a caller can learn the bound HTTP port before blocking:
//! [`bind`] loads assets, starts the HTTP server (workers included) and
//! returns a [`PlayServer`]; [`PlayServer::run`] then does the (blocking)
//! login + game loop. The `play` bin is a thin wrapper over these two calls;
//! `anima-desktop` uses the same pair with an ephemeral port and embedded
//! web assets so it needs no `web/` directory on disk.
//!
//! Usage (bin): `play [host] [port] [user] [pass] [http_port] [web_dir] [data_dir]`

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anima_assets::{
    Anim, AnimData, Art, Cliloc, Gumps, Hues, MapData, Multis, RadarCol, Sounds, Texmaps, TileData, ZReason,
};
use anima_core::net::LoginConfig;
use anima_core::path::{find_path, find_path_near};
use anima_core::Action;
use include_dir::{include_dir, Dir};
use tiny_http::{Header, Method, Response, Server};

use crate::regions::GuardRect;
use crate::scene::{
    build_scene, calculate_new_z, can_walk, decide_blocked_step, door_blocking_at, explain_tile_walkable,
    render_worldmap, BlockedStepAction, DoorUseAttempt, MapTerrain, StepDeny, WORLDMAP_STEP,
};
use crate::{Endpoint, Session};

/// Bundled copy of `web/` (renderer + PixiJS vendor lib), embedded at compile
/// time so this crate can serve the client with no `web/` directory on disk —
/// needed by `anima-desktop`, which runs outside the repo checkout. `web/` is
/// plain JS plus one vendored PixiJS build (~1MB total): small enough to embed
/// with no build step (`include_dir` is pure Rust, no `build.rs`, no bundler).
/// [`serve_static`] prefers a real disk `web_dir` when one is configured and
/// has the file; this is only the fallback.
static EMBEDDED_WEB: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../web");

/// (dx, dy) tile delta → UO direction (0=N..7=NW). Inverse of `dir_delta`.
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
/// A `WalkTo` whose *exact* clicked tile isn't reachable (a wall decoration,
/// a tree, a crate someone dropped on it) falls back to the nearest tile
/// within this many Chebyshev tiles instead of rejecting outright — see
/// `anima_core::path::find_path_near`'s doc for why (ClassicUO parity).
const WALKTO_GOAL_SLOP: u32 = 2;

/// ANIMA_DEBUG-only: probe the player's 8 neighbor tiles and print one compact
/// ALLOW/DENY line per direction, explaining exactly why a denied tile is
/// denied (out-of-climb-range, blocked by an overlapping static, no surface at
/// all, blocked by a placed world item, or already blacklisted by a previous
/// auto-walk deny — `blocked` is play_server-local state, not part of the
/// terrain check itself). Called from the WalkTo arm's no-path rejection so a
/// silent "no path" has something to look at. Reuses [`explain_tile_walkable`]
/// so this can never drift from the real walkability check.
fn debug_probe_neighbors(
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
        match explain_tile_walkable(world, map, multis, nx, ny, pz) {
            Ok(z) => eprintln!("[pathdbg] dir={dir} ({ux},{uy}): ALLOW z {pz}->{z}"),
            Err(StepDeny::OffMap) => eprintln!("[pathdbg] dir={dir} ({ux},{uy}): DENY off-map"),
            Err(StepDeny::Terrain(ZReason::NoSurface)) => {
                eprintln!("[pathdbg] dir={dir} ({ux},{uy}): DENY no-surface");
            }
            Err(StepDeny::Terrain(ZReason::OutOfReach { nearest_z })) => {
                eprintln!(
                    "[pathdbg] dir={dir} ({ux},{uy}): DENY z-delta player_z={pz} cand_z={nearest_z} (Δ{})",
                    (nearest_z - pz).abs()
                );
            }
            Err(StepDeny::Terrain(ZReason::Blocked { candidate_z, blocking_graphic })) => {
                eprintln!(
                    "[pathdbg] dir={dir} ({ux},{uy}): DENY static g=0x{blocking_graphic:04X} cand_z={candidate_z} (player z={pz})"
                );
            }
            Err(StepDeny::DynamicItem { graphic, item_z }) => {
                eprintln!("[pathdbg] dir={dir} ({ux},{uy}): DENY dynamic g=0x{graphic:04X} item_z={item_z}");
            }
        }
    }
}

/// Startup configuration for the play server.
pub struct PlayConfig {
    /// UO game-server host to log into.
    pub host: String,
    /// UO game-server port.
    pub port: u16,
    pub user: String,
    pub pass: String,
    /// HTTP port to serve the renderer on. `0` = OS-assigned (ephemeral) —
    /// read the real port back from [`PlayServer::port`] after [`bind`].
    pub http_port: u16,
    /// Disk directory holding `web/` (index.html/main.js/vendor/…). `None`
    /// (or a path that doesn't exist / is missing a file) falls back to the
    /// copy embedded in this binary at compile time.
    pub web_dir: Option<PathBuf>,
    /// UO client data directory (`.mul`/`.uop` files).
    pub data_dir: PathBuf,
    /// Serve the browser login page (server/account form) and wait for a
    /// `POST /login` instead of auto-logging in with `host`/`port`/`user`/`pass`.
    pub login_page: bool,
    /// Address to bind the HTTP server to. Should be `"127.0.0.1"` (loopback
    /// only) for any caller that doesn't have a specific reason to allow LAN
    /// access — the `play` bin's `ANIMA_BIND` env var is the one sanctioned
    /// escape hatch (see `bin/play.rs`); `anima-desktop` always hardcodes
    /// `"127.0.0.1"` regardless of environment, since it must never expose
    /// this process to the network.
    pub bind_addr: String,
}

/// A bound-but-not-yet-running play server: the HTTP side (and its worker
/// threads) are already listening; [`run`](PlayServer::run) does the
/// (blocking) game-server login + loop.
pub struct PlayServer {
    cfg: PlayConfig,
    port: u16,
    map: Option<MapData>,
    // Multi (house/boat) component reader — a placed multi's component list
    // (dx/dy/dz + graphic) never varies per facet, unlike `map`, so this is
    // loaded once at `bind()` and never reloaded on a facet switch.
    multis: Option<Multis>,
    art: Option<Arc<Mutex<Art>>>,
    anim: Option<Arc<Anim>>,
    cliloc: Option<Arc<Cliloc>>,
    animdata: Option<AnimData>,
    tiledata: Option<Arc<TileData>>,
    scene: Arc<Mutex<String>>,
    rx: mpsc::Receiver<Option<Action>>,
    login_rx: mpsc::Receiver<(String, u16, String, String)>,
    sse_hub: SseHub,
    /// Current session facet (`World::map_index`), kept in step with the game
    /// loop so the `/regions.json` HTTP thread can filter guard-zone rects to
    /// the facet the player is actually on without touching `scene`'s JSON.
    facet: Arc<AtomicU8>,
}

/// Load assets, bind the HTTP server (workers included), and return a
/// [`PlayServer`] with the real bound port available via
/// [`PlayServer::port`] — before any game-server connection is attempted, so
/// a caller (e.g. `anima-desktop`) can open a browser/webview at the right
/// URL right away. The login page (if `cfg.login_page`) or the auto-login
/// connect loop, and the game loop itself, only run once [`PlayServer::run`]
/// is called.
pub fn bind(cfg: PlayConfig) -> io::Result<PlayServer> {
    let data_dir = cfg.data_dir.clone();
    let mut map = MapData::open(&data_dir).ok();
    // Multi (house/boat) component reader — `multi.idx`/`multi.mul`. Same
    // dataset regardless of facet, so loaded once here (unlike `map`, which
    // reloads per facet in the game loop).
    let multis: Option<Multis> = Multis::open(&data_dir).ok();
    println!("play: multis {}", if multis.is_some() { "loaded" } else { "not loaded" });
    // Art is shared: the game loop reads avg colors, the HTTP thread encodes PNGs.
    let art: Option<Arc<Mutex<Art>>> = Art::open(&data_dir).ok().map(|a| Arc::new(Mutex::new(a)));
    let anim: Option<Arc<Anim>> = Anim::open(&data_dir).ok().map(Arc::new);
    // Gump art (gumpartLegacyMUL.uop) for the paperdoll (doll body + worn pieces).
    let gumps: Option<Arc<Gumps>> = Gumps::open(&data_dir).ok().map(Arc::new);
    // Hue table (hues.mul) for recoloring sprites (skin/clothes/hair); standalone
    // TileData for the /iteminfo route (item graphic → equipment AnimID).
    let hues: Option<Arc<Hues>> = Hues::open(&data_dir).ok().map(Arc::new);
    let tiledata: Option<Arc<TileData>> =
        TileData::open(&data_dir.join("tiledata.mul")).ok().map(Arc::new);
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
    let music: Arc<HashMap<u16, PathBuf>> = Arc::new(load_music_map(&data_dir));
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
    // Guard-zone rectangles: parsed once from a local ServUO `Regions.xml` if one
    // is reachable (`$ANIMA_REGIONS` or `$HOME/dev/uo/servuo/Data/Regions.xml` —
    // see `regions::resolve_path`). This is server-local data with no packet
    // equivalent, so a remote server with no local copy just gets no overlay
    // (never fails the server).
    let regions_path = crate::regions::resolve_path();
    let guard_rects: Arc<Vec<GuardRect>> = Arc::new(match std::fs::read_to_string(&regions_path) {
        Ok(xml) => {
            let rects = crate::regions::parse(&xml);
            println!("play: regions loaded ({} guarded rects from {})", rects.len(), regions_path.display());
            rects
        }
        Err(_) => {
            println!("regions: not loaded");
            Vec::new()
        }
    });
    // Current session facet, kept current by the game loop each tick so the
    // `/regions.json` HTTP thread can filter to the facet the player is on.
    let facet: Arc<AtomicU8> = Arc::new(AtomicU8::new(0));
    // Login credentials submitted by the web login page (host, port, user, pass).
    let (login_tx, login_rx) = mpsc::channel::<(String, u16, String, String)>();

    // The HTTP server comes up FIRST so the login page is reachable before we've
    // connected to any game server. Bound to loopback by default — this process
    // must never accept a connection from off the machine unless the caller
    // opted in via `cfg.bind_addr` (see its doc comment / the `play` bin's
    // `ANIMA_BIND`).
    let server = match Server::http((cfg.bind_addr.as_str(), cfg.http_port)) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("play: http server failed: {e}");
            return Err(io::Error::other(e));
        }
    };
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(cfg.http_port);

    spawn_http(
        server,
        SpawnHttp {
            web_dir: cfg.web_dir.clone(),
            scene: scene.clone(),
            tx,
            login: login_tx,
            art: art.clone(),
            anim: anim.clone(),
            gumps,
            hues,
            tiledata: tiledata.clone(),
            texmaps,
            worldmap,
            sounds,
            music,
            sse_hub: sse_hub.clone(),
            pois,
            guard_rects,
            facet: facet.clone(),
        },
    );

    Ok(PlayServer { cfg, port, map: map.take(), multis, art, anim, cliloc, animdata, tiledata, scene, rx, login_rx, sse_hub, facet })
}

impl PlayServer {
    /// The HTTP port actually bound (resolves `PlayConfig.http_port == 0`).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Log in (auto or via the served login page) and run the game loop.
    /// Blocks until the game connection closes.
    pub fn run(self) -> io::Result<()> {
        let PlayServer { cfg, port, mut map, multis, art, anim, cliloc, animdata, tiledata, scene, rx, login_rx, sse_hub, facet } = self;

        // Starting city for a newly-created character (ServUO honors the selection):
        // 0=Magincia/New Haven list-dependent, 3=Britain, ... Override via ANIMA_CITY.
        let city_index: u16 = std::env::var("ANIMA_CITY").ok().and_then(|s| s.parse().ok()).unwrap_or(3);

        // Connect to the game server. With login_page we serve the web login page
        // and wait for the browser to POST a server + account; otherwise we auto-login
        // with the configured host/port/user/pass (backward compatible with scripts/agents).
        let connect = |h: String, p: u16, u: String, pw: String| {
            let mut c = LoginConfig { username: u, password: pw, ..Default::default() };
            c.appearance.city_index = city_index;
            Session::connect_and_login(&Endpoint::new(h, p), c)
        };
        let mut session = if !cfg.login_page {
            println!("play: connecting to {}:{} as {} ...", cfg.host, cfg.port, cfg.user);
            match connect(cfg.host.clone(), cfg.port, cfg.user.clone(), cfg.pass.clone()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("login failed: {e}");
                    // Library code must not exit the process out from under an
                    // embedding GUI (anima-desktop) — return the error instead;
                    // the `play` bin maps it back to the same log line + exit(1).
                    return Err(io::Error::other(e));
                }
            }
        } else {
            *scene.lock().unwrap() = r#"{"auth":"login"}"#.into();
            println!("play: login page at http://127.0.0.1:{port}/  (enter server + account)");
            loop {
                let (lh, lp, lu, lpw) = match login_rx.recv() {
                    Ok(v) => v,
                    // Sender dropped (the HTTP worker pool is gone) — nothing can
                    // submit the login form anymore. Same reasoning as above: return
                    // rather than exit, so an embedding GUI keeps control.
                    Err(e) => return Err(io::Error::other(e)),
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
        println!("play: in world. open http://127.0.0.1:{port}/  (WASD/arrows move, T to talk)");

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
        // Bookkeeping for `Use` attempts sent to open a closed door blocking a
        // given tile on the current route — see `decide_blocked_step`, which
        // this feeds: how many attempts so far, when the most recent one was
        // sent, and the door's own graphic at that moment (to detect a
        // visible state change since — see `DoorUseAttempt`'s doc).
        let mut auto_door_attempts: std::collections::HashMap<(u32, u32), DoorUseAttempt> =
            std::collections::HashMap::new();
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
                            .and_then(|m| can_walk(&session.world, m, multis.as_ref(), px, py, pz, req));
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
                            if dist > AUTO_WALK_MAX_RANGE {
                                // Always print — right now a walkto rejection is 100%
                                // silent to both the log and the player; this is the fix.
                                eprintln!("play: walkto ({gx},{gy}) rejected: out-of-range dist={dist}");
                                session.world.push_system_note(format!(
                                    "walkto ({gx},{gy}) rejected: out of range ({dist} tiles, max {AUTO_WALK_MAX_RANGE})"
                                ));
                                auto_goal = None;
                            } else {
                                // Verify a route exists before committing (fail fast). If the
                                // exact clicked tile isn't reachable — a wall decoration, a
                                // tree, a crate someone dropped on it — fall back to the
                                // nearest reachable tile within `WALKTO_GOAL_SLOP` of it
                                // instead of rejecting outright, mirroring ClassicUO's own
                                // `Pathfinder.WalkTo` (its `distance = 1` relaxation for a
                                // blocked exact tile — see `find_path_near`'s doc).
                                let empty = std::collections::HashSet::new();
                                let mut terrain =
                                    MapTerrain { world: &session.world, map: &mut *m, blocked: &empty, multis: multis.as_ref() };
                                let resolved = find_path_near(
                                    &mut terrain,
                                    (px, py, pz),
                                    (gx, gy),
                                    WALKTO_GOAL_SLOP,
                                    AUTO_WALK_MAX_EXPANSIONS,
                                );
                                match resolved {
                                    Some((goal, path)) => {
                                        // `goal != (gx, gy)` means `find_path_near` adjusted the
                                        // click (the exact tile was blocked) — note it regardless
                                        // of whether any steps are actually needed, so the
                                        // adjacent-south-of-an-obstacle case (adjusted goal ==
                                        // where we're already standing) still surfaces *why* we
                                        // didn't move, not just that we didn't.
                                        if goal != (gx, gy) {
                                            eprintln!(
                                                "play: walkto ({gx},{gy}) adjusted to nearest reachable tile {goal:?}"
                                            );
                                            session.world.push_system_note(format!(
                                                "walkto ({gx},{gy}): exact tile blocked, walking to {goal:?} instead"
                                            ));
                                        }
                                        if path.is_empty() {
                                            // Already standing at `goal` — either the click landed
                                            // on our own tile, or (the adjacent-south case) it's
                                            // the nearest reachable tile and that happens to be
                                            // where we already are. Either way this is a legitimate
                                            // "arrived", not FIX 3's false "no path found" reject:
                                            // an empty path from `find_path_near` no longer implies
                                            // failure (only `None` does).
                                            if goal == (gx, gy) {
                                                session.world.push_system_note(format!("walkto ({gx},{gy}): already there"));
                                            }
                                            auto_goal = None;
                                        } else {
                                            auto_goal = Some(goal);
                                            auto_blocked.clear();
                                            auto_door_attempts.clear();
                                            auto_steps = 0;
                                            auto_pending_move = false;
                                            last_step = Instant::now() - Duration::from_millis(AUTO_WALK_STEP_MS);
                                        }
                                    }
                                    None => {
                                        eprintln!("play: walkto ({gx},{gy}) rejected: no path from ({px},{py},{pz})");
                                        session.world.push_system_note(format!(
                                            "walkto ({gx},{gy}) rejected: no path found"
                                        ));
                                        if std::env::var("ANIMA_DEBUG").is_ok() {
                                            // `empty`, not `auto_blocked`: this probe explains the
                                            // reachability check that just ran above, which (as a
                                            // fresh WalkTo, not an in-progress route) used no blacklist.
                                            debug_probe_neighbors(&session.world, m, multis.as_ref(), &empty, px, py, pz);
                                        }
                                        auto_goal = None;
                                    }
                                }
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
                                MapTerrain { world: &session.world, map: m, blocked: &auto_blocked, multis: multis.as_ref() };
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
                                    can_walk(&session.world, m, multis.as_ref(), px as i64, py as i64, pz as i32, want)
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
                                    // Fully blocked here. A closed door isn't a wall — it's
                                    // something we can open (see `decide_blocked_step`) — so
                                    // try that a bounded number of times before giving up on
                                    // the tile like any other blocker.
                                    let tile = (p[0].x, p[0].y);
                                    let door = map.as_ref().and_then(|m| {
                                        door_blocking_at(&session.world, m, tile.0 as i64, tile.1 as i64, pz as i32)
                                    });
                                    let prior = auto_door_attempts.get(&tile).copied();
                                    let attempts = prior.map_or(0, |p| p.count);
                                    // Has the door's own graphic moved since our last `Use`? If
                                    // so, that `Use` already landed (ServUO toggled it) — safe
                                    // (and necessary, e.g. it toggled back closed) to act again
                                    // immediately, cooldown or not. `door` being `None` here
                                    // (the tile's blocker vanished/changed identity) also counts
                                    // as "changed" so a stale wait can't get stuck.
                                    let door_state_changed = match (door, prior) {
                                        (Some(serial), Some(p)) => session
                                            .world
                                            .items
                                            .get(&serial)
                                            .is_none_or(|it| it.graphic != p.graphic_at_send),
                                        _ => true,
                                    };
                                    let pending_use_sent_at = prior.map(|p| p.sent_at);
                                    match decide_blocked_step(door, attempts, pending_use_sent_at, door_state_changed, Instant::now())
                                    {
                                        BlockedStepAction::OpenDoor(serial) => {
                                            if std::env::var("ANIMA_DEBUG").is_ok() {
                                                eprintln!(
                                                    "play: walkto ({gx},{gy}) opening door {serial:#x} at {tile:?} (attempt {})",
                                                    attempts + 1
                                                );
                                            }
                                            let graphic_at_send =
                                                session.world.items.get(&serial).map_or(0, |it| it.graphic);
                                            auto_door_attempts.insert(
                                                tile,
                                                DoorUseAttempt { count: attempts + 1, sent_at: Instant::now(), graphic_at_send },
                                            );
                                            let _ = session.apply_action(&Action::Use { serial });
                                        }
                                        BlockedStepAction::AwaitDoor => {
                                            // A `Use` for this door hasn't had time to land / show
                                            // an effect yet — do nothing this tick (see
                                            // `decide_blocked_step`'s doc); resending now would
                                            // risk toggling shut what the first `Use` is about to
                                            // open (the very race FIX 5 exists to close).
                                        }
                                        BlockedStepAction::Blacklist => {
                                            auto_blocked.insert(tile);
                                        }
                                    }
                                }
                                last_step = Instant::now();
                            }
                            // No route given what we've learned (boxed in by newly-
                            // blacklisted denied tiles) → stop, and say so. This fires at
                            // most once per abandoned route (clearing `auto_goal` stops
                            // this block from running again), so no spam risk.
                            _ => {
                                eprintln!("play: walkto ({gx},{gy}) abandoned: boxed in");
                                session
                                    .world
                                    .push_system_note(format!("walkto ({gx},{gy}) abandoned: boxed in"));
                                auto_goal = None;
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Keep the shared facet in step so `/regions.json` can filter its
            // guard-zone rects to wherever the player currently is (0xBF/0x08
            // MapChange updates `world.map_index` directly; see its doc).
            facet.store(session.world.map_index, Ordering::Relaxed);
            // Reload MapData when the server moves us to a different facet, so
            // land/statics come from the right map files (Malas/Ilshenar/…) instead
            // of staying on Felucca. Reload only on an actual change; if the new
            // facet's files won't open, keep the current map rather than going blank.
            let want_facet = session.world.map_index;
            if map.as_ref().map(MapData::facet) != Some(want_facet) {
                match MapData::open_facet(&cfg.data_dir, want_facet) {
                    Ok(m) => map = Some(m),
                    Err(e) => eprintln!("play: facet {want_facet} map load failed: {e} (keeping current map)"),
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
                    if let Some(z) =
                        calculate_new_z(&session.world, m, multis.as_ref(), pos.0 as i64, pos.1 as i64, last_pos.2 as i32, dir)
                    {
                        nz = z as i8;
                        if let Some(p) = session.world.player_mobile_mut() {
                            p.pos.z = nz;
                        }
                        // Stairs/ramps show up here as a Z change with the same (or a
                        // 1-tile) X/Y — best-effort detail only (diagnostics, not
                        // correctness-critical): name the static whose [z, z+height)
                        // span covers the resolved Z if one is cheaply findable, else
                        // just say the land surface accounts for it.
                        if std::env::var("ANIMA_DEBUG").is_ok() && nz != last_pos.2 {
                            let land_z = m.land(pos.0 as u32, pos.1 as u32).z;
                            let static_note = m
                                .statics(pos.0 as u32, pos.1 as u32)
                                .into_iter()
                                .find(|s| (s.z as i32) <= z && z <= s.z as i32 + s.height as i32)
                                .map(|s| format!("static g=0x{:04X} top={}", s.graphic, s.z as i32 + s.height as i32))
                                .unwrap_or_else(|| "land surface accounts for it".to_string());
                            eprintln!(
                                "play: step dir={dir} ({},{}) z {} -> {nz} (land z={land_z}, {static_note})",
                                pos.0, pos.1, last_pos.2
                            );
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
                let json = build_scene(
                    &mut session,
                    map.as_mut(),
                    art_guard.as_deref_mut(),
                    cliloc.as_deref(),
                    animdata.as_ref(),
                    anim.as_deref(),
                    multis.as_ref(),
                    &journal,
                );
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
        Ok(())
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
    web_dir: Option<PathBuf>,
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
    guard_rects: Arc<Vec<GuardRect>>,
    facet: Arc<AtomicU8>,
}

/// Spawn the worker-thread pool serving `server` (already bound by [`bind`]).
fn spawn_http(server: Arc<Server>, args: SpawnHttp) {
    let SpawnHttp { web_dir, scene, tx, login, art, anim, gumps, hues, tiledata, texmaps, worldmap, sounds, music, sse_hub, pois, guard_rects, facet } = args;
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
        let guard_rects = guard_rects.clone();
        let facet = facet.clone();
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
                    guard_rects: &guard_rects,
                    facet: &facet,
                });
            }
        });
    }
}

/// Everything a request handler needs (groups args to dodge the arg-count lint).
struct Ctx<'a> {
    req: tiny_http::Request,
    web_dir: &'a Option<PathBuf>,
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
    guard_rects: &'a Arc<Vec<GuardRect>>,
    facet: &'a Arc<AtomicU8>,
}

fn handle_request(ctx: Ctx) {
    REQ_COUNT.fetch_add(1, Ordering::Relaxed);
    let Ctx {
        mut req, web_dir, scene, tx, login, art, anim, gumps, hues, tiledata, texmaps, tile_cache,
        anim_cache, texmap_cache, gump_cache, worldmap, sounds, music, sse_hub, pois, guard_rects, facet,
    } = ctx;
    let raw_url = req.url().to_string();
    // Parse the optional `?hue=<n>` query before stripping it. 0 = no hue.
    let hue = parse_hue_query(&raw_url);
    let url = raw_url.split('?').next().unwrap_or("/").to_string();
    let is_post = *req.method() == Method::Post;

    // CSRF guard: every state-changing route here is a POST (`/input`, `/login`,
    // `/log`), and with the `play` bin's well-known port a malicious page loaded
    // in any tab could otherwise drive the session with no preflight (simple
    // requests aren't subject to CORS). A browser always sends `Origin` on a
    // cross-origin request and can't be told not to, so reject when it disagrees
    // with `Host`. No `Origin` header (curl/scripts/same-origin form posts) is
    // let through unchanged — this only blocks cross-origin *browser* requests.
    if is_post && !origin_allowed(header_value(&req, "Origin"), header_value(&req, "Host")) {
        let _ = req.respond(Response::from_string("cross-origin request rejected").with_status_code(403));
        return;
    }

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
        // connect loop in `PlayServer::run`; ignored if we're already in-world.
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
        // Stream on a DEDICATED thread, not the shared worker pool: an SSE connection
        // lives for the page's lifetime, so blocking a pooled worker here meant a few
        // browser refreshes (each leaving a stale stream until the next heartbeat
        // reaps it) could occupy all workers → /scene.json and /login stopped
        // responding ("can't connect"). The worker returns to the pool immediately.
        thread::spawn(move || {
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
        });
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
    } else if url == "/regions.json" {
        // Guard-zone (guarded-region) rectangles for the CURRENT facet only —
        // `guard_rects` holds every facet's, so filter by the live `facet` the
        // game loop keeps updated. No Cache-Control: unlike `/pois.json` this
        // depends on server-side session state (the facet can change mid-session
        // via a moongate/sewer), so the client must always get a fresh answer for
        // whichever facet it's asking about "now".
        let cur = facet.load(Ordering::Relaxed);
        let body = regions_json(guard_rects, cur);
        let mut r = Response::from_string(body);
        r.add_header(ctype("application/json"));
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

/// Case-insensitively look up a request header's value.
fn header_value<'a>(req: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

/// CSRF guard: is a POST from this `Origin` (if any) allowed against this
/// `Host`? A missing `Origin` (curl, scripts, same-origin form posts) is
/// always allowed — only a *present-but-mismatched* `Origin` is rejected, so
/// this blocks cross-origin browser requests without affecting anything else.
/// Pure and unit-tested (`play_server` otherwise has none — see FIX 4).
fn origin_allowed(origin: Option<&str>, host: Option<&str>) -> bool {
    let (Some(origin), Some(host)) = (origin, host) else { return true };
    // `Origin` is `<scheme>://<host>[:<port>]`; strip the scheme to compare
    // against `Host`'s `<host>[:<port>]`.
    let origin_host = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    origin_host.eq_ignore_ascii_case(host)
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
    const RAW: &str = include_str!("../data/Common.map");
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

/// Build the `/regions.json` body: every guarded rect tagged for facet `cur`,
/// as `[{"x":..,"y":..,"w":..,"h":..}, …]`. `facet` is omitted per-rect since
/// the whole array is already filtered to one.
fn regions_json(rects: &[GuardRect], cur: u8) -> String {
    let mut out = String::from("[");
    let mut first = true;
    for r in rects.iter().filter(|r| r.facet == cur) {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!("{{\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}", r.x, r.y, r.w, r.h));
    }
    out.push(']');
    out
}

/// Serve a `web/` static asset. A configured `web_dir` on disk wins when it has
/// the file; otherwise (or with `web_dir: None`) fall back to the copy embedded
/// in the binary at compile time ([`EMBEDDED_WEB`]) — this is what lets
/// `anima-desktop` serve the renderer with no `web/` directory on disk at all.
fn serve_static(web_dir: &Option<PathBuf>, url: &str, req: tiny_http::Request) {
    let rel = if url == "/" { "index.html" } else { url.trim_start_matches('/') };
    // Prevent path traversal.
    if rel.contains("..") {
        let _ = req.respond(Response::from_string("bad path").with_status_code(400));
        return;
    }
    let bytes = web_dir
        .as_ref()
        .and_then(|d| std::fs::read(d.join(rel)).ok())
        .or_else(|| EMBEDDED_WEB.get_file(rel).map(|f| f.contents().to_vec()));
    match bytes {
        Some(bytes) => {
            let mut r = Response::from_data(bytes);
            r.add_header(ctype(content_type(rel)));
            // Never cache the app shell (index.html / main.js / css) — Safari caches
            // it aggressively without this, so code changes never reached the page.
            r.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store, must-revalidate"[..]).unwrap());
            let _ = req.respond(r);
        }
        None => {
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
/// entries can't contain `:`, `,`, or `=`) · `prompt:<text>` / `promptcancel`
/// (answer/cancel a pending server text prompt, 0xC2 UnicodePrompt) ·
/// `tradeaccept:<mycont>:<0|1>` / `tradecancel:<mycont>` /
/// `tradegold:<mycont>:<gold>:<platinum>` (answer the secure-trade session
/// keyed by our own container serial `mycont`, 0x6F — multiple concurrent
/// sessions with different opponents are addressed by their own `mycont`,
/// from `scene.trades[].myCont`; items move via the normal `drop` command
/// targeting that same container serial).
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
        // Accept either delimiter: the web client sends `x,y`, but the whole input
        // line is already colon-split, so a hand-typed `walkto:x:y` (the natural
        // guess, and what tripped up shell/GM testing) must not silently no-op.
        "walkto" => {
            let (x, y) = arg.split_once([',', ':'])?;
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
        // prompt:<text> — answer a pending server text prompt (0xC2 UnicodePrompt:
        // pet rename, house sign, guild abbreviation, …).
        "prompt" => Some(Action::PromptResponse { text: arg.to_string() }),
        // promptcancel — cancel a pending server text prompt (Esc).
        "promptcancel" => Some(Action::PromptCancel),
        // tradeaccept:<mycont>:<0|1> — toggle our accept checkbox on the secure
        // trade session keyed by our own container serial (0x6F action 2).
        "tradeaccept" => {
            let mut p = arg.split(':');
            let container = parse_serial(p.next()?)?;
            let accept = p.next() == Some("1");
            Some(Action::TradeAccept { container, accept })
        }
        // tradecancel:<mycont> — cancel the secure trade session keyed by our
        // own container serial (0x6F action 1).
        "tradecancel" => Some(Action::TradeCancel { container: parse_serial(arg)? }),
        // tradegold:<mycont>:<gold>:<platinum> — set our virtual gold/platinum
        // offer on the session keyed by our own container serial. Parsed as u64
        // and saturated to u32::MAX rather than the usual `.ok()` "couldn't
        // parse → 0" fallback — a fat-fingered over-range entry (e.g.
        // 5000000000) must clamp, not silently become a 0-gold offer.
        "tradegold" => {
            let mut p = arg.split(':');
            let container = parse_serial(p.next()?)?;
            let gold = p.next().and_then(parse_saturating_u32).unwrap_or(0);
            let platinum = p.next().and_then(parse_saturating_u32).unwrap_or(0);
            Some(Action::TradeGold { container, gold, platinum })
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

/// Parse a decimal amount that may overflow `u32` (e.g. a mistyped gold
/// entry), saturating to `u32::MAX` instead of the `.ok()` pattern's usual
/// "couldn't parse → 0" fallback — a huge-but-real offer should clamp, not
/// silently vanish to zero.
fn parse_saturating_u32(s: &str) -> Option<u32> {
    s.trim().parse::<u64>().ok().map(|v| v.min(u32::MAX as u64) as u32)
}

#[cfg(test)]
mod csrf_tests {
    use super::origin_allowed;

    #[test]
    fn no_origin_header_is_allowed() {
        // curl / scripts / same-origin form posts never send Origin.
        assert!(origin_allowed(None, Some("127.0.0.1:8090")));
    }

    #[test]
    fn matching_origin_is_allowed() {
        assert!(origin_allowed(Some("http://127.0.0.1:8090"), Some("127.0.0.1:8090")));
    }

    #[test]
    fn scheme_is_ignored() {
        assert!(origin_allowed(Some("https://127.0.0.1:8090"), Some("127.0.0.1:8090")));
    }

    #[test]
    fn mismatched_origin_is_rejected() {
        assert!(!origin_allowed(Some("http://evil.example:1234"), Some("127.0.0.1:8090")));
    }

    #[test]
    fn no_host_header_is_allowed() {
        // Malformed request with no Host at all — nothing to compare against;
        // not this guard's job to reject it.
        assert!(origin_allowed(Some("http://evil.example"), None));
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
        assert_eq!(decide_blocked_step(None, 0, None, false, now), BlockedStepAction::Blacklist);
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
                pos: anima_core::types::Position { x: 1611, y: 1591, z: 0 },
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
                pos: anima_core::types::Position { x: 1612, y: 1591, z: 0 },
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
        let sealed: std::collections::HashSet<(u32, u32)> =
            (1583u32..=1599).flat_map(|y| (1625u32..=1640).map(move |x| (x, y))).collect();

        let path = {
            let mut terrain = MapTerrain { world: &world, map: &mut map, blocked: &sealed, multis: None };
            find_path(&mut terrain, (1620, 1595, 5), (1621, 1588), AUTO_WALK_MAX_EXPANSIONS)
        };
        assert!(path.is_some_and(|p| !p.is_empty()), "a closed door must not make the goal unreachable");

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
                if crate::scene::tile_walkable(self.world, self.map, None, x as i64, y as i64, from_z) {
                    self.map.walkable_z(x, y, from_z)
                } else {
                    None
                }
            }
        }
        let mut strict = StrictTerrain { world: &world, map: &mut map, blocked: &sealed };
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

        let mut terrain = MapTerrain { world: &world, map: &mut map, blocked: &empty, multis: None };
        let resolved =
            find_path_near(&mut terrain, (1500, 1620, 20), (1503, 1618), WALKTO_GOAL_SLOP, AUTO_WALK_MAX_EXPANSIONS);
        let (goal, path) = resolved.expect("a nearby tile must be reachable even though the exact click wasn't");
        assert_ne!(goal, (1503, 1618), "the exact tile is unstandable, so the resolved goal must differ");
        assert!(!path.is_empty());
    }
}

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

use anima_assets::{Anim, Art, MapData};
use anima_core::net::LoginConfig;
use anima_core::Action;
use anima_net::scene::build_scene;
use anima_net::{Endpoint, Session};
use tiny_http::{Header, Method, Response, Server};

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

    // Starting city for a newly-created character (ServUO honors the selection):
    // 0=Magincia/New Haven list-dependent, 3=Britain, ... Override via ANIMA_CITY.
    let city_index: u16 = std::env::var("ANIMA_CITY").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let mut cfg = LoginConfig {
        username: user.clone(),
        password: pass,
        ..Default::default()
    };
    cfg.appearance.city_index = city_index;
    println!("play: connecting to {host}:{port} as {user} ...");
    let mut session = match Session::connect_and_login(&Endpoint::new(host, port), cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("login failed: {e}");
            std::process::exit(1);
        }
    };
    println!("play: in world. open http://127.0.0.1:{http_port}/  (WASD/arrows move, T to talk)");

    // Shared scene JSON (HTTP thread reads, game loop writes) + input channel.
    let scene = Arc::new(Mutex::new(String::from("{}")));
    let (tx, rx) = mpsc::channel::<Action>();

    spawn_http(http_port, web_dir, scene.clone(), tx, art.clone(), anim.clone());

    let mut journal: Vec<serde_json::Value> = Vec::new();
    let mut cursor = 0usize;
    let mut last_ping = std::time::Instant::now();
    let mut last_build = Instant::now() - Duration::from_secs(1);
    let mut last_pos = (0u16, 0u16);
    let mut dirty = true;
    // diagnostics
    let mut diag_since = Instant::now();
    let mut builds = 0u32;
    let mut build_max_us = 0u128;
    let mut build_sum_us = 0u128;
    let mut last_reqs = 0u64;
    loop {
        // Apply any queued player input immediately.
        while let Ok(action) = rx.try_recv() {
            let _ = session.apply_action(&action);
        }
        if last_ping.elapsed().as_secs() >= 15 {
            let _ = session.send(&[0x73, 0x00]);
            last_ping = std::time::Instant::now();
        }
        // Pump the network briefly (keeps input responsive).
        if session.observe(Duration::from_millis(100)).is_err() {
            eprintln!("play: connection closed");
            break;
        }
        let obs = session.world.observe(&mut cursor);
        for j in &obs.new_journal {
            journal.push(serde_json::json!({ "name": j.name, "text": j.text, "type": j.msg_type }));
            dirty = true;
        }
        while journal.len() > 12 {
            journal.remove(0);
        }
        // Rebuild the (expensive) scene only when the player moved, the journal
        // changed, or ~250ms passed — not on every 100ms loop iteration.
        let pos = session
            .world
            .player_mobile()
            .map(|p| (p.pos.x, p.pos.y))
            .unwrap_or(last_pos);
        if pos != last_pos {
            dirty = true;
            last_pos = pos;
        }
        if dirty || last_build.elapsed() >= Duration::from_millis(250) {
            let t0 = Instant::now();
            let mut art_guard = art.as_ref().map(|a| a.lock().unwrap());
            let json = build_scene(&mut session, map.as_mut(), art_guard.as_deref_mut(), &journal);
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

type TileCache = Arc<Mutex<HashMap<(bool, u16), Vec<u8>>>>;
type AnimCache = Arc<Mutex<HashMap<(u16, u8, u8, u16), Vec<u8>>>>;

/// HTTP requests served (for the periodic diagnostics line).
static REQ_COUNT: AtomicU64 = AtomicU64::new(0);

#[allow(clippy::too_many_arguments)]
fn spawn_http(
    port: u16,
    web_dir: PathBuf,
    scene: Arc<Mutex<String>>,
    tx: mpsc::Sender<Action>,
    art: Option<Arc<Mutex<Art>>>,
    anim: Option<Arc<Anim>>,
) {
    let server = match Server::http(("0.0.0.0", port)) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("play: http server failed: {e}");
            return;
        }
    };
    let tile_cache: TileCache = Arc::new(Mutex::new(HashMap::new()));
    let anim_cache: AnimCache = Arc::new(Mutex::new(HashMap::new()));
    // Worker threads: a burst of tile/sprite PNG requests must never block the
    // frequent /scene.json polls (tiny_http's Server is shareable across threads).
    for _ in 0..6 {
        let server = server.clone();
        let web_dir = web_dir.clone();
        let scene = scene.clone();
        let tx = tx.clone();
        let art = art.clone();
        let anim = anim.clone();
        let tile_cache = tile_cache.clone();
        let anim_cache = anim_cache.clone();
        thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle_request(req, &web_dir, &scene, &tx, &art, &anim, &tile_cache, &anim_cache);
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_request(
    mut req: tiny_http::Request,
    web_dir: &Path,
    scene: &Arc<Mutex<String>>,
    tx: &mpsc::Sender<Action>,
    art: &Option<Arc<Mutex<Art>>>,
    anim: &Option<Arc<Anim>>,
    tile_cache: &TileCache,
    anim_cache: &AnimCache,
) {
    REQ_COUNT.fetch_add(1, Ordering::Relaxed);
    let url = req.url().split('?').next().unwrap_or("/").to_string();
    let is_post = *req.method() == Method::Post;

    if is_post && url == "/input" {
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        if let Some(action) = parse_command(&body) {
            let _ = tx.send(action);
        }
        let _ = req.respond(Response::from_string("ok"));
    } else if url == "/scene.json" {
        let body = scene.lock().unwrap().clone();
        let mut r = Response::from_string(body);
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else if let Some((is_static, g)) = parse_art_url(&url) {
        serve_art(art, tile_cache, is_static, g, req);
    } else if let Some((body, group, dir)) = parse_animinfo_url(&url) {
        let frames = anim.as_ref().and_then(|a| a.frame_count(body, group, dir)).unwrap_or(0);
        let mut r = Response::from_string(format!("{{\"frames\":{frames}}}"));
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else if let Some((body, group, dir, frame)) = parse_anim_url(&url) {
        serve_anim(anim, anim_cache, body, group, dir, frame, req);
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

/// Match `/anim/<body>/<group>/<dir>/<frame>.png` → (body, group, dir, frame).
fn parse_anim_url(url: &str) -> Option<(u16, u8, u8, u16)> {
    let mut p = url.strip_prefix("/anim/")?.split('/');
    let body = p.next()?.parse().ok()?;
    let group = p.next()?.parse().ok()?;
    let dir = p.next()?.parse().ok()?;
    let frame = p.next()?.strip_suffix(".png")?.parse().ok()?;
    Some((body, group, dir, frame))
}

/// Match `/animinfo/<body>/<group>/<dir>` → (body, group, dir).
fn parse_animinfo_url(url: &str) -> Option<(u16, u8, u8)> {
    let mut p = url.strip_prefix("/animinfo/")?.split('/');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

#[allow(clippy::too_many_arguments)]
fn serve_anim(
    anim: &Option<Arc<Anim>>,
    cache: &AnimCache,
    body: u16,
    group: u8,
    dir: u8,
    frame: u16,
    req: tiny_http::Request,
) {
    let key = (body, group, dir, frame);
    if let Some(b) = cache.lock().unwrap().get(&key).cloned() {
        return respond_png(req, b);
    }
    // Decode outside the cache lock so concurrent requests don't serialize.
    let bytes = anim
        .as_ref()
        .and_then(|a| a.frame(body, group, dir, frame as usize))
        .map(|i| i.to_png());
    match bytes {
        Some(b) => {
            cache.lock().unwrap().insert(key, b.clone());
            respond_png(req, b);
        }
        None => {
            let _ = req.respond(Response::from_string("no anim").with_status_code(404));
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
    cache: &TileCache,
    is_static: bool,
    g: u16,
    req: tiny_http::Request,
) {
    let key = (is_static, g);
    if let Some(b) = cache.lock().unwrap().get(&key).cloned() {
        return respond_png(req, b);
    }
    // Hold the Art lock only for the raw decode, not the PNG encode.
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
        .map(|i| i.to_png());
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

/// Parse a `cmd:arg` input line into an [`Action`].
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
        "say" => Some(Action::Say { text: arg.to_string() }),
        "use" => Some(Action::Use { serial: parse_serial(arg)? }),
        "attack" => Some(Action::Attack { serial: parse_serial(arg)? }),
        "pickup" => Some(Action::PickUp { serial: parse_serial(arg)?, amount: 1 }),
        "war" => Some(Action::WarMode { on: arg == "1" }),
        _ => None,
    }
}

fn parse_serial(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

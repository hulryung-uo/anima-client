//! The HTTP layer: routing, request limits, and static files.
//!
//! `handle_request` is the router; everything else here is the plumbing it needs
//! — body-size limits, the same-origin check that makes every state-changing
//! route safe to expose on loopback, and the SSE fan-out the sound stream uses.

use super::*;

pub(super) const MAX_POST_BODY_BYTES: usize = 16 * 1024;

/// Startup args for [`spawn_http`] (grouped to dodge the arg-count lint).
pub(super) struct SpawnHttp {
    pub(super) web_dir: Option<PathBuf>,
    pub(super) scene: Arc<Mutex<String>>,
    pub(super) tx: mpsc::Sender<Option<Action>>,
    pub(super) login: mpsc::Sender<LoginAttempt>,
    pub(super) character: mpsc::Sender<CharacterDecision>,
    pub(super) art: Option<Arc<Mutex<Art>>>,
    pub(super) anim: Option<Arc<Anim>>,
    pub(super) gumps: Option<Arc<Gumps>>,
    pub(super) hues: Option<Arc<Hues>>,
    pub(super) tiledata: Option<Arc<TileData>>,
    pub(super) cliloc: Option<Arc<Cliloc>>,
    pub(super) lights: Option<Arc<Lights>>,
    pub(super) texmaps: Option<Arc<Texmaps>>,
    pub(super) worldmap: Arc<Mutex<Option<Vec<u8>>>>,
    pub(super) sounds: Option<Arc<Sounds>>,
    pub(super) music: Arc<HashMap<u16, PathBuf>>,
    pub(super) sse_hub: SseHub,
    pub(super) pois: Arc<String>,
    pub(super) guard_rects: Arc<Vec<GuardRect>>,
    pub(super) house_catalog: Arc<HouseCatalogCache>,
    pub(super) facet: Arc<AtomicU8>,
    /// Refuse everything that would touch the world/session (spectator mode).
    pub(super) read_only: bool,
    /// Epoch-millis of the last `/scene.json` fetch.
    pub(super) watch: Arc<AtomicU64>,
}

/// Spawn the worker-thread pool serving `server` (already bound by [`bind`]).
pub(super) fn spawn_http(server: Arc<Server>, args: SpawnHttp) {
    let SpawnHttp {
        web_dir,
        scene,
        tx,
        login,
        character,
        art,
        anim,
        gumps,
        hues,
        tiledata,
        cliloc,
        lights,
        texmaps,
        worldmap,
        sounds,
        music,
        sse_hub,
        pois,
        guard_rects,
        house_catalog,
        facet,
        read_only,
        watch,
    } = args;
    let tile_cache: TileCache = Arc::new(Mutex::new(ByteCache::new(TILE_CACHE_BYTES)));
    let anim_cache: AnimCache = Arc::new(Mutex::new(ByteCache::new(ANIM_CACHE_BYTES)));
    let texmap_cache: TexmapCache = Arc::new(Mutex::new(ByteCache::new(TEXMAP_CACHE_BYTES)));
    let gump_cache: GumpCache = Arc::new(Mutex::new(ByteCache::new(GUMP_CACHE_BYTES)));
    // Worker threads: a burst of tile/sprite PNG requests must never block the
    // frequent /scene.json polls (tiny_http's Server is shareable across threads).
    for _ in 0..6 {
        let server = server.clone();
        let web_dir = web_dir.clone();
        let scene = scene.clone();
        let tx = tx.clone();
        let login = login.clone();
        let character = character.clone();
        let art = art.clone();
        let anim = anim.clone();
        let gumps = gumps.clone();
        let hues = hues.clone();
        let tiledata = tiledata.clone();
        let cliloc = cliloc.clone();
        let lights = lights.clone();
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
        let house_catalog = house_catalog.clone();
        let watch = watch.clone();
        let facet = facet.clone();
        thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle_request(Ctx {
                    req,
                    web_dir: &web_dir,
                    scene: &scene,
                    tx: &tx,
                    login: &login,
                    character: &character,
                    art: &art,
                    anim: &anim,
                    gumps: &gumps,
                    hues: &hues,
                    tiledata: &tiledata,
                    cliloc: &cliloc,
                    lights: &lights,
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
                    house_catalog: &house_catalog,
                    facet: &facet,
                    read_only,
                    watch: &watch,
                });
            }
        });
    }
}

/// Everything a request handler needs (groups args to dodge the arg-count lint).
pub(super) struct Ctx<'a> {
    pub(super) req: tiny_http::Request,
    pub(super) web_dir: &'a Option<PathBuf>,
    pub(super) scene: &'a Arc<Mutex<String>>,
    pub(super) tx: &'a mpsc::Sender<Option<Action>>,
    pub(super) login: &'a mpsc::Sender<LoginAttempt>,
    pub(super) character: &'a mpsc::Sender<CharacterDecision>,
    pub(super) art: &'a Option<Arc<Mutex<Art>>>,
    pub(super) anim: &'a Option<Arc<Anim>>,
    pub(super) gumps: &'a Option<Arc<Gumps>>,
    pub(super) hues: &'a Option<Arc<Hues>>,
    pub(super) tiledata: &'a Option<Arc<TileData>>,
    pub(super) cliloc: &'a Option<Arc<Cliloc>>,
    pub(super) lights: &'a Option<Arc<Lights>>,
    pub(super) texmaps: &'a Option<Arc<Texmaps>>,
    pub(super) tile_cache: &'a TileCache,
    pub(super) anim_cache: &'a AnimCache,
    pub(super) texmap_cache: &'a TexmapCache,
    pub(super) gump_cache: &'a GumpCache,
    pub(super) worldmap: &'a Arc<Mutex<Option<Vec<u8>>>>,
    pub(super) sounds: &'a Option<Arc<Sounds>>,
    pub(super) music: &'a Arc<HashMap<u16, PathBuf>>,
    pub(super) sse_hub: &'a SseHub,
    pub(super) pois: &'a Arc<String>,
    pub(super) guard_rects: &'a Arc<Vec<GuardRect>>,
    pub(super) house_catalog: &'a Arc<HouseCatalogCache>,
    pub(super) facet: &'a Arc<AtomicU8>,
    pub(super) read_only: bool,
    pub(super) watch: &'a Arc<AtomicU64>,
}

pub(super) fn handle_request(ctx: Ctx) {
    REQ_COUNT.fetch_add(1, Ordering::Relaxed);
    let Ctx {
        mut req,
        web_dir,
        scene,
        read_only,
        watch,
        tx,
        login,
        character,
        art,
        anim,
        gumps,
        hues,
        tiledata,
        cliloc,
        lights,
        texmaps,
        tile_cache,
        anim_cache,
        texmap_cache,
        gump_cache,
        worldmap,
        sounds,
        music,
        sse_hub,
        pois,
        guard_rects,
        house_catalog,
        facet,
    } = ctx;
    let raw_url = req.url().to_string();
    // Parse the optional `?hue=<n>` query before stripping it. 0 = no hue.
    let hue = parse_hue_query(&raw_url);
    let url = raw_url.split('?').next().unwrap_or("/").to_string();
    let is_post = *req.method() == Method::Post;

    // CSRF guard: every state-changing route here is a POST (`/input`, `/login`,
    // `/character`, `/log`), and with the `play` bin's well-known port a malicious page loaded
    // in any tab could otherwise drive the session with no preflight (simple
    // requests aren't subject to CORS). A browser always sends `Origin` on a
    // cross-origin request and can't be told not to, so reject when it disagrees
    // with `Host`. No `Origin` header (curl/scripts/same-origin form posts) is
    // let through unchanged — this only blocks cross-origin *browser* requests.
    if is_post && !origin_allowed(header_value(&req, "Origin"), header_value(&req, "Host")) {
        let _ = req
            .respond(Response::from_string("cross-origin request rejected").with_status_code(403));
        return;
    }

    if is_post && url == "/log" {
        // Diagnostic trace from the browser: print verbatim so client + server
        // events interleave in one log (only when ANIMA_DEBUG is set).
        let body = match read_request_body(&mut req) {
            Ok(body) => body,
            Err((status, message)) => {
                let _ = req.respond(Response::from_string(message).with_status_code(status));
                return;
            }
        };
        if std::env::var("ANIMA_DEBUG").is_ok() {
            eprintln!("[cli] {}", body.trim());
        }
        let _ = req.respond(Response::from_string("ok"));
    } else if read_only && is_post && matches!(url.as_str(), "/input" | "/login" | "/character") {
        // Spectator mode. Refused here, before the body is even read, so a request can
        // never reach the action/login channels — the guarantee is structural, not a
        // promise made by the renderer's UI.
        let _ = req.respond(
            Response::from_string("read-only monitor: input is disabled").with_status_code(403),
        );
    } else if is_post && url == "/input" {
        let body = match read_request_body(&mut req) {
            Ok(body) => body,
            Err((status, message)) => {
                let _ = req.respond(Response::from_string(message).with_status_code(status));
                return;
            }
        };
        if body.trim() == "stop" {
            let _ = tx.send(None); // key released → stop pacing now
        } else if let Some(action) = parse_house_design_command(&body) {
            let _ = tx.send(Some(action));
        } else if let Some(action) = parse_command(&body) {
            let _ = tx.send(Some(action));
        }
        let _ = req.respond(Response::from_string("ok"));
    } else if is_post && url == "/login" {
        // The browser sends JSON so an optional character-creation request can
        // accompany the credentials. Colon-separated legacy requests remain valid.
        let body = match read_request_body(&mut req) {
            Ok(body) => body,
            Err((status, message)) => {
                let _ = req.respond(Response::from_string(message).with_status_code(status));
                return;
            }
        };
        match parse_login_attempt(&body) {
            Ok(attempt) => {
                let mut current_scene = scene.lock().unwrap();
                if !login_attempt_expected(&current_scene) {
                    drop(current_scene);
                    let _ = req.respond(
                        Response::from_string("login is not expected while a session is active")
                            .with_status_code(409),
                    );
                    return;
                }
                *current_scene = serde_json::json!({
                    "auth": "connecting",
                    "msg": "Connecting…",
                })
                .to_string();
                drop(current_scene);
                if login.send(attempt).is_ok() {
                    let _ = req.respond(Response::from_string("ok"));
                } else {
                    *scene.lock().unwrap() = serde_json::json!({
                        "auth": "error",
                        "msg": "login service is unavailable",
                    })
                    .to_string();
                    let _ = req.respond(
                        Response::from_string("login service is unavailable").with_status_code(409),
                    );
                }
            }
            Err(message) => {
                let _ = req.respond(Response::from_string(message).with_status_code(400));
            }
        }
    } else if is_post && url == "/character" {
        let body = match read_request_body(&mut req) {
            Ok(body) => body,
            Err((status, message)) => {
                let _ = req.respond(Response::from_string(message).with_status_code(status));
                return;
            }
        };
        match parse_character_choice(&body) {
            Ok(decision) => {
                let progress = match &decision {
                    CharacterDecision::Choose(CharacterChoice::Play(_)) => "Entering world…",
                    CharacterDecision::Choose(CharacterChoice::Create(_)) => "Creating character…",
                    CharacterDecision::Choose(CharacterChoice::Delete(_)) => "Deleting character…",
                    CharacterDecision::Cancel => "Returning to account login…",
                };
                let mut current_scene = scene.lock().unwrap();
                let awaiting_choice = serde_json::from_str::<serde_json::Value>(&current_scene)
                    .ok()
                    .and_then(|value| value.get("auth")?.as_str().map(str::to_owned))
                    .is_some_and(|auth| auth == "characters");
                if !awaiting_choice {
                    drop(current_scene);
                    let _ = req.respond(
                        Response::from_string("character choice is not expected")
                            .with_status_code(409),
                    );
                    return;
                }
                *current_scene = serde_json::json!({
                    "auth": "connecting",
                    "msg": progress,
                })
                .to_string();
                drop(current_scene);
                match character.send(decision) {
                    Ok(()) => {
                        let _ = req.respond(Response::from_string("ok"));
                    }
                    Err(_) => {
                        let _ = req.respond(
                            Response::from_string("character chooser is unavailable")
                                .with_status_code(409),
                        );
                    }
                }
            }
            Err(message) => {
                let _ = req.respond(Response::from_string(message).with_status_code(400));
            }
        }
    } else if url == "/scene.json" {
        // Somebody is looking — let a session owner skip building frames nobody wants.
        watch.store(now_millis(), Ordering::Relaxed);
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
    } else if url == "/abilities.json" {
        // The combat book's catalogue: cliloc text for all 32 weapon moves plus
        // the tile names of the weapon graphics the caller listed. Static per
        // (data files, query), so it caches like /pois.json.
        let body = abilities_json(
            cliloc.as_deref(),
            tiledata.as_deref(),
            &parse_graphics_query(&raw_url),
        );
        // No Cache-Control on purpose, unlike /pois.json: this answer is
        // assembled from the server's cliloc and tiledata, and the renderer
        // already fetches it once per page load, so an hour of browser cache
        // buys one request and hides a data-file (or server) change behind a
        // stale copy. Found the hard way — a restarted server kept serving the
        // previous binary's answer to the same URL.
        let mut r = Response::from_string(body);
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else if url == "/staticfilters.json" {
        // ClassicUO's StaticFilters tables, resolved against this install's
        // tiledata (the tree/vegetation split depends on impassability). Static
        // per data files; the renderer fetches it once.
        let mut r = Response::from_string(static_filters_json(tiledata.as_deref()));
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else if url == "/housecatalog" {
        // Custom-house building catalog (walls/floors/doors/misc/stairs/roofs/
        // teleporters). Static per-process data, parsed once on the FIRST
        // request and cached from then on — see `HouseCatalogCache`'s doc.
        let mut r = Response::from_string((*house_catalog.get()).clone());
        r.add_header(ctype("application/json"));
        r.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"max-age=3600"[..]).unwrap());
        let _ = req.respond(r);
    } else if let Some(id) = parse_sound_url(&url) {
        serve_sound(sounds, id, req);
    } else if let Some(id) = parse_music_url(&url) {
        serve_music(music, id, req);
    } else if let Some((is_static, g)) = parse_art_url(&url) {
        serve_art(art, hues, tile_cache, is_static, g, hue, req);
    } else if let Some(id) = parse_light_url(&url) {
        serve_light(lights, id, req);
    } else if let Some(id) = parse_texmap_url(&url) {
        serve_texmap(texmaps, texmap_cache, id, req);
    } else if let Some((body, group, dir)) = parse_animinfo_url(&url) {
        // Per-frame draw-centers let the renderer position each part (body, worn
        // equipment, rider on mount) correctly instead of foot-anchoring them all.
        let centers = anim
            .as_ref()
            .and_then(|a| a.frame_centers(body, group, dir))
            .unwrap_or_default();
        let frames = centers.len();
        let c = centers
            .iter()
            .map(|(cx, cy)| format!("[{cx},{cy}]"))
            .collect::<Vec<_>>()
            .join(",");
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
    } else if url == "/hues/dyed.json" {
        // One compact palette fetch for the 0x95 picker. Loading 200/1000
        // individual `/hue/<id>.json` swatches would needlessly fan out HTTP.
        let mut r = Response::from_string(dyed_palette_json(hues.as_deref()));
        r.add_header(ctype("application/json"));
        r.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"max-age=3600"[..]).unwrap());
        let _ = req.respond(r);
    } else if let Some(hid) = url
        .strip_prefix("/hue/")
        .and_then(|s| s.strip_suffix(".json"))
        .and_then(|s| s.parse::<u16>().ok())
    {
        // Resolve a hue id → a representative swatch colour (mid-bright ramp), so the
        // paperdoll can show the dye colour of hair/beard/clothing on hover.
        let c = hues
            .as_ref()
            .map(|h| h.color(hid, 24))
            .unwrap_or([0, 0, 0, 0]);
        let mut r = Response::from_string(format!(
            "{{\"rgb\":\"#{:02x}{:02x}{:02x}\"}}",
            c[0], c[1], c[2]
        ));
        r.add_header(ctype("application/json"));
        let _ = req.respond(r);
    } else {
        serve_static(web_dir, &url, req);
    }
}

pub(super) fn read_request_body(
    req: &mut tiny_http::Request,
) -> Result<String, (u16, &'static str)> {
    if req
        .body_length()
        .is_some_and(|length| length > MAX_POST_BODY_BYTES)
    {
        return Err((413, "request body too large"));
    }
    match read_text_limited(req.as_reader(), MAX_POST_BODY_BYTES) {
        Ok(Some(body)) => Ok(body),
        Ok(None) => Err((413, "request body too large")),
        Err(_) => Err((400, "invalid request body")),
    }
}

pub(super) fn read_text_limited(
    reader: &mut dyn io::Read,
    max_bytes: usize,
) -> io::Result<Option<String>> {
    let mut bytes = Vec::with_capacity(max_bytes.min(1024));
    reader.take(max_bytes as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Case-insensitively look up a request header's value.
pub(super) fn header_value<'a>(req: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
    req.headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

/// CSRF guard: is a POST from this `Origin` (if any) allowed against this
/// `Host`? A missing `Origin` (curl, scripts, same-origin form posts) is
/// always allowed — only a *present-but-mismatched* `Origin` is rejected, so
/// this blocks cross-origin browser requests without affecting anything else.
/// Pure and unit-tested (`play_server` otherwise has none — see FIX 4).
pub(super) fn origin_allowed(origin: Option<&str>, host: Option<&str>) -> bool {
    let (Some(origin), Some(host)) = (origin, host) else {
        return true;
    };
    // `Origin` is `<scheme>://<host>[:<port>]`; strip the scheme to compare
    // against `Host`'s `<host>[:<port>]`.
    let origin_host = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    origin_host.eq_ignore_ascii_case(host)
}

/// Serve a `web/` static asset. A configured `web_dir` on disk wins when it has
/// the file; otherwise (or with `web_dir: None`) fall back to the copy embedded
/// in the binary at compile time ([`EMBEDDED_WEB`]) — this is what lets
/// `anima-desktop` serve the renderer with no `web/` directory on disk at all.
pub(super) fn serve_static(web_dir: &Option<PathBuf>, url: &str, req: tiny_http::Request) {
    let rel = if url == "/" {
        "index.html"
    } else {
        url.trim_start_matches('/')
    };
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
            r.add_header(
                Header::from_bytes(&b"Cache-Control"[..], &b"no-store, must-revalidate"[..])
                    .unwrap(),
            );
            let _ = req.respond(r);
        }
        None => {
            let _ = req.respond(Response::from_string("404").with_status_code(404));
        }
    }
}

pub(super) fn ctype(v: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], v.as_bytes()).unwrap()
}

/// Send a raw SSE frame to every connected client; drop any whose receiver is gone.
pub(super) fn sse_broadcast(hub: &SseHub, frame: &[u8]) {
    let mut g = hub.lock().unwrap();
    g.retain(|s| s.send(frame.to_vec()).is_ok());
}

pub(super) fn content_type(path: &str) -> &'static str {
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
        assert!(origin_allowed(
            Some("http://127.0.0.1:8090"),
            Some("127.0.0.1:8090")
        ));
    }

    #[test]
    fn scheme_is_ignored() {
        assert!(origin_allowed(
            Some("https://127.0.0.1:8090"),
            Some("127.0.0.1:8090")
        ));
    }

    #[test]
    fn mismatched_origin_is_rejected() {
        assert!(!origin_allowed(
            Some("http://evil.example:1234"),
            Some("127.0.0.1:8090")
        ));
    }

    #[test]
    fn no_host_header_is_allowed() {
        // Malformed request with no Host at all — nothing to compare against;
        // not this guard's job to reject it.
        assert!(origin_allowed(Some("http://evil.example"), None));
    }
}

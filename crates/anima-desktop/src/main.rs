//! Standalone desktop shell (Tauri v2): runs the `anima-net` play server
//! in-process (direct TCP to the UO server, no relay) on a stable loopback
//! port, then opens a native webview at that URL. The web renderer
//! (`web/`, embedded — see `anima_net::play_server`) needs no changes: it
//! already talks same-origin (relative `fetch`/`EventSource`) to whatever
//! host served the page.
//!
//! No bundler / npm step: the "frontend" is the play server's embedded
//! `web/` copy, so `frontendDist` in `tauri.conf.json` just points at an
//! empty placeholder directory that's never actually served.

use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};

use anima_net::play_server::{self, PlayConfig};
use anima_net::uo_dir;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// Persisted at `<app_config_dir>/config.json` so a manually-picked data dir
/// (see [`resolve_data_dir`]) and the webview's origin (see
/// [`choose_http_port`]) survive across runs.
#[derive(Serialize, Deserialize)]
struct DesktopConfig {
    data_dir: PathBuf,
    /// The loopback port last served to the webview. `serde(default)` keeps
    /// config.json files written before this field existed loadable.
    #[serde(default)]
    http_port: Option<u16>,
}

/// Ports tried, in order, when nothing usable is remembered. Deliberately a
/// fixed low range rather than an OS-assigned one: the webview's origin —
/// and therefore every `localStorage`-backed preference the web layer keeps
/// (`anima.settings`, `anima.macros`, the HUD positions, …) — is keyed by
/// port, so an ephemeral port meant a brand-new, empty store every launch.
/// A fixed range is also *stable*: the OS ephemeral range (49152+ on macOS
/// and Windows) is exactly where outgoing sockets get their source ports, so
/// a remembered ephemeral port is likely to be stolen between runs.
/// 8190 rather than 8090 so a dev copy of the `play` bin (which defaults to
/// 8090) and the shipped app don't fight over one port.
const PORT_RANGE: std::ops::RangeInclusive<u16> = 8190..=8199;

/// A remembered port below 1024 is privileged (we could never have bound it,
/// so we never wrote it) and 0 means "OS-assigned" — treat either as if
/// nothing was remembered rather than trusting a hand-edited config.
fn usable_remembered(port: Option<u16>) -> Option<u16> {
    port.filter(|p| *p >= 1024)
}

/// Pick the HTTP port to serve the renderer on: the remembered one if it's
/// still free, else the first free port in [`PORT_RANGE`], else `None` for
/// "let the OS assign one" (settings won't persist for that run — the caller
/// logs it). `free` is the bind probe, injected so this is unit-testable.
fn choose_http_port(remembered: Option<u16>, mut free: impl FnMut(u16) -> bool) -> Option<u16> {
    let remembered = usable_remembered(remembered);
    remembered
        .into_iter()
        .chain(PORT_RANGE.filter(|p| Some(*p) != remembered))
        .find(|p| free(*p))
}

/// Can we bind `port` on loopback right now? Inherently racy (the listener is
/// closed again immediately), so the caller must still handle a failing bind —
/// but std sets `SO_REUSEADDR` on non-Windows exactly like `tiny_http`'s own
/// listener, so a `TIME_WAIT` leftover from our previous run doesn't make an
/// otherwise-free port look taken.
fn port_is_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// Cheap sanity check that `dir` looks like an unpacked UO client install
/// (not necessarily complete — `anima-assets` opens each file independently
/// and logs "not loaded" for anything missing). Shares the validation the
/// `play` bin uses ([`anima_net::uo_dir::looks_like_uo_data`]).
fn looks_like_uo_data(dir: &Path) -> bool {
    uo_dir::looks_like_uo_data(dir)
}

/// Matches the `play` bin's CLI default (`$HOME/dev/uo/uo-resource`) so a
/// dev machine already set up for `cargo run -p anima-net --bin play` needs
/// no extra configuration for the desktop shell either.
fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!("{home}/dev/uo/uo-resource"))
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("config.json"))
}

fn load_config(app: &AppHandle) -> Option<DesktopConfig> {
    let text = std::fs::read_to_string(config_path(app)?).ok()?;
    serde_json::from_str::<DesktopConfig>(&text).ok()
}

fn save_config(app: &AppHandle, cfg: &DesktopConfig) {
    let Some(path) = config_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path, json);
    }
}

/// Read-modify-write so the other fields survive: blindly rewriting the file
/// with only `data_dir` would drop the remembered port, moving the webview's
/// origin and wiping every stored preference.
fn persist_data_dir(app: &AppHandle, dir: &Path) {
    let mut cfg = load_config(app).unwrap_or(DesktopConfig {
        data_dir: dir.to_path_buf(),
        http_port: None,
    });
    cfg.data_dir = dir.to_path_buf();
    save_config(app, &cfg);
}

fn persist_http_port(app: &AppHandle, data_dir: &Path, port: u16) {
    let mut cfg = load_config(app).unwrap_or(DesktopConfig {
        data_dir: data_dir.to_path_buf(),
        http_port: None,
    });
    // Re-read decides, not the value we loaded at startup: on the very first run
    // after upgrade two copies both start with no remembered port, and whichever
    // finishes second would otherwise clobber the first one's claim — stranding
    // every preference the user had just set under an origin nothing loads again.
    // Re-reading here narrows that to the window between this load and the write;
    // it is not atomic, but the alternative is a lock file for a case that needs
    // two copies started within the same second of a first launch.
    if let Some(claimed) = cfg.http_port {
        if claimed != port {
            eprintln!(
                "anima-desktop: another copy already claimed port {claimed}; leaving it \
                 and serving this session from {port} (its preferences are separate)"
            );
            return;
        }
    }
    cfg.http_port = Some(port);
    save_config(app, &cfg);
}

/// Resolve the UO client data directory: a previously-persisted pick, else
/// the dev-default path, validated by [`looks_like_uo_data`]. If invalid,
/// show the native folder picker and persist a valid pick. A cancelled
/// picker is not fatal — the play server already degrades gracefully with
/// assets logged as "not loaded" (`anima_net::play_server::bind`).
///
/// Must run off the main thread: `blocking_pick_folder` docs are explicit
/// that it deadlocks if called from it (the caller is our own spawned
/// thread — see `main`).
fn resolve_data_dir(app: &AppHandle) -> PathBuf {
    let candidate = load_config(app)
        .map(|c| c.data_dir)
        .unwrap_or_else(default_data_dir);
    if looks_like_uo_data(&candidate) {
        return candidate;
    }
    // Before bothering the user with a folder picker, search the known install
    // locations (dev path, /Applications, a configured ClassicUO, …). Persist a
    // hit so the picker never appears again on this machine.
    if let Some(found) = uo_dir::detect_uo_dir() {
        println!(
            "anima-desktop: auto-detected UO data at {}",
            found.display()
        );
        persist_data_dir(app, &found);
        return found;
    }
    println!(
        "anima-desktop: no UO client data at {} and none auto-detected; asking the user",
        candidate.display()
    );
    let picked = app
        .dialog()
        .file()
        .set_title(
            "Locate your Ultima Online client files (folder containing anim.mul / tiledata.mul)",
        )
        .blocking_pick_folder()
        .and_then(|f| f.into_path().ok());
    match picked {
        Some(dir) => {
            if !looks_like_uo_data(&dir) {
                eprintln!(
                    "anima-desktop: {} doesn't look like a UO data dir either; using it anyway",
                    dir.display()
                );
            }
            persist_data_dir(app, &dir);
            dir
        }
        None => {
            eprintln!(
                "anima-desktop: folder picker cancelled; continuing with {} (assets will show as not loaded)",
                candidate.display()
            );
            candidate
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            // Everything below is blocking (folder picker, TCP+HTTP bind, the
            // game loop) and must not run on the main thread, or it'd freeze
            // the (not-yet-created) window and deadlock the folder picker.
            std::thread::spawn(move || {
                let data_dir = resolve_data_dir(&app_handle);

                // Standalone default: the served login page collects
                // server/account (no baked-in credentials); web_dir None = the
                // copy embedded in anima-net at compile time (no `web/`
                // directory exists outside the repo).
                let make_cfg = |http_port: u16| PlayConfig {
                    host: String::new(),
                    port: 0,
                    user: String::new(),
                    pass: String::new(),
                    shard: 0, // the login page carries its own shard choice
                    http_port,
                    web_dir: None,
                    data_dir: data_dir.clone(),
                    login_page: true,
                    // Loopback only, unconditionally — unlike the `play` bin's
                    // `ANIMA_BIND` escape hatch (see `anima_net::play_server::PlayConfig`),
                    // the desktop shell must never honor an env var that could
                    // expose this process to the network.
                    bind_addr: "127.0.0.1".to_string(),
                    // The desktop shell drives its own session — full input.
                    read_only: false,
                };

                // Serve from the same port as last run whenever we can: the
                // renderer's preferences live in localStorage, which is keyed
                // by origin (port included). Scanning a small fixed range keeps
                // the original "multiple copies never collide" property — a
                // second copy just lands on the next port (with its own store)
                // instead of failing to start.
                let remembered = load_config(&app_handle).and_then(|c| c.http_port);
                let chosen = choose_http_port(remembered, port_is_free);
                if let Some(want) = usable_remembered(remembered) {
                    if chosen != Some(want) {
                        eprintln!(
                            "anima-desktop: port {want} is in use (another copy of Anima?); \
                             falling back to {} — settings saved under the old port stay there \
                             and come back once {want} is free again",
                            chosen.map_or("an OS-assigned port".to_string(), |p| p.to_string())
                        );
                    }
                } else if chosen.is_none() {
                    eprintln!(
                        "anima-desktop: every port in {}..={} is in use; using an OS-assigned one \
                         — settings will not persist past this run",
                        PORT_RANGE.start(),
                        PORT_RANGE.end()
                    );
                }

                // `port_is_free` closed its probe listener before we got here, so
                // another process can still win the race; `play_server::bind` only
                // fails on the HTTP bind, so retry once with an OS-assigned port
                // rather than refusing to start over a lost race.
                let server = match chosen {
                    Some(p) => play_server::bind(make_cfg(p)).or_else(|e| {
                        eprintln!(
                            "anima-desktop: port {p} was taken after all ({e}); \
                             retrying with an OS-assigned port"
                        );
                        play_server::bind(make_cfg(0))
                    }),
                    None => play_server::bind(make_cfg(0)),
                };
                let server = match server {
                    Ok(s) => s,
                    Err(e) => {
                        // No window exists yet here — without this dialog the app
                        // would keep running as an invisible dock zombie (FIX 1b):
                        // stderr goes nowhere a Finder user will ever see it.
                        eprintln!("anima-desktop: play server failed to bind: {e}");
                        fatal(&app_handle, &format!("Anima couldn't start: {e}"));
                        return;
                    }
                };
                // The port actually bound, which is what the webview loads and
                // what localStorage is keyed by (`chosen` may have lost the race).
                let port = server.port();
                println!("anima-desktop: play server bound on 127.0.0.1:{port}");
                // Claim an origin only if we don't have one yet: overwriting a
                // remembered port with a fallback would hand the user's stored
                // preferences to whichever copy launched second.
                if usable_remembered(remembered).is_none() && PORT_RANGE.contains(&port) {
                    persist_http_port(&app_handle, &data_dir, port);
                }

                let handle_for_window = app_handle.clone();
                if let Err(e) = app_handle.run_on_main_thread(move || {
                    let url = format!("http://127.0.0.1:{port}/");
                    let build = WebviewWindowBuilder::new(
                        &handle_for_window,
                        "main",
                        WebviewUrl::External(
                            url.parse()
                                .expect("http://127.0.0.1:<port>/ is a valid URL"),
                        ),
                    )
                    .title("Anima")
                    .inner_size(1280.0, 800.0);
                    if let Err(e) = build.build() {
                        eprintln!("anima-desktop: failed to open window: {e}");
                    }
                }) {
                    eprintln!("anima-desktop: run_on_main_thread failed: {e}");
                }

                // Blocks for the app's lifetime (login + game loop). There's no
                // graceful shutdown plumbing for tiny_http today (intentionally
                // deferred, see crates/anima-desktop/README.md), so the only way
                // out of this call is the game connection ending — a clean
                // `Ok(())` (ServUO closed the socket) or an `Err` (read/write
                // failure). Either way the window is left showing a frozen last
                // scene with nothing driving it (FIX 1a): surface that natively
                // instead of leaving a silent zombie window.
                let result = server.run();
                let msg = match &result {
                    Ok(()) => "Connection to the game server ended.".to_string(),
                    Err(e) => format!("Connection to the game server ended: {e}"),
                };
                eprintln!("anima-desktop: play server exited: {msg}");
                fatal(&app_handle, &msg);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Show a native blocking error dialog, then terminate the app. Must be
/// called off the main thread — `blocking_show` docs are explicit that it
/// deadlocks there, exactly like `blocking_pick_folder` (see
/// `resolve_data_dir`) — which both callers here already satisfy (the
/// background thread spawned in `main`). `AppHandle::exit` triggers a clean
/// `RunEvent::ExitRequested`/`Exit` and falls back to `std::process::exit`
/// itself if that fails, so there's no zombie process left behind either way.
fn fatal(app: &AppHandle, message: &str) {
    app.dialog()
        .message(message)
        .title("Anima")
        .kind(MessageDialogKind::Error)
        .blocking_show();
    app.exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything except `taken` is free.
    fn probe(taken: &[u16]) -> impl FnMut(u16) -> bool + '_ {
        |p| !taken.contains(&p)
    }

    #[test]
    fn first_launch_takes_the_base_port() {
        assert_eq!(
            choose_http_port(None, probe(&[])),
            Some(*PORT_RANGE.start())
        );
    }

    #[test]
    fn a_remembered_port_wins_even_outside_the_range() {
        assert_eq!(choose_http_port(Some(8190), probe(&[])), Some(8190));
        assert_eq!(choose_http_port(Some(9999), probe(&[])), Some(9999));
    }

    #[test]
    fn a_taken_remembered_port_falls_through_to_the_range() {
        assert_eq!(choose_http_port(Some(9999), probe(&[9999])), Some(8190));
        // The remembered port is skipped when the scan reaches it again.
        assert_eq!(choose_http_port(Some(8190), probe(&[8190])), Some(8191));
    }

    #[test]
    fn a_full_range_means_os_assigned() {
        let all: Vec<u16> = PORT_RANGE.collect();
        assert_eq!(choose_http_port(None, probe(&all)), None);
        assert_eq!(choose_http_port(Some(8195), probe(&all)), None);
    }

    #[test]
    fn unusable_remembered_ports_are_ignored() {
        assert_eq!(usable_remembered(None), None);
        assert_eq!(usable_remembered(Some(0)), None);
        assert_eq!(usable_remembered(Some(80)), None);
        assert_eq!(usable_remembered(Some(8190)), Some(8190));
    }

    /// The probe has to agree with a real listener in both directions, or we'd
    /// either skip a free port (new origin, lost settings) or hand
    /// `play_server::bind` a port it can't have.
    #[test]
    fn the_probe_matches_a_real_listener() {
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!port_is_free(port));
        drop(held);
        assert!(port_is_free(port));
    }

    /// A config.json written before `http_port` existed must still load (and
    /// then read as "nothing remembered"), or the data-dir pick is lost too.
    #[test]
    fn config_without_http_port_still_loads() {
        let cfg: DesktopConfig = serde_json::from_str(r#"{"data_dir":"/uo"}"#).unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from("/uo"));
        assert_eq!(cfg.http_port, None);
    }
}

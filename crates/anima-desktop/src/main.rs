//! Standalone desktop shell (Tauri v2): runs the `anima-net` play server
//! in-process (direct TCP to the UO server, no relay) on an ephemeral
//! loopback port, then opens a native webview at that URL. The web renderer
//! (`web/`, embedded — see `anima_net::play_server`) needs no changes: it
//! already talks same-origin (relative `fetch`/`EventSource`) to whatever
//! host served the page.
//!
//! No bundler / npm step: the "frontend" is the play server's embedded
//! `web/` copy, so `frontendDist` in `tauri.conf.json` just points at an
//! empty placeholder directory that's never actually served.

use std::path::{Path, PathBuf};

use anima_net::play_server::{self, PlayConfig};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// Persisted at `<app_config_dir>/config.json` so a manually-picked data dir
/// survives across runs (see [`resolve_data_dir`]).
#[derive(Serialize, Deserialize)]
struct DesktopConfig {
    data_dir: PathBuf,
}

/// Cheap sanity check that `dir` looks like an unpacked UO client install
/// (not necessarily complete — `anima-assets` opens each file independently
/// and logs "not loaded" for anything missing).
fn looks_like_uo_data(dir: &Path) -> bool {
    dir.join("anim.mul").exists() || dir.join("tiledata.mul").exists()
}

/// Matches the `play` bin's CLI default (`$HOME/dev/uo/uo-resource`) so a
/// dev machine already set up for `cargo run -p anima-net --bin play` needs
/// no extra configuration for the desktop shell either.
fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!("{home}/dev/uo/uo-resource"))
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("config.json"))
}

fn load_persisted_data_dir(app: &AppHandle) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_path(app)?).ok()?;
    serde_json::from_str::<DesktopConfig>(&text).ok().map(|c| c.data_dir)
}

fn persist_data_dir(app: &AppHandle, dir: &Path) {
    let Some(path) = config_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&DesktopConfig { data_dir: dir.to_path_buf() }) {
        let _ = std::fs::write(path, json);
    }
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
    let candidate = load_persisted_data_dir(app).unwrap_or_else(default_data_dir);
    if looks_like_uo_data(&candidate) {
        return candidate;
    }
    println!("anima-desktop: no UO client data at {}; asking the user", candidate.display());
    let picked = app
        .dialog()
        .file()
        .set_title("Locate your Ultima Online client files (folder containing anim.mul / tiledata.mul)")
        .blocking_pick_folder()
        .and_then(|f| f.into_path().ok());
    match picked {
        Some(dir) => {
            if !looks_like_uo_data(&dir) {
                eprintln!("anima-desktop: {} doesn't look like a UO data dir either; using it anyway", dir.display());
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
                // server/account (no baked-in credentials); http_port 0 =
                // OS-assigned, so multiple copies (or a busy 8090) never
                // collide; web_dir None = the copy embedded in anima-net at
                // compile time (no `web/` directory exists outside the repo).
                let cfg = PlayConfig {
                    host: String::new(),
                    port: 0,
                    user: String::new(),
                    pass: String::new(),
                    http_port: 0,
                    web_dir: None,
                    data_dir,
                    login_page: true,
                    // Loopback only, unconditionally — unlike the `play` bin's
                    // `ANIMA_BIND` escape hatch (see `anima_net::play_server::PlayConfig`),
                    // the desktop shell must never honor an env var that could
                    // expose this process to the network.
                    bind_addr: "127.0.0.1".to_string(),
                };
                let server = match play_server::bind(cfg) {
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
                let port = server.port();
                println!("anima-desktop: play server bound on 127.0.0.1:{port}");

                let handle_for_window = app_handle.clone();
                if let Err(e) = app_handle.run_on_main_thread(move || {
                    let url = format!("http://127.0.0.1:{port}/");
                    let build = WebviewWindowBuilder::new(
                        &handle_for_window,
                        "main",
                        WebviewUrl::External(url.parse().expect("http://127.0.0.1:<port>/ is a valid URL")),
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

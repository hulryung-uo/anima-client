//! Standalone desktop shell (Tauri v2): runs the `anima-net` play server
//! in-process (direct TCP to the UO server, no relay) on a stable loopback
//! port, then opens a native webview at that URL. The web renderer
//! (`web/`, embedded — see `anima_net::play_server`) needs no changes: it
//! already talks same-origin (relative `fetch`/`EventSource`) to whatever
//! host served the page.
//!
//! No bundler / npm step. `frontend-dist` is a local setup window with a
//! file checklist; the game renderer stays in the play server's embedded web/.

mod config;
mod credentials;
mod downloads;
mod setup;
use anima_net::launcher::LauncherStore;
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;

use anima_net::play_server::{self, PlayConfig};
#[cfg(test)]
use config::DesktopConfig;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            setup::setup_status,
            setup::setup_check,
            setup::setup_choose,
            setup::setup_detect,
            setup::setup_recover,
            setup::setup_apply,
            setup::setup_close,
            setup::setup_restart,
        ])
        .menu(|app| {
            use tauri::menu::{Menu, MenuItem, Submenu};
            let menu = Menu::default(app)?;
            let files =
                MenuItem::with_id(app, "game-files", "Game files…", true, Some("CmdOrCtrl+,"))?;
            menu.append(&Submenu::with_items(
                app,
                "Anima settings",
                true,
                &[&files],
            )?)?;
            Ok(menu)
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "game-files" {
                let _ = setup::open(app);
            }
        })
        .setup(|app| {
            let app_handle = app.handle().clone();
            // Isolated desktop QA can use its own config/profile directory.
            // This never changes the renderer's loopback-only network binding.
            let folder = match std::env::var_os("ANIMA_DESKTOP_CONFIG_DIR") {
                Some(path) => PathBuf::from(path),
                None => app.path().app_config_dir()?,
            };
            app.manage(Arc::new(setup::SetupState::new(folder.join("config.json"))));
            setup::open(&app_handle)?;
            std::thread::spawn(move || setup::initialize(app_handle));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn launch(app_handle: AppHandle, data_dir: PathBuf) {
    std::thread::spawn(move || {
        let state = app_handle.state::<Arc<setup::SetupState>>().inner().clone();
        let launcher = state
            .config
            .path
            .parent()
            .ok_or_else(|| "Cannot locate the Anima profile folder.".to_string())
            .and_then(|dir| {
                LauncherStore::open(dir.join("launcher.json"), credentials::native_vault())
            });
        let launcher = match launcher {
            Ok(store) => Arc::new(store),
            Err(error) => {
                setup::failed(&app_handle, error);
                return;
            }
        };

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
        let remembered = match state.config.load() {
            Ok(cfg) => cfg.and_then(|c| c.http_port),
            Err(error) => {
                setup::failed(&app_handle, error);
                return;
            }
        };
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
            Some(p) => {
                play_server::bind_with_launcher(make_cfg(p), launcher.clone()).or_else(|e| {
                    eprintln!(
                        "anima-desktop: port {p} was taken after all ({e}); \
                             retrying with an OS-assigned port"
                    );
                    play_server::bind_with_launcher(make_cfg(0), launcher.clone())
                })
            }
            None => play_server::bind_with_launcher(make_cfg(0), launcher.clone()),
        };
        let server = match server {
            Ok(s) => s,
            Err(e) => {
                // No window exists yet here — without this dialog the app
                // would keep running as an invisible dock zombie (FIX 1b):
                // stderr goes nowhere a Finder user will ever see it.
                eprintln!("anima-desktop: play server failed to bind: {e}");
                setup::failed(&app_handle, format!("Anima couldn't start: {e}"));
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
            if let Err(error) = state.config.claim_port(port) {
                // The port is already bound. Surface persistence failure
                // and continue this run without falsely claiming a save.
                state.view.lock().unwrap().notice = error;
            }
        }

        let handle_for_window = app_handle.clone();
        let window_data_dir = data_dir.clone();
        if let Err(e) = app_handle.run_on_main_thread(move || {
            let url = format!("http://127.0.0.1:{port}/");
            let downloads_dir = handle_for_window.path().download_dir().ok();
            let build = WebviewWindowBuilder::new(
                &handle_for_window,
                "main",
                WebviewUrl::External(
                    url.parse()
                        .expect("http://127.0.0.1:<port>/ is a valid URL"),
                ),
            )
            .on_download(move |webview, event| {
                use tauri::webview::DownloadEvent;
                let Ok(current) = webview.url() else {
                    return false;
                };
                match event {
                    DownloadEvent::Requested { url, destination } => {
                        downloads::from_renderer(&url, &current, port)
                            && downloads_dir
                                .as_ref()
                                .is_some_and(|dir| downloads::settings_file(destination, dir))
                    }
                    DownloadEvent::Finished { url, success, .. } => {
                        if downloads::from_renderer(&url, &current, port) {
                            let _ = webview.eval(format!(
                                "if(typeof preferenceDownloadResult==='function')preferenceDownloadResult({success})"
                            ));
                        }
                        true
                    }
                    _ => false,
                }
            })
            .title("Anima")
            .inner_size(1280.0, 800.0);
            match build.build() {
                Ok(_) => {
                    setup::running(&handle_for_window, &window_data_dir);
                    if let Some(window) = handle_for_window.get_webview_window("setup") {
                        // Keep a persistence warning visible until acknowledged.
                        let state = handle_for_window.state::<Arc<setup::SetupState>>();
                        if state.snapshot().notice.is_empty() {
                            let _ = window.close();
                        }
                    }
                }
                Err(e) => {
                    let handle = handle_for_window.clone();
                    std::thread::spawn(move || {
                        fatal(&handle, &format!("Anima couldn't open its window: {e}"))
                    });
                }
            }
        }) {
            eprintln!("anima-desktop: run_on_main_thread failed: {e}");
            fatal(&app_handle, &format!("Anima couldn't open its window: {e}"));
            return;
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
}

/// Show a native blocking error dialog, then terminate the app. Must be
/// called off the main thread — `blocking_show` docs are explicit that it
/// deadlocks there, exactly like `blocking_pick_folder`. Callers dispatch to
/// a background thread. `AppHandle::exit` triggers a clean
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

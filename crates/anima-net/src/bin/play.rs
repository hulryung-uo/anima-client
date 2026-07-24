//! `play` — a human-controlled UO client served over HTTP.
//!
//! Holds one live [`Session`], serves the `web/` renderer + `/scene.json`, and
//! accepts `POST /input` commands (walk/say/use/attack/pickup/war) which it
//! executes on the live session. Open the page, use the keyboard, and your
//! character moves/talks on the real server.
//!
//! Thin CLI wrapper over [`anima_net::play_server`] (`bind` + `PlayServer::run`),
//! which is the reusable engine — also driven in-process, with an ephemeral
//! port and embedded web assets, by `anima-desktop`.
//!
//! Usage: `play [host] [port] [user] [pass] [http_port] [web_dir] [data_dir]`
//!
//! `ANIMA_BIND=<addr>` (env var) overrides the HTTP bind address, default
//! `127.0.0.1` (loopback only). Set `ANIMA_BIND=0.0.0.0` to allow viewing
//! from another machine on the LAN — this is a deliberate escape hatch for
//! this bin only (`anima-desktop` always binds loopback, ignoring the
//! environment); anyone on the LAN who can reach the port gets the same
//! unauthenticated control as `/input`/`/login`, so only do this on a
//! network you trust.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anima_net::play_server::{self, PlayConfig};
use anima_net::uo_dir;

fn main() {
    let mut a = std::env::args().skip(1);
    let host = a.next().unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(2594);
    let user = a.next().unwrap_or_else(|| "animaplay".into());
    let pass = a.next().unwrap_or_else(|| "animaplay".into());
    let http_port: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(8090);
    let web_dir = PathBuf::from(a.next().unwrap_or_else(|| "web".into()));
    // The UO client-data directory: an explicit 7th arg wins; otherwise we
    // remember a previous pick, auto-detect known install locations, or (last
    // resort, on a real terminal) ask. See `resolve_uo_dir`.
    let data_dir = match resolve_uo_dir(a.next()) {
        Some(d) => d,
        None => {
            eprintln!(
                "play: could not find your UO client-data directory (the folder with \
                 tiledata.mul). Pass it as the 7th argument, e.g.\n  \
                 play 127.0.0.1 2594 user pass 8090 web /path/to/uo"
            );
            std::process::exit(2);
        }
    };
    // With ANIMA_LOGIN set we serve the web login page and wait for the
    // browser to POST a server + account; otherwise we auto-login with the
    // CLI host/port/user/pass (backward compatible with scripts/agents).
    let login_page = std::env::var("ANIMA_LOGIN").is_ok();
    // See the module doc comment: loopback by default, LAN-viewable opt-in.
    let bind_addr = std::env::var("ANIMA_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());

    let cfg = PlayConfig {
        host,
        port,
        user,
        pass,
        http_port,
        web_dir: Some(web_dir),
        data_dir,
        login_page,
        bind_addr,
    };

    let server = match play_server::bind(cfg) {
        Ok(s) => s,
        // bind() already printed the reason ("play: http server failed: ...").
        Err(_) => std::process::exit(1),
    };
    if let Err(e) = server.run() {
        eprintln!("play: {e}");
        std::process::exit(1);
    }
}

/// Resolve the UO client-data directory, in priority order:
///   1. an explicit path (the CLI's 7th arg) — used as-is if valid, else it
///      warns and falls through to a search (a typo shouldn't silently render a
///      blank world),
///   2. a directory remembered from a previous run (`~/.config/anima-client/uo_dir`),
///   3. auto-detection of known install locations ([`uo_dir::detect_uo_dir`]),
///   4. asking on the terminal (only when stdin is a TTY).
///
/// Whatever is resolved via (3)/(4) is remembered so later runs don't re-ask.
/// Returns `None` only when nothing is found and we can't prompt (e.g. launched
/// headless with no data present) — the caller then exits with guidance.
fn resolve_uo_dir(explicit: Option<String>) -> Option<PathBuf> {
    if let Some(arg) = explicit {
        let dir = expand_tilde(&arg);
        if uo_dir::looks_like_uo_data(&dir) {
            return Some(dir);
        }
        eprintln!(
            "play: {} has no UO data (tiledata.mul); searching known locations…",
            dir.display()
        );
    } else if let Some(saved) = load_saved_dir() {
        // Already configured on a previous run — no need to ask again.
        return Some(saved);
    }

    if let Some(found) = uo_dir::detect_uo_dir() {
        eprintln!("play: using UO data at {}", found.display());
        save_dir(&found);
        return Some(found);
    }

    let chosen = prompt_for_dir()?;
    save_dir(&chosen);
    Some(chosen)
}

/// Where a resolved directory is remembered between runs.
fn saved_dir_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/anima-client/uo_dir"))
}

fn load_saved_dir() -> Option<PathBuf> {
    let raw = std::fs::read_to_string(saved_dir_path()?).ok()?;
    let dir = expand_tilde(raw.trim());
    uo_dir::looks_like_uo_data(&dir).then_some(dir)
}

fn save_dir(dir: &Path) {
    let Some(path) = saved_dir_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, dir.to_string_lossy().as_bytes());
}

/// Ask on the terminal, validating each entry. Only prompts when stdin is an
/// interactive TTY — a headless/background launch (an agent, a pipe, nohup)
/// returns `None` instead of blocking forever on a read.
fn prompt_for_dir() -> Option<PathBuf> {
    if !std::io::stdin().is_terminal() {
        return None;
    }
    eprintln!("play: couldn't auto-detect your UO client-data directory.");
    for _ in 0..3 {
        print!("play: enter the folder containing tiledata.mul (blank to abort): ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            return None; // EOF (Ctrl-D)
        }
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let dir = expand_tilde(line);
        if uo_dir::looks_like_uo_data(&dir) {
            return Some(dir);
        }
        eprintln!(
            "play: {} has no UO data there — try again, or Ctrl-C to quit.",
            dir.display()
        );
    }
    None
}

/// Expand a leading `~` / `~/…` to `$HOME` — UO paths are often pasted or saved
/// with a tilde. Anything else is returned verbatim.
fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with('/') {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(rest.trim_start_matches('/'));
            }
        }
    }
    PathBuf::from(p)
}

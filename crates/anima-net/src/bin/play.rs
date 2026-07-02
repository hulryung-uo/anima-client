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

use std::path::PathBuf;

use anima_net::play_server::{self, PlayConfig};

fn main() {
    let mut a = std::env::args().skip(1);
    let host = a.next().unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(2594);
    let user = a.next().unwrap_or_else(|| "animaplay".into());
    let pass = a.next().unwrap_or_else(|| "animaplay".into());
    let http_port: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(8090);
    let web_dir = PathBuf::from(a.next().unwrap_or_else(|| "web".into()));
    let home = std::env::var("HOME").unwrap_or_default();
    let data_dir = PathBuf::from(a.next().unwrap_or_else(|| format!("{home}/dev/uo/uo-resource")));
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

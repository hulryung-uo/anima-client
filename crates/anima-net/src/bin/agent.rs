//! `anima-agent` — the brain↔body IPC bridge.
//!
//! Connects to a UO server and logs in, then speaks **newline-delimited JSON**
//! (NDJSON) over stdin/stdout so an out-of-process brain (anima2, Python) can
//! drive this character. stderr carries human logs only.
//!
//! Usage: `anima-agent [host] [port] [username] [password] [data_dir]`
//! (defaults: 127.0.0.1 2594 animatest animatest — ServUO auto-creates accounts;
//! `data_dir` defaults to `$HOME/dev/uo/uo-resource`, like `play.rs`/`anima-agent`'s
//! `main.rs`.) `data_dir` is where the UO client files live — it's the
//! pathfinding terrain for `Action::WalkTo` (see `pump` below); a brain that
//! never sends `WalkTo` runs fine without it.
//!
//! Protocol — one JSON object per line:
//!   → `{"cmd":"observe"}`            ← `{"ok":true,"obs":{...}}`
//!   → `{"cmd":"act","action":{...}}` ← `{"ok":true}`
//!   → `{"cmd":"pump","ms":400}`      ← `{"ok":true,"applied":N}`
//!   → `{"cmd":"quit"}`               ← `{"ok":true,"bye":true}` then exit
//! On any error: `{"ok":false,"error":"..."}` (the loop keeps running).
//! On startup, emits one line: `{"event":"ready","player":{...}}`.
//!
//! `act`'s `WalkTo` only queues the route (see [`anima_net::Session::apply_action`]);
//! `pump` is what actually drives it, one step per call at its own cadence —
//! call `pump` on a steady tick (like `main.rs`'s per-tick loop) or the route
//! stalls between brain turns.

use std::io::{BufRead, Write};
use std::time::Duration;

use anima_assets::MapData;
use anima_core::agent::Action;
use anima_core::net::LoginConfig;
use anima_net::json::{action_from_json, observation_to_json};
use anima_net::{Endpoint, Session};
use serde_json::{json, Value};

fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "127.0.0.1".to_string());
    let port: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2594);
    let username = args.next().unwrap_or_else(|| "animatest".to_string());
    let password = args.next().unwrap_or_else(|| "animatest".to_string());
    let home = std::env::var("HOME").unwrap_or_default();
    let data_dir = args.next().unwrap_or_else(|| format!("{home}/dev/uo/uo-resource"));

    eprintln!("[anima-agent] connecting to {host}:{port} as {username} ...");
    let cfg = LoginConfig { username, password, ..Default::default() };
    let mut session = match Session::connect_and_login(&Endpoint::new(host, port), cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[anima-agent] login failed: {e}");
            std::process::exit(1);
        }
    };
    // `MapData` is the pathfinding terrain `pump` feeds `Session::advance_route`
    // (see `handle`) so a brain's `Action::WalkTo` actually walks — mirrors
    // `anima-agent`'s `main.rs`. Missing game data degrades gracefully: `WalkTo`
    // still queues a route (the contract stays honest — `act` doesn't lie about
    // it), it just never advances, so we log it loudly once here and again on
    // every `WalkTo` `act` while it's missing.
    let mut map = MapData::open(&data_dir).ok();
    eprintln!(
        "[anima-agent] map data {}",
        if map.is_some() {
            "loaded".to_string()
        } else {
            format!("not loaded at {data_dir} (WalkTo actions will be accepted but won't path)")
        }
    );
    // Drain the initial burst so the first observe is meaningful.
    let _ = session.observe(Duration::from_millis(500));

    let player = observation_to_json(&session.observation());
    emit(&json!({ "event": "ready", "player": player["player"] }));
    eprintln!("[anima-agent] ready — speaking NDJSON on stdout");

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        match handle(&mut session, map.as_mut(), &line) {
            Ok(Some(reply)) => emit(&reply),
            Ok(None) => {
                emit(&json!({ "ok": true, "bye": true }));
                break;
            }
            Err(e) => emit(&json!({ "ok": false, "error": e })),
        }
    }
}

/// Returns `Ok(Some(reply))` to answer, `Ok(None)` to quit, `Err(msg)` on failure.
/// `map` is the `WalkTo` pathfinding terrain (`None` if `data_dir` had no game
/// data — see `main`'s startup log).
fn handle(session: &mut Session, map: Option<&mut MapData>, line: &str) -> Result<Option<Value>, String> {
    let msg: Value = serde_json::from_str(line).map_err(|e| format!("bad json: {e}"))?;
    let cmd = msg.get("cmd").and_then(Value::as_str).ok_or("missing 'cmd'")?;
    match cmd {
        "observe" => {
            let obs = session.observation();
            Ok(Some(json!({ "ok": true, "obs": observation_to_json(&obs) })))
        }
        "act" => {
            let action = action_from_json(msg.get("action").ok_or("missing 'action'")?)?;
            if matches!(action, Action::WalkTo { .. }) && map.is_none() {
                eprintln!(
                    "[anima-agent] WalkTo queued but no map data loaded — it can't path (see startup log)"
                );
            }
            session.apply_action(&action).map_err(|e| e.to_string())?;
            Ok(Some(json!({ "ok": true })))
        }
        "pump" => {
            let ms = msg.get("ms").and_then(Value::as_u64).unwrap_or(400);
            let applied = session
                .observe(Duration::from_millis(ms))
                .map_err(|e| e.to_string())?;
            // Advance any active `Action::WalkTo` route by at most one step,
            // paced internally — a no-op most calls. Only possible with map
            // data; without it a queued route just sits idle (see `main`).
            if let Some(m) = map {
                if let Err(e) = session.advance_route(m) {
                    eprintln!("[anima-agent] route error: {e}");
                }
            }
            Ok(Some(json!({ "ok": true, "applied": applied })))
        }
        "quit" => Ok(None),
        other => Err(format!("unknown cmd: {other}")),
    }
}

fn emit(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

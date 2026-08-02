//! `anima-agent` — the brain↔body IPC bridge.
//!
//! Connects to a UO server and logs in, then speaks **newline-delimited JSON**
//! (NDJSON) over stdin/stdout so an out-of-process brain (anima2, Python) can
//! drive this character. stderr carries human logs only.
//!
//! Set `ANIMA_MONITOR_PORT` to also serve a READ-ONLY spectator view of this
//! character on that HTTP port (`0` = pick a free one; the chosen port is printed
//! to stderr as `[anima-agent] monitor on http://.../`). It renders the same web
//! client `play` serves, but refuses every input — the brain stays the only thing
//! driving this body. A spectator cannot be a second login of the same character:
//! ServUO's character-select disposes the previous session, which would kick the
//! agent off, so the bridge publishes frames from the session it already owns.
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
//!     (add `"terrain_radius":N` to widen/narrow the walkability window in
//!     `obs.terrain`, or `0` to omit it; it is only ever filled when
//!     `data_dir` gave us map files to read)
//!   → `{"cmd":"act","action":{...}}` ← `{"ok":true}`
//!   → `{"cmd":"pump","ms":400}`      ← `{"ok":true,"applied":N}`
//!   → `{"cmd":"quit"}`               ← `{"ok":true,"bye":true}` then exit
//! On any error: `{"ok":false,"error":"..."}` (the loop keeps running).
//! On startup, emits one line:
//! `{"event":"ready","schema_version":N,"player":{...}}`.
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
use anima_net::json::{action_from_json, observation_to_json, SCHEMA_VERSION};
use anima_net::play_server::{self, PlayConfig};
use anima_net::{Endpoint, Session};

/// Default half-width of the walkability window in `observe`'s `obs.terrain`.
/// 12 covers roughly what the renderer draws, so a brain sees the same ground a
/// human would; 625 tiles of it costs about a kilobyte on the wire (see
/// `terrain_json`'s packing).
const TERRAIN_RADIUS: u8 = 12;
use serde_json::{json, Value};

fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "127.0.0.1".to_string());
    let port: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2594);
    let username = args.next().unwrap_or_else(|| "animatest".to_string());
    let password = args.next().unwrap_or_else(|| "animatest".to_string());
    let home = std::env::var("HOME").unwrap_or_default();
    let data_dir = args
        .next()
        .unwrap_or_else(|| format!("{home}/dev/uo/uo-resource"));

    eprintln!("[anima-agent] connecting to {host}:{port} as {username} ...");
    let cfg = LoginConfig {
        username,
        password,
        ..Default::default()
    };
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
    // Optional read-only spectator view. Bound BEFORE the NDJSON loop starts so the
    // port is printed while a human is still reading stderr; frames are only built
    // when somebody is actually watching (see `Monitor::watching`), so an unwatched
    // monitor costs nothing per tick beyond one clock read.
    let mut monitor = match std::env::var("ANIMA_MONITOR_PORT") {
        Ok(v) => match v.parse::<u16>() {
            Ok(http_port) => match play_server::bind(PlayConfig {
                host: String::new(),
                port: 0,
                user: String::new(),
                pass: String::new(),
                shard: 0, // spectator only — this config never logs in
                http_port,
                web_dir: None, // the copy embedded in anima-net at compile time
                data_dir: data_dir.clone().into(),
                login_page: false,
                bind_addr: "127.0.0.1".to_string(),
                read_only: true,
            }) {
                Ok(server) => {
                    let m = server.into_monitor();
                    eprintln!(
                        "[anima-agent] monitor on http://127.0.0.1:{}/ (read-only)",
                        m.port()
                    );
                    Some(m)
                }
                // A monitor is a convenience; never take the brain down with it.
                Err(e) => {
                    eprintln!("[anima-agent] monitor failed to bind: {e}");
                    None
                }
            },
            Err(_) => {
                eprintln!("[anima-agent] ANIMA_MONITOR_PORT={v:?} is not a port; monitor off");
                None
            }
        },
        Err(_) => None,
    };

    // Drain the initial burst so the first observe is meaningful.
    let _ = session.observe(Duration::from_millis(500));

    let player = observation_to_json(&session.observation());
    emit(&ready_event(&player["player"]));
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
        let result = handle(&mut session, map.as_mut(), &line);
        // Refresh the spectator's view off the session the brain just advanced. Done
        // here, on the bridge's own thread, so the monitor never touches `Session`
        // concurrently with the brain and needs no lock around it.
        if let Some(m) = monitor.as_mut() {
            if m.watching() {
                m.publish(&mut session, map.as_mut());
            }
        }
        match result {
            Ok(Some(reply)) => emit(&reply),
            Ok(None) => {
                emit(&json!({ "ok": true, "bye": true }));
                break;
            }
            Err(e) => emit(&json!({ "ok": false, "error": e })),
        }
    }
}

fn ready_event(player: &Value) -> Value {
    json!({
        "event": "ready",
        "schema_version": SCHEMA_VERSION,
        "player": player,
    })
}

/// Returns `Ok(Some(reply))` to answer, `Ok(None)` to quit, `Err(msg)` on failure.
/// `map` is the `WalkTo` pathfinding terrain (`None` if `data_dir` had no game
/// data — see `main`'s startup log).
fn handle(
    session: &mut Session,
    map: Option<&mut MapData>,
    line: &str,
) -> Result<Option<Value>, String> {
    let msg: Value = serde_json::from_str(line).map_err(|e| format!("bad json: {e}"))?;
    let cmd = msg
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or("missing 'cmd'")?;
    match cmd {
        "observe" => {
            // With map data loaded, the observation carries local walkability
            // too — the brain can see walls/water/doors rather than only being
            // able to hand `WalkTo` a destination and hope. `radius` is the
            // caller's, since a combat brain polling every tick wants a
            // smaller window than a mapper; the default covers the screen.
            let radius = msg
                .get("terrain_radius")
                .and_then(Value::as_u64)
                .map_or(TERRAIN_RADIUS, |r| r.min(u64::from(u8::MAX)) as u8);
            let obs = match (map, radius) {
                (Some(map), 1..) => session.observation_with_terrain(map, radius),
                // No map files, or the brain explicitly asked for none.
                _ => session.observation(),
            };
            Ok(Some(
                json!({ "ok": true, "obs": observation_to_json(&obs) }),
            ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_event_advertises_contract_schema_version() {
        let player = json!({ "serial": 0x1234, "dead": false });
        let ready = ready_event(&player);

        assert_eq!(ready["event"], "ready");
        assert_eq!(ready["schema_version"], SCHEMA_VERSION);
        assert_eq!(ready["player"], player);
    }
}

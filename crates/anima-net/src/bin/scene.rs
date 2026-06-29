//! `scene` — AI-patrol world → scene JSON for the web renderer (non-interactive).
//!
//! Logs in, wanders, and writes a scene snapshot to a JSON file every ~500ms.
//! For the human-controlled version see the `play` bin.
//!
//! Usage: `scene [host] [port] [user] [pass] [out.json] [data_dir]`

use std::io::Write;
use std::time::Duration;

use anima_assets::{Art, MapData};
use anima_core::net::LoginConfig;
use anima_net::scene::build_scene;
use anima_net::{Endpoint, Session};

fn main() {
    let mut a = std::env::args().skip(1);
    let host = a.next().unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(2594);
    let user = a.next().unwrap_or_else(|| "animascene".into());
    let pass = a.next().unwrap_or_else(|| "animascene".into());
    let out = a.next().unwrap_or_else(|| "web/scene.json".into());
    let home = std::env::var("HOME").unwrap_or_default();
    let data_dir = a.next().unwrap_or_else(|| format!("{home}/dev/uo/uo-resource"));

    let mut map = MapData::open(&data_dir).ok();
    let mut art = Art::open(&data_dir).ok();

    let cfg = LoginConfig {
        username: user.clone(),
        password: pass,
        ..Default::default()
    };
    println!("scene: connecting to {host}:{port} as {user} ...");
    let mut s = match Session::connect_and_login(&Endpoint::new(host, port), cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("login failed: {e}");
            std::process::exit(1);
        }
    };
    println!("scene: logged in; writing {out} every ~500ms");

    let dirs = [6u8, 6, 0, 2, 2, 4];
    let mut di = 0usize;
    let mut journal: Vec<serde_json::Value> = Vec::new();
    let mut cursor = 0usize;

    loop {
        if s.observe(Duration::from_millis(500)).is_err() {
            eprintln!("scene: connection closed");
            break;
        }
        let _ = s.walk(dirs[di % dirs.len()], false);
        di += 1;

        let obs = s.world.observe(&mut cursor);
        for j in &obs.new_journal {
            journal.push(serde_json::json!({ "name": j.name, "text": j.text, "type": j.msg_type }));
        }
        while journal.len() > 12 {
            journal.remove(0);
        }

        let scene = build_scene(&mut s, map.as_mut(), art.as_mut(), None, None, &journal);
        write_atomic(&out, &scene);
    }
}

fn write_atomic(path: &str, contents: &str) {
    let tmp = format!("{path}.tmp");
    if let Ok(mut f) = std::fs::File::create(&tmp) {
        let _ = f.write_all(contents.as_bytes());
        let _ = std::fs::rename(&tmp, path);
    }
}

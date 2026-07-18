//! `anima-login` — connect to a UO server, log in, observe, and print perception.
//!
//! Usage: `anima-login [host] [port] [username] [password] [--delete-existing]`
//! Defaults: 127.0.0.1 2594 animatest animatest (ServUO auto-creates accounts).
//! `--delete-existing` is opt-in and off by default: it deletes the character
//! that would have been selected, once, before letting the normal
//! select-or-create logic run against the refreshed character list (see
//! `LoginConfig::delete_existing`).

use std::time::Duration;

use anima_core::net::LoginConfig;
use anima_net::{Endpoint, Session};

fn main() {
    // Split flags out FIRST so `--delete-existing` works in any position — pulled
    // positionally it would silently become the username and the flag would be
    // dropped (login as the literal account "--delete-existing", no warning).
    let (flags, positional): (Vec<String>, Vec<String>) =
        std::env::args().skip(1).partition(|a| a.starts_with("--"));
    let delete_existing = flags.iter().any(|a| a == "--delete-existing");
    if let Some(unknown) = flags.iter().find(|a| *a != "--delete-existing") {
        eprintln!("unknown flag: {unknown}");
        eprintln!("usage: anima-login [host] [port] [username] [password] [--delete-existing]");
        std::process::exit(2);
    }
    let mut args = positional.into_iter();
    let host = args.next().unwrap_or_else(|| "127.0.0.1".to_string());
    let port: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2594);
    let username = args.next().unwrap_or_else(|| "animatest".to_string());
    let password = args.next().unwrap_or_else(|| "animatest".to_string());

    let cfg = LoginConfig {
        username: username.clone(),
        password,
        delete_existing,
        ..Default::default()
    };

    println!("connecting to {host}:{port} as {username} ...");
    let mut session = match Session::connect_and_login(&Endpoint::new(host, port), cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("login failed: {e}");
            std::process::exit(1);
        }
    };

    let player = session.world.player_mobile().cloned().unwrap_or_default();
    println!(
        "LOGGED IN ✓  serial=0x{:08X}  pos=({}, {}, {})  body=0x{:04X}",
        player.serial, player.pos.x, player.pos.y, player.pos.z, player.body
    );

    println!("observing the world for 3s ...");
    match session.observe(Duration::from_secs(3)) {
        Ok(n) => println!("applied {n} game packets"),
        Err(e) => eprintln!("observe error: {e}"),
    }

    let w = &session.world;
    let p = w.player_mobile().cloned().unwrap_or_default();
    let s = &w.player_stats;
    println!("\n=== PERCEPTION ===");
    println!(
        "player: {}  hp {}/{}  mana {}/{}  stam {}/{}",
        if p.name.is_empty() {
            "<unnamed>"
        } else {
            &p.name
        },
        p.hits,
        p.hits_max,
        p.mana,
        p.mana_max,
        p.stam,
        p.stam_max
    );
    println!(
        "stats:  str {} dex {} int {}  gold {}  armor {}  weight {}",
        s.strength, s.dexterity, s.intelligence, s.gold, s.armor, s.weight
    );
    println!("nearby mobiles: {}", w.mobiles.len().saturating_sub(1));
    for m in w.mobiles.values().filter(|m| m.serial != p.serial).take(8) {
        println!(
            "  - 0x{:08X} body=0x{:04X} at ({},{},{}) noto={} {}",
            m.serial, m.body, m.pos.x, m.pos.y, m.pos.z, m.notoriety, m.name
        );
    }
    println!("items in view: {}", w.items.len());
    println!("journal lines: {}", w.journal.len());
    for j in w.journal.iter().rev().take(6).rev() {
        println!("  [{}] {}: {}", j.msg_type, j.name, j.text);
    }

    // --- capstone: pathfind over real map data and navigate on the server ---
    let start = session.world.player_mobile().cloned().unwrap_or_default();
    println!("\n=== NAVIGATION (perception → A* → movement) ===");
    let data_dir = format!(
        "{}/dev/uo/uo-resource",
        std::env::var("HOME").unwrap_or_default()
    );
    match anima_assets::MapData::open(&data_dir) {
        Ok(mut map) => {
            // Target: 10 tiles north-west of spawn (pathfinder routes around walls).
            let gx = start.pos.x.saturating_sub(10) as u32;
            let gy = start.pos.y.saturating_sub(10) as u32;
            println!(
                "navigating from ({}, {}) to ({}, {}) ...",
                start.pos.x, start.pos.y, gx, gy
            );
            match session.navigate_to(&mut map, gx, gy, 60) {
                Ok(arrived) => {
                    let end = session.world.player_mobile().cloned().unwrap_or_default();
                    println!(
                        "now at ({}, {})  (confirms={} denies={})",
                        end.pos.x, end.pos.y, session.confirms, session.denies
                    );
                    if arrived {
                        println!("ARRIVED ✓");
                    } else if (end.pos.x, end.pos.y) != (start.pos.x, start.pos.y) {
                        println!("MOVED ✓ (partial — got closer)");
                    } else {
                        println!("did not move");
                    }
                }
                Err(e) => eprintln!("navigate error: {e}"),
            }
        }
        Err(e) => {
            eprintln!("(no map data at {data_dir}: {e}) — skipping navigation");
        }
    }
}

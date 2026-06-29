//! `anima-cmd` — drive the running `play` server's character from the shell.
//!
//! Lets an operator (or an AI agent) issue any action and observe the world
//! without a browser. Talks to the play server's HTTP `/input` + `/scene.json`.
//!
//! Usage:
//!   anima-cmd look                      # print player + nearby mobiles/items (with serials) + journal
//!   anima-cmd walk <dir>                # one step (dir = n/ne/e/se/s/sw/w/nw or 0..7)
//!   anima-cmd run  <dir>                # running step
//!   anima-cmd go   <dir> <n>            # n running steps (paced ~200ms)
//!   anima-cmd say  <text...>            # speak
//!   anima-cmd use|click|attack|pickup|target <serial>
//!   anima-cmd targetxy <x> <y> [z] [graphic]
//!   anima-cmd war on|off
//!   anima-cmd raw  <body>               # raw /input body
//! Env: ANIMA_URL (default http://127.0.0.1:8092)

use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread::sleep;
use std::time::Duration;

fn main() {
    let url = std::env::var("ANIMA_URL").unwrap_or_else(|_| "http://127.0.0.1:8092".into());
    let (host, port) = parse_host(&url);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: anima-cmd <look|walk|run|go|say|use|click|attack|pickup|target|targetxy|war|raw> ...");
        std::process::exit(2);
    }

    let cmd = args[0].as_str();
    let rest = &args[1..];
    match cmd {
        "look" => look(&host, port),
        "walk" => post(&host, port, &format!("walk:{}:0", dir(&rest[0]))),
        "run" => post(&host, port, &format!("walk:{}:1", dir(&rest[0]))),
        "go" => {
            let d = dir(&rest[0]);
            let n: usize = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
            for _ in 0..n {
                post(&host, port, &format!("walk:{d}:1"));
                sleep(Duration::from_millis(210));
            }
        }
        "say" => post(&host, port, &format!("say:{}", rest.join(" "))),
        "use" => post(&host, port, &format!("use:{}", rest[0])),
        "click" => post(&host, port, &format!("click:{}", rest[0])),
        "attack" => post(&host, port, &format!("attack:{}", rest[0])),
        "pickup" => post(&host, port, &format!("pickup:{}", rest[0])),
        "target" => post(&host, port, &format!("target:{}", rest[0])),
        "targetxy" => post(&host, port, &format!("targetxy:{}", rest.join(":"))),
        "war" => post(&host, port, &format!("war:{}", rest.first().map(|s| s.as_str()).unwrap_or("on"))),
        "raw" => post(&host, port, &rest.join(" ")),
        other => {
            eprintln!("unknown command: {other}");
            std::process::exit(2);
        }
    }
}

/// Direction name or number → UO direction 0..7.
fn dir(s: &str) -> u8 {
    match s.to_lowercase().as_str() {
        "n" | "north" => 0,
        "ne" => 1,
        "e" | "east" => 2,
        "se" => 3,
        "s" | "south" => 4,
        "sw" => 5,
        "w" | "west" => 6,
        "nw" => 7,
        n => n.parse::<u8>().unwrap_or(0) & 7,
    }
}

fn look(host: &str, port: u16) {
    let body = match http(host, port, "GET", "/scene.json", "") {
        Ok(b) => b,
        Err(e) => {
            eprintln!("look failed: {e}");
            std::process::exit(1);
        }
    };
    // The response body may be chunked-encoded; pull out the JSON object itself.
    let json = match (body.find('{'), body.rfind('}')) {
        (Some(a), Some(b)) if b >= a => &body[a..=b],
        _ => body.as_str(),
    };
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bad scene json: {e}");
            std::process::exit(1);
        }
    };
    let p = &v["player"];
    println!(
        "player {} @({},{},{}) dir {}  hp {}/{} mana {}/{} stam {}/{}  STR/DEX/INT {}/{}/{}  gold {}",
        p["name"], p["x"], p["y"], p["z"], p["dir"],
        p["hits"], p["hitsMax"], p["mana"], p["manaMax"], p["stam"], p["stamMax"],
        p["str"], p["dex"], p["int"], p["gold"]
    );
    if let Some(mobs) = v["mobiles"].as_array() {
        println!("mobiles ({}):", mobs.len());
        for m in mobs.iter().take(20) {
            println!(
                "  0x{:08X}  body 0x{:04X} noto {}  @({},{})  {}",
                m["serial"].as_u64().unwrap_or(0),
                m["body"].as_u64().unwrap_or(0),
                m["noto"], m["x"], m["y"],
                m["name"].as_str().unwrap_or("")
            );
        }
    }
    if let Some(items) = v["items"].as_array() {
        println!("ground items ({}):", items.len());
        for it in items.iter().take(20) {
            println!(
                "  0x{:08X}  g 0x{:04X}  @({},{})",
                it["serial"].as_u64().unwrap_or(0),
                it["g"].as_u64().unwrap_or(0),
                it["x"], it["y"]
            );
        }
    }
    if let Some(j) = v["journal"].as_array() {
        for line in j.iter().rev().take(6).rev() {
            println!("  [{}] {}: {}", line["type"], line["name"].as_str().unwrap_or(""), line["text"].as_str().unwrap_or(""));
        }
    }
}

fn post(host: &str, port: u16, body: &str) {
    match http(host, port, "POST", "/input", body) {
        Ok(_) => println!("→ {body}"),
        Err(e) => {
            eprintln!("post failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Minimal HTTP/1.1 client (local server). Handles `Transfer-Encoding: chunked`.
fn http(host: &str, port: u16, method: &str, path: &str, body: &str) -> std::io::Result<String> {
    let mut s = TcpStream::connect((host, port))?;
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf)?;
    let sep = find(&buf, b"\r\n\r\n").map(|i| i + 4).unwrap_or(0);
    let (head, payload) = buf.split_at(sep);
    let chunked = String::from_utf8_lossy(head).to_lowercase().contains("transfer-encoding: chunked");
    let out = if chunked { dechunk(payload) } else { payload.to_vec() };
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn dechunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let nl = match find(&b[i..], b"\r\n") {
            Some(n) => i + n,
            None => break,
        };
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[i..nl]).trim(), 16).unwrap_or(0);
        i = nl + 2;
        if size == 0 || i + size > b.len() {
            if i < b.len() {
                out.extend_from_slice(&b[i..(i + size).min(b.len())]);
            }
            break;
        }
        out.extend_from_slice(&b[i..i + size]);
        i += size;
        if b[i..].starts_with(b"\r\n") {
            i += 2;
        }
    }
    out
}

fn parse_host(url: &str) -> (String, u16) {
    let u = url.trim_start_matches("http://").trim_start_matches("https://");
    let hp = u.split('/').next().unwrap_or("127.0.0.1:8092");
    match hp.split_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(8092)),
        None => (hp.to_string(), 80),
    }
}

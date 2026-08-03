//! `anima-relay` — the WebSocket↔TCP bridge that lets a browser reach a shard.
//!
//! Browsers cannot open raw TCP sockets, and UO is raw TCP (DESIGN.md §4).
//! That is the whole reason this process exists. It is a **byte pump**: the UO
//! protocol runs in the browser, in `anima-core` compiled to WASM
//! (`anima-wasm`), so nothing here parses a packet, knows what a packet is, or
//! could tell a login from a footstep. Bytes off the WebSocket go to the
//! socket; bytes off the socket go back as binary frames.
//!
//! ```text
//!   browser (anima-wasm = anima-core)  ⇄ WebSocket ⇄  anima-relay  ⇄ TCP ⇄  shard
//! ```
//!
//! Usage:
//!
//! ```text
//! anima-relay [listen_addr] [allowed_target …]
//! anima-relay 127.0.0.1:2595 127.0.0.1:2593 127.0.0.1:2594     # the default
//! ```
//!
//! **The allowlist is not optional.** A relay that dials whatever a client
//! names is an open proxy: anyone who can reach it can use this process to
//! connect to anything it can reach, including hosts behind a firewall it
//! happens to sit inside. So the targets are given on the command line, the
//! client picks *among them* by index, and it never gets to name a host at
//! all. The default listen address is loopback for the same reason
//! `play_server`'s is.
//!
//! The client opens `ws://<listen>/relay?target=<index>`; `GET /targets`
//! returns the allowlist as JSON so a page can offer a shard picker without
//! being told what it may dial.

mod ws;

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// How long a socket read blocks before we check whether the other half died.
/// Both directions are plain blocking reads on their own thread; this is what
/// lets a closed WebSocket tear down its TCP peer promptly instead of leaving
/// it parked until the shard says something.
const POLL: Duration = Duration::from_millis(200);

fn main() {
    let mut args = std::env::args().skip(1);
    let listen = args.next().unwrap_or_else(|| "127.0.0.1:2595".into());
    let targets: Vec<String> = {
        let rest: Vec<String> = args.collect();
        if rest.is_empty() {
            vec!["127.0.0.1:2593".into(), "127.0.0.1:2594".into()]
        } else {
            rest
        }
    };
    let targets = Arc::new(targets);

    let listener = match TcpListener::bind(&listen) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("relay: cannot bind {listen}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("relay: listening on ws://{listen}/relay?target=<index>");
    for (i, t) in targets.iter().enumerate() {
        eprintln!("relay:   target {i} = {t}");
    }

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let targets = Arc::clone(&targets);
        std::thread::spawn(move || {
            if let Err(e) = serve(stream, &targets) {
                // A browser tab closing is an ordinary end, not news.
                if e.kind() != ErrorKind::UnexpectedEof {
                    eprintln!("relay: connection ended: {e}");
                }
            }
        });
    }
}

fn serve(mut client: TcpStream, targets: &[String]) -> std::io::Result<()> {
    client.set_nodelay(true).ok();
    let req = read_request_head(&mut client)?;
    let (path, headers) = parse_head(&req);

    if path.starts_with("/targets") {
        let body = format!(
            "[{}]",
            targets
                .iter()
                .enumerate()
                .map(|(i, t)| format!("{{\"index\":{i},\"target\":\"{t}\"}}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        return write_http(
            &mut client,
            "200 OK",
            "application/json",
            // A page served from the play server is a different origin than the
            // relay, and this list is not a secret.
            Some("Access-Control-Allow-Origin: *\r\n"),
            &body,
        );
    }
    if !path.starts_with("/relay") {
        return write_http(
            &mut client,
            "404 Not Found",
            "text/plain",
            None,
            "no such path",
        );
    }

    let index: usize = query_param(path, "target")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let Some(target) = targets.get(index) else {
        return write_http(
            &mut client,
            "400 Bad Request",
            "text/plain",
            None,
            "no such target index; see GET /targets",
        );
    };

    let Some(key) = header(&headers, "sec-websocket-key") else {
        return write_http(
            &mut client,
            "400 Bad Request",
            "text/plain",
            None,
            "not a WebSocket upgrade",
        );
    };

    let upstream = match TcpStream::connect(target.as_str()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("relay: cannot reach {target}: {e}");
            return write_http(
                &mut client,
                "502 Bad Gateway",
                "text/plain",
                None,
                "target unreachable",
            );
        }
    };
    upstream.set_nodelay(true).ok();
    upstream.set_read_timeout(Some(POLL)).ok();
    client.set_read_timeout(Some(POLL)).ok();

    client.write_all(
        format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Accept: {}\r\n\r\n",
            ws::accept_key(&key)
        )
        .as_bytes(),
    )?;
    eprintln!("relay: bridging a client to {target}");

    pump(client, upstream)
}

/// Run both directions until either end stops. Each gets a thread doing a
/// blocking read; when one side ends it shuts the other's socket down, which
/// unblocks that thread rather than leaving it waiting on a peer that is gone.
fn pump(client: TcpStream, upstream: TcpStream) -> std::io::Result<()> {
    let (mut c_read, mut c_write) = (client.try_clone()?, client);
    let (mut u_read, mut u_write) = (upstream.try_clone()?, upstream);

    // server → browser
    let to_browser = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match u_read.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if ws::write_frame(&mut c_write, ws::OP_BINARY, &buf[..n]).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
                Err(_) => break,
            }
        }
        let _ = ws::write_frame(&mut c_write, ws::OP_CLOSE, &[]);
        let _ = c_write.shutdown(std::net::Shutdown::Both);
    });

    // browser → server. A separate handle on the client socket answers pings;
    // `c_read` is busy blocking on the next frame.
    let mut c_pong = c_read.try_clone()?;
    loop {
        match ws::read_frame(&mut c_read) {
            Ok(None) => break,
            Ok(Some(f)) => match f.opcode {
                ws::OP_BINARY => {
                    if u_write.write_all(&f.payload).is_err() {
                        break;
                    }
                }
                ws::OP_PING => {
                    if ws::write_frame(&mut c_pong, ws::OP_PONG, &f.payload).is_err() {
                        break;
                    }
                }
                ws::OP_PONG => {}
                ws::OP_CLOSE => break,
                _ => break, // text or a fragment: not something a byte pump can mean
            },
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
    let _ = u_write.shutdown(std::net::Shutdown::Both);
    let _ = to_browser.join();
    Ok(())
}

/// Read up to the end of the HTTP request head. Bounded so a client cannot
/// make us buffer forever by never sending the blank line.
fn read_request_head(s: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while buf.len() < 8192 {
        match s.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                buf.push(byte[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn parse_head(req: &str) -> (&str, Vec<(String, &str)>) {
    let mut lines = req.split("\r\n");
    let path = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim()))
        .collect();
    (path, headers)
}

fn header(headers: &[(String, &str)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| (*v).to_string())
}

fn query_param<'a>(path: &'a str, name: &str) -> Option<&'a str> {
    path.split_once('?')?
        .1
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn write_http(
    s: &mut TcpStream,
    status: &str,
    ctype: &str,
    extra: Option<&str>,
    body: &str,
) -> std::io::Result<()> {
    s.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{}\r\n{body}",
            body.len(),
            extra.unwrap_or("")
        )
        .as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_head_is_split_into_path_and_headers() {
        let req = "GET /relay?target=1 HTTP/1.1\r\nHost: x\r\nSec-WebSocket-Key: abc\r\n\r\n";
        let (path, headers) = parse_head(req);
        assert_eq!(path, "/relay?target=1");
        assert_eq!(
            header(&headers, "sec-websocket-key").as_deref(),
            Some("abc")
        );
        // Header names are case-insensitive on the wire; browsers vary.
        assert_eq!(header(&headers, "host").as_deref(), Some("x"));
        assert_eq!(query_param(path, "target"), Some("1"));
        assert_eq!(query_param(path, "missing"), None);
    }
}

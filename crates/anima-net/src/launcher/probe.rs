//! Read-only RunUO/ServUO status request (RemoteAdmin command 0xFF, not auth).
//! Servers without it still get a TCP reachability result. No account is used.
use super::{now, ServerInfo};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub(super) fn check(host: &str, port: u16) -> ServerInfo {
    let mut info = ServerInfo {
        checked_at: now(),
        ..Default::default()
    };
    let started = Instant::now();
    let target = (host.to_string(), port);
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let addresses = target
            .to_socket_addrs()
            .map(|a| a.take(4).collect::<Vec<_>>());
        let _ = tx.send(addresses);
    });
    let Ok(Ok(addresses)) = rx.recv_timeout(Duration::from_secs(3)) else {
        return info;
    };
    for addr in addresses {
        let Some(remaining) = Duration::from_secs(5).checked_sub(started.elapsed()) else {
            break;
        };
        let dial = Instant::now();
        let Ok(mut stream) =
            TcpStream::connect_timeout(&addr, remaining.min(Duration::from_secs(2)))
        else {
            continue;
        };
        info.reachable = true;
        info.latency_ms = Some(dial.elapsed().as_millis() as u64);
        let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(800)));
        // Four-byte seed, then variable-length 0xF1 packet, command 0xFF.
        if stream.write_all(&[0x7f, 0, 0, 1, 0xf1, 0, 4, 0xff]).is_ok() {
            let mut bytes = vec![];
            let mut buf = [0u8; 1024];
            let deadline = Instant::now() + Duration::from_secs(2);
            while bytes.len() < 4096 && Instant::now() < deadline {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        bytes.extend_from_slice(&buf[..n]);
                        if bytes.contains(&0) {
                            break;
                        }
                    }
                }
            }
            parse_details(&bytes, &mut info);
        }
        break;
    }
    info
}
fn parse_details(bytes: &[u8], info: &mut ServerInfo) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    if !text.starts_with("ServUO,") && !text.starts_with("RunUO,") {
        return;
    }
    for part in text.trim_end_matches('\0').split(',').skip(1) {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        match key.trim() {
            "Name" => {
                info.reported_name = Some(
                    value
                        .trim()
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(80)
                        .collect(),
                )
            }
            "Clients" => info.clients = value.trim().parse().ok(),
            "Age" => info.uptime_hours = value.trim().parse().ok(),
            _ => {}
        }
    }
    if info.reported_name.is_some() {
        info.details_at = Some(info.checked_at);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_only_supported_public_status_not_random_tcp_banners() {
        let mut info = ServerInfo {
            checked_at: 42,
            ..Default::default()
        };
        parse_details(
            b"ServUO, Name=Britannia, Age=14, Clients=8, Items=6000\0",
            &mut info,
        );
        assert_eq!(info.reported_name.as_deref(), Some("Britannia"));
        assert_eq!(
            (info.clients, info.uptime_hours, info.details_at),
            (Some(8), Some(14), Some(42))
        );
        let mut other = ServerInfo::default();
        parse_details(b"HTTP/1.1 200 OK, Name=Fake", &mut other);
        assert!(other.reported_name.is_none());
    }
    #[test]
    fn reads_a_fragmented_reply_without_sending_credentials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 8];
            socket.read_exact(&mut request).unwrap();
            assert_eq!(request, [0x7f, 0, 0, 1, 0xf1, 0, 4, 0xff]);
            socket.write_all(b"ServUO, Name=Test, ").unwrap();
            socket.write_all(b"Clients=3, Age=5\0").unwrap();
        });
        let result = check("127.0.0.1", port);
        server.join().unwrap();
        assert!(result.reachable);
        assert_eq!(result.clients, Some(3));
    }
}

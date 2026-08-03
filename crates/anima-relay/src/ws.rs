//! The RFC 6455 subset a byte pump needs: the opening handshake, and binary
//! frames in both directions.
//!
//! Deliberately partial. We never send fragments (so no continuation state on
//! the write side), we never negotiate an extension, and the only opcodes that
//! mean anything here are binary, ping, pong and close. Text frames are
//! refused rather than guessed at — a client sending text to a raw-TCP relay
//! has misunderstood what this is.

use std::io::{self, Read, Write};

/// The fixed GUID RFC 6455 §4.2.2 appends to `Sec-WebSocket-Key` before
/// hashing, which is how the client proves the server understood the upgrade
/// rather than echoing whatever it was sent.
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Maximum payload we will accept in one frame. A UO packet is far smaller;
/// this exists so a hostile client cannot make us allocate arbitrarily.
const MAX_FRAME: u64 = 1 << 20;

pub struct Frame {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

/// Compute the `Sec-WebSocket-Accept` value for a client's key.
pub fn accept_key(client_key: &str) -> String {
    let mut input = client_key.trim().to_string();
    input.push_str(WS_GUID);
    base64(&sha1(input.as_bytes()))
}

/// Read one frame. `Ok(None)` means the peer closed cleanly.
///
/// Client→server frames must be masked (RFC 6455 §5.1); an unmasked one is a
/// protocol error rather than something to tolerate, since accepting it would
/// let a plain HTTP request be smuggled through a proxy as a frame.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut head = [0u8; 2];
    if let Err(e) = r.read_exact(&mut head) {
        return if e.kind() == io::ErrorKind::UnexpectedEof {
            Ok(None)
        } else {
            Err(e)
        };
    }
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            u16::from_be_bytes(b) as u64
        }
        127 => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            u64::from_be_bytes(b)
        }
        n => n as u64,
    };
    if !masked {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "client frame was not masked",
        ));
    }
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds the {MAX_FRAME}-byte cap"),
        ));
    }
    let mut mask = [0u8; 4];
    r.read_exact(&mut mask)?;
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)?;
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    Ok(Some(Frame { opcode, payload }))
}

/// Write one unfragmented, unmasked frame (server→client frames are never
/// masked — RFC 6455 §5.1).
pub fn write_frame(w: &mut impl Write, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode); // FIN + opcode
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n if n <= u16::MAX as usize => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    w.write_all(&out)
}

// ---------------------------------------------------------------------------
// The two primitives the handshake needs. Both are here rather than pulled in
// as dependencies because they are used for exactly one thing: proving to the
// client that we parsed its upgrade request. Neither is security-sensitive in
// this role — SHA-1's collision weakness has no bearing on echoing a nonce.
// ---------------------------------------------------------------------------

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6455 §1.3's worked example — the one value every WebSocket
    /// implementation is checked against.
    #[test]
    fn accept_key_matches_the_rfc_example() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn base64_pads_partial_groups() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn frames_round_trip_through_a_masked_read() {
        // Build what a browser would send: FIN + binary, masked.
        let payload = b"\x02\x01\x00\xde\xad\xbe\xef";
        let mask = [0x37u8, 0xfa, 0x21, 0x3d];
        let mut wire = vec![0x80 | OP_BINARY, 0x80 | payload.len() as u8];
        wire.extend_from_slice(&mask);
        wire.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));

        let f = read_frame(&mut &wire[..]).unwrap().expect("a frame");
        assert_eq!(f.opcode, OP_BINARY);
        assert_eq!(f.payload, payload);

        let mut out = Vec::new();
        write_frame(&mut out, OP_BINARY, payload).unwrap();
        assert_eq!(out[0], 0x80 | OP_BINARY);
        assert_eq!(out[1], payload.len() as u8, "server frames are not masked");
        assert_eq!(&out[2..], payload);
    }

    #[test]
    fn an_unmasked_client_frame_is_rejected() {
        let wire = [0x80 | OP_BINARY, 0x01, 0xAA];
        assert!(read_frame(&mut &wire[..]).is_err());
    }
}

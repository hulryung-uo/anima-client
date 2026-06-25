//! Sans-IO frame decoder: turns a byte stream into discrete packets.
//!
//! This handles the **login phase** (uncompressed) stream. The game phase adds
//! a Huffman layer *before* framing — that decompressor (TODO `net::huffman`)
//! will feed its decompressed output into the same length-based logic here.
//!
//! Sans-IO by design: you `feed()` whatever bytes arrived from the socket and
//! `pop()` complete frames. No sockets, no async — so it runs identically on
//! native and WASM, and is trivially testable from byte vectors (e.g. replaying
//! `uo_proxy` captures).

use super::lengths::{packet_length, PacketLength};

#[derive(Debug, PartialEq, Eq)]
pub enum FramingError {
    /// Packet id is not in the length table — we can't know its boundary, so the
    /// stream can't be safely resynced without higher-level knowledge.
    UnknownPacket(u8),
    /// A variable-length frame declared a total length < 3 (impossible: the
    /// id + length header alone is 3 bytes). Indicates a desync/corruption.
    MalformedLength { id: u8, declared: u16 },
}

/// Accumulates bytes and yields complete frames.
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append freshly-received bytes.
    pub fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Pop one complete frame (id byte included), or `None` if more bytes are
    /// needed. The returned frame includes the id and, for variable packets,
    /// the 2-byte length field — i.e. exactly the bytes on the wire.
    pub fn pop(&mut self) -> Result<Option<Vec<u8>>, FramingError> {
        if self.buf.is_empty() {
            return Ok(None);
        }
        let id = self.buf[0];
        match packet_length(id) {
            PacketLength::Fixed(n) => {
                if self.buf.len() < n {
                    return Ok(None);
                }
                Ok(Some(self.split_off_front(n)))
            }
            PacketLength::Variable => {
                if self.buf.len() < 3 {
                    return Ok(None);
                }
                let declared = u16::from_be_bytes([self.buf[1], self.buf[2]]);
                if declared < 3 {
                    return Err(FramingError::MalformedLength { id, declared });
                }
                let total = declared as usize;
                if self.buf.len() < total {
                    return Ok(None);
                }
                Ok(Some(self.split_off_front(total)))
            }
            PacketLength::Unknown => Err(FramingError::UnknownPacket(id)),
        }
    }

    fn split_off_front(&mut self, n: usize) -> Vec<u8> {
        let frame = self.buf[..n].to_vec();
        self.buf.drain(..n);
        frame
    }
}

/// Game-phase decoder: Huffman-decompress incoming bytes, then frame the
/// decompressed stream with the same length logic as [`FrameDecoder`].
#[derive(Default)]
pub struct GameFrameDecoder {
    compressed: Vec<u8>,
    frames: FrameDecoder,
}

impl GameFrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append freshly-received (compressed) bytes, decompressing every complete
    /// chunk into the inner frame buffer.
    pub fn feed(&mut self, data: &[u8]) {
        self.compressed.extend_from_slice(data);
        while let Some((chunk, consumed)) = super::huffman::decompress_one(&self.compressed, 0) {
            if consumed == 0 {
                break;
            }
            self.compressed.drain(..consumed);
            self.frames.feed(&chunk);
        }
    }

    pub fn pop(&mut self) -> Result<Option<Vec<u8>>, FramingError> {
        self.frames.pop()
    }
}

/// A connection's incoming decoder. Starts in login phase (plaintext) and is
/// switched to game phase (Huffman) when the login handshake reconnects to the
/// game server. Lets a driver hold one object across both phases.
pub enum StreamDecoder {
    Login(FrameDecoder),
    Game(GameFrameDecoder),
}

impl Default for StreamDecoder {
    fn default() -> Self {
        StreamDecoder::Login(FrameDecoder::new())
    }
}

impl StreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Switch to game phase. Call this exactly when reconnecting to the game
    /// server (a fresh connection → fresh, empty Huffman state).
    pub fn switch_to_game(&mut self) {
        *self = StreamDecoder::Game(GameFrameDecoder::new());
    }

    pub fn feed(&mut self, data: &[u8]) {
        match self {
            StreamDecoder::Login(d) => d.feed(data),
            StreamDecoder::Game(d) => d.feed(data),
        }
    }

    pub fn pop(&mut self) -> Result<Option<Vec<u8>>, FramingError> {
        match self {
            StreamDecoder::Login(d) => d.pop(),
            StreamDecoder::Game(d) => d.pop(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_frame_split_across_feeds() {
        // 0x55 LoginComplete = fixed 1 byte; 0x8C ServerRedirect = fixed 11.
        let mut d = FrameDecoder::new();
        d.feed(&[0x55]);
        assert_eq!(d.pop().unwrap(), Some(vec![0x55]));
        assert_eq!(d.pop().unwrap(), None);

        // 0x8C split into two reads.
        d.feed(&[0x8C, 0, 0, 0, 0]);
        assert_eq!(d.pop().unwrap(), None); // only 5 of 11 bytes
        d.feed(&[0, 0, 0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(d.pop().unwrap().unwrap().len(), 11);
    }

    #[test]
    fn variable_frame() {
        // 0xA8 ServerList = variable. Build a 6-byte frame: [id][len=6][body..]
        let mut d = FrameDecoder::new();
        d.feed(&[0xA8, 0x00, 0x06, 0x01, 0x02, 0x03]);
        let f = d.pop().unwrap().unwrap();
        assert_eq!(f, vec![0xA8, 0x00, 0x06, 0x01, 0x02, 0x03]);
        assert_eq!(d.pop().unwrap(), None);
    }

    #[test]
    fn two_frames_back_to_back() {
        let mut d = FrameDecoder::new();
        d.feed(&[0x55, 0x55]); // two LoginComplete
        assert_eq!(d.pop().unwrap(), Some(vec![0x55]));
        assert_eq!(d.pop().unwrap(), Some(vec![0x55]));
        assert_eq!(d.pop().unwrap(), None);
    }

    #[test]
    fn unknown_and_malformed() {
        let mut d = FrameDecoder::new();
        d.feed(&[0x50]); // not in table
        assert_eq!(d.pop(), Err(FramingError::UnknownPacket(0x50)));

        let mut d2 = FrameDecoder::new();
        d2.feed(&[0xA8, 0x00, 0x02]); // variable but declared < 3
        assert_eq!(
            d2.pop(),
            Err(FramingError::MalformedLength {
                id: 0xA8,
                declared: 2
            })
        );
    }
}

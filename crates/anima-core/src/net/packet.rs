//! Big-endian packet primitives. The entire UO wire protocol is big-endian.

use core::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum PacketError {
    /// Tried to read past the end of the buffer.
    UnexpectedEof { needed: usize, remaining: usize },
}

impl fmt::Display for PacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacketError::UnexpectedEof { needed, remaining } => write!(
                f,
                "unexpected end of packet: needed {needed} byte(s), {remaining} remaining"
            ),
        }
    }
}

impl std::error::Error for PacketError {}

pub type Result<T> = core::result::Result<T, PacketError>;

/// Cursor-based big-endian reader over a borrowed byte slice.
pub struct PacketReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> PacketReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(PacketError::UnexpectedEof {
                needed: n,
                remaining: self.remaining(),
            });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.take(n)
    }

    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    /// Advance past `n` bytes.
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    /// Consume and return all remaining bytes.
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }

    /// Read a fixed-length ASCII field, trimming at the first NUL.
    pub fn fixed_ascii(&mut self, len: usize) -> Result<String> {
        let raw = self.take(len)?;
        let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
        Ok(raw[..end].iter().map(|&c| c as char).collect())
    }
}

/// Growable big-endian writer.
#[derive(Default)]
pub struct PacketWriter {
    buf: Vec<u8>,
}

impl PacketWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    /// Write `n` zero bytes (reserved/padding fields).
    pub fn zeros(&mut self, n: usize) -> &mut Self {
        self.buf.resize(self.buf.len() + n, 0);
        self
    }

    /// Write an ASCII string into a fixed-width, NUL-padded field.
    pub fn fixed_ascii(&mut self, s: &str, width: usize) -> &mut Self {
        let bytes = s.as_bytes();
        let n = bytes.len().min(width);
        self.buf.extend_from_slice(&bytes[..n]);
        self.buf.resize(self.buf.len() + (width - n), 0);
        self
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_big_endian() {
        let mut w = PacketWriter::new();
        w.u8(0x80)
            .u32(0xDEAD_BEEF)
            .u16(0x1234)
            .fixed_ascii("anima", 30);
        let bytes = w.into_vec();

        let mut r = PacketReader::new(&bytes);
        assert_eq!(r.u8().unwrap(), 0x80);
        assert_eq!(r.u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.fixed_ascii(30).unwrap(), "anima");
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn reading_past_end_errors() {
        let buf = [0x01u8, 0x02];
        let mut r = PacketReader::new(&buf);
        assert_eq!(r.u16().unwrap(), 0x0102);
        assert!(r.u8().is_err());
    }
}

//! Shared decoding primitives for the game-phase handlers.
//!
//! Nothing here touches [`World`] — these are the string encodings and framed
//! reads the wire format keeps repeating (ASCII with a CP-1252 high half,
//! UTF-16 in both byte orders, NUL-terminated variants, the zlib block 0xDD
//! and the custom-house planes share). They live together because every
//! handler module needs some of them, and because getting an encoding subtly
//! wrong is the kind of bug that only shows up on somebody else's locale.

use super::super::packet::{PacketError, PacketReader, Result as PResult};

/// Decode a length-prefixed UTF-8 field, trimming at the first NUL (ClassicUO
/// `StackDataReader.ReadUTF8` decodes the field then truncates at the first
/// embedded NUL it finds, even though the cursor still advances by the full
/// field length).
pub(super) fn utf8_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Read a 0xDD compressed block: `[compLen+4:u32][decompLen:u32][zlib bytes]` and
/// return the inflated payload. The first u32 counts the 4-byte decompLen field
/// plus the zlib bytes, so the zlib data is `first - 4` bytes. A decode failure
/// (or a zero/short block) yields an empty buffer rather than erroring the stream.
pub(super) fn read_zlib_block(r: &mut PacketReader) -> PResult<Vec<u8>> {
    let packed_len = r.u32()? as usize;
    if packed_len < 4 {
        return Ok(Vec::new()); // ServUO writes a bare 0 u32 for an empty block
    }
    let _decomp_len = r.u32()?;
    let zlib = r.bytes(packed_len - 4)?;
    // The protocol mandates zlib here; miniz_oxide is a pure-Rust, wasm-clean
    // inflate (the one justified non-std dep in core). A corrupt block is skipped.
    Ok(miniz_oxide::inflate::decompress_to_vec_zlib(zlib).unwrap_or_default())
}

/// Read `count` gump text lines, each `[charLen:u16][text: utf16-be, charLen*2
/// bytes]`. `charLen` is a UTF-16 code-unit count (not a byte count). Stops early
/// if the buffer runs out (a truncated/odd line yields an empty string).
pub(super) fn read_gump_text_lines(r: &mut PacketReader, count: usize) -> Vec<String> {
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        let char_len = match r.u16() {
            Ok(n) => n as usize,
            Err(_) => break,
        };
        match r.bytes(char_len * 2) {
            Ok(b) => lines.push(unicode_string(b)),
            Err(_) => {
                lines.push(String::new());
                break;
            }
        }
    }
    lines
}

/// Read a NUL-terminated ASCII string from the reader (consuming the NUL). Stops at
/// end-of-buffer if no NUL is found.
pub(super) fn read_nul_ascii(r: &mut PacketReader) -> String {
    let mut s = String::new();
    while let Ok(b) = r.u8() {
        if b == 0 {
            break;
        }
        s.push(b as char);
    }
    s
}

pub(super) fn take_utf16be_nul(bytes: &[u8], offset: &mut usize) -> PResult<String> {
    let start = *offset;
    let remaining = bytes.get(start..).ok_or(PacketError::InvalidData(
        "profile string offset out of range",
    ))?;
    for (index, pair) in remaining.chunks_exact(2).enumerate() {
        if pair == [0, 0] {
            let end = start + index * 2;
            *offset = end + 2;
            return Ok(decode_unicode(&bytes[start..end], true));
        }
    }
    Err(PacketError::InvalidData(
        "profile unicode string has no terminator",
    ))
}

/// Read a UTF-16 BE, NUL-terminated string from `r` (UO's `ReadUnicodeBE`),
/// consuming code units up to and including the `0x0000` terminator. A
/// truncated stream (EOF before a terminator) returns what was decoded so far
/// rather than erroring — the caller has usually already read everything it
/// cares about by then.
pub(super) fn utf16be_string(r: &mut PacketReader) -> String {
    let mut units = Vec::new();
    while let Ok(c) = r.u16() {
        if c == 0 {
            break;
        }
        units.push(c);
    }
    String::from_utf16_lossy(&units)
}

/// Decode a UTF-16 string (LE or BE), stopping at the first NUL.
pub(super) fn decode_unicode(bytes: &[u8], big_endian: bool) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let c = if big_endian {
            u16::from_be_bytes([pair[0], pair[1]])
        } else {
            u16::from_le_bytes([pair[0], pair[1]])
        };
        if c == 0 {
            break;
        }
        units.push(c);
    }
    String::from_utf16_lossy(&units)
}

pub(super) fn ascii_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&c| cp1252_char(c)).collect()
}

/// ClassicUO `StringHelper.Cp1252ToUnicode`: server “ASCII” strings use the
/// Windows-1252 printable extension rather than ISO-8859-1's C1 controls.
pub(super) fn cp1252_char(byte: u8) -> char {
    match byte {
        128 => '\u{20AC}', // €
        130 => '\u{201A}', // ‚
        131 => '\u{0192}', // ƒ
        132 => '\u{201E}', // „
        133 => '\u{2026}', // …
        134 => '\u{2020}', // †
        135 => '\u{2021}', // ‡
        136 => '\u{02C6}', // ˆ
        137 => '\u{2030}', // ‰
        138 => '\u{0160}', // Š
        139 => '\u{2039}', // ‹
        140 => '\u{0152}', // Œ
        142 => '\u{017D}', // Ž
        145 => '\u{2018}', // ‘
        146 => '\u{2019}', // ’
        147 => '\u{201C}', // “
        148 => '\u{201D}', // ”
        149 => '\u{2022}', // •
        150 => '\u{2013}', // –
        151 => '\u{2014}', // —
        152 => '\u{02DC}', // ˜
        153 => '\u{2122}', // ™
        154 => '\u{0161}', // š
        155 => '\u{203A}', // ›
        156 => '\u{0153}', // œ
        158 => '\u{017E}', // ž
        159 => '\u{0178}', // Ÿ
        _ => byte as char,
    }
}

/// Decode a big-endian UTF-16 string, stopping at a NUL char.
pub(super) fn unicode_string(bytes: &[u8]) -> String {
    let mut out = String::new();
    for pair in bytes.chunks_exact(2) {
        let c = u16::from_be_bytes([pair[0], pair[1]]);
        if c == 0 {
            break;
        }
        out.push(char::from_u32(c as u32).unwrap_or('\u{FFFD}'));
    }
    out
}

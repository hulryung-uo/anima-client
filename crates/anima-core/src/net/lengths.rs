//! Packet-length table — how to frame each packet id off the wire.
//!
//! Ported verbatim from `anima/anima/client/packets.py` `PACKET_LENGTHS`
//! (itself from ClassicUO `Network/PacketsTable.cs`), assuming a modern
//! 7.0.102.3 client. Several ids have version-dependent lengths; the values
//! here match what ServUO emits for that advertised version. See the Python
//! source for the per-id rationale comments.

/// How a packet id is framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketLength {
    /// Fixed total length in bytes, **including** the 1-byte id.
    Fixed(usize),
    /// Variable: bytes `1..3` are a big-endian u16 total length (incl. id + len field).
    Variable,
    /// Not in the table. The caller must decide how to resync.
    Unknown,
}

const UNKNOWN: i16 = -1;
const VARIABLE: i16 = 0;

/// (id, length) pairs. `0` = variable, `>0` = fixed (incl. id). Anything not
/// listed is [`PacketLength::Unknown`]. Kept as a flat list so it eyeballs 1:1
/// against the Python source.
const ENTRIES: &[(u8, i16)] = &[
    (0x00, 104),
    (0x01, 5),
    (0x02, 7),
    (0x03, 0),
    (0x04, 2),
    (0x05, 5),
    (0x06, 5),
    (0x07, 7),
    (0x08, 15),
    (0x09, 5),
    (0x0A, 11),
    (0x0B, 7),
    (0x0C, 0),
    (0x0D, 3),
    (0x0E, 0),
    (0x0F, 61),
    (0x10, 0),
    (0x11, 0),
    (0x12, 0),
    (0x13, 10),
    (0x14, 6),
    (0x15, 9),
    (0x16, 0),
    (0x17, 0),
    (0x18, 0),
    (0x19, 0),
    (0x1A, 0),
    (0x1B, 37),
    (0x1C, 0),
    (0x1D, 5),
    (0x1E, 4),
    (0x1F, 8),
    (0x20, 19),
    (0x21, 8),
    (0x22, 3),
    (0x23, 26),
    (0x24, 9),
    (0x25, 21),
    (0x26, 0),
    (0x27, 2),
    (0x28, 5),
    (0x29, 1),
    (0x2A, 5),
    (0x2B, 2),
    (0x2C, 2),
    (0x2D, 17),
    (0x2E, 15),
    (0x2F, 10),
    (0x30, 5),
    (0x31, 1),
    (0x32, 2),
    (0x33, 0),
    (0x34, 10),
    (0x35, 0),
    (0x36, 0),
    (0x37, 8),
    (0x38, 7),
    (0x39, 0),
    (0x3A, 0),
    (0x3B, 0),
    (0x3C, 0),
    (0x3E, 37),
    (0x3F, 0),
    (0x40, 0),
    (0x41, 0),
    (0x42, 0),
    (0x43, 0),
    (0x44, 0),
    (0x45, 5),
    (0x46, 0),
    (0x47, 11),
    (0x48, 73),
    (0x49, 63),
    (0x4E, 6),
    (0x4F, 2),
    (0x51, 0),
    (0x53, 2),
    (0x54, 12),
    (0x55, 1),
    (0x56, 11),
    (0x57, 110),
    (0x58, 106),
    (0x5B, 4),
    (0x5D, 73),
    (0x65, 4),
    (0x66, 0),
    (0x6C, 19),
    (0x6D, 3),
    (0x6E, 14),
    (0x6F, 0),
    (0x70, 28),
    (0x71, 0),
    (0x72, 5),
    (0x73, 2),
    (0x74, 0),
    (0x75, 35),
    (0x76, 16),
    (0x77, 17),
    (0x78, 0),
    (0x7B, 2),
    (0x7C, 0),
    (0x7D, 13),
    (0x81, 0),
    (0x80, 62),
    (0x82, 2),
    (0x83, 39),
    (0x85, 2),
    (0x86, 0),
    (0x88, 66),
    (0x89, 0),
    (0x8B, 0),
    (0x8C, 11),
    (0x90, 19),
    (0x91, 65),
    (0x93, 99),
    (0x95, 9),
    (0x97, 2),
    (0x98, 0),
    (0x99, 30),
    (0x9A, 0),
    (0x9B, 258),
    (0x9E, 0),
    (0x9F, 0),
    (0xA0, 3),
    (0xA1, 9),
    (0xA2, 9),
    (0xA3, 9),
    (0xA4, 149),
    (0xA5, 0),
    (0xA6, 0),
    (0xA7, 4),
    (0xA8, 0),
    (0xA9, 0),
    (0xAA, 5),
    (0xAB, 0),
    (0xAD, 0),
    (0xAE, 0),
    (0xAF, 13),
    (0xB0, 0),
    (0xB1, 0),
    (0xB2, 0),
    (0xB5, 64),
    (0xB6, 9),
    (0xB7, 0),
    (0xB8, 0),
    (0xB9, 5),
    (0xBA, 10),
    (0xBB, 9),
    (0xBC, 3),
    (0xBD, 0),
    (0xBE, 0),
    (0xBF, 0),
    (0xC0, 36),
    (0xC1, 0),
    (0xC2, 0),
    (0xC3, 0),
    (0xC4, 6),
    (0xC6, 1),
    (0xC7, 49),
    (0xC8, 2),
    (0xC9, 6),
    (0xCA, 6),
    (0xCB, 7),
    (0xCC, 0),
    (0xCF, 0),
    (0xD0, 0),
    (0xD1, 2),
    (0xD2, 25),
    (0xD3, 0),
    (0xD4, 0),
    (0xD6, 0),
    (0xD7, 0),
    (0xD8, 0),
    (0xD9, 0),
    (0xDA, 0),
    (0xDB, 0),
    (0xDC, 9),
    (0xDD, 0),
    (0xDE, 0),
    (0xDF, 0),
    (0xE1, 0),
    (0xE2, 10),
    (0xE3, 0),
    (0xE5, 0),
    (0xE6, 5),
    (0xEC, 0),
    (0xED, 0),
    (0xEF, 21),
    (0xF0, 0),
    (0xF1, 0),
    (0xF3, 26),
    (0xF4, 0),
    (0xF5, 21),
    (0xF6, 0),
    (0xF7, 0),
    (0xF8, 106),
    (0xFB, 2),
    (0xFD, 2),
];

const fn build_table() -> [i16; 256] {
    let mut t = [UNKNOWN; 256];
    let mut i = 0;
    while i < ENTRIES.len() {
        let (id, len) = ENTRIES[i];
        t[id as usize] = len;
        i += 1;
    }
    t
}

static TABLE: [i16; 256] = build_table();

/// Framing length for a packet id.
pub fn packet_length(id: u8) -> PacketLength {
    match TABLE[id as usize] {
        UNKNOWN => PacketLength::Unknown,
        VARIABLE => PacketLength::Variable,
        n => PacketLength::Fixed(n as usize),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_lengths() {
        assert_eq!(packet_length(0x1B), PacketLength::Fixed(37)); // LoginConfirm
        assert_eq!(packet_length(0x80), PacketLength::Fixed(62)); // AccountLogin
        assert_eq!(packet_length(0x91), PacketLength::Fixed(65)); // GameLogin
        assert_eq!(packet_length(0x55), PacketLength::Fixed(1)); // LoginComplete
        assert_eq!(packet_length(0xA8), PacketLength::Variable); // ServerList
        assert_eq!(packet_length(0xA9), PacketLength::Variable); // CharacterList
        assert_eq!(packet_length(0x8C), PacketLength::Fixed(11)); // ServerRedirect
                                                                  // Treasure/decoration map packets (ServUO `Scripts/Items/Tools/MapItem.cs`):
                                                                  // a wrong entry here desyncs the whole stream, since these are Fixed, not
                                                                  // Variable — verified against `MapDetails : base(0x90, 19)`,
                                                                  // `NewMapDetails : base(0xF5, 21)`, `MapCommand : base(0x56, 11)`.
        assert_eq!(packet_length(0x90), PacketLength::Fixed(19)); // MapDetails
        assert_eq!(packet_length(0xF5), PacketLength::Fixed(21)); // NewMapDetails
        assert_eq!(packet_length(0x56), PacketLength::Fixed(11)); // MapCommand
    }

    #[test]
    fn unknown_id() {
        assert_eq!(packet_length(0x50), PacketLength::Unknown);
        assert_eq!(packet_length(0xFF), PacketLength::Unknown);
    }
}

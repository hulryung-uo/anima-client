//! Packet-length table — how to frame each packet id off the wire.
//!
//! Ported from ClassicUO `Network/PacketsTable.cs` (by way of
//! `anima/anima/client/packets.py` `PACKET_LENGTHS`), for the 7.0.102.3 client
//! we advertise in the 0xEF seed.
//!
//! The table covers **all 256 ids on purpose**. Framing and handling are
//! separate concerns: an id we have no handler for still has to be consumed by
//! exactly the right number of bytes, and a length we do not know cannot be
//! guessed — one unframed packet desyncs every packet after it, so a gap here
//! ends the session. ClassicUO's table is complete for the same reason.
//!
//! ClassicUO version-gates a handful of ids in its `PacketsTable` constructor.
//! 7.0.102.3 clears every gate except the last: `CV_7010400` is **7.0.104.0**
//! (`ClassicUO.Utility/ClientVersion.cs:51`), above what our 0xEF seed
//! advertises. So the values below are the "new client" branch for CV_500A,
//! CV_5090, CV_6013, CV_6017, CV_6060, CV_60142, CV_7000, CV_7090, CV_70180 and
//! CV_706400 — 0x0B=7, 0x16/0x31/0xE1/0xE3 variable, 0x08=15, 0x24=9, 0x25=21,
//! 0x99=30, 0xB9=5, 0xBA=10, 0xE6..0xEA fixed, 0xEE=10, 0xEF=21, 0xF1=9,
//! 0xF2=25, 0xF3=26, 0xFA=1, 0xFB=2 — but NOT for `CV_7010400`, whose 0xD5=9 we
//! must not take (see 0xD5 below). Nothing is version-conditional at runtime
//! because the version we send is fixed; if that ever changes, this gate is the
//! one that moves. Ids where ServUO (the server we actually run against)
//! contradicts ClassicUO are marked inline.

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

/// (id, length) pairs covering every id 0x00..=0xFF. `0` = variable, `>0` =
/// fixed (incl. id). Deliberately *not* a bare positional `[i16; 256]` like
/// ClassicUO's: theirs is one entry short — the final slot is labelled `// ff`
/// but actually lands on 0xFE — and a positional list gives no way to notice.
/// Explicit ids cannot silently shift.
const ENTRIES: &[(u8, i16)] = &[
    // ServUO `PacketHandlers.cs` registers 0x00 at 104 and the 7.0.16 106-byte
    // create-character as 0xF8; ClassicUO instead widens 0x00 to 106 at CV_70180.
    // We frame what ServUO speaks.
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
    (0x10, 215),
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
    (0x26, 5),
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
    (0x31, 0),
    (0x32, 2),
    (0x33, 2),
    (0x34, 10),
    (0x35, 653),
    (0x36, 0),
    (0x37, 8),
    (0x38, 7),
    (0x39, 9),
    (0x3A, 0),
    (0x3B, 0),
    (0x3C, 0),
    (0x3D, 2),
    (0x3E, 37),
    (0x3F, 0),
    (0x40, 201),
    (0x41, 0),
    (0x42, 0),
    (0x43, 553),
    (0x44, 713),
    (0x45, 5),
    (0x46, 0),
    (0x47, 11),
    (0x48, 73),
    (0x49, 93),
    (0x4A, 5),
    (0x4B, 9),
    (0x4C, 0),
    (0x4D, 0),
    (0x4E, 6),
    (0x4F, 2),
    (0x50, 0),
    (0x51, 0),
    (0x52, 0),
    (0x53, 2),
    (0x54, 12),
    (0x55, 1),
    (0x56, 11),
    (0x57, 110),
    (0x58, 106),
    (0x59, 0),
    (0x5A, 0),
    (0x5B, 4),
    (0x5C, 2),
    (0x5D, 73),
    (0x5E, 0),
    (0x5F, 49),
    (0x60, 5),
    (0x61, 9),
    (0x62, 15),
    (0x63, 13),
    (0x64, 1),
    (0x65, 4),
    (0x66, 0),
    (0x67, 21),
    (0x68, 0),
    (0x69, 0),
    (0x6A, 3),
    (0x6B, 9),
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
    (0x79, 9),
    (0x7A, 0),
    (0x7B, 2),
    (0x7C, 0),
    (0x7D, 13),
    (0x7E, 2),
    (0x7F, 0),
    (0x80, 62),
    (0x81, 0),
    (0x82, 2),
    (0x83, 39),
    (0x84, 69),
    (0x85, 2),
    (0x86, 0),
    (0x87, 0),
    (0x88, 66),
    (0x89, 0),
    (0x8A, 0),
    (0x8B, 0),
    (0x8C, 11),
    (0x8D, 0),
    (0x8E, 0),
    (0x8F, 0),
    (0x90, 19),
    (0x91, 65),
    (0x92, 0),
    (0x93, 99),
    (0x94, 0),
    (0x95, 9),
    (0x96, 0),
    (0x97, 2),
    (0x98, 0),
    (0x99, 30),
    (0x9A, 0),
    (0x9B, 258),
    (0x9C, 309),
    (0x9D, 51),
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
    (0xAC, 0),
    (0xAD, 0),
    (0xAE, 0),
    (0xAF, 13),
    (0xB0, 0),
    (0xB1, 0),
    (0xB2, 0),
    (0xB3, 0),
    (0xB4, 0),
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
    (0xC5, 203),
    (0xC6, 1),
    (0xC7, 49),
    (0xC8, 2),
    (0xC9, 6),
    (0xCA, 6),
    (0xCB, 7),
    (0xCC, 0),
    (0xCD, 1),
    (0xCE, 0),
    // 78, per ClassicUO. ServUO's `Register(0xCF, 0, ...)` (AccountLogin) is a
    // CLIENT->SERVER registration — the length ServUO expects to *receive* — and
    // this table frames the server->client direction only, so it is not evidence
    // either way. ClassicUO is the only source that speaks to our direction.
    (0xCF, 78),
    (0xD0, 0),
    (0xD1, 2),
    (0xD2, 25),
    (0xD3, 0),
    (0xD4, 0),
    // Variable, not the 9 ClassicUO uses — that 9 comes from its `CV_7010400`
    // (7.0.104.0) branch, and we advertise 7.0.102.3, so the server will speak
    // the older variable-length form to us. Framing it as 9 would silently
    // desync every packet after it.
    (0xD5, 0),
    (0xD6, 0),
    (0xD7, 0),
    (0xD8, 0),
    (0xD9, 268),
    (0xDA, 0),
    (0xDB, 0),
    (0xDC, 9),
    (0xDD, 0),
    (0xDE, 0),
    (0xDF, 0),
    (0xE0, 0),
    (0xE1, 0),
    (0xE2, 10),
    (0xE3, 0),
    (0xE4, 0),
    (0xE5, 0),
    (0xE6, 5),
    (0xE7, 12),
    (0xE8, 13),
    (0xE9, 75),
    (0xEA, 3),
    (0xEB, 0),
    (0xEC, 0),
    (0xED, 0),
    (0xEE, 10),
    (0xEF, 21),
    (0xF0, 0),
    (0xF1, 9),
    (0xF2, 25),
    (0xF3, 26),
    (0xF4, 0),
    (0xF5, 21),
    (0xF6, 0),
    (0xF7, 0),
    (0xF8, 106),
    (0xF9, 0),
    (0xFA, 1),
    (0xFB, 2),
    (0xFC, 0),
    // 2 ([id][state]). ClassicUO only tabulates 0xFD in its `CV_7010400` branch,
    // which we are below, leaving its slot at 0 — a length its own framing loop
    // (`PacketHandlers.cs:168`) does not special-case. That is a ClassicUO gap,
    // not a statement that the packet is shorter here, and ServUO never sends
    // 0xFD at all; 2 is the only shape this packet has ever had.
    (0xFD, 2),
    (0xFE, 0),
    (0xFF, 0),
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
    fn every_id_is_framed() {
        for id in 0x00..=0xFFu8 {
            assert_ne!(
                packet_length(id),
                PacketLength::Unknown,
                "0x{id:02X} has no framing"
            );
        }
        // 0x50 (bulletin-board header) and 0xFF were two of the 58 ids the old
        // sparse table left undefined; either one killed the session on arrival.
        assert_eq!(packet_length(0x50), PacketLength::Variable);
        assert_eq!(packet_length(0xFF), PacketLength::Variable);
    }

    #[test]
    fn servuo_beats_classicuo_where_they_disagree() {
        // ServUO `Server/Network/PacketHandlers.cs` registers 0x00 at 104 and
        // routes the 106-byte create-character to its own id, 0xF8; ClassicUO
        // widens 0x00 to 106 at CV_70180 instead. Both are client->server ids,
        // so this only pins what we SEND; 0xCF is the same direction and is left
        // at ClassicUO's value precisely because ServUO's registration says
        // nothing about the direction this table frames.
        assert_eq!(packet_length(0x00), PacketLength::Fixed(104));
        assert_eq!(packet_length(0xF8), PacketLength::Fixed(106));
        assert_eq!(packet_length(0xCF), PacketLength::Fixed(78));
        // 0x31 PetWindow is `: base(0x31)` in ServUO
        // `Scripts/Mobiles/Normal/BaseCreature.cs` — the single-arg `Packet`
        // ctor is the variable-length one, and ClassicUO agrees for CV_500A+.
        // The pre-5.0.0a Fixed(1) we used to carry desyncs on the first pet.
        assert_eq!(packet_length(0x31), PacketLength::Variable);
    }
}

//! Miscellaneous client→server game-phase packet builders.
//!
//! (Movement lives in [`crate::net::movement`]; login in [`crate::net::login`].)

use super::packet::PacketWriter;

/// ClientVersion `0xBD` (variable). The server requests our version with an
/// empty `0xBD`; until we answer, ServUO treats us as not-ready and **denies
/// movement**. Reply with the same version advertised in the login seed.
pub fn build_client_version(version: &str) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBD).u16(0); // length placeholder
    w.bytes(version.as_bytes()).u8(0); // NUL-terminated ASCII
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// Attack `0x05` (5 bytes).
pub fn build_attack(serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x05).u32(serial);
    w.into_vec()
}

/// DoubleClick `0x06` (5 bytes) — "use" an item/mobile.
pub fn build_double_click(serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x06).u32(serial);
    w.into_vec()
}

/// SingleClick `0x09` (5 bytes).
pub fn build_single_click(serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x09).u32(serial);
    w.into_vec()
}

/// StatusRequest `0x34` (10 bytes) — ask the server for our own stats/skills.
/// `request_type` 4 = stats (`0x11`), 5 = full skill list (`0x3A`). ServUO does
/// not push the skill list unsolicited, so the driver requests it on login.
/// Layout: `[0x34][0xEDEDEDED][type:u8][serial:u32]`.
pub fn build_status_request(request_type: u8, serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x34).u32(0xEDED_EDED).u8(request_type).u32(serial);
    w.into_vec()
}

/// TargetResponse `0x6C` (19 bytes) — answer a target cursor.
///
/// Echoes the server's `cursor_id`, `cursor_flag`, and `target_type` (several
/// servers reject a response whose flag/id doesn't match the request).
/// `target_type` 0 = object (use `serial`), 1 = ground (use `x,y,z,graphic`).
/// Layout: `[0x6C][type][cursorID:u32][flag][serial:u32][x:u16][y:u16][z:u16][graphic:u16]`.
#[allow(clippy::too_many_arguments)]
pub fn build_target_response(
    target_type: u8,
    cursor_id: u32,
    cursor_flag: u8,
    serial: u32,
    x: u16,
    y: u16,
    z: i16,
    graphic: u16,
) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x6C)
        .u8(target_type)
        .u32(cursor_id)
        .u8(cursor_flag)
        .u32(serial)
        .u16(x)
        .u16(y)
        .u16(z as u16)
        .u16(graphic);
    w.into_vec()
}

/// PickUp `0x07` (7 bytes): lift `amount` from a stack/item.
pub fn build_pick_up(serial: u32, amount: u16) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x07).u32(serial).u16(amount);
    w.into_vec()
}

/// WarMode `0x72` (5 bytes): toggle combat stance.
pub fn build_war_mode(war: bool) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x72).u8(war as u8).u8(0x32).u8(0x00).u8(0x00);
    w.into_vec()
}

/// AsciiSpeech `0x03` (variable): say `text` in-game.
/// `[0x03][len u16][type u8][hue u16][font u16][ascii + NUL]`.
pub fn build_say(text: &str, msg_type: u8, hue: u16, font: u16) -> Vec<u8> {
    let clamped: String = text.trim().chars().take(128).collect();
    let mut w = PacketWriter::new();
    w.u8(0x03).u16(0); // length placeholder
    w.u8(msg_type).u16(hue).u16(font);
    w.bytes(clamped.as_bytes()).u8(0);
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_packet_shapes() {
        assert_eq!(build_attack(0xABCD), vec![0x05, 0, 0, 0xAB, 0xCD]);
        assert_eq!(build_double_click(0x01), vec![0x06, 0, 0, 0, 1]);
        assert_eq!(build_pick_up(0x01, 5), vec![0x07, 0, 0, 0, 1, 0, 5]);
        assert_eq!(build_war_mode(true), vec![0x72, 1, 0x32, 0, 0]);
        let say = build_say("hi", 0, 0x34, 3);
        assert_eq!(say[0], 0x03);
        assert_eq!(u16::from_be_bytes([say[1], say[2]]) as usize, say.len());
        assert_eq!(&say[8..say.len() - 1], b"hi");
    }

    #[test]
    fn target_response_layout() {
        // Object target: 19 bytes, echoes type/cursor/flag, carries the serial.
        let p = build_target_response(0, 0x1122_3344, 1, 0xAABB_CCDD, 0, 0, 0, 0);
        assert_eq!(p.len(), 19);
        assert_eq!(p[0], 0x6C);
        assert_eq!(p[1], 0); // target_type
        assert_eq!(&p[2..6], &[0x11, 0x22, 0x33, 0x44]); // cursor_id (BE)
        assert_eq!(p[6], 1); // cursor_flag echoed
        assert_eq!(&p[7..11], &[0xAA, 0xBB, 0xCC, 0xDD]); // serial (BE)

        // Ground target: type 1, x/y/z/graphic populated, signed z wraps as u16.
        let g = build_target_response(1, 0, 0, 0, 1000, 2000, -5, 0x01A4);
        assert_eq!(g.len(), 19);
        assert_eq!(g[1], 1);
        assert_eq!(u16::from_be_bytes([g[11], g[12]]), 1000);
        assert_eq!(u16::from_be_bytes([g[13], g[14]]), 2000);
        assert_eq!(g[15..17], (-5i16 as u16).to_be_bytes());
        assert_eq!(u16::from_be_bytes([g[17], g[18]]), 0x01A4);
    }

    #[test]
    fn client_version_framing() {
        let p = build_client_version("7.0.102.3");
        assert_eq!(p[0], 0xBD);
        let len = u16::from_be_bytes([p[1], p[2]]) as usize;
        assert_eq!(len, p.len());
        assert_eq!(&p[3..p.len() - 1], b"7.0.102.3");
        assert_eq!(*p.last().unwrap(), 0);
    }
}

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

/// MegaClilocRequest `0xD6` (variable) — ask the server for the Object Property
/// List (tooltip) of one or more entities. The server replies with a 0xD6
/// MegaCliloc per serial. Ports ClassicUO `Send_MegaClilocRequest`:
/// `[0xD6][len:u16][serial:u32]…` — a length-framed batch of serials. Empty input
/// still produces a well-formed (header-only) packet.
pub fn build_opl_request(serials: &[u32]) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xD6).u16(0); // id + length placeholder
    for &serial in serials {
        w.u32(serial);
    }
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

/// DropItem `0x08` (14 bytes): drop a held item at `(x, y, z)` into `container`
/// (use `0xFFFF_FFFF` for the ground). `gridindex` is always 0 here.
/// Layout: `[0x08][serial:u32][x:u16][y:u16][z:i8][gridindex:u8=0][container:u32]`.
pub fn build_drop(serial: u32, x: u16, y: u16, z: i16, container: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x08)
        .u32(serial)
        .u16(x)
        .u16(y)
        .u8(z as u8)
        .u8(0) // gridindex
        .u32(container);
    w.into_vec()
}

/// EquipRequest `0x13` (10 bytes): wear the held `item` on `mobile` at `layer`.
/// Layout: `[0x13][item:u32][layer:u8][mobile:u32]`.
pub fn build_equip(item: u32, layer: u8, mobile: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x13).u32(item).u8(layer).u32(mobile);
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

/// UnicodeSpeech `0xAD` (variable): say `text` in-game as UNICODE so non-ASCII
/// (e.g. Korean/한글) survives. `[0xAD][len u16][type u8][hue u16][font u16]
/// [lang 4=ASCII "ENU\0"][utf16-be…][0x0000]`. Plain text only (no keyword bits).
pub fn build_unicode_say(text: &str, msg_type: u8, hue: u16, font: u16) -> Vec<u8> {
    let clamped: String = text.trim().chars().take(128).collect();
    let mut w = PacketWriter::new();
    w.u8(0xAD).u16(0); // id + length placeholder
    w.u8(msg_type).u16(hue).u16(font);
    w.bytes(b"ENU\0"); // language tag
    for unit in clamped.encode_utf16() {
        w.u16(unit);
    }
    w.u16(0x0000); // UNICODE NUL terminator
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// CastSpell GeneralInfo `0xBF`, subcommand `0x001C` (modern client path).
///
/// Mirrors ClassicUO's `Send_CastSpell` for `ClientVersion >= CV_60142`:
/// `[0xBF][len u16][0x001C][0x0002][spellID u16]` — all values big-endian, total
/// 9 bytes. The `0x0002` word is the fixed "spell" cast-type ClassicUO writes
/// (vs. casting from a book). ServUO's `0xBF` handler dispatches subcommand
/// `0x1C` to its cast-spell request. If a target is required, the server then
/// sends a target cursor, answered via [`build_target_response`].
pub fn build_cast_spell(spell_id: u16) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0); // packet id + length placeholder
    w.u16(0x001C); // subcommand: cast spell
    w.u16(0x0002); // cast type word (ClassicUO writes 0x0002)
    w.u16(spell_id);
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// Party message to all members. GeneralInfo `0xBF`, subcommand `0x0006`,
/// mode `0x04` (= "to all"), then the text as UNICODE (UTF-16 BE) NUL-terminated.
/// `[0xBF][len u16][0x0006][0x04][utf16-be…][0x0000]`.
pub fn build_party_message(text: &str) -> Vec<u8> {
    let clamped: String = text.trim().chars().take(128).collect();
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0); // packet id + length placeholder
    w.u16(0x0006); // subcommand: party
    w.u8(0x04); // mode: message to all members
    for unit in clamped.encode_utf16() {
        w.u16(unit);
    }
    w.u16(0x0000); // UNICODE NUL terminator
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// Finalize a `0xBF` GeneralInfo packet: patch the big-endian length word at [1..3].
fn finish_general_info(mut data: Vec<u8>) -> Vec<u8> {
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// Party invite-by-target. GeneralInfo `0xBF`, subcommand `0x0006`, sub-sub `0x01`
/// with a zero serial: the server replies with a target cursor, and we target the
/// player to invite. Ported from ClassicUO `Send_PartyInviteRequest`:
/// `[0xBF][len u16][0x0006][0x01][0x00000000]`.
pub fn build_party_invite() -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x01).u32(0);
    finish_general_info(w.into_vec())
}

/// Leave the party (remove ourself). GeneralInfo `0xBF`, subcommand `0x0006`,
/// sub-sub `0x02`, then our own serial. Ported from ClassicUO
/// `Send_PartyRemoveRequest`: `[0xBF][len u16][0x0006][0x02][self serial u32]`.
pub fn build_party_leave(self_serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x02).u32(self_serial);
    finish_general_info(w.into_vec())
}

/// Accept a party invitation. GeneralInfo `0xBF`, subcommand `0x0006`, sub-sub
/// `0x08`, then the inviting leader's serial. Ported from ClassicUO
/// `Send_PartyAccept`: `[0xBF][len u16][0x0006][0x08][leader serial u32]`.
pub fn build_party_accept(leader: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x08).u32(leader);
    finish_general_info(w.into_vec())
}

/// Decline a party invitation. GeneralInfo `0xBF`, subcommand `0x0006`, sub-sub
/// `0x09`, then the inviting leader's serial. Ported from ClassicUO
/// `Send_PartyDecline`: `[0xBF][len u16][0x0006][0x09][leader serial u32]`.
pub fn build_party_decline(leader: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x09).u32(leader);
    finish_general_info(w.into_vec())
}

/// BuyRequest `0x3B` (variable): buy `items` (each `(serial, amount)`) from the
/// vendor mobile `vendor`. Ported from ClassicUO `Send_BuyRequest`:
/// `[0x3B][len:u16][vendor:u32][flag:u8]` then, when buying, per item
/// `[0x1A][serial:u32][amount:u16]`. `flag` is `0x02` (accept-with-list) when
/// there are items, else `0x00` (cancel / close). The leading `0x1A` per item is
/// the layer byte ClassicUO writes verbatim.
pub fn build_buy(vendor: u32, items: &[(u32, u16)]) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x3B).u16(0); // id + length placeholder
    w.u32(vendor);
    if items.is_empty() {
        w.u8(0x00); // cancel
    } else {
        w.u8(0x02); // accept with list
        for &(serial, amount) in items {
            w.u8(0x1A).u32(serial).u16(amount);
        }
    }
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// SellRequest `0x9F` (variable): sell `items` (each `(serial, amount)`) to the
/// vendor mobile `vendor`. Ported from ClassicUO `Send_SellRequest`:
/// `[0x9F][len:u16][vendor:u32][count:u16]` then per item `[serial:u32][amount:u16]`.
pub fn build_sell(vendor: u32, items: &[(u32, u16)]) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x9F).u16(0); // id + length placeholder
    w.u32(vendor);
    w.u16(items.len() as u16);
    for &(serial, amount) in items {
        w.u32(serial).u16(amount);
    }
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// GumpResponse `0xB1` (variable): the player's answer to a server gump (0xB0/0xDD).
///
/// Layout (ports ClassicUO `Send_GumpResponse`):
/// `[0xB1][len:u16][serial:u32][gumpId:u32][buttonId:u32][switchCount:u32]
/// [switches:u32…][entryCount:u32]` then per entry `[entryId:u16][textLen:u16]
/// [text: utf16-be]`. `serial`/`gumpId` echo the gump being answered. A
/// "close/cancel" is `button_id = 0` with no switches and no entries. `text_entries`
/// is `(entryId, text)` for each on-screen text field the gump declared.
pub fn build_gump_response(
    serial: u32,
    gump_id: u32,
    button_id: u32,
    switches: &[u32],
    text_entries: &[(u16, String)],
) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xB1).u16(0); // id + length placeholder
    w.u32(serial).u32(gump_id).u32(button_id);
    w.u32(switches.len() as u32);
    for &s in switches {
        w.u32(s);
    }
    w.u32(text_entries.len() as u32);
    for (id, text) in text_entries {
        // ClassicUO caps each entry at 239 UTF-16 code units.
        let units: Vec<u16> = text.encode_utf16().take(239).collect();
        w.u16(*id).u16(units.len() as u16);
        for unit in units {
            w.u16(unit);
        }
    }
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// RequestPopupMenu GeneralInfo `0xBF`, subcommand `0x0013` (9 bytes).
/// Ask the server for `serial`'s right-click context menu; it replies with
/// 0xBF/0x14. `[0xBF][len u16][0x0013][serial u32]`.
pub fn build_popup_request(serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0); // id + length placeholder
    w.u16(0x0013); // subcommand: request popup menu
    w.u32(serial);
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// PopupMenuSelection GeneralInfo `0xBF`, subcommand `0x0015` (11 bytes).
/// Choose entry `index` from the open context menu for `serial`.
/// `[0xBF][len u16][0x0015][serial u32][index u16]`.
pub fn build_popup_select(serial: u32, index: u16) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0); // id + length placeholder
    w.u16(0x0015); // subcommand: popup selection
    w.u32(serial);
    w.u16(index);
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// BookPageRequest `0x66` (variable): ask the server to send every page of the
/// open book `serial`. `[0x66][len:u16][serial:u32][pageCount:u16=N]` then, for
/// each page `1..=N`, `[pageNum:u16][lineCount:u16=0xFFFF]` — the `0xFFFF` line
/// count is the "send me this page" sentinel (ClassicUO `Send_BookPageDataRequest`).
/// The server replies with one or more 0x66 BookData packets.
pub fn build_book_page_request(serial: u32, page_count: u16) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x66).u16(0); // id + length placeholder
    w.u32(serial).u16(page_count);
    for page in 1..=page_count {
        w.u16(page).u16(0xFFFF);
    }
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// UseCombatAbility `0xD7` (GenericAOS, 15 bytes) — arm a weapon special move.
///
/// ClassicUO `Send_UseCombatAbility` (OutgoingPackets.cs): after the player serial
/// it writes subcommand `0x19`, a 4-byte zero, the ability id, and a trailing `0x0A`.
/// `ability_id` is the `Ability` enum value (the specific move, 1..=32); `0` disarms
/// the currently-armed ability. The server arms/disarms the next swing accordingly.
/// Layout: `[0xD7][len:u16][playerSerial:u32][0x0019][0x00000000][abilityId:u8][0x0A]`.
pub fn build_use_ability(player_serial: u32, ability_id: u8) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xD7).u16(0); // id + length placeholder
    w.u32(player_serial).u16(0x0019).u32(0).u8(ability_id).u8(0x0A);
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// SkillLock `0x3A` (variable) — change a skill's lock state (up/down/locked).
///
/// Ports ClassicUO `Send_SkillStatusChangeRequest` (OutgoingPackets.cs): after the
/// 2-byte length it writes the skill index then the lock state byte.
/// `lock` is 0 = up (raise), 1 = down (lower), 2 = locked.
/// Layout: `[0x3A][len:u16][skillId:u16][lock:u8]` (6 bytes).
pub fn build_skill_lock(skill_id: u16, lock: u8) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x3A).u16(0); // id + length placeholder
    w.u16(skill_id).u8(lock);
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// UseSkill `0x12` ActionRequest (variable) — invoke an (active) skill by id.
///
/// Ports ClassicUO `Send_UseSkill` (OutgoingPackets.cs): the request type byte
/// `0x24` ("use skill"), then the command body as ASCII `"<skillId> 0"` followed
/// by a NUL terminator (ClassicUO's `WriteASCII` appends the NUL).
/// Layout: `[0x12][len:u16][0x24]["<skillId> 0"][0x00]`.
pub fn build_use_skill(skill_id: u16) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x12).u16(0); // id + length placeholder
    w.u8(0x24); // ActionRequest type: use skill
    w.bytes(format!("{skill_id} 0").as_bytes()).u8(0); // ASCII command + NUL
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// UnicodePromptResponse `0xC2` (variable) — answer (or cancel) a pending server
/// text prompt (0xC2 UnicodePrompt: pet rename, house sign, guild abbreviation, …).
///
/// Echoes the server's `serial`/`prompt_id` (ServUO matches by exact sender
/// serial + `Prompt.TypeId` — see `PacketHandlers.UnicodePromptResponse`).
/// `cancel = true` sends `type = 0` (fires `Prompt.OnCancel`) with no text;
/// otherwise `type = 1` and `text` follows as **UTF-16 LE** (unlike almost all the
/// rest of the protocol, which is big-endian — ClassicUO
/// `Send_UnicodePromptResponse` writes it via `WriteUnicodeLE`). `lang` is fixed
/// to `"ENU"` (English), NUL-padded to 4 bytes, matching ClassicUO's default.
/// Layout: `[0xC2][len:u16][serial:u32][promptId:u32][type:u32][lang:4][text:utf16-LE]`.
pub fn build_prompt_response(serial: u32, prompt_id: u32, text: &str, cancel: bool) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xC2).u16(0); // id + length placeholder
    w.u32(serial).u32(prompt_id);
    w.u32(if cancel { 0 } else { 1 });
    w.bytes(b"ENU").u8(0); // language, NUL-padded to 4 bytes
    if !cancel {
        // ServUO rejects the whole response if `text.Length > 128`
        // (PacketHandlers.cs `UnicodePromptResponse`) — and .NET `string.Length`
        // counts **UTF-16 code units**, not chars. Clamping by `.chars().take(128)`
        // would let an astral (non-BMP) char — 2 units each — slip through and
        // push the unit count past 128, so ServUO would silently drop the whole
        // reply. Walk whole chars, tracking the running UTF-16 unit count, and
        // stop before a char would push it over 128; a char's units are only
        // ever added as a pair, so a surrogate pair is never split.
        let mut clamped = String::new();
        let mut units = 0usize;
        for ch in text.trim().chars() {
            let ch_units = ch.len_utf16();
            if units + ch_units > 128 {
                break;
            }
            units += ch_units;
            clamped.push(ch);
        }
        for unit in clamped.encode_utf16() {
            w.u16(unit.swap_bytes()); // UTF-16 LE (the writer is BE, so swap first)
        }
    }
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
    fn opl_request_layout() {
        // Single serial: [0xD6][len=7][serial].
        let p = build_opl_request(&[0xDEAD_BEEF]);
        assert_eq!(p[0], 0xD6);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(p, vec![0xD6, 0x00, 0x07, 0xDE, 0xAD, 0xBE, 0xEF]);

        // Batch of two serials: header + 2×u32 (BE).
        let b = build_opl_request(&[0x0102_0304, 0x0506_0708]);
        assert_eq!(u16::from_be_bytes([b[1], b[2]]) as usize, b.len());
        assert_eq!(
            b,
            vec![0xD6, 0x00, 0x0B, 1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn skill_lock_layout() {
        // Lock skill 25 (Magery) to "locked" (2).
        let p = build_skill_lock(25, 2);
        assert_eq!(p[0], 0x3A);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(p.len(), 6);
        assert_eq!(u16::from_be_bytes([p[3], p[4]]), 25); // skill id (BE)
        assert_eq!(p[5], 2); // lock state
        assert_eq!(build_skill_lock(7, 0), vec![0x3A, 0x00, 0x06, 0x00, 7, 0]);
    }

    #[test]
    fn use_skill_layout() {
        // Use skill 21 (Hiding): [0x12][len][0x24]"21 0"\0
        let p = build_use_skill(21);
        assert_eq!(p[0], 0x12);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(p[3], 0x24); // type: use skill
        assert_eq!(&p[4..p.len() - 1], b"21 0"); // ASCII command body
        assert_eq!(*p.last().unwrap(), 0); // NUL terminator
        assert_eq!(p, vec![0x12, 0x00, 0x09, 0x24, b'2', b'1', b' ', b'0', 0]);
    }

    #[test]
    fn book_page_request_shape() {
        // Request all 2 pages of book 0xDEADBEEF.
        let p = build_book_page_request(0xDEAD_BEEF, 2);
        assert_eq!(p[0], 0x66);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], &[0xDE, 0xAD, 0xBE, 0xEF]); // serial (BE)
        assert_eq!(u16::from_be_bytes([p[7], p[8]]), 2); // page count
        // page 1 / 0xFFFF, page 2 / 0xFFFF
        assert_eq!(&p[9..], &[0x00, 0x01, 0xFF, 0xFF, 0x00, 0x02, 0xFF, 0xFF]);
    }

    #[test]
    fn use_ability_layout() {
        // Arm ability 7 (Double Strike) for player 0xDEADBEEF.
        let p = build_use_ability(0xDEAD_BEEF, 7);
        assert_eq!(p[0], 0xD7);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(p.len(), 15);
        assert_eq!(&p[3..7], &[0xDE, 0xAD, 0xBE, 0xEF]); // player serial (BE)
        assert_eq!(u16::from_be_bytes([p[7], p[8]]), 0x0019); // subcommand
        assert_eq!(u32::from_be_bytes([p[9], p[10], p[11], p[12]]), 0); // zero
        assert_eq!(p[13], 7); // ability id
        assert_eq!(p[14], 0x0A); // trailer
        // Disarm sends ability 0.
        let d = build_use_ability(0x01, 0);
        assert_eq!(d, vec![0xD7, 0x00, 0x0F, 0, 0, 0, 1, 0x00, 0x19, 0, 0, 0, 0, 0, 0x0A]);
    }

    #[test]
    fn popup_request_and_select_shapes() {
        let req = build_popup_request(0xDEAD_BEEF);
        assert_eq!(
            req,
            vec![0xBF, 0x00, 0x09, 0x00, 0x13, 0xDE, 0xAD, 0xBE, 0xEF]
        );
        assert_eq!(u16::from_be_bytes([req[1], req[2]]) as usize, req.len());

        let sel = build_popup_select(0x0102_0304, 3);
        assert_eq!(
            sel,
            vec![0xBF, 0x00, 0x0B, 0x00, 0x15, 0x01, 0x02, 0x03, 0x04, 0x00, 0x03]
        );
        assert_eq!(u16::from_be_bytes([sel[1], sel[2]]) as usize, sel.len());
    }

    #[test]
    fn gump_response_layout() {
        // Button 1, one switch (id 7), one text entry (id 3 = "ok").
        let p = build_gump_response(0xDEAD_BEEF, 0x2A, 1, &[7], &[(3, "ok".into())]);
        assert_eq!(p[0], 0xB1);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], &[0xDE, 0xAD, 0xBE, 0xEF]); // serial (BE)
        assert_eq!(u32::from_be_bytes([p[7], p[8], p[9], p[10]]), 0x2A); // gumpId
        assert_eq!(u32::from_be_bytes([p[11], p[12], p[13], p[14]]), 1); // button
        assert_eq!(u32::from_be_bytes([p[15], p[16], p[17], p[18]]), 1); // switchCount
        assert_eq!(u32::from_be_bytes([p[19], p[20], p[21], p[22]]), 7); // switch
        assert_eq!(u32::from_be_bytes([p[23], p[24], p[25], p[26]]), 1); // entryCount
        assert_eq!(u16::from_be_bytes([p[27], p[28]]), 3); // entryId
        assert_eq!(u16::from_be_bytes([p[29], p[30]]), 2); // textLen (code units)
        assert_eq!(&p[31..], &[0x00, b'o', 0x00, b'k']); // UTF-16 BE "ok"

        // Cancel: button 0, no switches, no entries.
        let c = build_gump_response(0x01, 0x02, 0, &[], &[]);
        assert_eq!(
            c,
            vec![0xB1, 0x00, 0x17, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn buy_request_layout() {
        // Two items: matches ClassicUO Send_BuyRequest (flag 0x02, per-item 0x1A).
        let p = build_buy(0xAABB_CCDD, &[(0x4000_0001, 3), (0x4000_0002, 1)]);
        assert_eq!(p[0], 0x3B);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], &[0xAA, 0xBB, 0xCC, 0xDD]); // vendor (BE)
        assert_eq!(p[7], 0x02); // flag: accept-with-list
        assert_eq!(
            &p[8..],
            &[0x1A, 0x40, 0, 0, 1, 0, 3, 0x1A, 0x40, 0, 0, 2, 0, 1]
        );

        // Empty list → cancel (flag 0x00), 8 bytes total.
        let c = build_buy(0x0102_0304, &[]);
        assert_eq!(c, vec![0x3B, 0x00, 0x08, 1, 2, 3, 4, 0x00]);
    }

    #[test]
    fn sell_request_layout() {
        let p = build_sell(0xAABB_CCDD, &[(0x4000_0009, 7), (0x4000_000A, 1)]);
        assert_eq!(p[0], 0x9F);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], &[0xAA, 0xBB, 0xCC, 0xDD]); // vendor (BE)
        assert_eq!(u16::from_be_bytes([p[7], p[8]]), 2); // count
        assert_eq!(
            &p[9..],
            &[0x40, 0, 0, 9, 0, 7, 0x40, 0, 0, 0x0A, 0, 1]
        );

        // Empty list → count 0, 9 bytes total.
        let c = build_sell(0x0102_0304, &[]);
        assert_eq!(c, vec![0x9F, 0x00, 0x09, 1, 2, 3, 4, 0, 0]);
    }

    #[test]
    fn cast_spell_shape() {
        // Fireball = spell id 18. Modern 0xBF/0x001C path, 9 bytes, all BE.
        let p = build_cast_spell(18);
        assert_eq!(p.len(), 9);
        assert_eq!(p[0], 0xBF);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(u16::from_be_bytes([p[3], p[4]]), 0x001C); // subcommand
        assert_eq!(u16::from_be_bytes([p[5], p[6]]), 0x0002); // cast type
        assert_eq!(u16::from_be_bytes([p[7], p[8]]), 18); // spell id
        assert_eq!(p, vec![0xBF, 0x00, 0x09, 0x00, 0x1C, 0x00, 0x02, 0x00, 18]);
    }

    #[test]
    fn party_message_shape() {
        let p = build_party_message("hi");
        assert_eq!(p[0], 0xBF);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(u16::from_be_bytes([p[3], p[4]]), 0x0006); // subcommand
        assert_eq!(p[5], 0x04); // mode: to all
        assert_eq!(&p[6..], &[0x00, b'h', 0x00, b'i', 0x00, 0x00]); // UTF-16 BE + NUL
    }

    #[test]
    fn party_command_shapes() {
        // Invite: 0xBF/0x0006/0x01 + zero serial → server drives the target cursor.
        assert_eq!(
            build_party_invite(),
            vec![0xBF, 0x00, 0x0A, 0x00, 0x06, 0x01, 0, 0, 0, 0]
        );
        // Leave: 0xBF/0x0006/0x02 + self serial.
        assert_eq!(
            build_party_leave(0x0102_0304),
            vec![0xBF, 0x00, 0x0A, 0x00, 0x06, 0x02, 1, 2, 3, 4]
        );
        // Accept: 0xBF/0x0006/0x08 + leader serial.
        assert_eq!(
            build_party_accept(0xAABB_CCDD),
            vec![0xBF, 0x00, 0x0A, 0x00, 0x06, 0x08, 0xAA, 0xBB, 0xCC, 0xDD]
        );
        // Decline: 0xBF/0x0006/0x09 + leader serial.
        assert_eq!(
            build_party_decline(0xAABB_CCDD),
            vec![0xBF, 0x00, 0x0A, 0x00, 0x06, 0x09, 0xAA, 0xBB, 0xCC, 0xDD]
        );
    }

    #[test]
    fn action_packet_shapes() {
        assert_eq!(build_attack(0xABCD), vec![0x05, 0, 0, 0xAB, 0xCD]);
        assert_eq!(build_double_click(0x01), vec![0x06, 0, 0, 0, 1]);
        assert_eq!(build_pick_up(0x01, 5), vec![0x07, 0, 0, 0, 1, 0, 5]);
        assert_eq!(
            build_drop(0x01, 100, 200, -5, 0xFFFF_FFFF),
            vec![0x08, 0, 0, 0, 1, 0, 100, 0, 200, (-5i8) as u8, 0, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(
            build_equip(0x0102_0304, 0x15, 0x0A0B_0C0D),
            vec![0x13, 1, 2, 3, 4, 0x15, 0x0A, 0x0B, 0x0C, 0x0D]
        );
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
    fn prompt_response_layout() {
        // Reply "Rex" to prompt (serial 0xDEADBEEF, promptId 0x2A): type=1, lang
        // "ENU\0", text as UTF-16 LE (note: byte order reversed vs the rest of the
        // protocol).
        let p = build_prompt_response(0xDEAD_BEEF, 0x2A, "Rex", false);
        assert_eq!(p[0], 0xC2);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], &[0xDE, 0xAD, 0xBE, 0xEF]); // serial (BE)
        assert_eq!(u32::from_be_bytes([p[7], p[8], p[9], p[10]]), 0x2A); // promptId
        assert_eq!(u32::from_be_bytes([p[11], p[12], p[13], p[14]]), 1); // type = reply
        assert_eq!(&p[15..19], b"ENU\0"); // language
        // "Rex" as UTF-16 LE: R=0x52, e=0x65, x=0x78.
        assert_eq!(&p[19..], &[0x52, 0x00, 0x65, 0x00, 0x78, 0x00]);

        // Cancel: type=0, no text bytes at all.
        let c = build_prompt_response(0x01, 0x02, "ignored", true);
        assert_eq!(
            c,
            vec![0xC2, 0x00, 0x13, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, b'E', b'N', b'U', 0]
        );
    }

    #[test]
    fn prompt_response_clamps_by_utf16_units_not_chars() {
        // 70 astral (non-BMP) chars, each 2 UTF-16 units — 140 units total, well
        // over the 128-unit limit ServUO enforces (`PacketHandlers.cs`
        // `UnicodePromptResponse`: `text.Length > 128`, and .NET `string.Length`
        // counts UTF-16 code units, not chars). A naive `.chars().take(128)`
        // clamp would keep all 70 *chars* (140 units) and ServUO would silently
        // drop the whole reply; clamping by running UTF-16 unit count must stop
        // at exactly 64 chars (64 × 2 = 128 units) and never emit half of a
        // surrogate pair.
        let text: String = "\u{1F600}".repeat(70);
        let p = build_prompt_response(0xDEAD_BEEF, 0x2A, &text, false);
        let payload = &p[19..]; // same 19-byte header as `prompt_response_layout`
        assert_eq!(payload.len() % 2, 0); // whole u16 units only — no stray half-unit
        let unit_count = payload.len() / 2;
        assert_eq!(unit_count, 128); // 64 whole chars, none split off

        // Reassemble as UTF-16 LE: a split surrogate pair would fail to decode.
        let units: Vec<u16> = payload
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let decoded = String::from_utf16(&units).expect("must not split a surrogate pair");
        assert_eq!(decoded.chars().count(), 64);
        assert!(decoded.chars().all(|c| c == '\u{1F600}'));
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

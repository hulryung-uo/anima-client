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

/// Serials per `0xD6` request. ClassicUO's `Send_MegaClilocRequest` batches this
/// many, and every shard is tuned for that client — ServUO's
/// `BatchQueryProperties` itself reads as many serials as the length carries, so
/// this is a compatibility choice, not a protocol limit. It lives beside the
/// builder rather than in a driver because it is a property of the packet: both
/// `anima-net` and `anima-wasm` send these, and a third caller would otherwise
/// have no way to know the cap exists.
pub const OPL_REQUEST_BATCH: usize = 15;

/// MegaClilocRequest `0xD6` (variable) — ask the server for the Object Property
/// List (tooltip) of one or more entities. The server replies with a 0xD6
/// MegaCliloc per serial. Ports ClassicUO `Send_MegaClilocRequest`:
/// `[0xD6][len:u16][serial:u32]…` — a length-framed batch of serials. Callers
/// chunk by [`OPL_REQUEST_BATCH`]. Empty input still produces a well-formed
/// (header-only) packet.
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

/// CustomHouse design-details request. GeneralInfo `0xBF`, subcommand
/// `0x001E` (9 bytes, fixed). Ask ServUO to (re)send the 0xD8 design for
/// `serial`'s house foundation — ServUO only ever emits 0xD8 in reply to
/// this; the unsolicited 0xBF/0x1D revision notice never carries the design
/// itself, only a counter telling us ours is stale (see [`crate::net::game`]'s
/// 0x1D handler, which queues the serials this builds a request for).
/// `[0xBF][len:u16=0x0009][0x001E][serial:u32]`.
pub fn build_house_design_request(serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(9); // id + fixed length (always 9 bytes for this subcommand)
    w.u16(0x001E); // subcommand: request custom house design details
    w.u32(serial);
    w.into_vec()
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

/// 0x73 Ping — `[0x73][seq]` (2 bytes). A keepalive heartbeat: the server echoes
/// it back (incoming 0x73), and sending it periodically resets the server's
/// idle-disconnect timer so an idle client isn't dropped (ClassicUO `Send_Ping`).
pub fn build_ping(seq: u8) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x73).u8(seq);
    w.into_vec()
}

/// 0xC8 ClientViewRange — `[0xC8][range]` (2 bytes). Tells the server our desired
/// draw range in tiles (clamped to UO's 5..=24); the server echoes it (incoming
/// 0xC8) and uses it to decide which mobiles/items fall in view. Sent on world
/// entry, mirroring ClassicUO `Send_ClientViewRange`.
pub fn build_client_view_range(range: u8) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xC8).u8(range.clamp(5, 24));
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

/// `MessageType.Encoded` — set on 0xAD when the body carries packed `speech.mul`
/// keyword ids. ServUO's `UnicodeSpeech` keys `e.Keywords` off this bit.
pub const SPEECH_ENCODED: u8 = 0xC0;

/// Pack keyword ids the way ClassicUO `Send_UnicodeSpeechRequest` does, which
/// is the exact nibble layout ServUO's `UnicodeSpeech` unpacks:
/// first byte `count >> 4`, then alternating 12-bit ids stuffed through a
/// leftover nibble. Cap is 50 — ServUO drops the packet above that.
pub fn pack_speech_keywords(ids: &[u16]) -> Vec<u8> {
    let len = ids.len().min(50);
    let mut code = Vec::new();
    code.push((len >> 4) as u8);
    let mut num3 = (len & 15) as u16;
    let mut flag = false;
    for &id in ids.iter().take(len) {
        if flag {
            code.push((id >> 4) as u8);
            num3 = id & 15;
        } else {
            code.push(((num3 << 4) | ((id >> 8) & 15)) as u8);
            code.push(id as u8);
        }
        flag = !flag;
    }
    if !flag {
        code.push((num3 << 4) as u8);
    }
    code
}

/// Unpack [`pack_speech_keywords`] the way ServUO `UnicodeSpeech` does, so the
/// builder test can prove the two sides agree.
#[cfg(test)]
fn unpack_speech_keywords(bytes: &[u8]) -> Option<Vec<u16>> {
    if bytes.len() < 2 {
        return None;
    }
    let value = u16::from_be_bytes([bytes[0], bytes[1]]);
    let count = ((value & 0xFFF0) >> 4) as usize;
    let mut hold = value & 0xF;
    if count > 50 {
        return None;
    }
    let mut p = 2usize;
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let speech_id = if i % 2 == 0 {
            if p >= bytes.len() {
                return None;
            }
            hold = (hold << 8) | bytes[p] as u16;
            p += 1;
            let id = hold;
            hold = 0;
            id
        } else {
            if p + 2 > bytes.len() {
                return None;
            }
            let value = u16::from_be_bytes([bytes[p], bytes[p + 1]]);
            p += 2;
            let id = (value & 0xFFF0) >> 4;
            hold = value & 0xF;
            id
        };
        ids.push(speech_id);
    }
    Some(ids)
}

/// UnicodeSpeech `0xAD` (variable): say `text` in-game as UNICODE so non-ASCII
/// (e.g. Korean/한글) survives. When `keyword_ids` is non-empty the type byte
/// gets [`SPEECH_ENCODED`] and the body is packed ids + UTF-8 + NUL (ClassicUO
/// `Send_UnicodeSpeechRequest`); otherwise UTF-16 BE + 0x0000.
/// `[0xAD][len u16][type u8][hue u16][font u16][lang 4=ASCII "ENU\0"][…]`.
pub fn build_unicode_say(
    text: &str,
    msg_type: u8,
    hue: u16,
    font: u16,
    keyword_ids: &[u16],
) -> Vec<u8> {
    let clamped: String = text.trim().chars().take(128).collect();
    let mut w = PacketWriter::new();
    w.u8(0xAD).u16(0); // id + length placeholder
    if keyword_ids.is_empty() {
        w.u8(msg_type).u16(hue).u16(font);
        w.bytes(b"ENU\0");
        for unit in clamped.encode_utf16() {
            w.u16(unit);
        }
        w.u16(0x0000);
    } else {
        w.u8(msg_type | SPEECH_ENCODED).u16(hue).u16(font);
        w.bytes(b"ENU\0");
        w.bytes(&pack_speech_keywords(keyword_ids));
        w.bytes(clamped.as_bytes()).u8(0);
    }
    finish_variable(w.into_vec())
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

/// Finalize a variable-framed packet: patch the big-endian length word at
/// `[1..3]` now that every field has been written. Used by any variable
/// builder whose fields are all fixed-width (a batch/text builder that needs
/// per-item length math inlines the patch itself instead).
fn finish_variable(mut data: Vec<u8>) -> Vec<u8> {
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
    finish_variable(w.into_vec())
}

/// Remove `member` from the party. GeneralInfo `0xBF`, subcommand `0x0006`,
/// sub-sub `0x02`, then the serial to drop. Ported from ClassicUO
/// `Send_PartyRemoveRequest`: `[0xBF][len u16][0x0006][0x02][serial u32]`.
///
/// **Leaving and kicking are the same packet** — the only difference is whose
/// serial you name, and ServUO enforces the distinction rather than the wire
/// format doing it: `PartyCommands.OnRemove` accepts the request when
/// `p.Leader == from || from == target`, so anyone may remove themselves and
/// only the leader may remove anyone else. A non-leader naming someone else is
/// silently ignored, not an error. See [`build_party_leave`] for the
/// remove-yourself spelling.
pub fn build_party_remove(member: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x02).u32(member);
    finish_variable(w.into_vec())
}

/// Leave the party (remove ourself) — [`build_party_remove`] naming our own
/// serial. Kept as its own name because "leave" is what the call sites mean.
pub fn build_party_leave(self_serial: u32) -> Vec<u8> {
    build_party_remove(self_serial)
}

/// Private party message to one member. GeneralInfo `0xBF`, subcommand
/// `0x0006`, sub-sub `0x03`, then the recipient's serial and the text as
/// UNICODE (UTF-16 BE) NUL-terminated — the same text encoding
/// [`build_party_message`] uses for the to-all form.
/// `[0xBF][len u16][0x0006][0x03][target serial u32][utf16-be…][0x0000]`.
///
/// ServUO's `PartyCommands.OnPrivateMessage` drops the message outright when
/// the trimmed text is empty or longer than 128 characters, so clamp here
/// rather than let a long line silently vanish (same rule and same clamp as
/// the to-all builder).
pub fn build_party_private_message(member: u32, text: &str) -> Vec<u8> {
    let clamped: String = text.trim().chars().take(128).collect();
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x03).u32(member);
    for unit in clamped.encode_utf16() {
        w.u16(unit);
    }
    w.u16(0x0000); // UNICODE NUL terminator
    finish_variable(w.into_vec())
}

/// Allow or forbid the rest of the party looting our corpse. GeneralInfo
/// `0xBF`, subcommand `0x0006`, sub-sub `0x06`, then a boolean byte.
/// `[0xBF][len u16][0x0006][0x06][canLoot u8]`.
///
/// The flag is per-member and lives server-side on `PartyMemberInfo.CanLoot`;
/// ServUO acknowledges it only as a journal line (cliloc 1005447 allow /
/// 1005448 prevent) and never sends the state back, so a client that wants to
/// show a checkbox has to remember what it asked for.
pub fn build_party_can_loot(can_loot: bool) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x06).u8(u8::from(can_loot));
    finish_variable(w.into_vec())
}

/// Accept a party invitation. GeneralInfo `0xBF`, subcommand `0x0006`, sub-sub
/// `0x08`, then the inviting leader's serial. Ported from ClassicUO
/// `Send_PartyAccept`: `[0xBF][len u16][0x0006][0x08][leader serial u32]`.
pub fn build_party_accept(leader: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x08).u32(leader);
    finish_variable(w.into_vec())
}

/// Decline a party invitation. GeneralInfo `0xBF`, subcommand `0x0006`, sub-sub
/// `0x09`, then the inviting leader's serial. Ported from ClassicUO
/// `Send_PartyDecline`: `[0xBF][len u16][0x0006][0x09][leader serial u32]`.
pub fn build_party_decline(leader: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0006);
    w.u8(0x09).u32(leader);
    finish_variable(w.into_vec())
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

/// MenuResponse `0x7D` (13 bytes) — answer a legacy 0x7C item/question menu.
/// `index` is one-based; zero cancels. Item menus echo the selected entry's
/// graphic/hue, while question menus and cancel responses use zeros.
/// `[0x7D][serial:u32][menu_id:u16][index:u16][graphic:u16][hue:u16]`.
pub fn build_legacy_menu_response(
    serial: u32,
    menu_id: u16,
    index: u16,
    graphic: u16,
    hue: u16,
) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x7D)
        .u32(serial)
        .u16(menu_id)
        .u16(index)
        .u16(graphic)
        .u16(hue);
    w.into_vec()
}

/// HuePickerResponse `0x95` (9 bytes) — answer a server DisplayHuePicker.
/// ServUO masks hue flags then applies `Utility.ClipDyedHue`, so mirror that
/// normalization locally: the ordinary dye palette is exactly `2..=1001`.
/// `[0x95][picker_serial:u32][reserved:u16=0][hue:u16]`.
pub fn build_hue_picker_response(serial: u32, hue: u16) -> Vec<u8> {
    let hue = (hue & 0x3FFF).clamp(2, 1001);
    let mut w = PacketWriter::new();
    w.u8(0x95).u32(serial).u16(0).u16(hue);
    w.into_vec()
}

/// Request the previous/next page from a pageable 0xA6 TipWindow. ClassicUO
/// sends fixed packet 0xA7 as `[id][tip:u16][direction:u8]`, truncating the
/// inbound 32-bit tip id to its low 16 bits; direction 0 = previous, 1 = next.
pub fn build_tip_request(tip: u32, next: bool) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xA7).u16(tip as u16).u8(next as u8);
    w.into_vec()
}

/// BookPageRequest `0x66` (variable): ask the server to send every page of the
/// open book `serial`. `[0x66][len:u16][serial:u32][pageCount:u16=N]` then, for
/// each page `1..=N`, `[pageNum:u16][lineCount:u16=0xFFFF]` — the `0xFFFF` line
/// count is the "send me this page" sentinel (ClassicUO `Send_BookPageDataRequest`).
/// The server replies with one or more 0x66 BookData packets.
///
/// **Inert against ServUO**, which registers 0x66 to `ContentChange` alone: the
/// request's `0xFFFF` line count fails that handler's `lineCount <= 8` check
/// and the packet is dropped. Nothing is lost, because ServUO never needs
/// asking — `BaseBook.OnDoubleClick` sends `BookHeader` *and*
/// `BookPageDetails` for every page unprompted. Kept for shards that do honour
/// the request form; see [`build_book_page_write`] for the same id's other
/// meaning.
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
    w.u32(player_serial)
        .u16(0x0019)
        .u32(0)
        .u8(ability_id)
        .u8(0x0A);
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// DisarmRequest — GeneralInfo `0xBF`, subcommand `0x0009`, no payload.
///
/// The **pre-AOS** twin of [`build_use_ability`]. Before weapon special moves
/// existed, a Wrestling character armed the next punch with the Disarm or Stun
/// special; ClassicUO still routes the ability bar to these when the shard did
/// not advertise AOS (`GameActions.SendAbility`: `if AOS flag is clear` →
/// `Send_StunRequest`/`Send_DisarmRequest`). ServUO's handler toggles
/// `Mobile.DisarmReady` and answers with cliloc 1019013/1019014, so the client
/// gets its feedback through the journal rather than a dedicated packet.
///
/// **The subcommand numbering here departs from ClassicUO, deliberately.**
/// ClassicUO's `Send_StunRequest` writes `0x09` and its `Send_DisarmRequest`
/// writes `0x0A` — the two are swapped relative to the servers. ServUO
/// registers `RegisterExtended(0x09, true, DisarmRequest)` /
/// `RegisterExtended(0x0A, true, StunRequest)`
/// (`Server/Network/PacketHandlers.cs`), and Razor — an assistant with two
/// decades of use against production shards — writes the same pairing
/// (`Razor/Network/Packets.cs`: `DisarmRequest` → `0x09`, `StunRequest` →
/// `0x0A`). Two independent implementations of the *receiving* side agree, so
/// ClassicUO is the outlier; sending its way would silently arm the wrong
/// special. Layout: `[0xBF][len:u16=0x0005][0x0009]`.
pub fn build_disarm_request() -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0009);
    finish_variable(w.into_vec())
}

/// ToggleGargoyleFlying — GeneralInfo `0xBF`, subcommand `0x0032`.
///
/// The one racial ability that is a *button*: every human and elf trait is
/// passive, and of the gargoyle's five only Flying is used rather than merely
/// possessed (ClassicUO's `RacialAbilitiesBookGump` marks the other four
/// "Passive" and hangs a double-click handler on this one alone).
///
/// Layout is ClassicUO's `Send_ToggleGargoyleFlying` verbatim:
/// `[0xBF][len:u16=0x000B][0x0032][0x0001][0x00000000]` — a constant `1` and
/// four zero bytes ServUO never reads (`ToggleFlying` takes the packet and
/// ignores its body), kept because the wire format is not ours to trim.
///
/// ServUO answers by race and nothing else: `PlayerMobile.ToggleFlying`
/// returns immediately unless `Race == Race.Gargoyle`, then casts/cancels
/// `FlySpell`. There is no AOS or expansion gate on the handler itself.
pub fn build_toggle_flying() -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0032).u16(0x0001).u32(0);
    finish_variable(w.into_vec())
}

/// Confirm a heritage / race-change dialog (0xBF/0x2A). ClassicUO
/// `Send_ChangeRaceRequest`: `[0xBF][len:u16=15][0x002A][skinHue:u16]
/// [hairStyle:u16][hairHue:u16][beardStyle:u16][beardHue:u16]`. ServUO
/// `HeritageTransform` accepts this 15-byte shape and applies the hues/ids
/// when a heritage quester or race-change token is pending. Female and elf
/// bodies send beard style/hue 0.
pub fn build_change_race_request(
    skin_hue: u16,
    hair_style: u16,
    hair_hue: u16,
    beard_style: u16,
    beard_hue: u16,
) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x002A);
    w.u16(skin_hue)
        .u16(hair_style)
        .u16(hair_hue)
        .u16(beard_style)
        .u16(beard_hue);
    finish_variable(w.into_vec())
}

/// Cancel a heritage / race-change dialog (0xBF/0x2A with no payload).
/// ServUO `HeritageTransform` treats a 5-byte packet as cancel and answers
/// cliloc 1073645 ("You may try this again later…").
pub fn build_change_race_cancel() -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x002A);
    finish_variable(w.into_vec())
}

/// Open the Ultima Store (0xFA). Fixed 1-byte packet — ClassicUO
/// `Send_OpenUOStore` / ServUO `UltimaStore.UOStoreRequest`.
pub fn build_open_uo_store() -> Vec<u8> {
    vec![0xFA]
}

/// StunRequest — GeneralInfo `0xBF`, subcommand `0x000A`, no payload. The Stun
/// half of the pre-AOS pair; see [`build_disarm_request`] for why the
/// subcommand is `0x0A` and not ClassicUO's `0x09`. ServUO gates it on
/// Anatomy + Wrestling ≥ 80 and toggles `Mobile.StunReady`.
/// Layout: `[0xBF][len:u16=0x0005][0x000A]`.
pub fn build_stun_request() -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x000A);
    finish_variable(w.into_vec())
}

/// BandageTarget — GeneralInfo `0xBF`, subcommand `0x002C`: apply `bandage` to
/// `target` in a single packet, with no target cursor round-trip.
///
/// This is the whole point of the subcommand. The ordinary path is
/// double-click the bandages → server sends 0x6C asking for a target → reply
/// 0x6C — three messages and a cursor the player (or brain) has to hold. ServUO
/// `BandageTarget` instead takes both serials at once and raises
/// `BandageTargetRequest` directly, which is what makes reliable self-healing
/// under pressure possible. It is still rate-limited server-side by
/// `Mobile.NextActionTime`, so spamming it just draws "You must wait…".
///
/// `target` may be our own serial (bandage self). Ported from ClassicUO
/// `Send_TargetSelectedObject`:
/// `[0xBF][len:u16=0x000D][0x002C][bandage:u32][target:u32]`.
pub fn build_bandage_target(bandage: u32, target: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x002C);
    w.u32(bandage).u32(target);
    finish_variable(w.into_vec())
}

/// TargetedSpell — GeneralInfo `0xBF`, subcommand `0x002D`: cast `spell` at
/// `target` in a single packet, with no target cursor round-trip.
///
/// Same idea as [`build_bandage_target`]. ServUO `TargetedSpell` reads a
/// 1-based spell id (subtracts 1, matching [`build_cast_spell`]) then a
/// serial. `[0xBF][len:u16=0x000B][0x002D][spell:u16][target:u32]`.
pub fn build_targeted_spell(spell: u16, target: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x002D);
    w.u16(spell).u32(target);
    finish_variable(w.into_vec())
}

/// TargetedSkillUse — GeneralInfo `0xBF`, subcommand `0x002E`: use `skill`
/// on `target` in a single packet.
///
/// `skill` is 0-based (Alchemy = 0), the same numbering [`build_use_skill`]
/// uses — ServUO does **not** subtract 1 here. `[0xBF][len:u16=0x000B]
/// [0x002E][skill:u16][target:u32]`.
pub fn build_targeted_skill(skill: u16, target: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x002E);
    w.u16(skill).u32(target);
    finish_variable(w.into_vec())
}

/// TargetByResourceMacro — GeneralInfo `0xBF`, subcommand `0x0030`: harvest
/// with `tool` for `resource`.
///
/// ServUO `HarvestSystem.TargetByResource`: 0 ore, 1 sand, 2 wood, 3 grave,
/// 4 red mushrooms. `[0xBF][len:u16=0x000B][0x0030][tool:u32][resource:u16]`.
pub fn build_target_by_resource(tool: u32, resource: u16) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0030);
    w.u32(tool).u16(resource);
    finish_variable(w.into_vec())
}

/// Boat pilot speed: one step per slow tick (ClassicUO sends this for a walk).
pub const BOAT_SPEED_SLOW: u8 = 1;
/// Boat pilot speed: fast tick (ClassicUO sends this for a run).
pub const BOAT_SPEED_FAST: u8 = 2;
/// Boat pilot speed meaning "stop".
///
/// Any value ServUO does not recognise stops the ship rather than erroring:
/// `GetMovementInterval(int speed, …)`'s `default` arm yields `clientSpeed = 0`,
/// `StartMove` refuses a zero client speed, and `OnMousePilotCommand` answers
/// that refusal with `StopMove`. So 0 is the honest spelling of a stop, and
/// there is no such thing as speed 3.
pub const BOAT_SPEED_STOP: u8 = 0;

/// MultiBoatMoveRequest — GeneralInfo `0xBF`, subcommand `0x0033`: steer the
/// boat the player is currently piloting.
/// `[0xBF][len:u16][0x0033][playerSerial:u32][dir:u8][dir:u8][speed:u8]`.
///
/// **The serial is the PLAYER's, not the boat's** — ServUO's
/// `MultiMouseMovementRequest` does `World.FindMobile(playerSerial)` and then
/// reaches the ship through `mob.Mount`. ClassicUO passes `_world.Player` for
/// the same reason. Sending the boat's serial finds no mobile and the packet
/// is dropped in silence.
///
/// The direction byte is written **twice** (movement, then facing); ServUO
/// reads the second and discards it, and ClassicUO writes the same value for
/// both. It is an ordinary 0..7 UO direction, the same space
/// [`build_walk_request`] uses — ClassicUO derives it with the identical
/// `facing - 1` expression it feeds to `Player.Walk`, so there is no offset to
/// apply here.
///
/// **This only works while piloting.** The handler returns unless the player
/// is `Mounted` on a `BaseBoat`, which happens when `LockPilot` attaches a
/// `BoatMountItem` (graphic `0x3E96`, worn on the Mount layer) — entered by
/// double-clicking the tiller man or a ship wheel. A brain must do that first;
/// nothing reports the omission.
///
/// The other boat-control path is tiller-man speech ("forward", "stop"):
/// `BaseBoat.OnSpeech` dispatches on `e.Keywords`, which this client now
/// fills by matching `speech.mul` and sending 0xAD with [`SPEECH_ENCODED`].
/// Mouse piloting remains the held-key path; speech is the typed one.
pub fn build_boat_move_request(player_serial: u32, dir: u8, speed: u8) -> Vec<u8> {
    let d = dir & 0x07;
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0033);
    w.u32(player_serial).u8(d).u8(d).u8(speed);
    finish_variable(w.into_vec())
}

/// Longest title ServUO will accept, in UTF-8 **bytes** (`HeaderChange`:
/// `if (titleLength > 60) return`).
const BOOK_TITLE_MAX: usize = 60;
/// Longest author, same units and the same all-or-nothing check.
const BOOK_AUTHOR_MAX: usize = 30;
/// Lines per page ServUO accepts (`if (lineCount <= 8)`, else the page — and
/// with it the rest of the packet — is dropped).
pub const BOOK_LINES_PER_PAGE: usize = 8;
/// Longest line, in UTF-16 code units as ServUO counts `string.Length`
/// (`if (...).Length >= 80) return` — note `>=`, so 79 is the real maximum).
const BOOK_LINE_MAX: usize = 79;

/// Truncate `s` to at most `max` UTF-8 bytes without splitting a character.
fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// BookHeaderChange `0xD4` (variable) — rewrite an open book's title/author.
///
/// Layout: `[0xD4][len:u16][serial:u32][0x00][0x00][0x0000][titleLen:u16]
/// [title UTF-8][authorLen:u16][author UTF-8]`. The four bytes after the serial
/// are the flags/page-count field ServUO seeks straight past
/// (`pvSrc.Seek(4, SeekOrigin.Current)`), so their value is irrelevant; we
/// write ClassicUO's zeros. Neither string is NUL-terminated — each is read
/// back with `ReadUTF8StringSafe(len)` using the length that precedes it.
///
/// **The lengths are byte counts and over-long is fatal, not truncating**:
/// ServUO answers `titleLength > 60` (or `authorLength > 30`) with a bare
/// `return`, discarding the whole packet, so a caller who sends a long title
/// loses the author edit too and hears nothing about it. Both are clamped here
/// on character boundaries. ServUO also refuses the edit outright unless the
/// book is `Writable`, within 1 tile, and accessible — none of which is
/// reported back either.
pub fn build_book_header_change(serial: u32, title: &str, author: &str) -> Vec<u8> {
    let title = truncate_utf8(title, BOOK_TITLE_MAX);
    let author = truncate_utf8(author, BOOK_AUTHOR_MAX);
    let mut w = PacketWriter::new();
    w.u8(0xD4).u16(0); // id + length placeholder
    w.u32(serial);
    w.u8(0x00).u8(0x00).u16(0x0000); // flags + page count: skipped server-side
    w.u16(title.len() as u16).bytes(title.as_bytes());
    w.u16(author.len() as u16).bytes(author.as_bytes());
    finish_variable(w.into_vec())
}

/// BookPageWrite `0x66` (variable) — send the edited text of ONE page.
///
/// Layout: `[0x66][len:u16][serial:u32][0x0001][page:u16][lineCount:u16]` then
/// each line as NUL-terminated UTF-8, then one trailing `0x00`. `page` is
/// 1-based (ServUO checks `index >= 1 && index <= PagesCount` before
/// decrementing). The leading `0x0001` is a page *count* — the wire can carry
/// several pages per packet, and ClassicUO always sends exactly one, so this
/// does too.
///
/// **Same all-or-nothing validation as the header.** ServUO drops the entire
/// packet if a page carries more than 8 lines, if any line's `.Length` reaches
/// 80, or if the page index is out of range — always silently. Lines are
/// clamped to [`BOOK_LINES_PER_PAGE`] and [`BOOK_LINE_MAX`] here so a long
/// paragraph costs its own tail rather than the whole page. Embedded newlines
/// are stripped, as ClassicUO does, because a line break inside a line would
/// desynchronize the NUL-terminated list.
///
/// **This is the same packet id as [`build_book_page_request`]**, distinguished
/// only by the line count — see that builder's note.
pub fn build_book_page_write(serial: u32, page: u16, lines: &[String]) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x66).u16(0); // id + length placeholder
    w.u32(serial).u16(1).u16(page);
    let lines: Vec<String> = lines
        .iter()
        .take(BOOK_LINES_PER_PAGE)
        .map(|l| {
            let flat: String = l.chars().filter(|c| *c != '\n' && *c != '\r').collect();
            flat.chars().take(BOOK_LINE_MAX).collect()
        })
        .collect();
    w.u16(lines.len() as u16);
    for line in &lines {
        w.bytes(line.as_bytes()).u8(0); // NUL-terminated UTF-8
    }
    w.u8(0); // ClassicUO's trailing byte; ServUO stops after `lineCount` lines
    finish_variable(w.into_vec())
}

/// MapCommand `0x56` (fixed 11 bytes) — edit the pins on map item `serial`.
/// `[0x56][serial:u32][command:u8][number:u8][x:i16][y:i16]`.
///
/// **The same packet id and shape travel both ways with different command
/// meanings**, the trap OpenShard's `findings.md` names for `0x22`. Here
/// client command `5` is ClearPins while the server's `5` is "display this
/// map"; our incoming decoder ([`crate::world::World::apply_map_command`]) owns
/// the server's reading and these builders own the client's. Ports ServUO
/// `MapItem.OnMapCommand`'s dispatch.
///
/// `x`/`y` are in the map's own pixel space (`MapView::width`/`height`), not
/// world coordinates; ServUO clamps them into range with `Validate(ref x, ref
/// y)`, so an out-of-bounds pin lands on the edge rather than being refused.
fn build_map_command(serial: u32, command: u8, number: u8, x: u16, y: u16) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x56).u32(serial).u8(command).u8(number).u16(x).u16(y);
    w.into_vec()
}

/// Toggle map `serial` between view and edit mode (command `6`).
///
/// **Required before any other pin edit.** ServUO gates all five mutators on
/// `ValidateEdit`, which is `m_Editable && Validate(from)`, and `m_Editable`
/// starts false — so an add/remove sent to a map still in view mode is
/// discarded without a reply. This command is the only thing that flips it.
///
/// Unlike the mutators, it *is* answered: the server replies with a `0x56`
/// command `7` (`MapSetEditable`) carrying the resulting state, which is why
/// the driver applies the others optimistically but leaves this one to the
/// echo. Note ServUO also re-sends `7` whenever the map is displayed, and its
/// `Validate(from)` refuses a map that is not in reach, is protected, or
/// belongs to somebody else — so a toggle can legitimately answer "still not
/// editable".
pub fn build_map_toggle_editable(serial: u32) -> Vec<u8> {
    build_map_command(serial, 6, 0, 0, 0)
}

/// Append a pin at `(x, y)` (command `1`). ServUO caps a map at
/// `MaxUserPins = 50` and silently ignores the request past that.
pub fn build_map_add_pin(serial: u32, x: u16, y: u16) -> Vec<u8> {
    build_map_command(serial, 1, 0, x, y)
}

/// Insert a pin at `index` (command `2`). ServUO's `InsertPin` **appends**
/// when the index is out of range rather than refusing, so this can never fail
/// on a bad index — it just moves the pin to the end.
pub fn build_map_insert_pin(serial: u32, index: u8, x: u16, y: u16) -> Vec<u8> {
    build_map_command(serial, 2, index, x, y)
}

/// Move the pin at `index` to `(x, y)` (command `3`). Out-of-range indices are
/// ignored server-side.
pub fn build_map_change_pin(serial: u32, index: u8, x: u16, y: u16) -> Vec<u8> {
    build_map_command(serial, 3, index, x, y)
}

/// Remove the pin at `index` (command `4`).
///
/// **Index 0 cannot be removed this way**: ServUO's `RemovePin` guards with
/// `index > 0`, which on a decoded treasure map protects the chest pin — the
/// one pin that is the point of the map. [`build_map_clear_pins`] has no such
/// guard and *will* take it, so "clear" is not "remove each in turn".
pub fn build_map_remove_pin(serial: u32, index: u8) -> Vec<u8> {
    build_map_command(serial, 4, index, 0, 0)
}

/// Remove every pin (command `5`), including index 0 — `ClearPins` carries no
/// `index > 0` guard, so this is genuinely not "remove each in turn" (see
/// [`build_map_remove_pin`]).
///
/// **On a treasure map the clear does not survive the next open.**
/// `TreasureMap.DisplayTo` ends with `if (Pins.Count == 0)
/// AddWorldPin(ChestLocation)`, so a pinless treasure map re-grows its chest
/// pin the moment it is displayed again — the map refuses to forget where the
/// treasure is. Measured live: the pin list read `[]` right after the clear and
/// `[[169, 184]]` after re-opening the same map. An ordinary `MapItem` has no
/// such rule and stays empty.
pub fn build_map_clear_pins(serial: u32) -> Vec<u8> {
    build_map_command(serial, 5, 0, 0, 0)
}

/// HelpRequest `0x9B` (fixed 258 bytes) — open the shard's help / GM-page
/// menu. The 257-byte payload is **entirely ignored**: ServUO's handler reads
/// nothing at all and just raises `HelpRequest`, and ClassicUO writes 257
/// zeros. Kept as zeros for wire parity rather than trimmed, because the
/// packet's registered length is fixed and a short one desynchronizes the
/// stream. Layout: `[0x9B][0x00 × 257]`.
pub fn build_help_request() -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x9B).zeros(257);
    w.into_vec()
}

/// Click the on-screen quest arrow — GeneralInfo `0xBF`, subcommand `0x0007`,
/// then a boolean: `false` = left click, `true` = right click.
///
/// The arrow is server-owned state (0xBA sets and clears it); this reports
/// that we clicked it, and ServUO forwards the click to whatever placed it
/// (`Mobile.QuestArrow.OnClick`), which is typically what dismisses it.
/// A click with no arrow outstanding is a harmless no-op server-side.
/// Layout: `[0xBF][len:u16=0x0006][0x0007][rightClick:u8]`. Ported from
/// ClassicUO `Send_ClickQuestArrow`.
pub fn build_quest_arrow_click(right_click: bool) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x0007);
    w.u8(u8::from(right_click));
    finish_variable(w.into_vec())
}

/// Ask the server to open the guild menu — `0xD7` subcommand `0x28`, no
/// arguments (ServUO `GuildGumpRequest`). The reply is an ordinary server
/// gump, so nothing new is needed to *read* it.
pub fn build_guild_menu_request(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x28, &[])
}

/// Ask the server to open the quest menu — `0xD7` subcommand `0x32`, no
/// arguments (ServUO `QuestGumpRequest`), answered with an ordinary gump.
///
/// Note a harmless divergence: ClassicUO's `Send_QuestMenuRequest` ends this
/// one with `0x00` where its own guild twin (and every other `0xD7` it sends)
/// ends with `0x0A`. We write `0x0A` for both, because ServUO's
/// `EncodedCommand` dispatches on the subcommand and hands the rest to a
/// handler that reads nothing — neither byte is ever looked at.
pub fn build_quest_menu_request(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x32, &[])
}

/// Shared framing for the `0xD7` GenericAOS family — ServUO's `RegisterEncoded`
/// table (same packet id as [`build_use_ability`] above, one subcommand space
/// per feature) — carrying zero or more encoded `Int32` arguments.
///
/// Was `build_house_design`, and renamed when the guild/quest gump requests
/// turned out to want the identical framing: the house designer is the biggest
/// user of this shape, not the only one.
///
/// ServUO's `EncodedReader::ReadInt32` (used by every `HouseFoundation`
/// designer handler) requires a type byte of `0` before the 4-byte
/// big-endian value; the other encoded types it defines (`2` = string, `3` =
/// `Point3D`) are never used by anything we send, so this only needs to
/// emit the `Int32` shape. Layout:
/// `[0xD7][len:u16][playerSerial:u32][subcmd:u16]([0x00][arg:u32])*[0x0A]`.
fn build_encoded_command(player_serial: u32, subcmd: u16, args: &[u32]) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xD7).u16(0); // id + length placeholder
    w.u32(player_serial).u16(subcmd);
    for &arg in args {
        w.u8(0x00).u32(arg); // encoded Int32: type byte 0, then value BE
    }
    w.u8(0x0A);
    let mut data = w.into_vec();
    let len = data.len() as u16;
    data[1] = (len >> 8) as u8;
    data[2] = (len & 0xFF) as u8;
    data
}

/// CustomHouse designer: Backup the in-progress design. Sub-command `0x02`
/// of ServUO's `HouseFoundation.RegisterEncoded` table; no arguments. Ports
/// ClassicUO `Send_CustomHouseBackup`.
pub fn build_house_design_backup(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x02, &[])
}

/// CustomHouse designer: Restore the last backed-up design. Sub-command
/// `0x03`; no arguments. Ports ClassicUO `Send_CustomHouseRestore`.
pub fn build_house_design_restore(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x03, &[])
}

/// CustomHouse designer: Commit the design (finalize and leave edit mode).
/// Sub-command `0x04`; no arguments. Ports ClassicUO `Send_CustomHouseCommit`.
pub fn build_house_design_commit(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x04, &[])
}

/// CustomHouse designer: delete a placed component at `(x, y, z)`. Sub-command
/// `0x05`, args `[graphic, x, y, z]`. Ports ClassicUO `Send_CustomHouseDeleteItem`
/// (`ServUO Designer_Delete` reads the same four `Int32`s in this order).
pub fn build_house_design_delete_item(
    player_serial: u32,
    graphic: u16,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<u8> {
    build_encoded_command(
        player_serial,
        0x05,
        &[graphic as u32, x as u32, y as u32, z as u32],
    )
}

/// CustomHouse designer: add (build) a component at `(x, y)`. Sub-command
/// `0x06`, args `[graphic, x, y]`. Ports ClassicUO `Send_CustomHouseAddItem`
/// (`ServUO Designer_Build`) — the canonical 0xD7 example this file's
/// house-designer builders are modeled on.
pub fn build_house_design_add_item(player_serial: u32, graphic: u16, x: i32, y: i32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x06, &[graphic as u32, x as u32, y as u32])
}

/// CustomHouse designer: response/acknowledge. Sub-command `0x0A`; no
/// arguments. Ports ClassicUO `Send_CustomHouseResponse`; ServUO registers it
/// as `Designer_Action` with the comment "WTF does this do?" — included here
/// for completeness since it is part of the same designer flow.
pub fn build_house_design_response(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x0A, &[])
}

/// CustomHouse designer: close the designer / exit the building. Sub-command
/// `0x0C`; no arguments. Ports ClassicUO `Send_CustomHouseBuildingExit`.
pub fn build_house_design_close(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x0C, &[])
}

/// CustomHouse designer: add a stair component at `(x, y)`. Sub-command
/// `0x0D`, args `[graphic, x, y]`. Ports ClassicUO `Send_CustomHouseAddStair`
/// (`ServUO Designer_Stairs`).
pub fn build_house_design_add_stair(player_serial: u32, graphic: u16, x: i32, y: i32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x0D, &[graphic as u32, x as u32, y as u32])
}

/// CustomHouse designer: request a full state re-sync. Sub-command `0x0E`;
/// no arguments. Ports ClassicUO `Send_CustomHouseSync`.
pub fn build_house_design_sync(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x0E, &[])
}

/// CustomHouse designer: clear the entire design back to the foundation.
/// Sub-command `0x10`; no arguments. Ports ClassicUO `Send_CustomHouseClear`.
pub fn build_house_design_clear(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x10, &[])
}

/// CustomHouse designer: switch the editor's active floor. Sub-command
/// `0x12`, arg `[floor]`. Ports ClassicUO `Send_CustomHouseGoToFloor`
/// (`ServUO Designer_Level`, which clamps `newLevel` to `1..=MaxLevels`).
pub fn build_house_design_go_to_floor(player_serial: u32, floor: u8) -> Vec<u8> {
    build_encoded_command(player_serial, 0x12, &[floor as u32])
}

/// CustomHouse designer: add a roof component at `(x, y, z)` (Samurai Empire
/// roofs). Sub-command `0x13`, args `[graphic, x, y, z]`. Ports ClassicUO
/// `Send_CustomHouseAddRoof` (`ServUO Designer_Roof`).
pub fn build_house_design_add_roof(
    player_serial: u32,
    graphic: u16,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<u8> {
    build_encoded_command(
        player_serial,
        0x13,
        &[graphic as u32, x as u32, y as u32, z as u32],
    )
}

/// CustomHouse designer: delete a roof component at `(x, y, z)`. Sub-command
/// `0x14`, args `[graphic, x, y, z]`. Ports ClassicUO `Send_CustomHouseDeleteRoof`
/// (`ServUO Designer_RoofDelete`).
pub fn build_house_design_delete_roof(
    player_serial: u32,
    graphic: u16,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<u8> {
    build_encoded_command(
        player_serial,
        0x14,
        &[graphic as u32, x as u32, y as u32, z as u32],
    )
}

/// CustomHouse designer: revert the whole design (undo back to the last
/// commit). Sub-command `0x1A`; no arguments. Ports ClassicUO
/// `Send_CustomHouseRevert`.
pub fn build_house_design_revert(player_serial: u32) -> Vec<u8> {
    build_encoded_command(player_serial, 0x1A, &[])
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

/// StatLock — change which way a stat trains. GeneralInfo `0xBF`, subcommand
/// `0x001A`. Ports ClassicUO `Send_StatLockStateRequest`:
/// `[0xBF][len:u16][0x001A][stat:u8][state:u8]`.
///
/// `stat` is 0 = Strength, 1 = Dexterity, 2 = Intelligence. `state` uses the
/// same `Lock` values as [`build_skill_lock`] — 0 = up (raise), 1 = down
/// (lower), 2 = locked — which is why the incoming half of this pair decodes
/// into `PlayerStats::{str,dex,int}_lock` with exactly those meanings.
pub fn build_stat_lock(stat: u8, lock: u8) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xBF).u16(0).u16(0x001A);
    w.u8(stat).u8(lock);
    finish_variable(w.into_vec())
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

/// ASCII 0x12 ActionRequest: `[0x12][len][type][body][NUL]`. Shared by
/// skill-use, cast-from-book, emote, and virtue — ClassicUO's `WriteASCII`
/// always appends the terminator.
fn build_ascii_action(typ: u8, body: &str) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x12).u16(0);
    w.u8(typ);
    w.bytes(body.as_bytes()).u8(0);
    finish_variable(w.into_vec())
}

/// OpenDoor `0x12` type `0x58` — ClassicUO `Send_OpenDoor`. The server opens
/// the door on the tile the player is facing; there is no serial on the wire.
/// Layout: `[0x12][len:u16=5][0x58][0x00]`.
pub fn build_open_door() -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x12).u16(0).u8(0x58).u8(0x00);
    finish_variable(w.into_vec())
}

/// CastSpellFromBook `0x12` type `0x27`. ClassicUO `Send_CastSpellFromBook`.
/// `spell` is 1-based (ServUO subtracts 1, same as [`build_cast_spell`]).
/// Layout: `[0x12][len][0x27]["<spell> <bookSerial>"][0x00]`.
pub fn build_cast_spell_from_book(spell: u16, book: u32) -> Vec<u8> {
    build_ascii_action(0x27, &format!("{spell} {book}"))
}

/// EmoteAction `0x12` type `0xC7` — ClassicUO `Send_EmoteAction` (bow, salute, …).
/// ServUO's Animate handler takes the ASCII verb. Clamped to 32 bytes so a
/// runaway string cannot bloat the packet.
pub fn build_emote_action(action: &str) -> Vec<u8> {
    let clamped: String = action.trim().chars().take(32).collect();
    build_ascii_action(0xC7, &clamped)
}

/// InvokeVirtue `0x12` type `0xF4`. ClassicUO `Send_InvokeVirtueRequest`.
/// `id` is 1-based (Honor=1 … Spirituality=8); ServUO subtracts 1.
pub fn build_invoke_virtue(id: u8) -> Vec<u8> {
    build_ascii_action(0xF4, &id.to_string())
}

/// EquipLastWeapon `0xD7` subcommand `0x1E`. ClassicUO `Send_EquipLastWeapon`.
/// Layout: `[0xD7][len][playerSerial:u32][0x001E][0x0A]`.
pub fn build_equip_last_weapon(player_serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xD7).u16(0).u32(player_serial).u16(0x001E).u8(0x0A);
    finish_variable(w.into_vec())
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

/// ASCIIPromptResponse `0x9A` (variable) — answer (or cancel) a pending legacy
/// 0x9A server prompt. The two opaque ids and `type` have the same meaning as
/// the Unicode response, but the trailing string is ClassicUO's CP1252 encoding
/// plus a NUL terminator: `[0x9A][len:u16][serial:u32][promptId:u32][type:u32]
/// [text:cp1252][0]`. ServUO rejects responses longer than 128 decoded chars,
/// so the payload is clamped before encoding. A cancel always carries an empty
/// string, matching ClassicUO's `CancelServerPrompt` path.
pub fn build_ascii_prompt_response(
    serial: u32,
    prompt_id: u32,
    text: &str,
    cancel: bool,
) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x9A).u16(0);
    w.u32(serial).u32(prompt_id);
    w.u32(if cancel { 0 } else { 1 });
    if !cancel {
        for ch in text.trim().chars().take(128) {
            w.u8(unicode_to_cp1252(ch));
        }
    }
    w.u8(0);
    finish_variable(w.into_vec())
}

/// TextEntryDialogResponse `0xAC` (variable) — answer one 0xAB legacy modal.
/// The callback tuple is echoed verbatim, `accepted` is ClassicUO's `code`
/// byte, and the entered text is sent for both OK and Cancel. Text is CP1252,
/// NUL-padded to `(UTF-16 code units + 1)` exactly as ClassicUO's
/// `WriteASCII(text, text.Length + 1)` does. Variant 2 admits only numeric
/// characters; a positive server maximum is enforced in UTF-16 units, while
/// zero means unlimited up to the protocol's u16 packet-size ceiling.
#[allow(clippy::too_many_arguments)]
pub fn build_text_entry_dialog_response(
    serial: u32,
    parent_id: u8,
    button_id: u8,
    text: &str,
    accepted: bool,
    variant: u8,
    max_length: u32,
) -> Vec<u8> {
    const MAX_TEXT_UNITS: usize = u16::MAX as usize - 13;
    let server_limit = if max_length == 0 {
        MAX_TEXT_UNITS
    } else {
        usize::try_from(max_length)
            .unwrap_or(usize::MAX)
            .min(MAX_TEXT_UNITS)
    };
    let mut encoded = Vec::new();
    let mut units = 0usize;
    for ch in text.chars() {
        if variant == 2 && !ch.is_numeric() {
            continue;
        }
        let ch_units = ch.len_utf16();
        if units + ch_units > server_limit {
            break;
        }
        units += ch_units;
        encoded.push(unicode_to_cp1252(ch));
    }

    let wire_text_len = units + 1;
    let mut w = PacketWriter::new();
    w.u8(0xAC)
        .u16(0)
        .u32(serial)
        .u8(parent_id)
        .u8(button_id)
        .u8(accepted as u8)
        .u16(wire_text_len as u16)
        .bytes(&encoded);
    for _ in encoded.len()..wire_text_len {
        w.u8(0);
    }
    finish_variable(w.into_vec())
}

/// Request a player's character profile with 0xB8 type 0. ServUO validates
/// range/visibility and answers with the same opcode's display shape.
/// `[0xB8][len:u16=8][type=0][serial:u32]`.
pub fn build_profile_request(serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xB8).u16(0).u8(0).u32(serial);
    finish_variable(w.into_vec())
}

/// Update a character profile with 0xB8 type 1. ClassicUO writes an opaque
/// `u16=1`, then a UTF-16-code-unit count and UTF-16BE text. ServUO rejects a
/// body over 511 units, so clamp a whole-character prefix without splitting a
/// surrogate pair: `[id][len][1][serial][1:u16][units:u16][utf16be...]`.
pub fn build_profile_update(serial: u32, text: &str) -> Vec<u8> {
    const MAX_PROFILE_UNITS: usize = 511;
    let mut units = Vec::new();
    for ch in text.chars() {
        let needed = ch.len_utf16();
        if units.len() + needed > MAX_PROFILE_UNITS {
            break;
        }
        let mut encoded = [0u16; 2];
        units.extend_from_slice(ch.encode_utf16(&mut encoded));
    }

    let mut w = PacketWriter::new();
    w.u8(0xB8)
        .u16(0)
        .u8(1)
        .u32(serial)
        .u16(1)
        .u16(units.len() as u16);
    for unit in units {
        w.u16(unit);
    }
    finish_variable(w.into_vec())
}

/// Ask the server for permission to log out. ClassicUO sends the fixed packet
/// `[0xD1, 0x00]`; a compliant server answers with the same opcode and a
/// non-zero allow byte before the client disconnects.
pub fn build_logout_request() -> Vec<u8> {
    vec![0xD1, 0x00]
}

/// Match ClassicUO `StringHelper.UnicodeToCp1252`. The C1 control range is
/// deliberately replaced with `?`; printable Windows-1252 punctuation maps to
/// its extension byte, and code points outside the repertoire also become `?`.
fn unicode_to_cp1252(ch: char) -> u8 {
    let code = ch as u32;
    if (0x80..=0x9F).contains(&code) {
        return b'?';
    }
    if code <= 0xFF {
        return code as u8;
    }
    match code {
        0x20AC => 128, // €
        0x201A => 130, // ‚
        0x0192 => 131, // ƒ
        0x201E => 132, // „
        0x2026 => 133, // …
        0x2020 => 134, // †
        0x2021 => 135, // ‡
        0x02C6 => 136, // ˆ
        0x2030 => 137, // ‰
        0x0160 => 138, // Š
        0x2039 => 139, // ‹
        0x0152 => 140, // Œ
        0x017D => 142, // Ž
        0x2018 => 145, // ‘
        0x2019 => 146, // ’
        0x201C => 147, // “
        0x201D => 148, // ”
        0x2022 => 149, // •
        0x2013 => 150, // –
        0x2014 => 151, // —
        0x02DC => 152, // ˜
        0x2122 => 153, // ™
        0x0161 => 154, // š
        0x203A => 155, // ›
        0x0153 => 156, // œ
        0x017E => 158, // ž
        0x0178 => 159, // Ÿ
        _ => b'?',
    }
}

/// SecureTrade `0x6F` (variable), action `1` Cancel — cancel the open trade
/// window; items on both sides return to their owners. `my_container` is
/// always the CALLER's own trade-container serial (ClassicUO `TradingGump`
/// only ever sends its own `ID1`, never the opponent's `ID2`; ServUO's
/// `PacketHandlers.SecureTrade` looks up the session from whichever
/// container it's given, so either would technically work, but we mirror the
/// reference client). Layout: `[0x6F][len:u16][0x01][myContainer:u32]`.
pub fn build_trade_cancel(my_container: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x6F).u16(0); // id + length placeholder
    w.u8(0x01).u32(my_container);
    finish_variable(w.into_vec())
}

/// SecureTrade `0x6F` (variable), action `2` Check — toggle our side's accept
/// checkbox. Layout: `[0x6F][len:u16][0x02][myContainer:u32][accepted:u32]`.
pub fn build_trade_accept(my_container: u32, accept: bool) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x6F).u16(0);
    w.u8(0x02).u32(my_container).u32(accept as u32);
    finish_variable(w.into_vec())
}

/// SecureTrade `0x6F` (variable), action `3` Update Gold — set the virtual
/// gold/platinum amount we're offering (ServUO `SecureTrade.From.Gold`/
/// `.Plat`; only visibly reflected to either side when the AOS/TOL "account
/// gold" feature is negotiated — see [`crate::world::TradeState`]'s doc).
/// Layout: `[0x6F][len:u16][0x03][myContainer:u32][gold:u32][platinum:u32]`.
pub fn build_trade_gold(my_container: u32, gold: u32, platinum: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x6F).u16(0);
    w.u8(0x03).u32(my_container).u32(gold).u32(platinum);
    finish_variable(w.into_vec())
}

/// NameRequest `0x98` (variable, 7 bytes) — ask the server for `serial`'s name.
/// The server replies with the same opcode (incoming `0x98` UpdateName).
/// Ports ClassicUO `Send_NameRequest`: despite the fixed 4-byte body, ClassicUO's
/// packet-length table marks `0x98` dynamic (`-1`), so it is framed with the
/// standard `[id][len:u16]` header rather than sent as a bare fixed packet.
/// Layout: `[0x98][len:u16=0x0007][serial:u32]`.
pub fn build_name_request(serial: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x98).u16(0); // id + length placeholder
    w.u32(serial);
    finish_variable(w.into_vec())
}

/// RenameRequest `0x75` (fixed, 35 bytes) — rename a pet/hireling `serial`.
/// Ports ClassicUO `Send_RenameRequest`: the packet-length table lists `0x75`
/// as a fixed `0x0023` (35) bytes, so — unlike the variable builders in this
/// file — there is **no** length field; `name` is `WriteASCII(name, 30)`, a
/// CP1252-encoded, NUL-padded 30-byte field (truncated if longer).
/// Layout: `[0x75][serial:u32][name: 30 bytes, CP1252, NUL-padded]`.
pub fn build_rename_request(serial: u32, name: &str) -> Vec<u8> {
    const NAME_WIDTH: usize = 30;
    let mut w = PacketWriter::new();
    w.u8(0x75).u32(serial);
    let mut written = 0usize;
    for ch in name.chars() {
        if written >= NAME_WIDTH {
            break;
        }
        w.u8(unicode_to_cp1252(ch));
        written += 1;
    }
    for _ in written..NAME_WIDTH {
        w.u8(0);
    }
    w.into_vec()
}

/// BulletinBoardRequestMessage `0x71` (variable), sub-command `0x03` — ask the
/// bulletin board `board` to send the full body of message `msg` (server
/// replies with incoming `0x71`). Ports ClassicUO
/// `Send_BulletinBoardRequestMessage`.
/// Layout: `[0x71][len:u16=0x000C][0x03][board:u32][msg:u32]`.
pub fn build_bulletin_request_message(board: u32, msg: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x71).u16(0); // id + length placeholder
    w.u8(0x03).u32(board).u32(msg);
    finish_variable(w.into_vec())
}

/// BulletinBoardRequestMessageSummary `0x71`, sub-command `0x04` — ask for the
/// summary (author/subject/time, no body) of message `msg` on `board`. Ports
/// ClassicUO `Send_BulletinBoardRequestMessageSummary`.
/// Layout: `[0x71][len:u16=0x000C][0x04][board:u32][msg:u32]`.
pub fn build_bulletin_request_summary(board: u32, msg: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x71).u16(0); // id + length placeholder
    w.u8(0x04).u32(board).u32(msg);
    finish_variable(w.into_vec())
}

/// BulletinBoardPostMessage `0x71`, sub-command `0x05` — post a new message
/// (or, when `reply_to != 0`, a reply) to bulletin board `board`.
///
/// Ports ClassicUO `Send_BulletinBoardPostMessage`, which is called with the
/// caller's *unsplit* body text and splits it on `\n` itself; here the caller
/// passes the already-split `lines` directly.
///
/// **One deliberate departure: the subject's length prefix is its UTF-8 byte
/// count, not ClassicUO's UTF-16 code-unit count.** ClassicUO writes
/// `subject.Length + 1` — .NET's char count — ahead of UTF-8 bytes, which
/// agrees only for ASCII. ServUO reads it with
/// `ReadUTF8StringSafe(pvSrc.ReadByte())`, a **byte** count, so a non-ASCII
/// subject is both truncated and desynchronizes everything after it: measured
/// live, `항해일지` (4 characters, 12 bytes, declared as 5) came back from the
/// board as `항` plus half a character. This builder already used the byte
/// count for each *line*, so following ClassicUO here made one field disagree
/// with its own neighbours for no reason. The server defines the format; a
/// client that cannot post a non-ASCII subject is simply broken.
/// Layout: `[0x71][len:u16][0x05][board:u32][replyTo:u32]
/// [subjectLen:u8][subject:utf8][0x00][lineCount:u8]
/// {[lineLen:u8][line:utf8][0x00]}…`.
pub fn build_bulletin_post_message(
    board: u32,
    reply_to: u32,
    subject: &str,
    lines: &[&str],
) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x71).u16(0); // id + length placeholder
    w.u8(0x05).u32(board).u32(reply_to);

    // UTF-8 byte count + the NUL, matching what ServUO's `ReadUTF8StringSafe`
    // consumes — and matching the per-line prefixes below (see the doc).
    let subject = subject.as_bytes();
    w.u8((subject.len() + 1) as u8);
    w.bytes(subject).u8(0);

    w.u8(lines.len() as u8);
    for line in lines {
        let encoded = line.as_bytes();
        w.u8((encoded.len() + 1) as u8);
        w.bytes(encoded).u8(0);
    }

    finish_variable(w.into_vec())
}

/// BulletinBoardRemoveMessage `0x71`, sub-command `0x06` — delete message
/// `msg` from bulletin board `board` (only the poster/board owner may
/// succeed; the server silently ignores unauthorized requests). Ports
/// ClassicUO `Send_BulletinBoardRemoveMessage`.
/// Layout: `[0x71][len:u16=0x000C][0x06][board:u32][msg:u32]`.
pub fn build_bulletin_remove_message(board: u32, msg: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x71).u16(0); // id + length placeholder
    w.u8(0x06).u32(board).u32(msg);
    finish_variable(w.into_vec())
}

/// OpenChat `0xB5` (fixed, 64 bytes) — open the chat window, optionally
/// pre-filling a conversation/channel `name`. Ports ClassicUO `Send_OpenChat`:
/// the packet-length table lists `0xB5` as a fixed `0x0040` (64) bytes, so
/// (like `0x75` above) there is no length field. `name` is UTF-16BE, capped
/// at 30 code units, written with **no** explicit NUL terminator — the fixed
/// packet is simply zero-padded out to 64 bytes afterward, which reads back
/// as an implicit terminator.
/// Layout: `[0xB5][0x00][name:utf16-be, ≤30 units][zero-pad to 64 bytes]`.
pub fn build_chat_open(name: &str) -> Vec<u8> {
    const TOTAL_LEN: usize = 64;
    let mut w = PacketWriter::new();
    w.u8(0xB5).u8(0x00);
    for unit in name.encode_utf16().take(30) {
        w.u16(unit);
    }
    let mut data = w.into_vec();
    data.resize(TOTAL_LEN, 0);
    data
}

/// The chat-command language tag ClassicUO writes ahead of every `0xB3`
/// sub-command: a fixed 4-byte ASCII field, NUL-padded. ClassicUO reads this
/// from `Settings.GlobalSettings.Language`, which `Main.cs` always sets to
/// `"ENU"` at startup, so it is effectively a constant.
const CHAT_LANGUAGE: &[u8; 4] = b"ENU\0";

/// ChatJoinCommand `0xB3` (variable) — join (or create-and-join, if it
/// doesn't yet exist) chat channel `channel`, with optional `password`. Ports
/// ClassicUO `Send_ChatJoinCommand`. The channel name is wrapped in literal
/// `"` (0x0022) quote code units followed by a `0x0020` space, then the
/// password (if any) follows as its own NUL-terminated UTF-16BE string —
/// exactly the raw code-unit sequence ClassicUO writes, quirks included.
/// Layout: `[0xB3][len:u16][lang:4]["ENU"][0x0062][0x0022]
/// [channel:utf16-be][0x0000][0x0022][0x0020]{[password:utf16-be][0x0000]}?`.
pub fn build_chat_join(channel: &str, password: &str) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xB3).u16(0); // id + length placeholder
    w.bytes(CHAT_LANGUAGE);
    w.u16(0x0062); // sub-command: join channel
    w.u16(0x0022); // opening quote
    for unit in channel.encode_utf16() {
        w.u16(unit);
    }
    w.u16(0x0000); // NUL terminator (ClassicUO's WriteUnicodeBE appends this)
    w.u16(0x0022); // closing quote
    w.u16(0x0020); // space
    if !password.is_empty() {
        for unit in password.encode_utf16() {
            w.u16(unit);
        }
        w.u16(0x0000);
    }
    finish_variable(w.into_vec())
}

/// ChatCreateChannelCommand `0xB3` (variable) — create (and join) a new chat
/// channel `channel`, with optional `password`. Ports ClassicUO
/// `Send_ChatCreateChannelCommand`. Unlike join, the channel name here is
/// unquoted; only the password (if any) is wrapped, in literal `{`/`}`
/// (0x007B/0x007D) code units.
/// Layout: `[0xB3][len:u16][lang:4]["ENU"][0x0063][channel:utf16-be][0x0000]
/// {[0x007B][password:utf16-be][0x0000][0x007D]}?`.
pub fn build_chat_create_channel(channel: &str, password: &str) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xB3).u16(0); // id + length placeholder
    w.bytes(CHAT_LANGUAGE);
    w.u16(0x0063); // sub-command: create channel
    for unit in channel.encode_utf16() {
        w.u16(unit);
    }
    w.u16(0x0000);
    if !password.is_empty() {
        w.u16(0x007B); // '{'
        for unit in password.encode_utf16() {
            w.u16(unit);
        }
        w.u16(0x0000);
        w.u16(0x007D); // '}'
    }
    finish_variable(w.into_vec())
}

/// ChatLeaveChannelCommand `0xB3` (variable), sub-command `0x0043` — leave the
/// current chat channel. Ports ClassicUO `Send_ChatLeaveChannelCommand`; no
/// payload beyond the language tag and sub-command word.
/// Layout: `[0xB3][len:u16=0x0009][lang:4]["ENU"][0x0043]`.
pub fn build_chat_leave() -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xB3).u16(0); // id + length placeholder
    w.bytes(CHAT_LANGUAGE);
    w.u16(0x0043); // sub-command: leave channel
    finish_variable(w.into_vec())
}

/// ChatMessageCommand `0xB3` (variable), sub-command `0x0061` — send `text`
/// to the currently-joined chat channel. Ports ClassicUO
/// `Send_ChatMessageCommand`.
/// Layout: `[0xB3][len:u16][lang:4]["ENU"][0x0061][text:utf16-be][0x0000]`.
pub fn build_chat_message(text: &str) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xB3).u16(0); // id + length placeholder
    w.bytes(CHAT_LANGUAGE);
    w.u16(0x0061); // sub-command: chat message
    for unit in text.encode_utf16() {
        w.u16(unit);
    }
    w.u16(0x0000);
    finish_variable(w.into_vec())
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
        assert_eq!(b, vec![0xD6, 0x00, 0x0B, 1, 2, 3, 4, 5, 6, 7, 8]);
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
    fn open_door_and_related_action_request_layouts() {
        assert_eq!(build_open_door(), vec![0x12, 0x00, 0x05, 0x58, 0x00]);

        let from_book = build_cast_spell_from_book(8, 0x4000_0001);
        assert_eq!(from_book[0], 0x12);
        assert_eq!(from_book[3], 0x27);
        assert_eq!(&from_book[4..from_book.len() - 1], b"8 1073741825");
        assert_eq!(*from_book.last().unwrap(), 0);

        let emote = build_emote_action("bow");
        assert_eq!(emote, vec![0x12, 0x00, 0x08, 0xC7, b'b', b'o', b'w', 0]);

        let virtue = build_invoke_virtue(1);
        assert_eq!(virtue, vec![0x12, 0x00, 0x06, 0xF4, b'1', 0]);

        let last = build_equip_last_weapon(0x0102_0304);
        assert_eq!(
            last,
            vec![0xD7, 0x00, 0x0A, 0x01, 0x02, 0x03, 0x04, 0x00, 0x1E, 0x0A]
        );
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
        assert_eq!(
            d,
            vec![0xD7, 0x00, 0x0F, 0, 0, 0, 1, 0x00, 0x19, 0, 0, 0, 0, 0, 0x0A]
        );
    }

    #[test]
    fn boat_move_request_layout() {
        let p = build_boat_move_request(0xDEAD_BEEF, 2, BOAT_SPEED_FAST);
        assert_eq!(
            p,
            vec![0xBF, 0x00, 0x0C, 0x00, 0x33, 0xDE, 0xAD, 0xBE, 0xEF, 2, 2, 2]
        );
        // The direction is written TWICE (movement, then facing) and ServUO
        // discards the second — dropping it would shift the speed byte into
        // the facing slot and read the speed as 0, i.e. a silent stop.
        assert_eq!(p[9], p[10]);
        // Out-of-range directions wrap rather than corrupting the facing byte.
        let w = build_boat_move_request(1, 9, BOAT_SPEED_SLOW);
        assert_eq!((w[9], w[10]), (1, 1));
        // Stop is speed 0; there is no speed 3 (see BOAT_SPEED_STOP).
        assert_eq!(build_boat_move_request(1, 0, BOAT_SPEED_STOP)[11], 0);
        assert_eq!((BOAT_SPEED_SLOW, BOAT_SPEED_FAST), (1, 2));
    }

    #[test]
    fn book_header_change_layout_and_clamps() {
        let p = build_book_header_change(0xDEAD_BEEF, "Title", "Me");
        assert_eq!(p[0], 0xD4);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(&p[7..11], &[0, 0, 0, 0]); // flags + page count, skipped
        assert_eq!(u16::from_be_bytes([p[11], p[12]]), 5); // titleLen (bytes)
        assert_eq!(&p[13..18], b"Title"); // no NUL — the length precedes it
        assert_eq!(u16::from_be_bytes([p[18], p[19]]), 2);
        assert_eq!(&p[20..22], b"Me");

        // Over-long is FATAL server-side (`if (titleLength > 60) return`), so
        // the clamp is what keeps a long title from also losing the author.
        let long = build_book_header_change(1, &"x".repeat(200), &"y".repeat(200));
        assert_eq!(u16::from_be_bytes([long[11], long[12]]), 60);
        assert_eq!(u16::from_be_bytes([long[73], long[74]]), 30);

        // Clamping counts BYTES but must not split a character: 'é' is 2 bytes,
        // so 60 bytes holds 30 of them and never a half.
        let multi = build_book_header_change(1, &"é".repeat(100), "");
        let n = u16::from_be_bytes([multi[11], multi[12]]) as usize;
        assert_eq!(n, 60);
        assert!(std::str::from_utf8(&multi[13..13 + n]).is_ok());
    }

    #[test]
    fn book_page_write_layout_and_clamps() {
        let lines = vec!["one".to_string(), "two".to_string()];
        let p = build_book_page_write(0x0000_ABCD, 3, &lines);
        assert_eq!(p[0], 0x66);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(u32::from_be_bytes([p[3], p[4], p[5], p[6]]), 0x0000_ABCD);
        assert_eq!(u16::from_be_bytes([p[7], p[8]]), 1); // one page per packet
        assert_eq!(u16::from_be_bytes([p[9], p[10]]), 3); // 1-based page
        assert_eq!(u16::from_be_bytes([p[11], p[12]]), 2); // line count
        assert_eq!(&p[13..], b"one\0two\0\0"); // NUL-terminated + trailer

        // A ninth line or an 80-character line makes ServUO drop the WHOLE
        // packet, so both are clamped rather than passed through.
        let many: Vec<String> = (0..20).map(|i| i.to_string()).collect();
        let p = build_book_page_write(1, 1, &many);
        assert_eq!(u16::from_be_bytes([p[11], p[12]]), 8);
        let long = vec!["z".repeat(200)];
        let p = build_book_page_write(1, 1, &long);
        assert_eq!(p.len() - 15, 79, "line clamped to 79 chars + its NUL");

        // An embedded newline would desynchronize the NUL-terminated list.
        let p = build_book_page_write(1, 1, &["a\nb".to_string()]);
        assert_eq!(&p[13..], b"ab\0\0");
    }

    #[test]
    fn map_commands_are_the_fixed_eleven_byte_packet() {
        let p = build_map_add_pin(0xDEAD_BEEF, 0x0102, 0x0304);
        assert_eq!(p.len(), 11, "ServUO registers 0x56 at a fixed 11");
        assert_eq!(
            p,
            vec![0x56, 0xDE, 0xAD, 0xBE, 0xEF, 1, 0, 0x01, 0x02, 0x03, 0x04]
        );
        // Command ids, per ServUO `MapItem.OnMapCommand`'s dispatch.
        assert_eq!(build_map_add_pin(1, 0, 0)[5], 1);
        assert_eq!(build_map_insert_pin(1, 2, 0, 0)[5], 2);
        assert_eq!(build_map_change_pin(1, 2, 0, 0)[5], 3);
        assert_eq!(build_map_remove_pin(1, 2)[5], 4);
        assert_eq!(build_map_clear_pins(1)[5], 5);
        assert_eq!(build_map_toggle_editable(1)[5], 6);
        // The index rides in `number`, and change/insert must carry coordinates
        // — dropping them is an easy slip that only shows as pins snapping to
        // the map's top-left corner.
        let c = build_map_change_pin(1, 7, 0x1122, 0x3344);
        assert_eq!(c[6], 7);
        assert_eq!(u16::from_be_bytes([c[7], c[8]]), 0x1122);
        assert_eq!(u16::from_be_bytes([c[9], c[10]]), 0x3344);
    }

    #[test]
    fn chat_open_is_the_fixed_64_byte_packet() {
        let p = build_chat_open("Anima");
        assert_eq!(p.len(), 64);
        assert_eq!(&p[..2], &[0xB5, 0x00]);
        // UTF-16 BE name, then zero padding out to the fixed length.
        assert_eq!(&p[2..12], &[0, b'A', 0, b'n', 0, b'i', 0, b'm', 0, b'a']);
        assert!(p[12..].iter().all(|&b| b == 0));
        // A long name is truncated rather than overflowing the fixed packet.
        assert_eq!(build_chat_open(&"x".repeat(80)).len(), 64);
    }

    #[test]
    fn chat_join_and_create_spell_the_same_argument_differently() {
        // Not a copy-paste slip: ServUO's `JoinChannel` parses quotes out of the
        // parameter while `CreateChannel` uses it verbatim as the channel name,
        // so one convention for both would either lose the join or create a
        // channel called `"foo"`.
        let j = build_chat_join("foo", "");
        assert_eq!(j[0], 0xB3);
        assert_eq!(u16::from_be_bytes([j[1], j[2]]) as usize, j.len());
        assert_eq!(&j[3..7], b"ENU\0"); // language tag
        assert_eq!(u16::from_be_bytes([j[7], j[8]]), 0x0062); // action: join
        assert_eq!(u16::from_be_bytes([j[9], j[10]]), 0x0022); // opening quote

        let c = build_chat_create_channel("foo", "");
        assert_eq!(u16::from_be_bytes([c[7], c[8]]), 0x0063); // action: create
        assert_eq!(u16::from_be_bytes([c[9], c[10]]), 0x0066); // 'f' — unquoted
    }

    #[test]
    fn chat_leave_and_say_carry_their_action_ids() {
        let l = build_chat_leave();
        assert_eq!(u16::from_be_bytes([l[7], l[8]]), 0x0043);
        assert_eq!(l.len(), 9, "leave takes no parameter");
        let m = build_chat_message("hi");
        assert_eq!(u16::from_be_bytes([m[7], m[8]]), 0x0061);
    }

    #[test]
    fn help_request_is_the_fixed_258_byte_packet() {
        // ServUO registers 0x9B at a FIXED 258 and reads none of it; the 257
        // zero bytes exist only so the stream stays in sync. Trimming them
        // would desynchronize everything after this packet, not fail locally.
        let p = build_help_request();
        assert_eq!(p.len(), 258);
        assert_eq!(p[0], 0x9B);
        assert!(p[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn quest_arrow_click_carries_the_button() {
        assert_eq!(
            build_quest_arrow_click(false),
            vec![0xBF, 0x00, 0x06, 0x00, 0x07, 0x00]
        );
        assert_eq!(
            build_quest_arrow_click(true),
            vec![0xBF, 0x00, 0x06, 0x00, 0x07, 0x01]
        );
    }

    #[test]
    fn guild_and_quest_menu_requests_are_bare_encoded_commands() {
        // `[0xD7][len:u16][playerSerial:u32][subcmd:u16][0x0A]` = 10 bytes.
        assert_eq!(
            build_guild_menu_request(0xDEAD_BEEF),
            vec![0xD7, 0x00, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x28, 0x0A]
        );
        // ClassicUO ends its quest variant with 0x00 instead of 0x0A; ServUO
        // reads neither, so we stay consistent with every other 0xD7 we send.
        assert_eq!(
            build_quest_menu_request(0xDEAD_BEEF),
            vec![0xD7, 0x00, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x32, 0x0A]
        );
    }

    #[test]
    fn party_remove_and_leave_are_the_same_packet() {
        // Not an accident worth hiding: the wire has one "remove member"
        // message and ServUO decides leave-vs-kick from whose serial it names
        // (`PartyCommands.OnRemove`: `p.Leader == from || from == target`).
        assert_eq!(
            build_party_remove(0xDEAD_BEEF),
            vec![0xBF, 0x00, 0x0A, 0x00, 0x06, 0x02, 0xDE, 0xAD, 0xBE, 0xEF]
        );
        assert_eq!(build_party_leave(0x1234), build_party_remove(0x1234));
    }

    #[test]
    fn party_private_message_layout_and_clamp() {
        let p = build_party_private_message(0x0000_ABCD, "hi");
        assert_eq!(p[0], 0xBF);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(u16::from_be_bytes([p[3], p[4]]), 0x0006); // subcommand
        assert_eq!(p[5], 0x03); // sub-sub: private message
        assert_eq!(u32::from_be_bytes([p[6], p[7], p[8], p[9]]), 0x0000_ABCD);
        // UTF-16 BE text then a UNICODE NUL, like the to-all builder.
        assert_eq!(&p[10..], &[0x00, b'h', 0x00, b'i', 0x00, 0x00]);

        // ServUO DROPS an over-long or empty message rather than truncating it
        // (`OnPrivateMessage`: `if (text.Length > 128 || trimmed.Length == 0) return`),
        // so clamping here is what keeps a long line from vanishing silently.
        let long = build_party_private_message(1, &"x".repeat(200));
        assert_eq!(long.len(), 10 + 128 * 2 + 2);
    }

    #[test]
    fn party_can_loot_layout() {
        assert_eq!(
            build_party_can_loot(true),
            vec![0xBF, 0x00, 0x07, 0x00, 0x06, 0x06, 0x01]
        );
        assert_eq!(
            build_party_can_loot(false),
            vec![0xBF, 0x00, 0x07, 0x00, 0x06, 0x06, 0x00]
        );
    }

    #[test]
    fn status_request_is_the_fixed_ten_byte_query() {
        // ServUO registers 0x34 with a FIXED length of 10, so any drift here
        // desynchronizes the whole stream rather than failing locally.
        let p = build_status_request(4, 0x0000_018A);
        assert_eq!(p.len(), 10);
        assert_eq!(
            p,
            vec![0x34, 0xED, 0xED, 0xED, 0xED, 0x04, 0x00, 0x00, 0x01, 0x8A]
        );
    }

    #[test]
    fn stun_and_disarm_use_the_servers_subcommand_numbering() {
        // Exact bytes, because the numbering is the whole point: ClassicUO's
        // `Send_StunRequest` writes 0x09 and its `Send_DisarmRequest` writes
        // 0x0A, the reverse of what ServUO registers and what Razor sends. If
        // these ever get "corrected" toward ClassicUO, the client silently arms
        // the wrong special — no error, just the other move.
        assert_eq!(build_disarm_request(), vec![0xBF, 0x00, 0x05, 0x00, 0x09]);
        assert_eq!(build_stun_request(), vec![0xBF, 0x00, 0x05, 0x00, 0x0A]);
    }

    #[test]
    fn toggle_flying_layout() {
        // ClassicUO's Send_ToggleGargoyleFlying byte for byte: subcommand
        // 0x0032, then a constant 1 and four zeros ServUO's handler never
        // reads. 11 bytes total, and the length field must say so.
        assert_eq!(
            build_toggle_flying(),
            vec![0xBF, 0x00, 0x0B, 0x00, 0x32, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn bandage_target_layout() {
        // 0xBF/0x2C: bandage serial then target serial, both BE.
        let p = build_bandage_target(0x4000_1234, 0x0000_ABCD);
        assert_eq!(p[0], 0xBF);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(p.len(), 13);
        assert_eq!(u16::from_be_bytes([p[3], p[4]]), 0x002C); // subcommand
        assert_eq!(u32::from_be_bytes([p[5], p[6], p[7], p[8]]), 0x4000_1234);
        assert_eq!(u32::from_be_bytes([p[9], p[10], p[11], p[12]]), 0x0000_ABCD);
    }

    #[test]
    fn targeted_spell_skill_resource_layout() {
        let p = build_targeted_spell(1, 0x0000_ABCD);
        assert_eq!(p[0], 0xBF);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(p.len(), 11);
        assert_eq!(u16::from_be_bytes([p[3], p[4]]), 0x002D);
        assert_eq!(u16::from_be_bytes([p[5], p[6]]), 1);
        assert_eq!(u32::from_be_bytes([p[7], p[8], p[9], p[10]]), 0x0000_ABCD);

        let p = build_targeted_skill(1, 0x0000_ABCE);
        assert_eq!(p.len(), 11);
        assert_eq!(u16::from_be_bytes([p[3], p[4]]), 0x002E);
        assert_eq!(u16::from_be_bytes([p[5], p[6]]), 1);
        assert_eq!(u32::from_be_bytes([p[7], p[8], p[9], p[10]]), 0x0000_ABCE);

        let p = build_target_by_resource(0x4000_1234, 2);
        assert_eq!(p.len(), 11);
        assert_eq!(u16::from_be_bytes([p[3], p[4]]), 0x0030);
        assert_eq!(u32::from_be_bytes([p[5], p[6], p[7], p[8]]), 0x4000_1234);
        assert_eq!(u16::from_be_bytes([p[9], p[10]]), 2);
    }

    #[test]
    fn house_design_commit_layout() {
        // No-arg command: [0xD7][len][serial][subcmd][0x0A] = 10 bytes.
        let p = build_house_design_commit(0xDEAD_BEEF);
        assert_eq!(p[0], 0xD7);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(
            p,
            vec![0xD7, 0x00, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x04, 0x0A]
        );
    }

    #[test]
    fn house_design_add_item_layout() {
        // AddItem (build): subcmd 0x06, 3 encoded Int32 args (graphic, x, y).
        let p = build_house_design_add_item(0xDEAD_BEEF, 0x0064, 5, -3);
        assert_eq!(p[0], 0xD7);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(
            p,
            vec![
                0xD7, 0x00, 0x19, // id + len (25)
                0xDE, 0xAD, 0xBE, 0xEF, // player serial
                0x00, 0x06, // subcmd: AddItem
                0x00, 0x00, 0x00, 0x00, 0x64, // graphic 0x0064
                0x00, 0x00, 0x00, 0x00, 0x05, // x = 5
                0x00, 0xFF, 0xFF, 0xFF, 0xFD, // y = -3 (two's complement)
                0x0A,
            ]
        );
    }

    #[test]
    fn house_design_delete_item_layout() {
        // DeleteItem: subcmd 0x05, 4 encoded Int32 args (graphic, x, y, z).
        let p = build_house_design_delete_item(0x01, 0x0064, 1, 2, 3);
        assert_eq!(p[0], 0xD7);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(
            p,
            vec![
                0xD7, 0x00, 0x1E, // id + len (30)
                0x00, 0x00, 0x00, 0x01, // player serial
                0x00, 0x05, // subcmd: DeleteItem
                0x00, 0x00, 0x00, 0x00, 0x64, // graphic
                0x00, 0x00, 0x00, 0x00, 0x01, // x
                0x00, 0x00, 0x00, 0x00, 0x02, // y
                0x00, 0x00, 0x00, 0x00, 0x03, // z
                0x0A,
            ]
        );
    }

    #[test]
    fn house_design_go_to_floor_layout() {
        // GoToFloor: subcmd 0x12, 1 encoded Int32 arg (floor).
        let p = build_house_design_go_to_floor(0x1234_5678, 2);
        assert_eq!(p[0], 0xD7);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(
            p,
            vec![
                0xD7, 0x00, 0x0F, // id + len (15)
                0x12, 0x34, 0x56, 0x78, // player serial
                0x00, 0x12, // subcmd: GoToFloor
                0x00, 0x00, 0x00, 0x00, 0x02, // floor = 2
                0x0A,
            ]
        );
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
    fn legacy_menu_response_has_fixed_item_and_cancel_shapes() {
        assert_eq!(
            build_legacy_menu_response(0x0102_0304, 0x0506, 2, 0x0F5E, 0x0481),
            vec![0x7D, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x00, 0x02, 0x0F, 0x5E, 0x04, 0x81,]
        );
        let cancel = build_legacy_menu_response(0x1122_3344, 7, 0, 0, 0);
        assert_eq!(cancel.len(), 13);
        assert_eq!(&cancel[7..], &[0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn hue_picker_response_matches_servuo_clipping_and_fixed_shape() {
        assert_eq!(
            build_hue_picker_response(0x0102_0304, 0x0386),
            vec![0x95, 1, 2, 3, 4, 0, 0, 0x03, 0x86]
        );
        assert_eq!(&build_hue_picker_response(7, 0)[7..], &[0, 2]);
        assert_eq!(&build_hue_picker_response(7, 0xFFFF)[7..], &[0x03, 0xE9]);
    }

    #[test]
    fn tip_request_matches_classicuo_fixed_shape_and_id_truncation() {
        assert_eq!(build_tip_request(0x1234_5678, false), [0xA7, 0x56, 0x78, 0]);
        assert_eq!(build_tip_request(0x1234_5678, true), [0xA7, 0x56, 0x78, 1]);
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
        assert_eq!(&p[9..], &[0x40, 0, 0, 9, 0, 7, 0x40, 0, 0, 0x0A, 0, 1]);

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
            vec![
                0x08,
                0,
                0,
                0,
                1,
                0,
                100,
                0,
                200,
                (-5i8) as u8,
                0,
                0xFF,
                0xFF,
                0xFF,
                0xFF
            ]
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
    fn pack_speech_keywords_roundtrips_servuo_nibble_layout() {
        // One id 0x0123 → bytes 00 11 23. ServUO: value=0x0011, count=1, hold=1,
        // then hold<<8|0x23 = 0x0123.
        assert_eq!(pack_speech_keywords(&[0x0123]), vec![0x00, 0x11, 0x23]);
        assert_eq!(
            unpack_speech_keywords(&[0x00, 0x11, 0x23]),
            Some(vec![0x0123])
        );
        // Two ids: 00 21 23 45 60 → 0x0123, 0x0456.
        let two = pack_speech_keywords(&[0x0123, 0x0456]);
        assert_eq!(two, vec![0x00, 0x21, 0x23, 0x45, 0x60]);
        assert_eq!(unpack_speech_keywords(&two), Some(vec![0x0123, 0x0456]));
    }

    #[test]
    fn unicode_say_encoded_sets_type_bit_and_utf8_body() {
        let p = build_unicode_say("vendor buy", 0, 0x34, 3, &[0x0009, 0x000C]);
        assert_eq!(p[0], 0xAD);
        assert_eq!(p[3] & SPEECH_ENCODED, SPEECH_ENCODED);
        assert_eq!(&p[8..12], b"ENU\0");
        let packed = pack_speech_keywords(&[0x0009, 0x000C]);
        assert_eq!(&p[12..12 + packed.len()], packed.as_slice());
        assert_eq!(&p[12 + packed.len()..], b"vendor buy\0");
        // No keywords → UTF-16 BE, Encoded bit clear.
        let plain = build_unicode_say("hi", 0, 0x34, 3, &[]);
        assert_eq!(plain[3] & SPEECH_ENCODED, 0);
        assert_eq!(&plain[12..], &[0x00, b'h', 0x00, b'i', 0x00, 0x00]);
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
    fn ascii_prompt_response_layout_cancel_and_cp1252() {
        let p = build_ascii_prompt_response(0xDEAD_BEEF, 0x2A, "  Café € 한글  ", false);
        assert_eq!(p[0], 0x9A);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(u32::from_be_bytes([p[7], p[8], p[9], p[10]]), 0x2A);
        assert_eq!(u32::from_be_bytes([p[11], p[12], p[13], p[14]]), 1);
        // ClassicUO CP1252: é is direct 0xE9, € is extension 0x80, Korean is
        // outside the repertoire and becomes one '?' byte per code point.
        assert_eq!(&p[15..], b"Caf\xE9 \x80 ??\0");

        let c = build_ascii_prompt_response(1, 2, "ignored", true);
        assert_eq!(c, vec![0x9A, 0, 16, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn ascii_prompt_response_clamps_to_128_encoded_characters() {
        let text = format!("{}tail", "a".repeat(128));
        let p = build_ascii_prompt_response(1, 2, &text, false);
        assert_eq!(p.len(), 15 + 128 + 1);
        assert!(p[15..15 + 128].iter().all(|&b| b == b'a'));
        assert_eq!(p.last(), Some(&0));
    }

    #[test]
    fn text_entry_dialog_response_matches_callbacks_cancel_cp1252_and_limits() {
        let accepted = build_text_entry_dialog_response(0x0102_0304, 5, 6, "12a€345", true, 2, 4);
        assert_eq!(
            accepted,
            vec![
                0xAC, 0x00, 0x11, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x01, 0x00, 0x05, b'1', b'2',
                b'3', b'4', 0x00,
            ]
        );

        let canceled = build_text_entry_dialog_response(7, 8, 9, "Café", false, 0, 0);
        assert_eq!(canceled[9], 0, "Cancel uses code=false but retains text");
        assert_eq!(&canceled[10..12], &[0, 5]);
        assert_eq!(&canceled[12..], &[b'C', b'a', b'f', 0xE9, 0]);

        let astral = build_text_entry_dialog_response(1, 2, 3, "😀", true, 0, 2);
        assert_eq!(&astral[10..12], &[0, 3], "wire length uses UTF-16 units");
        assert_eq!(&astral[12..], &[b'?', 0, 0]);

        let huge =
            build_text_entry_dialog_response(1, 2, 3, &"a".repeat(70_000), true, 0, u32::MAX);
        assert_eq!(huge.len(), u16::MAX as usize);
        assert_eq!(u16::from_be_bytes([huge[1], huge[2]]), u16::MAX);
    }

    #[test]
    fn profile_request_and_update_match_classicuo_and_servuo_limits() {
        assert_eq!(
            build_profile_request(0x0102_0304),
            vec![0xB8, 0, 8, 0, 1, 2, 3, 4]
        );

        let update = build_profile_update(0x0102_0304, "Hi 😀");
        assert_eq!(update[0], 0xB8);
        assert_eq!(
            u16::from_be_bytes([update[1], update[2]]) as usize,
            update.len()
        );
        assert_eq!(&update[3..8], &[1, 1, 2, 3, 4]);
        assert_eq!(&update[8..10], &[0, 1]);
        assert_eq!(&update[10..12], &[0, 5], "length is UTF-16 units");
        assert_eq!(
            &update[12..],
            &[0, b'H', 0, b'i', 0, b' ', 0xD8, 0x3D, 0xDE, 0x00]
        );

        let text = format!("{}😀tail", "a".repeat(510));
        let clamped = build_profile_update(7, &text);
        assert_eq!(u16::from_be_bytes([clamped[10], clamped[11]]), 510);
        assert_eq!(clamped.len(), 12 + 510 * 2);
        assert!(clamped[12..].chunks_exact(2).all(|pair| pair == [0, b'a']));
    }

    #[test]
    fn logout_request_matches_classicuo_fixed_shape() {
        assert_eq!(build_logout_request(), [0xD1, 0x00]);
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

    #[test]
    fn trade_cancel_shape() {
        // action 1, 8 bytes total: [0x6F][len=8][0x01][myContainer].
        assert_eq!(
            build_trade_cancel(0xAABB_CCDD),
            vec![0x6F, 0x00, 0x08, 0x01, 0xAA, 0xBB, 0xCC, 0xDD]
        );
    }

    #[test]
    fn trade_accept_shape() {
        // action 2, 12 bytes: [0x6F][len=12][0x02][myContainer][accepted:u32].
        let on = build_trade_accept(0x0102_0304, true);
        assert_eq!(on, vec![0x6F, 0x00, 0x0C, 0x02, 1, 2, 3, 4, 0, 0, 0, 1]);
        let off = build_trade_accept(0x0102_0304, false);
        assert_eq!(off, vec![0x6F, 0x00, 0x0C, 0x02, 1, 2, 3, 4, 0, 0, 0, 0]);
    }

    #[test]
    fn trade_gold_shape() {
        // action 3, 16 bytes: [0x6F][len=16][0x03][myContainer][gold:u32][platinum:u32].
        let p = build_trade_gold(0xAABB_CCDD, 500, 2);
        assert_eq!(p[0], 0x6F);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(p[3], 0x03);
        assert_eq!(&p[4..8], &[0xAA, 0xBB, 0xCC, 0xDD]); // my container (BE)
        assert_eq!(u32::from_be_bytes([p[8], p[9], p[10], p[11]]), 500); // gold
        assert_eq!(u32::from_be_bytes([p[12], p[13], p[14], p[15]]), 2); // platinum
        assert_eq!(p.len(), 16);
    }

    #[test]
    fn ping_shape() {
        assert_eq!(build_ping(0x2A), vec![0x73, 0x2A]);
    }

    #[test]
    fn client_view_range_clamps() {
        assert_eq!(build_client_view_range(18), vec![0xC8, 18]);
        assert_eq!(build_client_view_range(2), vec![0xC8, 5]); // clamp up to MIN
        assert_eq!(build_client_view_range(99), vec![0xC8, 24]); // clamp down to MAX
    }

    #[test]
    fn name_request_layout() {
        let p = build_name_request(0xDEAD_BEEF);
        assert_eq!(p, vec![0x98, 0x00, 0x07, 0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
    }

    #[test]
    fn rename_request_layout() {
        // Fixed 35 bytes, no length field: [0x75][serial:u32][name: 30B CP1252 NUL-padded].
        let p = build_rename_request(0x0102_0304, "Rex");
        assert_eq!(p.len(), 35);
        assert_eq!(p[0], 0x75);
        assert_eq!(&p[1..5], &[1, 2, 3, 4]); // serial (BE)
        assert_eq!(&p[5..8], b"Rex");
        assert!(p[8..].iter().all(|&b| b == 0));

        // A name longer than the 30-byte field is truncated, not overflowed.
        let long = build_rename_request(1, &"x".repeat(40));
        assert_eq!(long.len(), 35);
        assert!(long[5..35].iter().all(|&b| b == b'x'));
    }

    #[test]
    fn bulletin_board_request_shapes() {
        assert_eq!(
            build_bulletin_request_message(0xAABB_CCDD, 0x1122_3344),
            vec![0x71, 0x00, 0x0C, 0x03, 0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44]
        );
        assert_eq!(
            build_bulletin_request_summary(0xAABB_CCDD, 0x1122_3344),
            vec![0x71, 0x00, 0x0C, 0x04, 0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44]
        );
        assert_eq!(
            build_bulletin_remove_message(0xAABB_CCDD, 0x1122_3344),
            vec![0x71, 0x00, 0x0C, 0x06, 0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44]
        );
    }

    #[test]
    fn bulletin_post_message_layout() {
        let p = build_bulletin_post_message(0x0102_0304, 0x0506_0708, "Hi", &["line1", "line2"]);
        assert_eq!(p[0], 0x71);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(p[3], 0x05); // sub-command: post
        assert_eq!(&p[4..8], &[1, 2, 3, 4]); // board (BE)
        assert_eq!(&p[8..12], &[5, 6, 7, 8]); // replyTo (BE)
        assert_eq!(p[12], 3); // subjectLen = utf8-bytes("Hi") + 1
        assert_eq!(&p[13..15], b"Hi");
        assert_eq!(p[15], 0); // subject NUL
        assert_eq!(p[16], 2); // lineCount
        assert_eq!(p[17], 6); // "line1".len() + 1
        assert_eq!(&p[18..23], b"line1");
        assert_eq!(p[23], 0);
        assert_eq!(p[24], 6); // "line2".len() + 1
        assert_eq!(&p[25..30], b"line2");
        assert_eq!(p[30], 0);
        assert_eq!(p.len(), 31);

        // A non-ASCII subject is where ClassicUO's version breaks and ours must
        // not: it declares .NET's char count ahead of UTF-8 bytes, so ServUO's
        // `ReadUTF8StringSafe(byteCount)` reads too few and every field after
        // the subject shifts. Measured live before the fix: `항해일지` came back
        // from the board as `항` plus half a character.
        let q = build_bulletin_post_message(1, 0, "é", &[]);
        assert_eq!(q[12], 3, "2 UTF-8 bytes + the NUL, not 1 UTF-16 unit + 1");
        assert_eq!(&q[13..15], "é".as_bytes());
        assert_eq!(q[15], 0);
        assert_eq!(q[16], 0); // lineCount
        assert_eq!(q.len(), 17);

        // The prefix must equal exactly the bytes ServUO will consume, for a
        // subject of any width — the property the old code only held for ASCII.
        for subject in ["ascii", "é", "항해일지", "mixed 한글 abc"] {
            let r = build_bulletin_post_message(1, 0, subject, &[]);
            assert_eq!(r[12] as usize, subject.len() + 1, "{subject:?}");
            assert_eq!(&r[13..13 + subject.len()], subject.as_bytes());
            assert_eq!(r[13 + subject.len()], 0);
        }
    }

    #[test]
    fn chat_open_layout() {
        // Fixed 64 bytes, no length field: [0xB5][0x00][name:utf16-be][zero-pad].
        let p = build_chat_open("Bob");
        assert_eq!(p.len(), 64);
        assert_eq!(p[0], 0xB5);
        assert_eq!(p[1], 0x00);
        assert_eq!(&p[2..8], &[0x00, b'B', 0x00, b'o', 0x00, b'b']);
        assert!(p[8..].iter().all(|&b| b == 0));

        let empty = build_chat_open("");
        assert_eq!(empty.len(), 64);
        assert!(empty[2..].iter().all(|&b| b == 0));
    }

    #[test]
    fn chat_join_layout() {
        let p = build_chat_join("Test", "");
        assert_eq!(p[0], 0xB3);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], b"ENU\0"); // language
        assert_eq!(u16::from_be_bytes([p[7], p[8]]), 0x0062); // sub-command: join
        assert_eq!(u16::from_be_bytes([p[9], p[10]]), 0x0022); // opening quote
        assert_eq!(
            &p[11..19],
            &[0x00, b'T', 0x00, b'e', 0x00, b's', 0x00, b't']
        );
        assert_eq!(u16::from_be_bytes([p[19], p[20]]), 0x0000); // NUL terminator
        assert_eq!(u16::from_be_bytes([p[21], p[22]]), 0x0022); // closing quote
        assert_eq!(u16::from_be_bytes([p[23], p[24]]), 0x0020); // space
        assert_eq!(p.len(), 25); // no password → nothing trails the space

        let with_pw = build_chat_join("Test", "Pw");
        assert_eq!(with_pw.len(), 25 + 4 + 2); // "Pw" (2 units) + NUL
        assert_eq!(&with_pw[25..], &[0x00, b'P', 0x00, b'w', 0x00, 0x00]);
    }

    #[test]
    fn chat_create_channel_layout() {
        let p = build_chat_create_channel("Test", "");
        assert_eq!(p[0], 0xB3);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], b"ENU\0");
        assert_eq!(u16::from_be_bytes([p[7], p[8]]), 0x0063); // sub-command: create
        assert_eq!(&p[9..17], &[0x00, b'T', 0x00, b'e', 0x00, b's', 0x00, b't']);
        assert_eq!(u16::from_be_bytes([p[17], p[18]]), 0x0000); // NUL terminator
        assert_eq!(p.len(), 19); // no password → nothing trails the name

        let with_pw = build_chat_create_channel("Test", "Pw");
        assert_eq!(u16::from_be_bytes([with_pw[19], with_pw[20]]), 0x007B); // '{'
        assert_eq!(&with_pw[21..27], &[0x00, b'P', 0x00, b'w', 0x00, 0x00]);
        assert_eq!(u16::from_be_bytes([with_pw[27], with_pw[28]]), 0x007D); // '}'
        assert_eq!(with_pw.len(), 29);
    }

    #[test]
    fn chat_leave_layout() {
        assert_eq!(
            build_chat_leave(),
            vec![0xB3, 0x00, 0x09, b'E', b'N', b'U', 0x00, 0x00, 0x43]
        );
    }

    #[test]
    fn chat_message_layout() {
        let p = build_chat_message("Hi");
        assert_eq!(p[0], 0xB3);
        assert_eq!(u16::from_be_bytes([p[1], p[2]]) as usize, p.len());
        assert_eq!(&p[3..7], b"ENU\0");
        assert_eq!(u16::from_be_bytes([p[7], p[8]]), 0x0061); // sub-command: message
        assert_eq!(&p[9..13], &[0x00, b'H', 0x00, b'i']);
        assert_eq!(u16::from_be_bytes([p[13], p[14]]), 0x0000); // NUL terminator
        assert_eq!(p.len(), 15);
    }

    #[test]
    fn change_race_request_is_fifteen_bytes() {
        // ServUO HeritageTransform Size==15 is the confirm path.
        let p = build_change_race_request(0x83EA, 0x203B, 0x044E, 0x2040, 0x044E);
        assert_eq!(
            p,
            vec![
                0xBF, 0x00, 0x0F, 0x00, 0x2A, 0x83, 0xEA, 0x20, 0x3B, 0x04, 0x4E, 0x20, 0x40, 0x04,
                0x4E,
            ]
        );
        assert_eq!(
            build_change_race_cancel(),
            vec![0xBF, 0x00, 0x05, 0x00, 0x2A]
        );
        assert_eq!(build_open_uo_store(), vec![0xFA]);
    }
}

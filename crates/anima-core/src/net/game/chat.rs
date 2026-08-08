//! Speech, system messages, and the journal.
//!
//! The two talk packets (0x1C ASCII / 0xAE Unicode), their cliloc-templated
//! equivalents (0xC1/0xCC), the text prompts that expect an answer (0x9A/0xC2),
//! the chat-channel system (0xB2-0xB5) and bulletin boards (0x71) — plus the
//! `push_journal*` helpers every other module uses to record a line.

use super::*;

/// 0x71 BulletinBoardData — variable-length, multiplexed by a leading
/// sub-command byte. Ported from ClassicUO `PacketHandlers.BulletinBoardData`:
/// - sub `0` (open board): `[serial:u32][name:utf8*22]`. Sets a fresh
///   [`BulletinBoard`], discarding any previously accumulated summaries —
///   mirrors ClassicUO disposing and recreating its `BulletinBoardGump`.
/// - sub `1` (message summary): `[boardSerial:u32][serial:u32][parent:u32]
///   [posterLen:u8][poster:utf8*posterLen][subjectLen:u8][subject:utf8*subjectLen]
///   [datetimeLen:u8][datetime:utf8*datetimeLen]`. Only appended when a board
///   with a matching serial is currently open, mirroring ClassicUO only
///   adding to a `BulletinBoardGump` it can actually find.
/// - sub `2` (full message): `[boardSerial:u32][serial:u32][posterLen:u8]
///   [poster:ascii*posterLen][subjectLen:u8][subject:utf8*subjectLen]
///   [datetimeLen:u8][datetime:ascii*datetimeLen][unused:4][unk:u8]
///   [unk*4 bytes][lines:u8]{[lineLen:u8][line:utf8*lineLen if lineLen>0]}`.
///   Poster/datetime decode as CP1252 ("ASCII") while subject/body decode as
///   UTF-8 — a genuine ClassicUO wire quirk (mixed encodings in one packet),
///   not a bug to "fix" here. Non-empty lines are joined with `\n` and the
///   result is left-trimmed, matching ClassicUO's `msg.TrimStart()`.
///
/// Any other sub value is ignored.
pub(super) fn bulletin_board_data(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 3 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]);
    match r.u8()? {
        0 => {
            let serial = r.u32()?;
            let name = utf8_string(r.bytes(22)?);
            world.bulletin_board = Some(BulletinBoard {
                serial,
                name,
                summaries: Vec::new(),
            });
        }
        1 => {
            let board_serial = r.u32()?;
            let serial = r.u32()?;
            let parent = r.u32()?;
            let poster_len = r.u8()? as usize;
            let poster = utf8_string(r.bytes(poster_len)?);
            let subject_len = r.u8()? as usize;
            let subject = utf8_string(r.bytes(subject_len)?);
            let datetime_len = r.u8()? as usize;
            let datetime = utf8_string(r.bytes(datetime_len)?);
            if let Some(board) = world
                .bulletin_board
                .as_mut()
                .filter(|b| b.serial == board_serial)
            {
                // Upsert by serial rather than append: a header can legitimately
                // be requested more than once for the same message — a re-open
                // clears the list and any consumer re-asks, and two consumers
                // can ask at once — and a blind push turned that into duplicate
                // rows in the board listing (observed live, the same subject
                // twice).
                let summary = BulletinSummary {
                    serial,
                    parent,
                    poster,
                    subject,
                    datetime,
                };
                match board.summaries.iter_mut().find(|m| m.serial == serial) {
                    Some(existing) => *existing = summary,
                    None => board.summaries.push(summary),
                }
            }
        }
        2 => {
            let board = r.u32()?;
            let serial = r.u32()?;
            let poster_len = r.u8()? as usize;
            let poster = ascii_string(r.bytes(poster_len)?);
            let subject_len = r.u8()? as usize;
            let subject = utf8_string(r.bytes(subject_len)?);
            let datetime_len = r.u8()? as usize;
            let datetime = ascii_string(r.bytes(datetime_len)?);
            r.skip(4)?;
            let unk = r.u8()?;
            r.skip(unk as usize * 4)?;
            let lines = r.u8()?;
            let mut body_lines = Vec::with_capacity(lines as usize);
            for _ in 0..lines {
                let line_len = r.u8()? as usize;
                if line_len > 0 {
                    body_lines.push(utf8_string(r.bytes(line_len)?));
                }
            }
            world.bulletin_message = Some(BulletinMessage {
                board,
                serial,
                poster,
                subject,
                datetime,
                body: body_lines.join("\n").trim_start().to_string(),
            });
        }
        _ => {}
    }
    Ok(())
}

/// 0x9A ASCIIPrompt — legacy counterpart to 0xC2. ClassicUO reads the request's
/// entire payload as one opaque big-endian u64; ServUO's response handler splits
/// it back into `[senderSerial:u32][promptId:u32]`. The request is therefore the
/// variable 11-byte frame `[id][len:u16][senderSerial:u32][promptId:u32]`.
pub(super) fn ascii_prompt(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 11 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]);
    let sender_serial = r.u32()?;
    let prompt_id = r.u32()?;
    world.prompt = Some(PromptState {
        sender_serial,
        prompt_id,
        kind: PromptKind::Ascii,
    });
    Ok(())
}

/// 0xC2 UnicodePrompt — the server asks us to answer with typed text (pet rename,
/// house sign, guild abbreviation, … — ~38 ServUO flows). Fixed 21 bytes as
/// ServUO sends it: `[id][len:u16][senderSerial:u32][promptId:u32][type:u32=0]
/// [language:u32=0][textLen:u16=0]` — the question text itself is NOT carried
/// here (ServUO sends it separately as a cliloc/system message just before this,
/// which already lands in [`World::journal`]); only the two ids the response must
/// echo matter (mirrors ClassicUO `PacketHandlers.UnicodePrompt`, which reads just
/// the leading 8 bytes as one `u64`). Answer with
/// [`crate::agent::Action::PromptResponse`]/[`crate::agent::Action::PromptCancel`].
pub(super) fn unicode_prompt(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 11 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let sender_serial = r.u32()?;
    let prompt_id = r.u32()?;
    world.prompt = Some(PromptState {
        sender_serial,
        prompt_id,
        kind: PromptKind::Unicode,
    });
    Ok(())
}

/// 0x1C ASCII Talk → journal.
pub(super) fn ascii_talk(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() <= 8 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]);
    let serial = r.u32()?;
    r.skip(2)?; // graphic
    let msg_type = r.u8()?;
    let hue = r.u16()?;
    r.skip(2)?; // font
    let name = r.fixed_ascii(30)?;
    let text = ascii_string(r.rest());
    push_journal(world, serial, name, text, msg_type, hue);
    Ok(())
}

/// 0xAE Unicode Talk → journal.
pub(super) fn unicode_talk(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() <= 48 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]);
    let serial = r.u32()?;
    r.skip(2)?; // graphic
    let msg_type = r.u8()?;
    let hue = r.u16()?;
    r.skip(2)?; // font
    r.skip(4)?; // language
    let name = r.fixed_ascii(30)?;
    let text = unicode_string(r.rest());
    push_journal(world, serial, name, text, msg_type, hue);
    Ok(())
}

/// 0xB2 ChatMessage — the server chat system, multiplexed by a leading `cmd`
/// u16. Variable length: `[id][len:u16][cmd:u16]…`. Ported from ClassicUO
/// `PacketHandlers.ChatMessage`. Most subcommands begin with a 4-byte skip
/// (an unused header ClassicUO discards) followed by UTF-16 BE strings. We keep
/// no user-list state (ClassicUO's `AddUser`/`RemoveUser` only feed a UI gump);
/// unknown/unmodelled commands are parsed-and-consumed so the stream stays in
/// sync. anima has no `Chat.enu` template table, so the localized system-text
/// commands store the raw UTF-16 payload verbatim.
pub(super) fn chat_message(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 5 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let cmd = r.u16()?;
    match cmd {
        0x03E8 => {
            // create conference
            r.skip(4)?;
            let name = utf16be_string(&mut r);
            let has_password = r.u16()? == 0x31;
            world.chat.current_channel = name.clone();
            if !world.chat.channels.iter().any(|c| c.name == name) {
                world.chat.channels.push(ChatChannel { name, has_password });
            }
        }
        0x03E9 => {
            // destroy conference
            r.skip(4)?;
            let name = utf16be_string(&mut r);
            world.chat.channels.retain(|c| c.name != name);
        }
        0x03EB => {
            // display enter-username window
            world.chat.enabled = ChatStatus::EnabledUserRequest;
        }
        0x03EC => {
            // close chat — clears channels + current + sets Disabled
            world.chat = crate::world::ChatState::default();
        }
        0x03ED => {
            // username accepted, display chat
            r.skip(4)?;
            let _username = utf16be_string(&mut r);
            world.chat.enabled = ChatStatus::Enabled;
        }
        0x03EE => {
            // add user (no user-list state kept)
            r.skip(4)?;
            let _user_type = r.u16()?;
            let _username = utf16be_string(&mut r);
        }
        0x03EF => {
            // remove user (no user-list state kept)
            r.skip(4)?;
            let _username = utf16be_string(&mut r);
        }
        0x03F0 => {
            // clear all players — no-op
        }
        0x03F1 => {
            // you have joined a conference
            r.skip(4)?;
            let name = utf16be_string(&mut r);
            world.chat.current_channel = name.clone();
            if !world.chat.channels.iter().any(|c| c.name == name) {
                world.chat.channels.push(ChatChannel {
                    name,
                    has_password: false,
                });
            }
        }
        0x03F4 => {
            // you have left a channel
            r.skip(4)?;
            let _name = utf16be_string(&mut r);
        }
        0x0025..=0x0027 => {
            // a chat line: [skip4][msgType:u16][username][text]
            r.skip(4)?;
            let _msg_type = r.u16()?;
            let username = utf16be_string(&mut r);
            let mut text = utf16be_string(&mut r);
            // ClassicUO strips the first `{…}` span (a colour/format tag) from
            // the message before printing it — mirror that.
            if let (Some(open), Some(close)) = (text.find('{'), text.find('}')) {
                if close > open {
                    text.replace_range(open..=close, "");
                }
            }
            world.push_chat_message(username, text);
        }
        _ => {
            // Localized system text: ClassicUO looks these up in a `Chat.enu`
            // template table (which anima lacks) and substitutes a UTF-16 arg.
            // We have no table, so store the raw payload verbatim as a system
            // line (empty sender). Any other cmd is ignored entirely.
            if (0x0001..=0x0024).contains(&cmd) || (0x0028..=0x002C).contains(&cmd) {
                r.skip(4)?;
                let text = utf16be_string(&mut r);
                world.push_chat_message(String::new(), text);
            } else {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// 0xC1 ClilocMessage — a localized system message with optional args.
/// `[id][len:u16][serial:u32][graphic:u16][type:u8][hue:u16][font:u16][cliloc:u32][name:30][args:utf16-LE]`.
/// We keep the cliloc id + raw args; the brain resolves them against the Cliloc table.
pub(super) fn cliloc_message(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 48 {
        return Ok(());
    }
    let serial = u32::from_be_bytes([frame[3], frame[4], frame[5], frame[6]]);
    let msg_type = frame[9];
    let hue = u16::from_be_bytes([frame[10], frame[11]]);
    let cliloc = u32::from_be_bytes([frame[14], frame[15], frame[16], frame[17]]);
    let name = ascii_string(&frame[18..48]);
    let args = decode_unicode(&frame[48..], false); // 0xC1 args are little-endian
    push_journal_cliloc(
        world,
        serial,
        name,
        args,
        msg_type,
        hue,
        cliloc,
        String::new(),
        false,
    );
    Ok(())
}

/// 0xCC ClilocMessageAffix — like 0xC1 plus a 1-byte flag, a NUL-terminated ASCII
/// affix after the name, and **big-endian** args. The affix is appended to the text.
pub(super) fn cliloc_affix(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 49 {
        return Ok(());
    }
    let serial = u32::from_be_bytes([frame[3], frame[4], frame[5], frame[6]]);
    let msg_type = frame[9];
    let hue = u16::from_be_bytes([frame[10], frame[11]]);
    let cliloc = u32::from_be_bytes([frame[14], frame[15], frame[16], frame[17]]);
    // Affix flags (ClassicUO `AffixType`): 0x01 prepend, 0x02 system.
    let affix_prepend = frame[18] & 0x01 != 0;
    let name = ascii_string(&frame[19..49]);
    let affix_start = 49;
    let nul = frame[affix_start..]
        .iter()
        .position(|&b| b == 0)
        .map_or(frame.len(), |p| affix_start + p);
    let affix = ascii_string(&frame[affix_start..nul]);
    let args_start = (nul + 1).min(frame.len());
    // The arguments stay pure: ClassicUO joins the affix to the *translated*
    // line, so folding it in here both corrupts the argument list and loses
    // the affix entirely on a template with no placeholder.
    let text = decode_unicode(&frame[args_start..], true); // 0xCC args are big-endian
    push_journal_cliloc(
        world,
        serial,
        name,
        text,
        msg_type,
        hue,
        cliloc,
        affix,
        affix_prepend,
    );
    Ok(())
}

pub(super) fn push_journal(
    world: &mut World,
    serial: u32,
    name: String,
    text: String,
    msg_type: u8,
    hue: u16,
) {
    push_journal_cliloc(
        world,
        serial,
        name,
        text,
        msg_type,
        hue,
        0,
        String::new(),
        false,
    );
}

#[allow(clippy::too_many_arguments)] // mirrors 0xCC's flat field list
pub(super) fn push_journal_cliloc(
    world: &mut World,
    serial: u32,
    name: String,
    text: String,
    msg_type: u8,
    hue: u16,
    cliloc: u32,
    affix: String,
    affix_prepend: bool,
) {
    // A cliloc line is kept even with empty args (the id alone is meaningful);
    // plain speech with empty text is dropped.
    if text.is_empty() && cliloc == 0 {
        return;
    }
    // msg_type 6 = single-click label: the entity's NAME, not chat — store it on the
    // entity (so it drives the persistent overhead label / hover / all-names) and
    // don't scroll it in the journal. ServUO sends it either as raw text (cliloc 0)
    // or, the common case, as the localized "name" line (cliloc 1050045 = the OPL
    // header `~1_val~`, Mobile.OnSingleClick) whose `text` is already the resolved
    // name — the old `cliloc == 0`-only guard missed that path, so clicked names
    // leaked into the chat log and never reached `Mobile::name`.
    if msg_type == 6 && (cliloc == 0 || cliloc == 1050045) {
        let nm = text.trim();
        if !nm.is_empty() {
            if let Some(m) = world.mobiles.get_mut(&serial) {
                m.name = nm.to_string();
            }
            if let Some(it) = world.items.get_mut(&serial) {
                it.name = nm.to_string();
            }
        }
        return;
    }
    let name = if name.is_empty() {
        "System".to_string()
    } else {
        name
    };
    world.push_journal(JournalEntry {
        serial,
        name,
        text,
        msg_type,
        hue,
        cliloc,
        affix,
        affix_prepend,
        // Resolved by whoever holds the Cliloc table — see the field's doc.
        display: String::new(),
    });
}

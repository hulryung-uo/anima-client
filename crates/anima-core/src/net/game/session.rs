//! Session-level packets: things about the connection, not the world.
//!
//! External URL requests (0xA5, consent-gated), tip/notice windows (0xA6),
//! modal text entry (0xAB), character profiles (0xB8), the logout handshake
//! (0xD1), secure trading (0x6F), and High Seas boat movement (0xF6) — which is
//! here because it moves the *player's* frame of reference rather than an
//! ordinary mobile.

use super::*;

/// 0xA5 OpenUrl — `[id][len:u16][url:ascii-NUL]`. ClassicUO passes any
/// non-empty string straight to the OS browser. A remote shard is not trusted
/// enough to launch arbitrary URI handlers, so anima narrows this to a bounded,
/// absolute HTTP(S) URL with a well-formed authority and records a consent
/// request instead. The renderer is responsible for asking the user; receiving
/// this packet never navigates by itself.
pub(super) fn open_url(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 4 {
        return Ok(());
    }
    if let Some(url) = validated_http_url(&frame[3..]) {
        world.push_open_url(url);
    }
    Ok(())
}

pub(super) const MAX_OPEN_URL_BYTES: usize = 2_048;

/// Validate the deliberately small URL surface a game shard may ask a browser
/// to open. This is stricter than a general URL parser by design: HTTP(S) only,
/// printable ASCII only, no credentials, a DNS/IPv4/bracketed-IPv6 host, and an
/// optional numeric u16 port. Paths, queries, and fragments remain opaque.
pub(super) fn validated_http_url(payload: &[u8]) -> Option<String> {
    let end = payload
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(payload.len());
    let bytes = &payload[..end];
    if bytes.is_empty()
        || bytes.len() > MAX_OPEN_URL_BYTES
        || bytes
            .iter()
            .any(|&b| !(0x21..=0x7E).contains(&b) || matches!(b, b'\\' | b'"' | b'<' | b'>' | b'`'))
    {
        return None;
    }
    let url = std::str::from_utf8(bytes).ok()?;
    let scheme_end = url.find("://")?;
    let scheme = &url[..scheme_end];
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let remainder = &url[scheme_end + 3..];
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    if !valid_http_authority(authority) {
        return None;
    }
    Some(url.to_owned())
}

pub(super) fn valid_http_authority(authority: &str) -> bool {
    if authority.is_empty() || authority.contains('@') {
        return false;
    }

    if let Some(bracketed) = authority.strip_prefix('[') {
        let Some(close) = bracketed.find(']') else {
            return false;
        };
        let host = &bracketed[..close];
        let suffix = &bracketed[close + 1..];
        return host.parse::<std::net::Ipv6Addr>().is_ok() && valid_optional_port(suffix);
    }

    if authority.matches(':').count() > 1 {
        return false; // IPv6 must use brackets in an HTTP URL authority.
    }
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, ""), |(host, port)| (host, port));
    if host.is_empty() || (!port.is_empty() && port.parse::<u16>().is_err()) {
        return false;
    }
    if authority.ends_with(':') {
        return false;
    }

    if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return host.parse::<std::net::Ipv4Addr>().is_ok();
    }
    host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

pub(super) fn valid_optional_port(suffix: &str) -> bool {
    suffix.is_empty()
        || suffix
            .strip_prefix(':')
            .is_some_and(|port| !port.is_empty() && port.parse::<u16>().is_ok())
}

/// 0xA6 ScrollMessage / TipWindow — variable packet
/// `[id][len:u16][flag:u8][tip:u32][textLen:u16][text:cp1252*len]`.
/// ClassicUO ignores flag 1 entirely, renders flag 0 as a pageable “Tip of the
/// Day” (0xA7 previous/next), and every other flag as a close-only notice. A
/// truncated body is atomic: it cannot create a partial window.
pub(super) fn tip_window(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 4 {
        return Ok(());
    }
    let flag = frame[3];
    if flag == 1 {
        return Ok(());
    }
    if frame.len() < 10 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[4..]);
    let tip = r.u32()?;
    let text_len = r.u16()? as usize;
    let text = ascii_string(r.bytes(text_len)?).replace('\r', "\n");
    let kind = if flag == 0 {
        TipKind::Tip
    } else {
        TipKind::Notice
    };
    world.push_tip(tip, kind, text);
    Ok(())
}

/// 0xAB TextEntryDialog — legacy modal string query:
/// `[id][len:u16][serial:u32][parentId:u8][buttonId:u8][textLen:u16]
/// [text:cp1252*len][canClose:u8][variant:u8][maxLength:u32][descLen:u16]
/// [description:cp1252*len]`. Variant 2 is numeric-only. `canClose` controls a
/// silent right-click dismissal, not the explicit Cancel button (which always
/// emits 0xAC). Decode atomically so a truncated trailing description cannot
/// leave a half-valid callback in the world.
pub(super) fn text_entry_dialog(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 19 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]);
    let serial = r.u32()?;
    let parent_id = r.u8()?;
    let button_id = r.u8()?;
    let text_len = r.u16()? as usize;
    let text = ascii_string(r.bytes(text_len)?);
    let can_close = r.u8()? != 0;
    let variant = r.u8()?;
    let max_length = r.u32()?;
    let description_len = r.u16()? as usize;
    let description = ascii_string(r.bytes(description_len)?);
    world.push_text_entry_dialog(
        serial,
        parent_id,
        button_id,
        text,
        can_close,
        variant,
        max_length,
        description,
    );
    Ok(())
}

/// 0xB8 CharacterProfile response — `[id][len:u16][serial:u32]
/// [header:cp1252-NUL][footer:utf16be-NUL][body:utf16be-NUL]`. ClassicUO
/// replaces an existing profile gump with the same response serial and permits
/// editing only when that serial equals the local player's. ServUO deliberately
/// sends serial zero for a locked self profile, making it read-only. All three
/// strings are decoded atomically so truncation cannot replace a valid window.
pub(super) fn character_profile(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 12 {
        return Ok(());
    }
    let serial = u32::from_be_bytes([frame[3], frame[4], frame[5], frame[6]]);
    let payload = &frame[7..];
    let Some(header_end) = payload.iter().position(|&byte| byte == 0) else {
        return Err(PacketError::InvalidData("profile header has no terminator"));
    };
    let header = ascii_string(&payload[..header_end]);
    let mut offset = header_end + 1;
    let footer = take_utf16be_nul(payload, &mut offset)?;
    let body = take_utf16be_nul(payload, &mut offset)?;
    world.set_character_profile(serial, header, footer, body);
    Ok(())
}

/// 0xD1 LogoutAck — `[id][allowed:u8]`. This is only permission to terminate:
/// the driver still requires a matching client-side pending request before it
/// closes the socket, matching ClassicUO's `DisconnectionRequested` gate.
pub(super) fn logout_ack(world: &mut World, frame: &[u8]) -> PResult<()> {
    let Some(&allowed) = frame.get(1) else {
        return Ok(());
    };
    world.set_logout_ack(allowed != 0);
    Ok(())
}

/// 0xF6 High Seas smooth boat movement. The variable packet carries the boat's
/// destination plus every onboard entity's destination. Parse the complete
/// list before mutating so a truncated packet cannot split the rigid group.
pub(super) fn boat_moving(world: &mut World, frame: &[u8]) -> PResult<()> {
    let body = frame.get(3..).ok_or(PacketError::InvalidData(
        "0xF6 is missing its variable header",
    ))?;
    let mut r = PacketReader::new(body);
    let boat_serial = r.u32()?;
    let speed = r.u8()?;
    let moving_direction = r.u8()? & 0x07;
    let facing_direction = r.u8()? & 0x07;
    let boat_to = crate::types::Position {
        x: r.u16()?,
        y: r.u16()?,
        z: r.u16()? as i8,
    };
    let count = r.u16()?;
    if usize::from(count) > r.remaining() / 10 {
        return Err(PacketError::UnexpectedEof {
            needed: usize::from(count) * 10,
            remaining: r.remaining(),
        });
    }
    let mut destinations = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        destinations.push((
            r.u32()?,
            crate::types::Position {
                x: r.u16()?,
                y: r.u16()?,
                z: r.u16()? as i8,
            },
        ));
    }

    let Some(boat_from) = world.items.get(&boat_serial).map(|boat| boat.pos) else {
        return Ok(()); // ClassicUO ignores 0xF6 for an unknown multi.
    };
    if let Some(boat) = world.items.get_mut(&boat_serial) {
        boat.pos = boat_to;
        boat.direction = facing_direction;
    }

    let mut entities = Vec::new();
    for (serial, to) in destinations {
        let from = if let Some(mobile) = world.mobiles.get_mut(&serial) {
            let from = mobile.pos;
            mobile.pos = to;
            Some(from)
        } else if let Some(item) = world.items.get_mut(&serial) {
            let from = item.pos;
            item.pos = to;
            Some(from)
        } else {
            None
        };
        if let Some(from) = from {
            entities.push(BoatMovedEntity { serial, from, to });
        }
    }
    world.push_boat_movement(BoatMovement {
        seq: 0,
        boat_serial,
        speed,
        moving_direction,
        facing_direction,
        from: boat_from,
        to: boat_to,
        entities,
    });
    Ok(())
}

/// 0x6F SecureTrade — a player-to-player trade window (server→client
/// variants; the client→server actions the driver sends live in
/// [`crate::net::outgoing::build_trade_cancel`]/`build_trade_accept`/
/// `build_trade_gold`). Variable: `[id][len:u16][action:u8]` then, per
/// `action` (ServUO `Packets.cs` `DisplaySecureTrade`/`CloseSecureTrade`/
/// `UpdateSecureTrade`, cross-checked against ClassicUO
/// `PacketHandlers.SecureTrading` for the authoritative client-side
/// interpretation of each byte):
/// - `0` Display — opens a session: `[opponent:u32][myContainer:u32]
///   [theirContainer:u32][hasName:bool][name:ascii*30]`. ServUO always writes
///   `hasName = true` plus the full 30-byte (NUL-padded) name; we just skip
///   the bool and read the fixed field (defensively defaulting to empty if
///   the frame is short, rather than erroring the whole packet). Upserts by
///   opponent — see [`World::open_trade`] (ServUO allows only one session per
///   mobile pair, but a *different* opponent is a genuinely separate
///   concurrent session, so this does NOT clobber unrelated trades).
/// - `1` Close — `[container:u32]`: the trade ended (cancelled or completed).
///   `container` is always OUR OWN container serial (ServUO addresses this
///   packet per-mobile with that mobile's own `SecureTradeContainer`,
///   `SecureTrade.Close` sends `m_From.Container` to `m_From.Mobile` and
///   `m_To.Container` to `m_To.Mobile`) — [`World::close_trade`] removes just
///   that one session (and purges its leftover items); any other concurrent
///   session with a different opponent is untouched.
/// - `2` Update — `[container:u32][myAccept:u32][theirAccept:u32]`: both
///   sides' accept-checkbox state (ClassicUO `ImAccepting`/`HeIsAccepting`)
///   for the session keyed by `container`.
/// - `3` UpdateGold — `[container:u32][gold:u32][plat:u32]`: the OPPONENT's
///   virtual gold/platinum offer (ClassicUO `HisGold`/`HisPlatinum`) for the
///   session keyed by `container`.
/// - `4` UpdateLedger — same shape as `3`, but it's OUR OWN account's total
///   available currency (ClassicUO `Gold`/`Platinum` — an input CAP for our
///   offer, not an offer itself) for the session keyed by `container`. This
///   is the AOS/TOL "account gold" ledger (`TradeFlag.UpdateLedger`, gated on
///   ServUO `AccountGold.Enabled`/`NetState.NewSecureTrading`); see
///   [`crate::world::TradeState`]'s doc for how the three gold flavors (our
///   offer / their offer / our balance) differ.
///
/// Items on either side are NOT carried here — they arrive as ordinary
/// 0x25/0x3C container traffic against `my_container`/`their_container`
/// (ServUO's `SecureTradeEquip` packet literally reuses 0x25's layout), which
/// the existing container handlers already store with no special-casing.
pub(super) fn secure_trade(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 4 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    match r.u8()? {
        0x00 => {
            let opponent_serial = r.u32()?;
            let my_container = r.u32()?;
            let their_container = r.u32()?;
            r.skip(1)?; // "hasName" bool — ServUO always writes true (1)
            let opponent_name = if r.remaining() >= 30 {
                r.fixed_ascii(30)?
            } else {
                String::new()
            };
            world.open_trade(TradeState {
                opponent_serial,
                opponent_name,
                my_container,
                their_container,
                ..Default::default()
            });
        }
        0x01 => world.close_trade(r.u32()?),
        0x02 => {
            let container = r.u32()?;
            let my_accept = r.u32()? != 0;
            let their_accept = r.u32()? != 0;
            if let Some(t) = world.trade_mut(container) {
                t.my_accept = my_accept;
                t.their_accept = their_accept;
            }
        }
        0x03 => {
            let container = r.u32()?;
            let gold = r.u32()?;
            let plat = r.u32()?;
            if let Some(t) = world.trade_mut(container) {
                t.their_offer_gold = gold;
                t.their_offer_platinum = plat;
            }
        }
        0x04 => {
            let container = r.u32()?;
            let gold = r.u32()?;
            let plat = r.u32()?;
            if let Some(t) = world.trade_mut(container) {
                t.balance_gold = gold;
                t.balance_platinum = plat;
            }
        }
        _ => {}
    }
    Ok(())
}

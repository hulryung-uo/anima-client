//! Server-driven windows: gumps, vendors, books, maps, and menus.
//!
//! Everything the server opens on the player's screen and then waits on. The
//! layout grammar itself lives in [`crate::gump_layout`]; this module is the
//! transport — including 0xDD's zlib-packed form and the custom-house designer's
//! plane decoding, which share that compression.

use super::*;

/// 0x6C TargetCursor — the server asks us to pick a target.
/// `[id][type:u8][cursorID:u32][flag:u8]...` (19 bytes). `flag == 3` means the
/// server is *withdrawing* the cursor, so we clear any pending target instead.
pub(super) fn target_cursor(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let target_type = r.u8()?;
    let cursor_id = r.u32()?;
    let cursor_flag = r.u8()?;
    world.pending_target = if cursor_flag == 3 {
        None
    } else {
        Some(TargetCursor {
            target_type,
            cursor_id,
            cursor_flag,
        })
    };
    // A plain 0x6C is never a multi placement — whether it withdraws the
    // cursor or opens an unrelated one, any footprint an earlier 0x99 left
    // pending is now stale (only `multi_target_cursor` below ever sets it).
    world.pending_multi_placement = None;
    Ok(())
}

/// 0x99 TargetMultiPlacement — the cursor ServUO sends while a multi
/// placement tool (e.g. the house placement tool) is waiting for a spot,
/// fixed 30 bytes (`lengths.rs`): `[id][allowGround:u8][cursorId:u32]` then a
/// multi-id/offset/hue preview tail — the resulting 0xF3 + 0xD8 carry
/// everything we need to actually PLACE the multi, but the real client also
/// uses that tail to draw the footprint FOLLOWING the cursor before the
/// click lands, so the user isn't placing blind. We store it identically to
/// a plain ground target (`target_type: 1`) so the brain answers with an
/// ordinary 0x6C ground target reply (`Action::TargetGround` in
/// `agent.rs`) — ServUO doesn't care that the request arrived as 0x99, only
/// that the reply matches `cursor_id` with cursor-type ground.
pub(super) fn multi_target_cursor(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]); // skip id
    let _allow_ground = r.u8()?;
    let cursor_id = r.u32()?;
    world.pending_target = Some(TargetCursor {
        target_type: 1,
        cursor_id,
        cursor_flag: 0,
    });
    world.pending_multi_placement = multi_placement_tail(frame);
    Ok(())
}

/// The multi-id/offset/hue tail of a 0x99 (see `multi_target_cursor`'s doc),
/// for the browser's placement-preview outline. ClassicUO's
/// `PacketHandlers.MultiPlacement` reads this by seeking to ABSOLUTE offset
/// 18 in the full frame: `multiId:u16` at 18, `xOff/yOff/zOff/hue:u16` at
/// 20/22/24/26. `None` if the frame is shorter than that — a real ServUO
/// always sends the fixed 30-byte packet, but this must never turn a short
/// frame into an error (the plain ground target above still has to be
/// stored either way).
pub(super) fn multi_placement_tail(frame: &[u8]) -> Option<MultiPlacement> {
    if frame.len() < 28 {
        return None;
    }
    Some(MultiPlacement {
        multi_id: u16::from_be_bytes([frame[18], frame[19]]),
        x_off: u16::from_be_bytes([frame[20], frame[21]]),
        y_off: u16::from_be_bytes([frame[22], frame[23]]),
        z_off: u16::from_be_bytes([frame[24], frame[25]]),
        hue: u16::from_be_bytes([frame[26], frame[27]]),
    })
}

/// 0xE5 DisplayWaypoint — ServUO `Scripts/Misc/Waypoints.cs`:
/// `[id][len:u16][serial:u32][x:u16][y:u16][z:i8][map:u8][type:u16]
/// [ignoreObject:u16][cliloc:u32][name:utf16-le NUL][terminator:i16]`.
/// Type 1 is a corpse and type 6 a resurrection healer; all kinds stay raw in
/// the world model. A repeated serial refreshes the existing waypoint.
pub(super) fn display_waypoint(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // id + variable-length header
    let serial = r.u32()?;
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.i8()?;
    let map = r.u8()?;
    let kind = r.u16()?;
    let ignore_object = r.u16()? != 0;
    let cliloc = r.u32()?;
    let tail = r.rest();
    if tail.len() < 4 {
        return Err(PacketError::UnexpectedEof {
            needed: 4,
            remaining: tail.len(),
        });
    }
    if !tail.len().is_multiple_of(2) {
        return Err(PacketError::InvalidData("odd-length waypoint UTF-16 tail"));
    }
    let (name_with_nul, trailing) = tail.split_at(tail.len() - 2);
    if trailing != [0, 0] {
        return Err(PacketError::InvalidData("non-zero waypoint trailing short"));
    }
    let (name_bytes, name_nul) = name_with_nul.split_at(name_with_nul.len() - 2);
    if name_nul != [0, 0] {
        return Err(PacketError::InvalidData("unterminated waypoint name"));
    }
    // Mobile names are short. Cap the informational field so an adversarial
    // shard cannot retain 2,048 near-u16-sized strings in the World.
    let name = decode_unicode(&name_bytes[..name_bytes.len().min(512)], false);
    world.set_waypoint(Waypoint {
        serial,
        pos: crate::types::Position { x, y, z },
        map,
        kind,
        ignore_object,
        cliloc,
        name,
    });
    Ok(())
}

/// 0xE6 RemoveWaypoint — `[id][serial:u32]`.
pub(super) fn remove_waypoint(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.waypoints.remove(&r.u32()?);
    Ok(())
}

/// 0xDF AddOrRemoveBuffIcon — a buff/debuff icon added to or removed from the
/// player. Variable length: `[id][len:u16][serial:u32][icon:u16][count:u16]…`.
/// `count` (informally "action") == 0 *removes* the icon; >= 1 *adds* it, and
/// each block then carries `[source:u16][pad:2][icon:u16][queue:u16][pad:4]
/// [timer:u16][pad:3][titleCliloc:u32][descCliloc:u32][wtfCliloc:u32]…unicode
/// args]`. We only need `timer` — the duration in **seconds** — and the raw
/// `icon`; the localized name comes from a cliloc we lack, so we approximate it
/// from a small icon→name table (see [`buff_name`]). Ported from ClassicUO
/// `PacketHandlers.BuffDebuff` + `BuffTable.cs`/`BuffIconType`.
pub(super) fn buff(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 11 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let _serial = r.u32()?;
    let icon = r.u16()?;
    let count = r.u16()?;
    if count == 0 {
        world.remove_buff(icon);
        return Ok(());
    }
    // First block only — that's where the duration lives (mirrors ClassicUO).
    r.skip(2)?; // source_type
    r.skip(2)?; // padding
    r.skip(2)?; // icon (repeated)
    r.skip(2)?; // queue_index
    r.skip(4)?; // padding
    let timer = r.u16()?; // duration in seconds (0 = no timer / permanent)
    world.add_buff(icon, buff_name(icon), timer as u32);
    Ok(())
}

/// Map a raw `BuffIconType` id (off the wire) to a short display name. Ported
/// from ClassicUO's `BuffIconType` enum — the ~common magery/combat
/// buffs & debuffs. The real names are clilocs we don't carry, so this is an
/// approximation; anything unlisted falls back to `#<icon>`.
pub(super) fn buff_name(icon: u16) -> String {
    let n = match icon {
        0x03E9 => "Dismount Prevention",
        0x03ED => "Night Sight",
        0x03EE => "Death Strike",
        0x03EF => "Evil Omen",
        0x03F2 => "Divine Fury",
        0x03F3 => "Enemy of One",
        0x03F4 => "Hiding/Stealth",
        0x03F5 => "Meditation",
        0x03F7 => "Blood Oath",
        0x03F8 => "Corpse Skin",
        0x03FA => "Pain Spike",
        0x03FB => "Strangle",
        0x0401 => "Gift of Life",
        0x0403 => "Mortal Strike",
        0x0404 => "Reactive Armor",
        0x0405 => "Protection",
        0x0406 => "Arch Protection",
        0x0407 => "Magic Reflection",
        0x0408 => "Incognito",
        0x040B => "Polymorph",
        0x040C => "Invisibility",
        0x040D => "Paralyze",
        0x040E => "Poison",
        0x040F => "Bleed",
        0x0410 => "Clumsy",
        0x0411 => "Feeblemind",
        0x0412 => "Weaken",
        0x0413 => "Curse",
        0x0414 => "Mass Curse",
        0x0415 => "Agility",
        0x0416 => "Cunning",
        0x0417 => "Strength",
        0x0418 => "Bless",
        0x0419 => "Sleep",
        _ => return format!("#{icon}"),
    };
    n.to_string()
}

/// 0x74 OpenBuyWindow — a vendor's BUY list (prices for the items in its for-sale
/// container). Variable: `[id][len:u16][container:u32][count:u8]` then `count` ×
/// `[price:u32][nameLen:u8][name:ascii]`. The container's items already live in
/// [`World::items`]; the prices correspond to them **in packet order** (ClassicUO
/// matches by index — see `PacketHandlers.BuyList`). The vendor mobile is the
/// container's own container (`world.items[container].container`); a BUY request
/// (0x3B) is addressed to that vendor serial. A new window replaces any old one.
pub(super) fn open_buy_window(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 4 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let container = r.u32()?;
    let count = r.u8()?;
    // The vendor mobile owns the for-sale container (set when it entered view as a
    // worn shop layer). 0 if we haven't seen the linkage yet.
    let vendor = world
        .items
        .get(&container)
        .and_then(|it| it.container)
        .unwrap_or(0);
    // Read the `(price, name)` list in packet order — the wire carries no
    // serials/graphics. The concrete item behind each price is attached by
    // `recorrelate_shop_buy` below (and again whenever the container's contents
    // change), since the 0x74 list and the container's 0x3C contents arrive in
    // either order.
    let mut entries: Vec<crate::world::ShopBuyEntry> = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if r.remaining() < 5 {
            break;
        }
        let price = r.u32()?;
        let name_len = r.u8()? as usize;
        if r.remaining() < name_len {
            break;
        }
        let name = ascii_string(r.bytes(name_len)?);
        entries.push(crate::world::ShopBuyEntry {
            price,
            name,
            ..Default::default()
        });
    }
    world.shop_buy = Some(crate::world::ShopBuy {
        vendor,
        container,
        entries,
    });
    recorrelate_shop_buy(world);
    Ok(())
}

/// (Re)attach concrete for-sale items to an open BUY window's price lines.
///
/// The 0x74 BUY list prices a vendor's items in "correct" (buy-list) order, but
/// the matching 0x3C VendorBuyContent sends the *same* items **reversed** —
/// encoding each item's correct index+1 in its `x` coordinate for the client to
/// re-sort by (ServUO `Packets.cs::VendorBuyContent`: "OSI sends these in wierd
/// order… the x74 packet is sent in 'correct' order… The client sorts these by
/// their X/Y value"). So we pair `entries[i]` with the for-sale item whose `x`
/// ranks i-th, filling each [`crate::world::ShopBuyEntry`]'s serial/graphic/
/// amount/hue. Recomputed whenever either packet lands (they arrive in either
/// order); cheap no-op when no BUY window is open. This makes a BUY offer as
/// identifiable as a `ShopSellItem` — match by graphic, buy by serial.
pub(super) fn recorrelate_shop_buy(world: &mut World) {
    let container = match world.shop_buy.as_ref() {
        Some(sb) => sb.container,
        None => return,
    };
    let mut ordered: Vec<(u16, u32, u16, u16, u16)> = world
        .items
        .values()
        .filter(|it| it.container == Some(container))
        .map(|it| (it.pos.x, it.serial, it.graphic, it.amount, it.hue))
        .collect();
    // Sort by the X ServUO overloaded with each item's correct buy-list index+1.
    ordered.sort_by_key(|&(x, ..)| x);
    if let Some(sb) = world.shop_buy.as_mut() {
        for (i, entry) in sb.entries.iter_mut().enumerate() {
            if let Some(&(_, serial, graphic, amount, hue)) = ordered.get(i) {
                entry.serial = serial;
                entry.graphic = graphic;
                entry.amount = amount;
                entry.hue = hue;
            }
        }
    }
}

/// 0x7C OpenMenu — the pre-gump item/icon list and gray question menu.
///
/// Wire layout: `[id][len:u16][serial:u32][menu_id:u16][question_len:u8]
/// [question ASCII][count:u8]`, then `count` entries of
/// `[graphic:u16][hue:u16][text_len:u8][text ASCII]`. ServUO writes both
/// question-menu entry words as zero. Like ClassicUO, we classify the menu by
/// the first entry's graphic because real ServUO packets use a zero header id
/// for both menu types. The entire packet is decoded before replacing state, so
/// a truncated resend cannot destroy an already-open valid menu.
pub(super) fn open_legacy_menu(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 4 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]);
    let serial = r.u32()?;
    let menu_id = r.u16()?;
    let question_len = r.u8()? as usize;
    let question = ascii_string(r.bytes(question_len)?);
    let count = r.u8()? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let graphic = r.u16()?;
        let hue = r.u16()?;
        let text_len = r.u8()? as usize;
        let text = ascii_string(r.bytes(text_len)?);
        entries.push(LegacyMenuEntry { graphic, hue, text });
    }
    let kind = if entries.first().is_some_and(|entry| entry.graphic != 0) {
        LegacyMenuKind::Items
    } else {
        LegacyMenuKind::Question
    };
    world.open_legacy_menu(LegacyMenu {
        serial,
        menu_id,
        question,
        kind,
        entries,
    });
    Ok(())
}

/// 0x95 DisplayHuePicker — fixed 9-byte server request:
/// `[id][serial:u32][reserved:u16=0][graphic:u16]`. The same packet id and
/// width are reused client→server for the selected hue; see
/// [`crate::net::outgoing::build_hue_picker_response`].
pub(super) fn open_hue_picker(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    r.skip(2)?;
    let graphic = r.u16()?;
    world.open_hue_picker(HuePicker { serial, graphic });
    Ok(())
}

/// 0x9E SellList — the items a vendor will buy *from* our pack, with the price it
/// pays. Variable: `[id][len:u16][vendor:u32][count:u16]` then `count` ×
/// `[serial:u32][graphic:u16][hue:u16][amount:u16][price:u16][nameLen:u16][name:ascii]`.
/// `vendor` is the vendor mobile serial a SELL request (0x9F) is addressed to. A
/// new list replaces any old one.
pub(super) fn sell_list(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 4 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let vendor = r.u32()?;
    let count = r.u16()?;
    let mut items = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if r.remaining() < 14 {
            break;
        }
        let serial = r.u32()?;
        let graphic = r.u16()?;
        let hue = r.u16()?;
        let amount = r.u16()?;
        let price = r.u16()?;
        let name_len = r.u16()? as usize;
        if r.remaining() < name_len {
            break;
        }
        let name = ascii_string(r.bytes(name_len)?);
        items.push(crate::world::ShopSellItem {
            serial,
            graphic,
            hue,
            amount,
            price,
            name,
        });
    }
    world.shop_sell = Some(crate::world::ShopSell { vendor, items });
    Ok(())
}

/// 0xB0 DisplayGump — a server-sent generic gump/dialog (quest, NPC menu, …).
/// Variable: `[id][len:u16][serial:u32][gumpId:u32][x:u32][y:u32][layoutLen:u16]
/// [layout: ascii, layoutLen bytes][textLinesCount:u16]` then `count` ×
/// `[charLen:u16][text: utf16-be, charLen*2 bytes]`. The `layout` is the gump
/// command string (`{ resizepic … }{ button … }…`); the text lines are referenced
/// by index from `text`/`croppedtext`/`htmlgump` commands. Ported from ClassicUO
/// `PacketHandlers.OpenGump`.
pub(super) fn display_gump(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 3 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let serial = r.u32()?;
    let gump_id = r.u32()?;
    let x = r.u32()? as i32;
    let y = r.u32()? as i32;
    let layout_len = r.u16()? as usize;
    let layout = ascii_string(r.bytes(layout_len)?);
    let count = r.u16()? as usize;
    let text = read_gump_text_lines(&mut r, count);
    world.add_gump(Gump {
        serial,
        gump_id,
        x,
        y,
        layout,
        text,
    });
    Ok(())
}

/// 0xDD DisplayGumpPacked — the zlib-compressed form of 0xB0. Variable:
/// `[id][len:u16][serial:u32][gumpId:u32][x:u32][y:u32]` then a compressed layout
/// block `[compLen+4:u32][decompLen:u32][zlib: compLen bytes]`, then
/// `[textLinesCount:u32]`, then (only if count > 0) a compressed text block in the
/// same `[compLen+4][decompLen][zlib]` shape. Both inflated blocks have the same
/// content as 0xB0 (ASCII layout; `count` × `[charLen:u16][utf16-be]`). Ported
/// from ClassicUO `PacketHandlers.OpenCompressedGump` + ServUO `DisplayGumpPacked`.
pub(super) fn display_gump_packed(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 3 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let serial = r.u32()?;
    let gump_id = r.u32()?;
    let x = r.u32()? as i32;
    let y = r.u32()? as i32;

    let layout_bytes = read_zlib_block(&mut r)?;
    let layout = String::from_utf8_lossy(&layout_bytes)
        .trim_end_matches('\0')
        .to_string();

    let count = r.u32()? as usize;
    let text = if count > 0 {
        let text_bytes = read_zlib_block(&mut r)?;
        let mut tr = PacketReader::new(&text_bytes);
        read_gump_text_lines(&mut tr, count)
    } else {
        Vec::new()
    };
    world.add_gump(Gump {
        serial,
        gump_id,
        x,
        y,
        layout,
        text,
    });
    Ok(())
}

/// Defensive cap on [`World::house_designs`], mirroring `World::set_corpse_of`'s
/// `MAX_CORPSE_LINKS` pattern — a missed prune (or a very long multi-house
/// session) must not grow this map forever.
pub(super) const MAX_HOUSE_DESIGNS: usize = 64;

/// 0xD8 CustomHouse — normally sent in reply to our own 0xBF/0x1E design-details
/// request ([`crate::net::outgoing::build_house_design_request`]); ServUO also
/// pushes one unsolicited to the house OWNER entering customization mode
/// (`HouseFoundation.BeginCustomize`), which this handler absorbs identically.
/// Variable: `[id][len:u16][compression:u8][enableResponse:u8]
/// [serial:u32][revision:u32][tileCount:u16][bufferLength:u16][planeCount:u8]`
/// then `planeCount` × `[header:u32][zlib bytes: clen]`. Ported from
/// ClassicUO `PacketHandlers.cs CustomHouse` + ServUO `HouseFoundation`'s
/// design-packing writer (`Server/Multis/HouseFoundation.cs`).
///
/// Each plane's 32-bit header packs `mode`/`plane_z` as two 4-bit nibbles and
/// TWO 12-bit lengths (decompressed `dlen`, compressed `clen`) split across
/// the remaining three bytes: `dlen`'s low byte and `clen`'s low byte each get
/// a whole byte, and their two high nibbles share the header's last byte.
/// Get the nibble packing backwards and every plane decodes garbage lengths —
/// this exact split is ClassicUO-exact (`PacketHandlers.cs` `CustomHouse`) and
/// ServUO-writer-compatible (`HouseFoundation.cs`).
///
/// `clen == 0` means the plane was skipped server-side and consumes NO
/// payload bytes at all (distinct from a `graphic == 0` entry INSIDE a plane,
/// which consumes its bytes but decodes to nothing — see
/// [`crate::world::decode_house_planes`]). We store the design even when the
/// foundation item itself is unknown yet — position-decoding is deferred
/// regardless (mode-2 planes need multi.mul bounds core doesn't have), and
/// the purge sites ([`World::remove`]/[`World::on_map_change`]) cover cleanup
/// if the item never shows up.
pub(super) fn custom_house(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable: skip id + 2-byte length
    let _compression = r.u8()?; // ServUO always writes 3 (zlib); ClassicUO ignores it too
    let _enable_response = r.u8()?;
    let serial = r.u32()?;
    let revision = r.u32()?;
    r.skip(4)?; // advisory tile count (u16) + buffer length (u16) — never trust either
    let plane_count = r.u8()?;

    let mut planes = Vec::with_capacity(plane_count as usize);
    for _ in 0..plane_count {
        let header = r.u32()?;
        let mode = ((header >> 28) & 0x0F) as u8;
        let plane_z = ((header >> 24) & 0x0F) as u8;
        let dlen = (((header & 0x00FF_0000) >> 16) | ((header & 0x0000_00F0) << 4)) as usize;
        let clen = (((header & 0x0000_FF00) >> 8) | ((header & 0x0000_000F) << 8)) as usize;
        if clen == 0 {
            continue; // a skipped plane consumes no payload bytes
        }
        let zbytes = r.bytes(clen)?; // EOF-safe: errors bubble up, apply_packet swallows them
                                     // Inflate with `dlen` as a HARD output cap: both writers (ServUO/ClassicUO)
                                     // treat dlen as the exact decompressed size, and an unbounded inflate would
                                     // let a hostile plane zlib-bomb ~4 MB out of 4 KB of payload (deflate is up
                                     // to ~1032:1) — and a plain truncate() would still retain that capacity
                                     // inside `World::house_designs`. A stream that overruns dlen is corrupt by
                                     // definition; drop it to an empty plane like any other inflate error.
        let data = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(zbytes, dlen)
            .unwrap_or_default();
        planes.push(HousePlane {
            mode,
            plane_z,
            data,
        });
    }

    if world.house_designs.len() >= MAX_HOUSE_DESIGNS && !world.house_designs.contains_key(&serial)
    {
        // Prefer evicting a design that never got decoded (its foundation isn't in
        // view) over an arbitrary one — evicting a tiles_ready design would silently
        // revert an ON-SCREEN house to its stock foundation, and ServUO only re-sends
        // the 0x1D revision notice when the multi re-enters view.
        let victim = world
            .house_designs
            .iter()
            .find(|(_, d)| !d.tiles_ready)
            .map(|(k, _)| *k)
            .or_else(|| world.house_designs.keys().next().copied());
        if let Some(k) = victim {
            world.house_designs.remove(&k);
        }
    }
    world.house_designs.insert(
        serial,
        HouseDesign {
            revision,
            planes,
            tiles: std::collections::HashMap::new(),
            tiles_ready: false,
        },
    );
    // We just got the freshest design for this serial — any queued request is moot.
    world.pending_house_design_requests.retain(|s| *s != serial);
    Ok(())
}

/// 0xBA QuestArrow — show/hide the on-screen arrow pointing at a tile.
/// `[id][active:u8][x:u16][y:u16]` (classic 6 bytes); the modern/HS form appends a
/// `[serial:u32]` (10 bytes) which we read past and ignore. `active == 0` hides the
/// arrow (clears `quest_arrow`); otherwise it points at `(x, y)`. Ported from
/// ClassicUO `PacketHandlers.SetQuestArrow`.
pub(super) fn quest_arrow(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let active = r.u8()?;
    let x = r.u16()?;
    let y = r.u16()?;
    world.quest_arrow = if active != 0 { Some((x, y)) } else { None };
    Ok(())
}

/// 0xD6 MegaCliloc — an entity's Object Property List (the tooltip lines).
/// Variable: `[id][len:u16][0x0001:u16][serial:u32][0x00:u8][0x00:u8]
/// [revision:u32]` then repeated property entries `[clilocId:u32][argLen:u16]
/// [args: UTF-16 LE, argLen bytes]` until `clilocId == 0`. Each entry is one
/// property line — a cliloc id plus tab-separated args (the client resolves the id
/// to localized text and substitutes the args). Line 0 is the name; the rest are
/// magical mods. We store the raw `(cliloc, args)` list (core has no Cliloc table).
/// Ported from ClassicUO `PacketHandlers.MegaCliloc`.
pub(super) fn mega_cliloc(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 3 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let unknown = r.u16()?; // 0x0001 (ClassicUO ignores values > 1)
    if unknown > 1 {
        return Ok(());
    }
    let serial = r.u32()?;
    r.skip(2)?; // two zero bytes
    let revision = r.u32()?;
    let mut lines = Vec::new();
    while let Ok(cliloc) = r.u32() {
        if cliloc == 0 {
            break; // terminator
        }
        let arg_len = match r.u16() {
            Ok(n) => n as usize,
            Err(_) => break,
        };
        let args = match r.bytes(arg_len) {
            Ok(b) => decode_unicode(b, false), // args are UTF-16 LE
            Err(_) => break,
        };
        lines.push((cliloc, args));
    }
    world.set_opl(serial, revision, lines);
    Ok(())
}

/// 0xDC OPLInfo — the OPL revision hash for an entity (fixed 9 bytes):
/// `[id][serial:u32][revision:u32]`. ServUO emits one alongside every
/// `SendInfoTo`/`Delta` of an entity (`Server/Item.cs`, `Server/Mobile.cs`), so
/// like the 0xBF/0x1D house notice it is a *notice*, not a change event: act only
/// when it names a revision other than the one the OPL we're holding came with,
/// and then only by queueing — core never sends bytes, the session layer drains
/// [`World::pending_opl_requests`] and sends the 0xD6.
///
/// The comparison masks off `0x4000_0000`. ServUO's `ObjectPropertyList` writes
/// the raw `m_Hash` into the 0xD6 body (`Terminate`, seeking to offset 11) but
/// `0x4000_0000 + m_Hash` into this packet (its `Hash` property, used by
/// `OPLInfo`) — a literal comparison would therefore never match and every notice
/// would re-request forever. ClassicUO masks identically, with a plain-equality
/// fallback for shards that don't set the bit
/// (`Game/Managers/ObjectPropertiesListManager.IsRevisionEquals`).
///
/// A serial we hold no OPL for stays hover-driven and is ignored here. ClassicUO
/// does request one (`AddMegaClilocRequest`), but it can afford to: it coalesces
/// a frame's serials into batched 0xD6 packets and gates on the tooltip feature
/// flag. Since ServUO sends 0xDC for everything entering view, doing the same
/// per-serial here would be a request storm for tooltips nobody asked to see.
pub(super) fn opl_info(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let revision = r.u32()?;
    let Some(&held) = world.opl_revision.get(&serial) else {
        return Ok(());
    };
    if (revision & !0x4000_0000) != held && revision != held {
        world.push_opl_request(serial);
    }
    Ok(())
}

/// 0x93 OpenBook — the (legacy, fixed 99-byte) book header.
/// `[id][serial:u32][writable:u8][unk:u8][pageCount:u16][title:60 ascii][author:30 ascii]`.
/// Sets `world.book` with `page_count` empty pages; the content arrives via 0x66.
pub(super) fn open_book(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let writable = r.u8()? != 0;
    r.skip(1)?; // unknown (sealed/readable flag, unused)
    let page_count = r.u16()?;
    let title = r.fixed_ascii(60)?;
    let author = r.fixed_ascii(30)?;
    world.book = Some(crate::world::Book {
        serial,
        title,
        author,
        writable,
        page_count,
        pages: vec![Vec::new(); page_count as usize],
    });
    Ok(())
}

/// 0xD4 OpenBookNew — the modern (variable-length) book header with length-prefixed
/// UTF-8 title/author. `[id][len:u16][serial:u32][writable:u8][unk:u8][pageCount:u16]
/// [titleLen:u16][title:utf8][authorLen:u16][author:utf8]`. Like 0x93 it sizes
/// `pages` to `page_count`; content arrives via 0x66.
pub(super) fn open_book_new(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 3 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let serial = r.u32()?;
    let writable = r.u8()? != 0;
    r.skip(1)?; // unknown
    let page_count = r.u16()?;
    let title_len = r.u16()? as usize;
    let title = String::from_utf8_lossy(r.bytes(title_len)?)
        .trim_end_matches('\0')
        .to_string();
    let author_len = r.u16()? as usize;
    let author = String::from_utf8_lossy(r.bytes(author_len)?)
        .trim_end_matches('\0')
        .to_string();
    world.book = Some(crate::world::Book {
        serial,
        title,
        author,
        writable,
        page_count,
        pages: vec![Vec::new(); page_count as usize],
    });
    Ok(())
}

/// 0x66 BookData — incoming page content for the open book (variable).
/// `[id][len:u16][serial:u32][pageCount:u16]` then per page `[pageNum:u16]
/// [lineCount:u16]` then `lineCount` NUL-terminated ASCII lines. Fills the matching
/// pages of `world.book` (indexed `pageNum - 1`); a page out of range is skipped.
pub(super) fn book_data(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 3 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let serial = r.u32()?;
    let page_count = r.u16()?;
    // Only fill if it's the book we have open.
    let Some(book) = world.book.as_mut().filter(|b| b.serial == serial) else {
        return Ok(());
    };
    for _ in 0..page_count {
        if r.remaining() < 4 {
            break;
        }
        let page_num = r.u16()?;
        let line_count = r.u16()?;
        let mut lines = Vec::with_capacity(line_count as usize);
        for _ in 0..line_count {
            lines.push(read_nul_ascii(&mut r));
        }
        if let Some(idx) = (page_num as usize).checked_sub(1) {
            if idx < book.pages.len() {
                book.pages[idx] = lines;
            }
        }
    }
    Ok(())
}

/// 0x3B EndVendorBuy/EndVendorSell — the SAME wire opcode and 8-byte layout
/// for both completion paths (ServUO `Server/Network/Packets.cs`: `EndVendorBuy`
/// and `EndVendorSell` are both literally `base(0x3B, 8)`):
/// `[id][len:u16=8][vendor:u32][unused:u8=0]`. ServUO's
/// `PacketHandlers.VendorBuyReply`/`VendorSellReply` send this once a
/// buy/sell actually completes (`IVendor.OnBuyItems`/`OnSellItems` returns
/// true) or the vendor moved out of range/was deleted meanwhile — but NOT on
/// a rejected sale, so the window is meant to stay open for a retry in that
/// case. ClassicUO's own handler (`CloseVendorInterface`) disposes whichever
/// `ShopGump` is keyed by this vendor serial regardless of buy/sell — the same
/// single "close the vendor window for this serial" semantics we mirror here
/// against whichever of [`World::shop_buy`]/[`World::shop_sell`] actually
/// matches (closing is a no-op for whichever one doesn't, so this is safe to
/// call unconditionally on every 0x3B).
pub(super) fn end_vendor(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 7 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let vendor = r.u32()?;
    if world.shop_buy.as_ref().is_some_and(|b| b.vendor == vendor) {
        world.close_shop_buy();
    }
    if world.shop_sell.as_ref().is_some_and(|s| s.vendor == vendor) {
        world.close_shop_sell();
    }
    Ok(())
}

/// 0x88 DisplayPaperdoll — ServUO sends this whenever we double-click a mobile,
/// ours or another's (`Scripts/Misc/Paperdoll.cs`, off `Mobile.OnDoubleClick`).
/// Fixed 66 bytes (ServUO `DisplayPaperdoll : base(0x88, 66)`):
/// `[id][serial:u32][title: ascii fixed 60][flags:u8]`. `title` is the
/// server-precomputed name+title line (`Titles.ComputeTitle`) — plain text, no
/// cliloc to resolve. `flags`: `0x01` the mobile is in war mode; `0x02` we're
/// allowed to lift/equip items on this doll (`Mobile.AllowEquipFrom` — true
/// for our own, false for a stranger's). See [`crate::world::Paperdoll`] for
/// why every request (even a repeat for the same serial) gets a fresh `seq`.
pub(super) fn open_paperdoll(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 66 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let title = r.fixed_ascii(60)?;
    let flags = r.u8()?;
    world.set_paperdoll(serial, title, flags & 0x01 != 0, flags & 0x02 != 0);
    Ok(())
}

/// 0x90 DisplayMap (legacy) / 0xF5 DisplayMapNew — opens/refreshes a treasure
/// or decoration map item's window (ServUO `Scripts/Items/Tools/MapItem.cs`
/// `MapDetails : base(0x90, 19)` / `NewMapDetails : base(0xF5, 21)`;
/// cross-checked against ClassicUO `PacketHandlers.DisplayMap`). Both share
/// the same 17-byte body: `[id][serial:u32][gumpArt:u16][minX:u16][minY:u16]
/// [maxX:u16][maxY:u16][width:u16][height:u16]` (1+4+2*7 = 19, matching 0x90's
/// fixed length exactly); 0xF5 appends one more `[facet:u16]` at the very END
/// (verified in ServUO's `NewMapDetails` ctor — it writes the identical 8
/// fields as `MapDetails`, THEN one more `short`; the facet is NOT interleaved
/// before `width`/`height`), bringing it to 21 bytes. `has_facet` selects
/// which of the two this frame is. See [`crate::world::MapView`]'s doc for
/// what each field means and the pin-coordinate-space note.
pub(super) fn display_map(world: &mut World, frame: &[u8], has_facet: bool) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let gump_art = r.u16()?;
    let min_x = r.u16()?;
    let min_y = r.u16()?;
    let max_x = r.u16()?;
    let max_y = r.u16()?;
    let width = r.u16()?;
    let height = r.u16()?;
    // A legacy 0x90 carries no facet at all — ServUO's `MapDetails` ctor never
    // writes one, so 0 (Felucca) is the only sane default (matches
    // `World::map_index`'s own encoding).
    let facet = if has_facet { r.u16()? as u8 } else { 0 };
    world.set_map_view(
        serial, gump_art, facet, min_x, min_y, max_x, max_y, width, height,
    );
    Ok(())
}

/// 0x56 MapCommand — mutates the pins/editable flag of an already-open map
/// window (see [`display_map`]/[`crate::world::MapView`]). Fixed 11 bytes
/// (ServUO `MapCommand : base(0x56, 11)`): `[id][serial:u32][command:u8]
/// [number:u8][x:u16][y:u16]`. A no-op if `serial` has no [`crate::world::
/// MapView`] yet (a command for a map we haven't been shown, or one already
/// pruned) — see [`crate::world::World::apply_map_command`] for the full
/// per-command semantics (add/insert/move/remove/clear/toggle-editable/
/// set-editable), verified against ServUO `MapItem`'s `On*Pin`/
/// `OnToggleEditable` handlers and ClassicUO's `MapMessageType` enum.
pub(super) fn map_command(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let command = r.u8()?;
    let number = r.u8()?;
    let x = r.u16()?;
    let y = r.u16()?;
    world.apply_map_command(serial, command, number, x, y);
    Ok(())
}

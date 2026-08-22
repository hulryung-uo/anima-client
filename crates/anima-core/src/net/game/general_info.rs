//! The 0xBF GeneralInformation multiplexer.
//!
//! One packet id carrying a few dozen unrelated subcommands, so it gets its own
//! module rather than being smeared across the topical ones: party, popup menus,
//! extended stats, spellbook contents, facet changes, and the rest.

use super::*;

/// 0xBF GeneralInfo — multiplexed subcommands. We handle the fast-walk key
/// stack (sub 0x01 sets six keys, sub 0x02 pushes one; each walk consumes one),
/// close-gump-by-type (sub 0x04), party (sub 0x06), the facet switch (sub
/// 0x08), the popup menu (sub 0x14), map-diff enable (sub 0x18), extended
/// stats (sub 0x19: bonded-pet death / stat-training locks), spellbook content
/// (sub 0x1B), the custom house notification (sub 0x1D), house-designer
/// enter/leave (sub 0x20), the armed weapon special move being cleared (sub
/// 0x21), New Damage (sub 0x22), a special move / toggled spell going active
/// or inactive (sub 0x25), speed mode (sub 0x26), pre-OPL equipment info
/// (sub 0x10), the heritage / race-change dialog (sub 0x2A), and a forced
/// mobile animation (sub 0x2B).
///
/// Deliberately NOT wired: sub 0x16 CloseUserInterfaceWindows. ClassicUO does
/// handle it client-side (`ExtendedCommand` case 0x16: paperdoll/statusbar/
/// profile/container by numeric id), but a full-text search of ServUO's
/// `Server/Network/Packets.cs` finds no packet class for it at all — nothing
/// server-side ever constructs or sends a 0xBF/0x16 payload, so it's dead code
/// on this stack (ServUO never emits it) and there is nothing to test against
/// a live shard.
pub(super) fn general_info(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable
    let subcmd = r.u16()?;
    match subcmd {
        0x01 => {
            let mut keys = Vec::with_capacity(6);
            for _ in 0..6 {
                keys.push(r.u32()?);
            }
            world.fast_walk = keys;
        }
        0x02 => {
            let key = r.u32()?;
            if world.fast_walk.len() < 6 {
                world.fast_walk.push(key);
            }
        }
        // 0x04 CloseGump — ServUO `CloseGump(typeID, buttonID) : base(0xBF)`,
        // `EnsureCapacity(13)`: `[subcmd:u16][typeID:i32][buttonID:i32]`. Closes
        // by TYPE (see `World::close_gump_by_type`'s doc), not by the specific
        // open instance's serial. `buttonID` is read past and unused — every
        // real ServUO call site (`Mobile.CloseGump`, `BaseGump.Refresh`/
        // `.Cancel`) sends `0`, and we have no local "auto-click a button on
        // the player's behalf" behavior to drive with a nonzero one anyway.
        0x04 => {
            let type_id = r.u32()?;
            let _button_id = r.u32()?;
            world.close_gump_by_type(type_id);
        }
        0x06 => parse_party(world, &mut r)?,
        // 0x08 MapChange — `[mapId:u8]` (ServUO `MapChange`, CUO `PacketHandlers`
        // `case 8: world.MapIndex = ...`). Facet switch (Felucca/Trammel/Ilshenar/
        // Malas/Tokuno/TerMur). Routed through `on_map_change` (not a bare field
        // assignment) so the facet we're leaving gets purged — ServUO never sends
        // 0x1D deletes for it, so the old mobiles/items would otherwise become
        // permanent phantoms. See [`World::map_index`] for what a real facet
        // reload of `MapData` would additionally require.
        0x08 => world.on_map_change(r.u8()?),
        0x14 => parse_popup(world, &mut r)?,
        // 0x18 EnableMapDiffs — `[count:u32]{ [mapPatches:u32][staticPatches:u32] }×count`
        // (ClassicUO `ApplyPatches`, big-endian). The counts say how many of
        // each facet's `mapdiflN`/`stadiflN` list entries to apply, in file
        // order; missing diff files mean that facet simply has none. ServUO's
        // `MapPatches` packet writes four facets. ClassicUO reads (map, statics)
        // per facet — we match that read order even if a given shard swapped
        // the two words (the play server applies them as (land, statics)).
        0x18 => {
            let n = (r.u32()? as usize).min(6);
            let mut facets = Vec::with_capacity(n);
            for _ in 0..n {
                let map = r.u32()?;
                let sta = r.u32()?;
                facets.push((map, sta));
            }
            world.map_patch_counts = facets;
            world.map_patches_gen = world.map_patches_gen.saturating_add(1);
        }
        0x19 => parse_extended_stats(world, &mut r)?,
        0x1B => parse_spellbook_content(world, &mut r)?,
        // 0x1D CustomHouseNotification — `[serial:u32][revision:u32]`. ServUO
        // resends this on EVERY `SendInfoTo` of the multi (Server/Multis/
        // HouseFoundation.cs), not just after an edit, so it must not itself
        // trigger a request every time — only queue a fresh 0xBF/0x1E when
        // the revision we're holding (if any) is actually stale, and only
        // once (dedup). This packet never carries the design itself; ServUO
        // answers 0x1E with 0xD8, which is where the payload lives.
        0x1D => {
            let serial = r.u32()?;
            let revision = r.u32()?;
            if !world.items.contains_key(&serial) {
                // CUO parity (`HouseManager.Remove`): a notice for an item we
                // no longer know about means the house is gone (deleted, or
                // just out of view) — drop any design we were holding for
                // it. The staleness check below then naturally re-queues a
                // request (current becomes `None`); if the item never
                // reappears the request just sits harmlessly unanswered,
                // and if it does, `net::scene`'s decode self-heals (plan risk #10).
                world.house_designs.remove(&serial);
            }
            let current = world.house_designs.get(&serial).map(|d| d.revision);
            if current != Some(revision) && !world.pending_house_design_requests.contains(&serial) {
                world.pending_house_design_requests.push(serial);
            }
        }
        // 0x20 BeginHouseCustomization / EndHouseCustomization — ServUO
        // `Server/Multis/HouseFoundation.cs`:
        // `[serial:u32][state:u8][0x0000][0xFFFF][0xFFFF][0xFF]`. The trailing
        // four fields are a fixed, always-identical tail (no real content —
        // ClassicUO's `EnterHouseCustomization`/`GetInHouseCustomization`
        // handlers read past them too), so only `serial`/`state` are worth
        // reading here. `0x04` = entering the designer for `serial`'s
        // foundation, `0x05` = leaving it; any other state byte is reserved/
        // unknown and ignored, matching this function's general "unrecognized
        // sub-thing, no-op" stance (see e.g. the default arm below) rather
        // than erroring on it.
        0x20 => {
            let serial = r.u32()?;
            let state = r.u8()?;
            match state {
                0x04 => world.customizing_house = Some(serial),
                0x05 => world.customizing_house = None,
                _ => {}
            }
        }
        // 0x21 ClearWeaponAbility — payload-free (ServUO `ClearWeaponAbility`
        // is a 5-byte packet that is nothing but the subcommand). The server
        // sends it when the armed special move stops being armed *for reasons
        // we cannot see*: ServUO's `WeaponAbility.ClearCurrentAbility` fires it
        // when the move lands, when the swing misses, when mana runs short, and
        // when a weapon change invalidates it. Without this the arm is
        // write-only — we set it optimistically on our own 0xD7 and never learn
        // it's gone, so the bar's highlight survives every use.
        //
        // ClassicUO clears the `0x80` armed bit on both `Abilities[]` slots;
        // our single [`World::armed_ability`] holds only the armed move, so
        // zeroing it is the same thing (see that field's doc).
        0x21 => world.armed_ability = 0,
        // 0x22 New Damage — the AOS-era twin of 0x0B Damage (see [`damage`]'s
        // doc): some ServUO client/version combos emit this instead of 0x0B.
        // `[unk:u8][serial:u32][damage:u8]` (ClassicUO `ExtendedCommand` case
        // 0x22). Mirrors `damage`'s call to `push_damage`, just with a
        // one-byte (not u16) damage field.
        0x22 => {
            let _ = r.u8()?;
            let serial = r.u32()?;
            let dmg = r.u8()? as u16;
            // ClassicUO drops a 0-damage event; mirror it so the brain's
            // damage log never sees phantom 0-amount hits.
            if dmg > 0 {
                world.push_damage(serial, dmg);
            }
        }
        // 0x25 ToggleSpecialAbility — `[abilityID:u16][active:u8]` (ServUO
        // `ToggleSpecialAbility(int abilityID, bool active)`). Despite the
        // name this is NOT the weapon-ability family: ServUO sends it from
        // `SpecialMove` (`moveID + 1`) and `SamuraiSpell` (`spellID + 1`), so
        // the id is a **spell** id — the same space `Action::CastSpell` takes.
        // ClassicUO keeps these in `World.ActiveSpellIcons` and hues the
        // matching spellbook button; so do we (see
        // [`World::active_spell_icons`]).
        0x25 => {
            let spell = r.u16()?;
            let active = r.u8()? != 0;
            world.set_spell_icon_active(spell, active);
        }
        // 0x26 SpeedMode — `[val:u8]` (ClassicUO `CharacterSpeedType`;
        // `ExtendedCommand` case 0x26 resets an out-of-range value to 0/Normal
        // rather than reading past it). Values: 0 Normal, 1 FastUnmount, 2
        // CantRun, 3 FastUnmountAndCantRun — `>= 2` means the server forces us
        // to walk, which the movement driver can consult later.
        0x26 => {
            let val = r.u8()?;
            world.player_stats.speed_mode = if val > 3 { 0 } else { val };
        }
        0x10 => parse_display_equipment_info(world, &mut r)?,
        0x2A => parse_heritage(world, &mut r)?,
        0x2B => parse_forced_anim(world, &mut r)?,
        _ => {}
    }
    Ok(())
}

/// Overhead hue ClassicUO uses for 0xBF/0x10 equipment info
/// (`PacketHandlers` case 0x10: `0x3B2`, `MessageType.Regular`, `TextType.OBJECT`).
const EQUIP_INFO_HUE: u16 = 0x3B2;

/// 0xBF/0x10 DisplayEquipmentInfo — ServUO still sends this on equipment
/// `OnSingleClick` (crafted-by, unidentified, charges, exceptional, …) even
/// on shards that also have OPL. ClassicUO paints it as overhead text on the
/// item and then requests MegaCliloc; we journal the same lines (hue `0x3B2`)
/// so the renderer floats them and a brain sees them in `new_journal`.
///
/// Layout (ServUO `DisplayEquipmentInfo`): `[serial:u32][nameCliloc:u32]` then
/// optional tagged words `0xFFFFFFFD` (crafter ASCII) / `0xFFFFFFFC`
/// (unidentified), then `cliloc:u32` + `charges:i16` pairs, terminated by
/// `0xFFFFFFFF`. ClassicUO stops the attr loop at `Position < Length - 4`
/// so the terminator is never read as a cliloc. Unknown serials are dropped
/// the same way ClassicUO returns when `world.Items.Get` is null.
fn parse_display_equipment_info(world: &mut World, r: &mut PacketReader) -> PResult<()> {
    let serial = r.u32()?;
    let name_cliloc = r.u32()?;
    if !world.items.contains_key(&serial) {
        return Ok(());
    }
    let speaker = world
        .items
        .get(&serial)
        .map(|it| it.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| " ".to_string());

    if name_cliloc > 0 {
        push_journal_cliloc(
            world,
            serial,
            speaker.clone(),
            String::new(),
            0,
            EQUIP_INFO_HUE,
            name_cliloc,
            String::new(),
            false,
        );
    }

    let mut next = r.u32()?;
    let mut crafter_name_len: u16 = 0;
    let mut ascii = String::new();
    if next == 0xFFFF_FFFD {
        crafter_name_len = r.u16()?;
        if crafter_name_len > 0 {
            let raw = r.bytes(crafter_name_len as usize)?;
            ascii.push_str("Crafted by ");
            ascii.push_str(&ascii_string(raw));
        }
    }
    if crafter_name_len != 0 {
        next = r.u32()?;
    }
    let unidentified = next == 0xFFFF_FFFC;
    if unidentified {
        ascii.push_str("[Unidentified]");
    }

    let mut count: u8 = 0;
    let mut attrs: Vec<(u32, i16)> = Vec::new();
    while r.remaining() > 4 {
        if count != 0 || next == 0xFFFF_FFFD || next == 0xFFFF_FFFC {
            next = r.u32()?;
        }
        let charges = r.i16()?;
        attrs.push((next, charges));
        count = count.saturating_add(1);
    }

    if !ascii.is_empty() {
        push_journal(world, serial, speaker.clone(), ascii, 0, EQUIP_INFO_HUE);
    }
    for (cliloc, charges) in attrs {
        if cliloc == 0 || cliloc >= 0xFFFF_FFFC {
            continue;
        }
        let affix = if charges == -1 {
            String::new()
        } else {
            format!(" : {charges}")
        };
        push_journal_cliloc(
            world,
            serial,
            speaker.clone(),
            String::new(),
            0,
            EQUIP_INFO_HUE,
            cliloc,
            affix,
            false,
        );
    }
    Ok(())
}

/// 0xBF/0x2A HeritagePacket — `[female:u8][race:u8]` (ServUO
/// `HeritagePacket`, ClassicUO `ExtendedCommand` case 0x2A). Race `1` human /
/// `2` elf / `3` gargoyle opens the appearance dialog; `0` or `> 3` (including
/// ServUO's close sentinel `0xFF`) dismisses it. The client's confirm reply
/// is the same subcommand with five `u16`s — see
/// [`crate::net::outgoing::build_change_race_request`].
fn parse_heritage(world: &mut World, r: &mut PacketReader) -> PResult<()> {
    let female = r.u8()? != 0;
    let race = r.u8()?;
    world.race_change = if (1..=3).contains(&race) {
        Some(crate::world::RaceChangePrompt { female, race })
    } else {
        None
    };
    Ok(())
}

/// 0xBF/0x2B — force a mobile onto animation group `anim_id` at frame
/// `frame_count` (ClassicUO `SetAnimation` + `AnimIndex = frameCount`,
/// `ExecuteAnimation = false`). Serial on the wire is the low 16 bits of the
/// mobile serial. We queue it through the same 0x6E anim ring the renderer
/// already plays; freeze-on-frame is approximated by playing the group once.
fn parse_forced_anim(world: &mut World, r: &mut PacketReader) -> PResult<()> {
    let lo = r.u16()? as u32;
    let anim_id = r.u8()?;
    let frame_count = r.u8()?;
    if let Some(serial) = world
        .mobiles
        .values()
        .find(|m| (m.serial & 0xFFFF) == lo)
        .map(|m| m.serial)
    {
        world.push_anim(
            serial,
            u16::from(anim_id),
            u16::from(frame_count.max(1)),
            true,
            0,
        );
    }
    Ok(())
}

/// 0xBF/0x19 ExtendedStats — `[version:u8][serial:u32]` then version-specific
/// (ClassicUO `ExtendedCommand` case 0x19):
/// - version 0 (bonded-pet death flag): `[isDead:u8]`. **Existing-mobile-only**
///   — like ClassicUO's `Mobiles.Get(serial) == null` guard, this never spawns
///   a phantom mobile.
/// - version 2 (stat-training locks), only meaningful when `serial` is the
///   player: `[updateGump:u8][state:u8]`. `state` packs three 2-bit locks:
///   `str_lock = state>>4 & 3`, `dex_lock = state>>2 & 3`, `int_lock = state &
///   3` (0=Up, 1=Down, 2=Locked).
/// - other versions (e.g. 5, an animation override): ignored.
pub(super) fn parse_extended_stats(world: &mut World, r: &mut PacketReader) -> PResult<()> {
    let version = r.u8()?;
    let serial = r.u32()?;
    match version {
        0 => {
            let is_dead = r.u8()? != 0;
            if let Some(m) = world.mobiles.get_mut(&serial) {
                m.is_dead = is_dead;
            }
        }
        2 => {
            let _update_gump = r.u8()?;
            let state = r.u8()?;
            if world.is_player(serial) {
                world.player_stats.str_lock = (state >> 4) & 0x03;
                world.player_stats.dex_lock = (state >> 2) & 0x03;
                world.player_stats.int_lock = state & 0x03;
            }
        }
        _ => {}
    }
    Ok(())
}

/// 0xBF/0x1B NewSpellbookContent — `[unk:u16=0x0001][serial:u32][graphic:u16]
/// [offset:u16][content:u64]` (23 bytes total with the id/len/subcmd header,
/// matching ServUO `NewSpellbookContent`'s `EnsureCapacity(23)`). Sent only when
/// a spellbook is actually opened (ServUO `Spellbook.DisplayTo`, gated on
/// `NetState.NewSpellbook`). Unlike the rest of this packet's fields, `content`
/// is written **byte-by-byte LSB-first** (ServUO: `Write((byte)(content >> (i *
/// 8)))` for `i` 0..8) rather than big-endian like everything else on the wire —
/// ClassicUO's handler (`PacketHandlers.cs` case 0x1B) reconstructs it the same
/// way, one byte at a time. See [`crate::world::SpellbookContent`] for what the
/// fields mean.
pub(super) fn parse_spellbook_content(world: &mut World, r: &mut PacketReader) -> PResult<()> {
    r.skip(2)?; // unknown, always 0x0001
    let serial = r.u32()?;
    let graphic = r.u16()?;
    let offset = r.u16()?;
    let bytes = r.bytes(8)?;
    let mut content: u64 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        content |= (b as u64) << (i * 8);
    }
    world.set_spellbook_content(serial, graphic, offset, content);
    Ok(())
}

/// 0xBF/0x06 Party — a sub-sub byte selects the message (ported from ClassicUO
/// `PartyManager.ParsePacket` + ServUO `Engines.PartySystem.Packets`):
/// - `0x01` member list: `[count u8]` then `count × [serial u32]`. Replaces the
///   member set; `members[0]` is the leader. (We joined, so clear any pending invite.)
/// - `0x02` remove member: `[count u8][removed serial u32]` then `count × [serial
///   u32]` = the REMAINING members. `count == 0` ⇒ the party disbanded. We treat the
///   trailing serials as the authoritative member set (like ClassicUO).
/// - `0x03` private tell / `0x04` chat-to-all: `[from serial u32][unicode-BE text]`;
///   routed to the journal as party speech.
/// - `0x07` invitation: `[leader serial u32]` — someone invited us; stored as
///   `party.pending_invite` until we accept/decline.
pub(super) fn parse_party(world: &mut World, r: &mut PacketReader) -> PResult<()> {
    let code = r.u8()?;
    match code {
        0x01 | 0x02 => {
            let count = r.u8()? as usize;
            if code == 0x02 {
                // The removed member's serial precedes the remaining-member list.
                r.u32()?;
            }
            let mut members = Vec::with_capacity(count);
            for _ in 0..count {
                members.push(r.u32()?);
            }
            world.party.leader = members.first().copied().unwrap_or(0);
            world.party.members = members;
            // We're now in (or out of) a party; any outstanding invite is resolved.
            world.party.pending_invite = None;
        }
        0x03 | 0x04 => {
            let from = r.u32()?;
            let text = unicode_string(r.rest());
            let name = world
                .mobiles
                .get(&from)
                .map(|m| m.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Party".to_string());
            // msg_type 7 ≈ party/guild speech; carry a party hue so the journal can
            // tint it. (Avoids 6, which push_journal treats as a name label.)
            push_journal(world, from, name, text, 7, 0x0044);
        }
        0x07 => {
            world.party.pending_invite = Some(r.u32()?);
        }
        _ => {}
    }
    Ok(())
}

/// 0xBF/0x14 DisplayPopupMenu — the right-click context menu for `serial`.
///
/// `[version u16][serial u32][count u8]` then `count` entries. Two layouts exist
/// (ported from ClassicUO `PopupMenuData.Parse`):
/// - **version >= 2** (modern cliloc): `[cliloc u32][index u16][flags u16]`.
/// - **version 1** (legacy): `[index u16][cliloc-3000000 u16][flags u16]`, with
///   optional trailing words: `flags & 0x84` → skip 2, `flags & 0x40` → skip 2,
///   `flags & 0x20` → a color word.
///
/// We keep `(index, cliloc, flags)` per entry; the label text is resolved from
/// the Cliloc table by the renderer. Replaces any prior popup.
pub(super) fn parse_popup(world: &mut World, r: &mut PacketReader) -> PResult<()> {
    let version = r.u16()?;
    let serial = r.u32()?;
    let count = r.u8()?;
    let new_cliloc = version >= 2;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (index, cliloc, flags) = if new_cliloc {
            let cliloc = r.u32()?;
            let index = r.u16()?;
            let flags = r.u16()?;
            (index, cliloc, flags)
        } else {
            let index = r.u16()?;
            let cliloc = r.u16()? as u32 + 3_000_000;
            let flags = r.u16()?;
            if flags & 0x84 != 0 {
                r.skip(2)?;
            }
            if flags & 0x40 != 0 {
                r.skip(2)?;
            }
            if flags & 0x20 != 0 {
                r.skip(2)?; // replacement color word
            }
            (index, cliloc, flags)
        };
        entries.push(PopupEntry {
            index,
            cliloc,
            flags,
        });
    }
    world.popup = Some(PopupMenu { serial, entries });
    Ok(())
}

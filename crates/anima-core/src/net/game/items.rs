//! Items: on the ground, in containers, and in flight between them.
//!
//! The world-item family (0x1A legacy / 0xF3 HS), container contents and adds
//! (0x3C/0x25/0x24), and the drag/drop protocol — 0x23's animation plus the
//! 0x27/0x28/0x29 acknowledgements a client has to correlate before it can clear
//! its own cursor.

use super::*;

/// One container record `[serial:u32][graphic:u16][inc:u8][amount:u16][x:u16][y:u16][grid:u8][container:u32][hue:u16]`
/// (20 bytes). The increment byte is *added* to the graphic (variant ids).
pub(super) fn read_container_item(
    r: &mut PacketReader,
) -> PResult<(u32, u16, u16, u16, u16, u32, u16)> {
    let serial = r.u32()?;
    let graphic = r.u16()?.wrapping_add(r.u8()? as u16);
    let amount = r.u16()?;
    let x = r.u16()?;
    let y = r.u16()?;
    r.skip(1)?; // grid index
    let container = r.u32()?;
    let hue = r.u16()?;
    Ok((serial, graphic, amount.max(1), x, y, container, hue))
}

pub(super) fn put_in_container(world: &mut World, rec: (u32, u16, u16, u16, u16, u32, u16)) {
    let (serial, graphic, amount, x, y, container, hue) = rec;
    let it = world.item_mut(serial);
    it.graphic = graphic;
    it.amount = amount;
    it.pos.x = x;
    it.pos.y = y;
    it.container = Some(container);
    it.hue = hue;
    it.layer = 0; // a container item is not worn
}

/// 0xF3 WorldItemHS — a ground item, the modern form ServUO sends to 7.0.9+
/// clients (supersedes 0x1A). `[id][unk:u16][type:u8][serial:u32][graphic:u16]
/// [inc:u8][amount:u16][amount2:u16][x:u16][y:u16][z:i8][direction:u8][hue:u16][flags:u8]`.
/// `type == 2` is a **multi** (a placed boat or house), not a pickable item:
/// ClassicUO `UpdateGameObject` still stores it as an `Item` (`item.IsMulti =
/// true`). Unlike the legacy 0x1A path (where a multi is self-inferred from a
/// `graphic >= 0x4000` bank bit the *client* must notice and strip), 0xF3 tells
/// the client the type explicitly via this `type` byte, and ServUO's own
/// packet writer (`Server/Network/Packets.cs` `WorldItemHS`) masks `itemID &=
/// 0x3FFF` BEFORE ever writing a `BaseMulti`'s graphic — so the wire `graphic`
/// here NEVER carries the bank bit; there is nothing to strip. `graphic` is a
/// *multi id* (an index into `multi.idx`/`multi.mul`), not an ART graphic. We
/// mirror ClassicUO: store it via [`World::item_mut`] like any other item (so
/// 0x1D delete/prune/facet-purge all keep working unmodified) with
/// [`crate::world::Item::is_multi`] set; `anima-net::scene` expands its
/// components into the rendered/walkable world. The `direction` byte only
/// matters for a corpse (`graphic == 0x2006`), which uses it to orient the
/// death-pose sprite (ClassicUO `UpdateItemSA`/`Item.Direction`; same wire byte
/// it also reuses as `LightID` for non-corpse items, which we don't model).
pub(super) fn world_item_hs(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 24 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[1..]);
    r.skip(2)?; // unknown
    let data_type = r.u8()?;
    let serial = r.u32()?;
    let graphic = r.u16()?;
    let graphic_inc = r.u8()?;
    let amount = r.u16()?;
    r.skip(2)?; // amount (repeated)
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.i8()?;
    let direction = r.u8()?;
    let hue = r.u16()?;
    let is_multi = data_type == 0x02;
    let mut graphic = graphic.wrapping_add(graphic_inc as u16);
    if is_multi {
        // Defensive only: real ServUO traffic never sets the bank bit here at
        // all (see this fn's doc) — this mask is a no-op on the wire, kept so
        // the invariant "a multi's `graphic` field is always a plain, unmasked
        // multi id" matches the legacy 0x1A path, which really does need to
        // strip a live bank bit.
        graphic &= 0x3FFF;
    }
    let it = world.item_mut(serial);
    it.graphic = graphic;
    it.pos.x = x;
    it.pos.y = y;
    it.pos.z = z;
    it.hue = hue;
    it.amount = amount.max(1);
    it.container = None;
    it.layer = 0;
    it.direction = direction & 0x07;
    it.is_multi = is_multi;
    Ok(())
}

/// 0xF7 PacketList — a batch container: `[id][len:u16][count:u16]` then `count`
/// sub-packets, each a full packet with NO length prefix. ClassicUO's
/// `PacketList` dispatches only `0xF3` sub-packets (fixed 26 bytes) and stops on
/// any other id. ServUO does not batch, but OSI and some shards do; without this
/// the framing layer consumes the whole 0xF7 frame and its batched item updates
/// would be silently dropped (never dispatched). We rebuild each 26-byte 0xF3
/// sub-frame and reuse [`world_item_hs`].
pub(super) fn packet_list(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // skip id + u16 length
    let count = r.u16()?;
    for _ in 0..count {
        // ClassicUO only batches 0xF3 and breaks on anything else (it cannot
        // know an unknown id's length to keep walking the shared reader).
        if r.u8()? != 0xF3 {
            break;
        }
        let mut sub = [0u8; 26];
        sub[0] = 0xF3;
        sub[1..].copy_from_slice(r.bytes(25)?);
        world_item_hs(world, &sub)?;
    }
    Ok(())
}

/// 0x3C ContainerContent — a full refresh of one or more containers' items.
/// Stale items previously in a refreshed container (absent from the payload) are
/// dropped, mirroring ServUO's full-refresh semantics.
pub(super) fn container_content(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 5 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let count = r.u16()?;
    let mut fresh = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if r.remaining() < 20 {
            break;
        }
        fresh.push(read_container_item(&mut r)?);
    }

    // Drop stale items: anything currently in a container this packet refreshes
    // but missing from the new list.
    let mut refreshed: std::collections::HashMap<u32, std::collections::HashSet<u32>> =
        std::collections::HashMap::new();
    for &(s, .., c, _) in &fresh {
        refreshed.entry(c).or_default().insert(s);
    }
    let stale: Vec<u32> = world
        .items
        .values()
        .filter(|it| {
            it.container
                .and_then(|c| refreshed.get(&c))
                .is_some_and(|set| !set.contains(&it.serial))
        })
        .map(|it| it.serial)
        .collect();
    for s in stale {
        world.items.remove(&s);
    }

    for rec in fresh {
        put_in_container(world, rec);
    }
    // A vendor's for-sale container contents often land here *after* its 0x74
    // BUY list; backfill each price line's concrete item now that they exist.
    recorrelate_shop_buy(world);
    Ok(())
}

/// 0x25 AddItemToContainer — a single item placed into a container.
pub(super) fn add_to_container(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 21 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[1..]);
    let rec = read_container_item(&mut r)?;
    put_in_container(world, rec);
    recorrelate_shop_buy(world);
    Ok(())
}

/// 0x1A WorldItem — an item on the ground (legacy layout, with flag bits). A
/// wire graphic `>= 0x4000` marks a **multi** (placed boat/house) instead of a
/// normal item — see [`Item::is_multi`](crate::world::Item::is_multi)'s doc.
pub(super) fn world_item(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable
    let mut serial = r.u32()?;
    let has_amount = serial & 0x8000_0000 != 0;
    serial &= 0x7FFF_FFFF;

    let mut graphic = r.u16()?;
    let mut graphic_inc = 0u16;
    if graphic & 0x8000 != 0 {
        graphic &= 0x7FFF;
        graphic_inc = r.u8()? as u16;
    }
    // ClassicUO `UpdateItem` classifies `type = graphic >= 0x4000 ? 2 : 0`
    // (multi vs normal item) from the graphic AS READ off the wire —
    // `graphicInc` is stashed in a separate local and only added to `graphic`
    // later, inside `UpdateGameObject`, well AFTER this classification already
    // ran. Classifying on the post-increment value would misjudge an item
    // whose increment happens to cross the 0x4000 boundary (see the
    // `world_item_legacy_multi_classified_before_graphic_inc_added` regression
    // test) — so this must run BEFORE `graphic_inc` is folded in.
    let is_multi = graphic >= 0x4000;
    graphic = graphic.wrapping_add(graphic_inc);
    if is_multi {
        // ClassicUO masks a multi's graphic to `& 0x3FFF` (the wire value
        // carries the bank bit; strip it to get the plain multi id) — a
        // NON-multi item's graphic is stored unmasked, whatever
        // `graphic + graphic_inc` comes to (`UpdateGameObject`'s `item.Graphic
        // = graphic;` in its non-multi branch never masks).
        graphic &= 0x3FFF;
    }

    let amount = if has_amount { r.u16()? } else { 0 };

    let mut x = r.u16()?;
    let mut y = r.u16()?;
    // The direction byte is only present when this flag bit is set (ClassicUO
    // `UpdateItem`); absent → facing stays 0. Only meaningful for a corpse
    // (`graphic == 0x2006`), which uses it to orient the death-pose sprite.
    let mut direction = 0u8;
    if x & 0x8000 != 0 {
        x &= 0x7FFF;
        direction = r.u8()?;
    }
    let z = r.i8()?;
    let mut hue = 0u16;
    if y & 0x8000 != 0 {
        y &= 0x7FFF;
        hue = r.u16()?;
    }
    if y & 0x4000 != 0 {
        y &= 0x3FFF;
        r.skip(1)?; // flags
    }

    let it = world.item_mut(serial);
    it.graphic = graphic;
    it.pos.x = x;
    it.pos.y = y;
    it.pos.z = z;
    it.hue = hue;
    it.amount = if amount == 0 { 1 } else { amount };
    it.container = None; // on the ground
    it.direction = direction & 0x07;
    it.is_multi = is_multi;
    Ok(())
}

/// 0x23 DragAnimation — an item graphic flying from a source to a destination
/// (e.g. splitting gold off a stack). Fixed 26 bytes on the wire. `apply_packet`
/// can't render, so this just resolves the event (graphic remap + live source/
/// dest mobile position) and records it via [`World::push_drag_anim`] for the
/// renderer to play. Mirrors ClassicUO's `DragAnimation`.
pub(super) fn drag_animation(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let mut graphic = r.u16()?;
    // ClassicUO adds the graphic-increment byte into a `ushort` (wraps); use
    // wrapping_add so a malformed graphic near u16::MAX can't debug-panic and
    // crash the session (apply_packet only swallows parse *errors*, not panics).
    graphic = graphic.wrapping_add(r.u8()? as u16);
    let hue = r.u16()?;
    let count = r.u16()?;
    let mut source = r.u32()?;
    let mut source_x = r.u16()?;
    let mut source_y = r.u16()?;
    let mut source_z = r.i8()?;
    let mut dest = r.u32()?;
    let mut dest_x = r.u16()?;
    let mut dest_y = r.u16()?;
    let mut dest_z = r.i8()?;

    // Gold/gem stacks drag as their "in flight" variant graphic.
    match graphic {
        0x0EED => graphic = 0x0EEF,
        0x0EEA => graphic = 0x0EEC,
        0x0EF0 => graphic = 0x0EF2,
        _ => {}
    }

    // ClassicUO re-derives source/dest from the live mobile's position (and
    // zeroes the serial when it isn't a mobile we currently know about).
    if let Some(m) = world.mobiles.get(&source) {
        source_x = m.pos.x;
        source_y = m.pos.y;
        source_z = m.pos.z;
    } else {
        source = 0;
    }
    if let Some(m) = world.mobiles.get(&dest) {
        dest_x = m.pos.x;
        dest_y = m.pos.y;
        dest_z = m.pos.z;
    } else {
        dest = 0;
    }

    world.push_drag_anim(DragAnimation {
        seq: 0,
        graphic,
        hue,
        count,
        source,
        source_x,
        source_y,
        source_z,
        dest,
        dest_x,
        dest_y,
        dest_z,
    });
    Ok(())
}

/// 0x27 LiftRej — `[id][reason:u8]` (2 bytes). The server refused our last lift
/// (0x07 PickUp): the item never left its source. See [`World::recent_lift_rejects`]
/// for the reason-code table.
pub(super) fn lift_reject(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let reason = r.u8()?;
    world.push_lift_reject(reason);
    Ok(())
}

/// 0x28 EndDraggingItem — `[id][token:u32]` (5 bytes). ClassicUO releases its
/// held-item cursor and intentionally ignores the payload. We preserve it as an
/// opaque correlation token because older shards commonly put the item serial
/// there, allowing the polling renderer to avoid clearing a newer drag when this
/// acknowledgement arrives late.
pub(super) fn end_dragging_item(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.push_drag_completion(0x28, Some(r.u32()?));
    Ok(())
}

/// 0x29 DropItemAccepted — `[id]` (1 byte). This is the payload-free form of
/// the server's held-item cursor release acknowledgement.
pub(super) fn drop_item_accepted(world: &mut World) -> PResult<()> {
    world.push_drag_completion(0x29, None);
    Ok(())
}

/// 0x89 CorpseEquip — a corpse's worn-item layout, so it can be "undressed"
/// without opening its loot window first. Variable: `[id][len:u16][corpse:u32]`
/// then repeated `[layer:u8][serial:u32]` until `layer == 0` (Layer.Invalid
/// terminator). The wire layer is `real layer + 1` (ServUO
/// `Scripts/Items/Corpses/Packets.cs` `CorpseEquip`, CUO `CorpseEquipment`); we
/// store the un-shifted real layer. A truncated frame keeps whatever entries
/// parsed cleanly before it ran out. Ported from ClassicUO `PacketHandlers.CorpseEquipment`.
pub(super) fn corpse_equip(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 7 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let corpse = r.u32()?;
    let mut entries = Vec::new();
    // A read failure anywhere below (truncated frame) just stops early, keeping
    // whatever entries parsed cleanly before it ran out.
    while let Ok(layer) = r.u8() {
        if layer == 0 {
            break; // Layer.Invalid terminator
        }
        let Ok(serial) = r.u32() else {
            break; // truncated — drop the dangling layer byte
        };
        entries.push((layer - 1, serial));
    }
    world.set_corpse_equip(corpse, entries);
    Ok(())
}

/// 0x24 DrawContainer (ServUO `ContainerDisplay`/`ContainerDisplayHS`) — the
/// SERVER itself opens a container window, as opposed to the ordinary flow
/// where WE ask for it via our own double-click (banker "bank" speech, GM
/// `[bank`, a snoop menu pick, …). Fixed on our (High-Seas-negotiated,
/// 7.0.102.3-reporting) client: `[id][serial:u32][gumpId:i16]` plus a
/// trailing `[unk:i16=0x7D]` ServUO's `ContainerDisplayHS` always appends once
/// the client negotiates that protocol tier (`Container.DisplayTo` picks
/// `ContainerDisplayHS` vs the 7-byte legacy `ContainerDisplay` off
/// `NetState.HighSeas`, negotiated at client version 7.0.9.0+; ClassicUO's own
/// `PacketsTable` makes the identical 9-vs-7 split at the same version — see
/// `lengths.rs`'s `0x24` entry, `Fixed(9)`).
///
/// `gumpId` is NOT always a container: ServUO reuses this exact opcode for two
/// other gumps, distinguished only by the id (`Server/Network/Packets.cs`):
/// `DisplayBuyList`/`DisplayBuyListHS` (a vendor's "Buy" window) always writes
/// `gumpId = 0x30` with `serial` = the vendor **mobile**, and
/// `DisplaySpellbook`/`DisplaySpellbookHS` always writes `gumpId = -1`
/// (`0xFFFF` as the wire i16) with `serial` = the spellbook **item**; only
/// `ContainerDisplay`/`ContainerDisplayHS` write the container's real
/// `Item.GumpID` (e.g. a backpack/bank box art id). ClassicUO's own 0x24
/// handler (`PacketHandlers.OpenContainer`) special-cases exactly these two
/// ids — `graphic == 0xFFFF` opens a `SpellbookGump`, `== 0x0030` opens a
/// `ShopGump`, anything else opens a generic `ContainerGump` — and never
/// builds a container window for the first two. We already surface vendor
/// shops via 0x74/0x3B (`ShopBuy`/`ShopSell`) and spellbooks via 0xBF/0x1B
/// (`SpellbookContent`), so treating 0x30/0xFFFF as a container-open too would
/// spawn a spurious empty Container window (live-reproduced: opening a
/// cobbler's Buy list pushed the vendor's own mobile serial in as if it were a
/// container).
///
/// We still record `gump_id` in the ring for every 0x24 (see
/// [`World::recent_container_opens`]'s doc for why that stays unfiltered, raw
/// data) — deciding which of these ids is "really" a container-open window is
/// the renderer's call (`anima_net::scene`'s bridge to the web client), not
/// `World`'s, per D3 (core = data, renderer = policy).
pub(super) fn draw_container(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let gump_id = r.u16()?;
    world.push_container_open(serial, gump_id);
    Ok(())
}

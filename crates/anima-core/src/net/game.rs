//! Game-phase packet codec → [`World`] mutation.
//!
//! [`apply_packet`] decodes a single framed game packet and updates the world
//! state, which is the single source of truth. The brain/renderer read `World`;
//! they never parse bytes. Ported from `anima/anima/perception/handlers.py`.
//!
//! Only perception-relevant packets are handled so far; unrecognized ids are
//! ignored (returns `false`). Movement confirm/deny (0x21/0x22) are owned by
//! [`crate::net::movement`].

use super::packet::{PacketReader, Result as PResult};
use crate::world::{JournalEntry, Skill, TargetCursor, World};

/// Decode one framed game packet (id byte included) into `world`.
/// Returns `true` if the packet id was recognized.
pub fn apply_packet(world: &mut World, frame: &[u8]) -> bool {
    if frame.is_empty() {
        return false;
    }
    // A malformed/truncated packet must never crash the session — swallow parse
    // errors and treat the packet as handled-but-skipped.
    dispatch(world, frame[0], frame).unwrap_or(true)
}

fn dispatch(world: &mut World, id: u8, frame: &[u8]) -> PResult<bool> {
    match id {
        0x20 => mobile_update(world, frame)?,
        0x77 => mobile_moving(world, frame)?,
        0x78 => mobile_incoming(world, frame)?,
        0x1A => world_item(world, frame)?,
        0x1D => delete(world, frame)?,
        0x11 => char_status(world, frame)?,
        0xA1 => vital(world, frame, Vital::Hits)?,
        0xA2 => vital(world, frame, Vital::Mana)?,
        0xA3 => vital(world, frame, Vital::Stam)?,
        0x1C => ascii_talk(world, frame)?,
        0xAE => unicode_talk(world, frame)?,
        0xBF => general_info(world, frame)?,
        0x6C => target_cursor(world, frame)?,
        0x3A => skills(world, frame)?,
        0x3C => container_content(world, frame)?,
        0x25 => add_to_container(world, frame)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// 0x20 MobileUpdate — position/appearance reset.
fn mobile_update(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let body = r.u16()?;
    r.skip(1)?; // graphic_inc
    let hue = r.u16()?;
    r.skip(1)?; // flags
    let x = r.u16()?;
    let y = r.u16()?;
    r.skip(2)?; // server_id
    let direction = r.u8()? & 0x07;
    let z = r.i8()?;

    let m = world.mobile_mut(serial);
    m.body = body;
    m.hue = hue;
    m.pos.x = x;
    m.pos.y = y;
    m.pos.z = z;
    m.direction = direction;
    Ok(())
}

/// 0x6C TargetCursor — the server asks us to pick a target.
/// `[id][type:u8][cursorID:u32][flag:u8]...` (19 bytes). `flag == 3` means the
/// server is *withdrawing* the cursor, so we clear any pending target instead.
fn target_cursor(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let target_type = r.u8()?;
    let cursor_id = r.u32()?;
    let cursor_flag = r.u8()?;
    world.pending_target = if cursor_flag == 3 {
        None
    } else {
        Some(TargetCursor { target_type, cursor_id, cursor_flag })
    };
    Ok(())
}

/// One container record `[serial:u32][graphic:u16][inc:u8][amount:u16][x:u16][y:u16][grid:u8][container:u32][hue:u16]`
/// (20 bytes). The increment byte is *added* to the graphic (variant ids).
fn read_container_item(r: &mut PacketReader) -> PResult<(u32, u16, u16, u16, u16, u32, u16)> {
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

fn put_in_container(world: &mut World, rec: (u32, u16, u16, u16, u16, u32, u16)) {
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

/// 0x3C ContainerContent — a full refresh of one or more containers' items.
/// Stale items previously in a refreshed container (absent from the payload) are
/// dropped, mirroring ServUO's full-refresh semantics.
fn container_content(world: &mut World, frame: &[u8]) -> PResult<()> {
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
    Ok(())
}

/// 0x25 AddItemToContainer — a single item placed into a container.
fn add_to_container(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 21 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[1..]);
    let rec = read_container_item(&mut r)?;
    put_in_container(world, rec);
    Ok(())
}

/// 0x3A SkillUpdate — full skill list or a single skill change (variable).
/// `[id][len:u16][type:u8]` then entries `[skillID:u16][value][base][lock][cap?]`.
/// Ported from `anima/anima/perception/handlers.py::handle_skill_update`.
fn skills(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 4 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let list_type = r.u8()?;
    if list_type == 0xFE {
        return Ok(()); // skill-name metadata — ignored
    }
    // Match ClassicUO: caps present for 0x01/0x02/0x03/0xDF; ids are 1-based for
    // 0x00/0x02; single update for 0xDF/0xFF.
    let has_cap = matches!(list_type, 0x01 | 0x02 | 0x03 | 0xDF);
    let adjust = matches!(list_type, 0x00 | 0x02);
    let is_single = matches!(list_type, 0xDF | 0xFF);

    while r.remaining() >= 2 {
        let raw_id = r.u16()?;
        // The 1-based full list (0x00) terminates on id 0.
        if list_type == 0x00 && raw_id == 0 {
            break;
        }
        if r.remaining() < 5 {
            break;
        }
        let value = r.u16()?;
        let base = r.u16()?;
        let lock = r.u8()?;
        let cap = if has_cap && r.remaining() >= 2 { r.u16()? } else { 1000 };

        let id = if adjust {
            match raw_id.checked_sub(1) {
                Some(i) => i,
                None => {
                    if is_single {
                        break;
                    }
                    continue;
                }
            }
        } else {
            raw_id
        };

        let s = world.skills.entry(id).or_default();
        *s = Skill { id, value, base, cap, lock };

        if is_single {
            break;
        }
    }
    Ok(())
}

/// 0x77 MobileMoving — a mobile moves.
fn mobile_moving(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let body = r.u16()?;
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.i8()?;
    let direction = r.u8()? & 0x07;
    let hue = r.u16()?;
    let _flags = r.u8()?;
    let notoriety = r.u8()?;

    let m = world.mobile_mut(serial);
    m.body = body;
    m.pos.x = x;
    m.pos.y = y;
    m.pos.z = z;
    m.direction = direction;
    m.hue = hue;
    m.notoriety = notoriety;
    Ok(())
}

/// 0x78 MobileIncoming — a mobile enters view, with its worn-item list.
fn mobile_incoming(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable: skip id + length
    let serial = r.u32()?;
    let body = r.u16()?;
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.i8()?;
    let direction = r.u8()? & 0x07;
    let hue = r.u16()?;
    let _flags = r.u8()?;
    let notoriety = r.u8()?;

    {
        let m = world.mobile_mut(serial);
        m.body = body;
        m.pos.x = x;
        m.pos.y = y;
        m.pos.z = z;
        m.direction = direction;
        m.hue = hue;
        m.notoriety = notoriety;
    }

    // Worn items follow as fixed records: serial(u32) graphic(u16) layer(u8) hue(u16).
    // (NewMobileIncoming / CV_70331 format — hue always present, no 0x8000 flag.)
    while r.remaining() >= 4 {
        let item_serial = r.u32()?;
        if item_serial == 0 {
            break;
        }
        if r.remaining() < 5 {
            break;
        }
        let graphic = r.u16()?;
        let layer = r.u8()?;
        let ihue = r.u16()?;
        let it = world.item_mut(item_serial);
        it.graphic = graphic;
        it.layer = layer;
        it.hue = ihue;
        it.container = Some(serial);
    }
    Ok(())
}

/// 0x1A WorldItem — an item on the ground (legacy layout, with flag bits).
fn world_item(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable
    let mut serial = r.u32()?;
    let has_amount = serial & 0x8000_0000 != 0;
    serial &= 0x7FFF_FFFF;

    let mut graphic = r.u16()?;
    if graphic & 0x8000 != 0 {
        graphic &= 0x7FFF;
        graphic = graphic.wrapping_add(r.u8()? as u16); // graphic_inc
    }
    graphic &= 0x3FFF; // strip the 0x4000 multi flag

    let amount = if has_amount { r.u16()? } else { 0 };

    let mut x = r.u16()?;
    let mut y = r.u16()?;
    if x & 0x8000 != 0 {
        x &= 0x7FFF;
        r.skip(1)?; // direction
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
    Ok(())
}

/// 0x1D Delete — entity removed from the world.
fn delete(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    world.remove(serial);
    Ok(())
}

/// 0x11 CharacterStatus — name + full stat block for self, name/hits for others.
fn char_status(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable
    let serial = r.u32()?;
    let name = r.fixed_ascii(30)?;
    let hits = r.u16()?;
    let hits_max = r.u16()?;
    r.skip(1)?; // name_change_flag
    let flag = r.u8()?;

    let is_self = world.is_player(serial);
    {
        let m = world.mobile_mut(serial);
        m.name = name;
        m.hits = hits;
        m.hits_max = hits_max;
    }

    if is_self && flag >= 1 {
        let is_female = r.u8()? != 0;
        let strength = r.u16()?;
        let dexterity = r.u16()?;
        let intelligence = r.u16()?;
        let stam = r.u16()?;
        let stam_max = r.u16()?;
        let mana = r.u16()?;
        let mana_max = r.u16()?;
        let gold = r.u32()?;
        let armor = r.i16()?;
        let weight = r.u16()?;

        let stats = &mut world.player_stats;
        stats.is_female = is_female;
        stats.strength = strength;
        stats.dexterity = dexterity;
        stats.intelligence = intelligence;
        stats.gold = gold;
        stats.armor = armor;
        stats.weight = weight;

        let m = world.mobile_mut(serial);
        m.stam = stam;
        m.stam_max = stam_max;
        m.mana = mana;
        m.mana_max = mana_max;
    }
    Ok(())
}

enum Vital {
    Hits,
    Mana,
    Stam,
}

/// 0xA1/0xA2/0xA3 — a single vital bar update: `[id][serial:u32][max:u16][cur:u16]`.
fn vital(world: &mut World, frame: &[u8], which: Vital) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let max = r.u16()?;
    let cur = r.u16()?;
    let m = world.mobile_mut(serial);
    match which {
        Vital::Hits => {
            m.hits = cur;
            m.hits_max = max;
        }
        Vital::Mana => {
            m.mana = cur;
            m.mana_max = max;
        }
        Vital::Stam => {
            m.stam = cur;
            m.stam_max = max;
        }
    }
    Ok(())
}

/// 0x1C ASCII Talk → journal.
fn ascii_talk(world: &mut World, frame: &[u8]) -> PResult<()> {
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
fn unicode_talk(world: &mut World, frame: &[u8]) -> PResult<()> {
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

/// 0xBF GeneralInfo — multiplexed subcommands. We handle the fast-walk key
/// stack (sub 0x01 sets six keys, sub 0x02 pushes one); each walk consumes one.
fn general_info(world: &mut World, frame: &[u8]) -> PResult<()> {
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
        _ => {}
    }
    Ok(())
}

fn push_journal(world: &mut World, serial: u32, name: String, text: String, msg_type: u8, hue: u16) {
    if text.is_empty() {
        return;
    }
    // msg_type 6 = single-click label: update the entity's name instead of logging.
    if msg_type == 6 {
        if let Some(m) = world.mobiles.get_mut(&serial) {
            m.name = text.clone();
        }
        if let Some(it) = world.items.get_mut(&serial) {
            it.name = text.clone();
        }
        return;
    }
    world.journal.push(JournalEntry {
        serial,
        name,
        text,
        msg_type,
        hue,
    });
}

fn ascii_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&c| c as char).collect()
}

/// Decode a big-endian UTF-16 string, stopping at a NUL char.
fn unicode_string(bytes: &[u8]) -> String {
    let mut out = String::new();
    for pair in bytes.chunks_exact(2) {
        let c = u16::from_be_bytes([pair[0], pair[1]]);
        if c == 0 {
            break;
        }
        out.push(char::from_u32(c as u32).unwrap_or('\u{FFFD}'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::packet::PacketWriter;

    fn target_packet(target_type: u8, cursor_id: u32, flag: u8) -> Vec<u8> {
        let mut p = PacketWriter::new();
        p.u8(0x6C).u8(target_type).u32(cursor_id).u8(flag);
        p.zeros(12); // serial+x+y+z+graphic fields (server sends zero on request)
        p.into_vec()
    }

    #[test]
    fn container_content_refresh_and_stale_drop() {
        let mut w = World::new();
        // Pre-existing item in container 0xBAG that the refresh will NOT include.
        let old = w.item_mut(0x111);
        old.container = Some(0x4000_0BA6);

        // 0x3C: one item (a pickaxe, graphic 0x0E86) in container 0xBAG.
        let mut p = PacketWriter::new();
        p.u8(0x3C).u16(0).u16(1); // id, len, count
        p.u32(0x222) // serial
            .u16(0x0E86)
            .u8(0) // graphic + inc
            .u16(1) // amount
            .u16(3)
            .u16(4) // slot x,y
            .u8(0) // grid
            .u32(0x4000_0BA6) // container
            .u16(0); // hue
        apply_packet(&mut w, &p.into_vec());

        let pick = w.items.get(&0x222).expect("pickaxe added to bag");
        assert_eq!(pick.graphic, 0x0E86);
        assert_eq!(pick.container, Some(0x4000_0BA6));
        // The stale item (not in the refresh) is dropped.
        assert!(w.items.get(&0x111).is_none());
    }

    #[test]
    fn skills_full_list_and_single_update() {
        let mut w = World::new();
        // Type 0x02: full list, 1-based ids, with caps, terminated by id 0.
        // Entry: Mining (45 → wire 46), value 500, base 480, lock 0, cap 1000.
        let mut p = PacketWriter::new();
        p.u8(0x3A).u16(0).u8(0x02); // id, len(placeholder), type
        p.u16(46).u16(500).u16(480).u8(0).u16(1000); // Mining
        p.u16(0); // terminator
        apply_packet(&mut w, &p.into_vec());
        let mining = w.skills.get(&45).expect("mining stored at 0-based id");
        assert_eq!((mining.value, mining.base, mining.cap), (500, 480, 1000));

        // Single update 0xDF (has cap, NOT 1-based): Mining base ticks to 482.
        let mut s = PacketWriter::new();
        s.u8(0x3A).u16(0).u8(0xDF);
        s.u16(45).u16(502).u16(482).u8(0).u16(1000);
        apply_packet(&mut w, &s.into_vec());
        assert_eq!(w.skills.get(&45).unwrap().base, 482);
    }

    #[test]
    fn target_cursor_sets_and_cancels() {
        let mut w = World::new();
        apply_packet(&mut w, &target_packet(1, 0xDEAD_BEEF, 0));
        let t = w.pending_target.expect("cursor stored");
        assert_eq!((t.target_type, t.cursor_id, t.cursor_flag), (1, 0xDEAD_BEEF, 0));

        // flag == 3 is a withdrawal: it clears any pending cursor.
        apply_packet(&mut w, &target_packet(1, 0xDEAD_BEEF, 3));
        assert!(w.pending_target.is_none());
    }

    #[test]
    fn mobile_moving_updates_world() {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0x77)
            .u32(0xABCD)
            .u16(0x0190) // body
            .u16(100) // x
            .u16(200) // y
            .u8(5i8 as u8) // z
            .u8(0x03) // dir
            .u16(0) // hue
            .u8(0) // flags
            .u8(1); // notoriety
        assert!(apply_packet(&mut w, &p.into_vec()));
        let m = &w.mobiles[&0xABCD];
        assert_eq!((m.pos.x, m.pos.y, m.pos.z), (100, 200, 5));
        assert_eq!(m.body, 0x0190);
        assert_eq!(m.notoriety, 1);
    }

    #[test]
    fn delete_removes_entity() {
        let mut w = World::new();
        w.mobile_mut(0x55);
        let mut p = PacketWriter::new();
        p.u8(0x1D).u32(0x55);
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert!(!w.mobiles.contains_key(&0x55));
    }

    #[test]
    fn vital_hits_update() {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0xA1).u32(0x77).u16(120).u16(95); // max, cur
        assert!(apply_packet(&mut w, &p.into_vec()));
        let m = &w.mobiles[&0x77];
        assert_eq!((m.hits, m.hits_max), (95, 120));
    }

    #[test]
    fn ascii_talk_to_journal() {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0x1C)
            .u16(0) // length placeholder
            .u32(0x01)
            .u16(0) // graphic
            .u8(0) // type (regular)
            .u16(33) // hue
            .u16(3) // font
            .fixed_ascii("Hastin", 30)
            .bytes(b"hello there\0");
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.journal.len(), 1);
        assert_eq!(w.journal[0].name, "Hastin");
        assert_eq!(w.journal[0].text, "hello there");
    }

    #[test]
    fn unknown_packet_ignored() {
        let mut w = World::new();
        // 0x9B is fixed-len but not handled → recognized=false
        assert!(!apply_packet(&mut w, &[0x9B, 0, 0]));
    }
}

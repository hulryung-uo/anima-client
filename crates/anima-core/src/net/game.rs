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
use crate::world::{
    Effect, Gump, JournalEntry, PopupEntry, PopupMenu, PromptState, Skill, TargetCursor, TradeState,
    World,
};

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
        0x2E => equip_update(world, frame)?,
        0x1A => world_item(world, frame)?,
        0xF3 => world_item_hs(world, frame)?,
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
        0xC1 => cliloc_message(world, frame)?,
        0xCC => cliloc_affix(world, frame)?,
        0x0B => damage(world, frame)?,
        0x70 => graphic_effect(world, frame, false)?,
        0xC0 => graphic_effect(world, frame, true)?,
        0xC7 => graphic_effect(world, frame, true)?,
        0x54 => play_sound(world, frame)?,
        0x6E => character_anim(world, frame)?,
        0xE2 => typed_anim(world, frame)?,
        0x6D => play_music(world, frame)?,
        0x72 => war_mode(world, frame)?,
        0x4F => overall_light(world, frame)?,
        0x4E => personal_light(world, frame)?,
        0x65 => weather(world, frame)?,
        0xBC => season(world, frame)?,
        0x74 => open_buy_window(world, frame)?,
        0x9E => sell_list(world, frame)?,
        0xDF => buff(world, frame)?,
        0xB0 => display_gump(world, frame)?,
        0xDD => display_gump_packed(world, frame)?,
        0xBA => quest_arrow(world, frame)?,
        0xD6 => mega_cliloc(world, frame)?,
        0xDC => opl_info(world, frame)?,
        0x93 => open_book(world, frame)?,
        0xD4 => open_book_new(world, frame)?,
        0x66 => book_data(world, frame)?,
        0xAF => display_death(world, frame)?,
        0xAA => change_combatant(world, frame)?,
        0x27 => lift_reject(world, frame)?,
        0x89 => corpse_equip(world, frame)?,
        0xC2 => unicode_prompt(world, frame)?,
        0x6F => secure_trade(world, frame)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Status-flags bit for "hidden" on the mobile-update packets (0x20/0x77/
/// 0x78). ServUO `Mobile.cs GetPacketFlags`: 0x04 Poisoned, 0x08
/// YellowHealth, 0x40 WarMode, 0x80 Hidden — we only need the Hidden bit.
const FLAG_HIDDEN: u8 = 0x80;

/// 0x20 MobileUpdate — position/appearance reset. This is always about OUR
/// OWN mobile (ServUO sends it only to the mobile itself), so its flags byte
/// is the self-hidden feedback path: e.g. right after the Hiding skill
/// succeeds, or a GM `[set Hidden true`.
fn mobile_update(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let body = r.u16()?;
    r.skip(1)?; // graphic_inc
    let hue = r.u16()?;
    let flags = r.u8()?;
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
    m.hidden = flags & FLAG_HIDDEN != 0;
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

/// 0xF3 WorldItemHS — a ground item, the modern form ServUO sends to 7.0.9+
/// clients (supersedes 0x1A). `[id][unk:u16][type:u8][serial:u32][graphic:u16]
/// [inc:u8][amount:u16][amount2:u16][x:u16][y:u16][z:i8][direction:u8][hue:u16][flags:u8]`.
/// `type == 2` is a multi (house/boat), not a normal item — skipped. The
/// `direction` byte only matters for a corpse (`graphic == 0x2006`), which uses it
/// to orient the death-pose sprite (ClassicUO `UpdateItemSA`/`Item.Direction`;
/// same wire byte it also reuses as `LightID` for non-corpse items, which we don't
/// model).
fn world_item_hs(world: &mut World, frame: &[u8]) -> PResult<()> {
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
    if data_type == 0x02 {
        return Ok(()); // multi — not a pickable item
    }
    let it = world.item_mut(serial);
    it.graphic = graphic.wrapping_add(graphic_inc as u16);
    it.pos.x = x;
    it.pos.y = y;
    it.pos.z = z;
    it.hue = hue;
    it.amount = amount.max(1);
    it.container = None;
    it.layer = 0;
    it.direction = direction & 0x07;
    Ok(())
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
    let flags = r.u8()?;
    let notoriety = r.u8()?;

    // The Walker owns the player's own position/facing (prediction + ConfirmWalk).
    // A server MobileMoving *about us* must never overwrite it — that resets our
    // facing to a stale value and fights the walker, causing the turn/stall
    // direction oscillation. Mirror anima: ignore self here.
    if world.is_player(serial) {
        return Ok(());
    }

    let m = world.mobile_mut(serial);
    m.body = body;
    m.pos.x = x;
    m.pos.y = y;
    m.pos.z = z;
    m.direction = direction;
    m.hue = hue;
    m.notoriety = notoriety;
    m.hidden = flags & FLAG_HIDDEN != 0;
    Ok(())
}

/// 0x2E EquipUpdate — a single item equipped on a mobile (worn after the initial
/// 0x78, e.g. mounting puts the mount item on layer 0x19, or wearing/removing
/// gear). Without this, equip changes never reach the World — so a mount you put
/// on never appears (the client can't draw it) and `player_mounted()` stays false.
/// Format: serial(u32) graphic(u16) 0(u8) layer(u8) parent(u32) hue(u16).
fn equip_update(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let item_serial = r.u32()?;
    let graphic = r.u16()?;
    r.skip(1)?; // separator byte (RunUO writes a 0 between graphic and layer)
    let layer = r.u8()?;
    let parent = r.u32()?;
    let hue = r.u16()?;
    let it = world.item_mut(item_serial);
    it.graphic = graphic;
    it.layer = layer;
    it.hue = hue;
    it.container = Some(parent);
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
    let flags = r.u8()?;
    let notoriety = r.u8()?;

    // For self, the Walker owns position/facing — only refresh body/hue, never
    // pos/dir (see mobile_moving). Still parse the worn-item list below.
    let is_self = world.is_player(serial);
    {
        let m = world.mobile_mut(serial);
        m.body = body;
        m.hue = hue;
        // Hidden is a visual flag like body/hue, not movement state — refresh it
        // for self too (the self-hidden feedback path also flows through 0x78,
        // e.g. re-entering view after a facet change while hidden).
        m.hidden = flags & FLAG_HIDDEN != 0;
        if !is_self {
            m.pos.x = x;
            m.pos.y = y;
            m.pos.z = z;
            m.direction = direction;
            m.notoriety = notoriety;
        }
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
    Ok(())
}

/// 0x0B Damage — `[id][serial:u32][damage:u16]` (7 bytes). `serial` just took
/// `damage` HP; the renderer floats a number over it. (ClassicUO `Damage` /
/// `case 0x0B`.)
fn damage(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let amount = r.u16()?;
    world.push_damage(serial, amount);
    Ok(())
}

/// 0x70 GraphicalEffect / 0xC0 HuedEffect / 0xC7 ParticleEffect — a spell bolt,
/// hit sparkle, explosion, or field visual. All three share the 28-byte 0x70 core
/// (big-endian): `[id][type:u8][src:u32][tgt:u32][graphic:u16][sx:u16][sy:u16]
/// [sz:i8][tx:u16][ty:u16][tz:i8][speed:u8][duration:u8][unk:u16][fixedDir:u8]
/// [explode:u8]`. 0xC0 (36 B) then adds `[hue:u32][renderMode:u32]`; 0xC7 (49 B)
/// adds 13 further particle bytes the 2D client ignores (rendered like 0xC0).
/// `hued` = false for 0x70 (hue forced to 0), true for 0xC0/0xC7 (low 16 bits of
/// the hue u32). Ported from ClassicUO `PacketHandlers.GraphicEffect`.
fn graphic_effect(world: &mut World, frame: &[u8], hued: bool) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let kind = r.u8()?;
    let src_serial = r.u32()?;
    let tgt_serial = r.u32()?;
    let graphic = r.u16()?;
    let sx = r.u16()?;
    let sy = r.u16()?;
    let sz = r.i8()?;
    let tx = r.u16()?;
    let ty = r.u16()?;
    let tz = r.i8()?;
    let speed = r.u8()?;
    let duration = r.u8()?;
    r.skip(2)?; // unknown
    r.skip(1)?; // fixed direction
    r.skip(1)?; // explode flag
    // 0xC0/0xC7 carry a 32-bit hue (only the low 16 bits matter); the renderMode
    // u32 and any 0xC7 particle extras are ignored by the 2D client.
    let hue = if hued { r.u32()? as u16 } else { 0 };
    world.push_effect(Effect {
        seq: 0,
        kind,
        src_serial,
        tgt_serial,
        graphic,
        sx,
        sy,
        sz,
        tx,
        ty,
        tz,
        speed,
        duration,
        hue,
    });
    Ok(())
}

/// 0x54 PlaySoundEffect — `[id][mode:u8][soundID:u16][volume:u16][x:u16][y:u16][z:u16]`
/// (12 bytes). The (x, y) is where the sound originates — the renderer uses it to
/// attenuate volume + pan by distance from the player (ClassicUO-style).
fn play_sound(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    r.skip(1)?; // mode (0 = one-shot, 1 = repeating)
    let sound_id = r.u16()?;
    r.skip(2)?; // volume (server-side; we compute our own from distance)
    let x = r.u16()?;
    let y = r.u16()?;
    world.push_sound(sound_id, x, y);
    Ok(())
}

/// 0x6E CharacterAnimation — `[id][serial:u32][action:u16][frameCount:u16]
/// [repeatCount:u16][dir:u8][repeat:u8][delay:u8]` (14 bytes). Tells `serial` to
/// play animation group `action` once (combat swing, bow shot, get-hit, bow/salute
/// gesture, …). `dir == 0` plays forward. We queue it; the renderer plays the
/// matching frames then reverts to the idle/walk pose.
fn character_anim(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let action = r.u16()?;
    let frame_count = r.u16()?;
    r.skip(2)?; // repeat count (we play once)
    let dir = r.u8()?; // 0 = forward
    r.skip(1)?; // repeat flag
    let delay = r.u8()?;
    world.push_anim(serial, action, frame_count, dir == 0, delay);
    Ok(())
}

/// 0xE2 NewMobileAnimation — `[id][serial:u32][type:u16][action:u16][mode:u8]`
/// (10 bytes, ServUO `NewMobileAnimation : base(0xE2, 10)`). Sent by
/// `Mobile.Animate(AnimationType, action)` (e.g. the `.bow`/`.salute` text
/// emotes, spell-cast gestures, alerts, …) — `type` is the `AnimationType` enum
/// (0=Attack 1=Parry 2=Block 3=Die 4=Impact 5=Fidget 6=Eat 7=Emote 8=Alert
/// 9=TakeOff 10=Land 11=Spell 12=StartCombat 13=EndCombat 14=Pillage 15=Spawn),
/// not a raw animation group like 0x6E's `action`. `mode` is nominally a "delay"
/// (ServUO fills it with `Utility.Random(0, 60)`) but ClassicUO never uses it for
/// timing here — `Mobile.SetAnimation` is called with the default interval — it
/// only feeds `(mode % 2/3/4)` inside `Mobile.GetObjectNewAnimation` to pick
/// between cosmetically-equivalent variants of the same emote. We store the raw
/// triple; the renderer (which alone knows the body's animation-group layout)
/// converts `(type, action, mode)` to a real group, mirroring ClassicUO's
/// `GetObjectNewAnimation`/`GetObjectNewAnimationType_*`.
fn typed_anim(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let kind = r.u16()?;
    let action = r.u16()?;
    let mode = r.u8()?;
    world.push_typed_anim(serial, kind, action, mode);
    Ok(())
}

/// 0x6D PlayMusic — `[id][musicID:u16]` (3 bytes). Records the current track.
fn play_music(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let music_id = r.u16()?;
    // 0xFFFF is the conventional "stop music" sentinel.
    world.current_music = if music_id == 0xFFFF { None } else { Some(music_id) };
    Ok(())
}

/// 0x4F OverallLightLevel — `[id][level:u8]` (2 bytes). 0 = brightest day,
/// ~0x1F darkest night. The renderer darkens the scene by this level.
/// 0x72 SetWarMode — `[id][flag:u8][0x00][0x32][0x00]` (5 bytes). The server
/// echoes our authoritative war/peace stance: `flag` != 0 = war. ClassicUO reads
/// only the first byte after the id and ignores the trailing 3 (fixed padding).
fn war_mode(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.war = r.u8()? != 0;
    Ok(())
}

fn overall_light(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.light_level = r.u8()?;
    Ok(())
}

/// 0x4E PersonalLightLevel — `[id][serial:u32][level:u8]` (6 bytes). Stored only
/// for our own character; combined with the overall level in
/// [`World::effective_light`].
fn personal_light(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let level = r.u8()?;
    if world.is_player(serial) {
        world.personal_light = Some(level);
    }
    Ok(())
}

/// 0x65 Weather — `[id][type:u8][count:u8][temperature:u8]` (4 bytes). `type`:
/// 0 = rain, 1 = fierce storm, 2 = snow, 3 = storm, 0xFE/0xFF = none/reset.
/// `count` is the particle count (intensity). Temperature is unused here.
fn weather(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let kind = r.u8()?;
    let count = r.u8()?;
    let _temperature = r.u8()?;
    world.weather.kind = kind;
    world.weather.intensity = count;
    Ok(())
}

/// 0xBC Season — `[id][season:u8][playMusic:u8]` (3 bytes). `season`:
/// 0=Spring, 1=Summer, 2=Fall, 3=Winter, 4=Desolation. `playMusic` (whether the
/// client should (re)start seasonal music) is not used here. We only store the
/// season so the renderer can tint the scene; tree/foliage graphic remap is not
/// attempted.
fn season(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.season = r.u8()?;
    let _play_music = r.u8()?;
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
fn buff(world: &mut World, frame: &[u8]) -> PResult<()> {
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
fn buff_name(icon: u16) -> String {
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
fn open_buy_window(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 4 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let container = r.u32()?;
    let count = r.u8()?;
    // The vendor mobile owns the for-sale container (set when it entered view as a
    // worn shop layer). 0 if we haven't seen the linkage yet.
    let vendor = world.items.get(&container).and_then(|it| it.container).unwrap_or(0);
    let mut entries = Vec::with_capacity(count as usize);
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
        entries.push((price, name));
    }
    world.shop_buy = Some(crate::world::ShopBuy { vendor, container, entries });
    Ok(())
}

/// 0x9E SellList — the items a vendor will buy *from* our pack, with the price it
/// pays. Variable: `[id][len:u16][vendor:u32][count:u16]` then `count` ×
/// `[serial:u32][graphic:u16][hue:u16][amount:u16][price:u16][nameLen:u16][name:ascii]`.
/// `vendor` is the vendor mobile serial a SELL request (0x9F) is addressed to. A
/// new list replaces any old one.
fn sell_list(world: &mut World, frame: &[u8]) -> PResult<()> {
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
        items.push(crate::world::ShopSellItem { serial, graphic, hue, amount, price, name });
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
fn display_gump(world: &mut World, frame: &[u8]) -> PResult<()> {
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
    world.add_gump(Gump { serial, gump_id, x, y, layout, text });
    Ok(())
}

/// 0xDD DisplayGumpPacked — the zlib-compressed form of 0xB0. Variable:
/// `[id][len:u16][serial:u32][gumpId:u32][x:u32][y:u32]` then a compressed layout
/// block `[compLen+4:u32][decompLen:u32][zlib: compLen bytes]`, then
/// `[textLinesCount:u32]`, then (only if count > 0) a compressed text block in the
/// same `[compLen+4][decompLen][zlib]` shape. Both inflated blocks have the same
/// content as 0xB0 (ASCII layout; `count` × `[charLen:u16][utf16-be]`). Ported
/// from ClassicUO `PacketHandlers.OpenCompressedGump` + ServUO `DisplayGumpPacked`.
fn display_gump_packed(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 3 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let serial = r.u32()?;
    let gump_id = r.u32()?;
    let x = r.u32()? as i32;
    let y = r.u32()? as i32;

    let layout_bytes = read_zlib_block(&mut r)?;
    let layout = String::from_utf8_lossy(&layout_bytes).trim_end_matches('\0').to_string();

    let count = r.u32()? as usize;
    let text = if count > 0 {
        let text_bytes = read_zlib_block(&mut r)?;
        let mut tr = PacketReader::new(&text_bytes);
        read_gump_text_lines(&mut tr, count)
    } else {
        Vec::new()
    };
    world.add_gump(Gump { serial, gump_id, x, y, layout, text });
    Ok(())
}

/// Read a 0xDD compressed block: `[compLen+4:u32][decompLen:u32][zlib bytes]` and
/// return the inflated payload. The first u32 counts the 4-byte decompLen field
/// plus the zlib bytes, so the zlib data is `first - 4` bytes. A decode failure
/// (or a zero/short block) yields an empty buffer rather than erroring the stream.
fn read_zlib_block(r: &mut PacketReader) -> PResult<Vec<u8>> {
    let packed_len = r.u32()? as usize;
    if packed_len < 4 {
        return Ok(Vec::new()); // ServUO writes a bare 0 u32 for an empty block
    }
    let _decomp_len = r.u32()?;
    let zlib = r.bytes(packed_len - 4)?;
    // The protocol mandates zlib here; miniz_oxide is a pure-Rust, wasm-clean
    // inflate (the one justified non-std dep in core). A corrupt block is skipped.
    Ok(miniz_oxide::inflate::decompress_to_vec_zlib(zlib).unwrap_or_default())
}

/// Read `count` gump text lines, each `[charLen:u16][text: utf16-be, charLen*2
/// bytes]`. `charLen` is a UTF-16 code-unit count (not a byte count). Stops early
/// if the buffer runs out (a truncated/odd line yields an empty string).
fn read_gump_text_lines(r: &mut PacketReader, count: usize) -> Vec<String> {
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        let char_len = match r.u16() {
            Ok(n) => n as usize,
            Err(_) => break,
        };
        match r.bytes(char_len * 2) {
            Ok(b) => lines.push(unicode_string(b)),
            Err(_) => {
                lines.push(String::new());
                break;
            }
        }
    }
    lines
}

/// 0xBA QuestArrow — show/hide the on-screen arrow pointing at a tile.
/// `[id][active:u8][x:u16][y:u16]` (classic 6 bytes); the modern/HS form appends a
/// `[serial:u32]` (10 bytes) which we read past and ignore. `active == 0` hides the
/// arrow (clears `quest_arrow`); otherwise it points at `(x, y)`. Ported from
/// ClassicUO `PacketHandlers.SetQuestArrow`.
fn quest_arrow(world: &mut World, frame: &[u8]) -> PResult<()> {
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
fn mega_cliloc(world: &mut World, frame: &[u8]) -> PResult<()> {
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
/// `[id][serial:u32][revision:u32]`. Tells the client `serial`'s current tooltip
/// revision; if it differs from the cached one the client should re-request the
/// full 0xD6. We just record the revision (the hover flow re-requests on demand),
/// so this is effectively a lightweight note, not an action.
fn opl_info(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let revision = r.u32()?;
    world.opl_revision.insert(serial, revision);
    Ok(())
}

/// 0x93 OpenBook — the (legacy, fixed 99-byte) book header.
/// `[id][serial:u32][writable:u8][unk:u8][pageCount:u16][title:60 ascii][author:30 ascii]`.
/// Sets `world.book` with `page_count` empty pages; the content arrives via 0x66.
fn open_book(world: &mut World, frame: &[u8]) -> PResult<()> {
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
fn open_book_new(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 3 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let serial = r.u32()?;
    let writable = r.u8()? != 0;
    r.skip(1)?; // unknown
    let page_count = r.u16()?;
    let title_len = r.u16()? as usize;
    let title = String::from_utf8_lossy(r.bytes(title_len)?).trim_end_matches('\0').to_string();
    let author_len = r.u16()? as usize;
    let author = String::from_utf8_lossy(r.bytes(author_len)?).trim_end_matches('\0').to_string();
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
fn book_data(world: &mut World, frame: &[u8]) -> PResult<()> {
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

/// Read a NUL-terminated ASCII string from the reader (consuming the NUL). Stops at
/// end-of-buffer if no NUL is found.
fn read_nul_ascii(r: &mut PacketReader) -> String {
    let mut s = String::new();
    while let Ok(b) = r.u8() {
        if b == 0 {
            break;
        }
        s.push(b as char);
    }
    s
}

/// 0xAF DisplayDeath — `[id][killedSerial:u32][corpseSerial:u32][unused:u32=0]`
/// (13 bytes, ServUO `DeathAnimation : base(0xAF, 13)`). Sent on every mobile
/// death; links the new corpse item to the mobile that died. AI-facing only (no
/// death animation is modeled — no rendering in core); the renderer needs nothing
/// from this (the corpse item already carries its own body/hue/direction).
fn display_death(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let killed_serial = r.u32()?;
    let corpse_serial = r.u32()?;
    r.skip(4)?; // unused (ServUO always writes 0)
    if corpse_serial != 0 {
        world.set_corpse_of(corpse_serial, killed_serial);
    }
    Ok(())
}

/// 0xAA ChangeCombatant — `[id][serial:u32]` (5 bytes, ServUO `ChangeCombatant :
/// base(0xAA, 5)`), sent whenever the server's `Mobile.Combatant` changes
/// (Mobile.cs ~2213). `serial == 0` means combat ended.
fn change_combatant(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    world.combatant = if serial == 0 { None } else { Some(serial) };
    Ok(())
}

/// 0x27 LiftRej — `[id][reason:u8]` (2 bytes). The server refused our last lift
/// (0x07 PickUp): the item never left its source. See [`World::recent_lift_rejects`]
/// for the reason-code table.
fn lift_reject(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let reason = r.u8()?;
    world.push_lift_reject(reason);
    Ok(())
}

/// 0x89 CorpseEquip — a corpse's worn-item layout, so it can be "undressed"
/// without opening its loot window first. Variable: `[id][len:u16][corpse:u32]`
/// then repeated `[layer:u8][serial:u32]` until `layer == 0` (Layer.Invalid
/// terminator). The wire layer is `real layer + 1` (ServUO
/// `Scripts/Items/Corpses/Packets.cs` `CorpseEquip`, CUO `CorpseEquipment`); we
/// store the un-shifted real layer. A truncated frame keeps whatever entries
/// parsed cleanly before it ran out. Ported from ClassicUO `PacketHandlers.CorpseEquipment`.
fn corpse_equip(world: &mut World, frame: &[u8]) -> PResult<()> {
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

/// 0xC2 UnicodePrompt — the server asks us to answer with typed text (pet rename,
/// house sign, guild abbreviation, … — ~38 ServUO flows). Fixed 21 bytes as
/// ServUO sends it: `[id][len:u16][senderSerial:u32][promptId:u32][type:u32=0]
/// [language:u32=0][textLen:u16=0]` — the question text itself is NOT carried
/// here (ServUO sends it separately as a cliloc/system message just before this,
/// which already lands in [`World::journal`]); only the two ids the response must
/// echo matter (mirrors ClassicUO `PacketHandlers.UnicodePrompt`, which reads just
/// the leading 8 bytes as one `u64`). Answer with
/// [`crate::agent::Action::PromptResponse`]/[`crate::agent::Action::PromptCancel`].
fn unicode_prompt(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 11 {
        return Ok(());
    }
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let sender_serial = r.u32()?;
    let prompt_id = r.u32()?;
    world.prompt = Some(PromptState { sender_serial, prompt_id });
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
fn secure_trade(world: &mut World, frame: &[u8]) -> PResult<()> {
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
            let opponent_name = if r.remaining() >= 30 { r.fixed_ascii(30)? } else { String::new() };
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
/// stack (sub 0x01 sets six keys, sub 0x02 pushes one; each walk consumes one),
/// party (sub 0x06), the facet switch (sub 0x08), the popup menu (sub 0x14),
/// and spellbook content (sub 0x1B).
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
        0x1B => parse_spellbook_content(world, &mut r)?,
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
fn parse_spellbook_content(world: &mut World, r: &mut PacketReader) -> PResult<()> {
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
fn parse_party(world: &mut World, r: &mut PacketReader) -> PResult<()> {
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
fn parse_popup(world: &mut World, r: &mut PacketReader) -> PResult<()> {
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
        entries.push(PopupEntry { index, cliloc, flags });
    }
    world.popup = Some(PopupMenu { serial, entries });
    Ok(())
}

/// 0xC1 ClilocMessage — a localized system message with optional args.
/// `[id][len:u16][serial:u32][graphic:u16][type:u8][hue:u16][font:u16][cliloc:u32][name:30][args:utf16-LE]`.
/// We keep the cliloc id + raw args; the brain resolves them against the Cliloc table.
fn cliloc_message(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 48 {
        return Ok(());
    }
    let serial = u32::from_be_bytes([frame[3], frame[4], frame[5], frame[6]]);
    let msg_type = frame[9];
    let hue = u16::from_be_bytes([frame[10], frame[11]]);
    let cliloc = u32::from_be_bytes([frame[14], frame[15], frame[16], frame[17]]);
    let name = ascii_string(&frame[18..48]);
    let args = decode_unicode(&frame[48..], false); // 0xC1 args are little-endian
    push_journal_cliloc(world, serial, name, args, msg_type, hue, cliloc);
    Ok(())
}

/// 0xCC ClilocMessageAffix — like 0xC1 plus a 1-byte flag, a NUL-terminated ASCII
/// affix after the name, and **big-endian** args. The affix is appended to the text.
fn cliloc_affix(world: &mut World, frame: &[u8]) -> PResult<()> {
    if frame.len() < 49 {
        return Ok(());
    }
    let serial = u32::from_be_bytes([frame[3], frame[4], frame[5], frame[6]]);
    let msg_type = frame[9];
    let hue = u16::from_be_bytes([frame[10], frame[11]]);
    let cliloc = u32::from_be_bytes([frame[14], frame[15], frame[16], frame[17]]);
    // frame[18] = affix flags (prepend/system) — not needed for a plain append.
    let name = ascii_string(&frame[19..49]);
    let affix_start = 49;
    let nul = frame[affix_start..]
        .iter()
        .position(|&b| b == 0)
        .map_or(frame.len(), |p| affix_start + p);
    let affix = ascii_string(&frame[affix_start..nul]);
    let args_start = (nul + 1).min(frame.len());
    let mut text = decode_unicode(&frame[args_start..], true); // 0xCC args are big-endian
    text.push_str(&affix);
    push_journal_cliloc(world, serial, name, text, msg_type, hue, cliloc);
    Ok(())
}

/// Decode a UTF-16 string (LE or BE), stopping at the first NUL.
fn decode_unicode(bytes: &[u8], big_endian: bool) -> String {
    let mut out = String::new();
    for pair in bytes.chunks_exact(2) {
        let c = if big_endian {
            u16::from_be_bytes([pair[0], pair[1]])
        } else {
            u16::from_le_bytes([pair[0], pair[1]])
        };
        if c == 0 {
            break;
        }
        out.push(char::from_u32(c as u32).unwrap_or('\u{FFFD}'));
    }
    out
}

fn push_journal(world: &mut World, serial: u32, name: String, text: String, msg_type: u8, hue: u16) {
    push_journal_cliloc(world, serial, name, text, msg_type, hue, 0);
}

fn push_journal_cliloc(
    world: &mut World,
    serial: u32,
    name: String,
    text: String,
    msg_type: u8,
    hue: u16,
    cliloc: u32,
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
    let name = if name.is_empty() { "System".to_string() } else { name };
    world.journal.push(JournalEntry {
        serial,
        name,
        text,
        msg_type,
        hue,
        cliloc,
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
    fn mega_cliloc_parses_property_lines() {
        // Two property lines for serial 0xDEADBEEF, revision 0x12345678.
        // Line 0: cliloc 1050045 with args "\t\tLongsword" (a name template).
        // Line 1: cliloc 1060403 with args "15" (e.g. "physical damage 15%").
        let mut p = PacketWriter::new();
        p.u8(0xD6).u16(0); // id + length placeholder
        p.u16(0x0001) // unknown
            .u32(0xDEAD_BEEF) // serial
            .u8(0)
            .u8(0) // two zero bytes
            .u32(0x1234_5678); // revision
        let put_line = |p: &mut PacketWriter, cliloc: u32, args: &str| {
            let units: Vec<u8> = args
                .encode_utf16()
                .flat_map(|u| u.to_le_bytes()) // UTF-16 LE args
                .collect();
            p.u32(cliloc).u16(units.len() as u16).bytes(&units);
        };
        put_line(&mut p, 1_050_045, "\t\tLongsword");
        put_line(&mut p, 1_060_403, "15");
        p.u32(0); // terminator
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;

        let mut w = World::new();
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.opl_revision.get(&0xDEAD_BEEF), Some(&0x1234_5678));
        let lines = w.opl.get(&0xDEAD_BEEF).expect("opl stored");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], (1_050_045, "\t\tLongsword".to_string()));
        assert_eq!(lines[1], (1_060_403, "15".to_string()));

        // 0xDC OPLInfo updates just the revision hash.
        let mut q = PacketWriter::new();
        q.u8(0xDC).u32(0xDEAD_BEEF).u32(0x9999_0000);
        assert!(apply_packet(&mut w, &q.into_vec()));
        assert_eq!(w.opl_revision.get(&0xDEAD_BEEF), Some(&0x9999_0000));
    }

    #[test]
    fn popup_menu_modern_and_legacy() {
        // Modern (version 2): [cliloc u32][index u16][flags u16] per entry.
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0); // id + len placeholder
        p.u16(0x0014) // subcommand
            .u16(0x0002) // version 2
            .u32(0xDEAD_BEEF) // serial
            .u8(2); // count
        p.u32(3_000_122).u16(0).u16(0x0000); // entry 0
        p.u32(3_006_111).u16(1).u16(0x0001); // entry 1 (flag 0x01)
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;

        let mut w = World::new();
        assert!(apply_packet(&mut w, &frame));
        let menu = w.popup.as_ref().expect("popup set");
        assert_eq!(menu.serial, 0xDEAD_BEEF);
        assert_eq!(menu.entries.len(), 2);
        assert_eq!(menu.entries[0], PopupEntry { index: 0, cliloc: 3_000_122, flags: 0 });
        assert_eq!(menu.entries[1], PopupEntry { index: 1, cliloc: 3_006_111, flags: 1 });

        // Legacy (version 1): [index u16][cliloc-3000000 u16][flags u16].
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0);
        p.u16(0x0014).u16(0x0001).u32(0x0102_0304).u8(1);
        p.u16(7).u16(122).u16(0x0000); // index 7, cliloc 3000122
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));
        let menu = w.popup.as_ref().expect("popup set");
        assert_eq!(menu.serial, 0x0102_0304);
        assert_eq!(menu.entries, vec![PopupEntry { index: 7, cliloc: 3_000_122, flags: 0 }]);
    }

    #[test]
    fn spellbook_content_parses_and_prunes_on_delete() {
        let mut w = World::new();
        // 0xBF/0x1B NewSpellbookContent: magery book (graphic 0x0EFA, ServUO
        // BookOffset 0 -> offset 1) knows spells 1 (Clumsy) and 4 (Heal) — bits
        // 0 and 3 of the mask, content = 0b1001 = 0x9. `content` is written
        // byte-by-byte LSB-first (see `parse_spellbook_content`'s doc), unlike
        // the rest of the wire (big-endian).
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0); // id + len placeholder
        p.u16(0x001B) // subcommand
            .u16(0x0001) // unknown, always 1
            .u32(0x4000_0010) // book serial
            .u16(0x0EFA) // graphic (magery book ItemID)
            .u16(1) // offset = BookOffset(0) + 1
            .bytes(&[0x09, 0, 0, 0, 0, 0, 0, 0]); // content mask, LSB-first
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert_eq!(frame.len(), 23); // ServUO NewSpellbookContent EnsureCapacity(23)
        assert!(apply_packet(&mut w, &frame));

        let sb = w.spellbooks.get(&0x4000_0010).expect("spellbook content stored");
        assert_eq!(sb.graphic, 0x0EFA);
        assert_eq!(sb.offset, 1);
        assert_eq!(sb.content, 0x9);

        // The book is destroyed/despawned (0x1D Delete) — the entry is pruned with it.
        let mut d = PacketWriter::new();
        d.u8(0x1D).u32(0x4000_0010);
        assert!(apply_packet(&mut w, &d.into_vec()));
        assert!(!w.spellbooks.contains_key(&0x4000_0010));
    }

    fn party_frame(body: &[u8]) -> Vec<u8> {
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0).u16(0x0006);
        for &b in body {
            p.u8(b);
        }
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        frame
    }

    #[test]
    fn party_list_remove_invite_and_chat() {
        let mut w = World::new();
        w.party.pending_invite = Some(0xAAAA);

        // 0x01 member list: count 2, leader then member. Clears pending invite.
        let list =
            party_frame(&[0x01, 2, 0, 0, 0x11, 0x11, 0, 0, 0x22, 0x22]);
        assert!(apply_packet(&mut w, &list));
        assert_eq!(w.party.members, vec![0x0000_1111, 0x0000_2222]);
        assert_eq!(w.party.leader, 0x0000_1111);
        assert_eq!(w.party.pending_invite, None);

        // 0x02 remove: count 1, removed serial, then 1 remaining member.
        let remove =
            party_frame(&[0x02, 1, 0, 0, 0x22, 0x22, 0, 0, 0x11, 0x11]);
        assert!(apply_packet(&mut w, &remove));
        assert_eq!(w.party.members, vec![0x0000_1111]);
        assert_eq!(w.party.leader, 0x0000_1111);

        // 0x02 disband: count 0, removed serial, no members.
        let disband = party_frame(&[0x02, 0, 0, 0, 0x11, 0x11]);
        assert!(apply_packet(&mut w, &disband));
        assert!(w.party.members.is_empty());
        assert_eq!(w.party.leader, 0);

        // 0x07 invitation: leader serial → pending invite.
        let invite = party_frame(&[0x07, 0, 0, 0x33, 0x33]);
        assert!(apply_packet(&mut w, &invite));
        assert_eq!(w.party.pending_invite, Some(0x0000_3333));

        // 0x04 chat-to-all: from serial + UTF-16 BE text → journal.
        let mut body = vec![0x04, 0, 0, 0x11, 0x11];
        for u in "hi".encode_utf16() {
            body.extend_from_slice(&u.to_be_bytes());
        }
        let chat = party_frame(&body);
        assert!(apply_packet(&mut w, &chat));
        assert_eq!(w.journal.last().expect("party line").text, "hi");
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
        assert!(!w.items.contains_key(&0x111));
    }

    #[test]
    fn world_item_hs_parsed_as_ground_item() {
        let mut w = World::new();
        // 0xF3: a forge (graphic 0x0FB1) on the ground at (2566, 493, 19).
        let mut p = PacketWriter::new();
        p.u8(0xF3).u16(0x0001).u8(0x00); // id, unk, data_type=item
        p.u32(0x4000_1000).u16(0x0FB1).u8(0); // serial, graphic, inc
        p.u16(1).u16(1); // amount, amount2
        p.u16(2566).u16(493).u8(19i8 as u8); // x, y, z
        p.u8(0).u16(0).u8(0); // light, hue, flags
        apply_packet(&mut w, &p.into_vec());
        let it = w.items.get(&0x4000_1000).expect("ground item added");
        assert_eq!(it.graphic, 0x0FB1);
        assert_eq!((it.pos.x, it.pos.y), (2566, 493));
        assert_eq!(it.container, None);
    }

    #[test]
    fn world_item_hs_corpse_carries_body_and_direction() {
        let mut w = World::new();
        // 0xF3: a corpse (graphic 0x2006) — the dead creature's body (400 = human
        // male) rides in `amount`, its facing (south = 5) in the direction byte.
        let mut p = PacketWriter::new();
        p.u8(0xF3).u16(0x0001).u8(0x00); // id, unk, data_type=item
        p.u32(0x4000_2000).u16(0x2006).u8(0); // serial, graphic, inc
        p.u16(400).u16(400); // amount (body id), amount2 (repeated)
        p.u16(1500).u16(1600).u8(10i8 as u8); // x, y, z
        p.u8(5).u16(0x0044).u8(0); // direction, hue, flags
        apply_packet(&mut w, &p.into_vec());
        let it = w.items.get(&0x4000_2000).expect("corpse item added");
        assert_eq!(it.graphic, 0x2006);
        assert_eq!(it.amount, 400); // dead creature's body id
        assert_eq!(it.direction, 5);
        assert_eq!(it.hue, 0x0044);
    }

    #[test]
    fn world_item_legacy_corpse_direction_only_when_flagged() {
        let mut w = World::new();
        // 0x1A: a corpse (graphic 0x2006), body 0x00EE in `amount`, direction byte
        // present (x's 0x8000 flag) and hue present (y's 0x8000 flag).
        let mut p = PacketWriter::new();
        p.u8(0x1A).u16(0); // id, len (unused — frame is read from offset 3)
        p.u32(0x8000_0000 | 0x4000_1234); // serial | has_amount flag
        p.u16(0x2006); // graphic (corpse, no inc-flag bit)
        p.u16(0x00EE); // amount = body id
        p.u16(0x8000 | 1234); // x | direction-present flag
        p.u16(0x8000 | 5678); // y | hue-present flag
        p.u8(5); // direction (present because the x flag was set)
        p.u8((-2i8) as u8); // z
        p.u16(0x0033); // hue (present because the y flag was set)
        apply_packet(&mut w, &p.into_vec());
        let it = w.items.get(&0x4000_1234).expect("corpse item added");
        assert_eq!(it.graphic, 0x2006);
        assert_eq!(it.amount, 0x00EE);
        assert_eq!(it.direction, 5);
        assert_eq!(it.hue, 0x0033);
        assert_eq!((it.pos.x, it.pos.y, it.pos.z), (1234, 5678, -2));

        // A plain item (no direction/hue flags) leaves direction at its default 0.
        let mut w2 = World::new();
        let mut p2 = PacketWriter::new();
        p2.u8(0x1A).u16(0);
        p2.u32(0x4000_5555); // no has_amount flag
        p2.u16(0x0EED); // gold graphic, no inc
        p2.u16(100).u16(200).u8(0i8 as u8); // x, y (no flags), z
        apply_packet(&mut w2, &p2.into_vec());
        let it2 = w2.items.get(&0x4000_5555).expect("plain item added");
        assert_eq!(it2.direction, 0);
    }

    #[test]
    fn cliloc_message_keeps_id_and_args() {
        let mut w = World::new();
        // 0xC1: cliloc 1044625 ("You dig some ore...") with one LE-UTF16 arg "iron".
        let mut p = PacketWriter::new();
        p.u8(0xC1).u16(0); // id, len(placeholder)
        p.u32(0).u16(0).u8(0).u16(0).u16(3); // serial, graphic, type, hue, font
        p.u32(1044625); // cliloc
        p.zeros(30); // name (System)
        for ch in "iron".chars() {
            p.u16((ch as u16).swap_bytes()); // write LE by swapping (writer is BE)
        }
        apply_packet(&mut w, &p.into_vec());
        let e = w.journal.last().expect("cliloc journal line");
        assert_eq!(e.cliloc, 1044625);
        assert_eq!(e.text, "iron");
        assert_eq!(e.name, "System");
    }

    #[test]
    fn single_click_label_sets_mobile_name_not_journal() {
        let mut w = World::new();
        w.mobiles.insert(
            0x1234,
            crate::world::Mobile { serial: 0x1234, ..Default::default() },
        );
        // 0xC1 MessageLocalized as ServUO sends a single-click name: type 6 (Label),
        // cliloc 1050045 (the OPL name header `~1_val~`), the name as the sole arg.
        let mut p = PacketWriter::new();
        p.u8(0xC1).u16(0);
        p.u32(0x1234).u16(0).u8(6).u16(946).u16(3); // serial, graphic, type=6, hue, font
        p.u32(1050045); // cliloc = name header
        p.zeros(30); // name column (unused here)
        for ch in "Zurghed".chars() {
            p.u16((ch as u16).swap_bytes()); // LE-UTF16 arg
        }
        apply_packet(&mut w, &p.into_vec());
        // Stored on the mobile (drives the overhead label / hover), NOT scrolled in chat.
        assert_eq!(w.mobiles.get(&0x1234).unwrap().name, "Zurghed");
        assert!(w.journal.is_empty(), "a single-click name must not scroll in the journal");
    }

    #[test]
    fn regular_speech_still_journals_and_leaves_name_untouched() {
        let mut w = World::new();
        w.mobiles.insert(
            0x55,
            crate::world::Mobile { serial: 0x55, name: "Guard".into(), ..Default::default() },
        );
        // A normal (type 0) ascii talk from the same serial is chat, not a name.
        let mut p = PacketWriter::new();
        p.u8(0x1C).u16(0);
        p.u32(0x55).u16(0).u8(0).u16(0).u16(3); // serial, graphic, type=0, hue, font
        p.zeros(30); // name column
        p.bytes(b"halt!\0");
        apply_packet(&mut w, &p.into_vec());
        assert_eq!(w.mobiles.get(&0x55).unwrap().name, "Guard"); // name unchanged
        assert!(w.journal.iter().any(|e| e.text == "halt!"), "speech still journals");
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

    /// A fixed 0x77 MobileMoving frame with a chosen status-flags byte
    /// (`flags`), otherwise identical to `mobile_moving_updates_world`.
    fn mobile_moving_frame(serial: u32, flags: u8) -> Vec<u8> {
        let mut p = PacketWriter::new();
        p.u8(0x77)
            .u32(serial)
            .u16(0x0190) // body
            .u16(100) // x
            .u16(200) // y
            .u8(5i8 as u8) // z
            .u8(0x03) // dir
            .u16(0) // hue
            .u8(flags)
            .u8(1); // notoriety
        p.into_vec()
    }

    #[test]
    fn mobile_moving_hidden_flag_sets_and_clears() {
        let mut w = World::new();
        // Bit 0x80 set → hidden.
        assert!(apply_packet(&mut w, &mobile_moving_frame(0xBEEF, FLAG_HIDDEN)));
        assert!(w.mobiles[&0xBEEF].hidden);

        // A later update that omits the bit clears it back — not sticky.
        assert!(apply_packet(&mut w, &mobile_moving_frame(0xBEEF, 0x00)));
        assert!(!w.mobiles[&0xBEEF].hidden);
    }

    #[test]
    fn mobile_moving_no_hidden_flag_stays_false() {
        let mut w = World::new();
        assert!(apply_packet(&mut w, &mobile_moving_frame(0xCAFE, 0x00)));
        assert!(!w.mobiles[&0xCAFE].hidden);
    }

    /// A variable-length 0x78 MobileIncoming frame (id + u16 length + fixed
    /// fields, no worn-item records) with a chosen status-flags byte.
    fn mobile_incoming_frame(serial: u32, flags: u8) -> Vec<u8> {
        let mut p = PacketWriter::new();
        p.u8(0x78).u16(0); // id + length placeholder
        p.u32(serial)
            .u16(0x0190) // body
            .u16(100) // x
            .u16(200) // y
            .u8(5i8 as u8) // z
            .u8(0x03) // dir
            .u16(0) // hue
            .u8(flags)
            .u8(1); // notoriety
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        frame
    }

    #[test]
    fn mobile_incoming_hidden_flag_sets_and_clears() {
        let mut w = World::new();
        assert!(apply_packet(&mut w, &mobile_incoming_frame(0xABCD, FLAG_HIDDEN)));
        assert!(w.mobiles[&0xABCD].hidden);

        // A fresh 0x78 without the bit flips it back — proves it's not sticky.
        assert!(apply_packet(&mut w, &mobile_incoming_frame(0xABCD, 0x00)));
        assert!(!w.mobiles[&0xABCD].hidden);
    }

    #[test]
    fn mobile_update_hidden_flag_is_the_self_feedback_path() {
        // 0x20 MobileUpdate is fixed-length, no length prefix: serial, body,
        // graphic_inc, hue, flags, x, y, server_id, dir, z.
        let mut w = World::new();
        let build = |flags: u8| {
            let mut p = PacketWriter::new();
            p.u32(0x1001) // serial
                .u16(0x0190) // body
                .u8(0) // graphic_inc
                .u16(0) // hue
                .u8(flags)
                .u16(100) // x
                .u16(200) // y
                .u16(0) // server_id
                .u8(0x03) // dir
                .u8(5i8 as u8); // z
            let mut frame = p.into_vec();
            frame.insert(0, 0x20);
            frame
        };
        assert!(apply_packet(&mut w, &build(FLAG_HIDDEN)));
        assert!(w.mobiles[&0x1001].hidden);

        assert!(apply_packet(&mut w, &build(0x00)));
        assert!(!w.mobiles[&0x1001].hidden, "hidden must not be sticky");
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
    fn damage_queues_event() {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0x0B).u32(0x0000_1234).u16(17);
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert_eq!(w.recent_damage.last(), Some(&(1, 0x0000_1234, 17)));
        assert_eq!(w.damage_seq, 1);
    }

    #[test]
    fn play_sound_queues_event() {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0x54).u8(0).u16(0x0145).u16(0).u16(100).u16(200).u16(0);
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert_eq!(w.recent_sounds.last(), Some(&(1, 0x0145, 100, 200)));
        assert_eq!(w.sound_seq, 1);
    }

    #[test]
    fn graphic_effect_0x70_parsed() {
        let mut w = World::new();
        // 0x70: a Moving fireball (graphic 0x36D4) from 0xAAAA at (100,200,5)
        // to 0xBBBB at (110,210,5), speed 7, duration 30. Hue must be 0 for 0x70.
        let mut p = PacketWriter::new();
        p.u8(0x70)
            .u8(0) // type = Moving
            .u32(0xAAAA) // src serial
            .u32(0xBBBB) // tgt serial
            .u16(0x36D4) // graphic
            .u16(100).u16(200).u8(5i8 as u8) // src x,y,z
            .u16(110).u16(210).u8(5i8 as u8) // tgt x,y,z
            .u8(7) // speed
            .u8(30) // duration
            .u16(0) // unknown
            .u8(0) // fixed direction
            .u8(0); // explode
        let frame = p.into_vec();
        assert_eq!(frame.len(), 28); // 0x70 is 28 bytes
        assert!(apply_packet(&mut w, &frame));
        let e = w.recent_effects.last().expect("effect queued");
        assert_eq!(e.seq, 1);
        assert_eq!(e.kind, 0);
        assert_eq!((e.src_serial, e.tgt_serial), (0xAAAA, 0xBBBB));
        assert_eq!(e.graphic, 0x36D4);
        assert_eq!((e.sx, e.sy, e.sz), (100, 200, 5));
        assert_eq!((e.tx, e.ty, e.tz), (110, 210, 5));
        assert_eq!((e.speed, e.duration, e.hue), (7, 30, 0));
    }

    #[test]
    fn hued_effect_0xc0_carries_hue() {
        let mut w = World::new();
        // 0xC0: a FixedFrom effect on serial 0xCAFE with hue 0x0021 (low 16 bits
        // of the u32) and a renderMode u32 the client ignores.
        let mut p = PacketWriter::new();
        p.u8(0xC0)
            .u8(3) // type = FixedFrom
            .u32(0xCAFE).u32(0xCAFE)
            .u16(0x3728) // graphic
            .u16(50).u16(60).u8(0)
            .u16(50).u16(60).u8(0)
            .u8(10).u8(20)
            .u16(0).u8(0).u8(0)
            .u32(0x0000_0021) // hue u32
            .u32(0); // renderMode (ignored)
        let frame = p.into_vec();
        assert_eq!(frame.len(), 36); // 0xC0 is 36 bytes
        assert!(apply_packet(&mut w, &frame));
        let e = w.recent_effects.last().expect("effect queued");
        assert_eq!(e.kind, 3);
        assert_eq!(e.hue, 0x0021);
    }

    #[test]
    fn play_music_sets_and_stops() {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0x6D).u16(0x0009);
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert_eq!(w.current_music, Some(0x0009));

        let mut s = PacketWriter::new();
        s.u8(0x6D).u16(0xFFFF);
        assert!(apply_packet(&mut w, &s.into_vec()));
        assert_eq!(w.current_music, None);
    }

    #[test]
    fn overall_light_level_stored() {
        let mut w = World::new();
        assert_eq!(w.light_level, 0); // default = brightest day
        let mut p = PacketWriter::new();
        p.u8(0x4F).u8(0x18); // dusk
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert_eq!(w.light_level, 0x18);
        assert_eq!(w.effective_light(), 0x18);
    }

    #[test]
    fn personal_light_combines_with_overall() {
        let mut w = World::new();
        w.player = Some(crate::types::Serial(0x42));
        w.light_level = 0x18;
        // Personal light for us is brighter (lower) → wins via min().
        let mut p = PacketWriter::new();
        p.u8(0x4E).u32(0x42).u8(0x08);
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert_eq!(w.personal_light, Some(0x08));
        assert_eq!(w.effective_light(), 0x08);

        // A personal light for someone else is ignored.
        let mut q = PacketWriter::new();
        q.u8(0x4E).u32(0x99).u8(0x00);
        assert!(apply_packet(&mut w, &q.into_vec()));
        assert_eq!(w.personal_light, Some(0x08));
    }

    #[test]
    fn weather_sets_and_resets() {
        let mut w = World::new();
        assert_eq!(w.weather.kind, 0xFF); // default = none
        // Rain, 40 particles.
        let mut p = PacketWriter::new();
        p.u8(0x65).u8(0).u8(40).u8(70);
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert_eq!((w.weather.kind, w.weather.intensity), (0, 40));

        // Reset to none.
        let mut q = PacketWriter::new();
        q.u8(0x65).u8(0xFE).u8(0).u8(0);
        assert!(apply_packet(&mut w, &q.into_vec()));
        assert_eq!(w.weather.kind, 0xFE);
    }

    #[test]
    fn season_sets_field() {
        let mut w = World::new();
        assert_eq!(w.season, 0); // default = Spring
        // 0xBC: Winter (3), playMusic = 1.
        let mut p = PacketWriter::new();
        p.u8(0xBC).u8(3).u8(1);
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert_eq!(w.season, 3);
    }

    #[test]
    fn war_mode_sets_field() {
        let mut w = World::new();
        assert!(!w.war); // default = peace
        // 0x72: war on, trailing fixed padding 0x00 0x32 0x00.
        let mut on = PacketWriter::new();
        on.u8(0x72).u8(1).u8(0x00).u8(0x32).u8(0x00);
        assert!(apply_packet(&mut w, &on.into_vec()));
        assert!(w.war);
        // 0x72: war off.
        let mut off = PacketWriter::new();
        off.u8(0x72).u8(0).u8(0x00).u8(0x32).u8(0x00);
        assert!(apply_packet(&mut w, &off.into_vec()));
        assert!(!w.war);
    }

    #[test]
    fn buff_add_and_remove() {
        let mut w = World::new();
        // 0xDF add: Bless (icon 0x0418), 3600s duration, for our serial.
        let mut p = PacketWriter::new();
        p.u8(0xDF).u16(0); // id, len placeholder
        p.u32(0x42).u16(0x0418).u16(1); // serial, icon, count=1 (add)
        p.u16(0).u16(0).u16(0x0418).u16(0).u32(0); // source, pad, icon, queue, pad
        p.u16(3600); // timer (seconds)
        p.zeros(3).u32(0).u32(0).u32(0); // pad + 3 clilocs (parser stops at timer)
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.buffs.len(), 1);
        assert_eq!(w.buffs[0].icon, 0x0418);
        assert_eq!(w.buffs[0].name, "Bless");
        assert_eq!(w.buffs[0].dur, 3600);

        // Re-add same icon → upsert (no duplicate), new duration.
        let mut p2 = PacketWriter::new();
        p2.u8(0xDF).u16(0).u32(0x42).u16(0x0418).u16(1);
        p2.u16(0).u16(0).u16(0x0418).u16(0).u32(0).u16(120);
        apply_packet(&mut w, &p2.into_vec());
        assert_eq!(w.buffs.len(), 1);
        assert_eq!(w.buffs[0].dur, 120);

        // 0xDF remove: count=0 drops the icon.
        let mut q = PacketWriter::new();
        q.u8(0xDF).u16(0).u32(0x42).u16(0x0418).u16(0); // count=0
        assert!(apply_packet(&mut w, &q.into_vec()));
        assert!(w.buffs.is_empty());
    }

    #[test]
    fn buff_unknown_icon_falls_back() {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0xDF).u16(0).u32(1).u16(0x0999).u16(1);
        p.u16(0).u16(0).u16(0x0999).u16(0).u32(0).u16(0);
        apply_packet(&mut w, &p.into_vec());
        assert_eq!(w.buffs[0].name, "#2457"); // 0x0999 = 2457, no table entry
        assert_eq!(w.buffs[0].dur, 0); // dur 0 = permanent / no timer
    }

    #[test]
    fn open_buy_window_parses_prices_and_vendor() {
        let mut w = World::new();
        // The for-sale container (0x4000_0001) is worn by vendor 0xAABB.
        let cont = w.item_mut(0x4000_0001);
        cont.container = Some(0xAABB);

        // 0x74: container, count=2, two (price, name) entries.
        let mut p = PacketWriter::new();
        p.u8(0x74).u16(0); // id, len placeholder
        p.u32(0x4000_0001).u8(2);
        p.u32(15).u8(5).bytes(b"bread");
        p.u32(3).u8(3).bytes(b"egg");
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));

        let sb = w.shop_buy.as_ref().expect("buy window stored");
        assert_eq!(sb.vendor, 0xAABB);
        assert_eq!(sb.container, 0x4000_0001);
        assert_eq!(sb.entries.len(), 2);
        assert_eq!(sb.entries[0], (15, "bread".to_string()));
        assert_eq!(sb.entries[1], (3, "egg".to_string()));
    }

    #[test]
    fn sell_list_parses_items() {
        let mut w = World::new();
        // 0x9E: vendor 0xAABB will buy one item from our pack.
        let mut p = PacketWriter::new();
        p.u8(0x9E).u16(0); // id, len placeholder
        p.u32(0xAABB).u16(1);
        p.u32(0x4000_0009) // serial
            .u16(0x0EED) // graphic (gold-ish)
            .u16(0) // hue
            .u16(7) // amount
            .u16(12) // price
            .u16(6)
            .bytes(b"dagger"); // nameLen + name
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));

        let ss = w.shop_sell.as_ref().expect("sell list stored");
        assert_eq!(ss.vendor, 0xAABB);
        assert_eq!(ss.items.len(), 1);
        let it = &ss.items[0];
        assert_eq!(it.serial, 0x4000_0009);
        assert_eq!((it.graphic, it.amount, it.price), (0x0EED, 7, 12));
        assert_eq!(it.name, "dagger");
    }

    #[test]
    fn display_gump_parses_layout_and_text() {
        let mut w = World::new();
        // 0xB0: a tiny dialog — one button + one text line ("Hi").
        let layout = "{ resizepic 0 0 5054 200 100 }{ button 20 70 247 248 1 0 1 }{ text 20 20 0 0 }";
        let mut p = PacketWriter::new();
        p.u8(0xB0).u16(0); // id, len placeholder
        p.u32(0xDEAD_BEEF) // serial
            .u32(0x0000_002A) // gumpId
            .u32(100) // x
            .u32(50); // y
        p.u16(layout.len() as u16).bytes(layout.as_bytes());
        p.u16(1); // textLinesCount
        p.u16(2); // charLen for "Hi"
        p.u16(b'H' as u16).u16(b'i' as u16); // UTF-16 BE (writer is BE)
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));

        assert_eq!(w.gumps.len(), 1);
        let g = &w.gumps[0];
        assert_eq!(g.serial, 0xDEAD_BEEF);
        assert_eq!(g.gump_id, 0x2A);
        assert_eq!((g.x, g.y), (100, 50));
        assert_eq!(g.layout, layout);
        assert_eq!(g.text, vec!["Hi".to_string()]);

        // A re-send with the same serial upserts in place (no duplicate).
        apply_packet(&mut w, &frame);
        assert_eq!(w.gumps.len(), 1);

        // close_gump drops it.
        w.close_gump(0xDEAD_BEEF);
        assert!(w.gumps.is_empty());
    }

    #[test]
    fn quest_arrow_show_and_hide() {
        let mut w = World::new();
        // 0xBA: show an arrow pointing at (1234, 5678), with a trailing serial (HS
        // form) the handler should read past and ignore.
        let mut p = PacketWriter::new();
        p.u8(0xBA).u8(1).u16(1234).u16(5678).u32(0xDEAD_BEEF);
        assert!(apply_packet(&mut w, &p.into_vec()));
        assert_eq!(w.quest_arrow, Some((1234, 5678)));

        // active = 0 hides it.
        let mut q = PacketWriter::new();
        q.u8(0xBA).u8(0).u16(0).u16(0).u32(0);
        assert!(apply_packet(&mut w, &q.into_vec()));
        assert_eq!(w.quest_arrow, None);
    }

    #[test]
    fn open_book_header_parsed() {
        let mut w = World::new();
        // 0x93: a 2-page writable book "My Diary" by "Anima".
        let mut p = PacketWriter::new();
        p.u8(0x93).u32(0x4000_0001).u8(1).u8(0).u16(2);
        p.fixed_ascii("My Diary", 60).fixed_ascii("Anima", 30);
        let frame = p.into_vec();
        assert_eq!(frame.len(), 99); // 0x93 is fixed 99 bytes
        assert!(apply_packet(&mut w, &frame));
        let b = w.book.as_ref().expect("book opened");
        assert_eq!(b.serial, 0x4000_0001);
        assert_eq!(b.title, "My Diary");
        assert_eq!(b.author, "Anima");
        assert!(b.writable);
        assert_eq!(b.page_count, 2);
        assert_eq!(b.pages.len(), 2);
        assert!(b.pages[0].is_empty());
    }

    #[test]
    fn book_data_fills_pages() {
        let mut w = World::new();
        // Open a 2-page book first (so book_data has somewhere to write).
        let mut h = PacketWriter::new();
        h.u8(0x93).u32(0x55).u8(0).u8(0).u16(2);
        h.fixed_ascii("Tome", 60).fixed_ascii("Sage", 30);
        apply_packet(&mut w, &h.into_vec());

        // 0x66: page 1 has two lines, page 2 has one line.
        let mut p = PacketWriter::new();
        p.u8(0x66).u16(0); // id + length placeholder
        p.u32(0x55).u16(2); // serial, page count
        p.u16(1).u16(2).bytes(b"line one\0").bytes(b"line two\0");
        p.u16(2).u16(1).bytes(b"page two\0");
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));

        let b = w.book.as_ref().expect("book present");
        assert_eq!(b.pages[0], vec!["line one".to_string(), "line two".to_string()]);
        assert_eq!(b.pages[1], vec!["page two".to_string()]);
    }

    #[test]
    fn typed_animation_stores_kind_action_and_mode() {
        let mut w = World::new();
        // 0xE2 NewMobileAnimation: serial 0xDEAD_BEEF, AnimationType::Emote (7),
        // action 1 ("salute"), mode/delay 42 — matches ServUO's `.salute` emote.
        let mut p = PacketWriter::new();
        p.u8(0xE2).u32(0xDEAD_BEEF).u16(7).u16(1).u8(42);
        let frame = p.into_vec();
        assert_eq!(frame.len(), 10); // ServUO NewMobileAnimation : base(0xE2, 10)
        assert!(apply_packet(&mut w, &frame));
        let (seq, serial, kind, action, mode) =
            *w.recent_typed_anims.last().expect("typed anim recorded");
        assert_eq!(seq, 1);
        assert_eq!(serial, 0xDEAD_BEEF);
        assert_eq!(kind, 7); // Emote
        assert_eq!(action, 1); // salute
        assert_eq!(mode, 42);
    }

    #[test]
    fn unknown_packet_ignored() {
        let mut w = World::new();
        // 0x9B is fixed-len but not handled → recognized=false
        assert!(!apply_packet(&mut w, &[0x9B, 0, 0]));
    }

    #[test]
    fn display_death_links_corpse_and_prunes_on_delete() {
        let mut w = World::new();
        // 0xAF: killed mobile 0xAAAA's corpse is item 0x4000_0001.
        let mut p = PacketWriter::new();
        p.u8(0xAF).u32(0xAAAA).u32(0x4000_0001).u32(0);
        let frame = p.into_vec();
        assert_eq!(frame.len(), 13); // ServUO DeathAnimation : base(0xAF, 13)
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.corpse_of.get(&0x4000_0001), Some(&0xAAAA));

        // The corpse item despawns (0x1D Delete) — the link is pruned with it.
        let mut d = PacketWriter::new();
        d.u8(0x1D).u32(0x4000_0001);
        assert!(apply_packet(&mut w, &d.into_vec()));
        assert!(!w.corpse_of.contains_key(&0x4000_0001));
    }

    #[test]
    fn change_combatant_sets_and_clears() {
        let mut w = World::new();
        let mut p = PacketWriter::new();
        p.u8(0xAA).u32(0xDEAD_BEEF);
        let frame = p.into_vec();
        assert_eq!(frame.len(), 5); // ServUO ChangeCombatant : base(0xAA, 5)
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.combatant, Some(0xDEAD_BEEF));

        // serial 0 = combat ended.
        let mut q = PacketWriter::new();
        q.u8(0xAA).u32(0);
        assert!(apply_packet(&mut w, &q.into_vec()));
        assert_eq!(w.combatant, None);
    }

    #[test]
    fn lift_reject_queues_event() {
        let mut w = World::new();
        // 0x27: reason 3 = BelongsToAnother.
        let mut p = PacketWriter::new();
        p.u8(0x27).u8(3);
        let frame = p.into_vec();
        assert_eq!(frame.len(), 2); // ServUO LiftRej : base(0x27, 2)
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.recent_lift_rejects.last(), Some(&(1, 3)));
        assert_eq!(w.lift_reject_seq, 1);
    }

    #[test]
    fn corpse_equip_parses_entries_and_terminator() {
        let mut w = World::new();
        // 0x89: corpse 0x4000_0002 wearing a shirt (layer 5 → wire 6, serial
        // 0x4000_0003) and a hat (layer 7 → wire 8, serial 0x4000_0004), terminated
        // by the layer==0 (Layer.Invalid) sentinel.
        let mut p = PacketWriter::new();
        p.u8(0x89).u16(0); // id, len placeholder
        p.u32(0x4000_0002);
        p.u8(6).u32(0x4000_0003);
        p.u8(8).u32(0x4000_0004);
        p.u8(0); // terminator
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));
        let entries = w.corpse_equip.get(&0x4000_0002).expect("corpse equip stored");
        assert_eq!(entries, &vec![(5, 0x4000_0003), (7, 0x4000_0004)]);
    }

    #[test]
    fn corpse_equip_truncated_frame_keeps_what_parsed() {
        let mut w = World::new();
        // 0x89: corpse 0x55, one full entry, then a dangling layer byte with no
        // serial behind it (truncated mid-stream) — must not panic, and the
        // complete entry before it is kept.
        let mut p = PacketWriter::new();
        p.u8(0x89).u16(0);
        p.u32(0x55);
        p.u8(3).u32(0x4000_0009); // one complete entry (real layer 2)
        p.u8(4); // dangling layer byte, no serial follows
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));
        let entries = w.corpse_equip.get(&0x55).expect("corpse equip stored");
        assert_eq!(entries, &vec![(2, 0x4000_0009)]);
    }

    #[test]
    fn unicode_prompt_sets_pending_state() {
        let mut w = World::new();
        // 0xC2 UnicodePrompt (server→client): serial 0x0102_0304, promptId
        // 0xDEAD_BEEF, plus the type/language/textLen fields ServUO always zeros.
        let mut p = PacketWriter::new();
        p.u8(0xC2).u16(0); // id, len placeholder
        p.u32(0x0102_0304).u32(0xDEAD_BEEF).u32(0).u32(0).u16(0);
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        assert_eq!(len, 21); // ServUO UnicodePrompt EnsureCapacity(21)
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));
        let p = w.prompt.expect("prompt pending");
        assert_eq!((p.sender_serial, p.prompt_id), (0x0102_0304, 0xDEAD_BEEF));
    }

    /// Patch the big-endian length word at `[1..3]` of a variable-framed test packet.
    fn patch_len(mut frame: Vec<u8>) -> Vec<u8> {
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        frame
    }

    #[test]
    fn secure_trade_display_opens_session() {
        let mut w = World::new();
        assert!(w.trades.is_empty());
        // 0x6F action 0 (Display): opponent 0xBEEF, my container 0x4000_0001,
        // their container 0x4000_0002, hasName=true, name "Bob" (NUL-padded to 30).
        let mut p = PacketWriter::new();
        p.u8(0x6F).u16(0); // id, len placeholder
        p.u8(0x00).u32(0xBEEF).u32(0x4000_0001).u32(0x4000_0002);
        p.u8(1).fixed_ascii("Bob", 30);
        let frame = patch_len(p.into_vec());
        assert_eq!(frame.len(), 47); // 3 header + 1 action + 3×4 serials + 1 bool + 30 name
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.trades.len(), 1);
        let t = &w.trades[0];
        assert_eq!(t.opponent_serial, 0xBEEF);
        assert_eq!(t.my_container, 0x4000_0001);
        assert_eq!(t.their_container, 0x4000_0002);
        assert_eq!(t.opponent_name, "Bob");
        assert!(!t.my_accept && !t.their_accept);
    }

    #[test]
    fn secure_trade_display_same_opponent_replaces_not_duplicates() {
        let mut w = World::new();
        w.open_trade(TradeState { opponent_serial: 0xBEEF, my_container: 0x4000_0001, ..Default::default() });
        // A second Display for the SAME opponent (ServUO's FindTradeContainer
        // dedupe: only one session per mobile pair) must replace, not append.
        let mut p = PacketWriter::new();
        p.u8(0x6F).u16(0).u8(0x00).u32(0xBEEF).u32(0x4000_0003).u32(0x4000_0004);
        p.u8(1).fixed_ascii("Bob", 30);
        assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
        assert_eq!(w.trades.len(), 1);
        assert_eq!(w.trades[0].my_container, 0x4000_0003);
    }

    #[test]
    fn secure_trade_close_clears_only_matching_session() {
        let mut w = World::new();
        w.open_trade(TradeState { opponent_serial: 1, my_container: 0x4000_0001, ..Default::default() });
        w.open_trade(TradeState { opponent_serial: 2, my_container: 0x4000_0002, ..Default::default() });
        // 0x6F action 1 (Close): container 0x4000_0001 — only that session drops.
        let mut p = PacketWriter::new();
        p.u8(0x6F).u16(0).u8(0x01).u32(0x4000_0001);
        assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
        assert_eq!(w.trades.len(), 1);
        assert_eq!(w.trades[0].my_container, 0x4000_0002);
    }

    #[test]
    fn secure_trade_close_purges_leftover_container_items() {
        let mut w = World::new();
        w.open_trade(TradeState {
            opponent_serial: 1,
            my_container: 0x4000_0001,
            their_container: 0x4000_0002,
            ..Default::default()
        });
        // Items sitting in either trade container at close time (ServUO sends no
        // removal packet for the opponent's side — see `World::close_trade`'s doc).
        w.item_mut(0x5000_0001).container = Some(0x4000_0001); // mine
        w.item_mut(0x5000_0002).container = Some(0x4000_0002); // theirs
        // An unrelated item elsewhere must survive the purge.
        w.item_mut(0x5000_0003).container = Some(0x9999_0000);
        let mut p = PacketWriter::new();
        p.u8(0x6F).u16(0).u8(0x01).u32(0x4000_0001);
        assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
        assert!(!w.items.contains_key(&0x5000_0001));
        assert!(!w.items.contains_key(&0x5000_0002));
        assert!(w.items.contains_key(&0x5000_0003));
    }

    #[test]
    fn secure_trade_interleaved_two_sessions() {
        let mut w = World::new();
        // Open a session with B, then a second with C — two strangers can each
        // open a trade with us concurrently (no consent required).
        let mut open_b = PacketWriter::new();
        open_b.u8(0x6F).u16(0).u8(0x00).u32(0xB0B).u32(0x4000_0001).u32(0x4000_0002);
        open_b.u8(1).fixed_ascii("Bob", 30);
        assert!(apply_packet(&mut w, &patch_len(open_b.into_vec())));

        let mut open_c = PacketWriter::new();
        open_c.u8(0x6F).u16(0).u8(0x00).u32(0xC0C).u32(0x4000_0003).u32(0x4000_0004);
        open_c.u8(1).fixed_ascii("Carol", 30);
        assert!(apply_packet(&mut w, &patch_len(open_c.into_vec())));
        assert_eq!(w.trades.len(), 2);

        // C accepts and offers gold — must land on C's session only.
        let mut c_accept = PacketWriter::new();
        c_accept.u8(0x6F).u16(0).u8(0x02).u32(0x4000_0003).u32(0).u32(1);
        assert!(apply_packet(&mut w, &patch_len(c_accept.into_vec())));
        let mut c_gold = PacketWriter::new();
        c_gold.u8(0x6F).u16(0).u8(0x03).u32(0x4000_0003).u32(777).u32(3);
        assert!(apply_packet(&mut w, &patch_len(c_gold.into_vec())));

        // Close B (container 0x4000_0001) — C must survive untouched.
        let mut close_b = PacketWriter::new();
        close_b.u8(0x6F).u16(0).u8(0x01).u32(0x4000_0001);
        assert!(apply_packet(&mut w, &patch_len(close_b.into_vec())));

        assert_eq!(w.trades.len(), 1);
        let c = &w.trades[0];
        assert_eq!(c.opponent_serial, 0xC0C);
        assert_eq!(c.my_container, 0x4000_0003);
        assert!(c.their_accept);
        assert_eq!((c.their_offer_gold, c.their_offer_platinum), (777, 3));
    }

    #[test]
    fn secure_trade_update_accept_flags() {
        let mut w = World::new();
        w.open_trade(TradeState {
            my_container: 0x4000_0001,
            their_container: 0x4000_0002,
            ..Default::default()
        });
        // 0x6F action 2 (Update): I accepted (1), they haven't (0).
        let mut p = PacketWriter::new();
        p.u8(0x6F).u16(0).u8(0x02).u32(0x4000_0001).u32(1).u32(0);
        assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
        let t = &w.trades[0];
        assert!(t.my_accept);
        assert!(!t.their_accept);

        // Both accept → both flags flip.
        let mut q = PacketWriter::new();
        q.u8(0x6F).u16(0).u8(0x02).u32(0x4000_0001).u32(1).u32(1);
        assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
        let t = &w.trades[0];
        assert!(t.my_accept && t.their_accept);
    }

    #[test]
    fn secure_trade_update_gold_and_ledger() {
        let mut w = World::new();
        w.open_trade(TradeState {
            my_container: 0x4000_0001,
            their_container: 0x4000_0002,
            ..Default::default()
        });
        // 0x6F action 3 (UpdateGold): the OPPONENT offered 500 gold / 2 plat.
        let mut p = PacketWriter::new();
        p.u8(0x6F).u16(0).u8(0x03).u32(0x4000_0001).u32(500).u32(2);
        assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
        let t = &w.trades[0];
        assert_eq!((t.their_offer_gold, t.their_offer_platinum), (500, 2));
        assert_eq!((t.balance_gold, t.balance_platinum), (0, 0)); // untouched

        // 0x6F action 4 (UpdateLedger): OUR account balance is 1000 gold / 5 plat
        // (an input cap, not an offer — see `TradeState`'s doc).
        let mut q = PacketWriter::new();
        q.u8(0x6F).u16(0).u8(0x04).u32(0x4000_0001).u32(1000).u32(5);
        assert!(apply_packet(&mut w, &patch_len(q.into_vec())));
        let t = &w.trades[0];
        assert_eq!((t.balance_gold, t.balance_platinum), (1000, 5));
        assert_eq!((t.their_offer_gold, t.their_offer_platinum), (500, 2)); // untouched
        assert_eq!((t.my_offer_gold, t.my_offer_platinum), (0, 0)); // untouched — we never sent one
    }

    #[test]
    fn secure_trade_unrecognized_action_is_a_noop() {
        let mut w = World::new();
        w.open_trade(TradeState { my_container: 0x4000_0001, ..Default::default() });
        let mut p = PacketWriter::new();
        p.u8(0x6F).u16(0).u8(0xFF); // no such action — must not panic or touch state
        assert!(apply_packet(&mut w, &patch_len(p.into_vec())));
        assert_eq!(w.trades.len(), 1);
    }

    #[test]
    fn general_info_map_change_sets_facet() {
        let mut w = World::new();
        assert_eq!(w.map_index, 0);
        // 0xBF/0x08 MapChange: switch to facet 1 (Trammel).
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0).u16(0x0008).u8(1);
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert_eq!(frame.len(), 6); // ServUO MapChange EnsureCapacity(6)
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.map_index, 1);
    }

    #[test]
    fn map_change_purges_old_facet_but_keeps_player_and_holdings() {
        let mut w = World::new();
        let player = 0x1000_0001;
        w.player = Some(crate::types::Serial(player));
        w.mobile_mut(player).name = "Anima".into();

        // Worn equip: container == the player's own serial directly.
        let backpack = 0x4000_0001;
        w.item_mut(backpack).container = Some(player);
        // Backpack'd item: container == the backpack's serial (nested one level).
        let potion = 0x4000_0002;
        w.item_mut(potion).container = Some(backpack);

        // A stranger mobile and a loose ground item from the OLD facet —
        // neither is the player nor rooted at them, so both must be purged.
        let stranger = 0x1000_0002;
        w.mobile_mut(stranger).name = "Rat".into();
        let ground_item = 0x4000_0003;
        w.item_mut(ground_item);

        // A corpse (and its worn-item layout) from the old facet — purged along
        // with the links that index it, so nothing dangles afterward.
        let corpse = 0x4000_0004;
        w.item_mut(corpse);
        w.set_corpse_of(corpse, stranger);
        w.set_corpse_equip(corpse, vec![(1, 0x4000_0005)]);

        // 0xBF/0x08 MapChange: switch facet 0 → 1 (Trammel).
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0).u16(0x0008).u8(1);
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.map_index, 1);

        // Survivors: only the player mobile, and only what's rooted at them.
        assert_eq!(w.mobiles.keys().copied().collect::<Vec<_>>(), vec![player]);
        let mut kept: Vec<u32> = w.items.keys().copied().collect();
        kept.sort();
        assert_eq!(kept, vec![backpack, potion]);

        // Purged: the stranger, the ground item, and the now-dangling corpse links.
        assert!(!w.items.contains_key(&ground_item));
        assert!(!w.items.contains_key(&corpse));
        assert!(w.corpse_of.is_empty());
        assert!(w.corpse_equip.is_empty());
    }

    #[test]
    fn map_change_same_facet_is_a_noop() {
        let mut w = World::new();
        let player = 0x1000_0001;
        w.player = Some(crate::types::Serial(player));
        w.mobile_mut(player);
        let stranger = 0x1000_0002;
        w.mobile_mut(stranger);
        let ground_item = 0x4000_0003;
        w.item_mut(ground_item);

        // 0xBF/0x08 MapChange re-affirming the CURRENT facet (0) — must not
        // purge anything (only an actual facet change does).
        let mut p = PacketWriter::new();
        p.u8(0xBF).u16(0).u16(0x0008).u8(0);
        let mut frame = p.into_vec();
        let len = frame.len() as u16;
        frame[1] = (len >> 8) as u8;
        frame[2] = (len & 0xFF) as u8;
        assert!(apply_packet(&mut w, &frame));
        assert_eq!(w.map_index, 0);
        assert!(w.mobiles.contains_key(&stranger));
        assert!(w.items.contains_key(&ground_item));
    }
}

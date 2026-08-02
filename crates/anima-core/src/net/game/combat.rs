//! Combat, death, and the skills that feed them.
//!
//! Damage numbers (0x0B), swings (0x2F), whom the server thinks we are fighting
//! (0xAA), our own death (0x2C) and other people's (0xAF), plus the skill list
//! (0x3A) and server-driven pathfinding (0x38).

use super::*;

/// 0x3A SkillUpdate — full skill list or a single skill change (variable).
/// `[id][len:u16][type:u8]` then entries `[skillID:u16][value][base][lock][cap?]`.
/// Ported from `anima/anima/perception/handlers.py::handle_skill_update`.
pub(super) fn skills(world: &mut World, frame: &[u8]) -> PResult<()> {
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
        let cap = if has_cap && r.remaining() >= 2 {
            r.u16()?
        } else {
            1000
        };

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
        *s = Skill {
            id,
            value,
            base,
            cap,
            lock,
        };

        if is_single {
            break;
        }
    }
    Ok(())
}

/// 0x0B Damage — `[id][serial:u32][damage:u16]` (7 bytes). `serial` just took
/// `damage` HP; the renderer floats a number over it. (ClassicUO `Damage` /
/// `case 0x0B`.)
///
/// NOTE: floating damage numbers are an **AOS+** feature. ServUO gates the send on
/// `Mobile.VisibleDamageType`, which `CurrentExpansion` sets to `Related` only when
/// `Core.AOS`, else `None` (which sends *nothing*). So on a **pre-AOS (e.g. T2A)**
/// shard this packet never arrives — verified by a uo_proxy capture of live combat:
/// HP drains via 0xA1 vitals, but no 0x0B/0xBF-0x22 damage packet is ever on the
/// wire. This handler is correct and lights up unchanged against an AOS+ shard;
/// don't chase a "missing damage floater" as a client bug until you've confirmed
/// the shard's expansion. (See DESIGN.md.)
pub(super) fn damage(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let amount = r.u16()?;
    // ClassicUO's Damage() drops a 0-damage event (it only floats a number when
    // damage > 0); mirror it so the brain's damage log stays meaningful.
    if amount > 0 {
        world.push_damage(serial, amount);
    }
    Ok(())
}

/// 0xAF DisplayDeath — `[id][killedSerial:u32][corpseSerial:u32][unused:u32=0]`
/// (13 bytes, ServUO `DeathAnimation : base(0xAF, 13)`). Sent on every mobile
/// death; links the new corpse item to the mobile that died. AI-facing only (no
/// death animation is modeled — no rendering in core); the renderer needs nothing
/// from this (the corpse item already carries its own body/hue/direction).
pub(super) fn display_death(world: &mut World, frame: &[u8]) -> PResult<()> {
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
pub(super) fn change_combatant(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    world.combatant = if serial == 0 { None } else { Some(serial) };
    Ok(())
}

/// 0x15 FollowR — `[id][follower:u32][followed:u32]` (9 bytes, ServUO
/// `FollowMessage`; ClassicUO `FollowR` reads and discards both serials). We
/// keep the followed serial as [`World::follow_target`] — the mobile the
/// server says to follow — clearing it when that serial is 0.
pub(super) fn follow_r(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let _follower = r.u32()?;
    let followed = r.u32()?;
    world.follow_target = if followed == 0 { None } else { Some(followed) };
    Ok(())
}

/// 0x2C DeathStatus — `[id][action:u8]` (2 bytes). ServUO writes 0 at the
/// beginning and 2 at the end of one player-death sequence. ClassicUO applies
/// its screen/audio/weather/peace-mode effects for both (`action != 1`) and
/// derives actual dead/alive state separately from the player's body graphic.
pub(super) fn death_status(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.on_death_screen(r.u8()?);
    Ok(())
}

/// 0x38 Pathfinding — `[id][x:u16][y:u16][z:u16]` (7 bytes). ClassicUO passes
/// this directly to `Player.Pathfinder.WalkTo(x, y, z, 0)`. Core records the
/// request with a monotonic sequence; native and web route executors consume it
/// using their existing non-blocking WalkTo machinery.
pub(super) fn pathfinding(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.u16()?;
    world.set_server_pathfind(x, y, z);
    Ok(())
}

/// 0x2F Swing — `[id][flag:u8][attacker:u32][defender:u32]` (10 bytes, ServUO
/// `Swing : base(0x2F, 10)`). ServUO sends this only to the ATTACKING player's
/// own client (`attacker.Send(new Swing(...))` — an NPC attacker has no
/// `NetState`, so this never arrives unless WE are the one swinging), meaning
/// `attacker` is normally our own serial; stored generically anyway since
/// nothing about the wire format assumes that. `flag` is always `0` at every
/// real ServUO call site (`BaseWeapon`/`BaseRanged`) — vestigial, so we read
/// past it and don't store it. Purely cosmetic feedback (the renderer briefly
/// faces the attacker toward the defender) — recorded as a seq-numbered event
/// like the other renderer-facing rings.
pub(super) fn swing(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    r.skip(1)?; // flag — always 0 at every real ServUO call site
    let attacker = r.u32()?;
    let defender = r.u32()?;
    world.push_swing(attacker, defender);
    Ok(())
}

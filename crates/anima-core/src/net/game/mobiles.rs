//! Mobiles: who is where, what they look like, and their vitals.
//!
//! Covers the incoming/moving/update family (0x77/0x78/0xD3/0x20/0x1D... the ids
//! that create or move a `Mobile`), worn equipment (0x2E/0x89's living half),
//! names (0x98), and every flavour of status the server sends about a body —
//! 0x11 CharacterStatus, the 0xA1-3 vitals, and 0x2D's full attribute block.

use super::*;
use crate::world::AosStatus;

/// Status-flags bits on the mobile-update packets (0x20/0x77/0x78/0xD2/0xD3).
/// ServUO `Mobile.cs GetPacketFlags`: 0x01 Frozen/Paralyzed, 0x04 Flying, 0x08
/// YellowHealth/Blessed, 0x40 WarMode, 0x80 Hidden (ClassicUO
/// `EntityFlags.cs`). We only decode Paralyzed/WarMode/Hidden — for OTHER
/// mobiles this byte is the ONLY wire source for war-mode/paralysis. NOTE:
/// poison is NOT in this byte on a Stygian-Abyss+ client (which we report
/// as); it arrives in the separate 0x17 health-bar packet — see
/// [`health_bar_status`].
pub(super) const FLAG_HIDDEN: u8 = 0x80;

pub(super) const FLAG_WARMODE: u8 = 0x40;

pub(super) const FLAG_PARALYZED: u8 = 0x01;

/// 0x04. ClassicUO reuses one enum slot for two meanings and picks by version
/// (`Mobile.cs:137`): `IsFlying => Version >= CV_7000 && (Flags & Poisoned)`.
/// We report Stygian Abyss, so for us this bit is Flying and never poison —
/// poison arrives on 0x17 (see [`health_bar_status`]). ServUO agrees:
/// `Mobile.cs:8782-8785`, `if (m_Flying) flags |= 0x04;` in the 7.0 branch.
pub(super) const FLAG_FLYING: u8 = 0x04;

/// The status-flags byte, applied in one place because it has SEVEN call sites
/// across the mobile-update family. A bit wired into six of them is a bug that
/// only shows on the seventh packet, which is exactly how `flying` stayed
/// undecoded while its own constant was already documented above.
fn apply_status_flags(m: &mut crate::world::Mobile, flags: u8) {
    m.hidden = flags & FLAG_HIDDEN != 0;
    m.war_mode = flags & FLAG_WARMODE != 0;
    m.paralyzed = flags & FLAG_PARALYZED != 0;
    m.flying = flags & FLAG_FLYING != 0;
}

/// Split a mobile-update direction byte into facing and the running bit.
///
/// The 0x80 bit is `Direction.Running`, which ClassicUO peels off as `isrun`
/// (`PacketHandlers.cs:6313-6334`) and feeds to the walk-vs-run animation group
/// and the step duration. We masked it away at all five parse sites, which left
/// the renderer inferring "is it running?" from how fast updates happened to
/// arrive — a guess that misreads under poll jitter, a latency spike, or a
/// mobile stopping mid-run. ServUO sends the bit: its `Direction` enum has
/// `Running = 0x80` (`Mobile.cs:392`) and the movement packets write
/// `(byte)m.Direction` whole.
fn split_direction(b: u8) -> (u8, bool) {
    (b & 0x07, b & 0x80 != 0)
}

/// Wear layer of the backpack (ClassicUO `Layer.Backpack`). Excluded from the
/// stale-equipment sweep in [`apply_worn_items`] — see that function.
const LAYER_BACKPACK: u8 = 0x15;

/// Highest layer a mobile actually *wears* (`Layer.Mount`). Above it are the
/// four container pseudo-layers — `ShopBuyRestock`/`ShopBuy`/`ShopSell` (0x1A-
/// 0x1C) and `Bank` (0x1D) — which are never in a mobile-incoming equipment
/// list, so the sweep must leave them alone or opening your bank and then
/// walking past an NPC would empty it.
const LAYER_MAX_WORN: u8 = 0x19;

/// Apply the worn-item list that tails 0x78 MobileIncoming / 0xD3 UpdateObject:
/// fixed records of `serial(u32) graphic(u16) layer(u8) hue(u16)`, terminated by
/// a zero serial or the end of the frame. (CV_70331 `NewMobileIncoming` format —
/// hue always present, no `0x8000` graphic flag.)
///
/// The list is **authoritative**: it is the mobile's complete visible equipment,
/// so anything we still have worn by `owner` that the server did not just name
/// has been taken off, and must go. Without that sweep an unequipped item keeps
/// `container == Some(owner)` forever and every consumer that asks "what is this
/// mobile wearing" — paperdoll, worn-equipment sprites, a brain's inventory
/// view — keeps seeing it. The classic symptom is a mount that stays drawn under
/// a rider who dismounted out of view.
///
/// Two deliberate departures from ClassicUO's `UpdateObject`, which does the
/// same sweep by removing every non-backpack child and then recreating the
/// listed ones from scratch:
/// - **We remove only what the list omits**, instead of removing all and
///   re-adding. Recreating an item that never left drops its name and its OPL
///   (`World::remove` clears both), so ClassicUO's version silently invalidates
///   the tooltip of every piece of gear each time its wearer walks back into
///   view. Same end state, no churn.
/// - **We skip the container pseudo-layers** (see [`LAYER_MAX_WORN`]) rather
///   than guard on ClassicUO's "is this container's gump open" flag, which is
///   renderer state the core does not have (D3). The layers it protects are
///   exactly the ones a mobile cannot wear, so the narrower rule needs no
///   knowledge of open windows.
fn apply_worn_items(world: &mut World, owner: u32, r: &mut PacketReader) -> PResult<()> {
    let mut worn = Vec::new();
    // Did the list end the way a complete list ends — the zero terminator, or
    // exactly running out of frame? A record cut off mid-way did not, and the
    // difference matters now that omission *deletes*: a truncated frame would
    // otherwise read as "the server says all the rest came off" and strip gear
    // the mobile is still wearing. Parsing stays lenient (a short tail is not a
    // framing error, as before); only the sweep is withheld.
    let mut complete = true;
    while r.remaining() >= 4 {
        let item_serial = r.u32()?;
        if item_serial == 0 {
            break;
        }
        if r.remaining() < 5 {
            complete = false;
            break;
        }
        let graphic = r.u16()?;
        let layer = r.u8()?;
        let hue = r.u16()?;
        worn.push((item_serial, graphic, layer, hue));
    }

    if complete {
        let stale: Vec<u32> = world
            .items
            .values()
            .filter(|it| {
                it.container == Some(owner)
                    && it.layer != LAYER_BACKPACK
                    && it.layer <= LAYER_MAX_WORN
                    && !worn.iter().any(|(serial, ..)| *serial == it.serial)
            })
            .map(|it| it.serial)
            .collect();
        for serial in stale {
            world.remove(serial);
        }
    }

    for (item_serial, graphic, layer, hue) in worn {
        let it = world.item_mut(item_serial);
        it.graphic = graphic;
        it.layer = layer;
        it.hue = hue;
        it.container = Some(owner);
    }
    Ok(())
}

/// 0x20 MobileUpdate — position/appearance reset. This is always about OUR
/// OWN mobile (ServUO sends it only to the mobile itself), so its flags byte
/// is the self-hidden feedback path: e.g. right after the Hiding skill
/// succeeds, or a GM `[set Hidden true`.
pub(super) fn mobile_update(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let body = r.u16()?;
    r.skip(1)?; // graphic_inc
    let hue = r.u16()?;
    let flags = r.u8()?;
    let x = r.u16()?;
    let y = r.u16()?;
    r.skip(2)?; // server_id
    let (direction, running) = split_direction(r.u8()?);
    let z = r.i8()?;

    let is_self = world.is_player(serial);
    let old_body = world.mobiles.get(&serial).map(|m| m.body).unwrap_or(body);
    {
        let m = world.mobile_mut(serial);
        m.body = body;
        m.hue = hue;
        m.pos.x = x;
        m.pos.y = y;
        m.pos.z = z;
        m.direction = direction;
        m.running = running;
        apply_status_flags(m, flags);
    }
    if is_self {
        world.on_player_body_changed(old_body, body);
    }
    Ok(())
}

/// 0x16/0x17 NewHealthbarUpdate/MobileHealthbarStatus (ClassicUO
/// `NewHealthbarUpdate`; ServUO `HealthbarPoison`/`HealthbarYellow`):
/// `[id][len:u16][serial:u32][count:u16]` then `count × [type:u16][flag:u8]`.
/// Both ids share this layout (0x16 is the pre-Stygian-Abyss form); we route
/// both here. type 1 = poison bar: `flag > 0` means poisoned, and the poison
/// level is `flag - 1` (ServUO writes `level + 1`, so a cure sends `flag == 0`
/// → level -1). type 2 = yellow/blessed bar: `flag != 0` means yellow. A
/// packet only ever reports the type(s) that changed, so we only touch the
/// field(s) for types actually present — a poison-only packet must never
/// clobber `yellow_health`, and vice versa. **EXISTING-ONLY:** like
/// ClassicUO's handler (which returns when `Mobiles.Get(serial)` is null),
/// this packet carries no position, so it must never spawn a phantom mobile —
/// no-op if `serial` isn't already known (works for self too: the player's
/// own Mobile is already present once in-game).
pub(super) fn health_bar_status(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // skip id + u16 length
    let serial = r.u32()?;
    let count = r.u16()?;
    let mut poisoned = None;
    let mut yellow = None;
    for _ in 0..count {
        let kind = r.u16()?;
        let flag = r.u8()?;
        match kind {
            1 => poisoned = Some(flag),
            2 => yellow = Some(flag != 0),
            _ => {}
        }
    }
    let Some(m) = world.mobiles.get_mut(&serial) else {
        return Ok(());
    };
    if let Some(flag) = poisoned {
        m.poisoned = flag > 0;
        // ServUO sends poison level+1 (0 = cured); compute in a wider type so a
        // malformed flag >= 0x80 can't underflow-panic (apply_packet swallows
        // parse errors, not panics). Normal flags are 0..=5.
        m.poison_level = (flag as i16 - 1) as i8;
    }
    if let Some(y) = yellow {
        m.yellow_health = y;
    }
    Ok(())
}

/// 0xDE UpdateMobileStatus — `[id][len:u16][serial:u32][status:u8]` and, only
/// when `status == 1`, a trailing `[attacker:u32]` (ClassicUO
/// `UpdateMobileStatus`). ClassicUO's handler applies no state — it just reads
/// the fields to stay in sync with the stream — so we match that: parse and
/// discard, no `World` mutation, and no phantom mobile is created.
pub(super) fn update_mobile_status(_world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let _serial = r.u32()?;
    let status = r.u8()?;
    if status == 1 {
        let _attacker = r.u32()?;
    }
    Ok(())
}

/// 0xC4 Semivisible — `[id][serial:u32][flag:u8]` (6 bytes, ClassicUO
/// `Semivisible`). ClassicUO's handler is an empty no-op; we parse it anyway
/// so it's recognized (not treated as an unknown packet) rather than
/// mutating any state.
pub(super) fn semivisible(_world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let _serial = r.u32()?;
    let _flag = r.u8()?;
    Ok(())
}

/// 0x77 MobileMoving — a mobile moves.
pub(super) fn mobile_moving(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let body = r.u16()?;
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.i8()?;
    let (direction, running) = split_direction(r.u8()?);
    let hue = r.u16()?;
    let flags = r.u8()?;
    let notoriety = r.u8()?;

    // The Walker owns the player's own position/facing (prediction + ConfirmWalk).
    // A server MobileMoving *about us* must never overwrite it — that resets our
    // facing to a stale value and fights the walker, causing the turn/stall
    // direction oscillation. Mirror anima: ignore self's pos/dir here — but
    // notoriety/hidden are visual attrs (crime flag, invisibility), not movement, so
    // still refresh them so e.g. going criminal recolours your own name.
    if world.is_player(serial) {
        let m = world.mobile_mut(serial);
        m.notoriety = notoriety;
        apply_status_flags(m, flags);
        return Ok(());
    }

    let m = world.mobile_mut(serial);
    m.body = body;
    m.pos.x = x;
    m.pos.y = y;
    m.pos.z = z;
    m.direction = direction;
    m.running = running;
    m.hue = hue;
    m.notoriety = notoriety;
    apply_status_flags(m, flags);
    Ok(())
}

/// 0xD2 UpdateCharacter — a legacy full mobile update. Fixed 25 bytes on the
/// wire (`lengths.rs`), but ClassicUO's `UpdateCharacter` — the SAME handler
/// function it also registers for 0x77 — only reads the leading
/// `[serial:u32][graphic:u16][x:u16][y:u16][z:i8][direction:u8][hue:u16]
/// [flags:u8][notoriety:u8]` (17 bytes incl. id) and leaves the remaining 8
/// bytes unread; we mirror that exactly, reading only what's needed.
/// **EXISTING-ONLY:** like that handler (which returns when `Mobiles.Get(serial)`
/// is null), no-op if `serial` isn't already known.
pub(super) fn update_character(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let graphic = r.u16()?;
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.i8()?;
    let (direction, running) = split_direction(r.u8()?);
    let hue = r.u16()?;
    let flags = r.u8()?;
    let notoriety = r.u8()?;

    let is_self = world.is_player(serial);
    let Some(m) = world.mobiles.get_mut(&serial) else {
        return Ok(());
    };

    // For self, only visual/flags refresh — never position (mirror
    // mobile_moving's self-guard: the Walker owns our own pos/facing).
    if is_self {
        m.body = graphic;
        m.hue = hue;
        m.notoriety = notoriety;
        apply_status_flags(m, flags);
        return Ok(());
    }

    m.body = graphic;
    m.pos.x = x;
    m.pos.y = y;
    m.pos.z = z;
    m.direction = direction;
    m.running = running;
    m.hue = hue;
    m.notoriety = notoriety;
    apply_status_flags(m, flags);
    Ok(())
}

/// 0x2E EquipUpdate — a single item equipped on a mobile (worn after the initial
/// 0x78, e.g. mounting puts the mount item on layer 0x19, or wearing/removing
/// gear). Without this, equip changes never reach the World — so a mount you put
/// on never appears (the client can't draw it) and `player_mounted()` stays false.
/// Format: serial(u32) graphic(u16) 0(u8) layer(u8) parent(u32) hue(u16).
pub(super) fn equip_update(world: &mut World, frame: &[u8]) -> PResult<()> {
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
pub(super) fn mobile_incoming(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable: skip id + length
    let serial = r.u32()?;
    let body = r.u16()?;
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.i8()?;
    let (direction, running) = split_direction(r.u8()?);
    let hue = r.u16()?;
    let flags = r.u8()?;
    let notoriety = r.u8()?;

    // For self, the Walker owns position/facing — only refresh body/hue, never
    // pos/dir (see mobile_moving). Still parse the worn-item list below.
    let is_self = world.is_player(serial);
    let old_body = world.mobiles.get(&serial).map(|m| m.body).unwrap_or(body);
    {
        let m = world.mobile_mut(serial);
        m.body = body;
        m.hue = hue;
        // Hidden is a visual flag like body/hue, not movement state — refresh it
        // for self too (the self-hidden feedback path also flows through 0x78,
        // e.g. re-entering view after a facet change while hidden). Poisoned is
        // the same story: re-derive it for self too.
        // War-mode/paralyzed/flying are visual/status attrs like hidden —
        // refresh for self too.
        apply_status_flags(m, flags);
        // Notoriety is a visual attribute (like body/hue/hidden), not movement state,
        // so capture it for self too — ServUO sends the player their own noto here (and
        // on crime deltas), which is what colours your own single-click name. pos/dir
        // stay walker-owned for self (see mobile_moving).
        m.notoriety = notoriety;
        if !is_self {
            m.pos.x = x;
            m.pos.y = y;
            m.pos.z = z;
            m.direction = direction;
            m.running = running;
        }
    }
    if is_self {
        world.on_player_body_changed(old_body, body);
    }

    apply_worn_items(world, serial, &mut r)
}

/// 0xD3 UpdateObject — the legacy sibling of 0x78 MobileIncoming. ClassicUO
/// registers a SEPARATE `UpdateObject` handler for 0xD3, but it's the same
/// shape as `UpdateGameObject`'s caller for 0x78 plus 6 extra bytes: `if
/// (p[0] != 0x78) p.Skip(6);` (`UpdateObject` in PacketHandlers.cs). Otherwise
/// this mirrors `mobile_incoming` exactly, including the create-if-missing
/// (`mobile_mut`, NOT existing-only like 0xD2) and worn-item list.
pub(super) fn update_object(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable: skip id + length
    let serial = r.u32()?;
    let body = r.u16()?;
    let x = r.u16()?;
    let y = r.u16()?;
    let z = r.i8()?;
    let (direction, running) = split_direction(r.u8()?);
    let hue = r.u16()?;
    let flags = r.u8()?;
    let notoriety = r.u8()?;
    r.skip(6)?; // 0xD3-only padding — absent on 0x78 MobileIncoming

    // For self, the Walker owns position/facing — only refresh body/hue, never
    // pos/dir (see mobile_moving). Still parse the worn-item list below.
    let is_self = world.is_player(serial);
    let old_body = world.mobiles.get(&serial).map(|m| m.body).unwrap_or(body);
    {
        let m = world.mobile_mut(serial);
        m.body = body;
        m.hue = hue;
        // Hidden/war-mode/paralyzed/notoriety are visual attributes, not movement
        // state — refresh them for self too (same reasoning as mobile_incoming).
        apply_status_flags(m, flags);
        m.notoriety = notoriety;
        if !is_self {
            m.pos.x = x;
            m.pos.y = y;
            m.pos.z = z;
            m.direction = direction;
            m.running = running;
        }
    }
    if is_self {
        world.on_player_body_changed(old_body, body);
    }

    apply_worn_items(world, serial, &mut r)
}

/// 0x97 MovePlayer — `[id][direction:u8]` (2 bytes). ClassicUO forces
/// `Player.Walk(dir & 0x07, running = dir & 0x80)` directly; core has no
/// Walker here, so it records the request for the movement driver to execute
/// (mirrors 0x38 pathfinding above).
pub(super) fn move_player(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let d = r.u8()?;
    world.set_forced_walk(d & 0x07, d & 0x80 != 0);
    Ok(())
}

/// 0x1D Delete — entity removed from the world.
pub(super) fn delete(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    world.remove(serial);
    Ok(())
}

/// 0x11 CharacterStatus — name + full stat block for self, name/hits for
/// others. The self-only stat block is itself version-gated on `flag`
/// (ClassicUO's `type` byte): `>= 1` unlocks the basic block up through
/// `weight`, then `>= 5`/`>= 3`/`>= 4` each unlock a further tail (ML,
/// Renaissance, AOS) — see the ML/Renaissance/AOS comments below.
pub(super) fn char_status(world: &mut World, frame: &[u8]) -> PResult<()> {
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

        // Version-gated tail (ClassicUO `CharacterStatus`; `flag` here is its
        // `type` byte). ClassicUO reads these three blocks unconditionally in
        // this order — ML, then Renaissance, then AOS — each gated on the
        // same `flag`, regardless of which combination of thresholds it
        // clears. Every field read uses `?`, so a truncated packet stops
        // partway through rather than misreading a later block's bytes as an
        // earlier one's.
        if flag >= 5 {
            // ML: weight cap + race. `weight_max` already existed on
            // `PlayerStats` but was never written until now.
            stats.weight_max = r.u16()?;
            // ClassicUO normalizes an absent race (0) to Human (1); RaceType has
            // no valid 0 value (1=Human, 2=Elf, 3=Gargoyle).
            let race = r.u8()?;
            stats.race = if race == 0 { 1 } else { race };
        } else {
            // Non-ML shards (flag < 5) omit the weight cap on the wire; ClassicUO
            // derives it from strength (7*(str>>1)+40 for CV_500A+) so it is
            // never a stale 0. u32 math avoids a debug overflow on a malformed
            // strength.
            stats.weight_max = ((strength as u32 >> 1) * 7 + 40) as u16;
        }
        if flag >= 3 {
            // Renaissance: stat cap + follower count.
            stats.stats_cap = r.i16()?;
            stats.followers = r.u8()?;
            stats.followers_max = r.u8()?;
        }
        if flag >= 4 {
            // AOS: resistances, luck, damage range, tithing points.
            stats.fire_resistance = r.i16()?;
            stats.cold_resistance = r.i16()?;
            stats.poison_resistance = r.i16()?;
            stats.energy_resistance = r.i16()?;
            stats.luck = r.u16()?;
            stats.damage_min = r.i16()?;
            stats.damage_max = r.i16()?;
            stats.tithing_points = r.u32()?;
        }
        if flag >= 6 {
            // The AOS combat tail: ServUO writes `(short)AOS.GetStatus(i)` for
            // `i in 0..=14` (`Packets.cs` MobileStatus), and ClassicUO reads
            // exactly those fifteen back in the same order — two independent
            // implementations agreeing, as with the stun/disarm pair.
            //
            // Read them the way ClassicUO does, defaulting a missing value to 0
            // instead of failing the packet (`p.Position + 2 > p.Length ? 0 :
            // ...`). It matters: an Enhanced-Client session gets *29* values
            // here, and a shard that sends a shorter tail than it advertises
            // would otherwise cost us the whole status update — name, HP and
            // all — over trailing bytes nothing reads.
            let mut next = || r.i16().unwrap_or(0);
            stats.aos = AosStatus {
                max_physical_resistance: next(),
                max_fire_resistance: next(),
                max_cold_resistance: next(),
                max_poison_resistance: next(),
                max_energy_resistance: next(),
                defense_chance_increase: next(),
                max_defense_chance_increase: next(),
                hit_chance_increase: next(),
                swing_speed_increase: next(),
                damage_increase: next(),
                lower_reagent_cost: next(),
                spell_damage_increase: next(),
                faster_cast_recovery: next(),
                faster_casting: next(),
                lower_mana_cost: next(),
            };
        }

        let m = world.mobile_mut(serial);
        m.stam = stam;
        m.stam_max = stam_max;
        m.mana = mana;
        m.mana_max = mana_max;
    }
    Ok(())
}

/// 0x98 UpdateName — `[id][len:u16][serial:u32][name: ASCII, fills to the end
/// of the frame]` (ClassicUO `UpdateName`). Existing-only: like ClassicUO,
/// which gates the rename on `world.Get(serial) != null`, we only rename a
/// mobile we already track — a stray/late 0x98 (or one whose serial is an
/// item) must never spawn a phantom mobile at (0,0). We additionally skip an
/// empty name defensively: ClassicUO would blank the entity, but losing a good
/// name to a stray empty packet is undesirable.
pub(super) fn update_name(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // skip id + 2-byte length
    let serial = r.u32()?;
    let name = ascii_string(r.rest());
    if !name.is_empty() {
        if let Some(m) = world.mobiles.get_mut(&serial) {
            m.name = name;
        }
    }
    Ok(())
}

pub(super) enum Vital {
    Hits,
    Mana,
    Stam,
}

/// 0xA1/0xA2/0xA3 — a single vital bar update: `[id][serial:u32][max:u16][cur:u16]`.
pub(super) fn vital(world: &mut World, frame: &[u8], which: Vital) -> PResult<()> {
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

/// 0x2D MobileAttributes — all three vital bars in one fixed packet:
/// `[id][serial:u32][hitsMax:u16][hits:u16][manaMax:u16][mana:u16]`
/// `[stamMax:u16][stam:u16]` (17 bytes).
///
/// ServUO sends this after entering a map and for a full vital refresh; ClassicUO
/// applies it to the same fields updated individually by 0xA1/0xA2/0xA3.
pub(super) fn mobile_attributes(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let hits_max = r.u16()?;
    let hits = r.u16()?;
    let mana_max = r.u16()?;
    let mana = r.u16()?;
    let stam_max = r.u16()?;
    let stam = r.u16()?;

    let mobile = world.mobile_mut(serial);
    mobile.hits_max = hits_max;
    mobile.hits = hits;
    mobile.mana_max = mana_max;
    mobile.mana = mana;
    mobile.stam_max = stam_max;
    mobile.stam = stam;
    Ok(())
}

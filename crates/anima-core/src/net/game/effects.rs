//! Audio-visual events: things the renderer plays once and forgets.
//!
//! Graphic effects (0x70/0xC0/0xC7), sound (0x54), music (0x6D), the two
//! animation families (0x6E and 0xE2's typed emotes), and the ambient state that
//! tints all of it — light (0x4F/0x4E), weather (0x65), season (0xBC), and the
//! game clock (0x5B).

use super::*;

/// 0x70 GraphicalEffect / 0xC0 HuedEffect / 0xC7 ParticleEffect — a spell bolt,
/// hit sparkle, explosion, or field visual. All three share the 28-byte 0x70 core
/// (big-endian): `[id][type:u8][src:u32][tgt:u32][graphic:u16][sx:u16][sy:u16]
/// [sz:i8][tx:u16][ty:u16][tz:i8][speed:u8][duration:u8][unk:u16][fixedDir:u8]
/// [explode:u8]`. 0xC0 (36 B) then adds `[hue:u32][renderMode:u32]`; 0xC7 (49 B)
/// adds 13 further particle bytes the 2D client ignores (rendered like 0xC0).
/// `hued` = false for 0x70 (hue forced to 0), true for 0xC0/0xC7 (low 16 bits of
/// the hue u32). Ported from ClassicUO `PacketHandlers.GraphicEffect`.
pub(super) fn graphic_effect(world: &mut World, frame: &[u8], hued: bool) -> PResult<()> {
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
pub(super) fn play_sound(world: &mut World, frame: &[u8]) -> PResult<()> {
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
pub(super) fn character_anim(world: &mut World, frame: &[u8]) -> PResult<()> {
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
pub(super) fn typed_anim(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let serial = r.u32()?;
    let kind = r.u16()?;
    let action = r.u16()?;
    let mode = r.u8()?;
    world.push_typed_anim(serial, kind, action, mode);
    Ok(())
}

/// 0x6D PlayMusic — `[id][musicID:u16]` (3 bytes). Records the current track.
pub(super) fn play_music(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let music_id = r.u16()?;
    // 0xFFFF is the conventional "stop music" sentinel.
    world.current_music = if music_id == 0xFFFF {
        None
    } else {
        Some(music_id)
    };
    Ok(())
}

/// 0x4F OverallLightLevel — `[id][level:u8]` (2 bytes). 0 = brightest day,
/// ~0x1F darkest night. The renderer darkens the scene by this level.
/// 0x72 SetWarMode — `[id][flag:u8][0x00][0x32][0x00]` (5 bytes). The server
/// echoes our authoritative war/peace stance: `flag` != 0 = war. ClassicUO reads
/// only the first byte after the id and ignores the trailing 3 (fixed padding).
pub(super) fn war_mode(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.war = r.u8()? != 0;
    Ok(())
}

pub(super) fn overall_light(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.light_level = r.u8()?;
    Ok(())
}

/// 0x4E PersonalLightLevel — `[id][serial:u32][level:u8]` (6 bytes). Stored only
/// for our own character; combined with the overall level in
/// [`World::effective_light`].
pub(super) fn personal_light(world: &mut World, frame: &[u8]) -> PResult<()> {
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
pub(super) fn weather(world: &mut World, frame: &[u8]) -> PResult<()> {
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
/// season here. The graphic substitution it drives (grass → snow, green tree →
/// bare) happens one layer up, in `anima_net::scene::season`, and touches only
/// fields that decide a PIXEL: `World` is the single source of truth for
/// pathing and for the agent contract, so its graphics stay exactly what the
/// server sent — which is also what ServUO's own `MovementImpl` walks on, since
/// it never consults `Map.Season`. Keeping the remap out of core is both the
/// rendering-concern rule (DESIGN.md D3) and the reason `anima-agent`'s brain
/// can't be fooled by a season it cannot see.
pub(super) fn season(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.season = r.u8()?;
    let _play_music = r.u8()?;
    Ok(())
}

/// 0xC8 ClientViewRange — `[id][range:u8]` (2 bytes, ClassicUO
/// `ClientViewRange`). The server's authoritative echo of the client's
/// requested draw range, in tiles.
pub(super) fn client_view_range(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    world.client_view_range = r.u8()?;
    Ok(())
}

/// 0x5B SetTime — `[id][hour:u8][minute:u8][second:u8]` (4 bytes). Ported from
/// `anima/anima/perception/handlers.py` `handle_game_time`; ClassicUO's own
/// `SetTime` handler is a no-op, but the in-game clock is useful perception
/// data, so we keep it as [`World::game_time`].
pub(super) fn set_time(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[1..]);
    let hour = r.u8()?;
    let minute = r.u8()?;
    let second = r.u8()?;
    world.game_time = Some(GameTime {
        hour,
        minute,
        second,
    });
    Ok(())
}

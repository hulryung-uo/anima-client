//! 0xF0 protocol extensions: where your party and guild are on the world map.
//!
//! Not a UO packet in the usual sense — it is the "Krrios" extension that Razor,
//! ClassicUO and ServUO all agreed on, multiplexed by a leading type byte the
//! way 0xBF is. ServUO registers it in `Scripts/Misc/ProtocolExtensions.cs`.
//!
//! The numbering is asymmetric and easy to get wrong, because the same values
//! mean different things in each direction:
//!
//! | sub  | client → server           | server → client       |
//! |------|---------------------------|-----------------------|
//! | 0x00 | query party positions     | tracking accepted     |
//! | 0x01 | query guild positions     | party positions       |
//! | 0x02 | —                         | guild positions       |
//!
//! So a reply of sub 0x01 answers a *request* of sub 0x00. The two record
//! layouts differ too: guild records carry a health percentage and are preceded
//! by a flag saying whether positions are included at all; party records are
//! always positions and never carry health.

use super::*;

/// 0xF0 — `[id][len:u16][type:u8][…]`. Ported from ClassicUO
/// `KrriosClientSpecial` (`PacketHandlers.cs:5620`), cross-checked against the
/// ServUO side that actually produces it (`ProtocolExtensions.cs`).
///
/// Types 0x03 (runebook contents) and 0x04 (guardline data) are read by nothing
/// in ClassicUO either — both cases are empty there — and 0xFE is Razor's
/// handshake, which this client is not. They are accepted and ignored rather
/// than treated as malformed, exactly as ClassicUO does.
pub(super) fn protocol_extension(world: &mut World, frame: &[u8]) -> PResult<()> {
    let mut r = PacketReader::new(&frame[3..]); // variable: id + u16 length
    let kind = r.u8()?;
    match kind {
        // Krrios' explicit "tracking accepted" — ClassicUO gates its polling on
        // this (`SetACKReceived` → `SetEnable(true)`). See the note on
        // `World::map_tracking` for why we cannot gate on it alone.
        0x00 => {}
        // Party: always positions, never health.
        0x01 => world.party_positions = read_positions(&mut r, true, false)?,
        // Guild: a leading flag says whether positions follow at all. When it is
        // 0 each record is a bare serial — a membership list, not a map feed —
        // so the records still have to be walked to reach the terminator, but
        // there is nothing to plot and we clear rather than keep stale pins.
        0x02 => {
            let locations = r.u8()? != 0;
            world.guild_positions = read_positions(&mut r, locations, true)?;
        }
        _ => return Ok(()),
    }
    // Any reply we understood is proof the shard speaks this extension — which
    // is the fact the driver's polling gate actually needs. ServUO NEVER sends
    // the 0x00 accept (its only 0xF0 senders are `AckPartyLocations` and
    // `AckGuildsLocations`), so gating on 0x00 the way ClassicUO does would
    // switch tracking off forever against the very server that implements it.
    world.map_tracking = true;
    Ok(())
}

/// Walk `[serial:u32]( [x:u16][y:u16][map:u8]( [hits:u8] ) )` records until the
/// zero serial that terminates the list.
///
/// `locations` is whether coordinates follow each serial at all — false only for
/// a guild reply that carried no positions, where the whole body is serials.
/// `hits` is whether a health percentage trails each position; guild records
/// carry one (0 when the member is dead), party records do not. Getting either
/// wrong desynchronises the walk and the terminator is then never found at a
/// record boundary, so both are decided by the caller from the type byte rather
/// than guessed per record.
fn read_positions(
    r: &mut PacketReader,
    locations: bool,
    hits: bool,
) -> PResult<Vec<TrackedMember>> {
    let mut out = Vec::new();
    loop {
        let serial = r.u32()?;
        if serial == 0 {
            break;
        }
        if !locations {
            continue;
        }
        let x = r.u16()?;
        let y = r.u16()?;
        let map = r.u8()?;
        let hits_pct = if hits { Some(r.u8()?) } else { None };
        out.push(TrackedMember {
            serial,
            x,
            y,
            map,
            hits_pct,
        });
    }
    Ok(out)
}

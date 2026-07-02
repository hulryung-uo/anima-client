//! Golden-packet regression tests: replay a handful of **real, captured**
//! game-phase packets through [`apply_packet`] and assert the resulting
//! [`World`] state.
//!
//! Per `CLAUDE.md`'s porting method, `../anima`'s `uo_proxy` packet captures are
//! this project's golden tests. Provenance for every fixture below:
//!
//! - **Source file:** `~/dev/uo/anima/data/trajectories/demo-20260419-114417.jsonl`
//!   (a real `uo_proxy` capture — schema `uo_proxy.packet.v1`, see that repo's
//!   `uo_proxy/README.md` for the format). This is a genuine human play session
//!   against a live ServUO, captured via `python -m uo_proxy` sitting between
//!   ClassicUO and the server (`direction: "S->C"`, `phase: "game"` lines are
//!   already Huffman-decompressed by the proxy for logging — exactly the bytes
//!   [`apply_packet`] expects).
//! - **Session:** `session_id = "1776566669-e86360f4"` (character "mr miner",
//!   serial `0x00016107`), captured 2026-04-19.
//! - Each test below cites the exact JSONL line's `hex` field it embeds (no
//!   whole capture files are copied into this repo — just the packet bytes).
//!
//! ## Recording more captures (see also `docs/DESIGN.md` §7)
//! ```sh
//! cd ~/dev/uo/anima
//! uv run python -m uo_proxy --listen 127.0.0.1:2593 --upstream 127.0.0.1:2594 \
//!     --out data/trajectories/demo-<name>.jsonl
//! # point ClassicUO (or /etc/hosts) at 127.0.0.1:2593, play a while, then grep
//! # the JSONL for the packet id(s) you want (`"pid":"0x78"` etc.) and copy the
//! # `hex` field into a new #[test] here, following the pattern below.
//! ```

use anima_core::net::apply_packet;
use anima_core::types::Serial;
use anima_core::world::World;

/// Decode a hex string (no separators, as captured by `uo_proxy`'s `hex` field)
/// into raw packet bytes. `apply_packet` takes the full frame (id + length
/// prefix + payload for variable-length packets; id + payload for fixed ones)
/// exactly as captured.
fn hex(s: &str) -> Vec<u8> {
    assert_eq!(s.len() % 2, 0, "odd-length hex string");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex byte"))
        .collect()
}

/// `0x11` CharacterStatus for **self** (flag=3, full stat block).
///
/// Capture: session `1776566669-e86360f4`, `S->C 0x11` —
/// `hex: "110046000161076d72206d696e657200000000000000000000000000000000000000000000004600460003000028001900190019001900190019000003e80000009700e10005"`
/// (name "mr miner", serial `0x00016107`, matching this session's player).
#[test]
fn golden_0x11_character_status_self_carries_full_stat_block() {
    let frame = hex(
        "110046000161076d72206d696e6572000000000000000000000000000000\
         00000000000000004600460003000028001900190019001900190019000003e80000009700e10005",
    );
    let mut w = World::new();
    w.player = Some(Serial(0x0001_6107));

    assert!(apply_packet(&mut w, &frame), "0x11 must be recognized");

    let m = w.mobiles.get(&0x0001_6107).expect("mobile created");
    assert_eq!(m.name, "mr miner");
    assert_eq!(m.hits, 70);
    assert_eq!(m.hits_max, 70);
    // Self + flag>=1 → the full stat block lands on PlayerStats/mobile vitals.
    assert_eq!(w.player_stats.strength, 40);
    assert_eq!(w.player_stats.dexterity, 25);
    assert_eq!(w.player_stats.intelligence, 25);
    assert_eq!(w.player_stats.gold, 1000);
    assert_eq!(w.player_stats.armor, 0);
    assert_eq!(w.player_stats.weight, 151);
    assert_eq!(m.stam, 25);
    assert_eq!(m.stam_max, 25);
    assert_eq!(m.mana, 25);
    assert_eq!(m.mana_max, 25);
}

/// `0x11` CharacterStatus for **another** mobile (flag=0: name/hits only, no
/// stat block — the packet is genuinely shorter, not just an ignored field).
///
/// Capture: same session, `S->C 0x11` (an NPC, "Regis") —
/// `hex: "11002b00000110526567697300000000000000000000000000000000000000000000000000001900190000"`
#[test]
fn golden_0x11_character_status_other_carries_name_and_hits_only() {
    let frame = hex(
        "11002b00000110526567697300000000000000000000000000000000000000000000000000001900190000",
    );
    // No player registered → `is_self` is false regardless, but this also
    // matches the real session (0x110 != the player's 0x16107).
    let mut w = World::new();

    assert!(apply_packet(&mut w, &frame));

    let m = w.mobiles.get(&0x0110).expect("mobile created");
    assert_eq!(m.name, "Regis");
    assert_eq!(m.hits, 25);
    assert_eq!(m.hits_max, 25);
    // flag=0 → no stat block was even present on the wire; PlayerStats untouched.
    assert_eq!(w.player_stats.strength, 0);
    assert_eq!(w.player_stats.gold, 0);
}

/// `0x78` UpdateObject (mobile incoming) for a non-player mobile — exercises
/// the full field set (position/direction/hue/notoriety all apply; only the
/// player's own entry would skip pos/dir, see `net::game::mobile_incoming`).
///
/// Capture: same session, `S->C 0x78`, no worn items (terminator right after
/// the header) — `hex: "7800170000329800d909b6019d0f010908000300000000"`
#[test]
fn golden_0x78_mobile_incoming_populates_position_hue_and_notoriety() {
    let frame = hex("7800170000329800d909b6019d0f010908000300000000");
    let mut w = World::new();

    assert!(apply_packet(&mut w, &frame));

    let m = w.mobiles.get(&0x3298).expect("mobile created");
    assert_eq!(m.body, 217);
    assert_eq!(m.pos.x, 2486);
    assert_eq!(m.pos.y, 413);
    assert_eq!(m.pos.z, 15);
    assert_eq!(m.direction, 1);
    assert_eq!(m.hue, 2312);
    assert_eq!(m.notoriety, 3); // gray/criminal
}

/// `0x20` MobileUpdate (position/appearance reset) for the player's own avatar.
///
/// Capture: same session, `S->C 0x20` —
/// `hex: "200001610701900083ea0009c201990000810f"`
#[test]
fn golden_0x20_mobile_update_repositions_the_player() {
    let frame = hex("200001610701900083ea0009c201990000810f");
    let mut w = World::new();
    w.player = Some(Serial(0x0001_6107));

    assert!(apply_packet(&mut w, &frame));

    let m = w.mobiles.get(&0x0001_6107).expect("mobile created");
    assert_eq!(m.body, 400);
    assert_eq!(m.hue, 0x83EA);
    assert_eq!(m.pos.x, 2498);
    assert_eq!(m.pos.y, 409);
    assert_eq!(m.pos.z, 15);
    assert_eq!(m.direction, 1);
}

/// `0xF3` WorldItemHS — a ground item (High Seas fixed-length item format; this
/// capture never emits the legacy `0x1A`, i.e. the client negotiated HS+).
///
/// Capture: same session, `S->C 0xF3` —
/// `hex: "f300010040005986100b000001000109d201980f2b0000000000"`
#[test]
fn golden_0xf3_world_item_hs_places_a_ground_item() {
    let frame = hex("f300010040005986100b000001000109d201980f2b0000000000");
    let mut w = World::new();

    assert!(apply_packet(&mut w, &frame));

    let it = w.items.get(&0x4000_5986).expect("item created");
    assert_eq!(it.graphic, 0x100B);
    assert_eq!(it.amount, 1);
    assert_eq!(it.pos.x, 2514);
    assert_eq!(it.pos.y, 408);
    assert_eq!(it.pos.z, 15);
    assert!(it.container.is_none(), "ground item, no container");
}

/// `0x3C` ContainerContent — a full 19-item container refresh. Also exercises
/// the handler's stale-item pruning: we seed the world with an item that
/// claims to be in this container but isn't in the fresh payload, and confirm
/// it's dropped (ServUO's full-refresh semantics — see `container_content`'s
/// doc). The pre-existing stale item is synthetic (`apply_packet` can't know
/// about it from the wire alone); the 19-item payload itself is real.
///
/// Capture: same session, `S->C 0x3C` (a corpse loot, container `0x40085dd2`) —
/// `hex: "3c0181001340085dd30ff1000001009000450040085dd2000040085dd40eed0003e8\
///   003900800140085dd2000040085dd50a28000001009900810240085dd2000040085dd6\
///   0f520000010061006a0340085dd2000040085dd71eb8000001005f004b0440085dd200\
///   0040085dd81bf2000032002700580540085dd2000040085dd9105b00000100230049\
///   0640085dd2000040085dda1051000001002900550740085dd2000040085ddb105d00\
///   0001002d00630840085dd2000040085ddc104d0000010065005f0940085dd2000040\
///   085ddd0fbb000001003e00410a40085dd2000040085dde0e86000001006f00810b40\
///   085dd2000040085ddf0e860000010075006a0c40085dd2000040085de01bf20000320\
///   08b00590d40085dd2000040085de20e86000001002400820e40085dd2000040085de3\
///   0e76000001009a00660f40085dd2000040085dec1f30000001009200841040085dd2\
///   000040085ded1f3b000001008500491140085dd2000040085dee1f36000001008900\
///   771240085dd20000"`
#[test]
fn golden_0x3c_container_content_refreshes_and_prunes_stale_items() {
    let frame = hex(
        "3c0181001340085dd30ff1000001009000450040085dd2000040085dd40eed0003e8\
         003900800140085dd2000040085dd50a28000001009900810240085dd2000040085dd6\
         0f520000010061006a0340085dd2000040085dd71eb8000001005f004b0440085dd200\
         0040085dd81bf2000032002700580540085dd2000040085dd9105b00000100230049\
         0640085dd2000040085dda1051000001002900550740085dd2000040085ddb105d00\
         0001002d00630840085dd2000040085ddc104d0000010065005f0940085dd2000040\
         085ddd0fbb000001003e00410a40085dd2000040085dde0e86000001006f00810b40\
         085dd2000040085ddf0e860000010075006a0c40085dd2000040085de01bf20000320\
         08b00590d40085dd2000040085de20e86000001002400820e40085dd2000040085de3\
         0e76000001009a00660f40085dd2000040085dec1f30000001009200841040085dd2\
         000040085ded1f3b000001008500491140085dd2000040085dee1f36000001008900\
         771240085dd20000",
    );

    let mut w = World::new();
    // Seed a stale item: claims to be in the same container, but its serial
    // (0xDEADBEEF) never appears in the fresh 19-item payload above.
    let stale = w.item_mut(0xDEAD_BEEF);
    stale.container = Some(0x4008_5dd2);
    stale.graphic = 0x0EED;

    assert!(apply_packet(&mut w, &frame));

    let in_container: Vec<u32> = w
        .items
        .values()
        .filter(|it| it.container == Some(0x4008_5dd2))
        .map(|it| it.serial)
        .collect();
    assert_eq!(in_container.len(), 19, "19 fresh items, stale one pruned");
    assert!(!w.items.contains_key(&0xDEAD_BEEF), "stale item dropped");

    // Spot-check one real record: serial 0x40085dd4, a stack of 1000.
    let stacked = w.items.get(&0x4008_5dd4).expect("stacked item present");
    assert_eq!(stacked.graphic, 0x0EED);
    assert_eq!(stacked.amount, 1000);
    assert_eq!(stacked.container, Some(0x4008_5dd2));
}

/// `0x1D` Delete — the wire payload is just a serial, so we seed a mobile with
/// that serial to observe the removal (the byte content is real; the
/// pre-existing entity is synthetic, since a serial alone can't say what it
/// was).
///
/// Capture: same session, `S->C 0x1D` — `hex: "1d00015ad3"`
#[test]
fn golden_0x1d_delete_removes_the_entity() {
    let frame = hex("1d00015ad3");
    let mut w = World::new();
    w.mobile_mut(0x0001_5ad3).name = "gone".to_string();
    assert!(w.mobiles.contains_key(&0x0001_5ad3));

    assert!(apply_packet(&mut w, &frame));

    assert!(!w.mobiles.contains_key(&0x0001_5ad3));
}

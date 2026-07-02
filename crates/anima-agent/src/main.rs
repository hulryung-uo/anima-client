//! `anima-agent` — run an autonomous brain on the live server.
//!
//! Usage: `anima-agent [host] [port] [user] [pass] [ticks]`
//! Connects, then each tick: pump the network, observe, let the brain decide,
//! execute the actions. Logs perception + decisions so you can watch it live.

use std::time::Duration;

use anima_agent::WanderBrain;
use anima_core::{Action, Brain};
use anima_core::net::LoginConfig;
use anima_net::{Endpoint, Session};

fn main() {
    let mut a = std::env::args().skip(1);
    let host = a.next().unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(2594);
    let user = a.next().unwrap_or_else(|| "animaagent".into());
    let pass = a.next().unwrap_or_else(|| "animaagent".into());
    let ticks: u32 = a.next().and_then(|s| s.parse().ok()).unwrap_or(40);

    let cfg = LoginConfig {
        username: user.clone(),
        password: pass,
        ..Default::default()
    };
    println!("agent: connecting to {host}:{port} as {user} ...");
    let mut s = match Session::connect_and_login(&Endpoint::new(host, port), cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("login failed: {e}");
            std::process::exit(1);
        }
    };
    let p0 = s.world.player_mobile().cloned().unwrap_or_default();
    println!("agent: in world as {} at ({}, {})", p0.name, p0.pos.x, p0.pos.y);

    let mut brain = WanderBrain::new();
    // Settle: pump a moment so the initial perception (status, nearby) lands.
    let _ = s.observe(Duration::from_millis(800));

    for t in 0..ticks {
        let obs = s.observation();
        let actions = brain.decide(&obs);

        // Log a compact perception + decision line.
        let act_str: Vec<String> = actions
            .iter()
            .map(|a| match a {
                Action::Walk { dir, run } => format!("walk(d{dir}{})", if *run { ",run" } else { "" }),
                Action::WalkTo { x, y } => format!("walkto({x},{y})"),
                Action::Say { text } => format!("say({text:?})"),
                Action::PartySay { text } => format!("party({text:?})"),
                Action::PickUp { serial, .. } => format!("pickup(0x{serial:08X})"),
                Action::Drop { serial, x, y, z, container } => {
                    format!("drop(0x{serial:08X}→{x},{y},{z}@0x{container:08X})")
                }
                Action::Equip { serial, layer } => format!("equip(0x{serial:08X}@L{layer})"),
                Action::Attack { serial } => format!("attack(0x{serial:08X})"),
                Action::AutoAttack => "autoattack".into(),
                Action::AttackLast => "attacklast".into(),
                Action::Use { serial } => format!("use(0x{serial:08X})"),
                Action::Click { serial } => format!("click(0x{serial:08X})"),
                Action::WarMode { on } => format!("war({on})"),
                Action::CastSpell { spell } => format!("cast({spell})"),
                Action::TargetObject { serial } => format!("target(0x{serial:08X})"),
                Action::TargetGround { x, y, z, graphic } => {
                    format!("targetXY({x},{y},{z},0x{graphic:04X})")
                }
                Action::TargetCancel => "targetcancel".into(),
                Action::BuyItems { vendor, items } => {
                    format!("buy(0x{vendor:08X}×{})", items.len())
                }
                Action::SellItems { vendor, items } => {
                    format!("sell(0x{vendor:08X}×{})", items.len())
                }
                Action::GumpResponse { serial, button, .. } => {
                    format!("gump(0x{serial:08X} btn={button})")
                }
                Action::PopupRequest { serial } => format!("popupreq(0x{serial:08X})"),
                Action::PopupSelect { serial, index } => {
                    format!("popupsel(0x{serial:08X} i={index})")
                }
                Action::BookRequest { serial, pages } => {
                    format!("bookreq(0x{serial:08X}×{pages})")
                }
                Action::UseAbility { ability } => format!("ability({ability})"),
                Action::SkillLock { skill, lock } => format!("skilllock({skill}={lock})"),
                Action::UseSkill { skill } => format!("useskill({skill})"),
                Action::OplRequest { serial } => format!("oplreq(0x{serial:08X})"),
                Action::PartyInvite => "partyinvite".into(),
                Action::PartyAccept { leader } => format!("partyaccept(0x{leader:08X})"),
                Action::PartyDecline { leader } => format!("partydecline(0x{leader:08X})"),
                Action::PartyLeave => "partyleave".into(),
            })
            .collect();
        println!(
            "t{t:02}  pos=({},{})  mobs={} items={}  → {}",
            obs.player.pos.x,
            obs.player.pos.y,
            obs.mobiles.len(),
            obs.items.len(),
            if act_str.is_empty() { "(idle)".into() } else { act_str.join(", ") }
        );

        for action in &actions {
            if let Err(e) = s.apply_action(action) {
                eprintln!("agent: action error: {e}");
            }
        }
        if s.observe(Duration::from_millis(450)).is_err() {
            eprintln!("agent: connection closed");
            break;
        }
    }

    let pend = s.world.player_mobile().cloned().unwrap_or_default();
    let dx = pend.pos.x as i32 - p0.pos.x as i32;
    let dy = pend.pos.y as i32 - p0.pos.y as i32;
    println!(
        "agent: done. moved from ({},{}) to ({},{})  (net dx={dx} dy={dy}, confirms={} denies={})",
        p0.pos.x, p0.pos.y, pend.pos.x, pend.pos.y, s.confirms, s.denies
    );
}

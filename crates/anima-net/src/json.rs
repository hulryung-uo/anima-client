//! JSON wire format for the agent bridge (the brain↔body IPC).
//!
//! `anima-core` is zero-dep and has no serde, so the JSON shapes for its
//! [`Observation`]/[`Action`] live here (anima-net has `serde_json`). These
//! shapes are the contract mirrored by `anima2/anima2/contract.py` — keep the
//! two in lockstep.

use anima_core::agent::{Action, ItemView, MobileView, Observation, PlayerView, SkillView};
use anima_core::types::Position;
use anima_core::world::JournalEntry;
use serde_json::{json, Value};

fn pos_json(p: &Position) -> Value {
    json!({ "x": p.x, "y": p.y, "z": p.z })
}

fn player_json(p: &PlayerView) -> Value {
    json!({
        "serial": p.serial, "name": p.name, "pos": pos_json(&p.pos),
        "direction": p.direction, "hits": p.hits, "hits_max": p.hits_max,
        "mana": p.mana, "mana_max": p.mana_max, "stam": p.stam, "stam_max": p.stam_max,
        "strength": p.strength, "dexterity": p.dexterity, "intelligence": p.intelligence,
        "gold": p.gold, "weight": p.weight,
    })
}

fn mobile_json(m: &MobileView) -> Value {
    json!({
        "serial": m.serial, "name": m.name, "pos": pos_json(&m.pos), "body": m.body,
        "notoriety": m.notoriety, "hits": m.hits, "hits_max": m.hits_max, "distance": m.distance,
    })
}

fn item_json(i: &ItemView) -> Value {
    json!({
        "serial": i.serial, "graphic": i.graphic, "amount": i.amount,
        "pos": pos_json(&i.pos), "container": i.container, "layer": i.layer,
        "distance": i.distance,
    })
}

fn skill_json(s: &SkillView) -> Value {
    json!({ "id": s.id, "value": s.value, "base": s.base, "cap": s.cap, "lock": s.lock })
}

fn journal_json(j: &JournalEntry) -> Value {
    json!({
        "serial": j.serial, "name": j.name, "text": j.text,
        "msg_type": j.msg_type, "hue": j.hue,
    })
}

/// Serialize an [`Observation`] to the brain-facing JSON shape.
pub fn observation_to_json(obs: &Observation) -> Value {
    let pending = obs.pending_target.map(|t| {
        json!({ "target_type": t.target_type, "cursor_id": t.cursor_id, "cursor_flag": t.cursor_flag })
    });
    json!({
        "player": player_json(&obs.player),
        "mobiles": obs.mobiles.iter().map(mobile_json).collect::<Vec<_>>(),
        "items": obs.items.iter().map(item_json).collect::<Vec<_>>(),
        "new_journal": obs.new_journal.iter().map(journal_json).collect::<Vec<_>>(),
        "pending_target": pending,
        "skills": obs.skills.iter().map(skill_json).collect::<Vec<_>>(),
    })
}

/// Parse an [`Action`] from its JSON form (externally tagged by `"type"`).
pub fn action_from_json(v: &Value) -> Result<Action, String> {
    let t = v.get("type").and_then(Value::as_str).ok_or("action missing 'type'")?;
    let u32f = |k: &str| v.get(k).and_then(Value::as_u64).map(|n| n as u32);
    let req_u32 = |k: &str| u32f(k).ok_or_else(|| format!("action {t} missing u32 '{k}'"));
    match t {
        "Walk" => Ok(Action::Walk {
            dir: v.get("dir").and_then(Value::as_u64).unwrap_or(0) as u8,
            run: v.get("run").and_then(Value::as_bool).unwrap_or(false),
        }),
        "Say" => Ok(Action::Say {
            text: v.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
        }),
        "Attack" => Ok(Action::Attack { serial: req_u32("serial")? }),
        "Use" => Ok(Action::Use { serial: req_u32("serial")? }),
        "Click" => Ok(Action::Click { serial: req_u32("serial")? }),
        "PickUp" => Ok(Action::PickUp {
            serial: req_u32("serial")?,
            amount: v.get("amount").and_then(Value::as_u64).unwrap_or(1) as u16,
        }),
        "WarMode" => Ok(Action::WarMode {
            on: v.get("on").and_then(Value::as_bool).unwrap_or(false),
        }),
        "TargetObject" => Ok(Action::TargetObject { serial: req_u32("serial")? }),
        "TargetGround" => Ok(Action::TargetGround {
            x: v.get("x").and_then(Value::as_u64).unwrap_or(0) as u16,
            y: v.get("y").and_then(Value::as_u64).unwrap_or(0) as u16,
            z: v.get("z").and_then(Value::as_i64).unwrap_or(0) as i16,
            graphic: v.get("graphic").and_then(Value::as_u64).unwrap_or(0) as u16,
        }),
        other => Err(format!("unknown action type: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_roundtrips_from_python_shape() {
        let walk = action_from_json(&json!({"type": "Walk", "dir": 3, "run": true})).unwrap();
        assert_eq!(walk, Action::Walk { dir: 3, run: true });

        let pick =
            action_from_json(&json!({"type": "PickUp", "serial": 1073741825u64, "amount": 5}))
                .unwrap();
        assert_eq!(pick, Action::PickUp { serial: 0x4000_0001, amount: 5 });

        assert!(action_from_json(&json!({"type": "Nope"})).is_err());

        let obj = action_from_json(&json!({"type": "TargetObject", "serial": 4242})).unwrap();
        assert_eq!(obj, Action::TargetObject { serial: 4242 });

        let ground = action_from_json(
            &json!({"type": "TargetGround", "x": 1000, "y": 2000, "z": -5, "graphic": 420}),
        )
        .unwrap();
        assert_eq!(ground, Action::TargetGround { x: 1000, y: 2000, z: -5, graphic: 420 });
    }

    #[test]
    fn observation_serializes_pending_target() {
        use anima_core::world::{TargetCursor, World};
        let mut w = World::default();
        w.pending_target = Some(TargetCursor { target_type: 1, cursor_id: 0xABCD, cursor_flag: 0 });
        let v = observation_to_json(&w.observe(&mut 0));
        assert_eq!(v["pending_target"]["cursor_id"], 0xABCD);
        assert_eq!(v["pending_target"]["target_type"], 1);
    }

    #[test]
    fn observation_json_has_expected_keys() {
        let obs = Observation::default();
        let v = observation_to_json(&obs);
        for k in ["player", "mobiles", "items", "new_journal"] {
            assert!(v.get(k).is_some(), "missing key {k}");
        }
        assert!(v["player"].get("hits_max").is_some());
    }
}

//! Autonomous brains built on the `anima-core` Observation/Action contract.
//!
//! A [`Brain`] never touches the network or parses packets — it only reads an
//! [`Observation`] and returns [`Action`]s. The driver (`anima-net::Session`)
//! does the IO. This is the whole point of the core⊥brain split: the same brain
//! runs against the live server, a replay, or a test world unchanged.

use anima_core::{dir_toward, Action, Brain, Observation};

/// Notoriety byte for a murderer/red ("kill on sight").
const NOTO_MURDERER: u8 = 6;

/// A simple but genuinely autonomous wanderer:
/// - **flees** a nearby red (murderer) mobile,
/// - **greets** when someone speaks nearby (once per speaker),
/// - **picks up** a ground item within reach, walking to it,
/// - otherwise **explores**, changing direction when it gets stuck.
#[derive(Default)]
pub struct WanderBrain {
    dir: u8,
    steps_in_dir: u32,
    last_pos: (u16, u16),
    stuck: u32,
    greeted: std::collections::HashSet<String>,
    tick: u32,
}

impl WanderBrain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cheap deterministic "random" direction that varies over time without
    /// needing an RNG (Date/rand are unavailable in some targets).
    fn wander_dir(&self) -> u8 {
        // Cardinals only (0,2,4,6) make for clearer exploration than diagonals.
        [0u8, 2, 4, 6][((self.tick.wrapping_mul(2654435761)) >> 13 & 3) as usize]
    }
}

impl Brain for WanderBrain {
    fn decide(&mut self, obs: &Observation) -> Vec<Action> {
        self.tick = self.tick.wrapping_add(1);
        let p = &obs.player;
        let here = (p.pos.x, p.pos.y);

        // Stuck detection: position unchanged since last decide => blocked.
        if here == self.last_pos {
            self.stuck += 1;
        } else {
            self.stuck = 0;
        }
        self.last_pos = here;

        let mut actions = Vec::new();

        // 1) Flee the nearest red within 5 tiles — walk directly away from it.
        if let Some(threat) = obs
            .mobiles
            .iter()
            .find(|m| m.notoriety == NOTO_MURDERER && m.distance <= 5)
        {
            let dx = p.pos.x as i32 - threat.pos.x as i32;
            let dy = p.pos.y as i32 - threat.pos.y as i32;
            if let Some(dir) = dir_toward(dx, dy) {
                actions.push(Action::Walk { dir, run: true });
                return actions;
            }
        }

        // 2) Greet new *player/NPC* speakers (msg_type 0 = regular speech), once
        //    each. Skip server/system lines (serial 0 or the "System" sender).
        for line in &obs.new_journal {
            let is_real_speaker = line.msg_type == 0
                && line.serial != 0
                && !line.name.is_empty()
                && line.name != "System"
                && line.name != p.name;
            if is_real_speaker && self.greeted.insert(line.name.clone()) {
                actions.push(Action::Say {
                    text: format!("Well met, {}!", line.name),
                });
            }
        }

        // 3) Grab a nearby ground item; step toward it if not adjacent.
        if let Some(item) = obs
            .items
            .iter()
            .filter(|it| it.container.is_none())
            .min_by_key(|it| it.distance)
        {
            if item.distance == 0 {
                actions.push(Action::PickUp {
                    serial: item.serial,
                    amount: 1,
                });
                return actions;
            } else if item.distance <= 3 {
                let dx = item.pos.x as i32 - p.pos.x as i32;
                let dy = item.pos.y as i32 - p.pos.y as i32;
                if let Some(dir) = dir_toward(dx, dy) {
                    actions.push(Action::Walk { dir, run: false });
                    return actions;
                }
            }
        }

        // 4) Explore. Pick a fresh direction when stuck or after a stretch.
        if self.stuck >= 1 || self.steps_in_dir >= 6 {
            self.dir = self.wander_dir();
            self.steps_in_dir = 0;
        }
        self.steps_in_dir += 1;
        actions.push(Action::Walk {
            dir: self.dir,
            run: false,
        });
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anima_core::agent::MobileView;
    use anima_core::types::Position;

    fn obs_at(x: u16, y: u16) -> Observation {
        let mut o = Observation::default();
        o.player.pos = Position { x, y, z: 0 };
        o
    }

    #[test]
    fn flees_red_mobile() {
        let mut b = WanderBrain::new();
        let mut o = obs_at(100, 100);
        o.mobiles.push(MobileView {
            serial: 1,
            name: "PK".into(),
            pos: Position { x: 103, y: 100, z: 0 },
            body: 0x190,
            notoriety: NOTO_MURDERER,
            hits: 1,
            hits_max: 1,
            distance: 3,
        });
        // Threat is to the east (+x), so we should flee west (dir 6).
        let acts = b.decide(&o);
        assert_eq!(acts, vec![Action::Walk { dir: 6, run: true }]);
    }

    #[test]
    fn explores_when_alone() {
        let mut b = WanderBrain::new();
        let acts = b.decide(&obs_at(100, 100));
        assert!(matches!(acts.as_slice(), [Action::Walk { .. }]));
    }

    #[test]
    fn greets_a_speaker_once() {
        let mut b = WanderBrain::new();
        let mut o = obs_at(100, 100);
        o.new_journal.push(anima_core::world::JournalEntry {
            serial: 9,
            name: "Hastin".into(),
            text: "hello there".into(),
            msg_type: 0,
            hue: 0,
        });
        let acts = b.decide(&o);
        assert!(acts.iter().any(|a| matches!(a, Action::Say { text } if text.contains("Hastin"))));
        // Second time: no repeat greeting (journal already consumed; new empty).
        let acts2 = b.decide(&obs_at(100, 100));
        assert!(!acts2.iter().any(|a| matches!(a, Action::Say { .. })));
    }
}

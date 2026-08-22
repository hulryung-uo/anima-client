//! Autonomous brains built on the `anima-core` Observation/Action contract.
//!
//! A [`Brain`] never touches the network or parses packets — it only reads an
//! [`Observation`] and returns [`Action`]s. The driver (`anima-net::Session`)
//! does the IO. This is the whole point of the core⊥brain split: the same brain
//! runs against the live server, a replay, or a test world unchanged.

use anima_core::agent::{SpeechMode, TerrainTile};
use anima_core::net::movement::direction_delta;
use anima_core::{dir_toward, Action, Brain, Observation};

/// Notoriety byte for a murderer/red ("kill on sight").
const NOTO_MURDERER: u8 = 6;

/// The tile one step in UO direction `dir`, from the observation's walkability
/// window — `None` when the driver surveyed no terrain (then a brain has to go
/// back to walking into things to find out where they are) or when that step
/// falls outside the window.
fn step_tile(obs: &Observation, dir: u8) -> Option<TerrainTile> {
    let view = obs.terrain.as_ref()?;
    let (dx, dy) = direction_delta(dir);
    let x = obs.player.pos.x.checked_add_signed(dx as i16)?;
    let y = obs.player.pos.y.checked_add_signed(dy as i16)?;
    view.at(x, y)
}

/// Whether stepping `dir` is known to be pointless. Conservative on purpose:
/// with no terrain perception nothing is "known blocked", so behaviour is
/// exactly what it was before the window existed.
fn known_blocked(obs: &Observation, dir: u8) -> bool {
    step_tile(obs, dir).is_some_and(|t| !t.walkable)
}

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
                    mode: SpeechMode::Say,
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
        // With terrain perception, don't walk into what we can already see is a
        // wall — turn now instead of spending a tick discovering it by bumping.
        // A closed door isn't a wall: open it and step through next tick, which
        // is a route a blind wanderer could never take at all.
        if let Some(tile) = step_tile(obs, self.dir) {
            if let Some(door) = tile.door {
                actions.push(Action::Use { serial: door });
                return actions;
            }
            if !tile.walkable {
                if let Some(open) = (0u8..8).find(|&d| !known_blocked(obs, d)) {
                    self.dir = open;
                    self.steps_in_dir = 0;
                } // else: boxed in — step anyway and let stuck-detection run.
            }
        }
        self.steps_in_dir += 1;
        actions.push(Action::Walk {
            dir: self.dir,
            run: false,
        });
        actions
    }
}

/// Hunt nearby hostiles, loot their corpses, bandage when hurt.
///
/// Uses the same Observation/Action contract as [`WanderBrain`]: war-mode +
/// attack the nearest grey/criminal/enemy/murderer in 8 tiles; open a corpse
/// (`0x2006`) in reach and pick up what spilled; bandage (`0x0E21`) when HP
/// drops below half.
#[derive(Default)]
pub struct HunterBrain {
    wander: WanderBrain,
    looting: Option<u32>,
    looted: std::collections::HashSet<u32>,
}

impl HunterBrain {
    pub fn new() -> Self {
        Self::default()
    }
}

const BANDAGE_GRAPHIC: u16 = 0x0E21;
const CORPSE_GRAPHIC: u16 = 0x2006;

fn is_hostile(noto: u8) -> bool {
    matches!(noto, 3..=6)
}

impl Brain for HunterBrain {
    fn decide(&mut self, obs: &Observation) -> Vec<Action> {
        let p = &obs.player;
        if p.dead {
            return self.wander.decide(obs);
        }

        if p.hits_max > 0 && p.hits * 2 < p.hits_max {
            if let Some(bandage) = obs
                .items
                .iter()
                .find(|it| !it.is_multi && it.graphic == BANDAGE_GRAPHIC && it.container.is_some())
            {
                return vec![Action::BandageTarget {
                    bandage: bandage.serial,
                    target: p.serial,
                }];
            }
        }

        if let Some(corpse) = self.looting {
            if let Some(loot) = obs
                .items
                .iter()
                .filter(|it| it.container == Some(corpse) && !it.is_multi)
                .min_by_key(|it| it.serial)
            {
                return vec![Action::PickUp {
                    serial: loot.serial,
                    amount: loot.amount.max(1),
                }];
            }
            self.looting = None;
        }

        if let Some(corpse) = obs.items.iter().find(|it| {
            !it.is_multi
                && it.graphic == CORPSE_GRAPHIC
                && it.distance <= 2
                && it.container.is_none()
                && !self.looted.contains(&it.serial)
        }) {
            self.looted.insert(corpse.serial);
            self.looting = Some(corpse.serial);
            return vec![Action::Use {
                serial: corpse.serial,
            }];
        }

        if let Some(prey) = obs
            .mobiles
            .iter()
            .filter(|m| is_hostile(m.notoriety) && m.distance <= 8)
            .min_by_key(|m| m.distance)
        {
            let mut actions = Vec::new();
            if !obs.war {
                actions.push(Action::WarMode { on: true });
            }
            if prey.distance > 1 {
                let dx = prey.pos.x as i32 - p.pos.x as i32;
                let dy = prey.pos.y as i32 - p.pos.y as i32;
                if let Some(dir) = dir_toward(dx, dy) {
                    actions.push(Action::Walk { dir, run: true });
                }
            } else {
                actions.push(Action::Attack {
                    serial: prey.serial,
                });
            }
            return actions;
        }

        self.wander.decide(obs)
    }
}

/// POST the observation JSON to `url` and parse the response as an array of
/// contract actions (`{"type":"Walk",...}`). HTTP only (not HTTPS); meant for
/// a local LLM/RL sidecar. Failures fall back to [`WanderBrain`].
pub struct LlmBrain {
    url: String,
    wander: WanderBrain,
}

impl LlmBrain {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            wander: WanderBrain::new(),
        }
    }
}

impl Brain for LlmBrain {
    fn decide(&mut self, obs: &Observation) -> Vec<Action> {
        match post_actions(&self.url, obs) {
            Ok(actions) if !actions.is_empty() => actions,
            _ => self.wander.decide(obs),
        }
    }
}

fn post_actions(url: &str, obs: &Observation) -> Result<Vec<Action>, String> {
    let body = anima_contract_json::observation_to_json(obs).to_string();
    let resp = http_post_json(url, &body).map_err(|e| e.to_string())?;
    actions_from_json_body(&resp)
}

/// Parse a JSON array of contract actions (`{"type":"Walk",...}`).
fn actions_from_json_body(body: &str) -> Result<Vec<Action>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let list = v
        .as_array()
        .ok_or_else(|| "LLM response is not a JSON array".to_string())?;
    let mut out = Vec::new();
    for item in list {
        out.push(anima_contract_json::action_from_json(item)?);
    }
    Ok(out)
}

fn http_post_json(url: &str, body: &str) -> std::io::Result<String> {
    let url = url.strip_prefix("http://").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "ANIMA_LLM_URL must be http://host[:port]/path (HTTPS is not supported)",
        )
    })?;
    let (hostport, path) = url.split_once('/').unwrap_or((url, ""));
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        format!("/{path}")
    };
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => (h, p.parse().unwrap_or(80)),
        None => (hostport, 80u16),
    };
    let mut stream = std::net::TcpStream::connect((host, port))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    use std::io::{Read, Write};
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes())?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;
    let (_, rest) = buf
        .split_once("\r\n\r\n")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no HTTP body"))?;
    Ok(rest.to_string())
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

    /// An observation whose terrain window says every tile is walkable except
    /// those in `walls`, with an optional door serial on `door_at`.
    fn obs_with_terrain(
        x: u16,
        y: u16,
        radius: u8,
        walls: &[(u16, u16)],
        door_at: Option<((u16, u16), u32)>,
    ) -> Observation {
        use anima_core::agent::TerrainView;
        let mut o = obs_at(x, y);
        let origin = (x - u16::from(radius), y - u16::from(radius));
        let side = u16::from(radius) * 2 + 1;
        let mut tiles = Vec::new();
        for dy in 0..side {
            for dx in 0..side {
                let at = (origin.0 + dx, origin.1 + dy);
                tiles.push(TerrainTile {
                    walkable: !walls.contains(&at),
                    z: 0,
                    door: door_at.filter(|(t, _)| *t == at).map(|(_, s)| s),
                });
            }
        }
        o.terrain = Some(TerrainView {
            origin,
            radius,
            tiles,
        });
        o
    }

    #[test]
    fn wanders_around_a_wall_it_can_see() {
        let mut b = WanderBrain::new();
        // Box the player in on every side but west (dir 6).
        let walls = [
            (100, 99),
            (101, 99),
            (101, 100),
            (101, 101),
            (100, 101),
            (99, 101),
            (99, 99),
        ];
        let o = obs_with_terrain(100, 100, 2, &walls, None);
        let acts = b.decide(&o);
        // Whatever direction the wanderer picked, it must not be one it can
        // already see is a wall — the whole point of terrain perception.
        match acts.last() {
            Some(Action::Walk { dir, .. }) => {
                assert_eq!(*dir, 6, "west is the only open step; got dir {dir}")
            }
            other => panic!("expected a walk, got {other:?}"),
        }
    }

    #[test]
    fn opens_a_door_instead_of_walking_into_it() {
        let mut b = WanderBrain::new();
        // Put a door on every neighbouring tile so whichever direction the
        // wanderer picks, it meets the door.
        let mut o = obs_with_terrain(100, 100, 1, &[], None);
        for tile in o.terrain.as_mut().unwrap().tiles.iter_mut() {
            tile.door = Some(0x4001);
        }
        assert_eq!(b.decide(&o), vec![Action::Use { serial: 0x4001 }]);
    }

    #[test]
    fn without_terrain_the_wanderer_behaves_exactly_as_before() {
        // No window => nothing is "known blocked" => the old blind walk.
        let mut b = WanderBrain::new();
        let acts = b.decide(&obs_at(100, 100));
        assert!(matches!(acts.last(), Some(Action::Walk { .. })));
    }

    #[test]
    fn flees_red_mobile() {
        let mut b = WanderBrain::new();
        let mut o = obs_at(100, 100);
        o.mobiles.push(MobileView {
            serial: 1,
            name: "PK".into(),
            pos: Position {
                x: 103,
                y: 100,
                z: 0,
            },
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
            cliloc: 0,
            ..Default::default()
        });
        let acts = b.decide(&o);
        assert!(acts.iter().any(|a| matches!(a, Action::Say { text, mode }
                if text.contains("Hastin") && *mode == SpeechMode::Say)));
        // Second time: no repeat greeting (journal already consumed; new empty).
        let acts2 = b.decide(&obs_at(100, 100));
        assert!(!acts2.iter().any(|a| matches!(a, Action::Say { .. })));
    }

    fn item(
        serial: u32,
        graphic: u16,
        distance: u32,
        container: Option<u32>,
    ) -> anima_core::ItemView {
        anima_core::ItemView {
            serial,
            graphic,
            amount: 1,
            pos: Position {
                x: 100,
                y: 100,
                z: 0,
            },
            container,
            layer: 0,
            distance,
            is_multi: false,
        }
    }

    #[test]
    fn hunter_attacks_a_nearby_grey() {
        let mut b = HunterBrain::new();
        let mut o = obs_at(100, 100);
        o.player.serial = 7;
        o.mobiles.push(MobileView {
            serial: 9,
            name: "Brigand".into(),
            pos: Position {
                x: 101,
                y: 100,
                z: 0,
            },
            body: 0x190,
            notoriety: 3,
            hits: 10,
            hits_max: 10,
            distance: 1,
        });
        let acts = b.decide(&o);
        assert!(acts
            .iter()
            .any(|a| matches!(a, Action::WarMode { on: true })));
        assert!(acts
            .iter()
            .any(|a| matches!(a, Action::Attack { serial: 9 })));
    }

    #[test]
    fn hunter_bandages_when_hurt() {
        let mut b = HunterBrain::new();
        let mut o = obs_at(100, 100);
        o.player.serial = 7;
        o.player.hits = 10;
        o.player.hits_max = 40;
        o.items
            .push(item(0x4000_0001, BANDAGE_GRAPHIC, 0, Some(0x4000_0010)));
        assert_eq!(
            b.decide(&o),
            vec![Action::BandageTarget {
                bandage: 0x4000_0001,
                target: 7
            }]
        );
    }

    #[test]
    fn hunter_opens_then_loots_a_corpse() {
        let mut b = HunterBrain::new();
        let mut o = obs_at(100, 100);
        o.items.push(item(0x4000_00AA, CORPSE_GRAPHIC, 1, None));
        assert_eq!(
            b.decide(&o),
            vec![Action::Use {
                serial: 0x4000_00AA
            }]
        );
        o.items
            .push(item(0x4000_00BB, 0x0EED, 0, Some(0x4000_00AA)));
        assert_eq!(
            b.decide(&o),
            vec![Action::PickUp {
                serial: 0x4000_00BB,
                amount: 1
            }]
        );
    }

    #[test]
    fn llm_json_array_parses_contract_actions() {
        let acts = actions_from_json_body(
            r#"[{"type":"Walk","dir":2,"run":true},{"type":"WarMode","on":true}]"#,
        )
        .unwrap();
        assert_eq!(
            acts,
            vec![
                Action::Walk { dir: 2, run: true },
                Action::WarMode { on: true }
            ]
        );
    }
}

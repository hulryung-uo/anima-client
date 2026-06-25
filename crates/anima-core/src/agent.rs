//! The Observation/Action contract — the stable seam between the world and a
//! brain (AI or a human's input) or a renderer.
//!
//! - [`Observation`] is a read-only snapshot the brain consumes; build it with
//!   [`World::observe`].
//! - [`Action`] is a high-level intent the brain emits; a driver
//!   ([`anima-net`]'s `Session`) turns it into packets.
//!
//! Keeping this schema stable lets scripted / RL / LLM brains and the
//! native/WASM backends all plug into the same interface (see DESIGN.md §3).

use crate::types::Position;
use crate::world::{JournalEntry, TargetCursor, World};

/// A skill value, in human units (50.0 == GM-half). Derived from [`crate::world::Skill`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkillView {
    pub id: u16,
    pub value: f32,
    pub base: f32,
    pub cap: f32,
    pub lock: u8,
}

/// A read-only view of our own character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerView {
    pub serial: u32,
    pub name: String,
    pub pos: Position,
    pub direction: u8,
    pub hits: u16,
    pub hits_max: u16,
    pub mana: u16,
    pub mana_max: u16,
    pub stam: u16,
    pub stam_max: u16,
    pub strength: u16,
    pub dexterity: u16,
    pub intelligence: u16,
    pub gold: u32,
    pub weight: u16,
}

/// A nearby creature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileView {
    pub serial: u32,
    pub name: String,
    pub pos: Position,
    pub body: u16,
    pub notoriety: u8,
    pub hits: u16,
    pub hits_max: u16,
    /// Chebyshev distance from the player.
    pub distance: u32,
}

/// A nearby item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemView {
    pub serial: u32,
    pub graphic: u16,
    pub amount: u16,
    pub pos: Position,
    pub container: Option<u32>,
    /// Worn layer (0 if not equipped). 0x15 (21) = backpack.
    pub layer: u8,
    pub distance: u32,
}

/// A perception snapshot for the brain. Nearby lists are sorted by distance.
#[derive(Debug, Clone, Default)]
pub struct Observation {
    pub player: PlayerView,
    pub mobiles: Vec<MobileView>,
    pub items: Vec<ItemView>,
    /// Journal lines since the last observation (see [`World::observe`]).
    pub new_journal: Vec<JournalEntry>,
    /// Set when the server is waiting for us to pick a target (answer with
    /// [`Action::TargetObject`] / [`Action::TargetGround`]).
    pub pending_target: Option<TargetCursor>,
    /// Our skills, sorted by id (values in human units, e.g. 50.0).
    pub skills: Vec<SkillView>,
}

/// A decision-maker that turns perception into intent. Scripted, RL, or LLM
/// brains all implement this; a driver feeds it [`Observation`]s and executes the
/// [`Action`]s it returns. This is the top of the Interface⊥Brain split.
pub trait Brain {
    /// Decide what to do given the current perception. May return zero or more
    /// actions (typically one step + the occasional speech/use).
    fn decide(&mut self, obs: &Observation) -> Vec<Action>;
}

/// The UO direction (0..7) that heads from the player toward a `(dx, dy)` offset
/// (each component reduced to its sign). Returns `None` for a zero offset.
pub fn dir_toward(dx: i32, dy: i32) -> Option<u8> {
    use crate::net::movement::direction_delta;
    let sx = dx.signum();
    let sy = dy.signum();
    if (sx, sy) == (0, 0) {
        return None;
    }
    (0u8..8).find(|&d| direction_delta(d) == (sx, sy))
}

/// A high-level intent emitted by the brain. The driver executes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Step one tile in UO direction 0..7 (running optional).
    Walk { dir: u8, run: bool },
    /// Speak in-game.
    Say { text: String },
    /// Begin attacking a target.
    Attack { serial: u32 },
    /// Double-click ("use") an item or mobile.
    Use { serial: u32 },
    /// Single-click (request the name/label).
    Click { serial: u32 },
    /// Lift `amount` from a stack/item.
    PickUp { serial: u32, amount: u16 },
    /// Toggle war mode.
    WarMode { on: bool },
    /// Answer a pending target cursor by selecting an object/mobile.
    TargetObject { serial: u32 },
    /// Answer a pending target cursor by selecting a ground location.
    TargetGround { x: u16, y: u16, z: i16, graphic: u16 },
}

fn chebyshev(a: Position, b: Position) -> u32 {
    (a.x.abs_diff(b.x)).max(a.y.abs_diff(b.y)) as u32
}

impl World {
    /// Build an [`Observation`]. `journal_cursor` is the index into
    /// [`World::journal`] already seen; it is advanced to the current length and
    /// only newer lines are returned, so a brain sees each line once.
    pub fn observe(&self, journal_cursor: &mut usize) -> Observation {
        let pm = self.player_mobile().cloned().unwrap_or_default();
        let player = PlayerView {
            serial: pm.serial,
            name: pm.name.clone(),
            pos: pm.pos,
            direction: pm.direction,
            hits: pm.hits,
            hits_max: pm.hits_max,
            mana: pm.mana,
            mana_max: pm.mana_max,
            stam: pm.stam,
            stam_max: pm.stam_max,
            strength: self.player_stats.strength,
            dexterity: self.player_stats.dexterity,
            intelligence: self.player_stats.intelligence,
            gold: self.player_stats.gold,
            weight: self.player_stats.weight,
        };

        let mut mobiles: Vec<MobileView> = self
            .mobiles
            .values()
            .filter(|m| Some(m.serial) != self.player.map(|s| s.0))
            .map(|m| MobileView {
                serial: m.serial,
                name: m.name.clone(),
                pos: m.pos,
                body: m.body,
                notoriety: m.notoriety,
                hits: m.hits,
                hits_max: m.hits_max,
                distance: chebyshev(player.pos, m.pos),
            })
            .collect();
        mobiles.sort_by_key(|m| m.distance);

        let mut items: Vec<ItemView> = self
            .items
            .values()
            .map(|it| ItemView {
                serial: it.serial,
                graphic: it.graphic,
                amount: it.amount,
                pos: it.pos,
                container: it.container,
                layer: it.layer,
                distance: chebyshev(player.pos, it.pos),
            })
            .collect();
        items.sort_by_key(|it| it.distance);

        let start = (*journal_cursor).min(self.journal.len());
        let new_journal = self.journal[start..].to_vec();
        *journal_cursor = self.journal.len();

        let mut skills: Vec<SkillView> = self
            .skills
            .values()
            .map(|s| SkillView {
                id: s.id,
                value: s.value as f32 / 10.0,
                base: s.base as f32 / 10.0,
                cap: s.cap as f32 / 10.0,
                lock: s.lock,
            })
            .collect();
        skills.sort_by_key(|s| s.id);

        Observation {
            player,
            mobiles,
            items,
            new_journal,
            pending_target: self.pending_target,
            skills,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::login::LoginResult;

    #[test]
    fn observe_sorts_by_distance_and_advances_journal() {
        let mut w = World::new();
        w.enter_world(&LoginResult {
            serial: 0x311,
            x: 100,
            y: 100,
            z: 0,
            direction: 0,
            body: 0x190,
        });
        // Two mobiles at different distances.
        let far = w.mobile_mut(0xAA);
        far.pos = Position { x: 110, y: 100, z: 0 };
        let near = w.mobile_mut(0xBB);
        near.pos = Position { x: 102, y: 100, z: 0 };

        w.journal.push(JournalEntry {
            serial: 0,
            name: "System".into(),
            text: "hello".into(),
            msg_type: 0,
            hue: 0,
        });

        let mut cursor = 0;
        let obs = w.observe(&mut cursor);
        assert_eq!(obs.mobiles.len(), 2);
        assert_eq!(obs.mobiles[0].serial, 0xBB); // nearest first
        assert_eq!(obs.mobiles[0].distance, 2);
        assert_eq!(obs.new_journal.len(), 1);

        // A second observe with the advanced cursor sees no repeat lines.
        let obs2 = w.observe(&mut cursor);
        assert!(obs2.new_journal.is_empty());
    }
}

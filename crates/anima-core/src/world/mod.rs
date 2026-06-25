//! World model: the single source of truth for "what the game looks like now".
//!
//! Incoming packets mutate this (see [`crate::net::game`]); consumers (AI brain,
//! renderer) only read it. Mirrors ClassicUO's `World` — player plus mobiles and
//! items keyed by serial.

use std::collections::HashMap;

use crate::net::login::LoginResult;
use crate::types::{Position, Serial};

/// A creature/character (NPC or player) in the world.
#[derive(Debug, Clone, Default)]
pub struct Mobile {
    pub serial: u32,
    pub name: String,
    pub pos: Position,
    pub body: u16,
    /// Facing, low 3 bits (0..7).
    pub direction: u8,
    pub hue: u16,
    /// Notoriety byte (1=innocent, 3=gray/criminal, 6=murderer/red, ...).
    pub notoriety: u8,
    pub hits: u16,
    pub hits_max: u16,
    pub mana: u16,
    pub mana_max: u16,
    pub stam: u16,
    pub stam_max: u16,
}

/// An item — on the ground, in a container, or equipped.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub serial: u32,
    pub graphic: u16,
    pub amount: u16,
    pub pos: Position,
    /// Container serial, or `None` when lying on the ground.
    pub container: Option<u32>,
    /// Worn layer (0 when not equipped).
    pub layer: u8,
    pub hue: u16,
    pub name: String,
}

/// Self-only fields that don't live on the player's [`Mobile`].
#[derive(Debug, Clone, Default)]
pub struct PlayerStats {
    pub is_female: bool,
    pub strength: u16,
    pub dexterity: u16,
    pub intelligence: u16,
    pub gold: u32,
    pub armor: i16,
    pub weight: u16,
    pub weight_max: u16,
}

/// One journal (chat/system) line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub serial: u32,
    pub name: String,
    pub text: String,
    pub msg_type: u8,
    pub hue: u16,
}

/// One skill's standing. Values are in **tenths** (wire units): 500 == 50.0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skill {
    pub id: u16,
    /// Effective value (base + transient item/buff bonuses), tenths.
    pub value: u16,
    /// Trainable base — skill *progression* registers here, tenths.
    pub base: u16,
    /// Cap, tenths (default 1000 == 100.0).
    pub cap: u16,
    /// 0 = up, 1 = down, 2 = locked.
    pub lock: u8,
}

/// An outstanding target cursor the server is waiting on (from a 0x6C request).
/// The brain answers it with `Action::TargetObject`/`TargetGround`; the response
/// must echo `cursor_id`, `cursor_flag`, and `target_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetCursor {
    /// 0 = object/serial target, 1 = ground/location target.
    pub target_type: u8,
    /// Cursor id assigned by the server; echoed back in the response.
    pub cursor_id: u32,
    /// 0 neutral, 1 harmful, 2 helpful. (3 = cancel; never stored — it clears.)
    pub cursor_flag: u8,
}

/// The whole observable game state.
#[derive(Debug, Default)]
pub struct World {
    /// Our own character's serial, once we've entered the world.
    pub player: Option<Serial>,
    pub player_stats: PlayerStats,
    pub mobiles: HashMap<u32, Mobile>,
    pub items: HashMap<u32, Item>,
    /// Chat / system message log (journal), newest last.
    pub journal: Vec<JournalEntry>,
    /// Server-issued fast-walk prevention keys (FIFO), consumed one per step.
    pub fast_walk: Vec<u32>,
    /// An outstanding target cursor (0x6C), if the server is waiting on one.
    pub pending_target: Option<TargetCursor>,
    /// Our skills by id (0x3A), values in tenths. See [`Skill`].
    pub skills: HashMap<u16, Skill>,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Our own character, if we've entered the world and it's known.
    pub fn player_mobile(&self) -> Option<&Mobile> {
        self.player.and_then(|s| self.mobiles.get(&s.0))
    }

    pub fn player_mobile_mut(&mut self) -> Option<&mut Mobile> {
        match self.player {
            Some(s) => self.mobiles.get_mut(&s.0),
            None => None,
        }
    }

    /// Set the player and seed its mobile from a completed login.
    pub fn enter_world(&mut self, r: &LoginResult) {
        self.player = Some(Serial(r.serial));
        let m = self.mobiles.entry(r.serial).or_default();
        m.serial = r.serial;
        m.body = r.body;
        m.pos = Position {
            x: r.x,
            y: r.y,
            z: r.z,
        };
        m.direction = r.direction;
    }

    pub fn is_player(&self, serial: u32) -> bool {
        self.player == Some(Serial(serial))
    }

    /// Get-or-create a mobile by serial.
    pub fn mobile_mut(&mut self, serial: u32) -> &mut Mobile {
        let m = self.mobiles.entry(serial).or_default();
        m.serial = serial;
        m
    }

    /// Get-or-create an item by serial.
    pub fn item_mut(&mut self, serial: u32) -> &mut Item {
        let it = self.items.entry(serial).or_default();
        it.serial = serial;
        it
    }

    /// Remove an entity (mobile or item) by serial. Returns true if it was a mobile.
    pub fn remove(&mut self, serial: u32) -> bool {
        let was_mobile = self.mobiles.remove(&serial).is_some();
        self.items.remove(&serial);
        was_mobile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_resolves_from_serial() {
        let mut w = World::new();
        assert!(w.player_mobile().is_none());

        let me = Serial(0x0000_2A2A);
        w.mobile_mut(me.0).name = "Anima".into();
        w.player = Some(me);

        assert_eq!(w.player_mobile().unwrap().name, "Anima");
    }

    #[test]
    fn enter_world_seeds_player() {
        let mut w = World::new();
        w.enter_world(&LoginResult {
            serial: 0x311,
            x: 3503,
            y: 2574,
            z: 14,
            direction: 0,
            body: 0x0190,
        });
        assert!(w.is_player(0x311));
        let p = w.player_mobile().unwrap();
        assert_eq!(p.pos.x, 3503);
        assert_eq!(p.body, 0x0190);
    }
}

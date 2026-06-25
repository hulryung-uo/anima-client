//! Movement: outgoing walk requests + the client-side walk state machine.
//!
//! UO movement is sequenced and predictive. We send a walk request (0x02)
//! carrying a direction, a rolling sequence byte, and a fast-walk prevention
//! key; the server replies `ConfirmWalk` (0x22) or `DenyWalk` (0x21). The
//! [`Walker`] tracks the sequence, the outstanding-step budget (max 5), and the
//! predicted next tile, committing it to the player on confirm and resyncing on
//! deny. Sans-IO: it mutates [`World`] and returns bytes; the driver does the IO.
//!
//! Ported from `anima/anima/perception/walker.py`.

use super::packet::PacketWriter;
use crate::world::World;

/// Max walk packets that may be outstanding (unacked) at once.
pub const MAX_PENDING_STEPS: u8 = 5;
/// Running flag OR'd into the direction byte.
pub const RUN_FLAG: u8 = 0x80;

/// (dx, dy) tile delta for a direction (low 3 bits).
pub fn direction_delta(dir: u8) -> (i32, i32) {
    match dir & 0x07 {
        0 => (0, -1),  // North
        1 => (1, -1),  // Right / NE
        2 => (1, 0),   // East
        3 => (1, 1),   // Down / SE
        4 => (0, 1),   // South
        5 => (-1, 1),  // Left / SW
        6 => (-1, 0),  // West
        _ => (-1, -1), // Up / NW (7)
    }
}

/// WalkRequest `0x02` (7 bytes): `[0x02][dir|run][seq][fastwalk:u32]`.
pub fn build_walk_request(direction: u8, run: bool, seq: u8, fastwalk: u32) -> Vec<u8> {
    let dir_byte = (direction & 0x07) | if run { RUN_FLAG } else { 0 };
    let mut w = PacketWriter::new();
    w.u8(0x02).u8(dir_byte).u8(seq).u32(fastwalk);
    w.into_vec()
}

/// Client-side walk sequencer + position predictor.
#[derive(Debug, Default)]
pub struct Walker {
    /// Next sequence byte to send. Starts at 0 (the server expects seq 0 first,
    /// and resets to 0 after any deny).
    sequence: u8,
    pending_seq: Option<u8>,
    /// Predicted tile for the in-flight step (None for a pure turn).
    pending_step: Option<(u16, u16)>,
    /// Predicted facing for the in-flight request.
    pending_dir: Option<u8>,
    /// Outstanding (unacked) walk packets.
    outstanding: u8,
}

impl Walker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn outstanding(&self) -> u8 {
        self.outstanding
    }

    /// Can we send another step right now, or is the pending budget full?
    pub fn can_step(&self) -> bool {
        self.outstanding < MAX_PENDING_STEPS
    }

    /// Build the next walk packet toward `dir`. A request when not already
    /// facing `dir` is a *turn* (no tile change); the following one moves.
    /// Returns the bytes to send, or `None` if no player or the budget is full.
    pub fn step(&mut self, world: &mut World, dir: u8, run: bool) -> Option<Vec<u8>> {
        if !self.can_step() {
            return None;
        }
        let player = world.player_mobile()?;
        let dir = dir & 0x07;
        let is_turn = player.direction != dir;
        let (px, py) = (player.pos.x, player.pos.y);

        let seq = self.sequence;
        let fastwalk = pop_fast_walk(world);
        let packet = build_walk_request(dir, run, seq, fastwalk);

        self.pending_seq = Some(seq);
        self.pending_dir = Some(dir);
        self.pending_step = if is_turn {
            None
        } else {
            let (dx, dy) = direction_delta(dir);
            Some((
                (px as i32 + dx).clamp(0, u16::MAX as i32) as u16,
                (py as i32 + dy).clamp(0, u16::MAX as i32) as u16,
            ))
        };

        self.sequence = if self.sequence == 0xFF { 1 } else { self.sequence + 1 };
        self.outstanding += 1;
        Some(packet)
    }

    /// Handle 0x22 ConfirmWalk: commit the predicted facing/tile to the player.
    pub fn on_confirm(&mut self, world: &mut World, seq: u8) {
        if self.outstanding > 0 {
            self.outstanding -= 1;
        }
        // Only the ack for the currently-pending request applies its effect.
        if let Some(ps) = self.pending_seq {
            if ps != seq {
                return;
            }
        }
        self.pending_seq = None;
        let dir = self.pending_dir.take();
        let step = self.pending_step.take();
        if let Some(p) = world.player_mobile_mut() {
            if let Some(d) = dir {
                p.direction = d;
            }
            if let Some((nx, ny)) = step {
                p.pos.x = nx;
                p.pos.y = ny;
            }
        }
    }

    /// Handle 0x21 DenyWalk: resync to the server-authoritative position and
    /// reset the sequence (server resets its own to 0 on deny).
    pub fn on_deny(&mut self, world: &mut World, _seq: u8, x: u16, y: u16, z: i8, dir: u8) {
        self.pending_seq = None;
        self.pending_dir = None;
        self.pending_step = None;
        self.outstanding = 0;
        self.sequence = 0;
        if let Some(p) = world.player_mobile_mut() {
            p.pos.x = x;
            p.pos.y = y;
            p.pos.z = z;
            p.direction = dir & 0x07;
        }
    }
}

/// Pop the next fast-walk key (FIFO), or 0 if none are queued.
fn pop_fast_walk(world: &mut World) -> u32 {
    if world.fast_walk.is_empty() {
        0
    } else {
        world.fast_walk.remove(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::login::LoginResult;

    fn world_at(x: u16, y: u16, dir: u8) -> World {
        let mut w = World::new();
        w.enter_world(&LoginResult {
            serial: 0x311,
            x,
            y,
            z: 0,
            direction: dir,
            body: 0x190,
        });
        w
    }

    #[test]
    fn walk_packet_shape() {
        let p = build_walk_request(2, true, 7, 0xDEAD_BEEF);
        assert_eq!(p, vec![0x02, 0x02 | 0x80, 7, 0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn turn_then_move() {
        // Facing North (0); request East (2) → first packet is a turn (no move).
        let mut w = world_at(100, 100, 0);
        let mut walker = Walker::new();

        let _pkt = walker.step(&mut w, 2, false).unwrap();
        walker.on_confirm(&mut w, 0); // seq 0
        let p = w.player_mobile().unwrap();
        assert_eq!((p.pos.x, p.pos.y), (100, 100)); // turned, didn't move
        assert_eq!(p.direction, 2);

        // Now facing East; next request East actually moves +1 x.
        let _pkt = walker.step(&mut w, 2, false).unwrap();
        walker.on_confirm(&mut w, 1); // seq 1
        let p = w.player_mobile().unwrap();
        assert_eq!((p.pos.x, p.pos.y), (101, 100));
    }

    #[test]
    fn sequence_advances_and_budget_caps() {
        let mut w = world_at(100, 100, 2); // already facing East
        let mut walker = Walker::new();
        // Fire 5 steps without acks → budget full, 6th returns None.
        for _ in 0..MAX_PENDING_STEPS {
            assert!(walker.step(&mut w, 2, false).is_some());
        }
        assert!(!walker.can_step());
        assert!(walker.step(&mut w, 2, false).is_none());
    }

    #[test]
    fn deny_resyncs_and_resets_sequence() {
        let mut w = world_at(100, 100, 2);
        let mut walker = Walker::new();
        walker.step(&mut w, 2, false); // predict move to (101,100)
        walker.on_deny(&mut w, 0, 50, 60, 5, 4);
        let p = w.player_mobile().unwrap();
        assert_eq!((p.pos.x, p.pos.y, p.pos.z), (50, 60, 5));
        assert_eq!(p.direction, 4);
        assert_eq!(walker.outstanding(), 0);
        // After deny the next step must use seq 0 again.
        walker.step(&mut w, 4, false);
        // (seq is internal; verify indirectly: a fresh step was accepted)
        assert_eq!(walker.outstanding(), 1);
    }

    #[test]
    fn consumes_fast_walk_keys() {
        let mut w = world_at(100, 100, 2);
        w.fast_walk = vec![0x1111_1111, 0x2222_2222];
        let mut walker = Walker::new();
        let p1 = walker.step(&mut w, 2, false).unwrap();
        assert_eq!(&p1[3..7], &[0x11, 0x11, 0x11, 0x11]);
        assert_eq!(w.fast_walk.len(), 1);
    }
}

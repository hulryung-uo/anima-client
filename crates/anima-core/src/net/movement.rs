//! Movement: outgoing walk requests + the client-side walk state machine.
//!
//! UO movement is sequenced and predictive. We send a walk request (0x02)
//! carrying a direction, a rolling sequence byte, and a fast-walk prevention
//! key; the server replies `ConfirmWalk` (0x22) or `DenyWalk` (0x21).
//!
//! Faithful port of ClassicUO `WalkerManager` + `PlayerMobile.Walk` (the parts
//! relevant to a headless core): a 5-slot `StepInfos` ring tracks the predicted
//! steps in flight, each carrying its sequence; `ConfirmWalk` accepts the
//! matching step (committing its tile to [`World`]); an out-of-order / unknown
//! sequence triggers a **Resync** (`0x22`) + `WalkingFailed` (gating further
//! steps until a `DenyWalk` resyncs); `DenyWalk` clears the ring + `Reset`s.
//!
//! Pacing (one step per UO cadence) is done by the driver (the play-server), the
//! equivalent of ClassicUO's `LastStepRequestTime` gate.

use super::packet::PacketWriter;
use crate::world::World;

/// ClassicUO `Constants.MAX_STEP_COUNT` — queued/in-flight steps cap.
pub const MAX_STEP_COUNT: usize = 5;
/// Back-compat alias (old name).
pub const MAX_PENDING_STEPS: u8 = MAX_STEP_COUNT as u8;
/// Running flag OR'd into the direction byte.
pub const RUN_FLAG: u8 = 0x80;

/// Milliseconds to complete one step — ClassicUO `MovementSpeed.TimeToCompleteMovement`.
/// Mounted halves the time; running halves it again.
pub fn step_delay_ms(run: bool, mounted: bool) -> u64 {
    match (mounted, run) {
        (true, true) => 100,
        (true, false) => 200,
        (false, true) => 200,
        (false, false) => 400,
    }
}

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

/// Client Resync request (`0x22`, 3 bytes) — ClassicUO `Send_Resync`.
pub fn build_resync() -> Vec<u8> {
    vec![0x22, 0x00, 0x00]
}

/// One in-flight predicted step (ClassicUO `WalkerManager.StepInfo`, trimmed to
/// the fields our headless core commits on confirm).
#[derive(Debug, Clone, Copy, Default)]
struct StepInfo {
    sequence: u8,
    direction: u8,
    x: u16,
    y: u16,
    z: i8,
    /// True for a pure facing change (no tile move).
    turn: bool,
}

/// Client-side walk sequencer + position predictor (ClassicUO `WalkerManager`).
#[derive(Debug)]
pub struct Walker {
    /// Next sequence byte to send (`WalkSequence`). 0..255, FF→1, reset 0 on deny.
    walk_sequence: u8,
    /// Ring of predicted in-flight steps.
    steps: [StepInfo; MAX_STEP_COUNT],
    steps_count: usize,
    /// Unacked walk packets (`UnacceptedPacketsCount`).
    unaccepted: u8,
    /// A bad confirm desynced us; gate further steps until a deny resyncs.
    walking_failed: bool,
    /// We already queued a Resync for the current desync (don't spam).
    resend_resync: bool,
    /// Set when a Resync packet should be sent by the driver; cleared by `take_resync`.
    resync_out: bool,
}

impl Default for Walker {
    fn default() -> Self {
        Self {
            walk_sequence: 0,
            steps: [StepInfo::default(); MAX_STEP_COUNT],
            steps_count: 0,
            unaccepted: 0,
            walking_failed: false,
            resend_resync: false,
            resync_out: false,
        }
    }
}

impl Walker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Unacked packet count (kept for diagnostics / back-compat).
    pub fn outstanding(&self) -> u8 {
        self.unaccepted
    }

    /// ClassicUO `WalkingFailed` — a desync is awaiting a deny-resync.
    pub fn walking_failed(&self) -> bool {
        self.walking_failed
    }

    /// Can we queue another step right now? (ClassicUO Walk gate: not failed,
    /// queue not full.)
    pub fn can_step(&self) -> bool {
        !self.walking_failed && self.steps_count < MAX_STEP_COUNT
    }

    /// The tip of the prediction (last queued step, else the player's committed
    /// tile). ClassicUO `PlayerMobile.Walk` starts the next step from `Steps.Back()`.
    /// Returns (x, y, z, facing).
    pub fn tail(&self, world: &World) -> (u16, u16, i8, u8) {
        if self.steps_count > 0 {
            let s = &self.steps[self.steps_count - 1];
            (s.x, s.y, s.z, s.direction)
        } else if let Some(p) = world.player_mobile() {
            (p.pos.x, p.pos.y, p.pos.z, p.direction)
        } else {
            (0, 0, 0, 0)
        }
    }

    /// Build the next walk packet toward `dir`, queued from the prediction tip.
    /// A request when not already facing `dir` is a *turn* (no tile change); the
    /// following one moves. Returns the bytes to send, or `None` if blocked
    /// (failed/full/no player).
    pub fn step(&mut self, world: &mut World, dir: u8, run: bool) -> Option<Vec<u8>> {
        if !self.can_step() {
            return None;
        }
        world.player_mobile()?; // require a player
        let dir = dir & 0x07;
        let (bx, by, bz, facing) = self.tail(world);

        // turn-then-move: the first packet toward a new facing only turns.
        let is_turn = facing != dir;
        let (nx, ny) = if is_turn {
            (bx, by)
        } else {
            let (dx, dy) = direction_delta(dir);
            (
                (bx as i32 + dx).clamp(0, u16::MAX as i32) as u16,
                (by as i32 + dy).clamp(0, u16::MAX as i32) as u16,
            )
        };

        let seq = self.walk_sequence;
        let fastwalk = pop_fast_walk(world);
        let packet = build_walk_request(dir, run, seq, fastwalk);

        // Push the StepInfo (ClassicUO PlayerMobile.Walk → StepInfos[StepsCount]).
        self.steps[self.steps_count] = StepInfo {
            sequence: seq,
            direction: dir,
            x: nx,
            y: ny,
            z: bz,
            turn: is_turn,
        };
        self.steps_count += 1;
        self.unaccepted += 1;

        // Commit the facing *optimistically* (ClassicUO commits direction on send):
        // the scene/render must show the new facing immediately, and a paced
        // follow-up that arrives before the confirm must see it (tail() reads it).
        if let Some(p) = world.player_mobile_mut() {
            p.direction = dir;
        }

        // Advance WalkSequence (1..255, FF→1, never 0 again until a deny resets it).
        self.walk_sequence = if self.walk_sequence == 0xFF { 1 } else { self.walk_sequence + 1 };
        Some(packet)
    }

    /// Handle 0x22 ConfirmWalk for `seq`. In-order confirms accept the front step
    /// and commit its tile to the player (ClassicUO updates `RangeSize`; our
    /// headless core has no separate render, so the committed tile *is* the
    /// authoritative position the scene reports). An unknown/out-of-order seq is
    /// a bad step → Resync + `WalkingFailed` (ClassicUO `ConfirmWalk` isBadStep).
    pub fn on_confirm(&mut self, world: &mut World, seq: u8) {
        if self.unaccepted > 0 {
            self.unaccepted -= 1;
        }
        // Nothing pending → a stray/duplicate confirm; ignore it (resyncing here
        // would be harmful). Real desync detection only applies while we have
        // steps in flight.
        if self.steps_count == 0 {
            return;
        }
        // TCP delivers confirms in order, so the seq must match the front step.
        // Anything else is a desync (ClassicUO ConfirmWalk isBadStep).
        if self.steps[0].sequence != seq {
            return self.bad_step();
        }
        // Accept + commit the front step's tile, then shift the ring down.
        let s = self.steps[0];
        if let Some(p) = world.player_mobile_mut() {
            p.direction = s.direction;
            if !s.turn {
                p.pos.x = s.x;
                p.pos.y = s.y;
                // Z is resolved authoritatively by the driver (CalculateNewZ) after
                // the move; keep the predicted Z as a baseline.
                p.pos.z = s.z;
            }
        }
        for i in 1..self.steps_count {
            self.steps[i - 1] = self.steps[i];
        }
        self.steps_count -= 1;
        // A clean confirm clears any prior resync latch.
        self.resend_resync = false;
    }

    /// ConfirmWalk isBadStep: request a Resync once and fail walking until a deny.
    fn bad_step(&mut self) {
        if !self.resend_resync {
            self.resync_out = true;
            self.resend_resync = true;
        }
        self.walking_failed = true;
        self.steps_count = 0;
    }

    /// Handle 0x21 DenyWalk: clear the queue, reset the sequencer, and teleport to
    /// the server-authoritative position (ClassicUO `DenyWalk` → `ClearSteps` + `Reset`).
    pub fn on_deny(&mut self, world: &mut World, _seq: u8, x: u16, y: u16, z: i8, dir: u8) {
        self.reset();
        if let Some(p) = world.player_mobile_mut() {
            p.pos.x = x;
            p.pos.y = y;
            p.pos.z = z;
            p.direction = dir & 0x07;
        }
    }

    /// ClassicUO `WalkerManager.Reset`. Also call this after a server-initiated
    /// **teleport** (0x20 MobileUpdate for the player): the in-flight sequence and
    /// pending steps are stale, so further walk requests would be denied until a
    /// resync. Sequence restarts at 0 (UO's post-reset first-step value).
    pub fn reset(&mut self) {
        self.steps_count = 0;
        self.unaccepted = 0;
        self.walk_sequence = 0;
        self.walking_failed = false;
        self.resend_resync = false;
        self.resync_out = false;
    }

    /// Take a pending Resync packet to send, if any (driver pumps this each loop).
    pub fn take_resync(&mut self) -> Option<Vec<u8>> {
        if self.resync_out {
            self.resync_out = false;
            Some(build_resync())
        } else {
            None
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
            aos: false,
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
        for _ in 0..MAX_STEP_COUNT {
            assert!(walker.step(&mut w, 2, false).is_some());
        }
        assert!(!walker.can_step());
        assert!(walker.step(&mut w, 2, false).is_none());
    }

    #[test]
    fn queue_predicts_from_tail() {
        // Multiple steps in flight predict from the queue tail, not the committed
        // (still-unconfirmed) player tile.
        let mut w = world_at(100, 100, 2); // facing East
        let mut walker = Walker::new();
        walker.step(&mut w, 2, false); // queues (101,100) seq0
        walker.step(&mut w, 2, false); // queues (102,100) seq1 (from tail, not 100)
        assert_eq!(walker.tail(&w), (102, 100, 0, 2));
        walker.on_confirm(&mut w, 0);
        assert_eq!(w.player_mobile().unwrap().pos.x, 101);
        walker.on_confirm(&mut w, 1);
        assert_eq!(w.player_mobile().unwrap().pos.x, 102);
    }

    #[test]
    fn bad_confirm_triggers_resync_and_walking_failed() {
        let mut w = world_at(100, 100, 2);
        let mut walker = Walker::new();
        walker.step(&mut w, 2, false); // seq 0 pending
        walker.on_confirm(&mut w, 42); // unknown seq → bad
        assert!(walker.walking_failed());
        assert!(walker.take_resync().is_some());
        assert!(walker.take_resync().is_none()); // only once
        assert!(!walker.can_step()); // gated until deny
        walker.on_deny(&mut w, 0, 50, 60, 0, 4);
        assert!(!walker.walking_failed());
        assert!(walker.can_step());
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
        walker.step(&mut w, 4, false);
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

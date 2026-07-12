//! Native TCP driver for `anima-core`'s sans-IO protocol.
//!
//! `anima-core` knows the UO protocol but never touches a socket; this crate
//! provides the blocking `std::net` loop that feeds bytes in and writes bytes
//! out, driving the [`LoginMachine`] to completion and then maintaining a live
//! [`World`] from the server's game-packet stream. The browser build will have
//! an analogous WebSocket driver; the core stays identical.

use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub mod json;
pub mod play_server;
pub mod regions;
pub mod scene;

use anima_assets::MapData;
use anima_core::agent::{Action, Observation};
use anima_core::net::outgoing::{
    build_attack, build_book_page_request, build_buy, build_cast_spell, build_double_click,
    build_drop, build_equip, build_gump_response, build_opl_request, build_party_accept,
    build_party_decline, build_party_invite, build_party_leave, build_party_message,
    build_pick_up,
    build_popup_request, build_popup_select, build_prompt_response, build_say, build_sell,
    build_single_click, build_skill_lock, build_status_request, build_target_response,
    build_trade_accept, build_trade_cancel, build_trade_gold, build_unicode_say, build_use_ability,
    build_use_skill, build_war_mode,
};
use anima_core::net::{
    apply_packet, build_client_version, FramingError, LoginConfig, LoginDirective, LoginError,
    LoginMachine, LoginResult, StreamDecoder, Walker,
};
use anima_core::path::{find_path, find_path_near, Terrain, DEFAULT_MAX_EXPANSIONS};
use anima_core::world::World;

// `DOOR_USE_COOLDOWN`/`MAX_DOOR_OPEN_ATTEMPTS` are only referenced by
// `route_tests` below (production code only needs `decide_blocked_step` to
// already have them baked in) — imported there, not here, so a non-test
// build doesn't warn about unused imports.
use crate::scene::{decide_blocked_step, BlockedStepAction, MapTerrain};

/// Client version we report to the server (must match the login seed version).
const CLIENT_VERSION: &str = "7.0.102.3";

/// A UO server address.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}

#[derive(Debug)]
pub enum DriverError {
    Io(std::io::Error),
    Framing(FramingError),
    Login(LoginError),
    /// Server closed the connection before login finished.
    ConnectionClosed,
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::Io(e) => write!(f, "io error: {e}"),
            DriverError::Framing(e) => write!(f, "framing error: {e:?}"),
            DriverError::Login(e) => write!(f, "login error: {e:?}"),
            DriverError::ConnectionClosed => write!(f, "connection closed by server"),
        }
    }
}

impl std::error::Error for DriverError {}

impl From<std::io::Error> for DriverError {
    fn from(e: std::io::Error) -> Self {
        DriverError::Io(e)
    }
}

const CONNECT_READ_TIMEOUT: Duration = Duration::from_secs(20);
// Short so the game loop ticks fast (like ClassicUO's per-frame loop): the
// movement *pacing* gate (run 200ms / walk 400ms) is only as precise as how
// often the loop checks it. A long read timeout stalls the loop on the socket
// when no packet is arriving, which throttled running down to walk speed.
const PUMP_READ_TIMEOUT: Duration = Duration::from_millis(20);

/// [`Action::WalkTo`] step cadence, mirroring the play-server's own
/// click-to-walk pacing (`anima-net/src/bin/play.rs`'s `AUTO_WALK_STEP_MS`):
/// ClassicUO's unmounted-walk step is 400ms.
const ROUTE_STEP: Duration = Duration::from_millis(400);
/// Give up a route after this many issued steps (runaway guard, mirrors
/// `play.rs`'s `AUTO_WALK_MAX_STEPS`).
const ROUTE_MAX_STEPS: u32 = 200;
/// Like `play_server`'s `WALKTO_GOAL_SLOP`: a route whose *exact* goal tile
/// turns out unreachable (a wall decoration, a tree, a crate someone dropped
/// on it) still resolves to the nearest reachable tile within this many
/// Chebyshev tiles instead of giving up outright — see
/// `anima_core::path::find_path_near`'s doc.
const ROUTE_GOAL_SLOP: u32 = 2;

/// What [`Route::advance`] wants the caller ([`Session::advance_route`]) to do.
/// Kept separate from the actual packet send so the state machine itself is
/// network-free and unit-testable with a stubbed [`Terrain`].
#[derive(Debug, PartialEq, Eq)]
enum RouteStep {
    /// Cadence hasn't elapsed since the last step attempt — do nothing.
    Wait,
    /// Walk one step in this direction next.
    Walk(u8),
    /// The next hop is a closed door (see [`Terrain::door_at`]) — send `Use`
    /// on this serial instead of walking into it. Unlike `Walk`, the caller
    /// doesn't need to report back whether the packet actually landed (a
    /// `Use` has no `Walker`-style pending-step budget to gate it) — `advance`
    /// itself owns all the open/await/give-up bookkeeping (see
    /// [`Route::door_attempts`]).
    OpenDoor(u32),
    /// The goal is reached, or no path remains given what we've learned —
    /// drop the route.
    Done,
}

/// [`Action::WalkTo`] (click-to-walk) bookkeeping for the headless driver —
/// the non-blocking analogue of [`Session::navigate_to`]. Mirrors the
/// play-server's own click-to-walk loop (`anima-net/src/bin/play.rs`, its
/// `auto_goal`/`auto_blocked`/… locals), just packaged as a struct so
/// [`Session::advance_route`] can drive it one tick at a time instead of
/// owning a bespoke loop. Deliberately network-free (no `Session`/socket
/// access) so [`Route::advance`] is unit-testable with a stubbed [`Terrain`].
/// A step `advance` has proposed but not yet confirmed sent. [`Route::step_sent`]
/// only promotes this into the armed `pending_move`/`from`/`target` fields when
/// the packet actually reached the wire — mirrors `play.rs`'s `auto_pending_move`,
/// which is likewise only set inside `if session.walk(sd, false).unwrap_or(false)`.
/// A gated attempt (the movement-prediction budget was exhausted, or
/// `walking_failed` latched) must be dropped instead of armed, or the *next*
/// `advance` would mistake "we never sent this" for "the server denied this"
/// and wrongly blacklist a tile that was never attempted.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    from: (u16, u16),
    target: (u32, u32),
    is_move: bool,
}

/// Per-tile door-open retry bookkeeping for [`Route::advance`] — a lighter
/// mirror of `scene::DoorUseAttempt` (attempt count + when the last `Use` was
/// sent), minus the door's own graphic: `Route` never sees a live [`World`],
/// so it has no way to tell "this `Use` already landed and toggled the door"
/// the way `play_server`'s executor can (see `decide_blocked_step`'s
/// `door_state_changed` doc) — it always takes that function's cooldown-only
/// path instead. Safe (just occasionally a little slower to react to a door
/// that already reopened than `play_server`'s human-facing loop would be).
#[derive(Debug, Clone, Copy)]
struct DoorAttempt {
    count: u32,
    sent_at: Instant,
}

#[derive(Debug)]
struct Route {
    goal: (u32, u32),
    /// Tiles the server has *denied* (static map said walkable, a
    /// building/dynamic blocker disagreed) — re-paths route around them, like
    /// `navigate_to`'s `Avoiding`. Also gains a tile whose closed door never
    /// opened after [`MAX_DOOR_OPEN_ATTEMPTS`] tries (see `advance`'s
    /// door-handling arm) — treated like any other wall from then on.
    blocked: HashSet<(u32, u32)>,
    /// Steps successfully issued so far (the runaway guard). Only real walks
    /// count — a door `Use` attempt does not (mirrors `play_server`'s
    /// `auto_steps`, which likewise only increments on an actual walk send).
    steps: u32,
    last_step: Instant,
    /// Whether the last *armed* (successfully sent) step was a real move (not
    /// a turn) and, if so, where we were and which tile we aimed for — lets
    /// the next `advance` detect a server deny (the tile didn't change) and
    /// blacklist it. Only [`Route::step_sent`] arms these, from `candidate`.
    pending_move: bool,
    from: (u16, u16),
    target: (u32, u32),
    /// `advance`'s most recent proposed step, awaiting `step_sent` to say
    /// whether it actually went out. See [`Candidate`].
    candidate: Option<Candidate>,
    /// Closed doors currently blocking the route's next hop, keyed by tile —
    /// see [`DoorAttempt`] and `advance`'s door-handling arm.
    door_attempts: HashMap<(u32, u32), DoorAttempt>,
}

impl Route {
    fn new(gx: u32, gy: u32) -> Self {
        Route {
            goal: (gx, gy),
            blocked: HashSet::new(),
            steps: 0,
            // Already "due" so the very first `advance` after a `WalkTo` steps
            // immediately instead of waiting a full cadence.
            last_step: Instant::now() - ROUTE_STEP,
            pending_move: false,
            from: (0, 0),
            target: (0, 0),
            candidate: None,
            door_attempts: HashMap::new(),
        }
    }

    /// Decide the next move given the current player pose. Does not touch the
    /// network or mutate `steps`/`last_step` for a [`RouteStep::Walk`] — the
    /// caller reports back via [`Route::step_sent`] once it knows whether the
    /// packet actually went out (a zero movement-prediction budget can mean it
    /// didn't); only then does `step_sent` arm the deny-detection bookkeeping
    /// below (see [`Candidate`]). A [`RouteStep::OpenDoor`]/an internally
    /// abandoned door tile, by contrast, is fully decided here — see
    /// [`RouteStep::OpenDoor`]'s doc.
    ///
    /// Uses [`find_path_near`] (not the exact-goal-only `find_path`) so a
    /// goal whose precise tile isn't reachable still resolves to the nearest
    /// standable tile within [`ROUTE_GOAL_SLOP`] instead of hard-rejecting —
    /// ClassicUO parity, mirroring `play_server`'s own `WalkTo` handling (see
    /// `find_path_near`'s doc). `terrain`'s [`Terrain::door_at`] — a no-op
    /// default for a plain grid/`MapData`, real for `scene::MapTerrain` —
    /// is what actually gives this door awareness; the pathfinding itself
    /// doesn't need to know about doors (planning already treats a closed
    /// one as passable).
    fn advance<T: Terrain>(&mut self, terrain: &mut T, pos: (u16, u16, i8), facing: u8) -> RouteStep {
        let (px, py, pz) = pos;
        if (px as u32, py as u32) == self.goal {
            return RouteStep::Done;
        }
        if self.last_step.elapsed() < ROUTE_STEP {
            return RouteStep::Wait;
        }
        // Did the previously *armed* move land? If our tile didn't change, the
        // server denied that tile — blacklist it so the re-path detours (mirrors
        // `navigate_to`/`play.rs`'s own deny detection).
        if self.pending_move && (px, py) == self.from {
            self.blocked.insert(self.target);
        }
        self.pending_move = false;

        // Loops only on `BlockedStepAction::Blacklist` (a door that gave up):
        // that permanently adds one more tile to `self.blocked` before
        // re-pathing, so this terminates — bounded by the (finite) number of
        // distinct door tiles any candidate route could ever offer up.
        loop {
            let resolved = {
                let mut avoid = Avoiding { inner: terrain, blocked: &self.blocked };
                find_path_near(
                    &mut avoid,
                    (px as u32, py as u32, pz as i32),
                    self.goal,
                    ROUTE_GOAL_SLOP,
                    DEFAULT_MAX_EXPANSIONS,
                )
            };
            let Some((_resolved_goal, steps)) = resolved else {
                return RouteStep::Done; // nothing reachable at all, even nearby — give up
            };
            if steps.is_empty() {
                // Already standing at the nearest reachable tile — a legitimate
                // "arrived" (mirrors `find_path_near`'s empty-path semantics), not
                // a failure just because the exact goal itself is unstandable.
                return RouteStep::Done;
            }
            let step = steps[0];
            let tile = (step.x, step.y);

            // Is the chosen next hop a closed door right now? Planning already
            // treats it as passable (see `Terrain::door_at`'s doc), so negotiate
            // actually opening it instead of walking into what the real server
            // would just deny.
            if let Some(serial) = terrain.door_at(step.x, step.y, pz as i32) {
                let prior = self.door_attempts.get(&tile).copied();
                let attempts = prior.map_or(0, |a| a.count);
                let sent_at = prior.map(|a| a.sent_at);
                // `door_state_changed` is always `false` here — see
                // `DoorAttempt`'s doc for why `Route` can't tell any better.
                let action = decide_blocked_step(Some(serial), attempts, sent_at, false, Instant::now());
                match action {
                    BlockedStepAction::OpenDoor(serial) => {
                        self.door_attempts.insert(tile, DoorAttempt { count: attempts + 1, sent_at: Instant::now() });
                        // Consumes this tick's cadence, like a real walk attempt.
                        self.last_step = Instant::now();
                        return RouteStep::OpenDoor(serial);
                    }
                    BlockedStepAction::AwaitDoor => {
                        self.last_step = Instant::now();
                        return RouteStep::Wait;
                    }
                    BlockedStepAction::Blacklist => {
                        // Not a decision worth reporting to the caller — prune this
                        // dead-end tile and immediately re-path around it, same as
                        // if the server had denied it (see the `pending_move` check
                        // above). No cadence cost: whatever this loop lands on next
                        // (a detour, or truly `Done`) is this tick's real answer.
                        self.blocked.insert(tile);
                        self.door_attempts.remove(&tile);
                        continue;
                    }
                }
            }
            // No longer (or never) blocked by a door here — drop any stale
            // bookkeeping (harmless no-op if absent).
            self.door_attempts.remove(&tile);

            // Not armed yet — just proposed. `step_sent` decides whether this
            // becomes the next `advance`'s deny check.
            self.candidate = Some(Candidate {
                from: (px, py),
                target: (step.x, step.y),
                is_move: facing == step.dir,
            });
            return RouteStep::Walk(step.dir);
        }
    }

    /// Record that `advance`'s proposed step attempt is done for this tick —
    /// always resets the cadence clock (mirrors `play.rs`, which paces on
    /// attempts, not just successful sends). Only `sent` arms the pending
    /// candidate (see [`Candidate`]) into `pending_move`/`from`/`target` and
    /// counts it toward the runaway guard; a gated send (`false`) discards the
    /// candidate, so a tile that was never attempted can't be blacklisted as if
    /// the server had denied it — the route just retries the same tile once
    /// next due.
    fn step_sent(&mut self, sent: bool) {
        self.last_step = Instant::now();
        if sent {
            self.steps += 1;
            if let Some(c) = self.candidate.take() {
                self.from = c.from;
                self.target = c.target;
                self.pending_move = c.is_move;
            }
        } else {
            self.candidate = None;
        }
    }
}

/// A live connection to a UO server: the game-phase socket plus the world state
/// it feeds.
pub struct Session {
    stream: TcpStream,
    decoder: StreamDecoder,
    walker: Walker,
    journal_cursor: usize,
    pub world: World,
    pub confirms: u32,
    pub denies: u32,
    /// The active [`Action::WalkTo`] route, if any — see [`Session::advance_route`].
    route: Option<Route>,
}

impl Session {
    /// Connect, run the full two-phase login handshake, enter the world, and
    /// return a session whose [`World`] is seeded with the login result.
    pub fn connect_and_login(endpoint: &Endpoint, cfg: LoginConfig) -> Result<Session, DriverError> {
        let (result, stream, decoder) = login(endpoint, cfg)?;
        let mut world = World::new();
        world.enter_world(&result);
        stream.set_read_timeout(Some(PUMP_READ_TIMEOUT)).ok();
        let mut session = Session {
            stream,
            decoder,
            walker: Walker::new(),
            journal_cursor: 0,
            world,
            confirms: 0,
            denies: 0,
            route: None,
        };
        // ServUO doesn't push our stats/skills unsolicited — request them so the
        // first Observation carries them (ClassicUO does the same on login).
        session.send(&build_status_request(4, result.serial))?; // stats (0x11)
        session.send(&build_status_request(5, result.serial))?; // skills (0x3A)
        Ok(session)
    }

    /// Build a perception [`Observation`] for a brain (advances the journal cursor
    /// so each line is seen once).
    pub fn observation(&mut self) -> Observation {
        self.world.observe(&mut self.journal_cursor)
    }

    /// Execute a high-level [`Action`] from a brain.
    pub fn apply_action(&mut self, action: &Action) -> Result<(), DriverError> {
        match action {
            Action::Walk { dir, run } => {
                // A manual step cancels any active auto-walk route (mirrors
                // play.rs's manual-key handling).
                self.route = None;
                self.walk(*dir, *run)?;
            }
            // ASCII stays on the classic 0x03 path; anything else (Korean/한글…)
            // goes out as UNICODE 0xAD so it isn't mangled to '?'.
            Action::Say { text } => {
                if text.is_ascii() {
                    self.send(&build_say(text, 0, 0x0034, 3))?
                } else {
                    self.send(&build_unicode_say(text, 0, 0x0034, 3))?
                }
            }
            Action::PartySay { text } => self.send(&build_party_message(text))?,
            Action::Attack { serial } => {
                self.world.last_attack = Some(*serial);
                self.send(&build_attack(*serial))?
            }
            // Pick the best target from the world (last target if still a live
            // in-view hostile, else nearest in-view hostile) and attack it.
            Action::AutoAttack => {
                if let Some(serial) = self.world.auto_attack_target() {
                    self.world.last_attack = Some(serial);
                    self.send(&build_attack(serial))?;
                }
            }
            // Re-attack the remembered last target (no-op if none yet).
            Action::AttackLast => {
                if let Some(serial) = self.world.last_attack {
                    self.send(&build_attack(serial))?;
                }
            }
            Action::Use { serial } => self.send(&build_double_click(*serial))?,
            Action::Click { serial } => self.send(&build_single_click(*serial))?,
            Action::PickUp { serial, amount } => self.send(&build_pick_up(*serial, *amount))?,
            Action::Drop { serial, x, y, z, container } => {
                self.send(&build_drop(*serial, *x, *y, *z, *container))?
            }
            Action::Equip { serial, layer } => {
                let mobile = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.send(&build_equip(*serial, *layer, mobile))?
            }
            Action::WarMode { on } => self.send(&build_war_mode(*on))?,
            Action::CastSpell { spell } => self.send(&build_cast_spell(*spell))?,
            Action::TargetObject { serial } => self.respond_target(Some(*serial), 0, 0, 0, 0)?,
            Action::TargetGround { x, y, z, graphic } => {
                self.respond_target(None, *x, *y, *z, *graphic)?
            }
            Action::TargetCancel => self.cancel_target()?,
            Action::BuyItems { vendor, items } => self.send(&build_buy(*vendor, items))?,
            Action::SellItems { vendor, items } => {
                self.send(&build_sell(*vendor, items))?;
                // The sell list is consumed once we answer it — clear it
                // locally so a later, unrelated sell trip can't accidentally
                // re-answer this stale list (mirrors PopupSelect clearing
                // world.popup below).
                self.world.close_shop_sell();
            }
            Action::GumpResponse { serial, gump_id, button, switches, entries } => {
                self.send(&build_gump_response(*serial, *gump_id, *button, switches, entries))?;
                // The gump is consumed once we answer it — drop it from the world so
                // the renderer/brain stop seeing a stale dialog.
                self.world.close_gump(*serial);
            }
            Action::PopupRequest { serial } => self.send(&build_popup_request(*serial))?,
            Action::PopupSelect { serial, index } => {
                self.send(&build_popup_select(*serial, *index))?;
                // The menu is consumed once we pick — clear it locally so the
                // renderer/brain stop seeing a stale popup.
                self.world.popup = None;
            }
            Action::BookRequest { serial, pages } => {
                self.send(&build_book_page_request(*serial, *pages))?;
            }
            Action::UseAbility { ability } => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.send(&build_use_ability(serial, *ability))?;
            }
            Action::SkillLock { skill, lock } => {
                self.send(&build_skill_lock(*skill, *lock))?;
                // Optimistically reflect the new lock locally so the UI updates
                // immediately (the server also echoes a 0x3A single update).
                if let Some(s) = self.world.skills.get_mut(skill) {
                    s.lock = *lock;
                }
            }
            Action::UseSkill { skill } => self.send(&build_use_skill(*skill))?,
            Action::OplRequest { serial } => self.send(&build_opl_request(&[*serial]))?,
            Action::PartyInvite => self.send(&build_party_invite())?,
            Action::PartyAccept { leader } => {
                // leader 0 = "the pending inviter" (the UI may omit the serial).
                let leader = if *leader != 0 { *leader } else { self.world.party.pending_invite.unwrap_or(0) };
                self.send(&build_party_accept(leader))?;
                // We answered the invite — drop it locally so the prompt clears even
                // before the server's member-list update lands.
                self.world.party.pending_invite = None;
            }
            Action::PartyDecline { leader } => {
                let leader = if *leader != 0 { *leader } else { self.world.party.pending_invite.unwrap_or(0) };
                self.send(&build_party_decline(leader))?;
                self.world.party.pending_invite = None;
            }
            Action::PartyLeave => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.send(&build_party_leave(serial))?;
            }
            Action::PromptResponse { text } => self.respond_prompt(text, false)?,
            Action::PromptCancel => self.respond_prompt("", true)?,
            // No-op if no session has `container` (the brain raced it away — it
            // may have just closed, or belongs to a session with a different
            // opponent that never existed on our side).
            Action::TradeAccept { container, accept } => {
                if self.world.trades.iter().any(|t| t.my_container == *container) {
                    self.send(&build_trade_accept(*container, *accept))?;
                    // Optimistically reflect our own accept state locally (mirrors
                    // `SkillLock`'s optimistic update) — the server also echoes a
                    // 0x6F action-2 Update.
                    if let Some(t) = self.world.trade_mut(*container) {
                        t.my_accept = *accept;
                    }
                }
            }
            Action::TradeCancel { container } => {
                if self.world.trades.iter().any(|t| t.my_container == *container) {
                    self.send(&build_trade_cancel(*container))?;
                    // The trade is over the moment we cancel — drop just this
                    // session locally (and purge its leftover container contents,
                    // see `World::close_trade`) so the renderer/brain stop seeing a
                    // stale session; the server's own 0x6F Close echo would
                    // otherwise lag a poll behind. Other concurrent sessions (a
                    // different opponent) are untouched.
                    self.world.close_trade(*container);
                }
            }
            Action::TradeGold { container, gold, platinum } => {
                if self.world.trades.iter().any(|t| t.my_container == *container) {
                    self.send(&build_trade_gold(*container, *gold, *platinum))?;
                    if let Some(t) = self.world.trade_mut(*container) {
                        t.my_offer_gold = *gold;
                        t.my_offer_platinum = *platinum;
                    }
                }
            }
            // Start (or replace) a non-blocking auto-walk route. This only
            // records the goal — [`Session::advance_route`] (called once per
            // tick by a runner that owns the terrain/map, e.g. `anima-agent`'s
            // loop) does the actual pathfinding + pacing; the play-server's own
            // HTTP loop instead intercepts `WalkTo` before it reaches here and
            // paces its own equivalent `auto_goal`.
            Action::WalkTo { x, y } => {
                self.route = Some(Route::new(*x as u32, *y as u32));
            }
        }
        Ok(())
    }

    /// Advance the active [`Action::WalkTo`] route by at most one step, paced
    /// at [`ROUTE_STEP`] — call this once per tick (e.g. right after
    /// [`Session::observe`]) so a headless brain's `WalkTo` actually walks. A
    /// no-op if no route is active. `map` is the runner-owned static map data;
    /// this builds a [`scene::MapTerrain`] over it *and* `self.world` — the
    /// SAME door/dynamic-item-aware planning oracle `play_server`'s
    /// click-to-walk executor uses (see its doc), so a route can path through
    /// a closed door and, via `route.advance`'s [`RouteStep::OpenDoor`] arm
    /// below, actually open it on approach — not just the play-server's
    /// human-facing loop. Re-paths around a server deny, mirrors
    /// [`Session::navigate_to`]'s `Avoiding` for a tile the static map says is
    /// walkable but the server disagreed with (layered on top of, not
    /// instead of, `MapTerrain`'s own blacklist parameter, which is left
    /// empty here — `Route` owns the one blacklist that matters).
    pub fn advance_route(&mut self, map: &mut MapData) -> Result<(), DriverError> {
        let Some(mut route) = self.route.take() else { return Ok(()) };
        let Some(p) = self.world.player_mobile() else {
            self.route = Some(route); // not in the world yet — try again next tick
            return Ok(());
        };
        let pos = (p.pos.x, p.pos.y, p.pos.z);
        let facing = p.direction;
        // Scoped so `terrain`'s borrow of `self.world` ends before the match
        // arms below need `&mut self` (e.g. `self.walk`/`self.apply_action`).
        let step = {
            let empty = HashSet::new();
            let mut terrain = MapTerrain { world: &self.world, map, blocked: &empty, multis: None };
            route.advance(&mut terrain, pos, facing)
        };
        match step {
            RouteStep::Wait => self.route = Some(route),
            RouteStep::Done => {} // arrived, or no path left — drop the route
            RouteStep::Walk(dir) => {
                let sent = self.walk(dir, false)?;
                route.step_sent(sent);
                if route.steps <= ROUTE_MAX_STEPS {
                    self.route = Some(route);
                }
            }
            RouteStep::OpenDoor(serial) => {
                // Mirrors `play_server`'s `BlockedStepAction::OpenDoor` arm: a
                // closed door on the next hop — `Use` it instead of walking
                // into what the real server would just deny. `route.advance`
                // already owns the attempt/cooldown bookkeeping, so this is a
                // fire-and-forget send; the route stays live either way.
                if std::env::var("ANIMA_DEBUG").is_ok() {
                    eprintln!("anima-net: route to {:?} opening door {serial:#x}", route.goal);
                }
                self.apply_action(&Action::Use { serial })?;
                self.route = Some(route);
            }
        }
        Ok(())
    }

    /// Answer the pending target cursor (if any) and clear it. `serial = Some` is
    /// an object target (type 0); `None` is a ground target (type 1). No-ops when
    /// nothing is targeting (the brain raced the cursor away).
    fn respond_target(
        &mut self,
        serial: Option<u32>,
        x: u16,
        y: u16,
        z: i16,
        graphic: u16,
    ) -> Result<(), DriverError> {
        let Some(cursor) = self.world.pending_target else {
            return Ok(());
        };
        let (target_type, serial) = match serial {
            Some(s) => (0u8, s),
            None => (1u8, 0u32),
        };
        let pkt = build_target_response(
            target_type,
            cursor.cursor_id,
            cursor.cursor_flag,
            serial,
            x,
            y,
            z,
            graphic,
        );
        self.send(&pkt)?;
        self.world.pending_target = None;
        Ok(())
    }

    /// Cancel a pending target cursor (Esc). UO signals a cancel by echoing the
    /// cursor with serial 0 and an all-`0xFFFF` location; the server then aborts the
    /// spell/skill that was waiting for a target instead of staying in target mode.
    fn cancel_target(&mut self) -> Result<(), DriverError> {
        let Some(cursor) = self.world.pending_target else {
            return Ok(());
        };
        let pkt = build_target_response(
            cursor.target_type,
            cursor.cursor_id,
            cursor.cursor_flag,
            0,
            0xFFFF,
            0xFFFF,
            0,
            0,
        );
        self.send(&pkt)?;
        self.world.pending_target = None;
        Ok(())
    }

    /// Answer (or cancel) the pending server text prompt (0xC2 UnicodePrompt),
    /// echoing its `sender_serial`/`prompt_id`, and clear it locally. No-op when
    /// nothing is pending (the brain raced the prompt away).
    fn respond_prompt(&mut self, text: &str, cancel: bool) -> Result<(), DriverError> {
        let Some(p) = self.world.prompt else {
            return Ok(());
        };
        let pkt = build_prompt_response(p.sender_serial, p.prompt_id, text, cancel);
        self.send(&pkt)?;
        self.world.prompt = None;
        Ok(())
    }

    /// Read whatever is available once and apply every complete game packet to
    /// the world. Returns the number of packets applied (0 on a read timeout).
    pub fn pump_once(&mut self) -> Result<usize, DriverError> {
        let mut buf = [0u8; 8192];
        match self.stream.read(&mut buf) {
            Ok(0) => return Err(DriverError::ConnectionClosed),
            Ok(n) => self.decoder.feed(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                return Ok(0)
            }
            Err(e) => return Err(DriverError::Io(e)),
        }
        let mut applied = 0;
        loop {
            match self.decoder.pop() {
                Ok(Some(frame)) => {
                    self.handle_frame(&frame)?;
                    applied += 1;
                }
                Ok(None) => break,
                Err(e) => return Err(DriverError::Framing(e)),
            }
        }
        Ok(applied)
    }

    /// Route a frame: movement acks drive the [`Walker`], the version request
    /// gets answered, everything else goes to the world codec.
    fn handle_frame(&mut self, frame: &[u8]) -> Result<(), DriverError> {
        match frame.first().copied() {
            // 0x22 ConfirmWalk: [id][seq][notoriety]
            Some(0x22) if frame.len() >= 2 => {
                self.confirms += 1;
                self.walker.on_confirm(&mut self.world, frame[1]);
                // A bad/out-of-order confirm desynced us → request a Resync
                // (ClassicUO ConfirmWalk isBadStep → Send_Resync). Walking stays
                // gated (Walker.walking_failed) until the server replies with a deny.
                if let Some(pkt) = self.walker.take_resync() {
                    self.stream.write_all(&pkt)?;
                }
            }
            // 0x21 DenyWalk: [id][seq][x:u16][y:u16][dir:u8][z:i8]
            Some(0x21) if frame.len() >= 8 => {
                self.denies += 1;
                let seq = frame[1];
                let x = u16::from_be_bytes([frame[2], frame[3]]);
                let y = u16::from_be_bytes([frame[4], frame[5]]);
                let dir = frame[6] & 0x07;
                let z = frame[7] as i8;
                self.walker.on_deny(&mut self.world, seq, x, y, z, dir);
            }
            // 0xBD ClientVersion request — must answer or the server denies movement.
            Some(0xBD) => {
                self.stream.write_all(&build_client_version(CLIENT_VERSION))?;
            }
            _ => {
                // A server-pushed jump of the player's tile (>1 step) is a teleport
                // — e.g. a GM [Set X Y Z, a moongate, a recall. It can arrive as
                // 0x20 / 0x77 / 0x78 depending on the server, so detect it by the
                // position delta rather than the packet id, and resync the walk
                // predictor (stale sequence + pending steps would deny all movement).
                let before = self.world.player_mobile().map(|m| m.pos);
                apply_packet(&mut self.world, frame);
                if let (Some(b), Some(a)) =
                    (before, self.world.player_mobile().map(|m| m.pos))
                {
                    if b.x.abs_diff(a.x).max(b.y.abs_diff(a.y)) > 1 {
                        self.walker.reset();
                    }
                }
            }
        }
        Ok(())
    }

    /// Request one step in `dir` (UO direction 0..7). Sends the walk packet if
    /// the pending budget allows; the caller should [`pump_once`](Self::pump_once)
    /// to receive the confirm/deny. Returns whether a packet was sent.
    pub fn walk(&mut self, dir: u8, run: bool) -> Result<bool, DriverError> {
        if let Some(packet) = self.walker.step(&mut self.world, dir, run) {
            self.stream.write_all(&packet)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Pump until `duration` elapses, accumulating world state.
    pub fn observe(&mut self, duration: Duration) -> Result<usize, DriverError> {
        let deadline = Instant::now() + duration;
        let mut total = 0;
        while Instant::now() < deadline {
            total += self.pump_once()?;
        }
        Ok(total)
    }

    /// Pathfind to `(gx, gy)` over `terrain` and walk there on the server,
    /// re-pathing each step. Tiles the server *denies* (the static map said
    /// walkable but a building/dynamic blocker disagrees) are blacklisted and
    /// routed around. Returns whether we arrived within `max_steps`.
    pub fn navigate_to<T: Terrain>(
        &mut self,
        terrain: &mut T,
        gx: u32,
        gy: u32,
        max_steps: usize,
    ) -> Result<bool, DriverError> {
        let mut blocked: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        for _ in 0..max_steps {
            let p = match self.world.player_mobile() {
                Some(p) => p.clone(),
                None => return Ok(false),
            };
            let (px, py, pz) = (p.pos.x as u32, p.pos.y as u32, p.pos.z as i32);
            if (px, py) == (gx, gy) {
                return Ok(true);
            }

            let path = {
                let mut avoid = Avoiding {
                    inner: terrain,
                    blocked: &blocked,
                };
                match find_path(&mut avoid, (px, py, pz), (gx, gy), DEFAULT_MAX_EXPANSIONS) {
                    Some(p) if !p.is_empty() => p,
                    _ => return Ok(false), // no route given what we've learned
                }
            };

            let step = path[0];
            let was_facing = p.direction == step.dir; // a same-facing step is a real move
            self.walk(step.dir, false)?;
            self.observe(std::time::Duration::from_millis(450))?;

            // If a move (not a turn) didn't change our position, the server
            // denied that tile — remember it so the next re-path avoids it.
            if was_facing {
                if let Some(np) = self.world.player_mobile() {
                    if (np.pos.x as u32, np.pos.y as u32) == (px, py) {
                        blocked.insert((step.x, step.y));
                    }
                }
            }
        }
        Ok(false)
    }

    /// Send a pre-built packet to the server (client→server is uncompressed).
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), DriverError> {
        self.stream.write_all(bytes)?;
        Ok(())
    }
}

/// Run the login handshake and return the live game-server connection.
fn login(
    endpoint: &Endpoint,
    cfg: LoginConfig,
) -> Result<(LoginResult, TcpStream, StreamDecoder), DriverError> {
    let (mut machine, initial) = LoginMachine::start(cfg);

    let mut stream = connect(endpoint)?;
    stream.write_all(&initial)?;

    let mut decoder = StreamDecoder::new();
    let mut buf = [0u8; 8192];

    loop {
        loop {
            let frame = match decoder.pop() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => return Err(DriverError::Framing(e)),
            };
            for directive in machine.on_packet(&frame).map_err(DriverError::Login)? {
                match directive {
                    LoginDirective::Send(bytes) => stream.write_all(&bytes)?,
                    LoginDirective::ReconnectToGameServer { then } => {
                        stream = connect(endpoint)?;
                        decoder.switch_to_game();
                        stream.write_all(&then)?;
                    }
                    LoginDirective::Done(result) => return Ok((result, stream, decoder)),
                }
            }
        }

        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Err(DriverError::ConnectionClosed);
        }
        decoder.feed(&buf[..n]);
    }
}

/// Wraps a terrain with a dynamic blacklist of server-denied tiles.
struct Avoiding<'a, T: Terrain> {
    inner: &'a mut T,
    blocked: &'a std::collections::HashSet<(u32, u32)>,
}

impl<T: Terrain> Terrain for Avoiding<'_, T> {
    fn walkable_step(&mut self, x: u32, y: u32, from_z: i32) -> Option<i32> {
        if self.blocked.contains(&(x, y)) {
            None
        } else {
            self.inner.walkable_step(x, y, from_z)
        }
    }

    // Forwarded unchanged: a tile this wrapper blacklists never reaches
    // `find_path`/`find_path_near` as a candidate next hop in the first place
    // (its `walkable_step` above already denies it), so there's nothing
    // door-related to special-case here.
    fn door_at(&mut self, x: u32, y: u32, current_z: i32) -> Option<u32> {
        self.inner.door_at(x, y, current_z)
    }
}

fn connect(e: &Endpoint) -> Result<TcpStream, DriverError> {
    let stream = TcpStream::connect((e.host.as_str(), e.port))?;
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(CONNECT_READ_TIMEOUT)).ok();
    Ok(stream)
}

#[cfg(test)]
mod route_tests {
    //! [`Route`]'s state machine is deliberately network-free (see its doc), so
    //! it's tested directly here with a stubbed [`Terrain`] — no socket needed.
    //! `Session::apply_action`'s one-line cancel-on-`Walk`/replace-on-`WalkTo`
    //! wiring around it isn't separately unit-tested: exercising it needs a
    //! live `Session` (a connected `TcpStream`), which per this crate's testing
    //! rules stays out of unit tests.
    use super::*;

    /// An unbounded, always-walkable grid — isolates the route bookkeeping
    /// (cadence/blacklist/arrival) from pathfinding-around-obstacles, which
    /// `anima-core::path` already covers.
    struct OpenGrid;
    impl Terrain for OpenGrid {
        fn walkable_step(&mut self, _x: u32, _y: u32, _from_z: i32) -> Option<i32> {
            Some(0)
        }
    }

    /// Nothing is walkable — models an unreachable goal.
    struct Sealed;
    impl Terrain for Sealed {
        fn walkable_step(&mut self, _x: u32, _y: u32, _from_z: i32) -> Option<i32> {
            None
        }
    }

    #[test]
    fn advance_steps_toward_goal_when_due() {
        let mut terrain = OpenGrid;
        let mut route = Route::new(5, 5);
        // `Route::new` starts already "due" so the very first tick steps
        // immediately instead of waiting a full cadence.
        match route.advance(&mut terrain, (0, 0, 0), 0) {
            RouteStep::Walk(_) => {}
            other => panic!("expected Walk, got {other:?}"),
        }
    }

    #[test]
    fn advance_reports_done_on_arrival() {
        let mut terrain = OpenGrid;
        let mut route = Route::new(3, 3);
        assert_eq!(route.advance(&mut terrain, (3, 3, 0), 0), RouteStep::Done);
    }

    #[test]
    fn advance_reports_done_when_unreachable() {
        let mut terrain = Sealed;
        let mut route = Route::new(5, 5);
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 0), RouteStep::Done);
    }

    #[test]
    fn advance_waits_out_the_cadence_between_steps() {
        let mut terrain = OpenGrid;
        let mut route = Route::new(5, 5);
        assert!(matches!(route.advance(&mut terrain, (0, 0, 0), 0), RouteStep::Walk(_)));
        route.step_sent(true);
        assert_eq!(route.steps, 1);
        // The cadence clock was just reset — immediately due again is a Wait,
        // not a second step (mirrors play.rs's own `AUTO_WALK_STEP_MS` gate).
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 0), RouteStep::Wait);
    }

    #[test]
    fn advance_blacklists_a_denied_tile_and_reroutes() {
        let mut terrain = OpenGrid;
        let mut route = Route::new(5, 0);
        // Already facing east (2 — see `direction_delta`), so the proposed
        // step is a real move, not a turn-first.
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::Walk(2));
        // Only `step_sent(true)` arms the candidate — the send succeeded here.
        route.step_sent(true);
        assert_eq!(route.target, (1, 0));

        // Force the cadence due again and simulate the server denying that
        // step: the player is still at (0, 0) instead of having moved to (1, 0).
        route.last_step = Instant::now() - ROUTE_STEP;
        let next = route.advance(&mut terrain, (0, 0, 0), 2);
        assert!(route.blocked.contains(&(1, 0)), "denied tile should be blacklisted");
        assert!(matches!(next, RouteStep::Walk(_)), "should reroute, not give up");
    }

    #[test]
    fn advance_does_not_blacklist_a_gated_send_and_retries_the_same_tile() {
        // Regression for the case where a Walker gate (5 unacked steps in
        // flight, or `walking_failed` latched) swallows the walk packet:
        // `advance` must not treat "we never sent this" as a server deny.
        let mut terrain = OpenGrid;
        let mut route = Route::new(5, 0);
        // Already facing east, so the proposed step is a real move.
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::Walk(2));
        // The send was gated (e.g. `Session::walk` returned `false`) — nothing
        // reached the wire, so nothing should be armed.
        route.step_sent(false);
        assert_eq!(route.steps, 0, "a gated send must not count toward the runaway guard");

        // Force the cadence due again. The player never moved (no packet went
        // out), so this must not be mistaken for a server deny on (1, 0).
        route.last_step = Instant::now() - ROUTE_STEP;
        let next = route.advance(&mut terrain, (0, 0, 0), 2);
        assert!(route.blocked.is_empty(), "a never-sent step must not blacklist anything");
        assert_eq!(next, RouteStep::Walk(2), "should simply retry the same tile, not give up");
    }

    #[test]
    fn advance_gives_up_once_fully_boxed_in() {
        // A single-width corridor along y=0: a complete path exists at first,
        // but once its only first step is blacklisted (a denied "move") there
        // is no detour (every other row is walled), so the route gives up.
        struct Corridor;
        impl Terrain for Corridor {
            fn walkable_step(&mut self, _x: u32, y: u32, _from_z: i32) -> Option<i32> {
                if y == 0 { Some(0) } else { None }
            }
        }
        let mut terrain = Corridor;
        let mut route = Route::new(5, 0);
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::Walk(2));
        route.step_sent(true);

        route.last_step = Instant::now() - ROUTE_STEP;
        let next = route.advance(&mut terrain, (0, 0, 0), 2); // deny: still at (0,0)
        assert!(route.blocked.contains(&(1, 0)));
        assert_eq!(next, RouteStep::Done);
    }

    // ------------------------------------------------------------------
    // Door awareness + `find_path_near` — mirrors `play_server`'s own
    // `find_path_routes_through_a_closed_door` / `decide_blocked_step_*` /
    // `find_path_near_*` tests, but exercised through `Route::advance` itself
    // (the actual thing a headless brain drives), using `scene`'s door
    // constants directly so a tuning change there can't silently desync the
    // two suites.
    // ------------------------------------------------------------------
    use crate::scene::{DOOR_USE_COOLDOWN, MAX_DOOR_OPEN_ATTEMPTS};

    /// A single-row corridor (like [`advance_gives_up_once_fully_boxed_in`]'s
    /// `Corridor`) whose only connection at `door_tile` is a closed door —
    /// PLANNING (`walkable_step`) treats it as passable (mirrors
    /// `tile_walkable_for_planning`'s door exception) so a route can be found
    /// through it at all; `door_at` reports the door only while `open` is
    /// `false`, letting a test flip it to simulate the server's `Use` response
    /// landing.
    struct DoorCorridor {
        door_tile: (u32, u32),
        door_serial: u32,
        open: std::cell::Cell<bool>,
    }
    impl Terrain for DoorCorridor {
        fn walkable_step(&mut self, _x: u32, y: u32, _from_z: i32) -> Option<i32> {
            if y == 0 { Some(0) } else { None }
        }
        fn door_at(&mut self, x: u32, y: u32, _current_z: i32) -> Option<u32> {
            if (x, y) == self.door_tile && !self.open.get() { Some(self.door_serial) } else { None }
        }
    }

    #[test]
    fn advance_plans_a_route_through_a_closed_door_only_connection() {
        // The door is the ONLY connection in this corridor — if planning
        // treated a closed door as an ordinary wall, this would report `Done`
        // (no path) immediately, exactly like `advance_gives_up_once_fully_boxed_in`'s
        // sealed corridor. Instead it must recognize the door and negotiate
        // opening it, not give up.
        let mut terrain = DoorCorridor { door_tile: (1, 0), door_serial: 0xDEAD, open: false.into() };
        let mut route = Route::new(5, 0);
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::OpenDoor(0xDEAD));
    }

    #[test]
    fn advance_opens_the_door_on_approach_then_walks_through_once_open() {
        let mut terrain = DoorCorridor { door_tile: (1, 0), door_serial: 0xDEAD, open: false.into() };
        let mut route = Route::new(5, 0);
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::OpenDoor(0xDEAD));
        // A door-open attempt is not a walk step — mirrors `play_server`'s
        // `auto_steps`, which likewise only increments on an actual walk send.
        assert_eq!(route.steps, 0);

        // The door "opens" (as if the server's `Use` response — and the
        // resulting item update — already landed). Once due again, `advance`
        // must actually walk onto it instead of negotiating forever.
        terrain.open.set(true);
        route.last_step = Instant::now() - ROUTE_STEP;
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::Walk(2));
    }

    #[test]
    fn advance_awaits_a_recent_door_use_before_resending() {
        // Mirrors `decide_blocked_step_awaits_a_recent_use_with_no_visible_change`:
        // a `Use` was JUST sent (well within `DOOR_USE_COOLDOWN`) — even though
        // the route's own (much shorter) `ROUTE_STEP` cadence is due again, it
        // must not resend yet (ServUO's `Use` toggles a door, so an impatient
        // resend could close what the first `Use` is about to open).
        let mut terrain = DoorCorridor { door_tile: (1, 0), door_serial: 0xDEAD, open: false.into() };
        let mut route = Route::new(5, 0);
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::OpenDoor(0xDEAD));

        route.last_step = Instant::now() - ROUTE_STEP; // only the route cadence elapses
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::Wait);
        // Still only the one attempt — the await did not resend.
        assert_eq!(route.door_attempts.get(&(1, 0)).map(|a| a.count), Some(1));
    }

    #[test]
    fn advance_resends_the_door_use_once_the_cooldown_elapses() {
        let mut terrain = DoorCorridor { door_tile: (1, 0), door_serial: 0xDEAD, open: false.into() };
        let mut route = Route::new(5, 0);
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::OpenDoor(0xDEAD));

        // Force BOTH the route cadence and the door cooldown to have elapsed.
        route.last_step = Instant::now() - ROUTE_STEP;
        if let Some(a) = route.door_attempts.get_mut(&(1, 0)) {
            a.sent_at = Instant::now() - DOOR_USE_COOLDOWN;
        }
        assert_eq!(route.advance(&mut terrain, (0, 0, 0), 2), RouteStep::OpenDoor(0xDEAD));
        assert_eq!(route.door_attempts.get(&(1, 0)).map(|a| a.count), Some(2));
    }

    #[test]
    fn advance_gives_up_on_a_door_past_the_attempt_cap_and_blacklists_it() {
        // Mirrors `decide_blocked_step_gives_up_on_a_door_past_the_cap`: a door
        // that never opens (a locked door, in real UO terms) still ends in
        // "boxed in" instead of hammering `Use` on it forever.
        let mut terrain = DoorCorridor { door_tile: (1, 0), door_serial: 0xDEAD, open: false.into() };
        let mut route = Route::new(5, 0);

        for attempt in 0..MAX_DOOR_OPEN_ATTEMPTS {
            let step = route.advance(&mut terrain, (0, 0, 0), 2);
            assert_eq!(step, RouteStep::OpenDoor(0xDEAD), "attempt {attempt}");
            // Simulate the cooldown elapsing with no visible effect, so the
            // NEXT `advance` is willing to retry rather than await.
            if let Some(a) = route.door_attempts.get_mut(&(1, 0)) {
                a.sent_at = Instant::now() - DOOR_USE_COOLDOWN;
            }
            route.last_step = Instant::now() - ROUTE_STEP;
        }
        // The cap is reached — treat the tile like any other wall: blacklist
        // it, and (being the corridor's only connection) abandon the route.
        let next = route.advance(&mut terrain, (0, 0, 0), 2);
        assert!(route.blocked.contains(&(1, 0)));
        assert_eq!(next, RouteStep::Done);
    }

    #[test]
    fn advance_walks_to_nearest_reachable_tile_when_exact_goal_is_blocked() {
        // Mirrors `find_path_near`'s ClassicUO-parity fallback (and
        // `play_server`'s own `WalkTo` adjustment): the exact goal tile is
        // unstandable (a wall decoration, a tree, a crate), but `Route` must
        // still walk up to it and stop adjacent instead of refusing to move
        // at all (the OLD `find_path`-only behavior: no path to the exact
        // goal → `Done` at zero steps issued).
        struct BlockedGoal;
        impl Terrain for BlockedGoal {
            fn walkable_step(&mut self, x: u32, y: u32, _from_z: i32) -> Option<i32> {
                if (x, y) == (5, 5) { None } else { Some(0) }
            }
        }
        let mut terrain = BlockedGoal;
        let mut route = Route::new(5, 5);
        let mut pos = (0u16, 0u16, 0i8);
        let mut facing = 2u8;
        let mut walked = 0;
        for _ in 0..20 {
            route.last_step = Instant::now() - ROUTE_STEP;
            match route.advance(&mut terrain, pos, facing) {
                RouteStep::Walk(dir) => {
                    route.step_sent(true);
                    walked += 1;
                    let (dx, dy) = anima_core::net::movement::direction_delta(dir);
                    pos = ((pos.0 as i32 + dx) as u16, (pos.1 as i32 + dy) as u16, 0);
                    facing = dir;
                }
                RouteStep::Done => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(walked > 0, "must actually walk toward the blocked goal, not give up at step 0");
        let cheb = (pos.0 as i32 - 5).unsigned_abs().max((pos.1 as i32 - 5).unsigned_abs());
        assert_eq!(cheb, 1, "should stop exactly adjacent to the blocked goal");
        assert_ne!((pos.0 as u32, pos.1 as u32), (5, 5), "must not walk onto the unstandable goal tile itself");
    }
}

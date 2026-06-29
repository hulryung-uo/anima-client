//! Native TCP driver for `anima-core`'s sans-IO protocol.
//!
//! `anima-core` knows the UO protocol but never touches a socket; this crate
//! provides the blocking `std::net` loop that feeds bytes in and writes bytes
//! out, driving the [`LoginMachine`] to completion and then maintaining a live
//! [`World`] from the server's game-packet stream. The browser build will have
//! an analogous WebSocket driver; the core stays identical.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub mod json;
pub mod scene;

use anima_core::agent::{Action, Observation};
use anima_core::net::outgoing::{
    build_attack, build_book_page_request, build_buy, build_cast_spell, build_double_click,
    build_drop, build_equip, build_gump_response, build_opl_request, build_party_accept,
    build_party_decline, build_party_invite, build_party_leave, build_party_message,
    build_pick_up,
    build_popup_request, build_popup_select, build_say, build_sell, build_single_click,
    build_skill_lock, build_status_request, build_target_response, build_unicode_say,
    build_use_ability, build_use_skill, build_war_mode,
};
use anima_core::net::{
    apply_packet, build_client_version, FramingError, LoginConfig, LoginDirective, LoginError,
    LoginMachine, LoginResult, StreamDecoder, Walker,
};
use anima_core::path::{find_path, Terrain, DEFAULT_MAX_EXPANSIONS};
use anima_core::world::World;

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
            Action::BuyItems { vendor, items } => self.send(&build_buy(*vendor, items))?,
            Action::SellItems { vendor, items } => self.send(&build_sell(*vendor, items))?,
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
            // Auto-walk needs the terrain/map to pathfind + pace steps, which the
            // headless driver doesn't own. The play-server intercepts `WalkTo`
            // before it reaches here (see its game loop); this arm is a no-op so
            // a stray `WalkTo` through the generic path is simply ignored.
            Action::WalkTo { .. } => {}
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
                apply_packet(&mut self.world, frame);
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
}

fn connect(e: &Endpoint) -> Result<TcpStream, DriverError> {
    let stream = TcpStream::connect((e.host.as_str(), e.port))?;
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(CONNECT_READ_TIMEOUT)).ok();
    Ok(stream)
}

//! The two-phase UO login flow, as a **sans-IO state machine**.
//!
//! Phase 1 (login server, uncompressed):
//!   Seed → AccountLogin → [ServerList] → ServerSelect → [ServerRedirect]
//! Phase 2 (game server, Huffman-compressed incoming):
//!   GameSeed → GameLogin → [CharacterList] → PlayCharacter → [LoginConfirm]
//!
//! `[...]` are server packets we receive; the rest we send. The machine owns
//! *only* the protocol logic: you drive the actual sockets and hand it framed
//! packets via [`LoginMachine::on_packet`], executing the [`LoginDirective`]s it
//! returns. This keeps it WASM/native-agnostic and unit-testable without IO.
//!
//! Spec source: `anima/anima/client/{packets,connection}.py`. Character
//! create/delete are not yet implemented (see `TODO`s) — the happy path assumes
//! an existing character.

use super::packet::{PacketReader, PacketWriter};

// ---------------------------------------------------------------------------
// Packet builders (client → server). Pure; each returns the exact wire bytes.
// ---------------------------------------------------------------------------

/// Seed packet `0xEF` (21 bytes) — opens the phase-1 connection and advertises
/// the client version (default 7.0.102.3).
pub fn build_seed(seed: u32, version: (u32, u32, u32, u32)) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xEF)
        .u32(seed)
        .u32(version.0)
        .u32(version.1)
        .u32(version.2)
        .u32(version.3);
    w.into_vec()
}

/// AccountLogin `0x80` (62 bytes).
pub fn build_account_login(username: &str, password: &str) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x80)
        .fixed_ascii(username, 30)
        .fixed_ascii(password, 30)
        .u8(0xFF); // next_login_key
    w.into_vec()
}

/// ServerSelect `0xA0` (3 bytes).
pub fn build_server_select(index: u16) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xA0).u16(index);
    w.into_vec()
}

/// Phase-2 game seed: a bare 4-byte big-endian auth key (NO `0xEF` header).
/// Sent first on the freshly-opened game-server connection.
pub fn build_game_seed(auth_key: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u32(auth_key);
    w.into_vec()
}

/// GameLogin `0x91` (65 bytes).
pub fn build_game_login(auth_key: u32, username: &str, password: &str) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x91)
        .u32(auth_key)
        .fixed_ascii(username, 30)
        .fixed_ascii(password, 30);
    w.into_vec()
}

/// All facets enabled (Fel|Tram|Ilsh|Malas|Tokuno|TerMur), matching the modern
/// client version we advertise. See `anima` `_ALL_FACET_CLIENT_FLAGS`.
pub const ALL_FACET_CLIENT_FLAGS: u32 = 0x3F;

/// PlayCharacter `0x5D` (73 bytes) — select an existing character by slot.
pub fn build_play_character(name: &str, slot: u32, client_ip: u32, client_flags: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x5D)
        .u32(0xEDED_EDED) // pattern
        .fixed_ascii(name, 30)
        .zeros(2)
        .u32(client_flags)
        .zeros(24)
        .u32(slot)
        .u32(client_ip);
    w.into_vec()
}

/// Character appearance for creation. Defaults to a valid human ServUO accepts
/// (stats sum to exactly 90, as modern `NewCharacterCreation` requires).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterAppearance {
    pub name: String,
    pub female: bool,
    pub skin_hue: u16,
    pub hair_style: u16,
    pub hair_hue: u16,
    pub facial_hair_style: u16,
    pub facial_hair_hue: u16,
    pub shirt_hue: u16,
    pub pants_hue: u16,
    pub strength: u8,
    pub dexterity: u8,
    pub intelligence: u8,
    pub city_index: u16, // 0 = New Haven
    /// Four (skill_id, value) pairs.
    pub skills: [(u8, u8); 4],
}

impl Default for CharacterAppearance {
    fn default() -> Self {
        Self {
            name: "Anima".to_string(),
            female: false,
            skin_hue: 0x03EA,
            hair_style: 0x203B,
            hair_hue: 0x044D,
            facial_hair_style: 0,
            facial_hair_hue: 0x044D,
            shirt_hue: 0x0002,
            pants_hue: 0x0002,
            strength: 60,
            dexterity: 20,
            intelligence: 10, // 60+20+10 = 90
            city_index: 0,
            skills: [(0, 50), (1, 50), (2, 0), (3, 0)],
        }
    }
}

/// Human race id in the gender+race byte. The modern (CV ≥ 7.0.0.0) encoding is
/// `race * 2 + female`, so a human sends 2 (male) / 3 (female).
const HUMAN_RACE_ID: u8 = 1;

/// CreateCharacter `0xF8` (106 bytes). See `anima` `build_create_character` for
/// the per-field rationale (gender/race encoding, client-flags, stat rules).
pub fn build_create_character(app: &CharacterAppearance, slot: u16, client_flags: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0xF8)
        .u32(0xEDED_EDED) // pattern1
        .u32(0xFFFF_FFFF) // pattern2
        .u8(0x00) // pattern3
        .fixed_ascii(&app.name, 30)
        .zeros(2) // unknown
        .u32(client_flags)
        .u32(0x0000_0001) // unknown (ClassicUO sends 1)
        .u32(0x0000_0000) // login count
        .u8(0) // profession (0 = custom)
        .zeros(15); // reserved

    let gender_race = HUMAN_RACE_ID * 2 + app.female as u8;
    w.u8(gender_race)
        .u8(app.strength)
        .u8(app.dexterity)
        .u8(app.intelligence);

    for (skill_id, value) in app.skills {
        w.u8(skill_id).u8(value);
    }

    w.u16(app.skin_hue)
        .u16(app.hair_style)
        .u16(app.hair_hue)
        .u16(app.facial_hair_style)
        .u16(app.facial_hair_hue)
        .u16(app.city_index)
        .zeros(2) // padding
        .u16(slot)
        .u32(0x7F00_0001) // client IP
        .u16(app.shirt_hue)
        .u16(app.pants_hue);

    let mut data = w.into_vec();
    data.resize(106, 0); // pad/trim to exactly 106
    data
}

// ---------------------------------------------------------------------------
// Packet parsers (server → client). Each takes the full frame (id included).
// ---------------------------------------------------------------------------

/// Result of a completed login: who/where we are in the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginResult {
    pub serial: u32,
    pub x: u16,
    pub y: u16,
    pub z: i8,
    pub direction: u8,
    pub body: u16,
    /// Server advertised the AOS expansion via SupportedFeatures `0xB9`.
    pub aos: bool,
}

/// SupportedFeatures `0xB9` AOS expansion bit (ClassicUO `LockedFeatureFlags.AOS`).
const FEATURE_AOS: u32 = 0x0000_0010;

/// Parse the SupportedFeatures `0xB9` flags. The payload is a big-endian u16 on
/// pre-6.0.14.2 clients and a u32 on newer ones; we read whatever the frame
/// carries (id byte + 2 or 4 flag bytes).
fn parse_supported_features(frame: &[u8]) -> u32 {
    let body = &frame[1..];
    if body.len() >= 4 {
        u32::from_be_bytes([body[0], body[1], body[2], body[3]])
    } else if body.len() >= 2 {
        u16::from_be_bytes([body[0], body[1]]) as u32
    } else {
        0
    }
}

/// Parse the auth key out of ServerRedirect `0x8C`.
/// Layout: `[0x8C][ip:u32][port:u16][auth_key:u32]` (11 bytes).
pub fn parse_server_redirect(frame: &[u8]) -> Result<u32, LoginError> {
    let mut r = PacketReader::new(&frame[1..]);
    r.bytes(4).map_err(|_| LoginError::Truncated(0x8C))?; // ip (we reconnect to same host)
    r.bytes(2).map_err(|_| LoginError::Truncated(0x8C))?; // port
    r.u32().map_err(|_| LoginError::Truncated(0x8C))
}

/// A character slot from the character-list packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharSlot {
    pub index: u8,
    pub name: String,
}

/// Parse CharacterList `0xA9` / `0x86`. Layout after the 3-byte `[id][len:u16]`
/// header: `[count:u8]` then `count × ([name:ascii30][password/pad:30])`.
/// Returns only the *named* (non-empty) slots.
pub fn parse_character_list(frame: &[u8]) -> Result<Vec<CharSlot>, LoginError> {
    let id = frame[0];
    let mut r = PacketReader::new(&frame[3..]); // skip id + u16 length
    let count = r.u8().map_err(|_| LoginError::Truncated(id))?;
    let mut slots = Vec::new();
    for i in 0..count {
        let name = r.fixed_ascii(30).map_err(|_| LoginError::Truncated(id))?;
        r.bytes(30).map_err(|_| LoginError::Truncated(id))?; // password/pad field
        if !name.is_empty() {
            slots.push(CharSlot { index: i, name });
        }
    }
    Ok(slots)
}

/// Parse LoginConfirm `0x1B` (37 bytes).
/// Layout: `[0x1B][serial:u32][0:u32][body:u16][x:u16][y:u16][z:u16][dir:u8]...`
/// Z is written as `(short)Z` and narrowed to a signed byte; direction is the
/// next byte masked with `0x7`. (See `anima` `parse_login_confirm` for the
/// alignment history.)
pub fn parse_login_confirm(frame: &[u8]) -> Result<LoginResult, LoginError> {
    let mut r = PacketReader::new(&frame[1..]);
    let t = |_| LoginError::Truncated(0x1B);
    let serial = r.u32().map_err(t)?;
    r.bytes(4).map_err(t)?; // unknown (always 0)
    let body = r.u16().map_err(t)?;
    let x = r.u16().map_err(t)?;
    let y = r.u16().map_err(t)?;
    let z = r.u16().map_err(t)? as i8; // (short) → (sbyte) narrowing, matches ClassicUO
    let direction = r.u8().map_err(t)? & 0x07;
    Ok(LoginResult {
        serial,
        x,
        y,
        z,
        direction,
        body,
        aos: false, // filled in by the LoginMachine from SupportedFeatures 0xB9
    })
}

// ---------------------------------------------------------------------------
// The state machine.
// ---------------------------------------------------------------------------

/// What the driver must do in response to a fed packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginDirective {
    /// Write these bytes to the current connection.
    Send(Vec<u8>),
    /// Close the phase-1 (login-server) connection, open a fresh one to the game
    /// server, switch the incoming framer to **game mode (Huffman)**, then write
    /// `then`. Everything received after this is Huffman-compressed.
    ReconnectToGameServer { then: Vec<u8> },
    /// Login finished — we're in the world.
    Done(LoginResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginError {
    /// Server rejected us. `0x82` (login) / `0x82` (game). `reason` is the code.
    Denied(u8),
    /// A packet ended before we'd read everything its layout requires.
    Truncated(u8),
    /// We reached the character list but there were no characters and creation
    /// is not yet implemented.
    NoCharacterAndCreateUnsupported,
    /// Got a packet that doesn't belong in the current state in a way we can't ignore.
    Unexpected { state: &'static str, id: u8 },
}

/// Inputs that vary per login attempt.
#[derive(Debug, Clone)]
pub struct LoginConfig {
    pub username: String,
    pub password: String,
    pub seed: u32,
    pub version: (u32, u32, u32, u32),
    pub server_index: u16,
    /// Preferred character slot; falls back to the first named slot.
    pub character_slot: u8,
    pub client_ip: u32,
    /// When the account has no character, create one from this appearance.
    pub create_if_missing: bool,
    pub appearance: CharacterAppearance,
}

impl Default for LoginConfig {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            seed: 0x0102_0304,
            version: (7, 0, 102, 3),
            server_index: 0,
            character_slot: 0,
            client_ip: 0x7F00_0001, // 127.0.0.1
            create_if_missing: true,
            appearance: CharacterAppearance::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    AwaitServerList,
    AwaitRedirect,
    AwaitCharacterList,
    AwaitLoginConfirm,
    Done,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::AwaitServerList => "AwaitServerList",
            State::AwaitRedirect => "AwaitRedirect",
            State::AwaitCharacterList => "AwaitCharacterList",
            State::AwaitLoginConfirm => "AwaitLoginConfirm",
            State::Done => "Done",
        }
    }
}

/// Sans-IO driver for the login handshake.
pub struct LoginMachine {
    cfg: LoginConfig,
    state: State,
    auth_key: u32,
    /// AOS expansion advertised by the server's SupportedFeatures `0xB9`. Drives
    /// client-side gating of AOS-only UI (e.g. the weapon special-ability bar).
    aos: bool,
}

impl LoginMachine {
    /// Create the machine and the initial bytes to send on the freshly-opened
    /// **login-server** connection (Seed + AccountLogin).
    pub fn start(cfg: LoginConfig) -> (Self, Vec<u8>) {
        let mut initial = build_seed(cfg.seed, cfg.version);
        initial.extend(build_account_login(&cfg.username, &cfg.password));
        let m = Self {
            cfg,
            state: State::AwaitServerList,
            auth_key: 0,
            aos: false,
        };
        (m, initial)
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// Feed one fully-framed packet (id byte included). Returns the directives
    /// to execute, or an error. Packets irrelevant to the current step are
    /// ignored (empty result) so the driver can pass everything through.
    pub fn on_packet(&mut self, frame: &[u8]) -> Result<Vec<LoginDirective>, LoginError> {
        if frame.is_empty() {
            return Ok(vec![]);
        }
        let id = frame[0];

        // LoginDenied can arrive in either phase.
        if id == 0x82 {
            let reason = frame.get(1).copied().unwrap_or(0);
            return Err(LoginError::Denied(reason));
        }

        // SupportedFeatures `0xB9` (sent during the character-list phase): records
        // the AOS expansion bit so the world can gate AOS-only UI later. Ignorable
        // otherwise — fall through to an empty result.
        if id == 0xB9 {
            self.aos = parse_supported_features(frame) & FEATURE_AOS != 0;
            return Ok(vec![]);
        }

        match self.state {
            State::AwaitServerList => {
                if id == 0xA8 {
                    self.state = State::AwaitRedirect;
                    Ok(vec![LoginDirective::Send(build_server_select(
                        self.cfg.server_index,
                    ))])
                } else {
                    Ok(vec![]) // ignore unrelated phase-1 chatter
                }
            }
            State::AwaitRedirect => {
                if id == 0x8C {
                    self.auth_key = parse_server_redirect(frame)?;
                    self.state = State::AwaitCharacterList;
                    let mut then = build_game_seed(self.auth_key);
                    then.extend(build_game_login(
                        self.auth_key,
                        &self.cfg.username,
                        &self.cfg.password,
                    ));
                    Ok(vec![LoginDirective::ReconnectToGameServer { then }])
                } else {
                    Ok(vec![])
                }
            }
            State::AwaitCharacterList => {
                if id == 0xA9 || id == 0x86 {
                    let slots = parse_character_list(frame)?;
                    let chosen = slots
                        .iter()
                        .find(|s| s.index == self.cfg.character_slot)
                        .or_else(|| slots.first());
                    match chosen {
                        Some(slot) => {
                            self.state = State::AwaitLoginConfirm;
                            Ok(vec![LoginDirective::Send(build_play_character(
                                &slot.name,
                                slot.index as u32,
                                self.cfg.client_ip,
                                ALL_FACET_CLIENT_FLAGS,
                            ))])
                        }
                        None if self.cfg.create_if_missing => {
                            self.state = State::AwaitLoginConfirm;
                            Ok(vec![LoginDirective::Send(build_create_character(
                                &self.cfg.appearance,
                                0,
                                ALL_FACET_CLIENT_FLAGS,
                            ))])
                        }
                        None => Err(LoginError::NoCharacterAndCreateUnsupported),
                    }
                } else {
                    Ok(vec![]) // e.g. 0xB9 SupportedFeatures, 0xBD version req, etc.
                }
            }
            State::AwaitLoginConfirm => {
                if id == 0x1B {
                    let mut result = parse_login_confirm(frame)?;
                    result.aos = self.aos;
                    self.state = State::Done;
                    Ok(vec![LoginDirective::Done(result)])
                } else {
                    Ok(vec![]) // pre-login-confirm packets (map change, etc.)
                }
            }
            State::Done => Err(LoginError::Unexpected {
                state: self.state.name(),
                id,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_have_expected_lengths() {
        assert_eq!(build_seed(0x0102_0304, (7, 0, 102, 3)).len(), 21);
        assert_eq!(build_account_login("user", "pass").len(), 62);
        assert_eq!(build_server_select(0).len(), 3);
        assert_eq!(build_game_seed(0xDEAD_BEEF).len(), 4);
        assert_eq!(build_game_login(0, "user", "pass").len(), 65);
        assert_eq!(build_play_character("Anima", 0, 0x7F00_0001, 0x3F).len(), 73);
    }

    #[test]
    fn account_login_layout() {
        let p = build_account_login("test5", "test5");
        assert_eq!(p[0], 0x80);
        // username field is NUL-padded ASCII starting at offset 1.
        assert_eq!(&p[1..6], b"test5");
        assert_eq!(p[6], 0); // padding
        assert_eq!(*p.last().unwrap(), 0xFF); // next_login_key
    }

    #[test]
    fn parse_redirect_and_login_confirm() {
        // ServerRedirect 0x8C: id, ip(4), port(2), auth(4) = 11 bytes
        let frame = [0x8C, 1, 2, 3, 4, 0x0A, 0x21, 0xDE, 0xAD, 0xBE, 0xEF];
        assert_eq!(parse_server_redirect(&frame).unwrap(), 0xDEAD_BEEF);

        // LoginConfirm 0x1B (37 bytes), serial=0x2A, body=400, x=1000, y=2000,
        // z=-5 (0xFFFB as short), dir=3.
        let mut w = PacketWriter::new();
        w.u8(0x1B)
            .u32(0x2A)
            .u32(0)
            .u16(400)
            .u16(1000)
            .u16(2000)
            .u16(0xFFFB)
            .u8(3)
            .zeros(19); // pad 18 bytes of fields up to the 37-byte frame
        let frame = w.into_vec();
        assert_eq!(frame.len(), 37);
        let r = parse_login_confirm(&frame).unwrap();
        assert_eq!(
            r,
            LoginResult {
                serial: 0x2A,
                x: 1000,
                y: 2000,
                z: -5,
                direction: 3,
                body: 400,
                aos: false,
            }
        );
    }

    /// Drive the whole happy path with scripted server packets.
    #[test]
    fn full_happy_path() {
        let cfg = LoginConfig {
            username: "test5".into(),
            password: "test5".into(),
            ..Default::default()
        };
        let (mut m, initial) = LoginMachine::start(cfg);
        assert_eq!(initial[0], 0xEF); // seed first
        assert!(!m.is_done());

        // ServerList 0xA8 (variable). Minimal valid frame: [id][len:u16][body].
        let server_list = vec![0xA8, 0x00, 0x06, 0x00, 0x01, 0x00];
        let d = m.on_packet(&server_list).unwrap();
        assert_eq!(d, vec![LoginDirective::Send(build_server_select(0))]);

        // ServerRedirect 0x8C → reconnect + game seed/login.
        let redirect = [0x8C, 127, 0, 0, 1, 0x0A, 0x21, 0x11, 0x22, 0x33, 0x44];
        let d = m.on_packet(&redirect).unwrap();
        match &d[0] {
            LoginDirective::ReconnectToGameServer { then } => {
                assert_eq!(&then[0..4], &[0x11, 0x22, 0x33, 0x44]); // game seed = auth key
                assert_eq!(then[4], 0x91); // GameLogin follows
            }
            other => panic!("expected reconnect, got {other:?}"),
        }

        // An ignorable phase-2 packet (SupportedFeatures 0xB9) before the list.
        assert_eq!(m.on_packet(&[0xB9, 0, 0, 0, 0]).unwrap(), vec![]);

        // CharacterList 0xA9: one char "Anima" in slot 0.
        let mut w = PacketWriter::new();
        w.u8(0xA9).u16(0).u8(1).fixed_ascii("Anima", 30).zeros(30);
        let mut char_list = w.into_vec();
        let total = char_list.len() as u16;
        char_list[1] = (total >> 8) as u8;
        char_list[2] = (total & 0xFF) as u8;
        let d = m.on_packet(&char_list).unwrap();
        assert_eq!(
            d,
            vec![LoginDirective::Send(build_play_character(
                "Anima",
                0,
                0x7F00_0001,
                ALL_FACET_CLIENT_FLAGS
            ))]
        );

        // LoginConfirm 0x1B → Done.
        let mut w = PacketWriter::new();
        w.u8(0x1B).u32(0x2A).u32(0).u16(400).u16(1000).u16(2000).u16(0).u8(0).zeros(17);
        let confirm = w.into_vec();
        let d = m.on_packet(&confirm).unwrap();
        assert!(matches!(d[0], LoginDirective::Done(_)));
        assert!(m.is_done());
    }

    #[test]
    fn login_denied_errors() {
        let cfg = LoginConfig::default();
        let (mut m, _) = LoginMachine::start(cfg);
        assert_eq!(m.on_packet(&[0x82, 0x03]), Err(LoginError::Denied(3)));
    }
}

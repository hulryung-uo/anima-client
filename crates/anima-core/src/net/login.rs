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
//! creation (`LoginConfig::create_if_missing` / `LoginConfig::create_new`),
//! deferred server-list selection (`LoginConfig::defer_character_choice`), and
//! one-shot deletion (`LoginConfig::delete_existing`, mirroring the Python
//! client's delete-then-recreate login flow) are implemented; the default happy
//! path otherwise assumes an existing character.

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

/// DeleteCharacter `0x83` (39 bytes) — request deletion of the character in
/// `slot`.
///
/// Layout: `[0x83][30 zero bytes][slot:u32 BE][clientIP:u32 BE]`. The 30-byte
/// field is **all zeros** — it is NOT the account password. Modern clients
/// (ClassicUO `Send_DeleteCharacter`) stopped putting the password on the
/// wire here, and ServUO's `PacketHandlers.DeleteCharacter` simply
/// `Seek(30, ...)`s past it before reading the slot; writing a real password
/// into this field would only leak it to anything that *does* read those 30
/// bytes. (`anima` `build_delete_character` keeps a vestigial `password`
/// parameter for call-site compatibility — we don't imitate that here.)
pub fn build_delete_character(slot: u32, client_ip: u32) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.u8(0x83).zeros(30).u32(slot).u32(client_ip);
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

impl CharacterAppearance {
    /// Validate the fields that would otherwise make ServUO reject the whole
    /// `0xF8` request without a useful client-side explanation.
    pub fn validate(&self) -> Result<(), &'static str> {
        let name = self.name.trim();
        validate_character_name(name)?;
        if !(10..=60).contains(&self.strength)
            || !(10..=60).contains(&self.dexterity)
            || !(10..=60).contains(&self.intelligence)
            || u16::from(self.strength) + u16::from(self.dexterity) + u16::from(self.intelligence)
                != 90
        {
            return Err("strength, dexterity, and intelligence must each be 10-60 and total 90");
        }
        let mut skill_total = 0u16;
        let mut used = [false; 256];
        for (id, value) in self.skills {
            if value > 50 {
                return Err("a starting skill may not exceed 50");
            }
            skill_total += u16::from(value);
            if value > 0 {
                if used[id as usize] {
                    return Err("starting skills with a non-zero value must be unique");
                }
                used[id as usize] = true;
            }
        }
        if !matches!(skill_total, 100 | 120) {
            return Err("starting skill values must total exactly 100 or 120");
        }
        Ok(())
    }
}

/// Mirror ServUO `NameVerification.Validate(name, 2, 16, true, false, true,
/// 1, SpaceDashPeriodQuote)` as used by `CharacterCreation.SetName`. Without
/// this, ServUO silently accepts the creation request but replaces an invalid
/// name with `Generic Player`, which looks like a successful client request.
fn validate_character_name(name: &str) -> Result<(), &'static str> {
    const START_DISALLOWED: &[&str] = &["seer", "counselor", "gm", "admin", "lady", "lord"];
    const DISALLOWED_WORDS: &[&str] = &[
        "jigaboo",
        "chigaboo",
        "wop",
        "kyke",
        "kike",
        "tit",
        "spic",
        "prick",
        "piss",
        "lezbo",
        "lesbo",
        "felatio",
        "dyke",
        "dildo",
        "chinc",
        "chink",
        "cunnilingus",
        "cum",
        "cocksucker",
        "cock",
        "clitoris",
        "clit",
        "ass",
        "hitler",
        "penis",
        "nigga",
        "nigger",
        "klit",
        "kunt",
        "jiz",
        "jism",
        "jerkoff",
        "jackoff",
        "goddamn",
        "fag",
        "blowjob",
        "bitch",
        "asshole",
        "dick",
        "pussy",
        "snatch",
        "cunt",
        "twat",
        "shit",
        "fuck",
        "tailor",
        "smith",
        "scholar",
        "rogue",
        "novice",
        "neophyte",
        "merchant",
        "medium",
        "master",
        "mage",
        "lb",
        "journeyman",
        "grandmaster",
        "fisherman",
        "expert",
        "chef",
        "carpenter",
        "british",
        "blackthorne",
        "blackthorn",
        "beggar",
        "archer",
        "apprentice",
        "adept",
        "gamemaster",
        "frozen",
        "squelched",
        "invulnerable",
        "osi",
        "origin",
    ];

    if !(2..=16).contains(&name.len()) {
        return Err("character name must be between 2 and 16 ASCII characters");
    }
    let is_separator = |byte| matches!(byte, b' ' | b'-' | b'.' | b'\'');
    let mut previous_was_separator = false;
    for (index, byte) in name.bytes().enumerate() {
        if byte.is_ascii_alphabetic() {
            previous_was_separator = false;
        } else if is_separator(byte) && index > 0 && !previous_was_separator {
            previous_was_separator = true;
        } else {
            return Err(
                "character name may contain ASCII letters and non-consecutive spaces, dashes, periods, or apostrophes",
            );
        }
    }

    let lower = name.to_ascii_lowercase();
    if START_DISALLOWED
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || lower
            .split([' ', '-', '.', '\''])
            .any(|word| DISALLOWED_WORDS.contains(&word))
    {
        return Err("character name contains a ServUO-reserved word or prefix");
    }
    Ok(())
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
    /// Character-list capability flags from 0xA9. Bit 0x02 selects the
    /// request/ack logout handshake; without it ClassicUO disconnects directly.
    pub character_list_flags: u32,
}

/// SupportedFeatures `0xB9` AOS expansion bit (ClassicUO `LockedFeatureFlags.AOS`).
const FEATURE_AOS: u32 = 0x0000_0010;
/// `CharacterListFlags.OverwriteConfigButton`: use 0xD1 logout request/ack.
pub const CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE: u32 = 0x0000_0002;

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

/// One shard advertised in the server list `0xA8`.
///
/// `index` is what ServerSelect `0xA0` must echo back — it is the server's own
/// id for the row, not our position in the vector, so a shard that numbers its
/// rows sparsely still round-trips (ClassicUO selects `Servers[i].Index`, not
/// `i`, for the same reason).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardEntry {
    pub index: u16,
    pub name: String,
    /// 0..=100, how full the shard reports itself to be.
    pub percent_full: u8,
    /// Signed hours from GMT, as the server states it (`sbyte` on the wire).
    pub timezone: i8,
    /// Dotted quad the shard advertises for itself, already un-reversed (see
    /// [`parse_server_list`]). `[0, 0, 0, 0]` when the shard sends none.
    pub ip: [u8; 4],
}

/// Parse ServerList `0xA8`. Layout after the 3-byte `[id][len:u16]` header:
/// `[flag:u8][count:u16]` then `count ×
/// ([index:u16][name:ascii32][percent_full:u8][timezone:i8][ip:u32])`
/// (ServUO `Server/Network/Packets.cs` `AccountLoginAck`, ClassicUO
/// `LoginScene.ServerListEntry.Create`).
///
/// **The address byte order here is the reverse of `0x8C`'s.** ServUO writes
/// this field with its big-endian `Write(int)` over a value that already holds
/// the address in network-order-as-little-endian, so the four bytes arrive
/// reversed; `0x8C` writes the same value byte-by-byte from the low end, so
/// those arrive in dotted-quad order. ClassicUO carries both interpretations
/// per entry and pings the reversed one — that reversed one is the reachable
/// address, so it is what we store.
pub fn parse_server_list(frame: &[u8]) -> Result<Vec<ShardEntry>, LoginError> {
    let t = |_| LoginError::Truncated(0xA8);
    let mut r = PacketReader::new(frame.get(3..).unwrap_or(&[]));
    r.u8().map_err(t)?; // unused flag byte (ServUO writes 0x5D)
    let count = r.u16().map_err(t)?;
    let mut servers = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let index = r.u16().map_err(t)?;
        let name = r.fixed_ascii(32).map_err(t)?;
        let percent_full = r.u8().map_err(t)?;
        let timezone = r.u8().map_err(t)? as i8;
        let ip = r.bytes(4).map_err(t)?;
        servers.push(ShardEntry {
            index,
            name,
            percent_full,
            timezone,
            ip: [ip[3], ip[2], ip[1], ip[0]],
        });
    }
    Ok(servers)
}

/// Where the login server told us to reconnect, from ServerRedirect `0x8C`.
/// Layout: `[0x8C][ip:4][port:u16][auth_key:u32]` (11 bytes).
///
/// `ip` is in dotted-quad order as received (unlike `0xA8`'s — see
/// [`parse_server_list`]). It is routinely **unusable**: a shard behind NAT
/// advertises its LAN address, and a shard reached through a proxy or an SSH
/// tunnel advertises an address that resolves to the wrong host entirely.
/// ClassicUO handles this with an explicit `IgnoreRelayIp` setting plus an
/// `ip == 0` special case (`LoginScene.HandleRelayServerPacket`), both of
/// which fall back to the address the user originally typed. The core only
/// reports what the packet said; deciding whether to trust it is the driver's
/// call — see [`GameServerAddress::is_routable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameServerAddress {
    pub ip: [u8; 4],
    pub port: u16,
}

impl GameServerAddress {
    /// Whether this address is worth dialing at all. `0.0.0.0` is ClassicUO's
    /// documented "ignore me" value, and a zero port can't be connected to.
    /// Everything else is plausible — a driver that fails to connect anyway
    /// should fall back to the endpoint the user gave it.
    pub fn is_routable(&self) -> bool {
        self.ip != [0, 0, 0, 0] && self.port != 0
    }

    /// `"a.b.c.d"` — the host half, for handing to a resolver/socket API.
    pub fn host(&self) -> String {
        let [a, b, c, d] = self.ip;
        format!("{a}.{b}.{c}.{d}")
    }
}

/// Parse ServerRedirect `0x8C` into the game-server address and auth key.
pub fn parse_server_redirect(frame: &[u8]) -> Result<(GameServerAddress, u32), LoginError> {
    let t = |_| LoginError::Truncated(0x8C);
    let mut r = PacketReader::new(&frame[1..]);
    let ip = r.bytes(4).map_err(t)?;
    let ip = [ip[0], ip[1], ip[2], ip[3]];
    let port = r.u16().map_err(t)?;
    let auth_key = r.u32().map_err(t)?;
    Ok((GameServerAddress { ip, port }, auth_key))
}

/// A character slot from the character-list packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharSlot {
    pub index: u8,
    pub name: String,
}

/// One starting city the server offers in the 0xA9 city list. `index` is what
/// CreateCharacter 0xF8 must echo back in `city_index` — it is a position in
/// THIS list, and shards/expansions order it differently, so it can never be
/// hardcoded client-side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartingCity {
    pub index: u8,
    pub name: String,
    pub building: String,
    /// Starting coordinates + facet, and the cliloc id of the city blurb the
    /// real client shows at character creation. `None` on legacy (63-byte)
    /// records, which carry only the names.
    pub location: Option<CityLocation>,
}

/// The trailing fields of a modern (89-byte) 0xA9 city record: where the
/// character lands, and which cliloc holds the descriptive blurb ServUO's
/// own client displays when picking a starting city (e.g. 1075074 for
/// Britain: "<h2>Britain</h2><br>The City of Bards<br><br> ..."). Verified
/// against ServUO `Server/Network/Packets.cs` `CharacterList`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CityLocation {
    pub x: u32,
    pub y: u32,
    pub z: i32,           // written as i32 by the server
    pub map: u32,         // facet id: 0 Felucca, 1 Trammel, 2 Ilshenar, 3 Malas, 4 Tokuno, 5 TerMur
    pub description: u32, // cliloc id, 0 when unset
}

/// Parse CharacterList `0xA9` / `0x86`. Layout after the 3-byte `[id][len:u16]`
/// header: `[count:u8]` then `count × ([name:ascii30][password/pad:30])`.
/// Returns only the *named* (non-empty) slots.
pub fn parse_character_list(frame: &[u8]) -> Result<Vec<CharSlot>, LoginError> {
    Ok(parse_character_list_with_capacity(frame)?.slots)
}

/// Character slots advertised by the game server after account authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterList {
    pub slots: Vec<CharSlot>,
    pub slot_count: u8,
    /// Starting cities from the 0xA9 tail. Empty for a 0x86 update (no tail).
    pub cities: Vec<StartingCity>,
    pub flags: u32,
}

fn parse_character_list_with_capacity(frame: &[u8]) -> Result<CharacterList, LoginError> {
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
    let (cities, flags) = parse_character_list_tail(r.rest());
    Ok(CharacterList {
        slots,
        slot_count: count,
        cities,
        flags,
    })
}

/// Parse the 0xA9 tail after the character slots: `[city_count:u8]` then that
/// many city records, then the big-endian `CharacterListFlags` u32. A 0x86
/// list update has no tail at all (empty `tail` yields no cities and flags 0).
///
/// City records are 89 bytes on modern clients (`index`, 32-byte City, 32-byte
/// Building, then X/Y/Z/MapID/Description(cliloc)/reserved u32s, captured in
/// `StartingCity::location`) or 63 bytes on legacy ones (`index`, 31-byte
/// City, 31-byte Building only — `location` stays `None`). We don't know up
/// front which shape a given server sent, so we try both
/// strides: an **exact fit** — `1 + count * stride + 4 == tail.len()` — is
/// unambiguous and preferred; if neither stride accounts for every tail byte
/// (some modern frames carry a couple of extra bytes after the flags), fall
/// back to the first stride whose flags offset is still readable, matching
/// the previous (flags-only) parser's behaviour so nothing regresses.
fn parse_character_list_tail(tail: &[u8]) -> (Vec<StartingCity>, u32) {
    let Some(&city_count) = tail.first() else {
        return (Vec::new(), 0);
    };
    let count = usize::from(city_count);

    let exact_fit = |stride: usize| 1 + count * stride + 4 == tail.len();
    let offset_readable = |stride: usize| {
        let offset = 1usize.saturating_add(count.saturating_mul(stride));
        tail.get(offset..offset + 4).is_some()
    };
    let Some(stride) = [89usize, 63]
        .into_iter()
        .find(|&stride| exact_fit(stride))
        .or_else(|| {
            [89usize, 63]
                .into_iter()
                .find(|&stride| offset_readable(stride))
        })
    else {
        return (Vec::new(), 0);
    };

    // Modern (89-byte) records give City/Building 32 bytes each; legacy
    // (63-byte) records give them 31. Either way the layout starts
    // `[index:u8][City][Building]`.
    let name_width = if stride == 89 { 32 } else { 31 };

    let mut cities = Vec::with_capacity(count);
    let mut offset = 1usize;
    for _ in 0..count {
        let Some(record) = tail.get(offset..offset + stride) else {
            break;
        };
        // Modern records carry six more big-endian u32s after the two names:
        // X, Y, Z, MapID, Description(cliloc), reserved. Legacy (63-byte)
        // records end right after Building, so `location` stays `None`.
        let location = (stride == 89).then(|| {
            let fields = &record[1 + 2 * name_width..];
            let u32_at = |i: usize| {
                u32::from_be_bytes([fields[i], fields[i + 1], fields[i + 2], fields[i + 3]])
            };
            CityLocation {
                x: u32_at(0),
                y: u32_at(4),
                z: u32_at(8) as i32,
                map: u32_at(12),
                description: u32_at(16),
                // fields[20..24] is `reserved` — unused.
            }
        });
        cities.push(StartingCity {
            index: record[0],
            name: trim_fixed_ascii(&record[1..1 + name_width]),
            building: trim_fixed_ascii(&record[1 + name_width..1 + 2 * name_width]),
            location,
        });
        offset += stride;
    }

    let flags = tail
        .get(offset..offset + 4)
        .map(|f| u32::from_be_bytes([f[0], f[1], f[2], f[3]]))
        .unwrap_or(0);
    (cities, flags)
}

/// Trim a fixed-width ASCII field at the first NUL (like
/// `PacketReader::fixed_ascii`) and then trim trailing whitespace, so a
/// zero-filled (unused) city record and a NUL-padded name both come out as
/// clean strings — `""` for the former.
fn trim_fixed_ascii(raw: &[u8]) -> String {
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    raw[..end]
        .iter()
        .map(|&c| c as char)
        .collect::<String>()
        .trim_end()
        .to_string()
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
        character_list_flags: 0, // filled from the preceding 0xA9 by LoginMachine
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
    ///
    /// `address` is what the shard advertised in `0x8C`. Prefer it when
    /// [`GameServerAddress::is_routable`], but keep the endpoint the user
    /// originally dialed as a fallback: NAT'd and tunnelled shards routinely
    /// advertise an address only reachable from inside their own network, and
    /// ClassicUO ships an `IgnoreRelayIp` setting precisely because of that.
    ReconnectToGameServer {
        address: GameServerAddress,
        then: Vec<u8>,
    },
    /// Account authentication succeeded (or a previous `CharacterChoice::Delete`
    /// was rejected by the server). The driver must obtain a user choice and
    /// feed it back through [`LoginMachine::choose_character`].
    ChooseCharacter(CharacterPrompt),
    /// Login finished — we're in the world.
    Done(LoginResult),
}

/// A character-choice prompt handed to the driver: the current slot list,
/// plus — when this prompt is a re-prompt after a rejected delete — why the
/// server said no. ClassicUO parity: a rejected delete is informational, not
/// fatal, so the driver gets the same list back and can choose again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterPrompt {
    pub list: CharacterList,
    /// `Some` only when this prompt follows a `0x85` DeleteResult that
    /// rejected the `CharacterChoice::Delete` the driver most recently made.
    pub delete_rejected: Option<DeleteRejection>,
}

/// Why the server's `0x85` DeleteResult rejected a `CharacterChoice::Delete`.
/// `reason` is the raw `DeleteResultType` byte; `text` is the human-readable
/// gloss from [`delete_result_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteRejection {
    pub reason: u8,
    pub text: &'static str,
}

/// A decision made after inspecting the server-provided character list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterChoice {
    Play(u8),
    Create(CharacterAppearance),
    Delete(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginError {
    /// The server rejected the **account** (`0x82`) — bad password, account in
    /// use, blocked, and so on. `reason` is the raw code; `text` is the gloss
    /// from [`account_denied_text`].
    Denied { reason: u8, text: &'static str },
    /// The server accepted the account but refused to put the **character** in
    /// the world (`0x53`). `reason` is the raw code, `text` the gloss from
    /// [`character_login_rejected_text`], and `retry_minutes` the window a
    /// preceding `0xFD` gave for the two queue-related codes.
    CharacterLoginRejected {
        reason: u8,
        text: &'static str,
        retry_minutes: Option<(u16, u16)>,
    },
    /// A packet ended before we'd read everything its layout requires.
    Truncated(u8),
    /// We reached the character list with no selectable character and automatic
    /// creation was disabled.
    NoCharacterAndCreateUnsupported,
    /// Explicit new-character creation was requested, but every advertised
    /// character slot is already occupied.
    CharacterSlotsFull,
    /// An exact existing-character slot was requested, but that slot is empty
    /// (or outside the slots advertised by the shard).
    CharacterSlotEmpty(u8),
    /// The requested appearance violates client-known creation constraints.
    InvalidCharacterAppearance(&'static str),
    /// A character choice was supplied while the machine was not waiting for one.
    CharacterChoiceNotExpected,
    /// Server rejected our `0x83` DeleteCharacter with DeleteResult `0x85`,
    /// and there's no driver-supplied chooser to re-prompt (the automatic
    /// `LoginConfig::delete_existing` flow, not the interactive one) — so the
    /// delete-then-recreate plan can't proceed and login fails. The
    /// interactive [`LoginDirective::ChooseCharacter`] path never raises this:
    /// it re-prompts via [`CharacterPrompt::delete_rejected`] instead, since a
    /// rejected delete is informational, not fatal (ClassicUO parity).
    /// `reason` is the raw `DeleteResultType` byte; `text` is a human-readable
    /// gloss for logs/UI.
    CharacterDeleteRejected { reason: u8, text: &'static str },
    /// `LoginConfig::server_index` names a shard the server list `0xA8` never
    /// advertised. Sending it anyway is a silent hang — the server just never
    /// answers `0x8C` — so the list we did get comes back with the error.
    /// `available` is `(index, name)` per advertised shard rather than bare
    /// indices, because the only useful thing to do with this error is print
    /// the choices — and the driver that catches it no longer holds the machine
    /// it could ask via [`LoginMachine::servers`].
    ServerIndexUnknown {
        requested: u16,
        available: Vec<(u16, String)>,
    },
    /// Got a packet that doesn't belong in the current state in a way we can't ignore.
    Unexpected { state: &'static str, id: u8 },
}

impl std::fmt::Display for LoginError {
    /// Written for a human staring at a failed login — the browser login page
    /// and the CLI both surface this string verbatim, so every arm has to say
    /// what the *server* refused, not which packet carried the refusal.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoginError::Denied { reason, text } => {
                write!(f, "the server rejected the account: {text} (code {reason})")
            }
            LoginError::CharacterLoginRejected {
                reason,
                text,
                retry_minutes,
            } => {
                write!(f, "the server refused the character: {text}")?;
                if let Some((min, max)) = retry_minutes {
                    write!(f, " (retry in {min}–{max} minutes)")?;
                }
                write!(f, " (code {reason})")
            }
            LoginError::Truncated(id) => {
                write!(f, "packet 0x{id:02X} ended mid-field")
            }
            LoginError::NoCharacterAndCreateUnsupported => {
                write!(f, "the account has no character and creation is disabled")
            }
            LoginError::CharacterSlotsFull => {
                write!(f, "every character slot on the account is occupied")
            }
            LoginError::CharacterSlotEmpty(slot) => {
                write!(f, "character slot {slot} is empty")
            }
            LoginError::InvalidCharacterAppearance(why) => {
                write!(f, "invalid character appearance: {why}")
            }
            LoginError::CharacterChoiceNotExpected => {
                write!(f, "a character choice arrived out of turn")
            }
            LoginError::CharacterDeleteRejected { reason, text } => {
                write!(f, "the server refused the delete: {text} (code {reason})")
            }
            LoginError::ServerIndexUnknown {
                requested,
                available,
            } => {
                write!(f, "this login server has no shard {requested}; it offers")?;
                for (index, name) in available {
                    write!(f, " {index}={name}")?;
                }
                Ok(())
            }
            LoginError::Unexpected { state, id } => {
                write!(f, "packet 0x{id:02X} is not valid while {state}")
            }
        }
    }
}

impl std::error::Error for LoginError {}

/// Maps an account-rejection `0x82` code to a human-readable reason. Order and
/// clamping ported from ClassicUO `Game/Data/ServerErrorMessages.cs`
/// (`_generalErrors`, `GetError` case `0x82`): anything `>= 9` collapses onto
/// the last row rather than reading past the table.
pub fn account_denied_text(reason: u8) -> &'static str {
    match reason {
        0 => "incorrect name or password",
        1 => "someone is already using this account",
        2 => "your account has been blocked",
        3 => "your account credentials are invalid",
        4 => "communication problem",
        5 => "the IGR concurrency limit has been met",
        6 => "the IGR time limit has been met",
        7 => "general IGR authentication failure",
        _ => "couldn't connect to Ultima Online",
    }
}

/// Maps a character-login rejection `0x53` code to a human-readable reason.
/// Ported from ClassicUO `ServerErrorMessages.GetLoginError`. Codes 13 and 14
/// are the login-queue pair whose real text embeds the minute window a
/// preceding `0xFD` supplied — see [`LoginError::CharacterLoginRejected`].
pub fn character_login_rejected_text(reason: u8) -> &'static str {
    match reason {
        0 => "incorrect password",
        1 => "character does not exist",
        2 => "character already exists",
        3 | 4 => "client could not attach to the server",
        5 => "another character on this account is already online",
        6 => "error in synchronization",
        7 => "idle for too long",
        8 => "could not attach to the server",
        9 => "character transfer in progress",
        10 => "name is invalid",
        13 | 14 => "the login queue is full — try again later",
        _ => "unknown login-rejection reason",
    }
}

/// Maps ServUO's `DeleteResultType` (`Server/Network/Packets.cs`) byte order
/// to a human-readable reason. Order verified against ServUO source:
/// `PasswordInvalid=0, CharNotExist=1, CharBeingPlayed=2, CharTooYoung=3,
/// CharQueued=4, BadRequest=5`.
fn delete_result_text(reason: u8) -> &'static str {
    match reason {
        0 => "password invalid",
        1 => "character does not exist",
        2 => "character is currently being played",
        3 => "character is too young to delete",
        4 => "character deletion is queued",
        5 => "bad request",
        _ => "unknown delete-result reason",
    }
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
    /// Require `character_slot` to contain an existing character instead of
    /// falling back to another slot or auto-creating one.
    pub require_character_slot: bool,
    /// Stop after receiving the character list and ask the driver to choose.
    /// Native browser login uses this; CLI and agents keep automatic selection.
    pub defer_character_choice: bool,
    pub client_ip: u32,
    /// When the account has no character, create one from this appearance.
    pub create_if_missing: bool,
    /// Create a new character in the first empty slot even when the account
    /// already has other characters. Existing selection remains the default.
    pub create_new: bool,
    /// Mirrors the Python client's login-flow `delete_existing` option
    /// (`anima/anima/client/connection.py`): once, delete the character that
    /// WOULD have been selected (by `character_slot`, falling back to the
    /// first named slot), then proceed with the refreshed character list
    /// ServUO sends back — normally empty, so `create_if_missing` takes over.
    pub delete_existing: bool,
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
            require_character_slot: false,
            defer_character_choice: false,
            client_ip: 0x7F00_0001, // 127.0.0.1
            create_if_missing: true,
            create_new: false,
            delete_existing: false,
            appearance: CharacterAppearance::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    AwaitServerList,
    AwaitRedirect,
    AwaitCharacterList,
    AwaitCharacterChoice,
    AwaitLoginConfirm,
    Done,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::AwaitServerList => "AwaitServerList",
            State::AwaitRedirect => "AwaitRedirect",
            State::AwaitCharacterList => "AwaitCharacterList",
            State::AwaitCharacterChoice => "AwaitCharacterChoice",
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
    /// Capabilities advertised in the most recent full 0xA9 character list.
    character_list_flags: u32,
    /// Latches once we've sent the one-shot `cfg.delete_existing` DeleteCharacter,
    /// so a subsequent (refreshed) character list is selected/created normally
    /// instead of looping the delete forever.
    delete_sent: bool,
    /// Shards advertised by `0xA8`, in the order the server listed them.
    /// Exposed through [`LoginMachine::servers`] so a driver can show a shard
    /// picker; empty until the list arrives.
    servers: Vec<ShardEntry>,
    /// Login-queue window `(min, max)` in minutes from the most recent `0xFD`,
    /// which a following `0x53` code 13/14 quotes.
    login_delay: Option<(u16, u16)>,
    pending_characters: Option<CharacterList>,
    /// The list shown to the driver when it most recently chose
    /// `CharacterChoice::Delete`. Kept only so a rejected delete (`0x85`) can
    /// re-prompt [`LoginDirective::ChooseCharacter`] with the same slots
    /// instead of failing the whole login. Taken (cleared) by that rejection
    /// handling, or dropped by the next successful character list (the
    /// delete evidently went through, so it's no longer "pending").
    pending_delete_list: Option<CharacterList>,
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
            character_list_flags: 0,
            delete_sent: false,
            servers: Vec::new(),
            login_delay: None,
            pending_characters: None,
            pending_delete_list: None,
        };
        (m, initial)
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// Shards the login server advertised in `0xA8`, in list order. Empty until
    /// that packet arrives (and on shards that send none).
    pub fn servers(&self) -> &[ShardEntry] {
        &self.servers
    }

    /// Resume a deferred login after the driver displayed [`CharacterList`].
    pub fn choose_character(
        &mut self,
        choice: CharacterChoice,
    ) -> Result<Vec<LoginDirective>, LoginError> {
        if self.state != State::AwaitCharacterChoice {
            return Err(LoginError::CharacterChoiceNotExpected);
        }
        let list = self
            .pending_characters
            .take()
            .ok_or(LoginError::CharacterChoiceNotExpected)?;
        match choice {
            CharacterChoice::Play(index) => {
                let slot = list
                    .slots
                    .iter()
                    .find(|slot| slot.index == index)
                    .ok_or(LoginError::CharacterSlotEmpty(index))?;
                self.state = State::AwaitLoginConfirm;
                Ok(vec![LoginDirective::Send(build_play_character(
                    &slot.name,
                    u32::from(slot.index),
                    self.cfg.client_ip,
                    ALL_FACET_CLIENT_FLAGS,
                ))])
            }
            CharacterChoice::Create(appearance) => {
                appearance
                    .validate()
                    .map_err(LoginError::InvalidCharacterAppearance)?;
                let slot = first_empty_slot(&list).ok_or(LoginError::CharacterSlotsFull)?;
                self.state = State::AwaitLoginConfirm;
                Ok(vec![LoginDirective::Send(build_create_character(
                    &appearance,
                    u16::from(slot),
                    ALL_FACET_CLIENT_FLAGS,
                ))])
            }
            CharacterChoice::Delete(index) => {
                let slot_index = list
                    .slots
                    .iter()
                    .find(|slot| slot.index == index)
                    .map(|slot| slot.index)
                    .ok_or(LoginError::CharacterSlotEmpty(index))?;
                self.delete_sent = true;
                self.state = State::AwaitCharacterList;
                // Keep the list around: if the server rejects this delete
                // (`0x85`), we re-prompt with exactly what the driver was
                // just looking at instead of failing the whole login.
                self.pending_delete_list = Some(list);
                Ok(vec![LoginDirective::Send(build_delete_character(
                    u32::from(slot_index),
                    self.cfg.client_ip,
                ))])
            }
        }
    }

    /// Feed one fully-framed packet (id byte included). Returns the directives
    /// to execute, or an error. Packets irrelevant to the current step are
    /// ignored (empty result) so the driver can pass everything through.
    pub fn on_packet(&mut self, frame: &[u8]) -> Result<Vec<LoginDirective>, LoginError> {
        if frame.is_empty() {
            return Ok(vec![]);
        }
        let id = frame[0];

        // Account rejection `0x82` can arrive in either phase.
        if id == 0x82 {
            let reason = frame.get(1).copied().unwrap_or(0);
            return Err(LoginError::Denied {
                reason,
                text: account_denied_text(reason),
            });
        }

        // Character-login rejection `0x53`: the account was fine, the character
        // is not going into the world. Framed but dropped before, so every one
        // of these refusals — "another character is already online" above all —
        // showed up as a login that simply never completed.
        if id == 0x53 {
            let reason = frame.get(1).copied().unwrap_or(0);
            return Err(LoginError::CharacterLoginRejected {
                reason,
                text: character_login_rejected_text(reason),
                // Only the queue codes have a window, and only if the server
                // sent the `0xFD` that carries it.
                retry_minutes: matches!(reason, 13 | 14)
                    .then_some(self.login_delay)
                    .flatten(),
            });
        }

        // `0xFD` LoginDelay: `[id][delay:u8]`, the queue window in units of ten
        // minutes. It always precedes the `0x53` that cites it (ClassicUO
        // `LoginScene.HandleLoginDelayPacket`), so it is stored, not raised.
        if id == 0xFD {
            let delay = u16::from(frame.get(1).copied().unwrap_or(0));
            self.login_delay = Some((delay.saturating_sub(1) * 10, delay * 10));
            return Ok(vec![]);
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
                    self.servers = parse_server_list(frame)?;
                    // `cfg.server_index` names a shard's OWN index, so honour
                    // it when the list contains it. Selecting an index the
                    // shard never advertised is a silent hang (the server
                    // simply never answers 0x8C), so refuse it here — except
                    // for the default 0 against a server that sent no list at
                    // all, which is how this worked before the list was
                    // parsed and is still the right guess.
                    let known = self
                        .servers
                        .iter()
                        .any(|s| s.index == self.cfg.server_index);
                    if !known && !self.servers.is_empty() {
                        return Err(LoginError::ServerIndexUnknown {
                            requested: self.cfg.server_index,
                            available: self
                                .servers
                                .iter()
                                .map(|s| (s.index, s.name.clone()))
                                .collect(),
                        });
                    }
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
                    let (address, auth_key) = parse_server_redirect(frame)?;
                    self.auth_key = auth_key;
                    self.state = State::AwaitCharacterList;
                    let mut then = build_game_seed(self.auth_key);
                    then.extend(build_game_login(
                        self.auth_key,
                        &self.cfg.username,
                        &self.cfg.password,
                    ));
                    Ok(vec![LoginDirective::ReconnectToGameServer {
                        address,
                        then,
                    }])
                } else {
                    Ok(vec![])
                }
            }
            State::AwaitCharacterList => {
                if id == 0xA9 || id == 0x86 {
                    let mut parsed = parse_character_list_with_capacity(frame)?;
                    if id == 0xA9 {
                        self.character_list_flags = parsed.flags;
                    } else {
                        // 0x86 refreshes slots only; retain the capabilities
                        // negotiated by the preceding full 0xA9 list.
                        parsed.flags = self.character_list_flags;
                    }
                    // A real list just arrived, so any delete we were tracking
                    // for a possible rejection evidently went through — drop
                    // it rather than let a later, unrelated `0x85` reuse it.
                    self.pending_delete_list = None;
                    if self.cfg.defer_character_choice {
                        self.state = State::AwaitCharacterChoice;
                        self.pending_characters = Some(parsed.clone());
                        return Ok(vec![LoginDirective::ChooseCharacter(CharacterPrompt {
                            list: parsed,
                            delete_rejected: None,
                        })]);
                    }
                    let preferred = parsed
                        .slots
                        .iter()
                        .find(|s| s.index == self.cfg.character_slot);
                    let chosen = if self.cfg.require_character_slot {
                        preferred
                    } else {
                        preferred.or_else(|| parsed.slots.first())
                    };
                    let first_empty_slot = first_empty_slot(&parsed);
                    match chosen {
                        _ if self.cfg.create_new => {
                            self.cfg
                                .appearance
                                .validate()
                                .map_err(LoginError::InvalidCharacterAppearance)?;
                            let slot = first_empty_slot.ok_or(LoginError::CharacterSlotsFull)?;
                            self.state = State::AwaitLoginConfirm;
                            Ok(vec![LoginDirective::Send(build_create_character(
                                &self.cfg.appearance,
                                u16::from(slot),
                                ALL_FACET_CLIENT_FLAGS,
                            ))])
                        }
                        Some(slot) if self.cfg.delete_existing && !self.delete_sent => {
                            // Python-flow mirror (`anima/anima/client/connection.py`):
                            // delete the character that WOULD have been selected,
                            // once, then keep waiting — ServUO re-sends the
                            // character list (0x86) and we run this selection
                            // again against the refreshed (usually now-empty) list.
                            self.delete_sent = true;
                            Ok(vec![LoginDirective::Send(build_delete_character(
                                slot.index as u32,
                                self.cfg.client_ip,
                            ))])
                        }
                        Some(slot) => {
                            self.state = State::AwaitLoginConfirm;
                            Ok(vec![LoginDirective::Send(build_play_character(
                                &slot.name,
                                slot.index as u32,
                                self.cfg.client_ip,
                                ALL_FACET_CLIENT_FLAGS,
                            ))])
                        }
                        None if self.cfg.require_character_slot => {
                            Err(LoginError::CharacterSlotEmpty(self.cfg.character_slot))
                        }
                        None if self.cfg.create_if_missing => {
                            self.cfg
                                .appearance
                                .validate()
                                .map_err(LoginError::InvalidCharacterAppearance)?;
                            self.state = State::AwaitLoginConfirm;
                            Ok(vec![LoginDirective::Send(build_create_character(
                                &self.cfg.appearance,
                                u16::from(first_empty_slot.unwrap_or(0)),
                                ALL_FACET_CLIENT_FLAGS,
                            ))])
                        }
                        None => Err(LoginError::NoCharacterAndCreateUnsupported),
                    }
                } else if id == 0x85 && self.delete_sent {
                    // DeleteResult: our 0x83 DeleteCharacter was rejected. This is a
                    // normal server answer — the account simply still has the
                    // character we tried to remove — NOT a login failure: ClassicUO
                    // reports the reason and lets the user pick again from the same
                    // character list, so a rejection must not tear down the session.
                    // Gated on `delete_sent`: a 0x85 we never solicited (stray proxy
                    // echo, odd shard) must stay ignorable chatter on the default
                    // path, exactly as before.
                    self.delete_sent = false;
                    let reason = frame.get(1).copied().unwrap_or(0);
                    let rejection = DeleteRejection {
                        reason,
                        text: delete_result_text(reason),
                    };
                    match self.pending_delete_list.take() {
                        // Interactive (chooser-driven) flow: re-prompt with the
                        // same list the driver was showing when it chose
                        // Delete, so it can surface the reason and let the
                        // user choose again instead of the session dying.
                        Some(list) => {
                            self.state = State::AwaitCharacterChoice;
                            self.pending_characters = Some(list.clone());
                            Ok(vec![LoginDirective::ChooseCharacter(CharacterPrompt {
                                list,
                                delete_rejected: Some(rejection),
                            })])
                        }
                        // Automatic (`LoginConfig::delete_existing`) flow has no
                        // driver to re-prompt — surface the rejection as a
                        // login failure, as before.
                        None => Err(LoginError::CharacterDeleteRejected {
                            reason,
                            text: rejection.text,
                        }),
                    }
                } else {
                    Ok(vec![]) // e.g. 0xB9 SupportedFeatures, 0xBD version req, etc.
                }
            }
            State::AwaitCharacterChoice => Ok(vec![]),
            State::AwaitLoginConfirm => {
                if id == 0x1B {
                    let mut result = parse_login_confirm(frame)?;
                    result.aos = self.aos;
                    result.character_list_flags = self.character_list_flags;
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

fn first_empty_slot(list: &CharacterList) -> Option<u8> {
    (0..list.slot_count)
        .find(|index| !list.slots.iter().any(|slot| slot.index == *index))
        // Some older shards advertise zero entries for a fresh account instead
        // of a fixed bank of empty slots.
        .or((list.slot_count == 0).then_some(0))
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
        assert_eq!(
            build_play_character("Anima", 0, 0x7F00_0001, 0x3F).len(),
            73
        );
        assert_eq!(build_delete_character(0, 0x7F00_0001).len(), 39);
    }

    #[test]
    fn delete_character_layout() {
        // 0x83, 30 zero bytes, then slot:u32 and clientIP:u32, big-endian.
        let p = build_delete_character(3, 0x7F00_0001);
        assert_eq!(p.len(), 39);
        assert_eq!(p[0], 0x83);
        assert!(p[1..31].iter().all(|&b| b == 0)); // reserved — NOT the password
        assert_eq!(u32::from_be_bytes([p[31], p[32], p[33], p[34]]), 3);
        assert_eq!(
            u32::from_be_bytes([p[35], p[36], p[37], p[38]]),
            0x7F00_0001
        );
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
        // ServerRedirect 0x8C: id, ip(4), port(2), auth(4) = 11 bytes.
        // 0x0A21 = 2593. Unlike 0xA8's, this address is NOT byte-reversed
        // (ServUO writes it low byte first, ClassicUO reads it as UInt32LE).
        let frame = [0x8C, 1, 2, 3, 4, 0x0A, 0x21, 0xDE, 0xAD, 0xBE, 0xEF];
        let (addr, auth) = parse_server_redirect(&frame).unwrap();
        assert_eq!(auth, 0xDEAD_BEEF);
        assert_eq!(addr.ip, [1, 2, 3, 4]);
        assert_eq!(addr.port, 2593);
        assert_eq!(addr.host(), "1.2.3.4");
        assert!(addr.is_routable());

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
                character_list_flags: 0,
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

        // ServerList 0xA8: [id][len:u16][flag][count:u16] and no records — the
        // shape a shard that advertises nothing sends. Selection falls back to
        // `cfg.server_index`, which is what happened before the list was
        // parsed at all.
        let d = m.on_packet(&empty_server_list()).unwrap();
        assert!(m.servers().is_empty());
        assert_eq!(d, vec![LoginDirective::Send(build_server_select(0))]);

        // ServerRedirect 0x8C → reconnect + game seed/login.
        let redirect = [0x8C, 127, 0, 0, 1, 0x0A, 0x21, 0x11, 0x22, 0x33, 0x44];
        let d = m.on_packet(&redirect).unwrap();
        match &d[0] {
            LoginDirective::ReconnectToGameServer { address, then } => {
                assert_eq!(&then[0..4], &[0x11, 0x22, 0x33, 0x44]); // game seed = auth key
                assert_eq!(then[4], 0x91); // GameLogin follows
                assert_eq!(address.host(), "127.0.0.1");
                assert_eq!(address.port, 2593);
            }
            other => panic!("expected reconnect, got {other:?}"),
        }

        // An ignorable phase-2 packet (SupportedFeatures 0xB9) before the list.
        assert_eq!(m.on_packet(&[0xB9, 0, 0, 0, 0]).unwrap(), vec![]);

        // CharacterList 0xA9: one char "Anima" in slot 0 and the negotiated
        // server-authorized logout capability.
        let char_list = append_character_list_tail(
            build_character_list_frame(0xA9, &["Anima"]),
            89,
            CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE,
            true,
        );
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
        w.u8(0x1B)
            .u32(0x2A)
            .u32(0)
            .u16(400)
            .u16(1000)
            .u16(2000)
            .u16(0)
            .u8(0)
            .zeros(17);
        let confirm = w.into_vec();
        let d = m.on_packet(&confirm).unwrap();
        let LoginDirective::Done(result) = &d[0] else {
            panic!("expected completed login, got {:?}", d[0]);
        };
        assert_eq!(
            result.character_list_flags,
            CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE
        );
        assert!(m.is_done());
    }

    #[test]
    fn login_denied_errors() {
        let cfg = LoginConfig::default();
        let (mut m, _) = LoginMachine::start(cfg);
        assert_eq!(
            m.on_packet(&[0x82, 0x03]),
            Err(LoginError::Denied {
                reason: 3,
                text: "your account credentials are invalid",
            })
        );
    }

    /// A `0xA8` advertising no shards: `[id][len:u16][flag][count=0]`.
    fn empty_server_list() -> Vec<u8> {
        vec![0xA8, 0x00, 0x06, 0x5D, 0x00, 0x00]
    }

    /// A `0xA8` carrying `(index, name, ip)` records in ServUO's layout. The
    /// address goes out **reversed** — see [`parse_server_list`].
    fn server_list(entries: &[(u16, &str, [u8; 4])]) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.u8(0xA8)
            .u16((6 + entries.len() * 40) as u16)
            .u8(0x5D)
            .u16(entries.len() as u16);
        for (index, name, ip) in entries {
            w.u16(*index)
                .fixed_ascii(name, 32)
                .u8(50) // percent full
                .u8(0xFB) // timezone -5
                .bytes(&[ip[3], ip[2], ip[1], ip[0]]);
        }
        w.into_vec()
    }

    #[test]
    fn server_list_parses_and_unreverses_the_address() {
        let frame = server_list(&[(0, "Anima", [127, 0, 0, 1]), (7, "Second", [10, 0, 1, 5])]);
        let servers = parse_server_list(&frame).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "Anima");
        assert_eq!(servers[0].ip, [127, 0, 0, 1]);
        assert_eq!(servers[0].percent_full, 50);
        assert_eq!(servers[0].timezone, -5);
        // The shard's own index, not the row position — 0xA0 must echo THIS.
        assert_eq!(servers[1].index, 7);
        assert_eq!(servers[1].ip, [10, 0, 1, 5]);
    }

    #[test]
    fn server_list_is_exposed_and_selection_uses_the_shard_index() {
        let cfg = LoginConfig {
            server_index: 7,
            ..Default::default()
        };
        let (mut m, _initial) = LoginMachine::start(cfg);
        let frame = server_list(&[(0, "First", [127, 0, 0, 1]), (7, "Second", [10, 0, 1, 5])]);
        let d = m.on_packet(&frame).unwrap();
        assert_eq!(d, vec![LoginDirective::Send(build_server_select(7))]);
        assert_eq!(m.servers().len(), 2);
        assert_eq!(m.servers()[1].name, "Second");
    }

    #[test]
    fn selecting_a_shard_the_server_never_listed_fails_loudly() {
        // Silence here means the server simply never answers 0x8C and login
        // hangs forever, which is what made this worth an error at all.
        let cfg = LoginConfig {
            server_index: 3,
            ..Default::default()
        };
        let (mut m, _initial) = LoginMachine::start(cfg);
        let frame = server_list(&[(0, "First", [127, 0, 0, 1]), (7, "Second", [10, 0, 1, 5])]);
        assert_eq!(
            m.on_packet(&frame),
            Err(LoginError::ServerIndexUnknown {
                requested: 3,
                available: vec![(0, "First".into()), (7, "Second".into())],
            })
        );
    }

    #[test]
    fn a_character_login_rejection_names_its_reason() {
        let cfg = LoginConfig::default();
        let (mut m, _) = LoginMachine::start(cfg);
        assert_eq!(
            m.on_packet(&[0x53, 0x05]),
            Err(LoginError::CharacterLoginRejected {
                reason: 5,
                text: "another character on this account is already online",
                retry_minutes: None,
            })
        );
    }

    #[test]
    fn a_queue_rejection_carries_the_window_the_preceding_0xfd_gave() {
        let cfg = LoginConfig::default();
        let (mut m, _) = LoginMachine::start(cfg);
        // 0xFD is informational on its own — it must not fail the login.
        assert_eq!(m.on_packet(&[0xFD, 3]).unwrap(), vec![]);
        assert_eq!(
            m.on_packet(&[0x53, 13]),
            Err(LoginError::CharacterLoginRejected {
                reason: 13,
                text: "the login queue is full — try again later",
                retry_minutes: Some((20, 30)),
            })
        );
    }

    #[test]
    fn an_unroutable_advertised_address_is_reported_as_such() {
        // ClassicUO's `ip == 0` case: the shard is telling us to keep using
        // the address we already have.
        let frame = [0x8C, 0, 0, 0, 0, 0x0A, 0x21, 0x11, 0x22, 0x33, 0x44];
        let (addr, _) = parse_server_redirect(&frame).unwrap();
        assert!(!addr.is_routable());
        // …and so is a real address with no port to dial.
        let frame = [0x8C, 10, 0, 0, 5, 0x00, 0x00, 0x11, 0x22, 0x33, 0x44];
        let (addr, _) = parse_server_redirect(&frame).unwrap();
        assert!(!addr.is_routable());
    }

    /// Drives past phase 1 into `AwaitCharacterList` and returns the machine.
    fn machine_at_character_list(cfg: LoginConfig) -> LoginMachine {
        let (mut m, _initial) = LoginMachine::start(cfg);
        m.on_packet(&empty_server_list()).unwrap();
        let redirect = [0x8C, 127, 0, 0, 1, 0x0A, 0x21, 0x11, 0x22, 0x33, 0x44];
        m.on_packet(&redirect).unwrap();
        m
    }

    /// Builds a well-formed CharacterList frame (`0xA9`/`0x86`) for the given
    /// (index-order) names; empty names are skipped in the parsed result but
    /// still occupy a slot on the wire, matching real server frames.
    fn build_character_list_frame(id: u8, names: &[&str]) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.u8(id).u16(0).u8(names.len() as u8);
        for name in names {
            w.fixed_ascii(name, 30).zeros(30);
        }
        let mut frame = w.into_vec();
        let total = frame.len() as u16;
        frame[1] = (total >> 8) as u8;
        frame[2] = (total & 0xFF) as u8;
        frame
    }

    /// Adds one zero-filled city record, flags, and the modern-only trailer to
    /// a full 0xA9 character list, then fixes its variable packet length.
    fn append_character_list_tail(
        mut frame: Vec<u8>,
        city_stride: usize,
        flags: u32,
        modern_trailer: bool,
    ) -> Vec<u8> {
        frame.push(1);
        frame.resize(frame.len() + city_stride, 0);
        frame.extend_from_slice(&flags.to_be_bytes());
        if modern_trailer {
            frame.extend_from_slice(&u16::MAX.to_be_bytes());
        }
        let total = frame.len() as u16;
        frame[1..3].copy_from_slice(&total.to_be_bytes());
        frame
    }

    #[test]
    fn character_list_parses_modern_and_legacy_capability_flags() {
        for (city_stride, modern_trailer) in [(89, true), (63, false)] {
            let frame = append_character_list_tail(
                build_character_list_frame(0xA9, &["Anima"]),
                city_stride,
                CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE,
                modern_trailer,
            );
            let parsed = parse_character_list_with_capacity(&frame).unwrap();
            assert_eq!(parsed.slots[0].name, "Anima");
            assert_eq!(
                parsed.flags, CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE,
                "city stride {city_stride}"
            );
        }
    }

    /// Builds one modern (89-byte) city record: `[index][City:32][Building:32]`
    /// then the location fields `parse_character_list_tail` decodes into
    /// `StartingCity::location` (X/Y/Z/MapID/Description-cliloc), plus 4
    /// bytes of unused `reserved`.
    #[allow(clippy::too_many_arguments)]
    fn modern_city_record(
        index: u8,
        city: &str,
        building: &str,
        x: u32,
        y: u32,
        z: i32,
        map: u32,
        description: u32,
    ) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.u8(index)
            .fixed_ascii(city, 32)
            .fixed_ascii(building, 32)
            .u32(x)
            .u32(y)
            .u32(z as u32)
            .u32(map)
            .u32(description)
            .zeros(4); // reserved
        let record = w.into_vec();
        assert_eq!(record.len(), 89);
        record
    }

    /// Builds one legacy (63-byte) city record: `[index][City:31][Building:31]`.
    fn legacy_city_record(index: u8, city: &str, building: &str) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.u8(index).fixed_ascii(city, 31).fixed_ascii(building, 31);
        let record = w.into_vec();
        assert_eq!(record.len(), 63);
        record
    }

    #[test]
    fn character_list_parses_modern_named_cities() {
        // Three named cities, indices out of order (as a real shard's list is),
        // proving `index` is read from the record and not assumed to be its
        // position — that index is what CreateCharacter 0xF8 must echo back.
        let mut frame = build_character_list_frame(0xA9, &["Anima"]);
        frame.push(3);
        // Real-ish coordinates/facet/cliloc so the round-trip assertion below
        // is meaningful — 1075074 is the actual Britain city-blurb cliloc
        // ("<h2>Britain</h2><br>The City of Bards<br><br> ...").
        frame.extend(modern_city_record(
            0,
            "New Haven",
            "New Haven Bank",
            3494,
            2559,
            30,
            1,
            1075072,
        ));
        frame.extend(modern_city_record(
            3,
            "Britain",
            "Britain Bank",
            3734,
            2222,
            20,
            0,
            1075074,
        ));
        frame.extend(modern_city_record(
            4,
            "Moonglow",
            "Moonglow Bank",
            4408,
            1173,
            0,
            0,
            1075076,
        ));
        frame.extend_from_slice(&CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE.to_be_bytes());
        let total = frame.len() as u16;
        frame[1..3].copy_from_slice(&total.to_be_bytes());

        let parsed = parse_character_list_with_capacity(&frame).unwrap();
        assert_eq!(parsed.flags, CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE);
        assert_eq!(
            parsed.cities,
            vec![
                StartingCity {
                    index: 0,
                    name: "New Haven".into(),
                    building: "New Haven Bank".into(),
                    location: Some(CityLocation {
                        x: 3494,
                        y: 2559,
                        z: 30,
                        map: 1,
                        description: 1075072,
                    }),
                },
                StartingCity {
                    index: 3,
                    name: "Britain".into(),
                    building: "Britain Bank".into(),
                    location: Some(CityLocation {
                        x: 3734,
                        y: 2222,
                        z: 20,
                        map: 0,
                        description: 1075074,
                    }),
                },
                StartingCity {
                    index: 4,
                    name: "Moonglow".into(),
                    building: "Moonglow Bank".into(),
                    location: Some(CityLocation {
                        x: 4408,
                        y: 1173,
                        z: 0,
                        map: 0,
                        description: 1075076,
                    }),
                },
            ]
        );
    }

    #[test]
    fn character_list_parses_legacy_named_cities() {
        let mut frame = build_character_list_frame(0xA9, &["Anima"]);
        frame.push(2);
        frame.extend(legacy_city_record(0, "Yew", "Yew Abbey"));
        frame.extend(legacy_city_record(1, "Minoc", "Minoc Bank"));
        frame.extend_from_slice(&0u32.to_be_bytes());
        let total = frame.len() as u16;
        frame[1..3].copy_from_slice(&total.to_be_bytes());

        let parsed = parse_character_list_with_capacity(&frame).unwrap();
        assert_eq!(parsed.flags, 0);
        assert_eq!(
            parsed.cities,
            vec![
                StartingCity {
                    index: 0,
                    name: "Yew".into(),
                    building: "Yew Abbey".into(),
                    location: None, // legacy (63-byte) record: no location fields
                },
                StartingCity {
                    index: 1,
                    name: "Minoc".into(),
                    building: "Minoc Bank".into(),
                    location: None,
                },
            ]
        );
    }

    #[test]
    fn delete_existing_sends_delete_then_awaits_refresh() {
        let cfg = LoginConfig {
            username: "test5".into(),
            password: "test5".into(),
            delete_existing: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);

        // One character "Anima" in slot 0 — the one that would have been
        // selected, so it's the one we delete.
        let char_list = build_character_list_frame(0xA9, &["Anima"]);
        let d = m.on_packet(&char_list).unwrap();
        assert_eq!(
            d,
            vec![LoginDirective::Send(build_delete_character(0, 0x7F00_0001))]
        );
        assert!(!m.is_done()); // stayed in AwaitCharacterList, waiting for the resend

        // ServUO re-sends the character list after the delete — now empty —
        // and create_if_missing (the default) kicks in.
        let empty_list = build_character_list_frame(0x86, &[]);
        let d = m.on_packet(&empty_list).unwrap();
        assert_eq!(
            d,
            vec![LoginDirective::Send(build_create_character(
                &CharacterAppearance::default(),
                0,
                ALL_FACET_CLIENT_FLAGS,
            ))]
        );
    }

    #[test]
    fn delete_result_rejected_fails_login() {
        let cfg = LoginConfig {
            delete_existing: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);

        // An UNSOLICITED 0x85 (we haven't sent 0x83 yet) is ignorable chatter —
        // the default path never hard-fails on a stray DeleteResult.
        assert_eq!(m.on_packet(&[0x85, 2]).unwrap(), vec![]);

        // Drive the realistic sequence: the char list makes the machine send its
        // 0x83 delete; ONLY THEN does a DeleteResult mean our delete was rejected.
        // Reason=2 = CharBeingPlayed in ServUO's DeleteResultType.
        m.on_packet(&build_character_list_frame(0xA9, &["Anima"]))
            .unwrap();
        let err = m.on_packet(&[0x85, 2]).unwrap_err();
        assert_eq!(
            err,
            LoginError::CharacterDeleteRejected {
                reason: 2,
                text: "character is currently being played",
            }
        );
    }

    #[test]
    fn delete_existing_false_leaves_selection_untouched() {
        // Default config (delete_existing = false) must behave exactly like
        // before: the character list resolves straight to PlayCharacter, no
        // DeleteCharacter ever sent.
        let cfg = LoginConfig {
            username: "test5".into(),
            password: "test5".into(),
            ..Default::default()
        };
        assert!(!cfg.delete_existing);
        let mut m = machine_at_character_list(cfg);

        let char_list = build_character_list_frame(0xA9, &["Anima"]);
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
    }

    #[test]
    fn explicit_creation_uses_first_empty_slot_without_deleting_existing() {
        let appearance = CharacterAppearance {
            name: "Second Hero".into(),
            ..Default::default()
        };
        let cfg = LoginConfig {
            create_new: true,
            appearance: appearance.clone(),
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);

        let list = build_character_list_frame(0xA9, &["Existing", "", "Other", "", ""]);
        let directives = m.on_packet(&list).unwrap();
        assert_eq!(
            directives,
            vec![LoginDirective::Send(build_create_character(
                &appearance,
                1,
                ALL_FACET_CLIENT_FLAGS,
            ))]
        );
    }

    #[test]
    fn explicit_creation_rejects_a_full_account() {
        let cfg = LoginConfig {
            create_new: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);
        let list = build_character_list_frame(0xA9, &["A", "B", "C", "D", "E"]);
        assert_eq!(m.on_packet(&list), Err(LoginError::CharacterSlotsFull));
    }

    #[test]
    fn exact_character_selection_plays_the_requested_slot() {
        let cfg = LoginConfig {
            character_slot: 2,
            require_character_slot: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);
        let list = build_character_list_frame(0xA9, &["First", "", "Third", "", ""]);
        assert_eq!(
            m.on_packet(&list).unwrap(),
            vec![LoginDirective::Send(build_play_character(
                "Third",
                2,
                0x7F00_0001,
                ALL_FACET_CLIENT_FLAGS,
            ))]
        );
    }

    #[test]
    fn exact_character_selection_rejects_an_empty_slot_without_fallback() {
        let cfg = LoginConfig {
            character_slot: 1,
            require_character_slot: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);
        let list = build_character_list_frame(0xA9, &["First", "", "Third", "", ""]);
        assert_eq!(m.on_packet(&list), Err(LoginError::CharacterSlotEmpty(1)));
    }

    #[test]
    fn character_appearance_validation_catches_bad_stats() {
        let appearance = CharacterAppearance {
            strength: 60,
            dexterity: 30,
            intelligence: 30,
            ..Default::default()
        };
        assert_eq!(
            appearance.validate(),
            Err("strength, dexterity, and intelligence must each be 10-60 and total 90")
        );
    }

    #[test]
    fn deferred_character_list_waits_for_and_plays_the_user_choice() {
        let cfg = LoginConfig {
            defer_character_choice: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);
        let list = build_character_list_frame(0xA9, &["First", "", "Third", "", ""]);
        assert_eq!(
            m.on_packet(&list).unwrap(),
            vec![LoginDirective::ChooseCharacter(CharacterPrompt {
                list: CharacterList {
                    slots: vec![
                        CharSlot {
                            index: 0,
                            name: "First".into(),
                        },
                        CharSlot {
                            index: 2,
                            name: "Third".into(),
                        },
                    ],
                    slot_count: 5,
                    cities: vec![],
                    flags: 0,
                },
                delete_rejected: None,
            })]
        );
        assert_eq!(
            m.choose_character(CharacterChoice::Play(2)).unwrap(),
            vec![LoginDirective::Send(build_play_character(
                "Third",
                2,
                0x7F00_0001,
                ALL_FACET_CLIENT_FLAGS,
            ))]
        );
    }

    #[test]
    fn deferred_character_list_creates_in_the_first_empty_slot() {
        let cfg = LoginConfig {
            defer_character_choice: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);
        let list = build_character_list_frame(0xA9, &["First", "", "Third", "", ""]);
        m.on_packet(&list).unwrap();
        let appearance = CharacterAppearance {
            name: "New Hero".into(),
            ..Default::default()
        };
        assert_eq!(
            m.choose_character(CharacterChoice::Create(appearance.clone()))
                .unwrap(),
            vec![LoginDirective::Send(build_create_character(
                &appearance,
                1,
                ALL_FACET_CLIENT_FLAGS,
            ))]
        );
    }

    #[test]
    fn deferred_character_list_deletes_then_displays_the_refreshed_list() {
        let cfg = LoginConfig {
            defer_character_choice: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);
        let list = append_character_list_tail(
            build_character_list_frame(0xA9, &["First", "", "Third", "", ""]),
            89,
            CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE,
            true,
        );
        m.on_packet(&list).unwrap();
        let expected = build_delete_character(2, 0x7F00_0001);
        assert_eq!(
            m.choose_character(CharacterChoice::Delete(2)).unwrap(),
            vec![LoginDirective::Send(expected)]
        );

        let refreshed = build_character_list_frame(0x86, &["First", "", "", "", ""]);
        assert_eq!(
            m.on_packet(&refreshed).unwrap(),
            vec![LoginDirective::ChooseCharacter(CharacterPrompt {
                list: CharacterList {
                    slots: vec![CharSlot {
                        index: 0,
                        name: "First".into(),
                    }],
                    slot_count: 5,
                    cities: vec![],
                    flags: CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE,
                },
                delete_rejected: None,
            })]
        );
    }

    #[test]
    fn deferred_delete_rejection_reprompts_instead_of_failing_login() {
        // The exact bug this guards: a rejected delete (e.g. ServUO's 7-day
        // `Accounts.DeleteDelay`, reason 3 "too young to delete") must NOT
        // tear down the login — it must land back in the character-choice
        // state with the same list, so the driver can show the reason and
        // let the user pick again.
        let cfg = LoginConfig {
            defer_character_choice: true,
            ..Default::default()
        };
        let mut m = machine_at_character_list(cfg);
        let list = build_character_list_frame(0xA9, &["Anima", "", "", "", ""]);
        let prompt_before = m.on_packet(&list).unwrap();
        let LoginDirective::ChooseCharacter(CharacterPrompt {
            list: shown_list, ..
        }) = &prompt_before[0]
        else {
            panic!("expected a character prompt, got {:?}", prompt_before[0]);
        };
        let shown_list = shown_list.clone();

        assert_eq!(
            m.choose_character(CharacterChoice::Delete(0)).unwrap(),
            vec![LoginDirective::Send(build_delete_character(0, 0x7F00_0001))]
        );

        // Reason 3 = CharTooYoung in ServUO's DeleteResultType.
        let directives = m.on_packet(&[0x85, 3]).unwrap();
        assert_eq!(
            directives,
            vec![LoginDirective::ChooseCharacter(CharacterPrompt {
                list: shown_list,
                delete_rejected: Some(DeleteRejection {
                    reason: 3,
                    text: "character is too young to delete",
                }),
            })]
        );

        // The machine must still be able to accept another character choice
        // (not stuck, not erroring) — e.g. play the character we tried (and
        // failed) to delete.
        assert_eq!(
            m.choose_character(CharacterChoice::Play(0)).unwrap(),
            vec![LoginDirective::Send(build_play_character(
                "Anima",
                0,
                0x7F00_0001,
                ALL_FACET_CLIENT_FLAGS,
            ))]
        );
    }

    #[test]
    fn character_choice_is_rejected_outside_the_deferred_state() {
        let (mut m, _) = LoginMachine::start(LoginConfig::default());
        assert_eq!(
            m.choose_character(CharacterChoice::Play(0)),
            Err(LoginError::CharacterChoiceNotExpected)
        );
    }

    #[test]
    fn character_name_validation_matches_servuo_creation_rules() {
        for valid in ["Iron Warden", "O'Neil", "Anne-Marie", "A.B"] {
            assert_eq!(validate_character_name(valid), Ok(()), "{valid}");
        }
        for invalid in [
            "A",
            "This Name Is Too Long",
            "Forge Master",
            "GM Helper",
            "Hero42",
            "Two  Spaces",
        ] {
            assert!(validate_character_name(invalid).is_err(), "{invalid}");
        }
    }
}

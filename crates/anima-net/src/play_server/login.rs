//! Parsing what the browser login page sends back.
//!
//! The account form, the character choice, and the character-creation payload —
//! each of which arrives as JSON from a page the user is looking at, so every
//! field is validated here rather than trusted downstream.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoginAttempt {
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) username: String,
    pub(super) password: String,
    /// Exact existing slot to play. `None` preserves the historical
    /// preferred-slot-with-fallback behavior used by legacy scripts.
    pub(super) character_slot: Option<u8>,
    /// Pause after account authentication and expose the server's character
    /// list through `scene.json` before entering the world.
    pub(super) interactive: bool,
    /// `Some` means explicitly create a new character, even if other slots
    /// are occupied. `None` keeps the existing select-or-create-if-empty flow.
    pub(super) create: Option<CharacterAppearance>,
    /// Which shard from the login server's `0xA8` list to enter, by the
    /// shard's own index. Defaults to 0 — the only value that worked before
    /// the list was parsed, and still the right answer for the single-shard
    /// ServUO this is usually pointed at. A wrong one now fails with the
    /// available shards named rather than hanging.
    pub(super) shard: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CharacterDecision {
    Choose(CharacterChoice),
    Cancel,
}

pub(super) fn starting_skills(profession: &str) -> Option<[(u8, u8); 4]> {
    Some(match profession {
        "warrior" => [(40, 50), (27, 50), (0, 0), (0, 0)], // swords + tactics
        "mage" => [(25, 50), (16, 50), (0, 0), (0, 0)],    // magery + eval int
        "ranger" => [(31, 50), (27, 50), (0, 0), (0, 0)],  // archery + tactics
        "crafter" => [(7, 50), (45, 50), (0, 0), (0, 0)],  // smithing + mining
        _ => return None,
    })
}

/// Parse the browser's JSON login request. The legacy colon-separated body is
/// retained for scripts and older embedded web assets.
pub(super) fn parse_login_attempt(body: &str) -> Result<LoginAttempt, &'static str> {
    let body = body.trim();
    if !body.starts_with('{') {
        let mut fields = body.splitn(4, ':');
        let host = fields.next().unwrap_or("").trim().to_string();
        let port = fields
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2593);
        let username = fields.next().unwrap_or("").trim().to_string();
        let password = fields.next().unwrap_or("").to_string();
        if host.is_empty() || username.is_empty() {
            return Err("server and account are required");
        }
        return Ok(LoginAttempt {
            host,
            port,
            username,
            password,
            character_slot: None,
            interactive: false,
            create: None,
            shard: 0,
        });
    }

    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| "invalid login JSON")?;
    let text = |key| {
        value
            .get(key)
            .and_then(|field| field.as_str())
            .unwrap_or("")
    };
    let host = text("host").trim().to_string();
    let username = text("username").trim().to_string();
    if host.is_empty() || username.is_empty() {
        return Err("server and account are required");
    }
    let port = value
        .get("port")
        .and_then(|field| field.as_u64())
        .and_then(|port| u16::try_from(port).ok())
        .ok_or("port must be between 0 and 65535")?;
    let character_slot = match value.get("character_slot") {
        Some(field) if !field.is_null() => Some(
            field
                .as_u64()
                .and_then(|slot| u8::try_from(slot).ok())
                .ok_or("character slot must be a non-negative integer")?,
        ),
        _ => None,
    };
    let interactive = value
        .get("interactive")
        .and_then(|field| field.as_bool())
        .unwrap_or(false);
    let shard = match value.get("shard") {
        Some(field) if !field.is_null() => field
            .as_u64()
            .and_then(|shard| u16::try_from(shard).ok())
            .ok_or("shard must be between 0 and 65535")?,
        _ => 0,
    };

    let create = value.get("create").filter(|field| !field.is_null());
    let create = if let Some(create) = create {
        Some(parse_character_appearance(create)?)
    } else {
        None
    };

    Ok(LoginAttempt {
        host,
        port,
        username,
        password: text("password").to_string(),
        character_slot,
        interactive,
        create,
        shard,
    })
}

pub(super) fn login_attempt_expected(scene: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(scene)
        .ok()
        .and_then(|value| value.get("auth")?.as_str().map(str::to_owned))
        .is_some_and(|auth| matches!(auth.as_str(), "login" | "error"))
}

pub(super) fn parse_character_appearance(
    create: &serde_json::Value,
) -> Result<CharacterAppearance, &'static str> {
    let text = |key| {
        create
            .get(key)
            .and_then(|field| field.as_str())
            .unwrap_or("")
    };
    let u8num = |key| {
        create
            .get(key)
            .and_then(|field| field.as_u64())
            .and_then(|number| u8::try_from(number).ok())
    };
    // Appearance hues/styles are optional; a missing field means "server default"
    // (0), exactly as the previous `..Default::default()` behavior.
    let u16opt = |key| {
        create
            .get(key)
            .and_then(|field| field.as_u64())
            .map(|number| u16::try_from(number).ok().ok_or("value out of range"))
            .transpose()
    };
    let mut appearance = CharacterAppearance {
        name: text("name").trim().to_string(),
        female: create
            .get("female")
            .and_then(|field| field.as_bool())
            .unwrap_or(false),
        skin_hue: u16opt("skin_hue")?.unwrap_or(0),
        hair_style: u16opt("hair_style")?.unwrap_or(0),
        hair_hue: u16opt("hair_hue")?.unwrap_or(0),
        facial_hair_style: u16opt("facial_hair_style")?.unwrap_or(0),
        facial_hair_hue: u16opt("facial_hair_hue")?.unwrap_or(0),
        shirt_hue: u16opt("shirt_hue")?.unwrap_or(0),
        pants_hue: u16opt("pants_hue")?.unwrap_or(0),
        strength: u8num("strength").ok_or("invalid strength")?,
        dexterity: u8num("dexterity").ok_or("invalid dexterity")?,
        intelligence: u8num("intelligence").ok_or("invalid intelligence")?,
        city_index: create
            .get("city_index")
            .and_then(|field| field.as_u64())
            .and_then(|number| u16::try_from(number).ok())
            .ok_or("invalid starting city")?,
        skills: [(0, 0); 4],
    };
    // An explicit `skills` array (the custom picker) wins; otherwise fall back to
    // the named profession's preset. `validate()` enforces the UO rules either way
    // (each value <= 50, unique non-zero, total 100 or 120).
    appearance.skills = match create.get("skills").and_then(|field| field.as_array()) {
        Some(list) => parse_skill_choices(list)?,
        None => starting_skills(text("profession")).ok_or("unknown starting profession")?,
    };
    appearance.validate()?;
    Ok(appearance)
}

/// Parse the custom skill picker's JSON (`[{"id":N,"value":M}, …]`, up to 4)
/// into the fixed 4-slot array `build_create_character` expects, zero-padded.
pub(super) fn parse_skill_choices(
    list: &[serde_json::Value],
) -> Result<[(u8, u8); 4], &'static str> {
    if list.len() > 4 {
        return Err("at most 4 starting skills");
    }
    let mut skills = [(0u8, 0u8); 4];
    for (slot, item) in list.iter().enumerate() {
        let field = |key| {
            item.get(key)
                .and_then(|field| field.as_u64())
                .and_then(|number| u8::try_from(number).ok())
        };
        skills[slot] = (
            field("id").ok_or("invalid skill id")?,
            field("value").ok_or("invalid skill value")?,
        );
    }
    Ok(skills)
}

pub(super) fn parse_character_choice(body: &str) -> Result<CharacterDecision, &'static str> {
    let value: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|_| "invalid character-choice JSON")?;
    let create = value.get("create").filter(|field| !field.is_null());
    let play_slot = value
        .get("slot")
        .and_then(|field| field.as_u64())
        .and_then(|slot| u8::try_from(slot).ok());
    let delete_slot = value
        .get("delete_slot")
        .and_then(|field| field.as_u64())
        .and_then(|slot| u8::try_from(slot).ok());
    let cancel = value
        .get("cancel")
        .and_then(|field| field.as_bool())
        .unwrap_or(false);
    match (create, play_slot, delete_slot, cancel) {
        (Some(appearance), None, None, false) => Ok(CharacterDecision::Choose(
            CharacterChoice::Create(parse_character_appearance(appearance)?),
        )),
        (None, Some(slot), None, false) => {
            Ok(CharacterDecision::Choose(CharacterChoice::Play(slot)))
        }
        (None, None, Some(slot), false) => {
            Ok(CharacterDecision::Choose(CharacterChoice::Delete(slot)))
        }
        (None, None, None, true) => Ok(CharacterDecision::Cancel),
        _ => Err("choose exactly one of create, slot, delete_slot, or cancel"),
    }
}

/// Convert a cliloc city blurb (e.g. 1075074 for Britain:
/// `"<h2>Britain</h2><br>The City of Bards<br><br> The thriving city..."`)
/// into plain text for the browser's character-creation city picker — this is
/// the same blurb the real UO client shows there, but ClilocLoader entries
/// carry light HTML-ish markup that must never be shipped to the page as-is.
///
/// Rules: `<br>`/`<br/>` (any case) and `</h2>` become newlines, every other
/// `<...>` tag is dropped, 3+ consecutive newlines collapse to 2, and the
/// result is trimmed (leading/trailing whitespace, and trailing spaces on
/// each line).
pub(super) fn cliloc_markup_to_plain_text(markup: &str) -> String {
    // Pass 1: turn the couple of tags that carry layout meaning into
    // newlines, drop every other tag outright. Hand-rolled (no regex — core
    // stays near-zero-dep) over the byte stream since cliloc text is ASCII/
    // Latin-1-ish markup.
    let mut out = String::with_capacity(markup.len());
    let bytes = markup.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let Some(end) = markup[i..].find('>') else {
                break; // unterminated tag: stop rather than emit a partial one
            };
            let tag = markup[i + 1..i + end].to_ascii_lowercase();
            if matches!(tag.as_str(), "br" | "br/" | "br /" | "/h2") {
                out.push('\n');
            }
            // Any other tag (e.g. `<h2>`) is simply dropped.
            i += end + 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }

    // Pass 2: collapse 3+ consecutive newlines to 2, then trim trailing
    // spaces on each line and leading/trailing whitespace overall.
    let mut collapsed = String::with_capacity(out.len());
    let mut newline_run = 0u32;
    for ch in out.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                collapsed.push(ch);
            }
        } else {
            newline_run = 0;
            collapsed.push(ch);
        }
    }
    collapsed
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod login_request_tests {
    use super::{
        cliloc_markup_to_plain_text, login_attempt_expected, parse_character_choice,
        parse_login_attempt, starting_skills, CharacterDecision,
    };
    use anima_core::net::CharacterChoice;

    #[test]
    fn legacy_login_body_remains_supported() {
        let attempt = parse_login_attempt("127.0.0.1:2594:account:pass:with:colons").unwrap();
        assert_eq!(attempt.host, "127.0.0.1");
        assert_eq!(attempt.port, 2594);
        assert_eq!(attempt.username, "account");
        assert_eq!(attempt.password, "pass:with:colons");
        assert_eq!(attempt.character_slot, None);
        assert!(!attempt.interactive);
        assert!(attempt.create.is_none());
        assert_eq!(attempt.shard, 0);
    }

    #[test]
    fn a_login_body_can_name_the_shard_to_enter() {
        let attempt = parse_login_attempt(
            r#"{"host":"127.0.0.1","port":2593,"username":"a","password":"b","shard":7}"#,
        )
        .unwrap();
        assert_eq!(attempt.shard, 7);
        // Omitted stays 0, which is what every single-shard server answers to.
        let attempt = parse_login_attempt(
            r#"{"host":"127.0.0.1","port":2593,"username":"a","password":"b"}"#,
        )
        .unwrap();
        assert_eq!(attempt.shard, 0);
        assert!(parse_login_attempt(
            r#"{"host":"h","port":1,"username":"a","password":"b","shard":99999}"#
        )
        .is_err());
    }

    #[test]
    fn login_posts_are_accepted_only_on_login_or_error_scenes() {
        assert!(login_attempt_expected(r#"{"auth":"login"}"#));
        assert!(login_attempt_expected(
            r#"{"auth":"error","msg":"try again"}"#
        ));
        assert!(!login_attempt_expected(r#"{"auth":"connecting"}"#));
        assert!(!login_attempt_expected(r#"{"player":{"serial":7}}"#));
        assert!(!login_attempt_expected("not json"));
    }

    #[test]
    fn json_login_body_carries_new_character_configuration() {
        let attempt = parse_login_attempt(
            r#"{
                "host":"127.0.0.1","port":2594,
                "username":"account","password":"secret","character_slot":4,"interactive":true,
                "create":{"name":"New Hero","female":true,"profession":"mage",
                    "strength":20,"dexterity":20,"intelligence":50,"city_index":3}
            }"#,
        )
        .unwrap();
        let appearance = attempt.create.unwrap();
        assert_eq!(attempt.character_slot, Some(4));
        assert!(attempt.interactive);
        assert_eq!(appearance.name, "New Hero");
        assert!(appearance.female);
        assert_eq!(
            (
                appearance.strength,
                appearance.dexterity,
                appearance.intelligence
            ),
            (20, 20, 50)
        );
        assert_eq!(appearance.skills, starting_skills("mage").unwrap());
        assert_eq!(appearance.city_index, 3);
    }

    #[test]
    fn json_login_body_selects_an_exact_existing_slot() {
        let attempt = parse_login_attempt(
            r#"{"host":"127.0.0.1","port":2594,"username":"account","password":"secret",
                "character_slot":2,"create":null}"#,
        )
        .unwrap();
        assert_eq!(attempt.character_slot, Some(2));
        assert!(attempt.create.is_none());
    }

    #[test]
    fn character_choice_parser_accepts_play_create_delete_and_cancel() {
        assert_eq!(
            parse_character_choice(r#"{"slot":3}"#),
            Ok(CharacterDecision::Choose(CharacterChoice::Play(3)))
        );
        let choice = parse_character_choice(
            r#"{"create":{"name":"New Hero","female":false,"profession":"warrior",
                "strength":60,"dexterity":20,"intelligence":10,"city_index":0}}"#,
        )
        .unwrap();
        let CharacterDecision::Choose(CharacterChoice::Create(appearance)) = choice else {
            panic!("expected create choice");
        };
        assert_eq!(appearance.name, "New Hero");
        assert_eq!(appearance.skills, starting_skills("warrior").unwrap());
        assert_eq!(
            parse_character_choice(r#"{"delete_slot":2}"#),
            Ok(CharacterDecision::Choose(CharacterChoice::Delete(2)))
        );
        assert_eq!(
            parse_character_choice(r#"{"cancel":true}"#),
            Ok(CharacterDecision::Cancel)
        );
        assert!(parse_character_choice(r#"{"slot":1,"delete_slot":1}"#).is_err());
        assert!(parse_character_choice(r#"{"slot":1,"cancel":true}"#).is_err());
    }

    #[test]
    fn character_create_accepts_custom_skills_and_appearance() {
        // The step-by-step wizard sends an explicit `skills` array (overriding the
        // profession preset) plus appearance hues/styles.
        let choice = parse_character_choice(
            r#"{"create":{"name":"Custom One","female":true,
                "strength":45,"dexterity":35,"intelligence":10,"city_index":1,
                "skills":[{"id":40,"value":50},{"id":27,"value":50},{"id":22,"value":20}],
                "skin_hue":1024,"hair_style":8253,"hair_hue":1110,
                "facial_hair_style":0,"facial_hair_hue":0,
                "shirt_hue":337,"pants_hue":842}}"#,
        )
        .unwrap();
        let CharacterDecision::Choose(CharacterChoice::Create(a)) = choice else {
            panic!("expected create choice");
        };
        assert!(a.female);
        assert_eq!((a.strength, a.dexterity, a.intelligence), (45, 35, 10));
        // Custom skills win over any profession preset; padded to four slots.
        assert_eq!(a.skills, [(40, 50), (27, 50), (22, 20), (0, 0)]);
        assert_eq!((a.skin_hue, a.hair_style, a.hair_hue), (1024, 8253, 1110));
        assert_eq!((a.shirt_hue, a.pants_hue), (337, 842));
        // A custom skill total that breaks the UO rules is rejected by validate().
        assert!(parse_character_choice(
            r#"{"create":{"name":"Bad Skills","female":false,
                "strength":60,"dexterity":20,"intelligence":10,"city_index":0,
                "skills":[{"id":40,"value":50},{"id":27,"value":40}]}}"#
        )
        .is_err());
    }

    #[test]
    fn json_login_rejects_invalid_creation_stats_before_connecting() {
        let result = parse_login_attempt(
            r#"{"host":"127.0.0.1","port":2594,"username":"account","password":"secret",
                "create":{"name":"Bad Hero","female":false,"profession":"warrior",
                    "strength":60,"dexterity":60,"intelligence":60,"city_index":3}}"#,
        );
        assert_eq!(
            result,
            Err("strength, dexterity, and intelligence must each be 10-60 and total 90")
        );
    }

    #[test]
    fn cliloc_markup_becomes_readable_plain_text() {
        // Real ServUO/ClilocLoader shape for cliloc 1075074 (Britain's
        // character-creation city blurb): `<h2>` opens with no newline of its
        // own, `</h2>` immediately followed by `<br>` yields a blank line
        // (2 newlines — the *pair* deliberately survives collapsing, since
        // only a run of 3+ gets folded down to 2), and a run of three
        // consecutive `<br>`s also folds down to that same single blank line.
        let markup = "<h2>Britain</h2><br>The City of Bards<br><br><br>The thriving city of Britain is the capital.  ";
        assert_eq!(
            cliloc_markup_to_plain_text(markup),
            "Britain\n\nThe City of Bards\n\nThe thriving city of Britain is the capital."
        );

        // Case-insensitive tags, self-closing `<br/>`, and stray unknown tags
        // are all handled: unknown tags vanish, `<BR/>` still breaks a line.
        assert_eq!(
            cliloc_markup_to_plain_text("Line one<BR/>Line two<unknown>tag</unknown>"),
            "Line one\nLine twotag"
        );

        assert_eq!(cliloc_markup_to_plain_text(""), "");
    }
}

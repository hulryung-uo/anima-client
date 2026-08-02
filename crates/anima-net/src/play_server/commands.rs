//! `POST /input` → [`Action`]: the browser's whole command surface.
//!
//! One `cmd:arg` line per request, parsed into the same [`Action`] a brain would
//! emit — which is the point: the human client and an AI drive the session
//! through the identical contract.

use super::*;

/// Parse an `hdesign:*` house-designer command (`POST /input`, tried before
/// [`parse_command`] — see the `/input` handler) into the matching
/// [`Action::HouseDesign`]. `graphic` is a catalog piece id (`GET
/// /housecatalog`); `x`/`y`/`z` are foundation-relative offsets, the same
/// convention the outgoing `build_house_design_*` builders this maps 1:1 onto
/// expect. Kept as its own small parser — NOT folded into [`parse_command`]
/// below — only because the `hdesign:*` wire grammar (colon-separated verb +
/// ints) is its own family distinct from the rest of `parse_command`'s verbs;
/// the result is an ordinary [`Action`] from here on, flowing through the same
/// `tx` channel → [`crate::Session::apply_action`] path as everything else
/// (an AI brain can emit [`Action::HouseDesign`] directly, with no `hdesign:*`
/// string involved). Supported:
/// `hdesign:add:<graphic>:<x>:<y>` · `hdesign:del:<graphic>:<x>:<y>:<z>` ·
/// `hdesign:stair:<graphic>:<x>:<y>` · `hdesign:roof:<graphic>:<x>:<y>:<z>` ·
/// `hdesign:roofdel:<graphic>:<x>:<y>:<z>` · `hdesign:floor:<n>` ·
/// `hdesign:commit` · `hdesign:close` · `hdesign:clear` · `hdesign:revert` ·
/// `hdesign:backup` · `hdesign:restore` · `hdesign:sync` (the last seven take
/// no argument; a trailing `:` after them is rejected like [`parse_command`]'s
/// `logout` already does).
pub(super) fn parse_house_design_command(body: &str) -> Option<Action> {
    let rest = body.trim().strip_prefix("hdesign:")?;
    let (cmd, arg) = rest.split_once(':').unwrap_or((rest, ""));
    let cmd = match cmd {
        "add" => {
            let mut p = arg.split(':');
            HouseDesignAction::AddItem {
                graphic: p.next()?.parse().ok()?,
                x: p.next()?.parse().ok()?,
                y: p.next()?.parse().ok()?,
            }
        }
        "del" => {
            let mut p = arg.split(':');
            HouseDesignAction::DeleteItem {
                graphic: p.next()?.parse().ok()?,
                x: p.next()?.parse().ok()?,
                y: p.next()?.parse().ok()?,
                z: p.next()?.parse().ok()?,
            }
        }
        "stair" => {
            let mut p = arg.split(':');
            HouseDesignAction::AddStair {
                graphic: p.next()?.parse().ok()?,
                x: p.next()?.parse().ok()?,
                y: p.next()?.parse().ok()?,
            }
        }
        "roof" => {
            let mut p = arg.split(':');
            HouseDesignAction::AddRoof {
                graphic: p.next()?.parse().ok()?,
                x: p.next()?.parse().ok()?,
                y: p.next()?.parse().ok()?,
                z: p.next()?.parse().ok()?,
            }
        }
        "roofdel" => {
            let mut p = arg.split(':');
            HouseDesignAction::DeleteRoof {
                graphic: p.next()?.parse().ok()?,
                x: p.next()?.parse().ok()?,
                y: p.next()?.parse().ok()?,
                z: p.next()?.parse().ok()?,
            }
        }
        "floor" => HouseDesignAction::GoToFloor(arg.parse().ok()?),
        "commit" => arg.is_empty().then_some(HouseDesignAction::Commit)?,
        "close" => arg.is_empty().then_some(HouseDesignAction::Close)?,
        "clear" => arg.is_empty().then_some(HouseDesignAction::Clear)?,
        "revert" => arg.is_empty().then_some(HouseDesignAction::Revert)?,
        "backup" => arg.is_empty().then_some(HouseDesignAction::Backup)?,
        "restore" => arg.is_empty().then_some(HouseDesignAction::Restore)?,
        "sync" => arg.is_empty().then_some(HouseDesignAction::Sync)?,
        _ => return None,
    };
    Some(Action::HouseDesign(cmd))
}

/// Parse a `cmd:arg` input line into an [`Action`]. Supported:
/// `walk:<dir>:<run>` · `run:<dir>` · `say:<text>` (plus the same-shaped
/// `whisper:` / `yell:` / `emote:` / `guild:` / `alliance:`, which differ only
/// in the MessageType byte) · `statlock:<stat>:<lock>` · `use:<serial>` ·
/// `click:<serial>` · `attack:<serial>` · `pickup:<serial>[:<amount>]` ·
/// `drop:<serial>:<x>:<y>:<z>[:<container>]` (container default 0xFFFFFFFF =
/// ground) · `equip:<serial>[:<layer>]` (layer 0 = derive from tiledata) ·
/// `war:<0|1>` · `cast:<spellId>` · `target:<serial>` · `targetxy:<x>:<y>:<z>:<graphic>` ·
/// `gump:<serial>:<gumpId>:<button>[:sw=1,2][:e=<id>=<text>,…]` (gump reply; text
/// entries can't contain `:`, `,`, or `=`) · `menusel:<serial>:<index>` (legacy
/// 0x7C menu; index 0 cancels) · `huepick:<serial>:<hue>` (0x95 dye picker) ·
/// `prompt:<text>` / `promptcancel`
/// (answer/cancel a pending 0x9A ASCII / 0xC2 Unicode server text prompt) ·
/// `tipnav:<seq>:<0|1>` / `tipclose:<seq>` (previous/next or dismiss an exact
/// 0xA6 Tip/Notice window) ·
/// `textentry:<seq>:<0|1>:<text>` / `textentryclose:<seq>` (Cancel/OK response
/// or permitted silent close for an exact 0xAB dialog) ·
/// `profile:<serial>` / `profileupdate:<seq>:<text>` / `profileclose:<seq>`
/// (request, save/close an editable profile, or close a read-only 0xB8 profile) ·
/// `logout` (negotiate 0xD1 session termination when supported) ·
/// `tradeaccept:<mycont>:<0|1>` / `tradecancel:<mycont>` /
/// `tradegold:<mycont>:<gold>:<platinum>` (answer the secure-trade session
/// keyed by our own container serial `mycont`, 0x6F — multiple concurrent
/// sessions with different opponents are addressed by their own `mycont`,
/// from `scene.trades[].myCont`; items move via the normal `drop` command
/// targeting that same container serial).
///
/// House-designer commands (`hdesign:*`) are a separate family — see
/// [`parse_house_design_command`], tried first by the `/input` handler.
pub(super) fn parse_command(body: &str) -> Option<Action> {
    let raw_body = body;
    let body = body.trim();
    let (cmd, arg) = body.split_once(':').unwrap_or((body, ""));
    match cmd {
        "walk" => {
            let mut p = arg.split(':');
            let dir: u8 = p.next()?.parse().ok()?;
            let run = p.next() == Some("1");
            Some(Action::Walk { dir: dir & 7, run })
        }
        "run" => Some(Action::Walk {
            dir: arg.parse::<u8>().ok()? & 7,
            run: true,
        }),
        // walkto:<x>,<y> — click-to-walk: pathfind to a ground tile and auto-walk.
        // Accept either delimiter: the web client sends `x,y`, but the whole input
        // line is already colon-split, so a hand-typed `walkto:x:y` (the natural
        // guess, and what tripped up shell/GM testing) must not silently no-op.
        "walkto" => {
            let (x, y) = arg.split_once([',', ':'])?;
            Some(Action::WalkTo {
                x: x.trim().parse().ok()?,
                y: y.trim().parse().ok()?,
            })
        }
        // say / whisper / yell / emote / guild / alliance — the same packet
        // with the MessageType byte the mode implies. The receive side already
        // styles all of them; only the send side was stuck on plain speech.
        "say" | "whisper" | "yell" | "emote" | "guild" | "alliance" => Some(Action::Say {
            text: arg.to_string(),
            mode: SpeechMode::from_name(cmd)?,
        }),
        "party" => Some(Action::PartySay {
            text: arg.to_string(),
        }),
        "use" => Some(Action::Use {
            serial: parse_serial(arg)?,
        }),
        "click" => Some(Action::Click {
            serial: parse_serial(arg)?,
        }),
        "attack" => Some(Action::Attack {
            serial: parse_serial(arg)?,
        }),
        // Auto-attack the best in-view hostile (last target, else nearest hostile).
        "autoattack" => Some(Action::AutoAttack),
        // Re-attack the remembered "last target".
        "attacklast" => Some(Action::AttackLast),
        "pickup" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let amount = p.next().and_then(|s| s.parse().ok()).unwrap_or(1);
            Some(Action::PickUp { serial, amount })
        }
        "drop" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            Some(Action::Drop {
                serial,
                x: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                y: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                z: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                container: p.next().and_then(parse_serial).unwrap_or(0xFFFF_FFFF),
            })
        }
        "equip" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            // layer 0 = "derive from the item's tiledata layer" (done in the loop).
            let layer = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            Some(Action::Equip { serial, layer })
        }
        "war" => Some(Action::WarMode {
            on: arg == "1" || arg == "on",
        }),
        "cast" => Some(Action::CastSpell {
            spell: arg.parse().ok()?,
        }),
        // ability:<id> — arm a weapon special move (0 disarms). 0xD7 UseCombatAbility.
        "ability" => Some(Action::UseAbility {
            ability: arg.parse().ok()?,
        }),
        // buy:<vendor>:<serial>x<amt>,<serial>x<amt>,…  (amount defaults to 1)
        "buy" => {
            let (vendor, list) = arg.split_once(':')?;
            Some(Action::BuyItems {
                vendor: parse_serial(vendor)?,
                items: parse_shop_items(list),
            })
        }
        // sell:<vendor>:<serial>x<amt>,…
        "sell" => {
            let (vendor, list) = arg.split_once(':')?;
            Some(Action::SellItems {
                vendor: parse_serial(vendor)?,
                items: parse_shop_items(list),
            })
        }
        // gump:<serial>:<gumpId>:<button>[:sw=1,2,3][:e=<id>=<text>,<id>=<text>]
        // Answer a server gump (0xB0/0xDD). `button` 0 = close/cancel. The optional
        // `sw=` group lists checked switch ids; the optional `e=` group lists text
        // entries as `<id>=<text>` (text may contain anything except a comma).
        "gump" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let gump_id = parse_serial(p.next()?)?;
            let button = p.next().and_then(parse_serial).unwrap_or(0);
            let mut switches = Vec::new();
            let mut entries = Vec::new();
            for seg in p {
                if let Some(sw) = seg.strip_prefix("sw=") {
                    switches = sw
                        .split(',')
                        .filter_map(|s| if s.is_empty() { None } else { s.parse().ok() })
                        .collect();
                } else if let Some(es) = seg.strip_prefix("e=") {
                    for pair in es.split(',') {
                        if let Some((id, text)) = pair.split_once('=') {
                            if let Ok(id) = id.parse::<u16>() {
                                entries.push((id, text.to_string()));
                            }
                        }
                    }
                }
            }
            Some(Action::GumpResponse {
                serial,
                gump_id,
                button,
                switches,
                entries,
            })
        }
        // oplreq:<serial> — request an entity's Object Property List / tooltip (0xD6).
        "oplreq" => Some(Action::OplRequest {
            serial: parse_serial(arg)?,
        }),
        // partyinvite — invite a player (0xBF/0x06/0x01); the server opens a target cursor.
        "partyinvite" => Some(Action::PartyInvite),
        // partyleave — leave the party (0xBF/0x06/0x02, self serial filled by the driver).
        "partyleave" => Some(Action::PartyLeave),
        // partyaccept[:<leader>] — accept an invite (0xBF/0x06/0x08). Defaults to the
        // pending inviter when no serial is given (the UI omits it).
        "partyaccept" => Some(Action::PartyAccept {
            leader: parse_serial(arg).unwrap_or(0),
        }),
        // partydecline[:<leader>] — decline an invite (0xBF/0x06/0x09).
        "partydecline" => Some(Action::PartyDecline {
            leader: parse_serial(arg).unwrap_or(0),
        }),
        // popupreq:<serial> — request the right-click context menu (0xBF/0x13).
        "popupreq" => Some(Action::PopupRequest {
            serial: parse_serial(arg)?,
        }),
        // popupsel:<serial>:<index> — choose an entry from the open menu (0xBF/0x15).
        "popupsel" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let index = p.next()?.parse().ok()?;
            Some(Action::PopupSelect { serial, index })
        }
        // menusel:<serial>:<index> — answer/cancel a legacy 0x7C menu (0x7D).
        "menusel" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let index = p.next()?.parse().ok()?;
            Some(Action::LegacyMenuSelect { serial, index })
        }
        // huepick:<serial>:<hue> — choose a dyed hue in a server 0x95 picker.
        "huepick" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let hue = p.next()?.parse().ok()?;
            Some(Action::HuePickerSelect { serial, hue })
        }
        // bookreq:<serial>:<count> — request all pages of the open book (0x66).
        "bookreq" => {
            let mut p = arg.split(':');
            let serial = parse_serial(p.next()?)?;
            let pages = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            Some(Action::BookRequest { serial, pages })
        }
        // skilllock:<id>:<lock> — set a skill's lock (0=up,1=down,2=locked). 0x3A.
        "skilllock" => {
            let mut p = arg.split(':');
            let skill = p.next()?.parse().ok()?;
            let lock = p.next()?.parse().ok()?;
            Some(Action::SkillLock { skill, lock })
        }
        // statlock:<stat>:<lock> — stat is 0 str / 1 dex / 2 int, lock is
        // 0 up / 1 down / 2 locked, exactly as `skilllock` above.
        "statlock" => {
            let mut p = arg.split(':');
            let stat: u8 = p.next()?.parse().ok()?;
            let lock: u8 = p.next()?.parse().ok()?;
            (stat <= 2 && lock <= 2).then_some(Action::StatLock { stat, lock })
        }
        // useskill:<id> — invoke an active skill (0x12 ActionRequest type 0x24).
        "useskill" => Some(Action::UseSkill {
            skill: arg.parse().ok()?,
        }),
        "target" => Some(Action::TargetObject {
            serial: parse_serial(arg)?,
        }),
        "targetcancel" => Some(Action::TargetCancel),
        "targetxy" => {
            let mut p = arg.split(':');
            Some(Action::TargetGround {
                x: p.next()?.parse().ok()?,
                y: p.next()?.parse().ok()?,
                z: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
                graphic: p.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            })
        }
        // prompt:<text> — answer a pending 0x9A ASCII / 0xC2 Unicode server
        // prompt (pet rename, house sign, guild abbreviation, …).
        "prompt" => Some(Action::PromptResponse {
            text: arg.to_string(),
        }),
        // promptcancel — cancel a pending server text prompt (Esc).
        "promptcancel" => Some(Action::PromptCancel),
        // tipnav:<seq>:<0|1> — previous/next on a pageable Tip window.
        "tipnav" => {
            let mut p = arg.split(':');
            Some(Action::TipNavigate {
                seq: p.next()?.parse().ok()?,
                next: p.next()? == "1",
            })
        }
        // tipclose:<seq> — dismiss a Tip or Notice locally (no wire response).
        "tipclose" => Some(Action::TipClose {
            seq: arg.parse().ok()?,
        }),
        // textentry:<seq>:<0|1>:<text> — explicit Cancel/OK on an exact 0xAB
        // dialog. splitn preserves colons in the actual text.
        "textentry" => {
            let mut p = arg.splitn(3, ':');
            let seq = p.next()?.parse().ok()?;
            let accepted = match p.next()? {
                "0" => false,
                "1" => true,
                _ => return None,
            };
            let text = p.next().unwrap_or_default().to_string();
            Some(Action::TextEntryResponse {
                seq,
                text,
                accepted,
            })
        }
        // textentryclose:<seq> — silent close; the driver enforces canClose.
        "textentryclose" => Some(Action::TextEntryClose {
            seq: arg.parse().ok()?,
        }),
        // profile:<serial> — request a character's 0xB8 profile.
        "profile" => Some(Action::ProfileRequest {
            serial: parse_serial(arg)?,
        }),
        // profileupdate:<seq>:<text> — save and close an exact editable
        // profile. Parse the text from the untrimmed request body so leading/
        // trailing whitespace and terminal newlines remain part of the profile.
        "profileupdate" => {
            let raw_arg = raw_body.trim_start().strip_prefix("profileupdate:")?;
            let (seq, text) = raw_arg.split_once(':')?;
            Some(Action::ProfileUpdate {
                seq: seq.parse().ok()?,
                text: text.to_string(),
            })
        }
        // profileclose:<seq> — dismiss an exact read-only profile locally.
        "profileclose" => Some(Action::ProfileClose {
            seq: arg.parse().ok()?,
        }),
        "logout" => arg.is_empty().then_some(Action::Logout),
        // tradeaccept:<mycont>:<0|1> — toggle our accept checkbox on the secure
        // trade session keyed by our own container serial (0x6F action 2).
        "tradeaccept" => {
            let mut p = arg.split(':');
            let container = parse_serial(p.next()?)?;
            let accept = p.next() == Some("1");
            Some(Action::TradeAccept { container, accept })
        }
        // tradecancel:<mycont> — cancel the secure trade session keyed by our
        // own container serial (0x6F action 1).
        "tradecancel" => Some(Action::TradeCancel {
            container: parse_serial(arg)?,
        }),
        // tradegold:<mycont>:<gold>:<platinum> — set our virtual gold/platinum
        // offer on the session keyed by our own container serial. Parsed as u64
        // and saturated to u32::MAX rather than the usual `.ok()` "couldn't
        // parse → 0" fallback — a fat-fingered over-range entry (e.g.
        // 5000000000) must clamp, not silently become a 0-gold offer.
        "tradegold" => {
            let mut p = arg.split(':');
            let container = parse_serial(p.next()?)?;
            let gold = p.next().and_then(parse_saturating_u32).unwrap_or(0);
            let platinum = p.next().and_then(parse_saturating_u32).unwrap_or(0);
            Some(Action::TradeGold {
                container,
                gold,
                platinum,
            })
        }
        _ => None,
    }
}

/// Parse a comma-separated `<serial>x<amt>` list (amount defaults to 1) into
/// `(serial, amount)` pairs, skipping any malformed entry. e.g.
/// `0x4000001x3,0x4000002` → `[(0x4000001, 3), (0x4000002, 1)]`.
pub(super) fn parse_shop_items(list: &str) -> Vec<(u32, u16)> {
    list.split(',')
        .filter_map(|e| {
            let e = e.trim();
            if e.is_empty() {
                return None;
            }
            let (s, a) = e.split_once('x').unwrap_or((e, "1"));
            let serial = parse_serial(s)?;
            let amount = a.trim().parse().unwrap_or(1);
            Some((serial, amount))
        })
        .collect()
}

pub(super) fn parse_serial(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Parse a decimal amount that may overflow `u32` (e.g. a mistyped gold
/// entry), saturating to `u32::MAX` instead of the `.ok()` pattern's usual
/// "couldn't parse → 0" fallback — a huge-but-real offer should clamp, not
/// silently vanish to zero.
pub(super) fn parse_saturating_u32(s: &str) -> Option<u32> {
    s.trim()
        .parse::<u64>()
        .ok()
        .map(|v| v.min(u32::MAX as u64) as u32)
}

#[cfg(test)]
mod command_tests {
    //! `parse_command`/`parse_house_design_command` coverage. These lived in
    //! `hue_palette_tests` next door, which is where they landed rather than
    //! where they belonged — the module had become whatever was nearest.
    use super::*;

    #[test]
    fn huepick_command_preserves_picker_serial_and_hue() {
        assert_eq!(
            parse_command("huepick:0x01020304:902"),
            Some(Action::HuePickerSelect {
                serial: 0x0102_0304,
                hue: 902,
            })
        );
        assert!(parse_command("huepick:bad:902").is_none());
    }

    #[test]
    fn tip_commands_preserve_window_seq_and_direction() {
        assert_eq!(
            parse_command("tipnav:42:1"),
            Some(Action::TipNavigate {
                seq: 42,
                next: true
            })
        );
        assert_eq!(
            parse_command("tipnav:42:0"),
            Some(Action::TipNavigate {
                seq: 42,
                next: false,
            })
        );
        assert_eq!(
            parse_command("tipclose:42"),
            Some(Action::TipClose { seq: 42 })
        );
        assert!(parse_command("tipnav:bad:1").is_none());
        assert!(parse_command("tipclose:bad").is_none());
    }

    #[test]
    fn text_entry_commands_preserve_seq_code_and_colons() {
        assert_eq!(
            parse_command("textentry:42:1:part:two"),
            Some(Action::TextEntryResponse {
                seq: 42,
                text: "part:two".into(),
                accepted: true,
            })
        );
        assert_eq!(
            parse_command("textentry:42:0:"),
            Some(Action::TextEntryResponse {
                seq: 42,
                text: String::new(),
                accepted: false,
            })
        );
        assert_eq!(
            parse_command("textentryclose:42"),
            Some(Action::TextEntryClose { seq: 42 })
        );
        assert!(parse_command("textentry:42:maybe:no").is_none());
        assert!(parse_command("textentryclose:bad").is_none());
    }

    #[test]
    fn profile_commands_preserve_serial_seq_and_body_colons() {
        assert_eq!(
            parse_command("profile:0x01020304"),
            Some(Action::ProfileRequest {
                serial: 0x0102_0304,
            })
        );
        assert_eq!(
            parse_command("profileupdate:42:Born in: Britain"),
            Some(Action::ProfileUpdate {
                seq: 42,
                text: "Born in: Britain".into(),
            })
        );
        assert_eq!(
            parse_command("profileupdate:42:"),
            Some(Action::ProfileUpdate {
                seq: 42,
                text: String::new(),
            })
        );
        assert_eq!(
            parse_command("profileupdate:42:  verse\n"),
            Some(Action::ProfileUpdate {
                seq: 42,
                text: "  verse\n".into(),
            })
        );
        assert_eq!(
            parse_command("profileclose:42"),
            Some(Action::ProfileClose { seq: 42 })
        );
        assert!(parse_command("profile:bad").is_none());
        assert!(parse_command("profileupdate:bad:text").is_none());
        assert!(parse_command("profileclose:bad").is_none());
    }

    #[test]
    fn speech_commands_carry_their_message_type() {
        for (cmd, mode) in [
            ("say", SpeechMode::Say),
            ("whisper", SpeechMode::Whisper),
            ("yell", SpeechMode::Yell),
            ("emote", SpeechMode::Emote),
            ("guild", SpeechMode::Guild),
            ("alliance", SpeechMode::Alliance),
        ] {
            assert_eq!(
                parse_command(&format!("{cmd}:hello there")),
                Some(Action::Say {
                    text: "hello there".into(),
                    mode,
                }),
                "{cmd}"
            );
        }
        // Party is a different packet entirely, not a speech mode.
        assert_eq!(
            parse_command("party:hello"),
            Some(Action::PartySay {
                text: "hello".into()
            })
        );
    }

    #[test]
    fn stat_lock_command_rejects_out_of_range_values() {
        assert_eq!(
            parse_command("statlock:1:2"),
            Some(Action::StatLock { stat: 1, lock: 2 })
        );
        // Only three stats and three lock states exist; anything else would be
        // a packet the server has no meaning for.
        assert!(parse_command("statlock:3:0").is_none());
        assert!(parse_command("statlock:0:9").is_none());
        assert!(parse_command("statlock:0").is_none());
    }

    #[test]
    fn logout_command_is_exact_and_argument_free() {
        assert_eq!(parse_command("logout"), Some(Action::Logout));
        assert!(parse_command("logout:anything").is_none());
    }

    #[test]
    fn house_design_add_command_parses_graphic_and_position() {
        assert_eq!(
            parse_house_design_command("hdesign:add:1234:5:-6"),
            Some(Action::HouseDesign(HouseDesignAction::AddItem {
                graphic: 1234,
                x: 5,
                y: -6,
            }))
        );
        assert!(parse_house_design_command("hdesign:add:1234:5").is_none());
        assert!(parse_house_design_command("hdesign:add:bad:5:6").is_none());
    }

    #[test]
    fn house_design_delete_command_parses_graphic_position_and_z() {
        assert_eq!(
            parse_house_design_command("hdesign:del:1234:5:-6:2"),
            Some(Action::HouseDesign(HouseDesignAction::DeleteItem {
                graphic: 1234,
                x: 5,
                y: -6,
                z: 2,
            }))
        );
        assert!(parse_house_design_command("hdesign:del:1234:5:-6").is_none());
    }

    #[test]
    fn house_design_floor_command_parses_level() {
        assert_eq!(
            parse_house_design_command("hdesign:floor:3"),
            Some(Action::HouseDesign(HouseDesignAction::GoToFloor(3)))
        );
        assert!(parse_house_design_command("hdesign:floor:bad").is_none());
        assert!(parse_house_design_command("hdesign:floor:").is_none());
    }

    #[test]
    fn house_design_no_arg_commands_reject_a_trailing_argument() {
        assert_eq!(
            parse_house_design_command("hdesign:commit"),
            Some(Action::HouseDesign(HouseDesignAction::Commit))
        );
        assert_eq!(
            parse_house_design_command("hdesign:close"),
            Some(Action::HouseDesign(HouseDesignAction::Close))
        );
        assert_eq!(
            parse_house_design_command("hdesign:clear"),
            Some(Action::HouseDesign(HouseDesignAction::Clear))
        );
        assert_eq!(
            parse_house_design_command("hdesign:revert"),
            Some(Action::HouseDesign(HouseDesignAction::Revert))
        );
        assert_eq!(
            parse_house_design_command("hdesign:backup"),
            Some(Action::HouseDesign(HouseDesignAction::Backup))
        );
        assert_eq!(
            parse_house_design_command("hdesign:restore"),
            Some(Action::HouseDesign(HouseDesignAction::Restore))
        );
        assert_eq!(
            parse_house_design_command("hdesign:sync"),
            Some(Action::HouseDesign(HouseDesignAction::Sync))
        );
        assert!(parse_house_design_command("hdesign:commit:anything").is_none());
    }

    #[test]
    fn house_design_command_rejects_unknown_verb_and_missing_prefix() {
        assert!(parse_house_design_command("hdesign:bogus").is_none());
        assert!(parse_house_design_command("add:1234:5:6").is_none());
        // Falls through to the ordinary command parser instead — never a match here.
        assert!(parse_command("hdesign:commit").is_none());
    }
}

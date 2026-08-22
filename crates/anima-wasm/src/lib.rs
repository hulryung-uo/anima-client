//! WASM bindings for `anima-core`.
//!
//! The browser owns the WebSocket (a relay bridges it to the server's raw TCP);
//! this module owns the *protocol*: it runs the login handshake, decodes the
//! (Huffman) game stream into a [`World`], and emits an [`Observation`] as JSON
//! for the JS renderer. Because `anima-core` is sans-IO, the exact same code
//! that powers the native agent runs here unchanged.
//!
//! Build: `wasm-pack build crates/anima-wasm --target web`.

use anima_contract_json::{action_from_json, observation_to_json, SCHEMA_VERSION};
use anima_core::agent::{HouseDesignAction, SpeechMode};
use anima_core::net::outgoing::{
    build_attack, build_bandage_target, build_boat_move_request, build_book_header_change,
    build_book_page_request, build_book_page_write, build_bulletin_post_message,
    build_bulletin_remove_message, build_bulletin_request_message, build_bulletin_request_summary,
    build_buy, build_cast_spell, build_cast_spell_from_book, build_change_race_cancel,
    build_change_race_request, build_chat_create_channel, build_chat_join, build_chat_leave,
    build_chat_message, build_chat_open, build_disarm_request, build_drop, build_emote_action,
    build_equip, build_equip_last_weapon, build_guild_menu_request, build_gump_response,
    build_help_request, build_house_design_add_item, build_house_design_add_roof,
    build_house_design_add_stair, build_house_design_backup, build_house_design_clear,
    build_house_design_close, build_house_design_commit, build_house_design_delete_item,
    build_house_design_delete_roof, build_house_design_go_to_floor, build_house_design_restore,
    build_house_design_revert, build_house_design_sync, build_hue_picker_response,
    build_invoke_virtue, build_map_add_pin, build_map_change_pin, build_map_clear_pins,
    build_map_insert_pin, build_map_remove_pin, build_map_toggle_editable, build_open_door,
    build_open_uo_store, build_opl_request, build_party_accept, build_party_can_loot,
    build_party_decline, build_party_invite, build_party_leave, build_party_message,
    build_party_private_message, build_party_remove, build_pick_up, build_popup_request,
    build_popup_select, build_profile_request, build_quest_arrow_click, build_quest_menu_request,
    build_rename_request, build_sell, build_single_click, build_skill_lock, build_stat_lock,
    build_status_request, build_stun_request, build_target_by_resource, build_targeted_skill,
    build_targeted_spell, build_toggle_flying, build_trade_accept, build_trade_cancel,
    build_trade_gold, build_use_ability, build_use_skill, BOAT_SPEED_FAST, BOAT_SPEED_SLOW,
    BOAT_SPEED_STOP,
};
use anima_core::net::{
    apply_packet, build_client_version, CharacterAppearance, CharacterChoice, CharacterPrompt,
    LoginConfig, LoginDirective, LoginMachine, StreamDecoder, Walker,
};
use anima_core::world::World;
use anima_core::Action;
use wasm_bindgen::prelude::*;

/// A browser-side UO client core. JS feeds it bytes from the WebSocket and reads
/// back any bytes to send plus the current observation.
#[wasm_bindgen]
pub struct WasmClient {
    login: Option<LoginMachine>,
    decoder: StreamDecoder,
    walker: Walker,
    world: World,
    journal_cursor: usize,
    /// Bytes queued to send to the server (drained by JS).
    outbox: Vec<u8>,
    logged_in: bool,
    logout_handshake: bool,
    /// Game-server address the shard advertised in `0x8C`, `None` when it sent
    /// none worth dialing. The relay, not this crate, opens that socket.
    game_server: Option<String>,
    game_server_port: u16,
    /// Why the login handshake failed, if it did. See [`WasmClient::login_error`].
    login_error: Option<String>,
    /// Last [`LoginDirective::ChooseCharacter`] prompt, until the page calls
    /// [`WasmClient::play_character`] / [`WasmClient::create_character`].
    character_prompt: Option<CharacterPrompt>,
}

#[wasm_bindgen]
impl WasmClient {
    /// JSON schema version returned by [`WasmClient::observation_json`].
    pub fn schema_version() -> u32 {
        SCHEMA_VERSION
    }

    /// Start a login. Returns the initial bytes to send on the (login-server)
    /// socket. `version` is e.g. "7.0.102.3".
    #[wasm_bindgen(constructor)]
    pub fn new(username: String, password: String) -> WasmClient {
        let cfg = LoginConfig {
            username,
            password,
            // The page owns the character list (ClassicUO LoginScene). Auto-pick
            // would skip [`WasmClient::character_list_json`] entirely.
            defer_character_choice: true,
            ..Default::default()
        };
        let (machine, initial) = LoginMachine::start(cfg);
        WasmClient {
            login: Some(machine),
            decoder: StreamDecoder::new(),
            walker: Walker::new(),
            world: World::new(),
            journal_cursor: 0,
            outbox: initial,
            logged_in: false,
            logout_handshake: false,
            game_server: None,
            game_server_port: 0,
            login_error: None,
            character_prompt: None,
        }
    }

    /// Why the login handshake failed — the server's own stated reason, e.g.
    /// "the server rejected the account: incorrect name or password (code 0)".
    /// Empty while nothing has gone wrong. A page that sees this should stop
    /// waiting: no further packet is coming.
    pub fn login_error(&self) -> String {
        self.login_error.clone().unwrap_or_default()
    }

    /// The game server the shard named in `0x8C`, as `"a.b.c.d"`, once
    /// [`WasmClient::logged_in`] is true. Empty when the shard advertised
    /// nothing routable — the relay should then reuse the host the page
    /// logged in through (ClassicUO's `IgnoreRelayIp` case).
    pub fn game_server_host(&self) -> String {
        self.game_server.clone().unwrap_or_default()
    }

    /// Port half of [`WasmClient::game_server_host`]; 0 when there is none.
    pub fn game_server_port(&self) -> u16 {
        self.game_server_port
    }

    /// True once the login handshake reconnected to the game server (JS must
    /// open the second socket and switch to the game stream).
    pub fn logged_in(&self) -> bool {
        self.logged_in
    }

    /// Feed bytes received from the socket.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.decoder.feed(bytes);
        while let Ok(Some(frame)) = self.decoder.pop() {
            self.handle(&frame);
        }
    }

    fn handle(&mut self, frame: &[u8]) {
        // During login, drive the handshake.
        if let Some(machine) = self.login.as_mut() {
            let outcome = machine.on_packet(frame);
            // A refusal (bad password, character already online, queue full…)
            // arrives as an error on exactly one packet and nothing follows it,
            // so dropping it here left the page waiting on a login that was
            // already over. Record it for `login_error` instead.
            if let Err(e) = &outcome {
                self.login_error = Some(e.to_string());
            }
            if let Ok(directives) = outcome {
                for d in directives {
                    match d {
                        LoginDirective::Send(b) => self.outbox.extend(b),
                        LoginDirective::ReconnectToGameServer { address, then } => {
                            // JS owns the socket here (a WebSocket to the
                            // relay, §4), so the core can't dial this itself —
                            // record it for `game_server_address` and let the
                            // page decide, exactly as the native driver's
                            // fallback does.
                            self.game_server = address.is_routable().then(|| address.host());
                            self.game_server_port = address.port;
                            self.decoder.switch_to_game();
                            self.outbox.extend(then);
                            self.logged_in = true;
                        }
                        // The page shows the list via `character_list_json` and
                        // resumes with `play_character` / `create_character`.
                        LoginDirective::ChooseCharacter(prompt) => {
                            self.character_prompt = Some(prompt);
                        }
                        LoginDirective::Done(r) => {
                            self.logout_handshake = r.character_list_flags
                                & anima_core::net::CHARACTER_LIST_FLAG_LOGOUT_HANDSHAKE
                                != 0;
                            self.world.enter_world(&r);
                            self.login = None;
                        }
                    }
                }
                return;
            }
        }
        // In-world: route movement acks / version request / world codec.
        match frame.first().copied() {
            Some(0x22) if frame.len() >= 2 => self.walker.on_confirm(&mut self.world, frame[1]),
            Some(0x21) if frame.len() >= 8 => {
                let x = u16::from_be_bytes([frame[2], frame[3]]);
                let y = u16::from_be_bytes([frame[4], frame[5]]);
                self.walker.on_deny(
                    &mut self.world,
                    frame[1],
                    x,
                    y,
                    frame[7] as i8,
                    frame[6] & 7,
                );
            }
            Some(0xBD) => self.outbox.extend(build_client_version("7.0.102.3")),
            _ => {
                apply_packet(&mut self.world, frame);
            }
        }
        // Custom-house design requests queue in World (a 0xBF/0x1D revision notice
        // marks a design stale); core never sends bytes, so each embedder drains
        // them itself — the native Session does this in pump_once, and here they
        // ride the same outbox as the 0xBD version reply above.
        for serial in self.world.take_house_design_requests() {
            self.outbox
                .extend(anima_core::net::outgoing::build_house_design_request(
                    serial,
                ));
        }
        // Stale tooltips (a 0xDC OPLInfo naming a revision we don't hold) queue
        // the same way; batched 15 serials to a packet like ClassicUO's
        // `Send_MegaClilocRequest`.
        let stale_opl = self.world.take_opl_requests();
        for batch in stale_opl.chunks(anima_core::net::outgoing::OPL_REQUEST_BATCH) {
            self.outbox
                .extend(anima_core::net::outgoing::build_opl_request(batch));
        }
        for on in self.world.take_war_mode_requests() {
            self.outbox
                .extend(anima_core::net::outgoing::build_war_mode(on));
        }
    }

    /// Take queued bytes to send to the server (clears the queue).
    pub fn take_outbox(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.outbox)
    }

    /// Request a walk step (UO direction 0..7); queues the packet in the outbox.
    pub fn walk(&mut self, dir: u8, run: bool) {
        if let Some(pkt) = self.walker.step(&mut self.world, dir, run) {
            self.outbox.extend(pkt);
        }
    }

    /// Speak. ASCII uses 0x03; anything else is unencoded 0xAD (no `speech.mul`
    /// in the browser). `mode` is a [`SpeechMode`] name (`say`, `yell`, …).
    pub fn say(&mut self, text: String, mode: String) {
        let msg_type = SpeechMode::from_name(&mode)
            .unwrap_or(SpeechMode::Say)
            .wire();
        self.queue_say(&text, msg_type);
    }

    /// Double-click (use) a serial.
    pub fn use_serial(&mut self, serial: u32) {
        self.outbox
            .extend(anima_core::net::outgoing::build_double_click(serial));
    }

    /// Attack a serial.
    pub fn attack(&mut self, serial: u32) {
        self.world.last_attack = Some(serial);
        self.outbox
            .extend(anima_core::net::outgoing::build_attack(serial));
    }

    /// Toggle war mode.
    pub fn war_mode(&mut self, on: bool) {
        self.outbox
            .extend(anima_core::net::outgoing::build_war_mode(on));
    }

    /// Apply one contract action, or a JSON array of them. Empty string = ok.
    pub fn apply_action_json(&mut self, json: String) -> String {
        let v: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => return e.to_string(),
        };
        let items: Vec<serde_json::Value> = match v {
            serde_json::Value::Array(a) => a,
            other => vec![other],
        };
        for item in items {
            match action_from_json(&item) {
                Ok(action) => self.queue_action(&action),
                Err(e) => return e,
            }
        }
        String::new()
    }

    /// Server-provided character list (empty object while the handshake has not
    /// reached 0xA9 / a delete re-prompt).
    pub fn character_list_json(&self) -> String {
        let Some(prompt) = &self.character_prompt else {
            return "{}".into();
        };
        let slots: Vec<String> = prompt
            .list
            .slots
            .iter()
            .map(|s| format!("{{\"index\":{},\"name\":{}}}", s.index, json_str(&s.name)))
            .collect();
        let cities: Vec<String> = prompt
            .list
            .cities
            .iter()
            .map(|c| {
                format!(
                    "{{\"index\":{},\"name\":{},\"building\":{}}}",
                    c.index,
                    json_str(&c.name),
                    json_str(&c.building)
                )
            })
            .collect();
        let rejected = prompt.delete_rejected.map(|r| {
            format!(
                ",\"deleteRejected\":{{\"reason\":{},\"text\":{}}}",
                r.reason,
                json_str(r.text)
            )
        });
        format!(
            "{{\"slots\":[{}],\"cities\":[{}],\"slotCount\":{}{}}}",
            slots.join(","),
            cities.join(","),
            prompt.list.slot_count,
            rejected.as_deref().unwrap_or("")
        )
    }

    /// Play the named slot from the current character list. Returns false when
    /// there is no prompt or the slot is empty.
    pub fn play_character(&mut self, slot: u8) -> bool {
        self.choose(CharacterChoice::Play(slot))
    }

    /// Create a character from the same JSON the play-server login form sends
    /// (`name`, stats, skills, `profession`, hues). Returns an empty string on
    /// success, otherwise the error.
    pub fn create_character(&mut self, json: String) -> String {
        if self.character_prompt.is_none() {
            return "no character list".into();
        }
        match appearance_from_json(&json) {
            Ok(appearance) => {
                if self.choose(CharacterChoice::Create(appearance)) {
                    String::new()
                } else {
                    "create was rejected".into()
                }
            }
            Err(e) => e,
        }
    }

    /// Delete the named slot. The next 0xA9/0x86 refreshes [`character_list_json`].
    pub fn delete_character(&mut self, slot: u8) -> bool {
        self.choose(CharacterChoice::Delete(slot))
    }

    /// Answer a legacy 0x7C item/question menu. Returns false when the menu no
    /// longer exists or `index` is out of range; zero is the cancel response.
    pub fn legacy_menu_select(&mut self, serial: u32, index: u16) -> bool {
        let response = self.world.legacy_menu(serial).and_then(|menu| {
            if index == 0 {
                Some((menu.menu_id, 0, 0))
            } else {
                menu.entries.get(index as usize - 1).map(|entry| {
                    let (graphic, hue) = match menu.kind {
                        anima_core::world::LegacyMenuKind::Items => (entry.graphic, entry.hue),
                        anima_core::world::LegacyMenuKind::Question => (0, 0),
                    };
                    (menu.menu_id, graphic, hue)
                })
            }
        });
        let Some((menu_id, graphic, hue)) = response else {
            return false;
        };
        self.outbox
            .extend(anima_core::net::outgoing::build_legacy_menu_response(
                serial, menu_id, index, graphic, hue,
            ));
        self.world.close_legacy_menu(serial);
        true
    }

    /// Answer a pending server 0x95 hue picker. Returns false for a stale
    /// callback serial. Hue normalization matches ServUO (`2..=1001`).
    pub fn hue_picker_select(&mut self, serial: u32, hue: u16) -> bool {
        if self.world.hue_picker(serial).is_none() {
            return false;
        }
        self.outbox
            .extend(anima_core::net::outgoing::build_hue_picker_response(
                serial, hue,
            ));
        self.world.close_hue_picker(serial);
        true
    }

    /// Answer the currently pending 0x9A ASCII or 0xC2 Unicode text prompt.
    /// Returns false when the callback is stale and no prompt remains.
    pub fn prompt_response(&mut self, text: String) -> bool {
        self.answer_prompt(&text, false)
    }

    /// Cancel the currently pending server text prompt.
    pub fn prompt_cancel(&mut self) -> bool {
        self.answer_prompt("", true)
    }

    fn answer_prompt(&mut self, text: &str, cancel: bool) -> bool {
        let Some(prompt) = self.world.prompt else {
            return false;
        };
        let packet = match prompt.kind {
            anima_core::world::PromptKind::Ascii => {
                anima_core::net::outgoing::build_ascii_prompt_response(
                    prompt.sender_serial,
                    prompt.prompt_id,
                    text,
                    cancel,
                )
            }
            anima_core::world::PromptKind::Unicode => {
                anima_core::net::outgoing::build_prompt_response(
                    prompt.sender_serial,
                    prompt.prompt_id,
                    text,
                    cancel,
                )
            }
        };
        self.outbox.extend(packet);
        self.world.prompt = None;
        true
    }

    /// Request the previous/next page for an exact open 0xA6 Tip window.
    /// Returns false for a stale seq or a non-pageable notice.
    pub fn tip_navigate(&mut self, seq: u64, next: bool) -> bool {
        let Some(tip) = self
            .world
            .tip(seq)
            .filter(|tip| tip.kind == anima_core::world::TipKind::Tip)
            .map(|tip| tip.tip)
        else {
            return false;
        };
        self.outbox
            .extend(anima_core::net::outgoing::build_tip_request(tip, next));
        self.world.close_tip(seq);
        true
    }

    /// Dismiss one exact Tip/Notice window without a server packet.
    pub fn tip_close(&mut self, seq: u64) -> bool {
        if self.world.tip(seq).is_none() {
            return false;
        }
        self.world.close_tip(seq);
        true
    }

    /// Answer one exact 0xAB text-entry dialog. `accepted=false` is the explicit
    /// Cancel button and still carries the current text.
    pub fn text_entry_response(&mut self, seq: u64, text: String, accepted: bool) -> bool {
        let Some(dialog) = self.world.text_entry_dialog(seq).cloned() else {
            return false;
        };
        self.outbox
            .extend(anima_core::net::outgoing::build_text_entry_dialog_response(
                dialog.serial,
                dialog.parent_id,
                dialog.button_id,
                &text,
                accepted,
                dialog.variant,
                dialog.max_length,
            ));
        self.world.close_text_entry_dialog(seq);
        true
    }

    /// Silently right-click-close one exact 0xAB dialog when the server allows
    /// it. Explicit Cancel uses `text_entry_response(..., false)` instead.
    pub fn text_entry_close(&mut self, seq: u64) -> bool {
        if !self
            .world
            .text_entry_dialog(seq)
            .is_some_and(|dialog| dialog.can_close)
        {
            return false;
        }
        self.world.close_text_entry_dialog(seq);
        true
    }

    /// Request a character profile (0xB8 type 0). The server decides whether
    /// the target is a visible player in range and returns the display packet.
    pub fn profile_request(&mut self, serial: u32) {
        self.outbox
            .extend(anima_core::net::outgoing::build_profile_request(serial));
    }

    /// Save and close an exact editable self profile. Returns false for stale or
    /// read-only windows; unchanged text closes without emitting an update.
    pub fn profile_update(&mut self, seq: u64, text: String) -> bool {
        let Some(profile) = self
            .world
            .character_profile(seq)
            .filter(|profile| profile.can_edit)
            .cloned()
        else {
            return false;
        };
        if text != profile.body {
            self.outbox
                .extend(anima_core::net::outgoing::build_profile_update(
                    profile.serial,
                    &text,
                ));
        }
        self.world.close_character_profile(seq);
        true
    }

    /// Dismiss one exact profile locally without modifying it.
    pub fn profile_close(&mut self, seq: u64) -> bool {
        if self.world.character_profile(seq).is_none() {
            return false;
        }
        self.world.close_character_profile(seq);
        true
    }

    /// Start ending this game session. Returns `false` when a negotiated 0xD1
    /// request was queued and the host must wait for a fresh
    /// `observation_json().logout_ack.allowed` reply. Returns `true` when the
    /// server did not advertise that handshake and the host may close now.
    pub fn logout(&mut self) -> bool {
        if self.logout_handshake {
            self.outbox
                .extend(anima_core::net::outgoing::build_logout_request());
            false
        } else {
            true
        }
    }

    /// Current perception using the shared, versioned Observation JSON schema.
    pub fn observation_json(&mut self) -> String {
        let obs = self.world.observe(&mut self.journal_cursor);
        observation_to_json(&obs).to_string()
    }
}

impl WasmClient {
    fn choose(&mut self, choice: CharacterChoice) -> bool {
        let Some(machine) = self.login.as_mut() else {
            return false;
        };
        match machine.choose_character(choice) {
            Ok(dirs) => {
                self.character_prompt = None;
                for d in dirs {
                    match d {
                        LoginDirective::Send(b) => self.outbox.extend(b),
                        LoginDirective::ChooseCharacter(prompt) => {
                            self.character_prompt = Some(prompt);
                        }
                        _ => {}
                    }
                }
                true
            }
            Err(e) => {
                self.login_error = Some(e.to_string());
                false
            }
        }
    }

    fn queue_say(&mut self, text: &str, msg_type: u8) {
        if text.is_ascii() {
            self.outbox.extend(anima_core::net::outgoing::build_say(
                text, msg_type, 0x0034, 3,
            ));
        } else {
            self.outbox
                .extend(anima_core::net::outgoing::build_unicode_say(
                    text,
                    msg_type,
                    0x0034,
                    3,
                    &[],
                ));
        }
    }

    fn queue_action(&mut self, action: &Action) {
        match action {
            Action::Walk { dir, run } => self.walk(*dir, *run),
            Action::WalkTo { .. } => {
                // Needs MapData pathfinding (play-server / Session::advance_route).
                // Keyboard Walk still reaches the shard; click-to-walk is a no-op here.
            }
            Action::Say { text, mode } => self.queue_say(text, mode.wire()),
            Action::PartySay { text } => self.outbox.extend(build_party_message(text)),
            Action::Attack { serial } => self.attack(*serial),
            Action::AutoAttack => {
                if let Some(serial) = self.world.auto_attack_target() {
                    self.attack(serial);
                }
            }
            Action::AttackLast => {
                if let Some(serial) = self.world.last_attack {
                    self.outbox.extend(build_attack(serial));
                }
            }
            Action::Use { serial } => self.use_serial(*serial),
            Action::Click { serial } => self.outbox.extend(build_single_click(*serial)),
            Action::PickUp { serial, amount } => {
                self.outbox.extend(build_pick_up(*serial, *amount));
            }
            Action::Drop {
                serial,
                x,
                y,
                z,
                container,
            } => self
                .outbox
                .extend(build_drop(*serial, *x, *y, *z, *container)),
            Action::Equip { serial, layer } => {
                let mobile = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.outbox.extend(build_equip(*serial, *layer, mobile));
            }
            Action::WarMode { on } => self.war_mode(*on),
            Action::CastSpell { spell } => self.outbox.extend(build_cast_spell(*spell)),
            Action::TargetObject { serial } => self.respond_target(Some(*serial), 0, 0, 0, 0),
            Action::TargetGround { x, y, z, graphic } => {
                self.respond_target(None, *x, *y, *z, *graphic);
            }
            Action::TargetCancel => self.cancel_target(),
            Action::BuyItems { vendor, items } => self.outbox.extend(build_buy(*vendor, items)),
            Action::SellItems { vendor, items } => {
                self.outbox.extend(build_sell(*vendor, items));
                self.world.close_shop_sell();
            }
            Action::GumpResponse {
                serial,
                gump_id,
                button,
                switches,
                entries,
            } => {
                self.outbox.extend(build_gump_response(
                    *serial, *gump_id, *button, switches, entries,
                ));
                self.world.close_gump(*serial);
            }
            Action::PopupRequest { serial } => {
                self.outbox.extend(build_popup_request(*serial));
            }
            Action::PopupSelect { serial, index } => {
                self.outbox.extend(build_popup_select(*serial, *index));
                self.world.popup = None;
            }
            Action::LegacyMenuSelect { serial, index } => {
                let _ = self.legacy_menu_select(*serial, *index);
            }
            Action::HuePickerSelect { serial, hue } => {
                if self.world.hue_picker(*serial).is_some() {
                    self.outbox.extend(build_hue_picker_response(*serial, *hue));
                    self.world.close_hue_picker(*serial);
                }
            }
            Action::BookRequest { serial, pages } => {
                self.outbox.extend(build_book_page_request(*serial, *pages));
            }
            Action::UseAbility { ability } => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.outbox.extend(build_use_ability(serial, *ability));
                self.world.arm_ability(*ability);
            }
            Action::DisarmRequest => self.outbox.extend(build_disarm_request()),
            Action::StunRequest => self.outbox.extend(build_stun_request()),
            Action::ToggleFlying => self.outbox.extend(build_toggle_flying()),
            Action::BandageTarget { bandage, target } => {
                let target = if *target != 0 {
                    *target
                } else {
                    self.world.player_mobile().map(|p| p.serial).unwrap_or(0)
                };
                self.outbox.extend(build_bandage_target(*bandage, target));
            }
            Action::TargetedSpell { spell, target } => {
                let target = if *target != 0 {
                    *target
                } else {
                    self.world.player_mobile().map(|p| p.serial).unwrap_or(0)
                };
                self.outbox.extend(build_targeted_spell(*spell, target));
            }
            Action::TargetedSkill { skill, target } => {
                let target = if *target != 0 {
                    *target
                } else {
                    self.world.player_mobile().map(|p| p.serial).unwrap_or(0)
                };
                self.outbox.extend(build_targeted_skill(*skill, target));
            }
            Action::TargetByResource { tool, resource } => {
                self.outbox
                    .extend(build_target_by_resource(*tool, *resource));
            }
            Action::SkillLock { skill, lock } => {
                self.outbox.extend(build_skill_lock(*skill, *lock));
                if let Some(s) = self.world.skills.get_mut(skill) {
                    s.lock = *lock;
                }
            }
            Action::StatLock { stat, lock } => {
                self.outbox.extend(build_stat_lock(*stat, *lock));
                match stat {
                    0 => self.world.player_stats.str_lock = *lock,
                    1 => self.world.player_stats.dex_lock = *lock,
                    2 => self.world.player_stats.int_lock = *lock,
                    _ => {}
                }
            }
            Action::UseSkill { skill } => self.outbox.extend(build_use_skill(*skill)),
            Action::OpenDoor => self.outbox.extend(build_open_door()),
            Action::EquipLastWeapon => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.outbox.extend(build_equip_last_weapon(serial));
            }
            Action::InvokeVirtue { id } => self.outbox.extend(build_invoke_virtue(*id)),
            Action::EmoteAction { action } => self.outbox.extend(build_emote_action(action)),
            Action::CastSpellFromBook { spell, book } => {
                self.outbox
                    .extend(build_cast_spell_from_book(*spell, *book));
            }
            Action::AllNames => {
                const CAP: usize = 60;
                let self_serial = self.world.player_mobile().map(|p| p.serial);
                let mut n = 0usize;
                let mobiles: Vec<u32> = self
                    .world
                    .mobiles
                    .values()
                    .filter(|m| Some(m.serial) != self_serial)
                    .map(|m| m.serial)
                    .collect();
                for serial in mobiles {
                    if n >= CAP {
                        break;
                    }
                    self.outbox.extend(build_single_click(serial));
                    n += 1;
                }
                let corpses: Vec<u32> = self
                    .world
                    .items
                    .values()
                    .filter(|it| it.graphic == 0x2006)
                    .map(|it| it.serial)
                    .collect();
                for serial in corpses {
                    if n >= CAP {
                        break;
                    }
                    self.outbox.extend(build_single_click(serial));
                    n += 1;
                }
            }
            Action::ChangeRace {
                skin_hue,
                hair_style,
                hair_hue,
                beard_style,
                beard_hue,
            } => {
                self.outbox.extend(build_change_race_request(
                    *skin_hue,
                    *hair_style,
                    *hair_hue,
                    *beard_style,
                    *beard_hue,
                ));
                self.world.race_change = None;
            }
            Action::ChangeRaceCancel => {
                self.outbox.extend(build_change_race_cancel());
                self.world.race_change = None;
            }
            Action::OpenUOStore => self.outbox.extend(build_open_uo_store()),
            Action::OplRequest { serial } => self.outbox.extend(build_opl_request(&[*serial])),
            Action::PartyInvite => self.outbox.extend(build_party_invite()),
            Action::PartyAccept { leader } => {
                let leader = if *leader != 0 {
                    *leader
                } else {
                    self.world.party.pending_invite.unwrap_or(0)
                };
                self.outbox.extend(build_party_accept(leader));
                self.world.party.pending_invite = None;
            }
            Action::PartyDecline { leader } => {
                let leader = if *leader != 0 {
                    *leader
                } else {
                    self.world.party.pending_invite.unwrap_or(0)
                };
                self.outbox.extend(build_party_decline(leader));
                self.world.party.pending_invite = None;
            }
            Action::PartyLeave => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.outbox.extend(build_party_leave(serial));
            }
            Action::PartyKick { member } => self.outbox.extend(build_party_remove(*member)),
            Action::PartyPrivateMessage { member, text } => {
                self.outbox
                    .extend(build_party_private_message(*member, text));
            }
            Action::PartySetCanLoot { can_loot } => {
                self.outbox.extend(build_party_can_loot(*can_loot));
            }
            Action::StatusRequest { serial } => {
                let serial = if *serial != 0 {
                    *serial
                } else {
                    self.world.player_mobile().map(|p| p.serial).unwrap_or(0)
                };
                self.outbox.extend(build_status_request(4, serial));
            }
            Action::BulletinRequestMessage { board, message } => {
                self.outbox
                    .extend(build_bulletin_request_message(*board, *message));
            }
            Action::BulletinRequestSummary { board, message } => {
                self.outbox
                    .extend(build_bulletin_request_summary(*board, *message));
            }
            Action::BulletinPost {
                board,
                reply_to,
                subject,
                lines,
            } => {
                let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
                self.outbox.extend(build_bulletin_post_message(
                    *board, *reply_to, subject, &refs,
                ));
            }
            Action::BulletinRemove { board, message } => {
                self.outbox
                    .extend(build_bulletin_remove_message(*board, *message));
            }
            Action::BoatMove { dir, run } => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                let speed = if *run {
                    BOAT_SPEED_FAST
                } else {
                    BOAT_SPEED_SLOW
                };
                self.outbox
                    .extend(build_boat_move_request(serial, *dir, speed));
            }
            Action::BoatStop => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                let dir = self.world.player_mobile().map(|p| p.direction).unwrap_or(0);
                self.outbox
                    .extend(build_boat_move_request(serial, dir, BOAT_SPEED_STOP));
            }
            Action::BookHeaderChange {
                serial,
                title,
                author,
            } => {
                self.outbox
                    .extend(build_book_header_change(*serial, title, author));
            }
            Action::BookPageWrite {
                serial,
                page,
                lines,
            } => {
                self.outbox
                    .extend(build_book_page_write(*serial, *page, lines));
            }
            Action::MapToggleEditable { serial } => {
                self.outbox.extend(build_map_toggle_editable(*serial));
            }
            Action::MapAddPin { serial, x, y } => {
                self.outbox.extend(build_map_add_pin(*serial, *x, *y));
            }
            Action::MapInsertPin {
                serial,
                index,
                x,
                y,
            } => {
                self.outbox
                    .extend(build_map_insert_pin(*serial, *index, *x, *y));
            }
            Action::MapChangePin {
                serial,
                index,
                x,
                y,
            } => {
                self.outbox
                    .extend(build_map_change_pin(*serial, *index, *x, *y));
            }
            Action::MapRemovePin { serial, index } => {
                self.outbox.extend(build_map_remove_pin(*serial, *index));
            }
            Action::MapClearPins { serial } => {
                self.outbox.extend(build_map_clear_pins(*serial));
            }
            Action::ChatOpen => {
                let name = self
                    .world
                    .player_mobile()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.outbox.extend(build_chat_open(&name));
            }
            Action::ChatJoin { channel, password } => {
                self.outbox.extend(build_chat_join(channel, password));
            }
            Action::ChatCreate { channel, password } => {
                self.outbox
                    .extend(build_chat_create_channel(channel, password));
            }
            Action::ChatLeave => self.outbox.extend(build_chat_leave()),
            Action::ChatSay { text } => self.outbox.extend(build_chat_message(text)),
            Action::Rename { serial, name } => {
                self.outbox.extend(build_rename_request(*serial, name));
            }
            Action::QuestArrowClick { right_click } => {
                self.outbox.extend(build_quest_arrow_click(*right_click));
            }
            Action::HelpRequest => self.outbox.extend(build_help_request()),
            Action::GuildMenu => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.outbox.extend(build_guild_menu_request(serial));
            }
            Action::QuestMenu => {
                let serial = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                self.outbox.extend(build_quest_menu_request(serial));
            }
            Action::Logout => {
                let _ = self.logout();
            }
            Action::TradeAccept { container, accept } => {
                if self
                    .world
                    .trades
                    .iter()
                    .any(|t| t.my_container == *container)
                {
                    self.outbox.extend(build_trade_accept(*container, *accept));
                    if let Some(t) = self.world.trade_mut(*container) {
                        t.my_accept = *accept;
                    }
                }
            }
            Action::TradeCancel { container } => {
                if self
                    .world
                    .trades
                    .iter()
                    .any(|t| t.my_container == *container)
                {
                    self.outbox.extend(build_trade_cancel(*container));
                    self.world.close_trade(*container);
                }
            }
            Action::TradeGold {
                container,
                gold,
                platinum,
            } => {
                if self
                    .world
                    .trades
                    .iter()
                    .any(|t| t.my_container == *container)
                {
                    self.outbox
                        .extend(build_trade_gold(*container, *gold, *platinum));
                    if let Some(t) = self.world.trade_mut(*container) {
                        t.my_offer_gold = *gold;
                        t.my_offer_platinum = *platinum;
                    }
                }
            }
            Action::ProfileRequest { serial } => {
                self.outbox.extend(build_profile_request(*serial));
            }
            Action::HouseDesign(cmd) => {
                let player = self.world.player_mobile().map(|p| p.serial).unwrap_or(0);
                let bytes = match *cmd {
                    HouseDesignAction::AddItem { graphic, x, y } => {
                        build_house_design_add_item(player, graphic, x, y)
                    }
                    HouseDesignAction::DeleteItem { graphic, x, y, z } => {
                        build_house_design_delete_item(player, graphic, x, y, z)
                    }
                    HouseDesignAction::AddStair { graphic, x, y } => {
                        build_house_design_add_stair(player, graphic, x, y)
                    }
                    HouseDesignAction::AddRoof { graphic, x, y, z } => {
                        build_house_design_add_roof(player, graphic, x, y, z)
                    }
                    HouseDesignAction::DeleteRoof { graphic, x, y, z } => {
                        build_house_design_delete_roof(player, graphic, x, y, z)
                    }
                    HouseDesignAction::GoToFloor(floor) => {
                        build_house_design_go_to_floor(player, floor)
                    }
                    HouseDesignAction::Commit => build_house_design_commit(player),
                    HouseDesignAction::Close => build_house_design_close(player),
                    HouseDesignAction::Clear => build_house_design_clear(player),
                    HouseDesignAction::Revert => build_house_design_revert(player),
                    HouseDesignAction::Backup => build_house_design_backup(player),
                    HouseDesignAction::Restore => build_house_design_restore(player),
                    HouseDesignAction::Sync => build_house_design_sync(player),
                };
                self.outbox.extend(bytes);
                if matches!(
                    *cmd,
                    HouseDesignAction::AddItem { .. }
                        | HouseDesignAction::DeleteItem { .. }
                        | HouseDesignAction::AddStair { .. }
                        | HouseDesignAction::AddRoof { .. }
                        | HouseDesignAction::DeleteRoof { .. }
                ) {
                    self.outbox.extend(build_house_design_sync(player));
                }
            }
            _ => {}
        }
    }

    fn respond_target(&mut self, serial: Option<u32>, x: u16, y: u16, z: i16, graphic: u16) {
        let Some(cursor) = self.world.pending_target else {
            return;
        };
        let (target_type, serial) = match serial {
            Some(s) => (0u8, s),
            None => (1u8, 0u32),
        };
        self.outbox
            .extend(anima_core::net::outgoing::build_target_response(
                target_type,
                cursor.cursor_id,
                cursor.cursor_flag,
                serial,
                x,
                y,
                z,
                graphic,
            ));
        self.world.pending_target = None;
    }

    fn cancel_target(&mut self) {
        let Some(cursor) = self.world.pending_target else {
            return;
        };
        self.outbox
            .extend(anima_core::net::outgoing::build_target_response(
                cursor.target_type,
                cursor.cursor_id,
                cursor.cursor_flag,
                0,
                0xFFFF,
                0xFFFF,
                0,
                0,
            ));
        self.world.pending_target = None;
    }
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn appearance_from_json(json: &str) -> Result<CharacterAppearance, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let u8n = |k: &str, d: u8| {
        v.get(k)
            .and_then(|x| x.as_u64())
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(d)
    };
    let u16n = |k: &str, d: u16| {
        v.get(k)
            .and_then(|x| x.as_u64())
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(d)
    };
    let mut appearance = CharacterAppearance {
        name: v
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        female: v.get("female").and_then(|x| x.as_bool()).unwrap_or(false),
        skin_hue: u16n("skin_hue", 0),
        hair_style: u16n("hair_style", 0),
        hair_hue: u16n("hair_hue", 0),
        facial_hair_style: u16n("facial_hair_style", 0),
        facial_hair_hue: u16n("facial_hair_hue", 0),
        shirt_hue: u16n("shirt_hue", 0),
        pants_hue: u16n("pants_hue", 0),
        strength: u8n("strength", 60),
        dexterity: u8n("dexterity", 20),
        intelligence: u8n("intelligence", 10),
        city_index: u16n("city_index", 0),
        skills: [(0, 0); 4],
        profession: u8n("profession", 0),
    };
    if let Some(list) = v.get("skills").and_then(|x| x.as_array()) {
        for (i, item) in list.iter().take(4).enumerate() {
            appearance.skills[i] = (
                item.get("id")
                    .and_then(|x| x.as_u64())
                    .and_then(|n| u8::try_from(n).ok())
                    .unwrap_or(0),
                item.get("value")
                    .and_then(|x| x.as_u64())
                    .and_then(|n| u8::try_from(n).ok())
                    .unwrap_or(0),
            );
        }
    }
    appearance.validate().map_err(|e| e.to_string())?;
    Ok(appearance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_uses_the_shared_contract_schema() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        let expected = observation_to_json(&anima_core::Observation::default()).to_string();
        assert_eq!(client.observation_json(), expected);
        assert_eq!(WasmClient::schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn death_status_queues_the_classicuo_peace_mode_reply() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();

        client.handle(&[0x2C, 0]);
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_war_mode(false)
        );
        assert_eq!(client.world.current_music, Some(42));
        assert!(client.world.pending_war_mode_requests.is_empty());
    }

    #[test]
    fn server_pathfind_decodes_without_emitting_spurious_wasm_bytes() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();

        client.handle(&[0x38, 0x04, 0xB0, 0x03, 0x20, 0x00, 0x11]);
        let request = client.world.server_pathfind.expect("0x38 request");
        assert_eq!(
            (request.seq, request.x, request.y, request.z),
            (1, 1200, 800, 17)
        );
        assert!(client.take_outbox().is_empty());
    }

    #[test]
    fn legacy_menu_select_queues_resolved_item_response() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();
        let mut frame = vec![
            0x7C, 0, 0, // id + patched length
            0x01, 0x02, 0x03, 0x04, // serial
            0x00, 0x07, // menu id
            0x06, b'C', b'h', b'o', b'o', b's', b'e', // question
            0x01, // one entry
            0x0F, 0x5E, 0x04, 0x81, // graphic + hue
            0x05, b'S', b'w', b'o', b'r', b'd',
        ];
        let len = frame.len() as u16;
        frame[1..3].copy_from_slice(&len.to_be_bytes());
        client.handle(&frame);
        assert!(client.legacy_menu_select(0x0102_0304, 1));
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_legacy_menu_response(
                0x0102_0304,
                7,
                1,
                0x0F5E,
                0x0481
            )
        );
        assert!(client.world.legacy_menus.is_empty());
    }

    #[test]
    fn hue_picker_select_queues_clipped_response_and_consumes_picker() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();
        client.handle(&[0x95, 0x01, 0x02, 0x03, 0x04, 0, 0, 0x0F, 0xAB]);
        assert!(client.hue_picker_select(0x0102_0304, u16::MAX));
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_hue_picker_response(0x0102_0304, u16::MAX)
        );
        assert!(client.world.hue_pickers.is_empty());
        assert!(!client.hue_picker_select(0x0102_0304, 10));
    }

    #[test]
    fn ascii_prompt_response_queues_matching_packet_and_consumes_prompt() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();
        client.handle(&[0x9A, 0, 11, 0x01, 0x02, 0x03, 0x04, 0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(client.prompt_response("Café".into()));
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_ascii_prompt_response(
                0x0102_0304,
                0xDEAD_BEEF,
                "Café",
                false
            )
        );
        assert!(client.world.prompt.is_none());
        assert!(!client.prompt_cancel());
    }

    #[test]
    fn tip_navigation_and_notice_close_use_distinct_semantics() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();

        // Pageable tip #0x12345678 with text "Tip".
        client.handle(&[
            0xA6, 0, 13, 0, 0x12, 0x34, 0x56, 0x78, 0, 3, b'T', b'i', b'p',
        ]);
        assert!(client.tip_navigate(1, true));
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_tip_request(0x1234_5678, true)
        );
        assert!(client.world.tips.is_empty());
        assert!(!client.tip_navigate(1, false));

        // Flag 2 is a close-only notice: navigation is rejected, local close works.
        client.handle(&[0xA6, 0, 11, 2, 0, 0, 0, 9, 0, 1, b'N']);
        assert!(!client.tip_navigate(2, true));
        assert!(client.tip_close(2));
        assert!(client.take_outbox().is_empty());
        assert!(!client.tip_close(2));
    }

    #[test]
    fn text_entry_response_echoes_live_callback_and_close_permission() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();
        client.world.push_text_entry_dialog(
            0x0102_0304,
            5,
            6,
            "Amount".into(),
            false,
            2,
            3,
            "Digits".into(),
        );

        assert!(!client.text_entry_close(1));
        assert!(client.text_entry_response(1, "1a234".into(), false));
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_text_entry_dialog_response(
                0x0102_0304,
                5,
                6,
                "1a234",
                false,
                2,
                3,
            )
        );
        assert!(!client.text_entry_response(1, "stale".into(), true));

        client.world.push_text_entry_dialog(
            7,
            8,
            9,
            "Optional".into(),
            true,
            0,
            0,
            "Close me".into(),
        );
        assert!(client.text_entry_close(2));
        assert!(client.take_outbox().is_empty());
    }

    #[test]
    fn profile_request_update_and_read_only_close_have_distinct_semantics() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();

        client.profile_request(0x0102_0304);
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_profile_request(0x0102_0304)
        );

        client.world.player = Some(anima_core::Serial(0x0102_0304));
        client.world.set_character_profile(
            0x0102_0304,
            "Anima".into(),
            "Account".into(),
            "Old".into(),
        );
        assert!(client.profile_update(1, "New".into()));
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_profile_update(0x0102_0304, "New")
        );
        assert!(!client.profile_update(1, "stale".into()));

        client
            .world
            .set_character_profile(9, "Other".into(), "".into(), "Read only".into());
        assert!(!client.profile_update(2, "forged".into()));
        assert!(client.profile_close(2));
        assert!(client.take_outbox().is_empty());

        client
            .world
            .set_character_profile(0x0102_0304, "Anima".into(), "".into(), "Same".into());
        assert!(client.profile_update(3, "Same".into()));
        assert!(client.take_outbox().is_empty());
    }

    #[test]
    fn logout_queues_request_and_exposes_server_permission() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();
        client.logout_handshake = true;
        assert!(!client.logout());
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_logout_request()
        );
        client.handle(&[0xD1, 0x00]);
        assert_eq!(
            client.world.logout_ack,
            Some(anima_core::world::LogoutAck {
                seq: 1,
                allowed: false,
            })
        );
        client.logout_handshake = false;
        assert!(client.logout());
        assert!(client.take_outbox().is_empty());
        client.handle(&[0xD1, 0x01]);
        assert_eq!(
            client.world.logout_ack,
            Some(anima_core::world::LogoutAck {
                seq: 2,
                allowed: true,
            })
        );
    }

    #[test]
    fn say_use_attack_and_action_json_queue_packets() {
        let mut client = WasmClient::new("user".into(), "pass".into());
        client.login = None;
        client.outbox.clear();

        client.say("hi".into(), "say".into());
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_say("hi", 0, 0x0034, 3)
        );

        client.use_serial(0x4000_0001);
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_double_click(0x4000_0001)
        );

        client.attack(0x1234);
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_attack(0x1234)
        );

        let err = client.apply_action_json(r#"[{"type":"WarMode","on":true}]"#.into());
        assert!(err.is_empty(), "{err}");
        assert_eq!(
            client.take_outbox(),
            anima_core::net::outgoing::build_war_mode(true)
        );
    }

    #[test]
    fn character_list_json_is_empty_until_prompted() {
        let client = WasmClient::new("user".into(), "pass".into());
        assert_eq!(client.character_list_json(), "{}");
        assert!(!WasmClient::new("user".into(), "pass".into()).play_character(0));
    }
}

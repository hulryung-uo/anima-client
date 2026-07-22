//! WASM bindings for `anima-core`.
//!
//! The browser owns the WebSocket (a relay bridges it to the server's raw TCP);
//! this module owns the *protocol*: it runs the login handshake, decodes the
//! (Huffman) game stream into a [`World`], and emits an [`Observation`] as JSON
//! for the JS renderer. Because `anima-core` is sans-IO, the exact same code
//! that powers the native agent runs here unchanged.
//!
//! Build: `wasm-pack build crates/anima-wasm --target web`.

use anima_contract_json::{observation_to_json, SCHEMA_VERSION};
use anima_core::net::{
    apply_packet, build_client_version, LoginConfig, LoginDirective, LoginMachine, StreamDecoder,
    Walker,
};
use anima_core::world::World;
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
        }
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
            if let Ok(directives) = machine.on_packet(frame) {
                for d in directives {
                    match d {
                        LoginDirective::Send(b) => self.outbox.extend(b),
                        LoginDirective::ReconnectToGameServer { then } => {
                            self.decoder.switch_to_game();
                            self.outbox.extend(then);
                            self.logged_in = true;
                        }
                        // The JS/WASM constructor currently uses automatic slot
                        // selection, so this directive is reserved for native UI.
                        LoginDirective::ChooseCharacter(_) => {}
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
}

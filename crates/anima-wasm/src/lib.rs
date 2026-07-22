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
}

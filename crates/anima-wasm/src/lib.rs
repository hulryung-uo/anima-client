//! WASM bindings for `anima-core`.
//!
//! The browser owns the WebSocket (a relay bridges it to the server's raw TCP);
//! this module owns the *protocol*: it runs the login handshake, decodes the
//! (Huffman) game stream into a [`World`], and emits an [`Observation`] as JSON
//! for the JS renderer. Because `anima-core` is sans-IO, the exact same code
//! that powers the native agent runs here unchanged.
//!
//! Build: `wasm-pack build crates/anima-wasm --target web`.

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

    /// Current perception as JSON (player, nearby mobiles/items, new journal).
    pub fn observation_json(&mut self) -> String {
        let obs = self.world.observe(&mut self.journal_cursor);
        let mobiles: Vec<_> = obs
            .mobiles
            .iter()
            .map(|m| {
                serde_json::json!({
                    "serial": m.serial, "name": m.name, "x": m.pos.x, "y": m.pos.y, "z": m.pos.z,
                    "body": m.body, "noto": m.notoriety, "hits": m.hits, "hitsMax": m.hits_max,
                    "dist": m.distance,
                })
            })
            .collect();
        let items: Vec<_> = obs
            .items
            .iter()
            .map(|it| {
                serde_json::json!({
                    "serial": it.serial, "graphic": it.graphic, "amount": it.amount,
                    "x": it.pos.x, "y": it.pos.y, "z": it.pos.z, "dist": it.distance,
                })
            })
            .collect();
        let journal: Vec<_> = obs
            .new_journal
            .iter()
            .map(|j| serde_json::json!({ "name": j.name, "text": j.text, "type": j.msg_type }))
            .collect();
        serde_json::json!({
            "player": {
                "serial": obs.player.serial, "name": obs.player.name,
                "x": obs.player.pos.x, "y": obs.player.pos.y, "z": obs.player.pos.z,
                "dir": obs.player.direction,
                "hits": obs.player.hits, "hitsMax": obs.player.hits_max,
                "mana": obs.player.mana, "manaMax": obs.player.mana_max,
                "stam": obs.player.stam, "stamMax": obs.player.stam_max,
                "str": obs.player.strength, "dex": obs.player.dexterity, "int": obs.player.intelligence,
                "gold": obs.player.gold,
            },
            "mobiles": mobiles,
            "items": items,
            "journal": journal,
        })
        .to_string()
    }
}

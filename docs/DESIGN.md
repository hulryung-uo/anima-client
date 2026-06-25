# anima-client — Design & Handoff

> **Purpose of this document:** make the project resumable from docs alone. A new
> Claude session (or human) should be able to read this and continue without the
> original chat. It captures *why* every major decision was made, the target
> architecture, the current state, the roadmap, and the concrete protocol/asset
> knowledge needed to implement Phase 1.

Last updated: 2026-06-25 · Status: **Phases 1–3 (core threads) COMPLETE.** 5 crates
(anima-core / anima-assets / anima-net / anima-wasm / anima-agent) + web/; 39 tests
green; clippy clean; wasm32 builds.
- **Phase 1:** headless agent connects to a live ServUO, logs in (create + select),
  builds a World, and navigates to a target tile by A* over real UO map data.
- **Phase 2:** `anima-core` → **wasm32** (sans-IO pays off); `anima-wasm` wraps it for
  the browser; a **web/PixiJS renderer** draws a live minimap + HUD from the same
  `Observation` the AI consumes (scene bridge). Screenshot-verified.
- **Phase 3:** (a) an **autonomous AI brain** (`anima-agent` `WanderBrain`) consumes
  `Observation` and emits `Action` — explores, greets speakers, flees reds, grabs
  items — verified playing live on the server (the AI-native loop, the project's
  thesis). (b) the renderer now paints **real UO terrain** (avg colors decoded from
  `artLegacyMUL.uop`: grass/dirt roads/water + buildings) — screenshot-verified.
**Playable milestone:** a human can now actually play (top-down). `anima-net`'s `play`
bin holds a live Session, serves `web/` + `/scene.json` over HTTP (tiny_http), and
accepts `POST /input` (walk/say/use/attack/pickup/war) executed on the live session.
The browser sends WASD/arrow + chat input → verified: keyed input walks and talks on
the real server. Run: `cargo run -p anima-net --bin play -- 127.0.0.1 2594 <u> <p>`
then open `http://127.0.0.1:8090/`.

**Isometric renderer:** the web client now draws **real UO tile sprites in iso
projection** — `anima-assets::art` decodes land (44×44 diamond) + static (RLE) art
and PNG-encodes it (`Image::to_png`); the `play` server serves `/art/land/<g>.png`
and `/art/static/<g>.png` (cached); the scene includes per-tile land graphic + a
window static list; `web/main.js` streams those textures and draws the diamond
field + statics (grass, New Haven roads, buildings), falling back to avg-color
diamonds while textures load. Screenshot-verified.

**Mobile sprites:** `anima-assets::anim` decodes legacy `anim.mul`/`.idx` (palette +
RLE frames; people base `(body-400)*175+35000`, monster `body*110`; groups Stand=4/
Walk=0; 8→5 direction map + mirror). The `play` server serves `/anim/<body>/<dir>.png`
(Stand frame, mirror baked) and the scene carries each mobile's `body`/`dir`; the
renderer draws real body sprites (player + humans), markers as fallback. Known gaps to
iterate on: only people bodies (400/401/…) resolve (no `body.def` remap yet → monsters
fall back to markers); no walk-frame cycling yet (idle pose only); sprite foot-anchor
is approximate (bottom-center, no centerX/Y offset).

Remaining for full human-playable fidelity: walk/attack **animation cycling**, monster
body remap (`body.def`/`mobtypes.txt`), **gumps**
(paperdoll/backpack/vendor, `gumpartLegacyMUL.uop`), **click-to-interact** +
targeting UI (the `0x6C` cursor + `build_target_response` + `TargetCursor` plumbing
is in place; needs Action variants + browser wiring), audio. Then optionally a Tauri
standalone shell. (Parallel track: `anima-net::json` + the `anima2` Python brain.)

### Verified end-to-end against ServUO (127.0.0.1:2594)
- Two-phase login: account → server select → reconnect → game login → **char create**
  (new account) and **select** (re-login) → LoginConfirm. Real serial/pos returned.
- Huffman decompression + framing of the live compressed game stream.
- Perception: 0x11 status (name/hp/stats matched the created character), items, journal.
- Movement: walk requests (0x02) with sequence + confirm(0x22)/deny(0x21) + resync.
- Asset readers: UOP map + tiledata + statics parsed against real data — the spawn
  tile's Z (14) matched the server's login Z exactly.
- **Capstone:** `navigate_to` walked the avatar (3503,2574)→(3493,2564) on the live
  server via A* (22 confirms, 2 denies → blacklisted + routed around). ARRIVED ✓.
- Gotcha learned: the server **denies movement until the client answers the 0xBD
  ClientVersion request** — handled in the session.

### How to run it
```
cd ~/dev/uo/servuo && MONO_GAC_PREFIX=/opt/homebrew nohup mono ServUO.exe -noconsole &
cd ~/dev/uo/anima-client
cargo run -p anima-net -- 127.0.0.1 2594 <user> <pass>   # auto-creates the account
```

---

## 1. What we're building (one paragraph)

A **new, from-scratch Ultima Online client**, designed **AI-native** and
**cross-platform (Windows + macOS)**. It is the companion "body" for the
[`anima`](../../anima) AI-player project. The central artifact is a **headless
game core** (`anima-core`, Rust) that speaks the UO protocol and maintains world
state with **no rendering/UI/audio**. A thin renderer (web: TypeScript + PixiJS)
sits on top for humans. The same core serves three consumers: AI agents
(headless, many), a browser client (core compiled to WASM), and a desktop
standalone app (Tauri, native TCP).

---

## 2. Decision history (the *why* — do not re-litigate without reason)

These were settled deliberately over a long design discussion. Each row is a
decision + the reasoning so a future session understands the constraints.

| # | Decision | Why |
|---|----------|-----|
| D1 | **Structured observation/action, never pixels/vision** | AlphaStar & OpenAI Five both used structured game-state interfaces, not pixels, for performance + scale. UO exposes full game state in packets, so vision adds nothing but cost/brittleness. |
| D2 | **Separate two layers: Interface (body) ⊥ Brain (decision)** | Lets us swap brains (scripted/RL/LLM) and backends independently. The boundary is an explicit Observation/Action contract. |
| D3 | **Core ⊥ Renderer split; core-first** | ClassicUO is 152k LOC, ~40% of which (Game/UI gumps alone = 50k, ~33%) is throwaway for an AI. The AI-relevant core is ~40k LOC. Making the headless core the primary artifact is the whole reason to build new vs fork. |
| D4 | **Build new instead of forking ClassicUO** | User wants a *serious* AI-native client where the headless core is first-class and reused across agents/browser/desktop. ClassicUO can't give that cleanly (human-first, GPU-coupled init). |
| D5 | **Language = Rust for the core** | One codebase compiles to **native** (agents, desktop) *and* **WASM** (browser). No-GC perf + strong concurrency for many parallel agents. Graph-y world model handled via HashMap-by-serial now, `slotmap`/ECS later. |
| D6 | **Frontend = Web (TypeScript + PixiJS), WebGPU + WebGL2 fallback** | User chose web. Cross-platform by definition; PixiJS is a battle-tested 2D isometric renderer that cuts rendering plumbing. WebGPU is Metal-backed on Mac (no Apple GPTK needed). |
| D7 | **Standalone via Tauri (desktop) + optional browser (PWA)** | User wants standalone. See the TCP constraint (§4): only a native shell can open raw TCP, so desktop standalone is self-contained; pure browser needs a relay. Same web frontend ships both ways. |
| D8 | **Apple Game Porting Toolkit is NOT used** | GPTK ports *existing Windows/DirectX* games to Mac (DXIL→Metal shader conversion, Wine-style eval env). For a new cross-platform build you target Metal natively via wgpu/WebGPU — there's no DirectX to translate. |
| D9 | **`anima-core` is the name** (was tentatively `uocore`) | User's choice. It's the headless heart, not a separate product. |
| D10 | **Sans-IO protocol core** — `anima-core` never touches a socket | Maintainability + the WASM requirement (D5). Protocol logic is pure (feed bytes → get packets/events; produce bytes to send). The actual TCP/WebSocket loop is a thin shim *outside* the core, injected per platform. Makes the whole protocol unit-testable from byte vectors and identical on native/WASM. The login handshake is a `LoginMachine` that emits `LoginDirective`s the driver executes. |

### Rejected / deferred
- **Forking ClassicUO headless** (strategy A/B from the discussion): viable and faster to a working agent, but rejected in favor of a clean new core (D4). Still a useful *reference* (§7).
- **Go for the core**: simpler concurrency but weaker WASM story and no shared-language renderer. Rust won (D5).
- **Full client port** (rendering included): bad ROI (~40% wasted). Never do this.

---

## 3. Target architecture

```
                  anima-core  (Rust — the headless heart)
                  net · world · assets · path     (NO rendering/UI/audio)
        ┌──────────────────┼──────────────────────┐
   native lib            WASM                  Tauri backend (native)
        ▼                  ▼                        ▼
   AI agents          browser client          desktop standalone
   (many, headless)   (anima-core = WASM       (Tauri: direct TCP,
                       + WebSocket relay)       reads local UO data)
```

- **anima-core** — protocol, world state, asset (`.mul`/`.uop`) IO, pathfinding. Pure logic, platform-agnostic.
- **Renderer** (web/TS/PixiJS) — the *only* place cross-platform graphics concerns live. Reads world state, draws; sends user intents back.
- **Brain** — decision-making (AI or human input) lives *above* the core, never inside it.

### The Observation/Action contract (the Interface↔Brain boundary, D2)
Not yet codified in code; design it before wiring an AI. Shape:
- **Observation** (core → brain): player state (pos/hp/mana/stam/skills), nearby mobiles & items, journal deltas, war/hidden flags, targeting/gump prompts pending.
- **Action** (brain → core): move(dir, run), use/double-click(serial), attack(serial), cast(spell), say(text), target(serial|xyz), pickup/drop/equip, gump-response.
Keep it a stable schema so scripted/RL/LLM brains and the native/WASM backends all plug into the same thing.

### AI training layers (context for later, not Phase 1)
From the design discussion — when the AI side is built, structure it as:
- **Play plane** = the Observation/Action contract above (normal player surface).
- **Control plane** = scenario control for repeatable curriculum/RL: reset, teleport, grant items, set skills, measure. *This needs a GM account or server save/restore — a UO client is "just a player" and can't do it alone.* The existing `anima` Foundry kernel already implements a GM control plane; reuse/formalize it. Keep it OUT of both the core and the brain — it's a separate component.
- **Director/Curriculum** = automatic curriculum (Voyager-style task proposal) + skill library. Sandbox UO has no reward gradient, so a curriculum + LLM priors (the `anima` companion wiki = the "textbook") is the fastest accelerant — faster than gradient RL. In-game New Haven tutorial = free curriculum stage 0 (reachable via the play plane, no GM).

---

## 4. The defining constraint: browsers can't open raw TCP

UO is raw TCP. Browsers forbid arbitrary TCP sockets. This drives the
standalone/relay split:

- **Desktop (Tauri):** the Rust backend opens TCP directly → fully self-contained standalone. **Recommended primary target.**
- **Browser:** needs a thin **WebSocket↔TCP relay** (dumb byte pump; protocol parsing still runs in-browser via `anima-core` WASM). Not fully standalone (relay required) but zero-install.

**Assets reinforce this:** UO `.mul/.uop` files are large and copyrighted (Broadsword/EA) — cannot be redistributed. Users must supply their own UO install. Desktop reads local files natively (easy); pure browser needs the Chromium-only File System Access API (Safari/Firefox gaps) or manual upload. → another reason desktop standalone is the cleaner primary.

**Tauri vs Electron caveat:** Tauri uses the OS webview (WKWebView on Mac = Safari engine), so WebGPU/API maturity can lag Chrome. Mitigate with a WebGL2 fallback, or use Electron (bundles Chromium → consistent rendering, larger binary). Decide when the renderer starts.

---

## 5. Repo layout & current status

```
anima-client/
├── Cargo.toml                 # Rust workspace (anima-core, anima-net, anima-assets)
├── README.md · CLAUDE.md · .gitignore
├── docs/DESIGN.md             # ← this file
└── crates/
    ├── anima-core/            # headless protocol + world + path + contract (zero-dep, sans-IO)
    │   └── src/
    │       ├── lib.rs · types.rs        # Serial, Position, Direction
    │       ├── agent.rs                 # Observation/Action contract + World::observe
    │       ├── net/
    │       │   ├── packet.rs            # big-endian reader/writer
    │       │   ├── lengths.rs           # packet-length framing table
    │       │   ├── framing.rs           # frame decoder + game-mode (Huffman) + StreamDecoder
    │       │   ├── huffman.rs           # server→client decompression
    │       │   ├── login.rs             # builders/parsers + LoginMachine (+ char create)
    │       │   ├── game.rs              # game packet codec → World mutation
    │       │   ├── movement.rs          # walk requests + Walker (seq/confirm/deny)
    │       │   └── outgoing.rs          # client version + action builders
    │       ├── world/mod.rs             # World/Mobile/Item/PlayerStats/journal/fastwalk
    │       └── path/mod.rs              # A* + Terrain trait
    ├── anima-assets/          # .mul/.uop readers (dep: flate2); impls path::Terrain
    │   └── src/{lib,uop,tiledata,map}.rs
    ├── anima-net/             # native TCP driver: Session (login, pump, walk, navigate, actions)
    │   └── src/lib.rs · main.rs (`anima-login`) · bin/scene.rs (`scene` → web/scene.json)
    ├── anima-wasm/            # wasm-bindgen wrapper: WasmClient (feed bytes → Observation JSON)
    │   └── src/lib.rs         # build: `wasm-pack build crates/anima-wasm --target web`
    └── anima-agent/           # autonomous brains on the contract
        └── src/lib.rs (Brain, WanderBrain) · main.rs (`anima-agent` runner bin)
web/                          # Phase 2 renderer (outside the Cargo workspace)
├── index.html · main.js      # PixiJS minimap + HUD, polls scene.json
└── vendor/pixi.min.js        # vendored PixiJS v8 (standalone, no CDN)
```

### Running the Phase 2 renderer
```
cd ~/dev/uo/servuo && MONO_GAC_PREFIX=/opt/homebrew nohup mono ServUO.exe -noconsole &
cd ~/dev/uo/anima-client
cargo run -p anima-net --bin scene -- 127.0.0.1 2594 <user> <pass> web/scene.json &
( cd web && python3 -m http.server 8011 )      # open http://127.0.0.1:8011/
```
The scene bridge logs in, patrols, and rewrites `web/scene.json` ~2×/s; the page
polls it. (Future: swap the JSON bridge for `anima-wasm` in-browser + a
WebSocket↔TCP relay so the browser runs the core directly — §4.)

**Done:** all three crates build, clippy clean, `cargo test` = 36 passing (+1
ignored real-data asset test). Validated end-to-end against a live ServUO (see the
status block at the top).

### Sans-IO contract (how a driver uses the login flow)
```
let (mut m, initial) = LoginMachine::start(cfg);   // cfg = user/pass/server/slot
// open TCP to login server; write `initial` (Seed + AccountLogin)
loop {
    framer.feed(bytes_from_socket);
    while let Some(frame) = framer.pop()? {
        for directive in m.on_packet(&frame)? {
            match directive {
                Send(b)                        => socket.write(b),
                ReconnectToGameServer { then } => { reconnect_to_game_server();
                                                    framer = game_mode_framer(); // +Huffman
                                                    socket.write(then); }
                Done(result)                   => return result, // we're in the world
            }
        }
    }
}
```
The driver is the only code that knows about sockets — write it once for native
(TCP) and once for WASM (WebSocket). The core stays pure.

---

## 6. Roadmap

### Phase 1 — `anima-core` (rendering = 0). ✅ COMPLETE — all validated vs live ServUO.
1. ✅ Connection state machine + two-phase login (sans-IO `LoginMachine`).
2. ✅ Huffman decompressor + game-mode framer (`net/huffman.rs`, `framing.rs`).
3. ✅ Native TCP driver (`anima-net::Session`) — connects to ServUO end-to-end.
4. ✅ Character create + select (`build_create_character`, `CharacterAppearance`).
   (Delete deferred — format documented in `anima`, not needed for the goal.)
5. ✅ Game packet codec → World mutation (`net/game.rs`): 0x20/0x77/0x78/0x1A/0x1D/
   0x11/0xA1-3/0x1C/0xAE/0xBF.
6. ✅ Movement (`net/movement.rs`): walk 0x02 + seq + confirm/deny + resync + fastwalk.
7. ✅ Asset readers (`anima-assets`): UOP map, tiledata.mul (HS), statics — real data verified.
8. ✅ Pathfinding (`path/`): A* + `Terrain` trait, Z-aware, diagonal-safe.
9. ✅ Observation/Action contract (`agent.rs`) + `Session::apply_action` / `navigate_to`.

**Remaining tail (optional, deferred):** delete-character; broader packet coverage
(combat 0x0B, containers 0x24/0x25/0x3C, gumps 0xB0/0xDD, targeting 0x6C, vendors);
fastwalk is consumed but most shards send 0 keys.

### Phase 2 — renderer (web) + WASM. ✅ COMPLETE.
- ✅ `anima-core` compiles to `wasm32-unknown-unknown`; `anima-wasm` (wasm-bindgen)
  exposes `WasmClient` (feed bytes → outbox + `Observation` JSON). `wasm-pack build`
  produces the browser module.
- ✅ PixiJS renderer (`web/`) draws a live minimap (walkability/Z) + mobiles/items +
  HUD from `Observation`, fed by `anima-net`'s `scene` bridge. Screenshot-verified.

**Remaining tail (deferred):** wire `anima-wasm` into the browser via a
WebSocket↔TCP relay (so the browser runs the core, not a JSON bridge); a Tauri
shell for a true standalone desktop app.

### Phase 3 — AI brains + real art. ✅ CORE THREADS COMPLETE.
- ✅ `anima-agent`: `Brain` trait (`decide(&Observation)->Vec<Action>`) + `WanderBrain`
  (explore/greet/flee/grab) — runs live: `cargo run -p anima-agent -- 127.0.0.1 2594
  <u> <p> [ticks]`. The full perception→decision→action loop on the real server.
- ✅ `anima-assets::art`: decodes `artLegacyMUL.uop` (land diamond + static RLE,
  ARGB1555→RGBA) + per-land-graphic average color; the scene bridge sends real tile
  colors and the renderer paints actual UO terrain. (Note: art UOP paths use a
  `.tga` extension, unlike the map's `.dat`.)

**Remaining tail (human-playable polish):** full isometric sprite blitting (draw the
44×44 land diamonds + static/animation sprites, not just avg colors), mobile
animations (`AnimationFrame*.uop`), gumps (`gumpartLegacyMUL.uop`), audio; richer
brains (RL/LLM over the contract); browser WASM + WebSocket↔TCP relay, or a Tauri shell.

---

## 7. Reference sources (use these, don't reinvent)

All present on the same machine under `~/dev/uo/`:

- **`~/dev/uo/anima`** — the Python AI-player. Its `anima/client/` (packet codec, `packets.py`, `parser.py`), `anima/perception/` (WorldState), `anima/pathfinding.py`, `anima/map.py`, `anima/uop.py` are a **partial port of exactly this core in Python**. Use as the **spec/oracle** for porting. Its `CLAUDE.md` has concise protocol notes (mirrored in §8).
- **`~/dev/uo/anima/uo_proxy`** — a packet-logging proxy. Capture real packet streams and turn them into **golden tests** for the Rust codec (port handler-by-handler, validate against capture = strangler migration, low risk).
- **`~/dev/uo/classicuo`** — the C# reference client (your fork, `.NET 10`). Authoritative for packet handlers (`Network/PacketHandlers.cs` ~119 handlers, `Network/OutgoingPackets.cs` ~80), world model (`Game/World.cs`, `Game/GameObjects/`), file formats (`ClassicUO.Assets`, `ClassicUO.IO`), pathfinding (`Game/Pathfinder.cs`), and the login flow (`Game/Scenes/LoginScene.cs`). **Read these when implementing the equivalent Rust.**
- **ServUO** (`~/dev/uo/servuo`) and **ModernUO** — server-side view of the protocol for cross-checking. ModernUO is a clean modern .NET rewrite, good for world-model reference.
- **`~/dev/uo/uowiki`** — companion knowledge base (game facts). Also exposed as an MCP server (`wiki_search`, `wiki_read_page`) in sessions here.

---

## 8. Protocol knowledge (concrete, for Phase 1)

Distilled from `anima/CLAUDE.md` — verify against ClassicUO/captures while implementing.

- **Endianness:** all network values are **big-endian**. (`anima-core`'s `PacketReader/Writer` are BE.)
- **No encryption:** send plaintext TCP. (Treat accounts as disposable — login sends user/pass in plaintext.)
- **Compression:** **Huffman** decompression required for **game-phase server→client** packets only (not login phase, not client→server). Implement in `net/`.
- **Packet framing:** fixed = `[id][payload]` (length from a static table); variable = `[id][len: u16 BE][payload]`. Need the packet-length table (see ClassicUO `Network/PacketsTable.cs`).
- **Two-connection login flow:**
  1. Connect to login server → `Send_FirstLogin(account, password)`.
  2. Receive server list → `Send_SelectServer(index)`.
  3. Receive relay (game server ip/port + auth key) → **reconnect** to game server.
  4. Receive character list → `Send_SelectCharacter(index, name, ...)`.
  5. `EnterWorld (0x1B)` → create player, load map → `LoginComplete (0x55)`.
- **Movement protocol:**
  - Walk packet `0x02`: `[dir|run_flag] [seq: u8] [fastwalk_key: u32]` — 7 bytes.
  - `seq`: 1–255, wraps to 1 (never 0). Max **5** pending steps.
  - Server replies `ConfirmWalk (0x22)` or `DenyWalk (0x21)` (deny → resync to server position).
  - Throttle: ~400ms walk, ~200ms run, ~100ms mounted run.
  - Direction low 3 bits = facing; `0x80` = running flag (see `types::Direction`).
- **Key packet ids** (start set): `0x1B` EnterWorld, `0x55` LoginComplete, `0x11` CharacterStatus, `0x20` UpdatePlayer, `0x78` UpdateObject, `0x1A` UpdateItem, `0x25` UpdateContainedItem, `0x2E` EquipItem, `0x3A` UpdateSkills, `0x1C`/`0xAE` Talk/UnicodeTalk, `0xB0` OpenGump, `0x6C` Target. (Full list: ClassicUO `PacketHandlers.cs`.)

---

## 9. Toolchain & commands

- **Rust:** 1.96 + cargo present. **Node:** 26 / npm 11 present.
- WASM target (add when needed): `rustup target add wasm32-unknown-unknown`.

```bash
cd ~/dev/uo/anima-client
cargo build      # build workspace
cargo test       # run anima-core unit tests
cargo doc --open # browse the core API docs
```

- **macOS / Apple Silicon note:** the core is pure logic, no native graphics deps, so no arm64 friction (unlike ClassicUO's SDL/FNA). Friction only appears at the renderer/Tauri stage — prefer arm64-native deps, WebGL2 fallback for WKWebView WebGPU gaps.

---

## 10. Open decisions (resolve when you reach them)

- **Tauri vs Electron** for the desktop shell (renderer consistency vs binary size) — §4.
- **WASM binding strategy** — `wasm-bindgen` + a JS API surface vs a message/snapshot protocol mirroring the Observation/Action contract.
- **World model data structure at scale** — HashMap-by-serial now; move to `slotmap`/ECS (or Bevy ECS if Bevy becomes the renderer) if entity churn/perf demands.
- **Relay implementation** — standalone tiny Rust/Go service, or bundle into the same dev server. Only needed for the browser path.
- **How the AI brain attaches** — in-process (link `anima-core` as a Rust lib + Rust brain) vs out-of-process (brain in Python/anima over the Observation/Action contract via IPC). The existing `anima` is Python, so an IPC contract may be the pragmatic bridge.

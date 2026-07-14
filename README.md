# anima-client

A **new, from-scratch Ultima Online client**, built AI-native and cross-platform
(Windows + macOS). Companion to the [`anima`](../anima) AI player project.

![anima-client running against a live ServUO shard — real isometric UO terrain
and sprites, minimap, and HUD](docs/img/screenshot.png)

*The web/PixiJS renderer running live against a ServUO shard: real
`artLegacyMUL`/`anim` sprites in isometric projection, minimap, and a full HUD
(stats, journal, controls). The same headless `anima-core` also drives AI agents
and the Tauri desktop app.*

> **New here? Read [`docs/DESIGN.md`](docs/DESIGN.md)** — the full design & handoff
> doc (decision history, architecture, roadmap, protocol notes, references). This
> project is resumable from that doc alone.

## Thesis

Existing clients (ClassicUO) are *human-first*, with automation bolted on. This
project is **core-first**: a headless game core (`anima-core`) is the primary
artifact, and the human-facing renderer is just *one* front-end among several.
The same core powers AI agents, a browser client, and a desktop app.

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

Cross-platform concern is isolated to the thin **renderer** layer; the core is
pure logic and platform-agnostic.

## Stack

- **Core:** Rust `anima-core` → native (agents, desktop) + WASM (browser)
- **Renderer / UI:** plain JavaScript + PixiJS (2D isometric), WebGPU with WebGL2 fallback
- **Networking:** desktop = direct TCP (Tauri/Rust); browser = thin WebSocket↔TCP relay
  - (browsers can't open raw TCP — this constraint drives the desktop/relay split)
- **Packaging:** Tauri for standalone Win/Mac desktop; PWA/web for zero-install

## Layout

```
anima-client/
├── Cargo.toml                 # Rust workspace
├── crates/
│   ├── anima-core/            # headless core: protocol, world, path, contract, gump layout
│   │                          #   (sans-IO, near-zero-dep: one exception, miniz_oxide,
│   │                          #   for the protocol-mandated 0xDD zlib)
│   │   └── src/{lib,types,agent,gump_layout}.rs · net/ · world/ · path/ · tests/golden.rs
│   ├── anima-assets/          # .mul/.uop readers: map/tiledata/anim/art/gump/hues/sound/…
│   ├── anima-net/             # native TCP driver (Session) + `anima-login`/`play`/`scene`/`anima-agent`/`cmd` bins
│   ├── anima-wasm/            # wasm-bindgen wrapper: WasmClient (feed bytes → Observation JSON)
│   └── anima-agent/           # in-process autonomous brains (Brain trait, WanderBrain)
└── web/                       # plain JavaScript + PixiJS renderer (outside the Cargo workspace)
```
Planned sibling: `crates/anima-desktop` (Tauri, standalone desktop shell) — not yet
started; everything else above is built.

## Status — Phases 1–3 COMPLETE ✅, incl. the Phase 3 tail (validated against a live ServUO)

The headless agent connects to a real UO server, logs in, builds a live `World`, and
**navigates by A\* over real UO map data**. An **autonomous AI brain** (`WanderBrain`)
consumes the same `Observation` and **plays the game live** (explores, greets, flees
reds, grabs items) — the AI-native loop, the whole point. A human can also just
**play**: the `play` HTTP server renders real UO terrain, full isometric sprites,
resolved mobile/monster animation (legacy + UOP), gumps (paperdoll/containers/
vendor/spellbook/books/party), audio, and secure trading in a **web/PixiJS
renderer**; `anima-core` also compiles to **WASM**. 170 tests, clippy clean.

Crates: `anima-core` (protocol/world/path/contract — sans-IO, near-zero-dep: one
exception (miniz_oxide, for the protocol-mandated 0xDD zlib), WASM-ready),
`anima-assets` (.mul/.uop + art/anim/gump/sound readers), `anima-net` (TCP driver +
`anima-login`/`play`/`scene`/`anima-agent`(NDJSON bridge)/`cmd` bins),
`anima-wasm` (browser bindings), `anima-agent` (in-process autonomous brains); plus
`web/` (PixiJS). Full detail + decision history: [`docs/DESIGN.md`](docs/DESIGN.md).

### Roadmap
1. ✅ **Phase 1 — headless core:** protocol, world, perception, movement, assets,
   A\* pathfinding, Observation/Action contract.
2. ✅ **Phase 2 — renderer + WASM:** `anima-core`→wasm32, `anima-wasm`, live PixiJS
   minimap/HUD fed by the scene bridge.
3. ✅ **Phase 3 — AI + real art + human-playable polish:** `anima-agent` plays
   autonomously on the contract; the `play` server is a full human-playable client
   (real terrain/sprites/animation/gumps/audio/trading).
   *Remaining:* richer/RL/LLM brains; browser WASM+relay; Tauri; `multi.mul` houses;
   sitting; treasure maps. See [`docs/DESIGN.md`](docs/DESIGN.md) §6 for detail.

## Build & run

```bash
cargo build && cargo test --workspace   # 170 tests (+16 ignored real-data-file tests)
# boot a local ServUO (port 2594), then pick one:
cargo run -p anima-net --bin play -- 127.0.0.1 2594 <user> <pass>  # human-playable (open :8090)
ANIMA_LOGIN=1 cargo run -p anima-net --bin play                    # same, but log in via the browser page
cargo run -p anima-agent -- 127.0.0.1 2594 <user> <pass> 40       # autonomous AI brain
# or the live web renderer (real terrain):
cargo run -p anima-net --bin scene -- 127.0.0.1 2594 <user> <pass> web/scene.json &
( cd web && python3 -m http.server 8011 )   # → http://127.0.0.1:8011/
```

WASM module: `cargo install wasm-pack && wasm-pack build crates/anima-wasm --target web`.

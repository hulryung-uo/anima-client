# anima-client

A **new, from-scratch Ultima Online client**, built AI-native and cross-platform
(Windows + macOS). Companion to the [`anima`](../anima) AI player project.

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
- **Renderer / UI:** TypeScript + PixiJS (2D isometric), WebGPU with WebGL2 fallback
- **Networking:** desktop = direct TCP (Tauri/Rust); browser = thin WebSocket↔TCP relay
  - (browsers can't open raw TCP — this constraint drives the desktop/relay split)
- **Packaging:** Tauri for standalone Win/Mac desktop; PWA/web for zero-install

## Layout

```
anima-client/
├── Cargo.toml                 # Rust workspace
└── crates/
    ├── anima-core/            # headless core: protocol, world, path, contract
    │                          #   (sans-IO, zero external deps)
    │   └── src/{lib,types,agent}.rs · net/ · world/ · path/
    ├── anima-assets/          # .mul/.uop readers (map/statics/tiledata; dep: flate2)
    └── anima-net/             # native TCP driver + `anima-login` bin
```
Planned siblings: `crates/anima-desktop` (Tauri), `crates/anima-agent` (AI driver),
and `web/` (TypeScript frontend, outside the Cargo workspace).

## Status — Phases 1–3 (core) COMPLETE ✅ (validated against a live ServUO)

The headless agent connects to a real UO server, logs in, builds a live `World`, and
**navigates by A\* over real UO map data**. An **autonomous AI brain** (`WanderBrain`)
consumes the same `Observation` and **plays the game live** (explores, greets, flees
reds, grabs items) — the AI-native loop, the whole point. A **web/PixiJS renderer**
draws a live minimap painted with **real UO terrain** (colors decoded from
`artLegacyMUL.uop`) + HUD, and `anima-core` compiles to **WASM**. 39 tests, clippy clean.

Crates: `anima-core` (protocol/world/path/contract — sans-IO, zero-dep, WASM-ready),
`anima-assets` (.mul/.uop + art readers), `anima-net` (TCP driver + `anima-login` /
`scene` bins), `anima-wasm` (browser bindings), `anima-agent` (autonomous brains);
plus `web/` (PixiJS). Full detail + decision history: [`docs/DESIGN.md`](docs/DESIGN.md).

### Roadmap
1. ✅ **Phase 1 — headless core:** protocol, world, perception, movement, assets,
   A\* pathfinding, Observation/Action contract.
2. ✅ **Phase 2 — renderer + WASM:** `anima-core`→wasm32, `anima-wasm`, live PixiJS
   minimap/HUD fed by the scene bridge.
3. ✅ **Phase 3 (core) — AI + real art:** `anima-agent` plays autonomously on the
   contract; renderer paints real UO terrain from `artLegacyMUL.uop`.
   *Tail:* iso sprite blitting, animations, gumps, audio; RL/LLM brains; WASM+relay/Tauri.

## Build & run

```bash
cargo build && cargo test            # 39 tests
# boot a local ServUO (port 2594), then pick one:
cargo run -p anima-net   -- 127.0.0.1 2594 <user> <pass>          # navigate demo
cargo run -p anima-agent -- 127.0.0.1 2594 <user> <pass> 40       # autonomous AI brain
# or the live web renderer (real terrain):
cargo run -p anima-net --bin scene -- 127.0.0.1 2594 <user> <pass> web/scene.json &
( cd web && python3 -m http.server 8011 )   # → http://127.0.0.1:8011/
```

WASM module: `cargo install wasm-pack && wasm-pack build crates/anima-wasm --target web`.

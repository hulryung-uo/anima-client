# anima-client

A **new, from-scratch Ultima Online client**, built AI-native and cross-platform
(Windows + macOS). The **body** of the Anima family: a headless Rust core that
speaks UO, plus renderers on top of it.

![The web/PixiJS renderer live against a ServUO shard: real isometric UO terrain
and sprites, minimap, and HUD](docs/img/screenshot.png)

*Live against a real ServUO shard — genuine `artLegacyMUL`/`anim` sprites in
isometric projection, the minimap, and the HUD. No pre-baked scene: every tile,
sprite and animation frame is read from your own UO installation and driven by
real server packets.*

![The same street at night: the world is dark, the street lamp and a carried
torch light it](docs/img/night.png)

*The same street after `globallight 26`. Darkness, `light.mul`'s real
hand-drawn light shapes rather than circles, and a torch that lights from the
hand that carries it — with a wall able to block the glow behind it.*

> **New here? Read [`docs/DESIGN.md`](docs/DESIGN.md)** — the full design & handoff
> doc (decision history, architecture, roadmap, protocol notes, references). This
> project is resumable from that doc alone.

## The Anima family

Four repositories, one idea: **AI characters that actually live in Britannia.**
This one is the body; the others are minds, and they are separate repos because
a brain should be replaceable without touching the thing that speaks the
protocol.

| Repo | What it is |
|---|---|
| **[`anima-client`](https://github.com/hulryung-uo/anima-client)** (here) | The **body**. Headless Rust core (`anima-core`) that logs in, keeps a live `World`, paths with A\*, and reads UO's own `.mul`/`.uop` files — plus the renderers on top: a browser client, a Tauri desktop app, and a human-playable `play` server. |
| **[`anima`](https://github.com/hulryung-uo/anima)** | **Anima Foundry** — an AI that *develops* AI players: it mutates their code, evaluates every variant against a live server, and keeps the best of each behavioural kind. Evolution, not just automation. |
| **[`anima2`](https://github.com/hulryung-uo/anima2)** | A rule-based **brain** on this body. Reads a structured world, decides, emits actions — and never parses a packet or touches a socket. |
| **[`anima-agent`](https://github.com/hulryung-uo/anima-agent)** | The **LLM-first brain**, successor to `anima2`. Same contract, different way of deciding. |

The split is the whole design. A brain receives an `Observation` and returns an
`Action` (`anima-contract-json`); it never sees a byte of the wire. That is what
lets a Python LLM agent, an evolved Foundry variant and a Rust in-process brain
all drive the same character through the same core.

> **Naming, because it trips people up:** the crate `crates/anima-agent` in THIS
> repo is not the [`anima-agent`](https://github.com/hulryung-uo/anima-agent)
> repo. The crate holds small in-process Rust brains (`WanderBrain`,
> `HunterBrain`, `LlmBrain`) used to exercise the contract from inside the
> workspace. The repo is the real LLM agent, in Python, on the other side of it.

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
│   ├── anima-contract-json/   # shared versioned Observation/Action JSON adapter
│   ├── anima-net/             # native TCP driver (Session) + `anima-login`/`play`/`scene`/`anima-agent`/`cmd` bins
│   ├── anima-wasm/            # wasm-bindgen wrapper: WasmClient (feed bytes → Observation JSON)
│   ├── anima-agent/           # in-process autonomous brains (Brain trait, WanderBrain)
│   └── anima-desktop/         # Tauri standalone shell (native TCP + embedded web renderer)
└── web/                       # plain JavaScript + PixiJS renderer (outside the Cargo workspace)
```

## Status

**Playable, and played.** A human can log in and play: real terrain, full
isometric sprites, resolved mobile and monster animation (legacy + UOP), gumps
(paperdoll, containers, vendors, spellbook, books, party), audio, secure
trading, macros, name plates, and a world map. An **autonomous brain** consumes
the same `Observation` and plays live. `anima-core` also compiles to **WASM**.

Latest release: **[v0.6.0](https://github.com/hulryung-uo/anima-client/releases/latest)**
— signed and notarized on macOS, installable on Windows.

The work is measured against ClassicUO handler by handler and validated against
a live ServUO shard;
[`docs/CLASSICUO_GAPS.md`](docs/CLASSICUO_GAPS.md) is the honest ledger of it,
including — deliberately — what each change was **not** verified against.

Quality gates run in CI on every push: `cargo clippy --all-targets -D warnings`,
the workspace tests, a wasm32 check, and two that exist because of specific
bugs that shipped — one compiles every `web/js` file *together* in the page's
real load order (they share one scope, and a duplicate top-level `const` is a
SyntaxError that kills the client while `node --check` passes it), and one
*runs* them head-less in a fake DOM.

### Roadmap
1. ✅ **Phase 1 — headless core:** protocol, world, perception, movement, assets,
   A\* pathfinding, Observation/Action contract.
2. ✅ **Phase 2 — renderer + WASM:** `anima-core`→wasm32, `anima-wasm`, live PixiJS
   renderer fed by the scene bridge.
3. ✅ **Phase 3 — AI + real art + human-playable polish:** brains play
   autonomously on the contract; the `play` server is a full human-playable
   client.
4. ⏳ **Ongoing — ClassicUO parity.** Tracked in
   [`docs/CLASSICUO_GAPS.md`](docs/CLASSICUO_GAPS.md), which is a record of what
   was done rather than a description of what is left; the way to find the next
   gap is to re-read ClassicUO against the shipped code.

## Build & run

```bash
cargo build && cargo test --workspace   # ignored tests require local real-data files
scripts/check.sh                        # every gate CI runs, in CI's order
# boot a local ServUO (port 2594), then pick one:
cargo run -p anima-net --bin play -- 127.0.0.1 2594 <user> <pass>  # human-playable (open :8090)
ANIMA_LOGIN=1 cargo run -p anima-net --bin play                    # same, but log in via the browser page
cargo run -p anima-agent -- 127.0.0.1 2594 <user> <pass> 40       # in-process Rust brain
cargo run -p anima-net --bin anima-agent -- 127.0.0.1 2594 <u> <p> # NDJSON bridge for an
                                                                   # external brain (anima2 /
                                                                   # anima-agent, over stdio)
# or the live web renderer (real terrain):
cargo run -p anima-net --bin scene -- 127.0.0.1 2594 <user> <pass> web/scene.json &
( cd web && python3 -m http.server 8011 )   # → http://127.0.0.1:8011/
```

Browser login is a two-step flow: after account authentication, it shows the
character names and slots reported by the server. Choose one of those characters
to enter the world, or enable **Create a new character** and choose the name,
gender, profession, stats, and starting city. New characters use the account's
first empty slot without deleting an existing character; creation is disabled
when the server reports that every slot is occupied. Existing characters can be
deleted from the same list after an explicit irreversible-action confirmation;
the refreshed server list is displayed before any subsequent choice. **Back**
cancels the pending game-server connection and restores the account form.

WASM module: `cargo install wasm-pack && wasm-pack build crates/anima-wasm --target web`.
Browser transport: `cargo run -p anima-relay -- 127.0.0.1:2595 127.0.0.1:2594`
bridges a WebSocket to the shard's raw TCP (browsers cannot open sockets). It
only dials targets named on its command line — never one the client picks.
ClassicUO compatibility work is tracked in
[`docs/CLASSICUO_GAPS.md`](docs/CLASSICUO_GAPS.md).

## License

Dual-licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option — the Rust ecosystem's usual pairing, and what every crate here
has declared since the initial commit. The license *files* only arrived later,
which is why GitHub reported this repository as unlicensed for a while; the
declaration was never the missing part.

**This covers the code in this repository and nothing else.** Ultima Online's
data files (`.mul`/`.uop` — art, maps, animation, sound, clilocs) are
copyrighted by Broadsword/EA and are neither included nor redistributable: you
supply your own UO installation and point the tools at it (see the `play`
binary's `data_dir` argument). Nothing here grants any right to that content.

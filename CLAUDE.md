# CLAUDE.md — anima-client

**Read [`docs/DESIGN.md`](docs/DESIGN.md) first.** It is the source of truth:
decision history (the *why*), target architecture, roadmap, protocol/asset
knowledge, and reference sources. This project is designed to be resumable from
that doc alone.

## What this is
A new, from-scratch, **AI-native, cross-platform (Win+Mac)** Ultima Online client.
Core-first: the headless Rust core (`crates/anima-core`) is the primary artifact;
renderers/agents/desktop sit on top. Companion to `../anima` (Python AI player).

## Current phase
**Phases 1–3 COMPLETE, including the Phase 3 "human-playable polish" tail**
(validated vs live ServUO). P1: login/perception/movement/assets/A*/contract. P2:
`anima-core`→wasm32 + `anima-wasm`; web/PixiJS renderer. P3: `anima-agent`
`WanderBrain` plays autonomously live; the human-playable `play` server (`cargo run
-p anima-net --bin play -- 127.0.0.1 2594 <u> <p>`, open `:8090`) renders real
terrain + full iso sprites, walk/attack/typed mobile animation (legacy + UOP,
Body/Bodyconv/Corpse/Equipconv.def remap), gumps, audio, and secure trading. 5
crates (core/assets/net/wasm/agent) + `web/` + `crates/anima-desktop` (Tauri).
**Remaining:** richer/RL/LLM brains, browser WASM+relay.
(Tauri shell, `multi.mul` houses/boats, sitting, treasure maps, custom housing
(0xD8 viewing), and delete-character (0x83) are done.) See DESIGN.md §6.

## Conventions
- **Rust**, edition 2021. Core stays **near-zero-dep: one documented exception**
  (`miniz_oxide`, for the protocol-mandated 0xDD zlib) until there's a concrete
  reason for more (keeps it small + WASM-clean). Justify any new dependency.
- **Big-endian** everywhere (UO wire protocol). Use `net::PacketReader/Writer`.
- **World is the single source of truth.** Packet handlers mutate `World`; the brain
  and renderer only *read* it. The brain never parses bytes.
- No rendering/UI/audio/input in `anima-core` — ever. That's the whole point (DESIGN.md D3).
- Match surrounding code style; keep comments at the existing density.

## Porting method (de-risked)
`../anima` Python codec = **spec**; its `uo_proxy` packet captures = **golden tests**;
`../classicuo` (C# handlers/formats) + ServUO/ModernUO = **cross-check**. Port
handler-by-handler, validate against captures (strangler migration).

## Build / test
```bash
cargo build             # workspace
cargo test --workspace  # ignored tests require local real-data files
```

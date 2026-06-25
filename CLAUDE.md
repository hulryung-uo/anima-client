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
**Phases 1–3 core COMPLETE** (validated vs live ServUO). P1: login, perception,
movement, assets, A* pathfinding, contract — agent navigates a tile on the server.
P2: `anima-core`→wasm32 + `anima-wasm`; web/PixiJS renderer from `Observation` via
the `scene` bridge. P3: `anima-agent` `WanderBrain` plays autonomously live
(`cargo run -p anima-agent -- 127.0.0.1 2594 <u> <p>`); renderer paints **real UO
terrain** decoded from `artLegacyMUL.uop`. 5 crates (core/assets/net/wasm/agent) +
`web/`. **Remaining (Phase 3 tail):** iso sprite blitting, animations, gumps, audio;
RL/LLM brains; browser WASM+relay/Tauri. See DESIGN.md §6.

## Conventions
- **Rust**, edition 2021. Core stays **std-only / zero external deps** until there's
  a concrete reason (keeps it small + WASM-clean). Justify any new dependency.
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
cargo build        # workspace
cargo test         # anima-core unit tests (currently 3 passing)
```

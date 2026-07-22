# ClassicUO compatibility gap inventory

This is the working inventory for bringing `anima-client` toward ClassicUO
feature coverage. It is intentionally broader than the old phase roadmap: a
feature is only marked complete when its packet/state, native driver contract,
scene/agent exposure, and user-facing behavior (where applicable) are present.

## Audit baseline

- Audited: 2026-07-22
- ClassicUO source: local upstream checkout at commit `575f35040`
- Authoritative handler registry:
  `src/ClassicUO.Client/Network/PacketHandlers.cs`
- anima source: `crates/anima-core/src/net/game.rs` plus login and movement
  state machines
- Current game decoder: 76 packet IDs after adding `0xF6 BoatMoving`;
  `0x21`/`0x22` are handled separately by `Walker`

The comparison is mechanical at the packet-ID level, followed by a semantic
review of the corresponding ClassicUO handler. Login-only packets and handlers
that are acknowledgements/no-ops are not counted as missing game UI features.

## Implemented vertical slices

- Login/server/character selection, creation, deletion, cancellation
- World/mobile/item updates, equipment, containers, paperdolls, status/vitals
- `0x2D MobileAttributes`: full HP/Mana/Stamina refresh
- `0x28 EndDraggingItem` / `0x29 DropItemAccepted`: bounded acknowledgement
  events and delayed-ack-safe held-cursor reconciliation
- `0x2C DeathStatus`: ClassicUO-compatible weather reset, death music, timed
  screen banner, peace-mode request, and body-derived death/resurrection
  environment transitions
- `0x38 Pathfinding`: seq-gated server WalkTo requests executed by both native
  and web route drivers with ClassicUO-compatible blocked-goal fallback
- `0x7C OpenMenu` / `0x7D MenuResponse`: concurrent legacy item/icon and gray
  question menus across core state, brain/WASM/native response contracts, scene,
  and browser dialogs
- `0x95 DisplayHuePicker` / `0x95 HuePickerResponse`: concurrent server dye
  pickers, ServUO-compatible `2..=1001` hue normalization, versioned brain/WASM/
  native contracts, a real `hues.mul` palette API, and browser grid/preview UI
- `0x9A ASCIIPrompt` / `0x9A ASCIIPromptResponse`: prompt-kind-aware core state,
  ClassicUO-compatible CP1252/NUL response and cancel packets, versioned brain/
  WASM/native contracts, and the shared browser response dialog
- `0xA5 OpenUrl`: bounded HTTP(S)-only request events, credential/authority and
  control-character validation, versioned brain/scene exposure, and a browser
  consent dialog whose explicit link click uses `noopener`/`noreferrer`
- `0xA6 TipWindow` / `0xA7 TipRequest`: concurrent pageable tips and close-only
  notices, CP1252/CR-normalized text, versioned brain/native/WASM actions, and
  browser windows with ClassicUO-compatible previous/next/close behavior
- `0xAB TextEntryDialog` / `0xAC TextEntryDialogResponse`: concurrent modal
  dialogs with exact callback identity, CP1252 labels/responses, numeric and
  UTF-16 length constraints, explicit OK/Cancel replies, permission-gated silent
  close, versioned brain/native/WASM actions, and browser UI
- `0xB8 CharacterProfile`: CP1252 header plus UTF-16 footer/body decoding,
  concurrent exact-response state, self-only editable profiles, ClassicUO-style
  request and save-on-close behavior, ServUO's 511 UTF-16-unit update limit,
  versioned brain/native/WASM actions, and Paperdoll/browser profile windows
- `0xD1 LogoutRequest` / `0xD1 LogoutAck`: capability-negotiated,
  server-authorized termination with stale/unsolicited-reply gating, explicit
  allow/deny state, ClassicUO's immediate-disconnect fallback when the 0xA9
  flag is absent, versioned brain/native/WASM contracts, Options logout
  confirmation, and clean login-scene recovery after an accepted logout or
  lost game connection
- `0xF6 BoatMoving`: atomic High Seas boat/passenger/item relocation, bounded
  monotonic movement events, exact ClassicUO speed intervals, and rigid-group
  browser interpolation for the hull, onboard entities, and following camera
- Speech, localized messages, OPL/tooltips, prompts, targeting
- Movement confirmation/denial, pathfinding, doors, facet changes
- Combat state, damage/effects, animations, death/corpse links
- Skills, buffs, spellbooks, books, maps, waypoints
- Vendors, secure trade, popup menus, gumps, custom-house viewing
- Light, weather, season, sound, and music

## Missing gameplay handlers

Priority reflects player impact and how much state/UI already exists to support
the feature. Each row is still open unless a later change moves it above.

| Priority | Packet(s) | ClassicUO behavior | Required anima vertical slice |
|---|---:|---|---|
| P1 | `0x15` | Follow response | Follow/autowalk state |
| P1 | `0x16` | Older health-bar status | Version-compatible poison/yellow health parsing |
| P1 | `0x23` | Drag animation | World-to-world item movement animation |
| P1 | `0x5B` | Server time | World clock state and optional HUD exposure |
| P1 | `0x71` | Bulletin board | Board/message model and compose/reply UI |
| P1 | `0x97` | Forced player movement | Server-directed step through movement prediction |
| P1 | `0x98` | Name update | Mobile rename without a status refresh |
| P1 | `0xB2` | Legacy chat message | Chat-channel state/UI where supported by the shard |
| P1 | `0xC4` | Semivisible | Explicit translucency state |
| P1 | `0xC8` | Client view range | Dynamic scene/range limits |
| P1 | `0xD2`, `0xD3` | Legacy mobile/item updates | Older-client compatibility parsers |
| P1 | `0xDE` | Mobile status update | SA-era status flags not covered by `0x17` |

## Protocol/session items to audit separately

These ClassicUO registrations are login negotiation, shard extensions,
acknowledgements, or currently intentional no-ops rather than standalone game
features: `0x32`, `0x53`, `0x55`, `0x73`, `0x82`, `0x85`, `0x86`, `0x8C`,
`0xA8`, `0xA9`, `0xB7`, `0xB9`, `0xBB`, `0xBD`, `0xBE`, `0xC6`, `0xCA`,
`0xCB`, `0xD0`, `0xD7`, `0xDB`, `0xE3`, `0xF0`, `0xF1`, `0xF7`, `0xFD`.
Several are already consumed by `LoginMachine`; the rest need a per-shard value
assessment before being promoted into the gameplay table.

## Beyond packet parity

Packet registration parity alone does not equal ClassicUO feature parity. Major
systems that require their own audits include custom-house **editing** (viewing
is implemented), complete boat controls, bulletin boards, remaining legacy
prompts, assistant/plugin APIs, settings persistence,
accessibility, and renderer polish. This file should stay evidence-based: add a
ClassicUO source location and an end-to-end acceptance test when closing a row.

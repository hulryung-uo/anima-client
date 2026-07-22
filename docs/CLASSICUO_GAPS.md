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
- Current game decoder: ~90 packet IDs. Two waves closed the ClassicUO gap:
  (1) the 13 previously-missing P1 gameplay handlers below (0x15/0x16/0x23/
  0x5B/0x71/0x97/0x98/0xB2/0xC4/0xC8/0xD2/0xD3/0xDE); (2) 0xF7 PacketList plus
  the *sub-command / field* gaps inside already-dispatched multiplexed handlers
  (mobile war-mode/paralysis flags, 0xBF 0x22/0x19/0x26, and the full 0x11
  CharacterStatus version tail). `0x21`/`0x22` movement stay in `Walker`. Every
  change was validated live (a `play` session logged into ServUO, entered the
  world, and processed the real packet stream with no panic) and passed
  multi-agent adversarial audits against the ClassicUO/ServUO/Python sources.

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
- `0x98 UpdateName`: existing-only mobile rename (no phantom, like ClassicUO)
- `0x16`/`0x17 NewHealthbarUpdate`: unified poison (bool + level) and
  yellow/blessed bar, existing-only, only the field the packet carried
- `0xDE UpdateMobileStatus` / `0xC4 Semivisible`: parse-only, matching
  ClassicUO's own no-op handlers (recognized, never a phantom)
- `0x5B SetTime`: `World::game_time` clock (ported from anima's handler)
- `0xC8 ClientViewRange`: `World::client_view_range`
- `0x15 FollowR`: `World::follow_target`
- `0x97 MovePlayer`: server-forced step recorded as `World::forced_walk`
  (+seq), mirroring the 0x38 pathfinding request pattern
- `0xD2 UpdateCharacter` / `0xD3 UpdateObject`: legacy full mobile updates
  (self-guarded; 0xD3 also parses the worn-item list past its 6-byte padding)
- `0x23 DragAnimation`: item-drag visual as a `World::recent_drag_anims`
  event log, with ClassicUO's gold/gem graphic remap and live endpoint
  position substitution
- `0x71 BulletinBoardData`: board + summaries + full-message state model
- `0xB2 ChatMessage`: chat channel/status state + capped message log
- `0xF7 PacketList`: batch container — dispatches each 0xF3 world-item
  sub-packet (defensive parity; framing would otherwise drop them)
- Mobile status-flags byte (0x20/0x77/0x78/0xD2/0xD3): now also decodes
  `war_mode` (0x40) and `paralyzed` (0x01), not just Hidden — the only wire
  source for another mobile's war-mode/paralysis
- `0xBF/0x22 New Damage` (AOS twin of 0x0B) → the damage log; `0xBF/0x19`
  ExtendedStats (v0 bonded-pet death → `Mobile::is_dead`; v2 stat-training
  locks → `PlayerStats::{str,dex,int}_lock`); `0xBF/0x26 SpeedMode` →
  `PlayerStats::speed_mode` (server-forced walk)
- `0x11 CharacterStatus` version-gated tail: weight_max (+ non-ML strength
  fallback), race, stats_cap, followers, four resistances, luck, damage
  range, tithing — the full character sheet (surfaced through `PlayerView`)

## Missing gameplay handlers

All P1 gameplay handlers previously listed here (`0x15`, `0x16`, `0x23`,
`0x5B`, `0x71`, `0x97`, `0x98`, `0xB2`, `0xC4`, `0xC8`, `0xD2`, `0xD3`,
`0xDE`) are now implemented (see the vertical slices above) and validated
live + audited. No P1 gameplay packet handlers remain open.

## Protocol/session items to audit separately

These ClassicUO registrations were assessed (ClassicUO-vs-anima handler diff)
and left unimplemented on purpose — they are login negotiation (handled by
`LoginMachine`/session: `0x53`, `0x55`, `0xF1`, `0xFD`, `0xDB`, `0x32`),
Enhanced-Client-only (`0xE3`), or ClassicUO's own empty no-ops whose return
value anima's caller already ignores (`0xC6`, `0xCA`, `0xCB`, `0xD0`, `0xD7`,
`0x73` ping, `0xB7` help, `0xBB` messenger). Recognizing them as parsed
no-ops has no functional effect (the framing table already consumes each
frame). `0xF7` was the one carrying real payload and is now handled (above).

Lower-priority open items (evidence-based, deferred): `0xF0 KrriosClientSpecial`
(assist-tool party radar), `0xBF/0x21 ClearWeaponAbility` (needs the outgoing
UseAbility path to also track the armed slot), `0xBF/0x10 DisplayEquipInfo`
(pre-OPL, superseded by the implemented 0xD6/0xDC OPL), `0xBF/0x18` map-file
patches (asset concern), and the `0x11 flag>=6` combat-bonus tail (shard-
dependent; framing harmlessly drops it).

## Outgoing (client → server) parity

A diff of ClassicUO `OutgoingPackets.cs` vs anima `net/outgoing.rs` (~42
builders): the client already sends everything the agent/renderer drives
(move, attack, say, target, gump/context/prompt responses, buy/sell, trade,
party, skills, OPL request, logout, house design, …). Two always-active
session packets were missing and are now sent: `0x73 Ping` (keepalive) and
`0xC8 ClientViewRange` (world-entry handshake). The rest of ClassicUO's
outgoing set is login-phase (handled by `LoginMachine`), EC-only (`0xEC`), or
**feature-completing builders that only matter once a UI/agent action drives
them** — `0x98 NameRequest`, `0x75 RenameRequest`, the `0x71` bulletin-board
post/read requests, the `0xB3/0xB5` chat commands, `0x93/0xD4` book-header
edit, `0x2C` death-screen response, `0xF0` party/guild query. These pair with
already-decoded incoming state; add each builder alongside the input/agent
action that triggers it.

## Beyond packet parity

Packet registration parity alone does not equal ClassicUO feature parity. Major
systems that require their own audits include custom-house **editing** (viewing
is implemented), complete boat controls, bulletin-board / chat compose+reply UI
(the packet *state models* are now decoded; the renderer/brain surfaces for
authoring are not built), remaining legacy prompts, assistant/plugin APIs,
settings persistence, accessibility, and renderer polish. This file should stay
evidence-based: add a ClassicUO source location and an end-to-end acceptance test
when closing a row.

Known minor divergence (low priority, tracked by the adversarial audit): the
mobile-incoming family (`0x78`, and the new `0xD3`) does not yet clear a
mobile's *stale* worn items before applying the incoming equipment list, so an
unequipped item can linger in `World::items` until overwritten. ClassicUO
removes non-backpack worn items first. `0x78` has always had this gap; fold the
fix into both when the equipment list is next touched.

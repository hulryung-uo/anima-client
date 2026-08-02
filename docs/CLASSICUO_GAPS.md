# ClassicUO compatibility gap inventory

Working inventory for bringing `anima-client` toward ClassicUO feature coverage.
A feature counts as complete only when its packet/state, native driver contract,
scene/agent exposure, and user-facing behavior (where applicable) all exist.

Closed since the audit (2026-08-02): the five Tier 1 rows marked **CLOSED**
below (speech modes, stat locks, auto-walk speed tiers, shard list + relay
address, login/character rejection reasons) and the Tier 4 terrain-perception
row — the one the audit called out as cutting against the project's own thesis.

## Audit baseline

- Audited: **2026-07-30** (supersedes the 2026-07-22 pass; 60 commits landed between
  them, including the whole custom-house **editing** vertical the old file listed as
  missing)
- ClassicUO source: local upstream checkout, `src/ClassicUO.Client` + `src/ClassicUO.Assets`
- anima source: whole workspace — `crates/anima-{core,assets,net,agent,wasm,desktop}`
  and `web/{main.js,dialogs.js,index.html}`
- Method: 14 subsystem sweeps (packets in/out, gumps, managers, rendering, assets,
  input/macros, audio, config, world/map/houses, text/journal/chat,
  combat/targeting/spells, login/character, network robustness, plus a
  "what did we miss" critic), each followed by an adversarial pass whose job was to
  *refute* the sweep.

### Reliability note — read this before trusting a row

The first adversarial pass **rubber-stamped**: it confirmed essentially every claim.
An independent re-test of 31 claims then found **12 outright wrong and 11
overstated** (~39%). The false positives clustered in one place: surveyors read the
Rust crates and never properly read `web/main.js`, which is where most player-facing
UI actually lives. All remaining claims were then re-run through skeptics primed with
that failure rate; that pass refuted 0 of 112 and downgraded 10, which is the reason
the list below is worth acting on.

**Wrongly reported as missing — these are implemented, do not re-do them:**
user-definable macros + hotkeys (`web/index.html:1092-1120`, `web/main.js:9041-9210`,
localStorage `anima.macros`); health bars over other mobiles and party
(`web/main.js:5028-5140`); overhead name plates (`web/main.js:5087-5136`); world-map
user markers (`web/main.js:4261-4264`, `:4475-4487`); spell definitions for all eight
schools plus casting from the spellbook (`web/main.js:5422-5574`); the weapon
special-ability bar (`web/main.js:5306-5416`); lightning/moving/fixed graphic effects
(`crates/anima-core/src/net/game.rs:899-925`); animated statics
(`crates/anima-assets/src/animdata.rs` + `crates/anima-net/src/scene.rs:839-857`);
distance-based sound falloff (`web/main.js:333-364`); the music playlist config
(`crates/anima-net/src/play_server.rs:2387-2441`); per-sound volume
(`web/main.js:231-232`, `:376-381`). "Looping ambient sound" is not a ClassicUO
feature at all (zero hits across its tree).

Rows marked **[verified]** were re-checked by hand against the code, not just by an
agent.

---

## Tier 0 — defects in shipped behavior

These are not missing features; they are things that are wrong today.

### T0.1 Facets 1–5 load no terrain at all — **CLOSED** (`d4cd6d0`)

`crates/anima-assets/src/uop.rs:286` hardcodes the UOP virtual path to
`build/map0legacymul/{index:08}.dat` regardless of which facet the reader was opened
for. `MapData::open_facet` (`crates/anima-assets/src/map.rs:198`) correctly opens
`map{facet}LegacyMUL.uop`, so there is no error — and `load_land_block`
(`map.rs:232-243`) silently fills 64 cells with `(0, 0)` on a miss.

Measured against the real data files:

```
facet 0 (Felucca)  : terrain present 4/4 samples   g=0x0006 z=10 …
facet 1 (Trammel)  : terrain present 0/4 samples   g=0x0000 z=0
facet 2..5         : 0/4
```

The moment the server moves the player to Trammel (a normal ServUO destination),
Ilshenar, Malas, Tokuno or Ter Mur, the ground vanishes and every land Z reads 0, so
`CalculateNewZ`, walkability and step prediction all desync from the server.
ClassicUO uses the per-index pattern (`src/ClassicUO.Assets/MapLoader.cs:149`).
Closed by threading the facet into `by_map_chunk` and making `open_facet` refuse
a container whose chunk 0 is unreachable — a silent void is indistinguishable from
real flat terrain. All six facets verified against the real data files.

### T0.2 An unrecognized opcode kills the session — **CLOSED** (`d4cd6d0`)

`crates/anima-core/src/net/lengths.rs` is a sparse 198-entry table; anything absent
returns `PacketLength::Unknown` → `FramingError::UnknownPacket` →
`DriverError::Framing` at `crates/anima-net/src/lib.rs:1095`, which ends the session.
ClassicUO's `Network/PacketsTable.cs` is a complete 256-slot array (`-1` = read the
u16 length), so it can frame and ignore anything.

Diffing the two tables, **44 opcodes ClassicUO can frame and we cannot**:

```
0x3D 0x4A 0x4B 0x4C 0x4D 0x50 0x52 0x59 0x5A 0x5C 0x5E 0x5F 0x60 0x61 0x62 0x63
0x64 0x67 0x68 0x69 0x6A 0x6B 0x79 0x7A 0x7E 0x7F 0x84 0x87 0x8A 0x8D 0x8E 0x8F
0x92 0x94 0x96 0x9C 0x9D 0xAC 0xB3 0xB4 0xC5 0xCD 0xCE 0xD5
```

Closed: the table now covers all 256 ids. Two values deliberately depart from
ClassicUO — 0xD5 stays variable (its fixed 9 comes from the CV_7010400 / 7.0.104.0
branch, above the 7.0.102.3 we advertise) and 0xCF keeps ClassicUO's 78 (ServUO's
registration for it is client→server, the opposite of the direction this table
frames). An unknown id stays fatal by design, since a packet whose length you do
not know cannot be skipped.

### T0.3 The packaged desktop app forgets every preference — **CLOSED** (`d4cd6d0`)

`crates/anima-desktop/src/main.rs:145` binds the play server with `http_port: 0`
(OS-assigned) and points the webview at `http://127.0.0.1:<random>/`. `localStorage`
is keyed by origin *including the port*, so every launch of the shipped app gets a
brand-new, empty store: settings, world-map markers, POI filters, macros, and all
remembered panel positions are lost each time. The `play` dev binary defaults to
8090, which is why this never showed up in development.

Closed by serving from the first free port in `8190..=8199` and remembering the
port actually bound, which keeps the original reason for an ephemeral port (never
collide with the `play` bin, or with a second copy) while giving a stable origin.

### T0.4 Dyed items render in their default colour — **CLOSED**

A ground item's scene entry (`crates/anima-net/src/scene.rs:2589`) carries
`x,y,z,g,serial,pz` and **no hue** — `Item::hue` is only used for corpses
(`scene.rs:2625`). The browser then requests art with no hue query in all three
places items appear: world (`web/main.js:2650`), container/backpack grids
(`:6922`) and the vendor window (`:7755`, `:7770`). Only worn equipment on the
paperdoll is hued (`:6830`).

Closed, and PartialHue-correct: `tiledata`'s `TileFlag.PartialHue` (0x40000) is
folded into bit `0x8000` of the shipped hue — ClassicUO's own encoding, which
`anima_assets::apply_hue` already read — so only the gray pixels are recoloured on
the 533 graphics that ask for it. Applied to every item surface including worn
equipment, which the first pass had deferred (a full hue on a partial-hue item
looks worse than no hue at all, and the doll and the backpack would have shown the
same item in two different colours).

### T0.5 Animated dynamic items never animate — **CLOSED**

`anim_suffix` (`crates/anima-net/src/scene.rs:839`) is applied to map statics
(`:3090`) and multi components (`:2370`) but not to the dynamic-item loop
(`:2586`), so server-spawned animated items — spell fields, campfires, some
braziers — render as a single frozen frame.

Closed: the item loop now emits the same animdata frame list statics get, and the
renderer registers those sprites into the existing animation tick. The per-pixel
hit area follows the drawn frame rather than frame 0, so a click still lands where
the flame actually is.

### T0.6 Item tooltips (OPL) never refresh — **CLOSED**

`opl_info` (`crates/anima-core/src/net/game.rs:1769`) records the 0xDC revision and
does nothing with it; `World::opl_revision` is written in four places and compared
nowhere. The only 0xD6 request path is `Action::OplRequest`, driven solely by a
browser hover, and `web/main.js:726/769/6821` gate on `!oplReq.has(serial)` so a
serial is fetched at most once. An item whose properties change keeps its stale
tooltip forever. The identical staleness pattern *is* handled correctly for
custom-house designs (`game.rs:3068-3085`), which is the model this followed.

Closed: a 0xDC naming a revision we do not hold queues a re-request, drained by
the driver (core still never touches a socket), batched by `OPL_REQUEST_BATCH`
next to the builder so both drivers and any future caller share the cap.

### T0.7 62 multi ids are invisible and non-solid — **CLOSED**

`crates/anima-assets/src/multis.rs` reads only `multi.idx`/`multi.mul`;
`MultiCollection.uop` is unread. 62 ids exist only in the UOP — `0x50-0x53`,
`0x147C-0x14A1`, `0x177C`, `0x1DF4-0x1DFB`, `0x2120-0x212A` — which covers ServUO's
pre-built castles. For those, `placement_json` (`scene.rs:2166`) bails and the
walkability fold gets nothing, so both the placement preview and collision are
silently absent.

Closed by reading `MultiCollection.uop` and merging it under `multi.mul`. The
audit missed the more important half, which the review surfaced: a component's
**visible** set is not its **solid** set. ServUO maps UOP flag `0x101` to
`TileFlag.Generic` and keeps it in the collision grid while ClassicUO does not
draw it — a nodraw-but-solid component, and both references are right. Walkability
now folds on a separate `server_keeps`, and because our ServUO reads the UOP
(`Config/DataPath.cfg` → the same directory this reader opens) while our merge
takes the MUL for shared ids, the UOP's keep answer is overlaid onto MUL
components. That corrects 907 components across 70 multis — 281 of them on
ordinary shared ids — which the client had called open where the server blocks.

---

## Tier 1 — capabilities the player/brain cannot express

The receive side is generally complete; there is no way to *act*.

| Gap | Where it stands | Effort |
|---|---|---|
| ~~**Speech modes** (whisper / yell / emote / guild / alliance)~~ — **CLOSED** | `Action::Say` now carries a `SpeechMode` (`agent.rs`), the driver writes its `MessageType` byte, `play_server` gained `whisper:`/`yell:`/`emote:`/`guild:`/`alliance:` commands, and the chat bar routes `/w`, `/y`, `/e`, `/g`, `/a` prefixes. JSON `Say.mode` is optional, so pre-mode brains are unaffected | S |
| ~~**Stat locks** (0xBF/0x1A)~~ — **CLOSED** | `build_stat_lock` + `Action::StatLock` + `statlock:<stat>:<lock>`, with the same optimistic local update the skill-lock twin does | S |
| **Armed weapon ability state** (0xBF/0x21 clear, 0xBF/0x25 toggle) | send works, but the server's clear/toggle feedback is unhandled, so the bar's armed highlight goes stale after every use | S |
| Pre-AOS stun / disarm (0xBF/0x09, 0x0A) | absent | S |
| Targeted use (0xBF/0x2C) — bandage self/target in one packet | absent | S |
| ~~**Auto-walk always runs at unmounted-walk speed**~~ — **CLOSED** | new `movement::walk_pacing(world, want_run)` returns `(run, ms)` from live state — ClassicUO `PlayerMobile.Walk`'s rules, including `SpeedMode >= CantRun`, spent stamina (ghosts exempt), and `FastUnmount` taking the mounted tier without a mount. Both auto-walkers (`Route::step_delay`, `play_server`'s loop) consult it per tick and now run | S |
| ~~**Shard list**~~ — **CLOSED** | `parse_server_list` (0xA8) + `LoginMachine::servers()`; `cfg.server_index` names the shard's *own* index and an unlisted one fails with the shard names instead of hanging. 0x8C now yields a `GameServerAddress`, which the native driver dials first (5s timeout) before falling back to the login endpoint — ClassicUO's `IgnoreRelayIp`/`ip == 0` case, available as `Endpoint::ignore_relay_ip`. **Watch the byte order:** 0xA8's address is reversed, 0x8C's is not | M |
| ~~**Login/character rejection reasons**~~ — **CLOSED** | 0x82 gained `account_denied_text`, 0x53 became `LoginError::CharacterLoginRejected` with `character_login_rejected_text`, and 0xFD's queue window is stored and quoted by 0x53 codes 13/14. `LoginError` now implements `Display`, so the browser login page and CLI show the server's stated reason instead of a Debug dump | S |
| Book authoring (title/author + page text) | 0x93/0xD4 header edit and page write builders exist as round-trip stubs; no UI drives them | M |
| Map pin editing (0x56) | read-only | S |
| Boat helm control (0xBF/0x33) | absent | M |
| Bulletin board post/read/reply | state model decoded (0x71); no authoring surface | M |
| Chat channels (0xB2/0xB3/0xB5) | **core is complete** — create/destroy/join/leave/lines all decoded — with no Action, scene field or UI above it | M |
| Guild / quest menus (0xD7 sub 0x28 / 0x32), help+GM page (0x9B), rename (0x75), quest-arrow click (0xBF/0x07) | absent | S each |
| Party: loot flag, private message, leader kick | partial | S |
| Request another entity's status (0x34 non-self) → party mana/stam bars | absent | S |

---

## Tier 2 — UI for state we already decode

| Gap | Note | Effort |
|---|---|---|
| **Extended status sheet** | resistances, damage range, luck, followers, tithing, weight and stats-cap are all parsed into `World` but `scene/mod.rs`'s `build_scene` never emits them, so `refreshStatus` can only show HP/mana/stam/str/dex/int/gold | M |
| **0x11 `type >= 6` combat tail** | max resists, HCI/DCI/SSI/DI/LRC/SDI/FCR/FC/LMC are explicitly not parsed (`game.rs:2757`); no field of that family exists anywhere | M |
| **Buff names** | 0xDF's title/description clilocs and their args are never read; names come from a hardcoded 35-entry English table against 189 icon graphics, so most buffs show as `#1234` and the buff/debuff tint is a regex over that name | M |
| Journal | works, but one flat colour, no message-type filter, no tabs, no timestamps, not its own resizable window | M |
| Grid loot | corpses already open as a grid; the one-click loot workflow is what's missing | M |
| Info bar | a fixed HUD readout exists; ClassicUO's user-configurable field set does not | M |
| Counter bar, ignore list, combat-book gump, racial-abilities book, network-stats and inspector windows | absent | S–M |
| Container gumps ignore real container art and each item's stored (x,y) | contents render as a uniform grid rather than the authentic bag layout | M |
| Window management | no resize, no anchoring/docking, no saved per-window layout | M |

---

## Tier 3 — rendering fidelity

Shadows; corpse equipment layers (corpses render as a bare body); death animation
(mobiles do not fall); `light.mul` light shapes/colours (a radius-only light system
exists — `build_scene`'s `lights` list in `scene/mod.rs`); directional lighting on stretched land; seasonal
land/static graphic remap (the season *system* exists, the remap does not);
`TileFlag.Translucent` statics drawn opaque; static hue from `statics.mul` discarded
at decode; mount rider vertical offset; seated-character deformation; roof/ceiling
fading; 0x23 DragEffect decoded but never drawn; GameEffect blend modes and
projectile rotation; StaticFilters (tree→stumps, hide vegetation).

---

## Tier 4 — the AI contract

This is the one that cuts against the project's own thesis.

- ~~**Brains have no terrain perception.**~~ — **CLOSED.** `Observation.terrain` is
  an optional `TerrainView`: a square walkability window (`walkable`, standing `z`,
  and the serial of a closed door that would have to be opened first) filled by
  `agent::survey_terrain` over the existing `path::Terrain` trait. The core still
  reads no map files — `World::observe` leaves it `None` and a driver holding
  `MapData` fills it (`Session::observation_with_terrain`, surveyed through the same
  `scene::MapTerrain` the auto-walk route uses, so brain and driver cannot disagree
  about what is walkable). Wired through the JSON contract (schema **v17**; per-tile
  data packed as parallel `walk` string + `z` array + sparse `doors`, since this is
  the largest field in an observation and is re-sent every poll), the NDJSON bridge
  (`{"cmd":"observe","terrain_radius":N}`), and both agent runners. `WanderBrain`
  now turns before hitting a wall it can see and opens a door instead of walking
  into it, with tests for both — and, with no window supplied, behaves exactly as
  it did before.
  **Known limit, by design:** every tile is judged as a step from the player's
  current Z, so the far edge of the window is approximate on sharply sloped
  ground; exact multi-step reachability still means `find_path`.
- Cliloc-localized messages reach brains as an unresolved id plus raw args, so a brain
  cannot read most system messages.
- 0xCC ClilocMessageAffix drops the affix-flags byte and concatenates the affix into
  the cliloc *args* rather than the resolved text, so templates without a placeholder
  discard it entirely (`game.rs:3310-3331`).
- Server-gump layout grammar: `tilepic`/`tilepichue`, `gumppictiled`, `checkertrans`,
  `tooltip`, `itemproperty`, `buttontileart`, `picinpic`, `textentrylimited`,
  `noclose`/`nodispose`/`nomove` are silently dropped, and radio buttons are never
  grouped — so complex shard gumps are mis-rendered for both the player and the brain.

---

## Tier 5 — assets (`crates/anima-assets`)

Beyond T0.1 and T0.7 above: `verdata.mul` patching; `mapdif`/`stadif` map-diff files
and 0xBF/0x18; `speech.mul` keyword encoding (so NPC keyword commands can never be
sent); `Multimap.rle`/`facet0N.mul` (the map window shows only its gump frame and
pins, no terrain raster); `fonts.mul` + `unifont*.mul`; `art.def`/`TexTerr.def` alias
tables; `gump.def` alias table; `skills.mul` (skill names + HasAction);
`Prof.txt`/`Professn.enu` professions (0xF8's profession byte is hardcoded 0);
`tileart.uop`; `Anim1.def`/`Anim2.def` group-replacement tables **and** the missing
`% AnimationCount` clamp for animals/monsters; `light.mul`; cliloc language selection
and BWT-compressed clilocs; `Client.Version`-driven format selection; MUL fallback
when a UOP is absent; and `hues.mul` ramp-index selection (we pick the ramp by pixel
brightness where ClassicUO uses the red channel).

---

## Deliberately not doing

Login/game-stream encryption (this client targets shards that accept unencrypted
connections); ClassicUO's assist/plugin API; Enhanced-Client-only packets (0xE3,
0xEC); ClassicUO's internal plumbing with no player- or agent-visible effect.

## Keeping this file honest

Add a ClassicUO source location and an end-to-end acceptance test when closing a row.
When a row is closed, say so here rather than deleting it — the "wrongly reported as
missing" list above exists because that history was not written down.

Known minor divergence carried over from the previous audit: the mobile-incoming
family (`0x78`, `0xD3`) does not clear a mobile's *stale* worn items before applying
the incoming equipment list, so an unequipped item can linger in `World::items` until
overwritten. ClassicUO removes non-backpack worn items first. Fold the fix into both
when the equipment list is next touched.

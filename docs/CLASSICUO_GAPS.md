# ClassicUO compatibility gap inventory

Working inventory for bringing `anima-client` toward ClassicUO feature coverage.
A feature counts as complete only when its packet/state, native driver contract,
scene/agent exposure, and user-facing behavior (where applicable) all exist.

Closed since the audit (2026-08-02/03): the five Tier 1 rows marked **CLOSED**
below (speech modes, stat locks, auto-walk speed tiers, shard list + relay
address, login/character rejection reasons) and three of Tier 4's four —
terrain perception (the row the audit called out as cutting against the
project's own thesis), cliloc-resolved journal text, the 0xCC affix bug, and
the gump layout grammar — plus both Tier 2 rows (status sheet, buff names).
Tier 4 is now closed out entirely.

Closed 2026-08-06 (the combat batch): three more Tier 1 rows — armed weapon
ability state, pre-AOS stun/disarm, targeted bandage use — and the stale
worn-item divergence recorded at the bottom of this file. Contract schema
v18 → v19. That batch also found a real disagreement between the two reference
implementations (ClassicUO swaps the stun and disarm subcommands relative to
ServUO and Razor); see the Tier 1 row for which side we took and why.

Closed 2026-08-07 (the party batch): both remaining party rows — loot flag /
private message / leader kick, and non-self status request. Contract schema
v19 → v20. Live-verified end to end by running **two `play` instances** against
the shard (a GM leader on 8788, an ordinary account on 8789) and actually
forming a party, which is the only way to test any of it. That pass also found
the second row's stated premise wrong in two ways — see the row.

Closed 2026-08-07 (the menus batch): the last "S each" Tier 1 row — guild/quest
menus, help+GM page, rename, quest-arrow click. Contract schema v20 → v21.
`build_house_design` was renamed `build_encoded_command`: the guild and quest
gump requests want byte-identical framing, so the house designer is the
biggest user of that shape, not the only one.

**Know what the local shard can and cannot prove.** `Config/Expansion.cfg` on
the ServUO at `127.0.0.1:2594` says `CurrentExpansion=T2A`, so `Core.AOS` is
**false** there. That single fact splits this batch in half, and it is worth
checking before writing off a live test as a failed feature:

- It makes the shard the *ideal* rig for the pre-AOS rows. `Fists.cs`'s
  disarm/stun handlers open with `if (Core.AOS) return;`, so on T2A they
  actually run — which is how the ClassicUO subcommand divergence got settled
  empirically rather than by reading three sources and picking a majority.
- It makes 0xBF/0x21 and 0xBF/0x25 **unobservable** there. Both are gated:
  `WeaponAbility.ClearCurrentAbility` sends its packet only
  `if (Core.AOS && m.NetState != null)`, and 0x25's senders (`SpecialMove`,
  `SamuraiSpell`) are AOS+/SE content that cannot be invoked at all. A live
  session will therefore show an arm that never clears no matter how long you
  swing — **that is the shard, not the client**, and it cost a confused
  detour to work out. Those two rows rest on unit tests plus the ServUO
  source; confirming them live needs a shard at AOS or later.
- The same gate has now caught three separate features, so check it *first*
  when something reaches ServUO and nothing happens: `0x11 type >= 6` needs
  `Core.ML` (the shard sends type 3), and the guild menu needs
  `Guild.NewGuildSystem => Core.SE`. In each case the client is fine and the
  expansion is the reason — a live test that produces silence here is not
  evidence of a bug.

## Cross-check against OpenShard's `docs/findings.md` (2026-08-07)

A second opinion from a *server* that speaks the same protocol (see DESIGN.md
§7). Twelve of its claims are checkable against this tree; ten confirmed what
we already do, and the two that did not were both real defects.

**Confirmed, no change needed:** `Equipconv.def` is an override table and a
worn item with no entry draws from its own tiledata `AnimID` (`Look::equip_conv`
already falls back — this is the defect that silently dropped every piece of
plain clothing over there, and we also read the `mobtypes.txt` they note they
do not); the texmap id sits in the two bytes after the land entry's 8-byte flags
and its size is `len == 0x2000 ? 64 : 128`; land is preferred out of the `.uop`;
distance is Chebyshev; `0x8C`/`0xA8` carry the address in opposite orders; a
declared frame length is validated before anything is reserved; `0x22` means
opposite things per direction and our incoming table / outgoing builders
already keep the two apart; sloped land is drawn as a four-corner quad whose
vertices come from the neighbours' z, not as a flat diamond at its own.

**Fixed — land art read a black pixel as a hole.** Land and statics decode the
same 16-bit colour and disagree about zero: a static carries transparency as
`0x0000`, but a land tile's shape is the diamond and nothing else, so a zero
inside it is a genuinely black pixel. ClassicUO makes the split explicit —
`LoadLand` writes `Color16To32(...) | 0xFF_00_00_00` with no test at all, while
`LoadArt` guards every run with `if (val != 0)`. Ours ran both through the
transparent-on-zero path. Measured against the real `artLegacyMUL.uop`:
**361 of 851 land tiles (42.4%) are affected, 25,877 pixels**, three to six on
an ordinary grass tile — small enough to read as dark speckle in the texture
rather than as holes, which is why it survived every screenshot. Now
`argb1555_land` vs `argb1555`, pinned by a test on an all-zero tile.

**Fixed — one bad walk confirm froze movement for the whole session.** The
repair leg is a request/response: a confirm we cannot place sets
`walking_failed` (gating every further step) and sends one 0x22 Resync, and
ServUO's `Resynchronize` answers with **0x20 MobileUpdate** — not a 0x21 deny.
Only `Walker::reset` cleared the gate, and the driver called it solely on a
position jump greater than one tile. But a sequence desync is a disagreement
about *count*, not *place*: the resync answer normally carries the position we
already hold, so the jump is zero tiles, the reset never fired, and
`resend_resync` stayed latched so we never asked again. `Walker::on_player_update`
now performs the repair on any self-0x20, gated on `walking_failed` so an
unrelated 0x20 (the hiding-flag path) cannot drop live pending steps. A full
reset is correct there because `Resynchronize` also does `state.Sequence = 0`.

**Known divergence, deliberate, not fixed.** ClassicUO refuses to stretch a
land tile whose texmap entry is empty (`Land.ApplyStretch` bails and draws a
flat diamond, seams and all); we stretch the tile's own art instead. Measured:
**23 of 2724 land graphics have `TexID == 0`, and all 23 are `Wet`** — so the
whole footprint is "water at a slope", which ClassicUO deliberately never
stretches (`IsStretched = TexID == 0 && IsWet`). Ours is arguably the better
picture (no seams) and is certainly not a crash; recorded here so the next
person meets a decision rather than a discrepancy.

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
| ~~**Armed weapon ability state** (0xBF/0x21 clear, 0xBF/0x25 toggle)~~ — **CLOSED** (unit-tested; not live-verifiable on a T2A shard — see the note under "Audit baseline") | Both directions now land. `World::armed_ability` is written optimistically by the driver on 0xD7 (an arm is *never* acknowledged — the only message about it is the one revoking it) and cleared by 0xBF/0x21; 0xBF/0x25 fills `World::active_spell_icons`, which despite the packet's name carries **spell** ids (ServUO sends `moveID + 1` / `spellID + 1`), so it lights up the spellbook, not the bar. Both reach `Observation` and `scene.json`. The renderer keeps a 500ms optimistic window before believing the server, because `scene.json` is polled every 150ms and the snapshot answering a click predates it (the D11 hazard); outside that window the server simply wins, since a swing can resolve before its own echo arrives | S |
| ~~Pre-AOS stun / disarm (0xBF/0x09, 0x0A)~~ — **CLOSED, live-verified** | `build_disarm_request`/`build_stun_request` + `Action::DisarmRequest`/`StunRequest` + `disarm`/`stun` commands. **Watch the subcommand numbering:** ClassicUO has the two swapped — its `Send_StunRequest` writes 0x09 and `Send_DisarmRequest` writes 0x0A, while ServUO registers `RegisterExtended(0x09, DisarmRequest)`/`(0x0A, StunRequest)` and Razor (`Razor/Network/Packets.cs`) writes the same pairing as ServUO. **Settled empirically, not by majority vote:** the two handlers gate on *different* skills, so skills alone identify which one a subcommand reached. With hands free, ArmsLore+Wrestling 100 / Anatomy 0 → `disarm` answered "You get yourself ready to disarm your opponent" and `stun` answered "You are not skilled enough to stun your opponent"; inverting to ArmsLore 0 / Anatomy 100 inverted both replies exactly. ClassicUO's numbering would have swapped them. A byte-exact test pins it so the divergence can't be "corrected" back into a silent wrong-move bug | S |
| ~~Targeted use (0xBF/0x2C) — bandage self/target in one packet~~ — **CLOSED, live-verified** | `build_bandage_target` + `Action::BandageTarget { bandage, target }` + `bandage:<serial>[:<target>]`. `target` 0 = self (the `PartyAccept` sentinel convention), which is the case worth the shortcut. Skips the double-click → 0x6C cursor → reply round-trip, which is what makes reliable self-healing under pressure possible. Live: `bandage:<serial>` with no target healed the player, with `scene.target.active == 0` throughout — **no cursor is ever raised**. Note ServUO still emits cliloc 500948 "Who will you use the bandages on?" from inside the 0x2C handler before invoking the target itself (`Bandage.cs BandageTargetRequest`), so that line in the journal is not evidence of a cursor. ServUO's siblings 0x2D TargetedSpell / 0x2E TargetedSkillUse / 0x30 TargetByResourceMacro are the same idea and still absent | S |
| ~~**Auto-walk always runs at unmounted-walk speed**~~ — **CLOSED** | new `movement::walk_pacing(world, want_run)` returns `(run, ms)` from live state — ClassicUO `PlayerMobile.Walk`'s rules, including `SpeedMode >= CantRun`, spent stamina (ghosts exempt), and `FastUnmount` taking the mounted tier without a mount. Both auto-walkers (`Route::step_delay`, `play_server`'s loop) consult it per tick and now run | S |
| ~~**Shard list**~~ — **CLOSED** | `parse_server_list` (0xA8) + `LoginMachine::servers()`; `cfg.server_index` names the shard's *own* index and an unlisted one fails with the shard names instead of hanging. 0x8C now yields a `GameServerAddress`, which the native driver dials first (5s timeout) before falling back to the login endpoint — ClassicUO's `IgnoreRelayIp`/`ip == 0` case, available as `Endpoint::ignore_relay_ip`. **Watch the byte order:** 0xA8's address is reversed, 0x8C's is not | M |
| ~~**Login/character rejection reasons**~~ — **CLOSED** | 0x82 gained `account_denied_text`, 0x53 became `LoginError::CharacterLoginRejected` with `character_login_rejected_text`, and 0xFD's queue window is stored and quoted by 0x53 codes 13/14. `LoginError` now implements `Display`, so the browser login page and CLI show the server's stated reason instead of a Debug dump | S |
| Book authoring (title/author + page text) | 0x93/0xD4 header edit and page write builders exist as round-trip stubs; no UI drives them | M |
| Map pin editing (0x56) | read-only | S |
| Boat helm control (0xBF/0x33) | absent | M |
| Bulletin board post/read/reply | state model decoded (0x71); no authoring surface | M |
| Chat channels (0xB2/0xB3/0xB5) | **core is complete** — create/destroy/join/leave/lines all decoded — with no Action, scene field or UI above it | M |
| ~~Guild / quest menus (0xD7 sub 0x28 / 0x32), help+GM page (0x9B), rename (0x75), quest-arrow click (0xBF/0x07)~~ — **CLOSED** (4 of 5 live-verified) | `Action::GuildMenu`/`QuestMenu`/`HelpRequest`/`Rename`/`QuestArrowClick` + `guildmenu`/`questmenu`/`help`/`rename:<serial>:<name>`/`questarrow[:<0\|1>]`. All five answer with an ordinary gump or journal line, so nothing new was needed to *read* the result — these were purely missing outgoing verbs. **"Absent" was wrong about rename:** `build_rename_request` already existed, CP1252-encoded and correct, with no caller anywhere — only the Action and command were missing. Live: `help` → the "Ultima Online Help Menu" gump; `questmenu` → the "Quest Log" gump; `rename` turned "a horse" into "Bucephalus", cross-confirmed by the Tracking skill's own list showing the new name; `questarrow` discriminated by button — a **left** click left the arrow up and a **right** click cleared it, which is exactly `TrackArrow.OnClick`'s asymmetry, so the boolean byte demonstrably arrives intact. Only `guildmenu` is unverified here: ServUO gates it on `Guild.NewGuildSystem => Core.SE`, above this shard's T2A (see the shard note above) | S each |
| ~~Party: loot flag, private message, leader kick~~ — **CLOSED, live-verified** | `build_party_can_loot` (0xBF/0x06/0x06), `build_party_private_message` (0x06/0x03) and `build_party_remove` (0x06/0x02) + `Action::PartySetCanLoot`/`PartyPrivateMessage`/`PartyKick` + `partyloot:<0\|1>`/`partytell:<member>:<text>`/`partykick:<member>`. **Kick and leave are one packet**, distinguished only by whose serial it names; ServUO's `PartyCommands.OnRemove` gates it on `p.Leader == from \|\| from == target` and ignores a non-leader silently — verified both ways live (a member's kick of the leader changed nothing; the leader's kick of the member disbanded the party on both clients). Loot flag answers cliloc 1005447/1005448 and is **never sent back**, so a UI must remember what it asked for. Private messages are clamped to 128 chars because ServUO *drops* longer ones rather than truncating | S |
| ~~Request another entity's status (0x34 non-self) → party mana/stam bars~~ — **CLOSED, live-verified — and the row's premise was wrong twice** | `Action::StatusRequest { serial }` (0 = self) → 0x34 type 4; `party[].mana/manaMax/stam/stamMax` now leave `build_scene`. Two corrections worth keeping: **(a) it is a resync, not the only source.** ServUO pushes a member's mana/stam changes unprompted (`Party.OnManaChanged`/`OnStamChanged` → 0xA2/0xA3) — but only while they are in update range and visible, so a member you lost sight of freezes at the last value with nothing scheduled to fix it. That is the real gap this fills. **(b) The values are percentages, not points.** Every party-facing vitals packet goes through `AttributeNormalizer` (max written as a fixed 25, current as `cur * 25 / max`). Measured live: a member at a real 7/10 mana reached the leader as 17/25. Never compare another member's mana against a spell cost — our own vitals are un-normalized, theirs are not | S |

---

## Tier 2 — UI for state we already decode

| Gap | Note | Effort |
|---|---|---|
| ~~**Extended status sheet**~~ — **CLOSED** | all of it (armor + the four resistances, weight/max, stats-cap, followers/max, damage range, luck, tithing, and the three stat locks) now leaves `build_scene` and shows in the status panel. It had been parsed into `World` all along and simply never sent | M |
| **0x11 `type >= 6` combat tail** | max resists, HCI/DCI/SSI/DI/LRC/SDI/FCR/FC/LMC are explicitly not parsed (`game.rs:2757`); no field of that family exists anywhere | M |
| ~~**Buff names**~~ — **CLOSED** | 0xDF's title/description clilocs and their (little-endian) argument blocks are parsed into `Buff`; the renderer resolves the title for the bar and shows the description on hover, and `anima_net::localize` fills `display`/`display_desc` for brains. The 35-entry English table survives as the fallback when a shard sends no title cliloc. The debuff tint is still a regex over the name — UO carries no buff/debuff flag on 0xDF (ClassicUO hardcodes the split by icon id) | M |
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
- ~~Cliloc-localized messages reach brains as an unresolved id plus raw args~~ —
  **CLOSED.** `JournalEntry::display` carries the resolved line, filled by
  `anima_net::resolve_journal` — the driver, because the core has no Cliloc table
  (the same split as `Observation::terrain`). Wired into both agent runners and the
  JSON contract (schema **v18**); `text` still carries the raw args. Verified live
  through the NDJSON bridge: a party invite now reaches the brain as
  `display: "Who would you like to add to your party?"` instead of `cliloc 1005454`
  with empty args.
- ~~0xCC ClilocMessageAffix drops the affix-flags byte and concatenates the affix
  into the cliloc *args*~~ — **CLOSED.** The affix is kept beside the arguments
  (`JournalEntry::affix`/`affix_prepend`) and joined to the *resolved* text on the
  side flag `0x01` asks for, matching ClassicUO `DisplayClilocString`/`AffixType`.
  Folding it into the args had corrupted the argument list and lost the affix
  entirely on any template without a placeholder.
- ~~Server-gump layout grammar~~ — **CLOSED.** All of `tilepic`/`tilepichue`,
  `gumppictiled`, `checkertrans`, `tooltip`, `itemproperty`, `buttontileart`,
  `picinpic`, `textentrylimited` and `noclose`/`nodispose`/`nomove` now parse into
  typed elements (the window flags onto `GumpLayout` rather than the element list),
  reach both the JSON contract and the renderer, and draw. `{ group N }` is tracked
  so `Radio` carries its group — the renderer names each group separately, without
  which a two-question gump could only ever hold one answer. `tooltip`/`itemproperty`
  decorate the **preceding** element, as UO attaches them, rather than taking a slot.
  Verified by parser tests plus a synthetic gump injected into the live renderer:
  two radio groups each hold their own selection, `textentrylimited` caps input, and
  the art/tiled/crop/translucent elements draw.

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

~~Known minor divergence carried over from the previous audit: the mobile-incoming
family (`0x78`, `0xD3`) does not clear a mobile's *stale* worn items…~~ —
**CLOSED.** Both handlers now share one `apply_worn_items`, which treats the
list as authoritative and drops equipment it omits (symptom: a mount stayed
drawn under a rider who dismounted out of view). Two deliberate departures from
ClassicUO's `UpdateObject`, which removes every non-backpack child and recreates
the listed ones: we remove only what the list omits, because recreating an item
that never left drops its name and OPL — ClassicUO silently invalidates the
tooltip of every worn piece each time its wearer walks back into view; and we
skip the container pseudo-layers (0x1A-0x1D) rather than guard on ClassicUO's
"is this container's gump open" flag, which is renderer state the core does not
have (D3). That second point is load-bearing rather than cosmetic: a vendor's
shop containers hang off the vendor mobile on 0x1A-0x1C (see
`recorrelate_shop_buy`, which resolves buy-list prices through exactly that
link) and the bank box on 0x1D, so a layer-blind sweep would empty the buy
window. Five tests cover it, including the vendor/bank case, the OPL one, and a
truncated-frame guard (a record cut off mid-way must not read as "everything
else came off" now that omission deletes).

**Live A/B against ServUO, same experiment both ways.** Spawn a vendor, note
its worn layers, teleport out of view, `[Remove` one worn item server-side (so
no 0x1D ever reaches us), teleport back and let its 0x78 resend:
*old code* — the deleted item was still listed as worn. *new code* — gone,
while layers 0x03/0x05/0x0B/0x15 and the shop containers 0x1A/0x1B survived,
and the Buy window still opened with all four prices resolved to concrete
serials through `cont: 0x400177C3` — the very layer-0x1A container the sweep
had to leave alone.

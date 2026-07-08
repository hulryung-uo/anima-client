# TESTING.md — GM-assisted testing playbook

How to drive the running client against a live ServUO shard, screenshot what
it renders, and verify it — with a real GameMaster account so you can
teleport, spawn, and equip things on demand instead of playing normally to
reach a test state.

Companion to [`docs/DESIGN.md`](DESIGN.md) (architecture/roadmap) and
[`docs/MOVEMENT.md`](MOVEMENT.md)/[`docs/RENDERING.md`](RENDERING.md)
(subsystem detail). This doc is about *process*: how to get the client into
a known state and confirm it looks/behaves right.

---

## 1. Overview

The loop is: **drive the client into a state → screenshot it (or read its
live state) → check it against what ServUO says should be true → iterate.**
GM commands are what make "drive into a state" fast — teleport instead of
walking, spawn instead of hunting, `[set` a skill instead of training it.

Two client entry points, same renderer underneath (`web/` + `anima_net::play_server`):

- **`cargo run -p anima-desktop`** — the real desktop app (Tauri). This is
  what a human plays on; it runs `play_server` in-process on an **ephemeral**
  port (see `crates/anima-desktop/src/main.rs`), so it isn't addressable by a
  fixed URL/port and isn't the automation target.
- **The headless `play` bin, on a fixed HTTP port** — `cargo run -p anima-net
  --bin play -- <host> <port> <user> <pass> <http_port>` (or the built binary
  directly, see below). Same renderer, same protocol handling, but you choose
  the port, so scripts/curl/CDP can find it reliably every time. **This is
  the automation target for everything in this doc.**

Both are thin wrappers over the same reusable engine
(`anima_net::play_server::{bind, PlayConfig}`) — testing against the `play`
bin exercises the same code path the desktop app runs, just addressable.

Two halves to the toolkit:

- **Setting up state** — GM commands over chat (`say:[<command>`), or plain
  movement/action commands — via `curl`/`scripts/gm.sh` directly against
  `POST /input`. No browser needed for this half.
- **Verifying the result** — screenshot / read live JS state from the actual
  rendered page, via `scripts/drive.py` driving a real Chrome tab over the
  Chrome DevTools Protocol (CDP).

---

## 2. Bringing up a test session

**(a) Confirm ServUO is up on 127.0.0.1:2594** (don't start/stop/restart it —
it's the user's live environment):

```bash
nc -z 127.0.0.1 2594 && echo "ServUO is listening" || echo "ServUO is NOT listening"
```

**(b) Launch a headless `play` instance** bound to a fixed port, with
movement/pathfinding debug logging on:

```bash
cd ~/dev/uo/anima-client
cargo build -p anima-net --bin play          # only needed once / after changes
ANIMA_DEBUG=1 ./target/debug/play 127.0.0.1 2594 animagm <pass> 8788
```

- `animagm` is the dedicated GM test account (see §3 to activate it as
  GameMaster+). Until it's activated, this still logs in and plays fine as a
  normal Player — `[` commands just get silently ignored by ServUO (see the
  gap noted in §3).
- `8788` is an arbitrary fixed port so scripts/CDP can always find it — pick
  any free port; it doesn't need to match the desktop app's (ephemeral) one.
- Leave off `<web_dir>`/`<data_dir>` to use the repo's `web/` and
  `$HOME/dev/uo/uo-resource` defaults (see `crates/anima-net/src/bin/play.rs`).
- Run it in the background (`&`, or a second terminal, or
  `run_in_background` if you're an agent) — it blocks serving HTTP.
- Open `http://127.0.0.1:8788/` in a browser (regular interactive use), *or*
  point Chrome's CDP-driven tab at it for automated screenshots (§4).

**(c) Drive it.** Two ways, often combined:

- **curl / `scripts/gm.sh`** for GM commands and movement (no browser
  needed — see §3, §4):

  ```bash
  scripts/gm.sh 8788 'go 1416 1500'          # -> say:[go 1416 1500
  curl -sS --data-binary 'walk:0:1' http://127.0.0.1:8788/input   # walk north, running
  ```

- **`scripts/drive.py`** against an actual Chrome tab showing the page, for
  anything that needs to be *seen* (screenshots) or read from live JS state
  (§5).

---

## 3. GM account activation

Two ways to make `animagm` GameMaster+ on the account. **(A) is recommended**
— it's the method already used successfully in this environment, requires no
server restart, and can't corrupt `accounts.xml`.

### (A) Recommended: `[admin` while logged in as Owner

1. Log in to the shard as `hulryung` (the only Owner-level account) — via the
   desktop app, `play`, or a real UO client.
2. `[admin` → **ACCOUNT LIST** → select `animagm` (if `AutoAccountCreation` in
   `Config/Accounts.cfg` already created it from a prior login attempt) or
   type/select to create it → set **Access Level** to **GameMaster**.
3. No restart needed — ServUO's account-list gump edits the live in-memory
   `Account` object directly.

### (B) Offline: edit `accounts.xml` while ServUO is stopped

Only do this while **ServUO is not running**. Editing `Saves/Accounts/accounts.xml`
while the server is live is unreliable: ServUO's periodic world/account save
(`Scripts/Misc/AutoSave.cs`, default every 5 minutes,
`Config.Get("AutoSave.Frequency", ...)`) rewrites the whole file from its
in-memory account list on its own schedule, silently clobbering any manual
edit that happened in between.

1. Stop ServUO.
2. Add an account block to `Saves/Accounts/accounts.xml` (inside the
   `<accounts>` root, alongside the existing entries):

   ```xml
   <account>
     <username>animagm</username>
     <password>animagm</password>
     <accessLevel>GameMaster</accessLevel>
   </account>
   ```

   A plaintext `<password>` element is accepted on load: `Account`'s
   XML-loading constructor reads it (`Utility.GetText(node["password"], ...)`)
   and, since this shard's `Config/Accounts.cfg` sets
   `ProtectPasswords=NewCrypt`, immediately calls `SetPassword(...)` on it —
   which hashes it into the SHA1 `newCryptPassword` form ServUO actually
   checks against at login (`Account.CheckPassword`, which also
   re-upgrades the stored hash on a successful login if the protection
   scheme ever changes). Cite: `Scripts/Accounting/Account.cs` — the
   constructor's `PasswordProtection.NewCrypt` case
   (`plainPassword != null → SetPassword(plainPassword)`, ~line 256) and
   `SetPassword`'s `NewCrypt` branch (`HashSHA1(Username + plainPassword)`,
   ~line 700); `AccountHandler.ProtectPasswords` is read from
   `Config/Accounts.cfg:25` (`ProtectPasswords=NewCrypt`).
3. Start ServUO.

Real accounts.xml entries in this environment look like this (password
redacted — this is the shape to match, not to copy):

```xml
<account>
  <username>hulryung</username>
  <newCryptPassword>16-B4-E0-0A-...-FC</newCryptPassword>
  <accessLevel>Owner</accessLevel>
  <created>2026-06-25T13:19:55.571126Z</created>
</account>
```

Either way, once `animagm` is GameMaster+, **`say:[<command>`** through
`/input` (see §1, §4) executes real GM commands — no other plumbing needed;
`play_server`'s `/input` → `Action::Say` → normal chat packet is exactly what
a real client sends, and ServUO's `Server/Commands.cs` (`m_Prefix = "["`)
parses any chat message starting with `[` as a command if the sender's
`AccessLevel` clears the command's registered level.

**Gap to watch for:** if `animagm` is still Player-level, ServUO's command
parser (`CommandSystem`) just says "you don't have access to that command"
back in the chat channel — check the `play` process's log
(`ANIMA_DEBUG=1` prints server text; also visible via any in-page chat log)
if a `[go`/`[add` appears to do nothing.

---

## 4. GM command cheat-sheet

All verified against this repo's ServUO source
(`/Users/dkkang/dev/uo/servuo/Scripts`, `/Server`) — command name, required
`AccessLevel`, and whether it needs a target click all cited below. Sent as
`scripts/gm.sh <port> '<command without [>'` (→ `say:[<command>` over
`/input`), unless noted.

**On "needs a target": a click, or a self-modifier.** ServUO's generic
command framework (`Scripts/Commands/Generic/`) dispatches `Add`/`Set`/`Get`/
`Kill`/`Teleport`/etc. through `BaseCommand`, and several of those, invoked
bare, pop a **target cursor** — normally answered by clicking somewhere in a
real client. Our headless harness can't click, but has two ways round it:

1. **The `Self` modifier**, where the command supports it (`AddCommand`,
   `SetCommand`/`GetCommand`, `KillCommand` all declare `CommandSupport.Self`
   — see `Scripts/Commands/Generic/Commands/Commands.cs`): prefix the command
   with `Self`, e.g. `[Self Add orc`, `[Self Set Str 100`. This runs the
   command directly against the caller (`SelfCommandImplementor.Compile`
   sets `obj = from`, no target prompt at all) — the cleanest, fully
   scriptable path, **use this whenever the table below offers it**.
2. **Answer the target cursor yourself**, for commands with no `Self`
   variant (`Teleport`/`Tele`, `Dupe`, `Bank`): our client already implements
   target-cursor UI for spellcasting, and it's the same underlying protocol
   packet (`0x6C TargetCursor` / `0x6C` reply) for a GM command's cursor. Send
   the GM command, then answer via `/input`:
   - `target:<serial>` — target a mobile/item (get the serial from
     `/scene.json`, e.g. `.player.serial`, or an item/mobile listed there).
   - `targetxy:<x>:<y>:<z>:<graphic>` — target a ground tile (get `x`/`y`/`z`
     from `/scene.json`'s `.player.pos`, or elsewhere).
   - Or drive it for real: `scripts/drive.py click <screen-x> <screen-y>`
     against the live canvas while the target cursor is active (the
     renderer's `scene.target.active === 1` state — see `web/main.js`).

| Command | AccessLevel | Target? | What it lets you verify |
|---|---|---|---|
| `go <x> <y> [z]` | Counselor | no | Teleport to exact coords (z optional, defaults to ground) — the main way to set up any scene. `Scripts/Commands/Handlers.cs:24,758` (`Go_OnCommand`, the `e.Length==2\|\|3` branch). |
| `go "<region name>"` / `go <OneWordRegionName>` | Counselor | no | Teleport to a named region's stored "go location" (quote multi-word names — `CommandSystem.Split` in `Server/Commands.cs:135` is quote-aware). E.g. `go Britain`. |
| `where` | Counselor | no | Prints your own map/x/y/z back to chat — cross-check against `/scene.json`'s `.player.pos` after a `go`. `Handlers.cs:32`. |
| `Self Add <type> [params]` | GameMaster | no (Self) | Spawn any item/mobile type by ServUO class name at your own location — combat/anim/corpse tests (`Self Add Orc`, `Self Add Dragon`), inventory tests (`Self Add Gold 1000`, `Self Add Katana`, `Self Add BlackPearl`), vendor gump tests (`Self Add Cobbler`). `Commands.cs:471` (`AddCommand`, `Supports = Simple \| Self`). |
| `add <type>` (bare) | GameMaster | yes | Same as above but via `targetxy:<your x>:<your y>:<your z>:0` after sending the command — this is the "`[add` places at your feet" behavior (click yourself/your tile). `Commands.cs:515` (`AddCommand.Execute` uses the clicked point). |
| `Self Set <PropertyPath> <value>` | Counselor (write-gated per-property, GameMaster for most stats/skills) | no (Self) | Set stats/skills/etc. on yourself by dotted property path: `Self Set Str 100`, `Self Set Skills.Magery.Base 100` (`Skill.Base` is `[CommandProperty(Counselor, GameMaster)]`, `Skills.Magery` is a property of `Mobile.Skills` — `Server/Skills.cs:321`, `Server/Mobile.cs:7053`). |
| `Self Get <PropertyPath>` | Counselor | no (Self) | Read a property back (e.g. `Self Get Skills.Magery.Base`) to confirm a `Set` took. `Commands.cs:705`. |
| `set <PropertyPath> <value>` (bare, item-targeted) | Counselor (write-gated per-property) | yes | Set a property on a **clicked item**, not yourself — e.g. fill a spellbook: `Self Add Spellbook` then check `/scene.json` for its new serial, then `set Content 18446744073709551615` + `target:<spellbook serial>`. `Spellbook.Content` is `[CommandProperty(GameMaster)] public ulong Content` — `Scripts/Items/Equipment/Spellbooks/Spellbook.cs:171`. |
| `Self Kill` | GameMaster | no (Self) | Kill yourself on the spot — corpse rendering, death gump, resurrection flow. `Commands.cs:966,975` (`KillCommand`, `Supports = AllMobiles` which includes `Self`). |
| `Self Resurrect` (alias `Self Res`) | GameMaster | no (Self) | Resurrect yourself after the above. Same class, `value=false` branch. |
| `dupe [amount]` | GameMaster | yes (item, no `Self`) | Duplicate a targeted item `amount` times into your pack — quick way to get N of something. `Scripts/Commands/Dupe.cs:15` (`CommandSystem.Register` directly, not the generic framework — no `Self` variant exists). Answer with `target:<item serial>`. |
| `tele` | Counselor | yes (tile, no `Self`) | Teleport by clicking a destination tile (alternative to `go x y` when you want on-screen picking). `Commands.cs:531,535` (`TeleCommand`, `Supports = Simple` only — no `Self`, since "teleport to yourself" is meaningless). |
| `bank` | GameMaster | yes (mobile, no `Self`) | Open a mobile's bank box remotely — container/paperdoll/drag tests without walking to an actual bank. `Handlers.cs:61,380` (`Bank_OnCommand`, raw `BankTarget`). Answer with `target:<your own serial>` to open your own. |

---

## 5. Test spots (verified against ServUO's own `Data/Regions.xml`)

All of these come straight from the shard's actual region data
(`/Users/dkkang/dev/uo/servuo/Data/Regions.xml`), not guesses — coordinates
that ServUO itself treats as meaningful.

- **Guard-zone edge (Britain).** `Data/Regions.xml:260` — Britain's
  `TownRegion` (which *is* a `GuardedRegion`,
  `Scripts/Regions/TownRegion.cs:10: class TownRegion : GuardedRegion`) has a
  rect `x="1416" y="1498" width="324" height="279"`, i.e. the guarded area
  spans x∈[1416,1740), y∈[1498,1777).
  - **Just outside:** `go 1414 1600` (x=1414 < 1416).
  - **Just inside:** `go 1418 1600` (x=1418, well within, y=1600 mid-range).
- **Open field / town-center spot (Britain).** `go 1495 1629 10` — the
  region's own `<go>` point, `Data/Regions.xml:265`
  (`<go x="1495" y="1629" z="10" />`); also corroborated by
  `Scripts/Accounting/AccountHandler.cs:81`'s starting-city entry for Britain
  (`new CityInfo("Britain", "Sweet Dreams Inn", 1496, 1628, 10)`, 1 tile off
  from rounding) — solid ground-level (z=10), no elevation weirdness, good
  generic "known-good" test spot.
- **Elevation / stairs (Z transition).** `go 1523 1443 15` — Britain's
  "Blackthorn Castle" sub-region's `<go>` point,
  `Data/Regions.xml:268-270` (`<region priority="50" name="Blackthorn
  Castle"><rect x="1500" y="1408" width="46" height="90" /><go x="1523"
  y="1443" z="15" />`). z=15 is explicitly elevated relative to the outside
  ground (a real second-floor spot inside a real multi-story building) —
  teleporting there directly (3-arg `go`) drops you *at* that Z; walking
  there from ground level via the actual stairs is the real test (see the Z
  worked example below). `go "Blackthorn Castle"` (quoted, single argument)
  should also work via the named-region lookup in `Go_OnCommand`
  (`Handlers.cs:714-725`) — the 3-arg coordinate form above is the
  guaranteed fallback if that name lookup ever behaves unexpectedly.

---

## 6. The screenshot driver (`scripts/drive.py`)

See the script's own docstring (`python3 scripts/drive.py` with no args
prints it) for full usage. Summary:

```bash
# One-time setup: a venv with websocket-client, and Chrome with CDP open:
python3 -m venv .venv && . .venv/bin/activate && pip install websocket-client
google-chrome --remote-debugging-port=9333 --remote-allow-origins=* \
  --user-data-dir=/tmp/anima-chrome-profile &

# Open the play server's page once (or let `goto` do it):
python3 scripts/drive.py goto http://127.0.0.1:8788/ -- sleep 1.5 -- shot /tmp/world.png
```

Ops (chain with `--`): `goto <url>` · `eval <js-expr>` · `shot <file.png>` ·
`key <DOM-code>` · `click x y` · `dblclick x y` · `drag x1 y1 x2 y2` ·
`sleep <secs>`. `CDP_PORT` env var overrides the default port (9333) if you
run more than one Chrome instance.

**The pattern:** set up GM/movement state via curl/`scripts/gm.sh` against
`/input` (no browser involved), *then* screenshot/read via `drive.py` against
the browser tab that's actually showing the result — the two halves are
independent processes talking to the same `play` server.

Then **read the screenshot** (`Read` the PNG file) to actually look at it, or
`eval` a JS expression against the live `scene`/`settings` objects to check
state programmatically instead of visually (e.g.
`eval "scene.player.pos"`, `eval "settings.guardZones"`).

---

## 7. Worked end-to-end recipes

### Verify guard lines render at the real boundary

```bash
scripts/gm.sh 8788 'go 1414 1600'                     # just outside Britain's guard zone
python3 scripts/drive.py key KeyR -- sleep 0.3 -- shot /tmp/guard_outside.png
scripts/gm.sh 8788 'go 1418 1600'                     # just inside
python3 scripts/drive.py shot /tmp/guard_inside.png
```

Expect: `KeyR` toggles `settings.guardZones` on (`web/main.js:5589`,
`toggleGuardZones`), drawing the boundary as a gold-outlined polygon
(`guardLineLayer`, stroke colors `0xffd24a`/`0xffcc33` — `web/main.js:1924-2011`).
From outside (1414,1600) the line should appear to your east, roughly at the
x=1416 world column; from inside (1418,1600) you should be past it, no line
between you and the town center.

### Verify stairs / Z transition

```bash
scripts/gm.sh 8788 'go 1500 1443'          # ground level, just outside the castle rect
# now walk north (dir 0) repeatedly toward/into the building, watching stderr:
scripts/gm.sh 8788 --raw 'walk:0:1'
```

With `ANIMA_DEBUG=1` set on the `play` process, watch its stderr for:

- `[srv <ms>] MOVED (x,y) -> (x,y) confirms=N denies=N` — every accepted step
  (`play_server.rs:679-686`).
- `play: step dir=<d> (x,y) z <old> -> <new> (land z=<z>, <static note>)` —
  printed only when Z actually changes on a step, naming the static whose
  span covers the new Z if one is cheaply findable
  (`play_server.rs:705-715`) — this is *the* line that proves a stair/ramp
  climb resolved correctly.
- If a `walkto` ever silently fails to path, `[pathdbg] dir=<d> (x,y):
  ALLOW/DENY <reason>` for all 8 neighbors explains exactly why
  (`play_server.rs:96-145`, `debug_probe_neighbors`).

Optionally enable the in-page HUD instead of/alongside stderr-watching:
`python3 scripts/drive.py eval "settings.debugMove = true"` (Options panel's
"Movement debug" checkbox, `web/main.js:182,2729` — shows predicted vs.
server Z live over the character).

### Verify a spawned mob + corpse

```bash
scripts/gm.sh 8788 'Self Add Orc'
python3 scripts/drive.py shot /tmp/orc_spawned.png
scripts/gm.sh 8788 --raw 'autoattack'      # fight it (already-supported /input action)
# ... wait, screenshot again for corpse/death animation once it dies
```

---

## 8. Known gaps

- **No plumbing changes needed.** Everything above rides the existing
  `say:` chat path (`Action::Say` → normal 0x03/0x0D-family speech packet →
  ServUO's `CommandSystem` parses the `[` prefix) plus the existing
  `target:`/`targetxy:` actions. Nothing in `crates/` needed to change for
  this harness to work.
- **`animagm` must actually be GameMaster+** before any `[` command does
  anything (§3) — this doc can't do that step for you (needs `hulryung`
  logged in, or ServUO stopped).
- **Multi-word region names via bare `go`** are untested end-to-end here
  (quoting *should* work per `CommandSystem.Split`'s quote-awareness, but
  wasn't exercised against a live shard as part of writing this doc) — the
  3-arg coordinate form is the verified fallback everywhere above.
- **Housing/`multi.mul`** isn't rendered by this client yet (see
  `docs/DESIGN.md` §6 roadmap), so GM housing commands (`[house`, add a
  house sign/deed, etc.) aren't listed above — there's nothing yet to verify
  visually. Revisit this doc once multi-tile houses render.

# Client readiness — ongoing product audit

Objective: deliver a polished, usable Ultima Online client across macOS and
Windows, preserving the shared native/WASM/agent protocol core. A green unit
suite or an exhausted historical gaps list does not establish product readiness.

## Acceptance areas

| Area | Evidence required | Current evidence / next work |
| --- | --- | --- |
| First launch and game files | Clean launch, discover/pick/change valid files, actionable missing-file errors | Local setup/checklist, valid-folder gating, native picker, settings recovery and next-launch changes implemented. Actual macOS QA bundle verified through picker, invalid folder, exact recovery backup, login and in-app restart. Windows runtime and broader client-package compatibility checks remain; legacy-only world maps are explicitly unsupported. See GAME_FILES.md. |
| Accounts and servers | Multiple profiles, optional OS passwords, restart persistence, cache accuracy | Library/browser UI and OS-vault lifecycle checks passed, including restart reuse and deletion (Windows CI 34711822404, v0.7.0 source 10aa945). Writes stage profile data and attempt vault rollback on reported failures. Current source adds worlds export, preview/merge import and exact-original recovery. An isolated macOS app verified recovery, actual file import/export and duplicate-free reimport for two servers and three accounts. Windows interactive UI and installed-release checks remain. See LOGIN_PROFILES.md and NATIVE_BACKUP_VERIFICATION.md. |
| Connection lifecycle | Bounded waits, cancellation, accurate progress, reconnect after failure without stale world/input | Native cancel/deadlines, serialized scene polling, interruption UI and WASM callback isolation implemented. Loopback protocol fixtures, stale-cancel HTTP checks and Chrome outage/recovery verified. Missed native login-frame transitions now use a stable per-connection ID and reload once before assigning a new world. Input is bound at HTTP acceptance and queue consumption; character prompts and sound SSE are also isolated. Regressions cover identical player serials on different connections, pending actions and stale replies. Current-build live-shard disconnect/re-entry and relay runtime checks remain. |
| Core gameplay | Live movement/pathing, combat/casting/targeting, inventory/trade/vendor, chat/party, character creation/deletion | Historical ServUO evidence in TESTING/DESIGN/CLASSICUO_GAPS; current-build end-to-end audit still required. |
| Interface and accessibility | Usable default layout, keyboard/focus, resizing/scaling, readable feedback, no debug-only dead ends | Login library was visually verified at desktop/narrow widths; wider UI audit remains. |
| Settings and state | Reliable persistence, recovery from corrupt data, useful backup/restore, isolation across characters/windows | Desktop configuration recovery and Chrome preference recovery/reload passed. Renderer preferences validate data, preserve originals and migrate into one atomic record including geometry. Headless tests cover malformed geometry, restore/defaults and concurrent-window saves. A current macOS source build verified settings-file import/reload, actual export contents, previous-copy restore and separately named repeat downloads. Full app restart, Windows UI and actual game-window resizing remain; see CLIENT_SETTINGS.md and NATIVE_BACKUP_VERIFICATION.md. Character-specific layouts and the broader session/window-isolation audit remain. |
| Reliability and performance | No stale callbacks, clear disconnect/crash behavior, bounded resources, responsive long sessions | Connection races, login deadlines, preference boot failures and lost passwords on reported save errors repaired. Profile reads are bounded; corrupt profiles leave the recovery screen available. 2026-09-13 local full gate passed (333 web tests / 1664 assertions). Sound loading reads the bank lazily, bounds retained WAV/decoded caches and caps physical work; full-byte comparison across 4096 stock sound IDs passed. Texture byte/count retention, safe asynchronous eviction, alpha-mask ownership and retryable animation metadata now have regression and actual Chrome PNG/WebGL evidence. Whole-client long-session, remaining native/light caches and process-crash audits remain; see AUDIO_PERFORMANCE.md and GRAPHICS_CACHE.md. |
| Delivery | Versioned Mac/Windows installers containing current features, install/run checks, honest release notes and download links | v0.7.0 draft installers passed signature/notarization, mount/app-copy and silent install/uninstall checks (34712709301), but predate current source additions. A newer isolated Mac QA bundle now has graphical backup/recovery evidence. Version 0.8.0 and its draft notes prepare installers containing worlds backups, geometry, session isolation and sound/graphics improvements. Actual v0.8.0 build/install results, interactive installed-app checks and publication remain; public v0.6.0 is unchanged. |

## Verification rules

- Use `scripts/check.sh` and inspect its actual exit status; run relevant desktop
  tests and both platform CI compile checks.
- Use isolated protocol fixtures for failure/timeout tests. Label them as fixtures,
  not live-shard verification. Do not start/stop the user's ServUO instance.
- Capture real UI for user-visible feature announcements, preview the payload,
  then verify the published forum thread and image.
- Keep unfinished rows open until evidence covers their complete acceptance area.
  This ledger tracks the full objective; completing a repair is not completing
  the overall client.

## Session-isolation repair — 2026-09-13

- Before: a headless poll probe moved from serial 9 to 10 through the old renderer
  with zero reloads when it missed the login frame. Serial comparison alone would
  also miss different servers assigning the same number.
- After: 13 renderer regressions cover reload-before-assignment for equal and
  different serials, a one-shot reload latch, empty startup scenes, normal WASM
  ownership, session-bound input and sound events, character form reset, detached
  old rows, and late/pending Play, delete and cancel responses.
- Native loopback protocol fixtures log in twice with serial 42 and verify a
  distinct connection ID with stable scene serialization within each connection.
  Real HTTP requests reject missing/expired input IDs, wrong character-prompt
  IDs and stale login cancellation. Queue tests discard previously accepted
  actions after replacement; none of these fixtures connects to ServUO.
- The GM shell wrapper was separately exercised against ephemeral HTTP fixtures:
  one bound command succeeds, a 409 exits without replay, and a login-only scene
  sends no command. Evidence: `/tmp/anima-session-gm-fixtures.json`.
- `scripts/check.sh` completed with actual exit 0. Full log:
  `/tmp/anima-session-isolation-gate.log`. Web: 296 tests / 1486 assertions;
  Python tooling: 23 tests; desktop: 14 passed, 2 real-vault tests intentionally
  excluded from this ordinary gate. The prior geometry commit 4896eb0 passed all
  three platform/quality jobs in CI run 34714202675. Repair commit 33968c5 passed all three quality/platform jobs in CI run 34714856910.
- Interactive native re-entry and real-shard character operations remain open.
  No new release or feature announcement is justified by fixture evidence alone.

## Delayed sound cancellation — 2026-09-13

A headless deferred-decode probe reproduced a sound starting after mute was
already enabled (`/tmp/anima-pending-sound-probe.json`). Stopping effects now
invalidates pending playback callbacks as well as stopping active sources.
Mute, disabling effects, client transport loss and world reload all use that
path. Re-enabling sound does not revive an old request; a fresh request can still
reuse the decoded asset. Pending playback also checks the observed session ID.

Five regressions exercise these races through real audio functions with fixture
Web Audio nodes and controlled decode promises. `scripts/check.sh` completed
with actual exit 0 again: 301 web tests / 1503 assertions, 23 Python tooling
tests, native/WASM checks, and 14 desktop tests (2 real-vault tests excluded as
usual). Full log: `/tmp/anima-audio-cancellation-gate.log`. This verifies state
and callback behavior; listening in the current native apps remains open.
Commit 3911641 passed all three quality/platform jobs in CI run 34715009760. Sound-cache limits and loading admission are now implemented and verified separately below.

## Sound loading budgets — 2026-09-13

The sound pipeline now has lazy native file reads, WAV and decoded-buffer LRU
budgets, bounded HTTP bodies, load/decode admission limits, real physical-work
accounting after timeouts and expiring playback events. See AUDIO_PERFORMANCE.md
for measured before/after results and preserved stock-sound output. The full
local gate completed with actual exit 0 (314 web tests / 1575 assertions), and
an additional real-resource sound test passed. This source is newer than the
v0.7.0 draft installers. Platform CI 34715780452 passed Linux/macOS but exposed
three existing Windows fixture-path failures: SystemTime debug text in directory
names and a Unix-only `/dev/null` path. Commit `ab38450` made those test fixtures
portable. CI 34716082291 passed all three jobs, including native asset/connection
tests on both desktop platforms and the Windows Credential Manager lifecycle.

## Graphics cache repair — 2026-09-13

Textures now have byte/count retention, bounded load admission and retries.
Eviction waits for Pixi's asynchronous release before allowing the same URL to
reload. Live URL pools and actual on-stage texture sources are both protected,
including stationary previews and corpse clothing. Alpha hit masks share the
texture's lifetime. Animation counts/centers evict together; transport failures
and deadlines no longer leave permanent zero-frame records.

Nineteen focused regressions and a Chrome/WebGL fixture with 1,608 real generated
PNGs passed; the complete local gate returned exit 0 (333 web tests / 1664
assertions). See GRAPHICS_CACHE.md for exact coverage and limits. The broader
native/light-cache, whole-process and live-shard audits remain open. These
graphics and sound changes, like worlds backups and session isolation, are
source additions absent from the v0.7.0 draft installers.

CI 34727768842 passed all three Linux/macOS/Windows jobs for graphics commit
`e6bb50c`. Native macOS backup/recovery acceptance progressed separately; see
NATIVE_BACKUP_VERIFICATION.md for the completed file operations and the remaining
restart/interactive checks.

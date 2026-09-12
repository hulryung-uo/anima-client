# Client readiness — ongoing product audit

Objective: deliver a polished, usable Ultima Online client across macOS and
Windows, preserving the shared native/WASM/agent protocol core. A green unit
suite or an exhausted historical gaps list does not establish product readiness.

## Acceptance areas

| Area | Evidence required | Current evidence / next work |
| --- | --- | --- |
| First launch and game files | Clean launch, discover/pick/change valid files, actionable missing-file errors | Local setup/checklist, valid-folder gating, native picker, settings recovery and next-launch changes implemented. Actual macOS QA bundle verified through picker, invalid folder, exact recovery backup, login and in-app restart. Windows runtime and broader client-package compatibility checks remain; legacy-only world maps are explicitly unsupported. See GAME_FILES.md. |
| Accounts and servers | Multiple profiles, optional OS passwords, restart persistence, cache accuracy | Library and browser UI verified. Password changes stage file writes first and undo reported vault/rename failures; regressions cover lost credentials, rollback failure and oversized caches. Actual Keychain and Windows Credential Manager/profile lifecycle checks passed, including restart reuse and deletion (Windows CI 34711822404, v0.7.0 source 10aa945). New source adds non-secret worlds export, preview/merge import and exact-original profile recovery; fixture and real loopback HTTP checks passed. Windows interactive UI and actual backup file/recovery UI checks remain. See LOGIN_PROFILES.md. |
| Connection lifecycle | Bounded waits, cancellation, accurate progress, reconnect after failure without stale world/input | Native cancel/deadlines, serialized scene polling, interruption UI and WASM callback isolation implemented. Loopback protocol fixtures, stale-cancel HTTP checks and Chrome outage/recovery verified. Missed native login-frame transitions now use a stable per-connection ID and reload once before assigning a new world. Input is bound at HTTP acceptance and queue consumption; character prompts and sound SSE are also isolated. Regressions cover identical player serials on different connections, pending actions and stale replies. Current-build live-shard disconnect/re-entry and relay runtime checks remain. |
| Core gameplay | Live movement/pathing, combat/casting/targeting, inventory/trade/vendor, chat/party, character creation/deletion | Historical ServUO evidence in TESTING/DESIGN/CLASSICUO_GAPS; current-build end-to-end audit still required. |
| Interface and accessibility | Usable default layout, keyboard/focus, resizing/scaling, readable feedback, no debug-only dead ends | Login library was visually verified at desktop/narrow widths; wider UI audit remains. |
| Settings and state | Reliable persistence, recovery from corrupt data, useful backup/restore, isolation across characters/windows | Desktop configuration recovery is verified. Renderer preferences validate values, preserve originals and migrate into one atomic record; window geometry now joins that record and its backups, including adoption after older migrations. Headless geometry tests cover malformed values, restore/defaults and concurrent-window saves; Chrome preference recovery/reload previously passed. Worlds backups/profile recovery are implemented separately; Chrome showed the controls but confirmation handling stalled and the UI fixture remained intact. Actual file export/import, native profile recovery and real window resizing remain unverified; see CLIENT_SETTINGS.md and LOGIN_PROFILES.md. Character-specific layouts and the broader session/window-isolation audit remain. |
| Reliability and performance | No stale callbacks, clear disconnect/crash behavior, bounded resources, responsive long sessions | Connection races, login deadlines, preference boot failures and lost passwords on reported save errors repaired. Profile reads are bounded; corrupt profiles now leave the recovery screen available. 2026-09-13 local full gate passed with session isolation, profile and geometry regressions (23 launcher tests, 314 web tests / 1575 assertions), in addition to the earlier native profile/vault integration evidence. Sound loading now reads the bank lazily, bounds retained WAV/decoded caches and caps physical browser loading work. Full-byte comparison across 4096 stock sound IDs passed; standalone sound-reader peak RSS fell from about 390 MB to 121 MB during a scan. Whole-client long-session, remaining asset-cache and process-crash audits remain; see AUDIO_PERFORMANCE.md. |
| Delivery | Versioned Mac/Windows installers containing current features, install/run checks, honest release notes and download links | v0.7.0 draft contains both verified installers and SHA-256 manifests. macOS signature/notarization, DMG mount/app-copy and Windows silent install/uninstall passed; installer run 34712709301. The actual Mac process responds in login state, but graphical startup was not observable. The newer worlds backups, profile recovery, preference geometry and session-isolation additions are source-only and need a subsequent build. Interactive app checks and publication remain; public v0.6.0 predates these features. See releases/v0.7.0-verification.md. |

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
v0.7.0 draft installers; platform CI for this change is pending.

The broader cache audit remains open. The texture cache has live-sprite
protection and an idle/count policy, but no byte budget. Alpha hit masks are
retained independently of texture eviction. Animation metadata's loading keys
are never removed after completion, and failed metadata is retained without a
retry path. These findings need targeted reproduction and repair; they are not
covered by the sound-memory measurement. Interactive native and live-shard
acceptance items in the matrix remain unchanged.

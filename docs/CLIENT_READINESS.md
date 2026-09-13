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
| Interface and accessibility | Usable default layout, keyboard/focus, resizing/scaling, readable feedback, no debug-only dead ends | Basic login now asks only for account name/password after server selection, with optional saving and folded management. Browser layout/save-dialog checks passed. Actual v0.8.1 Mac installer reached this login screen; further native interaction was interrupted. Renderer regressions cover native-select Enter, profile readiness and in-flight submission locks. Wider keyboard/accessibility and game-window audit remains. |
| Settings and state | Reliable persistence, recovery from corrupt data, useful backup/restore, isolation across characters/windows | Desktop configuration recovery and Chrome preference recovery/reload passed. Renderer preferences validate data, preserve originals and migrate into one atomic record including geometry. Headless tests cover malformed geometry, restore/defaults and concurrent-window saves. A current macOS source build verified settings-file import/reload, actual export contents, previous-copy restore and separately named repeat downloads. Full app restart, Windows UI and actual game-window resizing remain; see CLIENT_SETTINGS.md and NATIVE_BACKUP_VERIFICATION.md. Character-specific window geometry now has automated identity/backup checks and an isolated real-browser two-character save/reload verification. Native installed-app character switching and the broader session/window-isolation audit remain. |
| Reliability and performance | No stale callbacks, clear disconnect/crash behavior, bounded resources, responsive long sessions | Connection/session races, pending audio, corrupt profile/settings handling and bounded sound loading have automated and selected runtime evidence. Texture/animation retention and retry have Chrome/WebGL evidence. World-map caches now identify resource folders and source metadata and reject corrupt records. Light-mask retry/retention and native light allocation bounds are implemented; stock-light tests passed. Latest local gate: 370 renderer tests / 1845 assertions. Whole-client long sessions, remaining native graphics caches and crash audits remain. See AUDIO_PERFORMANCE.md and GRAPHICS_CACHE.md. |
| Delivery | Versioned Mac/Windows installers containing current features, install/run checks, honest release notes and download links | Immutable v0.8.3 (`c55d882`) passed all source, bundle and actual installer checks in run 34732482083. Both local downloads matched manifests and checksums; the Mac app copy passed signature, notarization and Gatekeeper checks. Candidate remains a draft. Installed-app interaction and live gameplay remain unverified for this version; public v0.6.0 is unchanged. |

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

## Login keyboard repair — 2026-09-13

The native Mac QA app exposed an unintended connection when Return confirmed a
saved-account menu choice. The login handler registered Enter on every select
and input, including file pickers and checkboxes. It now only submits from text,
password and number fields; native option confirmation and file-picker actions
retain their default behavior. IME composition and held-key repeats do not submit.

Four automated regressions cover these behaviors and deliberate password-field
submission. Against the pre-fix source, three fail; against the repaired source,
all pass. The complete local gate returned exit 0: 337 web tests / 1686 assertions,
23 Python tooling tests and native/WASM checks including 14 desktop tests (two
real-vault tests excluded). Log: `/tmp/anima-login-keyboard-gate.log`.
The repaired code has not yet been checked in a rebuilt native app or installer.
It is newer than immutable tag v0.8.0 (`97c6722`) and will require a subsequent
release. CI 34728265793 passed for that tag's source; its installer build
34728267361 was still running when checked. Public v0.6.0 remains unchanged.

## Profile availability and login controls — 2026-09-13

Connect now stays disabled while profile loading, recovery or a profile operation
prevents a login. Successful retry or completion restores it immediately.
Keyboard submission observes the same readiness gate. The profile callback
preserves an in-flight login's button lock, including when a scene refresh
arrives before the POST completes. A save failure that requires profile recovery
keeps Connect disabled; an ordinary transport failure allows retry. An existing
character session can still use Play when account storage is unavailable.

Five regressions exercise deferred load/failure/retry, deferred profile writes,
credential storage followed by a pending login and scene refresh, recovery during
save, and character selection. Four fail against the previous source. All pass
with the repair, and the complete gate returned actual exit 0 (342 web tests /
1704 assertions, 23 Python tooling tests, native/WASM checks, 14 desktop tests
with the usual two real-vault exclusions). Log:
`/tmp/anima-login-availability-gate.log`. This source is newer than v0.8.0 and has
not been verified in a rebuilt native app. No additional UI automation was used.

## Installer delivery — v0.8.0 verified, v0.8.1 prepared

Release run `34728267361` completed successfully for immutable v0.8.0 commit
`97c67220f71794a23b3aa86b294032183a84eab0`. Both bundles and the checks against
actual draft assets passed: macOS signature/notarization, disk-image mount and
app copy; Windows silent installation/uninstallation. GitHub release ID
`387749521` is still a draft with both installers and three verification files.
The downloaded build manifests identify exactly that source commit:

| Asset | Bytes | SHA-256 | Signing |
| --- | ---: | --- | --- |
| Anima_0.8.0_aarch64.dmg | 5086381 | f6a7b1d94f53a9f1b25c228b6b683a276906bc160fc6cdcbad82ddd494721c07 | notarized |
| Anima_0.8.0_x64-setup.exe | 3377852 | 85e21f117a2c5df7ebebc0151c1485558c48ca8cedf6716264fb2d7f61bd5ec4 | unsigned |

These checks do not launch the game UI. The subsequent source fixes `60c6880`
and `6817083` are being packaged as v0.8.1 without moving v0.8.0. The v0.8.1
candidate's full local gate returned actual exit 0 (342 web tests / 1704
assertions, 23 Python tooling tests and native/WASM/desktop checks), logged at
`/tmp/anima-v081-candidate-gate.log`. Its build and installed-app evidence remain
pending. Public v0.6.0 has not been changed.

## Fast account login and optional saving — 2026-09-13

The user's clarified flow is account name plus password, with server selection
already established. Address, notes and optional account nicknames are now in
folded management sections; adding a server opens its address editor. The client
sends the ordinary login request for both existing and new shard credentials.
It adds no separate registration operation: automatic account creation remains
the shard's responsibility.

New/changed local profiles offer Save & connect, Connect without saving, and
Back. Password storage is a separate desktop-only choice. An unchanged saved
account reuses its native reference without asking again or rewriting profiles.
Declining storage sends no launcher write; changing endpoints without saving
cannot reuse a vault reference bound to the previous endpoint. Escape/Back sends
no credentials, restores the form, and returns keyboard focus to Connect. An
incoming character session dismisses a stale save prompt.

Nine additional regressions cover management visibility, one-time login without
profile writes, opt-in password storage, prompt cancellation/retry, endpoint
isolation, ordinary login without registration, and stale prompt dismissal.
The complete gate returned actual exit 0: 351 renderer tests / 1749 assertions,
23 Python tooling tests and native/WASM/desktop checks. Log:
`/tmp/anima-fast-login-final-gate.log`.

A brief Chrome check used the real HTML/CSS/renderer with a fixture bootstrap
that opens login, one synthetic server and no accounts. Two screenshots verified
the folded layout and save-choice dialog; the one-time option received focus,
and Escape returned to the enabled form. The fixture rejects all POSTs and
performs no shard connection or profile write. Its temporary server and tab were
closed afterward; evidence index: `/tmp/anima-fast-login-qa.json`. This is browser
source-UI evidence, not installed-native or live-account verification. Version
0.8.1 includes this clarified flow; its tag had not been created before these
changes, so no existing release tag was moved.

## One-time login and character-cache binding — 2026-09-13

A real loopback HTTP regression now covers native login admission with a
persistent disposable profile file and an instrumented fixture vault. With a
null account reference, both typed and empty passwords pass through unchanged
and the other endpoint's saved password is never read. With a saved reference,
the native process binds the stored endpoint; a temporary typed password does
not replace its stored secret. Blank input resolves that original secret.
All four requests leave the profile file byte-for-byte unchanged and perform
zero vault writes. The game connection loop is never started, so this is HTTP
admission/resolution evidence, not live authentication or actual OS-vault UI.

The equivalent renderer audit reproduced a browser-only defect: editing the
username and declining storage could cache the new account's characters under
the previously selected saved account. Browser caching now captures the actual
login binding (account, endpoint and relay) and verifies it against the latest
stored library before writing. One-time accounts/changed relays have no saved
binding. Concurrent notes are preserved; changed endpoints and corrupt originals
are not overwritten. Four browser regressions cover these cases, including the
observed pre-fix failure.

The complete local gate returned actual exit 0 (355 renderer tests / 1757
assertions, 23 Python tooling tests, native/WASM/desktop checks) with the new
native HTTP test included. Log: `/tmp/anima-login-cache-binding-gate.log`.
These browser-cache source changes postdate immutable v0.8.1 (`f5ccd24`); its
installer build must not be described as containing them. No release tag was
moved and no public release was published. Native installed-app spot checks and
live-shard acceptance remain outstanding.

## v0.8.1 installer verification and world-map cache repair

Release run `34729212934` completed successfully for v0.8.1 source
`f5ccd248be1d9f85ca31c63f010d83ee6163616b`, including both installer checks.
Both actual draft installers were downloaded to `target/installers/v0.8.1` and
verified locally against their tagged source, file lengths and SHA-256 manifests:

| Asset | Bytes | SHA-256 | Signing |
| --- | ---: | --- | --- |
| Anima_0.8.1_aarch64.dmg | 5090221 | e5189922be8f5fb7708db1b78d7b793a6f186a351d764f8fd81c318e11303afc | notarized |
| Anima_0.8.1_x64-setup.exe | 3382240 | 9cc443e5557a5718f71b41619587dded9d268aafcc8d08955c8b432b73b3534a | unsigned |

The downloaded DMG also passed local image verification, read-only mounting and
copying to an isolated installation folder. The copied app passed strict code
signature validation, arm64/version checks, stapled-ticket validation and
Gatekeeper acceptance. The image was detached afterward. No app UI was launched.
Evidence: `/tmp/anima-v081-downloaded-manifests.json` and
`/tmp/anima-v081-installed-copy.json`. Windows installation/uninstallation passed
on its CI runner, not on this Mac. Interactive installed-app/gameplay checks
remain open. This draft was not published.

The subsequent native world-map repair separates cached maps by resource folder,
invalidates source metadata changes, rejects damaged/oversized payloads, and
uses unique atomic staging for concurrent writers. Six native regressions and
the complete local quality gate passed with actual exit 0 (355 renderer tests /
1757 assertions, 23 tooling tests, native/WASM/desktop checks). See
GRAPHICS_CACHE.md for scope and limits. Like the browser account-cache binding
repair, it is absent from immutable v0.8.1; no tag was moved.

## Installed macOS v0.8.1 startup — 2026-09-13

CUA launched the exact notarized application copied from the downloaded DMG at
`target/installers/v0.8.1/installed/Anima.app` (PID 46090, loopback port 8190).
The actual native webview reached the simplified login screen with folded server
and account management, an account-name/password form and Connect. The served
`js/04-launcher.js` matched `git show v0.8.1:web/js/04-launcher.js` byte-for-byte.
This advances installed-app startup evidence beyond mount/signature checks.

This launch used the standard `dev.anima.client` configuration, not the earlier
isolated Worlds QA identity. Subsequent CUA actions twice reported other app
input. The later screen contained a cached manual server probe and a refused
connection; the profile hash also changed. Those actions were not controlled
acceptance tests and are not attributed to this check. Automation stopped and
the app was left running. Saving-choice confirmation, keyboard Enter behavior,
restart persistence and live gameplay in this installed build remain unverified.
No saved password was retrieved. Evidence index:
`/tmp/anima-v081-native-login.json`.

## Light-mask recovery and retention — 2026-09-13

The canvas light-shape loader no longer stores permanent failure records. It
retries with backoff, cancels stalled image sources after five seconds and ignores
late callbacks. Admission is capped at eight active image loads; retention uses
256 variants and a soft 16 MiB RGBA budget that protects recently drawn masks.
The regular graphics sweep handles idle-scene pressure. See GRAPHICS_CACHE.md
for the fallback behavior and remaining native/whole-process limits.

Six new regressions and the existing lighting/texture tests passed. The complete
local gate returned actual exit 0: 361 renderer tests / 1791 assertions, 23 Python
tooling tests, native/WASM checks and the usual desktop tests. Log:
`/tmp/anima-light-shapes-final-gate.log`. The existing native app was left alone.
CI 34729855987 independently passed all three platform jobs for the preceding
world-map cache repair (`661d4b5`). Light-mask changes require their own platform
CI and a subsequent installer; immutable v0.8.1 does not contain them.


## Native light decoding — 2026-09-13

Light decoding now limits index reads to the addressable 100 entries and checks
pixel count/backing-file bounds before allocation. The 16 MiB RGBA per-shape
ceiling matches the renderer. Three native fixture regressions and both existing
real-resource light tests passed. The stock index's 55 valid shapes fit the
limit; the largest is 350×350. See GRAPHICS_CACHE.md for exact scope and custom
mask limits. The complete gate returned actual exit 0 (361 renderer tests /
1791 assertions, 23 tooling tests, native/WASM and desktop checks), logged at
`/tmp/anima-native-light-gate.log`. This source is newer than v0.8.1 and still
needs its own platform CI and later installer verification.


## v0.8.2 candidate

Version 0.8.2 and its player-facing notes package the browser account-cache
binding (`7e66cb0`), world-map cache isolation (`661d4b5`), light-mask retry and
retention (`d9046ce`), and native light bounds (`76d5629`). The complete candidate
gate returned actual exit 0: 361 renderer tests / 1791 assertions, 23 tooling
tests, native/WASM and desktop checks. Log:
`/tmp/anima-v082-candidate-gate.log`.

CI 34730266304 passed all three platform jobs for the light-mask change. Native
light-bound CI 34730425959 was still running when checked; the release workflow
will independently gate its exact tagged source before building installers.
No existing tag has been moved. Installer artifacts and interactive acceptance
for v0.8.2 remain pending; the running v0.8.1 app was not touched.


## v0.8.2 installers available for interactive acceptance

Release run `34730516431` completed successfully for exact source
`38febbe2c9e7e29d27e46f1b8c809fc61f969ad1`: all platform gates, both bundles,
draft assembly, macOS mount/copy and Windows install/uninstall checks passed.
GitHub release ID `387760123` remains a draft with both installers and three
verification files. The actual draft files were downloaded to
`target/installers/v0.8.2` and matched the tagged source, sizes and SHA-256:

| Asset | Bytes | SHA-256 | Signing |
| --- | ---: | --- | --- |
| Anima_0.8.2_aarch64.dmg | 5094944 | 3cbe93b0c366820b2fac87559b52425af0689e9488515b332ea21516f506b0f2 | notarized |
| Anima_0.8.2_x64-setup.exe | 3382885 | 1ba4b28032f1dff9cf3507163c0c75a9eb531463e39ae122212cc1210f95eda9 | unsigned |

The matching Mac build artifact was also verified locally as a disk image,
mounted read-only and copied to
`target/installers/v0.8.2-macos-build/installed/Anima.app`. Strict signature,
arm64/version, stapled-ticket and Gatekeeper checks passed. The final draft's
Mac manifest and bytes matched that already-checked artifact, so mounting was
not repeated. The image was detached; no v0.8.2 UI was launched and the existing
v0.8.1 app was left alone. Evidence:
`/tmp/anima-v082-macos-build-verification.json` and
`/tmp/anima-v082-downloaded-manifests.json`.

Native light-bound CI `34730425959` also completed successfully. The full scope
of installed-app interaction and live-shard acceptance remains open, with an
execution record in [INSTALLER_ACCEPTANCE.md](INSTALLER_ACCEPTANCE.md). A playable
shard and test account have been requested from the user; the last controlled
local probe was connection refused. Public v0.6.0 is unchanged. Installer
availability is not a claim that the overall client goal is complete.

### UOP directory parsing after v0.8.2

The shared eager/lazy UOP directory parser now rejects cyclic block chains and
negative directory or active payload addresses. Payload header addition is
checked before storing the entry. Unused records retain their existing skip
behavior. This prevents malformed game data from keeping directory traversal
running indefinitely; it does not change normal multi-block file ordering.

Three regression tests cover populated and empty cycles with a bounded fixture,
invalid addresses, ignored empty entries, and valid multi-block payloads. All
nine non-ignored UOP tests passed, and the existing real-file eager/lazy comparison
passed explicitly. The full `bash scripts/check.sh` gate exited successfully;
logs are `/tmp/anima-uop-directory-gate.log` and
`/tmp/anima-uop-real-test.log`. This source change is newer than the immutable
v0.8.2 installers and does not close the outstanding interactive acceptance.

### Native animation cache retention after v0.8.2

UOP animation payloads now have least-recently-used eviction with a 16-entry /
64 MiB allocated-byte retention budget. Oversized custom payloads remain
readable without caching. Three cache regressions, the real-resource UOP frame
test, and the complete local quality gate passed (actual exit 0). See
[GRAPHICS_CACHE.md](GRAPHICS_CACHE.md) for accounting and remaining allocation
limits. No native UI or live shard was controlled for this repair; long-session
and installed-app acceptance remain open.

### Focused-button keyboard isolation

The global input guard recognized text fields and selects but not buttons.
A regression reproduced Space being prevented while a button was focused,
allowing game auto-attack to take precedence over native button activation.
Buttons now own their keyboard input, including Space, Enter and Tab; focused
button input also cannot start movement or target actions. Canvas shortcuts
remain active, and keyup still releases held movement after focus changes.

Two renderer regressions exercise the real global handlers. The complete local
gate exited 0 with 363 renderer tests / 1802 assertions and all native/WASM/
desktop checks. Evidence: `/tmp/anima-button-keys-before.log`,
`/tmp/anima-button-keys-after.log`, `/tmp/anima-button-keys-gate.log`.
This verifies event routing; native keyboard navigation remains an interactive
acceptance item. The preceding UOP parser and cache commits passed all three
platform jobs in CI run `34731642897` (`6c73556`).

The character-layout audit also confirmed that current geometry is intentionally
origin-wide. A durable layout identity must include the selected destination,
account and actual player, rather than the reconnect-specific `sessionId` or
player serial alone. Native `Session` currently retains no destination/account
identity after login; browser WASM connects through a relay URL. Both paths need
an explicit identity contract before character-specific geometry is implemented.
Existing startup panel restoration and backup/recovery behavior must also be
preserved. This remains unfinished work, not a claim of character isolation.

### Character window geometry implementation

The identity contract and scoped geometry described above are now implemented
for native and browser WASM renderers. Native login captures destination,
shard index and account without the password; WASM captures the connected relay
and account. Before the first world update, the renderer hashes that identity
with the actual player serial and restores the character's window positions and
sizes. Common defaults remain intact, including when logging in without saving
a launcher profile. Character layouts share the existing atomic preference
backup/recovery envelope. See [CLIENT_SETTINGS.md](CLIENT_SETTINGS.md) for the
precise scope, origin boundaries and unchanged-relay limitation.

Seven renderer regressions and the native loopback identity assertions passed;
the full local quality gate exited 0 (370 tests / 1845 renderer assertions).
Evidence: `/tmp/anima-character-layout-final-gate.log`. Live character switching,
native visual resizing and installed-release verification are still required.
This does not establish isolation for other common preferences or complete the
overall client acceptance areas.


### Real-browser character geometry verification

Source `47c18d5` was exercised in a hidden Codex in-app browser on an isolated
loopback origin. The fixture loaded the production scripts and replaced only
`main()` with a window harness; no shard or native app was controlled. Its
controls changed real DOM geometry, and the browser's real ResizeObserver
persisted dimensions through the production code.

Mage (serial 42) retained position 230/350 and body size 340×190. Warrior
(serial 43) retained position 560/410 and body size 270×150. Navigating to each
character again restored its own values; the common geometry remained empty
and the exported settings did not contain the fixture account name. A viewport
screenshot confirmed the restored Mage frame and readable contents after the
fixture result panel was shortened so it did not obscure the window.

Evidence: `/tmp/anima-character-layout-browser-result.json` includes source
hashes, observed geometry and limitations; the harness is
`/tmp/anima-character-layout-qa.py`. The temporary tab was closed and the
fixture server's termination was confirmed. This does not verify live-character
switching, native resize gestures or installed macOS/Windows behavior.

### v0.8.3 candidate preparation

Version 0.8.3 collects character window geometry, focused-button keyboard
isolation, native UOP animation LRU retention and malformed UOP directory
rejection. Its complete local gate exited 0 with 370 renderer tests / 1845
assertions, tooling, native/WASM and desktop checks. Evidence:
`/tmp/anima-v083-candidate-gate.log`. Player-facing draft notes are in
[releases/v0.8.3.md](releases/v0.8.3.md). Installer build and actual-asset
verification are still pending; no existing tag or installer is replaced.

### Real-browser button keyboard verification for v0.8.3

The tagged renderer's production input handlers were loaded in a hidden in-app
browser with a disposable canvas and HTTP receiver. Space and Enter each
activated the focused fixture button once; Tab focused the next button. The
receiver recorded no game input for those actions. Space on the fixture canvas
then produced exactly one `/input` request containing `autoattack`, preserving
the ordinary game shortcut. This verifies real browser key/default behavior,
not native installed-app or live-shard interaction.

Evidence and source hashes are in `/tmp/anima-button-key-browser-result.json`;
the fixture is `/tmp/anima-button-key-qa.py`, with its received requests in
`/tmp/anima-button-key-inputs.json`. The temporary tab was closed and fixture
server termination was confirmed. Release `34732482083` has passed all three
platform source jobs; both installer bundles are still running. Installed-app
acceptance now explicitly includes button keys and two-character window layouts.

### v0.8.3 Mac build artifact verification

Release run `34732482083` completed the Mac bundle and uploaded artifact
`10310465824` (`anima-v0.8.3-macos`). Its downloaded manifest matched the
immutable tag and source. `Anima_0.8.3_aarch64.dmg` is 5,098,213 bytes with
SHA-256 `9947970340a83387dc15bdcd597eef45a6cfa0f18749459f66276897c5361417`.

The disk image passed verification, read-only mounting and app copy. The copied
app passed strict signature, arm64/version 0.8.3, stapled-ticket and Gatekeeper
checks. It is at `target/installers/v0.8.3-macos-build/installed/Anima.app`;
the image was detached and the UI was not launched. Evidence:
`/tmp/anima-v083-macos-build-verification.json`. Windows bundling and final draft
asset verification are still pending; this is build-artifact evidence only.


### Verified v0.8.3 installer downloads

Release run `34732482083` completed successfully: exact-source platform gates,
both bundles, draft assembly and actual Mac/Windows installer checks. Both draft
files were downloaded to `target/installers/v0.8.3`; manifests and the supplied
`SHA256SUMS.txt` passed local verification.

| Asset | Bytes | SHA-256 | Signing |
| --- | --- | --- | --- |
| Anima_0.8.3_aarch64.dmg | 5098213 | 9947970340a83387dc15bdcd597eef45a6cfa0f18749459f66276897c5361417 | notarized |
| Anima_0.8.3_x64-setup.exe | 3389431 | 3a6ecfee7ebfe1a1a431e54ec5b60269e2d45ccbe90e910d667967d8ead36fbf | unsigned |

The final Mac manifest and bytes match the previously checked build artifact,
so its mount/copy/signature/ticket checks were not repeated. Windows silent
install/uninstall passed in the release workflow. Local evidence is
`/tmp/anima-v083-downloaded-manifests.json`; the Mac copy remains at
`target/installers/v0.8.3-macos-build/installed/Anima.app`. No v0.8.3 app UI was
launched. The draft and immutable tag are preserved; public v0.6.0 is unchanged.
Current installed-app interaction and live-shard acceptance remain open in
[INSTALLER_ACCEPTANCE.md](INSTALLER_ACCEPTANCE.md).


### v0.8.3 native Mac login and prompt verification

The exact notarized app copy opened at port 8191 while v0.8.1 remained running.
Its launcher script matched the tag. Native UI checks verified the optional-save
prompt, default-off password checkbox, default focus on Connect without saving,
Escape returning to login, and account-popup Down/Return without submission.
After clearing disposable input, the backend remained at login and the saved
profile's SHA-256 was unchanged. The 0.8.3 window remains open. Evidence is
`/tmp/anima-v083-native-login.json`; details and remaining scope are in
[INSTALLER_ACCEPTANCE.md](INSTALLER_ACCEPTANCE.md). The earlier old-app window
lookup failure did not prevent opening the new version by its exact path.
Live-shard checks still require a reachable designated shard and test account.

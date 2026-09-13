# Installed-client acceptance

Use this record for the exact candidate being tested. It complements the
GM-assisted scenarios in [TESTING.md](TESTING.md); source tests, installer
checks and interactive game results are separate evidence.

Current candidate: **v0.8.3**, immutable source
`c55d882b515793cd6547b68d29c7e9caa3b367b1`.
Release workflow: `34732482083` completed successfully, including all platform
source checks, both bundles, draft assembly and tests against actual downloaded
installers. Local downloads matched both manifests and `SHA256SUMS.txt`. The Mac
file matched the build artifact already verified by read-only mount/copy,
arm64/version, strict signature, stapled-ticket and Gatekeeper checks. Windows
silent install/uninstall passed on its runner. No interactive row below is
marked passed by these file checks.

Downloads: `target/installers/v0.8.3/Anima_0.8.3_aarch64.dmg` and
`target/installers/v0.8.3/Anima_0.8.3_x64-setup.exe`. Evidence:
`/tmp/anima-v083-downloaded-manifests.json` and
`/tmp/anima-v083-macos-build-verification.json`. The candidate remains a draft.

## Record the environment

Record installer filename/SHA-256, OS version and architecture, game-data
package, selected shard endpoint, test-account identifier and character name.
The login form can use the OS-saved password; evidence records need no password.
For an isolated developer launch, `ANIMA_DESKTOP_CONFIG_DIR` selects a separate
configuration/profile folder without changing the signed application. Keep the
running user's session intact. The local ServUO instance is user-managed and
must not be started or stopped by the test.

The pending environment decision is the playable shard and test account. The
last controlled probe of `127.0.0.1:2594` returned connection refused. This is
not an authentication failure and does not verify in-world behavior.

## Run these flows on each supported desktop platform

| Flow | Expected result | Evidence to retain |
| --- | --- | --- |
| Fresh installation and files | Setup accepts a compatible data folder, explains invalid/missing files, and reaches login | Installer hash; setup result; login screenshot |
| Ordinary account login | Select the server once, enter account name/password, and reach the shard's character list through the ordinary login request | Auth stages and character-list result |
| One-time account | Choose Connect without saving; account/password are not added to local profiles | Profile comparison and resulting character list |
| Save for next time | Save an account with password storage off, then exercise the optional OS-password choice with the designated test account | Saved profile identity; subsequent login outcome |
| Restart and quick return | Relaunch the same installed app, select the saved account and connect without another saving prompt or repeated address entry | Selected server/account and successful character list |
| Native controls | Confirm a saved-account menu using Enter; no connection occurs until deliberately submitting credentials or Connect | Before/after auth stage; focused control |
| Button keyboard input | Space/Enter activate a focused button and Tab moves focus without attacking or toggling war mode; game-canvas shortcuts still work | Focused control, click outcome and game state/input |
| Cancellation and retry | Back/Escape closes the save prompt; failed/refused login can be corrected and retried; cancellation during connection remains usable | Final enabled controls and subsequent successful attempt |
| Character entry | Choose an existing designated character and reach its world, inventory and status | Character identity, position, inventory and frame screenshot |
| Character window layouts | Move and resize windows for two designated characters, reconnect to each, and verify their separate layouts; settings export/restore retains both | Destination/account/character identifiers without passwords; before/after geometry and backup comparison |
| Movement and interaction | Walking/running, a blocked step, object use, targeting and an approved NPC/combat scenario produce matching server and client state | Short before/after state observations; visible rendering |
| Disconnect and re-entry | Return to login and re-enter; a controlled interruption recovers without stale character art, pending actions or sounds | Connection IDs/auth stages and the newly entered character |
| Accounts and server cache | Switch between the designated accounts/servers; character names and status remain associated with the correct profile | Selected profile and its cache before/after |
| Settings backup | Export settings/worlds, preview reimport and restore; repeat exports preserve earlier files | Downloaded-file comparison and visible restore result |
| Sustained use | Exercise a representative session with movement, inventory and effects; observe responsiveness and memory after revisiting earlier areas | Duration/workload, process measurements and any visible defects |

A row needs observed results from the candidate installer. Unit tests can
support its mechanisms but cannot stand in for the entire row. Record failures
with the trigger, expected/actual behavior and source version. Preserve tag
identity when fixing a defect; a changed installer gets a later version.

For remaining detailed acceptance areas, use [CLIENT_READINESS.md](CLIENT_READINESS.md).
Windows silent install/uninstall is already automated by the release workflow;
it does not close the Windows interactive rows in this record.


## Observed v0.8.3 Mac interaction

The exact notarized app copy was opened through its full path, reaching login
on port 8191 (PID 601); the existing v0.8.1 process remained running. This reused
the configured game files and existing library, so it is not a fresh-setup test.
The served launcher script matched the immutable v0.8.3 source byte for byte.

- Using another account with disposable input showed **Save for next time?**.
  Password storage was initially off and **Connect without saving** had focus.
- Escape dismissed the prompt, returned to login and focused Connect.
- The disposable fields were cleared. Selecting an existing account with Down
  and Return in the native popup changed the selected account without connecting.
- Final backend state was `auth: login`; the profile file's before/after SHA-256
  was identical. The v0.8.3 login window was left open. No saved password was read.

Evidence: `/tmp/anima-v083-native-login.json`; the native prompt screenshot was
visually inspected. This closes the Mac native-select check and verifies prompt
presentation/Escape, but not successful connection, saving through the prompt,
one-time login outcome, restart, or any live-character flow. Windows interaction
and the remaining rows still need their own observations.


## v0.8.3 Mac one-time connection failure and retry

The actual installed app submitted disposable credentials to a separate loopback
fixture on port 50715. The fixture accepted 83 login-handshake bytes and closed
without authenticating or creating an account. **Connect without saving** was
used twice; both failures displayed an actionable server-closed message and
returned enabled login controls. The second connection reached the fixture.
The saved profile file's before/after SHA-256 was identical. The temporary
listener exited after its two connections; selecting the existing server
cleared the disposable account/password fields.

Evidence: `/tmp/anima-v083-one-time-fixture.json` and its harness
`/tmp/anima-v083-one-time-fixture.py`. This verifies failure/retry and non-saving
in the installed Mac app; it does not verify successful login or live gameplay.

The test also found a defect: switching back to another saved server kept showing
the previous fixture's login error. Source after v0.8.3 now associates errors with
the submitted destination/account and hides them when the form selects another
one. The immutable v0.8.3 installers still contain the observed display defect.

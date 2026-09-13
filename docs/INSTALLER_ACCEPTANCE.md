# Installed-client acceptance

Use this record for the exact candidate being tested. It complements the
GM-assisted scenarios in [TESTING.md](TESTING.md); source tests, installer
checks and interactive game results are separate evidence.

Current candidate: **v0.8.2**, immutable source
`38febbe2c9e7e29d27e46f1b8c809fc61f969ad1`.
Release workflow: `34730516431` completed successfully, including both platform
quality gates, bundles and checks against actual draft installers. Both downloads
were verified locally; the Mac app copy also passed signature, notarization and
Gatekeeper checks. No interactive row below is marked passed by those checks.

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
| Cancellation and retry | Back/Escape closes the save prompt; failed/refused login can be corrected and retried; cancellation during connection remains usable | Final enabled controls and subsequent successful attempt |
| Character entry | Choose an existing designated character and reach its world, inventory and status | Character identity, position, inventory and frame screenshot |
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

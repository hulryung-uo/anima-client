# Client readiness — ongoing product audit

Objective: deliver a polished, usable Ultima Online client across macOS and
Windows, preserving the shared native/WASM/agent protocol core. A green unit
suite or an exhausted historical gaps list does not establish product readiness.

## Acceptance areas

| Area | Evidence required | Current evidence / next work |
| --- | --- | --- |
| First launch and game files | Clean launch, discover/pick/change valid files, actionable missing-file errors | Local setup/checklist, valid-folder gating, native picker, settings recovery and next-launch changes implemented. Actual macOS QA bundle verified through picker, invalid folder, exact recovery backup, login and in-app restart. Windows runtime and broader client-package compatibility checks remain; legacy-only world maps are explicitly unsupported. See GAME_FILES.md. |
| Accounts and servers | Multiple profiles, optional OS passwords, restart persistence, cache accuracy | Library and browser UI verified. Password changes stage file writes first and undo reported vault/rename failures; regressions cover lost credentials, rollback failure and oversized caches. Actual Keychain and Windows Credential Manager/profile lifecycle checks passed, including restart reuse and deletion (Windows CI 34711822404, v0.7.0 source 10aa945). New source adds non-secret worlds export, preview/merge import and exact-original profile recovery; fixture and real loopback HTTP checks passed. Windows interactive UI and actual backup file/recovery UI checks remain. See LOGIN_PROFILES.md. |
| Connection lifecycle | Bounded waits, cancellation, accurate progress, reconnect after failure without stale world/input | Native cancel/deadlines, serialized scene polling, interruption UI and WASM callback isolation implemented. Loopback protocol fixtures, stale-cancel HTTP checks and Chrome outage/recovery verified. New audit finding: when a native renderer misses the intervening login frame, a different character's world advances through the old renderer without a reload (headless probe: serial 9 → 10, zero reloads). A stable session identity is needed to catch changes even when different servers reuse the same player serial. This repair, current-build live-shard disconnect/re-entry and relay runtime checks remain. |
| Core gameplay | Live movement/pathing, combat/casting/targeting, inventory/trade/vendor, chat/party, character creation/deletion | Historical ServUO evidence in TESTING/DESIGN/CLASSICUO_GAPS; current-build end-to-end audit still required. |
| Interface and accessibility | Usable default layout, keyboard/focus, resizing/scaling, readable feedback, no debug-only dead ends | Login library was visually verified at desktop/narrow widths; wider UI audit remains. |
| Settings and state | Reliable persistence, recovery from corrupt data, useful backup/restore, isolation across characters/windows | Desktop configuration recovery is verified. Renderer preferences validate values, preserve originals and migrate into one atomic record; window geometry now joins that record and its backups, including adoption after older migrations. Headless geometry tests cover malformed values, restore/defaults and concurrent-window saves; Chrome preference recovery/reload previously passed. Worlds backups/profile recovery are implemented separately; Chrome showed the controls but confirmation handling stalled and the UI fixture remained intact. Actual file export/import, native profile recovery and real window resizing remain unverified; see CLIENT_SETTINGS.md and LOGIN_PROFILES.md. Character-specific layouts and the broader session/window-isolation audit remain. |
| Reliability and performance | No stale callbacks, clear disconnect/crash behavior, bounded resources, responsive long sessions | Connection races, login deadlines, preference boot failures and lost passwords on reported save errors repaired. Profile reads are bounded; corrupt profiles now leave the recovery screen available. 2026-09-13 local gate passed with profile and geometry regressions (23 launcher tests, 283 web tests), in addition to the earlier native profile/vault integration evidence. The newly found missed-session-transition case, long-session, asset-cache and process-crash audit remain. |
| Delivery | Versioned Mac/Windows installers containing current features, install/run checks, honest release notes and download links | v0.7.0 draft contains both verified installers and SHA-256 manifests. macOS signature/notarization, DMG mount/app-copy and Windows silent install/uninstall passed; installer run 34712709301. The actual Mac process responds in login state, but graphical startup was not observable. The newer worlds backup/profile recovery additions are source-only and need a subsequent build. Interactive app checks and publication remain; public v0.6.0 predates these features. See releases/v0.7.0-verification.md. |

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

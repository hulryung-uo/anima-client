# Client readiness — ongoing product audit

Objective: deliver a polished, usable Ultima Online client across macOS and
Windows, preserving the shared native/WASM/agent protocol core. A green unit
suite or an exhausted historical gaps list does not establish product readiness.

## Acceptance areas

| Area | Evidence required | Current evidence / next work |
| --- | --- | --- |
| First launch and game files | Clean launch, discover/pick/change valid files, actionable missing-file errors | Local setup/checklist, valid-folder gating, native picker, settings recovery and next-launch changes implemented. Actual macOS QA bundle verified through picker, invalid folder, exact recovery backup, login and in-app restart. Windows runtime and broader client-package compatibility checks remain; legacy-only world maps are explicitly unsupported. See GAME_FILES.md. |
| Accounts and servers | Multiple profiles, optional OS passwords, restart persistence, cache accuracy | Library and browser UI verified. Password changes stage file writes first and undo reported vault/rename failures; six regressions cover lost credentials, rollback failure and oversized caches. Actual Keychain and Windows Credential Manager/profile lifecycle checks passed, including restart reuse and deletion (Windows CI 34711822404, v0.7.0 source 10aa945). Windows interactive UI and profile repair/export remain. |
| Connection lifecycle | Bounded waits, cancellation, accurate progress, reconnect after failure without stale world/input | Native cancel/deadlines, serialized scene polling, interruption UI and WASM callback isolation implemented. Loopback protocol fixtures, stale-cancel HTTP checks and Chrome outage/recovery verified. Current-build live-shard disconnect/re-entry and relay runtime checks remain. |
| Core gameplay | Live movement/pathing, combat/casting/targeting, inventory/trade/vendor, chat/party, character creation/deletion | Historical ServUO evidence in TESTING/DESIGN/CLASSICUO_GAPS; current-build end-to-end audit still required. |
| Interface and accessibility | Usable default layout, keyboard/focus, resizing/scaling, readable feedback, no debug-only dead ends | Login library was visually verified at desktop/narrow widths; wider UI audit remains. |
| Settings and state | Reliable persistence, recovery from corrupt data, useful backup/restore, isolation across characters/windows | Desktop configuration recovery is verified. Renderer preferences now validate values, preserve originals and migrate into one atomic record; backup/restore UI and native download handling are implemented. Headless regressions and Chrome recovery/reload passed. Actual file export/import remains unverified due browser file-access and native-window tooling limitations; see CLIENT_SETTINGS.md. Profile repair/export and character/window isolation remain. |
| Reliability and performance | No stale callbacks, clear disconnect/crash behavior, bounded resources, responsive long sessions | Connection races, login deadlines, preference boot failures and lost passwords on reported save errors repaired; 2026-09-13 gate passed (661 main Rust tests, 14 desktop tests, 267 web tests), plus one explicit native profile/vault integration test. Long-session, asset-cache and process-crash audit remains. |
| Delivery | Versioned Mac/Windows installers containing current features, install/run checks, honest release notes and download links | v0.7.0 draft preparation validates tag/version/commit, reuses all CI gates and requires both installer checksum manifests before assembling a draft. Signing configuration is explicit and macOS artifacts are verified before hashing. Actual v0.7.0 bundles, install/run checks and publication remain; public v0.6.0 predates these features. |

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

# Game files and first launch

Anima uses your existing Ultima Online installation. On first launch, review
the detected folder or use **Choose folder…**. You can also enter a path and
click **Check folder**. **Open Anima** becomes available when required files
are readable and the app configuration can be saved. Cancelling the picker
does not change your saved location. A valid saved folder opens login directly
on later launches; missing or moved files return to setup.

## What the check means

The checklist covers tile definitions, world map/buildings, world artwork,
character animations and interface artwork. Sound and English localized names
are optional. Use the client package recommended by your shard.

This build requires `map0LegacyMUL.uop`, `staidx0.mul` and `statics0.mul` for its
world map. A package with only `map0.mul` is not supported by the current map
reader. Artwork and interface readers can use their supported UOP or MUL/index
pairs; character animations require `anim.mul` and `anim.idx`.

Checks open the same essential readers used by the game and reject missing,
unreadable or obviously incomplete files. They do not scan every asset for
corruption, guarantee shard compatibility, download files or modify game data.

## Change the folder

Open **Anima settings → Game files…** (`⌘,` on macOS; `Ctrl+,` on Windows).
Choose and check a folder, then **Save for next launch**. The current session
keeps its existing files. **Restart Anima** disconnects the current game session
and reopens the app using the saved folder, or you can restart later.

## Recover app settings

The **App settings location** disclosure shows `config.json` and the active
game-file path. Normal updates lock, reread and atomically replace the file;
unknown fields and the first claimed loopback port are preserved.

Malformed or unsupported-version settings are never silently replaced. The
setup screen can **Back up and reset app settings**: it writes the exact original
bytes to `config.recovered-<id>.json` before creating fresh app settings. Select
the game folder again afterward. Saved servers/accounts in `launcher.json` and
passwords in the OS vault are separate and are not reset by this action.

Unreadable files, files over 1 MB and unwritable settings directories are left
unchanged and need their access/storage issue resolved. Recovery here covers
desktop app configuration, not renderer options or a corrupt account library.

## Verification — 2026-09-13

The complete `bash scripts/check.sh` gate passed with Rust 1.96.1: 655 main Rust
tests, 12 desktop tests, 253 web tests / 1,249 assertions, lint and native/WASM
compile checks. The desktop tests cover exact corruption backups, stale recovery
actions, unknown-field retention and concurrent port claims. UI regressions cover
unchecked edits, delayed status responses, reselecting the original folder and
disabled launch while settings require recovery.

Actual macOS debug app checks used a disposable app identifier/configuration
directory and the locally installed game files: detection, native picker selection
and cancellation, missing-folder rejection without saving, exact recovery backup,
opening login, changing files during a running client and successful in-app
restart to the same loopback origin. No real account login was needed. A separate
ignored reader test passed against those installed files.

`ANIMA_DESKTOP_CONFIG_DIR` can isolate config/profile files for desktop QA; use
a distinct app identifier too when testing browser state. Windows CI checks the
shell and its headless tests; Windows interactive checks and newly versioned
installers remain part of the [client readiness audit](CLIENT_READINESS.md).
The published v0.6.0 installers predate this source change.

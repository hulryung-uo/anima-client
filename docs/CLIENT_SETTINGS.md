# Client settings, backups and recovery

The source build validates renderer preferences before using them and exposes
**Settings & backups** at login. During play, use **Options → Settings data →
Backups & recovery**. Game-file setup and saved server/account profiles remain
separate; see [Game files](GAME_FILES.md) and [Login profiles](LOGIN_PROFILES.md).

## Persistence and recovery

Options, macros, map markers, quick buttons, counters and HUD preferences are
stored in one versioned `anima.preferences.v1` browser-storage record. It belongs
to this browser/webview origin, including its port. Different native windows on
different ports therefore retain separate renderer preferences.

Older per-key preferences are read without changing them. The first successful
save migrates all recognized groups into the new record; the legacy values stay
untouched. Normal option changes preserve unknown fields and merge changed
options with the latest saved object. The account library and OS credential
vault are outside this record and outside settings exports.

Invalid types, out-of-range options and malformed arrays use safe defaults or
retain valid entries. A macro with an invalid step is skipped as a whole, so
half of a damaged command sequence cannot run. Loading macros never executes
them. Restored quick buttons and panel positions are clamped into the viewport.

If storage is unavailable or full, the renderer continues and shows a warning.
Changes remain in memory; **Retry saving** retries them. Invalid saved data is
not overwritten by ordinary changes. At login, **Keep original & recover**
replaces invalid entries while retaining valid preferences and pending edits.
The replacement and the original values are written in a single storage update.
A failed write leaves the existing record unchanged.

## Transfer settings

**Export settings** prepares `anima-settings.json`. An explicit **Save** link
stays available while the dialog is open. The desktop downloader accepts only
settings JSON blobs from the active loopback renderer and saves them in Downloads;
other origins, filenames and destinations are rejected. Desktop download
completion or failure is shown in the dialog.

Select a settings backup (up to 4 MB) to review its filename, stored groups,
macros and markers. Invalid values, unknown storage groups and unsupported file
versions reject the whole import. **Apply backup & reload** is available at
login; log out before restoring. Applying replaces all renderer preferences,
so groups absent from the file return to defaults. A stale preview cannot
overwrite settings changed after it was opened.

The most recent recovery/restore retains one previous copy. **Review previous
settings** can restore a valid copy; **Download recovery copy** preserves original
values for inspection even when they are damaged. A corrupt whole record is
preserved as its exact original string. This is a single previous copy, not an
unlimited history. Keep exported backups for longer-term recovery.

## Verification status — 2026-09-13

Implemented, with the complete quality gate passing: 655 main Rust tests,
14 desktop tests, 267 web tests / 1,301 assertions, lint, native compilation
and WASM compilation. New tests cover storage denial, quota failure, validation,
legacy migration, original-data retention, previous-copy restore, stale review,
late file reads, live-session guards, explicit download links and native download
origin/path restrictions.

Chrome showed a real renderer boot with deliberately invalid legacy options,
the warning/recovery dialog, and a successful recovery/reload to login. The
captured screen uses a disposable local fixture and no real account. The macOS
QA app bundle also built successfully.

**Actual file export/import round trips remain unverified.** Chrome's extension
rejected file selection because file-URL access is disabled. A download was
prepared in the UI, but the automation did not observe completion or an output
file. The running macOS QA app could not be inspected because native computer
use returned `cgWindowNotFound`; that does not prove an app failure or a successful
download. Validate actual file contents, import/reload and previous-copy restore
through a usable native window or authorized browser file access before announcing
this entire feature as complete. Windows interactive checks also remain.

This source change is not included in the published v0.6.0 installers. The
[readiness audit](CLIENT_READINESS.md) tracks the broader outstanding work.

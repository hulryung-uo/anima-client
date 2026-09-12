# Server and account library

The login screen keeps a library of worlds on the left, the selected server and
account in the middle, and a server notebook on the right. This feature is in
the source build; it is not part of the previously published v0.6.0 installers.

## Connect and remember

1. Choose **Add server**, enter a name, host and port. **Server notes & advanced**
   holds a personal note and the shard index (normally 0).
2. Enter an account label and username. **Save account** remembers the account;
   **Connect** also saves the server and account before connecting.
3. Use **New** beside Saved accounts for another account on the same server.
   Select a world on the left to switch to its own account list.
4. After authentication, choose a character in the existing character picker.
   Returning with **Back** shows the account form again.

These are local profiles, not new accounts on the game server. **Remove** only
removes the saved profile from this device; it never deletes a game account or
character. The last selected server and account are restored for this window's
origin. Account/server data is shared across native windows and survives a port
change; window selection follows the existing per-origin preference storage.

## Connection progress and recovery

The login screen reports address lookup, TCP connection, authentication and
character-response progress. **Cancel connection** abandons that attempt and
returns to your saved profiles. Cancelled attempts cannot affect a newer login.
Progress and action buttons remain visible while the login form scrolls.

Native address lookup and dialing have an 8-second caller deadline, with a
separate bounded advertised game-server dial. Each server-response phase has a
20-second deadline, even if the server sends only part of a packet. Choosing a
character is not timed out while you decide; its response deadline starts again
when you submit. OS DNS resolution may finish in the background after cancellation;
that worker receives only the destination, never account credentials.

If the page loses contact with the native client, a **Client connection
interrupted** notice pauses input and macros. Scene requests retry automatically
with backoff; **Retry now** checks immediately. Recovery restores keyboard focus,
without replaying held keys or restarting a macro. Reopen Anima if its process
has stopped. A lost game-server session returns to sign-in; it does not silently
authenticate again. Browser WASM mode also cancels and times out login attempts,
and discards callbacks from replaced WebSocket connections.

## Optional passwords

In the macOS and Windows desktop app, check **Save password on this device**.
Passwords are stored through the OS vault (macOS Keychain / Windows Credential
Manager). The native process resolves the password when connecting; the page
receives a saved-password indicator, never a password getter.

A blank password field reuses a saved password. Typing a replacement and saving
updates it. Uncheck the option and save to remove the stored password. Removing
an account or server also removes its saved passwords. Changing a host, port or
shard clears saved passwords and cached character names so an old credential
cannot silently follow an edited destination. Vault failures are shown on screen.

Profile saves prepare and sync the replacement file before changing passwords.
If a password operation or the final file replacement fails, Anima attempts to
restore every affected password and leaves the original profile file in place.
If the vault also refuses recovery, the error asks you to unlock it and save
the affected passwords again. Temporary profile files contain no passwords and
are removed after a failed save. Saves also enforce the same 1 MB limit as reads,
so an oversized cache cannot make the next launch's profile file unreadable.

Server/account metadata lives in `launcher.json` in Tauri's app-config directory,
separate from the desktop configuration. Writes lock and reread the file before
atomic replacement so multiple windows cannot overwrite unrelated profiles.
Malformed or newer-version files are left untouched and reported. The file
contains usernames, notes and character names, so keep backups private. Passwords
are not included in a profile backup and must be saved again on another device.
This protects passwords at rest; it does not change UO's existing network protocol.

## Server notebook

**Check server** makes an account-free TCP connection and caches reachability and
connection latency. Compatible RunUO/ServUO public status replies can also supply
server name, reported client count and uptime. A reachable port is not a guarantee
that account login will succeed. The timestamps distinguish the latest connection
check from previously cached server details; failed or unsupported status queries
preserve older reported details. Checks are manual and briefly rate limited.

The selected account's last authenticated character list is cached separately.
It is a preview: authenticate again to get the current list before choosing a
character. A status check never sends an account or password.

## Other runtimes

- `ANIMA_LOGIN=1 cargo run -p anima-net --bin play` saves non-secret profiles in
  `$HOME/.config/anima-client/launcher.json`. Set `ANIMA_PROFILE_DIR` to use an
  isolated folder. This development server does not enable OS password storage.
- WASM browser mode saves non-secret profiles in browser storage. Each world can
  remember a relay URL; the relay's configured target determines the destination,
  while Host and Port are reference information. Shard selection currently stays
  at 0. Password saving and direct TCP status checks are unavailable in this mode.
- Native profile endpoints require a loopback peer, a literal loopback Host,
  same-origin requests and `X-Anima-Launcher: 1`. They are unavailable to LAN
  visitors even if the development server itself is exposed with `ANIMA_BIND`.

## Validation

The quality gate covers Rust formatting, strict Clippy, Rust tests, WASM compilation,
JavaScript syntax/shared globals, the renderer suite and desktop compilation.
Regression tests cover persistence, concurrent windows, credential isolation,
endpoint changes, vault failures, character cache invalidation and public status
parsing with a loopback fixture. The macOS vault roundtrip was also run with one
isolated test credential, then removed. Chrome UI checks used disposable profiles
for two servers and three accounts: saving, switching, reload restoration, notes,
status caching, failed-login recovery, and narrow-window layout (580px / 390px).
The real ServUO instance was offline; no live character login was claimed.
The explicit profile lifecycle test also passed against Windows Credential
Manager on the Windows CI runner for the v0.7.0 source commit (`10aa945`). This
verifies OS-vault calls and profile persistence, not the interactive Windows UI.

Password persistence regressions additionally cover failed file staging, failed
final replacement, partially failed multi-account deletion, failed rollback, and
moving an account between two saved servers with the same endpoint. An isolated
macOS Keychain integration test verifies actual profile save, reuse after restart,
replacement, preservation after a file-write failure and deletion. All test
credentials and profile files are removed afterward. Run it explicitly with
`cargo test -p anima-desktop native_profile_password_lifecycle -- --ignored`;
macOS CI leaves it opt-in. Windows CI explicitly runs it with one disposable
runner credential, which is removed by the test. This exercises the OS API and
profile lifecycle, not the Windows login UI. The launcher fixture suite also
runs on both macOS and Windows CI.

The rollback handles reported operation failures while the client is running;
the filesystem and OS vault do not offer a shared transaction across a sudden
process termination. No plaintext password journal is written to disk.

Connection recovery was additionally checked with isolated loopback protocol
fixtures: cancellation during DNS/authentication, partial-packet timeout, both UO
login phases, and human character-choice time excluded from the deadline.
HTTP checks confirmed an old cancellation ID cannot stop a newer attempt.
Chrome checks covered cancellation/retry, a 580px-wide window, and stopping and
restarting only the disposable native client: the interruption notice appeared,
background keyboard access was disabled, and recovery restored the selected
profile and focused field. These are fixture/UI checks, not live-shard gameplay
or Windows runtime validation.

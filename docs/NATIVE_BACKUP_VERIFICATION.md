# Native backup verification — 2026-09-13

An isolated macOS app was built from `e6bb50c678a860af1b2c641a69a42463d81ecb83`
with product name **Anima Worlds QA** and identifier
`dev.anima.client.worldsqa`. Only packaging identity changed; the embedded
renderer, native downloader and profile store were the current source.
`cargo tauri build --debug --bundles app --config <qa-identity.json>` returned
exit 0. This locally ad-hoc-signed QA app is not a distribution installer.

Its new private configuration folder was seeded with the existing valid UO
resource directory, loopback port 8198 and a deliberately malformed profile file.
The production app's configuration and accounts were not used. UI actions and
screenshots were through CUA; the native file picker was used for imports.

| Check | Observed result |
| --- | --- |
| Startup with unreadable profiles | Login and recovery controls remained available; ordinary profile edits were disabled |
| Keep original & recover profiles | UI reported the copy path and enabled the empty library; file comparison proved exact original bytes, with mode 0600 |
| Import a worlds JSON file | Preview listed two servers and three accounts; applying added exactly those profiles |
| Export worlds | UI reported completion; actual Downloads JSON matched the input server names, endpoints, notes, account labels and usernames |
| Reimport that downloaded file | Native picker and preview worked; application reported zero added servers and zero accounts, retaining both existing groups |
| Import settings | Preview showed four groups, one macro and one marker; applying reloaded to login |
| Export restored settings | Actual downloaded JSON retained options, macro, marker and supplied window geometry; the normally created journal geometry was also present |
| Review and restore previous settings | Preview showed the earlier one-group/zero-macro/zero-marker copy; applying reloaded to login |
| Export previous settings | A new `anima-settings (1).json` contained only the previous journal geometry; the first `anima-settings.json` was preserved with its four restored groups |
| Reapply downloaded settings | Selecting the four-group export again and applying its preview reloaded successfully to login |
| Keyboard export | Tab visibly focused Export settings; Return produced the second file and the completion message |

No successful server login occurred. A later saved-account selection followed
by Return unexpectedly attempted a connection to the refused loopback endpoint
25111. This exposed the Enter-key bug documented in CLIENT_READINESS.md.
The two fixture endpoints were loopback ports 25111 and 25112, with synthetic
account names and no saved passwords. The real local
ServUO listener was offline and was not started.

The downloaded four-group settings file was subsequently selected again and its
preview rendered correctly. After an interruption, control of the same running
app was reacquired and applying that preview reloaded successfully to login.
A full app restart remains unverified. Some modal observations omitted dialog content
from the AX tree while screenshots still showed it; keyboard operation and some
later AX observations worked. This is not a screen-reader compatibility claim.

Temporary evidence is indexed by `/tmp/anima-native-worlds-qa.json`. The QA app's
configuration is in `~/Library/Application Support/dev.anima.client.worldsqa`.
The verified downloads were 716-byte `anima-worlds.json` and 553-byte
`anima-settings.json`, followed by a separately named previous-settings export.
Screenshots were captured in the task conversation. These records prove the
listed macOS source-build flows, not Windows UI, live gameplay, arbitrary window
resizing or installed-release behavior.

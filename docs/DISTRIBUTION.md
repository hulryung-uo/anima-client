# Distribution — building & shipping the Anima desktop app

This is the how-to for turning the client into an installable **application** and
distributing it. See [`DESIGN.md`](DESIGN.md) for architecture; this doc is only
about packaging.

## What the app is

`anima-desktop` (Tauri v2, `crates/anima-desktop`) is the shippable artifact. It:

- runs the `anima-net` **play server in-process** (direct TCP to the UO server,
  no relay) on an ephemeral **loopback** port, and opens a native webview at that
  URL;
- **embeds the `web/` renderer into the binary at compile time**
  (`anima_net::play_server`), so there is **no npm/bundler step** and no `web/`
  directory to ship;
- **ships no Ultima Online game data.** The `.mul`/`.uop` files are copyrighted
  and large, and stay on the user's machine. On first launch the app locates them
  by auto-detecting known install locations (incl. a configured ClassicUO) and,
  failing that, a native folder picker — then remembers the pick
  (`anima_net::uo_dir` + the desktop's persisted config). So the bundle is small
  and self-contained, and the user brings their own client files.

Consequences for distribution: the download is a few MB of native shell + the
embedded renderer; it needs **no data bundling**, and there is nothing
copyrighted in what you hand out.

## Build (one command)

```bash
scripts/build-app.sh                 # bundle for this machine's architecture
scripts/build-app.sh --universal     # macOS: one binary for Intel + Apple Silicon
scripts/build-app.sh --bundles dmg   # narrow outputs (passes through to `cargo tauri build`)
```

The script installs `tauri-cli` v2 on first use (compiles from source, a few
minutes — kept out of the workspace so the everyday `cargo build` stays lean),
then runs `cargo tauri build` against `crates/anima-desktop/tauri.conf.json`.

Prerequisites:

- **Rust** (stable) + the repo's usual toolchain.
- **macOS**: Xcode Command Line Tools (`xcode-select --install`) for the linker
  and `hdiutil` (the `.dmg` step). Building a `--universal` binary needs both
  `x86_64-apple-darwin` and `aarch64-apple-darwin` rustup targets (the script
  adds them).
- **Windows**: the MSVC build tools; WebView2 (Tauri's NSIS installer bootstraps
  the WebView2 runtime, so end users need nothing extra on Win 11 / recent 10).

### Outputs

Under the shared workspace `target/` (a per-target subdir when you pass
`--target`, e.g. `target/universal-apple-darwin/release/bundle/`):

| Platform | Path |
|---|---|
| macOS app | `…/release/bundle/macos/Anima.app` |
| macOS disk image | `…/release/bundle/dmg/Anima_0.1.0_<arch>.dmg` |
| Windows installer | `…/release/bundle/nsis/Anima_0.1.0_x64-setup.exe` |
| Windows MSI | `…/release/bundle/msi/Anima_0.1.0_x64_en-US.msi` |

`<arch>` is `aarch64` / `x64` / `universal`. The version comes from
`tauri.conf.json` `version` — bump it there for each release.

> **Headless note (macOS):** Tauri styles the `.dmg` window with an AppleScript
> that needs a GUI/Finder session, so a plain `cargo tauri build` fails at the
> DMG step over SSH / in some CI. `scripts/build-app.sh` handles this: it builds
> the styled DMG when a session is available and otherwise auto-retries with
> `CI=true`, which skips only the cosmetic styling and still emits a fully
> functional DMG (app + drag-to-`Applications` symlink). The `.app` itself is
> never affected.

## macOS: Gatekeeper, signing, notarization

An **unsigned** `.app`/`.dmg` runs fine on the machine that built it, but on
another Mac Gatekeeper blocks it ("Anima is damaged" / "unidentified developer")
because the download carries a `com.apple.quarantine` xattr.

- **Quick, unsigned sharing** (testers): tell them to right-click the app →
  **Open** → **Open** (once), or run
  `xattr -dr com.apple.quarantine /Applications/Anima.app`. Fine for a handful
  of trusted users; **not** acceptable for public distribution.
- **Proper release** needs an Apple **Developer ID** ($99/yr), signing, and
  **notarization**. Tauri does it during `cargo tauri build` when these env vars
  are set (nothing else to wire):

  ```bash
  export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
  export APPLE_ID="you@example.com"
  export APPLE_PASSWORD="app-specific-password"   # appleid.apple.com → App-Specific Passwords
  export APPLE_TEAM_ID="TEAMID"
  scripts/build-app.sh --universal
  ```

  Tauri signs the app with a hardened runtime and submits it to Apple's
  notary service, stapling the ticket into the `.dmg`. See Tauri's macOS
  code-signing guide for CI keychain setup (`APPLE_CERTIFICATE` +
  `APPLE_CERTIFICATE_PASSWORD` to import a base64 `.p12`). If you later need
  entitlements, add `bundle.macOS.entitlements` in `tauri.conf.json`.

## Windows: installer & signing

- **Build on Windows** (or CI — see below). Cross-compiling a Windows bundle
  from macOS is impractical (WebView2 + the MSVC/NSIS toolchain); don't try.
- `scripts/build-app.sh` on Windows yields the NSIS `…-setup.exe` (recommended)
  and an `.msi`. The NSIS installer bootstraps the WebView2 runtime.
- **Signing** (optional, removes SmartScreen friction): sign with `signtool` and
  an Authenticode cert, or set Tauri's `bundle.windows.certificateThumbprint` /
  `signCommand`.

## Cross-platform releases via CI (recommended)

The practical way to produce **both** macOS and Windows bundles is a CI matrix —
you can't build a Windows installer on macOS. A ready-to-use GitHub Actions
workflow is provided at [`.github/workflows/release.yml`](../.github/workflows/release.yml):
push a `v*` tag and it builds `Anima.app`/`.dmg` on `macos-latest` and the
`.exe`/`.msi` on `windows-latest`, then attaches them to a GitHub Release.

Signing in CI is opt-in: add the Apple / Windows secrets above as repository
secrets and the same build step picks them up; without them you get unsigned
artifacts (fine for internal testing).

## Release checklist

1. Bump `version` in `crates/anima-desktop/tauri.conf.json`.
2. `scripts/build-app.sh --universal` (macOS) and a Windows build (locally or CI).
3. Sign + notarize (macOS) / sign (Windows) if distributing publicly.
4. Smoke-test the bundle on a clean machine: it should open the login page,
   auto-detect or prompt for the UO data dir, connect, and render.
5. Publish the `.dmg` / `-setup.exe` (GitHub Release, or your channel).

## Known limitations (carried from `anima-desktop/README.md`)

- **No graceful shutdown**: closing the window ends the process (and its
  background game-loop/HTTP thread). No data loss — it's a client — but there's
  no "stop accepting" hook.
- **Loopback only**: the embedded HTTP server binds `127.0.0.1`, never
  `0.0.0.0`. Nothing on the network can reach it (unlike the `play` bin's
  `ANIMA_BIND` escape hatch, which the desktop shell deliberately ignores).
- End users must supply their own legally-obtained UO client files.

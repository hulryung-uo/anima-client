# Distribution — building and shipping Anima

The desktop app embeds the renderer and runs its native UO connection inside
Tauri. It needs the player's existing Ultima Online data files and account; no
UO game data is bundled. First launch validates the selected data folder before
opening the login screen. See [game-file setup](GAME_FILES.md).

## Local builds

```sh
scripts/build-app.sh
scripts/build-app.sh --universal
scripts/build-app.sh --bundles app,dmg
```

Use the pinned Rust toolchain, Node and Python 3.9+ for `scripts/check.sh`.
The packaging script installs Tauri CLI v2 if needed. macOS builds need Xcode
Command Line Tools; Windows builds need MSVC and the Windows build environment.
`--universal` adds both Apple targets and builds for Intel and Apple Silicon.
The default local build targets the current machine.

Outputs are under `target/release/bundle/`, or the relevant target subdirectory:

| Platform | Installer |
| --- | --- |
| Apple Silicon macOS | `dmg/Anima_<version>_aarch64.dmg` |
| Windows x64 | `nsis/Anima_<version>_x64-setup.exe` |
| macOS application | `macos/Anima.app` |

Local builds can also produce MSI or universal macOS bundles. CI deliberately
ships one installer per supported platform. The Windows NSIS installer can
install the WebView2 runtime. The Apple Silicon download does **not** run on
Intel Macs; Intel needs its own Intel or universal build. Rosetta translates
Intel applications on Apple Silicon, not the reverse.

## Versioned release drafts

The release workflow builds installers without publishing them automatically.
A stable tag must exactly match `crates/anima-desktop/tauri.conf.json` and have
notes at `docs/releases/<tag>.md`, beginning with `# Anima <tag>`.

1. Finish the source changes and run `bash scripts/check.sh`.
2. Set the app version and write player-facing notes with outstanding runtime
   checks stated accurately. Commit and push them.
3. Push the matching `vX.Y.Z` tag. For a retry, run **Actions → Release → Run
   workflow** with that existing tag. A branch such as `main` is not a release.
4. The workflow resolves the tag to a commit, invokes the shared CI workflow
   for that exact commit, and waits for Linux, macOS and Windows checks.
5. Build the macOS and Windows installers on their respective runners. Verify
   macOS architecture, app version, code signature and configured notarization.
6. Hash the final installers after signing/notarization and collect one build
   manifest per platform. Both platform artifacts must be present and match
   their source commit, filename, size and SHA-256 before a draft is assembled.
7. Download and test the actual installers before publishing the draft. Complete
   the applicable [client-readiness checks](CLIENT_READINESS.md).

The draft contains both installers, `SHA256SUMS.txt`, `macos-build.json` and
`windows-build.json`. The manifests record the exact source commit, target,
signing result, file size and hash. Workflow artifacts are retained for 14 days;
the draft's release assets remain attached afterward. Download verification:

```sh
shasum -a 256 -c SHA256SUMS.txt
```

On Windows, compare `Get-FileHash <installer> -Algorithm SHA256` with the supplied
checksum. A checksum confirms the bytes, not runtime compatibility.

An existing public release is rejected by the draft tooling. Corrections need a
new version. Retries may replace assets on an existing draft, with both platform
manifests checked again. Drafts are not announced by the UO Tavern publisher.
Stable public releases are announced after publication; see
[forum updates](FORUM_UPDATES.md).

## Apple signing and notarization

Local builds default to ad-hoc signing when no identity is configured. This
makes the bundle signature internally consistent but supplies neither a
Developer ID nor notarization. A passing `codesign --verify` alone does not
prove that a downloaded application will pass Gatekeeper.

CI uses these repository secrets for a Developer ID build:

| Secret | Purpose |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Password for that certificate export |
| `APPLE_SIGNING_IDENTITY` | Full `Developer ID Application: …` identity |
| `APPLE_TEAM_ID` | Apple developer team identifier |

Notarization additionally uses these App Store Connect API-key secrets:

| Secret | Purpose |
| --- | --- |
| `APPLE_API_KEY` | API key identifier |
| `APPLE_API_ISSUER` | Issuer identifier |
| `APPLE_API_KEY_BASE64` | Base64-encoded `.p8` private key |

With no signing secrets, CI produces an ad-hoc signed testing draft. All four
signing values produce a Developer ID build. All three additional API-key values
enable notarization. A partial configuration fails explicitly instead of
silently producing a less completely signed build. Secret values are never
printed. The temporary `.p8` has owner-only permissions and is removed at the
end of the job.

Tauri signs and notarizes the application. The workflow then checks the app's
stapled ticket and Gatekeeper acceptance, separately submits the DMG, staples
it, validates its ticket and checks Gatekeeper acceptance of the disk image.
Hashes are generated only afterward. Build notes record the result actually
requested and verified by these steps; they never infer notarization from the
mere existence of repository secret names.

For local Developer ID builds, supply `APPLE_SIGNING_IDENTITY` and an installed
certificate. Tauri also supports local Apple-ID credentials or API-key credentials
for notarization. Keep them outside source control. See the official
[Tauri signing guide](https://v2.tauri.app/distribute/sign/macos/) for credential
setup and [Tauri action](https://github.com/tauri-apps/tauri-action) for build inputs.

Windows Authenticode signing is not configured in this workflow. Its manifest
therefore says `unsigned`; passing Windows compilation is not a claim about
SmartScreen or OS-vault runtime behavior.

## Verification limits

The current macOS target is Apple Silicon, with a configured minimum macOS 11.0.
That minimum still needs compatibility checks against the system WebKit APIs
used by the renderer. A bundle build or notarization does not prove UI behavior.
The release candidate must be exercised through clean setup, saved account login,
file backup/restore, disconnect/reconnect and gameplay on the target platforms.

The app uses loopback HTTP only. Closing its window ends the process and its
background server; shutdown during a vault/file update remains a separate crash
recovery concern. See the readiness audit rather than treating green CI as
completion of the entire product.

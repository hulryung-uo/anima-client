#!/usr/bin/env bash
# build-app.sh — build a distributable Anima desktop application bundle.
#
# Produces, under target/**/release/bundle/:
#   macOS   → macos/Anima.app  +  dmg/Anima_<ver>_<arch>.dmg
#   Windows → nsis/Anima_<ver>_x64-setup.exe  (+ msi/Anima_<ver>_x64_en-US.msi)
#   Linux   → appimage/*.AppImage, deb/*.deb, …
#
# The bundle ships NO Ultima Online game data — it is copyrighted and stays on
# the user's machine. Anima locates the user's .mul/.uop files on first launch
# (auto-detect of known install paths + a native folder picker; see
# crates/anima-desktop/src/main.rs and anima_net::uo_dir). The web renderer is
# compiled into the binary (anima_net::play_server embeds web/), so there is no
# npm / bundler step and nothing external to ship.
#
# Usage:
#   scripts/build-app.sh                 # build for this machine's architecture
#   scripts/build-app.sh --universal     # macOS: one binary for Intel + Apple Silicon
#   scripts/build-app.sh --bundles dmg   # narrow the outputs (any `cargo tauri build`
#                                        #   flag passes straight through)
#
# Code signing / notarization is NOT done here (an unsigned bundle still runs;
# see docs/DISTRIBUTION.md for the Gatekeeper story and how to sign for release).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# tauri-cli (v2) is the packager. Install once if absent (compiles from source,
# a few minutes). Kept out of the workspace so the everyday build stays lean.
if ! cargo tauri --version >/dev/null 2>&1; then
  echo "==> installing tauri-cli v2 (one-time; compiles from source)…"
  cargo install tauri-cli --version "^2" --locked
fi

# --universal is our shorthand for a fat macOS binary (Intel + Apple Silicon).
args=()
for a in "$@"; do
  if [ "$a" = "--universal" ]; then
    rustup target add x86_64-apple-darwin aarch64-apple-darwin >/dev/null 2>&1 || true
    args+=(--target universal-apple-darwin)
  else
    args+=("$a")
  fi
done

# macOS: a failed .dmg step can leave an orphaned read-write image attached and a
# temp `rw.*.dmg` behind, both of which block a retry. Clean them up.
cleanup_stale_dmg() {
  [ "$(uname)" = "Darwin" ] || return 0
  local dev
  dev="$(hdiutil info 2>/dev/null | awk '/rw\..*Anima.*\.dmg/{i=1} i&&/\/dev\/disk/{print $1; exit}')"
  [ -n "$dev" ] && hdiutil detach "$dev" -force >/dev/null 2>&1 || true
  rm -f target/release/bundle/macos/rw.*.dmg 2>/dev/null || true
}

# tauri reads crates/anima-desktop/tauri.conf.json; artifacts land in the shared
# workspace target/ dir. `set -u`-safe empty-array expansion for bash 3.2 (macOS).
run_build() { ( cd crates/anima-desktop && env "$@" cargo tauri build ${args[@]+"${args[@]}"} ); }

echo "==> building optimized release bundle (first run is slow — full LTO build of Tauri)…"
cleanup_stale_dmg
if ! run_build; then
  # The usual macOS failure is the .dmg window-styling AppleScript, which needs a
  # GUI/Finder session (it fails over SSH / headless / in some CI). If we already
  # got as far as producing the .app, retry with CI=true — that makes Tauri's
  # create-dmg skip the cosmetic styling and emit a plain, fully-functional .dmg
  # (still with the drag-to-Applications symlink). A real compile failure won't
  # have produced the .app, so we don't pointlessly retry it.
  if [ -d target/release/bundle/macos/Anima.app ]; then
    echo "==> styled .dmg step failed (needs a GUI session); retrying headlessly (CI=true)…"
    cleanup_stale_dmg
    run_build CI=true
  else
    echo "==> build failed before bundling — see the output above." >&2
    exit 1
  fi
fi

echo
echo "==> done. bundles produced:"
found=0
while IFS= read -r d; do
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    echo "    $f"
    found=1
  done < <(find "$d" -maxdepth 2 \
             \( -name '*.dmg' -o -name '*.msi' -o -name '*-setup.exe' -o -name '*.app' \) 2>/dev/null)
done < <(find "$ROOT/target" -type d -path '*/release/bundle' 2>/dev/null)
[ "$found" = 1 ] || echo "    (none found — check the build output above)"

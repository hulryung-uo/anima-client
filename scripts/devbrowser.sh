#!/usr/bin/env bash
# Open a CDP-driven browser at the play server WITHOUT stealing focus.
#
# Live verification against a running shard needs a real browser with a real
# GPU-backed canvas, and it is normally left open across many checks. Launching
# it the obvious way (running the Chrome binary directly, or `open` without
# flags) raises the window and takes the keyboard, which interrupts whatever the
# person at the machine was doing — repeatedly, since a verification session
# relaunches it whenever the page target is lost.
#
# `open -g` is macOS's own "launch but do not bring to the foreground", and
# `-n` makes it a separate instance so it never touches the user's own profile
# or windows. It is not reliable by itself — measured over repeated launches it
# let the window take focus 2 times in 4 — so the frontmost app is restored
# below if it moved. Everything CDP does afterwards — Page.reload,
# Input.dispatchMouseEvent, Page.captureScreenshot — is synthetic and does NOT
# raise the window; measured by reading the frontmost process before and after.
# The one call that WOULD is `Page.bringToFront`. Do not use it.
#
# Headless (`--headless=new`) also works here and renders WebGL through
# SwiftShader, but it is deliberately not the default: the window is worth
# having open to look at, it is only the focus that is unwelcome.
#
#   scripts/devbrowser.sh [url] [port]     # defaults: the play server, 9333
#   scripts/devbrowser.sh --headless       # no window at all
set -euo pipefail

HEADLESS=""
if [ "${1-}" = "--headless" ]; then HEADLESS="--headless=new --enable-unsafe-swiftshader"; shift; fi
URL="${1:-http://127.0.0.1:8788/}"
PORT="${2:-9333}"
PROFILE="/tmp/anima-devbrowser-$PORT"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

if curl -s --max-time 2 "http://127.0.0.1:$PORT/json/version" >/dev/null 2>&1; then
  echo "devbrowser: already up on $PORT"; exit 0
fi

# Who had the keyboard before we touched anything. `open -g` is macOS's own
# "launch but do not bring to the foreground" and it is what does the work here,
# but it is not reliable on its own: measured over repeated launches it let the
# new window take focus 2 times in 4. So the frontmost app is put back if it
# actually changed. Where System Events is unavailable both reads come back
# empty and this is a no-op, leaving the launch flag to do what it can.
FRONT_BEFORE="$(osascript -e 'tell application "System Events" to get name of first process whose frontmost is true' 2>/dev/null || true)"

ARGS=(--remote-debugging-port="$PORT" --remote-allow-origins='*'
      --user-data-dir="$PROFILE" --no-first-run --no-default-browser-check
      --window-size=1600,1000 "$URL")

if [ -n "$HEADLESS" ]; then
  # No window to raise, so the binary is launched directly.
  nohup "$CHROME" $HEADLESS "${ARGS[@]}" >"/tmp/anima-devbrowser-$PORT.log" 2>&1 &
elif [ "$(uname)" = "Darwin" ]; then
  open -g -n -a "Google Chrome" --args "${ARGS[@]}"
else
  nohup "$CHROME" "${ARGS[@]}" >"/tmp/anima-devbrowser-$PORT.log" 2>&1 &
fi

for _ in $(seq 1 40); do
  curl -s --max-time 2 "http://127.0.0.1:$PORT/json/version" >/dev/null 2>&1 && break
  sleep 1
done
curl -s --max-time 2 "http://127.0.0.1:$PORT/json/version" >/dev/null 2>&1 \
  || { echo "devbrowser: CDP never came up on $PORT" >&2; exit 1; }

FRONT_AFTER="$(osascript -e 'tell application "System Events" to get name of first process whose frontmost is true' 2>/dev/null || true)"
if [ -n "$FRONT_BEFORE" ] && [ -n "$FRONT_AFTER" ] && [ "$FRONT_BEFORE" != "$FRONT_AFTER" ]; then
  osascript -e "tell application \"System Events\" to set frontmost of process \"$FRONT_BEFORE\" to true" \
    >/dev/null 2>&1 || true
  echo "devbrowser: focus had moved to $FRONT_AFTER; gave it back to $FRONT_BEFORE"
fi
echo "devbrowser: CDP on http://127.0.0.1:$PORT  ->  $URL"

#!/usr/bin/env python3
"""drive.py — Chrome DevTools Protocol (CDP) driver for the anima-client web
renderer: navigate, eval JS, send key/mouse events, take screenshots.

Built for the GM-assisted testing loop in docs/TESTING.md: drive the live
`play` server's web page, screenshot it, and read the image back — the
"verify" half of "set up GM state, then verify the client rendered/behaved
correctly."

Setup
-----
1. A Python venv with `websocket-client` (the *sync* `websocket` package, not
   `websockets`):

       python3 -m venv .venv && . .venv/bin/activate
       pip install websocket-client

2. Chrome/Chromium with a remote-debugging port open and the DevTools
   handshake's Origin check relaxed (CDP checks `Origin` against a fixed
   allowlist by default; a plain script isn't in it, so both flags are
   needed):

       google-chrome \\
         --remote-debugging-port=9333 \\
         --remote-allow-origins=* \\
         --user-data-dir=/tmp/anima-chrome-profile   # keep it separate from your daily profile

   (macOS: `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome ...`)

3. Open the `play` server's page in that Chrome instance yourself (or `goto`
   it as the first drive.py command — see below) — drive.py attaches to the
   first open page/tab, it doesn't launch Chrome.

The CDP port is configurable via `CDP_PORT` (default 9333), so multiple
Chrome instances (e.g. one for manual poking, one for automation) don't
collide.

Usage
-----
    drive.py <cmd> [args...] [-- <cmd> [args...] ...]

Commands (chain multiple with `--`, executed in order, one connection):
    goto <url>                        navigate the page, wait ~1s for load
    eval <js-expression>              Runtime.evaluate, prints the JSON value
    shot <outfile.png>                full-viewport screenshot
    key <DOM-code, e.g. KeyR>         keydown+keyup — a tap, for toggles
    hold <DOM-code> <seconds>         keydown, wait, keyup — for anything the
                                      client only does *while* a key is down
    keydown <DOM-code>                press and leave pressed
    keyup <DOM-code>                  release
    click <x> <y>                     mousePressed+mouseReleased (left button)
    dblclick <x> <y>                  click, then a clickCount=2 click
    drag <x1> <y1> <x2> <y2>          mousedown, 8 interpolated moves, mouseup
    sleep <seconds>                   pause between steps (float OK)

Examples
--------
    # Screenshot the current page state:
    python3 scripts/drive.py shot /tmp/out.png

    # Load the play server's page, wait, then screenshot:
    python3 scripts/drive.py goto http://127.0.0.1:8788/ -- sleep 1.5 -- shot /tmp/world.png

    # Toggle the guard-zone overlay (R key) and re-screenshot:
    python3 scripts/drive.py key KeyR -- sleep 0.3 -- shot /tmp/guardlines.png

    # Walk east for two seconds. `key` would NOT do this: the client walks
    # while the key is held (the step queue commits one step per cadence
    # tick), so a tap moves at most one tile and usually none.
    python3 scripts/drive.py hold KeyD 2 -- sleep 1 -- eval "scene.player.x"

    # Read a value out of the page's live `scene` object:
    python3 scripts/drive.py eval "scene.player.x + ',' + scene.player.y"

Pattern used throughout docs/TESTING.md: drive GM state and movement via
`scripts/gm.sh`/curl against the `play` server's `/input` (headless, no
browser needed for that half), then use drive.py only for the
screenshot/verify half once the browser page is showing the result.
"""
import base64
import json
import os
import sys
import time
import urllib.request

from websocket import create_connection

CDP_PORT = int(os.environ.get("CDP_PORT", "9333"))


def connect():
    tabs = json.load(urllib.request.urlopen(f"http://127.0.0.1:{CDP_PORT}/json"))
    pages = [t for t in tabs if t.get("type") == "page"]
    if not pages:
        raise RuntimeError(
            f"no open page/tab on CDP port {CDP_PORT} — open the play server's "
            "URL in the Chrome instance you launched with --remote-debugging-port"
        )
    page = pages[0]
    ws = create_connection(
        page["webSocketDebuggerUrl"],
        timeout=30,
        header=[f"Origin: http://127.0.0.1:{CDP_PORT}"],
    )
    return ws


_id = [0]


def cmd(ws, method, params=None):
    _id[0] += 1
    i = _id[0]
    ws.send(json.dumps({"id": i, "method": method, "params": params or {}}))
    while True:
        r = json.loads(ws.recv())
        if r.get("id") == i:
            return r


# DOM `code` → (`key`, virtual-key code) for the keys that aren't a plain
# letter or digit. The web client dispatches on `e.code` alone, but `e.key` and
# `e.keyCode` are what a *focused input* (the chat bar, the macro recorder, a
# gump's text field) reads, so a synthetic event that omits them is only half a
# key press.
_NAMED_KEYS = {
    "ArrowLeft": ("ArrowLeft", 37), "ArrowUp": ("ArrowUp", 38),
    "ArrowRight": ("ArrowRight", 39), "ArrowDown": ("ArrowDown", 40),
    "Enter": ("Enter", 13), "NumpadEnter": ("Enter", 13),
    "Escape": ("Escape", 27), "Space": (" ", 32), "Tab": ("Tab", 9),
    "Backspace": ("Backspace", 8), "Delete": ("Delete", 46),
    "Home": ("Home", 36), "End": ("End", 35),
    "PageUp": ("PageUp", 33), "PageDown": ("PageDown", 34),
    "ShiftLeft": ("Shift", 16), "ShiftRight": ("Shift", 16),
    "ControlLeft": ("Control", 17), "ControlRight": ("Control", 17),
    "AltLeft": ("Alt", 18), "AltRight": ("Alt", 18),
    "Minus": ("-", 189), "Equal": ("=", 187),
}


def _key_fields(code):
    """`(key, virtual-key code, text)` for a DOM `code`. `text` is empty for
    keys that produce no character — Chrome uses it to decide whether the
    press also generates a `keypress`/input."""
    if len(code) == 4 and code.startswith("Key"):        # KeyA … KeyZ
        ch = code[3].lower()                             # unshifted: "d", not "D"
        return ch, ord(ch.upper()), ch
    if len(code) == 6 and code.startswith("Digit"):      # Digit0 … Digit9
        return code[5], ord(code[5]), code[5]
    if code in _NAMED_KEYS:
        key, vk = _NAMED_KEYS[code]
        return key, vk, key if key == " " else ""
    if code.startswith("F") and code[1:].isdigit():      # F1 … F12
        return code, 111 + int(code[1:]), ""
    # Unknown code: pass it through as its own `key` and let the page decide.
    return code, 0, ""


def key_event(ws, code, typ):
    key, vk, text = _key_fields(code)
    params = {
        "type": typ,
        "code": code,
        "key": key,
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk,
    }
    # Only a keyDown carries text; sending it on keyUp makes Chrome drop the
    # event as malformed.
    if text and typ == "keyDown":
        params["text"] = text
        params["unmodifiedText"] = text
    cmd(ws, "Input.dispatchKeyEvent", params)


def mouse(ws, typ, x, y, button="left", click_count=1):
    cmd(
        ws,
        "Input.dispatchMouseEvent",
        {"type": typ, "x": x, "y": y, "button": button, "clickCount": click_count},
    )


def run(ws, groups):
    for g in groups:
        if not g:
            continue
        op = g[0]
        if op == "goto":
            cmd(ws, "Page.enable")
            cmd(ws, "Page.navigate", {"url": g[1]})
            time.sleep(1)
        elif op == "eval":
            r = cmd(
                ws,
                "Runtime.evaluate",
                {"expression": g[1], "returnByValue": True, "awaitPromise": True},
            )
            res = r.get("result", {}).get("result", {})
            print(json.dumps(res.get("value", res.get("description", None))))
        elif op == "shot":
            r = cmd(ws, "Page.captureScreenshot", {"format": "png"})
            with open(g[1], "wb") as f:
                f.write(base64.b64decode(r["result"]["data"]))
            print("shot", g[1])
        elif op == "key":
            key_event(ws, g[1], "keyDown")
            key_event(ws, g[1], "keyUp")
        elif op == "hold":
            key_event(ws, g[1], "keyDown")
            time.sleep(float(g[2]))
            key_event(ws, g[1], "keyUp")
        elif op == "keydown":
            key_event(ws, g[1], "keyDown")
        elif op == "keyup":
            key_event(ws, g[1], "keyUp")
        elif op == "click":
            x, y = int(g[1]), int(g[2])
            mouse(ws, "mousePressed", x, y)
            mouse(ws, "mouseReleased", x, y)
        elif op == "dblclick":
            x, y = int(g[1]), int(g[2])
            mouse(ws, "mousePressed", x, y)
            mouse(ws, "mouseReleased", x, y)
            mouse(ws, "mousePressed", x, y, click_count=2)
            mouse(ws, "mouseReleased", x, y, click_count=2)
        elif op == "drag":
            x1, y1, x2, y2 = (int(v) for v in g[1:5])
            mouse(ws, "mousePressed", x1, y1)
            steps = 8
            for i in range(1, steps + 1):
                mouse(ws, "mouseMoved", x1 + (x2 - x1) * i // steps, y1 + (y2 - y1) * i // steps)
                time.sleep(0.05)
            mouse(ws, "mouseReleased", x2, y2)
        elif op == "sleep":
            time.sleep(float(g[1]))
        else:
            print("unknown op", op, file=sys.stderr)
            sys.exit(1)


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__, file=sys.stderr)
        sys.exit(1)

    groups, cur = [], []
    for a in args:
        if a == "--":
            groups.append(cur)
            cur = []
        else:
            cur.append(a)
    groups.append(cur)

    ws = connect()
    cmd(ws, "Runtime.enable")
    try:
        run(ws, groups)
    finally:
        ws.close()


if __name__ == "__main__":
    main()

#!/bin/sh
# gm.sh — thin curl wrapper around a running `play` server's POST /input,
# for driving ServUO GM ("[") commands and other actions from the shell.
#
# Usage:
#   scripts/gm.sh <http_port> <gm-command-without-the-bracket>
#   scripts/gm.sh <http_port> --say   <text>       # plain chat (no "[")
#   scripts/gm.sh <http_port> --walk  <dir>[:run]  # walk:<dir>:<0|1>
#   scripts/gm.sh <http_port> --raw   <input-line> # passthrough, e.g. targetxy:...
#
# Examples:
#   scripts/gm.sh 8788 'go 1416 1500'        # -> say:[go 1416 1500
#   scripts/gm.sh 8788 'add orc'             # -> say:[add orc
#   scripts/gm.sh 8788 --say 'hello world'   # -> say:hello world
#   scripts/gm.sh 8788 --walk 0:1            # -> walk:0:1  (run north)
#   scripts/gm.sh 8788 --raw 'targetxy:1495:1629:10:0'
#
# What it does: POSTs the assembled line to http://127.0.0.1:<port>/input,
# exactly what the web renderer's own input handling sends (see
# crates/anima-net/src/play_server.rs::parse_command). No auth, loopback
# only, by design (dev/test tool) — see docs/TESTING.md.
#
# THROTTLE: every "[" command rides a *speech* packet, and ServUO flood-protects
# speech per connection — fire these faster than ~1/sec (esp. with `&`) and the
# server drops you with "play: connection closed" (NOT a client/teleport bug; see
# docs/TESTING.md §8). Keep >= ~0.8s between invocations in loops.
#
# Requires: curl. POSIX sh (works with bash/dash/zsh's sh mode).

set -eu

usage() {
    echo "usage: $0 <http_port> <gm-command-without-[> " >&2
    echo "       $0 <http_port> --say <text> | --walk <dir>[:run] | --raw <input-line>" >&2
    exit 1
}

[ $# -ge 2 ] || usage

port="$1"
shift

mode="$1"
case "$mode" in
    --say)
        shift
        [ $# -ge 1 ] || usage
        body="say:$*"
        ;;
    --walk)
        shift
        [ $# -ge 1 ] || usage
        body="walk:$1"
        ;;
    --raw)
        shift
        [ $# -ge 1 ] || usage
        body="$*"
        ;;
    *)
        # Default mode: everything after <http_port> is a GM command, sent
        # as chat text with ServUO's "[" command prefix prepended.
        body="say:[$*"
        ;;
esac

# curl --data-urlencode would URL-encode the whole body as a form field; we
# want it as the raw POST body instead (that's what /input expects), so we
# just let curl send it verbatim with --data-binary. Spaces and everything
# else survive as-is over HTTP POST bodies — no encoding needed here.
resp=$(curl -sS --data-binary "$body" "http://127.0.0.1:${port}/input")

echo "-> $body"
echo "<- ${resp:-<empty response>}"

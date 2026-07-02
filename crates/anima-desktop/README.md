# anima-desktop

Standalone desktop shell (Tauri v2). Runs the `anima-net` play server
in-process — direct TCP to the UO server, no relay — on an ephemeral
loopback (`127.0.0.1`) port, then opens a native webview at that URL. The
web renderer is the same `web/` used by `anima-net`'s `play` bin, embedded
into this binary at compile time (see `anima_net::play_server`), so there's
no `web/` directory to ship and no npm/bundler step.

## Dev run

```bash
cargo run -p anima-desktop
```

First run: resolves the UO client data directory (`.mul`/`.uop` files) from
a persisted pick (Tauri app-config dir) or `$HOME/dev/uo/uo-resource`; if
neither looks valid (no `anim.mul`/`tiledata.mul`), a native folder picker
asks for it. Cancelling isn't fatal — the play server logs assets as "not
loaded" and still runs (in case you just want to poke at the login screen).

Once bound, a window opens showing the server/account login page. There are
no baked-in credentials (unlike `play`'s CLI-arg auto-login) — this is the
standalone default, matching the login page mode of the `play` bin
(`ANIMA_LOGIN=1 cargo run -p anima-net --bin play`).

## Bundling a real .app / .dmg (optional)

Not required for development. If you want an installable bundle:

```bash
cargo install tauri-cli --locked
cargo tauri build
```

This produces a `.app` (and `.dmg` on macOS) under
`target/release/bundle/`. The icons in `icons/` and `tauri.conf.json` in
this crate are already wired up for it (`tauri-build`'s codegen is what
`cargo build`/`cargo run` already exercise, so a plain build should mostly
just work — `cargo tauri build` additionally does the platform packaging).

## Notes / known limitations

- **No graceful shutdown**: the play server (game loop + `tiny_http`
  workers) runs on a background thread for the app's lifetime; closing the
  window ends the process, taking that thread with it. `tiny_http` has no
  clean "stop accepting" hook worth plumbing through today.
- **Loopback only**: the HTTP server binds `127.0.0.1`, never
  `0.0.0.0` — nothing on the network can reach it.
- Wire format / gameplay logic all lives in `anima-core`/`anima-net`; this
  crate is purely the native shell (window + folder picker + process
  wiring). It must never gain a reverse dependency into those crates for
  anything Tauri-flavored.

//! `play_server.rs` embeds the sibling `web/` directory into this crate at
//! compile time via `include_dir!("$CARGO_MANIFEST_DIR/../../web")`. That
//! macro call runs at compile time but — on stable Rust — has no way to
//! register its own extra dependency paths with cargo, so cargo has no idea
//! the embedded binary depends on anything under `web/`: editing e.g.
//! `web/main.js` doesn't trigger a rebuild of this crate, and the embedded
//! copy (what `anima-desktop` actually serves) silently goes stale relative
//! to disk. Emitting `rerun-if-changed` ourselves fixes that; cargo watches
//! a directory recursively, so any add/edit/remove under `web/` invalidates
//! the build. Path is relative to this crate's root (`CARGO_MANIFEST_DIR`),
//! matching the `include_dir!` call above.
fn main() {
    println!("cargo:rerun-if-changed=../../web");
}

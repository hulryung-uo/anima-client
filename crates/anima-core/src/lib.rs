//! # anima-core
//!
//! The headless heart of an Ultima Online game client: protocol, world model,
//! asset IO, and pathfinding — **with no rendering, UI, audio, or input**.
//!
//! The same core is shared by three consumers:
//! - **AI agents** (headless, many instances)
//! - the **browser client** (compiled to WASM)
//! - the **desktop client** (Tauri native backend, direct TCP)
//!
//! ## Layers
//! - [`net`] — UO wire protocol (big-endian packets, login + game codec, movement)
//! - [`world`] — the live game state (player, mobiles, items, journal)
//! - [`path`] — pathfinding over the map (terrain via the [`path::Terrain`] trait)
//! - [`agent`] — the Observation/Action contract (the brain/renderer seam)
//!
//! `.mul`/`.uop` file reading lives in the sibling `anima-assets` crate (it needs
//! zlib); it implements [`path::Terrain`]. Decision-making (the "brain": AI or a
//! human's input) lives *above* this crate, never inside it.

pub mod agent;
pub mod net;
pub mod path;
pub mod types;
pub mod world;

pub use agent::{dir_toward, Action, Brain, ItemView, MobileView, Observation, PlayerView};
pub use types::{Direction, Position, Serial};
pub use world::World;

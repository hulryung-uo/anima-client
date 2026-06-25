//! Network layer: the UO wire protocol (login + game phases).
//!
//! Everything is big-endian. Packets are either fixed-length (`[id][payload]`)
//! or variable-length (`[id][len: u16][payload]`).
//!
//! **Sans-IO:** this layer never touches a socket. [`packet`] holds big-endian
//! read/write primitives, [`lengths`] the framing table, [`framing`] turns a
//! byte stream into discrete packets, and [`login`] is the handshake state
//! machine. A thin native (or WASM/WebSocket) shim drives the actual IO and
//! feeds bytes in / sends bytes out. This keeps the protocol identical across
//! platforms and testable from byte vectors.

pub mod framing;
pub mod game;
pub mod huffman;
pub mod lengths;
pub mod login;
pub mod movement;
pub mod outgoing;
pub mod packet;

pub use game::apply_packet;
pub use movement::{build_walk_request, Walker};
pub use outgoing::build_client_version;

pub use framing::{FrameDecoder, FramingError, GameFrameDecoder, StreamDecoder};
pub use lengths::{packet_length, PacketLength};
pub use login::{LoginConfig, LoginDirective, LoginError, LoginMachine, LoginResult};
pub use packet::{PacketError, PacketReader, PacketWriter};

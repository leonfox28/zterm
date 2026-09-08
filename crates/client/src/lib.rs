//! Shared semantic terminal client behavior, independent of host PTYs and renderers.

pub mod error;
pub mod input;
pub mod pair_framing;
pub mod surface;

pub mod pairing;

pub mod model;
pub mod progress;
pub mod protocol;

pub mod framing;
pub mod session;
pub mod transport;
pub mod view;

// Compile the exact desktop adapter only for the moved owner tests. This keeps
// real direct/tunnel fixtures without a duplicate adapter or host dependency.
#[cfg(all(test, unix))]
extern crate self as zterm_client;
#[cfg(all(test, unix))]
#[allow(dead_code)]
#[path = "../../daemon/src/client/transport.rs"]
mod desktop_transport;

pub mod unary;

pub mod remote_unary;

pub mod handshake;

pub mod route;

pub mod iroh_controller;

pub mod keyboard;
pub mod mouse;

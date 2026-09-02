//! Host-only authoritative terminal engine for zterm.
//!
//! This crate is the only zterm-owned dependency boundary around Alacritty's
//! terminal state engine. PTY ownership remains in `zterm-platform`.

mod ansi;
mod engine;
mod ingress;
mod model;
mod projection;

pub use model::{TerminalCheckpoint, TerminalError, TerminalModel};

/// Maximum UTF-8 bytes projected from one terminal cell.
pub const MAX_CELL_TEXT_BYTES: usize = 22;

/// Maximum retained combining-character bytes across both model screens.
pub const MAX_COMBINING_BYTES_PER_SESSION: usize = 64 * 1024;

/// Maximum retained cells with combining-character storage across both model screens.
pub const MAX_COMBINING_CELLS_PER_SESSION: usize = 4_096;

/// Maximum bytes retained for one control sequence before it is contained.
pub const MAX_CONTROL_SEQUENCE_BYTES: usize = 256;

/// Maximum bytes retained for one OSC/DCS/APC/PM/SOS string before containment.
pub const MAX_CONTROL_STRING_BYTES: usize = 1_024;

/// Maximum canonical reply bytes emitted by one ingest update.
pub const MAX_REPLY_BYTES_PER_UPDATE: usize = 64 * 1024;

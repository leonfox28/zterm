//! Bounded, content-free local diagnostics shared by desktop and Android.

mod event;
mod reader;
mod recorder;

pub use event::*;
pub use reader::*;
pub use recorder::*;

#[cfg(test)]
mod tests;

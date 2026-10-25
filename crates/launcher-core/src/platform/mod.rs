//! OS primitives the engine and console use.

pub mod awake;
pub mod child;
pub mod env;
#[cfg(windows)]
mod icmp;
pub mod ping;
pub mod process;
pub mod pty;
pub mod shell;
pub mod trash;
pub mod trust;
#[cfg(windows)]
pub mod wasapi;
pub mod watch;

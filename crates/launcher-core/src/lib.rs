//! The launcher's engine: everything that does not need a window (docs/PLAN.md, target
//! architecture). Headless-testable; no GUI crate is a dependency.
//!
//! No product knowledge lives here (N15): a test fails the build if one creeps in.

pub mod addons;
pub mod config;
pub mod dynval;
pub mod engine;
pub mod eval;
pub mod form;
pub mod logging;
pub mod lua;
pub mod paths;
pub mod platform;
pub mod recent;
pub mod replace_file;
pub mod state;
pub mod suggest;
pub mod time;

//! sokol_gfx and sokol_log bindings: the generated files from `vendor/sokol-rust` (floooh/sokol-rust
//! @ b22a545), compiled against only those two C implementations (see build.rs).

#![allow(clippy::all, dead_code, non_camel_case_types, non_upper_case_globals)]

// `rustfmt::skip`: the files belong to vendor/sokol-rust; `cargo fmt` here must not rewrite them.
#[rustfmt::skip]
#[path = "../../../vendor/sokol-rust/src/gfx.rs"]
pub mod gfx;
#[rustfmt::skip]
#[path = "../../../vendor/sokol-rust/src/log.rs"]
pub mod log;

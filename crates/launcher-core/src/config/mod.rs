//! `.launcher` files: the catalog of node kinds (the one registry), the header gate, loading into
//! the typed model with diagnostics, templates, validation, and the last-working-copy cache.

pub mod cache;
pub mod catalog;
pub mod diag;
pub mod edit;
pub mod format2;
pub mod header;
pub mod load;
pub mod model;
pub mod print;
pub mod summary;
pub mod template;
pub mod tool_edit;
pub mod validate;

pub use diag::{Diagnostic, Severity};
pub use load::{load_str, LoadOptions, Loaded};
pub use model::*;

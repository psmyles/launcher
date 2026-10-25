//! Product identity, from `product.json` via build.rs (the single source).

pub const NAME: &str = env!("LAUNCHER_PRODUCT_NAME");
pub const VERSION: &str = env!("LAUNCHER_VERSION");

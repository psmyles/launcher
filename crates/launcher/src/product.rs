//! Product identity, from `product.json` via build.rs (the single source).

pub const NAME: &str = env!("LAUNCHER_PRODUCT_NAME");
pub const VERSION: &str = env!("LAUNCHER_VERSION");
/// Linux: the window's app id (Wayland) and class (X11), which match it to its `.desktop` file.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const BUNDLE_ID: &str = env!("LAUNCHER_BUNDLE_ID");

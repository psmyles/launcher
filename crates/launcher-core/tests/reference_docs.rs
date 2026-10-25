//! The generated docs stay current (N15): `docs/config-reference.md` and `docs/lua-api.md` are
//! exactly what the registries produce. After changing one, run `cargo xtask docs`.

use std::path::Path;

fn assert_current(doc: &str, generated: String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs")
        .join(doc);
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        on_disk == generated,
        "{} is out of date: run `cargo xtask docs`",
        path.display()
    );
}

#[test]
fn the_config_reference_is_current() {
    assert_current(
        "config-reference.md",
        launcher_core::config::catalog::reference_markdown(),
    );
}

#[test]
fn the_lua_api_reference_is_current() {
    assert_current("lua-api.md", launcher_core::lua::api::reference_markdown());
}

#[test]
fn the_step_reference_is_current() {
    assert_current(
        "step-reference.md",
        launcher_core::addons::reference_markdown(),
    );
}

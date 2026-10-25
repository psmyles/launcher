//! Prints every generated document, each after a `\u{1}FILE <path>` line (`cargo xtask docs` writes
//! them).

/// A marker no document contains.
const MARK: &str = "\u{1}FILE ";

fn main() {
    print!(
        "{MARK}docs/config-reference.md\n{}",
        launcher_core::config::catalog::reference_markdown()
    );
    print!(
        "{MARK}docs/step-reference.md\n{}",
        launcher_core::addons::reference_markdown()
    );
    print!(
        "{MARK}docs/lua-api.md\n{}",
        launcher_core::lua::api::reference_markdown()
    );
}

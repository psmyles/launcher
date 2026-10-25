//! The Lua checks name the right file lines (N12): a fixture of mistakes against its golden
//! output (`UPDATE_GOLDENS=1` rewrites it).

use std::path::Path;

use launcher_core::config::{load_str, LoadOptions};

#[test]
fn lint_fixtures_report_the_right_file_lines() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lint");
    let file = dir.join("mistakes.launcher");
    let text = std::fs::read_to_string(&file).unwrap();
    let l = load_str(&text, &file, &LoadOptions::default());
    assert!(!l.has_errors(), "{:?}", l.diags);
    let got: String = launcher_core::lua::lint::diagnostics(&l.config)
        .iter()
        .map(|d| d.render("mistakes.launcher") + "\n")
        .collect();
    let golden = dir.join("mistakes.txt");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::write(&golden, &got).unwrap();
    }
    let want = std::fs::read_to_string(&golden).unwrap_or_default();
    assert_eq!(
        got, want,
        "rerun with UPDATE_GOLDENS=1 after checking the output"
    );
}

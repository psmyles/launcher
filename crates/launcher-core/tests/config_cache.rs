//! The last working copy (N10): a file that breaks falls back to the copy that last loaded cleanly.
//! Its own test binary, because it points the app-data folder somewhere temporary.

use launcher_core::config::cache;
use launcher_core::config::LoadOptions;

#[test]
fn a_broken_file_falls_back_to_its_last_working_copy() {
    let root = std::env::temp_dir().join(format!("launcher-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("LAUNCHER_DATA_DIR", root.join("data"));
    let file = root.join("tool.launcher");
    let opts = LoadOptions::default();

    let good = "layout {\n    checkbox \"tests\" label=\"Run tests\"\n}\n";
    std::fs::write(&file, good).unwrap();
    let first = cache::open(&file, &opts).unwrap();
    assert!(first.fallback.is_none());
    assert!(!first.loaded.has_errors());

    // An update breaks it: the window keeps working from the copy, and says why.
    std::fs::write(&file, "layout {\n    chekbox \"tests\"\n}\n").unwrap();
    let second = cache::open(&file, &opts).unwrap();
    let fallback = second.fallback.expect("falls back");
    assert_eq!(fallback.diags[0].code, "unknown-kind");
    assert!(!second.loaded.has_errors());
    assert_eq!(second.text, good);
    assert!(second.loaded.config.control("tests").is_some());

    // Fixed again: the file itself is used (and becomes the new copy).
    let fixed = "layout {\n    checkbox \"tests\" label=\"Tests\"\n}\n";
    std::fs::write(&file, fixed).unwrap();
    let third = cache::open(&file, &opts).unwrap();
    assert!(third.fallback.is_none());
    assert_eq!(cache::recall(&file).unwrap().0, fixed);

    // A header gate is not a breakage: no fallback.
    std::fs::write(&file, "launcher requires=\"999\"\n").unwrap();
    let gated = cache::open(
        &file,
        &LoadOptions {
            runtime: launcher_core::config::Version(1, 0, 0),
            os: launcher_core::config::catalog::Os::current(),
        },
    )
    .unwrap();
    assert!(gated.fallback.is_none());
    assert!(gated.loaded.gate.is_some());

    let _ = std::fs::remove_dir_all(&root);
}

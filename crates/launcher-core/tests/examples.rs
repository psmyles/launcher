//! Every example `.launcher` file loads without a single problem, and its Lua lints cleanly, for
//! the OS its header names.

use std::path::{Path, PathBuf};

use launcher_core::config::catalog::Os;
use launcher_core::config::{self, header, LoadOptions, Version};

/// Launcher's version, from `product.json` (the one place it's set): the runtime the
/// examples are loaded by, so a `requires=` newer than the release fails here.
fn product_version() -> Version {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../product.json");
    let raw = std::fs::read_to_string(&path).expect("product.json");
    let product: serde_json::Value = serde_json::from_str(&raw).expect("product.json parses");
    Version::parse(
        product["version"]
            .as_str()
            .expect("product.json has a version"),
    )
    .unwrap()
}

fn launcher_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            launcher_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "launcher") {
            out.push(p);
        }
    }
}

#[test]
fn the_examples_have_no_problems() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    launcher_files(&root, &mut files);
    assert!(!files.is_empty());
    let mut problems = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let os = match header::sniff(&text).and_then(|h| h.os).as_deref() {
            Some("macos") => Os::Macos,
            Some("windows") => Os::Windows,
            Some("linux") => Os::Linux,
            _ => Os::current(),
        };
        let loaded = config::load_str(
            &text,
            &f,
            &LoadOptions {
                runtime: product_version(),
                os,
            },
        );
        for d in &loaded.diags {
            problems.push(d.render(&f.display().to_string()));
        }
        // And their Lua, as `launcher check` sees it.
        for d in launcher_core::lua::lint::diagnostics(&loaded.config) {
            problems.push(d.render(&f.display().to_string()));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

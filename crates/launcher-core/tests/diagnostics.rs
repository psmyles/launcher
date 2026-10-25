//! Golden-file tests for diagnostics: every `tests/fixtures/diagnostics/*.launcher` renders its
//! problems exactly as `<name>.expected` says. The messages are what people read, so a change to
//! one is deliberate: `UPDATE_GOLDENS=1 cargo test -p launcher-core --test diagnostics` rewrites
//! the expected files for review.

use std::path::{Path, PathBuf};

use launcher_core::config::catalog::Os;
use launcher_core::config::{self, LoadOptions, Version};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/diagnostics")
}

fn render(path: &Path) -> String {
    let src = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
    let loaded = config::load_str(
        &src,
        path,
        &LoadOptions {
            runtime: Version(1, 0, 0),
            os: Os::Windows,
        },
    );
    let name = path.file_name().unwrap().to_string_lossy();
    let mut out: String = loaded
        .diags
        .iter()
        .map(|d| d.render(&name) + "\n")
        .collect();
    if out.is_empty() {
        out.push_str("(no problems)\n");
    }
    out
}

#[test]
fn diagnostics_match_their_golden_files() {
    let update = std::env::var_os("UPDATE_GOLDENS").is_some();
    let mut failures = Vec::new();
    let mut count = 0;
    let mut entries: Vec<_> = std::fs::read_dir(fixtures())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "launcher"))
        .collect();
    entries.sort();
    for path in entries {
        count += 1;
        let got = render(&path);
        let golden = path.with_extension("expected");
        if update {
            std::fs::write(&golden, &got).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&golden)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if got != want {
            failures.push(format!(
                "{}\n--- expected\n{want}--- got\n{got}",
                path.display()
            ));
        }
    }
    assert!(count > 0, "no fixtures in {}", fixtures().display());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

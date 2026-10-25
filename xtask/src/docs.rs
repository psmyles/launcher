//! `cargo xtask docs`: regenerate the docs that come from the registries (N15). The test in
//! `launcher-core/tests/reference_docs.rs` fails while they are out of date (and
//! `ui::theme::tests::the_reference_doc_is_current` for `docs/theme-reference.kdl`).

use crate::util::{self, step};

pub fn run_docs(_args: &[String]) -> Result<(), String> {
    step("docs");
    // The generators live in launcher-core; a tiny example binary prints each document.
    let root = util::root();
    let out =
        std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args([
                "run",
                "--quiet",
                "-p",
                "launcher-core",
                "--example",
                "gen-docs",
            ])
            .current_dir(&root)
            .output()
            .map_err(|e| format!("could not run the docs generator: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the docs generator failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8(out.stdout).map_err(|e| e.to_string())?;
    // The generator prints `\u{1}FILE <path>` before each document (a marker no document contains).
    let mut written = 0;
    for chunk in text.split("\u{1}FILE ").filter(|c| !c.is_empty()) {
        let (path, body) = chunk.split_once('\n').ok_or("malformed generator output")?;
        let dest = root.join(path.trim());
        std::fs::write(&dest, body).map_err(|e| format!("{}: {e}", dest.display()))?;
        eprintln!("  wrote {}", dest.display());
        written += 1;
    }
    if written == 0 {
        return Err("the docs generator printed nothing".into());
    }
    // The theme reference comes from egui's Style, so its generator is a test in the app crate
    // that rewrites the file under UPDATE_GOLDENS.
    util::run(
        util::cargo()
            .args([
                "test",
                "--quiet",
                "-p",
                "launcher",
                "--bin",
                "launcher",
                "the_reference_doc_is_current",
            ])
            .env("UPDATE_GOLDENS", "1"),
    )?;
    eprintln!(
        "  wrote {}",
        root.join("docs/theme-reference.kdl").display()
    );
    Ok(())
}

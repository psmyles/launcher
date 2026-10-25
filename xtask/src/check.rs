//! `cargo xtask check`: the gate every milestone closes on. Without `--fast` it also runs the
//! end-to-end cases (`xtask e2e`) and a release build.

use std::path::Path;

use crate::util::{self, cargo, flag, run, step};

/// Words that mark a work-in-progress note; banned in shipped code (N15).
const BANNED_NOTES: &[&str] = &["TODO", "FIXME", "XXX"];

pub fn run_check(args: &[String]) -> Result<(), String> {
    step("fmt");
    run(cargo().args(["fmt", "--all", "--", "--check"]))?;

    step("clippy (warnings are errors)");
    run(cargo().args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]))?;

    // A test checks the .ico against the artwork.
    crate::icons::ico_step()?;

    step("tests");
    run(cargo().args(["test", "--workspace"]))?;

    step("no TODO/FIXME notes");
    let mut hits = Vec::new();
    for dir in ["crates", "xtask/src", "tools"] {
        scan_notes(&util::root().join(dir), &mut hits);
    }
    if !hits.is_empty() {
        for h in &hits {
            eprintln!("  {h}");
        }
        return Err(format!(
            "{} work-in-progress note(s) in shipped code",
            hits.len()
        ));
    }

    if cfg!(windows) && !flag(args, "--no-macos") {
        step("macOS type-check (aarch64-apple-darwin)");
        macos_check()?;
    }
    if cfg!(target_os = "macos") && !flag(args, "--no-windows") {
        step("Windows type-check (x86_64-pc-windows-msvc)");
        windows_check()?;
    }

    if !flag(args, "--fast") {
        crate::e2e::run_e2e(&[])?;
        util::build_release_launcher()?;
        step("size");
        let exe = util::launcher_exe();
        let bytes = std::fs::metadata(&exe)
            .map_err(|e| format!("{}: {e}", exe.display()))?
            .len();
        eprintln!("  {}: {:.2} MB", exe.display(), bytes as f64 / 1e6);
    }
    eprintln!("\nall checks passed");
    Ok(())
}

/// Type-check every crate for macOS. Build scripts that compile C/C++ get stand-in tools
/// (`fakecc`), and sokol's build script skips its C step when cross-checking.
fn macos_check() -> Result<(), String> {
    let target = "aarch64-apple-darwin";
    let tools = util::root().join("target").join("xtask-fake-tools");
    let (cc, ar) = crate::fakecc::install(&tools)?;
    let triple = target.replace('-', "_");
    run(cargo()
        .args(["check", "--workspace", "--all-targets", "--target", target])
        .env(format!("CC_{triple}"), &cc)
        .env(format!("CXX_{triple}"), &cc)
        .env(format!("AR_{triple}"), &ar)
        // No Xcode here: keep cc-rs from asking `xcrun` for an SDK.
        .env("SDKROOT", &tools)
        .env("RUSTFLAGS", "-D warnings"))
}

/// Type-check every crate for Windows, from macOS: the same stand-in tools, MSVC style.
fn windows_check() -> Result<(), String> {
    let target = "x86_64-pc-windows-msvc";
    let tools = util::root().join("target").join("xtask-fake-tools");
    let (cc, ar) = crate::fakecc::install(&tools)?;
    let triple = target.replace('-', "_");
    run(cargo()
        .args(["check", "--workspace", "--all-targets", "--target", target])
        .env(format!("CC_{triple}"), &cc)
        .env(format!("CXX_{triple}"), &cc)
        .env(format!("AR_{triple}"), &ar)
        .env("RUSTFLAGS", "-D warnings"))
}

fn scan_notes(dir: &Path, hits: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            scan_notes(&p, hits);
            continue;
        }
        if p.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            // Only comments: the ban is on notes, not on code that talks about them.
            let Some((_, comment)) = line.split_once("//") else {
                continue;
            };
            let found = BANNED_NOTES.iter().any(|w| {
                comment.match_indices(w).any(|(i, _)| {
                    let before = comment[..i].chars().next_back();
                    let after = comment[i + w.len()..].chars().next();
                    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
                    boundary(before) && boundary(after)
                })
            });
            if found {
                hits.push(format!("{}:{}: {}", p.display(), n + 1, line.trim()));
            }
        }
    }
}

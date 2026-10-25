//! A stand-in C/C++ compiler and archiver for type-checking the other OS's target (macOS from
//! Windows, Windows from macOS).
//!
//! `cargo check --target aarch64-apple-darwin` still runs build scripts, and Luau's (mlua-sys)
//! compiles C++ with the target's compiler, which doesn't exist here. `check` never links, so the
//! object files and archives only have to exist: this program, copied as `fake-cc` and `fake-ar`,
//! writes empty ones wherever it is told to.

use std::path::{Path, PathBuf};

/// When this binary runs under one of the fake names, act as that tool and return its exit code.
pub fn dispatch() -> Option<i32> {
    let exe = std::env::current_exe().ok()?;
    let stem = exe.file_stem()?.to_string_lossy().to_lowercase();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match stem.as_str() {
        "fake-cc" => Some(compile(&args)),
        "fake-ar" => Some(archive(&args)),
        _ => None,
    }
}

fn touch(path: &Path, contents: &[u8]) -> i32 {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(path, contents) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// `cc ... -o out.o` (or `-oout.o`): write an empty object. Anything else (version and
/// preprocessor probes) just succeeds.
fn compile(args: &[String]) -> i32 {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-o" {
            if let Some(out) = it.next() {
                return touch(Path::new(out), b"");
            }
        } else if let Some(out) = a.strip_prefix("-Fo").or_else(|| a.strip_prefix("/Fo")) {
            // MSVC style (cl.exe).
            return touch(Path::new(out), b"");
        } else if let Some(out) = a.strip_prefix("-o") {
            return touch(Path::new(out), b"");
        }
    }
    0
}

/// `ar <ops> lib.a objs...`: write an empty archive if there isn't one.
fn archive(args: &[String]) -> i32 {
    // MSVC style (lib.exe): `-out:lib.lib`.
    if let Some(lib) = args.iter().find_map(|a| {
        a.strip_prefix("-out:")
            .or_else(|| a.strip_prefix("/OUT:"))
            .or_else(|| a.strip_prefix("/out:"))
    }) {
        return touch(Path::new(lib), b"!<arch>\n");
    }
    match args.iter().find(|a| a.ends_with(".a")) {
        Some(lib) if !Path::new(lib).exists() => touch(Path::new(lib), b"!<arch>\n"),
        _ => 0,
    }
}

/// Copy the running xtask as `fake-cc` and `fake-ar` into `dir`, returning their paths.
pub fn install(dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let suffix = std::env::consts::EXE_SUFFIX;
    let cc = dir.join(format!("fake-cc{suffix}"));
    let ar = dir.join(format!("fake-ar{suffix}"));
    for p in [&cc, &ar] {
        std::fs::copy(&exe, p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok((cc, ar))
}

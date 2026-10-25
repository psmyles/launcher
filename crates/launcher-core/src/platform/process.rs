//! Finding programs and building Windows command lines, shared by every way the engine starts a
//! process (pseudo-terminal, pipes, capture).
//!
//! A program name without a path separator is looked up on `PATH` (with `PATHEXT` on Windows),
//! then in the given folder, which is the config's folder (N3): `find-file` gives `Game.exe` for a
//! file beside the config, and `PATH` goes first so a file there can't stand in for `git`. A name
//! with a separator resolves against that folder, never the working directory. Windows' own
//! `CreateProcessW` search would also try the launcher's folder and the current directory first.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The executable `program` names, or `None` when it can't be found.
///
/// `base` resolves a relative path (`tools/build.exe`); `path_var` is the `PATH` to search (the
/// child's, which the run context may have changed).
pub fn find_program(program: &str, base: &Path, path_var: Option<&OsString>) -> Option<PathBuf> {
    let has_separator = program.contains('/') || (cfg!(windows) && program.contains('\\'));
    if has_separator || Path::new(program).is_absolute() {
        let p = crate::paths::resolve(base, program);
        return with_extensions(&p).into_iter().find(|c| c.is_file());
    }
    let path_var = path_var
        .cloned()
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    std::env::split_paths(&path_var)
        .filter(|d| !d.as_os_str().is_empty())
        .chain(std::iter::once(base.to_path_buf()))
        .flat_map(|d| with_extensions(&d.join(program)))
        .find(|c| c.is_file())
}

/// Candidates for `p`: itself, then with each executable extension (Windows, only when `p` has
/// none).
fn with_extensions(p: &Path) -> Vec<PathBuf> {
    if !cfg!(windows) || p.extension().is_some() {
        return vec![p.to_path_buf()];
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let mut out = vec![];
    for e in exts.split(';').filter(|e| !e.is_empty()) {
        let mut s = p.as_os_str().to_os_string();
        s.push(e.to_ascii_lowercase());
        out.push(PathBuf::from(s));
    }
    out
}

/// Whether a Windows program is a batch file, which only `cmd.exe` can run.
pub fn is_batch(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("bat") || e.eq_ignore_ascii_case("cmd"))
}

/// Quote one argument the way `CommandLineToArgvW` and the MSVC runtime read it back.
pub fn quote_windows_arg(arg: &str, out: &mut String) {
    let needs = arg.is_empty() || arg.contains([' ', '\t', '\n', '\u{b}', '"']);
    if !needs {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes before a quote are doubled, and the quote is escaped.
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    // Backslashes before the closing quote are doubled.
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
}

/// A full Windows command line: the program then each argument, quoted as needed.
pub fn windows_command_line(program: &str, args: &[String]) -> String {
    let mut s = String::new();
    quote_windows_arg(program, &mut s);
    for a in args {
        s.push(' ');
        quote_windows_arg(a, &mut s);
    }
    s
}

/// A Windows environment block: `K=V\0` pairs, sorted case-insensitively by name, then `\0`.
#[cfg(windows)]
pub fn windows_env_block(env: &[(OsString, OsString)]) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut pairs: Vec<&(OsString, OsString)> = env.iter().collect();
    pairs.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (k, v) in pairs {
        block.extend(k.encode_wide());
        block.push('=' as u16);
        block.extend(v.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

/// An exit code for people: a Windows status (a crash, Ctrl+C) in hex with what it means, since
/// `-1073741510` says nothing.
pub fn exit_code_text(code: i32) -> String {
    let status = code as u32;
    if status < 0xC000_0000 {
        return code.to_string();
    }
    let meaning = match status {
        0xC000_013A => Some("interrupted with Ctrl+C"),
        0xC000_0005 => Some("access violation"),
        0xC000_00FD => Some("stack overflow"),
        0xC000_0409 => Some("stopped by a fast-fail check"),
        0xC000_0135 => Some("a DLL was not found"),
        0xC000_0139 => Some("a DLL entry point was not found"),
        0xC000_0142 => Some("a DLL failed to initialise"),
        0xC000_0094 => Some("integer division by zero"),
        0xC000_001D => Some("illegal instruction"),
        _ => None,
    };
    match meaning {
        Some(m) => format!("0x{status:08X} ({m})"),
        None => format!("0x{status:08X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_read_well() {
        assert_eq!(exit_code_text(2), "2");
        assert_eq!(exit_code_text(-1), "0xFFFFFFFF");
        assert_eq!(
            exit_code_text(-1073741510),
            "0xC000013A (interrupted with Ctrl+C)"
        );
    }

    fn q(a: &str) -> String {
        let mut s = String::new();
        quote_windows_arg(a, &mut s);
        s
    }

    #[test]
    fn quotes_like_the_msvc_runtime() {
        assert_eq!(q("plain"), "plain");
        assert_eq!(q(""), "\"\"");
        assert_eq!(q("a b"), "\"a b\"");
        assert_eq!(q(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(q(r"C:\dir with space\"), r#""C:\dir with space\\""#);
        assert_eq!(q(r"C:\no\space\"), r"C:\no\space\");
        assert_eq!(
            windows_command_line("git", &["commit".into(), "-m".into(), "a b".into()]),
            r#"git commit -m "a b""#
        );
    }

    #[test]
    fn finds_programs_on_path() {
        let found = if cfg!(windows) {
            find_program("cmd", Path::new("."), None)
        } else {
            find_program("sh", Path::new("."), None)
        };
        assert!(found.is_some());
        assert!(find_program("no-such-program-xyz", Path::new("."), None).is_none());
    }

    #[test]
    fn a_bare_name_is_found_beside_the_config_after_path() {
        let dir = std::env::temp_dir().join(format!("launcher-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let game = dir.join("Game.exe");
        std::fs::write(&game, b"").unwrap();
        assert_eq!(find_program("Game.exe", &dir, None), Some(game));
        // PATH wins over a file of the same name in the folder.
        let shadow = if cfg!(windows) { "cmd.exe" } else { "sh" };
        std::fs::write(dir.join(shadow), b"").unwrap();
        assert_ne!(
            find_program(shadow, &dir, None),
            Some(dir.join(shadow)),
            "a file beside the config replaced a program on PATH"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

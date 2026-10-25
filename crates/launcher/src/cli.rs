//! The command line. Output never panics on a closed pipe (LESSONS L2.4): shells don't wait for a
//! GUI-subsystem exe, so stdout may be gone by the time we write.

use std::io::Write as _;
use std::path::PathBuf;

use launcher_core::config::catalog::{self, Os};
use launcher_core::config::{self, header, LoadOptions, Version};

use crate::app::Start;

pub const EXIT_OK: i32 = 0;
/// `check` found errors.
pub const EXIT_ERRORS: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

pub enum Mode {
    Gui(Start),
    Version,
    Help,
    HelpConfig,
    Check { files: Vec<PathBuf> },
    Run(crate::headless::RunArgs),
}

pub fn usage() -> String {
    format!(
        "{name} {version}

Usage:
  launcher [FILE.launcher]            open a file (or the home screen)
  launcher edit FILE                  open a file in the visual editor
  launcher check FILE...              check files for problems (exit code 1 when any has errors)
  launcher run FILE [--action ID] [--set ID=VALUE]... [--preview [--answer STEP=@FILE]...]
               [--mock] [--trust]    run an action without the window
  launcher --help-config              print every node and property a .launcher file may use
  launcher --console [--pipes] PROGRAM [ARG...]
                                      run a program in a console window
  launcher --version                  print the version
  launcher --help                     print this help",
        name = crate::product::NAME,
        version = crate::product::VERSION
    )
}

pub fn parse(args: &[String]) -> Result<Mode, String> {
    let mut it = args.iter();
    let Some(first) = it.next() else {
        return Ok(Mode::Gui(Start::Home));
    };
    match first.as_str() {
        "--version" | "-V" => Ok(Mode::Version),
        "--help" | "-h" | "/?" => Ok(Mode::Help),
        "--help-config" => Ok(Mode::HelpConfig),
        "edit" | "--edit" => match (it.next(), it.next()) {
            (Some(f), None) => Ok(Mode::Gui(Start::Edit(PathBuf::from(f)))),
            _ => Err(format!("edit takes one file\n\n{}", usage())),
        },
        "check" => {
            let files: Vec<PathBuf> = it.map(PathBuf::from).collect();
            if files.is_empty() {
                return Err(format!("check needs at least one file\n\n{}", usage()));
            }
            Ok(Mode::Check { files })
        }
        "run" => {
            let rest: Vec<String> = it.cloned().collect();
            crate::headless::parse(&rest)
                .map(Mode::Run)
                .map_err(|e| format!("{e}\n\n{}", usage()))
        }
        // The theme gallery (ui/gallery.rs): a development tool, so not in the usage text.
        "--gallery" => Ok(Mode::Gui(Start::Gallery)),
        "--console" => {
            let mut it = it.peekable();
            let pipes = it.next_if(|a| *a == "--pipes").is_some();
            let program = it.next().ok_or("--console needs a program to run")?.clone();
            Ok(Mode::Gui(Start::Console {
                program,
                args: it.cloned().collect(),
                pipes,
            }))
        }
        flag if flag.starts_with("--") => Err(format!("unknown option `{flag}`\n\n{}", usage())),
        file => {
            if let Some(extra) = it.next() {
                return Err(format!("unexpected argument `{extra}`\n\n{}", usage()));
            }
            Ok(Mode::Gui(Start::Document(PathBuf::from(file))))
        }
    }
}

/// The running Launcher's version, for `requires=`.
pub fn runtime_version() -> Version {
    Version::parse(crate::product::VERSION).unwrap_or_default()
}

/// `launcher --help-config`.
pub fn help_config() -> i32 {
    out(&catalog::reference_text());
    EXIT_OK
}

/// `launcher check FILE...`: print every problem as `file:line:col: severity[code]: message`, then
/// a summary. A file is checked for the OS its header names (so a macOS file can be checked on
/// Windows), otherwise for this one. Never touches the last-working-copy cache.
pub fn check(files: &[PathBuf]) -> i32 {
    let mut failed = 0;
    let (mut errors, mut warnings) = (0, 0);
    for file in files {
        let shown = file.display().to_string();
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                err(&format!("{shown}: error: can't read the file: {e}"));
                failed += 1;
                errors += 1;
                continue;
            }
        };
        // An add-on file (N21): its own checks.
        if text.trim_start().starts_with("add-on") || file.extension().is_some_and(|x| x == "kdl") {
            let (_, diags) = launcher_core::addons::parse_addon(
                &text,
                file,
                launcher_core::addons::Origin::Folder(file.clone()),
                &LoadOptions::default(),
            );
            for d in &diags {
                out(&d.render(&shown));
            }
            let e = diags.iter().filter(|d| d.is_error()).count();
            errors += e;
            warnings += diags.len() - e;
            if e > 0 {
                failed += 1;
            }
            continue;
        }
        let os = header::sniff(&text)
            .and_then(|h| h.os)
            .and_then(|os| match os.as_str() {
                "windows" => Some(Os::Windows),
                "macos" => Some(Os::Macos),
                _ => None,
            })
            .unwrap_or(Os::current());
        let loaded = config::load_str(
            &text,
            file,
            &LoadOptions {
                runtime: runtime_version(),
                os,
            },
        );
        for d in &loaded.diags {
            out(&d.render(&shown));
        }
        errors += loaded.errors();
        warnings += loaded.warnings();
        // The Lua in a file that loaded (N12): syntax errors, lint, unknown values.
        let mut lua_errors = 0;
        if !loaded.has_errors() && loaded.gate.is_none() {
            for d in launcher_core::lua::lint::diagnostics(&loaded.config) {
                out(&d.render(&shown));
                if d.is_error() {
                    lua_errors += 1;
                } else {
                    warnings += 1;
                }
            }
        }
        errors += lua_errors;
        if loaded.has_errors() || lua_errors > 0 {
            failed += 1;
        }
    }
    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    let checked = plural(files.len(), "file");
    if errors == 0 && warnings == 0 {
        out(&format!("{checked} checked: no problems"));
    } else {
        out(&format!(
            "{checked} checked: {}, {}",
            plural(errors, "error"),
            plural(warnings, "warning")
        ));
    }
    if failed > 0 {
        EXIT_ERRORS
    } else {
        EXIT_OK
    }
}

/// Print to stdout, ignoring a closed pipe.
pub fn out(text: &str) {
    let mut o = std::io::stdout();
    let _ = writeln!(o, "{text}");
    let _ = o.flush();
}

/// Print to stderr, ignoring a closed pipe.
pub fn err(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_modes() {
        assert!(matches!(parse(&[]), Ok(Mode::Gui(Start::Home))));
        assert!(matches!(parse(&args(&["--version"])), Ok(Mode::Version)));
        assert!(matches!(
            parse(&args(&["--help-config"])),
            Ok(Mode::HelpConfig)
        ));
        assert!(matches!(
            parse(&args(&["check", "a.launcher", "b.launcher"])),
            Ok(Mode::Check { files }) if files.len() == 2
        ));
        assert!(parse(&args(&["check"])).is_err());
        assert!(matches!(
            parse(&args(&["x.launcher"])),
            Ok(Mode::Gui(Start::Document(_)))
        ));
        match parse(&args(&["--console", "git", "clone", "u"])) {
            Ok(Mode::Gui(Start::Console {
                program,
                args,
                pipes: false,
            })) => {
                assert_eq!(program, "git");
                assert_eq!(args, ["clone", "u"]);
            }
            _ => panic!("expected the console mode"),
        }
        assert!(matches!(
            parse(&args(&["--console", "--pipes", "cmd"])),
            Ok(Mode::Gui(Start::Console { pipes: true, .. }))
        ));
        assert!(matches!(
            parse(&args(&["--gallery"])),
            Ok(Mode::Gui(Start::Gallery))
        ));
        assert!(parse(&args(&["--nope"])).is_err());
        assert!(parse(&args(&["a", "b"])).is_err());
    }
}

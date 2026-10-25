//! Shared helpers: the workspace root, running commands with an echo, and flag parsing.

use std::path::PathBuf;
use std::process::Command;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}

/// Run a command from the workspace root, echoing it; an error names the command.
pub fn run(cmd: &mut Command) -> Result<(), String> {
    cmd.current_dir(root());
    let shown = format!("{cmd:?}").replace('"', "");
    eprintln!("  $ {shown}");
    let status = cmd
        .status()
        .map_err(|e| format!("could not start `{shown}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{shown}` failed ({status})"))
    }
}

pub fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// A section header in the output.
pub fn step(name: &str) {
    eprintln!("\n== {name}");
}

/// `--name value` from a flag list.
pub fn value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

pub fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// The release launcher binary.
pub fn launcher_exe() -> PathBuf {
    root()
        .join("target")
        .join("release")
        .join(format!("launcher{}", std::env::consts::EXE_SUFFIX))
}

/// The release build, after redrawing the .ico if the artwork changed.
pub fn build_release_launcher() -> Result<(), String> {
    crate::icons::ico_step()?;
    step("release build");
    run(cargo().args(["build", "--release", "-p", "launcher"]))
}

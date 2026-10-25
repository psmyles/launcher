//! `cargo xtask register` / `unregister`: make double-clicking a `.launcher` file open this
//! checkout's release build, for the current user only (`HKCU\Software\Classes`), until the
//! installer (M10) does it for real with the same ProgID. Windows only.

use std::process::Command;

use crate::util;

/// The file type's ProgID; the installer uses the same one.
pub const PROGID: &str = "Launcher.File";

fn reg(args: &[&str]) -> Result<(), String> {
    util::run(Command::new("reg").args(args))
}

pub fn run_register(args: &[String]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("register is for Windows (macOS registers through the app bundle)".into());
    }
    if !args.iter().any(|a| a == "--no-build") {
        util::build_release_launcher()?;
    }
    let exe = util::root()
        .join("target")
        .join("release")
        .join("launcher.exe");
    if !exe.is_file() {
        return Err(format!("{} is missing; build it first", exe.display()));
    }
    let exe = exe.display().to_string();
    let classes = r"HKCU\Software\Classes";
    let ext = format!(r"{classes}\.launcher");
    let prog = format!(r"{classes}\{PROGID}");
    reg(&["add", &ext, "/ve", "/d", PROGID, "/f"])?;
    reg(&["add", &prog, "/ve", "/d", "Launcher file", "/f"])?;
    reg(&[
        "add",
        &format!(r"{prog}\DefaultIcon"),
        "/ve",
        "/d",
        &format!("\"{exe}\",0"),
        "/f",
    ])?;
    reg(&[
        "add",
        &format!(r"{prog}\shell\open\command"),
        "/ve",
        "/d",
        &format!("\"{exe}\" \"%1\""),
        "/f",
    ])?;
    println!(
        ".launcher files now open with {exe} (for this user; `cargo xtask unregister` undoes it)"
    );
    Ok(())
}

pub fn run_unregister(_args: &[String]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("unregister is for Windows".into());
    }
    let classes = r"HKCU\Software\Classes";
    for key in [
        format!(r"{classes}\.launcher"),
        format!(r"{classes}\{PROGID}"),
    ] {
        // A key that isn't there is fine.
        let _ = Command::new("reg").args(["delete", &key, "/f"]).output();
    }
    println!(".launcher files are no longer registered for this user");
    Ok(())
}

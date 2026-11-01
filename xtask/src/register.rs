//! `cargo xtask register` / `unregister`: make double-clicking a `.launcher` file open this
//! checkout's release build, for the current user only, until an installer does it for real.
//!
//! * Windows: `HKCU\Software\Classes`, with the installer's ProgID (M10).
//! * Linux: the build is copied to `~/.local/bin` (so `cargo clean` doesn't break it), with a
//!   `.desktop` entry named by product.json's `bundleId` (the window's app id matches it), the
//!   `application/x-launcher` file type, and both icons drawn from the artwork, all under
//!   `$XDG_DATA_HOME`. Run it again after a rebuild to update the copy.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util;

/// The file type's ProgID; the installer uses the same one.
pub const PROGID: &str = "Launcher.File";

fn reg(args: &[&str]) -> Result<(), String> {
    util::run(Command::new("reg").args(args))
}

pub fn run_register(args: &[String]) -> Result<(), String> {
    if cfg!(target_os = "linux") {
        return linux::register(args);
    }
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
    if cfg!(target_os = "linux") {
        return linux::unregister();
    }
    if !cfg!(windows) {
        return Err("unregister is for Windows and Linux".into());
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

mod linux {
    use super::*;

    /// The file type, and its icon's name in the icon theme.
    const MIME: &str = "application/x-launcher";
    const MIME_ICON: &str = "application-x-launcher";
    /// The sizes the hicolor theme looks for.
    const SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256, 512];

    struct Places {
        exe: PathBuf,
        desktop: PathBuf,
        mime_xml: PathBuf,
        data: PathBuf,
        bundle_id: String,
    }

    fn places() -> Result<Places, String> {
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"));
        let bundle_id = crate::package::product("bundleId")?;
        if bundle_id.is_empty() {
            return Err("product.json needs \"bundleId\": it names the .desktop entry".into());
        }
        Ok(Places {
            exe: home.join(".local/bin/launcher"),
            desktop: data.join(format!("applications/{bundle_id}.desktop")),
            mime_xml: data.join(format!("mime/packages/{bundle_id}.xml")),
            data,
            bundle_id,
        })
    }

    fn icon(data: &Path, size: u32, kind: &str, name: &str) -> PathBuf {
        data.join(format!("icons/hicolor/{size}x{size}/{kind}/{name}.png"))
    }

    fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Tell the desktop about the change. Each tool is optional (KDE, GNOME and others ship
    /// different ones), so a missing one is only reported.
    fn refresh(p: &Places) {
        let mime = p.data.join("mime");
        let apps = p.data.join("applications");
        let hicolor = p.data.join("icons/hicolor");
        let mut tools: Vec<Command> = Vec::new();
        let mut c = Command::new("update-mime-database");
        c.arg(&mime);
        tools.push(c);
        let mut c = Command::new("update-desktop-database");
        c.arg(&apps);
        tools.push(c);
        if hicolor.join("index.theme").is_file() {
            let mut c = Command::new("gtk-update-icon-cache");
            c.args(["-f", "-t"]).arg(&hicolor);
            tools.push(c);
        }
        tools.push(Command::new("kbuildsycoca6"));
        for mut t in tools {
            let name = t.get_program().to_string_lossy().into_owned();
            match t.output() {
                Ok(o) if o.status.success() => {}
                Ok(o) => eprintln!(
                    "  {name} failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
                Err(_) => eprintln!("  {name} not found; skipped"),
            }
        }
    }

    pub fn register(args: &[String]) -> Result<(), String> {
        if !args.iter().any(|a| a == "--no-build") {
            util::build_release_launcher()?;
        }
        let built = util::launcher_exe();
        if !built.is_file() {
            return Err(format!("{} is missing; build it first", built.display()));
        }
        let p = places()?;
        util::step("install");

        // Copy, then rename over the old one: a running copy keeps its file.
        if let Some(dir) = p.exe.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let part = p.exe.with_extension("part");
        std::fs::copy(&built, &part).map_err(|e| format!("{}: {e}", part.display()))?;
        std::fs::rename(&part, &p.exe).map_err(|e| format!("{}: {e}", p.exe.display()))?;
        eprintln!("  {}", p.exe.display());

        let art = crate::icons::Artwork::load()?;
        for &size in SIZES {
            let png = crate::icons::png(&art.draw(size, size, size, None));
            write(&icon(&p.data, size, "apps", &p.bundle_id), &png)?;
            write(&icon(&p.data, size, "mimetypes", MIME_ICON), &png)?;
        }
        eprintln!("  icons in {}", p.data.join("icons/hicolor").display());

        let id = &p.bundle_id;
        let desktop = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Launcher\n\
             Comment={comment}\n\
             Exec=\"{exe}\" %f\n\
             Icon={id}\n\
             Terminal=false\n\
             Categories=Development;\n\
             MimeType={MIME};\n\
             StartupWMClass={id}\n",
            comment = crate::package::product("description")?,
            exe = p.exe.display(),
        );
        write(&p.desktop, desktop.as_bytes())?;
        eprintln!("  {}", p.desktop.display());

        // A subclass of plain text, so text editors still offer to open it.
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="{MIME}">
    <comment>Launcher tool</comment>
    <sub-class-of type="text/plain"/>
    <icon name="{MIME_ICON}"/>
    <glob pattern="*.launcher"/>
  </mime-type>
</mime-info>
"#
        );
        write(&p.mime_xml, xml.as_bytes())?;
        eprintln!("  {}", p.mime_xml.display());

        refresh(&p);
        util::run(Command::new("xdg-mime").args(["default", &format!("{id}.desktop"), MIME]))?;
        println!(
            ".launcher files now open with {} (for this user; `cargo xtask unregister` undoes it)",
            p.exe.display()
        );
        Ok(())
    }

    pub fn unregister() -> Result<(), String> {
        let p = places()?;
        let mut files = vec![p.exe.clone(), p.desktop.clone(), p.mime_xml.clone()];
        for &size in SIZES {
            files.push(icon(&p.data, size, "apps", &p.bundle_id));
            files.push(icon(&p.data, size, "mimetypes", MIME_ICON));
        }
        for f in files {
            // A file that isn't there is fine.
            let _ = std::fs::remove_file(f);
        }
        refresh(&p);
        println!(".launcher files are no longer registered for this user");
        Ok(())
    }
}

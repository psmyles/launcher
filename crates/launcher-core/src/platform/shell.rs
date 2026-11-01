//! Opening things the way the OS does: `open` (a file, folder or URL with its default program)
//! and `reveal` (select a file in Explorer, Finder, or the Linux desktop's file manager).

#[cfg(windows)]
pub fn open(target: &str) -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    // SAFETY: HSTRINGs outlive the call; ShellExecuteW takes no ownership.
    let r = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(target),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value greater than 32 on success.
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "Windows could not open {target} (error {})",
            r.0 as isize
        ))
    }
}

#[cfg(target_os = "macos")]
pub fn open(target: &str) -> Result<(), String> {
    run_open(&[target])
}

#[cfg(windows)]
pub fn reveal(path: &std::path::Path) -> Result<(), String> {
    // `explorer /select,"path"` opens the folder with the item selected. Explorer's exit code is
    // meaningless (1 on success), so only a failure to start counts.
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start Explorer: {e}"))
}

#[cfg(target_os = "macos")]
pub fn reveal(path: &std::path::Path) -> Result<(), String> {
    run_open(&["-R", &path.display().to_string()])
}

#[cfg(target_os = "linux")]
pub fn open(target: &str) -> Result<(), String> {
    let s = std::process::Command::new("xdg-open")
        .arg(target)
        .status()
        .map_err(|e| format!("could not run xdg-open: {e}"))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("xdg-open {target} failed"))
    }
}

/// Linux: the file manager's ShowItems over D-Bus selects the item; without one, open the folder.
#[cfg(target_os = "linux")]
pub fn reveal(path: &std::path::Path) -> Result<(), String> {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let shown = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("array:string:{}", file_uri(&path)))
        .arg("string:")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if shown {
        return Ok(());
    }
    let folder = if path.is_dir() {
        path.as_path()
    } else {
        path.parent().unwrap_or(&path)
    };
    open(&folder.display().to_string())
}

/// `file://` and the path, percent-encoded outside the characters a URI path allows as is.
#[cfg(target_os = "linux")]
fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(target_os = "macos")]
fn run_open(args: &[&str]) -> Result<(), String> {
    let s = std::process::Command::new("/usr/bin/open")
        .args(args)
        .status()
        .map_err(|e| format!("could not run open: {e}"))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("open {} failed", args.join(" ")))
    }
}

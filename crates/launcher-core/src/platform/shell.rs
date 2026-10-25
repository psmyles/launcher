//! Opening things the way the OS does: `open` (a file, folder or URL with its default program)
//! and `reveal` (select a file in Explorer or Finder).

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

//! Moving a file or folder to the Recycle Bin (Windows) or the Trash (macOS), so a Delete step
//! can be undone (N33). Ok(false) when the path isn't there.

use std::path::Path;

#[cfg(windows)]
pub fn trash(path: &Path) -> Result<bool, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };
    if !path.exists() {
        return Ok(false);
    }
    let full = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // The shell wants a plain path (no \\?\ prefix), ending in two NULs.
    let shown = full.display().to_string();
    let plain = shown.strip_prefix(r"\\?\").unwrap_or(&shown);
    let mut wide: Vec<u16> = std::ffi::OsStr::new(plain).encode_wide().collect();
    wide.push(0);
    wide.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(wide.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    // SAFETY: `op` points at `wide`, which outlives the call.
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(format!(
            "couldn't move {} to the Recycle Bin (error {r})",
            path.display()
        ));
    }
    Ok(true)
}

#[cfg(not(windows))]
pub fn trash(path: &Path) -> Result<bool, String> {
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(false);
    }
    let home = std::env::var_os("HOME").ok_or("there's no home folder for the Trash")?;
    let bin = Path::new(&home).join(".Trash");
    std::fs::create_dir_all(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "item".into());
    // Never overwrite something already in the Trash: "name 2", "name 3"...
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut dest = bin.join(&name);
    let mut n = 2;
    while std::fs::symlink_metadata(&dest).is_ok() {
        dest = bin.join(format!("{stem} {n}{ext}"));
        n += 1;
    }
    std::fs::rename(path, &dest).map_err(|e| {
        format!(
            "couldn't move {} to the Trash ({e}); it may be on another drive",
            path.display()
        )
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_missing_path_is_not_an_error() {
        let p = std::env::temp_dir().join("launcher-trash-never-there");
        assert_eq!(super::trash(&p), Ok(false));
    }
}

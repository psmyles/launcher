//! The trust gate (N9): a `.launcher` file runs programs, so one that came from the internet needs
//! the user's OK before any of its code is evaluated, even the code that fills a dropdown.
//!
//! "Came from the internet" is the OS's own mark:
//! * Windows: the Mark of the Web, a `Zone.Identifier` stream with `ZoneId=3` (Internet) or `4`
//!   (Restricted) that browsers and mail clients add to downloads.
//! * macOS: the `com.apple.quarantine` extended attribute.
//!
//! Files from git carry neither, so a team's tools never ask. Trust is remembered in
//! `<app data>/trust.kdl` per file and content: when the file changes, it asks again.

use std::fmt::Write as _;
use std::path::Path;

use crate::paths;

/// Why a file needs trusting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Windows: downloaded (the zone: 3 Internet, 4 Restricted).
    Web { zone: u32 },
    /// macOS: quarantined.
    Quarantine,
}

impl Mark {
    pub fn describe(&self) -> &'static str {
        match self {
            Mark::Web { .. } => "was downloaded from the internet",
            Mark::Quarantine => "was downloaded (macOS has quarantined it)",
        }
    }
}

/// The file's download mark, if it has one that calls for trust.
pub fn mark(path: &Path) -> Option<Mark> {
    #[cfg(windows)]
    {
        let stream = format!("{}:Zone.Identifier", path.display());
        let text = std::fs::read_to_string(stream).ok()?;
        let zone = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("ZoneId="))
            .and_then(|z| z.trim().parse::<u32>().ok())?;
        (zone >= 3).then_some(Mark::Web { zone })
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: a size query (null buffer) on a NUL-terminated path and attribute name.
        let n = unsafe {
            libc::getxattr(
                c_path.as_ptr(),
                c"com.apple.quarantine".as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                0,
            )
        };
        (n >= 0).then_some(Mark::Quarantine)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = path;
        None
    }
}

fn content_hash(text: &str) -> String {
    format!("{:016x}", paths::fnv1a64(text.as_bytes()))
}

fn store_path() -> Option<std::path::PathBuf> {
    Some(paths::app_data_dir()?.join("trust.kdl"))
}

/// The trusted (file key, content hash) pairs.
fn load_store() -> Vec<(String, String)> {
    let Some(text) = store_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    let Ok(doc) = kdl::KdlDocument::parse_v2(&text) else {
        return Vec::new();
    };
    doc.nodes()
        .iter()
        .filter(|n| n.name().value() == "trusted")
        .filter_map(|n| {
            let key = n.entries().first()?.value().as_string()?.to_string();
            let hash = n.get("hash")?.as_string()?.to_string();
            Some((key, hash))
        })
        .collect()
}

/// Whether the file (with this content) needs the user's OK first: it has a download mark and
/// hasn't been trusted as it is now.
pub fn needs_trust(path: &Path, text: &str) -> Option<Mark> {
    let m = mark(path)?;
    let key = paths::config_key(path);
    let hash = content_hash(text);
    let trusted = load_store().iter().any(|(k, h)| *k == key && *h == hash);
    (!trusted).then_some(m)
}

/// Proof that a file may be evaluated: nothing in the engine evaluates a file's code without one
/// (`eval::Evaluator::new` takes it).
#[derive(Clone, Copy, Debug)]
pub struct Trusted(());

/// A file may be evaluated when it has no download mark, or the user trusted it as it is now.
pub fn check(path: &Path, text: &str) -> Result<Trusted, Mark> {
    match needs_trust(path, text) {
        None => Ok(Trusted(())),
        Some(m) => Err(m),
    }
}

/// The user said yes: remember it and allow evaluation.
pub fn grant(path: &Path, text: &str) -> Result<Trusted, String> {
    trust(path, text)?;
    Ok(Trusted(()))
}

/// Trust without asking: `launcher check` and headless runs the user started on purpose from a
/// terminal, and tests.
pub fn assume() -> Trusted {
    Trusted(())
}

/// Remember that the user trusts this file as it is now.
pub fn trust(path: &Path, text: &str) -> Result<(), String> {
    let store = store_path().ok_or("no app data folder")?;
    let key = paths::config_key(path);
    let mut entries: Vec<(String, String)> = load_store()
        .into_iter()
        .filter(|(k, _)| *k != key)
        .collect();
    entries.push((key, content_hash(text)));
    let mut out = String::from(
        "// Files trusted despite a download mark (safe to delete: they will ask again)\n",
    );
    for (k, h) in entries {
        let _ = writeln!(
            out,
            "trusted {} hash={}",
            crate::state::quote(&k),
            crate::state::quote(&h)
        );
    }
    if let Some(dir) = store.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crate::replace_file::write_bytes_replacing(&store, out)
        .map_err(|e| format!("can't save {}: {e}", store.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unmarked_file_needs_no_trust() {
        let dir = std::env::temp_dir().join(format!("launcher-trust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("plain.launcher");
        std::fs::write(&f, "layout {}").unwrap();
        assert_eq!(mark(&f), None);
        assert_eq!(needs_trust(&f, "layout {}"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_mark_of_the_web() {
        let dir = std::env::temp_dir().join(format!("launcher-motw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("downloaded.launcher");
        std::fs::write(&f, "layout {}").unwrap();
        std::fs::write(
            format!("{}:Zone.Identifier", f.display()),
            "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/x.launcher\r\n",
        )
        .unwrap();
        assert_eq!(mark(&f), Some(Mark::Web { zone: 3 }));
        // Intranet (1) and trusted sites (2) don't call for a prompt.
        std::fs::write(
            format!("{}:Zone.Identifier", f.display()),
            "[ZoneTransfer]\r\nZoneId=1\r\n",
        )
        .unwrap();
        assert_eq!(mark(&f), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

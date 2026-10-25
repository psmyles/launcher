//! Writing a file by replacing it, so a failed write never destroys what was
//! already there.
//!
//! ## Why this exists
//!
//! Copied from 3D Review (`crates/optimize/src/replace_file.rs`). `fopen(…, "wb")`
//! truncates *before* the first byte is written, so a disk that fills up, a process
//! that is killed, or a writer that errors halfway leaves a fragment. Here that
//! would be the user's launcher.kdl (saved by the editor), their state file, or a
//! config.json a game reads.
//!
//! ## The shape
//!
//! [`write_replacing`] hands the writer a temporary sibling, and only if that
//! writer returns `Ok` does it rename the temp over the destination. A rename
//! within one directory is the closest thing either OS gives us to an atomic
//! swap, and `std::fs::rename` replaces an existing file on Windows as well as
//! Unix. On any failure the temp is removed and the destination is untouched,
//! bit for bit.
//!
//! The temp is a *sibling*, not a file in the system temp directory: a rename
//! across volumes is a copy, which is not atomic.
//!
//! This is deliberately not a `tempfile` dependency — the whole mechanism is
//! twenty lines of `std`, and the crate's other file work is `std` too.

use std::io;
use std::path::{Path, PathBuf};

/// Where a replacement is staged before it takes the destination's place.
///
/// The process id keeps two launchers off each other's temp even when they target
/// one path.
fn staging_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".partial-{}", std::process::id()));
    path.with_file_name(name)
}

/// Run `write` against a temporary sibling of `path`, then put its result in
/// `path`'s place.
///
/// The destination is replaced only when `write` returns `Ok` **and** the rename
/// succeeds; on any other outcome the temp is removed and `path` is left exactly
/// as it was. The writer must have closed its handle by the time it returns —
/// Windows refuses to rename a file that is still open.
pub fn write_replacing<E>(path: &Path, write: impl FnOnce(&Path) -> Result<(), E>) -> Result<(), E>
where
    E: From<io::Error>,
{
    let staging = staging_path(path);
    // A temp left behind by a killed process would otherwise be appended to or
    // refuse to open.
    let _ = std::fs::remove_file(&staging);

    match write(&staging) {
        Ok(()) => match std::fs::rename(&staging, path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&staging);
                Err(E::from(error))
            }
        },
        Err(error) => {
            let _ = std::fs::remove_file(&staging);
            Err(error)
        }
    }
}

/// [`write_replacing`] for the common case of writing bytes in one call.
pub fn write_bytes_replacing(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write_replacing(path, |staging| std::fs::write(staging, contents.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A per-test directory, removed and recreated so a rerun never sees the
    /// previous run's files. (`CARGO_TARGET_TMPDIR` is only set for integration
    /// tests, so a lib test uses the system temp directory.)
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("launcher-replace-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the temp directory");
        dir
    }

    /// No temp may survive a successful write, or the export directory fills
    /// with `.partial-*` files nobody deletes.
    fn assert_no_staging(dir: &Path) {
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .expect("read the directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".partial-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn a_successful_write_replaces_the_destination() {
        let dir = temp_dir("replace-success");
        let path = dir.join("asset.fbx");
        std::fs::write(&path, b"old").expect("seed");

        write_bytes_replacing(&path, b"new").expect("writes");

        assert_eq!(std::fs::read(&path).expect("read"), b"new");
        assert_no_staging(&dir);
    }

    #[test]
    fn a_failed_write_leaves_the_existing_file_untouched() {
        let dir = temp_dir("replace-failure");
        let path = dir.join("asset.fbx");
        std::fs::write(&path, b"the good export").expect("seed");

        let result: Result<(), io::Error> = write_replacing(&path, |staging| {
            // A writer that produces a partial file and *then* fails — the case
            // a direct write turns into a truncated destination.
            std::fs::write(staging, b"half a fi")?;
            Err(io::Error::other("disk full"))
        });

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&path).expect("read"),
            b"the good export",
            "the previous export must survive a failed replacement"
        );
        assert_no_staging(&dir);
    }

    #[test]
    fn writing_a_new_file_needs_no_existing_destination() {
        let dir = temp_dir("replace-new");
        let path = dir.join("fresh.fbx");

        write_bytes_replacing(&path, b"content").expect("writes");

        assert_eq!(std::fs::read(&path).expect("read"), b"content");
    }
}

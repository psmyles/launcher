//! Run locks (N11): one run at a time per `.launcher` file, through an OS file lock in app data
//! keyed by the file's resolved path; and an action's `lock="<path>"`, which guards a shared
//! resource across different files. The OS releases a lock when its process dies, so a crash never
//! leaves one behind.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::paths;

/// Held for the length of a run; dropping it releases the lock.
pub struct RunLock {
    _file: File,
    pub path: PathBuf,
}

#[derive(Debug, PartialEq)]
pub enum LockError {
    /// Another run holds it.
    Busy,
    Io(String),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy => f.write_str("another run holds the lock"),
            LockError::Io(e) => f.write_str(e),
        }
    }
}

fn take(path: &Path) -> Result<RunLock, LockError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| LockError::Io(format!("{}: {e}", dir.display())))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| LockError::Io(format!("{}: {e}", path.display())))?;
    match file.try_lock() {
        Ok(()) => Ok(RunLock {
            _file: file,
            path: path.to_path_buf(),
        }),
        Err(std::fs::TryLockError::WouldBlock) => Err(LockError::Busy),
        Err(std::fs::TryLockError::Error(e)) => {
            Err(LockError::Io(format!("{}: {e}", path.display())))
        }
    }
}

fn locks_dir() -> Result<PathBuf, LockError> {
    paths::app_data_dir()
        .map(|d| d.join("locks"))
        .ok_or_else(|| LockError::Io("no app data folder".into()))
}

/// The per-file lock: `locks/<hash>.lock`.
pub fn lock_file(config: &Path) -> Result<RunLock, LockError> {
    take(&locks_dir()?.join(format!("{}.lock", paths::config_hash(config))))
}

/// An action's `lock=`: keyed by the resolved path it names, so every file naming the same path
/// shares it.
pub fn lock_named(path: &Path) -> Result<RunLock, LockError> {
    take(&locks_dir()?.join(format!("named-{}.lock", paths::config_hash(path))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_runs_once_at_a_time() {
        let dir = std::env::temp_dir().join(format!("launcher-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = take(&dir.join("a.lock")).unwrap();
        assert_eq!(take(&dir.join("a.lock")).err(), Some(LockError::Busy));
        // A different file isn't blocked.
        let _b = take(&dir.join("b.lock")).unwrap();
        drop(a);
        assert!(take(&dir.join("a.lock")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Watching a `.launcher` file for changes, so the window reloads it (M2; reloads wait while a run
//! is going, N11).
//!
//! * The file's **folder** is watched, not the file: editors save by renaming a temp file over it,
//!   which ends a watch on the file itself (LESSONS L2.8).
//! * Events are debounced: the deadline is set by the first matching event and checked after every
//!   wake-up, so a stream of unrelated events in the same folder can't starve it (L2.8).
//! * One check is queued right after starting: the file may have been saved between reading it and
//!   the watcher existing (L2.12). The callback compares content, so a spurious call is harmless.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

const DEBOUNCE: Duration = Duration::from_millis(250);

pub struct FileWatcher {
    // Dropping the watcher ends the watch (and the debounce thread with it).
    _watcher: notify::RecommendedWatcher,
}

fn same_name(a: &Path, b: &str) -> bool {
    let Some(n) = a.file_name().map(|n| n.to_string_lossy()) else {
        return false;
    };
    if cfg!(windows) {
        n.eq_ignore_ascii_case(b)
    } else {
        n == b
    }
}

impl FileWatcher {
    /// Call `changed` (on a background thread) whenever `file` may have changed.
    pub fn start(file: &Path, changed: impl Fn() + Send + 'static) -> Result<FileWatcher, String> {
        let full: PathBuf = crate::paths::absolute(file);
        let dir = full
            .parent()
            .ok_or_else(|| format!("{} has no folder", full.display()))?
            .to_path_buf();
        let name = full
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} has no file name", full.display()))?;
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(tx)
            .map_err(|e| format!("can't watch {}: {e}", dir.display()))?;
        watcher
            .watch(&dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("can't watch {}: {e}", dir.display()))?;
        std::thread::Builder::new()
            .name("file-watch".into())
            .spawn(move || {
                // The queued first check.
                let mut deadline = Some(Instant::now());
                loop {
                    let wait = deadline
                        .map(|d| d.saturating_duration_since(Instant::now()))
                        .unwrap_or(Duration::from_secs(3600));
                    match rx.recv_timeout(wait) {
                        Ok(Ok(event)) => {
                            if event.paths.iter().any(|p| same_name(p, &name)) {
                                deadline.get_or_insert(Instant::now() + DEBOUNCE);
                            }
                        }
                        Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    if deadline.is_some_and(|d| d <= Instant::now()) {
                        deadline = None;
                        changed();
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(FileWatcher { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn reports_a_save_by_rename() {
        let dir = std::env::temp_dir().join(format!("launcher-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.launcher");
        std::fs::write(&file, "one").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _w = FileWatcher::start(&file, move || {
            c.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        // The queued first check.
        let wait_for = |n: usize| {
            let start = Instant::now();
            while calls.load(Ordering::SeqCst) < n && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(20));
            }
            calls.load(Ordering::SeqCst)
        };
        assert_eq!(wait_for(1), 1);
        // An editor's save: write a temp file, rename it over the original.
        let tmp = dir.join("a.launcher.tmp");
        std::fs::write(&tmp, "two").unwrap();
        std::fs::rename(&tmp, &file).unwrap();
        assert!(wait_for(2) >= 2);
        // Unrelated files in the folder don't count.
        let before = calls.load(Ordering::SeqCst);
        std::fs::write(dir.join("other.txt"), "x").unwrap();
        std::thread::sleep(DEBOUNCE * 3);
        assert_eq!(calls.load(Ordering::SeqCst), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

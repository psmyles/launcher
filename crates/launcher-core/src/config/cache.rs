//! The last working copy (N10): every time a file loads without errors its text is kept in
//! `<app data>/cache/<hash>.launcher`. When a later version of the file has errors (an update
//! pulled from version control that breaks it), the window uses the cached copy instead, with a
//! banner saying so, rather than refusing to work.
//!
//! A header gate (a newer Launcher needed, the other OS) is not a breakage and never falls back.
//! `launcher check` never reads or writes the cache: it checks the file itself.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::diag::Diagnostic;
use super::load::{load_str, LoadOptions, Loaded};
use crate::paths;

pub fn cache_path(config: &Path) -> Option<PathBuf> {
    Some(
        paths::app_data_dir()?
            .join("cache")
            .join(format!("{}.launcher", paths::config_hash(config))),
    )
}

/// Keep `text` as the file's last working copy (only when it changed).
pub fn remember(config: &Path, text: &str) {
    let Some(path) = cache_path(config) else {
        return;
    };
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = crate::replace_file::write_bytes_replacing(&path, text) {
        crate::log_warn!("could not keep a copy of {}: {e}", config.display());
    }
}

/// The last working copy and when it was saved.
pub fn recall(config: &Path) -> Option<(String, SystemTime)> {
    let path = cache_path(config)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let when = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Some((text, when))
}

/// Using the cached copy because the file itself has errors.
pub struct Fallback {
    /// When the cached copy was saved.
    pub saved: SystemTime,
    /// What is wrong with the file on disk.
    pub diags: Vec<Diagnostic>,
}

pub struct Opened {
    pub loaded: Loaded,
    /// The text that was loaded (the file's, or the cached copy's).
    pub text: String,
    pub fallback: Option<Fallback>,
}

/// Read and load a file for the window, falling back to its last working copy when it is broken.
pub fn open(path: &Path, opts: &LoadOptions) -> Result<Opened, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let loaded = load_str(&text, path, opts);
    if !loaded.has_errors() {
        remember(path, &text);
        return Ok(Opened {
            loaded,
            text,
            fallback: None,
        });
    }
    if loaded.gate.is_none() {
        if let Some((cached, saved)) = recall(path) {
            let old = load_str(&cached, path, opts);
            if !old.has_errors() {
                crate::log_warn!("{} has errors; using its last working copy", path.display());
                return Ok(Opened {
                    loaded: old,
                    text: cached,
                    fallback: Some(Fallback {
                        saved,
                        diags: loaded.diags,
                    }),
                });
            }
        }
    }
    Ok(Opened {
        loaded,
        text,
        fallback: None,
    })
}

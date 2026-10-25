//! Run history (M4), for a file that saves logs (`save-logs`): `<app data>/runs/<hash>/<timestamp>/`
//! holds `steps.json` (what ran and how it ended) and one log per step with everything the step
//! printed. The last 20 runs of a file are kept. The window uses the last run of an action for
//! "last time: 12 min".

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::paths;

const KEEP: usize = 20;
/// How much of a step's output [`StepLog`] keeps in memory.
const TAIL: usize = 16 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepRecord {
    pub label: String,
    /// "ok", "failed", "warned", "skipped", "cancelled", "not run".
    pub outcome: String,
    pub duration_ms: u64,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunRecord {
    pub action: String,
    /// `YYYY-MM-DD HH:MM:SS`, local time.
    pub started: String,
    pub duration_ms: u64,
    /// "ok", "failed", "cancelled".
    pub outcome: String,
    pub steps: Vec<StepRecord>,
}

impl RunRecord {
    fn to_json(&self) -> Value {
        json!({
            "action": self.action,
            "started": self.started,
            "duration_ms": self.duration_ms,
            "outcome": self.outcome,
            "steps": self.steps.iter().map(|s| json!({
                "label": s.label,
                "outcome": s.outcome,
                "duration_ms": s.duration_ms,
                "message": s.message,
            })).collect::<Vec<_>>(),
        })
    }

    fn from_json(v: &Value) -> Option<RunRecord> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(RunRecord {
            action: s("action")?,
            started: s("started")?,
            duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
            outcome: s("outcome")?,
            steps: v
                .get("steps")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|st| StepRecord {
                            label: st.get("label").and_then(Value::as_str).unwrap_or("").into(),
                            outcome: st
                                .get("outcome")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .into(),
                            duration_ms: st.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
                            message: st
                                .get("message")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// A file's run history.
pub struct History {
    dir: PathBuf,
}

/// One run's folder.
pub struct RunDir {
    pub path: PathBuf,
}

impl History {
    pub fn for_config(config: &Path) -> Option<History> {
        Some(History {
            dir: paths::app_data_dir()?
                .join("runs")
                .join(paths::config_hash(config)),
        })
    }

    /// A new run's folder (older runs beyond the last 20 are removed).
    pub fn begin(&self) -> Option<RunDir> {
        std::fs::create_dir_all(&self.dir).ok()?;
        let stamp = crate::time::now().file_stamp();
        let mut path = self.dir.join(&stamp);
        let mut n = 2;
        while path.exists() {
            path = self.dir.join(format!("{stamp}-{n}"));
            n += 1;
        }
        std::fs::create_dir_all(&path).ok()?;
        self.prune();
        Some(RunDir { path })
    }

    fn runs(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        // Timestamps sort in time order.
        v.sort();
        v
    }

    fn prune(&self) {
        let runs = self.runs();
        if runs.len() > KEEP {
            for old in &runs[..runs.len() - KEEP] {
                let _ = std::fs::remove_dir_all(old);
            }
        }
    }

    /// The most recent finished run of `action`.
    pub fn last(&self, action: &str) -> Option<RunRecord> {
        self.runs().iter().rev().find_map(|p| {
            let text = std::fs::read_to_string(p.join("steps.json")).ok()?;
            let rec = RunRecord::from_json(&serde_json::from_str(&text).ok()?)?;
            (rec.action == action).then_some(rec)
        })
    }
}

impl RunDir {
    /// The log file of step `i` (appended to).
    pub fn step_log(&self, i: usize) -> Option<File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path.join(format!("step-{:02}.log", i + 1)))
            .ok()
    }

    pub fn write_record(&self, record: &RunRecord) {
        if let Ok(text) = serde_json::to_string_pretty(&record.to_json()) {
            let _ = crate::replace_file::write_bytes_replacing(&self.path.join("steps.json"), text);
        }
    }
}

/// What a step printed: the end of it in memory, and all of it in the step's log file when the
/// file saves logs.
#[derive(Default)]
pub struct StepLog {
    file: Option<File>,
    tail: Vec<u8>,
}

impl StepLog {
    pub fn new(file: Option<File>) -> StepLog {
        StepLog {
            file,
            tail: Vec::new(),
        }
    }

    /// The end of the output so far, without colour codes.
    pub fn text(&self) -> String {
        crate::engine::runner::strip_ansi(&String::from_utf8_lossy(&self.tail))
    }
}

/// Add to a step's log, ignoring failures (a log that can't be written never stops a run).
pub fn append(log: &mut StepLog, bytes: &[u8]) {
    if let Some(f) = log.file.as_mut() {
        let _ = f.write_all(bytes);
    }
    log.tail.extend_from_slice(bytes);
    if log.tail.len() > TAIL {
        log.tail.drain(..log.tail.len() - TAIL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip() {
        let r = RunRecord {
            action: "build".into(),
            started: "2026-09-29 10:00:00".into(),
            duration_ms: 1234,
            outcome: "failed".into(),
            steps: vec![StepRecord {
                label: "Building".into(),
                outcome: "failed".into(),
                duration_ms: 1000,
                message: Some("exit code 1".into()),
            }],
        };
        assert_eq!(RunRecord::from_json(&r.to_json()), Some(r));
    }
}

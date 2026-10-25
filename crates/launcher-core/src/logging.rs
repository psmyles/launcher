//! The session log (adapted from bite's `logging.rs`).
//!
//! * A 1000-entry ring buffer feeds the in-app log window.
//! * The file is `<app data>/logs/launcher.log` (or `LAUNCHER_LOG_PATH`), opened on the
//!   first line, held open, and written unbuffered so a tail sees each line as it lands. At the
//!   start of a session a file over 1 MB is rolled to `launcher.log.1`.
//! * `LAUNCHER_LOG=trace|debug|info|warn|error` sets the level (default `info`).
//! * Consecutive identical lines collapse to one plus "(repeated N times)".
//! * Stamps are local time.
//!
//! Nothing here reports its own failure: a log that cannot be written is not worth interrupting
//! a launch over, and the in-app window still reads the buffer.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    fn parse(s: &str) -> Option<Level> {
        match s.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(Level::Trace),
            "debug" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" | "warning" => Some(Level::Warn),
            "error" => Some(Level::Error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub timestamp: String,
    pub level: Level,
    pub message: String,
}

const MAX_ENTRIES: usize = 1000;
const ROLL_BYTES: u64 = 1024 * 1024;

struct Sink {
    file: Option<File>,
    opened: bool,
    last: Option<(Level, String)>,
    repeats: u32,
}

fn buffer() -> &'static Mutex<VecDeque<Entry>> {
    static BUFFER: OnceLock<Mutex<VecDeque<Entry>>> = OnceLock::new();
    BUFFER.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn sink() -> &'static Mutex<Sink> {
    static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();
    SINK.get_or_init(|| {
        Mutex::new(Sink {
            file: None,
            opened: false,
            last: None,
            repeats: 0,
        })
    })
}

/// 255 = not yet read from the environment.
static LEVEL: AtomicU8 = AtomicU8::new(255);
static STDERR: AtomicBool = AtomicBool::new(false);
/// Bumped on every new entry, so the log window can tell when to scroll.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The minimum level that is recorded.
pub fn level() -> Level {
    let raw = LEVEL.load(Ordering::Relaxed);
    if raw != 255 {
        return level_from_u8(raw);
    }
    let lvl = std::env::var("LAUNCHER_LOG")
        .ok()
        .and_then(|v| Level::parse(&v))
        .unwrap_or(Level::Info);
    LEVEL.store(lvl as u8, Ordering::Relaxed);
    lvl
}

fn level_from_u8(v: u8) -> Level {
    match v {
        0 => Level::Trace,
        1 => Level::Debug,
        2 => Level::Info,
        3 => Level::Warn,
        _ => Level::Error,
    }
}

/// Headless runs also echo recorded lines at `Warn` and above to stderr.
pub fn echo_to_stderr(on: bool) {
    STDERR.store(on, Ordering::Relaxed);
}

pub fn enabled(lvl: Level) -> bool {
    lvl >= level()
}

/// Where the log file lives: `LAUNCHER_LOG_PATH`, else `<app data>/logs/launcher.log`.
pub fn log_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("LAUNCHER_LOG_PATH") {
        return Some(PathBuf::from(explicit));
    }
    crate::paths::app_data_dir().map(|d| d.join("logs").join("launcher.log"))
}

fn open_file() -> Option<File> {
    let path = log_path()?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > ROLL_BYTES) {
        let mut rolled = path.clone().into_os_string();
        rolled.push(".1");
        let _ = std::fs::rename(&path, rolled);
    }
    let mut f = File::options().create(true).append(true).open(&path).ok()?;
    let _ = writeln!(
        f,
        "--- Session {} (pid {}) ---",
        crate::time::now().iso(),
        std::process::id()
    );
    Some(f)
}

/// Record a line: the buffer, the file, and (headless, warnings and up) stderr.
pub fn log(lvl: Level, message: impl Into<String>) {
    if !enabled(lvl) {
        return;
    }
    let message = message.into();
    let timestamp = crate::time::now().clock();
    if STDERR.load(Ordering::Relaxed) && lvl >= Level::Warn {
        // Never panic on a closed stderr (a shell that did not wait for us).
        use std::io::Write as _;
        let _ = writeln!(
            std::io::stderr(),
            "{}: {}",
            lvl.label().to_ascii_lowercase(),
            message
        );
    }
    let Ok(mut s) = sink().lock() else { return };
    if !s.opened {
        s.opened = true;
        s.file = open_file();
    }
    // Collapse consecutive duplicates: the first is written, the rest counted, and the count is
    // written before the next different line.
    if s.last
        .as_ref()
        .is_some_and(|(l, m)| *l == lvl && *m == message)
    {
        s.repeats += 1;
        return;
    }
    if s.repeats > 0 {
        let n = s.repeats;
        s.repeats = 0;
        let line = format!("{timestamp} [INFO] (previous line repeated {n} more times)");
        if let Some(f) = s.file.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
    s.last = Some((lvl, message.clone()));
    if let Some(f) = s.file.as_mut() {
        let _ = writeln!(f, "{timestamp} [{}] {message}", lvl.label());
    }
    drop(s);
    if let Ok(mut entries) = buffer().lock() {
        while entries.len() >= MAX_ENTRIES {
            entries.pop_front();
        }
        entries.push_back(Entry {
            timestamp,
            level: lvl,
            message,
        });
    }
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Every buffered line, newest last.
pub fn entries() -> Vec<Entry> {
    buffer()
        .lock()
        .map(|e| e.iter().cloned().collect())
        .unwrap_or_default()
}

/// Changes whenever a line is added.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub fn clear_buffer() {
    if let Ok(mut e) = buffer().lock() {
        e.clear();
    }
}

#[macro_export]
macro_rules! log_info { ($($t:tt)*) => { $crate::logging::log($crate::logging::Level::Info, format!($($t)*)) } }
#[macro_export]
macro_rules! log_warn { ($($t:tt)*) => { $crate::logging::log($crate::logging::Level::Warn, format!($($t)*)) } }
#[macro_export]
macro_rules! log_error { ($($t:tt)*) => { $crate::logging::log($crate::logging::Level::Error, format!($($t)*)) } }
#[macro_export]
macro_rules! log_debug {
    ($($t:tt)*) => {
        if $crate::logging::enabled($crate::logging::Level::Debug) {
            $crate::logging::log($crate::logging::Level::Debug, format!($($t)*))
        }
    }
}

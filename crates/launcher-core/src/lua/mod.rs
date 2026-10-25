//! Luau (N4, N6): the logic language of `.launcher` files.
//!
//! * **Sandboxed.** Luau's standard library without `io` or process control; the globals are
//!   frozen. Every side effect goes through [`Effects`] (and, during a run, the [`Host`]), so it
//!   shows in the console, obeys Cancel and dry runs, and is logged.
//! * **Budgeted.** An interrupt checks a deadline and a cancel flag, so evaluation always ends:
//!   load-time code has a time budget, and Cancel stops a running `lua` step.
//! * **Phased.** While a file loads (no run, so no [`Host`]), only the API marked
//!   [`api::Phase::Any`] works; the rest raises "can't run while the file loads".
//! * **Mapped.** Errors name `.launcher` file lines, not chunk lines ([`source`]).
//!
//! `values` and `fields` are read-only views of a [`Snapshot`] the host replaces as the form and
//! the run change.

pub mod api;
mod bind;
pub mod lint;
pub mod source;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Counts `eval_str` snippets, so each gets its own compile-cache key.
static SCRATCH: AtomicUsize = AtomicUsize::new(0);
use std::time::{Duration, Instant};

use mlua::{Function, Lua, VmState};

use crate::config::LuaSrc;
use crate::dynval::Dyn;
use crate::engine::effects::{CommandSpec, Context, Effects};

pub use api::Phase;

/// What Lua code sees: `values.x` and `fields.x.Key`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub values: BTreeMap<String, Dyn>,
    pub fields: BTreeMap<String, BTreeMap<String, Dyn>>,
}

/// Run-time interactions (the run worker implements this; nothing does while a file loads).
pub trait Host: Send {
    /// Run a program in the console (`pipe`: plain output instead of a terminal); its exit code.
    fn run(&mut self, cmd: &CommandSpec, pipe: bool) -> Result<i32, String>;
    /// A `step(label, fn)` section begins (`Some(label)`) or ends (`None`, with its outcome).
    fn step(&mut self, label: Option<&str>, ok: bool);
    fn log(&mut self, text: &str);
    fn message(&mut self, text: &str);
    fn confirm(&mut self, text: &str) -> bool;
    fn ask(&mut self, prompt: &str, default: Option<&str>) -> Option<String>;
    /// A run variable or a control value.
    fn set(&mut self, name: &str, value: Dyn);
}

/// A Lua error, its message already pointing at the `.launcher` file.
#[derive(Clone, Debug, PartialEq)]
pub struct LuaError {
    pub message: String,
    /// The file line and column the error points at, when known.
    pub line: Option<usize>,
    pub col: Option<usize>,
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `fail(message)`: an error shown as just its message.
#[derive(Debug)]
struct Fail(String);

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Fail {}

/// State the bindings share with the VM's owner.
pub(crate) struct Shared {
    pub snapshot: Mutex<Snapshot>,
    pub deadline: Mutex<Option<(Instant, Duration)>>,
    pub cancel: AtomicBool,
    pub effects: Arc<dyn Effects>,
    pub cx: Mutex<Context>,
    pub host: Mutex<Option<Box<dyn Host>>>,
    /// Running a control's on-change: of the run-time API only `set()` and `log()` work.
    pub instant: AtomicBool,
}

impl Shared {
    /// Time left in the current budget.
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .lock()
            .ok()
            .and_then(|d| *d)
            .map(|(at, _)| at.saturating_duration_since(Instant::now()))
    }

    pub fn cx(&self) -> Context {
        self.cx.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Call the host (a run is going), or say this can't be done while loading.
    pub fn with_host<R>(&self, what: &str, f: impl FnOnce(&mut dyn Host) -> R) -> mlua::Result<R> {
        let mut host = self
            .host
            .lock()
            .map_err(|_| mlua::Error::runtime("the run state is poisoned"))?;
        match host.as_mut() {
            Some(h) => Ok(f(h.as_mut())),
            None => Err(mlua::Error::runtime(format!(
                "{what} can only be used in an action (it can't run while the file loads)"
            ))),
        }
    }
}

pub struct Vm {
    lua: Lua,
    shared: Arc<Shared>,
    /// The file's name, for error messages.
    file_name: String,
    /// Compiled chunks by their place in the file (conditions are evaluated again and again).
    compiled: Mutex<HashMap<(usize, usize, bool), Function>>,
    /// Where each chunk came from, by its tag (`@N` in Luau's messages).
    sources: Mutex<Vec<LuaSrc>>,
}

/// The default time budget for code that runs while a file loads.
pub const LOAD_BUDGET: Duration = Duration::from_secs(5);

impl Vm {
    /// A sandboxed VM for the file at `file`.
    pub fn new(file: &Path, effects: Arc<dyn Effects>) -> Result<Vm, String> {
        let dir = file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot::default()),
            deadline: Mutex::new(None),
            cancel: AtomicBool::new(false),
            effects,
            cx: Mutex::new(Context::new(&dir)),
            host: Mutex::new(None),
            instant: AtomicBool::new(false),
        });
        let lua = Lua::new();
        // In a sandbox Luau resolves `values.x` once, when a chunk loads, unless the global is
        // declared mutable; these views change under the same compiled chunk.
        lua.set_compiler(
            mlua::chunk::Compiler::new()
                .set_mutable_globals(["values", "fields", "env", "settings"]),
        );
        bind::install(&lua, &shared, file, &dir).map_err(|e| e.to_string())?;
        lua.sandbox(true).map_err(|e| e.to_string())?;
        let s = shared.clone();
        lua.set_interrupt(move |_| {
            if s.cancel.load(Ordering::Relaxed) {
                return Err(mlua::Error::runtime("cancelled"));
            }
            if let Some((at, budget)) = s.deadline.lock().ok().and_then(|d| *d) {
                if Instant::now() > at {
                    return Err(mlua::Error::runtime(format!(
                        "the Lua code ran longer than its {} s budget",
                        budget.as_secs_f32()
                    )));
                }
            }
            Ok(VmState::Continue)
        });
        Ok(Vm {
            lua,
            shared,
            file_name: file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into()),
            compiled: Mutex::new(HashMap::new()),
            sources: Mutex::new(Vec::new()),
        })
    }

    pub fn set_snapshot(&self, s: Snapshot) {
        if let Ok(mut snap) = self.shared.snapshot.lock() {
            *snap = s;
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .snapshot
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// The run's host; with one, the run-time API works (a run is going).
    pub fn set_host(&self, host: Option<Box<dyn Host>>) {
        if let Ok(mut h) = self.shared.host.lock() {
            *h = host;
        }
    }

    /// On-change code runs with a host that takes `set()` and `log()`; nothing else of the
    /// run-time API works there (no console, no dialogs, no changes on disk).
    pub fn set_instant(&self, on: bool) {
        self.shared.instant.store(on, Ordering::Relaxed);
    }

    pub fn set_env(&self, env: BTreeMap<String, String>) {
        if let Ok(mut cx) = self.shared.cx.lock() {
            cx.env = env;
        }
    }

    /// Stop running Lua code as soon as it reaches its next check (Cancel).
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// A handle that cancels from another thread.
    pub fn canceller(&self) -> impl Fn() + Send + Sync + 'static {
        let s = self.shared.clone();
        move || s.cancel.store(true, Ordering::Relaxed)
    }

    /// Compile `src` (as an expression or a chunk), once per place in the file.
    fn function(&self, src: &LuaSrc, expr: bool) -> Result<Function, LuaError> {
        let key = (src.span.offset, src.span.len, expr);
        if let Some(f) = self.compiled.lock().ok().and_then(|c| c.get(&key).cloned()) {
            return Ok(f);
        }
        // Registered before compiling, so a syntax error maps too.
        let tag = match self.sources.lock() {
            Ok(mut s) => {
                s.push(src.clone());
                s.len() - 1
            }
            Err(_) => 0,
        };
        let code = if expr {
            format!("return ({}\n)", src.code)
        } else {
            src.code.clone()
        };
        let function = self
            .lua
            .load(&code)
            .set_name(format!("=@{tag}"))
            .into_function()
            .map_err(|e| self.map_error(&e, Some(src)))?;
        if let Ok(mut c) = self.compiled.lock() {
            c.insert(key, function.clone());
        }
        Ok(function)
    }

    fn with_budget<R>(&self, budget: Option<Duration>, f: impl FnOnce() -> R) -> R {
        self.shared.cancel.store(false, Ordering::Relaxed);
        if let Ok(mut d) = self.shared.deadline.lock() {
            *d = budget.map(|b| (Instant::now() + b, b));
        }
        let r = f();
        if let Ok(mut d) = self.shared.deadline.lock() {
            *d = None;
        }
        r
    }

    /// Run statements (a `script` block or a `lua` step).
    pub fn exec(&self, src: &LuaSrc, budget: Option<Duration>) -> Result<(), LuaError> {
        let f = self.function(src, false)?;
        self.with_budget(budget, || {
            f.call::<()>(()).map_err(|e| self.map_error(&e, Some(src)))
        })
    }

    /// Run statements and keep what they return (an add-on step's `script`, a list's).
    pub fn exec_value(&self, src: &LuaSrc, budget: Option<Duration>) -> Result<Dyn, LuaError> {
        let f = self.function(src, false)?;
        self.with_budget(budget, || {
            f.call::<mlua::Value>(())
                .map(|v| bind::to_dyn(&v))
                .map_err(|e| self.map_error(&e, Some(src)))
        })
    }

    /// Evaluate an expression (a `(lua)"..."` value or a condition).
    pub fn eval(&self, src: &LuaSrc, budget: Option<Duration>) -> Result<Dyn, LuaError> {
        let f = self.function(src, true)?;
        self.with_budget(budget, || {
            f.call::<mlua::Value>(())
                .map(|v| bind::to_dyn(&v))
                .map_err(|e| self.map_error(&e, Some(src)))
        })
    }

    /// Run code that isn't from the file (tests, the editor's scratch pad).
    pub fn eval_str(&self, code: &str) -> Result<Dyn, LuaError> {
        let src = LuaSrc {
            code: code.to_string(),
            repr: format!("{code:?}"),
            // A unique place per snippet: the compile cache is keyed by place, and two snippets
            // of the same length must not share an entry.
            span: crate::config::Span {
                offset: usize::MAX / 2 + SCRATCH.fetch_add(1, Ordering::Relaxed),
                len: code.len(),
            },
            line: 1,
            col: 1,
        };
        self.eval(&src, Some(LOAD_BUDGET))
    }

    /// Turn an mlua error into one that names file lines.
    fn map_error(&self, e: &mlua::Error, fallback: Option<&LuaSrc>) -> LuaError {
        let (message, traceback) = describe(e);
        let sources = self.sources.lock();
        let lookup = |tag: usize| -> Option<LuaSrc> {
            sources.as_ref().ok().and_then(|s| s.get(tag).cloned())
        };
        let mut first: Option<(usize, usize)> = None;
        let mut mapped = rewrite_positions(&message, &self.file_name, &lookup, &mut first);
        if first.is_none() {
            // A Rust callback's error carries no position; the traceback knows where the call was.
            if let Some(tb) = &traceback {
                let _ = rewrite_positions(tb, &self.file_name, &lookup, &mut first);
            }
            // fail(message) is shown as just its message: it's for people, and in an add-on the
            // line is the add-on's, not the tool's.
            if let (Some((line, _)), false, false) =
                (first, mapped.starts_with(&self.file_name), is_fail(e))
            {
                mapped = format!("{}:{line}: {mapped}", self.file_name);
            }
        }
        if first.is_none() {
            if let Some(src) = fallback {
                let (line, col) = source::file_position(src, 1, 1);
                first = Some((line, col));
            }
        }
        LuaError {
            message: mapped,
            line: first.map(|f| f.0),
            col: first.map(|f| f.1),
        }
    }
}

/// Whether the error is `fail(message)`.
fn is_fail(e: &mlua::Error) -> bool {
    match e {
        mlua::Error::CallbackError { cause, .. } | mlua::Error::WithContext { cause, .. } => {
            is_fail(cause)
        }
        mlua::Error::ExternalError(inner) => inner.downcast_ref::<Fail>().is_some(),
        _ => false,
    }
}

/// The message of an mlua error (without Luau's traceback), and the traceback if there is one.
fn describe(e: &mlua::Error) -> (String, Option<String>) {
    match e {
        mlua::Error::CallbackError { traceback, cause } => {
            let (m, _) = describe(cause);
            (m, Some(traceback.clone()))
        }
        mlua::Error::ExternalError(inner) => (inner.to_string(), None),
        mlua::Error::SyntaxError { message, .. } => (message.clone(), None),
        mlua::Error::RuntimeError(m) => (m.clone(), None),
        mlua::Error::WithContext { cause, .. } => describe(cause),
        other => (other.to_string(), None),
    }
}

/// Replace every `@N:LINE:` (a chunk position) in `text` with `file:FILELINE:`; the first one's
/// file position goes into `first`.
fn rewrite_positions(
    text: &str,
    file: &str,
    lookup: &dyn Fn(usize) -> Option<LuaSrc>,
    first: &mut Option<(usize, usize)>,
) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let tag_len = after.chars().take_while(|c| c.is_ascii_digit()).count();
        let parsed = (tag_len > 0 && after[tag_len..].starts_with(':'))
            .then(|| {
                let line_part = &after[tag_len + 1..];
                let line_len = line_part.chars().take_while(|c| c.is_ascii_digit()).count();
                let tag: usize = after[..tag_len].parse().ok()?;
                let line: usize = line_part[..line_len].parse().ok()?;
                let src = lookup(tag)?;
                Some((tag_len + 1 + line_len, src, line))
            })
            .flatten();
        match parsed {
            Some((consumed, src, line)) => {
                let (fl, fc) = source::file_position(&src, line, 1);
                first.get_or_insert((fl, fc));
                out.push_str(&format!("{file}:{fl}"));
                rest = &after[consumed..];
            }
            None => {
                out.push('@');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;

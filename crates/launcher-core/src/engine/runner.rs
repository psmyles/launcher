//! Carrying out an action (M4): its steps, in order, on a worker thread.
//!
//! * **Locks (N11):** a run holds its file's lock (one run per file) and its action's `lock=`
//!   path; a second run of the same file is refused, a different file isn't.
//! * **Checks first:** an action's `check` steps are its preconditions and run before any other
//!   step; one that fails ends the run before anything has changed.
//! * **Failures:** `on-error="abort"` ends the run (Retry resumes at that step, keeping the run's
//!   variables); `warn` shows the warning, which pauses the run until it is acknowledged (N15);
//!   `continue` notes it and goes on.
//! * **Cancel** kills the running program's whole tree and interrupts Lua.
//! * **Lua** in a run (`lua` steps, `(lua)` values, `if=`) shares one VM per run with the file's
//!   scripts loaded; its `run()`, `ask()`... reach the same console and dialogs as steps do.
//! * Each run keeps the system awake. It's written to the file's run history only when the file
//!   asks (`save-logs`); a step's output is also kept in memory, for an add-on's explanations.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::effects::{CommandSpec, Context, Effects, IoMode, Proc};
use super::history::{self, History, RunRecord, StepRecord};
use super::lock::{self, LockError, RunLock};
use crate::addons::{Body, Need, SettingDef, SettingKind, StepDef};
use crate::config::template::Part;
use crate::config::*;
use crate::dynval::Dyn;
use crate::eval::{lines_of, render, render_lines, whole_value};
use crate::lua::{Host, Snapshot, Vm, LOAD_BUDGET};
use crate::platform::process::exit_code_text;
use crate::platform::trust::Trusted;

/// How a step ended.
#[derive(Clone, Debug, PartialEq)]
pub enum StepOutcome {
    Ok,
    /// Its `if=` was false.
    Skipped,
    /// It failed, and `on-error` said to go on.
    Warned(String),
    Failed(String),
    Cancelled,
}

/// How the run ended.
#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    Ok,
    /// A step failed (its message).
    Failed(String),
    Cancelled,
    /// A `stop` step, or No to a `confirm`: ended early, quietly.
    Stopped,
}

/// What a Retry needs: where to start and the run's variables so far.
#[derive(Clone, Debug)]
pub struct Resume {
    pub from: usize,
    pub vars: Snapshot,
    pub env: BTreeMap<String, String>,
    /// The files the run wrote, so a step that adds to one still adds after a Retry.
    pub written: BTreeSet<PathBuf>,
}

/// A top-level step, as the step list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct StepInfo {
    pub label: String,
    /// An instant step (set, set-control, reset, log): no row of its own in a step list.
    pub quiet: bool,
}

/// Progress, sent to the UI as it happens.
pub enum RunEvent {
    /// The action's top-level steps, and the folder that keeps this run's logs (none for a dry
    /// run, or when history is off).
    Started {
        steps: Vec<StepInfo>,
        log_dir: Option<std::path::PathBuf>,
    },
    StepStarted {
        index: usize,
    },
    /// Program output for a step (raw bytes, as a terminal would get them).
    Output {
        index: usize,
        bytes: Vec<u8>,
    },
    /// A line from the engine or a `log` step.
    Log {
        index: usize,
        text: String,
    },
    /// A program started in a pty: the console can type into it and resize it.
    Attached {
        index: usize,
        process: Arc<dyn Proc>,
    },
    StepFinished {
        index: usize,
        outcome: StepOutcome,
        duration: Duration,
    },
    Finished {
        outcome: RunOutcome,
        /// Where a Retry would start (set when the run failed).
        resume: Option<Resume>,
    },
}

/// The UI side of a run: progress and the dialogs a run can open. The dialog calls block the run
/// until answered.
pub trait RunUi: Send {
    fn event(&mut self, e: RunEvent);
    fn message(&mut self, text: &str, level: Level);
    fn confirm(&mut self, text: &str, yes: &str, no: &str) -> bool;
    fn ask(&mut self, prompt: &str, default: Option<&str>, password: bool) -> Option<String>;
    /// `on-error="warn"`: show it; the run waits until it's acknowledged.
    fn warn(&mut self, text: &str);
    /// `set-control` (Some) or `reset` (None).
    fn set_control(&mut self, id: &str, value: Option<Dyn>);
    /// `put` of a list into a choice: its new choices (the window keeps them).
    fn set_choices(&mut self, _id: &str, _choices: Vec<String>) {}
}

pub struct RunSpec {
    pub config: Arc<Config>,
    pub action: String,
    /// The form's values when the run started.
    pub snapshot: Snapshot,
    pub env: BTreeMap<String, String>,
    pub effects: Arc<dyn Effects>,
    pub trusted: Trusted,
    /// Retry: resume here instead of starting at the top.
    pub resume: Option<Resume>,
    /// Write the run to the file's history, if the file saves logs (`save-logs`).
    pub history: bool,
    /// The console's size (columns, rows) for programs in a pty.
    pub term_size: (u16, u16),
    /// Give programs a pseudo-terminal (`io="auto"`); without one (output to a file or a pipe)
    /// they get pipes.
    pub pty: bool,
}

/// Stops a run from another thread.
#[derive(Default)]
struct Cancel {
    flag: AtomicBool,
    current: Mutex<Option<Arc<dyn Proc>>>,
    lua: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl Cancel {
    fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    fn fire(&self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(p) = self.current.lock().ok().and_then(|c| c.clone()) {
            p.kill();
        }
        if let Ok(l) = self.lua.lock() {
            if let Some(f) = l.as_ref() {
                f();
            }
        }
    }
}

pub struct RunHandle {
    cancel: Arc<Cancel>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RunHandle {
    pub fn cancel(&self) {
        self.cancel.fire();
    }

    /// A function that cancels the run, for a Ctrl+C handler or another thread.
    pub fn canceller(&self) -> impl Fn() + Send + Sync + 'static {
        let c = self.cancel.clone();
        move || c.fire()
    }

    pub fn join(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Take the locks and start the run on its own thread.
pub fn start(spec: RunSpec, ui: Box<dyn RunUi>) -> Result<RunHandle, String> {
    let action = spec
        .config
        .action(&spec.action)
        .cloned()
        .ok_or_else(|| format!("there is no action `{}`", spec.action))?;
    let mut locks: Vec<RunLock> = Vec::new();
    if !spec.effects.is_dry() {
        match lock::lock_file(&spec.config.path) {
            Ok(l) => locks.push(l),
            Err(LockError::Busy) => {
                return Err(format!(
                    "{} is already running an action",
                    spec.config.path.display()
                ))
            }
            Err(LockError::Io(e)) => return Err(format!("can't take the run lock: {e}")),
        }
        if let Some(Val::Text(t)) = &action.lock {
            let p = crate::paths::resolve(&spec.config.dir, &render(t, &spec.snapshot, &spec.env));
            match lock::lock_named(&p) {
                Ok(l) => locks.push(l),
                Err(LockError::Busy) => {
                    return Err(format!(
                        "another run holds the lock {} (lock= of `{}`)",
                        p.display(),
                        action.id
                    ))
                }
                Err(LockError::Io(e)) => return Err(format!("can't take the lock: {e}")),
            }
        }
    }
    let cancel = Arc::new(Cancel::default());
    let c = cancel.clone();
    let thread = std::thread::Builder::new()
        .name("run".into())
        .spawn(move || {
            let _locks = locks;
            let _awake = crate::platform::awake::KeepAwake::start();
            let ui = Arc::new(Mutex::new(ui));
            let mut run = Runner::new(spec, action, ui.clone(), c);
            let (outcome, resume) = run.go();
            run.record_history(&outcome);
            if let Ok(mut u) = ui.lock() {
                u.event(RunEvent::Finished { outcome, resume });
            };
        })
        .map_err(|e| format!("could not start the run: {e}"))?;
    Ok(RunHandle {
        cancel,
        thread: Some(thread),
    })
}

type SharedUi = Arc<Mutex<Box<dyn RunUi>>>;

/// Lua's side of a run: `run()`, `ask()` and friends reach the same console and dialogs.
struct LuaHost {
    ui: SharedUi,
    effects: Arc<dyn Effects>,
    cx: Arc<Mutex<Context>>,
    cancel: Arc<Cancel>,
    index: Arc<Mutex<usize>>,
    term: (u16, u16),
    pty: bool,
    controls: Vec<String>,
    /// The running step's log, lent while its Lua runs (`Runner::lua`): what `run()` and
    /// `log()` print goes in it, as a run step's output does.
    log: Arc<Mutex<history::StepLog>>,
}

impl LuaHost {
    fn index(&self) -> usize {
        self.index.lock().map(|i| *i).unwrap_or(0)
    }

    fn event(&self, e: RunEvent) {
        if let Ok(mut u) = self.ui.lock() {
            u.event(e);
        }
    }
}

impl Host for LuaHost {
    fn run(&mut self, cmd: &CommandSpec, pipe: bool) -> Result<i32, String> {
        let cx = self.cx.lock().map(|c| c.clone()).unwrap_or_default();
        let index = self.index();
        let mut log = self
            .log
            .lock()
            .map(|mut l| std::mem::take(&mut *l))
            .unwrap_or_default();
        let code = run_process(
            &self.effects,
            &cx,
            cmd,
            if self.pty && !pipe {
                IoMode::Auto
            } else {
                IoMode::Pipe
            },
            self.term,
            index,
            &self.ui,
            &self.cancel,
            None,
            &mut log,
        );
        if let Ok(mut l) = self.log.lock() {
            *l = log;
        }
        code
    }

    fn step(&mut self, label: Option<&str>, ok: bool) {
        let text = match label {
            Some(l) => format!("── {l}"),
            None if ok => "── done".to_string(),
            None => "── failed".to_string(),
        };
        let index = self.index();
        self.event(RunEvent::Log { index, text });
    }

    fn log(&mut self, text: &str) {
        let index = self.index();
        if let Ok(mut l) = self.log.lock() {
            history::append(&mut l, format!("{text}\n").as_bytes());
        }
        self.event(RunEvent::Log {
            index,
            text: text.to_string(),
        });
    }

    fn message(&mut self, text: &str) {
        if let Ok(mut u) = self.ui.lock() {
            u.message(text, Level::Info);
        }
    }

    fn confirm(&mut self, text: &str) -> bool {
        self.ui
            .lock()
            .map(|mut u| u.confirm(text, "Yes", "No"))
            .unwrap_or(false)
    }

    fn ask(&mut self, prompt: &str, default: Option<&str>) -> Option<String> {
        self.ui
            .lock()
            .ok()
            .and_then(|mut u| u.ask(prompt, default, false))
    }

    fn set(&mut self, name: &str, value: Dyn) {
        if self.controls.iter().any(|c| c == name) {
            if let Ok(mut u) = self.ui.lock() {
                u.set_control(name, Some(value));
            }
        }
    }
}

/// Start a program, stream its output to the UI (and the step's log), and wait for it with
/// Cancel and the timeout watched.
#[allow(clippy::too_many_arguments)]
fn run_process(
    effects: &Arc<dyn Effects>,
    cx: &Context,
    cmd: &CommandSpec,
    io: IoMode,
    term: (u16, u16),
    index: usize,
    ui: &SharedUi,
    cancel: &Arc<Cancel>,
    fail_if_output: Option<&regex::Regex>,
    log: &mut history::StepLog,
) -> Result<i32, String> {
    let matched: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let p = effects.start(
        cx,
        cmd,
        io,
        term,
        Box::new(move |b| {
            let _ = tx.send(b.to_vec());
        }),
    )?;
    let p: Arc<dyn Proc> = Arc::from(p);
    if let Ok(mut c) = cancel.current.lock() {
        *c = Some(p.clone());
    }
    if p.is_pty() {
        if let Ok(mut u) = ui.lock() {
            u.event(RunEvent::Attached {
                index,
                process: p.clone(),
            });
        }
    }
    let started = Instant::now();
    let mut line = String::new();
    let mut timed_out = false;
    let code = loop {
        // Output first, so it's all shown before the end.
        while let Ok(chunk) = rx.try_recv() {
            if chunk.is_empty() {
                continue;
            }
            history::append(log, &chunk);
            if let Some(re) = fail_if_output {
                line.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(nl) = line.find('\n') {
                    let l: String = line.drain(..=nl).collect();
                    let clean = strip_ansi(l.trim_end());
                    if re.is_match(&clean) {
                        if let Ok(mut m) = matched.lock() {
                            m.get_or_insert(clean);
                        }
                    }
                }
            }
            if let Ok(mut u) = ui.lock() {
                u.event(RunEvent::Output {
                    index,
                    bytes: chunk,
                });
            }
        }
        if let Some(code) = p.wait_timeout(Duration::from_millis(20)) {
            break Some(code);
        }
        if cancel.is_set() {
            p.kill();
            break None;
        }
        if cmd.timeout.is_some_and(|t| started.elapsed() > t) {
            p.kill();
            timed_out = true;
            break None;
        }
    };
    // What was printed just before the end.
    let drain_until = Instant::now() + Duration::from_secs(2);
    while let Ok(chunk) = rx.recv_timeout(drain_until.saturating_duration_since(Instant::now())) {
        if chunk.is_empty() {
            break;
        }
        history::append(log, &chunk);
        if let Some(re) = fail_if_output {
            for l in String::from_utf8_lossy(&chunk).lines() {
                let clean = strip_ansi(l);
                if re.is_match(&clean) {
                    if let Ok(mut m) = matched.lock() {
                        m.get_or_insert(clean);
                    }
                }
            }
        }
        if let Ok(mut u) = ui.lock() {
            u.event(RunEvent::Output {
                index,
                bytes: chunk,
            });
        }
    }
    if let Ok(mut c) = cancel.current.lock() {
        *c = None;
    }
    if timed_out {
        return Err(format!(
            "`{}` didn't finish within {} s and was stopped",
            cmd.program,
            cmd.timeout.unwrap_or_default().as_secs_f32()
        ));
    }
    let Some(code) = code else {
        return Err("cancelled".into());
    };
    if let Some(m) = matched.lock().ok().and_then(|m| m.clone()) {
        return Err(format!("the output matched fail-if-output: {m}"));
    }
    Ok(code)
}

/// Terminal escape sequences removed, for matching output lines against a pattern.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: until BEL or ESC \.
                    for n in chars.by_ref() {
                        if n == '\u{7}' || n == '\\' {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// A step's result: values (`{id}`) and fields (`{id.field}`).
#[derive(Default)]
struct Produced {
    value: Option<Dyn>,
    fields: BTreeMap<String, Dyn>,
    /// Other names the step sets (`set`, `ask`, `extract`, `args`).
    names: Vec<(String, Dyn)>,
}

enum Flow {
    Next(Produced),
    /// End the run quietly (`stop`, No to `confirm`).
    Stop,
}

struct Runner {
    config: Arc<Config>,
    action: Action,
    effects: Arc<dyn Effects>,
    ui: SharedUi,
    cancel: Arc<Cancel>,
    vm: Option<Vm>,
    /// Where the Lua host prints: the log of the step whose Lua is running.
    lua_log: Arc<Mutex<history::StepLog>>,
    cx: Arc<Mutex<Context>>,
    index: Arc<Mutex<usize>>,
    term: (u16, u16),
    pty: bool,
    resume_from: usize,
    procs: HashMap<String, Arc<dyn Proc>>,
    history: Option<history::RunDir>,
    record: RunRecord,
    started: Instant,
    /// The files this run has written: a step that adds to a file starts it afresh only when
    /// it is the run's first write to it.
    written: BTreeSet<PathBuf>,
}

impl Runner {
    fn new(spec: RunSpec, action: Action, ui: SharedUi, cancel: Arc<Cancel>) -> Runner {
        let cx = Arc::new(Mutex::new(Context {
            config_dir: spec.config.dir.clone(),
            env: spec
                .resume
                .as_ref()
                .map(|r| r.env.clone())
                .unwrap_or_else(|| spec.env.clone()),
        }));
        let index = Arc::new(Mutex::new(0usize));
        let vars = spec
            .resume
            .as_ref()
            .map(|r| r.vars.clone())
            .unwrap_or_else(|| spec.snapshot.clone());
        let vm = Vm::new(&spec.config.path, spec.effects.clone()).ok();
        let lua_log = Arc::new(Mutex::new(history::StepLog::default()));
        if let Some(vm) = &vm {
            vm.set_snapshot(vars);
            if let Ok(c) = cx.lock() {
                vm.set_env(c.env.clone());
            }
            if let Ok(mut l) = cancel.lua.lock() {
                *l = Some(Box::new(vm.canceller()));
            }
            vm.set_host(Some(Box::new(LuaHost {
                ui: ui.clone(),
                effects: spec.effects.clone(),
                cx: cx.clone(),
                cancel: cancel.clone(),
                index: index.clone(),
                term: spec.term_size,
                pty: spec.pty,
                controls: spec.config.controls.iter().map(|c| c.id.clone()).collect(),
                log: lua_log.clone(),
            })));
        }
        let history = (spec.history && spec.config.header.save_logs && !spec.effects.is_dry())
            .then(|| History::for_config(&spec.config.path).and_then(|h| h.begin()))
            .flatten();
        let _ = spec.trusted;
        Runner {
            record: RunRecord {
                action: action.id.clone(),
                started: crate::time::now().iso(),
                ..Default::default()
            },
            config: spec.config,
            action,
            effects: spec.effects,
            ui,
            cancel,
            vm,
            lua_log,
            cx,
            index,
            term: spec.term_size,
            pty: spec.pty,
            resume_from: spec.resume.as_ref().map_or(0, |r| r.from),
            procs: HashMap::new(),
            history,
            started: Instant::now(),
            written: spec
                .resume
                .as_ref()
                .map(|r| r.written.clone())
                .unwrap_or_default(),
        }
    }

    fn event(&self, e: RunEvent) {
        if let Ok(mut u) = self.ui.lock() {
            u.event(e);
        }
    }

    fn vars(&self) -> Snapshot {
        self.vm.as_ref().map(|v| v.snapshot()).unwrap_or_default()
    }

    fn set_vars(&self, s: Snapshot) {
        if let Some(vm) = &self.vm {
            vm.set_snapshot(s);
        }
    }

    fn env(&self) -> BTreeMap<String, String> {
        self.cx.lock().map(|c| c.env.clone()).unwrap_or_default()
    }

    fn cx(&self) -> Context {
        self.cx.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// A template or `(lua)` value as text; `extra` adds names ({error}, {key}...).
    fn text_with(&self, v: &Val, extra: &[(&str, &str)]) -> Result<String, String> {
        match v {
            Val::Text(t) => {
                let mut s = self.vars();
                for (k, val) in extra {
                    s.values.insert(k.to_string(), Dyn::str(*val));
                }
                Ok(render(t, &s, &self.env()))
            }
            Val::Lua(src) => self.eval(src).map(|d| d.to_text()),
        }
    }

    fn text(&self, v: &Val) -> Result<String, String> {
        self.text_with(v, &[])
    }

    /// Text that has lines (a file's content): a list goes in one item per line.
    fn text_lines(&self, v: &Val) -> Result<String, String> {
        match v {
            Val::Text(t) => Ok(render_lines(t, &self.vars(), &self.env())),
            Val::Lua(src) => self.eval(src).map(|d| lines_of(&d)),
        }
    }

    fn eval(&self, src: &LuaSrc) -> Result<Dyn, String> {
        let vm = self.vm.as_ref().ok_or("the Lua VM couldn't start")?;
        vm.eval(src, None).map_err(|e| e.message)
    }

    /// Run the file's scripts in the run's VM.
    fn load_scripts(&self) -> Result<(), String> {
        let vm = self.vm.as_ref().ok_or("the Lua VM couldn't start")?;
        for s in &self.config.scripts {
            vm.exec(s, Some(LOAD_BUDGET)).map_err(|e| e.message)?;
        }
        Ok(())
    }

    fn label(&self, s: &Step) -> String {
        if let Some(l) = &s.label {
            if let Ok(t) = self.text(l) {
                if !t.is_empty() {
                    return t;
                }
            }
        }
        // A row's words are worked out before the run: what an earlier step will find isn't
        // known yet, so it's named ("the Models to fix from Audit models").
        let found = |v: &Val| -> Option<String> {
            let Val::Text(t) = v else { return None };
            let [Part::Ref {
                name,
                field: Some(f),
                ..
            }] = t.parts.as_slice()
            else {
                return None;
            };
            self.action.steps.iter().find_map(|st| {
                let StepKind::Use(u) = &st.kind else {
                    return None;
                };
                if st.id.as_deref() != Some(name.as_str()) {
                    return None;
                }
                let r = u.def.results.iter().find(|r| r.id == *f)?;
                Some(format!("the {} from {}", r.label, u.def.name))
            })
        };
        default_label(s, |v| {
            let t = self.text(v).unwrap_or_default();
            if t.is_empty() {
                found(v).unwrap_or(t)
            } else {
                t
            }
        })
    }

    /// The whole run: (outcome, where a Retry would start).
    fn go(&mut self) -> (RunOutcome, Option<Resume>) {
        let steps = self.action.steps.clone();
        let infos: Vec<StepInfo> = steps
            .iter()
            .map(|s| StepInfo {
                label: self.label(s),
                quiet: matches!(
                    s.kind,
                    StepKind::Set { .. }
                        | StepKind::SetControl { .. }
                        | StepKind::Put { .. }
                        | StepKind::Reset { .. }
                        | StepKind::Log { .. }
                ),
            })
            .collect();
        self.record.steps = infos
            .iter()
            .map(|i| StepRecord {
                label: i.label.clone(),
                outcome: "not run".into(),
                ..Default::default()
            })
            .collect();
        let log_dir = self.history.as_ref().map(|h| h.path.clone());
        self.event(RunEvent::Started {
            steps: infos,
            log_dir,
        });
        if let Err(e) = self.load_scripts() {
            return (RunOutcome::Failed(e), None);
        }
        // Checks first (preconditions), then the rest in order.
        let checks: Vec<usize> = (0..steps.len())
            .filter(|i| steps[*i].first || matches!(steps[*i].kind, StepKind::Check { .. }))
            .collect();
        let rest: Vec<usize> = (self.resume_from..steps.len())
            .filter(|i| !checks.contains(i))
            .collect();
        for i in checks.into_iter().chain(rest) {
            if self.cancel.is_set() {
                return (RunOutcome::Cancelled, None);
            }
            match self.run_step(i, &steps[i]) {
                Ok(true) => {}
                Ok(false) => return (RunOutcome::Stopped, None),
                Err(outcome) => {
                    let resume = matches!(outcome, RunOutcome::Failed(_)).then(|| Resume {
                        from: i,
                        vars: self.vars(),
                        env: self.env(),
                        written: self.written.clone(),
                    });
                    return (outcome, resume);
                }
            }
        }
        (RunOutcome::Ok, None)
    }

    /// One top-level step: Ok(true) to go on, Ok(false) to stop quietly, Err to end the run.
    fn run_step(&mut self, i: usize, s: &Step) -> Result<bool, RunOutcome> {
        if let Ok(mut idx) = self.index.lock() {
            *idx = i;
        }
        let t0 = Instant::now();
        self.event(RunEvent::StepStarted { index: i });
        let mut log = history::StepLog::new(self.history.as_ref().and_then(|h| h.step_log(i)));
        let finish = |me: &mut Runner, outcome: StepOutcome| {
            let duration = t0.elapsed();
            let (word, msg) = match &outcome {
                StepOutcome::Ok => ("ok", None),
                StepOutcome::Skipped => ("skipped", None),
                StepOutcome::Warned(m) => ("warned", Some(m.clone())),
                StepOutcome::Failed(m) => ("failed", Some(m.clone())),
                StepOutcome::Cancelled => ("cancelled", None),
            };
            if let Some(r) = me.record.steps.get_mut(i) {
                r.outcome = word.into();
                r.duration_ms = duration.as_millis() as u64;
                r.message = msg;
            }
            me.event(RunEvent::StepFinished {
                index: i,
                outcome,
                duration,
            });
        };
        if let Some(cond) = &s.if_ {
            match self.eval(cond) {
                Ok(v) if !v.truthy() => {
                    finish(self, StepOutcome::Skipped);
                    return Ok(true);
                }
                Ok(_) => {}
                Err(e) => {
                    let msg = format!("the if= condition failed: {e}");
                    finish(self, StepOutcome::Failed(msg.clone()));
                    return Err(RunOutcome::Failed(msg));
                }
            }
        }
        let result = self.execute(i, s, &mut log);
        match result {
            Ok(Flow::Next(p)) => {
                self.store(s, p);
                finish(self, StepOutcome::Ok);
                Ok(true)
            }
            Ok(Flow::Stop) => {
                finish(self, StepOutcome::Ok);
                Ok(false)
            }
            Err(_) if self.cancel.is_set() => {
                finish(self, StepOutcome::Cancelled);
                Err(RunOutcome::Cancelled)
            }
            Err(e) => {
                let msg = match &s.message {
                    Some(m) => self.text_with(m, &[("error", &e)]).unwrap_or(e.clone()),
                    None => e.clone(),
                };
                history::append(&mut log, format!("\n{msg}\n").as_bytes());
                match s.on_error {
                    OnError::Abort => {
                        finish(self, StepOutcome::Failed(msg.clone()));
                        Err(RunOutcome::Failed(msg))
                    }
                    OnError::Warn => {
                        if let Ok(mut u) = self.ui.lock() {
                            u.warn(&msg);
                        }
                        finish(self, StepOutcome::Warned(msg));
                        Ok(true)
                    }
                    OnError::Continue => {
                        finish(self, StepOutcome::Warned(msg));
                        Ok(true)
                    }
                }
            }
        }
    }

    /// Keep a step's result for later steps.
    fn store(&self, s: &Step, p: Produced) {
        let mut vars = self.vars();
        for (k, v) in p.names {
            vars.values.insert(k, v);
        }
        if let Some(id) = &s.id {
            if let Some(v) = p.value {
                vars.values.insert(id.clone(), v);
            }
            if !p.fields.is_empty() {
                vars.fields.insert(id.clone(), p.fields);
            }
        }
        self.set_vars(vars);
    }

    /// An add-on step (N21): its settings are worked out here, then its body runs in a scope of
    /// its own, where `{name}` is a setting; its results come back as `{id.result}`.
    fn use_step(
        &mut self,
        i: usize,
        u: &UseStep,
        log: &mut history::StepLog,
    ) -> Result<Flow, String> {
        let def = u.def.clone();
        needs_met(&def, &self.cx(), self.effects.as_ref())?;
        let settings = self.settings_of(u)?;
        let saved = self.vars();
        self.set_vars(Snapshot {
            values: settings,
            fields: BTreeMap::new(),
        });
        let ran = match &def.body {
            Body::Does(steps) => self.run_inner(i, steps, log),
            Body::Script(src) => self.lua(log, |vm| {
                vm.exec_value(src, None).map(Some).map_err(|e| e.message)
            }),
            Body::BuiltIn(spec) => Err(format!(
                "the built-in step `{}` can't be used this way",
                spec.name
            )),
        };
        let scope = self.vars();
        self.set_vars(saved);
        let mut out = Produced::default();
        let returned = match ran {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(Flow::Stop),
            Err(e) => {
                // What it printed, for the add-on's explanations ("would be overwritten").
                return Err(explain(&def, e, &log.text()));
            }
        };
        for r in &def.results {
            let v = match &returned {
                Dyn::Map(m) => m.get(&r.id).cloned(),
                _ => None,
            }
            .or_else(|| scope.values.get(&r.id).cloned());
            if let Some(v) = v {
                out.fields.insert(r.id.clone(), v);
            }
        }
        Ok(Flow::Next(out))
    }

    /// An add-on step's settings, worked out in the caller's scope and typed by their kind.
    fn settings_of(&self, u: &UseStep) -> Result<BTreeMap<String, Dyn>, String> {
        let mut out = BTreeMap::new();
        for s in &u.def.settings {
            let given = u.settings.iter().find(|(k, _)| *k == s.id).map(|(_, v)| v);
            let v = match given {
                // Text takes a list one item per line.
                Some(v) if matches!(s.kind, SettingKind::Text | SettingKind::LongText) => {
                    typed_setting(s, &self.text_lines(v)?)?
                }
                Some(v) => {
                    // Just a list an earlier step found (`{audit.to-fix}`): the step gets the
                    // list, not its items joined into text (a path may hold a comma).
                    let list = match v {
                        Val::Text(t)
                            if !matches!(s.kind, SettingKind::TickBox | SettingKind::Number) =>
                        {
                            whole_value(t, &self.vars()).filter(|d| matches!(d, Dyn::List(_)))
                        }
                        _ => None,
                    };
                    match list {
                        Some(l) => l,
                        None => typed_setting(s, &self.text(v)?)?,
                    }
                }
                None => match &s.starts {
                    Some(d) => d.clone(),
                    None if s.required => {
                        return Err(format!("Fill in \"{}\" for \"{}\"", s.label, u.def.name))
                    }
                    None => Dyn::Str(String::new()),
                },
            };
            out.insert(s.id.clone(), v);
        }
        Ok(out)
    }

    /// The steps of an add-on step's `does { }`: Ok(Some(Nil)) when done, Ok(None) when one
    /// stopped the run quietly.
    fn run_inner(
        &mut self,
        i: usize,
        steps: &[Step],
        log: &mut history::StepLog,
    ) -> Result<Option<Dyn>, String> {
        for st in steps {
            if self.cancel.is_set() {
                return Err("cancelled".into());
            }
            if let Some(cond) = &st.if_ {
                if !self.eval(cond)?.truthy() {
                    continue;
                }
            }
            match self.execute(i, st, log) {
                Ok(Flow::Next(p)) => self.store(st, p),
                Ok(Flow::Stop) => return Ok(None),
                Err(e) => {
                    let msg = match &st.message {
                        Some(m) => self.text_with(m, &[("error", &e)]).unwrap_or(e.clone()),
                        None => e,
                    };
                    match st.on_error {
                        OnError::Abort => return Err(msg),
                        OnError::Warn => {
                            if let Ok(mut u) = self.ui.lock() {
                                u.warn(&msg);
                            }
                        }
                        OnError::Continue => self.log_line(i, msg, log),
                    }
                }
            }
        }
        Ok(Some(Dyn::Nil))
    }

    fn log_line(&self, i: usize, text: String, log: &mut history::StepLog) {
        history::append(log, format!("{text}\n").as_bytes());
        self.event(RunEvent::Log { index: i, text });
    }

    /// Runs a step's Lua with the step's log lent to it, so what the script runs and logs is
    /// in the step's log like a run step's output: saved with the run, and read by an add-on's
    /// explanations.
    fn lua<T>(
        &self,
        log: &mut history::StepLog,
        f: impl FnOnce(&Vm) -> Result<T, String>,
    ) -> Result<T, String> {
        let vm = self.vm.as_ref().ok_or("the Lua VM couldn't start")?;
        if let Ok(mut lent) = self.lua_log.lock() {
            *lent = std::mem::take(log);
        }
        let ran = f(vm);
        if let Ok(mut lent) = self.lua_log.lock() {
            *log = std::mem::take(&mut *lent);
        }
        ran
    }

    fn execute(&mut self, i: usize, s: &Step, log: &mut history::StepLog) -> Result<Flow, String> {
        let mut out = Produced::default();
        match &s.kind {
            StepKind::Run(r) => {
                let program = self.text(&r.program)?;
                let args = r
                    .args
                    .iter()
                    .map(|a| self.text(a))
                    .collect::<Result<Vec<_>, _>>()?;
                let raw = r.command_line.as_ref().map(|c| self.text(c)).transpose()?;
                let cwd = r.cwd.as_ref().map(|c| self.text(c)).transpose()?;
                let stdin = r.stdin.as_ref().map(|c| self.text(c)).transpose()?;
                let env = r
                    .env
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.text(v)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                let cmd = CommandSpec {
                    program: program.clone(),
                    args,
                    raw_command_line: raw,
                    cwd: cwd.map(Into::into),
                    env,
                    stdin,
                    timeout: r.timeout_ms.map(Duration::from_millis),
                };
                self.log_line(i, format!("> {}", cmd.display()), log);
                if !r.wait {
                    let cx = self.cx();
                    let p = self.effects.start(
                        &cx,
                        &cmd,
                        IoMode::Detached,
                        self.term,
                        Box::new(|_| {}),
                    )?;
                    let p: Arc<dyn Proc> = Arc::from(p);
                    out.fields.insert("pid".into(), Dyn::Num(p.pid() as f64));
                    if let Some(id) = &s.id {
                        self.procs.insert(id.clone(), p);
                    }
                    return Ok(Flow::Next(out));
                }
                let io = match r.io {
                    Io::Auto if !self.pty => IoMode::Pipe,
                    Io::Auto => IoMode::Auto,
                    Io::Pty => IoMode::Pty,
                    Io::Pipe => IoMode::Pipe,
                };
                let re = r
                    .fail_if_output
                    .as_ref()
                    .map(|p| regex::Regex::new(p).map_err(|e| format!("fail-if-output: {e}")))
                    .transpose()?;
                let cx = self.cx();
                let code = run_process(
                    &self.effects,
                    &cx,
                    &cmd,
                    io,
                    self.term,
                    i,
                    &self.ui,
                    &self.cancel,
                    re.as_ref(),
                    log,
                )?;
                out.fields.insert("exit_code".into(), Dyn::Num(code as f64));
                if !r.expect_exit.accepts(code as i64) {
                    return Err(format!(
                        "`{program}` exited with code {}",
                        exit_code_text(code)
                    ));
                }
            }
            StepKind::Check {
                program,
                args,
                cwd,
                expect_exit,
            } => {
                let program = self.text(program)?;
                let cmd = CommandSpec {
                    program: program.clone(),
                    args: args
                        .iter()
                        .map(|a| self.text(a))
                        .collect::<Result<_, _>>()?,
                    cwd: cwd
                        .as_ref()
                        .map(|c| self.text(c))
                        .transpose()?
                        .map(Into::into),
                    ..Default::default()
                };
                self.log_line(i, format!("check: {}", cmd.display()), log);
                let cancel = self.cancel.clone();
                let r = self
                    .effects
                    .capture(&self.cx(), &cmd, &move || cancel.is_set())?;
                history::append(log, r.stdout.as_bytes());
                out.fields
                    .insert("exit_code".into(), Dyn::Num(r.exit_code as f64));
                if !expect_exit.accepts(r.exit_code as i64) {
                    return Err(format!(
                        "the check `{}` failed (exit code {})",
                        cmd.display(),
                        exit_code_text(r.exit_code)
                    ));
                }
            }
            StepKind::Capture {
                program,
                args,
                cwd,
                expect_exit,
            } => {
                let cmd = CommandSpec {
                    program: self.text(program)?,
                    args: args
                        .iter()
                        .map(|a| self.text(a))
                        .collect::<Result<_, _>>()?,
                    cwd: cwd
                        .as_ref()
                        .map(|c| self.text(c))
                        .transpose()?
                        .map(Into::into),
                    ..Default::default()
                };
                self.log_line(i, format!("capture: {}", cmd.display()), log);
                let cancel = self.cancel.clone();
                let r = self
                    .effects
                    .capture(&self.cx(), &cmd, &move || cancel.is_set())?;
                history::append(log, r.stdout.as_bytes());
                let output = r.stdout.trim_end_matches(['\r', '\n']).to_string();
                out.fields
                    .insert("exit_code".into(), Dyn::Num(r.exit_code as f64));
                out.fields.insert("output".into(), Dyn::Str(output.clone()));
                out.value = Some(Dyn::Str(output));
                if !expect_exit.accepts(r.exit_code as i64) {
                    return Err(format!(
                        "`{}` stopped with an error (exit code {})",
                        cmd.display(),
                        exit_code_text(r.exit_code)
                    ));
                }
            }
            StepKind::Use(u) => return self.use_step(i, u, log),
            StepKind::EnvFrom { script, args } => {
                let script = self.text(script)?;
                let args: Vec<String> = args
                    .iter()
                    .map(|a| self.text(a))
                    .collect::<Result<_, _>>()?;
                let change = self.effects.env_from(&self.cx(), &script, &args)?;
                self.log_line(
                    i,
                    format!(
                        "{} variables set, {} removed",
                        change.set.len(),
                        change.removed.len()
                    ),
                    log,
                );
                if let Ok(mut cx) = self.cx.lock() {
                    change.apply(&mut cx.env);
                    if let Some(vm) = &self.vm {
                        vm.set_env(cx.env.clone());
                    }
                }
            }
            StepKind::Set { name, value } => {
                let v = match value {
                    Val::Lua(src) => self.eval(src)?,
                    Val::Text(_) => Dyn::Str(self.text(value)?),
                };
                out.names.push((name.clone(), v));
            }
            StepKind::SetControl { control, value } => {
                let v = match value {
                    Val::Lua(src) => self.eval(src)?,
                    Val::Text(_) => Dyn::Str(self.text(value)?),
                };
                if let Ok(mut u) = self.ui.lock() {
                    u.set_control(control, Some(v.clone()));
                }
                out.names.push((control.clone(), v));
            }
            StepKind::Reset { control } => {
                if let Ok(mut u) = self.ui.lock() {
                    u.set_control(control, None);
                }
            }
            StepKind::Put {
                control,
                step,
                result,
            } => {
                // Nothing to put when the step didn't run or gave nothing back.
                let Some(v) = self
                    .vars()
                    .fields
                    .get(step)
                    .and_then(|f| f.get(result))
                    .cloned()
                else {
                    return Ok(Flow::Next(out));
                };
                let choice = self
                    .config
                    .control(control)
                    .is_some_and(|c| matches!(c.kind, ControlKind::Choice { .. }));
                if let Ok(mut u) = self.ui.lock() {
                    match (&v, choice) {
                        (Dyn::List(items), true) => {
                            u.set_choices(control, items.iter().map(Dyn::to_text).collect())
                        }
                        _ => u.set_control(control, Some(v.clone())),
                    }
                }
            }
            StepKind::Copy {
                from,
                to,
                overwrite,
                ask_first,
            } => {
                let (f, t) = (self.text(from)?, self.text(to)?);
                let cx = self.cx();
                if *ask_first && self.effects.exists(&cx, &t) && !self.effects.is_dry() {
                    let replace = self
                        .ui
                        .lock()
                        .map(|mut u| {
                            u.confirm(
                                &format!("{t} is already there. Replace it?"),
                                "Replace",
                                "Keep the old one",
                            )
                        })
                        .unwrap_or(false);
                    if !replace {
                        self.log_line(i, format!("kept the old {t}"), log);
                        return Ok(Flow::Next(out));
                    }
                }
                if !overwrite
                    && !to.as_literal().is_some_and(|x| x.ends_with(['/', '\\']))
                    && self.effects.exists(&cx, &t)
                {
                    return Err(format!("{t} is already there (overwrite=#false)"));
                }
                self.log_line(i, format!("copy {f} to {t}"), log);
                self.effects.copy(&cx, &f, &t)?;
            }
            StepKind::Move {
                from,
                to,
                overwrite,
            } => {
                let (f, t) = (self.text(from)?, self.text(to)?);
                let cx = self.cx();
                if self.effects.exists(&cx, &t) {
                    if *overwrite {
                        self.effects.delete(&cx, &t)?;
                    } else {
                        return Err(format!("{t} is already there (overwrite=#false)"));
                    }
                }
                self.log_line(i, format!("move {f} to {t}"), log);
                self.effects.rename(&cx, &f, &t)?;
            }
            StepKind::Delete {
                path,
                missing_ok,
                to_trash,
            } => {
                let p = self.text(path)?;
                let existed = if *to_trash {
                    self.effects.trash(&self.cx(), &p)?
                } else {
                    self.effects.delete(&self.cx(), &p)?
                };
                if !existed && !missing_ok {
                    return Err(format!("{p} isn't there"));
                }
                self.log_line(
                    i,
                    if existed {
                        format!("deleted {p}")
                    } else {
                        format!("{p} wasn't there")
                    },
                    log,
                );
            }
            StepKind::Mkdir { path } => {
                let p = self.text(path)?;
                self.effects.mkdir(&self.cx(), &p)?;
            }
            StepKind::FindFile(f) => {
                let vars = self.vars();
                let cx = self.cx();
                let effects = self.effects.clone();
                let dir = self.config.dir.clone();
                let found =
                    crate::eval::find_file(effects.as_ref(), &cx, &dir, f, &vars, &mut |v, s| {
                        match v {
                            Val::Text(t) => Ok(render(t, s, &cx.env)),
                            Val::Lua(src) => self.eval(src).map(|d| d.to_text()),
                        }
                    })?;
                self.log_line(i, format!("found: {}", found.to_text()), log);
                out.value = Some(found);
            }
            StepKind::ReadFile { path } => {
                let p = self.text(path)?;
                let text = self.effects.read_file(&self.cx(), &p)?;
                if let Ok(j) = serde_json::from_str::<serde_json::Value>(&text) {
                    out.fields.insert("json".into(), Dyn::from_json(&j));
                }
                out.fields.insert("text".into(), Dyn::Str(text.clone()));
                out.value = Some(Dyn::Str(text));
            }
            StepKind::WriteFile {
                path,
                content,
                format,
                newline,
                create_dirs,
                heading,
                append,
                keep,
                item,
            } => {
                let p = self.text(path)?;
                // A list found earlier: counted beside the heading, and "none" when empty.
                let list = match content {
                    Val::Text(t) => match whole_value(t, &self.vars()) {
                        Some(Dyn::List(l)) => Some(l.len()),
                        _ => None,
                    },
                    Val::Lua(_) => None,
                };
                let mut text = match list {
                    Some(0) => "none".to_string(),
                    _ => self.text_lines(content)?,
                };
                text = match format {
                    WriteFormat::Text => text,
                    WriteFormat::JsonPretty | WriteFormat::JsonCompact => {
                        let j: serde_json::Value = serde_json::from_str(&text)
                            .map_err(|e| format!("the content isn't JSON: {e}"))?;
                        if *format == WriteFormat::JsonPretty {
                            serde_json::to_string_pretty(&j).map_err(|e| e.to_string())?
                        } else {
                            serde_json::to_string(&j).map_err(|e| e.to_string())?
                        }
                    }
                };
                let target = self.cx().resolve(&p);
                // Adding: after what this run wrote there (or, kept, what any run wrote).
                let adding = *append && (*keep || self.written.contains(&target));
                if let Some(h) = heading {
                    let mut h = self.text(h)?;
                    if let Some(n @ 1..) = list.filter(|_| !h.is_empty()) {
                        let (one, many) = item;
                        h = format!("{h} ({n} {})", if n == 1 { one } else { many });
                    }
                    if !h.is_empty() {
                        let gap = if adding { "\n" } else { "" };
                        text = format!("{gap}{h}\n{text}");
                    }
                }
                // Added pieces are whole lines, so the next one starts on a line of its own.
                if *append && !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text = match newline {
                    Newline::Keep => text,
                    Newline::Lf => text.replace("\r\n", "\n"),
                    Newline::Crlf => text.replace("\r\n", "\n").replace('\n', "\r\n"),
                };
                let cx = self.cx();
                if !create_dirs {
                    if let Some(dir) = cx.resolve(&p).parent() {
                        if !dir.exists() && !self.effects.is_dry() {
                            return Err(format!("the folder {} doesn't exist", dir.display()));
                        }
                    }
                }
                if adding {
                    self.log_line(i, format!("add to {p}"), log);
                    self.effects.append_file(&cx, &p, &text)?;
                } else {
                    self.log_line(i, format!("write {p}"), log);
                    self.effects.write_file(&cx, &p, &text)?;
                }
                self.written.insert(target);
            }
            StepKind::HttpGet {
                url,
                timeout_ms,
                extract,
                headers,
            } => {
                let u = self.text(url)?;
                let hs = headers
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.text(v)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                self.log_line(i, format!("GET {u}"), log);
                let r = self.effects.http_get(
                    s.id.as_deref(),
                    &u,
                    &hs,
                    Duration::from_millis(*timeout_ms),
                )?;
                out.fields
                    .insert("status".into(), Dyn::Num(r.status as f64));
                out.fields.insert("body".into(), Dyn::Str(r.body.clone()));
                if !extract.is_empty() {
                    let j: serde_json::Value = serde_json::from_str(&r.body)
                        .map_err(|e| format!("the response isn't JSON: {e}"))?;
                    out.fields.insert("json".into(), Dyn::from_json(&j));
                    for x in extract {
                        match super::json_path::get(&j, &x.path) {
                            Some(v) => {
                                let d = match v {
                                    serde_json::Value::String(s) => Dyn::Str(s.clone()),
                                    other => Dyn::from_json(other),
                                };
                                self.log_line(i, format!("{} = {}", x.name, d.to_text()), log);
                                out.names.push((x.name.clone(), d));
                            }
                            None if x.required => {
                                return Err(format!("the response has no `{}`", x.path))
                            }
                            None => {}
                        }
                    }
                }
            }
            StepKind::Ping { host, timeout_ms } => {
                let h = self.text(host)?;
                let r = self.effects.ping(&h, *timeout_ms)?;
                self.log_line(i, format!("{} answered in {} ms", r.address, r.ms), log);
                out.fields.insert("ms".into(), Dyn::Num(r.ms as f64));
                out.fields.insert("address".into(), Dyn::Str(r.address));
            }
            StepKind::Open { target } => {
                let t = self.text(target)?;
                let resolved = if t.contains("://") {
                    t
                } else {
                    self.cx().resolve(&t).display().to_string()
                };
                self.effects.open(&resolved)?;
            }
            StepKind::Reveal { path } => {
                let p = self.text(path)?;
                self.effects.reveal(&self.cx(), &p)?;
            }
            StepKind::Message { text, level } => {
                let t = self.text(text)?;
                if let Ok(mut u) = self.ui.lock() {
                    u.message(&t, *level);
                }
            }
            StepKind::Confirm { text, yes, no } => {
                let t = self.text(text)?;
                let ok = self
                    .ui
                    .lock()
                    .map(|mut u| u.confirm(&t, yes, no))
                    .unwrap_or(false);
                if !ok {
                    self.log_line(i, "No: the run ends here".into(), log);
                    return Ok(Flow::Stop);
                }
            }
            StepKind::Ask {
                name,
                prompt,
                default,
                password,
            } => {
                let p = self.text(prompt)?;
                let d = default.as_ref().map(|d| self.text(d)).transpose()?;
                let answer = self
                    .ui
                    .lock()
                    .ok()
                    .and_then(|mut u| u.ask(&p, d.as_deref(), *password));
                match answer {
                    Some(a) => out.names.push((name.clone(), Dyn::Str(a))),
                    None => {
                        self.log_line(i, "Cancelled: the run ends here".into(), log);
                        return Ok(Flow::Stop);
                    }
                }
            }
            StepKind::Wait { ms } => {
                let until = Instant::now() + Duration::from_millis(*ms);
                while Instant::now() < until {
                    if self.cancel.is_set() {
                        return Err("cancelled".into());
                    }
                    std::thread::sleep(Duration::from_millis(20).min(until - Instant::now()));
                }
            }
            StepKind::Log { text, .. } => {
                let t = self.text(text)?;
                self.log_line(i, t, log);
            }
            StepKind::Fail { text } => return Err(self.text(text)?),
            StepKind::Stop => return Ok(Flow::Stop),
            StepKind::Lua { code } => {
                self.lua(log, |vm| vm.exec(code, None).map_err(|e| e.message))?;
            }
            StepKind::Args(spec) => {
                let (line, list) = super::args::build(spec, self)?;
                self.log_line(i, format!("{} = {line}", spec.name), log);
                out.names.push((spec.name.clone(), Dyn::Str(line)));
                out.fields.insert(
                    "list".into(),
                    Dyn::List(list.into_iter().map(Dyn::Str).collect()),
                );
            }
            StepKind::SetProcessVolume {
                run,
                volume,
                timeout_ms,
            } => {
                let p = self
                    .procs
                    .get(run)
                    .cloned()
                    .ok_or_else(|| format!("the run step `{run}` hasn't started a program"))?;
                let v: f32 = self
                    .text(volume)?
                    .trim()
                    .parse()
                    .map_err(|_| "the volume must be a number from 0 to 100".to_string())?;
                let cancel = self.cancel.clone();
                let alive_p = p.clone();
                self.effects.set_process_volume(
                    p.pid(),
                    (v / 100.0).clamp(0.0, 1.0),
                    Duration::from_millis(*timeout_ms),
                    &move || alive_p.try_wait().is_none(),
                    &move || cancel.is_set(),
                )?;
                self.log_line(i, format!("volume set to {v}%"), log);
            }
        }
        Ok(Flow::Next(out))
    }

    fn record_history(&mut self, outcome: &RunOutcome) {
        self.record.duration_ms = self.started.elapsed().as_millis() as u64;
        self.record.outcome = match outcome {
            RunOutcome::Ok | RunOutcome::Stopped => "ok",
            RunOutcome::Failed(_) => "failed",
            RunOutcome::Cancelled => "cancelled",
        }
        .into();
        if let Some(h) = &self.history {
            h.write_record(&self.record);
        }
    }
}

impl super::args::ArgsScope for Runner {
    fn render(&mut self, v: &Val) -> Result<Option<String>, String> {
        // A value that is only a placeholder naming something missing is itself missing.
        if let Val::Text(t) = v {
            if let [Part::Ref { name, field, .. }] = t.parts.as_slice() {
                let vars = self.vars();
                let present = match field {
                    None => vars.values.get(name).is_some_and(|d| *d != Dyn::Nil),
                    Some(f) => vars
                        .fields
                        .get(name)
                        .is_some_and(|m| m.keys().any(|k| k.eq_ignore_ascii_case(f))),
                };
                if !present {
                    return Ok(None);
                }
            }
        }
        self.text(v).map(Some)
    }

    fn condition(&mut self, src: &LuaSrc) -> Result<bool, String> {
        self.eval(src).map(|d| d.truthy())
    }

    fn bag(&mut self, source: &str) -> Option<Vec<(String, String)>> {
        if let Some(d) = self.config.data_block(source) {
            return Some(d.bag.clone());
        }
        let c = self.config.control(source)?;
        let ControlKind::Choice { options, .. } = &c.kind else {
            return None;
        };
        let vars = self.vars();
        let chosen = vars.values.get(source)?.to_text();
        // Static options keep their written order; computed ones come from the fields.
        if let Some(o) = options.iter().find(|o| o.value == chosen) {
            return Some(o.bag.clone());
        }
        vars.fields.get(source).map(|m| {
            m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "name" | "value" | "label"))
                .map(|(k, v)| (k.clone(), v.to_text()))
                .collect()
        })
    }
}

/// A step's label when it has none: what it does, briefly.
/// An add-on step's row: its sentence (`says`) with its settings filled in, or its name.
fn use_label(u: &UseStep, text: &impl Fn(&Val) -> String) -> String {
    let Some(says) = &u.def.says else {
        return u.def.name.clone();
    };
    let Ok(t) = crate::config::template::Template::parse(says) else {
        return says.clone();
    };
    let mut snap = Snapshot::default();
    for s in &u.def.settings {
        let v = match u.settings.iter().find(|(k, _)| *k == s.id) {
            Some((_, v)) => Dyn::Str(text(v)),
            None => s.starts.clone().unwrap_or_default(),
        };
        snap.values.insert(s.id.clone(), v);
    }
    render(&t, &snap, &BTreeMap::new())
}

/// Whether the computer has what an add-on step needs; the plain message when it doesn't.
pub fn needs_met(def: &StepDef, cx: &Context, effects: &dyn Effects) -> Result<(), String> {
    for n in &def.needs {
        match n {
            Need::App { name, help } => {
                if !effects.has_app(cx, name) {
                    let mut msg = format!("This step needs {name}, which isn't installed.");
                    if let Some(h) = help {
                        msg.push(' ');
                        msg.push_str(h);
                    }
                    return Err(msg);
                }
            }
            Need::Os(os) => {
                let here = crate::config::catalog::Os::current();
                if *os != here {
                    return Err(format!(
                        "\"{}\" only works on {}.",
                        def.name,
                        if *os == crate::config::catalog::Os::Windows {
                            "Windows"
                        } else {
                            "macOS"
                        }
                    ));
                }
            }
        }
    }
    Ok(())
}

/// A setting's text as the kind of value the step expects.
fn typed_setting(s: &SettingDef, text: &str) -> Result<Dyn, String> {
    Ok(match s.kind {
        SettingKind::TickBox => Dyn::Bool(matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "1" | "on"
        )),
        SettingKind::Number => match text.trim() {
            "" => Dyn::Nil,
            t => Dyn::Num(
                t.parse::<f64>()
                    .map_err(|_| format!("{} should be a number, not \"{t}\"", s.label))?,
            ),
        },
        _ => Dyn::Str(text.to_string()),
    })
}

/// A failed add-on step's problem, with the add-on's advice when it knows the problem.
/// An add-on's plain advice for a problem, when one of its phrases is in the problem or in what
/// the step printed.
fn explain(def: &StepDef, problem: String, printed: &str) -> String {
    let said = format!("{problem}\n{printed}").to_lowercase();
    for (phrase, advice) in &def.explain {
        if said.contains(&phrase.to_lowercase()) {
            return format!("{advice}\n({problem})");
        }
    }
    problem
}

pub fn default_label(s: &Step, text: impl Fn(&Val) -> String) -> String {
    match &s.kind {
        StepKind::Run(r) => {
            let mut l = text(&r.program);
            for a in r.args.iter().take(3) {
                l.push(' ');
                l.push_str(&text(a));
            }
            if r.args.len() > 3 {
                l.push_str(" ...");
            }
            l
        }
        StepKind::Check { program, .. } => format!("Check: {}", text(program)),
        StepKind::Capture { program, .. } => format!("Read the output of {}", text(program)),
        StepKind::Use(u) => use_label(u, &text),
        StepKind::Log { text: t, .. } => text(t),
        StepKind::HttpGet { url, .. } => format!("Fetch {}", text(url)),
        StepKind::Ping { host, .. } => format!("Ping {}", text(host)),
        StepKind::Ask { prompt, .. } => text(prompt),
        StepKind::Confirm { text: t, .. } | StepKind::Message { text: t, .. } => text(t),
        StepKind::Copy { from, to, .. } => format!("Copy {} to {}", text(from), text(to)),
        StepKind::Move { from, to, .. } => format!("Move {} to {}", text(from), text(to)),
        StepKind::Delete { path, .. } => format!("Delete {}", text(path)),
        StepKind::Mkdir { path } => format!("Create {}", text(path)),
        StepKind::WriteFile { path, .. } => format!("Write {}", text(path)),
        StepKind::ReadFile { path } => format!("Read {}", text(path)),
        StepKind::Open { target } => format!("Open {}", text(target)),
        StepKind::Reveal { path } => format!("Show {}", text(path)),
        StepKind::Wait { ms } => format!("Wait {} s", crate::dynval::fmt_num(*ms as f64 / 1000.0)),
        other => other.name().to_string(),
    }
}

/// Convenience for tests and the CLI: load and resolve paths for a file's folder.
pub fn config_dir_of(p: &Path) -> std::path::PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_default()
}

#[cfg(test)]
mod tests;

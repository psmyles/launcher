//! The run view (M6): the log beside a tool's window, and the dialogs a run opens. One console
//! holds every run since the log opened: each run's title, a heading for each step, what the
//! step printed, and how it ended, written in as lines of their own. A bar above it shows the
//! newest run's status and buttons.
//!
//! The run happens on the runner's thread (`engine::runner`). [`WindowRunUi`] is its side of the
//! window: it feeds the log right there (the emulator parses on the run thread, the window only
//! draws), records the rest in a shared [`RunState`], and asks for a repaint. A dialog
//! (`message`, `confirm`, `ask`, a warning) parks a question in the state and blocks the run until
//! the window answers it (N15: warnings are modal and pause the run).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, RichText, Sense, Ui, Vec2};
use launcher_core::config::{Config, Level, Val};
use launcher_core::dynval::Dyn;
use launcher_core::engine::effects::{DryRunEffects, Effects};
use launcher_core::engine::history::{History, RunRecord};
use launcher_core::engine::runner::{
    self, Resume, RunEvent, RunHandle, RunOutcome, RunSpec, RunUi, StepInfo, StepOutcome,
};
use launcher_core::lua::Snapshot;
use launcher_core::platform::trust::Trusted;
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::console::{self, SharedTerm};
use crate::ui::theme;

/// Repaint pace while output flows.
const FRAME: Duration = Duration::from_millis(16);
/// How much of each step's output is kept for "Copy details for help".
const TAIL: usize = 32 * 1024;

/// The log's own lines, as SGR styles: a run's title, a step's heading, a run that went well, a
/// problem, a warning, and something that didn't happen.
const TITLE: &str = "4;97";
/// A light blue (256-colour 111, #87afff), as bright as the green and red beside it; the
/// palette's own blues are darker, and programs' blue stays as it is.
const HEADING: &str = "38;5;111";
const DONE: &str = "92";
const WRONG: &str = "91";
const WARNING: &str = "93";
const QUIET: &str = "2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Running,
    Ok,
    Skipped,
    Warned,
    Failed,
    Cancelled,
}

/// What "Copy details for help" copies: the tool, the button, the step that didn't work, the
/// problem, the end of that step's output, and the versions.
fn help_details(inputs: &RunInputs, button: &str, problem: &str, steps: &[StepView]) -> String {
    let failed = steps.iter().rfind(|v| v.status == Status::Failed);
    let config = &inputs.config;
    let tool = match &config.window.title {
        Val::Text(t) if !t.raw.trim().is_empty() => {
            launcher_core::eval::render(t, &inputs.snapshot, &inputs.env)
        }
        _ => config
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    };
    let mut out = format!("Tool: {tool}\nButton: {button}\n");
    if let Some(v) = failed {
        out.push_str(&format!("Step: {}\n", v.info.label));
    }
    out.push_str(&format!("Problem: {problem}\n"));
    out.push_str(&format!(
        "Launcher {} on {}\n",
        crate::product::VERSION,
        if cfg!(windows) { "Windows" } else { "macOS" }
    ));
    if let Some(v) = failed {
        let text = runner::strip_ansi(&String::from_utf8_lossy(&v.tail));
        // A line a program rewrote in place (progress) as it ended up.
        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.rsplit('\r').next().unwrap_or(l))
            .filter(|l| !l.trim().is_empty())
            .collect();
        let tail = &lines[lines.len().saturating_sub(40)..];
        if !tail.is_empty() {
            out.push_str("\nThe last lines it printed:\n");
            for l in tail {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    out
}

pub struct StepView {
    pub info: StepInfo,
    pub status: Status,
    message: Option<String>,
    /// The end of what the step printed (for "Copy details for help").
    tail: Vec<u8>,
}

impl StepView {
    /// Keep `bytes` of output, dropping the oldest beyond [`TAIL`].
    fn keep(&mut self, bytes: &[u8]) {
        self.tail.extend_from_slice(bytes);
        if self.tail.len() > TAIL {
            self.tail.drain(..self.tail.len() - TAIL);
        }
    }
}

pub enum Question {
    Message {
        text: String,
        level: Level,
    },
    Confirm {
        text: String,
        yes: String,
        no: String,
    },
    Ask {
        prompt: String,
        answer: String,
        password: bool,
    },
    Warn {
        text: String,
    },
}

pub enum Answer {
    Ok,
    Yes,
    No,
    Text(Option<String>),
}

pub struct Dialog {
    pub question: Question,
    reply: mpsc::Sender<Answer>,
}

/// What the run thread and the window share.
#[derive(Default)]
pub struct RunState {
    pub steps: Vec<StepView>,
    /// The log, shared by every run since it opened.
    log: SharedTerm,
    pub log_dir: Option<PathBuf>,
    pub finished: Option<(RunOutcome, Option<Resume>)>,
    pub dialog: Option<Dialog>,
    /// `set-control` and `reset` from the run, for the form.
    pub control_edits: Vec<(String, Option<Dyn>)>,
    /// Choices a step filled in (`put`), for the form.
    pub choice_edits: Vec<(String, Vec<String>)>,
    started: Option<Instant>,
    ended: Option<Instant>,
    /// Retry: the earlier run's steps before the one it resumes at.
    carried: Vec<StepView>,
    /// A step's problem is in the log already (the run's end doesn't say it again).
    told: bool,
}

impl RunState {
    /// A line of the log's own.
    fn write(&self, style: &str, text: &str) {
        if let Ok(mut t) = self.log.lock() {
            t.line(style, text);
        }
    }

    /// Whether a step has a heading in the log: instant steps don't, unless all of them are.
    fn has_heading(&self, index: usize) -> bool {
        let all_quiet = self.steps.iter().all(|v| v.info.quiet);
        self.steps
            .get(index)
            .is_some_and(|v| !v.info.quiet || all_quiet)
    }
}

/// The run's side of the window.
struct WindowRunUi {
    state: Arc<Mutex<RunState>>,
    proxy: EventLoopProxy<AppEvent>,
    /// The run's title, for the log once it starts.
    title: Option<String>,
}

impl WindowRunUi {
    fn repaint(&self, after: Duration) {
        let _ = self.proxy.send_event(AppEvent::Repaint(after));
    }

    /// Ask the window and wait for the answer (the run pauses).
    fn ask_window(&self, question: Question) -> Answer {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut s) = self.state.lock() {
            s.dialog = Some(Dialog {
                question,
                reply: tx,
            });
        }
        self.repaint(Duration::ZERO);
        rx.recv().unwrap_or(Answer::No)
    }
}

impl RunUi for WindowRunUi {
    fn event(&mut self, e: RunEvent) {
        let Ok(mut guard) = self.state.lock() else {
            return;
        };
        let s = &mut *guard;
        match e {
            RunEvent::Started { steps, log_dir } => {
                if let Some(title) = self.title.take() {
                    if let Ok(mut t) = s.log.lock() {
                        // A blank line between runs.
                        if t.has_text() {
                            t.line("0", "");
                        }
                        t.line(TITLE, &title);
                        t.follow_newest();
                    }
                }
                s.steps = steps
                    .into_iter()
                    .map(|info| StepView {
                        info,
                        status: Status::Pending,
                        message: None,
                        tail: Vec::new(),
                    })
                    .collect();
                // A Retry keeps what the steps before the failed one did.
                for (i, old) in std::mem::take(&mut s.carried).into_iter().enumerate() {
                    if s.steps.get(i).is_some_and(|v| v.info == old.info) {
                        s.steps[i] = old;
                    }
                }
                s.log_dir = log_dir;
                s.started = Some(Instant::now());
            }
            RunEvent::StepStarted { index } => {
                if s.has_heading(index) {
                    let label = s.steps[index].info.label.clone();
                    s.write(HEADING, &format!("\u{25b8} {label}"));
                }
                if let Some(st) = s.steps.get_mut(index) {
                    st.status = Status::Running;
                }
            }
            RunEvent::Output { index, bytes } => {
                if let Some(st) = s.steps.get_mut(index) {
                    st.keep(&bytes);
                }
                let log = s.log.clone();
                drop(guard);
                if let Ok(mut t) = log.lock() {
                    t.feed(&bytes);
                }
                self.repaint(FRAME);
                return;
            }
            RunEvent::Log { index, text } => {
                if let Some(st) = s.steps.get_mut(index) {
                    st.keep(format!("{text}\n").as_bytes());
                }
                let log = s.log.clone();
                drop(guard);
                if let Ok(mut t) = log.lock() {
                    t.note(&text);
                }
                self.repaint(FRAME);
                return;
            }
            RunEvent::Attached { process, .. } => {
                if let Ok(mut t) = s.log.lock() {
                    t.attach(process);
                }
            }
            RunEvent::StepFinished { index, outcome, .. } => {
                if let Ok(mut t) = s.log.lock() {
                    t.detach();
                }
                // How it ended, when there's more to say than "done" (the run's end says that).
                let line = match &outcome {
                    StepOutcome::Ok | StepOutcome::Cancelled => None,
                    StepOutcome::Skipped if s.has_heading(index) => {
                        Some((QUIET, "Skipped".to_string()))
                    }
                    StepOutcome::Skipped => None,
                    StepOutcome::Warned(m) => Some((WARNING, format!("! {m}"))),
                    StepOutcome::Failed(m) => Some((WRONG, format!("\u{2717} {m}"))),
                };
                if let Some((style, text)) = line {
                    s.write(style, &text);
                }
                s.told |= matches!(outcome, StepOutcome::Failed(_));
                if let Some(st) = s.steps.get_mut(index) {
                    (st.status, st.message) = match outcome {
                        StepOutcome::Ok => (Status::Ok, None),
                        StepOutcome::Skipped => (Status::Skipped, None),
                        StepOutcome::Warned(m) => (Status::Warned, Some(m)),
                        StepOutcome::Failed(m) => (Status::Failed, Some(m)),
                        StepOutcome::Cancelled => (Status::Cancelled, None),
                    };
                }
            }
            RunEvent::Finished { outcome, resume } => {
                // Steps that never started stay as they are; a running one was cut short.
                for st in &mut s.steps {
                    if st.status == Status::Running {
                        st.status = Status::Cancelled;
                    }
                }
                let took = duration_text(s.started.map(|t| t.elapsed()).unwrap_or_default());
                let line = match &outcome {
                    RunOutcome::Ok => Some((DONE, format!("\u{2713} Done in {took}"))),
                    RunOutcome::Stopped => Some((QUIET, "Stopped".to_string())),
                    RunOutcome::Cancelled => Some((WARNING, format!("Stopped after {took}"))),
                    RunOutcome::Failed(_) if s.told => None,
                    RunOutcome::Failed(m) => Some((WRONG, format!("\u{2717} {m}"))),
                };
                if let Some((style, text)) = line {
                    s.write(style, &text);
                }
                s.finished = Some((outcome, resume));
                s.ended = Some(Instant::now());
            }
        }
        drop(guard);
        self.repaint(Duration::ZERO);
    }

    fn message(&mut self, text: &str, level: Level) {
        self.ask_window(Question::Message {
            text: text.to_string(),
            level,
        });
    }

    fn confirm(&mut self, text: &str, yes: &str, no: &str) -> bool {
        matches!(
            self.ask_window(Question::Confirm {
                text: text.to_string(),
                yes: yes.to_string(),
                no: no.to_string(),
            }),
            Answer::Yes
        )
    }

    fn ask(&mut self, prompt: &str, default: Option<&str>, password: bool) -> Option<String> {
        match self.ask_window(Question::Ask {
            prompt: prompt.to_string(),
            answer: default.unwrap_or_default().to_string(),
            password,
        }) {
            Answer::Text(t) => t,
            _ => None,
        }
    }

    fn warn(&mut self, text: &str) {
        self.ask_window(Question::Warn {
            text: text.to_string(),
        });
    }

    fn set_control(&mut self, id: &str, value: Option<Dyn>) {
        if let Ok(mut s) = self.state.lock() {
            s.control_edits.push((id.to_string(), value));
        }
        self.repaint(Duration::ZERO);
    }

    fn set_choices(&mut self, id: &str, choices: Vec<String>) {
        if let Ok(mut s) = self.state.lock() {
            s.choice_edits.push((id.to_string(), choices));
        }
        self.repaint(Duration::ZERO);
    }
}

/// What a run needs to start again (Retry).
#[derive(Clone)]
pub struct RunInputs {
    pub config: Arc<Config>,
    pub action: String,
    pub snapshot: Snapshot,
    pub env: BTreeMap<String, String>,
    pub trusted: Trusted,
    pub dry: bool,
}

/// What the user asked of the run view.
pub enum RunRequest {
    Back,
    Retry(Resume),
}

pub struct RunView {
    pub inputs: RunInputs,
    pub title: String,
    state: Arc<Mutex<RunState>>,
    handle: Option<RunHandle>,
    dry: Option<Arc<DryRunEffects>>,
    /// The action's previous run, for "last time".
    last: Option<RunRecord>,
    /// The finish has been handed to the document.
    reported: bool,
    /// A preview's list of what it would do is in the log.
    listed: bool,
}

impl RunView {
    /// Start the run, writing into `log`. A Retry passes where to resume and the earlier run's
    /// steps before that.
    pub fn start(
        inputs: RunInputs,
        resume: Option<Resume>,
        carried: Vec<StepView>,
        log: SharedTerm,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<RunView, String> {
        let dry = inputs.dry.then(|| Arc::new(DryRunEffects::default()));
        let effects: Arc<dyn Effects> = match &dry {
            Some(d) => d.clone(),
            None => Arc::new(launcher_core::engine::effects::RealEffects),
        };
        let last = History::for_config(&inputs.config.path).and_then(|h| h.last(&inputs.action));
        let title = inputs
            .config
            .action(&inputs.action)
            .and_then(|a| a.label.clone())
            .unwrap_or_else(|| inputs.action.clone());
        let heading = match (inputs.dry, resume.is_some()) {
            (true, _) => format!("{title} (preview)"),
            (false, true) => format!("{title}: trying again"),
            (false, false) => title.clone(),
        };
        let state = Arc::new(Mutex::new(RunState {
            carried,
            log,
            ..Default::default()
        }));
        let handle = runner::start(
            RunSpec {
                config: inputs.config.clone(),
                action: inputs.action.clone(),
                snapshot: inputs.snapshot.clone(),
                env: inputs.env.clone(),
                effects,
                trusted: inputs.trusted,
                resume,
                history: !inputs.dry,
                term_size: (100, 24),
                pty: true,
            },
            Box::new(WindowRunUi {
                state: state.clone(),
                proxy,
                title: Some(heading),
            }),
        )?;
        Ok(RunView {
            inputs,
            title,
            state,
            handle: Some(handle),
            dry,
            last,
            reported: false,
            listed: false,
        })
    }

    /// The steps before `from` (for a Retry to show them as they ended).
    pub fn steps_before(&self, from: usize) -> Vec<StepView> {
        self.state
            .lock()
            .map(|mut s| {
                let n = from.min(s.steps.len());
                s.steps.drain(..n).collect()
            })
            .unwrap_or_default()
    }

    pub fn is_running(&self) -> bool {
        self.state
            .lock()
            .map(|s| s.finished.is_none())
            .unwrap_or(false)
    }

    /// Stop the run (a dialog it waits on is answered No first).
    pub fn cancel(&self) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(d) = s.dialog.take() {
                let _ = d.reply.send(Answer::No);
            }
        }
        if let Some(h) = &self.handle {
            h.cancel();
        }
    }

    /// Choices a step filled in (`put`) since the last call.
    pub fn take_choice_edits(&self) -> Vec<(String, Vec<String>)> {
        self.state
            .lock()
            .map(|mut s| std::mem::take(&mut s.choice_edits))
            .unwrap_or_default()
    }

    /// `set-control`/`reset` the run asked for since the last call.
    pub fn take_control_edits(&self) -> Vec<(String, Option<Dyn>)> {
        self.state
            .lock()
            .map(|mut s| std::mem::take(&mut s.control_edits))
            .unwrap_or_default()
    }

    /// How far the run is, for the taskbar: the share of steps (with a row) that have ended.
    pub fn progress(&self) -> crate::os::Progress {
        use crate::os::Progress;
        let Ok(s) = self.state.lock() else {
            return Progress::Off;
        };
        let rows: Vec<&StepView> = s.steps.iter().filter(|v| !v.info.quiet).collect();
        let done = rows
            .iter()
            .filter(|v| !matches!(v.status, Status::Pending | Status::Running))
            .count();
        let share = if rows.is_empty() {
            0.0
        } else {
            done as f32 / rows.len() as f32
        };
        match &s.finished {
            None if rows.is_empty() => Progress::Busy,
            None => Progress::Part(share),
            Some((RunOutcome::Failed(_), _)) => Progress::Failed(share.max(0.05)),
            Some(_) => Progress::Off,
        }
    }

    /// Once, when the run has ended: how.
    pub fn take_finish(&mut self) -> Option<RunOutcome> {
        if self.reported {
            return None;
        }
        let outcome = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.finished.as_ref().map(|(o, _)| o.clone()))?;
        self.reported = true;
        if let Some(h) = self.handle.take() {
            h.join();
        }
        Some(outcome)
    }

    /// The log beside the window: a bar with this run's status and buttons, then the console
    /// with every run in it.
    pub fn ui(&mut self, ui: &mut Ui) -> Option<RunRequest> {
        let mut request = None;
        let mut s = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return None,
        };
        let running = s.finished.is_none();
        if running {
            // The clock in the bar.
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        // A preview lists what it would have done, once it's over.
        if let (Some(d), Some(_), false) = (&self.dry, &s.finished, self.listed) {
            self.listed = true;
            s.write(HEADING, "What it would do:");
            for l in d.lines() {
                s.write("0", &l);
            }
        }

        // The bar, on lines of its own so it fits beside the window: what runs and how it's
        // going, then the buttons.
        ui.horizontal_wrapped(|ui| {
            let title = if self.inputs.dry {
                format!("{} (preview)", self.title)
            } else {
                self.title.clone()
            };
            ui.label(RichText::new(title).strong());
            // Step n of m: the steps with a heading (instant steps have none).
            let rows: Vec<usize> = (0..s.steps.len()).filter(|i| s.has_heading(*i)).collect();
            let doing = rows
                .iter()
                .position(|i| s.steps[*i].status == Status::Running);
            let status = match &s.finished {
                None => RichText::new(match doing {
                    Some(i) => format!("Doing step {} of {}", i + 1, rows.len()),
                    None => "Working…".to_string(),
                }),
                Some((RunOutcome::Ok, _)) => RichText::new("Done").color(theme::extras(ui).success),
                Some((RunOutcome::Stopped, _)) => RichText::new("Stopped").weak(),
                Some((RunOutcome::Cancelled, _)) => {
                    RichText::new("Stopped").color(ui.visuals().warn_fg_color)
                }
                Some((RunOutcome::Failed(_), _)) => {
                    RichText::new("Didn't work").color(ui.visuals().error_fg_color)
                }
            };
            ui.label(status);
            let elapsed = match (s.started, s.ended) {
                (Some(a), Some(b)) => Some(b - a),
                (Some(a), None) => Some(a.elapsed()),
                _ => None,
            };
            if let Some(e) = elapsed {
                ui.label(RichText::new(duration_text(e)).weak());
            }
            if let (true, Some(last)) = (running, &self.last) {
                ui.label(
                    RichText::new(format!(
                        "last time {}",
                        duration_text(Duration::from_millis(last.duration_ms))
                    ))
                    .weak(),
                );
            }
        });
        ui.horizontal_wrapped(|ui| {
            if running {
                if ui
                    .button("Stop")
                    .on_hover_text("Stop the tool now")
                    .clicked()
                {
                    if let Some(d) = s.dialog.take() {
                        let _ = d.reply.send(Answer::No);
                    }
                    if let Some(h) = &self.handle {
                        h.cancel();
                    }
                }
                return;
            }
            if let Some((RunOutcome::Failed(m), resume)) = &s.finished {
                if let Some(resume) = resume {
                    if ui
                        .button("Try again")
                        .on_hover_text("Start again from the step that didn't work")
                        .clicked()
                    {
                        request = Some(RunRequest::Retry(resume.clone()));
                    }
                }
                if ui
                    .button("Copy details for help")
                    .on_hover_text("Copies what went wrong, to send to someone who can help")
                    .clicked()
                {
                    ui.ctx()
                        .copy_text(help_details(&self.inputs, &self.title, m, &s.steps));
                }
            }
            if let Some(dir) = &s.log_dir {
                if ui
                    .button("Log files")
                    .on_hover_text("Open the folder with this run's log files")
                    .clicked()
                {
                    if let Err(e) = launcher_core::platform::shell::open(&dir.display().to_string())
                    {
                        launcher_core::log_warn!("{e}");
                    }
                }
            }
            if ui
                .button("Close")
                .on_hover_text("Close the log beside the window")
                .clicked()
            {
                request = Some(RunRequest::Back);
            }
        });
        if let Some((RunOutcome::Failed(m), _)) = &s.finished {
            ui.add(egui::Label::new(RichText::new(m).color(ui.visuals().error_fg_color)).wrap());
        }
        theme::gap_small(ui);

        // The log. Typing goes to a program that's waiting for it.
        let log = s.log.clone();
        let typing = running && log.lock().is_ok_and(|t| t.attached().is_some());
        let r = console::view::show(ui, &log, false);
        if typing && !r.has_focus() && ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }

        // The question the run waits on.
        let mut answer = None;
        if let Some(d) = s.dialog.as_mut() {
            let modal = egui::Modal::new(egui::Id::new("run-question")).show(ui.ctx(), |ui| {
                ui.set_max_width(460.0);
                match &mut d.question {
                    Question::Message { text, level } => {
                        let color = match level {
                            Level::Error => Some(ui.visuals().error_fg_color),
                            Level::Warning => Some(ui.visuals().warn_fg_color),
                            _ => None,
                        };
                        let mut t = RichText::new(text.as_str());
                        if let Some(c) = color {
                            t = t.color(c);
                        }
                        ui.label(t);
                        theme::gap(ui);
                        if ui.button("OK").clicked() {
                            answer = Some(Answer::Ok);
                        }
                    }
                    Question::Warn { text } => {
                        ui.label(RichText::new(text.as_str()).color(ui.visuals().warn_fg_color));
                        theme::gap(ui);
                        if ui.button("Continue").clicked() {
                            answer = Some(Answer::Ok);
                        }
                    }
                    Question::Confirm { text, yes, no } => {
                        ui.label(text.as_str());
                        theme::gap(ui);
                        ui.horizontal(|ui| {
                            if ui.button(yes.as_str()).clicked() {
                                answer = Some(Answer::Yes);
                            }
                            if ui.button(no.as_str()).clicked() {
                                answer = Some(Answer::No);
                            }
                        });
                    }
                    Question::Ask {
                        prompt,
                        answer: text,
                        password,
                    } => {
                        ui.label(prompt.as_str());
                        let r = ui.add(
                            egui::TextEdit::singleline(text)
                                .password(*password)
                                .desired_width(420.0),
                        );
                        r.request_focus();
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        theme::gap(ui);
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() || enter {
                                answer = Some(Answer::Text(Some(text.clone())));
                            }
                            if ui.button("Cancel").clicked() {
                                answer = Some(Answer::Text(None));
                            }
                        });
                    }
                }
            });
            if modal.should_close() && answer.is_none() {
                answer = Some(match d.question {
                    Question::Ask { .. } => Answer::Text(None),
                    Question::Confirm { .. } => Answer::No,
                    _ => Answer::Ok,
                });
            }
        }
        if let Some(a) = answer {
            if let Some(d) = s.dialog.take() {
                let _ = d.reply.send(a);
            }
        }
        request
    }
}

/// "0.4 s", "12 s", "3 min 5 s", "1 h 2 min".
pub fn duration_text(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 10.0 {
        format!("{s:.1} s")
    } else if s < 60.0 {
        format!("{} s", s as u64)
    } else if s < 3600.0 {
        let m = s as u64 / 60;
        match s as u64 % 60 {
            0 => format!("{m} min"),
            r => format!("{m} min {r} s"),
        }
    } else {
        let h = s as u64 / 3600;
        format!("{h} h {} min", (s as u64 % 3600) / 60)
    }
}

/// A step's state as a small mark: a ring (waiting), a spinner (running), a dot (done). Drawn at
/// 14 px and scaled to `extras.status_icon_size`.
pub fn status_icon(ui: &mut Ui, status: Status) {
    let ex = theme::extras(ui);
    let k = ex.status_icon_size / 14.0;
    let size = Vec2::splat(ex.status_icon_size);
    if status == Status::Running {
        ui.add_sized(size, egui::Spinner::new().size(12.0 * k));
        return;
    }
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let c = rect.center();
    let v = ui.visuals();
    let (weak, mark) = (v.weak_text_color(), v.panel_fill);
    let painter = ui.painter();
    let at = |x: f32, y: f32| c + Vec2::new(x, y) * k;
    let ring = |color: Color32| {
        painter.circle_stroke(c, 5.0 * k, egui::Stroke::new(1.5, color));
    };
    match status {
        Status::Pending => ring(weak),
        Status::Skipped => {
            ring(weak);
            painter.line_segment([at(-3.5, 3.5), at(3.5, -3.5)], egui::Stroke::new(1.5, weak));
        }
        Status::Ok => {
            painter.circle_filled(c, 6.0 * k, ex.success);
            let s = egui::Stroke::new(1.6, mark);
            painter.line_segment([at(-3.0, 0.2), at(-0.8, 2.6)], s);
            painter.line_segment([at(-0.8, 2.6), at(3.2, -2.4)], s);
        }
        Status::Warned => {
            painter.circle_filled(c, 6.0 * k, v.warn_fg_color);
        }
        Status::Failed => {
            painter.circle_filled(c, 6.0 * k, v.error_fg_color);
            let s = egui::Stroke::new(1.6, mark);
            painter.line_segment([at(-2.5, -2.5), at(2.5, 2.5)], s);
            painter.line_segment([at(-2.5, 2.5), at(2.5, -2.5)], s);
        }
        Status::Cancelled => {
            painter.circle_filled(c, 5.0 * k, weak);
        }
        Status::Running => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_well() {
        assert_eq!(duration_text(Duration::from_millis(400)), "0.4 s");
        assert_eq!(duration_text(Duration::from_secs(12)), "12 s");
        assert_eq!(duration_text(Duration::from_secs(185)), "3 min 5 s");
        assert_eq!(duration_text(Duration::from_secs(720)), "12 min");
        assert_eq!(duration_text(Duration::from_secs(3720)), "1 h 2 min");
    }
}

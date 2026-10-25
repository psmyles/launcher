//! `launcher run FILE`: an action without the window (M4). Output goes to the terminal; prompts
//! read the terminal when there is one. Remembered values are read but never written (N15).
//!
//! ```text
//! launcher run FILE [--action ID] [--set ID=VALUE]... [--preview [--answer ID=@FILE|TEXT|fail]...]
//!                   [--mock] [--trust]
//! ```

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use launcher_core::config::catalog::Os;
use launcher_core::config::{self, Level, LoadOptions};
use launcher_core::dynval::Dyn;
use launcher_core::engine::effects::{DryRunEffects, Effects, Mock, MockEffects, RealEffects};
use launcher_core::engine::runner::{
    self, Resume, RunEvent, RunOutcome, RunSpec, RunUi, StepInfo, StepOutcome,
};
use launcher_core::eval::Evaluator;
use launcher_core::form::Form;
use launcher_core::lua::Snapshot;
use launcher_core::platform::trust;

use crate::cli::{err, out, EXIT_ERRORS, EXIT_OK, EXIT_USAGE};

pub const EXIT_CANCELLED: i32 = 130;

#[derive(Default, Debug)]
pub struct RunArgs {
    pub file: PathBuf,
    pub action: Option<String>,
    pub sets: Vec<(String, String)>,
    pub dry_run: bool,
    pub answers: Vec<(String, String)>,
    pub mock: bool,
    pub trust: bool,
}

pub fn parse(args: &[String]) -> Result<RunArgs, String> {
    let mut r = RunArgs::default();
    let mut it = args.iter();
    let mut file = None;
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--action" => r.action = Some(value("--action")?),
            "--set" => {
                let v = value("--set")?;
                let (k, val) = v
                    .split_once('=')
                    .ok_or_else(|| format!("--set {v}: write it as id=value"))?;
                r.sets.push((k.to_string(), val.to_string()));
            }
            "--answer" => {
                let v = value("--answer")?;
                let (k, val) = v.split_once('=').ok_or_else(|| {
                    format!("--answer {v}: write it as step-id=@file, =text or =fail")
                })?;
                r.answers.push((k.to_string(), val.to_string()));
            }
            // --preview is the app's word for it; --dry-run still works.
            "--preview" | "--dry-run" => r.dry_run = true,
            "--mock" => r.mock = true,
            "--trust" => r.trust = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            f => {
                if file.replace(PathBuf::from(f)).is_some() {
                    return Err(format!("unexpected argument `{f}`"));
                }
            }
        }
    }
    r.file = file.ok_or("run needs a file")?;
    if !r.answers.is_empty() && !r.dry_run {
        return Err("--answer only works with --preview".into());
    }
    Ok(r)
}

/// Set by Ctrl+C; the run is cancelled when it is.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

fn catch_ctrl_c() {
    #[cfg(windows)]
    {
        use windows::core::BOOL;
        use windows::Win32::System::Console::SetConsoleCtrlHandler;
        unsafe extern "system" fn handler(_: u32) -> BOOL {
            INTERRUPTED.store(true, Ordering::SeqCst);
            BOOL(1)
        }
        // SAFETY: a static handler that only sets an atomic.
        let _ = unsafe { SetConsoleCtrlHandler(Some(handler), true) };
    }
    #[cfg(unix)]
    {
        extern "C" fn handler(_: libc::c_int) {
            INTERRUPTED.store(true, Ordering::SeqCst);
        }
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
        unsafe {
            libc::signal(
                libc::SIGINT,
                handler as extern "C" fn(libc::c_int) as libc::sighandler_t,
            )
        };
    }
}

/// The terminal side of a run.
struct Terminal {
    steps: Vec<StepInfo>,
    /// The last warning shown, so a warned step's end doesn't repeat it.
    last_warning: Option<String>,
    done: mpsc::Sender<(RunOutcome, Option<Resume>)>,
    color: bool,
    interactive: bool,
    /// A program's output ended without a line break (the next line starts a new one).
    mid_line: bool,
}

impl Terminal {
    /// Start a new line when a program's output left the cursor mid-line.
    fn fresh_line(&mut self) {
        if std::mem::take(&mut self.mid_line) {
            out("");
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\u{1b}[{code}m{text}\u{1b}[0m")
        } else {
            text.to_string()
        }
    }

    fn read_line(&self, prompt: &str) -> Option<String> {
        if !self.interactive {
            return None;
        }
        // Not print!: that panics when stdout is closed.
        let mut o = std::io::stdout();
        let _ = write!(o, "{prompt}");
        let _ = o.flush();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).ok()?;
        Some(line.trim_end_matches(['\r', '\n']).to_string())
    }
}

impl RunUi for Terminal {
    fn event(&mut self, e: RunEvent) {
        if !matches!(e, RunEvent::Output { .. }) {
            self.fresh_line();
        }
        match e {
            RunEvent::Started { steps, .. } => self.steps = steps,
            RunEvent::StepStarted { index } => {
                if let Some(s) = self.steps.get(index).filter(|s| !s.quiet) {
                    out(&self.paint("1;36", &format!("==> {}", s.label)));
                }
            }
            RunEvent::Output { bytes, .. } => {
                let mut o = std::io::stdout();
                let _ = o.write_all(&bytes);
                let _ = o.flush();
                if let Some(last) = bytes.last() {
                    self.mid_line = *last != b'\n';
                }
            }
            RunEvent::Log { text, .. } => out(&text),
            RunEvent::Attached { .. } => {}
            RunEvent::StepFinished { outcome, .. } => match outcome {
                StepOutcome::Failed(m) => err(&self.paint("1;31", &format!("error: {m}"))),
                StepOutcome::Warned(m) => {
                    if self.last_warning.take().as_deref() != Some(m.as_str()) {
                        err(&self.paint("1;33", &format!("warning: {m}")));
                    }
                }
                StepOutcome::Skipped => out(&self.paint("2", "    skipped")),
                StepOutcome::Cancelled => err("cancelled"),
                StepOutcome::Ok => {}
            },
            RunEvent::Finished { outcome, resume } => {
                let _ = self.done.send((outcome, resume));
            }
        }
    }

    fn message(&mut self, text: &str, level: Level) {
        self.fresh_line();
        let t = match level {
            Level::Warning => self.paint("1;33", text),
            Level::Error => self.paint("1;31", text),
            _ => text.to_string(),
        };
        out(&t);
    }

    fn confirm(&mut self, text: &str, yes: &str, no: &str) -> bool {
        self.fresh_line();
        match self.read_line(&format!("{text} [{yes}/{no}] ")) {
            Some(a) => {
                let a = a.trim().to_lowercase();
                a == yes.to_lowercase() || a == "y" || a == "yes"
            }
            None => {
                err(&format!(
                    "{text}: no terminal to answer on, so the answer is {no}"
                ));
                false
            }
        }
    }

    fn ask(&mut self, prompt: &str, default: Option<&str>, _password: bool) -> Option<String> {
        self.fresh_line();
        let shown = match default {
            Some(d) => format!("{prompt} [{d}] "),
            None => format!("{prompt} "),
        };
        match self.read_line(&shown) {
            Some(a) if a.is_empty() => default.map(str::to_string),
            Some(a) => Some(a),
            None => default.map(str::to_string),
        }
    }

    fn warn(&mut self, text: &str) {
        self.fresh_line();
        err(&self.paint("1;33", &format!("warning: {text}")));
        self.last_warning = Some(text.to_string());
    }

    fn set_control(&mut self, id: &str, value: Option<Dyn>) {
        self.fresh_line();
        match value {
            Some(v) => out(&self.paint("2", &format!("    {id} = {}", v.to_text()))),
            None => out(&self.paint("2", &format!("    {id} reset"))),
        }
    }

    fn set_choices(&mut self, id: &str, choices: Vec<String>) {
        self.fresh_line();
        out(&self.paint("2", &format!("    {id} choices: {}", choices.join(", "))));
    }
}

pub fn run(a: &RunArgs) -> i32 {
    let path = launcher_core::paths::absolute(&a.file);
    let shown = a.file.display().to_string();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            err(&format!("error: can't read {shown}: {e}"));
            return EXIT_USAGE;
        }
    };
    let loaded = config::load_str(
        &text,
        &path,
        &LoadOptions {
            runtime: crate::cli::runtime_version(),
            os: Os::current(),
        },
    );
    if loaded.has_errors() {
        for d in &loaded.diags {
            err(&d.render(&shown));
        }
        return EXIT_USAGE;
    }
    let token = match trust::check(&path, &text) {
        Ok(t) => t,
        Err(_) if a.trust => trust::assume(),
        Err(mark) => {
            err(&format!(
                "error: {shown} {}; nothing in it has run. Check where it came from, then run it again with --trust (or open it in the window and trust it there).",
                mark.describe()
            ));
            return EXIT_USAGE;
        }
    };
    let config = Arc::new(loaded.config);
    let dry = a.dry_run.then(|| {
        Arc::new(DryRunEffects {
            mocks: a
                .answers
                .iter()
                .filter_map(|(k, v)| Mock::parse(v, &config.dir).ok().map(|m| (k.clone(), m)))
                .collect(),
            ..Default::default()
        })
    });
    let mock = a.mock.then(|| Arc::new(MockEffects::default()));
    let effects: Arc<dyn Effects> = match (&dry, &mock) {
        (Some(d), _) => d.clone(),
        (None, Some(m)) => m.clone(),
        (None, None) => Arc::new(RealEffects),
    };
    // The run's environment: ours (on macOS, with the login shell's PATH).
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    if cfg!(target_os = "macos") {
        if let Some(login) = launcher_core::platform::env::login_shell_env() {
            env.extend(login);
        }
    }
    // Load-time values, then the form. Loading only reads (N6), and the window evaluates the file
    // when it opens anyway, so a dry run evaluates it for real and only records the run.
    let load_effects: Arc<dyn Effects> = match &mock {
        Some(m) => m.clone(),
        None => Arc::new(RealEffects),
    };
    let mut ev = match Evaluator::new((*config).clone(), load_effects, token) {
        Ok(e) => e,
        Err(e) => {
            err(&format!("error: {e}"));
            return EXIT_ERRORS;
        }
    };
    let results = ev.load(Snapshot::default());
    let mut failed_loading = false;
    for e in &results.script_errors {
        err(&format!("error: {}", e.message));
        failed_loading = true;
    }
    for (id, r) in results.values.iter().chain(results.defaults.iter()) {
        if let Err(e) = r {
            err(&format!("error: {id}: {e}"));
            failed_loading = true;
        }
    }
    for (id, r) in &results.options {
        if let Err(e) = r {
            err(&format!("error: {id}: {e}"));
            failed_loading = true;
        }
    }
    if failed_loading {
        return EXIT_ERRORS;
    }
    let remembered = launcher_core::state::state_path(&path)
        .map(|p| launcher_core::state::load(&p).values)
        .unwrap_or_default();
    // The form as the window would start it, then each --set as if the user made it.
    let mut form = Form::new(config.clone(), results, remembered, env.clone());
    let mut problems = Vec::new();
    for (id, text) in &a.sets {
        match form.set_text(id, text) {
            Ok(changed) => {
                let ch = launcher_core::form::follow(&mut form, &mut ev, changed);
                for l in ch.logs {
                    out(&l);
                }
                problems.extend(ch.errors);
            }
            Err(e) => problems.push(format!("--set {e}")),
        }
    }
    problems.extend(form.problems().iter().cloned());
    let snapshot = form.snapshot().clone();
    if !problems.is_empty() {
        for p in problems {
            err(&format!("error: {p}"));
        }
        return EXIT_USAGE;
    }
    let action = match (&a.action, config.actions.len()) {
        (Some(id), _) => id.clone(),
        (None, 1) => config.actions[0].id.clone(),
        (None, 0) => {
            err(&format!("error: {shown} has no actions"));
            return EXIT_USAGE;
        }
        (None, _) => {
            let ids: Vec<&str> = config.actions.iter().map(|x| x.id.as_str()).collect();
            err(&format!(
                "error: {shown} has several actions; pick one with --action ({})",
                ids.join(", ")
            ));
            return EXIT_USAGE;
        }
    };
    if config.action(&action).is_none() {
        err(&format!("error: {shown} has no action `{action}`"));
        return EXIT_USAGE;
    }
    catch_ctrl_c();
    let (tx, rx) = mpsc::channel();
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let handle = match runner::start(
        RunSpec {
            config: config.clone(),
            action,
            snapshot,
            env,
            effects,
            trusted: token,
            resume: None,
            history: !a.dry_run && !a.mock,
            term_size: (120, 40),
            // Output to a file or a pipe gets plain text: programs get pipes too.
            pty: std::io::stdout().is_terminal(),
        },
        Box::new(Terminal {
            steps: Vec::new(),
            last_warning: None,
            done: tx,
            color,
            interactive: std::io::stdin().is_terminal(),
            mid_line: false,
        }),
    ) {
        Ok(h) => h,
        Err(e) => {
            err(&format!("error: {e}"));
            return EXIT_ERRORS;
        }
    };
    let (outcome, _) = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(r) => break r,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if INTERRUPTED.swap(false, Ordering::SeqCst) {
                    handle.cancel();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break (
                    RunOutcome::Failed("the run ended unexpectedly".into()),
                    None,
                )
            }
        }
    };
    handle.join();
    if let Some(d) = &dry {
        out("");
        out("Preview: this is what it would have done:");
        for l in d.lines() {
            out(&format!("  {l}"));
        }
    }
    if let Some(m) = &mock {
        out("");
        out("Mocked calls:");
        for c in m.calls() {
            out(&format!("  {c}"));
        }
    }
    match outcome {
        RunOutcome::Ok | RunOutcome::Stopped => EXIT_OK,
        RunOutcome::Failed(m) => {
            err(&format!("the run failed: {m}"));
            EXIT_ERRORS
        }
        RunOutcome::Cancelled => {
            err("the run was cancelled");
            EXIT_CANCELLED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_run_options() {
        let r = parse(&args(&[
            "tool.launcher",
            "--action",
            "build",
            "--set",
            "mode=Release",
            "--dry-run",
            "--answer",
            "api=@a.json",
        ]))
        .unwrap();
        assert_eq!(r.action.as_deref(), Some("build"));
        assert_eq!(r.sets, [("mode".to_string(), "Release".to_string())]);
        assert!(r.dry_run);
        assert!(parse(&args(&["x.launcher", "--answer", "a=b"])).is_err());
        assert!(parse(&args(&["--set", "a=b"])).is_err());
    }
}

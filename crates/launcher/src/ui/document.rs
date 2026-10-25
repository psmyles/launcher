//! An opened `.launcher` file: loaded through the last-working-copy cache (N10), watched and
//! reloaded when it changes on disk (not during a run, N11), passed through the trust gate (N9),
//! then shown as its form (M6) and run in the run view.
//!
//! Lua never runs here (N6): the evaluation thread works out load-time values, conditions,
//! `(lua)` texts and on-change blocks, and this applies what comes back.
//!
//! The form appears at once with what's known without Lua; computed lists show a spinner and
//! conditions keep their buttons off until the answers arrive.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use egui::RichText;
use launcher_core::config::cache::{self, Opened};
use launcher_core::config::catalog::Os;
use launcher_core::config::summary::summary;
use launcher_core::config::{After, Config, ControlKind, Diagnostic, Item, LoadOptions, Val};
use launcher_core::dynval::Dyn;
use launcher_core::engine::effects::RealEffects;
use launcher_core::engine::runner::RunOutcome;
use launcher_core::eval::{render, EvalThread, LoadResults, Request, Response, Which};
use launcher_core::form::{Form, MAX_CHANGE_DEPTH};
use launcher_core::platform::trust::{self, Mark, Trusted};
use launcher_core::platform::watch::FileWatcher;
use launcher_core::state::Stored;
use winit::event_loop::EventLoopProxy;

use crate::app::{AppEvent, WindowRequest};
use crate::console::{SharedTerm, TermModel};
use crate::ui::form::{shown_controls, FormCx, FormEvent, FormUi};
use crate::ui::run::{RunInputs, RunRequest, RunView, StepView};
use crate::ui::theme;

/// The log beside the window: its width, and the least height it needs.
const RUN_PANEL: (f32, f32) = (520.0, 480.0);
/// The editor's.
const EDITOR_SIZE: (f32, f32) = (1280.0, 820.0);

/// A question the document itself asks.
enum Asking {
    /// A button's `confirm=`.
    Press { action: String, question: String },
    /// The window is closing while a run goes.
    Close,
    /// The window is closing with unsaved edits in the editor.
    CloseUnsaved,
}

pub struct Document {
    pub path: PathBuf,
    /// What loaded, or why nothing could be read.
    state: Result<Opened, String>,
    /// The text on disk the current state came from (a change reloads only when it differs).
    disk_text: Option<String>,
    _watcher: Option<FileWatcher>,
    proxy: EventLoopProxy<AppEvent>,
    /// The file carries a download mark and hasn't been trusted: nothing is evaluated.
    untrusted: Option<Mark>,
    token: Option<Trusted>,
    eval: Option<EvalThread>,
    /// The environment runs and templates see (on macOS, with the login shell's PATH).
    env: BTreeMap<String, String>,
    remembered: BTreeMap<String, Stored>,
    /// Choices steps filled in (`put`), kept like remembered values.
    filled: BTreeMap<String, Vec<String>>,
    form: Option<Form>,
    live: launcher_core::eval::Live,
    /// Load-time results are in.
    loaded: bool,
    /// Scripts, values and lists that failed to load; on-change errors.
    errors: Vec<String>,
    refreshing: BTreeSet<String>,
    /// The newest run: the log beside the window shows while there is one.
    run: Option<RunView>,
    /// The log's console, with every run since it opened.
    log: Option<SharedTerm>,
    /// A run couldn't start (another run holds the lock, say).
    run_error: Option<String>,
    /// The file changed during a run: reload once it ends.
    reload_pending: bool,
    asking: Option<Asking>,
    /// For the app: close, minimize or resize the window.
    request: Option<WindowRequest>,
    /// A run ended: title, text, and whether it went wrong (shown if the window is behind).
    notification: Option<(String, String, bool)>,
    /// The content's size (logical px), measured on the last frame.
    size: (f32, f32),
    /// The window's margin in the theme, as of the last frame.
    margin: f32,
    /// The form's width beside the log (the window's side of the line, its margins in it).
    form_width: f32,
    /// The visual editor, when open (it replaces the form).
    editor: Option<crate::maker::AnyEditor>,
}

fn options() -> LoadOptions {
    LoadOptions {
        runtime: crate::cli::runtime_version(),
        os: Os::current(),
    }
}

/// The environment of runs: ours, and on macOS the login shell's PATH (N14).
fn run_env() -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    if cfg!(target_os = "macos") {
        if let Some(login) = launcher_core::platform::env::login_shell_env() {
            env.extend(login);
        }
    }
    env
}

impl Document {
    pub fn open(
        path: PathBuf,
        remembered: BTreeMap<String, Stored>,
        filled: BTreeMap<String, Vec<String>>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Document {
        let path = launcher_core::paths::absolute(&path);
        launcher_core::recent::add(&path);
        let state = cache::open(&path, &options());
        let disk_text = std::fs::read_to_string(&path).ok();
        let p = proxy.clone();
        let watcher = FileWatcher::start(&path, move || {
            let _ = p.send_event(AppEvent::FileChanged);
        })
        .map_err(|e| launcher_core::log_warn!("{e}"))
        .ok();
        let mut d = Document {
            path,
            state,
            disk_text,
            _watcher: watcher,
            proxy,
            untrusted: None,
            token: None,
            eval: None,
            env: run_env(),
            remembered,
            filled,
            form: None,
            live: Default::default(),
            loaded: false,
            errors: Vec::new(),
            refreshing: BTreeSet::new(),
            run: None,
            log: None,
            run_error: None,
            reload_pending: false,
            asking: None,
            request: None,
            notification: None,
            size: (0.0, 0.0),
            margin: 0.0,
            form_width: 0.0,
            editor: None,
        };
        d.evaluate();
        d
    }

    fn config(&self) -> Option<Arc<Config>> {
        let opened = self.state.as_ref().ok()?;
        opened
            .loaded
            .gate
            .is_none()
            .then(|| Arc::new(opened.loaded.config.clone()))
    }

    /// Start over with the loaded file: the form at once, then the evaluation (if the trust
    /// gate allows it, N9).
    fn evaluate(&mut self) {
        self.eval = None;
        self.untrusted = None;
        self.token = None;
        self.loaded = false;
        self.errors.clear();
        self.refreshing.clear();
        self.live = Default::default();
        let Some(config) = self.config() else {
            self.form = None;
            return;
        };
        // Keep what is on screen across a reload.
        if let Some(f) = &self.form {
            self.remembered = f.remembered().clone();
            self.filled = f.filled().clone();
        }
        let text = self
            .state
            .as_ref()
            .map(|o| o.text.clone())
            .unwrap_or_default();
        let mut form = Form::new(
            config.clone(),
            LoadResults::default(),
            self.remembered.clone(),
            self.env.clone(),
        );
        form.set_filled(self.filled.clone());
        self.form = Some(form);
        match trust::check(&self.path, &text) {
            Ok(token) => self.start(token),
            Err(mark) => {
                launcher_core::log_info!(
                    "{} {}; waiting for trust",
                    self.path.display(),
                    mark.describe()
                );
                self.untrusted = Some(mark);
            }
        }
    }

    fn start(&mut self, token: Trusted) {
        let Some(config) = self.config() else { return };
        let proxy = self.proxy.clone();
        let t = EvalThread::start((*config).clone(), Arc::new(RealEffects), token, move |r| {
            let _ = proxy.send_event(AppEvent::Eval(Box::new(r)));
        });
        let base = self
            .form
            .as_ref()
            .map(|f| f.snapshot().clone())
            .unwrap_or_default();
        t.send(Request::Load(base));
        self.eval = Some(t);
        self.token = Some(token);
    }

    /// The user trusts the file (as it is now).
    fn grant_trust(&mut self) {
        let Ok(opened) = &self.state else { return };
        match trust::grant(&self.path, &opened.text) {
            Ok(token) => {
                self.untrusted = None;
                self.start(token);
            }
            Err(e) => launcher_core::log_warn!("{e}"),
        }
    }

    fn send(&self, r: Request) {
        if let Some(t) = &self.eval {
            t.send(r);
        }
    }

    /// Controls changed: their on-change blocks, then the conditions and texts again.
    fn changed(&mut self, ids: Vec<String>, depth: usize) {
        let Some(form) = &self.form else { return };
        for id in ids {
            let has_block = form
                .config()
                .control(&id)
                .is_some_and(|c| !c.on_change.is_empty());
            if !has_block {
                continue;
            }
            if depth >= MAX_CHANGE_DEPTH {
                self.errors.push(format!(
                    "on-change of `{id}` set off too many others; stopped"
                ));
                continue;
            }
            self.send(Request::OnChange {
                control: id,
                snapshot: form.snapshot().clone(),
                depth,
            });
        }
        if let Some(form) = &self.form {
            self.send(Request::Live(form.snapshot().clone()));
        }
    }

    pub fn on_eval(&mut self, r: Response) {
        match r {
            Response::Loaded(results) => {
                self.loaded = true;
                for e in &results.script_errors {
                    self.errors.push(e.message.clone());
                }
                for (id, r) in results
                    .values
                    .iter()
                    .chain(results.defaults.iter())
                    .filter_map(|(id, r)| r.as_ref().err().map(|e| (id, e)))
                {
                    self.errors.push(format!("{id}: {r}"));
                }
                for (id, r) in &results.options {
                    if let Err(e) = r {
                        self.errors.push(format!("{id}: {e}"));
                    }
                }
                if let Some(f) = self.form.as_mut() {
                    f.set_results(*results);
                    let snap = f.snapshot().clone();
                    self.send(Request::Live(snap));
                }
            }
            Response::Live(live) => {
                self.live = *live;
                let changed = match self.form.as_mut() {
                    Some(f) => f.set_texts(&self.live.texts),
                    None => Vec::new(),
                };
                if !changed.is_empty() {
                    self.changed(changed, 0);
                }
            }
            Response::List { .. } => {}
            Response::Options { control, result } => {
                self.refreshing.remove(&control);
                let changed = match self.form.as_mut() {
                    Some(f) => f.set_options(&control, result),
                    None => Vec::new(),
                };
                self.changed(changed, 0);
            }
            Response::Changed {
                control,
                changes,
                depth,
            } => {
                for l in &changes.logs {
                    launcher_core::log_info!("on-change of {control}: {l}");
                }
                for e in changes.errors {
                    self.errors.push(format!("on-change of `{control}`: {e}"));
                }
                let mut next = Vec::new();
                if let Some(f) = self.form.as_mut() {
                    for e in &changes.edits {
                        next.extend(f.apply(e));
                    }
                }
                self.changed(next, depth + 1);
            }
            Response::Failed(e) => {
                self.loaded = true;
                self.errors.push(e);
            }
        }
    }

    /// A Browse dialog answered.
    pub fn on_picked(&mut self, control: String, path: Option<PathBuf>) {
        let (Some(p), Some(form)) = (path, self.form.as_mut()) else {
            return;
        };
        let relative = form.config().control(&control).is_some_and(|c| {
            matches!(
                c.kind,
                ControlKind::File { relative: true, .. }
                    | ControlKind::Folder { relative: true, .. }
            )
        });
        let dir = form.config().dir.clone();
        let text = if relative {
            launcher_core::paths::display_relative(&dir, &p)
        } else {
            p.display().to_string()
        };
        let changed = form.set(&control, Dyn::Str(text));
        self.changed(changed, 0);
    }

    /// Open the visual editor on the file as it is on disk.
    pub fn open_editor(&mut self) {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        self.editor = Some(crate::maker::AnyEditor::open(
            self.path.clone(),
            text,
            self.token,
            self.env.clone(),
            Some(self.proxy.clone()),
        ));
        self.request = Some(WindowRequest::Size(EDITOR_SIZE.0, EDITOR_SIZE.1));
    }

    /// The editor's preview evaluated something.
    pub fn on_preview(&mut self, generation: u64, r: Response) {
        if let Some(e) = self.editor.as_mut() {
            e.on_preview(generation, r);
        }
    }

    /// The file changed on disk: the editor takes it (or asks), and the form reloads.
    pub fn on_file_changed(&mut self) {
        if let Some(e) = self.editor.as_mut() {
            e.on_disk_change();
        }
        self.reload_if_changed();
    }

    /// The file may have changed on disk: reload it if its text did (after a run, N11).
    pub fn reload_if_changed(&mut self) -> bool {
        if self.run.as_ref().is_some_and(RunView::is_running) {
            self.reload_pending = true;
            return false;
        }
        let now = std::fs::read_to_string(&self.path).ok();
        if now == self.disk_text {
            return false;
        }
        launcher_core::log_info!("{} changed; reloading", self.path.display());
        self.disk_text = now;
        self.state = cache::open(&self.path, &options());
        self.evaluate();
        true
    }

    /// The values to remember (the app writes the state file).
    pub fn remembered(&self) -> Option<&BTreeMap<String, Stored>> {
        self.form.as_ref().map(Form::remembered)
    }

    /// Choices steps filled in, to remember.
    pub fn filled(&self) -> Option<&BTreeMap<String, Vec<String>>> {
        self.form.as_ref().map(Form::filled)
    }

    /// A remembered value changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        self.form.as_mut().is_some_and(Form::take_dirty)
    }

    pub fn take_request(&mut self) -> Option<WindowRequest> {
        self.request.take()
    }

    /// The window wants to close: fine, unless a run is going or the editor has unsaved edits
    /// (then ask first).
    pub fn close_requested(&mut self) -> bool {
        if self.editor.as_ref().is_some_and(|e| e.unsaved()) {
            self.asking = Some(Asking::CloseUnsaved);
            return false;
        }
        if self.run.as_ref().is_some_and(RunView::is_running) {
            self.asking = Some(Asking::Close);
            return false;
        }
        true
    }

    /// Stop a run for good (the app is exiting).
    pub fn stop(&mut self) {
        if let Some(r) = &self.run {
            r.cancel();
        }
    }

    /// Run progress for the taskbar.
    pub fn progress(&self) -> crate::os::Progress {
        self.run
            .as_ref()
            .map_or(crate::os::Progress::Off, RunView::progress)
    }

    pub fn take_notification(&mut self) -> Option<(String, String, bool)> {
        self.notification.take()
    }

    /// The window's taskbar identity: this file's own.
    pub fn identity(&self) -> crate::os::Identity {
        let exe = std::env::current_exe().unwrap_or_default();
        crate::os::Identity {
            id: format!(
                "{}.File.{}",
                crate::product::NAME,
                launcher_core::paths::config_hash(&self.path)
            ),
            relaunch: format!("\"{}\" \"{}\"", exe.display(), self.path.display()),
            // The file's name: the title may change with the form's values.
            name: file_name(&self.path)
                .trim_end_matches(".launcher")
                .to_string(),
        }
    }

    /// The window title the file asks for.
    pub fn title(&self) -> Option<String> {
        let opened = self.state.as_ref().ok()?;
        if opened.loaded.gate.is_some() {
            return None;
        }
        let w = &opened.loaded.config.window;
        let node = w.node.as_ref()?;
        let t = match &w.title {
            Val::Text(t) => {
                let snap = self
                    .form
                    .as_ref()
                    .map(|f| f.snapshot().clone())
                    .unwrap_or_default();
                render(t, &snap, &self.env)
            }
            Val::Lua(_) => self
                .live
                .texts
                .get(&(node.path.clone(), "title".to_string()))
                .and_then(|r| r.as_ref().ok())
                .cloned()?,
        };
        Some(t).filter(|s| !s.trim().is_empty())
    }

    /// The window size the content asks for (logical px), or `None` to keep the window's.
    pub fn fit_size(&self) -> Option<(f32, f32)> {
        if self.run.is_some() || self.editor.is_some() {
            return None;
        }
        let w = self.config().map(|c| c.window.clone())?;
        let width = w.width.unwrap_or(self.size.0);
        let height = w.height.unwrap_or(self.size.1);
        (width > 0.0 && height > 0.0).then_some((width, height))
    }

    /// Whether the user may resize the window.
    pub fn resizable(&self) -> bool {
        if self.run.is_some() || self.editor.is_some() {
            return true;
        }
        match self.config() {
            Some(c) => {
                c.window.resizable && (c.window.width.is_some() || c.window.height.is_some())
            }
            None => true,
        }
    }

    fn start_run(
        &mut self,
        action: &str,
        dry: bool,
        resume: Option<launcher_core::engine::runner::Resume>,
    ) {
        let (Some(config), Some(form), Some(token)) = (self.config(), &self.form, self.token)
        else {
            return;
        };
        let inputs = RunInputs {
            config,
            action: action.to_string(),
            snapshot: form.snapshot().clone(),
            env: self.env.clone(),
            trusted: token,
            dry,
        };
        self.start_run_with(inputs, resume, Vec::new());
    }

    fn start_run_with(
        &mut self,
        inputs: RunInputs,
        resume: Option<launcher_core::engine::runner::Resume>,
        carried: Vec<StepView>,
    ) {
        let log = self.log.get_or_insert_with(TermModel::shared).clone();
        match RunView::start(inputs, resume, carried, log, self.proxy.clone()) {
            Ok(r) => {
                self.run_error = None;
                // The window grows to the right for the log, the first time it shows. The form
                // keeps its width: the window gains the log and the gap before it.
                if self.run.is_none() {
                    let (w, h) = self.size;
                    self.form_width = w - self.margin;
                    self.request = Some(WindowRequest::Size(
                        w + self.margin + RUN_PANEL.0,
                        h.max(RUN_PANEL.1),
                    ));
                }
                self.run = Some(r);
            }
            Err(e) => self.run_error = Some(e),
        }
    }

    /// Close the log beside the window (the window fits its form again).
    fn back_to_form(&mut self) {
        self.run = None;
        self.log = None;
    }

    /// A run is going (not just showing how it went).
    fn running(&self) -> bool {
        self.run.as_ref().is_some_and(RunView::is_running)
    }

    /// Why actions can't run: a shown field that needs attention.
    fn blocked(&self) -> Option<String> {
        let form = self.form.as_ref()?;
        if self.token.is_none() {
            return Some("The file isn't trusted yet".into());
        }
        shown_controls(form.config(), &self.live)
            .iter()
            .find_map(|id| form.check(id))
    }

    /// Everything that went wrong loading or evaluating, for the problems list.
    fn problems(&self) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = Vec::new();
        if let Ok(opened) = &self.state {
            for d in &opened.loaded.diags {
                out.push((diag_line(&self.path, d), d.is_error()));
            }
        }
        for e in &self.errors {
            out.push((e.clone(), true));
        }
        if let Some(f) = &self.form {
            for p in f.problems() {
                out.push((p.clone(), true));
            }
        }
        for ((_, which), r) in &self.live.conditions {
            if let Err(e) = r {
                let what = match which {
                    Which::Visible => "visible-if",
                    Which::Enabled => "enabled-if",
                };
                out.push((format!("{what}: {e}"), true));
            }
        }
        for ((_, prop), r) in &self.live.texts {
            if let Err(e) = r {
                out.push((format!("{prop}: {e}"), true));
            }
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.margin = theme::extras(ui).margin;
        if let Some(e) = self.editor.as_mut() {
            e.ui(ui);
            if e.done() {
                self.editor = None;
                self.reload_if_changed();
            }
            self.ask_ui(ui);
            return;
        }
        // A run shows beside the window, on the right: the window stays usable once it ends (its
        // buttons can be pressed again, or another button's steps run).
        if self.run.is_some() {
            // The window's margin on both sides of the line between them.
            let margin = theme::extras(ui).margin as i8;
            // Resizing the window resizes the log; the form keeps its width until its edge is
            // dragged. egui keeps a panel's width itself, clamped to the window, and the first
            // frame comes before the window grows for the log: so the width is ours, given to
            // the panel each frame, and only a drag changes it.
            let id = egui::Id::new("form-panel");
            let rect = ui.max_rect();
            ui.ctx().data_mut(|d| {
                d.insert_persisted(
                    id,
                    egui::PanelState {
                        outer_rect: egui::Rect::from_min_size(
                            rect.min,
                            egui::vec2(self.form_width, rect.height()),
                        ),
                    },
                )
            });
            let max = (ui.available_width() - 360.0).max(200.0);
            let form = egui::Panel::left(id)
                .resizable(true)
                .size_range(200.0..=max)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                    right: margin,
                    ..Default::default()
                }))
                .show(ui, |ui| self.main_ui(ui));
            let resize = id.with("__resize");
            let ctx = ui.ctx();
            if ctx.is_being_dragged(resize) || ctx.drag_stopped_id() == Some(resize) {
                self.form_width = form.response.rect.width();
            }
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(ui.visuals().panel_fill)
                        .inner_margin(egui::Margin {
                            left: margin,
                            ..Default::default()
                        }),
                )
                .show(ui, |ui| self.run_ui(ui));
        } else {
            self.main_ui(ui);
        }
    }

    /// The window: the form (or why it can't show yet).
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        // The editor: a small square button floating in the top-right margin (it takes no space
        // in the form's layout, so the window's fit is unchanged) that shows only while the
        // pointer is over its spot, or Ctrl+E. Not while a run shows beside the window (that
        // corner is the run's edge).
        let margin = theme::extras(ui).margin;
        let corner = ui.max_rect().right_top() + egui::vec2(margin - 2.0, 2.0 - margin);
        let edit_key = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::E));
        let shown = self.run.is_none();
        let clicked = shown
            && egui::Area::new(egui::Id::new("edit-button"))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::RIGHT_TOP)
                .fixed_pos(corner)
                .show(ui.ctx(), edit_button)
                .inner;
        if (clicked || edit_key) && !self.running() && self.state.is_ok() {
            self.back_to_form();
            self.open_editor();
            return;
        }
        let name = file_name(&self.path);
        let top = ui.cursor().min;
        let mut need_w = 0.0;
        match &self.state {
            Err(e) => {
                ui.heading(&name);
                ui.colored_label(ui.visuals().error_fg_color, e);
                need_w = 420.0;
            }
            Ok(opened) => {
                if let Some(gate) = &opened.loaded.gate {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.heading(&name);
                        theme::gap(ui);
                        ui.label(RichText::new(capitalize(&gate.message())).strong());
                        ui.label(RichText::new(capitalize(&gate.help())).weak());
                    });
                    self.size = (480.0, 200.0);
                    return;
                }
                if let Some(fb) = &opened.fallback {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{name} has problems, so this is the version that last worked ({}).",
                                    ago(fb.saved)
                                ))
                                .color(ui.visuals().warn_fg_color),
                            );
                            diag_list(ui, &self.path, &fb.diags);
                        });
                    theme::gap(ui);
                }
                if let Some(mark) = &self.untrusted {
                    let mut trust = false;
                    let mut show = false;
                    let s = summary(&opened.loaded.config);
                    let list = |ui: &mut egui::Ui, what: &str, items: &[String]| {
                        if items.is_empty() {
                            return;
                        }
                        theme::gap_small(ui);
                        ui.label(RichText::new(what).strong());
                        for i in items {
                            ui.label(RichText::new(format!("  {i}")).monospace());
                        }
                    };
                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(
                            theme::extras(ui).callout_padding as i8,
                        ))
                        .show(ui, |ui| {
                            ui.set_max_width(480.0);
                            ui.label(
                                RichText::new(format!("{name} {}.", mark.describe()))
                                    .color(ui.visuals().warn_fg_color)
                                    .strong(),
                            );
                            ui.label(
                                "Tools can change files and start apps on this computer. Nothing in this one has run yet. Only open tools from people you know.",
                            );
                            list(ui, "It uses these apps:", &s.programs);
                            list(ui, "It uses these setup scripts:", &s.scripts);
                            list(ui, "It goes to these web addresses:", &s.urls);
                            if s.lua {
                                theme::gap_small(ui);
                                ui.label(
                                    RichText::new(
                                        "It has scripts, which can start other apps too.",
                                    )
                                    .weak(),
                                );
                            }
                            theme::gap(ui);
                            ui.horizontal(|ui| {
                                trust = ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Open it").strong(),
                                        ),
                                    )
                                    .clicked();
                                show = ui
                                    .button("Show in folder")
                                    .on_hover_text("Look at the file first")
                                    .clicked();
                            });
                        });
                    if show {
                        let _ = launcher_core::platform::shell::reveal(&self.path);
                    }
                    if trust {
                        self.grant_trust();
                    }
                    let margin = theme::extras(ui).margin;
                    self.size = (540.0, ui.cursor().min.y - top.y + 2.0 * margin);
                    return;
                }
            }
        }

        let blocked = self.blocked();
        let mut events = Vec::new();
        if let Some(form) = &self.form {
            let mut f = FormUi::new(FormCx {
                form,
                live: &self.live,
                env: &self.env,
                refreshing: &self.refreshing,
                loaded: self.loaded,
                locked: self.running(),
                blocked: blocked.as_deref(),
                pick: false,
                selected: None,
                numbers: None,
            });
            f.show(ui);
            need_w = f.need;
            events = f.events;
            // Actions no button starts get a button row of their own.
            let config = form.config().clone();
            let loose: Vec<_> = config
                .actions
                .iter()
                .filter(|a| !has_button(&config, &a.id))
                .collect();
            if !loose.is_empty() {
                theme::gap(ui);
                let r = ui.horizontal(|ui| {
                    for (k, a) in loose.iter().enumerate() {
                        let text = a.label.clone().unwrap_or_else(|| a.id.clone());
                        let b = if k == 0 {
                            egui::Button::new(RichText::new(text).strong())
                        } else {
                            egui::Button::new(text)
                        };
                        let r = ui.add_enabled(blocked.is_none(), b);
                        let r = match &blocked {
                            Some(why) => r.on_disabled_hover_text(why),
                            None => r,
                        };
                        if r.clicked() {
                            events.push(FormEvent::Action(a.id.clone()));
                        }
                        r.context_menu(|ui| {
                            if ui.button("Preview").clicked() {
                                events.push(FormEvent::DryRun(a.id.clone()));
                                ui.close();
                            }
                        });
                    }
                });
                need_w = need_w.max(r.response.rect.right() - top.x);
            }
        }
        if let Some(e) = &self.run_error {
            ui.add_space(6.0);
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        let problems = self.problems();
        if !problems.is_empty() {
            theme::gap(ui);
            egui::CollapsingHeader::new(
                RichText::new(format!(
                    "{} problem{}",
                    problems.len(),
                    if problems.len() == 1 { "" } else { "s" }
                ))
                .color(if problems.iter().any(|p| p.1) {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().warn_fg_color
                }),
            )
            .id_salt("problems")
            .show(ui, |ui| {
                for (p, error) in &problems {
                    ui.label(RichText::new(p).small().color(if *error {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().warn_fg_color
                    }));
                }
            });
        }
        let height = ui.cursor().min.y - top.y;
        let margin = theme::extras(ui).margin;
        self.size = (
            need_w.max(260.0) + 2.0 * margin,
            height.max(40.0) + 2.0 * margin,
        );
        for e in events {
            self.on_form_event(e);
        }
        self.ask_ui(ui);
    }

    fn run_ui(&mut self, ui: &mut egui::Ui) {
        let Some(run) = self.run.as_mut() else { return };
        let request = run.ui(ui);
        // What the run did to the form.
        let edits = run.take_control_edits();
        let choices = run.take_choice_edits();
        let finish = run.take_finish();
        let inputs = run.inputs.clone();
        for (id, items) in choices {
            let changed = match self.form.as_mut() {
                Some(f) => f.set_choices(&id, items),
                None => Vec::new(),
            };
            self.changed(changed, 0);
        }
        for (id, v) in edits {
            let changed = match (self.form.as_mut(), v) {
                (Some(f), Some(v)) => f.set(&id, v),
                (Some(f), None) => f.reset(&id),
                _ => Vec::new(),
            };
            self.changed(changed, 0);
        }
        if let Some(outcome) = finish {
            let after = inputs
                .config
                .action(&inputs.action)
                .map(|a| a.after)
                .unwrap_or(After::Stay);
            let what = self
                .run
                .as_ref()
                .map(|r| r.title.clone())
                .unwrap_or_default();
            let (body, failed) = match &outcome {
                RunOutcome::Ok => (format!("{what} finished"), false),
                RunOutcome::Stopped => (format!("{what} stopped"), false),
                RunOutcome::Cancelled => (format!("{what} was cancelled"), true),
                RunOutcome::Failed(m) => (format!("{what} failed: {m}"), true),
            };
            let title = self.title().unwrap_or_else(|| {
                file_name(&self.path)
                    .trim_end_matches(".launcher")
                    .to_string()
            });
            self.notification = Some((title, body, failed));
            if outcome == RunOutcome::Ok && !inputs.dry {
                match after {
                    After::Exit => self.request = Some(WindowRequest::Close),
                    After::Minimize => self.request = Some(WindowRequest::Minimize),
                    After::Stay => {}
                }
            }
            // The file changed during the run (N11): reload it now it's over.
            if std::mem::take(&mut self.reload_pending) {
                self.reload_if_changed();
            }
        }
        match request {
            Some(RunRequest::Back) => self.back_to_form(),
            Some(RunRequest::Retry(resume)) => {
                let carried = self
                    .run
                    .as_ref()
                    .map(|r| r.steps_before(resume.from))
                    .unwrap_or_default();
                self.start_run_with(inputs, Some(resume), carried);
            }
            None => {}
        }
    }

    fn on_form_event(&mut self, e: FormEvent) {
        match e {
            FormEvent::Set(id, v) => {
                let changed = match self.form.as_mut() {
                    Some(f) => f.set(&id, v),
                    None => Vec::new(),
                };
                self.changed(changed, 0);
            }
            FormEvent::Refresh(id) => {
                if let Some(f) = &self.form {
                    self.refreshing.insert(id.clone());
                    self.send(Request::Refresh {
                        control: id,
                        snapshot: f.snapshot().clone(),
                    });
                }
            }
            FormEvent::Browse(id) => self.browse(&id),
            FormEvent::Press(id) => {
                let Some(form) = &self.form else { return };
                let Some(c) = form.config().control(&id) else {
                    return;
                };
                let ControlKind::Button {
                    action, confirm, ..
                } = &c.kind
                else {
                    return;
                };
                let question = confirm.as_ref().map(|q| match q {
                    Val::Text(t) => render(t, form.snapshot(), &self.env),
                    Val::Lua(_) => self
                        .live
                        .texts
                        .get(&(c.node.path.clone(), "confirm".to_string()))
                        .and_then(|r| r.as_ref().ok())
                        .cloned()
                        .unwrap_or_default(),
                });
                let action = action.clone();
                match question.filter(|q| !q.trim().is_empty()) {
                    Some(question) => self.asking = Some(Asking::Press { action, question }),
                    None => self.start_run(&action, false, None),
                }
            }
            FormEvent::Action(action) => self.start_run(&action, false, None),
            FormEvent::DryRun(action) => self.start_run(&action, true, None),
            FormEvent::Pick(_) | FormEvent::Place { .. } => {}
        }
    }

    /// A file or folder field's Browse: the system dialog, on its own thread (N15).
    fn browse(&self, id: &str) {
        let Some(form) = &self.form else { return };
        let Some(c) = form.config().control(id) else {
            return;
        };
        let dir = form.config().dir.clone();
        let current = form.value(id).to_text();
        let start = if current.trim().is_empty() {
            dir.clone()
        } else {
            let p = launcher_core::paths::resolve(&dir, &current);
            if p.is_dir() {
                p
            } else {
                p.parent().map(Path::to_path_buf).unwrap_or(dir.clone())
            }
        };
        let (folder, save, filter) = match &c.kind {
            ControlKind::Folder { .. } => (true, false, None),
            ControlKind::File { save, filter, .. } => (false, *save, filter.clone()),
            _ => return,
        };
        let title = c.label.clone().unwrap_or_else(|| c.id.clone());
        let control = id.to_string();
        let proxy = self.proxy.clone();
        let _ = std::thread::Builder::new()
            .name("browse".into())
            .spawn(move || {
                let mut d = rfd::FileDialog::new()
                    .set_directory(&start)
                    .set_title(&title);
                let types = filter
                    .as_deref()
                    .map(launcher_core::paths::file_types)
                    .unwrap_or_default();
                for (name, exts) in types {
                    d = d.add_filter(&name, &exts);
                }
                let path = if folder {
                    d.pick_folder()
                } else if save {
                    d.save_file()
                } else {
                    d.pick_file()
                };
                let _ = proxy.send_event(AppEvent::Picked { control, path });
            });
    }

    /// The document's own questions.
    fn ask_ui(&mut self, ui: &mut egui::Ui) {
        let Some(asking) = &self.asking else { return };
        let mut yes = false;
        let mut no = false;
        let (text, yes_text, no_text) = match asking {
            Asking::Press { question, .. } => (question.clone(), "Yes", "No"),
            Asking::Close => (
                "A run is still going. Stop it and close the window?".to_string(),
                "Stop and close",
                "Keep running",
            ),
            Asking::CloseUnsaved => (
                "The editor has unsaved changes. Close without saving them?".to_string(),
                "Close without saving",
                "Keep editing",
            ),
        };
        let modal = egui::Modal::new(egui::Id::new("document-question")).show(ui.ctx(), |ui| {
            ui.set_max_width(420.0);
            ui.label(text);
            theme::gap(ui);
            ui.horizontal(|ui| {
                yes = ui.button(yes_text).clicked();
                no = ui.button(no_text).clicked();
            });
        });
        if modal.should_close() {
            no = true;
        }
        if !(yes || no) {
            return;
        }
        match self.asking.take() {
            Some(Asking::Press { action, .. }) if yes => self.start_run(&action, false, None),
            Some(Asking::Close) if yes => {
                self.stop();
                self.request = Some(WindowRequest::Close);
            }
            Some(Asking::CloseUnsaved) if yes => {
                self.editor = None;
                if self.close_requested() {
                    self.request = Some(WindowRequest::Close);
                }
            }
            _ => {}
        }
    }
}

/// Whether some button in the layout starts `action`.
fn has_button(config: &Config, action: &str) -> bool {
    fn walk(config: &Config, items: &[Item], action: &str) -> bool {
        items.iter().any(|it| match it {
            Item::Control(i) => matches!(
                &config.controls[*i].kind,
                ControlKind::Button { action: a, .. } if a == action
            ),
            Item::Group { items, .. } | Item::Row { items, .. } => walk(config, items, action),
            Item::Tabs { tabs, .. } => tabs.iter().any(|t| walk(config, &t.items, action)),
            Item::Columns { columns, .. } => columns.iter().any(|c| walk(config, c, action)),
            _ => false,
        })
    }
    walk(config, &config.layout, action)
}

/// `"Programs|*.exe;*.bat|Scripts|*.ps1"` → the dialog's filters.
/// The editor's button: a small square with a pencil, drawn only while the pointer is over its
/// spot (or it has the keyboard's focus). Returns whether it was clicked.
fn edit_button(ui: &mut egui::Ui) -> bool {
    const SIZE: f32 = 20.0;
    let (rect, r) = ui.allocate_exact_size(egui::Vec2::splat(SIZE), egui::Sense::click());
    let keys = if cfg!(target_os = "macos") {
        "Cmd+E"
    } else {
        "Ctrl+E"
    };
    let r = r.on_hover_text(format!("Edit this tool ({keys})"));
    if r.hovered() || r.has_focus() {
        let v = ui.style().interact(&r);
        ui.painter().rect(
            rect,
            v.corner_radius,
            v.weak_bg_fill,
            v.bg_stroke,
            egui::StrokeKind::Inside,
        );
        if let Some(icon) = crate::ui::icons::texture(ui.ctx(), crate::ui::icons::EDIT) {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter()
                .image(icon.id(), rect.shrink(4.0), uv, v.fg_stroke.color);
        }
    }
    r.clicked()
}

fn diag_line(path: &Path, d: &Diagnostic) -> String {
    format!("{}:{}:{}: {}", file_name(path), d.line, d.col, d.message)
}

fn diag_list(ui: &mut egui::Ui, path: &Path, diags: &[Diagnostic]) {
    for d in diags {
        let color = if d.is_error() {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().warn_fg_color
        };
        ui.label(RichText::new(diag_line(path, d)).color(color));
        if let Some(h) = &d.help {
            ui.label(RichText::new(format!("    {h}")).weak());
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// "5 minutes ago", roughly.
fn ago(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => "moments ago".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

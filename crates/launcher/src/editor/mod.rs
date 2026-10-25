//! The visual editor (N12, M8), a mode of the document's window: an outline of the file, a live
//! preview of the form (Pick mode selects what's clicked), properties generated from the catalog,
//! and a bottom panel with Problems, a Dry run and the raw Text.
//!
//! * **Every edit is a splice** of the file's text at the parser's spans (`config::edit`); the
//!   file is never re-printed, so comments and formatting survive (LESSONS L1.1).
//! * **Undo** keeps up to 200 full texts; typing into one field is one step.
//! * **Save** replaces the file atomically.
//! * **A change on disk** reloads the editor when it has no unsaved edits, and asks otherwise.
//! * **Lua** is linted 300 ms after the last edit (`lua::lint`); the preview is evaluated again
//!   then too, on its own evaluation thread, when the file is trusted.

pub(crate) mod bottom;
pub mod luaedit;
mod outline;
mod props;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use kdl::KdlDocument;
use launcher_core::config::catalog::{Os, Ty};
use launcher_core::config::{self, edit, Config, Diagnostic, LoadOptions, Loaded, NodePath};
use launcher_core::engine::effects::RealEffects;
use launcher_core::eval::{EvalThread, Live, LoadResults, Request, Response};
use launcher_core::form::Form;
use launcher_core::platform::trust::Trusted;
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::ui::form::{FormCx, FormEvent, FormUi};
use crate::ui::theme;
use bottom::{DryRun, Tab};
use outline::{OutlineEvent, OutlineState};

/// Undo keeps this many texts.
const UNDO: usize = 200;
/// Lint and the preview wait this long after the last edit.
const SETTLE: Duration = Duration::from_millis(300);
/// Edits to one field this close together are one undo step.
const MERGE: Duration = Duration::from_millis(1500);

enum Asking {
    /// Done with unsaved edits.
    Leave,
    /// The file changed on disk while there were unsaved edits: its new text.
    DiskChanged(String),
}

pub struct Editor {
    pub path: PathBuf,
    text: String,
    /// What's on disk (as last loaded or saved).
    saved: String,
    undo: Vec<String>,
    redo: Vec<String>,
    last_merge: Option<(String, Instant)>,
    doc: Option<KdlDocument>,
    loaded: Loaded,
    lint: Vec<Diagnostic>,
    settle_at: Option<Instant>,
    selected: Option<NodePath>,
    outline: OutlineState,
    pick: bool,
    tab: Tab,
    /// The Text tab's buffer, and whether it has edits not yet applied.
    raw: String,
    raw_dirty: bool,
    message: Option<String>,
    token: Option<Trusted>,
    env: BTreeMap<String, String>,
    /// The window's event loop (none in tests: no preview thread, no dry run).
    proxy: Option<EventLoopProxy<AppEvent>>,
    // The preview.
    form: Option<Form>,
    live: Live,
    eval: Option<EvalThread>,
    /// Answers from an older preview thread are ignored.
    generation: u64,
    dry: DryRun,
    asking: Option<Asking>,
    /// The user is done: the document leaves the editor.
    pub done: bool,
}

fn load(path: &std::path::Path, text: &str) -> Loaded {
    config::load_str(
        text,
        path,
        &LoadOptions {
            runtime: crate::cli::runtime_version(),
            os: Os::current(),
        },
    )
}

impl Editor {
    pub fn new(
        path: PathBuf,
        text: String,
        token: Option<Trusted>,
        env: BTreeMap<String, String>,
        proxy: Option<EventLoopProxy<AppEvent>>,
    ) -> Editor {
        let loaded = load(&path, &text);
        let mut e = Editor {
            doc: KdlDocument::parse_v2(&text).ok(),
            loaded,
            path,
            saved: text.clone(),
            raw: text.clone(),
            text,
            undo: Vec::new(),
            redo: Vec::new(),
            last_merge: None,
            lint: Vec::new(),
            settle_at: Some(Instant::now()),
            selected: None,
            outline: OutlineState::default(),
            pick: false,
            tab: Tab::Problems,
            raw_dirty: false,
            message: None,
            token,
            env,
            proxy,
            form: None,
            live: Live::default(),
            eval: None,
            generation: 0,
            dry: DryRun::default(),
            asking: None,
            done: false,
        };
        e.dry.action = e
            .loaded
            .config
            .actions
            .first()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        e
    }

    pub fn unsaved(&self) -> bool {
        self.text != self.saved
    }

    /// A new text: kept for undo (typing into one field is one step), parsed, and linted and
    /// previewed once the edits settle.
    fn apply(&mut self, new: String, merge: Option<String>) {
        if new == self.text {
            return;
        }
        let now = Instant::now();
        let merged = matches!(
            (&merge, &self.last_merge),
            (Some(k), Some((last, at))) if k == last && now - *at < MERGE
        );
        if !merged {
            self.undo.push(std::mem::take(&mut self.text));
            if self.undo.len() > UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_merge = merge.map(|k| (k, now));
        self.set_text(new);
    }

    fn set_text(&mut self, new: String) {
        self.text = new;
        self.doc = KdlDocument::parse_v2(&self.text).ok();
        self.loaded = load(&self.path, &self.text);
        self.settle_at = Some(Instant::now() + SETTLE);
        if !self.raw_dirty {
            self.raw = self.text.clone();
        }
        // A selection that no longer exists goes.
        if let (Some(p), Some(doc)) = (&self.selected, &self.doc) {
            if edit::node_at(doc, p).is_none() {
                self.selected = None;
            }
        }
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::take(&mut self.text));
            self.last_merge = None;
            self.set_text(prev);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::take(&mut self.text));
            self.last_merge = None;
            self.set_text(next);
        }
    }

    /// Replace the file atomically.
    fn save(&mut self) {
        match launcher_core::replace_file::write_bytes_replacing(&self.path, &self.text) {
            Ok(()) => {
                self.saved = self.text.clone();
                self.message = None;
            }
            Err(e) => self.message = Some(format!("Couldn't save {}: {e}", self.path.display())),
        }
    }

    /// The file changed on disk: take it when nothing is unsaved, else ask.
    pub fn on_disk_change(&mut self) {
        let Ok(disk) = std::fs::read_to_string(&self.path) else {
            return;
        };
        if disk == self.saved || disk == self.text {
            self.saved = disk;
            return;
        }
        if self.unsaved() {
            self.asking = Some(Asking::DiskChanged(disk));
        } else {
            self.saved = disk.clone();
            self.apply(disk, None);
        }
    }

    /// The preview's evaluation thread answered.
    pub fn on_preview(&mut self, generation: u64, r: Response) {
        if generation != self.generation {
            return;
        }
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let mut changed = Vec::new();
        match r {
            Response::Loaded(results) => {
                changed = form.set_results(*results);
            }
            Response::Live(live) => {
                changed = form.set_texts(&live.texts);
                self.live = *live;
            }
            Response::List { .. } => {}
            Response::Options { control, result } => changed = form.set_options(&control, result),
            Response::Changed { changes, .. } => {
                for e in &changes.edits {
                    changed.extend(form.apply(e));
                }
            }
            Response::Failed(e) => {
                self.message = Some(format!("The preview couldn't evaluate: {e}"))
            }
        }
        let snap = form.snapshot().clone();
        if let Some(t) = &self.eval {
            for id in changed {
                t.send(Request::OnChange {
                    control: id,
                    snapshot: snap.clone(),
                    depth: 0,
                });
            }
            t.send(Request::Live(snap));
        }
    }

    /// After the edits settle: lint, and a fresh preview.
    fn settle(&mut self) {
        self.settle_at = None;
        let usable = !self.loaded.has_errors() && self.loaded.gate.is_none();
        self.lint = if usable {
            launcher_core::lua::lint::diagnostics(&self.loaded.config)
        } else {
            Vec::new()
        };
        if !usable {
            return;
        }
        let config = Arc::new(self.loaded.config.clone());
        let form = Form::new(
            config.clone(),
            LoadResults::default(),
            BTreeMap::new(),
            self.env.clone(),
        );
        self.live = Live::default();
        self.generation += 1;
        self.eval = self.token.zip(self.proxy.clone()).map(|(token, proxy)| {
            let generation = self.generation;
            let t = EvalThread::start((*config).clone(), Arc::new(RealEffects), token, move |r| {
                let _ = proxy.send_event(AppEvent::PreviewEval {
                    generation,
                    response: Box::new(r),
                });
            });
            t.send(Request::Load(form.snapshot().clone()));
            t
        });
        self.form = Some(form);
    }

    /// Every problem: loading's, then the Lua's.
    fn problems(&self) -> Vec<Diagnostic> {
        let mut all = self.loaded.diags.clone();
        all.extend(self.lint.iter().cloned());
        all.sort_by_key(|d| (d.line, d.col));
        all
    }

    fn on_outline(&mut self, e: OutlineEvent) {
        let result = match e {
            OutlineEvent::Select(p) => {
                self.selected = Some(p);
                return;
            }
            OutlineEvent::Add {
                parent,
                index,
                kind,
            } => {
                let taken = self.taken_ids();
                let config = self.loaded.config.clone();
                let pick = move |ty: Ty| -> Option<String> {
                    match ty {
                        Ty::ControlRef => config.controls.first().map(|c| c.id.clone()),
                        Ty::ActionRef => config.actions.first().map(|a| a.id.clone()),
                        Ty::BagRef => config.data.first().map(|d| d.id.clone()),
                        _ => None,
                    }
                };
                let Some(node) = edit::new_node_text(kind, &taken, &pick) else {
                    return;
                };
                edit::insert_node(&self.text, parent.as_deref(), index, &node)
            }
            OutlineEvent::Duplicate(p) => edit::duplicate_node(&self.text, &p),
            OutlineEvent::Delete(p) => {
                let r = edit::remove_node(&self.text, &p);
                match r {
                    Ok(t) => {
                        self.selected = None;
                        self.apply(t, None);
                    }
                    Err(e) => self.message = Some(e),
                }
                return;
            }
            OutlineEvent::Move(p, delta) => {
                let (last, parent) = p.split_last().expect("a path");
                let parent = (!parent.is_empty()).then_some(parent);
                if delta < 0 && *last == 0 {
                    return;
                }
                // The index is counted before the node is taken out.
                let to = if delta < 0 { last - 1 } else { last + 2 };
                edit::move_node(&self.text, &p, parent, to)
            }
        };
        match result {
            Ok((t, path)) => {
                self.apply(t, None);
                self.selected = Some(path);
            }
            Err(e) => self.message = Some(e),
        }
    }

    /// Every id in the file (a new node's id must be new).
    fn taken_ids(&self) -> std::collections::BTreeSet<String> {
        let c = &self.loaded.config;
        c.controls
            .iter()
            .map(|x| x.id.clone())
            .chain(c.actions.iter().map(|a| a.id.clone()))
            .chain(c.values.iter().map(|v| v.id.clone()))
            .chain(c.data.iter().map(|d| d.id.clone()))
            .chain(
                c.actions
                    .iter()
                    .flat_map(|a| a.steps.iter())
                    .filter_map(|s| s.id.clone()),
            )
            .collect()
    }

    /// Names the Lua fields complete.
    fn names(config: &Config) -> (Vec<String>, Vec<String>) {
        let mut globals: Vec<String> = launcher_core::lua::api::API
            .iter()
            .map(|i| i.name.to_string())
            .collect();
        globals.extend(launcher_core::lua::lint::script_globals(config));
        let mut values: Vec<String> = config
            .controls
            .iter()
            .filter(|c| c.has_value())
            .map(|c| c.id.clone())
            .chain(config.values.iter().map(|v| v.id.clone()))
            .collect();
        values.sort();
        (globals, values)
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if self.settle_at.is_some_and(|t| Instant::now() >= t) {
            self.settle();
        }
        if let Some(t) = self.settle_at {
            ui.ctx()
                .request_repaint_after(t.saturating_duration_since(Instant::now()));
        }
        // Keys: save, undo, redo.
        let (save, undo, redo) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                    || i.consume_key(
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                        egui::Key::Z,
                    ),
            )
        });
        if save {
            self.save();
        }
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }

        egui::Panel::top("editor-bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let name = self
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.label(RichText::new(name).strong());
                if self.unsaved() {
                    ui.label(RichText::new("● unsaved").color(ui.visuals().warn_fg_color));
                }
                if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                    self.save();
                }
                if ui
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                    .on_hover_text("Ctrl+Z")
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo"))
                    .on_hover_text("Ctrl+Y")
                    .clicked()
                {
                    self.redo();
                }
                ui.separator();
                ui.toggle_value(&mut self.pick, "Pick")
                    .on_hover_text("Click an item in the preview to select it");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Done").clicked() {
                        if self.unsaved() {
                            self.asking = Some(Asking::Leave);
                        } else {
                            self.done = true;
                        }
                    }
                    if let Some(m) = &self.message {
                        ui.label(RichText::new(m).color(ui.visuals().error_fg_color));
                    }
                });
            });
        });

        let problems = self.problems();
        egui::Panel::bottom("editor-bottom")
            .resizable(true)
            .default_size(200.0)
            .min_size(80.0)
            .show(ui, |ui| self.bottom_ui(ui, &problems));

        let mut outline_event = None;
        egui::Panel::left("editor-outline")
            .resizable(true)
            .default_size(300.0)
            .min_size(180.0)
            .max_size(340.0)
            .show(ui, |ui| {
                outline_event = outline::show(
                    ui,
                    self.doc.as_ref(),
                    self.selected.as_ref(),
                    &mut self.outline,
                );
            });
        if let Some(e) = outline_event {
            self.on_outline(e);
        }

        let mut change = None;
        egui::Panel::right("editor-props")
            .resizable(true)
            .default_size(380.0)
            .min_size(240.0)
            .show(ui, |ui| {
                let (Some(doc), Some(path)) = (&self.doc, &self.selected) else {
                    ui.label(
                        RichText::new(
                            "Select something in the outline or, with Pick, in the preview.",
                        )
                        .weak(),
                    );
                    return;
                };
                let (globals, values) = Self::names(&self.loaded.config);
                let names = luaedit::Names {
                    globals: &globals,
                    values: &values,
                };
                let snapshot = self
                    .form
                    .as_ref()
                    .map(|f| f.snapshot().clone())
                    .unwrap_or_default();
                let cx = props::PropsCx {
                    text: &self.text,
                    doc,
                    config: &self.loaded.config,
                    snapshot: &snapshot,
                    env: &self.env,
                    issues: &problems,
                    names: &names,
                };
                change = props::show(ui, &cx, path);
            });
        match change {
            Some(Ok(c)) => self.apply(c.text, c.merge),
            Some(Err(e)) => self.message = Some(e),
            None => {}
        }

        egui::CentralPanel::default().show(ui, |ui| self.preview_ui(ui));
        self.ask_ui(ui);
    }

    fn preview_ui(&mut self, ui: &mut Ui) {
        let Some(form) = &self.form else {
            ui.label(RichText::new("The preview appears once the file has no errors.").weak());
            return;
        };
        let mut events = Vec::new();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let refreshing = Default::default();
                let mut f = FormUi::new(FormCx {
                    form,
                    live: &self.live,
                    env: &self.env,
                    refreshing: &refreshing,
                    loaded: self.eval.is_none()
                        || !self.live.conditions.is_empty()
                        || !form.results().options.is_empty(),
                    locked: false,
                    blocked: None,
                    pick: self.pick,
                    selected: self.selected.as_ref(),
                    numbers: None,
                });
                f.show(ui);
                events = f.events;
            });
        for e in events {
            match e {
                FormEvent::Pick(p) => {
                    self.selected = Some(p);
                }
                FormEvent::Set(id, v) => {
                    if let Some(form) = self.form.as_mut() {
                        let changed = form.set(&id, v);
                        let snap = form.snapshot().clone();
                        if let Some(t) = &self.eval {
                            for c in changed {
                                t.send(Request::OnChange {
                                    control: c,
                                    snapshot: snap.clone(),
                                    depth: 0,
                                });
                            }
                            t.send(Request::Live(snap));
                        }
                    }
                }
                FormEvent::Refresh(id) => {
                    if let (Some(t), Some(form)) = (&self.eval, &self.form) {
                        t.send(Request::Refresh {
                            control: id,
                            snapshot: form.snapshot().clone(),
                        });
                    }
                }
                // Buttons in the preview don't run anything; the Dry run tab does.
                _ => {}
            }
        }
    }

    fn bottom_ui(&mut self, ui: &mut Ui, problems: &[Diagnostic]) {
        ui.horizontal(|ui| {
            let n = problems.len();
            ui.selectable_value(&mut self.tab, Tab::Problems, format!("Problems ({n})"));
            ui.selectable_value(&mut self.tab, Tab::DryRun, "Dry run");
            ui.selectable_value(&mut self.tab, Tab::Text, "Text");
        });
        ui.separator();
        match self.tab {
            Tab::Problems => {
                if let Some(p) = bottom::problems(ui, problems) {
                    self.selected = Some(p);
                }
            }
            Tab::DryRun => self.dry_ui(ui),
            Tab::Text => self.text_ui(ui),
        }
    }

    fn dry_ui(&mut self, ui: &mut Ui) {
        let Some(token) = self.token else {
            ui.label(RichText::new("Trust the file to try a dry run.").weak());
            return;
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("dry-action")
                .selected_text(self.dry.action.clone())
                .show_ui(ui, |ui| {
                    for a in &self.loaded.config.actions {
                        ui.selectable_value(&mut self.dry.action, a.id.clone(), &a.id);
                    }
                });
            let usable = !self.loaded.has_errors() && !self.dry.action.is_empty();
            if ui
                .add_enabled(usable, egui::Button::new("Dry run"))
                .on_hover_text("List what the action would do with the preview's values (questions get their defaults)")
                .clicked()
            {
                let snapshot = self.form.as_ref().map(|f| f.snapshot().clone()).unwrap_or_default();
                if let Some(proxy) = self.proxy.clone() {
                    self.dry.start(
                        Arc::new(self.loaded.config.clone()),
                        snapshot,
                        self.env.clone(),
                        token,
                        proxy,
                    );
                }
            }
        });
        let result = self.dry.lines.lock().ok().and_then(|l| l.clone());
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match result {
                Some(Ok(lines)) if lines.is_empty() => {
                    ui.label(RichText::new("It would do nothing that a dry run records.").weak());
                }
                Some(Ok(lines)) => {
                    for l in lines {
                        ui.label(RichText::new(l).monospace());
                    }
                }
                Some(Err(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                None => {}
            });
    }

    fn text_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.raw_dirty, egui::Button::new("Apply"))
                .clicked()
            {
                match KdlDocument::parse_v2(&self.raw) {
                    Ok(_) => {
                        self.raw_dirty = false;
                        let raw = self.raw.clone();
                        self.apply(raw, None);
                    }
                    Err(e) => self.message = Some(format!("Not valid KDL: {e}")),
                }
            }
            if ui
                .add_enabled(self.raw_dirty, egui::Button::new("Discard"))
                .clicked()
            {
                self.raw_dirty = false;
                self.raw = self.text.clone();
            }
            if self.raw_dirty {
                ui.label(RichText::new("edited here; Apply to use it").weak());
            }
        });
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut self.raw)
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.raw_dirty = true;
                }
            });
    }

    fn ask_ui(&mut self, ui: &mut Ui) {
        let Some(asking) = &self.asking else { return };
        let (text, buttons): (&str, &[&str]) = match asking {
            Asking::Leave => (
                "Save your changes before leaving the editor?",
                &["Save", "Don't save", "Cancel"],
            ),
            Asking::DiskChanged(_) => (
                "The file changed on disk, and you have unsaved edits.",
                &["Load the file from disk", "Keep my edits"],
            ),
        };
        let mut chosen = None;
        egui::Modal::new(egui::Id::new("editor-question")).show(ui.ctx(), |ui| {
            ui.set_max_width(420.0);
            ui.label(text);
            theme::gap(ui);
            ui.horizontal(|ui| {
                for (i, b) in buttons.iter().enumerate() {
                    if ui.button(*b).clicked() {
                        chosen = Some(i);
                    }
                }
            });
        });
        let Some(i) = chosen else { return };
        match self.asking.take() {
            Some(Asking::Leave) => match i {
                0 => {
                    self.save();
                    self.done = !self.unsaved();
                }
                1 => self.done = true,
                _ => {}
            },
            Some(Asking::DiskChanged(disk)) => {
                self.saved = disk.clone();
                if i == 0 {
                    self.apply(disk, None);
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::config::catalog::{Category, SPECS};

    const FILE: &str = "// A test file.
launcher requires=\"0.1\"

layout {
    // Who runs it.
    text \"who\" label=\"Who\"
}

action \"go\" {
    log \"hi {who}\"
}
";

    fn editor(dir: &std::path::Path) -> Editor {
        let path = dir.join("t.launcher");
        std::fs::write(&path, FILE).unwrap();
        Editor::new(path, FILE.to_string(), None, BTreeMap::new(), None)
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("launcher-editor-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_outline_adds_every_layout_kind_and_step_without_errors() {
        let dir = temp("add");
        let mut e = editor(&dir);
        for spec in SPECS {
            // A kind for the other OS is refused on this one (its own test covers that).
            if spec.os != Os::Any && spec.os != Os::current() {
                continue;
            }
            let parent = match spec.cat {
                Category::Layout | Category::Control => vec![1],
                Category::Step => vec![2],
                _ => continue,
            };
            let before = e.text.clone();
            e.on_outline(OutlineEvent::Add {
                parent: Some(parent.clone()),
                index: 0,
                kind: spec.name,
            });
            assert_ne!(
                e.text, before,
                "{} added nothing: {:?}",
                spec.name, e.message
            );
            assert!(
                e.text.contains("// Who runs it."),
                "a comment was lost adding {}",
                spec.name
            );
            let errors: Vec<_> = e
                .loaded
                .diags
                .iter()
                .filter(|d| d.is_error() && !d.code.starts_with("unknown-"))
                .map(|d| d.message.clone())
                .collect();
            assert!(errors.is_empty(), "{}: {errors:?}\n{}", spec.name, e.text);
            // The new node is selected, and it can be taken out again.
            let sel = e.selected.clone().expect("the new node is selected");
            assert_eq!(sel, [parent[0], 0]);
            e.on_outline(OutlineEvent::Delete(sel));
            assert_eq!(
                e.text, before,
                "deleting the new {} left the file changed",
                spec.name
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typing_is_one_undo_step_and_undo_redo_walk_the_texts() {
        let dir = temp("undo");
        let mut e = editor(&dir);
        let path = vec![1, 0];
        for label in ["W", "Wh", "Who?"] {
            let t = edit::set_prop(&e.text, &path, "label", Some(edit::Repr::Str(label))).unwrap();
            e.apply(t, Some("label".into()));
        }
        assert!(e.text.contains("label=\"Who?\""));
        assert_eq!(e.undo.len(), 1, "one field's typing is one step");
        e.on_outline(OutlineEvent::Duplicate(path.clone()));
        assert_eq!(e.undo.len(), 2);
        e.undo();
        e.undo();
        assert_eq!(e.text, FILE);
        e.redo();
        assert!(e.text.contains("label=\"Who?\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_writes_the_file_and_a_change_on_disk_is_taken_or_asked_about() {
        let dir = temp("save");
        let mut e = editor(&dir);
        let t = edit::set_prop(&e.text, &[1, 0], "label", Some(edit::Repr::Str("Name"))).unwrap();
        e.apply(t, None);
        assert!(e.unsaved());
        e.save();
        assert!(!e.unsaved());
        assert!(!dir.join("t.launcher.bak").exists());
        assert!(std::fs::read_to_string(dir.join("t.launcher"))
            .unwrap()
            .contains("label=\"Name\""));
        // Changed on disk with nothing unsaved: taken.
        std::fs::write(dir.join("t.launcher"), FILE).unwrap();
        e.on_disk_change();
        assert_eq!(e.text, FILE);
        assert!(e.asking.is_none());
        // Changed on disk with unsaved edits: asked.
        let t = edit::set_prop(&e.text, &[1, 0], "label", Some(edit::Repr::Str("X"))).unwrap();
        e.apply(t, None);
        std::fs::write(dir.join("t.launcher"), FILE.replace("hi", "hello")).unwrap();
        e.on_disk_change();
        assert!(matches!(e.asking, Some(Asking::DiskChanged(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn moving_keeps_notes_with_their_node() {
        let dir = temp("move");
        let mut e = editor(&dir);
        e.on_outline(OutlineEvent::Add {
            parent: Some(vec![1]),
            index: 1,
            kind: "checkbox",
        });
        e.on_outline(OutlineEvent::Move(vec![1, 0], 1));
        assert_eq!(e.selected, Some(vec![1, 1]));
        let at_note = e.text.find("// Who runs it.").unwrap();
        let at_checkbox = e.text.find("checkbox").unwrap();
        let at_text = e.text.find("text \"who\"").unwrap();
        assert!(at_checkbox < at_note && at_note < at_text, "{}", e.text);
        let _ = std::fs::remove_dir_all(dir);
    }
}

//! The editor for tools (format 2), side by side (N26): what people see on the left, what the
//! tool does in the middle, and the settings of what's selected on the right. Steps come from the
//! step picker, which opens from a +; "Let people choose when they use the tool" on a setting puts
//! a control in the window. Controls in the window are dragged to arrange them: above, below, or
//! side by side (up to three in a row).
//!
//! The tool's parsed text is what changes: each edit is made to it (`config::tool_edit`) and the
//! result printed the one way the editor writes files (`config::print`, N24). Undo, save and
//! changes on disk work as in the first editor.

mod picker;
mod settings;
mod steps;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use kdl::{KdlDocument, KdlEntry, KdlValue};
use launcher_core::addons::{Library, Need, StepDef};
use launcher_core::config::catalog::Os;
use launcher_core::config::template::{Part, Template};
use launcher_core::config::tool_edit::{self, StepAt};
use launcher_core::config::{self, print, Diagnostic, LoadOptions, Loaded};
use launcher_core::engine::effects::{Context, Effects, RealEffects};
use launcher_core::eval::{EvalThread, Live, LoadResults, Request, Response};
use launcher_core::form::Form;
use launcher_core::platform::trust::Trusted;
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::ui::form::{number_text, FormCx, FormEvent, FormUi};
use crate::ui::theme;
use picker::{PickCx, Picked, Picker};
use settings::{Edit, ListAnswer};
use steps::{Badge, Card, StepsCx, StepsEvent};

const UNDO: usize = 200;
const SETTLE: Duration = Duration::from_millis(300);
const MERGE: Duration = Duration::from_millis(1500);

/// What's selected.
#[derive(Clone, PartialEq)]
enum Sel {
    Step(StepAt),
    Control(String),
    /// Text, a line or a section in the window: where it is (`tool_edit::item`).
    Item(Vec<usize>),
}

enum Asking {
    Leave,
    DiskChanged(String),
}

pub struct Maker {
    pub path: PathBuf,
    text: String,
    saved: String,
    undo: Vec<String>,
    redo: Vec<String>,
    last_merge: Option<(String, Instant)>,
    doc: Option<KdlDocument>,
    loaded: Loaded,
    lib: Library,
    settle_at: Option<Instant>,
    sel: Option<Sel>,
    /// The `steps` block shown in the middle (a button's).
    shown: String,
    drag: Option<StepAt>,
    picker: Option<Picker>,
    fixes_open: bool,
    text_open: bool,
    token: Option<Trusted>,
    env: BTreeMap<String, String>,
    proxy: Option<EventLoopProxy<AppEvent>>,
    form: Option<Form>,
    live: Live,
    eval: Option<EvalThread>,
    generation: u64,
    /// The add-ons whose apps are in the tool's folder.
    found: Vec<String>,
    /// Whether each app is on this computer (asked once per app).
    apps: RefCell<HashMap<String, bool>>,
    /// Add-on lists' choices for the settings column (Git's branches), by list and settings.
    lists: RefCell<HashMap<String, ListAnswer>>,
    asking: Option<Asking>,
    /// A text field had the keyboard at the end of the last frame. egui lets go of it as soon as
    /// Esc is pressed, so this is how Esc knows it only stops the editing.
    was_typing: bool,
    message: Option<String>,
    /// The file on disk isn't in the editor's layout (saving will tidy it).
    untidy: bool,
    /// A card to bring into view (one just added or moved).
    scroll_to: Option<StepAt>,
    /// Where the window column is (the picker opens over it).
    left_rect: egui::Rect,
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

/// Whether a file's text is a tool (format 2): its first node is `tool`.
pub fn is_tool(text: &str) -> bool {
    KdlDocument::parse_v2(text)
        .map(|d| config::format2::is_tool(&d))
        .unwrap_or_else(|_| text.trim_start().starts_with("tool"))
}

impl Maker {
    pub fn new(
        path: PathBuf,
        text: String,
        token: Option<Trusted>,
        env: BTreeMap<String, String>,
        proxy: Option<EventLoopProxy<AppEvent>>,
    ) -> Maker {
        let loaded = load(&path, &text);
        let lib = Library::current().with_carried_arcs(&loaded.config.carried);
        let folder = path.parent().map(PathBuf::from).unwrap_or_default();
        let found = lib.found_in(&folder);
        let doc = KdlDocument::parse_v2(&text).ok();
        let untidy = !print::is_tidy(&text);
        let mut m = Maker {
            path,
            saved: text.clone(),
            text,
            undo: Vec::new(),
            redo: Vec::new(),
            last_merge: None,
            doc,
            loaded,
            lib,
            settle_at: Some(Instant::now()),
            sel: None,
            shown: String::new(),
            drag: None,
            picker: None,
            fixes_open: false,
            text_open: false,
            token,
            env,
            proxy,
            form: None,
            live: Live::default(),
            eval: None,
            generation: 0,
            found,
            apps: RefCell::new(HashMap::new()),
            lists: RefCell::new(HashMap::new()),
            asking: None,
            was_typing: false,
            message: None,
            untidy,
            left_rect: egui::Rect::NOTHING,
            scroll_to: None,
            done: false,
        };
        m.fix_shown();
        m
    }

    pub fn unsaved(&self) -> bool {
        self.text != self.saved
    }

    /// Show a button's steps: the first button's unless one is shown already.
    fn fix_shown(&mut self) {
        let Some(doc) = &self.doc else { return };
        if tool_edit::steps_node(doc, &self.shown).is_some() {
            return;
        }
        let first_button = tool_edit::buttons(doc)
            .into_iter()
            .map(|(id, _)| id)
            .find(|id| tool_edit::steps_node(doc, id).is_some());
        let first_steps = doc
            .nodes()
            .iter()
            .find(|n| n.name().value() == "steps")
            .and_then(|n| n.entries().first())
            .and_then(|e| e.value().as_string())
            .map(str::to_string);
        self.shown = first_button.or(first_steps).unwrap_or_default();
    }

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
        self.lib = Library::current().with_carried_arcs(&self.loaded.config.carried);
        self.settle_at = Some(Instant::now() + SETTLE);
        self.fix_shown();
        // A selection that's gone goes.
        let gone = match (&self.sel, &self.doc) {
            (Some(Sel::Step(at)), Some(d)) => tool_edit::step_node(d, at).is_none(),
            (Some(Sel::Control(id)), Some(d)) => tool_edit::control(d, id).is_none(),
            (Some(Sel::Item(p)), Some(d)) => tool_edit::item(d, p).is_none(),
            _ => false,
        };
        if gone {
            self.sel = None;
        }
    }

    /// One change to the tool: made to its parsed text, then printed.
    fn edit(&mut self, merge: Option<String>, f: impl FnOnce(&mut KdlDocument, &Library)) {
        let Some(mut d) = self.doc.clone() else {
            self.message =
                Some("The file has a mistake in its text; fix it in the file's text first.".into());
            return;
        };
        f(&mut d, &self.lib);
        let text = print::print(&d);
        self.apply(text, merge);
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

    /// Write the tool in the editor's layout.
    fn save(&mut self) {
        let text = match &self.doc {
            Some(d) => print::print(d),
            None => self.text.clone(),
        };
        match launcher_core::replace_file::write_bytes_replacing(&self.path, &text) {
            Ok(()) => {
                if text != self.text {
                    self.set_text(text.clone());
                }
                self.saved = text;
                self.untidy = false;
                self.message = None;
            }
            Err(e) => self.message = Some(format!("Couldn't save {}: {e}", self.path.display())),
        }
    }

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

    pub fn on_preview(&mut self, generation: u64, r: Response) {
        if let Response::List { key, result } = r {
            let answer = match result {
                Ok(items) => ListAnswer::Items(items.into_iter().map(|o| o.value).collect()),
                Err(e) => ListAnswer::Failed(format!("Couldn't get the list: {e}")),
            };
            self.lists.borrow_mut().insert(key, answer);
            return;
        }
        if generation != self.generation {
            return;
        }
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let mut changed = Vec::new();
        match r {
            Response::Loaded(results) => changed = form.set_results(*results),
            Response::Live(live) => {
                changed = form.set_texts(&live.texts);
                self.live = *live;
            }
            Response::Options { control, result } => changed = form.set_options(&control, result),
            Response::Changed { changes, .. } => {
                for e in &changes.edits {
                    changed.extend(form.apply(e));
                }
            }
            Response::List { .. } => {}
            Response::Failed(e) => {
                self.message = Some(format!("The window couldn't be worked out: {e}"))
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

    /// After the edits settle: a fresh window on the left.
    fn settle(&mut self) {
        self.settle_at = None;
        if self.loaded.gate.is_some() {
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
        // Lists still being worked out were asked of the thread that's going: ask again.
        self.lists
            .borrow_mut()
            .retain(|_, a| !matches!(a, ListAnswer::Loading));
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

    // ---- what the columns show ----------------------------------------------------------------

    /// Whether an app is on this computer.
    fn has_app(&self, name: &str) -> bool {
        if let Some(b) = self.apps.borrow().get(name) {
            return *b;
        }
        let cx = Context::new(&self.loaded.config.dir);
        let b = RealEffects.has_app(&cx, name);
        self.apps.borrow_mut().insert(name.to_string(), b);
        b
    }

    /// Why a step can't run here, if its app is missing.
    fn missing(&self, d: &StepDef) -> Option<String> {
        d.needs.iter().find_map(|n| match n {
            Need::App { name, .. } if !self.has_app(name) => Some(format!(
                "{} isn't on this computer; the step can still be added for someone who has it.",
                d.app
            )),
            Need::Os(os) if *os != Os::current() => Some(format!(
                "This step only works on {}.",
                if *os == Os::Windows {
                    "Windows"
                } else {
                    "macOS"
                }
            )),
            _ => None,
        })
    }

    /// Problems, each with what it belongs to.
    fn problems(&self) -> Vec<(String, Option<Sel>)> {
        let Some(doc) = &self.doc else {
            return self
                .loaded
                .diags
                .iter()
                .map(|d| (format!("Line {}: {}", d.line, d.message), None))
                .collect();
        };
        let mut out = Vec::new();
        for d in self.loaded.diags.iter().filter(|d| d.is_error()) {
            let sel = self.owner(doc, d);
            let mut msg = d.message.clone();
            // The help is written for the file's text; a problem with a step or a control is
            // fixed in its settings instead.
            if let (Some(h), None) = (&d.help, &sel) {
                msg = format!("{msg} ({h})");
            }
            out.push((msg, sel));
        }
        out
    }

    /// The step or control a problem is about.
    fn owner(&self, doc: &KdlDocument, d: &Diagnostic) -> Option<Sel> {
        let inside = |n: &kdl::KdlNode| {
            let s = n.span();
            d.offset >= s.offset() && d.offset < s.offset() + s.len().max(1)
        };
        for steps in doc.nodes().iter().filter(|n| n.name().value() == "steps") {
            let id = steps
                .entries()
                .first()
                .and_then(|e| e.value().as_string())
                .unwrap_or_default();
            for before in [true, false] {
                for (i, n) in tool_edit::step_list(doc, id, before).iter().enumerate() {
                    if inside(n) {
                        return Some(Sel::Step(StepAt {
                            steps: id.to_string(),
                            before,
                            index: i,
                        }));
                    }
                }
            }
        }
        let ids = tool_edit::taken_ids(doc);
        ids.into_iter()
            .find(|id| tool_edit::control(doc, id).is_some_and(inside))
            .map(Sel::Control)
    }

    /// A step's sentence: its `says`, with its settings filled in.
    fn sentence(&self, doc: &KdlDocument, def: &StepDef, n: &kdl::KdlNode) -> String {
        let Some(says) = &def.says else {
            return String::new();
        };
        let Ok(t) = Template::parse(says) else {
            return says.clone();
        };
        let mut out = String::new();
        for p in &t.parts {
            match p {
                Part::Lit(s) => out.push_str(s),
                Part::Ref { name, .. } => {
                    if let Some(c) = tool_edit::asked_by(n, name) {
                        let label = tool_edit::control(doc, &c)
                            .and_then(|c| {
                                c.entries()
                                    .iter()
                                    .find(|e| e.name().is_some_and(|k| k.value() == "label"))
                                    .and_then(|e| e.value().as_string())
                                    .map(str::to_string)
                            })
                            .unwrap_or(c);
                        out.push_str(&format!("the chosen {label}"));
                        continue;
                    }
                    // What an earlier step found: "the Branches from List branches".
                    if let Some(words) = tool_edit::step_value(n, name)
                        .and_then(|v| v.as_string().map(str::to_string))
                        .and_then(|t| self.found_words(doc, &t))
                    {
                        out.push_str(&words);
                        continue;
                    }
                    let v = tool_edit::step_value(n, name)
                        .map(|v| match v {
                            KdlValue::String(s) => s,
                            other => other.to_string(),
                        })
                        .or_else(|| {
                            def.setting(name)
                                .and_then(|s| s.starts.as_ref())
                                .map(|d| d.to_text())
                        });
                    match v {
                        Some(v) if v == "." => out.push_str("this folder"),
                        Some(v) if !v.is_empty() => out.push_str(&v),
                        _ => out.push('…'),
                    }
                }
                Part::Env { name, .. } => out.push_str(&format!("{{env.{name}}}")),
            }
        }
        out
    }

    /// `{step.result}` in words, when that's all a value is: "the Branches from List branches".
    fn found_words(&self, doc: &KdlDocument, value: &str) -> Option<String> {
        let inner = value.strip_prefix('{')?.strip_suffix('}')?;
        let (step, result) = inner.split_once('.')?;
        for before in [true, false] {
            for n in tool_edit::step_list(doc, &self.shown, before) {
                let id = n.entries().first().and_then(|e| e.value().as_string());
                if id != Some(step) {
                    continue;
                }
                let def = self.lib.step(n.name().value())?;
                let r = def.results.iter().find(|r| r.id == result)?;
                let name = n
                    .get("label")
                    .and_then(|v| v.as_string())
                    .unwrap_or(&def.name);
                return Some(format!("the {} from {name}", r.label));
            }
        }
        None
    }

    fn cards(
        &self,
        doc: &KdlDocument,
        before: bool,
        problems: &[(String, Option<Sel>)],
    ) -> Vec<Card> {
        let numbers = self.numbers();
        tool_edit::step_list(doc, &self.shown, before)
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                let at = StepAt {
                    steps: self.shown.clone(),
                    before,
                    index: i,
                };
                let def = self.lib.step(n.name().value());
                let prop = |k: &str| {
                    n.entries()
                        .iter()
                        .find(|e| e.name().is_some_and(|x| x.value() == k))
                        .and_then(|e| e.value().as_string())
                        .map(str::to_string)
                };
                let mut badges = Vec::new();
                let (app, name, sentence) = match &def {
                    Some(d) => {
                        // The controls it reads and fills, by their numbers.
                        let control = |c: &str| {
                            let label = tool_edit::control(doc, c)
                                .and_then(|n| n.get("label").and_then(|v| v.as_string()))
                                .unwrap_or(c)
                                .to_string();
                            numbers.get(c).map(|n| (*n, label))
                        };
                        for s in &d.settings {
                            if let Some(c) = tool_edit::asked_by(n, &s.id) {
                                let found = control(&c);
                                badges.push(Badge {
                                    text: format!("{}: uses", s.label),
                                    bad: found.is_none(),
                                    control: found,
                                });
                            }
                        }
                        for (_, c) in tool_edit::puts(n) {
                            let found = control(&c);
                            badges.push(Badge {
                                text: "Fills".into(),
                                bad: found.is_none(),
                                control: found,
                            });
                        }
                        if let Some(m) = self.missing(d) {
                            badges.push(Badge {
                                text: m.split(';').next().unwrap_or_default().to_string(),
                                bad: true,
                                control: None,
                            });
                        }
                        (
                            d.app.clone(),
                            prop("label").unwrap_or_else(|| d.name.clone()),
                            self.sentence(doc, d, n),
                        )
                    }
                    None => (
                        "?".into(),
                        n.name().value().to_string(),
                        "Launcher doesn't know this step".into(),
                    ),
                };
                if prop("if").is_some() {
                    badges.push(Badge {
                        text: "Only when its script says so".into(),
                        bad: false,
                        control: None,
                    });
                }
                match prop("if-wrong").as_deref() {
                    Some("ask") => badges.push(Badge {
                        text: "If it goes wrong: ask".into(),
                        bad: false,
                        control: None,
                    }),
                    Some("keep-going") => badges.push(Badge {
                        text: "If it goes wrong: keep going".into(),
                        bad: false,
                        control: None,
                    }),
                    _ => {}
                }
                let problem = problems
                    .iter()
                    .find(|(_, s)| s.as_ref() == Some(&Sel::Step(at.clone())))
                    .map(|(m, _)| m.clone());
                Card {
                    at,
                    app,
                    name,
                    sentence,
                    badges,
                    problem,
                }
            })
            .collect()
    }

    /// An add-on list's choices, asked of the evaluation thread the first time.
    fn list(&self, l: &launcher_core::addons::ListUse) -> ListAnswer {
        let key = format!("{}{:?}", l.list, l.settings);
        if let Some(a) = self.lists.borrow().get(&key) {
            return match a {
                ListAnswer::Loading => ListAnswer::Loading,
                ListAnswer::Items(i) => ListAnswer::Items(i.clone()),
                ListAnswer::Failed(e) => ListAnswer::Failed(e.clone()),
            };
        }
        let Some(def) = self.lib.list(&l.list) else {
            return ListAnswer::Failed(format!("There's no list called \"{}\"", l.list));
        };
        let Some(t) = &self.eval else {
            return ListAnswer::Failed("Open the tool as trusted to see the list.".into());
        };
        t.send(Request::List {
            key: key.clone(),
            list: config::ListRef {
                def,
                settings: l.settings.clone(),
            },
        });
        self.lists.borrow_mut().insert(key, ListAnswer::Loading);
        ListAnswer::Loading
    }

    /// What the steps before a step found: their results, for its settings' "Use…". Checks
    /// ("before starting") come before every step.
    fn found_before(&self, doc: &KdlDocument, at: &StepAt) -> Vec<settings::Found> {
        let mut out = Vec::new();
        let parts: &[(bool, usize)] = if at.before {
            &[(true, at.index)]
        } else {
            &[(true, usize::MAX), (false, at.index)]
        };
        for (before, upto) in parts {
            for (i, n) in tool_edit::step_list(doc, &at.steps, *before)
                .into_iter()
                .enumerate()
                .take(*upto)
            {
                let Some(def) = self.lib.step(n.name().value()) else {
                    continue;
                };
                let Some(id) = n
                    .entries()
                    .iter()
                    .find(|e| e.name().is_none())
                    .and_then(|e| e.value().as_string())
                else {
                    continue;
                };
                let name = n
                    .get("label")
                    .and_then(|v| v.as_string())
                    .unwrap_or(&def.name)
                    .to_string();
                for r in &def.results {
                    out.push(settings::Found {
                        at: StepAt {
                            steps: at.steps.clone(),
                            before: *before,
                            index: i,
                        },
                        step: id.to_string(),
                        step_name: name.clone(),
                        result: r.id.clone(),
                        label: r.label.clone(),
                        kind: r.kind,
                    });
                }
            }
        }
        out
    }

    /// The controls' numbers, in the window's order (shown in badges wherever a control is named).
    fn numbers(&self) -> HashMap<String, usize> {
        self.loaded
            .config
            .controls
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.clone(), i + 1))
            .collect()
    }

    /// What the shown steps need: ("Git is installed", there).
    fn needs(&self, doc: &KdlDocument) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = Vec::new();
        let mut seen = Vec::new();
        for before in [true, false] {
            for n in tool_edit::step_list(doc, &self.shown, before) {
                let Some(d) = self.lib.step(n.name().value()) else {
                    continue;
                };
                for need in &d.needs {
                    if let Need::App { name, .. } = need {
                        if seen.contains(name) {
                            continue;
                        }
                        seen.push(name.clone());
                        out.push((format!("{} is installed", d.app), self.has_app(name)));
                    }
                }
            }
        }
        out
    }

    // ---- the window ---------------------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut Ui) {
        if self.settle_at.is_some_and(|t| Instant::now() >= t) {
            self.settle();
        }
        if let Some(t) = self.settle_at {
            ui.ctx()
                .request_repaint_after(t.saturating_duration_since(Instant::now()));
        }
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (save, undo, redo, delete) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                    || i.consume_key(
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                        egui::Key::Z,
                    ),
                !typing && i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
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
        if delete && self.picker.is_none() {
            if let Some(Sel::Step(at)) = self.sel.clone() {
                self.delete_step(&at);
            }
        }
        // Esc lets go of the selection (the Settings column goes back to the whole tool), unless
        // something else takes it first: a text field stops editing, an open list or menu closes,
        // and so do the step picker and a question.
        let busy = typing
            || self.was_typing
            || self.picker.is_some()
            || self.asking.is_some()
            || egui::Popup::is_any_open(ui.ctx());
        if !busy && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.sel = None;
        }
        let problems = self.problems();
        self.top_bar(ui, &problems);
        let mut edits = Vec::new();
        // The three columns share one frame, so their headings and edges line up.
        let frame = egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(egui::Margin::symmetric(12, 10));
        egui::Panel::left("maker-window")
            .resizable(true)
            .default_size(460.0)
            .min_size(300.0)
            .frame(frame)
            .show(ui, |ui| {
                self.left_rect = ui.max_rect();
                self.window_column(ui);
            });
        egui::Panel::right("maker-settings")
            .resizable(true)
            .default_size(340.0)
            .min_size(260.0)
            .frame(frame)
            .show(ui, |ui| {
                column_head(
                    ui,
                    "SETTINGS",
                    "Click a step, or anything in the window, to change it. Esc goes back to the whole tool.",
                );
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let Some(doc) = &self.doc else {
                            return;
                        };
                        match &self.sel {
                            Some(Sel::Step(at)) => {
                                let def = tool_edit::step_node(doc, at)
                                    .and_then(|n| self.lib.step(n.name().value()));
                                let lists = |l: &launcher_core::addons::ListUse| self.list(l);
                                let found = self.found_before(doc, at);
                                let numbers = self.numbers();
                                let around = settings::Around {
                                    lists: &lists,
                                    found: &found,
                                    numbers: &numbers,
                                };
                                settings::step(ui, doc, at, def.as_ref(), &around, &mut edits);
                            }
                            Some(Sel::Control(id)) => {
                                let step_name = |n: &kdl::KdlNode| {
                                    n.get("label")
                                        .and_then(|v| v.as_string())
                                        .map(str::to_string)
                                        .or_else(|| {
                                            self.lib.step(n.name().value()).map(|d| d.name.clone())
                                        })
                                        .unwrap_or_else(|| n.name().value().to_string())
                                };
                                let numbers = self.numbers();
                                let conflicts = self.control_conflicts(doc, id);
                                settings::control(
                                    ui,
                                    doc,
                                    id,
                                    &step_name,
                                    &numbers,
                                    &conflicts,
                                    &mut edits,
                                )
                            }
                            Some(Sel::Item(p)) => settings::item(ui, doc, p, &mut edits),
                            None => settings::tool(ui, doc, &mut edits),
                        }
                    });
            });
        egui::CentralPanel::default()
            .frame(frame)
            .show(ui, |ui| self.steps_column(ui, &problems));
        for e in edits {
            self.on_edit(e);
        }
        self.picker_ui(ui);
        self.fixes_ui(ui, &problems);
        self.text_ui(ui);
        self.ask_ui(ui);
        self.was_typing = ui.ctx().egui_wants_keyboard_input();
    }

    fn top_bar(&mut self, ui: &mut Ui, problems: &[(String, Option<Sel>)]) {
        // The same side margin as the columns below.
        let frame = egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(egui::Margin::symmetric(12, 8));
        egui::Panel::top("maker-bar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                // The tool's name.
                let mut name = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.nodes().first())
                    .filter(|n| n.name().value() == "tool")
                    .and_then(|n| n.entries().first())
                    .and_then(|e| e.value().as_string())
                    .unwrap_or_default()
                    .to_string();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .font(egui::TextStyle::Heading)
                            .desired_width(260.0)
                            .hint_text("Name of the tool"),
                    )
                    .on_hover_text("The tool's name: its window's title")
                    .changed()
                {
                    let new = name.clone();
                    self.edit(Some("tool-name".into()), move |d, _| {
                        if let Some(t) = d
                            .nodes_mut()
                            .first_mut()
                            .filter(|n| n.name().value() == "tool")
                        {
                            if let Some(e) = t.entries_mut().iter_mut().find(|e| e.name().is_none())
                            {
                                *e = KdlEntry::new(new);
                            }
                        }
                    });
                }
                if self.unsaved() {
                    ui.label(RichText::new("● not saved").color(ui.visuals().warn_fg_color));
                } else if self.untidy {
                    ui.label(
                        RichText::new("Saving will tidy this file's layout.")
                            .small()
                            .weak(),
                    );
                }
                ui.separator();
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
                let n = problems.len();
                if n > 0 {
                    let text = if n == 1 {
                        "1 thing to fix".to_string()
                    } else {
                        format!("{n} things to fix")
                    };
                    let fill = ui.visuals().error_fg_color;
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(text).color(crate::ui::contrast::on(fill)),
                            )
                            .fill(fill),
                        )
                        .clicked()
                    {
                        self.fixes_open = !self.fixes_open;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Done").clicked() {
                        if self.unsaved() {
                            self.asking = Some(Asking::Leave);
                        } else {
                            self.done = true;
                        }
                    }
                    ui.menu_button("More", |ui| {
                        let mut on = self.doc.as_ref().is_some_and(tool_edit::saves_logs);
                        if ui
                            .checkbox(&mut on, settings::SAVE_LOGS)
                            .on_hover_text(settings::SAVE_LOGS_HELP)
                            .changed()
                        {
                            self.on_edit(Edit::SaveLogs(on));
                        }
                        ui.separator();
                        if ui.button("Show the file's text").clicked() {
                            self.text_open = true;
                            ui.close();
                        }
                        if ui.button("Open the Add-ons folder").clicked() {
                            if let Some(d) = launcher_core::paths::app_data_subdir("add-ons") {
                                let _ = RealEffects.open(&d.display().to_string());
                            }
                            ui.close();
                        }
                        if ui.button("Read the Add-ons folder again").clicked() {
                            Library::reload();
                            let t = self.text.clone();
                            self.set_text(t);
                            ui.close();
                        }
                    });
                    if ui
                        .button(RichText::new("Save").strong())
                        .on_hover_text("Ctrl+S")
                        .clicked()
                    {
                        self.save();
                    }
                    if let Some(m) = &self.message {
                        ui.label(RichText::new(m).color(ui.visuals().error_fg_color));
                    }
                });
            });
        });
    }

    fn window_column(&mut self, ui: &mut Ui) {
        column_head(
            ui,
            "WHAT PEOPLE SEE",
            "Click anything to change it. Drag to move it: onto the side of a control to put them \
             in a row, or into a section.",
        );
        let spans = layout_spans(&self.loaded.config);
        let numbers = self.numbers();
        let selected_path = match &self.sel {
            Some(Sel::Control(id)) => self.loaded.config.control(id).map(|c| c.node.path.clone()),
            Some(Sel::Item(p)) => self
                .doc
                .as_ref()
                .and_then(|d| tool_edit::item(d, p))
                .and_then(|n| {
                    spans
                        .iter()
                        .find(|(_, o)| *o == n.span().offset())
                        .map(|(path, _)| path.clone())
                }),
            _ => None,
        };
        let mut events = Vec::new();
        egui::ScrollArea::both()
            .id_salt("maker-window-scroll")
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| match &self.form {
                Some(form) => {
                    egui::Frame::group(ui.style())
                        .inner_margin(12.0)
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
                                pick: true,
                                selected: selected_path.as_ref(),
                                numbers: Some(&numbers),
                            });
                            f.show(ui);
                            events = f.events;
                        });
                }
                None => {
                    ui.label(RichText::new("The window appears here.").weak());
                }
            });
        // What's drawn, matched to the file by where each item is written.
        let doc = self.doc.clone().unwrap_or_default();
        let ids: Vec<(config::NodePath, String)> = self
            .loaded
            .config
            .controls
            .iter()
            .map(|c| (c.node.path.clone(), c.id.clone()))
            .collect();
        let id_of = |path: &config::NodePath| {
            ids.iter()
                .find(|(p, _)| p == path)
                .map(|(_, id)| id.clone())
        };
        let item_of = |path: &config::NodePath| {
            spans
                .iter()
                .find(|(p, _)| p == path)
                .and_then(|(_, o)| tool_edit::item_at_offset(&doc, *o))
        };
        for e in events {
            match e {
                FormEvent::Place { from, to, side } => {
                    let (Some(a), Some(b)) = (item_of(&from), item_of(&to)) else {
                        continue;
                    };
                    let moved = id_of(&from);
                    let mut done = false;
                    self.edit(None, |d, _| done = tool_edit::place_item(d, &a, &b, side));
                    if !done {
                        self.message = Some(match side {
                            tool_edit::Side::Left | tool_edit::Side::Right => format!(
                                "Only controls go side by side, at most {} in a row.",
                                tool_edit::ROW_MAX
                            ),
                            _ => "That can't go there.".into(),
                        });
                    }
                    self.sel = moved.map(Sel::Control);
                }
                FormEvent::Pick(path) => {
                    if let Some(id) = id_of(&path) {
                        if tool_edit::steps_node(&doc, &id).is_some() {
                            self.shown = id.clone();
                        }
                        self.sel = Some(Sel::Control(id));
                    } else if let Some(p) = item_of(&path) {
                        self.sel = Some(Sel::Item(p));
                    }
                }
                _ => {}
            }
        }
        theme::gap_small(ui);
        ui.menu_button("+ Add a control", |ui| {
            // (word, its name here, the new control's label, what it is)
            let kinds = [
                ("choice", "Choice", "A list to pick one from"),
                ("choices", "Choices", "Tick boxes for several"),
                ("tick-box", "Tick box", "Yes or no"),
                ("text", "Text", "Something typed"),
                ("number", "Number", "A number"),
                ("file", "File", "A file, with Browse"),
                ("folder", "Folder", "A folder, with Browse"),
                ("button", "Button", "Runs its own steps"),
            ];
            for (word, label, about) in kinds {
                let name = settings::kind_name(word);
                if ui.button(name).on_hover_text(about).clicked() {
                    let mut new_id = String::new();
                    self.edit(None, |d, _| new_id = tool_edit::add_control(d, word, label));
                    self.sel = Some(Sel::Control(new_id.clone()));
                    if word == "button" {
                        self.shown = new_id;
                    }
                    ui.close();
                }
            }
            ui.separator();
            // (word, its name here, the new item's text)
            for (word, name, text) in [
                ("info", settings::kind_name("info"), "Some words to read"),
                ("line", settings::kind_name("line"), ""),
                ("section", settings::kind_name("section"), "Section"),
            ] {
                if ui.button(name).clicked() {
                    let mut at = Vec::new();
                    self.edit(None, |d, _| at = tool_edit::add_layout(d, word, text));
                    self.sel = Some(Sel::Item(at));
                    ui.close();
                }
            }
        });
    }

    fn steps_column(&mut self, ui: &mut Ui, problems: &[(String, Option<Sel>)]) {
        column_head(
            ui,
            "WHAT IT DOES",
            "The steps run from top to bottom. Click + to add one; drag a step to move it.",
        );
        let Some(doc) = self.doc.clone() else {
            ui.label(
                RichText::new("The file's text has a mistake; open it from the More menu.")
                    .color(ui.visuals().error_fg_color),
            );
            return;
        };
        let numbers = self.numbers();
        let buttons: Vec<(String, String)> = tool_edit::buttons(&doc)
            .into_iter()
            .filter(|(id, _)| tool_edit::steps_node(&doc, id).is_some())
            .collect();
        if buttons.len() > 1 {
            // Wrapped: a row wider than the column would widen everything below it.
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Steps for the button:").weak());
                for (id, label) in &buttons {
                    let b = match numbers.get(id) {
                        Some(n) => egui::Button::selectable(
                            self.shown == *id,
                            (number_text(ui, *n), RichText::new(label)),
                        ),
                        None => egui::Button::selectable(self.shown == *id, label.as_str()),
                    };
                    if ui.add(b).clicked() {
                        self.shown = id.clone();
                        self.sel = None;
                    }
                }
            });
        }
        if self.shown.is_empty() {
            ui.label(RichText::new("Add a button to give the tool something to do.").weak());
            return;
        }
        let label = buttons
            .iter()
            .find(|(id, _)| *id == self.shown)
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| self.shown.clone());
        let before = self.cards(&doc, true, problems);
        let main = self.cards(&doc, false, problems);
        let needs = self.needs(&doc);
        let selected = match &self.sel {
            Some(Sel::Step(at)) => Some(at.clone()),
            _ => None,
        };
        let cx = StepsCx {
            steps: &self.shown,
            button_label: &label,
            button_number: numbers.get(&self.shown).copied(),
            before: &before,
            main: &main,
            needs: &needs,
            selected: selected.as_ref(),
            can_drop: &|from: &StepAt, to: &StepAt| !tool_edit::move_misplaces(&doc, from, to),
            scroll_to: self.scroll_to.take(),
        };
        let events = steps::show(ui, &cx, &mut self.drag);
        for e in events {
            match e {
                StepsEvent::Select(at) => self.sel = Some(Sel::Step(at)),
                StepsEvent::Plus(at) => {
                    let has = !self.found.is_empty() || !self.next_after(&doc, &at).is_empty();
                    self.picker = Some(Picker::new(at, has));
                }
                StepsEvent::Move(from, to) => {
                    self.edit(None, |d, _| {
                        tool_edit::move_step(d, &from, &to);
                    });
                    self.scroll_to = Some(to.clone());
                    self.sel = Some(Sel::Step(to));
                }
                StepsEvent::Delete(at) => self.delete_step(&at),
                StepsEvent::Duplicate(at) => self.edit(None, |d, _| {
                    tool_edit::duplicate_step(d, &at);
                }),
            }
        }
    }

    /// What usually follows the step just above a +.
    fn next_after(&self, doc: &KdlDocument, at: &StepAt) -> Vec<Arc<StepDef>> {
        if at.index == 0 {
            return Vec::new();
        }
        tool_edit::step_list(doc, &at.steps, at.before)
            .get(at.index - 1)
            .and_then(|n| self.lib.step(n.name().value()))
            .map(|d| {
                d.comes_next
                    .iter()
                    .filter_map(|id| self.lib.step(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn on_edit(&mut self, e: Edit) {
        match e {
            Edit::StepProp(at, name, v) => {
                let merge = format!("step:{}:{}:{name}", at.before, at.index);
                self.edit(Some(merge), |d, _| {
                    tool_edit::set_step_prop(d, &at, &name, v)
                });
            }
            Edit::StepList(at, name, items) => {
                let merge = format!("list:{}:{}:{name}", at.before, at.index);
                self.edit(Some(merge), |d, _| {
                    tool_edit::set_step_list(d, &at, &name, &items)
                });
            }
            Edit::Ask(at, name) => self.edit(None, |d, lib| {
                if let Some(def) =
                    tool_edit::step_node(d, &at).and_then(|n| lib.step(n.name().value()))
                {
                    tool_edit::use_new_control(d, &at, &name, &def);
                }
            }),
            Edit::SetPut(at, result, control) => self.edit(None, |d, _| {
                tool_edit::set_put(d, &at, &result, control.as_deref())
            }),
            Edit::SelectStep(at) => {
                self.shown = at.steps.clone();
                self.scroll_to = Some(at.clone());
                self.sel = Some(Sel::Step(at));
            }
            Edit::DeleteStep(at) => self.delete_step(&at),
            Edit::ControlProp(id, name, v) => {
                let merge = format!("control:{id}:{name}");
                self.edit(Some(merge), |d, _| {
                    if let Some(n) = tool_edit::control_mut(d, &id) {
                        tool_edit::set_prop(n, &name, v);
                    }
                });
            }
            Edit::ControlOptions(id, options) => {
                let merge = format!("options:{id}");
                self.edit(Some(merge), |d, _| tool_edit::set_options(d, &id, &options));
            }
            Edit::DeleteControl(id) => {
                let mut changed = Vec::new();
                self.edit(None, |d, _| {
                    tool_edit::remove_control(d, &id);
                    changed = tool_edit::forget_uses(d, &id);
                });
                self.say_forgotten("The control", &changed);
            }
            Edit::MoveControl(id, delta) => self.edit(None, |d, _| {
                tool_edit::move_control(d, &id, delta);
            }),
            Edit::ShowSteps(id) => {
                self.shown = id;
                self.sel = None;
            }
            Edit::SelectControl(id) => self.sel = Some(Sel::Control(id)),
            Edit::ItemArg(p, text) => {
                let merge = format!("item:{p:?}");
                self.edit(Some(merge), |d, _| {
                    if let Some(n) = tool_edit::item_mut(d, &p) {
                        match n.entries_mut().iter_mut().find(|e| e.name().is_none()) {
                            Some(e) => *e = KdlEntry::new(text),
                            None => n.entries_mut().insert(0, KdlEntry::new(text)),
                        }
                    }
                });
            }
            Edit::ItemProp(p, name, v) => {
                let merge = format!("item:{p:?}:{name}");
                self.edit(Some(merge), |d, _| {
                    if let Some(n) = tool_edit::item_mut(d, &p) {
                        tool_edit::set_prop(n, &name, v);
                    }
                });
            }
            Edit::MoveItem(p, delta) => {
                let mut moved = false;
                self.edit(None, |d, _| moved = tool_edit::move_item(d, &p, delta));
                if moved {
                    let mut to = p.clone();
                    if let Some(last) = to.last_mut() {
                        *last = (*last as i64 + delta as i64) as usize;
                    }
                    self.sel = Some(Sel::Item(to));
                }
            }
            Edit::DeleteItem(p) => {
                // A section takes its controls with it: steps stop using them.
                let mut changed = Vec::new();
                self.edit(None, |d, _| {
                    let before = tool_edit::value_controls(d);
                    tool_edit::remove_item(d, &p);
                    let after = tool_edit::value_controls(d);
                    for (id, _, _) in before.iter().filter(|c| !after.contains(c)) {
                        changed.extend(tool_edit::forget_uses(d, id));
                    }
                });
                self.say_forgotten("What was in it", &changed);
                self.sel = None;
            }
            Edit::SaveLogs(on) => self.edit(None, move |d, _| {
                tool_edit::set_save_logs(d, on);
            }),
        }
    }

    /// Take a step out, and clear what used what it found: nothing is left pointing at it.
    fn delete_step(&mut self, at: &StepAt) {
        let id = self.doc.as_ref().and_then(|d| {
            tool_edit::step_node(d, at)
                .and_then(|n| n.entries().iter().find(|e| e.name().is_none()))
                .and_then(|e| e.value().as_string())
                .map(str::to_string)
        });
        let mut changed = Vec::new();
        self.edit(None, |d, _| {
            tool_edit::remove_step(d, at);
            if let Some(id) = &id {
                changed = tool_edit::forget_uses(d, id);
            }
        });
        self.say_forgotten("What it found", &changed);
    }

    /// Say which steps had a setting cleared because what it used is gone.
    fn say_forgotten(&mut self, what: &str, steps: &[String]) {
        if steps.is_empty() {
            return;
        }
        let names: Vec<String> = steps
            .iter()
            .map(|s| format!("\u{201c}{s}\u{201d}"))
            .collect();
        self.message = Some(format!(
            "{what} was used by {}: those settings are cleared, so choose them again.",
            names.join(", ")
        ));
    }

    /// Steps that use a control but can't as it is now (its extensions or options changed), and
    /// results put into it that it can't hold: one sentence each.
    fn control_conflicts(&self, doc: &KdlDocument, id: &str) -> Vec<String> {
        let Some(word) = tool_edit::control(doc, id).map(|n| n.name().value().to_string()) else {
            return Vec::new();
        };
        let exts = tool_edit::control_extensions(doc, id);
        let options = tool_edit::control_options(doc, id);
        let mut out = Vec::new();
        for (block, _) in tool_edit::buttons(doc) {
            for before in [true, false] {
                for n in tool_edit::step_list(doc, &block, before) {
                    let Some(def) = self.lib.step(n.name().value()) else {
                        continue;
                    };
                    let name = n
                        .get("label")
                        .and_then(|v| v.as_string())
                        .unwrap_or(&def.name)
                        .to_string();
                    for s in &def.settings {
                        if tool_edit::asked_by(n, &s.id).as_deref() == Some(id)
                            && !s.takes(&word, &exts, &options)
                        {
                            out.push(format!(
                                "\u{201c}{}\u{201d} on \u{201c}{name}\u{201d} needs {}, and this \
                                 isn't that now.",
                                s.label,
                                s.needs()
                            ));
                        }
                    }
                    for (result, into) in tool_edit::puts(n) {
                        let Some(r) = def.results.iter().find(|r| r.id == result) else {
                            continue;
                        };
                        if into == id && !r.kind.fills(&word) {
                            out.push(format!(
                                "\u{201c}{name}\u{201d} puts {} here ({}), which this can't hold.",
                                r.label,
                                r.kind.words()
                            ));
                        }
                    }
                }
            }
        }
        out
    }

    fn picker_ui(&mut self, ui: &mut Ui) {
        let Some(mut p) = self.picker.take() else {
            return;
        };
        let doc = self.doc.clone().unwrap_or_default();
        let next = self.next_after(&doc, &p.at);
        let missing = |d: &StepDef| self.missing(d);
        // The tool says which OS it's for: a step for the other one can never run in it.
        let tool_os = doc
            .nodes()
            .iter()
            .find(|n| n.name().value() == "tool")
            .and_then(|n| n.get("os"))
            .and_then(|v| v.as_string())
            .map(str::to_string);
        let blocked = |d: &StepDef| {
            let tool_os = tool_os.as_deref()?;
            d.needs.iter().find_map(|n| match n {
                Need::Os(Os::Windows) if tool_os != "windows" => {
                    Some("This step only works on Windows, and this tool is for macOS.".into())
                }
                Need::Os(Os::Macos) if tool_os != "macos" => {
                    Some("This step only works on macOS, and this tool is for Windows.".into())
                }
                _ => None,
            })
        };
        let cx = PickCx {
            lib: &self.lib,
            found: &self.found,
            next: &next,
            missing: &missing,
            blocked: &blocked,
        };
        match picker::show(ui.ctx(), &mut p, &cx) {
            Picked::Nothing => self.picker = Some(p),
            Picked::Close => {}
            Picked::Add(def) => {
                let at = p.at.clone();
                self.edit(None, |d, _| {
                    tool_edit::insert_step(d, &at, &def);
                });
                self.scroll_to = Some(at.clone());
                self.sel = Some(Sel::Step(at));
            }
        }
    }

    fn fixes_ui(&mut self, ui: &mut Ui, problems: &[(String, Option<Sel>)]) {
        if !self.fixes_open || problems.is_empty() {
            self.fixes_open = false;
            return;
        }
        let mut open = true;
        let mut pick = None;
        egui::Window::new("Things to fix")
            .open(&mut open)
            .collapsible(false)
            .default_width(420.0)
            .show(ui.ctx(), |ui| {
                for (msg, sel) in problems {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("●").color(ui.visuals().error_fg_color));
                        ui.label(msg);
                        if sel.is_some() && ui.button("Show me").clicked() {
                            pick = sel.clone();
                        }
                    });
                }
            });
        if let Some(s) = pick {
            if let Sel::Step(at) = &s {
                self.shown = at.steps.clone();
            }
            self.sel = Some(s);
        }
        self.fixes_open = open;
    }

    fn text_ui(&mut self, ui: &mut Ui) {
        if !self.text_open {
            return;
        }
        let mut open = true;
        let mut text = self.text.clone();
        egui::Window::new("The file's text")
            .open(&mut open)
            .default_width(640.0)
            .default_height(480.0)
            .show(ui.ctx(), |ui| {
                ui.label(
                    RichText::new(
                        "This is what's saved. Edit it here only if you know the format.",
                    )
                    .weak(),
                );
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
            });
        if text != self.text {
            self.apply(text, Some("raw-text".into()));
        }
        self.text_open = open;
    }

    fn ask_ui(&mut self, ui: &mut Ui) {
        let Some(asking) = &self.asking else { return };
        let (text, buttons): (&str, &[&str]) = match asking {
            Asking::Leave => (
                "Save your changes before you stop editing?",
                &["Save", "Don't save", "Keep editing"],
            ),
            Asking::DiskChanged(_) => (
                "The file changed on disk while you have changes that aren't saved.",
                &["Use the file from disk", "Keep my changes"],
            ),
        };
        let mut chosen = None;
        egui::Modal::new(egui::Id::new("maker-question")).show(ui.ctx(), |ui| {
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

/// Either editor: the first one for `launcher` files, this one for tools.
pub enum AnyEditor {
    Old(Box<crate::editor::Editor>),
    Tool(Box<Maker>),
}

impl AnyEditor {
    pub fn open(
        path: PathBuf,
        text: String,
        token: Option<Trusted>,
        env: BTreeMap<String, String>,
        proxy: Option<EventLoopProxy<AppEvent>>,
    ) -> AnyEditor {
        if is_tool(&text) {
            AnyEditor::Tool(Box::new(Maker::new(path, text, token, env, proxy)))
        } else {
            AnyEditor::Old(Box::new(crate::editor::Editor::new(
                path, text, token, env, proxy,
            )))
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        match self {
            AnyEditor::Old(e) => e.ui(ui),
            AnyEditor::Tool(e) => e.ui(ui),
        }
    }

    pub fn done(&self) -> bool {
        match self {
            AnyEditor::Old(e) => e.done,
            AnyEditor::Tool(e) => e.done,
        }
    }

    pub fn unsaved(&self) -> bool {
        match self {
            AnyEditor::Old(e) => e.unsaved(),
            AnyEditor::Tool(e) => e.unsaved(),
        }
    }

    pub fn on_preview(&mut self, generation: u64, r: Response) {
        match self {
            AnyEditor::Old(e) => e.on_preview(generation, r),
            AnyEditor::Tool(e) => e.on_preview(generation, r),
        }
    }

    pub fn on_disk_change(&mut self) {
        match self {
            AnyEditor::Old(e) => e.on_disk_change(),
            AnyEditor::Tool(e) => e.on_disk_change(),
        }
    }
}

/// A column's heading and what to do in it, the same in all three columns.
fn column_head(ui: &mut Ui, title: &str, hint: &str) {
    ui.label(RichText::new(title).small().weak());
    ui.add(egui::Label::new(RichText::new(hint).small().weak()).wrap());
    theme::gap(ui);
}

/// Where each item of the window is written in the file (its byte offset), by the path the
/// window draws it at.
fn layout_spans(config: &config::Config) -> Vec<(config::NodePath, usize)> {
    fn walk(
        config: &config::Config,
        items: &[config::Item],
        out: &mut Vec<(config::NodePath, usize)>,
    ) {
        use config::Item;
        for it in items {
            let node = match it {
                Item::Control(i) => &config.controls[*i].node,
                Item::Group { node, .. }
                | Item::Tabs { node, .. }
                | Item::Columns { node, .. }
                | Item::Row { node, .. }
                | Item::Separator { node, .. }
                | Item::Spacer { node, .. }
                | Item::Note { node, .. } => node,
            };
            out.push((node.path.clone(), node.span.offset));
            match it {
                Item::Group { items, .. } | Item::Row { items, .. } => walk(config, items, out),
                Item::Tabs { tabs, .. } => {
                    for t in tabs {
                        walk(config, &t.items, out);
                    }
                }
                Item::Columns { columns, .. } => {
                    for c in columns {
                        walk(config, c, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(config, &config.layout, &mut out);
    out
}

#[cfg(test)]
mod tests {
    /// Everything the new editor and the home screen say is in plain words (N19): no string in
    /// their code holds a developer word, except names that are the file format's own.
    #[test]
    fn the_editor_speaks_plainly() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let files = [
            "maker/mod.rs",
            "maker/steps.rs",
            "maker/picker.rs",
            "maker/settings.rs",
            "ui/home.rs",
        ];
        // The file format's own words, which the code reads and writes but never shows.
        let format_words = [
            "list-from",
            "option",
            "steps",
            "show-as",
            "if-wrong",
            "label",
        ];
        let mut found = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(src.join(f)).unwrap();
            for (n, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") || line.contains("#[cfg(test)]") {
                    continue;
                }
                let code = line.split(" //").next().unwrap_or("");
                for lit in code.split('"').skip(1).step_by(2) {
                    if format_words.contains(&lit) {
                        continue;
                    }
                    let j = launcher_core::addons::jargon(lit);
                    if !j.is_empty() {
                        found.push(format!("{f}:{}: {j:?} in \"{lit}\"", n + 1));
                    }
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }
}

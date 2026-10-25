//! The right column: the settings of what's selected. For a step: each setting with its "Use…"
//! menu (type a value, what people chose in a control, or what an earlier step found), "If it
//! goes wrong", what the step gives back (each result can be put into a control, which is how one
//! button's findings reach another), and a collapsed Advanced section. For a control: its label,
//! kind, choices (or the step that fills them) and starting value. It changes nothing itself: it
//! returns [`Edit`]s for the editor to apply.

use std::sync::Arc;

use egui::{RichText, Ui};
use kdl::{KdlDocument, KdlNode, KdlValue};
use launcher_core::addons::{ListUse, ResultKind, SettingDef, SettingKind, StepDef};
use launcher_core::config::tool_edit::{self, StepAt};
use launcher_core::config::Val;

use super::steps::app_mark;
use crate::ui::form::number_text;
use crate::ui::theme;

/// The controls' numbers (as the editor's window shows them), by id.
pub type Numbers = std::collections::HashMap<String, usize>;

/// Pieces of text as one line that wraps as a whole.
fn rich_line(ui: &Ui, parts: Vec<RichText>) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for (i, p) in parts.into_iter().enumerate() {
        if i > 0 {
            RichText::new(" ").append_to(
                &mut job,
                ui.style(),
                egui::FontSelection::Default,
                egui::Align::Center,
            );
        }
        p.append_to(
            &mut job,
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Center,
        );
    }
    job
}

/// A full-width button whose text is several pieces, wrapping as one line.
fn link(ui: &mut Ui, parts: Vec<RichText>) -> egui::Response {
    let job = rich_line(ui, parts);
    ui.add(
        egui::Button::new((egui::WidgetText::from(job), egui::Atom::grow()))
            .wrap()
            .min_size(egui::vec2(ui.available_width(), 0.0)),
    )
}

/// A control by its number and label (and its kind, weak), as text pieces.
fn control_words(ui: &Ui, numbers: &Numbers, id: &str, label: &str) -> Vec<RichText> {
    let mut out = Vec::new();
    if let Some(n) = numbers.get(id) {
        out.push(number_text(ui, *n));
    }
    out.push(RichText::new(label));
    out
}

/// A change the person made.
pub enum Edit {
    /// A step's property (a setting, `label`, `if-wrong`, `if`); None takes it away.
    StepProp(StepAt, String, Option<KdlValue>),
    /// A list setting (one item per line).
    StepList(StepAt, String, Vec<String>),
    /// Use… → a new control for this setting.
    Ask(StepAt, String),
    /// Put a result into a control (None: nowhere).
    SetPut(StepAt, String, Option<String>),
    DeleteStep(StepAt),
    /// Select a step (from "What … found", or "Filled by").
    SelectStep(StepAt),
    ControlProp(String, String, Option<KdlValue>),
    ControlOptions(String, Vec<String>),
    DeleteControl(String),
    MoveControl(String, i32),
    /// Show a button's steps in the middle column.
    ShowSteps(String),
    /// Select a control (from a step's "They choose it in…").
    SelectControl(String),
    /// Text, a line or a section, by where it is: its text or title.
    ItemArg(Vec<usize>, String),
    ItemProp(Vec<usize>, String, Option<KdlValue>),
    MoveItem(Vec<usize>, i32),
    DeleteItem(Vec<usize>),
    /// Save each run's log to files, or don't.
    SaveLogs(bool),
}

/// The choices of an add-on list (Git's branches, say), as far as they're known.
pub enum ListAnswer {
    Loading,
    Items(Vec<String>),
    /// It couldn't be worked out (why), or can't be here (an untrusted file).
    Failed(String),
}

/// Looks up a list's choices for a setting (asking for them the first time).
pub type Lists<'a> = &'a dyn Fn(&ListUse) -> ListAnswer;

/// A result of an earlier step, which a setting can use.
pub struct Found {
    pub at: StepAt,
    /// The step's id: the result is `{step.result}`.
    pub step: String,
    pub step_name: String,
    pub result: String,
    pub label: String,
    /// What kind of value it is: a setting uses only what it can take.
    pub kind: ResultKind,
}

/// Where a setting's value comes from.
enum Source {
    Typed,
    Control(String),
    Found(usize),
}

/// A value as text, for a field.
fn text_of(v: &KdlValue) -> String {
    match v {
        KdlValue::String(s) => s.clone(),
        KdlValue::Bool(b) => b.to_string(),
        KdlValue::Integer(i) => i.to_string(),
        KdlValue::Float(f) => launcher_core::dynval::fmt_num(*f),
        KdlValue::Null => String::new(),
    }
}

fn label_of(n: &KdlNode) -> Option<String> {
    n.entries()
        .iter()
        .find(|e| e.name().is_some_and(|k| k.value() == "label"))
        .and_then(|e| e.value().as_string())
        .map(str::to_string)
}

/// What a step's settings can point at, from the rest of the tool.
pub struct Around<'a> {
    pub lists: Lists<'a>,
    /// What earlier steps found.
    pub found: &'a [Found],
    pub numbers: &'a Numbers,
}

/// A step's settings.
pub fn step(
    ui: &mut Ui,
    doc: &KdlDocument,
    at: &StepAt,
    def: Option<&Arc<StepDef>>,
    around: &Around,
    edits: &mut Vec<Edit>,
) {
    let Around {
        lists,
        found,
        numbers,
    } = *around;
    let Some(node) = tool_edit::step_node(doc, at) else {
        return;
    };
    let Some(def) = def else {
        ui.label(
            RichText::new(format!(
                "\"{}\" isn't a step Launcher knows. Its add-on may be missing.",
                node.name().value()
            ))
            .color(ui.visuals().error_fg_color),
        );
        if ui.button("Delete this step").clicked() {
            edits.push(Edit::DeleteStep(at.clone()));
        }
        return;
    };
    ui.horizontal(|ui| {
        app_mark(ui, &def.app);
        ui.vertical(|ui| {
            ui.label(RichText::new(&def.name).strong().size(15.0));
            ui.label(RichText::new(&def.app).weak());
        });
    });
    if let Some(a) = &def.about {
        ui.label(a);
    }
    theme::gap(ui);
    // Its name on the card (and in the run's list).
    ui.label(RichText::new("Name on its card").weak());
    let mut name = label_of(node).unwrap_or_default();
    if ui
        .add(
            egui::TextEdit::singleline(&mut name)
                .hint_text(&def.name)
                .desired_width(f32::INFINITY),
        )
        .changed()
    {
        let v = (!name.trim().is_empty()).then(|| KdlValue::String(name.clone()));
        edits.push(Edit::StepProp(at.clone(), "label".into(), v));
    }
    theme::gap(ui);
    let mut shown: Vec<&SettingDef> = def.settings.iter().filter(|s| !s.advanced).collect();
    shown.sort_by_key(|s| !(s.required && s.starts.is_none()));
    let controls = tool_edit::value_controls(doc);
    let here = Here {
        doc,
        node,
        at,
        def,
        lists,
        found,
        controls: &controls,
        numbers,
    };
    for s in shown {
        setting(ui, &here, s, edits);
        theme::gap_small(ui);
    }
    // If it goes wrong.
    ui.label(RichText::new("If it goes wrong").strong());
    let now = node
        .entries()
        .iter()
        .find(|e| e.name().is_some_and(|k| k.value() == "if-wrong"))
        .and_then(|e| e.value().as_string())
        .unwrap_or("stop")
        .to_string();
    let words = [
        ("stop", "Stop"),
        ("ask", "Ask what to do"),
        ("keep-going", "Keep going"),
    ];
    let shown_word = words
        .iter()
        .find(|(w, _)| *w == now)
        .map_or("Stop", |(_, t)| *t);
    egui::ComboBox::from_id_salt(("if-wrong", at.before, at.index))
        .selected_text(shown_word)
        .show_ui(ui, |ui| {
            for (w, t) in words {
                if ui.selectable_label(now == w, t).clicked() && now != w {
                    let v = (w != "stop").then(|| KdlValue::String(w.into()));
                    edits.push(Edit::StepProp(at.clone(), "if-wrong".into(), v));
                }
            }
        });
    // What it gives back.
    let id = node
        .entries()
        .iter()
        .find(|e| e.name().is_none())
        .and_then(|e| e.value().as_string())
        .unwrap_or_default()
        .to_string();
    if !def.results.is_empty() {
        theme::gap(ui);
        ui.label(RichText::new("This step gives you").strong());
        ui.label(
            RichText::new(
                "Later steps can use it (Use… on their settings). Put it into a control to show \
                 it in the window, where other buttons' steps can use it too.",
            )
            .small()
            .weak(),
        );
        let puts = tool_edit::puts(node);
        for r in &def.results {
            let now = puts
                .iter()
                .find(|(res, _)| *res == r.id)
                .map(|(_, c)| c.clone());
            let shown: egui::WidgetText = match &now {
                Some(c) => {
                    let label = controls
                        .iter()
                        .find(|(id, _, _)| id == c)
                        .map_or(c.clone(), |(_, _, l)| l.clone());
                    rich_line(ui, control_words(ui, numbers, c, &label)).into()
                }
                None => "Nowhere".into(),
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(&r.label);
                ui.label(RichText::new("Put it into").weak());
                egui::ComboBox::from_id_salt(("put", at.before, at.index, &r.id))
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(now.is_none(), "Nowhere").clicked() && now.is_some()
                        {
                            edits.push(Edit::SetPut(at.clone(), r.id.clone(), None));
                        }
                        // Only controls that can hold it: a list fills a drop-down's choices.
                        let fitting: Vec<_> = controls
                            .iter()
                            .filter(|(_, w, _)| r.kind.fills(w))
                            .collect();
                        if fitting.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "No control in the window can hold {}.",
                                    r.kind.words()
                                ))
                                .small()
                                .weak(),
                            );
                        }
                        for (id, word, label) in fitting {
                            let on = now.as_deref() == Some(id.as_str());
                            let mut parts = control_words(ui, numbers, id, label);
                            parts.push(RichText::new(kind_name(word)).weak());
                            let text = rich_line(ui, parts);
                            if ui.selectable_label(on, text).clicked() && !on {
                                edits.push(Edit::SetPut(
                                    at.clone(),
                                    r.id.clone(),
                                    Some(id.clone()),
                                ));
                            }
                        }
                    });
            });
            // Written in the file by hand, or the control changed since.
            if let Some((_, word, label)) = now
                .as_ref()
                .and_then(|c| controls.iter().find(|(id, _, _)| id == c))
            {
                if !r.kind.fills(word) {
                    ui.label(
                        RichText::new(format!(
                            "{} is {}, and {label} can't hold it. Put it somewhere else.",
                            r.label,
                            r.kind.words()
                        ))
                        .small()
                        .color(ui.visuals().error_fg_color),
                    );
                }
            }
        }
    }
    theme::gap(ui);
    egui::CollapsingHeader::new("Advanced")
        .id_salt(("step-advanced", at.before, at.index))
        .show(ui, |ui| {
            for s in def.settings.iter().filter(|s| s.advanced) {
                setting(ui, &here, s, edits);
                theme::gap_small(ui);
            }
            ui.label(RichText::new("Only run it when (script)").weak());
            let mut cond = node
                .entries()
                .iter()
                .find(|e| e.name().is_some_and(|k| k.value() == "if"))
                .map(|e| text_of(e.value()))
                .unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut cond)
                        .hint_text("always")
                        .code_editor()
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                let v = (!cond.trim().is_empty()).then(|| KdlValue::String(cond.clone()));
                edits.push(Edit::StepProp(at.clone(), "if".into(), v));
            }
            ui.label(RichText::new(format!("Its name in the file: {id}")).weak());
            for r in &def.results {
                ui.label(
                    RichText::new(format!("{}: {{{id}.{}}}", r.label, r.id))
                        .weak()
                        .monospace(),
                )
                .on_hover_text("Write this in a later step's setting to use it");
            }
        });
    theme::gap(ui);
    if ui.button("Delete this step").clicked() {
        edits.push(Edit::DeleteStep(at.clone()));
    }
}

/// The step whose settings are shown.
struct Here<'a> {
    doc: &'a KdlDocument,
    node: &'a KdlNode,
    at: &'a StepAt,
    def: &'a StepDef,
    lists: Lists<'a>,
    /// What earlier steps found.
    found: &'a [Found],
    /// The window's controls with a value: (id, kind word, label).
    controls: &'a [(String, String, String)],
    numbers: &'a Numbers,
}

/// The kind word of the control "Use… → a new …" makes for a setting.
fn new_control_word(s: &SettingDef) -> &'static str {
    match s.kind {
        SettingKind::TickBox => "tick-box",
        SettingKind::Number => "number",
        SettingKind::File => "file",
        SettingKind::Folder => "folder",
        SettingKind::Choice => "choice",
        SettingKind::Choices => "choices",
        _ => "text",
    }
}

fn setting(ui: &mut Ui, here: &Here, s: &SettingDef, edits: &mut Vec<Edit>) {
    let Here {
        node,
        found,
        controls,
        ..
    } = *here;
    let needed = s.required && s.starts.is_none();
    let value = tool_edit::step_value(node, &s.id);
    let text = value.as_ref().map(text_of).unwrap_or_default();
    // Where its value comes from: just `{control}`, just `{step.result}`, or typed.
    let source = match text
        .strip_prefix('{')
        .and_then(|t| t.strip_suffix('}'))
        .filter(|t| !t.contains(['{', '}']))
    {
        Some(name) if controls.iter().any(|(id, _, _)| id == name) => {
            Source::Control(name.to_string())
        }
        Some(name) => match found
            .iter()
            .position(|f| format!("{}.{}", f.step, f.result) == name)
        {
            Some(i) => Source::Found(i),
            None => Source::Typed,
        },
        None => Source::Typed,
    };
    let can_use = !matches!(s.kind, SettingKind::List | SettingKind::Table);
    ui.horizontal(|ui| {
        let l = ui.label(RichText::new(&s.label).strong());
        if let Some(h) = &s.help {
            l.on_hover_text(h);
        }
        if needed {
            ui.label(RichText::new("needed").small().weak());
        }
        if can_use {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                use_menu(ui, here, s, &source, edits);
            });
        }
    });
    match &source {
        Source::Typed => value_field(ui, here, s, &value, edits),
        Source::Control(id) => {
            let label = controls
                .iter()
                .find(|(c, _, _)| c == id)
                .map_or(id.clone(), |(_, _, l)| l.clone());
            let mut parts = vec![RichText::new("What people chose in").weak()];
            parts.extend(control_words(ui, here.numbers, id, &label));
            let (show, clear) = used_row(ui, parts, "Show it in the window");
            if show {
                edits.push(Edit::SelectControl(id.clone()));
            }
            if clear {
                edits.push(Edit::StepProp(here.at.clone(), s.id.clone(), None));
            }
            // Written in the file by hand, or the control's kind changed since.
            if let Some((_, word, _)) = controls.iter().find(|(c, _, _)| c == id) {
                let exts = tool_edit::control_extensions(here.doc, id);
                let options = tool_edit::control_options(here.doc, id);
                if !s.takes(word, &exts, &options) {
                    let why = if s.kind.takes(word) && word != "file" {
                        format!(
                            "This takes only {}, and {label} has other options. Give it only \
                             those, or clear this.",
                            s.options.join(", ")
                        )
                    } else if s.kind.takes(word) {
                        let takes = if exts.is_empty() {
                            "any file".to_string()
                        } else {
                            format!("{} files", launcher_core::paths::extension_words(&exts))
                        };
                        format!(
                            "This needs {}, and {label} takes {takes}. Set its extensions to {}, \
                             or clear this.",
                            s.needs(),
                            s.extensions.join(", ")
                        )
                    } else {
                        format!(
                            "This needs {}, and {label} is a {}. Clear it, or use a {}.",
                            s.needs(),
                            kind_name(word),
                            kind_name(new_control_word(s))
                        )
                    };
                    ui.label(
                        RichText::new(why)
                            .small()
                            .color(ui.visuals().error_fg_color),
                    );
                }
            }
        }
        Source::Found(i) => {
            let f = &found[*i];
            let parts = vec![
                RichText::new(format!("What \u{201c}{}\u{201d} found:", f.step_name)).weak(),
                RichText::new(&f.label),
            ];
            let (show, clear) = used_row(ui, parts, "Show that step");
            if show {
                edits.push(Edit::SelectStep(f.at.clone()));
            }
            if clear {
                edits.push(Edit::StepProp(here.at.clone(), s.id.clone(), None));
            }
            // Written in the file by hand, or chosen before results had kinds.
            if !s.takes_result(f.kind) {
                ui.label(
                    RichText::new(format!(
                        "This needs {}, and {} is {}. Clear it, or choose another with Use….",
                        s.needs(),
                        f.label,
                        f.kind.words()
                    ))
                    .small()
                    .color(ui.visuals().error_fg_color),
                );
            }
        }
    }
}

/// What a setting uses (a control, or an earlier step's result) as a link, with Clear beside
/// it. Returns (the link was clicked, Clear was clicked).
fn used_row(ui: &mut Ui, parts: Vec<RichText>, hover: &str) -> (bool, bool) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let clear = ui
                .button("Clear")
                .on_hover_text("Stop using it, and set the value here instead")
                .clicked();
            let show = link(ui, parts).on_hover_text(hover).clicked();
            (show, clear)
        })
        .inner
    })
    .inner
}

/// "Use…": where a setting's value comes from.
fn use_menu(ui: &mut Ui, here: &Here, s: &SettingDef, source: &Source, edits: &mut Vec<Edit>) {
    let Here {
        at,
        found,
        controls,
        ..
    } = *here;
    ui.menu_button("Use…", |ui| {
        ui.set_min_width(240.0);
        if ui
            .add(egui::Button::selectable(
                matches!(source, Source::Typed),
                "Set it here",
            ))
            .on_hover_text("Its own value, the same every time")
            .clicked()
        {
            if !matches!(source, Source::Typed) {
                edits.push(Edit::StepProp(at.clone(), s.id.clone(), None));
            }
            ui.close();
        }
        ui.separator();
        ui.label(RichText::new("What people chose in").weak());
        // Only controls it can take: a folder setting, a folder; a .json file, a file control
        // for .json files.
        let fitting: Vec<_> = controls
            .iter()
            .filter(|(id, word, _)| {
                s.takes(
                    word,
                    &tool_edit::control_extensions(here.doc, id),
                    &tool_edit::control_options(here.doc, id),
                )
            })
            .collect();
        if fitting.is_empty() {
            let kind = kind_name(new_control_word(s));
            let none = if s.kind == SettingKind::File && !s.extensions.is_empty() {
                format!(
                    "No {kind} for {} files in the window yet.",
                    launcher_core::paths::extension_words(&s.extensions)
                )
            } else {
                format!("No {kind} in the window yet.")
            };
            ui.label(RichText::new(none).small().weak());
        }
        for (id, word, label) in fitting {
            let on = matches!(source, Source::Control(c) if c == id);
            let mut parts = control_words(ui, here.numbers, id, label);
            parts.push(RichText::new(kind_name(word)).weak());
            let text = rich_line(ui, parts);
            if ui.add(egui::Button::selectable(on, text)).clicked() {
                if !on {
                    edits.push(Edit::StepProp(
                        at.clone(),
                        s.id.clone(),
                        Some(KdlValue::String(format!("{{{id}}}"))),
                    ));
                }
                ui.close();
            }
        }
        let new = format!("+ A new {} for this", kind_name(new_control_word(s)));
        if ui.button(new).clicked() {
            edits.push(Edit::Ask(at.clone(), s.id.clone()));
            ui.close();
        }
        ui.separator();
        ui.label(RichText::new("What an earlier step found").weak());
        // Only what it can take: a folder setting, not a count.
        let fitting: Vec<(usize, &Found)> = found
            .iter()
            .enumerate()
            .filter(|(_, f)| s.takes_result(f.kind))
            .collect();
        if found.is_empty() {
            ui.label(
                RichText::new("No earlier step gives anything back.")
                    .small()
                    .weak(),
            );
        } else if fitting.is_empty() {
            ui.label(
                RichText::new(format!("No earlier step gives back {}.", s.needs()))
                    .small()
                    .weak(),
            );
        }
        for (i, f) in fitting {
            let on = matches!(source, Source::Found(k) if *k == i);
            let text = format!("{} \u{203a} {}", f.step_name, f.label);
            if ui.add(egui::Button::selectable(on, text)).clicked() {
                if !on {
                    edits.push(Edit::StepProp(
                        at.clone(),
                        s.id.clone(),
                        Some(KdlValue::String(format!("{{{}.{}}}", f.step, f.result))),
                    ));
                }
                ui.close();
            }
        }
    });
}

/// The value a setting always has.
fn value_field(
    ui: &mut Ui,
    here: &Here,
    s: &SettingDef,
    value: &Option<KdlValue>,
    edits: &mut Vec<Edit>,
) {
    let Here {
        node,
        at,
        def,
        lists,
        ..
    } = *here;
    let key = |what: &str| (what.to_string(), at.before, at.index, s.id.clone());
    match s.kind {
        SettingKind::TickBox => {
            let mut on = match &value {
                Some(KdlValue::Bool(b)) => *b,
                _ => matches!(s.starts, Some(launcher_core::dynval::Dyn::Bool(true))),
            };
            if ui.checkbox(&mut on, "Yes").changed() {
                edits.push(Edit::StepProp(
                    at.clone(),
                    s.id.clone(),
                    Some(KdlValue::Bool(on)),
                ));
            }
        }
        SettingKind::Choice if s.list_from.is_some() => {
            let now = value.as_ref().map(text_of).unwrap_or_default();
            let answer = lists(&list_for(node, def, s.list_from.as_ref().expect("checked")));
            match answer {
                ListAnswer::Items(items) => {
                    egui::ComboBox::from_id_salt(key("list-choice"))
                        .width(ui.available_width())
                        .selected_text(if now.is_empty() {
                            RichText::new("Choose one…").weak()
                        } else {
                            RichText::new(now.clone())
                        })
                        .show_ui(ui, |ui| {
                            if items.is_empty() {
                                ui.label(RichText::new("The list is empty.").weak());
                            }
                            for o in &items {
                                if ui.selectable_label(&now == o, o).clicked() && &now != o {
                                    edits.push(Edit::StepProp(
                                        at.clone(),
                                        s.id.clone(),
                                        Some(KdlValue::String(o.clone())),
                                    ));
                                }
                            }
                        });
                }
                other => {
                    let note = match other {
                        ListAnswer::Loading => "Getting the list…".to_string(),
                        ListAnswer::Failed(why) => why,
                        ListAnswer::Items(_) => unreachable!(),
                    };
                    let mut text = now.clone();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut text)
                                .id_salt(key("text"))
                                .hint_text("Type it")
                                .desired_width(f32::INFINITY),
                        )
                        .changed()
                    {
                        let v = (!text.is_empty()).then(|| KdlValue::String(text.clone()));
                        edits.push(Edit::StepProp(at.clone(), s.id.clone(), v));
                    }
                    ui.label(RichText::new(note).small().weak());
                }
            }
        }
        SettingKind::Choice if !s.options.is_empty() => {
            let now = value
                .as_ref()
                .map(text_of)
                .or_else(|| s.starts.as_ref().map(|d| d.to_text()))
                .unwrap_or_default();
            egui::ComboBox::from_id_salt(key("choice"))
                .selected_text(if now.is_empty() {
                    "Choose…".to_string()
                } else {
                    now.clone()
                })
                .show_ui(ui, |ui| {
                    for o in &s.options {
                        if ui.selectable_label(&now == o, o).clicked() && &now != o {
                            edits.push(Edit::StepProp(
                                at.clone(),
                                s.id.clone(),
                                Some(KdlValue::String(o.clone())),
                            ));
                        }
                    }
                });
        }
        SettingKind::List => {
            let items: Vec<String> = node
                .children()
                .and_then(|c| c.nodes().iter().find(|n| n.name().value() == s.id))
                .map(|n| n.entries().iter().map(|e| text_of(e.value())).collect())
                .unwrap_or_default();
            let mut text = items.join("\n");
            ui.label(RichText::new("One per line").small().weak());
            if ui
                .add(
                    egui::TextEdit::multiline(&mut text)
                        .id_salt(key("list"))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                let items = text
                    .lines()
                    .map(str::to_string)
                    .filter(|l| !l.is_empty())
                    .collect();
                edits.push(Edit::StepList(at.clone(), s.id.clone(), items));
            }
        }
        SettingKind::Table => {
            ui.label(
                RichText::new("Edit this in the file's text (More menu).")
                    .small()
                    .weak(),
            );
        }
        _ => {
            let mut text = value.as_ref().map(text_of).unwrap_or_default();
            let hint = match (&s.starts, s.kind) {
                (Some(d), SettingKind::Folder | SettingKind::File) if d.to_text() == "." => {
                    "Same folder as this tool".to_string()
                }
                (Some(d), _) => d.to_text(),
                (None, SettingKind::Choice) => "Type it".into(),
                (None, SettingKind::File) if !s.extensions.is_empty() => format!(
                    "A {} file",
                    launcher_core::paths::extension_words(&s.extensions)
                ),
                (None, _) => String::new(),
            };
            let edit = if s.kind == SettingKind::LongText {
                egui::TextEdit::multiline(&mut text).desired_rows(3)
            } else {
                egui::TextEdit::singleline(&mut text)
            };
            if ui
                .add(
                    edit.id_salt(key("text"))
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                let v = if text.is_empty() {
                    None
                } else if s.kind == SettingKind::Number {
                    Some(
                        text.trim()
                            .parse::<f64>()
                            .map(KdlValue::Float)
                            .unwrap_or(KdlValue::String(text.clone())),
                    )
                } else {
                    Some(KdlValue::String(text.clone()))
                };
                edits.push(Edit::StepProp(at.clone(), s.id.clone(), v));
            }
            // What would go wrong with what's typed, said before the tool runs.
            if let Some(problem) = (!text.contains('{'))
                .then(|| s.check_value(&text))
                .flatten()
            {
                ui.label(
                    RichText::new(format!("This {problem}."))
                        .small()
                        .color(ui.visuals().error_fg_color),
                );
            }
        }
    }
}

/// A list's settings as this step has them: `folder="{folder}"` reads the step's own Project
/// folder (its value, or where it starts).
fn list_for(node: &KdlNode, def: &StepDef, l: &ListUse) -> ListUse {
    let settings = l
        .settings
        .iter()
        .map(|(k, v)| {
            let own = match v {
                Val::Text(t) => t
                    .raw
                    .trim()
                    .strip_prefix('{')
                    .and_then(|r| r.strip_suffix('}'))
                    .and_then(|name| def.setting(name.trim())),
                Val::Lua(_) => None,
            };
            let v = match own {
                Some(st) => tool_edit::step_value(node, &st.id)
                    .map(|v| text_of(&v))
                    .or_else(|| st.starts.as_ref().map(|d| d.to_text()))
                    .map_or_else(|| v.clone(), |t| Val::literal(&t)),
                None => v.clone(),
            };
            (k.clone(), v)
        })
        .collect();
    ListUse {
        list: l.list.clone(),
        settings,
    }
}

/// The plain name of a control's kind.
pub fn kind_name(word: &str) -> &'static str {
    match word {
        "choice" => "Drop Down Button",
        "choices" => "Multi Choice Tick Box",
        "tick-box" => "Tick Box",
        "text" => "Text Input",
        "number" => "Number Input",
        "file" => "File",
        "folder" => "Folder",
        "display" => "Info",
        "button" => "Button",
        "info" => "Text Label",
        "line" => "Line",
        "section" => "Section",
        "pages" => "Pages",
        "page" => "Page",
        _ => "Control",
    }
}

/// A control's settings.
/// `step_name` gives a step's name as its card shows it.
/// A control's settings. `conflicts`: steps that use it but can't take it as it is now (its
/// extensions or options changed), one sentence each.
pub fn control(
    ui: &mut Ui,
    doc: &KdlDocument,
    id: &str,
    step_name: &dyn Fn(&KdlNode) -> String,
    numbers: &Numbers,
    conflicts: &[String],
    edits: &mut Vec<Edit>,
) {
    let Some(n) = tool_edit::control(doc, id) else {
        return;
    };
    let word = n.name().value().to_string();
    let prop = |k: &str| {
        n.entries()
            .iter()
            .find(|e| e.name().is_some_and(|x| x.value() == k))
            .map(|e| e.value().clone())
    };
    ui.horizontal(|ui| {
        if let Some(n) = numbers.get(id) {
            ui.label(number_text(ui, *n));
        }
        ui.label(RichText::new(kind_name(&word)).strong().size(15.0));
    });
    theme::gap(ui);
    ui.label(RichText::new("Label").weak());
    let mut label = prop("label").map(|v| text_of(&v)).unwrap_or_default();
    if ui
        .add(egui::TextEdit::singleline(&mut label).desired_width(f32::INFINITY))
        .changed()
    {
        edits.push(Edit::ControlProp(
            id.into(),
            "label".into(),
            Some(KdlValue::String(label.clone())),
        ));
    }
    if word == "button" {
        theme::gap(ui);
        ui.label("Pressing it runs its steps.");
        if ui.button("Show its steps").clicked() {
            edits.push(Edit::ShowSteps(id.into()));
        }
    } else {
        ui.label(
            RichText::new("Steps use what people choose here with Use… on their settings.")
                .small()
                .weak(),
        );
        theme::gap(ui);
        ui.label(RichText::new("Help when hovering").weak());
        let mut help = prop("help").map(|v| text_of(&v)).unwrap_or_default();
        if ui
            .add(egui::TextEdit::singleline(&mut help).desired_width(f32::INFINITY))
            .changed()
        {
            let v = (!help.is_empty()).then(|| KdlValue::String(help.clone()));
            edits.push(Edit::ControlProp(id.into(), "help".into(), v));
        }
        if matches!(word.as_str(), "choice" | "choices") {
            theme::gap(ui);
            let fillers = tool_edit::filled_by(doc, id);
            let buttons = tool_edit::buttons(doc);
            for at in &fillers {
                let Some(step) = tool_edit::step_node(doc, at) else {
                    continue;
                };
                let button = buttons
                    .iter()
                    .find(|(b, _)| *b == at.steps)
                    .map_or(at.steps.clone(), |(_, l)| l.clone());
                ui.label(RichText::new("Its choices are filled by").weak());
                let mut parts = vec![
                    RichText::new(format!("\u{201c}{}\u{201d}", step_name(step))),
                    RichText::new("when").weak(),
                ];
                parts.extend(control_words(ui, numbers, &at.steps, &button));
                parts.push(RichText::new("is pressed").weak());
                if link(ui, parts).on_hover_text("Show that step").clicked() {
                    edits.push(Edit::SelectStep(at.clone()));
                }
            }
            if fillers.is_empty() {
                choices(ui, n, id, edits);
                ui.label(
                    RichText::new(
                        "Or let a step fill them: pick the step, then \u{201c}Put it into\u{201d} \
                         this.",
                    )
                    .small()
                    .weak(),
                );
            } else {
                ui.label(RichText::new("Until then it shows these:").small().weak());
                choices(ui, n, id, edits);
            }
        }
        if word == "file" {
            theme::gap(ui);
            ui.label(RichText::new("Supported extensions").weak());
            let mut exts = prop("extensions").map(|v| text_of(&v)).unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut exts)
                        .hint_text("Any file")
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                let v = (!exts.trim().is_empty()).then(|| KdlValue::String(exts.clone()));
                edits.push(Edit::ControlProp(id.into(), "extensions".into(), v));
            }
            ui.label(
                RichText::new(
                    "Like json, fbx. Browse… shows only these, and another kind of file isn't \
                     taken.",
                )
                .small()
                .weak(),
            );
        }
        if word == "choice" {
            let buttons = prop("show-as").is_some_and(|v| v.as_string() == Some("buttons"));
            let mut b = buttons;
            if ui.checkbox(&mut b, "Show as buttons").changed() {
                let v = b.then(|| KdlValue::String("buttons".into()));
                edits.push(Edit::ControlProp(id.into(), "show-as".into(), v));
            }
        }
        theme::gap(ui);
        ui.label(RichText::new("Starts as").weak());
        if word == "tick-box" {
            let mut on = matches!(prop("starts"), Some(KdlValue::Bool(true)));
            if ui.checkbox(&mut on, "Ticked").changed() {
                edits.push(Edit::ControlProp(
                    id.into(),
                    "starts".into(),
                    Some(KdlValue::Bool(on)),
                ));
            }
        } else if word == "choice" && !tool_edit::control_options(doc, id).is_empty() {
            // One of its options, picked from them.
            let now = prop("starts").map(|v| text_of(&v)).unwrap_or_default();
            egui::ComboBox::from_id_salt(("control-starts", id))
                .width(ui.available_width())
                .selected_text(if now.is_empty() {
                    RichText::new("The first one").weak()
                } else {
                    RichText::new(now.clone())
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(now.is_empty(), "The first one")
                        .clicked()
                        && !now.is_empty()
                    {
                        edits.push(Edit::ControlProp(id.into(), "starts".into(), None));
                    }
                    for o in tool_edit::control_options(doc, id) {
                        if ui.selectable_label(now == o, &o).clicked() && now != o {
                            edits.push(Edit::ControlProp(
                                id.into(),
                                "starts".into(),
                                Some(KdlValue::String(o.clone())),
                            ));
                        }
                    }
                });
        } else {
            let mut starts = prop("starts").map(|v| text_of(&v)).unwrap_or_default();
            if ui
                .add(egui::TextEdit::singleline(&mut starts).desired_width(f32::INFINITY))
                .changed()
            {
                let v = (!starts.is_empty()).then(|| KdlValue::String(starts.clone()));
                edits.push(Edit::ControlProp(id.into(), "starts".into(), v));
            }
            // What it starts as is something it can hold.
            let t = starts.trim();
            let options = tool_edit::control_options(doc, id);
            let exts = tool_edit::control_extensions(doc, id);
            let wrong = if t.is_empty() || t.contains('{') {
                None
            } else if word == "number" && t.parse::<f64>().is_err() {
                Some("This should be a number.".to_string())
            } else if word == "choices" && !options.is_empty() {
                t.split(',')
                    .map(str::trim)
                    .find(|x| !x.is_empty() && !options.iter().any(|o| o == x))
                    .map(|x| format!("{x} isn't one of its options."))
            } else if word == "file" && !launcher_core::paths::has_extension(t, &exts) {
                Some(format!(
                    "This should be a {} file.",
                    launcher_core::paths::extension_words(&exts)
                ))
            } else {
                None
            };
            if let Some(w) = wrong {
                ui.label(RichText::new(w).small().color(ui.visuals().error_fg_color));
            }
        }
        theme::gap(ui);
        let mut required = matches!(prop("required"), Some(KdlValue::Bool(true)));
        if ui
            .checkbox(&mut required, "Must be filled in")
            .on_hover_text("The buttons stay off until it is")
            .changed()
        {
            edits.push(Edit::ControlProp(
                id.into(),
                "required".into(),
                required.then_some(KdlValue::Bool(true)),
            ));
        }
        let mut remember = !matches!(prop("remember"), Some(KdlValue::Bool(false)));
        if ui
            .checkbox(&mut remember, "Remember the last value")
            .changed()
        {
            edits.push(Edit::ControlProp(
                id.into(),
                "remember".into(),
                (!remember).then_some(KdlValue::Bool(false)),
            ));
        }
    }
    // Steps that use it but can't, as it is now.
    if !conflicts.is_empty() {
        theme::gap(ui);
        for c in conflicts {
            ui.label(RichText::new(c).small().color(ui.visuals().error_fg_color));
        }
    }
    theme::gap(ui);
    ui.horizontal(|ui| {
        if ui.button("Move up").clicked() {
            edits.push(Edit::MoveControl(id.into(), -1));
        }
        if ui.button("Move down").clicked() {
            edits.push(Edit::MoveControl(id.into(), 1));
        }
    });
    theme::gap(ui);
    if ui.button("Delete this control").clicked() {
        edits.push(Edit::DeleteControl(id.into()));
    }
}

/// The settings of text, a line or a section in the window.
/// The whole tool, while nothing is selected.
pub fn tool(ui: &mut Ui, doc: &KdlDocument, edits: &mut Vec<Edit>) {
    ui.label(RichText::new("This tool").strong().size(15.0));
    ui.label(RichText::new("Settings for the whole tool.").weak());
    theme::gap(ui);
    let mut on = tool_edit::saves_logs(doc);
    if ui
        .checkbox(&mut on, SAVE_LOGS)
        .on_hover_text(SAVE_LOGS_HELP)
        .changed()
    {
        edits.push(Edit::SaveLogs(on));
    }
    let what = if on {
        "Each run's log is also saved on this computer (the last 20 runs). \"Log files\", beside the window, opens them."
    } else {
        "A run's log only shows beside the window, and goes when it's closed."
    };
    ui.add(egui::Label::new(RichText::new(what).weak()).wrap());
}

/// The tool's "save logs" choice, as the settings and the More menu word it.
pub const SAVE_LOGS: &str = "Save each run's log to files";
pub const SAVE_LOGS_HELP: &str =
    "Keep what each run printed in files on this computer, as well as beside the window";

pub fn item(ui: &mut Ui, doc: &KdlDocument, path: &[usize], edits: &mut Vec<Edit>) {
    let Some(n) = tool_edit::item(doc, path) else {
        return;
    };
    let word = n.name().value();
    let text = n
        .entries()
        .iter()
        .find(|e| e.name().is_none())
        .map(|e| text_of(e.value()))
        .unwrap_or_default();
    let p = path.to_vec();
    let title = kind_name(word);
    let about = match word {
        "info" => "Words people read in the window.",
        "line" => "A line across the window, between its parts.",
        "section" => "A box around some controls, with a title. Drag controls into it.",
        "pages" => "Pages people switch between.",
        "page" => "One of the pages.",
        _ => "",
    };
    ui.label(RichText::new(title).strong().size(15.0));
    ui.label(RichText::new(about).weak());
    theme::gap(ui);
    match word {
        "info" => {
            ui.label(RichText::new("Text").weak());
            let mut t = text.clone();
            if ui
                .add(
                    egui::TextEdit::multiline(&mut t)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                edits.push(Edit::ItemArg(p.clone(), t));
            }
            theme::gap_small(ui);
            ui.label(RichText::new("Looks").weak());
            let now = n
                .entries()
                .iter()
                .find(|e| e.name().is_some_and(|k| k.value() == "style"))
                .map(|e| text_of(e.value()))
                .unwrap_or_else(|| "normal".into());
            let looks = [
                ("normal", "Plain"),
                ("muted", "Quiet"),
                ("warning", "Warning"),
                ("error", "Problem"),
            ];
            let shown = looks
                .iter()
                .find(|(w, _)| *w == now)
                .map_or("Plain", |(_, l)| *l);
            egui::ComboBox::from_id_salt(("info-style", path))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (w, l) in looks {
                        if ui.selectable_label(now == w, l).clicked() && now != w {
                            let v = (w != "normal").then(|| KdlValue::String(w.into()));
                            edits.push(Edit::ItemProp(p.clone(), "style".into(), v));
                        }
                    }
                });
        }
        "section" | "page" => {
            ui.label(RichText::new("Title").weak());
            let mut t = text.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut t).desired_width(f32::INFINITY))
                .changed()
            {
                edits.push(Edit::ItemArg(p.clone(), t));
            }
        }
        _ => {}
    }
    theme::gap(ui);
    ui.horizontal(|ui| {
        if ui.button("Move up").clicked() {
            edits.push(Edit::MoveItem(p.clone(), -1));
        }
        if ui.button("Move down").clicked() {
            edits.push(Edit::MoveItem(p.clone(), 1));
        }
    });
    theme::gap(ui);
    let delete = match word {
        "section" | "page" => "Delete this section (keeps what's in it)",
        _ => "Delete this",
    };
    if ui.button(delete).clicked() {
        edits.push(Edit::DeleteItem(p));
    }
}

/// A choice's options, or where its list comes from.
fn choices(ui: &mut Ui, n: &KdlNode, id: &str, edits: &mut Vec<Edit>) {
    let kids: Vec<&KdlNode> = n
        .children()
        .map(|c| c.nodes().iter().collect())
        .unwrap_or_default();
    if let Some(l) = kids.iter().find(|k| k.name().value() == "list-from") {
        let list = l
            .entries()
            .first()
            .map(|e| text_of(e.value()))
            .unwrap_or_default();
        ui.label(RichText::new("Its choices come from").weak());
        ui.label(list);
        return;
    }
    ui.label(RichText::new("Choices").weak());
    let mut options: Vec<String> = kids
        .iter()
        .filter(|k| k.name().value() == "option")
        .filter_map(|k| k.entries().first().map(|e| text_of(e.value())))
        .collect();
    let mut changed = false;
    let mut remove = None;
    for (i, o) in options.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(o)
                        .id_salt(("option", id, i))
                        .desired_width(180.0),
                )
                .changed()
            {
                changed = true;
            }
            if ui.button("Remove").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        options.remove(i);
        changed = true;
    }
    if ui.button("+ Add a choice").clicked() {
        options.push(format!("Choice {}", options.len() + 1));
        changed = true;
    }
    if changed {
        edits.push(Edit::ControlOptions(id.into(), options));
    }
}

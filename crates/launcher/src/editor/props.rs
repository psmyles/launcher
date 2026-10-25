//! The editor's properties (M8), generated from the catalog: the selected node's arguments and
//! properties with a widget per type, its notes (the comment lines above it), a table for a data
//! bag's keys, and the live value of each template. Every change is a splice of the file's text
//! (`config::edit`); nothing is re-printed.

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use launcher_core::config::catalog::{self, Children, Os, Param, Ty};
use launcher_core::config::edit::{self, string_repr, Repr};
use launcher_core::config::template::Template;
use launcher_core::config::{Config, Diagnostic, NodePath};
use launcher_core::eval::render;
use launcher_core::lua::Snapshot;

use super::luaedit::{self, Names};
use crate::ui::theme;

/// A new text for the file. `merge`: edits with the same key in a row are one undo step (typing
/// into one field).
pub struct Change {
    pub text: String,
    pub merge: Option<String>,
}

pub struct PropsCx<'a> {
    pub text: &'a str,
    pub doc: &'a KdlDocument,
    pub config: &'a Config,
    /// The preview's values, for templates' live values.
    pub snapshot: &'a Snapshot,
    pub env: &'a BTreeMap<String, String>,
    /// Problems with file lines (Lua lint shows under its field).
    pub issues: &'a [Diagnostic],
    pub names: &'a Names<'a>,
}

enum Key {
    Prop(&'static str),
    Arg(usize),
}

impl Key {
    fn merge(&self, path: &NodePath) -> String {
        match self {
            Key::Prop(n) => format!("{path:?}.{n}"),
            Key::Arg(i) => format!("{path:?}#{i}"),
        }
    }
}

fn prop_entry<'a>(n: &'a KdlNode, name: &str) -> Option<&'a KdlEntry> {
    n.entries()
        .iter()
        .rev()
        .find(|e| e.name().is_some_and(|k| k.value() == name))
}

fn args(n: &KdlNode) -> Vec<&KdlEntry> {
    n.entries().iter().filter(|e| e.name().is_none()).collect()
}

/// An entry's value as the field shows it, and whether it is `(lua)`.
fn value_text(e: &KdlEntry) -> (String, bool) {
    let lua = e.ty().is_some_and(|t| t.value() == "lua");
    let text = match e.value() {
        KdlValue::String(s) => s.clone(),
        KdlValue::Bool(b) => if *b { "#true" } else { "#false" }.into(),
        KdlValue::Null => String::new(),
        other => other.to_string(),
    };
    (text, lua)
}

/// The file lines an entry covers.
fn lines_of(text: &str, e: &KdlEntry) -> (usize, usize) {
    let start = e.span().offset().min(text.len());
    let end = (e.span().offset() + e.span().len()).min(text.len());
    let first = text[..start].matches('\n').count() + 1;
    (first, first + text[start..end].matches('\n').count())
}

fn is_number(s: &str) -> bool {
    s.trim().parse::<f64>().is_ok()
}

/// What to write for text `t` typed into a field of type `ty` (`lua`: as `(lua)"..."`).
fn repr_for(ty: Ty, t: &str, lua: bool) -> String {
    if lua || ty == Ty::LuaValue {
        return format!("(lua){}", string_repr(t));
    }
    match ty {
        Ty::Int | Ty::Number if is_number(t) => t.trim().to_string(),
        Ty::Width if is_number(t) => t.trim().to_string(),
        Ty::Any if is_number(t) || t == "#true" || t == "#false" => t.to_string(),
        _ => string_repr(t),
    }
}

fn write(
    cx: &PropsCx,
    path: &NodePath,
    key: &Key,
    value: Option<String>,
) -> Result<String, String> {
    let repr = value.as_deref().map(Repr::Raw);
    match key {
        Key::Prop(name) => edit::set_prop(cx.text, path, name, repr),
        Key::Arg(i) => edit::set_arg(cx.text, path, *i, repr),
    }
}

/// Ids of a kind of thing, for a reference picker.
fn refs(config: &Config, ty: Ty) -> Vec<String> {
    match ty {
        Ty::ControlRef => config.controls.iter().map(|c| c.id.clone()).collect(),
        Ty::ActionRef => config.actions.iter().map(|a| a.id.clone()).collect(),
        Ty::BagRef => config
            .data
            .iter()
            .map(|d| d.id.clone())
            .chain(
                config
                    .controls
                    .iter()
                    .filter(|c| matches!(c.kind, launcher_core::config::ControlKind::Choice { .. }))
                    .map(|c| c.id.clone()),
            )
            .collect(),
        Ty::RunRef => config
            .actions
            .iter()
            .flat_map(|a| a.steps.iter())
            .filter(|s| matches!(s.kind, launcher_core::config::StepKind::Run(_)))
            .filter_map(|s| s.id.clone())
            .collect(),
        _ => Vec::new(),
    }
}

/// One parameter's widget. Returns a change when the user made one.
fn field(
    ui: &mut Ui,
    cx: &PropsCx,
    path: &NodePath,
    key: Key,
    p: &Param,
    entry: Option<&KdlEntry>,
) -> Option<Result<Change, String>> {
    let (current, is_lua) = entry.map(value_text).unwrap_or_default();
    let merge = Some(key.merge(path));
    let id = ui.id().with(key.merge(path));
    let mut out: Option<Result<String, String>> = None;
    ui.horizontal(|ui| {
        let mut name = RichText::new(p.name).strong();
        if p.required {
            name = name.color(ui.visuals().hyperlink_color);
        }
        ui.label(name).on_hover_text(p.doc);
        if p.os != Os::Any {
            ui.label(
                RichText::new(format!("({} only)", p.os.name()))
                    .small()
                    .weak(),
            );
        }
        if !p.default.is_empty() && entry.is_none() {
            ui.label(
                RichText::new(format!("default {}", p.default))
                    .small()
                    .weak(),
            );
        }
        // A template (or number, or any value) may be Lua instead.
        if p.ty.accepts_lua() && p.ty != Ty::LuaValue {
            let mut lua = is_lua;
            if ui
                .checkbox(&mut lua, RichText::new("Lua").small())
                .on_hover_text("Work the value out with a Lua expression")
                .changed()
            {
                out = Some(write(cx, path, &key, Some(repr_for(p.ty, &current, lua))));
            }
        }
        if entry.is_some() && !p.required && ui.small_button("✕").on_hover_text("Remove").clicked()
        {
            out = Some(write(cx, path, &key, None));
        }
    });
    if out.is_some() {
        return out.map(|r| r.map(|text| Change { text, merge: None }));
    }
    let lua_field = is_lua || matches!(p.ty, Ty::Lua | Ty::LuaChunk | Ty::LuaValue);
    let mut edited: Option<Option<String>> = None;
    match p.ty {
        _ if lua_field => {
            let mut code = current.clone();
            let rows = if matches!(p.ty, Ty::LuaChunk | Ty::LuaValue) || code.contains('\n') {
                code.lines().count().clamp(4, 24)
            } else {
                1
            };
            let issues: Vec<(String, bool)> = entry
                .map(|e| {
                    let (a, b) = lines_of(cx.text, e);
                    cx.issues
                        .iter()
                        .filter(|d| (a..=b).contains(&d.line))
                        .map(|d| (format!("line {}: {}", d.line, d.message), d.is_error()))
                        .collect()
                })
                .unwrap_or_default();
            if luaedit::show(ui, id, &mut code, rows, cx.names, &issues) {
                edited = Some(Some(repr_for(p.ty, &code, is_lua)));
            }
        }
        Ty::Bool => {
            let now = entry.and_then(|e| e.value().as_bool());
            let shown = match now {
                Some(true) => "yes",
                Some(false) => "no",
                None => "(default)",
            };
            egui::ComboBox::from_id_salt(id)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(now.is_none(), "(default)").clicked() {
                        edited = Some(None);
                    }
                    if ui.selectable_label(now == Some(true), "yes").clicked() {
                        edited = Some(Some("#true".into()));
                    }
                    if ui.selectable_label(now == Some(false), "no").clicked() {
                        edited = Some(Some("#false".into()));
                    }
                });
        }
        Ty::Enum(words) => {
            let shown = if entry.is_some() {
                current.clone()
            } else {
                "(default)".into()
            };
            egui::ComboBox::from_id_salt(id)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if !p.required && ui.selectable_label(entry.is_none(), "(default)").clicked() {
                        edited = Some(None);
                    }
                    for w in words {
                        if ui
                            .selectable_label(entry.is_some() && current == *w, *w)
                            .clicked()
                        {
                            edited = Some(Some(string_repr(w)));
                        }
                    }
                });
        }
        Ty::ControlRef | Ty::ActionRef | Ty::BagRef | Ty::RunRef => {
            let ids = refs(cx.config, p.ty);
            ui.horizontal(|ui| {
                let mut t = current.clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut t)
                            .desired_width(160.0)
                            .id(id),
                    )
                    .changed()
                {
                    edited = Some(Some(string_repr(&t)));
                }
                egui::ComboBox::from_id_salt(id.with("pick"))
                    .selected_text("pick…")
                    .show_ui(ui, |ui| {
                        for r in &ids {
                            if ui.selectable_label(*r == current, r).clicked() {
                                edited = Some(Some(string_repr(r)));
                            }
                        }
                    });
            });
        }
        _ => {
            let mut t = current.clone();
            let multiline = t.contains('\n');
            let edit = if multiline {
                egui::TextEdit::multiline(&mut t)
            } else {
                egui::TextEdit::singleline(&mut t)
            };
            let r = ui.add(edit.desired_width(ui.available_width()).id(id));
            if r.changed() {
                edited = Some(if t.is_empty() && !p.required {
                    None
                } else {
                    Some(repr_for(p.ty, &t, false))
                });
            }
            // A template's live value, with the preview's current values.
            if p.ty == Ty::Template && t.contains('{') {
                if let Ok(tpl) = Template::parse(&t) {
                    let v = render(&tpl, cx.snapshot, cx.env);
                    ui.label(RichText::new(format!("= {v}")).small().weak());
                }
            }
        }
    }
    edited.map(|v| write(cx, path, &key, v).map(|text| Change { text, merge }))
}

/// A data bag's keys (a `data` block, an option's fields) as a table.
fn bag(ui: &mut Ui, cx: &PropsCx, path: &NodePath, n: &KdlNode) -> Option<Result<Change, String>> {
    let mut out = None;
    ui.label(RichText::new("Fields").strong());
    let kids = n.children().map(|c| c.nodes()).unwrap_or(&[]);
    egui::Grid::new(("bag", path))
        .num_columns(3)
        .show(ui, |ui| {
            for (i, k) in kids.iter().enumerate() {
                let mut p = path.clone();
                p.push(i);
                let mut key = k.name().value().to_string();
                if ui
                    .add(egui::TextEdit::singleline(&mut key).desired_width(140.0))
                    .changed()
                    && !key.trim().is_empty()
                {
                    out = Some(
                        edit::rename_node(cx.text, &p, key.trim()).map(|text| Change {
                            text,
                            merge: Some(format!("{p:?}.key")),
                        }),
                    );
                }
                let mut value = k
                    .entries()
                    .first()
                    .map(|e| value_text(e).0)
                    .unwrap_or_default();
                if ui
                    .add(egui::TextEdit::singleline(&mut value).desired_width(220.0))
                    .changed()
                {
                    out = Some(edit::set_arg(cx.text, &p, 0, Some(Repr::Str(&value))).map(
                        |text| Change {
                            text,
                            merge: Some(format!("{p:?}.value")),
                        },
                    ));
                }
                if ui
                    .small_button("✕")
                    .on_hover_text("Remove this field")
                    .clicked()
                {
                    out = Some(
                        edit::remove_node(cx.text, &p).map(|text| Change { text, merge: None }),
                    );
                }
                ui.end_row();
            }
        });
    if ui.button("+ Field").clicked() {
        let taken = kids.iter().map(|k| k.name().value().to_string()).collect();
        let key = edit::fresh_id("Key", &taken);
        out = Some(
            edit::insert_node(cx.text, Some(path), kids.len(), &format!("{key} \"\""))
                .map(|(text, _)| Change { text, merge: None }),
        );
    }
    out
}

pub fn show(ui: &mut Ui, cx: &PropsCx, path: &NodePath) -> Option<Result<Change, String>> {
    let n = edit::node_at(cx.doc, path)?;
    let kind = n.name().value();
    let mut out = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let Some(spec) = catalog::get(kind) else {
                ui.heading(kind);
                ui.label(
                    RichText::new(format!(
                        "`{kind}` is not something a .launcher file knows here; see Problems."
                    ))
                    .color(ui.visuals().warn_fg_color),
                );
                return;
            };
            ui.heading(kind);
            if !spec.doc.is_empty() {
                ui.label(RichText::new(spec.doc).weak());
            }
            ui.add_space(6.0);
            // Notes: the comment lines above the node.
            let notes = edit::notes(cx.text, path).unwrap_or_default();
            let mut t = notes.clone();
            ui.label(RichText::new("Notes").strong())
                .on_hover_text("The // comment lines above this node in the file");
            if ui
                .add(
                    egui::TextEdit::multiline(&mut t)
                        .desired_rows(1)
                        .desired_width(ui.available_width()),
                )
                .changed()
            {
                out = Some(edit::set_notes(cx.text, path, &t).map(|text| Change {
                    text,
                    merge: Some(format!("{path:?}.notes")),
                }));
            }
            ui.add_space(6.0);
            let present = args(n);
            for (i, p) in spec.args.iter().enumerate() {
                if let Some(c) = field(ui, cx, path, Key::Arg(i), p, present.get(i).copied()) {
                    out = Some(c);
                }
                theme::gap_small(ui);
            }
            if let Some(rest) = &spec.rest {
                ui.label(RichText::new(format!("{}s", rest.name)).strong())
                    .on_hover_text(rest.doc);
                let mut remove = None;
                for (i, entry) in present.iter().enumerate().skip(spec.args.len()) {
                    ui.horizontal(|ui| {
                        let (mut t, _) = value_text(entry);
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut t)
                                    .desired_width(ui.available_width() - 30.0),
                            )
                            .changed()
                        {
                            out = Some(edit::set_arg(cx.text, path, i, Some(Repr::Str(&t))).map(
                                |text| Change {
                                    text,
                                    merge: Some(format!("{path:?}#{i}")),
                                },
                            ));
                        }
                        if ui.small_button("✕").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    out = Some(
                        edit::set_arg(cx.text, path, i, None)
                            .map(|text| Change { text, merge: None }),
                    );
                }
                if ui.button(format!("+ {}", rest.name)).clicked() {
                    out = Some(
                        edit::set_arg(
                            cx.text,
                            path,
                            present.len().max(spec.args.len()),
                            Some(Repr::Str("")),
                        )
                        .map(|text| Change { text, merge: None }),
                    );
                }
                theme::gap_small(ui);
            }
            for p in spec.all_props() {
                if let Some(c) = field(ui, cx, path, Key::Prop(p.name), p, prop_entry(n, p.name)) {
                    out = Some(c);
                }
                theme::gap_small(ui);
            }
            if spec.children == Children::Bag {
                ui.separator();
                if let Some(c) = bag(ui, cx, path, n) {
                    out = Some(c);
                }
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_values_are_written_as_kdl() {
        assert_eq!(repr_for(Ty::Number, "150", false), "150");
        assert_eq!(repr_for(Ty::Number, "wide", false), "\"wide\"");
        assert_eq!(repr_for(Ty::Template, r"C:\x", false), r##"#"C:\x"#"##);
        assert_eq!(
            repr_for(Ty::Template, "values.x", true),
            "(lua)\"values.x\""
        );
        assert_eq!(repr_for(Ty::LuaValue, "{}", false), "(lua)\"{}\"");
        assert_eq!(repr_for(Ty::Any, "#true", false), "#true");
    }
}

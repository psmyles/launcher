//! The editor's outline (M8): the file's nodes as a tree. Select, add (from the catalog: what the
//! selected node may hold, or what may stand beside it), duplicate, delete, and move with
//! Alt+Up/Down. A data bag's keys are edited in the properties table, not here.

use std::collections::BTreeSet;

use egui::{RichText, Ui};
use kdl::{KdlDocument, KdlNode};
use launcher_core::config::catalog::{self, Category, Children};
use launcher_core::config::NodePath;

pub enum OutlineEvent {
    Select(NodePath),
    /// Add a node of `kind` as child `index` of `parent` (`None`: the top level).
    Add {
        parent: Option<NodePath>,
        index: usize,
        kind: &'static str,
    },
    Duplicate(NodePath),
    Delete(NodePath),
    /// Move up (-1) or down (+1) among its siblings.
    Move(NodePath, i32),
}

/// Nodes the user folded.
#[derive(Default)]
pub struct OutlineState {
    pub folded: BTreeSet<NodePath>,
}

/// A node's one-line summary: `dropdown "role"  Role`.
fn summary(n: &KdlNode) -> (String, String) {
    let name = n.name().value().to_string();
    let first = n
        .entries()
        .iter()
        .find(|e| e.name().is_none())
        .map(|e| match e.value().as_string() {
            Some(s) => {
                let one: String = s.lines().next().unwrap_or("").chars().take(40).collect();
                format!(" \"{one}\"")
            }
            None => format!(" {}", e.value()),
        })
        .unwrap_or_default();
    let label = n
        .entries()
        .iter()
        .find(|e| {
            e.name()
                .is_some_and(|k| k.value() == "label" || k.value() == "title")
        })
        .and_then(|e| e.value().as_string())
        .unwrap_or("")
        .to_string();
    (format!("{name}{first}"), label)
}

/// What a node's block may hold (a bag's keys aren't outline items).
fn holds(n: &KdlNode) -> Option<Children> {
    catalog::get(n.name().value())
        .map(|s| s.children)
        .filter(|c| !matches!(c, Children::None | Children::Bag))
}

/// The kinds that may go in a block of `children` (the top level: `None`).
fn kinds_for(children: Option<Children>) -> Vec<&'static str> {
    match children {
        None => catalog::SPECS
            .iter()
            .filter(|s| s.cat == Category::Top)
            .map(|s| s.name)
            .collect(),
        Some(c) => catalog::allowed(c),
    }
}

fn add_menu(ui: &mut Ui, kinds: &[&'static str], out: &mut Option<&'static str>) {
    let groups = [
        (Category::Top, "File"),
        (Category::Layout, "Layout"),
        (Category::Control, "Controls"),
        (Category::Step, "Steps"),
        (Category::Part, "Parts"),
    ];
    for (cat, title) in groups {
        let here: Vec<&&str> = kinds
            .iter()
            .filter(|k| catalog::get(k).is_some_and(|s| s.cat == cat))
            .collect();
        if here.is_empty() {
            continue;
        }
        ui.label(RichText::new(title).small().weak());
        for k in here {
            let doc = catalog::get(k).map(|s| s.doc).unwrap_or("");
            if ui.button(*k).on_hover_text(doc).clicked() {
                *out = Some(k);
                ui.close();
            }
        }
    }
}

pub fn show(
    ui: &mut Ui,
    doc: Option<&KdlDocument>,
    selected: Option<&NodePath>,
    state: &mut OutlineState,
) -> Option<OutlineEvent> {
    let mut event = None;
    let Some(doc) = doc else {
        ui.label(RichText::new("The file isn't valid KDL; fix it in the Text tab.").weak());
        return None;
    };
    let sel_node = selected.and_then(|p| launcher_core::config::edit::node_at(doc, p));
    // Toolbar: Add (into the selection if it holds things, else beside it), and the rest.
    ui.horizontal(|ui| {
        let (parent, index, kinds) = match (selected, sel_node) {
            (Some(p), Some(n)) if holds(n).is_some() => {
                let count = n.children().map_or(0, |c| c.nodes().len());
                (Some(p.clone()), count, kinds_for(holds(n)))
            }
            (Some(p), Some(_)) => {
                let (last, parent) = p.split_last().expect("a path");
                let parent_node = (!parent.is_empty())
                    .then(|| launcher_core::config::edit::node_at(doc, parent))
                    .flatten();
                let kinds = kinds_for(parent_node.and_then(holds));
                (
                    (!parent.is_empty()).then(|| parent.to_vec()),
                    last + 1,
                    kinds,
                )
            }
            _ => (None, doc.nodes().len(), kinds_for(None)),
        };
        let mut picked = None;
        ui.menu_button("Add", |ui| {
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .show(ui, |ui| add_menu(ui, &kinds, &mut picked));
        });
        if let Some(kind) = picked {
            event = Some(OutlineEvent::Add {
                parent,
                index,
                kind,
            });
        }
        let has = selected.is_some();
        if ui
            .add_enabled(has, egui::Button::new("Duplicate"))
            .on_hover_text("Ctrl+D")
            .clicked()
        {
            event = selected.cloned().map(OutlineEvent::Duplicate);
        }
        if ui.add_enabled(has, egui::Button::new("Delete")).clicked() {
            event = selected.cloned().map(OutlineEvent::Delete);
        }
        if ui
            .add_enabled(has, egui::Button::new("↑"))
            .on_hover_text("Alt+Up")
            .clicked()
        {
            event = selected.cloned().map(|p| OutlineEvent::Move(p, -1));
        }
        if ui
            .add_enabled(has, egui::Button::new("↓"))
            .on_hover_text("Alt+Down")
            .clicked()
        {
            event = selected.cloned().map(|p| OutlineEvent::Move(p, 1));
        }
    });
    if let Some(p) = selected {
        let (up, down, dup) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::D),
            )
        });
        if up {
            event = Some(OutlineEvent::Move(p.clone(), -1));
        } else if down {
            event = Some(OutlineEvent::Move(p.clone(), 1));
        } else if dup {
            event = Some(OutlineEvent::Duplicate(p.clone()));
        }
    }
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([true, false])
        .show(ui, |ui| {
            let mut path = Vec::new();
            rows(ui, doc.nodes(), &mut path, selected, state, &mut event);
        });
    event
}

fn rows(
    ui: &mut Ui,
    nodes: &[KdlNode],
    path: &mut NodePath,
    selected: Option<&NodePath>,
    state: &mut OutlineState,
    event: &mut Option<OutlineEvent>,
) {
    for (i, n) in nodes.iter().enumerate() {
        path.push(i);
        let kids = holds(n).and_then(|_| n.children()).map(|c| c.nodes());
        let has_kids = kids.is_some_and(|k| !k.is_empty());
        let folded = state.folded.contains(path);
        ui.horizontal(|ui| {
            ui.add_space(14.0 * (path.len() - 1) as f32);
            if has_kids {
                let arrow = if folded { "+" } else { "-" };
                if ui.small_button(arrow).clicked() {
                    if folded {
                        state.folded.remove(path);
                    } else {
                        state.folded.insert(path.clone());
                    }
                }
            } else {
                ui.add_space(18.0);
            }
            let (head, label) = summary(n);
            let mut text = RichText::new(head).monospace();
            if catalog::get(n.name().value()).is_none() {
                text = text.color(ui.visuals().warn_fg_color);
            }
            let r = ui.selectable_label(selected == Some(path), text);
            if !label.is_empty() {
                ui.label(RichText::new(label).weak());
            }
            if r.clicked() {
                *event = Some(OutlineEvent::Select(path.clone()));
            }
            if selected == Some(path) && r.gained_focus() {
                r.scroll_to_me(None);
            }
        });
        if has_kids && !folded {
            if let Some(k) = kids {
                rows(ui, k, path, selected, state, event);
            }
        }
        path.pop();
    }
}

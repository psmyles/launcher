//! Editing a tool (format 2) the way the editor does (N24): changes to the parsed document, which
//! is then printed canonically (`config::print`). Each function is one thing a person does in the
//! editor; they never touch text, so there is nothing to splice and nothing to get out of step.

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

use crate::addons::{SettingDef, SettingKind, StepDef};
use crate::config::template::Template;

/// Where a step is: the button's `steps` block, its "Before starting" part or the main list, and
/// its place in that list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepAt {
    pub steps: String,
    pub before: bool,
    pub index: usize,
}

fn first_arg(n: &KdlNode) -> Option<&str> {
    n.entries()
        .iter()
        .find(|e| e.name().is_none())
        .and_then(|e| e.value().as_string())
}

fn prop<'a>(n: &'a KdlNode, name: &str) -> Option<&'a KdlValue> {
    n.entries()
        .iter()
        .rev()
        .find(|e| e.name().is_some_and(|k| k.value() == name))
        .map(KdlEntry::value)
}

/// The `steps "<id>"` block.
pub fn steps_node<'a>(doc: &'a KdlDocument, id: &str) -> Option<&'a KdlNode> {
    doc.nodes()
        .iter()
        .find(|n| n.name().value() == "steps" && first_arg(n) == Some(id))
}

fn steps_node_mut<'a>(doc: &'a mut KdlDocument, id: &str) -> Option<&'a mut KdlNode> {
    doc.nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "steps" && first_arg(n) == Some(id))
}

/// The steps of one part (main or "Before starting"), in order.
pub fn step_list<'a>(doc: &'a KdlDocument, steps: &str, before: bool) -> Vec<&'a KdlNode> {
    let Some(s) = steps_node(doc, steps) else {
        return Vec::new();
    };
    let kids = s.children().map(|d| d.nodes()).unwrap_or_default();
    if before {
        kids.iter()
            .find(|n| n.name().value() == "before-starting")
            .and_then(|b| b.children())
            .map(|d| d.nodes().iter().collect())
            .unwrap_or_default()
    } else {
        kids.iter()
            .filter(|n| n.name().value() != "before-starting")
            .collect()
    }
}

/// The document holding a part's steps (made when missing), and the physical index of each step.
fn part_mut<'a>(
    doc: &'a mut KdlDocument,
    steps: &str,
    before: bool,
) -> Option<(&'a mut KdlDocument, Vec<usize>)> {
    let s = steps_node_mut(doc, steps)?;
    let kids = s.ensure_children();
    if before {
        let i = match kids
            .nodes()
            .iter()
            .position(|n| n.name().value() == "before-starting")
        {
            Some(i) => i,
            None => {
                kids.nodes_mut().insert(0, KdlNode::new("before-starting"));
                0
            }
        };
        let b = kids.nodes_mut()[i].ensure_children();
        let idx = (0..b.nodes().len()).collect();
        Some((b, idx))
    } else {
        let idx = kids
            .nodes()
            .iter()
            .enumerate()
            .filter(|(_, n)| n.name().value() != "before-starting")
            .map(|(i, _)| i)
            .collect();
        Some((kids, idx))
    }
}

/// Every id in the tool: controls' and steps'. A new one must be new.
pub fn taken_ids(doc: &KdlDocument) -> Vec<String> {
    fn walk(d: &KdlDocument, out: &mut Vec<String>) {
        for n in d.nodes() {
            let name = n.name().value();
            let has_id = name.contains('.')
                || matches!(
                    name,
                    "choice"
                        | "choices"
                        | "tick-box"
                        | "number"
                        | "text"
                        | "file"
                        | "folder"
                        | "display"
                        | "button"
                        | "steps"
                        | "value"
                        | "data"
                );
            if has_id {
                if let Some(a) = first_arg(n) {
                    out.push(a.to_string());
                }
            }
            if name != "add-on" {
                if let Some(c) = n.children() {
                    walk(c, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(doc, &mut out);
    out
}

/// `base`, or `base-2`, `base-3`... whichever isn't taken.
pub fn unique_id(doc: &KdlDocument, base: &str) -> String {
    let taken = taken_ids(doc);
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken.contains(c))
        .expect("an unused id")
}

/// Add a step from the library at `at` (its index is where it goes). Returns its id.
pub fn insert_step(doc: &mut KdlDocument, at: &StepAt, def: &StepDef) -> Option<String> {
    let base = def.id.split_once('.').map_or(def.id.as_str(), |(_, s)| s);
    let id = unique_id(doc, base);
    let mut n = KdlNode::new(def.id.as_str());
    n.push(KdlEntry::new(id.clone()));
    let (part, idx) = part_mut(doc, &at.steps, at.before)?;
    let phys = idx.get(at.index).copied().unwrap_or(part.nodes().len());
    part.nodes_mut().insert(phys, n);
    Some(id)
}

/// Take a step out.
pub fn remove_step(doc: &mut KdlDocument, at: &StepAt) -> Option<KdlNode> {
    let (part, idx) = part_mut(doc, &at.steps, at.before)?;
    let phys = *idx.get(at.index)?;
    Some(part.nodes_mut().remove(phys))
}

/// Move a step to another place (in the same part or the other one).
pub fn move_step(doc: &mut KdlDocument, from: &StepAt, to: &StepAt) -> bool {
    let Some(node) = remove_step(doc, from) else {
        return false;
    };
    let Some((part, idx)) = part_mut(doc, &to.steps, to.before) else {
        return false;
    };
    let phys = idx.get(to.index).copied().unwrap_or(part.nodes().len());
    part.nodes_mut().insert(phys, node);
    true
}

/// The step at `at`.
pub fn step_node<'a>(doc: &'a KdlDocument, at: &StepAt) -> Option<&'a KdlNode> {
    step_list(doc, &at.steps, at.before).get(at.index).copied()
}

fn step_node_mut<'a>(doc: &'a mut KdlDocument, at: &StepAt) -> Option<&'a mut KdlNode> {
    let (part, idx) = part_mut(doc, &at.steps, at.before)?;
    let phys = *idx.get(at.index)?;
    part.nodes_mut().get_mut(phys)
}

/// Set a property of a node (a setting, `label`, `if-wrong`...), or take it away (None).
pub fn set_prop(n: &mut KdlNode, name: &str, value: Option<KdlValue>) {
    n.entries_mut()
        .retain(|e| !e.name().is_some_and(|k| k.value() == name));
    if let Some(v) = value {
        n.push(KdlEntry::new_prop(name, v));
    }
}

/// Set one of a step's properties (None takes it away).
pub fn set_step_prop(doc: &mut KdlDocument, at: &StepAt, name: &str, value: Option<KdlValue>) {
    if let Some(n) = step_node_mut(doc, at) {
        // A setting given as a child (a list) is replaced too.
        if let Some(kids) = n.children_mut() {
            kids.nodes_mut().retain(|c| c.name().value() != name);
        }
        if n.children().is_some_and(|c| c.nodes().is_empty()) {
            n.clear_children();
        }
        set_prop(n, name, value);
    }
}

/// Set a list setting (a program's arguments): a child node with one value per item.
pub fn set_step_list(doc: &mut KdlDocument, at: &StepAt, name: &str, items: &[String]) {
    set_step_prop(doc, at, name, None);
    if items.is_empty() {
        return;
    }
    if let Some(n) = step_node_mut(doc, at) {
        let mut c = KdlNode::new(name);
        for i in items {
            c.push(KdlEntry::new(i.as_str()));
        }
        n.ensure_children().nodes_mut().push(c);
    }
}

/// A copy of a step just after it, with a new id.
pub fn duplicate_step(doc: &mut KdlDocument, at: &StepAt) -> Option<String> {
    let mut copy = step_node(doc, at)?.clone();
    let base = first_arg(&copy).unwrap_or("step").to_string();
    let id = unique_id(doc, &base);
    if let Some(e) = copy.entries_mut().iter_mut().find(|e| e.name().is_none()) {
        *e = KdlEntry::new(id.clone());
    }
    let (part, idx) = part_mut(doc, &at.steps, at.before)?;
    let phys = idx.get(at.index + 1).copied().unwrap_or(part.nodes().len());
    part.nodes_mut().insert(phys, copy);
    Some(id)
}

/// A choice control's fixed options.
pub fn set_options(doc: &mut KdlDocument, id: &str, options: &[String]) {
    if let Some(n) = control_mut(doc, id) {
        let kids = n.ensure_children();
        let first = kids
            .nodes()
            .iter()
            .position(|c| c.name().value() == "option")
            .unwrap_or(kids.nodes().len());
        kids.nodes_mut().retain(|c| c.name().value() != "option");
        for (i, o) in options.iter().enumerate() {
            let mut on = KdlNode::new("option");
            on.push(KdlEntry::new(o.as_str()));
            let at = (first + i).min(kids.nodes().len());
            kids.nodes_mut().insert(at, on);
        }
        if kids.nodes().is_empty() {
            n.clear_children();
        }
    }
}

/// A step's value for a setting, as written (None: not set).
pub fn step_value(n: &KdlNode, name: &str) -> Option<KdlValue> {
    prop(n, name).cloned().or_else(|| {
        n.children()?
            .nodes()
            .iter()
            .find(|c| c.name().value() == name)
            .and_then(|c| c.entries().first())
            .map(|e| e.value().clone())
    })
}

/// The control "Use… → a new control" adds for a setting.
fn control_for(setting: &SettingDef) -> KdlNode {
    let word = match setting.kind {
        SettingKind::TickBox => "tick-box",
        SettingKind::Number => "number",
        SettingKind::File => "file",
        SettingKind::Folder => "folder",
        SettingKind::Choice => "choice",
        SettingKind::Choices => "choices",
        _ => "text",
    };
    let mut c = KdlNode::new(word);
    c.push(KdlEntry::new(setting.id.clone()));
    c.push(KdlEntry::new_prop("label", setting.label.clone()));
    if let Some(h) = &setting.help {
        c.push(KdlEntry::new_prop("help", h.clone()));
    }
    if word == "file" && !setting.extensions.is_empty() {
        c.push(KdlEntry::new_prop(
            "extensions",
            setting.extensions.join(", "),
        ));
    }
    if let Some(s) = &setting.starts {
        let v = match s {
            crate::dynval::Dyn::Bool(b) => KdlValue::Bool(*b),
            crate::dynval::Dyn::Num(n) => KdlValue::Float(*n),
            d => KdlValue::String(d.to_text()),
        };
        c.push(KdlEntry::new_prop("starts", v));
    }
    if setting.required {
        c.push(KdlEntry::new_prop("required", true));
    }
    let kids = c.ensure_children();
    for o in &setting.options {
        let mut on = KdlNode::new("option");
        on.push(KdlEntry::new(o.clone()));
        kids.nodes_mut().push(on);
    }
    // A choice from an app's list (Git's branches) starts empty: a step fills it (`put`).
    if kids.nodes().is_empty() {
        c.clear_children();
    }
    c
}

/// The `window` block (made when missing, after the `tool` line).
fn window_mut(doc: &mut KdlDocument) -> &mut KdlNode {
    let i = match doc
        .nodes()
        .iter()
        .position(|n| n.name().value() == "window")
    {
        Some(i) => i,
        None => {
            let at = doc
                .nodes()
                .iter()
                .position(|n| n.name().value() == "tool")
                .map_or(0, |i| i + 1);
            doc.nodes_mut().insert(at, KdlNode::new("window"));
            at
        }
    };
    &mut doc.nodes_mut()[i]
}

/// Add a control to the window, above the buttons. Returns where it went (its index).
pub fn insert_control(doc: &mut KdlDocument, control: KdlNode, at: Option<usize>) -> usize {
    let w = window_mut(doc).ensure_children();
    let first_button = w
        .nodes()
        .iter()
        .position(|n| n.name().value() == "button")
        .unwrap_or(w.nodes().len());
    let i = at.unwrap_or(first_button).min(w.nodes().len());
    w.nodes_mut().insert(i, control);
    i
}

/// A new control of a kind (from "+ Add a control"): its id.
pub fn add_control(doc: &mut KdlDocument, word: &str, label: &str) -> String {
    let base = label
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() {
        word.to_string()
    } else {
        base
    };
    let mut n = KdlNode::new(word);
    let layout = matches!(word, "info" | "line" | "section" | "pages" | "row");
    let id = if layout {
        String::new()
    } else {
        unique_id(doc, &base)
    };
    match word {
        "info" => n.push(KdlEntry::new(label)),
        "section" => n.push(KdlEntry::new(label)),
        "line" | "pages" | "row" => {}
        _ => {
            n.push(KdlEntry::new(id.clone()));
            n.push(KdlEntry::new_prop("label", label));
        }
    }
    if word == "choice" || word == "choices" {
        let kids = n.ensure_children();
        for o in ["First", "Second"] {
            let mut on = KdlNode::new("option");
            on.push(KdlEntry::new(o));
            kids.nodes_mut().push(on);
        }
    }
    if word == "button" {
        // A button runs its own steps: make an empty `steps` block for it.
        let mut s = KdlNode::new("steps");
        s.push(KdlEntry::new(id.clone()));
        doc.nodes_mut().push(s);
        let w = window_mut(doc).ensure_children();
        w.nodes_mut().push(n);
        return id;
    }
    insert_control(doc, n, None);
    id
}

/// The control with this id, anywhere in the window (in rows and sections too).
pub fn control_mut<'a>(doc: &'a mut KdlDocument, id: &str) -> Option<&'a mut KdlNode> {
    fn find<'a>(d: &'a mut KdlDocument, id: &str) -> Option<&'a mut KdlNode> {
        for n in d.nodes_mut() {
            if first_arg(n) == Some(id) && !matches!(n.name().value(), "section" | "info" | "page")
            {
                return Some(n);
            }
            if let Some(c) = n.children_mut() {
                if let Some(f) = find(c, id) {
                    return Some(f);
                }
            }
        }
        None
    }
    let w = doc
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "window")?;
    find(w.children_mut().as_mut()?, id)
}

/// The control with this id.
pub fn control<'a>(doc: &'a KdlDocument, id: &str) -> Option<&'a KdlNode> {
    fn find<'a>(d: &'a KdlDocument, id: &str) -> Option<&'a KdlNode> {
        for n in d.nodes() {
            if first_arg(n) == Some(id) && !matches!(n.name().value(), "section" | "info" | "page")
            {
                return Some(n);
            }
            if let Some(c) = n.children() {
                if let Some(f) = find(c, id) {
                    return Some(f);
                }
            }
        }
        None
    }
    let w = doc.nodes().iter().find(|n| n.name().value() == "window")?;
    find(w.children()?, id)
}

/// A new text, line or section (from "+ Add a control"): where it went.
pub fn add_layout(doc: &mut KdlDocument, word: &str, text: &str) -> Vec<usize> {
    let mut n = KdlNode::new(word);
    if word != "line" {
        n.push(KdlEntry::new(text));
    }
    vec![insert_control(doc, n, None)]
}

/// Take a control out of the window (and a button's steps with it).
pub fn remove_control(doc: &mut KdlDocument, id: &str) -> bool {
    match window_children(doc).and_then(|w| find_path(w, id)) {
        Some(p) => remove_item(doc, &p),
        None => false,
    }
}

/// Move a control up or down by one, among the items beside it.
pub fn move_control(doc: &mut KdlDocument, id: &str, delta: i32) -> bool {
    match window_children(doc).and_then(|w| find_path(w, id)) {
        Some(p) => move_item(doc, &p, delta),
        None => false,
    }
}

/// Where a dragged item goes, next to another (or into a section or page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Above,
    Below,
    Left,
    Right,
    Inside,
}

/// At most this many controls side by side.
pub const ROW_MAX: usize = 3;

/// Window items that hold others.
fn is_container(n: &KdlNode) -> bool {
    matches!(n.name().value(), "row" | "section" | "pages" | "page")
}

/// The indices (through rows, sections and pages) of the control with this id.
fn find_path(d: &KdlDocument, id: &str) -> Option<Vec<usize>> {
    for (i, n) in d.nodes().iter().enumerate() {
        if is_container(n) {
            if let Some(mut p) = n.children().and_then(|c| find_path(c, id)) {
                p.insert(0, i);
                return Some(p);
            }
        } else if first_arg(n) == Some(id) && !matches!(n.name().value(), "info" | "line") {
            return Some(vec![i]);
        }
    }
    None
}

fn node_at<'a>(d: &'a KdlDocument, path: &[usize]) -> &'a KdlNode {
    let n = &d.nodes()[path[0]];
    match path.len() {
        1 => n,
        _ => node_at(n.children().as_ref().expect("a container"), &path[1..]),
    }
}

/// The list that holds the item at `path` (path without its last index).
fn holder_mut<'a>(d: &'a mut KdlDocument, parent: &[usize]) -> &'a mut KdlDocument {
    match parent.split_first() {
        None => d,
        Some((i, rest)) => holder_mut(d.nodes_mut()[*i].ensure_children(), rest),
    }
}

/// A row left with one control becomes that control; an empty one goes.
fn tidy_rows(d: &mut KdlDocument) {
    let mut i = 0;
    while i < d.nodes().len() {
        if is_container(&d.nodes()[i]) {
            if let Some(c) = d.nodes_mut()[i].children_mut().as_mut() {
                tidy_rows(c);
            }
        }
        let n = &d.nodes()[i];
        if n.name().value() == "row" {
            let kids = n.children().map(|c| c.nodes().to_vec()).unwrap_or_default();
            match kids.len() {
                0 => {
                    d.nodes_mut().remove(i);
                    continue;
                }
                1 => d.nodes_mut()[i] = kids[0].clone(),
                _ => {}
            }
        }
        i += 1;
    }
}

/// The window's items, if it has any.
fn window_children(doc: &KdlDocument) -> Option<&KdlDocument> {
    doc.nodes()
        .iter()
        .find(|n| n.name().value() == "window")?
        .children()
}

/// The item at `path` (indices through rows, sections and pages), if there is one.
fn get_at<'a>(d: &'a KdlDocument, path: &[usize]) -> Option<&'a KdlNode> {
    let (first, rest) = path.split_first()?;
    let n = d.nodes().get(*first)?;
    if rest.is_empty() {
        Some(n)
    } else {
        get_at(n.children().as_ref()?, rest)
    }
}

/// A window item by where it is (for items without a name: text, lines, sections).
pub fn item<'a>(doc: &'a KdlDocument, path: &[usize]) -> Option<&'a KdlNode> {
    get_at(window_children(doc)?, path)
}

pub fn item_mut<'a>(doc: &'a mut KdlDocument, path: &[usize]) -> Option<&'a mut KdlNode> {
    fn at<'a>(d: &'a mut KdlDocument, path: &[usize]) -> Option<&'a mut KdlNode> {
        let (first, rest) = path.split_first()?;
        let n = d.nodes_mut().get_mut(*first)?;
        if rest.is_empty() {
            Some(n)
        } else {
            at(n.children_mut().as_mut()?, rest)
        }
    }
    let w = doc
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "window")?;
    at(w.children_mut().as_mut()?, path)
}

/// Where the window item written at byte `offset` of the file is (the editor matches what it
/// draws to the file this way).
pub fn item_at_offset(doc: &KdlDocument, offset: usize) -> Option<Vec<usize>> {
    fn find(d: &KdlDocument, offset: usize) -> Option<Vec<usize>> {
        for (i, n) in d.nodes().iter().enumerate() {
            if n.span().offset() == offset {
                return Some(vec![i]);
            }
            if is_container(n) {
                if let Some(mut p) = n.children().and_then(|c| find(c, offset)) {
                    p.insert(0, i);
                    return Some(p);
                }
            }
        }
        None
    }
    find(window_children(doc)?, offset)
}

/// Take an item out of the window. A button takes its steps with it; a section or page leaves
/// what it held where it was.
pub fn remove_item(doc: &mut KdlDocument, path: &[usize]) -> bool {
    let Some(n) = item(doc, path).cloned() else {
        return false;
    };
    let w = window_mut(doc).ensure_children();
    let (last, parent) = path.split_last().expect("a path");
    let list = holder_mut(w, parent);
    list.nodes_mut().remove(*last);
    if matches!(n.name().value(), "section" | "page") {
        let kids = n.children().map(|c| c.nodes().to_vec()).unwrap_or_default();
        for (k, c) in kids.into_iter().enumerate() {
            list.nodes_mut().insert(last + k, c);
        }
    }
    tidy_rows(w);
    if let Some(id) = first_arg(&n).filter(|_| !is_container(&n)) {
        drop_puts_into(doc, id);
    }
    if n.name().value() == "button" {
        if let Some(id) = first_arg(&n) {
            doc.nodes_mut()
                .retain(|s| !(s.name().value() == "steps" && first_arg(s) == Some(id)));
        }
    }
    true
}

/// Move an item up or down by one, among the items beside it.
pub fn move_item(doc: &mut KdlDocument, path: &[usize], delta: i32) -> bool {
    if item(doc, path).is_none() {
        return false;
    }
    let w = window_mut(doc).ensure_children();
    let (last, parent) = path.split_last().expect("a path");
    let list = holder_mut(w, parent);
    let j = *last as i64 + delta as i64;
    if j < 0 || j as usize >= list.nodes().len() {
        return false;
    }
    let n = list.nodes_mut().remove(*last);
    list.nodes_mut().insert(j as usize, n);
    true
}

/// Drag and drop in the window: put control `id` above, below or beside `target`.
pub fn place_control(doc: &mut KdlDocument, id: &str, target: &str, side: Side) -> bool {
    let Some(w) = window_children(doc) else {
        return false;
    };
    match (find_path(w, id), find_path(w, target)) {
        (Some(a), Some(b)) => place_item(doc, &a, &b, side),
        _ => false,
    }
}

/// Drag and drop in the window: put the item at `from` above, below, beside or (a section or
/// page) inside the one at `to`. Beside makes a row (or joins the target's row, up to
/// [`ROW_MAX`]); above or below a control in a row goes above or below the whole row. Only
/// controls go side by side. False when nothing changed.
pub fn place_item(doc: &mut KdlDocument, from: &[usize], to: &[usize], side: Side) -> bool {
    if from.is_empty() || to.starts_with(from) {
        return false;
    }
    let (Some(a), Some(b)) = (item(doc, from), item(doc, to)) else {
        return false;
    };
    let beside = matches!(side, Side::Left | Side::Right);
    if beside && (is_container(a) || is_container(b)) {
        return false;
    }
    if side == Side::Inside && !matches!(b.name().value(), "section" | "page") {
        return false;
    }
    let w = window_mut(doc).ensure_children();
    let in_row = |d: &KdlDocument, p: &[usize]| {
        p.len() >= 2 && node_at(d, &p[..p.len() - 1]).name().value() == "row"
    };
    if beside && in_row(w, to) {
        let row = &to[..to.len() - 1];
        let full = node_at(w, row).children().map_or(0, |c| c.nodes().len()) >= ROW_MAX;
        if full && from[..from.len() - 1] != *row {
            return false;
        }
    }
    let (last, parent) = from.split_last().expect("a path");
    let node = holder_mut(w, parent).nodes_mut().remove(*last);
    // Where the target is now that the item is out.
    let mut to = to.to_vec();
    let d = parent.len();
    if to.len() > d && to[..d] == *parent && to[d] > *last {
        to[d] -= 1;
    }
    let after = matches!(side, Side::Below | Side::Right);
    if side == Side::Inside {
        holder_mut(w, &to).nodes_mut().push(node);
    } else if beside && !in_row(w, &to) {
        let (last, parent) = to.split_last().expect("a path");
        let list = holder_mut(w, parent);
        let other = list.nodes()[*last].clone();
        let mut row = KdlNode::new("row");
        let kids = row.ensure_children();
        let pair = if after { [other, node] } else { [node, other] };
        kids.nodes_mut().extend(pair);
        list.nodes_mut()[*last] = row;
    } else {
        // Above or below a control in a row goes above or below the row.
        let anchor = if !beside && in_row(w, &to) {
            &to[..to.len() - 1]
        } else {
            &to[..]
        };
        let (last, parent) = anchor.split_last().expect("a path");
        holder_mut(w, parent)
            .nodes_mut()
            .insert(last + after as usize, node);
    }
    tidy_rows(w);
    true
}

/// "Use… → a new control" for a setting: a control for it in the window (one with the setting's
/// name that's already there is reused), and the setting reading it. Returns the control's id.
pub fn use_new_control(
    doc: &mut KdlDocument,
    at: &StepAt,
    setting: &str,
    def: &StepDef,
) -> Option<String> {
    let s = def.setting(setting)?.clone();
    step_node(doc, at)?;
    // A control with that name is there already, and the setting can take it: use it.
    let fits = control(doc, &s.id).is_some_and(|c| {
        s.takes(
            c.name().value(),
            &control_extensions(doc, &s.id),
            &control_options(doc, &s.id),
        )
    });
    let id = if fits {
        s.id.clone()
    } else {
        let id = unique_id(doc, &s.id);
        let mut c = control_for(&s);
        if id != s.id {
            c.entries_mut()[0] = KdlEntry::new(id.clone());
        }
        insert_control(doc, c, None);
        id
    };
    set_step_prop(
        doc,
        at,
        setting,
        Some(KdlValue::String(format!("{{{id}}}"))),
    );
    Some(id)
}

/// The extensions a file control takes (`extensions="json, fbx"`); empty for any file, or for
/// another kind of control.
pub fn control_extensions(doc: &KdlDocument, id: &str) -> Vec<String> {
    control(doc, id)
        .and_then(|n| prop(n, "extensions"))
        .and_then(|v| v.as_string())
        .map(crate::paths::extensions)
        .unwrap_or_default()
}

/// The window's controls that hold a value, in order: (id, kind word, label).
pub fn value_controls(doc: &KdlDocument) -> Vec<(String, String, String)> {
    fn walk(d: &KdlDocument, out: &mut Vec<(String, String, String)>) {
        for n in d.nodes() {
            let word = n.name().value();
            if is_container(n) {
                if let Some(c) = n.children() {
                    walk(c, out);
                }
            } else if matches!(
                word,
                "choice" | "choices" | "tick-box" | "text" | "number" | "file" | "folder"
            ) {
                if let Some(id) = first_arg(n) {
                    let label = prop(n, "label")
                        .and_then(|v| v.as_string())
                        .unwrap_or(id)
                        .to_string();
                    out.push((id.to_string(), word.to_string(), label));
                }
            }
        }
    }
    let mut out = Vec::new();
    if let Some(w) = window_children(doc) {
        walk(w, &mut out);
    }
    out
}

/// Where a step puts its results: (result, control), from its `put "result" into="control"`.
pub fn puts(step: &KdlNode) -> Vec<(String, String)> {
    step.children()
        .map(|c| {
            c.nodes()
                .iter()
                .filter(|n| n.name().value() == "put")
                .filter_map(|n| {
                    let r = first_arg(n)?.to_string();
                    let into = prop(n, "into")?.as_string()?.to_string();
                    Some((r, into))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Put a step's result into a control (None: nowhere).
pub fn set_put(doc: &mut KdlDocument, at: &StepAt, result: &str, control: Option<&str>) {
    let Some(n) = step_node_mut(doc, at) else {
        return;
    };
    let kids = n.ensure_children();
    kids.nodes_mut()
        .retain(|p| !(p.name().value() == "put" && first_arg(p) == Some(result)));
    if let Some(c) = control {
        let mut p = KdlNode::new("put");
        p.push(KdlEntry::new(result));
        p.push(KdlEntry::new_prop("into", c));
        kids.nodes_mut().push(p);
    }
    if kids.nodes().is_empty() {
        n.clear_children();
    }
}

/// The steps that put something into a control.
pub fn filled_by(doc: &KdlDocument, control: &str) -> Vec<StepAt> {
    let mut out = Vec::new();
    for s in doc.nodes().iter().filter(|n| n.name().value() == "steps") {
        let Some(id) = first_arg(s) else { continue };
        for before in [true, false] {
            for (index, n) in step_list(doc, id, before).into_iter().enumerate() {
                if puts(n).iter().any(|(_, c)| c == control) {
                    out.push(StepAt {
                        steps: id.to_string(),
                        before,
                        index,
                    });
                }
            }
        }
    }
    out
}

/// Take out every `put` into a control (it's gone).
fn drop_puts_into(doc: &mut KdlDocument, control: &str) {
    fn walk(d: &mut KdlDocument, control: &str) {
        for n in d.nodes_mut() {
            if let Some(kids) = n.children_mut().as_mut() {
                kids.nodes_mut().retain(|p| {
                    !(p.name().value() == "put"
                        && prop(p, "into").and_then(|v| v.as_string()) == Some(control))
                });
                walk(kids, control);
            }
        }
    }
    for s in doc
        .nodes_mut()
        .iter_mut()
        .filter(|n| n.name().value() == "steps")
    {
        if let Some(kids) = s.children_mut().as_mut() {
            walk(kids, control);
        }
    }
}

/// The options a drop-down or tick boxes control has written in it (none when a step fills
/// them).
pub fn control_options(doc: &KdlDocument, id: &str) -> Vec<String> {
    control(doc, id)
        .and_then(|n| n.children())
        .map(|c| {
            c.nodes()
                .iter()
                .filter(|o| o.name().value() == "option")
                .filter_map(|o| first_arg(o).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a value is just `{name}` or `{name.field}`.
fn is_use_of(value: &str, name: &str) -> bool {
    value
        .strip_prefix('{')
        .and_then(|t| t.strip_suffix('}'))
        .filter(|t| !t.contains(['{', '}']))
        .is_some_and(|t| t == name || t.strip_prefix(name).is_some_and(|r| r.starts_with('.')))
}

/// A step's name for a message: its label, or its id.
fn step_name(n: &KdlNode) -> String {
    prop(n, "label")
        .and_then(|v| v.as_string())
        .or_else(|| first_arg(n))
        .unwrap_or_else(|| n.name().value())
        .to_string()
}

/// A control or step is gone: every step setting that was just it (`{name}`, `{name.result}`)
/// is cleared, back to its starting value, and every `put` into it goes. Returns the steps that
/// changed, by name.
pub fn forget_uses(doc: &mut KdlDocument, name: &str) -> Vec<String> {
    fn steps_in(d: &mut KdlDocument, name: &str, changed: &mut Vec<String>) {
        for n in d.nodes_mut() {
            if n.name().value() == "before-starting" {
                if let Some(kids) = n.children_mut().as_mut() {
                    steps_in(kids, name, changed);
                }
                continue;
            }
            let before = n.entries().len();
            n.entries_mut().retain(|e| {
                e.name().is_none() || !e.value().as_string().is_some_and(|v| is_use_of(v, name))
            });
            let mut gone = n.entries().len() != before;
            if let Some(kids) = n.children_mut().as_mut() {
                let had = kids.nodes().len();
                kids.nodes_mut().retain(|p| {
                    !(p.name().value() == "put"
                        && prop(p, "into").and_then(|v| v.as_string()) == Some(name))
                });
                gone |= kids.nodes().len() != had;
            }
            if n.children().is_some_and(|c| c.nodes().is_empty()) {
                n.clear_children();
            }
            if gone {
                changed.push(step_name(n));
            }
        }
    }
    let mut changed = Vec::new();
    for s in doc
        .nodes_mut()
        .iter_mut()
        .filter(|n| n.name().value() == "steps")
    {
        if let Some(kids) = s.children_mut().as_mut() {
            steps_in(kids, name, &mut changed);
        }
    }
    changed
}

/// Results a block's steps use before the step that finds them has run: (the step using it,
/// the step it uses), by id. "Before starting" runs first, then the rest in order.
pub fn misplaced_uses(doc: &KdlDocument, steps: &str) -> Vec<(String, String)> {
    let order: Vec<&KdlNode> = step_list(doc, steps, true)
        .into_iter()
        .chain(step_list(doc, steps, false))
        .collect();
    let ids: Vec<&str> = order.iter().filter_map(|n| first_arg(n)).collect();
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for n in &order {
        let own = first_arg(n).unwrap_or_default();
        // Its settings, and those given as children (a list), not its `put`s or its `if` code.
        let values = n
            .entries()
            .iter()
            .filter(|e| e.name().is_some_and(|k| k.value() != "if"))
            .chain(
                n.children()
                    .into_iter()
                    .flat_map(|c| c.nodes())
                    .filter(|c| c.name().value() != "put")
                    .flat_map(|c| c.entries()),
            )
            .filter_map(|e| e.value().as_string());
        for v in values {
            let Ok(t) = Template::parse(v) else { continue };
            for (name, _, _) in t.refs() {
                if ids.contains(&name) && name != own && !seen.contains(&name) {
                    let pair = (own.to_string(), name.to_string());
                    if !out.contains(&pair) {
                        out.push(pair);
                    }
                }
            }
        }
        seen.push(own);
    }
    out
}

/// Whether moving a step would have one use a result before it's found (when it didn't before).
pub fn move_misplaces(doc: &KdlDocument, from: &StepAt, to: &StepAt) -> bool {
    let now = misplaced_uses(doc, &from.steps);
    let mut moved = doc.clone();
    if !move_step(&mut moved, from, to) {
        return true;
    }
    misplaced_uses(&moved, &to.steps)
        .iter()
        .any(|m| !now.contains(m))
}

/// The control a setting reads, when its value is just that control (`branch="{branch}"`).
pub fn asked_by(step: &KdlNode, setting: &str) -> Option<String> {
    let v = step_value(step, setting)?;
    let s = v.as_string()?;
    let inner = s.strip_prefix('{')?.strip_suffix('}')?;
    (!inner.contains(['{', '}', '.'])).then(|| inner.to_string())
}

/// Stop asking: the setting goes back to its starting value, and the control goes too when
/// nothing else uses it.
pub fn stop_asking(doc: &mut KdlDocument, at: &StepAt, setting: &str) {
    let Some(id) = step_node(doc, at).and_then(|n| asked_by(n, setting)) else {
        return;
    };
    set_step_prop(doc, at, setting, None);
    if !uses_name(doc, &id) {
        remove_control(doc, &id);
    }
}

/// Whether anything outside the window says `{name}` (a step's setting, a title...).
pub fn uses_name(doc: &KdlDocument, name: &str) -> bool {
    let needle = format!("{{{name}");
    fn walk(d: &KdlDocument, needle: &str) -> bool {
        d.nodes().iter().any(|n| {
            n.entries()
                .iter()
                .any(|e| e.value().as_string().is_some_and(|s| s.contains(needle)))
                || n.children().is_some_and(|c| walk(c, needle))
        })
    }
    doc.nodes()
        .iter()
        .filter(|n| n.name().value() != "window")
        .any(|n| {
            n.entries()
                .iter()
                .any(|e| e.value().as_string().is_some_and(|s| s.contains(&needle)))
                || n.children().is_some_and(|c| walk(c, &needle))
        })
}

/// The buttons, in window order: (id, label).
pub fn buttons(doc: &KdlDocument) -> Vec<(String, String)> {
    fn walk(d: &KdlDocument, out: &mut Vec<(String, String)>) {
        for n in d.nodes() {
            if n.name().value() == "button" {
                if let Some(id) = first_arg(n) {
                    let label = prop(n, "label")
                        .and_then(|v| v.as_string())
                        .unwrap_or(id)
                        .to_string();
                    out.push((id.to_string(), label));
                }
            }
            if let Some(c) = n.children() {
                walk(c, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(w) = doc.nodes().iter().find(|n| n.name().value() == "window") {
        if let Some(c) = w.children() {
            walk(c, &mut out);
        }
    }
    out
}

/// Whether the tool saves what each run printed to files (`save-logs` on its first line).
pub fn saves_logs(doc: &KdlDocument) -> bool {
    doc.nodes()
        .first()
        .filter(|n| n.name().value() == "tool")
        .and_then(|n| n.get("save-logs"))
        .and_then(KdlValue::as_bool)
        .unwrap_or(false)
}

/// Turn saving each run's output on or off (off is the default, so it's left out).
pub fn set_save_logs(doc: &mut KdlDocument, on: bool) {
    if let Some(n) = doc
        .nodes_mut()
        .first_mut()
        .filter(|n| n.name().value() == "tool")
    {
        set_prop(n, "save-logs", on.then_some(KdlValue::Bool(true)));
    }
}

/// The text of a new, empty tool.
pub fn new_tool_text(name: &str, os: Option<&str>) -> String {
    let os = os.map(|o| format!(" os=\"{o}\"")).unwrap_or_default();
    format!(
        "tool {} format=2 requires=\"0.2\"{os}\n\nwindow {{\n    button \"start\" label=\"Start\"\n}}\n\nsteps \"start\" {{\n}}\n",
        kdl_string(name)
    )
}

fn kdl_string(s: &str) -> String {
    KdlValue::String(s.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addons::Library;
    use crate::config::print::print;

    fn doc(s: &str) -> KdlDocument {
        KdlDocument::parse_v2(s).unwrap()
    }

    #[test]
    fn saving_logs_is_off_until_turned_on() {
        let mut d = doc(&new_tool_text("Logs", None));
        let before = print(&d);
        assert!(!saves_logs(&d));
        set_save_logs(&mut d, true);
        assert!(saves_logs(&d));
        assert!(print(&d).starts_with("tool \"Logs\" format=2 requires=\"0.2\" save-logs=#true\n"));
        set_save_logs(&mut d, false);
        assert!(!saves_logs(&d));
        assert_eq!(print(&d), before);
    }

    fn main(i: usize) -> StepAt {
        StepAt {
            steps: "start".into(),
            before: false,
            index: i,
        }
    }

    #[test]
    fn a_session_builds_a_tool() {
        let lib = Library::builtin();
        let mut d = doc(&new_tool_text("Update", None));
        // Add "Switch branch", ask for the branch each time, add "Pull" before it, then move it
        // after, and take "Pull" out again.
        let switch = lib.step("git.switch-branch").unwrap();
        assert_eq!(
            insert_step(&mut d, &main(0), &switch).as_deref(),
            Some("switch-branch")
        );
        assert_eq!(
            use_new_control(&mut d, &main(0), "branch", &switch).as_deref(),
            Some("branch")
        );
        let pull = lib.step("git.pull").unwrap();
        insert_step(&mut d, &main(0), &pull);
        assert!(move_step(&mut d, &main(0), &main(1)));
        let before = StepAt {
            steps: "start".into(),
            before: true,
            index: 0,
        };
        let check = lib.step("git.no-unsaved-changes").unwrap();
        insert_step(&mut d, &before, &check);
        assert_eq!(
            print(&d),
            r#"tool "Update" format=2 requires="0.2"

window {
    choice "branch" label="Branch" help="The branch to switch to." required=#true
    button "start" label="Start"
}

steps "start" {
    before-starting {
        git.no-unsaved-changes "no-unsaved-changes"
    }
    git.switch-branch "switch-branch" branch="{branch}"
    git.pull "pull"
}
"#
        );
        // The tool loads without problems.
        let l = crate::config::load_str(
            &print(&d),
            std::path::Path::new("t.launcher"),
            &crate::config::LoadOptions::default(),
        );
        assert!(!l.has_errors(), "{:?}", l.diags);
        // Stop asking: the control goes, as nothing else uses it.
        stop_asking(&mut d, &main(0), "branch");
        assert!(control(&d, "branch").is_none());
        assert!(remove_step(&mut d, &main(1)).is_some());
        assert_eq!(step_list(&d, "start", false).len(), 1);
    }

    #[test]
    fn ids_stay_unique() {
        let lib = Library::builtin();
        let mut d = doc(&new_tool_text("T", None));
        let w = lib.step("core.wait").unwrap();
        insert_step(&mut d, &main(0), &w);
        assert_eq!(insert_step(&mut d, &main(1), &w).as_deref(), Some("wait-2"));
        assert_eq!(add_control(&mut d, "text", "Wait"), "wait-3");
    }

    #[test]
    fn a_new_button_gets_its_steps_and_taking_it_out_takes_them() {
        let mut d = doc(&new_tool_text("T", None));
        let id = add_control(&mut d, "button", "Clean up");
        assert_eq!(id, "clean-up");
        assert!(steps_node(&d, "clean-up").is_some());
        assert_eq!(buttons(&d).len(), 2);
        assert!(remove_control(&mut d, "clean-up"));
        assert!(steps_node(&d, "clean-up").is_none());
    }

    #[test]
    fn controls_are_placed_side_by_side_and_back() {
        let mut d = doc(r#"tool "T" format=2
window {
    text "a" label="A"
    text "b" label="B"
    text "c" label="C"
    text "e" label="E"
    button "go" label="Go"
}
steps "go" {
}
"#);
        let window = |d: &KdlDocument| {
            let p = print(d);
            let start = p.find("window {").unwrap();
            let end = start + p[start..].find("\n}").unwrap();
            p[start..end + 2].to_string()
        };
        // B beside A: a row; then C to the right of B; a fourth can't join.
        assert!(place_control(&mut d, "b", "a", Side::Right));
        assert!(place_control(&mut d, "c", "b", Side::Right));
        assert!(!place_control(&mut d, "e", "a", Side::Left));
        assert_eq!(
            window(&d),
            r#"window {
    row {
        text "a" label="A"
        text "b" label="B"
        text "c" label="C"
    }
    text "e" label="E"
    button "go" label="Go"
}"#
        );
        // Moving within the row is fine; below a control in a row goes below the row.
        assert!(place_control(&mut d, "a", "c", Side::Right));
        assert!(place_control(&mut d, "go", "b", Side::Below));
        // Taking two out leaves one: the row goes.
        assert!(place_control(&mut d, "b", "e", Side::Above));
        assert!(place_control(&mut d, "c", "e", Side::Below));
        assert_eq!(
            window(&d),
            r#"window {
    text "a" label="A"
    button "go" label="Go"
    text "b" label="B"
    text "e" label="E"
    text "c" label="C"
}"#
        );
        // A button in a row still takes its steps with it.
        assert!(place_control(&mut d, "go", "a", Side::Right));
        assert!(remove_control(&mut d, "go"));
        assert!(steps_node(&d, "go").is_none());
        assert!(!window(&d).contains("row"));
    }

    #[test]
    fn text_lines_and_sections_are_found_moved_and_removed() {
        let src = r#"tool "T" format=2

window {
    info "Read me"
    section "More" {
        text "a" label="A"
    }
    line
    button "go" label="Go"
}

steps "go" {
}
"#;
        let mut d = doc(src);
        // The editor finds an item by where it's written.
        let at = src.find("info").unwrap();
        assert_eq!(item_at_offset(&d, at), Some(vec![0]));
        assert_eq!(
            item_at_offset(&d, src.find("text \"a\"").unwrap()),
            Some(vec![1, 0])
        );
        // A section doesn't go side by side with anything.
        assert!(!place_item(&mut d, &[1], &[0], Side::Right));
        // Only a section or page takes things inside it.
        assert!(!place_item(&mut d, &[0], &[2], Side::Inside));
        assert!(place_item(&mut d, &[0], &[1], Side::Inside));
        assert_eq!(item(&d, &[0, 1]).unwrap().name().value(), "info");
        // The line moves up; the section goes but keeps what it held.
        assert!(move_item(&mut d, &[1], -1));
        assert_eq!(item(&d, &[0]).unwrap().name().value(), "line");
        assert!(remove_item(&mut d, &[1]));
        let names: Vec<&str> = window_children(&d)
            .unwrap()
            .nodes()
            .iter()
            .map(|n| n.name().value())
            .collect();
        assert_eq!(names, ["line", "text", "info", "button"]);
        // Removing the button takes its steps.
        assert!(remove_item(&mut d, &[3]));
        assert!(steps_node(&d, "go").is_none());
    }

    #[test]
    fn a_step_puts_its_result_into_a_control() {
        let lib = Library::builtin();
        let mut d = doc(&new_tool_text("T", None));
        let list = lib.step("git.list-branches").unwrap();
        insert_step(&mut d, &main(0), &list);
        let id = add_control(&mut d, "choice", "Select branch");
        set_put(&mut d, &main(0), "branches", Some(&id));
        let n = step_node(&d, &main(0)).unwrap();
        assert_eq!(puts(n), [("branches".to_string(), id.clone())]);
        assert!(print(&d).contains(r#"put "branches" into="select-branch""#));
        assert_eq!(filled_by(&d, &id).len(), 1);
        // The tool loads: the put is checked and translated.
        let l = crate::config::load_str(
            &print(&d),
            std::path::Path::new("t.launcher"),
            &crate::config::LoadOptions::default(),
        );
        assert!(!l.has_errors(), "{:?}", l.diags);
        // Nowhere again; and a control that goes takes the puts into it with it.
        set_put(&mut d, &main(0), "branches", None);
        assert!(puts(step_node(&d, &main(0)).unwrap()).is_empty());
        set_put(&mut d, &main(0), "branches", Some(&id));
        assert!(remove_control(&mut d, &id));
        assert!(puts(step_node(&d, &main(0)).unwrap()).is_empty());
    }

    const LODS: &str = r#"tool "Make LODs" format=2 requires="0.2"
window {
    file "rules" label="Audit preset" extensions="json"
    file "preset" label="Opt preset" extensions="json"
    folder "models" label="Models"
    folder "out" label="Save to"
    button "start" label="Start"
}
steps "start" {
    review3d.audit "audit" profile="{rules}" models="{models}"
    review3d.run-preset "optimize" preset="{preset}" models="{audit.to-fix}" save-to="{out}"
    files.show-in-folder "show" path="{out}"
}
"#;

    #[test]
    fn a_step_never_moves_above_what_it_uses() {
        let d = doc(LODS);
        assert!(misplaced_uses(&d, "start").is_empty());
        // The optimize step above the audit it uses: no. Show (uses only a control): anywhere.
        assert!(move_misplaces(&d, &main(1), &main(0)));
        assert!(move_misplaces(&d, &main(0), &main(1)));
        assert!(!move_misplaces(&d, &main(2), &main(0)));
        let mut moved = d.clone();
        move_step(&mut moved, &main(1), &main(0));
        assert_eq!(
            misplaced_uses(&moved, "start"),
            [("optimize".to_string(), "audit".to_string())]
        );
    }

    #[test]
    fn what_is_gone_is_no_longer_used() {
        // The audit goes: the setting that was just its result is cleared, nothing else.
        let mut d = doc(LODS);
        remove_step(&mut d, &main(0));
        assert_eq!(forget_uses(&mut d, "audit"), ["optimize"]);
        let opt = step_node(&d, &main(0)).unwrap();
        assert!(step_value(opt, "models").is_none());
        assert!(step_value(opt, "preset").is_some());
        // A control goes: the settings that read it, and what was put into it.
        let mut d = doc(&LODS.replace(
            "review3d.audit \"audit\" profile=\"{rules}\" models=\"{models}\"",
            "review3d.audit \"audit\" profile=\"{rules}\" models=\"{models}\" {\n        put \"to-fix-count\" into=\"rules\"\n    }",
        ));
        assert!(puts(step_node(&d, &main(0)).unwrap()).len() == 1);
        remove_control(&mut d, "rules");
        assert_eq!(forget_uses(&mut d, "rules"), ["audit"]);
        let audit = step_node(&d, &main(0)).unwrap();
        assert!(step_value(audit, "profile").is_none());
        assert!(puts(audit).is_empty());
    }

    #[test]
    fn a_new_control_for_a_setting_is_one_it_can_take() {
        let lib = Library::builtin();
        let def = lib.step("review3d.run-preset").unwrap();
        // A "preset" control is there, but for .fbx files: a new one is made for the setting.
        let mut d = doc(&LODS.replace(
            r#"file "preset" label="Opt preset" extensions="json""#,
            r#"file "preset" label="Opt preset" extensions="fbx""#,
        ));
        let id = use_new_control(&mut d, &main(1), "preset", &def).unwrap();
        assert_ne!(id, "preset");
        assert_eq!(control_extensions(&d, &id), ["json"]);
        // One that fits is used as it is.
        let mut d = doc(LODS);
        assert_eq!(
            use_new_control(&mut d, &main(1), "preset", &def).as_deref(),
            Some("preset")
        );
    }
}

//! Load-time evaluation (M3): a file's `script` blocks, computed values, computed option lists,
//! `(lua)` defaults and conditions. It runs on the evaluation thread ([`EvalThread`]); Lua never
//! runs on the UI thread (N6), and the UI shows the last results.
//!
//! Nothing here runs without a [`Trusted`] token (N9, `platform::trust`).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::config::template::{Part, Template};
use crate::config::*;
use crate::dynval::Dyn;
use crate::engine::effects::{CommandSpec, Context, Effects};
use crate::lua::{Host, LuaError, Snapshot, Vm, LOAD_BUDGET};
use crate::platform::trust::Trusted;

/// One entry of a computed option list.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionItem {
    pub name: String,
    pub value: String,
    pub label: Option<String>,
    pub fields: BTreeMap<String, Dyn>,
}

/// What loading a file worked out.
#[derive(Clone, Debug, Default)]
pub struct LoadResults {
    /// `value` definitions, by id.
    pub values: BTreeMap<String, Result<Dyn, String>>,
    /// Computed option lists (`options=(lua)`), by control id.
    pub options: BTreeMap<String, Result<Vec<OptionItem>, String>>,
    /// `(lua)` defaults, by control id.
    pub defaults: BTreeMap<String, Result<Dyn, String>>,
    /// `script` blocks that failed.
    pub script_errors: Vec<LuaError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Which {
    Visible,
    Enabled,
}

/// Every condition's result, by (node path, which).
pub type Conditions = BTreeMap<(NodePath, Which), Result<bool, String>>;

/// Every `(lua)` text of the form (titles, notes, tooltips, display texts, presets, option
/// labels), by (node path, property). A computed option's label is `option-label:<index>` on its
/// control.
pub type Texts = BTreeMap<(NodePath, String), Result<String, String>>;

/// What the form shows that depends on its values and needs Lua: worked out on the evaluation
/// thread whenever the values change.
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub conditions: Conditions,
    pub texts: Texts,
}

/// A change an on-change block makes to the form.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Set(String, Dyn),
    Reset(String),
}

/// What a control's on-change did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub edits: Vec<Edit>,
    pub logs: Vec<String>,
    pub errors: Vec<String>,
}

/// The names an option label sees: the option's fields, `name`, `value`, `label` and `index`.
pub fn option_scope(o: &OptionItem, index: usize) -> Snapshot {
    let mut s = Snapshot::default();
    for (k, v) in &o.fields {
        s.values.insert(k.clone(), v.clone());
    }
    s.values.insert("name".into(), Dyn::str(o.name.clone()));
    s.values.insert("value".into(), Dyn::str(o.value.clone()));
    s.values.insert(
        "label".into(),
        Dyn::str(o.label.clone().unwrap_or_else(|| o.name.clone())),
    );
    s.values.insert("index".into(), Dyn::Num(index as f64));
    s
}

/// What `{name}` or `{name.field}` names: a value, or a field (a step's result, an option's
/// data; the field's case doesn't matter).
fn referenced<'a>(snap: &'a Snapshot, name: &str, field: Option<&str>) -> Option<&'a Dyn> {
    match field {
        None => snap.values.get(name),
        Some(f) => snap.fields.get(name).and_then(|m| {
            m.get(f).or_else(|| {
                m.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(f))
                    .map(|(_, v)| v)
            })
        }),
    }
}

/// The value a template names when it is just one `{name}` or `{name.field}`, as it is (a list
/// stays a list); None for anything else.
pub fn whole_value(t: &Template, snap: &Snapshot) -> Option<Dyn> {
    match t.parts.as_slice() {
        [Part::Ref { name, field, .. }] => referenced(snap, name, field.as_deref()).cloned(),
        _ => None,
    }
}

/// Render a template against a snapshot and an environment.
pub fn render(t: &Template, snap: &Snapshot, env: &BTreeMap<String, String>) -> String {
    render_with(t, snap, env, Dyn::to_text)
}

/// Render a template for text that has lines (a file's content): a list goes in one item per
/// line.
pub fn render_lines(t: &Template, snap: &Snapshot, env: &BTreeMap<String, String>) -> String {
    render_with(t, snap, env, lines_of)
}

/// A value as lines: a list one item per line, anything else as text.
pub fn lines_of(v: &Dyn) -> String {
    match v {
        Dyn::List(l) => l.iter().map(Dyn::to_text).collect::<Vec<_>>().join("\n"),
        _ => v.to_text(),
    }
}

fn render_with(
    t: &Template,
    snap: &Snapshot,
    env: &BTreeMap<String, String>,
    text: fn(&Dyn) -> String,
) -> String {
    let mut out = String::new();
    for p in &t.parts {
        match p {
            Part::Lit(s) => out.push_str(s),
            Part::Ref { name, field, .. } => {
                let v = referenced(snap, name, field.as_deref()).map(text);
                out.push_str(&v.unwrap_or_default());
            }
            Part::Env { name, .. } => {
                let v = env.get(name).cloned().or_else(|| {
                    cfg!(windows)
                        .then(|| {
                            env.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                                .map(|(_, v)| v.clone())
                        })
                        .flatten()
                });
                out.push_str(&v.unwrap_or_default());
            }
        }
    }
    out
}

/// `find-file`: the first match of the pattern (skipping the launcher's own executable, and
/// files without the required sibling), relative to the file's folder when inside it; "" when
/// nothing matches. `text` renders the folder and sibling templates.
pub fn find_file(
    effects: &dyn Effects,
    cx: &Context,
    config_dir: &Path,
    f: &FindFile,
    snap: &Snapshot,
    text: &mut dyn FnMut(&Val, &Snapshot) -> Result<String, String>,
) -> Result<Dyn, String> {
    let dir = match text(&f.dir, snap)? {
        d if d.is_empty() => ".".to_string(),
        d => d,
    };
    let pattern = if f.recursive {
        format!("{dir}/**/{}", f.pattern)
    } else {
        format!("{dir}/{}", f.pattern)
    };
    let own = std::env::current_exe()
        .ok()
        .and_then(|e| e.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    for found in effects.glob(cx, &pattern) {
        let p = Path::new(&found);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if f.exclude_self && own.as_deref() == Some(name.to_lowercase().as_str()) {
            continue;
        }
        if let Some(sib) = &f.require_sibling {
            let mut s = snap.clone();
            let stem = p
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ext = p
                .extension()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            s.values.insert("stem".into(), Dyn::str(stem));
            s.values.insert("name".into(), Dyn::str(name.clone()));
            s.values.insert("ext".into(), Dyn::str(ext));
            let sibling = text(sib, &s)?;
            let sib_path = p.parent().unwrap_or(Path::new(".")).join(&sibling);
            if !effects.exists(cx, &sib_path.display().to_string()) {
                continue;
            }
        }
        let full = crate::paths::resolve(config_dir, &found);
        return Ok(Dyn::Str(crate::paths::display_relative(config_dir, &full)));
    }
    Ok(Dyn::Str(String::new()))
}

/// Fields of the data blocks, for `{common.Key}` and `fields.common.Key`.
pub fn data_fields(config: &Config, snap: &mut Snapshot) {
    for d in &config.data {
        snap.fields.insert(
            d.id.clone(),
            d.bag
                .iter()
                .map(|(k, v)| (k.clone(), Dyn::str(v.clone())))
                .collect(),
        );
    }
}

pub struct Evaluator {
    vm: Vm,
    config: Config,
    effects: Arc<dyn Effects>,
    cx: Context,
    /// What the last load worked out (computed option lists, for option labels).
    last: LoadResults,
}

impl Evaluator {
    pub fn new(
        config: Config,
        effects: Arc<dyn Effects>,
        _trusted: Trusted,
    ) -> Result<Evaluator, String> {
        let vm = Vm::new(&config.path, effects.clone())?;
        let cx = Context::new(&config.dir);
        Ok(Evaluator {
            vm,
            config,
            effects,
            cx,
            last: LoadResults::default(),
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// A template or `(lua)` value as text.
    pub fn text(&mut self, v: &Val, snap: &Snapshot) -> Result<String, String> {
        match v {
            Val::Text(t) => Ok(render(t, snap, &self.cx.env)),
            Val::Lua(src) => {
                self.vm.set_snapshot(snap.clone());
                self.vm
                    .eval(src, Some(LOAD_BUDGET))
                    .map(|d| d.to_text())
                    .map_err(|e| e.message)
            }
        }
    }

    fn value(&mut self, v: &Val, snap: &Snapshot) -> Result<Dyn, String> {
        match v {
            Val::Text(t) => Ok(Dyn::Str(render(t, snap, &self.cx.env))),
            Val::Lua(src) => {
                self.vm.set_snapshot(snap.clone());
                self.vm.eval(src, Some(LOAD_BUDGET)).map_err(|e| e.message)
            }
        }
    }

    /// `find-file`: the first match (relative to the file's folder when inside it), or "".
    pub fn find_file(&mut self, f: &FindFile, snap: &Snapshot) -> Result<Dyn, String> {
        let effects = self.effects.clone();
        let cx = self.cx.clone();
        let dir = self.config.dir.clone();
        find_file(effects.as_ref(), &cx, &dir, f, snap, &mut |v, s| {
            self.text(v, s)
        })
    }

    /// Run the scripts and work out every load-time value. `base` holds the form's values so far
    /// (remembered and default values).
    pub fn load(&mut self, base: Snapshot) -> LoadResults {
        let mut out = LoadResults::default();
        let mut snap = base;
        data_fields(&self.config, &mut snap);
        self.vm.set_snapshot(snap.clone());
        let scripts = self.config.scripts.clone();
        for s in &scripts {
            if let Err(e) = self.vm.exec(s, Some(LOAD_BUDGET)) {
                out.script_errors.push(e);
            }
        }
        let values = self.config.values.clone();
        for v in &values {
            let r = match &v.source {
                ValueSource::Val(val) => self.value(val, &snap),
                ValueSource::Find(f) => self.find_file(f, &snap),
            };
            if let Ok(d) = &r {
                snap.values.insert(v.id.clone(), d.clone());
            }
            out.values.insert(v.id.clone(), r);
        }
        let controls = self.config.controls.clone();
        for c in &controls {
            if let ControlKind::Choice {
                options_lua: Some(src),
                ..
            } = &c.kind
            {
                out.options
                    .insert(c.id.clone(), self.options_from(src, &snap));
            }
            if let ControlKind::Choice { list: Some(l), .. } = &c.kind {
                out.options
                    .insert(c.id.clone(), self.list_options(l, &snap));
            }
            if let Some(DefaultVal::Val(Val::Lua(src))) = &c.default {
                self.vm.set_snapshot(snap.clone());
                out.defaults.insert(
                    c.id.clone(),
                    self.vm.eval(src, Some(LOAD_BUDGET)).map_err(|e| e.message),
                );
            }
        }
        self.last = out.clone();
        out
    }

    /// An add-on list's choices: its settings worked out from the form, then its script.
    fn list_options(&mut self, l: &ListRef, snap: &Snapshot) -> Result<Vec<OptionItem>, String> {
        let mut settings = Snapshot::default();
        for s in &l.def.settings {
            let v = match l.settings.iter().find(|(k, _)| *k == s.id) {
                Some((_, v)) => self.value(v, snap)?,
                None => s.starts.clone().unwrap_or_default(),
            };
            settings.values.insert(s.id.clone(), v);
        }
        self.vm.set_snapshot(settings);
        let d = self
            .vm
            .exec_value(&l.def.script, Some(LOAD_BUDGET))
            .map_err(|e| e.message);
        self.vm.set_snapshot(snap.clone());
        Self::option_items(d?)
    }

    fn options_from(&mut self, src: &LuaSrc, snap: &Snapshot) -> Result<Vec<OptionItem>, String> {
        self.vm.set_snapshot(snap.clone());
        let d = self
            .vm
            .eval(src, Some(LOAD_BUDGET))
            .map_err(|e| e.message)?;
        Self::option_items(d)
    }

    /// A list (from Lua) as options: text, numbers, or tables with a name.
    fn option_items(d: Dyn) -> Result<Vec<OptionItem>, String> {
        let Dyn::List(items) = d else {
            return Err("the options expression must return a list".into());
        };
        items
            .into_iter()
            .map(|item| match item {
                Dyn::Str(s) => Ok(OptionItem {
                    name: s.clone(),
                    value: s,
                    label: None,
                    fields: BTreeMap::new(),
                }),
                Dyn::Num(n) => {
                    let s = crate::dynval::fmt_num(n);
                    Ok(OptionItem {
                        name: s.clone(),
                        value: s,
                        label: None,
                        fields: BTreeMap::new(),
                    })
                }
                Dyn::Map(mut m) => {
                    let name = m
                        .remove("name")
                        .map(|v| v.to_text())
                        .ok_or("an option table needs a name field")?;
                    let value = m
                        .remove("value")
                        .map(|v| v.to_text())
                        .unwrap_or(name.clone());
                    let label = m.remove("label").map(|v| v.to_text());
                    Ok(OptionItem {
                        name,
                        value,
                        label,
                        fields: m,
                    })
                }
                other => Err(format!(
                    "an option must be text or a table with a name, not {}",
                    other.to_text()
                )),
            })
            .collect()
    }

    /// The refresh button of a computed list.
    pub fn refresh_options(
        &mut self,
        control: &str,
        snap: &Snapshot,
    ) -> Result<Vec<OptionItem>, String> {
        let kind = self.config.control(control).map(|c| c.kind.clone());
        let r = match kind {
            Some(ControlKind::Choice {
                options_lua: Some(s),
                ..
            }) => self.options_from(&s, snap),
            Some(ControlKind::Choice { list: Some(l), .. }) => self.list_options(&l, snap),
            _ => return Err(format!("`{control}` has no computed options")),
        };
        self.last.options.insert(control.to_string(), r.clone());
        r
    }

    /// Every `visible-if` and `enabled-if` in the layout, for the current values.
    pub fn conditions(&mut self, snap: &Snapshot) -> Conditions {
        self.vm.set_snapshot(snap.clone());
        let mut out = Conditions::new();
        let mut conds: Vec<(NodePath, Which, LuaSrc)> = Vec::new();
        collect_conditions(&self.config.layout, &self.config, &mut conds);
        for (path, which, src) in conds {
            let r = self
                .vm
                .eval(&src, Some(LOAD_BUDGET))
                .map(|d| d.truthy())
                .map_err(|e| e.message);
            out.insert((path, which), r);
        }
        out
    }

    /// Conditions and `(lua)` texts for the current values.
    pub fn live(&mut self, snap: &Snapshot) -> Live {
        let conditions = self.conditions(snap);
        let mut texts = Texts::new();
        let mut srcs: Vec<(NodePath, &'static str, LuaSrc)> = Vec::new();
        collect_lua_texts(&self.config, &mut srcs);
        self.vm.set_snapshot(snap.clone());
        for (path, prop, src) in srcs {
            let r = self
                .vm
                .eval(&src, Some(LOAD_BUDGET))
                .map(|d| d.to_text())
                .map_err(|e| e.message);
            texts.insert((path, prop.to_string()), r);
        }
        let controls = self.config.controls.clone();
        for c in &controls {
            if let ControlKind::Choice {
                option_label: Some(Val::Lua(src)),
                ..
            } = &c.kind
            {
                for (i, o) in crate::form::options(c, &self.last).iter().enumerate() {
                    let mut scope = option_scope(o, i);
                    scope.fields = snap.fields.clone();
                    self.vm.set_snapshot(scope);
                    let r = self
                        .vm
                        .eval(src, Some(LOAD_BUDGET))
                        .map(|d| d.to_text())
                        .map_err(|e| e.message);
                    texts.insert((c.node.path.clone(), format!("option-label:{i}")), r);
                }
            }
        }
        Live { conditions, texts }
    }

    /// A control's on-change block, for the form's current values. Only instant steps run: `set`
    /// (a name later steps read), `set-control`, `reset`, `log`, and `lua` (whose `set()` of a
    /// control is a `set-control`).
    pub fn on_change(&mut self, control: &str, snap: &Snapshot) -> Changes {
        let mut out = Changes::default();
        let Some(steps) = self.config.control(control).map(|c| c.on_change.clone()) else {
            return out;
        };
        let controls: Vec<String> = self.config.controls.iter().map(|c| c.id.clone()).collect();
        let mut snap = snap.clone();
        for s in &steps {
            if let Some(cond) = &s.if_ {
                self.vm.set_snapshot(snap.clone());
                match self.vm.eval(cond, Some(LOAD_BUDGET)) {
                    Ok(d) if d.truthy() => {}
                    Ok(_) => continue,
                    Err(e) => {
                        out.errors.push(e.message);
                        continue;
                    }
                }
            }
            match &s.kind {
                StepKind::Set { name, value } => match self.value(value, &snap) {
                    Ok(v) => {
                        snap.values.insert(name.clone(), v);
                    }
                    Err(e) => out.errors.push(e),
                },
                StepKind::SetControl { control, value } => match self.value(value, &snap) {
                    Ok(v) => {
                        snap.values.insert(control.clone(), v.clone());
                        out.edits.push(Edit::Set(control.clone(), v));
                    }
                    Err(e) => out.errors.push(e),
                },
                StepKind::Reset { control } => out.edits.push(Edit::Reset(control.clone())),
                StepKind::Log { text, .. } => match self.text(text, &snap) {
                    Ok(t) => out.logs.push(t),
                    Err(e) => out.errors.push(e),
                },
                StepKind::Lua { code } => {
                    let found = Arc::new(Mutex::new(Changes::default()));
                    self.vm.set_snapshot(snap.clone());
                    self.vm.set_host(Some(Box::new(ChangeHost {
                        found: found.clone(),
                        controls: controls.clone(),
                    })));
                    self.vm.set_instant(true);
                    let r = self.vm.exec(code, Some(LOAD_BUDGET));
                    self.vm.set_instant(false);
                    self.vm.set_host(None);
                    snap = self.vm.snapshot();
                    if let Ok(mut f) = found.lock() {
                        out.edits.append(&mut f.edits);
                        out.logs.append(&mut f.logs);
                    }
                    if let Err(e) = r {
                        out.errors.push(e.message);
                    }
                }
                other => out.errors.push(format!(
                    "`{}` can't run in on-change (only set, set-control, reset, log and lua)",
                    other.name()
                )),
            }
        }
        out
    }
}

/// On-change Lua's host: `set()` of a control becomes an edit, `log()` a line; nothing else of
/// the run-time API reaches it (the VM refuses it first, `Vm::set_instant`).
struct ChangeHost {
    found: Arc<Mutex<Changes>>,
    controls: Vec<String>,
}

impl Host for ChangeHost {
    fn run(&mut self, _cmd: &CommandSpec, _pipe: bool) -> Result<i32, String> {
        Err("run() can't be used in on-change".into())
    }
    fn step(&mut self, _label: Option<&str>, _ok: bool) {}
    fn log(&mut self, text: &str) {
        if let Ok(mut f) = self.found.lock() {
            f.logs.push(text.to_string());
        }
    }
    fn message(&mut self, _text: &str) {}
    fn confirm(&mut self, _text: &str) -> bool {
        false
    }
    fn ask(&mut self, _prompt: &str, _default: Option<&str>) -> Option<String> {
        None
    }
    fn set(&mut self, name: &str, value: Dyn) {
        if self.controls.iter().any(|c| c == name) {
            if let Ok(mut f) = self.found.lock() {
                f.edits.push(Edit::Set(name.to_string(), value));
            }
        }
    }
}

/// Every `(lua)` text in the form, with its node and property.
fn collect_lua_texts(config: &Config, out: &mut Vec<(NodePath, &'static str, LuaSrc)>) {
    fn add(
        out: &mut Vec<(NodePath, &'static str, LuaSrc)>,
        node: &NodeRef,
        prop: &'static str,
        v: Option<&Val>,
    ) {
        if let Some(Val::Lua(src)) = v {
            out.push((node.path.clone(), prop, src.clone()));
        }
    }
    fn items(config: &Config, list: &[Item], out: &mut Vec<(NodePath, &'static str, LuaSrc)>) {
        for it in list {
            match it {
                Item::Group {
                    title,
                    items: i,
                    node,
                    ..
                } => {
                    add(out, node, "title", title.as_ref());
                    items(config, i, out);
                }
                Item::Tabs { tabs, .. } => {
                    for t in tabs {
                        add(out, &t.node, "title", Some(&t.title));
                        items(config, &t.items, out);
                    }
                }
                Item::Columns { columns, .. } => {
                    for c in columns {
                        items(config, c, out);
                    }
                }
                Item::Row { items: i, .. } => items(config, i, out),
                Item::Separator { title, node } => add(out, node, "title", title.as_ref()),
                Item::Note { text, node, .. } => add(out, node, "text", Some(text)),
                Item::Spacer { .. } => {}
                Item::Control(i) => {
                    let c = &config.controls[*i];
                    add(out, &c.node, "tooltip", c.tooltip.as_ref());
                    add(
                        out,
                        &c.node,
                        "disabled-tooltip",
                        c.disabled_tooltip.as_ref(),
                    );
                    match &c.kind {
                        ControlKind::Display { text, .. } => add(out, &c.node, "text", Some(text)),
                        ControlKind::Button { confirm, .. } => {
                            add(out, &c.node, "confirm", confirm.as_ref())
                        }
                        ControlKind::Choice {
                            options, presets, ..
                        } => {
                            for p in presets {
                                add(out, &p.node, "preset", Some(&p.value));
                            }
                            for o in options {
                                add(out, &o.node, "label", o.label.as_ref());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if let Some(node) = &config.window.node {
        add(out, node, "title", Some(&config.window.title));
    }
    items(config, &config.layout, out);
}

fn collect_conditions(items: &[Item], config: &Config, out: &mut Vec<(NodePath, Which, LuaSrc)>) {
    let mut add = |node: &NodeRef, which: Which, src: &Option<LuaSrc>| {
        if let Some(s) = src {
            out.push((node.path.clone(), which, s.clone()));
        }
    };
    let mut nested: Vec<&[Item]> = Vec::new();
    for it in items {
        match it {
            Item::Group {
                visible_if,
                items,
                node,
                ..
            }
            | Item::Row {
                visible_if,
                items,
                node,
            } => {
                add(node, Which::Visible, visible_if);
                nested.push(items);
            }
            Item::Tabs { tabs, .. } => {
                for t in tabs {
                    add(&t.node, Which::Visible, &t.visible_if);
                    nested.push(&t.items);
                }
            }
            Item::Columns { columns, .. } => {
                for c in columns {
                    nested.push(c);
                }
            }
            Item::Note {
                visible_if, node, ..
            } => add(node, Which::Visible, visible_if),
            Item::Control(i) => {
                let c = &config.controls[*i];
                add(&c.node, Which::Visible, &c.visible_if);
                add(&c.node, Which::Enabled, &c.enabled_if);
            }
            Item::Separator { .. } | Item::Spacer { .. } => {}
        }
    }
    for n in nested {
        collect_conditions(n, config, out);
    }
}

// ---- the evaluation thread ---------------------------------------------------------------------

pub enum Request {
    /// Scripts, values, option lists and defaults. `Loaded` answers; the form then asks for
    /// its `Live` results with its starting values.
    Load(Snapshot),
    /// Conditions and `(lua)` texts for these values.
    Live(Snapshot),
    Refresh {
        control: String,
        snapshot: Snapshot,
    },
    /// An add-on list's choices on their own (the editor's settings for a step), answered by
    /// `Response::List` with the same key.
    List {
        key: String,
        list: ListRef,
    },
    /// A control changed: its on-change block. `depth` counts on-change blocks set off by
    /// other on-change blocks.
    OnChange {
        control: String,
        snapshot: Snapshot,
        depth: usize,
    },
}

pub enum Response {
    Loaded(Box<LoadResults>),
    Live(Box<Live>),
    Options {
        control: String,
        result: Result<Vec<OptionItem>, String>,
    },
    Changed {
        control: String,
        changes: Changes,
        depth: usize,
    },
    List {
        key: String,
        result: Result<Vec<OptionItem>, String>,
    },
    /// The evaluator couldn't start.
    Failed(String),
}

/// A thread that owns the file's Lua VM and answers requests in order; only the newest pending
/// `Live` request is answered (the form may change faster than conditions evaluate).
pub struct EvalThread {
    tx: mpsc::Sender<Request>,
}

impl EvalThread {
    pub fn start(
        config: Config,
        effects: Arc<dyn Effects>,
        trusted: Trusted,
        on_response: impl Fn(Response) + Send + 'static,
    ) -> EvalThread {
        let (tx, rx) = mpsc::channel::<Request>();
        let _ = std::thread::Builder::new()
            .name("evaluate".into())
            .spawn(move || {
                let mut ev = match Evaluator::new(config, effects, trusted) {
                    Ok(e) => e,
                    Err(e) => {
                        on_response(Response::Failed(e));
                        return;
                    }
                };
                while let Ok(first) = rx.recv() {
                    // Take everything queued; a newer Conditions replaces an older one.
                    let mut batch = vec![first];
                    while let Ok(r) = rx.try_recv() {
                        batch.push(r);
                    }
                    let last_live = batch.iter().rposition(|r| matches!(r, Request::Live(_)));
                    for (i, req) in batch.into_iter().enumerate() {
                        match req {
                            Request::Load(snap) => {
                                let results = ev.load(snap);
                                on_response(Response::Loaded(Box::new(results)));
                            }
                            Request::Live(snap) => {
                                if Some(i) == last_live {
                                    on_response(Response::Live(Box::new(ev.live(&snap))));
                                }
                            }
                            Request::OnChange {
                                control,
                                snapshot,
                                depth,
                            } => {
                                let changes = ev.on_change(&control, &snapshot);
                                on_response(Response::Changed {
                                    control,
                                    changes,
                                    depth,
                                });
                            }
                            Request::List { key, list } => {
                                let result = ev.list_options(&list, &Snapshot::default());
                                on_response(Response::List { key, result });
                            }
                            Request::Refresh { control, snapshot } => {
                                let result = ev.refresh_options(&control, &snapshot);
                                on_response(Response::Options { control, result });
                            }
                        }
                    }
                }
            });
        EvalThread { tx }
    }

    pub fn send(&self, r: Request) {
        let _ = self.tx.send(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::effects::MockEffects;
    use crate::platform::trust;

    const FILE: &str = r##"
script #"""
    function branches()
        return split(capture("git", "branch", "--format=%(refname:short)"), "\n")
    end
    """#
value "app" {
    find-file "*.exe" require-sibling="{stem}_Data"
}
value "greeting" "Hello {env.LAUNCHER_TEST_USER} from {app}"
value "twice" (lua)"values.greeting .. '!'"
data "common" {
    Mode "fast"
}
layout {
    dropdown "branch" label="Branch" options=(lua)"branches()" default=(lua)"'main'"
    dropdown "server" label="Server" options=(lua)"{ {name='Local', ip='127.0.0.1'}, 'Other' }"
    checkbox "tests" label="Tests" visible-if="values.branch == 'main'" enabled-if="fields.common.Mode == 'fast'"
    group "More" visible-if="values.nope" {
        text "x" label="X"
    }
}
"##;

    fn config() -> Config {
        let l = load_str(
            FILE,
            Path::new("C:/work/tool.launcher"),
            &LoadOptions::default(),
        );
        assert!(!l.has_errors(), "{:?}", l.diags);
        l.config
    }

    #[test]
    fn works_out_values_options_defaults_and_conditions() {
        let fx = Arc::new(MockEffects::default());
        fx.answer_capture("git", "main\nfeature/x\n", 0);
        fx.add_file("./Game.exe", "");
        fx.add_file("./Game_Data", "");
        fx.add_file("./Tool.exe", "");
        let mut ev = Evaluator::new(config(), fx.clone(), trust::assume()).unwrap();
        ev.cx.env.insert("LAUNCHER_TEST_USER".into(), "ada".into());
        let r = ev.load(Snapshot::default());
        assert!(r.script_errors.is_empty(), "{:?}", r.script_errors);
        assert_eq!(r.values["app"], Ok(Dyn::str("Game.exe")));
        assert_eq!(
            r.values["greeting"],
            Ok(Dyn::str("Hello ada from Game.exe"))
        );
        assert_eq!(r.values["twice"], Ok(Dyn::str("Hello ada from Game.exe!")));
        let branches = r.options["branch"].as_ref().unwrap();
        assert_eq!(
            branches.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
            ["main", "feature/x"]
        );
        let servers = r.options["server"].as_ref().unwrap();
        assert_eq!(servers[0].fields["ip"], Dyn::str("127.0.0.1"));
        assert_eq!(servers[1].value, "Other");
        assert_eq!(r.defaults["branch"], Ok(Dyn::str("main")));

        let mut snap = Snapshot::default();
        data_fields(ev.config(), &mut snap);
        snap.values.insert("branch".into(), Dyn::str("main"));
        let conds = ev.conditions(&snap);
        let tests = ev.config().control("tests").unwrap().node.path.clone();
        assert_eq!(conds[&(tests.clone(), Which::Visible)], Ok(true));
        assert_eq!(conds[&(tests, Which::Enabled)], Ok(true));
        assert!(
            conds.values().any(|v| *v == Ok(false)),
            "the group is hidden"
        );
    }

    #[test]
    fn a_marked_file_evaluates_nothing_before_it_is_trusted() {
        let dir = std::env::temp_dir().join(format!("launcher-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::paths::use_test_data_dir();
        let file = dir.join("downloaded.launcher");
        std::fs::write(&file, FILE).unwrap();
        if cfg!(windows) {
            std::fs::write(
                format!("{}:Zone.Identifier", file.display()),
                "[ZoneTransfer]\r\nZoneId=3\r\n",
            )
            .unwrap();
        } else {
            // macOS marks come from the OS (the quarantine attribute); nothing to fake here.
            return;
        }
        // The gate says no: there is no token, so no evaluator, so nothing runs.
        let mark = trust::check(&file, FILE).unwrap_err();
        assert_eq!(mark, trust::Mark::Web { zone: 3 });
        // Once the user says yes, the same text evaluates; a changed file asks again.
        let token = trust::grant(&file, FILE).unwrap();
        assert!(trust::check(&file, FILE).is_ok());
        assert!(trust::check(&file, &format!("{FILE}\n// changed")).is_err());
        let fx = Arc::new(MockEffects::default());
        fx.answer_capture("git", "main\n", 0);
        let mut ev = Evaluator::new(config(), fx.clone(), token).unwrap();
        ev.load(Snapshot::default());
        assert!(!fx.calls().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn works_out_lua_texts_and_runs_on_change_blocks() {
        let l = load_str(
            r#"
window title=(lua)"'Build ' .. values.mode"
layout {
    radio "mode" label="Mode" {
        option "Debug"
        option "Release"
        on-change {
            set "was" "{mode}"
            set-control "tests" (lua)"values.was == 'Release'"
            reset "who" if="values.mode == 'Debug'"
            log "mode is {mode}"
            lua "set('who', 'lua ' .. values.mode); log('from lua')"
            lua "copy('a', 'b')"
        }
    }
    checkbox "tests" label="Tests"
    text "who" label="Who" tooltip=(lua)"'hello ' .. values.who"
    dropdown "server" label="Server" option-label=(lua)"values.name .. ' @ ' .. values.ip" {
        option "Local" {
            ip "127.0.0.1"
        }
    }
}
"#,
            Path::new("C:/work/tool.launcher"),
            &LoadOptions::default(),
        );
        assert!(!l.has_errors(), "{:?}", l.diags);
        let fx = Arc::new(MockEffects::default());
        let mut ev = Evaluator::new(l.config, fx.clone(), trust::assume()).unwrap();
        ev.load(Snapshot::default());
        let mut snap = Snapshot::default();
        snap.values.insert("mode".into(), Dyn::str("Release"));
        snap.values.insert("who".into(), Dyn::str("ada"));

        let live = ev.live(&snap);
        let title = ev.config().window.node.clone().unwrap().path;
        assert_eq!(
            live.texts[&(title, "title".to_string())],
            Ok("Build Release".to_string())
        );
        let who = ev.config().control("who").unwrap().node.path.clone();
        assert_eq!(
            live.texts[&(who, "tooltip".to_string())],
            Ok("hello ada".to_string())
        );
        let server = ev.config().control("server").unwrap().node.path.clone();
        assert_eq!(
            live.texts[&(server, "option-label:0".to_string())],
            Ok("Local @ 127.0.0.1".to_string())
        );

        let ch = ev.on_change("mode", &snap);
        assert_eq!(
            ch.edits,
            [
                Edit::Set("tests".into(), Dyn::Bool(true)),
                Edit::Set("who".into(), Dyn::str("lua Release")),
            ]
        );
        assert_eq!(ch.logs, ["mode is Release", "from lua"]);
        // copy() is refused, and nothing reached the effects.
        assert_eq!(ch.errors.len(), 1, "{:?}", ch.errors);
        assert!(
            ch.errors[0].contains("only set() and log()"),
            "{:?}",
            ch.errors
        );
        assert!(fx.calls().is_empty(), "{:?}", fx.calls());

        snap.values.insert("mode".into(), Dyn::str("Debug"));
        let ch = ev.on_change("mode", &snap);
        assert_eq!(ch.edits[1], Edit::Reset("who".into()), "{ch:?}");
    }

    #[test]
    fn the_thread_answers_in_order() {
        let fx = Arc::new(MockEffects::default());
        fx.answer_capture("git", "main\n", 0);
        let (tx, rx) = mpsc::channel();
        let t = EvalThread::start(config(), fx, trust::assume(), move |r| {
            let _ = tx.send(r);
        });
        t.send(Request::Load(Snapshot::default()));
        t.send(Request::Refresh {
            control: "branch".into(),
            snapshot: Snapshot::default(),
        });
        let first = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert!(matches!(first, Response::Loaded(..)));
        let second = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert!(
            matches!(second, Response::Options { control, result: Ok(_) } if control == "branch")
        );
    }
}

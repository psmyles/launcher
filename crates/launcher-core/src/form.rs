//! The form's values: what each control holds, and the rules that tie controls together. The
//! window, the CLI and the tests all drive it through [`Form::set`] and friends; it runs no Lua
//! itself (N6), so on-change blocks and `(lua)` texts come from the evaluation thread
//! ([`crate::eval`]) and are applied here.
//!
//! **Where a value comes from**, first match wins: a preset, the value chosen this session (the
//! user, `--set`, `set-control`), the remembered value (N15, unless `persist=#false`), the
//! default, the first option.
//!
//! **Presets** (a role that picks a mode, say): a control's `preset` is worked out again whenever the
//! values change.
//! * Non-empty: the option it names is selected. The user may still pick another one; that
//!   choice lasts until the preset's value changes and is **not** remembered.
//! * Empty: the preset lets go and the control returns to its own value.
//! * A value that matches no option is a problem (N15).
//!
//! The result is the [`Snapshot`] templates and Lua read: `values.x` for a control's value and
//! `fields.x.Key` for its chosen option's data (plus `name`, `value` and `label`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::config::*;
use crate::dynval::Dyn;
use crate::eval::{render, Changes, Edit, Evaluator, LoadResults, OptionItem, Texts};
use crate::lua::Snapshot;
use crate::state::Stored;

/// How many times values are worked out again when a preset or a default reads a control that
/// comes later in the form.
const MAX_PASSES: usize = 4;

/// A control's current options (static ones, or the computed list). Choices a step filled in
/// (`put`) come before both.
pub fn options(c: &Control, results: &LoadResults) -> Vec<OptionItem> {
    if let Some(Ok(filled)) = results.options.get(&c.id) {
        if matches!(c.kind, ControlKind::Choice { .. }) {
            return filled.clone();
        }
    }
    match &c.kind {
        ControlKind::Choice {
            options,
            options_lua,
            list,
            ..
        } => {
            if options_lua.is_some() || list.is_some() {
                results
                    .options
                    .get(&c.id)
                    .and_then(|r| r.as_ref().ok())
                    .cloned()
                    .unwrap_or_default()
            } else {
                options
                    .iter()
                    .map(|o| OptionItem {
                        name: o.name.clone(),
                        value: o.value.clone(),
                        label: None,
                        fields: o
                            .bag
                            .iter()
                            .map(|(k, v)| (k.clone(), Dyn::str(v.clone())))
                            .collect(),
                    })
                    .collect()
            }
        }
        _ => Vec::new(),
    }
}

/// The fields a chosen option exposes: its data plus name, value and label.
pub fn option_fields(o: &OptionItem) -> BTreeMap<String, Dyn> {
    let mut m = o.fields.clone();
    m.insert("name".into(), Dyn::str(o.name.clone()));
    m.insert("value".into(), Dyn::str(o.value.clone()));
    m.insert(
        "label".into(),
        Dyn::str(o.label.clone().unwrap_or_else(|| o.name.clone())),
    );
    m
}

fn stored_to_dyn(s: &Stored) -> Dyn {
    match s {
        Stored::Bool(b) => Dyn::Bool(*b),
        Stored::Num(n) => Dyn::Num(*n),
        Stored::Text(t) => Dyn::Str(t.clone()),
        Stored::List(l) => Dyn::List(l.iter().map(|x| Dyn::Str(x.clone())).collect()),
    }
}

/// What the state file keeps for a value.
pub fn to_stored(v: &Dyn) -> Stored {
    match v {
        Dyn::Bool(b) => Stored::Bool(*b),
        Dyn::Num(n) => Stored::Num(*n),
        Dyn::List(l) => Stored::List(l.iter().map(Dyn::to_text).collect()),
        other => Stored::Text(other.to_text()),
    }
}

/// Text given on the command line, as the control's kind of value.
fn parse_set(kind: &ControlKind, text: &str) -> Result<Dyn, String> {
    match kind {
        ControlKind::Checkbox => match text.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => Ok(Dyn::Bool(true)),
            "false" | "no" | "0" | "off" => Ok(Dyn::Bool(false)),
            _ => Err(format!("`{text}` is not true or false")),
        },
        ControlKind::Slider { .. } | ControlKind::Number { .. } => text
            .parse::<f64>()
            .map(Dyn::Num)
            .map_err(|_| format!("`{text}` is not a number")),
        ControlKind::Choice {
            style: ChoiceStyle::Checklist,
            ..
        } => Ok(list_of(&Dyn::str(text))),
        _ => Ok(Dyn::str(text)),
    }
}

/// A checklist's value: a list, or text with the items separated by commas.
fn list_of(v: &Dyn) -> Dyn {
    match v {
        Dyn::List(l) => Dyn::List(l.iter().map(|x| Dyn::Str(x.to_text())).collect()),
        Dyn::Nil => Dyn::List(Vec::new()),
        other => Dyn::List(
            other
                .to_text()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(Dyn::str)
                .collect(),
        ),
    }
}

fn number_of(v: &Dyn, fallback: f64) -> f64 {
    match v {
        Dyn::Num(n) => *n,
        Dyn::Bool(b) => *b as u8 as f64,
        other => other.to_text().trim().parse().unwrap_or(fallback),
    }
}

/// A value as the control's kind holds it.
fn normalize(c: &Control, v: Dyn) -> Dyn {
    match &c.kind {
        // `set-control "x" "false"` gives text: read it as a word, not as Lua truth.
        ControlKind::Checkbox => Dyn::Bool(match &v {
            Dyn::Bool(b) => *b,
            Dyn::Num(n) => *n != 0.0,
            Dyn::Nil => false,
            other => !matches!(
                other.to_text().trim().to_ascii_lowercase().as_str(),
                "" | "false" | "no" | "0" | "off"
            ),
        }),
        ControlKind::Slider { min, max, .. } => Dyn::Num(number_of(&v, *min).clamp(*min, *max)),
        ControlKind::Number { min, max, .. } => {
            let mut n = number_of(&v, 0.0);
            if let Some(lo) = min {
                n = n.max(*lo);
            }
            if let Some(hi) = max {
                n = n.min(*hi);
            }
            Dyn::Num(n)
        }
        ControlKind::Choice {
            style: ChoiceStyle::Checklist,
            ..
        } => list_of(&v),
        _ => Dyn::Str(v.to_text()),
    }
}

pub struct Form {
    config: Arc<Config>,
    results: LoadResults,
    env: BTreeMap<String, String>,
    /// `(lua)` presets and option labels, from the evaluation thread.
    texts: Texts,
    remembered: BTreeMap<String, Stored>,
    /// Values chosen this session, over remembered values and defaults.
    chosen: BTreeMap<String, Dyn>,
    /// The user's choice over a preset: the preset's value at the time, and the choice.
    overrides: BTreeMap<String, (String, Dyn)>,
    /// Controls whose preset currently names a value.
    preset_want: BTreeMap<String, String>,
    /// Controls a preset currently holds (no override).
    held: BTreeSet<String>,
    snap: Snapshot,
    problems: Vec<String>,
    /// Choices steps filled in (`put`), kept over newly worked-out lists and remembered.
    filled: BTreeMap<String, Vec<String>>,
    /// A remembered value or filled choices changed since the last [`Form::take_dirty`].
    dirty: bool,
    /// Text fields' `pattern=`, compiled once.
    patterns: BTreeMap<String, Result<regex::Regex, String>>,
}

/// Plain text choices as options.
fn option_items(items: &[String]) -> Vec<OptionItem> {
    items
        .iter()
        .map(|t| OptionItem {
            name: t.clone(),
            value: t.clone(),
            label: None,
            fields: BTreeMap::new(),
        })
        .collect()
}

impl Form {
    pub fn new(
        config: Arc<Config>,
        results: LoadResults,
        remembered: BTreeMap<String, Stored>,
        env: BTreeMap<String, String>,
    ) -> Form {
        let mut f = Form {
            config,
            results,
            env,
            texts: Texts::new(),
            remembered,
            chosen: BTreeMap::new(),
            overrides: BTreeMap::new(),
            preset_want: BTreeMap::new(),
            held: BTreeSet::new(),
            snap: Snapshot::default(),
            problems: Vec::new(),
            filled: BTreeMap::new(),
            dirty: false,
            patterns: BTreeMap::new(),
        };
        for c in &f.config.controls {
            if let ControlKind::Text {
                pattern: Some(p), ..
            } = &c.kind
            {
                // Anchored: the whole value must match.
                let re = regex::Regex::new(&format!("^(?:{p})$")).map_err(|e| e.to_string());
                f.patterns.insert(c.id.clone(), re);
            }
        }
        // Forget remembered values of controls that are gone or no longer remembered, so the
        // state file doesn't grow forever.
        let keep: BTreeSet<&str> = f
            .config
            .controls
            .iter()
            .filter(|c| c.persist && c.has_value())
            .map(|c| c.id.as_str())
            .collect();
        f.remembered.retain(|k, _| keep.contains(k.as_str()));
        f.compute();
        f
    }

    pub fn config(&self) -> &Arc<Config> {
        &self.config
    }

    pub fn results(&self) -> &LoadResults {
        &self.results
    }

    /// What templates and Lua read.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snap
    }

    pub fn value(&self, id: &str) -> Dyn {
        self.snap.values.get(id).cloned().unwrap_or(Dyn::Nil)
    }

    /// The control's options (static, or computed when the file loaded).
    pub fn options(&self, id: &str) -> Vec<OptionItem> {
        self.config
            .control(id)
            .map(|c| options(c, &self.results))
            .unwrap_or_default()
    }

    /// A preset chose this control's value (and the user hasn't picked another since).
    pub fn preset_holds(&self, id: &str) -> bool {
        self.held.contains(id)
    }

    /// Presets that match no option, and `--set` values that don't fit.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Why the control's value keeps actions from running (`required`, `pattern`,
    /// `must-exist`), or `None` when it's fine.
    pub fn check(&self, id: &str) -> Option<String> {
        let c = self.config.control(id)?;
        let name = c.label.clone().unwrap_or_else(|| c.id.clone());
        let v = self.value(id);
        let text = v.to_text();
        let empty = match &v {
            Dyn::Nil => true,
            Dyn::List(l) => l.is_empty(),
            _ => text.trim().is_empty(),
        };
        if c.required && empty {
            return Some(match &c.kind {
                ControlKind::Choice {
                    style: ChoiceStyle::Checklist,
                    ..
                } => format!("Tick at least one {name} option"),
                ControlKind::Choice { .. } => format!("Pick a {name}"),
                _ => format!("Fill in {name}"),
            });
        }
        if empty {
            return None;
        }
        match &c.kind {
            ControlKind::Text {
                pattern_message, ..
            } => match self.patterns.get(id) {
                Some(Ok(re)) if !re.is_match(&text) => Some(
                    pattern_message
                        .clone()
                        .unwrap_or_else(|| format!("{name} isn't in the expected form")),
                ),
                Some(Err(e)) => Some(format!("{name}: the pattern is not valid ({e})")),
                _ => None,
            },
            ControlKind::File {
                filter, must_exist, ..
            } => {
                let exts = filter
                    .as_deref()
                    .map(crate::paths::extensions)
                    .unwrap_or_default();
                let p = crate::paths::resolve(&self.config.dir, &text);
                if !crate::paths::has_extension(&text, &exts) {
                    Some(format!(
                        "{name}: choose a {} file",
                        crate::paths::extension_words(&exts)
                    ))
                } else if *must_exist && !p.is_file() {
                    Some(format!("{name}: {} doesn't exist", p.display()))
                } else {
                    None
                }
            }
            ControlKind::Folder {
                must_exist: true, ..
            } => {
                let p = crate::paths::resolve(&self.config.dir, &text);
                (!p.is_dir()).then(|| format!("{name}: {} doesn't exist", p.display()))
            }
            _ => None,
        }
    }

    /// What to write to the state file.
    pub fn remembered(&self) -> &BTreeMap<String, Stored> {
        &self.remembered
    }

    /// True once after a remembered value changed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// A change by the user or a `set-control`. Returns the controls whose value changed (this
    /// one and any a preset ties to it), for their on-change blocks.
    pub fn set(&mut self, id: &str, v: Dyn) -> Vec<String> {
        let Some(c) = self.config.control(id).filter(|c| c.has_value()) else {
            return Vec::new();
        };
        let v = normalize(c, v);
        let persist = c.persist;
        match self.preset_want.get(id) {
            Some(want) => {
                self.overrides.insert(id.to_string(), (want.clone(), v));
            }
            None => {
                self.chosen.insert(id.to_string(), v.clone());
                if persist {
                    let s = to_stored(&v);
                    if self.remembered.get(id) != Some(&s) {
                        self.remembered.insert(id.to_string(), s);
                        self.dirty = true;
                    }
                }
            }
        }
        self.recompute()
    }

    /// `--set id=text`: the text as the control's kind of value.
    pub fn set_text(&mut self, id: &str, text: &str) -> Result<Vec<String>, String> {
        let c = self
            .config
            .control(id)
            .filter(|c| c.has_value())
            .ok_or_else(|| format!("there is no control `{id}` that holds a value"))?;
        let v = parse_set(&c.kind, text).map_err(|e| format!("{id}: {e}"))?;
        Ok(self.set(id, v))
    }

    /// Back to the default (a `reset` step): the session's and the remembered choice are dropped.
    pub fn reset(&mut self, id: &str) -> Vec<String> {
        self.chosen.remove(id);
        self.overrides.remove(id);
        if self.remembered.remove(id).is_some() {
            self.dirty = true;
        }
        self.recompute()
    }

    /// An on-change block's edit.
    pub fn apply(&mut self, edit: &Edit) -> Vec<String> {
        match edit {
            Edit::Set(id, v) => self.set(id, v.clone()),
            Edit::Reset(id) => self.reset(id),
        }
    }

    /// What the file's load worked out (it arrives after the form is first shown).
    pub fn set_results(&mut self, results: LoadResults) -> Vec<String> {
        self.results = results;
        for (id, items) in &self.filled {
            self.results
                .options
                .insert(id.clone(), Ok(option_items(items)));
        }
        self.recompute()
    }

    /// A step's list became a choice's choices (`put`). They stay, and are remembered.
    pub fn set_choices(&mut self, id: &str, items: Vec<String>) -> Vec<String> {
        if self.config.control(id).is_none() {
            return Vec::new();
        }
        self.results
            .options
            .insert(id.to_string(), Ok(option_items(&items)));
        if self.filled.get(id) != Some(&items) {
            self.filled.insert(id.to_string(), items);
            self.dirty = true;
        }
        self.recompute()
    }

    /// Choices filled in earlier (from the state file).
    pub fn set_filled(&mut self, filled: BTreeMap<String, Vec<String>>) -> Vec<String> {
        let filled: BTreeMap<String, Vec<String>> = filled
            .into_iter()
            .filter(|(id, _)| {
                self.config
                    .control(id)
                    .is_some_and(|c| matches!(c.kind, ControlKind::Choice { .. }))
            })
            .collect();
        for (id, items) in &filled {
            self.results
                .options
                .insert(id.clone(), Ok(option_items(items)));
        }
        self.filled = filled;
        self.recompute()
    }

    pub fn filled(&self) -> &BTreeMap<String, Vec<String>> {
        &self.filled
    }

    /// One computed list again (the refresh button).
    pub fn set_options(
        &mut self,
        id: &str,
        result: Result<Vec<OptionItem>, String>,
    ) -> Vec<String> {
        self.results.options.insert(id.to_string(), result);
        self.recompute()
    }

    /// New `(lua)` texts: a `(lua)` preset may now name another option.
    pub fn set_texts(&mut self, texts: &Texts) -> Vec<String> {
        let presets_changed = self.config.controls.iter().any(|c| match &c.kind {
            ControlKind::Choice { presets, .. } => presets.iter().any(|p| {
                let key = (p.node.path.clone(), "preset".to_string());
                matches!(p.value, Val::Lua(_)) && self.texts.get(&key) != texts.get(&key)
            }),
            _ => false,
        });
        self.texts = texts.clone();
        if presets_changed {
            self.recompute()
        } else {
            Vec::new()
        }
    }

    fn recompute(&mut self) -> Vec<String> {
        let before = self.snap.values.clone();
        self.compute();
        self.config
            .controls
            .iter()
            .filter(|c| c.has_value() && before.get(&c.id) != self.snap.values.get(&c.id))
            .map(|c| c.id.clone())
            .collect()
    }

    /// Work out every value, again while a later control changes an earlier one's preset or
    /// default.
    fn compute(&mut self) {
        let mut base = Snapshot::default();
        crate::eval::data_fields(&self.config, &mut base);
        for (id, v) in &self.results.values {
            if let Ok(v) = v {
                base.values.insert(id.clone(), v.clone());
            }
        }
        let mut prev = base.clone();
        for _ in 0..MAX_PASSES {
            let snap = self.pass(&base, &prev);
            let settled = snap == prev;
            prev = snap;
            if settled {
                break;
            }
        }
        // A choice made over a preset lasts until the preset's value changes.
        let want = &self.preset_want;
        self.overrides
            .retain(|id, (w, _)| want.get(id).is_some_and(|now| now == w));
        self.snap = prev;
    }

    /// One pass over the controls in form order. Templates read `prev` (the last pass) overlaid
    /// with what this pass has worked out so far.
    fn pass(&mut self, base: &Snapshot, prev: &Snapshot) -> Snapshot {
        let mut snap = base.clone();
        let mut lookup = prev.clone();
        let mut problems = Vec::new();
        self.preset_want.clear();
        self.held.clear();
        let config = self.config.clone();
        for c in config.controls.iter().filter(|c| c.has_value()) {
            let remembered = if c.persist {
                self.remembered.get(&c.id).map(stored_to_dyn)
            } else {
                None
            };
            let default = match &c.default {
                Some(DefaultVal::Bool(b)) => Some(Dyn::Bool(*b)),
                Some(DefaultVal::Num(n)) => Some(Dyn::Num(*n)),
                Some(DefaultVal::Val(Val::Text(t))) => {
                    Some(Dyn::Str(render(t, &lookup, &self.env)))
                }
                Some(DefaultVal::Val(Val::Lua(_))) => self
                    .results
                    .defaults
                    .get(&c.id)
                    .and_then(|r| r.as_ref().ok())
                    .cloned(),
                None => None,
            };
            let own = [self.chosen.get(&c.id).cloned(), remembered, default];
            let (value, fields) = match &c.kind {
                ControlKind::Choice {
                    style: ChoiceStyle::Checklist,
                    ..
                } => (
                    own.into_iter()
                        .flatten()
                        .next()
                        .map(|v| list_of(&v))
                        .unwrap_or(Dyn::List(Vec::new())),
                    None,
                ),
                ControlKind::Choice { presets, .. } => {
                    let opts = options(c, &self.results);
                    let find = |value: &str, field: Option<&str>| -> Option<&OptionItem> {
                        opts.iter().find(|o| match field {
                            None => o.value == value || o.name == value,
                            Some(f) => o
                                .fields
                                .iter()
                                .any(|(k, v)| k.eq_ignore_ascii_case(f) && v.to_text() == value),
                        })
                    };
                    let mut picked: Option<&OptionItem> = None;
                    for p in presets {
                        let wanted = match &p.value {
                            Val::Text(t) => render(t, &lookup, &self.env),
                            Val::Lua(_) => self
                                .texts
                                .get(&(p.node.path.clone(), "preset".to_string()))
                                .and_then(|r| r.as_ref().ok())
                                .cloned()
                                .unwrap_or_default(),
                        };
                        let wanted = wanted.trim().to_string();
                        if wanted.is_empty() {
                            continue;
                        }
                        self.preset_want.insert(c.id.clone(), wanted.clone());
                        match find(&wanted, p.match_field.as_deref()) {
                            Some(o) => {
                                let overridden =
                                    self.overrides.get(&c.id).is_some_and(|(w, _)| *w == wanted);
                                if !overridden {
                                    picked = Some(o);
                                    self.held.insert(c.id.clone());
                                }
                            }
                            None => problems.push(format!(
                                "{}: the preset value `{wanted}` matches no option{}",
                                c.id,
                                p.match_field
                                    .as_ref()
                                    .map(|f| format!(" (by {f})"))
                                    .unwrap_or_default()
                            )),
                        }
                        break;
                    }
                    let over = self
                        .overrides
                        .get(&c.id)
                        .filter(|(w, _)| self.preset_want.get(&c.id) == Some(w))
                        .map(|(_, v)| v.clone());
                    let chosen = picked
                        .or_else(|| {
                            std::iter::once(over)
                                .chain(own)
                                .flatten()
                                .find_map(|v| find(&v.to_text(), None))
                        })
                        .or(opts.first());
                    match chosen {
                        Some(o) => (Dyn::str(o.value.clone()), Some(option_fields(o))),
                        None => (Dyn::Nil, None),
                    }
                }
                _ => {
                    let v = own.into_iter().flatten().next();
                    let v = match (&c.kind, v) {
                        (ControlKind::Checkbox, v) => v.unwrap_or(Dyn::Bool(false)),
                        (ControlKind::Slider { min, .. }, v) => v.unwrap_or(Dyn::Num(*min)),
                        (ControlKind::Number { .. }, v) => v.unwrap_or(Dyn::Num(0.0)),
                        (_, v) => v.unwrap_or(Dyn::str("")),
                    };
                    (normalize(c, v), None)
                }
            };
            snap.values.insert(c.id.clone(), value.clone());
            lookup.values.insert(c.id.clone(), value);
            match fields {
                Some(f) => {
                    snap.fields.insert(c.id.clone(), f.clone());
                    lookup.fields.insert(c.id.clone(), f);
                }
                None => {
                    lookup.fields.remove(&c.id);
                }
            }
        }
        self.problems = problems;
        snap
    }
}

/// How deep on-change blocks may set off other on-change blocks.
pub const MAX_CHANGE_DEPTH: usize = 8;

/// Run the on-change blocks of `changed` controls, and of the controls those change, right away
/// (the CLI's `--set`; the window does the same through the evaluation thread). Returns what
/// they logged and their errors.
pub fn follow(form: &mut Form, ev: &mut Evaluator, changed: Vec<String>) -> Changes {
    let mut out = Changes::default();
    let mut queue: std::collections::VecDeque<(String, usize)> =
        changed.into_iter().map(|id| (id, 0)).collect();
    while let Some((id, depth)) = queue.pop_front() {
        let has_block = form
            .config()
            .control(&id)
            .is_some_and(|c| !c.on_change.is_empty());
        if !has_block {
            continue;
        }
        if depth >= MAX_CHANGE_DEPTH {
            out.errors.push(format!(
                "on-change of `{id}` set off too many others; stopped"
            ));
            continue;
        }
        let snap = form.snapshot().clone();
        let mut ch = ev.on_change(&id, &snap);
        for e in &ch.edits {
            for next in form.apply(e) {
                queue.push_back((next, depth + 1));
            }
        }
        out.logs.append(&mut ch.logs);
        out.errors.append(&mut ch.errors);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const FILE: &str = r#"
layout {
    dropdown "role" label="Role" {
        option "Viewer" {
            Mode "KeyboardOnly"
        }
        option "Player"
    }
    dropdown "mode" label="Mode" {
        preset "{role.Mode}"
        option "Gamepad"
        option "Keyboard" value="KeyboardOnly"
    }
    checkbox "tests" label="Tests" default=#true
    slider "volume" min=0 max=100 default=50
    text "who" label="Who" default="{env.LAUNCHER_FORM_USER}"
    text "secret" label="Secret" persist=#false
    checklist "parts" label="Parts" default="a, b" {
        option "a"
        option "b"
        option "c"
    }
}
"#;

    fn cfg() -> Arc<Config> {
        let l = load_str(FILE, Path::new("x.launcher"), &LoadOptions::default());
        assert!(!l.has_errors(), "{:?}", l.diags);
        Arc::new(l.config)
    }

    fn form(remembered: &[(&str, Stored)]) -> Form {
        let env: BTreeMap<String, String> =
            [("LAUNCHER_FORM_USER".to_string(), "ada".to_string())].into();
        Form::new(
            cfg(),
            LoadResults::default(),
            remembered
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            env,
        )
    }

    #[test]
    fn values_come_from_presets_chosen_remembered_and_defaults() {
        let mut f = form(&[
            ("mode", Stored::Text("Gamepad".into())),
            ("volume", Stored::Num(80.0)),
            ("secret", Stored::Text("leaked".into())),
            ("gone", Stored::Text("old".into())),
        ]);
        f.set_text("tests", "no").unwrap();
        assert!(f.problems().is_empty(), "{:?}", f.problems());
        let snap = f.snapshot();
        // The first role presets the mode, over the remembered "Gamepad".
        assert_eq!(snap.values["role"], Dyn::str("Viewer"));
        assert_eq!(snap.values["mode"], Dyn::str("KeyboardOnly"));
        assert!(f.preset_holds("mode"));
        assert_eq!(snap.fields["mode"]["name"], Dyn::str("Keyboard"));
        assert_eq!(snap.values["tests"], Dyn::Bool(false));
        assert_eq!(snap.values["volume"], Dyn::Num(80.0));
        assert_eq!(snap.values["who"], Dyn::str("ada"));
        assert_eq!(
            snap.values["parts"],
            Dyn::List(vec![Dyn::str("a"), Dyn::str("b")])
        );
        // persist=#false ignores a remembered value; a gone control is forgotten.
        assert_eq!(snap.values["secret"], Dyn::str(""));
        assert!(!f.remembered().contains_key("gone"));
    }

    #[test]
    fn a_preset_lets_go_and_overrides_are_not_remembered() {
        let mut f = form(&[("mode", Stored::Text("Gamepad".into()))]);
        assert_eq!(f.value("mode"), Dyn::str("KeyboardOnly"));
        // The user picks another mode while the preset applies: shown, not remembered.
        let changed = f.set("mode", Dyn::str("Gamepad"));
        assert_eq!(changed, ["mode"]);
        assert_eq!(f.value("mode"), Dyn::str("Gamepad"));
        assert!(!f.preset_holds("mode"));
        assert!(!f.take_dirty());
        assert_eq!(f.remembered()["mode"], Stored::Text("Gamepad".into()));
        // A role without a Mode: the preset lets go, back to the remembered choice.
        let changed = f.set("role", Dyn::str("Player"));
        assert_eq!(changed, ["role"]);
        assert!(f.take_dirty());
        assert_eq!(f.value("mode"), Dyn::str("Gamepad"));
        // Back to Viewer: the preset applies again (the old override is gone).
        let changed = f.set("role", Dyn::str("Viewer"));
        assert_eq!(changed, ["role", "mode"]);
        assert_eq!(f.value("mode"), Dyn::str("KeyboardOnly"));
        assert!(f.preset_holds("mode"));
    }

    #[test]
    fn set_and_reset_follow_the_control_kind() {
        let mut f = form(&[]);
        f.set("volume", Dyn::str("250"));
        assert_eq!(f.value("volume"), Dyn::Num(100.0));
        f.set("tests", Dyn::str("false"));
        assert_eq!(f.value("tests"), Dyn::Bool(false));
        assert_eq!(f.remembered()["tests"], Stored::Bool(false));
        f.set("parts", Dyn::str("c"));
        assert_eq!(f.value("parts"), Dyn::List(vec![Dyn::str("c")]));
        f.reset("tests");
        assert_eq!(f.value("tests"), Dyn::Bool(true));
        assert!(!f.remembered().contains_key("tests"));
        assert!(f.set_text("volume", "loud").is_err());
        assert!(f.set_text("nope", "1").is_err());
    }

    #[test]
    fn required_pattern_and_must_exist_are_checked() {
        let dir = std::env::temp_dir();
        let config = load_str(
            r#"layout {
    text "name" label="Name" required=#true pattern="[a-z]+" pattern-message="Lower-case letters only"
    folder "out" label="Output" must-exist=#true
    file "preset" label="Preset" filter="json"
    checklist "parts" label="Parts" required=#true {
        option "a"
    }
}"#,
            &dir.join("x.launcher"),
            &LoadOptions::default(),
        );
        assert!(!config.has_errors(), "{:?}", config.diags);
        let mut f = Form::new(
            Arc::new(config.config),
            LoadResults::default(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        assert_eq!(f.check("name").as_deref(), Some("Fill in Name"));
        // A file of another kind than it takes.
        f.set("preset", Dyn::str("a.fbx"));
        assert_eq!(
            f.check("preset").as_deref(),
            Some("Preset: choose a .json file")
        );
        f.set("preset", Dyn::str("a.JSON"));
        assert_eq!(f.check("preset"), None);
        f.set("name", Dyn::str("Ada"));
        assert_eq!(f.check("name").as_deref(), Some("Lower-case letters only"));
        f.set("name", Dyn::str("ada"));
        assert_eq!(f.check("name"), None);
        assert_eq!(f.check("out"), None, "empty and not required");
        f.set("out", Dyn::str("surely-not-here-4242"));
        assert!(f.check("out").unwrap().contains("doesn't exist"));
        f.set("out", Dyn::str("."));
        assert_eq!(f.check("out"), None);
        assert!(f.check("parts").unwrap().contains("at least one"));
    }

    #[test]
    fn a_preset_matching_no_option_is_reported() {
        let config = load_str(
            r#"layout {
    dropdown "a" label="A" {
        preset "Nope"
        option "Yes"
    }
}"#,
            Path::new("x.launcher"),
            &LoadOptions::default(),
        )
        .config;
        let f = Form::new(
            Arc::new(config),
            LoadResults::default(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        assert_eq!(f.problems().len(), 1, "{:?}", f.problems());
        assert!(f.problems()[0].contains("matches no option"));
        assert_eq!(f.value("a"), Dyn::str("Yes"));
    }

    #[test]
    fn a_preset_may_read_a_later_control() {
        let config = load_str(
            r#"layout {
    dropdown "first" label="First" {
        preset "{second}"
        option "x"
        option "y"
    }
    dropdown "second" label="Second" default="y" {
        option "x"
        option "y"
    }
}"#,
            Path::new("x.launcher"),
            &LoadOptions::default(),
        )
        .config;
        let f = Form::new(
            Arc::new(config),
            LoadResults::default(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        assert_eq!(f.value("first"), Dyn::str("y"));
    }
}

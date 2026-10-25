//! The look: egui's stock dark and light styles with the changes in `theme.kdl`, plus the few
//! values egui has no place for (`Extras`).
//!
//! * **The file only changes stock values.** Any field of egui's `Style` can be set, by its serde
//!   name: the stock style is turned into a JSON tree, the file's nodes are applied to it, and the
//!   tree is turned back into a `Style`. `docs/theme-reference.kdl` lists every key with its stock
//!   value (generated; a test keeps it current).
//! * Top-level nodes apply to both themes; `dark { }` and `light { }` apply to one, after them.
//!   `extras { }` holds Launcher's own values, in either place. `fonts { }` (top level only)
//!   nudges font faces by name: egui's `FontTweak`, for faces whose metrics sit text off centre.
//! * A value's shape follows the stock value it replaces: a colour takes `"#rrggbb"` or
//!   `"#rrggbbaa"` (or four numbers, egui's premultiplied form); a struct takes its fields as a
//!   block, as `name=value` pairs, as values in field order, or one number for all of them when
//!   they are all numbers (`corner_radius 4`).
//! * **Built in, file in dev:** the release app uses the copy compiled in. A debug build (and any
//!   build with `LAUNCHER_THEME=path`) reads the file from disk and reloads it when it's saved. A
//!   file with problems changes nothing; the problems are logged and shown over the window.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use egui::{Color32, FontDefinitions, FontTweak, Style, Theme as Mode};
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use launcher_core::config::diag::LineIndex;
use launcher_core::platform::watch::FileWatcher;
use serde_json::{Map, Value};

/// The theme compiled into the binary.
pub const BUILT_IN: &str = include_str!("../../theme.kdl");

/// Launcher's own values: what the UI needs beyond egui's `Style`. Warning, error and muted text
/// are egui's own (`visuals.warn_fg_color`, `visuals.error_fg_color`, `visuals.weak_text_alpha`).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Extras {
    /// What is selected or current: the editor's selection outline.
    pub accent: Color32,
    /// A step or run that succeeded.
    pub success: Color32,
    /// Space between a document's content and the window's edge.
    pub margin: f32,
    /// Space between sections.
    pub gap: f32,
    /// Space between a heading and what it heads, or between lines of a block.
    pub gap_small: f32,
    /// Space between a form's label column and its field column.
    pub label_gap: f32,
    /// Padding inside a form's group box.
    pub group_padding: f32,
    /// Padding inside a callout (the trust prompt).
    pub callout_padding: f32,
    /// Height of a step's row in the run view.
    pub step_row_height: f32,
    /// Size of a step's status mark.
    pub status_icon_size: f32,
    /// Width of the editor's selection outline.
    pub outline_width: f32,
}

impl Extras {
    /// Unset extras: egui's own value where it has a matching one, so they follow the style.
    pub fn defaults(style: &Style) -> Extras {
        let v = &style.visuals;
        Extras {
            accent: v.selection.stroke.color,
            success: if v.dark_mode {
                Color32::from_rgb(102, 204, 128)
            } else {
                Color32::from_rgb(26, 140, 64)
            },
            margin: 16.0,
            gap: 8.0,
            gap_small: 4.0,
            label_gap: style.spacing.item_spacing.x,
            group_padding: 10.0,
            callout_padding: 12.0,
            step_row_height: style.spacing.interact_size.y,
            status_icon_size: 14.0,
            outline_width: 2.0,
        }
    }
}

/// One theme's style and extras.
#[derive(Clone, Debug)]
pub struct Look {
    pub style: Style,
    pub extras: Extras,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub dark: Look,
    pub light: Look,
    /// Tweaks by font name (`ui::fonts` names faces by file); both themes share the fonts.
    pub fonts: BTreeMap<String, FontTweak>,
}

impl Theme {
    /// egui's stock styles and the default extras.
    pub fn stock() -> Theme {
        let look = |m: Mode| {
            let style = m.default_style();
            Look {
                extras: Extras::defaults(&style),
                style,
            }
        };
        Theme {
            dark: look(Mode::Dark),
            light: look(Mode::Light),
            fonts: BTreeMap::new(),
        }
    }

    /// The theme compiled in (a test keeps it free of problems; stock if it had any).
    pub fn built_in() -> Theme {
        Theme::parse(BUILT_IN, "theme.kdl").unwrap_or_else(|problems| {
            for p in problems {
                launcher_core::log_warn!("built-in theme: {p}");
            }
            Theme::stock()
        })
    }

    /// Read a theme file. `name` labels the problems (`name:line:col: message`).
    pub fn parse(text: &str, name: &str) -> Result<Theme, Vec<String>> {
        let mut p = Parser {
            name,
            lines: LineIndex::new(text),
            problems: Vec::new(),
        };
        let doc = match KdlDocument::parse_v2(text) {
            Ok(d) => d,
            Err(e) => {
                for d in &e.diagnostics {
                    let message = d.message.clone().unwrap_or_else(|| "not valid KDL".into());
                    let help = d
                        .help
                        .as_deref()
                        .map(|h| format!(" ({h})"))
                        .unwrap_or_default();
                    p.problem(d.span.offset(), format!("{message}{help}"));
                }
                return Err(p.problems);
            }
        };
        let mut fonts = BTreeMap::new();
        for n in doc.nodes() {
            let name = n.name().value();
            let kids = n.children().map(|c| c.nodes()).unwrap_or_default();
            if matches!(name, "dark" | "light" | "fonts")
                && (!n.entries().is_empty() || n.children().is_none())
            {
                p.problem(
                    n.span().offset(),
                    format!("`{name}` takes a block: {name} {{ ... }}"),
                );
            }
            match name {
                "fonts" => {
                    for f in kids {
                        if let Some(t) = p.font(f) {
                            fonts.insert(f.name().value().to_string(), t);
                        }
                    }
                }
                "dark" | "light" => {
                    for k in kids.iter().filter(|k| k.name().value() == "fonts") {
                        p.problem(
                            k.span().offset(),
                            "fonts are shared by both themes: put `fonts` at the top level".into(),
                        );
                    }
                }
                _ => {}
            }
        }
        let dark = p.look(&doc, Mode::Dark);
        let light = p.look(&doc, Mode::Light);
        match (dark, light) {
            (Some(dark), Some(light)) if p.problems.is_empty() => Ok(Theme { dark, light, fonts }),
            _ => Err(p.problems),
        }
    }

    /// Give egui both styles, and keep the extras where `extras()` finds them.
    pub fn install(&self, ctx: &egui::Context) {
        ctx.set_style_of(Mode::Dark, self.dark.style.clone());
        ctx.set_style_of(Mode::Light, self.light.style.clone());
        ctx.data_mut(|d| {
            d.insert_temp(extras_id(true), self.dark.extras);
            d.insert_temp(extras_id(false), self.light.extras);
        });
    }
}

/// Space of `extras.gap` (between sections).
pub fn gap(ui: &mut egui::Ui) {
    let g = extras(ui).gap;
    ui.add_space(g);
}

/// Space of `extras.gap_small` (under a heading, between lines of a block).
pub fn gap_small(ui: &mut egui::Ui) {
    let g = extras(ui).gap_small;
    ui.add_space(g);
}

fn extras_id(dark: bool) -> egui::Id {
    egui::Id::new(("launcher-theme-extras", dark))
}

/// The extras for the theme `ui` draws with.
pub fn extras(ui: &egui::Ui) -> Extras {
    extras_of(ui.ctx(), ui.visuals().dark_mode)
}

pub fn extras_of(ctx: &egui::Context, dark: bool) -> Extras {
    ctx.data(|d| d.get_temp(extras_id(dark)))
        .unwrap_or_else(|| {
            Extras::defaults(&ctx.style_of(if dark { Mode::Dark } else { Mode::Light }))
        })
}

/// A value written into the tree, kept to find the culprit when the tree won't deserialize.
struct Write {
    path: Vec<String>,
    value: Value,
    offset: usize,
}

struct Parser<'a> {
    name: &'a str,
    lines: LineIndex<'a>,
    problems: Vec<String>,
}

impl Parser<'_> {
    fn problem(&mut self, offset: usize, message: String) {
        let (line, col) = self.lines.line_col(offset);
        let p = format!("{}:{line}:{col}: {message}", self.name);
        // Top-level nodes are read once per theme; say each problem once.
        if !self.problems.contains(&p) {
            self.problems.push(p);
        }
    }

    /// One theme: the top-level nodes, then its own block; style first, then the extras (whose
    /// defaults follow the style).
    fn look(&mut self, doc: &KdlDocument, mode: Mode) -> Option<Look> {
        let own = if mode == Mode::Dark { "dark" } else { "light" };
        let mut nodes: Vec<&KdlNode> = Vec::new();
        for n in doc.nodes() {
            match n.name().value() {
                "dark" | "light" | "fonts" => {}
                _ => nodes.push(n),
            }
        }
        for n in doc.nodes().iter().filter(|n| n.name().value() == own) {
            let kids = n.children().map(|c| c.nodes()).unwrap_or_default();
            nodes.extend(kids.iter().filter(|k| k.name().value() != "fonts"));
        }
        let (extras, style_nodes): (Vec<&KdlNode>, Vec<&KdlNode>) = nodes
            .into_iter()
            .partition(|n| n.name().value() == "extras");

        let stock = to_tree(&mode.default_style());
        let mut tree = stock.clone();
        let mut writes = Vec::new();
        for n in style_nodes {
            self.merge(
                &mut tree,
                n,
                &mut Vec::new(),
                &mut writes,
                Some("extras, fonts, dark, light"),
            );
        }
        let style: Style = self.decode(tree, &stock, &writes, 0)?;

        // `extras` is read like any struct (a block, name=value pairs), under a root holding it.
        let defaults = to_tree(&Extras::defaults(&style));
        let mut root = Value::Object(Map::from_iter([("extras".to_string(), defaults.clone())]));
        let mut writes = Vec::new();
        for n in extras {
            self.merge(&mut root, n, &mut Vec::new(), &mut writes, None);
        }
        let tree = root["extras"].take();
        let extras: Extras = self.decode(tree, &defaults, &writes, 1)?;
        Some(Look { style, extras })
    }

    /// One face's tweak in `fonts { }`: any name (a face this OS doesn't load is skipped when
    /// the fonts are set), fields as for any struct.
    fn font(&mut self, node: &KdlNode) -> Option<FontTweak> {
        let key = node.name().value().to_string();
        let stock = to_tree(&FontTweak::default());
        let mut root = Value::Object(Map::from_iter([(key.clone(), stock.clone())]));
        let mut writes = Vec::new();
        self.merge(
            &mut root,
            node,
            &mut vec!["fonts".into()],
            &mut writes,
            None,
        );
        let tree = root[&key].take();
        self.decode(tree, &stock, &writes, 2)
    }

    /// Turn the tree back into its type. On failure, find the write that breaks it: each one is
    /// tried alone on the stock tree. A write's path is below the tree after `skip` segments.
    fn decode<T: serde::de::DeserializeOwned>(
        &mut self,
        tree: Value,
        stock: &Value,
        writes: &[Write],
        skip: usize,
    ) -> Option<T> {
        match serde_json::from_value(tree) {
            Ok(v) => Some(v),
            Err(e) => {
                let mut found = false;
                for w in writes {
                    let mut t = stock.clone();
                    let mut rel = w.path.iter().skip(skip);
                    if let Some(slot) = rel.try_fold(&mut t, |t, k| t.get_mut(k)) {
                        *slot = w.value.clone();
                    }
                    if let Err(e) = serde_json::from_value::<T>(t) {
                        found = true;
                        self.problem(w.offset, format!("`{}`: {e}", w.path.join(".")));
                    }
                }
                if !found {
                    self.problem(0, e.to_string());
                }
                None
            }
        }
    }

    /// Apply `node` to the object `target`: its name must be one of the object's keys.
    fn merge(
        &mut self,
        target: &mut Value,
        node: &KdlNode,
        path: &mut Vec<String>,
        writes: &mut Vec<Write>,
        also: Option<&str>,
    ) {
        let key = node.name().value();
        let Some(obj) = target.as_object_mut() else {
            return;
        };
        let Some(slot) = obj.get_mut(key) else {
            let place = if path.is_empty() {
                "at the top".to_string()
            } else {
                format!("in `{}`", path.join("."))
            };
            let mut known: Vec<&str> = obj.keys().map(String::as_str).collect();
            if let Some(also) = also {
                known.extend(also.split(", "));
            }
            let hint = match launcher_core::suggest::did_you_mean(key, known.iter().copied()) {
                Some(s) => format!(" (did you mean `{s}`?)"),
                None => format!("; the keys are: {}", known.join(", ")),
            };
            self.problem(
                node.name().span().offset(),
                format!("unknown key `{key}` {place}{hint}"),
            );
            return;
        };
        path.push(key.to_string());
        self.assign(slot, node, path, writes);
        path.pop();
    }

    fn assign(
        &mut self,
        slot: &mut Value,
        node: &KdlNode,
        path: &mut Vec<String>,
        writes: &mut Vec<Write>,
    ) {
        let offset = node.span().offset();
        let write = |writes: &mut Vec<Write>, slot: &mut Value, value: Value, path: &[String]| {
            *slot = value.clone();
            writes.push(Write {
                path: path.to_vec(),
                value,
                offset,
            });
        };
        // An enum (or an unset option) is replaced whole: its shape isn't known from the slot.
        if slot.is_null() || is_enum(slot) {
            match generic(node) {
                Ok(v) => write(writes, slot, v, path),
                Err(e) => self.problem(offset, format!("`{}`: {e}", path.join("."))),
            }
            return;
        }
        let args: Vec<&KdlEntry> = node
            .entries()
            .iter()
            .filter(|e| e.name().is_none())
            .collect();
        let props: Vec<&KdlEntry> = node
            .entries()
            .iter()
            .filter(|e| e.name().is_some())
            .collect();
        let shown = path.join(".");
        if !args.is_empty() && (!props.is_empty() || node.children().is_some()) {
            self.problem(
                offset,
                format!("`{shown}`: give values, or name=value pairs and a block, not both"),
            );
            return;
        }
        if args.is_empty() {
            if props.is_empty() && node.children().is_none() {
                self.problem(offset, format!("`{shown}` needs a value"));
                return;
            }
            if !slot.is_object() {
                self.problem(
                    offset,
                    format!("`{shown}` takes a value, not a block or name=value pairs"),
                );
                return;
            }
            for e in props {
                let (Some(name), Some(obj)) = (e.name(), slot.as_object_mut()) else {
                    continue;
                };
                let k = name.value();
                let Some(field) = obj.get_mut(k) else {
                    let known: Vec<&str> = obj.keys().map(String::as_str).collect();
                    self.problem(
                        e.span().offset(),
                        format!(
                            "`{shown}` has no field `{k}`; its fields are: {}",
                            known.join(", ")
                        ),
                    );
                    continue;
                };
                path.push(k.to_string());
                match scalar(e.value(), field) {
                    Ok(v) => {
                        *field = v.clone();
                        writes.push(Write {
                            path: path.clone(),
                            value: v,
                            offset: e.span().offset(),
                        });
                    }
                    Err(m) => self.problem(e.span().offset(), format!("`{}`: {m}", path.join("."))),
                }
                path.pop();
            }
            for c in node.children().map(|c| c.nodes()).unwrap_or_default() {
                self.merge(slot, c, path, writes, None);
            }
            return;
        }
        let values: Vec<&KdlValue> = args.iter().map(|e| e.value()).collect();
        let result = if let [one] = values.as_slice() {
            scalar(one, slot)
        } else {
            list(&values, slot)
        };
        match result {
            Ok(v) => write(writes, slot, v, path),
            Err(m) => self.problem(offset, format!("`{shown}`: {m}")),
        }
    }
}

fn to_tree<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// egui's colours serialize as four bytes.
fn is_color(v: &Value) -> bool {
    v.as_array()
        .is_some_and(|a| a.len() == 4 && a.iter().all(|x| x.as_u64().is_some_and(|n| n <= 255)))
}

/// A unit enum variant (`"Center"`) or a variant with data (`{"Rect": {...}}`).
fn is_enum(v: &Value) -> bool {
    let upper = |s: &str| s.starts_with(|c: char| c.is_ascii_uppercase());
    match v {
        Value::String(s) => upper(s),
        Value::Object(o) => o.len() == 1 && o.keys().all(|k| upper(k)),
        _ => false,
    }
}

fn color_tree(c: Color32) -> Value {
    Value::from(c.to_array().to_vec())
}

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`, not premultiplied (as in CSS).
pub fn parse_color(s: &str) -> Option<Color32> {
    let hex = s.strip_prefix('#')?;
    let digits: Vec<u8> = hex
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()?;
    let [r, g, b, a] = match digits.as_slice() {
        [r, g, b] => [r * 17, g * 17, b * 17, 255],
        [r, g, b, a] => [r * 17, g * 17, b * 17, a * 17],
        [r1, r2, g1, g2, b1, b2] => [r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, 255],
        [r1, r2, g1, g2, b1, b2, a1, a2] => {
            [r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, a1 * 16 + a2]
        }
        _ => return None,
    };
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

/// How a colour is written: `#rrggbb[aa]` when that reads back exactly, else egui's four
/// premultiplied bytes (additive colours have no CSS form).
pub fn color_text(c: Color32) -> Result<String, [u8; 4]> {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let text = if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    };
    if parse_color(&text) == Some(c) {
        Ok(text)
    } else {
        Err(c.to_array())
    }
}

/// One value for `slot`, shaped like it.
fn scalar(v: &KdlValue, slot: &Value) -> Result<Value, String> {
    if matches!(v, KdlValue::Null) {
        return Ok(Value::Null);
    }
    if is_color(slot) {
        return match v {
            KdlValue::String(s) => parse_color(s)
                .map(color_tree)
                .ok_or_else(|| format!("`{s}` isn't a colour; write \"#rrggbb\" or \"#rrggbbaa\"")),
            _ => Err("takes a colour: \"#rrggbb\" or \"#rrggbbaa\"".into()),
        };
    }
    match slot {
        Value::Number(n) => {
            let x = match v {
                KdlValue::Integer(i) => *i as f64,
                KdlValue::Float(f) => *f,
                _ => return Err("takes a number".into()),
            };
            if n.is_f64() {
                serde_json::Number::from_f64(x)
                    .map(Value::Number)
                    .ok_or_else(|| "takes a finite number".into())
            } else if x.fract() == 0.0 {
                Ok(Value::from(x as i64))
            } else {
                Err("takes a whole number".into())
            }
        }
        Value::Bool(_) => match v {
            KdlValue::Bool(b) => Ok(Value::Bool(*b)),
            _ => Err("takes #true or #false".into()),
        },
        Value::String(_) => match v {
            KdlValue::String(s) => Ok(Value::String(s.clone())),
            _ => Err("takes a string".into()),
        },
        Value::Object(o) if !o.is_empty() && o.values().all(Value::is_number) => {
            let mut out = Map::new();
            for (k, field) in o {
                out.insert(k.clone(), scalar(v, field)?);
            }
            Ok(Value::Object(out))
        }
        Value::Object(o) => Err(format!(
            "takes its fields ({}): as a block, as name=value pairs, or as values in that order",
            o.keys().cloned().collect::<Vec<_>>().join(", ")
        )),
        Value::Array(a) => Err(format!("takes {} values", a.len())),
        Value::Null => Ok(kdl_to_json(v)),
    }
}

/// Several values for `slot`: a colour's four premultiplied bytes, an array's items, or a
/// struct's fields in order.
fn list(vs: &[&KdlValue], slot: &Value) -> Result<Value, String> {
    if is_color(slot) {
        let bytes: Option<Vec<Value>> = vs
            .iter()
            .map(|v| match v {
                KdlValue::Integer(i) if (0..=255).contains(i) => Some(Value::from(*i as u64)),
                _ => None,
            })
            .collect();
        return match bytes {
            Some(b) if b.len() == 4 => Ok(Value::Array(b)),
            _ => Err(
                "takes a colour: \"#rrggbb\", \"#rrggbbaa\", or four premultiplied bytes".into(),
            ),
        };
    }
    match slot {
        Value::Array(a) if a.len() == vs.len() => a
            .iter()
            .zip(vs)
            .map(|(item, v)| scalar(v, item))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(o) if o.len() == vs.len() => {
            let mut out = Map::new();
            for ((k, field), v) in o.iter().zip(vs) {
                out.insert(k.clone(), scalar(v, field).map_err(|m| format!("{k} {m}"))?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(a) => Err(format!("takes {} values, not {}", a.len(), vs.len())),
        Value::Object(o) => Err(format!(
            "takes {} values ({}), not {}",
            o.len(),
            o.keys().cloned().collect::<Vec<_>>().join(", "),
            vs.len()
        )),
        _ => Err(format!("takes one value, not {}", vs.len())),
    }
}

fn kdl_to_json(v: &KdlValue) -> Value {
    match v {
        KdlValue::String(s) => match parse_color(s) {
            Some(c) => color_tree(c),
            None => Value::String(s.clone()),
        },
        KdlValue::Integer(i) => Value::from(*i as i64),
        KdlValue::Float(f) => serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number),
        KdlValue::Bool(b) => Value::Bool(*b),
        KdlValue::Null => Value::Null,
    }
}

/// A node read without a stock value to follow: a block or name=value pairs become an object,
/// one value a value, several an array.
fn generic(node: &KdlNode) -> Result<Value, String> {
    let args: Vec<&KdlValue> = node
        .entries()
        .iter()
        .filter(|e| e.name().is_none())
        .map(|e| e.value())
        .collect();
    let props: Vec<&KdlEntry> = node
        .entries()
        .iter()
        .filter(|e| e.name().is_some())
        .collect();
    match (
        args.as_slice(),
        props.is_empty() && node.children().is_none(),
    ) {
        ([], true) => Err("needs a value".into()),
        ([], false) => {
            let mut out = Map::new();
            for e in props {
                if let Some(n) = e.name() {
                    out.insert(n.value().to_string(), kdl_to_json(e.value()));
                }
            }
            for c in node.children().map(|c| c.nodes()).unwrap_or_default() {
                out.insert(c.name().value().to_string(), generic(c)?);
            }
            Ok(Value::Object(out))
        }
        ([one], true) => Ok(kdl_to_json(one)),
        (many, true) => Ok(Value::Array(many.iter().map(|v| kdl_to_json(v)).collect())),
        _ => Err("give values, or name=value pairs and a block, not both".into()),
    }
}

// The reference.

#[cfg(test)]
/// `docs/theme-reference.kdl`: every key with its stock value, shared keys at the top and each
/// theme's visuals (and anything else that differs) in `dark { }` and `light { }`.
pub fn reference() -> String {
    let tree = |m: Mode| {
        let style = m.default_style();
        let mut t = to_tree(&style);
        if let Some(o) = t.as_object_mut() {
            // Debugging switches, not looks (and they differ between debug and release builds).
            o.remove("debug");
            o.insert("extras".into(), to_tree(&Extras::defaults(&style)));
        }
        t
    };
    let (dark, light) = (tree(Mode::Dark), tree(Mode::Light));
    let (shared, only_dark, only_light) = split(&dark, &light);
    let mut out = String::from(
        "// Every key a theme file takes, with egui's stock value. Generated by `cargo xtask docs`;
// don't edit. Copy a line into crates/launcher/theme.kdl to change it.
//
// Top-level keys apply to both themes; `dark { }` and `light { }` hold each theme's visuals and
// the other keys whose stock values differ, and apply after the top level.
//
// Colours are \"#rrggbb\" or \"#rrggbbaa\"; four plain numbers are egui's premultiplied form
// (some stock colours are additive and have no other).
// A struct takes a block, name=value pairs, its values in field order, or (when its fields are
// all numbers) one number for all of them: `corner_radius 4`.
//
// `extras` are Launcher's own. Unless set, accent follows visuals.selection.stroke, label_gap
// follows spacing.item_spacing.x and step_row_height follows spacing.interact_size.y.

",
    );
    write_nodes(&mut out, &shared, 0);
    out.push_str(
        "
// Nudges per font face (top level only), by the face's file: the built-in Inter-Regular.ttf (text)
// and JetBrainsMonoNL-Regular.ttf (monospace); the text fallbacks segoeui.ttf (Windows), SFNS.ttf
// or Helvetica.ttc (macOS); or egui's own Ubuntu-Light, Hack, NotoEmoji-Regular, emoji-icon-font.
// Faces this OS doesn't load are skipped, so one file serves both. y_offset_factor moves glyphs
// down by that share of the size.
fonts {
",
    );
    write_node(
        &mut out,
        "Inter-Regular.ttf",
        &to_tree(&FontTweak::default()),
        1,
    );
    out.push_str(
        "}
",
    );
    for (name, part) in [("dark", only_dark), ("light", only_light)] {
        out.push_str(&format!("\n{name} {{\n"));
        write_nodes(&mut out, &part, 1);
        out.push_str("}\n");
    }
    out
}

#[cfg(test)]
/// Split two trees into what they share and what each has of its own. A value written on one
/// line (a colour, a stroke, a margin) moves as a whole, and so does `visuals`: each theme's
/// visuals read best in one piece.
fn split(a: &Value, b: &Value) -> (Map<String, Value>, Map<String, Value>, Map<String, Value>) {
    let (mut shared, mut only_a, mut only_b) = (Map::new(), Map::new(), Map::new());
    let (Some(ao), Some(bo)) = (a.as_object(), b.as_object()) else {
        return (shared, only_a, only_b);
    };
    for (k, av) in ao {
        let Some(bv) = bo.get(k) else { continue };
        if av == bv {
            shared.insert(k.clone(), av.clone());
        } else if k != "visuals"
            && av.is_object()
            && bv.is_object()
            && inline(av).is_none()
            && inline(bv).is_none()
        {
            let (s, x, y) = split(av, bv);
            for (map, part) in [(&mut shared, s), (&mut only_a, x), (&mut only_b, y)] {
                if !part.is_empty() {
                    map.insert(k.clone(), Value::Object(part));
                }
            }
        } else {
            only_a.insert(k.clone(), av.clone());
            only_b.insert(k.clone(), bv.clone());
        }
    }
    (shared, only_a, only_b)
}

#[cfg(test)]
fn write_nodes(out: &mut String, map: &Map<String, Value>, depth: usize) {
    for (k, v) in map {
        write_node(out, k, v, depth);
    }
}

#[cfg(test)]
fn write_node(out: &mut String, key: &str, v: &Value, depth: usize) {
    let pad = "    ".repeat(depth);
    if let Some(args) = inline(v) {
        out.push_str(&format!("{pad}{key} {args}\n"));
    } else if let Value::Object(o) = v {
        out.push_str(&format!("{pad}{key} {{\n"));
        write_nodes(out, o, depth + 1);
        out.push_str(&format!("{pad}}}\n"));
    }
}

#[cfg(test)]
/// A value's one-line form, if it has one: a value, a colour, an array of values, or a short
/// struct of values (one number when they are all the same number).
fn inline(v: &Value) -> Option<String> {
    if let Some(s) = value_text(v) {
        return Some(s);
    }
    match v {
        // An empty list has no form here (a font's variation axes): left out.
        Value::Array(a) if a.is_empty() => None,
        Value::Array(a) => a
            .iter()
            .map(value_text)
            .collect::<Option<Vec<_>>>()
            .map(|x| x.join(" ")),
        Value::Object(o) if !o.is_empty() && !is_enum(v) => {
            let mut nums = o.values().filter(|x| x.is_number() && !is_color(x));
            if let Some(first) = nums.next() {
                if o.len() > 1 && o.values().all(|x| x == first) {
                    return value_text(first);
                }
            }
            let line = o
                .iter()
                .map(|(k, x)| value_text(x).map(|t| format!("{k}={t}")))
                .collect::<Option<Vec<_>>>()?
                .join(" ");
            (line.len() <= 72).then_some(line)
        }
        _ => None,
    }
}

#[cfg(test)]
/// A single value's text: a colour, number, string, boolean or null.
fn value_text(v: &Value) -> Option<String> {
    if is_color(v) {
        let b: Vec<u8> = v
            .as_array()?
            .iter()
            .filter_map(|x| x.as_u64().map(|n| n as u8))
            .collect();
        return match color_text(Color32::from_rgba_premultiplied(b[0], b[1], b[2], b[3])) {
            Ok(text) => Some(format!("\"{text}\"")),
            Err(_) => None,
        };
    }
    match v {
        Value::Number(n) if n.is_f64() => Some(format!("{:?}", n.as_f64()? as f32)),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(format!("{s:?}")),
        Value::Bool(b) => Some(if *b { "#true" } else { "#false" }.into()),
        Value::Null => Some("#null".into()),
        _ => None,
    }
}

// Hot reload.

/// What the theme came from and how its last load went (the gallery shows it).
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// The file on disk, or None for the built-in copy.
    pub file: Option<PathBuf>,
    /// Problems in the file's current text (the theme before it stays).
    pub problems: Vec<String>,
    /// Loads that changed the theme.
    pub loads: u64,
    /// The font faces text is drawn with (their names in `fonts { }`).
    pub faces: Vec<String>,
}

fn status_id() -> egui::Id {
    egui::Id::new("launcher-theme-status")
}

pub fn status(ctx: &egui::Context) -> Status {
    ctx.data(|d| d.get_temp(status_id())).unwrap_or_default()
}

/// The file to read instead of the built-in copy: `LAUNCHER_THEME`, else (debug builds) the
/// source file.
fn file_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LAUNCHER_THEME") {
        return Some(launcher_core::paths::absolute(Path::new(&p)));
    }
    if cfg!(debug_assertions) {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("theme.kdl");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// The installed theme, reloaded from its file when that changes.
pub struct Live {
    file: Option<PathBuf>,
    changed: Arc<AtomicBool>,
    _watcher: Option<FileWatcher>,
    /// The text last read, so a save that changed nothing changes nothing.
    text: Option<String>,
    /// The fonts as loaded, before the theme's tweaks.
    fonts: Option<FontDefinitions>,
    /// The tweaks last given to egui (None: the fonts aren't set yet).
    tweaks: Option<BTreeMap<String, FontTweak>>,
}

impl Live {
    /// Install the theme and the fonts now: the file's theme if there is one (and it has no
    /// problems), else the built-in copy.
    pub fn start(ctx: &egui::Context, fonts: Option<FontDefinitions>) -> Live {
        let file = file_path();
        let changed = Arc::new(AtomicBool::new(false));
        let watcher = file.as_deref().and_then(|f| {
            let flag = changed.clone();
            let c = ctx.clone();
            FileWatcher::start(f, move || {
                flag.store(true, Ordering::SeqCst);
                c.request_repaint();
            })
            .map_err(|e| launcher_core::log_warn!("theme: {e}"))
            .ok()
        });
        let mut live = Live {
            file,
            changed,
            _watcher: watcher,
            text: None,
            fonts,
            tweaks: None,
        };
        let faces = live.fonts.as_ref().map(|f| {
            [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
                .iter()
                .filter_map(|family| f.families.get(family)?.first().cloned())
                .collect()
        });
        ctx.data_mut(|d| {
            d.insert_temp(
                status_id(),
                Status {
                    faces: faces.unwrap_or_default(),
                    ..Status::default()
                },
            )
        });
        if live.file.is_none() || !live.load(ctx) {
            live.apply(ctx, &Theme::built_in());
        }
        live
    }

    /// Install `theme`; set the fonts again only when its tweaks changed (that redraws every
    /// glyph). A face this OS doesn't load is skipped, so one file serves Windows and macOS.
    fn apply(&mut self, ctx: &egui::Context, theme: &Theme) {
        theme.install(ctx);
        if self.tweaks.as_ref() == Some(&theme.fonts) {
            return;
        }
        if let Some(base) = &self.fonts {
            let mut defs = base.clone();
            for (name, tweak) in &theme.fonts {
                if let Some(data) = defs.font_data.get_mut(name) {
                    Arc::make_mut(data).tweak = tweak.clone();
                }
            }
            ctx.set_fonts(defs);
        }
        self.tweaks = Some(theme.fonts.clone());
    }

    /// Before a pass: reload the file if it changed.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if self.changed.swap(false, Ordering::SeqCst) {
            self.load(ctx);
        }
    }

    /// Read the file and install it. Returns whether a theme from the file is installed.
    fn load(&mut self, ctx: &egui::Context) -> bool {
        let Some(file) = self.file.clone() else {
            return false;
        };
        let mut status = status(ctx);
        status.file = Some(file.clone());
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                status.problems = vec![format!("{}: can't read the theme: {e}", file.display())];
                ctx.data_mut(|d| d.insert_temp(status_id(), status));
                return false;
            }
        };
        if self.text.as_deref() == Some(text.as_str()) {
            return status.problems.is_empty();
        }
        self.text = Some(text.clone());
        // Problems name the file only; the gallery's status shows where it is.
        let name = file.file_name().map_or_else(
            || file.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let ok = match Theme::parse(&text, &name) {
            Ok(theme) => {
                self.apply(ctx, &theme);
                status.problems.clear();
                status.loads += 1;
                launcher_core::log_info!("theme loaded from {}", file.display());
                true
            }
            Err(problems) => {
                for p in &problems {
                    launcher_core::log_warn!("theme: {p}");
                }
                status.problems = problems;
                false
            }
        };
        ctx.data_mut(|d| d.insert_temp(status_id(), status));
        ctx.request_repaint();
        ok
    }
}

/// The theme file's problems over the window, while it has any.
pub fn show_problems(ctx: &egui::Context) {
    let status = status(ctx);
    if status.problems.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("theme-problems"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(8.0, -8.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(ctx.content_rect().width() - 40.0);
                ui.label(
                    egui::RichText::new("The theme file has problems; the last good theme stays:")
                        .strong(),
                );
                for p in &status.problems {
                    ui.label(
                        egui::RichText::new(p)
                            .monospace()
                            .color(ui.visuals().error_fg_color),
                    );
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problems(text: &str) -> Vec<String> {
        Theme::parse(text, "t.kdl").err().unwrap_or_default()
    }

    #[test]
    fn the_built_in_theme_has_no_problems() {
        if let Err(p) = Theme::parse(BUILT_IN, "theme.kdl") {
            panic!("{}", p.join("\n"));
        }
    }

    #[test]
    fn an_empty_file_is_stock() {
        let t = Theme::parse("", "t.kdl").unwrap();
        let s = Theme::stock();
        assert_eq!(to_tree(&t.dark.style), to_tree(&s.dark.style));
        assert_eq!(to_tree(&t.light.style), to_tree(&s.light.style));
        assert_eq!(t.dark.extras, s.dark.extras);
    }

    #[test]
    fn values_follow_the_stock_shape() {
        let t = Theme::parse(
            r##"
            spacing {
                item_spacing 10 6
                button_padding x=12
                window_margin 9
            }
            text_styles { Body size=15 }
            visuals {
                widgets { inactive { corner_radius 4; bg_stroke 1.5 "#ff0000" } }
                override_text_color "#102030"
                handle_shape "Circle"
            }
            dark { visuals { panel_fill "#112233" } }
            light { extras { accent "#00000080"; margin 20 } }
            extras { margin 12 }
            "##,
            "t.kdl",
        )
        .unwrap();
        let d = &t.dark.style;
        assert_eq!(d.spacing.item_spacing, egui::vec2(10.0, 6.0));
        assert_eq!(d.spacing.button_padding.x, 12.0);
        assert_eq!(d.spacing.window_margin, egui::Margin::same(9));
        assert_eq!(d.text_styles[&egui::TextStyle::Body].size, 15.0);
        let w = &d.visuals.widgets.inactive;
        assert_eq!(w.corner_radius, egui::CornerRadius::same(4));
        assert_eq!(
            w.bg_stroke,
            egui::Stroke::new(1.5, Color32::from_rgb(255, 0, 0))
        );
        assert_eq!(
            d.visuals.override_text_color,
            Some(Color32::from_rgb(16, 32, 48))
        );
        assert_eq!(d.visuals.handle_shape, egui::style::HandleShape::Circle);
        assert_eq!(d.visuals.panel_fill, Color32::from_rgb(17, 34, 51));
        assert_eq!(
            t.light.style.visuals.panel_fill,
            Mode::Light.default_visuals().panel_fill
        );
        assert_eq!(t.dark.extras.margin, 12.0);
        assert_eq!(t.light.extras.margin, 20.0);
        assert_eq!(
            t.light.extras.accent,
            Color32::from_rgba_unmultiplied(0, 0, 0, 128)
        );
        // Unset extras follow the style.
        assert_eq!(t.dark.extras.label_gap, 10.0);
    }

    #[test]
    fn fonts_take_tweaks_by_face() {
        let t = Theme::parse(
            "fonts {
    segoeui.ttf y_offset_factor=-0.1
    JetBrainsMonoNL-Regular.ttf { scale 1.1 }
}",
            "t.kdl",
        )
        .unwrap();
        assert_eq!(t.fonts["segoeui.ttf"].y_offset_factor, -0.1);
        let mono = &t.fonts["JetBrainsMonoNL-Regular.ttf"];
        assert_eq!(mono.scale, 1.1);
        assert_eq!(mono.y_offset_factor, 0.0);

        let p = problems("dark { fonts { a.ttf scale=2 } }");
        assert!(p[0].contains("put `fonts` at the top level"), "{p:?}");
        let p = problems("fonts { a.ttf y_ofset=1 }");
        assert!(p[0].contains("has no field `y_ofset`"), "{p:?}");
    }

    #[test]
    fn problems_name_the_line_and_the_fix() {
        let p = problems("visuals {\n    pannel_fill \"#000\"\n}\n");
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(
            p[0].starts_with("t.kdl:2:5: unknown key `pannel_fill` in `visuals`"),
            "{p:?}"
        );
        assert!(p[0].contains("did you mean `panel_fill`"), "{p:?}");

        let p = problems("visuals { panel_fill \"red\" }");
        assert!(p[0].contains("isn't a colour"), "{p:?}");

        // A type error only serde sees is blamed on its line.
        let p = problems("\n\nvisuals { window_corner_radius 300 }");
        assert!(
            p[0].starts_with("t.kdl:3:11: `visuals.window_corner_radius`"),
            "{p:?}"
        );

        let p = problems("text_styles { Body 14 }");
        assert!(p[0].contains("size, family"), "{p:?}");

        let p = problems("dark \"x\"");
        assert!(p[0].contains("takes a block"), "{p:?}");

        let p = problems("spacing {");
        assert!(!p.is_empty());
    }

    #[test]
    fn colours_read_and_write_back() {
        assert_eq!(parse_color("#fff"), Some(Color32::WHITE));
        assert_eq!(
            parse_color("#00000060"),
            Some(Color32::from_black_alpha(0x60))
        );
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(color_text(Color32::from_rgb(1, 2, 3)), Ok("#010203".into()));
        assert!(color_text(Color32::from_additive_luminance(5)).is_err());
    }

    #[test]
    fn the_reference_reads_back_as_stock() {
        let t = Theme::parse(&reference(), "reference").unwrap_or_else(|p| {
            panic!(
                "{}",
                p.join(
                    "
"
                )
            )
        });
        let s = Theme::stock();
        let tree = |style: &Style| {
            let mut t = to_tree(style);
            if let Some(o) = t.as_object_mut() {
                o.remove("debug");
            }
            t
        };
        assert_eq!(tree(&t.dark.style), tree(&s.dark.style));
        assert_eq!(tree(&t.light.style), tree(&s.light.style));
        assert_eq!(t.dark.extras, s.dark.extras);
        assert_eq!(t.light.extras, s.light.extras);
    }

    /// `docs/theme-reference.kdl` is current (`UPDATE_GOLDENS=1` or `cargo xtask docs` rewrites it).
    #[test]
    fn the_reference_doc_is_current() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/theme-reference.kdl");
        let want = reference();
        if std::env::var_os("UPDATE_GOLDENS").is_some() {
            std::fs::write(&path, &want).unwrap();
            return;
        }
        let have = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            have == want,
            "docs/theme-reference.kdl is out of date; run `cargo xtask docs`"
        );
    }
}

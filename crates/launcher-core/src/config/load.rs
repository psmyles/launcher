//! `.launcher` text → [`Config`] + [`Diagnostic`]s.
//!
//! The loader is error-tolerant: it never stops at the first problem, and whatever it could read is
//! still returned, so the window can show the parts that are fine next to the list of problems.
//! Errors keep actions from running; warnings never do. The generic checks (placement, arguments,
//! properties, types, `(lua)`, OS availability) come from the catalog; cross-references are checked
//! afterwards by [`super::validate`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

use super::catalog::{self, Category, Children, Os, Param, Spec, Ty};
use super::diag::{Diagnostic, LineIndex, Severity};
use super::header::{self, Gate};
use super::model::*;
use super::template::{self, Template};
use crate::addons::Library;
use crate::suggest::did_you_mean;

/// What the file is loaded for.
#[derive(Clone, Copy, Debug)]
pub struct LoadOptions {
    /// The running Launcher's version (for `requires`).
    pub runtime: Version,
    /// The OS the file will run on (for `os=` and OS-only nodes).
    pub os: Os,
}

impl Default for LoadOptions {
    fn default() -> Self {
        LoadOptions {
            runtime: Version(u32::MAX, 0, 0),
            os: Os::current(),
        }
    }
}

pub struct Loaded {
    pub config: Config,
    pub diags: Vec<Diagnostic>,
    /// The parsed document (None when the text is not valid KDL, or the header stopped the load).
    pub doc: Option<KdlDocument>,
    /// Set when the header says the file isn't for this Launcher or OS; nothing else is loaded.
    pub gate: Option<Gate>,
}

impl Loaded {
    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(Diagnostic::is_error)
    }

    pub fn errors(&self) -> usize {
        self.diags.iter().filter(|d| d.is_error()).count()
    }

    pub fn warnings(&self) -> usize {
        self.diags.len() - self.errors()
    }
}

/// Load a file from text. `path` is where it lives (for relative paths and messages).
pub fn load_str(src: &str, path: &Path, opts: &LoadOptions) -> Loaded {
    let lines = LineIndex::new(src);
    let empty = Config::empty(path.to_path_buf());
    if let Some(sniffed) = header::sniff(src) {
        if let Some(g) = header::gate(&sniffed, opts.runtime, opts.os) {
            let (line, col) = lines.line_col(sniffed.offset);
            let d = Diagnostic {
                severity: Severity::Error,
                code: match g {
                    Gate::TooNew { .. } => "requires-newer",
                    Gate::WrongOs { .. } => "wrong-os",
                },
                message: g.message(),
                help: Some(g.help()),
                offset: sniffed.offset,
                len: sniffed.len,
                line,
                col,
                path: None,
            };
            return Loaded {
                config: empty,
                diags: vec![d],
                doc: None,
                gate: Some(g),
            };
        }
    }
    let doc = match KdlDocument::parse_v2(src) {
        Ok(d) => d,
        Err(e) => {
            let mut diags: Vec<Diagnostic> = e
                .diagnostics
                .iter()
                .map(|d| {
                    let offset = d.span.offset();
                    let (line, col) = lines.line_col(offset);
                    let message = d
                        .message
                        .clone()
                        .unwrap_or_else(|| "the file is not valid KDL".into());
                    let help = syntax_hint(src, offset).or_else(|| d.help.clone());
                    Diagnostic {
                        severity: Severity::Error,
                        code: "syntax",
                        message,
                        help,
                        offset,
                        len: d.span.len(),
                        line,
                        col,
                        path: None,
                    }
                })
                .collect();
            diags.sort_by_key(|d| d.offset);
            // The KDL parser reports one root cause several ways; keep the first per line.
            diags.dedup_by_key(|d| d.line);
            return Loaded {
                config: empty,
                diags,
                doc: None,
                gate: None,
            };
        }
    };
    // A format-2 tool is checked in its own words, then translated into the kinds below.
    let translated =
        super::format2::is_tool(&doc).then(|| super::format2::translate(src, &doc, opts));
    let (to_load, library) = match &translated {
        Some(t) => (&t.doc, t.library.clone()),
        None => (&doc, Library::current()),
    };
    let mut ld = Ld {
        src,
        lines,
        diags: Vec::new(),
        controls: Vec::new(),
        target: opts.os,
        library: Some(Arc::new(library)),
        tool: translated.is_some(),
    };
    let mut config = empty;
    ld.top(to_load, &mut config);
    config.controls = std::mem::take(&mut ld.controls);
    if let Some(t) = &translated {
        config.format = 2;
        config.carried = t.carried.clone();
        ld.diags.extend(t.diags.iter().cloned());
    }
    let mut diags = ld.diags;
    super::validate::validate(&config, &LineIndex::new(src), &mut diags);
    diags.sort_by(|a, b| a.offset.cmp(&b.offset).then(b.severity.cmp(&a.severity)));
    Loaded {
        config,
        diags,
        doc: Some(doc),
        gate: None,
    }
}

/// Hints for the syntax mistakes people actually make in KDL v2 (LESSONS L1.4).
fn syntax_hint(src: &str, offset: usize) -> Option<String> {
    let offset = offset.min(src.len());
    let line_start = src[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line_end = src[offset..].find('\n').map_or(src.len(), |i| offset + i);
    let line = &src[line_start..line_end];
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == '=')
        .collect();
    if line.contains("} else") {
        return Some("`else` isn't a thing here: use `if=` on a step, or Lua".into());
    }
    if words
        .iter()
        .any(|w| matches!(*w, "true" | "false" | "null"))
    {
        return Some("write yes/no as #true / #false (and nothing as #null)".into());
    }
    if src[offset..line_end].contains('\\') || line.contains('\\') {
        return Some(
            r##"inside "..." a backslash starts an escape; write Windows paths as #"C:\path"# (a raw string) or double each backslash"##.into(),
        );
    }
    None
}

/// The loader's typed reads, for files that hold steps but aren't tools (add-ons): the same
/// steps, values and Lua, checked the same way.
pub(crate) struct Reader<'a> {
    ld: Ld<'a>,
}

impl<'a> Reader<'a> {
    pub fn new(src: &'a str, opts: &LoadOptions) -> Reader<'a> {
        Reader {
            ld: Ld {
                src,
                lines: LineIndex::new(src),
                diags: Vec::new(),
                controls: Vec::new(),
                target: opts.os,
                library: None,
                tool: false,
            },
        }
    }

    /// The steps in a block (a `does { }`).
    pub fn steps(&mut self, doc: &'a KdlDocument, path: NodePath) -> Vec<Step> {
        self.ld.steps(doc, &path, false)
    }

    /// A template, or `(lua)"expr"`.
    pub fn val(&self, e: &KdlEntry) -> Option<Val> {
        self.ld.val(e)
    }

    /// Lua source from a string entry, with its place in the file.
    pub fn lua(&self, e: &KdlEntry) -> Option<LuaSrc> {
        self.ld.lua_src(e)
    }

    pub fn error(&mut self, code: &'static str, msg: String, help: Option<String>, span: Span) {
        self.ld.err_at(code, msg, help, span, &Vec::new());
    }

    pub fn node_span(n: &KdlNode) -> Span {
        Ld::node_span(n)
    }

    pub fn entry_span(e: &KdlEntry) -> Span {
        Ld::entry_span(e)
    }

    pub fn finish(self) -> Vec<Diagnostic> {
        self.ld.diags
    }
}

struct Ld<'a> {
    src: &'a str,
    lines: LineIndex<'a>,
    diags: Vec<Diagnostic>,
    controls: Vec<Control>,
    target: Os,
    /// The add-ons `use` steps come from (None while reading an add-on itself).
    library: Option<Arc<Library>>,
    /// Reading a tool (format 2, translated): its own checks cover what this would warn about in
    /// first-format words.
    tool: bool,
}

/// A node checked against its spec, ready for typed reads.
struct Nv<'a> {
    node: &'a KdlNode,
    spec: &'static Spec,
    path: NodePath,
    args: Vec<&'a KdlEntry>,
    props: HashMap<&'a str, &'a KdlEntry>,
}

impl Nv<'_> {
    fn node_ref(&self) -> NodeRef {
        NodeRef {
            path: self.path.clone(),
            span: Span {
                offset: self.node.span().offset(),
                len: self.node.span().len(),
            },
        }
    }

    fn arg(&self, i: usize) -> Option<&KdlEntry> {
        self.args.get(i).copied()
    }

    fn prop(&self, name: &str) -> Option<&KdlEntry> {
        self.props.get(name).copied()
    }

    fn name(&self) -> &'static str {
        self.spec.name
    }
}

impl<'a> Ld<'a> {
    fn push(
        &mut self,
        severity: Severity,
        code: &'static str,
        message: String,
        help: Option<String>,
        span: Span,
        path: &NodePath,
    ) {
        let (line, col) = self.lines.line_col(span.offset);
        self.diags.push(Diagnostic {
            severity,
            code,
            message,
            help,
            offset: span.offset,
            len: span.len,
            line,
            col,
            path: Some(path.clone()),
        });
    }

    fn err_at(
        &mut self,
        code: &'static str,
        msg: String,
        help: Option<String>,
        span: Span,
        path: &NodePath,
    ) {
        self.push(Severity::Error, code, msg, help, span, path);
    }

    fn warn_at(
        &mut self,
        code: &'static str,
        msg: String,
        help: Option<String>,
        span: Span,
        path: &NodePath,
    ) {
        self.push(Severity::Warning, code, msg, help, span, path);
    }

    fn node_span(n: &KdlNode) -> Span {
        Span {
            offset: n.span().offset(),
            len: n.name().span().len().max(1),
        }
    }

    fn entry_span(e: &KdlEntry) -> Span {
        Span {
            offset: e.span().offset(),
            len: e.span().len().max(1),
        }
    }

    /// Where an entry's value starts (the span covers `name=value`, LESSONS L1.3).
    fn value_span(e: &KdlEntry) -> Span {
        let whole = e.span();
        let repr_len = e.format().map(|f| f.value_repr.len()).unwrap_or(0);
        let end = whole.offset() + whole.len();
        let offset = end.saturating_sub(repr_len).max(whole.offset());
        Span {
            offset,
            len: end - offset,
        }
    }

    /// Check `node` against the catalog in a block that holds `allowed`. Reports and returns
    /// None for unknown and misplaced nodes.
    fn view(
        &mut self,
        node: &'a KdlNode,
        path: NodePath,
        allowed: Children,
        parent: &str,
    ) -> Option<Nv<'a>> {
        let name = node.name().value();
        let here = catalog::allowed(allowed);
        let Some(spec) = catalog::get(name) else {
            let hint = did_you_mean(name, here.iter().copied())
                .or_else(|| did_you_mean(name, catalog::SPECS.iter().map(|s| s.name)))
                .map(|s| format!("did you mean `{s}`?"));
            let listing = (!here.is_empty() && here.len() <= 12)
                .then(|| format!("allowed {parent}: {}", here.join(", ")));
            self.err_at(
                "unknown-kind",
                format!("unknown node `{name}`"),
                hint.or(listing),
                Self::node_span(node),
                &path,
            );
            return None;
        };
        let placed = match allowed {
            Children::Only(k) => k.contains(&name),
            Children::Layout => matches!(spec.cat, Category::Layout | Category::Control),
            Children::Steps => spec.cat == Category::Step,
            Children::None | Children::Bag => false,
        } || (allowed == Children::None && spec.cat == Category::Top);
        if !placed {
            let wher = match spec.cat {
                Category::Top => "at the top of the file".to_string(),
                Category::Layout | Category::Control => {
                    "inside `layout` (or a container)".to_string()
                }
                Category::Step => "inside an `action` (or `on-change`)".to_string(),
                Category::Part => {
                    let parents: Vec<&str> = catalog::SPECS
                        .iter()
                        .filter(|s| matches!(s.children, Children::Only(k) if k.contains(&name)))
                        .map(|s| s.name)
                        .collect();
                    format!("inside {}", crate::suggest::one_of(&parents))
                }
            };
            self.err_at(
                "misplaced",
                format!("`{name}` can't go {parent}"),
                Some(format!("`{name}` belongs {wher}")),
                Self::node_span(node),
                &path,
            );
            return None;
        }
        if !spec.os.allows(self.target) {
            self.err_at(
                "os-only",
                format!(
                    "`{name}` only works on {}, and this file is for {}",
                    header::os_title(spec.os),
                    header::os_title(self.target)
                ),
                None,
                Self::node_span(node),
                &path,
            );
        }
        if let Some(ty) = node.ty() {
            self.err_at(
                "annotation",
                format!("a node can't have a type annotation (`({})`)", ty.value()),
                Some("annotations like (lua) go on values: name=(lua)\"...\"".into()),
                Self::node_span(node),
                &path,
            );
        }

        let mut args = Vec::new();
        let mut props: HashMap<&'a str, &'a KdlEntry> = HashMap::new();
        for e in node.entries() {
            match e.name() {
                None => args.push(e),
                Some(k) => {
                    let key = k.value();
                    match spec.prop(key) {
                        None => {
                            let names: Vec<&str> = spec.all_props().map(|p| p.name).collect();
                            let hint = did_you_mean(key, names.iter().copied())
                                .map(|s| format!("did you mean `{s}`?"))
                                .or_else(|| {
                                    (!names.is_empty())
                                        .then(|| format!("`{name}` takes: {}", names.join(", ")))
                                });
                            self.err_at(
                                "unknown-property",
                                format!("`{name}` has no property `{key}`"),
                                hint,
                                Self::entry_span(e),
                                &path,
                            );
                        }
                        Some(pr) => {
                            if props.insert(key, e).is_some() {
                                self.warn_at(
                                    "duplicate-property",
                                    format!("`{key}` is set twice; the last one wins"),
                                    None,
                                    Self::entry_span(e),
                                    &path,
                                );
                            }
                            if !pr.os.allows(self.target) {
                                self.err_at(
                                    "os-only",
                                    format!(
                                        "`{key}` only works on {}, and this file is for {}",
                                        header::os_title(pr.os),
                                        header::os_title(self.target)
                                    ),
                                    None,
                                    Self::entry_span(e),
                                    &path,
                                );
                            }
                        }
                    }
                }
            }
        }
        for (i, a) in spec.args.iter().enumerate() {
            if a.required && args.get(i).is_none() {
                self.err_at(
                    "missing-argument",
                    format!("`{name}` needs its <{}> argument", a.name),
                    Some(a.doc.to_string()),
                    Self::node_span(node),
                    &path,
                );
            }
        }
        if spec.rest.is_none() && args.len() > spec.args.len() {
            let extra = args[spec.args.len()];
            let takes = match spec.args.len() {
                0 => "no arguments".to_string(),
                1 => "one argument".to_string(),
                n => format!("{n} arguments"),
            };
            let named_hint = spec
                .args
                .is_empty()
                .then(|| "properties are written name=value".to_string());
            self.err_at(
                "extra-argument",
                format!("`{name}` takes {takes}"),
                named_hint,
                Self::entry_span(extra),
                &path,
            );
        }
        for pr in spec.all_props().filter(|p| p.required) {
            if !props.contains_key(pr.name) {
                self.err_at(
                    "missing-property",
                    format!("`{name}` needs `{}=`", pr.name),
                    Some(pr.doc.to_string()),
                    Self::node_span(node),
                    &path,
                );
            }
        }
        // Every value, checked against its parameter's type.
        for (i, e) in args.iter().enumerate() {
            if let Some(pr) = spec.args.get(i).or(spec.rest.as_ref()) {
                self.check_value(e, pr, &path);
            }
        }
        for (key, e) in &props {
            if let Some(pr) = spec.prop(key) {
                self.check_value(e, pr, &path);
            }
        }
        if node.children().is_some() && spec.children == Children::None {
            self.err_at(
                "unexpected-block",
                format!("`{name}` doesn't take a {{ }} block"),
                None,
                Self::node_span(node),
                &path,
            );
        }
        Some(Nv {
            node,
            spec,
            path,
            args,
            props,
        })
    }

    /// Type-check one value; typed reads later assume this passed (and quietly skip otherwise).
    fn check_value(&mut self, e: &KdlEntry, pr: &Param, path: &NodePath) {
        let span = Self::value_span(e);
        if let Some(ann) = e.ty() {
            let a = ann.value();
            if a != "lua" {
                self.err_at(
                    "annotation",
                    format!("unknown annotation `({a})`"),
                    Some("the only annotation is (lua), for a Lua expression".into()),
                    span,
                    path,
                );
                return;
            }
            if !(pr.ty.accepts_lua() || matches!(pr.ty, Ty::Lua | Ty::LuaChunk)) {
                self.err_at(
                    "lua-not-allowed",
                    format!("`{}` can't be (lua)", pr.name),
                    Some(format!("it takes a {}", catalog::ty_name(pr.ty))),
                    span,
                    path,
                );
                return;
            }
            if !matches!(e.value(), KdlValue::String(_)) {
                self.err_at(
                    "wrong-type",
                    "a (lua) value is Lua code in a string: (lua)\"...\"".into(),
                    None,
                    span,
                    path,
                );
            }
            return;
        }
        let v = e.value();
        let wrong = |expected: &str| format!("`{}` should be {expected}", pr.name);
        match pr.ty {
            Ty::LuaValue => {
                self.err_at(
                    "wrong-type",
                    format!(
                        "`{}` is a Lua expression: write {}=(lua)\"...\"",
                        pr.name, pr.name
                    ),
                    None,
                    span,
                    path,
                );
            }
            Ty::Text
            | Ty::Template
            | Ty::Lua
            | Ty::LuaChunk
            | Ty::Id
            | Ty::ControlRef
            | Ty::ActionRef
            | Ty::BagRef
            | Ty::RunRef
            | Ty::Version => {
                let Some(s) = v.as_string() else {
                    let msg = wrong("text (in quotes)");
                    self.err_at("wrong-type", msg, None, span, path);
                    return;
                };
                match pr.ty {
                    Ty::Template => {
                        if let Err(te) = Template::parse(s) {
                            let offset = self.string_offset(e, te.offset);
                            self.err_at(
                                "bad-template",
                                te.message,
                                None,
                                Span { offset, len: 1 },
                                path,
                            );
                        }
                        self.lint_text(e, s, path);
                    }
                    Ty::Text => self.lint_text(e, s, path),
                    Ty::Id if !template::is_name(s) => {
                        self.err_at(
                            "bad-name",
                            format!("`{s}` can't be a name"),
                            Some(
                                "names start with a letter or _, then letters, digits, _ or -"
                                    .into(),
                            ),
                            span,
                            path,
                        );
                    }
                    Ty::Version if Version::parse(s).is_none() => {
                        self.err_at(
                            "bad-version",
                            format!("`{s}` is not a version"),
                            Some("write it like \"1.2\"".into()),
                            span,
                            path,
                        );
                    }
                    _ => {}
                }
            }
            Ty::Width => {
                let ok = match v {
                    KdlValue::Integer(_) | KdlValue::Float(_) => true,
                    KdlValue::String(s) => {
                        matches!(s.as_str(), "fill" | "auto") || s.parse::<f32>().is_ok()
                    }
                    _ => false,
                };
                if !ok {
                    self.err_at(
                        "wrong-type",
                        wrong("a number of pixels, \"fill\" or \"auto\""),
                        None,
                        span,
                        path,
                    );
                }
            }
            Ty::Int => {
                if !matches!(v, KdlValue::Integer(_)) {
                    self.err_at("wrong-type", wrong("a whole number"), None, span, path);
                }
            }
            Ty::Number => {
                if !matches!(v, KdlValue::Integer(_) | KdlValue::Float(_)) {
                    self.err_at("wrong-type", wrong("a number"), None, span, path);
                }
            }
            Ty::Bool => {
                if !matches!(v, KdlValue::Bool(_)) {
                    let hint = v
                        .as_string()
                        .filter(|s| matches!(*s, "true" | "false"))
                        .map(|s| format!("write #{s}, without quotes"));
                    self.err_at("wrong-type", wrong("#true or #false"), hint, span, path);
                }
            }
            Ty::Enum(words) => match v.as_string() {
                Some(s) if words.contains(&s) => {}
                Some(s) => {
                    let hint = did_you_mean(s, words.iter().copied())
                        .map(|w| format!("did you mean \"{w}\"?"))
                        .unwrap_or_else(|| format!("one of: {}", words.join(", ")));
                    self.err_at(
                        "wrong-type",
                        format!("`{}` can't be \"{s}\"", pr.name),
                        Some(hint),
                        span,
                        path,
                    );
                }
                None => {
                    let msg = wrong(&format!("one of: {}", words.join(", ")));
                    self.err_at("wrong-type", msg, None, span, path);
                }
            },
            Ty::Any => {
                if let Some(s) = v.as_string() {
                    if let Err(te) = Template::parse(s) {
                        let offset = self.string_offset(e, te.offset);
                        self.err_at(
                            "bad-template",
                            te.message,
                            None,
                            Span { offset, len: 1 },
                            path,
                        );
                    }
                } else if matches!(v, KdlValue::Null) {
                    self.err_at("wrong-type", wrong("a value"), None, span, path);
                }
            }
        }
    }

    /// Warnings about text that almost certainly isn't what was meant.
    fn lint_text(&mut self, e: &KdlEntry, s: &str, path: &NodePath) {
        let repr = e.format().map(|f| f.value_repr.as_str()).unwrap_or("");
        let quoted_single_line = repr.starts_with('"') && !repr.starts_with("\"\"\"");
        // A lone \r (not a CR LF line ending), \t, \b or \f is what `"C:\tools\run.exe"` or
        // `".\report.json"` turn into.
        let accidental = s.contains(['\t', '\u{8}', '\u{c}'])
            || s.match_indices('\r')
                .any(|(i, _)| !s[i + 1..].starts_with('\n'));
        if quoted_single_line && accidental {
            self.warn_at(
                "escaped-backslash",
                "this text contains a control character from a backslash escape (\\r, \\t...)".into(),
                Some(r##"for a path write a raw string: #".\path\file"# (inside "..." a backslash starts an escape)"##.into()),
                Self::value_span(e),
                path,
            );
        }
        for (at, var) in template::percent_vars(s) {
            let offset = self.string_offset(e, at);
            self.warn_at(
                "percent-variable",
                format!("`%{var}%` is not expanded"),
                Some(format!("write {{env.{var}}} for an environment variable")),
                Span {
                    offset,
                    len: var.len() + 2,
                },
                path,
            );
        }
    }

    /// The file offset of byte `at` of a string value (exact for raw strings and strings without
    /// escapes, LESSONS L1.3; the value's start otherwise).
    fn string_offset(&self, e: &KdlEntry, at: usize) -> usize {
        let vs = Self::value_span(e);
        let repr = &self.src[vs.offset..(vs.offset + vs.len).min(self.src.len())];
        let hashes = repr.chars().take_while(|c| *c == '#').count();
        let quote_len = if repr[hashes..].starts_with("\"\"\"") {
            return vs.offset;
        } else if repr[hashes..].starts_with('"') {
            1
        } else {
            0
        };
        let has_escapes = hashes == 0 && repr.contains('\\');
        if has_escapes {
            vs.offset
        } else {
            vs.offset + hashes + quote_len + at
        }
    }

    // ---- typed reads (after `view` checked the types) ---------------------------------------

    fn string(e: &KdlEntry) -> Option<String> {
        e.value().as_string().map(str::to_string)
    }

    fn lua_src(&self, e: &KdlEntry) -> Option<LuaSrc> {
        let code = e.value().as_string()?.to_string();
        let span = Self::value_span(e);
        let (line, col) = self.lines.line_col(span.offset);
        let repr = e
            .format()
            .map(|f| f.value_repr.clone())
            .unwrap_or_else(|| code.clone());
        Some(LuaSrc {
            code,
            repr,
            span,
            line,
            col,
        })
    }

    /// A template, or `(lua)"expr"`.
    fn val(&self, e: &KdlEntry) -> Option<Val> {
        if e.ty().is_some_and(|t| t.value() == "lua") {
            return self.lua_src(e).map(Val::Lua);
        }
        match e.value() {
            KdlValue::String(s) => Template::parse(s).ok().map(Val::Text),
            KdlValue::Integer(i) => Some(Val::literal(&i.to_string())),
            KdlValue::Float(f) => Some(Val::literal(&f.to_string())),
            KdlValue::Bool(b) => Some(Val::literal(if *b { "true" } else { "false" })),
            KdlValue::Null => None,
        }
    }

    fn lua(&self, e: &KdlEntry) -> Option<LuaSrc> {
        self.lua_src(e)
    }

    fn int(e: &KdlEntry) -> Option<i64> {
        match e.value() {
            KdlValue::Integer(i) => i64::try_from(*i).ok(),
            _ => None,
        }
    }

    fn num(e: &KdlEntry) -> Option<f64> {
        match e.value() {
            KdlValue::Integer(i) => Some(*i as f64),
            KdlValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    fn boolean(e: &KdlEntry) -> Option<bool> {
        match e.value() {
            KdlValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn p_string(nv: &Nv, name: &str) -> Option<String> {
        nv.prop(name).and_then(Self::string)
    }

    fn p_val(&self, nv: &Nv, name: &str) -> Option<Val> {
        nv.prop(name).and_then(|e| self.val(e))
    }

    fn p_lua(&self, nv: &Nv, name: &str) -> Option<LuaSrc> {
        nv.prop(name).and_then(|e| self.lua(e))
    }

    fn p_bool(nv: &Nv, name: &str, default: bool) -> bool {
        nv.prop(name).and_then(Self::boolean).unwrap_or(default)
    }

    fn p_num(nv: &Nv, name: &str) -> Option<f64> {
        nv.prop(name).and_then(Self::num)
    }

    fn p_int(nv: &Nv, name: &str) -> Option<i64> {
        nv.prop(name).and_then(Self::int)
    }

    fn a_string(nv: &Nv, i: usize) -> Option<String> {
        nv.arg(i).and_then(Self::string)
    }

    fn a_val(&self, nv: &Nv, i: usize) -> Option<Val> {
        nv.arg(i).and_then(|e| self.val(e))
    }

    fn rest_vals(&self, nv: &Nv, from: usize) -> Vec<Val> {
        nv.args[from.min(nv.args.len())..]
            .iter()
            .filter_map(|e| self.val(e))
            .collect()
    }

    // ---- the document ----------------------------------------------------------------------

    fn top(&mut self, doc: &'a KdlDocument, config: &mut Config) {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (i, node) in doc.nodes().iter().enumerate() {
            let Some(nv) = self.view(node, vec![i], Children::None, "at the top of the file")
            else {
                continue;
            };
            if nv.spec.cat != Category::Top {
                continue;
            }
            let name = nv.name();
            if matches!(name, "launcher" | "window" | "layout") {
                let count = seen.entry(name).or_insert(0);
                *count += 1;
                if *count > 1 {
                    self.err_at(
                        "duplicate-node",
                        format!("there can only be one `{name}`"),
                        None,
                        Self::node_span(node),
                        &nv.path,
                    );
                    continue;
                }
            }
            match name {
                "launcher" => {
                    if i != 0 {
                        self.err_at(
                            "header-first",
                            "`launcher` must be the first node in the file".into(),
                            Some("move it to the top: it is read before everything else".into()),
                            Self::node_span(node),
                            &nv.path,
                        );
                    }
                    config.header = Header {
                        requires: Self::p_string(&nv, "requires").and_then(|s| Version::parse(&s)),
                        os: Self::p_string(&nv, "os").and_then(|s| match s.as_str() {
                            "windows" => Some(Os::Windows),
                            "macos" => Some(Os::Macos),
                            _ => None,
                        }),
                        save_logs: Self::p_bool(&nv, "save-logs", false),
                        node: Some(nv.node_ref()),
                    };
                }
                "window" => config.window = self.window(&nv),
                "script" => {
                    if let Some(code) = nv.arg(0).and_then(|e| self.lua(e)) {
                        config.scripts.push(code);
                    }
                }
                "value" => {
                    if let Some(v) = self.value(&nv) {
                        config.values.push(v);
                    }
                }
                "data" => {
                    if let Some(id) = Self::a_string(&nv, 0) {
                        let bag = self.bag(node, &nv.path);
                        config.data.push(DataDef {
                            id,
                            bag,
                            node: nv.node_ref(),
                        });
                    }
                }
                "layout" => {
                    config.layout_node = Some(nv.node_ref());
                    if let Some(kids) = node.children() {
                        config.layout = self.items(kids, &nv.path, "in `layout`");
                    }
                }
                "action" => {
                    if let Some(a) = self.action(&nv) {
                        config.actions.push(a);
                    }
                }
                _ => {}
            }
        }
    }

    fn window(&mut self, nv: &Nv) -> WindowSpec {
        let d = WindowSpec::default();
        WindowSpec {
            title: self.p_val(nv, "title").unwrap_or(d.title),
            width: Self::p_num(nv, "width").map(|v| v as f32),
            height: Self::p_num(nv, "height").map(|v| v as f32),
            resizable: Self::p_bool(nv, "resizable", d.resizable),
            theme: match Self::p_string(nv, "theme").as_deref() {
                Some("dark") => Theme::Dark,
                Some("light") => Theme::Light,
                _ => Theme::System,
            },
            label_width: Self::p_num(nv, "label-width").map_or(d.label_width, |v| v as f32),
            field_width: Self::p_num(nv, "field-width").map_or(d.field_width, |v| v as f32),
            center: Self::p_string(nv, "position").as_deref() == Some("center"),
            node: Some(nv.node_ref()),
        }
    }

    fn value(&mut self, nv: &Nv<'a>) -> Option<ValueDef> {
        let id = Self::a_string(nv, 0)?;
        let find = nv.node.children().and_then(|kids| {
            let mut found = None;
            for (i, k) in kids.nodes().iter().enumerate() {
                let mut path = nv.path.clone();
                path.push(i);
                if let Some(kv) = self.view(k, path, nv.spec.children, "in a `value`") {
                    if found.is_none() {
                        found = Some(self.find_file(&kv));
                    } else {
                        self.err_at(
                            "duplicate-node",
                            "a `value` holds one find-file".into(),
                            None,
                            Self::node_span(k),
                            &kv.path,
                        );
                    }
                }
            }
            found
        });
        let source = match (self.a_val(nv, 1), find) {
            (Some(v), None) => ValueSource::Val(v),
            (None, Some(f)) => ValueSource::Find(f),
            (Some(v), Some(_)) => {
                self.err_at(
                    "value-both",
                    format!("`value \"{id}\"` has both a value and a find-file block"),
                    Some("keep one".into()),
                    Self::node_span(nv.node),
                    &nv.path,
                );
                ValueSource::Val(v)
            }
            (None, None) => {
                self.err_at(
                    "missing-argument",
                    format!("`value \"{id}\"` needs a value or a find-file block"),
                    None,
                    Self::node_span(nv.node),
                    &nv.path,
                );
                return None;
            }
        };
        Some(ValueDef {
            id,
            source,
            node: nv.node_ref(),
        })
    }

    fn find_file(&mut self, nv: &Nv) -> FindFile {
        FindFile {
            pattern: Self::a_string(nv, 0).unwrap_or_default(),
            dir: self.p_val(nv, "in").unwrap_or_else(|| Val::literal(".")),
            recursive: Self::p_bool(nv, "recursive", false),
            exclude_self: Self::p_bool(nv, "exclude-self", true),
            require_sibling: self.p_val(nv, "require-sibling"),
        }
    }

    /// `Key "value"` lines.
    fn bag(&mut self, node: &KdlNode, path: &NodePath) -> Bag {
        let mut bag: Bag = Vec::new();
        let Some(kids) = node.children() else {
            return bag;
        };
        for (i, k) in kids.nodes().iter().enumerate() {
            let mut kp = path.clone();
            kp.push(i);
            let key = k.name().value().to_string();
            let args: Vec<&KdlEntry> = k.entries().iter().collect();
            let ok = args.len() == 1 && args[0].name().is_none() && k.children().is_none();
            if !ok {
                self.err_at(
                    "bad-data",
                    format!("`{key}` should be a key with one value: {key} \"value\""),
                    None,
                    Self::node_span(k),
                    &kp,
                );
                continue;
            }
            let value = match args[0].value() {
                KdlValue::String(s) => {
                    self.lint_text(args[0], s, &kp);
                    s.clone()
                }
                KdlValue::Integer(n) => n.to_string(),
                KdlValue::Float(f) => f.to_string(),
                KdlValue::Bool(b) => b.to_string(),
                KdlValue::Null => String::new(),
            };
            if bag.iter().any(|(k2, _)| k2.eq_ignore_ascii_case(&key)) {
                self.warn_at(
                    "duplicate-key",
                    format!("`{key}` appears twice; the first one is used"),
                    None,
                    Self::node_span(k),
                    &kp,
                );
                continue;
            }
            bag.push((key, value));
        }
        bag
    }

    fn items(&mut self, doc: &'a KdlDocument, path: &NodePath, parent: &str) -> Vec<Item> {
        let mut out = Vec::new();
        for (i, node) in doc.nodes().iter().enumerate() {
            let mut p = path.clone();
            p.push(i);
            let Some(nv) = self.view(node, p, Children::Layout, parent) else {
                continue;
            };
            if let Some(item) = self.item(&nv) {
                out.push(item);
            }
        }
        out
    }

    fn item(&mut self, nv: &Nv<'a>) -> Option<Item> {
        let node = nv.node_ref();
        let kids = nv.node.children();
        let name = nv.name();
        Some(match name {
            "group" => Item::Group {
                title: self.a_val(nv, 0),
                visible_if: self.p_lua(nv, "visible-if"),
                items: kids
                    .map(|k| self.items(k, &nv.path, "in a `group`"))
                    .unwrap_or_default(),
                node,
            },
            "row" => Item::Row {
                visible_if: self.p_lua(nv, "visible-if"),
                items: kids
                    .map(|k| self.items(k, &nv.path, "in a `row`"))
                    .unwrap_or_default(),
                node,
            },
            "tabs" => {
                let mut tabs = Vec::new();
                if let Some(k) = kids {
                    for (i, t) in k.nodes().iter().enumerate() {
                        let mut p = nv.path.clone();
                        p.push(i);
                        let Some(tv) = self.view(t, p, nv.spec.children, "in `tabs`") else {
                            continue;
                        };
                        tabs.push(Tab {
                            title: self.a_val(&tv, 0).unwrap_or_else(|| Val::literal("Tab")),
                            visible_if: self.p_lua(&tv, "visible-if"),
                            items: tv
                                .node
                                .children()
                                .map(|k| self.items(k, &tv.path, "in a `tab`"))
                                .unwrap_or_default(),
                            node: tv.node_ref(),
                        });
                    }
                }
                Item::Tabs { tabs, node }
            }
            "columns" => {
                let mut columns = Vec::new();
                if let Some(k) = kids {
                    for (i, c) in k.nodes().iter().enumerate() {
                        let mut p = nv.path.clone();
                        p.push(i);
                        let Some(cv) = self.view(c, p, nv.spec.children, "in `columns`") else {
                            continue;
                        };
                        columns.push(
                            cv.node
                                .children()
                                .map(|k| self.items(k, &cv.path, "in a `column`"))
                                .unwrap_or_default(),
                        );
                    }
                }
                Item::Columns { columns, node }
            }
            "separator" => Item::Separator {
                title: self.a_val(nv, 0),
                node,
            },
            "spacer" => Item::Spacer {
                height: Self::p_num(nv, "height").unwrap_or(8.0) as f32,
                node,
            },
            "note" => Item::Note {
                text: self.a_val(nv, 0).unwrap_or_else(|| Val::literal("")),
                style: match Self::p_string(nv, "style").as_deref() {
                    Some("muted") => NoteStyle::Muted,
                    Some("warning") => NoteStyle::Warning,
                    Some("error") => NoteStyle::Error,
                    _ => NoteStyle::Normal,
                },
                visible_if: self.p_lua(nv, "visible-if"),
                node,
            },
            _ => {
                let c = self.control(nv)?;
                self.controls.push(c);
                Item::Control(self.controls.len() - 1)
            }
        })
    }

    fn width(&mut self, nv: &Nv) -> Width {
        let Some(e) = nv.prop("width") else {
            return Width::Field;
        };
        match e.value() {
            KdlValue::Integer(i) => Width::Px(*i as f32),
            KdlValue::Float(f) => Width::Px(*f as f32),
            KdlValue::String(s) => match s.as_str() {
                "fill" => Width::Fill,
                "auto" => Width::Auto,
                other => other.parse::<f32>().map_or(Width::Field, Width::Px),
            },
            _ => Width::Field,
        }
    }

    fn control(&mut self, nv: &Nv<'a>) -> Option<Control> {
        let id = Self::a_string(nv, 0)?;
        let name = nv.name();
        let mut on_change = Vec::new();
        let mut options = Vec::new();
        let mut presets = Vec::new();
        let mut list = None;
        if let Some(kids) = nv.node.children() {
            for (i, k) in kids.nodes().iter().enumerate() {
                let mut p = nv.path.clone();
                p.push(i);
                let parent = format!("in a `{name}`");
                let Some(kv) = self.view(k, p, nv.spec.children, &parent) else {
                    continue;
                };
                match kv.name() {
                    "on-change" => {
                        if let Some(steps) = kv.node.children() {
                            on_change.extend(self.steps(steps, &kv.path, true));
                        }
                    }
                    "option" => {
                        let Some(oname) = Self::a_string(&kv, 0) else {
                            continue;
                        };
                        let value = Self::p_string(&kv, "value").unwrap_or_else(|| oname.clone());
                        let bag = self.bag(k, &kv.path);
                        options.push(OptionDef {
                            name: oname,
                            value,
                            label: self.p_val(&kv, "label"),
                            bag,
                            node: kv.node_ref(),
                        });
                    }
                    "list-from" => list = self.list_ref(&kv),
                    "preset" => {
                        if let Some(value) = self.a_val(&kv, 0) {
                            presets.push(Preset {
                                value,
                                match_field: Self::p_string(&kv, "match"),
                                node: kv.node_ref(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        let placeholder = Self::p_string(nv, "placeholder");
        let kind = match name {
            "dropdown" | "radio" | "checklist" => {
                let style = match name {
                    "dropdown" => ChoiceStyle::Dropdown,
                    "radio" => ChoiceStyle::Radio {
                        horizontal: Self::p_bool(nv, "horizontal", true),
                    },
                    _ => ChoiceStyle::Checklist,
                };
                let options_lua = self.p_lua(nv, "options");
                if list.is_some() && (options_lua.is_some() || !options.is_empty()) {
                    self.err_at(
                        "options-both",
                        format!("`{id}` fills its choices from a list and also has its own"),
                        Some("keep one".into()),
                        Self::node_span(nv.node),
                        &nv.path,
                    );
                } else if options_lua.is_some() && !options.is_empty() {
                    self.err_at(
                        "options-both",
                        format!("`{id}` has both options=(lua) and option lines"),
                        Some("keep one".into()),
                        Self::node_span(nv.node),
                        &nv.path,
                    );
                } else if options_lua.is_none()
                    && options.is_empty()
                    && list.is_none()
                    && !self.tool
                {
                    self.warn_at(
                        "no-options",
                        format!("`{id}` has no options"),
                        Some(
                            "add option lines, or options=(lua)\"...\" for a computed list".into(),
                        ),
                        Self::node_span(nv.node),
                        &nv.path,
                    );
                }
                ControlKind::Choice {
                    style,
                    options,
                    options_lua,
                    list,
                    option_label: self.p_val(nv, "option-label"),
                    presets,
                }
            }
            "checkbox" => ControlKind::Checkbox,
            "slider" => {
                let min = Self::p_num(nv, "min").unwrap_or(0.0);
                let max = Self::p_num(nv, "max").unwrap_or(100.0);
                if max <= min {
                    self.err_at(
                        "bad-range",
                        format!("`{id}`: max ({max}) must be above min ({min})"),
                        None,
                        Self::node_span(nv.node),
                        &nv.path,
                    );
                }
                ControlKind::Slider {
                    min,
                    max,
                    step: Self::p_num(nv, "step").unwrap_or(1.0),
                    suffix: Self::p_string(nv, "suffix"),
                }
            }
            "number" => ControlKind::Number {
                min: Self::p_num(nv, "min"),
                max: Self::p_num(nv, "max"),
                step: Self::p_num(nv, "step").unwrap_or(1.0),
                decimals: Self::p_int(nv, "decimals").unwrap_or(0).clamp(0, 12) as u32,
            },
            "text" => {
                let pattern = Self::p_string(nv, "pattern");
                ControlKind::Text {
                    multiline: Self::p_bool(nv, "multiline", false),
                    lines: Self::p_int(nv, "lines").unwrap_or(4).clamp(1, 100) as u32,
                    password: Self::p_bool(nv, "password", false),
                    placeholder,
                    pattern,
                    pattern_message: Self::p_string(nv, "pattern-message"),
                }
            }
            "file" => ControlKind::File {
                filter: Self::p_string(nv, "filter"),
                save: Self::p_bool(nv, "save", false),
                must_exist: Self::p_bool(nv, "must-exist", false),
                relative: Self::p_bool(nv, "relative", true),
                placeholder,
            },
            "folder" => ControlKind::Folder {
                must_exist: Self::p_bool(nv, "must-exist", false),
                relative: Self::p_bool(nv, "relative", true),
                placeholder,
            },
            "display" => ControlKind::Display {
                text: self.p_val(nv, "text").unwrap_or_else(|| Val::literal("")),
                copyable: Self::p_bool(nv, "copyable", true),
            },
            "button" => ControlKind::Button {
                action: Self::p_string(nv, "action").unwrap_or_default(),
                confirm: self.p_val(nv, "confirm"),
                primary: Self::p_bool(nv, "primary", false),
            },
            _ => return None,
        };
        let default = nv.prop("default").and_then(|e| {
            if e.ty().is_some_and(|t| t.value() == "lua") {
                return self.lua(e).map(|l| DefaultVal::Val(Val::Lua(l)));
            }
            match e.value() {
                KdlValue::Bool(b) => Some(DefaultVal::Bool(*b)),
                KdlValue::Integer(i) => Some(DefaultVal::Num(*i as f64)),
                KdlValue::Float(f) => Some(DefaultVal::Num(*f)),
                KdlValue::String(_) => self.val(e).map(DefaultVal::Val),
                KdlValue::Null => None,
            }
        });
        Some(Control {
            id,
            label: Self::p_string(nv, "label"),
            tooltip: self.p_val(nv, "tooltip"),
            default,
            persist: Self::p_bool(nv, "persist", true),
            visible_if: self.p_lua(nv, "visible-if"),
            enabled_if: self.p_lua(nv, "enabled-if"),
            disabled_tooltip: self.p_val(nv, "disabled-tooltip"),
            required: Self::p_bool(nv, "required", false),
            width: self.width(nv),
            on_change,
            node: nv.node_ref(),
            kind,
        })
    }

    fn action(&mut self, nv: &Nv<'a>) -> Option<Action> {
        let id = Self::a_string(nv, 0)?;
        let steps = nv
            .node
            .children()
            .map(|k| self.steps(k, &nv.path, false))
            .unwrap_or_default();
        Some(Action {
            id,
            label: Self::p_string(nv, "label"),
            after: match Self::p_string(nv, "after").as_deref() {
                Some("exit") => After::Exit,
                Some("minimize") => After::Minimize,
                _ => After::Stay,
            },
            lock: self.p_val(nv, "lock"),
            steps,
            node: nv.node_ref(),
        })
    }

    fn steps(&mut self, doc: &'a KdlDocument, path: &NodePath, on_change: bool) -> Vec<Step> {
        let mut out = Vec::new();
        let parent = if on_change {
            "in `on-change`"
        } else {
            "in an `action`"
        };
        for (i, node) in doc.nodes().iter().enumerate() {
            let mut p = path.clone();
            p.push(i);
            let Some(nv) = self.view(node, p, Children::Steps, parent) else {
                continue;
            };
            if on_change && !nv.spec.on_change {
                let allowed: Vec<&str> = catalog::SPECS
                    .iter()
                    .filter(|s| s.on_change)
                    .map(|s| s.name)
                    .collect();
                self.err_at(
                    "not-instant",
                    format!("`{}` can't run in on-change", nv.name()),
                    Some(format!(
                        "on-change only runs instant steps: {}",
                        allowed.join(", ")
                    )),
                    Self::node_span(node),
                    &nv.path,
                );
                continue;
            }
            if let Some(s) = self.step(&nv) {
                out.push(s);
            }
        }
        out
    }

    fn level(nv: &Nv) -> Level {
        match Self::p_string(nv, "level").as_deref() {
            Some("debug") => Level::Debug,
            Some("warning") | Some("warn") => Level::Warning,
            Some("error") => Level::Error,
            _ => Level::Info,
        }
    }

    fn expect_exit(&mut self, nv: &Nv) -> ExpectExit {
        let Some(e) = nv.prop("expect-exit") else {
            return ExpectExit::Codes(vec![0]);
        };
        let s = Self::string(e).unwrap_or_default();
        match ExpectExit::parse(&s) {
            Some(x) => x,
            None => {
                self.err_at(
                    "wrong-type",
                    format!("`expect-exit` can't be \"{s}\""),
                    Some("exit codes separated by commas, like \"0,1\", or \"any\"".into()),
                    Self::value_span(e),
                    &nv.path,
                );
                ExpectExit::Codes(vec![0])
            }
        }
    }

    fn env_children(&mut self, nv: &Nv<'a>) -> Vec<(String, Val)> {
        let mut env = Vec::new();
        if let Some(kids) = nv.node.children() {
            for (i, k) in kids.nodes().iter().enumerate() {
                let mut p = nv.path.clone();
                p.push(i);
                let parent = format!("in `{}`", nv.name());
                let Some(kv) = self.view(k, p, nv.spec.children, &parent) else {
                    continue;
                };
                if let (Some(n), Some(v)) = (Self::a_string(&kv, 0), self.a_val(&kv, 1)) {
                    env.push((n, v));
                }
            }
        }
        env
    }

    fn step(&mut self, nv: &Nv<'a>) -> Option<Step> {
        let name = nv.name();
        let kind = match name {
            "run" => {
                let program = self.a_val(nv, 0)?;
                let command_line = self.p_val(nv, "command-line");
                if command_line.is_some() && nv.args.len() > 1 {
                    self.err_at(
                        "args-both",
                        "`run` has both arguments and command-line=".into(),
                        Some("use one: separate arguments, or one raw command line".into()),
                        Self::node_span(nv.node),
                        &nv.path,
                    );
                }
                let io = match Self::p_string(nv, "io").as_deref() {
                    Some("pty") => Io::Pty,
                    Some("pipe") => Io::Pipe,
                    _ => Io::Auto,
                };
                let fail_if_output = Self::p_string(nv, "fail-if-output");
                StepKind::Run(Box::new(RunStep {
                    program,
                    args: self.rest_vals(nv, 1),
                    command_line,
                    cwd: self.p_val(nv, "cwd"),
                    stdin: self.p_val(nv, "stdin"),
                    wait: Self::p_bool(nv, "wait", true),
                    timeout_ms: Self::p_int(nv, "timeout").map(|t| t.max(0) as u64),
                    expect_exit: self.expect_exit(nv),
                    io,
                    fail_if_output,
                    env: self.env_children(nv),
                }))
            }
            "check" => StepKind::Check {
                program: self.a_val(nv, 0)?,
                args: self.rest_vals(nv, 1),
                cwd: self.p_val(nv, "cwd"),
                expect_exit: self.expect_exit(nv),
            },
            "capture" => StepKind::Capture {
                program: self.a_val(nv, 0)?,
                args: self.rest_vals(nv, 1),
                cwd: self.p_val(nv, "cwd"),
                expect_exit: self.expect_exit(nv),
            },
            "use" => StepKind::Use(Box::new(self.use_step(nv)?)),
            "env-from" => StepKind::EnvFrom {
                script: self.a_val(nv, 0)?,
                args: self.rest_vals(nv, 1),
            },
            "set" => StepKind::Set {
                name: Self::a_string(nv, 0)?,
                value: self.a_val(nv, 1)?,
            },
            "set-control" => StepKind::SetControl {
                control: Self::a_string(nv, 0)?,
                value: self.a_val(nv, 1)?,
            },
            "reset" => StepKind::Reset {
                control: Self::a_string(nv, 0)?,
            },
            "put" => StepKind::Put {
                control: Self::a_string(nv, 0)?,
                step: Self::a_string(nv, 1)?,
                result: Self::a_string(nv, 2)?,
            },
            "copy" => StepKind::Copy {
                from: self.a_val(nv, 0)?,
                to: self.a_val(nv, 1)?,
                overwrite: Self::p_bool(nv, "overwrite", true),
                ask_first: Self::p_bool(nv, "ask-first", false),
            },
            "move" => StepKind::Move {
                from: self.a_val(nv, 0)?,
                to: self.a_val(nv, 1)?,
                overwrite: Self::p_bool(nv, "overwrite", false),
            },
            "delete" => StepKind::Delete {
                path: self.a_val(nv, 0)?,
                missing_ok: Self::p_string(nv, "missing").as_deref() != Some("error"),
                to_trash: Self::p_bool(nv, "to-trash", false),
            },
            "mkdir" => StepKind::Mkdir {
                path: self.a_val(nv, 0)?,
            },
            "find-file" => StepKind::FindFile(self.find_file(nv)),
            "read-file" => StepKind::ReadFile {
                path: self.a_val(nv, 0)?,
            },
            "write-file" => StepKind::WriteFile {
                path: self.a_val(nv, 0)?,
                content: self.p_val(nv, "content")?,
                format: match Self::p_string(nv, "format").as_deref() {
                    Some("json-pretty") => WriteFormat::JsonPretty,
                    Some("json-compact") => WriteFormat::JsonCompact,
                    _ => WriteFormat::Text,
                },
                newline: match Self::p_string(nv, "newline").as_deref() {
                    Some("lf") => Newline::Lf,
                    Some("crlf") => Newline::Crlf,
                    _ => Newline::Keep,
                },
                create_dirs: Self::p_bool(nv, "create-dirs", true),
                heading: self.p_val(nv, "heading"),
                append: Self::p_bool(nv, "append", false),
                keep: Self::p_bool(nv, "keep", false),
                item: {
                    let one = Self::p_string(nv, "item").unwrap_or_else(|| "item".into());
                    let many = Self::p_string(nv, "items").unwrap_or_else(|| format!("{one}s"));
                    (one, many)
                },
            },
            "http-get" => {
                let mut extract = Vec::new();
                let mut headers = Vec::new();
                if let Some(kids) = nv.node.children() {
                    for (i, k) in kids.nodes().iter().enumerate() {
                        let mut p = nv.path.clone();
                        p.push(i);
                        let Some(kv) = self.view(k, p, nv.spec.children, "in `http-get`") else {
                            continue;
                        };
                        match kv.name() {
                            "extract" => {
                                if let (Some(n), Some(path)) =
                                    (Self::a_string(&kv, 0), Self::a_string(&kv, 1))
                                {
                                    extract.push(Extract {
                                        name: n,
                                        path,
                                        required: Self::p_bool(&kv, "required", false),
                                    });
                                }
                            }
                            "header" => {
                                if let (Some(n), Some(v)) =
                                    (Self::a_string(&kv, 0), self.a_val(&kv, 1))
                                {
                                    headers.push((n, v));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                StepKind::HttpGet {
                    url: self.a_val(nv, 0)?,
                    timeout_ms: Self::p_int(nv, "timeout").unwrap_or(10_000).max(0) as u64,
                    extract,
                    headers,
                }
            }
            "ping" => StepKind::Ping {
                host: self.a_val(nv, 0)?,
                timeout_ms: Self::p_int(nv, "timeout").unwrap_or(1500).max(0) as u64,
            },
            "open" => StepKind::Open {
                target: self.a_val(nv, 0)?,
            },
            "reveal" => StepKind::Reveal {
                path: self.a_val(nv, 0)?,
            },
            "message" => StepKind::Message {
                text: self.a_val(nv, 0)?,
                level: Self::level(nv),
            },
            "confirm" => StepKind::Confirm {
                text: self.a_val(nv, 0)?,
                yes: Self::p_string(nv, "yes").unwrap_or_else(|| "Yes".into()),
                no: Self::p_string(nv, "no").unwrap_or_else(|| "No".into()),
            },
            "ask" => StepKind::Ask {
                name: Self::a_string(nv, 0)?,
                prompt: self.a_val(nv, 1)?,
                default: self.p_val(nv, "default"),
                password: Self::p_bool(nv, "password", false),
            },
            "wait" => StepKind::Wait {
                ms: nv.arg(0).and_then(Self::int)?.max(0) as u64,
            },
            "log" => StepKind::Log {
                text: self.a_val(nv, 0)?,
                level: Self::level(nv),
            },
            "fail" => StepKind::Fail {
                text: self.a_val(nv, 0)?,
            },
            "stop" => StepKind::Stop,
            "lua" => StepKind::Lua {
                code: nv.arg(0).and_then(|e| self.lua(e))?,
            },
            "args" => StepKind::Args(self.args_spec(nv)?),
            "set-process-volume" => StepKind::SetProcessVolume {
                run: Self::a_string(nv, 0)?,
                volume: self.p_val(nv, "volume")?,
                timeout_ms: Self::p_int(nv, "timeout").unwrap_or(10_000).max(0) as u64,
            },
            _ => return None,
        };
        Some(Step {
            kind,
            first: Self::p_bool(nv, "first", false),
            id: Self::p_string(nv, "id"),
            label: self.p_val(nv, "label"),
            if_: self.p_lua(nv, "if"),
            on_error: match Self::p_string(nv, "on-error").as_deref() {
                Some("warn") => OnError::Warn,
                Some("continue") => OnError::Continue,
                _ => OnError::Abort,
            },
            message: self.p_val(nv, "message"),
            node: nv.node_ref(),
        })
    }

    /// `list-from "git.branches" { with "folder" "." }`: an add-on list and its settings.
    fn list_ref(&mut self, nv: &Nv<'a>) -> Option<ListRef> {
        let name = Self::a_string(nv, 0)?;
        let span = nv.arg(0).map(Ld::value_span).unwrap_or_default();
        let lib = self.library.clone()?;
        let Some(def) = lib.list(&name) else {
            let names: Vec<String> = lib
                .addons
                .iter()
                .flat_map(|a| a.lists.iter().map(|l| l.id.clone()))
                .collect();
            self.err_at(
                "unknown-list",
                format!("There's no list called \"{name}\""),
                did_you_mean(&name, names.iter().map(String::as_str))
                    .map(|m| format!("did you mean `{m}`?")),
                span,
                &nv.path,
            );
            return None;
        };
        let settings = self.with_children(nv, |sname| {
            def.settings
                .iter()
                .map(|s| s.id.clone())
                .find(|s| s == sname)
        });
        Some(ListRef { def, settings })
    }

    /// The `with "name" value` children of a `use` or `list-from`; `known` says whether a name is
    /// a setting.
    fn with_children(
        &mut self,
        nv: &Nv<'a>,
        known: impl Fn(&str) -> Option<String>,
    ) -> Vec<(String, Val)> {
        let mut out = Vec::new();
        let Some(kids) = nv.node.children() else {
            return out;
        };
        for (i, c) in kids.nodes().iter().enumerate() {
            let mut p = nv.path.clone();
            p.push(i);
            let Some(w) = self.view(c, p, Children::Only(&["with"]), "here") else {
                continue;
            };
            let Some(sname) = Self::a_string(&w, 0) else {
                continue;
            };
            if known(&sname).is_none() {
                self.err_at(
                    "unknown-setting",
                    format!("There's no setting \"{sname}\" here"),
                    None,
                    w.arg(0).map(Ld::value_span).unwrap_or_default(),
                    &w.path,
                );
                continue;
            }
            if let Some(v) = self.a_val(&w, 1) {
                out.push((sname, v));
            }
        }
        out
    }

    /// `use "git.pull" { with "folder" "." }`: the add-on step and its settings.
    fn use_step(&mut self, nv: &Nv<'a>) -> Option<UseStep> {
        let name = Self::a_string(nv, 0)?;
        let span = nv.arg(0).map(Ld::value_span).unwrap_or_default();
        let Some(lib) = self.library.clone() else {
            self.err_at(
                "use-in-add-on",
                "An add-on's own steps can't use other add-on steps yet".into(),
                None,
                span,
                &nv.path,
            );
            return None;
        };
        let Some(def) = lib.step(&name) else {
            let (addon, _) = name.split_once('.').unwrap_or((name.as_str(), ""));
            let help = if !name.contains('.') || lib.addon(addon).is_some() {
                did_you_mean(&name, lib.step_names()).map(|m| format!("did you mean `{m}`?"))
            } else {
                Some(format!(
                    "this needs the \"{addon}\" add-on, which isn't on this computer: put it in the Add-ons folder, or ask whoever made the tool to include it"
                ))
            };
            self.err_at(
                "unknown-step",
                format!("There's no step called \"{name}\""),
                help,
                span,
                &nv.path,
            );
            return None;
        };
        let mut settings: Vec<(String, Val)> = Vec::new();
        if let Some(kids) = nv.node.children() {
            for (i, c) in kids.nodes().iter().enumerate() {
                let mut p = nv.path.clone();
                p.push(i);
                let Some(w) = self.view(c, p, Children::Only(&["with"]), "in a `use`") else {
                    continue;
                };
                let Some(sname) = Self::a_string(&w, 0) else {
                    continue;
                };
                if def.setting(&sname).is_none() {
                    self.err_at(
                        "unknown-setting",
                        format!("\"{}\" has no setting \"{sname}\"", def.name),
                        did_you_mean(&sname, def.settings.iter().map(|s| s.id.as_str()))
                            .map(|m| format!("did you mean \"{m}\"?")),
                        w.arg(0).map(Ld::value_span).unwrap_or_default(),
                        &w.path,
                    );
                    continue;
                }
                if let Some(v) = self.a_val(&w, 1) {
                    settings.push((sname, v));
                }
            }
        }
        for s in &def.settings {
            if s.required && s.starts.is_none() && !settings.iter().any(|(k, _)| *k == s.id) {
                self.err_at(
                    "missing-setting",
                    format!("Fill in \"{}\" for \"{}\"", s.label, def.name),
                    None,
                    Ld::node_span(nv.node),
                    &nv.path,
                );
            }
        }
        Some(UseStep { def, settings })
    }

    fn args_spec(&mut self, nv: &Nv<'a>) -> Option<ArgsSpec> {
        let mut spec = ArgsSpec {
            name: Self::a_string(nv, 0)?,
            format: None,
            replace: Vec::new(),
            dedupe: Dedupe::None,
            dedupe_ignore_case: true,
            parts: Vec::new(),
        };
        let Some(kids) = nv.node.children() else {
            return Some(spec);
        };
        for (i, k) in kids.nodes().iter().enumerate() {
            let mut p = nv.path.clone();
            p.push(i);
            let Some(kv) = self.view(k, p, nv.spec.children, "in `args`") else {
                continue;
            };
            match kv.name() {
                "format" => {
                    spec.format = Self::a_string(&kv, 0).and_then(|s| Template::parse(&s).ok())
                }
                "replace" => {
                    if let (Some(a), Some(b)) = (Self::a_string(&kv, 0), Self::a_string(&kv, 1)) {
                        spec.replace.push((a, b));
                    }
                }
                "dedupe" => {
                    spec.dedupe = match Self::a_string(&kv, 0).as_deref() {
                        Some("first") => Dedupe::First,
                        Some("last") => Dedupe::Last,
                        _ => Dedupe::None,
                    };
                    spec.dedupe_ignore_case = Self::p_bool(&kv, "ignore-case", true);
                }
                "raw" => {
                    if let Some(text) = self.a_val(&kv, 0) {
                        spec.parts.push(ArgPart::Raw {
                            text,
                            if_: self.p_lua(&kv, "if"),
                        });
                    }
                }
                "pair" => {
                    if let (Some(key), Some(value)) = (Self::a_string(&kv, 0), self.a_val(&kv, 1)) {
                        spec.parts.push(ArgPart::Pair {
                            key,
                            value,
                            if_: self.p_lua(&kv, "if"),
                        });
                    }
                }
                "pairs" => {
                    if let Some(source) = Self::a_string(&kv, 0) {
                        spec.parts.push(ArgPart::Pairs {
                            source,
                            exclude: Self::p_string(&kv, "exclude")
                                .map(|s| {
                                    s.split(',')
                                        .map(|x| x.trim().to_string())
                                        .filter(|x| !x.is_empty())
                                        .collect()
                                })
                                .unwrap_or_default(),
                            if_: self.p_lua(&kv, "if"),
                        });
                    }
                }
                _ => {}
            }
        }
        Some(spec)
    }
}

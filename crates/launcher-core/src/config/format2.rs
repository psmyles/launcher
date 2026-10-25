//! Tool files, format 2 (N19, N23–N26): the file the editor writes, in the app's own words.
//!
//! ```kdl
//! tool "Build the game" format=2 requires="0.2" os="windows"
//! window {
//!     choice "branch" label="Branch" { list-from "git.branches" folder="." }
//!     tick-box "tests" label="Run the tests" starts=#true
//!     button "build" label="Build"
//! }
//! steps "build" {
//!     before-starting { git.no-unsaved-changes }
//!     git.switch-branch "switch" branch="{branch}"
//!     dotnet.test "test" configuration="Release" if-wrong="ask"
//! }
//! ```
//!
//! A tool is checked here in those words (problems name what the person wrote, with "did you
//! mean"), then translated node by node into the loader's own kinds, which the rest of the engine
//! already knows: `window` items become layout items, `steps` an action, a built-in add-on step
//! the engine's step, any other add-on step a `use`. Translated nodes keep the original entries
//! (with their places), so problems found later still point at the right spot in the file.

use std::sync::Arc;

use kdl::{KdlDocument, KdlEntry, KdlNode};

use super::catalog::{self, Category};
use super::diag::{Diagnostic, LineIndex, Severity};
use super::load::LoadOptions;
use super::model::Span;
use super::template::Template;
use crate::addons::{self, AddOn, Body, Library, Origin, StepDef};
use crate::suggest::did_you_mean;

/// A tool translated for the loader.
pub(crate) struct Translated {
    pub doc: KdlDocument,
    pub diags: Vec<Diagnostic>,
    /// The add-ons this tool sees: Launcher's, plus the copies it carries.
    pub library: Library,
    pub carried: Vec<Arc<AddOn>>,
}

/// One kind of window item: its word in a tool, the loader's kind, and its properties as
/// (tool word, loader word).
struct Item {
    word: &'static str,
    v1: &'static str,
    /// Takes an id (controls) or a title (sections, pages, text) as its first argument.
    first_arg: bool,
    props: &'static [(&'static str, &'static str)],
    children: &'static [&'static str],
}

const CONTROL: &[(&str, &str)] = &[
    ("label", "label"),
    ("help", "tooltip"),
    ("starts", "default"),
    ("remember", "persist"),
    ("required", "required"),
    ("width", "width"),
    ("show-if", "visible-if"),
    ("enable-if", "enabled-if"),
    ("disabled-help", "disabled-tooltip"),
];

const ITEMS: &[Item] = &[
    Item {
        word: "row",
        v1: "row",
        first_arg: false,
        props: &[("show-if", "visible-if")],
        children: &[],
    },
    Item {
        word: "section",
        v1: "group",
        first_arg: true,
        props: &[("show-if", "visible-if")],
        children: &[],
    },
    Item {
        word: "pages",
        v1: "tabs",
        first_arg: false,
        props: &[],
        children: &["page"],
    },
    Item {
        word: "page",
        v1: "tab",
        first_arg: true,
        props: &[("show-if", "visible-if")],
        children: &[],
    },
    Item {
        word: "info",
        v1: "note",
        first_arg: true,
        props: &[("style", "style"), ("show-if", "visible-if")],
        children: &[],
    },
    Item {
        word: "line",
        v1: "separator",
        first_arg: true,
        props: &[],
        children: &[],
    },
    Item {
        word: "choice",
        v1: "dropdown",
        first_arg: true,
        props: &[
            ("show-as", ""),
            ("side-by-side", "horizontal"),
            ("option-label", "option-label"),
            ("options", "options"),
        ],
        children: &["option", "preset", "on-change", "list-from"],
    },
    Item {
        word: "choices",
        v1: "checklist",
        first_arg: true,
        props: &[("option-label", "option-label"), ("options", "options")],
        children: &["option", "on-change", "list-from"],
    },
    Item {
        word: "tick-box",
        v1: "checkbox",
        first_arg: true,
        props: &[],
        children: &["on-change"],
    },
    Item {
        word: "number",
        v1: "number",
        first_arg: true,
        props: &[
            ("show-as", ""),
            ("min", "min"),
            ("max", "max"),
            ("step", "step"),
            ("decimals", "decimals"),
            ("suffix", "suffix"),
        ],
        children: &["on-change"],
    },
    Item {
        word: "text",
        v1: "text",
        first_arg: true,
        props: &[
            ("long", "multiline"),
            ("lines", "lines"),
            ("hidden", "password"),
            ("hint", "placeholder"),
            ("pattern", "pattern"),
            ("pattern-help", "pattern-message"),
        ],
        children: &["on-change"],
    },
    Item {
        word: "file",
        v1: "file",
        first_arg: true,
        props: &[
            ("extensions", "filter"),
            ("new", "save"),
            ("must-exist", "must-exist"),
            ("relative", "relative"),
            ("hint", "placeholder"),
        ],
        children: &["on-change"],
    },
    Item {
        word: "folder",
        v1: "folder",
        first_arg: true,
        props: &[
            ("must-exist", "must-exist"),
            ("relative", "relative"),
            ("hint", "placeholder"),
        ],
        children: &["on-change"],
    },
    Item {
        word: "display",
        v1: "display",
        first_arg: true,
        props: &[("text", "text"), ("copyable", "copyable")],
        children: &[],
    },
    Item {
        word: "button",
        v1: "button",
        first_arg: true,
        props: &[("ask-first", "confirm")],
        children: &[],
    },
];

/// A control's kind, as people say it: "a folder", "a drop-down".
fn control_kind(word: &str) -> &'static str {
    match word {
        "choice" => "a drop-down",
        "choices" => "a set of tick boxes",
        "tick-box" => "a tick box",
        "number" => "a number",
        "text" => "text",
        "file" => "a file",
        "folder" => "a folder",
        "button" => "a button",
        _ => "something to show",
    }
}

/// Which items are controls (they take the common control properties).
fn is_control(word: &str) -> bool {
    matches!(
        word,
        "choice"
            | "choices"
            | "tick-box"
            | "number"
            | "text"
            | "file"
            | "folder"
            | "display"
            | "button"
    )
}

const TOP: &[&str] = &[
    "tool", "window", "steps", "value", "data", "script", "add-on", "example",
];
const WINDOW_PROPS: &[&str] = &[
    "width",
    "height",
    "resizable",
    "theme",
    "label-width",
    "field-width",
    "position",
];
const STEPS_PROPS: &[(&str, &str)] =
    &[("when-done", "after"), ("lock", "lock"), ("label", "label")];
const STEP_PROPS: &[(&str, &str)] = &[
    ("label", "label"),
    ("if-wrong", "on-error"),
    ("problem", "message"),
    ("if", "if"),
];

struct Tx<'a> {
    lines: LineIndex<'a>,
    diags: Vec<Diagnostic>,
    library: Library,
    /// `steps` blocks by id, for buttons.
    steps_ids: Vec<String>,
    /// The `put` steps that follow the step just read.
    after: Vec<KdlNode>,
    /// The window's controls by name, for `put … into=`.
    control_ids: Vec<String>,
    /// The window's controls, for a setting that reads one.
    controls: Vec<WindowControl>,
    /// The steps read so far in this `steps` block: (name, step, results), for a setting that
    /// uses what one found.
    found: Vec<(String, String, Vec<crate::addons::ResultDef>)>,
    /// The `steps` block being read.
    block: String,
    /// Every step in the tool: (name, its block, what it's called), to say why a result it
    /// gives can't be used somewhere.
    all_steps: Vec<(String, String, String)>,
    /// The OS the tool says it's for (`tool … os=`).
    tool_os: Option<String>,
    /// The window's buttons: (name, label).
    buttons: Vec<(String, String)>,
}

/// Translate a format-2 tool.
pub(crate) fn translate(src: &str, doc: &KdlDocument, opts: &LoadOptions) -> Translated {
    // Carried add-ons first: the tool's steps may come from them.
    let mut carried = Vec::new();
    let mut diags = Vec::new();
    for n in doc.nodes().iter().filter(|n| n.name().value() == "add-on") {
        let (a, d) = addons::parse_addon_node(src, n, Origin::Carried, opts);
        diags.extend(d);
        if let Some(a) = a {
            carried.push(a);
        }
    }
    let library = Library::current().with_carried(carried);
    let carried: Vec<Arc<AddOn>> = library
        .addons
        .iter()
        .filter(|a| a.origin == Origin::Carried)
        .cloned()
        .collect();
    let mut tx = Tx {
        lines: LineIndex::new(src),
        diags,
        library,
        steps_ids: doc
            .nodes()
            .iter()
            .filter(|n| n.name().value() == "steps")
            .filter_map(first_string)
            .collect(),
        after: Vec::new(),
        control_ids: window_controls(doc).into_iter().map(|c| c.id).collect(),
        controls: window_controls(doc),
        found: Vec::new(),
        block: String::new(),
        all_steps: Vec::new(),
        tool_os: doc
            .nodes()
            .iter()
            .find(|n| n.name().value() == "tool")
            .and_then(|n| n.get("os"))
            .and_then(|v| v.as_string())
            .map(str::to_string),
        buttons: window_buttons(doc),
    };
    tx.index_steps(doc);
    let out = tx.top(doc);
    tx.empty_choices(doc);
    tx.control_starts();
    Translated {
        doc: out,
        diags: tx.diags,
        library: tx.library,
        carried,
    }
}

/// The window's buttons, (name, label), in rows, sections and pages too.
fn window_buttons(doc: &KdlDocument) -> Vec<(String, String)> {
    fn walk(d: &KdlDocument, out: &mut Vec<(String, String)>) {
        for n in d.nodes() {
            match n.name().value() {
                "row" | "section" | "pages" | "page" => {
                    if let Some(c) = n.children() {
                        walk(c, out);
                    }
                }
                "button" => {
                    if let (Some(id), Some(l)) = (
                        first_string(n),
                        n.entry("label").and_then(|e| e.value().as_string()),
                    ) {
                        out.push((id, l.to_string()));
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for w in doc.nodes().iter().filter(|n| n.name().value() == "window") {
        if let Some(c) = w.children() {
            walk(c, &mut out);
        }
    }
    out
}

/// A window control: its name, kind word, label, and (a file) the extensions it takes.
struct WindowControl {
    id: String,
    word: String,
    label: String,
    extensions: Vec<String>,
    /// The options written in a drop-down or tick boxes (none when a step fills them).
    options: Vec<String>,
    node: KdlNode,
}

/// The window's controls, in rows, sections and pages too.
fn window_controls(doc: &KdlDocument) -> Vec<WindowControl> {
    fn walk(d: &KdlDocument, out: &mut Vec<WindowControl>) {
        for n in d.nodes() {
            let w = n.name().value();
            if matches!(w, "row" | "section" | "pages" | "page") {
                if let Some(c) = n.children() {
                    walk(c, out);
                }
            } else if is_control(w) {
                if let Some(id) = first_string(n) {
                    let label = n
                        .get("label")
                        .and_then(|v| v.as_string())
                        .map_or_else(|| id.clone(), str::to_string);
                    let extensions = n
                        .get("extensions")
                        .and_then(|v| v.as_string())
                        .map(crate::paths::extensions)
                        .unwrap_or_default();
                    let options = children(n)
                        .filter(|o| o.name().value() == "option")
                        .filter_map(first_string)
                        .collect();
                    out.push(WindowControl {
                        id,
                        word: w.to_string(),
                        label,
                        extensions,
                        options,
                        node: n.clone(),
                    });
                }
            }
        }
    }
    let mut out = Vec::new();
    for w in doc.nodes().iter().filter(|n| n.name().value() == "window") {
        if let Some(c) = w.children() {
            walk(c, &mut out);
        }
    }
    out
}

fn first_string(n: &KdlNode) -> Option<String> {
    n.entries()
        .iter()
        .find(|e| e.name().is_none())
        .and_then(|e| e.value().as_string())
        .map(str::to_string)
}

fn args(n: &KdlNode) -> Vec<&KdlEntry> {
    n.entries().iter().filter(|e| e.name().is_none()).collect()
}

fn props(n: &KdlNode) -> impl Iterator<Item = (&str, &KdlEntry)> {
    n.entries()
        .iter()
        .filter_map(|e| Some((e.name()?.value(), e)))
}

fn renamed(e: &KdlEntry, name: &str) -> KdlEntry {
    let mut e = e.clone();
    e.set_name(Some(name));
    e
}

fn positional(e: &KdlEntry) -> KdlEntry {
    let mut e = e.clone();
    e.set_name(None::<String>);
    e
}

fn children(n: &KdlNode) -> impl Iterator<Item = &KdlNode> {
    n.children().into_iter().flat_map(|d| d.nodes().iter())
}

impl Tx<'_> {
    fn error(&mut self, code: &'static str, message: String, help: Option<String>, span: Span) {
        let (line, col) = self.lines.line_col(span.offset);
        self.diags.push(Diagnostic {
            severity: Severity::Error,
            code,
            message,
            help,
            offset: span.offset,
            len: span.len,
            line,
            col,
            path: None,
        });
    }

    fn warning(&mut self, code: &'static str, message: String, help: Option<String>, span: Span) {
        let (line, col) = self.lines.line_col(span.offset);
        self.diags.push(Diagnostic {
            severity: Severity::Warning,
            code,
            message,
            help,
            offset: span.offset,
            len: span.len,
            line,
            col,
            path: None,
        });
    }

    /// Every step in the tool, for problems about where a result comes from.
    fn index_steps(&mut self, doc: &KdlDocument) {
        for b in doc.nodes().iter().filter(|n| n.name().value() == "steps") {
            let Some(block) = first_string(b) else {
                continue;
            };
            let steps = children(b).flat_map(|c| {
                if c.name().value() == "before-starting" {
                    children(c).collect::<Vec<_>>()
                } else {
                    vec![c]
                }
            });
            for st in steps {
                let Some(id) = first_string(st) else { continue };
                let name = st
                    .get("label")
                    .and_then(|v| v.as_string())
                    .map(str::to_string)
                    .or_else(|| self.library.step(st.name().value()).map(|d| d.name.clone()))
                    .unwrap_or_else(|| st.name().value().to_string());
                self.all_steps.push((id, block.clone(), name));
            }
        }
    }

    /// A control's starting value is one it can hold: a number, one of its options, a file of
    /// the kinds it takes.
    fn control_starts(&mut self) {
        let mut problems = Vec::new();
        for c in &self.controls {
            let Some(e) = c.node.entry("starts") else {
                continue;
            };
            let Some(v) = e.value().as_string().map(str::trim) else {
                continue;
            };
            if v.is_empty() || v.contains('{') {
                continue;
            }
            let wrong = match c.word.as_str() {
                "number" if v.parse::<f64>().is_err() => Some("isn't a number".to_string()),
                "choice" if !c.options.is_empty() && !c.options.iter().any(|o| o == v) => Some(
                    format!("isn't one of its options ({})", c.options.join(", ")),
                ),
                "choices" if !c.options.is_empty() => v
                    .split(',')
                    .map(str::trim)
                    .find(|x| !x.is_empty() && !c.options.iter().any(|o| o == x))
                    .map(|x| format!("has \"{x}\", which isn't one of its options")),
                "file" if !crate::paths::has_extension(v, &c.extensions) => Some(format!(
                    "isn't a {} file",
                    crate::paths::extension_words(&c.extensions)
                )),
                _ => None,
            };
            if let Some(w) = wrong {
                problems.push((
                    format!("\"{}\" starts as \"{v}\", which {w}", c.label),
                    Self::entry_span(e),
                ));
            }
        }
        for (m, span) in problems {
            self.error(
                "bad-value",
                m,
                Some("change what it starts as".into()),
                span,
            );
        }
    }

    /// A list or tick boxes with no choices, that no step fills (`put … into=`).
    fn empty_choices(&mut self, doc: &KdlDocument) {
        fn puts_into(d: &KdlDocument, out: &mut Vec<String>) {
            for n in d.nodes() {
                if n.name().value() == "put" {
                    if let Some(c) = n.entry("into").and_then(|e| e.value().as_string()) {
                        out.push(c.to_string());
                    }
                }
                if let Some(c) = n.children() {
                    puts_into(c, out);
                }
            }
        }
        fn choices(d: &KdlDocument, out: &mut Vec<KdlNode>) {
            for n in d.nodes() {
                match n.name().value() {
                    "choice" | "choices" => out.push(n.clone()),
                    "row" | "section" | "pages" | "page" => {
                        if let Some(c) = n.children() {
                            choices(c, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut filled = Vec::new();
        let mut found = Vec::new();
        for n in doc.nodes() {
            match n.name().value() {
                "steps" => {
                    if let Some(c) = n.children() {
                        puts_into(c, &mut filled);
                    }
                }
                "window" => {
                    if let Some(c) = n.children() {
                        choices(c, &mut found);
                    }
                }
                _ => {}
            }
        }
        for c in found {
            let has = children(&c).any(|k| matches!(k.name().value(), "option" | "list-from"))
                || c.entry("options").is_some();
            let id = first_string(&c).unwrap_or_default();
            if !has && !filled.contains(&id) {
                let label = c
                    .entry("label")
                    .and_then(|e| e.value().as_string())
                    .unwrap_or(&id)
                    .to_string();
                self.warning(
                    "no-choices",
                    format!("\"{label}\" has no choices yet"),
                    Some("add some, or let a step fill it (a step's \"Put it into\")".into()),
                    Self::node_span(&c),
                );
            }
        }
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

    fn unknown_node(&mut self, n: &KdlNode, here: &str, allowed: &[&str]) {
        let name = n.name().value();
        let help = did_you_mean(name, allowed.iter().copied())
            .map(|m| format!("did you mean `{m}`?"))
            .or_else(|| {
                (!allowed.is_empty()).then(|| format!("{here} can hold: {}", allowed.join(", ")))
            });
        self.error(
            "unknown-node",
            format!("`{name}` doesn't belong {here}"),
            help,
            Self::node_span(n),
        );
    }

    fn unknown_prop(&mut self, e: &KdlEntry, what: &str, allowed: &[&str]) {
        let k = e.name().map(|k| k.value()).unwrap_or_default();
        let help = did_you_mean(k, allowed.iter().copied())
            .map(|m| format!("did you mean `{m}`?"))
            .or_else(|| {
                (!allowed.is_empty()).then(|| format!("`{what}` takes: {}", allowed.join(", ")))
            });
        self.error(
            "unknown-property",
            format!("`{what}` doesn't have a `{k}`"),
            help,
            Self::entry_span(e),
        );
    }

    // ---- the file ---------------------------------------------------------------------------

    fn top(&mut self, doc: &KdlDocument) -> KdlDocument {
        let mut out = KdlDocument::new();
        let mut window_props: Vec<KdlEntry> = Vec::new();
        let mut layout = KdlNode::new("layout");
        let mut title = None;
        let mut seen_window = false;
        for n in doc.nodes() {
            match n.name().value() {
                "tool" => {
                    let mut h = KdlNode::new("launcher");
                    h.set_span(n.span());
                    title = args(n).first().map(|e| (*e).clone());
                    for (k, e) in props(n) {
                        match k {
                            "format" => {}
                            "requires" | "os" | "save-logs" => h.push(e.clone()),
                            _ => self.unknown_prop(
                                e,
                                "tool",
                                &["format", "requires", "os", "save-logs"],
                            ),
                        }
                    }
                    out.nodes_mut().push(h);
                }
                "window" => {
                    if seen_window {
                        self.error(
                            "two-windows",
                            "A tool has one `window`".into(),
                            None,
                            Self::node_span(n),
                        );
                        continue;
                    }
                    seen_window = true;
                    for (k, e) in props(n) {
                        if WINDOW_PROPS.contains(&k) {
                            window_props.push(e.clone());
                        } else {
                            self.unknown_prop(e, "window", WINDOW_PROPS);
                        }
                    }
                    layout.set_span(n.span());
                    let items = self.items(n, "in the window");
                    layout.set_children(items);
                }
                "steps" => {
                    if let Some(a) = self.steps(n) {
                        out.nodes_mut().push(a);
                    }
                }
                "value" | "data" | "script" => out.nodes_mut().push(n.clone()),
                "add-on" => {}
                // What "Make a new tool" says about this tool when it's offered as an example.
                "example" => {
                    for (k, e) in props(n) {
                        if !["name", "about", "suggest"].contains(&k) {
                            self.unknown_prop(e, "example", &["name", "about", "suggest"]);
                        }
                    }
                }
                _ => self.unknown_node(n, "at the top of a tool", TOP),
            }
        }
        // The window: its title is the tool's name.
        let mut w = KdlNode::new("window");
        if let Some(mut t) = title {
            t.set_name(Some("title"));
            w.push(t);
        }
        for e in window_props {
            w.push(e);
        }
        out.nodes_mut().push(w);
        // A tool with steps but no button gets a Start button.
        let has_button = layout
            .children()
            .is_some_and(|d| contains_kind(d, "button"));
        if !has_button {
            if let Some(id) = self.steps_ids.first().cloned() {
                let mut b = KdlNode::new("button");
                b.push(KdlEntry::new("start"));
                b.push(KdlEntry::new_prop("label", "Start"));
                b.push(KdlEntry::new_prop("action", id));
                b.push(KdlEntry::new_prop("width", "fill"));
                layout.ensure_children().nodes_mut().push(b);
            }
        }
        out.nodes_mut().push(layout);
        out
    }

    // ---- the window ---------------------------------------------------------------------------

    fn items(&mut self, parent: &KdlNode, here: &str) -> KdlDocument {
        let mut out = KdlDocument::new();
        let words: Vec<&str> = ITEMS
            .iter()
            .filter(|i| i.word != "page")
            .map(|i| i.word)
            .collect();
        for n in children(parent) {
            let word = n.name().value();
            let Some(item) = ITEMS.iter().find(|i| i.word == word && word != "page") else {
                self.unknown_node(n, here, &words);
                continue;
            };
            if let Some(t) = self.item(n, item) {
                out.nodes_mut().push(t);
            }
        }
        out
    }

    fn item(&mut self, n: &KdlNode, item: &Item) -> Option<KdlNode> {
        let word = item.word;
        let mut v1 = item.v1;
        // `show-as` picks the loader's kind.
        if let Some(e) = n.entry("show-as") {
            let s = e.value().as_string().unwrap_or_default();
            v1 = match (word, s) {
                ("choice", "list") => "dropdown",
                ("choice", "buttons") => "radio",
                ("number", "box") => "number",
                ("number", "slider") => "slider",
                _ => {
                    let allowed: &[&str] = match word {
                        "choice" => &["list", "buttons"],
                        _ => &["box", "slider"],
                    };
                    self.error(
                        "bad-value",
                        format!("`show-as` is one of: {}", allowed.join(", ")),
                        did_you_mean(s, allowed.iter().copied())
                            .map(|m| format!("did you mean \"{m}\"?")),
                        Self::entry_span(e),
                    );
                    item.v1
                }
            };
        }
        let mut t = KdlNode::new(v1);
        t.set_span(n.span());
        if item.first_arg {
            match args(n).first() {
                Some(a) => t.push(positional(a)),
                None if matches!(word, "section" | "line") => {}
                None => {
                    let what = if is_control(word) {
                        "a name"
                    } else {
                        "its text"
                    };
                    self.error(
                        "missing-name",
                        format!("`{word}` needs {what}, like `{word} \"…\"`"),
                        None,
                        Self::node_span(n),
                    );
                    return None;
                }
            }
        }
        let mut allowed: Vec<&str> = item.props.iter().map(|(w, _)| *w).collect();
        if is_control(word) {
            allowed.extend(CONTROL.iter().map(|(w, _)| *w));
        }
        for (k, e) in props(n) {
            let mapped = item
                .props
                .iter()
                .chain(if is_control(word) { CONTROL } else { &[] })
                .find(|(w, _)| *w == k)
                .map(|(_, v)| *v);
            match mapped {
                Some("") => {}
                Some(v) => t.push(renamed(e, v)),
                None => self.unknown_prop(e, word, &allowed),
            }
        }
        // Controls stretch with the window (the form counts them at their smallest).
        if is_control(word) && n.entry("width").is_none() {
            t.push(KdlEntry::new_prop("width", "fill"));
        }
        if word == "button" {
            let id = first_string(n).unwrap_or_default();
            if self.steps_ids.contains(&id) {
                t.push(KdlEntry::new_prop("action", id.clone()));
            } else {
                let help = did_you_mean(&id, self.steps_ids.iter().map(String::as_str))
                    .map(|m| format!("did you mean `{m}`?"))
                    .or_else(|| Some(format!("add `steps \"{id}\" {{ }}` for it")));
                self.error(
                    "button-without-steps",
                    format!("The button \"{id}\" has no steps"),
                    help,
                    Self::node_span(n),
                );
                return None;
            }
        }
        // Children.
        let mut kids = KdlDocument::new();
        if matches!(word, "row" | "section" | "page") {
            kids = self.items(n, &format!("in a `{word}`"));
        } else if word == "pages" {
            for c in children(n) {
                if c.name().value() != "page" {
                    self.unknown_node(c, "in `pages`", &["page"]);
                    continue;
                }
                let page = ITEMS.iter().find(|i| i.word == "page").expect("page");
                if let Some(p) = self.item(c, page) {
                    kids.nodes_mut().push(p);
                }
            }
        } else {
            for c in children(n) {
                let cw = c.name().value();
                if !item.children.contains(&cw) {
                    self.unknown_node(c, &format!("in a `{word}`"), item.children);
                    continue;
                }
                match cw {
                    "list-from" => kids.nodes_mut().push(self.with_node(c, "list-from")),
                    "on-change" => {
                        let mut oc = KdlNode::new("on-change");
                        oc.set_span(c.span());
                        oc.set_children(self.step_list(c, false));
                        kids.nodes_mut().push(oc);
                    }
                    // option and preset are written the same way in both.
                    _ => kids.nodes_mut().push(c.clone()),
                }
            }
        }
        if !kids.nodes().is_empty() {
            t.set_children(kids);
        }
        Some(t)
    }

    /// `list-from "git.branches" folder="."` → `list-from "git.branches" { with "folder" "." }`.
    fn with_node(&mut self, n: &KdlNode, name: &str) -> KdlNode {
        let mut t = KdlNode::new(name);
        t.set_span(n.span());
        if let Some(a) = args(n).first() {
            t.push(positional(a));
        }
        let mut kids = KdlDocument::new();
        for (k, e) in props(n) {
            let mut w = KdlNode::new("with");
            w.push(KdlEntry::new(k));
            w.push(positional(e));
            kids.nodes_mut().push(w);
        }
        if !kids.nodes().is_empty() {
            t.set_children(kids);
        }
        t
    }

    // ---- steps ----------------------------------------------------------------------------

    fn steps(&mut self, n: &KdlNode) -> Option<KdlNode> {
        let Some(id) = first_string(n) else {
            self.error(
                "missing-name",
                "`steps` needs the name of its button, like `steps \"build\"`".into(),
                None,
                Self::node_span(n),
            );
            return None;
        };
        let mut a = KdlNode::new("action");
        a.set_span(n.span());
        a.push(KdlEntry::new(id.clone()));
        let allowed: Vec<&str> = STEPS_PROPS.iter().map(|(w, _)| *w).collect();
        for (k, e) in props(n) {
            match STEPS_PROPS.iter().find(|(w, _)| *w == k) {
                Some(("when-done", v)) => {
                    let mut e = renamed(e, v);
                    if e.value().as_string() == Some("close") {
                        e.set_value("exit");
                    }
                    a.push(e);
                }
                Some((_, v)) => a.push(renamed(e, v)),
                None => self.unknown_prop(e, "steps", &allowed),
            }
        }
        // The run is called what its button says ("Get list of all branches").
        if n.entry("label").is_none() {
            if let Some(l) = self.button_label(&id) {
                a.push(KdlEntry::new_prop("label", l));
            }
        }
        // A run starts with nothing found: earlier blocks are other buttons' runs.
        self.found.clear();
        self.block = id.clone();
        a.set_children(self.step_list(n, true));
        Some(a)
    }

    /// The steps in a block; `before` allows a `before-starting { }` block.
    fn step_list(&mut self, n: &KdlNode, before: bool) -> KdlDocument {
        let mut out = KdlDocument::new();
        for c in children(n) {
            if before && c.name().value() == "before-starting" {
                for s in children(c) {
                    if let Some(t) = self.step(s) {
                        for mut t in std::iter::once(t).chain(std::mem::take(&mut self.after)) {
                            t.push(KdlEntry::new_prop("first", true));
                            out.nodes_mut().push(t);
                        }
                    }
                }
                continue;
            }
            if let Some(t) = self.step(c) {
                out.nodes_mut().push(t);
                out.nodes_mut().append(&mut self.after);
            }
        }
        out
    }

    /// One step: `git.pull "pull" folder="." { only-when … }`.
    fn step(&mut self, n: &KdlNode) -> Option<KdlNode> {
        let name = n.name().value().to_string();
        let Some(def) = self.library.step(&name) else {
            let (addon, _) = name.split_once('.').unwrap_or((name.as_str(), ""));
            let help = if !name.contains('.') {
                let lib = &self.library;
                did_you_mean(&name, lib.steps().map(|s| s.id.as_str()))
                    .map(|m| format!("did you mean `{m}`?"))
                    .or_else(|| {
                        lib.steps()
                            .find(|s| s.id.ends_with(&format!(".{name}")))
                            .map(|s| format!("steps are written with their add-on: `{}`", s.id))
                    })
            } else if self.library.addon(addon).is_some() {
                did_you_mean(&name, self.library.steps().map(|s| s.id.as_str()))
                    .map(|m| format!("did you mean `{m}`?"))
            } else {
                Some(format!(
                    "this needs the \"{addon}\" add-on, which isn't on this computer: put it in the Add-ons folder, or ask whoever made the tool to include it"
                ))
            };
            self.error(
                "unknown-step",
                format!("There's no step called \"{name}\""),
                help,
                Self::node_span(n),
            );
            return None;
        };
        // A step for another OS than the tool's can never run in it.
        if let Some(tool_os) = self.tool_os.clone() {
            for need in &def.needs {
                if let crate::addons::Need::Os(os) = need {
                    let (word, name) = match os {
                        crate::config::catalog::Os::Windows => ("windows", "Windows"),
                        crate::config::catalog::Os::Macos => ("macos", "macOS"),
                        crate::config::catalog::Os::Any => continue,
                    };
                    if tool_os != word {
                        self.error(
                            "wrong-os",
                            format!(
                                "\"{}\" only works on {name}, and this tool is for {}",
                                def.name,
                                if tool_os == "windows" {
                                    "Windows"
                                } else {
                                    "macOS"
                                }
                            ),
                            Some("take the step out, or choose another".into()),
                            Self::node_span(n),
                        );
                    }
                }
            }
        }
        let id = args(n).first().map(|e| (*e).clone());
        if args(n).len() > 1 {
            self.error(
                "extra-value",
                "A step takes one name, then its settings by name".into(),
                Some(format!(
                    "write settings like `{}=\"…\"`",
                    def.settings.first().map_or("setting", |s| s.id.as_str())
                )),
                Self::entry_span(args(n)[1]),
            );
        }
        // Common step properties, and settings given as properties or children.
        let mut common: Vec<KdlEntry> = Vec::new();
        let mut given: Vec<(String, Vec<KdlEntry>, Span)> = Vec::new();
        let mut passthrough: Vec<KdlNode> = Vec::new();
        let mut puts: Vec<KdlNode> = Vec::new();
        for (k, e) in props(n) {
            if let Some((_, v)) = STEP_PROPS.iter().find(|(w, _)| *w == k) {
                let mut e = renamed(e, v);
                if k == "if-wrong" {
                    let word = e.value().as_string().unwrap_or_default().to_string();
                    let v1 = match word.as_str() {
                        "stop" => "abort",
                        "ask" => "warn",
                        "keep-going" => "continue",
                        _ => {
                            self.error(
                                "bad-value",
                                "`if-wrong` is one of: stop, ask, keep-going".into(),
                                did_you_mean(&word, ["stop", "ask", "keep-going"])
                                    .map(|m| format!("did you mean \"{m}\"?")),
                                Self::entry_span(&e),
                            );
                            "abort"
                        }
                    };
                    e.set_value(v1);
                }
                common.push(e);
            } else if def.setting(k).is_some() {
                given.push((k.to_string(), vec![positional(e)], Self::entry_span(e)));
            } else {
                self.unknown_setting(&def, k, Self::entry_span(e));
            }
        }
        for c in children(n) {
            let cw = c.name().value();
            if cw == "put" {
                if let Some(p) = self.put(c, &def, id.as_ref().and_then(|e| e.value().as_string()))
                {
                    puts.push(p);
                }
            } else if def.setting(cw).is_some() {
                given.push((
                    cw.to_string(),
                    args(c).into_iter().map(positional).collect(),
                    Self::node_span(c),
                ));
            } else if matches!(def.body, Body::BuiltIn(spec) if catalog::allowed(spec.children).contains(&cw))
            {
                passthrough.push(c.clone());
            } else {
                self.unknown_setting(&def, cw, Self::node_span(c));
            }
        }
        // A setting that reads a control reads one of its kind: a folder setting, a folder.
        for (k, vals, span) in &given {
            let (Some(s), [v]) = (def.setting(k), vals.as_slice()) else {
                continue;
            };
            // A value typed in: one the setting can take.
            if let Some(t) = v.value().as_string().filter(|t| !t.contains('{')) {
                if let Some(problem) = s.check_value(t) {
                    self.error(
                        "bad-value",
                        format!("\"{}\" for \"{}\" {problem}", s.label, def.name),
                        None,
                        *span,
                    );
                }
                continue;
            }
            let Some(name) = v
                .value()
                .as_string()
                .and_then(|t| t.strip_prefix('{')?.strip_suffix('}'))
            else {
                continue;
            };
            let Some(c) = self.controls.iter().find(|c| c.id == name) else {
                continue;
            };
            if !s.takes(&c.word, &c.extensions, &c.options) {
                // Of its kind, but for other files or with other options: say which.
                let of_kind = s.kind.takes(&c.word);
                let is = if of_kind && c.word == "file" {
                    if c.extensions.is_empty() {
                        "for any file".to_string()
                    } else {
                        format!("for {} files", crate::paths::extension_words(&c.extensions))
                    }
                } else if of_kind {
                    format!("{} with other options", control_kind(&c.word))
                } else {
                    control_kind(&c.word).to_string()
                };
                let help = if of_kind && c.word == "file" {
                    format!(
                        "set \"{}\"'s extensions to {}, or choose another control with Use…",
                        c.label,
                        s.extensions.join(", ")
                    )
                } else if of_kind {
                    format!(
                        "give \"{}\" only these options: {}, or choose another control with Use…",
                        c.label,
                        s.options.join(", ")
                    )
                } else {
                    format!("choose {} with Use…, or set it on the step", s.needs())
                };
                self.error(
                    "wrong-kind",
                    format!(
                        "\"{}\" for \"{}\" needs {}, but \"{}\" is {is}",
                        s.label,
                        def.name,
                        s.needs(),
                        c.label
                    ),
                    Some(help),
                    *span,
                );
            }
        }
        // A setting that uses what an earlier step found takes only a result it can: a folder
        // setting, not a count.
        for (k, vals, span) in &given {
            let (Some(s), [v]) = (def.setting(k), vals.as_slice()) else {
                continue;
            };
            let Some((step, result)) = v
                .value()
                .as_string()
                .and_then(|t| t.strip_prefix('{')?.strip_suffix('}'))
                .and_then(|t| t.split_once('.'))
            else {
                continue;
            };
            let Some((_, step_name, results)) = self.found.iter().find(|(id, _, _)| id == step)
            else {
                continue;
            };
            let Some(r) = results.iter().find(|r| r.id == result) else {
                continue;
            };
            if !s.takes_result(r.kind) {
                // What it could use instead, from the steps before it.
                let instead = self.found.iter().find_map(|(_, name, rs)| {
                    rs.iter()
                        .find(|r| s.takes_result(r.kind))
                        .map(|r| format!("use \"{}\" from \"{name}\" instead", r.label))
                });
                self.error(
                    "wrong-kind",
                    format!(
                        "\"{}\" for \"{}\" needs {}, but \"{}\" from \"{step_name}\" is {}",
                        s.label,
                        def.name,
                        s.needs(),
                        r.label,
                        r.kind.words()
                    ),
                    Some(instead.unwrap_or_else(|| {
                        format!("choose {} with Use…, or set it on the step", s.needs())
                    })),
                    *span,
                );
            }
        }
        // A result is used after its step has run, in the same button's steps.
        let own = id
            .as_ref()
            .and_then(|e| e.value().as_string())
            .unwrap_or_default()
            .to_string();
        for (k, e) in props(n) {
            if k == "if" {
                continue;
            }
            let Some(t) = e.value().as_string().and_then(|t| Template::parse(t).ok()) else {
                continue;
            };
            for (name, field, _) in t.refs() {
                if name == own || self.found.iter().any(|(id, _, _)| id == name) {
                    continue;
                }
                let Some((_, block, step_name)) =
                    self.all_steps.iter().find(|(id, _, _)| id == name).cloned()
                else {
                    continue;
                };
                let what = field.map_or_else(|| "what it found".into(), |f| format!("\"{f}\""));
                let (message, help) = if block == self.block {
                    (
                        format!(
                            "\"{}\" uses {what} from \"{step_name}\", which runs after it",
                            def.name
                        ),
                        format!("move it below \"{step_name}\""),
                    )
                } else {
                    (
                        format!(
                            "\"{}\" uses {what} from \"{step_name}\", one of another button's steps",
                            def.name
                        ),
                        "put it into a control (that step's \"Put it into\"), and use the control"
                            .into(),
                    )
                };
                self.error("wrong-order", message, Some(help), Self::entry_span(e));
            }
        }
        if let Some(step_id) = id.as_ref().and_then(|e| e.value().as_string()) {
            let name = n
                .get("label")
                .and_then(|v| v.as_string())
                .unwrap_or(&def.name)
                .to_string();
            self.found
                .push((step_id.to_string(), name, def.results.clone()));
        }
        let mut complete = true;
        for s in &def.settings {
            if s.required && s.starts.is_none() && !given.iter().any(|(k, _, _)| *k == s.id) {
                self.error(
                    "missing-setting",
                    format!("Fill in \"{}\" for \"{}\"", s.label, def.name),
                    Some(format!("add {}=\"…\"", s.id)),
                    Self::node_span(n),
                );
                complete = false;
            }
        }
        // A step with a problem already reported isn't passed on (no second, vaguer message).
        if !complete {
            return None;
        }
        let mut t = match def.body {
            Body::BuiltIn(spec) => self.builtin(spec, &def, given),
            _ => {
                let mut t = KdlNode::new("use");
                t.push(KdlEntry::new(def.id.clone()));
                let mut kids = KdlDocument::new();
                for (k, vals, _) in given {
                    let mut w = KdlNode::new("with");
                    w.push(KdlEntry::new(k));
                    if let Some(v) = vals.into_iter().next() {
                        w.push(v);
                    }
                    kids.nodes_mut().push(w);
                }
                if !kids.nodes().is_empty() {
                    t.set_children(kids);
                }
                t
            }
        };
        t.set_span(n.span());
        if let Some(mut id) = id {
            id.set_name(Some("id"));
            t.push(id);
        }
        for e in common {
            t.push(e);
        }
        if !passthrough.is_empty() {
            let kids = t.ensure_children();
            for p in passthrough {
                kids.nodes_mut().push(p);
            }
        }
        // A step that runs only when… puts only then.
        let only = t
            .entries()
            .iter()
            .find(|e| e.name().is_some_and(|k| k.value() == "if"))
            .cloned();
        for mut p in puts {
            if let Some(e) = &only {
                p.push(e.clone());
            }
            self.after.push(p);
        }
        Some(t)
    }

    /// `put "branches" into="select-branch"` on a step: the engine's `put` step that follows it.
    fn put(&mut self, c: &KdlNode, def: &StepDef, step: Option<&str>) -> Option<KdlNode> {
        let result = first_string(c);
        let into = c
            .entry("into")
            .and_then(|e| e.value().as_string())
            .map(str::to_string);
        for (k, e) in props(c) {
            if k != "into" {
                self.unknown_prop(e, "put", &["into"]);
            }
        }
        let Some(result) = result.filter(|r| def.results.iter().any(|x| x.id == *r)) else {
            let names: Vec<&str> = def.results.iter().map(|r| r.id.as_str()).collect();
            let help = if names.is_empty() {
                Some(format!("\"{}\" doesn't give anything back", def.name))
            } else {
                Some(format!("it gives back: {}", names.join(", ")))
            };
            self.error(
                "unknown-result",
                format!("\"{}\" has no result to put there", def.name),
                help,
                Self::node_span(c),
            );
            return None;
        };
        let Some(into) = into else {
            self.error(
                "missing-into",
                "`put` needs the control it goes into".into(),
                Some(format!("write `put \"{result}\" into=\"…\"`")),
                Self::node_span(c),
            );
            return None;
        };
        if !self.control_ids.contains(&into) {
            let help = did_you_mean(&into, self.control_ids.iter().map(String::as_str))
                .map(|m| format!("did you mean \"{m}\"?"));
            self.error(
                "unknown-control",
                format!("There's no control called \"{into}\" to put it into"),
                help,
                Self::node_span(c),
            );
            return None;
        }
        // Only into a control that can hold it: a list into a drop-down's choices, a number into
        // a number.
        if let Some(r) = def.results.iter().find(|x| x.id == result) {
            if let Some(ctl) = self.controls.iter().find(|x| x.id == into) {
                if !r.kind.fills(&ctl.word) {
                    let message = format!(
                        "\"{}\" from \"{}\" is {}, and \"{}\" is {}: it can't go there",
                        r.label,
                        def.name,
                        r.kind.words(),
                        ctl.label,
                        control_kind(&ctl.word)
                    );
                    self.error(
                        "wrong-kind",
                        message,
                        Some("choose a control that can hold it with Put it into".into()),
                        Self::node_span(c),
                    );
                    return None;
                }
            }
        }
        let Some(step) = step else {
            self.error(
                "missing-name",
                "A step that puts its results somewhere needs a name".into(),
                Some(format!("write `{} \"a-name\"`", def.id)),
                Self::node_span(c),
            );
            return None;
        };
        let mut p = KdlNode::new("put");
        p.set_span(c.span());
        p.push(KdlEntry::new(into));
        p.push(KdlEntry::new(step));
        p.push(KdlEntry::new(result));
        Some(p)
    }

    fn button_label(&self, id: &str) -> Option<String> {
        self.buttons
            .iter()
            .find(|(b, _)| b == id)
            .map(|(_, l)| l.clone())
    }

    fn unknown_setting(&mut self, def: &StepDef, k: &str, span: Span) {
        let mut names: Vec<&str> = def.settings.iter().map(|s| s.id.as_str()).collect();
        names.extend(STEP_PROPS.iter().map(|(w, _)| *w));
        self.error(
            "unknown-setting",
            format!("\"{}\" has no setting \"{k}\"", def.name),
            did_you_mean(k, names.iter().copied()).map(|m| format!("did you mean \"{m}\"?")),
            span,
        );
    }

    /// A built-in step as the engine's own: settings by position, the rest as properties.
    fn builtin(
        &mut self,
        spec: &'static catalog::Spec,
        def: &StepDef,
        given: Vec<(String, Vec<KdlEntry>, Span)>,
    ) -> KdlNode {
        debug_assert_eq!(spec.cat, Category::Step);
        let mut t = KdlNode::new(spec.name);
        let mut positional_args: Vec<Option<KdlEntry>> = vec![None; spec.args.len()];
        let mut rest: Vec<KdlEntry> = Vec::new();
        let mut named: Vec<KdlEntry> = Vec::new();
        for (k, vals, _) in given {
            let Some(s) = def.setting(&k) else { continue };
            if let Some(i) = s.position {
                positional_args[i] = vals.into_iter().next();
            } else if s.rest {
                rest.extend(vals);
            } else if let Some(v) = vals.into_iter().next() {
                named.push(renamed(&v, &k));
            }
        }
        // A starting value the add-on changed from the engine's own.
        for s in &def.settings {
            if s.starts_overridden
                && !s.rest
                && s.position.is_none()
                && !named
                    .iter()
                    .any(|e| e.name().is_some_and(|n| n.value() == s.id))
            {
                let v = match &s.starts {
                    Some(crate::dynval::Dyn::Bool(b)) => kdl::KdlValue::Bool(*b),
                    Some(crate::dynval::Dyn::Num(n)) => kdl::KdlValue::Float(*n),
                    Some(d) => kdl::KdlValue::String(d.to_text()),
                    None => continue,
                };
                named.push(KdlEntry::new_prop(s.id.as_str(), v));
            }
        }
        // What a list in its text holds, worked out from the step that found it, for a count
        // ("Models to fix (54 models)").
        for p in spec.props.iter().filter(|p| p.filled) {
            let words = def
                .settings
                .iter()
                .filter(|s| s.text_lines())
                .filter_map(|s| {
                    named
                        .iter()
                        .find(|e| e.name().is_some_and(|n| n.value() == s.id))
                })
                .find_map(|e| self.result_of(e));
            let word = match (words, p.name) {
                (Some((one, _)), "item") => one,
                (Some((_, many)), "items") => many,
                _ => continue,
            };
            named.push(KdlEntry::new_prop(p.name, word));
        }
        // Positional arguments in order, as long as they are given.
        for a in positional_args.into_iter().map_while(|a| a) {
            t.push(a);
        }
        for r in rest {
            t.push(r);
        }
        for p in named {
            t.push(p);
        }
        t
    }
}

impl Tx<'_> {
    /// What a value that is just `{step.result}` holds, one and several, when an earlier step
    /// found it.
    fn result_of(&self, e: &KdlEntry) -> Option<(String, String)> {
        let (step, result) = e
            .value()
            .as_string()?
            .strip_prefix('{')?
            .strip_suffix('}')?
            .split_once('.')?;
        let (_, _, results) = self.found.iter().find(|(id, _, _)| id == step)?;
        results
            .iter()
            .find(|r| r.id == result)
            .map(|r| r.item.clone())
    }
}

fn contains_kind(doc: &KdlDocument, kind: &str) -> bool {
    doc.nodes()
        .iter()
        .any(|n| n.name().value() == kind || n.children().is_some_and(|d| contains_kind(d, kind)))
}

/// Whether a file is a format-2 tool: its first node is `tool`.
pub fn is_tool(doc: &KdlDocument) -> bool {
    doc.nodes()
        .first()
        .is_some_and(|n| n.name().value() == "tool")
}

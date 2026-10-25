//! Reading an add-on file: one `add-on { }` block, checked node by node with plain messages.

use std::path::Path;
use std::sync::Arc;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

use super::{
    AddOn, Body, Detect, ListDef, ListUse, Need, Origin, ResultDef, ResultKind, SettingDef,
    SettingKind, StepDef,
};
use crate::config::catalog::{self, Category, Os, Param, Ty};
use crate::config::load::Reader;
use crate::config::{Diagnostic, LoadOptions, Severity, Version};
use crate::dynval::Dyn;
use crate::suggest::did_you_mean;

/// Read an add-on file: its one `add-on { }` block.
pub fn parse_addon(
    src: &str,
    path: &Path,
    origin: Origin,
    opts: &LoadOptions,
) -> (Option<AddOn>, Vec<Diagnostic>) {
    let _ = path;
    let doc = match KdlDocument::parse_v2(src) {
        Ok(d) => d,
        Err(e) => {
            let lines = crate::config::diag::LineIndex::new(src);
            let diags =
                e.diagnostics
                    .iter()
                    .take(1)
                    .map(|d| {
                        let (line, col) = lines.line_col(d.span.offset());
                        Diagnostic {
                            severity: Severity::Error,
                            code: "syntax",
                            message: d.message.clone().unwrap_or_else(|| {
                                "the add-on file isn't written correctly".into()
                            }),
                            help: d.help.clone(),
                            offset: d.span.offset(),
                            len: d.span.len(),
                            line,
                            col,
                            path: None,
                        }
                    })
                    .collect();
            return (None, diags);
        }
    };
    let mut r = Reader::new(src, opts);
    let nodes: Vec<&KdlNode> = doc.nodes().iter().collect();
    let addon = match nodes.as_slice() {
        [n] if n.name().value() == "add-on" => parse_addon_with(&mut r, src, n, origin),
        _ => {
            let span = nodes
                .first()
                .map(|n| Reader::node_span(n))
                .unwrap_or_default();
            r.error(
                "not-an-add-on",
                "An add-on file holds one `add-on \"name\" { }` block and nothing else.".into(),
                None,
                span,
            );
            None
        }
    };
    let mut diags = r.finish();
    diags.sort_by_key(|d| d.offset);
    (addon, diags)
}

/// Read an `add-on { }` block from inside another file (a copy carried by a tool). `src` is that
/// whole file, so problems point at the right lines.
pub fn parse_addon_node<'a>(
    src: &'a str,
    node: &'a KdlNode,
    origin: Origin,
    opts: &LoadOptions,
) -> (Option<AddOn>, Vec<Diagnostic>) {
    let mut r = Reader::new(src, opts);
    let addon = parse_addon_with(&mut r, src, node, origin);
    (addon, r.finish())
}

// ---- checking a node's shape --------------------------------------------------------------------

fn string(e: &KdlEntry) -> Option<String> {
    e.value().as_string().map(str::to_string)
}

fn args(n: &KdlNode) -> Vec<&KdlEntry> {
    n.entries().iter().filter(|e| e.name().is_none()).collect()
}

fn prop<'a>(n: &'a KdlNode, name: &str) -> Option<&'a KdlEntry> {
    n.entries()
        .iter()
        .rev()
        .find(|e| e.name().is_some_and(|k| k.value() == name))
}

fn prop_str(n: &KdlNode, name: &str) -> Option<String> {
    prop(n, name).and_then(string)
}

fn prop_bool(n: &KdlNode, name: &str) -> bool {
    matches!(
        prop(n, name).map(KdlEntry::value),
        Some(KdlValue::Bool(true))
    )
}

/// Check that `n` only has these properties and children; report the rest with a hint.
fn shape(r: &mut Reader, n: &KdlNode, props: &[&str], children: &[&str]) {
    let what = n.name().value();
    for e in n.entries() {
        if let Some(k) = e.name() {
            let k = k.value();
            if !props.contains(&k) {
                let help = did_you_mean(k, props.iter().copied())
                    .map(|m| format!("did you mean `{m}`?"))
                    .or_else(|| {
                        (!props.is_empty()).then(|| format!("`{what}` takes: {}", props.join(", ")))
                    });
                r.error(
                    "unknown-property",
                    format!("`{what}` doesn't have a `{k}`"),
                    help,
                    Reader::entry_span(e),
                );
            }
        }
    }
    if let Some(doc) = n.children() {
        for c in doc.nodes() {
            let name = c.name().value();
            if !children.contains(&name) {
                let help = did_you_mean(name, children.iter().copied())
                    .map(|m| format!("did you mean `{m}`?"))
                    .or_else(|| {
                        (!children.is_empty())
                            .then(|| format!("`{what}` can hold: {}", children.join(", ")))
                    });
                r.error(
                    "unknown-node",
                    format!("`{name}` doesn't belong in `{what}`"),
                    help,
                    Reader::node_span(c),
                );
            }
        }
    }
}

fn first_string(r: &mut Reader, n: &KdlNode, what: &str) -> Option<String> {
    match args(n).first().and_then(|e| string(e)) {
        Some(s) => Some(s),
        None => {
            r.error(
                "missing-name",
                format!("`{}` needs {what}", n.name().value()),
                Some(format!("write it like `{} \"name\"`", n.name().value())),
                Reader::node_span(n),
            );
            None
        }
    }
}

/// Names are lower case words joined with `-`: they become `{name}` in templates and the
/// `add-on.step` names in tools.
fn check_name(r: &mut Reader, n: &KdlNode, name: &str) -> bool {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    if !ok {
        r.error(
            "bad-name",
            format!("\"{name}\" can't be used as a name"),
            Some("use lower-case letters, digits and `-`, starting with a letter".into()),
            Reader::node_span(n),
        );
    }
    ok
}

fn children(n: &KdlNode) -> impl Iterator<Item = &KdlNode> {
    n.children().into_iter().flat_map(|d| d.nodes().iter())
}

fn child_text(n: &KdlNode, name: &str) -> Option<String> {
    children(n)
        .find(|c| c.name().value() == name)
        .and_then(|c| args(c).first().and_then(|e| string(e)))
}

fn value_dyn(e: &KdlEntry) -> Dyn {
    match e.value() {
        KdlValue::String(s) => Dyn::str(s),
        KdlValue::Integer(i) => Dyn::Num(*i as f64),
        KdlValue::Float(f) => Dyn::Num(*f),
        KdlValue::Bool(b) => Dyn::Bool(*b),
        KdlValue::Null => Dyn::Nil,
    }
}

// ---- the parts ------------------------------------------------------------------------------------

fn parse_addon_with<'a>(
    r: &mut Reader<'a>,
    src: &str,
    n: &'a KdlNode,
    origin: Origin,
) -> Option<AddOn> {
    shape(
        r,
        n,
        &["name", "version", "requires", "icon"],
        &["about", "suggest-if", "list", "step"],
    );
    let id = first_string(r, n, "a name, like `add-on \"git\"`")?;
    if !check_name(r, n, &id) {
        return None;
    }
    let version = match prop(n, "version") {
        Some(e) => match string(e).as_deref().and_then(Version::parse) {
            Some(v) => v,
            None => {
                r.error(
                    "bad-version",
                    "The version must look like \"1.0\"".into(),
                    None,
                    Reader::entry_span(e),
                );
                Version(1, 0, 0)
            }
        },
        None => {
            r.error(
                "missing-version",
                format!("The add-on \"{id}\" needs a version, like version=\"1.0\""),
                None,
                Reader::node_span(n),
            );
            Version(1, 0, 0)
        }
    };
    let requires = prop_str(n, "requires").and_then(|s| Version::parse(&s));
    let name = prop_str(n, "name").unwrap_or_else(|| id.clone());
    let span = n.span();
    let text = src
        .get(span.offset()..span.offset() + span.len())
        .unwrap_or_default()
        .trim()
        .to_string();
    let mut addon = AddOn {
        id: id.clone(),
        name: name.clone(),
        version,
        requires,
        icon: prop_str(n, "icon"),
        about: child_text(n, "about"),
        suggest_if: Vec::new(),
        lists: Vec::new(),
        steps: Vec::new(),
        origin: origin.clone(),
        text,
    };
    for c in children(n) {
        match c.name().value() {
            "suggest-if" => {
                shape(r, c, &[], &["exists", "glob"]);
                for d in children(c) {
                    let Some(s) = args(d).first().and_then(|e| string(e)) else {
                        continue;
                    };
                    match d.name().value() {
                        "exists" => addon.suggest_if.push(Detect::Exists(s)),
                        "glob" => addon.suggest_if.push(Detect::Glob(s)),
                        _ => {}
                    }
                }
            }
            "list" => {
                if let Some(l) = list(r, c, &id) {
                    addon.lists.push(Arc::new(l));
                }
            }
            "step" => {
                if let Some(s) = step(r, c, &id, &name, &origin) {
                    if addon.steps.iter().any(|x| x.id == s.id) {
                        r.error(
                            "same-step-twice",
                            format!("There are two steps called \"{}\"", s.id),
                            None,
                            Reader::node_span(c),
                        );
                    } else {
                        addon.steps.push(Arc::new(s));
                    }
                }
            }
            _ => {}
        }
    }
    Some(addon)
}

fn list(r: &mut Reader, n: &KdlNode, addon: &str) -> Option<ListDef> {
    shape(r, n, &[], &["about", "setting", "script"]);
    let name = first_string(r, n, "a name, like `list \"branches\"`")?;
    check_name(r, n, &name);
    let mut settings = Vec::new();
    let mut script = None;
    for c in children(n) {
        match c.name().value() {
            "setting" => {
                if let Some(s) = setting(r, c) {
                    settings.push(s);
                }
            }
            "script" => script = args(c).first().and_then(|e| r.lua(e)),
            _ => {}
        }
    }
    let Some(script) = script else {
        r.error(
            "missing-script",
            format!("The list \"{name}\" needs a `script` that returns its choices"),
            None,
            Reader::node_span(n),
        );
        return None;
    };
    Some(ListDef {
        id: format!("{addon}.{name}"),
        about: child_text(n, "about"),
        settings,
        script,
    })
}

const SETTING_PROPS: &[&str] = &[
    "type",
    "label",
    "help",
    "starts",
    "required",
    "advanced",
    "extensions",
    "several",
];

fn setting(r: &mut Reader, n: &KdlNode) -> Option<SettingDef> {
    shape(r, n, SETTING_PROPS, &["option", "list-from"]);
    let id = first_string(r, n, "a name, like `setting \"folder\"`")?;
    check_name(r, n, &id);
    let kind = match prop(n, "type") {
        Some(e) => match string(e).as_deref().and_then(SettingKind::parse) {
            Some(k) => k,
            None => {
                let got = string(e).unwrap_or_default();
                r.error(
                    "bad-type",
                    format!("\"{got}\" isn't a kind of setting"),
                    Some(
                        did_you_mean(&got, SettingKind::WORDS.iter().copied())
                            .map(|m| format!("did you mean \"{m}\"?"))
                            .unwrap_or_else(|| {
                                format!("use one of: {}", SettingKind::WORDS.join(", "))
                            }),
                    ),
                    Reader::entry_span(e),
                );
                SettingKind::Text
            }
        },
        None => SettingKind::Text,
    };
    // A file setting's extensions: `extensions="json, fbx"`.
    let extensions = match prop(n, "extensions") {
        Some(e) => {
            let exts = string(e)
                .map(|t| crate::paths::extensions(&t))
                .unwrap_or_default();
            if kind != SettingKind::File {
                r.error(
                    "bad-prop",
                    "Only a file setting takes `extensions`".into(),
                    Some("take it out, or make the setting type=\"file\"".into()),
                    Reader::entry_span(e),
                );
            } else if exts.is_empty() {
                r.error(
                    "bad-value",
                    "`extensions` names no extension".into(),
                    Some("write them like extensions=\"json, fbx\"".into()),
                    Reader::entry_span(e),
                );
            }
            exts
        }
        None => Vec::new(),
    };
    // A file or folder setting that also takes a list of files an earlier step found; text
    // that takes a list, one item per line.
    let several = prop_bool(n, "several");
    if several && !takes_several(kind) {
        if let Some(e) = prop(n, "several") {
            r.error(
                "bad-prop",
                "Only a file, folder or text setting takes `several`".into(),
                Some(
                    "take it out, or make the setting type=\"folder\", \"file\" or \"long-text\""
                        .into(),
                ),
                Reader::entry_span(e),
            );
        }
    }
    let mut options = Vec::new();
    let mut list_from = None;
    for c in children(n) {
        match c.name().value() {
            "option" => options.extend(args(c).first().and_then(|e| string(e))),
            "list-from" => list_from = list_use(r, c),
            _ => {}
        }
    }
    Some(SettingDef {
        label: prop_str(n, "label").unwrap_or_else(|| label_from(&id)),
        id,
        kind,
        help: prop_str(n, "help"),
        starts: prop(n, "starts").map(value_dyn),
        required: prop_bool(n, "required"),
        advanced: prop_bool(n, "advanced"),
        options,
        list_from,
        extensions,
        several,
        position: None,
        rest: false,
        starts_overridden: false,
    })
}

/// `list-from "git.branches" folder="{folder}"`.
pub(crate) fn list_use(r: &mut Reader, n: &KdlNode) -> Option<ListUse> {
    let list = first_string(r, n, "the list's name, like `list-from \"git.branches\"`")?;
    let settings = n
        .entries()
        .iter()
        .filter_map(|e| Some((e.name()?.value().to_string(), r.val(e)?)))
        .collect();
    Some(ListUse { list, settings })
}

/// "ff-only" → "Ff only": a readable label when the add-on gives none.
fn label_from(id: &str) -> String {
    let words = id.replace(['-', '_'], " ");
    let mut c = words.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

const STEP_PROPS: &[&str] = &[
    "name",
    "says",
    "icon",
    "built-in",
    "check-step",
    "quiet",
    "advanced",
    "replaced-by",
];
const STEP_CHILDREN: &[&str] = &[
    "about",
    "search-words",
    "comes-next",
    "setting",
    "result",
    "needs",
    "explain",
    "does",
    "script",
    "example",
];

fn step<'a>(
    r: &mut Reader<'a>,
    n: &'a KdlNode,
    addon: &str,
    app: &str,
    origin: &Origin,
) -> Option<StepDef> {
    shape(r, n, STEP_PROPS, STEP_CHILDREN);
    let name = first_string(r, n, "a name, like `step \"pull\"`")?;
    check_name(r, n, &name);
    let id = format!("{addon}.{name}");
    let mut def = StepDef {
        name: prop_str(n, "name").unwrap_or_else(|| label_from(&name)),
        id,
        says: prop_str(n, "says"),
        about: child_text(n, "about"),
        icon: prop_str(n, "icon"),
        search_words: Vec::new(),
        comes_next: Vec::new(),
        settings: Vec::new(),
        results: Vec::new(),
        needs: Vec::new(),
        explain: Vec::new(),
        body: Body::Does(Vec::new()),
        check_step: prop_bool(n, "check-step"),
        quiet: prop_bool(n, "quiet"),
        advanced: prop_bool(n, "advanced"),
        replaced_by: prop_str(n, "replaced-by"),
        app: app.to_string(),
    };
    // A built-in step: the engine's own step, its settings from the catalog.
    let mut builtin = None;
    let asked_builtin = prop(n, "built-in").is_some();
    if let Some(e) = prop(n, "built-in") {
        let kind = string(e).unwrap_or_default();
        match catalog::get(&kind).filter(|s| s.cat == Category::Step) {
            Some(spec) if *origin == Origin::BuiltIn => {
                def.settings = catalog_settings(spec);
                builtin = Some(spec);
            }
            Some(_) => r.error(
                "built-in-only",
                "Only add-ons built into Launcher can use `built-in`".into(),
                Some("write the step's `does { }` instead".into()),
                Reader::entry_span(e),
            ),
            None => r.error(
                "unknown-built-in",
                format!("Launcher has no built-in step \"{kind}\""),
                None,
                Reader::entry_span(e),
            ),
        }
    }
    let mut body = None;
    for c in children(n) {
        match c.name().value() {
            "search-words" => def
                .search_words
                .extend(args(c).iter().filter_map(|e| string(e))),
            "comes-next" => def
                .comes_next
                .extend(args(c).iter().filter_map(|e| string(e))),
            "setting" => {
                let Some(s) = setting(r, c) else { continue };
                if builtin.is_some() {
                    // A built-in step's setting may only be relabelled, or its text made long
                    // text, or made to take a list.
                    match def.settings.iter_mut().find(|x| x.id == s.id) {
                        Some(x) => {
                            if prop(c, "label").is_some() {
                                x.label = s.label;
                            }
                            if let Some(e) = prop(c, "type") {
                                if x.kind == SettingKind::Text && s.kind == SettingKind::LongText {
                                    x.kind = SettingKind::LongText;
                                } else if x.kind != s.kind {
                                    r.error(
                                        "bad-type",
                                        format!(
                                            "The built-in step's \"{}\" is {}",
                                            x.id,
                                            x.kind.word()
                                        ),
                                        Some("only text can become long-text".into()),
                                        Reader::entry_span(e),
                                    );
                                }
                            }
                            if s.several {
                                if takes_several(x.kind) {
                                    x.several = true;
                                } else if let Some(e) = prop(c, "several") {
                                    r.error(
                                        "bad-prop",
                                        format!(
                                            "The built-in step's \"{}\" can't take several",
                                            x.id
                                        ),
                                        None,
                                        Reader::entry_span(e),
                                    );
                                }
                            }
                            x.help = s.help.or(x.help.take());
                            x.advanced |= s.advanced;
                            if s.starts.is_some() {
                                x.starts = s.starts;
                                x.starts_overridden = true;
                            }
                        }
                        None => r.error(
                            "unknown-setting",
                            format!("The built-in step has no setting \"{}\"", s.id),
                            did_you_mean(&s.id, def.settings.iter().map(|x| x.id.as_str()))
                                .map(|m| format!("did you mean \"{m}\"?")),
                            Reader::node_span(c),
                        ),
                    }
                } else if def.settings.iter().any(|x| x.id == s.id) {
                    r.error(
                        "same-setting-twice",
                        format!("There are two settings called \"{}\"", s.id),
                        None,
                        Reader::node_span(c),
                    );
                } else {
                    def.settings.push(s);
                }
            }
            "result" => {
                shape(r, c, &["label", "type", "item", "items"], &[]);
                // What kind of value it is, so only settings that can take it use it.
                let kind = match prop(c, "type") {
                    Some(e) => match string(e).as_deref().and_then(ResultKind::parse) {
                        Some(k) => k,
                        None => {
                            let got = string(e).unwrap_or_default();
                            r.error(
                                "bad-type",
                                format!("\"{got}\" isn't a kind of result"),
                                Some(
                                    did_you_mean(&got, ResultKind::WORDS.iter().copied())
                                        .map(|m| format!("did you mean \"{m}\"?"))
                                        .unwrap_or_else(|| {
                                            format!("use one of: {}", ResultKind::WORDS.join(", "))
                                        }),
                                ),
                                Reader::entry_span(e),
                            );
                            ResultKind::Text
                        }
                    },
                    None => ResultKind::Text,
                };
                if let Some(rid) = first_string(r, c, "a name, like `result \"commit\"`") {
                    check_name(r, c, &rid);
                    // What a list holds, for counts: "54 models".
                    let one = prop_str(c, "item").unwrap_or_else(|| {
                        if kind == ResultKind::Files {
                            "file"
                        } else {
                            "item"
                        }
                        .to_string()
                    });
                    let many = prop_str(c, "items").unwrap_or_else(|| format!("{one}s"));
                    def.results.push(ResultDef {
                        label: prop_str(c, "label").unwrap_or_else(|| label_from(&rid)),
                        id: rid,
                        kind,
                        item: (one, many),
                    });
                }
            }
            "needs" => {
                shape(r, c, &[], &["app", "os"]);
                for d in children(c) {
                    match d.name().value() {
                        "app" => {
                            shape(r, d, &["help"], &[]);
                            if let Some(name) =
                                first_string(r, d, "the program, like `app \"git\"`")
                            {
                                def.needs.push(Need::App {
                                    name,
                                    help: prop_str(d, "help"),
                                });
                            }
                        }
                        "os" => match args(d).first().and_then(|e| string(e)).as_deref() {
                            Some("windows") => def.needs.push(Need::Os(Os::Windows)),
                            Some("macos") => def.needs.push(Need::Os(Os::Macos)),
                            _ => r.error(
                                "bad-os",
                                "`os` is \"windows\" or \"macos\"".into(),
                                None,
                                Reader::node_span(d),
                            ),
                        },
                        _ => {}
                    }
                }
            }
            "explain" => {
                let a = args(c);
                match (a.first().and_then(|e| string(e)), a.get(1).and_then(|e| string(e))) {
                    (Some(phrase), Some(advice)) => def.explain.push((phrase, advice)),
                    _ => r.error(
                        "bad-explain",
                        "`explain` takes a phrase from the output, then the advice to show".into(),
                        Some("explain \"would be overwritten\" \"Commit or stash your changes first.\"".into()),
                        Reader::node_span(c),
                    ),
                }
            }
            "does" => {
                if body.is_some() || builtin.is_some() {
                    r.error(
                        "two-bodies",
                        "A step does one thing: `does`, `script` or `built-in`".into(),
                        None,
                        Reader::node_span(c),
                    );
                    continue;
                }
                let steps = match c.children() {
                    Some(doc) => r.steps(doc, Vec::new()),
                    None => Vec::new(),
                };
                body = Some(Body::Does(steps));
            }
            "script" => {
                if body.is_some() || builtin.is_some() {
                    r.error(
                        "two-bodies",
                        "A step does one thing: `does`, `script` or `built-in`".into(),
                        None,
                        Reader::node_span(c),
                    );
                    continue;
                }
                if let Some(src) = args(c).first().and_then(|e| r.lua(e)) {
                    body = Some(Body::Script(src));
                }
            }
            _ => {}
        }
    }
    def.body = match (builtin, body) {
        (Some(spec), _) => Body::BuiltIn(spec),
        (None, Some(b)) => b,
        (None, None) if asked_builtin => Body::Does(Vec::new()),
        (None, None) => {
            r.error(
                "nothing-to-do",
                format!("The step \"{}\" needs `does {{ }}` or `script`", def.id),
                None,
                Reader::node_span(n),
            );
            Body::Does(Vec::new())
        }
    };
    Some(def)
}

/// A built-in step's settings: its catalog arguments, then its own properties.
/// The kinds of setting that can take several values.
fn takes_several(kind: SettingKind) -> bool {
    matches!(
        kind,
        SettingKind::File | SettingKind::Folder | SettingKind::Text | SettingKind::LongText
    )
}

fn catalog_settings(spec: &'static catalog::Spec) -> Vec<SettingDef> {
    let mut out = Vec::new();
    for (i, a) in spec.args.iter().enumerate() {
        let mut s = from_param(a);
        s.position = Some(i);
        out.push(s);
    }
    if let Some(rest) = &spec.rest {
        let mut s = from_param(rest);
        s.kind = SettingKind::List;
        s.rest = true;
        out.push(s);
    }
    for p in spec.props.iter().filter(|p| !p.filled) {
        out.push(from_param(p));
    }
    out
}

fn from_param(p: &Param) -> SettingDef {
    let (kind, options) = match p.ty {
        Ty::Bool => (SettingKind::TickBox, Vec::new()),
        Ty::Int | Ty::Number => (SettingKind::Number, Vec::new()),
        Ty::Enum(words) => (
            SettingKind::Choice,
            words.iter().map(|w| w.to_string()).collect(),
        ),
        Ty::LuaChunk => (SettingKind::LongText, Vec::new()),
        _ => (SettingKind::Text, Vec::new()),
    };
    let starts = match (p.default, p.ty) {
        ("", _) => None,
        ("#true", _) => Some(Dyn::Bool(true)),
        ("#false", _) => Some(Dyn::Bool(false)),
        (d, Ty::Int | Ty::Number) => d.parse::<f64>().ok().map(Dyn::Num),
        (d, _) => Some(Dyn::str(d)),
    };
    SettingDef {
        id: p.name.to_string(),
        kind,
        label: label_from(p.name),
        help: Some(p.doc.to_string()),
        starts,
        required: p.required,
        advanced: matches!(p.ty, Ty::Lua | Ty::LuaChunk | Ty::LuaValue),
        options,
        list_from: None,
        extensions: Vec::new(),
        several: false,
        position: None,
        rest: false,
        starts_overridden: false,
    }
}

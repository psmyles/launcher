//! Cross-reference checks, after loading: ids are unique, references name things that exist, and
//! template placeholders name something the run will know.

use std::collections::{HashMap, HashSet};

use super::catalog::{self, Os};
use super::diag::{Diagnostic, LineIndex, Severity};
use super::model::*;
use crate::suggest::did_you_mean;

/// Names every template may use besides the file's own: the file's location, the OS, a step's
/// error (in `message=`), an `args` pair (in `format`), and a found file (in `require-sibling`).
pub const BUILTIN_NAMES: &[&str] = &[
    "config_dir",
    "config_file",
    "os",
    "error",
    "key",
    "value",
    "stem",
    "name",
    "ext",
];

struct Ctx<'a> {
    lines: &'a LineIndex<'a>,
    diags: &'a mut Vec<Diagnostic>,
}

impl Ctx<'_> {
    fn push(
        &mut self,
        severity: Severity,
        code: &'static str,
        message: String,
        help: Option<String>,
        node: &NodeRef,
    ) {
        let (line, col) = self.lines.line_col(node.span.offset);
        self.diags.push(Diagnostic {
            severity,
            code,
            message,
            help,
            offset: node.span.offset,
            len: node.span.len,
            line,
            col,
            path: Some(node.path.clone()),
        });
    }
}

fn hint<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    did_you_mean(word, candidates).map(|s| format!("did you mean `{s}`?"))
}

pub fn validate(config: &Config, lines: &LineIndex, diags: &mut Vec<Diagnostic>) {
    let mut cx = Ctx { lines, diags };

    // ---- ids ------------------------------------------------------------------------------------
    // Controls, values and data share one namespace: templates and Lua name them the same way.
    let mut names: HashMap<&str, &NodeRef> = HashMap::new();
    let everything = config
        .controls
        .iter()
        .map(|c| (c.id.as_str(), &c.node))
        .chain(config.values.iter().map(|v| (v.id.as_str(), &v.node)))
        .chain(config.data.iter().map(|d| (d.id.as_str(), &d.node)));
    for (id, node) in everything {
        if names.insert(id, node).is_some() {
            cx.push(
                Severity::Error,
                "duplicate-id",
                format!("`{id}` is used twice"),
                Some("controls, values and data blocks each need their own name".into()),
                node,
            );
        }
        if BUILTIN_NAMES.contains(&id) || id == "env" {
            cx.push(
                Severity::Warning,
                "reserved-name",
                format!("`{id}` is also a built-in name; {{{id}}} will mean the built-in"),
                Some("pick another name".into()),
                node,
            );
        }
    }
    let mut actions: HashSet<&str> = HashSet::new();
    for a in &config.actions {
        if !actions.insert(&a.id) {
            cx.push(
                Severity::Error,
                "duplicate-id",
                format!("there are two actions named `{}`", a.id),
                None,
                &a.node,
            );
        }
    }

    // Names steps define, anywhere (a run's variables are dynamic; a name defined by any step is
    // accepted everywhere rather than guessed per path).
    let mut step_names: HashSet<String> = HashSet::new();
    let mut all_steps: Vec<&Step> = Vec::new();
    for a in &config.actions {
        collect_steps(&a.steps, &mut all_steps);
    }
    for c in &config.controls {
        collect_steps(&c.on_change, &mut all_steps);
    }
    for s in &all_steps {
        if let Some(id) = &s.id {
            step_names.insert(id.clone());
        }
        match &s.kind {
            StepKind::Set { name, .. } | StepKind::Ask { name, .. } => {
                step_names.insert(name.clone());
            }
            StepKind::Args(a) => {
                step_names.insert(a.name.clone());
            }
            StepKind::HttpGet { extract, .. } => {
                for e in extract {
                    step_names.insert(e.name.clone());
                }
            }
            _ => {}
        }
    }
    for a in &config.actions {
        let mut seen = HashSet::new();
        for s in &a.steps {
            if let Some(id) = &s.id {
                if !seen.insert(id.as_str()) {
                    cx.push(
                        Severity::Warning,
                        "duplicate-id",
                        format!("two steps of `{}` have the id `{id}`", a.id),
                        Some("the later one's result replaces the earlier one's".into()),
                        &s.node,
                    );
                }
            }
        }
    }

    // ---- references ------------------------------------------------------------------------------
    let control_ids: Vec<&str> = config.controls.iter().map(|c| c.id.as_str()).collect();
    let valued: Vec<&str> = config
        .controls
        .iter()
        .filter(|c| c.has_value())
        .map(|c| c.id.as_str())
        .collect();
    for c in &config.controls {
        if let ControlKind::Button { action, .. } = &c.kind {
            if config.action(action).is_none() {
                cx.push(
                    Severity::Error,
                    "unknown-action",
                    format!(
                        "button `{}` runs `{action}`, but there is no such action",
                        c.id
                    ),
                    hint(action, actions.iter().copied()),
                    &c.node,
                );
            }
        }
        if c.required && !c.has_value() {
            cx.push(
                Severity::Warning,
                "required-no-value",
                format!(
                    "`{}` is a {} and holds no value to require",
                    c.id,
                    c.kind_name()
                ),
                None,
                &c.node,
            );
        }
    }
    for a in &config.actions {
        check_step_refs(&mut cx, config, &a.steps, &valued, &control_ids);
    }
    for c in &config.controls {
        check_step_refs(&mut cx, config, &c.on_change, &valued, &control_ids);
    }

    // ---- template names ---------------------------------------------------------------------------
    let mut known: HashSet<String> = names.keys().map(|s| s.to_string()).collect();
    known.extend(step_names.iter().cloned());
    known.extend(BUILTIN_NAMES.iter().map(|s| s.to_string()));
    let mut vals: Vec<(&Val, &NodeRef)> = Vec::new();
    collect_vals(config, &mut vals);
    // An option-label also names the option's own fields: {name}, {Host}.
    for c in &config.controls {
        if let ControlKind::Choice {
            option_label: Some(label),
            options,
            options_lua,
            list,
            ..
        } = &c.kind
        {
            let mut local = known.clone();
            local.extend(
                options
                    .iter()
                    .flat_map(|o| o.bag.iter().map(|(k, _)| k.clone())),
            );
            local.insert("label".into());
            // Computed options can carry any field; only literal ones are checked.
            if options_lua.is_none() && list.is_none() {
                vals_named_in(&mut cx, label, &c.node, &local);
            }
        }
    }
    let mut reported: HashSet<(usize, String)> = HashSet::new();
    for (val, node) in vals {
        let Val::Text(t) = val else { continue };
        for (name, field, _) in t.refs() {
            if !known.contains(name) {
                if reported.insert((node.span.offset, name.to_string())) {
                    cx.push(
                        Severity::Warning,
                        "unknown-name",
                        format!(
                            "`{{{name}}}` doesn't name a control, value, data block or step result"
                        ),
                        hint(name, known.iter().map(String::as_str)),
                        node,
                    );
                }
                continue;
            }
            let Some(field) = field else { continue };
            let Some(c) = config.control(name) else {
                continue;
            };
            if let ControlKind::Choice {
                options,
                options_lua: None,
                list: None,
                ..
            } = &c.kind
            {
                let has = |o: &OptionDef| o.bag.iter().any(|(k, _)| k.eq_ignore_ascii_case(field));
                let builtin = matches!(field, "name" | "value" | "label");
                if !builtin && !options.iter().any(has) {
                    let keys: HashSet<&str> = options
                        .iter()
                        .flat_map(|o| o.bag.iter().map(|(k, _)| k.as_str()))
                        .collect();
                    if reported.insert((node.span.offset, format!("{name}.{field}"))) {
                        cx.push(
                            Severity::Warning,
                            "unknown-field",
                            format!("no option of `{name}` has a `{field}` field"),
                            hint(field, keys.iter().copied()),
                            node,
                        );
                    }
                }
            }
        }
    }

    // ---- OS ----------------------------------------------------------------------------------------
    // A file without `os=` is meant for both; OS-only nodes then work on only one of them.
    if config.header.os.is_none() {
        for s in &all_steps {
            let Some(spec) = catalog::get(s.kind.name()) else {
                continue;
            };
            if spec.os != Os::Any {
                cx.push(
                    Severity::Warning,
                    "os-only",
                    format!(
                        "`{}` only works on {}; on the other OS this step fails",
                        spec.name,
                        super::header::os_title(spec.os)
                    ),
                    Some(format!(
                        "add os=\"{}\" to the `launcher` line if the file is only for {}",
                        spec.os.name(),
                        super::header::os_title(spec.os)
                    )),
                    &s.node,
                );
            }
            if let StepKind::Run(r) = &s.kind {
                if r.command_line.is_some() {
                    cx.push(
                        Severity::Warning,
                        "os-only",
                        "`command-line=` only works on Windows".into(),
                        Some("pass the arguments separately to work on macOS too".into()),
                        &s.node,
                    );
                }
            }
        }
    }
}

/// Warn about placeholders in `val` that `known` doesn't contain.
fn vals_named_in(cx: &mut Ctx, val: &Val, node: &NodeRef, known: &HashSet<String>) {
    let Val::Text(t) = val else { return };
    for (name, _, _) in t.refs() {
        if !known.contains(name) {
            cx.push(
                Severity::Warning,
                "unknown-name",
                format!("`{{{name}}}` doesn't name an option field, control, value or data block"),
                hint(name, known.iter().map(String::as_str)),
                node,
            );
        }
    }
}

fn collect_steps<'a>(steps: &'a [Step], out: &mut Vec<&'a Step>) {
    out.extend(steps.iter());
}

fn check_step_refs(
    cx: &mut Ctx,
    config: &Config,
    steps: &[Step],
    valued: &[&str],
    controls: &[&str],
) {
    let mut runs_before: Vec<&str> = Vec::new();
    for s in steps {
        match &s.kind {
            StepKind::SetControl { control, .. }
            | StepKind::Reset { control }
            | StepKind::Put { control, .. } => {
                if !valued.contains(&control.as_str()) {
                    let what = if controls.contains(&control.as_str()) {
                        format!("`{control}` holds no value to set")
                    } else {
                        format!("there is no control `{control}`")
                    };
                    cx.push(
                        Severity::Error,
                        "unknown-control",
                        what,
                        hint(control, valued.iter().copied()),
                        &s.node,
                    );
                }
            }
            StepKind::Args(a) => {
                for part in &a.parts {
                    if let ArgPart::Pairs { source, .. } = part {
                        let is_data = config.data_block(source).is_some();
                        let is_choice = config
                            .control(source)
                            .is_some_and(|c| matches!(c.kind, ControlKind::Choice { .. }));
                        if !is_data && !is_choice {
                            let candidates: Vec<&str> = config
                                .data
                                .iter()
                                .map(|d| d.id.as_str())
                                .chain(
                                    config
                                        .controls
                                        .iter()
                                        .filter(|c| matches!(c.kind, ControlKind::Choice { .. }))
                                        .map(|c| c.id.as_str()),
                                )
                                .collect();
                            cx.push(
                                Severity::Error,
                                "unknown-source",
                                format!("`pairs \"{source}\"`: there is no data block or choice control `{source}`"),
                                hint(source, candidates),
                                &s.node,
                            );
                        }
                    }
                }
            }
            StepKind::SetProcessVolume { run, .. } if !runs_before.contains(&run.as_str()) => {
                cx.push(
                    Severity::Error,
                    "unknown-run",
                    format!("no earlier `run` step has id=\"{run}\""),
                    hint(run, runs_before.iter().copied()),
                    &s.node,
                );
            }
            _ => {}
        }
        if let (StepKind::Run(_), Some(id)) = (&s.kind, &s.id) {
            runs_before.push(id);
        }
    }
}

fn push_val<'a>(v: Option<&'a Val>, node: &'a NodeRef, out: &mut Vec<(&'a Val, &'a NodeRef)>) {
    if let Some(v) = v {
        out.push((v, node));
    }
}

fn collect_items<'a>(items: &'a [Item], out: &mut Vec<(&'a Val, &'a NodeRef)>) {
    for it in items {
        match it {
            Item::Group {
                title, items, node, ..
            } => {
                push_val(title.as_ref(), node, out);
                collect_items(items, out);
            }
            Item::Row { items, .. } => collect_items(items, out),
            Item::Tabs { tabs, .. } => {
                for t in tabs {
                    push_val(Some(&t.title), &t.node, out);
                    collect_items(&t.items, out);
                }
            }
            Item::Columns { columns, .. } => {
                for c in columns {
                    collect_items(c, out);
                }
            }
            Item::Separator { title, node } => push_val(title.as_ref(), node, out),
            Item::Note { text, node, .. } => push_val(Some(text), node, out),
            Item::Spacer { .. } | Item::Control(_) => {}
        }
    }
}

fn collect_step_vals<'a>(steps: &'a [Step], out: &mut Vec<(&'a Val, &'a NodeRef)>) {
    for s in steps {
        let n = &s.node;
        push_val(s.label.as_ref(), n, out);
        push_val(s.message.as_ref(), n, out);
        match &s.kind {
            StepKind::Run(r) => {
                push_val(Some(&r.program), n, out);
                for a in &r.args {
                    push_val(Some(a), n, out);
                }
                push_val(r.command_line.as_ref(), n, out);
                push_val(r.cwd.as_ref(), n, out);
                push_val(r.stdin.as_ref(), n, out);
                for (_, v) in &r.env {
                    push_val(Some(v), n, out);
                }
            }
            StepKind::Check {
                program, args, cwd, ..
            }
            | StepKind::Capture {
                program, args, cwd, ..
            } => {
                push_val(Some(program), n, out);
                for a in args {
                    push_val(Some(a), n, out);
                }
                push_val(cwd.as_ref(), n, out);
            }
            StepKind::Use(u) => {
                for (_, v) in &u.settings {
                    push_val(Some(v), n, out);
                }
            }
            StepKind::EnvFrom { script, args } => {
                push_val(Some(script), n, out);
                for a in args {
                    push_val(Some(a), n, out);
                }
            }
            StepKind::Set { value, .. } | StepKind::SetControl { value, .. } => {
                push_val(Some(value), n, out)
            }
            StepKind::Copy { from, to, .. } | StepKind::Move { from, to, .. } => {
                push_val(Some(from), n, out);
                push_val(Some(to), n, out);
            }
            StepKind::Delete { path, .. }
            | StepKind::Mkdir { path }
            | StepKind::ReadFile { path }
            | StepKind::Reveal { path } => push_val(Some(path), n, out),
            StepKind::FindFile(f) => {
                push_val(Some(&f.dir), n, out);
                push_val(f.require_sibling.as_ref(), n, out);
            }
            StepKind::WriteFile {
                path,
                content,
                heading,
                ..
            } => {
                push_val(Some(path), n, out);
                push_val(Some(content), n, out);
                push_val(heading.as_ref(), n, out);
            }
            StepKind::HttpGet { url, headers, .. } => {
                push_val(Some(url), n, out);
                for (_, v) in headers {
                    push_val(Some(v), n, out);
                }
            }
            StepKind::Ping { host, .. } => push_val(Some(host), n, out),
            StepKind::Open { target } => push_val(Some(target), n, out),
            StepKind::Message { text, .. }
            | StepKind::Log { text, .. }
            | StepKind::Fail { text }
            | StepKind::Confirm { text, .. } => push_val(Some(text), n, out),
            StepKind::Ask {
                prompt, default, ..
            } => {
                push_val(Some(prompt), n, out);
                push_val(default.as_ref(), n, out);
            }
            StepKind::Args(a) => {
                for part in &a.parts {
                    match part {
                        ArgPart::Raw { text, .. } => push_val(Some(text), n, out),
                        ArgPart::Pair { value, .. } => push_val(Some(value), n, out),
                        ArgPart::Pairs { .. } => {}
                    }
                }
            }
            StepKind::SetProcessVolume { volume, .. } => push_val(Some(volume), n, out),
            StepKind::Reset { .. }
            | StepKind::Put { .. }
            | StepKind::Wait { .. }
            | StepKind::Stop
            | StepKind::Lua { .. } => {}
        }
    }
}

/// Every template-or-Lua value in the file, with its node.
pub(crate) fn collect_vals<'a>(config: &'a Config, out: &mut Vec<(&'a Val, &'a NodeRef)>) {
    if let Some(n) = &config.window.node {
        push_val(Some(&config.window.title), n, out);
    }
    for v in &config.values {
        match &v.source {
            ValueSource::Val(val) => push_val(Some(val), &v.node, out),
            ValueSource::Find(f) => {
                push_val(Some(&f.dir), &v.node, out);
                push_val(f.require_sibling.as_ref(), &v.node, out);
            }
        }
    }
    collect_items(&config.layout, out);
    for c in &config.controls {
        let n = &c.node;
        push_val(c.tooltip.as_ref(), n, out);
        push_val(c.disabled_tooltip.as_ref(), n, out);
        if let Some(DefaultVal::Val(v)) = &c.default {
            push_val(Some(v), n, out);
        }
        match &c.kind {
            ControlKind::Choice {
                options,
                option_label,
                presets,
                ..
            } => {
                // option-label is checked with the options' fields (see `validate`).
                let _ = option_label;
                for o in options {
                    push_val(o.label.as_ref(), &o.node, out);
                }
                for p in presets {
                    push_val(Some(&p.value), &p.node, out);
                }
            }
            ControlKind::Display { text, .. } => push_val(Some(text), n, out),
            ControlKind::Button { confirm, .. } => push_val(confirm.as_ref(), n, out),
            _ => {}
        }
        collect_step_vals(&c.on_change, out);
    }
    for a in &config.actions {
        push_val(a.lock.as_ref(), &a.node, out);
        collect_step_vals(&a.steps, out);
    }
}

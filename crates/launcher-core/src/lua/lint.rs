//! Checking a file's Lua without running it (N12, M8): for the editor's Problems and for
//! `launcher check`.
//!
//! * **Parse errors** from full_moon (Luau syntax).
//! * **Lint** from selene-lib, with a standard library of Luau plus this project's API, generated
//!   from the registry ([`super::api`]), plus the globals the file's `script` blocks define:
//!   undefined names, argument counts, unused variables, shadowing and the rest of selene's
//!   defaults.
//! * **Cross-checks** against the file: `values.x` must name a control, a value or a run variable.
//!
//! Positions are the file's lines and columns ([`super::source::file_position`]).

use std::collections::{BTreeMap, BTreeSet};

use full_moon::ast::{Stmt, Var};
use full_moon::tokenizer::{Lexer, LexerResult, Symbol, TokenType};
use full_moon::LuaVersion;
use selene_lib::standard_library::{
    Argument, ArgumentType, Field, FieldKind, FunctionBehavior, Observes, PropertyWritability,
    Required, StandardLibrary,
};
use selene_lib::{Checker, CheckerConfig};

use super::api::API;
use super::source::file_position;
use crate::config::*;

/// A problem in a piece of Lua, at a file position.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub error: bool,
    /// selene's lint name, `parse`, or `unknown-value`.
    pub code: String,
}

/// Every piece of Lua in a file: the source, and whether it is an expression (a `(lua)` value, a
/// condition, a computed list) rather than statements (a `script` block, a `lua` step).
pub fn sources(config: &Config) -> Vec<(LuaSrc, bool)> {
    let mut out: Vec<(LuaSrc, bool)> = config.scripts.iter().map(|s| (s.clone(), false)).collect();
    let mut vals = Vec::new();
    crate::config::validate::collect_vals(config, &mut vals);
    for (v, _) in vals {
        if let Val::Lua(src) = v {
            out.push((src.clone(), true));
        }
    }
    fn steps(list: &[Step], out: &mut Vec<(LuaSrc, bool)>) {
        for s in list {
            if let Some(c) = &s.if_ {
                out.push((c.clone(), true));
            }
            match &s.kind {
                StepKind::Lua { code } => out.push((code.clone(), false)),
                StepKind::Args(a) => {
                    for p in &a.parts {
                        let cond = match p {
                            ArgPart::Raw { if_, .. }
                            | ArgPart::Pair { if_, .. }
                            | ArgPart::Pairs { if_, .. } => if_,
                        };
                        if let Some(c) = cond {
                            out.push((c.clone(), true));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn items(list: &[Item], out: &mut Vec<(LuaSrc, bool)>) {
        for it in list {
            match it {
                Item::Group {
                    visible_if,
                    items: i,
                    ..
                }
                | Item::Row {
                    visible_if,
                    items: i,
                    ..
                } => {
                    if let Some(c) = visible_if {
                        out.push((c.clone(), true));
                    }
                    items(i, out);
                }
                Item::Tabs { tabs, .. } => {
                    for t in tabs {
                        if let Some(c) = &t.visible_if {
                            out.push((c.clone(), true));
                        }
                        items(&t.items, out);
                    }
                }
                Item::Columns { columns, .. } => {
                    for c in columns {
                        items(c, out);
                    }
                }
                Item::Note {
                    visible_if: Some(c),
                    ..
                } => out.push((c.clone(), true)),
                _ => {}
            }
        }
    }
    items(&config.layout, &mut out);
    for c in &config.controls {
        for cond in [&c.visible_if, &c.enabled_if].into_iter().flatten() {
            out.push((cond.clone(), true));
        }
        if let ControlKind::Choice {
            options_lua: Some(src),
            ..
        } = &c.kind
        {
            out.push((src.clone(), true));
        }
        steps(&c.on_change, &mut out);
    }
    for a in &config.actions {
        steps(&a.steps, &mut out);
    }
    // One entry per place in the file.
    let mut seen = BTreeSet::new();
    out.retain(|(s, _)| seen.insert((s.span.offset, s.span.len)));
    out
}

/// A signature's arguments: `name`, `name?` (optional), `{table}`, `arg...` (any number).
fn arguments(signature: &str) -> Vec<Argument> {
    let Some(open) = signature.find('(') else {
        return Vec::new();
    };
    let mut depth = 0;
    let mut close = signature.len();
    for (i, c) in signature[open..].char_indices() {
        match c {
            '(' | '{' => depth += 1,
            ')' | '}' => {
                depth -= 1;
                if depth == 0 && c == ')' {
                    close = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = &signature[open + 1..close];
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(inner[start..].trim());
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (required, argument_type) = if p.ends_with("...") {
                (Required::NotRequired, ArgumentType::Vararg)
            } else if let Some(t) = p.strip_suffix('?') {
                let ty = if t.starts_with('{') {
                    ArgumentType::Table
                } else {
                    ArgumentType::Any
                };
                (Required::NotRequired, ty)
            } else if p.starts_with('{') {
                (Required::Required(None), ArgumentType::Table)
            } else if p == "function" {
                (Required::Required(None), ArgumentType::Function)
            } else {
                (Required::Required(None), ArgumentType::Any)
            };
            Argument {
                required,
                argument_type,
                observes: Observes::default(),
                deprecated: None,
            }
        })
        .collect()
}

/// Luau's standard library plus this project's API and `extra` globals (defined by the file's
/// scripts).
pub fn standard_library(extra: &BTreeSet<String>) -> StandardLibrary {
    let mut std = StandardLibrary::from_name("luau").unwrap_or_default();
    let mut globals = BTreeMap::new();
    for item in API {
        let kind = if item.function {
            FieldKind::Function(FunctionBehavior {
                arguments: arguments(item.signature),
                method: false,
                must_use: false,
            })
        } else if matches!(item.name, "values" | "fields" | "env" | "settings") {
            FieldKind::Any
        } else {
            FieldKind::Property(PropertyWritability::ReadOnly)
        };
        globals.insert(item.name.to_string(), Field::from_field_kind(kind));
    }
    for name in extra {
        globals
            .entry(name.clone())
            .or_insert(Field::from_field_kind(FieldKind::Any));
    }
    // Before the library is first used (it caches its global tree then).
    std.globals.extend(globals);
    std
}

/// Names `script` blocks define at their top level (`function name()`, `name = ...`).
pub fn script_globals(config: &Config) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for s in &config.scripts {
        let Ok(ast) = full_moon::parse_fallible(&s.code, LuaVersion::luau()).into_result() else {
            continue;
        };
        for stmt in ast.nodes().stmts() {
            match stmt {
                Stmt::FunctionDeclaration(f) => {
                    if let Some(first) = f.name().names().iter().next() {
                        out.insert(first.token().to_string());
                    }
                }
                Stmt::Assignment(a) => {
                    for v in a.variables() {
                        if let Var::Name(t) = v {
                            out.insert(t.token().to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// 1-based line and column of byte `at` in `code`.
fn line_col(code: &str, at: usize) -> (usize, usize) {
    let at = at.min(code.len());
    let before = &code[..at];
    let line = before.matches('\n').count() + 1;
    let col = before.rfind('\n').map_or(at, |nl| at - nl - 1) + 1;
    (line, col)
}

/// What an expression is linted as (the same wrapping the VM compiles it with).
const EXPR_PREFIX: &str = "return (";

pub struct Linter {
    checker: Checker<serde_json::Value>,
}

impl Linter {
    pub fn new(config: &Config) -> Result<Linter, String> {
        let std = standard_library(&script_globals(config));
        let checker = Checker::new(CheckerConfig::default(), std).map_err(|e| e.to_string())?;
        Ok(Linter { checker })
    }

    /// Issues in one piece of Lua.
    pub fn lint(&self, src: &LuaSrc, expr: bool) -> Vec<Issue> {
        let code = if expr {
            format!("{EXPR_PREFIX}{}\n)", src.code)
        } else {
            src.code.clone()
        };
        // Chunk position → file position (an expression's first line is shifted by the prefix).
        let at = |line: usize, col: usize| {
            let col = if expr && line == 1 {
                col.saturating_sub(EXPR_PREFIX.len()).max(1)
            } else {
                col
            };
            file_position(src, line, col)
        };
        let parsed = full_moon::parse_fallible(&code, LuaVersion::luau());
        if !parsed.errors().is_empty() {
            return parsed
                .errors()
                .iter()
                .map(|e| {
                    let p = e.range().0;
                    let (line, col) = at(p.line(), p.character());
                    Issue {
                        message: e.error_message().to_string(),
                        line,
                        col,
                        error: true,
                        code: "parse".into(),
                    }
                })
                .collect();
        }
        let ast = parsed.into_ast();
        let mut out: Vec<Issue> = self
            .checker
            .test_on(&ast)
            .into_iter()
            .filter(|d| d.severity != selene_lib::lints::Severity::Allow)
            .map(|d| {
                let (l, c) = line_col(&code, d.diagnostic.primary_label.range.0 as usize);
                let (line, col) = at(l, c);
                Issue {
                    message: d.diagnostic.message.clone(),
                    line,
                    col,
                    error: d.severity == selene_lib::lints::Severity::Error,
                    code: d.diagnostic.code.to_string(),
                }
            })
            .collect();
        out.sort_by_key(|i| (i.line, i.col));
        out
    }
}

/// The names `values.x` may use: controls, computed values, and run variables (set, ask, args,
/// step ids, http-get extracts, and literal `set("name", ...)` calls in Lua).
fn known_values(config: &Config, sources: &[(LuaSrc, bool)]) -> BTreeSet<String> {
    let mut known: BTreeSet<String> = config
        .controls
        .iter()
        .map(|c| c.id.clone())
        .chain(config.values.iter().map(|v| v.id.clone()))
        .collect();
    fn steps(list: &[Step], known: &mut BTreeSet<String>) {
        for s in list {
            if let Some(id) = &s.id {
                known.insert(id.clone());
            }
            match &s.kind {
                StepKind::Set { name, .. } | StepKind::Ask { name, .. } => {
                    known.insert(name.clone());
                }
                StepKind::Args(a) => {
                    known.insert(a.name.clone());
                }
                StepKind::HttpGet { extract, .. } => {
                    known.extend(extract.iter().map(|e| e.name.clone()));
                }
                _ => {}
            }
        }
    }
    for a in &config.actions {
        steps(&a.steps, &mut known);
    }
    for c in &config.controls {
        steps(&c.on_change, &mut known);
    }
    for (src, _) in sources {
        let toks = significant_tokens(&src.code);
        for w in toks.windows(3) {
            if let (
                TokenType::Identifier { identifier },
                TokenType::Symbol {
                    symbol: Symbol::LeftParen,
                },
                TokenType::StringLiteral { literal, .. },
            ) = (&w[0].1, &w[1].1, &w[2].1)
            {
                if identifier.as_str() == "set" {
                    known.insert(literal.to_string());
                }
            }
        }
    }
    known
}

/// Tokens without whitespace and comments, with their byte offsets.
fn significant_tokens(code: &str) -> Vec<(usize, TokenType)> {
    let tokens = match Lexer::new(code, LuaVersion::luau()).collect() {
        LexerResult::Ok(t) | LexerResult::Recovered(t, _) => t,
        LexerResult::Fatal(_) => return Vec::new(),
    };
    tokens
        .into_iter()
        .filter(|t| {
            !matches!(
                t.token_type(),
                TokenType::Whitespace { .. }
                    | TokenType::SingleLineComment { .. }
                    | TokenType::MultiLineComment { .. }
            )
        })
        .map(|t| (t.start_position().bytes(), t.token_type().clone()))
        .collect()
}

/// Every issue in a file's Lua: parse errors, lint, and `values.x` names that don't exist.
pub fn lint_config(config: &Config) -> Vec<Issue> {
    let all = sources(config);
    let linter = match Linter::new(config) {
        Ok(l) => l,
        Err(e) => {
            crate::log_warn!("the Lua linter couldn't start: {e}");
            return Vec::new();
        }
    };
    let known = known_values(config, &all);
    let mut out = Vec::new();
    for (src, expr) in &all {
        out.extend(linter.lint(src, *expr));
        let toks = significant_tokens(&src.code);
        for w in toks.windows(3) {
            if let (
                TokenType::Identifier { identifier },
                TokenType::Symbol {
                    symbol: Symbol::Dot,
                },
                TokenType::Identifier { identifier: name },
            ) = (&w[0].1, &w[1].1, &w[2].1)
            {
                if identifier.as_str() == "values" && !known.contains(name.as_str()) {
                    let (l, c) = line_col(&src.code, w[2].0);
                    let (line, col) = file_position(src, l, c);
                    let hint = crate::suggest::did_you_mean(
                        name.as_str(),
                        known.iter().map(String::as_str),
                    )
                    .map(|s| format!(" (did you mean `{s}`?)"))
                    .unwrap_or_default();
                    out.push(Issue {
                        message: format!(
                            "values.{name}: no control, value or run variable is called `{name}`{hint}"
                        ),
                        line,
                        col,
                        error: false,
                        code: "unknown-value".into(),
                    });
                }
            }
        }
    }
    out.sort_by_key(|i| (i.line, i.col));
    out.dedup();
    out
}

/// The issues as diagnostics (for `launcher check` and the editor's Problems): a syntax error is
/// an error, the rest are warnings.
pub fn diagnostics(config: &Config) -> Vec<Diagnostic> {
    lint_config(config)
        .into_iter()
        .map(|i| {
            let (code, message) = match i.code.as_str() {
                "parse" => ("lua-syntax", i.message),
                "unknown-value" => ("unknown-value", i.message),
                lint => ("lua-lint", format!("{} ({lint})", i.message)),
            };
            Diagnostic {
                severity: if i.error {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                code,
                message,
                help: None,
                offset: 0,
                len: 0,
                line: i.line,
                col: i.col,
                path: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn config(text: &str) -> Config {
        let l = load_str(text, Path::new("x.launcher"), &LoadOptions::default());
        assert!(!l.has_errors(), "{:?}", l.diags);
        l.config
    }

    #[test]
    fn the_examples_lint_cleanly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        for f in [
            "git-build/git-build-windows.launcher",
            "git-build/git-build-macos.launcher",
            "unity-build/unity-build-windows.launcher",
            "unity-build/unity-build-macos.launcher",
        ] {
            let text = std::fs::read_to_string(root.join(f)).unwrap();
            let l = load_str(&text, &root.join(f), &LoadOptions::default());
            let issues = lint_config(&l.config);
            assert!(issues.is_empty(), "{f}: {issues:#?}");
        }
    }

    #[test]
    fn issues_name_file_lines() {
        let c = config(
            r##"script #"""
    function greet(name)
        local unused = 1
        return "hi " .. name
    end
    """#
layout {
    text "who" label="Who"
    checkbox "loud" label="Loud" enabled-if="values.woh ~= ''"
}
action "go" {
    lua "greet(values.who, 2); nope()"
    log (lua)"greet(values.who"
    set "x" (lua)"trim()"
}
"##,
        );
        let issues = lint_config(&c);
        let find = |code: &str| {
            issues
                .iter()
                .find(|i| i.code == code)
                .unwrap_or_else(|| panic!("no {code} in {issues:#?}"))
        };
        // The unused local on line 3 of the file.
        let u = find("unused_variable");
        assert_eq!((u.line, u.col), (3, 15));
        // values.woh: a typo, with a hint.
        let v = find("unknown-value");
        assert_eq!(v.line, 9);
        assert!(v.message.contains("did you mean `who`"), "{}", v.message);
        // Too many arguments to a script function isn't selene's business, but an undefined
        // global and the API's argument counts are.
        assert!(issues
            .iter()
            .any(|i| i.code == "undefined_variable" && i.line == 12));
        assert!(issues
            .iter()
            .any(|i| i.code == "incorrect_standard_library_use" && i.line == 14));
        // A syntax error, as an error, on its line.
        let p = find("parse");
        assert!(p.error);
        assert_eq!(p.line, 13);
    }
}

//! The `args` step (N17): a command line built from pieces. Ported from master:
//!
//! * `raw` text is trimmed and added as-is; empty text is skipped. Raw text takes no part in
//!   de-duplication.
//! * `pair K V` renders `format` with `{key}`/`{value}` after applying every `replace` to the
//!   value. A value that is missing (a field no option has) skips the pair; empty text is kept.
//! * `pairs SRC` is a `pair` for each field of a data block, or of the option chosen in a control,
//!   in their written order.
//! * `dedupe first` keeps the first pair of each key, `last` the last (appended where it occurs),
//!   `none` keeps all. Keys compare ignoring case unless `ignore-case=#false`.
//! * The pieces are joined with single spaces.

use crate::config::template::{Part, Template};
use crate::config::{ArgPart, ArgsSpec, Dedupe};

/// What the builder needs from the run: values, fields and conditions.
pub trait ArgsScope {
    /// A template, or None when it is a single placeholder naming something missing.
    fn render(&mut self, v: &crate::config::Val) -> Result<Option<String>, String>;
    fn condition(&mut self, src: &crate::config::LuaSrc) -> Result<bool, String>;
    /// A data block's, or a control's chosen option's, fields in written order.
    fn bag(&mut self, source: &str) -> Option<Vec<(String, String)>>;
}

/// Build the command line: (joined text, the pieces).
pub fn build(spec: &ArgsSpec, scope: &mut dyn ArgsScope) -> Result<(String, Vec<String>), String> {
    let format = spec
        .format
        .clone()
        .unwrap_or_else(|| Template::parse("{key}={value}").expect("valid"));
    let norm = |k: &str| {
        if spec.dedupe_ignore_case {
            k.to_lowercase()
        } else {
            k.to_string()
        }
    };
    // (key for de-duplication, text); raw pieces have no key.
    let mut tokens: Vec<(Option<String>, String)> = Vec::new();
    let add_pair = |tokens: &mut Vec<(Option<String>, String)>, key: &str, value: &str| {
        let mut v = value.to_string();
        for (from, to) in &spec.replace {
            v = v.replace(from.as_str(), to);
        }
        let k = norm(key);
        match spec.dedupe {
            Dedupe::First => {
                if tokens
                    .iter()
                    .any(|(tk, _)| tk.as_deref() == Some(k.as_str()))
                {
                    return;
                }
            }
            Dedupe::Last => tokens.retain(|(tk, _)| tk.as_deref() != Some(k.as_str())),
            Dedupe::None => {}
        }
        let mut text = String::new();
        for p in &format.parts {
            match p {
                Part::Lit(s) => text.push_str(s),
                Part::Ref { name, .. } if name == "key" => text.push_str(key),
                Part::Ref { name, .. } if name == "value" => text.push_str(&v),
                Part::Ref { .. } | Part::Env { .. } => {}
            }
        }
        tokens.push((Some(k), text));
    };
    for part in &spec.parts {
        let cond = match part {
            ArgPart::Raw { if_, .. } | ArgPart::Pair { if_, .. } | ArgPart::Pairs { if_, .. } => {
                if_
            }
        };
        if let Some(c) = cond {
            if !scope.condition(c)? {
                continue;
            }
        }
        match part {
            ArgPart::Raw { text, .. } => {
                let t = scope.render(text)?.unwrap_or_default();
                let t = t.trim();
                if !t.is_empty() {
                    tokens.push((None, t.to_string()));
                }
            }
            ArgPart::Pair { key, value, .. } => {
                if let Some(v) = scope.render(value)? {
                    add_pair(&mut tokens, key, &v);
                }
            }
            ArgPart::Pairs {
                source, exclude, ..
            } => {
                let Some(bag) = scope.bag(source) else {
                    continue;
                };
                for (k, v) in bag {
                    if exclude.iter().any(|e| e.eq_ignore_ascii_case(&k)) {
                        continue;
                    }
                    add_pair(&mut tokens, &k, &v);
                }
            }
        }
    }
    let list: Vec<String> = tokens.into_iter().map(|(_, t)| t).collect();
    Ok((list.join(" "), list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LuaSrc, Val};
    use std::collections::HashMap;

    struct Scope {
        values: HashMap<String, String>,
        bags: HashMap<String, Vec<(String, String)>>,
    }

    impl ArgsScope for Scope {
        fn render(&mut self, v: &Val) -> Result<Option<String>, String> {
            let Val::Text(t) = v else {
                return Ok(None);
            };
            if let [Part::Ref { name, field, .. }] = t.parts.as_slice() {
                let key = match field {
                    Some(f) => format!("{name}.{f}"),
                    None => name.clone(),
                };
                return Ok(self.values.get(&key).cloned());
            }
            Ok(Some(t.raw.clone()))
        }
        fn condition(&mut self, src: &LuaSrc) -> Result<bool, String> {
            Ok(src.code == "true")
        }
        fn bag(&mut self, source: &str) -> Option<Vec<(String, String)>> {
            self.bags.get(source).cloned()
        }
    }

    fn lua(code: &str) -> Option<LuaSrc> {
        Some(LuaSrc {
            code: code.into(),
            repr: String::new(),
            span: Default::default(),
            line: 1,
            col: 1,
        })
    }

    #[test]
    fn builds_like_the_legacy_launcher() {
        let spec = ArgsSpec {
            name: "game_args".into(),
            format: Some(Template::parse(r#"{key}="{value}""#).unwrap()),
            replace: vec![("\"".into(), "'".into())],
            dedupe: Dedupe::First,
            dedupe_ignore_case: true,
            parts: vec![
                ArgPart::Raw {
                    text: Val::literal("-screen-width 1920"),
                    if_: lua("false"),
                },
                ArgPart::Pair {
                    key: "ServerIP".into(),
                    value: Val::Text(Template::parse("{server.ServerIP}").unwrap()),
                    if_: None,
                },
                ArgPart::Pair {
                    key: "SessionID".into(),
                    value: Val::Text(Template::parse("{session_id}").unwrap()),
                    if_: None,
                },
                ArgPart::Pairs {
                    source: "common".into(),
                    exclude: vec!["Skip".into()],
                    if_: None,
                },
                ArgPart::Raw {
                    text: Val::literal("  -extra  "),
                    if_: None,
                },
            ],
        };
        let mut scope = Scope {
            values: [("server.ServerIP".to_string(), "10.0.0.1".to_string())].into(),
            bags: [(
                "common".to_string(),
                vec![
                    ("serverip".to_string(), "ignored".to_string()),
                    ("Videos".to_string(), r#"["a.mp4"]"#.to_string()),
                    ("Skip".to_string(), "x".to_string()),
                ],
            )]
            .into(),
        };
        let (line, list) = build(&spec, &mut scope).unwrap();
        assert_eq!(line, r#"ServerIP="10.0.0.1" Videos="['a.mp4']" -extra"#);
        assert_eq!(list.len(), 3);
    }
}

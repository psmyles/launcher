//! Templates: text with `{name}`, `{name.field}` and `{env.NAME}` placeholders (N4: plain
//! substitution, no expressions; logic is Luau). `{{` and `}}` write a literal brace.

/// One piece of a template.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Lit(String),
    /// `{name}` or `{name.field}`: a control, value, data block or step result, and optionally
    /// one of its fields (an option's data, a JSON path).
    Ref {
        name: String,
        field: Option<String>,
        /// Byte offset of the `{` in the template text.
        offset: usize,
    },
    /// `{env.NAME}`: an environment variable of the run.
    Env {
        name: String,
        offset: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub raw: String,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TemplateError {
    /// Byte offset in the template text.
    pub offset: usize,
    pub message: String,
}

impl Template {
    pub fn literal(s: &str) -> Template {
        Template {
            raw: s.to_string(),
            parts: vec![Part::Lit(s.to_string())],
        }
    }

    pub fn parse(raw: &str) -> Result<Template, TemplateError> {
        let mut parts = Vec::new();
        let mut lit = String::new();
        let bytes = raw.as_bytes();
        let mut i = 0;
        while i < raw.len() {
            let c = raw[i..].chars().next().unwrap_or('\0');
            match c {
                '{' if bytes.get(i + 1) == Some(&b'{') => {
                    lit.push('{');
                    i += 2;
                }
                '}' if bytes.get(i + 1) == Some(&b'}') => {
                    lit.push('}');
                    i += 2;
                }
                '{' => {
                    let Some(end) = raw[i + 1..].find('}') else {
                        return Err(TemplateError {
                            offset: i,
                            message: "a `{` is never closed (write `{{` for a literal brace)"
                                .into(),
                        });
                    };
                    let inner = raw[i + 1..i + 1 + end].trim();
                    if inner.is_empty() {
                        return Err(TemplateError {
                            offset: i,
                            message: "an empty placeholder `{}` (write `{{}}` for literal braces)"
                                .into(),
                        });
                    }
                    let (name, field) = match inner.split_once('.') {
                        Some((n, f)) => (n.trim(), Some(f.trim().to_string())),
                        None => (inner, None),
                    };
                    if !is_name(name) {
                        return Err(TemplateError {
                            offset: i,
                            message: format!(
                                "`{{{inner}}}` is not a placeholder: write {{name}}, {{name.field}} or {{env.NAME}}; Lua expressions go in (lua)\"...\""
                            ),
                        });
                    }
                    if !lit.is_empty() {
                        parts.push(Part::Lit(std::mem::take(&mut lit)));
                    }
                    if name == "env" {
                        let Some(var) = field.filter(|f| !f.is_empty()) else {
                            return Err(TemplateError {
                                offset: i,
                                message: "`{env}` needs a variable name: {env.NAME}".into(),
                            });
                        };
                        parts.push(Part::Env {
                            name: var,
                            offset: i,
                        });
                    } else {
                        parts.push(Part::Ref {
                            name: name.to_string(),
                            field,
                            offset: i,
                        });
                    }
                    i += end + 2;
                }
                c => {
                    lit.push(c);
                    i += c.len_utf8();
                }
            }
        }
        if !lit.is_empty() || parts.is_empty() {
            parts.push(Part::Lit(lit));
        }
        Ok(Template {
            raw: raw.to_string(),
            parts,
        })
    }

    /// No placeholders.
    pub fn is_literal(&self) -> bool {
        self.parts.iter().all(|p| matches!(p, Part::Lit(_)))
    }

    /// The names this template refers to (not `env`).
    pub fn refs(&self) -> impl Iterator<Item = (&str, Option<&str>, usize)> {
        self.parts.iter().filter_map(|p| match p {
            Part::Ref {
                name,
                field,
                offset,
            } => Some((name.as_str(), field.as_deref(), *offset)),
            _ => None,
        })
    }

    /// Fill the placeholders. `lookup(name, field)` gives a reference's text; `env(name)` an
    /// environment variable. Unknown references and variables become empty text.
    pub fn render(
        &self,
        lookup: &dyn Fn(&str, Option<&str>) -> Option<String>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> String {
        let mut out = String::new();
        for p in &self.parts {
            match p {
                Part::Lit(s) => out.push_str(s),
                Part::Ref { name, field, .. } => {
                    out.push_str(&lookup(name, field.as_deref()).unwrap_or_default())
                }
                Part::Env { name, .. } => out.push_str(&env(name).unwrap_or_default()),
            }
        }
        out
    }
}

/// A placeholder name: a letter or `_`, then letters, digits, `_` and `-`.
pub fn is_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_alphabetic() || f == '_')
        && c.all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-')
}

/// `%NAME%` spans in text (cmd.exe variable syntax, which templates don't expand): the byte offset
/// and the name, for the `{env.NAME}` lint.
pub fn percent_vars(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(start) = text[i..].find('%').map(|s| s + i) {
        let Some(len) = text[start + 1..].find('%') else {
            break;
        };
        let name = &text[start + 1..start + 1 + len];
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '(' || c == ')')
            && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        {
            out.push((start, name.to_string()));
            i = start + len + 2;
        } else {
            i = start + 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_placeholders() {
        let t = Template::parse("-screen {role.ScreenId} at {env.HOME}/x {{literal}}").unwrap();
        assert_eq!(
            t.parts,
            vec![
                Part::Lit("-screen ".into()),
                Part::Ref {
                    name: "role".into(),
                    field: Some("ScreenId".into()),
                    offset: 8
                },
                Part::Lit(" at ".into()),
                Part::Env {
                    name: "HOME".into(),
                    offset: 27
                },
                Part::Lit("/x {literal}".into()),
            ]
        );
        let out = t.render(&|n, f| Some(format!("<{n}:{}>", f.unwrap_or(""))), &|v| {
            Some(format!("${v}"))
        });
        assert_eq!(out, "-screen <role:ScreenId> at $HOME/x {literal}");
    }

    #[test]
    fn rejects_bad_placeholders() {
        assert!(Template::parse("a {b").is_err());
        assert!(Template::parse("a {}").is_err());
        assert!(Template::parse("{if(x, 1, 2)}").is_err());
        assert!(Template::parse("{env}").is_err());
        assert!(Template::parse("plain").unwrap().is_literal());
        assert!(Template::parse("").unwrap().is_literal());
    }

    #[test]
    fn finds_cmd_style_variables() {
        assert_eq!(
            percent_vars("%USERPROFILE%\\x 50% and 60%"),
            vec![(0, "USERPROFILE".to_string())]
        );
        assert!(percent_vars("100% done").is_empty());
    }
}

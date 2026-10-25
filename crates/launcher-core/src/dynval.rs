//! `Dyn`: a plain value that crosses between Lua, templates, the form and step results.

use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Dyn {
    #[default]
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Dyn>),
    Map(BTreeMap<String, Dyn>),
}

impl Dyn {
    pub fn str(s: impl Into<String>) -> Dyn {
        Dyn::Str(s.into())
    }

    /// Lua truthiness: only nil and false are false.
    pub fn truthy(&self) -> bool {
        !matches!(self, Dyn::Nil | Dyn::Bool(false))
    }

    /// The text a template shows for this value.
    pub fn to_text(&self) -> String {
        match self {
            Dyn::Nil => String::new(),
            Dyn::Bool(b) => b.to_string(),
            Dyn::Num(n) => fmt_num(*n),
            Dyn::Str(s) => s.clone(),
            Dyn::List(l) => l.iter().map(Dyn::to_text).collect::<Vec<_>>().join(","),
            Dyn::Map(_) => serde_json::to_string(&self.to_json()).unwrap_or_default(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value as J;
        match self {
            Dyn::Nil => J::Null,
            Dyn::Bool(b) => J::Bool(*b),
            Dyn::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 9.0e15 {
                    J::from(*n as i64)
                } else {
                    serde_json::Number::from_f64(*n).map_or(J::Null, J::Number)
                }
            }
            Dyn::Str(s) => J::String(s.clone()),
            Dyn::List(l) => J::Array(l.iter().map(Dyn::to_json).collect()),
            Dyn::Map(m) => J::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
        }
    }

    pub fn from_json(j: &serde_json::Value) -> Dyn {
        use serde_json::Value as J;
        match j {
            J::Null => Dyn::Nil,
            J::Bool(b) => Dyn::Bool(*b),
            J::Number(n) => Dyn::Num(n.as_f64().unwrap_or(0.0)),
            J::String(s) => Dyn::Str(s.clone()),
            J::Array(a) => Dyn::List(a.iter().map(Dyn::from_json).collect()),
            J::Object(o) => Dyn::Map(
                o.iter()
                    .map(|(k, v)| (k.clone(), Dyn::from_json(v)))
                    .collect(),
            ),
        }
    }

    /// A field by name (maps only), case-insensitive as data keys are.
    pub fn field(&self, key: &str) -> Option<&Dyn> {
        match self {
            Dyn::Map(m) => m.get(key).or_else(|| {
                m.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(key))
                    .map(|(_, v)| v)
            }),
            _ => None,
        }
    }
}

/// Numbers as people write them: `3`, not `3.0`.
pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_truthiness() {
        assert_eq!(Dyn::Num(3.0).to_text(), "3");
        assert_eq!(Dyn::Num(0.5).to_text(), "0.5");
        assert_eq!(
            Dyn::List(vec![Dyn::str("a"), Dyn::str("b")]).to_text(),
            "a,b"
        );
        assert!(Dyn::str("").truthy());
        assert!(!Dyn::Nil.truthy() && !Dyn::Bool(false).truthy());
        let j = serde_json::json!({"a": [1, 2.5, "x"], "b": null});
        assert_eq!(Dyn::from_json(&j).to_json(), j);
    }
}

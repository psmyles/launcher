//! A `table` control's rows: what the evaluator makes of `rows=(lua)`, and how the window
//! groups them. A table is a choice underneath: each row is one of its options, a button chooses
//! one, and steps read its fields as `{table.field}`. Nothing here knows any row format; the
//! tool's own Lua shapes the rows.

use std::collections::BTreeMap;

use crate::dynval::Dyn;
use crate::eval::OptionItem;

/// One group of rows: the `group-by` field's value they share, and the rows' places in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub key: String,
    pub rows: Vec<usize>,
}

/// The rows from a `rows=(lua)` list: tables with any fields (strings and numbers become rows
/// with only a name). A row's value is its `value` field, or else its number from 1, so each
/// row can be chosen on its own.
pub fn rows_from(d: Dyn) -> Result<Vec<OptionItem>, String> {
    let Dyn::List(items) = d else {
        return Err("the rows expression must return a list".into());
    };
    items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let number = (i + 1).to_string();
            match item {
                Dyn::Map(mut m) => {
                    let value = m.remove("value").map_or(number, |v| v.to_text());
                    let name = m
                        .remove("name")
                        .map_or_else(|| value.clone(), |v| v.to_text());
                    let label = m.remove("label").map(|v| v.to_text());
                    Ok(OptionItem {
                        name,
                        value,
                        label,
                        fields: m,
                    })
                }
                Dyn::Str(_) | Dyn::Num(_) => Ok(OptionItem {
                    name: item.to_text(),
                    value: number,
                    label: None,
                    fields: BTreeMap::new(),
                }),
                other => Err(format!(
                    "row {} must be a table of fields, not {}",
                    i + 1,
                    other.to_text()
                )),
            }
        })
        .collect()
}

/// A row's field as text ("" when it has none): its own fields, then name, value and label.
/// The field's case doesn't matter, as in `{table.field}`.
pub fn field(row: &OptionItem, name: &str) -> String {
    let found = row.fields.get(name).or_else(|| {
        row.fields
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    });
    match found {
        Some(v) => v.to_text(),
        None => match name.to_ascii_lowercase().as_str() {
            "name" => row.name.clone(),
            "value" => row.value.clone(),
            "label" => row.label.clone().unwrap_or_else(|| row.name.clone()),
            _ => String::new(),
        },
    }
}

/// The rows in groups by the `group-by` field, in the order each group first appears; without
/// `group-by`, one group of every row.
pub fn groups(rows: &[OptionItem], group_by: Option<&str>) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let key = group_by.map(|f| field(row, f)).unwrap_or_default();
        match out.iter_mut().find(|g| g.key == key) {
            Some(g) => g.rows.push(i),
            None => out.push(Group { key, rows: vec![i] }),
        }
    }
    out
}

/// A row's mark, from its `icon-from` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Error,
    Warning,
    Info,
}

impl Mark {
    /// "error", "warning" (or "warn") and "info", in any case; anything else has no mark.
    pub fn from_word(word: &str) -> Option<Mark> {
        match word.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Mark::Error),
            "warning" | "warn" => Some(Mark::Warning),
            "info" => Some(Mark::Info),
            _ => None,
        }
    }
}

/// Where the evaluator keeps a row's `(lua)` texts: the table's node, and this property.
pub fn text_key(prop: &str, row: usize) -> String {
    format!("{prop}:{row}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> Dyn {
        Dyn::Map(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), Dyn::str(*v)))
                .collect(),
        )
    }

    #[test]
    fn rows_need_no_name_and_are_numbered() {
        let rows = rows_from(Dyn::List(vec![
            map(&[("path", "a.png"), ("title", "Too large")]),
            map(&[("path", "a.png"), ("value", "own")]),
            Dyn::str("plain"),
        ]))
        .unwrap();
        assert_eq!(rows[0].value, "1");
        assert_eq!(rows[0].name, "1");
        assert_eq!(field(&rows[0], "TITLE"), "Too large");
        assert_eq!(field(&rows[0], "value"), "1");
        assert_eq!(field(&rows[0], "missing"), "");
        assert_eq!(rows[1].value, "own");
        assert_eq!(rows[2].value, "3");
        assert_eq!(rows[2].name, "plain");
        assert!(rows_from(Dyn::str("x")).is_err());
        assert!(rows_from(Dyn::List(vec![Dyn::Bool(true)])).is_err());
    }

    #[test]
    fn groups_keep_the_order_they_first_appear_in() {
        let rows = rows_from(Dyn::List(vec![
            map(&[("title", "run-level")]),
            map(&[("path", "b.fbx")]),
            map(&[("path", "a.png")]),
            map(&[("path", "b.fbx")]),
        ]))
        .unwrap();
        let g = groups(&rows, Some("path"));
        assert_eq!(
            g,
            [
                Group {
                    key: String::new(),
                    rows: vec![0]
                },
                Group {
                    key: "b.fbx".into(),
                    rows: vec![1, 3]
                },
                Group {
                    key: "a.png".into(),
                    rows: vec![2]
                },
            ]
        );
        assert_eq!(groups(&rows, None).len(), 1);
    }

    #[test]
    fn marks_come_from_words() {
        assert_eq!(Mark::from_word("Error"), Some(Mark::Error));
        assert_eq!(Mark::from_word("warn"), Some(Mark::Warning));
        assert_eq!(Mark::from_word(" info "), Some(Mark::Info));
        assert_eq!(Mark::from_word("note"), None);
    }
}

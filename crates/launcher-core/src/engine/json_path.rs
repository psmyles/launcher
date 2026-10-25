//! `extract "name" "data[0].id"`: a field of a JSON document by path (ported from master).
//! Dots and brackets both work: `a.b[0].c`, `data.0.id`, `["a.b"].c`.

pub fn get<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    let b = path.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'.' => i += 1,
            b'[' => {
                let close = path[i..].find(']')? + i;
                let inner = path[i + 1..close].trim();
                cur = if let Some(q) = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    cur.get(q)?
                } else if let Some(q) = inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
                {
                    cur.get(q)?
                } else {
                    let n: usize = inner.parse().ok()?;
                    cur.get(n)?
                };
                i = close + 1;
            }
            _ => {
                let end = path[i..].find(['.', '[']).map_or(b.len(), |e| e + i);
                let key = &path[i..end];
                cur = match cur {
                    serde_json::Value::Array(a) => a.get(key.parse::<usize>().ok()?)?,
                    other => other.get(key)?,
                };
                i = end;
            }
        }
    }
    if cur.is_null() {
        return None;
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_paths() {
        let j: serde_json::Value =
            serde_json::from_str(r#"{"data":[{"id":"s1"}],"a.b":{"c":1},"n":null}"#).unwrap();
        assert_eq!(get(&j, "data[0].id").unwrap(), "s1");
        assert_eq!(get(&j, "data.0.id").unwrap(), "s1");
        assert_eq!(get(&j, r#"["a.b"].c"#).unwrap(), 1);
        assert!(get(&j, "data[1].id").is_none());
        assert!(get(&j, "n").is_none());
        assert!(get(&j, "missing").is_none());
    }
}

//! Per-user state (N15): remembered control values and window placement, one small KDL file per
//! config at `<app data>/state/<stem>-<hash>.kdl` (the hash is [`paths::config_hash`]). The home
//! screen, which has no config, uses `state/home.kdl`.
//!
//! Rules: anything missing, unreadable or unknown falls back to defaults (a bad state file never
//! stops the launcher); saves are atomic; sizes are sanity-checked. Headless runs never write
//! state.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kdl::{KdlDocument, KdlValue};

use crate::paths;

pub const STATE_VERSION: u32 = 1;

/// A window rect: physical outer position and inner size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A remembered control value.
#[derive(Clone, Debug, PartialEq)]
pub enum Stored {
    Bool(bool),
    Num(f64),
    Text(String),
    List(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub window: Option<Rect>,
    pub maximized: bool,
    pub editor: Option<Rect>,
    pub editor_maximized: bool,
    pub values: BTreeMap<String, Stored>,
    /// Choices steps filled in (`put`), by control.
    pub choices: BTreeMap<String, Vec<String>>,
}

/// The state file for a config.
pub fn state_path(config: &Path) -> Option<PathBuf> {
    Some(paths::app_data_dir()?.join("state").join(format!(
        "{}-{}.kdl",
        paths::config_stem(config),
        paths::config_hash(config)
    )))
}

/// The home screen's state file.
pub fn home_state_path() -> Option<PathBuf> {
    Some(paths::app_data_dir()?.join("state").join("home.kdl"))
}

pub fn load(path: &Path) -> State {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(_) => State::default(),
    }
}

/// `about` names what the state belongs to, in the file's header comment.
pub fn save(path: &Path, about: &str, st: &State) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    crate::replace_file::write_bytes_replacing(path, render(about, st))
        .map_err(|e| format!("cannot save {}: {e}", path.display()))
}

fn sane(r: Rect) -> Option<Rect> {
    let ok = |v: u32| (100..=32767).contains(&v);
    let pos = |v: i32| (-32768..=32767).contains(&v);
    (ok(r.width) && ok(r.height) && pos(r.x) && pos(r.y)).then_some(r)
}

pub fn parse(text: &str) -> State {
    let mut st = State::default();
    let Ok(doc) = KdlDocument::parse_v2(text) else {
        crate::log_warn!("the state file is not readable; starting from defaults");
        return st;
    };
    for n in doc.nodes() {
        let args: Vec<&KdlValue> = n
            .entries()
            .iter()
            .filter(|e| e.name().is_none())
            .map(|e| e.value())
            .collect();
        let prop = |name: &str| n.get(name);
        let int = |name: &str| prop(name).and_then(|v| v.as_integer());
        let rect = || -> Option<Rect> {
            sane(Rect {
                x: int("x")?.try_into().ok()?,
                y: int("y")?.try_into().ok()?,
                width: int("width")?.try_into().ok()?,
                height: int("height")?.try_into().ok()?,
            })
        };
        let maximized = prop("maximized").and_then(|v| v.as_bool()).unwrap_or(false);
        match n.name().value() {
            "window" => {
                st.window = rect();
                st.maximized = maximized;
            }
            "editor" => {
                st.editor = rect();
                st.editor_maximized = maximized;
            }
            "value" => {
                let (Some(KdlValue::String(id)), Some(v)) = (args.first(), args.get(1)) else {
                    continue;
                };
                let stored = match v {
                    KdlValue::Bool(b) => Stored::Bool(*b),
                    KdlValue::Integer(i) => Stored::Num(*i as f64),
                    KdlValue::Float(f) => Stored::Num(*f),
                    KdlValue::String(s) => Stored::Text(s.clone()),
                    KdlValue::Null => continue,
                };
                st.values.insert(id.clone(), stored);
            }
            "list" => {
                let Some(KdlValue::String(id)) = args.first() else {
                    continue;
                };
                let items = args[1..]
                    .iter()
                    .filter_map(|v| v.as_string().map(str::to_string))
                    .collect();
                st.values.insert(id.clone(), Stored::List(items));
            }
            "choices" => {
                let Some(KdlValue::String(id)) = args.first() else {
                    continue;
                };
                let items = args[1..]
                    .iter()
                    .filter_map(|v| v.as_string().map(str::to_string))
                    .collect();
                st.choices.insert(id.clone(), items);
            }
            _ => {}
        }
    }
    st
}

/// A KDL string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn num(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        format!("{}", f as i64)
    } else {
        // Four decimals is plenty for a slider and keeps the file stable.
        let s = format!("{:.4}", f);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s.contains('.') {
            s.to_string()
        } else {
            format!("{s}.0")
        }
    }
}

pub fn render(about: &str, st: &State) -> String {
    let mut out = String::new();
    let about = about.replace(['\r', '\n'], " ");
    let _ = writeln!(
        out,
        "// Launcher state for {about} (written by the launcher; safe to delete)"
    );
    let _ = writeln!(out, "state version={STATE_VERSION}");
    let rect = |out: &mut String, name: &str, r: Option<Rect>, max: bool| {
        if let Some(r) = r {
            let _ = writeln!(
                out,
                "{name} x={} y={} width={} height={} maximized={}",
                r.x,
                r.y,
                r.width,
                r.height,
                if max { "#true" } else { "#false" }
            );
        }
    };
    rect(&mut out, "window", st.window, st.maximized);
    rect(&mut out, "editor", st.editor, st.editor_maximized);
    for (k, v) in &st.values {
        let line = match v {
            Stored::Bool(b) => {
                format!("value {} {}", quote(k), if *b { "#true" } else { "#false" })
            }
            Stored::Num(f) => format!("value {} {}", quote(k), num(*f)),
            Stored::Text(t) => format!("value {} {}", quote(k), quote(t)),
            Stored::List(l) => {
                let mut s = format!("list {}", quote(k));
                for i in l {
                    s.push(' ');
                    s.push_str(&quote(i));
                }
                s
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    for (k, items) in &st.choices {
        let mut s = format!("choices {}", quote(k));
        for i in items {
            s.push(' ');
            s.push_str(&quote(i));
        }
        out.push_str(&s);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut st = State {
            window: Some(Rect {
                x: -8,
                y: 20,
                width: 420,
                height: 330,
            }),
            maximized: false,
            ..State::default()
        };
        st.values
            .insert("role".into(), Stored::Text("Player \"1\" \\ x".into()));
        st.values.insert("custom_res".into(), Stored::Bool(true));
        st.values.insert("volume".into(), Stored::Num(80.0));
        st.values.insert("gain".into(), Stored::Num(0.25));
        st.values.insert(
            "checks".into(),
            Stored::List(vec!["a".into(), "b c".into()]),
        );
        st.choices
            .insert("branch".into(), vec!["main".into(), "feature/x".into()]);
        let text = render("C:\\x\\build.launcher", &st);
        assert_eq!(parse(&text), st);
    }

    #[test]
    fn junk_falls_back_to_defaults() {
        assert_eq!(parse("{{{"), State::default());
        let st = parse("window x=0 y=0 width=5 height=99999\nvalue \"a\"\n");
        assert_eq!(st.window, None);
        assert!(st.values.is_empty());
    }

    #[test]
    fn the_state_file_is_named_after_the_config() {
        let p = state_path(Path::new("some/dir/My Build.launcher")).unwrap();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("My_Build-") && name.ends_with(".kdl"),
            "{name}"
        );
        assert_eq!(p.parent().unwrap().file_name().unwrap(), "state");
    }
}

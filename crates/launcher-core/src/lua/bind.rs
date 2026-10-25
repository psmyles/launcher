//! The bindings behind [`super::api::API`]. Installed before the sandbox freezes the globals.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use mlua::{Lua, MultiValue, Table, Value};

#[cfg(test)]
use super::api;
use super::{Fail, Shared};
use crate::dynval::Dyn;
use crate::engine::effects::CommandSpec;
use crate::platform::process::exit_code_text;

/// Lua → Dyn. A table with only 1..n keys is a list; any other table is a map (string keys).
pub fn to_dyn(v: &Value) -> Dyn {
    match v {
        Value::Nil => Dyn::Nil,
        Value::Boolean(b) => Dyn::Bool(*b),
        Value::Integer(i) => Dyn::Num(*i as f64),
        Value::Number(n) => Dyn::Num(*n),
        Value::String(s) => Dyn::Str(s.to_string_lossy()),
        Value::Table(t) => {
            let len = t.raw_len();
            let mut count = 0usize;
            let mut map = std::collections::BTreeMap::new();
            for pair in t.clone().pairs::<Value, Value>().flatten() {
                count += 1;
                let key = match &pair.0 {
                    Value::String(s) => s.to_string_lossy(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => crate::dynval::fmt_num(*n),
                    other => format!("{other:?}"),
                };
                map.insert(key, to_dyn(&pair.1));
            }
            if len > 0 && count == len {
                Dyn::List(
                    (1..=len)
                        .map(|i| {
                            t.raw_get::<Value>(i)
                                .map(|v| to_dyn(&v))
                                .unwrap_or_default()
                        })
                        .collect(),
                )
            } else if count == 0 {
                Dyn::List(Vec::new())
            } else {
                Dyn::Map(map)
            }
        }
        other => Dyn::Str(other.type_name().to_string()),
    }
}

/// Dyn → Lua.
pub fn from_dyn(lua: &Lua, d: &Dyn) -> mlua::Result<Value> {
    Ok(match d {
        Dyn::Nil => Value::Nil,
        Dyn::Bool(b) => Value::Boolean(*b),
        Dyn::Num(n) => Value::Number(*n),
        Dyn::Str(s) => Value::String(lua.create_string(s)?),
        Dyn::List(l) => {
            let t = lua.create_table_with_capacity(l.len(), 0)?;
            for (i, v) in l.iter().enumerate() {
                t.raw_set(i + 1, from_dyn(lua, v)?)?;
            }
            Value::Table(t)
        }
        Dyn::Map(m) => {
            let t = lua.create_table_with_capacity(0, m.len())?;
            for (k, v) in m {
                t.raw_set(k.as_str(), from_dyn(lua, v)?)?;
            }
            Value::Table(t)
        }
    })
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.to_string_lossy()),
        Value::Integer(i) => Some(i.to_string()),
        Value::Number(n) => Some(crate::dynval::fmt_num(*n)),
        Value::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Refuse run-time functions while a file loads (no run is going, so there is no host).
fn run_only(shared: &Shared, name: &str) -> mlua::Result<()> {
    if shared.host.lock().is_ok_and(|h| h.is_none()) {
        return Err(mlua::Error::runtime(format!(
            "{name}() can't run while the file loads; use it in an action"
        )));
    }
    if shared.instant.load(Ordering::Relaxed) && name != "set" {
        return Err(mlua::Error::runtime(format!(
            "{name}() can't be used in on-change; only set() and log() work there"
        )));
    }
    Ok(())
}

/// `program, arg..., {options}?` → a command.
/// What `command()` read: the command, whether a non-zero exit is an error, and `io="pipe"`.
struct Command {
    spec: CommandSpec,
    check: bool,
    pipe: bool,
}

fn command(args: MultiValue, what: &str) -> mlua::Result<Command> {
    let mut items: Vec<Value> = args.into_iter().collect();
    let options = match items.last() {
        Some(Value::Table(t)) => {
            let t = t.clone();
            items.pop();
            Some(t)
        }
        _ => None,
    };
    let mut it = items.iter();
    let program = it
        .next()
        .and_then(text)
        .ok_or_else(|| mlua::Error::runtime(format!("{what} needs a program")))?;
    let mut spec = CommandSpec {
        program,
        ..Default::default()
    };
    for a in it {
        spec.args.push(
            text(a)
                .ok_or_else(|| mlua::Error::runtime(format!("{what}: arguments must be text")))?,
        );
    }
    let mut check = true;
    let mut pipe = false;
    if let Some(o) = options {
        check = o.get::<Option<bool>>("check")?.unwrap_or(true);
        pipe = match o.get::<Option<String>>("io")?.as_deref() {
            None | Some("auto") | Some("pty") => false,
            Some("pipe") => true,
            Some(other) => {
                return Err(mlua::Error::runtime(format!(
                    "{what}: io must be \"auto\" or \"pipe\", not \"{other}\""
                )))
            }
        };
        spec.cwd = o.get::<Option<String>>("cwd")?.map(Into::into);
        spec.stdin = o.get::<Option<String>>("stdin")?;
        spec.timeout = o
            .get::<Option<f64>>("timeout")?
            .map(|ms| Duration::from_millis(ms.max(0.0) as u64));
        if let Some(env) = o.get::<Option<Table>>("env")? {
            for pair in env.pairs::<String, String>() {
                spec.env.push(pair?);
            }
        }
    }
    Ok(Command { spec, check, pipe })
}

/// A read-only proxy table: `__index` reads live data, writing raises.
fn proxy(
    lua: &Lua,
    what: &'static str,
    get: impl Fn(&Lua, String) -> mlua::Result<Value> + Send + 'static,
) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let mt = lua.create_table()?;
    mt.set(
        "__index",
        lua.create_function(move |lua, (_t, k): (Value, String)| get(lua, k))?,
    )?;
    mt.set(
        "__newindex",
        lua.create_function(move |_, _: MultiValue| -> mlua::Result<()> {
            Err(mlua::Error::runtime(format!(
                "{what} is read-only (change values with set() or a set-control step)"
            )))
        })?,
    )?;
    t.set_metatable(Some(mt))?;
    Ok(t)
}

pub fn install(lua: &Lua, shared: &Arc<Shared>, file: &Path, dir: &Path) -> mlua::Result<()> {
    let g = lua.globals();
    let s = shared.clone();

    // ---- the form and the run ------------------------------------------------------------------
    let sv = s.clone();
    g.set(
        "values",
        proxy(lua, "values", move |lua, k| {
            let v = sv
                .snapshot
                .lock()
                .ok()
                .and_then(|s| s.values.get(&k).cloned())
                .unwrap_or_default();
            from_dyn(lua, &v)
        })?,
    )?;
    // An add-on step's (or list's) settings: the same view, named for what it holds there.
    let ss = s.clone();
    g.set(
        "settings",
        proxy(lua, "settings", move |lua, k| {
            let v = ss
                .snapshot
                .lock()
                .ok()
                .and_then(|s| s.values.get(&k).cloned())
                .unwrap_or_default();
            from_dyn(lua, &v)
        })?,
    )?;
    let sf = s.clone();
    g.set(
        "fields",
        proxy(lua, "fields", move |lua, k| {
            let v = sf
                .snapshot
                .lock()
                .ok()
                .and_then(|s| s.fields.get(&k).cloned());
            match v {
                Some(m) => from_dyn(lua, &Dyn::Map(m)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    let se = s.clone();
    g.set(
        "env",
        proxy(lua, "env", move |lua, k| {
            let v = se.cx.lock().ok().and_then(|cx| {
                cx.env.get(&k).cloned().or_else(|| {
                    // Windows variable names are case-insensitive.
                    cfg!(windows)
                        .then(|| {
                            cx.env
                                .iter()
                                .find(|(n, _)| n.eq_ignore_ascii_case(&k))
                                .map(|(_, v)| v.clone())
                        })
                        .flatten()
                })
            });
            match v {
                Some(v) => Ok(Value::String(lua.create_string(v)?)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    g.set("config_dir", dir.display().to_string())?;
    g.set("config_file", file.display().to_string())?;
    let os: Table = g.get("os")?;
    os.set("name", if cfg!(windows) { "windows" } else { "macos" })?;

    // ---- programs --------------------------------------------------------------------------------
    let sc = s.clone();
    g.set(
        "capture",
        lua.create_function(move |_, args: MultiValue| {
            let Command {
                mut spec, check, ..
            } = command(args, "capture()")?;
            // While a file loads, a capture gets whatever is left of the budget.
            if let Some(left) = sc.remaining() {
                spec.timeout = Some(spec.timeout.map_or(left, |t| t.min(left)));
            }
            let out = sc
                .effects
                .capture(&sc.cx(), &spec, &|| sc.cancel.load(Ordering::Relaxed))
                .map_err(mlua::Error::runtime)?;
            if check && out.exit_code != 0 {
                let detail = out
                    .stderr
                    .trim()
                    .lines()
                    .last()
                    .map(|l| format!(": {l}"))
                    .unwrap_or_default();
                return Err(mlua::Error::runtime(format!(
                    "`{}` exited with code {}{detail}",
                    spec.display(),
                    exit_code_text(out.exit_code)
                )));
            }
            let stdout = out.stdout.trim_end_matches(['\r', '\n']).to_string();
            Ok(stdout)
        })?,
    )?;
    let sr = s.clone();
    g.set(
        "run",
        lua.create_function(move |lua, args: MultiValue| {
            run_only(&sr, "run")?;
            let Command { spec, check, pipe } = command(args, "run()")?;
            let code = sr
                .with_host("run()", |h| h.run(&spec, pipe))?
                .map_err(mlua::Error::runtime)?;
            if check && code != 0 {
                return Err(mlua::Error::runtime(format!(
                    "`{}` exited with code {}",
                    spec.display(),
                    exit_code_text(code)
                )));
            }
            let t = lua.create_table()?;
            t.set("exit_code", code)?;
            Ok(t)
        })?,
    )?;
    let ss = s.clone();
    g.set(
        "step",
        lua.create_function(move |_, (label, f): (String, mlua::Function)| {
            run_only(&ss, "step")?;
            ss.with_host("step()", |h| h.step(Some(&label), true))?;
            let r = f.call::<MultiValue>(());
            ss.with_host("step()", |h| h.step(None, r.is_ok()))?;
            r
        })?,
    )?;

    // ---- files -------------------------------------------------------------------------------------
    let sx = s.clone();
    g.set(
        "exists",
        lua.create_function(move |_, p: String| Ok(sx.effects.exists(&sx.cx(), &p)))?,
    )?;
    let sg = s.clone();
    g.set(
        "glob",
        lua.create_function(move |_, p: String| Ok(sg.effects.glob(&sg.cx(), &p)))?,
    )?;
    let srf = s.clone();
    g.set(
        "read_file",
        lua.create_function(move |_, p: String| {
            srf.effects
                .read_file(&srf.cx(), &p)
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    let sw = s.clone();
    g.set(
        "write_file",
        lua.create_function(move |_, (p, t): (String, String)| {
            run_only(&sw, "write_file")?;
            sw.effects
                .write_file(&sw.cx(), &p, &t)
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    let s2 = s.clone();
    g.set(
        "copy",
        lua.create_function(move |_, (a, b): (String, String)| {
            run_only(&s2, "copy")?;
            s2.effects
                .copy(&s2.cx(), &a, &b)
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    let s3 = s.clone();
    g.set(
        "move",
        lua.create_function(move |_, (a, b): (String, String)| {
            run_only(&s3, "move")?;
            s3.effects
                .rename(&s3.cx(), &a, &b)
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    let s4 = s.clone();
    g.set(
        "delete",
        lua.create_function(move |_, p: String| {
            run_only(&s4, "delete")?;
            s4.effects
                .delete(&s4.cx(), &p)
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    let s5 = s.clone();
    g.set(
        "mkdir",
        lua.create_function(move |_, p: String| {
            run_only(&s5, "mkdir")?;
            s5.effects.mkdir(&s5.cx(), &p).map_err(mlua::Error::runtime)
        })?,
    )?;

    // ---- path ------------------------------------------------------------------------------------
    let path = lua.create_table()?;
    path.set(
        "join",
        lua.create_function(|_, parts: MultiValue| {
            let mut p = std::path::PathBuf::new();
            for (i, v) in parts.iter().enumerate() {
                p.push(text(v).ok_or_else(|| {
                    mlua::Error::runtime(format!("path.join: part {} is not text", i + 1))
                })?);
            }
            Ok(p.display().to_string())
        })?,
    )?;
    let part = |f: fn(&Path) -> Option<String>| {
        move |_: &Lua, p: String| Ok(f(Path::new(&p)).unwrap_or_default())
    };
    path.set(
        "dirname",
        lua.create_function(part(|p| p.parent().map(|d| d.display().to_string())))?,
    )?;
    path.set(
        "basename",
        lua.create_function(part(|p| {
            p.file_name().map(|n| n.to_string_lossy().into_owned())
        }))?,
    )?;
    path.set(
        "stem",
        lua.create_function(part(|p| {
            p.file_stem().map(|n| n.to_string_lossy().into_owned())
        }))?,
    )?;
    path.set(
        "ext",
        lua.create_function(part(|p| {
            p.extension().map(|n| n.to_string_lossy().into_owned())
        }))?,
    )?;
    let sp = s.clone();
    path.set(
        "absolute",
        lua.create_function(move |_, p: String| Ok(sp.cx().resolve(&p).display().to_string()))?,
    )?;
    g.set("path", path)?;

    // ---- data --------------------------------------------------------------------------------------
    let json = lua.create_table()?;
    json.set(
        "encode",
        lua.create_function(|_, v: Value| {
            serde_json::to_string(&to_dyn(&v).to_json()).map_err(mlua::Error::external)
        })?,
    )?;
    json.set(
        "decode",
        lua.create_function(|lua, t: String| {
            let j: serde_json::Value = serde_json::from_str(&t)
                .map_err(|e| mlua::Error::runtime(format!("json.decode: {e}")))?;
            from_dyn(lua, &Dyn::from_json(&j))
        })?,
    )?;
    g.set("json", json)?;
    let sh = s.clone();
    g.set(
        "http_get",
        lua.create_function(move |lua, (url, opts): (String, Option<Table>)| {
            run_only(&sh, "http_get")?;
            let mut timeout = Duration::from_secs(10);
            let mut headers = Vec::new();
            if let Some(o) = opts {
                if let Some(ms) = o.get::<Option<f64>>("timeout")? {
                    timeout = Duration::from_millis(ms.max(0.0) as u64);
                }
                if let Some(h) = o.get::<Option<Table>>("headers")? {
                    for pair in h.pairs::<String, String>() {
                        headers.push(pair?);
                    }
                }
            }
            let r = sh
                .effects
                .http_get(None, &url, &headers, timeout)
                .map_err(mlua::Error::runtime)?;
            let t = lua.create_table()?;
            t.set("status", r.status)?;
            t.set("body", r.body)?;
            Ok(t)
        })?,
    )?;
    g.set(
        "trim",
        lua.create_function(|_, s: Option<String>| Ok(s.unwrap_or_default().trim().to_string()))?,
    )?;
    g.set(
        "split",
        lua.create_function(|_, (s, sep): (String, String)| {
            if sep.is_empty() {
                return Err(mlua::Error::runtime("split: the separator is empty"));
            }
            Ok(s.split(sep.as_str())
                .map(str::to_string)
                .collect::<Vec<_>>())
        })?,
    )?;

    // ---- talking to the user -------------------------------------------------------------------
    let sl = s.clone();
    g.set(
        "log",
        lua.create_function(move |_, args: MultiValue| {
            let line = args
                .iter()
                .map(|v| text(v).unwrap_or_else(|| format!("{:?}", to_dyn(v))))
                .collect::<Vec<_>>()
                .join(" ");
            let logged = sl
                .host
                .lock()
                .ok()
                .and_then(|mut h| h.as_mut().map(|h| h.log(&line)));
            if logged.is_none() {
                sl.effects.log(&line);
            }
            Ok(())
        })?,
    )?;
    let sm = s.clone();
    g.set(
        "message",
        lua.create_function(move |_, t: String| {
            run_only(&sm, "message")?;
            sm.with_host("message()", |h| h.message(&t))
        })?,
    )?;
    let sco = s.clone();
    g.set(
        "confirm",
        lua.create_function(move |_, t: String| {
            run_only(&sco, "confirm")?;
            sco.with_host("confirm()", |h| h.confirm(&t))
        })?,
    )?;
    let sa = s.clone();
    g.set(
        "ask",
        lua.create_function(move |_, (p, d): (String, Option<String>)| {
            run_only(&sa, "ask")?;
            sa.with_host("ask()", |h| h.ask(&p, d.as_deref()))
        })?,
    )?;
    let sset = s.clone();
    g.set(
        "set",
        lua.create_function(move |_, (name, v): (String, Value)| {
            run_only(&sset, "set")?;
            let d = to_dyn(&v);
            if let Ok(mut snap) = sset.snapshot.lock() {
                snap.values.insert(name.clone(), d.clone());
            }
            sset.with_host("set()", |h| h.set(&name, d))
        })?,
    )?;
    g.set(
        "fail",
        lua.create_function(|_, m: String| -> mlua::Result<()> {
            Err(mlua::Error::external(Fail(m)))
        })?,
    )?;
    Ok(())
}

/// The globals this module installs, for the test that the registry and the bindings agree.
#[cfg(test)]
pub fn installed(lua: &Lua) -> Vec<String> {
    let mut out = Vec::new();
    for item in api::API {
        let mut parts = item.name.split('.');
        let head = parts.next().unwrap_or_default();
        let mut v: Value = lua.globals().get(head).unwrap_or(Value::Nil);
        for p in parts {
            v = match v {
                Value::Table(t) => t.get(p).unwrap_or(Value::Nil),
                _ => Value::Nil,
            };
        }
        let ok = match (item.function, &v) {
            (true, Value::Function(_)) => true,
            (false, Value::Nil) => false,
            (false, _) => true,
            _ => false,
        };
        if ok {
            out.push(item.name.to_string());
        }
    }
    out
}

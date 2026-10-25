//! What a file would do to the computer, for the trust prompt (N9): the programs it runs, the
//! scripts it sources, the addresses it fetches, and whether it runs Lua (which can start
//! programs too). Read from the text of the file only; nothing is evaluated.

use super::model::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Programs `run` and `check` steps start, as written (`{app}` stays a placeholder).
    pub programs: Vec<String>,
    /// Setup scripts `env-from` runs.
    pub scripts: Vec<String>,
    /// Addresses `http-get` fetches.
    pub urls: Vec<String>,
    /// It has Lua code (script blocks, `lua` steps, `(lua)` values).
    pub lua: bool,
}

fn shown(v: &Val) -> String {
    match v {
        Val::Text(t) => t.raw.clone(),
        Val::Lua(_) => "(worked out by Lua)".into(),
    }
}

fn push(list: &mut Vec<String>, s: String) {
    if !list.contains(&s) {
        list.push(s);
    }
}

fn steps(list: &[Step], out: &mut Summary) {
    for s in list {
        if s.if_.is_some() {
            out.lua = true;
        }
        match &s.kind {
            StepKind::Run(r) => push(&mut out.programs, shown(&r.program)),
            StepKind::Check { program, .. } | StepKind::Capture { program, .. } => {
                push(&mut out.programs, shown(program))
            }
            // An add-on step: the apps it needs, and what its own steps start.
            StepKind::Use(u) => {
                for n in &u.def.needs {
                    if let crate::addons::Need::App { name, .. } = n {
                        push(&mut out.programs, name.clone());
                    }
                }
                match &u.def.body {
                    crate::addons::Body::Does(inner) => steps(inner, out),
                    crate::addons::Body::Script(_) => out.lua = true,
                    crate::addons::Body::BuiltIn(_) => {}
                }
            }
            StepKind::EnvFrom { script, .. } => push(&mut out.scripts, shown(script)),
            StepKind::HttpGet { url, .. } => push(&mut out.urls, shown(url)),
            StepKind::Lua { .. } => out.lua = true,
            _ => {}
        }
    }
}

pub fn summary(config: &Config) -> Summary {
    let mut out = Summary {
        lua: !config.scripts.is_empty(),
        ..Default::default()
    };
    for a in &config.actions {
        steps(&a.steps, &mut out);
    }
    for c in &config.controls {
        steps(&c.on_change, &mut out);
        let lua_default = matches!(c.default, Some(DefaultVal::Val(Val::Lua(_))));
        let lua_options = matches!(
            c.kind,
            ControlKind::Choice {
                options_lua: Some(_),
                ..
            } | ControlKind::Choice { list: Some(_), .. }
        );
        if lua_default || lua_options || c.visible_if.is_some() || c.enabled_if.is_some() {
            out.lua = true;
        }
    }
    if config
        .values
        .iter()
        .any(|v| matches!(v.source, ValueSource::Val(Val::Lua(_))))
    {
        out.lua = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load_str, LoadOptions};
    use std::path::Path;

    #[test]
    fn lists_what_a_file_runs() {
        let l = load_str(
            r#"
value "app" "Game.exe"
layout {
    checkbox "tests" label="Tests"
}
action "build" {
    check "git" "diff" "--quiet"
    env-from "vcvars.bat"
    run "git" "pull"
    run "{app}" "-batchmode"
    http-get "https://example.com/api" id="api"
    run "dotnet" "test" if="values.tests"
}
"#,
            Path::new("x.launcher"),
            &LoadOptions::default(),
        );
        assert!(!l.has_errors(), "{:?}", l.diags);
        let s = summary(&l.config);
        assert_eq!(s.programs, ["git", "{app}", "dotnet"]);
        assert_eq!(s.scripts, ["vcvars.bat"]);
        assert_eq!(s.urls, ["https://example.com/api"]);
        assert!(s.lua, "an if= is Lua");
    }
}

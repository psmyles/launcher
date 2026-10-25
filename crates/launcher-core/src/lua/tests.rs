//! The M3 criteria: the sandbox holds, the budget trips, errors name file lines, load-time code
//! can't change anything, and API calls reach the effects and the host.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::config::{self, LoadOptions, Val, ValueSource};
use crate::engine::effects::MockEffects;

fn vm(effects: Arc<MockEffects>) -> Vm {
    Vm::new(Path::new("C:/work/tool.launcher"), effects).unwrap()
}

fn mock() -> Arc<MockEffects> {
    Arc::new(MockEffects::default())
}

#[test]
fn the_sandbox_has_no_side_doors() {
    let v = vm(mock());
    for code in ["io.open('x')", "os.execute('calc')", "require('x')"] {
        assert!(v.eval_str(code).is_err(), "{code} should fail");
    }
    // The standard library is frozen.
    assert!(v.eval_str("(function() string.upper = nil end)()").is_err());
    // values is read-only.
    let e = v.eval_str("(function() values.x = 1 end)()").unwrap_err();
    assert!(e.message.contains("read-only"), "{}", e.message);
    // What is there works.
    assert_eq!(
        v.eval_str("string.format('%d-%s', 3, os.name)").unwrap(),
        Dyn::str(format!(
            "3-{}",
            if cfg!(windows) { "windows" } else { "macos" }
        ))
    );
}

#[test]
fn the_budget_stops_runaway_code() {
    let v = vm(mock());
    let src = LuaSrc {
        code: "while true do end".into(),
        repr: "\"while true do end\"".into(),
        span: config::Span { offset: 1, len: 1 },
        line: 1,
        col: 1,
    };
    let start = Instant::now();
    let e = v.exec(&src, Some(Duration::from_millis(100))).unwrap_err();
    assert!(e.message.contains("budget"), "{}", e.message);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn cancel_stops_a_running_chunk() {
    let v = Arc::new(vm(mock()));
    let cancel = v.canceller();
    let src = LuaSrc {
        code: "while true do end".into(),
        repr: "\"...\"".into(),
        span: config::Span { offset: 2, len: 1 },
        line: 1,
        col: 1,
    };
    let v2 = v.clone();
    let t = std::thread::spawn(move || v2.exec(&src, None));
    std::thread::sleep(Duration::from_millis(50));
    cancel();
    let e = t.join().unwrap().unwrap_err();
    assert!(e.message.contains("cancelled"), "{}", e.message);
}

const FILE: &str = r##"launcher requires="0.1"
script #"""
    function branches()
        local out = capture("git", "branch", "--format=%(refname:short)")
        return split(out, "\n")
    end
    function broken()
        local t = nil
        return t.field
    end
    """#
value "first" (lua)"branches()[1]"
value "bad" (lua)"broken()"
value "typo" (lua)"1 +"
"##;

fn loaded() -> config::Loaded {
    config::load_str(
        FILE,
        Path::new("C:/work/tool.launcher"),
        &LoadOptions::default(),
    )
}

fn value_src(l: &config::Loaded, id: &str) -> LuaSrc {
    match &l.config.value(id).unwrap().source {
        ValueSource::Val(Val::Lua(s)) => s.clone(),
        _ => panic!("{id} is a lua value"),
    }
}

#[test]
fn scripts_define_functions_and_capture_reaches_the_effects() {
    let fx = mock();
    fx.answer_capture("git", "main\ndevelop\n", 0);
    let v = vm(fx.clone());
    let l = loaded();
    v.exec(&l.config.scripts[0], Some(LOAD_BUDGET)).unwrap();
    assert_eq!(
        v.eval(&value_src(&l, "first"), Some(LOAD_BUDGET)).unwrap(),
        Dyn::str("main")
    );
    assert_eq!(fx.calls(), ["capture git branch --format=%(refname:short)"]);
}

#[test]
fn runtime_errors_name_file_lines() {
    let v = vm(mock());
    let l = loaded();
    v.exec(&l.config.scripts[0], Some(LOAD_BUDGET)).unwrap();
    let e = v
        .eval(&value_src(&l, "bad"), Some(LOAD_BUDGET))
        .unwrap_err();
    // `return t.field` is line 9 of the file (chunk line 8).
    assert_eq!(e.line, Some(9), "{}", e.message);
    assert!(e.message.starts_with("tool.launcher:9:"), "{}", e.message);
    assert!(!e.message.contains("@"), "{}", e.message);
}

#[test]
fn syntax_errors_name_file_lines() {
    let v = vm(mock());
    let l = loaded();
    let e = v
        .eval(&value_src(&l, "typo"), Some(LOAD_BUDGET))
        .unwrap_err();
    assert_eq!(e.line, Some(14), "{}", e.message);
    assert!(e.message.starts_with("tool.launcher:14:"), "{}", e.message);
}

#[test]
fn a_failing_program_names_the_call_site() {
    let fx = mock();
    fx.answer_capture("git", "", 128);
    let v = vm(fx);
    let l = loaded();
    v.exec(&l.config.scripts[0], Some(LOAD_BUDGET)).unwrap();
    let e = v
        .eval(&value_src(&l, "first"), Some(LOAD_BUDGET))
        .unwrap_err();
    assert!(e.message.contains("exited with code 128"), "{}", e.message);
    // The capture() call is on line 4.
    assert_eq!(e.line, Some(4), "{}", e.message);
}

#[test]
fn loading_code_cannot_change_anything() {
    let fx = mock();
    let v = vm(fx.clone());
    for code in [
        "copy('a', 'b')",
        "write_file('x', 'y')",
        "delete('x')",
        "run('git', 'status')",
        "ask('name?')",
        "set('x', 1)",
    ] {
        let e = v.eval_str(code).unwrap_err();
        assert!(
            e.message.contains("can't run while the file loads"),
            "{code}: {}",
            e.message
        );
    }
    assert!(fx.calls().is_empty(), "{:?}", fx.calls());
}

#[test]
fn values_and_fields_read_the_snapshot() {
    let v = vm(mock());
    let mut snap = Snapshot::default();
    snap.values.insert("mode".into(), Dyn::str("Release"));
    snap.values.insert("tests".into(), Dyn::Bool(true));
    snap.fields.insert(
        "server".into(),
        [("ServerIP".to_string(), Dyn::str("10.0.0.1"))].into(),
    );
    v.set_snapshot(snap);
    assert_eq!(
        v.eval_str("values.mode == 'Release' and values.tests")
            .unwrap(),
        Dyn::Bool(true)
    );
    assert_eq!(
        v.eval_str("fields.server.ServerIP").unwrap(),
        Dyn::str("10.0.0.1")
    );
    assert_eq!(v.eval_str("values.nothing").unwrap(), Dyn::Nil);
    assert_eq!(v.eval_str("fields.nothing").unwrap(), Dyn::Nil);
}

#[test]
fn a_compiled_condition_reads_the_current_values() {
    // The same chunk (compiled once) after the values change: no value is baked in at load.
    let v = vm(mock());
    let mut snap = Snapshot::default();
    for mode in ["Debug", "Release", "Debug"] {
        snap.values.insert("mode".into(), Dyn::str(mode));
        v.set_snapshot(snap.clone());
        assert_eq!(v.eval_str("values.mode").unwrap(), Dyn::str(mode));
    }
}

/// A host that records what the run asked of it.
#[derive(Default, Clone)]
struct RecordingHost(Arc<Mutex<Vec<String>>>);

impl Host for RecordingHost {
    fn run(&mut self, cmd: &CommandSpec, pipe: bool) -> Result<i32, String> {
        self.0.lock().unwrap().push(format!(
            "run {}{}",
            cmd.display(),
            if pipe { " (pipe)" } else { "" }
        ));
        Ok(if cmd.program == "fails" { 2 } else { 0 })
    }
    fn step(&mut self, label: Option<&str>, ok: bool) {
        self.0.lock().unwrap().push(format!("step {label:?} {ok}"));
    }
    fn log(&mut self, text: &str) {
        self.0.lock().unwrap().push(format!("log {text}"));
    }
    fn message(&mut self, text: &str) {
        self.0.lock().unwrap().push(format!("message {text}"));
    }
    fn confirm(&mut self, _text: &str) -> bool {
        true
    }
    fn ask(&mut self, _prompt: &str, default: Option<&str>) -> Option<String> {
        default.map(str::to_string)
    }
    fn set(&mut self, name: &str, value: Dyn) {
        self.0
            .lock()
            .unwrap()
            .push(format!("set {name}={}", value.to_text()));
    }
}

#[test]
fn a_run_reaches_the_host_and_the_effects() {
    let fx = mock();
    let v = vm(fx.clone());
    let host = RecordingHost::default();
    v.set_host(Some(Box::new(host.clone())));
    let code = r#"
        step("Build", function()
            run("cargo", "build", "--release")
        end)
        set("who", ask("Name?", "ada"))
        log("hello", values.who)
        copy("a.txt", "b.txt")
        local ok = pcall(run, "fails")
        set("failed", not ok)
        local r = run("fails", {check = false})
        set("code", r.exit_code)
        run("tool", {io = "pipe"})
    "#;
    let src = LuaSrc {
        code: code.into(),
        repr: format!("{code:?}"),
        span: config::Span { offset: 9, len: 1 },
        line: 1,
        col: 1,
    };
    v.exec(&src, None).unwrap();
    assert_eq!(
        *host.0.lock().unwrap(),
        [
            "step Some(\"Build\") true",
            "run cargo build --release",
            "step None true",
            "set who=ada",
            "log hello ada",
            "run fails",
            "set failed=true",
            "run fails",
            "set code=2",
            "run tool (pipe)",
        ]
    );
    assert_eq!(fx.calls(), ["copy a.txt b.txt"]);
}

#[test]
fn fail_shows_just_its_message() {
    let v = vm(mock());
    let e = v.eval_str("fail('No build found')").unwrap_err();
    assert!(e.message.ends_with("No build found"), "{}", e.message);
    assert!(!e.message.contains("stack traceback"), "{}", e.message);
}

#[test]
fn every_registry_entry_has_a_binding() {
    let lua = Lua::new();
    let shared = Arc::new(Shared {
        snapshot: Mutex::new(Snapshot::default()),
        deadline: Mutex::new(None),
        cancel: std::sync::atomic::AtomicBool::new(false),
        effects: mock(),
        cx: Mutex::new(Context::default()),
        instant: std::sync::atomic::AtomicBool::new(false),
        host: Mutex::new(None),
    });
    bind::install(&lua, &shared, Path::new("x.launcher"), Path::new(".")).unwrap();
    let installed = bind::installed(&lua);
    let missing: Vec<&str> = api::API
        .iter()
        .map(|i| i.name)
        .filter(|n| !installed.iter().any(|m| m == n))
        .collect();
    assert!(missing.is_empty(), "no binding for: {missing:?}");
}

#[test]
fn json_round_trips() {
    let v = vm(mock());
    assert_eq!(
        v.eval_str(r#"json.decode(json.encode({a = {1, 2}, b = "x"})).a[2]"#)
            .unwrap(),
        Dyn::Num(2.0)
    );
}

#[test]
fn two_snippets_of_the_same_length_are_not_mixed_up() {
    let v = vm(mock());
    assert_eq!(v.eval_str("1 + 1").unwrap(), Dyn::Num(2.0));
    assert_eq!(v.eval_str("2 + 2").unwrap(), Dyn::Num(4.0));
}

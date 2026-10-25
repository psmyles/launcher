//! Tool files in format 2 (N19–N26): read in the app's own words, with plain problems, and run
//! with add-on steps in scopes of their own.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use launcher_core::config::catalog::Os;
use launcher_core::config::{load_str, ControlKind, Level, LoadOptions, Loaded, StepKind};
use launcher_core::dynval::Dyn;
use launcher_core::engine::effects::MockEffects;
use launcher_core::engine::runner::{self, RunEvent, RunOutcome, RunSpec, RunUi};
use launcher_core::eval::Evaluator;
use launcher_core::form::Form;
use launcher_core::platform::trust;

fn load(src: &str) -> Loaded {
    let opts = LoadOptions {
        os: Os::Windows,
        ..LoadOptions::default()
    };
    load_str(src, Path::new("C:/work/tool.launcher"), &opts)
}

fn problems(l: &Loaded) -> Vec<String> {
    l.diags
        .iter()
        .map(|d| match &d.help {
            Some(h) => format!("{}:{} {} ({h})", d.line, d.col, d.message),
            None => format!("{}:{} {}", d.line, d.col, d.message),
        })
        .collect()
}

const BUILD: &str = r#"// Switch to a branch, update it, build, and maybe test.
tool "Build {branch}" format=2 requires="0.2" os="windows"
window {
    choice "branch" label="Branch" {
        list-from "git.branches" folder="."
    }
    row {
        choice "mode" label="Mode" show-as="buttons" {
            option "Debug"
            option "Release"
        }
        tick-box "tests" label="Run the tests" starts=#true
    }
    button "build" label="Build"
}
steps "build" {
    before-starting {
        git.no-unsaved-changes "clean"
    }
    git.switch-branch "switch" branch="{branch}"
    git.pull "pull"
    dotnet.build "compile" configuration="{mode}"
    dotnet.test "test" configuration="{mode}" if="values.tests"
    core.note "done" text="Was on {switch.previous}, now at {pull.commit}"
}
"#;

#[test]
fn a_tool_reads_into_the_model() {
    let l = load(BUILD);
    assert!(!l.has_errors(), "{:#?}", problems(&l));
    let c = &l.config;
    assert_eq!(c.format, 2);
    assert!(matches!(
        &c.window.title,
        launcher_core::config::Val::Text(_)
    ));
    let ids: Vec<&str> = c.controls.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["branch", "mode", "tests", "build"]);
    assert!(matches!(
        &c.controls[0].kind,
        ControlKind::Choice { list: Some(l), .. } if l.def.id == "git.branches"
    ));
    let a = c.action("build").unwrap();
    let kinds: Vec<&str> = a.steps.iter().map(|s| s.kind.name()).collect();
    assert_eq!(kinds, ["use", "use", "use", "use", "use", "log"]);
    assert!(a.steps[0].first);
    match &a.steps[1].kind {
        StepKind::Use(u) => {
            assert_eq!(u.def.id, "git.switch-branch");
            assert_eq!(u.settings[0].0, "branch");
        }
        k => panic!("{k:?}"),
    }
}

#[test]
fn problems_are_in_the_tools_own_words() {
    let l = load(
        r#"tool "T" format=2
window {
    tick-bx "a" label="A"
    choice "b" lable="B" { option "x" }
    button "go" label="Go"
}
steps "go" {
    git.pul "p"
    git.pull "q" fodler="."
    perforce.sync "s"
    files.copy "c" from="a"
    pull "r"
}
"#,
    );
    assert_eq!(
        problems(&l),
        [
            "3:5 `tick-bx` doesn't belong in the window (did you mean `tick-box`?)",
            "4:16 `choice` doesn't have a `lable` (did you mean `label`?)",
            "8:5 There's no step called \"git.pul\" (did you mean `git.pull`?)",
            "9:18 \"Pull\" has no setting \"fodler\" (did you mean \"folder\"?)",
            "10:5 There's no step called \"perforce.sync\" (this needs the \"perforce\" add-on, which isn't on this computer: put it in the Add-ons folder, or ask whoever made the tool to include it)",
            "11:5 Fill in \"To\" for \"Copy\" (add to=\"…\")",
            "12:5 There's no step called \"pull\" (steps are written with their add-on: `git.pull`)",
        ]
    );
}

#[test]
fn a_button_needs_its_steps() {
    let l = load("tool \"T\" format=2\nwindow {\n    button \"go\" label=\"Go\"\n}\nsteps \"run\" {\n    core.wait \"w\" ms=1\n}\n");
    assert_eq!(
        problems(&l),
        ["3:5 The button \"go\" has no steps (add `steps \"go\" { }` for it)"]
    );
}

#[test]
fn a_tool_without_a_button_gets_start() {
    let l = load("tool \"T\" format=2\nsteps \"go\" {\n    core.wait \"w\" ms=1\n}\n");
    assert!(!l.has_errors(), "{:#?}", problems(&l));
    let b = l.config.control("start").expect("a Start button");
    assert_eq!(b.label.as_deref(), Some("Start"));
}

struct Collect(mpsc::Sender<(RunOutcome, Vec<String>)>, Vec<String>);

impl RunUi for Collect {
    fn event(&mut self, e: RunEvent) {
        match e {
            RunEvent::Log { text, .. } => self.1.push(text),
            RunEvent::Started { steps, .. } => {
                // Quiet steps (set, put…) aren't rows in the window.
                self.1.extend(
                    steps
                        .into_iter()
                        .filter(|s| !s.quiet)
                        .map(|s| format!("row: {}", s.label)),
                );
            }
            RunEvent::Finished { outcome, .. } => {
                let _ = self.0.send((outcome, std::mem::take(&mut self.1)));
            }
            _ => {}
        }
    }
    fn message(&mut self, _: &str, _: Level) {}
    fn confirm(&mut self, _: &str, _: &str, _: &str) -> bool {
        true
    }
    fn ask(&mut self, _: &str, d: Option<&str>, _: bool) -> Option<String> {
        d.map(str::to_string)
    }
    fn warn(&mut self, _: &str) {}
    fn set_control(&mut self, id: &str, v: Option<Dyn>) {
        self.1.push(format!(
            "set {id} = {}",
            v.map(|v| v.to_text()).unwrap_or_default()
        ));
    }
    fn set_choices(&mut self, id: &str, choices: Vec<String>) {
        self.1.push(format!("choices {id}: {}", choices.join(", ")));
    }
}

fn run(src: &str, sets: &[(&str, &str)], fx: Arc<MockEffects>) -> (RunOutcome, Vec<String>) {
    std::env::set_var(
        "LAUNCHER_DATA_DIR",
        std::env::temp_dir().join(format!("launcher-format2-{}", std::process::id())),
    );
    // One file per run: tests run at once, and a run holds its file's lock (N11).
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let opts = LoadOptions {
        os: Os::Windows,
        ..LoadOptions::default()
    };
    let l = load_str(src, Path::new(&format!("C:/work/{n}/tool.launcher")), &opts);
    assert!(!l.has_errors(), "{:#?}", problems(&l));
    let config = Arc::new(l.config);
    let mut ev = Evaluator::new((*config).clone(), fx.clone(), trust::assume()).unwrap();
    let results = ev.load(Default::default());
    let mut form = Form::new(config.clone(), results, BTreeMap::new(), BTreeMap::new());
    for (id, v) in sets {
        form.set_text(id, v).unwrap();
    }
    let (tx, rx) = mpsc::channel();
    let action = config.actions[0].id.clone();
    let h = runner::start(
        RunSpec {
            config,
            action,
            snapshot: form.snapshot().clone(),
            env: BTreeMap::new(),
            effects: fx,
            trusted: trust::assume(),
            resume: None,
            history: false,
            term_size: (80, 24),
            pty: false,
        },
        Box::new(Collect(tx, Vec::new())),
    )
    .unwrap();
    let r = rx.recv_timeout(Duration::from_secs(20)).unwrap();
    h.join();
    r
}

#[test]
fn add_on_steps_run_in_their_own_scope() {
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("git", "main\n", 0);
    // The branch list comes from git when the tool opens (the mock answers "main").
    let (outcome, log) = run(
        BUILD,
        &[("branch", "main"), ("mode", "Release")],
        fx.clone(),
    );
    assert_eq!(outcome, RunOutcome::Ok, "{:#?}", fx.calls());
    let calls = fx.calls();
    assert_eq!(
        calls,
        [
            "capture git -C . for-each-ref --sort=-committerdate --format=%(refname:lstrip=3) refs/remotes/origin",
            "capture git -C . for-each-ref --sort=-committerdate --format=%(refname:lstrip=2) refs/heads",
            "capture git -C . diff --quiet",
            "capture git -C . diff --cached --quiet",
            "capture git -C . branch --show-current",
            "start git -C . switch main Pipe",
            "start git -C . pull --ff-only Pipe",
            "capture git -C . rev-parse --short HEAD",
            "start dotnet build -c Release Pipe",
            "start dotnet test -c Release --no-build Pipe",
        ]
    );
    assert!(
        log.contains(&"row: Make sure there are no unsaved changes".to_string()),
        "{log:#?}"
    );
    assert!(log.contains(&"row: Switch to main".to_string()), "{log:#?}");
    assert!(
        log.contains(&"Was on main, now at main".to_string()),
        "{log:#?}"
    );
}

#[test]
fn a_missing_app_stops_with_a_plain_message() {
    let fx = Arc::new(MockEffects::default());
    fx.missing_apps.lock().unwrap().push("git".into());
    let (outcome, _) = run(
        "tool \"T\" format=2\nsteps \"go\" {\n    git.pull \"p\"\n}\n",
        &[],
        fx,
    );
    assert_eq!(
        outcome,
        RunOutcome::Failed(
            "This step needs git, which isn't installed. Install Git from https://git-scm.com/downloads".into()
        )
    );
}

#[test]
fn an_add_on_explains_a_known_problem() {
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("git", "", 0);
    let (outcome, _) = run(
        "tool \"T\" format=2\nsteps \"go\" {\n    git.push \"p\"\n}\n",
        &[],
        fx,
    );
    // The mock's `start` of git succeeds; a failure path is covered by `explain` below.
    assert_eq!(outcome, RunOutcome::Ok);
}

#[test]
fn an_explanation_matches_what_the_step_printed_without_saved_logs() {
    let src = r##"tool "T" format=2
steps "go" {
    acme.build "b"
}
add-on "acme" name="Acme" version="1.0" {
    step "build" says="Build it" {
        explain "disk is full" "There's no room left on the disk. Free some space, then try again."
        does {
            run "make"
        }
    }
}
"##;
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("make", "writing out.o: disk is full\n", 2);
    let (outcome, _) = run(src, &[], fx);
    let RunOutcome::Failed(m) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(m.starts_with("There's no room left on the disk."), "{m}");
}

#[test]
fn an_explanation_matches_what_a_script_step_ran() {
    // What a script's run() prints is the step's, as a does block's run is.
    let src = r##"tool "T" format=2
steps "go" {
    acme.build "b"
}
add-on "acme" name="Acme" version="1.0" {
    step "build" says="Build it" {
        explain "disk is full" "There's no room left on the disk. Free some space, then try again."
        script #"""
            if run("make", { check = false }).exit_code ~= 0 then fail("The build didn't work.") end
            return {}
            """#
    }
}
"##;
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("make", "writing out.o: disk is full\n", 2);
    let (outcome, _) = run(src, &[], fx);
    let RunOutcome::Failed(m) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(m.starts_with("There's no room left on the disk."), "{m}");
    assert!(m.ends_with("(The build didn't work.)"), "{m}");
}

#[test]
fn a_tool_can_carry_its_own_add_on() {
    let src = r##"tool "T" format=2
steps "go" {
    acme.greet "hi" who="Ada"
}
add-on "acme" name="Acme" version="1.0" {
    step "greet" says="Greet {who}" {
        setting "who" type="text" required=#true
        result "greeting"
        script #"""
            return { greeting = "Hello, " .. settings.who }
            """#
    }
}
"##;
    let fx = Arc::new(MockEffects::default());
    let l = load(src);
    assert!(!l.has_errors(), "{:#?}", problems(&l));
    assert_eq!(l.config.carried.len(), 1);
    let src2 = src.replace(
        "    acme.greet \"hi\" who=\"Ada\"\n",
        "    acme.greet \"hi\" who=\"Ada\"\n    core.note \"n\" text=\"{hi.greeting}\"\n",
    );
    let (outcome, log) = run(&src2, &[], fx);
    assert_eq!(outcome, RunOutcome::Ok);
    assert!(log.contains(&"Hello, Ada".to_string()), "{log:#?}");
    assert!(log.contains(&"row: Greet Ada".to_string()), "{log:#?}");
}

#[test]
fn delete_goes_to_the_trash_and_copy_asks_first() {
    let fx = Arc::new(MockEffects::default());
    fx.add_file("out/a.txt", "old");
    let (outcome, log) = run(
        "tool \"T\" format=2\nsteps \"go\" {\n    files.delete \"d\" path=\"tmp\"\n    files.delete \"gone\" path=\"tmp2\" to-trash=#false\n    files.copy \"c\" from=\"a.txt\" to=\"out/a.txt\"\n}\n",
        &[],
        fx.clone(),
    );
    assert_eq!(outcome, RunOutcome::Ok, "{log:#?}");
    let calls = fx.calls();
    assert!(calls.contains(&"trash tmp".to_string()), "{calls:#?}");
    assert!(calls.contains(&"delete tmp2".to_string()), "{calls:#?}");
    // The test's dialog answers Replace.
    assert!(
        calls.iter().any(|c| c.starts_with("copy a.txt")),
        "{calls:#?}"
    );
}

/// One button fills a Drop Down Button with Git's branches (through the window); another
/// switches to the one chosen.
const BRANCHES: &str = r##"tool "Branches" format=2 requires="0.2"

window {
    button "get-list" label="Get list of all branches"
    choice "select-branch" label="Select branch" {
        option "First"
    }
    button "switch" label="Switch to selected branch"
}

steps "get-list" {
    git.list-branches "list" {
        put "branches" into="select-branch"
    }
}

steps "switch" {
    git.switch-branch "switch" branch="{select-branch}"
}
"##;

#[test]
fn a_step_fills_a_drop_down_with_its_result() {
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("git", "main\ndev\n", 0);
    let (outcome, log) = run(BRANCHES, &[], fx.clone());
    assert_eq!(outcome, RunOutcome::Ok, "{log:#?}");
    assert!(
        log.contains(&"choices select-branch: main, dev".to_string()),
        "{log:#?}"
    );
    // The put isn't a row of its own (it's quiet).
    assert_eq!(
        log.iter().filter(|l| l.starts_with("row:")).count(),
        1,
        "{log:#?}"
    );
}

#[test]
fn a_put_needs_a_result_and_a_control() {
    let bad = BRANCHES.replace(r#"put "branches""#, r#"put "branch""#);
    let l = load_str(&bad, Path::new("t.launcher"), &LoadOptions::default());
    assert!(
        problems(&l)
            .iter()
            .any(|p| p.contains("\"List branches\" has no result to put there")),
        "{:#?}",
        problems(&l)
    );
    let bad = BRANCHES.replace(r#"into="select-branch""#, r#"into="nowhere""#);
    let l = load_str(&bad, Path::new("t.launcher"), &LoadOptions::default());
    assert!(
        problems(&l)
            .iter()
            .any(|p| p.contains("There's no control called \"nowhere\" to put it into")),
        "{:#?}",
        problems(&l)
    );
}

#[test]
fn a_setting_reads_only_a_control_of_its_kind() {
    let tool = r#"tool "Run" format=2 requires="0.2"
window {
    file "preset" label="Preset to use" extensions="json"
    folder "models" label="Models"
    text "note" label="Note"
    button "start" label="Start"
}
steps "start" {
    review3d.run-preset "run" preset="{preset}" models="{MODELS}" save-to="{models}" suffix="{SUFFIX}"
}
"#;
    // Each the right kind; a text setting takes a folder's path as its text.
    let good = tool
        .replace("{MODELS}", "{models}")
        .replace("{SUFFIX}", "{models}");
    let l = load_str(&good, Path::new("t.launcher"), &LoadOptions::default());
    assert!(!l.has_errors(), "{:#?}", problems(&l));
    // A folder setting reading a file, and text reading a folder: only the first is wrong.
    let bad = tool
        .replace("{MODELS}", "{preset}")
        .replace("{SUFFIX}", "{note}");
    let l = load_str(&bad, Path::new("t.launcher"), &LoadOptions::default());
    let p = problems(&l);
    assert_eq!(p.len(), 1, "{p:#?}");
    assert!(
        p[0].contains(
            "\"Models\" for \"Run an optimization preset\" needs a folder or a list of files, but \"Preset to use\" is a file"
        ),
        "{p:#?}"
    );
}

#[test]
fn a_file_setting_takes_only_files_of_its_extensions() {
    let tool = r#"tool "Run" format=2 requires="0.2"
window {
    file "preset" label="Preset to use" EXTS
    folder "models" label="Models"
    button "start" label="Start"
}
steps "start" {
    review3d.run-preset "run" preset="{preset}" models="{models}" save-to="{models}"
}
"#;
    let problems_with = |exts: &str| {
        let l = load_str(
            &tool.replace("EXTS", exts),
            Path::new("t.launcher"),
            &LoadOptions::default(),
        );
        problems(&l)
    };
    assert!(problems_with(r#"extensions="json""#).is_empty());
    assert!(problems_with(r#"extensions=".JSON""#).is_empty());
    let p = problems_with(r#"extensions="fbx""#);
    assert_eq!(p.len(), 1, "{p:#?}");
    assert!(
        p[0].contains(
            "\"Preset\" for \"Run an optimization preset\" needs a .json file, but \"Preset to use\" is for .fbx files"
        ),
        "{p:#?}"
    );
    // A file control for any file isn't enough for a setting that names its extensions.
    let p = problems_with("");
    assert!(p[0].contains("is for any file"), "{p:#?}");
}

#[test]
fn a_list_a_step_found_reaches_the_next_step_whole() {
    let src = r##"tool "T" format=2
steps "go" {
    acme.find "f"
    acme.count "c" items="{f.found}"
    core.note "n" text="{c.how-many}: {c.first}"
}
add-on "acme" name="Acme" version="1.0" {
    step "find" says="Find" {
        result "found" type="files"
        script #"""
            return { found = { "a, b.fbx", "c.fbx" } }
            """#
    }
    step "count" says="Count {items}" {
        setting "items" type="folder" several=#true required=#true
        result "how-many"
        result "first"
        script #"""
            local items = type(settings.items) == "table" and settings.items or {}
            return { ["how-many"] = #items, first = items[1] or "" }
            """#
    }
}
"##;
    let (outcome, log) = run(src, &[], Arc::new(MockEffects::default()));
    assert_eq!(outcome, RunOutcome::Ok, "{log:#?}");
    // Two paths, the first with its comma: not "a", " b.fbx" and "c.fbx".
    assert!(log.contains(&"2: a, b.fbx".to_string()), "{log:#?}");
    // Before the run, the row names what it will use.
    assert!(
        log.contains(&"row: Count the Found from Find".to_string()),
        "{log:#?}"
    );
}

#[test]
fn steps_write_lists_into_one_text_file() {
    let src = r##"tool "T" format=2
steps "go" {
    acme.check "c"
    files.write "fix" path="report.txt" heading="Models to fix" content="{c.to-fix}"
    files.write "ok" path="report.txt" heading="Models that passed" content="{c.passed}"
    files.write "skipped" path="report.txt" heading="Skipped" content="{c.skipped}"
    files.write "count" path="report.txt" content="{c.count} checked"
}
add-on "acme" name="Acme" version="1.0" {
    step "check" says="Check" {
        result "to-fix" type="files" item="model"
        result "passed" type="files"
        result "skipped" type="list" item="box" items="boxes"
        result "count" type="number"
        script #"""
            return { ["to-fix"] = { "a, b.fbx", "c.fbx" }, passed = { "d.fbx" }, skipped = {}, count = 3 }
            """#
    }
}
"##;
    let fx = Arc::new(MockEffects::default());
    let (outcome, log) = run(src, &[], fx.clone());
    assert_eq!(outcome, RunOutcome::Ok, "{log:#?}");
    assert_eq!(
        fx.file("report.txt").as_deref(),
        Some(
            "Models to fix (2 models)\na, b.fbx\nc.fbx\n\n\
             Models that passed (1 file)\nd.fbx\n\n\
             Skipped\nnone\n\
             3 checked\n"
        )
    );
    // A list can't be a file's name.
    let l = load(&src.replace(
        r#"path="report.txt" heading="Models to fix""#,
        r#"path="{c.to-fix}""#,
    ));
    let p = problems(&l);
    assert_eq!(p.len(), 1, "{p:#?}");
    assert!(
        p[0].contains("\"File\" for \"Write to a text file\" needs text"),
        "{p:#?}"
    );
}

#[test]
fn an_add_on_fails_with_just_its_message() {
    let src = r##"tool "T" format=2
steps "go" {
    acme.boom "b"
}
add-on "acme" name="Acme" version="1.0" {
    step "boom" says="Boom" {
        script #"""
            fail("3 of 9 models need fixing.")
            """#
    }
}
"##;
    let (outcome, _) = run(src, &[], Arc::new(MockEffects::default()));
    assert_eq!(
        outcome,
        RunOutcome::Failed("3 of 9 models need fixing.".into())
    );
}

#[test]
fn a_setting_uses_only_a_result_it_can_take() {
    let tool = r#"tool "Make LODs" format=2 requires="0.2"
window {
    file "rules" label="Audit preset" extensions="json"
    file "preset" label="Opt preset" extensions="json"
    folder "models" label="Models"
    folder "out" label="Save to"
    button "start" label="Start"
}
steps "start" {
    review3d.audit "audit" profile="{rules}" models="{models}"
    review3d.run-preset "optimize" preset="{preset}" models="{audit.RESULT}" save-to="{out}"
}
"#;
    let problems_with = |result: &str| {
        let l = load_str(
            &tool.replace("RESULT", result),
            Path::new("t.launcher"),
            &LoadOptions::default(),
        );
        problems(&l)
    };
    assert!(
        problems_with("to-fix").is_empty(),
        "{:#?}",
        problems_with("to-fix")
    );
    // A count isn't a folder; the problem names what fits.
    let p = problems_with("errors");
    assert_eq!(p.len(), 1, "{p:#?}");
    assert!(
        p[0].contains(
            "\"Models\" for \"Run an optimization preset\" needs a folder or a list of files, \
             but \"Errors found\" from \"Audit models\" is a number \
             (use \"Models to fix\" from \"Audit models\" instead)"
        ),
        "{p:#?}"
    );
}

#[test]
fn what_would_go_wrong_is_found_before_a_run() {
    let check = |src: &str| {
        let l = load_str(src, Path::new("t.launcher"), &LoadOptions::default());
        problems(&l)
    };
    let has = |p: &[String], want: &str| {
        assert!(p.iter().any(|x| x.contains(want)), "{want}\n{p:#?}");
    };
    // A typed value the setting can't take.
    let p = check(
        r#"tool "T" format=2
window {
    folder "models" label="Models"
}
steps "start" {
    review3d.run-preset "optimize" preset="lods.fbx" models="{models}" save-to="out"
}
"#,
    );
    has(
        &p,
        "\"Preset\" for \"Run an optimization preset\" should be a .json file",
    );
    // A result used before its step runs, or from another button's steps.
    let p = check(
        r#"tool "T" format=2
window {
    folder "models" label="Models"
    button "start" label="Start"
    button "again" label="Again"
}
steps "start" {
    review3d.run-preset "optimize" preset="p.json" models="{audit.to-fix}" save-to="out"
    review3d.audit "audit" profile="a.json" models="{models}"
}
steps "again" {
    files.show-in-folder "show" path="{audit.to-fix-count}"
}
"#,
    );
    has(&p, "uses \"to-fix\" from \"Audit models\", which runs after it (move it below \"Audit models\")");
    has(
        &p,
        "uses \"to-fix-count\" from \"Audit models\", one of another button's steps",
    );
    // A result put into a control that can't hold it; starting values a control can't hold.
    let p = check(
        r#"tool "T" format=2
window {
    folder "models" label="Models"
    number "count" label="Count" starts="lots"
    choice "mode" label="Mode" starts="Fast" {
        option "Debug"
        option "Release"
    }
    button "start" label="Start"
}
steps "start" {
    review3d.audit "audit" profile="a.json" models="{models}" {
        put "to-fix" into="count"
    }
}
"#,
    );
    has(&p, "\"Models to fix\" from \"Audit models\" is a list of files, and \"Count\" is a number: it can't go there");
    has(&p, "\"Count\" starts as \"lots\", which isn't a number");
    has(
        &p,
        "\"Mode\" starts as \"Fast\", which isn't one of its options (Debug, Release)",
    );
    // A drop-down setting with fixed options, from a drop-down with other options.
    let p = check(
        r#"tool "T" format=2
window {
    folder "models" label="Models"
    choice "when" label="When" {
        option "Errors"
        option "Always"
    }
    button "start" label="Start"
}
steps "start" {
    review3d.audit "audit" profile="a.json" models="{models}" needs-fixing="{when}"
}
"#,
    );
    has(&p, "\"When\" is a drop-down with other options (give \"When\" only these options: Errors, Warnings or errors, Anything to report");
    // A step for another OS than the tool's.
    let p = check(
        r#"tool "T" format=2 os="macos"
steps "start" {
    acme.registry "r"
}
add-on "acme" name="Acme" version="1.0" {
    step "registry" says="Read the registry" {
        needs {
            os "windows"
        }
        script "return {}"
    }
}
"#,
    );
    has(
        &p,
        "\"Registry\" only works on Windows, and this tool is for macOS",
    );
}

//! The results table (a `table` control): read from a tool, rows worked out from two real
//! results files, grouped, with per-row buttons whose actions see the chosen row's fields; and the
//! exit codes a window ends with.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use launcher_core::config::catalog::Os;
use launcher_core::config::{
    load_str, After, ChoiceStyle, ControlKind, Level, LoadOptions, Loaded, TableSpec,
};
use launcher_core::dynval::Dyn;
use launcher_core::engine::effects::MockEffects;
use launcher_core::engine::runner::{self, RunEvent, RunOutcome, RunSpec, RunUi};
use launcher_core::eval::{Evaluator, Which};
use launcher_core::form::Form;
use launcher_core::lua::Snapshot;
use launcher_core::platform::trust;
use launcher_core::state::Stored;
use launcher_core::table;

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

fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/asset-check")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The sample tool beside the results files: the shape Asset Check's tool will have.
fn tool() -> String {
    fixture("asset-check.launcher")
}

fn spec(l: &Loaded) -> &TableSpec {
    match &l.config.control("findings").expect("the table").kind {
        ControlKind::Choice {
            style: ChoiceStyle::Table(t),
            ..
        } => t,
        other => panic!("not a table: {other:?}"),
    }
}

#[test]
fn a_tool_table_reads_into_the_model() {
    let l = load(&tool());
    assert!(!l.has_errors(), "{:#?}", problems(&l));
    let t = spec(&l);
    assert_eq!(t.group_by.as_deref(), Some("path"));
    assert_eq!(t.icon_from.as_deref(), Some("severity"));
    assert!(t.open);
    assert_eq!(t.height, 360.0);
    let buttons: Vec<(&str, &str, bool, bool)> = t
        .buttons
        .iter()
        .map(|b| {
            (
                b.id.as_str(),
                b.action.as_str(),
                b.group,
                b.visible_if.is_some(),
            )
        })
        .collect();
    assert_eq!(
        buttons,
        [
            ("reveal", "reveal", true, true),
            ("open", "open", true, true),
            ("explain", "explain", false, false),
        ]
    );
    let c = l.config.control("findings").unwrap();
    assert!(!c.persist, "a table's choice is never remembered");
    // Table buttons count as buttons: no Start button, and each run is called what its button says.
    assert!(l.config.control("start").is_none());
    assert_eq!(
        l.config.action("reveal").unwrap().label.as_deref(),
        Some("Show in Explorer")
    );
    // The window's exit codes.
    assert_eq!(l.config.window.close_code, 1);
    let dont = l.config.action("dont").unwrap();
    assert_eq!((dont.after, dont.exit_code), (After::Exit, Some(1)));
    assert_eq!(l.config.action("commit").unwrap().exit_code, Some(0));
    assert_eq!(l.config.action("explain").unwrap().exit_code, None);
}

#[test]
fn a_table_button_needs_its_steps_and_known_words() {
    let l = load(
        r#"tool "T" format=2
window {
    table "t" rows=(lua)"{}" {
        group-button "gone" label="Gone"
        row-button "go" label="Go" when="always"
    }
}
steps "go" {
    core.wait "w" ms=1
}
"#,
    );
    assert_eq!(
        problems(&l),
        [
            "4:9 The button \"gone\" has no steps (add `steps \"gone\" { }` for it)",
            "5:36 `row-button` doesn't have a `when` (`row-button` takes: label, show-if)",
        ]
    );
}

#[test]
fn exit_codes_are_checked() {
    let l = load(
        r#"launcher
window close-code=300
action "a" exit-code=2 {
    log "x"
}
action "b" after="exit" exit-code=-1 {
    log "x"
}
layout {
    table "t" {
        row-button "r" action="nope"
    }
}
"#,
    );
    let p = problems(&l);
    assert!(
        p.iter()
            .any(|m| m.contains("close-code 300 isn't an exit code")),
        "{p:#?}"
    );
    assert!(
        p.iter()
            .any(|m| m.contains("`a` has an exit-code, but its run doesn't close the window")),
        "{p:#?}"
    );
    assert!(
        p.iter()
            .any(|m| m.contains("exit-code -1 isn't an exit code")),
        "{p:#?}"
    );
    assert!(
        p.iter()
            .any(|m| m.contains("`t`'s button `r` runs `nope`, but there is no such action")),
        "{p:#?}"
    );
}

/// The tool with both results files where its Lua reads them, and the form as `--set` starts it.
fn opened() -> (Arc<MockEffects>, Evaluator, Form) {
    let l = load(&tool());
    assert!(!l.has_errors(), "{:#?}", problems(&l));
    let config = Arc::new(l.config);
    let fx = Arc::new(MockEffects::default());
    fx.add_file("results-fire-cli.json", &fixture("results-fire-cli.json"));
    fx.add_file(
        "results-3d-review-cli.json",
        &fixture("results-3d-review-cli.json"),
    );
    let mut form = Form::new(
        config.clone(),
        Default::default(),
        BTreeMap::new(),
        BTreeMap::new(),
    );
    form.set_for_launch("fire", "results-fire-cli.json")
        .unwrap();
    form.set_for_launch("review", "results-3d-review-cli.json")
        .unwrap();
    let mut ev = Evaluator::new((*config).clone(), fx.clone(), trust::assume()).unwrap();
    let results = ev.load(form.snapshot().clone());
    assert!(
        results.options["findings"].is_ok(),
        "{:?}",
        results.options["findings"]
    );
    form.set_results(results);
    (fx, ev, form)
}

#[test]
fn two_results_files_make_one_table_with_the_run_level_group_first() {
    let (_, mut ev, form) = opened();
    let rows = form.options("findings");
    let seen: Vec<(String, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r.value.clone(),
                table::field(r, "path"),
                table::field(r, "rule"),
            )
        })
        .collect();
    let row = |v: &str, p: &str, r: &str| (v.to_string(), p.to_string(), r.to_string());
    assert_eq!(
        seen,
        [
            row("1", "", "tool.too_old"),
            row(
                "2",
                "Assets/Art/Props/ComfyUI_00042_.png",
                "file.no_profile"
            ),
            row(
                "3",
                "Assets/Art/Props/T_Crate_D.png",
                "texture.color_profile"
            ),
            row("4", "Assets/Art/Props/SM_Crate.fbx", "geometry.hard_edges"),
            row("5", "Assets/Art/Props/SM_Crate.fbx", "density.texel"),
        ]
    );
    let groups = table::groups(&rows, Some("path"));
    let firsts: Vec<usize> = groups.iter().map(|g| g.rows[0]).collect();
    assert_eq!(firsts, [0, 1, 2, 3]);
    assert_eq!(groups[3].rows, [3, 4]);
    // What a group's header says, and a row's details (null fields left out).
    assert!(table::field(&rows[1], "heading").ends_with("(no type matches this name)"));
    assert_eq!(
        table::field(&rows[2], "heading"),
        "Assets/Art/Props/T_Crate_D.png    Checked as: Texture · prototype"
    );
    let details = table::field(&rows[0], "details");
    assert!(
        details.starts_with("This is 3d-review-cli 0.8.0"),
        "{details}"
    );
    assert!(details.contains("\nHow: Update 3D Review"), "{details}");
    assert!(!details.contains("Note from"), "{details}");

    // The buttons' visible-if, per group and per row: no file buttons on the run-level group.
    let live = ev.live(form.snapshot());
    let t = match &form.config().control("findings").unwrap().kind {
        ControlKind::Choice {
            style: ChoiceStyle::Table(t),
            ..
        } => t.clone(),
        _ => unreachable!(),
    };
    let reveal = &t.buttons[0].node.path;
    let shown: Vec<Option<bool>> = (0..rows.len())
        .map(|i| {
            live.conditions
                .get(&(reveal.clone(), Which::Row(i)))
                .map(|r| *r.as_ref().unwrap())
        })
        .collect();
    // Worked out on each group's first row only.
    assert_eq!(
        shown,
        [Some(false), Some(true), Some(true), Some(true), None]
    );
}

#[test]
fn a_rows_lua_texts_and_conditions_read_its_fields() {
    let l = load(
        r#"tool "T" format=2
window {
    table "t" rows=(lua)"{ {path='a', n=1}, {path='a', n=2}, {path='b', n=3} }" group-by="path" row-label=(lua)"'row ' .. values.n" group-label=(lua)"'file ' .. values.path" details=(lua)"values.n > 1 and 'more' or ''" {
        row-button "go" label="Go" show-if="values.n ~= 2"
    }
}
steps "go" {
    core.wait "w" ms=1
}
"#,
    );
    assert!(!l.has_errors(), "{:#?}", problems(&l));
    let config = l.config;
    let node = config.control("t").unwrap().node.path.clone();
    let button = match &config.control("t").unwrap().kind {
        ControlKind::Choice {
            style: ChoiceStyle::Table(t),
            ..
        } => t.buttons[0].node.path.clone(),
        _ => unreachable!(),
    };
    let mut ev = Evaluator::new(config, Arc::new(MockEffects::default()), trust::assume()).unwrap();
    ev.load(Snapshot::default());
    let live = ev.live(&Snapshot::default());
    let text = |key: &str| live.texts[&(node.clone(), key.to_string())].clone();
    assert_eq!(text("option-label:2"), Ok("row 3".into()));
    assert_eq!(text("group-label:0"), Ok("file a".into()));
    assert_eq!(text("group-label:2"), Ok("file b".into()));
    assert!(!live
        .texts
        .contains_key(&(node.clone(), "group-label:1".into())));
    assert_eq!(text("details:0"), Ok("".into()));
    assert_eq!(text("details:1"), Ok("more".into()));
    let shown: Vec<bool> = (0..3)
        .map(|i| live.conditions[&(button.clone(), Which::Row(i))] == Ok(true))
        .collect();
    assert_eq!(shown, [true, false, true]);
}

#[test]
fn launch_values_and_table_choices_are_not_remembered() {
    let (_, _, mut form) = opened();
    assert!(!form.take_dirty());
    assert!(form.remembered().is_empty(), "{:?}", form.remembered());
    // A button chose row 4: its fields are the table's.
    form.set("findings", Dyn::str("4"));
    let f = &form.snapshot().fields["findings"];
    assert_eq!(f["path"], Dyn::str("Assets/Art/Props/SM_Crate.fbx"));
    assert_eq!(f["rule"], Dyn::str("geometry.hard_edges"));
    assert!(!form.take_dirty());
    assert!(!form.remembered().contains_key("findings"));
    // A launch value wins over a remembered one, which stays as it was.
    let l = load(&tool());
    let mut form = Form::new(
        Arc::new(l.config),
        Default::default(),
        [("fire".to_string(), Stored::Text("old.json".into()))].into(),
        BTreeMap::new(),
    );
    form.set_for_launch("fire", "new.json").unwrap();
    assert_eq!(form.value("fire"), Dyn::str("new.json"));
    assert_eq!(form.remembered()["fire"], Stored::Text("old.json".into()));
    assert!(form.set_for_launch("nope", "x").is_err());
}

struct Collect(mpsc::Sender<(RunOutcome, Vec<String>)>, Vec<String>);

impl RunUi for Collect {
    fn event(&mut self, e: RunEvent) {
        match e {
            RunEvent::Log { text, .. } => self.1.push(text),
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
    fn set_control(&mut self, _: &str, _: Option<Dyn>) {}
    fn set_choices(&mut self, _: &str, _: Vec<String>) {}
}

/// What pressing a table button does: the table's value becomes the row, then its action runs
/// with the form as it is then.
fn press(form: &mut Form, fx: Arc<MockEffects>, row: &str, action: &str) -> Vec<String> {
    std::env::set_var(
        "LAUNCHER_DATA_DIR",
        std::env::temp_dir().join(format!("launcher-table-{}", std::process::id())),
    );
    form.set("findings", Dyn::str(row));
    let (tx, rx) = mpsc::channel();
    let h = runner::start(
        RunSpec {
            config: form.config().clone(),
            action: action.to_string(),
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
    let (outcome, log) = rx.recv_timeout(Duration::from_secs(20)).unwrap();
    h.join();
    assert_eq!(outcome, RunOutcome::Ok, "{log:#?}");
    log
}

#[test]
fn group_and_row_buttons_see_their_rows_fields() {
    let (fx, _, mut form) = opened();
    let rows = form.options("findings");
    let groups = table::groups(&rows, Some("path"));
    // The model's group: its button chooses the group's first row.
    let first = &rows[groups[3].rows[0]].value;
    let log = press(&mut form, fx.clone(), first, "reveal");
    assert!(
        log.contains(
            &"group path=Assets/Art/Props/SM_Crate.fbx rule=geometry.hard_edges".to_string()
        ),
        "{log:#?}"
    );
    // The model's second finding's own button.
    let second = &rows[groups[3].rows[1]].value;
    let log = press(&mut form, fx, second, "explain");
    assert!(
        log.contains(
            &"row path=Assets/Art/Props/SM_Crate.fbx rule=density.texel severity=info".to_string()
        ),
        "{log:#?}"
    );
}

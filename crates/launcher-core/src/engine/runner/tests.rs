//! The runner against mock effects: order (checks first), results feeding later steps, failures
//! and on-error, Retry, Cancel, locks, dry runs, and Lua reaching the same console.

use std::path::Path;
use std::sync::mpsc;

use super::*;
use crate::engine::effects::{DryRunEffects, MockEffects};
use crate::platform::trust;

/// A UI that writes down what happened.
struct Recorder {
    lines: Arc<Mutex<Vec<String>>>,
    done: mpsc::Sender<(RunOutcome, Option<Resume>)>,
    confirm: bool,
}

impl RunUi for Recorder {
    fn event(&mut self, e: RunEvent) {
        let line = match e {
            RunEvent::Started { steps, .. } => format!(
                "started {}",
                steps
                    .iter()
                    .map(|s| s.label.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
            RunEvent::StepStarted { index } => format!("step {index}"),
            RunEvent::Output { bytes, .. } => {
                format!("output {}", String::from_utf8_lossy(&bytes).trim())
            }
            RunEvent::Log { text, .. } => format!("log {text}"),
            RunEvent::Attached { .. } => return,
            RunEvent::StepFinished { index, outcome, .. } => format!("done {index} {outcome:?}"),
            RunEvent::Finished { outcome, resume } => {
                let _ = self.done.send((outcome, resume));
                return;
            }
        };
        self.lines.lock().unwrap().push(line);
    }
    fn message(&mut self, text: &str, _level: Level) {
        self.lines.lock().unwrap().push(format!("message {text}"));
    }
    fn confirm(&mut self, text: &str, _yes: &str, _no: &str) -> bool {
        self.lines.lock().unwrap().push(format!("confirm {text}"));
        self.confirm
    }
    fn ask(&mut self, prompt: &str, default: Option<&str>, _password: bool) -> Option<String> {
        self.lines.lock().unwrap().push(format!("ask {prompt}"));
        default.map(str::to_string)
    }
    fn warn(&mut self, text: &str) {
        self.lines.lock().unwrap().push(format!("warn {text}"));
    }
    fn set_control(&mut self, id: &str, value: Option<Dyn>) {
        self.lines
            .lock()
            .unwrap()
            .push(format!("set-control {id} {:?}", value.map(|v| v.to_text())));
    }
}

fn config(src: &str, name: &str) -> Arc<Config> {
    let dir = std::env::temp_dir().join(format!("launcher-run-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(name);
    let l = load_str(src, &path, &LoadOptions::default());
    assert!(
        !l.has_errors(),
        "{:?}",
        l.diags.iter().map(|d| d.render(name)).collect::<Vec<_>>()
    );
    Arc::new(l.config)
}

struct Run {
    lines: Arc<Mutex<Vec<String>>>,
    done: mpsc::Receiver<(RunOutcome, Option<Resume>)>,
    handle: RunHandle,
}

fn start_run(
    config: Arc<Config>,
    effects: Arc<dyn Effects>,
    resume: Option<Resume>,
    confirm: bool,
) -> Result<Run, String> {
    // Runs take their locks in app data: the tests' own, never the user's.
    crate::paths::use_test_data_dir();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let action = config.actions[0].id.clone();
    let handle = start(
        RunSpec {
            config,
            action,
            snapshot: Snapshot::default(),
            env: std::env::vars().collect(),
            effects,
            trusted: trust::assume(),
            resume,
            history: false,
            term_size: (80, 24),
            pty: true,
        },
        Box::new(Recorder {
            lines: lines.clone(),
            done: tx,
            confirm,
        }),
    )?;
    Ok(Run {
        lines,
        done: rx,
        handle,
    })
}

impl Run {
    fn finish(self) -> (RunOutcome, Option<Resume>, Vec<String>) {
        let (o, r) = self
            .done
            .recv_timeout(Duration::from_secs(20))
            .expect("the run finishes");
        self.handle.join();
        let lines = self.lines.lock().unwrap().clone();
        (o, r, lines)
    }
}

#[test]
fn checks_run_first_and_results_feed_later_steps() {
    let cfg = config(
        r#"
action "build" {
    set "who" "ada"
    run "git" "switch" "{who}" id="sw"
    check "git" "diff" "--quiet"
    log "switch exited with {sw.exit_code}"
}
"#,
        "order.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("git", "", 0);
    let (outcome, _, lines) = start_run(cfg, fx.clone(), None, true).unwrap().finish();
    assert_eq!(outcome, RunOutcome::Ok);
    let calls = fx.calls();
    assert_eq!(calls[0], "capture git diff --quiet");
    assert_eq!(calls[1], "start git switch ada Auto");
    assert!(
        lines.contains(&"log switch exited with 0".to_string()),
        "{lines:?}"
    );
}

#[test]
fn a_failure_stops_the_run_and_retry_resumes_there() {
    let cfg = config(
        r#"
action "a" {
    set "n" "1"
    run "build" id="b"
    log "after {n}"
}
"#,
        "retry.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("build", "compiling\n", 2);
    let (outcome, resume, _) = start_run(cfg.clone(), fx.clone(), None, true)
        .unwrap()
        .finish();
    assert_eq!(
        outcome,
        RunOutcome::Failed("`build` exited with code 2".into())
    );
    let resume = resume.expect("a failed run can be retried");
    assert_eq!(resume.from, 1);
    assert_eq!(resume.vars.values["n"], Dyn::str("1"));

    // Fixed; Retry starts at the failed step and keeps `n`.
    let fx2 = Arc::new(MockEffects::default());
    fx2.answer_capture("build", "", 0);
    let (outcome, _, lines) = start_run(cfg, fx2.clone(), Some(resume), true)
        .unwrap()
        .finish();
    assert_eq!(outcome, RunOutcome::Ok);
    assert_eq!(fx2.calls(), ["start build Auto"]);
    assert!(lines.contains(&"log after 1".to_string()), "{lines:?}");
}

#[test]
fn on_error_warn_pauses_with_a_warning_and_goes_on() {
    let cfg = config(
        r#"
action "a" {
    run "flaky" on-error="warn" message="Flaky said no: {error}"
    run "tidy" if="false"
    log "went on"
}
"#,
        "warn.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("flaky", "", 1);
    let (outcome, _, lines) = start_run(cfg, fx, None, true).unwrap().finish();
    assert_eq!(outcome, RunOutcome::Ok);
    assert!(
        lines.contains(&"warn Flaky said no: `flaky` exited with code 1".to_string()),
        "{lines:?}"
    );
    assert!(lines.contains(&"done 1 Skipped".to_string()), "{lines:?}");
    assert!(lines.contains(&"log went on".to_string()));
}

#[test]
fn cancel_kills_the_running_program() {
    let cfg = config(
        r#"
action "a" {
    run "server"
    log "never"
}
"#,
        "cancel.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    fx.hang("server");
    let run = start_run(cfg, fx, None, true).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    run.handle.cancel();
    let (outcome, _, lines) = run.finish();
    assert_eq!(outcome, RunOutcome::Cancelled);
    assert!(!lines.iter().any(|l| l == "log never"));
}

#[test]
fn one_run_per_file_at_a_time() {
    let src = "action \"a\" {\n    run \"server\"\n}\n";
    let a = config(src, "locked-a.launcher");
    let b = config(src, "locked-b.launcher");
    let fx = Arc::new(MockEffects::default());
    fx.hang("server");
    let first = start_run(a.clone(), fx.clone(), None, true).unwrap();
    let again = start_run(a, fx.clone(), None, true);
    assert!(
        again
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("already running")),
        "the second run: {:?}",
        again.as_ref().map(|_| "started").map_err(String::as_str)
    );
    // A different file runs.
    let other = start_run(b, fx, None, true).unwrap();
    first.handle.cancel();
    other.handle.cancel();
    first.finish();
    other.finish();
}

#[test]
fn a_dry_run_lists_what_it_would_do() {
    let cfg = config(
        r#"
action "a" {
    run "git" "pull"
    write-file "out/config.json" content="{{}}"
    delete "old.txt"
    run "game.exe" "-windowed" wait=#false
}
"#,
        "dry.launcher",
    );
    let dry = Arc::new(DryRunEffects::default());
    let (outcome, _, _) = start_run(cfg, dry.clone(), None, true).unwrap().finish();
    assert_eq!(outcome, RunOutcome::Ok);
    assert_eq!(
        dry.lines(),
        [
            "run git pull",
            "write out/config.json (2 bytes)",
            "run game.exe -windowed (left running)"
        ]
    );
}

#[test]
fn steps_that_add_to_a_file_make_one_file_each_run() {
    let src = r##"
action "a" {
    lua #"set("models", {"a.fbx", "b.fbx"})"#
    lua #"set("none", {})"#
    write-file "out/report.txt" heading="To fix" content="{models}" append=#true item="model"
    write-file "out/report.txt" heading="Passed" content="c.fbx" append=#true
    write-file "out/report.txt" heading="Skipped" content="{none}" append=#true
}
"##;
    let cfg = config(src, "append.launcher");
    let fx = Arc::new(MockEffects::default());
    // A list is counted beside its heading; an empty one says so.
    let want = "To fix (2 models)\na.fbx\nb.fbx\n\nPassed\nc.fbx\n\nSkipped\nnone\n";
    for _ in 0..2 {
        let (outcome, _, lines) = start_run(cfg.clone(), fx.clone(), None, true)
            .unwrap()
            .finish();
        assert_eq!(outcome, RunOutcome::Ok, "{lines:?}");
        // The run's first write starts the file afresh: the second run doesn't add to the first.
        assert_eq!(fx.file("out/report.txt").as_deref(), Some(want));
    }
    let calls: Vec<String> = fx
        .calls()
        .into_iter()
        .filter(|c| c.contains("file"))
        .collect();
    assert_eq!(
        calls[..2],
        [
            "write_file out/report.txt \"To fix (2 models)\\na.fbx\\nb.fbx\\n\"",
            "append_file out/report.txt \"\\nPassed\\nc.fbx\\n\"",
        ]
    );

    // Kept: each run adds to what the runs before it wrote.
    let kept = config(
        &src.replace("append=#true", "append=#true keep=#true"),
        "keep.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    fx.add_file("out/report.txt", "Earlier");
    let (outcome, _, _) = start_run(kept, fx.clone(), None, true).unwrap().finish();
    assert_eq!(outcome, RunOutcome::Ok);
    assert_eq!(
        fx.file("out/report.txt").as_deref(),
        Some(&*format!("Earlier\n\n{want}"))
    );
}

#[test]
fn a_retry_adds_to_what_the_run_wrote() {
    let cfg = config(
        r#"
action "a" {
    write-file "log.txt" content="first" append=#true
    run "build"
    write-file "log.txt" content="second" append=#true
}
"#,
        "append-retry.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture("build", "", 1);
    let (_, resume, _) = start_run(cfg.clone(), fx.clone(), None, true)
        .unwrap()
        .finish();
    let resume = resume.expect("a failed run can be retried");
    let fx2 = Arc::new(MockEffects::default());
    fx2.add_file("log.txt", &fx.file("log.txt").unwrap());
    fx2.answer_capture("build", "", 0);
    let (outcome, _, _) = start_run(cfg, fx2.clone(), Some(resume), true)
        .unwrap()
        .finish();
    assert_eq!(outcome, RunOutcome::Ok);
    assert_eq!(fx2.file("log.txt").as_deref(), Some("first\nsecond\n"));
}

#[test]
fn no_to_a_confirm_ends_the_run_quietly() {
    let cfg = config(
        "action \"a\" {\n    confirm \"Deploy?\"\n    run \"deploy\"\n}\n",
        "confirm.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    let (outcome, _, _) = start_run(cfg, fx.clone(), None, false).unwrap().finish();
    assert_eq!(outcome, RunOutcome::Stopped);
    assert!(fx.calls().is_empty());
}

#[test]
fn lua_steps_share_the_run() {
    let cfg = config(
        r##"
script #"""
    function greet(who) return "hi " .. who end
    """#
action "a" {
    ask "name" "Who?" default="ada"
    lua #"""
        run("git", "status")
        set("greeting", greet(values.name))
        """#
    log "{greeting}"
}
"##,
        "lua.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    let (outcome, _, lines) = start_run(cfg, fx.clone(), None, true).unwrap().finish();
    assert_eq!(outcome, RunOutcome::Ok, "{lines:?}");
    assert!(fx.calls().contains(&"start git status Auto".to_string()));
    assert!(lines.contains(&"log hi ada".to_string()), "{lines:?}");
}

#[test]
fn http_results_and_extracts_feed_later_steps() {
    let cfg = config(
        r#"
action "a" {
    http-get "https://example.com/s" id="api" {
        extract "session" "data[0].id" required=#true
    }
    log "{api.status} {session}"
}
"#,
        "http.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    fx.add_file("https://example.com/s", r#"{"data":[{"id":"s42"}]}"#);
    let (outcome, _, lines) = start_run(cfg, fx, None, true).unwrap().finish();
    assert_eq!(outcome, RunOutcome::Ok);
    assert!(lines.contains(&"log 200 s42".to_string()), "{lines:?}");
}

#[test]
fn fail_if_output_fails_a_step_that_exits_zero() {
    let cfg = config(
        r#"
action "a" {
    run "msbuild" fail-if-output="error [A-Z]+[0-9]+"
    log "never"
}
"#,
        "failout.launcher",
    );
    let fx = Arc::new(MockEffects::default());
    fx.answer_capture(
        "msbuild",
        "Build started\n\u{1b}[31merror CS1002\u{1b}[0m: ; expected\nDone\n",
        0,
    );
    let (outcome, _, lines) = start_run(cfg, fx, None, true).unwrap().finish();
    match outcome {
        RunOutcome::Failed(m) => assert!(m.contains("error CS1002: ; expected"), "{m}"),
        other => panic!("expected a failure, got {other:?} ({lines:?})"),
    }
}

#[test]
fn escape_sequences_are_stripped_for_matching() {
    assert_eq!(strip_ansi("\u{1b}[31merror\u{1b}[0m: x\r"), "error: x");
    assert_eq!(strip_ansi("\u{1b}]0;title\u{7}done"), "done");
    let _ = Path::new("");
}

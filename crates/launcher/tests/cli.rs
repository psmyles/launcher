//! The command line (M9), run as the real binary: exit codes, output, and a closed stdout pipe
//! (the output helpers ignore write errors, LESSONS L2.4; a panic would exit 101).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn launcher() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_launcher"));
    // A private data folder: no state, trust or recent files of the user's.
    c.env("LAUNCHER_DATA_DIR", temp("data"));
    c
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("launcher-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> (i32, String, String) {
    let o = launcher().args(args).output().unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

#[test]
fn version_and_help() {
    let (code, out, _) = run(&["--version"]);
    assert_eq!(code, 0);
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");
    let (code, out, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("launcher check FILE"), "{out}");
    assert!(out.contains("launcher edit FILE"), "{out}");
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        &["--nope"][..],
        &["check"],
        &["edit"],
        &["edit", "a", "b"],
        &["a", "b"],
    ] {
        let (code, _, err) = run(args);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(err.contains("Usage"), "{args:?}: {err}");
    }
}

#[test]
fn check_passes_the_examples_and_fails_a_broken_file() {
    let files: Vec<String> = [
        "git-build/git-build-windows.launcher",
        "git-build/git-build-macos.launcher",
        "unity-build/unity-build-windows.launcher",
        "unity-build/unity-build-macos.launcher",
    ]
    .iter()
    .map(|f| examples().join(f).display().to_string())
    .collect();
    let args: Vec<&str> = std::iter::once("check")
        .chain(files.iter().map(String::as_str))
        .collect();
    let (code, out, err) = run(&args);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("4 files checked: no problems"), "{out}");

    let bad = temp("bad").join("bad.launcher");
    std::fs::write(&bad, "layout {\n    dropdwn \"x\"\n}\n").unwrap();
    let (code, out, _) = run(&["check", &bad.display().to_string()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("did you mean"), "{out}");
}

#[test]
fn a_dry_run_lists_the_commands() {
    let f = temp("dry").join("tool.launcher");
    std::fs::write(
        &f,
        "layout {\n    text \"who\" label=\"Who\" default=\"ada\"\n}\naction \"go\" {\n    run \"git\" \"log\" \"--author={who}\"\n}\n",
    )
    .unwrap();
    let (code, out, err) = run(&[
        "run",
        &f.display().to_string(),
        "--dry-run",
        "--set",
        "who=bob",
    ]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("--author=bob"), "{out}");
}

#[test]
fn a_tool_previews_in_plain_steps() {
    let dir = temp("tool");
    let f = dir.join("tool.launcher");
    std::fs::write(
        &f,
        "tool \"Copy\" format=2\nwindow {\n    text \"to\" label=\"To\" starts=\"out\"\n}\nsteps \"go\" {\n    files.copy \"c\" from=\"a.txt\" to=\"{to}/\"\n}\n",
    )
    .unwrap();
    let (code, out, err) = run(&["run", &f.display().to_string(), "--preview"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("==> Copy a.txt to out/"), "{out}");
    assert!(
        out.contains("Preview: this is what it would have done"),
        "{out}"
    );
}

#[test]
fn check_reads_add_on_files() {
    let f = temp("addon").join("acme.kdl");
    std::fs::write(
        &f,
        "add-on \"acme\" version=\"1.0\" {\n    stpe \"hi\"\n}\n",
    )
    .unwrap();
    let (code, out, _) = run(&["check", &f.display().to_string()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("did you mean `step`?"), "{out}");
}

#[test]
fn a_closed_stdout_is_not_a_crash() {
    // The reader goes away at once; the launcher keeps writing its long output into a closed
    // pipe and must still exit normally.
    let mut child = launcher()
        .arg("--help-config")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let status = child.wait().unwrap();
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    assert_eq!(status.code(), Some(0), "stderr: {err}");
    assert!(!err.contains("panicked"), "{err}");
}

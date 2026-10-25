//! `cargo xtask e2e` (M4): headless runs of small `.launcher` files that start `tools/argdump`,
//! checked against what argdump recorded:
//!
//! * exact arguments (quotes, spaces, backslashes, empty ones), in a pty and through pipes;
//! * the Windows raw command line, byte for byte;
//! * environment, working directory and stdin;
//! * a timeout (like Cancel) kills the whole tree, grandchildren included;
//! * the per-file lock blocks a second run of the same file, not of a different one;
//! * a dry run lists the commands and starts nothing;
//! * env-from carries a setup script's variables into later steps.
//!
//! (Retry resuming at the failed step is covered by the runner's unit tests.)

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::util::{self, cargo, run, step};

struct Env {
    launcher: PathBuf,
    argdump: PathBuf,
    dir: PathBuf,
}

impl Env {
    /// Write a `.launcher` file and return its path.
    fn file(&self, name: &str, body: &str) -> PathBuf {
        let p = self.dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn launcher(&self, file: &Path, extra: &[&str]) -> Command {
        let mut c = Command::new(&self.launcher);
        c.arg("run").arg(file).args(extra);
        c.env("LAUNCHER_DATA_DIR", self.dir.join("data"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    fn run(&self, file: &Path, extra: &[&str]) -> Output {
        self.launcher(file, extra).output().unwrap()
    }

    /// A raw string literal for KDL: #"..."#.
    fn kdl_path(&self, p: &Path) -> String {
        format!("#\"{}\"#", p.display())
    }

    fn json(&self, name: &str) -> Value {
        let text = std::fs::read_to_string(self.dir.join(name))
            .unwrap_or_else(|_| panic!("argdump wrote no {name}"));
        serde_json::from_str(&text).unwrap()
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: plain queries on a handle we open and close.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == 259 // STILL_ACTIVE
        }
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
}

type Case = fn(&Env) -> Result<(), String>;

fn check(cond: bool, what: impl Into<String>) -> Result<(), String> {
    if cond {
        Ok(())
    } else {
        Err(what.into())
    }
}

const TRICKY: &[&str] = &[
    "plain",
    "with space",
    r#"quote"inside"#,
    r"back\slash\",
    "",
    "tab\there",
    "ünïcödé",
];

/// KDL string literal for any text.
fn kdl_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn case_arguments(e: &Env, io: &str) -> Result<(), String> {
    let json = format!("args-{io}.json");
    let args: Vec<String> = TRICKY.iter().map(|a| kdl_str(a)).collect();
    let f = e.file(
        &format!("args-{io}.launcher"),
        &format!(
            "layout {{\n    text \"who\" label=\"Who\"\n}}\naction \"a\" {{\n    run {} {} \"{{who}}\" io=\"{io}\" cwd=\"sub\" {{\n        env \"ARGDUMP_JSON\" {}\n        env \"ARGDUMP_NO_STDIN\" \"1\"\n        env \"E2E_VAR\" \"value 1\"\n    }}\n}}\n",
            e.kdl_path(&e.argdump),
            args.join(" "),
            e.kdl_path(&e.dir.join(&json)),
        ),
    );
    std::fs::create_dir_all(e.dir.join("sub")).unwrap();
    let o = e.run(&f, &["--set", "who=Ada Lovelace"]);
    check(o.status.success(), format!("the run failed:\n{}", text(&o)))?;
    let j = e.json(&json);
    let got: Vec<&str> = j["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let mut want: Vec<&str> = TRICKY.to_vec();
    want.push("Ada Lovelace");
    check(
        got == want,
        format!("arguments: {got:?}, expected {want:?}"),
    )?;
    check(
        j["env"]["E2E_VAR"] == "value 1",
        format!("E2E_VAR = {}", j["env"]["E2E_VAR"]),
    )?;
    let cwd = PathBuf::from(j["cwd"].as_str().unwrap());
    let want_cwd = std::fs::canonicalize(e.dir.join("sub")).unwrap();
    check(
        std::fs::canonicalize(&cwd).ok() == Some(want_cwd.clone()),
        format!("cwd {} (expected {})", cwd.display(), want_cwd.display()),
    )
}

fn case_arguments_pty(e: &Env) -> Result<(), String> {
    case_arguments(e, "pty")
}

fn case_arguments_pipe(e: &Env) -> Result<(), String> {
    case_arguments(e, "pipe")
}

fn case_stdin(e: &Env) -> Result<(), String> {
    let f = e.file(
        "stdin.launcher",
        &format!(
            "action \"a\" {{\n    run {} io=\"pipe\" stdin=\"first line\\nsecond line\\n\" {{\n        env \"ARGDUMP_JSON\" {}\n    }}\n}}\n",
            e.kdl_path(&e.argdump),
            e.kdl_path(&e.dir.join("stdin.json")),
        ),
    );
    let o = e.run(&f, &[]);
    check(o.status.success(), format!("the run failed:\n{}", text(&o)))?;
    let j = e.json("stdin.json");
    check(
        j["stdin"] == "first line\nsecond line\n",
        format!("stdin {}", j["stdin"]),
    )
}

fn case_raw_command_line(e: &Env) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    let raw = r#"-Key="a b" /flag:C:\dir\ "trailing\\" x"#;
    let f = e.file(
        "raw.launcher",
        &format!(
            "launcher os=\"windows\"\naction \"a\" {{\n    run {} command-line=#\"{raw}\"# io=\"pipe\" {{\n        env \"ARGDUMP_JSON\" {}\n        env \"ARGDUMP_NO_STDIN\" \"1\"\n    }}\n}}\n",
            e.kdl_path(&e.argdump),
            e.kdl_path(&e.dir.join("raw.json")),
        ),
    );
    let o = e.run(&f, &[]);
    check(o.status.success(), format!("the run failed:\n{}", text(&o)))?;
    let j = e.json("raw.json");
    let line = j["command_line"].as_str().unwrap();
    check(
        line.ends_with(&format!(" {raw}")),
        format!("command line {line:?} doesn't end with {raw:?}"),
    )
}

fn case_tree_kill(e: &Env) -> Result<(), String> {
    let pid_file = e.dir.join("grandchild.pid");
    let _ = std::fs::remove_file(&pid_file);
    let f = e.file(
        "tree.launcher",
        &format!(
            "action \"a\" {{\n    run {} io=\"pipe\" timeout=2000 {{\n        env \"ARGDUMP_CHILD_PID\" {}\n        env \"ARGDUMP_SLEEP_MS\" \"60000\"\n        env \"ARGDUMP_NO_STDIN\" \"1\"\n    }}\n}}\n",
            e.kdl_path(&e.argdump),
            e.kdl_path(&pid_file),
        ),
    );
    let started = Instant::now();
    let o = e.run(&f, &[]);
    check(!o.status.success(), "a timed-out run should fail")?;
    check(
        started.elapsed() < Duration::from_secs(20),
        "the run took too long to stop",
    )?;
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .map_err(|_| "argdump didn't start its grandchild".to_string())?
        .trim()
        .parse()
        .unwrap();
    // Give the OS a moment to finish tearing the tree down.
    let until = Instant::now() + Duration::from_secs(3);
    while alive(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    check(
        !alive(pid),
        format!("the grandchild {pid} is still running"),
    )
}

fn case_lock(e: &Env) -> Result<(), String> {
    let body = format!(
        "action \"a\" {{\n    run {} io=\"pipe\" {{\n        env \"ARGDUMP_SLEEP_MS\" \"4000\"\n        env \"ARGDUMP_NO_STDIN\" \"1\"\n    }}\n}}\n",
        e.kdl_path(&e.argdump)
    );
    let a = e.file("lock-a.launcher", &body);
    let b = e.file("lock-b.launcher", &body.replace("4000", "10"));
    let first = e.launcher(&a, &[]).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let again = e.run(&a, &[]);
    let other = e.run(&b, &[]);
    let first = first.wait_with_output().unwrap();
    check(
        first.status.success(),
        format!("the first run failed:\n{}", text(&first)),
    )?;
    check(
        !again.status.success() && text(&again).contains("already running"),
        format!(
            "a second run of the same file wasn't refused:\n{}",
            text(&again)
        ),
    )?;
    check(
        other.status.success(),
        format!("a different file was blocked:\n{}", text(&other)),
    )
}

fn case_dry_run(e: &Env) -> Result<(), String> {
    let marker = e.dir.join("dry-should-not-exist.json");
    let _ = std::fs::remove_file(&marker);
    let f = e.file(
        "dry.launcher",
        &format!(
            "action \"a\" {{\n    run {} \"--flag\" \"a b\" {{\n        env \"ARGDUMP_JSON\" {}\n    }}\n    write-file \"out.txt\" content=\"x\"\n}}\n",
            e.kdl_path(&e.argdump),
            e.kdl_path(&marker),
        ),
    );
    let o = e.run(&f, &["--dry-run"]);
    let t = text(&o);
    check(o.status.success(), format!("the dry run failed:\n{t}"))?;
    check(
        t.contains("--flag \"a b\""),
        format!("the command isn't listed:\n{t}"),
    )?;
    check(
        t.contains("write out.txt"),
        format!("the write isn't listed:\n{t}"),
    )?;
    check(!marker.exists(), "a dry run started the program")?;
    check(!e.dir.join("out.txt").exists(), "a dry run wrote a file")
}

fn case_env_from(e: &Env) -> Result<(), String> {
    let (script, body) = if cfg!(windows) {
        (
            "setup.bat",
            "@echo off\r\nset E2E_FROM_SCRIPT=set by %1\r\n",
        )
    } else {
        ("setup.sh", "export E2E_FROM_SCRIPT=\"set by $1\"\n")
    };
    std::fs::write(e.dir.join(script), body).unwrap();
    let f = e.file(
        "envfrom.launcher",
        &format!(
            "action \"a\" {{\n    env-from \"{script}\" \"setup\"\n    run {} io=\"pipe\" {{\n        env \"ARGDUMP_JSON\" {}\n        env \"ARGDUMP_NO_STDIN\" \"1\"\n    }}\n}}\n",
            e.kdl_path(&e.argdump),
            e.kdl_path(&e.dir.join("envfrom.json")),
        ),
    );
    let o = e.run(&f, &[]);
    check(o.status.success(), format!("the run failed:\n{}", text(&o)))?;
    let j = e.json("envfrom.json");
    check(
        j["env"]["E2E_FROM_SCRIPT"] == "set by setup",
        format!("E2E_FROM_SCRIPT = {}", j["env"]["E2E_FROM_SCRIPT"]),
    )
}

fn case_exit_codes(e: &Env) -> Result<(), String> {
    let f = e.file(
        "exit.launcher",
        &format!(
            "action \"a\" {{\n    run {} io=\"pipe\" expect-exit=\"0,3\" {{\n        env \"ARGDUMP_EXIT\" \"3\"\n        env \"ARGDUMP_NO_STDIN\" \"1\"\n    }}\n    run {} io=\"pipe\" {{\n        env \"ARGDUMP_EXIT\" \"4\"\n        env \"ARGDUMP_NO_STDIN\" \"1\"\n    }}\n}}\n",
            e.kdl_path(&e.argdump),
            e.kdl_path(&e.argdump),
        ),
    );
    let o = e.run(&f, &[]);
    let t = text(&o);
    check(
        o.status.code() == Some(1) && t.contains("exited with code 4"),
        format!("expected the second step to fail with code 4:\n{t}"),
    )
}

pub fn run_e2e(_args: &[String]) -> Result<(), String> {
    step("build (debug: launcher, argdump)");
    run(cargo().args(["build", "-p", "launcher", "-p", "argdump"]))?;
    let target = util::root().join("target").join("debug");
    let exe = |n: &str| target.join(format!("{n}{}", std::env::consts::EXE_SUFFIX));
    let dir = std::env::temp_dir().join(format!("launcher-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Every example, as its author would check it.
    step("launcher check examples/**");
    let mut examples = Vec::new();
    let mut dirs = vec![util::root().join("examples")];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "launcher") {
                examples.push(p);
            }
        }
    }
    examples.sort();
    run(std::process::Command::new(exe("launcher"))
        .arg("check")
        .args(&examples))?;
    let env = Env {
        launcher: exe("launcher"),
        argdump: exe("argdump"),
        dir: dir.clone(),
    };
    let cases: &[(&str, Case)] = &[
        ("arguments in a pty", case_arguments_pty),
        ("arguments through pipes", case_arguments_pipe),
        ("stdin", case_stdin),
        ("Windows raw command line", case_raw_command_line),
        ("a timeout kills the whole tree", case_tree_kill),
        ("one run per file", case_lock),
        ("dry run", case_dry_run),
        ("env-from", case_env_from),
        ("exit codes", case_exit_codes),
    ];
    step("end-to-end");
    let mut failed = 0;
    for (name, case) in cases {
        match case(&env) {
            Ok(()) => eprintln!("  ok    {name}"),
            Err(e) => {
                failed += 1;
                eprintln!("  FAIL  {name}: {e}");
            }
        }
    }
    if failed > 0 {
        eprintln!("  (files kept in {})", dir.display());
        return Err(format!("{failed} end-to-end case(s) failed"));
    }
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!("  all end-to-end cases passed");
    Ok(())
}

//! Test stub that stands in for a real program: records exactly what it was given.
//!
//! Writes `argdump.txt` next to itself (or to `ARGDUMP_OUT`):
//!   line 1: the raw command line (Windows: `GetCommandLineW`, untouched by any argument parsing;
//!           elsewhere the arguments joined with spaces)
//!   line 2: the working directory
//!   then:   everything read from stdin (until end of input), unless ARGDUMP_NO_STDIN is set
//!
//! With `ARGDUMP_JSON=<path>` it also writes the same as JSON, plus the arguments one by one and
//! the whole environment: `{"command_line", "args", "cwd", "stdin", "env"}`.
//!
//! Behaviour: `ARGDUMP_PRINT=<text>` prints that text to stdout; `ARGDUMP_SLEEP_MS` waits that
//! long before exiting; `ARGDUMP_EXIT` sets the exit code (default 0); `ARGDUMP_CHILD_PID=<path>`
//! starts a copy of itself that sleeps 60 s and writes that grandchild's pid to the file (for the
//! test that Cancel kills a whole process tree).

use std::io::{Read, Write};

#[cfg(windows)]
fn command_line() -> String {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCommandLineW() -> *const u16;
    }
    // SAFETY: GetCommandLineW returns a NUL-terminated string owned by the process.
    unsafe {
        let p = GetCommandLineW();
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

#[cfg(not(windows))]
fn command_line() -> String {
    std::env::args().collect::<Vec<_>>().join(" ")
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn main() {
    if let Ok(text) = std::env::var("ARGDUMP_PRINT") {
        let mut out = std::io::stdout();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
    if let Ok(pid_file) = std::env::var("ARGDUMP_CHILD_PID") {
        let exe = std::env::current_exe().unwrap();
        // Left running on purpose: the test checks that killing the tree ends it.
        #[allow(clippy::zombie_processes)]
        let child = std::process::Command::new(exe)
            .env_remove("ARGDUMP_CHILD_PID")
            .env_remove("ARGDUMP_JSON")
            .env("ARGDUMP_OUT", format!("{pid_file}.grandchild.txt"))
            .env("ARGDUMP_NO_STDIN", "1")
            .env("ARGDUMP_SLEEP_MS", "60000")
            .spawn()
            .unwrap();
        let _ = std::fs::write(&pid_file, child.id().to_string());
    }
    let out = std::env::var_os("ARGDUMP_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .with_file_name("argdump.txt")
        });
    // ARGDUMP_NO_STDIN: do not read stdin (a parent that keeps the pipe open, as the legacy
    // launcher does, would otherwise hold us until it exits).
    let mut stdin = String::new();
    if std::env::var_os("ARGDUMP_NO_STDIN").is_none() {
        let _ = std::io::stdin().read_to_string(&mut stdin);
    }
    let cwd = std::env::current_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let line = command_line();
    let _ = std::fs::write(&out, format!("{line}\n{cwd}\n{stdin}"));
    if let Some(json) = std::env::var_os("ARGDUMP_JSON") {
        let args: Vec<String> = std::env::args().skip(1).map(|a| json_str(&a)).collect();
        let mut env: Vec<(String, String)> = std::env::vars().collect();
        env.sort();
        let env: Vec<String> = env
            .iter()
            .map(|(k, v)| format!("{}:{}", json_str(k), json_str(v)))
            .collect();
        let text = format!(
            "{{\"command_line\":{},\"args\":[{}],\"cwd\":{},\"stdin\":{},\"env\":{{{}}}}}",
            json_str(&line),
            args.join(","),
            json_str(&cwd),
            json_str(&stdin),
            env.join(",")
        );
        let _ = std::fs::write(json, text);
    }
    if let Some(ms) = std::env::var("ARGDUMP_SLEEP_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    let code = std::env::var("ARGDUMP_EXIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    std::process::exit(code);
}

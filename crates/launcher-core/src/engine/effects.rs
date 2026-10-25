//! Effects: the only way the engine and its Lua API touch the world (N6). Every process, file and
//! network operation goes through an [`Effects`], so a run can be real, a dry run that records what
//! it would do ([`DryRunEffects`]), or mocked in tests ([`MockEffects`]).

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use crate::platform::child::{self, Child, Spawn};
use crate::platform::env::EnvChange;
use crate::platform::pty::OutputFn;

/// A program to start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Windows: the arguments as one raw command line (instead of `args`).
    pub raw_command_line: Option<String>,
    pub cwd: Option<PathBuf>,
    /// Variables set on top of the run's environment.
    pub env: Vec<(String, String)>,
    pub stdin: Option<String>,
    pub timeout: Option<Duration>,
}

impl CommandSpec {
    pub fn new(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            ..Default::default()
        }
    }

    /// `program arg...` for logs and dry runs.
    pub fn display(&self) -> String {
        match &self.raw_command_line {
            Some(raw) => format!("{} {raw}", self.program),
            None => crate::platform::process::windows_command_line(&self.program, &self.args),
        }
    }
}

/// How a started program's output reaches the console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoMode {
    /// A pseudo-terminal, or pipes for a Windows GUI-subsystem program.
    Auto,
    Pty,
    Pipe,
    /// Started and left running.
    Detached,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaptureOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Where effects run: the file's folder and the run's environment.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub config_dir: PathBuf,
    /// The run's environment (starts as the launcher's, changed by env-from).
    pub env: BTreeMap<String, String>,
}

impl Context {
    pub fn new(config_dir: &Path) -> Context {
        Context {
            config_dir: config_dir.to_path_buf(),
            env: std::env::vars().collect(),
        }
    }

    /// A path from the file, resolved against its folder (N3).
    pub fn resolve(&self, p: &str) -> PathBuf {
        crate::paths::resolve(&self.config_dir, p)
    }

    /// The PATH to look programs up on (the run's, which env-from may have changed).
    pub fn path_var(&self) -> Option<OsString> {
        self.env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| OsString::from(v))
    }

    /// The full environment for a program: the run's, plus the step's own variables.
    pub fn child_env(&self, extra: &[(String, String)]) -> Vec<(OsString, OsString)> {
        let mut env = self.env.clone();
        EnvChange {
            set: extra.to_vec(),
            removed: vec![],
        }
        .apply(&mut env);
        env.into_iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }
}

/// A started program.
pub trait Proc: Send + Sync {
    fn pid(&self) -> u32;
    /// Keystrokes for a pty.
    fn write(&self, bytes: &[u8]);
    fn resize(&self, cols: u16, rows: u16);
    /// Kill it and everything it started.
    fn kill(&self);
    fn try_wait(&self) -> Option<i32>;
    /// Wait up to `d` for the exit code.
    fn wait_timeout(&self, d: Duration) -> Option<i32>;
    /// In a pseudo-terminal: it takes typed keys and a size.
    fn is_pty(&self) -> bool {
        false
    }
}

impl Proc for Child {
    fn pid(&self) -> u32 {
        Child::pid(self)
    }
    fn write(&self, bytes: &[u8]) {
        let _ = Child::write(self, bytes);
    }
    fn resize(&self, cols: u16, rows: u16) {
        Child::resize(self, cols, rows)
    }
    fn kill(&self) {
        Child::kill(self)
    }
    fn try_wait(&self) -> Option<i32> {
        Child::try_wait(self)
    }
    fn wait_timeout(&self, d: Duration) -> Option<i32> {
        Child::wait_timeout(self, d)
    }
    fn is_pty(&self) -> bool {
        Child::is_pty(self)
    }
}

/// Everything a run (or a load-time evaluation) can do to the world.
pub trait Effects: Send + Sync {
    /// A dry run: nothing is started, written or fetched.
    fn is_dry(&self) -> bool {
        false
    }
    /// Start a program; its output goes to `on_output` (an empty slice ends it).
    fn start(
        &self,
        cx: &Context,
        cmd: &CommandSpec,
        io: IoMode,
        size: (u16, u16),
        on_output: OutputFn,
    ) -> Result<Box<dyn Proc>, String>;
    /// Run a program with pipes, no console, and collect its output. `stop` is polled while it
    /// runs: when it says yes (Cancel), the program's whole tree is killed.
    fn capture(
        &self,
        cx: &Context,
        cmd: &CommandSpec,
        stop: &dyn Fn() -> bool,
    ) -> Result<CaptureOutput, String>;
    fn exists(&self, cx: &Context, path: &str) -> bool;
    /// Whether a program can be found (on the run's PATH, or by its path from the file's folder):
    /// what an add-on step's `needs { app "git" }` asks.
    fn has_app(&self, cx: &Context, name: &str) -> bool {
        crate::platform::process::find_program(name, &cx.config_dir, cx.path_var().as_ref())
            .is_some()
    }
    /// Paths matching a glob pattern (relative ones against the file's folder), sorted.
    fn glob(&self, cx: &Context, pattern: &str) -> Vec<String>;
    fn read_file(&self, cx: &Context, path: &str) -> Result<String, String>;
    fn write_file(&self, cx: &Context, path: &str, text: &str) -> Result<(), String>;
    /// Add to the end of a file (making it when it isn't there), on a line of its own.
    fn append_file(&self, cx: &Context, path: &str, text: &str) -> Result<(), String>;
    fn copy(&self, cx: &Context, from: &str, to: &str) -> Result<(), String>;
    fn rename(&self, cx: &Context, from: &str, to: &str) -> Result<(), String>;
    /// Delete a file or folder; Ok(false) when it wasn't there.
    fn delete(&self, cx: &Context, path: &str) -> Result<bool, String>;
    /// Move a file or folder to the Recycle Bin or Trash; Ok(false) when it wasn't there.
    fn trash(&self, cx: &Context, path: &str) -> Result<bool, String>;
    fn mkdir(&self, cx: &Context, path: &str) -> Result<(), String>;
    /// `id` names the step, for a dry run's `--mock id=...` answers.
    fn http_get(
        &self,
        id: Option<&str>,
        url: &str,
        headers: &[(String, String)],
        timeout: Duration,
    ) -> Result<HttpResponse, String>;
    fn ping(&self, host: &str, timeout_ms: u64) -> Result<crate::platform::ping::Reply, String>;
    fn open(&self, target: &str) -> Result<(), String>;
    fn reveal(&self, cx: &Context, path: &str) -> Result<(), String>;
    fn env_from(&self, cx: &Context, script: &str, args: &[String]) -> Result<EnvChange, String>;
    /// Set a program's audio volume (0–1) once it has opened its audio (Windows).
    fn set_process_volume(
        &self,
        pid: u32,
        volume: f32,
        timeout: Duration,
        alive: &dyn Fn() -> bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String>;
    fn log(&self, text: &str);
}

// ---- real effects --------------------------------------------------------------------------------

/// The real world.
#[derive(Default)]
pub struct RealEffects;

/// Paths matching `pattern` under `base`: `*` and `?` match within a name, `**` any number of
/// folders.
pub fn glob_paths(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let pattern = pattern.replace('\\', "/");
    let absolute = Path::new(&pattern).is_absolute();
    let (root, rest) = if absolute {
        let parts: Vec<&str> = pattern.split('/').collect();
        let fixed = parts.iter().take_while(|p| !p.contains(['*', '?'])).count();
        let root = parts[..fixed].join("/");
        (
            PathBuf::from(if root.is_empty() { "/" } else { &root }),
            parts[fixed..].join("/"),
        )
    } else {
        (base.to_path_buf(), pattern)
    };
    let parts: Vec<&str> = rest
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let mut out = Vec::new();
    walk(&root, &parts, &mut out);
    out.sort();
    out.dedup();
    out
}

fn walk(dir: &Path, parts: &[&str], out: &mut Vec<PathBuf>) {
    let Some((first, rest)) = parts.split_first() else {
        out.push(dir.to_path_buf());
        return;
    };
    if *first == "**" {
        walk(dir, rest, out);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), parts, out);
                }
            }
        }
        return;
    }
    if *first == ".." {
        walk(&dir.join(".."), rest, out);
        return;
    }
    if !first.contains(['*', '?']) {
        let p = dir.join(first);
        if p.exists() {
            walk(&p, rest, out);
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if wildcard(first, &name) {
            walk(&e.path(), rest, out);
        }
    }
}

/// `*` and `?` matching, case-insensitive on Windows.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let fold = |s: &str| -> Vec<char> {
        if cfg!(windows) {
            s.to_lowercase().chars().collect()
        } else {
            s.chars().collect()
        }
    };
    let (p, n) = (fold(pattern), fold(name));
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to).map(|_| ())
    }
}

impl RealEffects {
    fn spawn(
        &self,
        cx: &Context,
        cmd: &CommandSpec,
        io: IoMode,
        size: (u16, u16),
        on_output: OutputFn,
    ) -> Result<Child, String> {
        let program = crate::platform::process::find_program(
            &cmd.program,
            &cx.config_dir,
            cx.path_var().as_ref(),
        )
        .ok_or_else(|| format!("`{}` was not found", cmd.program))?;
        if cmd.raw_command_line.is_some() && !cfg!(windows) {
            return Err("a raw command line (command-line=) only works on Windows".into());
        }
        let io = match io {
            IoMode::Pty => child::Io::Pty {
                cols: size.0,
                rows: size.1,
            },
            IoMode::Pipe => child::Io::Pipes,
            IoMode::Detached => child::Io::Detached,
            IoMode::Auto => {
                if child::is_gui_program(&program) {
                    child::Io::Pipes
                } else {
                    child::Io::Pty {
                        cols: size.0,
                        rows: size.1,
                    }
                }
            }
        };
        let cwd = cmd
            .cwd
            .clone()
            .map(|c| crate::paths::resolve(&cx.config_dir, &c.display().to_string()))
            .unwrap_or_else(|| cx.config_dir.clone());
        child::spawn(
            &Spawn {
                program,
                args: cmd.args.clone(),
                raw_command_line: cmd.raw_command_line.clone(),
                cwd,
                env: cx.child_env(&cmd.env),
                stdin: cmd.stdin.clone().map(String::into_bytes),
                io,
            },
            on_output,
        )
        .map_err(|e| format!("could not start `{}`: {e}", cmd.program))
    }
}

impl Effects for RealEffects {
    fn start(
        &self,
        cx: &Context,
        cmd: &CommandSpec,
        io: IoMode,
        size: (u16, u16),
        on_output: OutputFn,
    ) -> Result<Box<dyn Proc>, String> {
        self.spawn(cx, cmd, io, size, on_output)
            .map(|c| Box::new(c) as Box<dyn Proc>)
    }

    fn capture(
        &self,
        cx: &Context,
        spec: &CommandSpec,
        stop: &dyn Fn() -> bool,
    ) -> Result<CaptureOutput, String> {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let c = self.spawn(
            cx,
            spec,
            IoMode::Pipe,
            (80, 24),
            Box::new(move |b| {
                let _ = tx.send(b.to_vec());
            }),
        )?;
        // Wait in short slices so Cancel and the timeout are noticed quickly.
        let started = std::time::Instant::now();
        let code = loop {
            if let Some(code) = c.wait_timeout(Duration::from_millis(50)) {
                break code;
            }
            if stop() {
                c.kill();
                return Err(format!("`{}` was cancelled", spec.program));
            }
            if let Some(t) = spec.timeout {
                if started.elapsed() >= t {
                    c.kill();
                    return Err(format!(
                        "`{}` didn't finish within {} ms",
                        spec.program,
                        t.as_millis()
                    ));
                }
            }
        };
        let mut bytes = Vec::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(5)) {
            if chunk.is_empty() {
                break;
            }
            bytes.extend(chunk);
        }
        Ok(CaptureOutput {
            exit_code: code,
            // Pipes merge stdout and stderr; checks and capture() read the text as one.
            stdout: String::from_utf8_lossy(&bytes).into_owned(),
            stderr: String::new(),
        })
    }

    fn exists(&self, cx: &Context, path: &str) -> bool {
        !path.is_empty() && cx.resolve(path).exists()
    }

    fn glob(&self, cx: &Context, pattern: &str) -> Vec<String> {
        glob_paths(&cx.config_dir, pattern)
            .into_iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    fn read_file(&self, cx: &Context, path: &str) -> Result<String, String> {
        let p = cx.resolve(path);
        std::fs::read_to_string(&p).map_err(|e| format!("can't read {}: {e}", p.display()))
    }

    fn write_file(&self, cx: &Context, path: &str, text: &str) -> Result<(), String> {
        let p = cx.resolve(path);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        crate::replace_file::write_bytes_replacing(&p, text)
            .map_err(|e| format!("can't write {}: {e}", p.display()))
    }

    fn append_file(&self, cx: &Context, path: &str, text: &str) -> Result<(), String> {
        use std::io::{Read, Seek, SeekFrom, Write};
        let p = cx.resolve(path);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let failed = |e: std::io::Error| format!("can't add to {}: {e}", p.display());
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&p)
            .map_err(failed)?;
        // What was there may not end its last line.
        let len = f.metadata().map_err(failed)?.len();
        let mut last = *b"\n";
        if len > 0 {
            f.seek(SeekFrom::End(-1)).map_err(failed)?;
            f.read_exact(&mut last).map_err(failed)?;
        }
        let mut bytes = Vec::with_capacity(text.len() + 1);
        if last[0] != b'\n' {
            bytes.push(b'\n');
        }
        bytes.extend_from_slice(text.as_bytes());
        f.write_all(&bytes).map_err(failed)
    }

    fn copy(&self, cx: &Context, from: &str, to: &str) -> Result<(), String> {
        let (f, mut t) = (cx.resolve(from), cx.resolve(to));
        if to.ends_with(['/', '\\']) || t.is_dir() {
            t = t.join(f.file_name().unwrap_or_default());
        }
        copy_tree(&f, &t).map_err(|e| format!("can't copy {} to {}: {e}", f.display(), t.display()))
    }

    fn rename(&self, cx: &Context, from: &str, to: &str) -> Result<(), String> {
        let (f, t) = (cx.resolve(from), cx.resolve(to));
        if let Some(dir) = t.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::rename(&f, &t)
            .map_err(|e| format!("can't move {} to {}: {e}", f.display(), t.display()))
    }

    fn delete(&self, cx: &Context, path: &str) -> Result<bool, String> {
        let p = cx.resolve(path);
        let r = if p.is_dir() {
            std::fs::remove_dir_all(&p)
        } else if p.exists() {
            std::fs::remove_file(&p)
        } else {
            return Ok(false);
        };
        r.map(|_| true)
            .map_err(|e| format!("can't delete {}: {e}", p.display()))
    }

    fn trash(&self, cx: &Context, path: &str) -> Result<bool, String> {
        crate::platform::trash::trash(&cx.resolve(path))
    }

    fn mkdir(&self, cx: &Context, path: &str) -> Result<(), String> {
        let p = cx.resolve(path);
        std::fs::create_dir_all(&p).map_err(|e| format!("can't create {}: {e}", p.display()))
    }

    fn http_get(
        &self,
        _id: Option<&str>,
        url: &str,
        headers: &[(String, String)],
        timeout: Duration,
    ) -> Result<HttpResponse, String> {
        let mut req = ureq::get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .config()
            .timeout_global(Some(timeout.max(Duration::from_millis(1))))
            .http_status_as_error(false)
            .build()
            .call();
        let mut resp = resp.map_err(|e| match e {
            ureq::Error::Timeout(_) => format!("no answer within {} ms", timeout.as_millis()),
            ureq::Error::HostNotFound => "the host name could not be resolved".into(),
            ureq::Error::ConnectionFailed => "the connection failed".into(),
            other => other.to_string(),
        })?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| format!("could not read the response: {e}"))?;
        if !(200..300).contains(&status) {
            let reason = resp.status().canonical_reason().unwrap_or("");
            return Err(format!("the server answered {status} {reason}")
                .trim()
                .to_string());
        }
        Ok(HttpResponse { status, body })
    }

    fn ping(&self, host: &str, timeout_ms: u64) -> Result<crate::platform::ping::Reply, String> {
        crate::platform::ping::ping(host, timeout_ms)
    }

    fn open(&self, target: &str) -> Result<(), String> {
        crate::platform::shell::open(target)
    }

    fn reveal(&self, cx: &Context, path: &str) -> Result<(), String> {
        crate::platform::shell::reveal(&cx.resolve(path))
    }

    fn env_from(&self, cx: &Context, script: &str, args: &[String]) -> Result<EnvChange, String> {
        crate::platform::env::env_from(
            &cx.resolve(script),
            args,
            &cx.config_dir,
            &cx.env,
            Duration::from_secs(120),
        )
    }

    fn set_process_volume(
        &self,
        pid: u32,
        volume: f32,
        timeout: Duration,
        alive: &dyn Fn() -> bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        #[cfg(windows)]
        {
            crate::platform::wasapi::set_process_volume(
                pid,
                volume,
                timeout.as_millis() as u64,
                100,
                alive,
                cancelled,
            )
            .map(|_| ())
        }
        #[cfg(not(windows))]
        {
            let _ = (pid, volume, timeout, alive, cancelled);
            Err("set-process-volume only works on Windows".into())
        }
    }

    fn log(&self, text: &str) {
        crate::log_info!("{text}");
    }
}

// ---- dry run -------------------------------------------------------------------------------------

/// An answer for a step in a dry run (`--mock <step-id>=@file | =text | =fail`).
#[derive(Clone, Debug, PartialEq)]
pub enum Mock {
    Body(String),
    Fail(String),
}

impl Mock {
    /// Parse the right-hand side of `--mock id=...`.
    pub fn parse(spec: &str, base: &Path) -> Result<Mock, String> {
        if spec.eq_ignore_ascii_case("fail") {
            return Ok(Mock::Fail("mocked failure".into()));
        }
        if let Some(file) = spec.strip_prefix('@') {
            let p = crate::paths::resolve(base, file);
            return std::fs::read_to_string(&p)
                .map(Mock::Body)
                .map_err(|e| format!("--mock: can't read {}: {e}", p.display()));
        }
        Ok(Mock::Body(spec.to_string()))
    }
}

/// Reads for real; every change is recorded instead of made.
#[derive(Default)]
pub struct DryRunEffects {
    pub mocks: HashMap<String, Mock>,
    /// What the run would have done, in order.
    pub would: Mutex<Vec<String>>,
}

/// A program a dry run pretends to start: it exits 0 at once.
struct FakeProc;

impl Proc for FakeProc {
    fn pid(&self) -> u32 {
        0
    }
    fn write(&self, _bytes: &[u8]) {}
    fn resize(&self, _cols: u16, _rows: u16) {}
    fn kill(&self) {}
    fn try_wait(&self) -> Option<i32> {
        Some(0)
    }
    fn wait_timeout(&self, _d: Duration) -> Option<i32> {
        Some(0)
    }
}

impl DryRunEffects {
    fn note(&self, line: String) {
        if let Ok(mut w) = self.would.lock() {
            w.push(line);
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.would.lock().map(|w| w.clone()).unwrap_or_default()
    }
}

impl Effects for DryRunEffects {
    fn is_dry(&self) -> bool {
        true
    }

    fn start(
        &self,
        _cx: &Context,
        cmd: &CommandSpec,
        io: IoMode,
        _size: (u16, u16),
        mut on_output: OutputFn,
    ) -> Result<Box<dyn Proc>, String> {
        let how = if io == IoMode::Detached {
            " (left running)"
        } else {
            ""
        };
        self.note(format!("run {}{how}", cmd.display()));
        on_output(&[]);
        Ok(Box::new(FakeProc))
    }

    fn capture(
        &self,
        cx: &Context,
        cmd: &CommandSpec,
        _stop: &dyn Fn() -> bool,
    ) -> Result<CaptureOutput, String> {
        let _ = cx;
        self.note(format!("run {} (quietly)", cmd.display()));
        Ok(CaptureOutput::default())
    }

    fn exists(&self, cx: &Context, path: &str) -> bool {
        RealEffects.exists(cx, path)
    }

    fn glob(&self, cx: &Context, pattern: &str) -> Vec<String> {
        RealEffects.glob(cx, pattern)
    }

    fn read_file(&self, cx: &Context, path: &str) -> Result<String, String> {
        RealEffects.read_file(cx, path)
    }

    fn write_file(&self, _cx: &Context, path: &str, text: &str) -> Result<(), String> {
        self.note(format!("write {path} ({} bytes)", text.len()));
        Ok(())
    }

    fn append_file(&self, _cx: &Context, path: &str, text: &str) -> Result<(), String> {
        self.note(format!("add to {path} ({} bytes)", text.len()));
        Ok(())
    }

    fn copy(&self, _cx: &Context, from: &str, to: &str) -> Result<(), String> {
        self.note(format!("copy {from} to {to}"));
        Ok(())
    }

    fn rename(&self, _cx: &Context, from: &str, to: &str) -> Result<(), String> {
        self.note(format!("move {from} to {to}"));
        Ok(())
    }

    fn delete(&self, cx: &Context, path: &str) -> Result<bool, String> {
        let there = RealEffects.exists(cx, path);
        if there {
            self.note(format!("delete {path}"));
        }
        Ok(there)
    }

    fn trash(&self, cx: &Context, path: &str) -> Result<bool, String> {
        let there = RealEffects.exists(cx, path);
        if there {
            self.note(format!("move {path} to the Recycle Bin or Trash"));
        }
        Ok(there)
    }

    fn mkdir(&self, _cx: &Context, path: &str) -> Result<(), String> {
        self.note(format!("create folder {path}"));
        Ok(())
    }

    fn http_get(
        &self,
        id: Option<&str>,
        url: &str,
        _headers: &[(String, String)],
        _timeout: Duration,
    ) -> Result<HttpResponse, String> {
        self.note(format!("fetch {url}"));
        match id.and_then(|i| self.mocks.get(i)) {
            Some(Mock::Body(b)) => Ok(HttpResponse {
                status: 200,
                body: b.clone(),
            }),
            Some(Mock::Fail(e)) => Err(e.clone()),
            None => Err(match id {
                Some(i) => {
                    format!("not fetched in a dry run (answer it with --answer {i}=@file.json)")
                }
                None => "not fetched in a dry run (give the step an id= to --answer it)".into(),
            }),
        }
    }

    fn ping(&self, host: &str, _timeout_ms: u64) -> Result<crate::platform::ping::Reply, String> {
        self.note(format!("ping {host}"));
        Ok(crate::platform::ping::Reply {
            address: host.to_string(),
            ms: 0,
        })
    }

    fn open(&self, target: &str) -> Result<(), String> {
        self.note(format!("open {target}"));
        Ok(())
    }

    fn reveal(&self, _cx: &Context, path: &str) -> Result<(), String> {
        self.note(format!("reveal {path}"));
        Ok(())
    }

    fn env_from(&self, _cx: &Context, script: &str, args: &[String]) -> Result<EnvChange, String> {
        self.note(format!(
            "take the environment from {script} {}",
            args.join(" ")
        ));
        Ok(EnvChange::default())
    }

    fn set_process_volume(
        &self,
        pid: u32,
        volume: f32,
        _timeout: Duration,
        _alive: &dyn Fn() -> bool,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        self.note(format!(
            "set the volume of process {pid} to {:.0}%",
            volume * 100.0
        ));
        Ok(())
    }

    fn log(&self, text: &str) {
        crate::log_info!("{text}");
    }
}

// ---- mock effects --------------------------------------------------------------------------------

/// Mock paths compare with either separator.
fn norm(p: &str) -> String {
    p.replace('\\', "/")
}

/// Records every call and answers from scripted responses: for tests.
#[derive(Default)]
pub struct MockEffects {
    pub calls: Mutex<Vec<String>>,
    /// `capture` and `start` answer by program name (the first matching entry).
    pub captures: Mutex<Vec<(String, CaptureOutput)>>,
    /// Paths that exist, and their text.
    pub files: Mutex<BTreeMap<String, String>>,
    /// Programs `start` keeps running until killed.
    pub hangs: Mutex<Vec<String>>,
    /// Programs `has_app` says aren't installed (every other one is).
    pub missing_apps: Mutex<Vec<String>>,
}

/// A mocked program: exits with its scripted code at once, or runs until killed.
struct MockProc {
    code: i32,
    hang: bool,
    killed: std::sync::atomic::AtomicBool,
}

impl Proc for MockProc {
    fn pid(&self) -> u32 {
        4242
    }
    fn write(&self, _bytes: &[u8]) {}
    fn resize(&self, _cols: u16, _rows: u16) {}
    fn kill(&self) {
        self.killed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn try_wait(&self) -> Option<i32> {
        if !self.hang {
            Some(self.code)
        } else if self.killed.load(std::sync::atomic::Ordering::SeqCst) {
            Some(1)
        } else {
            None
        }
    }
    fn wait_timeout(&self, d: Duration) -> Option<i32> {
        if self.try_wait().is_none() {
            std::thread::sleep(d.min(Duration::from_millis(5)));
        }
        self.try_wait()
    }
}

impl MockEffects {
    pub fn record(&self, call: String) {
        if let Ok(mut c) = self.calls.lock() {
            c.push(call);
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn answer_capture(&self, program: &str, stdout: &str, exit_code: i32) {
        if let Ok(mut c) = self.captures.lock() {
            c.push((
                program.to_string(),
                CaptureOutput {
                    exit_code,
                    stdout: stdout.to_string(),
                    stderr: String::new(),
                },
            ));
        }
    }

    pub fn hang(&self, program: &str) {
        if let Ok(mut h) = self.hangs.lock() {
            h.push(program.to_string());
        }
    }

    pub fn add_file(&self, path: &str, text: &str) {
        if let Ok(mut f) = self.files.lock() {
            f.insert(norm(path), text.to_string());
        }
    }

    /// What a file holds now (one given with `add_file`, or one a run wrote).
    pub fn file(&self, path: &str) -> Option<String> {
        self.files
            .lock()
            .ok()
            .and_then(|f| f.get(&norm(path)).cloned())
    }

    fn answer(&self, program: &str) -> Option<CaptureOutput> {
        self.captures
            .lock()
            .ok()
            .and_then(|c| c.iter().find(|(p, _)| p == program).map(|(_, o)| o.clone()))
    }
}

impl Effects for MockEffects {
    fn start(
        &self,
        _cx: &Context,
        cmd: &CommandSpec,
        io: IoMode,
        _size: (u16, u16),
        mut on_output: OutputFn,
    ) -> Result<Box<dyn Proc>, String> {
        self.record(format!("start {} {io:?}", cmd.display()));
        let answer = self.answer(&cmd.program).unwrap_or_default();
        if !answer.stdout.is_empty() {
            on_output(answer.stdout.as_bytes());
        }
        let hang = self.hangs.lock().is_ok_and(|h| h.contains(&cmd.program));
        if !hang {
            on_output(&[]);
        }
        Ok(Box::new(MockProc {
            code: answer.exit_code,
            hang,
            killed: Default::default(),
        }))
    }

    fn capture(
        &self,
        _cx: &Context,
        cmd: &CommandSpec,
        _stop: &dyn Fn() -> bool,
    ) -> Result<CaptureOutput, String> {
        self.record(format!("capture {}", cmd.display()));
        self.answer(&cmd.program)
            .ok_or_else(|| format!("`{}` was not found", cmd.program))
    }

    fn has_app(&self, _cx: &Context, name: &str) -> bool {
        !self
            .missing_apps
            .lock()
            .map(|m| m.iter().any(|x| x == name))
            .unwrap_or(false)
    }

    fn exists(&self, _cx: &Context, path: &str) -> bool {
        self.record(format!("exists {path}"));
        self.files.lock().is_ok_and(|f| f.contains_key(&norm(path)))
    }

    fn glob(&self, _cx: &Context, pattern: &str) -> Vec<String> {
        self.record(format!("glob {pattern}"));
        self.files
            .lock()
            .map(|f| {
                f.keys()
                    .filter(|k| wildcard(&norm(pattern), k))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn read_file(&self, _cx: &Context, path: &str) -> Result<String, String> {
        self.record(format!("read_file {path}"));
        self.files
            .lock()
            .ok()
            .and_then(|f| f.get(&norm(path)).cloned())
            .ok_or_else(|| format!("can't read {path}"))
    }

    fn write_file(&self, _cx: &Context, path: &str, text: &str) -> Result<(), String> {
        self.record(format!("write_file {path} {text:?}"));
        self.add_file(path, text);
        Ok(())
    }

    fn append_file(&self, _cx: &Context, path: &str, text: &str) -> Result<(), String> {
        self.record(format!("append_file {path} {text:?}"));
        let mut was = self.file(path).unwrap_or_default();
        if !was.is_empty() && !was.ends_with('\n') {
            was.push('\n');
        }
        self.add_file(path, &format!("{was}{text}"));
        Ok(())
    }

    fn copy(&self, _cx: &Context, from: &str, to: &str) -> Result<(), String> {
        self.record(format!("copy {from} {to}"));
        Ok(())
    }

    fn rename(&self, _cx: &Context, from: &str, to: &str) -> Result<(), String> {
        self.record(format!("move {from} {to}"));
        Ok(())
    }

    fn delete(&self, _cx: &Context, path: &str) -> Result<bool, String> {
        self.record(format!("delete {path}"));
        Ok(true)
    }

    fn trash(&self, _cx: &Context, path: &str) -> Result<bool, String> {
        self.record(format!("trash {path}"));
        Ok(true)
    }

    fn mkdir(&self, _cx: &Context, path: &str) -> Result<(), String> {
        self.record(format!("mkdir {path}"));
        Ok(())
    }

    fn http_get(
        &self,
        _id: Option<&str>,
        url: &str,
        _headers: &[(String, String)],
        _timeout: Duration,
    ) -> Result<HttpResponse, String> {
        self.record(format!("http_get {url}"));
        let body = self
            .files
            .lock()
            .ok()
            .and_then(|f| f.get(url).cloned())
            .unwrap_or_default();
        Ok(HttpResponse { status: 200, body })
    }

    fn ping(&self, host: &str, _timeout_ms: u64) -> Result<crate::platform::ping::Reply, String> {
        self.record(format!("ping {host}"));
        Ok(crate::platform::ping::Reply {
            address: host.to_string(),
            ms: 1,
        })
    }

    fn open(&self, target: &str) -> Result<(), String> {
        self.record(format!("open {target}"));
        Ok(())
    }

    fn reveal(&self, _cx: &Context, path: &str) -> Result<(), String> {
        self.record(format!("reveal {path}"));
        Ok(())
    }

    fn env_from(&self, _cx: &Context, script: &str, _args: &[String]) -> Result<EnvChange, String> {
        self.record(format!("env_from {script}"));
        Ok(EnvChange {
            set: vec![("FROM_SCRIPT".into(), "1".into())],
            removed: vec![],
        })
    }

    fn set_process_volume(
        &self,
        pid: u32,
        volume: f32,
        _timeout: Duration,
        _alive: &dyn Fn() -> bool,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        self.record(format!("volume {pid} {volume}"));
        Ok(())
    }

    fn log(&self, text: &str) {
        self.record(format!("log {text}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_names() {
        assert!(wildcard("*.exe", "Game.exe"));
        assert!(wildcard("g?me*", "game-1.2"));
        assert!(!wildcard("*.exe", "Game.exe.config"));
        assert_eq!(wildcard("*.EXE", "game.exe"), cfg!(windows));
    }

    #[test]
    fn globs_walk_folders() {
        let root = std::env::temp_dir().join(format!("launcher-glob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        for f in ["x.log", "a/y.log", "a/b/z.log", "a/b/w.txt"] {
            std::fs::write(root.join(f), "").unwrap();
        }
        let names = |pattern: &str| -> Vec<String> {
            glob_paths(&root, pattern)
                .into_iter()
                .map(|p| {
                    p.strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect()
        };
        assert_eq!(names("*.log"), ["x.log"]);
        assert_eq!(names("**/*.log"), ["a/b/z.log", "a/y.log", "x.log"]);
        assert_eq!(names("a/*/*.txt"), ["a/b/w.txt"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn captures_a_real_program() {
        let cx = Context::new(&std::env::temp_dir());
        let spec = if cfg!(windows) {
            CommandSpec::new("cmd", &["/c", "echo hello"])
        } else {
            CommandSpec::new("sh", &["-c", "echo hello"])
        };
        let out = RealEffects.capture(&cx, &spec, &|| false).unwrap();
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "hello");
    }

    #[test]
    fn cancel_and_timeout_stop_a_capture() {
        let cx = Context::new(&std::env::temp_dir());
        let mut spec = if cfg!(windows) {
            CommandSpec::new("ping", &["-n", "30", "127.0.0.1"])
        } else {
            CommandSpec::new("sleep", &["30"])
        };
        let started = std::time::Instant::now();
        let stop_at = started + Duration::from_millis(200);
        let e = RealEffects
            .capture(&cx, &spec, &|| std::time::Instant::now() >= stop_at)
            .unwrap_err();
        assert!(e.contains("cancelled"), "{e}");
        spec.timeout = Some(Duration::from_millis(200));
        let e = RealEffects.capture(&cx, &spec, &|| false).unwrap_err();
        assert!(e.contains("didn't finish"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn adding_to_a_file_starts_on_a_line_of_its_own() {
        let root = std::env::temp_dir().join(format!("launcher-append-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cx = Context::new(&root);
        // Made, with the folder it goes in, when it isn't there.
        RealEffects
            .append_file(&cx, "out/log.txt", "one\n")
            .unwrap();
        RealEffects.append_file(&cx, "out/log.txt", "two").unwrap();
        RealEffects
            .append_file(&cx, "out/log.txt", "three\n")
            .unwrap();
        let text = std::fs::read_to_string(root.join("out/log.txt")).unwrap();
        assert_eq!(text, "one\ntwo\nthree\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dry_run_records_instead_of_doing() {
        let dry = DryRunEffects::default();
        let cx = Context::new(&std::env::temp_dir());
        dry.write_file(&cx, "out/x.json", "{}").unwrap();
        dry.copy(&cx, "a", "b").unwrap();
        let p = dry
            .start(
                &cx,
                &CommandSpec::new("git", &["pull"]),
                IoMode::Auto,
                (80, 24),
                Box::new(|_| {}),
            )
            .unwrap();
        assert_eq!(p.try_wait(), Some(0));
        assert_eq!(
            dry.lines(),
            ["write out/x.json (2 bytes)", "copy a to b", "run git pull"]
        );
        assert!(!cx.resolve("out/x.json").exists());
    }
}

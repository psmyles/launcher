//! Starting programs for `run` steps, `capture()` and checks: one entry point, three ways.
//!
//! * [`Io::Pty`]: in a pseudo-terminal (N7), for the console: colours, progress bars, typing.
//! * [`Io::Pipes`]: stdout and stderr through pipes, stdin fed then closed. For Windows
//!   GUI-subsystem programs (they don't attach to ConPTY), `io="pipe"`, and captures.
//! * [`Io::Detached`]: started and left running (`wait=#false`: a game, an editor). Not in the
//!   run's process tree, so it outlives the launcher.
//!
//! Waited-for programs are killed as a tree: on Windows they start suspended in a kill-on-close
//! Job Object; on macOS they lead their own process group. A Windows raw command line is passed to
//! `CreateProcessW` untouched (N5, LESSONS L2.7). Batch files run through `cmd.exe /d /s /c`, and
//! their arguments may not contain the characters `cmd` would act on, so user input can't inject
//! commands.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::pty::{OutputFn, Pty, PtyCommand};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Io {
    Pty { cols: u16, rows: u16 },
    Pipes,
    Detached,
}

#[derive(Clone, Debug)]
pub struct Spawn {
    /// The resolved program (`process::find_program`).
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Windows: the arguments as one raw command line.
    pub raw_command_line: Option<String>,
    pub cwd: PathBuf,
    /// The complete environment.
    pub env: Vec<(OsString, OsString)>,
    /// Fed to the program's input (then closed, except in a pty, where it is typed).
    pub stdin: Option<Vec<u8>>,
    pub io: Io,
}

/// Characters `cmd.exe` acts on inside a batch file's arguments.
const BATCH_UNSAFE: &[char] = &['"', '%', '^', '&', '|', '<', '>', '!', '\r', '\n'];

/// How the process is really started: batch files become `cmd.exe /d /s /c "..."`.
struct Launch {
    program: PathBuf,
    command_line: String,
}

fn launch(spec: &Spawn) -> io::Result<Launch> {
    let prog = spec.program.display().to_string();
    if cfg!(windows) && super::process::is_batch(&spec.program) {
        let args = match &spec.raw_command_line {
            Some(raw) => raw.clone(),
            None => {
                if let Some(bad) = spec.args.iter().find(|a| a.contains(BATCH_UNSAFE)) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "the argument {bad:?} can't be passed to a batch file: it holds a character cmd.exe would act on (\" % ^ & | < > !)"
                        ),
                    ));
                }
                let mut s = String::new();
                for a in &spec.args {
                    s.push(' ');
                    super::process::quote_windows_arg(a, &mut s);
                }
                s
            }
        };
        let comspec = std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
        let mut quoted = String::new();
        super::process::quote_windows_arg(&prog, &mut quoted);
        return Ok(Launch {
            command_line: format!("cmd.exe /d /s /c \"{quoted}{}\"", args),
            program: comspec,
        });
    }
    let command_line = match &spec.raw_command_line {
        Some(raw) => {
            let mut s = String::new();
            super::process::quote_windows_arg(&prog, &mut s);
            if !raw.is_empty() {
                s.push(' ');
                s.push_str(raw);
            }
            s
        }
        None => super::process::windows_command_line(&prog, &spec.args),
    };
    Ok(Launch {
        program: spec.program.clone(),
        command_line,
    })
}

enum Inner {
    Pty(Pty),
    Other(imp::Proc),
}

/// A started program.
pub struct Child {
    inner: Inner,
    pid: u32,
}

impl Child {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Input for a pty (typed keys).
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        match &self.inner {
            Inner::Pty(p) => p.write(bytes),
            Inner::Other(_) => Ok(()),
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Inner::Pty(p) = &self.inner {
            p.resize(cols, rows);
        }
    }

    /// Kill the program and everything it started (not a detached one).
    pub fn kill(&self) {
        match &self.inner {
            Inner::Pty(p) => p.kill(),
            Inner::Other(p) => p.kill(),
        }
    }

    pub fn try_wait(&self) -> Option<i32> {
        match &self.inner {
            Inner::Pty(p) => p.try_wait(),
            Inner::Other(p) => p.try_wait(),
        }
    }

    /// Wait up to `d` for the exit code.
    pub fn wait_timeout(&self, d: Duration) -> Option<i32> {
        let until = Instant::now() + d;
        loop {
            if let Some(c) = self.try_wait() {
                return Some(c);
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5).min(until - Instant::now()));
        }
    }

    /// Started in a pseudo-terminal (it takes typed keys and a size).
    pub fn is_pty(&self) -> bool {
        matches!(self.inner, Inner::Pty(_))
    }

    pub fn is_alive(&self) -> bool {
        self.try_wait().is_none()
    }
}

/// The environment for a program in a pty: it says what the terminal understands, unless the run
/// set that itself. Without `TERM`, tools on Windows (cargo among them) print no colour in a
/// ConPTY.
fn terminal_env(env: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
    let mut env = env.to_vec();
    for (k, v) in [("TERM", "xterm-256color"), ("COLORTERM", "truecolor")] {
        if !env.iter().any(|(name, _)| name.eq_ignore_ascii_case(k)) {
            env.push((k.into(), v.into()));
        }
    }
    env
}

/// Start `spec`. Output (pty or pipes) goes to `on_output` from reader threads; an empty slice
/// marks the end, after the exit code is known. A detached program reports no output.
pub fn spawn(spec: &Spawn, on_output: OutputFn) -> io::Result<Child> {
    let l = launch(spec)?;
    match spec.io {
        Io::Pty { cols, rows } => {
            let pty = Pty::spawn(
                &PtyCommand {
                    program: l.program,
                    args: spec.args.clone(),
                    raw_command_line: cfg!(windows).then_some(l.command_line),
                    cwd: Some(spec.cwd.clone()),
                    env: Some(terminal_env(&spec.env)),
                    cols,
                    rows,
                },
                on_output,
            )?;
            if let Some(input) = &spec.stdin {
                pty.write(input)?;
            }
            let pid = pty.pid();
            Ok(Child {
                inner: Inner::Pty(pty),
                pid,
            })
        }
        Io::Pipes | Io::Detached => {
            let p = imp::spawn(spec, &l, on_output)?;
            let pid = p.pid;
            Ok(Child {
                inner: Inner::Other(p),
                pid,
            })
        }
    }
}

/// The exit code once known, shared with the waiter thread.
#[derive(Default)]
struct Exit {
    code: std::sync::Mutex<Option<i32>>,
}

impl Exit {
    fn set(&self, c: i32) {
        if let Ok(mut code) = self.code.lock() {
            *code = Some(c);
        }
    }

    fn get(&self) -> Option<i32> {
        self.code.lock().ok().and_then(|c| *c)
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::Arc;

    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{
        CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
    };
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Pipes::CreatePipe;
    use windows::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
        InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
        UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NEW_CONSOLE, CREATE_NO_WINDOW,
        CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
        LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    use super::{Exit, Io, Launch, OutputFn, Spawn};
    use crate::platform::process;

    struct Owned(HANDLE);
    // SAFETY: kernel handles may be used from any thread.
    unsafe impl Send for Owned {}
    unsafe impl Sync for Owned {}
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_invalid() && !self.0 .0.is_null() {
                // SAFETY: we own the handle and close it once.
                let _ = unsafe { CloseHandle(self.0) };
            }
        }
    }

    pub struct Proc {
        pub pid: u32,
        process: Arc<Owned>,
        job: Option<Owned>,
        exit: Arc<Exit>,
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    /// A pipe whose `child` end is inheritable and whose `ours` end isn't.
    fn pipe(child_reads: bool) -> io::Result<(Owned, Owned)> {
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: true.into(),
        };
        let (mut r, mut w) = (HANDLE::default(), HANDLE::default());
        // SAFETY: out-params are locals; the attributes outlive the call.
        unsafe { CreatePipe(&mut r, &mut w, Some(&sa), 0)? };
        let (child, ours) = if child_reads {
            (Owned(r), Owned(w))
        } else {
            (Owned(w), Owned(r))
        };
        // SAFETY: a live handle we own.
        unsafe { SetHandleInformation(ours.0, HANDLE_FLAG_INHERIT.0, Default::default())? };
        Ok((child, ours))
    }

    pub fn spawn(spec: &Spawn, l: &Launch, mut on_output: OutputFn) -> io::Result<Proc> {
        let detached = spec.io == Io::Detached;
        let exit = Arc::new(Exit::default());
        // SAFETY: the Win32 sequence below owns every handle it creates; the attribute list and
        // the inherited handle array outlive CreateProcessW.
        unsafe {
            let mut si = STARTUPINFOEXW::default();
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            let mut child_ends: Vec<Owned> = Vec::new();
            let mut ours: Vec<Owned> = Vec::new();
            let mut attr_buf: Vec<u8> = Vec::new();
            let mut inherit: Vec<HANDLE> = Vec::new();
            let mut flags = CREATE_UNICODE_ENVIRONMENT;
            if !detached {
                let (in_child, in_ours) = pipe(true)?;
                let (out_child, out_ours) = pipe(false)?;
                let (err_child, err_ours) = pipe(false)?;
                si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
                si.StartupInfo.hStdInput = in_child.0;
                si.StartupInfo.hStdOutput = out_child.0;
                si.StartupInfo.hStdError = err_child.0;
                inherit = vec![in_child.0, out_child.0, err_child.0];
                child_ends = vec![in_child, out_child, err_child];
                ours = vec![in_ours, out_ours, err_ours];
                // Only these three handles are inherited, not whatever else is inheritable.
                let mut size = 0usize;
                let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
                attr_buf = vec![0u8; size];
                let attrs = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr() as *mut c_void);
                InitializeProcThreadAttributeList(Some(attrs), 1, None, &mut size)?;
                UpdateProcThreadAttribute(
                    attrs,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    Some(inherit.as_ptr() as *const c_void),
                    std::mem::size_of_val(inherit.as_slice()),
                    None,
                    None,
                )?;
                si.lpAttributeList = attrs;
                flags |= EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW;
            } else {
                // Left running on its own, as a double-click would start it: a console program
                // gets a console of its own rather than the one the launcher may have borrowed
                // from its parent (whose input it would wait on). GUI programs ignore this.
                flags |= CREATE_NEW_CONSOLE;
            }
            let mut line: Vec<u16> = l.command_line.encode_utf16().chain(Some(0)).collect();
            let app = wide(l.program.as_os_str());
            let cwd = wide(spec.cwd.as_os_str());
            let env = process::windows_env_block(&spec.env);
            let mut pi = PROCESS_INFORMATION::default();
            let created = CreateProcessW(
                PCWSTR(app.as_ptr()),
                Some(PWSTR(line.as_mut_ptr())),
                None,
                None,
                !detached,
                PROCESS_CREATION_FLAGS(flags.0),
                Some(env.as_ptr() as *const c_void),
                PCWSTR(cwd.as_ptr()),
                &si.StartupInfo,
                &mut pi,
            );
            if !si.lpAttributeList.0.is_null() {
                DeleteProcThreadAttributeList(si.lpAttributeList);
            }
            drop(attr_buf);
            drop(inherit);
            created?;
            // Our copies of the child's ends: the reader sees the end when the child exits.
            drop(child_ends);
            let process = Arc::new(Owned(pi.hProcess));
            let thread = Owned(pi.hThread);
            let mut job = None;
            if !detached {
                let j = Owned(CreateJobObjectW(None, PCWSTR::null())?);
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    j.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )?;
                if let Err(e) = AssignProcessToJobObject(j.0, process.0) {
                    let _ = TerminateProcess(process.0, 1);
                    return Err(e.into());
                }
                ResumeThread(thread.0);
                job = Some(j);
            }
            drop(thread);

            if !detached {
                let mut it = ours.into_iter();
                let stdin = it.next();
                let stdout = it.next();
                let stderr = it.next();
                // Feed stdin, then close it (the program sees the end of its input).
                let input = spec.stdin.clone();
                std::thread::spawn(move || {
                    let stdin = stdin;
                    if let (Some(h), Some(bytes)) = (&stdin, input) {
                        let mut rest = bytes.as_slice();
                        while !rest.is_empty() {
                            let mut n = 0u32;
                            if WriteFile(h.0, Some(rest), Some(&mut n), None).is_err() || n == 0 {
                                break;
                            }
                            rest = &rest[n as usize..];
                        }
                    }
                });
                let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
                for h in [stdout, stderr].into_iter().flatten() {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let h = h;
                        let mut buf = vec![0u8; 16 * 1024];
                        loop {
                            let mut n = 0u32;
                            if ReadFile(h.0, Some(&mut buf), Some(&mut n), None).is_err() || n == 0
                            {
                                break;
                            }
                            if tx.send(buf[..n as usize].to_vec()).is_err() {
                                break;
                            }
                        }
                    });
                }
                drop(tx);
                let reader_exit = exit.clone();
                std::thread::spawn(move || {
                    while let Ok(chunk) = rx.recv() {
                        on_output(&chunk);
                    }
                    // Both pipes are done; the end marker comes once the exit code is known.
                    while reader_exit.get().is_none() {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    on_output(&[]);
                });
            }
            let waiter = process.clone();
            let waiter_exit = exit.clone();
            std::thread::spawn(move || {
                let mut code = 1u32;
                if WaitForSingleObject(waiter.0, INFINITE) == WAIT_OBJECT_0 {
                    let _ = GetExitCodeProcess(waiter.0, &mut code);
                }
                waiter_exit.set(code as i32);
            });
            Ok(Proc {
                pid: pi.dwProcessId,
                process,
                job,
                exit,
            })
        }
    }

    impl Proc {
        pub fn kill(&self) {
            // SAFETY: live handles.
            unsafe {
                if let Some(j) = &self.job {
                    let _ = TerminateJobObject(j.0, 1);
                }
                let _ = TerminateProcess(self.process.0, 1);
            }
        }

        pub fn try_wait(&self) -> Option<i32> {
            self.exit.get()
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::io::{self, Read, Write};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::Arc;

    use super::{Exit, Io, Launch, OutputFn, Spawn};

    pub struct Proc {
        pub pid: u32,
        group: bool,
        exit: Arc<Exit>,
    }

    pub fn spawn(spec: &Spawn, l: &Launch, mut on_output: OutputFn) -> io::Result<Proc> {
        let detached = spec.io == Io::Detached;
        let mut c = Command::new(&l.program);
        c.args(&spec.args)
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(spec.env.iter().map(|(k, v)| (k, v)));
        if detached {
            c.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            c.stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        }
        // Its own session and process group: the tree is killed as one (and a detached program
        // doesn't get our terminal's signals).
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            c.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = c.spawn()?;
        let pid = child.id();
        let exit = Arc::new(Exit::default());
        if !detached {
            if let (Some(mut stdin), Some(bytes)) = (child.stdin.take(), spec.stdin.clone()) {
                std::thread::spawn(move || {
                    let _ = stdin.write_all(&bytes);
                });
            }
            let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
            let out: Option<Box<dyn Read + Send>> = child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>);
            let err: Option<Box<dyn Read + Send>> = child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>);
            for mut r in [out, err].into_iter().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let mut buf = vec![0u8; 16 * 1024];
                    loop {
                        match r.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if tx.send(buf[..n].to_vec()).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
            drop(tx);
            let reader_exit = exit.clone();
            std::thread::spawn(move || {
                while let Ok(chunk) = rx.recv() {
                    on_output(&chunk);
                }
                while reader_exit.get().is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                on_output(&[]);
            });
        }
        let waiter_exit = exit.clone();
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
            waiter_exit.set(code);
        });
        Ok(Proc {
            pid,
            group: !detached,
            exit,
        })
    }

    impl Proc {
        pub fn kill(&self) {
            if self.group {
                // SAFETY: plain signal delivery to the group we created.
                unsafe { libc::killpg(self.pid as libc::pid_t, libc::SIGKILL) };
            }
        }

        pub fn try_wait(&self) -> Option<i32> {
            self.exit.get()
        }
    }
}

/// Whether a Windows program is a GUI-subsystem executable (these don't attach to a
/// pseudo-console, so `io="auto"` gives them pipes). Reads the PE header.
pub fn is_gui_program(path: &std::path::Path) -> bool {
    fn read(path: &std::path::Path) -> Option<u16> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path).ok()?;
        let mut mz = [0u8; 64];
        f.read_exact(&mut mz).ok()?;
        if &mz[..2] != b"MZ" {
            return None;
        }
        let pe = u32::from_le_bytes(mz[0x3c..0x40].try_into().ok()?) as u64;
        // "PE\0\0", the 20-byte COFF header, then the optional header, whose Subsystem field is
        // at offset 68 in both PE32 and PE32+.
        f.seek(SeekFrom::Start(pe)).ok()?;
        let mut sig = [0u8; 4];
        f.read_exact(&mut sig).ok()?;
        if &sig != b"PE\0\0" {
            return None;
        }
        f.seek(SeekFrom::Start(pe + 4 + 20 + 68)).ok()?;
        let mut sub = [0u8; 2];
        f.read_exact(&mut sub).ok()?;
        Some(u16::from_le_bytes(sub))
    }
    const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;
    read(path) == Some(IMAGE_SUBSYSTEM_WINDOWS_GUI)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn shell(script: &str) -> (PathBuf, Vec<String>) {
        if cfg!(windows) {
            (
                crate::platform::process::find_program("cmd", std::path::Path::new("."), None)
                    .unwrap(),
                vec!["/d".into(), "/c".into(), script.into()],
            )
        } else {
            (PathBuf::from("/bin/sh"), vec!["-c".into(), script.into()])
        }
    }

    fn collect(rx: &mpsc::Receiver<Vec<u8>>) -> String {
        let mut all = Vec::new();
        while let Ok(c) = rx.recv_timeout(Duration::from_secs(10)) {
            if c.is_empty() {
                break;
            }
            all.extend(c);
        }
        String::from_utf8_lossy(&all).into_owned()
    }

    #[test]
    fn pipes_carry_stdin_and_output() {
        let (program, args) = if cfg!(windows) {
            shell("findstr x")
        } else {
            shell("grep x")
        };
        let (tx, rx) = mpsc::channel();
        let c = spawn(
            &Spawn {
                program,
                args,
                raw_command_line: None,
                cwd: std::env::temp_dir(),
                env: std::env::vars_os().collect(),
                stdin: Some(b"a\nxyz\nb\n".to_vec()),
                io: Io::Pipes,
            },
            Box::new(move |b| {
                let _ = tx.send(b.to_vec());
            }),
        )
        .unwrap();
        assert_eq!(c.wait_timeout(Duration::from_secs(10)), Some(0));
        assert_eq!(collect(&rx).trim(), "xyz");
    }

    #[test]
    fn a_tree_is_killed_as_one() {
        // A shell that starts a long sleep: killing the shell must end the sleep too.
        let (program, args) = if cfg!(windows) {
            shell("ping -n 30 127.0.0.1 >nul")
        } else {
            shell("sleep 30; true")
        };
        let c = spawn(
            &Spawn {
                program,
                args,
                raw_command_line: None,
                cwd: std::env::temp_dir(),
                env: std::env::vars_os().collect(),
                stdin: None,
                io: Io::Pipes,
            },
            Box::new(|_| {}),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let start = Instant::now();
        c.kill();
        assert!(c.wait_timeout(Duration::from_secs(5)).is_some());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[cfg(windows)]
    #[test]
    fn knows_gui_programs() {
        let windir = std::env::var("WINDIR").unwrap();
        assert!(is_gui_program(std::path::Path::new(&format!(
            r"{windir}\explorer.exe"
        ))));
        assert!(!is_gui_program(std::path::Path::new(&format!(
            r"{windir}\System32\cmd.exe"
        ))));
    }

    #[cfg(windows)]
    #[test]
    fn batch_arguments_cannot_inject_commands() {
        let spec = Spawn {
            program: PathBuf::from(r"C:\x\build.bat"),
            args: vec!["ok".into(), "a & calc".into()],
            raw_command_line: None,
            cwd: std::env::temp_dir(),
            env: vec![],
            stdin: None,
            io: Io::Pipes,
        };
        assert!(launch(&spec).is_err());
        let ok = Spawn {
            args: vec!["Release".into(), "with space".into()],
            ..spec
        };
        assert_eq!(
            launch(&ok).unwrap().command_line,
            r#"cmd.exe /d /s /c "C:\x\build.bat Release "with space"""#
        );
    }
}

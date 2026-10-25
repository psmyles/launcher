//! A process running in a pseudo-terminal (N7): the console shows its progress bars and colours,
//! and the user can type into it.
//!
//! * **Windows:** ConPTY, spawned by our own `CreateProcessW` so a raw command line passes through
//!   untouched. The process starts suspended, joins a kill-on-close Job Object (so cancelling
//!   kills the whole tree), then resumes.
//! * **macOS:** `openpty`; the child gets a new session with the terminal as its controlling tty.
//!   Its process group is the tree that cancel kills.
//!
//! Output arrives on a reader thread through the `on_output` callback; an empty slice marks the
//! end, and comes only once the exit code is known too. `wait` blocks for the exit code.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

/// What to run.
#[derive(Clone, Debug, Default)]
pub struct PtyCommand {
    /// The resolved program (see `process::find_program`).
    pub program: PathBuf,
    /// Arguments after the program.
    pub args: Vec<String>,
    /// Windows only: the whole command line, used verbatim instead of quoting `args` (N5).
    pub raw_command_line: Option<String>,
    pub cwd: Option<PathBuf>,
    /// The complete environment; `None` inherits ours.
    pub env: Option<Vec<(OsString, OsString)>>,
    pub cols: u16,
    pub rows: u16,
}

pub type OutputFn = Box<dyn FnMut(&[u8]) + Send + 'static>;

/// The exit code once known, shared with the waiter thread.
#[derive(Default)]
struct Exit {
    code: Mutex<Option<i32>>,
    cv: Condvar,
}

impl Exit {
    fn set(&self, code: i32) {
        if let Ok(mut c) = self.code.lock() {
            *c = Some(code);
        }
        self.cv.notify_all();
    }

    fn get(&self) -> Option<i32> {
        self.code.lock().ok().and_then(|c| *c)
    }

    fn wait(&self) -> i32 {
        let Ok(mut c) = self.code.lock() else {
            return -1;
        };
        loop {
            if let Some(code) = *c {
                return code;
            }
            c = match self.cv.wait(c) {
                Ok(c) => c,
                Err(_) => return -1,
            };
        }
    }
}

pub struct Pty {
    imp: imp::Pty,
    exit: Arc<Exit>,
}

impl Pty {
    pub fn spawn(cmd: &PtyCommand, on_output: OutputFn) -> io::Result<Pty> {
        let exit = Arc::new(Exit::default());
        let imp = imp::Pty::spawn(cmd, on_output, exit.clone())?;
        Ok(Pty { imp, exit })
    }

    pub fn pid(&self) -> u32 {
        self.imp.pid()
    }

    /// Send keystrokes (already encoded as terminal input).
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.imp.write(bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.imp.resize(cols.max(1), rows.max(1));
    }

    /// Kill the process and everything it started.
    pub fn kill(&self) {
        self.imp.kill();
    }

    pub fn try_wait(&self) -> Option<i32> {
        self.exit.get()
    }

    pub fn wait(&self) -> i32 {
        self.exit.wait()
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::{Arc, Mutex};

    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
    use windows::Win32::System::Console::{
        ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
    };
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Pipes::CreatePipe;
    use windows::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
        InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
        UpdateProcThreadAttribute, WaitForSingleObject, CREATE_SUSPENDED,
        CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
        LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
        STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    use super::{Exit, OutputFn, PtyCommand};
    use crate::platform::process;

    /// A handle owned by this module, closed on drop.
    struct Owned(HANDLE);
    // SAFETY: kernel handles may be used from any thread.
    unsafe impl Send for Owned {}
    unsafe impl Sync for Owned {}
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                // SAFETY: we own the handle and close it once.
                let _ = unsafe { CloseHandle(self.0) };
            }
        }
    }

    /// The pseudo console, closed once (after the process exits, so the output pipe ends).
    struct Console(Mutex<Option<HPCON>>);
    // SAFETY: an HPCON is a kernel object handle, usable from any thread.
    unsafe impl Send for Console {}
    unsafe impl Sync for Console {}
    impl Console {
        fn close(&self) {
            if let Ok(mut h) = self.0.lock() {
                if let Some(h) = h.take() {
                    // SAFETY: closed exactly once; the reader keeps draining meanwhile, which
                    // older Windows builds need for this call to return.
                    unsafe { ClosePseudoConsole(h) };
                }
            }
        }
    }

    pub struct Pty {
        pid: u32,
        input: Owned,
        process: Arc<Owned>,
        job: Owned,
        console: Arc<Console>,
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    impl Pty {
        pub fn spawn(
            cmd: &PtyCommand,
            mut on_output: OutputFn,
            exit: Arc<Exit>,
        ) -> io::Result<Pty> {
            let size = COORD {
                X: cmd.cols.max(1) as i16,
                Y: cmd.rows.max(1) as i16,
            };
            // SAFETY: straightforward Win32 sequence; every handle is owned by an `Owned` or
            // handed to the pseudo console, and the attribute list outlives CreateProcessW.
            unsafe {
                let (mut in_read, mut in_write) = (HANDLE::default(), HANDLE::default());
                let (mut out_read, mut out_write) = (HANDLE::default(), HANDLE::default());
                CreatePipe(&mut in_read, &mut in_write, None, 0)?;
                let in_read = Owned(in_read);
                let in_write = Owned(in_write);
                CreatePipe(&mut out_read, &mut out_write, None, 0)?;
                let out_read = Owned(out_read);
                let out_write = Owned(out_write);
                let hpc = CreatePseudoConsole(size, in_read.0, out_write.0, 0)?;
                let console = Arc::new(Console(Mutex::new(Some(hpc))));
                // The console holds its own references to the pty ends.
                drop(in_read);
                drop(out_write);

                let mut attr_size = 0usize;
                let _ = InitializeProcThreadAttributeList(None, 1, None, &mut attr_size);
                let mut attr_buf = vec![0u8; attr_size];
                let attrs = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr() as *mut c_void);
                InitializeProcThreadAttributeList(Some(attrs), 1, None, &mut attr_size)?;
                let update = UpdateProcThreadAttribute(
                    attrs,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                    Some(hpc.0 as *const c_void),
                    std::mem::size_of::<HPCON>(),
                    None,
                    None,
                );
                if let Err(e) = update {
                    DeleteProcThreadAttributeList(attrs);
                    console.close();
                    return Err(e.into());
                }

                let mut si = STARTUPINFOEXW::default();
                si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
                si.lpAttributeList = attrs;
                // No inherited standard handles: without this a launcher that borrowed its
                // parent's console would hand that console to the child instead of the pty.
                si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
                si.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
                si.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
                si.StartupInfo.hStdError = INVALID_HANDLE_VALUE;

                let line = match &cmd.raw_command_line {
                    Some(raw) => raw.clone(),
                    None => {
                        process::windows_command_line(&cmd.program.display().to_string(), &cmd.args)
                    }
                };
                let mut line_w: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
                let app_w = wide(cmd.program.as_os_str());
                let cwd_w = cmd.cwd.as_ref().map(|c| wide(c.as_os_str()));
                let env_block = cmd.env.as_ref().map(|e| process::windows_env_block(e));
                let mut pi = PROCESS_INFORMATION::default();
                let created = CreateProcessW(
                    PCWSTR(app_w.as_ptr()),
                    Some(PWSTR(line_w.as_mut_ptr())),
                    None,
                    None,
                    false,
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                    env_block.as_ref().map(|b| b.as_ptr() as *const c_void),
                    match &cwd_w {
                        Some(c) => PCWSTR(c.as_ptr()),
                        None => PCWSTR::null(),
                    },
                    &si.StartupInfo,
                    &mut pi,
                );
                DeleteProcThreadAttributeList(attrs);
                drop(attr_buf);
                if let Err(e) = created {
                    console.close();
                    return Err(e.into());
                }
                let process = Arc::new(Owned(pi.hProcess));
                let thread = Owned(pi.hThread);

                let job = Owned(CreateJobObjectW(None, PCWSTR::null())?);
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )?;
                if let Err(e) = AssignProcessToJobObject(job.0, process.0) {
                    let _ = TerminateProcess(process.0, 1);
                    console.close();
                    return Err(e.into());
                }
                ResumeThread(thread.0);
                drop(thread);

                let reader_exit = exit.clone();
                std::thread::Builder::new()
                    .name("pty-reader".into())
                    .spawn(move || {
                        // Capture the owning wrapper, not its raw handle field.
                        let out_read = out_read;
                        let mut buf = vec![0u8; 16 * 1024];
                        loop {
                            let mut n = 0u32;
                            let ok = ReadFile(out_read.0, Some(&mut buf), Some(&mut n), None);
                            if ok.is_err() || n == 0 {
                                break;
                            }
                            on_output(&buf[..n as usize]);
                        }
                        reader_exit.wait();
                        on_output(&[]);
                    })?;
                let waiter_process = process.clone();
                let waiter_console = console.clone();
                std::thread::Builder::new()
                    .name("pty-waiter".into())
                    .spawn(move || {
                        let mut code = 1u32;
                        if WaitForSingleObject(waiter_process.0, INFINITE) == WAIT_OBJECT_0 {
                            let _ = GetExitCodeProcess(waiter_process.0, &mut code);
                        }
                        // The output pipe stays open until the console closes; closing it now
                        // lets the reader drain the rest and see the end.
                        waiter_console.close();
                        exit.set(code as i32);
                    })?;

                Ok(Pty {
                    pid: pi.dwProcessId,
                    input: in_write,
                    process,
                    job,
                    console,
                })
            }
        }

        pub fn pid(&self) -> u32 {
            self.pid
        }

        pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
            let mut rest = bytes;
            while !rest.is_empty() {
                let mut n = 0u32;
                // SAFETY: a live pipe handle and a live buffer.
                unsafe { WriteFile(self.input.0, Some(rest), Some(&mut n), None)? };
                rest = &rest[n as usize..];
            }
            Ok(())
        }

        pub fn resize(&self, cols: u16, rows: u16) {
            if let Ok(h) = self.console.0.lock() {
                if let Some(h) = *h {
                    // SAFETY: a live pseudo console.
                    let _ = unsafe {
                        ResizePseudoConsole(
                            h,
                            COORD {
                                X: cols as i16,
                                Y: rows as i16,
                            },
                        )
                    };
                }
            }
        }

        pub fn kill(&self) {
            // SAFETY: live job and process handles.
            unsafe {
                let _ = TerminateJobObject(self.job.0, 1);
                let _ = TerminateProcess(self.process.0, 1);
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};

    use super::{Exit, OutputFn, PtyCommand};

    pub struct Pty {
        pid: u32,
        master: Mutex<File>,
    }

    fn winsize(cols: u16, rows: u16) -> libc::winsize {
        libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }

    impl Pty {
        pub fn spawn(
            cmd: &PtyCommand,
            mut on_output: OutputFn,
            exit: Arc<Exit>,
        ) -> io::Result<Pty> {
            let (mut master, mut slave) = (0, 0);
            let mut ws = winsize(cmd.cols.max(1), cmd.rows.max(1));
            // SAFETY: openpty fills two fds; the null pointers mean "no name, default termios".
            let r = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut ws,
                )
            };
            if r != 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: openpty returned two fresh fds that we now own.
            let (master, slave) =
                unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };

            let mut c = Command::new(&cmd.program);
            c.args(&cmd.args)
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(slave));
            if let Some(dir) = &cmd.cwd {
                c.current_dir(dir);
            }
            if let Some(env) = &cmd.env {
                c.env_clear().envs(env.iter().map(|(k, v)| (k, v)));
            }
            c.env("TERM", "xterm-256color");
            // SAFETY: only async-signal-safe calls between fork and exec.
            unsafe {
                c.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = c.spawn()?;
            // Our copies of the slave end are gone with `c`; the reader sees EOF when the child
            // and everything it started have closed theirs.
            drop(c);
            let pid = child.id();
            let master = File::from(master);
            let mut reader = master.try_clone()?;
            let reader_exit = exit.clone();
            std::thread::Builder::new()
                .name("pty-reader".into())
                .spawn(move || {
                    let mut buf = vec![0u8; 16 * 1024];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => on_output(&buf[..n]),
                        }
                    }
                    reader_exit.wait();
                    on_output(&[]);
                })?;
            std::thread::Builder::new()
                .name("pty-waiter".into())
                .spawn(move || {
                    let code = match child.wait() {
                        Ok(s) => s.code().unwrap_or(-1),
                        Err(_) => -1,
                    };
                    exit.set(code);
                })?;
            Ok(Pty {
                pid,
                master: Mutex::new(master),
            })
        }

        pub fn pid(&self) -> u32 {
            self.pid
        }

        pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
            let mut m = self
                .master
                .lock()
                .map_err(|_| io::Error::other("pty lock poisoned"))?;
            m.write_all(bytes)
        }

        pub fn resize(&self, cols: u16, rows: u16) {
            if let Ok(m) = self.master.lock() {
                let ws = winsize(cols, rows);
                // SAFETY: a live pty master fd and a valid winsize.
                unsafe { libc::ioctl(m.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
            }
        }

        pub fn kill(&self) {
            // The child is a session (and process group) leader: kill the whole group.
            // SAFETY: plain signal delivery.
            unsafe { libc::killpg(self.pid as libc::pid_t, libc::SIGKILL) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn runs_a_program_and_collects_its_output() {
        let (program, args) = if cfg!(windows) {
            (
                crate::platform::process::find_program("cmd", std::path::Path::new("."), None)
                    .unwrap(),
                vec!["/c".to_string(), "echo pty-ok".to_string()],
            )
        } else {
            (
                PathBuf::from("/bin/sh"),
                vec!["-c".to_string(), "echo pty-ok".to_string()],
            )
        };
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let pty = Pty::spawn(
            &PtyCommand {
                program,
                args,
                cols: 80,
                rows: 24,
                ..Default::default()
            },
            Box::new(move |b| {
                let _ = tx.send(b.to_vec());
            }),
        )
        .unwrap();
        assert_eq!(pty.wait(), 0);
        let mut all = Vec::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(10)) {
            if chunk.is_empty() {
                break;
            }
            all.extend(chunk);
        }
        let text = String::from_utf8_lossy(&all);
        assert!(text.contains("pty-ok"), "{text:?}");
    }
}

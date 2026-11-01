//! Keeping the computer awake while a run is going (M4): a long build shouldn't be cut short by
//! the machine going to sleep. The display may still turn off.

/// Keeps the system awake until dropped. Windows: a thread execution state, so create and drop it
/// on the same thread (the run worker). macOS: a `caffeinate` process tied to ours. Linux: a
/// `systemd-inhibit` sleep lock, held by a process that also ends with ours.
pub struct KeepAwake {
    #[cfg(unix)]
    child: Option<std::process::Child>,
    #[cfg(windows)]
    _not_send: std::marker::PhantomData<*const ()>,
}

impl KeepAwake {
    pub fn start() -> KeepAwake {
        #[cfg(windows)]
        {
            use windows::Win32::System::Power::{
                SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
            };
            // SAFETY: plain call; it changes this thread's state only.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            KeepAwake {
                _not_send: std::marker::PhantomData,
            }
        }
        #[cfg(target_os = "macos")]
        {
            let child = std::process::Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", &std::process::id().to_string()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            KeepAwake { child }
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // Its own process group, so dropping it ends the inhibitor and its `tail` together.
            let child = std::process::Command::new("systemd-inhibit")
                .args([
                    "--what=sleep",
                    "--who=Launcher",
                    "--why=A run is going",
                    "--mode=block",
                    "tail",
                    "-f",
                    "/dev/null",
                    "--pid",
                    &std::process::id().to_string(),
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .process_group(0)
                .spawn()
                .ok();
            KeepAwake { child }
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS};
            // SAFETY: plain call; resets this thread's state.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
        #[cfg(target_os = "macos")]
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        #[cfg(target_os = "linux")]
        if let Some(mut c) = self.child.take() {
            // SAFETY: signals the process group we created for the child.
            unsafe { libc::kill(-(c.id() as libc::pid_t), libc::SIGTERM) };
            let _ = c.wait();
        }
    }
}

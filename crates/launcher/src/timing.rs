//! Startup measurement, inert unless asked (master's protocol, so `xtask ttfp` numbers stay
//! comparable with M0):
//!
//! * `LAUNCHER_TIMING=1` prints a startup breakdown to stderr; `LAUNCHER_TIMING=<path>` appends
//!   it to that file (a release build started from Explorer or Finder has no console).
//! * `LAUNCHER_TTFP_OUT=<path>` writes the milliseconds from kernel process creation to the first
//!   frame on screen (presented and `set_visible(true)` returned) into `<path>`, then exits at
//!   once. Process creation, not `main()`: the loader and runtime start are paid by the user too.

use std::io::Write as _;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds from the kernel's process-creation time to now (NaN if unknown).
pub fn ms_since_start() -> f64 {
    match process_start_us() {
        Some(start) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_micros() as u64)
                .unwrap_or(0);
            now.saturating_sub(start) as f64 / 1000.0
        }
        None => f64::NAN,
    }
}

#[cfg(windows)]
fn process_start_us() -> Option<u64> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;
    let (mut created, mut exit, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: the pseudo-handle is always valid; the out-params are ours.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }
    .ok()?;
    let ft = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
    Some(ft.saturating_sub(UNIX_EPOCH_AS_FILETIME) / 10)
}

#[cfg(target_os = "macos")]
fn process_start_us() -> Option<u64> {
    // SAFETY: plain libproc query into a zeroed struct of the size we pass.
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        let n = libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut std::ffi::c_void,
            size,
        );
        (n == size).then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn process_start_us() -> Option<u64> {
    None
}

/// Append a startup-timing line to wherever `LAUNCHER_TIMING` points. A cheap no-op when unset.
pub fn report(line: &str) {
    static SINK: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();
    let Some(sink) = SINK.get_or_init(|| std::env::var_os("LAUNCHER_TIMING")) else {
        return;
    };
    let line = format!("[{:8.2} ms] {line}", ms_since_start());
    if sink == "1" {
        let _ = writeln!(std::io::stderr(), "launcher: {line}");
    } else if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(sink)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// The first frame is on screen: record it, and exit at once if `LAUNCHER_TTFP_OUT` is set.
pub fn first_frame_shown() {
    report("first frame presented, window shown");
    static OUT: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    let Some(path) = OUT.get_or_init(|| std::env::var_os("LAUNCHER_TTFP_OUT").map(Into::into))
    else {
        return;
    };
    let ms = ms_since_start();
    let _ = std::fs::write(path, format!("{ms:.3}\n"));
    exit_now();
}

/// Exit without teardown (driver unloading in DLL detach must not delay the next launch).
fn exit_now() -> ! {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        // SAFETY: terminating ourselves, by design.
        let _ = unsafe { TerminateProcess(GetCurrentProcess(), 0) };
    }
    #[cfg(unix)]
    {
        // SAFETY: immediate exit without atexit handlers, by design.
        unsafe { libc::_exit(0) }
    }
    #[allow(unreachable_code)]
    std::process::exit(0)
}

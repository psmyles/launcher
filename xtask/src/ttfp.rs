//! `cargo xtask ttfp` and `cargo xtask idle` (docs/PLAN.md, M0 and M1).
//!
//! **ttfp:** launches the release launcher N+1 times through master's `LAUNCHER_TTFP_OUT` protocol
//! (ms from kernel process creation to the first frame on screen), drops the warm-up launch, and
//! reports the upper median (as master's `ttfp.ps1`), p90 and a 95% order-statistic interval.
//! Fails when the median exceeds the budget (150 ms, N8). On Windows each launch also records the
//! out-of-process "window shown" time (`EVENT_OBJECT_SHOW` on the real window).
//!
//! **idle:** starts the launcher, lets it settle, and measures the CPU it uses while nothing
//! happens. An idle window draws nothing (N15), so this must be about zero.
//!
//! Binaries are timed from a copy in a normal folder (`%LOCALAPPDATA%\Programs\launcher-ttfp` on
//! Windows): a Dev Drive or `target/` changes what Defender does on launch (M0).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::util::{self, flag, step, value};

/// Where timed binaries are copied.
fn timing_dir() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
            .join("Programs")
            .join("launcher-ttfp")
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir().join("launcher-ttfp")
    }
}

fn staged_exe(args: &[String]) -> Result<PathBuf, String> {
    if !flag(args, "--no-build") {
        util::build_release_launcher()?;
    }
    let src = util::launcher_exe();
    let dir = timing_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dst = dir.join(src.file_name().expect("exe name"));
    std::fs::copy(&src, &dst).map_err(|e| format!("copy to {}: {e}", dst.display()))?;
    Ok(dst)
}

fn file_arg(args: &[String]) -> Option<PathBuf> {
    value(args, "--file").map(|f| util::root().join(f))
}

#[cfg(windows)]
fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

struct Launch {
    ttfp_ms: f64,
    oop_shown_ms: Option<f64>,
}

fn launch_once(exe: &Path, file: Option<&Path>, out: &Path) -> Result<Launch, String> {
    let _ = std::fs::remove_file(out);
    let mut cmd = Command::new(exe);
    if let Some(f) = file {
        cmd.arg(f);
    }
    cmd.env("LAUNCHER_TTFP_OUT", out)
        .env("LAUNCHER_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    #[cfg(windows)]
    winevent::arm(child.id());
    let start = Instant::now();
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if start.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("a launch did not finish within 15 s".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    #[cfg(windows)]
    let oop_shown_ms = {
        let shown = winevent::take();
        let created = winevent::creation_us(&child);
        match (shown, created) {
            (Some(s), Some(c)) => Some(s.saturating_sub(c) as f64 / 1000.0),
            _ => None,
        }
    };
    #[cfg(not(windows))]
    let oop_shown_ms = None;
    let text = std::fs::read_to_string(out)
        .map_err(|_| "the launcher exited without writing its time to first frame".to_string())?;
    let ttfp_ms = text
        .trim()
        .parse()
        .map_err(|_| format!("unreadable time: {text:?}"))?;
    Ok(Launch {
        ttfp_ms,
        oop_shown_ms,
    })
}

fn sorted(mut xs: Vec<f64>) -> Vec<f64> {
    xs.retain(|x| x.is_finite());
    xs.sort_by(|a, b| a.total_cmp(b));
    xs
}

/// Upper median, as master's ttfp.ps1.
fn median(s: &[f64]) -> f64 {
    s.get(s.len() / 2).copied().unwrap_or(f64::NAN)
}

fn percentile(s: &[f64], p: f64) -> f64 {
    if s.is_empty() {
        return f64::NAN;
    }
    let k = ((p * s.len() as f64).ceil() as usize).clamp(1, s.len());
    s[k - 1]
}

/// Distribution-free 95% confidence interval of the median.
fn median_ci(s: &[f64]) -> Option<(f64, f64)> {
    if s.len() < 6 {
        return None;
    }
    let n = s.len() as f64;
    let half = 1.96 * n.sqrt() / 2.0;
    let lo = ((n / 2.0 - half).floor() as usize).max(1);
    let hi = ((n / 2.0 + 1.0 + half).ceil() as usize).min(s.len());
    Some((s[lo - 1], s[hi - 1]))
}

pub fn run_ttfp(args: &[String]) -> Result<(), String> {
    let n: usize = value(args, "--n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let budget: f64 = value(args, "--budget")
        .and_then(|v| v.parse().ok())
        .unwrap_or(150.0);
    let file = file_arg(args);
    let exe = staged_exe(args)?;
    #[cfg(windows)]
    winevent::start();
    step(&format!(
        "time to first frame: {} launches after a warm-up{}",
        n,
        file.as_ref()
            .map(|f| format!(", {}", f.display()))
            .unwrap_or_default()
    ));
    let out = timing_dir().join(format!("ttfp-{}.txt", std::process::id()));
    let mut first = None;
    let mut ttfp = Vec::new();
    let mut shown = Vec::new();
    for i in 0..=n {
        let l = launch_once(&exe, file.as_deref(), &out)?;
        if i == 0 {
            first = Some(l.ttfp_ms);
        } else {
            ttfp.push(l.ttfp_ms);
            if let Some(s) = l.oop_shown_ms {
                shown.push(s);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let _ = std::fs::remove_file(&out);
    let s = sorted(ttfp);
    let med = median(&s);
    eprintln!(
        "  first launch {:.1} ms (warm-up, not counted)",
        first.unwrap_or(f64::NAN)
    );
    eprintln!(
        "  median {med:.1} ms, p90 {:.1}, min {:.1}, max {:.1}{}",
        percentile(&s, 0.9),
        s.first().copied().unwrap_or(f64::NAN),
        s.last().copied().unwrap_or(f64::NAN),
        median_ci(&s)
            .map(|(lo, hi)| format!(", 95% CI {lo:.1}-{hi:.1}"))
            .unwrap_or_default()
    );
    if !shown.is_empty() {
        eprintln!(
            "  window shown (out of process): median {:.1} ms",
            median(&sorted(shown))
        );
    }
    if med > budget {
        return Err(format!(
            "median time to first frame {med:.1} ms is over the {budget} ms budget"
        ));
    }
    eprintln!("  within the {budget} ms budget");
    Ok(())
}

pub fn run_idle(args: &[String]) -> Result<(), String> {
    let seconds: u64 = value(args, "--seconds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let file = file_arg(args);
    let exe = staged_exe(args)?;
    step(&format!("idle CPU over {seconds} s"));
    let mut cmd = Command::new(&exe);
    if let Some(f) = &file {
        cmd.arg(f);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    // Let startup finish: the first frame, the switch to the hardware GPU, egui's settle frames.
    std::thread::sleep(Duration::from_secs(3));
    let t0 = cpu_ms(&child)?;
    std::thread::sleep(Duration::from_secs(seconds));
    let t1 = cpu_ms(&child)?;
    close(&mut child);
    let used = t1 - t0;
    eprintln!("  {used:.1} ms of CPU in {seconds} s");
    // Scheduler-tick granularity makes a few ticks of noise possible; anything more is drawing.
    let limit = 50.0 * seconds as f64 / 10.0;
    if used > limit {
        return Err(format!(
            "the idle window used {used:.1} ms of CPU (limit {limit:.0} ms): it is drawing while idle"
        ));
    }
    eprintln!("  idle: ok");
    Ok(())
}

#[cfg(windows)]
fn cpu_ms(child: &std::process::Child) -> Result<f64, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
    use windows_sys::Win32::System::Threading::GetProcessTimes;
    let z = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut t = [z; 4];
    // SAFETY: the child's handle is open (the `Child` owns it).
    let ok = unsafe {
        let [a, b, c, d] = &mut t;
        GetProcessTimes(child.as_raw_handle() as HANDLE, a, b, c, d)
    };
    if ok == 0 {
        return Err("could not read the launcher's CPU time".into());
    }
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    Ok((ft(t[2]) + ft(t[3])) as f64 / 10_000.0)
}

#[cfg(not(windows))]
fn cpu_ms(child: &std::process::Child) -> Result<f64, String> {
    // `ps -o time=` prints [[dd-]hh:]mm:ss.cc.
    let out = Command::new("ps")
        .args(["-o", "time=", "-p", &child.id().to_string()])
        .output()
        .map_err(|e| format!("ps: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut secs = 0.0;
    for part in text.split(':') {
        secs = secs * 60.0
            + part
                .parse::<f64>()
                .map_err(|_| format!("ps said {text:?}"))?;
    }
    Ok(secs * 1000.0)
}

/// Close the launcher's window (so it saves its state), then make sure it's gone.
fn close(child: &mut std::process::Child) {
    #[cfg(windows)]
    winevent::close_windows_of(child.id());
    #[cfg(unix)]
    {
        // SAFETY: plain signal delivery to our own child.
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Out-of-process "window shown" (Windows): a WinEvent hook for `EVENT_OBJECT_SHOW`, scoped to
/// the child's process, on its own thread with a message loop. Out-of-context hooks are posted,
/// so the child never waits on it. A window that is already gone when the event is handled is
/// accepted (a launch that stamps and exits at once); winit's helper window is shown at startup,
/// long before exit, so it is always still there to be filtered out by class (LESSONS L2.11).
#[cfg(windows)]
mod winevent {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{FILETIME, HANDLE, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{GetCurrentThreadId, GetProcessTimes};
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, EnumWindows, GetAncestor, GetClassNameW, GetMessageW, GetWindow,
        GetWindowThreadProcessId, IsWindow, IsWindowVisible, PeekMessageW, PostMessageW,
        PostThreadMessageW, CHILDID_SELF, EVENT_OBJECT_SHOW, GA_ROOT, GW_OWNER, MSG, OBJID_WINDOW,
        PM_NOREMOVE, WINEVENT_OUTOFCONTEXT, WM_APP, WM_CLOSE,
    };

    const WM_ARM: u32 = WM_APP + 1;
    static THREAD: AtomicU32 = AtomicU32::new(0);
    static PID: AtomicU32 = AtomicU32::new(0);
    static SHOWN_US: AtomicU64 = AtomicU64::new(0);
    const HELPER_CLASSES: &[&str] = &["Winit Thread Event Target", "IME", "MSCTFIME UI"];

    fn class_of(hwnd: HWND) -> String {
        let mut class = [0u16; 256];
        // SAFETY: a buffer of the length we pass.
        let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) }.max(0);
        String::from_utf16_lossy(&class[..n as usize])
    }

    pub fn start() {
        std::thread::spawn(|| {
            // SAFETY: the hooks and the message loop live on this thread for the process's life.
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
                let mut hook: HWINEVENTHOOK = std::ptr::null_mut();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    if msg.hwnd.is_null() && msg.message == WM_ARM {
                        if !hook.is_null() {
                            UnhookWinEvent(hook);
                        }
                        let pid = msg.wParam as u32;
                        hook = SetWinEventHook(
                            EVENT_OBJECT_SHOW,
                            EVENT_OBJECT_SHOW,
                            std::ptr::null_mut(),
                            Some(on_event),
                            pid,
                            0,
                            WINEVENT_OUTOFCONTEXT,
                        );
                        PID.store(pid, Ordering::SeqCst);
                        continue;
                    }
                    DispatchMessageW(&msg);
                }
            }
        });
        while THREAD.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
    }

    pub fn arm(pid: u32) {
        SHOWN_US.store(0, Ordering::SeqCst);
        // SAFETY: posting a thread message to our own hook thread.
        unsafe { PostThreadMessageW(THREAD.load(Ordering::SeqCst), WM_ARM, pid as usize, 0) };
        let deadline = Instant::now() + Duration::from_millis(200);
        while PID.load(Ordering::SeqCst) != pid && Instant::now() < deadline {
            std::thread::yield_now();
        }
    }

    pub fn take() -> Option<u64> {
        std::thread::sleep(Duration::from_millis(20));
        PID.store(0, Ordering::SeqCst);
        let v = SHOWN_US.swap(0, Ordering::SeqCst);
        (v != 0).then_some(v)
    }

    pub fn creation_us(child: &std::process::Child) -> Option<u64> {
        use std::os::windows::io::AsRawHandle;
        let z = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut t = [z; 4];
        // SAFETY: the child's process handle is still open.
        let ok = unsafe {
            let [a, b, c, d] = &mut t;
            GetProcessTimes(child.as_raw_handle() as HANDLE, a, b, c, d)
        };
        let ft = ((t[0].dwHighDateTime as u64) << 32) | t[0].dwLowDateTime as u64;
        (ok != 0).then(|| ft.saturating_sub(116_444_736_000_000_000) / 10)
    }

    unsafe extern "system" fn on_event(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        id_object: i32,
        id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        let now = super::now_us();
        let pid = PID.load(Ordering::SeqCst);
        if pid == 0 || id_object != OBJID_WINDOW || id_child != CHILDID_SELF as i32 {
            return;
        }
        // SAFETY: plain window queries; a destroyed window just fails them.
        unsafe {
            if IsWindow(hwnd) != 0 {
                let mut owner_pid = 0u32;
                GetWindowThreadProcessId(hwnd, &mut owner_pid);
                if owner_pid != pid
                    || GetAncestor(hwnd, GA_ROOT) != hwnd
                    || !GetWindow(hwnd, GW_OWNER).is_null()
                    || HELPER_CLASSES.contains(&class_of(hwnd).as_str())
                {
                    return;
                }
            }
        }
        let _ = SHOWN_US.compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Post `WM_CLOSE` to the process's real top-level windows (never winit's helper, whose
    /// destruction the app wouldn't notice; LESSONS L2.11).
    pub fn close_windows_of(pid: u32) {
        unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> i32 {
            let mut p = 0u32;
            // SAFETY: plain window queries on an enumerated window.
            unsafe {
                GetWindowThreadProcessId(hwnd, &mut p);
                if p == lparam as u32
                    && IsWindowVisible(hwnd) != 0
                    && GetWindow(hwnd, GW_OWNER).is_null()
                    && !HELPER_CLASSES.contains(&class_of(hwnd).as_str())
                {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
            }
            1
        }
        // SAFETY: the callback only reads window properties and posts a message.
        unsafe { EnumWindows(Some(each), pid as LPARAM) };
    }
}

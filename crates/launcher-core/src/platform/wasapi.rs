//! A process's audio volume, through WASAPI's per-session `ISimpleAudioVolume` (replacing NAudio).
//!
//! A game creates its audio session some time after it starts, so this polls: every `poll_ms`,
//! enumerate the default render endpoint's sessions **again** (an `IAudioSessionEnumerator` is a
//! snapshot, which is why NAudio re-reads `Sessions` each time), find the one owned by `pid`, and
//! set its master volume. It gives up at the timeout, when the process exits, or on cancel.

use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
    ISimpleAudioVolume, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

/// Keeps COM initialized on this thread for the guard's lifetime.
pub struct ComGuard(bool);

impl ComGuard {
    pub fn new() -> ComGuard {
        // SAFETY: balanced by Drop on the same thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        ComGuard(hr.is_ok())
    }
}

impl Default for ComGuard {
    fn default() -> Self {
        ComGuard::new()
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: CoInitializeEx succeeded on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// Set the volume (0.0-1.0) of `pid`'s audio session. `alive` says whether the process is still
/// running; `cancelled` whether to stop waiting. Returns the milliseconds it took.
pub fn set_process_volume(
    pid: u32,
    volume: f32,
    timeout_ms: u64,
    poll_ms: u64,
    mut alive: impl FnMut() -> bool,
    cancelled: impl Fn() -> bool,
) -> Result<u64, String> {
    let _com = ComGuard::new();
    let start = Instant::now();
    let volume = volume.clamp(0.0, 1.0);
    loop {
        match try_once(pid, volume) {
            Ok(true) => return Ok(start.elapsed().as_millis() as u64),
            Ok(false) => {}
            Err(e) => crate::log_debug!("audio sessions: {e}"),
        }
        let elapsed = start.elapsed().as_millis() as u64;
        if elapsed >= timeout_ms {
            return Err(format!("Elapsed Time: {elapsed}ms"));
        }
        if !alive() {
            return Err("the program exited before it opened its audio".into());
        }
        if cancelled() {
            return Err("cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(
            poll_ms.min(timeout_ms - elapsed).max(1),
        ));
    }
}

fn try_once(pid: u32, volume: f32) -> Result<bool, String> {
    // SAFETY: COM calls on interfaces we own; all out-params are handled by the crate.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .map_err(|e| format!("no default audio device: {e}"))?;
        let manager: IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())?;
        let sessions = manager.GetSessionEnumerator().map_err(|e| e.to_string())?;
        let count = sessions.GetCount().map_err(|e| e.to_string())?;
        for i in 0..count {
            let Ok(control) = sessions.GetSession(i) else {
                continue;
            };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            if control2.GetProcessId().ok() != Some(pid) {
                continue;
            }
            let simple: ISimpleAudioVolume = control.cast().map_err(|e| e.to_string())?;
            simple
                .SetMasterVolume(volume, std::ptr::null())
                .map_err(|e| e.to_string())?;
            return Ok(true);
        }
        Ok(false)
    }
}

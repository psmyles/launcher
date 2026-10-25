//! Windows integration: the console, the icon, and the taskbar (M7).
//!
//! * **Identity.** Each file's window carries its own AppUserModelID, relaunch command and name
//!   (the window's property store), so its taskbar button groups apart from other files and a
//!   pinned button reopens that file.
//! * **Progress** on the taskbar button (`ITaskbarList3`).
//! * **Notifications:** the taskbar button flashes, and a notification (a notification-area
//!   balloon, which Windows 10 and later show as a toast) says how a run ended, while the window
//!   isn't in front.

use std::cell::RefCell;
use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, PROPERTYKEY};
use windows::Win32::Storage::EnhancedStorage::{
    PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchCommand,
    PKEY_AppUserModel_RelaunchDisplayNameResource, PKEY_AppUserModel_RelaunchIconResource,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{
    ITaskbarList3, Shell_NotifyIconW, TaskbarList, NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO,
    NIIF_WARNING, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, TBPF_ERROR, TBPF_INDETERMINATE,
    TBPF_NOPROGRESS, TBPF_NORMAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FlashWindowEx, LoadIconW, FLASHWINFO, FLASHW_TIMERNOFG, FLASHW_TRAY,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::{Identity, Progress};

/// A release build is a GUI-subsystem exe; started from a terminal it borrows that console so
/// `--help`, `check` and `run` output lands there. Started from Explorer there is no parent
/// console: the call fails and nothing is created. Debug builds are console apps already.
pub fn attach_parent_console() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: plain Win32 call; failure means there is no parent console.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS).is_ok() }
}

/// The exe's own icon resource (id 1, embedded by build.rs).
pub fn window_icon() -> Option<winit::window::Icon> {
    use winit::platform::windows::IconExtWindows;
    winit::window::Icon::from_resource(1, None).ok()
}

fn hwnd(window: &Window) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut c_void)),
        _ => None,
    }
}

/// COM on this (the UI) thread, once. winit may have done it already (OLE for drag and drop);
/// then this is a no-op.
fn com() {
    thread_local!(static DONE: RefCell<bool> = const { RefCell::new(false) });
    DONE.with(|d| {
        if !*d.borrow() {
            // SAFETY: plain COM initialisation of the calling thread.
            let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            *d.borrow_mut() = true;
        }
    });
}

pub fn identify(window: &Window, id: &Identity) {
    let Some(h) = hwnd(window) else { return };
    com();
    let icon = std::env::current_exe()
        .map(|e| format!("{},0", e.display()))
        .unwrap_or_default();
    let props: [(&PROPERTYKEY, &str); 4] = [
        (&PKEY_AppUserModel_ID, &id.id),
        (&PKEY_AppUserModel_RelaunchCommand, &id.relaunch),
        (&PKEY_AppUserModel_RelaunchDisplayNameResource, &id.name),
        (&PKEY_AppUserModel_RelaunchIconResource, &icon),
    ];
    // SAFETY: the store belongs to a live window; each value is a fresh PROPVARIANT.
    let r = unsafe {
        SHGetPropertyStoreForWindow::<IPropertyStore>(h).and_then(|store| {
            for (key, value) in props {
                store.SetValue(key, &PROPVARIANT::from(value))?;
            }
            store.Commit()
        })
    };
    if let Err(e) = r {
        launcher_core::log_warn!("could not set the taskbar identity: {e}");
    }
}

thread_local!(static TASKBAR: RefCell<Option<ITaskbarList3>> = const { RefCell::new(None) });

pub fn show_progress(window: &Window, p: Progress) {
    let Some(h) = hwnd(window) else { return };
    com();
    TASKBAR.with(|t| {
        let mut t = t.borrow_mut();
        if t.is_none() {
            // SAFETY: CoCreateInstance of the shell's taskbar list on a COM thread.
            *t = unsafe {
                CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                    .and_then(|l| l.HrInit().map(|_| l))
                    .ok()
            };
        }
        let Some(list) = t.as_ref() else { return };
        const SCALE: u64 = 1000;
        // SAFETY: calls on a live taskbar list with a live window.
        let _ = unsafe {
            match p {
                Progress::Off => list.SetProgressState(h, TBPF_NOPROGRESS),
                Progress::Busy => list.SetProgressState(h, TBPF_INDETERMINATE),
                Progress::Part(f) => list
                    .SetProgressState(h, TBPF_NORMAL)
                    .and_then(|_| list.SetProgressValue(h, (f * SCALE as f32) as u64, SCALE)),
                Progress::Failed(f) => list
                    .SetProgressState(h, TBPF_ERROR)
                    .and_then(|_| list.SetProgressValue(h, (f * SCALE as f32) as u64, SCALE)),
            }
        };
    });
}

/// Copy `s` into a fixed UTF-16 field, cut to fit, NUL-terminated.
fn fill(dst: &mut [u16], s: &str) {
    let n = dst.len() - 1;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16().take(n).chain(Some(0))) {
        *d = c;
    }
}

fn note_data(h: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: h,
        uID: 1,
        ..Default::default()
    }
}

pub fn notify(window: &Window, title: &str, body: &str, failed: bool) {
    let Some(h) = hwnd(window) else { return };
    let flash = FLASHWINFO {
        cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
        hwnd: h,
        dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
        uCount: 0,
        dwTimeout: 0,
    };
    // SAFETY: plain Win32 calls with a live window and a struct that outlives them.
    unsafe {
        let _ = FlashWindowEx(&flash);
        let _ = Shell_NotifyIconW(NIM_DELETE, &note_data(h));
        let mut d = note_data(h);
        d.uFlags = NIF_ICON | NIF_INFO | NIF_TIP;
        d.hIcon = GetModuleHandleW(PCWSTR::null())
            .and_then(|m| LoadIconW(Some(m.into()), PCWSTR(std::ptr::without_provenance(1))))
            .unwrap_or_default();
        fill(&mut d.szTip, crate::product::NAME);
        fill(&mut d.szInfoTitle, title);
        fill(&mut d.szInfo, body);
        d.dwInfoFlags = if failed { NIIF_WARNING } else { NIIF_INFO };
        if !Shell_NotifyIconW(NIM_ADD, &d).as_bool() {
            launcher_core::log_warn!("could not show a notification");
        }
    }
}

/// The window is in front again (or closing): the notification's icon goes.
pub fn clear_notification(window: &Window) {
    if let Some(h) = hwnd(window) {
        // SAFETY: removing an icon we may have added; a missing one is fine.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &note_data(h));
        }
    }
}

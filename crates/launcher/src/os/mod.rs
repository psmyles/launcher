//! Per-OS integration: the Windows console and icon, the macOS open-document events, and the
//! taskbar / Dock (M7): a file's own taskbar identity, run progress, and a notification when a run
//! ends while the window is in the background.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

use winit::window::Window;

/// Who a window is to the taskbar: one per file, so its button groups apart and a pinned button
/// reopens the file.
pub struct Identity {
    /// The AppUserModelID.
    pub id: String,
    /// The command a pinned button runs (Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub relaunch: String,
    /// The pinned button's name (Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub name: String,
}

/// Run progress on the taskbar button or the Dock tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Off,
    /// Running, with no steps to count.
    Busy,
    /// Running: the share of steps done.
    Part(f32),
    /// Failed, this far in.
    Failed(f32),
}

/// Borrow the launching terminal's console (Windows; a GUI-subsystem process starts with none),
/// so CLI output reaches it. Returns whether there is a console to write to.
pub fn attach_parent_console() -> bool {
    #[cfg(windows)]
    {
        windows::attach_parent_console()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// The window icon, where the OS doesn't take it from the bundle.
pub fn window_icon() -> Option<winit::window::Icon> {
    #[cfg(windows)]
    {
        windows::window_icon()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Give the window its file's taskbar identity (Windows; macOS apps have one identity, the
/// bundle).
pub fn identify(window: &Window, id: &Identity) {
    #[cfg(windows)]
    windows::identify(window, id);
    #[cfg(not(windows))]
    let _ = (window, id);
}

pub fn show_progress(window: &Window, p: Progress) {
    #[cfg(windows)]
    windows::show_progress(window, p);
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        macos::show_progress(p);
    }
}

/// A run ended while the window was in the background.
pub fn notify(window: &Window, title: &str, body: &str, failed: bool) {
    #[cfg(windows)]
    windows::notify(window, title, body, failed);
    #[cfg(target_os = "macos")]
    {
        let _ = (window, failed);
        macos::notify(title, body);
    }
}

/// The window is in front again, or closing.
pub fn clear_notification(window: &Window) {
    #[cfg(windows)]
    windows::clear_notification(window);
    #[cfg(not(windows))]
    let _ = window;
}

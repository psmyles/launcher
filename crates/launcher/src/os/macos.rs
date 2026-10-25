//! macOS integration.
//!
//! **Open-document events (M1 prototype 2).** A document double-clicked in Finder doesn't arrive
//! in argv: LaunchServices sends an Apple Event, which AppKit turns into
//! `application:openURLs:` on the app delegate, before `applicationDidFinishLaunching`. winit's
//! delegate (`WinitApplicationDelegate`) doesn't implement it, so we add the method to that class
//! once the event loop exists (the class is registered by then) and before the loop runs.
//! Documents that arrive before the window exists (Finder started the app to open them) are held
//! for [`take_launch_files`], so the first one opens in place of the home screen and the home
//! screen never shows. Later ones reach the app as an [`AppEvent::OpenFiles`] through the
//! event-loop proxy (N14: each starts a new process).
//!
//! **Progress** is the Dock tile's badge ("40%", "!" when a run failed). **Notifications** bounce
//! the Dock icon and post a notification through `osascript` (an unsigned build can't use the
//! notification framework without an entitlement).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{class, msg_send, sel};
use objc2_foundation::{NSArray, NSString, NSURL};
use winit::event_loop::EventLoopProxy;

use super::Progress;
use crate::app::AppEvent;

/// `NSApp` (the UI runs on the main thread).
fn app() -> *mut AnyObject {
    // SAFETY: `sharedApplication` returns the process's application object.
    unsafe { msg_send![class!(NSApplication), sharedApplication] }
}

pub fn show_progress(p: Progress) {
    let label = match p {
        Progress::Off => None,
        Progress::Busy => Some("…".to_string()),
        Progress::Part(f) => Some(format!("{}%", (f * 100.0).round() as u32)),
        Progress::Failed(_) => Some("!".to_string()),
    };
    let app = app();
    if app.is_null() {
        return;
    }
    // SAFETY: NSApp and its dock tile are live; the label outlives the call.
    unsafe {
        let tile: *mut AnyObject = msg_send![app, dockTile];
        if tile.is_null() {
            return;
        }
        let text = label.map(|l| NSString::from_str(&l));
        let _: () = msg_send![tile, setBadgeLabel: text.as_deref()];
    }
}

fn applescript_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn notify(title: &str, body: &str) {
    let app = app();
    if !app.is_null() {
        // NSInformationalRequest: one bounce.
        // SAFETY: a plain message to NSApp.
        let _: isize = unsafe { msg_send![app, requestUserAttention: 10isize] };
    }
    let script = format!(
        "display notification {} with title {}",
        applescript_quote(body),
        applescript_quote(title)
    );
    if let Err(e) = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .spawn()
    {
        launcher_core::log_warn!("could not post a notification: {e}");
    }
}

static PROXY: OnceLock<EventLoopProxy<AppEvent>> = OnceLock::new();

/// Documents opened before the window exists; `None` once [`take_launch_files`] has taken them.
static LAUNCH_FILES: Mutex<Option<Vec<PathBuf>>> = Mutex::new(Some(Vec::new()));

/// The documents Finder opened the app with (call once, before the window is made). Any later
/// ones come as [`AppEvent::OpenFiles`].
pub fn take_launch_files() -> Vec<PathBuf> {
    LAUNCH_FILES
        .lock()
        .ok()
        .and_then(|mut held| held.take())
        .unwrap_or_default()
}

unsafe extern "C-unwind" fn open_urls(
    _this: *mut AnyObject,
    _cmd: Sel,
    _app: *mut AnyObject,
    urls: *mut NSArray<NSURL>,
) {
    // SAFETY: AppKit passes a live NSArray of NSURL (or nil).
    let Some(urls) = (unsafe { urls.as_ref() }) else {
        return;
    };
    let mut paths = Vec::new();
    for i in 0..urls.count() {
        let url = urls.objectAtIndex(i);
        if let Some(p) = url.path() {
            paths.push(PathBuf::from(p.to_string()));
        }
    }
    if paths.is_empty() {
        return;
    }
    if let Ok(mut held) = LAUNCH_FILES.lock() {
        if let Some(held) = held.as_mut() {
            held.extend(paths);
            return;
        }
    }
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.send_event(AppEvent::OpenFiles(paths));
    }
}

/// Add `application:openURLs:` to winit's app delegate class. Call after the event loop is built
/// and before it runs.
pub fn install_open_documents(proxy: EventLoopProxy<AppEvent>) {
    let _ = PROXY.set(proxy);
    let Some(cls) = AnyClass::get(c"WinitApplicationDelegate") else {
        launcher_core::log_warn!(
            "winit's app delegate class was not found; open-document events are off"
        );
        return;
    };
    // SAFETY: the signature matches `v@:@@` (void; self, _cmd, NSApplication*, NSArray*), and the
    // class lives for the process. Adding a method to a registered class is allowed.
    unsafe {
        let imp: Imp = std::mem::transmute::<
            unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut NSArray<NSURL>),
            Imp,
        >(open_urls);
        let added = objc2::ffi::class_addMethod(
            cls as *const AnyClass as *mut AnyClass,
            sel!(application:openURLs:),
            imp,
            c"v@:@@".as_ptr(),
        );
        if !added.as_bool() {
            launcher_core::log_warn!("could not add application:openURLs: to the app delegate");
        }
    }
}

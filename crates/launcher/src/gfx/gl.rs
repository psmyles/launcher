//! Linux: an OpenGL core context through EGL, on the window's own display (Wayland or X11).
//!
//! EGL needs the display connection winit opens, and the GPU thread starts before the event loop
//! exists: [`display_ready`] hands the display over once it does, and the thread waits for it.
//! The context is made current on the GPU thread with no surface (EGL_KHR_surfaceless_context)
//! for sokol's setup and the painter, released there ([`Device::release_current`]), and made
//! current again on the main thread with the window's surface. There is one GPU: no WARP
//! question, as on macOS.
//!
//! Presenting: on X11 the swap waits for vsync. On Wayland it doesn't (a hidden or minimized
//! window gets no frame callbacks, and a waiting swap would hang the loop), and winit's own
//! pacing (`pre_present_notify`) can't be used either: EGL commits the frame before winit's
//! frame request goes out, so the callback never comes. Instead a present on Wayland waits out
//! the rest of one refresh since the last, so animation runs at the monitor's rate.

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CStr};
use std::ptr::null_mut;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use sokol_gfx::gfx as sg;
use winit::raw_window_handle::{HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use winit::window::Window;

pub const COLOR_FORMAT: sg::PixelFormat = sg::PixelFormat::Rgba8;

type EGLDisplay = *mut c_void;
type EGLConfig = *mut c_void;
type EGLContext = *mut c_void;
type EGLSurface = *mut c_void;
type EGLint = i32;
type EGLAttrib = isize;
type EGLBoolean = c_uint;

const EGL_NONE: EGLint = 0x3038;
const EGL_ALPHA_SIZE: EGLint = 0x3021;
const EGL_BLUE_SIZE: EGLint = 0x3022;
const EGL_GREEN_SIZE: EGLint = 0x3023;
const EGL_RED_SIZE: EGLint = 0x3024;
const EGL_SURFACE_TYPE: EGLint = 0x3033;
const EGL_WINDOW_BIT: EGLint = 0x0004;
const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
const EGL_OPENGL_BIT: EGLint = 0x0008;
const EGL_OPENGL_API: c_uint = 0x30A2;
const EGL_CONTEXT_MAJOR_VERSION: EGLint = 0x3098;
const EGL_CONTEXT_MINOR_VERSION: EGLint = 0x30FB;
const EGL_CONTEXT_OPENGL_PROFILE_MASK: EGLint = 0x30FD;
const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT: EGLint = 0x0001;
const EGL_PLATFORM_X11_KHR: c_uint = 0x31D5;
const EGL_PLATFORM_X11_SCREEN_KHR: EGLAttrib = 0x31D6;
const EGL_PLATFORM_WAYLAND_KHR: c_uint = 0x31D8;
const GL_RENDERER: c_uint = 0x1F01;
const GL_BACK: c_uint = 0x0405;

#[link(name = "EGL")]
extern "C" {
    fn eglGetError() -> EGLint;
    fn eglGetPlatformDisplay(
        platform: c_uint,
        native: *mut c_void,
        attribs: *const EGLAttrib,
    ) -> EGLDisplay;
    fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean;
    fn eglBindAPI(api: c_uint) -> EGLBoolean;
    fn eglChooseConfig(
        dpy: EGLDisplay,
        attribs: *const EGLint,
        configs: *mut EGLConfig,
        size: EGLint,
        count: *mut EGLint,
    ) -> EGLBoolean;
    fn eglGetConfigAttrib(
        dpy: EGLDisplay,
        config: EGLConfig,
        attribute: EGLint,
        value: *mut EGLint,
    ) -> EGLBoolean;
    fn eglCreateContext(
        dpy: EGLDisplay,
        config: EGLConfig,
        share: EGLContext,
        attribs: *const EGLint,
    ) -> EGLContext;
    fn eglCreatePlatformWindowSurface(
        dpy: EGLDisplay,
        config: EGLConfig,
        native_window: *mut c_void,
        attribs: *const EGLAttrib,
    ) -> EGLSurface;
    fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
    fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean;
    fn eglTerminate(dpy: EGLDisplay) -> EGLBoolean;
    fn eglMakeCurrent(
        dpy: EGLDisplay,
        draw: EGLSurface,
        read: EGLSurface,
        ctx: EGLContext,
    ) -> EGLBoolean;
    fn eglSwapInterval(dpy: EGLDisplay, interval: EGLint) -> EGLBoolean;
    fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
}

#[link(name = "wayland-egl")]
extern "C" {
    fn wl_egl_window_create(surface: *mut c_void, width: c_int, height: c_int) -> *mut c_void;
    fn wl_egl_window_resize(window: *mut c_void, width: c_int, height: c_int, dx: c_int, dy: c_int);
    fn wl_egl_window_destroy(window: *mut c_void);
}

#[link(name = "GL")]
extern "C" {
    fn glGetString(name: c_uint) -> *const c_char;
    fn glDrawBuffer(buf: c_uint);
    fn glReadBuffer(buf: c_uint);
}

fn egl_error(what: &str) -> String {
    // SAFETY: plain query of this thread's last EGL error.
    format!("{what} failed (EGL error 0x{:04X})", unsafe {
        eglGetError()
    })
}

/// winit's display connection, handed from the main thread to the GPU thread.
struct NativeDisplay(RawDisplayHandle);

// SAFETY: the display connection lives as long as the event loop, which outlives the GPU thread's
// use of it; EGL is told about it once, on that thread.
unsafe impl Send for NativeDisplay {}

static DISPLAY: Mutex<Option<Result<NativeDisplay, String>>> = Mutex::new(None);
static DISPLAY_READY: Condvar = Condvar::new();

/// The event loop exists: let the GPU thread open EGL on its display (or fail with why not).
pub fn display_ready(handle: Result<RawDisplayHandle, String>) {
    if let Ok(mut d) = DISPLAY.lock() {
        *d = Some(handle.map(NativeDisplay));
        DISPLAY_READY.notify_all();
    }
}

fn wait_for_display() -> Result<RawDisplayHandle, String> {
    let mut d = DISPLAY.lock().map_err(|_| "the display hand-off failed")?;
    loop {
        match d.as_ref() {
            Some(Ok(n)) => return Ok(n.0),
            Some(Err(e)) => return Err(e.clone()),
            None => {}
        }
        d = DISPLAY_READY
            .wait(d)
            .map_err(|_| "the display hand-off failed")?;
    }
}

pub struct Device {
    display: EGLDisplay,
    config: EGLConfig,
    context: EGLContext,
    wayland: bool,
    name: String,
}

// SAFETY: EGL handles are process-wide; the context is current on one thread at a time (released
// by the GPU thread before the join hands the device to the main thread).
unsafe impl Send for Device {}

impl Device {
    /// The window system's GPU, once [`display_ready`] has been called. Leaves the context
    /// current on this thread with no surface.
    pub fn create_default() -> Result<Device, String> {
        let native = wait_for_display()?;
        let (platform, native_display, attribs, wayland) = match native {
            RawDisplayHandle::Wayland(h) => (
                EGL_PLATFORM_WAYLAND_KHR,
                h.display.as_ptr(),
                vec![EGL_NONE as EGLAttrib],
                true,
            ),
            RawDisplayHandle::Xlib(h) => (
                EGL_PLATFORM_X11_KHR,
                h.display.map_or(null_mut(), |d| d.as_ptr()),
                vec![
                    EGL_PLATFORM_X11_SCREEN_KHR,
                    h.screen as EGLAttrib,
                    EGL_NONE as EGLAttrib,
                ],
                false,
            ),
            other => return Err(format!("unsupported display {other:?}")),
        };
        // SAFETY: EGL calls on a live display connection, with NONE-terminated attribute lists
        // and out-pointers to locals.
        unsafe {
            let display = eglGetPlatformDisplay(platform, native_display, attribs.as_ptr());
            if display.is_null() {
                return Err(egl_error("eglGetPlatformDisplay"));
            }
            let (mut major, mut minor) = (0, 0);
            if eglInitialize(display, &mut major, &mut minor) == 0 {
                return Err(egl_error("eglInitialize"));
            }
            if eglBindAPI(EGL_OPENGL_API) == 0 {
                return Err(egl_error("eglBindAPI(OpenGL)"));
            }
            let config = choose_config(display)?;
            // sokol wants 4.1 at least; 4.3 adds storage buffers and texture views it can use.
            let context = [(4, 3), (4, 1)]
                .into_iter()
                .map(|(maj, min)| {
                    let attribs = [
                        EGL_CONTEXT_MAJOR_VERSION,
                        maj,
                        EGL_CONTEXT_MINOR_VERSION,
                        min,
                        EGL_CONTEXT_OPENGL_PROFILE_MASK,
                        EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
                        EGL_NONE,
                    ];
                    eglCreateContext(display, config, null_mut(), attribs.as_ptr())
                })
                .find(|c| !c.is_null())
                .ok_or_else(|| egl_error("creating an OpenGL 4.1 core context"))?;
            if eglMakeCurrent(display, null_mut(), null_mut(), context) == 0 {
                return Err(egl_error("eglMakeCurrent without a surface"));
            }
            let renderer = glGetString(GL_RENDERER);
            let renderer = if renderer.is_null() {
                "unknown".into()
            } else {
                CStr::from_ptr(renderer).to_string_lossy().into_owned()
            };
            Ok(Device {
                display,
                config,
                context,
                wayland,
                name: format!("OpenGL {renderer}"),
            })
        }
    }

    pub fn is_software(&self) -> bool {
        false
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Nothing to flush (the device is never switched).
    pub fn release(&self) {}

    /// Let go of the context on this thread, so another thread can make it current.
    pub fn release_current(&self) {
        // SAFETY: unbinding this thread's context on a live display.
        unsafe { eglMakeCurrent(self.display, null_mut(), null_mut(), null_mut()) };
    }

    pub fn fill_environment(&self, _env: &mut sg::Environment) {}
}

impl Drop for Device {
    /// Close EGL while the display connection is still open: the driver keeps a thread on it.
    fn drop(&mut self) {
        // SAFETY: our own context and display; nothing draws after the device is dropped.
        unsafe {
            eglMakeCurrent(self.display, null_mut(), null_mut(), null_mut());
            eglDestroyContext(self.display, self.context);
            eglTerminate(self.display);
        }
    }
}

/// An RGB8 window config, preferring one without alpha: on X11 it matches the window's 24-bit
/// visual, and on Wayland an alpha channel would make the window translucent.
unsafe fn choose_config(display: EGLDisplay) -> Result<EGLConfig, String> {
    let attribs = [
        EGL_SURFACE_TYPE,
        EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_BIT,
        EGL_RED_SIZE,
        8,
        EGL_GREEN_SIZE,
        8,
        EGL_BLUE_SIZE,
        8,
        EGL_NONE,
    ];
    let mut configs = [null_mut(); 64];
    let mut count = 0;
    if eglChooseConfig(
        display,
        attribs.as_ptr(),
        configs.as_mut_ptr(),
        configs.len() as EGLint,
        &mut count,
    ) == 0
        || count == 0
    {
        return Err(egl_error("eglChooseConfig (RGB8, OpenGL, window)"));
    }
    let configs = &configs[..count as usize];
    let alpha = |c: EGLConfig| {
        let mut a = 0;
        eglGetConfigAttrib(display, c, EGL_ALPHA_SIZE, &mut a);
        a
    };
    Ok(configs
        .iter()
        .copied()
        .find(|&c| alpha(c) == 0)
        .unwrap_or(configs[0]))
}

pub struct Surface {
    display: EGLDisplay,
    surface: EGLSurface,
    /// The wl_egl_window (Wayland), which sets the buffer size; null on X11.
    wl_window: *mut c_void,
    size: [u32; 2],
    /// Wayland: one monitor refresh, the least time between presents.
    min_interval: Duration,
    last_present: Option<Instant>,
}

pub struct Frame;

impl Frame {
    pub fn fill_swapchain(&self, sc: &mut sg::Swapchain) {
        sc.gl.framebuffer = 0;
    }
}

impl Surface {
    pub fn new(device: &Device, window: &Window, size_px: [u32; 2]) -> Result<Surface, String> {
        let handle = window
            .window_handle()
            .map_err(|e| format!("no window handle: {e}"))?
            .as_raw();
        let w = size_px[0] as c_int;
        let h = size_px[1] as c_int;
        // SAFETY: the native window is winit's and outlives the surface (both live as long as the
        // app); EGL reads the X11 window id through the pointer during the call only.
        unsafe {
            let mut xid: c_ulong = match handle {
                RawWindowHandle::Xlib(wh) => wh.window,
                RawWindowHandle::Xcb(wh) => wh.window.get() as c_ulong,
                _ => 0,
            };
            let (native, wl_window) = match handle {
                RawWindowHandle::Wayland(wh) => {
                    let wl = wl_egl_window_create(wh.surface.as_ptr(), w, h);
                    if wl.is_null() {
                        return Err("wl_egl_window_create failed".into());
                    }
                    (wl, wl)
                }
                RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) => {
                    (&mut xid as *mut c_ulong as *mut c_void, null_mut())
                }
                other => return Err(format!("unsupported window {other:?}")),
            };
            let attribs = [EGL_NONE as EGLAttrib];
            let surface = eglCreatePlatformWindowSurface(
                device.display,
                device.config,
                native,
                attribs.as_ptr(),
            );
            if surface.is_null() {
                if !wl_window.is_null() {
                    wl_egl_window_destroy(wl_window);
                }
                return Err(egl_error("eglCreatePlatformWindowSurface"));
            }
            let refresh_mhz = window
                .current_monitor()
                .and_then(|m| m.refresh_rate_millihertz())
                .filter(|&r| r > 0)
                .unwrap_or(60_000);
            let s = Surface {
                display: device.display,
                surface,
                wl_window,
                size: size_px,
                min_interval: if device.wayland {
                    Duration::from_micros(1_000_000_000 / refresh_mhz as u64)
                } else {
                    Duration::ZERO
                },
                last_present: None,
            };
            if eglMakeCurrent(device.display, surface, surface, device.context) == 0 {
                return Err(egl_error("eglMakeCurrent with the window"));
            }
            // A context first made current without a surface starts drawing to GL_NONE, and
            // keeps it when a window is bound later: point it at the back buffer.
            glDrawBuffer(GL_BACK);
            glReadBuffer(GL_BACK);
            eglSwapInterval(device.display, if device.wayland { 0 } else { 1 });
            Ok(s)
        }
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn resize(&mut self, size_px: [u32; 2]) {
        if size_px[0] == 0 || size_px[1] == 0 || size_px == self.size {
            return;
        }
        self.size = size_px;
        if !self.wl_window.is_null() {
            // SAFETY: our live wl_egl_window; takes effect at the next swap.
            unsafe {
                wl_egl_window_resize(
                    self.wl_window,
                    size_px[0] as c_int,
                    size_px[1] as c_int,
                    0,
                    0,
                )
            };
        }
    }

    /// The default framebuffer is always there.
    pub fn acquire(&mut self) -> Option<Frame> {
        Some(Frame)
    }

    pub fn present(&mut self, _frame: Frame) {
        if let Some(last) = self.last_present {
            let next = last + self.min_interval;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            }
        }
        self.last_present = Some(Instant::now());
        // SAFETY: our live surface, current on this thread.
        if unsafe { eglSwapBuffers(self.display, self.surface) } == 0 {
            launcher_core::log_warn!("{}", egl_error("eglSwapBuffers"));
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: our own surface and wl_egl_window; EGL defers destroying a current surface.
        unsafe {
            eglDestroySurface(self.display, self.surface);
            if !self.wl_window.is_null() {
                wl_egl_window_destroy(self.wl_window);
            }
        }
    }
}

/// Nothing to wrap on GL.
pub fn pool<R>(f: impl FnOnce() -> R) -> R {
    f()
}

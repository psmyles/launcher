//! macOS: the Metal device sokol_gfx runs on and the `CAMetalLayer` it draws into (M0's variant B).
//!
//! The layer is attached to winit's NSView with raw-window-metal (which keeps its bounds and scale
//! in sync). sokol's Metal swapchain format is BGRA8; sokol presents the drawable itself at the end
//! of the swapchain pass, so `present` has nothing left to do. macOS has no WARP question: the
//! system GPU is fast to open and cheap to draw with.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CGSize;
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use sokol_gfx::gfx as sg;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

pub const COLOR_FORMAT: sg::PixelFormat = sg::PixelFormat::Bgra8;

pub struct Device {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    name: String,
}

// SAFETY: MTLDevice is thread-safe; it is created on the GPU thread and handed over by join.
unsafe impl Send for Device {}

impl Device {
    /// The system GPU.
    pub fn create_default() -> Result<Device, String> {
        let device = MTLCreateSystemDefaultDevice().ok_or("this Mac has no Metal device")?;
        let name = format!("Metal {}", device.name());
        Ok(Device { device, name })
    }

    pub fn is_software(&self) -> bool {
        false
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Nothing to flush on Metal (the device is never switched).
    pub fn release(&self) {}

    pub fn fill_environment(&self, env: &mut sg::Environment) {
        env.metal.device = Retained::as_ptr(&self.device) as *const c_void;
    }
}

pub struct Surface {
    /// Keeps the layer attached and observed.
    _attachment: raw_window_metal::Layer,
    layer: Retained<CAMetalLayer>,
    size: [u32; 2],
}

pub struct Frame {
    drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
}

impl Frame {
    pub fn fill_swapchain(&self, sc: &mut sg::Swapchain) {
        sc.metal.current_drawable = Retained::as_ptr(&self.drawable) as *const c_void;
    }
}

impl Surface {
    pub fn new(device: &Device, window: &Window, size_px: [u32; 2]) -> Result<Surface, String> {
        let handle = window
            .window_handle()
            .map_err(|e| format!("no window handle: {e}"))?
            .as_raw();
        let RawWindowHandle::AppKit(h) = handle else {
            return Err("the window is not an AppKit window".into());
        };
        // SAFETY: the NSView is winit's and outlives the surface (both live as long as the app).
        let attachment = unsafe { raw_window_metal::Layer::from_ns_view(h.ns_view) };
        // SAFETY: raw-window-metal hands back a live CAMetalLayer; we take our own reference.
        let layer: Retained<CAMetalLayer> =
            unsafe { Retained::retain(attachment.as_ptr().as_ptr().cast()) }
                .ok_or("no CAMetalLayer")?;
        layer.setDevice(Some(&device.device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        layer.setFramebufferOnly(true);
        layer.setOpaque(true);
        layer.setContentsScale(window.scale_factor());
        let mut s = Surface {
            _attachment: attachment,
            layer,
            size: [0, 0],
        };
        s.resize(size_px);
        Ok(s)
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn resize(&mut self, size_px: [u32; 2]) {
        if size_px[0] == 0 || size_px[1] == 0 || size_px == self.size {
            return;
        }
        self.size = size_px;
        self.layer.setDrawableSize(CGSize {
            width: size_px[0] as f64,
            height: size_px[1] as f64,
        });
    }

    /// At most one drawable is requested per frame; `nextDrawable` can stall for up to 1 s when
    /// earlier drawables were never displayed, so the shell presents at most once while hidden.
    pub fn acquire(&mut self) -> Option<Frame> {
        self.layer.nextDrawable().map(|drawable| Frame { drawable })
    }

    /// sokol already scheduled the present at the end of the swapchain pass.
    pub fn present(&mut self, frame: Frame) {
        drop(frame);
    }
}

/// Run `f` in an autorelease pool: sokol is compiled without ARC and autoreleases its per-frame
/// Metal objects.
pub fn pool<R>(f: impl FnOnce() -> R) -> R {
    objc2::rc::autoreleasepool(|_| f())
}

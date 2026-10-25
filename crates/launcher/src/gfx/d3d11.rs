//! Windows: the D3D11 device sokol_gfx runs on and the DXGI flip-model swapchain it draws into
//! (ported from master's `gfx/d3d11.rs`). The one place that names D3D11 or DXGI.
//!
//! The backbuffer is plain `R8G8B8A8_UNORM` (flip model disallows `*_SRGB` swapchain formats);
//! egui's colours are already sRGB. Devices are created off the main thread and used from it after
//! a join or a channel hand-over: D3D11 devices are free-threaded, and only the main thread touches
//! the immediate context.

use std::ffi::c_void;

use sokol_gfx::gfx as sg;
use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1, DXGI_ADAPTER_DESC, DXGI_PRESENT,
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// Plain `RGBA8`: flip-model chains disallow `*_SRGB` swapchain formats.
pub const COLOR_FORMAT: sg::PixelFormat = sg::PixelFormat::Rgba8;

/// The D3D11 device and its immediate context.
pub struct Device {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    software: bool,
    name: String,
}

// SAFETY: a D3D11 device is free-threaded by specification. The immediate context is not, and only
// the main thread uses it, after the creating thread hands both over (join or channel).
unsafe impl Send for Device {}

/// The software rasterizer's PCI ids ("Microsoft Basic Render Driver").
const SOFTWARE_VENDOR: u32 = 0x1414;
const SOFTWARE_DEVICE: u32 = 0x008c;

impl Device {
    /// WARP: ~1 ms to create, so the first frame never waits on a vendor driver (N16, M0).
    pub fn create_warp() -> Result<Device, String> {
        Self::create(D3D_DRIVER_TYPE_WARP)
    }

    /// The default hardware adapter. Loading its driver takes ~120 ms, so this runs off the
    /// startup path. On a machine with no GPU driver this is the Basic Render Driver, which
    /// [`Device::is_software`] reports.
    pub fn create_hardware() -> Result<Device, String> {
        Self::create(D3D_DRIVER_TYPE_HARDWARE)
    }

    fn create(driver: D3D_DRIVER_TYPE) -> Result<Device, String> {
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        // SAFETY: every out-pointer is a live local; the feature-level slice outlives the call.
        unsafe {
            D3D11CreateDevice(
                None,
                driver,
                Default::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|e| format!("D3D11CreateDevice failed: {e}"))?;
        let (Some(device), Some(context)) = (device, context) else {
            return Err("D3D11CreateDevice returned no device".into());
        };
        let desc = adapter_desc(&device);
        let software = driver == D3D_DRIVER_TYPE_WARP
            || desc
                .as_ref()
                .is_some_and(|d| d.VendorId == SOFTWARE_VENDOR && d.DeviceId == SOFTWARE_DEVICE);
        let adapter = desc
            .map(|d| {
                let n = d.Description.iter().position(|&c| c == 0).unwrap_or(128);
                String::from_utf16_lossy(&d.Description[..n])
            })
            .unwrap_or_default();
        let name = if driver == D3D_DRIVER_TYPE_WARP {
            "D3D11 WARP".to_string()
        } else {
            format!("D3D11 {adapter}")
        };
        Ok(Device {
            device,
            context,
            software,
            name,
        })
    }

    /// Rasterizes on the CPU (WARP or the Basic Render Driver).
    pub fn is_software(&self) -> bool {
        self.software
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Drop every reference the immediate context still holds and run deferred destruction, so a
    /// released swapchain really is gone. A window accepts a new flip-model swapchain only then
    /// ("Access is denied" otherwise).
    pub fn release(&self) {
        // SAFETY: the immediate context, used from the main thread only.
        unsafe {
            self.context.ClearState();
            self.context.Flush();
        }
    }

    /// Point `env` at this device and its immediate context, so `sg_setup` runs on them.
    pub fn fill_environment(&self, env: &mut sg::Environment) {
        env.d3d11.device = self.device.as_raw() as *const c_void;
        env.d3d11.device_context = self.context.as_raw() as *const c_void;
    }
}

fn adapter_desc(device: &ID3D11Device) -> Option<DXGI_ADAPTER_DESC> {
    // SAFETY: COM traversal device → adapter on a live device.
    unsafe {
        let dxgi: IDXGIDevice = device.cast().ok()?;
        dxgi.GetAdapter().ok()?.GetDesc().ok()
    }
}

/// One window's flip-model swapchain and the render-target view of its current backbuffer.
pub struct Surface {
    device: ID3D11Device,
    swapchain: IDXGISwapChain1,
    rtv: Option<ID3D11RenderTargetView>,
    size: [u32; 2],
}

pub struct Frame {
    render_view: *const c_void,
}

impl Frame {
    pub fn fill_swapchain(&self, sc: &mut sg::Swapchain) {
        sc.d3d11.render_view = self.render_view;
    }
}

impl Surface {
    /// A two-buffer `FLIP_DISCARD` swapchain on `window`'s client area, vsync-paced.
    pub fn new(device: &Device, window: &Window, size_px: [u32; 2]) -> Result<Surface, String> {
        let hwnd = match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => HWND(h.hwnd.get() as *mut c_void),
            _ => return Err("the window has no Win32 handle".into()),
        };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: size_px[0].max(1),
            Height: size_px[1].max(1),
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            ..Default::default()
        };
        // SAFETY: COM traversal device → adapter → factory on a live device; `hwnd` is live.
        let swapchain = unsafe {
            let dxgi_device: IDXGIDevice = device.device.cast().map_err(|e| e.to_string())?;
            let adapter: IDXGIAdapter = dxgi_device.GetAdapter().map_err(|e| e.to_string())?;
            let factory: IDXGIFactory2 = adapter.GetParent().map_err(|e| e.to_string())?;
            factory
                .CreateSwapChainForHwnd(&device.device, hwnd, &desc, None, None)
                .map_err(|e| format!("CreateSwapChainForHwnd failed: {e}"))?
        };
        Ok(Surface {
            device: device.device.clone(),
            swapchain,
            rtv: None,
            size: [size_px[0].max(1), size_px[1].max(1)],
        })
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Drop the view and resize the backbuffers. A zero dimension (minimized) is skipped: DXGI
    /// refuses it, and nothing is drawn while minimized anyway.
    pub fn resize(&mut self, size_px: [u32; 2]) {
        if size_px[0] == 0 || size_px[1] == 0 || size_px == self.size {
            return;
        }
        self.size = size_px;
        self.rtv = None;
        // SAFETY: the view, the only outstanding backbuffer reference, was released above.
        if let Err(e) = unsafe {
            self.swapchain.ResizeBuffers(
                0,
                size_px[0],
                size_px[1],
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        } {
            launcher_core::log_warn!("swapchain ResizeBuffers failed: {e}");
        }
    }

    /// This frame's render target, or `None` (logged) if the device refused it.
    pub fn acquire(&mut self) -> Option<Frame> {
        if self.rtv.is_none() {
            // SAFETY: buffer 0 of a live flip-model swapchain; the out-pointer is a live local.
            unsafe {
                let back: ID3D11Texture2D = match self.swapchain.GetBuffer(0) {
                    Ok(b) => b,
                    Err(e) => {
                        launcher_core::log_warn!("swapchain GetBuffer failed: {e}");
                        return None;
                    }
                };
                let mut rtv: Option<ID3D11RenderTargetView> = None;
                if let Err(e) = self
                    .device
                    .CreateRenderTargetView(&back, None, Some(&mut rtv))
                {
                    launcher_core::log_warn!("CreateRenderTargetView failed: {e}");
                    return None;
                }
                self.rtv = rtv;
            }
        }
        self.rtv.as_ref().map(|v| Frame {
            render_view: v.as_raw() as *const c_void,
        })
    }

    /// Present the committed frame, vsync-paced (sync interval 1).
    pub fn present(&mut self, _frame: Frame) {
        // SAFETY: a live swapchain; sync interval 1 with no flags is always valid.
        let _ = unsafe { self.swapchain.Present(1, DXGI_PRESENT(0)) };
    }
}

/// Windows needs no autorelease pool.
pub fn pool<R>(f: impl FnOnce() -> R) -> R {
    f()
}

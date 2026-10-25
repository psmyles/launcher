//! Drawing egui with sokol_gfx (N16): the device, the window surface, and the painter.
//!
//! **Device policy (N16, Windows):** the first frame is drawn on WARP, which opens in ~1 ms; a
//! hardware device costs ~120 ms of driver loading and would miss the startup budget. Once the
//! window is shown, a background thread opens the hardware device and the shell switches to it
//! ([`Gpu::switch_device`]): WARP costs ~1.4 cores while the console streams, the GPU ~2.5% of
//! one. With no real GPU (the Basic Render Driver) it stays on WARP. `LAUNCHER_GPU=warp|hardware`
//! pins one device. macOS uses the system GPU throughout.
//!
//! sokol_gfx is a process-wide singleton with no thread affinity: set up on the bring-up thread,
//! used from the main thread after the join.

pub mod painter;

#[cfg(windows)]
#[path = "d3d11.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "metal.rs"]
mod platform;

pub use platform::Device;
use platform::{pool, Surface, COLOR_FORMAT};

use painter::Painter;
use sokol_gfx::gfx as sg;
use winit::window::Window;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// WARP for the first frame, then hardware (Windows); the system GPU (macOS).
    Auto,
    Warp,
    Hardware,
}

impl Policy {
    pub fn from_env() -> Policy {
        match std::env::var("LAUNCHER_GPU").as_deref() {
            Ok("warp") => Policy::Warp,
            Ok("hardware") => Policy::Hardware,
            _ => Policy::Auto,
        }
    }
}

/// The first device to draw with, per policy.
fn first_device(policy: Policy) -> Result<Device, String> {
    #[cfg(windows)]
    {
        match policy {
            Policy::Hardware => Device::create_hardware().or_else(|_| Device::create_warp()),
            Policy::Auto | Policy::Warp => {
                Device::create_warp().or_else(|_| Device::create_hardware())
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = policy;
        Device::create_default()
    }
}

/// The device to switch to after the first frame, if the policy wants one and there is a real
/// GPU. Blocking (driver load): call it on a background thread.
pub fn upgrade_device() -> Option<Device> {
    #[cfg(windows)]
    {
        match Device::create_hardware() {
            Ok(d) if !d.is_software() => Some(d),
            Ok(_) => {
                launcher_core::log_info!("no hardware GPU; staying on WARP");
                None
            }
            Err(e) => {
                launcher_core::log_info!("no hardware GPU ({e}); staying on WARP");
                None
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        None
    }
}

fn setup_sokol(device: &Device) -> Result<(), String> {
    let mut desc = sg::Desc::new();
    desc.environment.defaults = sg::EnvironmentDefaults {
        color_format: COLOR_FORMAT,
        depth_format: sg::PixelFormat::None,
        sample_count: 1,
    };
    device.fill_environment(&mut desc.environment);
    desc.logger.func = Some(sokol_gfx::log::slog_func);
    sg::setup(&desc);
    if sg::isvalid() {
        Ok(())
    } else {
        Err("sokol_gfx could not be set up on the GPU device".into())
    }
}

/// The GPU: device, sokol, painter, and (once there is a window) its surface.
pub struct Gpu {
    device: Device,
    painter: Painter,
    surface: Option<Surface>,
    policy: Policy,
}

// SAFETY: the painter holds only sokol handles (plain integers) and CPU texture copies; sokol has
// no thread affinity. The whole `Gpu` moves to the main thread once, through a join.
unsafe impl Send for Gpu {}

pub struct PaintJob<'a> {
    pub textures_delta: &'a egui::TexturesDelta,
    pub primitives: &'a [egui::ClippedPrimitive],
    pub pixels_per_point: f32,
    pub size_px: [u32; 2],
    /// sRGB, 0–1: the window background, also the frame shown before any content.
    pub clear: [f32; 3],
}

impl Gpu {
    /// Bring the GPU up on a thread started first thing in `main`: the device and the painter need
    /// no window, so they run alongside event-loop and window creation.
    pub fn start(policy: Policy) -> std::thread::JoinHandle<Result<Gpu, String>> {
        std::thread::Builder::new()
            .name("gpu-init".into())
            .spawn(move || Self::bring_up(policy))
            .expect("the GPU bring-up thread must start")
    }

    fn bring_up(policy: Policy) -> Result<Gpu, String> {
        pool(|| {
            let device = first_device(policy)?;
            crate::timing::report(&format!("gpu device ({})", device.name()));
            setup_sokol(&device)?;
            let painter = Painter::new(COLOR_FORMAT);
            crate::timing::report("sokol + painter");
            Ok(Gpu {
                device,
                painter,
                surface: None,
                policy,
            })
        })
    }

    /// Attach to the (hidden) window.
    pub fn attach(&mut self, window: &Window) -> Result<(), String> {
        let size = window.inner_size();
        let surface = Surface::new(
            &self.device,
            window,
            [size.width.max(1), size.height.max(1)],
        )?;
        self.surface = Some(surface);
        Ok(())
    }

    pub fn device_name(&self) -> &str {
        self.device.name()
    }

    /// Whether to open a hardware device in the background after the first frame.
    pub fn wants_upgrade(&self) -> bool {
        self.policy == Policy::Auto && self.device.is_software()
    }

    /// The largest texture side the device allows (egui sizes its font atlas from it).
    pub fn max_texture_side(&self) -> usize {
        (sg::query_limits().max_image_size_2d.max(2048)) as usize
    }

    pub fn resize(&mut self, size_px: [u32; 2]) {
        if let Some(s) = self.surface.as_mut() {
            s.resize(size_px);
        }
    }

    /// Upload textures, draw and present one frame. False when nothing was presented (minimized,
    /// or the device refused the backbuffer); textures are applied either way.
    pub fn paint(&mut self, job: PaintJob<'_>) -> bool {
        pool(|| {
            self.painter.set_textures(job.textures_delta);
            let Some(surface) = self.surface.as_mut() else {
                self.painter.free_textures(job.textures_delta);
                return false;
            };
            if job.size_px[0] == 0 || job.size_px[1] == 0 {
                self.painter.free_textures(job.textures_delta);
                return false;
            }
            surface.resize(job.size_px);
            let size = surface.size();
            self.painter
                .prepare(job.primitives, job.pixels_per_point, size);
            let Some(frame) = surface.acquire() else {
                self.painter.free_textures(job.textures_delta);
                return false;
            };
            let mut pass = sg::Pass::new();
            pass.action.colors[0].load_action = sg::LoadAction::Clear;
            pass.action.colors[0].clear_value = sg::Color {
                r: job.clear[0],
                g: job.clear[1],
                b: job.clear[2],
                a: 1.0,
            };
            pass.swapchain.width = size[0] as i32;
            pass.swapchain.height = size[1] as i32;
            pass.swapchain.sample_count = 1;
            pass.swapchain.color_format = COLOR_FORMAT;
            pass.swapchain.depth_format = sg::PixelFormat::None;
            frame.fill_swapchain(&mut pass.swapchain);
            sg::begin_pass(&pass);
            // The pass must close whatever happens: a panic escaping here would leave sokol
            // mid-pass and assert on the next frame.
            let drew = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.painter.draw(job.pixels_per_point, size)
            }));
            if drew.is_err() {
                launcher_core::log_error!("recovered from a panic while drawing");
            }
            sg::end_pass();
            sg::commit();
            surface.present(frame);
            self.painter.free_textures(job.textures_delta);
            true
        })
    }

    /// Move drawing to `device` (N16's WARP → hardware switch). The surface is recreated on the
    /// new device (a window holds one flip-model swapchain at a time), sokol is set up again, and
    /// the painter rebuilds its GPU objects from its CPU copies. On failure the old device stays.
    pub fn switch_device(&mut self, device: Device, window: &Window) -> Result<(), String> {
        pool(|| {
            let size = window.inner_size();
            let size = [size.width.max(1), size.height.max(1)];
            // Let go of everything on the old device: sokol's objects, the swapchain, and the
            // context's deferred references to them.
            sg::shutdown();
            self.surface = None;
            self.device.release();
            let result = Surface::new(&device, window, size).and_then(|surface| {
                setup_sokol(&device).map(|()| surface).inspect_err(|_| {
                    sg::shutdown();
                    device.release();
                })
            });
            match result {
                Ok(surface) => {
                    self.device = device;
                    self.surface = Some(surface);
                    self.painter.rebuild();
                    Ok(())
                }
                Err(e) => {
                    // Back to the old device, which worked a moment ago.
                    drop(device);
                    setup_sokol(&self.device)?;
                    self.painter.rebuild();
                    self.surface = Some(Surface::new(&self.device, window, size)?);
                    Err(e)
                }
            }
        })
    }
}

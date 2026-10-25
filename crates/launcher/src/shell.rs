//! The shell: one window, one winit event loop, the GPU and egui, and the redraw pump.
//!
//! Rules (master's `shell.rs`, M0):
//! * **Every handler runs behind a panic firewall** (N15): a panic must not unwind into winit,
//!   and on macOS a panic in a handler would abort (the callback comes from Objective-C).
//! * **An idle window draws nothing.** Frames follow input, a cross-thread repaint request, or
//!   egui's own repaint deadline, which the loop sleeps on (`ControlFlow::WaitUntil`).
//! * **No white flash, no half-laid-out window:** the window is created hidden; the first frame is
//!   laid out (egui passes that don't present, as many as the layout needs), drawn and presented
//!   synchronously while hidden (a hidden window gets no `RedrawRequested`, LESSONS L2.2), then
//!   the window is shown.
//! * **Modal dialogs never run inside a handler:** a fatal error is recorded and shown by `main`
//!   after the loop returns.
//! * egui-winit answers `RedrawRequested` itself with `repaint: true`; requesting another redraw
//!   for it would redraw forever (M0). It also says `repaint` for `Moved`, which is ignored:
//!   a frame per move makes dragging the window lag.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::FontDefinitions;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::{Window, WindowId};

use launcher_core::state::Rect;

use crate::app::{App, AppEvent, WindowRequest};
use crate::gfx::{self, Gpu, PaintJob};

/// Layout passes allowed before the first frame is drawn anyway.
const MAX_SETTLE_PASSES: usize = 3;

pub struct Shell {
    app: App,
    ctx: egui::Context,
    gpu_pending: Option<JoinHandle<Result<Gpu, String>>>,
    fonts_pending: Option<JoinHandle<FontDefinitions>>,
    gpu: Option<Gpu>,
    /// The theme, once the context exists (reloaded from its file in dev).
    theme: Option<crate::ui::theme::Live>,
    win: Option<Win>,
    proxy: EventLoopProxy<AppEvent>,
    fatal: Option<String>,
    started: bool,
    /// The content asked to close the window (an action with `after="exit"`).
    exit_requested: bool,
    /// The window is in front (notifications only show when it isn't).
    focused: bool,
    /// The taskbar identity and progress last given to the OS.
    identity: Option<String>,
    progress: crate::os::Progress,
}

/// Everything that exists once the window does.
struct Win {
    window: Arc<Window>,
    state: egui_winit::State,
    shown: bool,
    /// The next frame egui asked for.
    repaint_at: Option<Instant>,
    /// The last normal (not maximized or minimized) rect, for the state file.
    normal_rect: Option<Rect>,
}

impl Shell {
    pub fn new(
        app: App,
        gpu_pending: JoinHandle<Result<Gpu, String>>,
        fonts_pending: JoinHandle<FontDefinitions>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Shell {
        Shell {
            app,
            ctx: egui::Context::default(),
            gpu_pending: Some(gpu_pending),
            fonts_pending: Some(fonts_pending),
            gpu: None,
            theme: None,
            win: None,
            proxy,
            fatal: None,
            started: false,
            exit_requested: false,
            focused: true,
            identity: None,
            progress: crate::os::Progress::Off,
        }
    }

    pub fn take_fatal(&mut self) -> Option<String> {
        self.fatal.take()
    }

    /// After the loop returns: flush what must outlive the process.
    pub fn shutdown(&mut self) {
        self.sample_placement();
        self.app.on_exit();
        if let Some(w) = &self.win {
            crate::os::show_progress(&w.window, crate::os::Progress::Off);
            crate::os::clear_notification(&w.window);
        }
    }

    fn create_window(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let (rect, maximized) = self.app.placement();
        let (lw, lh) = self.app.default_size();
        let mut attrs = Window::default_attributes()
            .with_title(self.app.title())
            .with_visible(false)
            .with_resizable(true)
            .with_min_inner_size(LogicalSize::new(320.0, 200.0))
            .with_window_icon(crate::os::window_icon());
        attrs = match rect {
            Some(r) => attrs
                .with_inner_size(PhysicalSize::new(r.width, r.height))
                .with_position(PhysicalPosition::new(r.x, r.y)),
            None => attrs.with_inner_size(LogicalSize::new(lw, lh)),
        };
        let window = Arc::new(
            el.create_window(attrs)
                .map_err(|e| format!("could not create the window: {e}"))?,
        );
        if rect.is_none() || !title_bar_visible(&window) {
            center_on_monitor(&window);
        }
        if maximized {
            window.set_maximized(true);
        }
        crate::timing::report("window created");

        let mut gpu = self
            .gpu_pending
            .take()
            .ok_or("the GPU failed to initialize earlier")?
            .join()
            .map_err(|_| "the GPU bring-up thread panicked".to_string())??;
        crate::timing::report("gpu joined");
        gpu.attach(&window)
            .map_err(|e| format!("the GPU could not draw into the window: {e}"))?;
        crate::timing::report("surface ready");

        // The theme sets the fonts: it may tweak them.
        let fonts = self.fonts_pending.take().and_then(|h| h.join().ok());
        self.theme = Some(crate::ui::theme::Live::start(&self.ctx, fonts));
        crate::timing::report("theme");
        let proxy = self.proxy.clone();
        self.ctx.set_request_repaint_callback(move |info| {
            let _ = proxy.send_event(AppEvent::Repaint(info.delay));
        });
        let state = egui_winit::State::new(
            self.ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(gpu.max_texture_side()),
        );
        launcher_core::log_info!("drawing with {}", gpu.device_name());
        self.gpu = Some(gpu);
        self.app.begin(&self.ctx, self.proxy.clone());
        window.set_title(&self.app.title());
        window.set_resizable(self.app.resizable());
        self.sync_os(&window);
        self.win = Some(Win {
            window: window.clone(),
            state,
            shown: false,
            repaint_at: None,
            normal_rect: rect,
        });

        // The first frame, while hidden: lay out, draw, present, then show.
        // Another pass only when the content asks for a different window size (auto-fit).
        let mut textures = egui::TexturesDelta::default();
        let mut out = None;
        for _ in 0..MAX_SETTLE_PASSES {
            let Some(o) = self.run_pass() else { break };
            if let Some(prev) = out.replace(o) {
                textures.append(prev.textures_delta);
            }
            if !self.apply_fit_size() {
                break;
            }
        }
        crate::timing::report("first layout");
        let presented = match out {
            Some(mut o) => {
                textures.append(std::mem::take(&mut o.textures_delta));
                o.textures_delta = textures;
                self.paint(o)
            }
            None => false,
        };
        window.set_visible(true);
        if let Some(w) = self.win.as_mut() {
            w.shown = true;
        }
        if presented {
            crate::timing::first_frame_shown();
        } else {
            launcher_core::log_warn!("the first frame could not be presented");
            window.request_redraw();
        }

        if self.gpu.as_ref().is_some_and(Gpu::wants_upgrade) {
            let proxy = self.proxy.clone();
            let _ = std::thread::Builder::new()
                .name("gpu-upgrade".into())
                .spawn(move || {
                    if let Some(device) = gfx::upgrade_device() {
                        let _ = proxy.send_event(AppEvent::Gpu(device));
                    }
                });
        }
        Ok(())
    }

    /// One egui pass (layout only).
    fn run_pass(&mut self) -> Option<egui::FullOutput> {
        let w = self.win.as_mut()?;
        if let Some(t) = self.theme.as_mut() {
            t.poll(&self.ctx);
        }
        let raw = w.state.take_egui_input(&w.window);
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| {
            app.ui(ui);
            crate::ui::theme::show_problems(ui.ctx());
        });
        let platform = std::mem::take(&mut out.platform_output);
        w.state.handle_platform_output(&w.window, platform);
        Some(out)
    }

    /// Tessellate, draw and present a pass's output; schedule the next frame egui asks for.
    fn paint(&mut self, mut out: egui::FullOutput) -> bool {
        let (Some(w), Some(gpu)) = (self.win.as_mut(), self.gpu.as_mut()) else {
            out.textures_delta.clear();
            return false;
        };
        let primitives = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        let size = w.window.inner_size();
        let bg = self.ctx.style_of(self.ctx.theme()).visuals.panel_fill;
        let presented = gpu.paint(PaintJob {
            textures_delta: &out.textures_delta,
            primitives: &primitives,
            pixels_per_point: out.pixels_per_point,
            size_px: [size.width, size.height],
            clear: [
                bg.r() as f32 / 255.0,
                bg.g() as f32 / 255.0,
                bg.b() as f32 / 255.0,
            ],
        });
        // Applied (or, with no surface, dropped); egui asserts a delta is cleared once handled.
        out.textures_delta.clear();
        let delay = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .unwrap_or(Duration::MAX);
        w.repaint_at = (delay < Duration::from_secs(3600)).then(|| Instant::now() + delay);
        presented
    }

    /// Resize the window to the size its content asks for (logical px), if that differs. Returns
    /// whether it changed, so the caller lays out again at the new size.
    fn apply_fit_size(&mut self) -> bool {
        let Some(w) = self.win.as_ref() else {
            return false;
        };
        let Some((lw, lh)) = self.app.fit_size() else {
            return false;
        };
        if w.window.is_maximized() {
            return false;
        }
        let want =
            LogicalSize::new(lw as f64, lh as f64).to_physical::<u32>(w.window.scale_factor());
        let have = w.window.inner_size();
        if want.width.abs_diff(have.width) <= 1 && want.height.abs_diff(have.height) <= 1 {
            return false;
        }
        let _ = w.window.request_inner_size(want);
        true
    }

    fn frame(&mut self) {
        if let Some(out) = self.run_pass() {
            let resized = self.apply_fit_size();
            self.paint(out);
            if resized {
                if let Some(w) = &self.win {
                    w.window.request_redraw();
                }
            }
        }
        if let Some(w) = self.win.as_ref().map(|w| w.window.clone()) {
            self.sync_os(&w);
        }
        let title = self.app.title();
        let resizable = self.app.resizable();
        let request = self.app.take_request();
        if let Some(w) = &self.win {
            if w.window.title() != title {
                w.window.set_title(&title);
            }
            if w.window.is_resizable() != resizable {
                w.window.set_resizable(resizable);
            }
            match request {
                Some(WindowRequest::Close) => self.exit_requested = true,
                Some(WindowRequest::Minimize) => w.window.set_minimized(true),
                Some(WindowRequest::Size(lw, lh)) => {
                    let want = LogicalSize::new(lw as f64, lh as f64)
                        .to_physical::<u32>(w.window.scale_factor());
                    let have = w.window.inner_size();
                    if !w.window.is_maximized()
                        && (want.width > have.width || want.height > have.height)
                    {
                        let size = PhysicalSize::new(
                            want.width.max(have.width),
                            want.height.max(have.height),
                        );
                        let _ = w.window.request_inner_size(size);
                        w.window.request_redraw();
                    }
                }
                None => {}
            }
        }
    }

    /// Tell the OS what changed: the taskbar identity (set before the window shows, and again
    /// when the home screen opens a file), run progress, and a finished run's notification.
    fn sync_os(&mut self, window: &Window) {
        if let Some(id) = self.app.identity() {
            if self.identity.as_deref() != Some(id.id.as_str()) {
                crate::os::identify(window, &id);
                self.identity = Some(id.id);
            }
        }
        let p = self.app.progress();
        if p != self.progress {
            crate::os::show_progress(window, p);
            self.progress = p;
        }
        if let Some((title, body, failed)) = self.app.take_notification() {
            if !self.focused && !self.exit_requested {
                crate::os::notify(window, &title, &body, failed);
            }
        }
    }

    /// The window's last normal rect and maximized state, for the state file.
    fn sample_placement(&mut self) {
        let Some(w) = self.win.as_mut() else { return };
        if !w.shown || w.window.is_minimized().unwrap_or(false) {
            return;
        }
        let maximized = w.window.is_maximized();
        if !maximized {
            if let Ok(pos) = w.window.outer_position() {
                let size = w.window.inner_size();
                if size.width > 0 && size.height > 0 {
                    w.normal_rect = Some(Rect {
                        x: pos.x,
                        y: pos.y,
                        width: size.width,
                        height: size.height,
                    });
                }
            }
        }
        self.app.set_placement(w.normal_rect, maximized);
    }

    fn switch_gpu(&mut self, device: gfx::Device) {
        let (Some(w), Some(gpu)) = (self.win.as_ref(), self.gpu.as_mut()) else {
            return;
        };
        let name = device.name().to_string();
        match gpu.switch_device(device, &w.window) {
            Ok(()) => launcher_core::log_info!("switched drawing to {name}"),
            Err(e) => launcher_core::log_warn!("staying on {}: {e}", gpu.device_name()),
        }
        w.window.request_redraw();
    }
}

/// The panic firewall around every handler. Returns false when `f` panicked (the panic hook in
/// `main` has logged it).
fn firewall(what: &str, f: impl FnOnce()) -> bool {
    let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_ok();
    if !ok {
        launcher_core::log_error!("recovered from a panic in {what}");
    }
    ok
}

impl ApplicationHandler<AppEvent> for Shell {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;
        crate::timing::report("resumed");
        // Finder's documents are in by now: the first one is what the window shows first.
        #[cfg(target_os = "macos")]
        self.app
            .open_at_launch(crate::os::macos::take_launch_files());
        let mut error = None;
        if !firewall("startup", || error = self.create_window(el).err()) {
            error = Some(format!(
                "an internal error ({})",
                crate::last_panic().unwrap_or_else(|| "no details".into())
            ));
        }
        if let Some(e) = error {
            launcher_core::log_error!("{e}");
            self.fatal = Some(format!(
                "{} could not open its window.\n\n{e}",
                crate::product::NAME
            ));
            el.exit();
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, event: AppEvent) {
        firewall("a cross-thread event", || match event {
            AppEvent::Repaint(delay) => {
                if let Some(w) = self.win.as_mut() {
                    let at = Instant::now() + delay;
                    w.repaint_at = Some(w.repaint_at.map_or(at, |t| t.min(at)));
                }
            }
            AppEvent::Gpu(device) => self.switch_gpu(device),
            other => {
                self.app.on_event(other);
                if let Some(w) = &self.win {
                    w.window.request_redraw();
                }
            }
        });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        firewall("a window event", || {
            let Some(w) = self.win.as_mut() else { return };
            let response = w.state.on_window_event(&w.window, &event);
            // egui-winit asks for a frame on `Moved` too, but moving changes nothing inside; a
            // vsync-paced present per move stalls the OS move loop and the drag lags.
            let redraws = !matches!(event, WindowEvent::RedrawRequested | WindowEvent::Moved(_));
            if response.repaint && redraws && w.shown {
                w.window.request_redraw();
            }
            match event {
                WindowEvent::RedrawRequested => {
                    self.frame();
                    if self.exit_requested {
                        el.exit();
                    }
                }
                WindowEvent::CloseRequested => {
                    if self.app.close_requested() {
                        el.exit();
                    } else if let Some(w) = &self.win {
                        // It asks first (a run is going).
                        w.window.request_redraw();
                    }
                }
                WindowEvent::Resized(size) => {
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.resize([size.width, size.height]);
                    }
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    if focused {
                        if let Some(w) = &self.win {
                            crate::os::clear_notification(&w.window);
                        }
                    }
                }
                _ => {}
            }
        });
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        firewall("the idle step", || {
            self.sample_placement();
            let Some(w) = self.win.as_mut() else {
                el.set_control_flow(ControlFlow::Wait);
                return;
            };
            if let Some(at) = w.repaint_at {
                if at <= Instant::now() {
                    w.repaint_at = None;
                    w.window.request_redraw();
                }
            }
            el.set_control_flow(match w.repaint_at {
                Some(at) => ControlFlow::WaitUntil(at),
                None => ControlFlow::Wait,
            });
        });
    }
}

/// Whether some monitor shows a useful part of the window's title bar: a remembered position on a
/// monitor that is gone must not open the window off-screen.
fn title_bar_visible(window: &Window) -> bool {
    let Ok(pos) = window.outer_position() else {
        return true;
    };
    let size = window.outer_size();
    let bar = (pos.x, pos.y, pos.x + size.width as i32, pos.y + 30);
    window.available_monitors().any(|m| {
        let mp = m.position();
        let ms = m.size();
        let (l, t, r, b) = (mp.x, mp.y, mp.x + ms.width as i32, mp.y + ms.height as i32);
        let w = bar.2.min(r) - bar.0.max(l);
        let h = bar.3.min(b) - bar.1.max(t);
        w >= 100 && h >= 10
    })
}

/// No remembered placement: centre on the monitor the window was created on, a little high.
fn center_on_monitor(window: &Window) {
    let Some(monitor) = window.current_monitor() else {
        return;
    };
    let m_pos = monitor.position();
    let m_size = monitor.size();
    let outer = window.outer_size();
    let x = m_pos.x + (m_size.width as i32 - outer.width as i32) / 2;
    let y = m_pos.y + (m_size.height as i32 - outer.height as i32) / 3;
    window.set_outer_position(PhysicalPosition::new(x.max(m_pos.x), y.max(m_pos.y)));
}

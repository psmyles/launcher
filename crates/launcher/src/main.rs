//! The launcher: a GUI front end for scripted workflows. A `.launcher` file defines a form, a list
//! of steps and a live console (docs/PLAN.md).
//!
//! Startup order:
//!   1. Borrow the parent console (Windows), so CLI output reaches a terminal.
//!   2. Parse the command line; non-GUI modes print and exit here, with no window or GPU.
//!   3. Start the GPU bring-up and font loading threads (the longest items) before anything else.
//!   4. Build the event loop and run the shell; show a fatal error after the loop returns.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cli;
mod console;
mod editor;
mod gfx;
mod headless;
mod maker;
mod os;
mod product;
mod shell;
mod timing;
mod ui;

use winit::event_loop::EventLoop;

fn main() {
    timing::report("main");
    install_panic_hook();
    let console = os::attach_parent_console();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let start = match cli::parse(&args) {
        Ok(cli::Mode::Gui(start)) => start,
        Ok(cli::Mode::Version) => {
            cli::out(&format!("{} {}", product::NAME, product::VERSION));
            return;
        }
        Ok(cli::Mode::Help) => {
            cli::out(&cli::usage());
            return;
        }
        Ok(cli::Mode::HelpConfig) => std::process::exit(cli::help_config()),
        Ok(cli::Mode::Check { files }) => std::process::exit(cli::check(&files)),
        Ok(cli::Mode::Run(r)) => std::process::exit(headless::run(&r)),
        Err(e) => {
            if console {
                cli::err(&format!("error: {e}"));
            } else {
                fatal(&e);
            }
            std::process::exit(cli::EXIT_USAGE);
        }
    };

    let gpu = gfx::Gpu::start(gfx::Policy::from_env());
    let fonts = ui::fonts::load();
    let event_loop = match EventLoop::<app::AppEvent>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            fatal(&format!(
                "{} could not create its event loop.\n\n{e}",
                product::NAME
            ));
            return;
        }
    };
    timing::report("event loop built");
    let proxy = event_loop.create_proxy();
    #[cfg(target_os = "macos")]
    os::macos::install_open_documents(proxy.clone());
    let mut shell = shell::Shell::new(app::App::new(start), gpu, fonts, proxy);
    if let Err(e) = event_loop.run_app(&mut shell) {
        launcher_core::log_error!("event loop error: {e}");
    }
    shell.shutdown();
    if let Some(message) = shell.take_fatal() {
        fatal(&message);
    }
}

/// Log every panic with its location (the firewall in `shell` recovers from it), and keep the last
/// message for a fatal report.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info.to_string();
        launcher_core::log_error!("panic: {message}");
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(message);
        }
        if cfg!(debug_assertions) {
            default(info);
        }
    }));
}

static LAST_PANIC: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The most recent panic message, if any.
pub fn last_panic() -> Option<String> {
    LAST_PANIC.lock().ok().and_then(|l| l.clone())
}

/// Last-resort error report: a message box, since a release build may have no console.
fn fatal(text: &str) {
    rfd::MessageDialog::new()
        .set_title(product::NAME)
        .set_level(rfd::MessageLevel::Error)
        .set_description(text)
        .show();
}

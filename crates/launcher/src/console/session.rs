//! One program in a console of its own (`launcher --console`): started through the real effects,
//! its output fed to a [`TermModel`] from the reader thread.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use launcher_core::engine::effects::{CommandSpec, Context, Effects, IoMode, Proc, RealEffects};
use launcher_core::platform::process::exit_code_text;

use super::{SharedTerm, TermModel};

/// Repaint pace while output flows (at most 60 frames a second, however fast it comes).
const FRAME: Duration = Duration::from_millis(16);

pub struct Session {
    pub term: SharedTerm,
    process: Arc<dyn Proc>,
    title: String,
    focus_pending: bool,
}

impl Session {
    /// Start `program` in a pty, or (`pipes`) with its output through pipes.
    pub fn start(
        program: &str,
        args: &[String],
        pipes: bool,
        ctx: egui::Context,
    ) -> Result<Session, String> {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let cmd = CommandSpec {
            program: program.to_string(),
            args: args.to_vec(),
            ..Default::default()
        };
        let term = TermModel::shared();
        let feed = term.clone();
        let started: Arc<OnceLock<Arc<dyn Proc>>> = Arc::default();
        let ended = started.clone();
        let on_output = Box::new(move |bytes: &[u8]| {
            if let Ok(mut m) = feed.lock() {
                if bytes.is_empty() {
                    m.detach();
                    if let Some(code) = ended.get().and_then(|p| p.try_wait()) {
                        m.note(&format!("exited with code {}", exit_code_text(code)));
                    }
                } else {
                    m.feed(bytes);
                }
            }
            ctx.request_repaint_after(FRAME);
        });
        let io = if pipes { IoMode::Pipe } else { IoMode::Auto };
        // Hold the model while starting, so output waits until the program is attached.
        let mut m = term.lock().map_err(|e| e.to_string())?;
        let size = m.size();
        let process: Arc<dyn Proc> = Arc::from(RealEffects.start(
            &Context::new(&cwd),
            &cmd,
            io,
            (size.0 as u16, size.1 as u16),
            on_output,
        )?);
        m.attach(process.clone());
        let _ = started.set(process.clone());
        drop(m);
        Ok(Session {
            term,
            process,
            title: cmd.display(),
            focus_pending: true,
        })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.process.try_wait()
    }

    pub fn kill(&self) {
        self.process.kill();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let focus = std::mem::take(&mut self.focus_pending);
        super::view::show(ui, &self.term, focus);
    }
}

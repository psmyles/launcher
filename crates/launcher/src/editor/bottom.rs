//! The editor's bottom panel (M8): Problems (loading and Lua lint), a Dry run of an action with
//! the preview's values, and the raw Text.

use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use egui::{RichText, Ui};
use launcher_core::config::{Config, Diagnostic, Level, NodePath};
use launcher_core::dynval::Dyn;
use launcher_core::engine::effects::DryRunEffects;
use launcher_core::engine::runner::{self, RunEvent, RunOutcome, RunSpec, RunUi};
use launcher_core::lua::Snapshot;
use launcher_core::platform::trust::Trusted;
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Problems,
    DryRun,
    Text,
}

/// A dry run's answer to every question: the default. It only lists what the run would do.
struct Defaults {
    done: mpsc::Sender<RunOutcome>,
}

impl RunUi for Defaults {
    fn event(&mut self, e: RunEvent) {
        if let RunEvent::Finished { outcome, .. } = e {
            let _ = self.done.send(outcome);
        }
    }
    fn message(&mut self, _text: &str, _level: Level) {}
    fn confirm(&mut self, _text: &str, _yes: &str, _no: &str) -> bool {
        true
    }
    fn ask(&mut self, _prompt: &str, default: Option<&str>, _password: bool) -> Option<String> {
        Some(default.unwrap_or_default().to_string())
    }
    fn warn(&mut self, _text: &str) {}
    fn set_control(&mut self, _id: &str, _value: Option<Dyn>) {}
}

/// The dry run's result, filled in from its thread.
/// What the dry run's thread reports: the lines, or why there are none. `None` until it ends.
pub type DryResult = Arc<Mutex<Option<Result<Vec<String>, String>>>>;

#[derive(Default)]
pub struct DryRun {
    pub action: String,
    pub lines: DryResult,
}

impl DryRun {
    pub fn start(
        &mut self,
        config: Arc<Config>,
        snapshot: Snapshot,
        env: std::collections::BTreeMap<String, String>,
        trusted: Trusted,
        proxy: EventLoopProxy<AppEvent>,
    ) {
        let slot = Arc::new(Mutex::new(None));
        self.lines = slot.clone();
        let action = self.action.clone();
        let _ = std::thread::Builder::new()
            .name("dry-run".into())
            .spawn(move || {
                let dry = Arc::new(DryRunEffects::default());
                let (tx, rx) = mpsc::channel();
                let result = runner::start(
                    RunSpec {
                        config,
                        action,
                        snapshot,
                        env,
                        effects: dry.clone(),
                        trusted,
                        resume: None,
                        history: false,
                        term_size: (100, 24),
                        pty: false,
                    },
                    Box::new(Defaults { done: tx }),
                )
                .and_then(|h| {
                    let outcome = rx.recv_timeout(Duration::from_secs(60)).map_err(|_| {
                        h.cancel();
                        "the dry run didn't finish in a minute".to_string()
                    })?;
                    h.join();
                    let mut lines = dry.lines();
                    if let RunOutcome::Failed(m) = outcome {
                        lines.push(format!("(it would stop here: {m})"));
                    }
                    Ok(lines)
                });
                if let Ok(mut s) = slot.lock() {
                    *s = Some(result);
                }
                let _ = proxy.send_event(AppEvent::Repaint(Duration::ZERO));
            });
    }
}

/// Problems: click one to select its node.
pub fn problems(ui: &mut Ui, diags: &[Diagnostic]) -> Option<NodePath> {
    let mut pick = None;
    if diags.is_empty() {
        ui.label(RichText::new("No problems.").weak());
        return None;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for d in diags {
                let color = if d.is_error() {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().warn_fg_color
                };
                let text =
                    RichText::new(format!("{}:{}  {}", d.line, d.col, d.message)).color(color);
                let r = ui.selectable_label(false, text);
                let r = match &d.help {
                    Some(h) => r.on_hover_text(h),
                    None => r,
                };
                if r.clicked() {
                    pick = d.path.clone();
                }
            }
        });
    pick
}

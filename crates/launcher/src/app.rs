//! What the window shows, independent of the winit/GPU plumbing in `shell`.

use std::path::PathBuf;
use std::time::Duration;

use launcher_core::state::{self, Rect, State};
use winit::event_loop::EventLoopProxy;

use crate::console::session::Session;
use crate::gfx;
use crate::ui::document::Document;
use crate::ui::gallery::Gallery;
use crate::ui::home::{self, HomeEvent};
use crate::ui::theme;

/// Cross-thread messages to the event loop.
pub enum AppEvent {
    /// egui asked for a repaint from another thread (or with a delay).
    Repaint(Duration),
    /// The hardware GPU is open; switch to it (N16).
    Gpu(gfx::Device),
    /// macOS: documents delivered by an open-document event.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    OpenFiles(Vec<PathBuf>),
    /// The opened file may have changed on disk.
    FileChanged,
    /// The evaluation thread answered.
    Eval(Box<launcher_core::eval::Response>),
    /// A Browse dialog closed (with the path, unless it was cancelled).
    Picked {
        control: String,
        path: Option<PathBuf>,
    },
    /// The editor's preview evaluation answered (answers from an older generation are dropped).
    PreviewEval {
        generation: u64,
        response: Box<launcher_core::eval::Response>,
    },
    /// The home screen's Open or New dialog closed.
    Chosen {
        /// `None`: open the file; `Some(t)`: save the new tool `t` there, then open it in the
        /// editor.
        template: Option<home::Template>,
        path: Option<PathBuf>,
    },
    /// "Make a new tool"'s folder dialog closed.
    FolderPicked(Option<PathBuf>),
}

/// What the content asks of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowRequest {
    Close,
    Minimize,
    /// At least this big (logical px).
    Size(f32, f32),
}

/// How the app was asked to start.
pub enum Start {
    Home,
    Document(PathBuf),
    /// A file in the visual editor (`launcher edit FILE`).
    Edit(PathBuf),
    /// One program in a console window (`--console`).
    Console {
        program: String,
        args: Vec<String>,
        /// Output through pipes instead of a pty.
        pipes: bool,
    },
    /// Every control in the current theme (`--gallery`).
    Gallery,
}

enum Mode {
    Home {
        recent: Vec<PathBuf>,
        /// A new tool that couldn't be written.
        error: Option<String>,
        /// "Make a new tool", while it's open.
        wizard: Option<home::Wizard>,
    },
    Document(Box<Document>),
    Console(Box<Session>),
    Gallery(Box<Gallery>),
    /// Starting the console failed; the message is shown instead.
    Failed(String),
}

pub struct App {
    start: Option<Start>,
    proxy: Option<EventLoopProxy<AppEvent>>,
    mode: Mode,
    state_path: Option<PathBuf>,
    about: String,
    state: State,
}

impl App {
    pub fn new(start: Start) -> App {
        let (state_path, about) = match &start {
            Start::Document(p) | Start::Edit(p) => (state::state_path(p), p.display().to_string()),
            Start::Gallery => (None, "the gallery".to_string()),
            _ => (state::home_state_path(), "the home screen".to_string()),
        };
        let state = state_path.as_deref().map(state::load).unwrap_or_default();
        App {
            start: Some(start),
            proxy: None,
            mode: Mode::Home {
                recent: Vec::new(),
                error: None,
                wizard: None,
            },
            state_path,
            about,
            state,
        }
    }

    /// macOS: the documents Finder started the app to open, before the window exists. The first
    /// takes the home screen's place (so it never shows); any others get processes of their own.
    #[cfg(target_os = "macos")]
    pub fn open_at_launch(&mut self, files: Vec<PathBuf>) {
        let mut files = files.into_iter();
        if matches!(self.start, Some(Start::Home)) {
            if let Some(p) = files.next() {
                launcher_core::log_info!("opened from Finder: {}", p.display());
                *self = App::new(Start::Document(p));
            }
        }
        for p in files {
            launcher_core::log_info!("opened from Finder: {} (a new process)", p.display());
            Self::open_elsewhere(&p);
        }
    }

    /// Set up the start mode once an egui context exists (the console needs it for repaints).
    pub fn begin(&mut self, ctx: &egui::Context, proxy: EventLoopProxy<AppEvent>) {
        self.proxy = Some(proxy.clone());
        let Some(start) = self.start.take() else {
            return;
        };
        self.mode = match start {
            Start::Home => Mode::Home {
                recent: launcher_core::recent::load(),
                error: None,
                wizard: None,
            },
            Start::Document(p) => Mode::Document(Box::new(Document::open(
                p,
                self.state.values.clone(),
                self.state.choices.clone(),
                proxy,
            ))),
            Start::Edit(p) => {
                let mut d = Document::open(
                    p,
                    self.state.values.clone(),
                    self.state.choices.clone(),
                    proxy,
                );
                d.open_editor();
                Mode::Document(Box::new(d))
            }
            Start::Console {
                program,
                args,
                pipes,
            } => match Session::start(&program, &args, pipes, ctx.clone()) {
                Ok(t) => Mode::Console(Box::new(t)),
                Err(e) => Mode::Failed(e),
            },
            Start::Gallery => Mode::Gallery(Box::new(Gallery::new())),
        };
    }

    pub fn title(&self) -> String {
        let name = crate::product::NAME;
        match (&self.mode, &self.start) {
            (Mode::Document(d), _) => match d.title() {
                Some(t) => t,
                None => format!(
                    "{} - {name}",
                    d.path.file_stem().unwrap_or_default().to_string_lossy()
                ),
            },
            (_, Some(Start::Document(p) | Start::Edit(p))) => format!(
                "{} - {name}",
                p.file_stem().unwrap_or_default().to_string_lossy()
            ),
            (Mode::Console(t), _) => format!("{} - {name}", t.title()),
            (_, Some(Start::Console { program, .. })) => format!("{program} - {name}"),
            (Mode::Gallery(_), _) | (_, Some(Start::Gallery)) => format!("Theme gallery - {name}"),
            _ => name.to_string(),
        }
    }

    /// The window size (logical px) when nothing is remembered.
    pub fn default_size(&self) -> (f64, f64) {
        match (&self.mode, &self.start) {
            (Mode::Console(_), _) | (_, Some(Start::Console { .. })) => (900.0, 560.0),
            (Mode::Gallery(_), _) | (_, Some(Start::Gallery)) => (1280.0, 860.0),
            _ => (560.0, 380.0),
        }
    }

    /// The window size the content asks for (logical px), when the window fits its content (N15).
    /// None while the window keeps the size it has.
    pub fn fit_size(&self) -> Option<(f32, f32)> {
        match &self.mode {
            Mode::Document(d) => d.fit_size(),
            _ => None,
        }
    }

    /// Whether the user may resize the window (not while it fits its content).
    pub fn resizable(&self) -> bool {
        match &self.mode {
            Mode::Document(d) => d.resizable(),
            _ => true,
        }
    }

    /// The window was asked to close: whether it may (a running action asks first).
    pub fn close_requested(&mut self) -> bool {
        match &mut self.mode {
            Mode::Document(d) => d.close_requested(),
            _ => true,
        }
    }

    pub fn take_request(&mut self) -> Option<WindowRequest> {
        match &mut self.mode {
            Mode::Document(d) => d.take_request(),
            _ => None,
        }
    }

    /// Write the state file (window placement and remembered values).
    fn save_state(&mut self) {
        if let Mode::Document(d) = &self.mode {
            if let Some(values) = d.remembered() {
                self.state.values = values.clone();
            }
            if let Some(choices) = d.filled() {
                self.state.choices = choices.clone();
            }
        }
        if let Some(path) = &self.state_path {
            if let Err(e) = state::save(path, &self.about, &self.state) {
                launcher_core::log_warn!("{e}");
            }
        }
    }

    pub fn placement(&self) -> (Option<Rect>, bool) {
        (self.state.window, self.state.maximized)
    }

    pub fn set_placement(&mut self, rect: Option<Rect>, maximized: bool) {
        self.state.window = rect;
        self.state.maximized = maximized;
    }

    /// Open a file in this window (from the home screen or an open-document event).
    fn open_document(&mut self, p: PathBuf) {
        let Some(proxy) = self.proxy.clone() else {
            return;
        };
        // The home screen's placement stays with it; the file has its own state.
        self.save_state();
        self.state_path = state::state_path(&p);
        self.about = p.display().to_string();
        self.state = self
            .state_path
            .as_deref()
            .map(state::load)
            .unwrap_or_default();
        self.mode = Mode::Document(Box::new(Document::open(
            p,
            self.state.values.clone(),
            self.state.choices.clone(),
            proxy,
        )));
    }

    /// Another file while one is open: it gets a process of its own (N14).
    fn open_elsewhere(p: &std::path::Path) {
        let started =
            std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg(p).spawn());
        if let Err(e) = started {
            launcher_core::log_warn!("could not open {}: {e}", p.display());
        }
    }

    /// The window's taskbar identity: a file's own, or none (the app's).
    pub fn identity(&self) -> Option<crate::os::Identity> {
        match &self.mode {
            Mode::Document(d) => Some(d.identity()),
            _ => None,
        }
    }

    pub fn progress(&self) -> crate::os::Progress {
        match &self.mode {
            Mode::Document(d) => d.progress(),
            _ => crate::os::Progress::Off,
        }
    }

    /// A run ended: what to tell the user if the window is behind others.
    pub fn take_notification(&mut self) -> Option<(String, String, bool)> {
        match &mut self.mode {
            Mode::Document(d) => d.take_notification(),
            _ => None,
        }
    }

    /// Show a problem on the home screen.
    fn home_error(&mut self, e: String) {
        if let Mode::Home { error, .. } = &mut self.mode {
            *error = Some(e);
        }
    }

    /// The home screen's Open or New dialog, on its own thread (N15).
    fn choose(&self, template: Option<home::Template>) {
        let Some(proxy) = self.proxy.clone() else {
            return;
        };
        let start = template
            .as_ref()
            .and_then(|t| t.folder.clone())
            .or_else(|| {
                launcher_core::recent::load()
                    .first()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            })
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let name = template.as_ref().map(|t| t.file_name.clone());
        let _ = std::thread::Builder::new()
            .name("choose".into())
            .spawn(move || {
                let d = rfd::FileDialog::new()
                    .set_directory(&start)
                    .add_filter("Tools", &["launcher"]);
                let path = match name {
                    Some(n) => d.set_file_name(n).save_file(),
                    None => d.pick_file(),
                };
                let _ = proxy.send_event(AppEvent::Chosen { template, path });
            });
    }

    /// "Make a new tool"'s Browse: a folder dialog on its own thread (N15).
    fn pick_folder(&self) {
        let Some(proxy) = self.proxy.clone() else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("choose-folder".into())
            .spawn(move || {
                let path = rfd::FileDialog::new().pick_folder();
                let _ = proxy.send_event(AppEvent::FolderPicked(path));
            });
    }

    pub fn on_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::OpenFiles(paths) => {
                let mut paths = paths.into_iter();
                if let (Mode::Home { .. }, Some(p)) = (&self.mode, paths.next()) {
                    launcher_core::log_info!("open-document event: {}", p.display());
                    self.open_document(p);
                }
                for p in paths {
                    launcher_core::log_info!(
                        "open-document event: {} (a new process)",
                        p.display()
                    );
                    Self::open_elsewhere(&p);
                }
            }
            AppEvent::FolderPicked(path) => {
                if let (
                    Mode::Home {
                        wizard: Some(w), ..
                    },
                    Some(p),
                ) = (&mut self.mode, path)
                {
                    w.folder = p.display().to_string();
                }
            }
            AppEvent::Chosen { template, path } => {
                let Some(mut path) = path else { return };
                let new = template.is_some();
                if let Some(t) = template {
                    if path.extension().is_none() {
                        path.set_extension("launcher");
                    }
                    if let Err(e) =
                        launcher_core::replace_file::write_bytes_replacing(&path, t.text)
                    {
                        if let Mode::Home { error, .. } = &mut self.mode {
                            *error = Some(format!("Couldn't write {}: {e}", path.display()));
                        }
                        return;
                    }
                }
                match self.mode {
                    Mode::Home { .. } => {
                        self.open_document(path);
                        // A new tool opens in the editor.
                        if let (true, Mode::Document(d)) = (new, &mut self.mode) {
                            d.open_editor();
                        }
                    }
                    _ => Self::open_elsewhere(&path),
                }
            }
            AppEvent::FileChanged => {
                if let Mode::Document(d) = &mut self.mode {
                    d.on_file_changed();
                }
            }
            AppEvent::PreviewEval {
                generation,
                response,
            } => {
                if let Mode::Document(d) = &mut self.mode {
                    d.on_preview(generation, *response);
                }
            }
            AppEvent::Eval(r) => {
                if let Mode::Document(d) = &mut self.mode {
                    d.on_eval(*r);
                }
            }
            AppEvent::Picked { control, path } => {
                if let Mode::Document(d) = &mut self.mode {
                    d.on_picked(control, path);
                }
            }
            AppEvent::Repaint(_) | AppEvent::Gpu(_) => {}
        }
    }

    /// The window is closing: stop what runs and remember the placement and values.
    pub fn on_exit(&mut self) {
        match &mut self.mode {
            Mode::Console(t) => {
                if t.exit_code().is_none() {
                    t.kill();
                }
            }
            Mode::Document(d) => d.stop(),
            _ => {}
        }
        self.save_state();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        match &mut self.mode {
            Mode::Home {
                recent,
                error,
                wizard,
            } => {
                let mut event = None;
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(egui::Margin::same(theme::extras(ui).margin as i8)),
                    )
                    .show(ui, |ui| {
                        event = home::show(ui, recent, error.as_deref());
                        if let Some(w) = wizard.as_mut() {
                            if let Some(e) = home::wizard(ui, w) {
                                event = Some(e);
                            }
                        }
                    });
                match event {
                    Some(HomeEvent::Open(p)) => self.open_document(p),
                    Some(HomeEvent::Browse) => self.choose(None),
                    Some(HomeEvent::NewTool) => {
                        if let Mode::Home { wizard, .. } = &mut self.mode {
                            *wizard = Some(home::Wizard::new());
                        }
                    }
                    Some(HomeEvent::ClearWizard) => {
                        if let Mode::Home { wizard, .. } = &mut self.mode {
                            *wizard = None;
                        }
                    }
                    Some(HomeEvent::PickFolder) => self.pick_folder(),
                    Some(HomeEvent::Create(t)) => {
                        if let Mode::Home { wizard, .. } = &mut self.mode {
                            *wizard = None;
                        }
                        self.choose(Some(t));
                    }
                    Some(HomeEvent::ExamplesFolder) => {
                        let opened = home::examples_dir().and_then(|d| {
                            launcher_core::platform::shell::open(&d.display().to_string())
                        });
                        if let Err(e) = opened {
                            self.home_error(e);
                        }
                    }
                    Some(HomeEvent::AddOnsFolder) => {
                        let opened = launcher_core::paths::app_data_subdir("add-ons")
                            .ok_or_else(|| "There is no app data folder.".to_string())
                            .and_then(|d| {
                                launcher_core::platform::shell::open(&d.display().to_string())
                            });
                        if let Err(e) = opened {
                            self.home_error(e);
                        }
                    }
                    Some(HomeEvent::Reveal(p)) => {
                        if let Err(e) = launcher_core::platform::shell::reveal(&p) {
                            self.home_error(e);
                        }
                    }
                    Some(HomeEvent::Forget(p)) => {
                        launcher_core::recent::remove(&p);
                        if let Mode::Home { recent, .. } = &mut self.mode {
                            recent.retain(|r| r != &p);
                        }
                    }
                    Some(HomeEvent::ClearRecent) => {
                        launcher_core::recent::clear();
                        if let Mode::Home { recent, .. } = &mut self.mode {
                            recent.clear();
                        }
                    }
                    None => {}
                }
            }
            Mode::Document(d) => {
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(egui::Margin::same(theme::extras(ui).margin as i8)),
                    )
                    .show(ui, |ui| d.ui(ui));
                // A changed choice is written when it's made, so it survives a crash or a
                // killed process.
                if d.take_dirty() {
                    self.save_state();
                }
            }
            Mode::Gallery(g) => g.ui(ui),
            Mode::Failed(e) => {
                let e = e.clone();
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                });
            }
            Mode::Console(t) => {
                egui::Panel::top("console-bar").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        match t.exit_code() {
                            None => {
                                ui.label("running");
                                if ui.button("Stop").clicked() {
                                    t.kill();
                                }
                            }
                            Some(0) => {
                                ui.label(
                                    egui::RichText::new("finished")
                                        .color(theme::extras(ui).success),
                                );
                            }
                            Some(code) => {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "exited with code {}",
                                        launcher_core::platform::process::exit_code_text(code)
                                    ))
                                    .color(ui.visuals().error_fg_color),
                                );
                            }
                        }
                        ui.add(egui::Label::new(egui::RichText::new(t.title()).weak()).truncate());
                    });
                });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| t.ui(ui));
            }
        }
    }
}

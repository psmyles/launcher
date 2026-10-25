//! The home screen: Make a new tool, Open a tool…, and Your tools (the ones opened lately). A
//! More menu opens the Examples and Add-ons folders.
//!
//! **Make a new tool** asks for a project folder (optional: a tool needn't be about one project),
//! shows what it found there, and offers examples to start from, the ones that fit the folder
//! first. Then it asks where to save the tool and opens it in the editor.

use std::path::{Path, PathBuf};

use egui::{RichText, Sense, Ui};
use launcher_core::addons::Library;

use crate::ui::theme;

pub enum HomeEvent {
    Open(PathBuf),
    /// Pick a tool to open.
    Browse,
    /// Start "Make a new tool".
    NewTool,
    /// Choose the new tool's project folder.
    PickFolder,
    /// Save a new tool (asking where), then open it in the editor.
    Create(Template),
    ExamplesFolder,
    AddOnsFolder,
    /// "Make a new tool" was cancelled.
    ClearWizard,
    /// Select the file in Explorer or Finder.
    Reveal(PathBuf),
    Forget(PathBuf),
    ClearRecent,
}

/// A new tool's text, and the name the save dialog suggests.
pub struct Template {
    pub file_name: String,
    pub text: String,
    /// Where the save dialog starts (the project folder, when one was chosen).
    pub folder: Option<PathBuf>,
}

/// A tool to start from: one with an `example` block.
#[derive(Clone)]
pub struct Example {
    pub name: String,
    pub about: String,
    /// The add-on whose app makes it a good fit for a folder (`git`).
    pub suggest: Option<String>,
    pub text: String,
}

/// The examples built into Launcher.
const BUILT_IN: [&str; 8] = [
    include_str!("../../../../examples/starters/get-latest.launcher"),
    include_str!("../../../../examples/starters/switch-and-update.launcher"),
    include_str!("../../../../examples/starters/copy-to-share.launcher"),
    include_str!("../../../../examples/starters/dotnet-build.launcher"),
    include_str!("../../../../examples/starters/run-preset.launcher"),
    include_str!("../../../../examples/starters/audit-models.launcher"),
    include_str!("../../../../examples/starters/audit-and-optimize.launcher"),
    include_str!("../../../../examples/starters/run-bite-workflow.launcher"),
];

fn folder_of(p: &Path) -> String {
    p.parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default()
}

/// The Examples folder, `<app data>/examples`: tools put there are offered too.
pub fn examples_dir() -> Result<PathBuf, String> {
    launcher_core::paths::app_data_subdir("examples").ok_or("There is no app data folder.".into())
}

/// A tool's `example` block, if it has one.
fn example_of(text: &str) -> Option<Example> {
    let doc = kdl::KdlDocument::parse_v2(text).ok()?;
    let n = doc.nodes().iter().find(|n| n.name().value() == "example")?;
    let get = |k: &str| {
        n.entries()
            .iter()
            .find(|e| e.name().is_some_and(|x| x.value() == k))
            .and_then(|e| e.value().as_string())
            .map(str::to_string)
    };
    Some(Example {
        name: get("name")?,
        about: get("about").unwrap_or_default(),
        suggest: get("suggest"),
        text: text.to_string(),
    })
}

/// Every example: Launcher's, then the Examples folder's.
pub fn examples() -> Vec<Example> {
    let mut out: Vec<Example> = BUILT_IN.iter().filter_map(|t| example_of(t)).collect();
    if let Some(rd) = examples_dir().ok().and_then(|d| std::fs::read_dir(d).ok()) {
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| Some(e.ok()?.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|x| x == launcher_core::paths::EXTENSION)
            })
            .collect();
        files.sort();
        for f in files {
            if let Some(e) = std::fs::read_to_string(&f)
                .ok()
                .and_then(|t| example_of(&t))
            {
                out.push(e);
            }
        }
    }
    out
}

/// "Make a new tool", while it's open.
pub struct Wizard {
    pub folder: String,
    /// The add-ons whose apps are in the folder: (id, name).
    found: Vec<(String, String)>,
    examples: Vec<Example>,
    /// The chosen example (None: start from nothing).
    chosen: Option<usize>,
    looked_at: String,
}

impl Wizard {
    pub fn new() -> Wizard {
        Wizard {
            folder: String::new(),
            found: Vec::new(),
            examples: examples(),
            chosen: None,
            looked_at: String::new(),
        }
    }

    /// Look at the folder again when it changes: which apps are there.
    fn detect(&mut self) {
        if self.looked_at == self.folder {
            return;
        }
        self.looked_at = self.folder.clone();
        let p = Path::new(self.folder.trim());
        self.found = if self.folder.trim().is_empty() || !p.is_dir() {
            Vec::new()
        } else {
            let lib = Library::current();
            lib.found_in(p)
                .into_iter()
                .filter_map(|id| lib.addon(&id).map(|a| (id.clone(), a.name.clone())))
                .collect()
        };
    }

    fn fits(&self, e: &Example) -> bool {
        e.suggest
            .as_ref()
            .is_some_and(|s| self.found.iter().any(|(id, _)| id == s))
    }
}

/// The "Make a new tool" dialog: Some(event) when the person chose.
pub fn wizard(ui: &mut Ui, w: &mut Wizard) -> Option<HomeEvent> {
    w.detect();
    let mut event = None;
    let mut close = false;
    let mut order: Vec<usize> = (0..w.examples.len()).collect();
    order.sort_by_key(|i| !w.fits(&w.examples[*i]));
    egui::Modal::new(egui::Id::new("new-tool")).show(ui.ctx(), |ui| {
        ui.set_width(520.0);
        ui.heading("Make a new tool");
        theme::gap(ui);
        ui.label(RichText::new("Project folder (optional)").strong());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut w.folder)
                    .hint_text("Leave empty for a tool that isn't about one project")
                    .desired_width(400.0),
            );
            if ui.button("Browse…").clicked() {
                event = Some(HomeEvent::PickFolder);
            }
        });
        if !w.found.is_empty() {
            let names: Vec<&str> = w.found.iter().map(|(_, n)| n.as_str()).collect();
            ui.label(RichText::new(format!("Found here: {}", names.join(" · "))).weak());
        } else if !w.folder.trim().is_empty() && !Path::new(w.folder.trim()).is_dir() {
            ui.label(RichText::new("That folder doesn't exist.").color(ui.visuals().warn_fg_color));
        }
        theme::gap(ui);
        ui.label(RichText::new("Start from").strong());
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                if ui
                    .radio(w.chosen.is_none(), "Nothing: an empty tool")
                    .clicked()
                {
                    w.chosen = None;
                }
                for i in order {
                    let e = &w.examples[i];
                    let fits = w.fits(e);
                    ui.horizontal(|ui| {
                        if ui.radio(w.chosen == Some(i), &e.name).clicked() {
                            w.chosen = Some(i);
                        }
                        if fits {
                            ui.label(
                                RichText::new("fits this folder")
                                    .small()
                                    .color(theme::extras(ui).success),
                            );
                        }
                    });
                    if !e.about.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(24.0);
                            ui.label(RichText::new(&e.about).small().weak());
                        });
                    }
                }
            });
        theme::gap(ui);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                close = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(RichText::new("Next: choose where to save it").strong())
                    .clicked()
                {
                    let (name, text) = match w.chosen.map(|i| &w.examples[i]) {
                        Some(e) => (e.name.clone(), strip_example(&e.text)),
                        None => (
                            "My tool".to_string(),
                            launcher_core::config::tool_edit::new_tool_text("My tool", None),
                        ),
                    };
                    let folder = Some(PathBuf::from(w.folder.trim()))
                        .filter(|p| !w.folder.trim().is_empty() && p.is_dir());
                    event = Some(HomeEvent::Create(Template {
                        file_name: format!("{name}.launcher"),
                        text,
                        folder,
                    }));
                }
            });
        });
    });
    if close {
        return Some(HomeEvent::ClearWizard);
    }
    event
}

/// A new tool made from an example doesn't carry the `example` block (it isn't an example).
fn strip_example(text: &str) -> String {
    match kdl::KdlDocument::parse_v2(text) {
        Ok(mut d) => {
            d.nodes_mut().retain(|n| n.name().value() != "example");
            launcher_core::config::print::print(&d)
        }
        Err(_) => text.to_string(),
    }
}

pub fn show(ui: &mut Ui, recent: &[PathBuf], error: Option<&str>) -> Option<HomeEvent> {
    let mut event = None;
    theme::gap(ui);
    ui.horizontal(|ui| {
        ui.heading(crate::product::NAME);
        ui.label(RichText::new(format!("Version {}", crate::product::VERSION)).weak());
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui
            .button(RichText::new("Make a new tool").strong())
            .clicked()
        {
            event = Some(HomeEvent::NewTool);
        }
        if ui.button("Open a tool…").clicked() {
            event = Some(HomeEvent::Browse);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("More", |ui| {
                if ui
                    .button("Examples folder")
                    .on_hover_text("Tools you put here are offered by Make a new tool")
                    .clicked()
                {
                    event = Some(HomeEvent::ExamplesFolder);
                    ui.close();
                }
                if ui
                    .button("Add-ons folder")
                    .on_hover_text("Add-ons you put here add steps to every tool")
                    .clicked()
                {
                    event = Some(HomeEvent::AddOnsFolder);
                    ui.close();
                }
                if ui
                    .add_enabled(!recent.is_empty(), egui::Button::new("Clear the list"))
                    .clicked()
                {
                    event = Some(HomeEvent::ClearRecent);
                    ui.close();
                }
            });
        });
    });
    if let Some(e) = error {
        ui.add_space(6.0);
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    ui.add_space(16.0);
    ui.label(RichText::new("Your tools").strong());
    theme::gap_small(ui);
    if recent.is_empty() {
        ui.label(RichText::new("Tools you open or make show up here.").weak());
        return event;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for p in recent {
                if let Some(e) = recent_row(ui, p) {
                    event = Some(e);
                }
            }
        });
    event
}

/// One recent file: a button the width of the list (the name over its folder) that opens it, with
/// Show in folder at its right end (Remove when the file is gone).
fn recent_row(ui: &mut Ui, p: &Path) -> Option<HomeEvent> {
    let mut event = None;
    let exists = p.is_file();
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (name_text, folder_text) = if exists {
        (RichText::new(&name).strong(), RichText::new(folder_of(p)))
    } else {
        (
            RichText::new(&name).weak(),
            RichText::new(format!("{} (not found)", folder_of(p))),
        )
    };
    let side = if exists { "Show in folder" } else { "Remove" };

    // Laid out by hand, so the text is painted (not selectable labels over the row).
    let pad = egui::vec2(ui.spacing().button_padding.x, theme::extras(ui).gap_small);
    let gap = ui.spacing().item_spacing;
    let width = ui.available_width();
    let side_galley = egui::WidgetText::from(side).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let side_size = egui::vec2(
        side_galley.size().x + 2.0 * ui.spacing().button_padding.x,
        ui.spacing().interact_size.y,
    );
    let text_width = (width - 2.0 * pad.x - side_size.x - gap.x).max(0.0);
    let name_galley = egui::WidgetText::from(name_text).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        text_width,
        egui::TextStyle::Body,
    );
    let folder_galley = egui::WidgetText::from(folder_text.small().weak()).into_galley(
        ui,
        Some(egui::TextWrapMode::Wrap),
        text_width,
        egui::TextStyle::Small,
    );
    let text_height = name_galley.size().y + folder_galley.size().y;
    let height = text_height.max(side_size.y) + 2.0 * pad.y;
    let (rect, row) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, exists, &name));

    // Lit while the pointer is over it (its side button too) or it has the focus.
    let lit = row.contains_pointer() || row.has_focus();
    if ui.is_rect_visible(rect) {
        // A frameless button: filled while lit or pressed.
        let w = &ui.visuals().widgets;
        let look = if !exists {
            None
        } else if row.is_pointer_button_down_on() {
            Some(w.active)
        } else if lit {
            Some(w.hovered)
        } else {
            None
        };
        if let Some(v) = look {
            ui.painter().rect(
                rect,
                v.corner_radius,
                v.weak_bg_fill,
                v.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        let text_color = ui.visuals().text_color();
        let mut at = egui::pos2(rect.left() + pad.x, rect.center().y - text_height / 2.0);
        let name_height = name_galley.size().y;
        ui.painter().galley(at, name_galley, text_color);
        at.y += name_height;
        ui.painter().galley(at, folder_galley, text_color);
    }

    let side_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - pad.x - side_size.x / 2.0, rect.center().y),
        side_size,
    );
    // Framed only on a lit row, so a column of them doesn't outweigh the names.
    let side_text = if lit {
        RichText::new(side)
    } else {
        RichText::new(side).weak()
    };
    let side_button = egui::Button::new(side_text).frame_when_inactive(lit);
    if ui.put(side_rect, side_button).clicked() {
        event = Some(if exists {
            HomeEvent::Reveal(p.to_path_buf())
        } else {
            HomeEvent::Forget(p.to_path_buf())
        });
    }
    if row.clicked() && exists {
        event = Some(HomeEvent::Open(p.to_path_buf()));
    }
    row.context_menu(|ui| {
        if exists && ui.button("Show in folder").clicked() {
            event = Some(HomeEvent::Reveal(p.to_path_buf()));
            ui.close();
        }
        if ui.button("Remove from the list").clicked() {
            event = Some(HomeEvent::Forget(p.to_path_buf()));
            ui.close();
        }
    });
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::config::{load_str, LoadOptions};

    #[test]
    fn every_built_in_example_loads_cleanly() {
        for text in BUILT_IN {
            let e = example_of(text).expect("an example block");
            let l = load_str(
                &strip_example(text),
                Path::new("x.launcher"),
                &LoadOptions {
                    runtime: crate::cli::runtime_version(),
                    os: launcher_core::config::catalog::Os::current(),
                },
            );
            assert!(l.diags.is_empty(), "{}: {:?}", e.name, l.diags);
        }
    }

    #[test]
    fn a_new_empty_tool_loads_cleanly() {
        let t = launcher_core::config::tool_edit::new_tool_text("My tool", None);
        let l = load_str(&t, Path::new("x.launcher"), &LoadOptions::default());
        assert!(l.diags.is_empty(), "{:?}", l.diags);
    }
}

//! The step picker (N26): a panel that opens from a +. Groups by app on the left, their steps in
//! the middle, and what the chosen step does on the right, before it's added. "Suggested" holds
//! the steps for the apps in the tool's folder and what usually comes after the step above the
//! +. Search covers every group, including the everyday words add-ons list for their steps. A
//! step whose app isn't on this computer is still shown (the tool may be for someone else), with
//! a note. Opened from "Add a check", it lists only checks.

use std::sync::Arc;

use egui::{RichText, Ui};
use launcher_core::addons::{Library, StepDef};
use launcher_core::config::tool_edit::StepAt;

use super::steps::app_mark;
use crate::ui::theme;

/// The space inside every pane and row of the panel, so their contents line up: across (the
/// title, the groups, the steps and the details start the same distance in) and down (the first
/// line of each pane at the same height).
const PAD: f32 = 8.0;
const PAD_Y: f32 = 6.0;

/// The built-in add-ons that aren't apps: listed after the apps.
const GENERAL: &[&str] = &["files", "internet", "ask-tell", "core"];

#[derive(Clone, PartialEq, Eq)]
pub enum Group {
    Suggested,
    Checks,
    AddOn(String),
    Advanced,
}

pub struct Picker {
    /// Where the step goes.
    pub at: StepAt,
    pub search: String,
    pub group: Group,
    pub selected: Option<String>,
    focus: bool,
    /// Opened from "Add a check": only checks.
    checks_only: bool,
}

impl Picker {
    pub fn new(at: StepAt, has_suggestions: bool) -> Picker {
        let checks_only = at.before;
        Picker {
            at,
            search: String::new(),
            checks_only,
            group: if checks_only {
                Group::Checks
            } else if has_suggestions {
                Group::Suggested
            } else {
                Group::AddOn("files".into())
            },
            selected: None,
            focus: true,
        }
    }
}

pub struct PickCx<'a> {
    pub lib: &'a Library,
    /// Add-ons whose apps are in the tool's folder.
    pub found: &'a [String],
    /// What usually follows the step above the +.
    pub next: &'a [Arc<StepDef>],
    /// Why a step can't run on this computer (its app is missing), if so.
    pub missing: &'a dyn Fn(&StepDef) -> Option<String>,
    /// Why a step can never run in this tool (it's for another OS than the tool's), if so: it
    /// can't be added.
    pub blocked: &'a dyn Fn(&StepDef) -> Option<String>,
}

pub enum Picked {
    Nothing,
    Add(Arc<StepDef>),
    Close,
}

/// The groups in order: Suggested, the apps, the general ones, Advanced.
fn groups(cx: &PickCx, checks_only: bool) -> Vec<(Group, String)> {
    let mut out = Vec::new();
    if checks_only {
        out.push((Group::Checks, "Checks".to_string()));
        return out;
    }
    if !cx.found.is_empty() || !cx.next.is_empty() {
        out.push((Group::Suggested, "Suggested".to_string()));
    }
    let mut apps: Vec<_> = cx
        .lib
        .addons
        .iter()
        .filter(|a| !GENERAL.contains(&a.id.as_str()))
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    for a in apps {
        out.push((Group::AddOn(a.id.clone()), a.name.clone()));
    }
    for id in GENERAL {
        if let Some(a) = cx.lib.addon(id) {
            out.push((Group::AddOn(a.id.clone()), a.name.clone()));
        }
    }
    out.push((Group::Advanced, "Advanced".to_string()));
    out
}

/// The steps a group lists.
fn group_steps(cx: &PickCx, g: &Group) -> Vec<Arc<StepDef>> {
    match g {
        Group::Suggested => {
            let mut v: Vec<Arc<StepDef>> = cx.next.to_vec();
            for id in cx.found {
                if let Some(a) = cx.lib.addon(id) {
                    v.extend(a.steps.iter().filter(|s| !s.advanced).cloned());
                }
            }
            let mut seen = std::collections::HashSet::new();
            v.retain(|s| seen.insert(s.id.clone()));
            v
        }
        Group::AddOn(id) => cx
            .lib
            .addon(id)
            .map(|a| a.steps.iter().filter(|s| !s.advanced).cloned().collect())
            .unwrap_or_default(),
        Group::Checks => cx.lib.steps().filter(|s| s.check_step).cloned().collect(),
        Group::Advanced => cx.lib.steps().filter(|s| s.advanced).cloned().collect(),
    }
}

/// Every step matching the search, best matches first: each word of the search has to be
/// somewhere in the step's name, everyday words, app or description.
fn search(cx: &PickCx, q: &str, checks_only: bool) -> Vec<Arc<StepDef>> {
    let q = q.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<(usize, Arc<StepDef>)> = cx
        .lib
        .steps()
        .filter(|s| !checks_only || s.check_step)
        .filter_map(|s| {
            let name = s.name.to_lowercase();
            let everyday = s.search_words.join(" ").to_lowercase();
            let app = s.app.to_lowercase();
            let rest = format!(
                "{} {} {}",
                s.about.as_deref().unwrap_or_default(),
                s.says.as_deref().unwrap_or_default(),
                s.id
            )
            .to_lowercase();
            // How well each word matches: 0 in the name, 1 in the everyday words, 2 the app, 3
            // elsewhere.
            let mut score = 0;
            for w in &words {
                score += if name.contains(w) {
                    0
                } else if everyday.contains(w) {
                    1
                } else if app.contains(w) {
                    2
                } else if rest.contains(w) {
                    3
                } else {
                    return None;
                };
            }
            Some((score, s.clone()))
        })
        .collect();
    scored.sort_by_key(|(score, s)| (*score, s.advanced, s.name.to_lowercase()));
    scored.into_iter().map(|(_, s)| s).collect()
}

/// The panel, over the editor: the rest of the window dims and can't be used while it's open.
pub fn show(ctx: &egui::Context, p: &mut Picker, cx: &PickCx) -> Picked {
    let mut picked = Picked::Nothing;
    let groups = groups(cx, p.checks_only);
    let list = if p.search.trim().is_empty() {
        group_steps(cx, &p.group)
    } else {
        search(cx, p.search.trim(), p.checks_only)
    };
    if p.selected
        .as_ref()
        .is_none_or(|s| !list.iter().any(|d| &d.id == s))
    {
        p.selected = list.first().map(|d| d.id.clone());
    }
    // Keys: arrows move, Enter adds, Escape closes.
    let (up, down, enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    let pos = p
        .selected
        .as_ref()
        .and_then(|s| list.iter().position(|d| &d.id == s));
    if let Some(i) = pos {
        if up && i > 0 {
            p.selected = Some(list[i - 1].id.clone());
        }
        if down && i + 1 < list.len() {
            p.selected = Some(list[i + 1].id.clone());
        }
    }
    if esc {
        return Picked::Close;
    }
    if enter {
        if let Some(d) = p
            .selected
            .as_ref()
            .and_then(|s| list.iter().find(|d| &d.id == s))
            .filter(|d| (cx.blocked)(d).is_none())
        {
            return Picked::Add(d.clone());
        }
    }
    let modal = egui::Modal::new(egui::Id::new("step-picker"))
        .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(12.0))
        .show(ctx, |ui| {
            ui.set_width(760.0);
            ui.set_height(480.0);
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                let title = if p.checks_only {
                    "Add a check"
                } else {
                    "Add a step"
                };
                ui.label(RichText::new(title).strong().size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(PAD);
                    if ui.button("Close").on_hover_text("Esc").clicked() {
                        picked = Picked::Close;
                    }
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut p.search)
                            .hint_text("Search steps…")
                            .desired_width(320.0),
                    );
                    if p.focus {
                        r.request_focus();
                        p.focus = false;
                    }
                });
            });
            theme::gap_small(ui);
            ui.separator();
            ui.horizontal_top(|ui| {
                // Groups.
                ui.vertical(|ui| {
                    ui.set_width(160.0);
                    ui.spacing_mut().button_padding = egui::vec2(PAD, PAD_Y);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (g, name) in &groups {
                        let on = p.search.trim().is_empty() && p.group == *g;
                        // The whole width is the button.
                        let b = egui::Button::selectable(on, (name.as_str(), egui::Atom::grow()))
                            .min_size(egui::vec2(ui.available_width(), 0.0));
                        if ui.add(b).clicked() {
                            p.group = g.clone();
                            p.search.clear();
                        }
                    }
                });
                ui.separator();
                // Steps.
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    egui::ScrollArea::vertical()
                        .id_salt("picker-steps")
                        .max_height(420.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            if list.is_empty() {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(PAD as i8, PAD_Y as i8))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("No steps match.").weak());
                                    });
                            }
                            for d in &list {
                                let on = p.selected.as_deref() == Some(d.id.as_str());
                                let blocked = (cx.blocked)(d).is_some();
                                let missing = blocked || (cx.missing)(d).is_some();
                                let r = entry(ui, d, on, missing);
                                if r.clicked() {
                                    p.selected = Some(d.id.clone());
                                }
                                if r.double_clicked() && !blocked {
                                    picked = Picked::Add(d.clone());
                                }
                                if on && (up || down) {
                                    r.scroll_to_me(None);
                                }
                            }
                        });
                });
                ui.separator();
                // What the step does.
                ui.vertical(|ui| {
                    ui.set_width(250.0);
                    let Some(d) = p
                        .selected
                        .as_ref()
                        .and_then(|s| list.iter().find(|d| &d.id == s))
                    else {
                        return;
                    };
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(PAD as i8, PAD_Y as i8))
                        .show(ui, |ui| {
                            let blocked = (cx.blocked)(d);
                            details(ui, d, blocked.clone().or_else(|| (cx.missing)(d)));
                            theme::gap(ui);
                            if ui
                                .add_enabled(
                                    blocked.is_none(),
                                    egui::Button::new(RichText::new("Add step").strong()),
                                )
                                .clicked()
                            {
                                picked = Picked::Add(d.clone());
                            }
                        });
                });
            });
        });
    // A click outside closes it (Escape is taken above).
    if modal.backdrop_response.clicked() {
        return Picked::Close;
    }
    picked
}

/// A step in the list: the whole width is its button, filled when selected.
fn entry(ui: &mut Ui, d: &StepDef, on: bool, missing: bool) -> egui::Response {
    let bg = ui.painter().add(egui::Shape::Noop);
    let width = ui.available_width();
    let inner = ui
        .scope(|ui| {
            ui.set_width(width);
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(PAD as i8, PAD_Y as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        app_mark(ui, &d.app);
                        ui.vertical(|ui| {
                            let name = RichText::new(&d.name).strong();
                            ui.label(if missing { name.weak() } else { name });
                            let line = d.about.as_deref().or(d.says.as_deref()).unwrap_or_default();
                            // Readable on the selection's fill.
                            let line = RichText::new(line).small();
                            ui.label(if on {
                                line.color(ui.visuals().strong_text_color())
                            } else {
                                line.weak()
                            });
                        });
                    });
                });
        })
        .response;
    let r = ui.interact(
        inner.rect,
        ui.id().with(("pick", &d.id)),
        egui::Sense::click(),
    );
    let v = ui.visuals();
    let fill = if on {
        Some(v.selection.bg_fill)
    } else if r.hovered() {
        Some(v.widgets.hovered.weak_bg_fill)
    } else {
        None
    };
    if let Some(f) = fill {
        ui.painter()
            .set(bg, egui::Shape::rect_filled(inner.rect, 6.0, f));
    }
    r
}

fn details(ui: &mut Ui, d: &StepDef, missing: Option<String>) {
    ui.horizontal(|ui| {
        app_mark(ui, &d.app);
        ui.label(RichText::new(&d.name).strong().size(15.0));
    });
    ui.label(RichText::new(&d.app).weak());
    theme::gap_small(ui);
    if let Some(a) = &d.about {
        ui.label(a);
        theme::gap_small(ui);
    }
    if let Some(s) = &d.says {
        // Its settings by name: "Switch to [Branch]".
        let mut shown = s.clone();
        for st in &d.settings {
            shown = shown.replace(&format!("{{{}}}", st.id), &format!("[{}]", st.label));
        }
        ui.label(RichText::new("On its card:").weak());
        ui.label(RichText::new(shown).italics());
        theme::gap_small(ui);
    }
    let choose: Vec<&str> = d
        .settings
        .iter()
        .filter(|s| s.required && s.starts.is_none())
        .map(|s| s.label.as_str())
        .collect();
    if !choose.is_empty() {
        ui.label(RichText::new("You choose:").weak());
        ui.label(choose.join(", "));
        theme::gap_small(ui);
    }
    // A setting with a list (Git's branches): it can be a list in the window.
    for st in d.settings.iter().filter(|s| s.list_from.is_some()) {
        ui.label(
            RichText::new(format!(
                "You can pick the {} here, or let people choose it from a list when they use \
                 the tool.",
                st.label
            ))
            .weak(),
        );
        theme::gap_small(ui);
    }
    if !d.results.is_empty() {
        let r: Vec<&str> = d.results.iter().map(|r| r.label.as_str()).collect();
        ui.label(RichText::new("It gives you:").weak());
        ui.label(r.join(", "));
        theme::gap_small(ui);
    }
    if let Some(m) = missing {
        ui.label(RichText::new(m).color(ui.visuals().warn_fg_color));
    }
}

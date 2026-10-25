//! The middle column: what the tool does when a button is pressed. "Before starting, make
//! sure…" first, a short list of checks (what the steps need, and checks added with "Add a
//! check"), then the steps as cards joined by arrows, with a + in every gap. A + opens the step
//! picker; a card is clicked to select it and dragged onto a gap to move it (N20: no free
//! placement, no wires, no zoom; it scrolls like a page).

use egui::{Color32, RichText, Sense, Ui};
use launcher_core::config::tool_edit::StepAt;

use crate::ui::form::number_text;
use crate::ui::theme;

/// A card's small note: what it asks, when it runs, what's missing.
pub struct Badge {
    pub text: String,
    pub bad: bool,
    /// The control it names: its number and label, after the text.
    pub control: Option<(usize, String)>,
}

/// One step, ready to draw.
pub struct Card {
    pub at: StepAt,
    pub app: String,
    pub name: String,
    pub sentence: String,
    pub badges: Vec<Badge>,
    /// A problem with this step, in a sentence.
    pub problem: Option<String>,
}

/// What the person did in the column.
pub enum StepsEvent {
    Select(StepAt),
    /// A + was clicked: add a step here.
    Plus(StepAt),
    Move(StepAt, StepAt),
    Delete(StepAt),
    Duplicate(StepAt),
}

/// Everything the column shows.
pub struct StepsCx<'a> {
    pub steps: &'a str,
    pub button_label: &'a str,
    /// The button's number in the window.
    pub button_number: Option<usize>,
    pub before: &'a [Card],
    pub main: &'a [Card],
    /// What the steps need on the computer: (sentence, there).
    pub needs: &'a [(String, bool)],
    pub selected: Option<&'a StepAt>,
    /// Whether a step can be moved there: not above a step whose results it uses, nor below one
    /// that uses its results.
    pub can_drop: &'a dyn Fn(&StepAt, &StepAt) -> bool,
    /// Bring this card into view.
    pub scroll_to: Option<StepAt>,
}

/// A card's frame around its content: margins and the widest outline, both sides.
const FRAME: f32 = 24.0;

/// The apps' colours: light enough for a dark initial to read on them at `contrast::MIN`.
const APP_HUES: [Color32; 6] = [
    Color32::from_rgb(0xea, 0x7a, 0x5a),
    Color32::from_rgb(0x5c, 0x98, 0xe8),
    Color32::from_rgb(0x5a, 0xb0, 0x6a),
    Color32::from_rgb(0xc0, 0x7c, 0xd8),
    Color32::from_rgb(0xd0, 0xa0, 0x30),
    Color32::from_rgb(0x40, 0xa8, 0xa8),
];

/// The initial on an app's colour.
const APP_INITIAL: Color32 = Color32::from_gray(0x10);

/// A colour per app, so cards of one app look alike.
fn app_color(app: &str) -> Color32 {
    let h = launcher_core::paths::fnv1a64(app.as_bytes());
    APP_HUES[(h % APP_HUES.len() as u64) as usize]
}

/// The app's mark: its initial on its colour.
pub fn app_mark(ui: &mut Ui, app: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::hover());
    let c = app_color(app);
    ui.painter().rect_filled(rect, 6.0, c);
    let letter = app
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        egui::FontId::proportional(14.0),
        APP_INITIAL,
    );
}

/// The column. `drag` is the card being dragged, kept across frames.
pub fn show(ui: &mut Ui, cx: &StepsCx, drag: &mut Option<StepAt>) -> Vec<StepsEvent> {
    let mut events = Vec::new();
    let ex = theme::extras(ui);
    // A drag ends when the pointer is let go, wherever that is.
    let released = ui.input(|i| i.pointer.any_released());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Everything is as wide as the column, from its left edge, like the other columns.
            let w = ui.available_width().max(160.0);
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label("When");
                    if let Some(n) = cx.button_number {
                        ui.label(number_text(ui, n));
                    }
                    ui.label(RichText::new(cx.button_label).strong());
                    ui.label("is pressed:");
                });
                ui.add_space(ex.gap);
                // Before starting, make sure…
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(8.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(w - FRAME);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.label(RichText::new("Before starting, make sure:").strong());
                            // Every line starts where a check's button puts its text.
                            let pad = ui.spacing().button_padding.x;
                            for (need, there) in cx.needs {
                                ui.horizontal(|ui| {
                                    ui.add_space(pad);
                                    if *there {
                                        ui.label(RichText::new("✔").color(ex.success));
                                        ui.label(need);
                                    } else {
                                        ui.label(
                                            RichText::new("✖").color(ui.visuals().error_fg_color),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{need}: it isn't on this computer"
                                            ))
                                            .color(ui.visuals().error_fg_color),
                                        );
                                    }
                                });
                            }
                            for c in cx.before {
                                check_line(ui, c, cx.selected == Some(&c.at), &mut events);
                            }
                            if cx.needs.is_empty() && cx.before.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(pad);
                                    ui.label(RichText::new("Nothing to check yet.").weak());
                                });
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("+ Add a check").small())
                                        .frame_when_inactive(false),
                                )
                                .on_hover_text("Something to make sure of before any step runs")
                                .clicked()
                            {
                                events.push(StepsEvent::Plus(StepAt {
                                    steps: cx.steps.to_string(),
                                    before: true,
                                    index: cx.before.len(),
                                }));
                            }
                        });
                    });
                ui.add_space(ex.gap);
                if cx.main.is_empty() && drag.is_none() {
                    if ui
                        .add(
                            egui::Button::new("+ Add the first step").min_size(egui::vec2(w, 40.0)),
                        )
                        .clicked()
                    {
                        events.push(StepsEvent::Plus(StepAt {
                            steps: cx.steps.to_string(),
                            before: false,
                            index: 0,
                        }));
                    }
                } else {
                    list(ui, cx, w, cx.main, drag, &mut events);
                }
                ui.add_space(ex.gap * 2.0);
            });
        });
    if released && drag.is_some() {
        *drag = None;
    }
    if drag.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    events
}

/// A check in "Before starting": one line, selected with a click.
fn check_line(ui: &mut Ui, c: &Card, selected: bool, events: &mut Vec<StepsEvent>) {
    let ex = theme::extras(ui);
    // Its name ("No unsaved changes"): its sentence would repeat "make sure". The whole line
    // is the button.
    let r = ui.add(
        egui::Button::selectable(
            selected,
            (
                RichText::new("✔").color(ex.success),
                c.name.as_str(),
                egui::Atom::grow(),
            ),
        )
        .min_size(egui::vec2(ui.available_width(), 0.0)),
    );
    if let Some(p) = &c.problem {
        ui.label(RichText::new(p).small().color(ui.visuals().error_fg_color));
    }
    if r.clicked() {
        events.push(StepsEvent::Select(c.at.clone()));
    }
    r.context_menu(|ui| {
        if ui.button("Delete").clicked() {
            events.push(StepsEvent::Delete(c.at.clone()));
            ui.close();
        }
    });
}

/// The cards, a + before each and after the last.
fn list(
    ui: &mut Ui,
    cx: &StepsCx,
    w: f32,
    cards: &[Card],
    drag: &mut Option<StepAt>,
    events: &mut Vec<StepsEvent>,
) {
    let at = |index| StepAt {
        steps: cx.steps.to_string(),
        before: false,
        index,
    };
    for (i, c) in cards.iter().enumerate() {
        gap(ui, cx, w, at(i), drag, events);
        let r = card(ui, w, c, cx.selected == Some(&c.at), drag, events);
        if cx.scroll_to.as_ref() == Some(&c.at) {
            r.scroll_to_me(Some(egui::Align::Center));
        }
    }
    gap(ui, cx, w, at(cards.len()), drag, events);
}

/// The space between two cards: an arrow and a +, or, while a card is dragged, a place to drop
/// it.
fn gap(
    ui: &mut Ui,
    cx: &StepsCx,
    w: f32,
    to: StepAt,
    drag: &mut Option<StepAt>,
    events: &mut Vec<StepsEvent>,
) {
    let ex = theme::extras(ui);
    if let Some(from) = drag.clone() {
        // Dropping a card just above or below itself changes nothing.
        let same_place =
            from.before == to.before && (from.index == to.index || from.index + 1 == to.index);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 28.0), Sense::hover());
        if same_place {
            return;
        }
        // Moving down: the card leaves its place first, so the gap is one earlier.
        let mut to = to;
        if from.before == to.before && from.index < to.index {
            to.index -= 1;
        }
        // Not where it would use what a step finds before that step has run.
        if !(cx.can_drop)(&from, &to) {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Not here",
                egui::FontId::proportional(12.0),
                ui.visuals().weak_text_color(),
            );
            resp.on_hover_text(
                "Here it would use what a step finds before that step has run, or run before \
                 a step that uses what it finds",
            );
            return;
        }
        let hovered = resp.contains_pointer();
        let stroke = egui::Stroke::new(
            if hovered { 2.0 } else { 1.0 },
            if hovered {
                ex.accent
            } else {
                ui.visuals().weak_text_color()
            },
        );
        ui.painter()
            .rect_stroke(rect.shrink(2.0), 6.0, stroke, egui::StrokeKind::Inside);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Move here",
            egui::FontId::proportional(12.0),
            ui.visuals().weak_text_color(),
        );
        if hovered && ui.input(|i| i.pointer.any_released()) {
            events.push(StepsEvent::Move(from, to));
            *drag = None;
        }
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), Sense::hover());
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        egui::Stroke::new(1.5, ui.visuals().weak_text_color()),
    );
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 24.0), Sense::hover());
    let plus = ui
        .put(
            egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0)),
            egui::Button::new(RichText::new("+").strong()).corner_radius(12.0),
        )
        .on_hover_text("Add a step here");
    if plus.clicked() {
        events.push(StepsEvent::Plus(to));
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), Sense::hover());
    ui.painter().arrow(
        rect.center_top(),
        egui::vec2(0.0, rect.height()),
        egui::Stroke::new(1.5, ui.visuals().weak_text_color()),
    );
}

fn card(
    ui: &mut Ui,
    w: f32,
    c: &Card,
    selected: bool,
    drag: &mut Option<StepAt>,
    events: &mut Vec<StepsEvent>,
) -> egui::Response {
    let ex = theme::extras(ui);
    let stroke = if c.problem.is_some() {
        egui::Stroke::new(2.0, ui.visuals().error_fg_color)
    } else if selected {
        egui::Stroke::new(2.0, ex.accent)
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    let dragged = drag.as_ref() == Some(&c.at);
    let frame = egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(stroke)
        .corner_radius(8.0)
        .inner_margin(10.0)
        .multiply_with_opacity(if dragged { 0.4 } else { 1.0 })
        .show(ui, |ui| {
            ui.set_width(w - FRAME);
            ui.horizontal(|ui| {
                app_mark(ui, &c.app);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&c.name).strong());
                    if !c.sentence.is_empty() && c.sentence != c.name {
                        ui.label(RichText::new(&c.sentence).weak());
                    }
                    for b in &c.badges {
                        let color = if b.bad {
                            ui.visuals().error_fg_color
                        } else {
                            ui.visuals().weak_text_color()
                        };
                        match &b.control {
                            Some((n, label)) => {
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.label(RichText::new(&b.text).small().color(color));
                                    ui.label(number_text(ui, *n));
                                    ui.label(RichText::new(label).small().color(color));
                                });
                            }
                            None => {
                                ui.label(RichText::new(&b.text).small().color(color));
                            }
                        }
                    }
                    if let Some(p) = &c.problem {
                        ui.label(RichText::new(p).small().color(ui.visuals().error_fg_color));
                    }
                });
            });
        });
    let id = ui.id().with(("card", c.at.before, c.at.index));
    let resp = ui.interact(frame.response.rect, id, Sense::click_and_drag());
    if resp.clicked() || resp.drag_started() {
        events.push(StepsEvent::Select(c.at.clone()));
    }
    if resp.drag_started() {
        *drag = Some(c.at.clone());
    }
    if dragged {
        if let Some(pos) = ui.ctx().pointer_interact_pos() {
            egui::Area::new(egui::Id::new("dragged-card"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + egui::vec2(12.0, 12.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new(&c.name).strong());
                    });
                });
        }
    }
    resp.context_menu(|ui| {
        if ui.button("Duplicate").clicked() {
            events.push(StepsEvent::Duplicate(c.at.clone()));
            ui.close();
        }
        if ui.button("Delete").clicked() {
            events.push(StepsEvent::Delete(c.at.clone()));
            ui.close();
        }
    });
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contrast;

    #[test]
    fn app_initials_read_on_their_colours() {
        for c in APP_HUES {
            let r = contrast::ratio(APP_INITIAL, c);
            assert!(r >= contrast::MIN, "{c:?} is {r:.2}:1");
        }
    }
}

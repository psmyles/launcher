//! The form (M6): a file's layout drawn with egui, from the values in a [`Form`] and the Lua
//! results in a [`Live`]. It changes nothing itself: what the user does comes back as
//! [`FormEvent`]s for the document to apply.
//!
//! **Layout.** A label column and a field column (`label-width`, `field-width`). Checkboxes show
//! their label beside the box; a row's first item starts in the label column and the rest follow
//! on the line; inside a group, labels are indented and fields keep the form's field column.
//!
//! **Width.** The window fits its content (N15), so the form reports its natural width
//! ([`FormUi::need`]): where its widgets end, counting a fill-width widget only at its minimum
//! (the field column). Otherwise a fill-width widget would hold an auto-fit window at whatever
//! width it once had (LESSONS L2.13).
//!
//! **Tools (format 2).** The label column is as wide as the longest label (none without labels),
//! controls stretch with the window, and a row's controls share its width equally, each with its
//! label beside it. Every button looks the same. In the editor (Pick mode) controls are dragged to
//! arrange them: above, below, or beside another ([`FormEvent::Place`]).
//!
//! **Conditions.** `visible-if` shows until evaluated; `enabled-if` stays off until evaluated, so a
//! button can't be pressed before its condition is known.

use std::collections::{BTreeMap, BTreeSet};

use egui::{RichText, Ui, Vec2};
use launcher_core::config::tool_edit::Side;
use launcher_core::config::*;
use launcher_core::dynval::Dyn;
use launcher_core::eval::{option_scope, render, Live, OptionItem, Which};
use launcher_core::form::Form;

use crate::ui::theme;

/// What the user did.
pub enum FormEvent {
    Set(String, Dyn),
    /// A computed list's refresh button.
    Refresh(String),
    /// A file or folder field's Browse button.
    Browse(String),
    /// A button: run its action (after its `confirm=`).
    Press(String),
    /// Run an action (the button row of actions no button starts).
    Action(String),
    /// A button's "Dry run" menu entry.
    DryRun(String),
    /// The editor's Pick mode: this layout item or control was clicked.
    Pick(NodePath),
    /// The editor's Pick mode (tools): a control was dragged next to another.
    Place {
        from: NodePath,
        to: NodePath,
        side: Side,
    },
}

/// Everything the form reads besides the layout.
pub struct FormCx<'a> {
    pub form: &'a Form,
    pub live: &'a Live,
    pub env: &'a BTreeMap<String, String>,
    /// Computed lists being worked out again.
    pub refreshing: &'a BTreeSet<String>,
    /// The file's load-time values are in (computed lists show a spinner until then).
    pub loaded: bool,
    /// A run is going: nothing can be changed.
    pub locked: bool,
    /// Why actions can't run (a field that needs attention).
    pub blocked: Option<&'a str>,
    /// The editor's Pick mode: a click selects an item instead of using it.
    pub pick: bool,
    /// The editor's selection, outlined.
    pub selected: Option<&'a NodePath>,
    /// The editor's numbers for the controls (by id), shown in a badge before each.
    pub numbers: Option<&'a std::collections::HashMap<String, usize>>,
}

/// A control's number badge's width.
const BADGE_W: f32 = 20.0;

/// The numbers' colours, one per number (round again after the last), so a control's number
/// looks the same wherever the editor names it and two numbers side by side look different.
/// Each reads at `contrast::MIN` on the badge (the theme's `extreme_bg_color`).
const BADGE_DARK: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0x7a, 0xb8, 0xff),
    egui::Color32::from_rgb(0x7d, 0xd8, 0x8a),
    egui::Color32::from_rgb(0xff, 0xad, 0x66),
    egui::Color32::from_rgb(0xff, 0x8f, 0xc8),
    egui::Color32::from_rgb(0x5f, 0xd4, 0xd0),
    egui::Color32::from_rgb(0xe8, 0xd3, 0x6a),
    egui::Color32::from_rgb(0xc0, 0x9c, 0xff),
    egui::Color32::from_rgb(0xff, 0x8a, 0x80),
];
const BADGE_LIGHT: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0x0b, 0x5c, 0xad),
    egui::Color32::from_rgb(0x1d, 0x6b, 0x2a),
    egui::Color32::from_rgb(0x8f, 0x4a, 0x00),
    egui::Color32::from_rgb(0xa3, 0x23, 0x6b),
    egui::Color32::from_rgb(0x0c, 0x66, 0x63),
    egui::Color32::from_rgb(0x6e, 0x5a, 0x00),
    egui::Color32::from_rgb(0x6a, 0x3f, 0xc0),
    egui::Color32::from_rgb(0xb0, 0x26, 0x2a),
];

/// Number `n`'s colour.
fn badge_color(ui: &Ui, n: usize) -> egui::Color32 {
    let colours = if ui.visuals().dark_mode {
        &BADGE_DARK
    } else {
        &BADGE_LIGHT
    };
    colours[n.saturating_sub(1) % colours.len()]
}

/// A control's number, in a small badge in its own colour (the editor's window, and wherever
/// the editor names that control).
pub fn number_badge(ui: &mut Ui, n: usize) -> egui::Response {
    let h = ui.spacing().interact_size.y;
    let (rect, r) = ui.allocate_exact_size(Vec2::new(BADGE_W, h), egui::Sense::hover());
    let v = ui.visuals();
    let pill = egui::Rect::from_center_size(rect.center(), Vec2::new(BADGE_W, 16.0));
    ui.painter().rect_filled(pill, 4.0, v.extreme_bg_color);
    ui.painter().text(
        pill.center(),
        egui::Align2::CENTER_CENTER,
        n.to_string(),
        egui::TextStyle::Small.resolve(ui.style()),
        badge_color(ui, n),
    );
    r
}

/// A control's number as text, to sit inside other text: " 2 " as its badge shows it.
pub fn number_text(ui: &Ui, n: usize) -> RichText {
    RichText::new(format!(" {n} "))
        .small()
        .color(badge_color(ui, n))
        .background_color(ui.visuals().extreme_bg_color)
}

pub struct FormUi<'a> {
    cx: FormCx<'a>,
    label_w: f32,
    field_w: f32,
    /// The narrowest a stretching control gets (the field column; less in a row).
    min_field: f32,
    /// Drawing one of a row's equal columns (tools).
    in_row: bool,
    /// The form's left edge.
    left: f32,
    /// Where fields start (screen x): the same inside groups, per column in columns.
    field_x: f32,
    /// The natural width from the left edge, measured this frame.
    pub need: f32,
    pub events: Vec<FormEvent>,
    /// Where each item was drawn (for Pick mode and the selection outline).
    rects: Vec<(NodePath, egui::Rect)>,
}

/// The Browse button's width.
const BROWSE_W: f32 = 76.0;

/// Where a control sits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// On a line of its own: label column, then field column.
    Alone,
    /// First in a row: starts in the label column.
    RowFirst,
    /// Later in a row: follows on the line.
    RowNext,
}

impl<'a> FormUi<'a> {
    pub fn new(cx: FormCx<'a>) -> FormUi<'a> {
        let w = &cx.form.config().window;
        FormUi {
            label_w: w.label_width,
            field_w: w.field_width,
            min_field: w.field_width,
            in_row: false,
            cx,
            left: 0.0,
            field_x: 0.0,
            need: 0.0,
            events: Vec::new(),
            rects: Vec::new(),
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        // In the editor a press picks or drags an item: it doesn't select a label's text.
        if self.cx.pick {
            ui.style_mut().interaction.selectable_labels = false;
        }
        let config = self.cx.form.config().clone();
        // A tool's label column fits its labels (unless the file sets its width).
        if self.tool() && config.window.label_width == WindowSpec::default().label_width {
            let font = egui::TextStyle::Body.resolve(ui.style());
            let mut widest: f32 = 0.0;
            label_texts(&config, &config.layout, &mut |t| {
                let w = ui
                    .painter()
                    .layout_no_wrap(t.to_string(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x;
                widest = widest.max(w);
            });
            self.label_w = widest.ceil();
        }
        self.left = ui.cursor().min.x;
        self.field_x = self.left + self.gutter(ui) + self.label_w + self.label_gap(ui);
        self.items(ui, &config.layout);
        self.pick_and_outline(ui);
    }

    /// A tool (format 2), not a first-format file.
    fn tool(&self) -> bool {
        self.cx.form.config().format >= 2
    }

    /// The room before each line for a control's number (the editor only).
    fn gutter(&self, ui: &Ui) -> f32 {
        if self.cx.numbers.is_some() {
            BADGE_W + ui.spacing().item_spacing.x
        } else {
            0.0
        }
    }

    /// The gap after the label column (none when there are no labels).
    fn label_gap(&self, ui: &Ui) -> f32 {
        if self.label_w > 0.0 {
            theme::extras(ui).label_gap
        } else {
            0.0
        }
    }

    /// Outline the selection; in Pick mode, outline what's under the pointer and select it on a
    /// click (the innermost item), and in a tool, drag controls to arrange them.
    fn pick_and_outline(&mut self, ui: &mut Ui) {
        if self.cx.pick && self.tool() && self.drag(ui) {
            return;
        }
        let ex = theme::extras(ui);
        let accent = ex.accent;
        let painter = ui.painter();
        if let Some(sel) = self.cx.selected {
            for (path, r) in &self.rects {
                if path == sel {
                    painter.rect_stroke(
                        r.expand(3.0),
                        4.0,
                        egui::Stroke::new(ex.outline_width, accent),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
        if !self.cx.pick {
            return;
        }
        let Some(pos) = pointer_here(ui) else {
            return;
        };
        let under = self
            .rects
            .iter()
            .filter(|(_, r)| r.expand(3.0).contains(pos))
            .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
            .cloned();
        if let Some((path, r)) = under {
            painter.rect_stroke(
                r.expand(3.0),
                4.0,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.7)),
                egui::StrokeKind::Outside,
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if ui.input(|i| i.pointer.primary_clicked()) {
                self.events.push(FormEvent::Pick(path));
            }
        }
    }

    /// Dragging a control onto another: press on one, move, let go on the other's edge (left or
    /// right quarter: beside it; else above or below). True while a drag is going.
    fn drag(&mut self, ui: &mut Ui) -> bool {
        let press_id = egui::Id::new("form-press");
        let drag_id = egui::Id::new("form-drag");
        // Everything drawn can be dragged and dropped on, except rows (their controls are).
        let mut kinds = std::collections::HashMap::new();
        let config = self.cx.form.config().clone();
        item_kinds(&config, &config.layout, &mut kinds);
        let controls: Vec<(NodePath, egui::Rect)> = self
            .rects
            .iter()
            .filter(|(p, _)| kinds.get(p).is_some_and(|k| *k != Kind::Row))
            .cloned()
            .collect();
        let (pressed, down, released, origin, pos) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.press_origin(),
                i.pointer.hover_pos(),
            )
        });
        let under = |at: egui::Pos2, skip: Option<&NodePath>| {
            controls
                .iter()
                .filter(|(p, r)| {
                    // Not the dragged item, nor anything inside it.
                    skip.is_none_or(|s| !p.starts_with(s)) && r.expand(3.0).contains(at)
                })
                .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
                .cloned()
        };
        if pressed {
            let hit = pointer_here(ui)
                .and_then(|p| under(p, None))
                .map(|(p, _)| p);
            ui.data_mut(|d| {
                d.insert_temp(press_id, hit);
                d.remove::<NodePath>(drag_id);
            });
        }
        let mut dragging: Option<NodePath> = ui.data(|d| d.get_temp(drag_id));
        if dragging.is_none() && down {
            let pressed_on: Option<NodePath> = ui.data(|d| d.get_temp(press_id)).flatten();
            let moved = origin.zip(pos).is_some_and(|(o, p)| o.distance(p) > 6.0);
            if let (Some(p), true) = (pressed_on, moved) {
                ui.data_mut(|d| d.insert_temp(drag_id, p.clone()));
                dragging = Some(p);
            }
        }
        let Some(from) = dragging else {
            return false;
        };
        let accent = theme::extras(ui).accent;
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some((_, r)) = controls.iter().find(|(p, _)| *p == from) {
            ui.painter()
                .rect_filled(*r, 4.0, accent.gamma_multiply(0.15));
        }
        let target = pos.and_then(|p| under(p, Some(&from)).map(|(path, r)| (path, r, p)));
        let mut drop = None;
        if let Some((to, r, p)) = target {
            let x = (p.x - r.left()) / r.width().max(1.0);
            // A section: near its top or bottom edge goes above or below it, else inside.
            let edge = (r.height() * 0.25).min(14.0);
            let side = if kinds.get(&to) == Some(&Kind::Section) {
                if p.y < r.top() + edge {
                    Side::Above
                } else if p.y > r.bottom() - edge {
                    Side::Below
                } else {
                    Side::Inside
                }
            } else if x < 0.25 {
                Side::Left
            } else if x > 0.75 {
                Side::Right
            } else if p.y < r.center().y {
                Side::Above
            } else {
                Side::Below
            };
            let r = r.expand(3.0);
            let line = match side {
                Side::Left => Some([r.left_top(), r.left_bottom()]),
                Side::Right => Some([r.right_top(), r.right_bottom()]),
                Side::Above => Some([r.left_top(), r.right_top()]),
                Side::Below => Some([r.left_bottom(), r.right_bottom()]),
                Side::Inside => None,
            };
            match line {
                Some(l) => {
                    ui.painter().line_segment(l, egui::Stroke::new(3.0, accent));
                }
                None => {
                    ui.painter().rect_stroke(
                        r,
                        4.0,
                        egui::Stroke::new(2.0, accent),
                        egui::StrokeKind::Outside,
                    );
                }
            }
            drop = Some((to, side));
        }
        if released || !down {
            if let Some((to, side)) = drop {
                self.events.push(FormEvent::Place { from, to, side });
            }
            ui.data_mut(|d| {
                d.remove::<NodePath>(drag_id);
                d.remove::<Option<NodePath>>(press_id);
            });
        } else {
            ui.ctx().request_repaint();
        }
        true
    }

    /// A template or `(lua)` text; a `(lua)` one not yet worked out (or failed) is empty.
    fn text(&self, v: &Val, node: &NodeRef, prop: &str) -> String {
        match v {
            Val::Text(t) => render(t, self.cx.form.snapshot(), self.cx.env),
            Val::Lua(_) => self
                .cx
                .live
                .texts
                .get(&(node.path.clone(), prop.to_string()))
                .and_then(|r| r.as_ref().ok())
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn visible(&self, node: &NodeRef, cond: &Option<LuaSrc>) -> bool {
        match cond {
            None => true,
            Some(_) => !matches!(
                self.cx
                    .live
                    .conditions
                    .get(&(node.path.clone(), Which::Visible)),
                Some(Ok(false))
            ),
        }
    }

    fn enabled(&self, c: &Control) -> bool {
        match &c.enabled_if {
            None => true,
            Some(_) => matches!(
                self.cx
                    .live
                    .conditions
                    .get(&(c.node.path.clone(), Which::Enabled)),
                Some(Ok(true))
            ),
        }
    }

    /// Something ends at screen x `right`.
    fn reach(&mut self, right: f32) {
        self.need = self.need.max(right - self.left);
    }

    fn items(&mut self, ui: &mut Ui, list: &[Item]) {
        for it in list {
            self.item(ui, it);
        }
    }

    fn item(&mut self, ui: &mut Ui, it: &Item) {
        let rect = ui.scope(|ui| self.item_inner(ui, it)).response.rect;
        let config = self.cx.form.config().clone();
        let path = match it {
            Item::Control(i) => Some(config.controls[*i].node.path.clone()),
            Item::Group { node, .. }
            | Item::Tabs { node, .. }
            | Item::Columns { node, .. }
            | Item::Row { node, .. }
            | Item::Separator { node, .. }
            | Item::Spacer { node, .. }
            | Item::Note { node, .. } => Some(node.path.clone()),
        };
        if let Some(p) = path {
            if rect.width() > 0.0 && rect.height() > 0.0 {
                self.rects.push((p, rect));
            }
        }
    }

    fn item_inner(&mut self, ui: &mut Ui, it: &Item) {
        let config = self.cx.form.config().clone();
        match it {
            Item::Control(i) => {
                let c = &config.controls[*i];
                if !self.visible(&c.node, &c.visible_if) {
                    return;
                }
                ui.horizontal(|ui| self.control(ui, c, Place::Alone));
                self.problem_line(ui, c);
            }
            Item::Row {
                visible_if,
                items,
                node,
            } => {
                if !self.visible(node, visible_if) {
                    return;
                }
                let shown: Vec<&Item> = items
                    .iter()
                    .filter(|it| match it {
                        Item::Control(i) => {
                            let c = &config.controls[*i];
                            self.visible(&c.node, &c.visible_if)
                        }
                        _ => true,
                    })
                    .collect();
                if self.tool() && !shown.is_empty() {
                    self.row_columns(ui, &shown);
                    for it in shown {
                        if let Item::Control(i) = it {
                            self.problem_line(ui, &config.controls[*i]);
                        }
                    }
                    return;
                }
                ui.horizontal(|ui| {
                    for (k, it) in shown.iter().enumerate() {
                        match it {
                            Item::Control(i) => {
                                let place = if k == 0 {
                                    Place::RowFirst
                                } else {
                                    Place::RowNext
                                };
                                self.control(ui, &config.controls[*i], place)
                            }
                            // A container inside a row just follows on the line.
                            other => {
                                ui.vertical(|ui| self.item(ui, other));
                            }
                        }
                    }
                });
                for it in shown {
                    if let Item::Control(i) = it {
                        self.problem_line(ui, &config.controls[*i]);
                    }
                }
            }
            Item::Group {
                title,
                visible_if,
                items,
                node,
            } => {
                if !self.visible(node, visible_if) {
                    return;
                }
                if let Some(t) = title {
                    let t = self.text(t, node, "title");
                    theme::gap_small(ui);
                    ui.label(RichText::new(t).weak());
                }
                let frame = egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(theme::extras(ui).group_padding as i8));
                let pad = 10.0 + 1.0;
                let before = std::mem::take(&mut self.need);
                frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.items(ui, items);
                });
                // The box's padding on the right counts toward the natural width.
                self.need = before.max(self.need + pad);
            }
            Item::Tabs { tabs, node } => {
                let shown: Vec<&Tab> = tabs
                    .iter()
                    .filter(|t| self.visible(&t.node, &t.visible_if))
                    .collect();
                if shown.is_empty() {
                    return;
                }
                let id = ui.id().with(("tabs", &node.path));
                let mut sel = ui.data_mut(|d| *d.get_persisted_mut_or_default::<usize>(id));
                if sel >= shown.len() {
                    sel = 0;
                }
                ui.horizontal(|ui| {
                    for (k, t) in shown.iter().enumerate() {
                        let title = self.text(&t.title, &t.node, "title");
                        if ui.selectable_label(k == sel, title).clicked() {
                            sel = k;
                        }
                    }
                });
                ui.data_mut(|d| d.insert_persisted(id, sel));
                ui.separator();
                self.items(ui, &shown[sel].items);
            }
            Item::Columns { columns, .. } => {
                let n = columns.len().max(1);
                let spacing = ui.spacing().item_spacing.x;
                let gap = self.label_gap(ui);
                let one = self.label_w + gap + self.field_w;
                let (left, field_x, need) = (self.left, self.field_x, self.need);
                ui.columns(n, |cols| {
                    for (k, items) in columns.iter().enumerate() {
                        self.left = cols[k].cursor().min.x;
                        self.field_x = self.left + self.label_w + gap;
                        self.items(&mut cols[k], items);
                    }
                });
                (self.left, self.field_x, self.need) = (left, field_x, need);
                self.reach(left + n as f32 * one + (n - 1) as f32 * spacing);
            }
            Item::Separator { title, node } => {
                theme::gap_small(ui);
                match title {
                    Some(t) => {
                        let t = self.text(t, node, "title");
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(t).weak());
                            ui.add(egui::Separator::default().horizontal());
                        });
                    }
                    None => {
                        ui.separator();
                    }
                }
            }
            Item::Spacer { height, .. } => ui.add_space(*height),
            Item::Note {
                text,
                style,
                visible_if,
                node,
            } => {
                if !self.visible(node, visible_if) {
                    return;
                }
                let t = self.text(text, node, "text");
                let v = ui.visuals();
                let color = match style {
                    NoteStyle::Normal => None,
                    NoteStyle::Muted => Some(v.weak_text_color()),
                    NoteStyle::Warning => Some(v.warn_fg_color),
                    NoteStyle::Error => Some(v.error_fg_color),
                };
                let mut rt = RichText::new(t);
                if let Some(c) = color {
                    rt = rt.color(c);
                }
                let width = self.label_w + self.label_gap(ui) + self.field_w;
                let gutter = self.gutter(ui);
                ui.horizontal_top(|ui| {
                    // Text lines up with the controls' labels (after their numbers).
                    ui.add_space(gutter);
                    ui.scope(|ui| {
                        ui.set_max_width(ui.available_width().max(width));
                        ui.add(egui::Label::new(rt).wrap());
                    });
                });
            }
        }
    }

    /// A tool's row: its controls share the width equally, each with its label beside it.
    fn row_columns(&mut self, ui: &mut Ui, shown: &[&Item]) {
        let config = self.cx.form.config().clone();
        let n = shown.len();
        let spacing = ui.spacing().item_spacing.x;
        let saved = (
            self.left,
            self.field_x,
            self.need,
            self.min_field,
            self.in_row,
        );
        let mut widest: f32 = 0.0;
        ui.columns(n, |cols| {
            for (k, it) in shown.iter().enumerate() {
                let col = &mut cols[k];
                self.left = col.cursor().min.x;
                self.field_x = self.left;
                self.need = 0.0;
                self.min_field = 120.0;
                self.in_row = true;
                match it {
                    Item::Control(i) => {
                        col.horizontal(|ui| self.control(ui, &config.controls[*i], Place::RowNext));
                    }
                    other => self.item(col, other),
                }
                widest = widest.max(self.need);
            }
        });
        (
            self.left,
            self.field_x,
            self.need,
            self.min_field,
            self.in_row,
        ) = saved;
        self.reach(self.left + n as f32 * widest + (n - 1) as f32 * spacing);
    }

    /// Where a control's field ends up, and whether it stretches to the form's right edge.
    fn field_width(&self, ui: &Ui, c: &Control) -> (Option<f32>, bool) {
        match c.width {
            Width::Field => (Some(self.field_w), false),
            Width::Px(w) => (Some(w), false),
            Width::Auto => (None, false),
            // It takes the room there is (the window fits `min_field`, so it has that much
            // unless something narrows the form, such as the log beside it).
            Width::Fill => (Some(ui.available_width().max(0.0)), true),
        }
    }

    /// Move on to the field column (a label wider than its column pushes the field right).
    fn to_field(&self, ui: &mut Ui) {
        let at = ui.cursor().min.x;
        if at < self.field_x {
            ui.add_space(self.field_x - at);
        }
    }

    /// The label column: the label, or nothing, then on to the field column.
    fn label_cell(&mut self, ui: &mut Ui, text: Option<&str>) {
        if let Some(t) = text {
            ui.label(t);
        }
        self.to_field(ui);
    }

    fn control(&mut self, ui: &mut Ui, c: &Control, place: Place) {
        // The editor's number for it, before everything else on its line.
        if let Some(numbers) = self.cx.numbers {
            match numbers.get(&c.id) {
                Some(n) => {
                    number_badge(ui, *n);
                }
                None => ui.add_space(self.gutter(ui)),
            }
        }
        let label = c.label.as_deref();
        let own_label = matches!(c.kind, ControlKind::Checkbox | ControlKind::Button { .. });
        let fill_button =
            matches!(c.kind, ControlKind::Button { .. }) && matches!(c.width, Width::Fill);
        match place {
            // A fill-width button spans both columns.
            _ if fill_button && place != Place::RowNext => {}
            // A row's first checkbox sits in the label column.
            Place::RowFirst if matches!(c.kind, ControlKind::Checkbox) => {}
            Place::Alone | Place::RowFirst => {
                self.label_cell(ui, if own_label { None } else { label });
            }
            Place::RowNext => {
                if let (false, Some(l)) = (own_label, label) {
                    ui.label(l);
                }
            }
        }
        let enabled = self.enabled(c) && !self.cx.locked;
        let start = ui.cursor().min.x;
        let r = ui
            .add_enabled_ui(enabled && !self.cx.pick, |ui| self.widget(ui, c))
            .inner;
        let Some(r) = r else { return };
        // A row's controls can be picked one by one.
        if place != Place::Alone {
            self.rects.push((c.node.path.clone(), r.rect));
        }
        let (_, fill) = self.field_width(ui, c);
        if fill_button && !self.in_row {
            self.reach(self.field_x + self.field_w);
        } else if fill {
            self.reach(start + self.min_field);
        } else {
            // Everything on the line so far (a refresh button beside a list, say).
            self.reach(ui.min_rect().right());
        }
        // A row's first checkbox fills the label column, so the next item starts at the field.
        if place == Place::RowFirst && matches!(c.kind, ControlKind::Checkbox) {
            self.to_field(ui);
        }
        let tooltip = c.tooltip.as_ref().map(|t| self.text(t, &c.node, "tooltip"));
        let why_off = if !self.enabled(c) {
            c.disabled_tooltip
                .as_ref()
                .map(|t| self.text(t, &c.node, "disabled-tooltip"))
        } else {
            None
        };
        let r = match (why_off, tooltip) {
            (Some(t), _) if !t.is_empty() => r.on_disabled_hover_text(t),
            (_, Some(t)) if !t.is_empty() => r.on_hover_text(t),
            _ => r,
        };
        if self.cx.form.preset_holds(&c.id) {
            r.on_hover_text(
                "Chosen by a preset. Your own choice isn't remembered while it applies.",
            );
        }
    }

    /// A value's problem under its field (a required field left empty only disables the
    /// actions; the button says why).
    fn problem_line(&mut self, ui: &mut Ui, c: &Control) {
        if self.cx.form.value(&c.id).to_text().trim().is_empty() {
            return;
        }
        if let Some(p) = self.cx.form.check(&c.id) {
            ui.horizontal(|ui| {
                self.label_cell(ui, None);
                ui.label(RichText::new(p).small().color(ui.visuals().error_fg_color));
            });
        }
    }

    fn option_text(&self, c: &Control, o: &OptionItem, i: usize) -> String {
        let ControlKind::Choice {
            options,
            option_label,
            ..
        } = &c.kind
        else {
            return o.name.clone();
        };
        match option_label {
            Some(Val::Text(t)) => {
                let mut scope = option_scope(o, i);
                scope.fields = self.cx.form.snapshot().fields.clone();
                render(t, &scope, self.cx.env)
            }
            Some(Val::Lua(_)) => self
                .cx
                .live
                .texts
                .get(&(c.node.path.clone(), format!("option-label:{i}")))
                .and_then(|r| r.as_ref().ok())
                .cloned()
                .unwrap_or_else(|| o.name.clone()),
            None => match options
                .get(i)
                .and_then(|d| d.label.as_ref().map(|l| (d, l)))
            {
                Some((d, l)) => self.text(l, &d.node, "label"),
                None => o.label.clone().unwrap_or_else(|| o.name.clone()),
            },
        }
    }

    fn set(&mut self, c: &Control, v: Dyn) {
        self.events.push(FormEvent::Set(c.id.clone(), v));
    }

    /// The widget; `None` when there's nothing to hover.
    fn widget(&mut self, ui: &mut Ui, c: &Control) -> Option<egui::Response> {
        let (w, _) = self.field_width(ui, c);
        let value = self.cx.form.value(&c.id);
        let h = ui.spacing().interact_size.y;
        match &c.kind {
            ControlKind::Choice {
                style, options_lua, ..
            } => {
                let opts = self.cx.form.options(&c.id);
                let computed = options_lua.is_some();
                let result = self.cx.form.results().options.get(&c.id);
                let loading = computed && (!self.cx.loaded || self.cx.refreshing.contains(&c.id));
                let error = match result {
                    Some(Err(e)) if !loading => Some(e.clone()),
                    _ => None,
                };
                let r = if loading {
                    let r = ui.add_sized(
                        [w.unwrap_or(self.field_w), h],
                        egui::Button::new(RichText::new("Loading…").weak()),
                    );
                    ui.add(egui::Spinner::new());
                    r
                } else {
                    match style {
                        ChoiceStyle::Dropdown => {
                            let cur = value.to_text();
                            let idx = opts.iter().position(|o| o.value == cur);
                            let shown = idx
                                .map(|i| self.option_text(c, &opts[i], i))
                                .unwrap_or_default();
                            let mut picked = None;
                            let mut combo = egui::ComboBox::from_id_salt(("dropdown", &c.id))
                                .selected_text(shown);
                            if let Some(w) = w {
                                combo = combo.width(w);
                            }
                            let r = combo
                                .show_ui(ui, |ui| {
                                    for (i, o) in opts.iter().enumerate() {
                                        let t = self.option_text(c, o, i);
                                        if ui.selectable_label(Some(i) == idx, t).clicked() {
                                            picked = Some(o.value.clone());
                                        }
                                    }
                                })
                                .response;
                            if let Some(v) = picked {
                                self.set(c, Dyn::Str(v));
                            }
                            r
                        }
                        ChoiceStyle::Radio { horizontal } => {
                            let cur = value.to_text();
                            let mut picked = None;
                            let inner = |ui: &mut Ui| {
                                for (i, o) in opts.iter().enumerate() {
                                    let t = self.option_text(c, o, i);
                                    if ui.radio(o.value == cur, t).clicked() {
                                        picked = Some(o.value.clone());
                                    }
                                }
                            };
                            let r = if *horizontal {
                                ui.horizontal(inner).response
                            } else {
                                ui.vertical(inner).response
                            };
                            if let Some(v) = picked {
                                self.set(c, Dyn::Str(v));
                            }
                            r
                        }
                        ChoiceStyle::Checklist => {
                            let ticked: Vec<String> = match &value {
                                Dyn::List(l) => l.iter().map(Dyn::to_text).collect(),
                                _ => Vec::new(),
                            };
                            let mut changed = None;
                            let r = ui
                                .vertical(|ui| {
                                    for (i, o) in opts.iter().enumerate() {
                                        let mut on = ticked.contains(&o.value);
                                        let t = self.option_text(c, o, i);
                                        if ui.checkbox(&mut on, t).changed() {
                                            // Keep the options' order.
                                            let list: Vec<Dyn> = opts
                                                .iter()
                                                .filter(|x| {
                                                    if x.value == o.value {
                                                        on
                                                    } else {
                                                        ticked.contains(&x.value)
                                                    }
                                                })
                                                .map(|x| Dyn::str(x.value.clone()))
                                                .collect();
                                            changed = Some(Dyn::List(list));
                                        }
                                    }
                                })
                                .response;
                            if let Some(v) = changed {
                                self.set(c, v);
                            }
                            r
                        }
                    }
                };
                if computed
                    && !loading
                    && ui
                        .button("⟳")
                        .on_hover_text("Work the list out again")
                        .clicked()
                {
                    self.events.push(FormEvent::Refresh(c.id.clone()));
                }
                if let Some(e) = error {
                    ui.label(RichText::new("⚠").color(ui.visuals().error_fg_color))
                        .on_hover_text(e);
                }
                Some(r)
            }
            ControlKind::Checkbox => {
                let mut on = value.truthy();
                let text = c.label.clone().unwrap_or_default();
                let r = ui.checkbox(&mut on, text);
                if r.changed() {
                    self.set(c, Dyn::Bool(on));
                }
                Some(r)
            }
            ControlKind::Slider {
                min,
                max,
                step,
                suffix,
            } => {
                let mut n = match value {
                    Dyn::Num(n) => n,
                    _ => *min,
                };
                let before = n;
                let width = w.unwrap_or(self.field_w);
                // The number box beside the track takes about 60 px.
                ui.spacing_mut().slider_width = (width - 60.0).max(60.0);
                let mut s = egui::Slider::new(&mut n, *min..=*max);
                if *step > 0.0 {
                    s = s.step_by(*step);
                }
                // As many decimals as the step and the ends have.
                let decimals = [*step, *min, *max]
                    .iter()
                    .map(|v| {
                        let t = launcher_core::dynval::fmt_num(*v);
                        t.split_once('.').map_or(0, |(_, f)| f.len())
                    })
                    .max()
                    .unwrap_or(0);
                s = s.fixed_decimals(decimals);
                if let Some(sfx) = suffix {
                    s = s.suffix(sfx.clone());
                }
                let r = ui.add(s);
                if n != before {
                    self.set(c, Dyn::Num(n));
                }
                Some(r)
            }
            ControlKind::Number {
                min,
                max,
                step,
                decimals,
            } => {
                let mut n = match value {
                    Dyn::Num(n) => n,
                    _ => 0.0,
                };
                let before = n;
                let mut d = egui::DragValue::new(&mut n)
                    .speed(if *step > 0.0 { *step } else { 1.0 })
                    .fixed_decimals(*decimals as usize);
                d = d.range(min.unwrap_or(f64::NEG_INFINITY)..=max.unwrap_or(f64::INFINITY));
                let r = ui.add_sized([w.unwrap_or(self.field_w), h], d);
                if n != before {
                    self.set(c, Dyn::Num(n));
                }
                Some(r)
            }
            ControlKind::Text {
                multiline,
                lines,
                password,
                placeholder,
                ..
            } => {
                let mut s = value.to_text();
                let mut t = if *multiline {
                    egui::TextEdit::multiline(&mut s).desired_rows(*lines as usize)
                } else {
                    egui::TextEdit::singleline(&mut s)
                }
                .password(*password);
                if let Some(p) = placeholder {
                    t = t.hint_text(p.clone());
                }
                if let Some(w) = w {
                    t = t.desired_width(w);
                }
                let r = ui.add(t);
                if r.changed() {
                    self.set(c, Dyn::Str(s));
                }
                Some(r)
            }
            ControlKind::File { placeholder, .. } | ControlKind::Folder { placeholder, .. } => {
                let mut s = value.to_text();
                let spacing = ui.spacing().item_spacing.x;
                let width = w.unwrap_or(self.field_w) - BROWSE_W - spacing;
                let mut t = egui::TextEdit::singleline(&mut s).desired_width(width.max(60.0));
                if let Some(p) = placeholder {
                    t = t.hint_text(p.clone());
                }
                let r = ui.add(t);
                if r.changed() {
                    self.set(c, Dyn::Str(s));
                }
                let b = ui.add_sized([BROWSE_W, h], egui::Button::new("Browse…"));
                if b.clicked() {
                    self.events.push(FormEvent::Browse(c.id.clone()));
                }
                Some(r.union(b))
            }
            ControlKind::Display { text, copyable } => {
                let t = self.text(text, &c.node, "text");
                let width = w.unwrap_or(self.field_w);
                let r = ui
                    .scope(|ui| {
                        ui.set_max_width(width);
                        ui.add(egui::Label::new(t).selectable(*copyable).wrap())
                    })
                    .inner;
                Some(r)
            }
            ControlKind::Button {
                action, primary, ..
            } => {
                let text = c.label.clone().unwrap_or_else(|| c.id.clone());
                // A first-format file's primary button stands out; a tool's buttons are alike.
                let primary = *primary && !self.tool();
                let mut b = if primary {
                    egui::Button::new(RichText::new(text).strong())
                } else {
                    egui::Button::new(text)
                };
                let height = if primary { h + 8.0 } else { h };
                b = b.min_size(Vec2::new(w.unwrap_or(0.0), height));
                let blocked = self.cx.blocked;
                let r = ui.add_enabled(blocked.is_none(), b);
                let r = match blocked {
                    Some(why) => r.on_disabled_hover_text(why),
                    None => r,
                };
                if r.clicked() {
                    self.events.push(FormEvent::Press(c.id.clone()));
                }
                let action = action.clone();
                r.context_menu(|ui| {
                    if ui
                        .button("Preview")
                        .on_hover_text(
                            "List what this would do with the current values, without doing it",
                        )
                        .clicked()
                    {
                        self.events.push(FormEvent::DryRun(action.clone()));
                        ui.close();
                    }
                });
                Some(r)
            }
        }
    }
}

/// Controls that are shown (their own `visible-if` and every enclosing one), in form order.
pub fn shown_controls(config: &Config, live: &Live) -> Vec<String> {
    fn visible(live: &Live, node: &NodeRef, cond: &Option<LuaSrc>) -> bool {
        cond.is_none()
            || !matches!(
                live.conditions.get(&(node.path.clone(), Which::Visible)),
                Some(Ok(false))
            )
    }
    fn walk(config: &Config, live: &Live, items: &[Item], out: &mut Vec<String>) {
        for it in items {
            match it {
                Item::Control(i) => {
                    let c = &config.controls[*i];
                    if visible(live, &c.node, &c.visible_if) {
                        out.push(c.id.clone());
                    }
                }
                Item::Group {
                    visible_if,
                    items,
                    node,
                    ..
                }
                | Item::Row {
                    visible_if,
                    items,
                    node,
                } => {
                    if visible(live, node, visible_if) {
                        walk(config, live, items, out);
                    }
                }
                Item::Tabs { tabs, .. } => {
                    for t in tabs {
                        if visible(live, &t.node, &t.visible_if) {
                            walk(config, live, &t.items, out);
                        }
                    }
                }
                Item::Columns { columns, .. } => {
                    for c in columns {
                        walk(config, live, c, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(config, live, &config.layout, &mut out);
    out
}

/// The labels drawn in the label column: controls on lines of their own (not checkboxes or
/// buttons, which carry their own), in sections and pages too.
fn label_texts(config: &Config, items: &[Item], f: &mut dyn FnMut(&str)) {
    for it in items {
        match it {
            Item::Control(i) => {
                let c = &config.controls[*i];
                if matches!(c.kind, ControlKind::Checkbox | ControlKind::Button { .. }) {
                    continue;
                }
                if let Some(l) = &c.label {
                    f(l);
                }
            }
            Item::Group { items, .. } => label_texts(config, items, f),
            Item::Tabs { tabs, .. } => {
                for t in tabs {
                    label_texts(config, &t.items, f);
                }
            }
            Item::Columns { columns, .. } => {
                for c in columns {
                    label_texts(config, c, f);
                }
            }
            _ => {}
        }
    }
}

/// Where the pointer is, when nothing is in front of the form there: a dialog (the step picker)
/// or an open list takes the pointer, so Pick neither outlines nor selects through it.
fn pointer_here(ui: &Ui) -> Option<egui::Pos2> {
    let pos = ui.input(|i| i.pointer.hover_pos())?;
    let front = ui.ctx().layer_id_at(pos);
    front.is_none_or(|l| l == ui.layer_id()).then_some(pos)
}

/// What a drawn item is, for dragging in the editor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Row,
    Section,
    Other,
}

fn item_kinds(
    config: &Config,
    items: &[Item],
    out: &mut std::collections::HashMap<NodePath, Kind>,
) {
    for it in items {
        match it {
            Item::Control(i) => {
                out.insert(config.controls[*i].node.path.clone(), Kind::Other);
            }
            Item::Row { node, items, .. } => {
                out.insert(node.path.clone(), Kind::Row);
                item_kinds(config, items, out);
            }
            Item::Group { node, items, .. } => {
                out.insert(node.path.clone(), Kind::Section);
                item_kinds(config, items, out);
            }
            Item::Tabs { node, tabs } => {
                out.insert(node.path.clone(), Kind::Other);
                for t in tabs {
                    item_kinds(config, &t.items, out);
                }
            }
            Item::Columns { node, columns } => {
                out.insert(node.path.clone(), Kind::Other);
                for c in columns {
                    item_kinds(config, c, out);
                }
            }
            Item::Separator { node, .. } | Item::Spacer { node, .. } | Item::Note { node, .. } => {
                out.insert(node.path.clone(), Kind::Other);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contrast;
    use crate::ui::theme::{Theme, BUILT_IN};

    #[test]
    fn every_number_reads_on_its_badge() {
        let t = Theme::parse(BUILT_IN, "theme.kdl").unwrap_or_else(|p| panic!("{p:?}"));
        for (colours, look) in [(&BADGE_DARK, &t.dark), (&BADGE_LIGHT, &t.light)] {
            let bg = look.style.visuals.extreme_bg_color;
            for c in colours {
                let r = contrast::ratio(*c, bg);
                assert!(r >= contrast::MIN, "{c:?} on {bg:?} is {r:.2}:1");
            }
        }
    }
}

//! The console's egui view: draws the visible rows of a [`TermModel`], and turns the mouse and
//! keyboard into selection, scrolling and program input.
//!
//! - Output follows the newest line until the wheel or the scroll bar scrolls back; scrolling to
//!   the bottom (or typing) follows again. The scroll bar is always there, so where the view is
//!   in the output shows at a glance.
//! - Drag selects (double-click a word, triple-click a line). Ctrl+C copies a selection and is
//!   the interrupt without one, as in Windows Terminal.
//! - Typing and paste go to the attached program; the program gets the view's size.

use std::time::Duration;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{CursorShape, NamedColor};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::{input, palette, SharedTerm, TermModel};
use crate::ui::contrast;

/// Space between the view's edge and the text.
const PAD: f32 = 4.0;

/// The scroll bar's width, at the right edge.
const BAR: f32 = 12.0;

/// Draw `term` into all of `ui`'s remaining space. `take_focus`: give it the keyboard now.
pub fn show(ui: &mut egui::Ui, term: &SharedTerm, take_focus: bool) -> egui::Response {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let (cell_w, cell_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let id = response.id;
    if take_focus || response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let focused = response.has_focus();
    if focused {
        // Arrows, Tab and Escape belong to the program, not to egui's focus navigation.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }
    let bar_rect = Rect::from_min_max(Pos2::new(rect.right() - BAR, rect.top()), rect.max);
    let text_rect =
        Rect::from_min_max(rect.min, Pos2::new(bar_rect.left(), rect.bottom())).shrink(PAD);
    let cell = Vec2::new(cell_w, cell_h);
    // Above the view, so it gets the pointer there.
    let bar = ui.interact(bar_rect, id.with("scroll-bar"), Sense::click_and_drag());

    let Ok(mut m) = term.lock() else {
        return response;
    };
    let cols = (text_rect.width() / cell_w).floor() as usize;
    let rows = (text_rect.height() / cell_h).floor() as usize;
    m.resize(cols, rows);

    // The wheel: back through the scrollback, or arrow keys for a full-screen program.
    if response.hovered() || bar.hovered() {
        let lines = wheel_lines(ui, cell_h, rows);
        if lines != 0 {
            let mode = *m.term.mode();
            if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                let key = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                let bytes = key.repeat(lines.unsigned_abs() as usize);
                m.input(&bytes);
            } else {
                m.term.scroll_display(Scroll::Delta(lines));
                m.follow = m.term.grid().display_offset() <= m.newest_offset();
            }
        }
    }

    scroll_bar(ui, &bar, &mut m);
    select(ui, &response, &mut m, text_rect.min, cell);
    if focused {
        keyboard(ui, &mut m);
    }
    response.context_menu(|ui| {
        let selected = m.term.selection_to_string().filter(|s| !s.is_empty());
        if ui
            .add_enabled(selected.is_some(), egui::Button::new("Copy"))
            .clicked()
        {
            if let Some(s) = selected {
                ui.ctx().copy_text(s);
            }
            ui.close();
        }
        if ui.button("Copy all").clicked() {
            ui.ctx().copy_text(m.text());
            ui.close();
        }
    });

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette::BACKGROUND);
    draw(&m, &painter, text_rect.min, cell, &font, focused);
    draw_bar(&painter, &bar, &m);
    response
}

/// Where the scroll bar's thumb is: (how far down, 0 at the oldest output and 1 at the newest;
/// its height as a fraction of the track). None when all the output fits.
fn thumb(m: &TermModel) -> Option<(f32, f32)> {
    let grid = m.term.grid();
    // From the oldest line (`history_size` lines back) to where following puts the view.
    let (top, bottom) = (grid.history_size(), m.newest_offset());
    if top <= bottom {
        return None;
    }
    let range = (top - bottom) as f32;
    let shown = grid.screen_lines() as f32;
    let size = shown / (range + shown);
    let at = (top - grid.display_offset().clamp(bottom, top)) as f32 / range;
    Some((at, size))
}

/// The thumb's rectangle in the track: never too small to grab.
fn thumb_rect(track: Rect, (at, size): (f32, f32)) -> Rect {
    let h = (size * track.height()).max(24.0).min(track.height());
    let top = track.top() + at * (track.height() - h);
    Rect::from_min_size(Pos2::new(track.left(), top), Vec2::new(track.width(), h))
}

/// Drag the thumb, or click the track to move a page.
fn scroll_bar(ui: &egui::Ui, bar: &egui::Response, m: &mut TermModel) {
    let Some(t) = thumb(m) else { return };
    let Some(pos) = bar.interact_pointer_pos() else {
        return;
    };
    let track = bar.rect.shrink2(Vec2::new(3.0, 2.0));
    let thumb = thumb_rect(track, t);
    let grab_id = bar.id.with("grab");
    if bar.drag_started() {
        // Where on the thumb it was taken, so it doesn't jump to the pointer.
        let from = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
        let grab = if thumb.contains(from) {
            from.y - thumb.top()
        } else {
            thumb.height() / 2.0
        };
        ui.data_mut(|d| d.insert_temp(grab_id, grab));
    }
    let grid = m.term.grid();
    let (top, bottom) = (grid.history_size(), m.newest_offset());
    let want = if bar.dragged() {
        let grab = ui.data(|d| d.get_temp::<f32>(grab_id)).unwrap_or(0.0);
        let free = (track.height() - thumb.height()).max(1.0);
        let at = ((pos.y - grab - track.top()) / free).clamp(0.0, 1.0);
        top as f32 - at * (top - bottom) as f32
    } else if bar.clicked() {
        let page = grid.screen_lines() as f32;
        let have = grid.display_offset() as f32;
        if pos.y < thumb.top() {
            have + page
        } else if pos.y > thumb.bottom() {
            have - page
        } else {
            return;
        }
    } else {
        return;
    };
    let want = want.round().clamp(bottom as f32, top as f32) as i32;
    let have = grid.display_offset() as i32;
    if want != have {
        m.term.scroll_display(Scroll::Delta(want - have));
    }
    m.follow = m.term.grid().display_offset() <= m.newest_offset();
}

fn draw_bar(painter: &egui::Painter, bar: &egui::Response, m: &TermModel) {
    painter.rect_filled(bar.rect, 0.0, palette::BAR_TRACK);
    let Some(t) = thumb(m) else { return };
    let track = bar.rect.shrink2(Vec2::new(3.0, 2.0));
    let color = if bar.dragged() {
        palette::BAR_THUMB_ACTIVE
    } else if bar.hovered() {
        palette::BAR_THUMB_HOVER
    } else {
        palette::BAR_THUMB
    };
    painter.rect_filled(thumb_rect(track, t), 3.0, color);
}

/// Wheel movement over the view as whole lines (up is positive).
fn wheel_lines(ui: &egui::Ui, cell_h: f32, rows: usize) -> i32 {
    let mut lines = 0.0;
    ui.input(|i| {
        for e in &i.events {
            if let egui::Event::MouseWheel { unit, delta, .. } = e {
                lines += match unit {
                    egui::MouseWheelUnit::Line => delta.y * 3.0,
                    egui::MouseWheelUnit::Point => delta.y / cell_h,
                    egui::MouseWheelUnit::Page => delta.y * rows as f32,
                };
            }
        }
    });
    if lines != 0.0 && lines.abs() < 1.0 {
        lines.signum() as i32
    } else {
        lines.round() as i32
    }
}

/// The grid point under `pos`, and which half of the cell it is on.
fn point_at(m: &TermModel, origin: Pos2, cell: Vec2, pos: Pos2) -> (Point, Side) {
    let grid = m.term.grid();
    let rel = pos - origin;
    let col_f = (rel.x / cell.x).max(0.0);
    let col = (col_f as usize).min(grid.columns() - 1);
    let row = ((rel.y / cell.y).floor() as i32).clamp(0, grid.screen_lines() as i32 - 1);
    let line = row - grid.display_offset() as i32;
    let side = if col_f.fract() < 0.5 {
        Side::Left
    } else {
        Side::Right
    };
    (Point::new(Line(line), Column(col)), side)
}

fn select(ui: &egui::Ui, response: &egui::Response, m: &mut TermModel, origin: Pos2, cell: Vec2) {
    let Some(pos) = response.interact_pointer_pos() else {
        return;
    };
    let (point, side) = point_at(m, origin, cell, pos);
    if response.triple_clicked() {
        m.term.selection = Some(Selection::new(SelectionType::Lines, point, side));
    } else if response.double_clicked() {
        m.term.selection = Some(Selection::new(SelectionType::Semantic, point, side));
    } else if response.drag_started() {
        // From where the button went down, not where the drag was recognised.
        let from = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
        let (start, start_side) = point_at(m, origin, cell, from);
        let mut s = Selection::new(SelectionType::Simple, start, start_side);
        s.update(point, side);
        m.term.selection = Some(s);
    } else if response.dragged() {
        // Past the top or bottom edge: scroll while selecting.
        let rect = response.rect;
        let edge = if pos.y < rect.top() {
            1
        } else if pos.y > rect.bottom() {
            -1
        } else {
            0
        };
        if edge != 0 {
            m.term.scroll_display(Scroll::Delta(edge));
            m.follow = m.term.grid().display_offset() <= m.newest_offset();
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        }
        let (point, side) = point_at(m, origin, cell, pos);
        if let Some(s) = m.term.selection.as_mut() {
            s.update(point, side);
        }
    } else if response.clicked() {
        m.term.selection = None;
    }
}

fn keyboard(ui: &egui::Ui, m: &mut TermModel) {
    let mode = *m.term.mode();
    let mut bytes = Vec::new();
    let events = ui.input(|i| i.events.clone());
    for e in &events {
        match e {
            egui::Event::Copy => match m.term.selection_to_string().filter(|s| !s.is_empty()) {
                Some(text) => {
                    ui.ctx().copy_text(text);
                    m.term.selection = None;
                }
                None => bytes.push(0x03),
            },
            egui::Event::Cut => bytes.push(0x18),
            e => input::encode_event(e, mode, &mut bytes),
        }
    }
    if !bytes.is_empty() && m.attached().is_some() {
        m.term.selection = None;
        m.input(&bytes);
    }
}

fn draw(
    m: &TermModel,
    painter: &egui::Painter,
    origin: Pos2,
    cell: Vec2,
    font: &FontId,
    focused: bool,
) {
    let term = &m.term;
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let colors = content.colors;
    let selection = content.selection;
    let cursor = content.cursor;
    let default_bg = palette::rgb(NamedColor::Background as usize);
    let at = |row: i32, col: usize| origin + Vec2::new(col as f32 * cell.x, row as f32 * cell.y);

    // Runs of same-coloured text on one row, drawn as one string each.
    let mut run = String::new();
    let mut run_start = (0i32, 0usize);
    let mut run_len = 0usize;
    let mut run_color = Color32::WHITE;
    // The last colour made readable (runs repeat it).
    let mut readable_for = (Color32::TRANSPARENT, Color32::TRANSPARENT);
    let mut readable_was = Color32::TRANSPARENT;
    let flush = |run: &mut String, start: (i32, usize), color: Color32| {
        if !run.trim_end().is_empty() {
            painter.text(
                at(start.0, start.1),
                egui::Align2::LEFT_TOP,
                run.as_str(),
                font.clone(),
                color,
            );
        }
        run.clear();
    };

    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset;
        let col = indexed.point.column.0;
        let c = &indexed.cell;
        if c.flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let mut fg = palette::resolve(colors, c.fg);
        let mut bg = palette::resolve(colors, c.bg);
        if c.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let width = if c.flags.contains(Flags::WIDE_CHAR) {
            2.0
        } else {
            1.0
        };
        let cell_rect = Rect::from_min_size(at(row, col), Vec2::new(cell.x * width, cell.y));
        let under = if selection.is_some_and(|s| s.contains(indexed.point)) {
            painter.rect_filled(cell_rect, 0.0, palette::SELECTION);
            palette::SELECTION
        } else {
            if bg != default_bg {
                painter.rect_filled(cell_rect, 0.0, palette::to_color32(bg));
            }
            palette::to_color32(bg)
        };
        let mut fg32 = palette::to_color32(fg);
        if c.flags.contains(Flags::DIM) {
            fg32 = fg32.lerp_to_gamma(under, 0.34);
        }
        // A program's colours can be anything (black on black): each reads at the least contrast.
        fg32 = if c.flags.contains(Flags::HIDDEN) {
            Color32::TRANSPARENT
        } else if readable_for == (fg32, under) {
            readable_was
        } else {
            readable_for = (fg32, under);
            readable_was = contrast::readable(fg32, under, contrast::MIN);
            readable_was
        };
        let contiguous = run_start.0 == row && run_start.1 + run_len == col;
        if !contiguous || fg32 != run_color {
            flush(&mut run, run_start, run_color);
            run_start = (row, col);
            run_len = 0;
            run_color = fg32;
        }
        run.push(c.c);
        run_len += width as usize;
        if c.flags.contains(Flags::UNDERLINE) {
            let y = cell_rect.bottom() - 1.0;
            painter.hline(cell_rect.x_range(), y, Stroke::new(1.0, fg32));
        }
    }
    flush(&mut run, run_start, run_color);

    // The cursor, only while a program takes input: a block when focused, an outline otherwise.
    if m.attached().is_some() && cursor.shape != CursorShape::Hidden {
        let row = cursor.point.line.0 + offset;
        if row >= 0 && (row as usize) < term.screen_lines() {
            let r = Rect::from_min_size(at(row, cursor.point.column.0), cell);
            let c = palette::to_color32(palette::rgb(NamedColor::Cursor as usize));
            if focused {
                painter.rect_filled(r, 0.0, c.gamma_multiply(0.7));
            } else {
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, c), egui::StrokeKind::Inside);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumb_shows_where_the_view_is() {
        let mut m = TermModel::new();
        m.resize(40, 10);
        assert_eq!(thumb(&m), None, "nothing to scroll");
        for i in 0..90 {
            m.feed(format!("line {i}\n").as_bytes());
        }
        m.show_newest();
        // Following: at the bottom, a tenth of the track for ten rows of a hundred.
        let (at, size) = thumb(&m).unwrap();
        assert_eq!(at, 1.0);
        assert!((size - 0.1).abs() < 0.02, "{size}");
        // Back at the oldest line: at the top.
        m.term.scroll_display(Scroll::Top);
        assert_eq!(thumb(&m).unwrap().0, 0.0);
        let track = Rect::from_min_size(Pos2::ZERO, Vec2::new(10.0, 200.0));
        assert_eq!(thumb_rect(track, (0.0, 0.1)).top(), 0.0);
        assert_eq!(thumb_rect(track, (1.0, 0.1)).bottom(), 200.0);
        // A long log still gets a thumb big enough to grab.
        assert_eq!(thumb_rect(track, (0.5, 0.001)).height(), 24.0);
    }
}

//! The gallery: every control the UI draws, in the current theme, to tune `theme.kdl` against
//! (`launcher --gallery`, `cargo xtask gallery`). A debug build reloads the theme file on save,
//! so the gallery redraws as the file changes. Temporary: it goes once the theme settles.

use egui::{
    Button, Checkbox, CollapsingHeader, Color32, ComboBox, DragValue, Frame, Grid, Modal,
    ProgressBar, RichText, ScrollArea, Sense, Slider, TextEdit, ThemePreference, Ui, Vec2, Window,
};

use crate::console::{self, SharedTerm, TermModel};
use crate::editor::luaedit;
use crate::ui::run::{status_icon, Status};
use crate::ui::theme;

const COMBO: [&str; 4] = ["Development", "Shipping", "Debug", "Test"];

/// Sample console output: styles, the 16 colours, and a build tool's look.
const CONSOLE: &str = "\x1b[1mBold\x1b[0m \x1b[2mdim\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[7minverse\x1b[0m\r\n\
\x1b[30m 0 \x1b[31m 1 \x1b[32m 2 \x1b[33m 3 \x1b[34m 4 \x1b[35m 5 \x1b[36m 6 \x1b[37m 7 \x1b[0m\r\n\
\x1b[90m 8 \x1b[91m 9 \x1b[92m10 \x1b[93m11 \x1b[94m12 \x1b[95m13 \x1b[96m14 \x1b[97m15 \x1b[0m\r\n\
\x1b[1;32m   Compiling\x1b[0m launcher v0.1.0\r\n\
\x1b[1;33mwarning\x1b[0m: unused variable: `x`\r\n\
\x1b[1;31merror\x1b[0m: could not compile `launcher`\r\n";

pub struct Gallery {
    enabled: bool,
    centre_lines: bool,
    animate: bool,
    text: String,
    password: String,
    multiline: String,
    code: String,
    lua: String,
    globals: Vec<String>,
    number: f64,
    int: i32,
    slider: f32,
    checked: bool,
    partial: bool,
    radio: usize,
    tab: usize,
    combo: usize,
    toggle: bool,
    color: Color32,
    window_open: bool,
    modal_open: bool,
    term: SharedTerm,
}

type Section = (&'static str, fn(&mut Gallery, &mut Ui));

impl Gallery {
    pub fn new() -> Gallery {
        let term = TermModel::shared();
        if let Ok(mut t) = term.lock() {
            t.feed(CONSOLE.as_bytes());
        }
        Gallery {
            enabled: true,
            centre_lines: false,
            animate: false,
            text: String::new(),
            password: "secret".into(),
            multiline: "A multi-line text field.\nIt grows with its text.".into(),
            code: "cargo build --release -p launcher".into(),
            lua: "-- A Lua field\nif values.config == \"Shipping\" then\n  log(\"shipping build\")\nend\n".into(),
            globals: vec!["log".into(), "set".into()],
            number: 3.5,
            int: 42,
            slider: 0.6,
            checked: true,
            partial: false,
            radio: 0,
            tab: 0,
            combo: 0,
            toggle: false,
            color: Color32::from_rgb(90, 170, 255),
            window_open: false,
            modal_open: false,
            term,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        egui::Panel::top("gallery-bar").show(ui, |ui| self.bar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.add_enabled_ui(self.enabled, |ui| self.sections(ui));
            });
        });
        self.floating(ui.ctx());
    }

    fn bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Theme gallery").strong());
            ui.separator();
            let mut pref = ctx.options(|o| o.theme_preference);
            let before = pref;
            ui.selectable_value(&mut pref, ThemePreference::Dark, "Dark");
            ui.selectable_value(&mut pref, ThemePreference::Light, "Light");
            ui.selectable_value(&mut pref, ThemePreference::System, "System");
            if pref != before {
                ctx.set_theme(pref);
            }
            ui.separator();
            let zoom = ctx.zoom_factor();
            if ui.small_button("−").on_hover_text("Zoom out").clicked() {
                ctx.set_zoom_factor((zoom - 0.1).max(0.5));
            }
            if ui
                .small_button(format!("{:.0}%", zoom * 100.0))
                .on_hover_text("Reset the zoom")
                .clicked()
            {
                ctx.set_zoom_factor(1.0);
            }
            if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                ctx.set_zoom_factor((zoom + 0.1).min(3.0));
            }
            ui.separator();
            ui.checkbox(&mut self.enabled, "Enabled")
                .on_hover_text("Untick to see every control disabled");
            ui.checkbox(&mut self.animate, "Animate")
                .on_hover_text("Spinners and animated progress (they redraw continuously)");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let status = theme::status(&ctx);
                match &status.file {
                    Some(f) => {
                        let name = f.file_name().unwrap_or_default().to_string_lossy();
                        ui.label(
                            RichText::new(format!("{name} · loaded {}×", status.loads)).weak(),
                        )
                        .on_hover_text(f.display().to_string());
                    }
                    None => {
                        ui.label(RichText::new("built-in theme").weak())
                            .on_hover_text(
                            "A release build: set LAUNCHER_THEME to a theme file to edit it live",
                        );
                    }
                }
                if !status.problems.is_empty() {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!("{} problems", status.problems.len()),
                    );
                }
            });
        });
    }

    fn sections(&mut self, ui: &mut Ui) {
        let sections: [Section; 12] = [
            ("Text", Gallery::text),
            ("Buttons", Gallery::buttons),
            ("Choices", Gallery::choices),
            ("Text input", Gallery::input),
            ("Values and progress", Gallery::values),
            ("Containers", Gallery::containers),
            ("Popups and windows", Gallery::popups),
            ("Colours", Gallery::colours),
            ("Widget states", Gallery::states),
            ("Run view", Gallery::run_view),
            ("Callouts and messages", Gallery::callouts),
            ("Console and code", Gallery::console),
        ];
        let n = ((ui.available_width() / 420.0) as usize).clamp(1, 3);
        let gap = theme::extras(ui).gap;
        ui.columns(n, |cols| {
            for (i, (title, body)) in sections.iter().enumerate() {
                let ui = &mut cols[i % n];
                Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new(*title).heading());
                    ui.add_space(theme::extras(ui).gap_small);
                    body(self, ui);
                });
                ui.add_space(gap);
            }
        });
    }

    fn text(&mut self, ui: &mut Ui) {
        let styles: Vec<(egui::TextStyle, egui::FontId)> = ui
            .style()
            .text_styles
            .iter()
            .map(|(s, f)| (s.clone(), f.clone()))
            .collect();
        for (style, font) in styles {
            ui.label(RichText::new(format!("{style} · {} px", font.size)).text_style(style));
        }
        let faces = theme::status(ui.ctx()).faces.join(", ");
        ui.label(
            RichText::new(format!("Faces: {faces} (nudge them in `fonts {{ }}`)"))
                .small()
                .weak(),
        );
        ui.separator();
        ui.label("Body text: the quick brown fox jumps over the lazy dog.");
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Strong").strong());
            ui.label(RichText::new("Weak").weak());
            ui.label(RichText::new("Small").small());
            ui.label(RichText::new("Small weak").small().weak());
            ui.label(RichText::new("Italics").italics());
            ui.label(RichText::new("Underline").underline());
            ui.label(RichText::new("Strikethrough").strikethrough());
            ui.label(RichText::new("Monospace").monospace());
            ui.code("code()");
        });
        ui.horizontal_wrapped(|ui| {
            let (v, ex) = (ui.visuals().clone(), theme::extras(ui));
            ui.colored_label(ex.accent, "Accent");
            ui.colored_label(ex.success, "Success");
            ui.colored_label(v.warn_fg_color, "Warning");
            ui.colored_label(v.error_fg_color, "Error");
            ui.hyperlink_to("Hyperlink", "https://github.com/emilk/egui");
            let _ = ui.link("Link");
        });
        ui.label(
            "A long label wraps at the edge of its container, which is what most of the form's \
             help texts and messages do when they run longer than the field column.",
        );
        ui.add(
            egui::Label::new(
                "A truncated label ends in an ellipsis instead of wrapping, like a step's message.",
            )
            .truncate(),
        );
    }

    fn buttons(&mut self, ui: &mut Ui) {
        let mut rects = Vec::new();
        ui.horizontal_wrapped(|ui| {
            rects.push(ui.button("Button").rect);
            rects.push(
                ui.add(Button::new(RichText::new("Run").strong()))
                    .on_hover_text("A document's main action: strong text")
                    .rect,
            );
            rects.push(ui.small_button("Small").rect);
            rects.push(ui.add(Button::new("Frameless").frame(false)).rect);
            rects.push(ui.add(Button::new("Selected").selected(true)).rect);
            rects.push(ui.add_enabled(false, Button::new("Disabled")).rect);
            rects.push(ui.toggle_value(&mut self.toggle, "Toggle").rect);
            rects.push(ui.add(Button::new("Shortcut").shortcut_text("Ctrl+S")).rect);
            rects.push(ui.add(Button::new("Browse…")).rect);
            rects.push(
                ui.add(Button::new(RichText::new("Edit").small().weak()).frame(false))
                    .rect,
            );
        });
        ui.horizontal(|ui| {
            let menu = ui.menu_button("Menu", |ui| {
                let _ = ui.button("Open…");
                let _ = ui.button("Save");
                ui.separator();
                ui.menu_button("More", |ui| {
                    let _ = ui.button("Nested item");
                });
            });
            rects.push(menu.response.rect);
            rects.push(
                ui.add(Button::new("Wide button").min_size(Vec2::new(160.0, 0.0)))
                    .rect,
            );
        });
        ui.checkbox(&mut self.centre_lines, "Centre lines")
            .on_hover_text("A line through each button's middle, to judge `fonts { }` nudges");
        if self.centre_lines {
            let stroke = egui::Stroke::new(1.0, ui.visuals().error_fg_color.gamma_multiply(0.6));
            for r in rects {
                ui.painter().hline(r.x_range(), r.center().y, stroke);
            }
        }
    }

    fn choices(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.checked, "Checkbox");
            ui.add(Checkbox::new(&mut self.partial, "Indeterminate").indeterminate(true));
            let mut on = true;
            ui.add_enabled(false, Checkbox::new(&mut on, "Disabled"));
        });
        ui.horizontal(|ui| {
            for (i, l) in ["One", "Two", "Three"].into_iter().enumerate() {
                ui.radio_value(&mut self.radio, i, l);
            }
        });
        ui.horizontal(|ui| {
            for (i, l) in ["Problems", "Dry run", "Text"].into_iter().enumerate() {
                ui.selectable_value(&mut self.tab, i, l);
            }
        });
        ComboBox::from_label("Combo box")
            .selected_text(COMBO[self.combo])
            .show_ui(ui, |ui| {
                for (i, l) in COMBO.into_iter().enumerate() {
                    ui.selectable_value(&mut self.combo, i, l);
                }
            });
    }

    fn input(&mut self, ui: &mut Ui) {
        let ex = theme::extras(ui);
        Grid::new("gallery-form")
            .num_columns(2)
            .spacing([ex.label_gap, ui.spacing().item_spacing.y])
            .show(ui, |ui| {
                ui.label("Single line");
                ui.add(TextEdit::singleline(&mut self.text).hint_text("Hint text"));
                ui.end_row();
                ui.label("Password");
                ui.add(TextEdit::singleline(&mut self.password).password(true));
                ui.end_row();
                ui.label("Path");
                ui.horizontal(|ui| {
                    let mut path = String::from(r"C:\Projects\Game");
                    ui.add(TextEdit::singleline(&mut path).desired_width(180.0));
                    let _ = ui.button("Browse…");
                });
                ui.end_row();
                ui.label("Disabled");
                let mut fixed = String::from("Can't change this");
                ui.add_enabled(false, TextEdit::singleline(&mut fixed));
                ui.end_row();
                ui.label("Numbers");
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut self.number).speed(0.1));
                    ui.add(DragValue::new(&mut self.int).range(0..=100).suffix(" %"));
                });
                ui.end_row();
            });
        ui.add(TextEdit::multiline(&mut self.multiline).desired_rows(3));
        ui.add(
            TextEdit::multiline(&mut self.code)
                .code_editor()
                .desired_rows(2),
        );
    }

    fn values(&mut self, ui: &mut Ui) {
        ui.add(Slider::new(&mut self.slider, 0.0..=1.0).text("Slider"));
        ui.add(Slider::new(&mut self.int, 0..=100).text("Integer"));
        ui.add(ProgressBar::new(self.slider).show_percentage());
        ui.add(
            ProgressBar::new(0.35)
                .text("Building…")
                .animate(self.animate),
        );
        ui.horizontal(|ui| {
            if self.animate {
                ui.add(egui::Spinner::new());
            }
            ui.label(if self.animate {
                "Spinner"
            } else {
                "Tick Animate for a spinner"
            });
        });
    }

    fn containers(&mut self, ui: &mut Ui) {
        CollapsingHeader::new("Collapsing header")
            .default_open(true)
            .show(ui, |ui| {
                ui.label("Its content, indented.");
            });
        ui.collapsing("Closed header", |ui| {
            ui.label("Hidden content");
        });
        ui.indent("gallery-indent", |ui| {
            ui.label("ui.indent()");
        });
        ui.separator();
        Grid::new("gallery-grid")
            .striped(true)
            .num_columns(2)
            .show(ui, |ui| {
                for (k, v) in [
                    ("Striped", "grid"),
                    ("Key", "value"),
                    ("Another", "row"),
                    ("Last", "row"),
                ] {
                    ui.label(k);
                    ui.label(v);
                    ui.end_row();
                }
            });
        ui.separator();
        ScrollArea::vertical()
            .id_salt("gallery-scroll")
            .max_height(80.0)
            .show(ui, |ui| {
                for i in 1..=30 {
                    ui.label(format!("Scrolling line {i}"));
                }
            });
        Frame::canvas(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label("Frame::canvas (extreme_bg_color)");
        });
    }

    fn popups(&mut self, ui: &mut Ui) {
        ui.label("Hover here for a tooltip")
            .on_hover_text("A tooltip: popup fill, shadow and corner radius");
        ui.label("Right-click here for a context menu")
            .context_menu(|ui| {
                let _ = ui.button("Copy");
                let _ = ui.button("Paste");
            });
        ui.horizontal(|ui| {
            if ui.button("Open a window").clicked() {
                self.window_open = true;
            }
            if ui.button("Open a dialog").clicked() {
                self.modal_open = true;
            }
        });
        ui.horizontal(|ui| {
            ui.color_edit_button_srgba(&mut self.color);
            ui.label("Colour picker");
        });
    }

    fn colours(&mut self, ui: &mut Ui) {
        let v = ui.visuals().clone();
        let ex = theme::extras(ui);
        let swatches = [
            ("extras.accent", ex.accent),
            ("extras.success", ex.success),
            ("warn_fg_color", v.warn_fg_color),
            ("error_fg_color", v.error_fg_color),
            ("weak text", v.weak_text_color()),
            ("strong text", v.strong_text_color()),
            ("hyperlink_color", v.hyperlink_color),
            ("selection.bg_fill", v.selection.bg_fill),
            ("panel_fill", v.panel_fill),
            ("window_fill", v.window_fill),
            ("extreme_bg_color", v.extreme_bg_color),
            ("faint_bg_color", v.faint_bg_color),
            ("code_bg_color", v.code_bg_color),
        ];
        Grid::new("gallery-colours").num_columns(3).show(ui, |ui| {
            for (name, c) in swatches {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(36.0, 16.0), Sense::hover());
                ui.painter().rect(
                    rect,
                    3.0,
                    c,
                    egui::Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
                    egui::StrokeKind::Inside,
                );
                ui.label(name);
                let hex = theme::color_text(c).unwrap_or_else(|b| {
                    format!("{} {} {} {} (premultiplied)", b[0], b[1], b[2], b[3])
                });
                ui.label(RichText::new(hex).monospace().weak());
                ui.end_row();
            }
        });
    }

    /// Each `visuals.widgets` state drawn still, to tune without hovering or pressing.
    fn states(&mut self, ui: &mut Ui) {
        let w = ui.visuals().widgets.clone();
        let size = Vec2::new(96.0, ui.spacing().interact_size.y);
        Grid::new("gallery-states").num_columns(3).show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("bg_fill").small().weak());
            ui.label(RichText::new("weak_bg_fill (buttons)").small().weak());
            ui.end_row();
            for (name, wv) in [
                ("noninteractive", w.noninteractive),
                ("inactive", w.inactive),
                ("hovered", w.hovered),
                ("active", w.active),
                ("open", w.open),
            ] {
                ui.label(name);
                for fill in [wv.bg_fill, wv.weak_bg_fill] {
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    let rect = rect.expand(wv.expansion);
                    ui.painter().rect(
                        rect,
                        wv.corner_radius,
                        fill,
                        wv.bg_stroke,
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Label",
                        egui::TextStyle::Button.resolve(ui.style()),
                        wv.fg_stroke.color,
                    );
                }
                ui.end_row();
            }
        });
    }

    /// The run view's step rows, drawn the way `ui::run` draws them.
    fn run_view(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Build").heading());
            ui.label(RichText::new("done").color(theme::extras(ui).success));
            ui.label(RichText::new("1 min 12 s").weak());
        });
        let ex = theme::extras(ui);
        let rows = [
            (Status::Ok, "Fetch", None, "3.2 s"),
            (
                Status::Warned,
                "Restore packages",
                Some("2 packages are out of date"),
                "12 s",
            ),
            (Status::Running, "Compile", None, "41 s"),
            (Status::Pending, "Test", None, "last time 20 s"),
            (Status::Skipped, "Sign", None, "skipped"),
            (
                Status::Failed,
                "Upload",
                Some("the server refused the upload"),
                "0.4 s",
            ),
            (Status::Cancelled, "Notify", None, ""),
        ];
        for (status, label, message, right) in rows {
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), ex.step_row_height),
                Sense::click(),
            );
            if response.hovered() {
                ui.painter().rect_filled(
                    rect,
                    3.0,
                    ui.visuals()
                        .widgets
                        .hovered
                        .weak_bg_fill
                        .gamma_multiply(0.5),
                );
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            if status == Status::Running && !self.animate {
                // A spinner redraws forever; the gallery keeps still unless asked.
                status_icon(&mut child, Status::Pending);
            } else {
                status_icon(&mut child, status);
            }
            child.label(label);
            if let Some(m) = message {
                let c = if status == Status::Warned {
                    child.visuals().warn_fg_color
                } else {
                    child.visuals().error_fg_color
                };
                child.add(egui::Label::new(RichText::new(m).color(c)).truncate());
            }
            child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(right).weak());
            });
        }
    }

    fn callouts(&mut self, ui: &mut Ui) {
        let ex = theme::extras(ui);
        Frame::group(ui.style())
            .inner_margin(egui::Margin::same(ex.callout_padding as i8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(
                    RichText::new("build.launcher came from the internet.")
                        .color(ui.visuals().warn_fg_color)
                        .strong(),
                );
                ui.add_space(ex.gap_small);
                ui.label(RichText::new("It runs these programs: git, cargo, powershell").weak());
                ui.add_space(ex.gap);
                ui.horizontal(|ui| {
                    let _ = ui.add(Button::new(RichText::new("Trust this file").strong()));
                    let _ = ui.button("Close");
                });
            });
        ui.add_space(ex.gap);
        Frame::group(ui.style())
            .inner_margin(egui::Margin::same(ex.group_padding as i8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("A form group").weak());
                ui.label("Its controls sit inside group_padding.");
            });
        ui.add_space(ex.gap);
        let v = ui.visuals().clone();
        ui.label(
            RichText::new("build.launcher:12:5: unknown property `titel`").color(v.error_fg_color),
        );
        ui.label(RichText::new("    did you mean `title`?").weak());
        ui.label(RichText::new("build.launcher:20:1: `cmd` is deprecated").color(v.warn_fg_color));
        ui.add_space(ex.gap);
        // The editor's selection outline (ui::form).
        let r = ui.label("A selected item in the editor's preview");
        ui.painter().rect_stroke(
            r.rect.expand(3.0),
            4.0,
            egui::Stroke::new(ex.outline_width, ex.accent),
            egui::StrokeKind::Outside,
        );
        ui.add_space(ex.gap);
        ui.label(
            RichText::new(format!(
                "margin {} · gap {} / {} · label_gap {} · step_row_height {}",
                ex.margin, ex.gap, ex.gap_small, ex.label_gap, ex.step_row_height
            ))
            .small()
            .weak(),
        );
    }

    fn console(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("The console keeps its own dark palette.")
                .small()
                .weak(),
        );
        let h = 130.0;
        ui.allocate_ui(Vec2::new(ui.available_width(), h), |ui| {
            ui.set_min_height(h);
            console::view::show(ui, &self.term, false);
        });
        ui.add_space(theme::extras(ui).gap);
        let names = luaedit::Names {
            globals: &self.globals,
            values: &[],
        };
        luaedit::show(
            ui,
            egui::Id::new("gallery-lua"),
            &mut self.lua,
            4,
            &names,
            &[("an example warning under a Lua field".into(), false)],
        );
    }

    /// The window and the dialog the Popups section opens.
    fn floating(&mut self, ctx: &egui::Context) {
        Window::new("A window")
            .open(&mut self.window_open)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.label("window_fill, window_stroke, window_shadow and window_corner_radius.");
                ui.checkbox(&mut self.checked, "A checkbox in a window");
            });
        if self.modal_open {
            let r = Modal::new(egui::Id::new("gallery-modal")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(RichText::new("Delete the build folder?").strong());
                ui.add_space(theme::extras(ui).gap_small);
                ui.label("A dialog like the ones a run opens (confirm, ask, a warning).");
                ui.add_space(theme::extras(ui).gap);
                let mut close = false;
                ui.horizontal(|ui| {
                    close |= ui
                        .add(Button::new(RichText::new("Delete").strong()))
                        .clicked();
                    close |= ui.button("Cancel").clicked();
                });
                close
            });
            if r.inner || r.should_close() {
                self.modal_open = false;
            }
        }
    }
}

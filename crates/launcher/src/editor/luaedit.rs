//! The editor's Lua code field (N12, M8): Luau highlighted from full_moon's tokens, line
//! numbers, Tab as four spaces, and completion of the API's names, the file's script functions
//! and `values.` + ids (Ctrl+Space takes the first suggestion; a click takes any).

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RichText, TextStyle, Ui};
use full_moon::tokenizer::{Lexer, LexerResult, TokenType};
use full_moon::LuaVersion;

/// Names the completion offers.
pub struct Names<'a> {
    /// API globals and script functions.
    pub globals: &'a [String],
    /// What `values.` can be followed by.
    pub values: &'a [String],
}

const KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "else", "elseif", "end", "false", "for", "function", "if",
    "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// VS Code's colours, nudged to read at `contrast::MIN` on the theme's code field.
struct Colors {
    keyword: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    api: Color32,
    text: Color32,
}

fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            keyword: Color32::from_rgb(197, 134, 192),
            string: Color32::from_rgb(206, 145, 120),
            number: Color32::from_rgb(181, 206, 168),
            comment: Color32::from_rgb(119, 168, 98),
            api: Color32::from_rgb(86, 156, 214),
            text: Color32::from_rgb(212, 212, 212),
        }
    } else {
        Colors {
            keyword: Color32::from_rgb(150, 0, 190),
            string: Color32::from_rgb(163, 21, 21),
            number: Color32::from_rgb(5, 110, 72),
            comment: Color32::from_rgb(0, 110, 0),
            api: Color32::from_rgb(0, 0, 255),
            text: Color32::from_rgb(30, 30, 30),
        }
    }
}

/// The highlighted layout of `code`.
pub fn highlight(code: &str, font: FontId, dark: bool, globals: &[String]) -> LayoutJob {
    let c = colors(dark);
    let mut job = LayoutJob::default();
    let tokens = match Lexer::new(code, LuaVersion::luau()).collect() {
        LexerResult::Ok(t) | LexerResult::Recovered(t, _) => t,
        LexerResult::Fatal(_) => Vec::new(),
    };
    let mut at = 0;
    for t in &tokens {
        let start = t.start_position().bytes().min(code.len());
        let end = t.end_position().bytes().min(code.len());
        if start > at {
            job.append(
                &code[at..start],
                0.0,
                TextFormat::simple(font.clone(), c.text),
            );
        }
        if end <= start {
            continue;
        }
        let text = &code[start..end];
        let color = match t.token_type() {
            TokenType::StringLiteral { .. } | TokenType::InterpolatedString { .. } => c.string,
            TokenType::Number { .. } => c.number,
            TokenType::SingleLineComment { .. } | TokenType::MultiLineComment { .. } => c.comment,
            TokenType::Symbol { .. } if KEYWORDS.contains(&text) => c.keyword,
            TokenType::Identifier { identifier }
                if globals.iter().any(|g| g == identifier.as_str()) =>
            {
                c.api
            }
            _ => c.text,
        };
        job.append(text, 0.0, TextFormat::simple(font.clone(), color));
        at = end;
    }
    if at < code.len() {
        job.append(&code[at..], 0.0, TextFormat::simple(font, c.text));
    }
    job
}

/// The word being typed before byte `at`: letters, digits, `_` and `.`.
fn word_before(code: &str, at: usize) -> (usize, &str) {
    let before = &code[..at.min(code.len())];
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_' || *ch == '.')
        .last()
        .map_or(before.len(), |(i, _)| i);
    (start, &before[start..])
}

/// What completes `word`.
fn suggestions(word: &str, names: &Names) -> Vec<String> {
    if let Some(rest) = word.strip_prefix("values.") {
        return names
            .values
            .iter()
            .filter(|v| v.starts_with(rest) && v.as_str() != rest)
            .map(|v| format!("values.{v}"))
            .take(8)
            .collect();
    }
    if word.len() < 2 {
        return Vec::new();
    }
    names
        .globals
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("values"))
        .filter(|g| g.starts_with(word) && *g != word)
        .map(str::to_string)
        .take(8)
        .collect()
}

/// A Lua field. `rows`: 1 for a one-line condition, more for a block. `issues`: messages shown
/// under it. Returns whether the code changed.
pub fn show(
    ui: &mut Ui,
    id: egui::Id,
    code: &mut String,
    rows: usize,
    names: &Names,
    issues: &[(String, bool)],
) -> bool {
    let dark = ui.visuals().dark_mode;
    let font = TextStyle::Monospace.resolve(ui.style());
    let mut changed = false;
    let globals = names.globals;
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight(text.as_str(), font.clone(), dark, globals);
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let output = ui
        .horizontal_top(|ui| {
            if rows > 1 {
                let lines = code.lines().count().max(1) + usize::from(code.ends_with('\n'));
                let numbers: String = (1..=lines.max(rows))
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.label(
                    RichText::new(numbers)
                        .monospace()
                        .color(ui.visuals().weak_text_color()),
                );
            }
            egui::TextEdit::multiline(code)
                .id(id)
                .code_editor()
                .desired_rows(rows)
                .desired_width(ui.available_width())
                .layouter(&mut layouter)
                .show(ui)
        })
        .inner;
    if output.response.changed() {
        changed = true;
        if code.contains('\t') {
            *code = code.replace('\t', "    ");
        }
        if rows == 1 && code.contains('\n') {
            *code = code.replace(['\r', '\n'], " ");
        }
    }
    // Completion of the word before the cursor.
    if output.response.has_focus() {
        if let Some(range) = output.cursor_range {
            let at_char = range.primary.index.0;
            let at = code
                .char_indices()
                .nth(at_char)
                .map_or(code.len(), |(i, _)| i);
            let (start, word) = word_before(code, at);
            let list = suggestions(word, names);
            let accept_first =
                ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Space));
            let mut take: Option<String> = accept_first.then(|| list.first().cloned()).flatten();
            if !list.is_empty() && take.is_none() {
                let pos = output.galley_pos
                    + output
                        .galley
                        .pos_from_cursor(range.primary)
                        .left_bottom()
                        .to_vec2();
                egui::Area::new(id.with("complete"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(pos)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            for s in &list {
                                if ui
                                    .selectable_label(false, RichText::new(s).monospace())
                                    .clicked()
                                {
                                    take = Some(s.clone());
                                }
                            }
                            ui.label(RichText::new("Ctrl+Space: the first").small().weak());
                        });
                    });
            }
            if let Some(s) = take {
                code.replace_range(start..at, &s);
                changed = true;
                let cursor = code[..start].chars().count() + s.chars().count();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                    let c = egui::text::CCursor::new(cursor);
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(c)));
                    state.store(ui.ctx(), id);
                }
            }
        }
    }
    for (msg, error) in issues {
        let color = if *error {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().warn_fg_color
        };
        ui.label(RichText::new(msg).small().color(color));
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contrast;
    use crate::ui::theme::{Theme, BUILT_IN};

    #[test]
    fn code_colours_read_on_the_code_field() {
        let t = Theme::parse(BUILT_IN, "theme.kdl").unwrap_or_else(|p| panic!("{p:?}"));
        for (dark, look) in [(true, &t.dark), (false, &t.light)] {
            let bg = look.style.visuals.text_edit_bg_color();
            let c = colors(dark);
            for fg in [c.keyword, c.string, c.number, c.comment, c.api, c.text] {
                let r = contrast::ratio(fg, bg);
                assert!(r >= contrast::MIN, "dark {dark}: {fg:?} is {r:.2}:1");
            }
        }
    }

    #[test]
    fn completes_api_names_and_values() {
        let globals = vec!["capture".to_string(), "copy".to_string()];
        let values = vec!["branch".to_string(), "mode".to_string()];
        let names = Names {
            globals: &globals,
            values: &values,
        };
        assert_eq!(suggestions("ca", &names), ["capture"]);
        assert_eq!(suggestions("values.b", &names), ["values.branch"]);
        assert!(suggestions("c", &names).is_empty(), "too short");
        let code = "if values.mo";
        assert_eq!(word_before(code, code.len()), (3, "values.mo"));
    }

    #[test]
    fn highlighting_keeps_every_byte() {
        let code = "local x = capture(\"git\") -- note\nreturn x";
        let job = highlight(code, FontId::monospace(12.0), true, &["capture".into()]);
        assert_eq!(job.text, code);
    }
}

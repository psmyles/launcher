//! Keys typed into the console, as the bytes a terminal sends.

use alacritty_terminal::term::TermMode;
use egui::{Key, Modifiers};

/// One egui input event as terminal input. Copy and Cut are handled by the view (with a
/// selection they copy; without one they are Ctrl+C / Ctrl+X).
pub fn encode_event(e: &egui::Event, mode: TermMode, out: &mut Vec<u8>) {
    match e {
        egui::Event::Text(t) => out.extend_from_slice(t.as_bytes()),
        egui::Event::Paste(t) => paste(t, mode, out),
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => encode_key(*key, *modifiers, mode, out),
        _ => {}
    }
}

/// Pasted text: Enter is CR in a terminal; bracketed when the program asked for it.
pub fn paste(t: &str, mode: TermMode, out: &mut Vec<u8>) {
    let bracketed = mode.contains(TermMode::BRACKETED_PASTE);
    if bracketed {
        out.extend_from_slice(b"\x1b[200~");
    }
    out.extend_from_slice(t.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
    if bracketed {
        out.extend_from_slice(b"\x1b[201~");
    }
}

pub fn encode_key(key: Key, m: Modifiers, mode: TermMode, out: &mut Vec<u8>) {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let arrow = |c: u8, out: &mut Vec<u8>| {
        if m.shift || m.ctrl || m.alt {
            let code = 1 + (m.shift as u8) + 2 * (m.alt as u8) + 4 * (m.ctrl as u8);
            out.extend_from_slice(format!("\x1b[1;{code}{}", c as char).as_bytes());
        } else if app_cursor {
            out.extend_from_slice(&[0x1b, b'O', c]);
        } else {
            out.extend_from_slice(&[0x1b, b'[', c]);
        }
    };
    match key {
        Key::Enter => out.push(b'\r'),
        Key::Backspace => out.push(if m.ctrl { 0x08 } else { 0x7f }),
        Key::Tab => out.extend_from_slice(if m.shift { b"\x1b[Z" } else { b"\t" }),
        Key::Escape => out.push(0x1b),
        Key::ArrowUp => arrow(b'A', out),
        Key::ArrowDown => arrow(b'B', out),
        Key::ArrowRight => arrow(b'C', out),
        Key::ArrowLeft => arrow(b'D', out),
        Key::Home => arrow(b'H', out),
        Key::End => arrow(b'F', out),
        Key::Insert => out.extend_from_slice(b"\x1b[2~"),
        Key::Delete => out.extend_from_slice(b"\x1b[3~"),
        Key::PageUp => out.extend_from_slice(b"\x1b[5~"),
        Key::PageDown => out.extend_from_slice(b"\x1b[6~"),
        k if m.ctrl && !m.alt => {
            // Ctrl+letter → the control byte (Ctrl+A = 1 … Ctrl+Z = 26).
            let name = k.name();
            if name.len() == 1 {
                let c = name.as_bytes()[0].to_ascii_uppercase();
                if c.is_ascii_uppercase() {
                    out.push(c - b'A' + 1);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_keys_as_a_terminal_would() {
        let mut out = Vec::new();
        encode_key(Key::ArrowUp, Modifiers::NONE, TermMode::NONE, &mut out);
        assert_eq!(out, b"\x1b[A");
        out.clear();
        encode_key(
            Key::ArrowUp,
            Modifiers::NONE,
            TermMode::APP_CURSOR,
            &mut out,
        );
        assert_eq!(out, b"\x1bOA");
        out.clear();
        encode_key(Key::D, Modifiers::CTRL, TermMode::NONE, &mut out);
        assert_eq!(out, [4]);
        out.clear();
        paste("a\nb", TermMode::BRACKETED_PASTE, &mut out);
        assert_eq!(out, b"\x1b[200~a\rb\x1b[201~");
    }
}

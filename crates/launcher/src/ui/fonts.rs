//! Fonts (M1): our own faces, compiled in, then the platform's UI face and egui's built-in fonts
//! as fallbacks for what ours lack (other scripts, symbols, emoji).
//!
//! * Text: Inter (`assets/fonts`, SIL Open Font License, `Inter-OFL.txt` beside it).
//! * Monospace, everywhere (the console, code, monospace labels): JetBrains Mono NL (OFL,
//!   `JetBrainsMono-OFL.txt`). NL is the build without ligatures: egui shapes text, and a terminal
//!   or code field must show `->` and `==` as typed.
//! * Fallback behind Inter: Segoe UI on Windows; SF (the system font) on macOS. It's read on a
//!   thread started at launch, so the disk read overlaps window creation.
//!
//! Each face is named by its file (`Inter-Regular.ttf`), the name the theme's `fonts { }` uses.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

/// A font face to try: the file and the face index within it (collections hold several).
struct Face(&'static str, u32);

fn fonts_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
        PathBuf::from(windir).join("Fonts")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/System/Library/Fonts")
    }
}

#[cfg(windows)]
const SYSTEM_FACES: &[Face] = &[Face("segoeui.ttf", 0)];
#[cfg(not(windows))]
const SYSTEM_FACES: &[Face] = &[Face("SFNS.ttf", 0), Face("Helvetica.ttc", 0)];

/// The text face, compiled in.
const TEXT_NAME: &str = "Inter-Regular.ttf";
const TEXT: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// The monospace face, compiled in.
const MONO_NAME: &str = "JetBrainsMonoNL-Regular.ttf";
const MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMonoNL-Regular.ttf");

/// The first face that can be read, with its name.
fn first_readable(faces: &[Face]) -> Option<(&'static str, FontData)> {
    let dir = fonts_dir();
    faces.iter().find_map(|f| {
        let bytes = std::fs::read(dir.join(f.0)).ok()?;
        let mut data = FontData::from_owned(bytes);
        data.index = f.1;
        Some((f.0, data))
    })
}

/// The font definitions: our faces first, the system's UI face next (text only), egui's defaults
/// behind them.
fn build() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let mut put = |family: FontFamily, at: usize, name: &str, data: FontData| {
        defs.font_data.insert(name.into(), Arc::new(data));
        if let Some(list) = defs.families.get_mut(&family) {
            list.insert(at.min(list.len()), name.into());
        }
    };
    put(
        FontFamily::Proportional,
        0,
        TEXT_NAME,
        FontData::from_static(TEXT),
    );
    put(
        FontFamily::Monospace,
        0,
        MONO_NAME,
        FontData::from_static(MONO),
    );
    if let Some((name, system)) = first_readable(SYSTEM_FACES) {
        put(FontFamily::Proportional, 1, name, system);
    }
    defs
}

/// Start reading the fonts now; join when the egui context is created.
pub fn load() -> std::thread::JoinHandle<FontDefinitions> {
    std::thread::Builder::new()
        .name("fonts".into())
        .spawn(build)
        .expect("the font loading thread must start")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_faces_come_first() {
        let defs = build();
        let first = |f: FontFamily| defs.families[&f].first().cloned();
        assert_eq!(first(FontFamily::Proportional).as_deref(), Some(TEXT_NAME));
        assert_eq!(first(FontFamily::Monospace).as_deref(), Some(MONO_NAME));
        // Real font files, not stray assets.
        assert_eq!(&TEXT[..4], &[0, 1, 0, 0]);
        assert_eq!(&MONO[..4], &[0, 1, 0, 0]);
    }
}

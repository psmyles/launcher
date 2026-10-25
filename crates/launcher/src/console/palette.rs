//! The console's colours: the 16 ANSI colours (VS Code's dark terminal theme), the xterm
//! 256-colour cube and grey ramp, and the named extras. The console is always dark: build tools'
//! colours are chosen for a dark background.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use egui::Color32;

pub const BACKGROUND: Color32 = Color32::from_rgb(0x16, 0x17, 0x19);
pub const SELECTION: Color32 = Color32::from_rgba_premultiplied(0x26, 0x4f, 0x78, 0xff);
/// The scroll bar: a track a shade off the background, and a thumb that brightens under the
/// pointer and while dragged.
pub const BAR_TRACK: Color32 = Color32::from_rgb(0x1f, 0x20, 0x23);
pub const BAR_THUMB: Color32 = Color32::from_rgb(0x5c, 0x60, 0x68);
pub const BAR_THUMB_HOVER: Color32 = Color32::from_rgb(0x80, 0x85, 0x8f);
pub const BAR_THUMB_ACTIVE: Color32 = Color32::from_rgb(0xa4, 0xa9, 0xb3);

pub fn to_color32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Resolve a cell colour against the terminal's overrides (OSC 4/10/11), then our palette.
pub fn resolve(colors: &Colors, c: AnsiColor) -> Rgb {
    match c {
        AnsiColor::Spec(rgb) => rgb,
        AnsiColor::Named(n) => colors[n].unwrap_or_else(|| rgb(n as usize)),
        AnsiColor::Indexed(i) => colors[i as usize].unwrap_or_else(|| rgb(i as usize)),
    }
}

/// Palette entry `i` (0–255, then the named colours by their index).
pub fn rgb(i: usize) -> Rgb {
    const ANSI: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0xcd, 0x31, 0x31),
        (0x0d, 0xbc, 0x79),
        (0xe5, 0xe5, 0x10),
        (0x24, 0x72, 0xc8),
        (0xbc, 0x3f, 0xbc),
        (0x11, 0xa8, 0xcd),
        (0xe5, 0xe5, 0xe5),
        (0x66, 0x66, 0x66),
        (0xf1, 0x4c, 0x4c),
        (0x23, 0xd1, 0x8b),
        (0xf5, 0xf5, 0x43),
        (0x3b, 0x8e, 0xea),
        (0xd6, 0x70, 0xd6),
        (0x29, 0xb8, 0xdb),
        (0xff, 0xff, 0xff),
    ];
    let c = |(r, g, b): (u8, u8, u8)| Rgb { r, g, b };
    match i {
        0..=15 => c(ANSI[i]),
        16..=231 => {
            let n = i - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            Rgb {
                r: level(n / 36),
                g: level((n / 6) % 6),
                b: level(n % 6),
            }
        }
        232..=255 => {
            let v = (8 + (i - 232) * 10) as u8;
            Rgb { r: v, g: v, b: v }
        }
        _ if i == NamedColor::Background as usize => c((0x16, 0x17, 0x19)),
        _ if i == NamedColor::Foreground as usize
            || i == NamedColor::BrightForeground as usize
            || i == NamedColor::Cursor as usize =>
        {
            c((0xd4, 0xd4, 0xd4))
        }
        _ if i == NamedColor::DimForeground as usize => c((0x9a, 0x9a, 0x9a)),
        _ if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
            let base = ANSI[i - NamedColor::DimBlack as usize];
            let dim = |v: u8| (v as u16 * 2 / 3) as u8;
            Rgb {
                r: dim(base.0),
                g: dim(base.1),
                b: dim(base.2),
            }
        }
        _ => c((0xd4, 0xd4, 0xd4)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_the_256_colours() {
        assert_eq!(rgb(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(
            rgb(231),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(rgb(232), Rgb { r: 8, g: 8, b: 8 });
    }
}

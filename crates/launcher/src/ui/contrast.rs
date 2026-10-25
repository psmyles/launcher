//! Text contrast: every piece of text reads at [`MIN`] or better against what it sits on (WCAG's
//! contrast ratio). The theme's colours are chosen for it, and the tests here check them; text
//! whose colours come from elsewhere (a program's output in the console) is moved until it does.

use egui::Color32;

/// The least contrast any text gets.
pub const MIN: f32 = 6.0;

/// WCAG's relative luminance of an opaque colour.
pub fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// The contrast ratio of two opaque colours, 1 to 21.
pub fn ratio(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `fg`, or the nearest colour towards white or black (whichever gets there) that reads at `min`
/// on `bg`. Its hue stays, so red output is still red.
pub fn readable(fg: Color32, bg: Color32, min: f32) -> Color32 {
    if ratio(fg, bg) >= min {
        return fg;
    }
    let toward = |end: Color32| {
        // The least step towards `end` that reads: the ratio grows with the step.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..12 {
            let mid = (lo + hi) / 2.0;
            if ratio(fg.lerp_to_gamma(end, mid), bg) >= min {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        fg.lerp_to_gamma(end, hi)
    };
    let (light, dark) = (toward(Color32::WHITE), toward(Color32::BLACK));
    let (rl, rd) = (ratio(light, bg), ratio(dark, bg));
    if rl >= min && (rd < min || luminance(bg) < 0.18) {
        light
    } else if rd >= min {
        dark
    } else if rl >= rd {
        light
    } else {
        dark
    }
}

/// Black or white, whichever reads better on `bg`: text on a coloured fill.
pub fn on(bg: Color32) -> Color32 {
    if ratio(Color32::WHITE, bg) >= ratio(Color32::BLACK, bg) {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{Look, Theme, BUILT_IN};

    #[test]
    fn ratios_match_wcag() {
        assert!((ratio(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
        assert!((ratio(Color32::from_gray(0x76), Color32::WHITE) - 4.54).abs() < 0.01);
        assert_eq!(ratio(Color32::RED, Color32::RED), 1.0);
    }

    #[test]
    fn readable_keeps_what_reads_and_moves_what_doesnt() {
        let bg = Color32::from_rgb(0x16, 0x17, 0x19);
        let fine = Color32::from_gray(0xd4);
        assert_eq!(readable(fine, bg, MIN), fine);
        for fg in [
            Color32::BLACK,
            Color32::from_rgb(0xcd, 0x31, 0x31),
            Color32::from_rgb(0x24, 0x72, 0xc8),
            bg,
        ] {
            let r = readable(fg, bg, MIN);
            assert!(ratio(r, bg) >= MIN, "{fg:?} -> {r:?}");
        }
        // Red stays reddish.
        let r = readable(Color32::from_rgb(0xcd, 0x31, 0x31), bg, MIN);
        assert!(r.r() > r.g() && r.r() > r.b(), "{r:?}");
        // On a light background, it darkens.
        let light = Color32::from_gray(0xf0);
        let r = readable(Color32::from_rgb(0xe5, 0xe5, 0x10), light, MIN);
        assert!(ratio(r, light) >= MIN && luminance(r) < luminance(light));
    }

    /// Every text colour of a theme on every surface it can be drawn on.
    fn check(name: &str, look: &Look, bad: &mut Vec<String>) {
        let v = &look.style.visuals;
        let w = &v.widgets;
        let surfaces = [
            ("panel", v.panel_fill),
            ("window", v.window_fill),
            ("field", v.text_edit_bg_color()),
            ("frame", w.noninteractive.weak_bg_fill),
            ("button", w.inactive.weak_bg_fill),
            ("hovered button", w.hovered.weak_bg_fill),
            ("pressed button", w.active.weak_bg_fill),
            ("open button", w.open.weak_bg_fill),
            ("selected", v.selection.bg_fill),
        ];
        let text = [
            ("text", v.text_color()),
            ("muted text", v.weak_text_color()),
            ("strong text", v.strong_text_color()),
            ("button text", w.inactive.fg_stroke.color),
            ("hovered button text", w.hovered.fg_stroke.color),
            ("pressed button text", w.active.fg_stroke.color),
            ("open button text", w.open.fg_stroke.color),
            ("selected text", v.selection.stroke.color),
        ];
        // Warnings, errors, links and success marks sit on the window, not on buttons.
        let status = [
            ("warning", v.warn_fg_color),
            ("error", v.error_fg_color),
            ("link", v.hyperlink_color),
            ("success", look.extras.success),
        ];
        let mut pair = |(tn, t): (&str, Color32), (sn, s): (&str, Color32)| {
            let r = ratio(t, s);
            if r < MIN {
                bad.push(format!("{name}: {tn} {t:?} on {sn} {s:?} is {r:.2}:1"));
            }
        };
        for t in text {
            for s in surfaces {
                pair(t, s);
            }
        }
        for t in status {
            for s in &surfaces[..4] {
                pair(t, *s);
            }
        }
    }

    #[test]
    fn the_built_in_theme_reads_at_six_to_one() {
        let t = Theme::parse(BUILT_IN, "theme.kdl").unwrap_or_else(|p| panic!("{p:?}"));
        let mut bad = Vec::new();
        check("dark", &t.dark, &mut bad);
        check("light", &t.light, &mut bad);
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }
}

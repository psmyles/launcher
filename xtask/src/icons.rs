//! `cargo xtask icons`: the app icon. Windows draws it from `assets/application-logo.png` (the
//! artwork: square, on transparency); macOS uses `assets/launcher-app.icon` (Icon Composer).
//!
//! * **Windows:** `assets/launcher.ico`, committed because every build reads it:
//!   `crates/launcher/build.rs` embeds it in the exe (Explorer, the taskbar, the window and
//!   `.launcher` files show it) and Inno Setup gives it to Setup. The artwork as it is, on
//!   transparency, at each size Windows asks for. A test keeps it current with the artwork, and
//!   every xtask release build (and `check`, before its tests) redraws it when it isn't.
//! * **macOS:** `package-macos` compiles the `.icon` with `actool` into the bundle's `Assets.car`,
//!   which macOS 26+ draws (named by `CFBundleIconName`). The icns beside it is for everything that
//!   reads `CFBundleIconFile`: `.launcher` files in Finder, older macOS. Icon Composer's `ictool`
//!   renders the `.icon` at each size, placed in the margin Apple's icon grid leaves around the
//!   tile, and `iconutil` packs them. Both need Xcode, not just the Command Line Tools.
//! * **Setup's wizard:** the small image (top right) and the large one (the last page), drawn by
//!   `package-windows` into `target/package/wizard`.
//!
//! Scaling is Lanczos on premultiplied colour, so edges don't pick up the black that the artwork's
//! clear pixels hold.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, ImageFormat, Rgba, RgbaImage};

use crate::util;

/// The artwork, relative to the workspace root.
const ARTWORK: &str = "assets/application-logo.png";
/// The Windows icon, relative to the workspace root.
pub const ICO: &str = "assets/launcher.ico";
/// The .ico's sizes: the 16 px small icon at 100 % to 400 % display scale (which covers the 32 px
/// large icon to 200 %), and 256 px for Explorer's big views.
const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];
/// The grey behind the artwork in Setup's large wizard image (sRGB); `launcher.iss` fills the rest
/// of that panel with it.
pub const TILE: [u8; 3] = [0x34, 0x36, 0x39];
/// The macOS icon, relative to the workspace root.
const MAC_ICON: &str = "assets/launcher-app.icon";
/// Its name in `Assets.car`, which `CFBundleIconName` gives: the `.icon`'s file name.
pub const MAC_ICON_NAME: &str = "launcher-app";
/// The tile's share of a macOS icon's edge: Apple's grid puts an 824 px tile on a 1024 px canvas.
const MAC_TILE: f32 = 824.0 / 1024.0;

/// The artwork, premultiplied, in 0..1.
pub struct Artwork(ImageBuffer<Rgba<f32>, Vec<f32>>);

impl Artwork {
    pub fn load() -> Result<Artwork, String> {
        let path = util::root().join(ARTWORK);
        let img = image::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Artwork::new(&img.to_rgba8()))
    }

    fn new(img: &RgbaImage) -> Artwork {
        let mut out = ImageBuffer::new(img.width(), img.height());
        for (o, p) in out.pixels_mut().zip(img.pixels()) {
            let a = f32::from(p[3]) / 255.0;
            let c = |v: u8| f32::from(v) / 255.0 * a;
            *o = Rgba([c(p[0]), c(p[1]), c(p[2]), a]);
        }
        Artwork(out)
    }

    /// A `width` x `height` image with the artwork fitted into a `fit` px square at its centre,
    /// over `back` (transparent without one).
    pub fn draw(&self, width: u32, height: u32, fit: u32, back: Option<[u8; 3]>) -> RgbaImage {
        let (aw, ah) = self.0.dimensions();
        let scale = fit as f32 / aw.max(ah) as f32;
        let sw = ((aw as f32 * scale).round() as u32).max(1);
        let sh = ((ah as f32 * scale).round() as u32).max(1);
        let art = imageops::resize(&self.0, sw, sh, FilterType::Lanczos3);
        let ox = width.saturating_sub(sw) / 2;
        let oy = height.saturating_sub(sh) / 2;
        let back = back.map(|b| b.map(|v| f32::from(v) / 255.0));
        RgbaImage::from_fn(width, height, |x, y| {
            let src = if x >= ox && y >= oy && x - ox < sw && y - oy < sh {
                let p = art.get_pixel(x - ox, y - oy);
                // Lanczos rings past the range; keep each colour within its alpha.
                let a = p[3].clamp(0.0, 1.0);
                [
                    p[0].clamp(0.0, a),
                    p[1].clamp(0.0, a),
                    p[2].clamp(0.0, a),
                    a,
                ]
            } else {
                [0.0; 4]
            };
            let [r, g, b, a] = match back {
                Some([br, bg, bb]) => {
                    let k = 1.0 - src[3];
                    [src[0] + br * k, src[1] + bg * k, src[2] + bb * k, 1.0]
                }
                None => src,
            };
            let byte = |v: f32| (v * 255.0).round() as u8;
            if a <= 0.0 {
                Rgba([0, 0, 0, 0])
            } else {
                Rgba([byte(r / a), byte(g / a), byte(b / a), byte(a)])
            }
        })
    }

    /// Windows: the artwork alone, edge to edge (it carries its own margin).
    fn windows(&self, size: u32) -> RgbaImage {
        self.draw(size, size, size, None)
    }
}

pub fn run_icons(args: &[String]) -> Result<(), String> {
    let art = Artwork::load()?;
    let path = util::root().join(ICO);
    if util::flag(args, "--force") || !ico_is_current(&art) {
        write(&path, &ico(&art))?;
        println!("{} redrawn", path.display());
    } else {
        println!("{} is current", path.display());
    }
    if let Some(dir) = util::value(args, "--preview") {
        let dir = Path::new(dir);
        write_wizard_images(&art, &dir.join("wizard"))?;
        if cfg!(target_os = "macos") {
            write_iconset(&dir.join("launcher.iconset"))?;
            println!(
                "the macOS iconset and Setup's wizard images are in {}",
                dir.display()
            );
        } else {
            println!("Setup's wizard images are in {}", dir.display());
        }
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn create_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// The build step that keeps `assets/launcher.ico` current: every release build runs it, since a
/// Windows build embeds the .ico, and so does `check` before its tests.
pub fn ico_step() -> Result<(), String> {
    util::step("icon");
    if refresh_ico(&Artwork::load()?)? {
        eprintln!("  redrew {ICO} from {ARTWORK}; commit it");
    } else {
        eprintln!("  {ICO} is current");
    }
    Ok(())
}

/// Redraw `assets/launcher.ico` if it no longer matches the artwork; true when it did.
fn refresh_ico(art: &Artwork) -> Result<bool, String> {
    if ico_is_current(art) {
        return Ok(false);
    }
    write(&util::root().join(ICO), &ico(art))?;
    Ok(true)
}

/// Whether the committed .ico holds what the artwork draws, give or take rounding: scaling isn't
/// bit-exact across platforms' maths libraries, so the bytes may differ where the pixels don't.
fn ico_is_current(art: &Artwork) -> bool {
    let Some(have) = std::fs::read(util::root().join(ICO))
        .ok()
        .and_then(|b| read_ico(&b))
    else {
        return false;
    };
    let want: Vec<RgbaImage> = ICO_SIZES.iter().map(|&s| art.windows(s)).collect();
    have.len() == want.len() && have.iter().zip(&want).all(|(h, w)| alike(h, w))
}

/// Two images of one size whose premultiplied pixels are within 2 of each other (straight colour
/// under a nearly clear pixel can differ by anything).
fn alike(a: &RgbaImage, b: &RgbaImage) -> bool {
    let pre = |p: &Rgba<u8>| {
        let alpha = u32::from(p[3]);
        [0, 1, 2].map(|i| (u32::from(p[i]) * alpha / 255) as u8)
    };
    a.dimensions() == b.dimensions()
        && a.pixels().zip(b.pixels()).all(|(p, q)| {
            p[3].abs_diff(q[3]) <= 2 && pre(p).iter().zip(pre(q)).all(|(x, y)| x.abs_diff(y) <= 2)
        })
}

/// The Windows icon file.
fn ico(art: &Artwork) -> Vec<u8> {
    let images: Vec<RgbaImage> = ICO_SIZES.iter().map(|&s| art.windows(s)).collect();
    ico_file(&images)
}

/// An .ico holding `images`: 256 px as PNG, as Windows stores it, smaller ones as 32-bit DIBs,
/// which every reader takes.
fn ico_file(images: &[RgbaImage]) -> Vec<u8> {
    let blobs: Vec<Vec<u8>> = images
        .iter()
        .map(|i| if i.width() >= 256 { png(i) } else { dib(i) })
        .collect();
    let count = u16::try_from(images.len()).expect("an .ico holds at most 65535 images");
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // 1: icons
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (img, blob) in images.iter().zip(&blobs) {
        // 256 is written as 0.
        out.push((img.width() % 256) as u8);
        out.push((img.height() % 256) as u8);
        out.push(0); // no palette
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += blob.len();
    }
    for blob in blobs {
        out.extend_from_slice(&blob);
    }
    out
}

/// A 32-bit DIB as an .ico stores it: a BITMAPINFOHEADER whose height counts the 1-bit AND mask
/// too, the BGRA rows bottom-up, then the mask (set where the pixel is clear).
fn dib(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mask_row = w.div_ceil(32) as usize * 4;
    let size = (w * h * 4) as usize + mask_row * h as usize;
    let mut out = Vec::with_capacity(40 + size);
    out.extend_from_slice(&40u32.to_le_bytes()); // header size
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(2 * h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]); // resolution, palette
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b, a] = img.get_pixel(x, y).0;
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    for y in (0..h).rev() {
        let mut row = vec![0u8; mask_row];
        for x in 0..w {
            if img.get_pixel(x, y)[3] == 0 {
                row[x as usize / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

/// The images in an .ico that [`ico_file`] wrote, or None for anything else.
fn read_ico(bytes: &[u8]) -> Option<Vec<RgbaImage>> {
    let u16_at = |o: usize| Some(u16::from_le_bytes(bytes.get(o..o + 2)?.try_into().ok()?));
    let u32_at = |o: usize| Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?));
    if u16_at(0)? != 0 || u16_at(2)? != 1 {
        return None;
    }
    (0..usize::from(u16_at(4)?))
        .map(|i| {
            let entry = 6 + 16 * i;
            let len = u32_at(entry + 8)? as usize;
            let offset = u32_at(entry + 12)? as usize;
            let blob = bytes.get(offset..offset + len)?;
            if blob.starts_with(b"\x89PNG") {
                let img = image::load_from_memory_with_format(blob, ImageFormat::Png).ok()?;
                return Some(img.to_rgba8());
            }
            let w = u32::from_le_bytes(blob.get(4..8)?.try_into().ok()?);
            let h = u32::from_le_bytes(blob.get(8..12)?.try_into().ok()?) / 2;
            let px = blob.get(40..40 + (w * h * 4) as usize)?;
            Some(RgbaImage::from_fn(w, h, |x, y| {
                let o = (((h - 1 - y) * w + x) * 4) as usize;
                Rgba([px[o + 2], px[o + 1], px[o], px[o + 3]])
            }))
        })
        .collect()
}

pub fn png(img: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, PngFilter::Adaptive)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgba8,
        )
        .expect("a PNG encodes into memory");
    out
}

/// Icon Composer's command-line tool, inside Xcode.
fn ictool() -> Result<PathBuf, String> {
    let out = Command::new("xcode-select")
        .arg("-p")
        .output()
        .map_err(|e| format!("could not start `xcode-select`: {e}"))?;
    let developer = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let tool = developer.join("../Applications/Icon Composer.app/Contents/Executables/ictool");
    if tool.is_file() {
        Ok(tool)
    } else {
        Err(format!(
            "{} is missing: the macOS icon needs Xcode 26 or later, selected with \
             `sudo xcode-select -s /Applications/Xcode.app`",
            tool.display()
        ))
    }
}

/// The `.iconset` folder `iconutil -c icns` takes: each point size at 1x and 2x, the `.icon` drawn
/// by `ictool` at the tile's size and centred on a clear canvas.
pub fn write_iconset(dir: &Path) -> Result<(), String> {
    let tool = ictool()?;
    create_dir(dir)?;
    let drawn = dir.join("drawn.png");
    for pt in [16, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let size = pt * scale;
            let fit = ((size as f32 * MAC_TILE).round() as u32).to_string();
            util::run(
                Command::new(&tool)
                    .arg(util::root().join(MAC_ICON))
                    .arg("--export-image")
                    .arg("--output-file")
                    .arg(&drawn)
                    .args(["--platform", "macOS", "--rendition", "Default"])
                    .args(["--width", &fit, "--height", &fit, "--scale", "1"])
                    // It prints an empty JSON object for each image.
                    .stdout(Stdio::null()),
            )?;
            let tile = image::open(&drawn)
                .map_err(|e| format!("{}: {e}", drawn.display()))?
                .to_rgba8();
            let mut icon = RgbaImage::new(size, size);
            let at = |edge: u32| i64::from(size.saturating_sub(edge) / 2);
            imageops::overlay(&mut icon, &tile, at(tile.width()), at(tile.height()));
            write(
                &dir.join(format!("icon_{pt}x{pt}{suffix}.png")),
                &png(&icon),
            )?;
        }
    }
    std::fs::remove_file(&drawn).map_err(|e| format!("{}: {e}", drawn.display()))
}

/// `Assets.car`, compiled from the `.icon` into `dir` (with an icns and a partial Info.plist that
/// the bundle doesn't use).
pub fn compile_assets(dir: &Path) -> Result<PathBuf, String> {
    create_dir(dir)?;
    util::run(
        Command::new("xcrun")
            .arg("actool")
            .arg(util::root().join(MAC_ICON))
            .arg("--compile")
            .arg(dir)
            .args(["--app-icon", MAC_ICON_NAME])
            .arg("--output-partial-info-plist")
            .arg(dir.join("partial.plist"))
            .args([
                "--platform",
                "macosx",
                "--minimum-deployment-target",
                "12.0",
            ])
            .args(["--output-format", "human-readable-text"]),
    )?;
    let car = dir.join("Assets.car");
    if car.is_file() {
        Ok(car)
    } else {
        Err(format!("actool wrote no {}", car.display()))
    }
}

/// Setup's wizard images, drawn into `dir`, returned as the comma-separated lists Inno Setup takes
/// (it picks the file that suits the display scale): the large image (the last page) in the macOS
/// tile's look, the small one (top right of the other pages) the artwork alone.
pub fn write_wizard_images(art: &Artwork, dir: &Path) -> Result<(String, String), String> {
    // Inno Setup's sizes for 100 % to 250 % display scale.
    const LARGE: [(u32, u32); 7] = [
        (164, 314),
        (192, 386),
        (246, 459),
        (273, 556),
        (328, 604),
        (355, 700),
        (410, 797),
    ];
    const SMALL: [(u32, u32); 7] = [
        (55, 55),
        (64, 68),
        (83, 80),
        (92, 97),
        (110, 106),
        (119, 123),
        (138, 140),
    ];
    create_dir(dir)?;
    let mut lists = [Vec::new(), Vec::new()];
    for (list, (name, sizes, back, fit)) in lists.iter_mut().zip([
        ("large", LARGE, Some(TILE), 0.7),
        ("small", SMALL, None, 1.0),
    ]) {
        for (w, h) in sizes {
            let path = dir.join(format!("{name}-{w}x{h}.png"));
            let fit = (w.min(h) as f32 * fit).round() as u32;
            write(&path, &png(&art.draw(w, h, fit, back)))?;
            list.push(path.display().to_string());
        }
    }
    let [large, small] = lists;
    Ok((large.join(","), small.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 40 px artwork: an opaque white disc on clear black.
    fn disc() -> Artwork {
        Artwork::new(&RgbaImage::from_fn(40, 40, |x, y| {
            let (dx, dy) = (x as f32 - 19.5, y as f32 - 19.5);
            if dx * dx + dy * dy < 15.0 * 15.0 {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        }))
    }

    #[test]
    fn the_committed_icon_is_current() {
        let art = Artwork::load().unwrap();
        assert!(
            ico_is_current(&art),
            "{ICO} doesn't match {ARTWORK}; run `cargo xtask icons`"
        );
    }

    #[test]
    fn an_ico_reads_back_as_written() {
        let art = disc();
        let images = vec![art.windows(16), art.windows(20), art.windows(256)];
        let bytes = ico_file(&images);
        // 256 is stored as 0, and as a PNG.
        assert_eq!(bytes[6 + 32], 0);
        let back = read_ico(&bytes).unwrap();
        assert_eq!(back, images);
    }

    #[test]
    fn edges_take_no_colour_from_clear_pixels() {
        for p in disc().windows(16).pixels().filter(|p| p[3] > 0) {
            assert!(p[0] >= 250 && p[1] >= 250 && p[2] >= 250, "{p:?}");
        }
    }
}

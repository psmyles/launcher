//! The UI's icons: white shapes on transparent PNGs in `assets/`, compiled in, decoded the first
//! time one is drawn and kept as a texture. Drawn tinted, so one image serves both themes.

use egui::{Color32, TextureHandle, TextureOptions};

/// The pencil on the window's Edit button.
pub const EDIT: (&str, &[u8]) = (
    "icon-edit",
    include_bytes!("../../../../assets/icon_edit.png"),
);

/// The icon's texture, made the first time it's asked for.
pub fn texture(ctx: &egui::Context, (name, png): (&str, &[u8])) -> Option<TextureHandle> {
    let id = egui::Id::new(("launcher-icon", name));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let image = decode(png)?;
    let t = ctx.load_texture(name, image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

/// An RGBA (or grey and alpha) PNG as an egui image.
fn decode(png_bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let bytes = &buf[..info.buffer_size()];
    let pixels: Vec<Color32> = match info.color_type {
        png::ColorType::Rgba => bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| Color32::from_rgba_unmultiplied(p[0], p[0], p[0], p[1]))
            .collect(),
        png::ColorType::Rgb => bytes
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| Color32::from_rgb(p[0], p[1], p[2]))
            .collect(),
        png::ColorType::Grayscale => bytes.iter().map(|g| Color32::from_gray(*g)).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(egui::ColorImage::new(
        [info.width as usize, info.height as usize],
        pixels,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icons_decode() {
        let img = decode(EDIT.1).expect("assets/icon_edit.png decodes");
        assert_eq!(img.size, [64, 64]);
        assert!(img.pixels.iter().any(|p| p.a() > 0));
    }
}

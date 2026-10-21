//! The menu logo ("OpenTT" in riveted camouflage-metal letters): OpenTT's own
//! artwork, compiled in (`art/logo.png`, made from `art/logo-source.png`,
//! see `art/README.md`), so it shows before any game data is imported.

use std::sync::OnceLock;

use egui::TextureHandle;

/// The logo, RGBA (keyed out of its background, trimmed, with its soft drop shadow).
pub const PNG: &[u8] = include_bytes!("../art/logo.png");

fn decoded() -> &'static image::RgbaImage {
    static IMG: OnceLock<image::RgbaImage> = OnceLock::new();
    IMG.get_or_init(|| image::load_from_memory_with_format(PNG, image::ImageFormat::Png).expect("art/logo.png").to_rgba8())
}

/// The logo resampled with a Lanczos filter to exactly `px_height` pixels,
/// so it is never scaled by the GPU and never aliased (egui textures have no
/// mipmaps). Cached in `ctx` per height.
pub fn texture(ctx: &egui::Context, px_height: u32) -> TextureHandle {
    let id = egui::Id::new("tt-logo");
    if let Some((h, t)) = ctx.data(|d| d.get_temp::<(u32, TextureHandle)>(id)) {
        if h == px_height {
            return t;
        }
    }
    let img = decoded();
    let h = px_height.clamp(1, img.height());
    let w = ((img.width() as f32 * h as f32 / img.height() as f32).round() as u32).max(1);
    let scaled = image::imageops::resize(img, w, h, image::imageops::FilterType::Lanczos3);
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], scaled.as_raw());
    let tex = ctx.load_texture("tt-logo", color, egui::TextureOptions::NEAREST);
    ctx.data_mut(|d| d.insert_temp(id, (px_height, tex.clone())));
    tex
}

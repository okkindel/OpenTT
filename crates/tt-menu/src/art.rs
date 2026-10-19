//! Loading the game's artwork as egui textures (from the asset directory at runtime).

use std::path::Path;

use crate::backend::{Art, TallyKind};

/// Asset path of a piece of artwork (an RGBA image).
pub fn art_path(art: &Art) -> String {
    match art {
        Art::Mission(id) => format!("missions/{id}.png"),
        Art::Slide(n) => format!("ui/backgrounds/marquee{n}.png"),
        Art::SlideBackdrop => "ui/backgrounds/marqueebackground.png".into(),
        Art::TallyIcon(k) => match k {
            TallyKind::Bronze => "ui/icons/bronzebothead.png",
            TallyKind::Silver => "ui/icons/silverbothead.png",
            TallyKind::Gold => "ui/icons/goldbothead.png",
            TallyKind::Brain => "ui/icons/brainicon.png",
        }
        .into(),
        Art::Life => "ui/icons/lives.png".into(),
    }
}

/// Largest texture kept for a piece of art (the 4x tutorial slides are
/// 2560x1920; on the menu stage they are at most about 1300 pixels wide).
fn max_size(art: &Art) -> Option<(u32, u32)> {
    matches!(art, Art::Slide(_) | Art::SlideBackdrop).then_some((1600, 1200))
}

/// Load image `path` (an asset path, [`tt_assets::layout`]) of the asset
/// directory `root` as RGBA.
pub fn load_rgba(root: &Path, path: &str) -> Option<image::RgbaImage> {
    Some(image::open(tt_assets::find_ci(&root.join(path))?).ok()?.to_rgba8())
}

/// Load artwork from the asset directory `root` into an egui texture.
pub fn load_art(ctx: &egui::Context, root: &Path, art: &Art) -> Option<egui::TextureHandle> {
    let mut img = load_rgba(root, &art_path(art))?;
    if let Some((mw, mh)) = max_size(art) {
        if img.width() > mw || img.height() > mh {
            let k = (mw as f32 / img.width() as f32).min(mh as f32 / img.height() as f32);
            img = image::imageops::resize(&img, (img.width() as f32 * k).round() as u32, (img.height() as f32 * k).round() as u32, image::imageops::FilterType::Triangle);
        }
    }
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(format!("tt-art:{art:?}"), color, egui::TextureOptions::LINEAR))
}

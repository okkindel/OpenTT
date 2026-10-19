//! Makes the menu logo `art/logo.png` (RGBA) from the supplied artwork
//! (used as is when it is already transparent, else keyed as below)
//! `art/logo-source.png` (RGB letters on a flat blue-black background):
//!
//! ```text
//! cargo run --release -p tt-menu --example logo
//! ```
//!
//! The background colour (the median of the border) is keyed out:
//!
//! * The letters are warm (camouflage paint, rust, their dark olive sides
//!   have red >= blue) or far from the background colour (the bright
//!   rivets): opaque, with a soft ramp at the edges.
//! * The drop shadow is the background darkened (still bluish): black with
//!   the alpha that darkens the background to it.
//!
//! The colour is un-blended from the background with that alpha. The
//! result is trimmed to its content plus [`MARGIN`] pixels.

use std::path::Path;

use image::{Rgba, RgbaImage};

/// Width of the border strip the background colour is measured on.
const BORDER: u32 = 16;
/// Distance to the background (RGB, 0..441) below which a pixel is background.
const KEY_LO: f32 = 24.0;
/// Distance to the background from which a pixel is opaque.
const KEY_HI: f32 = 60.0;
/// Warmth (red - blue) from which a pixel starts to be letter...
const WARM_LO: f32 = -1.0;
/// ... and is opaque (the near-black sides of the letters are (2, 2, 0),
/// the near-black shadow (2, 3, 5)).
const WARM_HI: f32 = 3.0;
/// Darkening of the background (0..1) below which a pixel is not shadow (noise).
const SHADOW_LO: f32 = 0.35;
/// Transparent margin around the content (alpha above 2: the noise of the
/// background stays out).
const MARGIN: u32 = 8;
/// Pixels at least this opaque decide the trimmed box.
const TRIM_ALPHA: u8 = 16;


fn lum(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn background(img: &image::RgbImage) -> [f32; 3] {
    let (w, h) = img.dimensions();
    let mut ch: [Vec<u8>; 3] = Default::default();
    for (x, y, p) in img.enumerate_pixels() {
        if x < BORDER || y < BORDER || x >= w - BORDER || y >= h - BORDER {
            for k in 0..3 {
                ch[k].push(p[k]);
            }
        }
    }
    ch.map(|mut v| {
        v.sort_unstable();
        v[v.len() / 2] as f32
    })
}

/// The background of each row: the median of the row's left and right
/// border pixels, blended with the whole border's (a brushed background
/// changes from row to row).
fn row_backgrounds(img: &image::RgbImage) -> Vec<[f32; 3]> {
    let all = background(img);
    let (w, h) = img.dimensions();
    (0..h)
        .map(|y| {
            let mut ch: [Vec<u8>; 3] = Default::default();
            for dy in y.saturating_sub(2)..(y + 3).min(h) {
                for x in (0..BORDER).chain(w - BORDER..w) {
                    let p = img.get_pixel(x, dy);
                    for k in 0..3 {
                        ch[k].push(p[k]);
                    }
                }
            }
            let row = ch.map(|mut v| {
                v.sort_unstable();
                v[v.len() / 2] as f32
            });
            std::array::from_fn(|k| 0.75 * row[k] + 0.25 * all[k])
        })
        .collect()
}

fn key(img: &image::RgbImage) -> RgbaImage {
    let rows = row_backgrounds(img);
    RgbaImage::from_fn(img.width(), img.height(), |x, y| {
        let bg = rows[y as usize];
        let lb = lum(bg);
        let p = img.get_pixel(x, y);
        let c = [p[0] as f32, p[1] as f32, p[2] as f32];
        let d = (0..3).map(|k| (c[k] - bg[k]).powi(2)).sum::<f32>().sqrt();
        let letter = smooth((d - KEY_LO) / (KEY_HI - KEY_LO)).max(smooth((c[0] - c[2] - WARM_LO) / (WARM_HI - WARM_LO)));
        let shadow = smooth((1.0 - lum(c) / lb - SHADOW_LO) / (1.0 - SHADOW_LO));
        let a = letter.max(shadow);
        if a <= 0.0 {
            return Rgba([0, 0, 0, 0]);
        }
        let f = |k: usize| (bg[k] + (c[k] - bg[k]) / a).round().clamp(0.0, 255.0) as u8;
        Rgba([f(0), f(1), f(2), (a * 255.0).round() as u8])
    })
}

fn trim(img: &RgbaImage) -> RgbaImage {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        // Faint background noise (alpha below TRIM_ALPHA) does not count.
        if p[3] >= TRIM_ALPHA {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    let (w, h) = (x1 - x0 + 1 + 2 * MARGIN, y1 - y0 + 1 + 2 * MARGIN);
    let mut out = RgbaImage::new(w, h);
    image::imageops::replace(&mut out, &image::imageops::crop_imm(img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image(), MARGIN as i64, MARGIN as i64);
    out
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("art");
    let src = image::open(dir.join("logo-source.png")).expect("art/logo-source.png");
    // Artwork that already has transparency is only trimmed; artwork on a
    // flat or brushed background is keyed first.
    let out = if src.color().has_alpha() { trim(&src.to_rgba8()) } else { trim(&key(&src.to_rgb8())) };
    let path = dir.join("logo.png");
    let file = std::io::BufWriter::new(std::fs::File::create(&path).expect("art/logo.png"));
    let enc = image::codecs::png::PngEncoder::new_with_quality(file, image::codecs::png::CompressionType::Best, image::codecs::png::FilterType::Adaptive);
    out.write_with_encoder(enc).expect("encode");
    println!("{}: {}x{}", path.display(), out.width(), out.height());
}

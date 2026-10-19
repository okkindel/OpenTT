//! A mask's alpha is its luma, taken when it has the image's size (as the
//! game's former `load_rgba` did).

use image::{DynamicImage, GenericImageView, RgbaImage};
use std::path::Path;

/// Decodes an image file (format from its content).
pub fn open(path: &Path) -> Result<DynamicImage, String> {
    image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| format!("{}: {e}", path.display()))?
        .decode()
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The colour image with the alpha channel from `mask` (its luma), as RGBA.
pub fn merge_alpha(color: &DynamicImage, mask: &DynamicImage) -> RgbaImage {
    let mut img = color.to_rgba8();
    let a = mask.to_luma8();
    if a.dimensions() == img.dimensions() {
        for (p, q) in img.pixels_mut().zip(a.pixels()) {
            p.0[3] = q.0[0];
        }
    }
    img
}

/// The image as it is stored: RGBA when it has an alpha channel, else RGB.
pub fn normalized(img: DynamicImage) -> DynamicImage {
    if img.color().has_alpha() {
        DynamicImage::ImageRgba8(img.to_rgba8())
    } else {
        DynamicImage::ImageRgb8(img.to_rgb8())
    }
}

/// Writes `img` as PNG (creating the directory).
pub fn save_png(img: &DynamicImage, out: &Path) -> Result<(), String> {
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    img.save_with_format(out, image::ImageFormat::Png).map_err(|e| format!("{}: {e}", out.display()))
}

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let hp = (h / 60.0).rem_euclid(6.0);
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (r + m, g + m, b + m)
}

/// OpenTT's recolouring ([`crate::mapping::How::Recolour`]): hue set,
/// saturation raised, value scaled; `thicken` > 0 grows the alpha by a max
/// filter of that size at an image `at_width` wide (scaled to this one)
/// and paints every pixel the alpha-weighted mean colour.
pub fn recolour(img: &DynamicImage, hue: f32, min_saturation: f32, value: f32, thicken: u32, at_width: u32) -> RgbaImage {
    let mut out = img.to_rgba8();
    for p in out.pixels_mut() {
        let [r, g, b, _] = p.0;
        let (_, s, v) = rgb_to_hsv(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let (r, g, b) = hsv_to_rgb(hue, s.max(min_saturation), (v * value).min(1.0));
        let q = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
        p.0 = [q(r), q(g), q(b), p.0[3]];
    }
    if thicken > 0 && at_width > 0 {
        let (mut sum, mut wsum) = ([0.0f64; 3], 0.0f64);
        for p in out.pixels() {
            let a = p.0[3] as f64;
            for (s, c) in sum.iter_mut().zip(p.0) {
                *s += c as f64 * a;
            }
            wsum += a;
        }
        let mean: [u8; 3] = std::array::from_fn(|c| if wsum > 0.0 { (sum[c] / wsum).round() as u8 } else { 0 });
        // The max filter runs at the reference width (resampled there and
        // back), so the ring grows by the same share of the image at any
        // resolution.
        let (w, h) = out.dimensions();
        let alpha = image::GrayImage::from_fn(w, h, |x, y| image::Luma([out.get_pixel(x, y).0[3]]));
        let (rw, rh) = (at_width.max(w), (at_width.max(w) as u64 * h as u64 / w as u64).max(1) as u32);
        let big = if (rw, rh) == (w, h) { alpha } else { image::imageops::resize(&alpha, rw, rh, image::imageops::FilterType::Triangle) };
        let grown = max_filter(&big, thicken as i64 / 2);
        let back = if (rw, rh) == (w, h) { grown } else { image::imageops::resize(&grown, w, h, image::imageops::FilterType::Triangle) };
        for (p, a) in out.pixels_mut().zip(back.pixels()) {
            p.0 = [mean[0], mean[1], mean[2], a.0[0]];
        }
    }
    out
}

/// Grey max filter of radius `r` (a square window).
fn max_filter(img: &image::GrayImage, r: i64) -> image::GrayImage {
    let (w, h) = (img.width() as i64, img.height() as i64);
    image::GrayImage::from_fn(img.width(), img.height(), |x, y| {
        let mut a = 0u8;
        for dy in -r..=r {
            for dx in -r..=r {
                let (sx, sy) = (x as i64 + dx, y as i64 + dy);
                if sx >= 0 && sy >= 0 && sx < w && sy < h {
                    a = a.max(img.get_pixel(sx as u32, sy as u32).0[0]);
                }
            }
        }
        image::Luma([a])
    })
}

/// Runs the external upscaler `cmd` (a command line with `{in}` and
/// `{out}` placeholders, split on whitespace, no shell) on `img`; the
/// alpha channel is upscaled separately as a greyscale image, and
/// `wrap` (tiled terrain textures) pads the image by wrapping before and
/// crops after, so the result tiles seamlessly. `scratch` is a directory
/// for the temporary files.
pub fn upscale(cmd: &str, img: &DynamicImage, wrap: bool, scratch: &Path) -> Result<DynamicImage, String> {
    let alpha = img.color().has_alpha();
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    let up_rgb = upscale_one(cmd, &rgb, wrap, scratch, "rgb")?;
    if !alpha {
        return Ok(up_rgb);
    }
    let a = DynamicImage::ImageLuma8(image::GrayImage::from_fn(img.width(), img.height(), |x, y| image::Luma([img.get_pixel(x, y).0[3]])));
    let up_a = upscale_one(cmd, &a, wrap, scratch, "alpha")?;
    Ok(DynamicImage::ImageRgba8(merge_alpha(&up_rgb, &up_a)))
}

fn upscale_one(cmd: &str, img: &DynamicImage, wrap: bool, scratch: &Path, tag: &str) -> Result<DynamicImage, String> {
    let (w, h) = img.dimensions();
    let pad = if wrap { (w.min(h) / 8).max(1) } else { 0 };
    let src = if pad > 0 {
        let rgb = img.to_rgb8();
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(w + 2 * pad, h + 2 * pad, |x, y| *rgb.get_pixel((x + w - pad) % w, (y + h - pad) % h)))
    } else {
        img.clone()
    };
    std::fs::create_dir_all(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let input = scratch.join(format!("upscale-{tag}-in.png"));
    let output = scratch.join(format!("upscale-{tag}-out.png"));
    save_png(&src, &input)?;
    let _ = std::fs::remove_file(&output);
    let args: Vec<String> = cmd.split_whitespace().map(|a| a.replace("{in}", &input.to_string_lossy()).replace("{out}", &output.to_string_lossy())).collect();
    let (prog, rest) = args.split_first().ok_or("--upscale-cmd is empty")?;
    let status = std::process::Command::new(prog).args(rest).stdout(std::process::Stdio::null()).status().map_err(|e| format!("{prog}: {e}"))?;
    if !status.success() {
        return Err(format!("upscaler failed ({status}): {cmd}"));
    }
    let up = open(&output)?;
    let f = up.width() / src.width();
    if f == 0 || up.width() != src.width() * f || up.height() != src.height() * f {
        return Err(format!("upscaler gave {}x{} for {}x{}", up.width(), up.height(), src.width(), src.height()));
    }
    let up = if pad > 0 { up.crop_imm(pad * f, pad * f, w * f, h * f) } else { up };
    Ok(if img.color().channel_count() == 1 { DynamicImage::ImageLuma8(up.to_luma8()) } else { DynamicImage::ImageRgb8(up.to_rgb8()) })
}

/// A placeholder menu picture of a mission without one: its terrain
/// (OpenTT's own heightfield) shaded from the north-west, tinted by
/// `tint`, `w` x `h` from the middle of the map.
pub fn terrain_picture(ter: &tt_assets::terrain::Terrain, tint: [f32; 3], w: u32, h: u32) -> DynamicImage {
    let n = ter.size() as f32;
    let scale = n / w as f32;
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let gx = (x as f32 * scale) as i64;
        let gy = ((y as f32 - h as f32 / 2.0) * scale + n / 2.0) as i64;
        let hz = ter.height(gx, gy);
        let dx = ter.height(gx + 1, gy) - ter.height(gx - 1, gy);
        let dy = ter.height(gx, gy + 1) - ter.height(gx, gy - 1);
        let shade = (0.6 + (dx + dy) * -0.05).clamp(0.15, 1.0);
        let lift = (hz / 200.0).clamp(0.0, 1.0) * 0.4 + 0.6;
        image::Rgb(tint.map(|t| (t * shade * lift * 255.0).round().clamp(0.0, 255.0) as u8))
    });
    DynamicImage::ImageRgb8(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip() {
        for (r, g, b) in [(0.2, 0.4, 0.6), (1.0, 0.0, 0.0), (0.5, 0.5, 0.5), (0.9, 0.8, 0.1)] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let (r2, g2, b2) = hsv_to_rgb(h, s, v);
            assert!((r - r2).abs() < 1e-5 && (g - g2).abs() < 1e-5 && (b - b2).abs() < 1e-5, "{r} {g} {b}");
        }
    }
}

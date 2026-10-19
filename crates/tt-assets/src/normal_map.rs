//! The normal maps of the "normal maps" graphics option: for a shape or
//! terrain texture `NAME.png`, `NAME.normal.png` next to it (512 px,
//! tangent space, +Y up the image), from the texture's brightness used as
//! a height field, at three scales (fine grain, plates / rivets, rocks;
//! each blurred, Sobel, wrapping like the tiled textures) summed, so both
//! small detail and larger shapes read.
//!
//! Used by the `normal_maps` example (the repository's `assets/`) and the
//! importer (`tt-import --normal-maps`).

use std::path::{Path, PathBuf};

/// Output edge length.
pub const SIZE: u32 = 512;
/// Height-to-slope factor (texture brightness 0..1 over one output pixel).
pub const STRENGTH: f32 = 20.0;
/// Scales: box blur radius (pixels at 512, applied twice) and the weight of
/// that scale's slope (wider blurs flatten the slope, hence the larger
/// weights).
pub const SCALES: [(i32, f32); 3] = [(1, 1.0), (2, 1.2), (5, 1.5)];

/// The normal map's file of texture `file` (`a/X.png` → `a/X.normal.png`).
pub fn path_of(file: &Path) -> PathBuf {
    let stem = file.file_stem().unwrap_or_default().to_string_lossy();
    file.with_file_name(format!("{stem}.normal.png"))
}

/// Whether `file` is a texture that gets a normal map: an image, not a
/// normal map itself, not the terrain detail texture.
pub fn wants(file: &Path) -> bool {
    let name = file.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let ext_ok = [".png", ".jpg", ".bmp", ".gif"].iter().any(|x| name.ends_with(x));
    ext_ok && !name.contains(".normal.") && !file.components().any(|c| c.as_os_str() == "detail")
}

/// Every texture under `dir` that gets a normal map ([`wants`]), sorted.
pub fn textures(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if wants(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

fn blur(h: &[f32], n: usize, r: i32) -> Vec<f32> {
    let at = |x: i32, y: i32| h[(y.rem_euclid(n as i32) as usize) * n + x.rem_euclid(n as i32) as usize];
    let mut tmp = vec![0.0; n * n];
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let mut s = 0.0;
            for d in -r..=r {
                s += at(x + d, y);
            }
            tmp[y as usize * n + x as usize] = s / (2 * r + 1) as f32;
        }
    }
    let at2 = |x: i32, y: i32| tmp[(y.rem_euclid(n as i32) as usize) * n + x.rem_euclid(n as i32) as usize];
    let mut out = vec![0.0; n * n];
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let mut s = 0.0;
            for d in -r..=r {
                s += at2(x, y + d);
            }
            out[y as usize * n + x as usize] = s / (2 * r + 1) as f32;
        }
    }
    out
}

/// The normal map of a texture; `None` for images of 16 px or less on a side.
pub fn normal_map(img: &image::DynamicImage) -> Option<image::RgbImage> {
    if img.width() <= 16 || img.height() <= 16 {
        return None;
    }
    let small = img.resize_exact(SIZE, SIZE, image::imageops::FilterType::Lanczos3).to_rgb8();
    let n = SIZE as usize;
    let h: Vec<f32> = small.pixels().map(|p| (0.299 * p.0[0] as f32 + 0.587 * p.0[1] as f32 + 0.114 * p.0[2] as f32) / 255.0).collect();
    let mut grad = vec![(0.0f32, 0.0f32); n * n];
    for (r, w) in SCALES {
        let hb = blur(&blur(&h, n, r), n, r);
        let at = |x: i32, y: i32| hb[(y.rem_euclid(n as i32) as usize) * n + x.rem_euclid(n as i32) as usize];
        for y in 0..n as i32 {
            for x in 0..n as i32 {
                // Sobel; image y grows downwards.
                let dx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
                let dy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
                let g = &mut grad[y as usize * n + x as usize];
                g.0 += dx / 8.0 * w;
                g.1 += dy / 8.0 * w;
            }
        }
    }
    let mut out = image::RgbImage::new(SIZE, SIZE);
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            // Bright = high; tangent-space +Y is up the image.
            let (dx, dy) = grad[y as usize * n + x as usize];
            let (nx, ny, nz) = (-dx * STRENGTH, dy * STRENGTH, 1.0f32);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            let enc = |v: f32| ((v / len * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
            out.put_pixel(x as u32, y as u32, image::Rgb([enc(nx), enc(ny), enc(nz)]));
        }
    }
    Some(out)
}

/// The provenance note of a normal map (`SOURCES.md`).
pub fn provenance() -> String {
    format!("generated by crates/tt-assets/examples/normal_maps.rs (brightness as height, Sobel at blur radii {:?} x{STRENGTH}, {SIZE} px)", SCALES.map(|s| s.0))
}

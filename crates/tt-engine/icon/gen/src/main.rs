//! Scales the application icon `opentt-source.png` to `opentt-<size>.png`
//! (16, 32, 48, 64, 128, 256), the Windows icon `opentt.ico` (PNG images
//! of 16, 32, 48 and 256 px) and the macOS icon `opentt.icns` (PNG images
//! of 16 to 1024 px).
//!
//! ```text
//! cargo run --release --manifest-path crates/tt-engine/icon/gen/Cargo.toml -- [ICON_DIR] [--sheet PNG]
//! ```
//!
//! 64 px and up: the whole picture (Lanczos), rounded corners. 48 px and
//! below: a tighter square around the tank and its brain jar, lightly
//! sharpened, so it stays readable. `ICON_DIR` defaults to the directory
//! above this tool; `--sheet` also writes a contact sheet for review.

use std::path::{Path, PathBuf};

use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};

const LARGE: &[u32] = &[64, 128, 256];
const SMALL: &[u32] = &[16, 32, 48];
/// Sizes in the `.ico`.
const ICO: &[u32] = &[16, 32, 48, 256];
/// The tank and its jar, as fractions of the source's side: (x, y, side).
const TIGHT: (f32, f32, f32) = (0.06, 0.07, 0.88);
/// Corner radius as a fraction of the size.
const RADIUS: f32 = 0.14;

/// Anti-aliased rounded corners.
fn round(img: &mut RgbaImage) {
    let s = img.width() as f32;
    let r = (s * RADIUS).max(2.0);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
        let dx = (r - cx).max(cx - (s - r)).max(0.0);
        let dy = (r - cy).max(cy - (s - r)).max(0.0);
        let d = (dx * dx + dy * dy).sqrt() - r;
        let a = (0.5 - d).clamp(0.0, 1.0);
        p.0[3] = (p.0[3] as f32 * a).round() as u8;
    }
}

fn icon(src: &RgbaImage, size: u32) -> RgbaImage {
    let mut img = if size >= LARGE[0] {
        imageops::resize(src, size, size, FilterType::Lanczos3)
    } else {
        let n = src.width() as f32;
        let (x, y, side) = ((TIGHT.0 * n) as u32, (TIGHT.1 * n) as u32, (TIGHT.2 * n) as u32);
        let crop = imageops::crop_imm(src, x, y, side, side).to_image();
        let img = imageops::resize(&crop, size, size, FilterType::Lanczos3);
        imageops::unsharpen(&img, 0.5, 2)
    };
    round(&mut img);
    img
}

fn png(img: &RgbaImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).expect("png");
    out.into_inner()
}

/// An `.ico` whose images are PNG files (read by Windows Vista and later).
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

/// The `.icns` entries: (type, edge length in px); all PNG images
/// (macOS 10.7 and later). `ic11`..`ic14` are the Retina (@2x) variants.
const ICNS: &[(&[u8; 4], u32)] = &[
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"icp6", 64),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic13", 256),
    (b"ic09", 512),
    (b"ic14", 512),
    (b"ic10", 1024),
];

/// A macOS `.icns` of PNG images: `icns`, total length, then (type,
/// length incl. this 8-byte header, data) per image; big-endian.
fn icns(images: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (ty, png) in images {
        body.extend_from_slice(*ty);
        body.extend_from_slice(&(png.len() as u32 + 8).to_be_bytes());
        body.extend_from_slice(png);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
    out.extend(body);
    out
}

/// 256/64/32/16 px on a dark and a light band, then 32 and 16 px magnified.
fn sheet(images: &[(u32, RgbaImage)]) -> RgbaImage {
    let get = |s: u32| &images.iter().find(|(n, _)| *n == s).unwrap().1;
    let shown = [256u32, 64, 32, 16];
    let pad = 24u32;
    let row = 256 + 2 * pad;
    let width = pad + shown.iter().map(|s| s + pad).sum::<u32>();
    let mut out = RgbaImage::new(width, 3 * row);
    for (x, y, p) in out.enumerate_pixels_mut() {
        let _ = x;
        *p = match y / row {
            0 => Rgba([0x0e, 0x10, 0x13, 255]),
            1 => Rgba([0xee, 0xee, 0xee, 255]),
            _ => Rgba([0x80, 0x80, 0x80, 255]),
        };
    }
    for band in 0..2 {
        let mut x = pad;
        for s in shown {
            imageops::overlay(&mut out, get(s), x as i64, (band * row + pad + (256 - s) / 2) as i64);
            x += s + pad;
        }
    }
    let big32 = imageops::resize(get(32), 256, 256, FilterType::Nearest);
    let big16 = imageops::resize(get(16), 128, 128, FilterType::Nearest);
    imageops::overlay(&mut out, &big32, pad as i64, (2 * row + pad) as i64);
    imageops::overlay(&mut out, &big16, (2 * pad + 256) as i64, (2 * row + pad + 64) as i64);
    out
}

fn main() {
    let mut dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut sheet_out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sheet" => sheet_out = Some(args.next().expect("--sheet PNG").into()),
            _ => dir = a.into(),
        }
    }
    let dir = dir.canonicalize().unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let src_path = dir.join("opentt-source.png");
    let src = image::open(&src_path).unwrap_or_else(|e| panic!("{}: {e}", src_path.display())).to_rgba8();
    let mut images: Vec<(u32, RgbaImage)> = SMALL.iter().chain(LARGE).map(|&s| (s, icon(&src, s))).collect();
    images.sort_by_key(|(s, _)| *s);
    let mut ico_images = Vec::new();
    for (s, img) in &images {
        let data = png(img);
        let path = dir.join(format!("opentt-{s}.png"));
        std::fs::write(&path, &data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        println!("wrote {}", path.display());
        if ICO.contains(s) {
            ico_images.push((*s, data));
        }
    }
    let path = dir.join("opentt.ico");
    std::fs::write(&path, ico(&ico_images)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
    // The macOS icon: also 512 and 1024 px (the whole picture).
    let mut by_size: Vec<(u32, Vec<u8>)> = images.iter().map(|(s, img)| (*s, png(img))).collect();
    for s in [512, 1024] {
        by_size.push((s, png(&icon(&src, s))));
    }
    let icns_images: Vec<(&[u8; 4], Vec<u8>)> = ICNS.iter().map(|(ty, s)| (*ty, by_size.iter().find(|(n, _)| n == s).unwrap().1.clone())).collect();
    let path = dir.join("opentt.icns");
    std::fs::write(&path, icns(&icns_images)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
    if let Some(out) = sheet_out {
        sheet(&images).save(&out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
        println!("wrote {}", out.display());
    }
}

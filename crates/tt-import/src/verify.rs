//! Comparison of an imported directory with the repository's `assets/`:
//! what an import reproduces exactly and what it cannot (the 4x upscales,
//! the rendered pictures, the ffmpeg-made sounds).

use crate::mapping::{How, Mapping};
use std::collections::BTreeSet;
use std::path::Path;

/// One difference.
#[derive(Debug, Clone, PartialEq)]
pub struct Mismatch {
    pub class: &'static str,
    pub asset: String,
    pub detail: String,
}

/// Classes of expected differences (not errors).
pub const EXPECTED: &[&str] = &["sound regenerated", "picture placeholder"];

fn dims(p: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(p).ok()
}

/// Compares the imported directory `imported` with `assets` file by file
/// (`mapping` says which are made how). The upscaled textures are
/// expected at the game's resolution, a quarter of the repository's.
pub fn compare(imported: &Path, assets: &Path, mapping: &Mapping) -> Vec<Mismatch> {
    let mut out = Vec::new();
    let mut push = |class: &'static str, asset: &str, detail: String| out.push(Mismatch { class, asset: asset.to_string(), detail });
    let listed = |root: &Path| -> BTreeSet<String> { tt_assets::layout::files(root).into_iter().map(|(r, _)| r).filter(|r| !r.ends_with("SOURCES.md")).collect() };
    let have = listed(imported);
    let want = listed(assets);
    for a in want.difference(&have) {
        push("missing", a, "in assets/, not imported".into());
    }
    for a in have.difference(&want) {
        push("extra", a, "imported, not in assets/".into());
    }
    let same_bytes = |a: &str| std::fs::read(imported.join(a)).ok() == std::fs::read(assets.join(a)).ok();
    for a in &mapping.ours {
        if have.contains(a) && !same_bytes(a) {
            push("ours differs", a, "the built-in file was not copied as it is".into());
        }
    }
    for e in &mapping.files {
        let a = e.asset.as_str();
        if !have.contains(a) || !want.contains(a) {
            continue;
        }
        let (got, exp) = (imported.join(a), assets.join(a));
        match &e.how {
            How::Image { upscaled: true, .. } | How::Recolour { .. } | How::Picture { .. } => {
                let (Some(g), Some(w)) = (dims(&got), dims(&exp)) else {
                    push("unreadable image", a, String::new());
                    continue;
                };
                if (g.0 * 4, g.1 * 4) != w && g != w {
                    push("texture size", a, format!("{}x{} imported, {}x{} in assets/ (x4 expected)", g.0, g.1, w.0, w.1));
                }
                if matches!(e.how, How::Picture { game_file: None, .. }) {
                    push("picture placeholder", a, "a new map: a shaded-terrain placeholder (the game's import renders it)".into());
                }
            }
            How::Image { upscaled: false, .. } => {
                let (Ok(g), Ok(w)) = (image::open(&got), image::open(&exp)) else {
                    push("unreadable image", a, String::new());
                    continue;
                };
                if g.to_rgba8() != w.to_rgba8() {
                    push("pixels differ", a, format!("{}x{} vs {}x{}", g.width(), g.height(), w.width(), w.height()));
                }
            }
            How::NormalMap(_) => {
                if dims(&got) != dims(&exp) {
                    push("normal map size", a, format!("{:?} vs {:?}", dims(&got), dims(&exp)));
                }
            }
            How::Sound { .. } => {
                let ogg = std::fs::read(&got).map(|b| b.starts_with(b"OggS")).unwrap_or(false);
                push(if ogg { "sound regenerated" } else { "sound broken" }, a, "made with ffmpeg from the imported sounds; not byte-identical".into());
            }
            _ => {
                if !same_bytes(a) {
                    let detail = match (std::fs::read_to_string(&got), std::fs::read_to_string(&exp)) {
                        (Ok(g), Ok(w)) => {
                            let line = g.lines().zip(w.lines()).position(|(x, y)| x != y).map_or(g.lines().count().min(w.lines().count()) + 1, |i| i + 1);
                            format!("first difference at line {line}: {:?} vs {:?}", g.lines().nth(line - 1).unwrap_or(""), w.lines().nth(line - 1).unwrap_or(""))
                        }
                        _ => "binary content differs".into(),
                    };
                    push("bytes differ", a, detail);
                }
            }
        }
    }
    out
}

/// One line per class: count and the first few files.
pub fn summary(m: &[Mismatch]) -> Vec<String> {
    let mut classes: Vec<&str> = m.iter().map(|x| x.class).collect();
    classes.sort();
    classes.dedup();
    classes
        .iter()
        .map(|c| {
            let of: Vec<&Mismatch> = m.iter().filter(|x| x.class == *c).collect();
            let first: Vec<String> = of.iter().take(4).map(|x| format!("{} ({})", x.asset, x.detail)).collect();
            format!("{c}: {} — {}", of.len(), first.join("; "))
        })
        .collect()
}

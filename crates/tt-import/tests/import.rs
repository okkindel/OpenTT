//! The importer against the repository: the mapping covers `assets/`
//! exactly, and an import of the game install reproduces it
//! (docs/import.md § Verification).

use std::collections::BTreeSet;
use tt_import::mapping::{self, How, Mapping};
use tt_import::verify;

fn asset_files() -> BTreeSet<String> {
    tt_assets::layout::files(&tt_import::repo_assets()).into_iter().map(|(r, _)| r).filter(|r| !r.ends_with("SOURCES.md")).collect()
}

/// Whether the repository's `assets/` is the full asset directory with all
/// its provenance tables (the private repository); a checkout with only
/// OpenTT's own files there (the game data imported elsewhere) skips the tests
/// of the whole tree.
fn full_repo_assets(test: &str) -> bool {
    let full = tt_assets::testdata::present_in(&tt_import::repo_assets()) && tt_assets::testdata::mode() != tt_assets::testdata::Mode::Skip;
    if !full {
        eprintln!("{test}: skipped, the repository's assets/ holds the built-in files only");
    }
    full
}

/// `mapping.ron` is what the provenance tables say (`tt-import mapping`).
#[test]
fn mapping_is_generated_from_sources() {
    if !full_repo_assets("mapping_is_generated_from_sources") {
        return;
    }
    let generated = mapping::from_sources(&tt_import::repo_assets()).unwrap();
    assert!(generated == Mapping::builtin(), "crates/tt-import/mapping.ron is stale: run `cargo run -p tt-import -- mapping`");
}

/// Every file of `assets/` is made by the importer or is OpenTT's own, once.
#[test]
fn mapping_covers_the_asset_tree() {
    if !full_repo_assets("mapping_covers_the_asset_tree") {
        return;
    }
    let m = Mapping::builtin();
    let mut listed = BTreeSet::new();
    for a in m.files.iter().map(|e| &e.asset).chain(&m.ours) {
        assert!(listed.insert(a.clone()), "{a} listed twice");
    }
    let tree = asset_files();
    let missing: Vec<_> = tree.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(&tree).collect();
    assert!(missing.is_empty(), "in assets/ but not in the mapping: {missing:?}");
    assert!(extra.is_empty(), "in the mapping but not in assets/: {extra:?}");
    // Files made from imported ones name files that exist.
    for e in &m.files {
        let inputs: Vec<&String> = match &e.how {
            How::Recolour { from, .. } | How::NormalMap(from) => vec![from],
            How::Sound { from, .. } => from.iter().collect(),
            _ => vec![],
        };
        for i in inputs {
            assert!(listed.contains(i), "{}: input {i} is not an asset", e.asset);
        }
    }
    // OpenTT's own files are not made from the game's.
    for a in &m.ours {
        assert!(m.get(a).is_none());
    }
}

/// Imports the game install into a temporary directory and compares it
/// with `assets/`: ignored unless `TT_ORIGINAL_DIR` was set when the
/// tests were built.
#[test]
#[cfg_attr(not(tt_original), ignore = "needs the game install: set TT_ORIGINAL_DIR")]
fn import_reproduces_the_assets() {
    let Some(game) = tt_import::original_dir() else {
        eprintln!("no game install (TT_ORIGINAL_DIR): skipped");
        return;
    };
    let m = Mapping::builtin();
    for e in &m.files {
        for o in e.how.game_files() {
            assert!(tt_assets::find_ci(&game.join(&o)).is_some(), "{}: {o} not in the install", e.asset);
        }
    }
    let out = std::env::temp_dir().join(format!("tt-import-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let mut opts = tt_import::Options::new(&game, &out);
    opts.ours = Some(tt_import::repo_assets());
    let report = tt_import::import(&opts).unwrap();
    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    let mismatches = verify::compare(&out, &tt_import::repo_assets(), &m);
    for l in verify::summary(&mismatches) {
        eprintln!("{l}");
    }
    let unexpected: Vec<_> = mismatches.iter().filter(|x| !verify::EXPECTED.contains(&x.class)).collect();
    // Every name lowercase, whatever the install's case (docs/layout.md § Naming).
    let mixed: Vec<String> = tt_assets::layout::files(&out).into_iter().map(|(r, _)| r).filter(|r| *r != r.to_ascii_lowercase()).collect();
    let _ = std::fs::remove_dir_all(&out);
    assert!(mixed.is_empty(), "file names not lowercase: {mixed:?}");
    assert!(unexpected.is_empty(), "{} unexpected differences: {:?}", unexpected.len(), unexpected.iter().take(10).collect::<Vec<_>>());
}

/// A copy of the install `game` under `dir` (symbolic links to its files)
/// without the files `drop` says to leave out.
fn linked_copy(game: &std::path::Path, dir: &std::path::Path, drop: &dyn Fn(&str) -> bool) {
    for (rel, path) in tt_assets::layout::files(game) {
        if drop(&rel) {
            continue;
        }
        let to = dir.join(&rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&path, &to).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(&path, &to).unwrap();
    }
}

/// Other editions of the game, made from the game install: the
/// Shockwave.com edition imports with those skipped and listed; the Mac
/// edition's slides (PNGs with their alpha) are taken instead.
#[test]
#[cfg_attr(not(tt_original), ignore = "needs the game install: set TT_ORIGINAL_DIR")]
fn other_editions_import() {
    let Some(game) = tt_import::original_dir() else {
        eprintln!("no game install (TT_ORIGINAL_DIR): skipped");
        return;
    };
    let base = std::env::temp_dir().join(format!("tt-import-editions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let marquee = |rel: &str| rel.to_ascii_lowercase().starts_with("game/client/ui/backgrounds/marquee");
    let missions = "game/data/missions";

    // Shockwave.com: no Marquee*, TT3_2.mis in UTF-16, TT2_1.mis zero-filled.
    let sw = base.join("shockwave");
    linked_copy(&game, &sw, &|rel| marquee(rel) || rel.ends_with("TT3_2.mis") || rel.ends_with("TT2_1.mis"));
    let text = std::fs::read(game.join(missions).join("TT3_2.mis")).unwrap();
    let mut utf16 = Vec::new();
    for u in tt_assets::latin1(&text).encode_utf16() {
        utf16.extend(u.to_le_bytes());
    }
    std::fs::write(sw.join(missions).join("TT3_2.mis"), &utf16).unwrap();
    std::fs::write(sw.join(missions).join("TT2_1.mis"), vec![0u8; 5000]).unwrap();
    std::fs::write(sw.join(missions).join("TT2_1.mis.dso"), b"compiled").unwrap();
    assert_eq!(tt_import::locate::resolve(&sw).unwrap(), sw, "the slides are optional");
    let out = base.join("out-sw");
    let mut opts = tt_import::Options::new(&sw, &out);
    opts.normal_maps = false;
    opts.ffmpeg = String::new();
    let r = tt_import::import(&opts).unwrap();
    for l in r.lines() {
        eprintln!("{l}");
    }
    assert!(!r.tutorial_available());
    let slides: Vec<_> = r.skipped.iter().filter(|s| mapping::tutorial_slide(&s.asset)).collect();
    assert_eq!(slides.len(), 8, "{slides:?}");
    assert!(slides.iter().all(|s| s.reason.ends_with("tutorial disabled")));
    assert_eq!(r.missing.iter().filter(|m| m.to_ascii_lowercase().contains("marquee")).count(), 17);
    let bad = r.skipped.iter().find(|s| s.asset == "missions/tt2_1.ron").expect("tt2_1 skipped");
    assert!(bad.reason.contains("TT2_1.mis.dso"), "{}", bad.reason);
    assert!(!out.join("missions/tt2_1.ron").exists() && !out.join("ui/backgrounds/marquee1.png").exists());
    assert!(r.warnings.iter().any(|w| w.contains("TT3_2.mis") && w.contains("UTF-16")), "{:?}", r.warnings);
    let utf16_ron = std::fs::read_to_string(out.join("missions/tt3_2.ron")).unwrap();

    // Mac: Marquee<n>.png with the alpha inside instead of .jpg + .alpha.jpg.
    let mac = base.join("mac");
    linked_copy(&game, &mac, &|rel| marquee(rel));
    let bg = "game/client/ui/backgrounds";
    for n in (1..=8).map(|n| n.to_string()).chain(["Background".to_string()]) {
        let color = image::open(game.join(bg).join(format!("Marquee{n}.jpg"))).unwrap();
        let img = match image::open(game.join(bg).join(format!("Marquee{n}.alpha.jpg"))) {
            Ok(mask) => image::DynamicImage::ImageRgba8(tt_import::images::merge_alpha(&color, &mask)),
            Err(_) => color,
        };
        img.save(mac.join(bg).join(format!("marquee{n}.png"))).unwrap();
    }
    let out = base.join("out-mac");
    let mut opts = tt_import::Options::new(&mac, &out);
    opts.normal_maps = false;
    opts.ffmpeg = String::new();
    let r = tt_import::import(&opts).unwrap();
    assert!(r.tutorial_available(), "{:?}", r.skipped);
    assert!(r.skipped.is_empty() && r.missing.is_empty(), "{:?} {:?}", r.skipped, r.missing);
    let a = image::open(out.join("ui/backgrounds/marquee1.png")).unwrap().to_rgba8();
    assert!(a.pixels().any(|p| p.0[3] < 128), "the slide keeps its alpha");
    assert_eq!(std::fs::read_to_string(out.join("missions/tt3_2.ron")).unwrap(), utf16_ron, "the UTF-16 mission converts as the plain one");
    let _ = std::fs::remove_dir_all(&base);
}

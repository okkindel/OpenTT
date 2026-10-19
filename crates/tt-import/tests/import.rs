//! The importer against the repository: the mapping covers `assets/`
//! exactly, and an import of the game install reproduces it
//! (docs/import.md § Verification).

use std::collections::BTreeSet;
use tt_import::mapping::{self, How, Mapping};
use tt_import::verify;

fn asset_files() -> BTreeSet<String> {
    tt_assets::layout::files(&tt_import::repo_assets()).into_iter().map(|(r, _)| r).filter(|r| !r.ends_with("SOURCES.md")).collect()
}

/// `mapping.ron` is what the provenance tables say (`tt-import mapping`).
#[test]
fn mapping_is_generated_from_sources() {
    // The provenance tables are in the private repository only.
    if !tt_assets::testdata::require_fixture("mapping_is_generated_from_sources", &tt_import::repo_assets().join("data/SOURCES.md")) {
        return;
    }
    let generated = mapping::from_sources(&tt_import::repo_assets()).unwrap();
    assert!(generated == Mapping::builtin(), "crates/tt-import/mapping.ron is stale: run `cargo run -p tt-import -- mapping`");
}

/// Every file of `assets/` is made by the importer or is OpenTT's own, once.
#[test]
fn mapping_covers_the_asset_tree() {
    tt_assets::require_game_data!();
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
    let mismatches = verify::compare(&out, &tt_import::repo_assets(), &m, false);
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

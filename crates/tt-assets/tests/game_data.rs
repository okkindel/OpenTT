//! Parse every terrain and mission of the asset directory.

use std::path::PathBuf;

fn assets() -> PathBuf {
    tt_assets::layout::default_dir()
}

fn files(ext: &str) -> Vec<PathBuf> {
    let dir = assets().join("missions");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    v.sort();
    v
}

#[test]
fn all_terrains_parse() {
    tt_assets::require_game_data!();
    let ters = files("ter");
    // The 12 maps, OpenTT's Power-up Test and its three new maps.
    assert_eq!(ters.len(), 16);
    for p in ters {
        let t = tt_assets::terrain::Terrain::parse(&std::fs::read(&p).unwrap()).unwrap();
        assert!(t.size() == 128 || t.size() == 64, "{}", p.display());
        assert!(t.materials.iter().any(|m| !m.is_empty()));
        for m in t.materials.iter().filter(|m| !m.is_empty()) {
            assert!(tt_assets::find_texture(&assets().join(m)).is_some(), "{}: {m}", p.display());
        }
    }
}

#[test]
fn all_missions_parse() {
    tt_assets::require_game_data!();
    let root = assets();
    // Every mission file (the map list, missions/catalog.ron, is none).
    let missions: Vec<_> = files("ron").into_iter().filter(|p| !p.ends_with("catalog.ron")).collect();
    assert_eq!(missions.len(), 20);
    for p in missions {
        let m = tt_assets::data::mission::load(&p).unwrap();
        let objs = m.scene_objects(|_| true);
        let group = objs.iter().find(|o| o.name == "MissionGroup").unwrap_or_else(|| panic!("{}: no MissionGroup", p.display()));
        if let Some(t) = group.walk().into_iter().find(|o| o.class == "Terrain") {
            let file = tt_assets::resolve(&root, p.parent().unwrap(), t.get("terrain_file").unwrap());
            assert!(tt_assets::find_ci(&file).is_some(), "{}", file.display());
        }
    }
}

#[test]
fn pickup_defs() {
    tt_assets::require_game_data!();
    let blocks = tt_assets::data::defs::load_file(&assets(), "pickups").unwrap();
    let pup = blocks.iter().find(|b| b.name() == "TestPowerup").expect("TestPowerup");
    assert_eq!(pup.class(), "PowerUpDef");
    let shape = pup.to_fields().into_iter().find(|f| f.key() == "shape").map(|f| f.value);
    assert_eq!(shape.as_deref(), Some("models/items/powerup.dts"));
}

/// The game's data files, missions and example mods use the current
/// names only (no deprecation note).
#[test]
fn data_files_use_the_current_names() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut all: Vec<PathBuf> = Vec::new();
    for dir in ["assets/data", "assets/missions", "mods/carpet-bombing/data"] {
        all.extend(std::fs::read_dir(repo.join(dir)).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "ron")));
    }
    assert!(all.len() > 10, "{}", all.len());
    for p in all {
        let text = std::fs::read_to_string(&p).unwrap();
        let (_, notes) = tt_assets::data::legacy::migrate(&text);
        assert!(notes.is_empty(), "{}: {notes:?}", p.display());
    }
}

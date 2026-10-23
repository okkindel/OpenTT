//! The example mods that ship with the game (`mods/` of the repository,
//! `docs/modding.md` § Example mods): they are found where every kind of
//! install keeps them, they are marked as examples, Desert Camo's
//! lowercase textures replace the game's whatever their case, and Carpet
//! Bombing's `data/weapons.ron` is the game's file with only its intended
//! changes.
//!
//! When `assets/data/weapons.ron` changes, rewrite the mod's copy with
//! `TT_BLESS_EXAMPLE_MODS=1 cargo test -p tt-assets --test example_mods`.

use std::path::{Path, PathBuf};

use tt_assets::mods::{self, ModInfo, Overlay};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn examples() -> Vec<ModInfo> {
    mods::discover(&[repo().join("mods")]).into_iter().filter(|m| m.example).collect()
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tt-example-mods-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn copy_dir(from: &Path, to: &Path) {
    for (rel, p) in tt_assets::layout::files(from) {
        let dst = to.join(&rel);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(p, dst).unwrap();
    }
}

#[test]
fn the_examples_are_marked_and_described() {
    let all = examples();
    let ids: Vec<&str> = all.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["carpet-bombing", "desert-camo"]);
    for m in &all {
        assert!(m.warnings.is_empty(), "{}: {:?}", m.id, m.warnings);
        assert!(m.name.ends_with("(example)") && m.author == "OpenTT" && !m.description.is_empty(), "{m:?}");
        assert!(m.dir.join("README.md").is_file() && m.dir.join("SOURCES.md").is_file(), "{}", m.id);
    }
    let (bomb, camo) = (&all[0], &all[1]);
    assert!(bomb.affects_gameplay && bomb.gameplay_hash != 0);
    assert_eq!(bomb.files.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>(), ["data/weapons.ron"]);
    assert!(!camo.affects_gameplay);
    let tex: Vec<&str> = camo.files.iter().map(|(r, _)| r.as_str()).collect();
    // Each tank texture and its normal map (`tank01.normal.png` ...).
    let want: Vec<String> = (1..=4).flat_map(|n| [format!("models/tanks/tank0{n}.normal.png"), format!("models/tanks/tank0{n}.png")]).collect();
    assert_eq!(tex, want);
    for (_, p) in &camo.files {
        let img = image::open(p).unwrap();
        assert!(img.width() >= 256 && img.width() == img.height(), "{}", p.display());
    }
    // The mod's data file is a valid def file.
    let d: tt_assets::data::defs::DefFile = tt_assets::data::load(&bomb.files[0].1).unwrap();
    assert!(d.iter().any(|b| b.name() == "MortarProjectile"));
}

/// Every install form finds the examples: next to the executable
/// (`.tar.gz`, `.zip`), `<prefix>/share/<DATA_DIR>/mods` (`.deb`,
/// AppImage), `Contents/Resources/mods` (macOS); the user's own copy of
/// an example (same folder name) wins over the shipped one.
#[test]
fn installs_ship_the_examples_where_the_game_looks() {
    let base = temp("installs");
    let data = base.join("user");
    let dd = tt_assets::product::DATA_DIR;
    for (form, bin, mods_dir) in [
        ("portable", "opentt-1.0", "opentt-1.0/mods".to_string()),
        ("deb", "usr/bin", format!("usr/share/{dd}/mods")),
        ("appimage", "AppDir/usr/bin", format!("AppDir/usr/share/{dd}/mods")),
        ("macos", "OpenTT.app/Contents/MacOS", "OpenTT.app/Contents/Resources/mods".to_string()),
    ] {
        let root = base.join(form);
        std::fs::create_dir_all(root.join(bin)).unwrap();
        copy_dir(&repo().join("mods"), &root.join(&mods_dir));
        let dirs = tt_assets::layout::mod_dirs_for(&data, None, Some(&root.join(bin)));
        let found = mods::discover(&dirs);
        let ex: Vec<&str> = found.iter().filter(|m| m.example).map(|m| m.id.as_str()).collect();
        assert_eq!(ex, ["carpet-bombing", "desert-camo"], "{form}: {dirs:?}");
        assert!(found.iter().all(|m| m.dir.starts_with(root.join(&mods_dir))), "{form}");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    assert!(tt_assets::layout::installed_mod_dirs(Path::new("/opt/x")).contains(&PathBuf::from(format!("/usr/share/{dd}/mods"))));
    // The player's copy in the user mods folder comes first.
    copy_dir(&repo().join("mods/desert-camo"), &data.join("mods/desert-camo"));
    let found = mods::discover(&tt_assets::layout::mod_dirs_for(&data, None, Some(&base.join("deb/usr/bin"))));
    assert!(found.iter().find(|m| m.id == "desert-camo").unwrap().dir.starts_with(&data));
    std::fs::remove_dir_all(&base).unwrap();
}

/// Desert Camo's file names are lowercase; they replace the game's
/// textures whatever case the game's names have: the game data's own are
/// lowercase, game data imported by an older build has `Tank01.png`.
#[test]
fn lowercase_textures_replace_the_games() {
    let camo = examples().into_iter().find(|m| m.id == "desert-camo").unwrap();
    let base = temp("case");
    let root = base.join("assets");
    // The names of an older build's import.
    let names = ["Tank01.png", "Tank02.png", "tank03.png", "tank04.png"];
    assert!(names.iter().all(|n| camo.files.iter().any(|(r, _)| r.rsplit('/').next().unwrap().eq_ignore_ascii_case(n))), "{:?}", camo.files);
    for n in &names {
        let p = root.join("models/tanks").join(n);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "game").unwrap();
    }
    mods::install(Overlay::new(&root, &[&camo]));
    for n in &names {
        let got = tt_assets::find_ci(&root.join("models/tanks").join(n)).unwrap();
        assert!(got.starts_with(&camo.dir), "{n}: {}", got.display());
        let stem = n.trim_end_matches(".png");
        assert!(tt_assets::find_texture(&root.join("models/tanks").join(stem)).unwrap().starts_with(&camo.dir), "{stem}");
    }
    mods::install(Overlay::new(&root, &[]));
    std::fs::remove_dir_all(&base).unwrap();
}

/// The fields of `MortarProjectile` the Carpet Bombing mod sets, in the
/// order new ones are added at the end of its block.
const CARPET: &[(&str, &str)] = &[
    ("blast_radius", "8"),
    ("rounds", "12"),
    ("reload_ms", "250"),
    ("volley_delay_ms", "1500"),
    ("volley_size", "3"),
    ("damage", "3"),
    ("pellet_count", "9"),
    ("spread_angle", "18"),
    ("spread_ring", "1"),
    ("spread_inner", "0.5"),
    ("spread_jitter", "1"),
];

/// The game's `weapons.ron` with [`CARPET`] applied to `MortarProjectile`.
fn carpet_weapons(game: &str) -> String {
    let start = game.find("    name: \"MortarProjectile\",").expect("MortarProjectile in weapons.ron");
    let end = start + game[start..].find("\n  ),").expect("the end of MortarProjectile");
    let mut lines: Vec<String> = game[start..end].lines().map(String::from).collect();
    for (k, v) in CARPET {
        let line = format!("    {k}: {v},");
        match lines.iter().position(|l| l.starts_with(&format!("    {k}: "))) {
            Some(i) => lines[i] = line,
            None => lines.push(line),
        }
    }
    format!("{}{}{}", &game[..start], lines.join("\n"), &game[end..])
}

#[test]
fn carpet_bombing_follows_the_games_weapons() {
    // Line endings as checked out (a Windows checkout may have CRLF).
    let game = std::fs::read_to_string(repo().join("assets/data/weapons.ron")).unwrap().replace("\r\n", "\n");
    let path = repo().join("mods/carpet-bombing/data/weapons.ron");
    let ours = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    // The mod's note: its comment lines before the game file's first line.
    let first = game.lines().next().unwrap();
    let note: String = ours.lines().take_while(|l| *l != first && l.starts_with("//")).map(|l| format!("{l}\n")).collect();
    assert!(note.contains("Carpet Bombing"), "the note at the top of {}", path.display());
    let want = note + &carpet_weapons(&game);
    if std::env::var_os("TT_BLESS_EXAMPLE_MODS").is_some() {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    assert!(
        ours == want,
        "{} differs from assets/data/weapons.ron in more than the mod's changes ({CARPET:?}): rewrite it with TT_BLESS_EXAMPLE_MODS=1 cargo test -p tt-assets --test example_mods",
        path.display()
    );
    // Each change is a change.
    for (k, v) in CARPET {
        let block = &game[game.find("name: \"MortarProjectile\"").unwrap()..];
        let block = &block[..block.find("\n  ),").unwrap()];
        assert!(!block.contains(&format!("    {k}: {v},")), "{k}: {v} is the game's value already");
    }
}

/// The packages ship every file of every example: the `.deb` lists each in
/// `[package.metadata.deb]` (cargo-deb has no folder copy), the other
/// packages copy the folders `packaging/common.sh` names.
#[test]
fn the_packages_ship_the_examples() {
    let manifest = std::fs::read_to_string(repo().join("crates/tt-engine/Cargo.toml")).unwrap();
    let common = std::fs::read_to_string(repo().join("packaging/common.sh")).unwrap();
    let listed = common.lines().find_map(|l| l.strip_prefix("EXAMPLE_MODS=")).expect("EXAMPLE_MODS in packaging/common.sh").trim_matches('"').to_string();
    // Every example mod ships, and the finished mods listed beside them.
    let listed: Vec<&str> = listed.split_whitespace().collect();
    for m in examples() {
        assert!(listed.contains(&m.id.as_str()), "EXAMPLE_MODS in packaging/common.sh: add {}", m.id);
    }
    for id in listed {
        let dir = repo().join("mods").join(id);
        assert!(dir.join("mod.ron").is_file(), "packaging/common.sh lists mods/{id}, which has no mod.ron");
        for (rel, _) in tt_assets::layout::files(&dir) {
            let row = format!("[\"../../mods/{id}/{rel}\", \"usr/share/opentt/mods/{id}/{rel}\", \"644\"],");
            assert!(manifest.contains(&row), "crates/tt-engine/Cargo.toml [package.metadata.deb] assets: add {row}");
        }
    }
}

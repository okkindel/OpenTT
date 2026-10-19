//! The directory is copied to a temporary one first and never written.

use std::path::{Path, PathBuf};

use tt_shell::save::{self, DirStore};
use tt_shell::{legacy, HiScores, Players, Prefs, Settings};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

fn latin1(p: &Path) -> String {
    std::fs::read(p).map(|b| b.iter().map(|&c| c as char).collect()).unwrap_or_default()
}

#[test]
fn import_a_real_save_dir() {
    let Some(src) = std::env::var_os("TT_IMPORT_DIR").map(PathBuf::from) else {
        eprintln!("TT_IMPORT_DIR not set: skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("tt-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&src, &dir);
    let cs = |f: &str| latin1(&dir.join(f));
    let old_files: Vec<(String, Vec<u8>)> = [save::LEGACY_PLAYERS, save::LEGACY_HISCORES, save::LEGACY_CLIENT_PREFS, save::LEGACY_SERVER_PREFS, save::LEGACY_PROGRESS, save::LEGACY_CONFIG]
        .iter()
        .filter_map(|f| std::fs::read(dir.join(f)).ok().map(|b| (f.to_string(), b)))
        .collect();

    let mut store = DirStore(dir.clone());
    let (saves, report) = save::load(&mut store);
    for l in report.lines() {
        eprintln!("{l}");
    }
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let players = Players::from_legacy(&cs(save::LEGACY_PLAYERS));
    assert_eq!(saves.players.names(), players.names());
    assert_eq!(saves.players.last_selection(), players.last_selection());
    for i in 0..players.count() {
        assert_eq!(saves.players.stats(i), players.stats(i), "player {i}");
        eprintln!("player {i}: {:?}", saves.players.stats(i));
    }
    let hiscores = HiScores::from_legacy(&cs(save::LEGACY_HISCORES));
    assert_eq!(saves.hiscores.entries(), hiscores.entries());
    let mut vars = legacy::parse(&cs(save::LEGACY_CLIENT_PREFS));
    vars.merge(&legacy::parse(&cs(save::LEGACY_SERVER_PREFS)));
    if let Some(s) = &saves.settings {
        let back = s.to_vars();
        const LINK: &str = "input.link_mouse_sensitivity";
        const MOUSE: &str = tt_shell::prefs::MOUSE_SENSITIVITY;
        for n in vars.names() {
            assert_eq!(back.get(n), vars.get(n), "{n}");
        }
        if vars.is_set(LINK) && !vars.is_set(MOUSE) {
            // Migrated to OpenTT's slider.
            assert_eq!(back.get_f64(MOUSE) as f32, tt_shell::prefs::migrate_mouse_sensitivity(vars.get_f64(LINK)) as f32);
            eprintln!("mouse sensitivity {} -> {}", vars.get(LINK), back.get(MOUSE));
        }
        assert!(back.names().count() >= vars.names().count());
        let p = Prefs::with_settings(s);
        eprintln!("settings: player {:?}, {:?} {:?}, {} other prefs", p.player_name(), p.resolution(), p.window_mode(), s.other.len());
        // The written file reads back to the same settings.
        let again = Settings::from_ron(&std::fs::read_to_string(dir.join(save::SETTINGS)).unwrap()).unwrap();
        assert_eq!(&again, s);
    }

    // A second load imports nothing.
    for (f, b) in &old_files {
        assert_eq!(&std::fs::read(dir.join(f)).unwrap(), b, "{f} unchanged");
    }
    let (again, report) = save::load(&mut store);
    assert!(report.imported.is_empty() && report.errors.is_empty(), "{report:?}");
    assert_eq!(again.players.vars.to_text(""), saves.players.vars.to_text(""), "the first start reads what the next ones do");
    assert_eq!(again.hiscores.entries(), saves.hiscores.entries());
    if std::env::var_os("TT_IMPORT_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        eprintln!("kept {}", dir.display());
    }
}

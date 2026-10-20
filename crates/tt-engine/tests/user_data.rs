//! `TT_MIGRATE_DIR=<dir> cargo test -p tt-engine --test user_data` also
//! migrates a copy of a real data directory (the directory itself is never
//! written) and prints what was done.

use std::path::{Path, PathBuf};

use tt_engine::{Config, Engine};

mod common;
use common::*;

fn put(dir: &Path, rel: &str, bytes: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

#[test]
fn the_engine_migrates_before_the_boot() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let data = std::env::temp_dir().join(format!("tt-engine-test-userdata-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let mut prefs = tt_shell::Prefs::defaults();
        let mut p = tt_shell::Players::new();
        p.create("Bait", &mut prefs).unwrap();
        p.prepare_save();
        put(&data, tt_shell::save::LEGACY_PLAYERS, p.to_legacy().as_bytes());
        put(&data, tt_shell::save::LEGACY_CLIENT_PREFS, tt_shell::legacy::prefs_file(&[("player.name", "Bait")]).as_bytes());
        put(&data, tt_shell::save::LEGACY_CONFIG, b"moveMap.bind(keyboard, \"f9\", moveforward);\r\n");
        put(&data, tt_shell::save::LEGACY_BANLISTS[0], tt_shell::legacy::banlist_file(&[tt_shell::banlist::Ban { guid: "0".into(), ip: "10.0.0.2".into(), until: 4000000000 }]).as_bytes());
        let cfg = Config { assets: tt_assets::layout::default_dir(), data: data.clone(), args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
        let mut e = Engine::new(&cfg).unwrap();
        assert!(e.boot());
        let log: Vec<&str> = e.host.log.history.iter().map(|(_, m)| m.as_str()).filter(|m| m.starts_with("user data: ")).collect();
        let (players, config) = (format!("imported {} into players.ron", tt_shell::save::LEGACY_PLAYERS), format!("imported {} into binds.ron", tt_shell::save::LEGACY_CONFIG));
        for want in [players.as_str(), config.as_str(), "removed the empty directory game"] {
            assert!(log.iter().any(|l| l.ends_with(want)), "{want}: {log:#?}");
        }
        assert_eq!(e.host.shell.players.names(), ["Bait"]);
        assert_eq!(e.host.shell.prefs.player_name(), "Bait");
        let f9 = e.host.input.bindings("MoveMap").iter().find(|b| b.action == "f9").map(|b| b.command.action());
        assert_eq!(f9, Some(tt_gameplay::client::binds::BindAction::MoveForward));
        tt_engine::netgame::admin::load(&mut e.host);
        assert!(tt_engine::netgame::admin::is_banned(&e.host, "10.0.0.2"));
        assert!(!data.join("game").exists() && data.join(tt_shell::save::LEGACY_DIR).join(tt_shell::save::LEGACY_PLAYERS).exists());
        assert!(std::fs::read_to_string(data.join("logs/console.log")).unwrap().contains("user data: imported"));
        e.shutdown();
        drop(e);
        // The next start has nothing to migrate.
        let e = Engine::new(&cfg).unwrap();
        assert!(!e.host.log.history.iter().any(|(_, m)| m.starts_with("user data: ")));
        let _ = std::fs::remove_dir_all(&data);
    });
}

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

#[test]
fn migrate_a_copy_of_a_real_data_dir() {
    let Some(src) = std::env::var_os("TT_MIGRATE_DIR").map(PathBuf::from) else {
        eprintln!("TT_MIGRATE_DIR not set: skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("tt-migrate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&src, &dir);
    let m = tt_engine::userdata::migrate(&dir, &tt_assets::layout::default_dir());
    for l in &m.done {
        eprintln!("done: {l}");
    }
    for l in &m.errors {
        eprintln!("error: {l}");
    }
    assert!(m.errors.is_empty());
    let s = tt_shell::save::read_dir(&dir);
    eprintln!("players {:?}, settings {}", s.players.names(), s.settings.is_some());
    assert_eq!(tt_engine::userdata::migrate(&dir, &tt_assets::layout::default_dir()), Default::default(), "idempotent");
    if std::env::var_os("TT_MIGRATE_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        eprintln!("kept {}", dir.display());
    }
}

//! The game without its data: the first-run screen's state, and the
//! import through the same API the screen uses (`gamedata::SetupFlow`)
//! followed by a mission booted headless on the result
//! (`docs/import.md` § The game's import).

use std::path::{Path, PathBuf};
use tt_engine::gamedata::{self, Outcome, SetupFlow};
use tt_engine::{launcher, mission, Config, Engine};
use tt_menu::{SetupAction, SetupMode};

mod common;
use common::*;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tt-game-data-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// One frame of the screen's UI, headless; returns its action.
fn frame(flow: &mut SetupFlow) -> Option<SetupAction> {
    let ctx = egui::Context::default();
    tt_menu::theme::install_fonts(&ctx);
    let mut action = None;
    for _ in 0..2 {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| action = flow.ui.ui(ctx));
    }
    action
}

#[test]
fn empty_asset_dir_starts_at_the_first_run_screen() {
    let base = temp("empty");
    let assets = base.join("assets");
    std::fs::create_dir_all(assets.join("missions")).unwrap();
    // Our own files only (as in a public checkout): still no game data.
    std::fs::write(assets.join("missions/tt1_5.ron"), b"").unwrap();
    assert!(!gamedata::present(&assets));
    let target = base.join("gamedata");
    let factory: launcher::AppFactory = Box::new(|_: &Path| Err("not in this test".into()));
    let l = launcher::Launcher::first_run(assets.clone(), target.clone(), factory);
    let flow = l.setup().expect("the first-run screen");
    assert_eq!(flow.ui.mode, SetupMode::FirstRun);
    assert_eq!(flow.target, target);
    assert_eq!(flow.ui.target, target.display().to_string());
    assert!(flow.ui.path.is_empty() && flow.ui.error.is_none() && flow.ui.progress.is_none());
    assert!(!flow.importing());

    // The screen draws headless and waits for the player.
    let mut flow = SetupFlow::new(SetupMode::FirstRun, target.clone(), None);
    assert_eq!(frame(&mut flow), None);
    // A folder that is no install: the reason, no import.
    flow.handle(SetupAction::Import(assets.display().to_string()));
    let e = flow.ui.error.clone().unwrap_or_default();
    assert!(e.contains("\"game\" folder"), "{e}");
    assert!(!flow.importing() && flow.outcome.is_none());
    // An install without its files: every missing file in the report and
    // the log, the log's place in the message; Continue goes back.
    let bare = base.join("bare");
    std::fs::create_dir_all(bare.join("game/data/missions")).unwrap();
    std::fs::create_dir_all(bare.join("game/server/scripts")).unwrap();
    flow.log_dir = Some(base.join("logs"));
    flow.handle(SetupAction::Import(bare.display().to_string()));
    assert!(!flow.importing());
    let missing = tt_import::locate::missing_required(&bare, &tt_import::mapping::Mapping::builtin());
    let r = flow.ui.report.clone().expect("the report");
    assert!(!r.ok && r.lines.len() == missing.len() + 1, "{} lines for {} files", r.lines.len(), missing.len());
    let e = flow.ui.error.clone().unwrap_or_default();
    assert!(e.contains("incomplete") && e.contains("import.log"), "{e}");
    let log = std::fs::read_to_string(base.join("logs/import.log")).unwrap();
    assert!(missing.iter().all(|m| log.contains(m.as_str())) && log.contains("result: failed"));
    assert_eq!(frame(&mut flow), None);
    flow.handle(SetupAction::Continue);
    assert!(flow.ui.report.is_none() && flow.outcome.is_none());
    // Quit leaves.
    flow.handle(SetupAction::Quit);
    assert!(matches!(flow.outcome, Some(Outcome::Quit)));

    // A windowless run on a directory without any data fails, saying how to get it.
    let nothing = base.join("nothing");
    std::fs::create_dir_all(&nothing).unwrap();
    let out = std::process::Command::new(game_exe()).args(["--assets"]).arg(&nothing).args(["--data"]).arg(base.join("data")).args(["--headless", "--ticks", "1", "--no-sound", "--quiet"]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no game data") && err.contains("--import"), "{err}");
    assert!(!target.exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// Imports `$TT_ORIGINAL_DIR` into a temporary `gamedata/` the way the
/// screen does (progress, all or nothing), checks Cancel, then boots a
/// mission headless on the result. Ignored unless `TT_ORIGINAL_DIR` was set
/// when the tests were built.
#[test]
#[cfg_attr(not(tt_original), ignore = "needs the game install: set TT_ORIGINAL_DIR")]
fn import_flow_then_a_mission() {
    let Some(source) = std::env::var_os("TT_ORIGINAL_DIR").map(PathBuf::from).filter(|p| p.is_dir()) else {
        eprintln!("no game install (TT_ORIGINAL_DIR): skipped");
        return;
    };
    let base = temp("import");
    // Cancel right after the start: nothing is left behind.
    let cancelled = base.join("cancelled");
    let mut flow = SetupFlow::new(SetupMode::FirstRun, cancelled.clone(), None);
    flow.pictures = false;
    flow.handle(SetupAction::Import(source.join("game").display().to_string()));
    assert!(flow.importing(), "{:?}", flow.ui.error);
    flow.handle(SetupAction::Cancel);
    while flow.importing() {
        std::thread::sleep(std::time::Duration::from_millis(20));
        flow.poll();
    }
    assert!(flow.outcome.is_none());
    assert!(flow.ui.note.as_deref().unwrap_or("").contains("cancelled"), "{:?} {:?}", flow.ui.note, flow.ui.error);
    assert!(!cancelled.exists() && !tt_import::import::sibling(&cancelled, ".tmp").exists());

    // The whole import (the screen's options; the pictures are not rendered here).
    let target = base.join("gamedata");
    let mut flow = SetupFlow::new(SetupMode::FirstRun, target.clone(), None);
    flow.pictures = false;
    flow.log_dir = Some(base.join("logs"));
    flow.handle(SetupAction::Import(source.display().to_string()));
    let mut seen = Vec::new();
    let mut last_done = 0;
    while flow.importing() {
        std::thread::sleep(std::time::Duration::from_millis(50));
        flow.poll();
        if let Some(p) = &flow.ui.progress {
            assert!(p.done >= last_done && p.done <= p.total, "{p:?}");
            last_done = p.done;
            if !seen.contains(&p.phase) {
                seen.push(p.phase.clone());
            }
        }
    }
    // The report is shown (and written to the log folder) until Continue.
    assert!(flow.outcome.is_none());
    let shown = flow.ui.report.clone().unwrap_or_else(|| panic!("import failed: {:?}", flow.ui.error));
    assert!(shown.ok && shown.summary.contains("files imported"), "{shown:?}");
    assert!(std::fs::read_to_string(base.join("logs/import.log")).unwrap().contains("result: imported"));
    flow.handle(SetupAction::Continue);
    assert!(flow.ui.report.is_none());
    let Some(Outcome::Imported { assets, source: from, report }) = flow.outcome.take() else { panic!("import failed: {:?}", flow.ui.error) };
    eprintln!("{} files; phases seen: {seen:?}; warnings: {:?}", report.written, report.warnings);
    assert_eq!(assets, target);
    assert_eq!(from, source);
    assert!(seen.len() >= 3, "{seen:?}");
    assert!(gamedata::present(&target));
    assert!(!tt_import::import::sibling(&target, ".tmp").exists());
    assert!(target.join("missions/tt1_5.ron").exists(), "the new maps are built in");

    // A mission headless on the imported data; the source goes to the settings.
    let data = base.join("data");
    big_stack(move || {
        let cfg = Config { assets: target.clone(), data, args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
        let mut e = Engine::new(&cfg).unwrap();
        assert!(e.boot());
        gamedata::record_source(&mut e.host, &source);
        assert_eq!(e.host.shell.prefs.get(gamedata::SOURCE_KEY), source.display().to_string());
        mission::load_dev_mission(&mut e, "missions/tt1_1.ron").unwrap();
        for _ in 0..60 {
            e.tick();
        }
        assert!(e.host.game.is_some(), "the mission's simulation runs");
        let (objects, _) = mission::mission_objects(&e.host).unwrap();
        assert!(objects.iter().any(|o| o.class == "Terrain"));
        e.shutdown();
        let settings = std::fs::read_to_string(e.host.fs.data_dir().join("settings.ron")).unwrap();
        assert!(settings.contains("game_data.source"), "{settings}");
    });
    let _ = std::fs::remove_dir_all(&base);
}

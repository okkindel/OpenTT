//! * `TT_GOLDEN_BLESS=1` re-records: each scenario is played twice, the two
//!   traces must be identical (determinism), then the fixture is written.
//!   Afterwards `cargo test --release -p tt-engine --test golden
//!   write_manifest -- --ignored` rewrites `MANIFEST`. Only for stage 0 and
//!   deliberate, reviewed behaviour changes.
//! * `TT_GOLDEN_DUMP=<dir>`: the trace of every scenario run is also
//!   written there (`<name>.trace`).
//!
//! Needs only the asset directory.

use std::path::PathBuf;
use tt_engine::gameplay::golden::record::fnv;
use tt_engine::gameplay::golden::scenarios;
use tt_engine::gameplay::golden::first_difference;

mod common;
use common::*;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn text(lines: &[String]) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

fn check(name: &'static str) {
    // The traces are recordings on the game data: both must be there
    // (not in a checkout without the game files).
    tt_assets::require_game_data!();
    if std::env::var_os("TT_GOLDEN_BLESS").is_none() && !tt_assets::testdata::require_fixture(name, &dir().join(format!("{name}.trace"))) {
        return;
    }
    big_stack(move || {
        let sc = scenarios::by_name(name).unwrap_or_else(|| panic!("no scenario {name}"));
        let lines = sc.run().unwrap_or_else(|e| panic!("{name}: {e}"));
        if let Ok(d) = std::env::var("TT_GOLDEN_DUMP") {
            let _ = std::fs::create_dir_all(&d);
            std::fs::write(PathBuf::from(d).join(format!("{name}.trace")), text(&lines)).unwrap();
        }
        let path = dir().join(format!("{name}.trace"));
        if std::env::var_os("TT_GOLDEN_BLESS").is_some() {
            let again = sc.run().unwrap_or_else(|e| panic!("{name}: {e}"));
            if let Some(d) = first_difference(&lines, &again, 20) {
                panic!("{name} is not deterministic: {d}");
            }
            std::fs::create_dir_all(dir()).unwrap();
            std::fs::write(&path, text(&lines)).unwrap();
            println!("{name}: blessed, {} lines", lines.len());
            return;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} (record with TT_GOLDEN_BLESS=1)", path.display()));
        let want: Vec<String> = want.lines().map(String::from).collect();
        if let Some(d) = first_difference(&want, &lines, 20) {
            panic!("{name}: the trace differs from the golden fixture (A = fixture, B = this build)\n{d}");
        }
    });
}

macro_rules! golden {
    ($($f:ident => $n:literal,)*) => {
        $(#[test] fn $f() { check($n) })*
        const NAMES: &[&str] = &[$($n),*];
    };
}

golden! {
    quick_tt1_1 => "quick-tt1_1",
    battle_tt1_1 => "battle-tt1_1",
    team_battle_tt1_1 => "team-battle-tt1_1",
    scrum_tt1_1 => "scrum-tt1_1",
    team_scrum_tt1_1 => "team-scrum-tt1_1",
    battle_tt2_3 => "battle-tt2_3",
    team_battle_tt2_3 => "team-battle-tt2_3",
    scrum_tt2_3 => "scrum-tt2_3",
    team_scrum_tt2_3 => "team-scrum-tt2_3",
    battle_tt3_1 => "battle-tt3_1",
    team_battle_tt3_1 => "team-battle-tt3_1",
    scrum_tt3_1 => "scrum-tt3_1",
    team_scrum_tt3_1 => "team-scrum-tt3_1",
    battle_tt2_1 => "battle-tt2_1",
    team_battle_tt2_1 => "team-battle-tt2_1",
    scrum_tt2_1 => "scrum-tt2_1",
    team_scrum_tt2_1 => "team-scrum-tt2_1",
    bots_tt1_2 => "bots-tt1_2",
    solo => "solo",
    target => "target",
    demo => "demo",
    launch_quick => "launch-quick",
    launch_mp => "launch-mp",
    launch_ded => "launch-ded",
    fx_team_battle_tt1_1 => "fx-team-battle-tt1_1",
    fx_battle_tt2_3 => "fx-battle-tt2_3",
    fx_team_scrum_tt3_1 => "fx-team-scrum-tt3_1",
    fx_solo => "fx-solo",
    fx_target => "fx-target",
    fx_server_tt1_3 => "fx-server-tt1_3",
    client_team_tt1_1 => "client-team-tt1_1",
    client_ffa_tt2_3 => "client-ffa-tt2_3",
    boot_connection => "boot-connection",
    solo_select_1 => "solo-select-1",
    solo_select_7 => "solo-select-7",
    solo_select_20 => "solo-select-20",
    kda_fight => "kda-fight",
    powerup_test_0 => "powerup-test-0",
    powerup_test_2 => "powerup-test-2",
    health_floor_tt2_2 => "health-floor-tt2_2",
    netplay_lan => "netplay-lan",
    netplay_dsl_scrum => "netplay-dsl-scrum",
}

/// Every scenario has a test above, and MANIFEST matches the fixtures.
#[test]
fn manifest_matches() {
    let all: Vec<String> = scenarios::all().iter().map(|e| e.name().to_string()).collect();
    assert_eq!(all, NAMES.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "the test list and scenarios::all() differ");
    if std::env::var_os("TT_GOLDEN_BLESS").is_some() || !tt_assets::testdata::require_fixture("manifest_matches", &dir().join("MANIFEST")) {
        return;
    }
    let manifest = std::fs::read_to_string(dir().join("MANIFEST")).expect("tests/golden/MANIFEST");
    let rows: Vec<Vec<&str>> = manifest.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).map(|l| l.split_whitespace().collect()).collect();
    assert_eq!(rows.iter().map(|r| r[0]).collect::<Vec<_>>(), NAMES, "MANIFEST scenarios");
    for r in rows {
        let b = std::fs::read(dir().join(format!("{}.trace", r[0]))).unwrap();
        let lines = b.iter().filter(|&&c| c == b'\n').count();
        assert_eq!((r[2], r[3], r[4]), (lines.to_string().as_str(), b.len().to_string().as_str(), format!("{:016x}", fnv(&b)).as_str()), "{}: MANIFEST row", r[0]);
    }
}

/// Rewrites MANIFEST from the fixtures (after a bless).
#[test]
#[ignore]
fn write_manifest() {
    let commit = std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(env!("CARGO_MANIFEST_DIR")).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let mut s = String::from("# Golden behaviour traces (docs/rust-native-plan.md §4; tests/golden.rs).\n");
    s += &format!("# Recorded from build {commit} (tag vm-final-reference: this commit plus the fixtures).\n");
    s += "# scenario seed lines bytes fnv1a64\n";
    for e in scenarios::all() {
        let b = std::fs::read(dir().join(format!("{}.trace", e.name()))).unwrap_or_else(|_| panic!("{}: no fixture", e.name()));
        let lines = b.iter().filter(|&&c| c == b'\n').count();
        s += &format!("{} {} {} {} {:016x}\n", e.name(), e.seed(), lines, b.len(), fnv(&b));
    }
    std::fs::write(dir().join("MANIFEST"), s).unwrap();
}

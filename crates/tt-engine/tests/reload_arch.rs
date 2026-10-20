//! The sequence (0.97 s, 30 keyframes) shows the `electro` object (hidden by default) between the
//! towers from keyframe 4 to 18, with its IFL lightning; the thread then stops on its last keyframe
//! (hidden again).
//!
//! `reload_arch_shots` saves offscreen frames of it for review:
//! `TT_ARCH_SHOTS=out/reload-arch cargo test --release -p tt-engine --test reload_arch -- --ignored --test-threads=1`

use glam::Vec3;
use std::path::{Path, PathBuf};
use tt_engine::{Config, Engine};
use tt_menu::{CustomGame, MenuAction, Team};

mod common;
use common::*;

const MISSION: &str = "missions/poweruptest.ron";
const ARCH: &str = "PupReload";

/// Draws every tick into an offscreen target when there is one.
struct View {
    off: tt_engine::gfx::Offscreen,
    fe: tt_engine::frontend::Frontend,
    dir: PathBuf,
}

fn step(e: &mut Engine, v: &mut Option<View>) {
    match v {
        Some(v) => {
            let t = e.host.virtual_clock_ms.unwrap_or(0.0) + tt_engine::TICK_MS as f64;
            e.host.virtual_clock_ms = Some(t);
            let ms = tt_engine::TICK_MS as f32;
            v.fe.step(e, &v.off.device, &v.off.queue, ms);
            v.fe.draw(e, &v.off.device, &v.off.queue, &v.off.view, v.off.size, ms);
        }
        None => e.tick(),
    }
}

fn steps(e: &mut Engine, v: &mut Option<View>, ms: u32) {
    for _ in 0..ms / tt_engine::TICK_MS {
        step(e, v);
    }
}

/// The Power-up Test map in battle mode, the player spawned and landed.
fn start(root: &Path, tag: &str, view: bool) -> (Engine, Option<View>) {
    let data = std::env::temp_dir().join(format!("tt-engine-test-arch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let cfg = Config { assets: root.to_path_buf(), data, args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    let mut v = None;
    if view {
        e.host.virtual_clock_ms = Some(0.0);
    }
    assert!(e.boot());
    e.set_var("input.aim_mode", "0");
    if let Some(dir) = view.then(|| std::env::var_os("TT_ARCH_SHOTS").map(PathBuf::from)).flatten() {
        let off = tt_engine::gfx::Offscreen::new((1280, 720)).unwrap();
        let fe = tt_engine::frontend::Frontend::new(&off.device, &off.queue, off.format, root.to_path_buf(), false);
        v = Some(View { off, fe, dir });
    }
    steps(&mut e, &mut v, 1000);
    new_player(&mut e, "Tester");
    let g = CustomGame { mission: MISSION.into(), mode: 0, bots: Vec::new(), player_tank: 1, player_team: Team::Green, minutes: 10, replace_bots: true };
    apply(&mut e, MenuAction::StartCustom(g));
    until_spawned(&mut e, |e| step(e, &mut v));
    steps(&mut e, &mut v, 3000);
    (e, v)
}

fn arch(e: &Engine) -> &tt_game::Trigger {
    let g = e.host.game.as_ref().unwrap();
    g.world.triggers.iter().find(|t| t.name.eq_ignore_ascii_case(ARCH)).expect("the Reload arch")
}

/// The arch as drawn: (`electro` shown, `electro` explicitly hidden, IFL
/// times of the electro material).
fn electro(e: &Engine) -> (bool, bool, Vec<f32>) {
    let (shapes, anims) = e.trigger_shapes_with_anim();
    let i = shapes.iter().position(|s| s.shape.to_string_lossy().to_ascii_lowercase().ends_with("recharge.dts")).expect("the arch is drawn");
    let has = |v: &[String]| v.iter().any(|o| o.eq_ignore_ascii_case("electro"));
    let ifl = anims[i].ifl.iter().filter(|(m, _)| m.to_ascii_lowercase().starts_with("electro")).map(|x| x.1).collect();
    (has(&shapes[i].shown), has(&shapes[i].hidden), ifl)
}

/// Puts the tank `back` metres south of the arch, facing north, landed.
fn place_before(e: &mut Engine, v: &mut Option<View>, back: f32) {
    let p: Vec3 = arch(e).transform.translation.into();
    let (o, id) = player(e).unwrap();
    let xf = format!("{} {} {} 0 0 1 0", p.x, p.y - back, p.z + 1.0);
    let g = e.host.game.as_mut().unwrap();
    g.set_transform(o, &xf);
    if let Some(t) = g.world.tank_mut(id) {
        t.vel = Vec3::ZERO;
    }
    steps(e, v, 2600);
}

fn threads(e: &Engine) -> u32 {
    e.host.game.as_ref().unwrap().event_counts.get("UserThreadPlayed").copied().unwrap_or(0) as u32
}

/// Drives north until the arch's thread starts (max 4 s); returns whether it did.
fn drive_through(e: &mut Engine, v: &mut Option<View>) -> bool {
    e.key("w", true, None);
    for _ in 0..4000 / tt_engine::TICK_MS {
        step(e, v);
        if arch(e).thread.is_some() {
            e.key("w", false, None);
            return true;
        }
    }
    e.key("w", false, None);
    false
}

#[test]
fn reload_arch_plays_its_lightning_once() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let (mut e, mut v) = start(&root, "flash", false);
        let e = &mut e;
        // Idle: no thread, the lightning hidden (default visibility 0).
        assert!(arch(e).thread.is_none());
        let (shown, _, _) = electro(e);
        assert!(!shown, "electro shows while idle");
        place_before(e, &mut v, 10.0);
        assert!(!electro(e).0, "electro shows before the arch is entered");
        assert!(drive_through(e, &mut v), "the arch's thread never started (tank at {})", e.host.game.as_ref().unwrap().world.tank(player(e).unwrap().1).unwrap().pos);
        let th = arch(e).thread.clone().unwrap();
        assert_eq!((th.slot, th.seq.as_str(), th.scale), (1, "activate", 1.0));
        // (The tank is held still below, under the arch.)
        let (_, id) = player(e).unwrap();
        // Visibility over 1.5 s of ticks from the thread's start.
        let mut seen = Vec::new();
        let mut ifl_times = Vec::new();
        for _ in 0..1500 / tt_engine::TICK_MS {
            let now = e.host.game.as_ref().unwrap().world.time_ms;
            let (shown, hidden, ifl) = electro(e);
            assert!(shown != hidden, "electro is either shown or hidden during the thread");
            if shown {
                ifl_times.extend(ifl.iter().copied());
            }
            seen.push((now - th.start_ms, shown));
            step(e, &mut v);
            if let Some(t) = e.host.game.as_mut().unwrap().world.tank_mut(id) {
                t.vel = Vec3::ZERO;
            }
        }
        // Hidden → shown → hidden, once, ~0.5 s lit between ~0.13 and ~0.6 s.
        let lit: Vec<i64> = seen.iter().filter(|x| x.1).map(|x| x.0).collect();
        assert!(!lit.is_empty(), "electro never shown: {seen:?}");
        let (first, last) = (lit[0], *lit.last().unwrap());
        eprintln!("electro lit from {first} to {last} ms after the thread started");
        assert_eq!(lit.len() as i64, (last - first) / tt_engine::TICK_MS as i64 + 1, "one contiguous flash: {seen:?}");
        assert!((90..=200).contains(&first), "lights up at {first} ms: {seen:?}");
        assert!((520..=640).contains(&last), "goes out at {last} ms: {seen:?}");
        assert!(!seen.last().unwrap().1, "the one-shot thread leaves electro hidden");
        assert!(!seen[0].1, "hidden on the first keyframes");
        // The lightning's IFL runs with the thread.
        assert!(ifl_times.len() >= 2 && ifl_times.windows(2).any(|w| w[1] > w[0]), "electro IFL animated: {ifl_times:?}");
        // One thread per pass.
        assert_eq!(threads(e), 1, "one user thread for one pass");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

#[test]
#[ignore]
fn reload_arch_shots() {
    if std::env::var_os("TT_ARCH_SHOTS").is_none() {
        return;
    }
    let root = assets();
    big_stack(move || {
        let (mut e, mut v) = start(&root, "shots", true);
        let e = &mut e;
        let dir = v.as_ref().unwrap().dir.clone();
        std::fs::create_dir_all(&dir).unwrap();
        let p: Vec3 = arch(e).transform.translation.into();
        let cam = look(p + Vec3::new(9.0, -7.0, 3.5), p + Vec3::new(0.0, 0.0, 2.0));
        v.as_mut().unwrap().fe.camera_override = Some(cam);
        place_before(e, &mut v, 10.0);
        let shot = |v: &mut Option<View>, name: &str| {
            let v = v.as_mut().unwrap();
            let path = v.dir.join(format!("{name}.png"));
            v.off.save(&path).unwrap();
            eprintln!("saved {}", path.display());
        };
        shot(&mut v, "idle");
        assert!(drive_through(e, &mut v), "the thread never started");
        let start = arch(e).thread.as_ref().unwrap().start_ms;
        for _ in 0..1300 / tt_engine::TICK_MS {
            step(e, &mut v);
            let t = e.host.game.as_ref().unwrap().world.time_ms - start;
            let lit = electro(e).0;
            shot(&mut v, &format!("t{t:04}{}", if lit { "_lit" } else { "" }));
        }
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

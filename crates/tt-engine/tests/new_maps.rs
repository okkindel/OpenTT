//! OpenTT's new maps (tt1_5 "The Neverending Stream", tt2_5 "Night of the Living Brains",
//! tt3_5 "The Dark Cortex", the Moon's tt4_1 "Lunar Lobe" and tt4_2 "Cortex Craters";
//! crates/tt-assets/examples/new_maps/): each loads through the menu's custom game path in all
//! four modes with the player spawned on the plateau and every power-up and pad present;
//! scripted drives check the landmarks (the stream's fords and its leaps, the jump pads up
//! onto the island, the witch's gully and the stone circle, the island fords and the ramp
//! jumps over the lava gorge), that every pad kicks and that a tank drives straight through
//! every reload arch, which stands on a level lane. `NEW_MAPS_SHOTS=DIR` saves in-game
//! screenshots of the landmarks there.

use glam::Vec3;
use std::path::Path;
use tt_engine::{Config, Engine};
use tt_menu::{CustomGame, MenuAction, MenuBackend, Team};

mod common;
use common::*;

struct MapSpec {
    id: &'static str,
    name: &'static str,
    world: &'static str,
    triggers: &'static [&'static str],
    statics: &'static [&'static str],
}

const GREEN: MapSpec = MapSpec {
    id: "tt1_5",
    name: "The Neverending Stream",
    world: "Green",
    triggers: &["pup1", "pup2", "pup3", "pup4", "pup5", "pup6", "pup7", "pup8", "pup9", "ReloadWest", "ReloadEast", "HealWest", "HealEast", "LeapWest", "LeapEast", "CurrentWest", "CurrentEast", "JumpNorth", "JumpSouth"],
    statics: &[],
};
const BLUE: MapSpec = MapSpec {
    id: "tt2_5",
    name: "Night of the Living Brains",
    world: "Blue",
    triggers: &["pup1", "pup2", "pup3", "pup4", "pup5", "pup6", "pup7", "ReloadSouth", "ReloadNorth", "HealWest", "HealEast", "BoostSouth", "BoostNorth", "JumpWest", "JumpEast"],
    statics: &["CircleStone", "DoorSouth", "DoorNorth"],
};
const RED: MapSpec = MapSpec {
    id: "tt3_5",
    name: "The Dark Cortex",
    world: "Red",
    triggers: &["pup1", "pup2", "pup3", "pup4", "pup5", "pup6", "pup7", "pup8", "pup9", "Reload", "HealWest", "HealEast", "BoostSouth", "BoostNorth", "RiverJumpN", "RiverJumpS"],
    statics: &["DarkTower", "ValleyGate"],
};

const MOON1: MapSpec = MapSpec {
    id: "tt4_1",
    name: "Lunar Lobe",
    world: "Moon",
    triggers: &["pup1", "pup2", "pup3", "pup4", "pup5", "pup6", "pup7", "Reload", "HealNorth", "HealSouth", "BoostNorth", "BoostSouth", "JumpEast", "JumpWest"],
    statics: &[],
};
const MOON2: MapSpec = MapSpec {
    id: "tt4_2",
    name: "Cortex Craters",
    world: "Moon",
    triggers: &["pup1", "pup2", "pup3", "pup4", "pup5", "pup6", "pup7", "Reload", "HealWest", "HealEast", "BoostWest", "BoostEast", "JumpEast", "JumpNorth"],
    statics: &[],
};

fn mission(spec: &MapSpec) -> String {
    tt_assets::layout::mission(spec.id)
}

fn start(root: &Path, spec: &MapSpec, mode: u32) -> Engine {
    start_with(root, spec, mode, 1)
}

/// Starts the map in `mode` with the player in tank `tank` (0 Light, 1
/// Medium, 2 Heavy).
fn start_with(root: &Path, spec: &MapSpec, mode: u32, tank: u8) -> Engine {
    // A data directory of its own per engine (tests run in parallel).
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let data = std::env::temp_dir().join(format!("tt-engine-test-newmaps-{}-{mode}-{tank}-{}-{n}", spec.id, std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let cfg = Config { assets: root.to_path_buf(), data, args: Vec::new(), audio_output: false, echo_log: std::env::var_os("NEW_MAPS_LOG").is_some(), windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    assert!(e.boot());
    e.set_var("input.aim_mode", "0");
    for _ in 0..30 {
        e.tick();
    }
    let file = mission(spec);
    {
        let Engine { host, menu, .. } = &mut e;
        let m = menu.as_mut().unwrap();
        let mut b = tt_engine::menu::Backend::new(host, &mut m.data);
        assert!(b.missions().iter().any(|m| m.file == file && m.name == spec.name && m.world == spec.world), "{} not in the custom game list", spec.id);
        b.create_player("Tester").unwrap();
    }
    let g = CustomGame { mission: file.clone(), mode, bots: Vec::new(), player_tank: tank as usize, player_team: Team::Green, minutes: 10, replace_bots: true };
    apply(&mut e, MenuAction::StartCustom(g));
    until_spawned(&mut e, |e| e.tick());
    assert!(e.server().session.mission_file.clone().eq_ignore_ascii_case(&file), "{}", e.server().session.mission_file.clone());
    for _ in 0..3000 / tt_engine::TICK_MS {
        e.tick();
    }
    e
}

fn tank(e: &Engine) -> &tt_game::Tank {
    let (_, id) = player(e).expect("player tank");
    e.host.game.as_ref().unwrap().world.tank(id).unwrap()
}

fn ground(e: &Engine, x: f32, y: f32) -> f32 {
    e.host.game.as_ref().unwrap().world.terrain.as_ref().unwrap().height(x, y)
}

/// Puts the tank at (x, y) on the ground facing `heading` (compass
/// degrees), still, and lets it land.
fn place(e: &mut Engine, x: f32, y: f32, heading: f32) {
    for _ in 0..200 {
        e.tick();
        if tank(e).vel.length() < 0.2 {
            break;
        }
    }
    let (o, _) = player(e).unwrap();
    let z = ground(e, x, y) + 1.0;
    // setTransform's angle is in radians (clockwise seen from above: the compass heading).
    e.host.game.as_mut().unwrap().set_transform(o, &format!("{x} {y} {z} 0 0 1 {}", heading.to_radians()));
    // Still: setTransform keeps the velocity.
    let (_, id) = player(e).unwrap();
    if let Some(t) = e.host.game.as_mut().unwrap().world.tanks.iter_mut().find(|t| t.id == id) {
        t.vel = Vec3::ZERO;
    }
    for _ in 0..2500 / tt_engine::TICK_MS {
        e.tick();
    }
    let p = tank(e).pos;
    assert!((p.x - x).hypot(p.y - y) < 7.0, "placed at ({x}, {y}) but the tank is at {p}");
}

/// Holds W and steers (A/D) towards the line through `from` and `to`
/// (aiming 12 m ahead on it), for up to `ms` or until `done`.
fn drive_line(e: &mut Engine, from: (f32, f32), to: (f32, f32), ms: u32, done: impl Fn(Vec3) -> bool) -> Vec<Vec3> {
    let (a, b) = (glam::Vec2::new(from.0, from.1), glam::Vec2::new(to.0, to.1));
    let dir = (b - a).normalize();
    e.key("w", true, None);
    let mut steer = 0i32;
    let mut path = Vec::new();
    for _ in 0..ms / tt_engine::TICK_MS {
        e.tick();
        let Some(_) = player(e) else { break };
        let t = tank(e);
        let p = t.pos;
        path.push(p);
        if done(p) {
            break;
        }
        let along = (p.truncate() - a).dot(dir);
        let aim = a + dir * (along + 12.0) - p.truncate();
        let fwd = t.rot.y_axis.truncate().normalize_or_zero();
        let cross = fwd.perp_dot(aim.normalize_or_zero());
        let want = if cross > 0.03 { -1 } else if cross < -0.03 { 1 } else { 0 };
        if want != steer {
            match steer {
                -1 => e.key("a", false, None),
                1 => e.key("d", false, None),
                _ => {}
            }
            match want {
                -1 => e.key("a", true, None),
                1 => e.key("d", true, None),
                _ => {}
            }
            steer = want;
        }
    }
    e.key("w", false, None);
    e.key("a", false, None);
    e.key("d", false, None);
    path
}

fn event_count(e: &Engine, name: &str) -> u32 {
    e.host.game.as_ref().unwrap().event_counts.get(name).copied().unwrap_or(0) as u32
}

/// An in-game screenshot (the game view with its tanks and triggers) from
/// the given camera, when `NEW_MAPS_SHOTS` names a directory.
fn shot(e: &mut Engine, name: &str, pos: Vec3, look_at: Vec3) {
    let Some(dir) = std::env::var_os("NEW_MAPS_SHOTS") else { return };
    let root = e.host.fs.root().to_path_buf();
    let scene = tt_engine::mission::build_scene(&e.host, &root, true).expect("mission").expect("scene");
    let Some(mut gv) = e.game_view() else { return };
    let d = look_at - pos;
    gv.camera = tt_render::Camera { pos, yaw: d.x.atan2(d.y), pitch: d.z.atan2(d.truncate().length()) };
    let out = Path::new(&dir).join(format!("{name}.png"));
    tt_engine::gfx::screenshot(&out, (1280, 720), Some(&scene), Some(&gv), 0.0).expect("screenshot");
}

fn check_loads(spec: &MapSpec, mode: u32) {
    let e = start(&assets(), spec, mode);
    let scrum = mode >= 2;
    for n in spec.triggers {
        let _ = trigger(&e, n);
    }
    for n in spec.statics {
        assert!(e.host.mission_load.as_ref().is_some_and(|l| l.nodes.iter().any(|(_, o)| o.name.eq_ignore_ascii_case(n))), "{} static {n}", spec.id);
    }
    let g = e.host.game.as_ref().unwrap();
    let ma = g.world.mission_area.unwrap();
    assert!(ma.x == ma.y && ma.w == ma.h && ma.w == -2.0 * ma.x && (280.0..=512.0).contains(&ma.w), "{}: mission area {ma:?}", spec.id);
    assert_eq!(g.world.flags.len(), usize::from(scrum), "{} mode {mode}: scrum ball", spec.id);
    let reloads = g.world.triggers.iter().filter(|t| t.def_name().eq_ignore_ascii_case("Reload")).count();
    let weapons = g.world.triggers.iter().filter(|t| t.def_name().eq_ignore_ascii_case("TestPowerup")).count();
    // One arch on the Moon maps and tt3_5, two on the others.
    assert!(reloads == if spec.world == "Moon" || spec.id == "tt3_5" { 1 } else { 2 } && weapons >= 6, "{}: {reloads} reload arches, {weapons} power-ups", spec.id);
    if scrum {
        let g = e.host.game.as_ref().unwrap();
        assert!(g.world.triggers.iter().any(|t| t.def_name().eq_ignore_ascii_case("DefaultGoal")), "{} mode {mode}: scrum goal", spec.id);
    }
    let t = tank(&e);
    assert!(t.pos.x.abs() < ma.w / 2.0 && t.pos.y.abs() < ma.w / 2.0 && t.pos.z > 96.0, "{} mode {mode}: spawned at {}", spec.id, t.pos);
    // Spawned on the ground, not on a rock.
    assert!((t.pos.z - ground(&e, t.pos.x, t.pos.y)).abs() < 2.0, "{} mode {mode}: spawned at {}", spec.id, t.pos);
    let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
}

/// Each pad kicks a tank driving onto it (from the south).
fn check_pads(e: &mut Engine, pads: &[&str]) {
    for pad in pads {
        let p: Vec3 = trigger(e, pad).transform.translation.into();
        let before = event_count(e, "PadUsed");
        place(e, p.x, p.y - 7.0, 0.0);
        let path = drive_line(e, (p.x, p.y - 7.0), (p.x, p.y + 7.0), 3000, |_| false);
        dump(&path);
        let used = event_count(e, "PadUsed") > before;
        assert!(used, "{pad} at {p} did not kick the tank (path {:?} .. {:?})", path.first(), path.last());
    }
}

#[test]
fn neverending_stream_loads_in_all_modes() {
    tt_assets::require_game_data!();
    big_stack(|| (0..4).for_each(|m| check_loads(&GREEN, m)));
}

#[test]
fn night_of_the_living_brains_loads_in_all_modes() {
    tt_assets::require_game_data!();
    big_stack(|| (0..4).for_each(|m| check_loads(&BLUE, m)));
}

#[test]
fn dark_cortex_loads_in_all_modes() {
    tt_assets::require_game_data!();
    big_stack(|| (0..4).for_each(|m| check_loads(&RED, m)));
}

#[test]
fn lunar_lobe_loads_in_all_modes() {
    tt_assets::require_game_data!();
    big_stack(|| (0..4).for_each(|m| check_loads(&MOON1, m)));
}

#[test]
fn cortex_craters_loads_in_all_modes() {
    tt_assets::require_game_data!();
    big_stack(|| (0..4).for_each(|m| check_loads(&MOON2, m)));
}

/// The Moon maps' gravity reaches the world (MissionInfo.gravity); the
/// other maps keep the game's.
#[test]
fn moon_maps_have_low_gravity() {
    tt_assets::require_game_data!();
    big_stack(|| {
        for (spec, g) in [(&MOON1, 0.45), (&GREEN, 1.0)] {
            let e = start(&assets(), spec, 0);
            assert_eq!(e.host.game.as_ref().unwrap().world.gravity, g, "{}", spec.id);
        }
    });
}

fn dump(path: &[Vec3]) {
    if std::env::var_os("NEW_MAPS_PATH").is_some() {
        for p in path.iter().step_by(4) {
            eprintln!("  {:.2} {:.2} {:.2}", p.x, p.y, p.z);
        }
    }
}

/// The stream's line of tt1_5 (crates/tt-assets/examples/new_maps/river.rs `line_y`).
fn stream_y(x: f32) -> f32 {
    24.0 * (x / 36.0).sin() + 8.0 * (x / 15.0).sin()
}

const TANKS: [&str; 3] = ["Light", "Medium", "Heavy"];

/// Drives from a standstill at `from` along the line to `to` (through the
/// boost pad and over the lip) and checks the tank lands on the far bank:
/// past `beyond` metres along the jump axis and no lower than `bank - 3`.
fn check_jump(e: &mut Engine, label: &str, from: (f32, f32), to: (f32, f32), axis_x: bool, beyond: f32, gap: (f32, f32), bank: f32) {
    let heading = (to.0 - from.0).atan2(to.1 - from.1).to_degrees();
    place(e, from.0, from.1, heading);
    let sign = if axis_x { (to.0 - from.0).signum() } else { (to.1 - from.1).signum() };
    let along = move |p: Vec3| if axis_x { p.x } else { p.y };
    let path = drive_line(e, from, to, 12_000, |p| along(p) * sign > beyond);
    dump(&path);
    let end = *path.last().unwrap();
    let lowest = path.iter().filter(|p| (gap.0..=gap.1).contains(&along(**p))).map(|p| p.z).fold(f32::MAX, f32::min);
    eprintln!("{label}: end {end}, lowest over the gap {lowest} (bank {bank})");
    assert!(along(end) * sign > beyond && end.z > bank - 3.0 && lowest > bank - 4.0, "{label}: end {end}, lowest over the gap {lowest} (bank {bank})");
}

#[test]
fn neverending_stream_landmarks() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let mut e = start(&assets(), &GREEN, 0);
        // The stream bed under the banks.
        let (bed, bank) = (ground(&e, -100.0, stream_y(-100.0)), ground(&e, -100.0, stream_y(-100.0) + 40.0));
        assert!(bank - bed > 6.0, "the stream bed is {} m under the bank", bank - bed);
        // No wall on the map (inside the plateau edge) is steeper than about 50 degrees.
        let mut steepest = (0.0f32, 0.0, 0.0);
        for j in -18..=18 {
            for i in -18..=18 {
                let (x, y) = (i as f32 * 8.0, j as f32 * 8.0);
                let s = ((ground(&e, x + 8.0, y) - ground(&e, x, y)).abs()).max((ground(&e, x, y + 8.0) - ground(&e, x, y)).abs()) / 8.0;
                if s > steepest.0 {
                    steepest = (s, x, y);
                }
            }
        }
        assert!(steepest.0 < 1.25, "steepest grid slope {:.2} at ({}, {})", steepest.0, steepest.1, steepest.2);
        // Over each end ford, both ways, bank to bank.
        for x in [-142.0f32, 142.0] {
            let y0 = stream_y(x);
            for s in [1.0f32, -1.0] {
                place(&mut e, x, y0 - s * 60.0, if s > 0.0 { 0.0 } else { 180.0 });
                let path = drive_line(&mut e, (x, y0 - s * 60.0), (x, y0 + s * 70.0), 20_000, |p| (p.y - y0) * s > 50.0);
                dump(&path);
                let end = *path.last().unwrap();
                assert!((end.y - y0) * s > 50.0, "ford at x {x} ({}): stopped at {end}", if s > 0.0 { "north" } else { "south" });
            }
        }
        // Down the north bank's ford into the island's channel.
        let y0 = stream_y(-26.0);
        place(&mut e, -26.0, y0 + 90.0, 180.0);
        let path = drive_line(&mut e, (-26.0, y0 + 90.0), (-26.0, y0), 20_000, |p| p.y < y0 + 34.0);
        dump(&path);
        assert!(path.last().unwrap().y < y0 + 34.0, "island ford: stopped at {}", path.last().unwrap());
        let island = Vec3::new(-4.0, stream_y(-4.0) + 3.0, ground(&e, -4.0, stream_y(-4.0) + 3.0));
        shot(&mut e, "tt1_5_island", island + Vec3::new(-40.0, -55.0, 14.0), island + Vec3::new(0.0, 0.0, 6.0));
        check_pads(&mut e, &["JumpNorth", "JumpSouth", "CurrentWest", "CurrentEast"]);
    });
}

/// The leaps over the stream on tt1_5, for each tank type, driving from a
/// standstill 40 m before the pad: the tank lands on the far bank.
#[test]
fn neverending_stream_leaps_every_tank() {
    tt_assets::require_game_data!();
    big_stack(|| {
        for (t, name) in TANKS.iter().enumerate() {
            let mut e = start_with(&assets(), &GREEN, 0, t as u8);
            for (pad, s) in [("LeapWest", -1.0f32), ("LeapEast", 1.0)] {
                let p: Vec3 = trigger(&e, pad).transform.translation.into();
                let y0 = stream_y(p.x);
                let far = ground(&e, p.x, y0 + s * 38.0);
                check_jump(&mut e, &format!("{name} {pad}"), (p.x, p.y - s * 40.0), (p.x, y0 + s * 90.0), false, (y0 + s * 36.0) * s, (y0 - 12.0, y0 + 12.0), far);
            }
            let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
        }
    });
}

/// The witch's gullies' centre radius on tt2_5 (crates/tt-assets/examples/new_maps/halloween.rs `GULLY_R`; east and west of the hill).
const GULLY_R: f32 = 98.0;

#[test]
fn night_of_the_living_brains_landmarks() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let mut e = start(&assets(), &BLUE, 0);
        // The ritual hill over the sunken pumpkin patch; the gullies deep.
        let (top, basin) = (ground(&e, 0.0, -18.0), ground(&e, -40.0, -50.0));
        assert!(top - basin > 10.0, "the hill is {} m over the patch", top - basin);
        for side in [-1.0f32, 1.0] {
            let floor = ground(&e, side * GULLY_R, 0.0);
            let bank = ground(&e, side * (GULLY_R + 28.0), 0.0).min(ground(&e, side * (GULLY_R - 28.0), 0.0));
            assert!(bank - floor > 6.0, "the gully at x {} is {} m deep", side * GULLY_R, bank - floor);
        }
        let (floor, bank) = (ground(&e, -GULLY_R, 0.0), ground(&e, -GULLY_R + 26.0, -30.0));
        shot(&mut e, "tt2_5_gully", Vec3::new(-GULLY_R + 26.0, -30.0, bank + 6.0), Vec3::new(-GULLY_R, 10.0, floor + 2.0));
        // Up the hill and through the south doorway into the circle.
        let gap = (360.0f32 / 14.0 * 6.5 + 7.0).to_radians();
        let (gx, gy) = (gap.sin(), gap.cos());
        place(&mut e, gx * 70.0, gy * 70.0, (-gx).atan2(-gy).to_degrees());
        let path = drive_line(&mut e, (gx * 70.0, gy * 70.0), (0.0, 0.0), 20_000, |p| p.truncate().length() < 14.0);
        dump(&path);
        assert!(path.last().unwrap().truncate().length() < 14.0, "stone circle: stopped at {}", path.last().unwrap());
        shot(&mut e, "tt2_5_stone_circle", Vec3::new(gx * 54.0, gy * 54.0, top + 8.0), Vec3::new(0.0, 0.0, top + 3.0));
        check_pads(&mut e, &["BoostSouth", "BoostNorth", "JumpWest", "JumpEast"]);
    });
}

/// The jumps over tt2_5's gullies, for each tank type, driving from a
/// standstill 30 m before the pad: the tank carries over the gully's
/// floor (no lower than 4 m under its far rim) and drives on out of it.
#[test]
fn night_of_the_living_brains_jumps_every_tank() {
    tt_assets::require_game_data!();
    big_stack(|| {
        for (t, name) in TANKS.iter().enumerate() {
            let mut e = start_with(&assets(), &BLUE, 0, t as u8);
            for (pad, s) in [("JumpWest", -1.0f32), ("JumpEast", 1.0)] {
                let p: Vec3 = trigger(&e, pad).transform.translation.into();
                let far = ground(&e, s * (GULLY_R + 24.0), p.y);
                let gap = if s > 0.0 { (GULLY_R - 6.0, GULLY_R + 6.0) } else { (-GULLY_R - 6.0, -GULLY_R + 6.0) };
                check_jump(&mut e, &format!("{name} {pad}"), (p.x - s * 30.0, p.y), (s * (GULLY_R + 60.0), p.y), true, GULLY_R + 24.0, gap, far);
            }
            let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
        }
    });
}

/// The gorge of tt3_5's south arm (crates/tt-assets/examples/new_maps/dark_cortex.rs `GORGE`) and its jump lanes (x from it).
const R_GORGE: (f32, f32) = (-48.0, -96.0);
const R_JUMPS: [f32; 2] = [-10.0, 10.0];

#[test]
fn dark_cortex_landmarks() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let mut e = start(&assets(), &RED, 0);
        let river = ground(&e, R_GORGE.0, R_GORGE.1);
        let bank = ground(&e, R_GORGE.0 + R_JUMPS[0], R_GORGE.1 + 20.0);
        assert!(bank - river > 6.0, "the gorge is {} m deep", bank - river);
        // The lava sea is level, over 20 m under the fortress's top by the tower.
        let (sea, top) = (ground(&e, 140.0, 0.0), ground(&e, 60.0, 0.0));
        assert!((ground(&e, 130.0, 20.0) - sea).abs() < 0.2 && top - sea > 20.0, "sea {sea}, top {top}");
        shot(&mut e, "tt3_5_gorge", Vec3::new(R_GORGE.0 - 40.0, R_GORGE.1 - 50.0, bank + 12.0), Vec3::new(R_GORGE.0, R_GORGE.1, river + 4.0));
        // Over the north arm by the fords either side of its island.
        place(&mut e, 52.0, 34.0, 0.0);
        let path = drive_line(&mut e, (52.0, 34.0), (54.0, 120.0), 25_000, |p| p.y > 108.0);
        dump(&path);
        assert!(path.last().unwrap().y > 108.0, "island fords: stopped at {}", path.last().unwrap());
        shot(&mut e, "tt3_5_fords", Vec3::new(20.0, 120.0, 124.0), Vec3::new(52.0, 76.0, 104.0));
        // A tank at the tower's foot, for scale.
        place(&mut e, 56.0, -4.0, 70.0);
        let z = ground(&e, 36.0, 6.0) + 9.0;
        shot(&mut e, "tt3_5_tower", Vec3::new(36.0, 6.0, z), Vec3::new(66.0, -6.0, z + 6.0));
        check_pads(&mut e, &["RiverJumpN", "RiverJumpS"]);
    });
}

/// The gorge jumps on tt3_5, both ways, for each tank type, driving from a
/// standstill 14 m before the boost pad.
#[test]
fn dark_cortex_gorge_jump_every_tank() {
    tt_assets::require_game_data!();
    big_stack(|| {
        for (t, name) in TANKS.iter().enumerate() {
            let mut e = start_with(&assets(), &RED, 0, t as u8);
            let (gx, gy) = R_GORGE;
            let bank = ground(&e, gx + R_JUMPS[0], gy + 20.0);
            check_jump(&mut e, &format!("{name} north"), (gx + R_JUMPS[0], gy - 56.0), (gx + R_JUMPS[0], gy + 60.0), false, gy + 20.0, (gy - 4.0, gy + 4.0), bank);
            check_jump(&mut e, &format!("{name} south"), (gx + R_JUMPS[1], gy + 56.0), (gx + R_JUMPS[1], gy - 60.0), false, -(gy - 20.0), (gy - 4.0, gy + 4.0), bank);
            let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
        }
    });
}

/// Every prop of the new maps is planted: no foot of its geometry (as
/// placed: turned, scaled; `Shape::base_points`: a skeleton's rib ends and
/// skull, a tree's roots, a rock's underside) is above the terrain (rocks
/// and trees at least 0.2 m under,
/// the stone circle's standing stones at least 0.5 m), and the point nearest
/// the surface is no more than 1.5 m under it. The def shapes (power-
/// ups, arches, pads, the volcano) too: their round base (the circle in
/// their bounds; an arch's whole bottom face) sits on the ground with no
/// gap over 0.1 m, and the volcano's base is at least a metre under the
/// lowest ground under it (no more than 3 m).
#[test]
fn new_map_props_touch_the_ground() {
    tt_assets::require_game_data!();
    use glam::Vec3;
    let root = assets();
    let mut bad = Vec::new();
    for spec in [&GREEN, &BLUE, &RED, &MOON1, &MOON2] {
        let dir = root.join("missions");
        let m = tt_assets::data::mission::load(&dir.join(format!("{}.ron", spec.id))).unwrap();
        let t = tt_assets::terrain::Terrain::parse(&std::fs::read(dir.join(format!("{}.ter", spec.id))).unwrap()).unwrap();
        let terrain = tt_game::terrain::HeightField::from_terrain(&t, 8.0);
        let objects = m.scene_objects(|c| !c.unless.1.is_empty());
        let (mut count, mut shaped, mut halloween) = (0, 0, 0);
        for o in objects.iter().flat_map(|o| o.walk()) {
            let class = o.class.to_ascii_lowercase();
            let db = o.get("def").unwrap_or_default().to_ascii_lowercase();
            // The def shapes (data/pickups.ron, data/map_objects.ron).
            let shape_name = match (class.as_str(), db.as_str()) {
                ("prop", _) => o.get("shape").unwrap().to_string(),
                ("powerup", "heal") => "models/items/health.dts".into(),
                ("powerup", "reload") => "models/items/recharge.dts".into(),
                ("powerup", _) => "models/items/powerup.dts".into(),
                ("boost", "testjump") => "models/items/jumppad.dts".into(),
                ("boost", _) => "models/items/boostpad.dts".into(),
                ("volcano", _) => "models/volcano/volcano.dts".into(),
                (c, _) => {
                    assert!(!["scrumgoal", "flag", "staticshape", "item"].contains(&c), "{}: a {c} the test does not know the shape of", spec.id);
                    continue;
                }
            };
            let path = tt_assets::find_ci(&tt_assets::resolve(&root, &dir, &shape_name)).unwrap();
            let shape = tt_assets::dts::Shape::parse(&std::fs::read(&path).unwrap()).unwrap();
            let xf = tt_game::mission::object_transform(o);
            let (lo, hi) = (Vec3::from(shape.bounds.min), Vec3::from(shape.bounds.max));
            let round = class != "prop" && db != "reload";
            let foot: Vec<Vec3> = if class == "prop" {
                // Every foot of its geometry (a skeleton's rib ends and
                // skull, a tree's roots, a rock's underside).
                let down = xf.matrix3.transpose() * Vec3::NEG_Z;
                shape.base_points(down.into()).into_iter().map(|c| xf.transform_point3(Vec3::from(c))).collect()
            } else if round {
                // The circle in the bounds, filled.
                let r = (hi.x - lo.x).min(hi.y - lo.y) / 2.0;
                let c = (lo + hi) / 2.0;
                let mut pts = vec![xf.transform_point3(Vec3::new(c.x, c.y, lo.z))];
                for ring in 1..=10 {
                    for k in 0..48 {
                        let a = k as f32 * std::f32::consts::TAU / 48.0;
                        let d = r * ring as f32 / 10.0;
                        pts.push(xf.transform_point3(Vec3::new(c.x + d * a.cos(), c.y + d * a.sin(), lo.z)));
                    }
                }
                pts
            } else {
                let corners: Vec<Vec3> = (0..8).map(|k| Vec3::new(if k & 1 == 0 { lo.x } else { hi.x }, if k & 2 == 0 { lo.y } else { hi.y }, if k & 4 == 0 { lo.z } else { hi.z })).map(|c| xf.transform_point3(c)).collect();
                let bottom = corners.iter().map(|c| c.z).fold(f32::MAX, f32::min);
                let face: Vec<Vec3> = corners.into_iter().filter(|c| c.z < bottom + 0.3).collect();
                let mut pts = face.clone();
                if face.len() == 4 {
                    for a in 0..=10 {
                        for b in 0..=10 {
                            let (u, v) = (a as f32 / 10.0, b as f32 / 10.0);
                            pts.push(face[0].lerp(face[1], u).lerp(face[2].lerp(face[3], u), v));
                        }
                    }
                }
                pts
            };
            let nearest = foot.iter().map(|c| c.z - terrain.height(c.x, c.y)).fold(f32::MIN, f32::max);
            let p = xf.translation;
            let standing_stone = spec.id == "tt2_5" && shape_name.contains("/blue/rock") && (28.0..42.0).contains(&p.truncate().length()) && xf.matrix3.z_axis.z > 0.9;
            // The trilithons' lintels lie on their uprights.
            if o.name == "Lintel" {
                continue;
            }
            let (least, most) = match class.as_str() {
                "volcano" => (-3.0, -1.0),
                "prop" if standing_stone => (-1.5, -0.5),
                // The Moon bases' landing pad is sunk 2 m (its top 0.4 m up) and
                // the lander stands on it.
                "prop" if shape_name.ends_with("landingpad.dts") => (-2.1, -1.6),
                "prop" if shape_name.ends_with("lander.dts") => (0.3, 0.5),
                // The Moon's crash sites: the wreck's parts half buried, tilted.
                // tt3_5's starfighter: come down nose first into the lava sea,
                // its tail and a wing high out of it.
                "prop" if shape_name.ends_with("wreck_fighter.dts") => (-8.0, 12.0),
                "prop" if shape_name.contains("/wreck_") => (-8.0, -0.1),
                // The Halloween map's pumpkins sit on the ground,
                // a few centimetres in.
                "prop" if shape_name.contains("/halloween/") => (-1.0, -0.02),
                // tt1_5's rowboats and logs lie on the
                // ground, sunk a little.
                "prop" if ["rowboat.dts", "driftlog.dts"].iter().any(|s| shape_name.ends_with(s)) => (-1.0, -0.05),
                "prop" => (-1.5, -0.2),
                _ => (-1.5, 0.1),
            };
            if std::env::var_os("NEW_MAPS_PROPS").is_some() && class != "prop" {
                println!("{} {class} {db} at ({:.1}, {:.1}, {:.1}): {nearest:.2}", spec.id, p.x, p.y, p.z);
            }
            if !(least..=most).contains(&nearest) {
                bad.push(format!("{}: {class} {} at ({:.1}, {:.1}, {:.1}): the highest point of its base is {nearest:.2} m from the ground (allowed {least}..{most})", spec.id, shape_name, p.x, p.y, p.z));
            }
            if class == "prop" {
                count += 1;
                halloween += usize::from(shape_name.contains("/halloween/"));
            } else {
                shaped += 1;
            }
        }
        // The Moon maps: a base and a few dishes and panels (about 20 props).
        assert!(count > 50 || spec.world == "Moon", "{}: {count} props", spec.id);
        assert!(shaped >= spec.triggers.len(), "{}: {shaped} def shapes", spec.id);
        // The Halloween map's own props (jack-o'-lanterns, carved pumpkins)
        // are among those checked.
        assert!(spec.id != "tt2_5" || halloween > 60, "{}: {halloween} Halloween props", spec.id);
    }
    assert!(bad.is_empty(), "{} objects off the ground:\n{}", bad.len(), bad.join("\n"));
}

/// No pickup of the new maps is crowded by scenery: no foot of a prop
/// (`Shape::base_points`, as placed) within 4.5 m of a power-up's or a
/// heal's centre, nor within 8 m of a pad's or a reload arch's (they are
/// bigger and driven onto at speed).
#[test]
fn new_map_pickups_clear_of_props() {
    tt_assets::require_game_data!();
    use glam::Vec3;
    let root = assets();
    let mut bad = Vec::new();
    for spec in [&GREEN, &BLUE, &RED, &MOON1, &MOON2] {
        let dir = root.join("missions");
        let m = tt_assets::data::mission::load(&dir.join(format!("{}.ron", spec.id))).unwrap();
        let objects = m.scene_objects(|c| !c.unless.1.is_empty());
        let (mut pickups, mut feet) = (Vec::new(), Vec::new());
        for o in objects.iter().flat_map(|o| o.walk()) {
            let class = o.class.to_ascii_lowercase();
            let db = o.get("def").unwrap_or_default().to_ascii_lowercase();
            let xf = tt_game::mission::object_transform(o);
            match class.as_str() {
                "powerup" => pickups.push((o.name.clone(), xf.translation.truncate(), if db == "reload" { 8.0 } else { 4.5 })),
                "boost" => pickups.push((o.name.clone(), xf.translation.truncate(), 8.0)),
                "prop" => {
                    let shape_name = o.get("shape").unwrap();
                    let path = tt_assets::find_ci(&tt_assets::resolve(&root, &dir, shape_name)).unwrap();
                    let shape = tt_assets::dts::Shape::parse(&std::fs::read(&path).unwrap()).unwrap();
                    let down = xf.matrix3.transpose() * Vec3::NEG_Z;
                    feet.extend(shape.base_points(down.into()).into_iter().map(|c| (shape_name.to_string(), xf.transform_point3(Vec3::from(c)).truncate())));
                }
                _ => {}
            }
        }
        for (name, p, r) in &pickups {
            if let Some((shape, q)) = feet.iter().find(|(_, q)| q.distance(*p) < *r) {
                bad.push(format!("{}: {name} at ({:.1}, {:.1}): {shape} reaches ({:.1}, {:.1}), {:.1} m from it (allowed {r})", spec.id, p.x, p.y, q.x, q.y, q.distance(*p)));
            }
        }
    }
    assert!(bad.is_empty(), "{} pickups crowded:\n{}", bad.len(), bad.join("\n"));
}

/// Every pad of the new maps lies flush on the ground: the bottom face of
/// its bounds (as placed; corners and an 11 x 11 grid over it) is nowhere
/// more than 0.1 m above the terrain nor more than 0.3 m under it, and its
/// up axis is within 8 degrees of the ground's normal over its footprint
/// (the plane fitted to the terrain under it), so no pad sticks out of a
/// slope or straddles a crease.
#[test]
fn new_map_pads_lie_flush() {
    tt_assets::require_game_data!();
    use glam::Vec3;
    let root = assets();
    let mut bad = Vec::new();
    for spec in [&GREEN, &BLUE, &RED, &MOON1, &MOON2] {
        let dir = root.join("missions");
        let m = tt_assets::data::mission::load(&dir.join(format!("{}.ron", spec.id))).unwrap();
        let t = tt_assets::terrain::Terrain::parse(&std::fs::read(dir.join(format!("{}.ter", spec.id))).unwrap()).unwrap();
        let terrain = tt_game::terrain::HeightField::from_terrain(&t, 8.0);
        let objects = m.scene_objects(|c| !c.unless.1.is_empty());
        let mut pads = 0;
        for o in objects.iter().flat_map(|o| o.walk()) {
            if !o.class.eq_ignore_ascii_case("boost") {
                continue;
            }
            pads += 1;
            let db = o.get("def").unwrap_or_default().to_ascii_lowercase();
            let shape_name = if db == "testjump" { "models/items/jumppad.dts" } else { "models/items/boostpad.dts" };
            let path = tt_assets::find_ci(&tt_assets::resolve(&root, &dir, shape_name)).unwrap();
            let shape = tt_assets::dts::Shape::parse(&std::fs::read(&path).unwrap()).unwrap();
            let xf = tt_game::mission::object_transform(o);
            let (lo, hi) = (Vec3::from(shape.bounds.min), Vec3::from(shape.bounds.max));
            let mut face = Vec::new();
            for a in 0..=10 {
                for b in 0..=10 {
                    let (u, v) = (a as f32 / 10.0, b as f32 / 10.0);
                    face.push(xf.transform_point3(Vec3::new(lo.x + (hi.x - lo.x) * u, lo.y + (hi.y - lo.y) * v, lo.z)));
                }
            }
            let gaps: Vec<f32> = face.iter().map(|c| c.z - terrain.height(c.x, c.y)).collect();
            let (above, below) = (gaps.iter().copied().fold(f32::MIN, f32::max), gaps.iter().copied().fold(f32::MAX, f32::min));
            // The ground plane over the footprint (least squares z = a x + b y + c).
            let (mut sxx, mut sxy, mut syy, mut sxz, mut syz, mut sx, mut sy, mut sz, n) = (0.0f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, face.len() as f64);
            let p = xf.translation;
            for c in &face {
                let (x, y, z) = ((c.x - p.x) as f64, (c.y - p.y) as f64, terrain.height(c.x, c.y) as f64);
                sxx += x * x;
                sxy += x * y;
                syy += y * y;
                sxz += x * z;
                syz += y * z;
                sx += x;
                sy += y;
                sz += z;
            }
            let (mx, my, mz) = (sx / n, sy / n, sz / n);
            let (cxx, cxy, cyy, cxz, cyz) = (sxx / n - mx * mx, sxy / n - mx * my, syy / n - my * my, sxz / n - mx * mz, syz / n - my * mz);
            let det = cxx * cyy - cxy * cxy;
            let (a, b) = ((cxz * cyy - cyz * cxy) / det, (cyz * cxx - cxz * cxy) / det);
            let normal = Vec3::new(-a as f32, -b as f32, 1.0).normalize();
            let up = Vec3::from(xf.matrix3.z_axis).normalize();
            let tilt = up.angle_between(normal).to_degrees();
            let slope = normal.angle_between(Vec3::Z).to_degrees();
            let name = o.name.clone();
            if std::env::var_os("NEW_MAPS_PROPS").is_some() {
                println!("{} {name} ({db}) at ({:.1}, {:.1}, {:.2}): base {below:.2}..{above:.2} m, ground {slope:.1} deg, tilt from it {tilt:.1} deg", spec.id, p.x, p.y, p.z);
            }
            if above > 0.1 || below < -0.3 || tilt > 8.0 {
                bad.push(format!("{}: pad {name} at ({:.1}, {:.1}, {:.2}): base {below:.2}..{above:.2} m from the ground (allowed -0.3..0.1), ground {slope:.1} deg, tilt {tilt:.1} deg from it (allowed 8)", spec.id, p.x, p.y, p.z));
            }
        }
        assert!(pads >= 4, "{}: {pads} pads", spec.id);
    }
    assert!(bad.is_empty(), "{} pads not flush:\n{}", bad.len(), bad.join("\n"));
}

/// Level run-up and run-out of a drive-through arch (the generator's
/// `ARCH_RUN`).
const ARCH_RUN: f32 = 10.0;

/// The reload arches of a map: (name, transform, shape bounds min, max).
fn arches(spec: &MapSpec) -> Vec<(String, glam::Affine3A, Vec3, Vec3)> {
    let root = assets();
    let dir = root.join("missions");
    let m = tt_assets::data::mission::load(&dir.join(format!("{}.ron", spec.id))).unwrap();
    let path = tt_assets::find_ci(&tt_assets::resolve(&root, &dir, "models/items/recharge.dts")).unwrap();
    let shape = tt_assets::dts::Shape::parse(&std::fs::read(&path).unwrap()).unwrap();
    let (lo, hi) = (Vec3::from(shape.bounds.min), Vec3::from(shape.bounds.max));
    let objects = m.scene_objects(|c| !c.unless.1.is_empty());
    let out: Vec<_> = objects
        .iter()
        .flat_map(|o| o.walk())
        .filter(|o| o.class.eq_ignore_ascii_case("powerup") && o.get("def").is_some_and(|d| d.eq_ignore_ascii_case("reload")))
        .map(|o| (o.name.clone(), tt_game::mission::object_transform(o), lo, hi))
        .collect();
    assert_eq!(out.len(), if spec.world == "Moon" || spec.id == "tt3_5" { 1 } else { 2 }, "{}: reload arches", spec.id);
    out
}

/// Every drive-through (reload) arch of the new maps stands upright on a
/// level lane: both legs' base within 0.1 m over and 0.3 m under the
/// ground, and the ground under the arch and its run-up and run-out
/// (`ARCH_RUN` metres either side, the arch's width) flat: the plane
/// fitted to it under 3 degrees, no point more than 0.15 m off it.
#[test]
fn new_map_arches_stand_level() {
    tt_assets::require_game_data!();
    let root = assets();
    let mut bad = Vec::new();
    for spec in [&GREEN, &BLUE, &RED, &MOON1, &MOON2] {
        let t = tt_assets::terrain::Terrain::parse(&std::fs::read(root.join("missions").join(format!("{}.ter", spec.id))).unwrap()).unwrap();
        let terrain = tt_game::terrain::HeightField::from_terrain(&t, 8.0);
        for (name, xf, lo, hi) in arches(spec) {
            let p = Vec3::from(xf.translation);
            let up = Vec3::from(xf.matrix3.z_axis).normalize();
            let grid = |y0: f32, y1: f32, z: f32| -> Vec<Vec3> {
                (0..=10)
                    .flat_map(|a| (0..=10).map(move |b| (a, b)))
                    .map(|(a, b)| xf.transform_point3(Vec3::new(lo.x + (hi.x - lo.x) * a as f32 / 10.0, y0 + (y1 - y0) * b as f32 / 10.0, z)))
                    .collect()
            };
            let legs: Vec<f32> = grid(lo.y, hi.y, 0.0).iter().map(|c| c.z - terrain.height(c.x, c.y)).collect();
            let (above, below) = (legs.iter().copied().fold(f32::MIN, f32::max), legs.iter().copied().fold(f32::MAX, f32::min));
            let lane: Vec<Vec3> = grid(lo.y - ARCH_RUN, hi.y + ARCH_RUN, 0.0).into_iter().map(|c| Vec3::new(c.x - p.x, c.y - p.y, terrain.height(c.x, c.y))).collect();
            // Least squares z = a x + b y + c.
            let n = lane.len() as f32;
            let m = lane.iter().fold(Vec3::ZERO, |s, c| s + *c) / n;
            let (mut cxx, mut cxy, mut cyy, mut cxz, mut cyz) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for c in &lane {
                let d = *c - m;
                cxx += d.x * d.x;
                cxy += d.x * d.y;
                cyy += d.y * d.y;
                cxz += d.x * d.z;
                cyz += d.y * d.z;
            }
            let det = cxx * cyy - cxy * cxy;
            let (a, b) = ((cxz * cyy - cyz * cxy) / det, (cyz * cxx - cxz * cxy) / det);
            let slope = f32::hypot(a, b).atan().to_degrees();
            let off = lane.iter().map(|c| (c.z - (m.z + a * (c.x - m.x) + b * (c.y - m.y))).abs()).fold(0.0, f32::max);
            if std::env::var_os("NEW_MAPS_PROPS").is_some() {
                println!("{} {name} at ({:.1}, {:.1}, {:.2}): legs {below:.2}..{above:.2} m, lane {slope:.2} deg, {off:.2} m off", spec.id, p.x, p.y, p.z);
            }
            if above > 0.1 || below < -0.3 || up.z < 0.999 || slope > 3.0 || off > 0.15 {
                bad.push(format!("{}: arch {name} at ({:.1}, {:.1}, {:.2}): legs {below:.2}..{above:.2} m from the ground (allowed -0.3..0.1), up {up}, lane {slope:.2} deg (allowed 3), {off:.2} m off its plane (allowed 0.15)", spec.id, p.x, p.y, p.z));
            }
        }
    }
    assert!(bad.is_empty(), "{} arches not level:\n{}", bad.len(), bad.join("\n"));
}

/// A tank drives straight through every arch of the new maps, both ways,
/// from a standstill 30 m before it (lined up) to 15 m past it, between
/// its legs.
#[test]
fn new_map_arches_drive_through() {
    tt_assets::require_game_data!();
    big_stack(|| {
        for spec in [&GREEN, &BLUE, &RED, &MOON1, &MOON2] {
            let mut e = start(&assets(), spec, 0);
            for (name, xf, _, _) in arches(spec) {
                let p = Vec3::from(xf.translation).truncate();
                let dir = Vec3::from(xf.matrix3.y_axis).truncate().normalize();
                for s in [1.0f32, -1.0] {
                    let (a, b) = (p - dir * s * 30.0, p + dir * s * 30.0);
                    let heading = (dir.x * s).atan2(dir.y * s).to_degrees();
                    place(&mut e, a.x, a.y, heading);
                    let path = drive_line(&mut e, (a.x, a.y), (b.x, b.y), 12_000, |q| (q.truncate() - p).dot(dir * s) > 15.0);
                    dump(&path);
                    let end = *path.last().unwrap();
                    // Where it passes the arch: between the legs.
                    let mid = path.iter().min_by(|u, v| (u.truncate() - p).dot(dir).abs().total_cmp(&(v.truncate() - p).dot(dir).abs())).unwrap();
                    let side = (mid.truncate() - p).perp_dot(dir).abs();
                    assert!((end.truncate() - p).dot(dir * s) > 15.0 && side < 3.0, "{} {name} ({}): stopped at {end}, passed {side:.1} m off the centre", spec.id, if s > 0.0 { "forward" } else { "back" });
                }
            }
            let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
        }
    });
}

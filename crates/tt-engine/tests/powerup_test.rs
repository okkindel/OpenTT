//! The "Power-up Test" map (assets/missions/poweruptest.ron, found through
//! the assets overlay): loads in every game mode through the menu's custom
//! game path, the player spawns, and driving the tank through each power-up
//! picks it up (projectile / health / reload), then it respawns after 3 s;
//! the pads kick the tank..

use glam::Vec3;
use std::path::Path;
use tt_engine::{Config, Engine};
use tt_menu::{CustomGame, MenuAction, MenuBackend, Team};

mod common;
use common::*;

const MISSION: &str = "missions/poweruptest.ron";

fn start(root: &Path, mode: u32) -> Engine {
    start_on(root, MISSION, mode)
}

/// Starts `mission` (the Power-up Test map: also checks it is in the
/// custom game list) in game mode `mode`, the player spawned and landed.
fn start_on(root: &Path, mission: &str, mode: u32) -> Engine {
    let tag = mission.rsplit('/').next().unwrap_or(mission);
    let data = std::env::temp_dir().join(format!("tt-engine-test-puptest-{tag}-{mode}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let cfg = Config { assets: root.to_path_buf(), data, args: Vec::new(), audio_output: false, echo_log: std::env::var_os("PUP_LOG").is_some(), windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    assert!(e.boot());
    // Classic aim: the test steers with the keys (camera aim is the default).
    e.set_var("input.aim_mode", "0");
    for _ in 0..30 {
        e.tick();
    }
    {
        let Engine { host, menu, .. } = &mut e;
        let m = menu.as_mut().unwrap();
        let b = tt_engine::menu::Backend::new(host, &mut m.data);
        assert!(b.missions().iter().any(|m| m.file == mission && (mission != MISSION || m.name == "Power-up Test")), "{mission} not in the custom game list");
    }
    {
        let Engine { host, menu, .. } = &mut e;
        let m = menu.as_mut().unwrap();
        tt_engine::menu::Backend::new(host, &mut m.data).create_player("Tester").unwrap();
    }
    let g = CustomGame { mission: mission.into(), mode, bots: Vec::new(), player_tank: 1, player_team: Team::Green, minutes: 10, replace_bots: true };
    {
        let Engine { host, menu, .. } = &mut e;
        menu.as_mut().unwrap().apply(MenuAction::StartCustom(g), host);
    }
    // Until the player's tank is in the world (SPACE respawns, like a player).
    for i in 0..3000 {
        e.tick();
        if i % 60 == 0 {
            e.key("space", true, None);
            e.key("space", false, None);
        }
        if player(&e).is_some() {
            break;
        }
    }
    assert!(e.server().session.mission_file.clone().eq_ignore_ascii_case(mission), "{}", e.server().session.mission_file.clone());
    assert!(player(&e).is_some(), "mode {mode}: the player did not spawn");
    // Spawns drop from 15 m: land first.
    for _ in 0..3000 / tt_engine::TICK_MS {
        e.tick();
    }
    e
}

fn tank(e: &Engine) -> &tt_game::Tank {
    let Some((_, id)) = player(e) else {
        let g = e.host.game.as_ref();
        let o = e.control_object();
        let t = o.and_then(|o| g?.tank_of(o)).and_then(|id| g?.world.tank(id));
        panic!("no player tank: control {o:?}, tank {:?}, events {:?}", t.map(|t| (t.pos, t.damage, t.is_corpse, t.deleted)), g.map(|g| &g.event_counts));
    };
    e.host.game.as_ref().unwrap().world.tank(id).unwrap()
}

fn projectile(e: &Engine) -> String {
    tank(e).current.as_ref().map(|p| p.name.clone()).unwrap_or_default()
}

/// Puts the tank `back` metres south of `name`, facing north, still.
fn place_before(e: &mut Engine, name: &str, back: f32) {
    // Come to a stop first (setTransform keeps the velocity).
    for _ in 0..200 {
        e.tick();
        if tank(e).vel.length() < 0.2 {
            break;
        }
    }
    let p: Vec3 = trigger(e, name).transform.translation.into();
    let (o, _) = player(e).unwrap();
    let xf = format!("{} {} {} 0 0 1 0", p.x, p.y - back, p.z + 1.0);
    e.host.game.as_mut().unwrap().set_transform(o, &xf);
    // SetTransform drops the tank from 15 m: land.
    for _ in 0..2500 / tt_engine::TICK_MS {
        e.tick();
    }
}

/// Holds W until `name` switches off (picked up) or `ms` pass; returns
/// whether it did.
fn drive_into(e: &mut Engine, name: &str, ms: u32) -> bool {
    e.key("w", true, None);
    let mut hit = false;
    for _ in 0..ms / tt_engine::TICK_MS {
        e.tick();
        if !trigger(e, name).on {
            hit = true;
            break;
        }
    }
    e.key("w", false, None);
    for _ in 0..10 {
        e.tick();
    }
    hit
}

fn pick_up(e: &mut Engine, name: &str) {
    assert!(trigger(e, name).on, "{name} starts on");
    place_before(e, name, 10.0);
    let t = trigger(e, name);
    let (p, tk) = (Vec3::from(t.transform.translation), tank(e).pos);
    assert!(drive_into(e, name, 4000), "{name}: not picked up driving from {tk} towards {p} (tank now at {})", tank(e).pos);
}

fn check_mode(root: &Path, mode: u32) {
    let mut e = start(root, mode);
    let scrum = mode >= 2;
    let names = ["PupSpeedy", "PupBounce", "PupSplash", "PupWeapon", "PupHeal", "PupReload", "PupMortar", "PupScatter", "BoostWest", "BoostEast", "JumpWest", "JumpEast", "Goal"];
    for n in names {
        let _ = trigger(&e, n);
        assert!(e.host.mission_load.as_ref().is_some_and(|l| l.nodes.iter().any(|(_, o)| o.name.eq_ignore_ascii_case(n))), "object {n}");
    }
    {
        let g = e.host.game.as_ref().unwrap();
        assert_eq!(g.world.flags.len(), 1, "mode {mode}: one ball");
        assert_eq!(g.world.flags[0].data.practice, !scrum, "mode {mode}: the practice ball outside Scrum");
        let ma = g.world.mission_area.unwrap();
        assert_eq!((ma.x, ma.y, ma.w, ma.h), (-128.0, -128.0, 256.0, 256.0));
        // Spawned on the flat floor inside the rim.
        let t = tank(&e);
        assert!(t.pos.x.abs() < 100.0 && t.pos.y.abs() < 100.0 && (t.pos.z - 100.0).abs() < 3.0, "spawn at {}", t.pos);
    }
    let default = projectile(&e);
    assert!(!default.is_empty());
    for (name, want) in [("PupSpeedy", "SpeedyProjectile"), ("PupBounce", "BounceProjectile"), ("PupSplash", "SplashProjectile")] {
        pick_up(&mut e, name);
        assert!(projectile(&e).eq_ignore_ascii_case(want), "{name}: {} instead of {want}", projectile(&e));
    }
    // The reload arch (always on): `setProjectile(defaultProjectile)` while
    // the default is the saved cartridge refills its ammo and keeps the
    // power-up cartridge (tt-game `set_projectile`). Drain the saved ammo,
    // drive through the arch.
    let saved = tank(&e).saved.as_ref().map(|p| (p.name.clone(), p.rounds)).expect("default cartridge saved");
    assert!(saved.0.eq_ignore_ascii_case(&default), "saved {saved:?}");
    {
        let (_, id) = player(&e).unwrap();
        let g = e.host.game.as_mut().unwrap();
        g.world.tanks.iter_mut().find(|t| t.id == id).unwrap().saved_ammo = 1;
    }
    let entered = |e: &Engine| e.host.game.as_ref().unwrap().event_counts.get("PowerUpEntered").copied().unwrap_or(0);
    let before = entered(&e);
    place_before(&mut e, "PupReload", 10.0);
    e.key("w", true, None);
    let mut reloaded = false;
    for _ in 0..4000 / tt_engine::TICK_MS {
        e.tick();
        if tank(&e).saved_ammo == saved.1 {
            reloaded = true;
            break;
        }
    }
    e.key("w", false, None);
    assert!(reloaded && entered(&e) == before + 1, "reload arch: saved ammo {} of {} (tank at {})", tank(&e).saved_ammo, saved.1, tank(&e).pos);
    assert!(projectile(&e).eq_ignore_ascii_case("SplashProjectile"), "keeps the cartridge");
    assert!(trigger(&e, "PupReload").on, "the arch never switches off");
    pick_up(&mut e, "PupWeapon");
    assert!(["SpeedyProjectile", "BounceProjectile", "SplashProjectile", "MortarProjectile", "ScatterProjectile"].iter().any(|p| projectile(&e).eq_ignore_ascii_case(p)), "random weapon: {}", projectile(&e));
    // Heal: hurt the tank (MediumTank damage_amount is 14) right before.
    place_before(&mut e, "PupHeal", 10.0);
    let (o, _) = player(&e).unwrap();
    {
        let g = e.host.game.as_mut().unwrap();
        let t = g.tank_of(o).unwrap();
        g.world.hurt_me(t, "8");
    }
    e.tick();
    let hurt = tank(&e).damage;
    assert!(hurt > 7.0, "hurtMe: {hurt}");
    assert!(drive_into(&mut e, "PupHeal", 4000), "PupHeal: not picked up (tank at {})", tank(&e).pos);
    // Repair alone would take far longer than the drive.
    assert_eq!(tank(&e).damage, 0.0, "healed from {hurt}");
    // The Mortar and Scatter shot pick-ups: their cartridge, full ammo,
    // and the announcer line a second later.
    for (name, want, count, voice) in [("PupMortar", "MortarProjectile", 6, "ActivateMortar"), ("PupScatter", "ScatterProjectile", 16, "ActivateScatter")] {
        pick_up(&mut e, name);
        assert!(projectile(&e).eq_ignore_ascii_case(want), "{name}: {} instead of {want}", projectile(&e));
        assert_eq!(tank(&e).ammo, count, "{name}: ammo");
        assert!(e.host.audio.profile_name(voice).is_some(), "{name}: sound profile {voice}");
    }
    // 3 s respawn; none turned into another def.
    for _ in 0..3500 / tt_engine::TICK_MS {
        e.tick();
    }
    for n in ["PupSpeedy", "PupBounce", "PupSplash", "PupWeapon", "PupHeal", "PupMortar", "PupScatter"] {
        let t = trigger(&e, n);
        assert!(t.on, "{n} back on after 3 s");
        assert!(t.def_name().starts_with("PT_"), "{n} is now {}", t.def_name());
    }
    // Pads kick the tank.
    for pad in ["BoostWest", "BoostEast", "JumpWest", "JumpEast"] {
        let before = e.host.game.as_ref().unwrap().event_counts.get("PadUsed").copied().unwrap_or(0);
        place_before(&mut e, pad, 8.0);
        e.key("w", true, None);
        let mut used = false;
        for _ in 0..3000 / tt_engine::TICK_MS {
            e.tick();
            if e.host.game.as_ref().unwrap().event_counts.get("PadUsed").copied().unwrap_or(0) > before {
                used = true;
                break;
            }
        }
        e.key("w", false, None);
        assert!(used, "mode {mode}: {pad} did not kick the tank (at {})", tank(&e).pos);
        for _ in 0..100 {
            e.tick();
        }
    }
    let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
}

#[test]
fn battlemode() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || check_mode(&root, 0));
}

#[test]
fn team_battlemode() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || check_mode(&root, 1));
}

#[test]
fn scrum() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || check_mode(&root, 2));
}

#[test]
fn team_scrum() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || check_mode(&root, 3));
}

/// The local client's `score` record field.
fn score(e: &Engine) -> f64 {
    let c = tt_engine::gameplay::server_host::local_client(&e.host).unwrap();
    tt_engine::gameplay::server_host::record(&e.host, c).and_then(|r| r.score).unwrap_or(0.0)
}

/// Puts the still tank `back` metres south of `p`, facing north, and lets it land.
fn place_at(e: &mut Engine, p: Vec3, back: f32) {
    for _ in 0..200 {
        e.tick();
        if tank(e).vel.length() < 0.2 {
            break;
        }
    }
    let (o, _) = player(e).unwrap();
    let xf = format!("{} {} {} 0 0 1 0", p.x, p.y - back, p.z + 1.0);
    e.host.game.as_mut().unwrap().set_transform(o, &xf);
    for _ in 0..2500 / tt_engine::TICK_MS {
        e.tick();
    }
}

/// Holds W until `done` or `ms` pass; returns whether `done` became true.
fn drive_until(e: &mut Engine, ms: u32, done: impl Fn(&Engine) -> bool) -> bool {
    e.key("w", true, None);
    let mut hit = false;
    for _ in 0..ms / tt_engine::TICK_MS {
        e.tick();
        if done(e) {
            hit = true;
            break;
        }
    }
    e.key("w", false, None);
    for _ in 0..10 {
        e.tick();
    }
    hit
}

fn carried(e: &Engine) -> Option<tt_game::FlagId> {
    let (_, id) = player(e)?;
    e.host.game.as_ref()?.world.get_flag(id)
}

/// The ball and the goal. Exactly one ball in every mode, a moving one
/// only (no Flag mission object the scene would draw as a static decoy):
/// in Scrum modes the game's own, in the battle modes the practice ball.
/// The tank picks it up and carries it into the goal: in Scrum that scores
/// a goal, in the battle modes it is a practice goal (no score) and the
/// ball goes back to its spot.
fn check_ball(root: &Path, mode: u32) {
    let mut e = start(root, mode);
    let scrum = mode >= 2;
    {
        let load = e.host.mission_load.as_ref().unwrap();
        assert!(!load.nodes.iter().any(|(_, o)| o.class.eq_ignore_ascii_case("Flag")), "mode {mode}: no static Flag object");
        let g = e.host.game.as_ref().unwrap();
        assert_eq!(g.world.flags.len(), 1, "mode {mode}: one ball");
        assert_eq!(g.world.flags[0].data.practice, !scrum, "mode {mode}: practice ball");
    }
    // Let the ball settle.
    for _ in 0..4000 / tt_engine::TICK_MS {
        e.tick();
    }
    let start_pos = e.host.game.as_ref().unwrap().world.flags[0].pos;
    if !scrum {
        assert!(start_pos.truncate().length() < 1.0 && (start_pos.z - 100.0).abs() < 2.0, "practice ball at its spot: {start_pos}");
    }
    place_at(&mut e, start_pos, 8.0);
    let ball = e.host.game.as_ref().unwrap().world.flags[0].pos;
    assert!(drive_until(&mut e, 4000, |e| carried(e).is_some()), "mode {mode}: ball at {ball} not picked up (tank at {})", tank(&e).pos);
    let before = score(&e);
    let goal: Vec3 = trigger(&e, "Goal").transform.translation.into();
    if !scrum {
        assert_eq!(goal, Vec3::new(0.0, 30.0, 100.0), "the goal stays put outside Scrum");
    }
    place_at(&mut e, goal, 8.0);
    assert!(carried(&e).is_some(), "mode {mode}: still carrying after the teleport");
    assert!(drive_until(&mut e, 4000, |e| carried(e).is_none()), "mode {mode}: not delivered into the goal at {goal} (tank at {})", tank(&e).pos);
    if scrum {
        assert_eq!(score(&e), before + 1.0, "mode {mode}: a goal scores 1");
    } else {
        assert_eq!(score(&e), before, "mode {mode}: a practice goal does not score");
    }
    let g = e.host.game.as_ref().unwrap();
    assert_eq!(g.world.flags.len(), 1);
    let f = &g.world.flags[0];
    assert!(f.carrier.is_none());
    if !scrum {
        assert!(f.pos.truncate().length() < 1.0 && f.pos.z > 99.0, "practice ball back at its spot: {}", f.pos);
    }
    let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
}

#[test]
fn ball_battlemode() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || check_ball(&root, 0));
}

#[test]
fn ball_team_battlemode() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || check_ball(&root, 1));
}

#[test]
fn ball_scrum() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || check_ball(&root, 2));
}

#[test]
fn ball_team_scrum() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || check_ball(&root, 3));
}

// ---- the Mortar's range follows the aim (camera aim and classic) ----

/// Pulls the trigger once and follows the new mortar shell: where it lands
/// (the blast point on the surface) and after how many seconds.
fn mortar_shot(e: &mut Engine) -> (Vec3, f32) {
    let before: std::collections::HashSet<u32> = e.host.game.as_ref().unwrap().world.projectiles.iter().map(|p| p.id.0).collect();
    e.host.client.moves.trigger(0);
    let mut shell = None;
    for k in 0..400 {
        e.tick();
        let g = e.host.game.as_ref().unwrap();
        let Some((id, at)) = shell else {
            shell = g.world.projectiles.iter().find(|p| p.data.name == "MortarProjectile" && !before.contains(&p.id.0)).map(|p| (p.id, k));
            if shell.is_some() {
                e.host.client.moves.trigger(0);
            }
            continue;
        };
        if let Some(p) = g.world.projectiles.iter().find(|p| p.id == id) {
            if p.exploded_at.is_some() {
                return (p.pos - Vec3::new(0.0, 0.0, p.data.radius), (k - at) as f32 * tt_engine::TICK_MS as f32 / 1000.0);
            }
        }
    }
    panic!("no mortar shot landed (fired: {})", shell.is_some());
}

/// A fresh Mortar cartridge (6 rounds) for the player.
fn give_mortar(e: &mut Engine) {
    let (_, id) = player(e).unwrap();
    let g = e.host.game.as_mut().unwrap();
    let p = g.projectile("MortarProjectile").unwrap();
    g.world.set_projectile(id, p);
    for _ in 0..60 {
        e.tick();
    }
}

/// Camera aim: for several view pitches the shell comes down on the point
/// under the screen centre; returns how many aim points were within reach.
fn mortar_follows_camera_aim(e: &mut Engine, label: &str, yaw_deg: f32) -> usize {
    e.set_var("input.aim_mode", "1");
    for _ in 0..30 {
        e.tick();
    }
    let (_, id) = player(e).unwrap();
    // The view `yaw_deg` right of the hull's heading.
    {
        let g = e.host.game.as_mut().unwrap();
        let fwd = g.world.tank(id).unwrap().rot.y_axis;
        g.free_aim.yaw = tt_engine::game::wrap_pi(tt_engine::game::heading_of(fwd) + yaw_deg.to_radians());
    }
    let (mut on_target, mut last, mut last_aim) = (0, 0.0f32, 0.0f32);
    let mut pitch = -0.3f32;
    while pitch < 1.2 {
        pitch += 0.002;
        // The point under the screen centre at this view pitch (the view
        // depends on the tank and the free camera only).
        let (aim, from) = {
            let g = e.host.game.as_mut().unwrap();
            g.free_aim.pitch = pitch;
            let v = g.free_view_at_tick(id).unwrap();
            (g.aim_point(&v, id), g.world.tank(id).unwrap().pos)
        };
        let d_aim = (aim - from).truncate().length();
        // Aim points about 15 m apart over the mortar's reach, and one beyond.
        if d_aim < last_aim + 15.0 || (last_aim > 150.0) {
            continue;
        }
        last_aim = d_aim;
        give_mortar(e);
        // The turret follows the view.
        for _ in 0..40 {
            e.tick();
        }
        // The aim point as the shot leaves (a view grazing a slope moves it
        // with the slightest settling of the tank).
        let (aim, from) = {
            let g = e.host.game.as_ref().unwrap();
            let v = g.free_view_at_tick(id).unwrap();
            (g.aim_point(&v, id), g.world.tank(id).unwrap().pos)
        };
        let d_aim = (aim - from).truncate().length();
        let (land, time) = mortar_shot(e);
        let d_land = (land - from).truncate().length();
        let miss = (land - aim).truncate().length();
        println!("{label} view {yaw_deg:+.0}° pitch {pitch:+.2}: aim point {d_aim:5.1} m (height {:+5.1}), lands {d_land:5.1} m, {miss:4.2} m off, {time:.2} s", aim.z - from.z);
        if (25.0..=145.0).contains(&d_aim) {
            on_target += 1;
            assert!(miss < 2.0, "{label}: view pitch {pitch}: the shell lands {miss} m from the aim point ({d_aim} m away)");
        }
        assert!(d_land > last - 1.0, "{label}: looking up reaches further ({last} -> {d_land})");
        last = d_land;
    }
    on_target
}

#[test]
fn mortar_lands_under_the_screen_centre() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        // The Power-up Test map (flat), then a hilly new map (tt1_5): up a
        // hillside and down into a valley.
        let mut e = start(&root, 0);
        let n = mortar_follows_camera_aim(&mut e, "poweruptest", 0.0);
        assert!(n >= 6, "{n} shots within reach");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
        let mut e = start_on(&root, "missions/tt1_5.ron", 0);
        // (x, y, hull rotation about +z in radians, view yaw from the hull)
        for (x, y, rot, yaw, label) in [(30.0f32, 105.0f32, std::f32::consts::FRAC_PI_2, 0.0f32, "tt1_5 uphill"), (-80.0, 105.0, -std::f32::consts::FRAC_PI_2, 0.0, "tt1_5 downhill")] {
            let (o, _) = player(&e).unwrap();
            let g = e.host.game.as_mut().unwrap();
            let z = g.world.terrain.as_ref().unwrap().height(x, y);
            g.set_transform(o, &format!("{x} {y} {} 0 0 1 {rot}", z + 1.0));
            for _ in 0..2500 / tt_engine::TICK_MS {
                e.tick();
            }
            let n = mortar_follows_camera_aim(&mut e, label, yaw);
            assert!(n >= 3, "{label}: {n} shots within reach");
        }
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

#[test]
fn mortar_classic_aim_mouse_up_reaches_further() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = start(&root, 0);
        let mouse = |e: &mut Engine, d: f32| e.input(&tt_engine::input::InputEvent::Axis { device: tt_engine::input::Device::Mouse, axis: "yaxis".into(), delta: d });
        give_mortar(&mut e);
        // Barrel all the way down: the shortest reach.
        for _ in 0..10 {
            mouse(&mut e, 200.0);
            e.tick();
        }
        let mut last = 0.0;
        let mut ranges = Vec::new();
        for step in 0..8 {
            give_mortar(&mut e);
            let from = tank(&e).pos;
            let pn = tank(&e).pitch_n;
            let (land, time) = mortar_shot(&mut e);
            let d = (land - from).truncate().length();
            println!("classic aim step {step}: pitchN {pn:+.2}, lands {d:5.1} m after {time:.2} s");
            assert!(d > last + 5.0, "mouse up reaches further: {last} -> {d}");
            last = d;
            ranges.push(d);
            // Mouse up (negative y).
            mouse(&mut e, -100.0);
            for _ in 0..10 {
                e.tick();
            }
        }
        assert!(ranges[0] < 30.0 && last > 100.0, "{ranges:?}");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

//! Boots the game from the asset directory headless (with the menu's native shell,
//! no window), starts games through the menu actions and plays them through
//! the engine's input path.

use tt_engine::{mission, Config, Engine};
use tt_menu::MenuAction;

mod common;
use common::*;

#[test]
fn boots_to_player_selection() {
    let root = assets();
    big_stack(move || {
        let mut e = engine_wall_clock(&root, "boot", &[]);
        assert!(e.boot());
        ticks(&mut e, 10_000);
        println!("{}", e.report(20));
        // No intro: the player selection stand-in is the canvas content.
        assert_eq!(content(&e), "loadPlayerGui");
        assert!(e.host.canvas.is_awake(tt_engine::canvas::Ctrl::LoadPlayerGui));
        assert!(e.menu.is_some());
        assert!(!e.host.quit_requested);
        new_player(&mut e, "Tester");
        ticks(&mut e, 1_000);
        assert_eq!(e.var("player.name"), "Tester");
        assert_eq!(content(&e), "tankSelectionGui");
        // Prefs go to the data dir on exit, never to the game dir.
        e.shutdown();
        assert!(e.host.fs.data_dir().join("settings.ron").exists());
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

#[test]
fn mission_scene_from_ron() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = engine_wall_clock(&root, "mission", &[]);
        assert!(e.boot());
        mission::load_dev_mission(&mut e, "missions/tt1_1.ron").unwrap();
        let (objects, defs) = mission::mission_objects(&e.host).unwrap();
        assert!(objects.iter().any(|o| o.class == "Terrain"));
        assert!(objects.iter().filter(|o| o.class == "Prop").count() > 10);
        assert!(defs.iter().any(|d| d.class == "PowerUpDef"));
        // The simulation is built on the next tick; entities the server
        // creates later (the scrum Goal) join it and follow set_transform.
        e.tick();
        let triggers = e.host.game.as_ref().unwrap().world.triggers.len();
        let o = tt_assets::mission::SceneObject { class: "PowerUp".into(), name: "TestGoal".into(), def: false, parent: None, fields: vec![("def".into(), "DefaultGoal".into())], children: Vec::new() };
        let g = e.host.game.as_mut().unwrap();
        let goal = g.add_trigger(&o).unwrap();
        g.set_transform(goal, "10 20 30 0 0 1 0");
        assert_eq!(g.world.triggers.len(), triggers + 1);
        let t = g.world.triggers.last().unwrap();
        assert_eq!(glam::Vec3::from(t.transform.translation), glam::Vec3::new(10.0, 20.0, 30.0));
        g.remove_entity(goal);
        assert_eq!(g.world.triggers.len(), triggers);
        let scene = mission::build_scene(&e.host, &root, true).unwrap().unwrap();
        assert!(!scene.terrain_vertices.is_empty());
        assert_eq!(scene.sky_faces.len(), 5);
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

fn ticks(e: &mut Engine, ms: u32) {
    for _ in 0..(ms / tt_engine::TICK_MS) {
        e.tick();
    }
}

/// Boots, creates a player and starts a solo game (Solo Campaign → Launch).
fn start_solo(e: &mut Engine) {
    assert!(e.boot());
    ticks(e, 1_000);
    new_player(e, "Tester");
    apply(e, MenuAction::StartSolo);
    for _ in 0..(60_000 / tt_engine::TICK_MS) {
        if content(e) == "PlayGui" {
            break;
        }
        e.tick();
    }
    ticks(e, 2_000);
    assert_eq!(content(e), "PlayGui", "{}", e.report(20));
}

/// New player -> solo: the local connection and the mission download phases
/// bring the client into PlayGui; leaving the game disconnects back to the
/// menus.
#[test]
fn solo_game_reaches_playgui() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = engine_wall_clock(&root, "solo", &[]);
        start_solo(&mut e);
        println!("{}", e.report(20));
        let tank = e.control_object().expect("control object");
        assert!(e.host.game.as_ref().and_then(|g| g.tank_of(tank)).is_some(), "a tank");
        assert!(e.view_camera().is_some());
        assert!(e.host.mission_load.is_some());
        // Drive: hold forward for 3 s; the tank moves and stays on the ground.
        let pos = |e: &Engine| {
            let g = e.host.game.as_ref().expect("simulation");
            let t = g.world.tank(g.tank_of(tank).expect("sim tank")).expect("tank");
            (t.pos, g.world.terrain.as_ref().map(|h| h.height(t.pos.x, t.pos.y)).unwrap_or(0.0))
        };
        let (p0, _) = pos(&e);
        e.key("w", true, None);
        ticks(&mut e, 3_000);
        e.key("w", false, None);
        let (p1, ground) = pos(&e);
        assert!((p1 - p0).truncate().length() > 20.0, "moved {p0} -> {p1}");
        assert!(p1.z > ground - 0.5 && p1.z < ground + 3.0, "z {} ground {ground}", p1.z);
        // `getTransform()` follows the sim.
        let t: Vec<f32> = e.host.game.as_ref().unwrap().script_transform(tank).split(' ').map(|v| v.parse().unwrap()).collect();
        assert!((t[0] - p1.x).abs() < 0.01 && (t[1] - p1.y).abs() < 0.01);
        // Fire: a projectile leaves the player's tank.
        let shots = |e: &Engine| e.host.game.as_ref().unwrap().world.tanks.iter().find(|t| t.ai.is_none()).map(|t| t.fire_count).unwrap_or(0);
        let before = shots(&e);
        e.mouse_button(0, true);
        e.mouse_button(0, false);
        ticks(&mut e, 100);
        assert_eq!(shots(&e), before + 1);
        let g = e.host.game.as_ref().unwrap();
        let me = g.tank_of(tank).unwrap();
        assert!(g.world.projectiles.iter().any(|p| p.source == Some(me) && p.is_live()));
        assert!(e.game_view().is_some() && !e.dynamic_shapes().is_empty());
        e.key("escape", true, None);
        e.key("escape", false, None);
        ticks(&mut e, 1_000);
        assert!(e.host.shell.pause_request, "pause requested");
        // Leave the game: disconnect, server destroyed, back to the menus.
        apply(&mut e, MenuAction::LeaveGame);
        ticks(&mut e, 2_000);
        assert_eq!(content(&e), "tankSelectionGui");
        assert!(e.host.mission_load.is_none());
        assert!(e.host.net.server.is_none() && e.host.net.client.is_none());
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

/// Into a solo game, then: death -> SPACE after longer than a bot corpse
/// lives -> respawn; two more deaths -> game over -> high-score screen; the
/// high scores are written to the data dir and decode on the next start.
#[test]
fn solo_death_respawn_game_over() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = engine_wall_clock(&root, "gameover", &[]);
        start_solo(&mut e);
        // An engine field now (the gameplay state's record): read like a script does.
        let lives = |e: &mut Engine| tt_engine::gameplay::server_host::local_record(&e.host).and_then(|r| r.lives).map(|l| l.to_string()).unwrap_or_default();
        for round in 0..3 {
            let first = e.control_object().unwrap();
            if let Some(g) = e.host.game.as_mut() {
                let t = g.tank_of(first).unwrap();
                g.world.hurt_me(t, "100");
            }
            ticks(&mut e, 7_000);
            assert!(e.host.game.as_ref().is_some_and(|g| g.entities.get(first).is_some()), "player corpse kept until respawn (round {round})");
            e.key("space", true, None);
            e.key("space", false, None);
            ticks(&mut e, 2_000);
            if round < 2 {
                assert_eq!(lives(&mut e), (2 - round).to_string());
                assert_ne!(Some(first), e.control_object(), "respawned");
            }
        }
        ticks(&mut e, 2_000);
        assert_eq!(content(&e), "soloGameOverGui");
        let data = e.host.fs.data_dir().to_path_buf();
        assert!(data.join("hiscores.ron").exists());
        let cfg = Config { assets: root.clone(), data: data.clone(), args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
        let mut e2 = Engine::new(&cfg).unwrap();
        assert!(e2.boot());
        let top = e2.host.shell.hiscores.entry(0).expect("top score decodes");
        assert!(!top.name.is_empty(), "{top:?}");
        let _ = std::fs::remove_dir_all(&data);
    });
}

/// Turret aim from mouse motion is symmetric: equal +X and −X motion gives
/// opposite yaw, no motion gives no drift, and full left/right deflection
/// renders mirror-image turret angles.
#[test]
fn turret_aim_is_symmetric() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = engine_wall_clock(&root, "turret", &[]);
        start_solo(&mut e);
        // Classic aim for this test (after the boot, which sets the defaults).
        tt_engine::shell::set_pref(&mut e.host, "input.aim_mode", "0");
        let tank = e.control_object().unwrap();
        let yaw_n = |e: &Engine| {
            let g = e.host.game.as_ref().unwrap();
            g.world.tank(g.tank_of(tank).unwrap()).unwrap().yaw_n
        };
        let mouse = |e: &mut Engine, dx: f32, n: u32| {
            for _ in 0..n {
                e.input(&tt_engine::input::InputEvent::Axis { device: tt_engine::input::Device::Mouse, axis: "xaxis".into(), delta: dx });
                e.tick();
            }
        };
        let y0 = yaw_n(&e);
        ticks(&mut e, 2_000);
        assert_eq!(yaw_n(&e), y0, "no drift without mouse motion");
        mouse(&mut e, 5.0, 10);
        let right = yaw_n(&e) - y0;
        mouse(&mut e, -5.0, 20);
        let left = yaw_n(&e) - y0;
        assert!(right > 0.05, "right {right}");
        assert!((right + left).abs() < 1e-3, "right {right} left {left}");
        // Full deflection both ways: rendered turret angles mirror each other.
        let turret_angle = |e: &Engine| {
            let inst = e.dynamic_shapes().into_iter().find(|i| i.nodes.iter().any(|(n, _)| n == "codeTurret") && i.transform.w_axis.truncate().distance(e.host.game.as_ref().unwrap().world.tank(e.host.game.as_ref().unwrap().tank_of(tank).unwrap()).unwrap().pos) < 0.5).unwrap();
            let shape = tt_render::pose::shape(&inst.shape).unwrap();
            let d = tt_render::pose::node_default(&shape, "codeTurret").unwrap();
            let m = inst.nodes.iter().find(|(n, _)| n == "codeTurret").unwrap().1;
            let r = d.inverse() * m;
            r.y_axis.x.atan2(r.y_axis.y)
        };
        mouse(&mut e, 400.0, 10);
        assert_eq!(yaw_n(&e), 1.0);
        let a_right = turret_angle(&e);
        mouse(&mut e, -400.0, 20);
        assert_eq!(yaw_n(&e), -1.0);
        let a_left = turret_angle(&e);
        assert!(a_right > 0.5 && (a_right + a_left).abs() < 1e-3, "right {a_right} left {a_left}");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

/// In camera aim the mouse turns a free camera at once and without limits; the turret
/// is steered to where it looks, and the hull turns when the view is beyond the
/// turret's range.
#[test]
fn camera_aim_free_camera_steers_the_turret() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        use tt_engine::game::{heading_of, wrap_pi};
        let mut e = engine_wall_clock(&root, "aimmode", &[]);
        start_solo(&mut e);
        // Classic aim for this test (after the boot, which sets the defaults).
        tt_engine::shell::set_pref(&mut e.host, "input.aim_mode", "0");
        let obj = e.control_object().expect("control object");
        let id = e.host.game.as_ref().unwrap().tank_of(obj).unwrap();
        let tank = |e: &Engine| e.host.game.as_ref().unwrap().world.tank(id).unwrap().clone();
        let mut now = 0.0;
        // Camera look (heading, pitch), snapped.
        let mut look = |e: &mut Engine| {
            now += 1.0e6;
            let aim = tt_engine::game::aim_mode(&e.host.shell.prefs);
            let m = e.host.game.as_mut().unwrap().camera_transform(aim, obj, now, 1.0).unwrap();
            let l: glam::Vec3 = m.matrix3.y_axis.into();
            (heading_of(l), l.z.asin())
        };
        let mouse = |e: &mut Engine, axis: &str, d: f32| e.input(&tt_engine::input::InputEvent::Axis { device: tt_engine::input::Device::Mouse, axis: axis.into(), delta: d });
        // Classic (set above; camera aim is the default): the mouse turns the
        // turret, the camera stays on the hull.
        assert_eq!(tt_engine::game::aim_mode(&e.host.shell.prefs), tt_engine::game::AimMode::Classic);
        mouse(&mut e, "xaxis", 100.0);
        ticks(&mut e, 200);
        let t = tank(&e);
        assert!(t.yaw_n > 0.03, "turret turned: {}", t.yaw_n);
        assert!(wrap_pi(look(&mut e).0 - heading_of(t.rot.y_axis)).abs() < 0.03, "classic camera along the hull");
        assert!(e.host.game.as_ref().unwrap().free_aim.tank.is_none());

        // Camera aim: the free camera starts along the turret.
        e.set_var("input.aim_mode", "1");
        ticks(&mut e, 64);
        let g = e.host.game.as_ref().unwrap();
        assert_eq!(g.free_aim.tank, Some(id));
        let t = tank(&e);
        let turret = heading_of(t.rot.y_axis) + (t.data.turret_yaw_range * t.yaw_n).to_radians();
        // The turret aims at the point under the screen centre; the eye sits
        // behind and above the tank, so on uneven ground the two headings
        // differ by a few degrees of parallax.
        assert!(wrap_pi(g.free_aim.yaw - turret).abs() < 0.12, "starts along the turret: view {} turret {}", g.free_aim.yaw, turret);
        // Mouse motion turns the view at once (before any tick), by the
        // scripts' sensitivity, and leaves no turret input behind.
        let (h0, _) = look(&mut e);
        mouse(&mut e, "xaxis", 10.0);
        let Engine { host, .. } = &mut e;
        host.game.as_mut().unwrap().free_look_input(&mut host.client.moves);
        assert_eq!(e.host.client.moves.yaw.string(), "0");
        let (h1, _) = look(&mut e);
        let turn = wrap_pi(h1 - h0);
        assert!(turn > 0.05 && turn < 0.3, "view turned right at once: {turn}");
        // The turret converges on the view within its range.
        ticks(&mut e, 300);
        let converged = |e: &mut Engine| {
            let Engine { host, .. } = e;
            let g = host.game.as_mut().unwrap();
            let v = g.free_view_at_tick(id).unwrap();
            let tg = g.turret_target(id, &v).unwrap();
            let t = g.world.tank(id).unwrap();
            (tg, t.yaw_n, t.pitch_n)
        };
        let (tg, y, p) = converged(&mut e);
        assert!(tg.beyond == 0.0 && (tg.yaw_n - y).abs() < 0.02 && (tg.pitch_n - p).abs() < 0.05, "{tg:?} vs {y} {p}");
        // Looking up: the view and the barrel go up.
        let (_, p0) = look(&mut e);
        mouse(&mut e, "yaxis", -40.0);
        ticks(&mut e, 300);
        let (_, p1) = look(&mut e);
        assert!(p1 > p0 + 0.2, "looked up: {p0} -> {p1}");
        assert!(tank(&e).pitch_n < p - 0.2, "barrel up: {p} -> {}", tank(&e).pitch_n);
        mouse(&mut e, "yaxis", 40.0);

        // Far beyond the turret's range (no limit, no lag): the turret pins
        // and the hull turns until the view is within range again.
        let hull0 = heading_of(tank(&e).rot.y_axis);
        let (v0, _) = look(&mut e);
        mouse(&mut e, "xaxis", -400.0);
        let Engine { host, .. } = &mut e;
        host.game.as_mut().unwrap().free_look_input(&mut host.client.moves);
        let (v1, _) = look(&mut e);
        assert!(wrap_pi(v1 - v0) < -1.5 || wrap_pi(v1 - v0) > 2.0, "view swung round: {v0} -> {v1}");
        ticks(&mut e, 200);
        let t = tank(&e);
        assert!(t.yaw_n < -0.99, "pinned left: {}", t.yaw_n);
        assert!(wrap_pi(heading_of(t.rot.y_axis) - hull0) < -0.1, "hull turning left");
        ticks(&mut e, 5_000);
        let (tg, y, _) = converged(&mut e);
        assert!(tg.beyond == 0.0 && (tg.yaw_n - y).abs() < 0.02, "hull brought the view into range: {tg:?} {y}");

        e.set_var("input.aim_mode", "0");
        ticks(&mut e, 64);
        assert!(e.host.game.as_ref().unwrap().free_aim.tank.is_none());
        let t = tank(&e);
        assert!(wrap_pi(look(&mut e).0 - heading_of(t.rot.y_axis)).abs() < 0.03);
        mouse(&mut e, "xaxis", 5.0);
        ticks(&mut e, 64);
        assert!(tank(&e).yaw_n > t.yaw_n + 0.01, "mouse turns the turret");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

/// The Light tank's (tank01) `fire` sequence spins its barrels: after a shot
/// `barrelA2` turns away from its rest pose mid-sequence (smoothly between
/// ticks), and is back at rest once the 0.17 s are over.
#[test]
fn light_tank_barrels_spin_when_firing() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = engine_wall_clock(&root, "barrels", &[]);
        start_solo(&mut e);
        let tank = e.control_object().unwrap();
        let shape = tt_render::pose::shape(&root.join("models/tanks/tank01.dts")).expect("tank01");
        let rest = tt_render::pose::node_default(&shape, "barrelA2").unwrap();
        let g = e.host.game.as_mut().unwrap();
        let id = g.tank_of(tank).unwrap();
        let mut barrel = |fired_ms_ago: i64, alpha: f32| {
            let now = g.world.time_ms;
            g.last_fired.insert(id, now - fired_ms_ago);
            let mut inst = tt_render::DynamicInstance::default();
            tt_engine::game::pose_tank(g, g.world.tank(id).unwrap(), alpha, &shape, &mut inst);
            inst.nodes.iter().find(|(n, _)| n == "barrelA2").map(|(_, m)| *m).unwrap_or(rest)
        };
        let angle = |m: glam::Mat4| glam::Quat::from_mat4(&(rest.inverse() * m)).angle_between(glam::Quat::IDENTITY);
        let (a0, a1, a2) = (angle(barrel(64, 0.0)), angle(barrel(64, 0.5)), angle(barrel(64, 1.0)));
        assert!(a0 > 0.3 && a1 > a0 && a2 > a1, "spinning mid-sequence: {a0} {a1} {a2}");
        assert!(barrel(64, 1.0).w_axis.distance(rest.w_axis) > 1e-3, "the barrel orbits the axis");
        assert_eq!(barrel(0, 0.0), rest, "starts from rest at the shot");
        assert_eq!(barrel(500, 1.0), rest, "back at rest after the sequence");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

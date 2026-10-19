//! - A power-up's `activate` thread: it grows in when switched on and
//!   shrinks away (still drawn) when taken.
//! - a projectile shape's first sequence (the ejected brain jar's `rocket`
//!   lights its exhaust flame);
//! - the scrum ball's first sequence (`ambient`, looping);
//! - a corpse's death thread over the tread IFLs (`death3`).

use glam::{Mat4, Vec3};
use std::path::Path;
use tt_engine::Engine;
use tt_game::Event;
use tt_menu::{CustomGame, MenuAction, MenuBackend, Team};

mod common;
use common::*;

const MISSION: &str = "missions/poweruptest.ron";

fn me(e: &Engine) -> Option<tt_game::TankId> {
    let o = e.control_object()?;
    let g = e.host.game.as_ref()?;
    let id = g.tank_of(o)?;
    g.world.tank(id).filter(|t| !t.deleted && !t.is_corpse)?;
    Some(id)
}

fn steps(e: &mut Engine, ms: u32) {
    for _ in 0..ms / tt_engine::TICK_MS {
        e.tick();
    }
}

/// The Power-up Test map in battle mode, the player spawned.
fn start(root: &Path, tag: &str) -> Engine {
    let data = std::env::temp_dir().join(format!("tt-engine-test-anims-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let cfg = tt_engine::Config { assets: root.to_path_buf(), data, args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    assert!(e.boot());
    steps(&mut e, 1000);
    {
        let Engine { host, menu, .. } = &mut e;
        let m = menu.as_mut().unwrap();
        tt_engine::menu::Backend::new(host, &mut m.data).create_player("Tester").unwrap();
    }
    let g = CustomGame { mission: MISSION.into(), mode: 0, bots: Vec::new(), player_tank: 1, player_team: Team::Green, minutes: 10, replace_bots: true };
    {
        let Engine { host, menu, .. } = &mut e;
        menu.as_mut().unwrap().apply(MenuAction::StartCustom(g), host);
    }
    for i in 0..3000 {
        e.tick();
        if i % 60 == 0 {
            e.key("space", true, None);
            e.key("space", false, None);
        }
        if me(&e).is_some() {
            break;
        }
    }
    assert!(me(&e).is_some(), "the player did not spawn");
    steps(&mut e, 1000);
    e
}

/// Scale of `node` in a posed instance (1 when not posed).
fn node_scale(nodes: &[(String, Mat4)], node: &str) -> f32 {
    nodes.iter().find(|(n, _)| n.eq_ignore_ascii_case(node)).map_or(1.0, |(_, m)| m.x_axis.truncate().length())
}

/// The drawn power-up at `pos`: (`powerup2` scale, its translation).
fn drawn(e: &Engine, pos: Vec3) -> Option<(f32, Option<Vec3>)> {
    let (shapes, _) = e.trigger_shapes_with_anim();
    let s = shapes.iter().find(|s| (s.transform.w_axis.truncate() - pos).length() < 0.01)?;
    let tr = s.nodes.iter().find(|(n, _)| n.eq_ignore_ascii_case("powerup2")).map(|(_, m)| m.w_axis.truncate());
    Some((node_scale(&s.nodes, "powerup2"), tr))
}

fn switch(e: &mut Engine, id: tt_game::TriggerId, on: bool) {
    let w = &mut e.host.game.as_mut().unwrap().world;
    let t = w.triggers.iter_mut().find(|t| t.id == id).unwrap();
    t.on = on;
    t.off_until = i64::MAX;
    w.push_event(Event::PowerUpSwitched { trigger: id, on });
}

/// A weapon pick-up taken shrinks away over its `activate` sequence
/// (0.18 s, `powerup2` scale 1 -> 0.03, played backwards) instead of vanishing, and grows
/// back in when it reappears; the ambient bob stays under the scale.
#[test]
fn power_up_grows_in_and_shrinks_away() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = start(&root, "activate");
        let (id, pos) = {
            let g = e.host.game.as_ref().unwrap();
            let t = g.world.triggers.iter().find(|t| t.on && matches!(&t.kind, tt_game::TriggerKind::PowerUp(d) if d.shape.to_ascii_lowercase().ends_with("powerup.dts"))).expect("a weapon pick-up that is on");
            (t.id, Vec3::from(t.transform.translation))
        };
        let (s, bob) = drawn(&e, pos).expect("drawn while on");
        assert!((s - 1.0).abs() < 1e-3, "full size while on: {s}");
        assert!(bob.is_some(), "the ambient bob poses powerup2");

        // Taken: drawn smaller and smaller, then gone.
        switch(&mut e, id, false);
        let mut scales = Vec::new();
        for _ in 0..10 {
            e.tick();
            match drawn(&e, pos) {
                Some((s, tr)) => {
                    assert!(tr.is_some(), "powerup2 keeps its ambient translation while scaled");
                    scales.push(s);
                }
                None => break,
            }
        }
        eprintln!("shrinking: {scales:?}");
        assert!(scales.len() >= 3 && scales.len() <= 7, "drawn for the 0.18 s of the thread: {scales:?}");
        // (The pop overshoots a little, up to ~1.07.)
        assert!(*scales.last().unwrap() < 0.7 && scales.iter().all(|&s| s < 1.1), "shrinks: {scales:?}");
        assert!(drawn(&e, pos).is_none(), "an off pick-up is not drawn once shrunk");

        // Back on: grows from small to full size.
        steps(&mut e, 500);
        switch(&mut e, id, true);
        let mut scales = Vec::new();
        for _ in 0..10 {
            e.tick();
            scales.push(drawn(&e, pos).expect("drawn once on").0);
        }
        eprintln!("growing: {scales:?}");
        assert!(scales[0] < 0.9, "starts small (the pop is quick: 0.03 -> ~0.8 in its first 32 ms): {scales:?}");
        assert!((scales.last().unwrap() - 1.0).abs() < 1e-3, "full size once the thread is done: {scales:?}");
        assert!(scales.iter().all(|&s| s < 1.1), "grows: {scales:?}");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

/// The ejected brain jar flies with its exhaust flame lit (`rocket`, its
/// shape's first sequence, played by the projectile from the spawn).
#[test]
fn ejected_brain_jar_lights_its_exhaust() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = start(&root, "brain");
        let id = me(&e).unwrap();
        // The mounted jar never plays it.
        let jar = |e: &Engine| e.dynamic_shapes().into_iter().find(|s| s.shape.to_string_lossy().to_ascii_lowercase().contains("brainjar"));
        let mounted = jar(&e).expect("the mounted brain jar");
        assert!(!mounted.shown.iter().any(|o| o.eq_ignore_ascii_case("exhaust")), "a mounted jar's exhaust is off");
        e.host.game.as_mut().unwrap().world.apply_damage(id, 10_000.0, None);
        let mut lit = None;
        for _ in 0..8000 / tt_engine::TICK_MS {
            e.tick();
            let g = e.host.game.as_ref().unwrap();
            if !g.brain_shapes.is_empty() {
                let s = jar(&e).expect("the flying jar is drawn");
                lit = Some(s.shown.iter().any(|o| o.eq_ignore_ascii_case("exhaust")));
                break;
            }
        }
        assert_eq!(lit, Some(true), "the ejected jar's exhaust is lit");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

/// The scrum ball loops its `ambient`: the flag bones move and its IFL
/// follows the sequence clock.
#[test]
fn scrum_ball_plays_ambient() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = start(&root, "ball");
        let ball = |e: &Engine| {
            let (shapes, ifl) = e.dynamic_shapes_with_ifl();
            let i = shapes.iter().position(|s| s.shape.to_string_lossy().to_ascii_lowercase().ends_with("scrumball.dts")).expect("the Power-up Test map has a practice ball");
            (shapes[i].nodes.clone(), ifl[i].clone())
        };
        let (a, ia) = ball(&e);
        assert!(a.iter().any(|(n, _)| n.eq_ignore_ascii_case("flagBONEA")), "flag bones posed: {a:?}");
        assert!(ia.iter().any(|(m, _)| m.eq_ignore_ascii_case("scrumball")), "IFL on the sequence clock: {ia:?}");
        e.tick();
        e.tick();
        let (b, _) = ball(&e);
        let m = |v: &[(String, Mat4)]| v.iter().find(|(n, _)| n.eq_ignore_ascii_case("flagBONEA")).unwrap().1;
        assert!(!m(&a).abs_diff_eq(m(&b), 1e-5), "the bones move");
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

/// A corpse playing `death3` takes its tread IFL times from the death
/// thread (added after the tread threads), not from the treads.
#[test]
fn death3_drives_the_tread_ifls() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = start(&root, "death3");
        let id = me(&e).unwrap();
        let w = &mut e.host.game.as_mut().unwrap().world;
        w.apply_damage(id, 10_000.0, None);
        let t = w.tank_mut(id).unwrap();
        let k = t.data.death_sequences.len();
        assert!(k >= 3, "the player's tank has death1..3 ({k})");
        t.death_sequence = Some((2, t.data.death_sequences[2]));
        steps(&mut e, 2000);
        let (shapes, ifl) = e.dynamic_shapes_with_ifl();
        let g = e.host.game.as_ref().unwrap();
        let t = g.world.tank(id).unwrap();
        assert!(t.is_corpse && t.corpse_timer > t.data.fx.corpse_shake_time, "the death sequence is playing");
        let i = shapes.iter().position(|s| s.shape.to_string_lossy().to_ascii_lowercase().contains("tank0")).unwrap();
        let time = (t.corpse_timer - t.data.fx.corpse_shake_time).max(0.0);
        let want = tt_render::pose::sequence_ifl(&shapes[i].shape, "death3", time);
        assert!(!want.is_empty(), "death3 animates IFLs");
        for (m, v) in &want {
            let last = ifl[i].iter().rev().find(|(n, _)| n == m).map(|x| x.1);
            assert_eq!(last, Some(*v), "{m}: the death thread's time wins");
        }
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

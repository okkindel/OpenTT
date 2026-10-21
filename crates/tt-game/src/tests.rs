//! Mechanic tests with the spec's numbers.

use std::sync::Arc;

use glam::{Affine3A, Mat3, Vec3};

use crate::data::*;
use crate::fields::Fields;
use crate::geom::Aabb;
use crate::statics::StaticShape;
use crate::world::{apply_boost, Event};
use crate::*;

const EPS: f32 = 1e-3;

fn projectile(name: &str, pairs: &[(&str, &str)]) -> Arc<ProjectileParams> {
    Arc::new(ProjectileParams::from_fields(&Fields::from_pairs(name, "ProjectileDef", pairs)))
}

fn default_projectile() -> Arc<ProjectileParams> {
    projectile(
        "DefaultProjectile",
        &[
            ("velocity", "85"),
            ("inherit_velocity", "0.97"),
            ("bounces", "0"),
            ("blast_radius", "0"),
            ("rounds", "145"),
            ("keep_previous_weapon", "0"),
            ("can_be_kept", "1"),
            ("reload_ms", "150"),
            ("volley_delay_ms", "1000"),
            ("volley_size", "5"),
            ("damage", "1"),
            ("gravity_scale", "0.4"),
            ("sprite_scale", "0.4"),
        ],
    )
}

fn speedy() -> Arc<ProjectileParams> {
    projectile(
        "SpeedyProjectile",
        &[("velocity", "190"), ("rounds", "35"), ("keep_previous_weapon", "1"), ("can_be_kept", "0"), ("reload_ms", "100"), ("volley_delay_ms", "1000"), ("volley_size", "5"), ("damage", "1"), ("gravity_scale", "0.3")],
    )
}

fn splash() -> Arc<ProjectileParams> {
    projectile(
        "SplashProjectile",
        &[("velocity", "40"), ("inherit_velocity", "1"), ("blast_radius", "5"), ("rounds", "10"), ("keep_previous_weapon", "1"), ("can_be_kept", "0"), ("reload_ms", "1000"), ("volley_delay_ms", "1000"), ("volley_size", "1"), ("damage", "3"), ("gravity_scale", "0.5"), ("sprite_scale", "0.7")],
    )
}

fn medium() -> TankParams {
    let f = Fields::from_pairs(
        "MediumTank",
        "TankDef",
        &[
            ("turn_rate", "68"),
            ("accel_rate", "12"),
            ("decel_rate", "20"),
            ("coast_decel_rate", "10"),
            ("anti_slide_rate", "11.1"),
            ("max_speed", "11.9"),
            ("turret_pitch_center", "2"),
            ("turret_yaw_range", "58.5"),
            ("turret_pitch_range", "21"),
            ("turret_auto_level", "0.5"),
            ("suspension_range", "0.6"),
            ("damage_amount", "14"),
            ("damage_scale", "1"),
            ("armor_scale", "1"),
            ("fire_scale", "1"),
        ],
    );
    let mut t = TankParams::from_fields(&f, None);
    t.misc = TankMiscParams::from_fields(&Fields::from_pairs(
        "DefaultTankMisc",
        "TankMiscDef",
        &[("boost_ticks", "5"), ("bounce_static", "0.4"), ("bounce_tank", "0.75"), ("unstuck_time", "1000"), ("unstuck_boost", "5"), ("repair_rate", "0")],
    ));
    t.fx = TankFxParams::from_fields(&Fields::from_pairs(
        "DefaultTankFx",
        "TankFxDef",
        &[("corpse_smoke_time", "4"), ("corpse_fade_time", "4"), ("corpse_fade_speed", "1"), ("hard_tank_collision_speed", "10"), ("damage_emitter_start", "0.3")],
    ));
    t.default_projectile = Some(default_projectile());
    t
}

fn flat_world() -> World {
    World::new(Some(HeightField::flat(0.0)))
}

fn forward_speed(t: &Tank) -> f32 {
    t.vel.dot(t.rot.y_axis)
}

fn run(w: &mut World, ticks: usize) -> Vec<(i64, Event)> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        let now = w.time_ms;
        out.extend(w.tick().into_iter().map(|e| (now, e)));
    }
    out
}

#[test]
fn spawn_rests_on_ground_with_full_contact() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::new(3.0, 4.0, 50.0), 0.3);
    let t = w.tank(id).unwrap();
    assert!(t.all_contact && t.any_contact);
    assert!(t.pos.z.abs() < EPS, "{}", t.pos.z);
    run(&mut w, 50);
    let t = w.tank(id).unwrap();
    assert!(t.pos.z.abs() < EPS && t.vel.length() < EPS);
}

#[test]
fn acceleration_reaches_max_speed_in_31_ticks() {
    // accel_rate 12 → 0.384 m/s per tick; 11.9 / 0.384 = 30.99.
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_move(id, Move { y: 1.0, ..Move::NULL });
    run(&mut w, 1);
    assert!((forward_speed(w.tank(id).unwrap()) - 0.384).abs() < EPS);
    run(&mut w, 29);
    assert!((forward_speed(w.tank(id).unwrap()) - 30.0 * 0.384).abs() < 0.01);
    run(&mut w, 1);
    assert!((forward_speed(w.tank(id).unwrap()) - 11.9).abs() < EPS);
    run(&mut w, 20);
    let t = w.tank(id).unwrap();
    assert!((forward_speed(t) - 11.9).abs() < EPS);
    assert!(t.vel.x.abs() < EPS && t.vel.z.abs() < EPS);
}

#[test]
fn coast_is_linear_then_exponential() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.tank_mut(id).unwrap().vel = Vec3::new(0.0, 11.9, 0.0);
    run(&mut w, 1);
    assert!((forward_speed(w.tank(id).unwrap()) - (11.9 - 0.32)).abs() < EPS);
    // Down to coast_decel_rate (10) the deceleration is constant.
    let mut v = 11.9 - 0.32;
    while v > 10.0 {
        run(&mut w, 1);
        v -= 0.32;
    }
    assert!((forward_speed(w.tank(id).unwrap()) - v).abs() < 0.01);
    // Below it: v ← v·(1 − dt).
    for _ in 0..20 {
        run(&mut w, 1);
        v *= 1.0 - DT;
    }
    assert!((forward_speed(w.tank(id).unwrap()) - v).abs() < 0.01, "{} {v}", forward_speed(w.tank(id).unwrap()));
}

#[test]
fn braking_uses_deccel_rate() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.tank_mut(id).unwrap().vel = Vec3::new(0.0, 10.0, 0.0);
    w.set_move(id, Move { y: -1.0, ..Move::NULL });
    run(&mut w, 1);
    assert!((forward_speed(w.tank(id).unwrap()) - (10.0 - 20.0 * DT)).abs() < EPS);
}

#[test]
fn over_max_speed_bleeds_only_with_throttle() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.tank_mut(id).unwrap().vel = Vec3::new(0.0, 20.0, 0.0);
    w.set_move(id, Move { y: 1.0, ..Move::NULL });
    run(&mut w, 1);
    // ex = 8.1 clamped to coast_decel_rate 10 → 8.1·dt? No: clamp(8.1, ±10)·dt.
    assert!((forward_speed(w.tank(id).unwrap()) - (20.0 - 8.1 * DT)).abs() < EPS);
}

#[test]
fn turn_rate_in_degrees_per_second() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_move(id, Move { x: 1.0, ..Move::NULL });
    run(&mut w, 10);
    let f = w.tank(id).unwrap().forward();
    let heading = f.x.atan2(f.y).to_degrees();
    assert!((heading - 68.0 * 0.32).abs() < 0.01, "{heading}");
    // Dead zone: |x| ≤ 0.316 does not turn (only reachable without the
    // network quantisation, which turns 0.3 into a full +1).
    w.quantize_moves = false;
    w.set_move(id, Move { x: 0.3, ..Move::NULL });
    run(&mut w, 10);
    let f = w.tank(id).unwrap().forward();
    assert!((f.x.atan2(f.y).to_degrees() - 68.0 * 0.32).abs() < 0.01);
    w.quantize_moves = true;
    run(&mut w, 10);
    let f = w.tank(id).unwrap().forward();
    assert!((f.x.atan2(f.y).to_degrees() - 68.0 * 0.64).abs() < 0.01);
}

/// Bots rely on this (BackOff negates x itself); OpenTT's car-like reverse
/// steering is a player-input option in tt-engine and must not leak into
/// the sim.
#[test]
fn sim_turn_ignores_throttle_sign() {
    for y in [1.0, 0.0, -1.0] {
        let mut w = flat_world();
        let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
        w.set_move(id, Move { x: 1.0, y, ..Move::NULL });
        run(&mut w, 10);
        let f = w.tank(id).unwrap().forward();
        let heading = f.x.atan2(f.y).to_degrees();
        assert!((heading - 68.0 * 0.32).abs() < 0.01, "y={y}: {heading}");
    }
}

#[test]
fn free_fall_then_land() {
    let mut w = flat_world();
    let id = w.add_tank_at(Arc::new(medium()), Vec3::new(0.0, 0.0, 30.0), Mat3::IDENTITY);
    run(&mut w, 5);
    let t = w.tank(id).unwrap();
    assert!(!t.any_contact);
    assert!((t.vel.z - 5.0 * GRAVITY * DT).abs() < EPS);
    // z after n ticks (semi-implicit Euler): 30 − 0.32·dt·n(n+1)/2.
    assert!((t.pos.z - (30.0 - 0.32 * DT * 15.0)).abs() < EPS);
    run(&mut w, 150);
    let t = w.tank(id).unwrap();
    assert!(t.all_contact);
    assert!(t.pos.z.abs() < 0.01 && t.vel.length() < 0.01, "{} {}", t.pos, t.vel);
}

#[test]
fn airborne_tank_keeps_last_turn_input() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_move(id, Move { x: 1.0, ..Move::NULL });
    run(&mut w, 1);
    // Lift it into the air. The contact flags of the previous tick still say
    // "grounded" for one more tick, then the stored turn input keeps yawing
    // even though the move no longer turns.
    w.tank_mut(id).unwrap().pos.z = 40.0;
    run(&mut w, 1);
    assert!(!w.tank(id).unwrap().any_contact);
    w.set_move(id, Move::NULL);
    run(&mut w, 3);
    let f = w.tank(id).unwrap().forward();
    assert!((f.x.atan2(f.y).to_degrees() - 5.0 * 68.0 * DT).abs() < 0.05);
}

#[test]
fn slope_alignment_without_sliding() {
    // 10% ramp along +x (heights grow by 0.8 m per 8 m square).
    let n = 64;
    let h = (0..n * n).map(|i| (i % n) as f32 * 0.8).collect();
    let terrain = HeightField::new(n, h, 8.0, glam::Vec2::new(-256.0, -256.0));
    let mut w = World::new(Some(terrain));
    let id = w.add_tank(Arc::new(medium()), Vec3::new(1.0, 1.0, 0.0), 0.0);
    run(&mut w, 100);
    let t = w.tank(id).unwrap();
    assert!(t.all_contact);
    let expect = Vec3::new(-0.1, 0.0, 1.0).normalize();
    assert!((t.rot.z_axis - expect).length() < 0.01, "{}", t.rot.z_axis);
    assert!(t.vel.length() < 0.01, "slid: {}", t.vel);
}

fn wall_world(gap: f32) -> (World, TankId) {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    // Wall face at y = 1.3 (tank front) + gap.
    let y0 = 1.3 + gap;
    let wall = StaticShape::from_box(Aabb::new(Vec3::new(-10.0, y0, -1.0), Vec3::new(10.0, y0 + 2.0, 5.0)));
    w.add_static(Arc::new(wall), Affine3A::IDENTITY);
    (w, id)
}

#[test]
fn static_bounce_restitution() {
    let (mut w, id) = wall_world(0.2);
    w.tank_mut(id).unwrap().vel = Vec3::new(0.0, 8.0, 0.0);
    let ev = run(&mut w, 1);
    let t = w.tank(id).unwrap();
    // Coast first (below coast_decel_rate: −v·dt), then vel −= (1 + 0.4)·vs.
    let before = 8.0 - 8.0 * DT;
    assert!((t.vel.y + 0.4 * before).abs() < 0.01, "{}", t.vel);
    assert!(ev.iter().any(|(_, e)| matches!(e, Event::Collided { other: HitObject::Static(0), .. })));
    // It stopped at the last clear eighth, short of the wall.
    assert!(t.pos.y > 0.0 && t.pos.y + 1.3 <= 1.5 + EPS, "{}", t.pos);
}

#[test]
fn tank_tank_restitution_conserves_momentum() {
    let mut data = medium();
    data.coast_decel_rate = 0.0;
    let data = Arc::new(data);
    let mut w = World::new(None);
    let a = w.add_tank(data.clone(), Vec3::ZERO, 0.0);
    let b = w.add_tank(data, Vec3::new(0.0, 3.8, 0.0), 0.0);
    w.tank_mut(a).unwrap().vel = Vec3::new(0.0, 10.0, 0.0);
    run(&mut w, 1);
    let (va, vb) = (w.tank(a).unwrap().vel.y, w.tank(b).unwrap().vel.y);
    assert!((va + vb - 10.0).abs() < 0.01, "{va} {vb}");
    assert!(((vb - va) / 10.0 - 0.75).abs() < 0.01, "{va} {vb}");
}

#[test]
fn collision_binary_search_eighths() {
    // Gap 0.55 m, move 1 m per tick (31.25 m/s): clear up to 4/8.
    let mut data = medium();
    data.coast_decel_rate = 0.0;
    data.anti_slide_rate = 0.0;
    let (mut w, id) = wall_world(0.55);
    w.tank_mut(id).unwrap().data = Arc::new(data);
    w.tank_mut(id).unwrap().vel = Vec3::new(0.0, 1.0 / DT, 0.0);
    run(&mut w, 1);
    let t = w.tank(id).unwrap();
    assert!((t.pos.y - 0.5).abs() < EPS, "{}", t.pos);
}

#[test]
fn burst_timing_reload_and_burst_delay() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_projectile(id, default_projectile());
    w.set_move(id, Move { trigger: [true, false, false, false, false, false], ..Move::NULL });
    let ev = run(&mut w, 60);
    let shots: Vec<i64> = ev.iter().filter(|(_, e)| matches!(e, Event::Fired { .. })).map(|(t, _)| *t).collect();
    // First shot once now > volley_delay_ms (1000), then every tick after > 150 ms.
    assert_eq!(shots, vec![1024, 1184, 1344, 1504, 1664]);
    assert_eq!(w.tank(id).unwrap().ammo, 140);
    // Release, press again: next burst once now > last + 1000.
    w.set_move(id, Move::NULL);
    run(&mut w, 1);
    w.set_move(id, Move { trigger: [true, false, false, false, false, false], ..Move::NULL });
    let start = w.time_ms;
    let ev = run(&mut w, 60);
    let first = ev.iter().find(|(_, e)| matches!(e, Event::Fired { .. })).map(|(t, _)| *t).unwrap();
    assert!(start < 2688 && first == 2688, "{start} {first}");
}

#[test]
fn projectile_spawns_from_muzzle_with_inherited_velocity() {
    let mut w = World::new(None);
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_projectile(id, default_projectile());
    w.tank_mut(id).unwrap().vel = Vec3::new(0.0, 5.0, 0.0);
    w.time_ms = 2000;
    w.set_move(id, Move { trigger: [true, false, false, false, false, false], ..Move::NULL });
    let ev = w.tick();
    let pid = ev.iter().find_map(|e| if let Event::Fired { projectile, .. } = e { Some(*projectile) } else { None }).unwrap();
    let p = w.projectile(pid).unwrap();
    let t = w.tank(id).unwrap();
    // Elevation = turret_pitch_center (2°) at pitchN 0, then one tick of flight.
    let fwd = Vec3::new(0.0, 2f32.to_radians().cos(), 2f32.to_radians().sin());
    let v0 = fwd * 85.0 + t.vel * 0.97;
    assert!((p.vel - (v0 - Vec3::new(0.0, 0.0, 0.4 * 0.32))).length() < 0.05, "{} {}", p.vel, v0);
    let muzzle = t.muzzle().translation;
    assert!((p.pos - (Vec3::from(muzzle) + v0 * DT)).length() < 0.05);
}

#[test]
fn ammo_runs_out_then_dry_fire() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let one = projectile("One", &[("rounds", "1"), ("reload_ms", "100"), ("volley_delay_ms", "100"), ("volley_size", "1"), ("can_be_kept", "1")]);
    w.set_projectile(id, one);
    let fire = Move { trigger: [true, false, false, false, false, false], ..Move::NULL };
    let mut fired = 0;
    let mut dry = 0;
    for i in 0..400 {
        w.set_move(id, if i % 2 == 0 { fire } else { Move::NULL });
        for e in w.tick() {
            match e {
                Event::Fired { .. } => fired += 1,
                Event::DryFired { .. } => dry += 1,
                _ => {}
            }
        }
    }
    // One real shot; then the default projectile with 0 ammo (players run dry).
    assert_eq!(fired, 1);
    assert!(dry > 0);
    let t = w.tank(id).unwrap();
    assert_eq!(t.ammo, 0);
    assert_eq!(t.current.as_ref().unwrap().name, "DefaultProjectile");
}

#[test]
fn bots_never_run_dry() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_ai(id, Some(Arc::new(TankAiParams::default())));
    w.set_move(id, Move { trigger: [true, false, false, false, false, false], ..Move::NULL });
    run(&mut w, 1);
    assert_eq!(w.tank(id).unwrap().ammo, 145);
}

#[test]
fn cartridge_swapping() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_projectile(id, default_projectile());
    w.tank_mut(id).unwrap().ammo = 100;
    w.set_projectile(id, speedy());
    {
        let t = w.tank(id).unwrap();
        assert_eq!(t.current.as_ref().unwrap().name, "SpeedyProjectile");
        assert_eq!(t.ammo, 35);
        assert_eq!(t.saved.as_ref().unwrap().name, "DefaultProjectile");
        assert_eq!(t.saved_ammo, 100);
    }
    // Splash over Speedy: Speedy cannot be saved, the default stays saved.
    w.set_projectile(id, splash());
    assert_eq!(w.tank(id).unwrap().saved_ammo, 100);
    // Reload pad (setProjectile(default)) with a special active refills the saved gun.
    w.set_projectile(id, default_projectile());
    {
        let t = w.tank(id).unwrap();
        assert_eq!(t.current.as_ref().unwrap().name, "SplashProjectile");
        assert_eq!(t.saved_ammo, 145);
    }
    // Special ammo runs out → the saved default comes back.
    w.tank_mut(id).unwrap().ammo = 0;
    run(&mut w, 1);
    let t = w.tank(id).unwrap();
    assert_eq!(t.current.as_ref().unwrap().name, "DefaultProjectile");
    assert_eq!(t.ammo, 145);
    assert!(t.saved.is_none());
}

#[test]
fn splash_damage_has_no_falloff_and_hits_shooter() {
    let mut w = flat_world();
    let a = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let b = w.add_tank(Arc::new(medium()), Vec3::new(5.5, 0.0, 0.0), 0.0);
    let far = w.add_tank(Arc::new(medium()), Vec3::new(30.0, 0.0, 0.0), 0.0);
    // Falling straight down beside both tanks.
    w.spawn_projectile(splash(), Vec3::new(2.5, -3.0, 3.0), Vec3::new(0.0, 0.0, -20.0), Some(a), 1.0);
    let ev = run(&mut w, 20);
    assert!(ev.iter().any(|(_, e)| matches!(e, Event::ProjectileExploded { hit: HitObject::Terrain, .. })));
    assert!((w.tank(a).unwrap().damage - 3.0).abs() < EPS);
    assert!((w.tank(b).unwrap().damage - 3.0).abs() < EPS);
    assert_eq!(w.tank(far).unwrap().damage, 0.0);
    assert_eq!(w.tank(b).unwrap().last_damage_source, Some(a));
}

#[test]
fn direct_hit_uses_damage_scale_and_armor() {
    let mut w = World::new(None);
    let mut armored = medium();
    armored.armor_scale = 1.8;
    let victim = w.add_tank(Arc::new(armored), Vec3::new(0.0, 20.0, 0.0), 0.0);
    w.spawn_projectile(default_projectile(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 85.0, 0.0), None, 2.0);
    run(&mut w, 10);
    assert!((w.tank(victim).unwrap().damage - 1.0 * 2.0 * 1.8).abs() < EPS);
}

#[test]
fn projectile_flight_is_explicit_euler() {
    let mut w = World::new(None);
    let id = w.spawn_projectile(default_projectile(), Vec3::ZERO, Vec3::new(10.0, 0.0, 5.0), None, 1.0);
    run(&mut w, 1);
    let p = w.projectile(id).unwrap();
    assert!((p.pos - Vec3::new(10.0, 0.0, 5.0) * DT).length() < 1e-5);
    assert!((p.vel.z - (5.0 - 0.4 * 0.32)).abs() < 1e-5);
    // Leaves the ±10000 bounds → deleted.
    let id = w.spawn_projectile(default_projectile(), Vec3::new(9999.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 0.0), None, 1.0);
    let ev = w.tick();
    assert!(ev.contains(&Event::ProjectileRemoved { projectile: id }));
}

#[test]
fn bounce_factor_and_bounce_count() {
    let mut w = flat_world();
    let bounce = projectile("BounceProjectile", &[("velocity", "35"), ("bounces", "8"), ("bounce_factor", "0.5"), ("blast_radius", "2"), ("gravity_scale", "1.7"), ("sprite_scale", "0.6")]);
    let id = w.spawn_projectile(bounce, Vec3::new(0.0, 0.0, 5.0), Vec3::new(3.0, 0.0, -10.0), None, 1.0);
    let mut bounces = 0;
    let mut checked = false;
    for _ in 0..2000 {
        let before = w.projectile(id).map(|p| p.vel);
        let ev = w.tick();
        for e in &ev {
            if let Event::ProjectileBounced { normal, .. } = e {
                bounces += 1;
                if !checked {
                    let v_in = before.unwrap() - Vec3::new(0.0, 0.0, 1.7 * 0.32);
                    let v_out = w.projectile(id).unwrap().vel;
                    assert!((normal.z - 1.0).abs() < EPS);
                    assert!((v_out.z + 0.5 * v_in.z).abs() < EPS, "{v_in} {v_out}");
                    assert!((v_out.x - v_in.x).abs() < EPS);
                    checked = true;
                }
            }
        }
        if ev.iter().any(|e| matches!(e, Event::ProjectileExploded { .. })) {
            break;
        }
    }
    assert_eq!(bounces, 8);
}

/// BounceProjectile shot level at 35 m/s from 1.5 m over a flat plane.
/// Reference: an f32 re-implementation of (end = pos + vel·dt; vel.z −=
/// 1.7·0.32; ray lowered by r = 0.3; on a hit pos = h − d̂·0.05 + r, vel.z ←
/// −0.5·vel.z; explode on the hit after 8 bounces).
#[test]
fn bounce_projectile_flight_matches_exe() {
    let mut w = flat_world();
    let bounce = projectile("BounceProjectile", &[("velocity", "35"), ("inherit_velocity", "0.93"), ("bounces", "8"), ("bounce_factor", "0.5"), ("blast_radius", "2"), ("gravity_scale", "1.7"), ("sprite_scale", "0.6")]);
    let id = w.spawn_projectile(bounce, Vec3::new(0.0, 0.0, 1.5), Vec3::new(35.0, 0.0, 0.0), None, 1.0);
    // (tick, hit x, pos x, pos z, vel z) after each bounce.
    let reference: [(usize, f32, f32, f32, f32); 8] = [
        (13, 13.7138, 13.66467, 0.30917, 3.53600),
        (28, 29.4233, 29.37366, 0.30579, 2.31200),
        (38, 40.0626, 40.01273, 0.30368, 1.56400),
        (45, 47.6149, 47.56498, 0.30243, 1.12200),
        (51, 53.3372, 53.2873, 0.30228, 1.07100),
        (56, 58.8734, 58.82339, 0.30158, 0.82450),
        (61, 63.3724, 63.32247, 0.30193, 0.94775),
        (66, 68.3380, 68.28803, 0.30175, 0.88613),
    ];
    let mut got = Vec::new();
    let mut apex = vec![1.5f32];
    let mut exploded = None;
    for tick in 1..=200 {
        let before = w.projectile(id).unwrap().pos;
        for e in w.tick() {
            match e {
                Event::ProjectileBounced { pos, .. } => {
                    let p = w.projectile(id).unwrap();
                    // InterpolateTick lerps from the pre-bounce position, not back
                    // along the reflected velocity.
                    assert_eq!(p.render_pos(0.0), before);
                    assert_eq!(p.render_pos(1.0), p.pos);
                    got.push((tick, pos.x, p.pos.x, p.pos.z, p.vel.z));
                    apex.push(p.pos.z);
                }
                Event::ProjectileExploded { pos, .. } => exploded = Some((tick, pos.x)),
                _ => {}
            }
        }
        if let Some(p) = w.projectile(id).filter(|p| p.is_live()) {
            let a = apex.last_mut().unwrap();
            *a = a.max(p.pos.z);
        }
        if exploded.is_some() {
            break;
        }
    }
    assert_eq!(got.len(), 8);
    for (g, e) in got.iter().zip(&reference) {
        assert_eq!(g.0, e.0, "{got:?}");
        for (a, b) in [(g.1, e.1), (g.2, e.2), (g.3, e.3), (g.4, e.4)] {
            assert!((a - b).abs() < 2e-3, "{g:?} vs {e:?}");
        }
    }
    let (tick, x) = exploded.unwrap();
    assert_eq!(tick, 71);
    assert!((x - 73.0592).abs() < 2e-3);
    // Peak centre height of each hop (the first is the launch height).
    let exe_apex = [1.5, 0.7357, 0.5016, 0.4016, 0.3579, 0.3534, 0.3369, 0.3452];
    for (a, b) in apex.iter().zip(exe_apex) {
        assert!((a - b).abs() < 2e-3, "{apex:?}");
    }
}

#[test]
fn slow_bounce_backs_off_by_unnormalised_step() {
    // |d|² ≤ 0.01: the 0.05 back-off uses the raw step.
    let mut w = flat_world();
    let p = projectile("P", &[("bounces", "8"), ("bounce_factor", "0.5"), ("gravity_scale", "0"), ("sprite_scale", "0.6")]);
    let id = w.spawn_projectile(p, Vec3::new(0.0, 0.0, 0.301), Vec3::new(0.0, 0.0, -2.0), None, 1.0);
    w.tick();
    let p = w.projectile(id).unwrap();
    // d = (0, 0, −0.064): pos.z = 0 + 0.05·0.064 + 0.3.
    assert!((p.pos.z - 0.3032).abs() < 1e-5, "{}", p.pos.z);
    assert!((p.vel.z - 1.0).abs() < 1e-5);
}

#[test]
fn proximity_fuse_explodes_on_bounce_near_tank() {
    let mut w = flat_world();
    let t = w.add_tank(Arc::new(medium()), Vec3::new(3.0, 0.0, 0.0), 0.0);
    let bounce = projectile("BounceProjectile", &[("bounces", "8"), ("bounce_factor", "0.5"), ("blast_radius", "2"), ("gravity_scale", "1.7"), ("damage", "1")]);
    // Lands ~3 m from the tank origin: 0.7·R + 2 ≈ 3.6 m → fuse.
    w.spawn_projectile(bounce, Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, -10.0), None, 1.0);
    let ev = run(&mut w, 10);
    assert!(ev.iter().any(|(_, e)| matches!(e, Event::ProjectileExploded { hit: HitObject::Terrain, .. })));
    assert!(!ev.iter().any(|(_, e)| matches!(e, Event::ProjectileBounced { .. })));
    assert!(w.tank(t).unwrap().damage > 0.0);
}

#[test]
fn boost_arrives_after_delay_spread_over_ticks() {
    // No terrain → always in contact, no gravity; a vertical kick shows the profile.
    let mut w = World::new(None);
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    apply_boost(w.tank_mut(id).unwrap(), Vec3::new(0.0, 0.0, 16.5));
    // A second boost while one runs is ignored.
    apply_boost(w.tank_mut(id).unwrap(), Vec3::new(0.0, 0.0, 100.0));
    let mut zs = Vec::new();
    for _ in 0..12 {
        w.tick();
        zs.push(w.tank(id).unwrap().vel.z);
    }
    let expect = [0.0, 0.0, 0.0, 0.0, 0.0, 3.3, 6.6, 9.9, 13.2, 16.5, 16.5, 16.5];
    for (z, e) in zs.iter().zip(expect) {
        assert!((z - e).abs() < EPS, "{zs:?}");
    }
}

fn jump_pad() -> Arc<BoostParams> {
    Arc::new(BoostParams::from_fields(&Fields::from_pairs("TestJump", "BoostDef", &[("kind", "additive"), ("num_states", "1"), ("velocity1", "16.5")])))
}

#[test]
fn pad_fires_once_per_entry() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let pad = w.add_boost("pad", jump_pad(), TriggerShape::default(), Affine3A::IDENTITY);
    let ev = run(&mut w, 1);
    let used: Vec<_> = ev.iter().filter_map(|(_, e)| if let Event::PadUsed { trigger, dv, .. } = e { Some((*trigger, *dv)) } else { None }).collect();
    assert_eq!(used, vec![(pad, Vec3::new(0.0, 0.0, 16.5))]);
    assert_eq!(w.tank(id).unwrap().boost_counter, 10);
    // Still overlapping a moment later (jump not yet applied): no second kick.
    let ev = run(&mut w, 3);
    assert!(!ev.iter().any(|(_, e)| matches!(e, Event::PadUsed { .. })));
    // The jump lifts it off.
    run(&mut w, 8);
    assert!(w.tank(id).unwrap().vel.z > 10.0);
}

#[test]
fn boost_override_modes() {
    let dir = Some(Affine3A::IDENTITY); // direction node: +Y
    let shape = TriggerShape { direction: dir, ..TriggerShape::default() };
    for (mode, expect) in [("additive", Vec3::new(0.0, 24.5, 0.0)), ("override", Vec3::new(-3.0, 24.5 - 4.0, 0.0)), ("overrideDir", Vec3::new(0.0, 24.5 - 4.0, 0.0))] {
        let mut w = World::new(None);
        let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
        w.tank_mut(id).unwrap().vel = Vec3::new(3.0, 4.0, 0.0);
        let data = BoostParams::from_fields(&Fields::from_pairs("B", "BoostDef", &[("kind", mode), ("num_states", "1"), ("velocity1", "24.5")]));
        w.add_boost("b", Arc::new(data), shape.clone(), Affine3A::IDENTITY);
        let ev = w.tick();
        let dv = ev.iter().find_map(|e| if let Event::PadUsed { dv, .. } = e { Some(*dv) } else { None }).unwrap();
        // The tank velocity at onEnter is after this tick's movement (anti-slide, coast).
        assert!(w.tank(id).unwrap().boost_counter > 0);
        assert!((dv - expect).length() < 0.5, "{mode}: {dv}");
    }
}

#[test]
fn boost_states_cycle_after_advance_delay() {
    let mut w = flat_world();
    w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let data = BoostParams::from_fields(&Fields::from_pairs(
        "TestBoost",
        "BoostDef",
        &[("advance_delay_on_enter", "200"), ("num_states", "4"), ("state_name1", "moe"), ("state_name2", "larry"), ("velocity1", "24.5"), ("velocity2", "24.5")],
    ));
    w.add_boost("b", Arc::new(data), TriggerShape::default(), Affine3A::IDENTITY);
    let ev = run(&mut w, 10);
    let changes: Vec<_> = ev.iter().filter_map(|(t, e)| if let Event::PadStateChanged { state, name, .. } = e { Some((*t, *state, name.clone())) } else { None }).collect();
    // Entered at t=0 → advance once now > 200 (t=224); minDuration 0 → stays.
    assert_eq!(changes, vec![(224, 1, "larry".to_string())]);
}

#[test]
fn power_up_pickup_and_respawn() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let data = PowerUpParams::from_fields(&Fields::from_pairs("TestPowerup", "PowerUpDef", &[("kind", "weapon"), ("min_off_ms", "1000"), ("max_off_ms", "1000")]));
    let pu = w.add_power_up("pup1", Arc::new(data), TriggerShape::default(), Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0)));
    let ev = run(&mut w, 1);
    assert!(ev.iter().any(|(_, e)| *e == Event::PowerUpEntered { trigger: pu, tank: id, def: "TestPowerup".into() }));
    assert!(ev.iter().any(|(_, e)| *e == Event::PowerUpSwitched { trigger: pu, on: false }));
    assert!(!w.trigger(pu).unwrap().on);
    let ev = run(&mut w, 40);
    let on_at = ev.iter().find(|(_, e)| *e == Event::PowerUpSwitched { trigger: pu, on: true }).map(|(t, _)| *t).unwrap();
    assert_eq!(on_at, 1024);
    // Still parked on it: no second pickup until the tank leaves and re-enters.
    assert!(!ev.iter().any(|(_, e)| matches!(e, Event::PowerUpEntered { .. })));
    w.tank_mut(id).unwrap().pos.x = 50.0;
    let ev = run(&mut w, 1);
    assert!(ev.iter().any(|(_, e)| matches!(e, Event::PowerUpLeft { .. })));
    w.tank_mut(id).unwrap().pos.x = 0.0;
    let ev = run(&mut w, 1);
    assert!(ev.iter().any(|(_, e)| matches!(e, Event::PowerUpEntered { .. })));
}

#[test]
fn power_up_start_off_and_never_off() {
    let mut w = flat_world();
    let off = PowerUpParams::from_fields(&Fields::from_pairs("Heal", "PowerUpDef", &[("starts_on", "0"), ("min_off_ms", "500"), ("max_off_ms", "500")]));
    let a = w.add_power_up("a", Arc::new(off), TriggerShape::default(), Affine3A::from_translation(Vec3::new(100.0, 0.0, 0.0)));
    let reload = PowerUpParams::from_fields(&Fields::from_pairs("Reload", "PowerUpDef", &[("kind", "ammo")]));
    let b = w.add_power_up("b", Arc::new(reload), TriggerShape::default(), Affine3A::IDENTITY);
    assert!(!w.trigger(a).unwrap().on);
    assert!(w.trigger(b).unwrap().on);
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    run(&mut w, 20);
    assert!(w.trigger(a).unwrap().on);
    // max_off_ms 0: stays on after a pickup.
    assert!(w.trigger(b).unwrap().on);
    assert_eq!(w.tank(id).unwrap().current_trigger, Some(b));
}

#[test]
fn trigger_radius_compares_squared_distance_with_radius() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let r = w.tank(id).unwrap().data.geometry.big_radius();
    let data = Arc::new(PowerUpParams::from_fields(&Fields::from_pairs("P", "PowerUpDef", &[])));
    // Box edge at distance d from the tank origin: overlaps iff d² < R.
    let d_in = r.sqrt() - 0.05;
    let d_out = r.sqrt() + 0.05;
    let tin = w.add_power_up("in", data.clone(), TriggerShape::default(), Affine3A::from_translation(Vec3::new(1.0 + d_in, 0.0, 0.0)));
    let tout = w.add_power_up("out", data, TriggerShape::default(), Affine3A::from_translation(Vec3::new(-1.0 - d_out, 0.0, 0.0)));
    let t = w.tank(id).unwrap().clone();
    assert!(w.trigger(tin).unwrap().overlaps(t.pos, r));
    assert!(!w.trigger(tout).unwrap().overlaps(t.pos, r));
}

#[test]
fn death_corpse_and_removal() {
    let mut w = flat_world();
    let killer = w.add_tank(Arc::new(medium()), Vec3::new(50.0, 0.0, 0.0), 0.0);
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.apply_damage(id, 20.0, Some(killer));
    assert_eq!(w.tank(id).unwrap().damage, 14.0);
    let ev = run(&mut w, 1);
    assert!(ev.iter().any(|(_, e)| *e == Event::Killed { tank: id, killer: Some(killer) }));
    assert!(w.tank(id).unwrap().is_corpse);
    // Deleted once corpseTimer > corpse_fade_time + corpse_fade_speed (5 s).
    let ev = run(&mut w, 200);
    let removed = ev.iter().find(|(_, e)| *e == Event::TankRemoved { tank: id }).map(|(t, _)| *t).unwrap();
    // Death tick at t=0 already adds 0.032 s: 157 increments → t = 156·32.
    assert_eq!(removed, 32 * 156);
    assert!(w.tank(id).is_none());
}

#[test]
fn heal_and_out_of_bounds() {
    let mut w = flat_world();
    w.mission_area = Some(MissionArea { x: -100.0, y: -100.0, w: 200.0, h: 200.0 });
    let id = w.add_tank(Arc::new(medium()), Vec3::new(104.0, 0.0, 0.0), 0.0);
    run(&mut w, 1);
    assert_eq!(w.tank(id).unwrap().damage, 0.0);
    w.tank_mut(id).unwrap().pos.x = 106.0;
    run(&mut w, 10);
    assert!((w.tank(id).unwrap().damage - 1.0).abs() < EPS);
    w.hurt_me(id, "-100.7");
    assert_eq!(w.tank(id).unwrap().damage, 0.0);
    w.hurt_me(id, "2.9");
    assert_eq!(w.tank(id).unwrap().damage, 2.0);
}

#[test]
fn below_mission_floor_kills() {
    // MissionArea.floor 95 on ground at 0 (a tank in the fogged pit, well
    // inside the area): 0.1 per tick, 14 → dead on tick 141 (4.5 s).
    let mut w = flat_world();
    w.mission_area = Some(MissionArea { x: -100.0, y: -100.0, w: 200.0, h: 200.0 });
    w.floor = 95.0;
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let ev = run(&mut w, 150);
    let died = ev.iter().position(|(_, e)| matches!(e, Event::Killed { tank, .. } if *tank == id));
    assert_eq!(died.map(|i| ev[i].0 / 32), Some(140));
}

#[test]
fn repair_rate_heals_per_second() {
    let mut data = medium();
    data.misc.repair_rate = 0.5;
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(data), Vec3::ZERO, 0.0);
    w.apply_damage(id, 1.0, None);
    run(&mut w, 10);
    assert!((w.tank(id).unwrap().damage - (1.0 - 0.5 * 0.32)).abs() < EPS);
}

#[test]
fn turret_yaw_clamp_and_auto_turn() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_move(id, Move { yaw: 1.0, ..Move::NULL });
    run(&mut w, 1);
    assert!((w.tank(id).unwrap().yaw_n - 1.0 / std::f32::consts::PI).abs() < EPS);
    run(&mut w, 5);
    assert_eq!(w.tank(id).unwrap().yaw_n, 1.0);
    let mut mv = Move::NULL;
    tank::apply_auto_turn(&mut mv, w.tank(id).unwrap());
    assert_eq!(mv.x, 1.0);
    // Pitch: +down, elevation = verCenter − verRange·pitchN.
    w.set_move(id, Move { pitch: -0.5 * std::f32::consts::PI, ..Move::NULL });
    run(&mut w, 1);
    let t = w.tank(id).unwrap();
    assert!((t.pitch_n + 0.5).abs() < EPS);
    assert!((t.data.turret_pitch_center - t.data.turret_pitch_range * t.pitch_n - 12.5).abs() < EPS);
    let fwd: Vec3 = t.muzzle().matrix3.y_axis.into();
    assert!((fwd.z.asin().to_degrees() - 12.5).abs() < 0.01);
}

#[test]
fn auto_level_keeps_world_elevation() {
    // Body pitches nose-up by α in one tick; auto-level 1 removes α from the barrel.
    let mut data = medium();
    data.turret_auto_level = 1.0;
    let data = Arc::new(data);
    let mut t = Tank::new(TankId(1), data.clone(), Vec3::ZERO, Mat3::IDENTITY);
    let old = t.rot;
    let alpha = 3f32.to_radians();
    t.rot = Mat3::from_rotation_x(alpha);
    tank::update_turret(&mut t, &Move::NULL, old);
    let world_elev = (t.rot * Mat3::from(data.geometry.muzzle(t.yaw_n, t.pitch_n, &data).matrix3).y_axis).z.asin().to_degrees();
    assert!((world_elev - 2.0).abs() < 0.05, "{world_elev}");
}

#[test]
fn unstuck_kick_after_unstuck_time() {
    // Tank resting on a big box high above the terrain: no wheel contact, ~0 velocity.
    let mut w = flat_world();
    let rock = StaticShape::from_box(Aabb::new(Vec3::new(-20.0, -20.0, 0.0), Vec3::new(20.0, 20.0, 50.0)));
    w.add_static(Arc::new(rock), Affine3A::IDENTITY);
    let id = w.add_tank_at(Arc::new(medium()), Vec3::new(0.0, 0.0, 50.5), Mat3::IDENTITY);
    let mut kicked = false;
    for _ in 0..200 {
        let before = w.tank(id).unwrap().stuck_ticks;
        w.tick();
        let t = w.tank(id).unwrap();
        if before > 0 && t.stuck_ticks == 0 && t.vel.length() > 1.0 {
            kicked = true;
            assert!(before as i64 * 32 >= 1000);
            break;
        }
    }
    assert!(kicked);
}

#[test]
fn volcano_erupts_rocks_toward_zones() {
    let mut w = flat_world();
    let rock = projectile("VolcanoRock", &[("velocity", "63"), ("inherit_velocity", "0"), ("bounces", "5"), ("bounce_factor", "0.3"), ("blast_radius", "5"), ("damage", "3"), ("gravity_scale", "0.75")]);
    let mut v = VolcanoParams::from_fields(
        &Fields::from_pairs(
            "VolcanoDB",
            "VolcanoDef",
            &[("num_zones", "2"), ("min_attack_delay", "100"), ("max_attack_delay", "200"), ("min_attack_rocks", "3"), ("max_attack_rocks", "6"), ("zone_name1", "target1"), ("zone_weight1", "1"), ("zone_focus_weight1", "4"), ("zone_radius1", "5"), ("zone_name2", "target2"), ("zone_weight2", "1"), ("zone_focus_weight2", "4"), ("zone_radius2", "5")],
        ),
        None,
    );
    v.projectile = Some(rock);
    w.add_marker("target1", Aabb::new(Vec3::new(80.0, 0.0, 0.0), Vec3::new(82.0, 2.0, 1.0)));
    w.add_marker("target2", Aabb::new(Vec3::new(-80.0, 0.0, 0.0), Vec3::new(-78.0, 2.0, 1.0)));
    let shape = TriggerShape { bounds: Aabb::new(Vec3::new(-10.0, -10.0, 0.0), Vec3::new(10.0, 10.0, 25.0)), ..TriggerShape::default() };
    let vid = w.add_volcano("volcano", Arc::new(v), shape, Affine3A::IDENTITY);
    let mut erupted = 0;
    let mut landed = Vec::new();
    for _ in 0..800 {
        for e in w.tick() {
            match e {
                Event::VolcanoErupted { trigger, rocks } => {
                    assert_eq!(trigger, vid);
                    erupted += rocks;
                }
                Event::ProjectileExploded { pos, .. } | Event::ProjectileBounced { pos, .. } if landed.len() < 1000 => landed.push(pos),
                _ => {}
            }
        }
    }
    assert!(erupted >= 3, "{erupted}");
    // Rocks come down in the zone boxes grown by the radius (first impact).
    let near = landed.iter().filter(|p| (p.x.abs() - 81.0).abs() < 8.0 && p.y.abs() < 9.0).count();
    assert!(near > 0, "{landed:?}");
}

#[test]
fn volcano_zone_weights() {
    let v = VolcanoParams::from_fields(
        &Fields::from_pairs("V", "VolcanoDef", &[("num_zones", "2"), ("zone_weight1", "1"), ("zone_focus_weight1", "4"), ("zone_weight2", "1"), ("zone_focus_weight2", "4")]),
        None,
    );
    let st = pads::VolcanoState { data: Arc::new(v), rocks_left: 0, next_attack: 0, focus_zone: 0 };
    let mut rng = rng::Rng::new(9);
    let mut counts = [0; 3];
    for _ in 0..60000 {
        counts[st.pick_zone(&mut rng).unwrap_or(2)] += 1;
    }
    // total 5, r = rand % 6: 4/6 focus, 1/6 other, 1/6 no pick.
    let f = |c: i32| c as f32 / 60000.0;
    assert!((f(counts[0]) - 4.0 / 6.0).abs() < 0.01 && (f(counts[1]) - 1.0 / 6.0).abs() < 0.01 && (f(counts[2]) - 1.0 / 6.0).abs() < 0.01, "{counts:?}");
}

// ------------------------------------------------------------------ bot AI

fn bot_world() -> (World, TankId, TankId) {
    let mut w = flat_world();
    let a = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let b = w.add_tank(Arc::new(medium()), Vec3::new(0.0, 80.0, 0.0), 0.0);
    for (id, team) in [(a, 1), (b, 2)] {
        w.set_team(id, team);
        w.set_projectile(id, default_projectile());
    }
    (w, a, b)
}

fn ai_data(pairs: &[(&str, &str)]) -> Arc<TankAiParams> {
    Arc::new(TankAiParams::from_fields(&Fields::from_pairs("TestAI", "TankAiDef", pairs)))
}

#[test]
fn set_ai_initial_state() {
    let (mut w, a, _) = bot_world();
    let mut expect = rng::Rng::default();
    w.set_ai(a, Some(ai_data(&[])));
    let bot = w.tank(a).unwrap().ai.as_ref().unwrap().clone();
    assert_eq!((bot.mode, bot.tactic, bot.los_countdown, bot.clear_shot), (ai::Mode::RoamReset, ai::Tactic::Choose, 15, true));
    assert_eq!(bot.wobble, [expect.rand_f(), expect.rand_f()]);
    assert_eq!(bot.home, w.tank(a).unwrap().pos);
    // Same def again: no reset.
    w.tank_mut(a).unwrap().ai.as_mut().unwrap().mode = ai::Mode::Flee;
    w.set_ai(a, Some(ai_data(&[])));
    assert_eq!(w.tank(a).unwrap().ai.as_ref().unwrap().mode, ai::Mode::Flee);
    w.set_ai(a, None);
    assert!(!w.tank(a).unwrap().is_bot());
}

#[test]
fn target_scoring_ranges_teams_oc_and_revenge() {
    let (mut w, a, b) = bot_world();
    let c = w.add_tank(Arc::new(medium()), Vec3::new(60.0, 0.0, 0.0), 0.0);
    w.set_team(c, 2);
    w.set_ai(a, Some(ai_data(&[("target_range", "100"), ("disengage_range", "200"), ("oc_weight", "2"), ("revenge_weight", "1")])));
    let t = w.tank(a).unwrap().clone();
    // Nearest enemy wins: score sqrt(200² − d²).
    assert_eq!(w.pick_target(&t), Some(ai::BotTarget::Tank(c)));
    // The current target keeps its lead with oc_weight.
    let mut t2 = t.clone();
    t2.ai.as_mut().unwrap().target = Some(ai::BotTarget::Tank(b));
    assert_eq!(w.pick_target(&t2), Some(ai::BotTarget::Tank(b)));
    // Revenge: my last attacker with a streak.
    let mut t3 = t.clone();
    t3.last_damage_source = Some(b);
    t3.ai.as_mut().unwrap().streak = 1.0;
    assert_eq!(w.pick_target(&t3), Some(ai::BotTarget::Tank(b)));
    // Teammates and corpses are skipped; beyond target_range only the current target counts.
    w.set_team(c, 1);
    w.tank_mut(b).unwrap().pos.y = 150.0;
    let t = w.tank(a).unwrap().clone();
    assert_eq!(w.pick_target(&t), None);
    // Free-for-all: team 0 tanks are enemies even to each other.
    w.set_team(a, 0);
    w.set_team(c, 0);
    let t = w.tank(a).unwrap().clone();
    assert_eq!(w.pick_target(&t), Some(ai::BotTarget::Tank(c)));
    w.team_game = true;
    assert_eq!(w.pick_target(&t), None);
}

#[test]
fn random_spawn_point_partitions_like_the_engine() {
    let (mut w, a, _) = bot_world();
    w.set_ai(a, Some(ai_data(&[("home_range", "100")])));
    w.spawn_spheres = vec![Vec3::new(500.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 0.0), Vec3::new(900.0, 0.0, 0.0)];
    let t = w.tank(a).unwrap().clone();
    let mut expect = w.rng.clone();
    let p = w.random_spawn_point(&t);
    // i=0 far: swapped with the last → [900, 10, 20, 500]; i=1, i=2 near; i=3 far (swap with itself).
    assert_eq!(w.spawn_spheres, vec![Vec3::new(900.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 0.0), Vec3::new(500.0, 0.0, 0.0)]);
    let k = (expect.rand_i() >> 16) as usize % 2;
    assert_eq!(p, w.spawn_spheres[k]);
    // Nothing in range: home.
    w.spawn_spheres = vec![Vec3::new(5000.0, 0.0, 0.0)];
    assert_eq!(w.random_spawn_point(&t), t.ai.as_ref().unwrap().home);
}

#[test]
fn steering_avoids_statics_and_seeks_pickups() {
    let mut w = flat_world();
    let a = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_ai(a, Some(ai_data(&[])));
    let t = w.tank(a).unwrap().clone();
    let dest = Vec3::new(0.0, 100.0, 0.0);
    assert!((w.steer(&t, dest, false) - Vec3::Y).length() < 1e-5);
    // A rock just to the right of the path pushes the steer left.
    let rock = StaticShape::from_box(Aabb::new(Vec3::new(-2.0, -2.0, 0.0), Vec3::new(2.0, 2.0, 4.0)));
    w.add_static(Arc::new(rock), Affine3A::from_translation(Vec3::new(6.0, 8.0, 0.0)));
    let s = w.steer(&t, dest, false);
    assert!(s.x < -0.05, "{s}");
    // Direct steering ignores obstacles.
    assert!((w.steer(&t, dest, true) - Vec3::Y).length() < 1e-5);
    // An ai_pickup power-up within 10 m takes over.
    let pu = PowerUpParams::from_fields(&Fields::from_pairs("P", "PowerUpDef", &[("ai_pickup", "1")]));
    w.add_power_up("p", Arc::new(pu), TriggerShape::default(), Affine3A::from_translation(Vec3::new(-5.0, 0.0, 0.0)));
    // (The rock's repulsion is still added to the new desired direction.)
    let s = w.steer(&t, dest, false);
    assert!(s.x < -0.95, "{s}");
}

#[test]
fn drive_mapping_and_unstuck() {
    let (mut w, a, _) = bot_world();
    w.set_ai(a, Some(ai_data(&[("target_range", "1"), ("disengage_range", "1")])));
    // Roaming towards a point behind-right: hard turn, throttle on.
    w.spawn_spheres = vec![Vec3::new(30.0, -60.0, 0.0)];
    let mut t = w.tank(a).unwrap().clone();
    let mv = w.bot_move(&mut t);
    assert_eq!((mv.x, mv.y), (1.0, 1.0));
    // Destination only 3 m ahead (but 15 m to the side): turn, no throttle.
    t.ai.as_mut().unwrap().dest = t.pos + Vec3::new(15.0, 3.0, 0.0);
    t.ai.as_mut().unwrap().mode = ai::Mode::Roam;
    let mv = w.bot_move(&mut t);
    assert_eq!((mv.x, mv.y), (1.0, 0.0));
    // Bumped: reverse for 30 ticks.
    t.ai.as_mut().unwrap().bumped = true;
    let mv = w.bot_move(&mut t);
    assert_eq!(mv.y, -1.0);
    assert_eq!(t.ai.as_ref().unwrap().unstuck, 29);
    assert!(t.ai.as_ref().unwrap().stuck > 0.98);
}

#[test]
fn bots_engage_and_shoot_each_other() {
    let (mut w, a, b) = bot_world();
    let data = ai_data(&[("aim", "4"), ("aim_speed", "0.5"), ("trigger_happy", "2"), ("target_range", "500"), ("disengage_range", "500")]);
    w.set_ai(a, Some(data.clone()));
    w.set_ai(b, Some(data));
    let mut engaged = false;
    let mut ev = Vec::new();
    for _ in 0..600 {
        ev.extend(run(&mut w, 1));
        engaged |= w.tank(a).is_some_and(|t| t.ai.as_ref().unwrap().target == Some(ai::BotTarget::Tank(b)));
    }
    let fired = ev.iter().filter(|(_, e)| matches!(e, Event::Fired { .. })).count();
    let hits = ev.iter().filter(|(_, e)| matches!(e, Event::Damaged { .. })).count();
    assert!(engaged);
    assert!(fired > 5 && hits > 0, "fired {fired} hits {hits}");
}

#[test]
fn bot_turret_tracks_target() {
    let (mut w, a, b) = bot_world();
    // Target off to the right: within a few seconds the barrel (hull turn
    // plus turret yaw) points at it.
    w.tank_mut(b).unwrap().pos = Vec3::new(40.0, 60.0, 0.0);
    w.set_ai(a, Some(ai_data(&[("aim", "0"), ("aim_speed", "0.5")])));
    run(&mut w, 60);
    let t = w.tank(a).unwrap();
    let barrel: Vec3 = t.muzzle().matrix3.y_axis.into();
    let to_b = (w.tank(b).unwrap().center() - Vec3::from(t.muzzle().translation)).normalize();
    assert!(barrel.truncate().normalize().dot(to_b.truncate().normalize()) > 0.98, "{barrel} {to_b}");
}

// ------------------------------------------------------------- scrum ball

fn ball() -> Arc<FlagParams> {
    Arc::new(FlagParams::from_fields(&Fields::from_pairs(
        "DefaultFlag",
        "FlagDef",
        &[("bounce_terrain", "0.7"), ("bounce_static", "0.5"), ("friction_terrain", "0.4"), ("unstuck_time", "2500"), ("unstuck_boost", "10"), ("no_pickup", "0")],
    )))
}

#[test]
fn bots_chase_the_scrum_ball_but_not_a_practice_ball() {
    let (mut w, a, b) = bot_world();
    w.set_ai(a, Some(ai_data(&[("target_range", "100"), ("disengage_range", "200")])));
    let t = w.tank(a).unwrap().clone();
    let f = w.add_flag(ball(), Vec3::new(30.0, 0.0, 0.0));
    assert_eq!(w.pick_target(&t), Some(ai::BotTarget::Flag(f)));
    w.flags.clear();
    let practice = Arc::new(FlagParams::from_fields(&Fields::from_pairs("PracticeBallData", "FlagDef", &[("practice", "1")])));
    assert!(practice.practice);
    w.add_flag(practice, Vec3::new(30.0, 0.0, 0.0));
    assert_eq!(w.pick_target(&t), Some(ai::BotTarget::Tank(b)), "a practice ball: the battle target choice");
}

#[test]
fn ball_falls_bounces_and_rests() {
    let mut w = flat_world();
    let f = w.add_flag(ball(), Vec3::new(50.0, 0.0, 10.0));
    let mut bounced = false;
    let mut rested = false;
    let mut prev = 0.0;
    for _ in 0..400 {
        w.tick();
        let fl = w.flag(f).unwrap();
        if prev < 0.0 && fl.vel.z > 0.0 {
            // vel = (v − vn·N)(1 − friction) − bounce·vn·N
            assert!((fl.vel.z + 0.7 * (prev - 0.32)).abs() < 0.01, "{} {prev}", fl.vel.z);
            bounced = true;
        }
        prev = fl.vel.z;
        rested |= fl.at_rest;
        assert!(fl.pos.z >= -1e-4);
    }
    assert!(bounced && rested);
}

#[test]
fn ball_unstuck_kick_after_rest() {
    let mut w = flat_world();
    let f = w.add_flag(ball(), Vec3::new(50.0, 0.0, 0.0));
    let mut kicked_at = None;
    for i in 0..200 {
        w.tick();
        if kicked_at.is_none() && w.flag(f).unwrap().vel.length() > 5.0 {
            kicked_at = Some(i);
        }
    }
    // At rest from the first tick; the kick comes once restTicks·32 > 2500 (79 ticks).
    let k = kicked_at.unwrap();
    assert!((78..=82).contains(&k), "{k}");
}

#[test]
fn ball_pickup_carry_drop_and_retake_lockout() {
    let (mut w, a, b) = bot_world();
    let f = w.add_flag(ball(), Vec3::new(0.0, 2.0, 0.0));
    let ev = run(&mut w, 1);
    assert!(ev.iter().any(|(_, e)| *e == Event::FlagPickedUp { flag: f, tank: a }));
    assert_eq!(w.get_flag(a), Some(f));
    run(&mut w, 1);
    let carried = w.flag(f).unwrap().pos;
    let ta = w.tank(a).unwrap();
    assert!((carried - (ta.pos + Vec3::new(0.0, 0.0, 3.0))).length() < 0.01, "{carried}");
    // B touching a carried ball does nothing; mounting a held ball on B (as a
    // script would) bars A from retaking it for 30 ticks.
    w.tank_mut(b).unwrap().pos = Vec3::new(0.0, 6.0, 0.0);
    run(&mut w, 1);
    assert_eq!(w.get_flag(a), Some(f));
    w.mount_flag(f, b);
    assert_eq!(w.get_flag(a), None);
    assert_eq!(w.flag(f).unwrap().no_retake, Some(a));
    assert_eq!(w.flag(f).unwrap().lockout, 30);
    // Carrier dies: the ball drops with the carrier's velocity.
    w.apply_damage(b, 100.0, Some(a));
    let ev = run(&mut w, 1);
    assert!(ev.iter().any(|(_, e)| *e == Event::FlagDropped { tank: b, flag: f }));
    assert!(w.flag(f).unwrap().carrier.is_none());
    // Script clearMount on a carried ball.
    w.tank_mut(b).unwrap().pos.x = 200.0;
    w.set_flag_position(f, w.tank(a).unwrap().pos + Vec3::new(0.0, 1.0, 0.5));
    w.flag_mut(f).unwrap().no_retake = None;
    run(&mut w, 2);
    assert_eq!(w.get_flag(a), Some(f));
    w.clear_mount(f);
    assert_eq!(w.get_flag(a), None);
}

#[test]
fn ball_push_when_no_pickup() {
    let mut w = flat_world();
    let a = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    let mut d = (*ball()).clone();
    d.no_pickup = true;
    let f = w.add_flag(Arc::new(d), Vec3::new(0.0, 3.0, 0.0));
    w.tank_mut(a).unwrap().vel = Vec3::new(0.0, 5.0, 0.0);
    run(&mut w, 1);
    let fl = w.flag(f).unwrap();
    assert!(fl.carrier.is_none());
    // kick = dir·(dir·v·1.5), kick.z = 0.1·(|kx| + |ky|)
    assert!(fl.vel.y > 5.0 && fl.vel.z > 0.0, "{}", fl.vel);
}

#[test]
fn hard_tank_hit_knocks_the_ball_loose() {
    let mut data = medium();
    data.coast_decel_rate = 0.0;
    let data = Arc::new(data);
    let mut w = World::new(None);
    let a = w.add_tank(data.clone(), Vec3::ZERO, 0.0);
    let b = w.add_tank(data, Vec3::new(0.0, 4.2, 0.0), 0.0);
    let f = w.add_flag(ball(), Vec3::new(0.0, 4.2, 0.0));
    run(&mut w, 1);
    assert_eq!(w.get_flag(b), Some(f));
    w.tank_mut(a).unwrap().vel = Vec3::new(0.0, 25.0, 0.0);
    let ev = run(&mut w, 2);
    assert!(ev.iter().any(|(_, e)| *e == Event::FlagDropped { tank: b, flag: f }), "{ev:?}");
}

/// The release compares the impulse, (1 + bounce_tank)/2 = 0.875 of the
/// closing speed, with hardTankCollVel (10): a full-speed ram (11.9 m/s,
/// impulse 10.4) knocks the ball loose, 11 m/s (9.6) does not.
#[test]
fn ram_knocks_the_ball_loose_by_impulse() {
    for (speed, drops) in [(11.0, false), (11.9, true)] {
        let mut data = medium();
        data.coast_decel_rate = 0.0;
        let data = Arc::new(data);
        let mut w = World::new(None);
        let a = w.add_tank(data.clone(), Vec3::ZERO, 0.0);
        let b = w.add_tank(data, Vec3::new(0.0, 4.2, 0.0), 0.0);
        let f = w.add_flag(ball(), Vec3::new(0.0, 4.2, 0.0));
        run(&mut w, 1);
        assert_eq!(w.get_flag(b), Some(f));
        w.tank_mut(a).unwrap().vel = Vec3::new(0.0, speed, 0.0);
        let ev = run(&mut w, 4);
        assert!(ev.iter().any(|(_, e)| matches!(e, Event::Collided { tank, .. } if *tank == a)), "{speed}: no hit {ev:?}");
        assert_eq!(w.get_flag(b).is_none(), drops, "{speed} m/s");
    }
}

/// The game's only pass: a hard ram knocks the ball loose and mount(NULL)
/// leaves it the carrier's velocity from its last tick, not the bounce.
/// Rammed, the carrier flies off and leaves the ball to the rammer;
/// ramming, it stops and the ball flies on over the tank it hit.
#[test]
fn a_hard_ram_passes_the_ball() {
    for carrier_rams in [false, true] {
        let data = Arc::new(medium());
        let mut w = flat_world();
        let a = w.add_tank(data.clone(), Vec3::ZERO, 0.0);
        let b = w.add_tank(data, Vec3::new(0.0, 4.5, 0.0), 0.0);
        let (carrier, other) = if carrier_rams { (a, b) } else { (b, a) };
        let f = w.add_flag(ball(), w.tank(carrier).unwrap().pos);
        run(&mut w, 1);
        assert_eq!(w.get_flag(carrier), Some(f));
        w.tank_mut(a).unwrap().vel = Vec3::new(0.0, 11.9, 0.0);
        w.set_move(a, Move { y: 1.0, ..Move::NULL });
        let mut before = Vec3::ZERO;
        while w.get_flag(carrier).is_some() {
            before = w.tank(carrier).unwrap().vel;
            run(&mut w, 1);
            assert!(w.time_ms < 300, "no release");
        }
        // A loose ball keeps the carrier's pre-ram velocity (the other tank may take it at once).
        let fl = w.flag(f).unwrap();
        assert!(fl.carrier.is_some() || (fl.vel.y - before.y).abs() < 0.01, "{before} {}", fl.vel);
        run(&mut w, 30);
        assert_eq!(w.get_flag(other), Some(f), "carrier rams: {carrier_rams}");
    }
}

// ---------------------------------------------------------------- brains

#[test]
fn brain_ejects_after_corpse_shake_time() {
    let mut data = medium();
    data.fx.corpse_shake_time = 0.8;
    data.fx.brain_projectile = Some(projectile("brainProjectile", &[("velocity", "21"), ("inherit_velocity", "1"), ("bounces", "8"), ("bounce_factor", "0.5"), ("damage", "0"), ("gravity_scale", "0"), ("sprite_scale", "0.2")]));
    data.brain_shape = "models/tanks/brainjar02.dts".into();
    data.death_sequences = vec![0.5333];
    let mut w = World::new(None);
    w.auto_eject_brains = true;
    let id = w.add_tank(Arc::new(data), Vec3::ZERO, 0.0);
    w.apply_damage(id, 100.0, None);
    let ev = run(&mut w, 60);
    let (t, pid) = ev
        .iter()
        .find_map(|(t, e)| if let Event::BrainEjected { projectile, shape, .. } = e { assert!(shape.ends_with("brainjar02.dts")); Some((*t, *projectile)) } else { None })
        .unwrap();
    // corpseTimer ≥ 0.8 s first after 25 increments (t = 24·32): before the flip.
    assert_eq!(t, 24 * 32);
    let p = w.projectile(pid.unwrap()).unwrap();
    assert!((p.vel - Vec3::new(0.0, 0.0, 21.0)).length() < 1e-3, "{}", p.vel);
    assert_eq!(ev.iter().filter(|(_, e)| matches!(e, Event::BrainEjected { .. })).count(), 1);
    // The jar never explodes and is removed 5 s after it was ejected.
    let pid = pid.unwrap();
    let ev = run(&mut w, 200);
    assert!(!ev.iter().any(|(_, e)| matches!(e, Event::ProjectileExploded { projectile, .. } if *projectile == pid)));
    let removed = ev.iter().find(|(_, e)| *e == Event::ProjectileRemoved { projectile: pid }).map(|(t, _)| *t).unwrap();
    assert_eq!(removed, ((t + 5000) / 32 + 1) * 32, "first tick after spawn + 5000 ms");
}

#[test]
fn brain_jar_bounces_off_the_ground_without_exploding() {
    let mut w = flat_world();
    let bp = projectile("brainProjectile", &[("velocity", "21"), ("bounces", "8"), ("bounce_factor", "0.5"), ("gravity_scale", "0"), ("damage", "0")]);
    let id = w.spawn_projectile(bp, Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, -21.0), None, 0.0);
    w.projectiles[0].cosmetic_since = Some(w.time_ms);
    let ev = run(&mut w, 100);
    assert!(ev.iter().filter(|(_, e)| matches!(e, Event::ProjectileBounced { .. })).count() >= 1);
    assert!(!ev.iter().any(|(_, e)| matches!(e, Event::ProjectileExploded { .. })));
    assert!(w.projectile(id).unwrap().vel.z > 0.0);
}

#[test]
fn no_brain_without_death_sequences() {
    let mut data = medium();
    data.fx.brain_projectile = Some(projectile("brainProjectile", &[("velocity", "21"), ("gravity_scale", "0")]));
    let mut w = World::new(None);
    w.auto_eject_brains = true;
    let id = w.add_tank(Arc::new(data), Vec3::ZERO, 0.0);
    w.set_controlling_client(id, true);
    w.apply_damage(id, 100.0, None);
    let ev = run(&mut w, 400);
    assert!(!ev.iter().any(|(_, e)| matches!(e, Event::BrainEjected { .. })));
    // A player's corpse stays until respawn (scripts delete it).
    assert!(w.is_corpse(id));
    w.remove_tank(id);
    run(&mut w, 1);
    assert!(w.tank(id).is_none());
}

#[test]
fn tanks_spawned_on_the_same_point_separate() {
    let mut w = flat_world();
    let a = w.add_tank(Arc::new(medium()), Vec3::new(10.0, 10.0, 0.0), 0.0);
    let b = w.add_tank(Arc::new(medium()), Vec3::new(10.0, 10.0, 0.0), 0.0);
    w.set_move(a, Move { y: 1.0, ..Move::NULL });
    w.set_move(b, Move { y: 1.0, ..Move::NULL });
    run(&mut w, 30);
    let (pa, pb) = (w.tank(a).unwrap().pos, w.tank(b).unwrap().pos);
    let r = 2.0 * w.tank(a).unwrap().data.geometry.small_radius();
    assert!((pa - pb).length() >= r - 0.02, "{pa} {pb}");
    // ... and then drive normally.
    let y0 = pa.y;
    let ev = run(&mut w, 30);
    assert!(w.tank(a).unwrap().pos.y > y0 + 5.0);
    assert!(!ev.iter().any(|(_, e)| matches!(e, Event::Collided { .. })));
    let mut w = flat_world();
    w.separate_tanks = false;
    let a = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.add_tank(Arc::new(medium()), Vec3::new(0.5, 0.0, 0.0), 0.0);
    w.set_move(a, Move { y: 1.0, ..Move::NULL });
    run(&mut w, 60);
    assert!(w.tank(a).unwrap().pos.y.abs() < 0.01);
}

#[test]
fn splash_direct_hit_deals_damage_once_by_box_overlap() {
    // Server splash: axis-aligned cube of half-size blast_radius around the
    // impact against tank world boxes; no extra direct-hit damage.
    let mut w = World::new(None);
    let victim = w.add_tank(Arc::new(medium()), Vec3::new(0.0, 20.0, 0.0), 0.0);
    // Box-corner case: within the cube but farther than blast_radius from the
    // impact point (a sphere test would miss it).
    let corner = w.add_tank(Arc::new(medium()), Vec3::new(6.2, 14.0, 0.0), 0.0);
    w.spawn_projectile(splash(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 40.0, 0.0), None, 1.0);
    let ev = run(&mut w, 30);
    let hits: Vec<_> = ev.iter().filter_map(|(_, e)| if let Event::Damaged { tank, amount, .. } = e { Some((*tank, *amount)) } else { None }).collect();
    assert_eq!(hits.iter().filter(|h| h.0 == victim).count(), 1, "{hits:?}");
    assert!((w.tank(victim).unwrap().damage - 3.0).abs() < EPS);
    let hit_pos = ev.iter().find_map(|(_, e)| if let Event::ProjectileExploded { pos, .. } = e { Some(*pos) } else { None }).unwrap();
    let c = w.tank(corner).unwrap().world_box();
    let d = (c.closest_point(hit_pos) - hit_pos).length();
    assert!(d > 5.0 && (w.tank(corner).unwrap().damage - 3.0).abs() < EPS, "corner dist {d}");
}

#[test]
fn sniping_bot_turns_to_face_its_target_and_fires() {
    // Snipe destination is one metre towards the target: the bot turns on
    // the spot instead of freezing with the target behind it.
    let (mut w, a, b) = bot_world();
    w.set_ai(a, Some(ai_data(&[("snipe_weight", "1"), ("ram_weight", "0"), ("flank_weight", "0"), ("back_weight", "0"), ("aim", "0")])));
    // Target behind-left; B is a passive player.
    w.tank_mut(b).unwrap().pos = Vec3::new(-30.0, -40.0, 0.0);
    let start = w.tank(a).unwrap().pos;
    let ev = run(&mut w, 400);
    let t = w.tank(a).unwrap();
    assert_eq!(t.ai.as_ref().unwrap().tactic, ai::Tactic::Snipe);
    let to_target = (w.tank(b).unwrap().pos - t.pos).normalize();
    assert!(t.forward().dot(to_target) > 0.9, "not facing the target");
    assert!((t.pos - start).truncate().length() < 3.0, "a sniper holds its ground");
    assert!(ev.iter().any(|(_, e)| matches!(e, Event::Fired { tank, .. } if *tank == a)));
}

fn barrel_elevation_deg(t: &Tank) -> f32 {
    let b: Vec3 = t.muzzle().matrix3.y_axis.into();
    b.z.clamp(-1.0, 1.0).asin().to_degrees()
}

#[test]
fn auto_level_returns_barrel_to_its_elevation_after_bumps() {
    // Flat ground with a row of 3 m bumps across x = 40..120 m.
    let n = 128;
    let h = (0..n * n)
        .map(|i| {
            let x = (i % n) as f32 * 2.0;
            // Smooth bumps, up to ~11° of slope (no turret saturation).
            if (40.0..120.0).contains(&x) { 0.6 - 0.6 * ((x - 40.0) * std::f32::consts::TAU / 20.0).cos() } else { 0.0 }
        })
        .collect();
    let terrain = HeightField::new(n, h, 2.0, glam::Vec2::ZERO);
    let mut w = World::new(Some(terrain));
    let id = w.add_tank(Arc::new(medium()), Vec3::new(10.0, 60.0, 0.0), std::f32::consts::FRAC_PI_2);
    run(&mut w, 5);
    let before = barrel_elevation_deg(w.tank(id).unwrap());
    let pitch_before = w.tank(id).unwrap().pitch_n;
    w.set_move(id, Move { y: 1.0, ..Move::NULL });
    let mut max_dev = 0.0f32;
    for _ in 0..400 {
        run(&mut w, 1);
        let t = w.tank(id).unwrap();
        if t.pos.x > 140.0 {
            break;
        }
        max_dev = max_dev.max((barrel_elevation_deg(t) - before).abs());
    }
    w.set_move(id, Move::NULL);
    run(&mut w, 60);
    let t = w.tank(id).unwrap();
    assert!(t.pos.x > 140.0, "{}", t.pos);
    let after = barrel_elevation_deg(t);
    eprintln!("elevation before {before:.3}° after {after:.3}° (pitchN {pitch_before:.4} → {:.4}), max deviation on the bumps {max_dev:.2}°", t.pitch_n);
    assert!((after - before).abs() < 0.05, "barrel drifted from {before}° to {after}°");
}

#[test]
fn trigger_box_is_the_bounds_with_halved_xy() {
    // PowerUpParams::preload: box = shape bounds, then min.x, min.y, max.x,
    // max.y *= 0.5; z is kept.
    let b = Aabb::new(Vec3::new(-6.0, -1.3, -0.6), Vec3::new(6.2, 1.4, 8.75));
    let t = TriggerShape::trigger_box_of(b);
    assert_eq!(t, Aabb::new(Vec3::new(-3.0, -0.65, -0.6), Vec3::new(3.1, 0.7, 8.75)));
}

/// A drive along +y from y = -20 at `x`, logging contact with the trigger's
/// collision geometry and its onEnter: (first collision tick, first enter
/// tick, final position).
fn drive_at_trigger(w: &mut World, tr: TriggerId, x: f32) -> (Option<usize>, Option<usize>, Vec3) {
    let id = w.add_tank(Arc::new(medium()), Vec3::new(x, -20.0, 0.0), 0.0);
    w.set_move(id, Move { y: 1.0, ..Move::NULL });
    let (mut hit, mut entered) = (None, None);
    for k in 0..150 {
        for e in w.tick() {
            match e {
                Event::Collided { other: HitObject::Static(i), .. } if w.statics[i].trigger == Some(tr) => {
                    hit.get_or_insert(k);
                }
                Event::PowerUpEntered { trigger, .. } if trigger == tr => {
                    entered.get_or_insert(k);
                }
                _ => {}
            }
        }
    }
    (hit, entered, w.tank(id).unwrap().pos)
}

/// The Reload pad's shape in miniature: an arch with legs at |x| ≥ 3.2,
/// open below z = 6, shape bounds x ±6 (so the trigger box is x ±3).
fn arch() -> TriggerShape {
    let mut tris = Vec::new();
    for b in [
        Aabb::new(Vec3::new(-6.0, -1.3, 0.0), Vec3::new(-3.2, 1.3, 8.0)),
        Aabb::new(Vec3::new(3.2, -1.3, 0.0), Vec3::new(6.0, 1.3, 8.0)),
        Aabb::new(Vec3::new(-6.0, -1.3, 6.0), Vec3::new(6.0, 1.3, 8.0)),
    ] {
        tris.extend(StaticShape::from_box(b).triangles);
    }
    let bounds = Aabb::new(Vec3::new(-6.0, -1.3, 0.0), Vec3::new(6.0, 1.3, 8.0));
    let collision = StaticShape { triangles: tris, bounds };
    TriggerShape { bounds, trigger_box: TriggerShape::trigger_box_of(bounds), collision: Some(Arc::new(collision)), ..TriggerShape::default() }
}

fn reload_data() -> Arc<PowerUpParams> {
    Arc::new(PowerUpParams::from_fields(&Fields::from_pairs("Reload", "PowerUpDef", &[("kind", "ammo")])))
}

#[test]
fn arch_leg_blocks_without_firing_the_action() {
    let mut w = flat_world();
    let tr = w.add_power_up("Reload", reload_data(), arch(), Affine3A::IDENTITY);
    let (hit, entered, end) = drive_at_trigger(&mut w, tr, 4.6);
    assert!(hit.is_some() && entered.is_none() && end.y < -1.3, "hit {hit:?} entered {entered:?} end {end}");
}

#[test]
fn arch_opening_fires_the_action_without_contact() {
    let mut w = flat_world();
    let tr = w.add_power_up("Reload", reload_data(), arch(), Affine3A::IDENTITY);
    let (hit, entered, end) = drive_at_trigger(&mut w, tr, 0.0);
    assert!(hit.is_none() && entered.is_some() && end.y > 20.0, "hit {hit:?} entered {entered:?} end {end}");
}

/// powerup.dts in miniature: bounds ±0.75 (trigger box ±0.375), a
/// collision box ±0.66 floating at z 1.0–2.5.
fn pickup() -> TriggerShape {
    let bounds = Aabb::new(Vec3::new(-0.75, -0.75, 0.0), Vec3::new(0.75, 0.75, 2.7));
    let col = StaticShape::from_box(Aabb::new(Vec3::new(-0.66, -0.69, 1.0), Vec3::new(0.66, 0.69, 2.5)));
    TriggerShape { bounds, trigger_box: TriggerShape::trigger_box_of(bounds), collision: Some(Arc::new(col)), ..TriggerShape::default() }
}

#[test]
fn power_up_is_driven_over_and_picked_up() {
    let mut w = flat_world();
    let data = PowerUpParams::from_fields(&Fields::from_pairs("TestPowerup", "PowerUpDef", &[("kind", "weapon"), ("min_off_ms", "60000"), ("max_off_ms", "90000")]));
    let tr = w.add_power_up("pup", Arc::new(data), pickup(), Affine3A::IDENTITY);
    assert!(w.trigger(tr).unwrap().on);
    let (hit, entered, end) = drive_at_trigger(&mut w, tr, 0.0);
    // No contact at all: the tank drives over it, collects it and carries on.
    assert!(hit.is_none() && entered.is_some(), "hit {hit:?} entered {entered:?}");
    assert!(!w.trigger(tr).unwrap().on);
    assert!(end.y > 15.0, "{end}");
    // While on it is still solid for shots.
    w.triggers[0].on = true;
    let (a, b) = (Vec3::new(0.0, -5.0, 1.8), Vec3::new(0.0, 5.0, 1.8));
    assert!(matches!(w.ray_cast(a, b, None), Some((_, _, _, HitObject::Static(_)))));
}

#[test]
fn power_up_that_is_off_is_not_solid() {
    // BuildPolyList / castRay return nothing while off.
    let mut w = flat_world();
    let data = PowerUpParams::from_fields(&Fields::from_pairs("TestPowerup", "PowerUpDef", &[("kind", "weapon"), ("starts_on", "0"), ("min_off_ms", "60000"), ("max_off_ms", "90000")]));
    let tr = w.add_power_up("pup", Arc::new(data), pickup(), Affine3A::IDENTITY);
    assert!(!w.trigger(tr).unwrap().on);
    let (hit, entered, end) = drive_at_trigger(&mut w, tr, 0.0);
    assert!(hit.is_none() && entered.is_none() && end.y > 15.0, "hit {hit:?} entered {entered:?} end {end}");
    // Shots pass through it too.
    let (a, b) = (Vec3::new(0.0, -5.0, 1.8), Vec3::new(0.0, 5.0, 1.8));
    assert!(w.ray_cast(a, b, None).is_none());
    w.triggers[0].on = true;
    assert!(matches!(w.ray_cast(a, b, None), Some((_, _, _, HitObject::Static(_)))));
}

#[test]
fn set_tank_transform_drops_from_15m_clear_of_airborne_tanks() {
    // A second tank set to the same point finds the first in the ±5 m cube
    // and moves by gRandGen [−2, 2) in x/y and 10 m higher.
    let mut w = flat_world();
    let data = Arc::new(medium());
    let a = w.add_tank(data.clone(), Vec3::new(50.0, 50.0, 0.0), 0.0);
    let b = w.add_tank(data.clone(), Vec3::new(-50.0, 50.0, 0.0), 0.0);
    let ground = w.tank(a).unwrap().pos.z;
    let tilted = Mat3::from_rotation_x(0.4) * Mat3::from_rotation_z(-0.7);
    w.set_tank_transform(a, Vec3::new(10.0, 20.0, 99.0), Some(tilted));
    let ta = w.tank(a).unwrap();
    assert!((ta.pos - Vec3::new(10.0, 20.0, ground + 15.0)).length() < 1e-4, "{}", ta.pos);
    assert!(ta.rot.z_axis.abs_diff_eq(Vec3::Z, 1e-5));
    let f = (tilted.y_axis * Vec3::new(1.0, 1.0, 0.0)).normalize();
    assert!(ta.rot.y_axis.abs_diff_eq(f, 1e-5), "{}", ta.rot.y_axis);
    let mut rng = w.rng.clone();
    w.set_tank_transform(b, Vec3::new(10.0, 20.0, 0.0), None);
    let dx = (rng.rand_i() as f64 / 2_147_483_648.0 * 4.0 - 2.0) as f32;
    let dy = (rng.rand_i() as f64 / 2_147_483_648.0 * 4.0 - 2.0) as f32;
    let pb = w.tank(b).unwrap().pos;
    assert!((pb - Vec3::new(10.0 + dx, 20.0 + dy, ground + 25.0)).length() < 1e-3, "{pb}");
    // A tank on the ground at the point is not in the cube: straight drop.
    let mut w = flat_world();
    let a = w.add_tank(data.clone(), Vec3::new(10.0, 20.0, 0.0), 0.0);
    let b = w.add_tank(data, Vec3::new(-50.0, 50.0, 0.0), 0.0);
    w.set_tank_transform(b, Vec3::new(10.0, 20.0, 0.0), None);
    assert!((w.tank(b).unwrap().pos - Vec3::new(10.0, 20.0, ground + 15.0)).length() < 1e-4);
    // It lands on the first one and does not stay locked to it.
    run(&mut w, 200);
    let (pa, pb) = (w.tank(a).unwrap().center(), w.tank(b).unwrap().center());
    let r = 2.0 * w.tank(a).unwrap().data.geometry.small_radius();
    assert!((pa - pb).length() >= r - 0.05, "{pa} {pb}");
    // Clients only snap.
    w.server = false;
    w.set_tank_transform(b, Vec3::new(-30.0, 0.0, 5.0), None);
    assert!((w.tank(b).unwrap().pos.z - ground).abs() < 1e-4);
}

// ------------------------------------------------- netcode stage 0 (docs/netcode.md §2.2, §2.4)

/// A varied move sequence: throttle, reverse, turns, turret aim.
fn scripted_move(i: usize) -> Move {
    let phase = (i / 25) % 6;
    let (x, y) = [(0.0, 1.0), (1.0, 1.0), (-1.0, 0.5), (0.0, -1.0), (0.3, 0.0), (0.0, 0.0)][phase];
    Move { x, y, yaw: ((i as f32) * 0.37).sin() * 0.05, pitch: ((i as f32) * 0.21).cos() * 0.02, ..Move::NULL }
}

fn world_with_rock() -> World {
    let mut w = flat_world();
    let rock = StaticShape::from_box(Aabb::new(Vec3::new(-2.0, -2.0, 0.0), Vec3::new(2.0, 2.0, 3.0)));
    w.add_static(Arc::new(rock), Affine3A::from_translation(Vec3::new(4.0, 30.0, 0.0)));
    w
}

#[test]
fn step_tank_is_bit_identical_to_tick_for_a_lone_tank() {
    for server in [true, false] {
        let mut a = world_with_rock();
        let mut b = world_with_rock();
        a.server = server;
        b.server = server;
        let ia = a.add_tank(Arc::new(medium()), Vec3::new(0.0, 0.0, 2.0), 0.1);
        let ib = b.add_tank(Arc::new(medium()), Vec3::new(0.0, 0.0, 2.0), 0.1);
        for i in 0..600 {
            let mv = scripted_move(i);
            a.set_move(ia, mv);
            let ea = a.tick();
            let eb = b.step_tank(ib, mv);
            b.time_ms += 32;
            assert_eq!(format!("{:?}", a.tank(ia)), format!("{:?}", b.tank(ib)), "tick {i}, server {server}");
            assert_eq!(format!("{ea:?}"), format!("{eb:?}"), "events at tick {i}");
            assert_eq!(a.rng, b.rng);
            assert_eq!(a.math_rng, b.math_rng);
        }
        assert!(a.tank(ia).unwrap().pos.length() > 10.0);
    }
}

#[test]
fn step_tank_on_a_client_draws_no_randomness() {
    // The unstuck case (server-only kick from math_rng) and driving around.
    let mut w = flat_world();
    let rock = StaticShape::from_box(Aabb::new(Vec3::new(-20.0, -20.0, 0.0), Vec3::new(20.0, 20.0, 50.0)));
    w.add_static(Arc::new(rock), Affine3A::IDENTITY);
    w.server = false;
    let id = w.add_tank_at(Arc::new(medium()), Vec3::new(0.0, 0.0, 50.5), Mat3::IDENTITY);
    let other = w.add_tank(Arc::new(medium()), Vec3::new(40.0, 0.0, 0.0), 0.0);
    w.tank_mut(other).unwrap().puppet = true;
    let (rng, math) = (w.rng.clone(), w.math_rng.clone());
    let mut stuck = 0;
    for i in 0..300 {
        let mv = if i < 100 { Move::NULL } else { scripted_move(i) };
        w.step_tank(id, mv);
        stuck = stuck.max(w.tank(id).unwrap().stuck_ticks);
        w.time_ms += 32;
    }
    assert!(stuck * 32 > 1000, "passed the unstuck time ({stuck} ticks)");
    assert_eq!(w.rng, rng);
    assert_eq!(w.math_rng, math);
}

#[test]
fn step_tank_keeps_queued_events_and_ignores_unknown_tanks() {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.hurt_me(id, "1");
    let queued = w.events.len();
    assert!(queued > 0);
    assert!(w.step_tank(TankId(9999), Move::NULL).is_empty());
    w.step_tank(id, Move { y: 1.0, ..Move::NULL });
    assert_eq!(w.events.len(), queued);
    assert_eq!(w.time_ms, 0);
}

#[test]
fn puppets_are_not_simulated_but_block() {
    let mut w = flat_world();
    let driver = w.add_tank(Arc::new(medium()), Vec3::new(0.0, 0.0, 0.0), 0.0);
    let puppet = w.add_tank(Arc::new(medium()), Vec3::new(0.0, 12.0, 0.0), 0.0);
    w.tank_mut(puppet).unwrap().puppet = true;
    // A velocity and an input: World::tick must leave it alone.
    w.tank_mut(puppet).unwrap().vel = Vec3::new(5.0, 0.0, 0.0);
    w.set_move(puppet, Move { y: 1.0, ..Move::NULL });
    let before = format!("{:?}", w.tank(puppet));
    w.set_move(driver, Move { y: 1.0, ..Move::NULL });
    let mut hit = false;
    for (_, e) in run(&mut w, 120) {
        if matches!(e, Event::Collided { tank, other: HitObject::Tank(o), .. } if tank == driver && o == puppet) {
            hit = true;
        }
    }
    assert!(hit, "the driver runs into the puppet");
    assert_eq!(format!("{:?}", w.tank(puppet)), before, "puppet untouched by World::tick");
    assert!(w.tank(driver).unwrap().pos.y < 12.0);
    // step_tank still steps a puppet when asked.
    w.step_tank(puppet, Move::NULL);
    assert_ne!(format!("{:?}", w.tank(puppet)), before);
}

/// Lag compensation: a shot of a shooter with a view delay hits the target
/// where it was that long ago (here: before it jumped aside); without one,
/// or with a splash weapon, it misses as before.
#[test]
fn lag_compensated_shot_hits_where_the_shooter_saw_the_target() {
    let shoot = |view_delay: Option<f32>, data: Arc<ProjectileParams>| -> f32 {
        let mut w = flat_world();
        let shooter = w.add_tank(Arc::new(medium()), Vec3::new(60.0, -60.0, 0.0), 0.0);
        let victim = w.add_tank(Arc::new(medium()), Vec3::new(0.0, 20.0, 0.0), 0.0);
        if let Some(v) = view_delay {
            w.lag_comp.view_delay.insert(shooter, v);
        }
        run(&mut w, 20);
        // The target moves 10 m aside; 3 ticks later the remote shot leaves,
        // aimed where the shooter (10 ticks behind) still sees it.
        let z = w.tank(victim).unwrap().center().z;
        w.tanks.iter_mut().find(|t| t.id == victim).unwrap().pos.x += 10.0;
        run(&mut w, 3);
        w.spawn_projectile(data, Vec3::new(0.0, 0.0, z), Vec3::new(0.0, 85.0, 0.0), Some(shooter), 1.0);
        run(&mut w, 30);
        w.tank(victim).unwrap().damage
    };
    assert_eq!(shoot(None, default_projectile()), 0.0, "uncompensated: a miss");
    assert!(shoot(Some(10.0), default_projectile()) > 0.0, "compensated: a hit");
    assert_eq!(shoot(Some(1.0), default_projectile()), 0.0, "too little rewind: a miss");
    let mut w = flat_world();
    w.lag_comp.view_delay.insert(TankId(99), 10.0);
    w.spawn_projectile(splash(), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0), Some(TankId(99)), 1.0);
    assert_eq!(w.lag_comp.rewind_ms(w.projectiles[0].id, w.time_ms), 0.0, "splash weapons are not compensated");
}

// ---- the Mortar and Scatter shot power-ups (assets/data/weapons.ron) ----

/// The game's effects and weapons (`effects.ron`, `weapons.ron`, with
/// OpenTT's two power-up weapons), as the engine declares them.
fn special_db() -> Defs {
    let root = tt_assets::layout::default_dir();
    let mut set = tt_assets::data::defs::DefSet::new();
    for set_name in ["effects", "weapons"] {
        for d in tt_assets::data::defs::load_file(&root, set_name).expect(set_name) {
            set.declare_def(&d);
        }
    }
    let mut db = Defs::new();
    db.add(set.objects().cloned());
    db
}

fn real(name: &str) -> Arc<ProjectileParams> {
    Arc::new(special_db().projectile(name).unwrap_or_else(|| panic!("{name}")))
}

const FIRE: Move = Move { trigger: [true, false, false, false, false, false], ..Move::NULL };

/// A mission's gravity scale (`World::gravity`, the Moon's 0.4): a tank
/// falls and a shell drops at that share of the game's gravity; a mortar
/// launches slower but keeps its reach. 1 (every other mission) is the
/// game's gravity exactly (`free_fall_then_land`, the goldens).
#[test]
fn mission_gravity_scales_falls() {
    let fall = |g: f32| {
        let mut w = flat_world();
        w.gravity = g;
        let id = w.add_tank_at(Arc::new(medium()), Vec3::new(0.0, 0.0, 30.0), Mat3::IDENTITY);
        run(&mut w, 5);
        w.tank(id).unwrap().vel.z
    };
    assert!((fall(0.4) - 0.4 * fall(1.0)).abs() < EPS);
    let shell_dvz = |g: f32| {
        let mut w = flat_world();
        w.gravity = g;
        let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
        w.set_projectile(id, real("MortarProjectile"));
        w.time_ms = 5000;
        w.set_move(id, FIRE);
        w.tick();
        w.set_move(id, Move::NULL);
        let v0 = w.projectiles[0].vel.z;
        run(&mut w, 3);
        w.projectiles[0].vel.z - v0
    };
    assert!(shell_dvz(1.0) < 0.0);
    assert!((shell_dvz(0.4) - 0.4 * shell_dvz(1.0)).abs() < 1e-3, "{} {}", shell_dvz(0.4), shell_dvz(1.0));
    let p = real("MortarProjectile");
    assert!(p.launch_speed(0.0, 0.4) < p.launch_speed(0.0, 1.0));
    let (r1, _) = p.high_angle_reach(0.0, 0.0, 1.0).unwrap();
    let (r4, _) = p.high_angle_reach(0.0, 0.0, 0.4).unwrap();
    assert!((r1 - r4).abs() < 0.02 * r1, "{r1} {r4}");
}

/// Fires one shot of `p` from a fresh tank at the origin facing +y with the
/// turret at `pitch_n`: (world, tank, the shot's events).
fn one_shot(p: Arc<ProjectileParams>, pitch_n: f32) -> (World, TankId, Vec<Event>) {
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_projectile(id, p);
    w.tank_mut(id).unwrap().pitch_n = pitch_n;
    w.time_ms = 5000;
    w.set_move(id, FIRE);
    let ev = w.tick();
    w.set_move(id, Move::NULL);
    (w, id, ev)
}

#[test]
fn scatter_shot_fires_seven_pellets_in_a_cone() {
    tt_assets::require_game_data!();
    let p = real("ScatterProjectile");
    assert_eq!((p.pellet_count, p.spread_angle, p.max_life_ms, p.rounds, p.reload_ms), (7, 6.0, 600, 16, 700));
    assert!(p.spread_ring && p.spread_inner == 1.0 && p.spread_jitter == 0.2 && p.trail_length > 0.0);
    let (mut w, id, ev) = one_shot(p.clone(), 0.0);
    // One Fired (one fire sound) for the shot, one round of ammo.
    assert_eq!(ev.iter().filter(|e| matches!(e, Event::Fired { .. })).count(), 1);
    assert_eq!(w.tank(id).unwrap().ammo, 15);
    let pellets: Vec<&Projectile> = w.projectiles.iter().filter(|q| q.source == Some(id)).collect();
    assert_eq!(pellets.len(), 7);
    // A cone about the barrel: one pellet along it, six evenly on a ring
    // at about 3° (±4 % jitter), all around it (above and below the
    // barrel too, not a flat fan).
    let t = w.tank(id).unwrap();
    let m = t.muzzle();
    let (fwd, up) = (Vec3::from(m.matrix3.y_axis).normalize(), Vec3::from(m.matrix3.z_axis).normalize());
    let right = fwd.cross(up).normalize();
    let up = right.cross(fwd);
    let off: Vec<(f32, f32)> = pellets
        .iter()
        .map(|q| {
            let d = q.vel.normalize();
            (d.angle_between(fwd).to_degrees(), d.dot(up).atan2(d.dot(right)).to_degrees())
        })
        .collect();
    let mut cone: Vec<f32> = off.iter().map(|o| o.0).collect();
    cone.sort_by(f32::total_cmp);
    // (The tank settles on the ground: a little of its velocity is inherited.)
    assert!(cone[0] < 0.15, "{cone:?}");
    assert!(cone[1..].iter().all(|a| (2.8..=3.2).contains(a)), "ring {cone:?}");
    assert!(off.iter().filter(|o| o.0 > 1.0 && o.1.to_radians().sin() > 0.5).count() >= 2, "pellets above: {off:?}");
    assert!(off.iter().filter(|o| o.0 > 1.0 && o.1.to_radians().sin() < -0.5).count() >= 2, "pellets below: {off:?}");
    for q in &pellets {
        assert!((q.vel.length() - 110.0).abs() < 0.5, "speed {}", q.vel.length());
    }
    // Deterministic from the shot number (the server, the shooter's
    // prediction and the reticle agree); another shot turns the ring.
    let same = p.launch_dirs(&m, t.pitch_n, t.fire_count);
    for (q, d) in pellets.iter().zip(&same) {
        assert!(q.vel.angle_between(*d) < 0.1f32.to_radians(), "{}", q.vel.angle_between(*d).to_degrees());
    }
    let next = p.launch_dirs(&m, t.pitch_n, t.fire_count + 1);
    assert!(same.iter().zip(&next).skip(1).any(|(a, b)| a.angle_between(*b) > 0.05f32.to_radians()), "the jitter changes per shot");
    // Gone 0.6 s later (about 66 m): expired, or a low ring pellet puffed
    // on the ground first.
    let ev = run(&mut w, 20);
    assert!(w.projectiles.iter().all(|q| q.deleted || q.exploded_at.is_some()), "pellets left: {}", w.projectiles.iter().filter(|q| !q.deleted && q.exploded_at.is_none()).count());
    let gone = ev.iter().filter(|(_, e)| matches!(e, Event::ProjectileRemoved { .. } | Event::ProjectileExploded { .. })).count();
    assert!(gone >= 7, "{gone}");
}

#[test]
fn scatter_shot_is_strong_close_up_and_spreads_at_range() {
    tt_assets::require_game_data!();
    let p = real("ScatterProjectile");
    let hits_at = |dist: f32| -> (usize, f32) {
        let mut w = flat_world();
        let shooter = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
        let victim = w.add_tank(Arc::new(medium()), Vec3::new(0.0, dist, 0.0), 0.0);
        w.set_projectile(shooter, p.clone());
        // Barrel level (turret_pitch_center 2° up at pitchN 0).
        w.tank_mut(shooter).unwrap().pitch_n = 2.0 / 21.0;
        w.time_ms = 5000;
        w.set_move(shooter, FIRE);
        let mut ev = run(&mut w, 1);
        w.set_move(shooter, Move::NULL);
        ev.extend(run(&mut w, 30));
        let hits = ev.iter().filter(|(_, e)| matches!(e, Event::Damaged { tank, .. } if *tank == victim)).count();
        (hits, w.tank(victim).unwrap().damage)
    };
    let (near, dmg) = hits_at(8.0);
    assert_eq!(near, 7, "every pellet hits at 8 m");
    assert!((dmg - 0.65 * near as f32).abs() < 1e-3, "0.65 per pellet: {dmg}");
    // The ring (3° off the barrel) passes a tank at about 30 m: the middle
    // pellet alone hits further out.
    let ring: Vec<usize> = [20.0, 30.0, 35.0, 40.0, 50.0].iter().map(|d| hits_at(*d).0).collect();
    eprintln!("hits by range: {ring:?}");
    assert!(ring.windows(2).all(|w| w[1] <= w[0]), "fewer pellets hit further out: {ring:?}");
    assert!(ring[0] >= 2 && *ring.last().unwrap() >= 1 && *ring.last().unwrap() < near, "{ring:?}");
    assert!(hits_at(60.0).0 >= 1, "still reaches 60 m");
    assert_eq!(hits_at(80.0).0, 0, "out of range at 80 m");
}

/// The mortar's flight from the origin: (apex height above the muzzle,
/// horizontal distance where it lands, flight time s).
fn mortar_flight(pitch_n: f32) -> (f32, f32, f32) {
    let (mut w, id, ev) = one_shot(real("MortarProjectile"), pitch_n);
    let pid = ev.iter().find_map(|e| if let Event::Fired { projectile, .. } = e { Some(*projectile) } else { None }).expect("fired");
    let z0 = Vec3::from(w.tank(id).unwrap().muzzle().translation).z;
    let mut apex = f32::MIN;
    for k in 0..400 {
        let ev = w.tick();
        if let Some(pos) = ev.iter().find_map(|e| if let Event::ProjectileExploded { projectile, pos, .. } = e { (*projectile == pid).then_some(*pos) } else { None }) {
            return (apex - z0, pos.truncate().length(), (k + 1) as f32 * DT);
        }
        apex = apex.max(w.projectile(pid).unwrap().pos.z);
    }
    panic!("the mortar never landed");
}

/// The muzzle of a fresh tank at the origin with the turret at `pitch_n`.
fn mortar_muzzle(pitch_n: f32) -> Vec3 {
    let (w, id, _) = one_shot(real("MortarProjectile"), pitch_n);
    let t = w.tank(id).unwrap();
    Vec3::from(t.muzzle_at(t.yaw_n, pitch_n).translation)
}

#[test]
fn mortar_flies_a_high_arc_and_the_barrel_sets_its_range() {
    tt_assets::require_game_data!();
    let p = real("MortarProjectile");
    assert_eq!((p.rounds, p.volley_size, p.reload_ms, p.blast_radius), (6, 1, 1800, 11.0));
    assert_eq!((p.launch_pitch_min, p.launch_pitch_max, p.launch_range_min, p.launch_range_max), (50.0, 65.0, 20.0, 150.0));
    assert!(p.high_angle());
    // The barrel sets the reach linearly: barrel down (pitchN 1) 20 m,
    // level 85 m, up (−1) 150 m on the flat (from the muzzle, 2 m up: the
    // shell lands a little further), always a lob and a short flight.
    let mut last = 0.0;
    for k in 0..=8 {
        let pn = 1.0 - k as f32 * 0.25;
        let (apex, range, time) = mortar_flight(pn);
        let want = 20.0 + 130.0 * (1.0 - pn) * 0.5;
        let e = p.launch_elevation_deg(pn);
        println!("pitchN {pn:5.2}: {e:4.1} deg {:5.1} m/s, lands at {range:5.1} m (nominal {want:5.1}) after {time:.2} s, apex {apex:4.1} m", p.launch_speed(pn, 1.0));
        assert!(range > want && range < want + 5.0, "pitchN {pn}: {range} m, want {want}");
        assert!(range > last + 10.0, "raising the barrel reaches further: {last} -> {range}");
        last = range;
        assert!(apex > 0.25 * range, "a lob: apex {apex} m at {range} m");
        assert!(time > 0.9 && time < 2.6, "flight {time} s at {range} m");
        // The reach the aim solvers use is the simulated one.
        let m = mortar_muzzle(pn);
        let (r, t) = p.high_angle_reach(pn, p.radius - m.z, 1.0).unwrap();
        assert!((r + m.y - range).abs() < 0.3 && (t - time).abs() < 0.07, "solver {r} m {t} s vs flight {range} m {time} s");
    }
    let (_, mid, mid_t) = mortar_flight(0.0);
    assert!((mid - 85.0).abs() < 5.0 && (1.5..=2.5).contains(&mid_t), "mid range {mid} m in {mid_t} s");
}

#[test]
fn mortar_lands_on_the_aim_point_flat_and_sloped() {
    tt_assets::require_game_data!();
    let p = real("MortarProjectile");
    // Flat ground, a 30 % ramp rising along +y and one falling along +y.
    let ramp = |grade: f32| {
        let n = 128;
        let h = (0..n * n).map(|i| ((i / n) as f32 * 8.0 - 512.0) * grade).collect();
        World::new(Some(HeightField::new(n, h, 8.0, glam::Vec2::new(-512.0, -512.0))))
    };
    for (name, grade) in [("flat", 0.0f32), ("uphill", 0.3), ("downhill", -0.3)] {
        for d in [25.0f32, 50.0, 80.0, 110.0, 140.0] {
            let mut w = ramp(grade);
            let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
            w.set_projectile(id, p.clone());
            run(&mut w, 30);
            w.time_ms = 5000;
            let t = w.tank(id).unwrap().clone();
            let y = t.pos.y + d;
            let aim = Vec3::new(t.pos.x, y, w.terrain.as_ref().unwrap().height(t.pos.x, y));
            let pn = t.high_angle_pitch_n(&p, 0.0, aim, 1.0);
            w.tank_mut(id).unwrap().pitch_n = pn;
            w.set_move(id, Move { pitch: 0.0, ..FIRE });
            let mut landed = None;
            for _ in 0..200 {
                let ev = w.tick();
                w.set_move(id, Move::NULL);
                if let Some(pos) = ev.iter().find_map(|e| if let Event::ProjectileExploded { pos, def, .. } = e { (def == "MortarProjectile").then_some(*pos) } else { None }) {
                    landed = Some(pos);
                    break;
                }
            }
            let pos = landed.expect("landed");
            let miss = (pos - aim).truncate().length();
            println!("{name:8} {d:5.0} m: pitchN {pn:+.3}, lands {miss:.2} m from the aim point");
            if pn.abs() < 0.999 {
                assert!(miss < 1.0, "{name} {d} m: landed {miss} m off ({pos} vs {aim})");
            }
        }
    }
}

#[test]
fn mortar_splash_damages_tanks_in_its_radius() {
    tt_assets::require_game_data!();
    let p = real("MortarProjectile");
    let mut w = flat_world();
    let a = w.add_tank(Arc::new(medium()), Vec3::new(-40.0, 0.0, 0.0), 0.0);
    let near = w.add_tank(Arc::new(medium()), Vec3::new(6.0, 0.0, 0.0), 0.0);
    let edge = w.add_tank(Arc::new(medium()), Vec3::new(0.0, -7.5, 0.0), 0.0);
    let far = w.add_tank(Arc::new(medium()), Vec3::new(0.0, 12.0, 0.0), 0.0);
    // Coming down steeply at the origin.
    w.spawn_projectile(p, Vec3::new(0.0, -2.0, 6.0), Vec3::new(0.0, 5.0, -40.0), Some(a), 1.0);
    let ev = run(&mut w, 20);
    assert!(ev.iter().any(|(_, e)| matches!(e, Event::ProjectileExploded { def, .. } if def == "MortarProjectile")));
    assert!((w.tank(near).unwrap().damage - 8.0).abs() < EPS, "{}", w.tank(near).unwrap().damage);
    assert!((w.tank(edge).unwrap().damage - 8.0).abs() < EPS, "{}", w.tank(edge).unwrap().damage);
    assert_eq!(w.tank(far).unwrap().damage, 0.0);
    assert_eq!(w.tank(a).unwrap().damage, 0.0);
    assert_eq!(w.tank(near).unwrap().last_damage_source, Some(a));
}

#[test]
fn power_up_ammo_counts_and_fallback() {
    tt_assets::require_game_data!();
    for (name, count) in [("MortarProjectile", 6), ("ScatterProjectile", 16)] {
        let p = real(name);
        let mut w = flat_world();
        let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
        w.set_projectile(id, default_projectile());
        w.set_projectile(id, p.clone());
        assert_eq!(w.tank(id).unwrap().ammo, count, "{name}");
        // Pull the trigger again and again (one round per shot, however many
        // pellets): every round fires, then the saved default is back.
        let mut shots = 0;
        for i in 0..(count as usize + 2) * (p.reload_ms as usize / 32 + 2) {
            w.set_move(id, if i % 2 == 0 { FIRE } else { Move::NULL });
            shots += w.tick().iter().filter(|e| matches!(e, Event::Fired { def, .. } if def == name)).count();
        }
        assert_eq!(shots, count as usize, "{name}");
        let t = w.tank(id).unwrap();
        assert_eq!(t.current.as_ref().unwrap().name, "DefaultProjectile", "{name}");
    }
}

#[test]
fn reticles_of_the_new_power_ups() {
    tt_assets::require_game_data!();
    let db = special_db();
    // Scatter shot: one row of marks (the middle pellet) only as far as
    // the pellets fly, each as wide as the cone there (the outer pellets'
    // ring of the next shot), and no spots.
    let mut w = flat_world();
    let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
    w.set_projectile(id, real("ScatterProjectile"));
    w.time_ms = 5000;
    let rd = db.reticle("ScatterReticle").unwrap();
    let mut st = ReticleState::default();
    let r = w.reticle(id, &rd, &mut st, w.time_ms).expect("reticle");
    let shown = (0..rd.ticks()).filter(|&i| rd.tick_time(i) <= 0.6).count();
    assert!(shown >= 5, "{shown}");
    assert_eq!(r.dots.len(), shown);
    assert!(r.spots.is_empty());
    let t = w.tank(id).unwrap().clone();
    let p0 = Vec3::from(t.muzzle().translation);
    let far = r.dots.last().unwrap();
    assert!((far.pos - p0).length() > 55.0, "marks to the pellets' reach");
    assert!(r.dots.windows(2).all(|d| d[1].size > d[0].size), "the marks widen with distance");
    w.set_move(id, FIRE);
    w.tick();
    let s = rd.tick_time(shown - 1);
    let mut widest = 0.0f32;
    for q in w.projectiles.iter().filter(|q| q.source == Some(id)) {
        let at = p0 + q.vel * s + Vec3::new(0.0, 0.0, -0.5 * 10.0 * q.data.gravity_scale * s * s);
        widest = widest.max(2.0 * (at - far.pos).length());
    }
    // The outer pellets on the last mark's edge (the jitter: within 15 %).
    assert!((widest / far.size - 1.0).abs() < 0.15, "pellet ring {widest} m, mark {} m", far.size);
    // Mortar: one high arc, the landing spot and a ring as wide as the blast.
    let (w, id, _) = one_shot(real("MortarProjectile"), 0.0);
    let rd = db.reticle("MortarReticle").unwrap();
    let mut st = ReticleState::default();
    let r = w.reticle(id, &rd, &mut st, w.time_ms).expect("reticle");
    assert_eq!(r.dots.len(), 20);
    let top = r.dots.iter().map(|d| d.pos.z).fold(f32::MIN, f32::max);
    assert!(top > 25.0, "arc top {top}");
    assert_eq!(r.spots.len(), 1);
    assert_eq!(r.rings.len(), 1);
    assert_eq!(r.rings[0].size, 22.0);
    assert!(r.rings[0].dir.z > 0.99, "flat on the ground");
    let land = r.rings[0].pos.truncate().length();
    let (_, range, _) = mortar_flight(0.0);
    assert!((land - range).abs() < 1.0, "the ring at the landing point: {land} vs {range}");
}

#[test]
fn reticle_of_a_multi_shell_mortar() {
    tt_assets::require_game_data!();
    let db = special_db();
    let rd = db.reticle("MortarReticle").unwrap();
    // The stock Mortar's dots keep the reticle's size (no cone).
    let (w, id, _) = one_shot(real("MortarProjectile"), 0.0);
    let r = w.reticle(id, &rd, &mut ReticleState::default(), w.time_ms).unwrap();
    assert!(r.dots.iter().all(|d| d.size == rd.size));
    // A carpet (as the Carpet Bombing example mod's): nine shells in a
    // cone, three volleys a pull, smaller blasts.
    let mut p = (*real("MortarProjectile")).clone();
    (p.pellet_count, p.spread_angle, p.spread_ring, p.spread_inner, p.spread_jitter) = (9, 18.0, true, 0.5, 1.0);
    (p.volley_size, p.reload_ms, p.blast_radius) = (3, 250, 8.0);
    let p = Arc::new(p);
    for pitch_n in [0.0f32, -1.0, 1.0] {
        let mut w = flat_world();
        let id = w.add_tank(Arc::new(medium()), Vec3::ZERO, 0.0);
        w.set_projectile(id, p.clone());
        w.tank_mut(id).unwrap().pitch_n = pitch_n;
        w.time_ms = 5000;
        let r = w.reticle(id, &rd, &mut ReticleState::default(), w.time_ms).expect("reticle");
        // The centre shell's arc only, dots as small as the stock ones.
        assert_eq!(r.dots.len(), 20);
        assert!(r.dots.iter().all(|d| d.size == rd.size), "no widening cone marks");
        // A small landing mark per shell, no splash ring each: one ring
        // around the whole carpet.
        assert_eq!(r.spots.len(), 9);
        assert!(r.spots.iter().all(|s| s.size < rd.spot_size_near), "small landing marks");
        assert_eq!(r.rings.len(), 1, "one footprint");
        let ring = r.rings[0];
        assert!(ring.dir.z > 0.99, "flat on the ground");
        let radius = 0.5 * ring.size;
        let c = ring.pos.truncate();
        // Every shell of the pull (three volleys) and its blast inside it.
        w.set_move(id, FIRE);
        let (mut volley, mut blasts) = (Vec::new(), Vec::new());
        for k in 0..600 {
            if k == 20 {
                w.set_move(id, Move::NULL);
            }
            for e in w.tick() {
                if let Event::ProjectileExploded { projectile, pos, .. } = e {
                    blasts.push((projectile, pos));
                }
            }
            if k == 0 {
                volley = w.projectiles.iter().map(|q| q.id).collect();
            }
            if blasts.len() == 27 {
                break;
            }
        }
        assert_eq!(blasts.len(), 27, "pitch {pitch_n}");
        let far = blasts.iter().map(|(_, b)| (b.truncate() - c).length()).fold(0.0f32, f32::max);
        assert!(far + p.blast_radius <= radius + 0.5, "pitch {pitch_n}: a blast {far} m out, ring {radius} m");
        // ... and not much bigger than the carpet.
        assert!(radius < 1.25 * (far + p.blast_radius), "pitch {pitch_n}: ring {radius} m, carpet {far} m");
        // The landing marks where the next volley comes down.
        assert_eq!(volley.len(), 9);
        let first: Vec<Vec3> = blasts.iter().filter(|(q, _)| volley.contains(q)).map(|(_, b)| *b).collect();
        for s in &r.spots {
            let near = first.iter().map(|b| (b.truncate() - s.pos.truncate()).length()).fold(f32::MAX, f32::min);
            assert!(near < 1.5, "pitch {pitch_n}: a mark {near} m from a blast");
        }
    }
}

#[test]
fn bot_aim_solves_the_mortar_reach() {
    tt_assets::require_game_data!();
    let p = real("MortarProjectile");
    for d in [30.0f32, 60.0, 90.0, 140.0] {
        for dz in [-10.0f32, 0.0, 10.0] {
            let pn = p.pitch_n_for_reach(d, dz, 1.0);
            assert!(pn.abs() < 1.0, "{d} m {dz} m: pitchN {pn}");
            let (r, _) = p.high_angle_reach(pn, dz, 1.0).unwrap();
            assert!((r - d).abs() < 0.05, "{d} m {dz} m: lands at {r}");
        }
    }
    // Out of reach: the nearest limit.
    assert_eq!(p.pitch_n_for_reach(400.0, 0.0, 1.0), -1.0);
    assert_eq!(p.pitch_n_for_reach(3.0, 0.0, 1.0), 1.0);
}

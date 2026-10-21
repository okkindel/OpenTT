//! Balance of the Scatter shot and the Mortar with the game's own defs
//! (`assets/data/*.ron`) and the real tank shapes (their hit boxes):
//!
//! - Scatter shot: shots to kill a Light, Medium and Heavy tank at 10 / 30
//!   / 50 m, against the Splash and Speedy power-ups' kill times;
//! - Mortar: flight times over its range, and the shell landing on the aim
//!   point (`Tank::high_angle_pitch_n`, what camera aim and the bots use)
//!   on a real map's hills.
//!
//! `cargo test -p tt-game --test powerup_balance -- --nocapture` prints
//! the tables.

mod common;

use std::sync::Arc;

use common::{assets, defs, tank, FIRE};
use glam::Vec3;
use tt_game::data::ProjectileParams;
use tt_game::fields::Defs;
use tt_game::terrain::HeightField;
use tt_game::{Event, Move, TankId, World, DT};

fn projectile(db: &Defs, name: &str) -> Arc<ProjectileParams> {
    Arc::new(db.projectile(name).unwrap_or_else(|| panic!("{name}")))
}

/// A Medium tank firing `p` at a stationary `victim` tank `dist` m ahead on
/// flat ground, the barrel aimed at the victim's centre (gravity drop
/// allowed for), the trigger pulled as often as the cartridge allows:
/// (shots to kill, seconds from the first shot to the kill).
fn kill(db: &Defs, p: &Arc<ProjectileParams>, victim: &str, dist: f32) -> (usize, f32) {
    let mut w = World::new(Some(HeightField::flat(0.0)));
    let shooter = w.add_tank(tank(db, "MediumTank"), Vec3::ZERO, 0.0);
    let target = w.add_tank(tank(db, victim), Vec3::new(0.0, dist, 0.0), 0.0);
    w.set_projectile(shooter, p.clone());
    w.time_ms = 5000;
    // Barrel elevation for the victim's centre: pitchN from
    // elevation = turret_pitch_center − turret_pitch_range·pitchN.
    let (s, v) = (w.tank(shooter).unwrap().clone(), w.tank(target).unwrap().clone());
    let m = Vec3::from(s.muzzle().translation);
    let d = v.center() - m;
    let t = d.truncate().length() / p.velocity;
    let elev = (d.z + 0.5 * 10.0 * p.gravity_scale * t * t).atan2(d.truncate().length()).to_degrees();
    w.tank_mut(shooter).unwrap().pitch_n = (s.data.turret_pitch_center - elev) / s.data.turret_pitch_range;
    let (mut shots, mut first) = (0usize, None);
    for i in 0..4000 {
        w.tank_mut(shooter).unwrap().ammo = 100;
        w.set_move(shooter, if i % 2 == 0 { FIRE } else { Move::NULL });
        let now = w.time_ms;
        for e in w.tick() {
            match e {
                Event::Fired { tank, .. } if tank == shooter => {
                    shots += 1;
                    first.get_or_insert(now);
                }
                Event::Killed { tank, .. } if tank == target => {
                    return (shots, (now - first.unwrap()) as f32 / 1000.0);
                }
                _ => {}
            }
        }
    }
    panic!("{} never killed the {victim} at {dist} m ({shots} shots)", p.name);
}

#[test]
fn scatter_shot_kill_shots_against_every_tank() {
    tt_assets::require_game_data!();
    let db = defs();
    let scatter = projectile(&db, "ScatterProjectile");
    let (splash, speedy) = (projectile(&db, "SplashProjectile"), projectile(&db, "SpeedyProjectile"));
    let tanks = ["LightTank", "MediumTank", "HeavyTank"];
    let mut table = std::collections::HashMap::new();
    for victim in tanks {
        for dist in [10.0f32, 30.0, 50.0] {
            let sc = kill(&db, &scatter, victim, dist);
            let sp = kill(&db, &splash, victim, dist);
            let sy = kill(&db, &speedy, victim, dist);
            println!(
                "{victim:10} {dist:3.0} m: Scatter {} shots {:.2} s | Splash {} shots {:.2} s | Speedy {} shots {:.2} s",
                sc.0, sc.1, sp.0, sp.1, sy.0, sy.1
            );
            table.insert((victim, dist as i32), (sc, sp, sy));
        }
    }
    let shots = |v: &str, d: i32| table[&(v, d)].0 .0;
    // A full hit close up: Light in 3 shots, Medium in 3–4, Heavy in 5.
    assert!(shots("LightTank", 10) <= 3, "{}", shots("LightTank", 10));
    assert!((3..=4).contains(&shots("MediumTank", 10)), "{}", shots("MediumTank", 10));
    assert!(shots("HeavyTank", 10) <= 5, "{}", shots("HeavyTank", 10));
    for v in tanks {
        let ((_, sc), (_, sp), (_, sy)) = table[&(v, 10)];
        // Close up the strongest of the three.
        assert!(sc < sp && sc < sy, "{v} at 10 m: scatter {sc} s, splash {sp} s, speedy {sy} s");
        // It falls off with range (the cone opens), unlike Speedy.
        assert!(shots(v, 50) >= shots(v, 30) && shots(v, 30) >= shots(v, 10) && shots(v, 50) > shots(v, 10), "{v}: {} {} {}", shots(v, 10), shots(v, 30), shots(v, 50));
        // At 50 m Speedy is the better gun against the light and medium
        // tanks (the big Heavy still takes most of the cone).
        let ((_, sc50), _, (_, sy50)) = table[&(v, 50)];
        assert!(v == "HeavyTank" || sc50 > sy50, "{v} at 50 m Speedy is better: scatter {sc50} s, speedy {sy50} s");
    }
}

/// A world with a mission's terrain.
fn mission_terrain(name: &str) -> World {
    let ron = assets().join("missions").join(format!("{name}.ron"));
    let m = tt_assets::data::mission::load(&ron).unwrap();
    let objects = m.scene_objects(|c| !c.unless.1.is_empty());
    let all: Vec<&tt_assets::mission::SceneObject> = objects.iter().flat_map(|o| o.walk()).collect();
    let tb = all.iter().find(|o| o.class.eq_ignore_ascii_case("Terrain")).expect("terrain");
    let square = tb.get("square_size").map_or(8.0, tt_game::fields::atof);
    let p = tt_assets::find_ci(&tt_assets::resolve(&assets(), ron.parent().unwrap(), tb.get("terrain_file").unwrap())).unwrap();
    World::new(Some(HeightField::from_terrain(&tt_assets::terrain::Terrain::parse(&std::fs::read(p).unwrap()).unwrap(), square)))
}

/// Fires the mortar of `id` with the turret at (`yaw_n`, `pitch_n`): where
/// the shell lands and after how long.
fn fire_mortar(w: &mut World, id: TankId, yaw_n: f32, pitch_n: f32) -> (Vec3, f32) {
    w.time_ms += 5000;
    {
        let t = w.tank_mut(id).unwrap();
        (t.yaw_n, t.pitch_n, t.ammo) = (yaw_n, pitch_n, 6);
    }
    w.set_move(id, FIRE);
    let mut shell = None;
    for k in 0..300 {
        for e in w.tick() {
            match e {
                Event::Fired { tank, projectile, .. } if tank == id => shell = Some(projectile),
                Event::ProjectileExploded { projectile, pos, .. } if Some(projectile) == shell => return (pos, k as f32 * DT),
                _ => {}
            }
        }
        w.set_move(id, Move::NULL);
    }
    panic!("the mortar never landed");
}

#[test]
fn mortar_lands_on_the_aim_point_on_hills() {
    tt_assets::require_game_data!();
    let db = defs();
    let mortar = projectile(&db, "MortarProjectile");
    let medium = tank(&db, "MediumTank");
    // Hilly maps; the tank at a few spots, aim points around it.
    let (mut checked, mut worst, mut slopes) = (0, 0.0f32, Vec::new());
    for (map, spots) in [("tt1_5", [Vec3::new(-60.0, 40.0, 0.0), Vec3::new(80.0, -90.0, 0.0)]), ("tt3_5", [Vec3::new(-30.0, -120.0, 0.0), Vec3::new(110.0, 60.0, 0.0)])] {
        for spot in spots {
            let mut w = mission_terrain(map);
            let hf = w.terrain.clone().unwrap();
            let id = w.add_tank(medium.clone(), Vec3::new(spot.x, spot.y, hf.height(spot.x, spot.y) + 1.0), 0.0);
            w.set_projectile(id, mortar.clone());
            for _ in 0..60 {
                w.tick();
            }
            for heading in [-40.0f32, 0.0, 35.0] {
                for d in [25.0f32, 60.0, 100.0, 140.0] {
                    let t = w.tank(id).unwrap().clone();
                    let yaw_n = heading / t.data.turret_yaw_range;
                    let dir = Vec3::from(t.muzzle_at(yaw_n, 0.0).matrix3.y_axis);
                    let flat = Vec3::new(dir.x, dir.y, 0.0).normalize();
                    let (x, y) = (t.pos.x + flat.x * d, t.pos.y + flat.y * d);
                    let aim = Vec3::new(x, y, hf.height(x, y));
                    let (yaw_n, pn) = t.high_angle_aim(&mortar, yaw_n, aim, 1.0);
                    let (pos, time) = fire_mortar(&mut w, id, yaw_n, pn);
                    let miss = (pos - aim).truncate().length();
                    let short = (aim - t.pos).truncate().length() - (pos - t.pos).truncate().length();
                    let dz = aim.z - t.pos.z;
                    // Out of reach (pinned) or blocked by a hill on the way: skipped.
                    let blocked = (pos - t.pos).truncate().length() < (aim - t.pos).truncate().length() - 3.0;
                    println!("{map} {:?} heading {heading:+3.0} {d:3.0} m (height {dz:+5.1} m): pitchN {pn:+.3}, lands {miss:5.2} m off ({short:+.2} short) after {time:.2} s{}", (spot.x, spot.y), if blocked { " (hill in the way)" } else { "" });
                    if pn.abs() < 0.999 && !blocked {
                        checked += 1;
                        worst = worst.max(miss);
                        slopes.push(dz);
                        assert!(miss < 1.5, "{map}: lands {miss} m from the aim point");
                    }
                }
            }
        }
    }
    println!("{checked} shots on the aim point, worst {worst:.2} m");
    assert!(checked >= 25, "{checked}");
    assert!(slopes.iter().any(|z| *z > 5.0) && slopes.iter().any(|z| *z < -5.0), "uphill and downhill targets: {slopes:?}");
}

#[test]
fn mortar_flight_times() {
    tt_assets::require_game_data!();
    let db = defs();
    let mortar = projectile(&db, "MortarProjectile");
    let mut w = World::new(Some(HeightField::flat(0.0)));
    let id = w.add_tank(tank(&db, "MediumTank"), Vec3::ZERO, 0.0);
    w.set_projectile(id, mortar.clone());
    let mut last = 0.0;
    for pn in [1.0f32, 0.5, 0.0, -0.5, -1.0] {
        let (pos, time) = fire_mortar(&mut w, id, 0.0, pn);
        let d = pos.truncate().length();
        println!("pitchN {pn:+.1}: lands at {d:5.1} m after {time:.2} s");
        assert!(d > last, "barrel up reaches further");
        last = d;
        assert!((1.0..=2.5).contains(&time), "{time} s");
        if (60.0..=110.0).contains(&d) {
            assert!((1.5..=2.5).contains(&time), "mid range {d} m: {time} s");
        }
    }
    assert!(last > 145.0, "{last}");
}

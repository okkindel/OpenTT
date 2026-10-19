//! The "Carpet Bombing (example)" mod (`mods/carpet-bombing/`, a data
//! mod: its own `data/weapons.ron`): one pull of the trigger with its
//! Mortar lays a carpet of blasts, with the mod's defs and a real
//! Medium tank on flat ground.
//!
//! `cargo test -p tt-game --test carpet_bombing -- --nocapture` prints the
//! pattern.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{tank, FIRE};
use glam::Vec3;
use tt_game::data::ProjectileParams;
use tt_game::fields::Defs;
use tt_game::terrain::HeightField;
use tt_game::{Event, Move, World};

fn mod_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/carpet-bombing")
}

/// The game's defs with the mod's `weapons.ron` in place of the game's.
fn defs() -> Defs {
    common::defs_with(|stem| (stem == "weapons").then(|| tt_assets::data::load(&mod_dir().join("data/weapons.ron")).unwrap()), &[])
}

/// Holds the trigger for `hold_ms` with the barrel at `pitch_n` (tank at
/// the origin facing +y): (shots fired, where each shell came down).
fn pull(p: &Arc<ProjectileParams>, pitch_n: f32, hold_ms: i64) -> (usize, Vec<Vec3>) {
    let db = defs();
    let mut w = World::new(Some(HeightField::flat(0.0)));
    let id = w.add_tank(tank(&db, "MediumTank"), Vec3::ZERO, 0.0);
    w.set_projectile(id, p.clone());
    w.tank_mut(id).unwrap().pitch_n = pitch_n;
    w.time_ms = 5000;
    let start = w.time_ms;
    let (mut shots, mut blasts) = (0, Vec::new());
    for _ in 0..1000 {
        w.set_move(id, if w.time_ms - start < hold_ms { FIRE } else { Move::NULL });
        for e in w.tick() {
            match e {
                Event::Fired { tank, .. } if tank == id => shots += 1,
                Event::ProjectileExploded { pos, def, .. } if def == p.name => blasts.push(pos),
                _ => {}
            }
        }
    }
    (shots, blasts)
}

#[test]
fn one_trigger_pull_lays_a_carpet() {
    tt_assets::require_game_data!();
    let db = defs();
    let p = Arc::new(db.projectile("MortarProjectile").expect("the mod's MortarProjectile"));
    for pitch_n in [0.5f32, 0.0, -0.5] {
        let (shots, blasts) = pull(&p, pitch_n, 2000);
        let (xs, ys): (Vec<f32>, Vec<f32>) = blasts.iter().map(|b| (b.x, b.y)).unzip();
        let span = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min);
        let (across, along) = (span(&xs), span(&ys));
        let mut spots: Vec<(i32, i32)> = blasts.iter().map(|b| ((b.x / 2.0).round() as i32, (b.y / 2.0).round() as i32)).collect();
        spots.sort();
        spots.dedup();
        println!("pitchN {pitch_n:+.1}: {shots} volleys, {} blasts at {} spots, {across:.1} m across, {along:.1} m along (from {:.0} m)", blasts.len(), spots.len(), ys.iter().cloned().fold(f32::MAX, f32::min));
        assert_eq!(shots, p.volley_size as usize, "one burst a pull");
        assert_eq!(blasts.len(), shots * p.pellet_count as usize);
        assert!(blasts.len() >= 15, "{} blasts", blasts.len());
        assert!(across >= 30.0 || pitch_n > 0.0, "{across} m across");
        assert!(along >= 8.0, "{along} m along");
        assert!(2 * spots.len() >= 3 * p.pellet_count as usize, "the volleys come down in different places: {} spots", spots.len());
    }
}

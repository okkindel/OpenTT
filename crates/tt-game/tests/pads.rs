//! Boost/jump pads in every shipped mission:
//!
//! - The sim's launch (a tank dropped on the pad, repeatedly, so the 4-state
//!   `TestBoost` pads cycle moe → larry → curly → schemp) goes the way the drawn
//!   arrow points: the arrow's world direction is derived the way the renderer
//!   poses the pad (`tt_render::pose::sequence_pose` of the current state's
//!   sequence on the object transform, as `Engine::trigger_shapes_with_anim`
//!   does). Jump pads (no `direction` node) launch along the pad's up axis.
//! - the pads sit on the terrain under them: the footprint corners of the
//!   transform the renderer and the trigger test use follow the slope.

mod common;

use std::path::{Path, PathBuf};

use common::assets;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use tt_game::data::TriggerShape;
use tt_game::fields::Defs;
use tt_game::mission::{load_shape, object_transform};
use tt_game::terrain::HeightField;
use tt_game::{Event, TriggerKind, World};

/// Every mission RON file (not the map list, `missions/catalog.ron`).
fn missions() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(assets().join("missions"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ron")) && !p.ends_with("catalog.ron"))
        .collect();
    v.sort();
    v
}

/// The server defs of the data files (`data/*.ron`, in declaration order).
/// The server defs and those the mission declares itself (the new
/// maps' `JumpBoost`).
fn mission_defs(own: &[tt_assets::data::defs::Def]) -> Defs {
    common::defs_with(|_| None, own)
}

/// A world with a mission's terrain and boost/jump pads (as the engine
/// adds them: `TriggerShape::from_shape`, the object transform).
fn load_mission(ron: &Path) -> World {
    let m = tt_assets::data::mission::load(ron).unwrap();
    let db = mission_defs(&m.defs);
    let objects = m.scene_objects(|c| !c.unless.1.is_empty());
    let all: Vec<&tt_assets::mission::SceneObject> = objects.iter().flat_map(|o| o.walk()).collect();
    let dir = ron.parent().unwrap();
    let terrain = all.iter().find(|o| o.class.eq_ignore_ascii_case("Terrain")).map(|tb| {
        let square = tb.get("square_size").map_or(8.0, tt_game::fields::atof);
        let p = tt_assets::find_ci(&tt_assets::resolve(&assets(), dir, tb.get("terrain_file").unwrap())).unwrap();
        HeightField::from_terrain(&tt_assets::terrain::Terrain::parse(&std::fs::read(p).unwrap()).unwrap(), square)
    });
    let mut w = World::new(terrain);
    for o in all.iter().filter(|o| o.class.eq_ignore_ascii_case("Boost")) {
        let data = db.boost(o.get("def").unwrap()).unwrap();
        let shape = TriggerShape::from_shape(&load_shape(&assets(), dir, &data.base.shape).unwrap());
        w.add_boost(&o.name, Arc::new(data), shape, object_transform(o));
    }
    w
}

fn name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

/// Drops the tank on pad `ti` and returns the boost Δv and the pad state
/// it was given in; then takes the tank away until the pad's state has
/// advanced (advance_delay_on_enter = 200 ms).
fn hit(w: &mut World, tank: tt_game::TankId, ti: usize) -> (Vec3, usize) {
    let (tid, xf) = (w.triggers[ti].id, w.triggers[ti].transform);
    let up = Vec3::from(xf.matrix3.z_axis);
    {
        let t = w.tank_mut(tank).unwrap();
        t.pos = Vec3::from(xf.translation) + up * 0.6;
        t.vel = Vec3::ZERO;
        t.current_trigger = None;
    }
    let mut got = None;
    for _ in 0..4 {
        let state = match &w.triggers[ti].kind {
            TriggerKind::Boost(b) => b.state,
            _ => unreachable!(),
        };
        for e in w.tick() {
            if let Event::PadUsed { trigger, dv, .. } = e {
                if trigger == tid {
                    got = Some((dv, state));
                }
            }
        }
        if got.is_some() {
            break;
        }
    }
    let got = got.unwrap_or_else(|| panic!("pad {} not triggered", w.triggers[ti].def_name()));
    w.tank_mut(tank).unwrap().pos = Vec3::from(xf.translation) + Vec3::new(0.0, 0.0, 300.0);
    for _ in 0..12 {
        w.tick();
    }
    got
}

/// World direction of the drawn arrow in pad state `state` (renderer path).
/// Boost pads: +Y of the node the `boostarrow` mesh hangs on (the arrow
/// geometry points along it). Jump pads: the pad's up axis.
fn drawn_arrow(w: &World, ti: usize, state: usize) -> Vec3 {
    let tr = &w.triggers[ti];
    let TriggerKind::Boost(b) = &tr.kind else { unreachable!() };
    let path = tt_assets::resolve(&assets(), &assets(), &b.data.base.shape);
    let shape = tt_render::pose::shape(&path).expect("pad shape");
    let xf = Mat4::from(tr.transform);
    if !shape.nodes.iter().any(|n| n.name.eq_ignore_ascii_case("direction")) {
        return xf.transform_vector3(Vec3::Z).normalize();
    }
    let seq = match b.data.state_name.get(state) {
        Some(n) if !n.is_empty() => n.clone(),
        _ => "ambient".to_string(),
    };
    let arrow = shape.objects.iter().find(|o| o.name.eq_ignore_ascii_case("boostarrow")).and_then(|o| o.node).expect("arrow object");
    let node = shape.nodes[arrow].name.clone();
    // The renderer samples the state sequence at the world time; both of
    // its keyframes are the same pose. Check both ends.
    let a = xf * tt_render::pose::node_transform(&shape, &tt_render::pose::sequence_pose(&path, &seq, 0.0), &node).unwrap();
    let b2 = xf * tt_render::pose::node_transform(&shape, &tt_render::pose::sequence_pose(&path, &seq, 10.0), &node).unwrap();
    let (da, db) = (a.transform_vector3(Vec3::Y).normalize(), b2.transform_vector3(Vec3::Y).normalize());
    assert!(da.angle_between(db) < 1e-3, "state {seq}: pose differs between keyframes");
    da
}

#[test]
fn every_pad_launches_where_its_arrow_points() {
    tt_assets::require_game_data!();
    let mut td = mission_defs(&[]).tank("MediumTank").unwrap();
    td.set_shape(&load_shape(&assets(), &assets(), &td.shape_file).unwrap());
    let td = Arc::new(td);
    let (mut pads, mut launches, mut turned) = (0, 0, 0);
    let mut worst: f32 = 0.0;
    for mis in missions() {
        let mut w = load_mission(&mis);
        let tank = w.add_tank(td.clone(), Vec3::new(0.0, 0.0, 500.0), 0.0);
        let boosts: Vec<usize> = (0..w.triggers.len()).filter(|&i| matches!(w.triggers[i].kind, TriggerKind::Boost(_))).collect();
        for ti in boosts {
            pads += 1;
            let states = match &w.triggers[ti].kind {
                TriggerKind::Boost(b) => b.data.states(),
                _ => unreachable!(),
            };
            let mut seen: Vec<Vec3> = Vec::new();
            // Every state, and back to the first.
            for n in 0..=states {
                let (dv, state) = hit(&mut w, tank, ti);
                assert_eq!(state, n % states, "{}: pad {} hit {n}", name(&mis), w.triggers[ti].name);
                let arrow = drawn_arrow(&w, ti, state);
                let ang = dv.normalize().angle_between(arrow).to_degrees();
                worst = worst.max(ang);
                assert!(
                    ang < 2.0,
                    "{} pad #{ti} {:?} ({}) hit {n} state {state}: launch {:?} vs arrow {:?}: {ang:.1} deg",
                    name(&mis),
                    w.triggers[ti].name,
                    w.triggers[ti].def_name(),
                    dv.normalize(),
                    arrow
                );
                if !seen.iter().any(|s| s.angle_between(arrow) < 0.05) {
                    seen.push(arrow);
                }
                launches += 1;
            }
            if seen.len() > 1 {
                turned += 1;
                // The 4-state pads turn a quarter each use.
                assert_eq!(seen.len(), states, "{}: pad #{ti}", name(&mis));
            }
        }
    }
    eprintln!("{pads} pads, {launches} launches, {turned} pads turn with their state; worst {worst:.2} deg");
    assert!(pads >= 100 && turned >= 20, "{pads} pads, {turned} turning");
}

/// The pad's rim (shape origin height) follows the terrain under it with
/// the object transform the renderer and the trigger test use; the
/// opposite rotation sense would tilt it against the slope.
#[test]
fn every_pad_sits_on_its_slope() {
    tt_assets::require_game_data!();
    let mut worst: (f32, String) = (0.0, String::new());
    for mis in missions() {
        let w = load_mission(&mis);
        let Some(terrain) = &w.terrain else { continue };
        for tr in w.triggers.iter().filter(|t| matches!(t.kind, TriggerKind::Boost(_))) {
            let b = tr.shape.bounds;
            let xf = Mat4::from(tr.transform);
            let centre = Vec3::from(tr.transform.translation);
            let lift = centre.z - terrain.height(centre.x, centre.y);
            // Pads on bridges or rocks float above the terrain: compare the
            // corners relative to the centre (the slope, not the height).
            let mut err: f32 = 0.0;
            let mut mirrored: f32 = 0.0;
            let m2 = Mat4::from_rotation_translation(glam::Quat::from_mat3(&glam::Mat3::from(tr.transform.matrix3)).inverse(), centre);
            // Eight points on the octagonal rim (inside the box corners).
            let r = 0.8 * 0.5 * (b.max.x - b.min.x).min(b.max.y - b.min.y);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let (x, y) = (r * a.cos(), r * a.sin());
                let c = xf.transform_point3(Vec3::new(x, y, 0.0));
                err = err.max((c.z - terrain.height(c.x, c.y) - lift).abs());
                let c2 = m2.transform_point3(Vec3::new(x, y, 0.0));
                mirrored = mirrored.max((c2.z - terrain.height(c2.x, c2.y) - lift).abs());
            }
            let what = format!("{} pad {:?} ({})", name(&mis), tr.name, tr.def_name());
            if err > worst.0 {
                worst = (err, what.clone());
            }
            assert!(err < 0.6, "{what}: footprint off the slope by {err:.2} m (lift {lift:.2})");
            // The opposite rotation sense would tilt it against the slope (a pad
            // turned 180 degrees about its tilt fits both ways equally).
            let tilt = tr.transform.matrix3.z_axis.angle_between(glam::Vec3A::Z).to_degrees();
            if tilt > 5.0 {
                assert!(mirrored >= err - 0.01, "{what}: the inverse rotation fits better ({mirrored:.2} vs {err:.2})");
            }
        }
    }
    eprintln!("worst footprint error {:.2} m: {}", worst.0, worst.1);
}

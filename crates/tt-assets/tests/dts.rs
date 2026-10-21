//! Parse every shape of the asset directory.

use std::path::{Path, PathBuf};
use tt_assets::dts::{MeshKind, Shape};

fn game() -> PathBuf {
    tt_assets::layout::default_dir()
}

fn find_dts(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            find_dts(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dts")) {
            out.push(p);
        }
    }
}

/// All shapes, with the version from their header.
fn shapes() -> Vec<(PathBuf, u16, Vec<u8>)> {
    let mut v = Vec::new();
    find_dts(&game(), &mut v);
    v.sort();
    v.into_iter()
        .map(|p| {
            let d = std::fs::read(&p).unwrap();
            let ver = u16::from_le_bytes([d[0], d[1]]);
            (p, ver, d)
        })
        .collect()
}

#[test]
fn all_shapes_parse() {
    tt_assets::require_game_data!();
    let all = shapes();
    // 48 of the game, plus OpenTT's mortarrocket.dts (examples/mortar_rocket.rs),
    // devbox.dts (examples/powerup_terrain.rs), the fifteen Moon props and tt1_5's
    // two (examples/gltf_to_dts.rs, [`MOON_PROPS`]) and tt2_5's three Halloween
    // props (models/props/halloween/) and tt3_5's darktower.dts
    // (examples/dark_cortex.rs) and wreck_fighter.dts (examples/gltf_to_dts.rs).
    assert_eq!(all.len(), 72);
    let mut parsed = 0;
    for (p, ver, data) in &all {
        let name = p.display();
        if *ver < 23 {
            // markers/octahedron.dts is a pre-v19 editor marker.
            assert!(Shape::parse(data).is_err(), "{name}");
            eprintln!("skipping {name}: dts version {ver} unsupported");
            continue;
        }
        let s = Shape::parse(data).unwrap_or_else(|e| panic!("{name}: {e}"));
        parsed += 1;
        assert_eq!(s.version, 24, "{name}");
        assert!(s.checkpoints >= 15, "{name}");
        for (mi, m) in s.meshes.iter().enumerate() {
            for (pi, prim) in m.primitives.iter().enumerate() {
                if let Some(mat) = prim.material {
                    assert!(
                        (mat as usize) < s.materials.len(),
                        "{name} mesh {mi} prim {pi}: material {mat}"
                    );
                }
                assert!(
                    (prim.start + prim.num_elements) as usize <= m.indices.len(),
                    "{name} mesh {mi} prim {pi} out of index range"
                );
            }
            let nv = m.vertices.len() as u32;
            assert!(
                m.indices.iter().all(|&i| i < nv),
                "{name} mesh {mi}: index out of {nv} vertices"
            );
            if m.kind != MeshKind::Null && m.kind != MeshKind::Decal {
                assert_eq!(m.normals.len(), m.vertices.len(), "{name} mesh {mi}");
            }
        }
        // Geometry that is visible in the default state at detail 0.
        let has_geometry = s.detail_objects(0).any(|o| {
            s.objects[o].default_visibility > 0.0
                && s.detail_mesh(0, o)
                    .is_some_and(|m| !s.meshes[m].primitives.is_empty())
        });
        let g = s.detail0_geometry();
        if has_geometry {
            assert!(
                g.iter().map(|r| r.triangles.len()).sum::<usize>() > 0,
                "{name}: empty detail 0"
            );
        }
        for r in &g {
            assert!(r.material.is_none_or(|m| m < s.materials.len()));
            assert_eq!(r.positions.len(), r.normals.len());
            assert_eq!(r.positions.len(), r.uvs.len());
            assert!(r
                .triangles
                .iter()
                .flatten()
                .all(|&i| (i as usize) < r.positions.len()));
            assert!(
                r.positions.iter().flatten().all(|c| c.is_finite()),
                "{name}"
            );
            for n in &r.normals {
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!((0.9..1.1).contains(&l), "{name}: normal length {l}");
            }
        }
    }
    assert_eq!(parsed, 71);
}

/// The Moon props converted from CC0 glTF models by examples/gltf_to_dts.rs
/// (`assets/models/SOURCES.md`): triangles of the visible detail, height (m).
const MOON_PROPS: &[(&str, usize, f32)] = &[
    ("models/props/moon/lander.dts", 1464, 7.0),
    ("models/props/moon/landingpad.dts", 478, 2.4),
    ("models/props/moon/basemodule.dts", 732, 4.35),
    ("models/props/moon/radardish.dts", 972, 5.0),
    ("models/props/moon/solarpanel.dts", 812, 3.0),
    ("models/props/moon/boulder.dts", 196, 3.82),
    ("models/props/moon/boulder2.dts", 68, 2.55),
    ("models/props/moon/rockslab.dts", 44, 1.49),
    ("models/props/moon/rockspire.dts", 176, 5.0),
    ("models/props/moon/rubble.dts", 60, 0.28),
    ("models/props/moon/wreck_engine.dts", 300, 6.0),
    ("models/props/moon/wreck_tank.dts", 130, 5.0),
    ("models/props/moon/wreck_nose.dts", 50, 4.0),
    ("models/props/moon/wreck_sphere.dts", 192, 2.6),
    ("models/props/moon/wreck_pipe.dts", 80, 3.0),
    // tt1_5 "The Neverending Stream".
    ("models/props/green/rowboat.dts", 224, 0.98),
    ("models/props/green/driftlog.dts", 372, 1.56),
    // tt3_5 "The Dark Cortex"'s crashed starfighter.
    ("models/props/red/wreck_fighter.dts", 280, 6.86),
];

/// Each stands on its origin and has a solid `Collision-1` detail (the
/// extruded hull of its footprint) as tall as the shape.
#[test]
fn moon_props() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    for &(rel, tris, height) in MOON_PROPS {
        let s = Shape::parse(&std::fs::read(dir.join(rel)).unwrap()).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert_eq!(s.detail0_geometry().iter().map(|r| r.triangles.len()).sum::<usize>(), tris, "{rel}");
        assert!(s.bounds.min[2].abs() < 1e-4 && (s.bounds.max[2] - height).abs() < 0.01, "{rel}: {:?}", s.bounds);
        let c = s.detail_levels.iter().position(|d| d.name == "Collision-1").unwrap_or_else(|| panic!("{rel}: no collision"));
        let mesh = s.detail_objects(c).find_map(|o| s.detail_mesh(c, o)).expect("collision mesh");
        let top = s.meshes[mesh].vertices.iter().map(|v| v[2]).fold(0.0, f32::max);
        assert!((top - height).abs() < 0.01, "{rel}: collision top {top}");
    }
}

/// `Shape::base_points` of a turned, unevenly scaled shape placed as the
/// new maps' `Map::rest` places it (the highest foot on the ground, every
/// other under it), down taken through the transpose of rotation × scale:
/// a log (6.5 m along shape x) pitched 40 degrees with scale [1, 0.6, 0.65]
/// rests on its lower end, its lowest vertex half a metre under flat
/// ground (the inverse would take its underside along the whole length
/// for the base and bury that end 4 m).
#[test]
fn base_points_of_a_tilted_scaled_log() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let s = Shape::parse(&std::fs::read(dir.join("models/props/green/driftlog.dts")).unwrap()).unwrap();
    let scale = [1.0f32, 0.6, 0.65];
    let (sn, cs) = 40f32.to_radians().sin_cos();
    let r = [[cs, 0.0, sn], [0.0, 1.0, 0.0], [-sn, 0.0, cs]];
    let placed_z = |p: &[f32; 3]| (0..3).map(|k| r[2][k] * p[k] * scale[k]).sum::<f32>();
    let ground = s.base_points(std::array::from_fn(|k| -r[2][k] * scale[k])).iter().map(placed_z).fold(f32::MIN, f32::max);
    let lowest = s.detail0_geometry().iter().flat_map(|g| g.positions.clone()).map(|p| placed_z(&p)).fold(f32::MAX, f32::min);
    assert!(ground - lowest < 0.6, "the lowest vertex {} m under the ground", ground - lowest);
}

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
    // 48 of the game, plus OpenTT's mortarrocket.dts (examples/mortar_rocket.rs).
    assert_eq!(all.len(), 49);
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
    assert_eq!(parsed, 48);
}

#[test]
fn summary_table() {
    let all = shapes();
    let root = game();
    println!(
        "{:<36} {:>5} {:>6} {:>5} {:>6}  {:<44} materials",
        "shape", "nodes", "meshes", "mats", "tris0", "detail0 bbox min .. max"
    );
    for (p, _, data) in &all {
        let rel = p
            .strip_prefix(root.join("models"))
            .unwrap_or(p)
            .display()
            .to_string();
        let s = match Shape::parse(data) {
            Ok(s) => s,
            Err(e) => {
                println!("{rel:<36} ERROR {e}");
                continue;
            }
        };
        let g = s.detail0_geometry();
        let tris: usize = g.iter().map(|r| r.triangles.len()).sum();
        let mut b = tt_assets::dts::Bounds::EMPTY;
        g.iter()
            .flat_map(|r| &r.positions)
            .for_each(|&p| b.extend(p));
        let f = |v: [f32; 3]| format!("({:.2},{:.2},{:.2})", v[0], v[1], v[2]);
        let mats: Vec<&str> = s.materials.iter().map(|m| m.name.as_str()).collect();
        println!(
            "{rel:<36} {:>5} {:>6} {:>5} {:>6}  {:<44} {}",
            s.nodes.len(),
            s.meshes.len(),
            s.materials.len(),
            tris,
            if b.is_empty() {
                "-".into()
            } else {
                format!("{} .. {}", f(b.min), f(b.max))
            },
            mats.join(",")
        );
    }
}

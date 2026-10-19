//! Dumps the animation content of `.dts` shapes: every sequence (duration,
//! keyframes, cyclic / blend), the nodes it rotates / translates / scales
//! (marked `*` when the track actually changes over the sequence), the
//! objects whose visibility, IFL, vertex frame and material (UV) frame it
//! drives, plus the shape's IFL materials and objects hidden by default.
//!
//! ```sh
//! cargo run -p tt-assets --example dts_sequences -- $(find assets/models -name '*.dts' | sort)
//! ```

use tt_assets::dts::{Sequence, Shape};

fn changes<T: PartialEq + Copy>(keys: &[T], base: i32, k: usize, n: i32, eq: impl Fn(T, T) -> bool) -> bool {
    let start = base as usize + k * n as usize;
    let Some(track) = keys.get(start..start + n as usize) else { return false };
    track.windows(2).any(|w| !eq(w[0], w[1]))
}

fn close4(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

fn dump(path: &str, s: &Shape) {
    println!("{path}");
    let hidden: Vec<_> = s.objects.iter().filter(|o| o.default_visibility < 0.5).map(|o| format!("{}({:.2})", o.name, o.default_visibility)).collect();
    if !hidden.is_empty() {
        println!("  hidden by default: {}", hidden.join(" "));
    }
    for i in &s.ifl_materials {
        let mat = s.materials.get(i.material_slot).map(|m| m.name.as_str()).unwrap_or("?");
        // The frame fields are filled in at load time from the .ifl file.
        println!("  IFL {} (material {mat})", i.name);
    }
    for q in &s.sequences {
        let mut fl = Vec::new();
        if q.flags & Sequence::CYCLIC != 0 {
            fl.push("cyclic");
        }
        if q.flags & Sequence::BLEND != 0 {
            fl.push("blend");
        }
        if q.flags & Sequence::IFL_INIT != 0 {
            fl.push("ifl-init");
        }
        if q.num_ground_frames > 0 {
            fl.push("ground");
        }
        println!("  seq {:<14} {:>6.3}s {:>3} keys prio {} [{}] triggers {}", q.name, q.duration, q.num_keyframes, q.priority, fl.join(","), q.num_triggers);
        let n = q.num_keyframes;
        let node = |i: usize| s.nodes.get(i).map(|n| n.name.clone()).unwrap_or_else(|| "?".into());
        let obj = |i: usize| s.objects.get(i).map(|o| o.name.clone()).unwrap_or_else(|| "?".into());
        let rot: Vec<_> = q
            .rotation_matters
            .iter()
            .enumerate()
            .map(|(k, &i)| format!("{}{}", node(i), if changes(&s.node_rotations, q.base_rotation, k, n, close4) { "*" } else { "" }))
            .collect();
        let tra: Vec<_> = q
            .translation_matters
            .iter()
            .enumerate()
            .map(|(k, &i)| format!("{}{}", node(i), if changes(&s.node_translations, q.base_translation, k, n, close3) { "*" } else { "" }))
            .collect();
        // Scale tracks with their range (first..last keyframe, uniform /
        // aligned scales: the x factor).
        let sca: Vec<_> = q
            .scale_matters
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let start = q.base_scale.max(0) as usize + k * n as usize;
                let xs: Vec<f32> = if q.flags & Sequence::UNIFORM_SCALE != 0 {
                    s.node_uniform_scales.get(start..start + n as usize).map(|v| v.to_vec()).unwrap_or_default()
                } else if q.flags & Sequence::ALIGNED_SCALE != 0 {
                    s.node_aligned_scales.get(start..start + n as usize).map(|v| v.iter().map(|a| a[0]).collect()).unwrap_or_default()
                } else {
                    s.node_arbitrary_scale_factors.get(start..start + n as usize).map(|v| v.iter().map(|a| a[0]).collect()).unwrap_or_default()
                };
                match (xs.first(), xs.last()) {
                    (Some(a), Some(b)) => format!("{}[{a:.2}->{b:.2}]", node(i)),
                    _ => node(i),
                }
            })
            .collect();
        let vis: Vec<_> = q
            .vis_matters
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let start = q.base_object_state as usize + k * n as usize;
                let v = s.object_states.get(start..start + n as usize).unwrap_or(&[]);
                let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
                format!("{}[{lo:.2}..{hi:.2}]", obj(i))
            })
            .collect();
        // `ifl_matters` indexes the IFL materials, not objects.
        let ifl = |i: usize| s.ifl_materials.get(i).map(|m| m.name.clone()).unwrap_or_else(|| "?".into());
        let list = |v: &[usize], f: &dyn Fn(usize) -> String| v.iter().map(|&i| f(i)).collect::<Vec<_>>().join(" ");
        for (what, v) in [("rot", rot.join(" ")), ("trans", tra.join(" ")), ("scale", sca.join(" ")), ("vis", vis.join(" ")), ("ifl", list(&q.ifl_matters, &ifl)), ("frame", list(&q.frame_matters, &obj)), ("matframe", list(&q.mat_frame_matters, &obj))] {
            if !v.is_empty() {
                println!("      {what:<8} {v}");
            }
        }
    }
}

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(data) = std::fs::read(&path) else {
            eprintln!("{path}: unreadable");
            continue;
        };
        match Shape::parse(&data) {
            Ok(s) => dump(&path, &s),
            Err(e) => eprintln!("{path}: {e:?}"),
        }
    }
}

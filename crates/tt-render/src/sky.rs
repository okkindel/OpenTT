//! Sky box seams.
//!
//! The sky is five faces (`.dml` entries 0-3: the side panorama, north,
//! east, south, west; 4: the top) drawn on a cube around the camera
//! ([`crate::scene`]'s `sky_cube`). Each face is sampled with clamped edges,
//! so a seam shows wherever the texels on both sides of a shared cube edge
//! differ. [`match_seams`] removes those steps from the decoded faces: along
//! every shared edge the faces are pulled towards their common mean, a
//! smooth (along the edge) correction fading out over a wide band plus the
//! remaining texel-level difference over a few texels, so both edge texel
//! rows end up identical.

/// Which border of a face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Border {
    Left,
    Right,
    Top,
    Bottom,
}

use Border::*;

/// The top face's index.
pub(crate) const TOP: usize = 4;

/// The cube's shared edges: (face, border, face, border, reversed). Position
/// `t` along the first face's border (top to bottom, left to right) meets
/// position `t` (or `n - 1 - t` when reversed) along the second's. Sides go
/// clockwise seen from above, so each side's right border meets the next
/// one's left border; the top face lies with north up (its row 0 against
/// the north face's row 0, east on its right).
pub(crate) const EDGES: [(usize, Border, usize, Border, bool); 8] = [
    (0, Right, 1, Left, false),
    (1, Right, 2, Left, false),
    (2, Right, 3, Left, false),
    (3, Right, 0, Left, false),
    (0, Top, TOP, Top, false),
    (1, Top, TOP, Right, false),
    (2, Top, TOP, Bottom, true),
    (3, Top, TOP, Left, true),
];

/// Texel index (`x + y * n`) at position `t` along `border`, `k` texels in.
pub(crate) fn texel(n: usize, border: Border, t: usize, k: usize) -> usize {
    let (x, y) = match border {
        Left => (k, t),
        Right => (n - 1 - k, t),
        Top => (t, k),
        Bottom => (t, n - 1 - k),
    };
    x + y * n
}

/// Makes the five `n`² RGBA8 faces meet without a step along every shared
/// cube edge (see the module docs). Alpha is left alone; other face counts
/// are left untouched.
pub fn match_seams(faces: &mut [Vec<u8>], n: usize) {
    if faces.len() < 5 || n < 8 || faces[..5].iter().any(|f| f.len() != n * n * 4) {
        return;
    }
    let mut f: Vec<Vec<f32>> = faces[..5].iter().map(|f| f.iter().map(|&b| b as f32).collect()).collect();
    // Wide band (smooth correction) and narrow band (texel residual).
    let wide = (n / 16).max(2);
    let narrow = (n / 128).max(2);
    // Smoothing radius of the wide correction along the edge.
    let radius = wide / 2;
    // Corrections near the cube's corners touch two edges of a face; a few
    // rounds settle them.
    for _ in 0..4 {
        for &(fa, ba, fb, bb, rev) in &EDGES {
            let at_b = |t: usize| if rev { n - 1 - t } else { t };
            for c in 0..3 {
                // Half the difference across the edge, per position.
                let half: Vec<f32> = (0..n).map(|t| (f[fb][texel(n, bb, at_b(t), 0) * 4 + c] - f[fa][texel(n, ba, t, 0) * 4 + c]) / 2.0).collect();
                let smooth = box_filter(&half, radius);
                for t in 0..n {
                    for k in 0..wide {
                        let w = 0.5 * (1.0 + (std::f32::consts::PI * k as f32 / wide as f32).cos());
                        f[fa][texel(n, ba, t, k) * 4 + c] += smooth[t] * w;
                        f[fb][texel(n, bb, at_b(t), k) * 4 + c] -= smooth[t] * w;
                    }
                }
                // What is left: texel-level differences, over a few texels.
                for t in 0..n {
                    let (ia, ib) = (texel(n, ba, t, 0) * 4 + c, texel(n, bb, at_b(t), 0) * 4 + c);
                    let rest = (f[fb][ib] - f[fa][ia]) / 2.0;
                    for k in 0..narrow {
                        let w = 1.0 - k as f32 / narrow as f32;
                        f[fa][texel(n, ba, t, k) * 4 + c] += rest * w;
                        f[fb][texel(n, bb, at_b(t), k) * 4 + c] -= rest * w;
                    }
                }
            }
        }
    }
    for (out, f) in faces.iter_mut().zip(&f) {
        for (i, (o, v)) in out.iter_mut().zip(f).enumerate() {
            if i % 4 != 3 {
                *o = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Mean of `v` over `[t - r, t + r]` (clamped to the ends) at every `t`.
fn box_filter(v: &[f32], r: usize) -> Vec<f32> {
    let mut sum = vec![0.0f64; v.len() + 1];
    for (i, x) in v.iter().enumerate() {
        sum[i + 1] = sum[i] + *x as f64;
    }
    (0..v.len())
        .map(|t| {
            let (lo, hi) = (t.saturating_sub(r), (t + r + 1).min(v.len()));
            ((sum[hi] - sum[lo]) / (hi - lo) as f64) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// Largest channel difference across the cube's shared edges.
    fn worst_seam(faces: &[Vec<u8>], n: usize) -> (u8, f32) {
        let (mut max, mut sum, mut count) = (0u8, 0.0f32, 0usize);
        for &(fa, ba, fb, bb, rev) in &EDGES {
            for t in 0..n {
                let tb = if rev { n - 1 - t } else { t };
                for c in 0..3 {
                    let d = faces[fa][texel(n, ba, t, 0) * 4 + c].abs_diff(faces[fb][texel(n, bb, tb, 0) * 4 + c]);
                    max = max.max(d);
                    sum += d as f32;
                    count += 1;
                }
            }
        }
        (max, sum / count as f32)
    }

    /// The direction of texel centre (`x`, `y`) of `face`, from the drawn
    /// cube's vertices (bilinear over the face's quad).
    fn face_dir(face: usize, n: usize, x: usize, y: usize) -> Vec3 {
        let v = crate::scene::sky_cube();
        let quad = &v[face * 6..face * 6 + 6];
        let (u, w) = ((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32);
        let corner = |cu: f32, cv: f32| Vec3::from(quad.iter().find(|p| p.uv == [cu, cv]).expect("quad corner").dir);
        corner(0.0, 0.0) * (1.0 - u) * (1.0 - w) + corner(1.0, 0.0) * u * (1.0 - w) + corner(0.0, 1.0) * (1.0 - u) * w + corner(1.0, 1.0) * u * w
    }

    #[test]
    fn edge_table_matches_the_drawn_cube() {
        // Texel centres facing each other across an edge lie half a texel
        // from it on either side: on the cube they are 1/n apart.
        let n = 16;
        for &(fa, ba, fb, bb, rev) in &EDGES {
            for t in 0..n {
                let ia = texel(n, ba, t, 0);
                let ib = texel(n, bb, if rev { n - 1 - t } else { t }, 0);
                let a = face_dir(fa, n, ia % n, ia / n);
                let b = face_dir(fb, n, ib % n, ib / n);
                assert!((a - b).length() < 1.5 / n as f32, "{fa} {ba:?} / {fb} {bb:?} at {t}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn seams_are_closed_and_the_rest_kept() {
        // Smooth random faces, each with its own brightness offset (like
        // faces upscaled one by one).
        let n = 64;
        let mut seed = 7u32;
        let mut faces: Vec<Vec<u8>> = (0..5)
            .map(|face| {
                (0..n * n * 4)
                    .map(|i| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        let (x, y) = ((i / 4) % n, (i / 4) / n);
                        (100.0 + face as f32 * 6.0 + ((x + y) as f32 * 0.2).sin() * 20.0 + (seed >> 29) as f32) as u8
                    })
                    .collect()
            })
            .collect();
        let before = faces.clone();
        assert!(worst_seam(&faces, n).0 > 10);
        match_seams(&mut faces, n);
        assert!(worst_seam(&faces, n).0 <= 1, "{:?}", worst_seam(&faces, n));
        // Face centres stay as they were.
        for (a, b) in faces.iter().zip(&before) {
            let centre = texel(n, Left, n / 2, n / 2) * 4;
            assert_eq!(a[centre..centre + 4], b[centre..centre + 4]);
        }
    }

    /// The shipped skies: their `.dml` face order fits the cube (each side
    /// continues into the next one clearly better than into any other), and
    /// the loaded faces meet without a step.
    #[test]
    fn shipped_skies_meet_at_every_edge() {
        let root = tt_assets::layout::default_dir();
        let n = crate::textures::size() as usize;
        for dml in ["sky/skyblue.dml", "sky/skyred.dml", "sky/sky_day.dml"] {
            let Some(bases) = crate::scene::sky_bases(&root, &root, dml) else {
                eprintln!("skipped {dml}: not found");
                continue;
            };
            let mut faces: Vec<Vec<u8>> = bases.iter().map(|b| crate::scene::load_material(b)).collect();
            let col_diff = |a: usize, b: usize| -> f32 {
                (0..n).map(|y| (0..3).map(|c| faces[a][((n - 1) + y * n) * 4 + c].abs_diff(faces[b][(y * n) * 4 + c]) as f32).sum::<f32>()).sum::<f32>() / (3 * n) as f32
            };
            for i in 0..4 {
                let next = col_diff(i, (i + 1) % 4);
                for j in (0..4).filter(|&j| j != i && j != (i + 1) % 4) {
                    assert!(next * 2.0 < col_diff(i, j), "{dml}: face {i} -> {} ({next}) vs {j} ({})", (i + 1) % 4, col_diff(i, j));
                }
            }
            let (raw_max, raw_mean) = worst_seam(&faces, n);
            match_seams(&mut faces, n);
            let (max, mean) = worst_seam(&faces, n);
            eprintln!("{dml}: seam max {raw_max} mean {raw_mean:.2} -> max {max} mean {mean:.2}");
            assert!(max <= 1, "{dml}: seam {max}");
        }
    }
}

//! Generates the Mortar power-up's rocket: `mortarrocket.dts` (a small
//! procedural mesh: a body cylinder, an ogive nose, a nozzle bell and four
//! swept fins, nose along +Y, the way `.dts` projectiles fly) with its two
//! textures, `mortarrocket.png` (a flat palette: olive-drab body, gunmetal
//! nose, darker fins, a near-black nozzle) and `mortarrockethot.png` (the
//! self-illuminated nozzle mouth). Writes a minimal DTS v24 (one node, object,
//! mesh and detail level, no sequences) and parses it back.
//!
//! ```sh
//! cargo run -p tt-assets --example mortar_rocket -- assets/models/projectiles
//! ```

use std::f32::consts::TAU;
use std::path::PathBuf;
use tt_assets::dts::{Material, Shape};

#[path = "shared/dts_writer.rs"]
mod dts_writer;
use dts_writer::{write_dts, Mesh};

/// Sides around the axis.
const SIDES: usize = 14;
/// Palette cells (u centre of each 1/4 band of `mortarrocket.png`).
const BODY: f32 = 0.125;
const FIN: f32 = 0.375;
const NOSE: f32 = 0.625;
const NOZZLE: f32 = 0.875;
const PALETTE: [[u8; 3]; 4] = [[92, 96, 72], [58, 61, 50], [74, 78, 82], [34, 34, 36]];

impl Mesh {
    fn vert(&mut self, p: [f32; 3], n: [f32; 3], u: f32) -> u16 {
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        self.verts.push(p);
        self.normals.push([n[0] / l, n[1] / l, n[2] / l]);
        self.uvs.push([u, 0.5]);
        (self.verts.len() - 1) as u16
    }

    /// A surface of revolution through `(y, r)` points, smooth around,
    /// creased between rings (each band its own vertices).
    fn lathe(&mut self, profile: &[(f32, f32)], u: f32) {
        for w in profile.windows(2) {
            let ((y0, r0), (y1, r1)) = (w[0], w[1]);
            // Outward normal of the band's slant (r grows with y: tilts back).
            let (dy, dr) = (y1 - y0, r1 - r0);
            let (ny, nr) = (-dr, dy);
            let ring = |m: &mut Mesh, y: f32, r: f32| -> Vec<u16> {
                (0..=SIDES)
                    .map(|i| {
                        let a = i as f32 / SIDES as f32 * TAU;
                        let (s, c) = a.sin_cos();
                        m.vert([r * c, y, r * s], [nr * c, ny, nr * s], u)
                    })
                    .collect()
            };
            let (a, b) = (ring(self, y0, r0), ring(self, y1, r1));
            for i in 0..SIDES {
                self.tris[0].push([a[i], b[i], b[i + 1]]);
                self.tris[0].push([a[i], b[i + 1], a[i + 1]]);
            }
        }
    }

    /// A flat disc at `y` facing `ny` (material `mat`).
    fn disc(&mut self, y: f32, r: f32, ny: f32, u: f32, mat: usize) {
        let c = self.vert([0.0, y, 0.0], [0.0, ny, 0.0], u);
        let ring: Vec<u16> = (0..=SIDES)
            .map(|i| {
                let (s, co) = (i as f32 / SIDES as f32 * TAU).sin_cos();
                self.vert([r * co, y, r * s], [0.0, ny, 0.0], u)
            })
            .collect();
        for i in 0..SIDES {
            self.tris[mat].push([c, ring[i], ring[i + 1]]);
        }
    }

    /// A thin swept fin at angle `a` around the axis: root `y0..y1` at
    /// radius `r0`, tip `t0..t1` at radius `r1`, `th` thick.
    #[allow(clippy::too_many_arguments)]
    fn fin(&mut self, a: f32, r0: f32, y0: f32, y1: f32, r1: f32, t0: f32, t1: f32, th: f32) {
        let (s, c) = a.sin_cos();
        let out = [c, 0.0, s];
        let side = [-s, 0.0, c];
        let p = |r: f32, y: f32, o: f32| [out[0] * r + side[0] * o, y, out[2] * r + side[2] * o];
        for sign in [-1.0f32, 1.0] {
            let o = sign * th * 0.5;
            let n = [side[0] * sign, 0.0, side[2] * sign];
            let q = [p(r0, y0, o), p(r0, y1, o), p(r1, t1, o), p(r1, t0, o)].map(|v| self.vert(v, n, FIN));
            self.tris[0].push([q[0], q[1], q[2]]);
            self.tris[0].push([q[0], q[2], q[3]]);
        }
        // The outer edge, so the fin is not paper thin edge-on.
        let q = [p(r1, t0, -th * 0.5), p(r1, t1, -th * 0.5), p(r1, t1, th * 0.5), p(r1, t0, th * 0.5)].map(|v| self.vert(v, out, FIN));
        self.tris[0].push([q[0], q[1], q[2]]);
        self.tris[0].push([q[0], q[2], q[3]]);
    }
}

fn rocket() -> Mesh {
    let mut m = Mesh::default();
    // 2 m long, 0.26 m body, fins 0.76 m across: nose at y = +1.
    let r = 0.13;
    m.lathe(&[(-0.80, r), (0.42, r)], BODY);
    let nose: Vec<(f32, f32)> = (0..=6)
        .map(|i| {
            let t = i as f32 / 6.0;
            (0.42 + t * 0.58, r * (1.0 - t * t).max(0.0).sqrt().powf(0.9))
        })
        .collect();
    m.lathe(&nose, NOSE);
    // Nozzle bell flaring out behind the body, its mouth glowing.
    m.disc(-0.80, r, -1.0, NOZZLE, 0);
    m.lathe(&[(-0.95, 0.105), (-0.80, 0.075)], NOZZLE);
    m.disc(-0.94, 0.098, -1.0, 0.5, 1);
    for k in 0..4 {
        let a = (k as f32 + 0.5) * TAU / 4.0;
        m.fin(a, r - 0.01, -0.78, -0.30, 0.38, -0.86, -0.64, 0.025);
    }
    m
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: mortar_rocket OUT_DIR"));
    let m = rocket();
    let flags = Material::S_WRAP | Material::T_WRAP | Material::NEVER_ENV_MAP;
    let dts = write_dts(&m, "Rocket", &[("MortarRocket", flags), ("MortarRocketHot", flags | Material::SELF_ILLUMINATING)], false);
    let shape = Shape::parse(&dts).expect("the written shape parses");
    let tris: usize = shape.detail0_geometry().iter().map(|r| r.triangles.len()).sum();
    std::fs::write(dir.join("mortarrocket.dts"), &dts).unwrap();
    // Palette: four vertical bands.
    let pal = image::RgbImage::from_fn(64, 64, |x, _| image::Rgb(PALETTE[(x / 16) as usize]));
    pal.save(dir.join("mortarrocket.png")).unwrap();
    // The nozzle mouth: white-hot centre to orange at the rim (u = 0.5 in
    // the middle, the disc uses one colour; kept as a gradient for mips).
    let hot = image::RgbImage::from_fn(64, 64, |x, y| {
        let d = (((x as f32 - 31.5) / 32.0).powi(2) + ((y as f32 - 31.5) / 32.0).powi(2)).sqrt().min(1.0);
        let lerp = |a: f32, b: f32| (a + (b - a) * d) as u8;
        image::Rgb([255, lerp(236.0, 150.0), lerp(170.0, 40.0)])
    });
    hot.save(dir.join("mortarrockethot.png")).unwrap();
    println!("mortarrocket.dts: {} vertices, {tris} triangles, radius {:.2} m, {} bytes", m.verts.len(), shape.radius, dts.len());
}

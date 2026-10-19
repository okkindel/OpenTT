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

/// Sides around the axis.
const SIDES: usize = 14;
/// Palette cells (u centre of each 1/4 band of `mortarrocket.png`).
const BODY: f32 = 0.125;
const FIN: f32 = 0.375;
const NOSE: f32 = 0.625;
const NOZZLE: f32 = 0.875;
const PALETTE: [[u8; 3]; 4] = [[92, 96, 72], [58, 61, 50], [74, 78, 82], [34, 34, 36]];

#[derive(Default)]
struct Mesh {
    verts: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    /// Triangles per material.
    tris: [Vec<[u16; 3]>; 2],
}

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

struct Streams {
    s32: Vec<u8>,
    s16: Vec<u8>,
    s8: Vec<u8>,
    guard: u32,
}

impl Streams {
    fn u32(&mut self, v: u32) {
        self.s32.extend(v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn vec3(&mut self, v: [f32; 3]) {
        v.iter().for_each(|&x| self.f32(x));
    }
    fn u16(&mut self, v: u16) {
        self.s16.extend(v.to_le_bytes());
    }
    fn u8(&mut self, v: u8) {
        self.s8.push(v);
    }
    fn guard(&mut self) {
        let g = self.guard;
        self.u32(g);
        self.u16(g as u16);
        self.u8(g as u8);
        self.guard += 1;
    }
}

fn write_dts(m: &Mesh, materials: &[(&str, u32)]) -> Vec<u8> {
    let names = ["Rocket", "detail2"];
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &m.verts {
        for k in 0..3 {
            min[k] = min[k].min(v[k]);
            max[k] = max[k].max(v[k]);
        }
    }
    let center = [0.0; 3];
    let radius = m.verts.iter().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).fold(0.0, f32::max);
    let tube = m.verts.iter().map(|v| (v[0] * v[0] + v[2] * v[2]).sqrt()).fold(0.0, f32::max);
    let mut s = Streams { s32: Vec::new(), s16: Vec::new(), s8: Vec::new(), guard: 0 };
    // Counts: nodes, objects, decals, sub-shapes, ifl, node rots, node
    // translations, uniform / aligned / arbitrary scales, ground frames,
    // object states, decal states, triggers, details, meshes, names.
    for n in [1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, names.len() as i32] {
        s.i32(n);
    }
    s.i32(0); // smallest visible size
    s.i32(0); // smallest visible detail level
    s.guard();
    s.f32(radius);
    s.f32(tube);
    s.vec3(center);
    s.vec3(min);
    s.vec3(max);
    s.guard();
    // Node: name, parent, first object, first child, next sibling.
    for v in [0, -1, -1, -1, -1] {
        s.i32(v);
    }
    s.guard();
    // Object: name, meshes, first mesh, node, next sibling, first decal.
    for v in [0, 1, 0, 0, -1, -1] {
        s.i32(v);
    }
    s.guard();
    s.guard(); // decals
    s.guard(); // ifl materials
    for v in [0, 0, 0] {
        s.i32(v); // sub-shape first node / object / decal
    }
    s.guard();
    for v in [1, 1, 0] {
        s.i32(v); // sub-shape node / object / decal counts
    }
    s.guard();
    for q in [0u16, 0, 0, 32767] {
        s.u16(q); // default rotation (identity)
    }
    s.vec3([0.0; 3]); // default translation
    s.guard(); // node transforms
    s.guard(); // scales
    s.guard(); // ground frames
    s.f32(1.0);
    s.i32(0);
    s.i32(0);
    s.guard(); // object states
    s.guard(); // decal states
    s.guard(); // triggers
    let polys: usize = m.tris.iter().map(Vec::len).sum();
    s.i32(1); // detail name "detail2"
    s.i32(0); // sub-shape
    s.i32(0); // object detail
    s.f32(2.0); // size
    s.f32(-1.0);
    s.f32(-1.0);
    s.i32(polys as i32);
    s.guard();
    // The mesh.
    s.u32(0); // standard
    s.guard();
    s.u32(1); // frames
    s.u32(1); // material frames
    s.i32(-1); // parent
    s.vec3(min);
    s.vec3(max);
    s.vec3(center);
    s.f32(radius);
    s.i32(m.verts.len() as i32);
    m.verts.iter().for_each(|&v| s.vec3(v));
    s.i32(m.uvs.len() as i32);
    m.uvs.iter().for_each(|&[u, v]| {
        s.f32(u);
        s.f32(v)
    });
    m.normals.iter().for_each(|&n| s.vec3(n));
    m.normals.iter().for_each(|_| s.u8(0));
    let prims: Vec<(usize, &Vec<[u16; 3]>)> = m.tris.iter().enumerate().filter(|(_, t)| !t.is_empty()).collect();
    s.i32(prims.len() as i32);
    let mut start = 0;
    for (_, t) in &prims {
        s.u16(start as u16);
        s.u16((t.len() * 3) as u16);
        start += t.len() * 3;
    }
    for (mat, _) in &prims {
        s.u32(0x2000_0000 | *mat as u32); // indexed triangles
    }
    s.i32(start as i32);
    for (_, t) in &prims {
        t.iter().flatten().for_each(|&i| s.u16(i));
    }
    s.i32(0); // merge indices
    s.u32(m.verts.len() as u32); // vertices per frame
    s.u32(0); // flags
    s.guard();
    s.guard(); // meshes
    for n in names {
        n.bytes().for_each(|b| s.u8(b));
        s.u8(0);
    }
    s.guard();
    while !s.s16.len().is_multiple_of(4) {
        s.s16.push(0);
    }
    while !s.s8.len().is_multiple_of(4) {
        s.s8.push(0);
    }
    let (w32, w16, w8) = (s.s32.len() / 4, s.s16.len() / 4, s.s8.len() / 4);
    let mut out = Vec::new();
    out.extend(24u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(((w32 + w16 + w8) as u32).to_le_bytes());
    out.extend((w32 as u32).to_le_bytes());
    out.extend(((w32 + w16) as u32).to_le_bytes());
    out.extend(&s.s32);
    out.extend(&s.s16);
    out.extend(&s.s8);
    out.extend(0u32.to_le_bytes()); // sequences
    out.push(1); // material list version
    out.extend((materials.len() as u32).to_le_bytes());
    for (name, _) in materials {
        out.push(name.len() as u8);
        out.extend(name.bytes());
    }
    let col = |out: &mut Vec<u8>, f: &dyn Fn(u32) -> u32| materials.iter().for_each(|&(_, fl)| out.extend(f(fl).to_le_bytes()));
    col(&mut out, &|fl| fl); // flags
    for _ in 0..3 {
        col(&mut out, &|_| u32::MAX); // reflectance / bump / detail maps
    }
    col(&mut out, &|_| 1.0f32.to_bits()); // detail scale
    col(&mut out, &|_| 0.0f32.to_bits()); // reflection amount
    out
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: mortar_rocket OUT_DIR"));
    let m = rocket();
    let flags = Material::S_WRAP | Material::T_WRAP | Material::NEVER_ENV_MAP;
    let dts = write_dts(&m, &[("MortarRocket", flags), ("MortarRocketHot", flags | Material::SELF_ILLUMINATING)]);
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

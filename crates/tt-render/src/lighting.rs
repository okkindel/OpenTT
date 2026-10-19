//! Terrain: the lightmap is walked in the direction the light travels, every
//! texel is super-sampled 2×2, and a running "shadow height" carried along the
//! ray gives the terrain self-shadowing. Direct light is `max(N·L, 0) ·
//! colour`. Shadows of static shapes then scale the direct light by the
//! unshadowed fraction of each texel. The finish adds ambient, clamps, applies
//! a 2×2 box blur and quantises every channel to 5 bits.
//!
//! The terrain shader multiplies the texture by the lightmap.

use glam::{Vec2, Vec3};
use tt_assets::terrain::Terrain;

/// `1 << lightShift`. Missions don't override the engine default 9.
pub const LIGHTMAP_SIZE: usize = 1 << 9;
/// As the game scripts set it (1 = 2×2 samples per lightmap texel).
const GENERATE_LEVEL: u32 = 1;
/// Sub-samples per texel edge when measuring how much of a texel a shape
/// shadow covers (the engine clips polygons exactly; 4×4 points is close).
const COVERAGE_SAMPLES: usize = 4;
/// Length of the rays cast from static shapes into the terrain.
const PROBE_LENGTH: f32 = 250.0;

/// The mission's `Sun` after `conform`: unit travel direction, colours
/// clamped to [0, 1].
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    /// Direction the light travels (from the sun towards the ground).
    pub direction: Vec3,
    pub color: Vec3,
    pub ambient: Vec3,
}

impl Sun {
    pub fn new(direction: Vec3, color: Vec3, ambient: Vec3) -> Sun {
        let d = direction.try_normalize().unwrap_or(Vec3::new(0.0, std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2));
        Sun { direction: d, color: color.clamp(Vec3::ZERO, Vec3::ONE), ambient: ambient.clamp(Vec3::ZERO, Vec3::ONE) }
    }

    /// Direction used by the terrain light pass: a vertical sun is tilted to
    /// `dx = dy = 0.01` so the scan has a major axis.
    fn scan_direction(&self) -> Vec3 {
        let mut d = self.direction;
        if d.x.abs() < 0.01 && d.y.abs() < 0.01 {
            d.x = 0.01;
            d.y = 0.01;
        }
        d
    }
}

/// A static shadow caster: world-space triangles and their bounds.
pub struct Caster {
    pub tris: Vec<[Vec3; 3]>,
    pub min: Vec3,
    pub max: Vec3,
}

impl Caster {
    pub fn new(tris: Vec<[Vec3; 3]>) -> Caster {
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in tris.iter().flatten() {
            min = min.min(*p);
            max = max.max(*p);
        }
        Caster { tris, min, max }
    }

    /// Whether the ray `o + t·d` (t > `t_min`) hits any triangle.
    fn hit(&self, o: Vec3, d: Vec3, t_min: f32) -> bool {
        if !ray_box(o, d, self.min, self.max) {
            return false;
        }
        self.tris.iter().any(|t| ray_triangle(o, d, t).is_some_and(|h| h > t_min))
    }
}

/// Terrain lightmap: `size`² texels covering one terrain block; texel `i`
/// spans grid coordinates `[i, i+1) · terrain_size / size`.
pub struct Lightmap {
    pub size: usize,
    /// Final colour per texel (`q / 32`, q the 5-bit channel value).
    pub texels: Vec<Vec3>,
    /// Terrain size in squares and metres per square (to map world positions).
    terrain_size: usize,
    square: f32,
    origin: f32,
}

impl Lightmap {
    /// A uniform lightmap (for scenes without terrain lighting).
    pub(crate) fn uniform(color: Vec3, terrain_size: usize, square: f32, origin: f32) -> Lightmap {
        Lightmap { size: 2, texels: vec![color; 4], terrain_size, square, origin }
    }

    /// Bilinear lightmap colour at a world position (the block repeats).
    pub fn sample(&self, world: Vec2) -> Vec3 {
        let per_square = self.size as f32 / self.terrain_size as f32;
        let p = (world - Vec2::splat(self.origin)) / self.square * per_square - Vec2::splat(0.5);
        let (x0, y0) = (p.x.floor(), p.y.floor());
        let (fx, fy) = (p.x - x0, p.y - y0);
        let at = |dx: i64, dy: i64| {
            let s = self.size as i64;
            self.texels[((y0 as i64 + dy).rem_euclid(s) * s + (x0 as i64 + dx).rem_euclid(s)) as usize]
        };
        (at(0, 0) * (1.0 - fx) + at(1, 0) * fx) * (1.0 - fy) + (at(0, 1) * (1.0 - fx) + at(1, 1) * fx) * fy
    }

    /// RGBA8 texels for an sRGB texture: the engine modulates 8-bit display
    /// values, so decoding the factor to linear before multiplying the
    /// (linear) texture reproduces `texture · q / 32` on screen.
    pub fn rgba8(&self) -> Vec<u8> {
        self.texels
            .iter()
            .flat_map(|c| {
                let b = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                [b(c.x), b(c.y), b(c.z), 255]
            })
            .collect()
    }
}

/// Bilinear height in metres at fractional grid coordinates (wrapping).
pub(crate) fn height_at(t: &Terrain, gx: f32, gy: f32) -> f32 {
    let (x0, y0) = (gx.floor(), gy.floor());
    let (fx, fy) = (gx - x0, gy - y0);
    let (x, y) = (x0 as i64, y0 as i64);
    let h = |dx, dy| t.height(x + dx, y + dy);
    (h(0, 0) * (1.0 - fx) + h(1, 0) * fx) * (1.0 - fy) + (h(0, 1) * (1.0 - fx) + h(1, 1) * fx) * fy
}

/// Grid-point normal from central differences (same as the render mesh).
pub(crate) fn grid_normal(t: &Terrain, square: f32, x: i64, y: i64) -> Vec3 {
    let dx = t.height(x + 1, y) - t.height(x - 1, y);
    let dy = t.height(x, y + 1) - t.height(x, y - 1);
    Vec3::new(-dx, -dy, 2.0 * square).normalize()
}

/// Everything the bake needs about the terrain block.
pub struct TerrainInfo<'a> {
    pub terrain: &'a Terrain,
    pub square: f32,
    /// World coordinate of grid point 0 on both axes.
    pub origin: f32,
    /// Terrain `tile` flag (0 in every shipped mission).
    pub tile: bool,
}

impl TerrainInfo<'_> {
    fn grid(&self, world: Vec2) -> Vec2 {
        (world - Vec2::splat(self.origin)) / self.square
    }

    fn height(&self, world: Vec2) -> f32 {
        let g = self.grid(world);
        height_at(self.terrain, g.x, g.y)
    }

    /// First terrain hit of `o + t·d` for t in (0, `len`], marching in 1 m
    /// steps and refining by bisection.
    fn raycast(&self, o: Vec3, d: Vec3, len: f32) -> Option<Vec3> {
        let below = |p: Vec3| p.z <= self.height(p.truncate());
        if below(o) {
            return Some(o);
        }
        let mut t0 = 0.0;
        let mut t = 1.0f32;
        while t <= len {
            if below(o + d * t) {
                let (mut a, mut b) = (t0, t);
                for _ in 0..12 {
                    let m = 0.5 * (a + b);
                    if below(o + d * m) {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                return Some(o + d * b);
            }
            t0 = t;
            t += 1.0;
        }
        None
    }
}

/// Direct sun light per lightmap texel with terrain self-shadowing.
fn direct_light(ti: &TerrainInfo, sun: &Sun, size: usize) -> Vec<Vec3> {
    let t = ti.terrain;
    let n = 1usize << GENERATE_LEVEL;
    let m = size * n; // samples per edge
    let ts = t.size();
    let samples_per_square = m as f32 / ts as f32;
    let spacing = ti.square / samples_per_square; // metres between samples

    let normals: Vec<Vec3> = (0..ts * ts).map(|i| grid_normal(t, ti.square, (i % ts) as i64, (i / ts) as i64)).collect();
    let normal_at = |gx: f32, gy: f32| {
        let (x0, y0) = (gx.floor(), gy.floor());
        let (fx, fy) = (gx - x0, gy - y0);
        let at = |dx: i64, dy: i64| normals[((y0 as i64 + dy).rem_euclid(ts as i64) * ts as i64 + (x0 as i64 + dx).rem_euclid(ts as i64)) as usize];
        ((at(0, 0) * (1.0 - fx) + at(1, 0) * fx) * (1.0 - fy) + (at(0, 1) * (1.0 - fx) + at(1, 1) * fx) * fy).normalize_or_zero()
    };

    let d = sun.scan_direction();
    let major_x = d.x.abs() >= d.y.abs();
    let (d_major, d_minor) = if major_x { (d.x, d.y) } else { (d.y, d.x) };
    // Shadow height change and minor-axis drift per sample along the ray.
    let z_step = d.z / d_major.abs() * spacing;
    let minor_step = d_minor / d_major.abs();

    let mut accum = vec![Vec3::ZERO; size * size];
    // Running shadow heights of the previous row (indexed by minor axis).
    let mut prev = vec![-1e11f32; m];
    let mut cur = vec![0.0f32; m];
    // `tile` set: one wrapped pass across the block primes the heights.
    let passes = if ti.tile { 2 } else { 1 };
    for pass in 0..passes {
        let light = pass + 1 == passes;
        for k in 0..m {
            let u = if d_major > 0.0 { k } else { m - 1 - k };
            for (v, slot) in cur.iter_mut().enumerate() {
                let src = v as f32 - minor_step;
                let i0 = src.floor();
                let f = src - i0;
                let a = prev[(i0 as i64).rem_euclid(m as i64) as usize];
                let b = prev[(i0 as i64 + 1).rem_euclid(m as i64) as usize];
                let h_shadow = a + (b - a) * f + z_step;
                let (sx, sy) = if major_x { (u, v) } else { (v, u) };
                let gx = (sx as f32 + 0.5) / samples_per_square;
                let gy = (sy as f32 + 0.5) / samples_per_square;
                let h = height_at(t, gx, gy);
                if h < h_shadow {
                    *slot = h_shadow;
                } else {
                    *slot = h;
                    if light {
                        let i = -normal_at(gx, gy).dot(d);
                        if i > 0.0 {
                            accum[(sy / n) * size + sx / n] += sun.color * i;
                        }
                    }
                }
            }
            std::mem::swap(&mut prev, &mut cur);
        }
    }
    let inv = 1.0 / (n * n) as f32;
    for a in &mut accum {
        *a *= inv;
    }
    accum
}

/// Scales the direct light of every texel a caster's shadow touches by its
/// unshadowed fraction. The shadowed area is estimated with `COVERAGE_SAMPLES`²
/// rays from points on the terrain towards the sun.
fn apply_casters(ti: &TerrainInfo, sun: &Sun, size: usize, accum: &mut [Vec3], casters: &[Caster]) {
    let d = sun.direction;
    let to_sun = -d;
    let texel_m = ti.square * ti.terrain.size() as f32 / size as f32;
    let world_of = |i: i64| ti.origin + i as f32 * texel_m;
    for c in casters {
        if c.tris.is_empty() {
            continue;
        }
        // Footprint: the box's corners and where rays from them along the
        // light direction meet the terrain.
        let (mut lo, mut hi) = (c.min.truncate(), c.max.truncate());
        for i in 0..8 {
            let p = Vec3::new(
                if i & 1 == 0 { c.min.x } else { c.max.x },
                if i & 2 == 0 { c.min.y } else { c.max.y },
                if i & 4 == 0 { c.min.z } else { c.max.z },
            );
            if let Some(h) = ti.raycast(p, d, PROBE_LENGTH) {
                lo = lo.min(h.truncate());
                hi = hi.max(h.truncate());
            }
        }
        let i0 = ((lo.x - ti.origin) / texel_m).floor() as i64 - 1;
        let i1 = ((hi.x - ti.origin) / texel_m).ceil() as i64 + 1;
        let j0 = ((lo.y - ti.origin) / texel_m).floor() as i64 - 1;
        let j1 = ((hi.y - ti.origin) / texel_m).ceil() as i64 + 1;
        for j in j0..j1 {
            for i in i0..i1 {
                let idx = (j.rem_euclid(size as i64) as usize) * size + i.rem_euclid(size as i64) as usize;
                if accum[idx].max_element() <= 0.0 {
                    continue;
                }
                let mut covered = 0;
                for sj in 0..COVERAGE_SAMPLES {
                    for si in 0..COVERAGE_SAMPLES {
                        let x = world_of(i) + (si as f32 + 0.5) / COVERAGE_SAMPLES as f32 * texel_m;
                        let y = world_of(j) + (sj as f32 + 0.5) / COVERAGE_SAMPLES as f32 * texel_m;
                        let p = Vec3::new(x, y, ti.height(Vec2::new(x, y)));
                        if c.hit(p, to_sun, 0.05) {
                            covered += 1;
                        }
                    }
                }
                let shadowed = covered as f32 / (COVERAGE_SAMPLES * COVERAGE_SAMPLES) as f32;
                accum[idx] *= (1.0 - shadowed).clamp(0.0, 1.0);
            }
        }
    }
}

/// Finish: ambient, clamp, 2×2 blur, 5-bit quantisation.
fn finish(accum: &[Vec3], sun: &Sun, size: usize) -> Vec<Vec3> {
    let c: Vec<Vec3> = accum.iter().map(|a| (*a + sun.ambient).clamp(Vec3::ZERO, Vec3::ONE)).collect();
    // Directional 2×2 box blur pulling from the side the light comes from.
    // The engine blurs in place in an order that never reads an already
    // blurred texel, which equals blurring from a copy as done here.
    let d = sun.scan_direction();
    let sx: i64 = if d.x > 0.0 { -1 } else { 1 };
    let sy: i64 = if d.y > 0.0 { -1 } else { 1 };
    let s = size as i64;
    let at = |x: i64, y: i64| c[(y.rem_euclid(s) * s + x.rem_euclid(s)) as usize];
    let mut out = Vec::with_capacity(size * size);
    for y in 0..s {
        for x in 0..s {
            let b = (at(x, y) + at(x + sx, y) + at(x, y + sy) + at(x + sx, y + sy)) * 0.25;
            // Q = trunc(c·31 + 0.5); the texture blend uses q/32.
            out.push(Vec3::new(quant(b.x), quant(b.y), quant(b.z)));
        }
    }
    out
}

fn quant(v: f32) -> f32 {
    (v * 31.0 + 0.5).trunc() / 32.0
}

/// Lightmaps with and without shape shadows. The latter is what static shapes
/// sample for their own light, so an object doesn't darken itself through its
/// own shadow (its occlusion by other casters is handled separately).
pub struct Baked {
    pub lightmap: Lightmap,
    pub terrain_only: Lightmap,
}

pub fn bake(ti: &TerrainInfo, sun: &Sun, casters: &[Caster]) -> Baked {
    let size = LIGHTMAP_SIZE;
    let mut accum = direct_light(ti, sun, size);
    let lm = |texels| Lightmap { size, texels, terrain_size: ti.terrain.size(), square: ti.square, origin: ti.origin };
    let terrain_only = lm(finish(&accum, sun, size));
    apply_casters(ti, sun, size, &mut accum, casters);
    Baked { lightmap: lm(finish(&accum, sun, size)), terrain_only }
}

/// Static-shape lighting inputs (Prop / PowerUp fields).
pub struct StaticObject {
    pub min: Vec3,
    pub max: Vec3,
    /// Index into the caster list, if this object casts a shadow.
    pub caster: Option<usize>,
    /// `tree_lighting`: the object's own geometry also blocks its light.
    pub tree_lighting: bool,
    /// `light_boost`: blend towards the full sun colour.
    pub light_boost: f32,
}

/// Diffuse scale `k` of a static shape.
///
/// - `T`: average lightmap colour where four rays from the object's box
///   corners (at mid height) along `direction · 250` meet the terrain. When a
///   ray misses (e.g. a sun below the horizon), the terrain straight below
///   that corner is used.
/// - `frac`: fraction of five probe points (box centre and corners at mid
///   height) that see the sun past the other casters (and past the object
///   itself with `tree_lighting`).
/// - The doc's `C = T·frac·(1−boost) + color·boost + ambient` adds ambient a second
///   time; since `T` already contains ambient, frac and boost are applied to the
///   direct part only: `C = clamp(ambient + max(T − ambient, 0)·frac·(1−boost) +
///   color·boost)`. With frac = 1 and boost = 0 this is the moving-object rule `k =
///   clamp(avg(T) − avg(ambient), 0, 1)`.
pub fn static_factor(ti: &TerrainInfo, sun: &Sun, lm: &Lightmap, casters: &[Caster], obj: &StaticObject) -> f32 {
    let mid = 0.5 * (obj.min.z + obj.max.z);
    let corners = [
        Vec3::new(obj.min.x, obj.min.y, mid),
        Vec3::new(obj.max.x, obj.min.y, mid),
        Vec3::new(obj.min.x, obj.max.y, mid),
        Vec3::new(obj.max.x, obj.max.y, mid),
    ];
    let mut t = Vec3::ZERO;
    for c in corners {
        let hit = ti.raycast(c, sun.direction, PROBE_LENGTH).unwrap_or(c);
        t += lm.sample(hit.truncate());
    }
    t /= 4.0;

    let centre = 0.5 * (obj.min + obj.max);
    let probes = [centre, corners[0], corners[1], corners[2], corners[3]];
    let mut lit = 0;
    for p in probes {
        let blocked = casters.iter().enumerate().any(|(ci, c)| (obj.tree_lighting || Some(ci) != obj.caster) && c.hit(p, -sun.direction, 0.05));
        if !blocked {
            lit += 1;
        }
    }
    let frac = lit as f32 / probes.len() as f32;

    let boost = obj.light_boost.clamp(0.0, 1.0);
    let c = (sun.ambient + (t - sun.ambient).max(Vec3::ZERO) * frac * (1.0 - boost) + sun.color * boost).clamp(Vec3::ZERO, Vec3::ONE);
    let avg = |v: Vec3| (v.x + v.y + v.z) / 3.0;
    (avg(c) - avg(sun.ambient)).clamp(0.0, 1.0)
}

/// Diffuse scale of a moving shape at `pos`: the lightmap straight
/// below it, minus the average ambient.
pub fn dynamic_factor(sun: &Sun, lm: &Lightmap, pos: Vec3) -> f32 {
    let t = lm.sample(pos.truncate());
    let avg = |v: Vec3| (v.x + v.y + v.z) / 3.0;
    (avg(t).clamp(0.0, 1.0) - avg(sun.ambient)).clamp(0.0, 1.0)
}

fn ray_box(o: Vec3, d: Vec3, min: Vec3, max: Vec3) -> bool {
    let inv = d.recip();
    let t1 = (min - o) * inv;
    let t2 = (max - o) * inv;
    let tmin = t1.min(t2).max_element();
    let tmax = t1.max(t2).min_element();
    tmax >= tmin.max(0.0)
}

/// Möller–Trumbore, both faces; returns the ray parameter.
fn ray_triangle(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(e2.dot(q) * inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantisation_matches_engine() {
        assert_eq!(quant(1.0), 31.0 / 32.0);
        assert_eq!(quant(0.3), 9.0 / 32.0);
        assert_eq!(quant(0.7), 22.0 / 32.0);
    }

    #[test]
    fn ray_hits_triangle() {
        let tri = [Vec3::new(-1.0, -1.0, 2.0), Vec3::new(1.0, -1.0, 2.0), Vec3::new(0.0, 1.0, 2.0)];
        assert!((ray_triangle(Vec3::ZERO, Vec3::Z, &tri).unwrap() - 2.0).abs() < 1e-6);
        assert!(ray_triangle(Vec3::new(5.0, 0.0, 0.0), Vec3::Z, &tri).is_none());
    }
}

//! Generates OpenTT's three new multiplayer maps (one per world), each as a terrain
//! (`assets/missions/<id>.ter`) and a mission file (`assets/missions/<id>.ron`, the RON
//! the game loads):
//!
//! * `tt1_5` "Amygdala Abyss" (green, 432 m mission area): grassy hills
//!   split by a winding ravine (about 15 m, walls worn to under 40 degrees) with ramps in
//!   and out; in the middle a land bridge (a ridge at bank level) crosses it
//!   up to a notch that the ravine road runs through, jumped (24 m) from
//!   lips on both banks; a raised plateau with ramps, a rocky outcrop field,
//!   rolling hills and a dry lake bed.
//! * `tt2_5` "Thalamus Pass" (blue, 408 m): a stone circle on a hill
//!   between two winding ridges with gentle flanks, each cut by a
//!   steep-walled slot canyon (a reload arch inside) and dipping into
//!   saddles further out; a sunken basin, a hollow with bones, rocky
//!   highlands.
//! * `tt3_5` "Cerebral Caldera" (red, 400 m): a volcano in a caldera ringed
//!   by obsidian spires; lava rivers run from it to the east and west
//!   edges, crossed by a natural causeway in the west and two ramp jumps
//!   over a narrow gorge in the east; dense small ridges, gullies and
//!   mounds, obsidian ridges and two plateaus.
//!
//! The terrain tool (`erosion.rs`) wears each field down after its
//! features are cut in: droplet erosion carving channels and depositing
//! sediment, thermal erosion to a stable talus slope, a hard slope cap;
//! `--preview DIR` writes shaded-relief and slope images per map.
//! Slopes are capped at about 45 degrees (29 on the blue
//! map) so the terrain textures do not stretch and tanks climb most of it;
//! the landmark walls are one 8 m grid square for at most 9.5 m (50
//! degrees).
//!
//! Terrain only: no overhangs (the heightfield cannot make them, and the
//! tank's suspension only stands on the terrain).
//!
//! Each heightfield is multi-octave value noise (ridged for the red map) shaped by the
//! map's hand-placed features (valleys, ridges, mesas, the caldera), worn down by droplet
//! and thermal erosion; the landmarks are then cut in exactly (the 8 m grid decides where
//! walls stand). Materials are painted by slope and by how low a spot lies against its
//! surroundings; each world's rock material in crisp, noise-broken strokes on steep
//! ground, sharp crests, slope breaks and deep creases (`rock`). Pads lie on platforms
//! and the reload arches on level lanes (the arch and 10 m of run-up and run-out) stamped
//! after the erosion. Scenery comes in natural clusters; spawns and power-ups sit along
//! the routes, the two teams' halves balanced but not mirrored.
//!
//! ```sh
//! cargo run -p tt-assets --example new_maps [-- --preview out/new-maps] [DIR]
//! #   writes DIR (default assets/missions)/tt{1,2,3}_5.ter and .ron
//! ```
//! The missions name assets by their asset paths (`models/props/green/...`,
//! `sky/sky_day.dml`, `missions/<id>.ter`).

use std::f32::consts::PI;
use tt_assets::data::defs::Def;
use tt_assets::data::mission::{Group, Mission, MissionObject};
use tt_assets::data::Field;
use tt_assets::terrain::Terrain;

mod erosion;

use erosion::{grid_xy, Erosion, Grid, ORIGIN, SIZE, SQUARE};

const SHIFT: u8 = 7;
const BLOCK: f32 = SIZE as f32 * SQUARE;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Rounded square distance from the centre (a superellipse, n = 4).
fn sdist(x: f32, y: f32) -> f32 {
    (x.powi(4) + y.powi(4)).powf(0.25)
}

/// 1 inside `r0`, 0 outside `r1`.
fn disc(x: f32, y: f32, cx: f32, cy: f32, r0: f32, r1: f32) -> f32 {
    1.0 - smoothstep(r0, r1, (x - cx).hypot(y - cy))
}


// ---- noise (tileable over the block) -------------------------------------------

fn hash(seed: u32, i: i32, j: i32) -> f32 {
    let mut h = seed ^ (i as u32).wrapping_mul(0x27d4_eb2d) ^ (j as u32).wrapping_mul(0x1656_67b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32
}

/// Value noise in 0..1 with `period` lattice cells across the block.
fn vnoise(seed: u32, x: f32, y: f32, period: i32) -> f32 {
    let u = (x - ORIGIN) / BLOCK * period as f32;
    let v = (y - ORIGIN) / BLOCK * period as f32;
    let (i, j) = (u.floor() as i32, v.floor() as i32);
    let (fx, fy) = (u - i as f32, v - j as f32);
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (sx, sy) = (fade(fx), fade(fy));
    let c = |di: i32, dj: i32| hash(seed, (i + di).rem_euclid(period), (j + dj).rem_euclid(period));
    lerp(lerp(c(0, 0), c(1, 0), sx), lerp(c(0, 1), c(1, 1), sx), sy)
}

/// Fractal sum, 0..1.
fn fbm(seed: u32, x: f32, y: f32, period: i32, octaves: u32, gain: f32) -> f32 {
    let (mut sum, mut amp, mut norm, mut p) = (0.0, 1.0, 0.0, period);
    for o in 0..octaves {
        sum += amp * vnoise(seed.wrapping_add(o * 101), x, y, p);
        norm += amp;
        amp *= gain;
        p *= 2;
    }
    sum / norm
}

/// Ridged fractal sum, 0..1 (sharp crests).
fn ridged(seed: u32, x: f32, y: f32, period: i32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut p) = (0.0, 1.0, 0.0, period);
    for o in 0..octaves {
        let n = 1.0 - (2.0 * vnoise(seed.wrapping_add(o * 131), x, y, p) - 1.0).abs();
        sum += amp * n * n;
        norm += amp;
        amp *= 0.5;
        p *= 2;
    }
    sum / norm
}

/// Ridged fractal sum, 0..1, its crests rounded (a smooth |x| over
/// `0.15` of the noise range): knolls, not knife edges.
fn soft_ridged(seed: u32, x: f32, y: f32, period: i32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut p) = (0.0, 1.0, 0.0, period);
    for o in 0..octaves {
        let v = 2.0 * vnoise(seed.wrapping_add(o * 131), x, y, p) - 1.0;
        let n = 1.0 - (v * v + 0.15 * 0.15).sqrt() + 0.15;
        sum += amp * n * n;
        norm += amp;
        amp *= 0.5;
        p *= 2;
    }
    sum / norm
}

/// `max(x, 0)` with the corner rounded over `k` metres (C1).
fn softplus(x: f32, k: f32) -> f32 {
    if x / k > 20.0 {
        x
    } else {
        k * (x / k).exp().ln_1p()
    }
}

/// `min(a, b)` with the crease rounded over `k` metres (C1).
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    lerp(b, a, h) - k * h * (1.0 - h)
}

/// `max(a, b)` with the crease rounded over `k` metres (C1).
fn smax(a: f32, b: f32, k: f32) -> f32 {
    -smin(-a, -b, k)
}

/// `h`, but never much below `lo` (a soft floor over `k` metres).
fn soft_floor(h: f32, lo: f32, k: f32) -> f32 {
    lo + k * ((h - lo) / k).exp().ln_1p()
}

// ---- heightfield pipeline ---------------------------------------------------------

/// Rounded-square distance with a noisy edge: where the plateau ends.
fn edge_dist(seed: u32, x: f32, y: f32) -> f32 {
    let r = (x.abs().powi(6) + y.abs().powi(6)).powf(1.0 / 6.0);
    r + 44.0 * (fbm(seed ^ 0xed6e, x, y, 6, 3, 0.5) - 0.5)
}

fn island(seed: u32, half: f32, x: f32, y: f32, play: f32, outer: f32) -> f32 {
    let s1 = smoothstep(half - 6.0, half + 18.0, edge_dist(seed, x, y));
    let s2 = smoothstep(half + 64.0, half + 144.0, sdist(x, y));
    (play * (1.0 - s1) * (1.0 - s2) + outer * s2).max(0.0)
}

fn outer_hills(seed: u32, x: f32, y: f32, base: f32, amp: f32) -> f32 {
    base + amp * (fbm(seed ^ 0x0c7e, x, y, 6, 4, 0.5) - 0.5) * 2.0
}

/// Features cut in → erosion over the whole field (`mask` 0..1 keeps
/// cells exact, `flush` 0..1 keeps sediment out of the designed channels,
/// `bed` is the bedrock erosion stops at) → plateau edge and pit.
#[allow(clippy::too_many_arguments)]
fn heightfield(seed: u32, half: f32, e: Erosion, shape: impl Fn(f32, f32) -> f32, mask: impl Fn(f32, f32) -> f32, flush: impl Fn(f32, f32) -> f32, bed: impl Fn(f32, f32) -> f32, outer: impl Fn(f32, f32) -> f32) -> Vec<f32> {
    let mut g = Grid::from_fn(shape);
    let (mask, flush, bed) = (Grid::from_fn(mask), Grid::from_fn(flush), Grid::from_fn(bed));
    g.wear(seed, &e, &mask, &flush, &bed, |x, y| edge_dist(seed, x, y) < half + 6.0);
    (0..SIZE * SIZE)
        .map(|k| {
            let (x, y) = grid_xy(k % SIZE, k / SIZE);
            island(seed, half, x, y, g.0[k], outer(x, y))
        })
        .collect()
}

// ---- small rotation helpers ----

#[derive(Clone, Copy)]
struct Quat([f32; 4]); // x y z w

impl Quat {
    fn axis_angle(ax: [f32; 3], rad: f32) -> Quat {
        let l = (ax[0] * ax[0] + ax[1] * ax[1] + ax[2] * ax[2]).sqrt();
        let s = (rad * 0.5).sin() / l;
        Quat([ax[0] * s, ax[1] * s, ax[2] * s, (rad * 0.5).cos()])
    }
    fn mul(self, o: Quat) -> Quat {
        let [ax, ay, az, aw] = self.0;
        let [bx, by, bz, bw] = o.0;
        Quat([aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz])
    }
    fn rotate(self, v: [f32; 3]) -> [f32; 3] {
        let p = Quat([v[0], v[1], v[2], 0.0]);
        let c = Quat([-self.0[0], -self.0[1], -self.0[2], self.0[3]]);
        let r = self.mul(p).mul(c);
        [r.0[0], r.0[1], r.0[2]]
    }
    /// As a mission object's `rotation` (axis, then the angle in degrees).
    fn rotation(self) -> String {
        let [x, y, z, w] = self.0;
        let s = (1.0 - w * w).max(0.0).sqrt();
        if s < 1e-5 {
            return "1 0 0 0".into();
        }
        let angle = 2.0 * w.clamp(-1.0, 1.0).acos();
        format!("{} {} {} {}", f(x / s), f(y / s), f(z / s), f(-angle.to_degrees()))
    }
}

/// Heading `deg` (compass, clockwise from north = +y): shape +y points there.
fn yaw(deg: f32) -> Quat {
    Quat::axis_angle([0.0, 0.0, 1.0], -deg.to_radians())
}

/// Lying: shape +z along heading `deg + 90` (shape +x down), i.e. a pillar
/// shape laid across the direction `deg` faces.
fn lying(deg: f32) -> Quat {
    yaw(deg).mul(Quat::axis_angle([0.0, 1.0, 0.0], PI / 2.0))
}

/// Tilted: heading `deg`, then leaning `lean` degrees about a horizontal axis.
fn tilted(deg: f32, lean: f32, lean_dir: f32) -> Quat {
    let a = lean_dir.to_radians();
    Quat::axis_angle([a.cos(), -a.sin(), 0.0], lean.to_radians()).mul(yaw(deg))
}

fn f(v: f32) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

fn v3(v: [f32; 3]) -> String {
    format!("{} {} {}", f(v[0]), f(v[1]), f(v[2]))
}

/// A mission object of class `class` with `fields` (text values).
fn obj(class: &str, name: &str, fields: &[(&str, String)]) -> MissionObject {
    let f: Vec<Field> = fields.iter().map(|(k, v)| Field::new(*k, "", v.clone())).collect();
    MissionObject::from_fields(class, name, None, false, &f)
}

/// A group of `children`.
fn group(name: &str, music_track: Option<&str>, children: Vec<MissionObject>) -> MissionObject {
    MissionObject::Group(Group { name: name.into(), music_track: music_track.map(Into::into), extra: Vec::new(), children })
}

// ---- map builder -------------------------------------------------------------

struct World {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    music: &'static str,
    layout: &'static str,
    sky: MissionObject,
    sun: MissionObject,
    floor: u32,
    /// Half the mission area's side.
    half: f32,
    materials: [&'static str; 4],
}

struct Map {
    w: World,
    heights: Vec<f32>,
    weights: [Vec<u8>; 4],
    green: Vec<MissionObject>,
    blue: Vec<MissionObject>,
    objects: Vec<MissionObject>,
    /// Defs the mission declares (before its objects).
    defs: Vec<Def>,
    shape_bounds: std::collections::HashMap<String, ([f32; 3], [f32; 3])>,
    /// (x, y, radius): keep scenery away (spawns, triggers, routes).
    keep_out: Vec<(f32, f32, f32)>,
    /// Stamped platforms and lanes blend back into the terrain over this
    /// many metres (a smoothstep from the plane; 0: no blend).
    blend: f32,
    rng: u32,
}

/// What the material painter knows about a grid point.
struct Spot {
    x: f32,
    y: f32,
    h: f32,
    /// Rise per metre.
    slope: f32,
    /// Metres below the surrounding mean (negative on crests).
    low: f32,
    /// Principal curvatures of the 8 m grid (1/m, the Hessian's
    /// eigenvalues): `kmin` < 0 across a convex ridge or slope break,
    /// `kmax` > 0 across a concave crease.
    kmin: f32,
    kmax: f32,
}

/// This mask: 0..1, crisp with a soft texel-wide edge.
fn rock(p: &Spot, seed: u32) -> f32 {
    let ridge = smoothstep(0.03, 0.065, -p.kmin);
    let steep = smoothstep(0.55, 0.80, p.slope);
    let crease = smoothstep(0.06, 0.12, p.kmax) * 0.8;
    let score = ridge.max(steep).max(crease) + 0.45 * (fbm(seed, p.x, p.y, 48, 2, 0.5) - 0.5);
    smoothstep(0.5, 0.68, score)
}

/// Paints the rock mask `r` over weights `w`: slot `k` takes the share
/// `r` of the spot, the others keep theirs in proportion.
fn with_rock(w: [f32; 4], k: usize, r: f32) -> [f32; 4] {
    let w = w.map(|v| v.max(0.0));
    let sum: f32 = w.iter().sum::<f32>().max(1e-6);
    let mut o = w.map(|v| v / sum * (1.0 - r));
    o[k] += r;
    o
}

impl Map {
    fn new(w: World, heights: Vec<f32>, weights: impl Fn(&Spot) -> [f32; 4]) -> Map {
        let n = SIZE * SIZE;
        let g = Grid(heights.iter().map(|h| (h * 32.0).round() / 32.0).collect());
        let mean = g.blurred(3);
        let mut ws: [Vec<u8>; 4] = [vec![0; n], vec![0; n], vec![0; n], vec![0; n]];
        for j in 0..SIZE {
            for i in 0..SIZE {
                let (x, y) = grid_xy(i, j);
                let (ii, jj) = (i as i32, j as i32);
                let gx = (g.at(ii + 1, jj) - g.at(ii - 1, jj)) / (2.0 * SQUARE);
                let gy = (g.at(ii, jj + 1) - g.at(ii, jj - 1)) / (2.0 * SQUARE);
                let s2 = SQUARE * SQUARE;
                let c = g.at(ii, jj);
                let hxx = (g.at(ii + 1, jj) - 2.0 * c + g.at(ii - 1, jj)) / s2;
                let hyy = (g.at(ii, jj + 1) - 2.0 * c + g.at(ii, jj - 1)) / s2;
                let hxy = (g.at(ii + 1, jj + 1) - g.at(ii - 1, jj + 1) - g.at(ii + 1, jj - 1) + g.at(ii - 1, jj - 1)) / (4.0 * s2);
                let (mid, d) = ((hxx + hyy) / 2.0, (((hxx - hyy) / 2.0).powi(2) + hxy * hxy).sqrt());
                let spot = Spot { x, y, h: c, slope: gx.hypot(gy), low: mean.at(ii, jj) - c, kmin: mid - d, kmax: mid + d };
                let w = weights(&spot).map(|v| v.max(0.0));
                let sum: f32 = w.iter().sum::<f32>().max(1e-6);
                let mut q: Vec<i32> = w.iter().map(|v| (v / sum * 255.0).round() as i32).collect();
                let total: i32 = q.iter().sum();
                let k = (0..4).max_by(|&a, &b| w[a].partial_cmp(&w[b]).unwrap()).unwrap();
                q[k] += 255 - total;
                for s in 0..4 {
                    ws[s][j * SIZE + i] = q[s].clamp(0, 255) as u8;
                }
            }
        }
        Map { w, heights, weights: ws, green: Vec::new(), blue: Vec::new(), objects: Vec::new(), defs: Vec::new(), shape_bounds: Default::default(), keep_out: Vec::new(), blend: 0.0, rng: 0x2545_f491 }
    }

    fn grid(&self, i: i32, j: i32) -> f32 {
        let n = SIZE as i32;
        (self.heights[(j.rem_euclid(n) * n + i.rem_euclid(n)) as usize] * 32.0).round() / 32.0
    }

    /// Ground height as the engine computes it (the triangle split of
    /// `tt_game::terrain::HeightField`).
    fn ground(&self, x: f32, y: f32) -> f32 {
        let gx = (x - ORIGIN) / SQUARE;
        let gy = (y - ORIGIN) / SQUARE;
        let (i, j) = (gx.floor() as i32, gy.floor() as i32);
        let (fx, fy) = (gx - i as f32, gy - j as f32);
        let (bl, br, tl, tr) = (self.grid(i, j), self.grid(i + 1, j), self.grid(i, j + 1), self.grid(i + 1, j + 1));
        if (i + j) % 2 == 0 {
            if fx <= fy {
                bl + (tl - bl) * fy + (tr - tl) * fx
            } else {
                bl + (br - bl) * fx + (tr - br) * fy
            }
        } else if 1.0 - fx <= fy {
            br + (tl - tr) * (1.0 - fx) + (tr - br) * fy
        } else {
            br + (bl - br) * (1.0 - fx) + (tl - bl) * fy
        }
    }

    /// Steepest rise around (x, y) within `r` metres, per metre.
    fn roughness(&self, x: f32, y: f32, r: f32) -> f32 {
        let h = self.ground(x, y);
        (0..8).map(|k| (k as f32 * PI / 4.0).sin_cos()).map(|(s, c)| (self.ground(x + c * r, y + s * r) - h).abs() / r).fold(0.0, f32::max)
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// The flattest clear spot within `r` metres of (x, y).
    fn flat_near(&self, x: f32, y: f32, r: f32) -> (f32, f32) {
        let ok = |px: f32, py: f32| self.ground(px, py) > self.w.floor as f32 + 6.0 && self.roughness(px, py, 14.0) < 0.6;
        let mut best = (if ok(x, y) { self.roughness(x, y, 6.0) } else { f32::MAX }, x, y);
        for k in 0..64 {
            let a = k as f32 * 2.399_963;
            let d = r * ((k as f32 + 0.5) / 64.0).sqrt();
            let (px, py) = (x + d * a.cos(), y + d * a.sin());
            let s = self.roughness(px, py, 6.0) + 0.002 * d;
            if s < best.0 && self.clear(px, py, 2.0) && ok(px, py) {
                best = (s, px, py);
            }
        }
        (best.1, best.2)
    }

    fn spawn_point(&self, x: f32, y: f32, heading: f32) -> MissionObject {
        let z = self.ground(x, y) + 0.5;
        spawn_point("", v3([x, y, z]), yaw(heading).rotation(), "1")
    }

    /// A drop point on the flattest spot near (x, y), facing the centre
    /// (turned by `turn` degrees).
    fn spawn(&mut self, blue: bool, x: f32, y: f32, turn: f32) {
        // Pulled in towards the centre until the spot is on the plateau,
        // well clear of its edge.
        let (mut x, mut y) = (x, y);
        for _ in 0..12 {
            let (fx, fy) = (x * 1.12, y * 1.12);
            if self.ground(fx, fy) > self.w.floor as f32 + 6.0 && self.roughness(fx, fy, 14.0) < 0.6 {
                break;
            }
            x *= 0.94;
            y *= 0.94;
        }
        let (x, y) = self.flat_near(x, y, 24.0);
        if self.ground(x, y) <= self.w.floor as f32 + 6.0 { eprintln!("{}: spawn at ({x}, {y}) is off the plateau", self.w.id); }
        let heading = (-x).atan2(-y).to_degrees() + turn;
        let s = self.spawn_point(x, y, heading);
        if blue {
            self.blue.push(s);
        } else {
            self.green.push(s);
        }
        self.keep_out.push((x, y, 10.0));
    }

    fn static_at(&mut self, shape: &str, p: [f32; 3], rot: Quat, scale: [f32; 3], name: &str) {
        let tree = shape.contains("tree");
        self.objects.push(obj(
            "Prop",
            name,
            &[("position", v3(p)), ("rotation", rot.rotation()), ("scale", v3(scale)), ("shape", format!("models/{shape}.dts")), ("tree_lighting", (tree as u8).to_string()), ("light_boost", "0".into())],
        ));
    }

    /// The shape's bounds (shape space), read once from its `.dts`.
    fn bounds(&mut self, shape: &str) -> ([f32; 3], [f32; 3]) {
        if let Some(b) = self.shape_bounds.get(shape) {
            return *b;
        }
        let path = tt_assets::layout::default_dir().join(format!("models/{shape}.dts"));
        let s = tt_assets::dts::Shape::parse(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).expect("dts");
        let b = (s.bounds.min, s.bounds.max);
        self.shape_bounds.insert(shape.to_string(), b);
        b
    }

    /// A static resting on the final ground at (x, y): the bottom of its
    /// bounds as turned by `rot` and `scale` (the bounds corners that end
    /// up lowest, so lying shapes rest on their side) goes to the lowest
    /// ground under it, `sink` deeper, so no part floats.
    #[allow(clippy::too_many_arguments)]
    fn rest(&mut self, shape: &str, x: f32, y: f32, rot: Quat, scale: [f32; 3], sink: f32, name: &str) {
        let (lo, hi) = self.bounds(shape);
        let corners: Vec<[f32; 3]> = (0..8)
            .map(|k| [if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }])
            .map(|c| rot.rotate([c[0] * scale[0], c[1] * scale[1], c[2] * scale[2]]))
            .collect();
        let bottom = corners.iter().map(|c| c[2]).fold(f32::MAX, f32::min);
        let foot: Vec<[f32; 3]> = corners.into_iter().filter(|c| c[2] < bottom + 0.3).collect();
        // Sample the whole bottom face (a 7 x 7 grid over its corners, in
        // bit order a bilinear quad), so no part of the base floats over a
        // dip between the corners.
        let mut samples = foot.clone();
        if foot.len() == 4 {
            for a in 0..=6 {
                for b in 0..=6 {
                    let (u, v) = (a as f32 / 6.0, b as f32 / 6.0);
                    let l = |p: [f32; 3], q: [f32; 3], t: f32| [lerp(p[0], q[0], t), lerp(p[1], q[1], t), lerp(p[2], q[2], t)];
                    samples.push(l(l(foot[0], foot[1], u), l(foot[2], foot[3], u), v));
                }
            }
        }
        let z = samples.iter().map(|c| self.ground(x + c[0], y + c[1]) - c[2]).fold(f32::MAX, f32::min) - sink;
        self.static_at(shape, [x, y, z], rot, scale, name);
    }

    /// The tilt of the ground over `r` metres at (x, y): (slope degrees,
    /// uphill heading).
    fn slope_at(&self, x: f32, y: f32, r: f32) -> (f32, f32) {
        let gx = (self.ground(x + r, y) - self.ground(x - r, y)) / (2.0 * r);
        let gy = (self.ground(x, y + r) - self.ground(x, y - r)) / (2.0 * r);
        (gx.hypot(gy).atan().to_degrees(), gx.atan2(gy).to_degrees())
    }

    /// A rock or tree on the ground. Trees stand upright on the lowest point
    /// under their trunk, sunk at most 0.1 m; rocks lean with the slope and
    /// rest on the ground under their whole footprint, sunk at most 0.3 m.
    fn prop(&mut self, shape: &str, x: f32, y: f32, sink: f32, heading: f32, s: f32) {
        let sink = sink.clamp(0.25, 0.3);
        if shape.contains("tree") {
            let r = 1.2 * s;
            let low = (0..8).map(|k| (k as f32 * PI / 4.0).sin_cos()).map(|(a, b)| self.ground(x + b * r, y + a * r)).fold(self.ground(x, y), f32::min);
            self.static_at(shape, [x, y, low - 0.1], yaw(heading), [s, s, s], "");
        } else {
            let (deg, dir) = self.slope_at(x, y, 2.0);
            self.rest(shape, x, y, tilted(heading, (deg * 0.6).min(18.0), dir), [s, s, s], sink, "");
        }
    }

    /// The plane fitted to the ground within 8 m of (x, y) (least
    /// squares): (height at (x, y), rise per metre east, north).
    fn plane_fit(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let mut s = Vec::new();
        for ring in 0..=8 {
            // Rings 1 m apart.
            let d = ring as f32;
            for k in 0..if ring == 0 { 1 } else { 24 } {
                let a = k as f32 * PI / 12.0;
                let (dx, dy) = (d * a.cos(), d * a.sin());
                s.push((dx, dy, self.ground(x + dx, y + dy)));
            }
        }
        let n = s.len() as f32;
        let (mx, my, mz) = (s.iter().map(|p| p.0).sum::<f32>() / n, s.iter().map(|p| p.1).sum::<f32>() / n, s.iter().map(|p| p.2).sum::<f32>() / n);
        let (mut cxx, mut cxy, mut cyy, mut cxz, mut cyz) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for &(dx, dy, z) in &s {
            let (dx, dy, dz) = (dx - mx, dy - my, z - mz);
            cxx += dx * dx;
            cxy += dx * dy;
            cyy += dy * dy;
            cxz += dx * dz;
            cyz += dy * dz;
        }
        let det = cxx * cyy - cxy * cxy;
        let (gx, gy) = ((cxz * cyy - cyz * cxy) / det, (cyz * cxx - cxz * cxy) / det);
        (mz - gx * mx - gy * my, gx, gy)
    }

    /// A platform for a pad at (x, y): the plane fitted to the ground within
    /// 8 m (its slope capped at `PAD_SLOPE`) is stamped into the final
    /// heightfield (after the erosion, so nothing wears it) on every grid
    /// point of every square the pad's footprint (its bounds `lo`..`hi`
    /// turned to `heading`, plus half a metre) touches, so the ground under
    /// the whole pad is that one plane (no crease or ridge under it); the
    /// squares around blend it back into the terrain (a wider ring would
    /// steepen the ramps next to the pads).
    /// Returns the plane: (height at (x, y), rise per metre east, north).
    fn platform(&mut self, x: f32, y: f32, heading: f32, lo: [f32; 3], hi: [f32; 3]) -> (f32, f32, f32) {
        let (z0, mut gx, mut gy) = self.plane_fit(x, y);
        let g = gx.hypot(gy);
        let cap = PAD_SLOPE.to_radians().tan();
        if g > cap {
            gx *= cap / g;
            gy *= cap / g;
        }
        self.stamp(x, y, heading, lo, hi, (z0, gx, gy));
        (z0, gx, gy)
    }

    /// Stamps the plane (height at (x, y), rise per metre east, north) on
    /// every grid point of every square the footprint `lo`..`hi` (turned to
    /// `heading`, plus half a metre) touches.
    fn stamp(&mut self, x: f32, y: f32, heading: f32, lo: [f32; 3], hi: [f32; 3], (z0, gx, gy): (f32, f32, f32)) {
        // Every square the footprint (half a metre wider) touches, then
        // every corner of those squares.
        let turn = yaw(heading);
        let mut cells = std::collections::BTreeSet::new();
        for a in 0..=24 {
            for b in 0..=24 {
                let (u, v) = (a as f32 / 24.0, b as f32 / 24.0);
                let c = turn.rotate([lerp(lo[0] - 0.5, hi[0] + 0.5, u), lerp(lo[1] - 0.5, hi[1] + 0.5, v), 0.0]);
                cells.insert((((x + c[0] - ORIGIN) / SQUARE).floor() as i32, ((y + c[1] - ORIGIN) / SQUARE).floor() as i32));
            }
        }
        let corners: std::collections::BTreeSet<(i32, i32)> = cells.iter().flat_map(|&(i, j)| [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)]).collect();
        let n = SIZE as i32;
        let plane = |i: i32, j: i32| z0 + gx * (ORIGIN + i as f32 * SQUARE - x) + gy * (ORIGIN + j as f32 * SQUARE - y);
        // The ring around blends from the plane back into the terrain (no
        // terrace at the platform's edge).
        let r = (self.blend / SQUARE).ceil() as i32;
        if r > 0 {
            let (i0, i1) = (corners.iter().map(|c| c.0).min().unwrap() - r, corners.iter().map(|c| c.0).max().unwrap() + r);
            let (j0, j1) = (corners.iter().map(|c| c.1).min().unwrap() - r, corners.iter().map(|c| c.1).max().unwrap() + r);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    if corners.contains(&(i, j)) {
                        continue;
                    }
                    let d = corners.iter().map(|&(ci, cj)| ((ci - i) as f32).hypot((cj - j) as f32)).fold(f32::MAX, f32::min) * SQUARE;
                    let t = smoothstep(0.0, self.blend, d);
                    if t < 1.0 {
                        let k = (j.rem_euclid(n) * n + i.rem_euclid(n)) as usize;
                        self.heights[k] = lerp(plane(i, j), self.heights[k], t);
                    }
                }
            }
        }
        for (i, j) in corners {
            let k = (j.rem_euclid(n) * n + i.rem_euclid(n)) as usize;
            self.heights[k] = plane(i, j);
        }
    }

    /// A level lane for a drive-through arch at (x, y) facing `heading`
    /// (its passage along the shape's y axis): the arch's whole footprint
    /// plus `ARCH_RUN` metres of run-up and run-out is stamped level (as
    /// the pads' platforms: into the final heightfield, after the erosion)
    /// at the mean ground height along its centre line, so both legs stand on the same
    /// level and the passage is flat. Returns that height.
    fn arch_lane(&mut self, x: f32, y: f32, heading: f32) -> f32 {
        let (lo, hi) = self.bounds("items/recharge");
        let (lo, hi) = ([lo[0], lo[1] - ARCH_RUN, lo[2]], [hi[0], hi[1] + ARCH_RUN, hi[2]]);
        let turn = yaw(heading);
        let z = (0..=16).map(|b| {
            let c = turn.rotate([(lo[0] + hi[0]) / 2.0, lerp(lo[1], hi[1], b as f32 / 16.0), 0.0]);
            self.ground(x + c[0], y + c[1])
        }).sum::<f32>() / 17.0;
        self.stamp(x, y, heading, lo, hi, (z, 0.0, 0.0));
        z
    }

    fn trigger(&mut self, class: &str, name: &str, db: &str, x: f32, y: f32, heading: f32) {
        let z = self.ground(x, y);
        self.trigger_at(class, name, db, [x, y, z], heading);
    }

    fn trigger_at(&mut self, class: &str, name: &str, db: &str, p: [f32; 3], heading: f32) {
        self.keep_out.push((p[0], p[1], 10.0));
        let shape = match (class, db) {
            ("PowerUp", "Heal") => Some("items/health"),
            ("PowerUp", "Reload") => None,
            ("PowerUp", _) => Some("items/powerup"),
            ("Boost", "TestJump") => Some("items/jumppad"),
            ("Boost", _) => Some("items/boostpad"),
            _ => None,
        };
        let mut p = p;
        // Pads lie flush on a platform stamped under them (level, or a
        // gentle uniform slope), the bottom of their bounds 5 cm under it;
        // power-ups stand upright; arches on a level lane stamped under
        // them.
        let rot;
        if let ("Boost", Some(shape)) = (class, shape) {
            let (lo, hi) = self.bounds(shape);
            let (z0, gx, gy) = self.platform(p[0], p[1], heading, lo, hi);
            rot = tilted(heading, gx.hypot(gy).atan().to_degrees(), gx.atan2(gy).to_degrees());
            // The bounds' bottom centre on the plane over (x, y); the pad's
            // up axis is the plane's normal, so its whole bottom face is on it.
            let c = rot.rotate([0.0, 0.0, lo[2]]);
            p = [p[0] - c[0], p[1] - c[1], z0 - 0.05 - c[2]];
        } else if let Some(shape) = shape {
            // No gap under the round base of a power-up (the circle in its
            // bounds): lowered until no point of it is over the ground.
            rot = yaw(heading);
            let (lo, hi) = self.bounds(shape);
            let r = (hi[0] - lo[0]).min(hi[1] - lo[1]) / 2.0;
            let mut gap = f32::MIN;
            for ring in 0..=6 {
                for k in 0..32 {
                    let (d, a) = (r * ring as f32 / 6.0, k as f32 * PI / 16.0);
                    let c = rot.rotate([d * a.cos(), d * a.sin(), lo[2]]);
                    gap = gap.max(p[2] + c[2] - self.ground(p[0] + c[0], p[1] + c[1]));
                }
            }
            p[2] -= gap.max(0.0);
        } else if db == "Reload" {
            // An arch: upright on its level lane, both legs 5 cm into it.
            rot = yaw(heading);
            p[2] = self.arch_lane(p[0], p[1], heading) - 0.05;
        } else {
            rot = yaw(heading);
        }
        self.objects.push(obj(class, name, &[("position", v3(p)), ("rotation", rot.rotation()), ("scale", "1 1 1".into()), ("def", db.into()), ("light_boost", "0".into())]));
    }

    /// The volcano at (x, y): its round base (the radius of its bounds) a
    /// metre under the lowest ground anywhere under it, so no part floats.
    fn volcano(&mut self, x: f32, y: f32) {
        let (lo, hi) = self.bounds("volcano/volcano");
        let r = lo[0].abs().max(lo[1].abs()).max(hi[0]).max(hi[1]);
        let mut low = self.ground(x, y);
        for ring in 1..=12 {
            let d = r * ring as f32 / 12.0;
            for k in 0..64 {
                let a = k as f32 * PI / 32.0;
                low = low.min(self.ground(x + d * a.cos(), y + d * a.sin()));
            }
        }
        self.trigger_at("Volcano", "", "VolcanoDB", [x, y, low - lo[2] - 1.0], 0.0);
    }

    /// A weapon power-up on the flattest spot near (x, y).
    fn power_up(&mut self, name: &str, db: &str, x: f32, y: f32) {
        let (x, y) = self.flat_near(x, y, 8.0);
        self.trigger("PowerUp", name, db, x, y, 0.0);
    }

    /// A pad on the least sloping clear ground within 16 m of (x, y) (the
    /// plane fitted over 8 m, so its platform barely reshapes the slope).
    fn pad(&mut self, name: &str, db: &str, x: f32, y: f32, heading: f32) {
        let slope = |m: &Map, px: f32, py: f32| {
            let (_, gx, gy) = m.plane_fit(px, py);
            gx.hypot(gy)
        };
        let mut best = (slope(self, x, y), x, y);
        for k in 0..128 {
            let a = k as f32 * 2.399_963;
            let d = 16.0 * ((k as f32 + 0.5) / 128.0).sqrt();
            let (px, py) = (x + d * a.cos(), y + d * a.sin());
            let s = slope(self, px, py) + 0.002 * d;
            if s < best.0 && self.clear(px, py, 2.0) && self.ground(px, py) > self.w.floor as f32 + 6.0 {
                best = (s, px, py);
            }
        }
        self.trigger("Boost", name, db, best.1, best.2, heading);
    }

    /// A named marker (volcano target zones): a spawn sphere outside the
    /// drop point groups, as tt3_1 uses them.
    fn marker(&mut self, name: &str, x: f32, y: f32) {
        let z = self.ground(x, y) + 0.5;
        self.objects.push(spawn_point(name, v3([x, y, z]), "1 0 0 0".into(), "15"));
    }

    fn clear(&self, x: f32, y: f32, r: f32) -> bool {
        self.keep_out.iter().all(|&(kx, ky, kr)| (x - kx).hypot(y - ky) > kr + r)
    }

    /// `clusters` groups of 2..=`max` props around centres `ok` accepts:
    /// shapes picked from `shapes`, spread over `spread` metres.
    #[allow(clippy::too_many_arguments)]
    fn clusters(&mut self, shapes: &[&str], clusters: usize, max: usize, spread: f32, scale: (f32, f32), sink: f32, ok: impl Fn(&Map, f32, f32) -> bool) {
        let mut made = 0;
        let mut tries = 0;
        while made < clusters && tries < clusters * 500 {
            tries += 1;
            let half = self.w.half;
            let (cx, cy) = ((self.rand() - 0.5) * 2.0 * (half - 16.0), (self.rand() - 0.5) * 2.0 * (half - 16.0));
            if sdist(cx, cy) > half - 31.0 || !ok(self, cx, cy) || !self.clear(cx, cy, spread * 0.6) {
                continue;
            }
            made += 1;
            let n = 2 + (self.rand() * (max - 1) as f32) as usize;
            let mut placed: Vec<(f32, f32)> = Vec::new();
            for _ in 0..n * 6 {
                if placed.len() >= n {
                    break;
                }
                // Gaussian-ish offset.
                let (a, r) = (self.rand() * 2.0 * PI, spread * (self.rand() + self.rand() + self.rand() - 1.5).abs() / 1.5);
                let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
                if !self.clear(x, y, 2.5) || placed.iter().any(|&(px, py)| (px - x).hypot(py - y) < 5.0) || sdist(x, y) > self.w.half - 24.0 {
                    continue;
                }
                let shape = shapes[(self.rand() * shapes.len() as f32) as usize % shapes.len()];
                // A tree's canopy needs fairly level ground around it.
                if shape.contains("tree") && self.roughness(x, y, 6.0) > 0.18 {
                    continue;
                }
                let s = lerp(scale.0, scale.1, self.rand());
                let heading = self.rand() * 360.0;
                self.prop(shape, x, y, sink * s, heading, s);
                placed.push((x, y));
            }
            for (x, y) in placed {
                self.keep_out.push((x, y, 3.0));
            }
        }
    }

    fn write(&self, dir: &std::path::Path) {
        let ter = dir.join(format!("{}.ter", self.w.id));
        let mut t = Terrain::new(
            SHIFT,
            &self.heights,
            &[
                (self.w.materials[0], self.weights[0].clone()),
                (self.w.materials[1], self.weights[1].clone()),
                (self.w.materials[2], self.weights[2].clone()),
                (self.w.materials[3], self.weights[3].clone()),
            ],
        );
        t.heightfield_script = "Generated by crates/tt-assets/examples/new_maps/".into();
        let bytes = t.to_bytes().expect("terrain bytes");
        std::fs::write(&ter, &bytes).unwrap_or_else(|e| panic!("{}: {e}", ter.display()));

        let w = &self.w;
        let mut children = vec![
            obj("Info", "MissionInfo", &[("display_name", w.name.into()), ("desc0", w.desc.into())]),
            obj(
                "MissionArea",
                "MissionArea",
                &[("area", format!("{0} {0} {1} {1}", -w.half, 2.0 * w.half)), ("flight_ceiling", "200".into()), ("flight_ceiling_range", "0".into()), ("floor", w.floor.to_string())],
            ),
            w.sky.clone(),
            w.sun.clone(),
            obj(
                "Terrain",
                "Terrain",
                &[
                    ("position", "-1024 -1024 0".into()),
                    ("rotation", "1 0 0 0".into()),
                    ("scale", "1 1 1".into()),
                    ("terrain_file", format!("missions/{}.ter", w.id)),
                    ("detail_texture", "terrain/detail/detail1.png".into()),
                    ("square_size", "8".into()),
                    ("tile", "0".into()),
                    ("block_shift", "7".into()),
                ],
            ),
            group("PlayerDropPoints", None, vec![group("GreenPlayerDropPoints", None, self.green.clone()), group("BluePlayerDropPoints", None, self.blue.clone())]),
        ];
        children.extend(self.objects.iter().cloned());
        let mission = Mission { defs: self.defs.clone(), objects: vec![group("MissionGroup", Some(w.music), children)], conditional: Vec::new() };
        let mut header = format!("Mission {}, our own map: generated by crates/tt-assets/examples/new_maps/.", w.id.to_ascii_uppercase());
        for l in w.layout.lines() {
            header.push_str(&format!("\n{}", l.trim()));
        }
        header.push_str("\nSee tt_assets::data::mission.");
        let ron = dir.join(format!("{}.ron", w.id));
        std::fs::write(&ron, tt_assets::data::to_ron(&header, &mission).expect("mission RON")).unwrap_or_else(|e| panic!("{}: {e}", ron.display()));
        println!("{} \"{}\": {} + {} spawns, {} objects; {} bytes of terrain", w.id, w.name, self.green.len(), self.blue.len(), self.objects.len(), bytes.len());
    }
}

fn sky(dml: &str, visible: u32, solid: &str, fog_distance: u32, fog_color: &str, fog_top: u32) -> MissionObject {
    let t = |v: &str| v.to_string();
    obj(
        "Sky",
        "Sky",
        &[
            ("position", t("0 0 0")),
            ("rotation", t("1 0 0 0")),
            ("scale", t("1 1 1")),
            ("material_list", format!("sky/{dml}")),
            ("visible_distance", visible.to_string()),
            ("use_sky_textures", t("1")),
            ("render_bottom_texture", t("0")),
            ("sky_solid_color", t(solid)),
            ("fog_distance", fog_distance.to_string()),
            ("fog_color", t(fog_color)),
            ("fog_volume1", t("1 1 1")),
            ("fog_volume2", t("1 1 1")),
            ("fog_volume3", format!("40 1 {fog_top}")),
            ("fog_volume_color1", t("128.000000 128.000000 128.000000 0.000000")),
            ("fog_volume_color2", t("128.000000 128.000000 128.000000 0.000000")),
            ("fog_volume_color3", t("128.000000 128.000000 128.000000 0.000000")),
            ("wind_velocity", t("1 1 0")),
            ("wind_effect_precipitation", t("1")),
            ("no_render_bans", t("0")),
            ("cloud_height_per0", t("0")),
            ("cloud_height_per1", t("0")),
            ("cloud_height_per2", t("0")),
            ("cloud_speed1", t("0.0001")),
            ("cloud_speed2", t("0.0002")),
            ("cloud_speed3", t("0.0003")),
        ],
    )
}

fn sun(direction: &str, color: &str, ambient: &str) -> MissionObject {
    let t = |v: &str| v.to_string();
    obj("Sun", "", &[("position", t("0 0 0")), ("rotation", t("1 0 0 0")), ("scale", t("1 1 1")), ("direction", t(direction)), ("color", t(color)), ("ambient", t(ambient))])
}

/// A spawn point (a drop point, or a named marker).
fn spawn_point(name: &str, position: String, rotation: String, radius: &str) -> MissionObject {
    let t = |v: &str| v.to_string();
    obj(
        "SpawnPoint",
        name,
        &[
            ("position", position),
            ("rotation", rotation),
            ("scale", t("1 1 1")),
            ("def", t("SpawnSphereMarker")),
            ("radius", t(radius)),
            ("sphere_weight", t("100")),
            ("indoor_weight", t("100")),
            ("outdoor_weight", t("100")),
            ("lock_count", t("0")),
            ("homing_count", t("0")),
        ],
    )
}

/// A mesa: flat top (radius `top` plus a noisy edge), steep sides to
/// `foot`, a ramp `width` m wide running `len` m down along `ramp_dir`
/// (compass degrees).
#[allow(clippy::too_many_arguments)]
fn mesa(seed: u32, x: f32, y: f32, c: (f32, f32), h: f32, top: f32, foot: f32, ramp_dir: f32, len: f32, width: f32) -> f32 {
    let (dx, dy) = (x - c.0, y - c.1);
    let wobble = 9.0 * (vnoise(seed, x, y, 24) - 0.5) * 2.0;
    let d = dx.hypot(dy) + wobble;
    let body = h * (1.0 - smoothstep(top, foot, d));
    let (s, co) = ramp_dir.to_radians().sin_cos();
    let along = dx * s + dy * co;
    let side = (dx * co - dy * s).abs();
    // A spur: an S-curve down (no crease at either end), widening as it
    // descends, its sides blending into the slope.
    let t = ((along - top * 0.8) / len).clamp(0.0, 1.0);
    let half = width * 0.5 * (1.0 + 0.8 * t) + 4.0 * (vnoise(seed ^ 77, x, y, 32) - 0.5);
    let ramp = if along > 0.0 { h * (1.0 - smoothstep(0.0, 1.0, t)) * (1.0 - smoothstep(half, half + 10.0 + 6.0 * t, side)) } else { 0.0 };
    smax(body, ramp, 3.0)
}

/// A boost pad for the jumps: Boost1's pad, but setting the speed along
/// its kick instead of adding to it. The steepest platform a pad lies
/// on.
const PAD_SLOPE: f32 = 5.0;
/// Level run-up and run-out before and after a drive-through arch (m).
const ARCH_RUN: f32 = 10.0;

fn jump_boost() -> Def {
    Def::from_fields("BoostDef", "JumpBoost", Some("Boost1"), true, &[Field::new("kind", "", "overridedir"), Field::new("velocity1", "", "34")])
}

// ---- Amygdala Abyss (green) ---------------------------------------------------

const G_SEED: u32 = 0x61_7263;
/// A 432 m mission area.
const G_HALF: f32 = 216.0;
/// Bank level at the land bridge; the ravine floor; the notch floor.
const G_BANK: f32 = 118.0;
const G_FLOOR: f32 = 98.0;
/// The notch floor: the walls up to the lips (2.5 m) stay at 9.5 m per square (50 degrees).
const G_NOTCH: f32 = G_BANK + 2.5 - 9.5;
const G_PLATEAU: (f32, f32) = (116.0, 108.0);
const G_LAKE: (f32, f32) = (-116.0, -108.0);
const G_OUTCROP: (f32, f32) = (126.0, -110.0);
const G_HILLS: (f32, f32) = (-126.0, 110.0);
/// Where the ravine's walls ease into ramps: (y, side: -1 west, 1 east).
const G_RAMPS: [(f32, f32); 6] = [(-140.0, 1.0), (-92.0, -1.0), (-50.0, 1.0), (52.0, -1.0), (92.0, 1.0), (138.0, -1.0)];
/// Ravine wall slope (37 degrees, under the talus slope so erosion keeps the walls) and ramp slope (17 degrees).
const G_WALL: f32 = 0.75;
const G_RAMP: f32 = 0.3;

/// Ravine centre line: straight through the land bridge, winding further out.
fn g_ravine_x(y: f32) -> f32 {
    34.0 * smoothstep(30.0, 100.0, y.abs()) * (y / 52.0).sin()
}

/// The ravine cut: floor 24 m wide at about 98 m (bedrock for the erosion),
/// walls of 37 degrees easing to ramps at G_RAMPS, the ends rising
/// out towards the plateau's north and south edges.
fn g_carve(x: f32, y: f32, h: f32) -> f32 {
    let dx = x - g_ravine_x(y);
    let d = dx.abs();
    let out = softplus(d - 14.0, 5.0);
    let mut slope = G_WALL;
    for (k, (ry, side)) in G_RAMPS.iter().enumerate() {
        if dx * side > 0.0 {
            // A curved gully up the wall: it bends along the ravine as it
            // climbs and widens towards the top; its edges blend smoothly.
            let bend = [0.010, -0.014, 0.012, -0.009, 0.015, -0.012][k];
            let cy = ry + bend * out * out;
            let half = 8.0 + 0.35 * out + 3.0 * (vnoise(G_SEED ^ (60 + k as u32), x, y, 24) - 0.5);
            let w = 1.0 - smoothstep(half, half + 16.0, (y - cy).abs());
            slope = lerp(slope, G_RAMP, w);
        }
    }
    let floor = G_FLOOR + 0.03 * y.abs() + 1.2 * (vnoise(G_SEED ^ 5, x, y, 32) - 0.5);
    let carve = floor + out * slope;
    let fade = smoothstep(160.0, 200.0, y.abs());
    lerp(smin(h, carve, 10.0), h, fade)
}

/// The land bridge: a ridge at bank level across the ravine from the west
/// bank, ending at x = -8; the notch (floor 7 m down at 0 <= x <= 8) lets
/// the ravine road through; the east bank stands from x = 16. The gap is
/// 24 m between the lips (both banks rise 2.5 m over their last 10 m);
/// every wall here is one 8 m grid square for at most 9.5 m (50 degrees).
fn g_bridge(x: f32, y: f32, h: f32) -> f32 {
    let zone = 1.0 - smoothstep(16.0, 40.0, y.abs());
    if zone <= 0.0 {
        return h;
    }
    let lane = 1.0 - smoothstep(8.0, 22.0, y.abs());
    // The lips span the ridge's top (no tongue standing up from it).
    let lip_lane = 1.0 - smoothstep(6.0, 14.0, y.abs());
    let lip_w = 2.5 * (1.0 - smoothstep(0.0, 12.0, -8.0 - x)) * lip_lane;
    let lip_e = 2.5 * (1.0 - smoothstep(0.0, 12.0, x - 16.0)) * lip_lane;
    let target = if (0.0..=8.0).contains(&x) {
        // The notch floor, sloping down into the ravine beyond the bridge.
        lerp(G_NOTCH, h, smoothstep(10.0, 30.0, y.abs()))
    } else if x < 0.0 {
        // The bridge ridge (16 m wide on top), its flanks down to the
        // ravine, and the west bank.
        // Rounded shoulders, flanks under the talus slope (erosion keeps them).
        let ridge = G_BANK - softplus(y.abs() - 10.0, 5.0) * 0.75;
        lerp(smax(h, ridge, 3.0).min(G_BANK), G_BANK, smoothstep(26.0, 44.0, -x)) + lip_w
    } else {
        G_BANK + lip_e
    };
    // The run-up lanes (|y| < 12) stay level out to 70 m from the notch.
    let reach = lerp(40.0, 70.0, lane);
    let t = if (-8.0..=16.0).contains(&x) { zone } else { zone * (1.0 - smoothstep(reach, reach + 22.0, (x - 4.0).abs())) };
    lerp(h, target, t)
}

fn green_shape(x: f32, y: f32) -> f32 {
    let s = G_SEED;
    // Broad rolling ground: most of the relief in swells a few hundred
    // metres across, little in between.
    let mut h = soft_floor(116.0 + 40.0 * (fbm(s, x, y, 4, 3, 0.45) - 0.5) + 10.0 * (fbm(s ^ 9, x, y, 8, 2, 0.4) - 0.5), 106.0, 6.0);
    h += 3.0 * (fbm(s ^ 10, x, y, 14, 2, 0.5) - 0.5);
    // North-west: rolling hills.
    h += disc(x, y, G_HILLS.0, G_HILLS.1, 30.0, 105.0) * 12.0 * (fbm(s ^ 21, x, y, 6, 2, 0.45) - 0.35);
    // South-east: a rocky outcrop field (rounded knolls).
    h += disc(x, y, G_OUTCROP.0, G_OUTCROP.1, 30.0, 95.0) * (5.0 + 16.0 * (soft_ridged(s ^ 33, x, y, 10, 2) - 0.4));
    // North-east: the raised plateau, ramps down to the south-west and west.
    let body = |dir: f32| mesa(s, x, y, G_PLATEAU, 22.0, 32.0, 66.0, dir, 56.0, 18.0);
    h += smax(body(215.0), body(280.0), 6.0);
    // South-west: a dry lake bed.
    let lake = disc(x, y, G_LAKE.0, G_LAKE.1, 40.0 + 10.0 * (vnoise(s ^ 44, x, y, 16) - 0.5), 72.0);
    h = lerp(h, 108.0 + 0.6 * (vnoise(s ^ 45, x, y, 32) - 0.5), lake);
    g_carve(x, y, h)
}

fn green_stamp(x: f32, y: f32, h: f32) -> f32 {
    // Erosion silts the ravine up: cut it again, then the bridge.
    g_bridge(x, y, g_carve(x, y, h))
}

fn green_weights(p: &Spot) -> [f32; 4] {
    // FTerrain01 sand, 02 flowers, 03 grass, 04 rocky sand. Grass on the
    // flats and in the lows, sand on slopes and the ravine floor, rocky
    // sand (the rock mask) on steep ground, crests, slope breaks and the
    // outcrops.
    let steep = smoothstep(0.6, 1.0, p.slope);
    let slope = smoothstep(0.3, 0.6, p.slope) * (1.0 - steep);
    let ravine = (1.0 - smoothstep(12.0, 18.0, (p.x - g_ravine_x(p.y)).abs())) * (1.0 - smoothstep(150.0, 190.0, p.y.abs())) * smoothstep(0.5, 5.0, 114.0 - p.h);
    let outcrop = disc(p.x, p.y, G_OUTCROP.0, G_OUTCROP.1, 25.0, 85.0) * smoothstep(0.1, 0.3, p.slope + 0.1 * (vnoise(19, p.x, p.y, 24) - 0.5));
    let lake = disc(p.x, p.y, G_LAKE.0, G_LAKE.1, 26.0, 50.0);
    let flowers = smoothstep(0.6, 0.8, fbm(11, p.x, p.y, 16, 2, 0.5)) + lake * 0.8;
    let flat = (1.0 - steep) * (1.0 - slope);
    let grass = flat * (1.0 - ravine) * (1.0 - outcrop * 0.8);
    with_rock([slope * 0.7 + steep * 0.7 + ravine * 1.3 + lake * 0.3, grass * flowers * 0.6, grass * 1.8, outcrop * 0.9], 3, rock(p, 13))
}

fn amygdala_abyss() -> Map {
    let w = World {
        id: "tt1_5",
        name: "Amygdala Abyss",
        desc: "Grassy hills split by a ravine: jump the gap of the land bridge or take the ravine road through the notch below it.",
        music: "Lush",
        layout: "Amygdala Abyss (tt1_5): our own green map (a 432 m mission area).
                 Layout (x east, y north; green spawns south, blue north):
                   a ravine (about 15 m deep, floor about 98 m, worn walls under 40 degrees) winds
                   from the south to the north, straight through the middle, with ramps in and out on
                   alternate sides;
                   (0, 0)     a land bridge: a ridge at bank level (118 m) from the west bank across the
                              ravine, ending at a notch (floor 111 m at 0 <= x <= 8) the ravine road runs
                              through; boost pads and lips on both banks jump the 24 m gap;
                   (116, 108) a raised plateau (+22 m) with ramps to the south-west and west;
                   (126, -110) a rocky outcrop field; (-126, 110) rolling hills;
                   (-116, -108) a dry lake bed;
                   reload arches on the ravine floor north and south, jump pads onto the outcrops and
                   hills, health in the east and west.",
        sky: sky("sky_day.dml", 1000, "0.547000 0.641000 0.789000 0.000000", 300, "0.820000 0.828000 0.844000 1.000000", 95),
        sun: sun("0.57735 0.57735 -0.57735", "0.988000 0.985000 0.980000 1.000000", "0.500000 0.500000 0.500000 1.000000"),
        floor: 90,
        half: G_HALF,
        materials: ["terrain/green/fterrain01.png", "terrain/green/fterrain02.png", "terrain/green/fterrain03.png", "terrain/green/fterrain04.png"],
    };
    // Slopes capped at about 45 degrees by the slump pass (no stretched
    // cliffs); the ravine and the notch are cut in afterwards no steeper.
    let e = Erosion { talus: 6.4, drops: 20_000, radius: 3, erode: 0.1, max_cut: 0.2, crease: 0.8, relax_passes: 120, ..Erosion::default() };
    // The notch, its lips and the level run-up lanes on both banks stay
    // exact (blended out over 8-16 m), so the bridge keeps its designed
    // shape instead of slumping into a saddle around a stump at each lip.
    let mask = |x: f32, y: f32| {
        let notch = (1.0 - smoothstep(10.0, 18.0, y.abs())) * (1.0 - smoothstep(22.0, 30.0, (x - 4.0).abs()));
        let lanes = (1.0 - smoothstep(12.0, 28.0, y.abs())) * (1.0 - smoothstep(76.0, 92.0, (x - 4.0).abs())) * smoothstep(12.0, 22.0, (x - 4.0).abs());
        1.0 - notch.max(lanes)
    };
    // The ravine is flushed: its water carries the sediment on.
    let flush = |x: f32, y: f32| lerp(0.3, 1.0, smoothstep(14.0, 30.0, (x - g_ravine_x(y)).abs()).max(smoothstep(170.0, 200.0, y.abs())));
    // Bedrock: the ravine floor, and nothing much below the low ground.
    let bed = |x: f32, y: f32| if (x - g_ravine_x(y)).abs() < 40.0 && y.abs() < 200.0 { G_FLOOR + 0.03 * y.abs() - 1.0 } else { 100.0 };
    let heights = heightfield(G_SEED, G_HALF, e, |x, y| green_stamp(x, y, green_shape(x, y)), mask, flush, bed, |x, y| outer_hills(G_SEED, x, y, 140.0, 80.0));
    let mut m = Map::new(w, heights, green_weights);
    m.blend = 20.0;
    // Keep the ravine floor, the bridge and the jump lanes clear.
    for k in -22..=22 {
        let y = k as f32 * 9.0;
        m.keep_out.push((g_ravine_x(y), y, 15.0));
    }
    for x in [-60.0f32, -44.0, -28.0, -14.0, 22.0, 36.0, 50.0, 64.0] {
        m.keep_out.push((x, 0.0, 10.0));
    }
    // Spawns: green in the south, blue in the north, on open ground.
    for (x, y) in [(-172.0, -147.0), (-118.0, -178.0), (-52.0, -172.0), (49.0, -181.0), (105.0, -172.0), (172.0, -160.0), (-183.0, -67.0), (181.0, -50.0)] {
        m.spawn(false, x, y, 0.0);
    }
    for (x, y) in [(172.0, 155.0), (109.0, 181.0), (50.0, 172.0), (-49.0, 180.0), (-108.0, 172.0), (-172.0, 151.0), (183.0, 59.0), (-181.0, 60.0)] {
        m.spawn(true, x, y, 0.0);
    }
    // The notch pads first (their platforms reshape the ground under them):
    // they set the launch speed along their kick ("overridedir") instead
    // of adding to it, so every tank type flies the same arc.
    m.defs.push(jump_boost());
    m.trigger("Boost", "BoostBridgeWest", "JumpBoost", -44.0, 0.0, 90.0);
    m.trigger("Boost", "BoostBridgeEast", "JumpBoost", 52.0, 0.0, 270.0);
    // Power-ups along the routes.
    let pups = [(G_PLATEAU.0, G_PLATEAU.1), (G_LAKE.0, G_LAKE.1), (g_ravine_x(-40.0), -40.0), (g_ravine_x(40.0), 40.0), (G_OUTCROP.0, G_OUTCROP.1 + 5.0), (G_HILLS.0, G_HILLS.1 - 5.0), (46.0, 0.0)];
    for (k, (x, y)) in pups.iter().enumerate() {
        if k == 2 || k == 3 {
            m.trigger("PowerUp", &format!("pup{}", k + 1), "TestPowerup", *x, *y, 0.0);
        } else {
            m.power_up(&format!("pup{}", k + 1), "TestPowerup", *x, *y);
        }
    }
    for (name, y) in [("ReloadSouth", -110.0f32), ("ReloadNorth", 112.0)] {
        let x = g_ravine_x(y);
        let heading = (g_ravine_x(y + 4.0) - g_ravine_x(y - 4.0)).atan2(8.0).to_degrees();
        m.trigger("PowerUp", name, "Reload", x, y, heading);
    }
    m.power_up("HealWest", "Heal", -172.0, 17.0);
    m.power_up("HealEast", "Heal", 172.0, -13.0);
    // Jump pads onto the outcrops and the hills.
    // Keep the ramps out of the ravine clear.
    for (ry, side) in G_RAMPS {
        for d in [18.0f32, 30.0, 42.0, 54.0] {
            m.keep_out.push((g_ravine_x(ry) + side * d, ry, 9.0));
        }
    }
    m.pad("Jump1", "TestJump", 88.0, -80.0, 135.0);
    m.pad("Jump2", "TestJump", -88.0, 80.0, 315.0);
    // Scenery: boulders on the outcrops, the plateau's feet and the ravine
    // rims; trees in groves on the grass; cover on the open ground.
    let rocks = ["props/green/rockgreen01", "props/green/rockgreen02", "props/green/rockgreen03", "props/green/rockgreen04", "props/green/rockgreen05"];
    let trees = ["props/green/treegreen01", "props/green/treegreen02", "props/green/treegreen03"];
    m.clusters(&rocks, 8, 7, 16.0, (0.8, 1.4), 0.3, |_, x, y| (x - G_OUTCROP.0).hypot(y - G_OUTCROP.1) < 75.0);
    m.clusters(&rocks, 5, 5, 12.0, (0.8, 1.3), 0.3, |_, x, y| ((x - G_PLATEAU.0).hypot(y - G_PLATEAU.1) - 60.0).abs() < 10.0);
    m.clusters(&rocks, 10, 4, 12.0, (0.7, 1.2), 0.3, |m, x, y| m.roughness(x, y, 8.0) > 0.12 && m.roughness(x, y, 8.0) < 0.7);
    m.clusters(&["props/green/rockgreen02", "props/green/rockgreen04", "props/green/rockgreen03"], 12, 3, 10.0, (0.7, 1.2), 0.2, |m, x, y| m.roughness(x, y, 8.0) < 0.15);
    m.clusters(&trees, 20, 6, 18.0, (0.9, 1.25), 0.1, |m, x, y| m.roughness(x, y, 8.0) < 0.2 && (x - g_ravine_x(y)).abs() > 30.0 && (x - G_LAKE.0).hypot(y - G_LAKE.1) > 42.0);
    m
}

// ---- Thalamus Pass (blue) -----------------------------------------------------

const B_SEED: u32 = 0x7468_616c;
/// A smaller map than the others.
const B_HALF: f32 = 204.0;
const B_RIDGE_Y: [f32; 2] = [88.0, -92.0];
const B_FLOOR: f32 = 110.0;
const B_CIRCLE_R: f32 = 34.0;
const B_HILL_TOP: f32 = 122.0;
const B_BASIN: (f32, f32) = (118.0, 12.0);
const B_HOLLOW: (f32, f32) = (-118.0, -28.0);
const B_HIGHLANDS: [(f32, f32); 2] = [(135.0, 140.0), (-138.0, -142.0)];

/// Ridge crest line: straight east-west where the slot canyon cuts
/// through (x = 0), winding further out.
fn b_ridge_y(k: usize, x: f32) -> f32 {
    let wind = smoothstep(26.0, 70.0, x.abs());
    B_RIDGE_Y[k] + wind * if k == 0 { 20.0 * (x / 50.0).sin() } else { -17.0 * (x / 46.0 + 0.8).sin() }
}

/// Ridge height above the ground at (x, y): ridged noise along the crest,
/// saddles about 115 m out on both sides.
fn b_ridge(x: f32, y: f32) -> f32 {
    let mut best: f32 = 0.0;
    for k in 0..2 {
        let across = (y - b_ridge_y(k, x)).abs();
        // Higher by the slot canyon, so its walls stand.
        let crest = 18.0 + 10.0 * ridged(B_SEED ^ k as u32, x, y, 8, 2) + 8.0 * (1.0 - smoothstep(20.0, 50.0, x.abs()));
        let saddle_x = if k == 0 { 118.0 } else { 110.0 };
        let saddle = 1.0 - 0.72 * (1.0 - smoothstep(8.0, 32.0, (x.abs() - saddle_x).abs()));
        let end = 1.0 - smoothstep(160.0, 190.0, x.abs());
        let profile = (1.0 - smoothstep(6.0, 40.0, across)).powf(1.2);
        best = best.max(crest * saddle * end * profile);
    }
    best
}

fn blue_shape(x: f32, y: f32) -> f32 {
    let s = B_SEED;
    let mut h = soft_floor(114.0 + 32.0 * (fbm(s, x, y, 4, 4, 0.55) - 0.5), 100.0, 6.0);
    // Gentle hummocks everywhere (no empty flats).
    h += 9.0 * (fbm(s ^ 4, x, y, 14, 3, 0.5) - 0.5);
    // Rocky highlands north-east and south-west.
    for c in B_HIGHLANDS {
        h += disc(x, y, c.0, c.1, 16.0, 70.0) * (6.0 + 14.0 * (ridged(s ^ 61, x, y, 10, 3) - 0.3));
    }
    // The stone circle hill.
    h = lerp(h, h.max(B_HILL_TOP), disc(x, y, 0.0, 0.0, 44.0, 84.0));
    h += b_ridge(x, y);
    // A sunken basin in the east, a hollow in the west.
    let basin = disc(x, y, B_BASIN.0, B_BASIN.1, 26.0, 60.0);
    h = lerp(h, 103.0 + 1.5 * (vnoise(s ^ 7, x, y, 32) - 0.5), basin);
    h -= 7.0 * disc(x, y, B_HOLLOW.0, B_HOLLOW.1, 8.0, 46.0);
    h
}

fn blue_stamp(x: f32, y: f32, h: f32) -> f32 {
    let mut h = lerp(h, B_HILL_TOP, disc(x, y, 0.0, 0.0, 46.0, 62.0));
    // Slot canyons: floor at |x| <= 8 through each ridge (the walls stay
    // steep: the map's sharp feature).
    for &ry in &B_RIDGE_Y {
        let along = (y - ry).abs();
        if along < 50.0 && x.abs() <= 8.0 {
            h = lerp(h, B_FLOOR, 1.0 - smoothstep(36.0, 50.0, along));
        }
    }
    h
}

fn blue_weights(p: &Spot) -> [f32; 4] {
    // BlueTerrain01 blue ground, 02 dark violet (the rock mask: steep
    // ground, crests, slope breaks), 03 teal moss, 04 stony.
    let steep = smoothstep(0.3, 0.8, p.slope);
    let low = smoothstep(-0.5, 2.5, p.low + 4.0 * (vnoise(17, p.x, p.y, 20) - 0.5));
    let circle = disc(p.x, p.y, 0.0, 0.0, 28.0, 46.0);
    let basin = disc(p.x, p.y, B_BASIN.0, B_BASIN.1, 22.0, 46.0);
    let moss = smoothstep(0.5, 0.8, fbm(23, p.x, p.y, 16, 2, 0.5));
    let flat = 1.0 - steep;
    with_rock([flat * (1.0 - low * 0.5) + circle * 0.4, 0.0, flat * low * (0.3 + moss) + basin * 1.2, steep * 0.5 + circle * 0.6], 1, rock(p, 29))
}

fn thalamus_pass() -> Map {
    let w = World {
        id: "tt2_5",
        name: "Thalamus Pass",
        desc: "A stone circle on a hill between two winding ridges; slot canyons and saddles lead through them.",
        music: "Spooky",
        layout: "Thalamus Pass (tt2_5): our own blue map (a 408 m mission area).
                 Layout (x east, y north; green spawns south, blue north):
                   (0, 0)       a hill (122 m) with a circle of standing stones, a power-up inside;
                   ridges wind east-west at y = 88 and y = -92 (+18..28 m, gentle flanks), with saddles
                   about 115 m out on both sides (power-ups on them);
                   (0, 88), (0, -92)  a steep-walled slot canyon through each ridge (floor 110 m), a
                   reload arch inside; boost pads in front shoot into them;
                   (118, 12) a sunken basin with a fish skeleton; (-118, -28) a hollow with bones;
                   rocky highlands north-east and south-west; jump pads towards the saddles.",
        sky: sky("skyblue.dml", 300, "0.300000 0.000000 0.600000 1.000000", 30, "0.250000 0.200000 0.600000 1.000000", 95),
        sun: sun("0.427338 0.587333 0.68733", "0.250000 0.250000 1.000000 1.000000", "0.370000 0.350000 0.800000 1.000000"),
        floor: 95,
        half: B_HALF,
        materials: ["terrain/blue/blueterrain01.png", "terrain/blue/blueterrain02.png", "terrain/blue/blueterrain03.png", "terrain/blue/blueterrain04.png"],
    };
    // Slopes capped at about 0.55 (29 degrees) by the slump pass, which
    // tanks climb; the slot canyon walls are cut in afterwards.
    let e = Erosion { talus: 4.6, thermal_passes: 60, ..Erosion::default() };
    // The slot canyons erode slowly (hard rock) and are flushed.
    let canyon = |x: f32, y: f32| B_RIDGE_Y.iter().map(|ry| (1.0 - smoothstep(24.0, 40.0, x.abs())) * (1.0 - smoothstep(40.0, 56.0, (y - ry).abs()))).fold(0.0, f32::max);
    let heights = heightfield(B_SEED, B_HALF, e, |x, y| blue_stamp(x, y, blue_shape(x, y)), |x, y| 1.0 - 0.9 * canyon(x, y), |x, y| 1.0 - 0.85 * canyon(x, y), |x, y| if canyon(x, y) > 0.0 && x.abs() < 12.0 { B_FLOOR - 0.5 } else { 100.0 }, |x, y| outer_hills(B_SEED, x, y, 80.0, 50.0));
    let mut m = Map::new(w, heights, blue_weights);
    m.keep_out.push((0.0, 0.0, B_CIRCLE_R - 8.0));
    for &ry in &B_RIDGE_Y {
        for dy in [-54.0f32, -36.0, -18.0, 0.0, 18.0, 36.0, 54.0] {
            m.keep_out.push((0.0, ry + dy, 14.0));
        }
    }
    // The stone circle: standing stones of varied height and lean, two
    // fallen; gaps wide enough to drive through.
    for k in 0..14 {
        let a = (k as f32 * 360.0 / 14.0 + 7.0).to_radians();
        let r = B_CIRCLE_R + 2.5 * ((k * 5 % 3) as f32 - 1.0);
        let (x, y) = (r * a.sin(), r * a.cos());
        let shape = if k % 3 == 0 { "props/blue/rockblue03" } else { "props/blue/rockblue01" };
        if k == 4 || k == 11 {
            // Fallen: lying on the ground, tilted with the slope.
            let (deg, dir) = m.slope_at(x, y, 4.0);
            let q = Quat::axis_angle([dir.to_radians().cos(), -dir.to_radians().sin(), 0.0], deg.to_radians()).mul(lying(a.to_degrees() + 25.0));
            let c = q.rotate([0.0, 0.0, 5.6 * 0.9]);
            m.rest(shape, x - c[0], y - c[1], q, [0.9, 0.9, 0.9], 0.3, "");
        } else {
            let tall = [1.0, 0.8, 1.15, 0.9, 1.05, 0.7][k % 6];
            // Planted: the base buried a metre under the lowest ground below it,
            // leaning a little into the slope.
            let (deg, dir) = m.slope_at(x, y, 4.0);
            let lean = (k % 4) as f32 * 2.0 + deg * 0.3;
            m.rest(shape, x, y, tilted(a.to_degrees() + 90.0, lean, dir + 180.0), [0.85, 0.85, tall + 0.08], 1.0, if k == 0 { "CircleStone" } else { "" });
        }
        m.keep_out.push((x, y, 5.0));
    }
    for (x, y) in [(-117.0, -160.0), (-74.0, -148.0), (-31.0, -160.0), (27.0, -148.0), (72.0, -160.0), (129.0, -144.0), (-160.0, -101.0), (160.0, -86.0)] {
        m.spawn(false, x, y, 0.0);
    }
    for (x, y) in [(125.0, 160.0), (78.0, 148.0), (33.0, 160.0), (-30.0, 147.0), (-76.0, 160.0), (-129.0, 148.0), (162.0, 101.0), (-160.0, 90.0)] {
        m.spawn(true, x, y, 0.0);
    }
    // The boost pads into the slot canyons first (their platforms reshape
    // the ground under them).
    m.trigger("Boost", "BoostSouth", "Boost1", 0.0, B_RIDGE_Y[1] - 50.0, 0.0);
    m.trigger("Boost", "BoostNorth", "Boost1", 0.0, B_RIDGE_Y[0] + 50.0, 180.0);
    m.trigger("PowerUp", "pup1", "TestPowerup", 0.0, 0.0, 0.0);
    let saddles = [(118.0, b_ridge_y(0, 118.0)), (-118.0, b_ridge_y(0, -118.0)), (110.0, b_ridge_y(1, 110.0)), (-110.0, b_ridge_y(1, -110.0))];
    for (k, (x, y)) in saddles.iter().enumerate() {
        m.power_up(&format!("pup{}", k + 2), "TestPowerup", *x, *y);
    }
    m.power_up("pup6", "TestPowerup", B_BASIN.0, B_BASIN.1 - 16.0);
    m.power_up("pup7", "TestPowerup", B_HOLLOW.0, B_HOLLOW.1 + 16.0);
    m.trigger("PowerUp", "ReloadNorth", "Reload", 0.0, B_RIDGE_Y[0], 180.0);
    m.trigger("PowerUp", "ReloadSouth", "Reload", 0.0, B_RIDGE_Y[1], 0.0);
    m.power_up("HealWest", "Heal", -48.0, 150.0);
    m.power_up("HealEast", "Heal", 48.0, -150.0);
    m.pad("JumpSouth", "TestJump", 82.0, -56.0, 60.0);
    m.pad("JumpNorth", "TestJump", -82.0, 55.0, 240.0);
    // Scenery: the skeletons, dead tree groves, stones on the highlands
    // and the slopes, scattered cover on the open ground.
    m.prop("props/blue/fishbones", B_BASIN.0 + 6.0, B_BASIN.1 + 10.0, 0.3, 110.0, 1.0);
    m.keep_out.push((B_BASIN.0 + 6.0, B_BASIN.1 + 10.0, 28.0));
    m.prop("props/blue/bones", B_HOLLOW.0, B_HOLLOW.1 - 8.0, 0.3, 290.0, 1.0);
    m.keep_out.push((B_HOLLOW.0, B_HOLLOW.1 - 8.0, 28.0));
    let trees = ["props/blue/treeblue01", "props/blue/treeblue02", "props/blue/treeblue03", "props/blue/treeblue04", "props/blue/treeblue05", "props/blue/treeblue06"];
    let stones = ["props/blue/rockblue01", "props/blue/rockblue02", "props/blue/rockblue03"];
    m.clusters(&stones, 7, 5, 14.0, (0.6, 1.1), 0.3, |_, x, y| B_HIGHLANDS.iter().any(|c| (x - c.0).hypot(y - c.1) < 55.0));
    m.clusters(&trees, 22, 6, 16.0, (0.9, 1.3), 0.1, |m, x, y| m.roughness(x, y, 8.0) < 0.3);
    m.clusters(&stones, 12, 3, 10.0, (0.5, 0.9), 0.3, |m, x, y| m.roughness(x, y, 8.0) < 0.5);
    m.clusters(&["props/blue/rockblue02", "props/blue/rockblue03"], 10, 2, 8.0, (0.4, 0.7), 0.2, |m, x, y| m.roughness(x, y, 8.0) < 0.15);
    m
}

// ---- Cerebral Caldera (red) --------------------------------------------------

const R_SEED: u32 = 0x6361_6c64;
const R_HALF: f32 = 200.0;
const R_BANK: f32 = 108.0;
/// River floor: 7 m under the banks, so the one-square walls at the jump
/// site stay at 50 degrees.
const R_RIVER: f32 = R_BANK - 7.0;
/// The jump site: the river runs straight east-west there, a narrow gorge
/// (floor at y = 0, banks from |y| = 8: a 16 m gap).
const R_JUMP_X: (f32, f32) = (72.0, 120.0);
const R_RAMPS: [(f32, f32); 2] = [(82.0, -1.0), (110.0, 1.0)]; // (x, side: the bank the ramp is on, -1 south launching north, 1 north launching south)
const R_CAUSEWAY_X: f32 = -98.0;
const R_PLATEAU: [(f32, f32); 2] = [(-70.0, 120.0), (70.0, -120.0)];
const R_RIDGES: [(f32, f32); 2] = [(110.0, 105.0), (-110.0, -105.0)];
/// The level pad the volcano stands on (r < R_PAD.0, kept out of the
/// erosion, blended into the caldera floor by R_PAD.1): the volcano's base
/// (r 24) sits a metre under it, so the shape rises out of the ground.
const R_VOLCANO_FLOOR: f32 = 106.0;
const R_PAD: (f32, f32) = (36.0, 44.0);

/// River centre line (lava from the caldera to the east and west edges).
fn r_river_y(x: f32) -> f32 {
    if x > 0.0 {
        let straight = 1.0 - smoothstep(R_JUMP_X.0 - 24.0, R_JUMP_X.0, x) * (1.0 - smoothstep(R_JUMP_X.1, R_JUMP_X.1 + 24.0, x));
        straight * 12.0 * ((x - 48.0) / 30.0).sin()
    } else {
        14.0 * ((x + 40.0) / 34.0).sin() * smoothstep(40.0, 60.0, -x)
    }
}

/// How strongly the river cuts at x (none in the caldera, on the causeway
/// and at the plateau's edges).
fn r_river_strength(x: f32) -> f32 {
    smoothstep(48.0, 60.0, x.abs()) * smoothstep(9.0, 16.0, (x - R_CAUSEWAY_X).abs()) * (1.0 - smoothstep(160.0, 185.0, x.abs()))
}

/// How deep the river cuts at (x, y) (for the painter and the keep-outs).
fn r_river(x: f32, y: f32) -> f32 {
    let d = (y - r_river_y(x)).abs();
    7.0 * (1.0 - smoothstep(8.0, 16.0, d)) * r_river_strength(x)
}

/// The river cut: floor at R_RIVER 16 m wide, walls about 42 degrees.
fn r_carve(x: f32, y: f32, h: f32) -> f32 {
    let d = (y - r_river_y(x)).abs();
    let carve = R_RIVER + 1.0 * (vnoise(R_SEED ^ 3, x, y, 32) - 0.5) + (d - 8.0).max(0.0) * 0.9;
    lerp(h, h.min(carve), r_river_strength(x))
}

/// The jump ramps: 2.5 m over 24 m up to a lip at the bank edge (|y| = 8).
fn r_ramp(x: f32, y: f32) -> f32 {
    let mut best: f32 = 0.0;
    for (rx, side) in R_RAMPS {
        let back = side * y - 8.0; // metres back from the lip on the launch side
        if (0.0..=24.0).contains(&back) {
            let w = 1.0 - smoothstep(6.0, 10.0, (x - rx).abs());
            best = best.max(2.5 * (1.0 - back / 24.0) * w);
        }
    }
    best
}

fn r_gap(x: f32, y: f32) -> f32 {
    let heading = x.atan2(y).to_degrees().rem_euclid(360.0);
    [0.0f32, 180.0].iter().map(|g| ((heading - g + 540.0).rem_euclid(360.0) - 180.0).abs()).fold(360.0, f32::min)
}

fn red_shape(x: f32, y: f32) -> f32 {
    let s = R_SEED;
    let r = x.hypot(y);
    let mut h = soft_floor(114.0 + 40.0 * (fbm(s, x, y, 4, 3, 0.55) - 0.5) + 16.0 * (ridged(s ^ 2, x, y, 10, 2) - 0.45), 104.0, 4.0);
    // Tank-scale relief: little ridges, gullies and mounds for cover.
    h += 7.0 * (ridged(s ^ 5, x, y, 20, 2) - 0.4) + 4.0 * (fbm(s ^ 6, x, y, 28, 2, 0.5) - 0.5);
    // Obsidian ridges north-east and south-west.
    for (k, c) in R_RIDGES.iter().enumerate() {
        h += disc(x, y, c.0, c.1, 22.0, 70.0) * 20.0 * (ridged(s ^ (71 + k as u32), x, y, 8, 3) - 0.3);
    }
    // The caldera: a noisy rim around r = 44 with gaps north and south, a
    // floor at 108 m (the volcano's pad is stamped on it).
    let rim_r = 44.0 + 6.0 * (vnoise(s ^ 7, x, y, 16) - 0.5);
    let rim = 13.0 * (1.0 - smoothstep(5.0, 18.0, (r - rim_r).abs())) * smoothstep(8.0, 18.0, r_gap(x, y));
    let inside = 1.0 - smoothstep(rim_r - 12.0, rim_r - 2.0, r);
    h = lerp(h, 108.0, inside * 0.9) + rim;
    for (k, c) in R_PLATEAU.iter().enumerate() {
        h += mesa(s ^ (9 + k as u32), x, y, *c, 11.0, 16.0, 28.0, if k == 0 { 160.0 } else { 340.0 }, 32.0, 12.0);
    }
    r_carve(x, y, h)
}

fn red_stamp(x: f32, y: f32, h: f32) -> f32 {
    // Erosion silts the river up: cut it again.
    let mut h = r_carve(x, y, h);
    // The volcano's pad: level under the whole base.
    h = lerp(R_VOLCANO_FLOOR, h, smoothstep(R_PAD.0, R_PAD.1, x.hypot(y)));
    // The jump site: the gorge and its banks exact, ramps on the banks.
    let zone = smoothstep(R_JUMP_X.0 - 16.0, R_JUMP_X.0, x) * (1.0 - smoothstep(R_JUMP_X.1, R_JUMP_X.1 + 16.0, x));
    if zone > 0.0 && y.abs() < 72.0 {
        let target = if y.abs() < 4.0 { R_RIVER } else { R_BANK };
        let t = if y.abs() <= 8.0 { zone } else { zone * (1.0 - smoothstep(54.0, 72.0, y.abs())) };
        h = lerp(h, target, t) + r_ramp(x, y);
    }
    h
}

fn red_weights(p: &Spot) -> [f32; 4] {
    // Obsidian is the rock mask (steep ground, crests, slope breaks),
    // not on the lava.
    let steep = smoothstep(0.3, 0.8, p.slope);
    let lava = smoothstep(4.0, 8.0, r_river(p.x, p.y)) + disc(p.x, p.y, 0.0, 0.0, 6.0, 18.0) * 0.9;
    let low = smoothstep(-1.0, 2.0, p.low + 3.0 * (vnoise(31, p.x, p.y, 20) - 0.5));
    let flat = (1.0 - steep) * (1.0 - lava.min(1.0));
    let r = rock(p, 41) * (1.0 - lava.min(1.0));
    with_rock([flat * (1.0 - low * 0.8) + steep * 0.5, 0.0, lava * 2.0 + flat * low * 1.6 + flat * 0.15, flat * 0.3 + steep * 0.4], 1, r)
}

fn cerebral_caldera() -> Map {
    let w = World {
        id: "tt3_5",
        name: "Cerebral Caldera",
        desc: "A volcano in a ring of obsidian spires; lava rivers split the land: take the causeway, jump the gorge or brave the caldera.",
        music: "Frantic",
        layout: "Cerebral Caldera (tt3_5): our own red map (a 400 m mission area).
                 Layout (x east, y north; green spawns south, blue north):
                   (0, 0)       the volcano on a level pad (106 m) in a caldera, the rim (r 44) ringed
                                by obsidian spires, open to the north and south;
                   lava rivers (7 m deep) run from the caldera to the east and west edges;
                   (-98, y)     a natural causeway over the west river, a reload arch on it;
                   (82, -8), (110, 8)  ramp jumps over a 16 m gorge of the east river (lips at the
                                banks, boost pads before them);
                   (-70, 120), (70, -120)  plateaus (+11 m) with ramps, a power-up on top;
                   obsidian ridges north-east and south-west, small ridges, gullies and mounds
                   everywhere; health in the east and west, jump pads out of the rivers.",
        sky: sky("skyred.dml", 1000, "0.600000 0.000000 0.000000 0.000000", 300, "0.600000 0.000000 0.000000 1.000000", 90),
        sun: sun("0.642885 0.642885 -0.416411", "1.000000 0.985000 0.960000 1.000000", "0.700000 0.600000 0.500000 1.000000"),
        floor: 90,
        half: R_HALF,
        materials: ["terrain/red/redterrain02.png", "terrain/red/redterrain04.png", "terrain/red/redterrain01.png", "terrain/red/redterrain03.png"],
    };
    // Slopes capped at about 45 degrees (no stretched cliffs).
    let e = Erosion { talus: 6.8, ..Erosion::default() };
    // The gorge jumps and the volcano's pad stay exact.
    let mask = |x: f32, y: f32| (1.0 - (1.0 - smoothstep(60.0, 70.0, y.abs())) * (1.0 - smoothstep(30.0, 38.0, (x - 96.0).abs()))) * smoothstep(R_PAD.0, R_PAD.1 + 4.0, x.hypot(y));
    // The lava rivers are flushed.
    let flush = |x: f32, y: f32| 1.0 - 0.7 * r_river_strength(x) * (1.0 - smoothstep(10.0, 22.0, (y - r_river_y(x)).abs()));
    let bed = |x: f32, y: f32| if r_river_strength(x) > 0.0 && (y - r_river_y(x)).abs() < 30.0 { R_RIVER - 0.5 } else { 100.0 };
    let heights = heightfield(R_SEED, R_HALF, e, |x, y| red_stamp(x, y, red_shape(x, y)), mask, flush, bed, |x, y| outer_hills(R_SEED, x, y, 130.0, 70.0));
    let mut m = Map::new(w, heights, red_weights);
    m.keep_out.push((0.0, 0.0, 20.0));
    for k in -20..=20 {
        let x = k as f32 * 9.0;
        if r_river_strength(x) > 0.3 {
            m.keep_out.push((x, r_river_y(x), 12.0));
        }
    }
    for k in -5..=5 {
        m.keep_out.push((R_CAUSEWAY_X, r_river_y(R_CAUSEWAY_X) + k as f32 * 12.0, 11.0));
    }
    for (rx, side) in R_RAMPS {
        for d in [14.0f32, 26.0, 38.0, 50.0, 62.0, 74.0] {
            m.keep_out.push((rx, -side * d, 10.0));
            m.keep_out.push((rx, side * d, 10.0));
        }
    }
    // The pads first (their platforms reshape the ground around them).
    // Boost pads before the ramps (the same launch for every tank type).
    m.defs.push(jump_boost());
    for (k, (rx, side)) in R_RAMPS.iter().enumerate() {
        let y = side * 42.0;
        m.trigger("Boost", ["BoostSouth", "BoostNorth"][k], "JumpBoost", *rx, y, if *side < 0.0 { 0.0 } else { 180.0 });
    }
    // Jump pads on the river floors to get out.
    m.trigger("Boost", "RiverJumpW", "TestJump", -62.0, r_river_y(-62.0), 0.0);
    m.trigger("Boost", "RiverJumpE", "TestJump", 145.0, r_river_y(145.0), 180.0);
    // The volcano and its target zones.
    m.volcano(0.0, 0.0);
    for (k, (x, y)) in [(20.0, 14.0), (-18.0, -20.0), (0.0, 60.0), (-8.0, -64.0), (64.0, 20.0)].iter().enumerate() {
        m.marker(&format!("target{}", k + 1), *x, *y);
    }
    // Obsidian spires on the caldera rim, irregular.
    let mut a = 26.0f32;
    let mut k = 0;
    while a < 360.0 {
        let gap = [0.0f32, 180.0].iter().map(|g| ((a - g + 540.0).rem_euclid(360.0) - 180.0).abs()).fold(360.0, f32::min);
        if gap > 20.0 {
            let r = 43.0 + 4.0 * ((k * 7 % 5) as f32 - 2.0) * 0.5;
            let (x, y) = (r * a.to_radians().sin(), r * a.to_radians().cos());
            let s = [1.0, 1.3, 0.85, 1.15, 0.95][k % 5];
            m.prop("props/red/rockred02", x, y, 0.3, a + 140.0 + k as f32 * 37.0, s);
            if k % 3 == 0 {
                m.prop("props/red/rockred02", x + 4.0, y - 3.0, 0.3, a + 60.0, s * 0.7);
            }
            m.keep_out.push((x, y, 5.0));
            k += 1;
        }
        a += 19.0 + (k % 3) as f32 * 6.0;
    }
    for (x, y) in [(-130.0, -150.0), (-85.0, -160.0), (-40.0, -148.0), (15.0, -160.0), (60.0, -150.0), (140.0, -150.0), (-160.0, -95.0), (160.0, -70.0)] {
        m.spawn(false, x, y, 0.0);
    }
    for (x, y) in [(130.0, 150.0), (85.0, 160.0), (40.0, 148.0), (-15.0, 160.0), (-60.0, 150.0), (-140.0, 150.0), (160.0, 95.0), (-160.0, 70.0)] {
        m.spawn(true, x, y, 0.0);
    }
    // The caldera's pair on the volcano's pad, clear of its foot (r 24).
    let pups = [(29.0, -8.0), (-29.0, 8.0), (R_PLATEAU[0].0, R_PLATEAU[0].1), (R_PLATEAU[1].0, R_PLATEAU[1].1), (-48.0, -90.0), (48.0, 90.0)];
    for (k, (x, y)) in pups.iter().enumerate() {
        m.power_up(&format!("pup{}", k + 1), "TestPowerup", *x, *y);
    }
    let wy = r_river_y(R_CAUSEWAY_X);
    m.trigger("PowerUp", "ReloadWest", "Reload", R_CAUSEWAY_X, wy, 0.0);
    m.power_up("ReloadEast", "Reload", 148.0, -30.0);
    m.power_up("HealWest", "Heal", -150.0, 15.0);
    m.power_up("HealEast", "Heal", 150.0, 30.0);
    // Cover: obsidian and rock clusters everywhere outside the caldera.
    let rocks = ["props/red/rockred01", "props/red/rockred03"];
    m.clusters(&rocks, 18, 4, 12.0, (0.7, 1.1), 0.3, |m, x, y| x.hypot(y) > 62.0 && m.roughness(x, y, 8.0) < 0.5);
    m.clusters(&["props/red/rockred02", "props/red/rockred01"], 10, 4, 12.0, (0.6, 1.1), 0.3, |_, x, y| R_RIDGES.iter().any(|c| (x - c.0).hypot(y - c.1) < 60.0));
    m.clusters(&["props/red/rockred03", "props/red/rockred02"], 12, 2, 8.0, (0.5, 0.8), 0.3, |m, x, y| x.hypot(y) > 58.0 && m.roughness(x, y, 8.0) < 0.3);
    m
}

fn main() {
    let mut dir = std::path::PathBuf::from("assets/missions");
    let mut preview: Option<std::path::PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--preview" {
            preview = args.next().map(Into::into);
        } else {
            dir = a.into();
        }
    }
    for m in [amygdala_abyss(), thalamus_pass(), cerebral_caldera()] {
        m.write(&dir);
        if let Some(p) = &preview {
            Grid(m.heights.clone()).previews(p, m.w.id, m.w.half).expect("preview images");
        }
    }
}

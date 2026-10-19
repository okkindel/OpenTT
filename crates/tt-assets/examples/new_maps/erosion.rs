//! The terrain tool of the map generator: a 128 × 128 heightfield (8 m
//! squares, repeating) worn down by droplet erosion (water carving
//! channels and dropping its sediment in hollows), thermal erosion to a
//! stable talus slope and a final hard slope cap, then preview images.
//! Everything is seeded and deterministic.
//!
//! Erosion runs over the whole field after the hand-designed features are
//! cut in, so ravines, canyons and channels get worn, slumped walls; a
//! mask keeps the few spots exact that gameplay needs exact (jump lips and
//! their landing banks).

use std::path::Path;

pub const SIZE: usize = 128;
pub const SQUARE: f32 = 8.0;
pub const ORIGIN: f32 = -(SIZE as f32) * SQUARE / 2.0;

pub fn grid_xy(i: usize, j: usize) -> (f32, f32) {
    (ORIGIN + i as f32 * SQUARE, ORIGIN + j as f32 * SQUARE)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Erosion settings. Slopes are metres of rise per 8 m grid square.
#[derive(Clone, Copy, Debug)]
pub struct Erosion {
    /// Water droplets (each runs up to `steps` cells).
    pub drops: usize,
    pub steps: usize,
    /// How much a droplet keeps its direction (0..1).
    pub inertia: f32,
    /// Sediment capacity factor and the smallest slope it counts.
    pub capacity: f32,
    pub min_slope: f32,
    /// Fractions of the free capacity eroded / of the excess deposited per step.
    pub erode: f32,
    pub deposit: f32,
    pub evaporate: f32,
    /// Erosion brush radius in cells (channels a few squares wide).
    pub radius: i32,
    /// The most a step erodes at one spot, metres.
    pub max_cut: f32,
    /// Thermal erosion: the stable slope (m per square) and passes.
    pub talus: f32,
    pub thermal_passes: usize,
    /// The final hard slope cap (m per square; 9.5 ≈ 50 degrees).
    pub cap: f32,
    /// Creases are relaxed down to this Laplacian, metres, in at most
    /// `relax_passes` passes.
    pub crease: f32,
    pub relax_passes: usize,
}

impl Default for Erosion {
    fn default() -> Erosion {
        Erosion { drops: 45_000, steps: 64, inertia: 0.25, capacity: 3.0, min_slope: 0.02, erode: 0.2, deposit: 0.2, evaporate: 0.015, radius: 2, max_cut: 0.35, talus: 6.4, thermal_passes: 40, cap: 9.5, crease: 1.2, relax_passes: 30 }
    }
}

#[derive(Clone)]
pub struct Grid(pub Vec<f32>);

fn idx(i: i32, j: i32) -> usize {
    let n = SIZE as i32;
    (j.rem_euclid(n) * n + i.rem_euclid(n)) as usize
}

impl Grid {
    pub fn from_fn(f: impl Fn(f32, f32) -> f32) -> Grid {
        Grid((0..SIZE * SIZE).map(|k| grid_xy(k % SIZE, k / SIZE)).map(|(x, y)| f(x, y)).collect())
    }

    pub fn at(&self, i: i32, j: i32) -> f32 {
        self.0[idx(i, j)]
    }

    fn add(&mut self, i: i32, j: i32, v: f32) {
        self.0[idx(i, j)] += v;
    }

    /// Height and gradient (m per cell) at grid coordinates.
    fn sample(&self, u: f32, v: f32) -> (f32, f32, f32) {
        let (i, j) = (u.floor() as i32, v.floor() as i32);
        let (fx, fy) = (u - i as f32, v - j as f32);
        let (a, b, c, d) = (self.at(i, j), self.at(i + 1, j), self.at(i, j + 1), self.at(i + 1, j + 1));
        (lerp(lerp(a, b, fx), lerp(c, d, fx), fy), lerp(b - a, d - c, fy), lerp(c - a, d - b, fx))
    }

    /// Droplet erosion. `mask` (0..1 per cell) scales how much a cell may
    /// change, `flush` how much sediment settles there (low in the designed
    /// channels: their water carries it on, so they stay open); droplets
    /// start where `start` allows.
    pub fn droplets(&mut self, seed: u32, e: &Erosion, mask: &Grid, flush: &Grid, start: impl Fn(f32, f32) -> bool) {
        let mut rng = seed | 1;
        let mut rand = || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng as f32 / u32::MAX as f32
        };
        // The brush: cells within `radius`, weights falling off linearly.
        let r = e.radius;
        let mut brush: Vec<(i32, i32, f32)> = Vec::new();
        for dj in -r..=r {
            for di in -r..=r {
                let d = ((di * di + dj * dj) as f32).sqrt();
                if d <= r as f32 {
                    brush.push((di, dj, r as f32 + 0.5 - d));
                }
            }
        }
        let total: f32 = brush.iter().map(|b| b.2).sum();
        for b in &mut brush {
            b.2 /= total;
        }
        for _ in 0..e.drops {
            let (mut u, mut v) = (rand() * SIZE as f32, rand() * SIZE as f32);
            if !start(ORIGIN + u * SQUARE, ORIGIN + v * SQUARE) {
                continue;
            }
            let (mut dx, mut dy, mut speed, mut water, mut sediment) = (0.0f32, 0.0f32, 1.0f32, 1.0f32, 0.0f32);
            for _ in 0..e.steps {
                let (h, gx, gy) = self.sample(u, v);
                dx = dx * e.inertia - gx * (1.0 - e.inertia);
                dy = dy * e.inertia - gy * (1.0 - e.inertia);
                let l = dx.hypot(dy);
                if l < 1e-6 {
                    break;
                }
                dx /= l;
                dy /= l;
                let (i, j) = (u.floor() as i32, v.floor() as i32);
                let (fx, fy) = (u - i as f32, v - j as f32);
                let (nu, nv) = (u + dx, v + dy);
                if !start(ORIGIN + nu * SQUARE, ORIGIN + nv * SQUARE) {
                    break;
                }
                let dh = self.sample(nu, nv).0 - h;
                let capacity = (-dh).max(e.min_slope) * speed * water * e.capacity;
                if sediment > capacity || dh > 0.0 {
                    // Deposit at the droplet's cell corners (fills hollows).
                    let amount = if dh > 0.0 { dh.min(sediment) } else { (sediment - capacity) * e.deposit };
                    sediment -= amount;
                    for (ci, cj, w) in [(i, j, (1.0 - fx) * (1.0 - fy)), (i + 1, j, fx * (1.0 - fy)), (i, j + 1, (1.0 - fx) * fy), (i + 1, j + 1, fx * fy)] {
                        self.add(ci, cj, amount * w * mask.at(ci, cj) * flush.at(ci, cj));
                    }
                } else {
                    // Erode over the brush (carves channels).
                    let amount = ((capacity - sediment) * e.erode).min(-dh).min(e.max_cut);
                    for &(di, dj, w) in &brush {
                        let (ci, cj) = (i + di, j + dj);
                        let cut = amount * w * mask.at(ci, cj);
                        self.add(ci, cj, -cut);
                        sediment += cut;
                    }
                }
                speed = (speed * speed + (-dh)).max(0.0).sqrt();
                water *= 1.0 - e.evaporate;
                u = nu;
                v = nv;
            }
        }
    }

    /// Thermal erosion: neighbours (including diagonals) steeper than
    /// `talus` per square shed material downhill; `mask` as above.
    pub fn thermal(&mut self, talus: f32, passes: usize, mask: &Grid) {
        let n = SIZE as i32;
        let neighbours = [(1, 0, 1.0f32), (0, 1, 1.0), (1, 1, std::f32::consts::SQRT_2), (1, -1, std::f32::consts::SQRT_2)];
        for _ in 0..passes {
            let mut delta = vec![0.0f32; SIZE * SIZE];
            for j in 0..n {
                for i in 0..n {
                    let h = self.at(i, j);
                    for &(di, dj, dist) in &neighbours {
                        let d = h - self.at(i + di, j + dj);
                        let limit = talus * dist;
                        if d.abs() > limit {
                            let k = mask.at(i, j).min(mask.at(i + di, j + dj));
                            let m = (d.abs() - limit) * 0.2 * d.signum() * k;
                            delta[idx(i, j)] -= m;
                            delta[idx(i + di, j + dj)] += m;
                        }
                    }
                }
            }
            for (h, d) in self.0.iter_mut().zip(delta) {
                *h += d;
            }
        }
    }

    /// The hard slope cap: no neighbour pair differs by more than `cap`
    /// per square (masked cells stay; their neighbours give way).
    pub fn cap(&mut self, cap: f32, mask: &Grid) {
        let n = SIZE as i32;
        for _ in 0..400 {
            let mut worst = 0.0f32;
            for j in 0..n {
                for i in 0..n {
                    for (di, dj) in [(1, 0), (0, 1)] {
                        let (a, b) = (self.at(i, j), self.at(i + di, j + dj));
                        let excess = (a - b).abs() - cap;
                        if excess <= 0.0 {
                            continue;
                        }
                        worst = worst.max(excess);
                        let (ka, kb) = (mask.at(i, j), mask.at(i + di, j + dj));
                        if ka + kb <= 0.0 {
                            continue;
                        }
                        // The higher cell comes down, the lower goes up, shared by the mask.
                        let s = (a - b).signum();
                        let (wa, wb) = (ka / (ka + kb), kb / (ka + kb));
                        self.0[idx(i, j)] -= s * excess * wa * 0.5;
                        self.0[idx(i + di, j + dj)] += s * excess * wb * 0.5;
                    }
                }
            }
            if worst < 0.05 {
                break;
            }
        }
    }

    /// Box blur mixed in by `k`, scaled by the mask.
    pub fn soften(&mut self, k: f32, mask: &Grid) {
        let n = SIZE as i32;
        let old = self.clone();
        for j in 0..n {
            for i in 0..n {
                let mut s = 0.0;
                for dj in -1..=1 {
                    for di in -1..=1 {
                        s += old.at(i + di, j + dj);
                    }
                }
                self.0[idx(i, j)] = lerp(old.at(i, j), s / 9.0, k * mask.at(i, j));
            }
        }
    }

    /// Mean height in a (2r+1)² window (for "how low is this spot").
    pub fn blurred(&self, r: i32) -> Grid {
        let n = SIZE as i32;
        let mut out = vec![0.0; SIZE * SIZE];
        for j in 0..n {
            for i in 0..n {
                let mut s = 0.0;
                for dj in -r..=r {
                    for di in -r..=r {
                        s += self.at(i + di, j + dj);
                    }
                }
                out[idx(i, j)] = s / ((2 * r + 1) * (2 * r + 1)) as f32;
            }
        }
        Grid(out)
    }

    /// The steepest rise to a 4-neighbour, per metre.
    pub fn slope(&self, i: i32, j: i32) -> f32 {
        let h = self.at(i, j);
        [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().map(|&(di, dj)| (self.at(i + di, j + dj) - h).abs()).fold(0.0, f32::max) / SQUARE
    }

    /// Erodes the field: droplets, thermal erosion to the talus slope, a
    /// light blur, the hard slope cap.
    /// `bed` is the bedrock: erosion cuts no deeper (the designed channel
    /// floors).
    pub fn wear(&mut self, seed: u32, e: &Erosion, mask: &Grid, flush: &Grid, bed: &Grid, start: impl Fn(f32, f32) -> bool) {
        self.droplets(seed, e, mask, flush, start);
        self.rest_on(bed);
        self.thermal(e.talus, e.thermal_passes, mask);
        self.soften(0.25, mask);
        self.cap(e.cap, mask);
        self.relax(e.crease, e.relax_passes, mask);
        self.cap(e.cap, mask);
        self.rest_on(bed);
    }

    /// The Laplacian: the 4-neighbour mean minus the height (> 0 in
    /// hollows and at the foot of slopes, < 0 on crests and rims).
    pub fn laplacian(&self, i: i32, j: i32) -> f32 {
        (self.at(i + 1, j) + self.at(i - 1, j) + self.at(i, j + 1) + self.at(i, j - 1)) / 4.0 - self.at(i, j)
    }

    /// Relaxes curvature spikes: where the Laplacian exceeds `limit` metres
    /// (a crease: a rim, a ramp's foot), the cell moves towards its
    /// neighbours by the excess, scaled by the mask. Broad shapes stay.
    pub fn relax(&mut self, limit: f32, passes: usize, mask: &Grid) {
        let n = SIZE as i32;
        for _ in 0..passes {
            let old = self.clone();
            let mut moved = false;
            for j in 0..n {
                for i in 0..n {
                    let l = old.laplacian(i, j);
                    if l.abs() > limit {
                        self.0[idx(i, j)] += (l - limit * l.signum()) * 0.5 * mask.at(i, j);
                        moved = true;
                    }
                }
            }
            if !moved {
                break;
            }
        }
    }

    /// Nothing below the bedrock.
    pub fn rest_on(&mut self, bed: &Grid) {
        for (h, b) in self.0.iter_mut().zip(&bed.0) {
            *h = h.max(*b);
        }
    }

    /// Preview images of the area within `half` metres of the centre:
    /// `<id>_relief.png` (shaded relief with 4 m contours),
    /// `<id>_slope.png` (slope: green flat .. yellow 30 degrees .. red over
    /// 45 degrees, magenta over 55) and `<id>_curvature.png` (the
    /// Laplacian: blue hollows, red crests, white where it exceeds 2 m —
    /// creases).
    pub fn previews(&self, dir: &Path, id: &str, half: f32) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let cells = ((half + 32.0) / SQUARE).ceil() as i32;
        let c = SIZE as i32 / 2;
        let px = 6u32;
        let side = (2 * cells) as u32 * px;
        let (mut relief, mut slope, mut curve) = (image::RgbImage::new(side, side), image::RgbImage::new(side, side), image::RgbImage::new(side, side));
        let (lo, hi) = (90.0f32, 160.0f32);
        for jj in 0..2 * cells {
            for ii in 0..2 * cells {
                let (i, j) = (c - cells + ii, c - cells + jj);
                let h = self.at(i, j);
                let gx = (self.at(i + 1, j) - self.at(i - 1, j)) / (2.0 * SQUARE);
                let gy = (self.at(i, j + 1) - self.at(i, j - 1)) / (2.0 * SQUARE);
                let shade = ((-gx + gy) * 0.7 + 1.0) / (gx * gx + gy * gy + 1.0).sqrt() * 0.75;
                let t = ((h - lo) / (hi - lo)).clamp(0.0, 1.0);
                let mut rgb = [lerp(70.0, 235.0, t), lerp(110.0, 215.0, t), lerp(60.0, 160.0, t)].map(|v| v * shade.clamp(0.25, 1.2));
                if h.rem_euclid(4.0) < 0.3 {
                    rgb = rgb.map(|v| v * 0.75);
                }
                let deg = self.slope(i, j).atan().to_degrees();
                let s = if deg > 55.0 {
                    [255.0, 0.0, 255.0]
                } else if deg > 45.0 {
                    [230.0, 20.0, 20.0]
                } else {
                    let t = deg / 45.0;
                    [lerp(40.0, 240.0, t), lerp(160.0, 210.0, t), lerp(40.0, 30.0, t)]
                };
                let l = self.laplacian(i, j);
                let cv = if l.abs() > 2.0 {
                    [255.0, 255.0, 255.0]
                } else {
                    let t = (l.abs() / 2.0).min(1.0);
                    if l > 0.0 {
                        [lerp(40.0, 40.0, t), lerp(40.0, 90.0, t), lerp(40.0, 255.0, t)]
                    } else {
                        [lerp(40.0, 255.0, t), lerp(40.0, 70.0, t), lerp(40.0, 40.0, t)]
                    }
                };
                for dy in 0..px {
                    for dx in 0..px {
                        // North up.
                        let (x, y) = (ii as u32 * px + dx, side - 1 - (jj as u32 * px + dy));
                        relief.put_pixel(x, y, image::Rgb(rgb.map(|v| v.clamp(0.0, 255.0) as u8)));
                        slope.put_pixel(x, y, image::Rgb(s.map(|v| v as u8)));
                        curve.put_pixel(x, y, image::Rgb(cv.map(|v| v as u8)));
                    }
                }
            }
        }
        // The mission area outline.
        let a = ((cells as f32 - half / SQUARE) * px as f32) as u32;
        for img in [&mut relief, &mut slope, &mut curve] {
            for k in a..side - a {
                for p in [(k, a), (k, side - 1 - a), (a, k), (side - 1 - a, k)] {
                    img.put_pixel(p.0, p.1, image::Rgb([255, 255, 255]));
                }
            }
        }
        relief.save(dir.join(format!("{id}_relief.png"))).map_err(std::io::Error::other)?;
        slope.save(dir.join(format!("{id}_slope.png"))).map_err(std::io::Error::other)?;
        curve.save(dir.join(format!("{id}_curvature.png"))).map_err(std::io::Error::other)?;
        Ok(())
    }
}

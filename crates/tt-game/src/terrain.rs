//! Terrain heightfield queries, as `Terrain` answers them.
//!
//! Each grid square is split into two triangles. The split direction follows
//! the engine's grid builder: on the finest level a square (x, y) gets the
//! `Split45` flag when `(x ^ y) & 1 == 0`, i.e. a checkerboard. `Split45`
//! squares are cut along the (0,0)–(1,1) diagonal, the others along
//! (1,0)–(0,1); the height/normal query confirms which triangle is used on
//! each side of the diagonal. Heights are exact on the triangles (the
//! vertical terrain ray cast of the tank probes returns the same value);
//! [`HeightField::height_bilinear`] is provided for comparison.

use glam::{Vec2, Vec3};

/// Square-grid heightfield that repeats infinitely.
#[derive(Debug, Clone)]
pub struct HeightField {
    size: usize,
    heights: Vec<f32>,
    /// Metres per grid square.
    pub square_size: f32,
    /// World x and y of grid point (0, 0).
    pub origin: Vec2,
}

impl HeightField {
    /// From a parsed `.ter` file. TT terrain blocks are centred on the world
    /// origin (see `tt_assets::terrain`).
    pub fn from_terrain(t: &tt_assets::terrain::Terrain, square_size: f32) -> HeightField {
        let o = t.origin(square_size);
        HeightField {
            size: t.size(),
            heights: t.heights.iter().map(|&h| h as f32 / 32.0).collect(),
            square_size,
            origin: Vec2::splat(o),
        }
    }

    /// `size`×`size` samples (row-major, x fastest).
    pub fn new(size: usize, heights: Vec<f32>, square_size: f32, origin: Vec2) -> HeightField {
        assert!(size > 0 && heights.len() == size * size);
        HeightField { size, heights, square_size, origin }
    }

    /// An infinite plane at height `z`.
    pub fn flat(z: f32) -> HeightField {
        HeightField::new(2, vec![z; 4], 8.0, Vec2::ZERO)
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Height of grid point (x, y), wrapping.
    pub fn grid_height(&self, x: i64, y: i64) -> f32 {
        let s = self.size as i64;
        self.heights[(y.rem_euclid(s) as usize) * self.size + x.rem_euclid(s) as usize]
    }

    /// True when square (x, y) is cut along its (0,0)–(1,1) diagonal.
    pub fn split45(x: i64, y: i64) -> bool {
        (x ^ y) & 1 == 0
    }

    fn locate(&self, x: f32, y: f32) -> (i64, i64, f32, f32) {
        let gx = (x - self.origin.x) / self.square_size;
        let gy = (y - self.origin.y) / self.square_size;
        let (fx, fy) = (gx.floor(), gy.floor());
        (fx as i64, fy as i64, gx - fx, gy - fy)
    }

    /// Exact height and (unnormalised, upward) normal at world (x, y).
    pub fn height_and_normal(&self, x: f32, y: f32) -> (f32, Vec3) {
        let (ix, iy, fx, fy) = self.locate(x, y);
        let bl = self.grid_height(ix, iy);
        let br = self.grid_height(ix + 1, iy);
        let tl = self.grid_height(ix, iy + 1);
        let tr = self.grid_height(ix + 1, iy + 1);
        let s = self.square_size;
        if Self::split45(ix, iy) {
            if fx <= fy {
                (bl + (tl - bl) * fy + (tr - tl) * fx, Vec3::new(tl - tr, bl - tl, s))
            } else {
                (bl + (br - bl) * fx + (tr - br) * fy, Vec3::new(bl - br, br - tr, s))
            }
        } else if 1.0 - fx <= fy {
            (br + (tl - tr) * (1.0 - fx) + (tr - br) * fy, Vec3::new(tl - tr, br - tr, s))
        } else {
            (br + (bl - br) * (1.0 - fx) + (tl - bl) * fy, Vec3::new(bl - br, bl - tl, s))
        }
    }

    pub fn height(&self, x: f32, y: f32) -> f32 {
        self.height_and_normal(x, y).0
    }

    /// Unit surface normal at world (x, y).
    pub fn normal(&self, x: f32, y: f32) -> Vec3 {
        self.height_and_normal(x, y).1.normalize()
    }

    /// First crossing of segment a→b from above the surface to below it.
    /// Returns (t, point, unit normal). Exact: the segment is cut at every
    /// grid line and triangle diagonal, and the height difference is linear
    /// on each piece.
    pub fn segment_hit(&self, a: Vec3, b: Vec3) -> Option<(f32, Vec3, Vec3)> {
        let d = b - a;
        let s = self.square_size;
        let g = |p: Vec2| (p - self.origin) / s;
        let (ga, gb) = (g(a.truncate()), g(b.truncate()));
        let mut ts = vec![0.0f32, 1.0];
        for axis in 0..2 {
            let (p0, p1) = (ga[axis], gb[axis]);
            if (p1 - p0).abs() > 1e-9 {
                let (lo, hi) = (p0.min(p1), p0.max(p1));
                let mut k = lo.ceil();
                while k <= hi {
                    ts.push((k - p0) / (p1 - p0));
                    k += 1.0;
                }
            }
        }
        ts.sort_by(|x, y| x.total_cmp(y));
        let f = |t: f32| {
            let p = a + d * t;
            p.z - self.height(p.x, p.y)
        };
        let mut prev_t = 0.0;
        let mut prev_f = f(0.0);
        if prev_f < 0.0 {
            return None;
        }
        let mut pieces = Vec::with_capacity(4);
        for w in ts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if t1 - t0 <= 0.0 {
                continue;
            }
            pieces.clear();
            // Diagonal crossing inside this cell.
            let mid = ga + (gb - ga) * (0.5 * (t0 + t1));
            let (cx, cy) = (mid.x.floor(), mid.y.floor());
            let lp = |t: f32| ga + (gb - ga) * t - Vec2::new(cx, cy);
            let (l0, l1) = (lp(t0), lp(t1));
            let diag = |l: Vec2| if Self::split45(cx as i64, cy as i64) { l.x - l.y } else { l.x + l.y - 1.0 };
            let (q0, q1) = (diag(l0), diag(l1));
            if q0 * q1 < 0.0 {
                pieces.push(t0 + (t1 - t0) * q0 / (q0 - q1));
            }
            pieces.push(t1);
            for &t in &pieces {
                let ft = f(t);
                if ft < 0.0 {
                    let th = if prev_f - ft > 0.0 { prev_t + (t - prev_t) * prev_f / (prev_f - ft) } else { prev_t };
                    let p = a + d * th;
                    let n = self.normal(p.x, p.y);
                    return Some((th, Vec3::new(p.x, p.y, self.height(p.x, p.y)), n));
                }
                prev_t = t;
                prev_f = ft;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bumpy() -> HeightField {
        let n = 8;
        let h = (0..n * n).map(|i| ((i * 7919) % 13) as f32).collect();
        HeightField::new(n, h, 8.0, Vec2::new(-32.0, -32.0))
    }

    #[test]
    fn triangles_match_corners_and_are_continuous() {
        let t = bumpy();
        for y in -3..3 {
            for x in -3..3 {
                let (wx, wy) = (-32.0 + x as f32 * 8.0, -32.0 + y as f32 * 8.0);
                assert!((t.height(wx, wy) - t.grid_height(x, y)).abs() < 1e-4);
                // Continuity across the diagonals and edges.
                for k in 1..8 {
                    let f = k as f32 / 8.0;
                    let e = 1e-3;
                    let a = t.height(wx + f * 8.0, wy + f * 8.0 - e);
                    let b = t.height(wx + f * 8.0, wy + f * 8.0 + e);
                    assert!((a - b).abs() < 0.02, "{a} {b}");
                }
            }
        }
    }

    #[test]
    fn normal_is_gradient() {
        let t = bumpy();
        for &(x, y) in &[(1.3, 2.7), (5.5, 1.2), (-9.1, 3.3), (12.0, -7.5)] {
            let e = 1e-2;
            let gx = (t.height(x + e, y) - t.height(x - e, y)) / (2.0 * e);
            let gy = (t.height(x, y + e) - t.height(x, y - e)) / (2.0 * e);
            let n = t.normal(x, y);
            let expect = Vec3::new(-gx, -gy, 1.0).normalize();
            assert!((n - expect).length() < 1e-2, "{n} {expect}");
        }
    }

    #[test]
    fn checkerboard_split() {
        // A single raised corner shows which diagonal is used.
        let mut h = vec![0.0; 16];
        h[0] = 8.0; // grid (0,0)
        let t = HeightField::new(4, h, 1.0, Vec2::ZERO);
        // Square (0,0) is Split45: the point (0.6, 0.4) lies in the lower-right
        // triangle (BL, BR, TR) whose height at that point is 8·(1−0.6) = 3.2.
        assert!((t.height(0.6, 0.4) - 3.2).abs() < 1e-5);
        // Square (-1,0) is not Split45: cut along (1,0)–(0,1).
        assert!(!HeightField::split45(-1, 0));
    }

    #[test]
    fn segment_hits_surface() {
        let t = bumpy();
        let a = Vec3::new(3.0, 4.0, 50.0);
        let b = Vec3::new(30.0, -11.0, -50.0);
        let (th, p, n) = t.segment_hit(a, b).unwrap();
        assert!((p.z - t.height(p.x, p.y)).abs() < 1e-3);
        assert!((a + (b - a) * th - p).length() < 1e-2);
        assert!(n.z > 0.0);
        assert!(t.segment_hit(Vec3::new(0.0, 0.0, 100.0), Vec3::new(1.0, 0.0, 99.0)).is_none());
    }
}


use glam::{Affine3A, Mat3, Quat, Vec3};

pub fn rot_z(angle: f32) -> Mat3 {
    Mat3::from_rotation_z(-angle)
}

pub fn rot_x(angle: f32) -> Mat3 {
    Mat3::from_rotation_x(-angle)
}

/// Converts a `tt_assets` row-major 3x4 transform into an affine transform.
pub fn affine_from_rows(m: &tt_assets::dts::Transform) -> Affine3A {
    let col = |j: usize| Vec3::new(m[0][j], m[1][j], m[2][j]);
    Affine3A::from_cols(col(0).into(), col(1).into(), col(2).into(), col(3).into())
}

/// Object transform from mission fields: `position` "x y z", `rotation` "ax
/// ay az degrees", `scale` "sx sy sz".
pub fn mission_transform(position: &[f32], rotation: &[f32], scale: &[f32]) -> Affine3A {
    let pos = match position {
        [x, y, z, ..] => Vec3::new(*x, *y, *z),
        _ => Vec3::ZERO,
    };
    let rot = match rotation {
        [x, y, z, deg, ..] if Vec3::new(*x, *y, *z).length_squared() > 0.0 => {
            Quat::from_axis_angle(Vec3::new(*x, *y, *z).normalize(), -deg.to_radians())
        }
        _ => Quat::IDENTITY,
    };
    let scale = match scale {
        [x, y, z, ..] => Vec3::new(*x, *y, *z),
        _ => Vec3::ONE,
    };
    Affine3A::from_scale_rotation_translation(scale, rot, pos)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) };

    pub fn new(min: Vec3, max: Vec3) -> Aabb {
        Aabb { min, max }
    }

    pub fn from_bounds(b: &tt_assets::dts::Bounds) -> Aabb {
        Aabb { min: Vec3::from(b.min), max: Vec3::from(b.max) }
    }

    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Aabb {
        let mut b = Aabb::EMPTY;
        for p in points {
            b.extend(p);
        }
        b
    }

    pub fn is_empty(&self) -> bool {
        self.min.cmpgt(self.max).any()
    }

    pub fn extend(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }

    pub fn grow(&self, d: f32) -> Aabb {
        Aabb { min: self.min - Vec3::splat(d), max: self.max + Vec3::splat(d) }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    pub fn overlaps(&self, o: &Aabb) -> bool {
        self.min.cmple(o.max).all() && o.min.cmple(self.max).all()
    }

    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        p.clamp(self.min, self.max)
    }

    pub fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|i| {
            Vec3::new(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            )
        })
    }

    /// World box of this (object-space) box under a transform.
    pub fn transformed(&self, m: &Affine3A) -> Aabb {
        Aabb::from_points(self.corners().map(|c| m.transform_point3(c)))
    }
}

/// Triangle vs oriented box overlap (separating axis test, 13 axes).
/// The triangle is given in the box's frame relative to its centre.
pub fn tri_box_overlap(half: Vec3, v: [Vec3; 3]) -> bool {
    let e = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
    let axes = [Vec3::X, Vec3::Y, Vec3::Z];
    let separated = |axis: Vec3| -> bool {
        if axis.length_squared() < 1e-12 {
            return false;
        }
        let p = [axis.dot(v[0]), axis.dot(v[1]), axis.dot(v[2])];
        let r = half.x * axis.x.abs() + half.y * axis.y.abs() + half.z * axis.z.abs();
        p[0].min(p[1]).min(p[2]) > r || p[0].max(p[1]).max(p[2]) < -r
    };
    for a in axes {
        for ed in e {
            if separated(a.cross(ed)) {
                return false;
            }
        }
    }
    for a in axes {
        if separated(a) {
            return false;
        }
    }
    !separated(e[0].cross(e[1]))
}

/// Segment `a + t (b - a)`, t in [0, 1], against a triangle (both sides).
/// Returns t.
pub fn segment_triangle(a: Vec3, b: Vec3, v: [Vec3; 3]) -> Option<f32> {
    let d = b - a;
    let e1 = v[1] - v[0];
    let e2 = v[2] - v[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = a - v[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let w = d.dot(q) * inv;
    if w < 0.0 || u + w > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (0.0..=1.0).contains(&t).then_some(t)
}

/// Segment against an axis-aligned box (slab test). Returns entry t and the
/// face normal at entry; a segment starting inside reports t = 0 and a normal
/// opposing the direction of travel.
pub fn segment_aabb(a: Vec3, b: Vec3, bx: &Aabb) -> Option<(f32, Vec3)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    let mut normal = -d.normalize_or(Vec3::Z);
    for i in 0..3 {
        if d[i].abs() < 1e-12 {
            if a[i] < bx.min[i] || a[i] > bx.max[i] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d[i];
        let (mut ta, mut tb) = ((bx.min[i] - a[i]) * inv, (bx.max[i] - a[i]) * inv);
        let mut n = Vec3::ZERO;
        n[i] = -d[i].signum();
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        if ta > t0 {
            t0 = ta;
            normal = n;
        }
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, normal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_of_text() {
        // Positive yaw turns forward towards +X (right).
        let f = rot_z(std::f32::consts::FRAC_PI_2) * Vec3::Y;
        assert!((f - Vec3::X).length() < 1e-6);
        // Positive pitch tips forward down.
        let f = rot_x(0.1) * Vec3::Y;
        assert!(f.z < 0.0);
    }

    #[test]
    fn sat_and_rays() {
        let half = Vec3::ONE;
        assert!(tri_box_overlap(half, [Vec3::new(-5.0, -5.0, 0.5), Vec3::new(5.0, -5.0, 0.5), Vec3::new(0.0, 5.0, 0.5)]));
        assert!(!tri_box_overlap(half, [Vec3::new(-5.0, -5.0, 1.5), Vec3::new(5.0, -5.0, 1.5), Vec3::new(0.0, 5.0, 1.5)]));
        // Diagonal plane cutting past the corner.
        assert!(!tri_box_overlap(half, [Vec3::new(2.5, 0.0, 0.0), Vec3::new(0.0, 2.5, 0.0), Vec3::new(0.0, 0.0, 2.5)].map(|v| v + Vec3::splat(0.2))));
        let t = segment_triangle(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, -1.0), [Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)]);
        assert_eq!(t, Some(0.5));
        let (t, n) = segment_aabb(Vec3::new(-3.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0), &Aabb::new(-Vec3::ONE, Vec3::ONE)).unwrap();
        assert!((t - 1.0 / 3.0).abs() < 1e-6 && n == Vec3::NEG_X);
    }
}

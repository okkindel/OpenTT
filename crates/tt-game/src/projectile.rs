//! Projectile state and helpers (spec §4, §6.4 ballistic aim).

use std::sync::Arc;

use glam::Vec3;

use crate::data::ProjectileParams;
use crate::geom::{segment_aabb, Aabb};
use crate::rng::Rng;
use crate::tank::Tank;
use crate::{ProjectileId, TankId};

#[derive(Debug, Clone)]
pub struct Projectile {
    pub id: ProjectileId,
    pub data: Arc<ProjectileParams>,
    pub pos: Vec3,
    /// Position at the start of the last tick.
    pub prev_pos: Vec3,
    pub vel: Vec3,
    /// Shooter (None for volcano rocks).
    pub source: Option<TankId>,
    /// Shooter's `TankParams.damage_scale` (1 without a shooter).
    pub damage_scale: f32,
    pub bounces: i32,
    /// Time of the explosion; the projectile is deleted 1000 ms later.
    pub exploded_at: Option<i64>,
    /// Cosmetic projectile with its spawn time: it never explodes and
    /// is deleted 5000 ms after spawning.
    pub cosmetic_since: Option<i64>,
    /// Simulation time it was spawned at (ms; `max_life_ms`).
    pub born_ms: i64,
    pub deleted: bool,
}

impl Projectile {
    pub fn is_live(&self) -> bool {
        !self.deleted && self.exploded_at.is_none()
    }

    /// Render position between ticks, `alpha` 0 = previous tick, 1 = this
    /// tick. Lerps between the last two tick positions; extrapolating back
    /// along `vel` instead is wrong on a bounce tick (`vel` is already
    /// reflected) and by g·dt² otherwise.
    pub fn render_pos(&self, alpha: f32) -> Vec3 {
        self.prev_pos.lerp(self.pos, alpha)
    }
}

/// What a projectile (or a tank) ran into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitObject {
    Terrain,
    Tank(TankId),
    /// Index into `World::statics`.
    Static(usize),
}

/// Segment against a tank's oriented bounds box: (t, world normal).
// SPEC-GAP: the spec only says the projectile ray includes tanks (mask
// 0x4000); the tank's ray-cast shape is taken to be its bounds box.
pub fn segment_tank(a: Vec3, b: Vec3, tank: &Tank) -> Option<(f32, Vec3)> {
    let inv = tank.rot.transpose();
    let la = inv * (a - tank.pos);
    let lb = inv * (b - tank.pos);
    let bx: Aabb = tank.data.geometry.bounds;
    segment_aabb(la, lb, &bx).map(|(t, n)| (t, tank.rot * n))
}

/// Volcano ballistic solve: launch velocity of speed `v` from `from` to `to`
/// under gravity `g` (m/s², positive down). Picks the low or high arc at
/// random when both exist.
pub fn ballistic(from: Vec3, to: Vec3, v: f32, g: f32, rng: &mut Rng) -> Vec3 {
    let d = to - from;
    if d.length_squared() < 0.001 {
        return Vec3::new(0.0, 0.0, if d.z > 0.0 { v } else { -v });
    }
    let k = g / (v * v);
    let disc = 1.0 - k * (2.0 * d.z + k * (d.x * d.x + d.y * d.y));
    if disc < 1e-6 {
        return Vec3::new(0.0, 0.0, v);
    }
    let s = disc.sqrt();
    let root = if s < 1.0 && rng.rand_f() < 0.5 { 1.0 - s } else { 1.0 + s };
    Vec3::new(d.x, d.y, root / k).normalize() * v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ballistic_reaches_target() {
        let mut rng = Rng::new(3);
        let (from, to) = (Vec3::new(0.0, 0.0, 20.0), Vec3::new(120.0, 40.0, 0.0));
        let (v, g) = (63.0, 10.0 * 0.75);
        for _ in 0..8 {
            let vel = ballistic(from, to, v, g, &mut rng);
            assert!((vel.length() - v).abs() < 1e-3);
            // Analytic flight: time to cover the horizontal distance.
            let h = (to - from).truncate();
            let t = h.length() / vel.truncate().length();
            let z = from.z + vel.z * t - 0.5 * g * t * t;
            assert!((z - to.z).abs() < 0.05, "{z}");
        }
        // Out of range: straight up.
        assert_eq!(ballistic(from, Vec3::new(5000.0, 0.0, 0.0), v, g, &mut rng), Vec3::new(0.0, 0.0, v));
    }
}

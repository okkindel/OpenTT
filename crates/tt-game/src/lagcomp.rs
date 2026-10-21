//! Server-side lag compensation for network shots. A remote player aims at tanks
//! drawn `V` in the past (V ≈ RTT + server move queue + interpolation delay: ~300
//! ms on DSL, measured in `tests/lagcomp.rs`). For a direct-fire shot of such a
//! player the server tests the projectile against the target tanks *rewound* by V
//! while the projectile is younger than V, then by a rewind decaying linearly to
//! zero over the next V; splash weapons are not compensated (their splash covers
//! the error). Damage still goes to the tank as it is now.
//!
//! Only the network server fills [`LagComp::view_delay`].

use std::collections::{HashMap, VecDeque};

use glam::{Mat3, Vec3};

use crate::{ProjectileId, TankId, TICK_MS};

/// The longest view delay compensated (ticks; 600 ms). Larger delays get
/// this much (very bad links accept the loss; it limits "shot behind cover").
pub const MAX_REWIND_TICKS: f32 = 600.0 / TICK_MS as f32;
/// Rewinds shorter than this (ms) are not worth the lookup.
const MIN_REWIND_MS: f32 = 8.0;

/// The tank poses of one tick.
type Poses = Vec<(TankId, Vec3, Mat3)>;

#[derive(Debug, Clone, Default)]
pub struct LagComp {
    /// Per shooter tank: its view delay in ticks (set by the network server
    /// before each tick; absent = not compensated).
    pub view_delay: HashMap<TankId, f32>,
    /// Compensated live projectiles: (view delay ticks, spawn time ms).
    pub(crate) shots: HashMap<ProjectileId, (f32, i64)>,
    /// Tank poses per tick (time ms, poses), newest last.
    history: VecDeque<(i64, Poses)>,
}

impl LagComp {
    pub fn active(&self) -> bool {
        !self.view_delay.is_empty() || !self.shots.is_empty()
    }

    /// A projectile fired by `source`: compensated if its shooter has a view delay.
    pub(crate) fn fired(&mut self, id: ProjectileId, source: Option<TankId>, splash: f32, now: i64) {
        if splash >= 0.1 {
            return;
        }
        if let Some(&v) = source.and_then(|s| self.view_delay.get(&s)) {
            if v > 0.0 {
                self.shots.insert(id, (v.min(MAX_REWIND_TICKS), now));
            }
        }
    }

    /// The rewind (ms) for projectile `id` at time `now`: V while younger
    /// than V, then decaying to 0 over the next V.
    pub fn rewind_ms(&self, id: ProjectileId, now: i64) -> f32 {
        let Some(&(v, t0)) = self.shots.get(&id) else { return 0.0 };
        let cap = v * TICK_MS as f32;
        let age = (now - t0) as f32;
        let w = if age <= cap { 1.0 } else { (1.0 - (age - cap) / cap).max(0.0) };
        let r = cap * w;
        if r < MIN_REWIND_MS {
            0.0
        } else {
            r
        }
    }

    /// Records the tank poses of this tick (only while compensating).
    pub(crate) fn record(&mut self, now: i64, poses: impl Iterator<Item = (TankId, Vec3, Mat3)>) {
        if !self.active() {
            self.history.clear();
            return;
        }
        self.history.push_back((now, poses.collect()));
        let keep = (MAX_REWIND_TICKS.ceil() as usize) + 2;
        while self.history.len() > keep {
            self.history.pop_front();
        }
    }

    /// Forgets shots no longer live.
    pub(crate) fn retain_shots(&mut self, live: impl Fn(ProjectileId) -> bool) {
        self.shots.retain(|id, _| live(*id));
    }

    /// Tank `id`'s pose at time `t` (ms), interpolated between recorded
    /// ticks; `None` if not recorded then (the caller uses the present).
    pub fn pose_at(&self, id: TankId, t: f32) -> Option<(Vec3, Mat3)> {
        let get = |e: &(i64, Poses)| e.1.iter().find(|p| p.0 == id).map(|p| (p.1, p.2));
        let i = self.history.iter().rposition(|e| (e.0 as f32) <= t)?;
        let a = &self.history[i];
        let pa = get(a)?;
        let Some(b) = self.history.get(i + 1) else { return Some(pa) };
        let Some(pb) = get(b) else { return Some(pa) };
        let f = ((t - a.0 as f32) / (b.0 - a.0).max(1) as f32).clamp(0.0, 1.0);
        // Rotation: the nearer tick's (a tick apart, the difference is small).
        Some((pa.0.lerp(pb.0, f), if f < 0.5 { pa.1 } else { pb.1 }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewind_profile_and_history() {
        let mut lc = LagComp::default();
        let (shooter, target) = (TankId(1), TankId(2));
        lc.view_delay.insert(shooter, 9.375); // 300 ms
        lc.fired(ProjectileId(5), Some(shooter), 0.0, 1000);
        lc.fired(ProjectileId(6), Some(shooter), 5.0, 1000);
        lc.fired(ProjectileId(7), Some(target), 0.0, 1000);
        assert_eq!(lc.rewind_ms(ProjectileId(5), 1000), 300.0);
        assert_eq!(lc.rewind_ms(ProjectileId(5), 1300), 300.0);
        assert!((lc.rewind_ms(ProjectileId(5), 1450) - 150.0).abs() < 1e-3);
        assert_eq!(lc.rewind_ms(ProjectileId(5), 1600), 0.0);
        assert_eq!(lc.rewind_ms(ProjectileId(6), 1000), 0.0, "splash is not compensated");
        assert_eq!(lc.rewind_ms(ProjectileId(7), 1000), 0.0, "no view delay");
        for k in 0..40 {
            let t = k * TICK_MS;
            lc.record(t, [(target, Vec3::new(k as f32, 0.0, 0.0), Mat3::IDENTITY)].into_iter());
        }
        let now = 39 * TICK_MS;
        let (p, _) = lc.pose_at(target, now as f32 - 48.0).unwrap();
        assert!((p.x - 37.5).abs() < 1e-4, "{p}");
        assert!(lc.pose_at(target, 0.0).is_none(), "older than the history");
        assert!(lc.pose_at(TankId(9), now as f32).is_none());
        lc.view_delay.clear();
        lc.retain_shots(|_| false);
        lc.record(now, std::iter::empty());
        assert!(!lc.active() && lc.history.is_empty());
    }
}

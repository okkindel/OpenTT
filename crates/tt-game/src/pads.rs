//! Power-ups, boost/jump pads and the volcano (spec §6). The per-tick logic
//! lives in [`crate::World`]; this module holds their state.

use std::sync::Arc;

use glam::{Affine3A, Vec3};

use crate::data::{BoostParams, PowerUpParams, TriggerShape, VolcanoParams};
use crate::rng::Rng;
use crate::TriggerId;

#[derive(Debug, Clone)]
pub struct BoostState {
    pub data: Arc<BoostParams>,
    pub state: usize,
    /// When the state advances (None = never).
    pub next_state_time: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct VolcanoState {
    pub data: Arc<VolcanoParams>,
    pub rocks_left: i32,
    pub next_attack: i64,
    pub focus_zone: usize,
}

#[derive(Debug, Clone)]
pub enum TriggerKind {
    PowerUp(Arc<PowerUpParams>),
    Boost(BoostState),
    Volcano(VolcanoState),
}

/// A `PowerUp`-derived object: pick-up, pad or volcano.
#[derive(Debug, Clone)]
pub struct Trigger {
    pub id: TriggerId,
    /// Object name in the mission (may be empty).
    pub name: String,
    pub transform: Affine3A,
    pub shape: TriggerShape,
    pub on: bool,
    /// While off: time at which it switches back on.
    pub off_until: i64,
    pub kind: TriggerKind,
    /// The user animation thread `playUserThread` started (cosmetic: the
    /// sim never reads it).
    pub thread: Option<UserThread>,
}

/// A one-shot user animation thread on a power-up's shape: `seq` played
/// from `start_ms` at `scale` × speed; non-cyclic sequences then hold
/// their last keyframe.
#[derive(Debug, Clone, PartialEq)]
pub struct UserThread {
    pub slot: u32,
    pub seq: String,
    pub start_ms: i64,
    pub scale: f32,
}

impl UserThread {
    /// Thread time in seconds at `now_s` (sim seconds).
    pub fn time_at(&self, now_s: f32) -> f32 {
        ((now_s - self.start_ms as f32 / 1000.0) * self.scale).max(0.0)
    }
}

impl Trigger {
    pub fn power_up_params(&self) -> &PowerUpParams {
        match &self.kind {
            TriggerKind::PowerUp(d) => d,
            TriggerKind::Boost(b) => &b.data.base,
            TriggerKind::Volcano(v) => &v.data.base,
        }
    }

    pub fn def_name(&self) -> &str {
        &self.power_up_params().name
    }

    /// §6.1 overlap: the tank origin, in the object's frame, against the trigger box. Compares the squared
    /// distance with the unsquared radius `R_tank`, as the engine does.
    pub fn overlaps(&self, point: Vec3, r_tank: f32) -> bool {
        let local = self.transform.inverse().transform_point3(point);
        let c = self.shape.trigger_box.closest_point(local);
        (local - c).length_squared() < r_tank
    }

    /// Returns true when the state changed. A pick-up with `max_off_ms
    /// == 0` never switches off.
    pub fn set_on(&mut self, on: bool, now: i64, rng: &mut Rng) -> bool {
        let d = self.power_up_params();
        let new = if d.max_off_ms != 0 { on } else { true };
        if self.on && !new {
            // OffUntil = ftol((max_off_ms − min_off_ms)·randF + (now + min_off_ms)).
            let (min, max) = (d.min_off_ms, d.max_off_ms);
            self.off_until = crate::rng::ftol_scaled(max.wrapping_sub(min), rng.rand_f(), now + min as i64);
        }
        let changed = self.on != new;
        self.on = new;
        changed
    }

    /// Launch direction of a pad: the Y axis of the `direction` node as the
    /// pad's current state sequence poses it (the 4-state pads turn it a
    /// quarter per state; the arrow mesh hangs below it), else (0,0,1);
    /// either way through the object transform, so a jump pad on a slope
    /// launches along its own up axis, not straight up.
    pub fn boost_direction(&self) -> Vec3 {
        let posed = match &self.kind {
            TriggerKind::Boost(b) => {
                let seq = &b.data.state_name[b.state.min(crate::data::MAX_STATES - 1)];
                self.shape.direction_by_sequence.iter().find(|(n, _)| !seq.is_empty() && n.eq_ignore_ascii_case(seq)).map(|(_, m)| *m)
            }
            _ => None,
        };
        match posed.or(self.shape.direction) {
            Some(n) => (self.transform * n).matrix3.y_axis.into(),
            None => self.transform.matrix3.z_axis.into(),
        }
    }

    /// Volcano rock origin: the `muzzleNode` node, else the top centre of
    /// the bounds (world space).
    pub fn volcano_origin(&self) -> Vec3 {
        let b = &self.shape.bounds;
        let local = self.shape.muzzle.unwrap_or(Vec3::new(0.5 * (b.min.x + b.max.x), 0.5 * (b.min.y + b.max.y), b.max.z));
        self.transform.transform_point3(local)
    }
}

impl BoostState {
    /// Schedules the end of `state`: forever when its `minDuration` is 0,
    /// else `ftol((max − min)·randF + now + min)`.
    pub fn schedule(&mut self, now: i64, rng: &mut Rng) {
        let s = self.state.min(crate::data::MAX_STATES - 1);
        let (min, max) = (self.data.min_duration[s], self.data.max_duration[s]);
        self.next_state_time = if min == 0 { None } else { Some(crate::rng::ftol_scaled(max.wrapping_sub(min), rng.rand_f(), now + min as i64)) };
    }

    /// Only acts on a different state. Returns true when the state
    /// changed.
    pub fn set_state(&mut self, s: usize, now: i64, rng: &mut Rng) -> bool {
        if s == self.state {
            return false;
        }
        self.state = s;
        self.schedule(now, rng);
        true
    }

    pub fn velocity(&self) -> f32 {
        self.data.velocity[self.state.min(crate::data::MAX_STATES - 1)]
    }
}

impl VolcanoState {
    /// Zone pick: `r = rand % (total + 1)` over the zone weights, the focus
    /// zone using its focus weight. Because the range includes `total`, a
    /// pick can fail (the engine then aborts the eruption).
    pub fn pick_zone(&self, rng: &mut Rng) -> Option<usize> {
        let n = self.data.zone_count();
        let w = |i: usize| if i == self.focus_zone { self.data.zones[i].focus_weight } else { self.data.zones[i].weight };
        let total: i32 = (0..n).map(w).sum();
        if total <= 0 {
            return None;
        }
        let mut r = rng.below_inclusive(total as u32) as i32;
        for i in 0..n {
            if r < w(i) {
                return Some(i);
            }
            r -= w(i);
        }
        None
    }
}

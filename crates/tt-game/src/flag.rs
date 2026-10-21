//! The scrum ball: physics, pickup/push, retake cooldown and drop rules.

use std::sync::Arc;

use glam::{Affine3A, Vec3};

use crate::data::FlagParams;
use crate::geom::{tri_box_overlap, Aabb};
use crate::tank::Tank;
use crate::world::{Event, World};
use crate::{FlagId, TankId, TriggerId, DT, GRAVITY, TICK_MS};

#[derive(Debug, Clone)]
pub struct Flag {
    pub id: FlagId,
    pub data: Arc<FlagParams>,
    pub pos: Vec3,
    pub vel: Vec3,
    pub at_rest: bool,
    pub rest_ticks: i32,
    pub carrier: Option<TankId>,
    /// `setGoal` (only the AI uses it).
    pub goal: Option<TriggerId>,
    /// Tank that may not re-take the ball yet.
    pub no_retake: Option<TankId>,
    pub lockout: i32,
    /// Network client: the position comes from snapshots; a loose puppet
    /// ball is not simulated (a carried one still follows its carrier).
    pub puppet: bool,
}

impl Flag {
    pub fn world_box(&self) -> Aabb {
        Aabb::new(self.data.bounds.min + self.pos, self.data.bounds.max + self.pos)
    }
}

impl World {
    pub fn add_flag(&mut self, data: Arc<FlagParams>, pos: Vec3) -> FlagId {
        let id = FlagId(self.alloc_id());
        self.flags.push(Flag { id, data, pos, vel: Vec3::ZERO, at_rest: false, rest_ticks: 0, carrier: None, goal: None, no_retake: None, lockout: 0, puppet: false });
        id
    }

    pub fn flag(&self, id: FlagId) -> Option<&Flag> {
        self.flags.iter().find(|f| f.id == id)
    }

    pub fn flag_mut(&mut self, id: FlagId) -> Option<&mut Flag> {
        self.flags.iter_mut().find(|f| f.id == id)
    }

    /// `flag.setGoal(goal)`.
    pub fn set_goal(&mut self, id: FlagId, goal: Option<TriggerId>) {
        if let Some(f) = self.flag_mut(id) {
            f.goal = goal;
        }
    }

    /// `flag.setTransform(pos)` (scripts reset the ball to a spawn point).
    pub fn set_flag_position(&mut self, id: FlagId, pos: Vec3) {
        if let Some(f) = self.flag_mut(id) {
            f.pos = pos;
            f.vel = Vec3::ZERO;
            f.at_rest = false;
        }
    }

    /// `flag.clearMount()`: releases the ball.
    pub fn clear_mount(&mut self, id: FlagId) {
        let Some(c) = self.flag(id).and_then(|f| f.carrier) else { return };
        if let Some(t) = self.tanks.iter_mut().find(|t| t.id == c) {
            t.carried_flag = None;
        }
        self.release_flag(id, c);
    }

    /// `tank.getFlag()`.
    pub fn get_flag(&self, tank: TankId) -> Option<FlagId> {
        self.tank(tank).and_then(|t| t.carried_flag)
    }

    /// Releases the ball a tank carries (death, hard tank hit).
    pub(crate) fn drop_flag(&mut self, t: &mut Tank) {
        if let Some(flag) = t.carried_flag.take() {
            self.release_flag(flag, t.id);
        }
    }

    /// `mount(NULL)`, the engine's only release path: the old carrier may
    /// not re-take the ball for 30 ticks. The ball keeps the velocity it
    /// copied from the carrier on its last tick, so a ram's bounce does not
    /// carry it off with the carrier.
    pub(crate) fn release_flag(&mut self, flag: FlagId, tank: TankId) {
        if let Some(f) = self.flag_mut(flag) {
            f.carrier = None;
            f.at_rest = false;
            f.no_retake = Some(tank);
            f.lockout = 30;
        }
        self.events.push(Event::FlagDropped { tank, flag });
    }

    /// `flag.mount(tank)`: hands the ball to `tank` (taking it from a carrier).
    pub fn mount_flag(&mut self, flag: FlagId, tank: TankId) {
        let (Some(fi), Some(ti)) = (self.flags.iter().position(|f| f.id == flag), self.tanks.iter().position(|t| t.id == tank && !t.deleted)) else { return };
        let mut t = self.tanks[ti].clone();
        self.mount_at(fi, &mut t);
        self.tanks[ti].carried_flag = t.carried_flag;
    }

    fn mount_at(&mut self, fi: usize, t: &mut Tank) {
        let id = self.flags[fi].id;
        if let Some(old) = self.flags[fi].carrier {
            self.flags[fi].no_retake = Some(old);
            self.flags[fi].lockout = 30;
            if let Some(o) = self.tanks.iter_mut().find(|o| o.id == old) {
                o.carried_flag = None;
            }
        }
        self.flags[fi].carrier = Some(t.id);
        t.carried_flag = Some(id);
        self.events.push(Event::FlagPickedUp { flag: id, tank: t.id });
    }

    /// Pickup and push from the tank trigger code: balls touching the tank's
    /// box grown by R. A held ball is never taken from its carrier; only a
    /// ram's release passes it.
    pub(crate) fn flag_triggers(&mut self, t: &mut Tank) {
        let tb = t.world_box().grow(t.data.geometry.big_radius());
        for fi in 0..self.flags.len() {
            let f = &self.flags[fi];
            if !f.world_box().overlaps(&tb) || f.carrier.is_some() || f.no_retake == Some(t.id) || t.is_corpse {
                continue;
            }
            self.flags[fi].no_retake = None;
            let f = &self.flags[fi];
            if !f.data.no_pickup && t.carried_flag.is_none() {
                if self.server {
                    self.mount_at(fi, t);
                }
            } else {
                let p = t.vel * 1.5;
                let mut dir = f.pos - t.pos;
                if dir.length_squared() > 1e-4 {
                    dir = dir.normalize();
                }
                let mut kick = dir * dir.dot(p);
                kick.z = 0.1 * (kick.x.abs() + kick.y.abs());
                let f = &mut self.flags[fi];
                f.vel += kick;
                f.at_rest = false;
            }
        }
    }

    pub(crate) fn tick_flags(&mut self) {
        for fi in 0..self.flags.len() {
            if let Some(c) = self.flags[fi].carrier {
                match self.tank(c) {
                    Some(t) => {
                        // No codeScrum node: 2 m up the tank's z axis.
                        let node = t.data.geometry.scrum_node.map_or(Vec3::new(0.0, 0.0, 2.0), |n| {
                            t.data.geometry.posed_node(n, t.yaw_n, t.pitch_n, &t.data).translation.into()
                        });
                        let pos = Affine3A::from_mat3_translation(t.rot, t.pos).transform_point3(node);
                        let vel = t.vel;
                        let f = &mut self.flags[fi];
                        f.pos = pos;
                        f.vel = vel;
                        f.at_rest = false;
                        continue;
                    }
                    None => self.flags[fi].carrier = None,
                }
            }
            if self.flags[fi].puppet {
                continue;
            }
            let f = &mut self.flags[fi];
            if f.lockout == 0 {
                f.no_retake = None;
            } else {
                f.lockout -= 1;
            }
            let mut touched_terrain = false;
            if f.at_rest {
                f.rest_ticks += 1;
                if f.rest_ticks as i64 * TICK_MS > f.data.unstuck_time as i64 && self.server {
                    let mut d = crate::rng::random_dir(&mut self.math_rng, Vec3::Z, 0.0, 45.0, 0.0, 360.0);
                    let f = &mut self.flags[fi];
                    if f.pos.x.abs() > 150.0 || f.pos.y.abs() > 150.0 {
                        d.x = -f.pos.x.signum() * d.x.abs();
                        d.y = -f.pos.y.signum() * d.y.abs();
                    }
                    f.vel += d * f.data.unstuck_boost;
                    f.at_rest = false;
                }
            } else {
                let start = f.pos;
                f.vel.z += GRAVITY * self.gravity * DT;
                f.pos += f.vel * DT;
                if let Some(ter) = &self.terrain {
                    let (h, n) = ter.height_and_normal(f.pos.x, f.pos.y);
                    if f.pos.z <= h {
                        let n = n.normalize();
                        f.pos.z = h;
                        let vn = f.vel.dot(n);
                        f.vel = (f.vel - n * vn) * (1.0 - f.data.friction_terrain) - n * (f.data.bounce_terrain * vn);
                        touched_terrain = true;
                    }
                }
                let (end, vel, bounds) = (f.pos, f.vel, f.data.bounds);
                if let Some((p, n)) = self.swept_box_statics(bounds, start, end, vel) {
                    let f = &mut self.flags[fi];
                    f.pos = p;
                    f.vel -= n * ((1.0 + f.data.bounce_static) * f.vel.dot(n));
                }
                let f = &mut self.flags[fi];
                if touched_terrain && f.vel.length_squared() < 0.1 {
                    f.vel = Vec3::ZERO;
                    f.at_rest = true;
                    f.rest_ticks = 0;
                }
            }
            let f = &self.flags[fi];
            let outside = self.mission_area.is_some_and(|m| {
                let (x, y) = (f.pos.x - m.x, f.pos.y - m.y);
                !(x < m.w + 5.0 && y < m.h + 5.0 && x > -5.0 && y > -5.0)
            });
            if self.server && (f.pos.z <= self.floor || outside) {
                self.events.push(Event::FlagOutOfBounds { flag: f.id });
            }
            if touched_terrain {
                self.flags[fi].no_retake = None;
            }
        }
    }

    /// Axis-aligned box swept start→end against static triangles with the
    /// same k/8 binary search as the tank. Returns the last clear position
    /// and the hit normal.
    fn swept_box_statics(&self, bounds: Aabb, start: Vec3, end: Vec3, vel: Vec3) -> Option<(Vec3, Vec3)> {
        let swept = Aabb::new(bounds.min + start.min(end), bounds.max + start.max(end));
        let mut tris = Vec::new();
        for s in &self.statics {
            if s.world_box.overlaps(&swept) && self.static_solid(s) {
                for (t, n) in s.triangles.iter().zip(&s.normals) {
                    if Aabb::from_points(t.iter().copied()).overlaps(&swept) {
                        tris.push((*t, *n));
                    }
                }
            }
        }
        if tris.is_empty() {
            return None;
        }
        let half = bounds.half_extents();
        let vd = vel.normalize_or_zero();
        let sample = |k: u32| -> Option<Vec3> {
            let c = start + (end - start) * (k as f32 * 0.125) + bounds.center();
            let mut best: Option<(f32, Vec3)> = None;
            for (t, n) in &tris {
                if tri_box_overlap(half, t.map(|v| v - c)) {
                    let d = n.dot(vd);
                    if d <= 0.0 && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, *n));
                    }
                }
            }
            best.map(|b| b.1)
        };
        let mut results = [None; 9];
        let (mut k, mut step) = (8u32, 8u32);
        loop {
            let hit = sample(k);
            results[k as usize] = hit;
            if hit.is_none() {
                if k == 8 {
                    return None;
                }
            } else {
                k ^= step;
            }
            step >>= 1;
            k |= step;
            if step == 0 {
                break;
            }
        }
        let n = results[k as usize + 1]?;
        Some((start + (end - start) * (k as f32 * 0.125), n))
    }
}

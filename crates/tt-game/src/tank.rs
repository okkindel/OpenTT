//! Tank state and its movement per tick.

use std::f32::consts::PI;
use std::sync::Arc;

use glam::{Mat3, Vec3};

use crate::data::{ProjectileParams, TankParams};
use crate::geom::{rot_z, Aabb};
use crate::terrain::HeightField;
use crate::{TankId, TriggerId, DT, GRAVITY};

/// Player input for one tick.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Move {
    /// Hull turn, +right, -1..1.
    pub x: f32,
    /// Throttle, +forward, -1..1.
    pub y: f32,
    pub z: f32,
    /// Turret yaw delta in radians (+right).
    pub yaw: f32,
    /// Turret pitch delta in radians (+down).
    pub pitch: f32,
    pub roll: f32,
    /// `trigger[0]` fires.
    pub trigger: [bool; 6],
}

impl Move {
    pub const NULL: Move = Move { x: 0.0, y: 0.0, z: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, trigger: [false; 6] };
}

#[derive(Debug, Clone)]
pub struct Tank {
    pub id: TankId,
    pub data: Arc<TankParams>,
    pub pos: Vec3,
    /// Orientation; columns are right (+X), forward (+Y), up (+Z).
    pub rot: Mat3,
    pub vel: Vec3,
    /// Velocity at the start of the last tick (client sway input).
    pub prev_vel: Vec3,
    /// Input for the next tick (the engine writes it; bots get their AI move here).
    pub input: Move,
    /// Last turn input / throttle seen while in contact.
    pub turn_in: f32,
    pub throttle: f32,
    /// Per-wheel vertical spring velocity (outer RF, RR, LR, LF).
    pub wheel_vel: [f32; 4],
    pub any_contact: bool,
    pub all_contact: bool,
    /// Render-only suspension extension per inner wheel.
    pub susp_display: [f32; 4],
    /// Turret yaw / pitch normalised to [-1, 1].
    pub yaw_n: f32,
    pub pitch_n: f32,
    pub damage: f32,
    pub last_damage_source: Option<TankId>,
    pub is_corpse: bool,
    /// Seconds since death.
    pub corpse_timer: f32,
    pub current: Option<Arc<ProjectileParams>>,
    pub ammo: i32,
    pub saved: Option<Arc<ProjectileParams>>,
    pub saved_ammo: i32,
    /// Simulation time of the last shot (ms).
    pub last_fire: i64,
    pub trigger_released: bool,
    pub burst_left: i32,
    pub fire_count: u32,
    pub dry_fire_count: u32,
    pub boost_impulse: Vec3,
    pub boost_counter: i32,
    pub stuck_ticks: i32,
    /// PowerUp/pad currently overlapped (fires once per entry).
    pub current_trigger: Option<TriggerId>,
    /// Scrum ball carried.
    pub carried_flag: Option<crate::FlagId>,
    /// Bot controller (`setAI`); its move replaces `input` every tick.
    pub ai: Option<Box<crate::ai::Bot>>,
    /// `setTeam`; team 0 tanks are everyone's enemies in a free-for-all.
    pub team: i32,
    /// `setPlayerName`.
    pub name: String,
    /// The brain jar has been ejected from this corpse.
    pub brain_ejected: bool,
    /// Death sequence playing on the corpse: (index into
    /// `TankParams::death_sequences`, its length in seconds).
    pub death_sequence: Option<(usize, f32)>,
    /// Driven by a client connection: the corpse is kept until respawn.
    pub controlled: bool,
    /// Set when the corpse timer ran out; the world removes the tank.
    pub deleted: bool,
    /// Network client: a remote tank whose state comes from snapshots
    /// (interpolated). [`crate::World::tick`] does not simulate it, but it
    /// stays a collision obstacle.
    pub puppet: bool,
}

impl Tank {
    /// A new tank as the engine constructs it (no ammo; the game scripts
    /// call `setProjectile(defaultProjectile)` right after spawning, which is
    /// [`crate::World::set_projectile`]).
    pub fn new(id: TankId, data: Arc<TankParams>, pos: Vec3, rot: Mat3) -> Tank {
        Tank {
            id,
            data,
            pos,
            rot,
            vel: Vec3::ZERO,
            prev_vel: Vec3::ZERO,
            input: Move::NULL,
            turn_in: 0.0,
            throttle: 0.0,
            wheel_vel: [0.0; 4],
            any_contact: false,
            all_contact: false,
            susp_display: [0.0; 4],
            yaw_n: 0.0,
            pitch_n: 0.0,
            damage: 0.0,
            last_damage_source: None,
            is_corpse: false,
            corpse_timer: 0.0,
            current: None,
            ammo: 0,
            saved: None,
            saved_ammo: 0,
            last_fire: 0,
            trigger_released: false,
            burst_left: 1,
            fire_count: 0,
            dry_fire_count: 0,
            boost_impulse: Vec3::ZERO,
            boost_counter: 0,
            stuck_ticks: 0,
            current_trigger: None,
            carried_flag: None,
            ai: None,
            team: 0,
            name: String::new(),
            brain_ejected: false,
            death_sequence: None,
            controlled: false,
            deleted: false,
            puppet: false,
        }
    }

    pub fn is_bot(&self) -> bool {
        self.ai.is_some()
    }

    /// Health shown to the player.
    pub fn health(&self) -> f32 {
        self.data.damage_amount - self.damage
    }

    pub fn forward(&self) -> Vec3 {
        self.rot.y_axis
    }

    /// World box of the shape bounds at the current pose.
    pub fn world_box(&self) -> Aabb {
        self.data.geometry.bounds.transformed(&glam::Affine3A::from_mat3_translation(self.rot, self.pos))
    }

    /// World centre of the shape bounds.
    pub fn center(&self) -> Vec3 {
        self.pos + self.rot * self.data.geometry.bounds.center()
    }

    /// World muzzle transform (Y axis = barrel direction).
    pub fn muzzle(&self) -> glam::Affine3A {
        self.muzzle_at(self.yaw_n, self.pitch_n)
    }

    /// [`Tank::muzzle`] with the turret at `yaw_n` / `pitch_n`.
    pub fn muzzle_at(&self, yaw_n: f32, pitch_n: f32) -> glam::Affine3A {
        glam::Affine3A::from_mat3_translation(self.rot, self.pos) * self.data.geometry.muzzle(yaw_n, pitch_n, &self.data)
    }

    /// The turret pitch (`pitch_n`) that drops a high-angle shell of `p` (the mortar), fired
    /// with the turret at `yaw_n`, on `point` (camera aim's point under the screen centre, a
    /// bot's target): the reach of the shell's real flight
    /// ([`crate::data::ProjectileParams::pitch_n_for_reach`]) from the muzzle to the point's
    /// horizontal distance and height (plus the projectile ray's offset `radius`). The muzzle
    /// moves a little with the pitch: solved twice.
    pub fn high_angle_pitch_n(&self, p: &ProjectileParams, yaw_n: f32, point: Vec3, world_g: f32) -> f32 {
        let mut pn = self.pitch_n;
        for _ in 0..2 {
            let p0 = Vec3::from(self.muzzle_at(yaw_n, pn).translation);
            pn = p.pitch_n_for_reach((point - p0).truncate().length(), point.z + p.radius - p0.z, world_g);
        }
        pn
    }

    /// Turret aim (`yaw_n`, `pitch_n`) that drops a high-angle shell of `p`
    /// on `point`, from a first `yaw_n`: the shell flies along the barrel's
    /// heading, which on a tilted hull shifts as the barrel pitches, so the
    /// yaw is corrected until that heading points at the point
    /// ([`Tank::high_angle_pitch_n`] for the pitch).
    pub fn high_angle_aim(&self, p: &ProjectileParams, yaw_n: f32, point: Vec3, world_g: f32) -> (f32, f32) {
        let range = self.data.turret_yaw_range.to_radians().max(1e-3);
        let mut yaw = yaw_n;
        let mut pn = self.high_angle_pitch_n(p, yaw, point, world_g);
        for _ in 0..10 {
            let m = self.muzzle_at(yaw, pn);
            let want = (point - Vec3::from(m.translation)).truncate();
            let have = Vec3::from(m.matrix3.y_axis).truncate();
            if want.length() < 1.0 || have.length() < 1e-3 {
                break;
            }
            // +yawN turns the barrel right (clockwise from above).
            let err = have.angle_to(want);
            if err.abs() < 1e-4 {
                break;
            }
            yaw = (yaw - err / range).clamp(-1.0, 1.0);
            pn = self.high_angle_pitch_n(p, yaw, point, world_g);
        }
        (yaw, pn)
    }

}

/// `clearance(P, M)`: height of the shape-space point `p` (under rotation
/// `rot` and translation `trans`) above the terrain; 0 when there is none.
pub fn clearance(terrain: Option<&HeightField>, rot: Mat3, trans: Vec3, p: Vec3) -> f32 {
    let w = trans + rot * p;
    match terrain {
        Some(t) => w.z - t.height(w.x, w.y),
        None => 0.0,
    }
}

/// §3.3 `updateMove`: drive, brake, coast, anti-slide, gravity and turn,
/// followed by §3.4 `updateSuspension`. Works on `tank.pos/rot/vel` in place
/// (starting from the pose at the beginning of the tick), under the
/// mission's gravity scale `world_g` ([`crate::World::gravity`]).
pub fn update_move(tank: &mut Tank, mv: &Move, terrain: Option<&HeightField>, world_g: f32) {
    let gravity = GRAVITY * world_g;
    let a = tank.data.clone();
    let start_pos = tank.pos;
    let (r, f, u) = (tank.rot.x_axis, tank.rot.y_axis, tank.rot.z_axis);
    let mut vel = tank.vel;

    if tank.all_contact {
        let vn = vel.dot(u).min(0.0);
        vel -= u * vn;
    } else if !tank.any_contact {
        vel.z += gravity * DT;
    }

    if tank.any_contact {
        tank.turn_in = mv.x;
        tank.throttle = mv.y;
        let thr = tank.throttle;
        let lat = vel.dot(r);
        vel -= r * lat.clamp(-a.anti_slide_rate * DT, a.anti_slide_rate * DT);

        let fs = vel.dot(f);
        let over = fs.abs() > a.max_speed && fs * thr > 0.0;
        if over {
            let ex = if fs > 0.0 { fs - a.max_speed } else { fs + a.max_speed };
            vel -= f * ex.clamp(-a.coast_decel_rate, a.coast_decel_rate) * DT;
        } else if thr.abs() <= 0.001 {
            vel -= f * fs.clamp(-a.coast_decel_rate, a.coast_decel_rate) * DT;
        } else if fs * thr < 0.0 {
            vel += f * (a.decel_rate * thr * DT);
        } else {
            let mut thr = thr;
            let n = (fs + thr * a.accel_rate * DT).abs();
            if n > a.max_speed {
                thr *= (a.max_speed - fs.abs()) / (n - fs.abs());
            }
            vel += f * (thr * a.accel_rate * DT);
        }
    }

    let mut rot = tank.rot;
    if tank.turn_in * tank.turn_in > 0.1 {
        let theta = a.turn_rate * (PI / 180.0) * tank.turn_in * DT;
        rot *= rot_z(theta);
    }

    tank.vel = vel;
    tank.pos = start_pos + vel * DT;
    tank.rot = rot;
    // M keeps the old translation: probes are sampled at the old position.
    update_suspension(tank, start_pos, terrain, world_g);
}

/// §3.4: fits the hull to the four outer probes and applies the lift rules.
/// `m_trans` is the translation of the matrix the probes are sampled with
/// (the position at the start of the tick — the one-tick lag).
pub fn update_suspension(tank: &mut Tank, m_trans: Vec3, terrain: Option<&HeightField>, world_g: f32) {
    let a = tank.data.clone();
    let g = &a.geometry;
    let m_rot = tank.rot;

    let mut q = [Vec3::ZERO; 4];
    for (i, qi) in q.iter_mut().enumerate() {
        let local = g.probe(4 + i);
        let pen = clearance(terrain, m_rot, m_trans, local).min(0.0);
        *qi = m_rot * local;
        qi.z += tank.wheel_vel[i] * DT - pen;
    }
    let mut sum = Vec3::ZERO;
    for k in 1..=4 {
        let base = q[k % 4];
        sum += (q[(k + 2) % 4] - base).cross(q[(k + 1) % 4] - base).normalize_or_zero();
    }
    let n = sum.normalize_or(m_rot.z_axis);
    let right = m_rot.y_axis.cross(n).normalize_or(m_rot.x_axis);
    let fwd = n.cross(right);
    let rot = Mat3::from_cols(right, fwd, n);

    let mut c: [f32; 8] = std::array::from_fn(|j| clearance(terrain, rot, m_trans, g.probe(j)));
    let lim: [f32; 8] = std::array::from_fn(|j| if j < 4 { a.lim_in[j] } else { a.lim_out[j - 4] });
    for j in 0..8 {
        if c[j] < -lim[j] {
            let d = c[j] + lim[j];
            c.iter_mut().for_each(|v| *v -= d);
            tank.pos.z -= d;
        }
    }
    let m = c[4..].iter().copied().fold(-100.0f32, f32::max);
    if m < 0.0 {
        c.iter_mut().for_each(|v| *v -= m);
        tank.pos.z -= m;
    }
    tank.susp_display = std::array::from_fn(|i| c[i].min(a.lim_in[i]));

    let contact: [bool; 4] = std::array::from_fn(|i| c[4 + i] < 0.75 * a.lim_in[i]);
    tank.any_contact = contact.iter().any(|&b| b);
    tank.all_contact = contact.iter().all(|&b| b);
    if tank.all_contact {
        tank.wheel_vel = [0.0; 4];
    } else if tank.any_contact {
        for (w, &touch) in tank.wheel_vel.iter_mut().zip(&contact) {
            *w = if touch { 0.0 } else { *w + GRAVITY * world_g * DT };
        }
        let w = tank.wheel_vel;
        tank.wheel_vel[0] = 0.9 * w[0] + 0.1 * w[3];
        tank.wheel_vel[3] = 0.9 * w[3] + 0.1 * w[0];
        tank.wheel_vel[1] = 0.9 * w[1] + 0.1 * w[2];
        tank.wheel_vel[2] = 0.9 * w[2] + 0.1 * w[1];
    }
    tank.rot = rot;
}

/// Spawn snap: drop to the terrain height, then fit once.
pub fn snap_to_ground(tank: &mut Tank, terrain: Option<&HeightField>, world_g: f32) {
    if let Some(t) = terrain {
        tank.pos.z = t.height(tank.pos.x, tank.pos.y);
    }
    let p = tank.pos;
    update_suspension(tank, p, terrain, world_g);
}

/// §5.1 turret aim with auto-level. `old_rot` is the body rotation at the start of
/// the tick, `tank.rot` the new one.
pub fn update_turret(tank: &mut Tank, mv: &Move, old_rot: Mat3) {
    let a = tank.data.clone();
    let mut pitch = tank.pitch_n;
    if !tank.is_bot() && a.turret_auto_level > 0.01 {
        let deg = PI / 180.0;
        let barrel = rot_z(a.turret_yaw_range * deg * tank.yaw_n) * crate::geom::rot_x(a.turret_pitch_center * deg + a.turret_pitch_range * deg * tank.pitch_n);
        let b_old = (old_rot * barrel).y_axis;
        let new_barrel = tank.rot * barrel;
        let (b_new, r_new) = (new_barrel.y_axis, new_barrel.x_axis);
        let p = b_old - r_new * b_old.dot(r_new);
        let mut alpha = 0.0;
        if p.length_squared() > 0.1 {
            let p = p.normalize();
            let c = b_new.dot(p);
            alpha = if c >= 0.99999 { 0.0 } else { c.clamp(-1.0, 1.0).acos() };
            if p.z < b_new.z {
                alpha = -alpha;
            }
        }
        pitch -= alpha * a.turret_auto_level / (a.turret_pitch_range * deg);
    }
    tank.yaw_n = (tank.yaw_n + mv.yaw / PI).clamp(-1.0, 1.0);
    tank.pitch_n = (pitch + mv.pitch / PI).clamp(-1.0, 1.0);
}

/// When the turret is pinned at a yaw limit and no full turn key is held, turn
/// the hull towards it.
pub fn apply_auto_turn(mv: &mut Move, tank: &Tank) {
    if tank.yaw_n.abs() >= 0.9999 && mv.x as i32 == 0 {
        mv.x = tank.yaw_n.signum();
    }
}

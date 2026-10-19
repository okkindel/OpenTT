//! Absent fields take the values the engine's def constructors set.

use std::sync::Arc;

use glam::{Affine3A, Vec3};
use tt_assets::dts::Shape;

use crate::fields::{Defs, Fields};
use crate::geom::{affine_from_rows, rot_x, rot_z, Aabb};

/// Most projectiles one shot may fire (`pellet_count`).
pub const MAX_PELLETS: usize = 9;

/// `TankMiscDef` (shared physics tuning).
#[derive(Debug, Clone, PartialEq)]
pub struct TankMiscParams {
    pub bounce_static: f32,
    pub bounce_tank: f32,
    pub boost_ticks: i32,
    /// Milliseconds without contact and with |v|² < 0.1 before the unstuck kick.
    pub unstuck_time: i32,
    pub unstuck_boost: f32,
    /// Damage repaired per second.
    pub repair_rate: f32,
}

impl Default for TankMiscParams {
    fn default() -> Self {
        // Engine constructor values; the shipped DefaultTankMisc overrides them
        // with 0.4 / 0.75 / 5 / 1000 / 5 / 0.
        TankMiscParams { bounce_static: 0.5, bounce_tank: 0.5, boost_ticks: 5, unstuck_time: 1000, unstuck_boost: 5.0, repair_rate: 0.5 }
    }
}

impl TankMiscParams {
    pub fn from_fields(f: &Fields) -> TankMiscParams {
        let d = TankMiscParams::default();
        TankMiscParams {
            bounce_static: f.f32_or("bounce_static", d.bounce_static),
            bounce_tank: f.f32_or("bounce_tank", d.bounce_tank),
            boost_ticks: f.i32_or("boost_ticks", d.boost_ticks),
            unstuck_time: f.i32_or("unstuck_time", d.unstuck_time),
            unstuck_boost: f.f32_or("unstuck_boost", d.unstuck_boost),
            repair_rate: f.f32_or("repair_rate", d.repair_rate),
        }
    }
}

/// The physics-relevant part of `TankFxDef`.
#[derive(Debug, Clone, PartialEq)]
pub struct TankFxParams {
    pub corpse_smoke_time: f32,
    /// Seconds after death when collisions stop.
    pub corpse_fade_time: f32,
    /// Extra seconds after `corpse_fade_time` before the corpse is deleted.
    pub corpse_fade_speed: f32,
    pub damage_emitter_start: f32,
    /// Relative speed above which a tank collision knocks the scrum ball loose.
    pub hard_tank_coll_vel: f32,
    /// Seconds after death before the death sequence (and the brain ejection) starts.
    pub corpse_shake_time: f32,
    /// Client-side cosmetic projectile carrying the brain jar out of a corpse.
    pub brain_projectile: Option<Arc<ProjectileParams>>,
}

impl Default for TankFxParams {
    fn default() -> Self {
        TankFxParams { corpse_smoke_time: 5.0, corpse_fade_time: 30.0, corpse_fade_speed: 1.0, damage_emitter_start: 0.0, hard_tank_coll_vel: 100.0, corpse_shake_time: 3.0, brain_projectile: None }
    }
}

impl TankFxParams {
    /// Like [`TankFxParams::from_fields`], resolving `brain_projectile` in `db`.
    pub fn from_fields_db(f: &Fields, db: Option<&Defs>) -> TankFxParams {
        let mut fx = TankFxParams::from_fields(f);
        fx.brain_projectile = f.reference("brain_projectile").and_then(|n| db?.fields(n)).map(|p| Arc::new(ProjectileParams::from_fields(&p)));
        fx
    }

    pub fn from_fields(f: &Fields) -> TankFxParams {
        let d = TankFxParams::default();
        TankFxParams {
            corpse_smoke_time: f.f32_or("corpse_smoke_time", d.corpse_smoke_time),
            corpse_fade_time: f.f32_or("corpse_fade_time", d.corpse_fade_time),
            corpse_fade_speed: f.f32_or("corpse_fade_speed", d.corpse_fade_speed),
            damage_emitter_start: f.f32_or("damage_emitter_start", d.damage_emitter_start),
            hard_tank_coll_vel: f.f32_or("hard_tank_collision_speed", d.hard_tank_coll_vel),
            corpse_shake_time: f.f32_or("corpse_shake_time", d.corpse_shake_time),
            brain_projectile: None,
        }
    }
}

/// `ProjectileDef`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileParams {
    pub name: String,
    pub velocity: f32,
    pub inherit_velocity: f32,
    pub bounces: i32,
    pub bounce_factor: f32,
    pub blast_radius: f32,
    pub rounds: i32,
    pub keep_previous_weapon: bool,
    pub can_be_kept: bool,
    /// Milliseconds between shots of a burst.
    pub reload_ms: i32,
    /// Milliseconds after the last shot before a new burst may start.
    pub volley_delay_ms: i32,
    pub volley_size: i32,
    pub sync_ms: i32,
    pub sync_lead_ms: i32,
    pub damage: f32,
    pub gravity_scale: f32,
    pub sprite_scale: f32,
    pub max_rotation: f32,
    pub resource: String,
    pub tank_explosion: String,
    pub own_tank_explosion: String,
    pub bounce_explosion: String,
    pub other_explosion: String,
    /// `ReticleDef` drawn while this cartridge is loaded.
    pub reticle: String,
    /// `EmitterDef` trailing the projectile in flight (client-only
    /// visual; the ejected brain jar's `RocketEmitter` flame and smoke).
    pub emitter: String,
    /// Ray z-offset `r`: 0.5·sprite_scale for sprites; for a `.dts` resource the
    /// engine uses half the shape's height ([`ProjectileParams::set_shape`]).
    pub radius: f32,
    /// Projectiles per shot (scatter shot), fanned out about the
    /// barrel's up axis over `spread_angle` degrees in all, or in a cone
    /// that wide (`spread_ring`).
    pub pellet_count: i32,
    pub spread_angle: f32,
    /// Pellet spacing over the fan: 1 = even, above 1 packs them towards
    /// the middle (pellet `i` at `sign(x)·|x|^e` of the half-fan, `x`
    /// evenly −1..1), so more of them hit at mid range.
    pub spread_exponent: f32,
    /// Pellet pattern: false = a horizontal fan, true = a cone (one pellet
    /// along the barrel, the others on a ring `spread_angle` wide, see
    /// [`ProjectileParams::launch_dirs`]).
    pub spread_ring: bool,
    /// Per-shot jitter of the ring (0..1): the ring turns by up to
    /// ±`spread_jitter`·half a slot and each ring pellet's angle varies by
    /// up to ±`spread_jitter`·20 %, from the shot number (deterministic).
    pub spread_jitter: f32,
    /// A cone's every other ring pellet at this fraction of the ring (1 = one
    /// ring; below 1 an inner and an outer triangle for a 1 + 6 cone, so
    /// fewer pellets hit as the range grows instead of all at once).
    pub spread_inner: f32,
    /// Tracer streak drawn behind a sprite projectile, metres (0 = none).
    pub trail_length: f32,
    /// High-angle launch (mortar) when `launch_pitch_max >
    /// launch_pitch_min`: see [`ProjectileParams::launch_dirs`].
    pub launch_pitch_min: f32,
    pub launch_pitch_max: f32,
    /// A high-angle cartridge's reach on the flat (m, from the muzzle
    /// back down to its height) with the barrel lowest / highest, linear
    /// in between; the launch speed follows from it
    /// ([`ProjectileParams::launch_speed`]). 0 = launch at `velocity`.
    pub launch_range_min: f32,
    pub launch_range_max: f32,
    /// Flight time limit in ms (0 = none).
    pub max_life_ms: i32,
    /// SoundProfile played when fired, instead of the tank's `fire_sound`.
    pub fire_sound: String,
    /// A `.dts` drawn instead of the sprite, nose (+Y) along the flight
    /// (looks only: collision still uses `sprite_scale`).
    pub display_shape: String,
    /// Additive glow sprite at the tail (an exhaust flame), its size (m)
    /// and the tail's distance behind the centre (m; the `emitter` trail
    /// starts there too).
    pub exhaust_glow: String,
    pub exhaust_glow_size: f32,
    pub exhaust_offset: f32,
}

/// A deterministic number in 0..1 from a shot number and a channel (an
/// integer hash: the same on the server and every client).
pub fn shot_noise(shot: u32, k: u32) -> f32 {
    let mut x = shot.wrapping_mul(0x9e37_79b9) ^ k.wrapping_mul(0x85eb_ca6b).rotate_left(13);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

impl ProjectileParams {
    pub fn from_fields(f: &Fields) -> ProjectileParams {
        let sprite_scale = f.f32_or("sprite_scale", 1.0);
        ProjectileParams {
            name: f.name.clone(),
            velocity: f.f32_or("velocity", 30.0),
            inherit_velocity: f.f32_or("inherit_velocity", 1.0),
            bounces: f.i32_or("bounces", 0),
            bounce_factor: f.f32_or("bounce_factor", 1.0),
            blast_radius: f.f32_or("blast_radius", 0.0),
            rounds: f.i32_or("rounds", 0),
            keep_previous_weapon: f.bool_or("keep_previous_weapon", false),
            can_be_kept: f.bool_or("can_be_kept", true),
            reload_ms: f.i32_or("reload_ms", 1000),
            volley_delay_ms: f.i32_or("volley_delay_ms", 0),
            volley_size: f.i32_or("volley_size", 0),
            sync_ms: f.i32_or("sync_ms", 1000),
            sync_lead_ms: f.i32_or("sync_lead_ms", 500),
            damage: f.f32_or("damage", 1.0),
            gravity_scale: f.f32_or("gravity_scale", 1.0),
            sprite_scale,
            max_rotation: f.f32_or("max_rotation", 0.0),
            resource: f.str_or("resource", "").to_string(),
            tank_explosion: f.str_or("tank_explosion", "").to_string(),
            own_tank_explosion: f.str_or("own_tank_explosion", "").to_string(),
            bounce_explosion: f.str_or("bounce_explosion", "").to_string(),
            other_explosion: f.str_or("other_explosion", "").to_string(),
            reticle: f.str_or("reticle", "").to_string(),
            emitter: f.str_or("emitter", "").to_string(),
            radius: 0.5 * sprite_scale,
            pellet_count: f.i32_or("pellet_count", 1).clamp(1, MAX_PELLETS as i32),
            spread_angle: f.f32_or("spread_angle", 0.0),
            spread_exponent: f.f32_or("spread_exponent", 1.0),
            spread_ring: f.bool_or("spread_ring", false),
            spread_jitter: f.f32_or("spread_jitter", 0.0),
            spread_inner: f.f32_or("spread_inner", 1.0),
            trail_length: f.f32_or("trail_length", 0.0),
            launch_pitch_min: f.f32_or("launch_pitch_min", 0.0),
            launch_pitch_max: f.f32_or("launch_pitch_max", 0.0),
            launch_range_min: f.f32_or("launch_range_min", 0.0),
            launch_range_max: f.f32_or("launch_range_max", 0.0),
            max_life_ms: f.i32_or("max_life_ms", 0),
            fire_sound: f.str_or("fire_sound", "").to_string(),
            display_shape: f.str_or("display_shape", "").to_string(),
            exhaust_glow: f.str_or("exhaust_glow", "").to_string(),
            exhaust_glow_size: f.f32_or("exhaust_glow_size", 0.0),
            exhaust_offset: f.f32_or("exhaust_offset", 0.0),
        }
    }

    /// Launches at a fixed high angle (the mortar).
    pub fn high_angle(&self) -> bool {
        self.launch_pitch_max > self.launch_pitch_min
    }

    /// A carpet: a high-angle cartridge firing several shells a shot
    /// (the reticle shows one footprint for the pattern).
    pub fn is_carpet(&self) -> bool {
        self.high_angle() && self.pellet_count > 1
    }

    /// The mortar's elevation above the horizon (degrees) for the barrel's
    /// normalised pitch (`pitch_n` −1 = barrel highest): raising the barrel
    /// lowers the arc towards `launch_pitch_min` and so reaches further, as
    /// with every other cartridge.
    pub fn launch_elevation_deg(&self, pitch_n: f32) -> f32 {
        let u = (1.0 - pitch_n.clamp(-1.0, 1.0)) * 0.5;
        self.launch_pitch_max - (self.launch_pitch_max - self.launch_pitch_min) * u
    }

    /// Muzzle speed of a shot with the turret at `pitch_n`: `velocity`. A
    /// high-angle cartridge with `launch_range_min/Max` launches as fast as
    /// needed to come back to the muzzle's height at the reach interpolated
    /// between them (barrel lowest → `launch_range_min`, highest →
    /// `launch_range_max`), so the barrel sets the reach linearly and the
    /// flight stays short (a mortar's charge).
    pub fn launch_speed(&self, pitch_n: f32) -> f32 {
        if !self.high_angle() || self.launch_range_max <= 0.0 {
            return self.velocity;
        }
        let u = (1.0 - pitch_n.clamp(-1.0, 1.0)) * 0.5;
        let reach = self.launch_range_min + (self.launch_range_max - self.launch_range_min) * u;
        let g = -crate::GRAVITY * self.gravity_scale;
        let s = (2.0 * self.launch_elevation_deg(pitch_n).to_radians()).sin().max(1e-3);
        (reach.max(0.0) * g / s).sqrt()
    }

    /// Where a high-angle shell launched at `pitch_n` comes down through the
    /// height `dz` relative to its spawn point: (horizontal distance, flight
    /// time s), following the simulation's tick integration exactly
    /// (explicit Euler at `DT`: the shell is `g·DT·t/2` above the analytic
    /// parabola at tick time `t`). None when the arc never gets that high.
    pub fn high_angle_reach(&self, pitch_n: f32, dz: f32) -> Option<(f32, f32)> {
        let e = self.launch_elevation_deg(pitch_n).to_radians();
        let v = self.launch_speed(pitch_n);
        let g = -crate::GRAVITY * self.gravity_scale;
        let (vh, vz) = (v * e.cos(), v * e.sin() + 0.5 * g * crate::DT);
        if g <= 0.0 {
            return None;
        }
        let disc = vz * vz - 2.0 * g * dz;
        if disc < 0.0 {
            return None;
        }
        let t = (vz + disc.sqrt()) / g;
        Some((vh * t, t))
    }

    /// The turret pitch (`pitch_n`) whose high-angle shell comes down at
    /// horizontal distance `d` and height `dz` relative to its spawn point
    /// ([`Self::high_angle_reach`], bisection: raising the barrel reaches
    /// further); the nearest limit when out of reach.
    pub fn pitch_n_for_reach(&self, d: f32, dz: f32) -> f32 {
        // Distance past the target (+) or short of it (−).
        let past = |pn: f32| self.high_angle_reach(pn, dz).map_or(-1e9, |(r, _)| r - d);
        if past(-1.0) <= 0.0 {
            return -1.0;
        }
        if past(1.0) >= 0.0 {
            return 1.0;
        }
        let (mut far, mut near) = (-1.0f32, 1.0f32);
        for _ in 0..24 {
            let m = 0.5 * (far + near);
            if past(m) > 0.0 {
                far = m;
            } else {
                near = m;
            }
        }
        0.5 * (far + near)
    }

    /// Unit launch direction of each projectile of one shot from the muzzle
    /// transform `muzzle` (Y = barrel forward, Z = barrel up) with the
    /// turret at `pitch_n`; `shot` is the shooter's shot number
    /// (`Tank::fire_count` after the shot; it seeds a cone's jitter). A
    /// high-angle cartridge launches along the barrel's heading at
    /// [`Self::launch_elevation_deg`] above the horizon; a scatter shot fans
    /// `pellet_count` pellets over `spread_angle` degrees about the barrel's
    /// up axis, `spread_exponent` packing them towards the middle, or with
    /// `spread_ring` in a cone: the first pellet along the barrel, the
    /// others evenly on a ring `spread_angle` wide around it (the first
    /// level with the barrel, right of it; `spread_inner` pulls every other
    /// one in), turned and spread a little per shot by `spread_jitter`
    /// (deterministic from `shot`: the server, the shooter's prediction and
    /// the reticle agree).
    pub fn launch_dirs(&self, muzzle: &Affine3A, pitch_n: f32, shot: u32) -> Vec<Vec3> {
        let fwd = Vec3::from(muzzle.matrix3.y_axis);
        let mut base = fwd;
        if self.high_angle() {
            let h = Vec3::new(fwd.x, fwd.y, 0.0).try_normalize().unwrap_or(Vec3::Y);
            let e = self.launch_elevation_deg(pitch_n).to_radians();
            base = h * e.cos() + Vec3::Z * e.sin();
        }
        let n = self.pellet_count.max(1) as usize;
        if n == 1 {
            return vec![base];
        }
        let up = Vec3::from(muzzle.matrix3.z_axis).try_normalize().unwrap_or(Vec3::Z);
        let half = 0.5 * self.spread_angle.to_radians();
        if self.spread_ring {
            // The ring's axes: right and up square to the launch direction.
            let right = base.cross(up).try_normalize().unwrap_or(Vec3::X);
            let up = right.cross(base).normalize();
            let ring = n - 1;
            let slot = std::f32::consts::TAU / ring as f32;
            let j = self.spread_jitter.clamp(0.0, 1.0);
            let turn = (shot_noise(shot, 0) - 0.5) * j * slot;
            let mut dirs = vec![base];
            dirs.extend((0..ring).map(|i| {
                let phi = turn + slot * i as f32;
                let inner = if i % 2 == 1 { self.spread_inner } else { 1.0 };
                let r = half * inner * (1.0 + 0.4 * j * (shot_noise(shot, 1 + i as u32) - 0.5));
                let off = right * phi.cos() + up * phi.sin();
                (base * r.cos() + off * r.sin()).normalize()
            }));
            return dirs;
        }
        let e = self.spread_exponent.max(0.1);
        (0..n)
            .map(|i| {
                let x = 2.0 * i as f32 / (n - 1) as f32 - 1.0;
                let a = half * x.signum() * x.abs().powf(e);
                glam::Quat::from_axis_angle(up, a) * base
            })
            .collect()
    }

    /// For `.dts` resources: the ray offset is half the shape's height.
    pub fn set_shape(&mut self, shape: &Shape) {
        let b = Aabb::from_bounds(&shape.bounds);
        if !b.is_empty() {
            self.radius = 0.5 * (b.max.z - b.min.z);
        }
    }

    pub fn is_dts(&self) -> bool {
        self.resource.to_ascii_lowercase().ends_with(".dts")
    }
}

/// A node of a shape in its default pose.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDef {
    pub name: String,
    pub parent: Option<usize>,
    /// Transform relative to the parent.
    pub local: Affine3A,
}

/// Tank shape data that the simulation needs (from the `.dts`).
#[derive(Debug, Clone, PartialEq)]
pub struct TankGeometry {
    /// Shape bounds (shape space).
    pub bounds: Aabb,
    /// Default-pose positions of `inner_RF, inner_RR, inner_LR, inner_LF`.
    pub inner: [Vec3; 4],
    /// Default-pose positions of `outer_RF, outer_RR, outer_LR, outer_LF`.
    pub outer: [Vec3; 4],
    pub nodes: Vec<NodeDef>,
    pub turret_node: Option<usize>,
    pub weapon_node: Option<usize>,
    pub muzzle_node: Option<usize>,
    /// `codeSeat` (brain jar seat, child of the turret).
    pub seat_node: Option<usize>,
    /// `codeScrum` (where a carried scrum ball sits).
    pub scrum_node: Option<usize>,
}

const PROBE_NODES: [&str; 8] = ["inner_RF", "inner_RR", "inner_LR", "inner_LF", "outer_RF", "outer_RR", "outer_LR", "outer_LF"];

impl Default for TankGeometry {
    /// A generic medium-sized tank (2.6 m cube bounds) for tests and for
    /// shapes that are not loaded.
    fn default() -> Self {
        let n = |name: &str, parent: Option<usize>, t: Vec3| NodeDef { name: name.into(), parent, local: Affine3A::from_translation(t) };
        let nodes = vec![
            n("codeTurret", None, Vec3::new(0.0, 0.2, 1.47)),
            n("codeWeapon", Some(0), Vec3::new(0.0, 0.4, 0.28)),
            n("codeMuzzle", Some(1), Vec3::new(0.0, 0.5, 0.0)),
            n("codeSeat", Some(0), Vec3::new(-0.2, 0.0, 0.58)),
            n("codeScrum", None, Vec3::new(0.0, 0.0, 3.0)),
        ];
        TankGeometry {
            bounds: Aabb::new(Vec3::new(-1.3, -1.3, 0.0), Vec3::new(1.3, 1.3, 2.6)),
            inner: [Vec3::new(0.6, 0.1, 0.25), Vec3::new(0.6, -0.3, 0.25), Vec3::new(-0.6, -0.3, 0.25), Vec3::new(-0.6, 0.1, 0.25)],
            outer: [Vec3::new(0.6, 0.7, 0.65), Vec3::new(0.6, -0.8, 0.5), Vec3::new(-0.6, -0.8, 0.5), Vec3::new(-0.6, 0.7, 0.65)],
            nodes,
            turret_node: Some(0),
            weapon_node: Some(1),
            muzzle_node: Some(2),
            seat_node: Some(3),
            scrum_node: Some(4),
        }
    }
}

impl TankGeometry {
    /// Extracts bounds, probe nodes and turret nodes. Missing probe nodes
    /// fall back to the default geometry's ("TankParams: Incomplete data").
    pub fn from_shape(shape: &Shape) -> TankGeometry {
        let fallback = TankGeometry::default();
        let world = shape.node_world_transforms();
        let find = |name: &str| shape.nodes.iter().position(|n| n.name.eq_ignore_ascii_case(name));
        let pos = |name: &str| find(name).map(|i| Vec3::new(world[i][0][3], world[i][1][3], world[i][2][3]));
        let mut inner = fallback.inner;
        let mut outer = fallback.outer;
        for (k, name) in PROBE_NODES.iter().enumerate() {
            if let Some(p) = pos(name) {
                if k < 4 {
                    inner[k] = p;
                } else {
                    outer[k - 4] = p;
                }
            }
        }
        let bounds = Aabb::from_bounds(&shape.bounds);
        TankGeometry {
            bounds: if bounds.is_empty() { fallback.bounds } else { bounds },
            inner,
            outer,
            nodes: shape
                .nodes
                .iter()
                .map(|n| NodeDef { name: n.name.clone(), parent: n.parent, local: affine_from_rows(&n.local_transform()) })
                .collect(),
            turret_node: find("codeTurret"),
            weapon_node: find("codeWeapon"),
            muzzle_node: find("codeMuzzle"),
            seat_node: find("codeSeat"),
            scrum_node: find("codeScrum"),
        }
    }

    /// `R = 0.5·|bounds diagonal| + 0.1`.
    pub fn big_radius(&self) -> f32 {
        0.5 * (self.bounds.max - self.bounds.min).length() + 0.1
    }

    /// `r = 0.5·|bounds diagonal in XY|`.
    pub fn small_radius(&self) -> f32 {
        0.5 * (self.bounds.max - self.bounds.min).truncate().length()
    }

    /// Probe point `j` (0..4 inner, 4..8 outer) at the bounds' bottom.
    pub fn probe(&self, j: usize) -> Vec3 {
        let p = if j < 4 { self.inner[j] } else { self.outer[j - 4] };
        Vec3::new(p.x, p.y, self.bounds.min.z)
    }

    /// Shape-space transform of node `idx` with the turret posed
    /// (yawN/pitchN normalised, angles from `data`).
    pub fn posed_node(&self, idx: usize, yaw_n: f32, pitch_n: f32, data: &TankParams) -> Affine3A {
        let mut m = Affine3A::IDENTITY;
        let mut chain = Vec::new();
        let mut cur = Some(idx);
        while let Some(i) = cur {
            if chain.len() > self.nodes.len() {
                break;
            }
            chain.push(i);
            cur = self.nodes[i].parent.filter(|&p| p < self.nodes.len());
        }
        for &i in chain.iter().rev() {
            let mut local = self.nodes[i].local;
            if Some(i) == self.turret_node {
                local *= Affine3A::from_mat3(rot_z((data.turret_yaw_range * yaw_n).to_radians()));
            }
            if Some(i) == self.weapon_node {
                local *= Affine3A::from_mat3(rot_x((data.turret_pitch_range * pitch_n - data.turret_pitch_center).to_radians()));
            }
            m *= local;
        }
        m
    }

    /// Shape-space muzzle transform (Y axis = barrel forward).
    pub fn muzzle(&self, yaw_n: f32, pitch_n: f32, data: &TankParams) -> Affine3A {
        match self.muzzle_node.or(self.weapon_node) {
            Some(i) => self.posed_node(i, yaw_n, pitch_n, data),
            None => {
                let rot = rot_z((data.turret_yaw_range * yaw_n).to_radians())
                    * rot_x((data.turret_pitch_range * pitch_n - data.turret_pitch_center).to_radians());
                let c = self.bounds.center();
                Affine3A::from_mat3_translation(rot, Vec3::new(c.x, self.bounds.max.y, self.bounds.max.z))
            }
        }
    }

    /// Shape-space barrel orientation (weapon node, or the bare turret angles).
    pub fn barrel_rotation(&self, yaw_n: f32, pitch_n: f32, data: &TankParams) -> glam::Mat3 {
        glam::Mat3::from(self.muzzle(yaw_n, pitch_n, data).matrix3)
    }
}

/// `TankDef`, with its misc/fx defs and default projectile resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct TankParams {
    pub name: String,
    pub shape_file: String,
    /// Degrees per second.
    pub turn_rate: f32,
    pub accel_rate: f32,
    pub decel_rate: f32,
    pub coast_decel_rate: f32,
    pub anti_slide_rate: f32,
    pub max_speed: f32,
    /// Degrees.
    pub turret_pitch_center: f32,
    pub turret_yaw_range: f32,
    pub turret_pitch_range: f32,
    pub turret_auto_level: f32,
    pub suspension_range: f32,
    pub spring_range: Vec3,
    pub spring_vel_scale: f32,
    pub spring_correct_scale1: f32,
    pub spring_correct_scale2: f32,
    pub damage_amount: f32,
    pub damage_scale: f32,
    pub armor_scale: f32,
    pub fire_scale: f32,
    pub default_projectile: Option<Arc<ProjectileParams>>,
    /// Brain jar shape ejected from the corpse (cosmetic).
    pub brain_shape: String,
    /// Lengths (s) of the shape's `death1`..`death4` sequences, up to the first
    /// missing one. Filled by [`TankParams::set_shape`].
    pub death_sequences: Vec<f32>,
    pub misc: TankMiscParams,
    pub fx: TankFxParams,
    pub geometry: TankGeometry,
    /// Derived in `preload`: `suspension_range · h_in[i]`.
    pub lim_in: [f32; 4],
    /// Derived in `preload`: `outer_i.z − minZ − mean(h_in)`.
    pub lim_out: [f32; 4],
}

impl TankParams {
    /// Parses a TankParams field map. `misc`, `fx` and `default_projectile`
    /// are looked up in `db` (defaults when missing).
    pub fn from_fields(f: &Fields, db: Option<&Defs>) -> TankParams {
        let lookup = |field: &str| f.reference(field).and_then(|n| db.and_then(|db| db.fields(n)));
        let mut t = TankParams {
            name: f.name.clone(),
            shape_file: f.str_or("shape", "").to_string(),
            turn_rate: f.f32_or("turn_rate", 30.0),
            accel_rate: f.f32_or("accel_rate", 10.0),
            decel_rate: f.f32_or("decel_rate", 20.0),
            coast_decel_rate: f.f32_or("coast_decel_rate", 10.0),
            anti_slide_rate: f.f32_or("anti_slide_rate", 30.0),
            max_speed: f.f32_or("max_speed", 15.0),
            turret_pitch_center: f.f32_or("turret_pitch_center", 30.0),
            turret_yaw_range: f.f32_or("turret_yaw_range", 30.0),
            turret_pitch_range: f.f32_or("turret_pitch_range", 30.0),
            turret_auto_level: f.f32_or("turret_auto_level", 0.0),
            suspension_range: f.f32_or("suspension_range", 0.1),
            spring_range: Vec3::new(f.f32_or("spring_range_x", 0.06), f.f32_or("spring_range_y", 0.06), f.f32_or("spring_range_z", 0.0)),
            spring_vel_scale: f.f32_or("spring_vel_scale", 1.0),
            spring_correct_scale1: f.f32_or("spring_correct_scale1", 2.0),
            spring_correct_scale2: f.f32_or("spring_correct_scale2", 0.999),
            damage_amount: f.f32_or("damage_amount", 10.0),
            damage_scale: f.f32_or("damage_scale", 1.0),
            armor_scale: f.f32_or("armor_scale", 1.0),
            fire_scale: f.f32_or("fire_scale", 1.0),
            default_projectile: lookup("default_projectile").map(|p| Arc::new(ProjectileParams::from_fields(&p))),
            misc: lookup("misc").map_or_else(TankMiscParams::default, |m| TankMiscParams::from_fields(&m)),
            fx: lookup("fx").map_or_else(TankFxParams::default, |m| TankFxParams::from_fields_db(&m, db)),
            brain_shape: f.str_or("brain_shape", "").to_string(),
            death_sequences: Vec::new(),
            geometry: TankGeometry::default(),
            lim_in: [0.0; 4],
            lim_out: [0.0; 4],
        };
        t.update_limits();
        t
    }

    /// Replaces the geometry with the tank's shape and recomputes the limits.
    pub fn set_shape(&mut self, shape: &Shape) {
        self.geometry = TankGeometry::from_shape(shape);
        self.death_sequences.clear();
        for i in 1..=4 {
            let name = format!("death{i}");
            match shape.sequences.iter().find(|s| s.name.eq_ignore_ascii_case(&name)) {
                Some(s) => self.death_sequences.push(s.duration),
                None => break,
            }
        }
        self.update_limits();
    }

    fn update_limits(&mut self) {
        let g = &self.geometry;
        let min_z = g.bounds.min.z;
        let h_in = g.inner.map(|p| p.z - min_z);
        let avg = h_in.iter().sum::<f32>() / 4.0;
        self.lim_in = h_in.map(|h| self.suspension_range * h);
        self.lim_out = g.outer.map(|p| p.z - min_z - avg);
    }
}

/// `PowerUpDef` (pick-ups; also the base of boosts and the volcano).
#[derive(Debug, Clone, PartialEq)]
pub struct PowerUpParams {
    pub name: String,
    pub kind: String,
    pub shape: String,
    /// Milliseconds.
    pub min_off_ms: i32,
    pub max_off_ms: i32,
    pub starts_on: bool,
    /// Bots drive to it when it is on (steering attraction).
    pub ai_pickup: bool,
    /// The sequences [`crate::World::play_user_thread`]`(slot, ..)` plays
    /// ("" if none): slot [`PowerUpParams::ACTIVATE_SLOT`] is the def's
    /// `activate_sequence`.
    pub user_seq: [String; PowerUpParams::USER_THREADS],
}

impl PowerUpParams {
    pub fn from_fields(f: &Fields) -> PowerUpParams {
        PowerUpParams {
            name: f.name.clone(),
            kind: f.str_or("kind", "").to_string(),
            shape: f.str_or("shape", "").to_string(),
            min_off_ms: f.i32_or("min_off_ms", 0),
            max_off_ms: f.i32_or("max_off_ms", 0),
            starts_on: f.bool_or("starts_on", true),
            ai_pickup: f.bool_or("ai_pickup", false),
            user_seq: std::array::from_fn(|i| if i == Self::ACTIVATE_SLOT { f.str_or("activate_sequence", "").to_string() } else { String::new() }),
        }
    }

    /// Number of user thread slots.
    pub const USER_THREADS: usize = 4;
    /// The slot of the `activate_sequence` (the reload arch's lightning).
    pub const ACTIVATE_SLOT: usize = 1;

    /// Tanks drive through these; see
    /// [`crate::World::static_blocks_tanks`].
    pub fn is_pickup(&self) -> bool {
        self.kind.eq_ignore_ascii_case("weapon") || self.kind.eq_ignore_ascii_case("health")
    }
}

/// How a pad combines its kick with the tank's velocity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoostMode {
    Additive,
    /// Replace the whole velocity.
    Override,
    /// Replace only the component along the pad direction.
    OverrideDir,
}

pub const MAX_STATES: usize = 5;

/// `BoostDef` (boost and jump pads).
#[derive(Debug, Clone, PartialEq)]
pub struct BoostParams {
    pub base: PowerUpParams,
    pub mode: BoostMode,
    pub advance_delay_on_enter: i32,
    pub num_states: i32,
    pub state_name: [String; MAX_STATES],
    pub velocity: [f32; MAX_STATES],
    pub min_duration: [i32; MAX_STATES],
    pub max_duration: [i32; MAX_STATES],
}

impl BoostParams {
    /// Array fields are registered as `velocity%i` etc. with suffixes 1..=5
    /// for slots 0..5.
    pub fn from_fields(f: &Fields) -> BoostParams {
        let base = PowerUpParams::from_fields(f);
        let mode = match base.kind.to_ascii_lowercase().as_str() {
            "override" => BoostMode::Override,
            "overridedir" => BoostMode::OverrideDir,
            _ => BoostMode::Additive,
        };
        BoostParams {
            mode,
            advance_delay_on_enter: f.i32_or("advance_delay_on_enter", 0),
            num_states: f.i32_or("num_states", 0),
            state_name: std::array::from_fn(|i| f.str_or(&format!("state_name{}", i + 1), "").to_string()),
            velocity: std::array::from_fn(|i| f.f32_or(&format!("velocity{}", i + 1), 0.0)),
            min_duration: std::array::from_fn(|i| f.i32_or(&format!("min_duration{}", i + 1), 0)),
            max_duration: std::array::from_fn(|i| f.i32_or(&format!("max_duration{}", i + 1), 0)),
            base,
        }
    }

    pub fn states(&self) -> usize {
        (self.num_states.max(1) as usize).min(MAX_STATES)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VolcanoZone {
    /// Name of the marker object whose box is the target area.
    pub name: String,
    pub weight: i32,
    pub focus_weight: i32,
    pub radius: f32,
}

/// `VolcanoDef`.
#[derive(Debug, Clone, PartialEq)]
pub struct VolcanoParams {
    pub base: PowerUpParams,
    pub num_zones: i32,
    pub min_attack_delay: i32,
    pub max_attack_delay: i32,
    pub min_attack_rocks: i32,
    pub max_attack_rocks: i32,
    pub zones: [VolcanoZone; MAX_STATES],
    pub projectile: Option<Arc<ProjectileParams>>,
}

impl VolcanoParams {
    pub fn from_fields(f: &Fields, db: Option<&Defs>) -> VolcanoParams {
        VolcanoParams {
            base: PowerUpParams::from_fields(f),
            num_zones: f.i32_or("num_zones", 0),
            min_attack_delay: f.i32_or("min_attack_delay", 0),
            max_attack_delay: f.i32_or("max_attack_delay", 0),
            min_attack_rocks: f.i32_or("min_attack_rocks", 0),
            max_attack_rocks: f.i32_or("max_attack_rocks", 0),
            zones: std::array::from_fn(|i| VolcanoZone {
                name: f.str_or(&format!("zone_name{}", i + 1), "").to_string(),
                weight: f.i32_or(&format!("zone_weight{}", i + 1), 0),
                focus_weight: f.i32_or(&format!("zone_focus_weight{}", i + 1), 0),
                radius: f.f32_or(&format!("zone_radius{}", i + 1), 10.0),
            }),
            projectile: f
                .reference("projectile")
                .and_then(|n| db.and_then(|db| db.fields(n)))
                .map(|p| Arc::new(ProjectileParams::from_fields(&p))),
        }
    }

    pub fn zone_count(&self) -> usize {
        (self.num_zones.max(0) as usize).min(MAX_STATES)
    }
}

/// Shape data of a trigger object (power-up, pad, volcano).
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerShape {
    /// Shape bounds (the object box; AI queries, volcano top centre).
    pub bounds: Aabb,
    /// Shape-space box the tank origin is tested against for `onEnter`: the
    /// shape bounds with the x and y extents halved about the shape origin (z
    /// unchanged), or the bounds of the mesh on a `hitbox` node (no shipped
    /// shape has one). For the Reload arch this is the opening between the
    /// legs.
    pub trigger_box: Aabb,
    /// Shape-space transform of the pad's `direction` node (default pose).
    pub direction: Option<Affine3A>,
    /// The `direction` node's shape-space transform in each sequence that
    /// moves it. boostpad.dts turns it a quarter per state: moe 0°, larry
    /// 90°, curly 180°, schemp 270°.
    pub direction_by_sequence: Vec<(String, Affine3A)>,
    /// Shape-space position of the volcano's `muzzleNode` node.
    pub muzzle: Option<Vec3>,
    /// Collision meshes: PowerUp, Boost and Volcano objects are also static
    /// collidables, but `buildPolyList` and `castRay` return nothing while
    /// the object is off, so only an "on" object is solid.
    pub collision: Option<Arc<crate::statics::StaticShape>>,
}

impl Default for TriggerShape {
    fn default() -> Self {
        let bounds = Aabb::new(Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 2.0));
        TriggerShape { bounds, trigger_box: bounds, direction: None, direction_by_sequence: Vec::new(), muzzle: None, collision: None }
    }
}

impl TriggerShape {
    /// `preload`: box = bounds, then min.x, min.y, max.x, max.y are
    /// multiplied by 0.5.
    pub fn trigger_box_of(bounds: Aabb) -> Aabb {
        let h = Vec3::new(0.5, 0.5, 1.0);
        Aabb::new(bounds.min * h, bounds.max * h)
    }

    pub fn from_shape(shape: &Shape) -> TriggerShape {
        let world = shape.node_world_transforms();
        let find = |name: &str| shape.nodes.iter().position(|n| n.name.eq_ignore_ascii_case(name));
        let bounds = Aabb::from_bounds(&shape.bounds);
        let bounds = if bounds.is_empty() { TriggerShape::default().bounds } else { bounds };
        let direction = find("direction");
        let default_dir = direction.map(|i| affine_from_rows(&world[i]));
        let direction_by_sequence = match direction {
            Some(i) => shape
                .sequences
                .iter()
                .filter_map(|q| {
                    let m = affine_from_rows(&shape.node_world_transforms_at(&q.name, 0)?[i]);
                    (Some(m) != default_dir).then(|| (q.name.clone(), m))
                })
                .collect(),
            None => Vec::new(),
        };
        TriggerShape {
            bounds,
            trigger_box: TriggerShape::trigger_box_of(bounds),
            direction: default_dir,
            direction_by_sequence,
            muzzle: find("muzzleNode").map(|i| Vec3::new(world[i][0][3], world[i][1][3], world[i][2][3])),
            collision: Some(crate::statics::StaticShape::from_shape(shape)).filter(|s| !s.triangles.is_empty()).map(Arc::new),
        }
    }
}

/// Loads every def the simulation understands from a registry.
impl Defs {
    pub fn tank(&self, name: &str) -> Option<TankParams> {
        self.fields(name).filter(|f| f.class.eq_ignore_ascii_case("TankDef")).map(|f| TankParams::from_fields(&f, Some(self)))
    }

    pub fn projectile(&self, name: &str) -> Option<ProjectileParams> {
        self.fields(name).filter(|f| f.class.eq_ignore_ascii_case("ProjectileDef")).map(|f| ProjectileParams::from_fields(&f))
    }

    pub fn power_up(&self, name: &str) -> Option<PowerUpParams> {
        self.fields(name).map(|f| PowerUpParams::from_fields(&f))
    }

    pub fn boost(&self, name: &str) -> Option<BoostParams> {
        self.fields(name).filter(|f| f.class.eq_ignore_ascii_case("BoostDef")).map(|f| BoostParams::from_fields(&f))
    }

    pub fn volcano(&self, name: &str) -> Option<VolcanoParams> {
        self.fields(name).filter(|f| f.class.eq_ignore_ascii_case("VolcanoDef")).map(|f| VolcanoParams::from_fields(&f, Some(self)))
    }
}

/// `FlagDef` (the scrum ball).
#[derive(Debug, Clone, PartialEq)]
pub struct FlagParams {
    pub name: String,
    pub shape_file: String,
    pub bounce_terrain: f32,
    pub bounce_static: f32,
    pub friction_terrain: f32,
    /// Milliseconds at rest before the unstuck kick.
    pub unstuck_time: i32,
    pub unstuck_boost: f32,
    pub no_pickup: bool,
    /// `practice`: the bots leave the ball alone.
    pub practice: bool,
    /// Ball box (shape space); from the shape when loaded.
    pub bounds: Aabb,
}

impl FlagParams {
    /// Absent fields: constructor values: bounce_terrain 0, bounce_static 0.5,
    /// friction_terrain 0.5, unstuck_time 1000, unstuck_boost 5.
    pub fn from_fields(f: &Fields) -> FlagParams {
        FlagParams {
            name: f.name.clone(),
            shape_file: f.str_or("shape", "").to_string(),
            bounce_terrain: f.f32_or("bounce_terrain", 0.0),
            bounce_static: f.f32_or("bounce_static", 0.5),
            friction_terrain: f.f32_or("friction_terrain", 0.5),
            unstuck_time: f.i32_or("unstuck_time", 1000),
            unstuck_boost: f.f32_or("unstuck_boost", 5.0),
            no_pickup: f.bool_or("no_pickup", false),
            practice: f.bool_or("practice", false),
            bounds: Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5)),
        }
    }

    pub fn set_shape(&mut self, shape: &Shape) {
        let b = Aabb::from_bounds(&shape.bounds);
        if !b.is_empty() {
            self.bounds = b;
        }
    }
}

impl Defs {
    pub fn flag(&self, name: &str) -> Option<FlagParams> {
        self.fields(name).filter(|f| f.class.eq_ignore_ascii_case("FlagDef")).map(|f| FlagParams::from_fields(&f))
    }

    pub fn tank_ai(&self, name: &str) -> Option<crate::ai::TankAiParams> {
        self.fields(name).filter(|f| f.class.eq_ignore_ascii_case("TankAiDef")).map(|f| crate::ai::TankAiParams::from_fields(&f))
    }
}

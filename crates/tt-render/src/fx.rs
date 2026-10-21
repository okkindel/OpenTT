//! Particles and explosions, simulated on the CPU and drawn by the renderer as
//! sprites and dynamic shapes ([`crate::Renderer::set_effects`]).
//!
//! - Particle init: `vel += emitterVel · inherit_velocity`, `acc = vel ·
//!   constant_acceleration`, `lifetime = lifetime_ms ± lifetime_variance_ms`
//!   (integer uniform), `spin = spinSpeed + rand(spin_random_min,
//!   spin_random_max)` degrees per second.
//! - Emission: `theta = rand(theta_min, theta_max)` about an axis perpendicular
//!   to the emitter axis, `phi = clock·phi_reference_vel/1000 +
//!   rand·phi_variance` about the axis, offset `ejection_offset`, speed
//!   `ejection_velocity ± velocity_variance`; period `ejection_period_ms ±
//!   period_variance_ms`; particles emitted during a step are spread along the
//!   path from the previous to the current position.
//! - Update: `a = acc − vel·drag + (0, 0, −9.81)·gravity`, `vel += a·dt`, `pos +=
//!   vel·dt`; colour and size interpolated over the `times` keys; billboards of
//!   side `size`. `inverse_alpha` selects alpha blending, otherwise additive.
//! - ThinkTanks' `ExplosionDef` has only `start_size`, `end_size`, `duration_ms`,
//!   `resource` and `sound`. A `.png` resource is a sprite whose side grows from
//!   `start_size` to `end_size` over `duration_ms` ms; a `.dts` resource plays
//!   its first sequence once over `duration · duration_ms` ms without scaling.
//!   There are no particle emitters, debris or light flashes.

use crate::render::DynamicInstance;
use crate::sprites::{Sprite, SpriteBlend};
use glam::{Mat4, Quat, Vec3};
use std::path::{Path, PathBuf};
use tt_assets::mission::SceneObject;

#[derive(Clone, Debug)]
pub struct ParticleParams {
    pub name: String,
    pub texture: PathBuf,
    pub drag: f32,
    pub wind: f32,
    pub gravity: f32,
    pub inherited_vel: f32,
    pub constant_acceleration: f32,
    pub lifetime_ms: i32,
    pub lifetime_variance_ms: i32,
    pub spin_speed: f32,
    pub spin_random: [f32; 2],
    pub inverse_alpha: bool,
    pub colors: [[f32; 4]; 4],
    pub sizes: [f32; 4],
    pub times: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct EmitterParams {
    pub name: String,
    pub period_ms: i32,
    pub period_variance_ms: i32,
    pub velocity: f32,
    pub velocity_variance: f32,
    pub offset: f32,
    pub theta: [f32; 2],
    pub phi_reference_vel: f32,
    pub phi_variance: f32,
    pub override_advance: bool,
    /// Emitter lifetime (0: forever).
    pub lifetime_ms: i32,
    /// Indices into [`FxLibrary::particles`].
    pub particles: Vec<usize>,
}

#[derive(Clone, Debug)]
pub enum ExplosionResource {
    Sprite(PathBuf),
    Shape(PathBuf),
    None,
}

#[derive(Clone, Debug)]
pub struct ExplosionParams {
    pub name: String,
    pub start_size: f32,
    pub end_size: f32,
    pub duration_ms: f32,
    pub resource: ExplosionResource,
    /// SoundProfile name (played by the caller).
    pub sound: Option<String>,
}

/// Effect defs of a mission.
#[derive(Clone, Debug, Default)]
pub struct FxLibrary {
    pub particles: Vec<ParticleParams>,
    pub emitters: Vec<EmitterParams>,
    pub explosions: Vec<ExplosionParams>,
}

fn num(o: &SceneObject, f: &str, d: f32) -> f32 {
    o.get(f).and_then(|v| v.trim().parse().ok()).unwrap_or(d)
}

fn flag(o: &SceneObject, f: &str, d: bool) -> bool {
    o.get(f).map_or(d, |v| v.eq_ignore_ascii_case("true") || v.trim().parse::<f32>().is_ok_and(|n| n != 0.0))
}

impl FxLibrary {
    /// Collects `ParticleDef`, `EmitterDef` and `ExplosionDef`
    /// defs (with inherited fields merged), resolving `~/` paths
    /// against `root`.
    pub fn from_defs(root: &Path, blocks: &[SceneObject]) -> FxLibrary {
        let is = |o: &SceneObject, c: &str| o.class.eq_ignore_ascii_case(c);
        let path = |v: &str| tt_assets::resolve(root, root, v);
        let mut lib = FxLibrary::default();
        for o in blocks.iter().filter(|o| is(o, "ParticleDef")) {
            let mut p = ParticleParams {
                name: o.name.clone(),
                texture: path(o.get("texture").unwrap_or("")),
                drag: num(o, "drag_coefficient", 0.0),
                wind: num(o, "wind_coefficient", 0.0),
                gravity: num(o, "gravity_coefficient", 0.0),
                inherited_vel: num(o, "inherit_velocity", 0.0),
                constant_acceleration: num(o, "constant_acceleration", 0.0),
                lifetime_ms: num(o, "lifetime_ms", 1000.0) as i32,
                lifetime_variance_ms: num(o, "lifetime_variance_ms", 0.0) as i32,
                spin_speed: num(o, "spin_speed", 0.0),
                spin_random: [num(o, "spin_random_min", 0.0), num(o, "spin_random_max", 0.0)],
                inverse_alpha: flag(o, "inverse_alpha", false),
                colors: [[1.0; 4]; 4],
                sizes: [1.0; 4],
                times: [0.0, 1.0, 2.0, 2.0],
            };
            for i in 0..4 {
                if let [r, g, b, a, ..] = o.floats(&format!("colors{i}")).as_slice() {
                    p.colors[i] = [*r, *g, *b, *a];
                }
                p.sizes[i] = num(o, &format!("sizes{i}"), p.sizes[i]);
                p.times[i] = num(o, &format!("times{i}"), p.times[i]);
            }
            lib.particles.push(p);
        }
        for o in blocks.iter().filter(|o| is(o, "EmitterDef")) {
            let particles = o
                .get("particles")
                .unwrap_or("")
                .split(|c: char| c.is_whitespace() || c == '\t')
                .filter(|n| !n.is_empty())
                .filter_map(|n| lib.particles.iter().position(|p| p.name.eq_ignore_ascii_case(n)))
                .collect();
            lib.emitters.push(EmitterParams {
                name: o.name.clone(),
                period_ms: (num(o, "ejection_period_ms", 100.0) as i32).max(1),
                period_variance_ms: num(o, "period_variance_ms", 0.0) as i32,
                velocity: num(o, "ejection_velocity", 2.0),
                velocity_variance: num(o, "velocity_variance", 1.0),
                offset: num(o, "ejection_offset", 0.0),
                theta: [num(o, "theta_min", 0.0), num(o, "theta_max", 90.0)],
                phi_reference_vel: num(o, "phi_reference_vel", 0.0),
                phi_variance: num(o, "phi_variance", 360.0),
                override_advance: flag(o, "override_advance", true),
                lifetime_ms: num(o, "lifetime_ms", 0.0) as i32,
                particles,
            });
        }
        for o in blocks.iter().filter(|o| is(o, "ExplosionDef")) {
            let res = o.get("resource").unwrap_or("").trim();
            let resource = if res.is_empty() {
                ExplosionResource::None
            } else if res.to_ascii_lowercase().ends_with(".dts") {
                ExplosionResource::Shape(path(res))
            } else {
                ExplosionResource::Sprite(path(res))
            };
            lib.explosions.push(ExplosionParams {
                name: o.name.clone(),
                start_size: num(o, "start_size", 1.0),
                end_size: num(o, "end_size", 1.0),
                duration_ms: num(o, "duration_ms", 1.0),
                resource,
                sound: o.get("sound").map(str::to_string),
            });
        }
        lib
    }

    pub fn emitter(&self, name: &str) -> Option<&EmitterParams> {
        self.emitters.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn explosion(&self, name: &str) -> Option<&ExplosionParams> {
        self.explosions.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

impl ExplosionParams {
    /// How long the explosion is drawn, ms: `duration_ms` times the
    /// shape's first sequence (a sprite: once).
    pub fn life_ms(&self) -> f64 {
        let duration = match &self.resource {
            ExplosionResource::Shape(p) => crate::pose::shape(p).and_then(|s| s.sequences.first().map(|q| q.duration)).unwrap_or(1.0),
            _ => 1.0,
        };
        (duration * self.duration_ms).max(1.0) as f64
    }
}

struct Particle {
    data: usize,
    pos: Vec3,
    vel: Vec3,
    acc: Vec3,
    age_ms: f32,
    life_ms: f32,
    /// Degrees per second.
    spin: f32,
}

struct EmitterState {
    data: usize,
    transform: Mat4,
    velocity: Vec3,
    last: Option<Vec3>,
    next_ms: i32,
    clock_ms: u32,
    elapsed_ms: u32,
    /// Removed: stops emitting (its particles live on).
    dead: bool,
}

struct ExplosionState {
    data: usize,
    pos: Vec3,
    start_ms: f64,
    life_ms: f64,
    /// Shape and its first sequence's name and duration.
    shape: Option<(PathBuf, Option<(String, f32)>)>,
}

/// Handle of a running emitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EmitterId(usize);

/// Running effects. Advance with [`Effects::advance`], then hand to
/// [`crate::Renderer::set_effects`].
pub struct Effects {
    lib: FxLibrary,
    emitters: Vec<Option<EmitterState>>,
    particles: Vec<Particle>,
    explosions: Vec<ExplosionState>,
    now_ms: f64,
    rng: u64,
    /// And the viewer position (see [`Effects::set_detail`]).
    detail_adjust: f32,
    viewer: Option<Vec3>,
}

/// Every emitter's far distance.
const PARTICLE_FAR_DIST: f32 = 1000.0;

impl Effects {
    pub fn new(lib: FxLibrary) -> Effects {
        Effects { lib, emitters: Vec::new(), particles: Vec::new(), explosions: Vec::new(), now_ms: 0.0, rng: 0x2545_f491_4f6c_dd1d, detail_adjust: 1.0, viewer: None }
    }

    /// Particle detail: emitters emit at `ejection_period_ms / f` with `f =
    /// detailAdjust · (1 − d² / farDist²)`, `d` the distance from the control
    /// object (here: `viewer`) to the emission point; below `f = 0.05` nothing
    /// is emitted. Without a viewer `f = detailAdjust`.
    pub fn set_detail(&mut self, detail_adjust: f32, viewer: Option<Vec3>) {
        self.detail_adjust = if detail_adjust.is_finite() { detail_adjust.max(0.0) } else { 1.0 };
        self.viewer = viewer;
    }

    /// The emission factor `f` of [`Effects::set_detail`] at `at`.
    pub fn emission_factor(&self, at: Vec3) -> f32 {
        match self.viewer {
            Some(v) => {
                let far2 = PARTICLE_FAR_DIST * PARTICLE_FAR_DIST;
                (far2 - at.distance_squared(v)) * self.detail_adjust / far2
            }
            None => self.detail_adjust,
        }
    }

    pub fn library(&self) -> &FxLibrary {
        &self.lib
    }

    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }

    fn rand(&mut self) -> f32 {
        // xorshift64*; visual only, not the engine's RNG.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
    }

    fn rand_var(&mut self, var: i32) -> i32 {
        if var <= 0 {
            return 0;
        }
        ((self.rand() * (2 * var + 1) as f32) as i32).min(2 * var) - var
    }

    /// Starts a continuous emitter. Its axis is the transform's Z axis.
    pub fn add_emitter(&mut self, emitter: &str, transform: Mat4) -> Option<EmitterId> {
        let data = self.lib.emitters.iter().position(|e| e.name.eq_ignore_ascii_case(emitter))?;
        let state = EmitterState { data, transform, velocity: Vec3::ZERO, last: None, next_ms: 0, clock_ms: 0, elapsed_ms: 0, dead: false };
        let slot = self.emitters.iter().position(Option::is_none).unwrap_or_else(|| {
            self.emitters.push(None);
            self.emitters.len() - 1
        });
        self.emitters[slot] = Some(state);
        Some(EmitterId(slot))
    }

    /// Moves an emitter; particles of the next [`Effects::advance`] are
    /// spread from the previous position to this one. `velocity` is
    /// inherited by new particles (`inherit_velocity`).
    pub fn set_emitter(&mut self, id: EmitterId, transform: Mat4, velocity: Vec3) {
        if let Some(Some(e)) = self.emitters.get_mut(id.0) {
            e.transform = transform;
            e.velocity = velocity;
        }
    }

    /// Stops an emitter; its particles finish their lives.
    pub fn remove_emitter(&mut self, id: EmitterId) {
        if let Some(slot) = self.emitters.get_mut(id.0) {
            *slot = None;
        }
    }

    /// One `emitParticles(from, to, axis, velocity, ms)` call of a fresh
    /// emitter: a burst of `ms` worth of particles along `from → to`.
    pub fn emit(&mut self, emitter: &str, from: Vec3, to: Vec3, axis: Vec3, velocity: Vec3, ms: u32) -> bool {
        let Some(data) = self.lib.emitters.iter().position(|e| e.name.eq_ignore_ascii_case(emitter)) else { return false };
        let mut st = EmitterState { data, transform: Mat4::IDENTITY, velocity, last: Some(from), next_ms: 0, clock_ms: 0, elapsed_ms: 0, dead: false };
        self.emit_particles(&mut st, from, to, axis, ms);
        true
    }

    /// Starts an explosion at `pos`. Returns false for unknown defs.
    pub fn explode(&mut self, explosion: &str, pos: Vec3) -> bool {
        let Some(data) = self.lib.explosions.iter().position(|e| e.name.eq_ignore_ascii_case(explosion)) else { return false };
        let e = &self.lib.explosions[data];
        let shape = match &e.resource {
            ExplosionResource::Shape(p) => Some((p.clone(), crate::pose::shape(p).and_then(|s| s.sequences.first().map(|q| (q.name.clone(), q.duration))))),
            _ => None,
        };
        let life_ms = e.life_ms();
        self.explosions.push(ExplosionState { data, pos, start_ms: self.now_ms, life_ms, shape });
        true
    }

    /// Advances all effects by `ms` milliseconds.
    pub fn advance(&mut self, ms: u32) {
        let dt = ms as f32;
        self.update_particles(dt, 0..self.particles.len());
        self.particles.retain(|p| p.age_ms < p.life_ms);
        let mut emitters = std::mem::take(&mut self.emitters);
        for e in emitters.iter_mut().flatten() {
            let lifetime = self.lib.emitters[e.data].lifetime_ms;
            e.elapsed_ms += ms;
            if lifetime > 0 && e.elapsed_ms as i32 > lifetime {
                e.dead = true;
            }
            if e.dead {
                continue;
            }
            let to = e.transform.w_axis.truncate();
            let from = e.last.unwrap_or(to);
            let axis = e.transform.z_axis.truncate().try_normalize().unwrap_or(Vec3::Z);
            self.emit_particles(e, from, to, axis, ms);
        }
        self.emitters = emitters;
        self.now_ms += ms as f64;
        let now = self.now_ms;
        self.explosions.retain(|x| now - x.start_ms < x.life_ms);
    }

    fn update_particles(&mut self, dt_ms: f32, range: std::ops::Range<usize>) {
        let dt = dt_ms / 1000.0;
        for p in &mut self.particles[range] {
            let d = &self.lib.particles[p.data];
            let a = p.acc - p.vel * d.drag + Vec3::new(0.0, 0.0, -9.81) * d.gravity;
            p.vel += a * dt;
            p.pos += p.vel * dt;
            p.age_ms += dt_ms;
        }
    }

    fn emit_particles(&mut self, e: &mut EmitterState, start: Vec3, end: Vec3, axis: Vec3, ms: u32) {
        let ed = self.lib.emitters[e.data].clone();
        if ed.particles.is_empty() {
            e.last = Some(end);
            return;
        }
        let factor = self.emission_factor(start);
        if factor < 0.05 {
            e.last = Some(end);
            return;
        }
        let ms_i = ms as i32;
        let mut curr = 0i32;
        let axisx = if axis.z.abs() < 0.9 { axis.cross(Vec3::Z) } else { axis.cross(Vec3::Y) }.normalize();
        let lerp = |t: i32| start.lerp(end, if ms_i > 0 { t as f32 / ms_i as f32 } else { 1.0 });
        if e.next_ms != 0 {
            if e.next_ms > ms_i {
                e.next_ms -= ms_i;
                e.clock_ms += ms;
                e.last = Some(end);
                return;
            }
            curr += e.next_ms;
            e.clock_ms += e.next_ms as u32;
            self.add_particle(&ed, e, lerp(curr), axis, axisx, ms_i - curr);
            e.next_ms = 0;
        }
        while curr < ms_i {
            let next = ((ed.period_ms as f32 / factor) as i32 + self.rand_var(ed.period_variance_ms)).max(1);
            if curr + next > ms_i {
                e.next_ms = curr + next - ms_i;
                e.clock_ms += (ms_i - curr) as u32;
                break;
            }
            curr += next;
            e.clock_ms += next as u32;
            self.add_particle(&ed, e, lerp(curr), axis, axisx, ms_i - curr);
        }
        e.last = Some(end);
    }

    fn add_particle(&mut self, ed: &EmitterParams, e: &EmitterState, pos: Vec3, axis: Vec3, axisx: Vec3, remaining_ms: i32) {
        let pick = ((self.rand() * ed.particles.len() as f32) as usize).min(ed.particles.len() - 1);
        let data = ed.particles[pick];
        let theta = (ed.theta[1] - ed.theta[0]) * self.rand() + ed.theta[0];
        let phi = e.clock_ms as f32 / 1000.0 * ed.phi_reference_vel + self.rand() * ed.phi_variance;
        let dir = Quat::from_axis_angle(axis, phi.to_radians()) * (Quat::from_axis_angle(axisx, theta.to_radians()) * axis);
        let speed = ed.velocity + ed.velocity_variance * 2.0 * self.rand() - ed.velocity_variance;
        let pd = self.lib.particles[data].clone();
        let vel = dir * speed + e.velocity * pd.inherited_vel;
        let life = (pd.lifetime_ms + self.rand_var(pd.lifetime_variance_ms)).max(1) as f32;
        let spin = pd.spin_speed + pd.spin_random[0] + (pd.spin_random[1] - pd.spin_random[0]) * self.rand();
        self.particles.push(Particle { data, pos: pos + dir * ed.offset, vel, acc: vel * pd.constant_acceleration, age_ms: 0.0, life_ms: life, spin });
        if !ed.override_advance && remaining_ms > 0 {
            let i = self.particles.len() - 1;
            self.update_particles(remaining_ms as f32, i..i + 1);
        }
    }

    /// Particles and sprite explosions to draw now.
    pub fn sprites(&self) -> Vec<Sprite> {
        let mut out = Vec::with_capacity(self.particles.len() + self.explosions.len());
        for p in &self.particles {
            let d = &self.lib.particles[p.data];
            let t = p.age_ms / p.life_ms;
            let (mut color, mut size) = (d.colors[0], d.sizes[0]);
            for i in 1..4 {
                if d.times[i] >= t {
                    let span = d.times[i] - d.times[i - 1];
                    let f = if span > 0.0 { ((t - d.times[i - 1]) / span).clamp(0.0, 1.0) } else { 1.0 };
                    color = std::array::from_fn(|c| d.colors[i - 1][c] + (d.colors[i][c] - d.colors[i - 1][c]) * f);
                    size = d.sizes[i - 1] + (d.sizes[i] - d.sizes[i - 1]) * f;
                    break;
                }
            }
            out.push(Sprite {
                texture: d.texture.clone(),
                pos: p.pos,
                size,
                rotation: (p.spin * p.age_ms / 1000.0).to_radians(),
                color,
                blend: if d.inverse_alpha { SpriteBlend::Alpha } else { SpriteBlend::Additive },
                ..Default::default()
            });
        }
        for x in &self.explosions {
            let e = &self.lib.explosions[x.data];
            if let ExplosionResource::Sprite(tex) = &e.resource {
                let f = (((self.now_ms - x.start_ms) / e.duration_ms.max(1.0) as f64) as f32).clamp(0.0, 1.0);
                out.push(Sprite {
                    texture: tex.clone(),
                    pos: x.pos,
                    size: f * e.end_size + (1.0 - f) * e.start_size,
                    ..Default::default()
                });
            }
        }
        out
    }

    /// Explosion shapes to draw now (posed at their sequence position).
    pub fn shapes(&self) -> Vec<DynamicInstance> {
        self.shapes_with_ifl().into_iter().map(|x| x.0).collect()
    }

    /// [`Effects::shapes`] with the IFL times of their sequences.
    pub(crate) fn shapes_with_ifl(&self) -> Vec<(DynamicInstance, Vec<(String, f32)>)> {
        let mut out = Vec::new();
        for x in &self.explosions {
            let Some((path, seq)) = &x.shape else { continue };
            let mut inst = DynamicInstance { shape: path.clone(), transform: Mat4::from_translation(x.pos), light: 1.0, ..Default::default() };
            if let (Some((name, dur)), Some(shape)) = (seq, crate::pose::shape(path)) {
                let pos = (((self.now_ms - x.start_ms) / x.life_ms) as f32).clamp(0.0, 0.999);
                let t = pos * dur;
                inst.nodes = crate::pose::sequence_pose_of(&shape, name, t);
                for (obj, vis) in crate::pose::sequence_visibility_of(&shape, name, t) {
                    if vis {
                        inst.shown.push(obj);
                    } else {
                        inst.hidden.push(obj);
                    }
                }
                out.push((inst, crate::pose::sequence_ifl_of(&shape, name, t)));
                continue;
            }
            out.push((inst, Vec::new()));
        }
        out
    }
}

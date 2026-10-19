//! The aiming reticle.
//!
//! The reticle belongs to the *current cartridge*: `ProjectileParams.reticle`
//! names a `ReticleDef`. It is a row of `tick_count` sprites placed where a
//! shell fired now would be after `start_time + i^spacing_exponent·time_step`
//! seconds (muzzle velocity only, gravity 10·gravity_scale), smoothed between
//! frames, plus "spot" sprites where the arc meets terrain, tanks or statics.
//!
//! The cartridge decides the arcs ([`ProjectileParams::launch_dirs`]): the
//! mortar's single high arc, the scatter shot's cone (dots along the middle
//! pellet, each as wide as the cone there, so they trace it; a spot where each
//! pellet lands when `show_spot`; dots only as far as a pellet flies,
//! `max_life_ms`; a fan: dots along the middle and the two outer pellets).
//! `ring_image` adds a flat ring as wide as the splash on the surface at the
//! landing spot. A carpet (a high-angle cartridge firing several shells,
//! [`ProjectileParams::is_carpet`]): the middle shell's arc at the dots' own
//! size, a small landing mark per shell of the next volley and one ring around
//! where every shell of a pull comes down (all `volley_size` volleys, each
//! with its own jitter) plus the splash.

use glam::Vec3;

use crate::data::{ProjectileParams, MAX_PELLETS};
use crate::fields::{Defs, Fields};
use crate::world::World;
use crate::{Tank, TankId};

pub const MAX_TICKS: usize = 20;

/// A carpet's landing marks: this much of the reticle's spot size.
pub const CARPET_MARK: f32 = 0.5;

/// `ReticleDef`. The script fields `animate`, `anim_image`, `anim_time` and
/// `anim_size` are not registered by the engine and have no effect.
#[derive(Debug, Clone, PartialEq)]
pub struct ReticleParams {
    pub name: String,
    /// Dot texture.
    pub image: String,
    /// Dot sprite size (m). For a cone cartridge (`spread_ring`) the
    /// smallest: each dot is as wide as the cone where it is (the art's
    /// marks run along the quad's edge, on the outer pellets' ring).
    pub size: f32,
    pub start_time: f32,
    pub time_step: f32,
    pub tick_count: i32,
    pub spacing_exponent: f32,
    /// Per-100 ms retention of the nearest / farthest dots (0.1..1).
    pub smooth_near: f32,
    pub smooth_far: f32,
    pub smooth_exponent: f32,
    pub show_spot: bool,
    pub spot_image: String,
    pub spot_size_near: f32,
    pub spot_size_far: f32,
    /// `ring_image`: flat splash ring at each spot ("" = none).
    pub ring_image: String,
}

impl ReticleParams {
    pub fn from_fields(f: &Fields) -> ReticleParams {
        ReticleParams {
            name: f.name.clone(),
            image: f.str_or("image", "").to_string(),
            size: f.f32_or("size", 1.0),
            start_time: f.f32_or("start_time", 0.5),
            time_step: f.f32_or("time_step", 0.5),
            tick_count: f.i32_or("tick_count", 10),
            spacing_exponent: f.f32_or("spacing_exponent", 1.0),
            smooth_near: f.f32_or("smooth_near", 0.5),
            smooth_far: f.f32_or("smooth_far", 0.5),
            smooth_exponent: f.f32_or("smooth_exponent", 0.5),
            show_spot: f.bool_or("show_spot", false),
            spot_image: f.str_or("spot_image", "").to_string(),
            spot_size_near: f.f32_or("spot_size_near", 1.0),
            spot_size_far: f.f32_or("spot_size_far", 1.0),
            ring_image: f.str_or("ring_image", "").to_string(),
        }
    }

    pub fn ticks(&self) -> usize {
        (self.tick_count.max(0) as usize).min(MAX_TICKS)
    }

    /// Flight time of dot `i`: `start_time + i^spacing_exponent · time_step`.
    pub fn tick_time(&self, i: usize) -> f32 {
        let e = if i == 0 { 0.0 } else { (i as f64).powf(self.spacing_exponent as f64) as f32 };
        e * self.time_step + self.start_time
    }

    /// Per-100 ms retention factor of dot `i` (before the elapsed-time power).
    pub fn retention(&self, i: usize) -> f32 {
        let f = (i as f32 / self.ticks().max(1) as f32).clamp(0.05, 0.95);
        let w = f.powf(self.smooth_exponent);
        (w * self.smooth_far + (1.0 - w) * self.smooth_near).clamp(0.1, 1.0)
    }
}

impl Defs {
    pub fn reticle(&self, name: &str) -> Option<ReticleParams> {
        self.fields(name).filter(|f| f.class.eq_ignore_ascii_case("ReticleDef")).map(|f| ReticleParams::from_fields(&f))
    }
}

/// Smoothing memory between frames, one row per arc (a scatter shot has one
/// per pellet). Positions are relative to the tank position.
#[derive(Debug, Clone, PartialEq)]
pub struct ReticleState {
    pub last_ms: i64,
    pub points: [[Vec3; MAX_TICKS]; MAX_PELLETS],
    pub dirs: [[Vec3; MAX_TICKS]; MAX_PELLETS],
}

impl Default for ReticleState {
    fn default() -> Self {
        ReticleState { last_ms: 0, points: [[Vec3::ZERO; MAX_TICKS]; MAX_PELLETS], dirs: [[Vec3::ZERO; MAX_TICKS]; MAX_PELLETS] }
    }
}

/// One reticle sprite. `dir` is the unit direction the textured quad is
/// aligned with (for dots: from the dot back along the arc towards the
/// muzzle; for spots: along the arc segment that hit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReticleSprite {
    pub pos: Vec3,
    pub dir: Vec3,
    pub size: f32,
}

/// Sprites for one frame (`spots` holds at most one per arc). Draw order: rings
/// (texture `ring_bmp`, lying on the surface: `dir` = its normal), spots (texture
/// `spot_bmp`), then dots (texture `bmp`); white, alpha-blended (SRC_ALPHA,
/// ONE_MINUS_SRC_ALPHA), texture modulate, depth test on, depth writes off, no
/// culling.
#[derive(Debug, Clone, PartialEq)]
pub struct Reticle {
    pub dots: Vec<ReticleSprite>,
    pub spots: Vec<ReticleSprite>,
    pub rings: Vec<ReticleSprite>,
    pub image: String,
    pub spot_image: String,
    pub ring_image: String,
}

impl World {
    /// Computes the reticle of `tank` for a frame drawn at `now_ms`. The
    /// engine should only call it for the tank the local client controls.
    pub fn reticle(&self, tank: TankId, data: &ReticleParams, state: &mut ReticleState, now_ms: i64) -> Option<Reticle> {
        self.reticle_of(self.tank(tank)?, data, state, now_ms)
    }

    /// [`World::reticle`] of a tank at a given pose: the renderer passes a
    /// copy at its interpolated (drawn) pose and turret aim.
    pub fn reticle_of(&self, t: &Tank, data: &ReticleParams, state: &mut ReticleState, now_ms: i64) -> Option<Reticle> {
        let p = t.current.as_ref()?;
        if t.is_corpse || t.ammo == 0 {
            return None;
        }
        let m = t.muzzle();
        // The next shot's pattern (a cone's jitter follows the shot number).
        let dirs = p.launch_dirs(&m, t.pitch_n, t.fire_count.wrapping_add(1));
        let elapsed = now_ms - state.last_ms;
        state.last_ms = now_ms;
        let n = data.ticks();
        // Dots no further than a pellet flies (`max_life_ms`).
        let shown = if p.max_life_ms > 0 { (0..n).filter(|&i| data.tick_time(i) * 1000.0 <= p.max_life_ms as f32).count() } else { n };
        let (mut dots, mut spots, mut rings) = (Vec::new(), Vec::new(), Vec::new());
        let last = dirs.len() - 1;
        // A cone: each dot as wide as the cone at its distance from the
        // muzzle (the outer pellets' ring, `spread_angle`). A carpet: the
        // arc at the dots' size, small landing marks and one ring around
        // the whole pattern (after the loop).
        let carpet = p.is_carpet();
        let cone = (p.spread_ring && last > 0 && !carpet).then(|| (0.5 * p.spread_angle.to_radians()).tan());
        let p0 = Vec3::from(m.translation) - t.pos;
        let ringed = !data.ring_image.is_empty() && p.blast_radius > 0.0;
        let mut centre = None;
        for (arc, dir) in dirs.iter().enumerate().take(MAX_PELLETS) {
            let (s, r) = self.reticle_arc(t, p, data, &mut state.points[arc], &mut state.dirs[arc], *dir, elapsed, shown);
            if carpet {
                spots.extend(s.map(|s| ReticleSprite { size: CARPET_MARK * s.size, ..s }));
                if arc == 0 {
                    centre = r;
                }
            } else {
                spots.extend(s);
                if ringed {
                    rings.extend(r.map(|(pos, normal)| ReticleSprite { pos, dir: normal, size: 2.0 * p.blast_radius }));
                }
            }
            // Dots along the middle arc and, for a fan, the two outer ones
            // (a cone or a carpet: the middle one; its spots ring the
            // impact area).
            let row = if p.spread_ring || carpet { arc == 0 } else { arc == 0 || arc == last || arc == last / 2 };
            if row {
                let size = |q: Vec3| cone.map_or(data.size, |tan| data.size.max(2.0 * tan * (q - p0).length()));
                dots.extend((0..shown).map(|i| ReticleSprite { pos: t.pos + state.points[arc][i], dir: state.dirs[arc][i], size: size(state.points[arc][i]) }));
            }
        }
        if let Some((pos, normal)) = centre.filter(|_| ringed) {
            let reach = carpet_reach(t, p, pos.z - t.pos.z);
            rings.push(ReticleSprite { pos, dir: normal, size: 2.0 * (reach + p.blast_radius) });
        }
        Some(Reticle { dots, spots, rings, image: data.image.clone(), spot_image: data.spot_image.clone(), ring_image: data.ring_image.clone() })
    }

    /// One arc of the reticle (launch direction `dir`): smooths its
    /// `points` / `dirs` towards this frame's targets and returns the spot
    /// where it first meets something within the first `shown` dots, with
    /// (hit point, surface normal).
    #[allow(clippy::too_many_arguments)]
    fn reticle_arc(&self, t: &Tank, p: &ProjectileParams, data: &ReticleParams, points: &mut [Vec3; MAX_TICKS], dirs: &mut [Vec3; MAX_TICKS], dir: Vec3, elapsed: i64, shown: usize) -> (Option<ReticleSprite>, Option<(Vec3, Vec3)>) {
        let tank = t.id;
        let m = t.muzzle();
        let origin = t.pos;
        let p0 = Vec3::from(m.translation) - origin;
        let v = dir * p.launch_speed(t.pitch_n);
        let g = -10.0 * p.gravity_scale;
        // A high-angle arc follows the simulation's ticks (explicit Euler:
        // g·DT·s/2 above the parabola), so the spot and ring sit where the
        // shell really lands (it flies long and falls fast).
        let euler = if p.high_angle() { -0.5 * g * crate::DT } else { 0.0 };
        let at = |s: f32| p0 + v * s + Vec3::new(0.0, 0.0, 0.5 * g * s * s + euler * s);

        let n = data.ticks();
        for i in 0..n {
            let s = data.tick_time(i);
            let target = at(s);
            let back = (at(s - 0.25) - target).normalize_or_zero();
            let k = if (0..=100).contains(&elapsed) { data.retention(i).powf(elapsed as f32 / 100.0) } else { 0.0 };
            points[i] = points[i] * k + target * (1.0 - k);
            dirs[i] = (dirs[i] * k + back * (1.0 - k)).normalize_or_zero();
        }

        if !data.show_spot {
            return (None, None);
        }
        let mut prev = Vec3::from(m.translation);
        for (i, point) in points.iter().enumerate().take(shown) {
            let cur = origin + *point;
            if let Some((_, hit, normal, _)) = self.ray_cast(prev, cur, Some(tank)) {
                let seg = cur - prev;
                let len = seg.length();
                // The loop ends at the first hit: at most one spot.
                if len <= 0.0 {
                    return (None, None);
                }
                let d = (hit - prev).length() / len;
                let a = if i > 0 { (i - 1) as f32 / n as f32 } else { 0.0 };
                let frac = (1.0 - d) * a + (i as f32 / n as f32) * d;
                let size = frac * data.spot_size_far + (1.0 - frac) * data.spot_size_near;
                let pull = (frac * 2.0 + (1.0 - frac) * 0.1) / len;
                return (Some(ReticleSprite { pos: hit - seg * pull, dir: seg / len, size }), Some((hit, normal)));
            }
            prev = cur;
        }
        // A pellet that runs out (`max_life_ms`) before it meets
        // anything: its spot where it ends, so the fan shows its reach.
        if p.max_life_ms > 0 && shown > 0 {
            let end = origin + points[shown - 1];
            let seg = (end - origin - p0).normalize_or_zero();
            return (Some(ReticleSprite { pos: end, dir: seg, size: data.spot_size_far }), None);
        }
        (None, None)
    }
}

/// How far from the middle shell's landing point (horizontally) a
/// carpet's shells come down, every shell of a whole pull (`volley_size`
/// volleys from the next shot, each with its own jitter) on level ground
/// at the height `dz` relative to the tank, along the same arcs as the
/// reticle's (the simulation's ticks).
fn carpet_reach(t: &Tank, p: &ProjectileParams, dz: f32) -> f32 {
    let m = t.muzzle();
    let p0 = Vec3::from(m.translation) - t.pos;
    let v = p.launch_speed(t.pitch_n);
    let a = -5.0 * p.gravity_scale;
    let euler = a * crate::DT;
    // p0.z + (v·d.z − euler)·s + a·s² = dz, the later root.
    let land = |d: Vec3| {
        let b = v * d.z - euler;
        let disc = b * b - 4.0 * a * (p0.z - dz);
        (disc >= 0.0 && a < 0.0).then(|| {
            let s = (-b - disc.sqrt()) / (2.0 * a);
            (p0 + d * v * s).truncate()
        })
    };
    let volleys = p.volley_size.max(1) as u32;
    let mut reach = 0.0f32;
    for k in 1..=volleys {
        let dirs = p.launch_dirs(&m, t.pitch_n, t.fire_count.wrapping_add(k));
        let Some(mid) = land(dirs[0]) else { continue };
        for d in &dirs[1..] {
            if let Some(q) = land(*d) {
                reach = reach.max((q - mid).length());
            }
        }
    }
    reach
}

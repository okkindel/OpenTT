//! Dynamic lights: explosions and the lamps of power-ups / pads light the terrain and shapes
//! around them with coloured point lights ([`tt_render::PointLight`], at most
//! [`tt_render::MAX_POINT_LIGHTS`] per view, the strongest near the camera kept). The
//! shaders add them as a tinted glow on the surface colour (N·L, smooth falloff, the sum
//! clamped), never more than +100 %.
//!
//! * explosions: a warm light of ~7 m for as long as the explosion is
//!   drawn (at least 0.3 s) plus a short fade, colour from the explosion's
//!   texture (sprite, or the shape's `NAME01.png`), else fire orange; big
//!   blasts (Mortar, volcano rocks, the Area power-up) a strong one about their own size (up
//!   to 25 m);
//! * power-ups (while on) and boost / jump pads: the colour of their lamp
//!   IFL's frame the renderer shows at that moment (the same IFL time, see
//!   [`crate::gfx::TriggerAnim`]), so the light follows the drawn
//!   animation: weapon / health pick-ups flash cyan / red once per 1.7 s
//!   `ambient` cycle (`powerupLight02..07` / `healthLight02..07`) and are
//!   dark in between (frame 01 is black); pads give a faint tint that
//!   flickers with their arrow frames. The Reload arch has no light;
//! * the ejected brain jar's rocket: a faint flickering flame.
//!
//! The Area power-up's blast (a shape about 45 m wide) is a big one too.
//!
//! Projectiles in flight and muzzle flashes do not light anything.

use crate::gfx::{Lamp, TriggerAnim};
use glam::Vec3;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tt_render::{PointLight, MAX_POINT_LIGHTS};

const FIRE: [f32; 3] = [1.0, 0.55, 0.2];
const EXPLOSION_LIFE: f32 = 0.3;
const EXPLOSION_RADIUS: f32 = 7.0;
/// Added fraction of the surface colour at the centre (before N·L).
const EXPLOSION_INTENSITY: f32 = 0.4;
/// Big blasts (sprite scale above [`BIG_EXPLOSION`], or a shape wider than [`BIG_SHAPE`]: the Mortar, volcano
/// rocks, the Area blast): a strong flash sized by the blast, as long as it is drawn.
const BIG_EXPLOSION: f32 = 5.0;
/// World width (m) above which a shape explosion is a big blast.
const BIG_SHAPE: f32 = 30.0;
const BIG_INTENSITY: f32 = 1.4;
/// The light outlives the drawn flame by this much (s), fading out.
const LIGHT_TAIL: f32 = 0.15;
/// Power-up lamp: light per unit of the lamp frame's mean colour (the
/// brightest frames are about 1.0 red / 0.88 cyan), above the pick-up.
const POWERUP_INTENSITY: f32 = 0.9;
const POWERUP_RADIUS: f32 = 7.0;
const POWERUP_HEIGHT: f32 = 1.5;
/// Pad arrows (frames about 0.45 cyan, the last 0.25): a faint tint.
const PAD_INTENSITY: f32 = 0.25;
const PAD_RADIUS: f32 = 4.5;
const PAD_HEIGHT: f32 = 1.0;
const ROCKET_RADIUS: f32 = 3.0;
const ROCKET_INTENSITY: f32 = 0.15;
/// Lights further than this from the camera are dropped.
const MAX_DISTANCE: f32 = 160.0;

struct Flash {
    pos: Vec3,
    color: [f32; 3],
    radius: f32,
    intensity: f32,
    life: f32,
    age: f32,
}

#[derive(Default)]
pub struct DynLights {
    flashes: Vec<Flash>,
    colors: HashMap<PathBuf, [f32; 3]>,
    /// Mean colour of the frame at each IFL tick, by (shape, material).
    lamps: HashMap<(PathBuf, String), Vec<[f32; 3]>>,
    time: f32,
    out: Vec<PointLight>,
}

/// Brightness-weighted mean colour of an image, strongest channel = 1.
fn image_color(path: &Path) -> Option<[f32; 3]> {
    let file = if path.extension().is_some() && path.exists() { path.to_path_buf() } else { tt_render::textures::find(path)? };
    let img = image::open(&file).ok()?.thumbnail(32, 32).to_rgba8();
    let mut sum = [0.0f32; 3];
    for p in img.pixels() {
        let [r, g, b, a] = p.0.map(|v| v as f32 / 255.0);
        let w = a * (r.max(g).max(b)).powi(2);
        sum[0] += r * w;
        sum[1] += g * w;
        sum[2] += b * w;
    }
    let m = sum[0].max(sum[1]).max(sum[2]);
    (m > 1e-4).then(|| sum.map(|c| c / m))
}

/// Plain mean colour of an image (0..1, alpha-weighted): how much light a
/// lamp frame gives (a black frame gives none).
fn mean_color(path: &Path) -> [f32; 3] {
    let Some(img) = image::open(path).ok().map(|i| i.thumbnail(32, 32).to_rgba8()) else { return [0.0; 3] };
    let mut sum = [0.0f32; 3];
    for p in img.pixels() {
        let [r, g, b, a] = p.0.map(|v| v as f32 / 255.0);
        sum[0] += r * a;
        sum[1] += g * a;
        sum[2] += b * a;
    }
    let n = (img.width() * img.height()).max(1) as f32;
    sum.map(|c| c / n)
}

/// Mean colour per tick of an IFL material's timeline (the renderer's).
fn lamp_timeline(shape: &Path, material: &str) -> Vec<[f32; 3]> {
    let Some(frames) = tt_render::shapes::ifl_timeline(shape, material) else { return Vec::new() };
    let mut by_file: HashMap<&Path, [f32; 3]> = HashMap::new();
    frames.iter().map(|f| *by_file.entry(f.as_path()).or_insert_with(|| mean_color(f))).collect()
}

/// The IFL tick the renderer shows at `time` seconds (shader: per-instance
/// time `floor(t * 30)`, frame `tick % len`).
fn ifl_tick(time: f32) -> usize {
    (time.max(0.0) * tt_render::shapes::IFL_TICKS_PER_SECOND).floor() as usize
}

impl DynLights {
    fn color_of(&mut self, path: &Path, fallback: [f32; 3]) -> [f32; 3] {
        *self.colors.entry(path.to_path_buf()).or_insert_with(|| image_color(path).unwrap_or(fallback))
    }

    /// Colour of the lamp frames `anim` shows on `shape` (summed over its
    /// IFLs), 0 while they are dark.
    fn lamp_color(&mut self, shape: &Path, anim: &TriggerAnim) -> [f32; 3] {
        let mut c = [0.0f32; 3];
        for (material, time) in &anim.ifl {
            let key = (shape.to_path_buf(), material.to_ascii_lowercase());
            let tl = self.lamps.entry(key).or_insert_with(|| lamp_timeline(shape, material));
            if let Some(f) = tl.get(ifl_tick(*time) % tl.len().max(1)) {
                for i in 0..3 {
                    c[i] += f[i];
                }
            }
        }
        c
    }

    /// Advances by `dt` seconds with this frame's new explosions, the
    /// power-ups / pads shown (`glows`, with their animation `anims`) and
    /// the live projectiles (only the rocket flame lights); returns the
    /// lights for the view at `camera`, scaled by `strength` (0..2).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f32, explosions: &[(String, Vec3)], fx: Option<&tt_render::fx::FxLibrary>, glows: &[tt_render::DynamicInstance], anims: &[TriggerAnim], projectiles: &[(u32, Vec3, Option<PathBuf>, &str)], camera: Vec3, strength: f32) -> &[PointLight] {
        self.time += dt;
        for f in &mut self.flashes {
            f.age += dt;
        }
        self.flashes.retain(|f| f.age < f.life);
        for (name, pos) in explosions {
            let (mut color, mut scale, mut size, mut drawn) = (FIRE, 1.0, 1.0, 0.0);
            if let Some(e) = fx.and_then(|l| l.explosion(name)) {
                let s = e.start_size.max(e.end_size);
                drawn = (e.life_ms() / 1000.0) as f32;
                scale = s.clamp(0.7, 1.3);
                // A sprite's scale is its size in metres. A shape's scales
                // the shape: only a shape blast wider than BIG_SHAPE metres
                // (the Area blast, ~45 m) counts as big, sized like a sprite.
                size = match &e.resource {
                    tt_render::fx::ExplosionResource::Shape(p) => {
                        let world = s * tt_render::pose::shape(p).map_or(0.0, |sh| 2.0 * sh.radius);
                        if world > BIG_SHAPE { world } else { s }
                    }
                    _ => s,
                };
                let tex = match &e.resource {
                    tt_render::fx::ExplosionResource::Sprite(p) => Some(p.clone()),
                    tt_render::fx::ExplosionResource::Shape(p) => p.file_stem().map(|s| p.with_file_name(format!("{}01", s.to_string_lossy()))),
                    tt_render::fx::ExplosionResource::None => None,
                };
                if let Some(t) = tex {
                    // Warm it up: mostly the texture colour, some fire.
                    let c = self.color_of(&t, FIRE);
                    color = std::array::from_fn(|i| c[i] * 0.6 + FIRE[i] * 0.4);
                }
            }
            let flash = if size > BIG_EXPLOSION {
                // About the blast's own size (Mortar 41 -> 25 m), lifted so
                // it lights the ground around rather than one spot.
                Flash { pos: *pos + Vec3::Z * 3.0, color, radius: (size * 0.6).clamp(12.0, 25.0), intensity: BIG_INTENSITY, life: drawn.max(EXPLOSION_LIFE) + LIGHT_TAIL, age: 0.0 }
            } else {
                Flash { pos: *pos + Vec3::Z * 1.0, color, radius: EXPLOSION_RADIUS * scale, intensity: EXPLOSION_INTENSITY, life: drawn.max(EXPLOSION_LIFE) + LIGHT_TAIL, age: 0.0 }
            };
            self.flashes.push(flash);
        }
        self.out.clear();
        let mut cands: Vec<(f32, PointLight)> = Vec::with_capacity(self.flashes.len() + glows.len() + 2);
        for f in &self.flashes {
            // Full while the flame burns, then a quick fade (1 - t^4).
            let t = f.age / f.life;
            let k = f.intensity * (1.0 - t * t * t * t);
            cands.push((0.0, PointLight { pos: f.pos, radius: f.radius, color: f.color.map(|v| v * k) }));
        }
        for (g, anim) in glows.iter().zip(anims) {
            let (k, radius, height) = match anim.lamp {
                Lamp::None => continue,
                Lamp::PowerUp => (POWERUP_INTENSITY, POWERUP_RADIUS, POWERUP_HEIGHT),
                Lamp::Pad => (PAD_INTENSITY, PAD_RADIUS, PAD_HEIGHT),
            };
            let mut c = self.lamp_color(&g.shape, anim);
            if anim.lamp == Lamp::PowerUp && c[2] > c[0] {
                // Weapon pick-ups (cyan lamp): a bluer light, same peak
                // brightness (health stays red).
                let m = c[0].max(c[1]).max(c[2]);
                c = [m * 0.25, m * 0.5, m];
            }
            // Above the object (its own up), so the ground around catches it (N·L).
            let pos = g.transform.transform_point3(Vec3::Z * height);
            cands.push((0.0, PointLight { pos, radius, color: c.map(|v| v * k) }));
        }
        for (id, pos, tex, emitter) in projectiles {
            if tex.is_none() && !emitter.is_empty() {
                let flicker = 0.8 + 0.2 * (self.time * 37.0 + *id as f32).sin() * (self.time * 23.0).cos();
                cands.push((0.0, PointLight { pos: *pos, radius: ROCKET_RADIUS, color: FIRE.map(|v| v * ROCKET_INTENSITY * flicker) }));
            }
        }
        let s = strength.clamp(0.0, 2.0);
        // The strongest at the camera: intensity × radius over distance.
        for c in &mut cands {
            c.1.color = c.1.color.map(|v| v * s);
            let d = c.1.pos.distance(camera);
            let k = c.1.color[0].max(c.1.color[1]).max(c.1.color[2]);
            c.0 = if d > MAX_DISTANCE + c.1.radius { -1.0 } else { k * c.1.radius / (d + c.1.radius) };
        }
        cands.retain(|c| c.0 > 1e-4);
        cands.sort_by(|a, b| b.0.total_cmp(&a.0));
        self.out.extend(cands.iter().take(MAX_POINT_LIGHTS).map(|c| c.1));
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn common(name: &str) -> Option<PathBuf> {
        tt_assets::find_ci(&tt_assets::layout::default_dir().join("models/items").join(name))
    }

    /// The health pick-up's lamp: dark for 10 ticks, red flash decaying
    /// over frames 02..07, dark again; the light follows exactly.
    #[test]
    fn health_lamp_follows_ifl() {
        let Some(shape) = common("health.dts") else { return };
        let mut dl = DynLights::default();
        let inst = tt_render::DynamicInstance { shape: shape.clone(), ..Default::default() };
        let at = |dl: &mut DynLights, t: f32| {
            let anim = TriggerAnim { ifl: tt_render::pose::sequence_ifl(&shape, "ambient", t), lamp: Lamp::PowerUp };
            dl.update(0.0, &[], None, std::slice::from_ref(&inst), &[anim], &[], Vec3::new(0.0, 5.0, 2.0), 1.0).first().map(|l| l.color)
        };
        assert_eq!(at(&mut dl, 0.1), None, "lamp dark before the flash");
        let c = at(&mut dl, 10.5 / 30.0).expect("lamp lit at frame 02");
        assert!(c[0] > 0.8 && c[1] < 0.05 && c[2] < 0.05, "red: {c:?}");
        let later = at(&mut dl, 12.5 / 30.0).unwrap();
        assert!(later[0] < c[0] * 0.5, "fades with the frames");
        assert_eq!(at(&mut dl, 1.0), None, "dark after the flash");
        // Next cycle (1.7 s) flashes again.
        assert!(at(&mut dl, 1.7 + 10.5 / 30.0).is_some());
        let none = TriggerAnim { ifl: tt_render::pose::sequence_ifl(&shape, "ambient", 10.5 / 30.0), lamp: Lamp::None };
        assert!(dl.update(0.0, &[], None, std::slice::from_ref(&inst), &[none], &[], Vec3::ZERO, 1.0).is_empty());
    }
}

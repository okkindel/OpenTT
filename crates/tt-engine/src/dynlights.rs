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
//!   flickers with their arrow frames;
//! * the Reload arch: a light bluish-white flicker around it while its
//!   lightning (`electro`, ~0.47 s of the `activate` thread) is drawn;
//! * the ejected brain jar's rocket: a faint flickering flame;
//! * the new maps' lamps ([`Glow`]): the Halloween map's jack-o'-lantern props (by file
//!   name), and tt3_5's ember wisps (a mission's `Wisp` objects of kind "ember",
//!   [`tt_render::Scene::wisps`]: no shape, no collision): a few small orange-yellow sparks
//!   rising from the lava and fading, drawn as additive sprites
//!   ([`DynLights::wisp_sprites`], `models/effects/ember.png`). They fade out with distance
//!   from the camera (no popping where the nearest are kept; a bigger one, the giant
//!   jack-o'-lantern, later) and give way to the other lights.
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
/// Reload arch lightning: bluish white, per unit of the lit IFL frames'
/// brightest channel (about 0.12..0.23), reaching a few metres past the posts.
const ARC: [f32; 3] = [0.75, 0.85, 1.0];
const ARC_INTENSITY: f32 = 2.5;
const ARC_REACH: f32 = 4.0;
const ARC_HEIGHT: f32 = 2.5;
const ROCKET_RADIUS: f32 = 3.0;
const ROCKET_INTENSITY: f32 = 0.15;
/// Lights further than this from the camera are dropped.
const MAX_DISTANCE: f32 = 160.0;
/// Prop lamps and wisps fade out from the first to the second distance (m)
/// from the camera and rank below the other lights by `PROP_RANK`.
const PROP_FADE: (f32, f32) = (55.0, 85.0);
const PROP_RANK: f32 = 0.25;
/// The ember wisps' spark sprite.
const EMBER_TEXTURE: &str = "models/effects/ember.png";

/// A light of the scene's own.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Glow {
    /// A jack-o'-lantern: orange, flickering gently.
    Lantern,
    /// An ember wisp of `size` (1: normal), pulsing slowly.
    Wisp { size: f32 },
}

impl Glow {
    /// The lamp a prop's shape carries: the Halloween props
    /// (`props/halloween/`) by the start of their file name, `lantern*` a
    /// jack-o'-lantern.
    fn of_prop(shape: &Path) -> Option<Glow> {
        let dir = shape.parent().and_then(|d| d.file_name())?.to_string_lossy().to_ascii_lowercase();
        let stem = shape.file_stem()?.to_string_lossy().to_ascii_lowercase();
        match dir.as_str() {
            "halloween" if stem.starts_with("lantern") => Some(Glow::Lantern),
            _ => None,
        }
    }

    /// Colour (times its intensity) and radius (m) at scale 1.
    fn light(self) -> ([f32; 3], f32) {
        match self {
            Glow::Lantern => ([1.0, 0.45, 0.1], 7.0),
            Glow::Wisp { .. } => ([1.0, 0.42, 0.1], 6.0),
        }
    }

    /// How bright it is at `t` seconds (its own `phase`).
    fn flicker(self, t: f32, phase: f32) -> f32 {
        match self {
            Glow::Lantern => 0.86 + 0.1 * (7.3 * t + phase).sin() * (3.1 * t + 2.0 * phase).sin() + 0.04 * (17.0 * t + 3.0 * phase).sin(),
            Glow::Wisp { .. } => 0.6 + 0.4 * (0.5 + 0.5 * (1.3 * t + phase).sin()),
        }
    }
}

/// A prop's lamp or a wisp, placed.
#[derive(Clone, Copy, Debug)]
struct PropLamp {
    glow: Glow,
    pos: Vec3,
    radius: f32,
    phase: f32,
}

impl PropLamp {
    /// Where it is at `t` seconds: a wisp drifts a little round its spot,
    /// bobbing up and down.
    fn pos_at(&self, t: f32) -> Vec3 {
        let ph = self.phase;
        match self.glow {
            Glow::Wisp { .. } => self.pos + Vec3::new(0.8 * (0.31 * t + ph).sin(), 0.8 * (0.27 * t + 2.0 * ph).cos(), 0.45 * (0.9 * t + 3.0 * ph).sin()),
            _ => self.pos,
        }
    }
}

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
    props: Vec<PropLamp>,
    ember_texture: PathBuf,
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
    /// The scene's lamps from now on: its props' and its wisps; `root` the
    /// asset directory (the wisps' sprites).
    pub fn set_scene(&mut self, scene: &tt_render::Scene, root: &Path) {
        self.set_props(&scene.props);
        for (k, &(pos, size)) in scene.wisps.iter().enumerate() {
            let glow = Glow::Wisp { size };
            self.props.push(PropLamp { glow, pos, radius: glow.light().1 * size.clamp(0.5, 2.0), phase: k as f32 * 2.399_963 });
        }
        self.ember_texture = root.join(EMBER_TEXTURE);
    }

    /// The ember wisps as drawn now: additive sparks rising 6 m in 4 s
    /// each, drifting, fading as they go, pulsing with their light.
    pub fn wisp_sprites(&self) -> Vec<tt_render::Sprite> {
        let mut out = Vec::new();
        for p in &self.props {
            let Glow::Wisp { size } = p.glow else { continue };
            let k = p.glow.flicker(self.time, p.phase);
            for s in 0..8 {
                let ph = p.phase + s as f32 * 2.399_963;
                let h = (0.25 * self.time + ph * 0.37).fract();
                let r = size * (0.6 + 1.2 * (ph * 1.7).sin().abs());
                let spark = p.pos + Vec3::new(r * (ph + 0.7 * h).cos(), r * (ph + 0.7 * h).sin(), 6.0 * size * h);
                out.push(tt_render::Sprite {
                    texture: self.ember_texture.clone(),
                    pos: spark,
                    size: 0.4 * size * (1.0 - 0.5 * h),
                    color: [1.0, 1.0, 1.0, (1.0 - h) * k],
                    blend: tt_render::SpriteBlend::Additive,
                    rotation: 0.4 * self.time + p.phase,
                    ..Default::default()
                });
            }
        }
        out
    }

    /// The props (shape file, transform) with a lamp ([`Glow::of_prop`]):
    /// it sits inside the shape's bounds, low (the ground around catches
    /// it).
    fn set_props(&mut self, props: &[(PathBuf, glam::Mat4)]) {
        self.props = props
            .iter()
            .filter_map(|(shape, xf)| {
                let glow = Glow::of_prop(shape)?;
                let b = tt_render::pose::shape(shape).map(|s| s.bounds)?;
                let (lo, hi) = (Vec3::from(b.min), Vec3::from(b.max));
                let at = Vec3::new(0.5, 0.5, 0.45);
                let pos = xf.transform_point3(lo + (hi - lo) * at);
                let scale = xf.x_axis.truncate().length();
                // Each its own flicker.
                let phase = ((pos.x * 12.9898 + pos.y * 78.233).sin() * 43758.547).fract() * std::f32::consts::TAU;
                Some(PropLamp { glow, pos, radius: glow.light().1 * scale.clamp(0.5, 3.5), phase })
            })
            .collect();
    }

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
                // The arch's bolt: lit only while drawn, flickering with its
                // IFL frames (electro + rechargelight), over the arch's size.
                Lamp::Arch if g.shown.iter().any(|o| o.eq_ignore_ascii_case("electro")) => {
                    let m = self.lamp_color(&g.shape, anim).into_iter().fold(0.0, f32::max);
                    let radius = tt_render::pose::shape(&g.shape).map_or(6.0, |s| s.radius) + ARC_REACH;
                    let pos = g.transform.transform_point3(Vec3::Z * ARC_HEIGHT);
                    cands.push((0.0, PointLight { pos, radius, color: ARC.map(|v| v * m * ARC_INTENSITY) }));
                    continue;
                }
                Lamp::Arch => continue,
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
        let props_from = cands.len();
        for p in &self.props {
            let pos = p.pos_at(self.time);
            let d = pos.distance(camera);
            // Big lamps (the giant jack-o'-lantern) reach further, so fade later.
            let k = (p.radius / 7.0).max(1.0);
            let (near, far) = (PROP_FADE.0 * k, PROP_FADE.1 * k);
            let fade = 1.0 - (((d - near) / (far - near)).clamp(0.0, 1.0));
            if fade <= 0.0 {
                continue;
            }
            let k = p.glow.flicker(self.time, p.phase) * fade * fade;
            cands.push((0.0, PointLight { pos, radius: p.radius, color: p.glow.light().0.map(|v| v * k) }));
        }
        let s = strength.clamp(0.0, 2.0);
        // The strongest at the camera: intensity × radius over distance.
        for (i, c) in cands.iter_mut().enumerate() {
            c.1.color = c.1.color.map(|v| v * s);
            let d = c.1.pos.distance(camera);
            let k = c.1.color[0].max(c.1.color[1]).max(c.1.color[2]);
            c.0 = if d > MAX_DISTANCE + c.1.radius { -1.0 } else { k * c.1.radius / (d + c.1.radius) * if i >= props_from { PROP_RANK } else { 1.0 } };
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

    /// The Reload arch: dark until its lightning shows (0.13 s into
    /// `activate`), a bluish-white light about the arch's size while the
    /// bolt is drawn (flickering with its frames), dark once it is gone.
    #[test]
    fn arch_lightning_lights() {
        let Some(shape) = common("recharge.dts") else { return };
        let mut dl = DynLights::default();
        let mut at = |t: f32| {
            let (nodes, hidden, shown, ifl) = crate::engine::trigger_pose(&shape, "ambient", 5.0, &[("activate", t)]);
            let inst = tt_render::DynamicInstance { shape: shape.clone(), nodes, hidden, shown, ..Default::default() };
            dl.update(0.0, &[], None, &[inst], &[TriggerAnim { ifl, lamp: Lamp::Arch }], &[], Vec3::new(0.0, 8.0, 2.0), 1.0).first().copied()
        };
        assert!(at(0.05).is_none(), "dark before the bolt");
        let lit: Vec<PointLight> = (5..18).map(|i| at(i as f32 / 30.0).expect("lit while the bolt is drawn")).collect();
        for l in &lit {
            assert!(l.color[2] >= l.color[1] && l.color[1] > l.color[0] && l.color[0] > 0.6 * l.color[2], "bluish white: {:?}", l.color);
            assert!((0.2..0.8).contains(&l.color[2]), "light, not blinding: {:?}", l.color);
            assert!((6.0..20.0).contains(&l.radius), "about the arch: {}", l.radius);
        }
        assert!(lit.windows(2).any(|w| w[0].color != w[1].color), "flickers");
        assert!(at(0.7).is_none() && at(2.0).is_none(), "dark once the bolt is gone");
    }

    /// The Halloween props' lamps: chosen by the shape's name (only in
    /// `props/halloween/`); a jack-o'-lantern lights orange around it,
    /// flickers gently, fades out with distance and gives way to the
    /// other lights when there are too many.
    #[test]
    fn prop_lamps_glow() {
        let dir = tt_assets::layout::default_dir().join("models/props/halloween");
        assert_eq!(Glow::of_prop(&dir.join("lantern_tall.dts")), Some(Glow::Lantern));
        assert_eq!(Glow::of_prop(&dir.join("pumpkin_carved.dts")), None);
        assert_eq!(Glow::of_prop(Path::new("models/props/green/rowboat.dts")), None);
        let Some(shape) = tt_assets::find_ci(&dir.join("lantern.dts")) else { return };
        let mut dl = DynLights::default();
        dl.set_props(&[(shape.clone(), glam::Mat4::from_translation(Vec3::new(10.0, 0.0, 100.0)))]);
        let cam = Vec3::new(10.0, -8.0, 103.0);
        let lit: Vec<PointLight> = (0..30).map(|_| *dl.update(0.05, &[], None, &[], &[], &[], cam, 1.0).first().expect("the lantern lights")).collect();
        for l in &lit {
            assert!(l.color[0] > 0.6 && l.color[1] < 0.6 * l.color[0] && l.color[2] < 0.2 * l.color[0], "orange: {:?}", l.color);
            assert!((0.3..1.0).contains(&(l.pos.z - 100.0)) && (5.0..10.0).contains(&l.radius), "inside the pumpkin, lighting around it: {l:?}");
        }
        assert!(lit.windows(2).any(|w| w[0].color != w[1].color), "flickers");
        let max = lit.iter().map(|l| l.color[0]).fold(0.0, f32::max);
        assert!(lit.iter().all(|l| l.color[0] > 0.75 * max), "gently");
        // Faded out far away.
        assert!(dl.update(0.0, &[], None, &[], &[], &[], Vec3::new(10.0, PROP_FADE.1 + 5.0, 100.0), 1.0).is_empty());
        // Many lanterns: the strongest MAX_POINT_LIGHTS kept, an explosion first.
        let many: Vec<(PathBuf, glam::Mat4)> = (0..40).map(|k| (shape.clone(), glam::Mat4::from_translation(Vec3::new(k as f32 * 2.0, 0.0, 100.0)))).collect();
        dl.set_props(&many);
        let out = dl.update(0.0, &[("x".to_string(), Vec3::new(30.0, 0.0, 100.0))], None, &[], &[], &[], cam, 1.0).to_vec();
        assert_eq!(out.len(), MAX_POINT_LIGHTS);
        assert!(out.iter().any(|l| (l.pos.x - 30.0).abs() < 0.1 && l.pos.z > 100.9), "the explosion is kept");
    }
}

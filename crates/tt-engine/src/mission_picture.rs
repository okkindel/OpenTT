//! Offscreen mission pictures (the menu's `missions/<id>.png`) rendered
//! by the game's renderer with the default graphics: the scene built like
//! the game's (the mission's objects, the conditional ones as with no game
//! type, with the server defs and the mission's own), power-ups,
//! reload arches and boost/jump pads drawn as the game draws them
//! (animated at a given time, lamps switched on, with their dynamic
//! lights), the renderer settings of `tt_shell::prefs::GRAPHICS_DEFAULTS`
//! (anisotropy 16, default detail), the game's view
//! ([`crate::gfx::render_game_view`]).
//!
//! Used by the `mission_picture` example and the importer
//! (`tt-import --pictures`).

use crate::gfx::{GameView, Lamp, TriggerAnim};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tt_assets::mission::SceneObject;
use tt_render::{Camera, Renderer, Scene};

fn transform(o: &SceneObject) -> Mat4 {
    let pos = match o.floats("position").as_slice() {
        [x, y, z] => Vec3::new(*x, *y, *z),
        _ => Vec3::ZERO,
    };
    let rot = match o.floats("rotation").as_slice() {
        [x, y, z, deg] if Vec3::new(*x, *y, *z).length_squared() > 0.0 => Quat::from_axis_angle(Vec3::new(*x, *y, *z).normalize(), -deg.to_radians()),
        _ => Quat::IDENTITY,
    };
    let scale = match o.floats("scale").as_slice() {
        [x, y, z] => Vec3::new(*x, *y, *z),
        _ => Vec3::ONE,
    };
    Mat4::from_scale_rotation_translation(scale, rot, pos)
}

/// A power-up / reload arch / pad: shape, transform, the sequence it plays and its lamp.
pub struct Trigger {
    shape: PathBuf,
    transform: Mat4,
    seq: String,
    lamp: Lamp,
}

pub fn load(root: &Path, mission: &str) -> Result<(Scene, Vec<Trigger>), String> {
    let ron = if mission.ends_with(".ron") { PathBuf::from(mission) } else { root.join(tt_assets::layout::mission(mission)) };
    let m = tt_assets::data::mission::load(&ron)?;
    let dir = ron.parent().unwrap_or(root).to_path_buf();
    // The game's defs: the data files', then the mission's own.
    let mut set = tt_assets::data::defs::DefSet::new();
    for stem in tt_assets::data::defs::SETS {
        for d in tt_assets::data::defs::load_file(root, stem)? {
            set.declare_def(&d);
        }
    }
    for d in &m.defs {
        set.declare_def(d);
    }
    let defs: Vec<SceneObject> = set.objects().cloned().collect();
    let objects = m.scene_objects(|c| !c.unless.1.is_empty());
    let all: Vec<&SceneObject> = objects.iter().flat_map(|o| o.walk()).collect();
    let is_trigger = |o: &SceneObject| o.class.eq_ignore_ascii_case("PowerUp") || o.class.eq_ignore_ascii_case("Boost");
    let mut scene = Scene::from_objects_filtered(root, &dir, &all, &defs, &|o| !is_trigger(o))?;
    scene.markers.clear();
    let mut triggers = Vec::new();
    for o in all.iter().filter(|o| is_trigger(o)) {
        let Some(db) = o.get("def").or_else(|| o.get("def")).and_then(|n| set.get(n)) else {
            eprintln!("warning: {} {}: no def", o.class, o.name);
            continue;
        };
        let shape = db.get("shape").unwrap_or("");
        if shape.is_empty() {
            continue;
        }
        let (seq, lamp) = if o.class.eq_ignore_ascii_case("PowerUp") {
            // The Reload arch (type "ammo") has no lamp.
            let kind = db.get("kind").unwrap_or("");
            ("ambient".to_string(), if kind.eq_ignore_ascii_case("ammo") { Lamp::None } else { Lamp::PowerUp })
        } else {
            // A pad in its first state (`state_name1`, else `ambient`).
            let name = db.get("state_name1").unwrap_or("");
            (if name.is_empty() { "ambient".to_string() } else { name.to_string() }, Lamp::Pad)
        };
        triggers.push(Trigger { shape: tt_assets::resolve(root, &dir, shape), transform: transform(o), seq, lamp });
    }
    Ok((scene, triggers))
}

/// The triggers posed at `t` seconds as the game draws them
/// (`Engine::trigger_shapes_with_anim`).
pub fn posed(triggers: &[Trigger], t: f32) -> (Vec<tt_render::DynamicInstance>, Vec<TriggerAnim>) {
    let mut out = Vec::new();
    let mut anims = Vec::new();
    for tr in triggers {
        let nodes = tt_render::pose::sequence_pose(&tr.shape, &tr.seq, t);
        let ifl = tt_render::pose::sequence_ifl(&tr.shape, "ambient", t);
        let (mut hidden, mut shown) = (Vec::new(), Vec::new());
        for (obj, v) in tt_render::pose::sequence_visibility_level(&tr.shape, "ambient", t) {
            if v >= 0.5 {
                shown.push(obj);
            } else {
                hidden.push(obj);
            }
        }
        out.push(tt_render::DynamicInstance { shape: tr.shape.clone(), transform: tr.transform, nodes, hidden, shown, ..Default::default() });
        anims.push(TriggerAnim { ifl, lamp: tr.lamp });
    }
    (out, anims)
}

/// The game's default renderer settings (Options → Graphics defaults).
pub fn settings() -> tt_render::RenderSettings {
    let d = tt_shell::prefs::GRAPHICS_DEFAULTS;
    let extras = tt_render::Extras {
        glass: d.glass_jar,
        hdr: d.hdr,
        bloom: d.bloom,
        normal_maps: d.normal_maps,
        normal_strength: d.normal_strength,
        ao_strength: d.ao_strength,
        shadows: d.shadows,
        ssao: d.ssao,
    };
    tt_render::RenderSettings { render_scale: 1.0, anisotropy: 16, detail: Default::default(), extras }
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub struct Pictures {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    pub scene: Scene,
    targets: HashMap<(u32, u32), Target>,
    dynamic: Vec<tt_render::DynamicInstance>,
    anims: Vec<TriggerAnim>,
    lights_dynamic: bool,
    time: f32,
    ssaa: u32,
}

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

impl Pictures {
    /// The renderer set up for mission `mission` (a name looked up in
    /// `root`, or a `.ron` path): its power-ups and pads posed at `time`
    /// seconds (`triggers`), pictures rendered `ssaa` times larger and
    /// box-filtered down.
    pub fn new(root: &Path, mission: &str, time: f32, triggers: bool, ssaa: u32) -> Result<Pictures, String> {
        let (scene, trigger_list) = load(root, mission)?;
        let instance = crate::gfx::instance();
        let (_, device, queue) = pollster::block_on(crate::gfx::gpu(None, &instance))?;
        let mut renderer = Renderer::new_with_settings(&device, &queue, FORMAT, &scene, settings());
        let (dynamic, anims) = if triggers { posed(&trigger_list, time) } else { Default::default() };
        let shapes: Vec<PathBuf> = dynamic.iter().map(|d| d.shape.clone()).collect();
        renderer.preload(&device, &queue, &shapes, &[], Some(&scene.fx));
        let lights_dynamic = tt_shell::prefs::GRAPHICS_DEFAULTS.dynamic_lights;
        eprintln!("{mission}: {} power-ups/pads", dynamic.len());
        Ok(Pictures { device, queue, renderer, scene, targets: HashMap::new(), dynamic, anims, lights_dynamic, time, ssaa: ssaa.clamp(1, 4) })
    }

    /// Renders the view from `cam` (x y z yaw pitch hfov, degrees) at
    /// `size` into the PNG `out`.
    pub fn render(&mut self, out: &Path, cam: [f32; 6], size: (u32, u32)) -> Result<(), String> {
        let [x, y, z, yaw, pitch, hfov] = cam;
        let big = (size.0 * self.ssaa, size.1 * self.ssaa);
        let device = &self.device;
        let target = self.targets.entry(big).or_insert_with(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("mission picture"),
                size: wgpu::Extent3d { width: big.0, height: big.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            Target { texture, view }
        });
        let camera = Camera { pos: Vec3::new(x, y, z), yaw: yaw.to_radians(), pitch: pitch.to_radians() };
        if self.lights_dynamic {
            // A fresh light state each picture (no explosions, no projectiles).
            let mut dl = crate::dynlights::DynLights::default();
            let lights = dl.update(0.0, &[], Some(&self.scene.fx), &self.dynamic, &self.anims, &[], camera.pos, 1.0).to_vec();
            self.renderer.set_point_lights(&lights);
        }
        let gv = GameView { camera, hfov, dynamic: Vec::new(), ifl: Vec::new(), triggers: self.dynamic.clone(), trigger_anim: self.anims.clone(), sprites: Vec::new(), smoke: Vec::new() };
        crate::gfx::render_game_view(&mut self.renderer, &self.scene, &self.device, &self.queue, &target.view, big, &gv, None, self.time);
        let px = crate::gfx::read_rgba(&self.device, &self.queue, &target.texture, big, FORMAT);
        let img = image::RgbaImage::from_raw(big.0, big.1, px).ok_or("bad frame")?;
        let img = if self.ssaa > 1 { downscale(&img, self.ssaa) } else { img };
        let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
        rgb.save(out).map_err(|e| format!("{}: {e}", out.display()))
    }
}

/// Box filter by an integer factor (in linear light).
pub fn downscale(img: &image::RgbaImage, f: u32) -> image::RgbaImage {
    let (w, h) = (img.width() / f, img.height() / f);
    let to_lin = |v: u8| (v as f32 / 255.0).powf(2.2);
    let to_srgb = |v: f32| (v.max(0.0).powf(1.0 / 2.2) * 255.0 + 0.5).min(255.0) as u8;
    let mut out = image::RgbaImage::new(w, h);
    let n = (f * f) as f32;
    for y in 0..h {
        for x in 0..w {
            let mut s = [0.0f32; 3];
            for dy in 0..f {
                for dx in 0..f {
                    let p = img.get_pixel(x * f + dx, y * f + dy).0;
                    for c in 0..3 {
                        s[c] += to_lin(p[c]);
                    }
                }
            }
            out.put_pixel(x, y, image::Rgba([to_srgb(s[0] / n), to_srgb(s[1] / n), to_srgb(s[2] / n), 255]));
        }
    }
    out
}


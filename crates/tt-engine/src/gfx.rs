//! Frame drawing (3D scene or a cleared canvas) and offscreen screenshots;
//! the adapter/device setup is [`tt_render::gpu`]'s.

use std::path::Path;
use tt_render::{Camera, Renderer, Scene};

/// Background when no 3D view is shown (menus).
pub const CLEAR: wgpu::Color = wgpu::Color { r: 0.02, g: 0.02, b: 0.03, a: 1.0 };

pub use tt_render::gpu::{adapter_rank, device_limits, gpu, initial_camera, instance, read_rgba, render_target};

pub use tt_render::Target;

pub fn clear(device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView) {
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(CLEAR), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    queue.submit([enc.finish()]);
}

/// Releases a window's GPU state in a fixed order: waits for the GPU, then
/// the window's surface (while the device is there to unconfigure it),
/// `resources` (renderers and their buffers and textures), the queue, the
/// device and last the instance (with the GL backend it owns the EGL
/// display, which is bound to the window system's connection). Call it
/// while the window and the event loop are still there.
pub fn release_gpu<R>(surface: wgpu::Surface<'static>, resources: R, device: wgpu::Device, queue: wgpu::Queue, instance: wgpu::Instance) {
    device.poll(wgpu::Maintain::Wait);
    drop(surface);
    drop(resources);
    drop(queue);
    drop(device);
    drop(instance);
}

/// The game view: camera, horizontal FOV (degrees) and moving shapes.
pub struct GameView {
    pub camera: Camera,
    pub hfov: f32,
    /// Tanks, `.dts` projectiles, balls (shadowed).
    pub dynamic: Vec<tt_render::DynamicInstance>,
    /// IFL material times per `dynamic` instance (tank treads).
    pub ifl: Vec<Vec<(String, f32)>>,
    /// Power-ups and pads (animated, not shadowed).
    pub triggers: Vec<tt_render::DynamicInstance>,
    /// IFL times and lamp per `triggers` instance.
    pub trigger_anim: Vec<TriggerAnim>,
    /// `.png` projectiles.
    pub sprites: Vec<tt_render::Sprite>,
    /// Damage smoke emitters: (key, emitter def, transform, velocity).
    pub smoke: Vec<(u32, String, glam::Mat4, glam::Vec3)>,
}

/// What lights a power-up / pad casts (dynamic lights extra).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lamp {
    #[default]
    None,
    /// The Reload arch: a bluish-white flicker while its lightning
    /// (`electro`) is drawn.
    Arch,
    /// Weapon / health pick-up: its lamp IFL's flash.
    PowerUp,
    /// Boost / jump pad: a faint tint of its arrow IFL.
    Pad,
}

/// Animation of a power-up / pad as drawn: the IFL times the renderer
/// shows (material, seconds; see [`tt_render::InstanceOptions::ifl`]) and
/// its lamp. The dynamic lights read the lamp's colour from the same IFL
/// frame.
#[derive(Clone, Debug, Default)]
pub struct TriggerAnim {
    pub ifl: Vec<(String, f32)>,
    pub lamp: Lamp,
}

/// Particle/explosion state that lives across frames (one per scene).
pub struct FxState {
    pub effects: tt_render::fx::Effects,
    emitters: std::collections::HashMap<u32, (String, tt_render::fx::EmitterId)>,
}

impl FxState {
    pub fn new(scene: &Scene) -> FxState {
        FxState { effects: tt_render::fx::Effects::new(scene.fx.clone()), emitters: Default::default() }
    }

    /// Starts new explosions, moves/creates/removes the smoke emitters and
    /// advances everything by `ms`.
    pub fn update(&mut self, explosions: &[(String, glam::Vec3)], smoke: &[(u32, String, glam::Mat4, glam::Vec3)], ms: u32) {
        for (name, pos) in explosions {
            self.effects.explode(name, *pos);
        }
        let live: std::collections::HashSet<u32> = smoke.iter().map(|s| s.0).collect();
        let gone: Vec<u32> = self.emitters.keys().copied().filter(|k| !live.contains(k)).collect();
        for k in gone {
            if let Some((_, id)) = self.emitters.remove(&k) {
                self.effects.remove_emitter(id);
            }
        }
        for (k, name, m, vel) in smoke {
            match self.emitters.get(k) {
                Some((n, id)) if n == name => self.effects.set_emitter(*id, *m, *vel),
                _ => {
                    if let Some(id) = self.effects.add_emitter(name, *m) {
                        self.emitters.insert(*k, (name.clone(), id));
                    }
                }
            }
        }
        self.effects.advance(ms);
    }
}

/// Draws the scene from a game view (moving shapes lit by the lightmap).
pub fn render_game_view(renderer: &mut Renderer, scene: &Scene, target: Target, gv: &GameView, fx: Option<&FxState>, time: f32) {
    let mut owned = GameView {
        camera: gv.camera,
        hfov: gv.hfov,
        dynamic: gv.dynamic.clone(),
        ifl: gv.ifl.clone(),
        triggers: gv.triggers.clone(),
        trigger_anim: gv.trigger_anim.clone(),
        sprites: gv.sprites.clone(),
        smoke: Vec::new(),
    };
    render_game_view_taking(renderer, scene, target, &mut owned, fx, time);
}

/// [`render_game_view`] that moves the instances out of `gv` (they are
/// handed to the renderer as they are, lit, without copies): `gv.dynamic`,
/// `ifl`, `triggers` and the triggers' IFL times are left empty.
pub fn render_game_view_taking(renderer: &mut Renderer, scene: &Scene, target: Target, gv: &mut GameView, fx: Option<&FxState>, time: f32) {
    let Target { device, queue, size, .. } = target;
    let mut dynamic = std::mem::take(&mut gv.dynamic);
    let mut options: Vec<tt_render::render::InstanceOptions> =
        (0..dynamic.len()).map(|i| tt_render::render::InstanceOptions { shadow: true, ifl: gv.ifl.get_mut(i).map(std::mem::take).unwrap_or_default() }).collect();
    options.extend((0..gv.triggers.len()).map(|i| tt_render::render::InstanceOptions { shadow: false, ifl: gv.trigger_anim.get_mut(i).map(|a| std::mem::take(&mut a.ifl)).unwrap_or_default() }));
    dynamic.append(&mut gv.triggers);
    for d in &mut dynamic {
        d.light = scene.shape_light(d.transform.w_axis.truncate());
    }
    renderer.set_dynamic_with(device, queue, &dynamic, &options);
    renderer.set_sprites(&gv.sprites);
    if let Some(f) = fx {
        renderer.set_effects(device, queue, &f.effects);
    }
    let (w, h) = size;
    let aspect = w.max(1) as f32 / h.max(1) as f32;
    let camera = Camera { pos: gv.camera.pos, yaw: gv.camera.yaw, pitch: gv.camera.pitch };
    let view = tt_render::View { camera, fov_deg: crate::game::game_vfov(gv.hfov, aspect), viewport: Some([0, 0, w, h]), clear: true };
    renderer.render_view(target, &view, time);
}

/// Loads the game's moving shapes, sprites and the scene's explosion/particle
/// assets into a new renderer.
pub fn preload(renderer: &mut Renderer, scene: &Scene, device: &wgpu::Device, queue: &wgpu::Queue, assets: &(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)) {
    renderer.preload(device, queue, &assets.0, &assets.1, Some(&scene.fx));
}

/// One game frame: clear, the 3D game view over the whole target (while
/// the game screen is up: `game`, the renderer with its scene and the view),
/// then the 2D overlay.
pub fn draw_game_frame(
    target: Target,
    game: Option<(&mut Renderer, &Scene, &mut GameView)>,
    fx: Option<&FxState>,
    gui: Option<crate::gui_gpu::GuiFrame>,
    gui_renderer: &mut crate::gui_gpu::GuiRenderer,
    time: f32,
    perf: &mut crate::frontend::Perf,
) {
    let Target { device, queue, view, size } = target;
    clear(device, queue, view);
    if let Some((r, scene, gv)) = game {
        perf.time("render 3d", || render_game_view_taking(r, scene, target, gv, fx, time));
    }
    if let Some(f) = gui {
        perf.time("render gui", || gui_renderer.render(device, queue, view, size, &f));
    }
}

/// Renders one frame offscreen (the game view, the scene from its initial
/// camera, or a cleared canvas) and saves it as PNG.
pub fn screenshot(out: &Path, size: (u32, u32), scene: Option<&Scene>, game: Option<&GameView>, time: f32) -> Result<(), String> {
    let instance = crate::gfx::instance();
    let (_, device, queue) = pollster::block_on(gpu(None, &instance))?;
    let (w, h) = size;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = render_target(&device, "screenshot", (w, h), format);
    let view = target.create_view(&Default::default());
    match scene {
        Some(s) => {
            let mut renderer = Renderer::new(&device, &queue, format, s);
            match game {
                Some(gv) => render_game_view(&mut renderer, s, Target { device: &device, queue: &queue, view: &view, size }, gv, None, time),
                None => renderer.render(Target { device: &device, queue: &queue, view: &view, size }, &initial_camera(s), time),
            }
        }
        None => clear(&device, &queue, &view),
    }
    let pixels = read_rgba(&device, &queue, &target, size, format);
    image::save_buffer(out, &pixels, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

/// PNG of a captured frame (RGBA8 rows; alpha is dropped, the frame is opaque).
pub fn encode_png(rgba: &[u8], size: (u32, u32)) -> Result<Vec<u8>, String> {
    let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let img = image::RgbImage::from_raw(size.0, size.1, rgb).ok_or("bad frame size")?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// An offscreen render target with its own device (screenshots, `--bench`).
pub struct Offscreen {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    /// GPU adapter description.
    pub adapter: String,
}

impl Offscreen {
    pub fn new(size: (u32, u32)) -> Result<Offscreen, String> {
        let instance = crate::gfx::instance();
        let (adapter, device, queue) = pollster::block_on(gpu(None, &instance))?;
        let info = adapter.get_info();
        let adapter = format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = render_target(&device, "offscreen", size, format);
        let view = texture.create_view(&Default::default());
        Ok(Offscreen { device, queue, texture, view, size, format, adapter })
    }

    /// The target to draw a frame into.
    pub fn target(&self) -> Target<'_> {
        Target { device: &self.device, queue: &self.queue, view: &self.view, size: self.size }
    }

    /// Waits for the GPU to finish the submitted work.
    pub fn wait(&self) {
        self.device.poll(wgpu::Maintain::Wait);
    }

    pub fn save(&self, out: &Path) -> Result<(), String> {
        let pixels = read_rgba(&self.device, &self.queue, &self.texture, self.size, self.format);
        image::save_buffer(out, &pixels, self.size.0, self.size.1, image::ColorType::Rgba8).map_err(|e| e.to_string())
    }
}

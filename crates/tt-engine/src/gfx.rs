//! GPU helpers: adapter/device setup, frame drawing (3D scene or a cleared
//! canvas), offscreen screenshots.

use glam::Vec3;
use std::path::Path;
use tt_render::{Camera, Renderer, Scene};

/// Background when no 3D view is shown (menus).
pub const CLEAR: wgpu::Color = wgpu::Color { r: 0.02, g: 0.02, b: 0.03, a: 1.0 };

pub async fn gpu(compatible: Option<&wgpu::Surface<'_>>, instance: &wgpu::Instance) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), String> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: compatible, ..Default::default() })
        .await
        .ok_or("no GPU adapter")?;
    // Optional features: GPU pass timing (`TT_GPU_TIMING`) and the packed
    // HDR scene format of the graphics extras.
    let features = adapter.features() & (tt_render::gputime::wanted_features() | tt_render::post::OPTIONAL_FEATURES);
    let desc = wgpu::DeviceDescriptor { required_features: features, ..Default::default() };
    let (device, queue) = adapter.request_device(&desc, None).await.map_err(|e| e.to_string())?;
    Ok((adapter, device, queue))
}

/// Camera above the first spawn point looking at the map centre (as tt-viewer).
pub fn initial_camera(scene: &Scene) -> Camera {
    let spawn = scene.spawn.unwrap_or(Vec3::new(0.0, -150.0, 100.0));
    let pos = spawn + Vec3::new(0.0, 0.0, 40.0);
    let to_centre = Vec3::new(-pos.x, -pos.y, 0.0);
    Camera { pos, yaw: to_centre.x.atan2(to_centre.y), pitch: -0.3 }
}

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

/// Renders one frame offscreen (scene, or a cleared canvas) and saves it as PNG.
/// Draws over the frame after the 3D pass (device, queue, view, size, format).
pub type Overlay<'a> = &'a mut dyn FnMut(&wgpu::Device, &wgpu::Queue, &wgpu::TextureView, (u32, u32), wgpu::TextureFormat);

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
    /// None (the Reload arch).
    #[default]
    None,
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
#[allow(clippy::too_many_arguments)]
pub fn render_game_view(renderer: &mut Renderer, scene: &Scene, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView, size: (u32, u32), gv: &GameView, fx: Option<&FxState>, time: f32) {
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
    render_game_view_taking(renderer, scene, device, queue, target, size, &mut owned, fx, time);
}

/// [`render_game_view`] that moves the instances out of `gv` (they are
/// handed to the renderer as they are, lit, without copies): `gv.dynamic`,
/// `ifl`, `triggers` and the triggers' IFL times are left empty.
#[allow(clippy::too_many_arguments)]
pub fn render_game_view_taking(renderer: &mut Renderer, scene: &Scene, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView, size: (u32, u32), gv: &mut GameView, fx: Option<&FxState>, time: f32) {
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
    let view = tt_render::View { camera, fov_deg: tt_render::vertical_fov(gv.hfov, aspect), viewport: Some([0, 0, w, h]), clear: true };
    renderer.render_view(device, queue, target, w, h, &view, time);
}

/// Loads the game's moving shapes, sprites and the scene's explosion/particle
/// assets into a new renderer.
pub fn preload(renderer: &mut Renderer, scene: &Scene, device: &wgpu::Device, queue: &wgpu::Queue, assets: &(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)) {
    renderer.preload(device, queue, &assets.0, &assets.1, Some(&scene.fx));
}

/// One game frame: clear, the 3D game view over the whole target (while
/// the game screen is up: `game_view`), then the 2D overlay.
#[allow(clippy::too_many_arguments)]
pub fn draw_game_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::TextureView,
    size: (u32, u32),
    renderer: Option<(&mut Renderer, &Scene)>,
    game_view: Option<&mut GameView>,
    fx: Option<&FxState>,
    gui: Option<crate::gui_gpu::GuiFrame>,
    gui_renderer: &mut crate::gui_gpu::GuiRenderer,
    time: f32,
    perf: &mut crate::frontend::Perf,
) {
    clear(device, queue, target);
    if let (Some((r, scene)), Some(gv)) = (renderer, game_view) {
        perf.time("render 3d", || render_game_view_taking(r, scene, device, queue, target, size, gv, fx, time));
    }
    if let Some(f) = gui {
        perf.time("render gui", || gui_renderer.render(device, queue, target, size, &f));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn screenshot(out: &Path, size: (u32, u32), scene: Option<&Scene>, camera: Option<Camera>, game: Option<&GameView>, time: f32, overlay: Option<Overlay>) -> Result<(), String> {
    let instance = wgpu::Instance::default();
    let (_, device, queue) = pollster::block_on(gpu(None, &instance))?;
    let (w, h) = size;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    match scene {
        Some(s) => {
            let mut renderer = Renderer::new(&device, &queue, format, s);
            match game {
                Some(gv) => render_game_view(&mut renderer, s, &device, &queue, &view, (w, h), gv, None, time),
                None => {
                    let cam = camera.unwrap_or_else(|| initial_camera(s));
                    renderer.render(&device, &queue, &view, w, h, &cam, time);
                }
            }
        }
        None => clear(&device, &queue, &view),
    }
    if let Some(o) = overlay {
        o(&device, &queue, &view, size, format);
    }
    let pixels = read_rgba(&device, &queue, &target, size, format);
    image::save_buffer(out, &pixels, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

/// Reads a whole texture (`COPY_SRC`, 8-bit RGBA or BGRA) back as tightly
/// packed RGBA8 rows, top row first.
pub fn read_rgba(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, size: (u32, u32), format: wgpu::TextureFormat) -> Vec<u8> {
    let (w, h) = size;
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let data = buf.slice(..).get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    if matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
        for p in pixels.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
        }
    }
    pixels
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
        let instance = wgpu::Instance::default();
        let (adapter, device, queue) = pollster::block_on(gpu(None, &instance))?;
        let info = adapter.get_info();
        let adapter = format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Ok(Offscreen { device, queue, texture, view, size, format, adapter })
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

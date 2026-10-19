//! wgpu renderer for a [`Scene`]; draws into any colour target view.

use crate::scene::{ColorVertex, Scene, SkyVertex, TerrainVertex};
use crate::pose::Rig;
use crate::shapes::{Blend, InstanceData, ShapeBatch, ShapeSet, ShapeVertex, IFL_TICKS_PER_SECOND};
use glam::{Mat4, Vec2, Vec3};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use wgpu::util::DeviceExt;

/// Depth/stencil format of the renderer's private depth buffer (the stencil
/// keeps projected shadows from darkening a pixel twice).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    fog_color: [f32; 4],
    params: [f32; 4],
    // x: seconds, y: IFL ticks per second
    time: [f32; 4],
    // x: terrain grid origin (world, both axes), y: metres per square, z: samples per edge
    terrain: [f32; 4],
    // Height fog bands: x visible distance (0 = unused), y min height, z max height.
    fog_volumes: [[f32; 4]; 3],
    // Camera basis for billboards (world space).
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    cam_forward: [f32; 4],
    // Graphics extras: x normal-map strength, y glass, z emissive boost, w sun glow.
    extras: [f32; 4],
    // Dynamic lights (graphics extra): x count.
    light_info: [f32; 4],
    // Per light: xyz position, w radius; rgb colour (linear, × intensity).
    lights: [[f32; 4]; 2 * MAX_POINT_LIGHTS],
}

/// Most dynamic point lights a view is lit by (the nearest are kept).
pub const MAX_POINT_LIGHTS: usize = 16;

/// A short-lived coloured point light (explosions, shots, projectiles; the
/// "Dynamic lights" graphics extra). Lights terrain and shapes within
/// `radius` with a smooth falloff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    pub pos: Vec3,
    pub radius: f32,
    /// Linear RGB times intensity (1 = a white light as bright as full sun
    /// at the centre).
    pub color: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    /// Radians; 0 looks along +Y (north), increasing turns towards +X.
    pub yaw: f32,
    pub pitch: f32,
}

impl Camera {
    /// Roll is not represented and is dropped (the view stays upright).
    pub fn from_matrix(m: Mat4) -> Camera {
        let pos = m.w_axis.truncate();
        let f = m.y_axis.truncate().try_normalize().unwrap_or(Vec3::Y);
        Camera { pos, yaw: f.x.atan2(f.y), pitch: f.z.clamp(-1.0, 1.0).asin() }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.yaw.cos() * self.pitch.cos(), self.pitch.sin())
    }

    /// `fov_deg`: vertical field of view.
    pub fn view_proj_fov(&self, aspect: f32, far: f32, fov_deg: f32) -> Mat4 {
        self.view_proj_near(aspect, 0.5, far, fov_deg)
    }

    /// [`Camera::view_proj_fov`] with an explicit near plane.
    pub fn view_proj_near(&self, aspect: f32, near: f32, far: f32, fov_deg: f32) -> Mat4 {
        let view = Mat4::look_to_rh(self.pos, self.forward(), Vec3::Z);
        Mat4::perspective_rh(fov_deg.clamp(1.0, 179.0).to_radians(), aspect, near.max(1e-3), far) * view
    }
}

/// - the light direction is hard-coded to (0.57, 0.57, -0.57), not the sun;
/// - the caster is the shape's `shadow-99` detail, else `Collision-1`;
/// - receivers are collected in a box of side 2·radius across the light and
///   `10 · shape radius` along it;
/// - it is drawn with `glBlendFunc(ZERO, ONE_MINUS_SRC_COLOR)` (darken).
///   The darkness comes from per-vertex fade colours we have not decoded, so
///   the opacity here is a chosen constant.
pub(crate) fn shadow_light_dir() -> Vec3 {
    Vec3::new(0.57, 0.57, -0.57).normalize()
}
const SHADOW_LENGTH_FACTOR: f32 = 10.0;
/// Opacity of a projected shadow (see [`shadow_light_dir`]).
const SHADOW_OPACITY: f32 = 0.5;
/// Height above the ground at which a shadow has faded out.
const SHADOW_FADE_HEIGHT: f32 = 10.0;

/// HDR extra: emissive surfaces (self-illuminated, additive) and sprites
/// are this much brighter, so they reach the bloom threshold.
const HDR_EMISSIVE_BOOST: f32 = 1.8;

/// Vertical field of view used by [`Renderer::render`].
pub const DEFAULT_FOV_DEG: f32 = 60.0;

pub fn vertical_fov(horizontal_deg: f32, aspect: f32) -> f32 {
    (2.0 * ((horizontal_deg.to_radians() * 0.5).tan() / aspect.max(1e-3)).atan()).to_degrees()
}

/// A moving shape drawn on top of the static scene (tank, projectile,
/// power-up, flag...). It is not part of the baked lightmap.
#[derive(Clone, Debug)]
pub struct DynamicInstance {
    /// `.dts` path, looked up case-insensitively like the statics.
    pub shape: PathBuf,
    pub transform: Mat4,
    /// Sun diffuse factor `k`, normally [`Scene::shape_light`] at the object.
    pub light: f32,
    /// Local transform overrides by node name (case-insensitive), replacing
    /// the node's default transform relative to its parent; children follow.
    /// See [`crate::pose`] for sequence poses and node defaults.
    pub nodes: Vec<(String, Mat4)>,
    /// Objects to hide / to show although hidden by default (by object name,
    /// case-insensitive; `hidden` wins).
    pub hidden: Vec<String>,
    pub shown: Vec<String>,
}

impl Default for DynamicInstance {
    fn default() -> DynamicInstance {
        DynamicInstance { shape: PathBuf::new(), transform: Mat4::IDENTITY, light: 1.0, nodes: Vec::new(), hidden: Vec::new(), shown: Vec::new() }
    }
}

/// Quality/performance settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSettings {
    /// The 3D view is rendered at this fraction of the viewport size and
    /// scaled up (bilinear), 0.25..=1.
    pub render_scale: f32,
    /// Anisotropic filtering of terrain, sky and shape textures (1 = off,
    /// up to 16). Fixed at creation.
    pub anisotropy: u16,
    /// The detail-level prefs the game's Detail setting
    /// (`setThinkTanksDetailLevel`) sets; see [`Detail`].
    pub detail: Detail,
    /// Graphics extras; default all off.
    pub extras: crate::post::Extras,
}

impl Default for RenderSettings {
    fn default() -> RenderSettings {
        RenderSettings { render_scale: 1.0, anisotropy: 16, detail: Detail::default(), extras: Default::default() }
    }
}

/// What the game's detail prefs change in the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detail {
    /// Scales the projected pixel size that picks a shape's detail level
    /// ([`select_detail`]).
    pub ts_detail_adjust: f32,
    /// The highest detail levels skipped.
    pub skip_render_dls: u32,
    /// Dynamic (tank) shadows are drawn only at 0.2 and above.
    pub shadow_detail: f32,
    /// Mip levels dropped from shape textures. Fixed at creation.
    pub mip_reduction: u32,
    /// The same for the sky. Fixed at creation.
    pub sky_mip_reduction: u32,
    /// Mip levels dropped from the terrain textures. Fixed at creation.
    pub terrain_tex_detail: u32,
}

impl Default for Detail {
    fn default() -> Detail {
        Detail { ts_detail_adjust: 1.0, skip_render_dls: 0, shadow_detail: 1.0, mip_reduction: 0, sky_mip_reduction: 0, terrain_tex_detail: 0 }
    }
}

impl Detail {
    /// The settings fixed when the renderer is created (texture sampling).
    pub fn textures(&self) -> (u32, u32, u32) {
        (self.mip_reduction, self.sky_mip_reduction, self.terrain_tex_detail)
    }

    /// Draws only if `0.2 <= ShadowDetail`.
    pub fn shadows(&self) -> bool {
        self.shadow_detail >= 0.2
    }
}

pub fn detail_pixel_scale(viewport: (u32, u32), vfov_deg: f32) -> f32 {
    let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
    (h * 0.5) / (vfov_deg.to_radians() * 0.5).tan().max(1e-4) * (w / 640.0)
}

/// Detail-level table of one shape for [`select_detail`].
#[derive(Clone, Debug, Default)]
pub struct DetailTable {
    /// `TSShape::radius`.
    pub radius: f32,
    /// `size` of each detail level (negative: collision/shadow details).
    pub sizes: Vec<f32>,
    pub smallest_visible_size: f32,
    pub smallest_visible_dl: i32,
}

impl DetailTable {
    pub fn of(shape: &tt_assets::dts::Shape) -> DetailTable {
        DetailTable {
            radius: shape.radius,
            sizes: shape.detail_levels.iter().map(|d| d.size).collect(),
            smallest_visible_size: shape.smallest_visible_size as f32,
            smallest_visible_dl: shape.smallest_visible_dl,
        }
    }

    /// Detail levels that can be selected: 0 up to the smallest visible one.
    pub fn visible_levels(&self) -> usize {
        if self.sizes.is_empty() {
            return 0;
        }
        let last = self.sizes.iter().position(|&s| s < 0.0).unwrap_or(self.sizes.len());
        ((self.smallest_visible_dl.max(0) as usize) + 1).min(last.max(1))
    }
}

/// [`select_detail`] for an object at `model` seen from `cam`: the distance
/// to the object's origin divided by its largest scale.
fn instance_detail(t: &DetailTable, model: Mat4, cam: Vec3, pixel_scale: f32, d: &Detail) -> usize {
    let scale = model.x_axis.truncate().length().max(model.y_axis.truncate().length()).max(model.z_axis.truncate().length()).max(1e-3);
    let dist = model.w_axis.truncate().distance(cam) / scale;
    select_detail(t, dist, pixel_scale, d.ts_detail_adjust, d.skip_render_dls)
}

/// - pixel radius = radius / dist · `pixel_scale` · `detailAdjust`; below
///   the smallest visible size it is raised to just above it, so a shape
///   never disappears through this path (its smallest level may be empty);
/// - the first level `i` with `size[i] < pixel radius`, or whose next level
///   is a collision/shadow level (negative size), or the last;
/// - levels below `min(skipRenderDLs, smallestVisibleDL)` are skipped.
pub fn select_detail(t: &DetailTable, dist: f32, pixel_scale: f32, detail_adjust: f32, skip_render_dls: u32) -> usize {
    let n = t.sizes.len();
    if n == 0 {
        return 0;
    }
    let mut pixel = t.radius / dist.max(1e-3) * pixel_scale * detail_adjust;
    if pixel <= t.smallest_visible_size {
        pixel = t.smallest_visible_size + 0.01;
    }
    let mut dl = n - 1;
    for i in 0..n {
        if t.sizes[i] < pixel || i + 1 >= n || t.sizes[i + 1] < 0.0 {
            dl = i;
            break;
        }
    }
    let skip = (skip_render_dls as i32).min(t.smallest_visible_dl).max(0) as usize;
    dl.max(skip).min(t.visible_levels().saturating_sub(1))
}

/// Extra per-instance settings for [`Renderer::set_dynamic_with`].
#[derive(Clone, Debug, Default)]
pub struct InstanceOptions {
    /// Cast the dynamic shadow onto the terrain.
    pub shadow: bool,
    /// IFL material times in seconds (material name, with or without
    /// `.ifl`, case-insensitive). Listed IFLs show the frame at that time;
    /// the others run on the global clock. Think Tanks drives the tread
    /// IFLs `LTread`/`Rtread` through the `LTread`/`RTread` sequences, see
    /// [`crate::pose::sequence_ifl`].
    pub ifl: Vec<(String, f32)>,
}

/// How to draw one frame.
pub struct View {
    pub camera: Camera,
    /// Vertical field of view in degrees (see [`vertical_fov`]).
    pub fov_deg: f32,
    /// Sub-rectangle `[x, y, w, h]` of the target in pixels; None = whole target.
    /// Pixels outside it are left untouched.
    pub viewport: Option<[u32; 4]>,
    /// Fill the viewport with the fog colour first; false keeps the existing
    /// colour (depth is always cleared).
    pub clear: bool,
}

/// Pipelines and layout shared by static and dynamic shapes.
struct ShapePipelines {
    opaque: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Normal maps: trilinear, no anisotropy.
    normal_sampler: wgpu::Sampler,
    /// Load `*.normal.png` for the texture layers (normal-map extra).
    normals: bool,
}

/// Edge length of the normal-map array layers (that of every layer,
/// [`crate::textures::size`]: their mips come from the same cache).
const NORMAL_SIZE: u32 = 512;

/// Normal maps for texture files (`X.png` -> `X.normal.png`, flat where
/// missing), `NORMAL_SIZE`² RGBA8 layers (at least two), shared from the
/// cache ([`crate::textures::normal_layer`]).
fn normal_layers(files: &[Option<PathBuf>]) -> Vec<std::sync::Arc<Vec<u8>>> {
    let flat = crate::textures::flat_normal();
    let mut maps: Vec<_> = crate::textures::parallel_map(files, |f| f.as_ref().and_then(|f| crate::textures::normal_layer(f, NORMAL_SIZE)).unwrap_or_else(|| flat.clone()));
    while maps.len() < 2 {
        maps.push(flat.clone());
    }
    maps
}

/// A 1x1 two-layer flat normal map (normal maps off).
fn flat_normals(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    texture(device, queue, 1, 2, wgpu::TextureFormat::Rgba8Unorm, &[128, 128, 255, 255, 128, 128, 255, 255])
}

/// A [`ShapeSet`] on the GPU (geometry, texture layers, IFL timeline).
struct ShapeMesh {
    group: wgpu::BindGroup,
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    batches: Vec<ShapeBatch>,
    /// Batch indices per shape (`lods` index) and detail level.
    by_level: Vec<Vec<Vec<usize>>>,
    lods: Vec<crate::shapes::LodGroup>,
    /// Dynamic-shadow geometry and shape radius (posed shapes).
    shadow: std::ops::Range<u32>,
    radius: f32,
    center: Vec3,
    bounds_radius: f32,
}

fn pose_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, buf: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("poses"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
    })
}

impl ShapeMesh {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, p: &ShapePipelines, set: &ShapeSet) -> ShapeMesh {
        let mut layers = set.layers.clone();
        if layers.len() < 2 {
            layers.push(layers[0].clone());
        }
        let tex = srgb_layers_texture(device, queue, &layers);
        let init = |data: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("shapes"), contents: data, usage });
        let timeline = init(bytemuck::cast_slice(&set.timeline), wgpu::BufferUsages::STORAGE);
        let normals = if p.normals {
            layers_texture(device, queue, &normal_layers(&set.layer_files), wgpu::TextureFormat::Rgba8Unorm)
        } else {
            flat_normals(device, queue)
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shapes"),
            layout: &p.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&tex) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&p.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: timeline.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&normals) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&p.normal_sampler) },
            ],
        });
        ShapeMesh {
            group,
            vb: init(bytemuck::cast_slice(&set.vertices), wgpu::BufferUsages::VERTEX),
            ib: init(bytemuck::cast_slice(&set.indices), wgpu::BufferUsages::INDEX),
            batches: set.batches.clone(),
            by_level: {
                let mut v: Vec<Vec<Vec<usize>>> = set.lods.iter().map(|g| vec![Vec::new(); g.table.visible_levels().max(1)]).collect();
                for (i, b) in set.batches.iter().enumerate() {
                    if let Some(l) = v.get_mut(b.group as usize).and_then(|g| g.get_mut(b.detail as usize)) {
                        l.push(i);
                    }
                }
                v
            },
            lods: set.lods.clone(),
            shadow: set.shadow_indices.clone(),
            radius: set.radius,
            center: Vec3::from(set.center),
            bounds_radius: set.bounds_radius,
        }
    }
}

/// One set of moving shapes with its instance and pose buffers, reused
/// (grown as needed) across frames.
#[derive(Default)]
struct Layer {
    instances: Option<wgpu::Buffer>,
    capacity: usize,
    /// Per-object pose matrices of all instances, and their bind group.
    poses: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    pose_capacity: usize,
    /// Mesh key and instance range per draw group.
    draws: Vec<(PathBuf, std::ops::Range<u32>)>,
    /// Some instance casts a projected shadow.
    shadows: bool,
    /// World position per instance (for sorting blended surfaces).
    positions: Vec<Vec3>,
    /// Object-to-world transform per instance (detail selection).
    transforms: Vec<Mat4>,
    /// Shadow casters of this layer: (instance index, caster record).
    casters: Vec<(u32, ShadowCaster)>,
    /// Terrain index ranges under each caster's box: (caster, indices).
    shadow_draws: Vec<(u32, std::ops::Range<u32>)>,
}

/// One dynamic shadow for the terrain shader (std430 layout, 64 bytes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowCaster {
    /// xyz: shape centre (world), w: radius of the shadow box.
    center: [f32; 4],
    /// xyz: light direction, w: shadow length.
    dir: [f32; 4],
    /// xyz: first axis across the light, w: opacity.
    u: [f32; 4],
    /// xyz: second axis across the light, w: atlas tile.
    v: [f32; 4],
}

/// Shadow atlas: `ATLAS_TILES`² tiles of `atlas_tile` texels.
fn atlas_tile(shadows: u8) -> u32 {
    match shadows {
        0 => 64,
        1 => 128,
        _ => 256,
    }
}

/// Light direction of the improved shadows: towards the ground from the
/// sun, at least ~25° down (a low sun would stretch them across the map).
fn sun_shadow_dir(sun_dir: [f32; 4]) -> Vec3 {
    let d = -Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]).try_normalize().unwrap_or(Vec3::Z);
    let flat = Vec2::new(d.x, d.y);
    if d.z > -0.42 {
        let h = flat.try_normalize().unwrap_or(Vec2::X) * 0.9;
        Vec3::new(h.x, h.y, -0.42).normalize()
    } else {
        d
    }
}
/// Opacity of the improved shadows (soft edges make them read lighter).
const SOFT_SHADOW_OPACITY: f32 = 0.6;
const ATLAS_TILES: u32 = 8;
const MAX_CASTERS: usize = (ATLAS_TILES * ATLAS_TILES) as usize;

/// Moving shapes: meshes loaded on first use and cached by path; layer 0 is
/// [`Renderer::set_dynamic`], layer 1 the shapes of [`Renderer::set_effects`].
#[derive(Default)]
struct Dynamic {
    /// Shape path as given -> mesh (None: failed to load; not retried).
    meshes: HashMap<PathBuf, Option<(ShapeMesh, Rig)>>,
    layers: [Layer; 2],
}

/// Terrain heights for fitting shadow planes.
struct Ground {
    terrain: tt_assets::terrain::Terrain,
    origin: f32,
    square: f32,
}

impl Ground {
    fn height(&self, x: f32, y: f32) -> f32 {
        crate::lighting::height_at(&self.terrain, (x - self.origin) / self.square, (y - self.origin) / self.square)
    }
}

pub struct Renderer {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    terrain_pipeline: wgpu::RenderPipeline,
    /// The terrain's depth alone (drawn before the other opaque surfaces).
    terrain_depth_pipeline: wgpu::RenderPipeline,
    color_pipeline: wgpu::RenderPipeline,
    terrain_vb: wgpu::Buffer,
    terrain_ib: wgpu::Buffer,
    terrain_index_count: u32,
    markers_vb: wgpu::Buffer,
    sky: Option<(wgpu::RenderPipeline, wgpu::BindGroup, wgpu::Buffer, u32)>,
    clear_pipeline: wgpu::RenderPipeline,
    shape_pipelines: ShapePipelines,
    pose_layout: wgpu::BindGroupLayout,
    /// Bound for draws without a dynamic pose buffer (static shapes).
    empty_poses: wgpu::BindGroup,
    /// Static shapes and their instance buffer.
    shapes: Option<(ShapeMesh, wgpu::Buffer)>,
    /// The static instances in load order (the GPU buffer holds them
    /// sorted by the current view's detail levels).
    static_instances: Vec<InstanceData>,
    /// Static draws of the current view: (batch, instance range).
    static_draws: Vec<(usize, std::ops::Range<u32>)>,
    dynamic: Dynamic,
    atlas_pipeline: wgpu::RenderPipeline,
    terrain_shadow_pipeline: wgpu::RenderPipeline,
    /// Terrain mesh grid: first cell (grid units) and cells per row.
    terrain_grid: (i64, i64, i64),
    atlas_group: wgpu::BindGroup,
    atlas_view: wgpu::TextureView,
    casters: wgpu::Buffer,
    ground: Ground,
    sprites: crate::sprites::SpriteGpu,
    /// Sprites of [`Renderer::set_sprites`] and of the effects.
    sprite_layers: [Vec<crate::sprites::Sprite>; 2],
    marker_count: u32,
    /// Draw sky, terrain, markers and static shapes (false for object views).
    world: bool,
    near: f32,
    /// Depth buffers by size (the scaled view and full-size views).
    /// (attachment view, width, height, depth-aspect view for sampling).
    depth: Vec<(wgpu::TextureView, u32, u32, wgpu::TextureView)>,
    settings: RenderSettings,
    format: wgpu::TextureFormat,
    /// Post path of the graphics extras (None: draw straight into the target).
    post: Option<crate::post::Post>,
    /// Dynamic lights of the next views ([`Renderer::set_point_lights`]).
    point_lights: Vec<PointLight>,
    /// `TT_GPU_TIMING` pass timer.
    timer: std::cell::RefCell<Option<crate::gputime::GpuTimer>>,
    behind_layout: wgpu::BindGroupLayout,
    behind_sampler: wgpu::Sampler,
    /// Group 3 when there is no glass copy.
    behind_dummy: wgpu::BindGroup,
    blit: (wgpu::RenderPipeline, wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler),
    /// Offscreen colour target of the scaled view and its blit bind group.
    scaled: Option<(wgpu::TextureView, u32, u32, wgpu::BindGroup)>,
    clear: wgpu::Color,
    far: f32,
    scene_params: [f32; 4],
    terrain_params: [f32; 4],
    fog_volumes: [[f32; 4]; 3],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    fog_color: [f32; 4],
}

/// Box-filtered mip chain of a square RGBA8 image (`size` must be a power of two).
/// sRGB data is averaged in linear space so distant textures don't darken.
pub(crate) fn mip_chain(layer: &[u8], size: u32, srgb: bool) -> Vec<u8> {
    let (to_lin, to_byte) = mip_tables(srgb);
    let mut out = Vec::with_capacity(layer.len() * 4 / 3 + 4);
    out.extend_from_slice(layer);
    let mut cur: Vec<f32> = layer.chunks(4).flat_map(|p| [to_lin[p[0] as usize], to_lin[p[1] as usize], to_lin[p[2] as usize], p[3] as f32 / 255.0]).collect();
    let mut s = size as usize;
    while s > 1 {
        let h = s / 2;
        let mut next = vec![0.0f32; h * h * 4];
        for y in 0..h {
            for x in 0..h {
                for c in 0..4 {
                    let at = |xx: usize, yy: usize| cur[(yy * s + xx) * 4 + c];
                    next[(y * h + x) * 4 + c] = (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1)) / 4.0;
                }
            }
        }
        out.extend(next.chunks(4).flat_map(|p| [to_byte.byte(p[0]), to_byte.byte(p[1]), to_byte.byte(p[2]), (p[3] * 255.0).round() as u8]));
        cur = next;
        s = h;
    }
    out
}

/// Colour channel conversions of [`mip_chain`]: byte to (linear) value by
/// table, and value to byte by the exact thresholds of
/// `round(v^(1/2.2) * 255)` (or `round(v * 255)`), clamped.
fn mip_tables(srgb: bool) -> (&'static [f32; 256], &'static ByteThresholds) {
    static T: OnceLock<[([f32; 256], ByteThresholds); 2]> = OnceLock::new();
    let t = T.get_or_init(|| {
        [false, true].map(|srgb| {
            let lin = std::array::from_fn(|c| if srgb { (c as f32 / 255.0).powf(2.2) } else { c as f32 / 255.0 });
            (lin, ByteThresholds::new(move |v: f32| ((if srgb { v.powf(1.0 / 2.2) } else { v }) * 255.0).round().clamp(0.0, 255.0) as u8))
        })
    });
    let (lin, th) = &t[srgb as usize];
    (lin, th)
}

/// For a monotonic `f: f32 -> u8` on [0, 1]: `first[k]` is the smallest
/// value with `f(v) >= k`, so `f(v)` is the count of thresholds <= `v`.
struct ByteThresholds {
    first: [f32; 256],
}

impl ByteThresholds {
    fn new(f: impl Fn(f32) -> u8 + Copy) -> ByteThresholds {
        let mut first = [f32::INFINITY; 256];
        first[0] = f32::NEG_INFINITY;
        for (k, slot) in first.iter_mut().enumerate().skip(1) {
            // Bisection over the bit patterns of non-negative floats (their
            // order is the numeric order) in [0, 1].
            let (mut lo, mut hi) = (0u32, 1.0f32.to_bits() + 1);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if f(f32::from_bits(mid)) as usize >= k {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            if lo <= 1.0f32.to_bits() {
                *slot = f32::from_bits(lo);
            }
        }
        ByteThresholds { first }
    }

    #[inline]
    fn byte(&self, v: f32) -> u8 {
        if !(0.0..=1.0).contains(&v) {
            return self.slow(v);
        }
        // Thresholds <= v (first[0] always counts).
        (self.first.partition_point(|&t| t <= v) - 1) as u8
    }

    #[cold]
    fn slow(&self, v: f32) -> u8 {
        if v.is_nan() || v <= 0.0 {
            0
        } else {
            255
        }
    }
}

/// [`texture`] of sRGB [`crate::textures::size`] layers, their mip levels
/// from the cache ([`crate::textures::srgb_mips`]).
/// The vertex buffer of the terrain pipelines.
const TERRAIN_BUFFERS: [wgpu::VertexBufferLayout<'static>; 1] = [wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<TerrainVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &TERRAIN_ATTRIBUTES,
}];
const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

/// The vertex and instance buffers of the shape pipelines (the scene's
/// and the shadow atlas').
const SHAPE_BUFFERS: [wgpu::VertexBufferLayout<'static>; 2] = [
    wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<ShapeVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SHAPE_VERTEX_ATTRIBUTES },
    wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<InstanceData>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &SHAPE_INSTANCE_ATTRIBUTES },
];
const SHAPE_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 9] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Uint32, 4 => Uint32, 5 => Uint32, 11 => Uint32, 14 => Uint8x4, 15 => Unorm8x4];
const SHAPE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] =
    wgpu::vertex_attr_array![6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 12 => Uint32x4, 13 => Float32x4];

/// A trilinear, repeating sampler (`anisotropy` 1 = none) that skips the
/// top `skip_mips` mip levels (at most 8).
fn repeat_sampler(device: &wgpu::Device, anisotropy: u16, skip_mips: u32) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: anisotropy,
        lod_min_clamp: skip_mips.min(8) as f32,
        ..Default::default()
    })
}

pub(crate) fn srgb_layers_texture(device: &wgpu::Device, queue: &wgpu::Queue, layers: &[std::sync::Arc<Vec<u8>>]) -> wgpu::TextureView {
    layers_texture(device, queue, layers, wgpu::TextureFormat::Rgba8UnormSrgb)
}

/// [`texture`] of [`crate::textures::size`] layers in `format` (sRGB or
/// linear RGBA8), their mip levels from the cache ([`crate::textures::mips`]).
pub(crate) fn layers_texture(device: &wgpu::Device, queue: &wgpu::Queue, layers: &[std::sync::Arc<Vec<u8>>], format: wgpu::TextureFormat) -> wgpu::TextureView {
    let size = crate::textures::size();
    let mips = crate::textures::parallel_map(layers, |l| crate::textures::mips(l, format.is_srgb()));
    let mut data = Vec::with_capacity(layers.iter().zip(&mips).map(|(l, m)| l.len() + m.len()).sum());
    for (l, m) in layers.iter().zip(&mips) {
        data.extend_from_slice(l);
        data.extend_from_slice(m);
    }
    upload(device, queue, size, layers.len() as u32, size.ilog2() + 1, format, &data)
}

pub(crate) fn texture(device: &wgpu::Device, queue: &wgpu::Queue, size: u32, layers: u32, format: wgpu::TextureFormat, data: &[u8]) -> wgpu::TextureView {
    let mips = if size.is_power_of_two() { size.ilog2() + 1 } else { 1 };
    let layer_bytes = (size * size * 4) as usize;
    let data: Vec<u8> = if mips > 1 {
        let layers: Vec<&[u8]> = data.chunks(layer_bytes).collect();
        crate::textures::parallel_map(&layers, |l| mip_chain(l, size, format.is_srgb())).concat()
    } else {
        data.to_vec()
    };
    upload(device, queue, size, layers, mips, format, &data)
}

/// A texture (array) from layer-major data with all `mips` levels.
fn upload(device: &wgpu::Device, queue: &wgpu::Queue, size: u32, layers: u32, mips: u32, format: wgpu::TextureFormat, data: &[u8]) -> wgpu::TextureView {
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        data,
    );
    tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(if layers > 1 { wgpu::TextureViewDimension::D2Array } else { wgpu::TextureViewDimension::D2 }),
        ..Default::default()
    })
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, scene: &Scene) -> Renderer {
        Renderer::new_with_settings(device, queue, format, scene, RenderSettings::default())
    }

    /// [`Renderer::new`] with explicit [`RenderSettings`].
    pub fn new_with_settings(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, scene: &Scene, settings: RenderSettings) -> Renderer {
        let _all = crate::loadstats::span(|| "renderer: total".into());
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        // The graphics extras draw the scene into an HDR texture first.
        let scene_format = if settings.extras.post() { crate::post::scene_format(device) } else { format };
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sp = crate::loadstats::span(|| "renderer: terrain textures + mips".into());
        let n = scene.terrain_size;
        let alpha_lo = texture(device, queue, n, 1, wgpu::TextureFormat::Rgba8Unorm, &scene.alpha[0]);
        let alpha_hi = texture(device, queue, n, 1, wgpu::TextureFormat::Rgba8Unorm, &scene.alpha[1]);
        let materials = texture(
            device,
            queue,
            crate::textures::size(),
            scene.materials.len() as u32,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &scene.materials.concat(),
        );
        // sRGB so the display-space factor q/32 modulates the texture as the
        // engine's 8-bit blend does.
        let lightmap = texture(device, queue, scene.lightmap_size, 1, wgpu::TextureFormat::Rgba8UnormSrgb, &scene.lightmap);
        // Linear format: the detail map is a multiplier around 0.5, not a colour.
        let detail_size = if scene.detail.is_some() { crate::textures::size() } else { 2 };
        let flat_detail = vec![128u8; (detail_size * detail_size * 4) as usize];
        let detail = texture(device, queue, detail_size, 1, wgpu::TextureFormat::Rgba8Unorm, scene.detail.as_deref().unwrap_or(&flat_detail));
        let terrain_normals = if settings.extras.normal_maps {
            layers_texture(device, queue, &normal_layers(&scene.material_files), wgpu::TextureFormat::Rgba8Unorm)
        } else {
            flat_normals(device, queue)
        };
        drop(sp);
        let anisotropy = settings.anisotropy.clamp(1, 16);
        let sampler = repeat_sampler(device, anisotropy, settings.detail.terrain_tex_detail);
        // Terrain normal maps: trilinear only (see shader.wgsl).
        let normal_sampler = repeat_sampler(device, 1, settings.detail.terrain_tex_detail);
        let shape_sampler = repeat_sampler(device, anisotropy, settings.detail.mip_reduction);

        // Dynamic shadows: caster records and their silhouette atlas.
        let casters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow casters"),
            size: (MAX_CASTERS * std::mem::size_of::<ShadowCaster>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow atlas"),
            size: wgpu::Extent3d { width: atlas_tile(settings.extras.shadows) * ATLAS_TILES, height: atlas_tile(settings.extras.shadows) * ATLAS_TILES, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let tex_entry = |binding, dim| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(1, wgpu::TextureViewDimension::D2),
                tex_entry(2, wgpu::TextureViewDimension::D2),
                tex_entry(3, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(5, wgpu::TextureViewDimension::D2),
                tex_entry(6, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(8, wgpu::TextureViewDimension::D2),
                tex_entry(9, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&alpha_lo) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&alpha_hi) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&materials) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&lightmap) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&detail) },
                wgpu::BindGroupEntry { binding: 7, resource: casters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&terrain_normals) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Sampler(&normal_sampler) },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        // (depth written, depth test, colour written)
        let pipeline_ext = |vs: &str, fs: &str, buffers: &[wgpu::VertexBufferLayout], depth_write: bool, depth_compare: wgpu::CompareFunction, color: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: scene_format,
                        blend: None,
                        write_mask: if color { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() },
                    })],
                }),
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: depth_write,
                    depth_compare,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let pipeline = |vs: &str, fs: &str, buffers: &[wgpu::VertexBufferLayout]| pipeline_ext(vs, fs, buffers, true, wgpu::CompareFunction::Less, true);
        // The terrain is drawn twice: its depth first (before everything
        // else opaque), then shaded where it is the nearest surface (the
        // same invariant positions; depth test LessEqual, no depth write),
        // so each visible terrain pixel runs the costly material shader
        // exactly once, and nothing hidden behind hills or shapes does.
        let terrain_depth_pipeline = pipeline_ext("vs_terrain", "fs_depth_only", &TERRAIN_BUFFERS, true, wgpu::CompareFunction::Less, false);
        let terrain_pipeline = pipeline_ext("vs_terrain", "fs_terrain", &TERRAIN_BUFFERS, false, wgpu::CompareFunction::LessEqual, true);
        let terrain_shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain shadows"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_terrain_shadow"),
                compilation_options: Default::default(),
                buffers: &TERRAIN_BUFFERS,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_terrain_shadow"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let color_pipeline = pipeline(
            "vs_color",
            "fs_color",
            &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ColorVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
            }],
        );

        let sky = (!scene.sky_faces.is_empty()).then(|| {
            let faces = texture(device, queue, crate::textures::size(), scene.sky_faces.len() as u32, wgpu::TextureFormat::Rgba8UnormSrgb, &scene.sky_faces.concat());
            // Clamped faces (their shared edges are matched on load,
            // `crate::sky`); trilinear and anisotropic so the mip level
            // does not jump where the view crosses from one face to the
            // next (the top face is seen at a grazing angle).
            let clamp = device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                anisotropy_clamp: settings.anisotropy.clamp(1, 16),
                lod_min_clamp: settings.detail.sky_mip_reduction.min(8) as f32,
                ..Default::default()
            });
            let sky_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sky"),
                entries: &[
                    tex_entry(0, wgpu::TextureViewDimension::D2Array),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sky"),
                layout: &sky_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&faces) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&clamp) },
                ],
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sky"),
                bind_group_layouts: &[&layout, &sky_layout],
                push_constant_ranges: &[],
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sky"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_sky"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<SkyVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Uint32],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_sky"),
                    compilation_options: Default::default(),
                    targets: &[Some(scene_format.into())],
                }),
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });
            let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sky"),
                contents: bytemuck::cast_slice(&scene.sky_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (pipeline, group, vb, scene.sky_vertices.len() as u32)
        });

        let shape_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shapes"),
            entries: &[
                tex_entry(0, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(3, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        // Group 3 of the shape pipelines: the opaque scene behind the glass.
        let behind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass behind"),
            entries: &[
                tex_entry(0, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let behind_sampler = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let behind_dummy = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass behind (none)"),
            layout: &behind_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&texture(device, queue, 1, 1, wgpu::TextureFormat::Rgba8Unorm, &[0, 0, 0, 255])) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&behind_sampler) },
            ],
        });
        let pose_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("poses"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let empty_poses = pose_group(device, &pose_layout, &device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("no poses"),
            contents: bytemuck::cast_slice(&[Mat4::IDENTITY.to_cols_array()]),
            usage: wgpu::BufferUsages::STORAGE,
        }));
        let shape_pipelines = {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shapes"),
                bind_group_layouts: &[&layout, &shape_layout, &pose_layout, &behind_layout],
                push_constant_ranges: &[],
            });
            let make_full = |vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth_stencil: wgpu::DepthStencilState| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(fs),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(vs),
                        compilation_options: Default::default(),
                        buffers: &SHAPE_BUFFERS,
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fs),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState { format: scene_format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    }),
                    // DTS front faces are clockwise; leaves are double-sided, so draw both.
                    primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                    depth_stencil: Some(depth_stencil),
                    multisample: Default::default(),
                    multiview: None,
                    cache: None,
                })
            };
            let make = |fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool| {
                make_full(
                    "vs_shape",
                    fs,
                    blend,
                    wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: depth_write,
                        depth_compare: wgpu::CompareFunction::Less,
                        stencil: Default::default(),
                        bias: Default::default(),
                    },
                )
            };
            // Shadow silhouettes into the atlas (Shadow::prepare renders the
            // shadow detail into a bitmap seen along the light).
            let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow atlas"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
            let atlas_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow atlas"),
                layout: &atlas_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: casters.as_entire_binding() },
                ],
            });
            let atlas_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow atlas"),
                bind_group_layouts: &[&atlas_layout, &shape_layout, &pose_layout],
                push_constant_ranges: &[],
            });
            let atlas_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow atlas"),
                layout: Some(&atlas_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_shadow_atlas"),
                    compilation_options: Default::default(),
                    buffers: &SHAPE_BUFFERS,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_shadow_atlas"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::TextureFormat::R8Unorm.into())],
                }),
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });
            let additive = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            };
            ((atlas_pipeline, atlas_group), ShapePipelines {
                opaque: make("fs_shape", None, true),
                translucent: make("fs_shape_blend", Some(wgpu::BlendState::ALPHA_BLENDING), false),
                additive: make("fs_shape_add", Some(additive), false),
                layout: shape_layout,
                sampler: shape_sampler,
                normal_sampler: repeat_sampler(device, 1, settings.detail.mip_reduction),
                normals: settings.extras.normal_maps,
            })
        };
        let ((atlas_pipeline, atlas_group), shape_pipelines) = shape_pipelines;
        let sprites = crate::sprites::SpriteGpu::new(device, &shader, &layout, scene_format, DEPTH_FORMAT);
        let shapes = (!scene.shapes.batches.is_empty()).then(|| {
            let _sp = crate::loadstats::span(|| format!("renderer: static shapes upload ({} layers)", scene.shapes.layers.len()));
            let mesh = ShapeMesh::new(device, queue, &shape_pipelines, &scene.shapes);
            let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("static instances"),
                contents: bytemuck::cast_slice(&scene.shapes.instances),
                // Rewritten per view, sorted by detail level.
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            (mesh, instances)
        });
        let clear_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport clear"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_clear"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_clear"),
                compilation_options: Default::default(),
                targets: &[Some(scene_format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let buffer = |data: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: data, usage });
        let markers: &[ColorVertex] = if scene.markers.is_empty() {
            &[ColorVertex { pos: [0.0; 3], color: [0.0; 3] }]
        } else {
            &scene.markers
        };
        let f = scene.fog_color;
        // The fog colour is specified in display (sRGB) space.
        let lin = |c: f32| c.clamp(0.0, 1.0).powf(2.2) as f64;
        Renderer {
            globals,
            bind_group,
            terrain_pipeline,
            terrain_depth_pipeline,
            color_pipeline,
            terrain_vb: buffer(bytemuck::cast_slice(&scene.terrain_vertices), wgpu::BufferUsages::VERTEX),
            terrain_ib: buffer(bytemuck::cast_slice(&scene.terrain_indices), wgpu::BufferUsages::INDEX),
            terrain_index_count: scene.terrain_indices.len() as u32,
            markers_vb: buffer(bytemuck::cast_slice(markers), wgpu::BufferUsages::VERTEX),
            sky,
            clear_pipeline,
            shape_pipelines,
            pose_layout,
            empty_poses,
            shapes,
            static_instances: scene.shapes.instances.clone(),
            static_draws: Vec::new(),
            dynamic: Dynamic::default(),
            atlas_pipeline,
            terrain_shadow_pipeline,
            terrain_grid: {
                let n = scene.terrain_vertices.len();
                let per_row = (n as f64).sqrt().round() as i64;
                let first = scene.terrain_vertices.first().map_or([0.0; 2], |v| v.grid);
                (first[0] as i64, first[1] as i64, (per_row - 1).max(0))
            },
            atlas_group,
            atlas_view,
            casters,
            ground: Ground { terrain: scene.terrain.clone(), origin: scene.terrain_origin, square: scene.square_size },
            sprites,
            sprite_layers: [Vec::new(), Vec::new()],
            marker_count: scene.markers.len() as u32,
            world: true,
            near: 0.5,
            depth: Vec::new(),
            settings,
            format,
            post: settings.extras.post().then(|| crate::post::Post::new(device, queue, format, settings.extras)),
            timer: std::cell::RefCell::new(crate::gputime::GpuTimer::new(device, queue)),
            point_lights: Vec::with_capacity(MAX_POINT_LIGHTS),
            behind_layout,
            behind_sampler,
            behind_dummy,
            blit: {
                let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("blit"),
                    entries: &[
                        tex_entry(0, wgpu::TextureViewDimension::D2),
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
                let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("blit"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
                let make = |blend: Option<wgpu::BlendState>| {
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("blit"),
                        layout: Some(&pl),
                        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_blit"), compilation_options: Default::default(), buffers: &[] },
                        fragment: Some(wgpu::FragmentState {
                            module: &shader,
                            entry_point: Some("fs_blit"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                        }),
                        primitive: Default::default(),
                        depth_stencil: None,
                        multisample: Default::default(),
                        multiview: None,
                        cache: None,
                    })
                };
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                });
                (make(None), make(Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)), layout, sampler)
            },
            scaled: None,
            clear: wgpu::Color { r: lin(f.x), g: lin(f.y), b: lin(f.z), a: 1.0 },
            far: scene.visible_distance.max(100.0) * 1.2,
            scene_params: [scene.fog_distance, scene.visible_distance, scene.terrain_size as f32, 32.0],
            terrain_params: [scene.terrain_origin, scene.square_size, scene.terrain_size as f32, 0.0],
            fog_volumes: {
                let mut v = [[0.0; 4]; 3];
                for (slot, f) in v.iter_mut().zip(&scene.fog_volumes) {
                    *slot = [f[0], f[1], f[2], 0.0];
                }
                v
            },
            sun_dir: scene.sun_dir.extend(0.0).into(),
            sun_color: scene.sun_color.into(),
            ambient: scene.ambient.into(),
            fog_color: [lin(f.x) as f32, lin(f.y) as f32, lin(f.z) as f32, 1.0],
        }
    }

    /// A renderer that draws only dynamic shapes and sprites (no world),
    /// for [`crate::objectview::ObjectView`].
    pub(crate) fn objects_only(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Renderer {
        let mut r = Renderer::new(device, queue, format, &Scene::empty());
        r.world = false;
        r
    }

    /// Lighting, background (linear RGBA), near/far planes of an object view.
    pub(crate) fn set_view_params(&mut self, to_light: Vec3, color: [f32; 3], ambient: [f32; 3], background: [f32; 4], near: f32, far: f32) {
        self.sun_dir = to_light.normalize_or(Vec3::Z).extend(0.0).into();
        self.sun_color = [color[0], color[1], color[2], 1.0];
        self.ambient = [ambient[0], ambient[1], ambient[2], 1.0];
        self.fog_color = background;
        self.clear = wgpu::Color { r: background[0] as f64, g: background[1] as f64, b: background[2] as f64, a: background[3] as f64 };
        self.near = near;
        self.far = far;
    }

    pub fn settings(&self) -> RenderSettings {
        self.settings
    }

    /// Renders the 3D view at `scale` (0.25..=1) of the viewport size and
    /// scales it up; 1 renders at full resolution.
    pub fn set_render_scale(&mut self, scale: f32) {
        self.settings.render_scale = if scale.is_finite() { scale.clamp(0.25, 1.0) } else { 1.0 };
    }

    /// Changes the detail settings for the following frames. The texture
    /// ones ([`Detail::textures`]) only take effect in a new renderer.
    pub fn set_detail(&mut self, detail: Detail) {
        self.settings.detail = detail;
    }

    /// Bloom strength of the HDR extra (0..1); the other extras are fixed
    /// at creation.
    pub fn set_bloom(&mut self, bloom: f32) {
        self.settings.extras.bloom = bloom;
        if let Some(p) = self.post.as_mut() {
            p.extras.bloom = bloom;
        }
    }

    /// The dynamic point lights of the following views (at most
    /// [`MAX_POINT_LIGHTS`]; the caller picks the nearest). Empty = none.
    pub fn set_point_lights(&mut self, lights: &[PointLight]) {
        self.point_lights.clear();
        self.point_lights.extend(lights.iter().take(MAX_POINT_LIGHTS).copied());
    }

    /// AO strength (0..2); applies at once.
    pub fn set_ao_strength(&mut self, strength: f32) {
        self.settings.extras.ao_strength = strength;
        if let Some(p) = self.post.as_mut() {
            p.extras.ao_strength = strength;
        }
    }

    /// Normal-map strength ("Surface detail", 0..2); applies at once.
    pub fn set_normal_strength(&mut self, strength: f32) {
        self.settings.extras.normal_strength = strength;
    }

    /// Replaces the set of moving shapes drawn by the following frames.
    /// Shapes load on first use and stay cached by path.
    pub fn set_dynamic(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, instances: &[DynamicInstance]) {
        self.set_layer(device, queue, 0, instances, &[]);
    }

    /// [`Renderer::set_dynamic`] with a projected shadow on the terrain for
    /// every instance whose entry in `shadows` is true (missing = false). The
    /// shadow is the shape's posed silhouette projected along the sun onto
    /// the ground plane under it; it fades with height above the ground and
    /// with the instance's `light` (no shadow inside terrain shadow).
    pub fn set_dynamic_shadowed(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, instances: &[DynamicInstance], shadows: &[bool]) {
        let options: Vec<InstanceOptions> = shadows.iter().map(|&shadow| InstanceOptions { shadow, ..Default::default() }).collect();
        self.set_layer(device, queue, 0, instances, &options);
    }

    /// [`Renderer::set_dynamic`] with per-instance [`InstanceOptions`]
    /// (`options[i]` belongs to `instances[i]`; missing = default).
    pub fn set_dynamic_with(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, instances: &[DynamicInstance], options: &[InstanceOptions]) {
        self.set_layer(device, queue, 0, instances, options);
    }

    /// Camera-facing sprites drawn by the following frames (e.g. `.png`
    /// projectiles: `Sprite { texture, pos, size: sprite_scale, ..Default::default() }`).
    pub fn set_sprites(&mut self, sprites: &[crate::sprites::Sprite]) {
        self.sprite_layers[0] = sprites.to_vec();
    }

    /// Takes over the current state of `fx` (particles, explosion sprites and
    /// shapes) for the following frames. Call after [`crate::fx::Effects::advance`].
    pub fn set_effects(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, fx: &crate::fx::Effects) {
        self.sprite_layers[1] = fx.sprites();
        let (shapes, options): (Vec<DynamicInstance>, Vec<InstanceOptions>) =
            fx.shapes_with_ifl().into_iter().map(|(inst, ifl)| (inst, InstanceOptions { shadow: false, ifl })).unzip();
        self.set_layer(device, queue, 1, &shapes, &options);
    }

    /// Takes over the moving shapes and sprite textures that `old` (a
    /// renderer of the same device) has on the GPU, when they were made with
    /// the same texture settings, so a new mission's renderer does not load
    /// and upload them again (bind group layouts of equal descriptors are
    /// one and the same in wgpu, so the old bind groups fit the new
    /// pipelines).
    pub fn reuse_assets(&mut self, old: Renderer) {
        let (a, b) = (&self.settings, &old.settings);
        if self.format != old.format || a.anisotropy != b.anisotropy || a.detail.textures() != b.detail.textures() || a.extras.fixed() != b.extras.fixed() {
            return;
        }
        let Renderer { dynamic, sprites, .. } = old;
        for (path, mesh) in dynamic.meshes {
            self.dynamic.meshes.entry(path).or_insert(mesh);
        }
        self.sprites.reuse_textures(sprites);
    }

    /// Loads moving shapes and sprite textures ahead of use (nothing is
    /// drawn), so the first frame that shows them does not stall. Also
    /// loads the shapes and sprites of every explosion in `fx`.
    pub fn preload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, shapes: &[PathBuf], sprite_textures: &[PathBuf], fx: Option<&crate::fx::FxLibrary>) {
        let mut shapes: Vec<PathBuf> = shapes.to_vec();
        let mut textures: Vec<PathBuf> = sprite_textures.to_vec();
        if let Some(lib) = fx {
            for e in &lib.explosions {
                match &e.resource {
                    crate::fx::ExplosionResource::Shape(p) => shapes.push(p.clone()),
                    crate::fx::ExplosionResource::Sprite(p) => textures.push(p.clone()),
                    _ => {}
                }
            }
            textures.extend(lib.particles.iter().map(|p| p.texture.clone()));
        }
        let sp = crate::loadstats::span(|| format!("preload: {} shapes", shapes.len()));
        // Shapes parsed and their textures decoded on all cores, then uploaded.
        shapes.sort();
        shapes.dedup();
        shapes.retain(|p| !self.dynamic.meshes.contains_key(p));
        let normals = self.shape_pipelines.normals;
        let loaded = crate::textures::parallel_map(&shapes, |p| {
            let l = ShapeSet::load_posable(p);
            // Their mip levels and normal maps too, while on all cores.
            for px in l.iter().flat_map(|(set, _)| &set.layers) {
                crate::textures::srgb_mips(px);
            }
            if normals {
                for f in l.iter().flat_map(|(set, _)| &set.layer_files).flatten() {
                    if let Some(n) = crate::textures::normal_layer(f, NORMAL_SIZE) {
                        crate::textures::mips(&n, false);
                    }
                }
            }
            l
        });
        drop(sp);
        let sp = crate::loadstats::span(|| "preload: shape uploads".into());
        for (p, l) in shapes.into_iter().zip(loaded) {
            let mesh = l.map(|(set, shape)| (ShapeMesh::new(device, queue, &self.shape_pipelines, &set), Rig::new(&shape)));
            self.dynamic.meshes.insert(p, mesh);
        }
        drop(sp);
        let _sp = crate::loadstats::span(|| format!("preload: {} sprite textures", textures.len()));
        // Those the GPU has already (a renderer's reused textures) are not decoded again.
        textures.retain(|t| !self.sprites.has_texture(t));
        let jobs: Vec<crate::textures::Job> = textures
            .iter()
            .filter_map(|t| t.parent().zip(t.file_name().and_then(|n| n.to_str())))
            .filter_map(|(dir, name)| crate::textures::image_file(dir, name))
            .collect();
        crate::textures::prefetch(&jobs);
        for t in textures {
            self.sprites.texture(device, queue, &t);
        }
        crate::loadstats::note(|| {
            let (l, n, m) = crate::textures::cache_bytes();
            format!("texture caches: layers {} MB, normal maps {} MB, mips {} MB", l >> 20, n >> 20, m >> 20)
        });
    }

    fn set_layer(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layer: usize, instances: &[DynamicInstance], options: &[InstanceOptions]) {
        let none = InstanceOptions::default();
        let d = &mut self.dynamic;
        // Group by shape, keeping first-seen order.
        let mut order: Vec<&PathBuf> = Vec::new();
        let mut groups: HashMap<&PathBuf, Vec<(&DynamicInstance, &InstanceOptions)>> = HashMap::new();
        for (i, inst) in instances.iter().enumerate() {
            let mesh = d.meshes.entry(inst.shape.clone()).or_insert_with(|| {
                let (set, shape) = ShapeSet::load_posable(&inst.shape)?;
                Some((ShapeMesh::new(device, queue, &self.shape_pipelines, &set), Rig::new(&shape)))
            });
            if mesh.is_none() {
                continue;
            }
            groups
                .entry(&inst.shape)
                .or_insert_with(|| {
                    order.push(&inst.shape);
                    Vec::new()
                })
                .push((inst, options.get(i).unwrap_or(&none)));
        }
        let l = &mut d.layers[layer];
        let mut data = Vec::with_capacity(instances.len());
        let mut poses: Vec<[f32; 16]> = Vec::new();
        l.draws.clear();
        l.shadows = false;
        l.positions.clear();
        l.transforms.clear();
        l.casters.clear();
        l.shadow_draws.clear();
        for path in order {
            let Some(Some((mesh, rig))) = d.meshes.get(path) else { continue };
            let start = data.len() as u32;
            for &(inst, opts) in &groups[path] {
                let shadow = opts.shadow && self.settings.detail.shadows();
                let base = poses.len() as u32;
                if inst.nodes.is_empty() && inst.hidden.is_empty() && inst.shown.is_empty() && opts.ifl.is_empty() {
                    poses.extend(rig.default_objects.iter().map(|m| m.to_cols_array()));
                } else {
                    poses.extend(rig.instance_matrices(&inst.nodes, &inst.hidden, &inst.shown, &opts.ifl).iter().map(|m| m.to_cols_array()));
                }
                let node_base = base + rig.object_count() as u32;
                let ifl_base = node_base + rig.node_count() as u32;
                if shadow && !mesh.shadow.is_empty() && layer == 0 && l.casters.len() < MAX_CASTERS {
                    // Shadow::prepare: box around the shape centre, 2r across
                    // the (fixed) light direction and 10·radius along it.
                    let c = inst.transform.transform_point3(mesh.center);
                    let scale = inst.transform.x_axis.truncate().length().max(1e-3);
                    let r = mesh.bounds_radius * scale;
                    let len = SHADOW_LENGTH_FACTOR * mesh.radius * scale;
                    let improved = self.settings.extras.shadows > 0;
                    let d = if improved { sun_shadow_dir(self.sun_dir) } else { shadow_light_dir() };
                    let u = if d.z.abs() <= 0.9 { d.cross(Vec3::Z) } else { d.cross(Vec3::X) }.normalize();
                    let v = u.cross(d);
                    let ground = self.ground.height(c.x, c.y);
                    let opacity = if improved { SOFT_SHADOW_OPACITY } else { SHADOW_OPACITY } * (1.0 - (c.z - ground).max(0.0) / SHADOW_FADE_HEIGHT).clamp(0.0, 1.0) * (inst.light * 2.0).clamp(0.0, 1.0);
                    if opacity > 0.0 {
                        // Terrain cells under the part of the box that can
                        // reach the ground (down to 4 m below the lowest
                        // terrain around), as rows of the terrain mesh.
                        let low = [(-r, 0.0), (r, 0.0), (0.0, -r), (0.0, r), (0.0, 0.0)]
                            .iter()
                            .map(|(dx, dy)| self.ground.height(c.x + dx, c.y + dy))
                            .fold(f32::MAX, f32::min);
                        let reach = ((c.z - low + 4.0 + r) / -d.z).clamp(0.0, len);
                        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
                        for along in [-r, reach] {
                            for (a, b) in [(-r, -r), (-r, r), (r, -r), (r, r)] {
                                let p = (c + d * along + u * a + v * b).truncate();
                                lo = lo.min(p);
                                hi = hi.max(p);
                            }
                        }
                        let (g0x, g0y, cells) = self.terrain_grid;
                        let cell = |w: f32| ((w - self.ground.origin) / self.ground.square).floor() as i64;
                        let (i0, i1) = ((cell(lo.x) - g0x).max(0), (cell(hi.x) - g0x).min(cells - 1));
                        let (j0, j1) = ((cell(lo.y) - g0y).max(0), (cell(hi.y) - g0y).min(cells - 1));
                        let caster = l.casters.len() as u32;
                        if i0 <= i1 {
                            for j in j0..=j1 {
                                l.shadow_draws.push((caster, ((j * cells + i0) * 6) as u32..((j * cells + i1 + 1) * 6) as u32));
                            }
                        }
                        let tile = l.casters.len() as f32;
                        l.casters.push((
                            data.len() as u32,
                            ShadowCaster { center: [c.x, c.y, c.z, r], dir: [d.x, d.y, d.z, len], u: [u.x, u.y, u.z, opacity], v: [v.x, v.y, v.z, tile] },
                        ));
                        l.shadows = true;
                    }
                }
                l.positions.push(inst.transform.w_axis.truncate());
                l.transforms.push(inst.transform);
                data.push(InstanceData {
                    model: inst.transform.to_cols_array_2d(),
                    light: [inst.light, 0.0, 0.0, 0.0],
                    pose: [base, l.casters.len().saturating_sub(1) as u32, node_base, ifl_base],
                    shadow: [0.0; 4],
                });
            }
            l.draws.push((path.clone(), start..data.len() as u32));
        }
        if !l.casters.is_empty() {
            let records: Vec<ShadowCaster> = l.casters.iter().map(|c| c.1).collect();
            queue.write_buffer(&self.casters, 0, bytemuck::cast_slice(&records));
        }
        if data.is_empty() {
            return;
        }
        if l.instances.is_none() || l.capacity < data.len() {
            l.capacity = data.len().next_power_of_two().max(64);
            l.instances = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dynamic instances"),
                size: (l.capacity * std::mem::size_of::<InstanceData>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        queue.write_buffer(l.instances.as_ref().unwrap(), 0, bytemuck::cast_slice(&data));
        if poses.is_empty() {
            poses.push(Mat4::IDENTITY.to_cols_array());
        }
        if l.poses.is_none() || l.pose_capacity < poses.len() {
            l.pose_capacity = poses.len().next_power_of_two().max(1024);
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dynamic poses"),
                size: (l.pose_capacity * 64) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = pose_group(device, &self.pose_layout, &buf);
            l.poses = Some((buf, group));
        }
        queue.write_buffer(&l.poses.as_ref().unwrap().0, 0, bytemuck::cast_slice(&poses));
    }

    /// Picks every static shape's detail level for a view from `cam`
    /// ([`select_detail`]) and rewrites the static instance buffer grouped by
    /// (shape, level), recording the draws in `static_draws`.
    fn select_static_details(&mut self, queue: &wgpu::Queue, cam: Vec3, pixel_scale: f32) {
        self.static_draws.clear();
        let Some((mesh, buffer)) = self.shapes.as_ref() else { return };
        let d = self.settings.detail;
        let mut sorted = Vec::with_capacity(self.static_instances.len());
        for (gi, g) in mesh.lods.iter().enumerate() {
            let levels = mesh.by_level[gi].len();
            let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); levels];
            for i in g.instances.clone() {
                let Some(inst) = self.static_instances.get(i as usize) else { continue };
                let m = Mat4::from_cols_array_2d(&inst.model);
                let level = instance_detail(&g.table, m, cam, pixel_scale, &d).min(levels - 1);
                buckets[level].push(i as usize);
            }
            for (level, list) in buckets.into_iter().enumerate() {
                if list.is_empty() {
                    continue;
                }
                let start = sorted.len() as u32;
                sorted.extend(list.iter().map(|&i| self.static_instances[i]));
                let range = start..sorted.len() as u32;
                self.static_draws.extend(mesh.by_level[gi][level].iter().map(|&b| (b, range.clone())));
            }
        }
        if !sorted.is_empty() {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&sorted));
        }
    }

    /// Screen rectangle `[x, y, w, h]` (of a `size` view) that covers every
    /// visible glass surface, for the glass copy; `None` without glass in
    /// view. Static glass (or glass around the camera) takes the whole view.
    fn glass_rect(&self, view_proj: Mat4, size: (u32, u32)) -> Option<[u32; 4]> {
        let full = Some([0, 0, size.0, size.1]);
        if let Some((mesh, _)) = self.shapes.as_ref().filter(|_| self.world) {
            if self.static_draws.iter().any(|(b, _)| mesh.batches[*b].glass) {
                return full;
            }
        }
        let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
        for l in self.dynamic.layers.iter().filter(|l| l.instances.is_some()) {
            for (path, range) in &l.draws {
                let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                if !mesh.batches.iter().any(|b| b.glass) {
                    continue;
                }
                for i in range.clone() {
                    let Some(m) = l.transforms.get(i as usize) else { return full };
                    let c = m.transform_point3(mesh.center);
                    let r = mesh.bounds_radius * m.x_axis.truncate().length().max(m.z_axis.truncate().length()) * 1.25 + 0.1;
                    for k in 0..8 {
                        let corner = c + Vec3::new(if k & 1 == 0 { -r } else { r }, if k & 2 == 0 { -r } else { r }, if k & 4 == 0 { -r } else { r });
                        let q = view_proj * corner.extend(1.0);
                        if q.w <= 1e-3 {
                            // Behind or around the camera: no usable bounds.
                            return full;
                        }
                        let ndc = glam::Vec2::new(q.x, q.y) / q.w;
                        lo = lo.min(ndc);
                        hi = hi.max(ndc);
                    }
                }
            }
        }
        if lo.x > hi.x || hi.x < -1.0 || lo.x > 1.0 || hi.y < -1.0 || lo.y > 1.0 {
            return None;
        }
        // NDC to pixels (y down), a few pixels of margin for the refraction.
        let (w, h) = (size.0 as f32, size.1 as f32);
        let x0 = ((lo.x.max(-1.0) * 0.5 + 0.5) * w - 4.0).floor().max(0.0) as u32;
        let x1 = ((hi.x.min(1.0) * 0.5 + 0.5) * w + 4.0).ceil().min(w) as u32;
        let y0 = ((0.5 - hi.y.min(1.0) * 0.5) * h - 4.0).floor().max(0.0) as u32;
        let y1 = ((0.5 - lo.y.max(-1.0) * 0.5) * h + 4.0).ceil().min(h) as u32;
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }

    /// Detail level of a moving shape's instance for the current view.
    fn dynamic_detail(&self, mesh: &ShapeMesh, l: &Layer, i: u32, cam: Vec3, pixel_scale: f32) -> u32 {
        let (Some(g), Some(m)) = (mesh.lods.first(), l.transforms.get(i as usize)) else { return 0 };
        instance_detail(&g.table, *m, cam, pixel_scale, &self.settings.detail) as u32
    }

    /// Draws the whole target with a 60° camera, clearing it first.
    /// `time`: seconds since the scene started (drives texture animation).
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView, width: u32, height: u32, camera: &Camera, time: f32) {
        let view = View { camera: *camera, fov_deg: DEFAULT_FOV_DEG, viewport: None, clear: true };
        self.render_view(device, queue, target, width, height, &view, time);
    }

    /// Draws one view into `target` (`width`×`height` pixels).
    #[allow(clippy::too_many_arguments)]
    pub fn render_view(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView, width: u32, height: u32, view: &View, time: f32) {
        if width == 0 || height == 0 {
            return;
        }
        let [vx, vy, vw, vh] = match view.viewport {
            Some([x, y, w, h]) => {
                let (x, y) = (x.min(width), y.min(height));
                [x, y, w.min(width - x), h.min(height - y)]
            }
            None => [0, 0, width, height],
        };
        if vw == 0 || vh == 0 {
            return;
        }
        let scale = self.settings.render_scale;
        let post = self.post.is_some();
        // The post path always draws into its own texture (of the scaled size).
        let scaled = scale < 0.999 && !post;
        let (sw, sh) = if scale < 0.999 { (((vw as f32 * scale).round() as u32).max(1), ((vh as f32 * scale).round() as u32).max(1)) } else { (vw, vh) };
        if let Some(p) = self.post.as_mut() {
            p.ensure(device, (sw, sh), DEPTH_FORMAT, (&self.behind_layout, &self.behind_sampler));
        }
        // Where the scene pass draws: the target's viewport, or a whole
        // offscreen texture of the scaled size.
        let (pw, ph, [px, py, pvw, pvh]) = if post {
            (sw, sh, [0, 0, sw, sh])
        } else if scaled {
            if self.scaled.as_ref().is_none_or(|s| (s.1, s.2) != (sw, sh)) {
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("scaled view"),
                    size: wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let tv = tex.create_view(&Default::default());
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("blit"),
                    layout: &self.blit.2,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&tv) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.blit.3) },
                    ],
                });
                self.scaled = Some((tv, sw, sh, group));
            }
            (sw, sh, [0, 0, sw, sh])
        } else {
            (width, height, [vx, vy, vw, vh])
        };
        let full = post || scaled || [vx, vy, vw, vh] == [0, 0, width, height];
        let depth_index = if post {
            0
        } else {
            match self.depth.iter().position(|d| (d.1, d.2) == (pw, ph)) {
                Some(i) => i,
                None => {
                    let tex = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("depth"),
                        size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: DEPTH_FORMAT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    });
                    if self.depth.len() >= 3 {
                        self.depth.remove(0);
                    }
                    let sample = tex.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
                    self.depth.push((tex.create_view(&Default::default()), pw, ph, sample));
                    self.depth.len() - 1
                }
            }
        };
        let camera = &view.camera;
        let pixel_scale = detail_pixel_scale((vw, vh), view.fov_deg);
        if self.world {
            self.select_static_details(queue, camera.pos, pixel_scale);
        }
        let forward = camera.forward();
        let right = forward.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(forward).normalize();
        let e = self.settings.extras;
        let g = Globals {
            view_proj: camera.view_proj_near(vw as f32 / vh as f32, self.near, self.far, view.fov_deg).to_cols_array_2d(),
            camera: camera.pos.extend(1.0).into(),
            sun_dir: self.sun_dir,
            sun_color: self.sun_color,
            ambient: self.ambient,
            fog_color: self.fog_color,
            params: self.scene_params,
            time: [time, IFL_TICKS_PER_SECOND, self.dynamic.layers[0].casters.len() as f32, self.near.max(1e-3)],
            terrain: self.terrain_params,
            fog_volumes: self.fog_volumes,
            cam_right: right.extend(0.0).into(),
            cam_up: up.extend(0.0).into(),
            cam_forward: forward.extend(0.0).into(),
            extras: [
                if e.normal_maps { e.normal_strength.clamp(0.0, 2.0) } else { 0.0 },
                if e.glass && post { 1.0 } else { 0.0 },
                if e.hdr && post { HDR_EMISSIVE_BOOST } else { 1.0 },
                if e.hdr && post { 1.0 } else { 0.0 },
            ],
            light_info: [self.point_lights.len() as f32, atlas_tile(e.shadows) as f32, e.shadows.min(2) as f32, self.far],
            lights: {
                let mut l = [[0.0; 4]; 2 * MAX_POINT_LIGHTS];
                for (k, p) in self.point_lights.iter().enumerate() {
                    l[2 * k] = [p.pos.x, p.pos.y, p.pos.z, p.radius.max(0.1)];
                    l[2 * k + 1] = [p.color[0], p.color[1], p.color[2], 0.0];
                }
                l
            },
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        let sprites = self.sprite_layers.iter().flatten();
        self.sprites.prepare(device, queue, sprites, camera.pos, forward);

        let load = if view.clear && full {
            wgpu::LoadOp::Clear(self.clear)
        } else if scaled || post {
            // Kept background: render over transparency, blend in the blit.
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
        } else {
            wgpu::LoadOp::Load
        };
        let mut enc = device.create_command_encoder(&Default::default());
        if let Some(t) = self.timer.get_mut().as_mut() {
            t.begin(device, &mut enc);
        }
        let mark = |enc: &mut wgpu::CommandEncoder, label: &'static str| {
            if let Some(t) = self.timer.borrow_mut().as_mut() {
                t.mark(enc, label);
            }
        };
        let l0 = &self.dynamic.layers[0];
        if let (false, Some(l0_instances)) = (l0.casters.is_empty(), &l0.instances) {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow atlas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.atlas_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.atlas_pipeline);
            pass.set_bind_group(0, &self.atlas_group, &[]);
            pass.set_bind_group(2, l0.poses.as_ref().map_or(&self.empty_poses, |p| &p.1), &[]);
            pass.set_vertex_buffer(1, l0_instances.slice(..));
            for (tile, (inst, _)) in l0.casters.iter().enumerate() {
                let Some((path, _)) = l0.draws.iter().find(|(_, r)| r.contains(inst)) else { continue };
                let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                let (tx, ty) = (tile as u32 % ATLAS_TILES, tile as u32 / ATLAS_TILES);
                let at = atlas_tile(self.settings.extras.shadows);
                pass.set_viewport((tx * at) as f32, (ty * at) as f32, at as f32, at as f32, 0.0, 1.0);
                pass.set_bind_group(1, &mesh.group, &[]);
                pass.set_vertex_buffer(0, mesh.vb.slice(..));
                pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
                if self.settings.extras.shadows > 0 {
                    // The real silhouette: everything but glows of the
                    // most detailed level (turret, barrel, treads, jar).
                    for b in mesh.batches.iter().filter(|b| b.detail == 0 && b.blend != Blend::Additive) {
                        pass.draw_indexed(b.indices.clone(), 0, *inst..*inst + 1);
                    }
                } else {
                    pass.draw_indexed(mesh.shadow.clone(), 0, *inst..*inst + 1);
                }
            }
        }
        mark(&mut enc, "shadows");
        let (color_view, depth_view, depth_sample) = match self.post.as_ref().and_then(|p| p.targets.as_ref()) {
            Some(t) => (&t.color, &t.depth, &t.depth_sample),
            None => (if scaled { &self.scaled.as_ref().unwrap().0 } else { target }, &self.depth[depth_index].0, &self.depth[depth_index].3),
        };
        // Glass needs the opaque scene in a texture: the blended surfaces
        // then go into a second pass.
        // Only when some glass is in view, and only its screen rectangle is
        // copied.
        let glass = if post && e.glass { self.glass_rect(Mat4::from_cols_array_2d(&g.view_proj), (pw, ph)) } else { None };
        // Soft sprites sample the opaque scene's depth: then the blended
        // surfaces go into a second pass with the depth read-only.
        let soft = self.sprites.len() > 0;
        if soft {
            self.sprites.set_scene_depth(device, depth_sample);
        }
        let split = glass.is_some() || soft;
        let begin = |enc: &mut wgpu::CommandEncoder, first: bool| -> wgpu::RenderPass<'static> {
            let (cl, dl, sl) = if first { (load, wgpu::LoadOp::Clear(1.0), wgpu::LoadOp::Clear(0)) } else { (wgpu::LoadOp::Load, wgpu::LoadOp::Load, wgpu::LoadOp::Load) };
            enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(if first { "scene" } else { "scene (blended)" }),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color_view, resolve_target: None, ops: wgpu::Operations { load: cl, store: wgpu::StoreOp::Store } })],
                // The depth buffer is private to the renderer, so clearing all
                // of it is fine for a viewport too. The blended pass only
                // tests it (read-only, so sprites can sample it).
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: first.then_some(wgpu::Operations { load: dl, store: wgpu::StoreOp::Store }),
                    stencil_ops: first.then_some(wgpu::Operations { load: sl, store: wgpu::StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            })
            .forget_lifetime()
        };
        let p = &self.shape_pipelines;
        let layers = &self.dynamic.layers;
        let bind_mesh = |pass: &mut wgpu::RenderPass, l: &Layer, mesh: &ShapeMesh| {
            pass.set_bind_group(2, l.poses.as_ref().map_or(&self.empty_poses, |p| &p.1), &[]);
            pass.set_bind_group(1, &mesh.group, &[]);
            pass.set_vertex_buffer(0, mesh.vb.slice(..));
            pass.set_vertex_buffer(1, l.instances.as_ref().unwrap().slice(..));
            pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
        };
        let draw_static = |pass: &mut wgpu::RenderPass, blend: Blend| {
            if let Some((mesh, instances)) = self.shapes.as_ref().filter(|_| self.world) {
                pass.set_bind_group(2, &self.empty_poses, &[]);
                pass.set_bind_group(1, &mesh.group, &[]);
                pass.set_vertex_buffer(0, mesh.vb.slice(..));
                pass.set_vertex_buffer(1, instances.slice(..));
                pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
                for (b, range) in &self.static_draws {
                    let b = &mesh.batches[*b];
                    if b.blend == blend {
                        pass.draw_indexed(b.indices.clone(), 0, range.clone());
                    }
                }
            }
        };
        let setup = |pass: &mut wgpu::RenderPass, behind: &wgpu::BindGroup| {
            pass.set_viewport(px as f32, py as f32, pvw as f32, pvh as f32, 0.0, 1.0);
            pass.set_scissor_rect(px, py, pvw, pvh);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(3, behind, &[]);
        };
        // Blended static surfaces, then blended dynamic shapes and sprites
        // together, far to near.
        let blended = |pass: &mut wgpu::RenderPass| {
            pass.set_pipeline(&p.translucent);
            draw_static(pass, Blend::Translucent);
            pass.set_pipeline(&p.additive);
            draw_static(pass, Blend::Additive);
            let mut items: Vec<(f32, usize, &PathBuf, u32)> = Vec::new();
            for (li, l) in layers.iter().enumerate().filter(|(_, l)| l.instances.is_some()) {
                for (path, range) in &l.draws {
                    let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                    if mesh.batches.iter().any(|b| b.blend != Blend::Opaque) {
                        for i in range.clone() {
                            let d = l.positions.get(i as usize).map_or(0.0, |p| p.distance(camera.pos));
                            items.push((d, li, path, i));
                        }
                    }
                }
            }
            items.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut next_sprite = 0;
            for (d, li, path, i) in items {
                let mut end = next_sprite;
                while end < self.sprites.len() && self.sprites.distance(end) > d {
                    end += 1;
                }
                self.sprites.draw_range(pass, next_sprite..end);
                next_sprite = end;
                let l = &layers[li];
                let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                bind_mesh(pass, l, mesh);
                let level = self.dynamic_detail(mesh, l, i, camera.pos, pixel_scale);
                for (blend, pipeline) in [(Blend::Translucent, &p.translucent), (Blend::Additive, &p.additive)] {
                    let mut batches = mesh.batches.iter().filter(|b| b.blend == blend && b.detail == level).peekable();
                    if batches.peek().is_some() {
                        pass.set_pipeline(pipeline);
                        for b in batches {
                            pass.draw_indexed(b.indices.clone(), 0, i..i + 1);
                        }
                    }
                }
            }
            self.sprites.draw_range(pass, next_sprite..self.sprites.len());
        };
        {
            let mut pass = begin(&mut enc, true);
            setup(&mut pass, &self.behind_dummy);
            if view.clear && !full {
                pass.set_pipeline(&self.clear_pipeline);
                pass.draw(0..3, 0..1);
            }
            // The terrain's depth, then the opaque shapes (tested against
            // it), and only then the sky and the shaded terrain where they
            // are still the nearest: the same picture as drawing sky,
            // terrain and shapes in that order (ties too: the terrain's
            // depth is first), without shading hidden terrain or sky.
            if self.world {
                pass.set_pipeline(&self.terrain_depth_pipeline);
                pass.set_vertex_buffer(0, self.terrain_vb.slice(..));
                pass.set_index_buffer(self.terrain_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.terrain_index_count, 0, 0..1);
            }
            // Opaque geometry of all shapes (dynamic shadows are part of the
            // terrain shader).
            pass.set_pipeline(&p.opaque);
            draw_static(&mut pass, Blend::Opaque);
            for l in layers.iter().filter(|l| l.instances.is_some()) {
                for (path, range) in &l.draws {
                    let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                    if mesh.batches.iter().any(|b| b.blend == Blend::Opaque) {
                        bind_mesh(&mut pass, l, mesh);
                        for i in range.clone() {
                            let level = self.dynamic_detail(mesh, l, i, camera.pos, pixel_scale);
                            for b in mesh.batches.iter().filter(|b| b.blend == Blend::Opaque && b.detail == level) {
                                pass.draw_indexed(b.indices.clone(), 0, i..i + 1);
                            }
                        }
                    }
                }
            }
            if let Some((pipeline, group, vb, count)) = self.sky.as_ref().filter(|_| self.world) {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, group, &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*count, 0..1);
            }
            if self.world {
                pass.set_pipeline(&self.terrain_pipeline);
                pass.set_vertex_buffer(0, self.terrain_vb.slice(..));
                pass.set_index_buffer(self.terrain_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.terrain_index_count, 0, 0..1);
            }
            if self.world && !self.dynamic.layers[0].shadow_draws.is_empty() {
                pass.set_pipeline(&self.terrain_shadow_pipeline);
                for (caster, range) in &self.dynamic.layers[0].shadow_draws {
                    pass.draw_indexed(range.clone(), 0, *caster..*caster + 1);
                }
            }
            if self.world && self.marker_count > 0 {
                pass.set_pipeline(&self.color_pipeline);
                pass.set_vertex_buffer(0, self.markers_vb.slice(..));
                pass.draw(0..self.marker_count, 0..1);
            }
            if !split {
                blended(&mut pass);
            }
        }
        mark(&mut enc, "scene");
        if split {
            let mut behind = &self.behind_dummy;
            if let (Some(post), Some(rect)) = (self.post.as_ref(), glass) {
                post.copy_behind(&mut enc, rect);
                mark(&mut enc, "glass copy");
                behind = post.targets.as_ref().and_then(|t| t.behind_group.as_ref()).unwrap_or(&self.behind_dummy);
            }
            {
                let mut pass = begin(&mut enc, false);
                setup(&mut pass, behind);
                blended(&mut pass);
            }
            mark(&mut enc, "blended");
        }
        if let Some(post) = self.post.as_ref() {
            let tan_y = (view.fov_deg.to_radians() * 0.5).tan();
            let params = crate::post::ViewParams { near: self.near, far: self.far, tan_half_x: tan_y * vw as f32 / vh as f32, tan_half_y: tan_y, visible: self.scene_params[1] };
            post.run(queue, &mut enc, target, [vx, vy, vw, vh], !view.clear, params, &mark);
        }
        if scaled {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("upscale"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_viewport(vx as f32, vy as f32, vw as f32, vh as f32, 0.0, 1.0);
            pass.set_scissor_rect(vx, vy, vw, vh);
            pass.set_pipeline(if view.clear { &self.blit.0 } else { &self.blit.1 });
            pass.set_bind_group(0, &self.scaled.as_ref().unwrap().3, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            mark(&mut enc, "upscale");
        }
        let mut timer = self.timer.borrow_mut();
        if let Some(t) = timer.as_mut() {
            t.end(&mut enc);
        }
        queue.submit([enc.finish()]);
        if let Some(t) = timer.as_mut() {
            t.submitted();
        }
    }
}

#[cfg(test)]
mod detail_tests {
    use super::*;

    /// `treeblue01.dts`: detail100..detail2, then shadow details.
    fn tree() -> DetailTable {
        DetailTable { radius: 6.77, sizes: vec![100.0, 80.0, 60.0, 40.0, 20.0, 10.0, 2.0, -1.0, -2.0, -3.0], smallest_visible_size: 2.0, smallest_visible_dl: 6 }
    }

    #[test]
    fn detail_follows_the_projected_size() {
        let t = tree();
        assert_eq!(t.visible_levels(), 7);
        // 640x480 at 90° horizontal FOV: 320 px per unit at distance 1.
        let ps = detail_pixel_scale((640, 480), vertical_fov(90.0, 640.0 / 480.0));
        assert!((ps - 320.0).abs() < 0.5, "{ps}");
        // Pixel radius 6.77·320/d: above 100 nearer than 21.7 m.
        assert_eq!(select_detail(&t, 20.0, ps, 1.0, 0), 0);
        assert_eq!(select_detail(&t, 25.0, ps, 1.0, 0), 1);
        assert_eq!(select_detail(&t, 50.0, ps, 1.0, 0), 3);
        // Far away: the smallest visible level, never past it.
        assert_eq!(select_detail(&t, 5000.0, ps, 1.0, 0), 6);
        // detailAdjust 0.8 drops a level sooner; skipRenderDLs 1 never draws detail 0.
        assert_eq!(select_detail(&t, 20.0, ps, 0.8, 0), 1);
        assert_eq!(select_detail(&t, 1.0, ps, 1.0, 1), 1);
        // The pixel scale grows with the square of the width.
        let hd = detail_pixel_scale((1280, 960), vertical_fov(90.0, 1280.0 / 960.0));
        assert!((hd - 4.0 * ps).abs() < 1.0);
    }

    #[test]
    fn shadows_need_shadow_detail_0_2() {
        assert!(Detail::default().shadows());
        assert!(Detail { shadow_detail: 0.5, ..Detail::default() }.shadows());
        assert!(!Detail { shadow_detail: 0.0, ..Detail::default() }.shadows());
    }
}

#[cfg(test)]
mod mip_tests {
    use super::*;

    /// The reference mip chain (per-channel `powf`), which the tables must match bit for bit.
    fn mip_chain_reference(layer: &[u8], size: u32, srgb: bool) -> Vec<u8> {
        let to_lin = |c: u8| if srgb { (c as f32 / 255.0).powf(2.2) } else { c as f32 / 255.0 };
        let to_byte = |v: f32| ((if srgb { v.powf(1.0 / 2.2) } else { v }) * 255.0).round().clamp(0.0, 255.0) as u8;
        let mut out = layer.to_vec();
        let mut cur: Vec<f32> = layer.chunks(4).flat_map(|p| [to_lin(p[0]), to_lin(p[1]), to_lin(p[2]), p[3] as f32 / 255.0]).collect();
        let mut s = size as usize;
        while s > 1 {
            let h = s / 2;
            let mut next = vec![0.0f32; h * h * 4];
            for y in 0..h {
                for x in 0..h {
                    for c in 0..4 {
                        let at = |xx: usize, yy: usize| cur[(yy * s + xx) * 4 + c];
                        next[(y * h + x) * 4 + c] = (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1)) / 4.0;
                    }
                }
            }
            out.extend(next.chunks(4).flat_map(|p| [to_byte(p[0]), to_byte(p[1]), to_byte(p[2]), (p[3] * 255.0).round() as u8]));
            cur = next;
            s = h;
        }
        out
    }

    #[test]
    fn byte_thresholds_match_powf_exactly() {
        for srgb in [false, true] {
            let (_, th) = mip_tables(srgb);
            let exact = |v: f32| ((if srgb { v.powf(1.0 / 2.2) } else { v }) * 255.0).round().clamp(0.0, 255.0) as u8;
            // Every float in [0, 1] at a stride, plus the neighbours of each threshold.
            let mut bits = 0u32;
            while bits <= 1.0f32.to_bits() {
                let v = f32::from_bits(bits);
                assert_eq!(th.byte(v), exact(v), "srgb {srgb} v {v:e}");
                bits += 997;
            }
            for &t in &th.first[1..] {
                if t.is_finite() {
                    for v in [f32::from_bits(t.to_bits() - 1), t, f32::from_bits(t.to_bits() + 1)] {
                        assert_eq!(th.byte(v), exact(v), "srgb {srgb} v {v:e}");
                    }
                }
            }
            for v in [-1.0, 0.0, 1.0, 1.5, f32::NAN] {
                assert_eq!(th.byte(v), exact(v), "srgb {srgb} v {v}");
            }
        }
    }

    #[test]
    fn mip_chain_matches_reference() {
        let size = 64u32;
        let mut seed = 12345u32;
        let layer: Vec<u8> = (0..size * size * 4)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        for srgb in [false, true] {
            assert_eq!(mip_chain(&layer, size, srgb), mip_chain_reference(&layer, size, srgb));
        }
    }
}

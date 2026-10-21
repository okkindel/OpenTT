//! wgpu renderer for a [`Scene`](crate::Scene); draws into any colour target view.

mod detail;
mod dynamic;
mod frame;
mod meshes;
mod setup;
mod shadows;
mod sky;
mod terrain;
mod upload;
mod view;

pub use detail::{detail_pixel_scale, select_detail, Detail, DetailTable};
pub use dynamic::{DynamicInstance, InstanceOptions};
pub use frame::DEFAULT_FOV_DEG;
pub use sky::MAX_SKY_SPRITES;
pub use view::{vertical_fov, Camera, PointLight, View, MAX_POINT_LIGHTS};
pub(crate) use upload::{mip_chain, srgb_layers_texture};

use crate::shapes::InstanceData;
use dynamic::Dynamic;
use glam::Vec3;
use meshes::{ShapeMesh, ShapePipelines};
use terrain::Ground;

/// Depth/stencil format of the renderer's private depth buffer (the stencil
/// keeps projected shadows from darkening a pixel twice).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// Where a frame is drawn: the device and queue, the target view and its
/// size in pixels.
#[derive(Clone, Copy)]
pub struct Target<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub view: &'a wgpu::TextureView,
    pub size: (u32, u32),
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

pub struct Renderer {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    terrain_pipeline: wgpu::RenderPipeline,
    /// The terrain's depth alone (drawn before the other opaque surfaces).
    terrain_depth_pipeline: wgpu::RenderPipeline,
    terrain_vb: wgpu::Buffer,
    terrain_ib: wgpu::Buffer,
    terrain_index_count: u32,
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
    /// Draw sky, terrain and static shapes (false for object views).
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

impl Renderer {
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

}

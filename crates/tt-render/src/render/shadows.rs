//! Dynamic shape shadows: casters, their silhouette atlas and its pass.

use super::meshes::SHAPE_BUFFERS;
use super::setup::Setup;
use super::{RenderSettings, Renderer};
use crate::shapes::Blend;
use glam::{Vec2, Vec3};

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
pub(super) const SHADOW_LENGTH_FACTOR: f32 = 10.0;
/// Opacity of a projected shadow (see [`shadow_light_dir`]).
pub(super) const SHADOW_OPACITY: f32 = 0.5;
/// Height above the ground at which a shadow has faded out.
pub(super) const SHADOW_FADE_HEIGHT: f32 = 10.0;

/// One dynamic shadow for the terrain shader (std430 layout, 64 bytes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct ShadowCaster {
    /// xyz: shape centre (world), w: radius of the shadow box.
    pub(super) center: [f32; 4],
    /// xyz: light direction, w: shadow length.
    pub(super) dir: [f32; 4],
    /// xyz: first axis across the light, w: opacity.
    pub(super) u: [f32; 4],
    /// xyz: second axis across the light, w: atlas tile.
    pub(super) v: [f32; 4],
}

/// Shadow atlas: `ATLAS_TILES`² tiles of `atlas_tile` texels.
pub(super) fn atlas_tile(shadows: u8) -> u32 {
    match shadows {
        0 => 64,
        1 => 128,
        _ => 256,
    }
}

/// Light direction of the improved shadows: towards the ground from the
/// sun, at least ~25° down (a low sun would stretch them across the map).
pub(super) fn sun_shadow_dir(sun_dir: [f32; 4]) -> Vec3 {
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
pub(super) const SOFT_SHADOW_OPACITY: f32 = 0.6;
pub(super) const ATLAS_TILES: u32 = 8;
pub(super) const MAX_CASTERS: usize = (ATLAS_TILES * ATLAS_TILES) as usize;

/// The caster records' buffer and the silhouette atlas.
pub(super) fn targets(device: &wgpu::Device, settings: &RenderSettings) -> (wgpu::Buffer, wgpu::TextureView) {
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
    (casters, atlas_view)
}

/// The pipeline drawing silhouettes into the atlas and its group 0
/// (`globals`, `casters`).
pub(super) fn atlas_pipeline(s: &Setup, shape_layout: &wgpu::BindGroupLayout, pose_layout: &wgpu::BindGroupLayout, globals: &wgpu::Buffer, casters: &wgpu::Buffer) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
    let Setup { device, shader, .. } = *s;
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
        bind_group_layouts: &[&atlas_layout, shape_layout, pose_layout],
        push_constant_ranges: &[],
    });
    let atlas_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow atlas"),
        layout: Some(&atlas_pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_shadow_atlas"),
            compilation_options: Default::default(),
            buffers: &SHAPE_BUFFERS,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
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
    (atlas_pipeline, atlas_group)
}

impl Renderer {
    /// Draws the silhouettes of layer 0's shadow casters into the atlas.
    pub(super) fn draw_shadow_atlas(&self, enc: &mut wgpu::CommandEncoder) {
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
    }
}

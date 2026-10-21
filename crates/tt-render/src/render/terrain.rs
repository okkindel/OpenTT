//! The terrain: its textures, pipelines and heights.

use super::setup::Setup;
use super::upload::{flat_normals, layers_texture, normal_layers, texture};
use super::{RenderSettings, DEPTH_FORMAT};
use crate::scene::{Scene, TerrainVertex};

/// The vertex buffer of the terrain pipelines.
const TERRAIN_BUFFERS: [wgpu::VertexBufferLayout<'static>; 1] = [wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<TerrainVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &TERRAIN_ATTRIBUTES,
}];
const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

/// The terrain textures of group 0.
pub(super) struct TerrainTextures {
    pub(super) alpha_lo: wgpu::TextureView,
    pub(super) alpha_hi: wgpu::TextureView,
    pub(super) materials: wgpu::TextureView,
    pub(super) lightmap: wgpu::TextureView,
    pub(super) detail: wgpu::TextureView,
    pub(super) macro_tex: wgpu::TextureView,
    pub(super) normals: wgpu::TextureView,
}

impl TerrainTextures {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene, settings: &RenderSettings) -> TerrainTextures {
        let _sp = crate::loadstats::span(|| "renderer: terrain textures + mips".into());
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
        // Linear format: the detail and macro colour maps are multipliers
        // around 0.5, not colours; a flat 2 x 2 stand-in when the terrain has
        // none (the shader then skips the macro map: `terrain.w` is 0).
        let multiplier = |img: &Option<Vec<u8>>| match img {
            Some(px) => texture(device, queue, crate::textures::size(), 1, wgpu::TextureFormat::Rgba8Unorm, px),
            None => texture(device, queue, 2, 1, wgpu::TextureFormat::Rgba8Unorm, &[128; 16]),
        };
        let (detail, macro_tex) = (multiplier(&scene.detail), multiplier(&scene.macro_tex));
        let normals = if settings.extras.normal_maps {
            layers_texture(device, queue, &normal_layers(&scene.material_files), wgpu::TextureFormat::Rgba8Unorm)
        } else {
            flat_normals(device, queue)
        };
        TerrainTextures { alpha_lo, alpha_hi, materials, lightmap, detail, macro_tex, normals }
    }
}

/// The terrain's pipelines: (depth only, shaded, dynamic shadows).
pub(super) fn pipelines(s: &Setup) -> (wgpu::RenderPipeline, wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let Setup { device, shader, pipeline_layout, scene_format, .. } = *s;
    // (depth written, depth test, colour written)
    let pipeline_ext = |vs: &str, fs: &str, buffers: &[wgpu::VertexBufferLayout], depth_write: bool, depth_compare: wgpu::CompareFunction, color: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(vs),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
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
    // The terrain is drawn twice: its depth first (before everything
    // else opaque), then shaded where it is the nearest surface (the
    // same invariant positions; depth test LessEqual, no depth write),
    // so each visible terrain pixel runs the costly material shader
    // exactly once, and nothing hidden behind hills or shapes does.
    let terrain_depth_pipeline = pipeline_ext("vs_terrain", "fs_depth_only", &TERRAIN_BUFFERS, true, wgpu::CompareFunction::Less, false);
    let terrain_pipeline = pipeline_ext("vs_terrain", "fs_terrain", &TERRAIN_BUFFERS, false, wgpu::CompareFunction::LessEqual, true);
    let terrain_shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("terrain shadows"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_terrain_shadow"),
            compilation_options: Default::default(),
            buffers: &TERRAIN_BUFFERS,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
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
    (terrain_depth_pipeline, terrain_pipeline, terrain_shadow_pipeline)
}

/// The terrain mesh grid: first cell (grid units) and cells per row.
pub(super) fn grid(scene: &Scene) -> (i64, i64, i64) {
    let n = scene.terrain_vertices.len();
    let per_row = (n as f64).sqrt().round() as i64;
    let first = scene.terrain_vertices.first().map_or([0.0; 2], |v| v.grid);
    (first[0] as i64, first[1] as i64, (per_row - 1).max(0))
}

/// Terrain heights for fitting shadow planes.
pub(super) struct Ground {
    pub(super) terrain: tt_assets::terrain::Terrain,
    pub(super) origin: f32,
    pub(super) square: f32,
}

impl Ground {
    pub(super) fn height(&self, x: f32, y: f32) -> f32 {
        crate::lighting::height_at(&self.terrain, (x - self.origin) / self.square, (y - self.origin) / self.square)
    }
}

//! Creating a [`Renderer`]: its resources and pipelines.

use super::dynamic::Dynamic;
use super::meshes::{self, ShapeMesh, ShapePipelines};
use super::terrain::{self, Ground, TerrainTextures};
use super::upload::repeat_sampler;
use super::view::Globals;
use super::{shadows, sky, RenderSettings, Renderer, DEPTH_FORMAT, MAX_POINT_LIGHTS};
use crate::scene::Scene;
use wgpu::util::DeviceExt;

/// What the pipeline builders of [`Renderer::new_with_settings`] share.
#[derive(Clone, Copy)]
pub(super) struct Setup<'a> {
    pub(super) device: &'a wgpu::Device,
    pub(super) queue: &'a wgpu::Queue,
    pub(super) shader: &'a wgpu::ShaderModule,
    /// Group 0: globals, terrain textures, shadow casters and atlas.
    pub(super) layout: &'a wgpu::BindGroupLayout,
    /// The pipeline layout of group 0 alone.
    pub(super) pipeline_layout: &'a wgpu::PipelineLayout,
    /// Colour format of the scene pass.
    pub(super) scene_format: wgpu::TextureFormat,
    pub(super) settings: RenderSettings,
}

/// A filterable float texture of the fragment stage at `binding`.
pub(super) fn tex_entry(binding: u32, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dim,
            multisampled: false,
        },
        count: None,
    }
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, scene: &Scene) -> Renderer {
        Renderer::new_with_settings(device, queue, format, scene, RenderSettings::default())
    }

    /// [`Renderer::new`] with explicit [`RenderSettings`].
    pub fn new_with_settings(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, scene: &Scene, settings: RenderSettings) -> Renderer {
        let _all = crate::loadstats::span(|| "renderer: total".into());
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shader.wgsl"));
        // The graphics extras draw the scene into an HDR texture first.
        let scene_format = if settings.extras.post() { crate::post::scene_format(device) } else { format };
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let terrain = TerrainTextures::new(device, queue, scene, &settings);
        let anisotropy = settings.anisotropy.clamp(1, 16);
        let sampler = repeat_sampler(device, anisotropy, settings.detail.terrain_tex_detail);
        // Terrain normal maps: trilinear only (see shader.wgsl).
        let normal_sampler = repeat_sampler(device, 1, settings.detail.terrain_tex_detail);
        let (casters, atlas_view) = shadows::targets(device, &settings);
        let layout = globals_layout(device);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&terrain.alpha_lo) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&terrain.alpha_hi) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&terrain.materials) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&terrain.lightmap) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&terrain.detail) },
                wgpu::BindGroupEntry { binding: 7, resource: casters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&terrain.normals) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Sampler(&normal_sampler) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&terrain.macro_tex) },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        let s = Setup { device, queue, shader: &shader, layout: &layout, pipeline_layout: &pipeline_layout, scene_format, settings };
        let (terrain_depth_pipeline, terrain_pipeline, terrain_shadow_pipeline) = terrain::pipelines(&s);
        let sky = sky::sky(&s, scene);
        let (behind_layout, behind_sampler, behind_dummy) = meshes::glass_behind(&s);
        let (pose_layout, empty_poses) = meshes::poses(device);
        let shape_pipelines = ShapePipelines::new(&s, &pose_layout, &behind_layout);
        let (atlas_pipeline, atlas_group) = shadows::atlas_pipeline(&s, &shape_pipelines.layout, &pose_layout, &globals, &casters);
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
        let f = scene.fog_color;
        // The fog colour is specified in display (sRGB) space.
        let lin = |c: f32| c.clamp(0.0, 1.0).powf(2.2) as f64;
        Renderer {
            globals,
            bind_group,
            terrain_pipeline,
            terrain_depth_pipeline,
            terrain_vb: buffer(bytemuck::cast_slice(&scene.terrain_vertices), wgpu::BufferUsages::VERTEX),
            terrain_ib: buffer(bytemuck::cast_slice(&scene.terrain_indices), wgpu::BufferUsages::INDEX),
            terrain_index_count: scene.terrain_indices.len() as u32,
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
            terrain_grid: terrain::grid(scene),
            atlas_group,
            atlas_view,
            casters,
            ground: Ground { terrain: scene.terrain.clone(), origin: scene.terrain_origin, square: scene.square_size },
            sprites,
            sprite_layers: [Vec::new(), Vec::new()],
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
            blit: blit(&s, format),
            scaled: None,
            clear: wgpu::Color { r: lin(f.x), g: lin(f.y), b: lin(f.z), a: 1.0 },
            far: scene.visible_distance.max(100.0) * 1.2,
            scene_params: [scene.fog_distance, scene.visible_distance, scene.terrain_size as f32, 32.0],
            terrain_params: [scene.terrain_origin, scene.square_size, scene.terrain_size as f32, if scene.macro_tex.is_some() { 1.0 } else { 0.0 }],
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
}

/// The layout of group 0 (globals, terrain textures, shadows).
fn globals_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            tex_entry(11, wgpu::TextureViewDimension::D2),
        ],
    })
}

/// The upscale of a scaled view: (pipeline replacing, pipeline blending
/// over the target, layout, sampler).
fn blit(s: &Setup, format: wgpu::TextureFormat) -> (wgpu::RenderPipeline, wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    let Setup { device, shader, .. } = *s;
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
            vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_blit"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: shader,
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
}

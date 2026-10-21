//! The sky box and its sprites.

use super::setup::{tex_entry, Setup};
use super::upload::texture;
use super::DEPTH_FORMAT;
use crate::scene::{Scene, SkyVertex};
use wgpu::util::DeviceExt;

/// The most sky sprites a sky draws (`Scene::sky_sprites`).
pub const MAX_SKY_SPRITES: usize = 2;

/// The sky's pipeline, bind group and vertex buffer with its vertex
/// count; None without sky faces.
pub(super) fn sky(s: &Setup, scene: &Scene) -> Option<(wgpu::RenderPipeline, wgpu::BindGroup, wgpu::Buffer, u32)> {
    let Setup { device, queue, shader, layout, scene_format, settings, .. } = *s;
    if scene.sky_faces.is_empty() {
        return None;
    }
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
            tex_entry(2, wgpu::TextureViewDimension::D2Array),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            },
        ],
    });
    // Sky sprites (`Scene::sky_sprites`): their images as layers (a
    // transparent stand-in when there are none: two layers, so its
    // view is an array as the layout wants; one sprite gets a blank
    // second layer too) and, per sprite, its direction and the
    // tangent of its angular radius.
    let (sprite_size, mut sprite_layers) = if scene.sky_sprites.is_empty() { (2, vec![vec![0u8; 16]]) } else { (crate::textures::size(), scene.sky_sprites.iter().map(|s| s.image.clone()).collect()) };
    if sprite_layers.len() == 1 {
        sprite_layers.push(vec![0u8; sprite_layers[0].len()]);
    }
    let sprites = texture(device, queue, sprite_size, sprite_layers.len() as u32, wgpu::TextureFormat::Rgba8UnormSrgb, &sprite_layers.concat());
    let mut info = [[0.0f32; 4]; 1 + MAX_SKY_SPRITES];
    info[0][0] = scene.sky_sprites.len().min(MAX_SKY_SPRITES) as f32;
    for (slot, s) in info[1..].iter_mut().zip(&scene.sky_sprites) {
        *slot = [s.dir.x, s.dir.y, s.dir.z, s.radius.tan()];
    }
    let sprite_info = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("sky sprites"), contents: bytemuck::cast_slice(&info), usage: wgpu::BufferUsages::UNIFORM });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sky"),
        layout: &sky_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&faces) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&clamp) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&sprites) },
            wgpu::BindGroupEntry { binding: 3, resource: sprite_info.as_entire_binding() },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sky"),
        bind_group_layouts: &[layout, &sky_layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sky"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_sky"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SkyVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Uint32],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
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
    Some((pipeline, group, vb, scene.sky_vertices.len() as u32))
}

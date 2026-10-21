//! Shapes on the GPU: their pipelines, meshes and pose groups.

use super::setup::{tex_entry, Setup};
use super::upload::{flat_normals, layers_texture, normal_layers, repeat_sampler, srgb_layers_texture, texture};
use super::DEPTH_FORMAT;
use crate::shapes::{InstanceData, ShapeBatch, ShapeSet, ShapeVertex};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

/// Pipelines and layout shared by static and dynamic shapes.
pub(super) struct ShapePipelines {
    pub(super) opaque: wgpu::RenderPipeline,
    pub(super) translucent: wgpu::RenderPipeline,
    pub(super) additive: wgpu::RenderPipeline,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) sampler: wgpu::Sampler,
    /// Normal maps: trilinear, no anisotropy.
    pub(super) normal_sampler: wgpu::Sampler,
    /// Load `*.normal.png` for the texture layers (normal-map extra).
    pub(super) normals: bool,
}

/// The vertex and instance buffers of the shape pipelines (the scene's
/// and the shadow atlas').
pub(super) const SHAPE_BUFFERS: [wgpu::VertexBufferLayout<'static>; 2] = [
    wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<ShapeVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SHAPE_VERTEX_ATTRIBUTES },
    wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<InstanceData>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &SHAPE_INSTANCE_ATTRIBUTES },
];
const SHAPE_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 9] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Uint32, 4 => Uint32, 5 => Uint32, 11 => Uint32, 14 => Uint8x4, 15 => Unorm8x4];
const SHAPE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] =
    wgpu::vertex_attr_array![6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 12 => Uint32x4, 13 => Float32x4];

/// A [`ShapeSet`] on the GPU (geometry, texture layers, IFL timeline).
pub(super) struct ShapeMesh {
    pub(super) group: wgpu::BindGroup,
    pub(super) vb: wgpu::Buffer,
    pub(super) ib: wgpu::Buffer,
    pub(super) batches: Vec<ShapeBatch>,
    /// Batch indices per shape (`lods` index) and detail level.
    pub(super) by_level: Vec<Vec<Vec<usize>>>,
    pub(super) lods: Vec<crate::shapes::LodGroup>,
    /// Dynamic-shadow geometry and shape radius (posed shapes).
    pub(super) shadow: std::ops::Range<u32>,
    pub(super) radius: f32,
    pub(super) center: Vec3,
    pub(super) bounds_radius: f32,
}

pub(super) fn pose_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, buf: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("poses"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
    })
}

impl ShapeMesh {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue, p: &ShapePipelines, set: &ShapeSet) -> ShapeMesh {
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

impl ShapePipelines {
    /// The pipelines of the scene's shapes (groups: globals, shape,
    /// `pose_layout`, `behind_layout`).
    pub(super) fn new(s: &Setup, pose_layout: &wgpu::BindGroupLayout, behind_layout: &wgpu::BindGroupLayout) -> ShapePipelines {
        let Setup { device, shader, layout, scene_format, settings, .. } = *s;
        let anisotropy = settings.anisotropy.clamp(1, 16);
        let shape_sampler = repeat_sampler(device, anisotropy, settings.detail.mip_reduction);
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shapes"),
            bind_group_layouts: &[layout, &shape_layout, pose_layout, behind_layout],
            push_constant_ranges: &[],
        });
        let make_full = |vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth_stencil: wgpu::DepthStencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &SHAPE_BUFFERS,
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
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
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        ShapePipelines {
            opaque: make("fs_shape", None, true),
            translucent: make("fs_shape_blend", Some(wgpu::BlendState::ALPHA_BLENDING), false),
            additive: make("fs_shape_add", Some(additive), false),
            layout: shape_layout,
            sampler: shape_sampler,
            normal_sampler: repeat_sampler(device, 1, settings.detail.mip_reduction),
            normals: settings.extras.normal_maps,
        }
    }
}

/// Group 3 of the shape pipelines, the opaque scene behind the glass:
/// its layout, sampler and the group used when there is no glass copy.
pub(super) fn glass_behind(s: &Setup) -> (wgpu::BindGroupLayout, wgpu::Sampler, wgpu::BindGroup) {
    let Setup { device, queue, .. } = *s;
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
    (behind_layout, behind_sampler, behind_dummy)
}

/// The layout of the pose matrices (group 2) and the group bound for
/// draws without a dynamic pose buffer.
pub(super) fn poses(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
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
    (pose_layout, empty_poses)
}

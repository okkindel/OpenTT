
use glam::Vec3;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SpriteBlend {
    /// `SRC_ALPHA, ONE_MINUS_SRC_ALPHA` (particles with `inverse_alpha`,
    /// projectile and explosion sprites).
    #[default]
    Alpha,
    /// `SRC_ALPHA, ONE`.
    Additive,
}

/// How a sprite quad is oriented.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SpriteOrient {
    /// Facing the camera (`dglDrawBillboard`).
    #[default]
    Billboard,
    /// The quad's x axis is horizontal (`dir × Z`), its y axis as close to
    /// world up as possible.
    Axis { dir: Vec3 },
    /// In the plane with normal `normal` (e.g. the terrain normal for a
    /// ground decal), its y axis towards `up` projected into the plane.
    Plane { normal: Vec3, up: Vec3 },
}

#[derive(Clone, Debug)]
pub struct Sprite {
    /// Image path (any case; `.png`/`.jpg`).
    pub texture: PathBuf,
    pub pos: Vec3,
    /// Full width and height in metres.
    pub size: f32,
    /// Radians about the view axis.
    pub rotation: f32,
    /// Display-space RGBA multiplied with the texture.
    pub color: [f32; 4],
    pub blend: SpriteBlend,
    pub orient: SpriteOrient,
    /// Moves the quad this many metres towards the camera (against
    /// z-fighting for quads lying on surfaces).
    pub depth_offset: f32,
}

impl Default for Sprite {
    fn default() -> Sprite {
        Sprite {
            texture: PathBuf::new(),
            pos: Vec3::ZERO,
            size: 1.0,
            rotation: 0.0,
            color: [1.0; 4],
            blend: SpriteBlend::Alpha,
            orient: SpriteOrient::Billboard,
            depth_offset: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpriteVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    /// Soft-particle fade distance in metres (0: a hard quad, e.g. reticles).
    soft: f32,
}

/// Soft-particle fade distance of a camera-facing sprite of side `size`: the
/// quad fades out over this many metres in front of the scene behind it, so
/// a big explosion billboard cutting the ground shows no hard line.
pub(crate) fn soft_distance(size: f32) -> f32 {
    (size * 0.1).clamp(0.15, 4.0)
}

pub(crate) struct SpriteGpu {
    alpha: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Group 2: the scene depth (read-only during the blended pass).
    depth_layout: wgpu::BindGroupLayout,
    depth_group: Option<wgpu::BindGroup>,
    sampler: wgpu::Sampler,
    textures: HashMap<PathBuf, Option<std::sync::Arc<wgpu::BindGroup>>>,
    vb: Option<wgpu::Buffer>,
    capacity: usize,
    /// This frame's sprites far to near: (distance to the eye, texture,
    /// blend); sprite `i` uses vertices `6i..6i+6`.
    sorted: Vec<(f32, std::sync::Arc<wgpu::BindGroup>, SpriteBlend)>,
}

pub(crate) fn axis_frame(dir: Vec3) -> (Vec3, Vec3) {
    let y = (-dir).try_normalize().unwrap_or(Vec3::NEG_Y);
    if y.z.abs() < 0.99 {
        let x = Vec3::new(y.y, -y.x, 0.0).normalize();
        (x, x.cross(y))
    } else {
        let z = Vec3::new(0.0, -y.z, y.y).normalize();
        (y.cross(z), z)
    }
}

impl SpriteGpu {
    pub(crate) fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule, globals_layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat, depth: wgpu::TextureFormat) -> SpriteGpu {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprites"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite scene depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprites"),
            bind_group_layouts: &[globals_layout, &layout, &depth_layout],
            push_constant_ranges: &[],
        });
        let make = |blend: wgpu::BlendState, fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sprites"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_sprite"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<SpriteVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Float32],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::OVER,
        };
        SpriteGpu {
            alpha: make(wgpu::BlendState::ALPHA_BLENDING, "fs_sprite"),
            additive: make(additive, "fs_sprite_add"),
            layout,
            depth_layout,
            depth_group: None,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            textures: HashMap::new(),
            vb: None,
            capacity: 0,
            sorted: Vec::new(),
        }
    }

    /// Takes over the textures `old` loaded (same device and sampler).
    pub(crate) fn reuse_textures(&mut self, old: SpriteGpu) {
        for (path, group) in old.textures {
            self.textures.entry(path).or_insert(group);
        }
    }

    /// Binds the scene depth `view` (depth aspect of the pass's depth
    /// attachment, which must then be read-only) for the soft fade.
    pub(crate) fn set_scene_depth(&mut self, device: &wgpu::Device, view: &wgpu::TextureView) {
        self.depth_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite scene depth"),
            layout: &self.depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }],
        }));
    }

    /// `path` was loaded (or found missing) already.
    pub(crate) fn has_texture(&self, path: &Path) -> bool {
        self.textures.contains_key(path)
    }

    pub(crate) fn texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, path: &Path) -> bool {
        self.group(device, queue, path).is_some()
    }

    /// The bind group of texture `path` (loaded on first use; None if missing).
    fn group(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, path: &Path) -> Option<std::sync::Arc<wgpu::BindGroup>> {
        if let Some(g) = self.textures.get(path) {
            return g.clone();
        }
        {
            let px = path.parent().zip(path.file_name().and_then(|n| n.to_str())).and_then(|(dir, name)| crate::shapes::load_image_shared(dir, name));
            if px.is_none() {
                eprintln!("warning: sprite texture not found: {}", path.display());
            }
            let group = px.map(|px| {
                let view = crate::render::srgb_layers_texture(device, queue, &[px]);
                std::sync::Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sprite"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    ],
                }))
            });
            self.textures.insert(path.to_path_buf(), group.clone());
            group
        }
    }

    /// Builds this frame's quads, sorted far to near.
    pub(crate) fn prepare<'a>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, sprites: impl Iterator<Item = &'a Sprite>, eye: Vec3, forward: Vec3) {
        let right = forward.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(forward).normalize();
        // Visible sprites with their texture's bind group, far to near.
        let mut list: Vec<(&Sprite, std::sync::Arc<wgpu::BindGroup>)> = sprites.filter(|s| s.size > 0.0 && s.color[3] > 0.0).filter_map(|s| Some((s, self.group(device, queue, &s.texture)?))).collect();
        list.sort_by(|a, b| b.0.pos.distance_squared(eye).total_cmp(&a.0.pos.distance_squared(eye)));
        let mut verts = Vec::with_capacity(list.len() * 6);
        self.sorted.clear();
        for (s, group) in list {
            let h = s.size * 0.5;
            let (sin, cos) = s.rotation.sin_cos();
            let (right, up) = match s.orient {
                SpriteOrient::Billboard => (right, up),
                SpriteOrient::Axis { dir } => axis_frame(dir),
                SpriteOrient::Plane { normal, up: hint } => {
                    let n = normal.try_normalize().unwrap_or(Vec3::Z);
                    let r = hint.cross(n).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
                    (r, n.cross(r))
                }
            };
            let (r, u) = (right * cos + up * sin, up * cos - right * sin);
            let pos = s.pos + (eye - s.pos).try_normalize().unwrap_or(Vec3::ZERO) * s.depth_offset;
            let color = [crate::srgb_to_linear(s.color[0]), crate::srgb_to_linear(s.color[1]), crate::srgb_to_linear(s.color[2]), s.color[3].clamp(0.0, 1.0)];
            // Billboards (particles, explosions, projectiles) are soft;
            // reticle dots and rings lying on the ground stay hard.
            let soft = if s.orient == SpriteOrient::Billboard { soft_distance(s.size) } else { 0.0 };
            let corner = |x: f32, y: f32, uv: [f32; 2]| SpriteVertex { pos: (pos + r * (x * h) + u * (y * h)).into(), uv, color, soft };
            let q = [corner(-1.0, -1.0, [0.0, 0.0]), corner(-1.0, 1.0, [0.0, 1.0]), corner(1.0, 1.0, [1.0, 1.0]), corner(1.0, -1.0, [1.0, 0.0])];
            verts.extend([q[0], q[1], q[2], q[0], q[2], q[3]]);
            self.sorted.push((s.pos.distance(eye), group, s.blend));
        }
        if verts.is_empty() {
            return;
        }
        if self.vb.is_none() || self.capacity < verts.len() {
            self.capacity = verts.len().next_power_of_two().max(1024);
            self.vb = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sprites"),
                size: (self.capacity * std::mem::size_of::<SpriteVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        queue.write_buffer(self.vb.as_ref().unwrap(), 0, bytemuck::cast_slice(&verts));
    }

    /// Number of sprites prepared for this frame.
    pub(crate) fn len(&self) -> usize {
        self.sorted.len()
    }

    /// Distance to the eye of prepared sprite `i` (far to near order).
    pub(crate) fn distance(&self, i: usize) -> f32 {
        self.sorted[i].0
    }

    /// Draws prepared sprites `range` (indices in far-to-near order),
    /// batching neighbours with the same texture and blend.
    pub(crate) fn draw_range(&self, pass: &mut wgpu::RenderPass, range: std::ops::Range<usize>) {
        let (Some(vb), Some(depth)) = (&self.vb, &self.depth_group) else { return };
        let range = range.start.min(self.sorted.len())..range.end.min(self.sorted.len());
        if range.is_empty() {
            return;
        }
        pass.set_vertex_buffer(0, vb.slice(..));
        pass.set_bind_group(2, depth, &[]);
        let mut i = range.start;
        while i < range.end {
            let (_, group, blend) = &self.sorted[i];
            let mut j = i + 1;
            while j < range.end && std::sync::Arc::ptr_eq(&self.sorted[j].1, group) && self.sorted[j].2 == *blend {
                j += 1;
            }
            pass.set_pipeline(match blend {
                SpriteBlend::Alpha => &self.alpha,
                SpriteBlend::Additive => &self.additive,
            });
            pass.set_bind_group(1, group.as_ref(), &[]);
            pass.draw((i * 6) as u32..(j * 6) as u32, 0..1);
            i = j;
        }
    }
}

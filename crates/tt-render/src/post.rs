//! Post-processing of the graphics extras: the scene is drawn into a linear
//! HDR texture, then SSAO (half resolution, from a half-resolution linear
//! depth), bloom (bright pass into 1/4 resolution, down/up chain) and one
//! composite pass (AO, bloom, soft tonemap) write the view into the target.
//! With every extra off the renderer never creates this and draws straight
//! into the target.
//!
//! Bandwidth is what costs on integrated GPUs: the scene texture is
//! `Rg11b10Ufloat` (4 bytes per pixel) when the device has
//! `RG11B10UFLOAT_RENDERABLE`, else `Rgba16Float` (8). The packed format has
//! no alpha, so a view that keeps the target's background (`View::clear`
//! off; no game view does) is composited opaque then. Every texture, bind
//! group and pipeline is made once per renderer / view size, never per frame.

use wgpu::util::DeviceExt;

/// Scene colour format of the post path without `RG11B10UFLOAT_RENDERABLE`.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Device features the post path uses when the adapter has them.
pub const OPTIONAL_FEATURES: wgpu::Features = wgpu::Features::RG11B10UFLOAT_RENDERABLE;
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// Half-resolution linear depth (SSAO input).
const AO_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Bloom chain levels (1/4 .. 1/32 of the view).
const BLOOM_LEVELS: usize = 4;

/// Scene colour format of the post path on `device`.
pub fn scene_format(device: &wgpu::Device) -> wgpu::TextureFormat {
    if device.features().contains(wgpu::Features::RG11B10UFLOAT_RENDERABLE) {
        wgpu::TextureFormat::Rg11b10Ufloat
    } else {
        HDR_FORMAT
    }
}

/// The graphics extras ("Options → Graphics").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extras {
    /// Glass brain jars: Fresnel reflection, sun highlight, refraction.
    pub glass: bool,
    /// HDR rendering with the soft tonemap, emissive boost and sun glow.
    pub hdr: bool,
    /// Bloom strength 0..1 (needs `hdr`); changes apply at once.
    pub bloom: f32,
    /// Normal maps on terrain and shapes (`*.normal.png`).
    pub normal_maps: bool,
    /// Normal-map strength ("Surface detail") 0..2; changes apply at once.
    pub normal_strength: f32,
    /// Screen-space ambient occlusion: 0 off, 1 low, 2 high.
    pub ssao: u8,
    /// AO strength 0..2 ("AO strength"); changes apply at once.
    pub ao_strength: f32,
    pub shadows: u8,
}

impl Default for Extras {
    fn default() -> Extras {
        Extras { glass: false, hdr: false, bloom: 0.0, normal_maps: false, normal_strength: 0.4, ssao: 0, ao_strength: 1.0, shadows: 0 }
    }
}

impl Extras {
    /// The scene goes through the post path (an HDR texture).
    pub fn post(&self) -> bool {
        self.glass || self.hdr || self.ssao > 0
    }

    /// Everything fixed when the renderer is created.
    pub fn fixed(&self) -> (bool, bool, bool, u8, u8) {
        (self.glass, self.hdr, self.normal_maps, self.ssao.min(2), self.shadows.min(2))
    }

    /// The "Modern" preset.
    pub fn modern() -> Extras {
        Extras { glass: true, hdr: true, bloom: 0.5, normal_maps: true, normal_strength: 0.4, ssao: 1, ao_strength: 1.0, shadows: 1 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
struct PostUniform {
    params: [f32; 4],
    proj: [f32; 4],
    texel: [f32; 4],
    // x, y: texel size of the bloom texture (composite).
    extra: [f32; 4],
    fog: [f32; 4],
    up: [f32; 4],
    vols: [[f32; 4]; 3],
}

/// Per-view projection data for SSAO.
#[derive(Clone, Copy, Debug)]
pub struct ViewParams {
    pub near: f32,
    pub far: f32,
    pub tan_half_x: f32,
    pub tan_half_y: f32,
    /// Fog visible distance (SSAO fades out towards it).
    pub visible: f32,
    /// Fog distance, camera world z, world z of the view axes, and the
    /// height fog bands (the composite keeps fogged pixels unoccluded).
    pub fog_distance: f32,
    pub cam_z: f32,
    pub up: [f32; 3],
    pub vols: [[f32; 4]; 3],
}

/// Where [`Post::run`] composites the scene: `target`'s viewport `[x, y,
/// w, h]`, over what is there (premultiplied) when `blend`.
pub(crate) struct Output<'a> {
    pub target: &'a wgpu::TextureView,
    pub viewport: [u32; 4],
    pub blend: bool,
}

struct Pass {
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    target: wgpu::TextureView,
    size: (u32, u32),
}

/// The textures of one view size.
pub(crate) struct Targets {
    pub size: (u32, u32),
    pub color_tex: wgpu::Texture,
    pub color: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    /// The depth aspect alone (sampled by soft sprites and the passes here).
    pub depth_sample: wgpu::TextureView,
    /// Copy of the opaque scene for glass refraction (and its group 3).
    pub behind_tex: Option<wgpu::Texture>,
    pub behind_group: Option<wgpu::BindGroup>,
    ssao_depth: Option<Pass>,
    ssao: Option<Pass>,
    ssao_blur: Option<Pass>,
    bloom_pre: Option<Pass>,
    bloom_down: Vec<Pass>,
    bloom_up: Vec<Pass>,
    composite_group: wgpu::BindGroup,
    composite_uniform: wgpu::Buffer,
    ao_texel: [f32; 2],
    bloom_texel: [f32; 2],
}

pub(crate) struct Post {
    pub extras: Extras,
    layout: wgpu::BindGroupLayout,
    /// SSAO pass: the uniform and the (unfilterable) linear depth.
    ao_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    ssao_depth: wgpu::RenderPipeline,
    ssao: wgpu::RenderPipeline,
    ssao_blur: wgpu::RenderPipeline,
    pre: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    composite: (wgpu::RenderPipeline, wgpu::RenderPipeline),
    dummy: wgpu::TextureView,
    dummy_ao: wgpu::TextureView,
    pub targets: Option<Targets>,
}

fn tex(device: &wgpu::Device, label: &str, size: (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn pixel(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, rgba: [u8; 4]) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &rgba,
        )
        .create_view(&Default::default())
}

impl Post {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, extras: Extras) -> Post {
        let shader = device.create_shader_module(wgpu::include_wgsl!("post.wgsl"));
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let ftex = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                uniform_entry,
                ftex(1, true),
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                ftex(4, true),
                ftex(5, true),
            ],
        });
        let ao_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("post ssao"), entries: &[uniform_entry, ftex(1, false)] });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let ao_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post ssao"), bind_group_layouts: &[&ao_layout], push_constant_ranges: &[] });
        let make = |pl: &wgpu::PipelineLayout, fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
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
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let hdr = scene_format(device);
        // Only the pipelines the fixed extras use.
        let (ao_depth, ao) = if extras.ssao > 0 { (AO_DEPTH_FORMAT, AO_FORMAT) } else { (AO_FORMAT, AO_FORMAT) };
        Post {
            extras,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
            ssao_depth: make(&pl, "fs_ssao_depth", ao_depth, None),
            ssao: make(&ao_pl, "fs_ssao", ao, None),
            ssao_blur: make(&pl, "fs_ssao_blur", AO_FORMAT, None),
            pre: make(&pl, "fs_bloom_pre", hdr, None),
            down: make(&pl, "fs_bloom_down", hdr, None),
            up: make(&pl, "fs_bloom_up", hdr, Some(add)),
            composite: (make(&pl, "fs_composite", format, None), make(&pl, "fs_composite", format, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING))),
            layout,
            ao_layout,
            format: hdr,
            dummy: pixel(device, queue, "post dummy", [0, 0, 0, 255]),
            dummy_ao: pixel(device, queue, "post dummy ao", [255, 255, 255, 255]),
            targets: None,
        }
    }

    fn group(&self, device: &wgpu::Device, uniform: &wgpu::Buffer, src: &wgpu::TextureView, depth: &wgpu::TextureView, ao: &wgpu::TextureView, bloom: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(ao) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(bloom) },
            ],
        })
    }

    /// Makes sure the textures fit a view of `size` (scene resolution).
    /// `behind`: the layout and sampler of the glass group (group 3).
    pub(crate) fn ensure(&mut self, device: &wgpu::Device, size: (u32, u32), depth_format: wgpu::TextureFormat, behind: (&wgpu::BindGroupLayout, &wgpu::Sampler)) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let e = self.extras;
        let hdr = self.format;
        let color_tex = tex(device, "hdr scene", size, hdr, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let color = color_tex.create_view(&Default::default());
        let depth_tex = tex(device, "post depth", size, depth_format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let depth = depth_tex.create_view(&Default::default());
        let depth_sample = depth_tex.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
        let (behind_tex, behind_group) = if e.glass {
            let t = tex(device, "glass behind", size, hdr, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
            let v = t.create_view(&Default::default());
            let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("glass behind"),
                layout: behind.0,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&v) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(behind.1) },
                ],
            });
            (Some(t), Some(g))
        } else {
            (None, None)
        };
        let uniform = |device: &wgpu::Device| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("post"), contents: bytemuck::bytes_of(&PostUniform::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST })
        };
        let (ssao_depth, ssao, ssao_blur, ao_view, ao_size) = if e.ssao > 0 {
            // Half resolution (low: 8 samples, high: 16), from a
            // half-resolution linear depth (cache-friendly taps).
            let s = ((size.0 / 2).max(1), (size.1 / 2).max(1));
            let lt = tex(device, "ssao depth", s, AO_DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
            let lv = lt.create_view(&Default::default());
            let lu = uniform(device);
            let lgroup = self.group(device, &lu, &self.dummy, &depth_sample, &self.dummy_ao, &self.dummy);
            let t = tex(device, "ssao", s, AO_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
            let v = t.create_view(&Default::default());
            let u = uniform(device);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post ssao"),
                layout: &self.ao_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lv) }],
            });
            // The blur at half resolution (four bilinear taps = a 4x4 box over
            // the noise pattern); the full-resolution composite then needs a
            // single tap (texture sampling is what full-screen passes pay for
            // on integrated GPUs).
            let bt = tex(device, "ssao blurred", s, AO_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
            let bv = bt.create_view(&Default::default());
            let bu = uniform(device);
            let bgroup = self.group(device, &bu, &v, &depth_sample, &self.dummy_ao, &self.dummy);
            (
                Some(Pass { group: lgroup, uniform: lu, target: lv, size: s }),
                Some(Pass { group, uniform: u, target: v, size: s }),
                Some(Pass { group: bgroup, uniform: bu, target: bv.clone(), size: s }),
                bv,
                s,
            )
        } else {
            (None, None, None, self.dummy_ao.clone(), (1, 1))
        };
        let mut bloom_pre = None;
        let mut bloom_down = Vec::new();
        let mut bloom_up = Vec::new();
        let mut bloom_view = self.dummy.clone();
        let mut bloom_texel = [1.0, 1.0];
        if e.hdr {
            // The bright pass box-filters 4x4 scene pixels into the 1/4 level.
            let mut levels = Vec::new();
            let mut s = ((size.0 / 2).max(1), (size.1 / 2).max(1));
            for _ in 0..BLOOM_LEVELS {
                s = ((s.0 / 2).max(1), (s.1 / 2).max(1));
                let t = tex(device, "bloom", s, hdr, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
                levels.push((t.create_view(&Default::default()), s));
            }
            let u = uniform(device);
            bloom_pre = Some(Pass { group: self.group(device, &u, &color, &depth_sample, &self.dummy_ao, &self.dummy), uniform: u, target: levels[0].0.clone(), size: levels[0].1 });
            for k in 1..levels.len() {
                let u = uniform(device);
                bloom_down.push(Pass { group: self.group(device, &u, &levels[k - 1].0, &depth_sample, &self.dummy_ao, &self.dummy), uniform: u, target: levels[k].0.clone(), size: levels[k].1 });
            }
            for k in (1..levels.len()).rev() {
                let u = uniform(device);
                bloom_up.push(Pass { group: self.group(device, &u, &levels[k].0, &depth_sample, &self.dummy_ao, &self.dummy), uniform: u, target: levels[k - 1].0.clone(), size: levels[k - 1].1 });
            }
            bloom_view = levels[0].0.clone();
            bloom_texel = [1.0 / levels[0].1 .0 as f32, 1.0 / levels[0].1 .1 as f32];
        }
        let cu = uniform(device);
        let composite_group = self.group(device, &cu, &color, &depth_sample, &ao_view, &bloom_view);
        self.targets = Some(Targets {
            size,
            color_tex,
            color,
            depth,
            depth_sample,
            behind_tex,
            behind_group,
            ssao_depth,
            ssao,
            ssao_blur,
            bloom_pre,
            bloom_down,
            bloom_up,
            composite_group,
            composite_uniform: cu,
            ao_texel: [1.0 / ao_size.0 as f32, 1.0 / ao_size.1 as f32],
            bloom_texel,
        });
    }

    /// Copies the opaque scene for the glass (between the opaque and the
    /// blended surfaces); only `rect` (`[x, y, w, h]`, around the glass).
    pub(crate) fn copy_behind(&self, enc: &mut wgpu::CommandEncoder, rect: [u32; 4]) {
        if let Some(t) = &self.targets {
            if let Some(b) = &t.behind_tex {
                let [x, y, w, h] = rect;
                let (w, h) = (w.min(t.size.0.saturating_sub(x)), h.min(t.size.1.saturating_sub(y)));
                if w == 0 || h == 0 {
                    return;
                }
                let origin = wgpu::Origin3d { x, y, z: 0 };
                enc.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo { origin, ..t.color_tex.as_image_copy() },
                    wgpu::TexelCopyTextureInfo { origin, ..b.as_image_copy() },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
        }
    }

    /// SSAO, bloom and the composite of the HDR scene into `out`.
    /// `mark`: GPU timer marks between the stages.
    pub(crate) fn run(&self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, out: Output, v: ViewParams, mark: &dyn Fn(&mut wgpu::CommandEncoder, &'static str)) {
        let Output { target, viewport, blend } = out;
        let Some(t) = &self.targets else { return };
        let e = self.extras;
        let (samples, strength, radius) = match e.ssao {
            0 => (0.0, 0.0, 0.0),
            // Radius (m) at the game's scale: tanks 3-5 m, rocks, trunks.
            1 => (8.0, 3.2, 3.0),
            _ => (12.0, 3.4, 3.5),
        };
        let strength = strength * e.ao_strength.clamp(0.0, 2.0);
        // (×1.25: the chain starts at 1/4 resolution, one level fewer to sum
        // than the earlier 1/2-resolution chain.)
        let bloom = if e.hdr { e.bloom.clamp(0.0, 1.0) * 1.5 * 1.25 } else { 0.0 };
        let base = PostUniform {
            params: [bloom, if e.hdr { 1.0 } else { 0.0 }, strength, samples],
            proj: [v.near, v.far, v.tan_half_x, v.tan_half_y],
            texel: [0.0, 0.0, radius, v.visible],
            extra: [t.bloom_texel[0], t.bloom_texel[1], 0.0, 0.0],
            fog: [v.fog_distance, v.cam_z, 0.0, 0.0],
            up: [v.up[0], v.up[1], v.up[2], 0.0],
            vols: v.vols,
        };
        let fullscreen = |enc: &mut wgpu::CommandEncoder, pass: &Pass, pipe: &wgpu::RenderPipeline, load: wgpu::LoadOp<wgpu::Color>, texel: [f32; 2]| {
            let mut u = base;
            u.texel[0] = texel[0];
            u.texel[1] = texel[1];
            queue.write_buffer(&pass.uniform, 0, bytemuck::bytes_of(&u));
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &pass.target, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_viewport(0.0, 0.0, pass.size.0 as f32, pass.size.1 as f32, 0.0, 1.0);
            rp.set_pipeline(pipe);
            rp.set_bind_group(0, &pass.group, &[]);
            rp.draw(0..3, 0..1);
        };
        // Every pixel is written: nothing needs loading or clearing.
        let fill = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        if let (Some(d), Some(s), Some(b)) = (&t.ssao_depth, &t.ssao, &t.ssao_blur) {
            let texel = [1.0 / d.size.0 as f32, 1.0 / d.size.1 as f32];
            fullscreen(enc, d, &self.ssao_depth, fill, texel);
            fullscreen(enc, s, &self.ssao, fill, texel);
            fullscreen(enc, b, &self.ssao_blur, fill, texel);
            mark(enc, "ssao");
        }
        if bloom > 0.0 {
            if let Some(pre) = &t.bloom_pre {
                // Taps at ±1 scene texel: four bilinear 2x2 = the 4x4 box.
                fullscreen(enc, pre, &self.pre, fill, [1.0 / t.size.0 as f32, 1.0 / t.size.1 as f32]);
            }
            for d in &t.bloom_down {
                // Source: the previous level (twice this size).
                fullscreen(enc, d, &self.down, fill, [0.5 / d.size.0 as f32, 0.5 / d.size.1 as f32]);
            }
            for u in &t.bloom_up {
                // Source: the smaller level; added into this one.
                let src = ((u.size.0 / 2).max(1), (u.size.1 / 2).max(1));
                fullscreen(enc, u, &self.up, wgpu::LoadOp::Load, [1.0 / src.0 as f32, 1.0 / src.1 as f32]);
            }
            mark(enc, "bloom");
        }
        let mut u = base;
        u.texel[0] = t.ao_texel[0];
        u.texel[1] = t.ao_texel[1];
        queue.write_buffer(&t.composite_uniform, 0, bytemuck::bytes_of(&u));
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let [x, y, w, h] = viewport;
        rp.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        rp.set_scissor_rect(x, y, w, h);
        rp.set_pipeline(if blend { &self.composite.1 } else { &self.composite.0 });
        rp.set_bind_group(0, &t.composite_group, &[]);
        rp.draw(0..3, 0..1);
        drop(rp);
        mark(enc, "composite");
    }
}

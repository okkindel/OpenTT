//! Draws text runs in the UI font and solid pixel rectangles with wgpu: one
//! textured-quad pipeline, alpha blending, scissor per run. The canvas
//! (e.g. 800x600) is scaled uniformly into the target and letterboxed. Text
//! is rasterised at the target's pixel size into a glyph atlas (tt-gui
//! `uifont`), so it stays sharp at any scale.

use tt_gui::uifont::{GlyphAtlas, GlyphQuad, TextRun};

/// A frame of 2D drawing.
pub struct GuiFrame<'a> {
    pub texts: &'a [TextRun],
    /// Canvas (design) size the texts were laid out for.
    pub size: (u32, u32),
    /// Solid rectangles in target pixels (x, y, w, h; RGBA), drawn after
    /// `texts` without scaling: pixel-exact marks such as camera aim's.
    pub pixel_fills: Vec<([u32; 4], [u8; 4])>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

const SHADER: &str = r"
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> };
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@vertex fn vs(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VsOut {
    var o: VsOut;
    o.pos = vec4<f32>(pos, 0.0, 1.0);
    o.uv = uv;
    o.color = color;
    return o;
}
@fragment fn fs(i: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, i.uv) * i.color;
}
";

/// Canvas -> target mapping: uniform scale and offset (letterbox).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub scale: f32,
    pub off: (f32, f32),
}

impl Fit {
    pub fn new(canvas: (u32, u32), target: (u32, u32)) -> Fit {
        let s = (target.0 as f32 / canvas.0.max(1) as f32).min(target.1 as f32 / canvas.1.max(1) as f32);
        Fit { scale: s, off: ((target.0 as f32 - canvas.0 as f32 * s) / 2.0, (target.1 as f32 - canvas.1 as f32 * s) / 2.0) }
    }
}

struct Tex {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    size: (f32, f32),
}

/// Batch texture: the white texel or the glyph atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TexKey {
    White,
    Atlas,
}

pub struct GuiRenderer {
    pipeline: wgpu::RenderPipeline,
    white: Tex,
    srgb: bool,
    atlas: GlyphAtlas,
    atlas_tex: Tex,
    atlas_version: u64,
}

impl GuiRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> GuiRenderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gui"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gui"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gui"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gui"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let srgb = format.is_srgb();
        let white = upload(device, queue, &layout, &sampler, srgb, (1, 1), &[255, 255, 255, 255]);
        let atlas = GlyphAtlas::new();
        let atlas_tex = upload(device, queue, &layout, &sampler, srgb, atlas.image().dimensions(), atlas.image().as_raw());
        let atlas_version = atlas.version();
        GuiRenderer { pipeline, white, srgb, atlas, atlas_tex, atlas_version }
    }

    /// Draws `frame` over the current contents of `view` (`target` = its size in pixels).
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView, target: (u32, u32), frame: &GuiFrame) {
        use wgpu::util::DeviceExt;
        let fit = Fit::new(frame.size, target);
        let (tw, th) = (target.0.max(1) as f32, target.1.max(1) as f32);
        let srgb = self.srgb;
        let color = |c: [u8; 4]| {
            let f = |v| if srgb { tt_render::srgb_to_linear(v as f32 / 255.0) } else { v as f32 / 255.0 };
            [f(c[0]), f(c[1]), f(c[2]), c[3] as f32 / 255.0]
        };
        // Batches: (texture, scissor, vertex range).
        let mut verts: Vec<Vertex> = Vec::new();
        let mut batches: Vec<(TexKey, [u32; 4], std::ops::Range<u32>)> = Vec::new();
        let mut quads: Vec<GlyphQuad> = Vec::new();
        let (atlas_w, atlas_h) = self.atlas_tex.size;
        let mut push = |tex: TexKey, p0: [f32; 2], p1: [f32; 2], uv: [f32; 4], col: [u8; 4], clip: tt_gui::Rect| {
            // Scissor in target pixels.
            let sx0 = (fit.off.0 + clip.x as f32 * fit.scale).floor().clamp(0.0, tw) as u32;
            let sy0 = (fit.off.1 + clip.y as f32 * fit.scale).floor().clamp(0.0, th) as u32;
            let sx1 = (fit.off.0 + clip.right() as f32 * fit.scale).ceil().clamp(0.0, tw) as u32;
            let sy1 = (fit.off.1 + clip.bottom() as f32 * fit.scale).ceil().clamp(0.0, th) as u32;
            if sx1 <= sx0 || sy1 <= sy0 {
                return;
            }
            let scissor = [sx0, sy0, sx1 - sx0, sy1 - sy0];
            let c = color(col);
            let start = verts.len() as u32;
            for (p, t) in [([p0[0], p0[1]], [uv[0], uv[1]]), ([p1[0], p0[1]], [uv[2], uv[1]]), ([p1[0], p1[1]], [uv[2], uv[3]]), ([p0[0], p0[1]], [uv[0], uv[1]]), ([p1[0], p1[1]], [uv[2], uv[3]]), ([p0[0], p1[1]], [uv[0], uv[3]])] {
                verts.push(Vertex { pos: p, uv: t, color: c });
            }
            let end = verts.len() as u32;
            match batches.last_mut() {
                Some((bt, bs, range)) if *bt == tex && *bs == scissor && range.end == start => range.end = end,
                _ => batches.push((tex, scissor, start..end)),
            }
        };
        // Target pixel -> NDC.
        let px = |x: f32, y: f32| [x / tw * 2.0 - 1.0, 1.0 - y / th * 2.0];
        for run in frame.texts {
            quads.clear();
            self.atlas.layout(run, fit.scale, fit.off, &mut quads);
            for q in &quads {
                let (d, s) = (q.dst, q.src);
                let uv = [s.x as f32 / atlas_w, s.y as f32 / atlas_h, s.right() as f32 / atlas_w, s.bottom() as f32 / atlas_h];
                push(TexKey::Atlas, px(d.x as f32, d.y as f32), px(d.right() as f32, d.bottom() as f32), uv, q.color.0, run.clip);
            }
        }
        let whole = tt_gui::Rect::new(0, 0, frame.size.0 as i32, frame.size.1 as i32);
        for &([x, y, w, h], col) in &frame.pixel_fills {
            let (x, y) = (x as f32, y as f32);
            push(TexKey::White, px(x, y), px(x + w as f32, y + h as f32), [0.0, 0.0, 1.0, 1.0], col, whole);
        }
        if self.atlas.version() != self.atlas_version {
            let img = self.atlas.image();
            let (w, h) = img.dimensions();
            queue.write_texture(
                self.atlas_tex.texture.as_image_copy(),
                img.as_raw(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            self.atlas_version = self.atlas.version();
        }
        if verts.is_empty() {
            return;
        }
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gui vertices"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, vb.slice(..));
            for (tex, s, range) in batches {
                let bind = match tex {
                    TexKey::White => &self.white.bind,
                    TexKey::Atlas => &self.atlas_tex.bind,
                };
                pass.set_bind_group(0, bind, &[]);
                pass.set_scissor_rect(s[0], s[1], s[2], s[3]);
                pass.draw(range, 0..1);
            }
        }
        queue.submit([enc.finish()]);
    }
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, srgb: bool, (w, h): (u32, u32), rgba: &[u8]) -> Tex {
    let format = if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
    let size = wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gui texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if w > 0 && h > 0 {
        queue.write_texture(tex.as_image_copy(), rgba, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) }, size);
    }
    let view = tex.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gui texture"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) }],
    });
    Tex { texture: tex, bind, size: (w.max(1) as f32, h.max(1) as f32) }
}

#[cfg(test)]
mod tests {
    use super::Fit;

    #[test]
    fn letterbox() {
        let f = Fit::new((800, 600), (1600, 900));
        assert_eq!(f.scale, 1.5);
        assert_eq!(f.off, (200.0, 0.0));
    }
}

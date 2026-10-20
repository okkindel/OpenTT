//! Drawing the new menus: egui on wgpu (egui-wgpu) over the frame, and the
//! garage's tank preview rendered by tt-render's object view into a texture
//! egui shows.

use crate::engine::Engine;
use tt_render::objectview::{Background, ObjectLight, ObjectView, OrbitCamera};

/// Format of the preview texture (the object view renders into it).
const PREVIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

struct Preview {
    view: ObjectView,
    texture_view: wgpu::TextureView,
    size: [u32; 2],
    id: egui::TextureId,
    camera: OrbitCamera,
}

/// The native in-game HUD (`tt_menu::hud`): its own egui context and
/// renderer, painted over the 3D view and under the menus.
pub struct HudLayer {
    ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
    art: tt_menu::hud::HudArt,
}

impl HudLayer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, assets: &std::path::Path) -> HudLayer {
        let ctx = egui::Context::default();
        tt_menu::theme::install_fonts(&ctx);
        HudLayer { ctx, renderer: egui_wgpu::Renderer::new(device, format, None, 1, false), art: tt_menu::hud::HudArt::new(assets) }
    }

    /// Draws `view` over `target` (`size` pixels; the target is kept) at
    /// HUD size `scale`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView, size: (u32, u32), view: &tt_menu::hud::HudView, time: f64, scale: f32) {
        let ppp = tt_menu::hud::scaled_pixels_per_point([size.0 as f32, size.1 as f32], scale);
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0 as f32 / ppp, size.1 as f32 / ppp))), time: Some(time), ..Default::default() };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
        if (self.ctx.pixels_per_point() - ppp).abs() > 1e-4 {
            self.ctx.set_pixels_per_point(ppp);
        }
        let art = &mut self.art;
        let out = self.ctx.run(raw, |ctx| tt_menu::hud::draw(ctx, view, art));
        paint(&mut self.renderer, &self.ctx, device, queue, target, size, out, "hud");
    }
}

/// Paints egui output over `target` (kept).
#[allow(clippy::too_many_arguments)]
fn paint(renderer: &mut egui_wgpu::Renderer, ctx: &egui::Context, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView, size: (u32, u32), out: egui::FullOutput, label: &str) {
    for (id, delta) in &out.textures_delta.set {
        renderer.update_texture(device, queue, *id, delta);
    }
    let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [size.0, size.1], pixels_per_point: out.pixels_per_point };
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    let extra = renderer.update_buffers(device, queue, &mut enc, &jobs, &screen);
    {
        let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            ..Default::default()
        });
        renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    queue.submit(extra.into_iter().chain([enc.finish()]));
    for id in &out.textures_delta.free {
        renderer.free_texture(id);
    }
}

/// egui context and renderer of the menus.
pub struct MenuLayer {
    pub ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
    preview: Option<Preview>,
}

impl MenuLayer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> MenuLayer {
        let ctx = egui::Context::default();
        tt_menu::theme::install_fonts(&ctx);
        MenuLayer { ctx, renderer: egui_wgpu::Renderer::new(device, format, None, 1, false), preview: None }
    }

    /// Sets the scale for a target of `size` physical pixels (the menus are
    /// laid out in 1280x720 points; see `tt_menu::theme::pixels_per_point`).
    pub fn set_size(&self, size: (u32, u32)) {
        let ppp = tt_menu::theme::pixels_per_point([size.0 as f32, size.1 as f32]);
        if (self.ctx.pixels_per_point() - ppp).abs() > 1e-4 {
            self.ctx.set_pixels_per_point(ppp);
        }
    }

    /// One menu frame (script sync, UI, action).
    pub fn run(&mut self, engine: &mut Engine, raw: egui::RawInput) -> egui::FullOutput {
        crate::menu::run_frame(engine, &self.ctx, raw)
    }

    /// Paints `out` over `target` (`size` pixels; the target is kept).
    pub fn paint(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView, size: (u32, u32), out: egui::FullOutput) {
        paint(&mut self.renderer, &self.ctx, device, queue, target, size, out, "menu");
    }

    /// Renders the tank preview the garage asked for in the last frame and
    /// hands its texture to the menu.
    pub fn render_preview(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, engine: &mut Engine, time: f32) {
        let Some(menu) = engine.menu.as_mut() else { return };
        let Some(req) = menu.data.preview_request.take() else { return };
        let size = [req.size[0].clamp(16, 2048), req.size[1].clamp(16, 2048)];
        if self.preview.as_ref().is_none_or(|p| p.size != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("tank preview"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: PREVIEW_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let texture_view = texture.create_view(&Default::default());
            match self.preview.take() {
                Some(p) => {
                    self.renderer.update_egui_texture_from_wgpu_texture(device, &texture_view, wgpu::FilterMode::Linear, p.id);
                    self.preview = Some(Preview { texture_view, size, ..p });
                }
                None => {
                    let id = self.renderer.register_native_texture(device, &texture_view, wgpu::FilterMode::Linear);
                    let mut view = ObjectView::new(device, queue, PREVIEW_FORMAT);
                    view.background = Background::Color([0.0, 0.0, 0.0, 0.0]);
                    let camera = OrbitCamera { distance: 6.0, ..Default::default() };
                    self.preview = Some(Preview { view, texture_view, size, id, camera });
                }
            }
        }
        let Some(p) = self.preview.as_mut() else { return };
        p.camera.spin_frame();
        p.view.time = time;
        let inst = ObjectView::tank_preview(&req.shape, &req.brain);
        let center = tt_render::objectview::shape_center(&req.shape).unwrap_or_default();
        let cam = p.camera.camera(center);
        p.view.render(device, queue, &p.texture_view, size[0], size[1], None, &inst, &cam, &ObjectLight::default());
        menu.data.preview_texture = Some((req.index, p.id));
    }
}

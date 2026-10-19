//! Offscreen preview of the first-run screen ("Locate your Think Tanks
//! installation"), drawn with no game data at all (egui + egui-wgpu on a
//! headless GPU). The PNG goes to `$TT_SETUP_PREVIEW` (a file path), else
//! `<tmp>/tt-menu-preview/setup.png`; with `TT_SETUP_PREVIEW_STAGE=progress`
//! the import progress is drawn instead. Skipped without a GPU adapter.

use std::path::PathBuf;
use tt_menu::{theme, SetupMode, SetupProgress, SetupState};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[test]
fn setup_screen_preview() {
    let instance = wgpu::Instance::default();
    let Some(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no GPU adapter; skipping the preview");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default(), None)) else {
        eprintln!("no GPU device; skipping the preview");
        return;
    };
    let mut st = SetupState::new(SetupMode::FirstRun);
    st.path = "/home/player/Games/Think Tanks".into();
    st.target = format!("/home/player/.local/share/{}/gamedata", tt_assets::product::DATA_DIR);
    st.error = Some("No Think Tanks installation in /home/player/Games/Think Tanks: it has no \"game\" folder. Choose the folder Think Tanks was installed to (it holds the \"game\" and \"common\" folders).".into());
    if std::env::var("TT_SETUP_PREVIEW_STAGE").as_deref() == Ok("progress") {
        st.error = None;
        st.progress = Some(SetupProgress { phase: "Converting textures".into(), done: 212, total: 467, file: "models/tanks/tank01.png".into() });
    }
    for size in [[1280u32, 720u32], [1600, 900], [1920, 1080], [1280, 800]] {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply_style(&ctx);
    let ppp = theme::pixels_per_point([size[0] as f32, size[1] as f32]);
    let mut renderer = egui_wgpu::Renderer::new(&device, FORMAT, None, 1, false);
    let mut output = None;
    for time in [0.0, 0.0, 1.0] {
        let input = egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp))),
            max_texture_side: Some(device.limits().max_texture_dimension_2d as usize),
            ..Default::default()
        };
        ctx.set_pixels_per_point(ppp);
        let out = ctx.run(input, |ctx| {
            let _ = st.ui(ctx);
        });
        for (id, delta) in &out.textures_delta.set {
            renderer.update_texture(&device, &queue, *id, delta);
        }
        output = Some(out);
    }
    let out = output.unwrap();
    let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point: out.pixels_per_point };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("setup-preview"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut enc = device.create_command_encoder(&Default::default());
    let extra = renderer.update_buffers(&device, &queue, &mut enc, &jobs, &screen);
    {
        let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            ..Default::default()
        });
        renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    let (w, h) = (size[0], size[1]);
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    enc.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit(extra.into_iter().chain([enc.finish()]));
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let data = buf.slice(..).get_mapped_range();
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let s = (y * row) as usize;
        px.extend_from_slice(&data[s..s + (w * 4) as usize]);
    }
    let img = image::RgbaImage::from_raw(w, h, px).unwrap();
    // Something was drawn: the screen is not one flat colour.
    let first = *img.get_pixel(0, 0);
    assert!(img.pixels().any(|p| *p != first), "the setup screen drew nothing");
    // 1280x720 at the path given; other sizes beside it (`setup_<w>x<h>.png`).
    let mut path = std::env::var_os("TT_SETUP_PREVIEW").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("tt-menu-preview").join("setup.png"));
    if size != [1280, 720] {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("setup").to_string();
        path.set_file_name(format!("{stem}_{}x{}.png", size[0], size[1]));
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).unwrap();
    }
    img.save(&path).unwrap();
    eprintln!("setup screen preview: {}", path.display());
    }
}

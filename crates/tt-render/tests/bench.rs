//! Offscreen render benchmark (no window, no audio) on the asset directory; run with
//! `cargo test --release -p tt-render --test bench -- --ignored --nocapture`.

use glam::Vec3;
use std::path::PathBuf;
use std::time::Instant;
use tt_render::{Camera, Renderer, Scene};

fn gpu() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let name = format!("{:?}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None)).ok()?;
    Some((device, queue, name))
}

/// Milliseconds per frame (GPU-complete) over `frames` frames.
fn time_frames(device: &wgpu::Device, queue: &wgpu::Queue, r: &mut Renderer, view: &wgpu::TextureView, (w, h): (u32, u32), cam: &Camera, frames: u32) -> f64 {
    for i in 0..5 {
        r.render(device, queue, view, w, h, cam, i as f32 * 0.016);
    }
    device.poll(wgpu::Maintain::Wait);
    let start = Instant::now();
    for i in 0..frames {
        r.render(device, queue, view, w, h, cam, i as f32 * 0.016);
        device.poll(wgpu::Maintain::Wait);
    }
    start.elapsed().as_secs_f64() * 1000.0 / frames as f64
}

#[test]
#[ignore]
fn render_benchmark() {
    let root = tt_assets::layout::default_dir();
    let mission = root.join("missions/tt1_1.ron");
    let Some((device, queue, name)) = gpu() else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let scene = Scene::load(&root, &mission).unwrap();
    let (w, h) = (1600, 1200);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    // A typical chase view over the terrain with shapes in sight.
    let cam = Camera { pos: Vec3::new(150.0, -135.0, 125.0), yaw: 40f32.to_radians(), pitch: -12f32.to_radians() };
    // 20 shadowed tanks spread ahead of the camera, and 100 smoke sprites.
    let tank = root.join("models/tanks/tank02.dts");
    let fwd = Vec3::new(cam.yaw.sin(), cam.yaw.cos(), 0.0);
    let right = Vec3::new(fwd.y, -fwd.x, 0.0);
    let tanks: Vec<tt_render::DynamicInstance> = (0..20)
        .map(|i| {
            let p = cam.pos + fwd * (15.0 + (i / 5) as f32 * 8.0) + right * ((i % 5) as f32 * 6.0 - 12.0);
            let p = Vec3::new(p.x, p.y, scene.terrain_height(p.x, p.y));
            tt_render::DynamicInstance { shape: tank.clone(), transform: glam::Mat4::from_translation(p), light: 0.5, ..Default::default() }
        })
        .collect();
    let sprites: Vec<tt_render::Sprite> = (0..100)
        .map(|i| tt_render::Sprite {
            texture: root.join("models/effects/smoke.png"),
            pos: cam.pos + fwd * (20.0 + i as f32 * 0.3) + Vec3::Z * ((i % 7) as f32 * 0.4 - 8.0),
            size: 2.0,
            color: [0.5, 0.4, 0.3, 0.6],
            ..Default::default()
        })
        .collect();
    let shadows = vec![true; tanks.len()];
    let mut r = Renderer::new(&device, &queue, format, &scene);
    let empty = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 60);
    r.set_dynamic(&device, &queue, &tanks);
    let tanks_ms = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 60);
    r.set_dynamic_shadowed(&device, &queue, &tanks, &shadows);
    let shadow_ms = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 60);
    r.set_sprites(&sprites);
    let full = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 60);
    eprintln!("[{name}] {w}x{h}: scene {empty:.2} | +20 tanks {tanks_ms:.2} | +shadows {shadow_ms:.2} | +100 sprites {full:.2} ms/frame");
    for scale in [0.75, 0.5] {
        r.set_render_scale(scale);
        let ms = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 60);
        eprintln!("[{name}] full load, render scale {scale}: {ms:.2} ms/frame");
    }
    for aniso in [4u16, 1] {
        let mut r = Renderer::new_with_settings(&device, &queue, format, &scene, tt_render::RenderSettings { render_scale: 1.0, anisotropy: aniso, ..Default::default() });
        r.set_dynamic_shadowed(&device, &queue, &tanks, &shadows);
        r.set_sprites(&sprites);
        let ms = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 60);
        eprintln!("[{name}] full load, anisotropy {aniso}: {ms:.2} ms/frame");
    }
    // Optional snapshot for eyeballing: TT_RENDER_OUT=<dir>.
    if let Some(dir) = std::env::var_os("TT_RENDER_OUT") {
        let dir = PathBuf::from(dir);
        let mut r = Renderer::new(&device, &queue, format, &scene);
        r.set_dynamic_shadowed(&device, &queue, &tanks, &shadows);
        let mut flat = sprites[..3].to_vec();
        // Reticle-style quads: one upright across the view, one on the ground.
        let p0 = tanks[2].transform.w_axis.truncate() + fwd * 3.0;
        let n = Vec3::Z;
        flat.push(tt_render::Sprite { texture: root.join("models/reticles/bouncereticle.png"), pos: p0 + Vec3::Z, size: 2.0, orient: tt_render::SpriteOrient::Axis { dir: fwd }, ..Default::default() });
        flat.push(tt_render::Sprite {
            texture: root.join("models/reticles/bouncereticle.png"),
            pos: Vec3::new(p0.x + 3.0, p0.y, scene.terrain_height(p0.x + 3.0, p0.y)),
            size: 3.0,
            orient: tt_render::SpriteOrient::Plane { normal: n, up: fwd },
            depth_offset: 0.05,
            ..Default::default()
        });
        r.set_sprites(&flat);
        for scale in [1.0, 0.5] {
            r.set_render_scale(scale);
            let path = dir.join(format!("bench_scale{scale}.png"));
            snapshot(&device, &queue, &mut r, &cam, path);
        }
    }
}

fn snapshot(device: &wgpu::Device, queue: &wgpu::Queue, r: &mut Renderer, cam: &Camera, path: PathBuf) {
    let (w, h) = (960u32, 540u32);
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    r.render(device, queue, &tex.create_view(&Default::default()), w, h, cam, 0.5);
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let data = buf.slice(..).get_mapped_range();
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        px.extend_from_slice(&data[(y * row) as usize..][..(w * 4) as usize]);
    }
    image::save_buffer(&path, &px, w, h, image::ColorType::Rgba8).unwrap();
    eprintln!("wrote {}", path.display());
}

/// Soft particles: a Mortar explosion (`MortarExplosion`: volcanoexplosion.png
/// growing to 41 m) on flat ground from a chase camera, with the graphics
/// extras off and on (both must render without validation errors); the
/// extras view is saved to `TT_SOFT_OUT` (a .png path) for eyeballing:
/// `TT_SOFT_OUT=out/soft-particles.png cargo test -p tt-render --test bench soft_particles_shot -- --ignored`.
#[test]
#[ignore]
fn soft_particles_shot() {
    let root = tt_assets::layout::default_dir();
    let Some((device, queue, _)) = gpu() else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let scene = Scene::load(&root, &root.join("missions/tt1_1.ron")).unwrap();
    // The flattest strip (camera 32 m south of the explosion) in the
    // playfield.
    let mut best = (f32::MAX, Vec3::ZERO);
    for gx in (-150..150).step_by(10) {
        for gy in (-150..150).step_by(10) {
            let (x, y) = (gx as f32, gy as f32);
            let h: Vec<f32> = (-3..=3).flat_map(|i| (-7..=3).map(move |j| (x + i as f32 * 5.0, y + j as f32 * 5.0))).map(|(a, b)| scene.terrain_height(a, b)).collect();
            let spread = h.iter().cloned().fold(f32::MIN, f32::max) - h.iter().cloned().fold(f32::MAX, f32::min);
            if spread < best.0 {
                best = (spread, Vec3::new(x, y, scene.terrain_height(x, y)));
            }
        }
    }
    let at = best.1;
    let cam_pos = at + Vec3::new(0.0, -32.0, 0.0);
    let cam_pos = Vec3::new(cam_pos.x, cam_pos.y, scene.terrain_height(cam_pos.x, cam_pos.y) + 5.0);
    let d = (at + Vec3::Z * 6.0 - cam_pos).normalize();
    let cam = Camera { pos: cam_pos, yaw: d.x.atan2(d.y), pitch: d.z.asin() };
    eprintln!("ground spread {:.2} m at {at}", best.0);
    // ~60 % into the 550 ms explosion.
    let boom = tt_render::Sprite { texture: root.join("models/volcano/volcanoexplosion.png"), pos: at, size: 25.0, ..Default::default() };
    let out = std::env::var_os("TT_SOFT_OUT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("tt-soft-particles.png"));
    for extras in [tt_render::post::Extras::default(), tt_render::post::Extras::modern()] {
        let settings = tt_render::RenderSettings { extras, ..Default::default() };
        let mut r = Renderer::new_with_settings(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, &scene, settings);
        r.set_sprites(std::slice::from_ref(&boom));
        snapshot(&device, &queue, &mut r, &cam, out.clone());
        // Cost at 1920×1080: the explosion and 200 smoke puffs around it
        // (heavy overdraw) against no sprites.
        let (w, h) = (1920, 1080);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut load = vec![boom.clone()];
        load.extend((0..200).map(|i| tt_render::Sprite {
            texture: root.join("models/effects/smoke.png"),
            pos: at + Vec3::new((i % 20) as f32 * 0.8 - 8.0, (i / 20) as f32 * 0.8 - 4.0, (i % 3) as f32 * 0.5),
            size: 4.0,
            color: [0.5, 0.4, 0.3, 0.5],
            ..Default::default()
        }));
        r.set_sprites(&[]);
        let none = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 120);
        r.set_sprites(&load[..1]);
        let one = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 120);
        r.set_sprites(&load);
        let with = time_frames(&device, &queue, &mut r, &view, (w, h), &cam, 120);
        eprintln!("post {}: no sprites {none:.2} ms, explosion {one:.2} ms, explosion + 200 smoke {with:.2} ms/frame", extras.post());
    }
}

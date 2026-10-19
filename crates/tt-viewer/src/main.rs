//! Think Tanks mission viewer.
//!
//! ```text
//! tt-viewer [--assets DIR] <MISSION> [--screenshot out.png] [--size WxH]
//!           [--camera X Y Z YAW_DEG PITCH_DEG] [--time SECONDS] [--markers]
//! ```
//! Shows a mission (an id, `tt1_1`, or its `.ron` file) as a static scene
//! (terrain, sky, shapes; only the defs the mission declares itself). `opentt
//! --mission tt1_1` shows it with the game's defs. `--assets` defaults to the
//! asset directory (`$TT_ASSETS`, else `./assets`, else the repository's).
//! Window controls: WASD move, Q/E down/up, Shift faster, right mouse drag or
//! arrow keys to look around.


use glam::Vec3;
use tt_render::{Camera, Renderer, Scene};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

struct Args {
    root: PathBuf,
    mission: PathBuf,
    screenshot: Option<PathBuf>,
    size: (u32, u32),
    camera: Option<[f32; 5]>,
    /// Scene time for screenshots (seconds).
    time: f32,
    /// Show boxes for invisible objects (spawn points etc.).
    markers: bool,
    /// Directory with upscaled textures mirroring the game tree.
    /// Hidden: place tank shapes in front of the camera as dynamic instances
    /// and draw an inset preview viewport (renderer test).
    dynamic_test: bool,
    /// Hidden: tank-selection style object views over the scene.
    object_view_test: bool,
    /// Hidden: one tank with its dynamic shadow 14 m ahead of the camera.
    shadow_test: bool,
    /// Hidden: tread animation test (two tanks and a scrum ball near the camera).
    tread_test: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut root = tt_assets::layout::default_dir();
    let (mut mission, mut screenshot, mut size, mut camera, mut time, mut markers) = (None, None, (1280, 720), None, 0.0, false);
    let mut dynamic_test = false;
    let mut object_view_test = false;
    let mut shadow_test = false;
    let mut tread_test = false;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--assets" => root = it.next().ok_or("--assets needs a value")?.into(),
            "--screenshot" => screenshot = Some(it.next().ok_or("--screenshot needs a path")?.into()),
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--markers" => markers = true,
            "--dynamic-test" => dynamic_test = true,
            "--object-view-test" => object_view_test = true,
            "--shadow-test" => shadow_test = true,
            "--tread-test" => tread_test = true,
            "--time" => time = it.next().and_then(|s| s.parse().ok()).ok_or("--time needs seconds")?,
            "--camera" => {
                let mut c = [0.0; 5];
                for v in &mut c {
                    *v = it.next().and_then(|s| s.parse().ok()).ok_or("--camera needs X Y Z YAW PITCH")?;
                }
                camera = Some(c);
            }
            _ if mission.is_none() => mission = Some(PathBuf::from(a)),
            _ => return Err(format!("unexpected argument {a}")),
        }
    }
    let mission: PathBuf = mission.ok_or("usage: tt-viewer [--assets DIR] <MISSION (tt1_1) | FILE.ron> [--screenshot out.png]")?;
    // A mission id (`tt1_1`) or its `.ron` file.
    let mission = if mission.extension().is_none() && mission.components().count() == 1 { root.join(tt_assets::layout::mission(&mission.to_string_lossy())) } else { mission };
    Ok(Args { root, mission, screenshot, size, camera, time, markers, dynamic_test, object_view_test, shadow_test, tread_test })
}

fn initial_camera(scene: &Scene, given: Option<[f32; 5]>) -> Camera {
    if let Some([x, y, z, yaw, pitch]) = given {
        return Camera { pos: Vec3::new(x, y, z), yaw: yaw.to_radians(), pitch: pitch.to_radians() };
    }
    // Above the first spawn point, looking at the map centre.
    let spawn = scene.spawn.unwrap_or(Vec3::new(0.0, -150.0, 100.0));
    let pos = spawn + Vec3::new(0.0, 0.0, 40.0);
    let to_centre = Vec3::new(-pos.x, -pos.y, 0.0);
    Camera { pos, yaw: to_centre.x.atan2(to_centre.y), pitch: -0.3 }
}

async fn gpu(compatible: Option<&wgpu::Surface<'_>>, instance: &wgpu::Instance) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), String> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: compatible, ..Default::default() })
        .await
        .ok_or("no GPU adapter")?;
    let (device, queue) = adapter.request_device(&Default::default(), None).await.map_err(|e| e.to_string())?;
    Ok((adapter, device, queue))
}

fn screenshot(args: &Args, scene: &Scene) -> Result<(), String> {
    let instance = wgpu::Instance::default();
    let (_, device, queue) = pollster::block_on(gpu(None, &instance))?;
    let (w, h) = args.size;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut renderer = Renderer::new(&device, &queue, format, scene);
    let camera = initial_camera(scene, args.camera);
    let view = target.create_view(&Default::default());
    if args.shadow_test {
        let fwd = Vec3::new(camera.yaw.sin(), camera.yaw.cos(), 0.0);
        let p = camera.pos + fwd * 14.0;
        let pos = Vec3::new(p.x, p.y, scene.terrain_height(p.x, p.y));
        let slope = (scene.terrain_height(p.x + 2.0, p.y) - scene.terrain_height(p.x - 2.0, p.y)).hypot(scene.terrain_height(p.x, p.y + 2.0) - scene.terrain_height(p.x, p.y - 2.0)) / 4.0;
        eprintln!("shadow test: tank at {pos:?}, slope {:.1} deg", slope.atan().to_degrees());
        let tank = tt_render::DynamicInstance {
            shape: args.root.join("models/tanks/tank02.dts"),
            transform: glam::Mat4::from_rotation_translation(glam::Quat::from_rotation_z(-(camera.yaw + 2.2)), pos),
            light: scene.shape_light(pos),
            ..Default::default()
        };
        renderer.set_dynamic_shadowed(&device, &queue, &[tank], &[true]);
    }
    if args.tread_test {
        // Left: tank02 in the default pose. Right: tank02 with the tread
        // sequences at `--time` (IFL frames), its front wheels raised 0.4 m
        // (skinned treads follow) and a scrum ball playing `ambient`.
        use tt_render::pose;
        let fwd = Vec3::new(camera.yaw.sin(), camera.yaw.cos(), 0.0);
        let right = Vec3::new(fwd.y, -fwd.x, 0.0);
        let tank = args.root.join("models/tanks/tank02.dts");
        let ball = args.root.join("models/items/scrumball.dts");
        let place = |side: f32, ahead: f32, up: f32| {
            let p = camera.pos + fwd * ahead + right * side;
            Vec3::new(p.x, p.y, scene.terrain_height(p.x, p.y) + up)
        };
        // Side view: tanks face along the camera's right axis.
        let rot = glam::Quat::from_rotation_z(-(camera.yaw + std::f32::consts::FRAC_PI_2));
        let mut instances = Vec::new();
        let mut options = Vec::new();
        for (i, side) in [-2.5f32, 2.5].into_iter().enumerate() {
            let pos = place(side, 7.0, 0.0);
            let mut inst = tt_render::DynamicInstance { shape: tank.clone(), transform: glam::Mat4::from_rotation_translation(rot, pos), light: 1.0, ..Default::default() };
            let mut opt = tt_render::InstanceOptions::default();
            if i == 1 {
                for wheel in ["inner_LF", "outer_LF", "inner_RF", "outer_RF"] {
                    if let Some(m) = pose::shape(&tank).and_then(|s| pose::node_default(&s, wheel)) {
                        inst.nodes.push((wheel.into(), glam::Mat4::from_translation(Vec3::Z * 0.4) * m));
                    }
                }
                opt.ifl = pose::sequence_ifl(&tank, "LTread", args.time);
                opt.ifl.extend(pose::sequence_ifl(&tank, "RTread", args.time));
                eprintln!("tread test: ifl {:?}", opt.ifl);
            }
            instances.push(inst);
            options.push(opt);
        }
        let pos = place(0.0, 14.0, 0.5);
        instances.push(tt_render::DynamicInstance {
            shape: ball.clone(),
            transform: glam::Mat4::from_translation(pos),
            light: 1.0,
            nodes: pose::sequence_pose(&ball, "ambient", args.time),
            ..Default::default()
        });
        options.push(tt_render::InstanceOptions { ifl: pose::sequence_ifl(&ball, "ambient", args.time), ..Default::default() });
        renderer.set_dynamic_with(&device, &queue, &instances, &options);
    }
    if args.dynamic_test {
        dynamic_test(&device, &queue, &mut renderer, &view, (w, h), args, scene, &camera);
    } else {
        renderer.render(&device, &queue, &view, w, h, &camera, args.time);
    }
    if args.object_view_test {
        object_view_test(&device, &queue, &view, (w, h), args);
    }

    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let data = buf.slice(..).get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    let out = args.screenshot.as_ref().unwrap();
    image::save_buffer(out, &pixels, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    println!("saved {} (camera {:.1} {:.1} {:.1})", out.display(), camera.pos.x, camera.pos.y, camera.pos.z);
    Ok(())
}

/// Six tanks on the terrain 25 m ahead of the camera (turrets turned,
/// barrels raised, wheels spun; tank04/05 with their wheels shown; the last
/// one in its `death1` pose), a power-up playing `ambient`, then an inset
/// view of the first tank in the top-right corner.
#[allow(clippy::too_many_arguments)]
fn dynamic_test(device: &wgpu::Device, queue: &wgpu::Queue, renderer: &mut Renderer, view: &wgpu::TextureView, (w, h): (u32, u32), args: &Args, scene: &Scene, camera: &Camera) {
    use glam::{Mat4, Quat};
    use tt_render::pose;
    let fwd = Vec3::new(camera.yaw.sin(), camera.yaw.cos(), 0.0);
    let right = Vec3::new(fwd.y, -fwd.x, 0.0);
    let tanks = ["tank01", "tank02", "tank03", "tank04", "tank05", "tank01"];
    let mut instances = Vec::new();
    for (i, name) in tanks.iter().enumerate() {
        let p = camera.pos + fwd * 25.0 + right * ((i as f32 - 2.5) * 6.0);
        let pos = Vec3::new(p.x, p.y, scene.terrain_height(p.x, p.y));
        // Face the camera, each turned a bit more.
        let yaw = camera.yaw + std::f32::consts::PI + i as f32 * 0.5;
        let path = args.root.join(format!("models/tanks/{name}.dts"));
        let mut inst = tt_render::DynamicInstance {
            shape: path.clone(),
            transform: Mat4::from_rotation_translation(Quat::from_rotation_z(-yaw), pos),
            light: scene.shape_light(pos),
            ..Default::default()
        };
        if let Some(shape) = pose::shape(&path) {
            let turn = |node: &str, r: Mat4| pose::node_default(&shape, node).map(|m| (node.to_string(), m * r));
            if i == 5 {
                inst.nodes = pose::sequence_pose_of(&shape, "death1", 0.25);
            } else {
                inst.nodes.extend(turn("codeTurret", Mat4::from_rotation_z(0.6 + i as f32 * 0.4)));
                inst.nodes.extend(turn("codeWeapon", Mat4::from_rotation_x(0.35)));
                for n in shape.nodes.iter().filter(|n| n.name.ends_with("_rot")) {
                    inst.nodes.extend(turn(&n.name, Mat4::from_rotation_x(0.7)));
                }
            }
            // Wheels of tank04/05 are hidden by default.
            inst.shown = shape
                .objects
                .iter()
                .filter(|o| o.default_visibility <= 0.0 && !o.name.eq_ignore_ascii_case("muzzleflash"))
                .map(|o| o.name.clone())
                .collect();
        }
        instances.push(inst);
    }
    // A power-up mid-way through its ambient sequence, between tanks 2 and 3.
    let p = camera.pos + fwd * 20.0;
    let pos = Vec3::new(p.x, p.y, scene.terrain_height(p.x, p.y) + 1.0);
    let powerup = args.root.join("models/items/powerup.dts");
    let t = args.time + 0.6;
    let vis = pose::sequence_visibility(&powerup, "ambient", t);
    instances.push(tt_render::DynamicInstance {
        nodes: pose::sequence_pose(&powerup, "ambient", t),
        hidden: vis.iter().filter(|v| !v.1).map(|v| v.0.clone()).collect(),
        shown: vis.iter().filter(|v| v.1).map(|v| v.0.clone()).collect(),
        shape: powerup,
        transform: Mat4::from_translation(pos),
        light: scene.shape_light(pos),
    });
    // Timing: 20 posed tanks + 50 power-ups (stand-ins for projectiles).
    let load: Vec<_> = (0..20).map(|i| instances[i % 6].clone()).chain((0..50).map(|_| instances[6].clone())).collect();
    renderer.set_dynamic(device, queue, &load);
    let started = Instant::now();
    for _ in 0..100 {
        renderer.set_dynamic(device, queue, &load);
    }
    eprintln!("set_dynamic: {:.1} us per call for {} instances", started.elapsed().as_secs_f64() * 1e4, load.len());
    // Projected shadows under the tanks (not the power-up).
    let shadows: Vec<bool> = (0..instances.len()).map(|i| i < tanks.len()).collect();
    renderer.set_dynamic_shadowed(device, queue, &instances, &shadows);

    // Effects at scene time `args.time`: smoke from tanks 0-2 (`smoke1`, the
    // damage emitter node), rocket trails from the muzzles of tanks 3-4,
    // explosions started so that they are half-way through now, and two
    // sprite projectiles in flight.
    let mut fx = tt_render::fx::Effects::new(scene.fx.clone());
    let node_pos = |i: usize, node: &str| {
        let inst: &tt_render::DynamicInstance = &instances[i];
        pose::shape(&inst.shape).and_then(|s| pose::node_transform(&s, &inst.nodes, node)).map(|m| inst.transform * m)
    };
    for i in 0..5 {
        let (emitter, node) = if i < 3 { ("SmokeEmitter", "smoke1") } else { ("RocketEmitter", "codeMuzzle") };
        if let Some(m) = node_pos(i, node) {
            // Emitter axis up, like a node emitter.
            fx.add_emitter(emitter, Mat4::from_translation(m.w_axis.truncate()));
        }
    }
    let ahead = |d: f32, side: f32, up: f32| {
        let p = camera.pos + fwd * d + right * side;
        Vec3::new(p.x, p.y, scene.terrain_height(p.x, p.y) + up)
    };
    let booms = [
        ("DefaultTankExplosion", ahead(18.0, -9.0, 1.0)),
        ("bounceExplosion", ahead(18.0, -4.0, 1.0)),
        ("SplashExplosion", ahead(18.0, 4.0, 1.0)),
        ("DefaultExplosion", ahead(14.0, 8.0, 1.0)),
        ("RockExplosion", ahead(60.0, 20.0, 4.0)),
        ("BounceBump", ahead(14.0, 0.0, 1.0)),
    ];
    let lives: Vec<f32> = booms
        .iter()
        .map(|(n, _)| {
            let e = scene.fx.explosion(n);
            let dur = e.and_then(|e| match &e.resource {
                tt_render::fx::ExplosionResource::Shape(p) => pose::shape(p).and_then(|s| s.sequences.first().map(|q| q.duration)),
                _ => None,
            });
            e.map_or(0.0, |e| dur.unwrap_or(1.0) * e.duration_ms)
        })
        .collect();
    let total = (args.time * 1000.0) as u32;
    let mut started = vec![false; booms.len()];
    let mut t = 0;
    while t < total {
        for (k, (name, pos)) in booms.iter().enumerate() {
            if !started[k] && t as f32 >= total as f32 - lives[k] * 0.5 {
                started[k] = fx.explode(name, *pos);
            }
        }
        let step = 32.min(total - t);
        fx.advance(step);
        t += step;
    }
    eprintln!("effects: {} particles, explosions started: {:?}", fx.particle_count(), booms.iter().zip(&started).map(|(b, s)| (b.0, *s)).collect::<Vec<_>>());
    renderer.set_effects(device, queue, &fx);
    let shots = [("DefaultProjectile", 0.4, ahead(12.0, -2.0, 2.5)), ("SpeedyProjectile", 0.6, ahead(12.0, 2.0, 2.5))];
    let sprites: Vec<tt_render::Sprite> = shots
        .iter()
        .map(|(n, size, pos)| tt_render::Sprite { texture: args.root.join(format!("models/projectiles/{n}.png")), pos: *pos, size: *size, ..Default::default() })
        .collect();
    renderer.set_sprites(&sprites);
    renderer.render(device, queue, view, w, h, camera, args.time);
    // Inset: from 6 m in front of the first tank, looking back at it.
    let t = instances[0].transform.w_axis.truncate();
    let eye = t + fwd * -6.0 + Vec3::Z * 3.0;
    let look = Mat4::look_to_rh(eye, (t + Vec3::Z - eye).normalize(), Vec3::Z).inverse();
    // look_to_rh's camera looks along -Z.
    let m = Mat4::from_cols(look.x_axis, -look.z_axis, look.y_axis, look.w_axis);
    let inset = tt_render::View {
        camera: Camera::from_matrix(m),
        fov_deg: tt_render::vertical_fov(90.0, 4.0 / 3.0),
        viewport: Some([w - w / 3 - 8, 8, w / 3, w / 4]),
        clear: true,
    };
    renderer.render_view(device, queue, view, w, h, &inset, args.time);
}

/// The four tank-selection previews (tank01/02/03/05 with their brain jars,
/// camera distance 6) in a 2×2 grid over the scene: unlit with the scene
/// showing through, unlit on a solid background, lit by a directional
/// light, and the first one again 300 frames of spin later.
fn object_view_test(device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView, (w, h): (u32, u32), args: &Args) {
    use tt_render::objectview::{shape_center, Background, ObjectLight, ObjectView, OrbitCamera};
    let mut ov = ObjectView::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    ov.time = args.time;
    let tanks = [("tank01", "brainjar01"), ("tank02", "brainjar02"), ("tank03", "brainjar03"), ("tank05", "brainjar04")];
    let dir = args.root.join("models/tanks");
    let (cw, ch) = (w / 2 - 12, h / 2 - 12);
    for (i, (tank, brain)) in tanks.iter().enumerate() {
        let tank = dir.join(format!("{tank}.dts"));
        let instances = ObjectView::tank_preview(&tank, &dir.join(format!("{brain}.dts")));
        let mut orbit = OrbitCamera { distance: 6.0, ..Default::default() };
        let mut light = ObjectLight::default();
        ov.background = Background::Keep;
        match i {
            1 => ov.background = Background::Color([0.15, 0.2, 0.35, 1.0]),
            2 => light = ObjectLight { direction: Vec3::new(0.5, 0.6, -0.6), color: [0.8, 0.8, 0.8], ambient: [0.4, 0.4, 0.4] },
            3 => (0..300).for_each(|_| orbit.spin_frame()),
            _ => {}
        }
        let camera = orbit.camera(shape_center(&tank).unwrap_or(Vec3::ZERO));
        let vp = [8 + (i as u32 % 2) * (w / 2), 8 + (i as u32 / 2) * (h / 2), cw, ch];
        ov.render(device, queue, view, w, h, Some(vp), &instances, &camera, &light);
    }
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

struct App {
    scene: Scene,
    camera: Camera,
    gfx: Option<Gfx>,
    keys: HashSet<KeyCode>,
    looking: bool,
    last: Instant,
    start: Instant,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let title = format!("{} viewer - {}", tt_assets::product::NAME, self.scene.name);
        let window = Arc::new(el.create_window(Window::default_attributes().with_title(title)).expect("window"));
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).expect("surface");
        let (adapter, device, queue) = pollster::block_on(gpu(Some(&surface), &instance)).expect("gpu");
        let size = window.inner_size();
        let config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        surface.configure(&device, &config);
        let renderer = Renderer::new(&device, &queue, config.format, &self.scene);
        self.gfx = Some(Gfx { window, surface, device, queue, config, renderer });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                gfx.config.width = s.width.max(1);
                gfx.config.height = s.height.max(1);
                gfx.surface.configure(&gfx.device, &gfx.config);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(k) = event.physical_key {
                    if k == KeyCode::Escape {
                        el.exit();
                    }
                    if event.state == ElementState::Pressed {
                        self.keys.insert(k);
                    } else {
                        self.keys.remove(&k);
                    }
                }
            }
            WindowEvent::MouseInput { button: MouseButton::Right, state, .. } => {
                self.looking = state == ElementState::Pressed;
            }
            WindowEvent::RedrawRequested => {
                let dt = self.last.elapsed().as_secs_f32().min(0.1);
                self.last = Instant::now();
                let speed = if self.keys.contains(&KeyCode::ShiftLeft) { 200.0 } else { 50.0 } * dt;
                let fwd = self.camera.forward();
                let right = fwd.cross(Vec3::Z).normalize_or_zero();
                for (k, d) in [
                    (KeyCode::KeyW, fwd),
                    (KeyCode::KeyS, -fwd),
                    (KeyCode::KeyD, right),
                    (KeyCode::KeyA, -right),
                    (KeyCode::KeyE, Vec3::Z),
                    (KeyCode::KeyQ, -Vec3::Z),
                ] {
                    if self.keys.contains(&k) {
                        self.camera.pos += d * speed;
                    }
                }
                let turn = 1.5 * dt;
                for (k, yaw, pitch) in [
                    (KeyCode::ArrowLeft, -turn, 0.0),
                    (KeyCode::ArrowRight, turn, 0.0),
                    (KeyCode::ArrowUp, 0.0, turn),
                    (KeyCode::ArrowDown, 0.0, -turn),
                ] {
                    if self.keys.contains(&k) {
                        self.camera.yaw += yaw;
                        self.camera.pitch = (self.camera.pitch + pitch).clamp(-1.5, 1.5);
                    }
                }
                match gfx.surface.get_current_texture() {
                    Ok(frame) => {
                        let view = frame.texture.create_view(&Default::default());
                        gfx.renderer.render(&gfx.device, &gfx.queue, &view, gfx.config.width, gfx.config.height, &self.camera, self.start.elapsed().as_secs_f32());
                        frame.present();
                    }
                    Err(_) => gfx.surface.configure(&gfx.device, &gfx.config),
                }
                gfx.window.request_redraw();
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent) {
        if let (true, DeviceEvent::MouseMotion { delta }) = (self.looking, event) {
            self.camera.yaw += delta.0 as f32 * 0.003;
            self.camera.pitch = (self.camera.pitch - delta.1 as f32 * 0.003).clamp(-1.5, 1.5);
        }
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let scene = match Scene::load(&args.root, &args.mission) {
        Ok(mut s) => {
            if !args.markers {
                s.markers.clear();
            }
            s
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    if args.screenshot.is_some() {
        if let Err(e) = screenshot(&args, &scene) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let camera = initial_camera(&scene, args.camera);
    let mut app = App { scene, camera, gfx: None, keys: HashSet::new(), looking: false, last: Instant::now(), start: Instant::now() };
    event_loop.run_app(&mut app).expect("run");
}

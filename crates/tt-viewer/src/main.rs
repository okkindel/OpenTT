//! ThinkTanks mission viewer.
//!
//! ```text
//! tt-viewer [--assets DIR] <MISSION> [--screenshot out.png] [--size WxH]
//!           [--camera X Y Z YAW_DEG PITCH_DEG] [--time SECONDS]
//! ```
//! Shows a mission (an id, `tt1_1`, or its `.ron` file) as a static scene
//! (terrain, sky, shapes; only the
//! defs the mission declares itself). `opentt --mission tt1_1`
//! shows it with the game's defs. `--assets` defaults to the asset
//! directory (`$TT_ASSETS`, else `./assets`, else the repository's).
//! Window controls: WASD move, Q/E down/up, Shift faster, right mouse drag or
//! arrow keys to look around.


use glam::Vec3;
use tt_render::{gpu, Camera, Renderer, Scene, Target};
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
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut root = tt_assets::layout::default_dir();
    let (mut mission, mut screenshot, mut size, mut camera, mut time) = (None, None, (1280, 720), None, 0.0);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--assets" => root = it.next().ok_or("--assets needs a value")?.into(),
            "--screenshot" => screenshot = Some(it.next().ok_or("--screenshot needs a path")?.into()),
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
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
    Ok(Args { root, mission, screenshot, size, camera, time })
}

fn initial_camera(scene: &Scene, given: Option<[f32; 5]>) -> Camera {
    match given {
        Some([x, y, z, yaw, pitch]) => Camera { pos: Vec3::new(x, y, z), yaw: yaw.to_radians(), pitch: pitch.to_radians() },
        None => tt_render::gpu::initial_camera(scene),
    }
}

fn screenshot(args: &Args, scene: &Scene) -> Result<(), String> {
    let (_, device, queue) = pollster::block_on(gpu::gpu(None, &gpu::instance()))?;
    let (w, h) = args.size;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = gpu::render_target(&device, "screenshot", args.size, format);
    let mut renderer = Renderer::new(&device, &queue, format, scene);
    let camera = initial_camera(scene, args.camera);
    renderer.render(Target { device: &device, queue: &queue, view: &target.create_view(&Default::default()), size: args.size }, &camera, args.time);
    let pixels = gpu::read_rgba(&device, &queue, &target, args.size, format);
    let out = args.screenshot.as_ref().unwrap();
    image::save_buffer(out, &pixels, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    println!("saved {} (camera {:.1} {:.1} {:.1})", out.display(), camera.pos.x, camera.pos.y, camera.pos.z);
    Ok(())
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
        let instance = gpu::instance();
        let surface = instance.create_surface(window.clone()).expect("surface");
        let (adapter, device, queue) = pollster::block_on(gpu::gpu(Some(&surface), &instance)).expect("gpu");
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
                        let target = Target { device: &gfx.device, queue: &gfx.queue, view: &view, size: (gfx.config.width, gfx.config.height) };
                        gfx.renderer.render(target, &self.camera, self.start.elapsed().as_secs_f32());
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
        Ok(s) => s,
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

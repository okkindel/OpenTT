//! Window, main loop and input translation (winit + wgpu).
//!
//! Each frame runs as many fixed 32 ms sim ticks as real time requires,
//! rebuilds the 3D scene when the mission objects changed, applies
//! video mode requests and draws the frame (3D scene if a mission is loaded,
//! otherwise a cleared background) with the GUI canvas on top, letterboxed at
//! the game's resolution (the `--mission` developer view shows the 3D scene
//! only).

use crate::engine::Engine;
use crate::input::{key_name, Device, InputEvent};
use glam::Vec3;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use crate::frontend::Frontend;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    fe: Frontend,
    /// Keyboard/text window events to egui input for the new menus.
    egui: Option<egui_winit::State>,
    /// Mouse events for the new menus, in menu points.
    pointer: crate::menu::PointerInput,
}

pub struct App {
    pub engine: Engine,
    pub root: PathBuf,
    /// Developer free-fly camera (`--mission`); Escape quits.
    pub free_fly: bool,
    gfx: Option<Gfx>,
    keys: HashSet<KeyCode>,
    looking: bool,
    last: Instant,
    title: String,
    /// Cursor in window pixels.
    /// How the pointer is captured for relative motion, if it is.
    capture: Option<Capture>,
    focused: bool,
    present_modes: Vec<wgpu::PresentMode>,
    /// Frame-time instrumentation (`--frame-stats`).
    pub stats: crate::framestats::FrameStats,
    /// When the next frame may start without vsync (frame cap, see `pace`).
    next_frame: Option<Instant>,
    /// FPS limit below the refresh rate with vsync: each drawn frame is
    /// shown for `repeat_k` vblanks; the extra presents copy `repeat_frame`
    /// (see `pace`). `repeat_left`: presents still due.
    repeat_k: u32,
    repeat_left: u32,
    repeat_frame: Option<wgpu::Texture>,
    /// Called before each frame's sim step with the real time (ms) the step
    /// will simulate (scripted input of unattended runs).
    pub before_step: Option<BeforeStep>,
    /// Never grab the pointer (unattended runs).
    pub no_grab: bool,
    /// A window to use instead of opening one (handed over by the import
    /// screen, [`crate::launcher`]).
    adopt: Option<Arc<Window>>,
}

/// Hook run before each frame's sim step: (engine, real time not yet
/// simulated in ms, this frame's real time in ms).
pub type BeforeStep = Box<dyn FnMut(&mut Engine, f32, f32)>;

/// Fifo with vsync: one frame per display refresh, paced by the display.
/// (Mailbox renders as fast as it can and throws most frames away, which
/// keeps an integrated GPU, shared with the desktop compositor, saturated:
/// frames then wait 20-30 ms for the GPU at random and the picture
/// stutters.) Without vsync Immediate, else Mailbox/Fifo.
fn present_mode(available: &[wgpu::PresentMode], vsync: bool) -> wgpu::PresentMode {
    let prefs: &[wgpu::PresentMode] = if vsync { &[wgpu::PresentMode::Fifo] } else { &[wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo] };
    prefs.iter().copied().find(|m| available.contains(m)).unwrap_or(wgpu::PresentMode::Fifo)
}

/// Pointer capture while the game wants relative mouse motion.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Capture {
    /// Locked in place: raw motion from `DeviceEvent::MouseMotion`.
    Locked,
    /// Confined to the window and re-centred after every move; motion is
    /// the cursor's offset from the centre (also works for absolute pointer
    /// devices such as tablets, VMs and remote desktops).
    Confined,
}

impl App {
    pub fn new(engine: Engine, root: PathBuf, free_fly: bool) -> App {
        App {
            engine,
            root,
            free_fly,
            gfx: None,
            keys: HashSet::new(),
            looking: false,
            last: Instant::now(),
            title: String::new(),
            capture: None,
            focused: true,
            present_modes: Vec::new(),
            stats: crate::framestats::FrameStats::new(false),
            next_frame: None,
            repeat_k: 1,
            repeat_left: 0,
            repeat_frame: None,
            before_step: None,
            no_grab: false,
            adopt: None,
        }
    }

    /// Use `window` (already open) instead of opening one in `resumed`.
    pub fn adopt_window(&mut self, window: Arc<Window>) {
        self.adopt = Some(window);
    }

    /// Lets go of the window (GPU state, the menus' egui state), releasing
    /// the pointer, and returns it (for the import screen).
    pub fn take_window(&mut self) -> Option<Arc<Window>> {
        let g = self.gfx.take()?;
        let _ = g.window.set_cursor_grab(winit::window::CursorGrabMode::None);
        g.window.set_cursor_visible(true);
        self.capture = None;
        let Gfx { window, .. } = g;
        Some(window)
    }

    fn apply_video(&mut self) {
        let v = &mut self.engine.host.video;
        if !v.changed {
            return;
        }
        v.changed = false;
        let Some(g) = &mut self.gfx else { return };
        let mode = present_mode(&self.present_modes, v.vsync);
        if g.config.present_mode != mode {
            g.config.present_mode = mode;
            g.surface.configure(&g.device, &g.config);
        }
        if v.fullscreen {
            g.window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        } else {
            g.window.set_fullscreen(None);
            let _ = g.window.request_inner_size(PhysicalSize::new(v.width, v.height));
        }
    }

    /// Captures or releases the pointer as the game requires: captured while
    /// the GUI cursor is off and `lockMouse(1)` (PlayGui), and the window
    /// has focus.
    fn update_capture(&mut self) {
        let Some(g) = &self.gfx else { return };
        let want = !self.free_fly && !self.no_grab && self.focused && !self.engine.host.canvas.cursor_on && self.engine.host.mouse_locked && !self.menu_visible();
        if want == self.capture.is_some() {
            return;
        }
        let w = &g.window;
        if want {
            use winit::window::CursorGrabMode;
            let mode = if w.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
                Some(Capture::Locked)
            } else if w.set_cursor_grab(CursorGrabMode::Confined).is_ok() {
                Some(Capture::Confined)
            } else {
                None
            };
            if let Some(m) = mode {
                w.set_cursor_visible(false);
                if m == Capture::Confined {
                    let s = w.inner_size();
                    let _ = w.set_cursor_position(winit::dpi::PhysicalPosition::new(s.width as f64 / 2.0, s.height as f64 / 2.0));
                }
                self.engine.host.log.info(&format!("Mouse captured ({m:?})"));
            }
            self.capture = mode;
        } else {
            let _ = w.set_cursor_grab(winit::window::CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.capture = None;
        }
    }

    /// The new menu is on screen and takes the keyboard and mouse.
    fn menu_visible(&self) -> bool {
        self.engine.menu.as_ref().is_some_and(|m| m.visible(&self.engine.host))
    }

    /// Relative mouse motion for the action maps (`xaxis`/`yaxis`).
    fn mouse_axes(&mut self, dx: f64, dy: f64) {
        // The script's sensitivity curve (`getMouseAdjustAmount`, 0.0065–0.023
        // rad per count) was tuned for ~400 DPI mice; modern mice send several
        // times more counts. (default 0.25 ≈ 1600 DPI → 400) scales the raw
        // counts before the action maps.
        let scale = {
            let v = self.engine.host.shell.prefs.get("input.mouse_scale");
            if v.is_empty() { 0.25 } else { tt_game::fmt::atof(&v).clamp(0.01, 4.0) }
        };
        let (dx, dy) = (dx * scale, dy * scale);
        if dx != 0.0 {
            self.engine.input(&InputEvent::Axis { device: Device::Mouse, axis: "xaxis".into(), delta: dx as f32 });
        }
        if dy != 0.0 {
            self.engine.input(&InputEvent::Axis { device: Device::Mouse, axis: "yaxis".into(), delta: dy as f32 });
        }
    }

    fn update_title(&mut self) {
        let Some(g) = &self.gfx else { return };
        let h = &self.engine.host;
        let mut t = h.video.title.clone();
        if let Some(s) = &g.fe.scene {
            t += &format!(" - {}", s.name);
        }
        if h.debug {
            t += &format!(" [{}]", h.canvas.describe());
        }
        if t != self.title {
            g.window.set_title(&t);
            self.title = t;
        }
    }

    fn fly(&mut self, dt: f32) {
        let Some(g) = &mut self.gfx else { return };
        let cam = &mut g.fe.camera;
        let speed = if self.keys.contains(&KeyCode::ShiftLeft) { 200.0 } else { 50.0 } * dt;
        let fwd = cam.forward();
        let right = fwd.cross(Vec3::Z).normalize_or_zero();
        for (k, d) in [(KeyCode::KeyW, fwd), (KeyCode::KeyS, -fwd), (KeyCode::KeyD, right), (KeyCode::KeyA, -right), (KeyCode::KeyE, Vec3::Z), (KeyCode::KeyQ, -Vec3::Z)] {
            if self.keys.contains(&k) {
                cam.pos += d * speed;
            }
        }
        let turn = 1.5 * dt;
        for (k, yaw, pitch) in [(KeyCode::ArrowLeft, -turn, 0.0), (KeyCode::ArrowRight, turn, 0.0), (KeyCode::ArrowUp, 0.0, turn), (KeyCode::ArrowDown, 0.0, -turn)] {
            if self.keys.contains(&k) {
                cam.yaw += yaw;
                cam.pitch = (cam.pitch + pitch).clamp(-1.5, 1.5);
            }
        }
        let f = cam.forward();
        self.engine.host.audio.set_listener(cam.pos.into(), f.into(), Vec3::Z.into());
    }

    fn frame(&mut self, el: &ActiveEventLoop) {
        if self.repeat_left > 0 {
            self.repeat_left -= 1;
            if self.present_repeat() {
                return;
            }
        }
        self.stats.begin_frame();
        let dt_ms = (self.last.elapsed().as_secs_f64() * 1000.0) as f32;
        self.last = Instant::now();
        {
            let Some(g) = &mut self.gfx else { return };
            if let Some(f) = self.before_step.as_mut() {
                f(&mut self.engine, g.fe.acc_ms, dt_ms);
            }
            let t = Instant::now();
            self.engine.frame_ticks = Default::default();
            let n = g.fe.step(&mut self.engine, &g.device, &g.queue, dt_ms);
            g.fe.perf.add("step", t.elapsed().as_secs_f64() * 1000.0);
            let ft = self.engine.frame_ticks;
            self.stats.step_done(n, ft.script_ms, ft.sim_ms);
        }
        crate::console::take_stop(&mut self.engine.host);
        if self.engine.host.quit_requested {
            self.engine.shutdown();
            el.exit();
            return;
        }
        self.apply_video();
        self.update_capture();
        self.update_title();
        if self.free_fly {
            self.fly((dt_ms / 1000.0).min(0.1));
        }
        let Some(g) = &mut self.gfx else { return };
        self.stats.other_done();
        match g.surface.get_current_texture() {
            Ok(frame) => {
                self.stats.acquire_done();
                let view = frame.texture.create_view(&Default::default());
                let size = (g.config.width, g.config.height);
                if let (Some(st), Some(layer)) = (g.egui.as_mut(), g.fe.menu.as_ref()) {
                    layer.set_size(size);
                    let keyboard = st.take_egui_input(&g.window);
                    g.fe.menu_input = Some(crate::menu::window_raw_input(Some(keyboard), &mut g.pointer, size, 0.0));
                }
                g.fe.draw(&mut self.engine, &g.device, &g.queue, &view, size, dt_ms);
                if self.engine.host.screenshot_request.is_some() {
                    capture_frame(&mut self.engine, g, &frame.texture, size);
                }
                self.stats.draw_done();
                if self.stats.enabled {
                    let mut probes = self.engine.motion_probe();
                    probes.push(("camera", g.fe.camera.pos));
                    self.stats.probe(&probes);
                    self.stats.perf(&g.fe.perf);
                }
                if self.repeat_k > 1 && g.config.usage.contains(wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST) {
                    // Kept for the repeated presents of the FPS limit.
                    let ext = frame.texture.size();
                    if self.repeat_frame.as_ref().is_none_or(|t| t.size() != ext || t.format() != frame.texture.format()) {
                        self.repeat_frame = Some(g.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("fps limit frame"),
                            size: ext,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: frame.texture.format(),
                            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        }));
                    }
                    let mut enc = g.device.create_command_encoder(&Default::default());
                    enc.copy_texture_to_texture(frame.texture.as_image_copy(), self.repeat_frame.as_ref().unwrap().as_image_copy(), ext);
                    g.queue.submit([enc.finish()]);
                    self.repeat_left = self.repeat_k - 1;
                } else {
                    self.repeat_frame = None;
                }
                frame.present();
                self.stats.present_done();
                if let (Some(st), Some(out)) = (g.egui.as_mut(), g.fe.menu_output.take()) {
                    st.handle_platform_output(&g.window, out);
                }
            }
            Err(_) => g.surface.configure(&g.device, &g.config),
        }
        g.window.request_redraw();
        self.pace();
        self.stats.end_frame();
    }

    /// Presents the last drawn frame again (FPS limit with vsync). False if
    /// there is nothing to repeat.
    fn present_repeat(&mut self) -> bool {
        let (Some(g), Some(saved)) = (&mut self.gfx, self.repeat_frame.as_ref()) else { return false };
        let Ok(frame) = g.surface.get_current_texture() else { return false };
        if frame.texture.size() != saved.size() {
            frame.present();
            return false;
        }
        let mut enc = g.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_texture(saved.as_image_copy(), frame.texture.as_image_copy(), saved.size());
        g.queue.submit([enc.finish()]);
        frame.present();
        g.window.request_redraw();
        true
    }

    /// Uncapped rendering at hundreds of fps saturates integrated GPUs
    /// (shared with the desktop compositor) and makes frames stall in
    /// acquire/present.
    ///
    /// With vsync a limit below the refresh rate paces too: the next frame
    /// starts a little (3 ms) before `1/limit` after this one, so Fifo
    /// presents it on the vblank that matches the limit (30 on 60 Hz = every
    /// second vblank) and the frame times stay whole multiples of the
    /// refresh period instead of drifting into an extra one.
    fn pace(&mut self) {
        let Some(g) = &self.gfx else { return };
        let pref = tt_game::fmt::atof(&self.engine.host.shell.prefs.get("video.max_fps"));
        let refresh = g.window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).map_or(60.0, |mhz| (mhz as f64 / 1000.0).clamp(30.0, 500.0));
        let vsync = self.engine.host.video.vsync;
        let limit = if pref > 0.0 { Some(pref.clamp(20.0, 1000.0)) } else if pref < 0.0 { None } else { Some(refresh) };
        let fps = match limit {
            // Vsync paces at the refresh rate by itself.
            Some(l) if vsync && l >= refresh * 0.95 => None,
            Some(l) => Some(l),
            None => None,
        };
        let Some(fps) = fps else {
            self.next_frame = None;
            self.repeat_k = 1;
            return;
        };
        if vsync {
            // Each frame stays up for k vblanks: the extra presents repeat
            // it (a copy, see `present_repeat`), so Fifo itself keeps the
            // cadence exact (30 on 60 Hz = every second vblank, no judder).
            self.repeat_k = (refresh / fps).round().clamp(1.0, 8.0) as u32;
            self.next_frame = None;
            return;
        }
        self.repeat_k = 1;
        let period = std::time::Duration::from_secs_f64(1.0 / fps);
        let now = Instant::now();
        let next = self.next_frame.map_or(now + period, |n| n + period);
        // Fell far behind (load, window drag): restart the schedule.
        let next = if next + period < now { now + period } else { next };
        if next > now {
            std::thread::sleep(next - now);
        }
        self.next_frame = Some(next);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let v = &self.engine.host.video;
        let window = match self.adopt.take() {
            Some(w) => {
                w.set_title(&v.title);
                if v.fullscreen {
                    w.set_fullscreen(Some(Fullscreen::Borderless(None)));
                } else {
                    w.set_fullscreen(None);
                    let _ = w.request_inner_size(PhysicalSize::new(v.width, v.height));
                }
                w
            }
            None => {
                let mut attrs = crate::icon::with_icon(Window::default_attributes()).with_title(v.title.clone()).with_inner_size(PhysicalSize::new(v.width, v.height));
                if v.fullscreen {
                    attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
                }
                Arc::new(el.create_window(attrs).expect("window"))
            }
        };
        if let Some(m) = window.current_monitor() {
            let s = m.size();
            self.engine.host.video.desktop = (s.width, s.height);
            let mut modes: Vec<(u32, u32)> = m.video_modes().map(|v| (v.size().width, v.size().height)).collect();
            modes.sort();
            modes.dedup();
            self.engine.host.video.modes = modes;
        }
        let instance = crate::gfx::instance();
        let surface = instance.create_surface(window.clone()).unwrap_or_else(|e| panic!("graphics: cannot draw in the window: {e}"));
        let (adapter, device, queue) = pollster::block_on(crate::gfx::gpu(Some(&surface), &instance)).unwrap_or_else(|e| panic!("graphics: {e}"));
        let size = window.inner_size();
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        // Fifo (vsync) unless vsync is off (see `present_mode`). One frame
        // of queue latency keeps input responsive.
        let caps = surface.get_capabilities(&adapter);
        config.present_mode = present_mode(&caps.present_modes, self.engine.host.video.vsync);
        self.present_modes = caps.present_modes.clone();
        config.desired_maximum_frame_latency = 1;
        // `screenShot` reads the presented frame back.
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        // The FPS limit with vsync re-presents a kept copy.
        if caps.usages.contains(wgpu::TextureUsages::COPY_DST) {
            config.usage |= wgpu::TextureUsages::COPY_DST;
        }
        surface.configure(&device, &config);
        let mut fe = Frontend::new(&device, &queue, config.format, self.root.clone(), self.free_fly);
        let egui = self.engine.menu.is_some().then(|| {
            let layer = crate::menu_gpu::MenuLayer::new(&device, config.format);
            layer.set_size((config.width, config.height));
            let st = egui_winit::State::new(layer.ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, Some(device.limits().max_texture_dimension_2d as usize));
            fe.menu = Some(layer);
            st
        });
        // Build the scene for a mission loaded before the window existed.
        if self.engine.host.mission_load.is_some() {
            self.engine.host.scene_dirty = true;
            fe.step(&mut self.engine, &device, &queue, 0.0);
        }
        // A window handed over from the import screen has had its first redraw.
        window.request_redraw();
        self.gfx = Some(Gfx { window, surface, device, queue, config, fe, egui, pointer: Default::default() });
        self.engine.host.video.changed = false;
        self.last = Instant::now();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if self.gfx.is_none() {
            return;
        }
        let _timed = (!matches!(event, WindowEvent::RedrawRequested)).then(|| self.stats.event());
        // While the new menu is on screen it gets the keyboard and mouse.
        let input = matches!(
            event,
            WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } | WindowEvent::CursorMoved { .. } | WindowEvent::CursorLeft { .. } | WindowEvent::Ime(_) | WindowEvent::ModifiersChanged(_) | WindowEvent::Touch(_)
        );
        let visible = self.menu_visible();
        {
            let g = self.gfx.as_mut().unwrap();
            let size = (g.config.width, g.config.height);
            // The menu's pointer follows the cursor even while hidden, so a
            // click right after it opens lands where the cursor is.
            g.pointer.window_event(&event, size, self.capture.is_none(), visible);
            // Keyboard, text, focus and the rest go through egui-winit (its
            // pointer events are replaced by ours).
            if visible || !input {
                if let Some(st) = g.egui.as_mut() {
                    let _ = st.on_window_event(&g.window, &event);
                }
            }
        }
        if visible && input {
            match &event {
                // A key held when the menu opened is released to the game too.
                WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Released => {
                    if let PhysicalKey::Code(k) = event.physical_key {
                        if self.keys.remove(&k) {
                            if let Some(name) = key_name(k) {
                                self.engine.key(name, false, None);
                            }
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                self.engine.host.quit_requested = true;
            }
            WindowEvent::Resized(s) => {
                let g = self.gfx.as_mut().unwrap();
                g.config.width = s.width.max(1);
                g.config.height = s.height.max(1);
                g.surface.configure(&g.device, &g.config);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(k) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    if pressed {
                        self.keys.insert(k);
                    } else {
                        self.keys.remove(&k);
                    }
                    if self.free_fly && k == KeyCode::Escape && pressed {
                        self.engine.host.quit_requested = true;
                    }
                    if let Some(name) = key_name(k) {
                        // Auto-repeated presses only reach text input (action maps ignore repeats).
                        let text = event.text.as_ref().map(|t| t.to_string());
                        self.engine.key(name, pressed, text.as_deref());
                    }
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                let pressed = state == ElementState::Pressed;
                if button == MouseButton::Right {
                    self.looking = pressed;
                }
                if let Some(i) = crate::menu::mouse_button_index(button) {
                    self.engine.mouse_button(i, pressed);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 120.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                self.engine.mouse_wheel(d);
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.capture == Some(Capture::Confined) {
                    // Offset from the centre, then back to the centre.
                    let g = self.gfx.as_ref().unwrap();
                    let s = g.window.inner_size();
                    let (cx, cy) = ((s.width / 2) as f64, (s.height / 2) as f64);
                    let (dx, dy) = (position.x - cx, position.y - cy);
                    if dx != 0.0 || dy != 0.0 {
                        let _ = g.window.set_cursor_position(winit::dpi::PhysicalPosition::new(cx, cy));
                        self.mouse_axes(dx, dy);
                    }
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                self.update_capture();
            }
            WindowEvent::RedrawRequested => self.frame(el),
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent) {
        let _timed = self.stats.event();
        if let DeviceEvent::MouseMotion { delta } = event {
            if let Some(g) = self.gfx.as_mut().filter(|g| self.looking && self.free_fly && g.fe.scene.is_some()) {
                g.fe.camera.yaw += delta.0 as f32 * 0.003;
                g.fe.camera.pitch = (g.fe.camera.pitch - delta.1 as f32 * 0.003).clamp(-1.5, 1.5);
            }
            // Raw motion drives the action maps only while the pointer is
            // locked; a free pointer must not turn the turret (moving it back
            // to the window centre, or stopping at a screen edge, would bias
            // the aim).
            if self.capture == Some(Capture::Locked) {
                self.mouse_axes(delta.0, delta.1);
            }
        }
    }
}

/// Saves the frame just drawn for a pending `screenShot` request: read
/// back from the window's frame, or (surfaces without `COPY_SRC`) drawn
/// again into an offscreen texture of the same size.
fn capture_frame(engine: &mut Engine, g: &mut Gfx, frame: &wgpu::Texture, size: (u32, u32)) {
    let pixels = if frame.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        crate::gfx::read_rgba(&g.device, &g.queue, frame, size, g.config.format)
    } else {
        let texture = crate::gfx::render_target(&g.device, "screenshot", size, g.config.format);
        let view = texture.create_view(&Default::default());
        g.fe.draw(engine, &g.device, &g.queue, &view, size, 0.0);
        crate::gfx::read_rgba(&g.device, &g.queue, &texture, size, g.config.format)
    };
    engine.save_screenshot(&pixels, size);
}

/// Runs `handler`'s event loop (the game, or [`crate::launcher::Launcher`])
/// until it exits. Only once per process; off the main thread on Linux and
/// Windows only (on macOS the game runs on the main thread: main.rs).
pub fn run_handler<H: ApplicationHandler>(handler: &mut H) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        use winit::platform::x11::EventLoopBuilderExtX11;
        EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
        EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
    }
    // Without this winit panics off the main thread on Windows ("Initializing
    // the event loop outside of the main thread ..."): the game runs on a
    // thread of its own (main.rs). The window and its messages then live on
    // that thread, which Windows allows. (macOS has no such option: there the
    // game runs on the main thread.)
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    let event_loop = builder.build().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(handler).map_err(|e| e.to_string())
}

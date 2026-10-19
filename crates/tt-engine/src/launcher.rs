//! The window's two phases: the import screen ([`crate::gamedata::SetupFlow`]
//! drawn by `tt_menu::SetupState`) when the game data is missing or the
//! player asked to import it again, and the game ([`App`]). One window
//! and one event loop for both (winit allows one per process): the
//! screen hands its window to the game when the import is done, the game
//! hands it back for Settings → Game data → Re-import (the engine is shut
//! down first and a new one boots afterwards).

use crate::app::App;
use crate::gamedata::{Outcome, SetupFlow};
use crate::menu::PointerInput;
use crate::menu_gpu::MenuLayer;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tt_menu::SetupMode;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

/// Boots a game on an asset directory: the engine (booted) and its window
/// state. Called after an import and when the import screen is left.
pub type AppFactory = Box<dyn FnMut(&Path) -> Result<App, String>>;

/// The import screen's window state.
struct SetupGfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    layer: MenuLayer,
    egui: egui_winit::State,
    pointer: PointerInput,
}

impl SetupGfx {
    fn new(window: Arc<Window>) -> Result<SetupGfx, String> {
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let (adapter, device, queue) = pollster::block_on(crate::gfx::gpu(Some(&surface), &instance))?;
        let size = window.inner_size();
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).ok_or("no surface configuration")?;
        // Paced by the display: the screen mostly waits.
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);
        let layer = MenuLayer::new(&device, config.format);
        layer.set_size((config.width, config.height));
        let egui = egui_winit::State::new(layer.ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, Some(device.limits().max_texture_dimension_2d as usize));
        window.request_redraw();
        Ok(SetupGfx { window, surface, device, queue, config, layer, egui, pointer: PointerInput::default() })
    }
}

enum Phase {
    Setup(Box<SetupFlow>),
    Game(Box<App>),
    /// Between phases (or after a failure).
    None,
}

/// The game's window: the import screen, then the game.
pub struct Launcher {
    phase: Phase,
    factory: AppFactory,
    /// The import screen's window.
    setup_gfx: Option<SetupGfx>,
    /// The asset directory to go back to when a re-import is left.
    assets: PathBuf,
    /// Where imports go.
    target: PathBuf,
    started: Instant,
    /// Why the launcher stopped, if it failed.
    pub error: Option<String>,
}

impl Launcher {
    /// Starts with the game (the game data is there).
    pub fn game(app: App, target: PathBuf, factory: AppFactory) -> Launcher {
        let assets = app.root.clone();
        Launcher { phase: Phase::Game(Box::new(app)), factory, setup_gfx: None, assets, target, started: Instant::now(), error: None }
    }

    /// Starts with the first-run screen (no game data in `assets`).
    pub fn first_run(assets: PathBuf, target: PathBuf, factory: AppFactory) -> Launcher {
        let flow = SetupFlow::new(SetupMode::FirstRun, target.clone(), None);
        Launcher { phase: Phase::Setup(Box::new(flow)), factory, setup_gfx: None, assets, target, started: Instant::now(), error: None }
    }

    /// The import screen's state, while it is up.
    pub fn setup(&self) -> Option<&SetupFlow> {
        match &self.phase {
            Phase::Setup(f) => Some(f),
            _ => None,
        }
    }

    /// The game, while it runs (also after the window closed).
    pub fn app(&mut self) -> Option<&mut App> {
        match &mut self.phase {
            Phase::Game(a) => Some(a),
            _ => None,
        }
    }

    /// Boots a game on `assets` in `window` (records the import's source first).
    fn start_game(&mut self, el: &ActiveEventLoop, window: Arc<Window>, assets: &Path, source: Option<&Path>) {
        match (self.factory)(assets) {
            Ok(mut app) => {
                if let Some(s) = source {
                    crate::gamedata::record_source(&mut app.engine.host, s);
                }
                self.assets = assets.to_path_buf();
                app.adopt_window(window);
                app.resumed(el);
                self.phase = Phase::Game(Box::new(app));
            }
            Err(e) => {
                self.error = Some(e);
                self.phase = Phase::None;
                el.exit();
            }
        }
    }

    /// Settings → Game data → Re-import: the game shuts down, the screen comes up.
    fn start_reimport(&mut self, el: &ActiveEventLoop) {
        let Phase::Game(mut app) = std::mem::replace(&mut self.phase, Phase::None) else { return };
        app.engine.host.reimport_requested = false;
        let source = app.engine.host.shell.prefs.get(crate::gamedata::SOURCE_KEY);
        app.engine.shutdown();
        let Some(window) = app.take_window() else {
            el.exit();
            return;
        };
        drop(app);
        let flow = SetupFlow::new(SetupMode::Reimport, self.target.clone(), (!source.is_empty()).then_some(source));
        self.open_setup(el, window, flow);
    }

    /// Options → Mods → Restart: the game shuts down and boots again on the
    /// same asset directory (with the mods now enabled).
    fn restart(&mut self, el: &ActiveEventLoop) {
        let Phase::Game(mut app) = std::mem::replace(&mut self.phase, Phase::None) else { return };
        app.engine.host.restart_requested = false;
        app.engine.shutdown();
        let Some(window) = app.take_window() else {
            el.exit();
            return;
        };
        drop(app);
        let assets = self.assets.clone();
        self.start_game(el, window, &assets, None);
    }

    fn open_setup(&mut self, el: &ActiveEventLoop, window: Arc<Window>, flow: SetupFlow) {
        window.set_title(&format!("{} - game data", tt_assets::product::WINDOW_TITLE));
        match SetupGfx::new(window) {
            Ok(g) => {
                self.setup_gfx = Some(g);
                self.phase = Phase::Setup(Box::new(flow));
            }
            Err(e) => {
                self.error = Some(format!("cannot draw the import screen: {e}"));
                el.exit();
            }
        }
    }

    /// One frame of the import screen; switches to the game when it is done.
    fn setup_frame(&mut self, el: &ActiveEventLoop) {
        let (Phase::Setup(flow), Some(g)) = (&mut self.phase, self.setup_gfx.as_mut()) else { return };
        let size = (g.config.width, g.config.height);
        g.layer.set_size(size);
        let keyboard = g.egui.take_egui_input(&g.window);
        let raw = crate::menu::window_raw_input(Some(keyboard), &mut g.pointer, size, self.started.elapsed().as_secs_f64());
        let mut action = None;
        let out = g.layer.ctx.run(raw, |ctx| action = flow.ui.ui(ctx));
        if let Some(a) = action {
            flow.handle(a);
        }
        flow.poll();
        let platform = out.platform_output.clone();
        match g.surface.get_current_texture() {
            Ok(frame) => {
                let view = frame.texture.create_view(&Default::default());
                let mut enc = g.device.create_command_encoder(&Default::default());
                {
                    let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("setup clear"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                        ..Default::default()
                    });
                }
                g.queue.submit([enc.finish()]);
                g.layer.paint(&g.device, &g.queue, &view, size, out);
                frame.present();
            }
            Err(_) => g.surface.configure(&g.device, &g.config),
        }
        g.egui.handle_platform_output(&g.window, platform);
        g.window.request_redraw();
        let Some(outcome) = flow.outcome.take() else { return };
        let window = self.setup_gfx.take().map(|g| g.window);
        self.phase = Phase::None;
        let Some(window) = window else { return };
        match outcome {
            Outcome::Imported { assets, source, report } => {
                for w in &report.warnings {
                    eprintln!("import: {w}");
                }
                self.start_game(el, window, &assets, Some(&source));
            }
            Outcome::Back => {
                let assets = self.assets.clone();
                self.start_game(el, window, &assets, None);
            }
            Outcome::Quit => el.exit(),
        }
    }

    fn setup_event(&mut self, el: &ActiveEventLoop, event: WindowEvent) {
        let Some(g) = self.setup_gfx.as_mut() else { return };
        let size = (g.config.width, g.config.height);
        g.pointer.window_event(&event, size, true, true);
        let _ = g.egui.on_window_event(&g.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                if let Phase::Setup(flow) = &mut self.phase {
                    // Stops a running import (its partial output is removed).
                    flow.handle(tt_menu::SetupAction::Quit);
                }
                el.exit();
            }
            WindowEvent::Resized(s) => {
                g.config.width = s.width.max(1);
                g.config.height = s.height.max(1);
                g.surface.configure(&g.device, &g.config);
            }
            WindowEvent::RedrawRequested => self.setup_frame(el),
            _ => {}
        }
    }
}

impl ApplicationHandler for Launcher {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        match &mut self.phase {
            Phase::Game(app) => app.resumed(el),
            Phase::Setup(_) if self.setup_gfx.is_none() => {
                let attrs = crate::icon::with_icon(Window::default_attributes()).with_title(tt_assets::product::WINDOW_TITLE).with_inner_size(PhysicalSize::new(1280, 720));
                match el.create_window(attrs) {
                    Ok(w) => {
                        let Phase::Setup(flow) = std::mem::replace(&mut self.phase, Phase::None) else { return };
                        self.open_setup(el, Arc::new(w), *flow);
                    }
                    Err(e) => {
                        self.error = Some(format!("cannot open the window: {e}"));
                        el.exit();
                    }
                }
            }
            _ => {}
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match &mut self.phase {
            Phase::Game(app) => {
                let redraw = matches!(event, WindowEvent::RedrawRequested);
                app.window_event(el, id, event);
                if redraw && app.engine.host.reimport_requested && !app.engine.host.quit_requested {
                    self.start_reimport(el);
                } else if redraw && app.engine.host.restart_requested && !app.engine.host.quit_requested {
                    self.restart(el);
                }
            }
            Phase::Setup(_) => self.setup_event(el, event),
            Phase::None => {}
        }
    }

    fn device_event(&mut self, el: &ActiveEventLoop, id: winit::event::DeviceId, event: DeviceEvent) {
        if let Phase::Game(app) = &mut self.phase {
            app.device_event(el, id, event);
        }
    }
}

/// Runs the launcher's window until the game quits.
pub fn run(launcher: &mut Launcher) -> Result<(), String> {
    crate::app::run_handler(launcher)?;
    match launcher.error.take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

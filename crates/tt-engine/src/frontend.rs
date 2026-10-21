//! The per-frame client loop shared by the window ([`crate::app`]) and the
//! offscreen `--bench` mode: fixed 32 ms sim ticks against real time, scene
//! (re)builds on mission changes, and drawing (3D view into the GUI's
//! `GameTSCtrl` viewports, effects, the GUI draw list, the menu). Every
//! stage is timed in [`Perf`].

use crate::engine::{Engine, TICK_MS};
use crate::gfx::{self, Target};
use glam::Vec3;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;
use tt_render::{Camera, Renderer, Scene};

/// Timings (milliseconds) and counters by stage. Each stage keeps its last
/// [`PERF_SAMPLES`] samples (a ring), so a long session does not grow.
#[derive(Default)]
pub struct Perf {
    pub samples: BTreeMap<&'static str, Vec<f64>>,
    /// Samples ever added per stage (the ring position).
    pub added: BTreeMap<&'static str, u64>,
    pub counts: BTreeMap<&'static str, u64>,
}

/// Samples kept per stage (about 7 minutes of frames at 240 fps).
pub const PERF_SAMPLES: usize = 100_000;

impl Perf {
    pub fn add(&mut self, key: &'static str, ms: f64) {
        let n = self.added.entry(key).or_default();
        let v = self.samples.entry(key).or_default();
        if v.len() < PERF_SAMPLES {
            v.push(ms);
        } else {
            v[(*n % PERF_SAMPLES as u64) as usize] = ms;
        }
        *n += 1;
    }

    /// Per stage: samples ever added and the latest one.
    pub fn latest(&self) -> impl Iterator<Item = (&'static str, u64, f64)> + '_ {
        self.added.iter().map(|(&k, &n)| {
            let v = &self.samples[k];
            (k, n, v[((n - 1) % PERF_SAMPLES as u64) as usize])
        })
    }

    pub fn count(&mut self, key: &'static str) {
        *self.counts.entry(key).or_default() += 1;
    }

    pub fn time<R>(&mut self, key: &'static str, f: impl FnOnce() -> R) -> R {
        let t = Instant::now();
        let r = f();
        self.add(key, t.elapsed().as_secs_f64() * 1000.0);
        r
    }

    /// min / avg / p95 / max (and sample count) per stage, then the counters.
    pub fn report(&self) -> String {
        let mut s = format!("{:<22} {:>7} {:>8} {:>8} {:>8} {:>8} {:>9}\n", "stage (ms)", "n", "min", "avg", "p95", "max", "total");
        for (k, v) in &self.samples {
            if v.is_empty() {
                continue;
            }
            let mut sorted = v.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let sum: f64 = v.iter().sum();
            let p95 = sorted[((sorted.len() as f64 * 0.95) as usize).min(sorted.len() - 1)];
            s += &format!("{k:<22} {:>7} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>9.1}\n", v.len(), sorted[0], sum / v.len() as f64, p95, sorted[sorted.len() - 1], sum);
        }
        for (k, n) in &self.counts {
            s += &format!("{k:<22} {n:>7}\n");
        }
        s
    }
}

pub struct Frontend {
    /// Draws the game from this camera instead of the player's (offscreen
    /// review shots, e.g. a projectile's arc from the side).
    pub camera_override: Option<Camera>,
    /// Horizontal field of view (degrees) with [`Self::camera_override`].
    pub fov_override: Option<f32>,
    /// The "Show FPS" counter, and whether it is on (the pref is read every
    /// [`FPS_PREF_FRAMES`] frames).
    pub fps: crate::fps::FpsCounter,
    show_fps: bool,
    fps_pref_in: u32,
    /// Dynamic lights.
    pub dynlights: crate::dynlights::DynLights,
    dynamic_lights: bool,
    dynamic_light_strength: f32,
    /// HUD size, 0.5..1.5.
    hud_scale: f32,
    pub root: PathBuf,
    /// Developer free-fly view (`--mission`): 3D only, no GUI.
    pub free_fly: bool,
    pub scene: Option<Scene>,
    pub renderer: Option<Renderer>,
    pub fx: Option<gfx::FxState>,
    pub gui: crate::gui_gpu::GuiRenderer,
    pub camera: Camera,
    pub format: wgpu::TextureFormat,
    /// Real time not yet simulated (ms).
    pub acc_ms: f32,
    /// Sim time of the last step, for effects.
    pub sim_dt_ms: f32,
    /// Fractions of a millisecond not yet given to the GUI animation and the
    /// effects (they advance in whole ms; at hundreds of frames per second
    /// truncating every frame would slow or stop them).
    gui_carry_ms: f32,
    fx_carry_ms: f32,
    pub perf: Perf,
    start: Instant,
    /// Fixed animation time (offscreen screenshots: repeatable pictures).
    pub time_override: Option<f32>,
    /// Seconds the HUD and menu animations see instead of the real time
    /// since start (offscreen video capture at a fixed frame rate).
    pub wall_override: Option<f64>,
    /// Draws no HUD and no aim mark (offscreen video capture: clean shots).
    pub hide_hud: bool,
    /// Render scale and anisotropy the renderer was set up with.
    settings: Option<tt_render::RenderSettings>,
    /// The native HUD's egui layer (created on first use).
    pub hud: Option<crate::menu_gpu::HudLayer>,
    /// The new menus' egui layer (created on first use).
    pub menu: Option<crate::menu_gpu::MenuLayer>,
    /// egui input for the next menu frame (the window fills it from its
    /// events; offscreen frames get an empty one).
    pub menu_input: Option<egui::RawInput>,
    /// egui output of the last menu frame (cursor icon, clipboard...).
    pub menu_output: Option<egui::PlatformOutput>,
    /// The next 3D frame is the first of a new scene (`TT_LOAD_STATS`).
    first_frame: bool,
}

/// At most this many ticks per frame; more means the frame took too long
/// (the rest of the backlog is dropped instead of spiralling).
const MAX_TICKS_PER_FRAME: u32 = 8;

/// How often the "Show FPS" pref is looked up (frames).
const FPS_PREF_FRAMES: u32 = 15;

impl Frontend {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, root: PathBuf, free_fly: bool) -> Frontend {
        Frontend {
            fps: crate::fps::FpsCounter::new(),
            show_fps: false,
            fps_pref_in: 0,
            dynlights: Default::default(),
            hud_scale: 1.0,
            dynamic_light_strength: 1.0,
            dynamic_lights: tt_shell::prefs::GRAPHICS_DEFAULTS.dynamic_lights,
            root,
            free_fly,
            scene: None,
            renderer: None,
            fx: None,
            gui: crate::gui_gpu::GuiRenderer::new(device, queue, format),
            camera: Camera { pos: Vec3::new(0.0, -150.0, 100.0), yaw: 0.0, pitch: -0.3 },
            format,
            acc_ms: 0.0,
            sim_dt_ms: 0.0,
            gui_carry_ms: 0.0,
            fx_carry_ms: 0.0,
            perf: Perf::default(),
            start: Instant::now(),
            time_override: None,
            wall_override: None,
            hide_hud: false,
            camera_override: None,
            fov_override: None,
            settings: None,
            hud: None,
            menu: None,
            menu_input: None,
            menu_output: None,
            first_frame: false,
        }
    }

    /// Runs the sim ticks `dt_ms` of real time call for, then rebuilds the
    /// scene if the mission changed. Returns the number of ticks run.
    pub fn step(&mut self, engine: &mut Engine, device: &wgpu::Device, queue: &wgpu::Queue, dt_ms: f32) -> u32 {
        // The real time `dt_ms` was measured at: the chase camera smooths
        // against it, so it moves in step with the interpolated poses.
        engine.frame_time_ms = Some(engine.host.now_ms_f64());
        // GUI animations run in real time.
        engine.gui_realtime = true;
        engine.gui_frame(whole_ms(&mut self.gui_carry_ms, dt_ms));
        let scale = engine.time_scale();
        self.sim_dt_ms = dt_ms * scale;
        self.acc_ms += dt_ms * scale;
        let mut n = 0;
        while self.acc_ms >= TICK_MS as f32 {
            let t = Instant::now();
            engine.tick();
            self.perf.add("tick", t.elapsed().as_secs_f64() * 1000.0);
            self.perf.add("tick.script", engine.last_tick.script_ms);
            self.perf.add("tick.sim", engine.last_tick.sim_ms);
            self.acc_ms -= TICK_MS as f32;
            n += 1;
            if n >= MAX_TICKS_PER_FRAME {
                self.perf.count("dropped backlogs");
                self.acc_ms = 0.0;
                break;
            }
        }
        if let Some(m) = engine.menu.as_mut() {
            m.sync(&mut engine.host);
        }
        // Fraction of the next tick already elapsed: render poses are
        // interpolated between the last two ticks by this much.
        engine.render_alpha = (self.acc_ms / TICK_MS as f32).clamp(0.0, 1.0);
        // Mouse aim since the last tick, drawn ahead of the sim.
        // Camera aim: the mouse turns the free camera at once.
        if let Some(g) = engine.host.game.as_mut() {
            g.free_look_input(&mut engine.host.client.moves);
            g.set_pending_aim(&engine.host.client.moves);
        }
        // Settings changed by the menu: the render scale and the shape
        // detail apply at once; anisotropy and the texture detail prefs need
        // a new renderer (the scene is kept).
        if let (Some(have), Some(scene)) = (self.settings, self.scene.as_ref()) {
            let want = engine.render_settings();
            if want.anisotropy != have.anisotropy || want.detail.textures() != have.detail.textures() || want.extras.fixed() != have.extras.fixed() {
                self.perf.count("renderer creations");
                self.renderer = Some(Renderer::new_with_settings(device, queue, self.format, scene, want));
                if let Some(r) = self.renderer.as_mut() {
                    crate::gfx::preload(r, scene, device, queue, &engine.preload_assets());
                }
                tt_render::textures::trim();
                self.settings = Some(want);
            } else if want != have {
                if let Some(r) = self.renderer.as_mut() {
                    r.set_render_scale(want.render_scale);
                    r.set_detail(want.detail);
                    r.set_bloom(want.extras.bloom);
                    r.set_normal_strength(want.extras.normal_strength);
                    r.set_ao_strength(want.extras.ao_strength);
                }
                self.settings = Some(want);
            }
        }
        if engine.host.scene_dirty {
            let t = Instant::now();
            self.update_scene(engine, device, queue);
            self.perf.add("scene rebuild", t.elapsed().as_secs_f64() * 1000.0);
        }
        n
    }

    fn update_scene(&mut self, engine: &mut Engine, device: &wgpu::Device, queue: &wgpu::Queue) {
        engine.host.scene_dirty = false;
        self.perf.count("scene rebuilds");
        tt_render::loadstats::milestone("scene rebuild starts", false);
        let sp = tt_render::loadstats::span(|| "build_scene".into());
        let built = crate::mission::build_scene(&engine.host, &self.root, !self.free_fly);
        drop(sp);
        match built {
            Some(Ok(s)) => {
                engine.host.log.info(&format!("Scene built: {} ({} terrain vertices)", s.name, s.terrain_vertices.len()));
                if self.scene.is_none() {
                    self.camera = gfx::initial_camera(&s);
                }
                self.perf.count("renderer creations");
                let settings = engine.render_settings();
                self.settings = Some(settings);
                let mut r = Renderer::new_with_settings(device, queue, self.format, &s, settings);
                // The last mission's moving shapes and sprites stay on the GPU.
                if let Some(old) = self.renderer.take() {
                    r.reuse_assets(old);
                }
                self.first_frame = true;
                // Load every shape and sprite the game can show now, not
                // on first use mid-game.
                let t = Instant::now();
                crate::gfx::preload(&mut r, &s, device, queue, &engine.preload_assets());
                self.perf.add("preload", t.elapsed().as_secs_f64() * 1000.0);
                tt_render::loadstats::note(|| format!("preload total: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0));
                // The GPU has the textures now: drop the CPU copies only a
                // later load could use.
                tt_render::textures::trim();
                tt_render::loadstats::note(|| {
                    let (l, n, m) = tt_render::textures::cache_bytes();
                    format!("texture caches after trim: layers {} MB, normal maps {} MB, mips {} MB", l >> 20, n >> 20, m >> 20)
                });
                tt_render::loadstats::milestone("scene + renderer ready", false);
                self.renderer = Some(r);
                self.fx = Some(gfx::FxState::new(&s));
                self.dynlights.set_scene(&s, &self.root);
                self.scene = Some(s);
            }
            Some(Err(e)) => engine.host.log.warn(&format!("scene: {e}")),
            None => {
                self.scene = None;
                self.fx = None;
                self.renderer = None;
            }
        }
    }

    /// Reads the per-frame prefs (FPS counter, dynamic lights, HUD size)
    /// again on the next frame.
    pub fn reload_prefs(&mut self) {
        self.fps_pref_in = 0;
    }

    /// Seconds since start for the HUD and menu animations.
    fn wall(&self) -> f64 {
        self.wall_override.unwrap_or_else(|| self.start.elapsed().as_secs_f64())
    }

    pub fn time(&self) -> f32 {
        self.time_override.unwrap_or_else(|| self.start.elapsed().as_secs_f32())
    }

    /// Draws one frame into `target`. `dt_ms` advances effects.
    pub fn draw(&mut self, engine: &mut Engine, target: Target, dt_ms: f32) {
        self.fps.frame();
        if self.fps_pref_in == 0 {
            self.fps_pref_in = FPS_PREF_FRAMES;
            let v = engine.host.shell.prefs.get("graphics.show_fps");
            self.show_fps = v.trim().eq_ignore_ascii_case("true") || tt_game::fmt::atof(&v) != 0.0;
            let v = engine.host.shell.prefs.get("graphics.dynamic_lights");
            let prefs = &engine.host.shell.prefs;
            self.dynamic_light_strength = crate::engine::pref_f32(prefs, "graphics.dynamic_light_strength", 1.0).clamp(0.0, 2.0);
            self.hud_scale = crate::engine::pref_f32(prefs, "graphics.hud_scale", tt_shell::prefs::HUD_SCALE_DEFAULT).clamp(0.5, 1.5);
            self.dynamic_lights = if v.is_empty() { tt_shell::prefs::GRAPHICS_DEFAULTS.dynamic_lights } else { v.trim().eq_ignore_ascii_case("true") || tt_game::fmt::atof(&v) != 0.0 };
        }
        self.fps_pref_in -= 1;
        self.draw_view(engine, target, dt_ms);
        if self.show_fps && !self.free_fly {
            self.fps.draw(&mut self.gui, target, self.hud_scale);
        }
    }

    fn draw_view(&mut self, engine: &mut Engine, target: Target, dt_ms: f32) {
        let Target { device, queue, view, size } = target;
        let t = self.time();
        if self.free_fly {
            match &mut self.renderer {
                Some(r) => r.render(target, &self.camera, t),
                None => gfx::clear(device, queue, view),
            }
            return;
        }
        let menu_visible = engine.menu.as_ref().is_some_and(|m| m.visible(&engine.host));
        if engine.menu.as_ref().is_some_and(|m| m.covers()) {
            // Shell: the menu is the whole picture.
            gfx::clear(device, queue, view);
            self.draw_menu(engine, target);
            return;
        }
        let tv = Instant::now();
        // Camera aim limits looking up by the view's vertical field of view.
        let half_vfov = tt_render::vertical_fov(engine.camera_fov(), size.0.max(1) as f32 / size.1.max(1) as f32).to_radians() * 0.5;
        if let Some(g) = engine.host.game.as_mut() {
            g.free_aim.half_vfov = half_vfov;
        }
        let aim_mark = engine.aim_mark();
        let mut game_view = if self.scene.is_some() { engine.game_view() } else { None };
        if let (Some(c), Some(gv)) = (self.camera_override, game_view.as_mut()) {
            gv.camera = c;
            if let Some(f) = self.fov_override {
                gv.hfov = f;
            }
        }
        self.perf.add("game view", tv.elapsed().as_secs_f64() * 1000.0);
        if let Some(gv) = &game_view {
            self.camera = Camera { pos: gv.camera.pos, yaw: gv.camera.yaw, pitch: gv.camera.pitch };
            let f = self.camera.forward();
            engine.host.audio.set_listener(self.camera.pos.into(), f.into(), Vec3::Z.into());
        }
        let explosions = engine.take_explosions();
        if let (Some(fx), Some(gv)) = (self.fx.as_mut(), game_view.as_ref()) {
            let t = Instant::now();
            // Explosion sounds are played by the simulation glue already.
            fx.effects.set_detail(engine.particle_detail(), Some(gv.camera.pos));
            fx.update(&explosions, &gv.smoke, whole_ms(&mut self.fx_carry_ms, self.sim_dt_ms.min(dt_ms)));
            self.perf.add("effects update", t.elapsed().as_secs_f64() * 1000.0);
        }
        if let (Some(r), Some(gv)) = (self.renderer.as_mut(), game_view.as_ref()) {
            if self.dynamic_lights {
                let projectiles = engine.light_projectiles();
                let lib = self.fx.as_ref().map(|f| f.effects.library());
                let sources = crate::dynlights::Sources { explosions: &explosions, fx: lib, glows: &gv.triggers, anims: &gv.trigger_anim, projectiles: &projectiles };
                r.set_point_lights(self.dynlights.update(self.sim_dt_ms.min(dt_ms) / 1000.0, sources, gv.camera.pos, self.dynamic_light_strength));
            } else {
                r.set_point_lights(&[]);
            }
        }
        // The scene's will-o'-the-wisps (dynlights' sprites).
        if let Some(gv) = game_view.as_mut() {
            gv.sprites.extend(self.dynlights.wisp_sprites());
        }
        let th = Instant::now();
        let hud = engine.hud_view(size, (self.wall() * 1000.0) as u64, game_view.as_ref().map(|g| (&g.camera, g.hfov)));
        self.perf.add("hud", th.elapsed().as_secs_f64() * 1000.0);
        // The 3D view fills the window while the game screen is up.
        if !engine.host.canvas.is_awake(crate::canvas::Ctrl::PlayGui) {
            game_view = None;
        }
        // Camera aim's mark at the centre of the view.
        let gui = (aim_mark && !self.hide_hud && game_view.is_some()).then(|| crate::gui_gpu::GuiFrame { texts: &[], size, pixel_fills: aim_mark_fills([0, 0, size.0, size.1], size) });
        let tr = Instant::now();
        let game = self.renderer.as_mut().zip(self.scene.as_ref()).zip(game_view.as_mut()).map(|((r, s), gv)| (r, s, gv));
        gfx::draw_game_frame(target, game, self.fx.as_ref(), gui, &mut self.gui, t, &mut self.perf);
        self.perf.add("draw (cpu)", tr.elapsed().as_secs_f64() * 1000.0);
        if let Some(v) = hud.as_ref().filter(|_| !self.hide_hud) {
            let th = Instant::now();
            let format = self.format;
            let root = &self.root;
            let wall = self.wall();
            let layer = self.hud.get_or_insert_with(|| crate::menu_gpu::HudLayer::new(device, format, root));
            layer.draw(target, v, wall, self.hud_scale);
            self.perf.add("hud draw", th.elapsed().as_secs_f64() * 1000.0);
        }
        if self.first_frame && game_view.is_some() {
            self.first_frame = false;
            tt_render::loadstats::milestone("first game frame drawn", true);
        }
        if menu_visible {
            self.draw_menu(engine, target);
        }
    }

    /// Runs a menu frame and paints it over `target`.
    fn draw_menu(&mut self, engine: &mut Engine, target: Target) {
        let Target { device, queue, size, .. } = target;
        let t = Instant::now();
        let format = self.format;
        let wall = self.wall();
        let layer = self.menu.get_or_insert_with(|| crate::menu_gpu::MenuLayer::new(device, format));
        layer.set_size(size);
        let mut raw = self.menu_input.take().unwrap_or_else(|| crate::menu::raw_input(size, Vec::new(), 0.0));
        raw.time = Some(wall);
        let mut out = layer.run(engine, raw);
        self.menu_output = Some(std::mem::take(&mut out.platform_output));
        layer.paint(target, out);
        let time = wall as f32;
        layer.render_preview(device, queue, engine, time);
        self.perf.add("menu", t.elapsed().as_secs_f64() * 1000.0);
    }
}

/// Whole milliseconds of `carry + ms`, keeping the fraction in `carry`.
fn whole_ms(carry: &mut f32, ms: f32) -> u32 {
    let total = *carry + ms.max(0.0);
    let whole = total.floor();
    *carry = total - whole;
    whole as u32
}

/// Camera aim's mark at the centre of the game viewport `vp` (x, y, w, h in
/// target pixels): a 2×2 white dot in a 1 px dark outline, at whole pixels
/// and scaled only by whole factors on large targets (×2 from 1800 px high).
pub fn aim_mark_fills(vp: [u32; 4], target: (u32, u32)) -> Vec<([u32; 4], [u8; 4])> {
    let k = (target.1 / 900).max(1);
    let (cx, cy) = (vp[0] + vp[2] / 2, vp[1] + vp[3] / 2);
    let (dot, rim) = (2 * k, 4 * k);
    vec![([cx - rim / 2, cy - rim / 2, rim, rim], [0, 0, 0, 200]), ([cx - dot / 2, cy - dot / 2, dot, dot], [255, 255, 255, 255])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_ms_keeps_fractions() {
        let mut carry = 0.0;
        // 1000 frames of 0.4 ms (2500 fps) are 400 ms, not 0.
        let total: u32 = (0..1000).map(|_| whole_ms(&mut carry, 0.4)).sum();
        assert!((399..=400).contains(&total), "{total}");
    }

    #[test]
    fn perf_ring_is_bounded() {
        let mut p = Perf::default();
        for i in 0..PERF_SAMPLES + 10 {
            p.add("x", i as f64);
        }
        assert_eq!(p.samples["x"].len(), PERF_SAMPLES);
        assert_eq!(p.latest().next(), Some(("x", PERF_SAMPLES as u64 + 10, (PERF_SAMPLES + 9) as f64)));
    }

    #[test]
    fn aim_mark_is_pixel_exact_at_the_view_centre() {
        // 1280×720: a 2×2 dot on the four centre pixels in a 4×4 rim.
        let f = aim_mark_fills([0, 0, 1280, 720], (1280, 720));
        assert_eq!(f[0].0, [638, 358, 4, 4]);
        assert_eq!(f[1].0, [639, 359, 2, 2]);
        assert_eq!(f[1].1, [255, 255, 255, 255]);
        // 4K: twice the size, still whole pixels; centred in a sub-viewport.
        let f = aim_mark_fills([100, 50, 3640, 2060], (3840, 2160));
        assert_eq!(f[1].0, [1918, 1078, 4, 4]);
        assert_eq!(f[0].0, [1916, 1076, 8, 8]);
    }
}

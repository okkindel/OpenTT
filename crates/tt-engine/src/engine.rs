//! The engine: the host, the boot sequence and the fixed-rate sim tick.

use crate::audio::{Audio, MixerAudio};
use crate::fs::GameFs;
use crate::host::EngineHost;
use crate::input::{Device, InputEvent};
use crate::log::{LogLevel, Logger};
use std::path::PathBuf;

pub const TICK_MS: u32 = 32;

/// A numeric pref (C `atof`), `default` when it is unset or empty.
pub fn pref_f32(prefs: &tt_shell::Prefs, name: &str, default: f32) -> f32 {
    let v = prefs.get(name);
    if v.is_empty() {
        default
    } else {
        tt_game::fmt::atof(&v) as f32
    }
}

#[derive(Clone)]
pub struct Config {
    /// The asset directory (`assets/`, [`tt_assets::layout`]): all game data.
    pub assets: PathBuf,
    /// User data (settings, players, high scores, logs; docs/layout.md § User data).
    pub data: PathBuf,
    pub args: Vec<String>,
    pub audio_output: bool,
    pub echo_log: bool,
    pub windowed: bool,
}

pub struct Engine {
    pub host: EngineHost,
    pub ticks: u64,
    pub booted: bool,
    /// Timings of the last tick.
    pub last_tick: TickPerf,
    /// Sum of the tick timings since the window loop last reset it (one frame).
    pub frame_ticks: TickPerf,
    /// First reticle dot drawn in the last frame (frame-stats motion probe).
    pub last_reticle_dot: Option<glam::Vec3>,
    /// Fraction (0..1) of the next tick already elapsed in real time; render
    /// poses are interpolated between the last two ticks by it.
    pub render_alpha: f32,
    /// Real time (ms, [`crate::host::EngineHost::now_ms_f64`]) of the frame
    /// being drawn, set by the frame loop; None outside it.
    pub frame_time_ms: Option<f64>,
    /// HUD animations are advanced in real time by the frontend; headless
    /// runs tick them with the sim.
    pub gui_realtime: bool,
    /// The menus (tt-menu), created once the boot sequence has run.
    pub menu: Option<crate::menu::MenuBridge>,
    pub args: Vec<String>,
}

/// Split of one tick's time (ms).
#[derive(Debug, Clone, Copy, Default)]
pub struct TickPerf {
    /// Game logic: packet delivery, timed events, HUD animation, audio.
    pub script_ms: f64,
    /// tt-game world tick and the game's reactions to it.
    pub sim_ms: f64,
}

impl Engine {
    pub fn new(cfg: &Config) -> Result<Engine, String> {
        if !cfg.assets.join("missions").is_dir() {
            return Err(format!("{}: no game data (missions/) found; point --assets at the asset directory", cfg.assets.display()));
        }
        // An older layout of the data directory into the current one,
        // before anything reads or writes there (the log file included).
        let migrated = crate::userdata::migrate(&cfg.data, &cfg.assets);
        // The enabled mods overlay the asset directory for every loader
        // from here on (docs/modding.md).
        let (mods, mods_log) = crate::mods::load(&cfg.data);
        let fs = GameFs::with_mods(&cfg.assets, &cfg.data, mods);
        let mut log = Logger::new(Some(&cfg.data), cfg.echo_log);
        for l in &mods_log {
            log.info(l);
        }
        for l in &migrated.done {
            log.info(&format!("user data: {l}"));
        }
        for l in &migrated.errors {
            log.warn(&format!("user data: {l}"));
        }
        let audio: Box<dyn Audio> = Box::new(MixerAudio::new(&cfg.assets, cfg.audio_output));
        let mut host = EngineHost::new(fs, log, audio);
        host.video.force_windowed = cfg.windowed;
        // The engine's always-active action map.
        host.input.new_map(crate::input::GLOBAL_MAP);
        // A test run (`-test` first) opens no network port.
        if cfg.args.first().is_some_and(|a| a.eq_ignore_ascii_case("-test")) {
            host.srv.state_mut().setup.skip_port = true;
        }
        Ok(Engine { host, ticks: 0, booted: false, last_tick: TickPerf::default(), frame_ticks: TickPerf::default(), last_reticle_dot: None, render_alpha: 1.0, frame_time_ms: None, gui_realtime: false, menu: None, args: cfg.args.clone() })
    }

    /// The boot sequence.
    pub fn boot(&mut self) -> bool {
        let line = format!("Data directory: {}", self.host.fs.data_dir().display());
        self.host.log.info(&line);
        self.booted = crate::boot::run(&mut self.host, &self.args);
        // The client's own defs survive deleteDefs.
        self.host.defs.mark_persistent();
        if self.booted {
            crate::menu::install(self);
        }
        self.booted
    }

    /// One simulation tick: packets, timed events, the world, HUD animation, audio.
    pub fn tick(&mut self) {
        // "Packets" sent since the last tick arrive first.
        let t0 = std::time::Instant::now();
        let h = &mut self.host;
        crate::netgame::poll(h);
        crate::net::deliver(h);
        run_events(h, TICK_MS);
        crate::hud::client_tick(h);
        let t1 = std::time::Instant::now();
        crate::netgame::replicate::pre_tick(h);
        crate::game::tick(h);
        let t2 = std::time::Instant::now();
        self.ticks += 1;
        if !self.gui_realtime {
            crate::hud::client_animate(&mut self.host, TICK_MS);
        }
        for e in self.host.audio.update() {
            self.host.log.log(LogLevel::Warning, &format!("audio: {e}"));
        }
        crate::netgame::flush(&mut self.host);
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        self.last_tick = TickPerf { script_ms: ms(t1 - t0) + ms(t2.elapsed()), sim_ms: ms(t2 - t1) };
        self.frame_ticks.script_ms += self.last_tick.script_ms;
        self.frame_ticks.sim_ms += self.last_tick.sim_ms;
    }

    /// Shapes and sprite textures the running game can show: every tank
    /// def's shape and brain jar, projectile resources, power-up/pad/
    /// ball shapes. Loaded with the scene to avoid hitches on first use.
    pub fn preload_assets(&self) -> (Vec<std::path::PathBuf>, Vec<std::path::PathBuf>) {
        let (mut shapes, mut textures) = (Vec::new(), Vec::new());
        let dir = crate::mission::mission_dir(&self.host);
        for o in self.host.defs.objects() {
            for f in ["shape", "brain_shape", "resource"] {
                let v = o.get(f).unwrap_or("");
                if v.is_empty() {
                    continue;
                }
                let p = tt_assets::resolve(self.host.fs.root(), &dir, v);
                let lower = v.to_ascii_lowercase();
                if lower.ends_with(".dts") {
                    shapes.push(p);
                } else if lower.ends_with(".png") || lower.ends_with(".jpg") {
                    textures.push(p);
                }
            }
        }
        shapes.sort();
        shapes.dedup();
        textures.sort();
        textures.dedup();
        (shapes, textures)
    }

    /// (the quit and target-practice dialogs pause the game with
    /// 0.00001): the rate sim time advances relative to real time.
    pub fn time_scale(&self) -> f32 {
        let v = self.host.cl.time_scale_text();
        if v.is_empty() {
            1.0
        } else {
            (tt_game::fmt::atof(&v) as f32).clamp(0.0, 100.0)
        }
    }

    /// Real-time HUD animation step (see [`Engine::gui_realtime`]).
    pub fn gui_frame(&mut self, ms: u32) {
        crate::hud::client_animate(&mut self.host, ms);
    }

    /// Action maps only, e.g. for mouse axes.
    pub fn input(&mut self, ev: &InputEvent) {
        dispatch_input(&mut self.host, ev, true, true);
    }

    /// Keyboard: `GlobalActionMap`, then the open chat input, then the
    /// active action maps.
    pub fn key(&mut self, key: &str, pressed: bool, text: Option<&str>) {
        self.host.input.update_modifiers(key, pressed);
        let ev = InputEvent::Button { device: Device::Keyboard, key: key.to_string(), pressed };
        if dispatch_input(&mut self.host, &ev, true, false) {
            return;
        }
        // The open chat input takes the keys first (the GUI's first responder).
        if crate::hud::chat_key(&mut self.host, key, pressed, text) {
            return;
        }
        dispatch_input(&mut self.host, &ev, false, true);
    }

    /// Mouse button: nothing while the cursor is on (the menus have their
    /// own input), otherwise to the action maps.
    pub fn mouse_button(&mut self, button: u8, pressed: bool) {
        if !self.host.canvas.cursor_on {
            self.input(&InputEvent::Button { device: Device::Mouse, key: format!("button{button}"), pressed });
        }
    }

    /// Mouse wheel: the `zaxis` binding while the cursor is off.
    pub fn mouse_wheel(&mut self, dy: f32) {
        if !self.host.canvas.cursor_on {
            self.input(&InputEvent::Axis { device: Device::Mouse, axis: "zaxis".into(), delta: dy });
        }
    }

    /// A setting (`player.name`, `server.max_players`, …) or player list
    /// entry (`players.…`) as text ("" when unset).
    pub fn var(&self, name: &str) -> String {
        if name.len() > 8 && name[..8].eq_ignore_ascii_case("players.") {
            return self.host.shell.players.vars.get(name);
        }
        self.host.shell.prefs.get(name)
    }

    /// Sets a setting or player list entry.
    pub fn set_var(&mut self, name: &str, value: &str) {
        if name.len() > 8 && name[..8].eq_ignore_ascii_case("players.") {
            self.host.shell.players.vars.set_value(name, tt_shell::VarValue::Str(value.to_string()));
        } else if tt_shell::settings::is_key(name) {
            crate::shell::set_pref_val(&mut self.host.shell.prefs, name, tt_gameplay::client::value::Val::s(value));
        } else {
            // The game's state is typed (`Engine::server`), not named.
            self.host.log.error(&format!("set_var: {name} is not a setting"));
            debug_assert!(false, "set_var: {name} is not a setting");
        }
    }

    /// The game server's state (between its calls).
    pub fn server(&self) -> &tt_gameplay::server::Server {
        self.host.srv.state()
    }

    /// The client's control object (the player's tank entity), if the game is running.
    pub fn control_object(&self) -> Option<tt_gameplay::EntityId> {
        crate::net::client_control_object(&self.host)
    }

    /// Camera for the game view: behind and above the control object, looking
    /// along its heading (placeholder until the tank camera from tt-game).
    pub fn view_camera(&mut self) -> Option<tt_render::Camera> {
        let obj = self.control_object()?;
        let now = self.frame_time_ms.unwrap_or_else(|| self.host.now_ms_f64());
        let alpha = self.render_alpha;
        let aim = crate::game::aim_mode(&self.host.shell.prefs);
        let m = self.host.game.as_mut().and_then(|g| g.camera_transform(aim, obj, now, alpha))?;
        Some(tt_render::Camera::from_matrix(glam::Mat4::from(m)))
    }

    /// The game view of the controlled tank, if a game is running.
    pub fn game_view(&mut self) -> Option<crate::gfx::GameView> {
        let camera = self.view_camera()?;
        let mut sprites = self.sprite_projectiles();
        sprites.extend(self.reticle_sprites());
        let (dynamic, ifl) = self.dynamic_shapes_with_ifl();
        let (triggers, trigger_anim) = self.trigger_shapes_with_anim();
        Some(crate::gfx::GameView { camera, hfov: self.camera_fov(), dynamic, ifl, triggers, trigger_anim, sprites, smoke: self.smoke_emitters() })
    }

    /// The local player's reticle (predicted trajectory dots and impact
    /// spots) from the current cartridge's `ReticleDef`, as upright quads
    /// perpendicular to each sprite's `dir` (the game's reticle quads).
    pub fn reticle_sprites(&mut self) -> Vec<tt_render::Sprite> {
        let Some(obj) = self.control_object() else { return Vec::new() };
        let alpha = self.render_alpha;
        let Some(g) = self.host.game.as_mut() else { return Vec::new() };
        let Some(id) = g.tank_of(obj) else { return Vec::new() };
        let Some(name) = g.world.tank(id).and_then(|t| t.current.as_ref()).map(|p| p.reticle.clone()).filter(|r| !r.is_empty()) else { return Vec::new() };
        let Some(rd) = g.db.reticle(&name) else { return Vec::new() };
        let now = g.world.time_ms + (alpha * tt_game::TICK_MS as f32) as i64;
        // From the tank as drawn (interpolated pose and aim), so the dots
        // stay attached to the barrel between ticks.
        let Some(tank) = g.world.tank(id).map(|t| g.render_tank(t, alpha)) else { return Vec::new() };
        let Some(r) = g.world.reticle_of(&tank, &rd, &mut g.reticle_state, now) else { return Vec::new() };
        let tex = |b: &str| tt_assets::resolve(&g.root, &g.mission_dir, b);
        let sprite = |s: &tt_game::ReticleSprite, texture: std::path::PathBuf| tt_render::Sprite {
            texture,
            pos: s.pos,
            size: s.size,
            orient: tt_render::sprites::SpriteOrient::Axis { dir: s.dir },
            ..Default::default()
        };
        self.last_reticle_dot = r.dots.first().map(|d| d.pos);
        // Splash rings lie on the surface (the mortar's blast area).
        let mut out: Vec<tt_render::Sprite> = r
            .rings
            .iter()
            .map(|s| {
                let up = r.dots.first().map_or(glam::Vec3::Y, |d| -d.dir);
                tt_render::Sprite { texture: tex(&r.ring_image), pos: s.pos, size: s.size, orient: tt_render::sprites::SpriteOrient::Plane { normal: s.dir, up }, depth_offset: 0.4, ..Default::default() }
            })
            .collect();
        out.extend(r.spots.iter().map(|s| sprite(s, tex(&r.spot_image))));
        out.extend(r.dots.iter().map(|s| sprite(s, tex(&r.image))));
        out
    }

    /// Camera aim: the free camera is on (the frontend then draws the aim
    /// mark at the centre of the game view).
    pub fn aim_mark(&self) -> bool {
        self.host.game.as_ref().is_some_and(|g| g.free_aim.tank.is_some()) && crate::game::aim_mode(&self.host.shell.prefs) == crate::game::AimMode::Camera
    }

    /// What the player sees move, for the frame-stats smoothness probes:
    /// the player tank's drawn position, its turret aim (yaw, pitch as
    /// drawn) and the first reticle dot relative to the drawn tank.
    pub fn motion_probe(&self) -> Vec<(&'static str, glam::Vec3)> {
        let mut out = Vec::new();
        let Some(obj) = self.control_object() else { return out };
        let Some(g) = self.host.game.as_ref() else { return out };
        let Some(t) = g.tank_of(obj).and_then(|id| g.world.tank(id)) else { return out };
        let (pos, _) = g.render_pose(t, self.render_alpha);
        out.push(("tank", pos));
        let (yaw, pitch) = g.render_aim(t, self.render_alpha);
        out.push(("turret", glam::Vec3::new(yaw, pitch, 0.0)));
        if let Some(d) = self.last_reticle_dot {
            out.push(("reticle-tank", d - pos));
        }
        out
    }

    pub fn camera_fov(&self) -> f32 {
        if let Some(f) = self.host.camera_fov {
            return f.clamp(1.0, 179.0);
        }
        let v = self.host.cl.camera_fov_num() as f32;
        if v > 0.0 {
            return v.clamp(1.0, 179.0);
        }
        self.control_object().and_then(|o| self.host.game.as_ref().and_then(|g| g.tank_of(o).map(|t| g.camera_data(t).fov))).unwrap_or(crate::game::CameraParams::default().fov)
    }

    /// Moving shapes of the simulation, posed: tanks (turret yaw, barrel
    /// pitch, wheel spin, muzzle flash from the `fire` sequence, `death1-3`
    /// on corpses), `.dts` projectiles (brain jars, volcano rocks) and scrum
    /// balls. Sprite projectiles (`.png`) are not drawn. `light` is filled in
    /// by the renderer glue.
    pub fn dynamic_shapes(&self) -> Vec<tt_render::DynamicInstance> {
        self.dynamic_shapes_with_ifl().0
    }

    /// [`Engine::dynamic_shapes`] plus each instance's IFL material times
    /// (tank treads: the `LTread`/`RTread` sequences at each side's clock).
    pub fn dynamic_shapes_with_ifl(&self) -> (Vec<tt_render::DynamicInstance>, Vec<Vec<(String, f32)>>) {
        let Some(g) = &self.host.game else { return (Vec::new(), Vec::new()) };
        let resolve = |p: &str| tt_assets::resolve(&g.root, &g.mission_dir, p);
        let mut out = Vec::new();
        let mut ifl: Vec<Vec<(String, f32)>> = Vec::new();
        for t in &g.world.tanks {
            if t.deleted || t.data.shape_file.is_empty() {
                continue;
            }
            let path = resolve(&t.data.shape_file);
            let (pos, rot) = g.render_pose(t, self.render_alpha);
            let mut inst = tt_render::DynamicInstance { shape: path.clone(), transform: glam::Mat4::from_mat3_translation(rot, pos), ..Default::default() };
            if let Some(shape) = tt_render::pose::shape(&path) {
                crate::game::pose_tank(g, t, self.render_alpha, &shape, &mut inst);
            }
            // The brain jar sits on the turret (`codeSeat`, following the
            // turret pose) until the brain is ejected from the corpse.
            let brain = (!t.data.brain_shape.is_empty() && !t.brain_ejected)
                .then(|| tt_render::objectview::mounted(&inst, &resolve(&t.data.brain_shape), "codeSeat"))
                .flatten();
            // Tread IFLs: the `LTread` / `RTread` threads at their positions.
            let mut treads = Vec::new();
            if let Some(tr) = g.treads.get(&t.id) {
                for (seq, time) in crate::treads::SEQUENCES.iter().zip(tr.sequence_times(self.render_alpha)) {
                    if let Some(time) = time {
                        treads.extend(tt_render::pose::sequence_ifl(&path, seq, time));
                    }
                }
            }
            // A corpse's death thread, added after the tread threads, sets
            // the IFLs it animates (`death3` holds both tread textures).
            if let (true, Some((k, _))) = (t.is_corpse, t.death_sequence) {
                let time = (t.corpse_timer - t.data.fx.corpse_shake_time).max(0.0);
                treads.extend(tt_render::pose::sequence_ifl(&path, &format!("death{}", k + 1), time));
            }
            out.push(inst);
            ifl.push(treads);
            if let Some(b) = brain {
                out.push(b);
                ifl.push(Vec::new());
            }
        }
        for p in &g.world.projectiles {
            // Ejected brain jars fly as the tank's jar shape, upright.
            let brain = g.brain_shapes.get(&p.id);
            // Sprite projectiles with a `display_shape` (the mortar
            // rocket) are drawn as that mesh, nose along the flight.
            let display = !p.data.is_dts() && !p.data.display_shape.is_empty();
            if !projectile_mesh_drawn(p, brain.is_some()) {
                continue;
            }
            let pos = p.render_pos(self.render_alpha);
            let along = glam::Quat::from_rotation_arc(glam::Vec3::Y, p.vel.try_normalize().unwrap_or(glam::Vec3::Y));
            let (shape, rot) = match brain {
                Some(b) => (resolve(b), glam::Quat::IDENTITY),
                None if display => (resolve(&p.data.display_shape), along),
                None => (resolve(&p.data.resource), along),
            };
            // A projectile's shape plays its first sequence from the spawn:
            // the brain jar's `rocket` lights its `exhaust` flame.
            let time = ((g.world.time_ms - p.born_ms) as f32 + self.render_alpha * tt_game::TICK_MS as f32) / 1000.0;
            let mut inst = tt_render::DynamicInstance { shape, transform: glam::Mat4::from_rotation_translation(rot, pos), ..Default::default() };
            ifl.push(first_sequence(&mut inst, time));
            out.push(inst);
        }
        // Scrum balls play their first sequence (`ambient`: the flag bones
        // wave, the ball's IFL turns) from the start, looping: Flag adds a
        // thread on sequence 0 and advances it.
        let t = (g.world.time_ms as f32 + self.render_alpha * tt_game::TICK_MS as f32) / 1000.0;
        for f in &g.world.flags {
            if !f.data.shape_file.is_empty() {
                let mut inst = tt_render::DynamicInstance { shape: resolve(&f.data.shape_file), transform: glam::Mat4::from_translation(g.render_flag_pos(f, self.render_alpha)), ..Default::default() };
                ifl.push(first_sequence(&mut inst, t));
                out.push(inst);
            }
        }
        (out, ifl)
    }

    /// Power-ups, boost/jump pads and the scrum goal as animated moving
    /// shapes (the scene leaves them out). Power-ups play their cyclic
    /// `ambient` sequence: nodes, the lamp IFL (`powerup.ifl` /
    /// `health.ifl`: dark, a flash, dark) and the `Flash` object's
    /// visibility, all at one time. Boost/jump pads take the pose of their
    /// current state's sequence (`stateName<N>`, else `ambient`) and their
    /// arrow IFL from `ambient`. Switched-off power-ups are not drawn.
    pub fn trigger_shapes(&self) -> Vec<tt_render::DynamicInstance> {
        self.trigger_shapes_with_anim().0
    }

    /// [`Engine::trigger_shapes`] with each one's IFL times and lamp (the
    /// dynamic lights follow the same animation).
    /// A power-up's user thread (the Reload arch's lightning) is laid
    /// over its ambient animation, see [`trigger_pose`]; so is its
    /// `activate` thread after an on/off switch (the pick-up grows in when
    /// it appears and shrinks away when taken, still drawn while it does).
    pub fn trigger_shapes_with_anim(&self) -> (Vec<tt_render::DynamicInstance>, Vec<crate::gfx::TriggerAnim>) {
        let Some(g) = &self.host.game else { return Default::default() };
        // Animation time between ticks, like the interpolated poses.
        let t = (g.world.time_ms as f32 + self.render_alpha * tt_game::TICK_MS as f32) / 1000.0;
        let (mut out, mut anims) = (Vec::new(), Vec::new());
        for tr in &g.world.triggers {
            let (shape, seq, lamp) = match &tr.kind {
                // The Reload arch (type "ammo") has no lamp.
                tt_game::TriggerKind::PowerUp(d) => {
                    let lamp = if d.kind.eq_ignore_ascii_case("ammo") { crate::gfx::Lamp::None } else { crate::gfx::Lamp::PowerUp };
                    (d.shape.clone(), "ambient".to_string(), lamp)
                }
                tt_game::TriggerKind::Boost(b) => {
                    let name = b.data.state_name.get(b.state).cloned().unwrap_or_default();
                    (b.data.base.shape.clone(), if name.is_empty() { "ambient".to_string() } else { name }, crate::gfx::Lamp::Pad)
                }
                tt_game::TriggerKind::Volcano(_) => continue,
            };
            if shape.is_empty() {
                continue;
            }
            let path = tt_assets::resolve(&g.root, &g.mission_dir, &shape);
            // The `activate` thread: only power-ups switch; an off one is
            // drawn while it shrinks away.
            let activate = match &tr.kind {
                tt_game::TriggerKind::PowerUp(_) => tt_render::pose::shape(&path)
                    .and_then(|s| s.sequences.iter().find(|q| q.name.eq_ignore_ascii_case("activate")).map(|q| q.duration))
                    .and_then(|dur| g.activation_time(tr.id, t, dur)),
                _ => None,
            };
            if !tr.on && activate.is_none() {
                continue;
            }
            let mut threads = Vec::new();
            threads.extend(activate.map(|a| ("activate", a)));
            threads.extend(tr.thread.as_ref().map(|th| (th.seq.as_str(), th.time_at(t))));
            let (nodes, hidden, shown, ifl) = trigger_pose(&path, &seq, t, &threads);
            out.push(tt_render::DynamicInstance { shape: path, transform: glam::Mat4::from(tr.transform), nodes, hidden, shown, ..Default::default() });
            anims.push(crate::gfx::TriggerAnim { ifl, lamp });
        }
        (out, anims)
    }

    /// `.png` projectiles as camera-facing sprites (`sprite_scale` metres).
    /// `trail_length` adds a tracer streak: smaller, fading copies behind
    /// the projectile along its flight (no further back than it has flown),
    /// drawn additively.
    pub fn sprite_projectiles(&self) -> Vec<tt_render::sprites::Sprite> {
        const TRAIL: usize = 5;
        let Some(g) = &self.host.game else { return Vec::new() };
        let mut out = Vec::new();
        for p in g.world.projectiles.iter().filter(|p| p.is_live() && !p.data.is_dts() && !p.data.resource.is_empty()) {
            let texture = tt_assets::resolve(&g.root, &g.mission_dir, &p.data.resource);
            let pos = p.render_pos(self.render_alpha);
            let speed = p.vel.length();
            if p.data.trail_length > 0.0 && speed > 1e-3 {
                let flown = speed * ((g.world.time_ms - p.born_ms) as f32 / 1000.0 + self.render_alpha * tt_game::DT);
                let len = p.data.trail_length.min(flown);
                let back = -p.vel / speed;
                for k in (1..=TRAIL).rev() {
                    let f = k as f32 / TRAIL as f32;
                    out.push(tt_render::sprites::Sprite {
                        texture: texture.clone(),
                        pos: pos + back * len * f,
                        size: p.data.sprite_scale * (1.0 - 0.6 * f),
                        color: [1.0, 0.8, 0.45, 0.7 * (1.0 - f) + 0.1],
                        blend: tt_render::sprites::SpriteBlend::Additive,
                        ..Default::default()
                    });
                }
            }
            // An exhaust glow at the tail, flickering a little.
            if !p.data.exhaust_glow.is_empty() && p.data.exhaust_glow_size > 0.0 {
                let back = (-p.vel).try_normalize().unwrap_or(glam::Vec3::NEG_Z);
                let t = g.world.time_ms as f32 / 1000.0 + self.render_alpha * tt_game::DT + p.id.0 as f32 * 0.37;
                let flicker = 1.0 + 0.12 * (t * 37.0).sin() + 0.08 * (t * 61.0 + 1.3).sin();
                out.push(tt_render::sprites::Sprite {
                    texture: tt_assets::resolve(&g.root, &g.mission_dir, &p.data.exhaust_glow),
                    pos: pos + back * p.data.exhaust_offset,
                    size: p.data.exhaust_glow_size * flicker,
                    color: [1.0, 0.8, 0.45, 1.0],
                    blend: tt_render::sprites::SpriteBlend::Additive,
                    ..Default::default()
                });
            }
            // `display_shape`: drawn as a mesh (`dynamic_shapes`).
            if !p.data.display_shape.is_empty() {
                continue;
            }
            out.push(tt_render::sprites::Sprite { texture, pos, size: p.data.sprite_scale, ..Default::default() });
        }
        out
    }

    /// Live projectiles as dynamic-light sources: (id, render position,
    /// sprite texture or None for `.dts` projectiles, trailing emitter).
    pub fn light_projectiles(&self) -> Vec<(u32, glam::Vec3, Option<PathBuf>, &str)> {
        let Some(g) = &self.host.game else { return Vec::new() };
        g.world
            .projectiles
            .iter()
            .filter(|p| p.is_live())
            .map(|p| {
                let tex = (!p.data.is_dts() && !p.data.resource.is_empty()).then(|| tt_assets::resolve(&g.root, &g.mission_dir, &p.data.resource));
                (p.id.0, p.render_pos(self.render_alpha), tex, p.data.emitter.as_str())
            })
            .collect()
    }

    /// Damage smoke of tanks above `damage_emitter_start` (TankFxParams
    /// `damage_emitter` at the `damage_emitter_node`): (tank id, emitter,
    /// world transform, velocity).
    pub fn smoke_emitters(&self) -> Vec<(u32, String, glam::Mat4, glam::Vec3)> {
        let Some(g) = &self.host.game else { return Vec::new() };
        let mut out = Vec::new();
        for t in &g.world.tanks {
            if g.entity_of_tank(t.id).is_none() {
                continue;
            }
            let field = |n: &str| g.fx_value(t.id, n).unwrap_or("");
            let (emitter, node) = (field("damage_emitter"), field("damage_emitter_node"));
            let start = tt_game::fields::atof(field("damage_emitter_start"));
            if emitter.is_empty() || t.deleted || t.damage < start * t.data.damage_amount {
                continue;
            }
            let emitter = emitter.to_string();
            let path = tt_assets::resolve(&g.root, &g.mission_dir, &t.data.shape_file);
            let local = tt_render::pose::shape(&path).and_then(|s| tt_render::pose::node_transform(&s, &[], if node.is_empty() { "smoke1" } else { node })).unwrap_or(glam::Mat4::IDENTITY);
            let (pos, rot) = g.render_pose(t, self.render_alpha);
            out.push((t.id.0, emitter, glam::Mat4::from_mat3_translation(rot, pos) * local, t.vel));
        }
        // Projectile trails (`ProjectileParams.emitter`, e.g. the ejected brain
        // jar's RocketEmitter), at the drawn (interpolated) position. Keys
        // have the top bit set so they never collide with tank ids.
        for p in &g.world.projectiles {
            if !projectile_trails(p) {
                continue;
            }
            let mut pos = p.render_pos(self.render_alpha);
            let axis = (-p.vel).try_normalize().unwrap_or(glam::Vec3::NEG_Z);
            // Trailed from the tail (`exhaust_offset`, the mortar rocket's nozzle).
            pos += axis * p.data.exhaust_offset;
            let rot = glam::Quat::from_rotation_arc(glam::Vec3::Z, axis);
            out.push((p.id.0 | 0x8000_0000, p.data.emitter.clone(), glam::Mat4::from_rotation_translation(rot, pos), p.vel));
        }
        out
    }

    /// Explosions since the last call (def, position).
    pub fn take_explosions(&mut self) -> Vec<(String, glam::Vec3)> {
        self.host.game.as_mut().map(|g| std::mem::take(&mut g.explosions)).unwrap_or_default()
    }

    /// Saves a captured frame for the pending `screenShot(file)` request
    /// (RGBA8 rows) as PNG in the data dir, under the requested name or,
    /// if that file exists (`doScreenShot` restarts at `screenshot_00001`
    /// every session), the next free number. Logs the path to the console
    /// and centre-prints a short confirmation. Returns the path written.
    pub fn save_screenshot(&mut self, rgba: &[u8], size: (u32, u32)) -> Option<PathBuf> {
        let name = self.host.screenshot_request.take()?;
        let name = free_screenshot_name(&name, |p| self.host.fs.exists(p));
        let result = crate::gfx::encode_png(rgba, size).and_then(|png| self.host.fs.write(&name, &png, false).map_err(|e| e.to_string()));
        match result {
            Ok(full) => {
                self.host.log.info(&format!("Screenshot saved: {}", full.display()));
                if !self.host.client.center.active {
                    let msg = format!("Screenshot saved: {}", full.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default());
                    let now = self.host.sim_time() as u64;
                    self.host.client.center_print(&msg, "2", "1", now);
                }
                Some(full)
            }
            Err(e) => {
                self.host.log.warn(&format!("screenShot({name}): {e}"));
                None
            }
        }
    }

    pub fn render_settings(&self) -> tt_render::render::RenderSettings {
        let pref = |n: &str, d: f32| pref_f32(&self.host.shell.prefs, n, d);
        let scale = pref("video.render_scale", 1.0).clamp(0.25, 1.0);
        let aniso = pref("video.anisotropy", 1.0).clamp(0.0, 1.0);
        tt_render::render::RenderSettings { render_scale: scale, anisotropy: (1.0 + aniso * 15.0).round() as u16, detail: self.detail_settings(), extras: self.graphics_extras() }
    }

    /// Unset = the defaults of tt-shell's `Prefs`.
    pub fn graphics_extras(&self) -> tt_render::Extras {
        let num = |n: &str, d: f32| pref_f32(&self.host.shell.prefs, n, d);
        let d = tt_shell::Prefs::defaults();
        tt_render::Extras {
            glass: num("graphics.glass_jar", d.glass_jar() as u8 as f32) != 0.0,
            hdr: num("graphics.hdr", d.hdr() as u8 as f32) != 0.0,
            bloom: num("graphics.bloom", d.bloom()).clamp(0.0, 1.0),
            normal_maps: num("graphics.normal_maps", d.normal_maps() as u8 as f32) != 0.0,
            normal_strength: num("graphics.normal_strength", d.normal_strength()).clamp(0.0, 2.0),
            ao_strength: num("graphics.ao_strength", d.ao_strength()).clamp(0.0, 2.0),
            shadows: num("graphics.shadows", d.shadows() as f32).clamp(0.0, 2.0) as u8,
            ssao: num("graphics.ssao", d.ssao() as f32).clamp(0.0, 2.0) as u8,
        }
    }

    /// The detail prefs `setThinkTanksDetailLevel` sets.
    pub fn detail_settings(&self) -> tt_render::render::Detail {
        let pref = |n: &str, d: f32| pref_f32(&self.host.shell.prefs, n, d);
        let level = |n: &str| pref(n, 0.0).clamp(0.0, 8.0) as u32;
        tt_render::render::Detail {
            ts_detail_adjust: pref("shapes.detail_adjust", 1.0).clamp(0.01, 10.0),
            skip_render_dls: level("shapes.skip_render_dls"),
            shadow_detail: pref("shapes.shadow_detail", 1.0),
            mip_reduction: level("opengl.mip_reduction"),
            sky_mip_reduction: level("opengl.sky_mip_reduction"),
            terrain_tex_detail: level("terrain.tex_detail"),
        }
    }

    pub fn particle_detail(&self) -> f32 {
        pref_f32(&self.host.shell.prefs, "particle.detail_adjust", 1.0)
    }

    /// Sizes of everything that could accumulate during a match (leak checks).
    pub fn counters(&self) -> Vec<(&'static str, usize)> {
        let h = &self.host;
        let g = h.game.as_ref();
        vec![
            ("timed events", h.sched.pending()),
            ("audio sources", h.audio.source_count()),
            ("log lines", h.log.history.len()),
            ("hud message lines", h.client.messages.lines.len()),
            ("client timers", h.client.pending_timers()),
            ("sim tanks", g.map_or(0, |g| g.world.tanks.len())),
            ("sim projectiles", g.map_or(0, |g| g.world.projectiles.len())),
            ("live projectiles", g.map_or(0, |g| g.world.projectiles.iter().filter(|p| p.is_live()).count())),
            ("brain projectiles", g.map_or(0, |g| g.world.projectiles.iter().filter(|p| p.data.name.to_ascii_lowercase().contains("brain")).count())),
            ("tank map", g.map_or(0, |g| g.entities.tanks())),
            ("engine sounds", g.map_or(0, |g| g.engine_sounds.len())),
            ("pending explosions", g.map_or(0, |g| g.explosions.len())),
            ("prev poses", g.map_or(0, |g| g.prev_pose.len())),
            ("cameras", g.map_or(0, |g| g.cameras.len())),
            ("tread states", g.map_or(0, |g| g.treads.len())),
            ("last fired", g.map_or(0, |g| g.last_fired.len())),
            ("coll sound times", g.map_or(0, |g| g.last_coll_sound.len())),
            ("defs", h.defs.len()),
        ]
    }

    pub fn shutdown(&mut self) {
        if self.booted {
            crate::boot::shutdown(&mut self.host);
        }
        crate::netgame::shutdown(&mut self.host);
        self.host.audio.shutdown_driver();
    }

    /// Canvas, dialogs and engine statistics.
    pub fn report(&self, top: usize) -> String {
        let h = &self.host;
        let mut s = String::new();
        s += &format!("sim time {} ms ({} ticks), pending events {}\n", h.sim_time(), self.ticks, h.sched.pending());
        s += &format!("canvas: {}\n", h.canvas.describe());
        s += &format!("canvas history: {}\n", h.canvas.history.join(" | "));
        s += &format!(
            "cursor: {} ({}), window: \"{}\" {}x{} {}\n",
            if h.canvas.cursor_on { "on" } else { "off" },
            h.canvas.cursor,
            h.video.title,
            h.video.width,
            h.video.height,
            if h.video.fullscreen { "fullscreen" } else { "windowed" }
        );
        let active = h.input.active_maps();
        s += &format!(
            "action maps: {} bound maps, active [GlobalActionMap{}{}]\n",
            h.input.maps.len(),
            if active.is_empty() { "" } else { ", " },
            active.join(", ")
        );
        s += &format!("audio: {}\n", h.audio.status());
        if let Some(g) = &h.game {
            let player = self.control_object().and_then(|o| g.tank_of(o)).and_then(|t| g.world.tank(t));
            let mut ev: Vec<_> = g.event_counts.iter().collect();
            ev.sort();
            s += &format!(
                "simulation: t={} ms, {} tanks, {} projectiles in flight, {} triggers, events {:?}\n",
                g.world.time_ms,
                g.world.tanks.len(),
                g.world.projectiles.iter().filter(|p| p.is_live()).count(),
                g.world.triggers.len(),
                ev
            );
            s += &format!(
                "render: {} moving shapes, {} trigger shapes, {} sprite projectiles, {} smoke emitters\n",
                self.dynamic_shapes().len(),
                self.trigger_shapes().len(),
                self.sprite_projectiles().len(),
                self.smoke_emitters().len()
            );
            if let Some(t) = player {
                let ground = g.world.terrain.as_ref().map(|h| h.height(t.pos.x, t.pos.y)).unwrap_or(0.0);
                s += &format!(
                    "player tank: pos {:.2} {:.2} {:.2} (ground {:.2}), speed {:.2}, damage {:.2}, ammo {}, shots {}, turret {:.2}/{:.2}, corpse {}\n",
                    t.pos.x, t.pos.y, t.pos.z, ground, t.vel.length(), t.damage, t.ammo, t.fire_count, t.yaw_n, t.pitch_n, t.is_corpse
                );
            }
        }
        if let (Some(c), Some(sv)) = (h.net.client, h.net.server) {
            let tanks: Vec<String> = h.game.as_ref().map(|g| g.entities.iter().filter_map(|e| g.tank_of(e.id).and_then(|t| g.world.tank(t)).map(|t| format!("{}({}) at {:.1} {:.1} {:.1}", e.def, e.id.0, t.pos.x, t.pos.y, t.pos.z))).collect()).unwrap_or_default();
            s += &format!("connection: {c} <-> {sv}, control object {:?}, messages {:?}\ntanks: {}\n", self.control_object(), h.net.sent, tanks.join(", "));
        }
        s += &format!("log: {} lines ({} warnings, {} errors)\n", h.log.counts.iter().sum::<usize>(), h.log.counts[1], h.log.counts[2]);
        fn list(m: &std::collections::HashMap<String, usize>) -> Vec<(&String, &usize)> {
            let mut v: Vec<(&String, &usize)> = m.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            v
        }
        let stubs = list(&h.stats.stubs);
        s += &format!("stubbed engine features: {} distinct\n", stubs.len());
        for (name, n) in stubs.iter().take(top) {
            s += &format!("  {n:5}  {name}\n");
        }
        let errs: Vec<&String> = h.log.history.iter().filter(|(l, _)| *l != LogLevel::Info).map(|(_, m)| m).take(top).collect();
        if !errs.is_empty() {
            s += "first warnings/errors:\n";
            for e in errs {
                s += &format!("  {e}\n");
            }
        }
        s
    }
}

/// Poses `inst` by its shape's first sequence at `t` seconds (a thread on
/// sequence 0: nodes, object visibility); returns the IFL times it sets.
fn first_sequence(inst: &mut tt_render::DynamicInstance, t: f32) -> Vec<(String, f32)> {
    use tt_render::pose;
    let Some(shape) = pose::shape(&inst.shape) else { return Vec::new() };
    let Some(name) = shape.sequences.first().map(|q| q.name.clone()) else { return Vec::new() };
    inst.nodes = pose::sequence_pose_of(&shape, &name, t);
    for (o, v) in pose::sequence_visibility_of(&shape, &name, t) {
        if v { inst.shown.push(o) } else { inst.hidden.push(o) }
    }
    pose::sequence_ifl_of(&shape, &name, t)
}

/// What a power-up / pad shape shows at sim time `t` (seconds): node poses
/// of `seq`, object visibility and IFL times of `ambient`, with threads
/// `(sequence, thread time)` laid on top in order, later ones winning (the
/// power-up's `activate` thread, a `playUserThread` one such as the Reload
/// arch's `activate`): the objects, nodes and IFLs each animates follow it.
/// Returns (nodes, hidden, shown, IFL times).
///
/// Drawn only on or off here, an object shows while at least half visible
/// (the `Flash` billboard: 1, 0.73, 0.44, 0.17 over ticks 10..13, with the
/// lamp's brightest IFL frames at ticks 10..11; the arch's `electro`: 0.5
/// on its last lit keyframe).
#[allow(clippy::type_complexity)]
pub fn trigger_pose(path: &std::path::Path, seq: &str, t: f32, threads: &[(&str, f32)]) -> (Vec<(String, glam::Mat4)>, Vec<String>, Vec<String>, Vec<(String, f32)>) {
    use tt_render::pose;
    let mut nodes = pose::sequence_pose(path, seq, t);
    let mut ifl = pose::sequence_ifl(path, "ambient", t);
    let mut vis = pose::sequence_visibility_level(path, "ambient", t);
    let shape = pose::shape(path);
    for &(tseq, tt) in threads {
        // Later entries win (the renderer / the lookups below take the last).
        fn overlay<T>(base: &mut Vec<(String, T)>, top: Vec<(String, T)>) {
            base.retain(|(n, _)| !top.iter().any(|(m, _)| m.eq_ignore_ascii_case(n)));
            base.extend(top);
        }
        let mut top = pose::sequence_pose(path, tseq, tt);
        if let Some((shape, q)) = shape.as_ref().and_then(|s| s.sequences.iter().find(|q| q.name.eq_ignore_ascii_case(tseq)).map(|q| (s, q))) {
            for (name, m) in &mut top {
                let Some(i) = shape.nodes.iter().position(|n| n.name == *name) else { continue };
                let moved = q.rotation_matters.contains(&i) || q.translation_matters.contains(&i);
                let under = nodes.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|x| x.1);
                if let (false, Some(under), Some(default)) = (moved, under, pose::node_default(shape, name)) {
                    *m = under * default.inverse() * *m;
                }
            }
        }
        overlay(&mut nodes, top);
        overlay(&mut ifl, pose::sequence_ifl(path, tseq, tt));
        overlay(&mut vis, pose::sequence_visibility_level(path, tseq, tt));
    }
    let (mut hidden, mut shown) = (Vec::new(), Vec::new());
    for (obj, v) in vis {
        if v >= 0.5 {
            shown.push(obj);
        } else {
            hidden.push(obj);
        }
    }
    (nodes, hidden, shown, ifl)
}

/// Advances the simulation clock by `ms`, running the timed events due on
/// the way in (time, id) order, each at its own time.
pub fn run_events(h: &mut EngineHost, ms: u32) {
    let target = h.sched.now().saturating_add(ms);
    while let Some((id, ev)) = h.sched.pop_due(target) {
        match ev {
            crate::sched::Event::ServerTimer => crate::gameplay::server_host::fire_timer(h, id),
            crate::sched::Event::UpdateLightingProgress => crate::gameplay::client::connection::update_lighting_progress(h),
            crate::sched::Event::SoloEndGame => crate::shell::solo_end_game(h),
        }
    }
    h.sched.set_now(target);
}

/// Runs an input event through `GlobalActionMap` (`global`) and/or the
/// active action maps, most recent first (`active`): the bound actions.
/// Returns true if a binding handled it.
pub fn dispatch_input(h: &mut EngineHost, ev: &InputEvent, global: bool, active: bool) -> bool {
    let maps = h.input.dispatch_maps(global, active);
    let actions = h.input.dispatch(&maps, ev);
    let handled = !actions.is_empty();
    for (a, v) in actions {
        crate::gameplay::client::game::bind_action(h, a, &v);
    }
    handled
}

/// `name`, or if `exists(name)`, the first free name made by counting up the
/// number before the extension (keeping its width: `shot_00001.png` →
/// `shot_00002.png`), or by appending `_2`, `_3`... when there is none.
pub fn free_screenshot_name(name: &str, exists: impl Fn(&str) -> bool) -> String {
    if !exists(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.').filter(|&i| !name[i..].contains('/')) {
        Some(i) => (&name[..i], &name[i..]),
        None => (name, ""),
    };
    let digits = stem.len() - stem.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (base, start, width, sep) = if digits > 0 {
        let n: u64 = stem[stem.len() - digits..].parse().unwrap_or(0);
        (&stem[..stem.len() - digits], n + 1, digits, "")
    } else {
        (stem, 2, 0, "_")
    };
    (start..start + 100_000).map(|n| format!("{base}{sep}{n:0width$}{ext}")).find(|c| !exists(c)).unwrap_or_else(|| name.to_string())
}

/// Whether a projectile is drawn as a moving shape. `.dts` projectiles and
/// ejected brain jars stay drawn until deleted (as before); a sprite
/// projectile with a `display_shape` (the mortar rocket) only until it
/// explodes, like its sprite. This holds on network clients too: their
/// copies get `exploded_at` from the replicated explosion or their own
/// prediction.
pub(crate) fn projectile_mesh_drawn(p: &tt_game::Projectile, brain: bool) -> bool {
    if p.deleted {
        return false;
    }
    if p.data.is_dts() || brain {
        return true;
    }
    !p.data.display_shape.is_empty() && p.exploded_at.is_none()
}

/// Whether a projectile's trail emitter runs: not after it exploded; the
/// particles already emitted fade out on their own.
pub(crate) fn projectile_trails(p: &tt_game::Projectile) -> bool {
    !p.data.emitter.is_empty() && !p.deleted && p.exploded_at.is_none()
}

#[cfg(test)]
mod projectile_render_tests {
    use super::{projectile_mesh_drawn, projectile_trails};
    use std::sync::Arc;
    use tt_game::fields::Fields;
    use tt_game::{Projectile, ProjectileParams, ProjectileId};

    fn proj(pairs: &[(&str, &str)]) -> Projectile {
        let f = Fields::new("P", "ProjectileDef", pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let data = Arc::new(ProjectileParams::from_fields(&f));
        Projectile { id: ProjectileId(1), data, pos: glam::Vec3::ZERO, prev_pos: glam::Vec3::ZERO, vel: glam::Vec3::X, source: None, damage_scale: 1.0, bounces: 0, exploded_at: None, cosmetic_since: None, born_ms: 0, deleted: false }
    }

    #[test]
    fn mortar_rocket_hidden_once_exploded() {
        let mut p = proj(&[("resource", "shell.png"), ("display_shape", "mortarrocket.dts"), ("emitter", "RocketEmitter")]);
        assert!(projectile_mesh_drawn(&p, false) && projectile_trails(&p));
        p.exploded_at = Some(100);
        assert!(!projectile_mesh_drawn(&p, false), "the rocket mesh must vanish in the blast");
        assert!(!projectile_trails(&p), "no new smoke after the blast");
    }

    #[test]
    fn other_projectiles_keep_their_mesh_rules() {
        // A plain sprite projectile is never a mesh.
        assert!(!projectile_mesh_drawn(&proj(&[("resource", "shell.png")]), false));
        // `.dts` projectiles (volcano rocks) and brain jars: until deleted.
        let mut rock = proj(&[("resource", "rock.dts")]);
        rock.exploded_at = Some(5);
        assert!(projectile_mesh_drawn(&rock, false));
        rock.deleted = true;
        assert!(!projectile_mesh_drawn(&rock, false));
        assert!(projectile_mesh_drawn(&proj(&[("resource", "shell.png")]), true));
    }
}

#[cfg(test)]
mod screenshot_tests {
    use super::free_screenshot_name;

    #[test]
    fn screenshot_names_never_overwrite() {
        let taken = ["screenshot_00001.png", "screenshot_00002.png", "shot.png"];
        let exists = |p: &str| taken.contains(&p);
        assert_eq!(free_screenshot_name("screenshot_00003.png", exists), "screenshot_00003.png");
        assert_eq!(free_screenshot_name("screenshot_00001.png", exists), "screenshot_00003.png");
        assert_eq!(free_screenshot_name("shot.png", exists), "shot_2.png");
        assert_eq!(free_screenshot_name("movie0099.png", |p| p == "movie0099.png"), "movie0100.png");
    }
}

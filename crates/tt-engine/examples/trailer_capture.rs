//! Offscreen video capture of scripted game scenes (the trailer's footage,
//! `tools/trailer/`). Each scene is a game started like the menu's Custom
//! game, driven by a script (bots, the player's inputs or a bot brain in
//! the player's tank, power-ups given, cameras), rendered offscreen at a
//! fixed frame rate (the sim and the effects advance exactly 1/fps per
//! frame, or slower in slow-motion stretches) and piped to ffmpeg.
//!
//! ```text
//! cargo run --release -p tt-engine --example trailer_capture -- \
//!     --out DIR [--size 1920x1080] [--fps 60] [--crf 12] [--list] SCENE...
//! ```
//! Writes `DIR/SCENE.mp4` (H.264, near-lossless; the edit re-encodes) and
//! `DIR/SCENE.txt` (frame numbers of the scene's marks, e.g. when the
//! volcano erupts or a shot lands, for the edit). Never opens a window and
//! plays no sound. `--list` prints the scenes. Some scenes need a temporary
//! data directory with a mod (`mod_*`): they make it themselves.

use glam::{Mat3, Vec3};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tt_engine::{Config, Engine};
use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, Team};
use tt_render::Camera;

/// Every scene: name, what it shows.
const SCENES: &[(&str, &str)] = &[
    ("c_green", "Amygdala Abyss: a fight on open ground, an orbit round its middle"),
    ("c_green_low", "Amygdala Abyss: the fight from a low arc"),
    ("c_blue", "Thalamus Pass at night: a fight, a slow arc"),
    ("c_red", "Cerebral Caldera: a fight with the volcano behind, a slow arc"),
    ("c_red_track", "Cerebral Caldera: following the player's tank, high and far back"),
    ("c_orig_blue", "A classic map (TT2_3): a fight, an orbit"),
    ("c_orig_green", "A classic map (TT1_2): a fight, a slow arc"),
    ("fly_volcano", "Cerebral Caldera: a push towards the volcano as it erupts"),
    ("pup_speedy", "Speedy shots at three target tanks (classic map TT1_2, day)"),
    ("pup_bounce", "Bounce shots (classic map TT2_3, night)"),
    ("pup_area", "Area blasts lighting the ground, side view (classic map TT2_3, night)"),
    ("pup_mortar", "Mortar at night: the launch, the shell coming in, the blast in slow motion"),
    ("pup_scatter", "Scatter: cone reticle and the ring of pellets (classic map TT3_1)"),
    ("pup_heal", "A damaged tank drives into a Heal pick-up (Amygdala Abyss)"),
    ("pup_reload", "Driving through a Reload arch (classic map TT3_1)"),
    ("aim_classic", "Classic aim: the mouse swings the turret, the camera stays behind the hull (TT3_1)"),
    ("aim_camera", "Camera aim: the view turns, the turret follows the screen centre (TT3_1)"),
    ("hud", "The game view and HUD in a team game on TT1_2 (player list with K/D/A, radar), full frame"),
    ("gfx_a_modern", "Graphics comparison A (classic map TT1_2, day, close orbit), Modern look"),
    ("gfx_a_classic", "Graphics comparison A, Classic look"),
    ("gfx_b_modern", "Graphics comparison B (classic map TT2_3, night, Area blasts), Modern look"),
    ("gfx_b_classic", "Graphics comparison B, Classic look"),
    ("fly_tt1_5", "Fly-in over Amygdala Abyss down to a fight"),
    ("fly_tt2_5", "Fly-in over Thalamus Pass down to a fight"),
    ("fly_tt3_5", "Cerebral Caldera: in towards the volcano along the clear west-south-west lane"),
    ("mods_menu", "Options -> Mods in the menu with a texture mod installed"),
    ("mod_off", "Tanks close up on TT1_2, stock textures"),
    ("mod_on", "The same close-up with the texture mod enabled"),
];

struct Args {
    out: PathBuf,
    size: (u32, u32),
    fps: u32,
    crf: u32,
    scenes: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { out: PathBuf::from("out/trailer/work/clips"), size: (1920, 1080), fps: 60, crf: 12, scenes: Vec::new() };
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        match s.as_str() {
            "--out" => a.out = it.next().ok_or("--out needs a directory")?.into(),
            "--fps" => a.fps = it.next().and_then(|v| v.parse().ok()).ok_or("--fps needs a number")?,
            "--crf" => a.crf = it.next().and_then(|v| v.parse().ok()).ok_or("--crf needs a number")?,
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--list" => {
                for (n, d) in SCENES {
                    println!("{n:14} {d}");
                }
                std::process::exit(0);
            }
            s if !s.starts_with('-') => a.scenes.push(s.to_string()),
            s => return Err(format!("unknown option {s}")),
        }
    }
    if a.scenes.is_empty() {
        return Err("no scene (--list shows them)".into());
    }
    Ok(a)
}

fn main() {
    let a = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("trailer_capture: {e}");
            std::process::exit(2);
        }
    };
    std::fs::create_dir_all(&a.out).unwrap();
    for name in a.scenes.clone() {
        if !SCENES.iter().any(|(n, _)| *n == name) {
            eprintln!("unknown scene {name}");
            std::process::exit(2);
        }
        let out = a.out.clone();
        let (size, fps, crf) = (a.size, a.fps, a.crf);
        // The engine's script interpreter recurses deeply.
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || {
                let t = std::time::Instant::now();
                let n = run_scene(&name, &out, size, fps, crf);
                eprintln!("{name}: {n} frames in {:.0} s", t.elapsed().as_secs_f64());
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

// ------------------------------------------------------------------ capture

/// The engine, its offscreen frontend and the encoder of one scene.
struct Cap {
    e: Engine,
    off: tt_engine::gfx::Offscreen,
    fe: tt_engine::frontend::Frontend,
    data: PathBuf,
    fps: u32,
    /// Sim speed (1 = real time; 0.3 = slow motion).
    rate: f32,
    clock_ms: f64,
    enc: Option<Child>,
    /// Frames written so far.
    frames: u32,
    marks: Vec<String>,
    /// Smoothed camera (position, look-at point).
    cam: Option<(Vec3, Vec3)>,
    /// A bot brain keeps driving the player's tank.
    autopilot: bool,
    /// Prefs of the scene, applied again once the player profile exists
    /// (a new profile starts from the defaults).
    want: Vec<(String, String)>,
    /// The player's weapon draws no reticle (shots from outside cameras).
    no_reticle: bool,
}

const ALL_MODERN: &[(&str, &str)] = &[
    ("graphics.show_fps", "0"),
    ("graphics.glass_jar", "1"),
    ("graphics.hdr", "1"),
    ("graphics.bloom", "0.7"),
    ("graphics.normal_maps", "1"),
    ("graphics.ssao", "1"),
    ("graphics.dynamic_lights", "1"),
    ("graphics.shadows", "2"),
    ("graphics.hud_scale", "0.75"),
];

const ALL_CLASSIC: &[(&str, &str)] = &[
    ("graphics.show_fps", "0"),
    ("graphics.glass_jar", "0"),
    ("graphics.hdr", "0"),
    ("graphics.normal_maps", "0"),
    ("graphics.ssao", "0"),
    ("graphics.dynamic_lights", "0"),
    ("graphics.shadows", "0"),
];

impl Cap {
    fn new(name: &str, out: &Path, size: (u32, u32), fps: u32, crf: u32, mods: Option<&dyn Fn(&Path)>) -> Cap {
        let root = tt_assets::layout::default_dir();
        let data = std::env::temp_dir().join(format!("tt-trailer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        if let Some(m) = mods {
            m(&data);
        }
        let cfg = Config { assets: root.clone(), data: data.clone(), args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
        let mut e = Engine::new(&cfg).unwrap();
        e.host.virtual_clock_ms = Some(0.0);
        assert!(e.boot(), "boot failed");
        let off = tt_engine::gfx::Offscreen::new(size).unwrap();
        let mut fe = tt_engine::frontend::Frontend::new(&off.device, &off.queue, off.format, root, false);
        fe.hide_hud = true;
        let file = out.join(format!("{name}.mp4"));
        let enc = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{}x{}", size.0, size.1), "-r", &fps.to_string(), "-i", "-"])
            .args(["-c:v", "libx264", "-preset", "fast", "-crf", &crf.to_string(), "-pix_fmt", "yuv420p"])
            .arg(&file)
            .stdin(Stdio::piped())
            .spawn()
            .expect("ffmpeg");
        let mut c = Cap { e, off, fe, data, fps, rate: 1.0, clock_ms: 0.0, enc: Some(enc), frames: 0, marks: Vec::new(), cam: None, autopilot: false, want: Vec::new(), no_reticle: false };
        c.prefs(ALL_MODERN);
        // `TRAILER_PREFS=name=value,...`: extra prefs (A/B checks).
        if let Ok(v) = std::env::var("TRAILER_PREFS") {
            let kv: Vec<(String, String)> = v.split(',').filter_map(|p| p.split_once('=')).map(|(a, b)| (a.trim().to_string(), b.trim().to_string())).collect();
            let refs: Vec<(&str, &str)> = kv.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            c.prefs(&refs);
        }
        c
    }

    fn prefs(&mut self, p: &[(&str, &str)]) {
        for (k, v) in p {
            self.want.retain(|(n, _)| n != k);
            self.want.push((k.to_string(), v.to_string()));
        }
        self.apply_prefs();
    }

    fn apply_prefs(&mut self) {
        for (k, v) in self.want.clone() {
            self.e.host.shell.prefs.set(&k, &v);
        }
        self.fe.reload_prefs();
    }

    /// The player profile ("Player"), with the scene's prefs.
    fn player(&mut self) {
        {
            let Engine { host, menu, .. } = &mut self.e;
            let m = menu.as_mut().unwrap();
            use tt_menu::MenuBackend;
            tt_engine::menu::Backend::new(host, &mut m.data).create_player("Player").unwrap();
        }
        self.apply_prefs();
    }

    fn dt(&self) -> f32 {
        1000.0 / self.fps as f32
    }

    /// One frame: the sim and effects advance by `rate`/fps, the camera
    /// `hook` runs on the new poses, the frame is drawn and, with
    /// `record`, encoded.
    fn frame(&mut self, record: bool, hook: &mut dyn FnMut(&mut Cap)) {
        let dt = self.dt() * self.rate;
        self.clock_ms += dt as f64;
        self.e.host.virtual_clock_ms = Some(self.clock_ms);
        self.fe.step(&mut self.e, &self.off.device, &self.off.queue, dt);
        self.keep_alive();
        hook(self);
        self.fe.time_override = Some((self.clock_ms / 1000.0) as f32);
        self.fe.wall_override = Some(self.clock_ms / 1000.0);
        self.fe.draw(&mut self.e, &self.off.device, &self.off.queue, &self.off.view, self.off.size, dt);
        if std::env::var_os("TRAILER_DEBUG").is_some() && self.e.ticks.is_multiple_of(15) {
            let pose = self.me().and_then(|id| self.pose(id));
            let g = self.e.host.game.as_ref();
            let n = g.map_or(0, |g| g.world.projectiles.len());
            let cur = self.me().and_then(|id| g?.world.tank(id)?.current.as_ref().map(|p| p.name.clone()));
            eprintln!("extras {:?} scene {}", self.e.render_settings().extras, self.fe.scene.is_some());
            eprintln!("t{} pose {pose:?} projectiles {n} current {cur:?} trig {:?} cam {:?}", self.e.ticks, self.e.host.client.moves.triggers[0], self.cam);
        }
        if record {
            let px = tt_engine::gfx::read_rgba(&self.off.device, &self.off.queue, &self.off.texture, self.off.size, self.off.format);
            self.enc.as_mut().unwrap().stdin.as_mut().unwrap().write_all(&px).unwrap();
            self.frames += 1;
        } else {
            self.off.wait();
        }
    }

    /// `n` recorded frames.
    fn rec(&mut self, n: u32, mut hook: impl FnMut(&mut Cap, u32)) {
        for i in 0..n {
            self.frame(true, &mut |c| hook(c, i));
        }
    }

    /// `ms` of drawn but not recorded frames (effects warm up).
    fn warm(&mut self, ms: u32, mut hook: impl FnMut(&mut Cap)) {
        for _ in 0..(ms as f32 / self.dt()) as u32 {
            self.frame(false, &mut hook);
        }
    }

    /// `ms` of sim only (no drawing; the explosions are dropped).
    fn skip(&mut self, ms: u32) {
        for _ in 0..(ms as f32 / self.dt()) as u32 {
            let dt = self.dt();
            self.clock_ms += dt as f64;
            self.e.host.virtual_clock_ms = Some(self.clock_ms);
            self.fe.step(&mut self.e, &self.off.device, &self.off.queue, dt);
            self.keep_alive();
            self.e.take_explosions();
        }
    }

    fn mark(&mut self, what: &str) {
        let s = format!("{} {what}", self.frames);
        eprintln!("mark: {s}");
        self.marks.push(s);
    }

    fn finish(mut self, out: &Path, name: &str) -> u32 {
        let mut enc = self.enc.take().unwrap();
        drop(enc.stdin.take());
        enc.wait().unwrap();
        std::fs::write(out.join(format!("{name}.txt")), self.marks.join("\n") + "\n").unwrap();
        let _ = std::fs::remove_dir_all(&self.data);
        self.frames
    }

    // -------------------------------------------------------------- game

    /// A free-for-all (mode 0) or team game on `mission` with `bots` bots;
    /// returns once the player's tank has landed.
    fn start(&mut self, mission: &str, bots: usize, mode: u32, skill: BotSkill) {
        self.player();
        const NAMES: [&str; 9] = ["Ajax", "Brick", "Cortex", "Dent", "Echo", "Fuse", "Grit", "Hex", "Ion"];
        let team = |i: usize| if mode == 1 || mode == 3 { if i.is_multiple_of(2) { Team::Blue } else { Team::Green } } else { Team::Auto };
        let bots = (0..bots).map(|i| BotSetup { name: NAMES[i % NAMES.len()].into(), skill: if skill == BotSkill::Gold { [BotSkill::Silver, BotSkill::Gold, BotSkill::Bronze][i % 3] } else { skill }, tank: i % 3, team: team(i) }).collect();
        let g = CustomGame { mission: tt_assets::layout::mission(mission), mode, bots, player_tank: 1, player_team: if mode == 1 || mode == 3 { Team::Green } else { Team::Auto }, minutes: 20, replace_bots: true };
        {
            let Engine { host, menu, .. } = &mut self.e;
            menu.as_mut().unwrap().apply(MenuAction::StartCustom(g), host);
        }
        for _ in 0..4000 {
            self.skip(32);
            if self.me().is_some() {
                break;
            }
        }
        assert!(self.me().is_some(), "the player did not spawn");
        self.skip(2500);
    }

    /// Eruptions of the volcano so far.
    fn erupted(&self) -> usize {
        self.e.host.game.as_ref().map_or(0, |g| g.event_counts.get("VolcanoErupted").copied().unwrap_or(0))
    }

    /// Runs the sim (no drawing) until engine tick `t`.
    fn skip_to(&mut self, t: u64) {
        while self.e.ticks < t {
            self.skip(32);
        }
    }

    /// The player's tank.
    fn me(&self) -> Option<tt_game::TankId> {
        let o = self.e.control_object()?;
        let g = self.e.host.game.as_ref()?;
        let id = g.tank_of(o)?;
        g.world.tank(id).filter(|t| !t.deleted && !t.is_corpse)?;
        Some(id)
    }

    /// Respawns the player (SPACE, like a player) and keeps the autopilot on.
    fn keep_alive(&mut self) {
        if self.e.host.game.is_none() {
            return;
        }
        if self.no_reticle {
            if let Some(t) = self.tank_mut() {
                if let Some(p) = t.current.as_ref().filter(|p| !p.reticle.is_empty()) {
                    let mut d = (**p).clone();
                    d.reticle.clear();
                    t.current = Some(std::sync::Arc::new(d));
                }
            }
        }
        match self.me() {
            None => {
                if self.e.ticks.is_multiple_of(20) {
                    self.e.key("space", true, None);
                    self.e.key("space", false, None);
                }
            }
            Some(id) if self.autopilot => {
                let g = self.e.host.game.as_mut().unwrap();
                if g.world.tank(id).is_some_and(|t| t.ai.is_none()) {
                    if let Some(d) = g.world.tanks.iter().find_map(|t| t.ai.as_ref().map(|a| a.data.clone())) {
                        g.world.set_ai(id, Some(d));
                    }
                }
            }
            _ => {}
        }
    }

    /// The drawn pose of a tank (position, forward).
    fn pose(&self, id: tt_game::TankId) -> Option<(Vec3, Vec3)> {
        let g = self.e.host.game.as_ref()?;
        let t = g.world.tank(id)?;
        let (p, r) = g.render_pose(t, self.e.render_alpha);
        Some((p, r.y_axis))
    }

    fn ground(&self, x: f32, y: f32) -> f32 {
        self.e.host.game.as_ref().and_then(|g| g.world.terrain.as_ref()).map_or(0.0, |t| t.height(x, y))
    }

    /// Puts the player's tank at `p` (on the ground) facing `yaw` (radians,
    /// 0 = north, towards +X positive), still.
    fn place(&mut self, p: Vec3, yaw: f32) {
        let id = self.me().unwrap();
        let g = self.e.host.game.as_mut().unwrap();
        let o = g.entity_of_tank(id).unwrap();
        place_tank(g, o, id, p, yaw);
    }

    /// Bots stop thinking (targets that sit still) and stand at `spots`.
    fn dummies(&mut self, spots: &[(Vec3, f32)]) {
        let me = self.me();
        let g = self.e.host.game.as_mut().unwrap();
        let ids: Vec<tt_game::TankId> = g.world.tanks.iter().filter(|t| Some(t.id) != me && !t.deleted && !t.is_corpse).map(|t| t.id).collect();
        for (i, id) in ids.into_iter().enumerate() {
            g.world.set_ai(id, None);
            if let Some(t) = g.world.tank_mut(id) {
                t.input = Default::default();
            }
            let (p, yaw) = spots.get(i).copied().unwrap_or((Vec3::new(400.0 + i as f32 * 20.0, 400.0, 0.0), 0.0));
            if let Some(o) = g.entity_of_tank(id) {
                place_tank(g, o, id, p, yaw);
            }
        }
    }

    /// `p` on the ground.
    fn on_ground(&self, x: f32, y: f32) -> Vec3 {
        Vec3::new(x, y, self.ground(x, y))
    }

    fn give(&mut self, projectile: &str) {
        let id = self.me().unwrap();
        let g = self.e.host.game.as_mut().unwrap();
        let p = g.projectile(projectile).unwrap_or_else(|| panic!("no projectile {projectile}"));
        g.world.set_projectile(id, p);
    }

    fn tank_mut(&mut self) -> Option<&mut tt_game::Tank> {
        let id = self.me()?;
        self.e.host.game.as_mut()?.world.tank_mut(id)
    }

    /// Presses (or releases) the fire trigger.
    fn fire(&mut self, down: bool) {
        let held = self.e.host.client.moves.triggers[0].float() as i64 % 2 == 1;
        if held != down {
            self.e.host.client.moves.trigger(0);
        }
    }

    fn key(&mut self, k: &str, down: bool) {
        self.e.key(k, down, None);
    }

    fn live(&self, name: &str) -> Vec<Vec3> {
        self.e.host.game.as_ref().map_or(Vec::new(), |g| g.world.projectiles.iter().filter(|p| p.is_live() && p.data.name == name).map(|p| p.pos).collect())
    }

    /// The power-ups of def `db` (`Heal`, `Reload`): position and
    /// heading of each.
    fn triggers_of(&self, db: &str) -> Vec<(Vec3, f32)> {
        let g = self.e.host.game.as_ref().unwrap();
        g.world
            .triggers
            .iter()
            .filter(|t| matches!(&t.kind, tt_game::TriggerKind::PowerUp(d) if d.name.eq_ignore_ascii_case(db)))
            .map(|t| {
                let y = t.transform.matrix3.y_axis;
                (Vec3::from(t.transform.translation), y.x.atan2(y.y))
            })
            .collect()
    }

    /// Height spread of the ground along `len` metres before `p` coming
    /// from heading `yaw` (how flat an approach is).
    fn approach_spread(&self, p: Vec3, yaw: f32, len: f32) -> f32 {
        let (f, _) = axes(yaw);
        let hs: Vec<f32> = (0..=(len as i32 / 2)).map(|k| {
            let q = p - f * (k as f32 * 2.0);
            self.ground(q.x, q.y)
        }).collect();
        hs.iter().cloned().fold(f32::MIN, f32::max) - hs.iter().cloned().fold(f32::MAX, f32::min)
    }

    // ------------------------------------------------------------ cameras

    /// Sets the camera to look from `pos` at `at` (smoothed by `k` per
    /// frame at 60 fps; 1 = no smoothing), kept above the ground.
    fn look(&mut self, pos: Vec3, at: Vec3, k: f32) {
        let k = 1.0 - (1.0 - k).powf(60.0 / self.fps as f32);
        // The wanted position clears the ground and the sight line first,
        // so the smoothing also smooths any lift.
        let pos = Vec3::new(pos.x, pos.y, pos.z.max(self.ground(pos.x, pos.y) + 0.7));
        let pos = self.clear_sight(pos, at);
        let (p, a) = match self.cam {
            Some((p0, a0)) => (p0.lerp(pos, k), a0.lerp(at, k)),
            None => (pos, at),
        };
        let p = Vec3::new(p.x, p.y, p.z.max(self.ground(p.x, p.y) + 0.7));
        self.cam = Some((p, a));
        self.fe.camera_override = Some(look(p, a));
    }

    /// `pos` raised until the terrain no longer hides `at` (sight line
    /// 1.2 m over the ground all the way).
    fn clear_sight(&self, mut pos: Vec3, at: Vec3) -> Vec3 {
        for _ in 0..40 {
            let blocked = (1..12).any(|k| {
                let q = pos.lerp(at, k as f32 / 12.0);
                q.z < self.ground(q.x, q.y) + 1.2
            });
            if !blocked {
                break;
            }
            pos.z += 0.5;
        }
        pos
    }

    /// Behind `id` at `dist`, `height` above it, looking `ahead` in front.
    fn chase(&mut self, id: Option<tt_game::TankId>, dist: f32, height: f32, ahead: f32, k: f32) {
        let Some((p, f)) = id.and_then(|id| self.pose(id)) else { return };
        let f = Vec3::new(f.x, f.y, 0.0).normalize_or(Vec3::Y);
        self.look(p - f * dist + Vec3::Z * height, p + f * ahead + Vec3::Z * 1.0, k);
    }

    /// Centre of the live tanks within `r` of `c` (`c` if none).
    fn fight_centre(&self, c: Vec3, r: f32) -> Vec3 {
        let Some(g) = self.e.host.game.as_ref() else { return c };
        let ps: Vec<Vec3> = g.world.tanks.iter().filter(|t| !t.deleted && !t.is_corpse).map(|t| g.render_pose(t, self.e.render_alpha).0).filter(|p| (*p - c).truncate().length() < r).collect();
        if ps.is_empty() {
            c
        } else {
            ps.iter().copied().sum::<Vec3>() / ps.len() as f32
        }
    }

    /// Puts every tank (the player's too) on a ring of radius `r` around
    /// `c`, facing the centre.
    fn ring(&mut self, c: Vec3, r: f32) {
        let ids: Vec<tt_game::TankId> = self.e.host.game.as_ref().unwrap().world.tanks.iter().filter(|t| !t.deleted && !t.is_corpse).map(|t| t.id).collect();
        let n = ids.len().max(1) as f32;
        for (i, id) in ids.into_iter().enumerate() {
            let a = i as f32 / n * std::f32::consts::TAU;
            let q = c + Vec3::new(a.sin(), a.cos(), 0.0) * r;
            let q = Vec3::new(q.x, q.y, self.ground(q.x, q.y));
            let g = self.e.host.game.as_mut().unwrap();
            if let Some(o) = g.entity_of_tank(id) {
                place_tank(g, o, id, q, a + std::f32::consts::PI);
            }
        }
    }

    fn orbit(&mut self, center: Vec3, radius: f32, height: f32, angle: f32, k: f32) {
        let pos = center + Vec3::new(angle.sin() * radius, -angle.cos() * radius, height);
        self.look(pos, center + Vec3::Z * 1.0, k);
    }
}

fn place_tank(g: &mut tt_engine::game::GameSim, o: tt_gameplay::EntityId, id: tt_game::TankId, p: Vec3, yaw: f32) {
    // setTransform's angle turns +Y (north) towards +X; the hull's
    // rotation is about +Z by -yaw.
    let xf = format!("{} {} {} 0 0 1 {}", p.x, p.y, p.z + 0.5, yaw);
    g.set_transform(o, &xf);
    if let Some(t) = g.world.tank_mut(id) {
        t.vel = Vec3::ZERO;
        t.yaw_n = 0.0;
        t.pitch_n = 0.0;
        t.rot = Mat3::from_rotation_z(-yaw);
    }
}

/// A camera at `pos` looking at `at`.
fn look(pos: Vec3, at: Vec3) -> Camera {
    let d = (at - pos).normalize_or(Vec3::Y);
    Camera { pos, yaw: d.x.atan2(d.y), pitch: d.z.clamp(-1.0, 1.0).asin() }
}

/// Smooth 0..1 ramp.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ------------------------------------------------------------------ scenes

/// Unit vectors of a heading (radians, 0 = north): forward, right.
fn axes(yaw: f32) -> (Vec3, Vec3) {
    let f = Vec3::new(yaw.sin(), yaw.cos(), 0.0);
    (f, Vec3::new(f.y, -f.x, 0.0))
}

/// Volcano eruptions on Cerebral Caldera in a free-for-all with `bots`
/// bots and the player on autopilot: the engine tick of the first one
/// after `after` ticks (a dry run without drawing; the game replays the
/// same way, so a take can start just before it).
fn eruption_tick(size: (u32, u32), fps: u32, bots: usize, after: u64) -> u64 {
    let tmp = std::env::temp_dir().join(format!("tt-trailer-probe-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut c = Cap::new("probe", &tmp, size, fps, 30, None);
    c.start("TT3_5", bots, 0, BotSkill::Gold);
    c.autopilot = true;
    let n = c.erupted();
    let mut tick = None;
    for _ in 0..(120 * fps) {
        c.skip(32);
        if c.erupted() > n && c.e.ticks >= after {
            tick = Some(c.e.ticks);
            break;
        }
    }
    drop(c.enc.take().map(|mut e| e.kill()));
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&c.data);
    tick.expect("no eruption in 2 minutes")
}

/// How a staged fight is filmed (angles: 0 = the camera south of the
/// centre, increasing anticlockwise seen from above; radians).
#[derive(Clone, Copy)]
enum FightCam {
    /// Round the middle of the fight at `r` metres, `h` up, `speed` rad/s.
    Orbit { r: f32, h: f32, a0: f32, speed: f32 },
    /// A slow arc from `a0` to `a1` over the take.
    Arc { r: f32, h: f32, a0: f32, a1: f32 },
    /// Behind the player's tank, high and far back.
    Track,
}

/// A fight staged on open ground: the player's tank (a bot brain) and
/// `bots` bots on a ring of 14 m round `centre`, facing each other, so it
/// starts at once; filmed by `cam` for `secs` seconds.
fn fight(c: &mut Cap, mission: &str, centre: (f32, f32), bots: usize, cam: FightCam, secs: f32) {
    let fps = c.fps;
    c.start(mission, bots, 0, BotSkill::Gold);
    c.autopilot = true;
    c.no_reticle = true;
    let centre = c.on_ground(centre.0, centre.1);
    c.ring(centre, 14.0);
    c.skip(1200);
    let n = (secs * fps as f32) as u32;
    let mut i = 0u32;
    let mut shot = move |c: &mut Cap| {
        let t = i as f32 / n as f32;
        i += 1;
        let m = c.fight_centre(centre, 40.0);
        let m = Vec3::new(m.x, m.y, c.ground(m.x, m.y));
        match cam {
            FightCam::Orbit { r, h, a0, speed } => c.orbit(m, r, h, a0 + speed * i as f32 / fps as f32, 0.04),
            FightCam::Arc { r, h, a0, a1 } => c.orbit(m, r, h, a0 + (a1 - a0) * ease(t), 0.04),
            FightCam::Track => {
                let me = c.me();
                c.chase(me, 13.0, 6.5, 14.0, 0.05);
            }
        }
    };
    c.warm(500, &mut shot);
    c.rec(n, |c, _| shot(c));
}

fn run_scene(name: &str, out: &Path, size: (u32, u32), fps: u32, crf: u32) -> u32 {
    use std::f32::consts::{FRAC_PI_2, PI};
    let secs = |s: f32| (s * fps as f32) as u32;
    let mods_dir = |data: &Path| make_mod(data, name == "mod_on" || name == "mods_menu");
    let with_mod = name.starts_with("mod");
    let mut c = Cap::new(name, out, size, fps, crf, if with_mod { Some(&mods_dir) } else { None });
    match name {
        // Combat across the worlds.
        "c_green" => fight(&mut c, "TT1_5", (-92.0, -108.0), 6, FightCam::Orbit { r: 27.0, h: 11.0, a0: -0.6, speed: 0.13 }, 9.0),
        "c_green_low" => fight(&mut c, "TT1_5", (-92.0, -108.0), 6, FightCam::Arc { r: 17.0, h: 4.0, a0: 2.2, a1: 2.9 }, 9.0),
        "c_blue" => fight(&mut c, "TT2_5", (-140.0, -26.0), 6, FightCam::Arc { r: 23.0, h: 8.0, a0: 2.7, a1: 3.4 }, 9.0),
        // The camera west of the fight: the volcano behind it.
        "c_red" => fight(&mut c, "TT3_5", (-138.0, -10.0), 6, FightCam::Arc { r: 22.0, h: 4.5, a0: -FRAC_PI_2 - 0.35, a1: -FRAC_PI_2 + 0.25 }, 9.0),
        "c_red_track" => fight(&mut c, "TT3_5", (-138.0, -10.0), 6, FightCam::Track, 9.0),
        "c_orig_blue" => fight(&mut c, "TT2_3", (-75.0, 85.0), 6, FightCam::Orbit { r: 21.0, h: 8.0, a0: 0.8, speed: 0.12 }, 9.0),
        "c_orig_green" => fight(&mut c, "TT1_2", (88.0, -5.0), 6, FightCam::Arc { r: 24.0, h: 9.0, a0: PI + 0.4, a1: PI - 0.3 }, 9.0),
        "fly_volcano" => {
            // Cerebral Caldera: a push towards the volcano, which erupts
            // 1.5 s into the take (a dry run finds when).
            drop(c.enc.take().map(|mut e| e.kill()));
            drop(c);
            let at = eruption_tick(size, fps, 6, 0);
            eprintln!("eruption at tick {at}");
            c = Cap::new(name, out, size, fps, crf, None);
            c.start("TT3_5", 6, 0, BotSkill::Gold);
            c.autopilot = true;
            c.no_reticle = true;
            c.skip_to(at - (1500 / tt_engine::TICK_MS) as u64);
            let n = c.erupted();
            c.fe.fov_override = Some(68.0);
            let mut seen = false;
            c.rec(secs(7.0), |c, i| {
                if !seen && c.erupted() > n {
                    seen = true;
                    c.mark("erupt");
                }
                volcano_cam(c, i as f32 / (7.0 * 60.0));
            });
        }
        n if n.starts_with("pup_") => powerup_scene(&mut c, n, fps),
        "aim_classic" | "aim_camera" => aim_scene(&mut c, name == "aim_camera", fps),
        "hud" => {
            // The game's own view and HUD in a team game, a fight round
            // the player's tank (radar dots, kills in the list).
            c.prefs(&[("graphics.hud_scale", "1.0")]);
            c.start("TT1_2", 7, 1, BotSkill::Gold);
            c.autopilot = true;
            c.fe.hide_hud = false;
            c.skip(3000);
            for _ in 0..3 {
                let centre = c.on_ground(88.0, -5.0);
                c.ring(centre, 16.0);
                c.skip(8000);
            }
            let centre = c.on_ground(88.0, -5.0);
            c.ring(centre, 16.0);
            c.skip(800);
            c.warm(600, |_| {});
            c.rec(secs(12.0), |_, _| {});
        }
        "gfx_a_modern" | "gfx_a_classic" | "gfx_b_modern" | "gfx_b_classic" => {
            if name.ends_with("classic") {
                c.prefs(ALL_CLASSIC);
            }
            gfx_scene(&mut c, name.starts_with("gfx_a"), fps);
        }
        "fly_tt3_5" => {
            // Cerebral Caldera: in towards the volcano along the clear lane
            // from the west-south-west, tanks fighting below.
            c.start("TT3_5", 6, 0, BotSkill::Gold);
            c.autopilot = true;
            c.no_reticle = true;
            c.skip(3000);
            c.fe.fov_override = Some(68.0);
            c.warm(300, |c| volcano_lane(c, 0.0, 255.0));
            c.rec(secs(7.0), |c, i| volcano_lane(c, i as f32 / (7.0 * 60.0), 255.0));
        }
        "fly_tt1_5" | "fly_tt2_5" => {
            // A fly-in from high over the map down to a fight.
            let m = &name[4..].to_uppercase();
            c.start(m, 6, 0, BotSkill::Gold);
            c.autopilot = true;
            c.no_reticle = true;
            let (centre, from) = match m.as_str() {
                "TT1_5" => ((-92.0, -108.0), Vec3::new(-10.0, -230.0, 38.0)),
                "TT2_5" => ((-140.0, -26.0), Vec3::new(-250.0, -60.0, 30.0)),
                _ => ((-138.0, -10.0), Vec3::new(-60.0, -95.0, 30.0)),
            };
            let centre = c.on_ground(centre.0, centre.1);
            c.ring(centre, 14.0);
            c.skip(1200);
            let cam = move |c: &mut Cap, t: f32| {
                let s = ease(t);
                let d = centre + (from - centre).normalize() * 30.0;
                let p = Vec3::new(from.x, from.y, 0.0).lerp(Vec3::new(d.x, d.y, 0.0), s);
                let z = c.ground(p.x, p.y).max(centre.z) + from.z + (10.0 - from.z) * s;
                let m = c.fight_centre(centre, 40.0);
                let at = Vec3::new(m.x, m.y, centre.z + 1.0).lerp(Vec3::new(m.x, m.y, centre.z + 1.0) + (centre - Vec3::new(from.x, from.y, centre.z)).normalize() * 40.0, 1.0 - s);
                c.look(Vec3::new(p.x, p.y, z), at, 0.15);
            };
            c.fe.fov_override = Some(72.0);
            c.warm(300, |c| cam(c, 0.0));
            c.rec(secs(7.0), |c, i| cam(c, i as f32 / (7.0 * 60.0)));
        }
        "mods_menu" => {
            c.player();
            {
                let m = c.e.menu.as_mut().unwrap();
                m.state.reset(tt_menu::Screen::MainMenu);
                m.state.open(tt_menu::Screen::Settings);
                m.state.set_settings_tab(tt_menu::SettingsTab::Mods);
            }
            c.warm(500, |_| {});
            c.rec(secs(5.0), |_, _| {});
        }
        "mod_off" | "mod_on" => {
            c.start("TT1_2", 3, 0, BotSkill::Gold);
            c.no_reticle = true;
            let h = 3.0 * std::f32::consts::FRAC_PI_4;
            let (f, r) = axes(h);
            let p = c.on_ground(78.0, 6.0);
            c.place(p, h);
            let spot = |c: &Cap, df: f32, dr: f32| {
                let q = p + f * df + r * dr;
                c.on_ground(q.x, q.y)
            };
            let spots = vec![(spot(&c, 6.0, -5.0), 2.2), (spot(&c, 7.0, 5.0), 0.9), (spot(&c, 13.0, 0.0), 1.6)];
            c.dummies(&spots);
            c.skip(3000);
            let at = spot(&c, 6.0, 0.0) + Vec3::Z;
            c.warm(300, |c| c.orbit(at, 12.0, 3.2, 0.6, 1.0));
            c.rec(secs(5.0), |c, i| c.orbit(at, 12.0, 3.2, 0.6 + 0.25 * i as f32 / 300.0, 1.0));
        }
        _ => unreachable!(),
    }
    c.finish(out, name)
}

/// The push towards the volcano along the lane at 167° (no crystals in
/// the way): the cone low in the frame, the sky above it for the rocks.
fn volcano_cam(c: &mut Cap, t: f32) {
    volcano_lane(c, t, 167.0)
}

/// [`volcano_cam`] along the clear lane at `deg` degrees.
fn volcano_lane(c: &mut Cap, t: f32, deg: f32) {
    let s = ease(t);
    let (sa, ca) = deg.to_radians().sin_cos();
    let d = 100.0 - 42.0 * s;
    let p = Vec3::new(sa * d, ca * d, 0.0);
    let z = c.ground(p.x, p.y).max(100.0) + 12.0 + 4.0 * s;
    c.look(Vec3::new(p.x, p.y, z), Vec3::new(0.0, 0.0, 127.0 - 3.0 * s), 1.0);
}

fn gfx_scene(c: &mut Cap, a: bool, fps: u32) {
    let secs = |s: f32| (s * fps as f32) as u32;
    c.prefs(&[("input.aim_mode", "0")]);
    let q = std::f32::consts::FRAC_PI_4;
    let (m, start, yaw) = if a { ("TT1_2", (78.0, 6.0), 3.0 * q) } else { ("TT2_3", (-86.0, 98.0), 3.0 * q) };
    c.start(m, 2, 0, BotSkill::Gold);
    c.no_reticle = true;
    let (f, r) = axes(yaw);
    let me = c.on_ground(start.0, start.1);
    c.place(me, yaw);
    let at = |c: &Cap, df: f32, dr: f32| {
        let q = me + f * df + r * dr;
        c.on_ground(q.x, q.y)
    };
    let spots = if a { vec![(at(c, 5.0, -5.0), yaw + 0.9), (at(c, 18.0, 3.0), yaw - 2.4)] } else { vec![(at(c, 20.0, -3.0), yaw + 2.6), (at(c, 23.0, 3.5), yaw - 2.4)] };
    c.dummies(&spots);
    c.skip(2500);
    c.give("SplashProjectile");
    let cam = move |c: &mut Cap, t: f32| {
        let s = ease(t);
        if a {
            // Round the two near tanks at 8 m, low.
            let m = me + f * 2.5 + r * -2.5;
            let ang = -0.4 + 1.1 * s;
            let d = (-f * ang.cos() + r * ang.sin()) * 8.5;
            c.look(m + d + Vec3::Z * 2.6, m + f * 3.0 + Vec3::Z * 0.8, 1.0);
        } else {
            let p = me - f * 13.0 + r * (-7.0 + 6.0 * s) + Vec3::Z * (6.0 - 1.0 * s);
            c.look(p, me + f * 15.0 + Vec3::Z * 1.0, 1.0);
        }
    };
    c.warm(500, |c| cam(c, 0.0));
    c.rec(secs(7.0), |c, i| {
        let t = i as f32 / fps as f32;
        c.no_reticle = true;
        c.fire((1.5..1.6).contains(&t) || (4.0..4.1).contains(&t));
        cam(c, t / 7.0);
    });
}

/// The Mortar at night (Thalamus Pass): a probe shot finds where the
/// shell lands and the target tanks wait there; the take starts behind the
/// tank at the launch, then a camera near the landing zone follows the
/// shell in and the blast plays in slow motion.
fn mortar_scene(c: &mut Cap, fps: u32) {
    let secs = |s: f32| (s * fps as f32) as u32;
    c.prefs(&[("input.aim_mode", "0")]);
    c.start("TT2_5", 3, 0, BotSkill::Gold);
    c.no_reticle = false;
    let yaw = std::f32::consts::FRAC_PI_2;
    let (f, r) = axes(yaw);
    let me = c.on_ground(-162.0, -26.0);
    c.place(me, yaw);
    c.dummies(&[]);
    c.skip(2500);
    c.give("MortarProjectile");
    c.skip(2000);
    // The probe shot.
    c.fire(true);
    c.skip(200);
    c.fire(false);
    let mut last = None;
    for _ in 0..300 {
        c.skip(32);
        match c.live("MortarProjectile").first() {
            Some(&q) => last = Some(q),
            None if last.is_some() => break,
            None => {}
        }
    }
    let land = last.expect("the mortar did not fire");
    let land = c.on_ground(land.x, land.y);
    eprintln!("mortar lands at {land}");
    let spot = |c: &Cap, df: f32, dr: f32| {
        let q = land + f * df + r * dr;
        c.on_ground(q.x, q.y)
    };
    let spots = vec![(spot(c, -2.0, -4.5), yaw + 2.6), (spot(c, 1.5, 4.0), yaw - 2.4), (spot(c, 5.0, -0.5), yaw + 3.0)];
    c.dummies(&spots);
    c.give("MortarProjectile");
    c.skip(3000);
    let behind = me - f * 10.0 + r * 3.0 + Vec3::Z * 4.5;
    let zone = land - f * 15.0 + r * 12.0 + Vec3::Z * 5.0;
    c.warm(400, |c| c.look(behind, me + f * 30.0 + Vec3::Z * 8.0, 1.0));
    c.fire(true);
    let mut i = 0;
    let mut shell_seen = false;
    let mut landed_at: Option<u32> = None;
    c.rec(secs(7.5), |c, _| {
        i += 1;
        if i == 4 {
            c.fire(false);
        }
        let shell = c.live("MortarProjectile").first().copied();
        shell_seen |= shell.is_some();
        // Fired: the reticle (arc dots, landing ring) goes.
        c.no_reticle = shell_seen;
        if i < secs(1.0) {
            // The launch, from behind the tank; the camera tilts up after the shell.
            let at = shell.unwrap_or(me + f * 30.0 + Vec3::Z * 8.0);
            c.look(behind, at, if i < 3 { 1.0 } else { 0.08 });
            return;
        }
        if i == secs(1.0) {
            c.cam = None;
        }
        match (shell, landed_at) {
            (Some(q), None) => c.look(zone, q.lerp(land + Vec3::Z * 2.0, 0.35), 0.12),
            (None, None) if shell_seen => {
                landed_at = Some(i);
                c.mark("blast");
                c.look(zone, land + Vec3::Z * 2.5, 0.06);
            }
            _ => c.look(zone, land + Vec3::Z * 2.5, 0.06),
        }
        // Slow motion for 1.6 s of screen time after the blast, then back.
        if let Some(l) = landed_at {
            let k = (i - l) as f32 / fps as f32;
            c.rate = if k < 1.6 { 0.3 } else { (0.3 + (k - 1.6) * 1.2).min(1.0) };
        }
    });
    c.rate = 1.0;
}

/// Heal and Reload at the map's own pick-up and arch, driven into along
/// the flattest approach.
fn powerup_scene(c: &mut Cap, name: &str, fps: u32) {
    let secs = |s: f32| (s * fps as f32) as u32;
    use std::f32::consts::FRAC_PI_4 as Q;
    if name == "pup_mortar" {
        return mortar_scene(c, fps);
    }
    // (map, bots, tank position, heading).
    let (mission, bots, start, yaw) = match name {
        "pup_speedy" => ("TT1_2", 3, (86.0, -6.0), 7.0 * Q),
        "pup_bounce" => ("TT2_3", 3, (-14.0, -86.0), 2.0 * Q),
        "pup_area" => ("TT2_3", 3, (-86.0, 98.0), 3.0 * Q),
        "pup_scatter" => ("TT3_1", 3, (-142.0, 96.0), 0.0),
        "pup_heal" => ("TT1_5", 0, (0.0, 0.0), 0.0),
        _ => ("TT3_1", 0, (0.0, 0.0), 0.0),
    };
    // Classic aim: the keys turn the hull.
    c.prefs(&[("input.aim_mode", "0")]);
    c.start(mission, bots, 0, BotSkill::Gold);
    match name {
        "pup_speedy" | "pup_bounce" | "pup_area" | "pup_scatter" => {
            let (f, r) = axes(yaw);
            let me = c.on_ground(start.0, start.1);
            c.place(me, yaw);
            let at = |c: &Cap, df: f32, dr: f32| {
                let q = me + f * df + r * dr;
                c.on_ground(q.x, q.y)
            };
            let spots = match name {
                "pup_scatter" => vec![(at(c, 14.0, -2.5), yaw + 2.6), (at(c, 16.0, 3.0), yaw - 2.4), (at(c, 20.0, 0.0), yaw + 3.0)],
                "pup_area" => vec![(at(c, 20.0, -3.0), yaw + 2.6), (at(c, 22.0, 3.0), yaw - 2.4), (at(c, 26.0, 0.0), yaw + 3.0)],
                _ => vec![(at(c, 17.0, -3.5), yaw + 2.6), (at(c, 19.0, 3.5), yaw - 2.4), (at(c, 24.0, 0.0), yaw + 3.0)],
            };
            c.dummies(&spots);
            let proj = match name {
                "pup_speedy" => "SpeedyProjectile",
                "pup_bounce" => "BounceProjectile",
                "pup_area" => "SplashProjectile",
                _ => "ScatterProjectile",
            };
            c.skip(2500);
            c.give(proj);
            // Raised three-quarter view from behind; the side view for the
            // Area blast (its light on the ground).
            let (pos, look_at) = match name {
                "pup_area" => (me + f * 8.0 + r * 20.0 + Vec3::Z * 6.5, me + f * 15.0 + Vec3::Z * 1.0),
                "pup_scatter" => (me - f * 8.0 + r * 4.0 + Vec3::Z * 5.5, me + f * 11.0 + Vec3::Z * 0.5),
                "pup_bounce" => (me - f * 6.0 + r * 5.0 + Vec3::Z * 3.5, me + f * 11.0 + Vec3::Z * 1.0),
                _ => (me - f * 8.0 + r * 4.5 + Vec3::Z * 4.5, me + f * 12.0 + Vec3::Z * 1.0),
            };
            let cam = move |c: &mut Cap, t: f32| {
                // A slow push in.
                let p = pos.lerp(look_at, 0.12 * ease(t / 6.0));
                c.look(p, look_at, 1.0);
            };
            c.warm(800, |c| cam(c, 0.0));
            c.rec(secs(6.0), |c, i| {
                let t = i as f32 / fps as f32;
                let on = match name {
                    "pup_area" => (0.4..0.5).contains(&t) || (2.6..2.7).contains(&t),
                    "pup_scatter" => (0.6..0.7).contains(&t) || (2.4..2.5).contains(&t) || (4.2..4.3).contains(&t),
                    _ => (0.3..4.8).contains(&t),
                };
                c.fire(on);
                if name == "pup_bounce" {
                    if let Some(t) = c.tank_mut() {
                        t.pitch_n = 0.5;
                    }
                }
                if name == "pup_speedy" || name == "pup_bounce" {
                    // Swing the hull over the targets.
                    c.key("a", (1.2..1.45).contains(&t));
                    c.key("d", (2.4..2.9).contains(&t));
                    c.key("a", (3.6..3.85).contains(&t));
                }
                cam(c, t);
            });
            c.fire(false);
        }
        "pup_heal" | "pup_reload" => {
            let heal = name == "pup_heal";
            // The map's pick-up (or arch) with the flattest approach; an
            // arch is driven through along its axis.
            let mut best: Option<(f32, Vec3, f32)> = None;
            for (p, h) in c.triggers_of(if heal { "Heal" } else { "Reload" }) {
                let yaws: Vec<f32> = if heal { (0..8).map(|k| k as f32 * Q).collect() } else { vec![h, h + std::f32::consts::PI] };
                for y in yaws {
                    let sp = c.approach_spread(p, y, 22.0);
                    if best.is_none_or(|b| sp < b.0) {
                        best = Some((sp, p, y));
                    }
                }
            }
            let (sp, a, yaw) = best.expect("no such pick-up on the map");
            eprintln!("{name}: at {a}, heading {:.0}°, spread {sp:.2}", yaw.to_degrees());
            let (f, r) = axes(yaw);
            let s0 = a - f * if heal { 16.0 } else { 20.0 };
            let me = c.on_ground(s0.x, s0.y);
            c.place(me, yaw);
            c.skip(2500);
            if heal {
                if let Some(t) = c.tank_mut() {
                    t.damage = t.data.damage_amount * 0.85;
                }
            } else {
                c.give("SpeedyProjectile");
                if let Some(t) = c.tank_mut() {
                    t.ammo = 1;
                }
            }
            // The pick-up is there (switched on) when the tank arrives.
            for t in c.e.host.game.as_mut().unwrap().world.triggers.iter_mut() {
                if (Vec3::from(t.transform.translation) - a).length() < 0.5 {
                    t.on = true;
                    t.off_until = 0;
                }
            }
            c.skip(1500);
            let ground = c.ground(a.x, a.y);
            let a = Vec3::new(a.x, a.y, ground);
            let cam = a + r * 11.0 - f * 6.0 + Vec3::Z * 4.0;
            let cam_at = a - f * 4.0 + Vec3::Z * 1.5;
            c.warm(500, |c| c.look(cam, cam_at, 1.0));
            c.rec(secs(5.0), |c, i| {
                let t = i as f32 / fps as f32;
                c.key("w", (0.2..if heal { 2.3 } else { 3.4 }).contains(&t));
                c.look(cam, cam_at, 1.0);
            });
            c.key("w", false);
        }
        _ => unreachable!(),
    }
}

/// Classic vs camera aim on Amygdala Abyss: the same mouse sweep across
/// three target tanks. With classic aim the mouse swings the turret and
/// the camera stays behind the hull; with camera aim the view turns and
/// the turret follows the screen centre.
fn aim_scene(c: &mut Cap, camera_aim: bool, fps: u32) {
    let secs = |s: f32| (s * fps as f32) as u32;
    c.prefs(&[("input.aim_mode", if camera_aim { "1" } else { "0" })]);
    c.start("TT3_1", 3, 0, BotSkill::Gold);
    let yaw = 0.0;
    let (f, r) = axes(yaw);
    let me = c.on_ground(-142.0, 92.0);
    c.place(me, yaw);
    let at = |c: &Cap, df: f32, dr: f32| {
        let q = me + f * df + r * dr;
        c.on_ground(q.x, q.y)
    };
    let _ = at;
    c.fe.hide_hud = false;
    // The targets ahead of where the view looks (with camera aim the view
    // keeps its own heading).
    c.skip(1500);
    c.warm(300, |_| {});
    let cy = c.fe.camera.yaw;
    let (f, r) = axes(cy);
    let at = |c: &Cap, df: f32, dr: f32| {
        let q = me + f * df + r * dr;
        c.on_ground(q.x, q.y)
    };
    let spots = vec![(at(c, 20.0, -8.0), cy + 2.6), (at(c, 23.0, 0.0), cy + 3.0), (at(c, 20.0, 8.0), cy - 2.4)];
    c.dummies(&spots);
    c.skip(1500);
    let rate: f32 = std::env::var("TRAILER_AIM_RATE").ok().and_then(|v| v.parse().ok()).unwrap_or(2.5);
    let lift: f32 = std::env::var("TRAILER_AIM_LIFT").ok().and_then(|v| v.parse().ok()).unwrap_or(-40.0);
    if camera_aim {
        // Look up a little: the targets in the middle of the view.
        let ev = tt_engine::input::InputEvent::Axis { device: tt_engine::input::Device::Mouse, axis: "yaxis".into(), delta: lift };
        c.e.input(&ev);
    }
    c.warm(800, |_| {});
    c.rec(secs(7.0), |c, i| {
        let t = i as f32 / fps as f32;
        // Left, fire, right across, fire, back to the middle, fire.
        let d = if t < 0.7 {
            -rate
        } else if (1.3..2.7).contains(&t) {
            rate
        } else if (3.3..4.0).contains(&t) {
            -rate
        } else {
            0.0
        };
        if d != 0.0 {
            let ev = tt_engine::input::InputEvent::Axis { device: tt_engine::input::Device::Mouse, axis: "xaxis".into(), delta: d };
            c.e.input(&ev);
        }
        c.fire((0.9..1.0).contains(&t) || (2.9..3.0).contains(&t) || (4.3..4.4).contains(&t));
    });
}

/// A cosmetic texture mod in `data/mods/`: the three tank textures
/// recoloured (a desert camo); `enabled` enables it in the settings.
fn make_mod(data: &Path, enabled: bool) {
    let root = tt_assets::layout::default_dir();
    let dir = data.join("mods").join("desert-camo");
    std::fs::create_dir_all(dir.join("models/tanks")).unwrap();
    std::fs::write(dir.join("mod.ron"), "(\n  name: \"Desert Camo\",\n  version: \"1.0\",\n  author: \"OpenTT\",\n  description: \"Sand-coloured camouflage for every tank.\",\n)\n").unwrap();
    for entry in std::fs::read_dir(root.join("models/tanks")).unwrap().flatten() {
        let n = entry.file_name().to_string_lossy().to_string();
        let lower = n.to_ascii_lowercase();
        if !(lower.starts_with("tank") && lower.ends_with(".png") && !lower.contains("normal")) {
            continue;
        }
        let Ok(img) = image::open(entry.path()) else { continue };
        let mut img = img.to_rgba8();
        for p in img.pixels_mut() {
            let [r, g, b, a] = p.0;
            let l = (0.3 * r as f32 + 0.59 * g as f32 + 0.11 * b as f32) / 255.0;
            // Sand to rust by brightness.
            let (sr, sg, sb) = (0.93, 0.78, 0.48);
            let (dr, dg, db) = (0.55, 0.25, 0.12);
            let m = |s: f32, d: f32| ((d + (s - d) * l) * 255.0 * (0.6 + 0.6 * l)).clamp(0.0, 255.0) as u8;
            p.0 = [m(sr, dr), m(sg, dg), m(sb, db), a];
        }
        img.save(dir.join("models/tanks").join(&n)).unwrap();
    }
    let mut settings = tt_shell::Settings::default();
    if enabled {
        settings.mods.enabled = Some(vec!["desert-camo".into()]);
    }
    std::fs::write(data.join("settings.ron"), settings.to_ron()).unwrap();
}

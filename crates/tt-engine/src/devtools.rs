//! The executable's developer paths (cargo feature `dev`, on by default):
//! scripted inputs (`--keys`, `--cmd`), `--headless`, `--screenshot` and
//! `--bench`.

use std::path::{Path, PathBuf};
use tt_engine::allocstats::allocs;
use tt_engine::{app, gfx, mission, Engine};

/// The developer options of the command line.
#[derive(Default)]
pub struct DevArgs {
    pub headless: bool,
    pub ticks: Option<u64>,
    pub screenshot: Option<PathBuf>,
    pub frames: u64,
    pub size: Option<(u32, u32)>,
    /// (tick, text) scripted key presses.
    pub keys: Vec<(u64, String)>,
    /// (tick, command) console commands.
    pub cmds: Vec<(u64, String)>,
    /// `--bench SECONDS`.
    pub bench: Option<f32>,
    /// `--frame-stats`: frame-time breakdown, spikes and summaries on stderr.
    pub frame_stats: bool,
}

impl DevArgs {
    /// Parses one developer option (`arg`, its value from `it`); false if
    /// `arg` is none.
    pub fn parse(&mut self, arg: &str, it: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        let num = |v: Option<String>, what: &str| v.and_then(|s| s.parse::<u64>().ok()).ok_or(format!("{what} needs a number"));
        match arg {
            "--headless" => self.headless = true,
            "--ticks" => self.ticks = Some(num(it.next(), "--ticks")?),
            "--screenshot" => self.screenshot = Some(it.next().ok_or("--screenshot needs a path")?.into()),
            "--frames" => self.frames = num(it.next(), "--frames")?,
            "--keys" => {
                let v = it.next().ok_or("--keys needs TICK:TEXT")?;
                let (t, text) = v.split_once(':').ok_or("--keys needs TICK:TEXT")?;
                self.keys.push((t.parse().map_err(|_| "--keys: bad tick")?, text.to_string()));
            }
            "--frame-stats" => self.frame_stats = true,
            // The menu's Quick Play on a mission at tick 1 (benchmarks of a real game).
            "--play" => self.cmds.push((1, format!("{PLAY}{}", it.next().ok_or("--play needs a mission (tt3_5)")?))),
            "--bench" => self.bench = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--bench needs seconds")?),
            "--cmd" => {
                let v = it.next().ok_or("--cmd needs TICK:COMMAND")?;
                let (t, src) = v.split_once(':').ok_or("--cmd needs TICK:COMMAND")?;
                self.cmds.push((t.parse().map_err(|_| "--cmd: bad tick")?, src.to_string()));
            }
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                self.size = Some((w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// A run without a window or sound.
    pub fn offline(&self) -> bool {
        self.headless || self.screenshot.is_some() || self.bench.is_some()
    }
}


/// The `--cmd` prefix of a `--play MISSION` (not a console command).
const PLAY: &str = "\u{1}play ";

/// One sim tick plus any scripted keys due (`{name}` = a named key, e.g. `{return}`).
fn tick(engine: &mut Engine, keys: &[(u64, String)], cmds: &[(u64, String)]) {
    apply_inputs(engine, keys, cmds);
    engine.tick();
}

thread_local! {
    /// Steady scripted mouse motion (`{mouse:AXIS,RATE}`), counts per second for x and y.
    static MOUSE_RATE: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((0.0, 0.0)) };
}

/// Scripted inputs of the current tick (`engine.ticks`).
fn apply_inputs(engine: &mut Engine, keys: &[(u64, String)], cmds: &[(u64, String)]) {
    let now = engine.ticks;
    for (_, src) in cmds.iter().filter(|(t, _)| *t == now) {
        if let Some(m) = src.strip_prefix(PLAY) {
            use tt_engine::gameplay::golden::drive;
            let game = match m.split_once(':') {
                // MISSION:BOTS: a free-for-all custom game with that many bots.
                Some((m, n)) => {
                    const TANKS: [&str; 6] = ["GoldHeavyTank", "SilverMediumTank", "BronzeLightTank", "GoldLightTank", "SilverMediumTank", "BronzeHeavyTank"];
                    let names: Vec<String> = (0..n.parse::<usize>().unwrap_or(6)).map(|i| format!("Bot{i} [Bot]")).collect();
                    let bots: Vec<(&str, &str, &str)> = names.iter().enumerate().map(|(i, n)| (TANKS[i % TANKS.len()], n.as_str(), "")).collect();
                    drive::custom_game(m, false, false, &bots, 10.0, 0)
                }
                None => drive::quick_play(m),
            };
            drive::apply(engine, &game);
            continue;
        }
        for l in tt_engine::console::execute(engine, src) {
            println!("{l}");
        }
    }
    for (_, text) in keys.iter().filter(|(t, _)| *t == now) {
        let mut rest = text.as_str();
        while let Some(c) = rest.chars().next() {
            let (key, typed, len) = match rest.strip_prefix('{').and_then(|r| r.split_once('}')) {
                Some((name, _)) => (name.to_ascii_lowercase(), None, name.len() + 2),
                None => (if c == ' ' { "space".into() } else { c.to_ascii_lowercase().to_string() }, Some(c.to_string()), c.len_utf8()),
            };
            let parts: Vec<i32> = key.split_once(':').map(|(_, p)| p.split(',').map(|v| v.trim().parse().unwrap_or(0)).collect()).unwrap_or_default();
            if key.starts_with("wheel:") && parts.len() == 3 {
                engine.mouse_wheel(parts[2] as f32);
            } else if let Some((axis, d)) = key.strip_prefix("axis:").and_then(|p| p.split_once(',')) {
                let ev = tt_engine::input::InputEvent::Axis { device: tt_engine::input::Device::Mouse, axis: format!("{axis}axis"), delta: d.trim().parse().unwrap_or(0.0) };
                engine.input(&ev);
            } else if let Some((axis, r)) = key.strip_prefix("mouse:").and_then(|p| p.split_once(',')) {
                let r: f32 = r.trim().parse().unwrap_or(0.0);
                MOUSE_RATE.with(|m| m.set(if axis == "y" { (m.get().0, r) } else { (r, m.get().1) }));
            } else if let Some(k) = key.strip_prefix("down:") {
                engine.key(k, true, None);
            } else if let Some(k) = key.strip_prefix("up:") {
                engine.key(k, false, None);
            } else if key.starts_with("click:") {
                engine.mouse_button(0, true);
                engine.mouse_button(0, false);
            } else {
                engine.key(&key, true, typed.as_deref());
                engine.key(&key, false, None);
            }
            rest = &rest[len..];
        }
    }
}

/// Scripted inputs due in ticks `[from, to)`.
fn apply_due(engine: &mut Engine, keys: &[(u64, String)], cmds: &[(u64, String)], from: u64, to: u64) {
    for now in from..to {
        let k: Vec<(u64, String)> = keys.iter().filter(|(t, _)| *t == now).cloned().collect();
        let e: Vec<(u64, String)> = cmds.iter().filter(|(t, _)| *t == now).cloned().collect();
        if k.is_empty() && e.is_empty() {
            continue;
        }
        let saved = engine.ticks;
        engine.ticks = now;
        // `tick` applies the inputs of `engine.ticks` and then ticks once;
        // here only the inputs are wanted.
        apply_inputs(engine, &k, &e);
        engine.ticks = saved;
    }
}

/// Runs the offline developer modes (`--bench`, `--screenshot`,
/// `--headless`); None if none was asked for.
pub fn run_offline(engine: &mut Engine, a: &DevArgs, assets: &Path, mission: Option<&Path>) -> Option<Result<(), String>> {
    if let Some(seconds) = a.bench {
        return Some(bench(engine, a, assets, mission.is_some(), seconds));
    }
    if let Some(out) = &a.screenshot {
        return Some(screenshot(engine, a, assets, mission.is_some(), out));
    }
    if a.headless {
        headless(engine, a, assets);
        return Some(Ok(()));
    }
    None
}

/// `--bench`: offscreen frames at 60 Hz through the same path as the window
/// (sim ticks, scene builds, 3D + effects + GUI), timing every stage.
fn bench(engine: &mut Engine, a: &DevArgs, assets: &Path, dev_mission: bool, seconds: f32) -> Result<(), String> {
    let canvas = (engine.host.video.width, engine.host.video.height);
    let off = gfx::Offscreen::new(a.size.unwrap_or((canvas.0 * 2, canvas.1 * 2)))?;
    let mut fe = tt_engine::frontend::Frontend::new(&off.device, &off.queue, off.format, assets.to_path_buf(), dev_mission);
    if engine.host.mission_load.is_some() {
        engine.host.scene_dirty = true;
    }
    let dt = 1000.0 / 60.0;
    let frames = (seconds * 60.0) as u32;
    // Frames over 16.7 ms and over 33.3 ms.
    let mut slow = [0usize; 2];
    let wall = std::time::Instant::now();
    let mut window: Vec<f64> = Vec::new();
    let mut next_input = 0;
    println!("{:>6} {:>8} {:>8} {:>8}  counters", "sim s", "avg ms", "p95 ms", "max ms");
    for f in 0..frames {
        // Every 60 s: frame times of the last minute and everything that
        // could accumulate.
        if f > 0 && f % 3600 == 0 {
            let mut w = window.clone();
            w.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let fx = fe.fx.as_ref().map_or(0, |x| x.effects.particle_count());
            let c: Vec<String> = engine.counters().iter().map(|(k, v)| format!("{k}={v}")).collect();
            println!(
                "{:>6} {:>8.2} {:>8.2} {:>8.2}  particles={fx} dynamic={} sprites={} {}",
                f / 60,
                w.iter().sum::<f64>() / w.len().max(1) as f64,
                w[(w.len() as f64 * 0.95) as usize],
                w[w.len() - 1],
                engine.dynamic_shapes().len(),
                engine.sprite_projectiles().len(),
                c.join(" ")
            );
            window.clear();
        }
        let t = std::time::Instant::now();
        // Inputs due in the ticks this frame will run, each applied once.
        let due = engine.ticks + ((fe.acc_ms + dt) / tt_engine::TICK_MS as f32) as u64;
        apply_due(engine, &a.keys, &a.cmds, next_input.max(engine.ticks), due);
        next_input = next_input.max(due);
        let ts = std::time::Instant::now();
        let a0 = allocs();
        fe.step(engine, &off.device, &off.queue, dt);
        fe.perf.add("step", ts.elapsed().as_secs_f64() * 1000.0);
        let a1 = allocs();
        fe.draw(engine, &off.device, &off.queue, &off.view, off.size, dt);
        let a2 = allocs();
        // Allocation counts per frame (in the "ms" columns), after the load.
        if f >= 120 {
            fe.perf.add("allocs: step", (a1.0 - a0.0) as f64);
            fe.perf.add("allocs: draw", (a2.0 - a1.0) as f64);
            fe.perf.add("alloc KB: step", (a1.1 - a0.1) as f64 / 1024.0);
            fe.perf.add("alloc KB: draw", (a2.1 - a1.1) as f64 / 1024.0);
        }
        let tw = std::time::Instant::now();
        off.wait();
        fe.perf.add("gpu wait", tw.elapsed().as_secs_f64() * 1000.0);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        fe.perf.add("frame", ms);
        slow[0] += usize::from(ms > 16.7);
        slow[1] += usize::from(ms > 33.3);
        window.push(ms);
    }
    let total = wall.elapsed().as_secs_f64();
    println!("==== bench: {} frames at {}x{} on {}, {:.1} s wall ({:.0} fps) ====", frames, off.size.0, off.size.1, off.adapter, total, frames as f64 / total);
    println!("frames > 16.7 ms: {}, > 33 ms: {}", slow[0], slow[1]);
    println!("{}", fe.perf.report());
    // Memory at the end of the run (Linux).
    if let Ok(st) = std::fs::read_to_string("/proc/self/status") {
        let mem: Vec<&str> = st.lines().filter(|l| l.starts_with("VmRSS") || l.starts_with("VmHWM")).collect();
        println!("memory: {}", mem.join(", ").split_whitespace().collect::<Vec<_>>().join(" "));
    }
    println!("canvas: {}", engine.host.canvas.describe());
    Ok(())
}

/// `--screenshot OUT.png`: renders offscreen after `--frames` ticks.
fn screenshot(engine: &mut Engine, a: &DevArgs, assets: &Path, dev_mission: bool, out: &Path) -> Result<(), String> {
    // Explosions of the last 300 ms show in the picture.
    let mut recent = Vec::new();
    for i in 0..a.frames {
        tick(engine, &a.keys, &a.cmds);
        let ex = engine.take_explosions();
        if i + 10 >= a.frames {
            recent.extend(ex);
        }
    }
    let scene = match mission::build_scene(&engine.host, assets, !dev_mission) {
        Some(Ok(s)) => Some(s),
        Some(Err(e)) => return Err(format!("scene: {e}")),
        None => None,
    };
    let t = engine.host.sim_time() as f32 / 1000.0;
    if dev_mission {
        // Developer view: the 3D scene only, like tt-viewer.
        gfx::screenshot(out, a.size.unwrap_or((1280, 720)), scene.as_ref(), None, t)?;
    } else {
        // The canvas at the game's resolution, drawn exactly like a window frame.
        let canvas = (engine.host.video.width, engine.host.video.height);
        let off = gfx::Offscreen::new(a.size.unwrap_or(canvas))?;
        let mut fe = tt_engine::frontend::Frontend::new(&off.device, &off.queue, off.format, assets.to_path_buf(), false);
        engine.host.scene_dirty = true;
        fe.time_override = Some(t);
        fe.step(engine, &off.device, &off.queue, 0.0);
        let again = recent.clone();
        if let Some(g) = engine.host.game.as_mut() {
            g.explosions = recent;
        }
        fe.draw(engine, &off.device, &off.queue, &off.view, off.size, 300.0);
        off.save(out)?;
        // Developer aid for A/B comparisons: `TT_SCREENSHOT_ALT=commands`
        // (console commands separated by `|`, e.g. `set
        // graphics.hdr 0`) draws the same moment again after
        // them, into `OUT.alt.png`.
        if let Ok(cmds) = std::env::var("TT_SCREENSHOT_ALT") {
            let now = engine.ticks;
            let cmds: Vec<(u64, String)> = cmds.split('|').map(|c| (now, c.trim().to_string())).collect();
            apply_inputs(engine, &[], &cmds);
            if let Some(g) = engine.host.game.as_mut() {
                g.explosions = again;
            }
            fe.reload_prefs();
            fe.dynlights = Default::default();
            if let Some(sc) = fe.scene.as_ref() {
                fe.dynlights.set_scene(sc, &fe.root);
            }
            // Settings that need a new renderer apply in `step`.
            fe.step(engine, &off.device, &off.queue, 0.0);
            fe.draw(engine, &off.device, &off.queue, &off.view, off.size, 300.0);
            off.save(&out.with_extension("alt.png"))?;
        }
    }
    println!("{}", engine.report(25));
    println!("saved {}{}", out.display(), if scene.is_none() { " (no mission loaded: empty canvas)" } else { "" });
    Ok(())
}

/// `--headless`: `--ticks` sim ticks (default 60 s), then the report.
fn headless(engine: &mut Engine, a: &DevArgs, assets: &Path) {
    for _ in 0..a.ticks.unwrap_or(60_000 / tt_engine::TICK_MS as u64) {
        tick(engine, &a.keys, &a.cmds);
        if engine.host.quit_requested {
            break;
        }
    }
    if let Some(r) = mission::build_scene(&engine.host, assets, true) {
        match r {
            Ok(s) => println!("mission scene: {} ({} terrain vertices, {} sky faces)", s.name, s.terrain_vertices.len(), s.sky_faces.len()),
            Err(e) => println!("mission scene: error {e}"),
        }
    }
    println!("==== engine report ====\n{}", engine.report(40));
    let before = engine.host.canvas.history.len();
    engine.shutdown();
    println!("onExit: canvas ops {:?}; prefs written to {}", &engine.host.canvas.history[before..], engine.host.fs.data_dir().display());
}

/// The window run's developer hooks: frame statistics, and scripted inputs
/// at their ticks (an unattended run: the pointer is never grabbed).
pub fn setup_window(app: &mut app::App, a: &DevArgs) {
    app.stats = tt_engine::framestats::FrameStats::new(a.frame_stats);
    if a.keys.is_empty() && a.cmds.is_empty() {
        return;
    }
    app.no_grab = true;
    let (keys, cmds) = (a.keys.clone(), a.cmds.clone());
    let mut next = 0;
    app.before_step = Some(Box::new(move |engine: &mut Engine, acc_ms: f32, dt_ms: f32| {
        // Inputs of the ticks this frame will run, each applied once.
        let due = engine.ticks + ((acc_ms + dt_ms) / tt_engine::TICK_MS as f32) as u64;
        apply_due(engine, &keys, &cmds, next.max(engine.ticks), due);
        next = next.max(due);
        let (rx, ry) = MOUSE_RATE.with(|m| m.get());
        for (axis, r) in [("xaxis", rx), ("yaxis", ry)] {
            if r != 0.0 {
                engine.input(&tt_engine::input::InputEvent::Axis { device: tt_engine::input::Device::Mouse, axis: axis.into(), delta: r * dt_ms / 1000.0 });
            }
        }
    }));
}

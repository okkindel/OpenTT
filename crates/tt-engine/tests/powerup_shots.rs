//! Offscreen screenshots of the Mortar and Scatter shot power-ups on the Power-up Test map (their reticles, a scatter
//! volley, the mortar's arc and blast, the two pick-ups' lamps) for visual review. Not a check: run on demand with
//! `TT_PUP_SHOTS=out/powerups-new cargo test --release -p tt-engine --test powerup_shots -- --ignored --test-threads=1`
//! (files `<name>.png` in that directory; `TT_PUP_PREFIX=v2_` prefixes every name, `TT_PUP_LAMPS=0` skips the pick-up
//! lamp frames, `TT_PUP_ONLY=scatter` takes only the scatter shot's frames, `TT_PUP_ONLY=scatter_reticle` only its
//! reticle (one frame), `TT_PUP_ONLY=rocket` only the mortar rocket in flight, `TT_PUP_ONLY=mortar_reticle` only the
//! mortar's reticle (one frame); `TT_PUP_MODS=carpet-bombing` enables those mods (comma-separated ids) in the throwaway
//! user data directory first).

use glam::Vec3;
use std::path::{Path, PathBuf};
use tt_engine::{Config, Engine};
use tt_menu::{CustomGame, MenuAction, MenuBackend, Team};

mod common;
use common::{assets, look, player};

const MISSION: &str = "missions/poweruptest.ron";
const SIZE: (u32, u32) = (1280, 720);

fn big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap();
}

/// The game drawn every tick into an offscreen target (effects run).
struct View {
    off: tt_engine::gfx::Offscreen,
    fe: tt_engine::frontend::Frontend,
    dir: PathBuf,
}

impl View {
    fn tick(&mut self, e: &mut Engine) {
        let t = e.host.virtual_clock_ms.unwrap_or(0.0) + tt_engine::TICK_MS as f64;
        e.host.virtual_clock_ms = Some(t);
        let ms = tt_engine::TICK_MS as f32;
        self.fe.step(e, &self.off.device, &self.off.queue, ms);
        self.fe.draw(e, &self.off.device, &self.off.queue, &self.off.view, self.off.size, ms);
    }

    fn ticks(&mut self, e: &mut Engine, ms: u32) {
        for _ in 0..ms / tt_engine::TICK_MS {
            self.tick(e);
        }
    }

    fn shot(&mut self, name: &str) {
        let prefix = std::env::var("TT_PUP_PREFIX").unwrap_or_default();
        let path = self.dir.join(format!("{prefix}{name}.png"));
        self.off.save(&path).unwrap();
        eprintln!("saved {}", path.display());
    }
}

fn start(root: &Path, dir: PathBuf) -> (Engine, View) {
    let base = std::env::temp_dir().join(format!("tt-pupshots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let data = base.join("data");
    std::fs::create_dir_all(&data).unwrap();
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut assets = root.to_path_buf();
    if let Some(on) = std::env::var("TT_PUP_MODS").ok().filter(|m| !m.is_empty()) {
        // The mod overlay is per asset directory: a link of its own.
        #[cfg(unix)]
        {
            assets = base.join("assets");
            std::os::unix::fs::symlink(root.canonicalize().unwrap(), &assets).unwrap();
        }
        let mut settings = tt_shell::Settings::default();
        settings.mods.enabled = Some(on.split(',').map(|m| m.trim().to_string()).collect());
        std::fs::write(data.join("settings.ron"), settings.to_ron()).unwrap();
    }
    let cfg = Config { assets, data, args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    e.host.virtual_clock_ms = Some(0.0);
    assert!(e.boot());
    e.set_var("input.aim_mode", "0");
    let off = tt_engine::gfx::Offscreen::new(SIZE).unwrap();
    let fe = tt_engine::frontend::Frontend::new(&off.device, &off.queue, off.format, root.to_path_buf(), false);
    let mut v = View { off, fe, dir };
    v.ticks(&mut e, 1000);
    {
        let Engine { host, menu, .. } = &mut e;
        let m = menu.as_mut().unwrap();
        tt_engine::menu::Backend::new(host, &mut m.data).create_player("Tester").unwrap();
    }
    let g = CustomGame { mission: MISSION.into(), mode: 0, bots: Vec::new(), player_tank: 1, player_team: Team::Green, minutes: 10, replace_bots: true };
    {
        let Engine { host, menu, .. } = &mut e;
        menu.as_mut().unwrap().apply(MenuAction::StartCustom(g), host);
    }
    for i in 0..3000 {
        v.tick(&mut e);
        if i % 60 == 0 {
            e.key("space", true, None);
            e.key("space", false, None);
        }
        if player(&e).is_some() {
            break;
        }
    }
    assert!(player(&e).is_some(), "the player did not spawn");
    v.ticks(&mut e, 3000);
    (e, v)
}

/// Puts the tank at `p` facing north, turret centred, and lets it land.
fn place(e: &mut Engine, v: &mut View, p: Vec3) {
    let (o, id) = player(e).unwrap();
    let xf = format!("{} {} {} 0 0 1 0", p.x, p.y, p.z + 1.0);
    let g = e.host.game.as_mut().unwrap();
    g.set_transform(o, &xf);
    if let Some(t) = g.world.tank_mut(id) {
        t.vel = Vec3::ZERO;
        t.yaw_n = 0.0;
        t.pitch_n = 0.0;
    }
    v.ticks(e, 2600);
}

fn give(e: &mut Engine, projectile: &str) {
    let (_, id) = player(e).unwrap();
    let g = e.host.game.as_mut().unwrap();
    let p = g.projectile(projectile).unwrap();
    g.world.set_projectile(id, p);
}

fn set_pitch(e: &mut Engine, pitch_n: f32) {
    let (_, id) = player(e).unwrap();
    if let Some(t) = e.host.game.as_mut().unwrap().world.tank_mut(id) {
        t.pitch_n = pitch_n;
    }
}

fn live(e: &Engine, name: &str) -> Vec<Vec3> {
    e.host.game.as_ref().unwrap().world.projectiles.iter().filter(|p| p.is_live() && p.data.name == name).map(|p| p.pos).collect()
}

#[test]
#[ignore]
fn powerup_shots() {
    let Some(dir) = std::env::var_os("TT_PUP_SHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let root = assets();
    big_stack(move || {
        let (mut e, mut v) = start(&root, dir);
        let e = &mut e;
        if std::env::var("TT_PUP_ONLY").is_ok_and(|v| v == "rocket") {
            rocket_shots(e, &mut v);
            let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
            return;
        }
        if std::env::var("TT_PUP_ONLY").is_ok_and(|v| v == "mortar_reticle") {
            place(e, &mut v, Vec3::new(-30.0, -60.0, 100.0));
            give(e, "MortarProjectile");
            set_pitch(e, 0.0);
            v.ticks(e, 600);
            v.shot("mortar_reticle");
            let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
            return;
        }
        if std::env::var("TT_PUP_ONLY").is_ok_and(|v| v == "scatter_reticle") {
            place(e, &mut v, Vec3::new(0.0, -40.0, 100.0));
            give(e, "ScatterProjectile");
            set_pitch(e, 0.0);
            v.ticks(e, 600);
            v.shot("scatter_reticle");
            let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
            return;
        }
        let only_scatter = std::env::var("TT_PUP_ONLY").is_ok_and(|v| v == "scatter");
        if !only_scatter {
            mortar_shots(e, &mut v);
        }
        scatter_shots(e, &mut v);
        // The two test pick-ups look like every weapon power-up, through
        // the lamp's 1.7 s flash cycle.
        let lamps = std::env::var("TT_PUP_LAMPS").map_or(true, |v| v != "0") && !only_scatter;
        for (name, at) in [("mortar", Vec3::new(-70.0, -50.0, 100.0)), ("scatter", Vec3::new(70.0, -50.0, 100.0))].into_iter().filter(|_| lamps) {
            let x = if at.x < 0.0 { 2.5 } else { -2.5 };
            v.fe.camera_override = Some(look(at + Vec3::new(x, -6.0, 2.2), at + Vec3::new(0.0, 0.0, 0.9)));
            // Every tick of one cycle (pick the lit frames afterwards).
            for k in 0..54 {
                v.tick(e);
                v.shot(&format!("lamp_{name}_{k:02}"));
            }
        }
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

/// Fires the mortar and calls `each(view, ticks since the launch)` every
/// tick before drawing while the shell flies (`before` may move the camera).
fn fly(e: &mut Engine, v: &mut View, mut before: impl FnMut(&mut View, Option<Vec3>), shots: &[(u32, &str)]) {
    e.host.client.moves.trigger(0);
    let mut fired_at = None;
    for k in 0..400u32 {
        before(v, live(e, "MortarProjectile").first().copied());
        v.tick(e);
        let shell = live(e, "MortarProjectile");
        if fired_at.is_none() && !shell.is_empty() {
            fired_at = Some(k);
        }
        let Some(f) = fired_at else { continue };
        if shell.is_empty() {
            break;
        }
        for (at, name) in shots {
            if k - f == *at {
                v.shot(&format!("{name}_{at:02}"));
            }
        }
    }
    assert!(fired_at.is_some(), "the mortar did not fire");
    e.host.client.moves.trigger(0);
}

/// The mortar rocket in flight: from the player's view (real scale and
/// distance), from 14 m beside it as it climbs, tops out and comes down,
/// and from 40 m behind and above the tank.
fn rocket_shots(e: &mut Engine, v: &mut View) {
    let tank_at = Vec3::new(-30.0, -60.0, 100.0);
    place(e, v, tank_at);
    give(e, "MortarProjectile");
    set_pitch(e, 0.0);
    v.ticks(e, 1500);
    fly(e, v, |_, _| {}, &[(4, "rocket_player"), (10, "rocket_player"), (20, "rocket_player"), (32, "rocket_player"), (45, "rocket_player")]);
    v.ticks(e, 2500);
    fly(
        e,
        v,
        |v, q| {
            if let Some(q) = q {
                v.fe.camera_override = Some(look(q + Vec3::new(14.0, 2.0, 1.5), q));
            }
        },
        &[(6, "rocket_side"), (20, "rocket_side"), (31, "rocket_side"), (42, "rocket_side"), (54, "rocket_side")],
    );
    v.fe.camera_override = None;
    v.ticks(e, 2500);
    v.fe.camera_override = Some(look(tank_at + Vec3::new(-25.0, -30.0, 22.0), tank_at + Vec3::new(0.0, 40.0, 15.0)));
    fly(e, v, |_, _| {}, &[(12, "rocket_mid"), (30, "rocket_mid"), (48, "rocket_mid")]);
    v.fe.camera_override = None;
}

fn mortar_shots(e: &mut Engine, v: &mut View) {
    {
        // Mortar: the reticle at three barrel pitches (arc, landing spot, blast ring).
        let tank_at = Vec3::new(-30.0, -60.0, 100.0);
        place(e, v, tank_at);
        give(e, "MortarProjectile");
        for (pitch, tag) in [(0.0, "level"), (-1.0, "barrel_up"), (1.0, "barrel_down")] {
            set_pitch(e, pitch);
            v.ticks(e, 600);
            v.shot(&format!("mortar_reticle_{tag}"));
        }
        // The same reticle from the side: the whole high arc.
        set_pitch(e, 0.0);
        v.ticks(e, 400);
        let side = look(tank_at + Vec3::new(75.0, 40.0, 25.0), tank_at + Vec3::new(0.0, 45.0, 22.0));
        v.fe.camera_override = Some(side);
        v.ticks(e, 64);
        v.shot("mortar_reticle_side");
        // The shell's flight from the side, and the blast.
        v.ticks(e, 1500);
        e.host.client.moves.trigger(0);
        let mut fired_at = None;
        for k in 0..400 {
            v.tick(e);
            let shell = live(e, "MortarProjectile");
            if fired_at.is_none() && !shell.is_empty() {
                fired_at = Some(k);
            }
            if let Some(f) = fired_at {
                // About 2 s (62 ticks) of flight with the barrel level.
                match k - f {
                    12 => v.shot("mortar_arc_climb"),
                    31 => v.shot("mortar_arc_top"),
                    50 => v.shot("mortar_arc_down"),
                    _ => {}
                }
                if shell.is_empty() {
                    v.ticks(e, 352);
                    v.shot("mortar_blast_side");
                    break;
                }
            }
        }
        assert!(fired_at.is_some(), "the mortar did not fire");
        e.host.client.moves.trigger(0);
        // The blast from the player's view.
        v.fe.camera_override = None;
        v.ticks(e, 2000);
        e.host.client.moves.trigger(0);
        let mut seen = false;
        for _ in 0..400 {
            v.tick(e);
            let n = live(e, "MortarProjectile").len();
            seen |= n > 0;
            if seen && n == 0 {
                v.ticks(e, 192);
                v.shot("mortar_blast");
                v.ticks(e, 160);
                v.shot("mortar_blast_late");
                break;
            }
        }
        e.host.client.moves.trigger(0);
        v.ticks(e, 1500);
        // The mortar's blast and the Splash one seen from 30 m, 0.35 s in.
        for (p, file) in [("MortarProjectile", "blast_mortar_30m"), ("SplashProjectile", "blast_splash_30m")] {
            give(e, p);
            set_pitch(e, 0.0);
            v.ticks(e, 2000);
            e.host.client.moves.trigger(0);
            let mut last = None;
            for _ in 0..400 {
                v.tick(e);
                match live(e, p).first() {
                    Some(&q) => last = Some(q),
                    None if last.is_some() => break,
                    None => {}
                }
            }
            let at = last.expect("fired");
            v.fe.camera_override = Some(look(at + Vec3::new(0.0, -30.0, 8.0), at + Vec3::new(0.0, 0.0, 3.0)));
            v.ticks(e, 352);
            v.shot(file);
            v.fe.camera_override = None;
            e.host.client.moves.trigger(0);
            v.ticks(e, 1500);
        }
    }
}

/// Scatter shot: the reticle (the cone and the pellets' reach), a volley
/// from behind, from the side and up close (the tracer streaks), and the
/// pattern on a Heavy tank 20 m ahead (reticle spots, then the hits).
fn scatter_shots(e: &mut Engine, v: &mut View) {
    let tank_at = Vec3::new(0.0, -40.0, 100.0);
    place(e, v, tank_at);
    give(e, "ScatterProjectile");
    set_pitch(e, 0.0);
    v.ticks(e, 600);
    v.shot("scatter_reticle");
    set_pitch(e, 0.6);
    v.ticks(e, 600);
    v.shot("scatter_reticle_down");
    set_pitch(e, 0.0);
    v.ticks(e, 800);
    // Fires one shot; calls `at(k)` every tick after the pellets appear
    // (k = 0, 1, ...) until it returns false.
    let volley = |e: &mut Engine, v: &mut View, at: &mut dyn FnMut(&mut Engine, &mut View, u32) -> bool| {
        e.host.client.moves.trigger(0);
        let mut fired = false;
        for _ in 0..60 {
            v.tick(e);
            if live(e, "ScatterProjectile").len() == 7 {
                fired = true;
                let mut k = 0;
                while at(e, v, k) {
                    v.tick(e);
                    k += 1;
                }
                break;
            }
        }
        assert!(fired, "the scatter shot did not fire");
        e.host.client.moves.trigger(0);
        v.fe.camera_override = None;
        v.ticks(e, 1000);
    };
    let muzzle = tank_at + Vec3::new(0.0, 2.0, 2.0);
    // From behind: the cone opening up.
    volley(e, v, &mut |_, v, k| {
        match k {
            2 => {
                v.shot("scatter_volley");
                // (The camera takes effect from the next frame drawn.)
                v.fe.camera_override = Some(look(muzzle + Vec3::new(0.6, -5.0, 1.2), muzzle + Vec3::new(0.0, 30.0, 0.0)));
            }
            4 => v.shot("scatter_volley_behind"),
            8 => v.shot("scatter_volley_behind_late"),
            _ => {}
        }
        k < 8
    });
    // From the side: above and below the barrel too.
    volley(e, v, &mut |_, v, k| {
        v.fe.camera_override = Some(look(muzzle + Vec3::new(14.0, 16.0, 0.5), muzzle + Vec3::new(0.0, 16.0, 0.0)));
        match k {
            4 => v.shot("scatter_volley_side"),
            7 => v.shot("scatter_volley_side_late"),
            _ => {}
        }
        k < 7
    });
    // Up close: the pellets and their tracer streaks.
    volley(e, v, &mut |e, v, k| {
        if k == 2 {
            // Where the pellets will be next tick, from the side and behind.
            let g = e.host.game.as_ref().unwrap();
            let next: Vec<Vec3> = g.world.projectiles.iter().filter(|p| p.is_live() && p.data.name == "ScatterProjectile").map(|p| p.pos + p.vel * (0.5 * tt_game::DT)).collect();
            let mid = next.iter().copied().sum::<Vec3>() / next.len().max(1) as f32;
            v.fe.camera_override = Some(look(mid + Vec3::new(4.5, -1.0, 0.6), mid));
        }
        if k == 3 {
            v.shot("scatter_pellets_close");
        }
        k < 3
    });
    // A Heavy tank side-on 20 m ahead: the reticle's spots on it (where
    // the next shot's pellets hit), then the hits.
    {
        let g = e.host.game.as_mut().unwrap();
        let ground = g.world.terrain.as_ref().map_or(100.0, |h| h.height(tank_at.x, tank_at.y + 22.0));
        let rot = glam::Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2);
        g.add_tank("HeavyTank", "ScatterTarget", Vec3::new(tank_at.x, tank_at.y + 22.0, ground + 0.5), rot, None).expect("HeavyTank");
    }
    v.ticks(e, 1500);
    v.shot("scatter_target_20m_reticle");
    let target = tank_at + Vec3::new(0.0, 22.0, 1.2);
    v.fe.camera_override = Some(look(target + Vec3::new(-2.0, -9.0, 1.0), target));
    v.ticks(e, 64);
    v.shot("scatter_target_20m_spots");
    volley(e, v, &mut |_, v, k| {
        v.fe.camera_override = Some(look(target + Vec3::new(-2.0, -9.0, 1.0), target));
        match k {
            6 => v.shot("scatter_target_20m_hits"),
            8 => v.shot("scatter_target_20m_hits_late"),
            _ => {}
        }
        k < 8
    });
}

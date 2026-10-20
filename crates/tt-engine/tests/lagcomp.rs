//! Lag compensation: is it needed? (docs/netcode.md §2.6 "Optional v2", §9).
//!
//! A dedicated server with 6 bots and 2 remote clients over `MemNet` per link
//! profile. Measured:
//!
//! 1. The shooter's **view delay V**: a client fires on its (predicted) tick;
//!    the matching server shot (same `(tank, fire_count)`) spawns on server
//!    tick `S`. The client drew the remote tanks at `render_tick` `r` at that
//!    moment, so its targets were `V = S − r` ticks in the past.
//! 2. **Target displacement during V** from the server history: `|p(t+V) −
//!    p(t)|` and the residual after linear extrapolation `|p(t+V) − p(t) −
//!    v(t)·V|`, and tank speeds.
//! 3. **Hit sizes**: the server tests a projectile's ray against the target's
//!    oriented shape box (`segment_tank`, not a sphere); splash weapons damage
//!    every tank whose world box overlaps a cube of half-size `blast_radius`
//!    around the impact, and explode near a tank within `0.7·R + splash`.
//! 4. **Hit rates** of a geometric aimbot run offline on the server history
//!    (no gravity: the aimbot is assumed to compensate drop perfectly): each
//!    client tank aims at every live tank 15–150 m away, at the target centre
//!    with linear lead, from what it draws (pose and velocity at `S − V`),
//!    then the straight ray from its centre is tested against the target's
//!    true box at arrival `S + T`. Variants: `ideal` (sees the server present,
//!    = what perfect rewind lag compensation gives), `lead T` (leads for the
//!    flight time only, what a player does), `lead T+V` (also leads for V),
//!    and two server-side rewinds of the `lead T` shot (§2.6 v2: targets
//!    rewound by V for the first 200 ms of flight, or for the first V).
//!
//! The measurement takes ~1 min wall in release:
//! `cargo test --release -p tt-engine --test lagcomp -- --ignored --nocapture --test-threads=2`

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use glam::{Mat3, Vec3};
use tt_engine::Engine;
use tt_game::TankParams;
use tt_net::{LinkConditions, MemNet};

mod common;
use common::*;

const MISSION: &str = "missions/tt1_1.ron";

/// One tank on the server after a tick.
#[derive(Clone)]
struct Snap {
    pos: Vec3,
    vel: Vec3,
    rot: Mat3,
    corpse: bool,
    data: Arc<TankParams>,
}

struct Game {
    net: MemNet,
    server: Engine,
    clients: Vec<Engine>,
    t: u64,
    /// Server tick -> tank id -> state.
    hist: HashMap<u32, HashMap<u32, Snap>>,
    /// Server tick -> server tank id of each client's control tank.
    ctrl: HashMap<u32, Vec<Option<u32>>>,
    /// Shots: (server tank id, fire_count) -> server tick of the spawn.
    server_fires: HashMap<(u32, u32), u32>,
    server_last: HashMap<u32, u32>,
    /// Shots: (server tank id, fire_count) -> (client, render tick when fired).
    client_fires: HashMap<(u32, u32), (usize, f64)>,
    client_last: Vec<(u32, u32)>,
}

impl Game {
    fn new(root: &Path, tag: &str, cond: LinkConditions, clients: usize) -> Game {
        let net = MemNet::new(11);
        net.set_conditions(cond);
        let mut server = engine(root, &format!("{tag}-server"), &["-dedicated", "-mission", MISSION]);
        server.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 1]));
        assert!(server.boot());
        // 6 bots with the humans in (one slot more: one bot leaves per human beyond the first).
        {
            let slots = &mut server.host.srv.state_mut().bot_slots;
            slots.resize(7, Default::default());
            slots[6] = tt_gameplay::server::BotSlot::new("SilverMediumTank", "Extra [Bot]", "BlueTeam");
        }
        tt_engine::gameplay::server_host::with_server(&mut server.host, |s, b| s.balance_bots(b));
        let mut cl = Vec::new();
        for i in 0..clients {
            let mut c = engine(root, &format!("{tag}-client{i}"), &[]);
            assert!(c.boot());
            new_player(&mut c, &format!("Player{i}"));
            c.host.netgame.factory = Some(mem_factory(&net, [10, 0, 1, i as u8 + 1]));
            tt_engine::net::connect_to(&mut c.host, "10.0.0.1:28000", "", &format!("Player{i}"), "");
            cl.push(c);
        }
        Game { net, server, clients: cl, t: 0, hist: HashMap::new(), ctrl: HashMap::new(), server_fires: HashMap::new(), server_last: HashMap::new(), client_fires: HashMap::new(), client_last: vec![(0, 0); clients] }
    }

    fn step(&mut self, record: bool) {
        self.t += 32;
        self.net.set_now(Duration::from_millis(self.t));
        self.server.host.virtual_clock_ms = Some(self.t as f64);
        self.server.tick();
        if record {
            if let Some(g) = self.server.host.game.as_ref() {
                let tick = (g.world.time_ms / 32) as u32;
                let mut m = HashMap::new();
                for t in g.world.tanks.iter().filter(|t| !t.deleted) {
                    m.insert(t.id.0, Snap { pos: t.pos, vel: t.vel, rot: t.rot, corpse: t.is_corpse, data: t.data.clone() });
                    let last = self.server_last.entry(t.id.0).or_insert(t.fire_count);
                    for fc in *last + 1..=t.fire_count {
                        self.server_fires.entry((t.id.0, fc)).or_insert(tick);
                    }
                    *last = t.fire_count;
                }
                self.hist.insert(tick, m);
                let ctrl = self.control_nets();
                self.ctrl.insert(tick, ctrl);
            }
        }
        for (i, c) in self.clients.iter_mut().enumerate() {
            c.host.virtual_clock_ms = Some(self.t as f64);
            c.tick();
            if !record {
                continue;
            }
            // The own (predicted) tank's shots, with the render time they were aimed in.
            let Some(rep) = c.host.netgame.client_rep() else { continue };
            let Some((snet, tid)) = rep.pred.tank else { continue };
            let Some(t) = c.host.game.as_ref().and_then(|g| g.world.tank(tid)) else { continue };
            let (lnet, lfc) = self.client_last[i];
            let from = if lnet == snet { lfc } else { t.fire_count };
            for fc in from + 1..=t.fire_count {
                self.client_fires.entry((snet, fc)).or_insert((i, rep.render_tick));
            }
            self.client_last[i] = (snet, t.fire_count);
        }
    }

    fn in_play(&self) -> bool {
        self.clients.iter().all(|c| tt_engine::netgame::control_object(&c.host).is_some())
    }

    /// Server tank id of each remote client's control tank (client order:
    /// connections sorted, as `netplay_game.rs`).
    fn control_nets(&self) -> Vec<Option<u32>> {
        let g = self.server.host.game.as_ref().unwrap();
        let mut conns: Vec<_> = self.server.host.net.control.iter().filter(|(c, _)| tt_engine::netgame::is_remote(&self.server.host, **c)).collect();
        conns.sort();
        conns.into_iter().map(|(_, o)| g.tank_of(*o).map(|t| t.0)).collect()
    }
}

fn drive(g: &mut Game, tick: u64) {
    for (i, c) in g.clients.iter_mut().enumerate() {
        let turn = i32::from((tick / 90 + i as u64).is_multiple_of(2));
        c.host.client.moves.set_drive(1.0, turn as f32);
        if tick % 30 == i as u64 * 7 {
            c.host.client.moves.trigger(0);
        }
        if tick % 125 == 3 {
            tt_engine::net::send_cmd(&mut c.host, tt_gameplay::ClientCmd::SpawnPlayer);
        }
    }
}

fn pct(v: &mut [f32], p: f64) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len().max(1) as f32
}

/// Pose of tank `id` at fractional server tick `t` (lerped; rotation of the
/// nearer tick). None if missing, a corpse or teleporting inside the interval.
fn pose(g: &Game, id: u32, t: f64) -> Option<Snap> {
    let k = t.floor() as u32;
    let f = (t - t.floor()) as f32;
    let a = g.hist.get(&k)?.get(&id)?;
    let b = g.hist.get(&(k + 1))?.get(&id)?;
    if a.corpse || b.corpse || (a.pos - b.pos).length() > 5.0 {
        return None;
    }
    let mut s = if f < 0.5 { a.clone() } else { b.clone() };
    s.pos = a.pos.lerp(b.pos, f);
    s.vel = a.vel.lerp(b.vel, f);
    Some(s)
}

fn centre(s: &Snap) -> Vec3 {
    s.pos + s.rot * s.data.geometry.bounds.center()
}

/// Distance from point `p` to the tank box at pose `s`.
fn box_dist(s: &Snap, p: Vec3) -> f32 {
    let l = s.rot.transpose() * (p - s.pos);
    let b = &s.data.geometry.bounds;
    (l.clamp(b.min, b.max) - l).length()
}

/// Ray `a -> b` against the tank box at pose `s` (the server's `segment_tank`).
fn ray_hits(s: &Snap, a: Vec3, b: Vec3) -> bool {
    let inv = s.rot.transpose();
    tt_game::geom::segment_aabb(inv * (a - s.pos), inv * (b - s.pos), &s.data.geometry.bounds).is_some()
}

/// A weapon for the aimbot: name, speed m/s, splash m.
struct Weapon {
    name: &'static str,
    speed: f32,
    splash: f32,
}

const WEAPONS: [Weapon; 6] = [
    Weapon { name: "defaultProjectile", speed: 85.0, splash: 0.0 },
    Weapon { name: "LightProjectile", speed: 77.0, splash: 0.0 },
    Weapon { name: "HeavyProjectile", speed: 120.0, splash: 0.0 },
    Weapon { name: "SpeedyProjectile", speed: 190.0, splash: 0.0 },
    Weapon { name: "BounceProjectile", speed: 35.0, splash: 2.0 },
    Weapon { name: "SplashProjectile", speed: 40.0, splash: 5.0 },
];

/// Aim / server variants. `ideal`: the shooter sees the server present (=
/// full-flight rewind lag compensation). `lead T`: leads what it draws for the
/// flight time. `lead T+V`: also leads for V. `rewind 200`/`rewind V`: `lead T`
/// aim, the server tests the projectile against targets rewound by V while the
/// impact comes within the first `cap` of flight (cap 200 ms as §2.6 v2 says,
/// or cap = V), decaying linearly to no rewind over the next `cap`.
const VARIANTS: [&str; NV] = ["ideal", "lead T", "lead T+V", "rewind 200", "rewind V"];
const NV: usize = 5;

/// Rewind (in ticks) the server applies to an impact after `t` s of flight.
fn rewind(v_ticks: f64, t: f32, cap: f32) -> f64 {
    let w = if t <= cap { 1.0 } else { (1.0 - (t - cap) / cap).max(0.0) };
    v_ticks * w as f64
}

/// Prints the hit sizes and weapon data from the server's defs.
fn print_sizes(g: &mut Game) {
    let sg = g.server.host.game.as_mut().unwrap();
    for name in ["LightTank", "MediumTank", "HeavyTank"] {
        let Some(d) = sg.tank_data(name) else {
            println!("lagcomp sizes: {name}: missing");
            continue;
        };
        let geo = &d.geometry;
        let h = (geo.bounds.max - geo.bounds.min) * 0.5;
        println!(
            "lagcomp sizes: {name}: box half-extents x {:.2} y {:.2} z {:.2} m; r (small radius, half XY diagonal) {:.2} m; R (radius) {:.2} m; splash explode-near radius 0.7R+2 = {:.2} m, 0.7R+5 = {:.2} m",
            h.x,
            h.y,
            h.z,
            geo.small_radius(),
            geo.big_radius(),
            0.7 * geo.big_radius() + 2.0,
            0.7 * geo.big_radius() + 5.0
        );
    }
    for w in &WEAPONS {
        let Some(p) = sg.projectile(w.name) else {
            println!("lagcomp sizes: {}: missing", w.name);
            continue;
        };
        println!(
            "lagcomp sizes: {}: velocity {} m/s, splashArea {} m, gravityScale {}, numBounce {}, flight 50 m {:.2} s, 100 m {:.2} s",
            p.name,
            p.velocity,
            p.blast_radius,
            p.gravity_scale,
            p.bounces,
            50.0 / p.velocity,
            100.0 / p.velocity
        );
        assert_eq!(p.velocity, w.speed, "{} speed", w.name);
        assert_eq!(p.blast_radius, w.splash, "{} splash", w.name);
    }
}

/// Results of one profile, for assertions.
struct Measured {
    v_ticks: Vec<f32>,
    /// The server's own estimate (the lag compensation uses it), per client.
    server_v: Vec<f32>,
}

fn measure(root: &Path, tag: &str, cond: LinkConditions, seconds: u64, sizes: bool) -> Measured {
    let mut g = Game::new(root, tag, cond, 2);
    let mut ticks = 0;
    while !g.in_play() {
        g.step(false);
        ticks += 1;
        assert!(ticks < 3000, "{tag}: clients did not reach the game");
    }
    if sizes {
        print_sizes(&mut g);
    }
    // Settle (interpolation delay and clock sync), then measure.
    for tick in 0..60 {
        drive(&mut g, tick);
        g.step(false);
    }
    for tick in 0..seconds * 1000 / 32 {
        drive(&mut g, tick);
        g.step(true);
    }

    // 1. View delay per client.
    let mut v_by_client: Vec<Vec<f32>> = vec![Vec::new(); 2];
    for (key, (i, r)) in &g.client_fires {
        if let Some(&s) = g.server_fires.get(key) {
            v_by_client[*i].push((s as f64 - r) as f32);
        }
    }
    let unmatched = g.client_fires.len() - v_by_client.iter().map(|v| v.len()).sum::<usize>();
    let mut v_all = Vec::new();
    let mut v_mean = [0f32; 2];
    for (i, v) in v_by_client.iter_mut().enumerate() {
        v_mean[i] = mean(v);
        let (p5, p50, p95) = (pct(v, 0.05), pct(v, 0.5), pct(v, 0.95));
        println!("lagcomp {tag}: client {i}: view delay V over {} shots: mean {:.0} ms, p5 {:.0} p50 {:.0} p95 {:.0} ms ({:.2} ticks mean)", v.len(), v_mean[i] * 32.0, p5 * 32.0, p50 * 32.0, p95 * 32.0, v_mean[i]);
        v_all.extend_from_slice(v);
    }
    let vm = mean(&v_all);
    println!("lagcomp {tag}: view delay V all: mean {:.0} ms p95 {:.0} ms; {unmatched} client shots without a server spawn; {} server shots in total", vm * 32.0, pct(&mut v_all.clone(), 0.95) * 32.0, g.server_fires.len());

    // 2. Target displacement during V (all tanks, every 2nd tick).
    let mut keys: Vec<u32> = g.hist.keys().copied().collect();
    keys.sort();
    let (mut disp, mut resid, mut speed) = (Vec::new(), Vec::new(), Vec::new());
    for &t in keys.iter().step_by(2) {
        let ids: Vec<u32> = g.hist[&t].keys().copied().collect();
        for id in ids {
            let (Some(a), Some(b)) = (pose(&g, id, t as f64), pose(&g, id, t as f64 + vm as f64)) else { continue };
            let d = b.pos - a.pos;
            if d.length() > 40.0 * vm * 0.032 + 1.0 {
                continue;
            }
            disp.push(d.length());
            resid.push((d - a.vel * vm * 0.032).length());
            speed.push(a.vel.length());
        }
    }
    let n = disp.len();
    println!(
        "lagcomp {tag}: target displacement in V={:.0} ms over {n} samples: |dp| p50 {:.2} p95 {:.2} p99 {:.2} m; after linear extrapolation p50 {:.2} p95 {:.2} p99 {:.2} m; speed p50 {:.1} p95 {:.1} max {:.1} m/s",
        vm * 32.0,
        pct(&mut disp, 0.5),
        pct(&mut disp, 0.95),
        pct(&mut disp, 0.99),
        pct(&mut resid, 0.5),
        pct(&mut resid, 0.95),
        pct(&mut resid, 0.99),
        pct(&mut speed, 0.5),
        pct(&mut speed, 0.95),
        pct(&mut speed, 1.0)
    );

    // 4. Aimbot hit rates against the server truth.
    // [weapon][variant] -> (shots, direct hits, splash hits); miss distances for the default weapon.
    let mut res = [[(0u32, 0u32, 0u32); NV]; WEAPONS.len()];
    let mut miss: [Vec<f32>; NV] = Default::default();
    // Default weapon by range bin: [bin][variant] -> (shots, hits).
    const BINS: [(f32, f32); 3] = [(15.0, 40.0), (40.0, 80.0), (80.0, 150.0)];
    let mut by_range = [[(0u32, 0u32); NV]; 3];
    for &s in keys.iter().step_by(2) {
        let Some(ctrl) = g.ctrl.get(&s) else { continue };
        for (ci, shooter) in ctrl.iter().enumerate() {
            let Some(shooter) = *shooter else { continue };
            let Some(sh) = pose(&g, shooter, s as f64) else { continue };
            let muzzle = centre(&sh);
            let v_ticks = v_mean[ci.min(1)] as f64;
            let targets: Vec<u32> = g.hist[&s].keys().copied().filter(|&t| t != shooter).collect();
            for tid in targets {
                let Some(now) = pose(&g, tid, s as f64) else { continue };
                let dist = (centre(&now) - muzzle).length();
                if !(15.0..=150.0).contains(&dist) {
                    continue;
                }
                let Some(seen) = pose(&g, tid, s as f64 - v_ticks) else { continue };
                'w: for (wi, w) in WEAPONS.iter().enumerate() {
                    // Shots of all variants are counted together or not at all.
                    let mut outcome = [(false, false, 0f32); NV];
                    for (vi, _) in VARIANTS.iter().enumerate() {
                        let (view, extra) = match vi {
                            0 => (&now, 0.0f32),
                            2 => (&seen, v_ticks as f32 * 0.032),
                            _ => (&seen, 0.0),
                        };
                        let c0 = centre(view);
                        let mut tt = (c0 - muzzle).length() / w.speed;
                        let mut aim = c0;
                        for _ in 0..5 {
                            aim = c0 + view.vel * (tt + extra);
                            tt = (aim - muzzle).length() / w.speed;
                        }
                        let back = match vi {
                            3 => rewind(v_ticks, tt, 0.2),
                            4 => rewind(v_ticks, tt, v_ticks as f32 * 0.032),
                            _ => 0.0,
                        };
                        let Some(truth) = pose(&g, tid, s as f64 + tt as f64 / 0.032 - back) else { continue 'w };
                        if (truth.pos - seen.pos).length() > 35.0 * (tt + v_ticks as f32 * 0.032) + 2.0 {
                            continue 'w;
                        }
                        let dir = (aim - muzzle).normalize();
                        let range = (aim - muzzle).length();
                        let direct = ray_hits(&truth, muzzle, muzzle + dir * (range + 15.0));
                        // Closest approach of the ray near the target to its box.
                        let mut d = f32::MAX;
                        let mut x = (range - 8.0).max(0.0);
                        while x <= range + 8.0 {
                            d = d.min(box_dist(&truth, muzzle + dir * x));
                            x += 0.25;
                        }
                        let tc = centre(&truth);
                        let perp = ((tc - muzzle) - dir * (tc - muzzle).dot(dir)).length();
                        outcome[vi] = (direct, direct || (w.splash > 0.0 && d <= w.splash), perp);
                    }
                    for vi in 0..NV {
                        let r = &mut res[wi][vi];
                        r.0 += 1;
                        r.1 += outcome[vi].0 as u32;
                        r.2 += outcome[vi].1 as u32;
                        if wi == 0 {
                            miss[vi].push(outcome[vi].2);
                            let bin = BINS.iter().position(|(a, b)| (*a..*b).contains(&dist)).unwrap_or(2);
                            by_range[bin][vi].0 += 1;
                            by_range[bin][vi].1 += outcome[vi].0 as u32;
                        }
                    }
                }
            }
        }
    }
    for (wi, w) in WEAPONS.iter().enumerate() {
        let line: Vec<String> = VARIANTS
            .iter()
            .enumerate()
            .map(|(vi, label)| {
                let (n, d, s) = res[wi][vi];
                let pc = |x: u32| x as f64 * 100.0 / n.max(1) as f64;
                if w.splash > 0.0 {
                    format!("{label} {:.1} % direct / {:.1} % incl. splash", pc(d), pc(s))
                } else {
                    format!("{label} {:.1} %", pc(d))
                }
            })
            .collect();
        println!("lagcomp {tag}: aimbot {} ({} m/s, splash {} m) over {} shots: {}", w.name, w.speed, w.splash, res[wi][0].0, line.join(", "));
    }
    for (bi, (a, b)) in BINS.iter().enumerate() {
        let line: Vec<String> = VARIANTS.iter().enumerate().map(|(vi, label)| format!("{label} {:.1} %", by_range[bi][vi].1 as f64 * 100.0 / by_range[bi][vi].0.max(1) as f64)).collect();
        println!("lagcomp {tag}: aimbot defaultProjectile {a:.0}-{b:.0} m over {} shots: {}", by_range[bi][0].0, line.join(", "));
    }
    for (vi, label) in VARIANTS.iter().enumerate() {
        let m = &mut miss[vi];
        println!("lagcomp {tag}: aimbot defaultProjectile {label}: centre miss distance p50 {:.2} p95 {:.2} p99 {:.2} m", pct(m, 0.5), pct(m, 0.95), pct(m, 0.99));
    }
    for (o, st, _) in tt_engine::netgame::server_stats(&mut g.server.host) {
        println!("lagcomp {tag}: to client {o}: rtt {:.0} ms, loss {:.1} %", st.rtt.as_secs_f64() * 1000.0, st.loss * 100.0);
    }
    for (i, c) in g.clients.iter().enumerate() {
        let rep = c.host.netgame.client_rep().unwrap();
        println!("lagcomp {tag}: client {i}: interpolation delay D {:.1} ticks", rep.delay);
    }
    let server_v = tt_engine::netgame::view_delays(&g.server.host).into_iter().map(|(_, v)| v).collect::<Vec<f32>>();
    println!("lagcomp {tag}: server estimate of V per client: {:?} ms", server_v.iter().map(|v| (v * 32.0).round()).collect::<Vec<_>>());
    Measured { v_ticks: v_all, server_v }
}

/// The measurement: lan, dsl, bad_wifi, awful, 60 s each.
/// Run: `cargo test --release -p tt-engine --test lagcomp -- --ignored --nocapture --test-threads=2`
#[test]
#[ignore]
fn lag_compensation_measurement() {
    let root = assets();
    big_stack(move || {
        let secs = std::env::var("TT_LAGCOMP_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        let profiles = [("lan", LinkConditions::lan()), ("dsl", LinkConditions::dsl()), ("bad_wifi", LinkConditions::bad_wifi()), ("awful", LinkConditions::awful())];
        for (i, (tag, cond)) in profiles.into_iter().enumerate() {
            let start = std::time::Instant::now();
            let m = measure(&root, tag, cond, secs, i == 0);
            println!("lagcomp {tag}: {} s of play in {:.1} s wall", secs, start.elapsed().as_secs_f64());
            assert!(m.v_ticks.len() > 20, "{tag}: {} view delay samples", m.v_ticks.len());
        }
    });
}

/// A few seconds on DSL: the view delay is measurable and plausible
/// (render time ≈ 2 ticks interpolation + one-way trip behind the server,
/// shot applied ≈ one-way trip ahead).
#[test]
fn lag_compensation_sanity_dsl() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let start = std::time::Instant::now();
        let m = measure(&root, "sanity-dsl", LinkConditions::dsl(), 6, true);
        println!("lagcomp sanity-dsl: {:.1} s wall", start.elapsed().as_secs_f64());
        let v = mean(&m.v_ticks);
        assert!(m.v_ticks.len() >= 5, "{} samples", m.v_ticks.len());
        // DSL: RTT 120 ms (3.75 ticks) + D ≈ 2–4 ticks.
        assert!((3.0..=12.0).contains(&v), "view delay {v} ticks");
        // The server estimates the same V from the render tick in the moves.
        assert_eq!(m.server_v.len(), 2);
        for s in &m.server_v {
            assert!((s - v).abs() < 1.0, "server V {s} vs measured {v} ticks");
        }
    });
}

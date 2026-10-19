//! Playing golden scenarios headless: a local game ([`run`]) or a dedicated
//! server with remote clients over `MemNet` ([`run_netplay`]).
//!
//! The configuration is the shipped game's: the Rust ports, the K/D/A
//! scoreboard (kill 2 / assist 1), the health-pack minimum, the default
//! prefs of a new install, and the new menu. Determinism: virtual clock
//! (advances with the sim), the script RNG seeded after boot, the shell
//! RNG from the seed, networking only over an in-memory `MemNet`
//! (isolated for local games), no LAN or master queries, no ban file.

use super::action::ScenarioAction;
use super::record::{self, Golden, GoldenAudio};
use super::drive::apply;
use crate::{Config, Engine, TICK_MS};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tt_menu::MenuBackend;
use tt_net::{Datagram, LinkConditions, MemNet};

/// A local scenario.
#[derive(Clone)]
pub struct GoldenScenario {
    pub name: String,
    pub seed: u32,
    /// Game time after the setup.
    pub ms: u32,
    /// Done once after boot and player creation.
    pub setup: Vec<ScenarioAction>,
    /// (ms after the setup, action).
    pub actions: Vec<(u32, ScenarioAction)>,
}

impl GoldenScenario {
    pub fn new(name: &str, seed: u32, ms: u32) -> GoldenScenario {
        GoldenScenario { name: name.into(), seed, ms, setup: Vec::new(), actions: Vec::new() }
    }

    pub fn setup(mut self, a: ScenarioAction) -> Self {
        self.setup.push(a);
        self
    }

    pub fn at(mut self, ms: u32, a: ScenarioAction) -> Self {
        self.actions.push((ms, a));
        self
    }

    /// `a` every `period` ms from `from` to the end.
    pub fn every(mut self, from: u32, period: u32, a: ScenarioAction) -> Self {
        let mut t = from;
        while t < self.ms {
            self.actions.push((t, a.clone()));
            t += period.max(TICK_MS);
        }
        self
    }

    /// A key press and release.
    pub fn tap(self, ms: u32, key: &str) -> Self {
        self.at(ms, ScenarioAction::key(key, true)).at(ms + TICK_MS, ScenarioAction::key(key, false))
    }

    /// Types `text` (one key per character, all at `ms`, in order).
    pub fn typing(mut self, ms: u32, text: &str) -> Self {
        for c in text.chars() {
            let key = if c == ' ' { "space".to_string() } else { c.to_ascii_lowercase().to_string() };
            self = self.at(ms, ScenarioAction::Type { key, text: c.to_string() });
        }
        self
    }
}

/// A dedicated server and remote clients over `MemNet`.
#[derive(Clone)]
pub struct NetScenario {
    pub name: String,
    /// `MemNet` seed (loss, jitter).
    pub seed: u64,
    pub cond: LinkConditions,
    /// Extra server command-line arguments (after `-dedicated -mission …`).
    pub server_args: Vec<String>,
    pub mission: String,
    pub server_setup: Vec<ScenarioAction>,
    /// Per client: (ms, action).
    pub clients: Vec<Vec<(u32, ScenarioAction)>>,
    pub ms: u32,
}

static RUNS: AtomicU32 = AtomicU32::new(0);

fn data_dir(name: &str, tag: &str) -> PathBuf {
    let n = RUNS.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("tt-golden-{name}-{tag}-{}-{n}", std::process::id()))
}

fn mem_factory(net: &MemNet, ip: [u8; 4]) -> crate::netgame::TransportFactory {
    let net = net.clone();
    let mut next = 40000u16;
    Box::new(move |port| {
        let port = if port == 0 {
            next += 1;
            next
        } else {
            port
        };
        Ok(Box::new(net.endpoint(SocketAddr::from((ip, port)))) as Box<dyn Datagram>)
    })
}

/// A fresh engine in the golden configuration, recording, not booted yet.
fn engine(name: &str, tag: &str, args: &[String], seed: u32, net: &MemNet, ip: [u8; 4]) -> Result<Engine, String> {
    let data = data_dir(name, tag);
    let _ = std::fs::remove_dir_all(&data);
    let cfg = Config { assets: tt_assets::layout::default_dir(), data, args: args.to_vec(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg)?;
    e.host.virtual_clock_ms = Some(0.0);
    e.host.shell.rng = tt_game::rng::Rng::new(seed | 1);
    let g = Golden::new();
    let out = g.shared();
    let inner = std::mem::replace(&mut e.host.audio, Box::new(crate::audio::NullAudio::default()));
    e.host.audio = Box::new(GoldenAudio { inner, out });
    e.host.golden = Some(g);
    e.host.netgame.factory = Some(mem_factory(net, ip));
    Ok(e)
}

fn boot(e: &mut Engine, player: Option<&str>, seed: u32) -> Result<(), String> {
    if !e.boot() {
        return Err("boot failed".into());
    }
    if let Some(p) = player {
        let Engine { host, menu, .. } = e;
        let m = menu.as_mut().ok_or("no menu")?;
        crate::menu::Backend::new(host, &mut m.data).create_player(p).map_err(|e| format!("{e:?}"))?;
    }
    e.host.rng.set_seed(seed as i32);
    Ok(())
}

fn act(e: &mut Engine, a: &ScenarioAction) {
    for l in a.lower() {
        apply(e, &l);
    }
}

fn golden(e: &Engine) -> &Golden {
    e.host.golden.as_ref().expect("recording")
}

/// One sim step at virtual time `t` (ms, after the step).
fn step(e: &mut Engine, t: u64) {
    e.host.virtual_clock_ms = Some(t as f64);
    e.tick();
    record::snapshot(e);
}

/// Shuts `e` down and returns the rest of its trace (saves, final files).
fn finish(mut e: Engine) -> Vec<String> {
    golden(&e).set_tick(e.ticks + 1);
    e.shutdown();
    let data = e.host.fs.data_dir().to_path_buf();
    let mut g = e.host.golden.take().expect("recording");
    record::finish(&mut g, &data);
    let lines = g.drain();
    let _ = std::fs::remove_dir_all(&data);
    lines
}

/// Plays a local scenario; its golden trace.
pub fn run(sc: &GoldenScenario) -> Result<Vec<String>, String> {
    let net = MemNet::new(1);
    let mut e = engine(&sc.name, "local", &[], sc.seed, &net, [10, 9, 9, 9])?;
    boot(&mut e, Some("Tester"), sc.seed)?;
    for a in &sc.setup {
        act(&mut e, a);
    }
    let mut actions = sc.actions.clone();
    actions.sort_by_key(|a| a.0);
    let mut next = 0;
    let mut lines = golden(&e).drain();
    for t in 0..sc.ms / TICK_MS {
        let now = t * TICK_MS;
        golden(&e).set_tick(e.ticks + 1);
        while next < actions.len() && actions[next].0 <= now {
            let a = actions[next].1.clone();
            act(&mut e, &a);
            next += 1;
        }
        step(&mut e, ((t + 1) * TICK_MS) as u64);
        lines.extend(golden(&e).drain());
    }
    lines.extend(finish(e));
    Ok(lines)
}

/// Plays a network scenario; one trace with the server's lines (`srv`)
/// and each client's (`cl<i>`), in step order.
pub fn run_netplay(sc: &NetScenario) -> Result<Vec<String>, String> {
    let net = MemNet::new(sc.seed);
    net.set_conditions(sc.cond.clone());
    let mut args: Vec<String> = ["-dedicated", "-mission", &sc.mission].iter().map(|s| s.to_string()).collect();
    args.extend(sc.server_args.iter().cloned());
    let mut server = engine(&sc.name, "server", &args, 1, &net, [10, 0, 0, 1])?;
    boot(&mut server, None, 1)?;
    for a in &sc.server_setup {
        act(&mut server, a);
    }
    let mut clients = Vec::new();
    for i in 0..sc.clients.len() {
        let mut c = engine(&sc.name, &format!("client{i}"), &[], 2 + i as u32, &net, [10, 0, 1, i as u8 + 1])?;
        boot(&mut c, Some(&format!("Player{i}")), 2 + i as u32)?;
        clients.push(c);
    }
    let mut lines = Vec::new();
    let tag = |lines: &mut Vec<String>, who: &str, e: &Engine| lines.extend(golden(e).drain().into_iter().map(|l| format!("{who} {l}")));
    tag(&mut lines, "srv", &server);
    for (i, c) in clients.iter().enumerate() {
        tag(&mut lines, &format!("cl{i}"), c);
    }
    let mut actions: Vec<Vec<(u32, ScenarioAction)>> = sc.clients.clone();
    for a in &mut actions {
        a.sort_by_key(|a| a.0);
    }
    let mut next = vec![0usize; clients.len()];
    for t in 0..sc.ms / TICK_MS {
        let now = t * TICK_MS;
        let after = ((t + 1) * TICK_MS) as u64;
        for (i, c) in clients.iter_mut().enumerate() {
            golden(c).set_tick(c.ticks + 1);
            while next[i] < actions[i].len() && actions[i][next[i]].0 <= now {
                let a = actions[i][next[i]].1.clone();
                act(c, &a);
                next[i] += 1;
            }
        }
        net.set_now(Duration::from_millis(after));
        golden(&server).set_tick(server.ticks + 1);
        step(&mut server, after);
        tag(&mut lines, "srv", &server);
        for (i, c) in clients.iter_mut().enumerate() {
            golden(c).set_tick(c.ticks + 1);
            step(c, after);
            tag(&mut lines, &format!("cl{i}"), c);
        }
    }
    lines.extend(finish(server).into_iter().map(|l| format!("srv {l}")));
    for (i, c) in clients.into_iter().enumerate() {
        lines.extend(finish(c).into_iter().map(|l| format!("cl{i} {l}")));
    }
    Ok(lines)
}

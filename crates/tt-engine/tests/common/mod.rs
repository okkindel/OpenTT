//! Helpers shared by the engine's integration tests (`mod common;` in each).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use tt_engine::{Config, Engine};
use tt_menu::{MenuAction, MenuBackend};
use tt_net::MemNet;

/// The asset directory: all game data.
pub fn assets() -> PathBuf {
    tt_assets::layout::default_dir()
}

/// The game executable built for these tests.
pub fn game_exe() -> &'static str {
    env!("CARGO_BIN_EXE_opentt")
}

/// Runs `f` on a thread with a 256 MB stack (the engine's deep call chains
/// in debug builds) and waits for it.
pub fn big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap();
}

/// A fresh data directory for engine `tag` of this test binary.
fn data_dir(tag: &str) -> PathBuf {
    let data = std::env::temp_dir().join(format!("tt-{}-{tag}-{}", env!("CARGO_CRATE_NAME"), std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    data
}

/// An engine on the wall clock with command-line `args`, not booted.
pub fn engine_wall_clock(root: &Path, tag: &str, args: &[&str]) -> Engine {
    let cfg = Config { assets: root.to_path_buf(), data: data_dir(tag), args: args.iter().map(|s| s.to_string()).collect(), audio_output: false, echo_log: false, windowed: true };
    Engine::new(&cfg).unwrap()
}

/// An engine on a virtual clock starting at 0 (advanced by [`tick`]), not booted.
pub fn engine(root: &Path, tag: &str, args: &[&str]) -> Engine {
    let mut e = engine_wall_clock(root, tag, args);
    e.host.virtual_clock_ms = Some(0.0);
    e
}

/// A booted virtual-clock engine with the player "Tester" selected.
pub fn client_engine(root: &Path, tag: &str) -> Engine {
    let mut c = engine(root, tag, &[]);
    assert!(c.boot());
    new_player(&mut c, "Tester");
    c
}

#[allow(unused_imports)]
pub use tt_engine::netgame::mem_factory;

/// One tick of a virtual-clock engine (the clock advances by a tick first).
pub fn tick(e: &mut Engine) {
    let t = e.host.virtual_clock_ms.unwrap_or(0.0) + tt_engine::TICK_MS as f64;
    e.host.virtual_clock_ms = Some(t);
    e.tick();
}

/// One virtual tick of `engines` on `net`.
pub fn step_all(net: &MemNet, t: &mut u64, engines: &mut [&mut Engine]) {
    *t += 32;
    net.set_now(Duration::from_millis(*t));
    for e in engines.iter_mut() {
        e.host.virtual_clock_ms = Some(*t as f64);
        e.tick();
    }
}

/// Creates and selects a player (the menu's new-player dialog).
pub fn new_player(e: &mut Engine, name: &str) {
    backend_do(e, |b| b.create_player(name).unwrap());
}

/// Runs `f` on the menu backend.
pub fn backend_do<R>(e: &mut Engine, f: impl FnOnce(&mut tt_engine::menu::Backend) -> R) -> R {
    let Engine { host, menu, .. } = e;
    let m = menu.as_mut().expect("menu");
    let mut b = tt_engine::menu::Backend::new(host, &mut m.data);
    f(&mut b)
}

/// A menu action, as the menu applies it.
pub fn apply(e: &mut Engine, a: MenuAction) {
    let Engine { host, menu, .. } = e;
    menu.as_mut().expect("menu").apply(a, host);
}

/// The canvas content control's name.
pub fn content(e: &Engine) -> String {
    e.host.canvas.content_name().to_string()
}

pub fn log_has(e: &Engine, s: &str) -> bool {
    e.host.log.history.iter().any(|(_, m)| m.contains(s))
}

/// The player's tank in the world: (control object, tank), None while
/// there is none (or it is a corpse).
pub fn player(e: &Engine) -> Option<(tt_gameplay::EntityId, tt_game::TankId)> {
    let o = e.control_object()?;
    let g = e.host.game.as_ref()?;
    let id = g.tank_of(o)?;
    g.world.tank(id).filter(|t| !t.deleted && !t.is_corpse)?;
    Some((o, id))
}

/// Runs `step` (one tick) until the player's tank is in the world,
/// pressing SPACE (respawn, like a player) every 60 ticks; panics when it
/// has not spawned after 3000 ticks.
pub fn until_spawned(e: &mut Engine, mut step: impl FnMut(&mut Engine)) {
    for i in 0..3000 {
        step(e);
        if i % 60 == 0 {
            e.key("space", true, None);
            e.key("space", false, None);
        }
        if player(e).is_some() {
            return;
        }
    }
    panic!("the player did not spawn");
}

/// The world's trigger (power-up, pad) named `name`.
pub fn trigger<'a>(e: &'a Engine, name: &str) -> &'a tt_game::Trigger {
    let g = e.host.game.as_ref().unwrap();
    g.world.triggers.iter().find(|t| t.name.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("no trigger {name}"))
}

/// A camera at `pos` looking at `at`.
pub fn look(pos: glam::Vec3, at: glam::Vec3) -> tt_render::Camera {
    let d = (at - pos).normalize();
    tt_render::Camera { pos, yaw: d.x.atan2(d.y), pitch: d.z.asin() }
}

/// The last 80 log lines to stderr, prefixed with `who`.
pub fn dump_log(e: &Engine, who: &str) {
    for (_, m) in e.host.log.history.iter().rev().take(80).collect::<Vec<_>>().into_iter().rev() {
        eprintln!("{who}: {m}");
    }
}

/// The server's remote connections.
pub fn remote_clients(s: &Engine) -> Vec<tt_engine::net::Conn> {
    tt_engine::netgame::remote_conns(&s.host)
}

/// The connection error the client shows (its message box), "" if none.
pub fn conn_err(e: &Engine) -> String {
    e.host.shell.message.as_ref().map(|m| m.text.clone()).unwrap_or_default()
}

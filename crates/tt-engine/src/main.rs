//! `opentt`: the game executable.
//!
//! ```text
//! opentt [--assets DIR] [--data DIR] [--windowed] [--no-sound] [--quiet]
//!            [--mission MISSION] [--dedicated [--port N]] [--connect HOST[:PORT]]
//!            [game args...] [-- game args...]
//! ```
//!
//! * default: boots and opens the window with the menus (tt-menu on
//!   tt-shell); all game data comes from the asset directory (`--assets`,
//!   default `$TT_ASSETS`, else an imported `gamedata/`, else `./assets`,
//!   else the repository's `assets/`: `tt_assets::layout::default_dir`).
//!   Without the game data there the window shows the first-run screen,
//!   which imports it from the player's ThinkTanks installation
//!   (`tt_engine::launcher`, docs/import.md);
//! * `--import DIR`: that import without a window (DIR: the ThinkTanks
//!   folder; into `--assets DIR` if given, else where the game looks
//!   first, `tt_assets::layout::import_target`), then exits;
//! * `--mission MISSION` (an id, `tt1_1`, or a mission file): developer
//!   shortcut that skips the menus — loads
//!   the server defs and the mission and shows it with a free-fly
//!   camera (WASD/QE, Shift, arrows, right-drag; Escape quits);
//! * `--dedicated [--mission MISSION] [--port N]`: headless dedicated server (the scripts'
//!   `-dedicated -mission`), real-time 32 ms ticks, UDP port N, with a console on stdin
//!   (`status`, `say`, `kick`, `ban`, `map`, `quit`, `set`/`get` for the settings;
//!   `tt_engine::console`).
//! * `--connect HOST[:PORT]`: joins a server at start (the scripts' `-connect`);
//! * `--net-stats`: prints RTT, loss and bytes/s per connection every 5 s;
//! * `--master-server HOST[:PORT][,HOST[:PORT]...]`: list the server on those master
//!   servers; `--port-mapping`: ask the router to forward the port (UPnP / NAT-PMP).
//! * `--master [--port N]`: run a master server instead (as `tt-master`);
//! * `--debug`: debug state in the window title;
//! * `TT_GPU_TIMING=1`: GPU time of each 3D pass (scene, glass copy, SSAO,
//!   bloom, composite) from timestamp queries, averaged every 300 views on
//!   stderr (`[gpu]`; `tt_render::gputime`);
//! * `TT_LOAD_STATS=1`: timings of the mission load path on stderr (menu
//!   action or `createServer` → defs → mission → scene build →
//!   renderer → preload → first game frame; `tt_render::loadstats`).
//!
//! Developer options (cargo feature `dev`, on by default; `src/devtools.rs`):
//!
//! * `--headless [--ticks N]`: no window; runs N sim ticks (default 1875 =
//!   60 s) and prints the canvas/dialog stack and engine statistics;
//! * `--screenshot OUT.png [--frames N] [--size WxH]`: renders offscreen
//!   after N ticks (default 1) instead of opening a window (the canvas at
//!   its resolution, or with `--mission` the 3D view);
//! * `--bench SECONDS`: offscreen frames at 60 Hz, timing every stage;
//! * `--play MISSION[:BOTS]`: the menu's Quick Play on MISSION (`tt3_5`) at the
//!   first tick (`:BOTS`: a free-for-all custom game with that many
//!   bots), for benchmarks and screenshots of a real game, e.g.
//!   `TT_GPU_TIMING=1 opentt --play tt3_5 --bench 20 --size 1920x1080`;
//! * `--cmd TICK:COMMAND` (repeatable): runs a console command at that tick.
//! * `--keys TICK:TEXT` (repeatable): types TEXT at that tick; `{name}`
//!   sends a named key, e.g. `--keys 300:Bait{return}`, `{click:X,Y}` a left
//!   click and `{wheel:X,Y,DY}` a wheel turn (120
//!   per notch) at canvas coordinates; `{down:KEY}` / `{up:KEY}` hold and
//!   release a key; `{axis:x,DX}` a mouse axis motion (to the action maps);
//!   in the window `{mouse:x,RATE}` moves the mouse steadily at RATE counts
//!   per second, sent every frame like a real mouse (`{mouse:x,0}` stops);
//!   `--keys`/`--cmd` also work in the window (unattended runs; the pointer
//!   is then never grabbed), e.g. `--cmd 3000:quit`;
//! * `--frame-stats` (or `TT_FRAME_STATS=1`): per-frame time breakdown in
//!   the window (events, sim ticks, acquire, draw, present), spikes over
//!   20 ms (`TT_FRAME_SPIKE_MS`) and a summary every 5 s on stderr.
//!
//! Unrecognised arguments starting with `-` (e.g. `-windowed`, `-log 2`) are the game's
//! command line options ([`tt_engine::boot::BootArgs`]). Everything the game writes
//! (settings, players, high scores, logs/console.log) goes to the data directory (default
//! `$XDG_DATA_HOME/opentt`; docs/layout.md § User data).
//!
//! A release build for Windows is a GUI program: no console window opens
//! with the game. Started from a console (cmd, PowerShell) it writes its
//! output there; `--dedicated` and `--master` open a console of their own
//! when there is none. A panic or the error that ends the game goes to
//! `logs/crash.log` in the data directory and, in a windowed run, to a
//! message box (`tt_engine::crash`; `TT_NO_ERROR_DIALOG=1` turns it off).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(feature = "dev")]
mod devtools;

#[cfg(feature = "dev")]
#[global_allocator]
static ALLOC: tt_engine::allocstats::CountingAlloc = tt_engine::allocstats::CountingAlloc;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tt_engine::app::App;
use tt_engine::{gamedata, launcher, mission, Config, Engine};

struct Args {
    cfg: Config,
    /// `--assets DIR` or `$TT_ASSETS`: imports go there.
    assets_explicit: bool,
    /// `--import DIR`: import the game data from that install and exit.
    import: Option<PathBuf>,
    mission: Option<PathBuf>,
    /// `--debug`: debug state in the window title.
    debug: bool,
    /// `--dedicated`.
    dedicated: bool,
    /// `--port N`.
    port: Option<u16>,
    /// `--net-stats`.
    net_stats: bool,
    /// `--master`: run a master server instead of the game.
    master: bool,
    /// `--master-server HOST[:PORT]...`: list this server there.
    master_server: Option<String>,
    /// `--port-mapping`: ask the router to forward the port.
    port_mapping: bool,
    #[cfg(feature = "dev")]
    dev: devtools::DevArgs,
}

const USAGE: &str = concat!("usage: ", env!("CARGO_BIN_NAME"), " [--assets DIR] [--import THINK_TANKS_DIR] [--data DIR] [--windowed] [--no-sound] [--quiet] [--mission MISSION] [--debug] [--dedicated [--port N]] [--connect HOST[:PORT]] [--net-stats] [--master-server HOST[:PORT][,...]] [--port-mapping] [--master [--port N]] [game args] [-- game args]");
#[cfg(feature = "dev")]
const DEV_USAGE: &str = "developer options: [--headless [--ticks N]] [--screenshot OUT.png [--frames N] [--size WxH]] [--keys TICK:TEXT] [--cmd TICK:COMMAND] [--play MISSION[:BOTS]] [--bench SECONDS] [--frame-stats]";
#[cfg(not(feature = "dev"))]
const DEV_USAGE: &str = "(developer options need the `dev` feature)";

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        cfg: Config { assets: tt_assets::layout::default_dir(), data: tt_engine::fs::default_data_dir(), args: Vec::new(), audio_output: true, echo_log: true, windowed: false },
        assets_explicit: std::env::var_os("TT_ASSETS").is_some_and(|v| !v.is_empty()),
        import: None,
        mission: None,
        debug: false,
        dedicated: false,
        port: None,
        net_stats: false,
        master: false,
        master_server: None,
        port_mapping: false,
        #[cfg(feature = "dev")]
        dev: devtools::DevArgs { frames: 1, ..Default::default() },
    };
    let num = |v: Option<String>, what: &str| v.and_then(|s| s.parse::<u64>().ok()).ok_or(format!("{what} needs a number"));
    while let Some(arg) = it.next() {
        #[cfg(feature = "dev")]
        if a.dev.parse(&arg, &mut it)? {
            continue;
        }
        match arg.as_str() {
            "--assets" => {
                a.cfg.assets = it.next().ok_or("--assets needs a directory")?.into();
                a.assets_explicit = true;
            }
            "--import" => a.import = Some(it.next().ok_or("--import needs the ThinkTanks folder")?.into()),
            "--data" => a.cfg.data = it.next().ok_or("--data needs a directory")?.into(),
            "--windowed" => a.cfg.windowed = true,
            "--no-sound" => a.cfg.audio_output = false,
            "--quiet" => a.cfg.echo_log = false,
            "--mission" => a.mission = Some(it.next().ok_or("--mission needs a mission (tt1_1)")?.into()),
            "--debug" => a.debug = true,
            "--dedicated" => a.dedicated = true,
            "--port" => a.port = Some(num(it.next(), "--port")?.min(65535) as u16),
            "--connect" => {
                let addr = it.next().ok_or("--connect needs HOST[:PORT]")?;
                a.cfg.args.extend(["-connect".to_string(), addr]);
            }
            "--net-stats" => a.net_stats = true,
            "--master" => a.master = true,
            "--master-server" => a.master_server = Some(it.next().ok_or("--master-server needs HOST[:PORT][,HOST[:PORT]...]")?),
            "--port-mapping" => a.port_mapping = true,
            "-h" | "--help" => return Err(format!("{USAGE}\n{DEV_USAGE}")),
            "--" => a.cfg.args.extend(it.by_ref()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}\n{DEV_USAGE}")),
            _ => a.cfg.args.push(arg),
        }
    }
    if a.dedicated {
        // The mission goes to the scripts (launchDedServer), not the dev viewer.
        a.cfg.args.push("-dedicated".into());
        if let Some(m) = a.mission.take() {
            a.cfg.args.extend(["-mission".to_string(), m.to_string_lossy().into_owned()]);
        }
        a.cfg.audio_output = false;
    }
    #[cfg(feature = "dev")]
    if a.dev.offline() {
        a.cfg.audio_output = false;
    }
    Ok(a)
}

fn run(a: Args) -> Result<(), String> {
    if std::env::var_os("TT_PRINT_ENGINE_THREAD").is_some() {
        // For the startup test (tests/startup.rs).
        eprintln!("engine thread: {}", std::thread::current().name().unwrap_or("?"));
    }
    if a.master {
        // The master (lobby) server alone: no game, no assets.
        let o = tt_master::Options { port: a.port.unwrap_or(tt_net::master::DEFAULT_MASTER_PORT), ..Default::default() };
        return tt_master::run(&o).map_err(|e| format!("master server: {e}"));
    }
    let target = gamedata::target_for(&a.cfg.assets, a.assets_explicit);
    if let Some(source) = &a.import {
        return import(&a, source, &target);
    }
    if windowed_run(&a) {
        tt_engine::update::start();
    }
    if windowed_run(&a) || a.dedicated {
        tt_engine::console::install_stop_handler();
    }
    let a = std::rc::Rc::new(a);
    let factory_args = a.clone();
    let factory: launcher::AppFactory = Box::new(move |assets: &Path| {
        let mut engine = boot_engine(&factory_args, assets)?;
        load_mission(&factory_args, &mut engine, assets)?;
        Ok(window_app_loaded(&factory_args, engine, assets))
    });
    let offline = offline(&a);
    let present = gamedata::present(&a.cfg.assets);
    // Data imported by another version: imported again in the window (the
    // own files are refreshed meanwhile; a run without a window is told how).
    let stale = present && gamedata::stale(&a.cfg.assets);
    if stale {
        gamedata::refresh_ours(&a.cfg.assets);
        if !windowed_run(&a) {
            eprintln!("note: {} was imported by another version; run `{} --import DIR` to refresh it", a.cfg.assets.display(), env!("CARGO_BIN_NAME"));
        }
    }
    // Runs without a window cannot show the import screen: they start on
    // what there is (OpenTT's own maps; the engine needs `missions/`).
    if !present && (a.dedicated || offline) {
        eprintln!("warning: {}", no_game_data(&a.cfg.assets));
    }
    let mut launcher = if present || a.dedicated || offline {
        let mut engine = boot_engine(&a, &a.cfg.assets).map_err(|e| if present { e } else { no_game_data(&a.cfg.assets) })?;
        if a.dedicated {
            return dedicated(&mut engine);
        }
        load_mission(&a, &mut engine, &a.cfg.assets)?;
        #[cfg(feature = "dev")]
        if let Some(r) = devtools::run_offline(&mut engine, &a.dev, &a.cfg.assets, a.mission.as_deref()) {
            return r;
        }
        let mut l = launcher::Launcher::game(window_app_loaded(&a, engine, &a.cfg.assets), target, factory);
        l.refresh = stale;
        l
    } else {
        launcher::Launcher::first_run(a.cfg.assets.clone(), target, factory)
    };
    launcher::run(&mut launcher)?;
    if let Some(app) = launcher.app() {
        if app.stats.enabled {
            eprintln!("[frame] last: {}", app.stats.summary());
        }
        if !app.engine.host.quit_requested {
            app.engine.shutdown();
        }
        // The settings are saved: the rest is the process's teardown.
        tt_engine::crash::exit_started();
    }
    Ok(())
}

/// The message for a run that needs the game data and has none.
fn no_game_data(assets: &Path) -> String {
    format!(
        "{}: no game data. Start the game normally to import it from your ThinkTanks installation, or run `{} --import DIR` (docs/import.md)",
        assets.display(),
        env!("CARGO_BIN_NAME")
    )
}

/// A booted engine on `assets` with the command line's settings.
fn boot_engine(a: &Args, assets: &Path) -> Result<Engine, String> {
    let cfg = Config { assets: assets.to_path_buf(), ..a.cfg.clone() };
    let mut engine = Engine::new(&cfg)?;
    engine.host.debug = a.debug;
    engine.host.netgame.port_override = a.port;
    if a.net_stats {
        engine.host.netgame.stats_every = Some(std::time::Duration::from_secs(5));
    }
    if !engine.boot() {
        return Err("the boot sequence failed (see the console log)".into());
    }
    // Read every tick by the server's master/port-mapping step.
    if let Some(m) = &a.master_server {
        tt_engine::shell::set_pref(&mut engine.host, "net.master_server", m);
    }
    if a.port_mapping {
        tt_engine::shell::set_pref(&mut engine.host, "net.port_mapping", "1");
    }
    Ok(engine)
}

/// Loads `--mission` into a booted `engine`.
fn load_mission(a: &Args, engine: &mut Engine, assets: &Path) -> Result<(), String> {
    if let Some(m) = &a.mission {
        mission::load_dev_mission(engine, &mission::game_relative(assets, m)?)?;
    }
    Ok(())
}

/// The window's game state for a booted `engine` with `--mission` loaded.
fn window_app_loaded(a: &Args, engine: Engine, assets: &Path) -> App {
    #[allow(unused_mut)]
    let mut app = App::new(engine, assets.to_path_buf(), a.mission.is_some());
    #[cfg(feature = "dev")]
    devtools::setup_window(&mut app, &a.dev);
    app
}

/// `--import DIR`: the game's import without a window (progress on
/// stderr), into `target`; records the source in the settings.
fn import(a: &Args, source: &Path, target: &Path) -> Result<(), String> {
    eprintln!("importing {} into {}", source.display(), target.display());
    let mut flow = gamedata::SetupFlow::new(tt_menu::SetupMode::FirstRun, target.to_path_buf(), None);
    flow.log_dir = Some(a.cfg.data.join("logs"));
    let mut last = (String::new(), 0);
    let report = flow.run_blocking(&source.to_string_lossy(), |p| {
        let step = p.done * 20 / p.total.max(1);
        if p.phase != last.0 || step != last.1 {
            eprintln!("  {:>3}%  {}", p.done * 100 / p.total.max(1), p.phase);
            last = (p.phase.clone(), step);
        }
    })?;
    // The report went to stderr and logs/import.log (SetupFlow).
    // Record where it came from (as the import screen does).
    let root = tt_import::locate::find_root(source).unwrap_or_else(|_| source.to_path_buf());
    let quiet = Config { assets: target.to_path_buf(), data: a.cfg.data.clone(), args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
    let mut engine = Engine::new(&quiet)?;
    if engine.boot() {
        gamedata::record_source(&mut engine.host, &root);
        engine.shutdown();
    }
    println!("{} files written to {}", report.written, target.display());
    Ok(())
}

/// `--dedicated`: real-time ticks until `quit()` (at most 5 catch-up ticks;
/// beyond that the stall is logged and the time dropped, §5.2); console
/// lines from stdin run between ticks.
fn dedicated(engine: &mut Engine) -> Result<(), String> {
    use std::time::{Duration, Instant};
    match tt_engine::netgame::server_addr(&engine.host) {
        Some(addr) => println!("dedicated server listening on {addr}"),
        None => return Err("dedicated server: no network server (missing or bad -mission?)".into()),
    }
    println!("console: type `help` for commands");
    let mut console = tt_engine::console::Console::stdin();
    let tick = Duration::from_millis(tt_engine::TICK_MS as u64);
    let mut next = Instant::now();
    while !engine.host.quit_requested {
        console.poll(engine);
        tt_engine::console::take_stop(&mut engine.host);
        if engine.host.quit_requested {
            break;
        }
        let mut n = 0;
        while Instant::now() >= next && n < 5 {
            engine.tick();
            next += tick;
            n += 1;
        }
        if Instant::now() >= next {
            let behind = Instant::now() - next;
            eprintln!("[server] stall: {} ms behind, dropped", behind.as_millis());
            next = Instant::now() + tick;
        }
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    engine.shutdown();
    Ok(())
}

/// Whether the run shows the window (fatal errors then also go to a message
/// box): not a server, an import or a developer run without a window.
fn windowed_run(a: &Args) -> bool {
    !(a.dedicated || a.master || a.import.is_some() || offline(a))
}

/// A developer run without a window (`--headless`, `--screenshot`, `--bench`).
#[cfg_attr(not(feature = "dev"), allow(unused_variables))]
fn offline(a: &Args) -> bool {
    #[cfg(feature = "dev")]
    return a.dev.offline();
    #[cfg(not(feature = "dev"))]
    false
}

/// Whether a fatal error of a windowed run goes to a message box as well:
/// always on Windows (a release build has no console window), elsewhere
/// when stderr is no terminal (started from the desktop) and not from cargo
/// (`cargo run`, the tests).
fn error_dialog() -> bool {
    use std::io::IsTerminal;
    cfg!(windows) || (!std::io::stderr().is_terminal() && std::env::var_os("CARGO_MANIFEST_DIR").is_none())
}

/// The stack of the thread that runs the game: 32 MB. The game needs
/// about 1 MB in a debug build (the 3D view's frame is the deepest; a
/// simulation, a mission load or an import need under 512 KB) and under
/// 256 KB in a release build, so this is a wide margin. On macOS the game
/// runs on the main thread, whose stack is set when the binary is linked
/// (`-stack_size` in build.rs, the same size; at most 64 MB there).
const ENGINE_STACK: usize = 32 << 20;

/// Whether the game runs on the main thread: on macOS, where the window's
/// event loop must be created there (winit has no way around it); elsewhere
/// on a thread of its own with [`ENGINE_STACK`] (the main thread's stack
/// is 1 MB on Windows).
const ENGINE_ON_MAIN_THREAD: bool = cfg!(target_os = "macos");

/// Runs [`run`] where [`ENGINE_ON_MAIN_THREAD`] says; a panic becomes the
/// error that ends the game.
fn run_engine(args: Args) -> Result<(), String> {
    let crashed = |_| match tt_engine::crash::first_panic() {
        Some(p) => format!("the game crashed: {p}"),
        None => "engine thread panicked".to_string(),
    };
    if ENGINE_ON_MAIN_THREAD {
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || run(args))).map_err(crashed).and_then(|r| r);
    }
    std::thread::Builder::new().name("engine".into()).stack_size(ENGINE_STACK).spawn(move || run(args)).map_err(|e| e.to_string()).and_then(|h| h.join().map_err(crashed)).and_then(|r| r)
}

fn main() -> ExitCode {
    let early: Vec<String> = std::env::args().skip(1).take_while(|a| a != "--").collect();
    let has = |flag: &str| early.iter().any(|a| a == flag);
    // Windows release builds have no console window of their own (see the
    // top): the output goes to the console the game was started from, a
    // server without one opens one.
    let console = tt_engine::crash::attach_console(has("--dedicated") || has("--master"));
    tt_engine::crash::install(&tt_engine::fs::default_data_dir());
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if console {
                eprintln!("{e}");
            } else {
                tt_engine::crash::fatal(&e, true);
            }
            return ExitCode::from(2);
        }
    };
    tt_engine::crash::install(&args.cfg.data);
    let dialog = windowed_run(&args) && error_dialog();
    let r = run_engine(args);
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tt_engine::crash::fatal(&e, dialog);
            ExitCode::FAILURE
        }
    }
}

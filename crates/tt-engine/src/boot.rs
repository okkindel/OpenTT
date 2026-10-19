//! 1. The command line.
//! 2. The defaults, then the saved settings and the command-line overrides.
//! 3. `initCommon` (the RNG seed), `initServer`, then `initDedicated` or
//!    [`init_client`]: the canvas, the client's sound data and HUD, the
//!    player's key bindings (`binds.ron`), players and high scores, the
//!    menu ([`crate::shell::init_menu`]).

use crate::host::EngineHost;
use tt_gameplay::client::value::Val;
use tt_shell::VarValue;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BootArgs {
    /// `-log <mode>` (`setLogMode(mode + 4)` when not 0).
    pub log_mode: Option<i32>,
    /// `-demo`: attract mode only.
    pub demo: bool,
    /// `-help`.
    pub help: bool,
    /// `-dedicated`.
    pub dedicated: bool,
    /// `-mission <mission>` (a file or a team rotation, `launchDedServer`).
    pub mission: Option<String>,
    /// `-game B|TB|S|TS`.
    pub game: Option<String>,
    /// `-connect <address>`.
    pub connect: Option<String>,
    /// `-prefs <file>`: settings to apply after the saved ones.
    pub prefs: Vec<String>,
    /// Pref overrides of `-fullscreen`, `-windowed`, `-openGL`, ... in order.
    pub set_prefs: Vec<(&'static str, &'static str)>,
    /// Options that need a value but had none (the game's usage lines).
    pub errors: Vec<String>,
    /// Arguments nobody used.
    pub unused: Vec<String>,
}

const USAGE_LOG: &str = "Error: Missing Command Line argument. Usage: -log <Mode: 0,1,2>";

impl BootArgs {
    /// Parses `args` (without the program name).
    pub fn parse(args: &[String]) -> BootArgs {
        let mut a = BootArgs::default();
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str();
            let next = args.get(i + 1).cloned();
            let value = |a: &mut BootArgs, usage: &str| -> Option<String> {
                if next.is_none() {
                    a.errors.push(usage.to_string());
                }
                next.clone()
            };
            let lower = arg.to_ascii_lowercase();
            let mut took = false;
            match lower.as_str() {
                "-log" => {
                    if let Some(v) = value(&mut a, USAGE_LOG) {
                        let m = tt_game::fmt::atoi(&v);
                        a.log_mode = Some(if m != 0 { m + 4 } else { 0 });
                        took = true;
                    }
                }
                "-demo" => a.demo = true,
                "-test" => {} // read by `Engine::new` (no network port)
                "-help" => a.help = true,
                "-console" => {}
                "-fullscreen" => a.set_prefs.push(("video.full_screen", "1")),
                "-windowed" => a.set_prefs.push(("video.full_screen", "0")),
                "-nosound" => a.errors.push("no support yet".into()),
                "-opengl" => a.set_prefs.push(("video.display_device", "OpenGL")),
                "-voodoo2" => a.set_prefs.push(("video.display_device", "Voodoo2")),
                "-autovideo" => a.set_prefs.push(("video.display_device", "")),
                "-monitorone" => a.set_prefs.push(("video.monitor_num", "1")),
                "-monitortwo" => a.set_prefs.push(("video.monitor_num", "2")),
                "-automonitor" => a.set_prefs.push(("video.monitor_num", "0")),
                "-prefs" => {
                    if let Some(v) = value(&mut a, "Error: Missing Command Line argument. Usage: -prefs <path/settings.ron>") {
                        a.prefs.push(v);
                        took = true;
                    }
                }
                "-dedicated" => a.dedicated = true,
                "-mission" => {
                    if let Some(v) = value(&mut a, "Error: Missing Command Line argument. Usage: -mission <mission_name>") {
                        a.mission = Some(v);
                        took = true;
                    }
                }
                "-game" => {
                    if let Some(v) = value(&mut a, "Error: Missing Command Line argument. Usage: -game <B, TB, S or TS>") {
                        if !["B", "TB", "S", "TS"].iter().any(|g| g.eq_ignore_ascii_case(&v)) {
                            a.errors.push("Error:  -game argument must be B, TB, S, or TS (for Battlemode, Team Battlemode, Scrum, or Team Scrum).".into());
                        }
                        a.game = Some(v);
                        took = true;
                    }
                }
                "-connect" => {
                    if let Some(v) = value(&mut a, "Error: Missing Command Line argument. Usage: -connect <ip_address>") {
                        a.connect = Some(v);
                        took = true;
                    }
                }
                _ => a.unused.push(arg.to_string()),
            }
            i += 1 + took as usize;
        }
        a
    }

    pub fn help_text() -> String {
        format!("{} command line options:\n", tt_assets::product::WINDOW_TITLE)
            + "  -log <logmode>         Logging behavior\n  -help                  Display this help message\n  -fullscreen            Starts game in full screen mode\n  -windowed              Starts game in windowed mode\n  -prefs <file>          Apply a settings file (settings.ron form, or an old prefs file)\n  -connect <address>     Connects to a game at <address>\n  -dedicated             Launch dedicated server\n  -mission <mission>     Load the mission (optionally, supply one of\n                         GREEN, BLUE, or RED to cycle between mission sets)\n  -game                  Use specified rule set.  Values: B=Battlemode,\n                         TB=Team Battlemode, S=Scrum, or TS=Team Scrum)\n"
    }
}

/// The server prefs, as written.
fn server_defaults(h: &mut EngineHost) {
    let p = &mut h.shell.prefs;
    let int = |p: &mut tt_shell::Prefs, n: &str, v: i32| p.declare(n, VarValue::Int(v));
    let text = |p: &mut tt_shell::Prefs, n: &str, v: &str| p.declare(n, VarValue::Str(v.to_string()));
    int(p, "server.region_mask", 2);
    text(p, "server.display_on_master", "Always");
    text(p, "server.name", &format!("{} Server", tt_assets::product::NAME));
    int(p, "server.password_enabled", 0);
    int(p, "server.game_type_index", 0);
    int(p, "server.mission_index", 1);
    int(p, "server.mission_world", 1);
    text(
        p,
        "server.connection_error",
        &format!("You do not have the correct version of {} or the game data needed to connect to this server, please contact the server operator to obtain the latest version of this game.", tt_assets::product::NAME),
    );
    int(p, "server.port", 28000);
    text(p, "server.password", "");
    text(p, "server.admin_password", "");
    int(p, "server.max_players", 10);
    int(p, "server.flood_protection_enabled", 1);
    int(p, "server.max_chat_len", 120);
    int(p, "server.max_bots", 6);
    int(p, "server.replace_bots", 1);
    let bots = [
        ("GoldHeavyTank", "Hum [Bot]", "GreenTeam"),
        ("SilverMediumTank", "Tom [Bot]", "GreenTeam"),
        ("BronzeLightTank", "Mal [Bot]", "GreenTeam"),
        ("GoldLightTank", "Ri [Bot]", "BlueTeam"),
        ("SilverMediumTank", "Gul [Bot]", "BlueTeam"),
        ("BronzeHeavyTank", "Gum [Bot]", "BlueTeam"),
    ];
    for (i, (db, name, team)) in bots.iter().enumerate() {
        text(p, &format!("server.bot.{i}"), db);
        text(p, &format!("server.bot_name.{i}"), name);
        text(p, &format!("server.bot_team.{i}"), team);
    }
}

/// Runs the boot sequence (see the module docs). False when the game does
/// not start (`-help`).
pub fn run(h: &mut EngineHost, args: &[String]) -> bool {
    let a = BootArgs::parse(args);
    h.log.mode = a.log_mode.unwrap_or(0);
    if a.demo {
        h.srv.state_mut().setup.demo_mode = Some(true);
    }
    // The defaults, then the saves.
    crate::gameplay::client::game::client_defaults(h);
    server_defaults(h);
    // OpenTT's defaults for a new install (tt_shell::prefs::PORT_DEFAULTS).
    for (name, v) in tt_shell::prefs::PORT_DEFAULTS {
        crate::shell::set_pref_val(&mut h.shell.prefs, name, crate::shell::literal(v));
    }
    crate::shell::load_settings(h);
    let res = h.shell.prefs.get("video.shell_resolution");
    crate::shell::set_pref_val(&mut h.shell.prefs, "video.resolution", Val::s(res));
    // parseArgs.
    for (name, v) in &a.set_prefs {
        crate::shell::set_pref_val(&mut h.shell.prefs, name, Val::s(*v));
    }
    for f in &a.prefs {
        crate::shell::apply_settings_file(h, f);
    }
    if a.dedicated {
        h.srv.state_mut().setup.dedicated = true;
    }
    if let Some(m) = &a.mission {
        // A mission as its file; `GREEN`, `BLUE`, `RED`, `ALL`
        // (rotations) as given.
        let rotation = ["GREEN", "BLUE", "RED", "ALL"].iter().any(|k| k.eq_ignore_ascii_case(m));
        h.cl.mission_arg = if rotation { m.clone() } else { crate::mission::game_relative(h.fs.root(), std::path::Path::new(m)).unwrap_or_else(|_| m.clone()) };
    }
    if let Some(g) = &a.game {
        h.srv.state_mut().setup.dedicated_mode = tt_gameplay::server::state::DedicatedMode::parse(g);
    }
    if let Some(c) = &a.connect {
        h.cl.join_address = c.clone();
    }
    for e in &a.errors {
        h.log.error(e);
    }
    let started = if a.help {
        h.log.error(&BootArgs::help_text());
        false
    } else {
        // onStart: initCommon, then the ThinkTanks mod.
        h.log.info("--------- Initializing MOD: Common ---------");
        // `setRandomSeed()`: from the real-time clock.
        let seed = h.now_ms_f64() as u32 as i32;
        h.rng.set_seed(seed);
        h.log.info(&format!("--------- Initializing MOD: {} ---------", tt_assets::product::WINDOW_TITLE));
        init_server(h);
        if a.dedicated {
            init_dedicated(h);
        } else {
            init_client(h);
        }
        true
    };
    for u in &a.unused {
        h.log.error(&format!("Error: Unkown command line argument: {u}"));
    }
    started
}

/// The server's start-up (its setup's defaults are
/// [`tt_gameplay::server::GameSetup::default`]).
fn init_server(h: &mut EngineHost) {
    h.log.info(&format!("\n--------- Initializing {}: Server ---------", tt_assets::product::WINDOW_TITLE));
}

/// `initDedicated`: the dedicated server on `-mission`.
fn init_dedicated(h: &mut EngineHost) {
    h.log.info("\n--------- Starting Dedicated Server ---------");
    h.srv.state_mut().setup.dedicated = true;
    let m = h.cl.mission_arg.clone();
    if !m.is_empty() {
        crate::gameplay::server_host::with_server(h, |sv, b| sv.launch_dedicated(b, &m));
    } else {
        h.log.info("No mission specified (use -mission filename)");
    }
}

/// `initClient`: the canvas, the client's sound data and HUD, the
/// bindings, the player's saves, the menu.
pub fn init_client(h: &mut EngineHost) {
    h.log.info(&format!("--------- Initializing {}: Client ---------", tt_assets::product::NAME));
    h.srv.state_mut().setup.dedicated = false;
    init_canvas(h, tt_assets::product::WINDOW_TITLE);
    crate::gameplay::client::game::declare_sounds(h);
    // PlayGui.gui, ChatHud.gui, playerList.gui, endGameGui.gui: the HUD's screens.
    h.cl.hud = true;
    crate::gameplay::client::game::declare_music(h);
    crate::gameplay::client::game::init_binds(h);
    crate::shell::load_players(h);
    if !h.srv.state().setup.skip_port {
        crate::netgame::set_net_port(h, 0);
    }
    crate::shell::init_menu(h);
    let addr = h.cl.join_address.clone();
    if !addr.is_empty() {
        let name = h.shell.prefs.player_name();
        crate::net::connect_to(h, &addr, "", &name, "");
    }
}

/// `initCanvas(title)`: the window, the cursor, the sound driver.
fn init_canvas(h: &mut EngineHost, title: &str) {
    let res = h.shell.prefs.get("video.shell_resolution");
    crate::shell::set_pref_val(&mut h.shell.prefs, "video.resolution", Val::s(res));
    crate::video::create_canvas(h, title);
    h.cl.screenshot_number = 0.0;
    h.cl.cursor_controlled = true;
    openal_init(h);
}

/// `OpenALInit`: the driver, the master and channel volumes.
fn openal_init(h: &mut EngineHost) {
    h.audio.shutdown_driver();
    h.log.info("");
    h.log.info("OpenAL Driver Init:");
    let driver = h.shell.prefs.get("audio.driver");
    h.log.info(&driver);
    if driver.eq_ignore_ascii_case("OpenAL") && !h.audio.init_driver() {
        h.log.error("   Failed to initialize driver.");
    }
    for (label, key) in [("   Vendor: ", "AL_VENDOR"), ("   Version: ", "AL_VERSION"), ("   Renderer: ", "AL_RENDERER"), ("   Extensions: ", "AL_EXTENSIONS")] {
        let v = h.audio.al_string(key);
        h.log.info(&format!("{label}{v}"));
    }
    let vol = tt_game::fmt::atof(&h.shell.prefs.get("audio.master_volume")) as f32;
    h.audio.listener_f(tt_audio::AlParam::GainLinear, vol);
    for channel in 1..=8u32 {
        let v = tt_game::fmt::atof(&h.shell.prefs.get(&tt_shell::prefs::channel_volume_key(channel))) as f32;
        h.audio.set_channel_volume(channel, v);
    }
    h.log.info("");
}

pub fn shutdown(h: &mut EngineHost) {
    h.log.info("Saving settings");
    crate::shell::save_prefs(h);
    crate::netgame::admin::export(h, crate::netgame::admin::BAN_FILE);
    h.audio.shutdown_driver();
    crate::gameplay::client::connection::disconnect(h);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn command_line() {
        let a = BootArgs::parse(&args("-dedicated -mission GREEN -game TS -windowed -log 1 -bogus"));
        assert!(a.dedicated);
        assert_eq!(a.mission.as_deref(), Some("GREEN"));
        assert_eq!(a.game.as_deref(), Some("TS"));
        assert_eq!(a.set_prefs, vec![("video.full_screen", "0")]);
        assert_eq!(a.log_mode, Some(5));
        assert_eq!(a.unused, vec!["-bogus".to_string()]);
        assert!(a.errors.is_empty());
        let b = BootArgs::parse(&args("-connect"));
        assert_eq!(b.errors.len(), 1);
        assert!(b.connect.is_none());
        assert!(BootArgs::parse(&args("-game X")).errors[0].contains("must be B, TB, S, or TS"));
    }
}

//! The native shell of the menus: crate `tt-shell` (players, high scores,
//! settings, the solo campaign, the map catalog).
//!
//! * **Saves** are RON files in the data directory (plan D3,
//!   [`tt_shell::save`]): `players.ron`, `hiscores.ron`, `progress.ron`,
//!   `settings.ron` (the key bindings, `binds.ron`, are
//!   [`crate::gameplay::client::game::load_binds`]'s).
//! * **Prefs** are [`Shell::prefs`] ([`Prefs`], typed accessors; saved as
//!   [`tt_shell::Settings`]).
//! * The **boot** sequence is [`crate::boot`]; it calls [`load_settings`],
//!   [`load_players`] and [`init_menu`].
//! * The **game control** the menu uses ([`start_target_practice`],
//!   [`start_solo_level`], [`launch_local`], [`host_game`], [`join`],
//!   [`leave_game`], ...) calls the server (`crate::gameplay::server_host`)
//!   and the client (`crate::gameplay::client`).
//! * The **screens** the game flow puts on the canvas (the loading screen,
//!   the solo screen, the main menu, ...; [`show`]) and the requests the
//!   game leaves for the menu (pause, help, game over, message boxes) are
//!   here; the menu ([`crate::menu`]) follows them.

use crate::canvas::{self, Ctrl};
use crate::host::EngineHost;
use crate::log::LogLevel;
use tt_gameplay::client::value::Val;
use tt_gameplay::server::{CampaignStep, MissionType, ServerType};
use tt_shell::save::{self, SaveStore};
use tt_shell::{HiScores, LevelTable, Players, Prefs, SoloRun, Tally, VarValue};

/// What answering a message box does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgAction {
    /// The game runs again.
    Resume,
    /// Leave the game, then run again.
    DisconnectResume,
}

/// A message box of the game (`ttMessageWindow`, `connErrorGui`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShellMessage {
    pub text: String,
    pub kind: MessageKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageKind {
    /// One button.
    Ok(MsgAction),
    /// Yes / No.
    YesNo(MsgAction, MsgAction),
    /// A connection error: OK cleans up after the disconnect (`disconnectedCleanup()`).
    ConnError,
}

/// Mission loading progress.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadingInfo {
    pub map: String,
    pub phase: String,
    pub progress: f32,
}

/// tt-shell state of the running game.
pub struct Shell {
    pub players: Players,
    pub hiscores: HiScores,
    /// Level-select progress (`progress.ron`).
    pub progress: tt_shell::Progress,
    pub prefs: Prefs,
    levels: Option<LevelTable>,
    /// The solo run (between levels and during play).
    pub run: Option<SoloRun>,
    /// Tally of the last finished level.
    pub last_tally: Option<Tally>,
    /// The rank in the top 10 of the last game over, for the high-score screen.
    pub last_rank: Option<usize>,
    /// A game over is waiting to be shown (cleared by the menu).
    pub game_over: bool,
    pub message: Option<ShellMessage>,
    pub loading: LoadingInfo,
    /// The pause menu was asked for (Escape / F1 in game).
    pub pause_request: bool,
    /// F1 in the menus: open the help.
    pub help_request: bool,
    /// The menu's screens exist ([`init_menu`]; none on a dedicated server).
    pub screens: bool,
    pub rng: tt_game::rng::Rng,
    /// Mission display names, by id.
    names: std::collections::HashMap<String, String>,
    /// The map list ([`missions`]), discovered once.
    missions: Option<Vec<tt_shell::MissionInfo>>,
}

impl Shell {
    pub fn new() -> Shell {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u32).unwrap_or(1);
        Shell {
            players: Players::new(),
            hiscores: HiScores::default(),
            progress: Default::default(),
            prefs: Prefs::default(),
            levels: None,
            run: None,
            last_tally: None,
            last_rank: None,
            game_over: false,
            message: None,
            loading: LoadingInfo::default(),
            pause_request: false,
            help_request: false,
            screens: false,
            rng: tt_game::rng::Rng::new(seed | 1),
            names: Default::default(),
            missions: None,
        }
    }
}

impl Default for Shell {
    fn default() -> Self {
        Shell::new()
    }
}

/// The saves in the data directory (never the asset directory).
struct DataStore<'a>(&'a mut crate::fs::GameFs);

impl SaveStore for DataStore<'_> {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        self.0.read_data(rel)
    }
    fn write(&mut self, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.0.write(rel, bytes, false).map(|_| ())
    }
}

fn write_save(h: &mut EngineHost, file: &str, text: &str) {
    if let Err(e) = h.fs.write(file, text.as_bytes(), false) {
        h.log.log(LogLevel::Error, &format!("saves: write {file}: {e}"));
    }
}

fn log_report(h: &mut EngineHost, r: &tt_shell::LoadReport) {
    for l in &r.imported {
        h.log.info(&format!("saves: imported {} into {} (the old file stays as a backup)", l.0.join(" + "), l.1));
    }
    for e in &r.errors {
        h.log.warn(&format!("saves: {e}"));
    }
}

// ---- prefs ---------------------------------------------------------------------------

/// A stored console value as the rules read it.
pub fn val_of(v: &VarValue) -> Val {
    match v {
        VarValue::Int(i) => Val::Int(*i as u32),
        VarValue::Float(f) => Val::Float(*f),
        VarValue::Str(s) => Val::Str(s.clone()),
    }
}

/// A rule's value as a stored console value.
pub fn var_of(v: &Val) -> VarValue {
    match v {
        Val::Int(i) => VarValue::Int(*i as i32),
        Val::Float(f) => VarValue::Float(*f),
        Val::Str(s) => VarValue::Str(s.clone()),
    }
}

/// A pref as the rules read it (None: unset).
pub fn pref_val(p: &Prefs, name: &str) -> Option<Val> {
    p.value(name).map(val_of)
}

pub fn set_pref_val(p: &mut Prefs, name: &str, v: Val) {
    p.declare(name, var_of(&v));
}

/// Sets a pref (a run-time change).
pub fn set_pref(h: &mut EngineHost, name: &str, value: &str) {
    h.shell.prefs.set(name, value);
}

/// As the defaults scripts write it: an integer literal as an integer,
/// another number as a float, anything else as text.
pub fn literal(value: &str) -> Val {
    if value.parse::<f64>().is_ok() {
        match value.parse::<u32>() {
            Ok(i) => Val::Int(i),
            Err(_) => Val::f(tt_game::fmt::atof(value)),
        }
    } else {
        Val::s(value)
    }
}

/// `settings.ron` over the defaults.
pub fn load_settings(h: &mut EngineHost) {
    let mut r = tt_shell::LoadReport::default();
    let s = save::load_settings(&mut DataStore(&mut h.fs), &mut r);
    log_report(h, &r);
    if let Some(s) = s {
        h.shell.prefs.merge(&s.to_vars());
    }
}

/// `-prefs <file>`: a settings file.
pub fn apply_settings_file(h: &mut EngineHost, file: &str) {
    let Some(text) = h.fs.read(file).or_else(|| std::fs::read(file).ok()).map(|b| tt_assets::latin1(&b)) else {
        h.log.log(LogLevel::Error, &format!("Missing file: {file}"));
        return;
    };
    let vars = match tt_shell::Settings::from_ron(&text) {
        Ok(s) => s.to_vars(),
        Err(_) => tt_shell::legacy::parse(&text),
    };
    h.shell.prefs.merge(&vars);
}

/// Writes `settings.ron`.
pub fn save_prefs(h: &mut EngineHost) {
    let text = h.shell.prefs.settings().to_ron();
    write_save(h, save::SETTINGS, &text);
}

// ---- players, high scores, progress, bindings -----------------------------------

/// The players, their level-select progress and the high scores.
pub fn load_players(h: &mut EngineHost) {
    let mut r = tt_shell::LoadReport::default();
    let (players, progress) = save::load_players(&mut DataStore(&mut h.fs), &mut r);
    let hiscores = save::load_hiscores(&mut DataStore(&mut h.fs), &mut r);
    log_report(h, &r);
    h.shell.players = players;
    h.shell.progress = progress;
    h.shell.hiscores = hiscores;
}

/// Writes `players.ron`.
pub fn save_players(h: &mut EngineHost) {
    let text = save::save_players_text(&h.shell.players);
    write_save(h, save::PLAYERS, &text);
}

/// Writes `hiscores.ron`.
pub fn save_hiscores(h: &mut EngineHost) {
    let text = save::save_hiscores_text(&h.shell.hiscores);
    write_save(h, save::HISCORES, &text);
}

/// Writes `progress.ron`.
pub fn save_progress(h: &mut EngineHost) {
    let text = save::save_progress_text(&h.shell.progress);
    write_save(h, save::PROGRESS, &text);
}

/// The SPLevels table (`data/campaign.ron`), built once.
pub fn levels(h: &mut EngineHost) -> Option<LevelTable> {
    if let Some(l) = h.shell.levels.clone() {
        return Some(l);
    }
    let table = crate::gameplay::defs::splevels_table(h.fs.root());
    if table.levels.is_empty() {
        h.log.warn("shell: no solo levels");
        return None;
    }
    h.shell.levels = Some(table.clone());
    Some(table)
}

/// The solo level table, if loaded ([`levels`] loads it).
pub fn level_table(h: &EngineHost) -> Option<&LevelTable> {
    h.shell.levels.as_ref()
}

/// The map list of the menus and Quick Play: every `missions/*.ron` of the
/// game data and the enabled mods, ordered and placed in worlds by the
/// `missions/catalog.ron` files ([`tt_shell::catalog::build`]). Discovered
/// on first use.
pub fn missions(h: &mut EngineHost) -> &[tt_shell::MissionInfo] {
    if h.shell.missions.is_none() {
        let list = discover_missions(h);
        h.shell.missions = Some(list);
    }
    h.shell.missions.as_deref().unwrap_or_default()
}

fn discover_missions(h: &mut EngineHost) -> Vec<tt_shell::MissionInfo> {
    use tt_shell::catalog::{CatalogFile, CATALOG};
    let mut catalogs = Vec::new();
    for bytes in h.fs.read_layers(CATALOG) {
        match CatalogFile::parse(&String::from_utf8_lossy(&bytes)) {
            Ok(c) => catalogs.push(c),
            Err(e) => h.log.warn(&format!("{CATALOG}: {e}")),
        }
    }
    let hidden: Vec<String> = catalogs.iter().flat_map(|c| c.hidden.iter().cloned()).collect();
    let mut found = Vec::new();
    for f in h.fs.find_files(&format!("{}/*.ron", tt_assets::layout::MISSIONS)) {
        let id = tt_assets::layout::mission_id(&f);
        // Not a mission; the hidden ones need not be read.
        if f.eq_ignore_ascii_case(CATALOG) || f.matches('/').count() != 1 || hidden.iter().any(|x| x.eq_ignore_ascii_case(&id)) {
            continue;
        }
        match crate::mission::mission_meta(h, &f) {
            Some(m) => found.push(m),
            None => h.log.warn(&format!("{f}: not a readable mission file; left out of the map list")),
        }
    }
    tt_shell::catalog::build(&catalogs, &found)
}

/// Display name of a mission id ("tt2_3" → "Greymatter Gorge"): the
/// catalog, else the mission's `MissionInfo` name, else the id.
pub fn mission_name(h: &mut EngineHost, id: &str) -> String {
    if let Some(n) = h.shell.names.get(id) {
        return n.clone();
    }
    let n = match missions(h).iter().find(|m| m.id.eq_ignore_ascii_case(id)) {
        Some(m) => m.name.clone(),
        None => {
            let info = crate::mission::mission_info_name(h, &tt_assets::layout::mission(id));
            if info.is_empty() {
                id.to_string()
            } else {
                info
            }
        }
    };
    h.shell.names.insert(id.to_string(), n.clone());
    n
}

// ---- the solo run and the local client's record ----------------------------------

/// A solo run is being played or is between levels (connected to the solo server).
pub fn in_run(h: &EngineHost) -> bool {
    h.shell.run.is_some() && h.net.server_connection().is_some() && h.net.server.is_some_and(|s| h.net.alive.contains(&s)) && h.srv.get().is_some_and(|s| s.setup.single_player())
}

/// The counters the server keeps for the local client and the level's
/// position into the run.
fn pull_run(run: &mut SoloRun, h: &mut EngineHost) {
    if crate::gameplay::server_host::local_client(h).is_none() {
        return;
    }
    let r = crate::gameplay::server_host::local_record(h).unwrap_or_default();
    run.lives = r.lives.unwrap_or(0);
    run.bronze_kills = r.bronze_kills.unwrap_or(0);
    run.silver_kills = r.silver_kills.unwrap_or(0);
    run.gold_kills = r.gold_kills.unwrap_or(0);
    run.brains = r.brains.unwrap_or(0);
    run.max_brains = r.max_brains.unwrap_or(0);
    run.deaths = r.deaths.unwrap_or(0);
    run.boss_tank_killed = r.boss_tank_killed.unwrap_or(0);
    run.score = r.spscore.unwrap_or(0.0);
    let c = &h.srv.state().campaign;
    run.mission_seq = c.mission_seq.unwrap_or(0);
    run.mission_seq_total = c.mission_seq_total.unwrap_or(0);
}

/// The run's counters back onto the local client's record.
fn push_run(run: &SoloRun, h: &mut EngineHost) {
    crate::gameplay::server_host::with_local_record(h, |r| {
        r.lives = Some(run.lives);
        r.spscore = Some(run.score);
        r.bronze_kills = Some(run.bronze_kills);
        r.silver_kills = Some(run.silver_kills);
        r.gold_kills = Some(run.gold_kills);
        r.brains = Some(run.brains);
        r.max_brains = Some(run.max_brains);
        r.deaths = Some(run.deaths);
        r.boss_tank_killed = Some(run.boss_tank_killed);
    });
}

/// Starts a new run (before [`start_solo_level`]).
pub fn new_run(h: &mut EngineHost) -> bool {
    new_run_at(h, 1)
}

/// Level select: a new run whose first level is campaign position
/// `position` ([`SoloRun::starting_at`]; 1 = [`new_run`]). Then
/// [`start_solo_level`] plays it.
pub fn new_run_at(h: &mut EngineHost, position: u32) -> bool {
    let Some(table) = levels(h) else { return false };
    let Some(run) = SoloRun::starting_at(table, position) else { return false };
    let sh = &mut h.shell;
    sh.run = Some(run);
    sh.last_tally = None;
    sh.last_rank = None;
    sh.game_over = false;
    true
}

/// Level select, after [`start_solo_level`]: the server set the
/// Tanks to the first level's scales; the run's level gets its own
/// (bumped) scales, as a played-through run gives them.
pub fn apply_level_scales(h: &mut EngineHost) {
    let Some(l) = h.shell.run.as_ref().and_then(|r| r.current_level().cloned()) else { return };
    let l = crate::gameplay::server_host::solo_level(&l);
    crate::gameplay::server_host::with_server(h, |s, b| s.apply_solo_scales(b, l.damage_scale, l.armor_scale, l.fire_scale));
}

/// The solo screen's lives on the in-game HUD: the lives of the local
/// client's record, or 3 before the run's first level.
pub fn show_lives_hud(h: &mut EngineHost) {
    let lives = match crate::gameplay::server_host::local_client(h) {
        Some(_) => crate::gameplay::server_host::local_record(h).and_then(|r| r.lives).map(|l| l.to_string()).unwrap_or_default(),
        None => "3".to_string(),
    };
    if h.cl.hud {
        h.client.solo.show_lives(&lives);
    }
}

/// Before the next level: the tally is complete (done at the level end),
/// the last kill time and the deaths reset.
pub fn prepare_next_level(h: &mut EngineHost) {
    let sh = &mut h.shell;
    let Some(run) = sh.run.as_mut() else { return };
    run.prepare_launch();
    sh.last_tally = None;
    crate::gameplay::server_host::with_local_record(h, |r| {
        r.last_kill_time = Some(0);
        r.deaths = Some(0);
    });
}

// ---- game control (the menu's actions) ------------------------------------------

/// The game setup, to change before a launch.
fn setup(h: &mut EngineHost) -> &mut tt_gameplay::server::GameSetup {
    &mut h.srv.state_mut().setup
}

/// The round length back to `secs` (custom games change it).
pub fn restore_duration(h: &mut EngineHost, secs: f32) {
    setup(h).round_secs = secs;
}

/// The tutorial (target practice).
pub fn start_target_practice(h: &mut EngineHost) {
    setup(h).single_player = Some(false);
    h.cl.disconnect_gui = Some(Ctrl::LoadPlayerGui);
    show(h, Ctrl::LoadingGui, "");
    crate::gameplay::server_host::launch_target_practice(h);
}

/// The run's next level on a new solo server (the level comes from the
/// run through [`next_sp_mission`]).
pub fn start_solo_level(h: &mut EngineHost) {
    setup(h).single_player = Some(false);
    h.cl.disconnect_gui = Some(Ctrl::TankSelectionGui);
    show(h, Ctrl::LoadingGui, "");
    crate::gameplay::server_host::launch_campaign(h);
}

/// The next level during a run (the solo screen's launch button).
pub fn continue_solo_run(h: &mut EngineHost) {
    prepare_next_level(h);
    show_lives_hud(h);
    h.cl.disconnect_gui = Some(Ctrl::TankSelectionGui);
    show(h, Ctrl::LoadingGui, "");
    crate::gameplay::server_host::with_server(h, |s, b| s.next_round(b));
}

/// Quick Play on `mission`: the default bots, free for all, a local game.
pub fn quick_play(h: &mut EngineHost, mission: &str) {
    let s = h.srv.state_mut();
    s.default_bots();
    s.setup.mission_type = MissionType::Deathmatch;
    s.setup.team_game = false;
    launch_local(h, mission);
}

/// A local game on `mission`: a single-player server and the local connection
/// with the player's name, team and tank.
pub fn launch_local(h: &mut EngineHost, mission: &str) {
    let s = setup(h);
    s.target_range = Some(false);
    s.single_player = Some(false);
    s.stay_on_mission = true;
    h.cl.disconnect_gui = Some(Ctrl::TankSelectionGui);
    show(h, Ctrl::LoadingGui, "");
    crate::gameplay::server_host::with_server(h, |s, b| s.create_server(b, ServerType::SinglePlayer, mission));
    let p = &h.shell.prefs;
    let args = vec![p.player_name(), p.team(), p.get("player.tank")];
    crate::net::connect_local(h, args);
    setup(h).ai_control = false;
}

/// A listen server with the set-up mode; the local player joins through
/// the local connection.
pub fn host_game(h: &mut EngineHost, mission: &str) {
    let s = setup(h);
    s.target_range = Some(false);
    s.single_player = Some(false);
    let (ty, teams) = (s.mission_type, s.team_game);
    h.cl.disconnect_gui = Some(Ctrl::JoinGameGui);
    show(h, Ctrl::LoadingGui, "");
    crate::gameplay::server_host::launch_listen(h, mission, ty, teams);
    setup(h).ai_control = false;
}

/// `team` is the connect argument ("Auto" lets the server pick).
pub fn join(h: &mut EngineHost, address: &str, password: &str, team: &str) {
    h.cl.password = password.to_string();
    h.cl.time_scale = Some(Val::Int(1));
    h.cl.disconnect_gui = Some(Ctrl::JoinGameGui);
    show(h, Ctrl::LoadingGui, "");
    let name = h.shell.prefs.player_name();
    crate::net::connect_to(h, address, password, &name, team);
}

/// Back to the game from the pause menu (`quitDlg.hide()`).
pub fn resume(h: &mut EngineHost) {
    quit_hide(h);
    h.cl.time_scale = Some(Val::Int(1));
}

/// Leaves the game.
pub fn leave_game(h: &mut EngineHost) {
    h.cl.time_scale = Some(Val::Int(1));
    if h.cl.disconnect_gui.is_none() {
        h.cl.disconnect_gui = Some(Ctrl::TankSelectionGui);
    }
    crate::gameplay::client::connection::disconnect(h);
}

/// The game-over screen's OK (`soloEndOk`): disconnect, to the tank selection.
pub fn end_solo_game(h: &mut EngineHost) {
    h.shell.run = None;
    h.cl.time_scale = Some(Val::Int(1));
    h.cl.disconnect_gui = Some(Ctrl::TankSelectionGui);
    crate::gameplay::client::connection::disconnect(h);
}

/// The accepted player: on to the tank selection (`loadPlayerGui.hide();
/// tankSelectionGui.show()`; the team is cleared).
pub fn show_tank_selection(h: &mut EngineHost) {
    set_pref(h, "player.team", "");
    show(h, Ctrl::TankSelectionGui, "");
}

/// Quit (`ttQuit()`: the game runs again, then the engine quits).
pub fn quit(h: &mut EngineHost) {
    h.cl.time_scale = Some(Val::Int(1));
    h.quit_requested = true;
}

/// The engine side of Options → Apply (optionsGui::apply plus the
/// engine's master volume and vsync), from [`Shell::prefs`].
pub fn apply_engine_settings(h: &mut EngineHost) {
    let p = h.shell.prefs.clone();
    crate::gameplay::client::game::set_detail_level(h, &p.get("video.detail_level"));
    let num = |n: &str| tt_game::fmt::atof(&p.get(n)) as f32;
    h.audio.set_channel_volume(4, num("audio.music_volume"));
    h.audio.set_channel_volume(2, num("audio.sound_volume"));
    h.audio.listener_f(tt_audio::AlParam::GainLinear, num("audio.master_volume"));
    h.video.vsync = p.vsync();
    h.video.changed = true;
    let (w, hgt) = p.resolution();
    let full = p.window_mode() != tt_shell::WindowMode::Windowed;
    crate::video::set_mode(h, w as i32, hgt as i32, 32, Some(full));
    save_prefs(h);
}

// ---- the menu's screens and the game's requests ------------------------------------

/// `screen.show(arg)`: screen `c` on the canvas (the menu follows). The
/// solo screen runs the level's tally first; the loading screen starts
/// empty; the connection error screen shows `arg`. Nothing on a dedicated
/// server (no screens).
pub fn show(h: &mut EngineHost, c: Ctrl, arg: &str) {
    if !h.shell.screens {
        return;
    }
    match c {
        Ctrl::SoloPlayGui => return solo_show(h),
        Ctrl::LoadingGui => h.shell.loading = LoadingInfo::default(),
        Ctrl::ConnErrorGui => h.shell.message = Some(ShellMessage { text: arg.to_string(), kind: MessageKind::ConnError }),
        _ => {}
    }
    canvas::show(h, c);
}

/// `connErrorGui.show(msg)`.
pub fn show_conn_error(h: &mut EngineHost, msg: &str) {
    show(h, Ctrl::ConnErrorGui, msg);
}

/// The level is over: the run takes the counters, runs the tally (score,
/// extra lives) and hands them back; the menu shows the solo screen.
fn solo_show(h: &mut EngineHost) {
    if in_run(h) {
        let mut run = h.shell.run.take().unwrap();
        pull_run(&mut run, h);
        if run.lives <= 0 {
            h.cl.disconnect_gui = Some(Ctrl::SoloPlayGui);
            crate::gameplay::client::connection::disconnect(h);
            return;
        }
        let tally = run.tally();
        push_run(&run, h);
        h.shell.run = Some(run);
        h.shell.last_tally = tally;
        show_lives_hud(h);
    }
    canvas::show(h, Ctrl::SoloPlayGui);
}

/// `soloPlayGui.endGame()` (the server's game over, no lives left): the
/// player's best and the top 10 are updated and saved; the menu shows the
/// high scores.
pub fn solo_end_game(h: &mut EngineHost) {
    if !h.shell.screens {
        return;
    }
    let name = h.shell.prefs.player_name();
    let Some(mut run) = h.shell.run.take().or_else(|| levels(h).map(SoloRun::new)) else {
        return;
    };
    pull_run(&mut run, h);
    let sh = &mut h.shell;
    let rank = run.end_run(&mut sh.players, &mut sh.hiscores, &name);
    // A mid-campaign start counts; its row shows the start level.
    let table = sh.hiscores.entries().unwrap_or_default();
    sh.progress.record_hiscore(rank.and_then(|r| table.get(r)), run.start_level, &table);
    sh.last_rank = rank;
    sh.game_over = true;
    sh.run = Some(run);
    save_players(h);
    save_hiscores(h);
    save_progress(h);
    canvas::show(h, Ctrl::SoloGameOverGui);
}

/// A message box (`ttMessageWindow.show` / `showYesNo`).
pub fn message_box(h: &mut EngineHost, text: &str, kind: MessageKind) {
    if !h.shell.screens {
        return;
    }
    h.shell.message = Some(ShellMessage { text: text.to_string(), kind });
}

/// Answers the pending message box: its action (`connErrorGui`'s OK is
/// `disconnectedCleanup()`).
pub fn answer_message(h: &mut EngineHost, yes: bool) {
    let Some(m) = h.shell.message.take() else {
        return;
    };
    let action = match m.kind {
        MessageKind::Ok(a) => a,
        MessageKind::YesNo(y, n) => {
            if yes {
                y
            } else {
                n
            }
        }
        MessageKind::ConnError => {
            crate::gameplay::client::connection::disconnected_cleanup(h);
            return;
        }
    };
    match action {
        MsgAction::Resume => h.cl.time_scale = Some(Val::Int(1)),
        MsgAction::DisconnectResume => {
            crate::gameplay::client::connection::disconnect(h);
            h.cl.time_scale = Some(Val::Int(1));
        }
    }
}

fn single_player(h: &EngineHost) -> bool {
    h.srv.get().is_some_and(|s| s.setup.single_player() || s.session.server_type == Some(ServerType::SinglePlayer))
}

/// `quitDlg.show()` (Escape in game): the pause menu; single player pauses the game.
pub fn quit_show(h: &mut EngineHost) {
    if !h.shell.screens {
        return;
    }
    h.input.pop_map(crate::input::MOVE_MAP);
    if single_player(h) {
        h.cl.time_scale = Some(Val::s("0.00001"));
    }
    h.shell.pause_request = true;
}

/// `quitDlg.hide()`: back to the game.
pub fn quit_hide(h: &mut EngineHost) {
    if !h.shell.screens {
        return;
    }
    h.input.push_map(crate::input::MOVE_MAP);
    if single_player(h) {
        h.cl.time_scale = Some(Val::s("1"));
    }
}

/// F1 (`toggleHelpGui`): the pause menu in game, the help in the menus.
pub fn toggle_help(h: &mut EngineHost) {
    if h.canvas.content() == Some(Ctrl::PlayGui) {
        return quit_show(h);
    }
    h.shell.help_request = true;
}

/// What `loadMainMenu()` does in the new menu: the screens, the play
/// count, then the player selection (no intro).
pub fn init_menu(h: &mut EngineHost) {
    h.shell.screens = true;
    let is_demo = if h.platform.demo { "1" } else { "0" };
    let p = &mut h.shell.prefs;
    let count = if p.get("player.play_count").is_empty() { 0.0 } else { tt_game::fmt::atof(&p.get("player.play_count")) };
    p.set_value("player.is_demo", VarValue::Str(is_demo.into()));
    p.set_value("player.play_count", VarValue::Float((count as f32 + 1.0) as f32));
    if p.get("player.unlock_baby_boss").is_empty() {
        p.set_value("player.unlock_baby_boss", VarValue::Int(1));
    }
    h.canvas.cursor = "DefaultCursor".into();
    show(h, Ctrl::LoadPlayerGui, "");
}

/// The next level of the [`SoloRun`] (a `goto_level` slot bumps the others
/// and jumps): its mission file, position and numbers. `fresh`: the
/// server starts a new sequence, so a run left over is replaced (a
/// level-select run that has not started yet is kept).
pub fn next_sp_mission(h: &mut EngineHost, fresh: bool) -> Option<CampaignStep> {
    let restart = fresh && h.shell.run.as_ref().is_none_or(|r| r.started());
    if restart && !new_run(h) {
        return None;
    }
    let mut level = h.shell.run.as_mut().and_then(|r| r.advance_level().cloned())?;
    // A level whose mission the import left out (an unreadable mission
    // file of the install) is passed over. The campaign's last slot
    // jumps back to level 2, so at most one lap of it is tried.
    let mut tries = h.shell.run.as_ref().map_or(0, |r| r.table.levels.len());
    while !h.fs.exists(&tt_assets::layout::mission(&level.mission)) {
        h.log.warn(&format!("solo: {} is not in the game data; level {} skipped", tt_assets::layout::mission(&level.mission), level.index));
        if tries == 0 {
            h.log.warn("solo: no level of the campaign is in the game data");
            return None;
        }
        tries -= 1;
        level = h.shell.run.as_mut().and_then(|r| r.advance_level().cloned())?;
    }
    let (seq, total) = h.shell.run.as_ref().map(|r| (r.mission_seq, r.mission_seq_total)).unwrap_or_default();
    // The level select opens every level the player has started.
    let player = h.shell.prefs.player_name();
    if h.shell.progress.record_reached(&player, total) {
        save_progress(h);
    }
    Some(CampaignStep { mission_file: tt_assets::layout::mission(&level.mission), seq, total, level: crate::gameplay::server_host::solo_level(&level) })
}

/// A player's multiplayer record (its field of the player list, e.g.
/// `cmissions`), "---" when missing or invalid.
pub fn player_record(h: &EngineHost, idx: &str, field: &str) -> String {
    let vars = &h.shell.players.vars;
    let src = format!("{idx}{}{}", vars.get(&format!("players.playerName{idx}")), tt_shell::IDLE_TEXT);
    let swiz = tt_game::obfuscation::strswiz(&src, "12");
    let code = vars.get(&format!("players.{field}{idx}"));
    tt_shell::players::player_data(&swiz, &code)
}

/// A finished solo run's score: the current player's high score if it
/// beats it (saved, except in the demo).
pub fn offer_high_score(h: &mut EngineHost, score: f64) {
    if h.shell.players.record_solo_score(score) && !h.platform.demo {
        save_players(h);
    }
}

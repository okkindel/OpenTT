//! The menus (crate `tt-menu`) on the native shell ([`crate::shell`],
//! crate `tt-shell`): players, high scores, prefs, the solo campaign and
//! the map catalog. The menu follows the game flow through the screen on
//! the canvas ([`crate::canvas`]):
//!
//! * `loadingGui` → Loading, `soloPlayGui` → Solo (between levels),
//!   `soloGameOverGui` → High scores, `PlayGui`/`EndGameGui` → menu hidden
//!   (the game runs), `tankSelectionGui`/`joinGameGui`/… → Main menu,
//!   `loadPlayerGui` → Player select.
//! * requests the game leaves in [`crate::shell::Shell`]: the pause menu
//!   (Escape in game), the help (F1), message boxes.
//!
//! [`Backend`] implements [`tt_menu::MenuBackend`] on tt-shell (players,
//! high scores, the typed prefs of [`crate::shell::Shell::prefs`]);
//! [`MenuBridge::apply`] starts and leaves games through the shell's game
//! control ([`shell::start_solo_level`], [`shell::launch_local`],
//! [`shell::host_game`], [`shell::join`], [`shell::leave_game`], ...).

use crate::engine::Engine;
use crate::host::EngineHost;
use crate::shell::{self, MessageKind};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use tt_menu::{
    Art, BotSetup, BotSkill, CustomGame, DisplayMode, GameModeInfo, HighScore, HostGame, KeyBinding, Loading, MenuAction, MenuBackend, MenuState, MissionInfo, PlayerInfo, Screen, ScriptMessage, ServerEntry, Settings, SoloStatus, SoloTally, TallyEvent, TallyKind, TallyStepInfo, TankInfo, Team, UiSound, WindowMode, MAX_PLAYERS,
};
use tt_game::fmt::{atof, atoi};
use tt_menu::{ServerPlayer, ServerSource};
use tt_shell::solo::{self as solo, KillKind, TallyRow};
use tt_shell::PlayerError;

/// A hit deals `damage × armor_scale` and the tank dies at `damage_amount`, so
/// the armor bar is `damage_amount / armor_scale` (Baby Boss: 10 / 1.8 ≈ 5.6).
const TANK_STATS: [(&str, f32, f32, f32, f32, f32); 4] = [("LightTank", 15.9, 14.0, 61.0, 11.8, 1.0), ("MediumTank", 11.9, 12.0, 68.0, 14.0, 1.0), ("HeavyTank", 9.1, 8.5, 82.0, 21.0, 1.0), ("BabyBoss", 16.0, 15.0, 61.0, 10.0, 1.8)];

/// The credits text.
pub const CREDITS: &str = "ui/credits.txt";

/// Where the game flow is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Menus (shell screens, loading, solo between levels).
    Shell,
    /// The game is on screen (`PlayGui`, `EndGameGui`).
    InGame,
}

/// What the tank preview should show; the GPU side renders it into
/// [`BridgeData::preview_texture`].
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewRequest {
    pub index: usize,
    pub shape: PathBuf,
    pub brain: PathBuf,
    pub size: [u32; 2],
}

/// Bridge state the backend can touch while the menu state is borrowed.
pub struct BridgeData {
    pub mode: Mode,
    last_content: Option<crate::canvas::Ctrl>,
    /// A solo game over is on the high-score screen; leaving it disconnects (`soloEndOk`).
    game_over: bool,
    tutorial_prompt: Cell<bool>,
    /// The round length at boot (s; custom games change it).
    boot_duration: f32,
    art: HashMap<Art, Option<egui::TextureHandle>>,
    credits: OnceCell<String>,
    /// Set by `tank_preview` each frame it is shown.
    pub preview_request: Option<PreviewRequest>,
    /// (tank index, texture) of the last rendered preview.
    pub preview_texture: Option<(usize, egui::TextureId)>,
    /// Log of actions performed (tests, reports).
    pub log: Vec<String>,
    /// The new-player dialog was opened for an empty player list.
    asked_new_player: bool,
    /// Sim time (ms) the round results appeared (`EndGameGui` content).
    results_t0: Option<u32>,
}

pub struct MenuBridge {
    /// Shell menus.
    pub state: MenuState,
    /// In-game menu, while open.
    pub pause: Option<MenuState>,
    /// Used to show script messages in game when the pause menu is closed.
    overlay: MenuState,
    /// The round results over the game (`EndGameGui` on the canvas).
    results: MenuState,
    pub data: BridgeData,
}

/// Creates the menu after the boot sequence.
pub fn install(engine: &mut Engine) {
    let boot_duration = engine.host.srv.state().setup.round_secs;
    engine.menu = Some(MenuBridge {
        state: MenuState::new(),
        pause: None,
        overlay: MenuState::pause(),
        results: MenuState::at(Screen::Results),
        data: BridgeData {
            mode: Mode::Shell,
            last_content: None,
            game_over: false,
            tutorial_prompt: Cell::new(false),
            boot_duration,
            art: HashMap::new(),
            credits: OnceCell::new(),
            preview_request: None,
            preview_texture: None,
            log: Vec::new(),
            asked_new_player: false,
            results_t0: None,
        },
    });
}

impl MenuBridge {
    /// The menu is drawn (shell screens, the pause menu or a script message).
    pub fn visible(&self, host: &EngineHost) -> bool {
        self.data.mode == Mode::Shell || self.pause.is_some() || host.shell.message.is_some() || self.data.results_t0.is_some()
    }

    pub fn covers(&self) -> bool {
        self.data.mode == Mode::Shell
    }

    /// Follows the canvas content and the shell's requests (see the module docs).
    pub fn sync(&mut self, host: &mut EngineHost) {
        let content = host.canvas.content();
        if content != self.data.last_content {
            self.data.last_content = content;
            let name = content.map(crate::canvas::Ctrl::name).unwrap_or("").to_ascii_lowercase();
            self.data.results_t0 = (name == "endgamegui").then(|| host.sim_time());
            self.on_content(&name, host);
        }
        let pause = std::mem::take(&mut host.shell.pause_request);
        let help = std::mem::take(&mut host.shell.help_request);
        if pause && self.data.mode == Mode::InGame && self.pause.is_none() {
            self.pause = Some(MenuState::pause());
            self.data.log.push("pause".into());
        }
        if help && self.data.mode == Mode::Shell && self.state.screen() != Screen::Help {
            self.state.open(Screen::Help);
        }
        if self.data.game_over && self.data.mode == Mode::Shell && self.state.screen() != Screen::HighScores {
            self.data.game_over = false;
            shell::end_solo_game(host);
        }
    }

    fn on_content(&mut self, name: &str, host: &mut EngineHost) {
        let was_game = self.data.mode == Mode::InGame || matches!(self.state.screen(), Screen::Loading);
        self.pause = None;
        match name {
            "playgui" | "endgamegui" => {
                self.data.mode = Mode::InGame;
                self.state.reset(Screen::MainMenu);
            }
            "loadinggui" => {
                self.data.mode = Mode::Shell;
                if self.state.screen() != Screen::Loading {
                    self.state.reset(Screen::Loading);
                }
            }
            "soloplaygui" => {
                self.data.mode = Mode::Shell;
                if self.state.screen() != Screen::Solo {
                    self.state.reset(Screen::MainMenu);
                    self.state.open(Screen::Solo);
                }
            }
            "sologameovergui" => {
                self.data.mode = Mode::Shell;
                self.data.game_over = std::mem::take(&mut host.shell.game_over);
                if self.state.screen() != Screen::HighScores {
                    self.state.reset(Screen::MainMenu);
                    self.state.open(Screen::HighScores);
                }
            }
            "loadplayergui" => {
                self.data.mode = Mode::Shell;
                if was_game {
                    self.state.reset(Screen::PlayerSelect);
                }
            }
            "connerrorgui" => {
                self.data.mode = Mode::Shell;
            }
            "joingamegui" => {
                // What a network game returns to.
                self.data.mode = Mode::Shell;
                if self.state.screen() != Screen::Online {
                    self.state.reset(Screen::MainMenu);
                    self.state.open(Screen::Online);
                }
            }
            _ => {
                // TankSelectionGui and anything else.
                self.data.mode = Mode::Shell;
                if was_game {
                    self.state.reset(Screen::MainMenu);
                }
            }
        }
        self.data.log.push(format!("content {name} -> {:?} {:?}", self.data.mode, self.state.screen()));
    }

    /// One menu frame: sync with the game, draw, perform the action.
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut EngineHost) -> Option<MenuAction> {
        self.sync(host);
        if !self.visible(host) {
            return None;
        }
        if self.data.mode == Mode::Shell && self.state.screen() == Screen::PlayerSelect {
            let none = host.shell.players.count() == 0;
            if !self.data.asked_new_player && none {
                self.data.asked_new_player = true;
                self.state.show_new_player_dialog("");
            }
        } else {
            self.data.asked_new_player = false;
        }
        let mode = self.data.mode;
        let action = {
            let mut b = Backend::new(host, &mut self.data);
            match (mode, self.pause.as_mut()) {
                (Mode::Shell, _) => self.state.ui(ctx, &mut b),
                (Mode::InGame, Some(p)) => p.ui(ctx, &mut b),
                (Mode::InGame, None) if b.d.results_t0.is_some() && b.host.borrow().shell.message.is_none() => self.results.ui(ctx, &mut b),
                (Mode::InGame, None) => self.overlay.ui(ctx, &mut b),
            }
        };
        if let Some(a) = &action {
            self.apply(a.clone(), host);
        }
        action
    }

    /// Performs a menu action.
    pub fn apply(&mut self, action: MenuAction, host: &mut EngineHost) {
        self.data.log.push(format!("{action:?}"));
        self.perform(action, host);
    }

    fn perform(&mut self, action: MenuAction, host: &mut EngineHost) {
        let boot_duration = self.data.boot_duration;
        let restore = |host: &mut EngineHost| shell::restore_duration(host, boot_duration);
        match action {
            MenuAction::StartTutorial => {
                restore(host);
                shell::start_target_practice(host);
                self.state.reset(Screen::Loading);
            }
            MenuAction::StartSolo => {
                tt_render::loadstats::start("solo level");
                restore(host);
                if shell::in_run(host) {
                    // The next level (soloPlayLaunchButton during a run).
                    shell::continue_solo_run(host);
                } else if shell::new_run(host) {
                    shell::show_lives_hud(host);
                    shell::start_solo_level(host);
                } else {
                    host.log.warn("menu: no solo levels");
                    return;
                }
                self.state.reset(Screen::Loading);
            }
            MenuAction::StartSoloAt(position) => {
                restore(host);
                if shell::new_run_at(host, position) {
                    shell::show_lives_hud(host);
                    shell::start_solo_level(host);
                    if position > 1 {
                        shell::apply_level_scales(host);
                    }
                } else {
                    host.log.warn(&format!("menu: no solo level {position}"));
                    return;
                }
                self.state.reset(Screen::Loading);
            }
            MenuAction::QuickPlay => {
                tt_render::loadstats::start("Quick Play");
                restore(host);
                let missions = shell::missions(host).to_vec();
                let Some(m) = tt_shell::catalog::pick_quick_play_mission(&missions, &mut host.shell.rng) else {
                    host.log.warn("menu: Quick Play: no map of worlds 1-3");
                    return;
                };
                let file = tt_assets::layout::mission(&m.id);
                self.data.log.push(format!("quick play {file}"));
                shell::quick_play(host, &file);
                self.state.reset(Screen::Loading);
            }
            MenuAction::StartCustom(g) => {
                tt_render::loadstats::start("custom game");
                start_custom(host, &g);
                self.state.reset(Screen::Loading);
            }
            MenuAction::Host(g) => {
                start_host(host, &g);
                self.state.reset(Screen::Loading);
            }
            MenuAction::Join { address, password, team } => {
                join(host, &address, &password, team);
                self.state.reset(Screen::Loading);
            }
            MenuAction::Resume => {
                self.pause = None;
                shell::resume(host);
            }
            MenuAction::LeaveGame => {
                self.pause = None;
                host.shell.run = None;
                host.shell.message = None;
                shell::leave_game(host);
                if self.data.mode == Mode::Shell && matches!(self.state.screen(), Screen::Loading | Screen::Solo) {
                    self.state.reset(Screen::MainMenu);
                }
            }
            MenuAction::Quit => shell::quit(host),
            MenuAction::ReimportGameData => host.reimport_requested = true,
            MenuAction::Restart => host.restart_requested = true,
        }
    }
}

/// Value of a bot.
fn bot_def(skill: BotSkill, tank: usize) -> String {
    let t = ["Light", "Medium", "Heavy"].get(tank).copied().unwrap_or("Medium");
    format!("{}{t}Tank", skill.label())
}

fn team_name(t: Team) -> &'static str {
    match t {
        Team::Auto => "",
        Team::Green => "GreenTeam",
        Team::Blue => "BlueTeam",
    }
}

/// Host a listen server (`launchMPServer`): the custom-game setup, the
/// server prefs, then the local player joins through the local connection.
fn start_host(host: &mut EngineHost, g: &HostGame) {
    custom_prefs(host, &g.game);
    {
        let p = &mut host.shell.prefs;
        p.set_server_name(g.name.trim());
        p.set_server_port(g.port);
        p.set_server_max_players(g.max_players);
        p.set_server_password(&g.password);
        p.set_host_mission(&g.game.mission);
        p.set_host_mode(g.game.mode);
        p.set_host_minutes(g.game.minutes);
        p.set_port_mapping(g.port_mapping);
        p.set_display_on_master(g.public);
    }
    shell::host_game(host, &g.game.mission);
}

/// Join a server: what `joinGameGui::onJoin` did, with the loading
/// screen showing the connection.
fn join(host: &mut EngineHost, address: &str, password: &str, team: Team) {
    host.shell.prefs.push_recent_server(address.trim_start_matches("IP:"));
    let shown = address.trim_start_matches("IP:").to_string();
    let name = crate::netgame::query::rows(host).into_iter().find(|r| r.address == address).map(|r| r.name).unwrap_or_else(|| shown.clone());
    // The team asked for, in the connect args: "Auto" (no team object) lets the
    // server pick.
    shell::join(host, address, password, if team == Team::Auto { "Auto" } else { team_name(team) });
    host.shell.loading = shell::LoadingInfo { map: format!("Joining {name}"), phase: format!("Connecting to {shown}"), progress: 0.0 };
}

/// The local-only custom game of `docs/new-menu.md` §4.
fn start_custom(host: &mut EngineHost, g: &CustomGame) {
    custom_prefs(host, g);
    shell::launch_local(host, &g.mission);
}

/// The prefs of a custom game: player tank and team, bots, mode, round.
fn custom_prefs(host: &mut EngineHost, g: &CustomGame) {
    let teams = g.mode % 2 == 1;
    let p = &mut host.shell.prefs;
    p.set_team(if teams { team_name(g.player_team) } else { "" });
    p.set("player.tank", &g.player_tank.to_string());
    p.set("server.max_bots", &g.bots.len().to_string());
    for (i, b) in g.bots.iter().enumerate() {
        p.set(&format!("server.bot.{i}"), &bot_def(b.skill, b.tank));
        p.set(&format!("server.bot_name.{i}"), &b.name);
        // Unassigned bots alternate between the teams.
        let team = match b.team {
            Team::Auto if i % 2 == 0 => "GreenTeam",
            Team::Auto => "BlueTeam",
            t => team_name(t),
        };
        p.set(&format!("server.bot_team.{i}"), team);
    }
    crate::gameplay::server_host::with_server(host, |s, b| s.bots_from_settings(b));
    shell::set_pref(host, "server.game_type_index", &g.mode.to_string());
    let s = &mut host.srv.state_mut().setup;
    s.mission_type = if g.mode >= 2 { tt_gameplay::server::MissionType::Scrum } else { tt_gameplay::server::MissionType::Deathmatch };
    s.team_game = teams;
    s.round_secs = (g.minutes.max(1) * 60) as f32;
    shell::set_pref(host, "server.replace_bots", if g.replace_bots { "1" } else { "0" });
}

fn bot_of(def: &str) -> (BotSkill, usize) {
    let d = def.to_ascii_lowercase();
    let skill = if d.starts_with("gold") {
        BotSkill::Gold
    } else if d.starts_with("silver") {
        BotSkill::Silver
    } else {
        BotSkill::Bronze
    };
    let tank = if d.contains("light") {
        0
    } else if d.contains("heavy") {
        2
    } else {
        1
    };
    (skill, tank)
}

fn player_error(e: PlayerError) -> String {
    match e {
        PlayerError::Full => format!("There can be at most {MAX_PLAYERS} players."),
        PlayerError::EmptyName => "Please enter a player name.".into(),
        PlayerError::LastPlayer => "The last player cannot be deleted.".into(),
        PlayerError::NoSuchPlayer => "No such player.".into(),
    }
}

fn tally_rows(t: &tt_shell::Tally) -> Vec<(String, String)> {
    let n = tt_game::fmt::format_g;
    let mut out: Vec<(String, String)> = t
        .rows
        .iter()
        .map(|r| match r {
            TallyRow::Kills { kind, count, pts_per, pts, out_of } => {
                let label = match kind {
                    KillKind::Bronze => "Bronze tanks",
                    KillKind::Silver => "Silver tanks",
                    KillKind::Gold => "Gold tanks",
                    KillKind::Brain => "Brains",
                };
                let of = out_of.map(|o| format!(" of {o}")).unwrap_or_default();
                (label.to_string(), format!("{count}{of} x {} = {}", n(*pts_per), n(*pts)))
            }
            TallyRow::Survival { pts } => ("Survival bonus".into(), n(*pts)),
            TallyRow::Boss { pts } => ("Boss bonus".into(), n(*pts)),
            TallyRow::Completion { pts } => ("Completion bonus".into(), n(*pts)),
        })
        .collect();
    if t.extra_lives > 0 {
        out.push(("Extra life!".into(), if t.extra_lives > 1 { format!("x{}", t.extra_lives) } else { String::new() }));
    }
    out
}

fn tally_kind(k: KillKind) -> TallyKind {
    match k {
        KillKind::Bronze => TallyKind::Bronze,
        KillKind::Silver => TallyKind::Silver,
        KillKind::Gold => TallyKind::Gold,
        KillKind::Brain => TallyKind::Brain,
    }
}

/// The tally's `advanceScore` steps for the animated solo screen.
fn tally_anim(t: &tt_shell::Tally) -> SoloTally {
    let mut out = SoloTally { start_score: t.start_score as i64, start_lives: t.lives_before.max(0) as u32, pts_per: [0; 4], max_brains: 0, survival_pts: 0, boss_pts: 0, completion_pts: 0, steps: Vec::new() };
    for r in &t.rows {
        match r {
            TallyRow::Kills { kind, pts_per, out_of, .. } => {
                out.pts_per[tally_kind(*kind) as usize] = *pts_per as i64;
                if let Some(m) = out_of {
                    out.max_brains = *m;
                }
            }
            TallyRow::Survival { pts } => out.survival_pts = *pts as i64,
            TallyRow::Boss { pts } => out.boss_pts = *pts as i64,
            TallyRow::Completion { pts } => out.completion_pts = *pts as i64,
        }
    }
    out.steps = t
        .steps
        .iter()
        .map(|s| TallyStepInfo {
            event: match s.event {
                // The `addLife` branch (150 ms) plays SPExtra; the 250 ms ones blink.
                solo::TallyEvent::ExtraLifeFlash if s.delay_ms == 150 => TallyEvent::ExtraLifeStart,
                solo::TallyEvent::ExtraLifeFlash => TallyEvent::ExtraLifeFlash,
                solo::TallyEvent::ExtraLife => TallyEvent::ExtraLife,
                solo::TallyEvent::BonusText => TallyEvent::BonusText,
                solo::TallyEvent::ShowRow(k) => TallyEvent::ShowRow(tally_kind(k)),
                solo::TallyEvent::Count(k) => TallyEvent::Count(tally_kind(k)),
                solo::TallyEvent::Survival => TallyEvent::Survival,
                solo::TallyEvent::Boss => TallyEvent::Boss,
                solo::TallyEvent::Completion => TallyEvent::Completion,
                solo::TallyEvent::Done => TallyEvent::Done,
            },
            delay_ms: s.delay_ms,
            score: s.score as i64,
            lives: s.lives.max(0) as u32,
        })
        .collect();
    out
}

/// [`MenuBackend`] over the shell for one frame.
pub struct Backend<'a> {
    host: RefCell<&'a mut EngineHost>,
    d: &'a mut BridgeData,
    players: OnceCell<Vec<PlayerInfo>>,
    solo: OnceCell<SoloStatus>,
}

impl<'a> Backend<'a> {
    pub fn new(host: &'a mut EngineHost, d: &'a mut BridgeData) -> Backend<'a> {
        Backend { host: RefCell::new(host), d, players: OnceCell::new(), solo: OnceCell::new() }
    }

    /// A pref ([`shell::Shell::prefs`]).
    fn pref(&self, name: &str) -> String {
        self.host.borrow().shell.prefs.get(name)
    }

    /// Sets a pref.
    fn set_pref(&self, name: &str, v: &str) {
        shell::set_pref(&mut self.host.borrow_mut(), name, v);
    }

    /// Runs `f` on the shell state.
    fn sh<R>(&self, f: impl FnOnce(&mut shell::Shell) -> R) -> R {
        f(&mut self.host.borrow_mut().shell)
    }

    fn player_index(&self, name: &str) -> Option<usize> {
        self.players().iter().position(|p| p.name == name)
    }

    /// Baby Boss is selectable.
    fn baby_boss(&self) -> bool {
        self.sh(|s| tt_shell::catalog::baby_boss_available(&s.players, &s.prefs))
    }

    /// After a player change: mirror, save (as `loadPlayerGui::hide` does)
    /// and the name pref.
    fn players_changed(&mut self) {
        let mut host = self.host.borrow_mut();
        let name = {
            let sh = &mut host.shell;
            sh.players.prepare_save();
            sh.players.name(sh.players.last_selection())
        };
        shell::save_players(&mut host);
        if !name.is_empty() {
            shell::set_pref(&mut host, "player.name", &name);
        }
        drop(host);
        self.players = OnceCell::new();
    }

    /// The accepted player: on to the tank selection (`loadPlayerGui.hide();
    /// tankSelectionGui.show()`).
    fn show_tank_selection(&self) {
        shell::show_tank_selection(&mut self.host.borrow_mut());
    }

    fn read_text(&self, path: &str) -> Option<String> {
        let data = self.host.borrow().fs.read(path)?;
        Some(String::from_utf8_lossy(&data).replace(" TAB ", "\t"))
    }

    fn mission_name(&self, id: &str) -> String {
        shell::mission_name(&mut self.host.borrow_mut(), id)
    }
}

impl MenuBackend for Backend<'_> {
    fn players(&self) -> Vec<PlayerInfo> {
        self.players
            .get_or_init(|| {
                self.sh(|s| {
                    (0..s.players.count())
                        .map(|i| {
                            let p = s.players.stats(i);
                            PlayerInfo { name: p.name, combat_ratio: p.combat_ratio, combat_exp: p.combat_exp, solo_high_score: p.solo_hiscore }
                        })
                        .collect()
                })
            })
            .clone()
    }

    fn current_player(&self) -> Option<String> {
        let n = self.pref("player.name");
        (!n.trim().is_empty() && self.players().iter().any(|p| p.name == n)).then_some(n)
    }

    fn select_player(&mut self, name: &str) -> Result<(), String> {
        let i = self.player_index(name).ok_or_else(|| format!("No player named \"{name}\""))?;
        self.sh(|s| s.players.select(i, &mut s.prefs)).map_err(player_error)?;
        self.players_changed();
        self.show_tank_selection();
        self.d.log.push(format!("select {name}"));
        Ok(())
    }

    fn create_player(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Please enter a player name.".into());
        }
        let players = self.players();
        if players.len() >= MAX_PLAYERS {
            return Err(format!("There can be at most {MAX_PLAYERS} players."));
        }
        if players.iter().any(|p| p.name.eq_ignore_ascii_case(name)) {
            return Err(format!("A player named \"{name}\" already exists."));
        }
        self.sh(|s| s.players.create(name, &mut s.prefs)).map_err(player_error)?;
        self.players_changed();
        self.show_tank_selection();
        self.d.tutorial_prompt.set(true);
        self.d.log.push(format!("create {name}"));
        Ok(())
    }

    fn delete_player(&mut self, name: &str) -> Result<(), String> {
        let i = self.player_index(name).ok_or_else(|| format!("No player named \"{name}\""))?;
        self.sh(|s| s.players.delete(i, &mut s.prefs)).map_err(player_error)?;
        // The deleted profile's level-select progress goes too.
        self.sh(|s| s.progress.forget(name));
        shell::save_progress(&mut self.host.borrow_mut());
        self.players_changed();
        self.d.log.push(format!("delete {name}"));
        Ok(())
    }

    fn wants_tutorial_prompt(&self) -> bool {
        self.d.tutorial_prompt.replace(false)
    }

    fn tanks(&self) -> Vec<TankInfo> {
        let unlocked = self.baby_boss();
        let host = self.host.borrow();
        tt_shell::TANKS
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let (db, speed, accel, turn, hp, armor_scale) = TANK_STATS[i];
                // Live def values when the server scripts are loaded.
                let d = host.defs.get(db);
                let get = |f: &str, def: f32| d.and_then(|o| o.get(f)).filter(|v| !v.is_empty()).map(|v| atof(v) as f32).unwrap_or(def);
                let (speed, accel, turn) = (get("max_speed", speed), get("accel_rate", accel), get("turn_rate", turn));
                let armor = get("damage_amount", hp) / get("armor_scale", armor_scale).max(0.01);
                let locked = i == 3 && !unlocked;
                TankInfo {
                    name: t.name.into(),
                    speed: (speed / 16.0).clamp(0.0, 1.0),
                    acceleration: (accel / 15.0).clamp(0.0, 1.0),
                    handling: (turn / 82.0).clamp(0.0, 1.0),
                    armor: (armor / 21.0).clamp(0.0, 1.0),
                    locked,
                    lock_reason: locked.then(|| "Score more than 500,000 points in a solo campaign to unlock.".into()),
                }
            })
            .collect()
    }

    fn selected_tank(&self) -> usize {
        let t = atoi(&self.pref("player.tank")).clamp(0, 3) as usize;
        if t == 3 && !self.baby_boss() {
            1
        } else {
            t
        }
    }

    fn select_tank(&mut self, index: usize) {
        if index > 3 || (index == 3 && !self.baby_boss()) {
            return;
        }
        self.set_pref("player.tank", &index.to_string());
        self.d.log.push(format!("tank {index}"));
    }

    fn tank_preview(&mut self, index: usize, size: [u32; 2]) -> Option<egui::TextureId> {
        let t = tt_shell::TANKS.get(index)?;
        let root = self.host.borrow().fs.root().to_path_buf();
        let dir = root.join("models/tanks");
        self.d.preview_request = Some(PreviewRequest { index, shape: dir.join(t.model), brain: dir.join(t.brain), size });
        self.d.preview_texture.filter(|(i, _)| *i == index).map(|(_, t)| t)
    }

    fn solo_status(&self) -> SoloStatus {
        self.solo
            .get_or_init(|| {
                let in_run = shell::in_run(&self.host.borrow());
                let table = shell::levels(&mut self.host.borrow_mut());
                let (level, number, lives, score, tally, anim) = self.sh(|s| match s.run.as_ref().filter(|_| in_run) {
                    Some(r) => (r.peek_next_level().cloned(), r.mission_seq_total + 1, r.lives, if r.mission_seq_total > 0 { r.score_value() } else { 0.0 }, s.last_tally.as_ref().map(tally_rows).unwrap_or_default(), s.last_tally.as_ref().map(tally_anim)),
                    None => {
                        let fresh = table.map(tt_shell::SoloRun::new);
                        (fresh.as_ref().and_then(|r| r.peek_next_level().cloned()), 1, 3, 0.0, Vec::new(), None)
                    }
                });
                let mission = level.as_ref().map(|l| l.mission.clone()).unwrap_or_default();
                // The level select: positions 1..20 (the three worlds and the
                // first of the harder second pass), more once reached.
                let player = self.pref("player.name");
                let reached = self.sh(|s| s.progress.highest(&player));
                let levels = match shell::levels(&mut self.host.borrow_mut()) {
                    Some(t) => tt_shell::campaign_levels(&t, reached.clamp(20, 99)),
                    None => Vec::new(),
                };
                let levels = levels
                    .into_iter()
                    .map(|l| tt_menu::SoloLevel { position: l.position, title: self.mission_name(&l.mission), mission: l.mission, level_type: l.level_type, briefing: l.briefing, cycle: l.cycle, unlocked: l.position <= reached })
                    .collect();
                SoloStatus {
                    level_number: number.max(1),
                    title: if mission.is_empty() { String::new() } else { self.mission_name(&mission) },
                    mission,
                    briefing: level.map(|l| l.mission_briefing).unwrap_or_default(),
                    lives: lives.max(0) as u32,
                    score: score as i64,
                    in_run,
                    tally,
                    tally_anim: anim,
                    levels,
                }
            })
            .clone()
    }

    fn high_scores(&self) -> Vec<HighScore> {
        let (list, reset) = self.sh(|s| {
            let reset = s.hiscores.entries().is_none();
            (s.hiscores.entries_or_defaults(), reset)
        });
        if reset {
            shell::save_hiscores(&mut self.host.borrow_mut());
        }
        let starts = self.sh(|s| s.progress.hiscore_starts(&list));
        list.into_iter().zip(starts).map(|(e, start)| HighScore { name: e.name, score: e.score.min(i64::MAX as u64) as i64, level: e.level, start_level: start }).collect()
    }

    fn last_run_rank(&self) -> Option<usize> {
        self.sh(|s| s.last_game_over.as_ref().and_then(|g| g.rank))
    }

    fn round_results(&self) -> Option<tt_menu::RoundResults> {
        let t0 = self.d.results_t0?;
        let host = self.host.borrow();
        let end = &host.client.end_game;
        let rows = |name: &str| {
            let list = if name == "EndGameTeamList" { &end.teams } else { &end.players };
            list.rows.iter().map(|r| r.text.clone()).collect::<Vec<String>>()
        };
        let me = host.shell.prefs.player_name();
        let team = |icon: &str| {
            let i = icon.to_ascii_lowercase();
            if i.contains("blue") {
                Some(Team::Blue)
            } else if i.contains("green") {
                Some(Team::Green)
            } else {
                None
            }
        };
        let parse = |row: &str| {
            let f: Vec<&str> = row.split('\t').collect();
            let n = |i: usize| f.get(i).map_or(0.0, |v| atof(v));
            // Without the markup control characters (bot name colours).
            let name: String = f.get(1).copied().unwrap_or("").chars().filter(|c| *c >= ' ').collect();
            let name = name.trim().to_string();
            let k = f.len() >= 7;
            tt_menu::ResultRow {
                me: !me.is_empty() && name.trim_end_matches(" [Admin]").trim_end_matches(" [Super]").trim() == me,
                team: team(f.first().copied().unwrap_or("")),
                score: n(2) as i64,
                kills: if k { n(3) as u32 } else { 0 },
                deaths: if k { n(4) as u32 } else { 0 },
                assists: if k { n(5) as u32 } else { 0 },
                name,
            }
        };
        let mut teams: Vec<tt_menu::ResultRow> = rows("EndGameTeamList").iter().map(|r| parse(r)).collect();
        teams.sort_by_key(|t| std::cmp::Reverse(t.score));
        let players = rows("EndGamePlayerList").iter().map(|r| parse(r)).collect();
        let mission_file = host.cl.mission_file.clone();
        let id = std::path::Path::new(&mission_file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let setup = &host.srv.state().setup;
        let (pause, scrum) = (setup.end_pause_secs as f32, setup.mission_type == tt_gameplay::server::MissionType::Scrum);
        let elapsed = host.sim_time().saturating_sub(t0) as f32 / 1000.0;
        let next_in = Some((pause - elapsed).max(0.0));
        let loading = Some(atof(&end.loading) as f32).filter(|v| *v > 0.0);
        drop(host);
        let mission = if id.is_empty() { String::new() } else { self.mission_name(&id) };
        Some(tt_menu::RoundResults { mission, scrum, teams, players, kda: true, next_in, loading })
    }

    fn missions(&self) -> Vec<MissionInfo> {
        let mut host = self.host.borrow_mut();
        shell::missions(&mut host)
            .iter()
            .map(|m| MissionInfo { file: tt_assets::layout::mission(&m.id), id: m.id.clone(), name: m.name.clone(), world: tt_shell::catalog::world_label(m.world).into() })
            .collect()
    }

    fn game_modes(&self) -> Vec<GameModeInfo> {
        let g = |index, label: &str, teams| GameModeInfo { index, label: label.into(), teams };
        vec![g(0, "Battlemode", false), g(1, "Team Battlemode", true), g(2, "Scrum", false), g(3, "Team Scrum", true)]
    }

    fn max_bots(&self) -> usize {
        6
    }

    fn bot_names(&self) -> Vec<String> {
        tt_gameplay::server::state::DEFAULT_BOTS.iter().map(|(_, name)| name.to_string()).collect()
    }

    fn mission_preview(&mut self, _mission: &str, _size: [u32; 2]) -> Option<egui::TextureId> {
        None
    }

    fn lan_servers(&self) -> Vec<ServerEntry> {
        crate::netgame::query::rows(&self.host.borrow())
            .into_iter()
            .map(|r| ServerEntry {
                address: r.address,
                name: r.name,
                map: r.map,
                mode: r.mode,
                players: r.players,
                max_players: r.max_players,
                bots: r.bots,
                ping_ms: r.ping_ms,
                password: r.password,
                compatible: r.compatible,
                dedicated: r.dedicated,
                player_names: r.player_list.iter().map(|(_, n)| n.clone()).collect(),
                player_list: r.player_list.iter().enumerate().map(|(i, (t, n))| ServerPlayer::new(t, n, r.player_scores.get(i).copied().unwrap_or(0))).collect(),
                modded: r.modded,
                mods: r.mods,
            })
            .collect()
    }

    fn refresh_servers(&mut self) {
        let mut host = self.host.borrow_mut();
        if host.netgame.query.source == ServerSource::Internet {
            match crate::netgame::master_addr(&host.shell.prefs) {
                Some(m) => crate::netgame::query::query_master(&mut host, m, true),
                None => host.log.warn("server list: no master server (net.master_server)"),
            }
            return;
        }
        let port = host.shell.prefs.server_port();
        crate::netgame::query::query_lan(&mut host, port, true);
    }

    fn set_server_source(&mut self, source: ServerSource) {
        let mut host = self.host.borrow_mut();
        if host.netgame.query.source != source {
            host.netgame.query.source = source;
            crate::netgame::query::cancel(&mut host);
            crate::netgame::query::clear(&mut host);
        }
    }

    fn master_server(&self) -> String {
        self.sh(|s| s.prefs.master_server())
    }

    fn set_master_server(&mut self, address: &str) {
        self.host.borrow_mut().shell.prefs.set_master_server(address);
    }

    fn querying(&self) -> bool {
        crate::netgame::query::status(&self.host.borrow()) == crate::netgame::query::QueryStatus::Querying
    }

    fn query_address(&mut self, address: &str) -> bool {
        crate::netgame::query::query_address(&mut self.host.borrow_mut(), address)
    }

    fn host_settings(&self) -> Option<HostGame> {
        let host = self.host.borrow();
        let p = &host.shell.prefs;
        if !p.has_host_settings() {
            return None;
        }
        let n = atoi(&p.get("server.max_bots")).clamp(0, 6) as usize;
        let bots = (0..n)
            .map(|i| {
                let (skill, tank) = bot_of(&p.get(&format!("server.bot.{i}")));
                let team = match p.get(&format!("server.bot_team.{i}")).to_ascii_lowercase().as_str() {
                    "greenteam" => Team::Green,
                    "blueteam" => Team::Blue,
                    _ => Team::Auto,
                };
                BotSetup { name: p.get(&format!("server.bot_name.{i}")), skill, tank, team }
            })
            .collect();
        let name = p.server_name();
        Some(HostGame {
            game: CustomGame {
                mission: p.host_mission(),
                mode: p.host_mode(),
                bots,
                player_tank: p.tank_selection() as usize,
                player_team: Team::Auto,
                minutes: p.host_minutes(),
                replace_bots: self.replace_bots(),
            },
            name: if name.trim().is_empty() { format!("{}'s game", p.player_name()) } else { name },
            max_players: p.server_max_players(),
            password: p.server_password(),
            port: p.server_port(),
            port_mapping: p.port_mapping(),
            public: p.display_on_master(),
        })
    }

    fn recent_addresses(&self) -> Vec<String> {
        self.host.borrow().shell.prefs.recent_servers()
    }

    fn take_password_request(&mut self) -> Option<String> {
        self.host.borrow_mut().netgame.password_request.take()
    }

    fn min_health_packs(&self) -> u32 {
        crate::gameplay::health_packs::wanted(&self.host.borrow().shell.prefs)
    }

    fn set_min_health_packs(&mut self, n: u32) {
        self.set_pref(crate::gameplay::health_packs::PREF, &n.min(crate::gameplay::health_packs::MAX).to_string());
    }

    fn replace_bots(&self) -> bool {
        let v = self.pref("server.replace_bots");
        v.trim().is_empty() || atoi(&v) != 0
    }

    fn play_ui_sound(&mut self, sound: UiSound) {
        // `alxPlay(<profile>)` (AudioGui description, the GUI channel).
        let mut host = self.host.borrow_mut();
        if let Some(name) = host.audio.profile_name(sound.profile()) {
            host.audio.play_profile(&name, None);
        }
    }

    fn loading(&self) -> Option<Loading> {
        let host = self.host.borrow();
        if host.canvas.content() != Some(crate::canvas::Ctrl::LoadingGui) {
            return None;
        }
        let l = host.shell.loading.clone();
        Some(Loading { map: l.map, phase: if l.phase.is_empty() { "Connecting...".into() } else { l.phase }, progress: l.progress })
    }

    fn script_message(&self) -> Option<ScriptMessage> {
        self.sh(|s| s.message.as_ref().map(|m| ScriptMessage { text: m.text.clone(), yes_no: matches!(m.kind, MessageKind::YesNo(..)) }))
    }

    fn answer_script_message(&mut self, yes: bool) {
        self.d.log.push(format!("answer {yes}"));
        shell::answer_message(&mut self.host.borrow_mut(), yes);
    }

    fn settings(&self) -> Settings {
        self.sh(|s| {
            let p = &s.prefs;
            Settings {
                resolution: p.resolution(),
                window_mode: match p.window_mode() {
                    tt_shell::WindowMode::Windowed => WindowMode::Windowed,
                    tt_shell::WindowMode::Fullscreen => WindowMode::Fullscreen,
                    tt_shell::WindowMode::Borderless => WindowMode::Borderless,
                },
                vsync: p.vsync(),
                detail_level: p.detail_level(),
                master_volume: p.master_volume(),
                music_volume: p.music_volume(),
                sound_volume: p.sound_volume(),
                mouse_sensitivity: p.mouse_sensitivity(),
                invert_mouse: p.invert_mouse(),
                reverse_steering: p.reverse_steering(),
                aim_mode: p.aim_mode(),
                glass_jar: p.glass_jar(),
                hdr: p.hdr(),
                bloom: p.bloom(),
                normal_maps: p.normal_maps(),
                normal_strength: p.normal_strength(),
                ssao: p.ssao(),
                show_fps: p.show_fps(),
                dynamic_lights: p.dynamic_lights(),
                dynamic_light_strength: p.dynamic_light_strength(),
                hud_scale: p.hud_scale(),
                ao_strength: p.ao_strength(),
                max_fps: p.max_fps(),
                shadows: p.shadows(),
            }
        })
    }

    fn apply_settings(&mut self, s: &Settings) {
        self.sh(|sh| {
            let p = &mut sh.prefs;
            p.set_resolution(s.resolution.0, s.resolution.1);
            p.set_window_mode(match s.window_mode {
                WindowMode::Windowed => tt_shell::WindowMode::Windowed,
                WindowMode::Fullscreen => tt_shell::WindowMode::Fullscreen,
                WindowMode::Borderless => tt_shell::WindowMode::Borderless,
            });
            p.set_vsync(s.vsync);
            p.set_detail_level(s.detail_level);
            p.set_master_volume(s.master_volume);
            p.set_music_volume(s.music_volume);
            p.set_sound_volume(s.sound_volume);
            p.set_mouse_sensitivity(s.mouse_sensitivity);
            p.set_invert_mouse(s.invert_mouse);
            p.set_reverse_steering(s.reverse_steering);
            p.set_aim_mode(s.aim_mode);
            p.set_glass_jar(s.glass_jar);
            p.set_hdr(s.hdr);
            p.set_bloom(s.bloom);
            p.set_normal_maps(s.normal_maps);
            p.set_normal_strength(s.normal_strength);
            p.set_ssao(s.ssao);
            p.set_show_fps(s.show_fps);
            p.set_dynamic_lights(s.dynamic_lights);
            p.set_dynamic_light_strength(if s.dynamic_lights { s.dynamic_light_strength } else { 0.0 });
            p.set_hud_scale(s.hud_scale);
            p.set_ao_strength(s.ao_strength);
            p.set_max_fps(s.max_fps);
            p.set_shadows(s.shadows);
        });
        {
            // Side effects of optionsGui::apply (plus the engine's master
            // volume and vsync); saves settings.ron.
            shell::apply_engine_settings(&mut self.host.borrow_mut());
        }
        self.d.log.push("apply settings".into());
    }

    fn display_modes(&self) -> Vec<DisplayMode> {
        self.host.borrow().video.modes.iter().map(|&(width, height)| DisplayMode { width, height }).collect()
    }

    fn desktop_mode(&self) -> DisplayMode {
        let (width, height) = self.host.borrow().video.desktop;
        DisplayMode { width, height }
    }

    fn key_bindings(&self) -> Vec<KeyBinding> {
        let host = self.host.borrow();
        let binds = host.input.bindings(crate::input::MOVE_MAP);
        // (function, label) in display order.
        const ACTIONS: [(&str, &str); 14] = [
            ("moveforward", "Forward"),
            ("movebackward", "Backward"),
            ("moveleft", "Turn left"),
            ("moveright", "Turn right"),
            ("yaw", "Aim left/right"),
            ("pitch", "Aim up/down"),
            ("mousefire", "Fire"),
            ("alttrigger", "Alternate fire"),
            ("sendspawncommand", "Respawn"),
            ("togglemessagehud", "Chat"),
            ("togglemessagehud2", "Team chat"),
            ("pagemessagehudup", "Chat page up"),
            ("pagemessagehuddown", "Chat page down"),
            ("escapefromgame", "Menu"),
        ];
        let mut out: Vec<KeyBinding> = ACTIONS.iter().map(|(_, l)| KeyBinding { action: l.to_string(), keys: Vec::new() }).collect();
        for bnd in binds {
            let f = bnd.command.action().function().to_ascii_lowercase();
            // moveleft2/3 etc. are the same action on other keys.
            let base = f.trim_end_matches(|c: char| c.is_ascii_digit());
            let key = key_label(&bnd.action);
            if let Some(i) = ACTIONS.iter().position(|(a, _)| *a == f).or_else(|| ACTIONS.iter().position(|(a, _)| *a == base)) {
                if !out[i].keys.contains(&key) {
                    out[i].keys.push(key);
                }
            }
        }
        out.retain(|k| !k.keys.is_empty());
        out
    }

    fn credits(&self) -> String {
        self.d.credits.get_or_init(|| self.read_text(CREDITS).unwrap_or_default()).clone()
    }

    /// The product and this build's version (`OpenTT 0.1.0`), not
    /// the version string the game scripts report.
    fn version(&self) -> String {
        format!("{} {}", tt_assets::product::NAME, env!("CARGO_PKG_VERSION"))
    }

    fn game_data(&self) -> Option<tt_menu::GameDataInfo> {
        let h = self.host.borrow();
        Some(tt_menu::GameDataInfo {
            source: h.shell.prefs.get(crate::gamedata::SOURCE_KEY),
            imported: h.shell.prefs.get(crate::gamedata::IMPORTED_KEY),
            location: h.fs.root().display().to_string(),
        })
    }

    fn mods(&self) -> Option<tt_menu::ModsInfo> {
        let h = self.host.borrow();
        let installed = crate::mods::installed(&h);
        let enabled = crate::mods::enabled_setting(&h);
        let (on, _) = tt_assets::mods::enabled(&installed, &enabled);
        let entry = |m: &tt_assets::mods::ModInfo, enabled: bool| tt_menu::ModEntry {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            author: m.author.clone(),
            description: m.description.clone(),
            enabled,
            gameplay: m.affects_gameplay,
            example: m.example,
        };
        let mut mods: Vec<tt_menu::ModEntry> = on.iter().map(|m| entry(m, true)).collect();
        mods.extend(installed.iter().filter(|m| !on.iter().any(|o| o.id == m.id)).map(|m| entry(m, false)));
        Some(tt_menu::ModsInfo { mods, running: h.fs.mods().iter().map(|m| m.id.clone()).collect(), folder: h.fs.data_dir().join(tt_assets::layout::MODS).display().to_string() })
    }

    fn set_mods(&mut self, enabled: &[String]) {
        crate::mods::set_enabled(&mut self.host.borrow_mut(), enabled);
    }

    fn open_mods_folder(&mut self) {
        crate::mods::open_folder(&mut self.host.borrow_mut());
    }

    fn art(&mut self, ctx: &egui::Context, art: &Art) -> Option<egui::TextureHandle> {
        if let Some(t) = self.d.art.get(art) {
            return t.clone();
        }
        let root = self.host.borrow().fs.root().to_path_buf();
        let t = tt_menu::art::load_art(ctx, &root, art);
        self.d.art.insert(art.clone(), t.clone());
        t
    }
}

/// Human-readable key name ("w" → "W", "button0" → "Mouse 1").
fn key_label(action: &str) -> String {
    let words: Vec<String> = action
        .split_whitespace()
        .map(|w| match w.to_ascii_lowercase().as_str() {
            "button0" => "Mouse 1".into(),
            "button1" => "Mouse 2".into(),
            "button2" => "Mouse 3".into(),
            "xaxis" => "Mouse X".into(),
            "yaxis" => "Mouse Y".into(),
            "zaxis" => "Wheel".into(),
            "return" => "Enter".into(),
            "pageup" => "Page Up".into(),
            "pagedown" => "Page Down".into(),
            "backslash" => "\\".into(),
            "slash" => "/".into(),
            s if s.starts_with("numpad") => format!("Num {}", &s[6..]),
            s => {
                let mut c = s.chars();
                c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
            }
        })
        .collect();
    words.join("+")
}

/// Raw egui input for a frame of `size` physical pixels without a window
/// (offscreen frontends, tests): `events` plus the screen rectangle in points.
pub fn raw_input(size: (u32, u32), events: Vec<egui::Event>, time: f64) -> egui::RawInput {
    let ppp = tt_menu::theme::pixels_per_point([size.0 as f32, size.1 as f32]);
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0 as f32 / ppp, size.1 as f32 / ppp))),
        events,
        time: Some(time),
        ..Default::default()
    };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
    raw
}

/// Mouse button 0 (left), 1 (right) or 2 (middle).
pub fn mouse_button_index(b: winit::event::MouseButton) -> Option<u8> {
    use winit::event::MouseButton;
    match b {
        MouseButton::Left => Some(0),
        MouseButton::Right => Some(1),
        MouseButton::Middle => Some(2),
        _ => None,
    }
}

/// Pointer input for the menus from window events in physical pixels. The
/// window feeds mouse events here (keyboard and text go through
/// egui-winit); [`window_raw_input`] merges both into one frame's input with
/// the menu's scale, so pointer positions are in the same points the menu
/// is laid out in, whatever the window's DPI scale factor.
#[derive(Default)]
pub struct PointerInput {
    pos: Option<egui::Pos2>,
    events: Vec<egui::Event>,
    pub modifiers: egui::Modifiers,
}

impl PointerInput {
    /// The window's pointer events: the cursor (when `track`), the
    /// buttons and the wheel (when `clicks`), the modifiers.
    pub fn window_event(&mut self, event: &winit::event::WindowEvent, size: (u32, u32), track: bool, clicks: bool) {
        use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
        match event {
            WindowEvent::CursorMoved { position, .. } if track => self.moved(position.x, position.y, size),
            WindowEvent::CursorLeft { .. } => self.left(),
            WindowEvent::ModifiersChanged(m) => {
                let m = m.state();
                self.modifiers = egui::Modifiers { alt: m.alt_key(), ctrl: m.control_key(), shift: m.shift_key(), mac_cmd: false, command: m.control_key() };
            }
            WindowEvent::MouseInput { button, state, .. } if clicks => {
                if let Some(b) = mouse_button_index(*button) {
                    self.button(b, *state == ElementState::Pressed);
                }
            }
            WindowEvent::MouseWheel { delta, .. } if clicks => match delta {
                MouseScrollDelta::LineDelta(x, y) => self.wheel(*x, *y, true),
                MouseScrollDelta::PixelDelta(p) => self.wheel(p.x as f32, p.y as f32, false),
            },
            _ => {}
        }
    }

    /// Cursor at window pixel `(x, y)` of a `size`-pixel window.
    pub fn moved(&mut self, x: f64, y: f64, size: (u32, u32)) {
        let ppp = tt_menu::theme::pixels_per_point([size.0 as f32, size.1 as f32]) as f64;
        let p = egui::pos2((x / ppp) as f32, (y / ppp) as f32);
        self.pos = Some(p);
        self.events.push(egui::Event::PointerMoved(p));
    }

    /// Mouse button 0 (left), 1 (right) or 2 (middle) at the last cursor position.
    pub fn button(&mut self, button: u8, pressed: bool) {
        let Some(pos) = self.pos else { return };
        let button = match button {
            0 => egui::PointerButton::Primary,
            1 => egui::PointerButton::Secondary,
            _ => egui::PointerButton::Middle,
        };
        self.events.push(egui::Event::PointerButton { pos, button, pressed, modifiers: self.modifiers });
    }

    /// Wheel motion in lines (`lines` true, e.g. notches) or pixels.
    pub fn wheel(&mut self, dx: f32, dy: f32, lines: bool) {
        let unit = if lines { egui::MouseWheelUnit::Line } else { egui::MouseWheelUnit::Point };
        self.events.push(egui::Event::MouseWheel { unit, delta: egui::vec2(dx, dy), modifiers: self.modifiers });
    }

    /// The cursor left the window.
    pub fn left(&mut self) {
        self.pos = None;
        self.events.push(egui::Event::PointerGone);
    }

    pub fn take_events(&mut self) -> Vec<egui::Event> {
        std::mem::take(&mut self.events)
    }
}

/// One frame's menu input for a `size`-pixel window: keyboard/text events
/// (from egui-winit, if any) plus `pointer`'s events, the screen in menu
/// points and the window focus.
pub fn window_raw_input(keyboard: Option<egui::RawInput>, pointer: &mut PointerInput, size: (u32, u32), time: f64) -> egui::RawInput {
    let (mut events, modifiers, focused) = match keyboard {
        Some(k) => (k.events, k.modifiers, k.focused),
        None => (Vec::new(), pointer.modifiers, true),
    };
    // Pointer motion/buttons from egui-winit would use its own scale: only ours count.
    events.retain(|e| !matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::PointerGone | egui::Event::MouseWheel { .. }));
    events.extend(pointer.take_events());
    let mut raw = raw_input(size, events, time);
    raw.modifiers = modifiers;
    // Focus only matters for text fields; a window that was never told it
    // has focus still gets input.
    raw.focused = focused || raw.focused;
    raw
}

/// Runs one menu frame on `ctx` (no drawing): sync, UI, action. Returns
/// egui's output for painting.
pub fn run_frame(engine: &mut Engine, ctx: &egui::Context, raw: egui::RawInput) -> egui::FullOutput {
    let Engine { host, menu, .. } = engine;
    let Some(menu) = menu.as_mut() else { return ctx.run(raw, |_| {}) };
    ctx.run(raw, |ctx| {
        let _ = menu.ui(ctx, host);
    })
}

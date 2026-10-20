//! What a golden scenario does, as typed actions (plan §4.2), and how each
//! acts on the engine ([`ScenarioAction::lower`]: [`Action`]s — typed calls
//! into the server, the client, the shell and the menu). The comments give
//! the script statement each action stood for when the fixtures were
//! recorded; the fixtures must stay identical.

use crate::gameplay::server_host;
use tt_gameplay::msg::{ClientCmd, ServerMsg, Value};
use tt_gameplay::client::value::Val;
use tt_gameplay::server::{ClientId, ServerHost};
use tt_gameplay::{EntityId, HudMsg as ClientMsg};
use super::drive::{set_var, Action};
use crate::canvas::Ctrl;
use crate::net::ClientCb;
use crate::Engine;
use std::sync::Arc;
use tt_menu::MenuAction;

/// Game mode of a custom game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Battle,
    TeamBattle,
    Scrum,
    TeamScrum,
}

impl Mode {
    pub fn scrum(self) -> bool {
        matches!(self, Mode::Scrum | Mode::TeamScrum)
    }
    pub fn teams(self) -> bool {
        matches!(self, Mode::TeamBattle | Mode::TeamScrum)
    }
}

/// A bot slot.
#[derive(Debug, Clone)]
pub struct Bot {
    pub tank: String,
    pub name: String,
    pub team: String,
}

impl Bot {
    pub fn new(tank: &str, name: &str, team: &str) -> Bot {
        Bot { tank: tank.into(), name: name.into(), team: team.into() }
    }
}

/// Aim mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aim {
    /// 0: keys and mouse turn the turret.
    Classic,
    /// 1: the mouse turns a free camera, the turret follows (the default).
    Camera,
}

/// A client of the server.
#[derive(Debug, Clone)]
pub enum Who {
    /// The local player's connection.
    Me,
    /// A test connection or fake client by its object name.
    Conn(String),
    /// `BotClientGroup` member `i`.
    Bot(usize),
}


/// Recipients of a server message.
#[derive(Debug, Clone)]
pub enum MsgTo {
    Client(Who),
    /// Everyone but this client (`messageAllExcept(client, -1, …)`).
    AllExcept(Who),
    /// A team by object name (`messageTeam(BlueTeam, …)`).
    Team(String),
    /// The client's team.
    TeamOf(Who),
    /// The client's team except the client (`messageTeamExcept`).
    TeamExcept(Who),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintKind {
    Center,
    Bottom,
}


/// Client connection events (what the network layer reports to the client).
#[derive(Debug, Clone)]
pub enum ConnEvent {
    /// A `MsgConnectionError` server message with this text.
    ErrorMessage(String),
    Error(String),
    TimedOut,
    Dropped(String),
    RequestTimedOut,
    /// Connect request rejected with a reason code (`CR_*`, `CHR_*`).
    Rejected(String),
    /// `CHR_PASSWORD` with this password entered.
    PasswordRejected(String),
    LagIcon(bool),
    Accepted,
}

/// Test helpers (cheats): effects a player cannot cause directly on cue.
#[derive(Debug, Clone)]
pub enum Cheat {
    /// The first live tank that is not the player's dies, credited to the
    /// player (`onDestroyed(victim, player)`, then deleted).
    KillOne,
    /// At the player's tank: a pick-up of TestPowerup, Heal, Reload,
    /// TestPowerup, the goal, and every flag out of bounds.
    Pickups,
    /// Solo: no quick-kill bonus pending, the level ends at the next check.
    EndLevel,
    /// Solo brain level: the player's tank onto every brain.
    BrainsToPlayer,
    /// Target practice: the first live target dies (`hurtMe(1000)`).
    KillTarget,
    /// `victim`'s tank takes `amount` damage from `attacker`'s (world, native).
    Damage { victim: Who, attacker: Who, amount: f32 },
    /// The player's tank takes `amount` damage (`hurtMe`).
    Hurt(f32),
    /// The player's tank `back` metres south of trigger `name`, facing
    /// north (`setTransform`: it drops from 15 m).
    Teleport { trigger: String, back: f32 },
    /// Solo game over with this score (`spscore`, no lives, respawn request).
    SoloGameOver(String),
}

#[derive(Debug, Clone)]
pub enum ScenarioAction {
    // ---- starting and leaving games ----
    /// The menu's Quick Play on a fixed map (default bots, free for all);
    /// `mission` is the name (`tt1_1`).
    QuickPlay { mission: String },
    /// A local game like the menu's Custom game, through the prefs
    /// (`difftest::custom_game`); `mission` is the name (`tt1_1`).
    StartCustom { mission: String, mode: Mode, bots: Vec<Bot>, minutes: f32, tank: u32 },
    /// A menu action (`tt_menu::MenuAction`) through the menu backend.
    Menu(MenuAction),
    /// Solo campaign → Launch (a new run, first level).
    SoloStart,
    /// Solo → Launch during a run (the next level).
    SoloNextLevel,
    /// The tutorial range (target practice).
    TargetPractice,
    /// The attract-mode demo server with rounds of `secs`.
    DemoServer { secs: u32 },
    /// `launchQuickServer()`: Quick Play's own map pick.
    LaunchQuickServer,
    /// A listen server (`launchMPServer(mission, type, teams)`).
    LaunchListenServer { mission: String, mission_type: String, teams: bool },
    /// A dedicated server (`launchDedServer(arg)`: a mission or a team rotation).
    LaunchDedicated { arg: String },
    /// The bot slots.
    SetBots(Vec<Bot>),
    /// `simLag(loss %, lag ms)`.
    SimLag { loss: u32, lag_ms: u32 },
    CycleGame,
    LoadMission { mission: String },
    ResetMission,
    /// Leave the game (`disconnect()`, back to the menus).
    Disconnect,
    // ---- settings ----
    /// A preference or setting (the typed console's `set name value`).
    SetPref { name: String, value: String },
    SetAim(Aim),
    /// The game setup's round length (s).
    RoundLength(f32),
    /// The game setup's dedicated server mode (`TB`, `S`, `TS`, else a battle).
    DedicatedMode(String),
    /// The bots to `index + 1` slots, the last one this (for the next bot
    /// count update).
    SetLastBotSlot { index: usize, tank: String, name: String, team: String },
    // ---- input ----
    Key { key: String, down: bool },
    /// A key typing `text` (GUI text edit), pressed and released.
    Type { key: String, text: String },
    Mouse { dx: f32, dy: f32 },
    Button { button: u8, down: bool },
    /// The action-map function a binding calls (`moveleft(0.5)`,
    /// `toggleFreeLook(1)`, `escapeFromGame()`).
    Bind { function: String, value: Option<String> },
    /// Open the chat input (`teamMessageHud(1)` / `messageHud`).
    OpenChat { team: bool },
    /// Tab: the player list (`PlayerListGui.toggle()`).
    TogglePlayerList,
    /// One shot.
    Fire,
    /// The move of the next ticks.
    Move { forward: f32, right: f32 },
    /// Respawn request while the player has a tank (as the difftest's `play` did).
    RespawnIfPlayer,
    /// Respawn request.
    RequestSpawn,
    // ---- client commands arriving at the server from the local client ----
    Chat { text: String, team: bool },
    Admin { password: String },
    AdminSetPassword { password: String },
    TogglePlayerAI,
    SpawnPlayer,
    // ---- bots and other clients ----
    DropBot { index: usize },
    UpdateBotCount,
    /// `updateTeamBotCount(0, 0)`.
    UpdateTeamBotCount,
    /// A non-network client object in `ClientGroup` and a team (bots make room).
    AddFakeClient { name: String, team: String },
    RemoveFakeClient { name: String },
    /// A test connection joins (`new GameConnection(conn)`, `onConnect(name, team, tank)`).
    Connect { conn: String, name: String, team: String, tank: u32 },
    /// The test connection leaves (`onDrop(reason)` and deleted).
    Drop { conn: String, reason: String },
    Rename { conn: String, name: String },
    Kick { conn: String },
    Ban { conn: String },
    // ---- server messages and effects ----
    Message { to: MsgTo, kind: String, text: String, args: Vec<String> },
    /// `centerPrint`/`bottomPrint` (one client) or `…All` (`to: None`).
    Print { to: Option<Who>, kind: PrintKind, text: String, secs: String, lines: String },
    /// Clear both prints of one client or everyone.
    ClearPrints { to: Option<Who> },
    /// `ServerPlay2D(profile)` / `ServerPlay3D(profile, transform)`.
    ServerSound { profile: String, at: Option<String> },
    /// The player's camera through corpse and observer modes and back.
    CameraModes,
    // ---- messages arriving at the local client (as if from a server) ----
    ClientChat { sender: String, pitch: String, text: String },
    ClientPrint { kind: PrintKind, text: String, secs: String, lines: String },
    ClientClearPrints,
    ClientGameStart,
    ClientSyncClock { secs: String, dir: String },
    ClientHud(ClientMsg),
    /// The mission load info (`MsgLoadInfo` then `MsgLoadInfoDone`).
    ClientLoadInfo { name: String, lines: Vec<String> },
    Connection(ConnEvent),
    Cheat(Cheat),
}


fn native(f: impl Fn(&mut Engine) + Send + Sync + 'static) -> Vec<Action> {
    vec![Action::Native(Arc::new(f))]
}

/// Messages to this process's client handlers, now.
fn client(msgs: Vec<ServerMsg>) -> Vec<Action> {
    native(move |e| {
        for m in &msgs {
            crate::gameplay::client::recv::apply(&mut e.host, m);
        }
    })
}

/// A command of the local client run by the server at once.
fn local_cmd(cmd: ClientCmd) -> Vec<Action> {
    native(move |e| {
        let Some(c) = server_host::local_client(&e.host) else { return };
        server_host::with_server(&mut e.host, |s, h| s.client_cmd(h, c, &cmd));
    })
}

fn set_bots(e: &mut Engine, bots: &[Bot]) {
    let h = &mut e.host;
    set_var(h, "server.max_bots", crate::shell::literal(&bots.len().to_string()));
    for (i, b) in bots.iter().enumerate() {
        set_var(h, &format!("server.bot.{i}"), Val::s(&b.tank));
        if !b.name.is_empty() {
            set_var(h, &format!("server.bot_name.{i}"), Val::s(&b.name));
        }
        if !b.team.is_empty() {
            set_var(h, &format!("server.bot_team.{i}"), Val::s(&b.team));
        }
    }
}

/// `ServerConnection.<callback>(...)`: a callback of the client's
/// connection, if it exists.
fn conn_cb(e: &mut Engine, cb: ClientCb) {
    if e.host.net.server_connection().is_some() {
        crate::gameplay::client::connection::callback(&mut e.host, cb);
    }
}

/// The server's client `who` (the local player, a test or fake client by
/// name, a bot by its place in `BotClientGroup`).
fn client_of(e: &Engine, who: &Who) -> Option<ClientId> {
    match who {
        Who::Me => server_host::local_client(&e.host),
        Who::Conn(n) => e.host.srv.named.get(&n.to_ascii_lowercase()).copied(),
        Who::Bot(i) => e.host.srv.get()?.bots().get(*i).copied(),
    }
}

/// `who`'s player tank (entity), while it exists.
fn player_of(e: &Engine, who: &Who) -> Option<EntityId> {
    let c = client_of(e, who)?;
    let p = e.host.srv.get()?.client(c)?.player?;
    e.host.game.as_ref()?.entities.get(p).map(|x| x.id)
}

/// `who`'s player tank (world id).
fn client_tank(e: &Engine, who: &Who) -> Option<tt_game::TankId> {
    e.host.game.as_ref()?.tank_of(player_of(e, who)?)
}

/// Runs `f` on the server with the engine.
fn with(e: &mut Engine, f: impl FnOnce(&mut tt_gameplay::server::Server, &mut server_host::Bridge)) {
    server_host::with_server(&mut e.host, f);
}

impl ScenarioAction {
    pub fn key(key: &str, down: bool) -> ScenarioAction {
        ScenarioAction::Key { key: key.into(), down }
    }
    pub fn pref(name: &str, value: &str) -> ScenarioAction {
        ScenarioAction::SetPref { name: name.into(), value: value.into() }
    }
    pub fn bind(function: &str, value: Option<&str>) -> ScenarioAction {
        ScenarioAction::Bind { function: function.into(), value: value.map(String::from) }
    }

    /// The harness actions of this build.
    pub fn lower(&self) -> Vec<Action> {
        use super::drive as d;
        use ScenarioAction::*;
        match self {
            QuickPlay { mission } => vec![d::quick_play(mission)],
            StartCustom { mission, mode, bots, minutes, tank } => {
                let b: Vec<(&str, &str, &str)> = bots.iter().map(|b| (b.tank.as_str(), b.name.as_str(), b.team.as_str())).collect();
                vec![d::custom_game(mission, mode.scrum(), mode.teams(), &b, *minutes, *tank)]
            }
            Menu(a) => vec![Action::Menu(a.clone())],
            SoloStart => d::solo_start(),
            SoloNextLevel => vec![d::solo_next_level()],
            TargetPractice => vec![d::target_practice()],
            DemoServer { secs } => {
                let secs = *secs;
                native(move |e| {
                    let s = &mut e.host.srv.state_mut().setup;
                    s.demo_mode = Some(true);
                    s.demo_round_secs = secs as f32;
                    server_host::launch_demo(&mut e.host);
                })
            }
            LaunchQuickServer => native(|e| server_host::launch_quick(&mut e.host)),
            LaunchListenServer { mission, mission_type, teams } => {
                let (m, t, n) = (mission.clone(), tt_gameplay::server::MissionType::parse(mission_type), *teams);
                native(move |e| server_host::launch_listen(&mut e.host, &m, t, n))
            }
            LaunchDedicated { arg } => {
                let arg = arg.clone();
                native(move |e| with(e, |s, h| s.launch_dedicated(h, &arg)))
            }
            SetBots(bots) => {
                let bots = bots.clone();
                native(move |e| set_bots(e, &bots))
            }
            SimLag { loss, lag_ms } => {
                let (loss, lag) = (loss.to_string(), lag_ms.to_string());
                native(move |e| server_host::sim_lag(&mut e.host, &loss, &lag))
            }
            CycleGame => native(|e| with(e, |s, h| s.cycle_round(h))),
            LoadMission { mission } => {
                let m = mission.clone();
                native(move |e| with(e, |s, h| s.load_mission(h, &m, false)))
            }
            ResetMission => native(|e| with(e, |s, h| s.reset_mission(h))),
            Disconnect => native(|e| {
                e.host.cl.disconnect_gui = Some(Ctrl::TankSelectionGui);
                crate::gameplay::client::connection::disconnect(&mut e.host);
            }),
            // `<name> = "<value>";`
            SetPref { name, value } => {
                let (name, value) = (name.clone(), value.clone());
                native(move |e| set_var(&mut e.host, &name, Val::s(&value)))
            }
            RoundLength(secs) => {
                let secs = *secs;
                native(move |e| e.host.srv.state_mut().setup.round_secs = secs)
            }
            DedicatedMode(arg) => {
                let mode = tt_gameplay::server::state::DedicatedMode::parse(arg);
                native(move |e| e.host.srv.state_mut().setup.dedicated_mode = mode)
            }
            SetLastBotSlot { index, tank, name, team } => {
                let slot = tt_gameplay::server::BotSlot::new(tank, name, team);
                let i = *index;
                native(move |e| {
                    let slots = &mut e.host.srv.state_mut().bot_slots;
                    slots.resize(i + 1, Default::default());
                    slots[i] = slot.clone();
                })
            }
            SetAim(a) => {
                let v = u32::from(*a != Aim::Classic);
                native(move |e| set_var(&mut e.host, "input.aim_mode", Val::Int(v)))
            }
            Key { key, down } => vec![Action::Key(key.clone(), *down)],
            Type { key, text } => vec![Action::Type(key.clone(), text.clone())],
            Mouse { dx, dy } => vec![Action::Mouse(*dx, *dy)],
            Button { button, down } => vec![Action::Button(*button, *down)],
            // `<function>(<value>);`: the bound action.
            Bind { function, value } if function.eq_ignore_ascii_case("setSpeed") => {
                let v = value.clone().unwrap_or_default();
                native(move |e| e.host.client.moves.set_speed(&v))
            }
            Bind { function, value } => {
                let a = tt_gameplay::client::binds::BindAction::from_function(function).unwrap_or_else(|| panic!("golden: no action {function}"));
                let v = value.clone().unwrap_or_default();
                native(move |e| crate::gameplay::client::game::bind_action(&mut e.host, a, &v))
            }
            // `teamMessageHud(<team>);`
            OpenChat { team } => {
                let v = (*team as u32).to_string();
                native(move |e| crate::gameplay::client::hud::message_hud_key(&mut e.host, &v, true))
            }
            // `PlayerListGui.toggle();`
            TogglePlayerList => native(|e| crate::gameplay::client::hud::toggle_player_list(&mut e.host)),
            Fire => native(|e| e.host.client.moves.trigger(0)),
            Move { forward, right } => {
                let (f, r) = (*forward, *right);
                native(move |e| e.host.client.moves.set_drive(f, r))
            }
            RespawnIfPlayer => native(|e| {
                if player_of(e, &Who::Me).is_some() {
                    crate::net::send_cmd(&mut e.host, ClientCmd::SpawnPlayer);
                }
            }),
            RequestSpawn => native(|e| crate::net::send_cmd(&mut e.host, ClientCmd::SpawnPlayer)),
            Chat { text, team } => {
                let cmd = if *team { ClientCmd::TeamMessageSent { text: text.clone() } } else { ClientCmd::MessageSent { text: text.clone() } };
                local_cmd(cmd)
            }
            Admin { password } => local_cmd(ClientCmd::Sad { password: password.clone() }),
            AdminSetPassword { password } => local_cmd(ClientCmd::SadSetPassword { password: password.clone() }),
            TogglePlayerAI => local_cmd(ClientCmd::Debug { cmd: tt_gameplay::DebugCmd::TogglePlayerAI, args: Vec::new() }),
            SpawnPlayer => local_cmd(ClientCmd::SpawnPlayer),
            DropBot { index } => {
                let i = *index;
                native(move |e| {
                    if let Some(b) = client_of(e, &Who::Bot(i)) {
                        with(e, |s, h| s.drop_bot(h, b));
                    }
                })
            }
            UpdateBotCount => native(|e| with(e, |s, h| s.balance_bots(h))),
            UpdateTeamBotCount => native(|e| with(e, |s, h| s.balance_team_bots(h, None))),
            AddFakeClient { name, team } => {
                let (name, team) = (name.clone(), team.clone());
                native(move |e| {
                    let mut id = None;
                    with(e, |s, h| id = Some(s.add_fake_client(h, &team)));
                    if let Some(c) = id {
                        e.host.srv.named.insert(name.to_ascii_lowercase(), c);
                    }
                })
            }
            RemoveFakeClient { name } => {
                let name = name.clone();
                native(move |e| {
                    if let Some(c) = e.host.srv.named.remove(&name.to_ascii_lowercase()) {
                        with(e, |s, h| s.remove_client(h, c));
                    }
                })
            }
            Connect { conn, name, team, tank } => {
                let (conn, name, team, tank) = (conn.clone(), name.clone(), team.clone(), tank.to_string());
                native(move |e| {
                    with(e, |s, h| {
                        let c = s.add_client(h, tt_gameplay::server::ClientKind::Test);
                        h.h.srv.tests.insert(c);
                        h.h.srv.named.insert(conn.to_ascii_lowercase(), c);
                        s.connected(h, c, &name, &team, &tank);
                    });
                })
            }
            Drop { conn, .. } => {
                let conn = conn.clone();
                native(move |e| {
                    if let Some(c) = client_of(e, &Who::Conn(conn.clone())) {
                        with(e, |s, h| {
                            s.dropped(h, c);
                            s.remove_client(h, c);
                        });
                    }
                })
            }
            Rename { conn, name } => {
                let (conn, name) = (conn.clone(), name.clone());
                native(move |e| {
                    if let Some(c) = client_of(e, &Who::Conn(conn.clone())) {
                        with(e, |s, _| s.set_player_name(c, &name));
                    }
                })
            }
            Kick { conn } | Ban { conn } => {
                let (conn, ban) = (conn.clone(), matches!(self, Ban { .. }));
                native(move |e| {
                    if let Some(c) = client_of(e, &Who::Conn(conn.clone())) {
                        with(e, |s, h| if ban { s.ban(h, c) } else { s.kick(h, c) });
                    }
                })
            }
            Message { to, kind, text, args } => {
                let (to, kind, text, args) = (to.clone(), kind.clone(), text.clone(), args.clone());
                native(move |e| {
                    let (k, t) = tt_gameplay::server::Server::message_of(&kind, &text, &args);
                    let who = |w: &Who| client_of(e, w);
                    match &to {
                        MsgTo::Client(w) => {
                            if let Some(c) = who(w) {
                                with(e, |s, h| s.message_client(h, c, &k, &t));
                            }
                        }
                        MsgTo::AllExcept(w) => {
                            if let Some(c) = who(w) {
                                with(e, |s, h| s.message_all_except(h, c, &k, &t));
                            }
                        }
                        MsgTo::Team(team) => {
                            let team = tt_gameplay::server::TeamColor::parse(team);
                            with(e, |s, h| s.message_team(h, team, &k, &t));
                        }
                        MsgTo::TeamOf(w) => {
                            if let Some(c) = who(w) {
                                with(e, |s, h| {
                                    let team = s.team_of(c);
                                    s.message_team(h, team, &k, &t)
                                });
                            }
                        }
                        MsgTo::TeamExcept(w) => {
                            if let Some(c) = who(w) {
                                with(e, |s, h| s.message_team_except(h, c, &k, &t));
                            }
                        }
                    }
                })
            }
            Print { to, kind, text, secs, lines } => {
                let (to, kind, text, secs, lines) = (to.clone(), *kind, text.clone(), secs.clone(), lines.clone());
                native(move |e| {
                    let c = to.as_ref().map(|w| client_of(e, w));
                    with(e, |s, h| match (c, kind) {
                        (Some(c), PrintKind::Center) => s.center_print(h, c, &tt_gameplay::Value::str(text.clone()), &secs, &lines),
                        (Some(c), PrintKind::Bottom) => s.bottom_print(h, c, &tt_gameplay::Value::str(text.clone()), &secs, &lines),
                        (None, PrintKind::Center) => s.center_print_all(h, &text, &secs, &lines),
                        (None, PrintKind::Bottom) => s.bottom_print_all(h, &text, &secs, &lines),
                    });
                })
            }
            ClearPrints { to } => {
                let to = to.clone();
                native(move |e| {
                    let c = to.as_ref().map(|w| client_of(e, w));
                    with(e, |s, h| match c {
                        Some(c) => {
                            s.clear_center_print(h, c);
                            s.clear_bottom_print(h, c);
                        }
                        None => {
                            s.clear_center_print_all(h);
                            s.clear_bottom_print_all(h);
                        }
                    });
                })
            }
            ServerSound { profile, at } => {
                let (profile, at) = (profile.clone(), at.clone());
                native(move |e| {
                    let Some(sound) = e.host.data.sounds.find(&profile) else { return };
                    with(e, |s, h| match &at {
                        Some(t) => s.server_play_3d(h, sound, t),
                        None => s.server_play_2d(h, sound),
                    })
                })
            }
            // The observer camera's modes had no effect on the game (the
            // camera object is gone with the native server).
            CameraModes => Vec::new(),
            // The client's handlers called directly.
            ClientChat { sender, pitch, text } => client(vec![ServerMsg::Chat { from: sender.clone(), voice: Value::empty(), pitch: pitch.clone(), text: Value::str(text.clone()), args: Vec::new() }]),
            ClientPrint { kind, text, secs, lines } => {
                let (text, secs, lines) = (Value::str(text.clone()), secs.clone(), lines.clone());
                client(vec![match kind {
                    PrintKind::Center => ServerMsg::CenterPrint { text, secs, lines, lc: false },
                    PrintKind::Bottom => ServerMsg::BottomPrint { text, secs, lines, lc: false },
                }])
            }
            ClientClearPrints => client(vec![ServerMsg::ClearCenterPrint, ServerMsg::ClearBottomPrint]),
            ClientGameStart => client(vec![ServerMsg::GameStart]),
            ClientSyncClock { secs, dir } => client(vec![ServerMsg::SyncClock { secs: secs.clone(), dir: dir.clone(), lc: false }]),
            // [stage 5] `SPLivesGui.showLives(v);`, `SPScoreGui.setScore(v);`,
            // `SPBrainCounter.maxBrains = v;`, `SPBrainCounter.setCount(v);`
            ClientHud(m) => {
                let m = m.clone();
                match &m {
                    ClientMsg::Lives(_) | ClientMsg::Score(_) | ClientMsg::BrainsMax(_) | ClientMsg::BrainsCount(_) => {}
                    other => panic!("ClientHud({other:?}) has no lowering"),
                }
                native(move |e| {
                    let s = &mut e.host.client.solo;
                    match &m {
                        ClientMsg::Lives(v) => s.show_lives(v),
                        ClientMsg::Score(v) => s.set_score(v),
                        ClientMsg::BrainsMax(v) => s.max_brains = v.clone(),
                        ClientMsg::BrainsCount(v) => s.set_brain_count(v),
                        _ => {}
                    }
                })
            }
            // `handleLoadInfoMessage("", "", name); handleLoadInfoDoneMessage("", "");`
            // (the load info lines are cleared, nothing shows them).
            ClientLoadInfo { name, .. } => {
                let name = name.clone();
                native(move |e| crate::gameplay::client::connection::handle_load_info_message(&mut e.host, &name))
            }
            Connection(c) => {
                let c = c.clone();
                native(move |e| match &c {
                    // `handleConnectionErrorMessage("", "", text);`
                    ConnEvent::ErrorMessage(t) => crate::gameplay::client::connection::handle_connection_error_message(&mut e.host, t),
                    // `ServerConnection.onConnectionError(text);` ...
                    ConnEvent::Error(t) => conn_cb(e, ClientCb::ConnectionError(t.clone())),
                    ConnEvent::TimedOut => conn_cb(e, ClientCb::ConnectionTimedOut),
                    ConnEvent::Dropped(t) => conn_cb(e, ClientCb::ConnectionDropped(t.clone())),
                    ConnEvent::RequestTimedOut => conn_cb(e, ClientCb::ConnectRequestTimedOut),
                    ConnEvent::Rejected(r) => conn_cb(e, ClientCb::ConnectRequestRejected(r.clone())),
                    ConnEvent::PasswordRejected(p) => {
                        e.host.cl.password = p.clone();
                        conn_cb(e, ClientCb::ConnectRequestRejected("CHR_PASSWORD".into()));
                    }
                    // `ServerConnection.setLagIcon(on);`
                    ConnEvent::LagIcon(on) => {
                        if let Some(c) = e.host.net.server_connection() {
                            crate::gameplay::client::connection::set_lag_icon(&mut e.host, c, *on);
                        }
                    }
                    // `ServerConnection.initialControlSet(); ServerConnection.onConnectionAccepted();`
                    ConnEvent::Accepted => {
                        conn_cb(e, ClientCb::InitialControlSet);
                        conn_cb(e, ClientCb::ConnectionAccepted);
                    }
                })
            }
            Cheat(c) => match c.clone() {
                self::Cheat::KillOne => native(|e| {
                    // The first live tank of `MissionCleanup` that is not the
                    // player's dies, credited to the player, and goes.
                    let Some(me) = player_of(e, &Who::Me) else { return };
                    let Some(srv) = e.host.srv.get() else { return };
                    let Some(g) = e.host.game.as_ref() else { return };
                    let victim = srv.cleanup_tanks().into_iter().find(|t| *t != me && g.tank_of(*t).is_some_and(|w| !g.world.is_corpse(w)));
                    if let Some(v) = victim {
                        with(e, |s, h| {
                            s.tank_destroyed(h, v, Some(me));
                            s.delete_tank(h, v);
                        });
                    }
                }),
                self::Cheat::Pickups => native(|e| {
                    // At the player's tank: a pick-up of TestPowerup, Heal,
                    // Reload, TestPowerup (new power-ups there), the goal,
                    // and every ball out of bounds.
                    let Some(me) = player_of(e, &Who::Me) else { return };
                    // `%db.onEnter(...)` by name: a mission object of that
                    // name (the reload arch is called `Reload`) has no
                    // `onEnter`, only the def does.
                    // (A mission object's name hides a def's.)
                    let def = |e: &Engine, n: &str| e.host.defs.get(n).is_some() && !e.host.mission_load.as_ref().is_some_and(|l| l.nodes.iter().any(|(_, o)| o.name.eq_ignore_ascii_case(n)));
                    for db in ["TestPowerup", "Heal", "Reload", "TestPowerup"] {
                        let enter = def(e, db);
                        let Some(db) = e.host.data.pickups.find(db) else { continue };
                        with(e, |s, h| {
                            if let Some(u) = s.test_power_up(h, db) {
                                let t = h.transform(me);
                                h.set_transform(u, &t);
                                if enter {
                                    s.pickup_entered(h, db, u, me);
                                }
                            }
                        });
                    }
                    if let Some(def) = e.host.data.pickups.find("DefaultGoal").filter(|_| def(e, "DefaultGoal")) {
                        with(e, |s, h| {
                            let goal = h.find_entity("Goal").unwrap_or(EntityId(0));
                            s.pickup_entered(h, def, goal, me);
                        });
                    }
                    let flags: Vec<EntityId> = match (e.host.srv.get(), e.host.game.as_ref()) {
                        (Some(s), Some(g)) => s.cleanup_entities().into_iter().filter(|x| g.entities.flag(*x).is_some()).collect(),
                        _ => Vec::new(),
                    };
                    if def(e, "DefaultFlag") {
                        for f in flags {
                            with(e, |s, h| s.ball_out_of_bounds(h, f));
                        }
                    }
                }),
                self::Cheat::EndLevel => native(|e| {
                    if server_host::local_client(&e.host).is_some() && e.host.srv.state().round.running() {
                        server_host::with_local_record(&mut e.host, |r| r.last_kill_time = Some(0));
                        if let Some(l) = e.host.srv.state_mut().campaign.level.as_mut() {
                            l.max_mission_ticks = 1.0;
                        }
                    }
                }),
                self::Cheat::BrainsToPlayer => native(|e| {
                    let Some(me) = player_of(e, &Who::Me) else { return };
                    let brains: Vec<String> = e
                        .host
                        .mission_load
                        .as_ref()
                        .and_then(|l| l.group_named("BrainGroup"))
                        .map(|n| n.children.iter().map(|c| crate::game::fields_transform(c.object.get("position").unwrap_or(""), c.object.get("rotation").unwrap_or(""))).collect())
                        .unwrap_or_default();
                    for t in brains {
                        if let Some(g) = e.host.game.as_mut() {
                            g.set_transform(me, &t);
                        }
                    }
                }),
                self::Cheat::KillTarget => native(|e| {
                    let Some(me) = player_of(e, &Who::Me) else { return };
                    let (Some(srv), Some(g)) = (e.host.srv.get(), e.host.game.as_ref()) else { return };
                    let t = srv.cleanup_entities().into_iter().filter_map(|x| g.tank_of(x).filter(|_| x != me)).find(|w| !g.world.is_corpse(*w));
                    if let (Some(t), Some(g)) = (t, e.host.game.as_mut()) {
                        g.world.hurt_me(t, "1000");
                    }
                }),
                self::Cheat::Hurt(n) => native(move |e| {
                    let Some(t) = client_tank(e, &Who::Me) else { return };
                    if let Some(g) = e.host.game.as_mut() {
                        g.world.hurt_me(t, &n.to_string());
                    }
                }),
                self::Cheat::SoloGameOver(score) => {
                    let score = tt_game::fmt::atof(&score);
                    native(move |e| {
                        let Some(me) = server_host::local_client(&e.host) else { return };
                        server_host::with_local_record(&mut e.host, |r| {
                            r.spscore = Some(score);
                            r.lives = Some(0);
                        });
                        with(e, |s, h| s.respawn_request(h, me));
                    })
                }
                self::Cheat::Damage { victim, attacker, amount } => native(move |e| {
                    let (Some(v), Some(a)) = (client_tank(e, &victim), client_tank(e, &attacker)) else { return };
                    if let Some(g) = e.host.game.as_mut() {
                        g.world.apply_damage(v, amount, Some(a));
                    }
                }),
                self::Cheat::Teleport { trigger, back } => native(move |e| {
                    let Some(o) = e.control_object() else { return };
                    let Some(p) = e.host.game.as_ref().and_then(|g| g.world.triggers.iter().find(|t| t.name.eq_ignore_ascii_case(&trigger)).map(|t| t.transform.translation)) else { return };
                    let xf = format!("{} {} {} 0 0 1 0", p.x, p.y - back, p.z + 1.0);
                    if let Some(g) = e.host.game.as_mut() {
                        g.set_transform(o, &xf);
                    }
                }),
            },
        }
    }
}

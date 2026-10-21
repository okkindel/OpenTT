//! What the golden harness does to a running engine ([`Action`]) and the
//! menu-equivalent game starts ([`custom_game`], [`quick_play`], …) the
//! scenario actions lower to ([`super::ScenarioAction::lower`]).

use crate::canvas::Ctrl;
use crate::host::EngineHost;
use crate::Engine;
use tt_gameplay::client::value::Val;
use tt_gameplay::server::MissionType;

/// Something the scenario does at a given time.
#[derive(Clone)]
pub enum Action {
    /// A key through the engine's input path (`pressed`).
    Key(String, bool),
    /// A key press with the text it types (text input), then its release.
    Type(String, String),
    /// Relative mouse motion (`xaxis`/`yaxis`) to the action maps.
    Mouse(f32, f32),
    /// A mouse button (0 = left) at canvas coordinates.
    Button(u8, bool),
    /// Native code (golden scenario actions, the menu's solo-run bookkeeping).
    Native(std::sync::Arc<dyn Fn(&mut Engine) + Send + Sync>),
    /// A menu action through the menu backend (`MenuBridge::apply`).
    Menu(tt_menu::MenuAction),
}

/// As the scenarios set a setting or one of the client's values.
pub fn set_var(h: &mut EngineHost, name: &str, v: Val) {
    if tt_shell::settings::is_key(name) {
        crate::shell::set_pref_val(&mut h.shell.prefs, name, v);
    } else if name.eq_ignore_ascii_case("camera.fov") {
        h.cl.camera_fov = Some(v);
    } else if name.eq_ignore_ascii_case("sim.time_scale") {
        h.cl.time_scale = Some(v);
    } else if name.eq_ignore_ascii_case("ui.disconnect_gui") {
        h.cl.disconnect_gui = Ctrl::from_name(&v.string());
    } else {
        panic!("golden: no setting {name}");
    }
}

/// A quoted text value.
fn text(s: &str) -> Val {
    Val::s(s)
}

/// The game setup.
fn setup(h: &mut EngineHost) -> &mut tt_gameplay::server::GameSetup {
    &mut h.srv.state_mut().setup
}

/// Starts a local game like the menu's Custom game: the mode, the bots (the settings'
/// bots), the round length, then the local game.
pub fn custom_game(mission: &str, scrum: bool, teams: bool, bots: &[(&str, &str, &str)], minutes: f32, tank: u32) -> Action {
    let bots: Vec<(String, String, String)> = bots.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect();
    let file = tt_assets::layout::mission(mission);
    native(move |e| {
        let h = &mut e.host;
        set_var(h, "player.team", text(if teams { "BlueTeam" } else { "" }));
        set_var(h, "player.tank", crate::shell::literal(&tank.to_string()));
        set_var(h, "server.max_bots", crate::shell::literal(&bots.len().to_string()));
        for (i, (db, name, team)) in bots.iter().enumerate() {
            set_var(h, &format!("server.bot.{i}"), text(db));
            set_var(h, &format!("server.bot_name.{i}"), text(name));
            set_var(h, &format!("server.bot_team.{i}"), text(team));
        }
        crate::gameplay::server_host::with_server(&mut e.host, |s, b| s.bots_from_settings(b));
        let s = setup(&mut e.host);
        s.mission_type = if scrum { MissionType::Scrum } else { MissionType::Deathmatch };
        s.team_game = teams;
        s.round_secs = minutes * 60.0;
        crate::shell::launch_local(&mut e.host, &file);
    })
}

/// The menu's Quick Play on `mission` (the default bots, free for all).
pub fn quick_play(mission: &str) -> Action {
    let file = tt_assets::layout::mission(mission);
    native(move |e| {
        let s = e.host.srv.state_mut();
        s.default_bots();
        s.setup.mission_type = MissionType::Deathmatch;
        s.setup.team_game = false;
        crate::shell::launch_local(&mut e.host, &file);
    })
}

fn native(f: impl Fn(&mut Engine) + Send + Sync + 'static) -> Action {
    Action::Native(std::sync::Arc::new(f))
}

/// Not in the solo campaign, back to `gui` on a disconnect, the loading
/// screen.
fn to_loading(h: &mut EngineHost, gui: Ctrl) {
    setup(h).single_player = Some(false);
    h.cl.disconnect_gui = Some(gui);
    crate::shell::show(h, Ctrl::LoadingGui, "");
}

/// The menu's Solo Campaign → Launch (a new run, first level).
pub fn solo_start() -> Vec<Action> {
    vec![
        native(|e| {
            crate::shell::new_run(&mut e.host);
            crate::shell::show_lives_hud(&mut e.host);
        }),
        native(|e| {
            to_loading(&mut e.host, Ctrl::TankSelectionGui);
            crate::gameplay::server_host::launch_campaign(&mut e.host);
        }),
    ]
}

/// The menu's Solo → Launch during a run (the next level).
pub fn solo_next_level() -> Action {
    native(|e| {
        let h = &mut e.host;
        if crate::shell::in_run(h) {
            crate::shell::prepare_next_level(h);
            crate::shell::show_lives_hud(h);
            h.cl.disconnect_gui = Some(Ctrl::TankSelectionGui);
            crate::shell::show(h, Ctrl::LoadingGui, "");
            crate::gameplay::server_host::with_server(h, |s, b| s.next_round(b));
        }
    })
}

/// The tutorial (target practice).
pub fn target_practice() -> Action {
    native(|e| {
        to_loading(&mut e.host, Ctrl::LoadPlayerGui);
        crate::gameplay::server_host::launch_target_practice(&mut e.host);
    })
}

pub fn apply(e: &mut Engine, a: &Action) {
    match a {
        Action::Key(k, p) => e.key(k, *p, None),
        Action::Type(k, text) => {
            e.key(k, true, Some(text));
            e.key(k, false, None);
        }
        Action::Mouse(dx, dy) => {
            use crate::input::{Device, InputEvent};
            if *dx != 0.0 {
                e.input(&InputEvent::Axis { device: Device::Mouse, axis: "xaxis".into(), delta: *dx });
            }
            if *dy != 0.0 {
                e.input(&InputEvent::Axis { device: Device::Mouse, axis: "yaxis".into(), delta: *dy });
            }
        }
        Action::Button(b, p) => e.mouse_button(*b, *p),
        Action::Native(f) => f(e),
        Action::Menu(a) => {
            let Engine { host, menu, .. } = e;
            if let Some(m) = menu.as_mut() {
                m.apply(a.clone(), host);
            }
        }
    }
}

/// The first difference of two traces with `context` lines around it
/// (None: equal).
pub fn first_difference(a: &[String], b: &[String], context: usize) -> Option<String> {
    let n = a.len().min(b.len());
    let i = (0..n).find(|&i| a[i] != b[i]).unwrap_or(n);
    if i == n && a.len() == b.len() {
        return None;
    }
    let from = i.saturating_sub(context);
    let mut s = format!("traces differ at line {i} (lengths {} / {}):\n", a.len(), b.len());
    for (j, l) in a.iter().enumerate().skip(from).take(context + 6) {
        s += &format!("{} A {l}\n", if j == i { ">" } else { " " });
    }
    s += "---\n";
    for (j, l) in b.iter().enumerate().skip(from).take(context + 6) {
        s += &format!("{} B {l}\n", if j == i { ">" } else { " " });
    }
    Some(s)
}

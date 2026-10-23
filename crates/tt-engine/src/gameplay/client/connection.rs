//! The loading screen is the shell's ([`crate::shell::Shell::loading`]).

use crate::canvas::{self, Ctrl};
use crate::host::EngineHost;
use crate::net::{ClientCb, Conn};
use crate::sched::Event;
use tt_gameplay::client::value::{fs, Val};
use tt_gameplay::ClientCmd;

/// A callback of the client's connection.
pub fn callback(h: &mut EngineHost, cb: ClientCb) {
    match cb {
        ClientCb::ConnectionAccepted => on_connection_accepted(h),
        ClientCb::InitialControlSet => initial_control_set(h),
        ClientCb::ConnectionDropped(r) => conn_error(h, &format!("The server has dropped the connection: {r}")),
        ClientCb::ConnectionTimedOut => conn_error(h, "The server connection has timed out."),
        ClientCb::ConnectionError(r) => {
            let m = h.client.conn_error.clone();
            conn_error(h, &format!("{m} ({r})"));
        }
        ClientCb::ConnectRequestRejected(r) => on_connect_request_rejected(h, &r),
        ClientCb::ConnectRequestTimedOut => conn_error(h, "Your connection to the server timed out."),
        ClientCb::DefReceived(i, n) => {
            let p = fs(f64::from(i) / f64::from(n));
            on_phase1_progress(h, &p);
        }
        ClientCb::ReplicasStarted(n) => {
            h.cl.replica_total = n;
            h.cl.replicas_received = 0.0;
        }
        ClientCb::ReplicaReceived => {
            h.cl.replicas_received = (h.cl.replicas_received as f64 + 1.0) as f32;
            let p = fs(h.cl.replicas_received as f64 / f64::from(h.cl.replica_total));
            set_loading_progress(h, &p);
        }
        ClientCb::SceneLightingComplete => scene_lighting_complete(h),
    }
}

/// `MsgConnectionError`: the reason of a connection error to come.
pub fn handle_connection_error_message(h: &mut EngineHost, text: &str) {
    h.client.conn_error = text.to_string();
}

/// `initialControlSet`: the game screen, and target practice's welcome.
pub fn initial_control_set(h: &mut EngineHost) {
    h.log.info("*** Initial Control Object");
    if h.canvas.content() != Some(Ctrl::PlayGui) {
        canvas::set_content(h, Ctrl::PlayGui);
    }
    if h.srv.state().setup.target_range() {
        crate::shell::message_box(
            h,
            "Welcome to target practice.\nIn target practice you can shoot at tanks and they won't shoot back.  If you want an easy target shoot a big tank: the little ones are much harder to hit because they run away.  Destroy 5 tanks in order to succeed.  You can press 'ESC' at any time to leave target practice.",
            crate::shell::MessageKind::Ok(crate::shell::MsgAction::Resume),
        );
        h.cl.time_scale = Some(Val::f(0.00001));
    }
}

/// `setLagIcon(on)` of the client's connection (not on the local one).
pub fn set_lag_icon(h: &mut EngineHost, conn: Conn, on: bool) {
    if crate::net::address(h, conn).eq_ignore_ascii_case("local") {
        return;
    }
    h.client.lag = on;
}

pub fn on_connection_accepted(h: &mut EngineHost) {
    h.client.connection_accepted();
}

/// `connErrorGui.show(msg)`.
fn conn_error(h: &mut EngineHost, msg: &str) {
    crate::shell::show_conn_error(h, msg);
}

pub fn on_connect_request_rejected(h: &mut EngineHost, msg: &str) {
    let m = msg.to_ascii_uppercase();
    let error = match m.as_str() {
        "CR_INVALID_PROTOCOL_VERSION" | "CHR_PROTOCOL" => "Incompatible protocol version: Your game version is not compatible with this server.".to_string(),
        "CR_INVALID_CONNECT_PACKET" => "Internal Error: badly formed network packet".to_string(),
        "CR_YOUAREBANNED" => "You are not allowed to play on this server.".to_string(),
        "CR_SERVERFULL" => "This server is full.".to_string(),
        "CHR_PASSWORD" => {
            if h.cl.password.is_empty() {
                conn_error(h, "That server requires a password.");
            } else {
                h.cl.password.clear();
                conn_error(h, "That password is incorrect.");
            }
            return;
        }
        "CHR_CLASSCRC" => "Incompatible game classes: Your game version is not compatible with this server.".to_string(),
        "CHR_INVALID_CHALLENGE_PACKET" => "Internal Error: Invalid server response packet".to_string(),
        _ => format!("Connection error.  Please try another server.  Error code: ({msg})"),
    };
    conn_error(h, &error);
}

/// `disconnect()`: the connection, the server, the round state.
pub fn disconnect(h: &mut EngineHost) {
    if let Some(c) = h.net.server_connection() {
        crate::net::delete_conn(h, c);
    }
    disconnected_cleanup(h);
    crate::gameplay::server_host::with_server(h, |sv, b| sv.destroy_server(b));
    let round = &mut h.srv.state_mut().round;
    let timer = round.end_timer.take();
    round.running = Some(false);
    round.cycling = Some(false);
    if let Some(id) = timer {
        h.sched.cancel(id.0);
    }
    if h.srv.state().setup.demo() {
        crate::shell::quit(h);
    }
}

pub fn disconnected_cleanup(h: &mut EngineHost) {
    h.audio.stop_all();
    if h.cl.music_player {
        h.audio.music_stop();
    }
    h.client.disconnected();
    h.cl.vote_menu_wanted = false;
    match h.cl.disconnect_gui.take() {
        Some(gui) => {
            if h.canvas.content() != Some(gui) {
                crate::shell::show(h, gui, "");
            }
        }
        None => crate::shell::show(h, Ctrl::JoinGameGui, ""),
    }
    switch_to_shell_resolution(h);
}

/// `switchToShellResolution()`: the shell's screen mode, if the game's differs.
pub fn switch_to_shell_resolution(h: &mut EngineHost) {
    let p = &h.shell.prefs;
    let (sim, shell) = (p.get("video.sim_resolution"), p.get("video.shell_resolution"));
    if tt_game::fmt::stricmp(&sim, &shell) == 0 {
        return;
    }
    let word = |i: usize| tt_shell::vars::get_word(&shell, i);
    let full = tt_game::fmt::atoi(&p.get("video.full_screen")) != 0;
    let (w, ht, d) = (tt_game::fmt::atoi(&word(0)), tt_game::fmt::atoi(&word(1)), tt_game::fmt::atoi(&word(2)));
    crate::video::set_mode(h, w, ht, d, Some(full));
}

// ---- the mission download -------------------------------------------------------------

pub fn mission_start_phase1(h: &mut EngineHost, seq: &str, mission: &str, music: &str) {
    h.log.info(&format!("*** New Mission: {mission}"));
    h.log.info("*** Phase 1: Download Definitions & Targets");
    // onMissionDownloadPhase1
    super::hud::close_chat(h);
    set_loading_progress(h, "0");
    h.shell.loading.phase = "Loading defs...".into();
    if h.cl.music_player {
        // `MusicPlayer.setTrack(track)`.
        let name = h.audio.music_name(music).unwrap_or_else(|| music.to_string());
        h.audio.music_set_track(&name);
    }
    crate::net::send_cmd(h, ClientCmd::PhaseAck { phase: 1, seq: seq.to_string() });
}

/// `onPhase1Progress(p)`: on the loading screen.
fn on_phase1_progress(h: &mut EngineHost, p: &str) {
    if h.canvas.content() == Some(Ctrl::LoadingGui) {
        set_loading_progress(h, p);
    }
}

pub fn mission_start_phase2(h: &mut EngineHost, seq: &str) {
    h.log.info("*** Phase 2: Download Mission Objects");
    loading_phase(h, "Loading objects...");
    crate::net::send_cmd(h, ClientCmd::PhaseAck { phase: 2, seq: seq.to_string() });
}

/// The lighting (done by the renderer: its callback comes on the next
/// tick).
pub fn mission_start_phase3(h: &mut EngineHost, seq: &str, mission: &str) {
    h.log.info("*** Phase 3: Mission Lighting");
    h.cl.mission_seq = seq.to_string();
    h.cl.mission_file = mission.to_string();
    crate::net::light_scene(h);
    h.log.error("Lighting mission....");
    h.sched.post(1, Event::UpdateLightingProgress);
    loading_phase(h, "Lighting mission...");
    h.cl.lighting_mission = true;
}

/// `updateLightingProgress()`: every millisecond while lighting.
pub fn update_lighting_progress(h: &mut EngineHost) {
    // Is "1" (the renderer lights at once).
    set_loading_progress(h, "1");
    if h.cl.lighting_mission {
        h.sched.post(1, Event::UpdateLightingProgress);
    }
}

fn scene_lighting_complete(h: &mut EngineHost) {
    h.log.info("Mission lighting done");
    set_loading_progress(h, "1");
    h.cl.lighting_mission = false;
    let seq = h.cl.mission_seq.clone();
    crate::net::send_cmd(h, ClientCmd::PhaseAck { phase: 3, seq });
}

/// `onMissionDownloadPhase2/3`: progress 0, the phase text.
fn loading_phase(h: &mut EngineHost, text: &str) {
    set_loading_progress(h, "0");
    h.shell.loading.phase = text.to_string();
}

/// `MsgLoadInfo`: the mission name on the loading screen.
pub fn handle_load_info_message(h: &mut EngineHost, name: &str) {
    h.shell.loading.map = name.to_string();
}

/// `setLoadingProgress(v)`.
fn set_loading_progress(h: &mut EngineHost, v: &str) {
    h.shell.loading.progress = (tt_game::fmt::atof(v) as f32).clamp(0.0, 1.0);
    h.client.end_game.loading = v.to_string();
}

//! The in-game HUD's client logic: message lines and their sounds, the chat
//! input, the player list's messages, and `PlayGui`'s wake. The state is
//! the native client state ([`tt_gameplay::ClientState`],
//! `EngineHost::client`); the chat input and the player list are the canvas
//! dialogs `MessageHud` and `PlayerListGui` (on the canvas = shown).

use crate::canvas::{self, Ctrl};
use crate::host::EngineHost;
use tt_gameplay::client::players::Join;
use tt_gameplay::client::text::detag;
use tt_gameplay::client::value::{f32r, lf, lu, sf, Val};
use tt_gameplay::ClientCmd;

/// The simulation clock (ms), as the scripts' `schedule` used it.
pub fn now(h: &EngineHost) -> u64 {
    h.sim_time() as u64
}

/// `Canvas.getContent() == PlayGui.getId()`: the game screen is up.
pub fn in_play(h: &EngineHost) -> bool {
    h.canvas.content() == Some(Ctrl::PlayGui)
}

/// As a condition.
fn single_player(h: &EngineHost) -> bool {
    h.srv.state().setup.single_player()
}


const MAX_MESSAGE_WAV_MS: f64 = 5000.0;

/// `playMessageSound(message, voice, pitch)`: plays the `~w` wave of a
/// message (one at a time); returns the position of `~w`, or -1.
fn play_message_sound(h: &mut EngineHost, message: &str, voice: &str, pitch: &str) -> String {
    let (Some(snd), _) = tt_gameplay::client::messages::message_sound(message, voice, pitch) else { return "-1".into() };
    let pitch = snd.pitch.as_deref().map(lf).unwrap_or(1.0);
    let len_ms = f32r(pitch * h.audio.wave_len_ms(&snd.file) as f64);
    if len_ms == 0.0 {
        h.log.error(&format!("** WAV file \"{}\" is nonexistent or sound is zero-length **", snd.file));
    } else if len_ms <= MAX_MESSAGE_WAV_MS {
        if let Some(old) = h.client.chat_sound.take().filter(|v| sf(v) != 0.0) {
            h.audio.stop(tt_game::fmt::atoi(&old) as u32);
        }
        let src = h.audio.create_source("AudioMessage", Some(&snd.file), None);
        if pitch != 1.0 {
            let p = snd.pitch.as_deref().unwrap_or("1");
            h.audio.source_f(src, tt_audio::AlParam::Pitch, tt_game::fmt::atof(p) as f32);
        }
        h.audio.play(src);
        h.client.chat_sound = Some(src.to_string());
    } else {
        h.log.error(&format!("** WAV file \"{}\" is too long **", snd.file));
    }
    tt_gameplay::client::text::strstr(message, "~w").to_string()
}

/// A message arriving: its sound, then its lines (`chat`: a chat line).
fn show_message(h: &mut EngineHost, message: &str, voice: &str, pitch: &str, chat: bool) {
    let start = play_message_sound(h, message, voice, pitch);
    let text = if sf(&start) != -1.0 { tt_gameplay::client::text::sub_str(message, "0", &start) } else { message.to_string() };
    let (play, t) = (in_play(h), now(h));
    h.client.add_message(&text, chat, play, t);
}

/// `onChatMessage(message, voice, pitch)`: a chat line.
pub fn chat_message(h: &mut EngineHost, message: &str, voice: &str, pitch: &str) {
    let voice = if voice.is_empty() { "default" } else { voice };
    show_message(h, message, voice, pitch, true);
}

/// `onServerMessage(message)`: a server message line.
pub fn server_message(h: &mut EngineHost, message: &str) {
    show_message(h, message, "", "", false);
}

/// The chat HUD length pref, 1..3.
pub fn next_chat_hud_len(h: &mut EngineHost) {
    let p = &mut h.shell.prefs;
    let v = Val::f(pref_float(p, "hud.chat_length") + 1.0);
    let v = if v.float() == 4.0 { Val::Int(1) } else { v };
    crate::shell::set_pref_val(p, "hud.chat_length", v);
}

/// A pref read as a number (`LOADVAR_FLT`).
pub fn pref_float(p: &tt_shell::Prefs, name: &str) -> f64 {
    crate::shell::pref_val(p, name).map(|v| v.float()).unwrap_or(0.0)
}


/// `MessageHud::open`: the chat input on the canvas.
pub fn open_chat(h: &mut EngineHost) {
    if h.client.chat.open {
        return;
    }
    canvas::push_dialog(h, Ctrl::MessageHud, 0);
    h.client.open_chat();
}

/// `MessageHud::close`.
pub fn close_chat(h: &mut EngineHost) {
    if !h.client.chat.open {
        return;
    }
    canvas::pop_dialog(h, Ctrl::MessageHud);
    h.client.close_chat();
}

fn toggle_chat(h: &mut EngineHost) {
    if h.client.chat.open {
        close_chat(h);
    } else {
        open_chat(h);
    }
}

/// `MessageHud_Edit::eval`: the typed line to the server, then closed.
pub fn send_chat(h: &mut EngineHost, text: &str) {
    if !text.is_empty() {
        let text = text.to_string();
        let cmd = if h.client.chat.team { ClientCmd::TeamMessageSent { text } } else { ClientCmd::MessageSent { text } };
        crate::net::send_cmd(h, cmd);
    }
    close_chat(h);
}

/// `toggleMessageHud(val)` / `teamMessageHud(val)`.
pub fn message_hud_key(h: &mut EngineHost, val: &str, team: bool) {
    if lu(val) != 0 {
        h.client.chat.team = team;
        toggle_chat(h);
    }
}


/// The messages, the player list in multiplayer, the game's action map.
pub fn play_gui_on_wake(h: &mut EngineHost) {
    let sp = single_player(h);
    let t = now(h);
    h.client.messages.visible = true;
    h.client.play_wake(sp, t);
    if !sp {
        canvas::push_dialog(h, Ctrl::PlayerListGui, 0);
    }
    h.input.push_map(crate::input::MOVE_MAP);
}


/// A server message's type callbacks by the type's text; `args` = (type,
/// text, arguments...).
pub fn message_callback(h: &mut EngineHost, kind: &str, args: &[String]) {
    let a = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    match kind {
        "MsgClientJoin" => handle_client_join(h, args),
        "MsgClientDrop" => h.client.players.remove(a(3)),
        "MsgClientScoreChanged" => {
            h.client.players.update_score(a(4), a(2), a(3));
        }
        "MsgTeamScoreChanged" => {
            if !single_player(h) {
                h.client.players.update_team_score(a(4), a(2), a(3));
            }
        }
        "MsgAddTeam" => {
            if !single_player(h) {
                h.client.players.add_team(a(2), a(3), a(4), a(5));
            }
        }
        "MsgConnectionError" => super::connection::handle_connection_error_message(h, a(2)),
        "MsgLoadInfo" => super::connection::handle_load_info_message(h, a(2)),
        _ => {}
    }
}

fn handle_client_join(h: &mut EngineHost, args: &[String]) {
    let a = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    let (msg_string, client_name, client_id, team) = (a(1), a(2), a(3), a(4));
    let team_name = detag(team);
    let sp = single_player(h);
    h.client.set_join_msg(&detag(msg_string), &team_name, sp);
    let name = detag(client_name);
    let j = Join { id: client_id, name: &name, is_super: a(10), is_admin: a(9), score: a(6), cum_score: a(7), team: &team_name };
    h.client.players.update(&j);
}

/// Tab: the player list on or off the canvas (`PlayerListGui::toggle`).
pub fn toggle_player_list(h: &mut EngineHost) {
    if h.canvas.is_awake(Ctrl::PlayerListGui) {
        canvas::pop_dialog(h, Ctrl::PlayerListGui);
    } else {
        canvas::push_dialog(h, Ctrl::PlayerListGui, 0);
    }
}

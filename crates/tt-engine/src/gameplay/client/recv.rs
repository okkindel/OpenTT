//! The local client gets the server's messages as the next "packet"
//! (`crate::net::Deferred::ServerMsg`), a remote server's arrive over the
//! network (`crate::netgame`); both end here.

use crate::host::EngineHost;
use tt_gameplay::{HudMsg, ServerMsg};

/// The effect of `msg` on this process's client.
pub fn apply(h: &mut EngineHost, msg: &ServerMsg) {
    let r = msg.received();
    let a: Vec<String> = r.iter().map(|x| x.text.clone()).collect();
    let t = |i: usize| a.get(i).cloned().unwrap_or_default();
    match msg {
        ServerMsg::Message { .. } => {
            // The message's line, then the handler of its type (only a tagged
            // type has one).
            let kind = r.first().filter(|k| k.tag).map(|k| k.text.clone());
            let forward: Vec<String> = (0..12).map(&t).collect();
            super::hud::server_message(h, &forward[1]);
            if let Some(kind) = kind {
                super::hud::message_callback(h, &kind, &forward);
            }
        }
        ServerMsg::Chat { .. } => super::hud::chat_message(h, &t(3), &t(1), &t(2)),
        ServerMsg::CenterPrint { .. } => {
            let now = h.sim_time() as u64;
            h.client.center_print(&t(0), &t(1), &t(2), now);
        }
        ServerMsg::BottomPrint { .. } => {
            let now = h.sim_time() as u64;
            h.client.bottom_print(&t(0), &t(1), &t(2), now);
        }
        ServerMsg::ClearCenterPrint => h.client.clear_center(),
        ServerMsg::ClearBottomPrint => h.client.clear_bottom(),
        ServerMsg::SyncClock { .. } => super::game::sync_clock(h, &t(0), &t(1)),
        ServerMsg::MissionStart { .. } => {}
        ServerMsg::MissionEnd { .. } => {
            // Sounds off, no lighting.
            h.audio.stop_all();
            h.cl.lighting_mission = false;
        }
        ServerMsg::MissionPhase1 { .. } => super::connection::mission_start_phase1(h, &t(0), &t(1), &t(2)),
        ServerMsg::MissionPhase2 { .. } => super::connection::mission_start_phase2(h, &t(0)),
        ServerMsg::MissionPhase3 { .. } => super::connection::mission_start_phase3(h, &t(0), &t(1)),
        ServerMsg::GameStart => super::game::game_start(h),
        ServerMsg::GameEnd { .. } => super::game::game_end(h, &t(0)),
        ServerMsg::Hud(m) => hud(h, m),
        ServerMsg::VoteStatus { map, yes, no, players, secs, mine } => {
            let now = h.sim_time() as u64;
            h.client.vote_status(map, *yes, *no, *players, *secs, *mine, now);
        }
        ServerMsg::MapList { maps } => super::hud::vote_maps(h, maps),
    }
}

/// A client-side effect of the server ([`HudMsg`]): the round clock, the
/// solo widgets, sounds, the end of target practice and of a solo run.
pub fn hud(h: &mut EngineHost, msg: &HudMsg) {
    use HudMsg::*;
    let now = h.sim_time() as u64;
    let c = &mut h.client;
    match msg {
        ClockShowTime(v) => c.clock.show_time = v.clone(),
        ClockSetTime(v) => c.clock.set_time(v, now),
        ClockVisible(v) => c.clock.visible = tt_gameplay::client::hud::visible_value(v),
        BrainsMax(v) => c.solo.max_brains = v.clone(),
        BrainsCount(v) => c.solo.set_brain_count(v),
        BrainsVisible(v) => c.solo.brains_visible = tt_gameplay::client::hud::visible_value(v),
        Score(v) => c.solo.set_score(v),
        Lives(v) => c.solo.show_lives(v),
        Sound(p) => {
            // `alxPlay(profile)`: a profile by name, else a source handle.
            match h.audio.profile_name(p) {
                Some(name) => {
                    h.audio.play_profile(&name, None);
                }
                None => {
                    h.audio.play(tt_game::fmt::atoi(p) as u32);
                }
            }
        }
        TargetPracticeDone => {
            h.cl.time_scale = Some(tt_gameplay::client::value::Val::f(0.00001));
            crate::shell::set_pref_val(&mut h.shell.prefs, "player.finished_tutorial", tt_gameplay::client::value::Val::Int(1));
            crate::shell::message_box(h, TARGET_DONE_TEXT, crate::shell::MessageKind::YesNo(crate::shell::MsgAction::Resume, crate::shell::MsgAction::DisconnectResume));
        }
        SoloGameOver => {
            // `soloPlayGui.schedule(0, endGame)` (the solo screen of the menu).
            if h.shell.screens {
                h.sched.post(0, crate::sched::Event::SoloEndGame);
            }
        }
    }
}

const TARGET_DONE_TEXT: &str = "Congratulations!  You have succeeded in destroying 5 tanks.  You can continue with target practice or return to the main menu.\n\nWould you like to continue with target practice?";

//! Client commands: the mission download's acknowledgements, respawn, the
//! AI toggle, chat and the map votes ([`super::vote`]).

use super::{ClientId, Server, ServerHost, Setting};
use crate::msg::{ClientCmd, DebugCmd, HudMsg, Value};

impl Server {
    /// A command from client `c`.
    pub fn client_cmd(&mut self, h: &mut dyn ServerHost, c: ClientId, cmd: &ClientCmd) {
        match cmd {
            ClientCmd::PhaseAck { phase, seq } => self.phase_ack(h, c, *phase, seq),
            ClientCmd::SpawnPlayer => self.respawn_request(h, c),
            ClientCmd::MessageSent { text } => self.message_sent(h, c, text, false),
            ClientCmd::TeamMessageSent { text } => self.message_sent(h, c, text, true),
            ClientCmd::Debug { cmd: DebugCmd::TogglePlayerAI, .. } => self.toggle_player_ai(h, c),
            ClientCmd::Debug { .. } => {}
            ClientCmd::CallVote { map } => self.call_vote(h, c, map),
            ClientCmd::CastVote { yes } => self.cast_vote(h, c, *yes),
            ClientCmd::MapList => self.send_map_list(h, c),
        }
    }

    /// Game over (solo, no lives) or a new tank for a dead one.
    pub fn respawn_request(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        if self.setup.single_player() && self.rec(Some(c), |r| r.lives.unwrap_or(0)) == 0 {
            h.hud(Some(c), &HudMsg::SoloGameOver);
        } else if let Some(p) = self.client(c).and_then(|x| x.player) {
            if h.entity_exists(p) && h.is_corpse(p) {
                self.clients.get_mut(&c).unwrap().player = None;
                self.spawn_player(h, c);
            }
        }
    }

    /// The player's tank drives itself or not (development builds, the demo).
    pub fn toggle_player_ai(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        if !h.is_release() || self.setup.demo() {
            let Some(x) = self.clients.get_mut(&c) else { return };
            x.using_ai = !x.using_ai;
            let (on, player) = (x.using_ai, x.player);
            if self.tank_exists(h, player) {
                let p = player.unwrap();
                let ai = if on { h.tank_def(p).and_then(|t| h.data().tanks[t].ai) } else { None };
                h.set_ai(p, ai);
            }
        }
    }

    /// A chat line cut to the longest allowed.
    fn chat_line(&self, h: &dyn ServerHost, text: &str) -> String {
        if text.chars().count() as f64 >= self.setting_num(h, Setting::MaxChatLen) {
            let max = tt_game::fmt::atoi(&h.setting(Setting::MaxChatLen));
            if max < 0 {
                return String::new();
            }
            return text.chars().take(max as usize).collect();
        }
        text.to_string()
    }

    /// A chat line from a client (to everyone or its team).
    pub fn message_sent(&mut self, h: &mut dyn ServerHost, c: ClientId, text_: &str, team: bool) {
        let t = self.chat_line(h, text_);
        let args = vec![self.name_value(Some(c)), Value::Str(t)];
        if team {
            self.chat_message_team(h, c, &Value::tag("\u{4}%1: %2"), &args);
        } else {
            self.chat_message_all(h, c, &Value::tag("\u{5}%1: %2"), &args);
        }
    }
}

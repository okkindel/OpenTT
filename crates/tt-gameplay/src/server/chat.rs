//! Messages to every client, a team, all but one; chat and its flood
//! protection; center and bottom prints; sounds for every client.
//!
//! Team messages and team chat reach only the clients on that team (the
//! sender is on its own team, so it sees its line too). A client without a
//! team -- every client of a free-for-all game, or one not (yet) on a team
//! in a team game -- shares the "no team" group: team chat in a
//! free-for-all game reaches everyone, and a teamless client in a team
//! game talks only to the other teamless ones (an observer's chat never
//! reaches the playing teams). The game sent them to everyone (it compared
//! team ids with team names, which never matched); the port kept that
//! until 2026-10-18 for its golden traces.

use super::state::*;
use super::*;
use crate::data::*;
use crate::msg::*;
use tt_game::fmt::atof;

/// The flood protection notice.
pub const SPAM_MESSAGE: &str = "\u{4}FLOOD PROTECTION:\u{f} You must wait another %1 seconds.";

/// A server message with `kind`, `text` and the kind's arguments.
pub fn server_message(kind: &MsgKind, text: &Value) -> ServerMsg {
    let mut v = vec![kind.kind_value(), text.clone()];
    v.extend(kind.args());
    message_from(v)
}

/// A server message from its arguments (padded to the full count).
fn message_from(mut v: Vec<Value>) -> ServerMsg {
    while v.len() < MESSAGE_ARGS {
        v.push(Value::empty());
    }
    ServerMsg::parse("ServerMessage", &v).expect("a server message")
}

/// A print's line count: 1 to 3 (anything else is 1).
pub fn print_lines(l: &str) -> String {
    let n = atof(l) as f32;
    if l.is_empty() || n > 3.0 || n < 1.0 {
        "1".into()
    } else {
        l.to_string()
    }
}

impl Server {
    /// One message to one client.
    pub fn send_to(&mut self, h: &mut dyn ServerHost, to: Option<ClientId>, msg: &ServerMsg) {
        h.send(to, msg);
    }

    /// A message to one client.
    pub fn message_client(&mut self, h: &mut dyn ServerHost, c: ClientId, kind: &MsgKind, text: &Value) {
        h.send(Some(c), &server_message(kind, text));
    }

    /// A message to every client.
    pub fn message_all(&mut self, h: &mut dyn ServerHost, kind: &MsgKind, text: &Value) {
        let msg = server_message(kind, text);
        for c in self.group.clone() {
            h.send(Some(c), &msg);
        }
    }

    /// A message to every client but `client`.
    pub fn message_all_except(&mut self, h: &mut dyn ServerHost, client: ClientId, kind: &MsgKind, text: &Value) {
        let msg = server_message(kind, text);
        for r in self.group.clone() {
            if r != client {
                h.send(Some(r), &msg);
            }
        }
    }

    /// The clients on `team` (`None`: the clients without a team), see the
    /// module docs.
    pub fn team_members(&self, team: Option<TeamColor>) -> Vec<ClientId> {
        self.group.iter().copied().filter(|c| self.team_of(*c) == team).collect()
    }

    /// A message to a team (see the module docs).
    pub fn message_team(&mut self, h: &mut dyn ServerHost, team: Option<TeamColor>, kind: &MsgKind, text: &Value) {
        let msg = server_message(kind, text);
        for c in self.team_members(team) {
            h.send(Some(c), &msg);
        }
    }

    /// A message to `client`'s team but `client`.
    pub fn message_team_except(&mut self, h: &mut dyn ServerHost, client: ClientId, kind: &MsgKind, text: &Value) {
        let msg = server_message(kind, text);
        for c in self.team_members(self.team_of(client)) {
            if c != client {
                h.send(Some(c), &msg);
            }
        }
    }

    /// A chat line from `sender` to `obj`.
    pub(crate) fn chat_to(&mut self, h: &mut dyn ServerHost, obj: ClientId, sender: ClientId, text: &Value, args: &[Value]) {
        let mut v = vec![Value::str(sender.text()), Value::empty(), Value::empty(), text.clone()];
        v.extend(args.iter().cloned());
        v.truncate(CHAT_ARGS);
        while v.len() < CHAT_ARGS {
            v.push(Value::empty());
        }
        if let Some(m) = ServerMsg::parse("ChatMessage", &v) {
            h.send(Some(obj), &m);
        }
    }

    /// Nothing to say, or the sender is flood-protected.
    pub(crate) fn chat_blocked(&mut self, h: &mut dyn ServerHost, sender: ClientId, text: &Value) -> bool {
        text.text().is_empty() || self.flood_check(h, sender)
    }

    /// Chat to everyone.
    pub fn chat_message_all(&mut self, h: &mut dyn ServerHost, sender: ClientId, text: &Value, args: &[Value]) {
        if self.chat_blocked(h, sender, text) {
            return;
        }
        for obj in self.group.clone() {
            self.chat_to(h, obj, sender, text, args);
        }
    }

    /// Team chat: to the sender's team (see the module docs).
    pub fn chat_message_team(&mut self, h: &mut dyn ServerHost, sender: ClientId, text: &Value, args: &[Value]) {
        if self.chat_blocked(h, sender, text) {
            return;
        }
        for obj in self.team_members(self.team_of(sender)) {
            self.chat_to(h, obj, sender, text, args);
        }
    }

    /// Whether the client is flood-protected (it is told how long it has
    /// to wait): after [`flood::THRESHOLD`] messages within
    /// [`flood::PERIOD_MS`] it is muted for [`flood::PENALTY_MS`].
    pub fn flood_check(&mut self, h: &mut dyn ServerHost, client: ClientId) -> bool {
        if self.setting_num(h, Setting::FloodProtection) != 1.0 {
            return false;
        }
        let Some(c) = self.clients.get_mut(&client) else { return false };
        if !c.flood.muted && c.flood.count >= flood::THRESHOLD {
            c.flood.since = h.sim_time() as i64;
            c.flood.muted = true;
            self.timers.schedule(h, flood::PENALTY_MS, Timer::Unmute(client));
        }
        let f = self.clients[&client].flood;
        if f.muted {
            let left_ms = flood::PENALTY_MS as i64 - (h.sim_time() as i64 - f.since);
            let wait = left_ms.div_euclid(1000).to_string();
            let m = message_from(vec![Value::empty(), Value::tag(SPAM_MESSAGE), Value::str(wait)]);
            h.send(Some(client), &m);
            return true;
        }
        self.clients.get_mut(&client).unwrap().flood.count += 1;
        self.timers.schedule(h, flood::PERIOD_MS, Timer::ForgetMessage(client));
        false
    }

    pub fn forget_message(&mut self, c: ClientId) {
        if let Some(c) = self.clients.get_mut(&c) {
            if c.flood.count > 0 {
                c.flood.count -= 1;
            }
        }
    }

    pub fn unmute(&mut self, c: ClientId) {
        if let Some(c) = self.clients.get_mut(&c) {
            c.flood.muted = false;
        }
    }

    /// To every client.
    pub(crate) fn send_humans(&mut self, h: &mut dyn ServerHost, msg: &ServerMsg) {
        for c in self.group.clone() {
            h.send(Some(c), msg);
        }
    }

    pub fn center_print_all(&mut self, h: &mut dyn ServerHost, text: &str, time: &str, lines: &str) {
        let msg = ServerMsg::CenterPrint { text: Value::str(text), secs: time.into(), lines: print_lines(lines), lc: true };
        self.send_humans(h, &msg);
    }

    pub fn bottom_print_all(&mut self, h: &mut dyn ServerHost, text: &str, time: &str, lines: &str) {
        let msg = ServerMsg::BottomPrint { text: Value::str(text), secs: time.into(), lines: print_lines(lines), lc: true };
        self.send_humans(h, &msg);
    }

    pub fn center_print(&mut self, h: &mut dyn ServerHost, c: Option<ClientId>, text: &Value, time: &str, lines: &str) {
        h.send(c, &ServerMsg::CenterPrint { text: text.clone(), secs: time.into(), lines: print_lines(lines), lc: false });
    }

    pub fn bottom_print(&mut self, h: &mut dyn ServerHost, c: Option<ClientId>, text: &Value, time: &str, lines: &str) {
        h.send(c, &ServerMsg::BottomPrint { text: text.clone(), secs: time.into(), lines: print_lines(lines), lc: false });
    }

    pub fn clear_center_print(&mut self, h: &mut dyn ServerHost, c: Option<ClientId>) {
        h.send(c, &ServerMsg::ClearCenterPrint);
    }

    pub fn clear_bottom_print(&mut self, h: &mut dyn ServerHost, c: Option<ClientId>) {
        h.send(c, &ServerMsg::ClearBottomPrint);
    }

    pub fn clear_center_print_all(&mut self, h: &mut dyn ServerHost) {
        self.send_humans(h, &ServerMsg::ClearCenterPrint);
    }

    pub fn clear_bottom_print_all(&mut self, h: &mut dyn ServerHost) {
        self.send_humans(h, &ServerMsg::ClearBottomPrint);
    }

    /// The clients that can hear (fake clients cannot).
    pub(crate) fn sound_targets(&self) -> Vec<ClientId> {
        self.group.iter().copied().filter(|c| self.kind(*c).is_some_and(|k| k != ClientKind::Fake)).collect()
    }

    /// A sound for every client.
    pub fn server_play_2d(&mut self, h: &mut dyn ServerHost, sound: Ref<SoundDef>) {
        for c in self.sound_targets() {
            h.play2d(c, sound);
        }
    }

    /// A sound at a place for every client.
    pub fn server_play_3d(&mut self, h: &mut dyn ServerHost, sound: Ref<SoundDef>, transform: &str) {
        for c in self.sound_targets() {
            h.play3d(c, sound, transform);
        }
    }

    /// A message of any type (the console, tests): `kind` a type tag (""
    /// for none).
    pub fn message_of(kind: &str, text: &str, args: &[String]) -> (MsgKind, Value) {
        let mut v = vec![if kind.is_empty() { Value::empty() } else { Value::tag(kind) }, Value::tag(text)];
        v.extend(args.iter().map(|a| Value::str(a.clone())));
        match message_from(v) {
            ServerMsg::Message { kind, text } => (kind, text),
            _ => unreachable!(),
        }
    }
}

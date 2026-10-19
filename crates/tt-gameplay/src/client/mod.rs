//! The engine feeds it the server's messages and the player's input and
//! reads it to draw the HUD.
//!
//! Times are the simulation clock in ms (`now`), as the scripts' `schedule`
//! used it; [`ClientState::advance`] runs the timers due by then (in due
//! time, then creation order, like the interpreter's event queue) and
//! [`ClientState::animate`] steps the fades by a GUI tick.

pub mod binds;
pub mod hud;
pub mod messages;
pub mod players;
pub mod text;
pub mod value;

use std::collections::BTreeMap;

pub use binds::{BindAction, Binding, Binds, Device, MoveState};
pub use hud::{HudClock, Print, SoloHud};
pub use messages::{Line, MessageHud, MessageSound};
pub use players::{EndGame, Join, Kda, PlayerList, RowView, TeamIcon};

/// What a client timer does when due.
#[derive(Debug, Clone, PartialEq)]
enum Timer {
    /// `messageScroll_addMessage2`: a message line appears.
    AddLine { text: String, chat: bool },
    /// `messagesHud.remove(line)`.
    RemoveLine(u64),
    ClearCenter,
    ClearBottom,
    /// Join message.
    JoinPrint(String),
}

/// The chat input (`MessageHud` with `MessageHud_Edit`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatInput {
    /// `MessageHud.visible` (open).
    pub open: bool,
    /// `MessageHud.isTeamMsg`.
    pub team: bool,
    /// The edit's text.
    pub text: String,
}

/// `MessageHud_Edit.maxLength`.
pub const CHAT_MAX_LENGTH: usize = 120;

/// What a key does to the open chat input.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatKey {
    /// Not for the input (the action maps get it).
    Pass,
    /// Taken (typed, deleted or ignored).
    Used,
    /// Return: send this text (trimmed, maybe empty) and close.
    Send(String),
    /// Escape: close.
    Close,
}

#[derive(Debug, Clone, Default)]
pub struct ClientState {
    pub messages: MessageHud,
    pub center: Print,
    pub bottom: Print,
    pub clock: HudClock,
    pub solo: SoloHud,
    pub players: PlayerList,
    pub end_game: EndGame,
    pub chat: ChatInput,
    /// `LagIcon` visible.
    pub lag: bool,
    /// `PlayGui.joinMsg`: the welcome line shown when the game screen wakes.
    pub join_msg: String,
    pub moves: MoveState,
    /// The audio source of the last message sound.
    pub chat_sound: Option<String>,
    /// A `MsgConnectionError`'s text.
    pub conn_error: String,
    timers: BTreeMap<(u64, u64), Timer>,
    seq: u64,
}

impl ClientState {
    fn schedule(&mut self, now: u64, delay: u32, t: Timer) -> u64 {
        self.seq += 1;
        self.timers.insert((now + delay as u64, self.seq), t);
        self.seq
    }

    fn cancel(&mut self, id: u64) {
        self.timers.retain(|&(_, seq), _| seq != id);
    }

    /// Pending timers (leak checks).
    pub fn pending_timers(&self) -> usize {
        self.timers.len()
    }

    /// Runs the timers due by `target` (sim ms), each at its due time.
    pub fn advance(&mut self, target: u64) {
        while let Some((&(due, seq), _)) = self.timers.iter().next() {
            if due > target {
                break;
            }
            let t = self.timers.remove(&(due, seq)).unwrap();
            self.fire(due, t);
        }
    }

    fn fire(&mut self, now: u64, t: Timer) {
        match t {
            Timer::AddLine { text, chat } => {
                let id = self.messages.add_line(&text, chat);
                self.schedule(now, messages::LINE_LIFETIME, Timer::RemoveLine(id));
            }
            Timer::RemoveLine(id) => self.messages.remove_line(id),
            Timer::ClearCenter => self.clear_center(),
            Timer::ClearBottom => self.clear_bottom(),
            Timer::JoinPrint(msg) => self.bottom_print(&msg, "4", "2", now),
        }
    }

    /// One GUI tick of the fades (`ms` of sim or real time).
    pub fn animate(&mut self, ms: u32) {
        self.messages.animate(ms);
    }

    /// `messagesHud.addMessage` / `addChatMessage` (already without the
    /// `~w` sound part): the lines of `text` appear one by one. Only while
    /// the game screen is up (`in_play`).
    pub fn add_message(&mut self, text: &str, chat: bool, in_play: bool, now: u64) {
        if !in_play {
            return;
        }
        for (i, line) in messages::wrap(text).into_iter().enumerate() {
            self.schedule(now, messages::line_delay(i), Timer::AddLine { text: line, chat });
        }
    }

    fn print(&mut self, center: bool, text: String, time: &str, now: u64) {
        let p = if center { &self.center } else { &self.bottom };
        let (active, remove) = (p.active, p.remove);
        if active {
            if let Some(id) = remove {
                self.cancel(id);
            }
        }
        let clear = if center { Timer::ClearCenter } else { Timer::ClearBottom };
        let id = hud::print_timeout(time).map(|ms| self.schedule(now, ms, clear));
        let p = if center { &mut self.center } else { &mut self.bottom };
        p.active = true;
        p.text = text;
        if id.is_some() {
            p.remove = id;
        }
    }

    pub fn center_print(&mut self, message: &str, time: &str, _lines: &str, now: u64) {
        self.print(true, message.to_string(), time, now);
    }

    pub fn bottom_print(&mut self, message: &str, time: &str, _lines: &str, now: u64) {
        self.print(false, format!("<just:center>{message}"), time, now);
    }

    pub fn clear_center(&mut self) {
        self.center.active = false;
        self.center.remove = None;
    }

    pub fn clear_bottom(&mut self) {
        self.bottom.active = false;
        self.bottom.remove = None;
    }

    /// `handleClientJoin`'s welcome line (`PlayGui.joinMsg`): the message
    /// text of a join, with the team in multiplayer team games.
    pub fn set_join_msg(&mut self, msg: &str, team: &str, single_player: bool) {
        self.join_msg.clear();
        if !single_player && text::strlen(msg) > 3 {
            let mut m = msg.to_string();
            if text::eq(team, "BlueTeam") {
                m += "  You are on the BLUE team.";
            } else if text::eq(team, "GreenTeam") {
                m += "  You are on the GREEN team.";
            }
            self.join_msg = m;
        }
    }

    /// HUD part: the solo widgets for single player, the pending welcome
    /// line as a bottom print 10 ms later.
    pub fn play_wake(&mut self, single_player: bool, now: u64) {
        let sp = single_player;
        self.solo.lives_visible = sp;
        self.solo.score_visible = sp;
        if text::strlen(&self.join_msg) > 3 {
            let msg = std::mem::take(&mut self.join_msg);
            self.schedule(now, 10, Timer::JoinPrint(msg));
        }
        self.join_msg.clear();
    }

    /// (multiplayer): the end-of-round lists.
    pub fn game_end(&mut self) {
        self.end_game.copy_from(&self.players);
    }

    /// Lag icon and brain counter off.
    pub fn connection_accepted(&mut self) {
        self.lag = false;
        self.solo.brains_visible = false;
    }

    /// `disconnectedCleanup`'s HUD part.
    pub fn disconnected(&mut self) {
        self.lag = false;
        self.players.clear();
        self.clear_bottom();
        self.clear_center();
    }

    /// A key while the chat input is open (`MessageHud_Edit` as the first
    /// responder): `text` is what the key types. Releases are not for it.
    pub fn chat_key(&mut self, key: &str, pressed: bool, typed: Option<&str>) -> ChatKey {
        if !self.chat.open || !pressed {
            return ChatKey::Pass;
        }
        match key {
            "return" | "numpadenter" => ChatKey::Send(text::trim(&self.chat.text)),
            "escape" => ChatKey::Close,
            "backspace" => {
                self.chat.text.pop();
                ChatKey::Used
            }
            "tab" | "up" | "down" | "f1" | "tilde" => ChatKey::Pass,
            _ => {
                if let Some(t) = typed.filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control())) {
                    for c in t.chars() {
                        if self.chat.text.chars().count() >= CHAT_MAX_LENGTH {
                            break;
                        }
                        self.chat.text.push(c);
                    }
                }
                ChatKey::Used
            }
        }
    }

    /// `MessageHud::open` (`team`: `isTeamMsg`).
    pub fn open_chat(&mut self) {
        self.chat.open = true;
    }

    /// `MessageHud::close`: closed and emptied.
    pub fn close_chat(&mut self) {
        self.chat.open = false;
        self.chat.text.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_time_out_and_are_replaced() {
        let mut c = ClientState::default();
        c.center_print("one", "2", "1", 1000);
        assert!(c.center.active);
        c.advance(2999);
        assert!(c.center.active);
        c.center_print("two", "0", "1", 2500);
        c.advance(3000);
        assert!(c.center.active && c.center.text == "two", "the old timeout was cancelled");
        c.bottom_print("b", "1.5", "1", 3000);
        assert_eq!(c.bottom.text, "<just:center>b");
        c.advance(4500);
        assert!(!c.bottom.active);
    }

    #[test]
    fn message_lines_appear_and_go() {
        let mut c = ClientState::default();
        let long = "this is a rather long chat line that has to be wrapped over several lines of the messages hud";
        c.add_message(long, true, true, 0);
        c.advance(0);
        assert_eq!(c.messages.lines.len(), 1);
        c.advance(100);
        assert_eq!(c.messages.lines.len(), 2);
        c.add_message("ignored", false, false, 100);
        c.advance(12_100);
        assert_eq!(c.messages.lines.len(), 1);
        c.advance(12_200);
        assert!(c.messages.lines.is_empty());
        assert_eq!(c.pending_timers(), 0);
    }

    #[test]
    fn join_message_on_wake() {
        let mut c = ClientState::default();
        c.set_join_msg("Welcome to Think Tanks Tester.", "BlueTeam", false);
        c.play_wake(false, 50);
        c.advance(59);
        assert!(!c.bottom.active);
        c.advance(60);
        assert_eq!(c.bottom.text, "<just:center>Welcome to Think Tanks Tester.  You are on the BLUE team.");
        assert!(!c.solo.lives_visible);
    }

    #[test]
    fn chat_input() {
        let mut c = ClientState::default();
        assert_eq!(c.chat_key("a", true, Some("a")), ChatKey::Pass);
        c.open_chat();
        assert_eq!(c.chat_key("h", true, Some("h")), ChatKey::Used);
        c.chat_key("i", true, Some("i"));
        c.chat_key("i", false, None);
        assert_eq!(c.chat_key("tab", true, None), ChatKey::Pass);
        assert_eq!(c.chat_key("return", true, None), ChatKey::Send("hi".into()));
        c.close_chat();
        assert!(c.chat.text.is_empty());
    }
}

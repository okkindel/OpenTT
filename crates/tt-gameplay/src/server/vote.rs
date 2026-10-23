//! The map rotation's votes: a player calls a vote for a map of the
//! rotation, the players vote yes or no, and a passed vote ends the round
//! and plays that map next (the rotation goes on after it).
//!
//! The rules, all checked here (a client's menu only helps):
//!
//! * only the players vote and count (bots and test clients do not);
//! * a player calls a vote at most once every [`GameSetup::vote_wait_ms`]
//!   (3 min), and not in his first 3 minutes on the server; the caller
//!   votes yes;
//! * one vote at a time, during a round, for a map of the rotation other
//!   than the one being played;
//! * it passes as soon as more than half the players present vote yes,
//!   and fails when that can no longer happen, after [`VOTE_SECS`], or
//!   when the round ends first.
//!
//! [`GameSetup::vote_wait_ms`]: super::GameSetup::vote_wait_ms

use super::*;
use crate::msg::{MsgKind, ServerMsg, Value};

/// How long a vote runs (s).
pub const VOTE_SECS: u32 = 30;
/// The shortest time between two changes of one player's vote (ms).
pub const VOTE_CHANGE_MS: u32 = 1000;

/// A running map vote.
#[derive(Debug, Clone, PartialEq)]
pub struct Vote {
    /// The mission file voted for, and its name.
    pub map: String,
    pub name: String,
    /// The players who voted yes / no.
    pub yes: Vec<ClientId>,
    pub no: Vec<ClientId>,
    /// When the time is up (sim ms).
    pub ends_ms: u32,
    timer: TimerId,
}

/// `ms` as minutes and seconds (`2:10`), rounded up to the second.
pub fn wait_text(ms: u32) -> String {
    let s = ms.div_ceil(1000);
    format!("{}:{:02}", s / 60, s % 60)
}

impl Server {
    /// The players present (the client list without fake clients).
    pub fn humans(&self) -> Vec<ClientId> {
        self.group.iter().copied().filter(|c| self.client(*c).is_some_and(Client::human)).collect()
    }

    /// The rotation's mission file that `map` names: its id or name (any
    /// case), else the only one whose id or name contains it.
    pub fn rotation_map(&self, h: &mut dyn ServerHost, map: &str) -> Option<String> {
        let want = map.trim().to_lowercase();
        if want.is_empty() {
            return None;
        }
        let named: Vec<(String, String, String)> = self.setup.rotation.iter().map(|f| (f.clone(), tt_assets::layout::mission_id(f), h.load_info(f).to_lowercase())).collect();
        if let Some((f, ..)) = named.iter().find(|(_, id, name)| *id == want || *name == want) {
            return Some(f.clone());
        }
        match named.iter().filter(|(_, id, name)| id.contains(&want) || name.contains(&want)).collect::<Vec<_>>()[..] {
            [(f, ..)] => Some(f.clone()),
            _ => None,
        }
    }

    /// A line to one player.
    fn vote_tell(&mut self, h: &mut dyn ServerHost, c: ClientId, text: &str) {
        self.message_client(h, c, &MsgKind::Other { kind: Value::tag("MsgVote"), args: vec![] }, &Value::Str(format!("\u{3}{text}")));
    }

    /// A line to every player.
    fn vote_announce(&mut self, h: &mut dyn ServerHost, text: &str) {
        self.message_all(h, &MsgKind::Other { kind: Value::tag("MsgVote"), args: vec![] }, &Value::Str(format!("\u{3}{text}")));
    }

    /// Player `c` calls a vote for `map` (an id or name of the rotation):
    /// refused with the reason, or started.
    pub fn call_vote(&mut self, h: &mut dyn ServerHost, c: ClientId, map: &str) {
        let Some(cl) = self.client(c).filter(|x| x.human()) else { return };
        let now = h.sim_time();
        let wait = self.setup.vote_wait_ms;
        let left = |since: u32| (since.saturating_add(wait)).saturating_sub(now);
        let wait_left = left(cl.joined_ms).max(cl.vote_called_ms.map_or(0, left));
        let refusal = if !self.session.multiplayer() {
            Some("There is no voting in this game.".to_string())
        } else if self.vote.is_some() {
            Some("A vote is already running.".into())
        } else if !self.round.running() {
            Some("Wait for the round to start.".into())
        } else if wait_left > 0 {
            Some(format!("Wait {} before calling a vote.", wait_text(wait_left)))
        } else {
            None
        };
        if let Some(r) = refusal {
            return self.vote_tell(h, c, &r);
        }
        let Some(file) = self.rotation_map(h, map) else {
            return self.vote_tell(h, c, &format!("\"{}\" is not in the rotation.", map.trim()));
        };
        let name = h.load_info(&file);
        if file.eq_ignore_ascii_case(&self.session.mission_file) {
            return self.vote_tell(h, c, &format!("{name} is being played already."));
        }
        self.clients.get_mut(&c).unwrap().vote_called_ms = Some(now);
        let timer = self.timers.schedule(h, VOTE_SECS * 1000, Timer::VoteEnd);
        self.vote = Some(Vote { map: file, name: name.clone(), yes: vec![c], no: Vec::new(), ends_ms: now + VOTE_SECS * 1000, timer });
        let who = self.raw_name(c);
        self.vote_announce(h, &format!("{who} called a vote: switch to {name}?"));
        self.vote_count(h);
    }

    /// Player `c` votes (again: the new vote counts).
    pub fn cast_vote(&mut self, h: &mut dyn ServerHost, c: ClientId, yes: bool) {
        if !self.client(c).is_some_and(Client::human) {
            return;
        }
        let now = h.sim_time();
        let running = self.round.running();
        let last = self.clients.get(&c).and_then(|cl| cl.vote_cast_ms);
        let Some(v) = self.vote.as_mut().filter(|v| running && now < v.ends_ms) else {
            return self.vote_tell(h, c, "No vote is running.");
        };
        // The same vote again changes nothing; a changed vote at most once
        // a second (each one is sent to every player).
        if (if yes { &v.yes } else { &v.no }).contains(&c) || last.is_some_and(|l| now.saturating_sub(l) < VOTE_CHANGE_MS) {
            return;
        }
        v.yes.retain(|x| *x != c);
        v.no.retain(|x| *x != c);
        if yes {
            v.yes.push(c);
        } else {
            v.no.push(c);
        }
        if let Some(cl) = self.clients.get_mut(&c) {
            cl.vote_cast_ms = Some(now);
        }
        self.vote_count(h);
    }

    /// The votes of the players present: passed, failed, or the new count
    /// to everyone.
    pub fn vote_count(&mut self, h: &mut dyn ServerHost) {
        let humans = self.humans();
        let Some(v) = self.vote.as_mut() else { return };
        v.yes.retain(|c| humans.contains(c));
        v.no.retain(|c| humans.contains(c));
        let (n, yes, no) = (humans.len(), v.yes.len(), v.no.len());
        if yes * 2 > n {
            self.vote_passed(h);
        } else if (n - no) * 2 <= n {
            self.vote_failed(h, "too many players voted no");
        } else {
            self.send_vote_status(h);
        }
    }

    /// The vote's state to every player.
    fn send_vote_status(&mut self, h: &mut dyn ServerHost) {
        let now = h.sim_time();
        let humans = self.humans();
        for c in &humans {
            let msg = match &self.vote {
                Some(v) => {
                    let mine = if v.yes.contains(c) { Some(true) } else if v.no.contains(c) { Some(false) } else { None };
                    ServerMsg::VoteStatus { map: v.name.clone(), yes: v.yes.len() as u32, no: v.no.len() as u32, players: humans.len() as u32, secs: v.ends_ms.saturating_sub(now).div_ceil(1000), mine }
                }
                None => ServerMsg::VoteStatus { map: String::new(), yes: 0, no: 0, players: 0, secs: 0, mine: None },
            };
            h.send(Some(*c), &msg);
        }
    }

    /// The vote ends: its timer goes, the players' prompts go.
    fn end_vote(&mut self, h: &mut dyn ServerHost) -> Option<Vote> {
        let v = self.vote.take()?;
        self.timers.cancel(h, v.timer);
        self.send_vote_status(h);
        Some(v)
    }

    /// The vote went through: the round ends now, its map comes next.
    fn vote_passed(&mut self, h: &mut dyn ServerHost) {
        let Some(v) = self.end_vote(h) else { return };
        self.vote_announce(h, &format!("Vote passed: switching to {}.", v.name));
        self.session.voted_mission = Some(v.map);
        self.cycle_round(h);
    }

    /// The vote did not go through (`why`).
    pub fn vote_failed(&mut self, h: &mut dyn ServerHost, why: &str) {
        if let Some(v) = self.end_vote(h) {
            self.vote_announce(h, &format!("Vote for {} failed: {why}.", v.name));
        }
    }

    /// The vote goes without a word (the server goes).
    pub fn cancel_vote(&mut self, h: &mut dyn ServerHost) {
        if let Some(v) = self.vote.take() {
            self.timers.cancel(h, v.timer);
        }
    }

    /// The maps player `c` can vote for: the rotation, (id, name).
    pub fn send_map_list(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        if !self.client(c).is_some_and(Client::human) {
            return;
        }
        if !self.session.multiplayer() {
            return self.vote_tell(h, c, "There is no voting in this game.");
        }
        let now = h.sim_time();
        let cl = self.clients.get_mut(&c).unwrap();
        if cl.map_list_ms.is_some_and(|l| now.saturating_sub(l) < VOTE_CHANGE_MS) {
            return;
        }
        cl.map_list_ms = Some(now);
        let maps = self.setup.rotation.clone().into_iter().map(|f| (tt_assets::layout::mission_id(&f), h.load_info(&f))).collect();
        h.send(Some(c), &ServerMsg::MapList { maps });
    }

    /// A new rotation (mission files), kept in the settings as ids.
    pub fn set_rotation(&mut self, h: &mut dyn ServerHost, files: Vec<String>) {
        let ids: Vec<String> = files.iter().map(|f| tt_assets::layout::mission_id(f)).collect();
        h.set_setting(Setting::MapRotation, &ids.join("\t"));
        self.setup.rotation = files;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn waits_read_as_minutes() {
        assert_eq!(super::wait_text(130_000), "2:10");
        assert_eq!(super::wait_text(59_001), "1:00");
        assert_eq!(super::wait_text(180_000), "3:00");
        assert_eq!(super::wait_text(1), "0:01");
    }
}

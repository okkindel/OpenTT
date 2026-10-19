//! Players connect and leave: admin flags, a unique name, the starting
//! tank, the team, the join messages, bot replacement, the server list's
//! player list; the mission download's three phases; kick and ban.

use super::*;
use crate::msg::*;
use tt_game::fmt::{atof, atoi};

/// Characters stripped from names for the player list (the name tags'
/// colour and style codes).
const NAME_CONTROL: &str = "\u{10}\u{11}\u{7}\u{b}\u{c}\u{e}";

pub use tt_game::fmt::str_to_player_name;

/// `s` without any of `chars`.
pub fn strip_chars(s: &str, chars: &str) -> String {
    s.chars().filter(|c| !chars.contains(*c)).collect()
}

/// The tanks a joining player may pick, by number.
const PLAYER_TANKS: [&str; 4] = ["LightTank", "MediumTank", "HeavyTank", "BabyBoss"];

impl Server {
    /// A setting read as a number.
    pub(crate) fn setting_num(&self, h: &dyn ServerHost, s: Setting) -> f64 {
        atof(&h.setting(s)) as f32 as f64
    }

    /// A setting read as a flag.
    pub(crate) fn setting_on(&self, h: &dyn ServerHost, s: Setting) -> bool {
        atoi(&h.setting(s)) != 0
    }

    /// A connection asks to join: a reason to refuse, or "".
    pub fn connect_request(&mut self, h: &mut dyn ServerHost, address: &str) -> String {
        h.echo(&format!("Connect request from: {address}"));
        if self.session.player_count as f64 >= self.setting_num(h, Setting::MaxPlayers) {
            return "CR_SERVERFULL".into();
        }
        String::new()
    }

    /// The join message describing `who` (bots: ai 1, no admin).
    pub(crate) fn join(&self, who: ClientId) -> MsgKind {
        let c = &self.clients[&who];
        let r = c.rec.as_ref();
        let flag = |f: Option<bool>| super::state::flag_text(f).to_string();
        let (ai, admin, super_admin) = match c.kind {
            ClientKind::Bot => ("1".into(), "0".into(), "0".into()),
            // A fake client cannot say whether it is AI controlled.
            ClientKind::Fake => (String::new(), flag(c.admin), flag(c.super_admin)),
            _ => ("0".into(), flag(c.admin), flag(c.super_admin)),
        };
        MsgKind::ClientJoin {
            name: self.name_value(Some(who)),
            id: who.text(),
            team: self.team_name_of(who),
            guid: if c.send_guid { "0".into() } else { String::new() },
            score: r.map(|r| r.score_text()).unwrap_or_default(),
            cum: r.map(|r| r.cum_score_text()).unwrap_or_default(),
            ai,
            admin,
            super_admin,
        }
    }

    /// A connection joins with its name, team and tank number.
    pub fn connected(&mut self, h: &mut dyn ServerHost, client: ClientId, name: &str, team_name: &str, tank_id: &str) {
        let err = h.setting(Setting::ConnectionError);
        self.message_client(h, client, &MsgKind::ConnectionError { text: Value::str(err) }, &Value::empty());
        self.send_mission_name(h, client);
        let c = self.clients.get_mut(&client).unwrap();
        c.guid = "0";
        c.admin = Some(false);
        c.super_admin = Some(false);
        self.set_player_name(client, name);
        self.with_record(Some(client), |r| {
            r.score = Some(0.0);
            r.cum_score = Some(0.0);
        });
        let db = if self.setup.demo() {
            self.setup.demo_tank.clone()
        } else {
            let id = atof(tank_id) as f32 as f64;
            PLAYER_TANKS.iter().enumerate().find(|(i, _)| id == *i as f64).map_or("MediumTank", |(_, d)| d).to_string()
        };
        let tank = h.data().tanks.find(&db);
        self.clients.get_mut(&client).unwrap().starting_tank = tank;
        if self.setup.team_game {
            let team = match self.team_named(team_name) {
                Some(t) => Some(t),
                None => self.smallest_team(),
            };
            self.clients.get_mut(&client).unwrap().team = team;
            self.team_add(team, client);
        }
        let addr = h.address(client);
        h.echo(&format!("CADD: {} {addr}", client.text()));
        let teams: Vec<(String, String, String, String)> = self.teams.iter().flatten().map(|t| (t.id.0.to_string(), t.rec.long_name.clone(), tt_game::fmt::format_g(t.rec.score), tt_game::fmt::format_g(t.rec.cum_score))).collect();
        for (team, long, score, cum) in teams {
            self.message_client(h, client, &MsgKind::AddTeam { team, name: Value::str(long), score, cum }, &Value::empty());
        }
        for other in self.group.clone() {
            if other != client {
                let kind = self.join(other);
                self.message_client(h, client, &kind, &Value::empty());
            }
        }
        for other in self.bots() {
            let kind = self.join(other);
            self.message_client(h, client, &kind, &Value::empty());
        }
        let welcome = if self.setup.target_range() { Value::empty() } else { Value::tag(format!("Welcome to {} \u{2}%1.", tt_net::PRODUCT_NAME)) };
        let kind = self.join(client);
        self.message_client(h, client, &kind, &welcome);
        let kind = self.join(client);
        self.message_all_except(h, client, &kind, &Value::tag("\u{2}%1 joined the game."));
        if self.setting_on(h, Setting::ReplaceBots) && self.bot_group.is_some() {
            self.balance_bots(h);
        }
        if self.session.mission_running {
            self.send_mission(h, client);
        }
        self.session.player_count += 1;
        if self.setup.single_player() {
            self.with_record(Some(client), |r| r.lives = Some(3));
        }
        self.refresh_player_list(h);
    }

    /// The team with the fewest players (a bot counts 1/100).
    pub(crate) fn smallest_team(&self) -> Option<TeamColor> {
        let teams = self.teams.as_ref()?;
        let mut min_count = 99999.0f64;
        let mut min_index = 0;
        for (ti, t) in teams.iter().enumerate() {
            let mut count = t.members.len() as f64 * 100.0;
            // (Bots have a driver.)
            count += t.members.iter().filter(|m| self.rec(Some(**m), |r| r.ai.is_some())).count() as f64;
            if count < min_count {
                min_count = count;
                min_index = ti;
            }
        }
        teams.get(min_index).map(|t| t.color)
    }

    /// A unique, clean name for the client (`name.1`, `name.2`, … when
    /// taken; `Loser` when nothing is left of it).
    pub fn set_player_name(&mut self, client: ClientId, name: &str) {
        let Some(c) = self.clients.get_mut(&client) else { return };
        c.send_guid = true;
        c.name = None;
        let mut name = str_to_player_name(name).trim_end_matches([' ', '_']).to_string();
        if name.chars().count() < 1 {
            name = "Loser".into();
        }
        if !self.is_name_unique(&name) {
            let mut suffix = 1;
            loop {
                let try_name = format!("{name}.{suffix}");
                suffix += 1;
                if self.is_name_unique(&try_name) {
                    name = try_name;
                    break;
                }
            }
        }
        self.with_record(Some(client), |r| r.name_base = Some(name.clone()));
        self.clients.get_mut(&client).unwrap().name = Some(name);
    }

    /// A client's plain name for comparisons and the player list.
    pub(crate) fn raw_name(&self, c: ClientId) -> String {
        strip_chars(&self.name_text(c), NAME_CONTROL)
    }

    /// No client of the client list has this name (case-sensitive).
    pub fn is_name_unique(&self, name: &str) -> bool {
        !self.group.iter().any(|&c| self.clients.get(&c).is_some_and(|x| x.name.as_deref().is_some_and(|n| !n.is_empty())) && self.raw_name(c) == name)
    }

    /// The client leaves (it is removed afterwards: [`Server::remove_client`]).
    pub fn dropped(&mut self, h: &mut dyn ServerHost, client: ClientId) {
        self.leave_game(h, client);
        let kind = MsgKind::ClientDrop { name: self.name_value(Some(client)), id: client.text() };
        self.message_all_except(h, client, &kind, &Value::tag("\u{2}%1 has left the game."));
        let addr = h.address(client);
        h.echo(&format!("CDROP: {} {addr}", client.text()));
        if self.client(client).is_some_and(|c| c.starting_tank.is_some()) {
            self.session.player_count -= 1;
        }
        if self.setting_on(h, Setting::ReplaceBots) && self.bot_group.is_some() {
            self.timers.schedule(h, 0, Timer::BalanceBots);
        }
        let num_bots = self.bot_group.as_ref().map_or(0, Vec::len) as i32;
        if num_bots == self.session.player_count && self.setup.dedicated && self.round.running() {
            self.timers.schedule(h, 0, Timer::RestartMission);
        }
        self.timers.schedule(h, 0, Timer::RefreshPlayerList);
    }

    /// A client's drop and removal.
    pub fn drop_client(&mut self, h: &mut dyn ServerHost, client: ClientId) {
        if self.kind(client).is_some_and(ClientKind::network) {
            self.dropped(h, client);
        }
        self.remove_client(h, client);
    }

    pub fn send_mission_start(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        let seq = self.session.mission_seq.to_string();
        h.send(Some(c), &ServerMsg::MissionStart { seq });
    }

    pub fn send_mission_end(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        let seq = self.session.mission_seq.to_string();
        h.send(Some(c), &ServerMsg::MissionEnd { seq });
    }

    /// The team letter for the player list (first letter of the short name).
    pub(crate) fn team_letter(&self, c: ClientId) -> String {
        self.team_short(self.team_of(c)).chars().take(1).collect()
    }

    /// The server list's player list of a multiplayer server.
    pub fn refresh_player_list(&mut self, _h: &mut dyn ServerHost) {
        if !self.session.multiplayer() {
            return;
        }
        let mut rows = Vec::new();
        for c in self.group.clone() {
            rows.push(format!("{}\t{}", self.team_letter(c), self.raw_name(c)));
        }
        for c in self.bots() {
            let base = self.rec(Some(c), |r| r.name().to_string());
            rows.push(format!("{}\t{}", self.team_letter(c), strip_chars(&base, NAME_CONTROL)));
        }
        self.session.player_list = rows.join("\t");
    }

    /// The client is told to load the mission (phase 1 starts).
    pub fn send_mission(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        if let Some(x) = self.clients.get_mut(&c) {
            x.phase = Some(DownloadPhase::Started);
        }
        let (seq, file, music) = (self.session.mission_seq.to_string(), self.session.mission_file.clone(), h.music_track());
        h.send(Some(c), &ServerMsg::MissionPhase1 { seq, file: file.clone(), music });
        h.echo(&format!("*** Sending mission load to client: {file}"));
    }

    /// An answer for another mission, or none running.
    pub(crate) fn stale(&self, seq: &str) -> bool {
        atof(seq) as f32 != self.session.mission_seq as f32 || !self.session.mission_running
    }

    pub(crate) fn phase(&self, c: ClientId) -> Option<DownloadPhase> {
        self.client(c).and_then(|c| c.phase)
    }

    pub(crate) fn set_phase(&mut self, c: ClientId, p: DownloadPhase) {
        if let Some(x) = self.clients.get_mut(&c) {
            x.phase = Some(p);
        }
    }

    /// The client acknowledges a download phase.
    pub fn phase_ack(&mut self, h: &mut dyn ServerHost, c: ClientId, phase: u8, seq: &str) {
        match phase {
            1 => {
                if self.stale(seq) || !matches!(self.phase(c), None | Some(DownloadPhase::Started)) {
                    return;
                }
                self.set_phase(c, DownloadPhase::Defs);
                h.transmit_defs(c, &self.session.mission_seq.to_string());
            }
            2 => {
                if self.stale(seq) || self.phase(c) != Some(DownloadPhase::DefsDone) {
                    return;
                }
                self.set_phase(c, DownloadPhase::Replication);
                h.start_replication(c, &self.session.mission_file);
            }
            _ => {
                if self.stale(seq) || self.phase(c) != Some(DownloadPhase::Replication) {
                    return;
                }
                self.set_phase(c, DownloadPhase::InGame);
                self.send_mission_start(h, c);
                self.enter_game(h, c);
            }
        }
    }

    /// The client has the defs of mission `seq`.
    pub fn defs_done(&mut self, h: &mut dyn ServerHost, c: ClientId, seq: &str) {
        if atof(seq) as f32 != self.session.mission_seq as f32 || self.phase(c) != Some(DownloadPhase::Defs) {
            return;
        }
        self.set_phase(c, DownloadPhase::DefsDone);
        let (seq, file) = (self.session.mission_seq.to_string(), self.session.mission_file.clone());
        h.send(Some(c), &ServerMsg::MissionPhase2 { seq, file });
    }

    pub fn objects_received(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        let (seq, file) = (self.session.mission_seq.to_string(), self.session.mission_file.clone());
        h.send(Some(c), &ServerMsg::MissionPhase3 { seq, file });
    }

    /// The mission's name to a client.
    pub fn send_mission_name(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        let name = self.mission_name.clone();
        self.message_client(h, c, &MsgKind::LoadInfo { name: Value::str(name) }, &Value::empty());
        self.message_client(h, c, &MsgKind::LoadInfoDone, &Value::empty());
    }

    pub(crate) fn remove_admin(&mut self, h: &mut dyn ServerHost, client: ClientId, text_: &str, time: Setting, reason: &str) {
        self.message_all(h, &MsgKind::AdminForce { who: self.name_value(Some(client)) }, &Value::tag(text_));
        let guid = self.client(client).map(|c| c.guid).unwrap_or_default();
        let addr = h.address(client);
        let secs = h.setting(time);
        h.ban(guid, &addr, &secs);
        if self.kind(client).is_some_and(ClientKind::network) {
            self.dropped(h, client);
            h.delete_connection(client, reason);
        }
        self.remove_client(h, client);
    }

    pub fn kick(&mut self, h: &mut dyn ServerHost, client: ClientId) {
        self.remove_admin(h, client, "\u{3}The Admin has kicked %1.", Setting::KickBanTime, "You have been banned from this server");
    }

    pub fn ban(&mut self, h: &mut dyn ServerHost, client: ClientId) {
        self.remove_admin(h, client, "\u{3}The Admin has banned %1.", Setting::BanTime, "You have been kicked from this server");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(str_to_player_name("  Bob__the,,.Tank   "), "Bob_theTank");
        assert_eq!(str_to_player_name("abcdefghijklmnopqrstu"), "abcdefghijklmnop");
        assert_eq!(strip_chars("\u{10}\u{c}Hum [Bot]\u{11}", NAME_CONTROL), "Hum [Bot]");
    }
}

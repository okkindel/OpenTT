//! Tanks destroyed: kills, deaths and scores of a multiplayer game (a kill
//! is worth 2, an assist 1), and the K/D/A count.
//!
//! From the world's events: a death for the destroyed tank's client; a
//! kill for the killer's client unless it is the victim itself (no killer
//! = an accident) or, in a team game, its team mate; an assist for every
//! other client that damaged the victim in the [`ASSIST_MS`] before a
//! counted kill (once per kill, never the killer or the victim; damage to
//! a team mate earns none). Bot clients count like players; the solo
//! campaign's bots have no client: the player's kills of them count. A new
//! round starts from zero.

use super::state::*;
use super::*;
use crate::entity::EntityId;
use crate::msg::*;
use tt_game::fmt::format_g;
use std::collections::{BTreeMap, HashMap};

pub const BRAIN_SEPARATED: &str = "Your brain has been separated from your tank, press SPACE.";

/// Damage this recent (ms of world time) before a kill earns an assist.
pub const ASSIST_MS: i64 = 5000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Kda {
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
}

#[derive(Debug, Default)]
pub struct Scoreboard {
    /// Per client.
    pub stats: BTreeMap<ClientId, Kda>,
    /// Recent damage per victim tank (attacker client, world ms).
    hits: HashMap<EntityId, Vec<(ClientId, i64)>>,
    /// At the last tick.
    running: bool,
    /// Changed since the lists were sent.
    pub unsent: bool,
}

impl Scoreboard {
    /// A new round: everyone back to zero.
    pub fn reset(&mut self) {
        self.stats.clear();
        self.hits.clear();
        self.unsent = true;
    }

    /// `victim` took damage from a tank of `attacker` at `now`.
    pub fn hit(&mut self, victim: EntityId, attacker: ClientId, now: i64) {
        let v = self.hits.entry(victim).or_default();
        v.retain(|(a, _)| *a != attacker);
        v.push((attacker, now));
    }

    /// `victim_tank` (of `victim`; None for a clientless bot) was destroyed
    /// by a tank of `killer`. `team_mates(a, b)`: same team of a team game.
    /// The clients credited with an assist.
    pub fn kill(&mut self, victim_tank: EntityId, victim: Option<ClientId>, killer: Option<ClientId>, now: i64, team_mates: impl Fn(ClientId, ClientId) -> bool) -> Vec<ClientId> {
        let hits = self.hits.remove(&victim_tank).unwrap_or_default();
        if let Some(v) = victim {
            self.stats.entry(v).or_default().deaths += 1;
        }
        self.unsent = true;
        let Some(k) = killer.filter(|&k| Some(k) != victim) else { return Vec::new() };
        if victim.is_some_and(|v| team_mates(k, v)) {
            return Vec::new();
        }
        self.stats.entry(k).or_default().kills += 1;
        let mut credited = Vec::new();
        for (a, t) in hits {
            if a == k || Some(a) == victim || now - t > ASSIST_MS || credited.contains(&a) || victim.is_some_and(|v| team_mates(a, v)) {
                continue;
            }
            credited.push(a);
            self.stats.entry(a).or_default().assists += 1;
        }
        credited
    }

    pub fn get(&self, client: ClientId) -> Kda {
        self.stats.get(&client).copied().unwrap_or_default()
    }

    /// The client is gone.
    pub fn forget(&mut self, client: ClientId) {
        self.stats.remove(&client);
    }

    /// (client, K/D/A) by client id.
    pub fn entries(&self) -> Vec<(ClientId, Kda)> {
        self.stats.iter().map(|(c, k)| (*c, *k)).collect()
    }
}

/// A world event the scoreboard counts, in entity terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KdaEvent {
    /// `victim` took damage (> 0) from tank `source` (not itself).
    Damaged { victim: EntityId, source: EntityId },
    Killed { victim: EntityId, killer: Option<EntityId> },
}

impl Server {
    /// The client of a tank the server made that still exists.
    pub(crate) fn client_of_tank(&self, h: &dyn ServerHost, t: Option<EntityId>) -> Option<ClientId> {
        let t = t.filter(|t| self.tank_exists(h, Some(*t)))?;
        self.tank_client(t)
    }

    /// The world destroyed `this`; `killer` dealt the last damage.
    pub fn tank_destroyed(&mut self, h: &mut dyn ServerHost, this: EntityId, killer: Option<EntityId>) {
        let killer = killer.filter(|k| self.tank_exists(h, Some(*k)));
        if self.setup.single_player() {
            self.solo_tank_destroyed(h, this, killer);
            return;
        }
        if self.setup.target_range() {
            self.target_destroyed(h, this, killer);
            return;
        }
        let client = self.tank_client(this);
        self.with_record(client, |r| r.deaths = Some(r.deaths.unwrap_or(0) + 1));
        let deathmatch = self.setup.mission_type == MissionType::Deathmatch;
        let kc = killer.and_then(|k| self.tank_client(k));
        if killer.is_some() {
            let kind = MsgKind::ClientKilled { victim: self.name_value(client), killer: self.name_value(kc) };
            self.message_all(h, &kind, &Value::tag("%1 has been eliminated by %2!"));
            if deathmatch {
                let counts = !self.setup.team_game || self.team_id_of(client) != self.team_id_of(kc);
                if counts {
                    // A kill is worth 2.
                    self.score_client(h, kc, 2.0, 2.0);
                }
            }
            self.with_record(kc, |r| r.kills = Some(r.kills.unwrap_or(0) + 1));
        } else {
            let kind = MsgKind::ClientKilled { victim: self.name_value(client), killer: Value::empty() };
            self.message_all(h, &kind, &Value::tag("%1 has been eliminated!"));
        }
        if deathmatch {
            if self.setup.team_game {
                let own_team = killer.is_some() && self.team_id_of(client) == self.team_id_of(killer.and_then(|k| self.tank_client(k)));
                if own_team {
                    self.score_client(h, client, -1.0, -1.0);
                } else {
                    self.score_client(h, client, -1.0, 0.0);
                }
            } else {
                self.score_client(h, client, -1.0, -1.0);
            }
        }
        let bot = self.rec(client, |r| r.ai.is_some());
        if !bot {
            if self.setup.ai_control {
                if let Some(c) = client.and_then(|c| self.clients.get_mut(&c)) {
                    c.player = None;
                }
                if let Some(c) = client {
                    self.timers.schedule(h, 4000, Timer::Respawn(c));
                }
            } else {
                self.center_print_to(h, client, BRAIN_SEPARATED, "1");
            }
        } else if let Some(c) = client {
            self.spawn_bot(h, c);
        }
    }

    /// `score` and `cum` by `delta`, never below zero.
    pub(crate) fn add_score(score: &mut f64, cum: &mut f64, delta: f64) {
        if -delta > *score {
            *cum -= *score;
            *score = 0.0;
        } else {
            *score += delta;
            *cum += delta;
        }
    }

    /// A client's score (and its team's) changes; everyone is told.
    pub fn score_client(&mut self, h: &mut dyn ServerHost, client: Option<ClientId>, delta: f64, delta_team: f64) {
        if let Some(r) = self.record_mut(client) {
            let (mut s, mut c) = (r.score.unwrap_or(0.0), r.cum_score.unwrap_or(0.0));
            Self::add_score(&mut s, &mut c, delta);
            r.score = Some(s);
            r.cum_score = Some(c);
        }
        let team = client.and_then(|c| self.team_of(c));
        if let Some(t) = self.team_mut(team) {
            Self::add_score(&mut t.rec.score, &mut t.rec.cum_score, delta_team);
        }
        let (score, cum) = self.rec(client, |r| (r.score_text(), r.cum_score_text()));
        let id = client.map(|c| c.text()).unwrap_or_default();
        self.message_all(h, &MsgKind::ScoreChanged { score, cum, id }, &Value::empty());
        if let Some(t) = self.team(team) {
            let (score, cum, id) = (format_g(t.rec.score), format_g(t.rec.cum_score), t.id.0.to_string());
            self.message_all(h, &MsgKind::TeamScoreChanged { score, cum, team: id }, &Value::empty());
        }
    }

    /// +1 score (and team score) for an assist, in the modes where kills
    /// score (battle).
    pub fn award_assist(&mut self, h: &mut dyn ServerHost, client: ClientId) {
        if self.setup.mission_type != MissionType::Deathmatch {
            return;
        }
        self.score_client(h, Some(client), 1.0, 1.0);
    }

    /// A lasting center print to one client (the brain and game-over notices).
    pub(crate) fn center_print_to(&mut self, h: &mut dyn ServerHost, client: Option<ClientId>, text: &str, lines: &str) {
        let msg = ServerMsg::CenterPrint { text: Value::str(text), secs: "0".into(), lines: lines.into(), lc: false };
        self.send_to(h, client, &msg);
    }

    /// Every authority tick, with the tick's damage and kill events.
    pub fn count_kda(&mut self, h: &mut dyn ServerHost, events: &[KdaEvent], now: i64) {
        let running = self.round.running();
        if running && !self.kda.running {
            self.kda.reset();
        }
        self.kda.running = running;
        let team_game = self.setup.team_game;
        fn client_of(s: &Server, h: &dyn ServerHost, t: EntityId) -> Option<ClientId> {
            if s.tank_exists(h, Some(t)) { s.tank_client(t) } else { None }
        }
        for e in events {
            match *e {
                KdaEvent::Damaged { victim, source } => {
                    if let Some(a) = client_of(self, h, source) {
                        self.kda.hit(victim, a, now);
                    }
                }
                KdaEvent::Killed { victim: tank, killer } => {
                    let victim = client_of(self, h, tank);
                    let killer = killer.and_then(|k| client_of(self, h, k));
                    let team = |c: ClientId| self.team_id_of(Some(c));
                    let teams: Vec<(ClientId, Option<super::TeamId>)> = if team_game {
                        let attackers: Vec<ClientId> = self.kda.hits.get(&tank).into_iter().flatten().map(|(a, _)| *a).collect();
                        [victim, killer].into_iter().flatten().chain(attackers).map(|c| (c, team(c))).collect()
                    } else {
                        Vec::new()
                    };
                    let team_of = |c: ClientId| teams.iter().find(|(o, _)| *o == c).and_then(|(_, t)| *t);
                    let assists = self.kda.kill(tank, victim, killer, now, |a, b| team_game && team_of(a).is_some() && team_of(a) == team_of(b));
                    for a in assists {
                        self.award_assist(h, a);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kills_assists_deaths() {
        let mut sb = Scoreboard::default();
        let (a, b, c, d) = (ClientId(10), ClientId(11), ClientId(12), ClientId(13));
        let (tb, tc) = (EntityId(2), EntityId(3));
        sb.hit(tb, d, 0);
        sb.hit(tb, c, 4000);
        sb.hit(tb, a, 5000);
        sb.hit(tb, c, 5500);
        sb.kill(tb, Some(b), Some(a), 6000, |_, _| false);
        assert_eq!(sb.get(a), Kda { kills: 1, deaths: 0, assists: 0 });
        assert_eq!(sb.get(b), Kda { kills: 0, deaths: 1, assists: 0 });
        assert_eq!(sb.get(c), Kda { kills: 0, deaths: 0, assists: 1 });
        assert_eq!(sb.get(d), Kda::default());
        sb.kill(tc, Some(c), Some(c), 7000, |_, _| false);
        sb.kill(EntityId(4), Some(c), None, 7000, |_, _| false);
        assert_eq!(sb.get(c), Kda { kills: 0, deaths: 2, assists: 1 });
        let mates = |x: ClientId, y: ClientId| [x, y].iter().all(|o| [a, b].contains(o));
        sb.hit(EntityId(5), d, 8000);
        sb.kill(EntityId(5), Some(b), Some(a), 8100, mates);
        assert_eq!((sb.get(a).kills, sb.get(b).deaths, sb.get(d).assists), (1, 2, 0));
        sb.hit(EntityId(6), a, 9000);
        sb.kill(EntityId(6), Some(b), Some(d), 9100, mates);
        assert_eq!((sb.get(d).kills, sb.get(a).assists), (1, 0));
        sb.kill(EntityId(7), None, Some(a), 9200, |_, _| false);
        assert_eq!(sb.get(a).kills, 2);
        sb.reset();
        assert!(sb.entries().is_empty());
    }
}

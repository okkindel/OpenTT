//! Bots of a multiplayer game: they join from the bot slots, leave and
//! come back as players join and leave, and drive their tanks.

use super::records::text_field;
use super::state::*;
use super::*;
use crate::msg::*;

impl Server {
    pub fn spawn_bot(&mut self, h: &mut dyn ServerHost, bc: ClientId) {
        if self.bot_tanks.is_none() || !self.exists(Some(bc)) {
            return;
        }
        let Some(def) = self.clients[&bc].bot_tank else { return };
        let Some(bot) = self.new_tank(h, def, Some(bc)) else { return };
        self.clients.get_mut(&bc).unwrap().player = Some(bot);
        // The bots' tanks are kept apart from the round's other objects.
        if let Some(c) = self.cleanup.as_mut() {
            c.retain(|x| *x != CleanupItem::Entity(bot));
        }
        if let Some(b) = self.bot_tanks.as_mut() {
            b.push(bot);
        }
        let team = self.team_of(bc);
        let team_name = self.team_short(team);
        let pos = self.spawn_point(h, &team_name);
        h.set_transform(bot, &pos);
        let name = self.rec(Some(bc), |r| r.name().to_string());
        let ai = h.data().tanks[def].ai;
        h.set_ai(bot, ai);
        h.set_player_name(bot, &name);
        if self.team(team).is_some() {
            h.set_team(bot, &team_name);
        }
    }

    /// The bot of slot `i` joins.
    pub fn add_bot(&mut self, h: &mut dyn ServerHost, i: u32) -> ClientId {
        let BotSlot { tank: db, name, team } = self.bot_slots.get(i as usize).cloned().unwrap_or_default();
        let def = h.data().tanks.find(&db);
        let ai = def.and_then(|t| h.data().tanks[t].ai).map(|a| h.data().ai[a].name.clone());
        let bc = ClientId(self.alloc_id());
        let mut c = super::Client::new(bc, ClientKind::Bot);
        c.bot_tank = def;
        {
            let r = c.rec.as_mut().unwrap();
            r.name_base = text_field(&name);
            r.ai = ai;
            r.score = Some(0.0);
            r.cum_score = Some(0.0);
            r.index = Some(i);
        }
        self.clients.insert(bc, c);
        if let Some(g) = self.bot_group.as_mut() {
            g.push(bc);
        }
        self.sync_groups(h);
        let team = if !self.setup.team_game {
            None
        } else {
            let t = self.team_named(&team).or_else(|| {
                let half = (self.bot_slots.len() as u32 >> 1) as f64;
                Some(if (i as f64) < half { TeamColor::Blue } else { TeamColor::Green })
            });
            self.team_add(t, bc);
            t
        };
        let c = self.clients.get_mut(&bc).unwrap();
        c.team = team;
        c.send_guid = true;
        let base = c.rec.as_ref().unwrap().name().to_string();
        c.name = Some(format!("\u{10}\u{c}{base}\u{11}"));
        let kind = self.join(bc);
        self.message_all(h, &kind, &Value::tag("\u{2}%1 joined the game."));
        self.session.player_count += 1;
        bc
    }

    pub fn drop_bot(&mut self, h: &mut dyn ServerHost, bc: ClientId) {
        let player = self.client(bc).and_then(|c| c.player);
        if self.tank_exists(h, player) {
            self.delete_tank(h, player.unwrap());
        }
        let team = self.team_of(bc);
        self.team_remove(team, bc);
        let kind = MsgKind::ClientDrop { name: self.name_value(Some(bc)), id: bc.text() };
        self.message_all(h, &kind, &Value::tag("\u{2}%1 has left the game."));
        self.remove_client(h, bc);
        self.session.player_count -= 1;
    }

    /// Whether bot slot `k` is on team `team` (its team named, existing).
    pub(crate) fn slot_on_team(&self, k: usize, team: TeamId) -> bool {
        let slot = self.bot_slots.get(k).map(|s| s.team.as_str()).unwrap_or("");
        self.team(self.team_named(slot)).map(|t| t.id) == Some(team)
    }

    /// Bots leave as players join and come back as they leave, keeping
    /// the server (or `team`) at its bot slots' strength: a bot goes once
    /// the players outnumber the missing bots by more than one, from the
    /// highest slot alternating between the halves; a missing bot comes
    /// back while fewer players than missing bots (plus one) are there.
    pub fn balance_team_bots(&mut self, h: &mut dyn ServerHost, team: Option<TeamId>) {
        let drop_slack = 1.0;
        let slots = self.bot_slots.len();
        let mut present = vec![false; slots];
        for bc in self.bots() {
            if let Some(i) = self.rec(Some(bc), |r| r.index) {
                if let Some(p) = present.get_mut(i as usize) {
                    *p = true;
                }
            }
        }
        let wanted = |s: &Server, k: usize| team.is_none_or(|t| s.slot_on_team(k, t));
        let (mut full_count, mut there) = (0.0f64, 0.0f64);
        for (k, (slot, &present)) in self.bot_slots.iter().zip(&present).enumerate() {
            if h.data().tanks.find(&slot.tank).is_some() && wanted(self, k) {
                full_count += 1.0;
                if present {
                    there += 1.0;
                }
            }
        }
        let players = match team {
            Some(t) => self.group.iter().filter(|c| self.team_id_of(Some(**c)) == Some(t)).count() as f64,
            None => self.group.len() as f64,
        };
        let half = (slots as u32 >> 1) as usize;
        if players > drop_slack && there > 0.0 && there + players - drop_slack > full_count {
            for i in 0..slots {
                // Below the first slot (an odd slot count): no such slot.
                let Some(k) = ((i % 2) * half + half).checked_sub((i >> 1) + 1) else { continue };
                if present.get(k).copied().unwrap_or(false) && wanted(self, k) {
                    if let Some(bc) = self.bots().into_iter().find(|bc| self.rec(Some(*bc), |r| r.index) == Some(k as u32)) {
                        h.echo("Dropping bot because human player joined");
                        self.drop_bot(h, bc);
                    }
                    break;
                }
            }
        } else if there < full_count && there + players - drop_slack < full_count {
            for i in 0..slots {
                let k = (i % 2) * half + (i >> 1);
                if !present.get(k).copied().unwrap_or(false) && wanted(self, k) {
                    h.echo("Adding bot because human player left");
                    let bc = self.add_bot(h, k as u32);
                    self.spawn_bot(h, bc);
                    break;
                }
            }
        }
    }

    pub fn balance_bots(&mut self, h: &mut dyn ServerHost) {
        if self.setup.team_game {
            let ids: Vec<TeamId> = self.teams.iter().flatten().map(|t| t.id).collect();
            for id in ids {
                self.balance_team_bots(h, Some(id));
            }
        } else {
            self.balance_team_bots(h, None);
        }
    }

    /// The bots of the settings.
    pub fn bots_from_settings(&mut self, h: &mut dyn ServerHost) {
        self.bot_slots = h.bot_settings();
    }

    /// The default bots: three green, three blue.
    pub fn default_bots(&mut self) {
        self.bot_slots = DEFAULT_BOTS.iter().take(6).enumerate().map(|(i, (tank, name))| BotSlot::new(tank, name, if i < 3 { "GreenTeam" } else { "BlueTeam" })).collect();
    }
}

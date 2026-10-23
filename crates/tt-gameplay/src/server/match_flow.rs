//! The match: creating and destroying the server, loading, ending and
//! resetting missions, a round's start, clock and end, the pause between
//! rounds and the next mission (the rotation, the demo's pick).

use super::respawn::vector_add;
use super::state::*;
use super::*;
use crate::msg::*;
use tt_game::fmt::format_g;

impl Server {
    /// Sets the server's port (the host tries ports upwards when it opens it).
    pub fn port_init(&mut self, h: &mut dyn ServerHost, port: &str) {
        if !self.setup.skip_port {
            h.set_net_port(port);
        }
    }

    /// A new server of type `ty` on `mission`.
    pub fn create_server(&mut self, h: &mut dyn ServerHost, ty: ServerType, mission: &str) {
        if mission.is_empty() {
            h.error("createServer: mission name unspecified");
            return;
        }
        if h.is_world_limited() && !mission.to_ascii_lowercase().contains("tt1_") && !mission.to_ascii_lowercase().contains("/target.ron") {
            h.error("createServer: mission not available in demo");
            return;
        }
        self.destroy_server(h);
        self.session.mission_seq = 0;
        self.session.player_count = 0;
        self.session.server_type = Some(ty);
        if ty == ServerType::MultiPlayer {
            h.echo("Starting multiplayer mode");
            let port = h.setting(Setting::Port);
            self.port_init(h, &port);
            h.allow_connections(true);
            if !h.setting(Setting::DisplayOnMaster).eq_ignore_ascii_case("Never") {
                self.timers.schedule(h, 0, Timer::StartHeartbeat);
            }
        }
        h.new_server_group();
        self.server_created(h);
        self.load_mission(h, mission, true);
    }

    /// The server goes: its clients, its mission, its data.
    pub fn destroy_server(&mut self, h: &mut dyn ServerHost) {
        self.cancel_vote(h);
        self.session.voted_mission = None;
        self.session.server_type = None;
        self.session.mission_running = false;
        h.allow_connections(false);
        h.stop_heartbeat();
        if h.server_group_exists() {
            self.server_destroyed(h);
        }
        if h.mission_group_exists() {
            self.delete_mission_group(h);
        }
        self.delete_cleanup(h);
        if h.server_group_exists() {
            self.delete_server_group(h);
        }
        while let Some(&c) = self.group.first() {
            if self.kind(c).is_some_and(ClientKind::network) {
                self.dropped(h, c);
                h.delete_connection(c, "");
            }
            self.remove_client(h, c);
        }
        h.delete_defs();
    }

    /// The mission's objects go: the power-up timers on them too.
    pub(crate) fn delete_mission_group(&mut self, h: &mut dyn ServerHost) {
        let cleanup = self.cleanup_entities();
        self.cancel_entity_timers(h, |e| !cleanup.contains(&e));
        h.delete_mission_group();
    }

    /// The server's object group goes: a pending second load stage too.
    pub(crate) fn delete_server_group(&mut self, h: &mut dyn ServerHost) {
        self.timers.cancel_where(h, |t| *t == Timer::LoadMissionObjects);
        h.delete_server_group();
    }

    /// A dedicated server left alone with its bots starts its mission over.
    pub fn restart_mission(&mut self, h: &mut dyn ServerHost) {
        h.echo("Resetting server defaults...");
        let m = self.session.mission_file.clone();
        self.load_mission(h, &m, false);
    }

    /// Loads mission `name`: the first of a server, a solo level and a
    /// local game at once, a multiplayer server's next one after a pause.
    pub fn load_mission(&mut self, h: &mut dyn ServerHost, name: &str, first: bool) {
        if self.vote.is_some() {
            self.vote_failed(h, "the map changed");
        }
        self.end_mission(h);
        h.echo(&format!("*** LOADING MISSION: {name}"));
        h.echo("*** Stage 1 load");
        self.clear_center_print_all(h);
        self.clear_bottom_print_all(h);
        self.session.mission_seq += 1;
        self.session.mission_running = false;
        self.session.mission_file = name.to_string();
        self.mission_name = h.load_info(name);
        for c in self.group.clone() {
            self.send_mission_name(h, c);
        }
        if first || self.session.server_type == Some(ServerType::SinglePlayer) {
            self.load_mission_objects(h);
        } else {
            self.timers.schedule(h, self.setup.load_pause_ms, Timer::LoadMissionObjects);
        }
    }

    pub fn load_mission_objects(&mut self, h: &mut dyn ServerHost) {
        h.echo("*** Stage 2 load");
        let file = self.session.mission_file.clone();
        if !h.is_file(&file) {
            h.error(&format!("Could not find mission {file}"));
            return;
        }
        h.exec_mission(&file);
        if !h.mission_group_exists() {
            h.error(&format!("No 'MissionGroup' found in mission \"{file}\"."));
            return;
        }
        self.cleanup = Some(Vec::new());
        h.echo("*** Mission loaded");
        self.session.mission_running = true;
        let mut i = 0;
        while i < self.group.len() {
            let c = self.group[i];
            if self.kind(c).is_some_and(ClientKind::network) {
                self.send_mission(h, c);
            }
            i += 1;
        }
        self.mission_loaded(h);
    }

    /// Ends the mission: the clients are told, the objects go.
    pub fn end_mission(&mut self, h: &mut dyn ServerHost) {
        if !h.mission_group_exists() {
            return;
        }
        h.echo("*** ENDING MISSION");
        self.mission_ended(h);
        for c in self.group.clone() {
            if self.kind(c).is_some_and(ClientKind::network) {
                self.send_mission_end(h, c);
                h.reset_replication(c);
            }
            if let Some(x) = self.clients.get_mut(&c) {
                x.player = None;
            }
        }
        self.delete_mission_group(h);
        self.delete_cleanup(h);
        self.delete_server_group(h);
        h.new_server_group();
    }

    /// The round's objects go; a new round starts on the same mission.
    pub fn reset_mission(&mut self, h: &mut dyn ServerHost) {
        h.echo("*** MISSION RESET");
        self.delete_cleanup(h);
        self.cleanup = Some(Vec::new());
    }

    pub(crate) fn cancel_round_timers(&mut self, h: &mut dyn ServerHost) {
        if let Some(t) = self.round.end_timer.take() {
            self.timers.cancel(h, t);
        }
        if let Some(t) = self.round.solo_timer.take() {
            self.timers.cancel(h, t);
        }
    }

    pub(crate) fn schedule_round(&mut self, h: &mut dyn ServerHost, delay: u32, t: Timer) {
        let id = self.timers.schedule(h, delay, t);
        self.round.end_timer = Some(id);
    }

    pub fn server_created(&mut self, h: &mut dyn ServerHost) {
        if self.setup.single_player() {
            self.setup.team_game = true;
            self.setup.mission_type = MissionType::Deathmatch;
        }
        let mode = match self.setup.mission_type {
            MissionType::Deathmatch => "Battlemode",
            MissionType::Scrum => "Scrum",
        };
        self.session.mode_label = if self.setup.team_game { format!("Team {mode}") } else { mode.to_string() };
        self.round.start_ms = 0;
        if self.setup.team_game {
            let mut teams = Vec::new();
            for (color, long, short) in [(TeamColor::Blue, "Blue Team", "Blue"), (TeamColor::Green, "Green Team", "Green")] {
                let rec = TeamRecord { long_name: long.into(), short_name: short.into(), score: 0.0, cum_score: 0.0 };
                teams.push(Team { id: TeamId(self.alloc_id()), color, rec, members: Vec::new() });
            }
            self.teams = Some(teams);
        }
        h.exec_server_data();
        if self.setup.single_player() {
            self.start_campaign_server(h);
        } else {
            self.bot_group = Some(Vec::new());
            for i in 0..self.bot_slots.len() {
                if h.data().tanks.find(&self.bot_slots[i].tank).is_some() {
                    self.add_bot(h, i as u32);
                }
            }
        }
        self.sync_groups(h);
        self.refresh_player_list(h);
        self.round.start_ms = h.sim_time();
    }

    pub fn server_destroyed(&mut self, h: &mut dyn ServerHost) {
        if let Some(bots) = self.bot_group.clone() {
            for b in bots {
                self.remove_client(h, b);
            }
            self.bot_group = None;
        }
        self.teams = None;
        self.sync_groups(h);
        self.setup.target_range = Some(false);
        self.cancel_round_timers(h);
        if self.setup.single_player() {
            self.stop_campaign_server(h);
        }
    }

    pub fn mission_loaded(&mut self, h: &mut dyn ServerHost) {
        self.start_round(h);
    }

    pub fn mission_ended(&mut self, h: &mut dyn ServerHost) {
        if let Some(t) = self.round.end_timer.take() {
            self.timers.cancel(h, t);
        }
        self.round.running = Some(false);
        self.round.cycling = Some(false);
    }

    pub fn start_round(&mut self, h: &mut dyn ServerHost) {
        if self.round.running() {
            h.error("startGame: End the game first!");
            return;
        }
        self.round.player_in_game = false;
        let platform = h.platform().to_ascii_lowercase();
        let os = match platform.as_str() {
            "macos" => "Mac",
            "windows" => "Windows",
            // A Unix build names its flavour; none does.
            "x86unix" => "",
            _ => "Unknown",
        };
        let file = &self.session.mission_file;
        let world = ["tt1_", "tt2_", "tt3_"].iter().position(|w| file.to_ascii_lowercase().contains(w)).map_or(0, |i| i + 1);
        self.session.info = format!("{os}\t{world}\t{}\t0", self.mission_name);
        for cl in self.group.clone() {
            h.send(Some(cl), &ServerMsg::GameStart);
            self.with_record(Some(cl), |r| {
                r.score = Some(0.0);
                r.kills = Some(0);
                r.deaths = Some(0);
            });
        }
        for t in self.teams.iter_mut().flatten() {
            t.rec.score = 0.0;
        }
        if self.setup.single_player() {
            let l = self.local();
            self.with_record(l, |r| r.score = r.cum_score);
            self.round.solo_timer = None;
            self.round.end_timer = None;
        } else {
            self.bot_tanks = Some(Vec::new());
            if let Some(c) = self.cleanup.as_mut() {
                c.push(CleanupItem::BotGroup);
            }
            for bc in self.bots() {
                self.with_record(Some(bc), |r| r.score = Some(0.0));
            }
        }
        self.refresh_player_list(h);
        self.round.running = Some(true);
        self.round.start_ms = h.sim_time();
    }

    pub fn end_round(&mut self, h: &mut dyn ServerHost) {
        if !self.round.running() {
            h.error("endGame: No game running!");
            return;
        }
        self.cancel_round_timers(h);
        if self.vote.is_some() {
            self.vote_failed(h, "the round is over");
        }
        // A free-for-all battle counts for the players' records.
        let counted = !self.setup.team_game && self.setup.mission_type == MissionType::Deathmatch && !self.setup.single_player();
        for cl in self.group.clone() {
            let (synch, kills, deaths) = self.rec(Some(cl), |r| (r.synch_time.map(secs_text).unwrap_or_default(), r.kills.map(|k| k.to_string()).unwrap_or_default(), r.deaths.map(|d| d.to_string()).unwrap_or_default()));
            let key = tt_game::obfuscation::strswiz(&format!("{synch}echo"), "12");
            let msg = tt_game::obfuscation::strfrip(&key, &format!("{} {kills} {deaths}", counted as u32));
            h.game_end_key(cl, &synch);
            h.send(Some(cl), &ServerMsg::GameEnd { stats: msg });
        }
        self.reset_mission(h);
        self.round.running = Some(false);
    }

    pub fn round_warning(&mut self, h: &mut dyn ServerHost) {
        for cl in self.group.clone() {
            h.send(Some(cl), &ServerMsg::SyncClock { secs: "35".into(), dir: "1".into(), lc: false });
        }
        self.schedule_round(h, delay_ms(35.0), Timer::RoundEnd);
    }

    pub fn round_time_up(&mut self, h: &mut dyn ServerHost) {
        if self.setup.round_secs.trunc() != 0.0 {
            self.cycle_round(h);
        }
    }

    pub fn cycle_round(&mut self, h: &mut dyn ServerHost) {
        if !self.round.cycling() {
            self.round.cycling = Some(true);
            self.schedule_round(h, 0, Timer::EndRound);
        }
    }

    pub fn end_round_and_pause(&mut self, h: &mut dyn ServerHost) {
        self.end_round(h);
        let pause = delay_ms(self.setup.end_pause_secs as f32 as f64);
        self.schedule_round(h, pause, Timer::NextRound);
    }

    pub fn next_round(&mut self, h: &mut dyn ServerHost) {
        self.round.cycling = Some(false);
        let file = if self.setup.demo() {
            self.setup.demo_tank = self.demo_tank();
            if let Some(l) = self.local() {
                let tank = h.data().tanks.find(&self.setup.demo_tank);
                self.clients.get_mut(&l).unwrap().starting_tank = tank;
            }
            self.demo_mission(h)
        } else if self.setup.single_player() {
            self.next_solo_level(h)
        } else {
            self.next_in_rotation()
        };
        self.load_mission(h, &file, false);
    }

    /// The next round's mission: the one a map vote chose, else (a random
    /// order) a random other map of the rotation, or the one after the
    /// current one in it (the first after the last, or when the current
    /// one is not in it), else the current one again.
    pub fn next_in_rotation(&mut self) -> String {
        if self.session.voted_mission.is_none() && self.setup.rotation_random {
            return self.random_other_map();
        }
        let m = self.next_mission();
        self.session.voted_mission = None;
        m
    }

    /// A random map of the rotation other than the current one (one draw of
    /// the server's RNG; the current one again when it is the only map).
    fn random_other_map(&mut self) -> String {
        let cur = self.session.mission_file.clone();
        let others: Vec<String> = self.setup.rotation.iter().filter(|f| !f.eq_ignore_ascii_case(&cur)).cloned().collect();
        if others.is_empty() {
            return self.setup.rotation.first().cloned().unwrap_or(cur);
        }
        self.random_mission(&others)
    }

    /// The mission [`Server::next_in_rotation`] gives in turn (the vote's
    /// stays; a random order picks only when the round ends).
    pub fn next_mission(&self) -> String {
        if let Some(m) = &self.session.voted_mission {
            return m.clone();
        }
        let files = &self.setup.rotation;
        let cur = &self.session.mission_file;
        let next = match files.iter().position(|f| f.eq_ignore_ascii_case(cur)) {
            Some(i) => files.get(i + 1).or(files.first()),
            None => files.first(),
        };
        next.unwrap_or(cur).clone()
    }

    /// The round's timer from its start: the 35 s warning, or the end of a
    /// shorter round.
    pub(crate) fn schedule_duration(&mut self, h: &mut dyn ServerHost, secs: f32) {
        let secs = secs as f64;
        if secs > 35.0 {
            self.schedule_round(h, delay_ms(secs - 35.0), Timer::RoundWarning);
        } else if secs != 0.0 {
            self.schedule_round(h, delay_ms(secs), Timer::RoundEnd);
        }
    }

    pub fn enter_game(&mut self, h: &mut dyn ServerHost, this: ClientId) {
        if !self.round.player_in_game {
            self.round.solo_timer = None;
            self.round.start_ms = h.sim_time();
            self.round.end_timer = None;
            if self.setup.demo() {
                self.schedule_duration(h, self.setup.demo_round_secs);
            } else if !self.setup.single_player() {
                self.schedule_duration(h, self.setup.round_secs);
            }
        }
        if self.setup.single_player() {
            let id = self.start_solo_spawner(h);
            self.round.solo_timer = Some(id);
            let local = self.local();
            if self.campaign.kind() == Some(LevelKind::Brain) {
                let ticks = self.campaign.level().max_mission_ticks as f64;
                h.hud(Some(this), &HudMsg::ClockShowTime(format_g(ticks * 5.0 + 1.0)));
                h.hud(Some(this), &HudMsg::ClockSetTime(format_g(ticks * 5.0)));
                let n = h.group("BrainGroup").map(|g| g.len() as i32);
                self.with_record(local, |r| {
                    r.max_brains = n;
                    r.brains = Some(0);
                });
                let (max, brains) = self.rec(local, |r| (r.max_brains.map(|n| n.to_string()).unwrap_or_default(), r.brains.map(|n| n.to_string()).unwrap_or_default()));
                h.hud(Some(this), &HudMsg::BrainsMax(max));
                h.hud(Some(this), &HudMsg::BrainsCount(brains));
                h.hud(Some(this), &HudMsg::BrainsVisible("1".into()));
            } else {
                h.hud(Some(this), &HudMsg::ClockShowTime(format_g(-1.0)));
                h.hud(Some(this), &HudMsg::ClockVisible("0".into()));
                self.with_record(local, |r| {
                    r.max_brains = Some(0);
                    r.brains = Some(0);
                });
                h.hud(Some(this), &HudMsg::BrainsVisible("0".into()));
            }
        } else {
            let elapsed_ms = h.sim_time().wrapping_sub(self.round.start_ms);
            let sync = self.setup.round_secs as f64 - elapsed_ms as f64 / 1000.0;
            self.with_record(Some(this), |r| r.synch_time = Some(sync));
            let secs = secs_text(sync);
            h.send(Some(this), &ServerMsg::SyncClock { secs, dir: "0".into(), lc: false });
        }
        self.with_record(Some(this), |r| r.score = Some(0.0));
        self.spawn_player(h, this);
        if !self.round.player_in_game {
            if self.setup.single_player() {
                self.spawn_solo_start_bots(h);
            } else {
                if self.setup.mission_type == MissionType::Scrum {
                    if let Some(scrum) = h.data().balls.find("DefaultFlag").and_then(|b| h.new_flag(b)) {
                        self.cleanup_add(scrum);
                        let p = self.spawn_point(h, "");
                        let pos = vector_add(&p, "0 0 10");
                        h.set_transform(scrum, &pos);
                        if let Some(goal) = self.place_goal(h) {
                            h.set_goal(scrum, goal);
                        }
                    }
                } else if let Some(start) = self.practice_ball_start(h) {
                    // The Power-up Test's practice ball (battle modes).
                    if let Some(ball) = h.data().balls.find(super::pickups::PRACTICE_BALL_DATA).and_then(|b| h.new_flag(b)) {
                        self.cleanup_add(ball);
                        h.set_transform(ball, &start);
                    }
                }
                for bc in self.bots() {
                    self.spawn_bot(h, bc);
                }
            }
        }
        self.round.player_in_game = true;
    }

    pub fn demo_tank(&mut self) -> String {
        match self.rng.get_random_max(2) {
            0 => "LightTank".into(),
            1 => "MediumTank".into(),
            _ => "HeavyTank".into(),
        }
    }

    /// A random one of `files`.
    pub(crate) fn random_mission(&mut self, files: &[String]) -> String {
        let r = self.rng.get_random_max(files.len() as i32 - 1);
        files.get(r as usize).cloned().unwrap_or_default()
    }

    pub fn demo_mission(&mut self, h: &mut dyn ServerHost) -> String {
        let team = !((self.rng.get_random_max(1) as f64) < 1.0 && self.teams.is_some());
        self.setup.team_game = team;
        self.setup.mission_type = MissionType::Deathmatch;
        let files = h.maps(None);
        if files.is_empty() {
            return String::new();
        }
        self.random_mission(&files)
    }
}

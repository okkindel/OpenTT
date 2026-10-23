//! The native server headless, against a recording host: a free-for-all
//! with two bots, the local player joining through the download phases,
//! a kill and the round cycle.

use std::collections::HashMap;
use tt_gameplay::entity::EntityId;
use tt_gameplay::msg::{HudMsg, MsgKind, ServerMsg};
use tt_gameplay::server::{BotSlot, CampaignStep, ClientId, ClientKind, GroupMember, MissionType, Server, ServerHost, ServerType, Setting, SoloLevel, SoloScales, TimerId};
use tt_gameplay::data::{AiDef, BallDef, GameData, PickupDef, ProjectileDef, Ref, SoundDef, TankDef};
use tt_assets::mission::SceneObject;

#[derive(Default)]
struct Host {
    time: u32,
    next_timer: u32,
    timers: Vec<(u32, u32)>,
    sent: Vec<(Option<ClientId>, ServerMsg)>,
    next_entity: u32,
    data: GameData,
    entities: HashMap<u32, (String, String)>,
    corpses: Vec<u32>,
    mission: bool,
    log: Vec<String>,
    /// `server.max_players` (empty: 10) and `server.replace_bots`.
    max_players: String,
    replace_bots: bool,
    /// `server.map_rotation`.
    rotation: String,
}

impl Host {
    fn new() -> Host {
        let obj = |class: &str, name: &str, fields: &[(&str, &str)]| SceneObject { class: class.into(), name: name.into(), def: true, parent: None, fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), children: Vec::new() };
        let objects = vec![
            obj("TankAiDef", "AiData", &[]),
            obj("ProjectileDef", "DefaultProjectile", &[]),
            obj("TankDef", "GoldHeavyTank", &[("ai", "AiData"), ("default_projectile", "DefaultProjectile")]),
            obj("TankDef", "BronzeLightTank", &[("ai", "AiData"), ("default_projectile", "DefaultProjectile")]),
            obj("TankDef", "MediumTank", &[("default_projectile", "DefaultProjectile")]),
            obj("TankDef", "LightTank", &[("default_projectile", "DefaultProjectile")]),
        ];
        Host { next_entity: 1, next_timer: 1, data: GameData::build(&objects, &[]), ..Default::default() }
    }
    fn new_entity(&mut self, db: &str) -> EntityId {
        let id = self.next_entity;
        self.next_entity += 1;
        self.entities.insert(id, (db.into(), String::new()));
        EntityId(id)
    }
    fn due(&mut self) -> Vec<u32> {
        let t = self.time;
        let (d, rest): (Vec<_>, Vec<_>) = self.timers.iter().partition(|(at, _)| *at <= t);
        self.timers = rest;
        d.into_iter().map(|(_, id)| id).collect()
    }
}

impl ServerHost for Host {
    fn setting(&self, s: Setting) -> String {
        match s {
            Setting::MaxPlayers if self.max_players.is_empty() => "10".into(),
            Setting::MaxPlayers => self.max_players.clone(),
            Setting::ReplaceBots => (self.replace_bots as u8).to_string(),
            Setting::MapRotation => self.rotation.clone(),
            _ => String::new(),
        }
    }
    fn set_setting(&mut self, s: Setting, v: &str) {
        if s == Setting::MapRotation {
            self.rotation = v.into();
        }
    }
    fn bot_settings(&self) -> Vec<BotSlot> {
        Vec::new()
    }
    fn combat_experience(&self) -> f64 {
        0.0
    }
    fn offer_high_score(&mut self, _: f64) {}
    fn platform(&self) -> String {
        "x86Unix".into()
    }
    fn sim_time(&self) -> u32 {
        self.time
    }
    fn schedule(&mut self, delay_ms: u32) -> TimerId {
        let id = self.next_timer;
        self.next_timer += 1;
        self.timers.push((self.time + delay_ms, id));
        TimerId(id)
    }
    fn cancel(&mut self, id: TimerId) {
        self.timers.retain(|(_, t)| *t != id.0);
    }
    fn echo(&mut self, text: &str) {
        self.log.push(text.into());
    }
    fn error(&mut self, text: &str) {
        self.log.push(format!("error: {text}"));
    }
    fn send(&mut self, to: Option<ClientId>, msg: &ServerMsg) {
        self.sent.push((to, msg.clone()));
    }
    fn hud(&mut self, _: Option<ClientId>, _: &HudMsg) {}
    fn play2d(&mut self, _: ClientId, _: Ref<SoundDef>) {}
    fn play3d(&mut self, _: ClientId, _: Ref<SoundDef>, _: &str) {}
    fn address(&self, _: ClientId) -> String {
        "local".into()
    }
    fn transmit_defs(&mut self, _: ClientId, _: &str) {}
    fn start_replication(&mut self, _: ClientId, _: &str) {}
    fn reset_replication(&mut self, _: ClientId) {}
    fn set_control(&mut self, _: ClientId, _: EntityId) {}
    fn delete_connection(&mut self, _: ClientId, _: &str) {}
    fn ban(&mut self, _: &str, _: &str, _: &str) {}
    fn allow_connections(&mut self, _: bool) {}
    fn start_heartbeat(&mut self) {}
    fn stop_heartbeat(&mut self) {}
    fn set_net_port(&mut self, _: &str) {}
    fn is_file(&self, f: &str) -> bool {
        !f.contains("nope")
    }
    fn maps(&mut self, _: Option<u8>) -> Vec<String> {
        vec!["missions/tt1_1.ron".into()]
    }
    fn load_info(&mut self, f: &str) -> String {
        match tt_assets::layout::mission_id(f).as_str() {
            "tt2_3" => "Snowblind".into(),
            "tt3_1" => "Hot Rocks".into(),
            _ => "Medulla Mesa".into(),
        }
    }
    fn new_server_group(&mut self) {}
    fn delete_server_group(&mut self) -> bool {
        true
    }
    fn server_group_exists(&self) -> bool {
        true
    }
    fn exec_mission(&mut self, _: &str) -> bool {
        self.mission = true;
        true
    }
    fn mission_group_exists(&self) -> bool {
        self.mission
    }
    fn delete_mission_group(&mut self) {
        self.mission = false;
    }
    fn music_track(&self) -> String {
        "Lush".into()
    }
    fn group(&self, name: &str) -> Option<Vec<GroupMember>> {
        name.eq_ignore_ascii_case("PlayerDropPoints").then(|| (0..4).map(|i| GroupMember::Object(format!("{} 0 100 1 0 0 0", i * 10))).collect())
    }
    fn exec_server_data(&mut self) {}
    fn delete_defs(&mut self) {}
    fn data(&self) -> &GameData {
        &self.data
    }
    fn set_tank_scales(&mut self, _: Option<SoloScales>) {}
    fn new_tank(&mut self, def: Ref<TankDef>, _: Option<ClientId>) -> Option<EntityId> {
        let name = self.data.tanks[def].name.clone();
        Some(self.new_entity(&name))
    }
    fn new_flag(&mut self, _: Ref<BallDef>) -> Option<EntityId> {
        Some(self.new_entity("Ball"))
    }
    fn new_power_up(&mut self, _: &str, _: Ref<PickupDef>) -> Option<EntityId> {
        Some(self.new_entity("Pickup"))
    }
    fn tank_def(&self, e: EntityId) -> Option<Ref<TankDef>> {
        self.entities.get(&e.0).and_then(|x| self.data.tanks.find(&x.0))
    }
    fn pickup_def(&self, _: EntityId) -> Option<Ref<PickupDef>> {
        None
    }
    fn ball_def(&self, _: EntityId) -> Option<Ref<BallDef>> {
        None
    }
    fn set_tank_def(&mut self, _: EntityId, _: Ref<TankDef>) {}
    fn set_pickup_def(&mut self, _: EntityId, _: Ref<PickupDef>) {}
    fn delete_entity(&mut self, e: EntityId) {
        self.entities.remove(&e.0);
    }
    fn find_entity(&self, _: &str) -> Option<EntityId> {
        None
    }
    fn entity_exists(&self, e: EntityId) -> bool {
        self.entities.contains_key(&e.0)
    }
    fn set_transform(&mut self, e: EntityId, t: &str) {
        if let Some(x) = self.entities.get_mut(&e.0) {
            x.1 = t.into();
        }
    }
    fn transform(&self, e: EntityId) -> String {
        self.entities.get(&e.0).map(|x| x.1.clone()).unwrap_or_default()
    }
    fn set_ai(&mut self, _: EntityId, _: Option<Ref<AiDef>>) {}
    fn set_team(&mut self, _: EntityId, _: &str) {}
    fn set_player_name(&mut self, _: EntityId, _: &str) {}
    fn set_projectile(&mut self, _: EntityId, _: Ref<ProjectileDef>) {}
    fn is_corpse(&self, e: EntityId) -> bool {
        self.corpses.contains(&e.0)
    }
    fn hurt(&mut self, _: EntityId, _: f32) {}
    fn play_user_thread(&mut self, _: EntityId, _: u32, _: f32, _: bool) {}
    fn flag_of(&self, _: EntityId) -> Option<EntityId> {
        None
    }
    fn set_goal(&mut self, _: EntityId, _: EntityId) {}
    fn clear_mount(&mut self, _: EntityId) {}
    fn next_sp_mission(&mut self, _: bool) -> Option<CampaignStep> {
        None
    }
    fn solo_level(&self, _: u32) -> Option<SoloLevel> {
        None
    }
    fn is_demo(&self) -> bool {
        false
    }
    fn is_release(&self) -> bool {
        true
    }
    fn is_world_limited(&self) -> bool {
        false
    }
    fn groups_changed(&mut self, _: &[ClientId], _: &[ClientId], _: &[(tt_gameplay::server::TeamId, &'static str)]) {}
    fn game_end_key(&mut self, _: ClientId, _: &str) {}
}

fn run_timers(s: &mut Server, h: &mut Host) {
    for id in h.due() {
        s.fire(h, TimerId(id));
    }
}

#[test]
fn free_for_all_round() {
    let mut h = Host::new();
    let mut s = Server::new();
    s.setup.mission_type = MissionType::Deathmatch;
    s.setup.team_game = false;
    s.setup.round_secs = 60.0;
    s.bot_slots = vec![BotSlot::new("GoldHeavyTank", "Hum [Bot]", ""), BotSlot::new("BronzeLightTank", "Mal [Bot]", "")];
    s.create_server(&mut h, ServerType::SinglePlayer, "missions/tt1_1.ron");
    assert_eq!(s.bots().len(), 2);
    assert!(s.round.running());
    // The local player joins.
    let me = s.add_client(&mut h, ClientKind::Local);
    assert_eq!(s.connect_request(&mut h, "local"), "");
    s.connected(&mut h, me, "Tester", "", "1");
    assert_eq!(s.name_text(me), "Tester");
    assert!(h.sent.iter().any(|(to, m)| *to == Some(me) && matches!(m, ServerMsg::MissionPhase1 { .. })));
    s.phase_ack(&mut h, me, 1, "1");
    s.defs_done(&mut h, me, "1");
    s.phase_ack(&mut h, me, 2, "1");
    s.phase_ack(&mut h, me, 3, "1");
    // The player and both bots have tanks at spawn points.
    let mine = s.client(me).unwrap().player.unwrap();
    assert!(h.transform(mine).ends_with(" 0 100 1 0 0 0"));
    assert_eq!(s.cleanup_tanks().len(), 3);
    // The player kills a bot: 2 points, the bot respawns.
    let bot = s.bots()[0];
    let victim = s.client(bot).unwrap().player.unwrap();
    h.corpses.push(victim.0);
    s.tank_destroyed(&mut h, victim, Some(mine));
    assert_eq!(s.record(Some(me)).unwrap().score, Some(2.0));
    assert_eq!(s.record(Some(bot)).unwrap().score, Some(0.0));
    assert_eq!(s.record(Some(bot)).unwrap().deaths, Some(1));
    assert_ne!(s.client(bot).unwrap().player, Some(victim));
    assert!(h.sent.iter().any(|(_, m)| matches!(m, ServerMsg::Message { kind: MsgKind::ClientKilled { .. }, .. })));
    // The round ends after its duration, the next starts.
    // (60 s: the 35 s warning at 25 s, the end at 60 s, 3 s pause.)
    while h.time < 70_000 {
        h.time += 500;
        run_timers(&mut s, &mut h);
    }
    assert!(h.sent.iter().any(|(_, m)| matches!(m, ServerMsg::SyncClock { secs, .. } if secs == "35")));
    assert!(h.sent.iter().any(|(_, m)| matches!(m, ServerMsg::GameEnd { .. })));
    assert_eq!(s.session.mission_seq, 2);
    assert_eq!(s.record(Some(me)).unwrap().cum_score, Some(2.0));
    // Leaving: the drop message, the record goes.
    s.drop_client(&mut h, me);
    assert!(s.client(me).is_none());
}

#[test]
fn full_server_lets_the_host_in_and_bots_make_room() {
    // A "1vs1" server (2 players) with three bots.
    let mut h = Host { max_players: "2".into(), ..Host::new() };
    let mut s = Server::new();
    s.setup.mission_type = MissionType::Deathmatch;
    s.bot_slots = vec![BotSlot::new("GoldHeavyTank", "Hum [Bot]", ""), BotSlot::new("BronzeLightTank", "Mal [Bot]", ""), BotSlot::new("GoldHeavyTank", "Tom [Bot]", "")];
    s.create_server(&mut h, ServerType::SinglePlayer, "missions/tt1_1.ron");
    assert_eq!(s.bots().len(), 3);
    // The host itself always gets in; others not while the bots fill it.
    assert_eq!(s.connect_request(&mut h, "local"), "");
    assert_eq!(s.connect_request(&mut h, "IP:203.0.113.5:28000"), "CR_SERVERFULL");
    // With bot replacement a human takes a bot's place: only humans count.
    h.replace_bots = true;
    assert_eq!(s.connect_request(&mut h, "IP:203.0.113.5:28000"), "");
}

const A: &str = "missions/tt1_1.ron";
const B: &str = "missions/tt2_3.ron";
const C: &str = "missions/tt3_1.ron";

#[test]
fn rotation_order_and_wrap() {
    let mut s = Server::new();
    s.setup.rotation = vec![A.into(), B.into(), C.into()];
    let next = |s: &mut Server, cur: &str| {
        s.session.mission_file = cur.into();
        s.next_in_rotation()
    };
    assert_eq!(next(&mut s, A), B);
    assert_eq!(next(&mut s, B), C);
    assert_eq!(next(&mut s, C), A, "wraps");
    assert_eq!(next(&mut s, "missions/tt1_2.ron"), A, "not in it: the first");
    // A passed vote's map comes once, then the rotation goes on after it.
    s.session.voted_mission = Some(C.into());
    assert_eq!(next(&mut s, A), C);
    assert_eq!(next(&mut s, C), A);
    // No rotation: the mission again.
    s.setup.rotation.clear();
    assert_eq!(next(&mut s, B), B);
    // The settings' rotation (unknown ids left out, duplicates once), else
    // the mission alone.
    let mut h = Host::new();
    h.rotation = "tt2_3, TT1_1 nope\ttt2_3".into();
    assert_eq!(s.rotation_setting(&mut h, C), vec![B.to_string(), A.to_string()]);
    assert!(h.log.iter().any(|l| l.contains("no mission nope")));
    h.rotation.clear();
    assert_eq!(s.rotation_setting(&mut h, C), vec![C.to_string()]);
    s.set_rotation(&mut h, vec![C.into(), A.into()]);
    assert_eq!(h.rotation, "tt3_1\ttt1_1");
}

/// The last vote line player `c` was told.
fn vote_line(h: &Host, c: ClientId) -> String {
    h.sent
        .iter()
        .rev()
        .find_map(|(to, m)| match m {
            ServerMsg::Message { kind: MsgKind::Other { kind, .. }, text } if *to == Some(c) && kind.text() == "MsgVote" => Some(text.text().trim_start_matches('\u{3}').to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The last vote status player `c` got: (map, yes, no, players).
fn vote_status(h: &Host, c: ClientId) -> Option<(String, u32, u32, u32)> {
    h.sent.iter().rev().find_map(|(to, m)| match m {
        ServerMsg::VoteStatus { map, yes, no, players, .. } if *to == Some(c) => Some((map.clone(), *yes, *no, *players)),
        _ => None,
    })
}

fn wait(s: &mut Server, h: &mut Host, ms: u32) {
    let end = h.time + ms;
    while h.time < end {
        h.time += 100;
        run_timers(s, h);
    }
}

#[test]
fn map_votes() {
    let mut h = Host::new();
    h.rotation = "tt1_1 tt2_3 tt3_1".into();
    let mut s = Server::new();
    s.launch_dedicated(&mut h, A);
    s.bot_slots = vec![BotSlot::new("GoldHeavyTank", "Hum [Bot]", ""), BotSlot::new("BronzeLightTank", "Mal [Bot]", "")];
    for i in 0..2 {
        s.add_bot(&mut h, i);
    }
    assert_eq!(s.setup.rotation, vec![A.to_string(), B.to_string(), C.to_string()]);
    let p: Vec<ClientId> = (0..3)
        .map(|i| {
            let c = s.add_client(&mut h, ClientKind::Remote);
            s.connected(&mut h, c, &format!("P{i}"), "", "1");
            c
        })
        .collect();
    // A fake client (no player) and the bots do not count.
    s.add_fake_client(&mut h, "");
    assert_eq!(s.bots().len(), 2);
    assert_eq!(s.humans(), p);
    // Not in the first 3 minutes on the server.
    s.call_vote(&mut h, p[0], "tt2_3");
    assert_eq!(vote_line(&h, p[0]), "Wait 3:00 before calling a vote.");
    wait(&mut s, &mut h, 50_000);
    s.call_vote(&mut h, p[0], "tt2_3");
    assert_eq!(vote_line(&h, p[0]), "Wait 2:10 before calling a vote.");
    wait(&mut s, &mut h, 130_000);
    // Not a map of the rotation; the map being played.
    s.call_vote(&mut h, p[0], "tt1_2");
    assert_eq!(vote_line(&h, p[0]), "\"tt1_2\" is not in the rotation.");
    s.call_vote(&mut h, p[0], "medulla");
    assert_eq!(vote_line(&h, p[0]), "Medulla Mesa is being played already.");
    assert!(s.vote.is_none());
    // A vote by name: the caller votes yes; one at a time.
    s.call_vote(&mut h, p[0], "snowblind");
    assert_eq!(vote_line(&h, p[2]), "P0 called a vote: switch to Snowblind?");
    assert_eq!(vote_status(&h, p[1]), Some(("Snowblind".into(), 1, 0, 3)));
    s.call_vote(&mut h, p[1], "hot rocks");
    assert_eq!(vote_line(&h, p[1]), "A vote is already running.");
    // Bots never vote.
    s.cast_vote(&mut h, s.bots()[0], true);
    assert_eq!(s.vote.as_ref().unwrap().yes.len(), 1);
    // A second yes is more than half of 3: passed, the round ends, the map
    // loads, the rotation goes on after it.
    s.cast_vote(&mut h, p[1], true);
    assert!(s.vote.is_none());
    assert_eq!(vote_line(&h, p[2]), "Vote passed: switching to Snowblind.");
    assert_eq!(vote_status(&h, p[2]).unwrap().0, "", "the prompt goes");
    wait(&mut s, &mut h, 10_000);
    assert!(h.sent.iter().any(|(_, m)| matches!(m, ServerMsg::GameEnd { .. })), "the usual round end");
    assert_eq!(s.session.mission_file, B);
    assert!(s.round.running());
    assert_eq!(s.next_in_rotation(), C);
    // The caller waits 3 minutes for his next vote; two noes out of three
    // make a yes majority impossible.
    s.call_vote(&mut h, p[0], "tt3_1");
    assert!(vote_line(&h, p[0]).starts_with("Wait 2:"), "{}", vote_line(&h, p[0]));
    s.call_vote(&mut h, p[1], "tt3_1");
    s.cast_vote(&mut h, p[0], false);
    assert_eq!(vote_status(&h, p[0]), Some(("Hot Rocks".into(), 1, 1, 3)));
    // The same vote again sends nothing; a change within a second is ignored.
    let sent = h.sent.len();
    s.cast_vote(&mut h, p[0], false);
    s.cast_vote(&mut h, p[0], true);
    assert_eq!((h.sent.len(), vote_status(&h, p[0])), (sent, Some(("Hot Rocks".into(), 1, 1, 3))));
    wait(&mut s, &mut h, 1_000);
    s.cast_vote(&mut h, p[2], false);
    assert_eq!(vote_line(&h, p[0]), "Vote for Hot Rocks failed: too many players voted no.");
    // Nobody else votes: it fails after 30 s.
    s.call_vote(&mut h, p[2], "tt1_1");
    wait(&mut s, &mut h, 29_000);
    assert!(s.vote.is_some());
    wait(&mut s, &mut h, 1_000);
    assert!(s.vote.is_none());
    assert_eq!(vote_line(&h, p[1]), "Vote for Medulla Mesa failed: time is up.");
    s.cast_vote(&mut h, p[1], true);
    assert_eq!(vote_line(&h, p[1]), "No vote is running.");
    // A player leaving: the votes of those present count.
    wait(&mut s, &mut h, 180_000);
    s.call_vote(&mut h, p[0], "tt1_1");
    s.cast_vote(&mut h, p[1], false);
    s.drop_client(&mut h, p[2]);
    assert_eq!(vote_line(&h, p[0]), "Vote for Medulla Mesa failed: too many players voted no.");
    // Alone on the server: one yes is a majority.
    s.drop_client(&mut h, p[1]);
    wait(&mut s, &mut h, 180_000);
    s.call_vote(&mut h, p[0], "tt3_1");
    assert_eq!(vote_line(&h, p[0]), "Vote passed: switching to Hot Rocks.");
}

#[test]
fn no_votes_in_a_local_game() {
    let mut h = Host::new();
    let mut s = Server::new();
    s.create_server(&mut h, ServerType::SinglePlayer, A);
    let me = s.add_client(&mut h, ClientKind::Local);
    s.connected(&mut h, me, "Tester", "", "1");
    h.time = 400_000;
    s.call_vote(&mut h, me, "tt1_1");
    assert_eq!(vote_line(&h, me), "There is no voting in this game.");
    s.send_map_list(&mut h, me);
    assert!(!h.sent.iter().any(|(_, m)| matches!(m, ServerMsg::MapList { .. })));
}

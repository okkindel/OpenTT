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
            Setting::MaxPlayers => "10".into(),
            _ => String::new(),
        }
    }
    fn set_setting(&mut self, _: Setting, _: &str) {}
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
    fn is_file(&self, _: &str) -> bool {
        true
    }
    fn maps(&mut self, _: Option<u8>) -> Vec<String> {
        vec!["missions/tt1_1.ron".into()]
    }
    fn load_info(&mut self, _: &str) -> String {
        "Medulla Mesa".into()
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

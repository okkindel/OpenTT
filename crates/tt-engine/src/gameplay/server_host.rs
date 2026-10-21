//! The engine side of the native game server (`tt_gameplay::server`):
//! [`ServerGlue`] owns the [`Server`] (with its typed state) and what the
//! engine keeps about it; [`Bridge`] is its [`ServerHost`]; the entry
//! points ([`launch_campaign`], [`connected`], [`client_cmd`], ...) are
//! what the menu, the connections and the console call.
//!
//! * Timers are events of the engine's queue ([`crate::sched`]), so they
//!   keep their order with the client's own timed events.
//! * The server's [`tt_game::rng::EngineRng`] is the engine's
//!   (`EngineHost::rng`): the state is handed over at every entry.
//! * Messages go as they are ([`ServerMsg`]) to the local client
//!   (`crate::net::send_to_client`) and over the network.
//! * Tanks the server creates before the mission's world exists (the
//!   target range's targets) wait in [`ServerGlue::pending`] and join the
//!   world when it is built, at the transform the server gave them.
//! * The server never runs re-entrantly: a callback that arrives while it
//!   runs (a connection's drop during its own deletion) is the server's
//!   own doing and is ignored.

use std::collections::{HashMap, HashSet};

use glam::{Mat3, Vec3};
use tt_gameplay::entity::EntityId;
use tt_gameplay::msg::{ClientCmd, HudMsg, ServerMsg};
use tt_gameplay::server::launch::LocalConnect;
use tt_gameplay::server::{BotSlot, CampaignStep, ClientId, ClientKind, ClientRecord, GroupMember, LevelKind, MissionType, Server, ServerHost, Setting, SoloLevel, SoloScales, TeamColor, TeamId, TimerId};
use tt_gameplay::data::{AiDef, BallDef, GameData, PickupDef, ProjectileDef, Ref, SoundDef, TankDef};

use crate::game::{fields_transform, parse_transform, set_transform_fields, team_index, rotation_rad};
use crate::host::EngineHost;
use crate::net::Conn;

/// A tank the server created before the world existed.
#[derive(Debug, Clone)]
pub struct Pending {
    pub id: EntityId,
    pub def: String,
    pub owner: Option<ClientId>,
    /// `position` / `rotation` fields as `setTransform` wrote them.
    pub xf: (String, String),
    pub ai: String,
    pub team: String,
    pub name: String,
}

/// The server and the engine's view of it.
#[derive(Debug, Default)]
pub struct ServerGlue {
    /// None while it runs (taken by [`with_server`]).
    pub server: Option<Server>,
    /// Connection → client.
    pub conns: HashMap<Conn, ClientId>,
    /// Test clients (connections without a network link).
    pub tests: HashSet<ClientId>,
    /// Test and fake clients by the name the harness gave them (lowercase).
    pub named: HashMap<String, ClientId>,
    /// `ClientGroup`, `BotClientGroup`, the teams (as the server last said).
    pub group: Vec<ClientId>,
    pub bots: Vec<ClientId>,
    pub teams: Vec<(TeamId, &'static str)>,
    /// `synch_time` of the clients a `GameEnd` went to (traces).
    pub game_end_keys: HashMap<ClientId, String>,
    pub pending: Vec<Pending>,
    /// The next entity id (unique across worlds).
    pub next_entity: u32,
    /// `setSPTankParams`: the solo scales of the campaign's tank defs.
    pub solo_scales: Option<SoloScales>,
    /// `ServerGroup` exists.
    pub server_group: bool,
}

impl ServerGlue {
    pub fn new() -> ServerGlue {
        ServerGlue { server: Some(Server::new()), next_entity: 1, ..Default::default() }
    }

    /// The client of a connection.
    pub fn conn_client(&self, o: Conn) -> Option<ClientId> {
        self.conns.get(&o).copied()
    }

    /// The connection of a client.
    pub fn client_conn(&self, c: ClientId) -> Option<Conn> {
        self.conns.iter().find(|(_, x)| **x == c).map(|(o, _)| *o)
    }

    /// The server, when it is not running.
    pub fn get(&self) -> Option<&Server> {
        self.server.as_ref()
    }

    /// The server's state. The engine reads it between the server's own
    /// calls only (what a callback needs comes with it).
    pub fn state(&self) -> &Server {
        self.server.as_ref().expect("server state read while the server runs")
    }

    pub fn state_mut(&mut self) -> &mut Server {
        self.server.as_mut().expect("server state changed while the server runs")
    }
}

/// Runs `f` on the server (None while it runs already).
pub fn with_server<R>(h: &mut EngineHost, f: impl FnOnce(&mut Server, &mut Bridge) -> R) -> Option<R> {
    let mut s = h.srv.server.take()?;
    s.rng = h.rng.clone();
    let r = {
        let mut b = Bridge { h: &mut *h };
        f(&mut s, &mut b)
    };
    h.rng = s.rng.clone();
    h.srv.server = Some(s);
    Some(r)
}

/// The world of the mission now loaded (not a previous mission's).
fn world_current(h: &EngineHost) -> bool {
    h.game.as_ref().is_some_and(|g| h.mission_load.as_ref().is_some_and(|l| l.group == g.mission_group))
}

/// The next entity id, past every world's.
fn next_entity(h: &mut EngineHost) -> EntityId {
    let n = h.srv.next_entity.max(h.game.as_ref().map_or(1, |g| g.entities.next_id()));
    h.srv.next_entity = n + 1;
    EntityId(n)
}

/// The world takes the tanks created before it (in creation order).
pub fn materialize_pending(h: &mut EngineHost, g: &mut crate::game::GameSim) {
    for p in std::mem::take(&mut h.srv.pending) {
        let (pos, rot) = parse_transform(&format!("{} {}", p.xf.0, rotation_rad(&p.xf.1)));
        let Some(e) = g.add_tank(&p.def, "", pos, rot.unwrap_or(Mat3::IDENTITY), Some(p.id)) else { continue };
        let Some(t) = g.tank_of(e) else { continue };
        if let Some(ent) = g.entities.get_mut(e) {
            ent.owner = p.owner.map(|c| c.0);
        }
        if !p.ai.is_empty() {
            let ai = g.declared(&p.ai);
            g.set_ai(t, &ai);
        }
        if let Some(team) = team_index(&p.team) {
            g.set_team(t, team);
        }
        g.world.set_player_name(t, &p.name);
    }
}

/// The engine as the server sees it.
pub struct Bridge<'a> {
    pub h: &'a mut EngineHost,
}

impl Bridge<'_> {
    /// An entity's def name (a tank waiting for the world, or the world's).
    fn def_of(&self, e: EntityId) -> String {
        if let Some(p) = self.h.srv.pending.iter().find(|p| p.id == e) {
            return p.def.clone();
        }
        self.h.game.as_ref().and_then(|g| g.entities.get(e)).map(|x| x.def.clone()).unwrap_or_default()
    }

    /// A tank or power-up becomes another of its kind.
    fn set_def(&mut self, e: EntityId, d: String) {
        if let Some(p) = self.pending(e) {
            p.def = d;
            return;
        }
        if let Some(g) = self.h.game.as_mut() {
            g.set_def(e, &d);
        }
    }

    fn conn(&self, c: ClientId) -> Option<Conn> {
        self.h.srv.client_conn(c)
    }

    fn remote(&self, c: ClientId) -> Option<Conn> {
        self.conn(c).filter(|o| crate::netgame::is_remote(self.h, *o))
    }

    fn local(&self, c: ClientId) -> bool {
        self.conn(c).is_some_and(|o| Some(o) == self.h.net.server)
    }

    /// A sound for one client: over the network to a remote one, else here.
    fn play(&mut self, to: ClientId, sound: Ref<SoundDef>, pos: Option<[f32; 3]>) {
        let profile = &self.h.data.sounds[sound].name;
        let Some(n) = self.h.audio.profile_name(profile) else { return };
        match self.remote(to) {
            Some(conn) => crate::netgame::play_sound(self.h, conn, &n, pos),
            None => {
                self.h.audio.play_profile(&n, pos);
            }
        }
    }

    fn pending(&mut self, e: EntityId) -> Option<&mut Pending> {
        self.h.srv.pending.iter_mut().find(|p| p.id == e)
    }

    fn game(&mut self) -> Option<&mut crate::game::GameSim> {
        if world_current(self.h) {
            self.h.game.as_mut()
        } else {
            None
        }
    }

    fn game_ref(&self) -> Option<&crate::game::GameSim> {
        if world_current(self.h) {
            self.h.game.as_ref()
        } else {
            None
        }
    }
}

/// The settings store's key of a server setting.
pub fn setting_key(s: Setting) -> &'static str {
    match s {
        Setting::MaxPlayers => "server.max_players",
        Setting::ReplaceBots => "server.replace_bots",
        Setting::Port => "server.port",
        Setting::DisplayOnMaster => "server.display_on_master",
        Setting::AdminPassword => "server.admin_password",
        Setting::MaxChatLen => "server.max_chat_len",
        Setting::FloodProtection => "server.flood_protection_enabled",
        Setting::KickBanTime => "server.kick_ban_time",
        Setting::BanTime => "server.ban_time",
        Setting::ConnectionError => "server.connection_error",
        Setting::PlayerName => "player.name",
        Setting::PlayerTeam => "player.team",
        Setting::PlayerTank => "player.tank",
    }
}

/// The bots of the settings (`server.max_bots` slots of `server.bot.<i>`,
/// `server.bot_name.<i>`, `server.bot_team.<i>`).
pub fn bot_settings(prefs: &tt_shell::Prefs) -> Vec<BotSlot> {
    let max = tt_game::fmt::atof(&prefs.get("server.max_bots")) as f32 as f64;
    let mut slots = Vec::new();
    let mut i = 0;
    while (i as f64) < max {
        slots.push(BotSlot::new(&prefs.get(&format!("server.bot.{i}")), &prefs.get(&format!("server.bot_name.{i}")), &prefs.get(&format!("server.bot_team.{i}"))));
        i += 1;
    }
    slots
}

/// A solo level's numbers from the level table (`level.field(f)` texts).
pub fn solo_level(l: &tt_shell::solo::SpLevel) -> SoloLevel {
    let f = |n: &str| tt_game::fmt::atof(&l.field(n)) as f32;
    let d = |n: &str| tt_game::fmt::atof(&l.field(n));
    SoloLevel {
        mission: l.mission.clone(),
        kind: LevelKind::parse(&l.level_type).unwrap_or_default(),
        damage_scale: f("damage_scale"),
        armor_scale: f("armor_scale"),
        fire_scale: f("fire_scale"),
        start_bots: f("start_bots"),
        max_live_bots: f("max_live_bots"),
        max_spawn_bots: f("max_spawn_bots"),
        end_bots: f("end_bots"),
        min_bot_ticks: f("min_bot_ticks"),
        max_bot_ticks: f("max_bot_ticks"),
        max_mission_ticks: f("max_mission_ticks"),
        quick_shot_time: f("quick_shot_time"),
        bronze: d("bronze"),
        silver: d("silver"),
        gold: d("gold"),
        light: d("light"),
        medium: d("medium"),
        heavy: d("heavy"),
        bronze_points: f("bronze_pts"),
        silver_points: f("silver_pts"),
        gold_points: f("gold_pts"),
        boss_points: f("boss_pts"),
        brain_points: f("brain_pts"),
    }
}

impl ServerHost for Bridge<'_> {
    fn setting(&self, s: Setting) -> String {
        self.h.shell.prefs.get(setting_key(s))
    }

    fn set_setting(&mut self, s: Setting, value: &str) {
        let v = match s {
            Setting::PlayerTank => crate::shell::literal(value),
            _ => tt_gameplay::client::value::Val::s(value),
        };
        crate::shell::set_pref_val(&mut self.h.shell.prefs, setting_key(s), v);
    }

    fn bot_settings(&self) -> Vec<BotSlot> {
        bot_settings(&self.h.shell.prefs)
    }

    fn combat_experience(&self) -> f64 {
        let sel = self.h.shell.players.vars.get("players.lastSelection");
        tt_game::fmt::atof(&crate::shell::player_record(self.h, &sel, "cmissions"))
    }

    fn offer_high_score(&mut self, score: f64) {
        crate::shell::offer_high_score(self.h, score);
    }

    fn platform(&self) -> String {
        self.h.platform.platform.clone()
    }

    fn sim_time(&self) -> u32 {
        self.h.sim_time()
    }

    fn schedule(&mut self, delay_ms: u32) -> TimerId {
        let t = self.h.sim_time().wrapping_add(delay_ms);
        TimerId(self.h.sched.post_at(t, crate::sched::Event::ServerTimer))
    }

    fn cancel(&mut self, id: TimerId) {
        self.h.sched.cancel(id.0);
    }

    fn echo(&mut self, text: &str) {
        self.h.log.info(text);
    }

    fn error(&mut self, text: &str) {
        self.h.log.error(text);
    }

    fn send(&mut self, to: Option<ClientId>, msg: &ServerMsg) {
        if self.h.golden.is_some() {
            crate::gameplay::golden::record::on_message(self.h, to, msg);
        }
        if let Some(conn) = to.and_then(|c| self.conn(c)) {
            crate::net::send_to_client(self.h, conn, msg);
        }
    }

    fn hud(&mut self, to: Option<ClientId>, msg: &HudMsg) {
        let remote_or_test = to.is_some_and(|c| self.remote(c).is_some() || self.h.srv.tests.contains(&c));
        if to.is_none() || to.is_some_and(|c| self.local(c)) || !remote_or_test {
            if self.h.golden.is_some() {
                crate::gameplay::golden::record::on_hud(self.h, to, msg);
            }
            crate::gameplay::client::recv::hud(self.h, msg);
        } else {
            self.send(to, &ServerMsg::Hud(msg.clone()));
        }
    }

    fn play2d(&mut self, to: ClientId, sound: Ref<SoundDef>) {
        self.play(to, sound, None);
    }

    fn play3d(&mut self, to: ClientId, sound: Ref<SoundDef>, transform: &str) {
        let p: Vec<f32> = transform.split_whitespace().map(|s| tt_game::fmt::atof(s) as f32).collect();
        self.play(to, sound, (p.len() >= 3).then(|| [p[0], p[1], p[2]]));
    }

    fn address(&self, c: ClientId) -> String {
        match self.conn(c) {
            Some(o) => crate::net::address(self.h, o),
            None if self.h.srv.tests.contains(&c) => "local".into(),
            None => String::new(),
        }
    }

    fn transmit_defs(&mut self, c: ClientId, seq: &str) {
        if let Some(o) = self.conn(c) {
            crate::net::transmit_defs(self.h, o, seq);
        }
    }

    fn start_replication(&mut self, c: ClientId, mission_file: &str) {
        if let Some(o) = self.conn(c) {
            crate::net::start_replication(self.h, o, mission_file);
        }
    }

    fn reset_replication(&mut self, c: ClientId) {
        if let Some(o) = self.conn(c) {
            crate::net::reset_replication(self.h, o);
        }
    }

    fn set_control(&mut self, c: ClientId, tank: EntityId) {
        if let Some(o) = self.conn(c) {
            crate::net::set_control(self.h, o, tank);
        }
    }

    fn delete_connection(&mut self, c: ClientId, reason: &str) {
        if let Some(o) = self.conn(c) {
            if crate::netgame::is_remote(self.h, o) {
                crate::netgame::remove_remote(self.h, o, reason);
            } else {
                crate::net::delete_conn(self.h, o);
            }
            self.h.srv.conns.remove(&o);
        }
    }

    fn ban(&mut self, guid: &str, address: &str, seconds: &str) {
        crate::netgame::admin::add(self.h, guid, address, tt_game::fmt::atoi(seconds) as i64);
        crate::netgame::admin::persist(self.h);
    }

    fn allow_connections(&mut self, on: bool) {
        if on {
            crate::netgame::admin::default_prefs(self.h);
        }
        crate::netgame::allow_connections(self.h, on);
    }

    fn start_heartbeat(&mut self) {
        crate::netgame::set_heartbeat(self.h, true);
    }

    fn stop_heartbeat(&mut self) {
        crate::netgame::set_heartbeat(self.h, false);
    }

    fn set_net_port(&mut self, port: &str) {
        let port = tt_game::fmt::atoi(port).clamp(0, 65535) as u16;
        crate::netgame::set_net_port(self.h, port)
    }

    fn is_file(&self, file: &str) -> bool {
        crate::mission::mission_exists(self.h, file)
    }

    fn maps(&mut self, world: Option<u8>) -> Vec<String> {
        crate::shell::missions(self.h).iter().filter(|m| world.map_or((1..=3).contains(&m.world), |w| m.world == w)).map(|m| tt_assets::layout::mission(&m.id)).collect()
    }

    fn load_info(&mut self, file: &str) -> String {
        crate::mission::mission_info_name(self.h, file)
    }

    fn new_server_group(&mut self) {
        self.h.srv.server_group = true;
    }

    fn delete_server_group(&mut self) -> bool {
        if !self.h.srv.server_group {
            return false;
        }
        self.h.srv.server_group = false;
        // Its members go with it: the mission.
        if self.h.mission_load.is_some() {
            self.delete_mission_group();
        }
        true
    }

    fn server_group_exists(&self) -> bool {
        self.h.srv.server_group
    }

    fn exec_mission(&mut self, file: &str) -> bool {
        crate::mission::exec_mission(self.h, file)
    }

    fn mission_group_exists(&self) -> bool {
        self.h.mission_load.is_some()
    }

    fn delete_mission_group(&mut self) {
        crate::mission::delete_mission(self.h);
    }

    fn music_track(&self) -> String {
        self.h.mission_load.as_ref().and_then(|l| l.tree.as_ref()).and_then(|t| t.object.get("music_track").map(str::to_string)).unwrap_or_default()
    }

    fn group(&self, name: &str) -> Option<Vec<GroupMember>> {
        let load = self.h.mission_load.as_ref()?;
        let node = load.group_named(name)?;
        if !node.object.class.eq_ignore_ascii_case("Group") {
            return None;
        }
        let xf = |o: &tt_assets::mission::SceneObject| fields_transform(o.get("position").unwrap_or(""), o.get("rotation").unwrap_or(""));
        Some(
            node.children
                .iter()
                .map(|c| {
                    if c.object.class.eq_ignore_ascii_case("Group") {
                        GroupMember::Group { name: c.object.name.clone(), members: c.children.iter().map(|m| xf(&m.object)).collect() }
                    } else {
                        GroupMember::Object(xf(&c.object))
                    }
                })
                .collect(),
        )
    }

    fn exec_server_data(&mut self) {
        crate::gameplay::defs::load_server_data(self.h);
    }

    fn delete_defs(&mut self) {
        crate::net::delete_defs(self.h);
        self.h.srv.solo_scales = None;
    }

    fn data(&self) -> &GameData {
        &self.h.data
    }

    fn set_tank_scales(&mut self, scales: Option<SoloScales>) {
        self.h.srv.solo_scales = scales;
    }

    fn new_tank(&mut self, def: Ref<TankDef>, owner: Option<ClientId>) -> Option<EntityId> {
        let def = self.h.data.tanks[def].name.clone();
        let def = def.as_str();
        if world_current(self.h) {
            let g = self.h.game.as_mut()?;
            // `new Tank()`: at the unset fields' transform (0 0 1), no name.
            let e = g.add_tank(def, "", Vec3::new(0.0, 0.0, 1.0), Mat3::IDENTITY, None)?;
            let t = g.tank_of(e)?;
            g.world.set_player_name(t, "");
            if let Some(ent) = g.entities.get_mut(e) {
                ent.owner = owner.map(|c| c.0);
            }
            return Some(e);
        }
        let id = next_entity(self.h);
        let def = def.to_string();
        self.h.srv.pending.push(Pending { id, def, owner, xf: Default::default(), ai: String::new(), team: String::new(), name: String::new() });
        Some(id)
    }

    fn new_flag(&mut self, def: Ref<BallDef>) -> Option<EntityId> {
        let def = self.h.data.balls[def].name.clone();
        self.game()?.add_flag(&def, "", Vec3::ZERO, None)
    }

    fn new_power_up(&mut self, name: &str, def: Ref<PickupDef>) -> Option<EntityId> {
        let db = self.h.data.pickups[def].name.clone();
        let o = tt_assets::mission::SceneObject { class: "PowerUp".into(), name: name.into(), def: false, parent: None, fields: vec![("def".into(), db)], children: Vec::new() };
        self.game()?.add_trigger(&o)
    }

    fn delete_entity(&mut self, e: EntityId) {
        self.h.srv.pending.retain(|p| p.id != e);
        if let Some(g) = self.h.game.as_mut() {
            g.remove_entity(e);
        }
    }

    fn find_entity(&self, name: &str) -> Option<EntityId> {
        self.game_ref()?.entities.named(name)
    }

    fn entity_exists(&self, e: EntityId) -> bool {
        self.h.srv.pending.iter().any(|p| p.id == e) || self.h.game.as_ref().is_some_and(|g| g.entities.get(e).is_some())
    }

    fn set_transform(&mut self, e: EntityId, t: &str) {
        if let Some(p) = self.pending(e) {
            set_transform_fields(&mut p.xf, t);
            return;
        }
        if let Some(g) = self.h.game.as_mut() {
            g.set_transform(e, t);
        }
    }

    fn transform(&self, e: EntityId) -> String {
        if let Some(p) = self.h.srv.pending.iter().find(|p| p.id == e) {
            return fields_transform(&p.xf.0, &p.xf.1);
        }
        self.h.game.as_ref().map(|g| g.script_transform(e)).unwrap_or_default()
    }

    fn set_ai(&mut self, tank: EntityId, ai: Option<Ref<AiDef>>) {
        let ai = ai.map(|a| self.h.data.ai[a].name.clone()).unwrap_or_default();
        if let Some(p) = self.pending(tank) {
            p.ai = ai;
            return;
        }
        let Some(g) = self.h.game.as_mut() else { return };
        let Some(t) = g.tank_of(tank) else { return };
        g.set_ai(t, &ai);
    }

    fn set_team(&mut self, tank: EntityId, color: &str) {
        if let Some(p) = self.pending(tank) {
            p.team = color.to_string();
            return;
        }
        let Some(g) = self.h.game.as_mut() else { return };
        let Some(t) = g.tank_of(tank) else { return };
        if let Some(team) = team_index(color) {
            g.set_team(t, team);
        }
    }

    fn set_player_name(&mut self, tank: EntityId, name: &str) {
        if let Some(p) = self.pending(tank) {
            p.name = name.to_string();
            return;
        }
        let Some(g) = self.h.game.as_mut() else { return };
        if let Some(t) = g.tank_of(tank) {
            g.world.set_player_name(t, name);
        }
    }

    fn set_projectile(&mut self, tank: EntityId, projectile: Ref<ProjectileDef>) {
        let name = self.h.data.projectiles[projectile].name.clone();
        let Some(g) = self.h.game.as_mut() else { return };
        let Some(t) = g.tank_of(tank) else { return };
        if let Some(p) = g.projectile(&name) {
            g.world.set_projectile(t, p);
        }
    }

    fn set_tank_def(&mut self, tank: EntityId, def: Ref<TankDef>) {
        let d = self.h.data.tanks[def].name.clone();
        self.set_def(tank, d);
    }

    fn set_pickup_def(&mut self, pup: EntityId, def: Ref<PickupDef>) {
        let d = self.h.data.pickups[def].name.clone();
        self.set_def(pup, d);
    }

    fn tank_def(&self, e: EntityId) -> Option<Ref<TankDef>> {
        self.h.data.tanks.find(&self.def_of(e))
    }

    fn pickup_def(&self, e: EntityId) -> Option<Ref<PickupDef>> {
        self.h.data.pickups.find(&self.def_of(e))
    }

    fn ball_def(&self, e: EntityId) -> Option<Ref<BallDef>> {
        self.h.data.balls.find(&self.def_of(e))
    }

    fn is_corpse(&self, tank: EntityId) -> bool {
        let Some(g) = self.h.game.as_ref() else { return false };
        g.tank_of(tank).is_some_and(|t| g.world.is_corpse(t))
    }

    fn play_user_thread(&mut self, pup: EntityId, slot: u32, scale: f32, reset: bool) {
        let Some(g) = self.h.game.as_mut() else { return };
        if let Some(t) = g.entities.trigger(pup) {
            g.world.play_user_thread(t, slot, scale, reset);
        }
    }

    fn hurt(&mut self, tank: EntityId, amount: f32) {
        let Some(g) = self.h.game.as_mut() else { return };
        if let Some(t) = g.tank_of(tank) {
            g.world.hurt_me(t, &tt_game::fmt::format_g(amount as f64));
        }
    }

    fn flag_of(&self, tank: EntityId) -> Option<EntityId> {
        let g = self.h.game.as_ref()?;
        let f = g.world.get_flag(g.tank_of(tank)?)?;
        g.entities.of_flag(f).map(|e| e.id)
    }

    fn set_goal(&mut self, flag: EntityId, goal: EntityId) {
        let Some(g) = self.h.game.as_mut() else { return };
        if let Some(f) = g.entities.flag(flag) {
            let t = g.entities.trigger(goal);
            g.world.set_goal(f, t);
        }
    }

    fn clear_mount(&mut self, flag: EntityId) {
        let Some(g) = self.h.game.as_mut() else { return };
        if let Some(f) = g.entities.flag(flag) {
            g.world.clear_mount(f);
        }
    }

    fn next_sp_mission(&mut self, fresh: bool) -> Option<CampaignStep> {
        crate::shell::next_sp_mission(self.h, fresh)
    }

    fn solo_level(&self, slot: u32) -> Option<SoloLevel> {
        crate::shell::level_table(self.h).and_then(|t| t.get(slot)).map(solo_level)
    }

    fn is_demo(&self) -> bool {
        self.h.platform.demo
    }

    fn is_release(&self) -> bool {
        self.h.platform.release
    }

    fn is_world_limited(&self) -> bool {
        self.h.platform.world_limited
    }

    fn groups_changed(&mut self, group: &[ClientId], bots: &[ClientId], teams: &[(TeamId, &'static str)]) {
        self.h.srv.group = group.to_vec();
        self.h.srv.bots = bots.to_vec();
        self.h.srv.teams = teams.to_vec();
    }

    fn game_end_key(&mut self, c: ClientId, synch: &str) {
        self.h.srv.game_end_keys.insert(c, synch.to_string());
    }
}

// ---- connections ---------------------------------------------------------------------

/// A connection is gone: its client goes (the server's own deletions are
/// handled by the server).
pub fn connection_removed(h: &mut EngineHost, conn: Conn) {
    let Some(c) = h.srv.conns.remove(&conn) else { return };
    with_server(h, |s, b| s.remove_client(b, c));
}

/// A launcher's local connection, then what it set after it.
fn launch(h: &mut EngineHost, lc: Option<LocalConnect>, after: impl FnOnce(&mut EngineHost)) {
    if let Some(lc) = lc {
        crate::net::connect_local(h, vec![lc.name, lc.team, lc.tank]);
        after(h);
    }
}

/// The players' tanks do not drive themselves.
fn no_ai_control(h: &mut EngineHost) {
    h.srv.state_mut().setup.ai_control = false;
}

/// The solo campaign's next level on a new server.
pub fn launch_campaign(h: &mut EngineHost) {
    let lc = with_server(h, |s, b| s.launch_campaign(b)).flatten();
    launch(h, lc, no_ai_control);
}

/// Target practice.
pub fn launch_target_practice(h: &mut EngineHost) {
    let lc = with_server(h, |s, b| s.launch_target_practice(b)).flatten();
    launch(h, lc, no_ai_control);
}

/// A listen server on `mission`.
pub fn launch_listen(h: &mut EngineHost, mission: &str, ty: MissionType, teams: bool) {
    let lc = with_server(h, |s, b| s.launch_listen(b, mission, ty, teams)).flatten();
    launch(h, lc, |_| {});
}

/// A quick free-for-all.
pub fn launch_quick(h: &mut EngineHost) {
    let lc = with_server(h, |s, b| s.launch_quick(b)).flatten();
    launch(h, lc, no_ai_control);
}

/// The attract-mode demo.
pub fn launch_demo(h: &mut EngineHost) {
    let lc = with_server(h, |s, b| s.launch_demo(b)).flatten();
    launch(h, lc, |h| h.srv.state_mut().after_demo_connect());
}

/// `simLag(loss, lag)`: half the lag on each end of the local connection
/// (and the sockets of a network game).
pub fn sim_lag(h: &mut EngineHost, loss: &str, lag: &str) {
    let lag = tt_game::fmt::format_g(tt_game::fmt::atof(lag) as f32 as f64 / 2.0);
    let (loss, lag) = (tt_game::fmt::atof(loss), tt_game::fmt::atof(&lag));
    for conn in [h.net.server.filter(|c| h.net.alive.contains(c)), h.net.server_connection()].into_iter().flatten() {
        crate::netgame::sim::set_params(h, conn, loss, lag);
    }
}

/// The client of the local connection.
pub fn local_client(h: &EngineHost) -> Option<ClientId> {
    h.net.server.and_then(|o| h.srv.conn_client(o))
}

/// The local client's record (a copy; None without one).
pub fn local_record(h: &EngineHost) -> Option<ClientRecord> {
    let c = local_client(h)?;
    h.srv.get()?.record(Some(c)).cloned()
}

/// Changes the local client's record.
pub fn with_local_record(h: &mut EngineHost, f: impl FnOnce(&mut ClientRecord)) {
    let Some(c) = local_client(h) else { return };
    if let Some(s) = h.srv.server.as_mut() {
        s.with_record(Some(c), f);
    }
}
/// A command from the client of connection `conn` (the local client's or a
/// remote one's, as it arrives).
pub fn client_cmd_from(h: &mut EngineHost, conn: Conn, cmd: &ClientCmd) {
    if let Some(c) = h.srv.conn_client(conn) {
        client_cmd(h, c, cmd);
    }
}

/// A command from client `c`, now.
pub fn client_cmd(h: &mut EngineHost, c: ClientId, cmd: &ClientCmd) {
    with_server(h, |s, b| s.client_cmd(b, c, cmd));
}

/// A server timer is due (`ttServerTimer`).
pub fn fire_timer(h: &mut EngineHost, id: u32) {
    with_server(h, |s, b| s.fire(b, TimerId(id)));
}

// ---- connection callbacks ------------------------------------------------------------

/// A connect request from `address` ("local" for the local connection):
/// "" accepts it, anything else is the reason for the refusal.
pub fn connect_request(h: &mut EngineHost, address: &str) -> String {
    with_server(h, |s, b| s.connect_request(b, address)).unwrap_or_default()
}

/// Connection `conn` is established with its connect arguments (name,
/// team, tank): a new client.
pub fn connected(h: &mut EngineHost, conn: Conn, args: &[String]) {
    let a = |i: usize| args.get(i).cloned().unwrap_or_default();
    let (name, team, tank) = (a(0), a(1), a(2));
    let kind = if Some(conn) == h.net.server { ClientKind::Local } else { ClientKind::Remote };
    with_server(h, |s, b| {
        let cl = s.add_client(b, kind);
        b.h.srv.conns.insert(conn, cl);
        s.connected(b, cl, &name, &team, &tank);
    });
}

/// Connection `conn` drops.
pub fn dropped(h: &mut EngineHost, conn: Conn) {
    if let Some(cl) = h.srv.conn_client(conn) {
        with_server(h, |s, b| s.dropped(b, cl));
    }
}

/// The client of `conn` has the defs of mission sequence `seq`.
pub fn defs_done(h: &mut EngineHost, conn: Conn, seq: &str) {
    if let Some(cl) = h.srv.conn_client(conn) {
        with_server(h, |s, b| s.defs_done(b, cl, seq));
    }
}

/// The client of `conn` has the mission objects.
pub fn replicas_done(h: &mut EngineHost, conn: Conn) {
    if let Some(cl) = h.srv.conn_client(conn) {
        with_server(h, |s, b| s.objects_received(b, cl));
    }
}

// ---- views for tests and tools ---------------------------------------------------------

/// The client of a connection, if any.
pub fn client_of_conn(h: &EngineHost, conn: Conn) -> Option<ClientId> {
    h.srv.conn_client(conn)
}

/// A client's mission download phase (None before the download).
pub fn client_phase(h: &EngineHost, c: ClientId) -> Option<tt_gameplay::server::DownloadPhase> {
    h.srv.get().and_then(|s| s.client(c)).and_then(|c| c.phase)
}

/// A client's record (a copy; None without one).
pub fn record(h: &EngineHost, c: ClientId) -> Option<ClientRecord> {
    h.srv.get()?.record(Some(c)).cloned()
}

/// Changes a client's record.
pub fn with_record(h: &mut EngineHost, c: ClientId, f: impl FnOnce(&mut ClientRecord)) {
    if let Some(s) = h.srv.server.as_mut() {
        s.with_record(Some(c), f);
    }
}

/// A client's player tank in the world.
pub fn player_tank(h: &EngineHost, c: ClientId) -> Option<tt_game::TankId> {
    let e = h.srv.get()?.client(c)?.player?;
    h.game.as_ref()?.tank_of(e)
}

/// A client's K/D/A.
pub fn kda(h: &EngineHost, c: ClientId) -> tt_gameplay::server::Kda {
    h.srv.get().map(|s| s.kda.get(c)).unwrap_or_default()
}

/// The bot clients, in order.
pub fn bots(h: &EngineHost) -> Vec<ClientId> {
    h.srv.get().map(|s| s.bots()).unwrap_or_default()
}

/// A client's team.
pub fn team_of(h: &EngineHost, c: ClientId) -> Option<TeamColor> {
    h.srv.get().and_then(|s| s.team_of(c))
}

/// The value the world uses for `def.field` (the solo scales applied).
pub fn tank_field(h: &EngineHost, def: &str, field: &str) -> String {
    if let Some(v) = h.srv.solo_scales.as_ref().and_then(|s| s.get(def, field)) {
        return v.to_string();
    }
    h.defs.get(def).and_then(|o| o.get(field)).unwrap_or("").to_string()
}

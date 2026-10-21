//! The game server: rounds, scoring, bots, the solo campaign and target
//! practice, the connections' join / leave / mission download, chat and
//! server messages, power-ups, with OpenTT's additions (K/D/A with
//! kill = 2 / assist = 1, the health-pack minimum).
//!
//! The server owns its state, typed ([`state`]: the game setup, the
//! session, the round, the campaign; [`Client`]s with their
//! [`ClientRecord`]s, [`Team`]s, timers, the game logic's random numbers
//! [`EngineRng`]), and acts on the engine through [`ServerHost`]. The
//! rules draw random numbers and set timers in a fixed order and format
//! the numbers clients see the same way every time, so the golden traces
//! stay identical.
//!
//! By what they do: [`match_flow`] (server, missions, rounds), [`players`]
//! (join / leave, names, teams, the mission download), [`bots`],
//! [`respawn`] (spawn points, player tanks), [`combat`] (kills, scores,
//! K/D/A), [`campaign`] (the solo campaign, target practice), [`pickups`]
//! and [`health_floor`], [`chat`] (messages, chat, prints, sounds),
//! [`commands`] (client commands), [`launch`] (the ways a game starts),
//! [`timers`], [`state`], [`records`].

pub mod bots;
pub mod campaign;
pub mod chat;
pub mod combat;
pub mod commands;
pub mod health_floor;
pub mod host;
pub mod launch;
pub mod match_flow;
pub mod pickups;
pub mod players;
pub mod records;
pub mod respawn;
pub mod state;
pub mod timers;

use std::collections::BTreeMap;

pub use host::{GroupMember, ServerHost, Setting};
pub use combat::{Kda, KdaEvent, Scoreboard};
pub use records::{ClientRecord, TeamRecord};
pub use timers::{Timer, TimerId, Timers};
pub use state::{BotSlot, Campaign, CampaignStep, GameSetup, LevelKind, MissionType, Round, ServerType, Session, SoloLevel, SoloScales};
use tt_game::rng::EngineRng;

use crate::data::{Ref, TankDef};
use crate::entity::EntityId;

/// A client of the server: a connection, a bot, or a test client. The id
/// is what messages carry (`ClientJoin id=…`, player list rows, K/D/A and
/// ping lists); allocated in creation order (with the teams').
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u32);

impl ClientId {
    /// As messages write it.
    pub fn text(self) -> String {
        self.0.to_string()
    }
}

/// A team's id (what messages carry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TeamId(pub u32);

/// The two teams of a team game. A client keeps its team across
/// servers; it is on a team while a team of that colour exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamColor {
    Blue,
    Green,
}

impl TeamColor {
    /// The team's object name (`BlueTeam`).
    pub fn name(self) -> &'static str {
        match self {
            TeamColor::Blue => "BlueTeam",
            TeamColor::Green => "GreenTeam",
        }
    }

    /// By object name (any case).
    pub fn parse(s: &str) -> Option<TeamColor> {
        [TeamColor::Blue, TeamColor::Green].into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

/// What a client is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    /// The local player's connection.
    Local,
    /// A network client.
    Remote,
    /// A connection without a network link outside the client list (the
    /// golden harness's test connections).
    Test,
    /// A client list member that is not a connection (the harness's fake
    /// clients: counted, messaged, no record).
    Fake,
    /// A bot.
    Bot,
}

impl ClientKind {
    /// A connection (it drops when it goes).
    pub fn network(self) -> bool {
        matches!(self, ClientKind::Local | ClientKind::Remote)
    }
}

/// How far a connection is with the mission download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    /// Told to load the mission.
    Started,
    /// Sent the defs.
    Defs,
    /// Has the defs.
    DefsDone,
    /// Receives the mission's objects.
    Replication,
    /// In the game.
    InGame,
}

impl DownloadPhase {
    pub fn name(self) -> &'static str {
        match self {
            DownloadPhase::Started => "0",
            DownloadPhase::Defs => "1",
            DownloadPhase::DefsDone => "1.5",
            DownloadPhase::Replication => "2",
            DownloadPhase::InGame => "3",
        }
    }
}

/// A client's flood protection state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flood {
    /// Messages in the protection period.
    pub count: i32,
    /// Muted until the penalty ends.
    pub muted: bool,
    /// When the mute started (sim ms).
    pub since: i64,
}

/// A client's state: its record and the other fields the rules keep.
#[derive(Debug, Clone)]
pub struct Client {
    pub id: ClientId,
    pub kind: ClientKind,
    /// The gameplay record (traced); fake clients have none.
    pub rec: Option<ClientRecord>,
    /// The tagged display name (None: unset).
    pub name: Option<String>,
    pub team: Option<TeamColor>,
    /// The player's guid ("0" for every connection: the game has no
    /// accounts; "" for others).
    pub guid: &'static str,
    /// Whether the join messages carry the guid.
    pub send_guid: bool,
    pub admin: Option<bool>,
    pub super_admin: Option<bool>,
    /// The tank the player drives.
    pub starting_tank: Option<Ref<TankDef>>,
    /// The client's tank (stays set when the tank is gone).
    pub player: Option<EntityId>,
    /// The tank drives itself.
    pub using_ai: bool,
    pub phase: Option<DownloadPhase>,
    pub flood: Flood,
    /// Bots: their tank.
    pub bot_tank: Option<Ref<TankDef>>,
}

impl Client {
    fn new(id: ClientId, kind: ClientKind) -> Client {
        Client {
            id,
            kind,
            rec: (kind != ClientKind::Fake).then(ClientRecord::default),
            name: None,
            team: None,
            guid: "",
            send_guid: false,
            admin: None,
            super_admin: None,
            starting_tank: None,
            player: None,
            using_ai: false,
            phase: None,
            flood: Flood::default(),
            bot_tank: None,
        }
    }
}

/// A team.
#[derive(Debug, Clone)]
pub struct Team {
    pub id: TeamId,
    pub color: TeamColor,
    pub rec: TeamRecord,
    /// The members, in joining order.
    pub members: Vec<ClientId>,
}

/// The round's objects, in order: tanks, balls and power-ups (the bots'
/// tanks in [`CleanupItem::BotGroup`]'s place).
#[derive(Debug, Clone, PartialEq)]
pub enum CleanupItem {
    BotGroup,
    Entity(EntityId),
}

/// A tank the server created.
#[derive(Debug, Clone, Copy)]
pub struct TankInfo {
    /// Its client (None: a campaign bot or target).
    pub client: Option<ClientId>,
}

/// The game server.
#[derive(Debug)]
pub struct Server {
    pub setup: GameSetup,
    pub session: Session,
    pub round: Round,
    pub campaign: Campaign,
    /// The bots of a multiplayer game.
    pub bot_slots: Vec<BotSlot>,
    /// Every client by id (creation order).
    pub clients: BTreeMap<ClientId, Client>,
    /// The client list, in order (connections and fake clients).
    pub group: Vec<ClientId>,
    /// The bot clients (None: not created).
    pub bot_group: Option<Vec<ClientId>>,
    /// The teams (None: not a team game).
    pub teams: Option<Vec<Team>>,
    /// The pending timers.
    pub timers: Timers,
    /// The game logic's random numbers (the engine keeps it in step with
    /// its own users of the same generator).
    pub rng: EngineRng,
    /// Kills / deaths / assists.
    pub kda: Scoreboard,
    /// The tanks the server created (live or gone, until the round ends).
    pub tanks: BTreeMap<EntityId, TankInfo>,
    /// The round's objects (None: none yet).
    pub cleanup: Option<Vec<CleanupItem>>,
    /// The bots' tanks (None: not created).
    pub bot_tanks: Option<Vec<EntityId>>,
    /// The name of the mission loading.
    pub mission_name: String,
    next_id: u32,
}

/// The first id the server gives a team or client (the wire codes them in
/// two bytes either way).
pub const FIRST_ID: u32 = 2000;

impl Default for Server {
    fn default() -> Server {
        Server::new()
    }
}

impl Server {
    pub fn new() -> Server {
        Server {
            setup: GameSetup::default(),
            session: Session::default(),
            round: Round::default(),
            campaign: Campaign::default(),
            bot_slots: Vec::new(),
            clients: BTreeMap::new(),
            group: Vec::new(),
            bot_group: None,
            teams: None,
            timers: Timers::default(),
            rng: EngineRng::new(),
            kda: Scoreboard::default(),
            tanks: BTreeMap::new(),
            cleanup: None,
            bot_tanks: None,
            mission_name: String::new(),
            next_id: FIRST_ID,
        }
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ---- clients and teams ------------------------------------------------------------

    pub fn client(&self, c: ClientId) -> Option<&Client> {
        self.clients.get(&c)
    }

    /// A client's record.
    pub fn record(&self, c: Option<ClientId>) -> Option<&ClientRecord> {
        c.and_then(|c| self.clients.get(&c)).and_then(|c| c.rec.as_ref())
    }

    /// A client's record, mutably.
    pub fn record_mut(&mut self, c: Option<ClientId>) -> Option<&mut ClientRecord> {
        c.and_then(|c| self.clients.get_mut(&c)).and_then(|c| c.rec.as_mut())
    }

    /// Changes a client's record (no record: nothing).
    pub fn with_record(&mut self, c: Option<ClientId>, f: impl FnOnce(&mut ClientRecord)) {
        if let Some(r) = self.record_mut(c) {
            f(r);
        }
    }

    /// A value of a client's record (the default without one).
    pub fn rec<T: Default>(&self, c: Option<ClientId>, f: impl FnOnce(&ClientRecord) -> T) -> T {
        self.record(c).map(f).unwrap_or_default()
    }

    /// The local player's connection.
    pub fn local(&self) -> Option<ClientId> {
        self.clients.values().find(|c| c.kind == ClientKind::Local).map(|c| c.id)
    }

    /// A client exists.
    pub fn exists(&self, c: Option<ClientId>) -> bool {
        c.is_some_and(|c| self.clients.contains_key(&c))
    }

    /// Adds a client of `kind` (a connection, test or fake client).
    pub fn add_client(&mut self, h: &mut dyn ServerHost, kind: ClientKind) -> ClientId {
        let id = ClientId(self.alloc_id());
        self.clients.insert(id, Client::new(id, kind));
        if matches!(kind, ClientKind::Local | ClientKind::Remote | ClientKind::Fake) {
            self.group.push(id);
        }
        self.sync_groups(h);
        id
    }

    /// Tells the host the groups' and teams' new state.
    pub fn sync_groups(&self, h: &mut dyn ServerHost) {
        let teams: Vec<(TeamId, &'static str)> = self.teams.iter().flatten().map(|t| (t.id, t.color.name())).collect();
        h.groups_changed(&self.group, &self.bots(), &teams);
    }

    /// The client goes: its record, K/D/A, team and group memberships and
    /// the timers on it.
    pub fn remove_client(&mut self, h: &mut dyn ServerHost, c: ClientId) {
        self.clients.remove(&c);
        self.group.retain(|x| *x != c);
        if let Some(g) = self.bot_group.as_mut() {
            g.retain(|x| *x != c);
        }
        if let Some(teams) = self.teams.as_mut() {
            for t in teams {
                t.members.retain(|x| *x != c);
            }
        }
        self.kda.forget(c);
        self.timers.cancel_where(h, |t| t.client() == Some(c));
        self.sync_groups(h);
    }

    /// The existing team a name or id text names, if any.
    pub fn team_named(&self, field: &str) -> Option<TeamColor> {
        if field.is_empty() {
            return None;
        }
        self.teams.as_ref()?.iter().find(|t| t.color.name().eq_ignore_ascii_case(field) || t.id.0.to_string() == field).map(|t| t.color)
    }

    /// The team of that colour, if it exists.
    pub fn team(&self, color: Option<TeamColor>) -> Option<&Team> {
        let color = color?;
        self.teams.as_ref()?.iter().find(|t| t.color == color)
    }

    fn team_mut(&mut self, color: Option<TeamColor>) -> Option<&mut Team> {
        let color = color?;
        self.teams.as_mut()?.iter_mut().find(|t| t.color == color)
    }

    /// The id of the client's team, if it exists.
    pub fn team_id_of(&self, c: Option<ClientId>) -> Option<TeamId> {
        self.team(c.and_then(|c| self.team_of(c))).map(|t| t.id)
    }

    /// The team's short name ("" without a team).
    pub fn team_short(&self, color: Option<TeamColor>) -> String {
        self.team(color).map(|t| t.rec.short_name.clone()).unwrap_or_default()
    }

    /// The client's team field.
    pub fn team_of(&self, c: ClientId) -> Option<TeamColor> {
        self.clients.get(&c).and_then(|c| c.team)
    }

    /// The client's team field as messages name it ("" without one).
    pub fn team_name_of(&self, c: ClientId) -> String {
        self.team_of(c).map(|t| t.name().to_string()).unwrap_or_default()
    }

    fn team_add(&mut self, team: Option<TeamColor>, c: ClientId) {
        if let Some(t) = self.team_mut(team) {
            if !t.members.contains(&c) {
                t.members.push(c);
            }
        }
    }

    fn team_remove(&mut self, team: Option<TeamColor>, c: ClientId) {
        if let Some(t) = self.team_mut(team) {
            t.members.retain(|x| *x != c);
        }
    }

    /// Bot clients, in order.
    pub fn bots(&self) -> Vec<ClientId> {
        self.bot_group.clone().unwrap_or_default()
    }

    /// The client's tagged display name.
    pub fn name_text(&self, c: ClientId) -> String {
        self.clients.get(&c).and_then(|c| c.name.clone()).unwrap_or_default()
    }

    /// The client's name as a message value (a tag; "" when unset).
    pub fn name_value(&self, c: Option<ClientId>) -> crate::msg::Value {
        match c.and_then(|c| self.clients.get(&c)).and_then(|c| c.name.clone()) {
            Some(t) => crate::msg::Value::Tag(t),
            None => crate::msg::Value::empty(),
        }
    }

    // ---- timers -----------------------------------------------------------------------

    /// A timer fired.
    pub fn fire(&mut self, h: &mut dyn ServerHost, id: TimerId) {
        let Some(t) = self.timers.take(id) else { return };
        match t {
            Timer::RoundWarning => self.round_warning(h),
            Timer::RoundEnd => self.round_time_up(h),
            Timer::EndRound => self.end_round_and_pause(h),
            Timer::NextRound => self.next_round(h),
            Timer::SoloTick => self.solo_tick(h),
            Timer::SoloLevelEnd => self.end_round(h),
            Timer::TargetPracticeDone => self.target_practice_done(h),
            Timer::BalanceBots => self.balance_bots(h),
            Timer::RefreshPlayerList => self.refresh_player_list(h),
            Timer::RestartMission => self.restart_mission(h),
            Timer::StartHeartbeat => h.start_heartbeat(),
            Timer::LoadMissionObjects => self.load_mission_objects(h),
            Timer::Respawn(c) => {
                if self.exists(Some(c)) {
                    self.spawn_player(h, c);
                }
            }
            Timer::PlaySound(c, sound) => {
                if self.client(c).is_some_and(|c| c.kind != ClientKind::Bot && c.kind != ClientKind::Fake) {
                    h.play2d(c, sound);
                }
            }
            Timer::SwapPickup(e, def) => {
                if h.entity_exists(e) {
                    h.set_pickup_def(e, def);
                }
            }
            Timer::Unmute(c) => self.unmute(c),
            Timer::ForgetMessage(c) => self.forget_message(c),
        }
    }

    /// Cancels the power-up timers of these entities (the objects went).
    fn cancel_entity_timers(&mut self, h: &mut dyn ServerHost, gone: impl Fn(EntityId) -> bool) {
        self.timers.cancel_where(h, |t| t.entity().is_some_and(&gone));
    }

    // ---- the round's objects ----------------------------------------------------------

    /// An entity created for the round.
    fn cleanup_add(&mut self, e: EntityId) {
        if let Some(c) = self.cleanup.as_mut() {
            c.push(CleanupItem::Entity(e));
        }
    }

    /// The round's objects go (the bots' tanks with them), last first, the
    /// timers on them cancelled.
    fn delete_cleanup(&mut self, h: &mut dyn ServerHost) {
        let Some(items) = self.cleanup.take() else { return };
        let mut gone = Vec::new();
        for it in items.into_iter().rev() {
            match it {
                CleanupItem::BotGroup => {
                    gone.extend(self.bot_tanks.take().unwrap_or_default().into_iter().rev());
                }
                CleanupItem::Entity(e) => gone.push(e),
            }
        }
        for e in &gone {
            if h.entity_exists(*e) {
                h.delete_entity(*e);
            }
            self.tanks.remove(e);
        }
        self.cancel_entity_timers(h, |e| gone.contains(&e));
    }

    /// A tank goes (corpse removed, player left, bot dropped).
    pub fn delete_tank(&mut self, h: &mut dyn ServerHost, e: EntityId) {
        if h.entity_exists(e) {
            h.delete_entity(e);
        }
        self.tanks.remove(&e);
        if let Some(c) = self.cleanup.as_mut() {
            c.retain(|x| *x != CleanupItem::Entity(e));
        }
        if let Some(b) = self.bot_tanks.as_mut() {
            b.retain(|x| *x != e);
        }
    }

    /// A tank the server made that still exists.
    pub fn tank_exists(&self, h: &dyn ServerHost, e: Option<EntityId>) -> bool {
        e.is_some_and(|e| self.tanks.contains_key(&e) && h.entity_exists(e))
    }

    /// The client of a tank.
    pub fn tank_client(&self, e: EntityId) -> Option<ClientId> {
        self.tanks.get(&e).and_then(|t| t.client).filter(|c| self.clients.contains_key(c))
    }

    /// The round's tanks in order (the bots' in their place), for the
    /// test helpers.
    pub fn cleanup_tanks(&self) -> Vec<EntityId> {
        let mut out = Vec::new();
        for it in self.cleanup.iter().flatten() {
            match it {
                CleanupItem::BotGroup => out.extend(self.bot_tanks.iter().flatten().copied()),
                CleanupItem::Entity(e) if self.tanks.contains_key(e) => out.push(*e),
                CleanupItem::Entity(_) => {}
            }
        }
        out
    }

    /// The round's entities in order (balls, power-ups included; the bots'
    /// tanks left out).
    pub fn cleanup_entities(&self) -> Vec<EntityId> {
        self.cleanup.iter().flatten().filter_map(|it| if let CleanupItem::Entity(e) = it { Some(*e) } else { None }).collect()
    }

    /// The client numbering of the golden traces: the client list, then
    /// the bots, in order.
    pub fn group_order(&self) -> Vec<ClientId> {
        let mut v = self.group.clone();
        v.extend(self.bots());
        v
    }

    /// A client's record fields, for traces.
    pub fn record_fields(&self, c: ClientId) -> Vec<(&'static str, String)> {
        self.record(Some(c)).map(ClientRecord::trace_fields).unwrap_or_default()
    }

    /// Clients with a record, by id.
    pub fn recorded(&self) -> Vec<ClientId> {
        self.clients.values().filter(|c| c.rec.is_some()).map(|c| c.id).collect()
    }

    /// The kind of a client.
    pub fn kind(&self, c: ClientId) -> Option<ClientKind> {
        self.clients.get(&c).map(|c| c.kind)
    }

    /// Every client's score (clients, then bots; the server info's player
    /// list).
    pub fn scores(&self) -> Vec<Option<f64>> {
        self.group_order().iter().map(|c| self.record(Some(*c)).and_then(|r| r.score)).collect()
    }

    /// The round summary's (name, value) pairs.
    pub fn round_fields(&self) -> Vec<(&'static str, String)> {
        use state::flag_text;
        vec![
            ("game.running", flag_text(self.round.running).into()),
            ("game.cycling", flag_text(self.round.cycling).into()),
            ("session.mission_file", self.session.mission_file.clone()),
            ("game.mission_type", self.setup.mission_type.name().into()),
            ("game.team_game", flag_text(Some(self.setup.team_game)).into()),
            ("game.single_player", flag_text(self.setup.single_player).into()),
            ("game.target_range", flag_text(self.setup.target_range).into()),
            ("game.mission_seq", self.campaign.mission_seq.map(|s| s.to_string()).unwrap_or_default()),
            ("game.mission_seq_total", self.campaign.mission_seq_total.map(|s| s.to_string()).unwrap_or_default()),
            ("game.level_type", self.campaign.kind().map(|k| k.name().to_string()).unwrap_or_default()),
            ("game.demo_mode", flag_text(self.setup.demo_mode).into()),
        ]
    }

    // ---- test helpers (the golden harness's clients and cheats) -------------------------

    /// A fake client: a client list member that is no connection (no
    /// record), on `team`.
    pub fn add_fake_client(&mut self, h: &mut dyn ServerHost, team: &str) -> ClientId {
        let c = self.add_client(h, ClientKind::Fake);
        let t = TeamColor::parse(team);
        self.clients.get_mut(&c).unwrap().team = t;
        self.team_add(t, c);
        self.sync_groups(h);
        c
    }

    /// A power-up of kind `def` created for the round.
    pub fn test_power_up(&mut self, h: &mut dyn ServerHost, def: crate::data::Ref<crate::data::PickupDef>) -> Option<EntityId> {
        let e = h.new_power_up("", def)?;
        self.cleanup_add(e);
        Some(e)
    }

    /// `(clients, bots)`.
    pub fn counts(&self) -> (usize, usize) {
        (self.group.len(), self.bot_group.as_ref().map_or(0, Vec::len))
    }
}

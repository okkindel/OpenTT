//! What the server needs from the engine ([`ServerHost`]): settings, time
//! and timers, connections and messages, the mission's objects, the data
//! and the world's entities. The rules ([`super::Server`]) never call back
//! into themselves through it: every method is an effect or a query.

use super::state::{BotSlot, CampaignStep, SoloLevel, SoloScales};
use super::ClientId;
use crate::data::{AiDef, BallDef, GameData, PickupDef, ProjectileDef, Ref, SoundDef, TankDef};
use crate::entity::EntityId;
use crate::msg::{HudMsg, ServerMsg};

/// A member of a mission group, as the spawn point pick sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupMember {
    /// A sub-group with its name and members' transforms.
    Group { name: String, members: Vec<String> },
    /// Any other object, with its transform (`x y z ax ay az angle`).
    Object(String),
}

/// A server setting of the settings store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    MaxPlayers,
    /// Bots leave and come back as players join and leave.
    ReplaceBots,
    Port,
    /// `Never`: not listed on the master server.
    DisplayOnMaster,
    AdminPassword,
    MaxChatLen,
    FloodProtection,
    /// Seconds a kicked or banned player stays out.
    KickBanTime,
    BanTime,
    /// The connection error a joining client is told about.
    ConnectionError,
    /// The local player.
    PlayerName,
    PlayerTeam,
    PlayerTank,
}

/// The engine side of the server.
pub trait ServerHost {
    // ---- settings and the player's profile -----------------------------------------

    /// A setting as text ("" when unset).
    fn setting(&self, s: Setting) -> String;
    fn set_setting(&mut self, s: Setting, value: &str);
    /// The bots the settings list for a multiplayer game.
    fn bot_settings(&self) -> Vec<BotSlot>;
    /// The current player's multiplayer experience (missions played).
    fn combat_experience(&self) -> f64;
    /// A finished solo run's score: the player's high score if
    /// better, saved.
    fn offer_high_score(&mut self, score: f64);
    /// `macos`, `windows`, `x86Unix`.
    fn platform(&self) -> String;

    // ---- time and timers ----------------------------------------------------------

    /// Simulation time (ms).
    fn sim_time(&self) -> u32;
    /// Queues a server timer `delay_ms` from now, after every event due
    /// by then; the id the engine fires it with ([`super::Server::fire`]).
    fn schedule(&mut self, delay_ms: u32) -> super::TimerId;
    /// Takes a timer off the queue (nothing if it is not there).
    fn cancel(&mut self, id: super::TimerId);

    // ---- log --------------------------------------------------------------------------

    fn echo(&mut self, text: &str);
    fn error(&mut self, text: &str);

    // ---- clients ------------------------------------------------------------------

    /// One message to one client's connection (test and fake clients:
    /// recorded only; None: to nobody, recorded as such).
    fn send(&mut self, to: Option<ClientId>, msg: &ServerMsg);
    /// A client-side effect: applied by this process's client for the
    /// local client, a client without a network connection or none at all
    /// (`to` = None), else sent as [`ServerMsg::Hud`].
    fn hud(&mut self, to: Option<ClientId>, msg: &HudMsg);
    /// A sound for one client: everywhere, or at a transform.
    fn play2d(&mut self, to: ClientId, sound: Ref<SoundDef>);
    fn play3d(&mut self, to: ClientId, sound: Ref<SoundDef>, transform: &str);
    /// A client's network address ("local" for the local connection).
    fn address(&self, c: ClientId) -> String;
    /// The mission download steps of a connection.
    fn transmit_defs(&mut self, c: ClientId, seq: &str);
    fn start_replication(&mut self, c: ClientId, mission_file: &str);
    fn reset_replication(&mut self, c: ClientId);
    /// The client controls this tank.
    fn set_control(&mut self, c: ClientId, tank: EntityId);
    /// Deletes a client's connection; the server has handled its drop
    /// already.
    fn delete_connection(&mut self, c: ClientId, reason: &str);
    /// Bans a client's address and guid for `seconds`.
    fn ban(&mut self, guid: &str, address: &str, seconds: &str);
    fn allow_connections(&mut self, on: bool);
    fn start_heartbeat(&mut self);
    fn stop_heartbeat(&mut self);
    /// The server's port (opened with `allow_connections`).
    fn set_net_port(&mut self, port: &str);

    // ---- missions -------------------------------------------------------------------

    fn is_file(&self, file: &str) -> bool;
    /// The map files of the map list (the game's and the enabled mods'), in
    /// its order: of one world (1 green, 2 blue, 3 red, 4 the Moon), or of the
    /// green, blue and red worlds (None). The special maps (no world) are in neither.
    fn maps(&mut self, world: Option<u8>) -> Vec<String>;
    /// The mission's name ("" if none).
    fn load_info(&mut self, file: &str) -> String;
    /// The server's object group (the mission and its round objects).
    fn new_server_group(&mut self);
    /// Deletes it (the mission with it); false if none.
    fn delete_server_group(&mut self) -> bool;
    fn server_group_exists(&self) -> bool;
    /// Loads the mission's objects.
    fn exec_mission(&mut self, file: &str) -> bool;
    fn mission_group_exists(&self) -> bool;
    fn delete_mission_group(&mut self);
    /// The mission's music.
    fn music_track(&self) -> String;
    /// A mission group by name: its members, None if none.
    fn group(&self, name: &str) -> Option<Vec<GroupMember>>;
    /// Loads the server's data ([`ServerHost::data`]).
    fn exec_server_data(&mut self);
    /// Unloads it.
    fn delete_defs(&mut self);

    // ---- data ---------------------------------------------------------------------

    /// The game's typed data (what the server's data and the mission declared).
    fn data(&self) -> &GameData;
    /// The campaign tanks' solo strengths (None: their own).
    fn set_tank_scales(&mut self, scales: Option<SoloScales>);

    // ---- entities -------------------------------------------------------------------

    /// A new tank for a client.
    fn new_tank(&mut self, def: Ref<TankDef>, owner: Option<ClientId>) -> Option<EntityId>;
    /// A new ball.
    fn new_flag(&mut self, def: Ref<BallDef>) -> Option<EntityId>;
    /// A new power-up (with an object name, "" for none).
    fn new_power_up(&mut self, name: &str, def: Ref<PickupDef>) -> Option<EntityId>;
    fn delete_entity(&mut self, e: EntityId);
    /// An entity by its object name.
    fn find_entity(&self, name: &str) -> Option<EntityId>;
    fn entity_exists(&self, e: EntityId) -> bool;
    /// Moves an entity (a transform text) / its transform.
    fn set_transform(&mut self, e: EntityId, t: &str);
    fn transform(&self, e: EntityId) -> String;
    /// A bot driver for the tank (None: the player drives).
    fn set_ai(&mut self, tank: EntityId, ai: Option<Ref<AiDef>>);
    fn set_team(&mut self, tank: EntityId, color: &str);
    fn set_player_name(&mut self, tank: EntityId, name: &str);
    /// The tank's cartridge.
    fn set_projectile(&mut self, tank: EntityId, projectile: Ref<ProjectileDef>);
    /// A tank becomes another kind of tank.
    fn set_tank_def(&mut self, tank: EntityId, def: Ref<TankDef>);
    /// A power-up becomes another kind of power-up.
    fn set_pickup_def(&mut self, pup: EntityId, def: Ref<PickupDef>);
    /// What an entity is.
    fn tank_def(&self, e: EntityId) -> Option<Ref<TankDef>>;
    fn pickup_def(&self, e: EntityId) -> Option<Ref<PickupDef>>;
    fn ball_def(&self, e: EntityId) -> Option<Ref<BallDef>>;
    fn is_corpse(&self, tank: EntityId) -> bool;
    /// Plays the power-up's user animation `slot` once (cosmetic, replicated).
    fn play_user_thread(&mut self, pup: EntityId, slot: u32, scale: f32, reset: bool);
    /// Damages a tank (negative: repairs).
    fn hurt(&mut self, tank: EntityId, amount: f32);
    /// The ball a tank carries.
    fn flag_of(&self, tank: EntityId) -> Option<EntityId>;
    fn set_goal(&mut self, flag: EntityId, goal: EntityId);
    fn clear_mount(&mut self, flag: EntityId);

    // ---- the solo campaign -------------------------------------------------------------

    /// The run's next level (`fresh`: a new run starts unless one is
    /// waiting to begin); None if there is none.
    fn next_sp_mission(&mut self, fresh: bool) -> Option<CampaignStep>;
    /// A level of the campaign's table (from 1).
    fn solo_level(&self, slot: u32) -> Option<SoloLevel>;
    /// Demo and limited builds.
    fn is_demo(&self) -> bool;
    fn is_release(&self) -> bool;
    fn is_world_limited(&self) -> bool;

    // ---- views of the server state for the engine --------------------------------

    /// The client list, the bots and the teams changed (the engine keeps a
    /// copy for what it records while the server runs).
    fn groups_changed(&mut self, group: &[ClientId], bots: &[ClientId], teams: &[(super::TeamId, &'static str)]);
    /// A round end report to `c` follows; its stats are keyed with `synch`.
    fn game_end_key(&mut self, c: ClientId, synch: &str);
}

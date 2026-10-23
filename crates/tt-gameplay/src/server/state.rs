//! The server's game state, typed: how the next game is set up
//! ([`GameSetup`]), the running server ([`Session`]), the round
//! ([`Round`]), the solo campaign ([`Campaign`]) and the bot slots
//! ([`BotSlot`]).
//!
//! A few values are tri-state (`Option<bool>`): "never set" is part of the
//! state the round summaries report (empty), distinct from "off".

use tt_game::fmt::format_g;

/// The kind of round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissionType {
    /// Battle: tanks score by destroying each other.
    #[default]
    Deathmatch,
    /// The ball game: tanks score by carrying the ball into the goal.
    Scrum,
}

impl MissionType {
    pub fn name(self) -> &'static str {
        match self {
            MissionType::Deathmatch => "Deathmatch",
            MissionType::Scrum => "Scrum",
        }
    }

    /// By name (any case); anything else is a battle.
    pub fn parse(s: &str) -> MissionType {
        if s.trim().eq_ignore_ascii_case("Scrum") {
            MissionType::Scrum
        } else {
            MissionType::Deathmatch
        }
    }
}

/// What a dedicated server plays (its command line's game argument).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DedicatedMode {
    #[default]
    Battle,
    TeamBattle,
    Scrum,
    TeamScrum,
}

impl DedicatedMode {
    /// `TB`, `S`, `TS` (any case); anything else is a free-for-all battle.
    pub fn parse(arg: &str) -> DedicatedMode {
        match arg.to_ascii_uppercase().as_str() {
            "TB" => DedicatedMode::TeamBattle,
            "S" => DedicatedMode::Scrum,
            "TS" => DedicatedMode::TeamScrum,
            _ => DedicatedMode::Battle,
        }
    }

    pub fn mission_type(self) -> MissionType {
        match self {
            DedicatedMode::Battle | DedicatedMode::TeamBattle => MissionType::Deathmatch,
            DedicatedMode::Scrum | DedicatedMode::TeamScrum => MissionType::Scrum,
        }
    }

    pub fn teams(self) -> bool {
        matches!(self, DedicatedMode::TeamBattle | DedicatedMode::TeamScrum)
    }
}

/// A single-player (local) or multiplayer server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerType {
    SinglePlayer,
    MultiPlayer,
}

/// A solo campaign level's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LevelKind {
    /// Destroy the bots.
    #[default]
    Battle,
    /// Destroy the boss tank.
    Boss,
    /// Collect every brain against the clock.
    Brain,
}

impl LevelKind {
    pub fn name(self) -> &'static str {
        match self {
            LevelKind::Battle => "Battle",
            LevelKind::Boss => "Boss",
            LevelKind::Brain => "Brain",
        }
    }

    /// By name (any case).
    pub fn parse(s: &str) -> Option<LevelKind> {
        [LevelKind::Battle, LevelKind::Boss, LevelKind::Brain].into_iter().find(|k| k.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// How the next game is set up: what the menus, the launchers and the
/// command line choose. It outlives the servers.
#[derive(Debug, Clone, PartialEq)]
pub struct GameSetup {
    pub mission_type: MissionType,
    pub team_game: bool,
    /// The solo campaign.
    pub single_player: Option<bool>,
    /// Target practice.
    pub target_range: Option<bool>,
    /// The attract-mode demo.
    pub demo_mode: Option<bool>,
    /// A round's length (s; 0: no limit).
    pub round_secs: f32,
    /// A demo round's length (s).
    pub demo_round_secs: f32,
    /// The pause between two rounds (s).
    pub end_pause_secs: u32,
    /// The demo player's tank.
    pub demo_tank: String,
    /// The players' tanks drive themselves after a death (the demo).
    pub ai_control: bool,
    /// The mission files a multiplayer server plays in turn after a round
    /// (wrapping); empty: it plays its mission again.
    pub rotation: Vec<String>,
    /// The next map is a random other one of the rotation, not the one
    /// after the current one.
    pub rotation_random: bool,
    /// How long a player waits after joining, and between two map votes
    /// he calls (ms).
    pub vote_wait_ms: u32,
    /// A dedicated server (no local player).
    pub dedicated: bool,
    /// What a dedicated server plays.
    pub dedicated_mode: DedicatedMode,
    /// The server opens no network port (test runs).
    pub skip_port: bool,
    /// Remote clients may use the test commands.
    pub test_cheats: bool,
    /// A multiplayer load waits this long between its two stages (ms).
    pub load_pause_ms: u32,
}

impl Default for GameSetup {
    fn default() -> GameSetup {
        GameSetup {
            mission_type: MissionType::Deathmatch,
            team_game: true,
            single_player: None,
            target_range: None,
            demo_mode: None,
            round_secs: 15.0 * 60.0,
            demo_round_secs: 5.0 * 60.0,
            end_pause_secs: 3,
            demo_tank: "GoldMediumTank".into(),
            ai_control: false,
            rotation: Vec::new(),
            rotation_random: false,
            vote_wait_ms: 180_000,
            dedicated: false,
            dedicated_mode: DedicatedMode::Battle,
            skip_port: false,
            test_cheats: true,
            load_pause_ms: 5000,
        }
    }
}

impl GameSetup {
    pub fn single_player(&self) -> bool {
        self.single_player == Some(true)
    }

    pub fn target_range(&self) -> bool {
        self.target_range == Some(true)
    }

    pub fn demo(&self) -> bool {
        self.demo_mode == Some(true)
    }
}

/// The running server: what it announces, its players and its mission.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    pub server_type: Option<ServerType>,
    /// The game mode as the server list shows it (`Team Battlemode`).
    pub mode_label: String,
    /// The server list's info line: platform, world, mission name.
    pub info: String,
    /// The server list's player list: team letter and name of every
    /// player, tab separated.
    pub player_list: String,
    /// Players and bots on the server.
    pub player_count: i32,
    /// The mission (asset path, `missions/tt1_1.ron`).
    pub mission_file: String,
    /// The mission's number on this server (from 1); clients answer with it.
    pub mission_seq: u32,
    pub mission_running: bool,
    /// The mission a passed map vote chose: the next round plays it.
    pub voted_mission: Option<String>,
}

impl Session {
    pub fn multiplayer(&self) -> bool {
        self.server_type == Some(ServerType::MultiPlayer)
    }
}

/// The round in progress.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Round {
    pub running: Option<bool>,
    /// Between two rounds (the end pause).
    pub cycling: Option<bool>,
    /// When the round started (sim ms).
    pub start_ms: u32,
    /// A player has entered the game since the mission loaded.
    pub player_in_game: bool,
    /// The round's next step: the warning, the end, the pause.
    pub end_timer: Option<super::TimerId>,
    /// The solo spawner's next step.
    pub solo_timer: Option<super::TimerId>,
}

impl Round {
    pub fn running(&self) -> bool {
        self.running == Some(true)
    }

    pub fn cycling(&self) -> bool {
        self.cycling == Some(true)
    }
}

/// A solo campaign level's numbers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoloLevel {
    /// The mission (id, `tt1_1`).
    pub mission: String,
    pub kind: LevelKind,
    /// The bots' strength (`damage`, `armor`, `fire`).
    pub damage_scale: f32,
    pub armor_scale: f32,
    pub fire_scale: f32,
    pub start_bots: f32,
    pub max_live_bots: f32,
    pub max_spawn_bots: f32,
    /// Destroyed bots that end the level (0: none).
    pub end_bots: f32,
    /// The spawner's pause between bots, in 5 s ticks.
    pub min_bot_ticks: f32,
    pub max_bot_ticks: f32,
    /// The level's length in 5 s ticks (0: none).
    pub max_mission_ticks: f32,
    /// Kills this close together (ms) earn the quick-shot bonus.
    pub quick_shot_time: f32,
    /// The bots' colour mix (percent).
    pub bronze: f64,
    pub silver: f64,
    pub gold: f64,
    /// The bots' size mix (percent).
    pub light: f64,
    pub medium: f64,
    pub heavy: f64,
    /// Points per kill and brain.
    pub bronze_points: f32,
    pub silver_points: f32,
    pub gold_points: f32,
    pub boss_points: f32,
    pub brain_points: f32,
}

/// The level a solo run plays next.
#[derive(Debug, Clone, PartialEq)]
pub struct CampaignStep {
    /// The mission file.
    pub mission_file: String,
    /// The level's slot and the run's position.
    pub seq: u32,
    pub total: u32,
    /// Its numbers (the run's, after the bumps of earlier levels).
    pub level: SoloLevel,
}

/// The campaign tanks and their solo strengths.
pub const SOLO_TANKS: [&str; 9] = ["BronzeLightTank", "SilverLightTank", "GoldLightTank", "BronzeMediumTank", "SilverMediumTank", "GoldMediumTank", "BronzeHeavyTank", "SilverHeavyTank", "GoldHeavyTank"];

/// The campaign tanks' strengths for a level: each tank's damage, armour
/// and fire scale (as the tanks' data text) times the level's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoloScales {
    /// (tank, damage, armor, fire) as data text.
    pub tanks: Vec<(&'static str, [String; 3])>,
}

/// The tank data fields the solo scales replace, in [`SoloScales`] order.
pub const SCALE_FIELDS: [&str; 3] = ["damage_scale", "armor_scale", "fire_scale"];

impl SoloScales {
    /// The level's scales over the tanks' own (`base`, per tank of
    /// [`SOLO_TANKS`]): damage times the level's, armour and fire divided
    /// by it.
    pub fn compute(base: &[[f32; 3]; 9], damage: f32, armor: f32, fire: f32) -> SoloScales {
        let scale = [damage as f64, (1.0 / armor as f64) as f32 as f64, (1.0 / fire as f64) as f32 as f64];
        let tanks = SOLO_TANKS.iter().zip(base).map(|(t, b)| (*t, std::array::from_fn(|i| format_g(b[i] as f64 * scale[i])))).collect();
        SoloScales { tanks }
    }

    /// The value of `tank.field`, if scaled.
    pub fn get(&self, tank: &str, field: &str) -> Option<&str> {
        let i = SCALE_FIELDS.iter().position(|f| f.eq_ignore_ascii_case(field))?;
        self.tanks.iter().find(|(t, _)| t.eq_ignore_ascii_case(tank)).map(|(_, v)| v[i].as_str())
    }
}

/// The solo campaign's state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Campaign {
    /// The level table was loaded for this run of the game.
    pub levels_loaded: bool,
    /// The level's slot and the run's position.
    pub mission_seq: Option<u32>,
    pub mission_seq_total: Option<u32>,
    /// The level being played (its numbers stay after it).
    pub level: Option<SoloLevel>,
    /// Bots alive, spawned and destroyed this level.
    pub live_bots: f32,
    pub spawned_bots: f32,
    pub dead_bots: f32,
    /// 5 s ticks until the next bot.
    pub next_bot_tick: f32,
    /// The campaign tanks' own scales (damage, armor, fire), per tank of
    /// [`SOLO_TANKS`].
    pub base_scales: [[f32; 3]; 9],
}

impl Campaign {
    pub fn level(&self) -> SoloLevel {
        self.level.clone().unwrap_or_default()
    }

    pub fn kind(&self) -> Option<LevelKind> {
        self.level.as_ref().map(|l| l.kind)
    }
}

/// A bot of a multiplayer server: its tank, name and team (empty: none).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BotSlot {
    pub tank: String,
    pub name: String,
    pub team: String,
}

impl BotSlot {
    pub fn new(tank: &str, name: &str, team: &str) -> BotSlot {
        BotSlot { tank: tank.into(), name: name.into(), team: team.into() }
    }
}

/// The bots a game can be set up with: (tank, name). A default game has
/// the first six; the menus' bot tables offer them all.
pub const DEFAULT_BOTS: [(&str, &str); 10] = [
    ("GoldHeavyTank", "Hum [Bot]"),
    ("SilverMediumTank", "Tom [Bot]"),
    ("BronzeLightTank", "Mal [Bot]"),
    ("GoldLightTank", "Ri [Bot]"),
    ("SilverMediumTank", "Gul [Bot]"),
    ("BronzeHeavyTank", "Gum [Bot]"),
    ("GoldMediumTank", "Zed [Bot]"),
    ("SilverLightTank", "Pip [Bot]"),
    ("BronzeMediumTank", "Bix [Bot]"),
    ("SilverHeavyTank", "Nub [Bot]"),
];

/// The most bots a game can be set up with.
pub const MAX_BOTS: usize = DEFAULT_BOTS.len();

/// The flood protection's numbers.
pub mod flood {
    /// A message is forgotten after this long (ms).
    pub const PERIOD_MS: u32 = 10_000;
    /// This many remembered messages mute the sender…
    pub const THRESHOLD: i32 = 4;
    /// …for this long (ms).
    pub const PENALTY_MS: u32 = 10_000;
}

/// A delay in seconds as milliseconds, rounded to the nearest (0 for a
/// negative or invalid delay).
pub fn delay_ms(secs: f64) -> u32 {
    let ms = (secs * 1000.0).round();
    if ms.is_nan() || ms <= 0.0 {
        0
    } else {
        ms.min(u32::MAX as f64) as u32
    }
}

/// Seconds as clocks and messages give them: plain decimal, at most
/// millisecond precision, no trailing zeros ("899.936", "900", "-0.5").
pub fn secs_text(secs: f64) -> String {
    if !secs.is_finite() {
        return "0".into();
    }
    let t = format!("{:.3}", secs);
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t == "-0" { "0".into() } else { t.to_string() }
}

/// A tri-state flag as the round summaries show it ("", "0", "1").
pub fn flag_text(f: Option<bool>) -> &'static str {
    match f {
        None => "",
        Some(false) => "0",
        Some(true) => "1",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_and_delays() {
        assert_eq!(delay_ms(35.0), 35_000);
        assert_eq!(delay_ms(865.0), 865_000);
        assert_eq!(delay_ms(1234.5678), 1_234_568);
        assert_eq!(delay_ms(0.0004), 0);
        assert_eq!(delay_ms(-1.0), 0);
        assert_eq!(delay_ms(f64::NAN), 0);
        assert_eq!(secs_text(900.0), "900");
        assert_eq!(secs_text(899.936), "899.936");
        assert_eq!(secs_text(900.0 - 1_234.567), "-334.567");
        assert_eq!(secs_text(2_000.5), "2000.5");
        assert_eq!(secs_text(123_456.789), "123456.789");
        assert_eq!(secs_text(-0.0001), "0");
        // Solo scores: plain, never %g ("1.23457e+06").
        assert_eq!(crate::server::records::score_text(1_234_567.0), "1234567");
        assert_eq!(crate::server::records::score_text(0.0), "0");
        assert_eq!(crate::server::records::score_text(12.5), "12.5");
    }

    #[test]
    fn solo_scales() {
        let base = [[1.0, 1.0, 1.0]; 9];
        let s = SoloScales::compute(&base, 0.4, 0.2, 0.5);
        assert_eq!(s.get("goldheavytank", "DAMAGE_SCALE"), Some("0.4"));
        assert_eq!(s.get("BronzeLightTank", "armor_scale"), Some("5"));
        assert_eq!(s.get("BronzeLightTank", "fire_scale"), Some("2"));
        assert_eq!(s.get("LightTank", "fire_scale"), None);
    }

    #[test]
    fn names() {
        assert_eq!(MissionType::parse("scrum"), MissionType::Scrum);
        assert_eq!(DedicatedMode::parse("ts"), DedicatedMode::TeamScrum);
        assert_eq!(LevelKind::parse("BRAIN"), Some(LevelKind::Brain));
        assert_eq!(flag_text(None), "");
    }
}

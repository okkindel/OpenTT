//! What the menus need from the game: the [`MenuBackend`] trait and a
//! [`MockBackend`] for tests and previews.

use std::collections::HashMap;
use std::path::PathBuf;

/// Maximum number of player profiles.
pub const MAX_PLAYERS: usize = 6;

/// A saved player profile.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInfo {
    pub name: String,
    /// `getCombatRatio(i)` (already formatted; "---" when unknown).
    pub combat_ratio: String,
    /// `getCombatExp(i)`.
    pub combat_exp: String,
    /// `getSolohiscore(i)`.
    pub solo_high_score: String,
}

/// One selectable tank type.
#[derive(Debug, Clone, PartialEq)]
pub struct TankInfo {
    /// "Light", "Medium", "Heavy", "Baby Boss".
    pub name: String,
    /// Normalised 0..1 bars (from max_speed, acceleration, turn rate, armor/damage).
    pub speed: f32,
    pub acceleration: f32,
    pub handling: f32,
    pub armor: f32,
    /// Not available yet (Baby Boss needs a solo high score above 500000).
    pub locked: bool,
    /// Why it is locked, shown to the player.
    pub lock_reason: Option<String>,
}

/// One row of the round results (a player or a team).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResultRow {
    pub name: String,
    /// Team colour (None: free for all).
    pub team: Option<Team>,
    /// Score (scrum: goals).
    pub score: i64,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    /// The local player.
    pub me: bool,
}

/// The round results, shown over the game until the next map is loaded.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoundResults {
    /// Display name of the map just played.
    pub mission: String,
    /// Scrum: the score column counts goals.
    pub scrum: bool,
    /// Team totals (team games), best first.
    pub teams: Vec<ResultRow>,
    /// Players, best score first (the game's order).
    pub players: Vec<ResultRow>,
    /// K/D/A columns available.
    pub kda: bool,
    /// Seconds until the next map starts loading, when known.
    pub next_in: Option<f32>,
    /// Loading progress of the next map (`endGameLoadingProgress`), 0..1, once it started.
    pub loading: Option<f32>,
}

impl RoundResults {
    pub fn example() -> RoundResults {
        let p = |name: &str, team, score, kills, deaths, assists, me| ResultRow { name: name.into(), team: Some(team), score, kills, deaths, assists, me };
        RoundResults {
            mission: "Cranium Crater".into(),
            scrum: false,
            teams: vec![ResultRow { name: "Green Team".into(), team: Some(Team::Green), score: 9, ..Default::default() }, ResultRow { name: "Blue Team".into(), team: Some(Team::Blue), score: 5, ..Default::default() }],
            players: vec![p("Zot", Team::Green, 4, 5, 1, 2, false), p("Bait", Team::Blue, 3, 4, 1, 1, true), p("Hum", Team::Green, 3, 3, 2, 3, false), p("Rascal the Great", Team::Green, 2, 2, 3, 0, false), p("Mud", Team::Blue, 1, 2, 4, 1, false), p("Tom", Team::Blue, 0, 1, 6, 2, false)],
            kda: true,
            next_in: Some(2.0),
            loading: None,
        }
    }
}

/// One entry of the Solo screen's level select (a campaign position).
#[derive(Debug, Clone, PartialEq)]
pub struct SoloLevel {
    /// Campaign position (1-based, the level number shown in game).
    pub position: u32,
    pub mission: String,
    /// Display name of the map.
    pub title: String,
    /// "Battle", "Brain" or "Boss".
    pub level_type: String,
    pub briefing: String,
    /// 1 on the first pass through the worlds, 2+ once the campaign repeats them, harder.
    pub cycle: u32,
    /// The current player has reached this position.
    pub unlocked: bool,
}

impl SoloLevel {
    /// The first 20 positions of the campaign as an example, `reached` unlocked.
    pub fn examples(reached: u32) -> Vec<SoloLevel> {
        let worlds = [
            [("tt1_1", "Medulla Mesa"), ("tt1_2", "Hypothalamus Hills"), ("tt1_3", "Cranium Crater"), ("tt1_4", "Freudian Flats"), ("greenbrain", "Green Brain")],
            [("tt2_1", "Hippocampus Hollow"), ("tt2_2", "Comatose Canyon"), ("tt2_3", "Greymatter Gorge"), ("tt2_4", "Pituitary Plateau"), ("bluebrain", "Blue Brain")],
            [("tt3_1", "Synaptic Slopes"), ("tt3_2", "Ganglia Gully"), ("tt3_3", "Neurotic Nexus"), ("tt3_4", "Receptor Ridge"), ("redbrain", "Red Brain")],
        ];
        (0..20u32)
            .map(|i| {
                let (w, k) = ((i / 5) % 3, i % 5);
                let (mission, title) = worlds[w as usize][k as usize];
                SoloLevel {
                    position: i + 1,
                    mission: mission.into(),
                    title: title.into(),
                    level_type: ["Battle", "Battle", "Battle", "Boss", "Brain"][k as usize].into(),
                    briefing: String::new(),
                    cycle: 1 + i / 15,
                    unlocked: i < reached,
                }
            })
            .collect()
    }
}

/// The solo campaign state as `soloPlayGui` shows it. The campaign is a run
/// of levels with three lives. Between runs a level the player has reached
/// can be chosen ([`SoloStatus::levels`], `MenuAction::StartSoloAt`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoloStatus {
    /// 1-based level number of the level about to be played.
    pub level_number: u32,
    /// Mission id of that level (`SPLevelN.mission`, e.g. "tt1_2", "greenbrain").
    pub mission: String,
    /// Display name of the map.
    pub title: String,
    pub briefing: String,
    /// Lives left in the current run (3 at the start).
    pub lives: u32,
    /// Score of the current run.
    pub score: i64,
    /// A run is in progress (connected to the solo server between levels).
    pub in_run: bool,
    /// Score tally of the level just finished (label, value), as `presentScore` lists it.
    pub tally: Vec<(String, String)>,
    /// The same tally step by step (`advanceScore`), for the animated
    /// presentation of `soloPlayGui`; `None` before the first level.
    pub tally_anim: Option<SoloTally>,
    /// The level select (campaign positions, unlocked as reached).
    pub levels: Vec<SoloLevel>,
}

/// A row kind of the solo tally (`soloBronze`, `soloSilver`, `soloGold`, `soloBrain`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TallyKind {
    Bronze,
    Silver,
    Gold,
    Brain,
}

impl TallyKind {
    pub const ALL: [TallyKind; 4] = [TallyKind::Bronze, TallyKind::Silver, TallyKind::Gold, TallyKind::Brain];
}

/// What one `advanceScore` call did (each one plays at most one sound).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TallyEvent {
    /// "Bonus Points:" fades in (`SPBonus`).
    BonusText,
    /// A row appears at 0 (`SPCount1`).
    ShowRow(TallyKind),
    /// One more kill counted on a row (`SPCount2`).
    Count(TallyKind),
    /// Survival bonus row (`SPSurvival`).
    Survival,
    /// Boss bonus row (`SPSurvival`).
    Boss,
    /// Completion bonus row (`SPSurvival`).
    Completion,
    /// 10 000 points crossed: "EXTRA LIFE!" appears (`SPExtra`).
    ExtraLifeStart,
    /// The "EXTRA LIFE!" text blinks (toggles).
    ExtraLifeFlash,
    /// The blinking ends and the life is added.
    ExtraLife,
    /// Nothing left: the tally is complete.
    Done,
}

impl TallyEvent {
    /// The sound `advanceScore` plays for this step.
    pub fn sound(self) -> Option<UiSound> {
        Some(match self {
            TallyEvent::BonusText => UiSound::Bonus,
            TallyEvent::ShowRow(_) => UiSound::Count1,
            TallyEvent::Count(_) => UiSound::Count2,
            TallyEvent::Survival | TallyEvent::Boss | TallyEvent::Completion => UiSound::Survival,
            TallyEvent::ExtraLifeStart => UiSound::Extra,
            _ => return None,
        })
    }
}

/// One `advanceScore` step: the event, the delay before the next step (ms,
/// -1 on the last) and the run's score and lives after it.
#[derive(Debug, Clone, PartialEq)]
pub struct TallyStepInfo {
    pub event: TallyEvent,
    pub delay_ms: i32,
    pub score: i64,
    pub lives: u32,
}

/// The animated tally of `soloPlayGui::presentScore`.
#[derive(Debug, Clone, PartialEq)]
pub struct SoloTally {
    pub start_score: i64,
    pub start_lives: u32,
    /// Points per kill of each row (`bronze_pts * 1.5`, ..., `brain_pts * 2`), in [`TallyKind::ALL`] order.
    pub pts_per: [i64; 4],
    /// `LocalClientConnection.maxBrains` (the "/20" of the brain row).
    pub max_brains: i32,
    pub survival_pts: i64,
    pub boss_pts: i64,
    pub completion_pts: i64,
    pub steps: Vec<TallyStepInfo>,
}

impl SoloTally {
    /// A Battle level like `soloPlayGui::testScore`, smaller: 3 bronze, 2 silver
    /// and 1 gold kill from 9100 points with no deaths (survival bonus), crossing
    /// 10 000 (an extra life).
    pub fn example() -> SoloTally {
        let pts_per = [75, 150, 225, 0];
        let (mut score, mut lives) = (9100i64, 2u32);
        let mut steps = Vec::new();
        let mut step = |e: TallyEvent, d: i32, add: i64, life: u32| {
            score += add;
            lives += life;
            steps.push(TallyStepInfo { event: e, delay_ms: d, score, lives });
        };
        step(TallyEvent::BonusText, 1500, 0, 0);
        for (k, n) in [(TallyKind::Bronze, 3), (TallyKind::Silver, 2), (TallyKind::Gold, 1)] {
            step(TallyEvent::ShowRow(k), 750, 0, 0);
            for _ in 0..n {
                step(TallyEvent::Count(k), 250, pts_per[k as usize], 0);
            }
        }
        step(TallyEvent::Survival, 500, 750, 0);
        step(TallyEvent::ExtraLifeStart, 150, 0, 0);
        for _ in 0..8 {
            step(TallyEvent::ExtraLifeFlash, 250, 0, 0);
        }
        step(TallyEvent::ExtraLife, 250, 0, 1);
        step(TallyEvent::Done, -1, 0, 0);
        SoloTally { start_score: 9100, start_lives: 2, pts_per, max_brains: 0, survival_pts: 750, boss_pts: 6000, completion_pts: 10000, steps }
    }
}

/// Interface sounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiSound {
    /// `AudioButtonOver` (`soundButtonOver` of every shell button profile).
    ButtonOver,
    /// `AudioButtonDown` (`soundButtonDown`).
    ButtonDown,
    /// `SPCount1`: a tally row appears.
    Count1,
    /// `SPCount2`: one kill counted.
    Count2,
    /// `SPBonus`: "Bonus Points:".
    Bonus,
    /// `SPSurvival`: survival, boss or completion bonus.
    Survival,
    /// `SPExtra`: extra life.
    Extra,
}

impl UiSound {
    /// The `SoundDef` def name.
    pub fn profile(self) -> &'static str {
        match self {
            UiSound::ButtonOver => "AudioButtonOver",
            UiSound::ButtonDown => "AudioButtonDown",
            UiSound::Count1 => "SPCount1",
            UiSound::Count2 => "SPCount2",
            UiSound::Bonus => "SPBonus",
            UiSound::Survival => "SPSurvival",
            UiSound::Extra => "SPExtra",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HighScore {
    pub name: String,
    pub score: i64,
    /// Level reached.
    pub level: u32,
    /// The run started at this level (level select), not at the first.
    pub start_level: Option<u32>,
}

/// A mission playable in a custom game (the 12 multiplayer maps).
#[derive(Debug, Clone, PartialEq)]
pub struct MissionInfo {
    /// Mission file relative to the game root ("missions/tt2_3.ron").
    pub file: String,
    /// Id used for pictures ("tt2_3").
    pub id: String,
    /// Display name (`createGameGui::getMissionDisplayName`, e.g. "Greymatter Gorge").
    pub name: String,
    /// World of the map ("Green" TT1, "Blue" TT2, "Red" TT3, "Moon" TT4).
    pub world: String,
}

/// A game mode of the server scripts.
#[derive(Debug, Clone, PartialEq)]
pub struct GameModeInfo {
    /// The gameTypeIndex (0 Battlemode, 1 Team Battlemode, 2 Scrum, 3 Team Scrum).
    pub index: u32,
    pub label: String,
    /// Uses teams.
    pub teams: bool,
}

/// Team choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Team {
    #[default]
    Auto,
    Green,
    Blue,
}

impl Team {
    pub const ALL: [Team; 3] = [Team::Auto, Team::Green, Team::Blue];
    pub fn label(self) -> &'static str {
        match self {
            Team::Auto => "Auto",
            Team::Green => "Green",
            Team::Blue => "Blue",
        }
    }
}

/// Bot skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BotSkill {
    #[default]
    Bronze,
    Silver,
    Gold,
}

impl BotSkill {
    pub const ALL: [BotSkill; 3] = [BotSkill::Bronze, BotSkill::Silver, BotSkill::Gold];
    pub fn label(self) -> &'static str {
        match self {
            BotSkill::Bronze => "Bronze",
            BotSkill::Silver => "Silver",
            BotSkill::Gold => "Gold",
        }
    }
}

/// One bot in a custom game.
#[derive(Debug, Clone, PartialEq)]
pub struct BotSetup {
    pub name: String,
    pub skill: BotSkill,
    /// Index into [`MenuBackend::tanks`] (Light/Medium/Heavy for bots).
    pub tank: usize,
    pub team: Team,
}

/// Everything needed to start a local custom game.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomGame {
    /// [`MissionInfo::file`].
    pub mission: String,
    /// [`GameModeInfo::index`].
    pub mode: u32,
    pub bots: Vec<BotSetup>,
    pub player_tank: usize,
    pub player_team: Team,
    /// Round length in minutes.
    pub minutes: u32,
    /// (createGameGui's "Replace bots"): a human joining takes a bot's
    /// place, the bot returns when they leave.
    pub replace_bots: bool,
}

/// One server of the join list (what `joinGameGui` showed).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerEntry {
    /// What [`crate::MenuAction::Join`] takes (`IP:a.b.c.d:port`).
    pub address: String,
    pub name: String,
    /// Display name of the map.
    pub map: String,
    /// "Battlemode", "Team Scrum", ...
    pub mode: String,
    /// Humans and bots.
    pub players: u32,
    pub max_players: u32,
    pub bots: u32,
    pub ping_ms: u32,
    /// Needs a password (the lock icon).
    pub password: bool,
    /// Same game version: joining can work.
    pub compatible: bool,
    /// The server's game version ("0.1.2"; empty when it does not say).
    pub version: String,
    pub dedicated: bool,
    /// Names of the players on it.
    pub player_names: Vec<String>,
    /// The players with their teams and scores (the server's info reply).
    pub player_list: Vec<ServerPlayer>,
    /// The server runs mods that change the game; joining needs the same
    /// ones ([`Self::mods`]: "name version" of those it listed).
    pub modded: bool,
    pub mods: Vec<String>,
}

impl ServerEntry {
    /// A team mode ("Team Battlemode", "Team Scrum"): joining can pick a team.
    pub fn team_game(&self) -> bool {
        self.mode.to_ascii_lowercase().starts_with("team")
    }
}

/// One player of a server's info reply.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerPlayer {
    pub name: String,
    /// `Auto` = no team (free-for-all modes).
    pub team: Team,
    pub score: i32,
}

impl ServerPlayer {
    /// From the info reply's team letter (`G`, `B`, empty).
    pub fn new(team_letter: &str, name: &str, score: i32) -> ServerPlayer {
        let team = match team_letter.chars().next().map(|c| c.to_ascii_uppercase()) {
            Some('G') => Team::Green,
            Some('B') => Team::Blue,
            _ => Team::Auto,
        };
        ServerPlayer { name: name.to_string(), team, score }
    }
}

/// Where the join list comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ServerSource {
    /// Broadcast on the local network (`queryLanServers`).
    Lan,
    /// The master server's list; the default.
    #[default]
    Internet,
}

/// A listen server to host: the custom game (map, mode, round, bots) plus
/// the server settings. The host plays on it through the local connection.
#[derive(Debug, Clone, PartialEq)]
pub struct HostGame {
    pub game: CustomGame,
    pub name: String,
    pub max_players: u32,
    /// empty: none.
    pub password: String,
    /// 28000; the next free one of 10 is used.
    pub port: u16,
    /// Ask the router to forward the port.
    pub port_mapping: bool,
    /// List the server on the master server.
    pub public: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowMode {
    Windowed,
    #[default]
    Fullscreen,
    /// Borderless window covering the monitor at desktop resolution.
    Borderless,
}

impl WindowMode {
    pub const ALL: [WindowMode; 3] = [WindowMode::Fullscreen, WindowMode::Borderless, WindowMode::Windowed];
    pub fn label(self) -> &'static str {
        match self {
            WindowMode::Windowed => "Windowed",
            WindowMode::Fullscreen => "Fullscreen",
            WindowMode::Borderless => "Borderless",
        }
    }
}

/// All user settings the menu edits. Volumes and sensitivity are 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Game resolution.
    pub resolution: (u32, u32),
    pub window_mode: WindowMode,
    pub vsync: bool,
    /// 0 Low, 1 Medium, 2 High.
    pub detail_level: u8,
    pub master_volume: f32,
    pub music_volume: f32,
    pub sound_volume: f32,
    pub mouse_sensitivity: f32,
    pub invert_mouse: bool,
    /// A/D steer like a car backing up while S is held.
    pub reverse_steering: bool,
    /// 0 classic, 1 camera aim.
    pub aim_mode: u8,
    pub glass_jar: bool,
    /// HDR, tonemap, glow.
    pub hdr: bool,
    pub bloom: f32,
    pub normal_maps: bool,
    /// ("Surface detail") 0..2.
    pub normal_strength: f32,
    /// 0 off, 1 low, 2 high.
    pub ssao: u8,
    /// frame-rate counter.
    pub show_fps: bool,
    pub dynamic_lights: bool,
    pub dynamic_light_strength: f32,
    /// HUD size 0.5..1.5.
    pub hud_scale: f32,
    pub ao_strength: f32,
    /// 0 monitor refresh, -1 unlimited, else fps.
    pub max_fps: i32,
    pub shadows: u8,
}

impl Settings {
    /// Graphics extras of the "Classic" (false) or "Modern" (true) preset.
    pub fn set_graphics_preset(&mut self, modern: bool) {
        self.glass_jar = modern;
        self.hdr = modern;
        self.bloom = 0.7;
        self.normal_maps = modern;
        self.ssao = u8::from(modern);
        self.shadows = u8::from(modern);
        self.dynamic_lights = modern;
        if modern && self.dynamic_light_strength <= 0.005 {
            self.dynamic_light_strength = 1.0;
        }
    }
}

/// Default normal-map strength ("Surface detail" shows it as 50 %; keep
/// in step with tt-shell's `GRAPHICS_DEFAULTS.normal_strength`).
pub const SURFACE_DETAIL_DEFAULT: f32 = 0.2;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            resolution: (1280, 720),
            window_mode: WindowMode::Fullscreen,
            vsync: true,
            detail_level: 2,
            master_volume: 1.0,
            music_volume: 0.8,
            sound_volume: 0.8,
            mouse_sensitivity: 0.25,
            invert_mouse: false,
            reverse_steering: false,
            aim_mode: 1,
            glass_jar: true,
            hdr: true,
            bloom: 0.7,
            normal_maps: true,
            normal_strength: SURFACE_DETAIL_DEFAULT,
            ssao: 1,
            show_fps: true,
            dynamic_lights: true,
            dynamic_light_strength: 1.0,
            hud_scale: 0.75,
            ao_strength: 0.5,
            max_fps: 0,
            shadows: 1,
        }
    }
}

/// A display mode of the current monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
}

/// One row of the controls list.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyBinding {
    pub action: String,
    /// Bound keys/buttons, human-readable ("W", "Up", "Mouse 1").
    pub keys: Vec<String>,
}

/// A message the game scripts want to show.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptMessage {
    pub text: String,
    /// Yes/No question (else a single OK button).
    pub yes_no: bool,
}

/// Loading progress (`loadingGui`).
#[derive(Debug, Clone, PartialEq)]
pub struct Loading {
    /// Map name (`handleLoadInfoMessage`).
    pub map: String,
    /// Phase text ("Loading defs", "Loading objects", "Lighting mission").
    pub phase: String,
    /// 0..1.
    pub progress: f32,
}

/// Artwork the menu draws; the backend loads it from the game files at
/// runtime (nothing is copied into the repository).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Art {
    /// Mission picture (`missions/<id>.png`).
    Mission(String),
    /// Tutorial slide `n` (1..8) of `MarqueeGui` (`backgrounds/Marquee<n>` + alpha).
    Slide(u32),
    /// The scene behind the slides (`backgrounds/MarqueeBackground`: a tank in the forest).
    SlideBackdrop,
    /// Tally row icon (`bitmaps/<Bronze|Silver|Gold>BotHead`, `bitmaps/BrainIcon`).
    TallyIcon(TallyKind),
    /// The solo life icon (`bitmaps/Lives`).
    Life,
}

/// Options → Mods: one installed mod (`docs/modding.md`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModEntry {
    /// The folder name (what the enabled list holds).
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub enabled: bool,
    /// Changes the game (data or maps): network games need it on every side.
    pub gameplay: bool,
    /// One of the example mods that ship with the game (an "Example" tag).
    pub example: bool,
}

/// Options → Mods: the installed mods and where they go.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModsInfo {
    /// In load order: the enabled ones first (a later one wins), then the
    /// disabled ones.
    pub mods: Vec<ModEntry>,
    /// The ids of the mods the running game uses, in load order.
    pub running: Vec<String>,
    /// The user's mods folder.
    pub folder: String,
}

impl ModsInfo {
    /// Mods of every kind, the two example mods among them (previews and tests).
    pub fn example() -> ModsInfo {
        let m = |id: &str, name: &str, version: &str, author: &str, description: &str, enabled, gameplay| ModEntry { id: id.into(), name: name.into(), version: version.into(), author: author.into(), description: description.into(), enabled, gameplay, example: false };
        let example = |e: ModEntry| ModEntry { example: true, ..e };
        ModsInfo {
            mods: vec![
                m("hd-tanks", "HD Tank Skins", "1.2", "Krevitz", "Sharper camouflage for the three tanks and the Baby Boss.", true, false),
                m("lava-arenas", "Lava Arenas", "0.9", "Locke", "Two new red-world maps with jump pads over the lava.", true, true),
                m("retro-sounds", "Retro Sounds", "2.0", "", "8-bit shots and explosions.", false, false),
                example(m("carpet-bombing", "Carpet Bombing (example)", "1.0", "OpenTT", "An example gameplay mod: the Mortar fires three volleys of nine shells a pull.", false, true)),
                example(m("desert-camo", "Desert Camo (example)", "1.0", "OpenTT", "An example texture mod: desert camouflage for the tanks.", false, false)),
            ],
            running: vec!["hd-tanks".into()],
            folder: format!("~/.local/share/{}/mods", tt_assets::product::DATA_DIR),
        }
    }

    /// The enabled ids in load order.
    pub fn enabled(&self) -> Vec<String> {
        self.mods.iter().filter(|m| m.enabled).map(|m| m.id.clone()).collect()
    }

    /// The enabled list differs from what the running game uses.
    pub fn restart_needed(&self) -> bool {
        let a: Vec<String> = self.enabled().iter().map(|s| s.to_ascii_lowercase()).collect();
        let b: Vec<String> = self.running.iter().map(|s| s.to_ascii_lowercase()).collect();
        a != b
    }
}

/// Settings → Game data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GameDataInfo {
    /// The ThinkTanks folder it was imported from (empty: not recorded).
    pub source: String,
    /// When (empty: not recorded).
    pub imported: String,
    /// The asset directory in use.
    pub location: String,
}

/// Queries and commands the menus need. Flow changes (starting a game,
/// quitting) are returned from [`crate::MenuState::ui`] as
/// [`crate::MenuAction`]s instead.
pub trait MenuBackend {
    // ---- players (loadPlayerGui)
    fn players(&self) -> Vec<PlayerInfo>;
    /// The active player, if any.
    fn current_player(&self) -> Option<String>;
    fn select_player(&mut self, name: &str) -> Result<(), String>;
    /// `loadPlayerGui.createPlayer(name)` + export; errors for an empty/duplicate
    /// name or a full list ([`MAX_PLAYERS`]).
    fn create_player(&mut self, name: &str) -> Result<(), String>;
    /// `loadPlayerGui.deletePlayer(i)` (refuses to delete the last player).
    fn delete_player(&mut self, name: &str) -> Result<(), String>;
    /// Ask "play the tutorial?" now (after creating a player, like `doTutorial`).
    fn wants_tutorial_prompt(&self) -> bool;
    /// The tutorial's slide art is in the game data (some editions of
    /// ThinkTanks have none: the tutorial is then not offered).
    fn tutorial_available(&mut self) -> bool {
        true
    }

    // ---- tanks (tankSelectionGui)
    fn tanks(&self) -> Vec<TankInfo>;
    fn selected_tank(&self) -> usize;
    fn select_tank(&mut self, index: usize);
    /// A live 3D preview of tank `index` (the engine renders the tank shape
    /// with its brain jar into an egui texture of about `size` pixels), or
    /// `None` to show the stats only.
    fn tank_preview(&mut self, index: usize, size: [u32; 2]) -> Option<egui::TextureId>;

    // ---- solo (soloPlayGui, soloGameOverGui)
    fn solo_status(&self) -> SoloStatus;
    /// Solo top-10.
    fn high_scores(&self) -> Vec<HighScore>;
    /// Rank (0-based) of the last finished run in [`MenuBackend::high_scores`], if it entered the table.
    fn last_run_rank(&self) -> Option<usize>;
    /// The round results while they are shown (`EndGameGui` is on the canvas).
    fn round_results(&self) -> Option<RoundResults> {
        None
    }

    // ---- custom game (createGameGui, local only)
    fn missions(&self) -> Vec<MissionInfo>;
    fn game_modes(&self) -> Vec<GameModeInfo>;
    fn max_bots(&self) -> usize;
    /// Default bot names (`Hum`, `Tom`, ... / `Mud`, `Zot`, ...).
    fn bot_names(&self) -> Vec<String>;
    /// Overview picture of a mission (terrain rendered by the engine), if available.
    fn mission_preview(&mut self, mission: &str, size: [u32; 2]) -> Option<egui::TextureId>;
    fn replace_bots(&self) -> bool {
        true
    }
    /// Health power-ups kept available at once on the server.
    fn min_health_packs(&self) -> u32 {
        2
    }
    fn set_min_health_packs(&mut self, _n: u32) {}

    // ---- in-game / transitions
    /// Loading progress while a mission loads, else `None`.
    fn loading(&self) -> Option<Loading>;
    /// A pending script message box, if any.
    fn script_message(&self) -> Option<ScriptMessage>;
    /// Answer it (OK / Yes = true): the backend evaluates the script command bound to the button.
    fn answer_script_message(&mut self, yes: bool);

    // ---- settings (optionsGui + modern extras)
    fn settings(&self) -> Settings;
    /// Apply and persist.
    fn apply_settings(&mut self, s: &Settings);
    /// The monitor's real modes, any order (the menu sorts and de-duplicates).
    fn display_modes(&self) -> Vec<DisplayMode>;
    /// The desktop resolution (default display mode).
    fn desktop_mode(&self) -> DisplayMode;
    fn key_bindings(&self) -> Vec<KeyBinding>;
    /// Where the game data came from (Settings → Game data); `None`: the
    /// game cannot import it again (no import support).
    fn game_data(&self) -> Option<GameDataInfo> {
        None
    }
    /// Options → Mods: the installed mods; `None`: no mod support.
    fn mods(&self) -> Option<ModsInfo> {
        None
    }
    /// Saves the enabled mods (ids, load order); used from the next start.
    fn set_mods(&mut self, _enabled: &[String]) {}
    /// Opens the user's mods folder in the file manager (best effort).
    fn open_mods_folder(&mut self) {}
    /// A newer release's tag ("v0.2.0") for the main menu's notice. Default: none.
    fn update_available(&self) -> Option<String> {
        None
    }
    /// Opens the releases page in the browser (the notice's button).
    fn open_releases(&mut self) {}

    // ---- text and art
    /// Credits text (`credits.hfl`, raw markup).
    fn credits(&self) -> String;
    /// `getVersionString()`.
    fn version(&self) -> String;
    /// Artwork as an egui texture.
    fn art(&mut self, ctx: &egui::Context, art: &Art) -> Option<egui::TextureHandle>;
    /// Play an interface sound (`alxPlay(<profile>)`). Default: silent.
    fn play_ui_sound(&mut self, _sound: UiSound) {}

    // ---- online (joinGameGui, joinPasswordDlg, the listen server)
    /// The servers found by the last LAN query (and by-address previews).
    fn lan_servers(&self) -> Vec<ServerEntry> {
        Vec::new()
    }
    /// Start a query of the current [`ServerSource`] (LAN: `queryLanServers`;
    /// internet: `queryMasterServer`), clearing the list.
    fn refresh_servers(&mut self) {}
    /// Switch the join list's source (the next refresh queries it).
    fn set_server_source(&mut self, _source: ServerSource) {}
    /// The master server address.
    fn master_server(&self) -> String {
        String::new()
    }
    fn set_master_server(&mut self, _address: &str) {}
    /// A query is collecting answers.
    fn querying(&self) -> bool {
        false
    }
    /// Ask one address for its info (it joins the list). False if it does not resolve.
    fn query_address(&mut self, _address: &str) -> bool {
        false
    }
    /// The saved host settings (prefs), or defaults.
    fn host_settings(&self) -> Option<HostGame> {
        None
    }
    /// Addresses joined recently, newest first.
    fn recent_addresses(&self) -> Vec<String> {
        Vec::new()
    }
    /// The server at this address asked for a password (`CHR_PASSWORD`):
    /// the menu asks for one. Returned once.
    fn take_password_request(&mut self) -> Option<String> {
        None
    }
}

/// In-memory backend with plausible data, for tests and previews. If
/// `game_root` is set, artwork is loaded from that asset directory.
#[derive(Default)]
pub struct MockBackend {
    pub players: Vec<PlayerInfo>,
    pub current: Option<String>,
    pub tanks: Vec<TankInfo>,
    pub tank: usize,
    pub solo: Option<SoloStatus>,
    /// What `round_results` returns.
    pub results: Option<RoundResults>,
    pub scores: Vec<HighScore>,
    pub rank: Option<usize>,
    pub settings: Settings,
    pub modes: Vec<DisplayMode>,
    pub game_root: Option<PathBuf>,
    pub tutorial_prompt: bool,
    /// What `tutorial_available` returns (default true).
    pub tutorial: bool,
    pub loading: Option<Loading>,
    pub message: Option<ScriptMessage>,
    art: HashMap<Art, Option<egui::TextureHandle>>,
    /// Commands received, for tests ("select Alice", "create Bob", ...).
    pub log: Vec<String>,
    /// Interface sounds played, in order.
    pub sounds: Vec<UiSound>,
    /// Join list (tests, previews).
    pub servers: Vec<ServerEntry>,
    pub recent: Vec<String>,
    pub password_request: Option<String>,
    pub querying: bool,
    /// The internet list (tests, previews) and the current source.
    pub internet_servers: Vec<ServerEntry>,
    pub source: ServerSource,
    pub master: String,
    /// Options → Mods (`None`: no mod support).
    pub mods: Option<ModsInfo>,
    /// Bot names (empty: the defaults) and maps after the 12 of [`MAPS`] (previews).
    pub bot_names: Vec<String>,
    pub extra_missions: Vec<MissionInfo>,
    /// [`MenuBackend::update_available`].
    pub update: Option<String>,
}

/// The 12 multiplayer maps with their display names.
pub const MAPS: [(&str, &str); 12] = [
    ("tt1_1", "Medulla Mesa"),
    ("tt1_2", "Hypothalamus Hills"),
    ("tt1_3", "Cranium Crater"),
    ("tt1_4", "Freudian Flats"),
    ("tt2_1", "Hippocampus Hollow"),
    ("tt2_2", "Comatose Canyon"),
    ("tt2_3", "Greymatter Gorge"),
    ("tt2_4", "Pituitary Plateau"),
    ("tt3_1", "Synaptic Slopes"),
    ("tt3_2", "Ganglia Gully"),
    ("tt3_3", "Neurotic Nexus"),
    ("tt3_4", "Receptor Ridge"),
];

impl MockBackend {
    pub fn new() -> MockBackend {
        let p = |n: &str, r: &str, e: &str, s: &str| PlayerInfo { name: n.into(), combat_ratio: r.into(), combat_exp: e.into(), solo_high_score: s.into() };
        let t = |n: &str, sp, ac, ha, ar, locked: bool| TankInfo {
            name: n.into(),
            speed: sp,
            acceleration: ac,
            handling: ha,
            armor: ar,
            locked,
            lock_reason: locked.then(|| "Score 500000 in solo play to unlock".into()),
        };
        MockBackend {
            players: vec![p("Alice", "1.42", "125", "18640"), p("Bob", "0.87", "31", "4200"), p("Tester", "---", "---", "---")],
            current: Some("Alice".into()),
            tanks: vec![
                t("Light", 0.99, 0.93, 0.74, 0.56, false),
                t("Medium", 0.74, 0.8, 0.83, 0.67, false),
                t("Heavy", 0.57, 0.57, 1.0, 1.0, false),
                t("Baby Boss", 1.0, 1.0, 0.74, 0.86, true),
            ],
            tank: 1,
            solo: Some(SoloStatus {
                level_number: 3,
                mission: "tt1_3".into(),
                title: "Cranium Crater".into(),
                briefing: "<font:Arial Bold:16>Destroy the bot tanks. Gold bots are worth the most points - and shoot back the hardest.".into(),
                lives: 3,
                score: 10600,
                in_run: true,
                tally: vec![("Bronze bots".into(), "6 x 100".into()), ("Silver bots".into(), "3 x 250".into()), ("Survival bonus".into(), "1000".into())],
                tally_anim: Some(SoloTally::example()),
                levels: Vec::new(),
            }),
            scores: [("Jonny", 498720, 31), ("Krevitz", 410010, 34), ("Shark", 310220, 27), ("Locke", 121700, 15), ("Patton", 83080, 12), ("Ribbit", 45070, 10), ("Mack", 25030, 8), ("Rascal", 18640, 8), ("Alice", 12450, 3), ("Mike", 3170, 3)]
                .iter()
                .map(|&(n, s, l)| HighScore { name: n.into(), score: s, level: l, start_level: (n == "Alice").then_some(2) })
                .collect(),
            rank: Some(8),
            settings: Settings::default(),
            modes: [(1920, 1080), (1280, 720), (2560, 1440), (1600, 900), (1920, 1080), (1024, 768), (800, 600)]
                .iter()
                .map(|&(width, height)| DisplayMode { width, height })
                .collect(),
            game_root: None,
            tutorial_prompt: false,
            tutorial: true,
            loading: None,
            message: None,
            art: HashMap::new(),
            log: Vec::new(),
            sounds: Vec::new(),
            results: None,
            servers: MockBackend::example_servers(),
            recent: vec!["192.168.1.20:28000".into(), "tanks.example.org".into()],
            password_request: None,
            querying: false,
            internet_servers: Vec::new(),
            source: ServerSource::Lan,
            master: String::new(),
            mods: Some(ModsInfo::example()),
            bot_names: Vec::new(),
            extra_missions: Vec::new(),
            update: None,
        }
    }

    /// Three LAN servers of every kind (previews and tests).
    pub fn example_servers() -> Vec<ServerEntry> {
        let s = |a: &str, n: &str, m: &str, mode: &str, p, max, bots, ping, pw, ok, ded| ServerEntry {
            address: a.into(),
            name: n.into(),
            map: m.into(),
            mode: mode.into(),
            players: p,
            max_players: max,
            bots,
            ping_ms: ping,
            password: pw,
            compatible: ok,
            version: if ok { env!("CARGO_PKG_VERSION") } else { "0.0.9" }.into(),
            dedicated: ded,
            player_names: ["Hum [Bot]", "Krevitz", "Locke", "Mal [Bot]", "Shark"].iter().take(p as usize).map(|s| s.to_string()).collect(),
            player_list: ["Hum [Bot]", "Krevitz", "Locke", "Mal [Bot]", "Shark"]
                .iter()
                .take(p as usize)
                .enumerate()
                .map(|(i, n)| ServerPlayer::new(if !mode.starts_with("Team") { "" } else if i % 2 == 0 { "G" } else { "B" }, n, [4, 11, 2, 0, 7][i]))
                .collect(),
            modded: n.contains("Scrum"),
            mods: if n.contains("Scrum") { vec!["Scrum Arenas 1.1".into()] } else { Vec::new() },
        };
        vec![
            s("IP:192.168.1.20:28000", "Brain Drain LAN", "Medulla Mesa", "Battlemode", 5, 16, 3, 12, false, true, true),
            s("IP:192.168.1.31:28000", "Krevitz's Scrum", "Cranium Crater", "Team Scrum", 3, 8, 1, 38, true, true, false),
            s("IP:192.168.1.44:28001", "old build", "Synaptic Slopes", "Battlemode", 2, 10, 0, 85, false, false, true),
        ]
    }
}

impl MenuBackend for MockBackend {
    fn players(&self) -> Vec<PlayerInfo> {
        self.players.clone()
    }
    fn current_player(&self) -> Option<String> {
        self.current.clone()
    }
    fn select_player(&mut self, name: &str) -> Result<(), String> {
        self.log.push(format!("select {name}"));
        self.current = Some(name.into());
        Ok(())
    }
    fn create_player(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Please enter a name.".into());
        }
        if self.players.len() >= MAX_PLAYERS {
            return Err(format!("There can be at most {MAX_PLAYERS} players. Delete one first."));
        }
        if self.players.iter().any(|p| p.name.eq_ignore_ascii_case(name)) {
            return Err("A player with that name already exists.".into());
        }
        self.log.push(format!("create {name}"));
        self.players.push(PlayerInfo { name: name.into(), combat_ratio: "---".into(), combat_exp: "---".into(), solo_high_score: "---".into() });
        self.current = Some(name.into());
        self.tutorial_prompt = true;
        Ok(())
    }
    fn delete_player(&mut self, name: &str) -> Result<(), String> {
        if self.players.len() < 2 {
            return Err("The last player cannot be deleted.".into());
        }
        self.log.push(format!("delete {name}"));
        self.players.retain(|p| p.name != name);
        if self.current.as_deref() == Some(name) {
            self.current = self.players.first().map(|p| p.name.clone());
        }
        Ok(())
    }
    fn wants_tutorial_prompt(&self) -> bool {
        self.tutorial_prompt
    }
    fn tutorial_available(&mut self) -> bool {
        self.tutorial
    }
    fn tanks(&self) -> Vec<TankInfo> {
        self.tanks.clone()
    }
    fn selected_tank(&self) -> usize {
        self.tank
    }
    fn select_tank(&mut self, index: usize) {
        self.log.push(format!("tank {index}"));
        self.tank = index;
    }
    fn tank_preview(&mut self, _index: usize, _size: [u32; 2]) -> Option<egui::TextureId> {
        None
    }
    fn solo_status(&self) -> SoloStatus {
        self.solo.clone().unwrap_or(SoloStatus {
            level_number: 1,
            mission: "tt1_1".into(),
            title: "Medulla Mesa".into(),
            briefing: String::new(),
            lives: 3,
            score: 0,
            in_run: false,
            tally: Vec::new(),
            tally_anim: None,
            levels: SoloLevel::examples(5),
        })
    }
    fn high_scores(&self) -> Vec<HighScore> {
        self.scores.clone()
    }
    fn last_run_rank(&self) -> Option<usize> {
        self.rank
    }
    fn round_results(&self) -> Option<RoundResults> {
        self.results.clone()
    }
    fn missions(&self) -> Vec<MissionInfo> {
        MAPS.iter()
            .map(|&(id, name)| MissionInfo {
                file: tt_assets::layout::mission(id),
                id: id.into(),
                name: name.into(),
                world: match &id[..3] {
                    "TT1" => "Green",
                    "TT2" => "Blue",
                    _ => "Red",
                }
                .into(),
            })
            .chain(self.extra_missions.iter().cloned())
            .collect()
    }
    fn game_modes(&self) -> Vec<GameModeInfo> {
        let g = |index, label: &str, teams| GameModeInfo { index, label: label.into(), teams };
        vec![g(0, "Battlemode", false), g(1, "Team Battlemode", true), g(2, "Scrum", false), g(3, "Team Scrum", true)]
    }
    fn max_bots(&self) -> usize {
        6
    }
    fn bot_names(&self) -> Vec<String> {
        if !self.bot_names.is_empty() {
            return self.bot_names.clone();
        }
        ["Hum", "Tom", "Mal", "Ri", "Gul", "Gum"].iter().map(|s| s.to_string()).collect()
    }
    fn mission_preview(&mut self, _mission: &str, _size: [u32; 2]) -> Option<egui::TextureId> {
        None
    }
    fn loading(&self) -> Option<Loading> {
        self.loading.clone()
    }
    fn script_message(&self) -> Option<ScriptMessage> {
        self.message.clone()
    }
    fn answer_script_message(&mut self, yes: bool) {
        self.log.push(format!("message {}", if yes { "yes" } else { "no" }));
        self.message = None;
    }
    fn settings(&self) -> Settings {
        self.settings.clone()
    }
    fn apply_settings(&mut self, s: &Settings) {
        self.log.push("apply settings".into());
        self.settings = s.clone();
    }
    fn display_modes(&self) -> Vec<DisplayMode> {
        self.modes.clone()
    }
    fn desktop_mode(&self) -> DisplayMode {
        DisplayMode { width: 1920, height: 1080 }
    }
    fn game_data(&self) -> Option<GameDataInfo> {
        Some(GameDataInfo { source: "/home/player/Games/ThinkTanks".into(), imported: "2026-10-18 12:00 UTC".into(), location: format!("/home/player/.local/share/{}/gamedata", tt_assets::product::DATA_DIR) })
    }
    fn key_bindings(&self) -> Vec<KeyBinding> {
        let k = |a: &str, keys: &[&str]| KeyBinding { action: a.into(), keys: keys.iter().map(|s| s.to_string()).collect() };
        vec![
            k("Move forward", &["W", "Up", "Numpad 8"]),
            k("Move backward", &["S", "Down", "Numpad 2", "Numpad 5"]),
            k("Turn left", &["A", "Left", "Numpad 4"]),
            k("Turn right", &["D", "Right", "Numpad 6"]),
            k("Fire", &["Mouse 1"]),
            k("Aim turret", &["Mouse"]),
            k("Respawn", &["Space"]),
            k("Chat", &["U", "/", "\\"]),
            k("Player list", &["Tab"]),
            k("Menu / pause", &["Escape"]),
        ]
    }
    fn credits(&self) -> String {
        // The game's own text when the assets are there (menu previews).
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/ui/credits.txt");
        std::fs::read_to_string(path).unwrap_or_else(|_| tt_assets::product::NAME.into())
    }
    fn version(&self) -> String {
        format!("{} {}", tt_assets::product::NAME, env!("CARGO_PKG_VERSION"))
    }
    fn art(&mut self, ctx: &egui::Context, art: &Art) -> Option<egui::TextureHandle> {
        if let Some(t) = self.art.get(art) {
            return t.clone();
        }
        let tex = self.game_root.as_ref().and_then(|root| crate::art::load_art(ctx, root, art));
        self.art.insert(art.clone(), tex.clone());
        tex
    }
    fn play_ui_sound(&mut self, sound: UiSound) {
        self.sounds.push(sound);
    }
    fn lan_servers(&self) -> Vec<ServerEntry> {
        match self.source {
            ServerSource::Lan => self.servers.clone(),
            ServerSource::Internet => self.internet_servers.clone(),
        }
    }
    fn refresh_servers(&mut self) {
        self.log.push(match self.source {
            ServerSource::Lan => "refresh".into(),
            ServerSource::Internet => format!("refresh internet {}", self.master),
        });
    }
    fn set_server_source(&mut self, source: ServerSource) {
        self.log.push(format!("source {source:?}"));
        self.source = source;
    }
    fn master_server(&self) -> String {
        self.master.clone()
    }
    fn set_master_server(&mut self, address: &str) {
        self.log.push(format!("master {address}"));
        self.master = address.to_string();
    }
    fn querying(&self) -> bool {
        self.querying
    }
    fn query_address(&mut self, address: &str) -> bool {
        self.log.push(format!("query {address}"));
        !address.trim().is_empty()
    }
    fn host_settings(&self) -> Option<HostGame> {
        None
    }
    fn recent_addresses(&self) -> Vec<String> {
        self.recent.clone()
    }
    fn mods(&self) -> Option<ModsInfo> {
        self.mods.clone()
    }
    fn set_mods(&mut self, enabled: &[String]) {
        self.log.push(format!("mods {}", enabled.join(",")));
        if let Some(m) = self.mods.as_mut() {
            let mut list: Vec<ModEntry> = enabled.iter().filter_map(|id| m.mods.iter().find(|e| &e.id == id).cloned()).map(|e| ModEntry { enabled: true, ..e }).collect();
            list.extend(m.mods.iter().filter(|e| !enabled.contains(&e.id)).cloned().map(|e| ModEntry { enabled: false, ..e }));
            m.mods = list;
        }
    }
    fn open_mods_folder(&mut self) {
        self.log.push("open mods folder".into());
    }
    fn update_available(&self) -> Option<String> {
        self.update.clone()
    }
    fn open_releases(&mut self) {
        self.log.push("open releases".into());
    }
    fn take_password_request(&mut self) -> Option<String> {
        self.password_request.take()
    }
}

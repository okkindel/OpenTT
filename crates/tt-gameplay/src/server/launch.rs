//! The ways a game starts: a listen server, Quick Play, the demo, the
//! solo campaign, target practice and a dedicated server.

use super::state::*;
use super::*;

/// The local connection a launcher asks for: the player's name, team and
/// tank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalConnect {
    pub name: String,
    pub team: String,
    pub tank: String,
}

impl Server {
    /// The player's own connect arguments.
    pub(crate) fn player_args(&self, h: &dyn ServerHost, team: Option<&str>) -> LocalConnect {
        let team = match team {
            Some(t) => t.to_string(),
            None => h.setting(super::Setting::PlayerTeam),
        };
        LocalConnect { name: h.setting(super::Setting::PlayerName), team, tank: h.setting(super::Setting::PlayerTank) }
    }

    /// A listen server with the settings' bots and map rotation; then the
    /// local connection.
    pub fn launch_listen(&mut self, h: &mut dyn ServerHost, mission: &str, mission_type: MissionType, teams: bool) -> Option<LocalConnect> {
        self.bots_from_settings(h);
        self.setup.mission_type = mission_type;
        self.setup.team_game = teams;
        self.setup.target_range = Some(false);
        self.setup.rotation = self.rotation_setting(h, mission);
        self.setup.rotation_random = self.setting_on(h, super::Setting::MapRotationRandom);
        self.create_server(h, ServerType::MultiPlayer, mission);
        Some(self.player_args(h, None))
    }

    /// A free-for-all on a random mission of a world that suits the
    /// player's experience.
    pub fn launch_quick(&mut self, h: &mut dyn ServerHost) -> Option<LocalConnect> {
        let exp = h.combat_experience();
        let world = if exp < 8.0 || h.is_world_limited() {
            1
        } else if exp < 15.0 {
            if (self.rng.get_random_max(100) as f64) < 50.0 {
                1
            } else {
                2
            }
        } else if (self.rng.get_random_max(100) as f64) < 33.0 {
            1
        } else if (self.rng.get_random_max(100) as f64) < 50.0 {
            2
        } else {
            3
        };
        let files = h.maps(Some(world));
        if files.is_empty() {
            return None;
        }
        self.default_bots();
        self.setup.mission_type = MissionType::Deathmatch;
        self.setup.team_game = false;
        self.setup.target_range = Some(false);
        self.setup.rotation.clear();
        let file = self.random_mission(&files);
        self.create_server(h, ServerType::SinglePlayer, &file);
        Some(self.player_args(h, None))
    }

    pub fn launch_demo(&mut self, h: &mut dyn ServerHost) -> Option<LocalConnect> {
        self.default_bots();
        let mission = self.demo_mission(h);
        self.setup.demo_tank = self.demo_tank();
        self.setup.target_range = Some(false);
        self.create_server(h, ServerType::SinglePlayer, &mission);
        Some(LocalConnect { name: "Radicator".into(), team: "Green".into(), tank: self.setup.demo_tank.clone() })
    }

    /// After the demo's local connection: the player drives with the AI.
    pub fn after_demo_connect(&mut self) {
        if let Some(l) = self.local() {
            self.clients.get_mut(&l).unwrap().using_ai = true;
        }
        self.setup.ai_control = true;
    }

    pub fn launch_campaign(&mut self, h: &mut dyn ServerHost) -> Option<LocalConnect> {
        self.campaign.levels_loaded = true;
        self.setup.single_player = Some(true);
        self.setup.target_range = Some(false);
        self.campaign.mission_seq = Some(0);
        self.campaign.mission_seq_total = Some(0);
        self.setup.rotation.clear();
        let file = self.next_solo_level(h);
        self.create_server(h, ServerType::SinglePlayer, &file);
        Some(self.player_args(h, Some("")))
    }

    pub fn launch_target_practice(&mut self, h: &mut dyn ServerHost) -> Option<LocalConnect> {
        self.setup.rotation.clear();
        self.bot_slots.clear();
        self.setup.target_range = Some(true);
        self.setup.team_game = false;
        self.create_server(h, ServerType::SinglePlayer, "missions/target.ron");
        for db in ["TargetLight", "TargetLight", "TargetMedium", "TargetMedium", "TargetHeavy", "TargetHeavy", "TargetHeavy", "TargetHeavy", "TargetHeavy"] {
            let def = h.data().tanks.find(db);
            self.spawn_target(h, def);
        }
        if h.is_demo() {
            h.set_setting(super::Setting::PlayerTank, "1");
        }
        Some(self.player_args(h, Some("")))
    }

    /// A dedicated server on `mission`: a mission file (then the
    /// rotation of the settings, or that mission alone), or every map of
    /// the map list's world (`GREEN`, `BLUE`, `RED`, `MOON`) or of every
    /// world (`ALL`) as the rotation, starting on a random one.
    pub fn launch_dedicated(&mut self, h: &mut dyn ServerHost, mission: &str) {
        let mut mission = mission.to_string();
        self.bots_from_settings(h);
        let mode = self.setup.dedicated_mode;
        self.setup.mission_type = mode.mission_type();
        self.setup.team_game = mode.teams();
        self.setup.rotation_random = self.setting_on(h, super::Setting::MapRotationRandom);
        match world_of(&mission) {
            Some(w) => {
                let mut maps = h.maps(w);
                if maps.is_empty() {
                    h.error(&format!("--mission {mission}: no maps of that world; every world's instead"));
                    maps = h.maps(None);
                }
                mission = self.random_mission(&maps);
                self.setup.rotation = maps;
            }
            None => {
                self.setup.rotation = self.rotation_setting(h, &mission);
                // A random order starts on a random map too.
                if self.setup.rotation_random && self.setup.rotation.len() > 1 {
                    mission = self.random_mission(&self.setup.rotation.clone());
                }
            }
        }
        self.setup.target_range = Some(false);
        self.create_server(h, ServerType::MultiPlayer, &mission);
    }

    /// The rotation of the settings (`server.map_rotation`: mission ids, or
    /// `GREEN`, `BLUE`, `RED`, `MOON`, `ALL` for a world's maps or every
    /// world's, separated by spaces, commas or tabs; ids without a mission
    /// file are left out), or `mission` alone when it names none.
    pub fn rotation_setting(&mut self, h: &mut dyn ServerHost, mission: &str) -> Vec<String> {
        let mut files = Vec::new();
        for id in h.setting(super::Setting::MapRotation).split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()) {
            let file = tt_assets::layout::mission(id);
            if let Some(w) = world_of(id) {
                for f in h.maps(w) {
                    if !files.contains(&f) {
                        files.push(f);
                    }
                }
            } else if !h.is_file(&file) {
                h.error(&format!("server.map_rotation: no mission {id}"));
            } else if !files.contains(&file) {
                files.push(file);
            }
        }
        if files.is_empty() {
            files.push(mission.to_string());
        }
        files
    }
}

/// The world a rotation name stands for: `GREEN`, `BLUE`, `RED`, `MOON`
/// (one world), `ALL` (every world: `Some(None)`); a mission id: `None`.
fn world_of(name: &str) -> Option<Option<u8>> {
    [("GREEN", Some(1)), ("BLUE", Some(2)), ("RED", Some(3)), ("MOON", Some(4)), ("ALL", None)].iter().find(|(k, _)| name.eq_ignore_ascii_case(k)).map(|(_, w)| *w)
}

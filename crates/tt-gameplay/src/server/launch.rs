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

    /// A listen server with the settings' bots; then the local connection.
    pub fn launch_listen(&mut self, h: &mut dyn ServerHost, mission: &str, mission_type: MissionType, teams: bool) -> Option<LocalConnect> {
        self.bots_from_settings(h);
        self.setup.mission_type = mission_type;
        self.setup.team_game = teams;
        self.setup.target_range = Some(false);
        self.setup.stay_on_mission = true;
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
        self.setup.stay_on_mission = true;
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
        self.setup.stay_on_mission = true;
        let file = self.next_solo_level(h);
        self.create_server(h, ServerType::SinglePlayer, &file);
        Some(self.player_args(h, Some("")))
    }

    pub fn launch_target_practice(&mut self, h: &mut dyn ServerHost) -> Option<LocalConnect> {
        self.setup.stay_on_mission = true;
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

    /// A dedicated server on `mission`: a mission file, or a rotation of
    /// the map list (`GREEN`, `BLUE`, `RED`, `MOON`: a world's maps; `ALL`: the
    /// green, blue and red worlds') starting on a random one of them.
    pub fn launch_dedicated(&mut self, h: &mut dyn ServerHost, mission: &str) {
        let mut mission = mission.to_string();
        self.bots_from_settings(h);
        let mode = self.setup.dedicated_mode;
        self.setup.mission_type = mode.mission_type();
        self.setup.team_game = mode.teams();
        let world = [("GREEN", 1), ("BLUE", 2), ("RED", 3), ("MOON", 4)].iter().find(|(k, _)| mission.eq_ignore_ascii_case(k)).map(|(_, w)| *w);
        let rotation = match world {
            Some(w) => Some(Rotation::World(w)),
            None if mission.eq_ignore_ascii_case("ALL") => Some(Rotation::All),
            None => None,
        };
        match rotation {
            Some(r) => {
                let maps = h.maps(r.world());
                mission = self.random_mission(&maps);
                self.setup.stay_on_mission = false;
                self.setup.rotation = r;
            }
            None => self.setup.stay_on_mission = true,
        }
        self.setup.target_range = Some(false);
        self.create_server(h, ServerType::MultiPlayer, &mission);
    }
}

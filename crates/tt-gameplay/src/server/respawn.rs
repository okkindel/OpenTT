//! Tanks for players: where they spawn (the spawn points, the scrum
//! goal) and the player's tank with its driver, cartridge, name and team.

use super::state::*;
use super::*;
use crate::data::*;
use crate::entity::EntityId;
use tt_game::fmt::{atof, format_g};

/// `a + b` of two position texts.
pub fn vector_add(a: &str, b: &str) -> String {
    let v = |s: &str| {
        let mut v = [0.0f64; 3];
        for (i, w) in s.split_whitespace().take(3).enumerate() {
            v[i] = atof(w) as f32 as f64;
        }
        v
    };
    let (a, b) = (v(a), v(b));
    let f = |x: f64| format_g(x as f32 as f64);
    format!("{} {} {}", f(a[0] + b[0]), f(a[1] + b[1]), f(a[2] + b[2]))
}

impl Server {
    /// A new tank of kind `def` for `client`, one of the round's objects.
    pub fn new_tank(&mut self, h: &mut dyn ServerHost, def: Ref<TankDef>, client: Option<ClientId>) -> Option<EntityId> {
        let e = h.new_tank(def, client)?;
        self.tanks.insert(e, TankInfo { client });
        self.cleanup_add(e);
        Some(e)
    }

    pub fn leave_game(&mut self, h: &mut dyn ServerHost, this: ClientId) {
        let player = self.client(this).and_then(|c| c.player);
        if self.tank_exists(h, player) {
            self.delete_tank(h, player.unwrap());
        }
    }

    pub fn spawn_player(&mut self, h: &mut dyn ServerHost, this: ClientId) {
        let mut team_name = String::new();
        if self.setup.single_player() || self.setup.target_range() {
            team_name = "Blue".into();
        } else {
            let team = self.team_of(this);
            if self.team(team).is_some() && self.setup.mission_type != MissionType::Scrum {
                team_name = self.team_short(team);
            }
        }
        let pos = self.spawn_point(h, &team_name);
        self.create_player(h, this, &pos);
    }

    pub fn create_player(&mut self, h: &mut dyn ServerHost, this: ClientId, pos: &str) {
        if self.client(this).is_some_and(|c| c.player.is_some()) {
            h.error("Attempting to create a tank twice!");
        }
        let Some(def) = self.client(this).and_then(|c| c.starting_tank) else { return };
        let Some(player) = self.new_tank(h, def, Some(this)) else { return };
        if self.setup.single_player() && self.campaign.kind() == Some(LevelKind::Brain) {
            if let Some(boss) = h.data().tanks.find("BabyBoss") {
                h.set_tank_def(player, boss);
            }
        }
        if self.client(this).is_some_and(|c| c.using_ai) {
            let ai = h.data().tanks[def].ai;
            h.set_ai(player, ai);
        }
        h.set_transform(player, pos);
        if let Some(p) = h.tank_def(player).and_then(|t| h.data().tanks[t].default_projectile) {
            h.set_projectile(player, p);
        }
        let name = self.rec(Some(this), |r| r.name().to_string());
        h.set_player_name(player, &name);
        if self.setup.single_player() {
            h.set_team(player, "Green");
        } else {
            let team = self.team_of(this);
            if self.team(team).is_some() {
                let short = self.team_short(team);
                h.set_team(player, &short);
            } else {
                h.set_team(player, "Red");
            }
        }
        if let Some(c) = self.clients.get_mut(&this) {
            c.player = Some(player);
        }
        h.set_control(this, player);
    }

    /// A random spawn point of `<team>PlayerDropPoints` (else
    /// `PlayerDropPoints`): one draw for the group member, one more when
    /// that member is a sub-group. Keeping tanks apart is the world's
    /// spawn clearance.
    pub fn spawn_point(&mut self, h: &mut dyn ServerHost, team_name: &str) -> String {
        let mut group_name = format!("{team_name}PlayerDropPoints");
        let mut group = h.group(&group_name);
        if group.is_none() {
            group_name = "PlayerDropPoints".into();
            group = h.group(&group_name);
        }
        let Some(group) = group else {
            h.error(&format!("Missing spawn points group {group_name}"));
            return "0 0 300 1 0 0 0".into();
        };
        if group.is_empty() {
            h.error(&format!("No spawn points found in {group_name}"));
            return "0 0 300 1 0 0 0".into();
        }
        let index = self.rng.get_random_max(group.len() as i32 - 1) as usize;
        let spawn = group.get(index).unwrap_or(&group[0]).clone();
        match spawn {
            GroupMember::Group { name, members } => {
                if members.is_empty() {
                    h.error(&format!("No spawn points found in {name}"));
                    // A group has no transform.
                    return String::new();
                }
                let index = self.rng.get_random_max(members.len() as i32 - 1) as usize;
                members[index.min(members.len() - 1)].clone()
            }
            GroupMember::Object(t) => t,
        }
    }

    /// The scrum goal (the object named `Goal`, created if there is none)
    /// at a random spawn point.
    pub fn place_goal(&mut self, h: &mut dyn ServerHost) -> Option<EntityId> {
        if h.find_entity("Goal").is_none() {
            if let Some(g) = h.data().pickups.find("DefaultGoal").and_then(|d| h.new_power_up("Goal", d)) {
                self.cleanup_add(g);
            }
        }
        let p = self.spawn_point(h, "");
        let goal = h.find_entity("Goal");
        if let Some(g) = goal {
            h.set_transform(g, &p);
        }
        goal
    }

    /// The client's tank is replaced at once.
    pub(crate) fn respawn_now(&mut self, h: &mut dyn ServerHost, client: Option<ClientId>) {
        let Some(c) = client.filter(|c| self.clients.contains_key(c)) else { return };
        self.clients.get_mut(&c).unwrap().player = None;
        self.spawn_player(h, c);
    }
}

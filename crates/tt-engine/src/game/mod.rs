//! Gameplay simulation (crate tt-game) and the game's entities.
//!
//! When a `MissionGroup` appears, a [`tt_game::World`] is built from the mission's
//! objects as its RON file declares them ([`MissionLoad`]: terrain, mission area,
//! prop obstacles, power-ups, boost pads, volcanoes, named markers) and from the
//! defs the data files declared, merged natively ([`EngineHost::defs`](crate::host::EngineHost::defs)). Tanks,
//! balls and triggers are world entities in an [`EntityTable`] (name, def, team,
//! owning client).
//!
//! The game server (`tt_gameplay::server`, through
//! [`crate::gameplay::server_host`]) creates and removes the entities and
//! gets the game events after each authority tick; a connection's control
//! object is its tank's entity id.

mod aim;
mod camera;
mod cosmetic;
mod entities;
mod render;
mod tick;
mod transform;

pub use aim::{aim_mode, predict_aim, AimMode, FreeAim, PlayerAim, TurretTarget};
pub use camera::{free_eye_height, free_view, heading_of, wrap_pi, CameraParams, ChaseCam, DEFAULT_HALF_VFOV, FREE_AIM_RANGE, FREE_ARM_MIN_ELEVATION, FREE_CAM_CLEARANCE, FREE_PITCH_RANGE, FREE_TANK_MARGIN};
pub use render::pose_tank;
pub use tick::{native_defs, tick, update_world};
pub use transform::{fields_transform, object_transform, parse_transform, rotation_rad, set_transform_fields, tank_fields, tank_script_transform, team_index, transform_string};

use glam::{Mat3, Quat, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tt_assets::mission::SceneObject;
use tt_game::fields::Defs;
use tt_game::{FlagId, ProjectileParams, StaticShape, TankParams, TankId, World};
use tt_gameplay::{EntityId, EntityTable};
use transform::floats;

/// The objects of a mission as loaded: the load's id (a new one per
/// mission load) and every object of its `MissionGroup` tree (depth first,
/// the group before its members) with its number in the load and its
/// declaration (fields as declared; no children).
#[derive(Debug, Clone, Default)]
pub struct MissionLoad {
    pub group: u32,
    pub nodes: Vec<(u32, SceneObject)>,
    /// The tree as built (groups with their members).
    pub tree: Option<MissionNode>,
}

impl MissionLoad {
    /// From a mission tree of declarations (see [`crate::mission::exec_mission`]).
    pub fn from_tree(group: u32, tree: &MissionNode) -> MissionLoad {
        fn flatten(n: &MissionNode, out: &mut Vec<(u32, SceneObject)>) {
            out.push((n.id, n.object.clone()));
            for c in &n.children {
                flatten(c, out);
            }
        }
        let mut nodes = Vec::new();
        flatten(tree, &mut nodes);
        MissionLoad { group, nodes, tree: Some(tree.clone()) }
    }

    /// A group of the mission by name (`nameToID`), depth first.
    pub fn group_named(&self, name: &str) -> Option<&MissionNode> {
        fn find<'a>(n: &'a MissionNode, name: &str) -> Option<&'a MissionNode> {
            if !n.object.name.is_empty() && n.object.name.eq_ignore_ascii_case(name) {
                return Some(n);
            }
            n.children.iter().find_map(|c| find(c, name))
        }
        find(self.tree.as_ref()?, name)
    }
}

/// One object of a loaded mission tree: its number, its declaration and
/// its members.
#[derive(Debug, Clone)]
pub struct MissionNode {
    pub id: u32,
    pub object: SceneObject,
    pub children: Vec<MissionNode>,
}

impl MissionNode {
    /// The node of object `name` (case-insensitive), depth first.
    pub fn find_named_mut(&mut self, name: &str) -> Option<&mut MissionNode> {
        if !self.object.name.is_empty() && self.object.name.eq_ignore_ascii_case(name) {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_named_mut(name))
    }
}

pub struct GameSim {
    pub world: World,
    /// Render-only pose corrections (network prediction smoothing):
    /// position offset and rotation added to the drawn pose. Empty unless
    /// this is a network client.
    pub render_offset: HashMap<TankId, (Vec3, Quat)>,
    /// Which parts of a tick run in this process. Set with [`GameSim::set_role`];
    /// `Local` unless networking says otherwise.
    pub role: Role,
    /// The mission load the world was built from ([`MissionLoad::group`]).
    pub mission_group: u32,
    pub db: Defs,
    pub root: PathBuf,
    pub mission_dir: PathBuf,
    tank_data: HashMap<String, Arc<TankParams>>,
    projectile_data: HashMap<String, Option<Arc<ProjectileParams>>>,
    /// The world's tanks, balls and triggers as game entities.
    pub entities: EntityTable,
    /// Power-ups' `position`/`rotation` fields as the scripts wrote them
    /// (what `getTransform()` reads back).
    pub script_xf: HashMap<EntityId, (String, String)>,
    /// At the last move.
    last_trigger_count: i32,
    pub cameras: HashMap<TankId, ChaseCam>,
    /// Sim time of each tank's last shot (muzzle flash / `fire` sequence).
    pub last_fired: HashMap<TankId, i64>,
    /// Player tank's damage and ammo at the previous tick (pickup hints).
    pub prev_player: Option<(TankId, f32, i32)>,
    pub ammo_warning: bool,
    /// Explosions to show (explosion def, position), drained by the renderer glue.
    pub explosions: Vec<(String, Vec3)>,
    /// Tank poses before the last world tick (render interpolation).
    pub prev_pose: HashMap<TankId, (Vec3, Mat3)>,
    /// Turret aim (`yaw_n`, `pitch_n`) before the last world tick (render
    /// interpolation).
    pub prev_aim: HashMap<TankId, (f32, f32)>,
    /// Flag (scrum ball) positions before the last world tick.
    pub prev_flags: HashMap<FlagId, Vec3>,
    /// Last collision sound per (tank, kind) in sim ms (`*_collision_interval_ms`).
    pub last_coll_sound: HashMap<(TankId, u8), i64>,
    /// Looping engine sounds per tank (TankFxParams engine_sound_a/B).
    pub engine_sounds: HashMap<TankId, (u32, u32)>,
    /// The local viewer's reticle smoothing state.
    pub reticle_state: tt_game::ReticleState,
    /// Power-ups' `activate` thread: sim time of the last on/off switch and
    /// the new state. Switching on plays the sequence forwards (the pick-up
    /// grows in), off backwards.
    pub activations: HashMap<tt_game::TriggerId, (i64, bool)>,
    /// Triggers that existed at the end of the previous tick: a switch of a
    /// newer one is its initial state, not animated (the first replica
    /// update skips the thread).
    pub known_triggers: std::collections::HashSet<tt_game::TriggerId>,
    /// Shapes of ejected brain jars by projectile (drawn as flying shapes).
    pub brain_shapes: HashMap<tt_game::ProjectileId, String>,
    /// Tread and wheel animation per tank ([`crate::treads`]).
    pub treads: HashMap<TankId, crate::treads::Treads>,
    /// Events seen, for reports: kind -> count.
    pub event_counts: HashMap<&'static str, usize>,
    /// The local player's mouse aim between ticks (render-only prediction,
    /// see [`GameSim::render_aim`]).
    pub player_aim: PlayerAim,
    /// Camera aim's free camera.
    pub free_aim: FreeAim,
}

/// Which parts of the game a process runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// Single player and the listen server (the host plays in the same
    /// process over the local connection): authority and presentation.
    #[default]
    Local,
    /// Headless dedicated server: authority only (no sounds, effects,
    /// treads or HUD hints).
    Dedicated,
    /// Remote client: presentation only. The world runs with
    /// `server = false`; outcomes (script callbacks, removals) come from the server.
    Client,
}

impl Role {
    /// Decides outcomes: server-only world parts and script callbacks.
    pub fn authority(self) -> bool {
        !matches!(self, Role::Client)
    }

    /// Shows the game: sounds, explosions, muzzle flashes, treads, hints.
    pub fn cosmetic(self) -> bool {
        !matches!(self, Role::Dedicated)
    }
}

impl GameSim {
    /// Builds the world of a mission from its objects ([`MissionLoad`])
    /// and defs; reads no script objects. Power-ups, pads and
    /// volcanoes become trigger entities (bound to the script objects the
    /// nodes name). Tanks and balls are added later ([`Self::add_tank`],
    /// [`Self::add_flag`]).
    pub fn build(root: &Path, mission_dir: &Path, mission: &MissionLoad, db: Defs, first_id: u32) -> Result<GameSim, String> {
        let objects = || mission.nodes.iter().map(|(_, o)| o);
        let class_of = |o: &SceneObject| o.class.to_ascii_lowercase();
        let field = |o: &SceneObject, f: &str| o.get(f).unwrap_or("").to_string();

        let terrain = match objects().find(|o| class_of(o) == "terrain") {
            Some(tb) => {
                let square = tt_game::fields::atof(&field(tb, "square_size"));
                let square = if square > 0.0 { square } else { 8.0 };
                let file = field(tb, "terrain_file");
                let p = tt_assets::find_ci(&tt_assets::resolve(root, mission_dir, &file)).ok_or_else(|| format!("terrain {file} not found"))?;
                let t = tt_assets::terrain::Terrain::parse(&std::fs::read(&p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                Some(tt_game::HeightField::from_terrain(&t, square))
            }
            None => None,
        };
        let mut sim = GameSim {
            world: World::new(terrain),
            role: Role::Local,
            render_offset: HashMap::new(),
            mission_group: mission.group,
            db,
            root: root.to_path_buf(),
            mission_dir: mission_dir.to_path_buf(),
            tank_data: HashMap::new(),
            projectile_data: HashMap::new(),
            entities: EntityTable::starting_at(first_id),
            script_xf: HashMap::new(),
            last_trigger_count: 0,
            cameras: HashMap::new(),
            last_fired: HashMap::new(),
            prev_player: None,
            brain_shapes: HashMap::new(),
            activations: HashMap::new(),
            known_triggers: Default::default(),
            treads: HashMap::new(),
            reticle_state: tt_game::ReticleState::default(),
            last_coll_sound: HashMap::new(),
            engine_sounds: HashMap::new(),
            prev_pose: HashMap::new(),
            prev_aim: HashMap::new(),
            prev_flags: HashMap::new(),
            explosions: Vec::new(),
            ammo_warning: false,
            event_counts: HashMap::new(),
            player_aim: PlayerAim::default(),
            free_aim: FreeAim { half_vfov: DEFAULT_HALF_VFOV, ..FreeAim::default() },
        };
        if let Some(ma) = objects().find(|o| class_of(o) == "missionarea") {
            if let [x, y, w, h] = floats(&field(ma, "area"))[..] {
                sim.world.mission_area = Some(tt_game::MissionArea { x, y, w, h });
            }
            if let Some(f) = ma.get("floor") {
                sim.world.floor = tt_game::fields::atof(f);
            }
        }
        // The mission's gravity scale (`MissionInfo.gravity`, 1 when
        // unset): server and clients load the same mission file.
        if let Some(g) = objects().find(|o| class_of(o) == "info").map(|o| tt_game::fields::atof(&field(o, "gravity"))).filter(|g| *g > 0.0) {
            sim.world.gravity = g;
        }
        let mut static_cache: HashMap<String, Option<Arc<StaticShape>>> = HashMap::new();
        for (_, o) in &mission.nodes {
            let class = class_of(o);
            let xf = object_transform(o);
            let name = &o.name;
            match class.as_str() {
                "prop" => {
                    let path = field(o, "shape");
                    if path.is_empty() {
                        continue;
                    }
                    let shape = static_cache
                        .entry(path.to_ascii_lowercase())
                        .or_insert_with(|| tt_game::mission::load_shape(root, mission_dir, &path).map(|s| Arc::new(StaticShape::from_shape(&s))))
                        .clone();
                    if let Some(s) = shape {
                        let i = sim.world.add_static(s, xf);
                        if !name.is_empty() {
                            let b = sim.world.statics[i].world_box;
                            sim.world.add_marker(name, b);
                        }
                    }
                }
                "spawnpoint" => {
                    sim.world.spawn_spheres.push(xf.translation.into());
                    if !name.is_empty() {
                        let p: Vec3 = xf.translation.into();
                        sim.world.markers.entry(name.to_ascii_lowercase()).or_insert(tt_game::Aabb::new(p, p));
                    }
                }
                "powerup" | "boost" | "volcano" => {
                    sim.add_trigger(o);
                }
                _ => {
                    if !name.is_empty() && !field(o, "position").is_empty() {
                        let p: Vec3 = xf.translation.into();
                        sim.world.markers.entry(name.to_ascii_lowercase()).or_insert(tt_game::Aabb::new(p, p));
                    }
                }
            }
        }
        sim.world.auto_eject_brains = true;
        Ok(sim)
    }

    /// Switches the role; the world's server-only and client-cosmetic
    /// parts follow it (`Local` = the defaults `build` sets).
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
        self.world.server = role.authority();
        // A client predicts its tank's boost/jump pad kicks (stage 6).
        self.world.predict_pads = role == Role::Client;
        self.world.predict_shots = role == Role::Client;
        // Brain jars stay on: a dedicated server ejects them so the jar
        // projectiles replicate (a client's puppets never eject their own).
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_split_authority_and_presentation() {
        assert_eq!(Role::default(), Role::Local);
        assert!(Role::Local.authority() && Role::Local.cosmetic());
        assert!(Role::Dedicated.authority() && !Role::Dedicated.cosmetic());
        assert!(!Role::Client.authority() && Role::Client.cosmetic());
    }
}

//! Gameplay simulation (crate tt-game) and the game's entities.
//!
//! When a `MissionGroup` appears, a [`tt_game::World`] is built from the
//! mission's objects as its RON file declares them ([`MissionLoad`]:
//! terrain, mission area, prop obstacles, power-ups, boost pads, volcanoes,
//! named markers) and from the defs the data files declared, merged natively
//! ([`EngineHost::defs`]). Tanks, balls and triggers are world entities in
//! an [`EntityTable`] (name, def, team, owning client).
//!
//! The game server (`tt_gameplay::server`, through
//! [`crate::gameplay::server_host`]) creates and removes the entities and
//! gets the game events after each authority tick; a connection's control
//! object is its tank's entity id.

use crate::host::EngineHost;
use glam::{Affine3A, Mat3, Quat, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tt_assets::mission::SceneObject;
use tt_game::fields::Defs;
use tt_game::{Event, FlagId, HitObject, ProjectileParams, StaticShape, TankParams, TankId, TriggerShape, World};
use tt_gameplay::server::combat::KdaEvent;
use tt_gameplay::{EntityId, EntityKind, EntityTable, GameEvent};
use tt_game::fmt::format_g;

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
    /// Which parts of a tick run in this process (docs/netcode.md §2.7).
    /// Set with [`GameSim::set_role`]; `Local` unless networking says otherwise.
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

/// Which parts of the game a process runs (docs/netcode.md §2.1, §2.7).
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

/// Camera aim's free camera: client-only view state turned by the mouse. The
/// player's turret is steered towards where it looks through the normal
/// moves.
#[derive(Debug, Clone, Copy, Default)]
pub struct FreeAim {
    /// The player's tank while camera aim is on (None: classic, or dead).
    pub tank: Option<TankId>,
    /// World heading of the view (radians, + = clockwise from above).
    pub yaw: f32,
    pub pitch: f32,
    /// Half the vertical field of view of the game view (radians), set by
    /// the frontend each frame.
    pub half_vfov: f32,
}

/// Where the free camera wants the turret (see [`GameSim::turret_target`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurretTarget {
    /// The aim point: under the screen centre, or [`FREE_AIM_RANGE`] along the view.
    pub point: Vec3,
    /// Turret yaw/pitch (`yawN`/`pitchN`) within the turret's range.
    pub yaw_n: f32,
    pub pitch_n: f32,
    /// -1/+1 when the aim point is beyond the turret's yaw range to the
    /// left/right (the hull should turn), else 0.
    pub beyond: f32,
}

/// Drawn aim of the local player from the tick-interpolated aim `lerped`:
/// the mouse part of the last tick's change (`consumed`) is shown in full,
/// not interpolated, plus the mouse motion since (`pending`); all in
/// `yawN`/`pitchN` units, clamped to the turret range.
pub fn predict_aim(lerped: (f32, f32), consumed: (f32, f32), pending: (f32, f32), alpha: f32) -> (f32, f32) {
    let k = 1.0 - alpha.clamp(0.0, 1.0);
    ((lerped.0 + k * consumed.0 + pending.0).clamp(-1.0, 1.0), (lerped.1 + k * consumed.1 + pending.1).clamp(-1.0, 1.0))
}

/// Mouse aim of the local player for rendering. Drawing the turret from
/// tick states alone makes it step with the uneven number of mouse events
/// per tick. Render-only: the simulation never reads this.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerAim {
    /// The player's tank while its moves reach the sim.
    pub tank: Option<TankId>,
    /// Mouse yaw/pitch (`yawN`/`pitchN` units, i.e. radians / π) the last
    /// tick consumed, as the server quantises it.
    pub consumed: (f32, f32),
    /// Mouse yaw/pitch (same units) accumulated since that tick.
    pub pending: (f32, f32),
}

fn floats(s: &str) -> Vec<f32> {
    s.split_whitespace().map(tt_game::fields::atof).collect()
}

/// A mission object's transform from its `position`/`rotation`/`scale` fields.
pub fn object_transform(o: &SceneObject) -> Affine3A {
    let f = |n: &str| floats(o.get(n).unwrap_or(""));
    tt_game::geom::mission_transform(&f("position"), &f("rotation"), &f("scale"))
}

pub fn transform_string(pos: Vec3, rot: Mat3) -> String {
    let q = Quat::from_mat3(&rot).normalize();
    let (axis, angle) = q.to_axis_angle();
    let (axis, angle) = if angle.abs() < 1e-6 { (Vec3::Z, 0.0) } else { (axis, -angle) };
    [pos.x, pos.y, pos.z, axis.x, axis.y, axis.z, angle].iter().map(|v| format_g(*v as f64)).collect::<Vec<_>>().join(" ")
}

pub fn parse_transform(s: &str) -> (Vec3, Option<Mat3>) {
    let v = floats(s);
    let pos = Vec3::new(v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0));
    let rot = (v.len() >= 7).then(|| {
        let axis = Vec3::new(v[3], v[4], v[5]);
        if axis.length_squared() < 1e-12 {
            Mat3::IDENTITY
        } else {
            Mat3::from_quat(Quat::from_axis_angle(axis.normalize(), -v[6]))
        }
    });
    (pos, rot)
}

/// `obj.setTransform(t)` on an object's `position` / `rotation` fields
/// (the angle stored in degrees).
pub fn set_transform_fields(f: &mut (String, String), t: &str) {
    let v: Vec<f64> = t.split_whitespace().map(tt_game::fmt::atof).collect();
    let fmt = |v: &[f64]| v.iter().map(|x| format_g(*x)).collect::<Vec<_>>().join(" ");
    if v.len() >= 3 {
        f.0 = fmt(&v[..3]);
    }
    if v.len() >= 7 {
        f.1 = fmt(&[v[3], v[4], v[5], v[6].to_degrees()]);
    }
}

/// `getTransform()` of an object with these `position` / `rotation`
/// fields (radians again).
pub fn fields_transform(position: &str, rotation: &str) -> String {
    let p: Vec<f64> = position.split_whitespace().map(tt_game::fmt::atof).collect();
    let r: Vec<f64> = rotation.split_whitespace().map(tt_game::fmt::atof).collect();
    let p = [p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0), p.get(2).copied().unwrap_or(0.0)];
    let (axis, deg) = match r.as_slice() {
        [x, y, z, a, ..] if x * x + y * y + z * z > 0.0 => ([*x, *y, *z], *a),
        _ => ([1.0, 0.0, 0.0], 0.0),
    };
    [p[0], p[1], p[2], axis[0], axis[1], axis[2], deg.to_radians()].iter().map(|x| format_g(*x)).collect::<Vec<_>>().join(" ")
}

/// A tank's `position` / `rotation` fields as the tick writes them from
/// its pose.
pub fn tank_fields(pos: Vec3, rot: Mat3) -> (String, String) {
    let s = transform_string(pos, rot);
    let mut parts = s.splitn(4, ' ');
    let p = format!("{} {} {}", parts.next().unwrap_or("0"), parts.next().unwrap_or("0"), parts.next().unwrap_or("0"));
    let r: Vec<f64> = parts.next().unwrap_or("0 0 1 0").split(' ').map(tt_game::fmt::atof).collect();
    let r = if r.len() == 4 { format!("{} {} {} {}", format_g(r[0]), format_g(r[1]), format_g(r[2]), format_g(r[3].to_degrees())) } else { "1 0 0 0".into() };
    (p, r)
}

/// A tank's `getTransform()` (its synced fields read back).
pub fn tank_script_transform(pos: Vec3, rot: Mat3) -> String {
    let (p, r) = tank_fields(pos, rot);
    fields_transform(&p, &r)
}

/// The player's move from the client's move state; resets the yaw/pitch/roll
/// as the engine does. The quantisation of docs/engine-notes.md §7 is applied
/// by the world (`World::quantize_moves`).
///
/// For the local player's move (`MoveState::take_move`). DEVIATION: when set,
/// the turn input is negated while the throttle is reversed, so A/D steer like
/// a car backing up. Only the local player's move is touched — bots and the
/// simulation are unchanged.
fn reverse_steering(prefs: &tt_shell::Prefs) -> bool {
    let v = prefs.get("input.reverse_steering");
    let v = v.trim();
    // Unset: off, as the settings default (tt_shell::Prefs::reverse_steering).
    !v.is_empty() && (v.eq_ignore_ascii_case("true") || tt_game::fmt::atof(v) != 0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimMode {
    Classic,
    /// 1: free camera: the mouse turns the camera, the turret follows it
    /// within its limits and the hull turns towards it beyond them.
    Camera,
}

pub fn aim_mode(prefs: &tt_shell::Prefs) -> AimMode {
    let v = prefs.get("input.aim_mode");
    if !v.is_empty() && tt_game::fmt::atoi(&v) == 0 {
        AimMode::Classic
    } else {
        AimMode::Camera
    }
}

/// How far up the view really goes is also limited by keeping the tank on
/// screen (see [`free_view`]).
pub const FREE_PITCH_RANGE: (f32, f32) = (-0.6, 1.45);

/// Free camera: the lowest elevation of the eye seen from the pivot above
/// the tank (radians); looking further up turns only the view.
pub const FREE_ARM_MIN_ELEVATION: f32 = 0.0;

/// Free camera: how far inside the bottom edge of the view the pivot above
/// the tank is kept (radians), so the tank stays visible.
pub const FREE_TANK_MARGIN: f32 = 0.03;

/// Half the vertical field of view assumed until a frame tells the real one
/// (90° horizontal at 16:9).
pub const DEFAULT_HALF_VFOV: f32 = 0.513;

/// Clearance of the free camera's eye over the terrain, metres.
pub const FREE_CAM_CLEARANCE: f32 = 1.0;

/// How far the free camera's aim ray looks for the point under the screen
/// centre, metres (beyond it the aim point is on the ray at that distance).
pub const FREE_AIM_RANGE: f32 = 500.0;

/// Heading of a direction (radians, + = clockwise from above; 0 = +y).
pub fn heading_of(v: Vec3) -> f32 {
    v.x.atan2(v.y)
}

/// An angle wrapped to -π..π.
pub fn wrap_pi(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// `TankCameraDef` (docs/engine-notes.md §2.1).
#[derive(Debug, Clone, Copy)]
pub struct CameraParams {
    pub focus_dist: f32,
    pub focus_up_dist: f32,
    pub backup_dist: f32,
    pub float_height: f32,
    pub smooth: f32,
    pub smooth_turn: f32,
    pub smooth_up: f32,
    pub fov: f32,
    /// The camera never goes below the mission floor.
    pub floor: f32,
}

impl Default for CameraParams {
    fn default() -> Self {
        CameraParams { focus_dist: 10.1, focus_up_dist: 0.9, backup_dist: 6.2, float_height: 6.22, smooth: 0.33, smooth_turn: 0.77, smooth_up: 0.75, fov: 90.0, floor: -100.0 }
    }
}

/// Smoothed chase-camera state kept per tank.
#[derive(Debug, Clone, Default)]
pub struct ChaseCam {
    prev: Option<(Vec3, Vec3, Vec3, f64)>,
    /// Free camera (camera aim): smoothed eye height over the tank and its time.
    free_z: Option<(f32, f64)>,
}

impl ChaseCam {
    /// Returns the camera transform (columns right, look, up; translation
    /// = eye). `now_ms` is real time (fractional: at hundreds of frames
    /// per second whole milliseconds would make the smoothing step
    /// unevenly).
    pub fn update(&mut self, now_ms: f64, pos: Vec3, rot: Mat3, cd: &CameraParams, ground: impl Fn(f32, f32) -> Option<f32>) -> Affine3A {
        self.free_z = None;
        let (mut r, mut u) = (rot.x_axis, rot.z_axis);
        let (snap, t, prev_cam) = match self.prev {
            Some((pr, pu, pc, last)) if now_ms - last <= 100.0 => {
                let t = ((now_ms - last).max(0.0) / 100.0) as f32;
                let f = cd.smooth_turn.clamp(0.1, 1.0).powf(t);
                r = r.lerp(pr, f).try_normalize().unwrap_or(r);
                let f = cd.smooth_up.clamp(0.1, 1.0).powf(t);
                u = u.lerp(pu, f).try_normalize().unwrap_or(u);
                (false, t, Some(pc))
            }
            _ => (true, 0.0, None),
        };
        let fwd = if snap { rot.y_axis } else { u.cross(r).try_normalize().unwrap_or(rot.y_axis) };
        let focus = pos + fwd * cd.focus_dist + Vec3::Z * cd.focus_up_dist;
        let mut cam = pos - fwd * cd.backup_dist;
        let gz = ground(cam.x, cam.y).unwrap_or(f32::MIN);
        cam.z = cam.z.max(gz) + cd.float_height;
        cam.z = cam.z.max(cd.floor);
        if let Some(pc) = prev_cam.filter(|pc| (cam - *pc).length() <= 50.0) {
            let f = cd.smooth.clamp(0.1, 1.0).powf(t);
            cam = cam.lerp(pc, f);
        }
        self.prev = Some((r, u, cam, now_ms));
        let look = (focus - cam).try_normalize().unwrap_or(fwd);
        let right = Vec3::new(look.y, -look.x, 0.0).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(look);
        Affine3A::from_cols(right.into(), look.into(), up.into(), cam.into())
    }

    /// Camera aim: the free camera `aim`, see [`free_view`]. Yaw and pitch
    /// are the mouse's, unsmoothed.
    pub fn free(&mut self, now_ms: f64, pos: Vec3, aim: &FreeAim, cd: &CameraParams, ground: impl Fn(f32, f32) -> Option<f32>) -> (Affine3A, f32) {
        let (yaw, pitch, half_vfov) = (aim.yaw, aim.pitch, aim.half_vfov);
        let mut h = free_eye_height(pos, yaw, cd, &ground);
        if let Some((prev, last)) = self.free_z.filter(|&(prev, last)| now_ms - last <= 100.0 && (prev - h).abs() <= 50.0) {
            let t = ((now_ms - last).max(0.0) / 100.0) as f32;
            h += (prev - h) * cd.smooth.clamp(0.1, 1.0).powf(t);
        }
        self.free_z = Some((h, now_ms));
        // The hull camera starts over (snapped) when camera aim is left.
        self.prev = None;
        free_view(pos, yaw, pitch, h, half_vfov, cd, ground)
    }
}

/// Free camera eye height over the tank before smoothing: the game's
/// `cam_float_height` over the ground (or the tank) behind it.
pub fn free_eye_height(pos: Vec3, yaw: f32, cd: &CameraParams, ground: impl Fn(f32, f32) -> Option<f32>) -> f32 {
    let fwd = Vec3::new(yaw.sin(), yaw.cos(), 0.0);
    let cam = pos - fwd * cd.backup_dist;
    let gz = ground(cam.x, cam.y).unwrap_or(f32::MIN);
    (cam.z.max(gz) + cd.float_height).max(cd.floor) - pos.z
}

/// Camera aim's free camera. Returns the view and the pitch it shows
/// (`pitch` less what the tank-visibility limit took off).
///
/// `pitch` (+ = up) turns the whole rig (eye and look) about a pivot
/// `cam_focus_up_dist` above the tank, around the view's right axis: looking
/// down raises the eye, looking up lowers it behind the tank, until the eye
/// is level with the pivot ([`FREE_ARM_MIN_ELEVATION`]); further up only the
/// view turns and the tank drops lower in the frame. The eye is kept
/// [`FREE_CAM_CLEARANCE`] over the terrain (lifted), and the view is tilted
/// back down where the pivot would come closer than [`FREE_TANK_MARGIN`] to
/// the bottom edge (`half_vfov` = half the vertical field of view), so the
/// tank never leaves the screen.
pub fn free_view(pos: Vec3, yaw: f32, pitch: f32, eye_height: f32, half_vfov: f32, cd: &CameraParams, ground: impl Fn(f32, f32) -> Option<f32>) -> (Affine3A, f32) {
    let fwd = Vec3::new(yaw.sin(), yaw.cos(), 0.0);
    let right = Vec3::new(fwd.y, -fwd.x, 0.0);
    let elev = |v: Vec3| v.normalize_or_zero().z.clamp(-1.0, 1.0).asin();
    let focus = pos + fwd * cd.focus_dist + Vec3::Z * cd.focus_up_dist;
    let cam0 = pos - fwd * cd.backup_dist + Vec3::Z * eye_height;
    let pivot = pos + Vec3::Z * cd.focus_up_dist;
    let orbit = pitch.min((elev(cam0 - pivot) - FREE_ARM_MIN_ELEVATION).max(0.0));
    let q = Quat::from_axis_angle(right, orbit);
    let mut cam = pivot + q * (cam0 - pivot);
    let mut look = Quat::from_axis_angle(right, pitch) * (focus - cam0).try_normalize().unwrap_or(fwd);
    if let Some(g) = ground(cam.x, cam.y) {
        cam.z = cam.z.max(g + FREE_CAM_CLEARANCE);
    }
    // The pivot's angle below the view centre.
    let below = elev(look) - elev(pivot - cam);
    let excess = below - (half_vfov - FREE_TANK_MARGIN).max(0.1);
    let mut shown = pitch;
    // Only looking up is limited.
    let excess = excess.min(pitch);
    if excess > 0.0 {
        look = Quat::from_axis_angle(right, -excess) * look;
        shown -= excess;
    }
    let up = right.cross(look);
    (Affine3A::from_cols(right.into(), look.into(), up.into(), cam.into()), shown)
}

impl GameSim {
    /// Time into the power-up's `activate` sequence (`dur` s long) at sim
    /// time `now_s`, while the thread runs: forwards after switching on,
    /// backwards after switching off. None once it is done or when it
    /// never switched.
    pub fn activation_time(&self, id: tt_game::TriggerId, now_s: f32, dur: f32) -> Option<f32> {
        let &(at, on) = self.activations.get(&id)?;
        let since = now_s - at as f32 / 1000.0;
        (since >= 0.0 && since < dur).then_some(if on { since } else { dur - since })
    }

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

    fn trigger_shape(&self, shape_path: &str) -> TriggerShape {
        match tt_game::mission::load_shape(&self.root, &self.mission_dir, shape_path) {
            Some(s) => TriggerShape::from_shape(&s),
            None => TriggerShape::default(),
        }
    }

    /// A def's name as declared (the scripts may write it in any case;
    /// `setAI(name)`).
    pub fn declared(&self, name: &str) -> String {
        self.db.object(name).map(|o| o.name.clone()).unwrap_or_else(|| name.to_string())
    }

    /// Adds a power-up, pad or volcano entity of the mission object `o`
    /// (its class, name, `def` and transform).
    pub fn add_trigger(&mut self, o: &SceneObject) -> Option<EntityId> {
        let dbn = self.declared(o.get("def").unwrap_or(""));
        let fields = self.db.fields(&dbn)?;
        let shape = self.trigger_shape(fields.get("shape").unwrap_or(""));
        let xf = object_transform(o);
        let name = &o.name;
        let t = match o.class.to_ascii_lowercase().as_str() {
            "boost" => self.world.add_boost(name, Arc::new(self.db.boost(&dbn)?), shape, xf),
            "volcano" => {
                let mut v = self.db.volcano(&dbn)?;
                if let Some(p) = &v.projectile {
                    if p.is_dts() {
                        let mut p2 = (**p).clone();
                        if let Some(s) = tt_game::mission::load_shape(&self.root, &self.mission_dir, &p.resource) {
                            p2.set_shape(&s);
                        }
                        v.projectile = Some(Arc::new(p2));
                    }
                }
                self.world.add_volcano(name, Arc::new(v), shape, xf)
            }
            _ => self.world.add_power_up(name, Arc::new(tt_game::PowerUpParams::from_fields(&fields)), shape, xf),
        };
        let e = self.entities.insert(EntityKind::Trigger(t), name, &dbn);
        // `getTransform()` reads the object's fields as the scripts set them.
        self.script_xf.insert(e, (o.get("position").unwrap_or("").to_string(), o.get("rotation").unwrap_or("").to_string()));
        Some(e)
    }

    /// Sim data of a tank def (shape geometry loaded), cached per mission.
    pub fn tank_data(&mut self, name: &str) -> Option<Arc<TankParams>> {
        let key = name.to_ascii_lowercase();
        if let Some(d) = self.tank_data.get(&key) {
            return Some(d.clone());
        }
        let mut d = self.db.tank(name)?;
        if let Some(s) = tt_game::mission::load_shape(&self.root, &self.mission_dir, &d.shape_file) {
            d.set_shape(&s);
        }
        let d = Arc::new(d);
        self.tank_data.insert(key, d.clone());
        Some(d)
    }

    pub fn projectile(&mut self, name: &str) -> Option<Arc<ProjectileParams>> {
        let key = name.to_ascii_lowercase();
        if let Some(p) = self.projectile_data.get(&key) {
            return p.clone();
        }
        let p = self.db.projectile(name).map(|mut p| {
            if p.is_dts() {
                if let Some(s) = tt_game::mission::load_shape(&self.root, &self.mission_dir, &p.resource) {
                    p.set_shape(&s);
                }
            }
            Arc::new(p)
        });
        self.projectile_data.insert(key, p.clone());
        p
    }

    /// The world tank of an entity (a control object, a client's player).
    pub fn tank_of(&self, e: EntityId) -> Option<TankId> {
        self.entities.tank(e)
    }

    /// The entity of a world tank.
    pub fn entity_of_tank(&self, t: TankId) -> Option<EntityId> {
        self.entities.of_tank(t).map(|e| e.id)
    }

    /// Adds a tank entity of def `def` at `pos`/`rot` with
    /// entity id `id` (None: a new id); None if there is no such tank def.
    pub fn add_tank(&mut self, def: &str, name: &str, pos: Vec3, rot: Mat3, id: Option<EntityId>) -> Option<EntityId> {
        let data = self.tank_data(def)?;
        let t = self.world.add_tank_at(data, pos, rot);
        let dbn = self.declared(def);
        Some(match id {
            Some(e) => {
                self.entities.insert_id(e, EntityKind::Tank(t), name, &dbn);
                e
            }
            None => self.entities.insert(EntityKind::Tank(t), name, &dbn),
        })
    }

    /// `Tank.setAI(TankAiParams)` by the AI def's name; empty = player control.
    pub fn set_ai(&mut self, id: TankId, ai: &str) {
        let data = (!ai.is_empty()).then(|| self.db.tank_ai(ai)).flatten().map(Arc::new);
        self.world.set_ai(id, data);
    }

    /// `Tank.setTeam`: the world team and the entity's.
    pub fn set_team(&mut self, id: TankId, team: i32) {
        self.world.set_team(id, team);
        if let Some(e) = self.entities.of_tank(id).map(|e| e.id).and_then(|e| self.entities.get_mut(e)) {
            e.team = Some(team);
        }
    }

    /// Adds a scrum ball entity of def `def` at `pos` (entity
    /// id `id`, None: a new one).
    pub fn add_flag(&mut self, def: &str, name: &str, pos: Vec3, id: Option<EntityId>) -> Option<EntityId> {
        let mut data = self.db.flag(def)?;
        if let Some(s) = tt_game::mission::load_shape(&self.root, &self.mission_dir, &data.shape_file.clone()) {
            data.set_shape(&s);
        }
        let f = self.world.add_flag(Arc::new(data), pos);
        let dbn = self.declared(def);
        Some(match id {
            Some(e) => {
                self.entities.insert_id(e, EntityKind::Flag(f), name, &dbn);
                e
            }
            None => self.entities.insert(EntityKind::Flag(f), name, &dbn),
        })
    }

    /// Removes an entity and its world part.
    pub fn remove_entity(&mut self, e: EntityId) {
        let Some(e) = self.entities.remove(e) else { return };
        self.script_xf.remove(&e.id);
        match e.kind {
            EntityKind::Tank(id) => {
                self.cameras.remove(&id);
                self.world.remove_tank(id);
            }
            EntityKind::Flag(id) => self.world.flags.retain(|f| f.id != id),
            EntityKind::Trigger(t) => self.world.triggers.retain(|x| x.id != t),
        }
    }

    /// `setTransform` on a tank ([`World::set_tank_transform`]), a ball or
    /// a power-up (whose `getTransform()` then reads it back as the fields
    /// were written).
    pub fn set_transform(&mut self, e: EntityId, s: &str) {
        let Some(kind) = self.entities.get(e).map(|e| e.kind) else { return };
        let (pos, rot) = parse_transform(s);
        match kind {
            EntityKind::Flag(f) => self.world.set_flag_position(f, pos),
            EntityKind::Trigger(t) => {
                if let Some(tr) = self.world.triggers.iter_mut().find(|x| x.id == t) {
                    let scale = Vec3::new(tr.transform.matrix3.x_axis.length(), tr.transform.matrix3.y_axis.length(), tr.transform.matrix3.z_axis.length());
                    let rot = rot.map(|r| Quat::from_mat3(&r)).unwrap_or_else(|| Quat::from_mat3(&Mat3::from(tr.transform.matrix3)));
                    tr.transform = Affine3A::from_scale_rotation_translation(scale, rot, pos);
                }
                let f = self.script_xf.entry(e).or_default();
                set_transform_fields(f, s);
            }
            EntityKind::Tank(id) => {
                self.world.set_tank_transform(id, pos, rot);
                self.cameras.remove(&id);
            }
        }
    }

    /// `getTransform()` as the scripts read it: a tank's pose as the tick
    /// synced it into its fields, a power-up's fields.
    pub fn script_transform(&self, e: EntityId) -> String {
        match self.entities.get(e).map(|e| e.kind) {
            Some(EntityKind::Tank(t)) => match self.world.tank(t) {
                Some(t) => tank_script_transform(t.pos, t.rot),
                None => String::new(),
            },
            Some(EntityKind::Flag(f)) => match self.world.flag(f) {
                Some(f) => fields_transform(&format!("{} {} {}", format_g(f.pos.x as f64), format_g(f.pos.y as f64), format_g(f.pos.z as f64)), ""),
                None => String::new(),
            },
            Some(EntityKind::Trigger(_)) => {
                let (p, r) = self.script_xf.get(&e).cloned().unwrap_or_default();
                fields_transform(&p, &r)
            }
            None => String::new(),
        }
    }

    /// `setDef(name)` on an entity: a tank takes the new tank data, a
    /// power-up its new type.
    pub fn set_def(&mut self, e: EntityId, name: &str) {
        let name = self.declared(name);
        let Some(ent) = self.entities.get_mut(e) else { return };
        ent.def = name.clone();
        match ent.kind {
            EntityKind::Tank(t) => {
                if let Some(d) = self.tank_data(&name) {
                    if let Some(tank) = self.world.tank_mut(t) {
                        tank.data = d;
                    }
                }
            }
            EntityKind::Trigger(id) => {
                let Some(fields) = self.db.fields(&name) else { return };
                if let Some(t) = self.world.triggers.iter_mut().find(|t| t.id == id) {
                    if let tt_game::TriggerKind::PowerUp(_) = t.kind {
                        t.kind = tt_game::TriggerKind::PowerUp(Arc::new(tt_game::PowerUpParams::from_fields(&fields)));
                    }
                }
            }
            EntityKind::Flag(_) => {}
        }
    }

    /// Fields of an entity's def.
    pub fn def_fields(&self, kind: EntityKind) -> Option<tt_game::fields::Fields> {
        self.db.fields(&self.entities.of_kind(kind)?.def)
    }

    /// One field of a tank's TankFxParams (`tank.def.fx.field`).
    pub fn fx_value(&self, t: TankId, field: &str) -> Option<&str> {
        let fx = self.db.field(&self.entities.of_kind(EntityKind::Tank(t))?.def, "fx")?;
        self.db.field(fx, field)
    }

    /// Camera def of a tank's def (`camera = DefaultTankCamera`).
    pub fn camera_data(&self, t: TankId) -> CameraParams {
        let cd = CameraParams { floor: self.world.floor, ..CameraParams::default() };
        let Some(cam) = self.def_fields(EntityKind::Tank(t)).and_then(|d| d.get("camera").map(str::to_string)).and_then(|c| self.db.fields(&c)) else { return cd };
        let f = |n: &str, d: f32| match cam.get(n) {
            Some(v) if !v.is_empty() => tt_game::fields::atof(v),
            _ => d,
        };
        CameraParams {
            focus_dist: f("cam_focus_dist", cd.focus_dist),
            focus_up_dist: f("cam_focus_up_dist", cd.focus_up_dist),
            backup_dist: f("cam_backup_dist", cd.backup_dist),
            float_height: f("cam_float_height", cd.float_height),
            smooth: f("cam_smooth", cd.smooth),
            smooth_turn: f("cam_smooth_turn", cd.smooth_turn),
            smooth_up: f("cam_smooth_up", cd.smooth_up),
            fov: f("cam_fov", cd.fov),
            floor: cd.floor,
        }
    }

    /// Pose of a tank for rendering: interpolated between the previous and
    /// the current tick by `alpha`.
    pub fn render_pose(&self, t: &tt_game::Tank, alpha: f32) -> (Vec3, Mat3) {
        let (p, r) = self.render_pose_raw(t, alpha);
        match self.render_offset.get(&t.id) {
            Some((dp, dq)) => (p + *dp, Mat3::from_quat(*dq * Quat::from_mat3(&r))),
            None => (p, r),
        }
    }

    fn render_pose_raw(&self, t: &tt_game::Tank, alpha: f32) -> (Vec3, Mat3) {
        match self.prev_pose.get(&t.id) {
            // A teleport (respawn, setTransform) is not interpolated.
            Some(&(p, r)) if (t.pos - p).length_squared() < 100.0 => {
                let q = Quat::from_mat3(&r).slerp(Quat::from_mat3(&t.rot), alpha);
                (p.lerp(t.pos, alpha), Mat3::from_quat(q))
            }
            _ => (t.pos, t.rot),
        }
    }

    /// Turret aim (`yaw_n`, `pitch_n`) of a tank for rendering, interpolated
    /// between the previous and the current tick like [`Self::render_pose`].
    ///
    /// For the local player the mouse motion is drawn as it arrives: the
    /// part of the last tick's change that came from the mouse is shown in
    /// full at once (it was already on screen as pending input before the
    /// tick), plus the mouse motion not yet consumed by a tick. The result
    /// is continuous across ticks and follows the mouse frame by frame;
    /// the rest of the change (auto-level, key aiming) still interpolates.
    pub fn render_aim(&self, t: &tt_game::Tank, alpha: f32) -> (f32, f32) {
        let (yaw, pitch) = match self.prev_aim.get(&t.id) {
            Some(p) => (lerp(p.0, t.yaw_n, alpha), lerp(p.1, t.pitch_n, alpha)),
            None => (t.yaw_n, t.pitch_n),
        };
        let a = &self.player_aim;
        if a.tank != Some(t.id) || t.is_corpse {
            return (yaw, pitch);
        }
        // Without a previous tick nothing is interpolated: `consumed` is in already.
        let alpha = if self.prev_aim.contains_key(&t.id) { alpha } else { 1.0 };
        predict_aim((yaw, pitch), a.consumed, a.pending, alpha)
    }

    /// Reads the mouse aim not yet consumed by a tick for
    /// [`Self::render_aim`]. Called once per frame before drawing.
    pub fn set_pending_aim(&mut self, moves: &tt_gameplay::client::MoveState) {
        let (yaw, pitch) = moves.aim();
        self.player_aim.pending = (yaw / std::f32::consts::PI, pitch / std::f32::consts::PI);
    }

    /// Position of a flag (scrum ball) for rendering.
    pub fn render_flag_pos(&self, f: &tt_game::Flag, alpha: f32) -> Vec3 {
        match self.prev_flags.get(&f.id) {
            Some(&p) if (f.pos - p).length_squared() < 100.0 => p.lerp(f.pos, alpha),
            _ => f.pos,
        }
    }

    /// A copy of a tank at its render pose and aim (for what is drawn
    /// attached to it, such as the reticle).
    pub fn render_tank(&self, t: &tt_game::Tank, alpha: f32) -> tt_game::Tank {
        let mut r = t.clone();
        (r.pos, r.rot) = self.render_pose(t, alpha);
        (r.yaw_n, r.pitch_n) = self.render_aim(t, alpha);
        r
    }

    /// Chase-camera transform of a tank object at real time `now_ms`, from
    /// its pose interpolated by `alpha`. In camera aim the player's tank
    /// has the free camera.
    pub fn camera_transform(&mut self, aim: AimMode, obj: EntityId, now_ms: f64, alpha: f32) -> Option<Affine3A> {
        let id = self.tank_of(obj)?;
        let t = self.world.tank(id)?;
        let (pos, rot) = self.render_pose(t, alpha);
        let cd = self.camera_data(id);
        let terrain = self.world.terrain.clone();
        let ground = |x: f32, y: f32| terrain.as_ref().map(|h| h.height(x, y));
        let cam = self.cameras.entry(id).or_default();
        if self.free_aim.tank == Some(id) && aim == AimMode::Camera {
            let a = self.free_aim;
            let (m, pitch) = cam.free(now_ms, pos, &a, &cd, ground);
            // What the view could not take (tank out of sight) is dropped,
            // so turning back down responds at once.
            self.free_aim.pitch = pitch;
            return Some(m);
        }
        Some(cam.update(now_ms, pos, rot, &cd, ground))
    }

    /// Camera aim: starts the free camera for the player's tank (looking
    /// along its turret), or stops it (classic aim, no player, dead).
    pub fn sync_free_aim(&mut self, aim: AimMode, player: Option<TankId>) {
        let want = player
            .filter(|_| aim == AimMode::Camera && !self.world.ai_control_mode)
            .filter(|&p| self.world.tank(p).is_some_and(|t| !t.is_corpse));
        if want == self.free_aim.tank {
            return;
        }
        let half_vfov = self.free_aim.half_vfov;
        self.free_aim = FreeAim { half_vfov, ..FreeAim::default() };
        if let Some(t) = want.and_then(|p| self.world.tank(p)) {
            let yaw = wrap_pi(heading_of(t.rot.y_axis) + (t.data.turret_yaw_range * t.yaw_n).to_radians());
            self.free_aim = FreeAim { tank: want, yaw, pitch: 0.0, half_vfov };
        }
    }

    /// Camera aim: turns the free camera by `yaw`/`pitch` radians of mouse
    /// aim.
    pub fn turn_free_aim(&mut self, yaw: f32, pitch: f32) {
        let a = &mut self.free_aim;
        a.yaw = wrap_pi(a.yaw + yaw);
        a.pitch = (a.pitch - pitch).clamp(FREE_PITCH_RANGE.0, FREE_PITCH_RANGE.1);
    }

    /// Camera aim: the mouse motion not yet taken by a tick turns the free
    /// camera at once and is taken out of the turret's move. Called every
    /// frame and before each tick.
    pub fn free_look_input(&mut self, moves: &mut tt_gameplay::client::MoveState) {
        if self.free_aim.tank.is_none() {
            return;
        }
        let (yaw, pitch) = moves.aim();
        if yaw != 0.0 || pitch != 0.0 {
            moves.clear_aim();
            self.turn_free_aim(yaw, pitch);
        }
    }

    /// The free camera of tank `id` at its tick pose (unsmoothed), for steering.
    pub fn free_view_at_tick(&self, id: TankId) -> Option<Affine3A> {
        let t = self.world.tank(id)?;
        let cd = self.camera_data(id);
        let ground = |x: f32, y: f32| self.world.terrain.as_ref().map(|h| h.height(x, y));
        let h = free_eye_height(t.pos, self.free_aim.yaw, &cd, ground);
        Some(free_view(t.pos, self.free_aim.yaw, self.free_aim.pitch, h, self.free_aim.half_vfov, &cd, ground).0)
    }

    /// The point under the screen centre of `view` (or [`FREE_AIM_RANGE`]
    /// along it), ignoring tank `id`.
    pub fn aim_point(&self, view: &Affine3A, id: TankId) -> Vec3 {
        let eye: Vec3 = view.translation.into();
        let end = eye + Vec3::from(view.matrix3.y_axis) * FREE_AIM_RANGE;
        self.world.segment_hit_point(eye, end, Some(id)).unwrap_or(end)
    }

    /// Camera aim: the turret aim of tank `id` that hits the point under the centre of `view`. Yaw points the
    /// barrel at it (in the hull's frame, less the hull's turn of the last tick, clamped to `turret_yaw_range`);
    /// pitch is found by bisection over the turret's range so the shell's ballistic path (the current cartridge's
    /// velocity and gravity, as the reticle draws it) passes through it, or the nearest limit when out of reach.
    /// For a high-angle cartridge (the mortar) the pitch is solved over its own launch
    /// (`Tank::high_angle_pitch_n`): the shell lands on the point.
    pub fn turret_target(&self, id: TankId, view: &Affine3A) -> Option<TurretTarget> {
        let t = self.world.tank(id)?;
        let point = self.aim_point(view, id);
        let d = &t.data;
        let local = t.rot.transpose() * (point - t.pos);
        // Too close to the tank for a direction: the view's heading.
        let rel = if local.truncate().length() > 3.0 {
            local.x.atan2(local.y)
        } else {
            wrap_pi(heading_of(view.matrix3.y_axis.into()) - heading_of(t.rot.y_axis))
        };
        // The hull turns during the coming tick about as much as in the
        // last one; the turret yaw is relative to it.
        let turn = self.prev_pose.get(&id).map_or(0.0, |(_, r)| wrap_pi(heading_of(t.rot.y_axis) - heading_of(r.y_axis)));
        let rel = wrap_pi(rel - turn);
        let range = d.turret_yaw_range.to_radians().max(1e-3);
        let beyond = if rel > range { 1.0 } else if rel < -range { -1.0 } else { 0.0 };
        let yaw_n = (rel / range).clamp(-1.0, 1.0);
        // A high-angle cartridge (the mortar) launches at its own
        // elevation and speed, not along the barrel: its pitch comes from
        // the shell's real flight, so it comes down on the aim point. The
        // yaw is corrected for the barrel's heading on a tilted hull
        // (with the hull's turn of the coming tick kept out).
        if let Some(p) = t.current.as_ref().filter(|p| p.high_angle()) {
            let turn_n = turn / range;
            let (y, pitch_n) = t.high_angle_aim(p, (yaw_n + turn_n).clamp(-1.0, 1.0), point, self.world.gravity);
            let yaw_n = if beyond == 0.0 { (y - turn_n).clamp(-1.0, 1.0) } else { yaw_n };
            return Some(TurretTarget { point, yaw_n, pitch_n, beyond });
        }
        let (speed, grav) = t.current.as_ref().map_or((100.0, 0.0), |p| (p.velocity, -p.fall(self.world.gravity)));
        // Height of the shell's path over the aim point where it passes it
        // horizontally (decreasing in pitchN: + = barrel down).
        let over = |pitch_n: f32| {
            let mut c = t.clone();
            (c.yaw_n, c.pitch_n) = (yaw_n, pitch_n);
            let m = c.muzzle();
            let (p0, v) = (Vec3::from(m.translation), Vec3::from(m.matrix3.y_axis) * speed);
            let vh = v.truncate().length();
            if vh < 1e-3 {
                return v.z;
            }
            let s = (point - p0).truncate().length() / vh;
            p0.z + v.z * s + 0.5 * grav * s * s - point.z
        };
        let pitch_n = if over(-1.0) <= 0.0 {
            -1.0
        } else if over(1.0) >= 0.0 {
            1.0
        } else {
            let (mut up, mut down) = (-1.0f32, 1.0f32);
            for _ in 0..20 {
                let m = 0.5 * (up + down);
                if over(m) > 0.0 {
                    up = m;
                } else {
                    down = m;
                }
            }
            0.5 * (up + down)
        };
        Some(TurretTarget { point, yaw_n, pitch_n, beyond })
    }
}

fn sequence_duration(shape: &tt_assets::dts::Shape, name: &str) -> Option<f32> {
    shape.sequences.iter().find(|s| s.name.eq_ignore_ascii_case(name)).map(|s| s.duration)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Poses a tank's nodes for drawing; `alpha` interpolates the turret aim and
/// wheel spin between the last two ticks.
pub fn pose_tank(g: &GameSim, t: &tt_game::Tank, alpha: f32, shape: &tt_assets::dts::Shape, inst: &mut tt_render::DynamicInstance) {
    use glam::Mat4;
    use tt_render::pose::{node_default, node_transform, sequence_pose_of, sequence_visibility_of};
    let d = &t.data;
    if t.is_corpse {
        // Corpse render transform: during corpse_shake_time the tank
        // shakes (Euler x/y wobble); then the death sequence flips it by
        // moving `codeFlip`: drawn at T · codeFlip · tankall⁻¹.
        let (ct, fx) = ((t.corpse_timer - (1.0 - alpha) * tt_game::DT).max(0.0), &d.fx);
        if ct < fx.corpse_shake_time {
            let (w, a) = (std::f32::consts::PI * fx.corpse_shake_speed, 0.15 * fx.corpse_shake_amp);
            let (x, y) = ((10.0 * w * ct).sin() * a, (9.0 * w * (ct - fx.corpse_shake_time)).sin() * a);
            inst.transform *= Mat4::from_mat3(tt_game::geom::rot_x(x) * Mat3::from_rotation_y(-y));
        } else if let Some((k, _)) = t.death_sequence {
            // The world picks death1..4 once the corpse shake is over.
            let seq = format!("death{}", k + 1);
            inst.nodes = sequence_pose_of(shape, &seq, ct - fx.corpse_shake_time);
            for (o, vis) in sequence_visibility_of(shape, &seq, ct - fx.corpse_shake_time) {
                if vis { inst.shown.push(o) } else { inst.hidden.push(o) }
            }
            if let (Some(flip), Some(all)) = (node_transform(shape, &inst.nodes, "codeFlip"), node_transform(shape, &inst.nodes, "tankall")) {
                inst.transform = inst.transform * flip * all.inverse();
            }
            return;
        }
    }
    let (yaw_n, pitch_n) = g.render_aim(t, alpha);
    let yaw = (d.turret_yaw_range * yaw_n).to_radians();
    let pitch = (d.turret_pitch_range * pitch_n - d.turret_pitch_center).to_radians();
    if let Some(m) = node_default(shape, "codeTurret") {
        inst.nodes.push(("codeTurret".into(), m * Mat4::from_mat3(tt_game::geom::rot_z(yaw))));
    }
    if let Some(m) = node_default(shape, "codeWeapon") {
        inst.nodes.push(("codeWeapon".into(), m * Mat4::from_mat3(tt_game::geom::rot_x(pitch))));
    }
    if let Some(tr) = g.treads.get(&t.id) {
        let angles = tr.state.wheel_angles(&tr.rig, alpha);
        for (n, a) in crate::treads::WHEEL_NODES.iter().zip(angles) {
            if let Some(m) = node_default(shape, n) {
                inst.nodes.push(((*n).into(), Mat4::from_translation(m.w_axis.truncate()) * Mat4::from_mat3(tt_game::geom::rot_x(a))));
            }
        }
    }
    // The `fire` thread, restarted by every shot: its visibility track
    // (muzzle flash) and its node tracks (the Light tank's
    // `barrelA2`/`barrelB2` spin 120° per shot, a gatling spin while it
    // keeps firing). Nodes posed above by code stay as they are.
    if let (Some(&at), Some(dur)) = (g.last_fired.get(&t.id), sequence_duration(shape, "fire")) {
        let since = (g.world.time_ms - at) as f32 / 1000.0;
        if since >= 0.0 && since <= dur {
            for (o, vis) in sequence_visibility_of(shape, "fire", since) {
                if vis { inst.shown.push(o) } else { inst.hidden.push(o) }
            }
        }
        // Between ticks for a smooth spin: `alpha` 0 is the previous tick.
        let since = since - (1.0 - alpha) * tt_game::DT;
        if since <= dur {
            for (n, m) in sequence_pose_of(shape, "fire", since.max(0.0)) {
                if !inst.nodes.iter().any(|(k, _)| *k == n) {
                    inst.nodes.push((n, m));
                }
            }
        }
    }
}

/// case-insensitive "red" 0, "green" 1, "blue" 2, "purple" 3; any other
/// string leaves the team unchanged (`None`).
pub fn team_index(color: &str) -> Option<i32> {
    match color.to_ascii_lowercase().as_str() {
        "red" => Some(0),
        "green" => Some(1),
        "blue" => Some(2),
        "purple" => Some(3),
        _ => None,
    }
}

/// `rotation` field (degrees) as the radians of a transform string.
pub fn rotation_rad(r: &str) -> String {
    let v: Vec<f64> = r.split_whitespace().map(tt_game::fmt::atof).collect();
    match v.as_slice() {
        [x, y, z, a] => format!("{x} {y} {z} {}", a.to_radians()),
        _ => "0 0 1 0".into(),
    }
}

fn label(e: &Event) -> &'static str {
    match e {
        Event::Fired { .. } => "Fired",
        Event::DryFired { .. } => "DryFired",
        Event::ProjectileBounced { .. } => "ProjectileBounced",
        Event::ProjectileExploded { .. } => "ProjectileExploded",
        Event::ProjectileRemoved { .. } => "ProjectileRemoved",
        Event::Damaged { .. } => "Damaged",
        Event::Killed { .. } => "Killed",
        Event::TankRemoved { .. } => "TankRemoved",
        Event::Collided { .. } => "Collided",
        Event::FlagDropped { .. } => "FlagDropped",
        Event::PowerUpEntered { .. } => "PowerUpEntered",
        Event::PowerUpLeft { .. } => "PowerUpLeft",
        Event::PowerUpSwitched { .. } => "PowerUpSwitched",
        Event::PadUsed { .. } => "PadUsed",
        Event::PadStateChanged { .. } => "PadStateChanged",
        Event::VolcanoErupted { .. } => "VolcanoErupted",
        Event::FlagPickedUp { .. } => "FlagPickedUp",
        Event::FlagOutOfBounds { .. } => "FlagOutOfBounds",
        Event::BrainEjected { .. } => "BrainEjected",
        Event::UserThreadPlayed { .. } => "UserThreadPlayed",
    }
}

/// The native defs as the sim reads them.
pub fn native_defs(h: &EngineHost) -> Defs {
    let mut db = Defs::new();
    let scales = h.srv.solo_scales.as_ref();
    db.add(h.defs.objects().filter(|o| !o.name.is_empty()).map(|o| {
        let mut o = o.clone();
        // `setSPTankParams`: the solo campaign's scales of the tank defs.
        if let Some(sc) = scales {
            for (k, v) in &mut o.fields {
                if let Some(x) = sc.get(&o.name, k) {
                    *v = x.to_string();
                }
            }
        }
        o
    }));
    db
}

/// Builds or drops the world as `MissionGroup` comes and goes.
pub fn update_world(h: &mut EngineHost) {
    let mg = h.mission_load.as_ref().map(|l| l.group);
    match (mg, h.game.as_ref().map(|g| g.mission_group)) {
        (Some(m), Some(cur)) if m == cur => {}
        (Some(_), _) => {
            let dir = crate::mission::mission_dir(h);
            // The mission as its RON file declared it.
            let load = h.mission_load.clone().expect("the group is the loaded mission's");
            let db = native_defs(h);
            let first = h.srv.next_entity.max(h.game.as_ref().map_or(1, |g| g.entities.next_id()));
            match GameSim::build(h.fs.root(), &dir, &load, db, first) {
                Ok(mut g) => {
                    // The mission's balls (the Power-up Test's practice ball),
                    // then the tanks the server created before the world.
                    for (_, o) in &load.nodes {
                        if o.class.eq_ignore_ascii_case("Flag") {
                            let (pos, _) = parse_transform(o.get("position").unwrap_or(""));
                            g.add_flag(o.get("def").unwrap_or(""), &o.name, pos, None);
                        }
                    }
                    crate::gameplay::server_host::materialize_pending(h, &mut g);
                    h.srv.next_entity = g.entities.next_id();
                    let seed = h.rng.seed() as u32;
                    g.world.set_random_seed(seed);
                    // Network roles (single player and listen servers stay Local).
                    if crate::netgame::is_remote_client(h) && h.net.server.is_none() {
                        g.set_role(Role::Client);
                    } else if h.srv.get().is_some_and(|s| s.setup.dedicated) {
                        g.set_role(Role::Dedicated);
                    }
                    if g.role == Role::Client {
                        crate::netgame::world_rebuilt(&mut h.netgame);
                    }
                    // Initial power-up states are not switches (no sounds).
                    let _ = g.world.take_events();
                    h.log.info(&format!("Simulation: {} statics, {} triggers, terrain {}", g.world.statics.len(), g.world.triggers.len(), g.world.terrain.is_some()));
                    h.game = Some(g);
                }
                Err(e) => {
                    h.log.warn(&format!("simulation: {e}"));
                    h.game = None;
                }
            }
        }
        (None, Some(_)) => {
            let next = h.game.as_ref().map_or(1, |g| g.entities.next_id());
            h.srv.next_entity = h.srv.next_entity.max(next);
            h.game = None;
        }
        (None, None) => {}
    }
}

/// Looping engine sounds of all living tanks: two 3D sources (TankFxParams
/// `engine_sound_a/B`), pitch and volume interpolated between their `0` and `1`
/// values by speed / max_speed. A linear blend is used.
fn update_engine_sounds(h: &mut EngineHost, g: &mut GameSim) {
    let live: Vec<(TankId, Vec3, f32, bool)> = g
        .world
        .tanks
        .iter()
        .filter(|t| g.entity_of_tank(t.id).is_some())
        .map(|t| (t.id, t.pos, (t.vel.length() / t.data.max_speed.max(0.1)).clamp(0.0, 1.0), t.is_corpse))
        .collect();
    let gone: Vec<TankId> = g.engine_sounds.keys().copied().filter(|id| !live.iter().any(|l| l.0 == *id && !l.3)).collect();
    for id in gone {
        if let Some((a, b)) = g.engine_sounds.remove(&id) {
            h.audio.stop(a);
            h.audio.stop(b);
        }
    }
    for (id, pos, f, corpse) in live {
        if corpse {
            continue;
        }
        let field = |n: &str| g.fx_value(id, n).unwrap_or("");
        let num = |n: &str| tt_game::fields::atof(field(n));
        // (pitch at 0, pitch at 1, volume at 0, volume at 1) of A and B.
        let curve = |k: [&str; 4]| (num(k[0]), num(k[1]), num(k[2]), num(k[3]));
        let curves = [curve(["engine_pitch_a0", "engine_pitch_a1", "engine_vol_a0", "engine_vol_a1"]), curve(["engine_pitch_b0", "engine_pitch_b1", "engine_vol_b0", "engine_vol_b1"])];
        let (a, b) = match g.engine_sounds.get(&id) {
            Some(&ab) => ab,
            None => {
                let (snd_a, snd_b) = (field("engine_sound_a").to_string(), field("engine_sound_b").to_string());
                let mut start = |snd: String| {
                    if snd.is_empty() || h.audio.profile_name(&snd).is_none() {
                        return 0;
                    }
                    let s = h.audio.create_source(&snd, None, Some(pos.into()));
                    h.audio.play(s)
                };
                let ab = (start(snd_a), start(snd_b));
                g.engine_sounds.insert(id, ab);
                ab
            }
        };
        for (src, (p0, p1, v0, v1)) in [(a, curves[0]), (b, curves[1])] {
            if src == 0 {
                continue;
            }
            h.audio.source_3f(src, tt_audio::AlParam::Position, pos.into());
            h.audio.source_f(src, tt_audio::AlParam::Pitch, (p0 + (p1 - p0) * f).max(0.05));
            h.audio.source_f(src, tt_audio::AlParam::Gain, (v0 + (v1 - v0) * f).max(0.0));
        }
    }
}

fn play_at(h: &mut EngineHost, profile: &str, pos: Vec3) {
    if !profile.is_empty() && h.audio.profile_name(profile).is_some() {
        h.audio.play_profile(profile, Some(pos.into()));
    }
}

/// Presentation of one world event: sounds, explosions, muzzle flashes,
/// brain jars (every role but `Dedicated`). Never calls scripts.
fn cosmetic_event(h: &mut EngineHost, g: &mut GameSim, e: &Event, player: Option<TankId>) {
    match e {
        Event::Fired { tank, def, .. } => {
            g.last_fired.insert(*tank, g.world.time_ms);
            if let Some(t) = g.world.tank(*tank) {
                let p = t.pos;
                // A cartridge's own `fire_sound` (mortar, scatter shot).
                let own = if def.is_empty() { None } else { g.projectile(def).map(|d| d.fire_sound.clone()).filter(|s| !s.is_empty()) };
                play_at(h, own.as_deref().unwrap_or("fireSound"), p);
            }
        }
        Event::DryFired { tank } => {
            if let Some(t) = g.world.tank(*tank) {
                let p = t.pos;
                play_at(h, "dryFireSound", p);
            }
        }
        Event::ProjectileBounced { projectile, pos, .. } => {
            if let Some(p) = g.world.projectile(*projectile).map(|p| p.data.bounce_explosion.clone()).filter(|e| !e.is_empty()) {
                g.explosions.push((p, *pos));
            }
        }
        Event::ProjectileExploded { pos, hit, def, .. } => {
            if let Some(p) = g.db.projectile(def) {
                let ex = match hit {
                    // The own-tank variant when the player's tank is hit.
                    HitObject::Tank(t) if Some(*t) == player && !p.own_tank_explosion.is_empty() => p.own_tank_explosion,
                    HitObject::Tank(_) => p.tank_explosion,
                    _ => p.other_explosion,
                };
                if !ex.is_empty() {
                    g.explosions.push((ex.clone(), *pos));
                }
                let snd = g.db.fields(&ex).and_then(|f| f.get("sound").map(str::to_string)).unwrap_or_default();
                play_at(h, &snd, *pos);
            }
        }
        Event::PadUsed { trigger, .. } | Event::VolcanoErupted { trigger, .. } => {
            if let (Some(_), Some(tr)) = (g.entities.of_trigger(*trigger), g.world.trigger(*trigger)) {
                let p: Vec3 = tr.transform.translation.into();
                play_at(h, g.def_fields(EntityKind::Trigger(*trigger)).as_ref().and_then(|f| f.get("sound")).unwrap_or(""), p);
            }
        }
        Event::PowerUpSwitched { trigger, on } => {
            if g.known_triggers.contains(trigger) {
                g.activations.insert(*trigger, (g.world.time_ms, *on));
            }
            // PowerUpParams `sound` when it (re)appears, `sound_off` when taken.
            if let (Some(_), Some(tr)) = (g.entities.of_trigger(*trigger), g.world.trigger(*trigger)) {
                let p: Vec3 = tr.transform.translation.into();
                play_at(h, g.def_fields(EntityKind::Trigger(*trigger)).as_ref().and_then(|f| f.get(if *on { "sound" } else { "sound_off" })).unwrap_or(""), p);
            }
        }
        Event::BrainEjected { tank, projectile, shape } => {
            if let Some(p) = projectile.filter(|_| !shape.is_empty()) {
                g.brain_shapes.insert(p, shape.clone());
            }
            if let (Some(_), Some(t)) = (g.entity_of_tank(*tank), g.world.tank(*tank)) {
                let p = t.pos;
                play_at(h, g.fx_value(*tank, "loose_head_sound").unwrap_or(""), p);
            }
        }
        Event::Collided { tank, other, speed } => {
            // TankFxDef hard/soft collision sounds by relative speed,
            // at most once per `*_collision_interval_ms` per tank and kind.
            if let (Some(_), Some(t)) = (g.entity_of_tank(*tank), g.world.tank(*tank)) {
                let (kind, k) = match other {
                    HitObject::Tank(_) => ("tank", 0u8),
                    HitObject::Static(_) => ("static", 1),
                    HitObject::Terrain => ("ground", 2),
                };
                let num = |n: String| tt_game::fields::atof(g.fx_value(*tank, &n).unwrap_or(""));
                let interval = num(format!("{kind}_collision_interval_ms")) as i64;
                let now = g.world.time_ms;
                if g.last_coll_sound.get(&(*tank, k)).is_none_or(|&last| now - last >= interval) {
                    let snd = if *speed >= num(format!("hard_{kind}_collision_speed")) {
                        g.fx_value(*tank, &format!("hard_{kind}_collision_sound")).unwrap_or_default().to_string()
                    } else if *speed >= num(format!("soft_{kind}_collision_speed")) {
                        g.fx_value(*tank, &format!("soft_{kind}_collision_sound")).unwrap_or_default().to_string()
                    } else {
                        String::new()
                    };
                    if !snd.is_empty() {
                        let p = t.pos;
                        play_at(h, &snd, p);
                        g.last_coll_sound.insert((*tank, k), now);
                    }
                }
            }
        }
        Event::ProjectileRemoved { projectile } => {
            // Expired brain jars (cosmetic, 5 s) stop being drawn.
            g.brain_shapes.remove(projectile);
        }
        _ => {}
    }
}

/// One simulation tick: moves in, world tick, events to scripts, poses out.
pub fn tick(h: &mut EngineHost) {
    update_world(h);
    let Some(mut g) = h.game.take() else { return };
    // The player's move (server-side control object of the local connection).
    let player_obj = if g.role == Role::Client { crate::netgame::control_object(h) } else { h.net.server.and_then(|s| h.net.control.get(&s).copied()) };
    let player = player_obj.and_then(|o| g.tank_of(o));
    // Camera aim: the mouse turns the free camera, not the turret.
    g.sync_free_aim(aim_mode(&h.shell.prefs), player);
    g.free_look_input(&mut h.client.moves);
    // Mouse aim this move consumes, as the server will see it (render
    // prediction only, see `GameSim::render_aim`).
    let mouse = {
        let (yaw, pitch) = h.client.moves.aim();
        let q = |a: f32| if g.world.quantize_moves { tt_game::netmove::u16_to_angle(tt_game::netmove::angle_to_u16(a)) } else { a };
        (q(yaw) / std::f32::consts::PI, q(pitch) / std::f32::consts::PI)
    };
    let mut mv = h.client.moves.take_move(reverse_steering(&h.shell.prefs), &mut g.last_trigger_count);
    g.player_aim = PlayerAim { tank: player.filter(|_| !g.world.ai_control_mode), consumed: mouse, pending: (0.0, 0.0) };
    if let Some(p) = player {
        if g.free_aim.tank == Some(p) {
            // Camera aim: key aiming turns the camera too; the move turns
            // the turret towards the camera's aim point, and the hull
            // towards it when it is beyond the turret's range.
            g.turn_free_aim(mv.yaw, mv.pitch);
            let target = g.free_view_at_tick(p).and_then(|v| g.turret_target(p, &v));
            if let (Some(tg), Some(t)) = (target, g.world.tank(p)) {
                let lim = 0.99 * std::f32::consts::PI;
                mv.yaw = ((tg.yaw_n - t.yaw_n) * std::f32::consts::PI).clamp(-lim, lim);
                mv.pitch = ((tg.pitch_n - t.pitch_n) * std::f32::consts::PI).clamp(-lim, lim);
                if tg.beyond != 0.0 && mv.x as i32 == 0 {
                    mv.x = tg.beyond;
                }
            }
            g.player_aim.tank = None;
        } else if tt_game::fmt::atoi(&h.shell.prefs.get("input.auto_turn")) != 0 {
            if let Some(t) = g.world.tank(p) {
                tt_game::tank::apply_auto_turn(&mut mv, t);
            }
        }
        if g.role == Role::Client {
            // A remote client's moves go to the server (no prediction yet).
            crate::netgame::push_move(&mut h.netgame, mv);
        } else {
            g.world.set_move(p, mv);
        }
    }
    // Remote clients' moves (server).
    if g.role.authority() {
        crate::netgame::apply_remote_moves(&mut h.netgame, &h.net, &mut g);
    }
    // Player tanks are a connection's control object: their corpses stay
    // until the client takes a new control object (respawn); then they
    // fade like a bot's.
    let controlled: std::collections::HashSet<TankId> = h.net.control.values().filter_map(|e| g.tank_of(*e)).collect();
    let flags: Vec<(TankId, bool)> = g.world.tanks.iter().map(|t| (t.id, t.controlled)).collect();
    for (t, was) in flags {
        let now = controlled.contains(&t);
        if was != now {
            g.world.set_controlling_client(t, now);
        }
    }
    g.world.ai_control_mode = h.srv.get().is_some_and(|s| s.setup.ai_control);
    g.world.team_game = h.srv.get().is_some_and(|s| s.setup.team_game);
    g.prev_pose = g.world.tanks.iter().map(|t| (t.id, (t.pos, t.rot))).collect();
    let was_corpse: std::collections::HashSet<TankId> = g.world.tanks.iter().filter(|t| t.is_corpse).map(|t| t.id).collect();
    g.prev_aim = g.world.tanks.iter().map(|t| (t.id, (t.yaw_n, t.pitch_n))).collect();
    g.prev_flags = g.world.flags.iter().map(|f| (f.id, f.pos)).collect();
    if g.role == Role::Client {
        // Puppets to the render time (their previous poses are saved above).
        crate::netgame::apply_states(&mut h.netgame, &mut g);
    }
    let mut events = g.world.take_events();
    events.extend(g.world.tick());
    if g.role.authority() {
        // The health power-up floor, before the clients see the tick.
        let respawned: Vec<tt_game::TriggerId> = events.iter().filter_map(|e| if let Event::PowerUpSwitched { trigger, on: true } = e { Some(*trigger) } else { None }).collect();
        let single = h.srv.get().is_some_and(|s| s.setup.single_player());
        crate::gameplay::health_packs::apply(&h.shell.prefs, single, &mut g, &respawned);
        crate::netgame::capture(&mut h.netgame, &g, &events);
    }
    // Per-tank render state of tanks that are gone.
    let live: std::collections::HashSet<TankId> = g.world.tanks.iter().map(|t| t.id).collect();
    g.last_fired.retain(|k, _| live.contains(k));
    g.last_coll_sound.retain(|k, _| live.contains(&k.0));
    g.cameras.retain(|k, _| live.contains(k));
    g.treads.retain(|k, _| live.contains(k));
    let projectiles: std::collections::HashSet<tt_game::ProjectileId> = g.world.projectiles.iter().map(|p| p.id).collect();
    g.brain_shapes.retain(|k, _| projectiles.contains(k));
    // Treads and wheels: each side's travel from the track edges'
    // displacement along the tank's forward axis; living tanks only (alive
    // before this tick, as processTick checks before it moves). Presentation
    // only.
    let cosmetic = g.role.cosmetic();
    for t in g.world.tanks.iter().filter(|_| cosmetic) {
        let Some(&old) = g.prev_pose.get(&t.id) else { continue };
        if !g.treads.contains_key(&t.id) {
            let Some(shape) = (!t.data.shape_file.is_empty()).then(|| tt_render::pose::shape(&tt_assets::resolve(&g.root, &g.mission_dir, &t.data.shape_file))).flatten() else { continue };
            g.treads.insert(t.id, crate::treads::Treads::new(crate::treads::TreadRig::from_shape(&shape)));
        }
        let alive = !was_corpse.contains(&t.id);
        if let Some(tr) = g.treads.get_mut(&t.id) {
            tr.tick(alive, old, (t.pos, t.rot));
        }
    }
    let mut game_events = Vec::new();
    if g.role.authority() {
        crate::gameplay::golden::record::on_world_events(h, &events);
        // K/D/A from the damage and kills.
        let kda: Vec<KdaEvent> = events
            .iter()
            .filter_map(|e| match e {
                Event::Damaged { tank, amount, source: Some(src) } if *amount > 0.0 && src != tank => Some(KdaEvent::Damaged { victim: g.entity_of_tank(*tank)?, source: g.entity_of_tank(*src)? }),
                Event::Killed { tank, killer } => Some(KdaEvent::Killed { victim: g.entity_of_tank(*tank)?, killer: killer.and_then(|k| g.entity_of_tank(k)) }),
                _ => None,
            })
            .collect();
        let now = g.world.time_ms;
        h.game = Some(g);
        crate::gameplay::scoreboard::track(h, &kda, now);
        g = h.game.take().expect("the world");
        game_events = GameEvent::of_world_events(&g.entities, &events);
    }
    for e in &events {
        *g.event_counts.entry(label(e)).or_default() += 1;
        if g.role.cosmetic() {
            cosmetic_event(h, &mut g, e, player);
        }
    }
    if g.role.cosmetic() {
        update_engine_sounds(h, &mut g);
    }
    g.known_triggers = g.world.triggers.iter().map(|t| t.id).collect();
    let known = &g.known_triggers;
    g.activations.retain(|k, _| known.contains(k));
    // Pickup hints for the player's tank.
    let mut hints: Vec<(&str, &str, &str)> = Vec::new();
    if let Some(t) = player.and_then(|p| g.world.tank(p)).filter(|_| cosmetic) {
        let (dmg, ammo) = (t.damage, t.ammo);
        let default = t.current.as_ref().zip(t.data.default_projectile.as_ref()).is_some_and(|(c, d)| c.name.eq_ignore_ascii_case(&d.name));
        if let Some((_, pd, pa)) = g.prev_player.filter(|(id, ..)| *id == t.id) {
            // Health: damage crosses int(damage_amount * 0.65); not in single player.
            let thr = (t.data.damage_amount * 0.65) as i32 as f32;
            if pd < thr && dmg >= thr && !h.srv.get().is_some_and(|s| s.setup.single_player()) {
                hints.push(("You need a health power-up!", "2", "2"));
            }
            // Ammo: the default projectile's ammo drops to 20 % of its count.
            if let Some(c) = t.current.as_ref().filter(|_| default) {
                let thr = (c.rounds as f32 * 0.2) as i32;
                if pa > thr && ammo <= thr {
                    hints.push(("You need ammo!", "4", "2"));
                }
            }
        }
        g.ammo_warning = default && t.current.as_ref().is_some_and(|c| ammo <= (c.rounds as f32 * 0.2) as i32);
        g.prev_player = Some((t.id, dmg, ammo));
    }
    h.game = Some(g);
    for (msg, time, size) in hints {
        let now = h.sim_time() as u64;
        h.client.bottom_print(msg, time, size, now);
    }
    // The game rules react to the tick (destroyed tanks, power-ups, the
    // ball out of bounds), then the corpses that timed out go.
    let exists = |h: &EngineHost, e: EntityId| h.game.as_ref().is_some_and(|g| g.entities.get(e).is_some());
    for e in &game_events {
        match e {
            GameEvent::TankDestroyed { tank, killer } if exists(h, *tank) => {
                let (tank, killer) = (*tank, *killer);
                crate::gameplay::server_host::with_server(h, |s, b| s.tank_destroyed(b, tank, killer));
            }
            GameEvent::PowerUpEnter { pup, tank, .. } if exists(h, *pup) && exists(h, *tank) => {
                let db = h.game.as_ref().and_then(|g| g.entities.get(*pup)).map(|e| e.def.clone()).unwrap_or_default();
                let (pup, tank) = (*pup, *tank);
                if let Some(def) = h.data.pickups.find(&db) {
                    crate::gameplay::server_host::with_server(h, |s, b| s.pickup_entered(b, def, pup, tank));
                }
            }
            GameEvent::FlagOutOfBounds { flag } if exists(h, *flag) => {
                let flag = *flag;
                crate::gameplay::server_host::with_server(h, |s, b| s.ball_out_of_bounds(b, flag));
            }
            _ => {}
        }
    }
    for e in &game_events {
        if let GameEvent::TankRemoved { tank } = e {
            if exists(h, *tank) {
                let tank = *tank;
                crate::gameplay::server_host::with_server(h, |s, b| s.delete_tank(b, tank));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tt_gameplay::client::binds::MoveAxis;
    use tt_gameplay::client::MoveState;
    use tt_game::Move;

    #[test]
    fn player_moves() {
        let prefs = tt_shell::Prefs::default();
        let mut m = MoveState::default();
        m.set_axis(MoveAxis::Forward, "1");
        m.set_axis(MoveAxis::Left, "1");
        m.yaw = tt_gameplay::client::text::SVal::Str("0.01".into());
        m.trigger(0);
        let mut last = 0;
        let mv = m.take_move(reverse_steering(&prefs), &mut last);
        assert_eq!((mv.x, mv.y, mv.yaw), (-1.0, 1.0, 0.01));
        assert!(mv.trigger[0]);
        assert_eq!(m.yaw.string(), "0");
        m.trigger(0);
        assert!(m.take_move(reverse_steering(&prefs), &mut last).trigger[0]); // changed
        assert!(!m.take_move(reverse_steering(&prefs), &mut last).trigger[0]); // even, unchanged
    }

    /// Yaw change (radians, + = right/clockwise from above) after holding
    /// the given keys for `ticks` ticks on flat ground.
    fn held_turn(prefs: &tt_shell::Prefs, keys: &[MoveAxis], ticks: usize) -> f32 {
        let mut m = MoveState::default();
        for k in [MoveAxis::Forward, MoveAxis::Backward, MoveAxis::Left, MoveAxis::Right] {
            m.set_axis(k, if keys.contains(&k) { "1" } else { "0" });
        }
        let f = tt_game::fields::Fields::from_pairs(
            "MediumTank",
            "TankDef",
            &[("turn_rate", "68"), ("accel_rate", "12"), ("decel_rate", "20"), ("coast_decel_rate", "10"), ("anti_slide_rate", "11.1"), ("max_speed", "11.9"), ("suspension_range", "0.6"), ("damage_amount", "14")],
        );
        let data = std::sync::Arc::new(tt_game::TankParams::from_fields(&f, None));
        let mut world = tt_game::World::new(Some(tt_game::HeightField::flat(0.0)));
        let id = world.add_tank(data, Vec3::ZERO, 0.0);
        let mut last = 0;
        for _ in 0..ticks {
            let mv = m.take_move(reverse_steering(prefs), &mut last);
            world.set_move(id, mv);
            world.tick();
        }
        let f = world.tank(id).unwrap().forward();
        f.x.atan2(f.y)
    }

    #[test]
    fn reverse_steering_turns_like_a_car() {
        use MoveAxis::*;
        let mut prefs = tt_shell::Prefs::default();
        // Pref unset: off (pivot steering), like the settings default.
        assert!(!reverse_steering(&prefs));
        // On: S+A backs up turning the hull right, S+D left.
        prefs.set("input.reverse_steering", "1");
        let sa = held_turn(&prefs, &[Backward, Left], 20);
        let sd = held_turn(&prefs, &[Backward, Right], 20);
        let wa = held_turn(&prefs, &[Forward, Left], 20);
        assert!(sa > 0.1, "S+A should yaw right, got {sa}");
        assert!(sd < -0.1, "S+D should yaw left, got {sd}");
        assert!(wa < -0.1, "W+A yaws left, got {wa}");
        prefs.set("input.reverse_steering", "0");
        let sa0 = held_turn(&prefs, &[Backward, Left], 20);
        assert!(sa0 < -0.1, "classic S+A yaws left, got {sa0}");
        // The move itself: x is negated only while y < 0.
        prefs.set("input.reverse_steering", "1");
        let mut m = MoveState::default();
        m.set_axis(Backward, "1");
        m.set_axis(Left, "1");
        let mut last = 0;
        assert_eq!(m.take_move(reverse_steering(&prefs), &mut last).x, 1.0);
        m.set_axis(Backward, "0");
        assert_eq!(m.take_move(reverse_steering(&prefs), &mut last).x, -1.0);
    }

    #[test]
    fn roles_split_authority_and_presentation() {
        assert_eq!(Role::default(), Role::Local);
        assert!(Role::Local.authority() && Role::Local.cosmetic());
        assert!(Role::Dedicated.authority() && !Role::Dedicated.cosmetic());
        assert!(!Role::Client.authority() && Role::Client.cosmetic());
    }

    #[test]
    fn transform_strings_roundtrip() {
        let rot = Mat3::from_quat(Quat::from_axis_angle(Vec3::Z, -0.7));
        let s = transform_string(Vec3::new(1.0, 2.0, 3.0), rot);
        let (p, r) = parse_transform(&s);
        assert_eq!(p, Vec3::new(1.0, 2.0, 3.0));
        assert!((r.unwrap().y_axis - rot.y_axis).length() < 1e-4);
        let (_, r) = parse_transform("0 0 0 0 0 1 0.7");
        assert!(r.unwrap().y_axis.x > 0.6);
    }

    #[test]
    fn chase_camera_behind_and_above() {
        let mut c = ChaseCam::default();
        let cd = CameraParams::default();
        let m = c.update(0.0, Vec3::ZERO, Mat3::IDENTITY, &cd, |_, _| Some(0.0));
        let eye: Vec3 = m.translation.into();
        assert!((eye - Vec3::new(0.0, -6.2, 6.22)).length() < 1e-4);
        assert!(m.matrix3.y_axis.y > 0.5);
        // Smoothing keeps part of the previous position after 32 ms.
        let m2 = c.update(32.0, Vec3::new(0.0, 5.0, 0.0), Mat3::IDENTITY, &cd, |_, _| Some(0.0));
        assert!(m2.translation.y < 5.0 - 6.2);
        // A gap over 100 ms snaps.
        let m3 = c.update(500.0, Vec3::new(0.0, 5.0, 0.0), Mat3::IDENTITY, &cd, |_, _| Some(0.0));
        assert!((m3.translation.y - (5.0 - 6.2)).abs() < 1e-4);
    }

    /// Heading (radians, + = clockwise from above) of a camera's look.
    fn heading(m: &Affine3A) -> f32 {
        heading_of(m.matrix3.y_axis.into())
    }

    /// Pitch (radians, + = up) of a camera's look.
    fn look_pitch(m: &Affine3A) -> f32 {
        m.matrix3.y_axis.z.clamp(-1.0, 1.0).asin()
    }

    #[test]
    fn free_camera_turns_with_the_mouse_at_once() {
        let cd = CameraParams::default();
        let flat = |_: f32, _: f32| Some(0.0);
        let mut c = ChaseCam::default();
        let aim = |yaw: f32| FreeAim { tank: None, yaw, pitch: 0.0, half_vfov: DEFAULT_HALF_VFOV };
        let m = c.free(0.0, Vec3::ZERO, &aim(0.7), &cd, flat).0;
        let o = ChaseCam::default().update(0.0, Vec3::ZERO, Mat3::from_rotation_z(-0.7), &cd, flat);
        assert!((Vec3::from(m.translation) - Vec3::from(o.translation)).length() < 1e-4);
        assert!((Vec3::from(m.matrix3.y_axis) - Vec3::from(o.matrix3.y_axis)).length() < 1e-4);
        // The next frame looks wherever the mouse turned it, any amount (no
        // smoothing, no limit), even all the way round.
        for (i, yaw) in [0.75f32, -2.0, 3.0, 0.1].into_iter().enumerate() {
            let m = c.free(16.0 * (i + 1) as f64, Vec3::ZERO, &aim(yaw), &cd, flat).0;
            assert!(wrap_pi(heading(&m) - yaw).abs() < 1e-4, "{yaw}: {}", heading(&m));
            let eye: Vec3 = m.translation.into();
            assert!((eye.truncate().length() - cd.backup_dist).abs() < 1e-3);
        }
    }

    #[test]
    fn free_camera_orbits_up_and_down() {
        let cd = CameraParams::default();
        let flat = |_: f32, _: f32| Some(0.0);
        let cam = |p: f32| free_view(Vec3::ZERO, 0.3, p, cd.float_height, DEFAULT_HALF_VFOV, &cd, flat);
        let (level, _) = cam(0.0);
        let pivot = Vec3::Z * cd.focus_up_dist;
        // The pivot's angle below the view centre.
        let below = |m: &Affine3A| look_pitch(m) - (pivot - Vec3::from(m.translation)).normalize().z.asin();
        for p in [FREE_PITCH_RANGE.0, -0.2, 0.2, 0.4] {
            let (m, shown) = cam(p);
            // The view pitches by as much, its heading stays.
            assert!((look_pitch(&m) - (look_pitch(&level) + p)).abs() < 1e-4 && shown == p, "{p}: {}", look_pitch(&m));
            assert!((heading(&m) - 0.3).abs() < 1e-4);
            // The eye orbits the pivot above the tank: same distance, lower
            // when looking up, higher when looking down; the tank keeps its
            // place on screen.
            let (eye, eye0): (Vec3, Vec3) = (m.translation.into(), level.translation.into());
            assert!(((eye - pivot).length() - (eye0 - pivot).length()).abs() < 1e-3);
            assert!(if p > 0.0 { eye.z < eye0.z - 0.5 } else { eye.z > eye0.z + 0.5 }, "{p}: {} vs {}", eye.z, eye0.z);
            assert!((below(&m) - below(&level)).abs() < 1e-3, "{p}");
        }
        // All the way up: the eye stops level with the pivot, the view looks
        // well above the horizon, and turns only as far as keeps the tank
        // in the frame (more with a taller view).
        let (m, shown) = cam(FREE_PITCH_RANGE.1);
        let eye: Vec3 = m.translation.into();
        assert!((eye.z - pivot.z.max(FREE_CAM_CLEARANCE)).abs() < 1e-3, "{eye}");
        assert!(look_pitch(&m) > 0.15 && shown < FREE_PITCH_RANGE.1, "{} {shown}", look_pitch(&m));
        assert!((below(&m) - (DEFAULT_HALF_VFOV - FREE_TANK_MARGIN)).abs() < 1e-3);
        let (tall, _) = free_view(Vec3::ZERO, 0.3, FREE_PITCH_RANGE.1, cd.float_height, 0.64, &cd, flat);
        assert!(look_pitch(&tall) > look_pitch(&m) + 0.1);
    }

    #[test]
    fn camera_stays_at_the_mission_floor() {
        // A tank 20 m down in the pit under floor 95: the eye stays at 95.
        let cd = CameraParams { floor: 95.0, ..CameraParams::default() };
        let pit = |_: f32, _: f32| Some(75.0);
        assert_eq!(free_eye_height(Vec3::new(0.0, 0.0, 75.0), 0.0, &cd, pit), 20.0);
        let m = ChaseCam::default().update(0.0, Vec3::new(0.0, 0.0, 75.0), Mat3::IDENTITY, &cd, pit);
        assert_eq!(m.translation.z, 95.0);
    }

    #[test]
    fn free_camera_stays_above_the_terrain() {
        let cd = CameraParams::default();
        // A ridge behind the tank (the camera looks along +y): the ground
        // rises 1 m per metre behind y = -2.
        let ridge = |_: f32, y: f32| Some((-2.0 - y).max(0.0));
        let h = free_eye_height(Vec3::ZERO, 0.0, &cd, ridge);
        let (m0, _) = free_view(Vec3::ZERO, 0.0, 0.0, h, DEFAULT_HALF_VFOV, &cd, ridge);
        let base = look_pitch(&m0);
        let pivot = Vec3::Z * cd.focus_up_dist;
        let mut prev: Option<Vec3> = None;
        for i in 0..=60 {
            let p = FREE_PITCH_RANGE.0 + (FREE_PITCH_RANGE.1 - FREE_PITCH_RANGE.0) * i as f32 / 60.0;
            let (m, _) = free_view(Vec3::ZERO, 0.0, p, h, DEFAULT_HALF_VFOV, &cd, ridge);
            let eye: Vec3 = m.translation.into();
            let g = ridge(eye.x, eye.y).unwrap();
            assert!(eye.z >= g + FREE_CAM_CLEARANCE - 1e-3, "{p}: eye {eye} over ground {g}");
            // Looking down pitches by as much; looking up looks higher than
            // level, with the tank still in the frame.
            if p <= 0.0 {
                assert!((look_pitch(&m) - (base + p)).abs() < 1e-3, "{p}");
            } else {
                assert!(look_pitch(&m) > base, "{p}: {} <= {base}", look_pitch(&m));
                let below = look_pitch(&m) - (pivot - eye).normalize().z.asin();
                assert!(below <= DEFAULT_HALF_VFOV - FREE_TANK_MARGIN + 1e-3, "{p}: tank at {below}");
            }
            if let Some(q) = prev {
                assert!((eye - q).length() < 0.8, "{p}: {q} -> {eye}");
            }
            prev = Some(eye);
        }
    }

    #[test]
    fn predicted_aim_is_continuous_across_a_tick() {
        // Before the tick: tick aim 0.2, mouse 0.05 pending.
        let before = predict_aim((0.2, 0.0), (0.0, 0.0), (0.05, 0.0), 1.0);
        // The tick consumed it: prev 0.2, cur 0.25, alpha ~0.
        let after = predict_aim((0.2, 0.0), (0.05, 0.0), (0.0, 0.0), 0.0);
        assert!((before.0 - after.0).abs() < 1e-6);
        // Late in the tick the lerp has caught up; nothing is counted twice.
        assert!((predict_aim((0.25, 0.0), (0.05, 0.0), (0.0, 0.0), 1.0).0 - 0.25).abs() < 1e-6);
        // Clamped to the turret range.
        assert_eq!(predict_aim((0.98, 0.0), (0.0, 0.0), (0.1, -2.0), 1.0), (1.0, -1.0));
    }

    /// Turret aim drawn while the mouse turns the turret and frames run at
    /// `fps`, with `jitter(frame)` scaling each frame's mouse motion.
    /// Returns the frame-stats judder and the largest lag behind the mouse
    /// (yawN units) of (the mouse itself, tick interpolation only, with the
    /// pending mouse motion drawn).
    #[cfg(feature = "dev")]
    fn turret_motion(fps: f64, jitter: impl Fn(usize) -> f32) -> [(f64, f32); 3] {
        let f = tt_game::fields::Fields::from_pairs("MediumTank", "TankDef", &[("turret_yaw_range", "30"), ("turret_pitch_range", "30"), ("turret_pitch_center", "10")]);
        let data = std::sync::Arc::new(tt_game::TankParams::from_fields(&f, None));
        let mut world = tt_game::World::new(Some(tt_game::HeightField::flat(0.0)));
        let id = world.add_tank(data, Vec3::ZERO, 0.0);
        let mut probes = [crate::framestats::Probe::default(); 3];
        let mut lag = [0.0f32; 3];
        let (mut acc, mut pending, mut consumed, mut t, mut total) = (0.0f64, 0.0f32, 0.0f32, 0.0f64, 0.0f32);
        let mut prev = 0.0f32;
        let dt = 1000.0 / fps;
        for i in 0..(3.0 * fps) as usize {
            // Mouse events between frames.
            pending += 0.5 * (dt / 1000.0) as f32 * jitter(i);
            acc += dt;
            t += dt;
            while acc >= tt_game::TICK_MS as f64 {
                prev = world.tank(id).unwrap().yaw_n;
                let q = tt_game::netmove::u16_to_angle(tt_game::netmove::angle_to_u16(pending));
                consumed = q / std::f32::consts::PI;
                total += consumed;
                world.set_move(id, Move { yaw: pending, ..Move::NULL });
                pending = 0.0;
                world.tick();
                acc -= tt_game::TICK_MS as f64;
            }
            let alpha = (acc / tt_game::TICK_MS as f64) as f32;
            let lerped = lerp(prev, world.tank(id).unwrap().yaw_n, alpha);
            let drawn = predict_aim((lerped, 0.0), (consumed, 0.0), (pending / std::f32::consts::PI, 0.0), alpha).0;
            // Mouse input so far as the sim gets it (16-bit quantised per move).
            let mouse = total + pending / std::f32::consts::PI;
            for (k, v) in [mouse, lerped, drawn].into_iter().enumerate() {
                probes[k].add(Vec3::new(v, 0.0, 0.0), t);
                lag[k] = lag[k].max(mouse - v);
            }
        }
        assert!(world.tank(id).unwrap().yaw_n < 0.99, "stayed off the limit");
        [0, 1, 2].map(|k| (probes[k].judder(), lag[k]))
    }

    #[test]
    #[cfg(feature = "dev")]
    fn mouse_aim_drawn_without_tick_judder() {
        for fps in [60.0, 75.0, 144.0, 240.0] {
            let [_, old, new] = turret_motion(fps, |_| 1.0);
            let [mouse_j, old_j, new_j] = turret_motion(fps, |i| if i % 3 == 0 { 1.3 } else { 0.85 });
            println!("{fps} fps steady mouse: judder {:.2} -> {:.2}, lag {:.4} -> {:.4} | uneven mouse (judder {:.2}): {:.2} -> {:.2}", old.0, new.0, old.1, new.1, mouse_j.0, old_j.0, new_j.0);
            // Steady motion stays steady, and the drawn turret is where the
            // mouse put it.
            assert!(new.0 <= old.0 + 0.01 && new.0 < 0.05, "{fps}: {old:?} -> {new:?}");
            assert!(new.1 < 1e-4 && old.1 > 0.004, "{fps}: lag {old:?} -> {new:?}");
            // Uneven mouse motion is drawn as it came, not made worse.
            assert!((new_j.0 - mouse_j.0).abs() < 0.05, "{fps}: {mouse_j:?} {new_j:?}");
        }
    }
}

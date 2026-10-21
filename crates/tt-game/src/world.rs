//! The simulation world: owns everything and runs the 32 ms tick.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Affine3A, Mat3, Vec3};

use crate::data::{BoostParams, BoostMode, PowerUpParams, ProjectileParams, TankParams, TriggerShape, VolcanoParams};
use crate::geom::{segment_triangle, tri_box_overlap, Aabb};
use crate::pads::{BoostState, Trigger, TriggerKind, VolcanoState};
use crate::projectile::{ballistic, segment_tank, HitObject, Projectile};
use crate::rng::Rng;
use crate::statics::{StaticObject, StaticShape};
use crate::tank::{self, Move, Tank};
use crate::terrain::HeightField;
use crate::{ProjectileId, TankId, TriggerId, DT, TICK_MS};

/// `MissionArea.area`: "x y width height".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionArea {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Something the engine should react to (script callbacks, sounds, effects).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A projectile left the muzzle (play `fire_sound`).
    Fired { tank: TankId, projectile: ProjectileId, def: String },
    /// Trigger pulled without ammo (play `dry_fire_sound`).
    DryFired { tank: TankId },
    ProjectileBounced { projectile: ProjectileId, pos: Vec3, normal: Vec3, hit: HitObject },
    /// The projectile exploded; it stays (stopped) for 1000 ms.
    ProjectileExploded { projectile: ProjectileId, pos: Vec3, normal: Vec3, hit: HitObject, def: String, source: Option<TankId> },
    ProjectileRemoved { projectile: ProjectileId },
    /// `amount` is after the victim's armor_scale.
    Damaged { tank: TankId, amount: f32, source: Option<TankId> },
    /// The tank is now a corpse.
    Killed { tank: TankId, killer: Option<TankId> },
    /// Corpse timer ran out; the tank was deleted.
    TankRemoved { tank: TankId },
    /// Collision response with another tank or a static (relative normal speed).
    Collided { tank: TankId, other: HitObject, speed: f32 },
    /// The tank lost the scrum ball (death or a hard tank collision).
    FlagDropped { tank: TankId, flag: crate::FlagId },
    PowerUpEntered { trigger: TriggerId, tank: TankId, def: String },
    PowerUpLeft { trigger: TriggerId, tank: TankId, def: String },
    PowerUpSwitched { trigger: TriggerId, on: bool },
    /// A boost/jump pad kicked a tank by `dv` (applied over the next ticks).
    PadUsed { trigger: TriggerId, tank: TankId, dv: Vec3 },
    PadStateChanged { trigger: TriggerId, state: usize, name: String },
    VolcanoErupted { trigger: TriggerId, rocks: u32 },
    /// `playUserThread(slot, scale, reset)` started a power-up's user
    /// animation thread (cosmetic; replicated to the clients).
    UserThreadPlayed { trigger: TriggerId, slot: u32, scale: f32 },
    /// A tank picked up the scrum ball (play `snag_sound`).
    FlagPickedUp { flag: crate::FlagId, tank: TankId },
    FlagOutOfBounds { flag: crate::FlagId },
    /// Client cosmetic: brain jar ejected (play `loose_head_sound`); the
    /// projectile carries `shape` (the tank's brain_shape).
    BrainEjected { tank: TankId, projectile: Option<ProjectileId>, shape: String },
}

#[derive(Debug, Clone, Copy)]
struct Contact {
    normal: Vec3,
    other: HitObject,
    other_index: Option<usize>,
}

pub struct World {
    /// Simulation clock in milliseconds ("now" during a tick).
    pub time_ms: i64,
    /// Run the server-only parts (firing, damage, death, boosts, pickups).
    pub server: bool,
    /// Players get no moves.
    pub ai_control_mode: bool,
    /// Team game: team-0 tanks are then teammates too.
    pub team_game: bool,
    /// Quantise player moves like the network does (`netmove::server_move`).
    pub quantize_moves: bool,
    /// Push apart tanks that already interpenetrate.
    pub separate_tanks: bool,
    /// Client cosmetic: eject brain jars from corpses automatically.
    pub auto_eject_brains: bool,
    /// Network client (`server = false`): apply boost/jump pad kicks to the
    /// tanks it steps (prediction); pad states stay the server's.
    pub predict_pads: bool,
    /// Network client: run the firing logic for the tanks it steps (own
    /// shots predicted, stage 6).
    pub predict_shots: bool,
    /// Set while a network client re-runs moves after a correction: the
    /// firing state advances but no projectile or event is produced again.
    pub replay: bool,
    /// Server spawn point positions, in the order the engine keeps
    /// them.
    pub spawn_spheres: Vec<Vec3>,
    /// Scrum balls (the first one is "the" ball the AI looks at).
    pub flags: Vec<crate::flag::Flag>,
    /// The mission's gravity scale (`MissionInfo.gravity`, 1: the game's
    /// [`crate::GRAVITY`]): tanks, shells, balls, brain jars.
    pub gravity: f32,
    pub terrain: Option<HeightField>,
    pub statics: Vec<StaticObject>,
    pub mission_area: Option<MissionArea>,
    /// `MissionArea.floor`: below it a tank is out of bounds.
    pub floor: f32,
    /// World boxes of named mission objects (volcano zone markers).
    pub markers: HashMap<String, Aabb>,
    pub tanks: Vec<Tank>,
    pub projectiles: Vec<Projectile>,
    pub triggers: Vec<Trigger>,
    /// `gRandGen`: everything except the unstuck kicks.
    pub rng: Rng,
    /// `sgRandom(0xDEADBEEF)` of the math utilities: `randomDir` only.
    pub math_rng: Rng,
    /// Server lag compensation of network shots (empty: off).
    pub lag_comp: crate::lagcomp::LagComp,
    next_id: u32,
    pub(crate) events: Vec<Event>,
}

impl Default for World {
    fn default() -> Self {
        World::new(None)
    }
}

impl World {
    pub fn new(terrain: Option<HeightField>) -> World {
        World {
            time_ms: 0,
            server: true,
            ai_control_mode: false,
            team_game: false,
            quantize_moves: true,
            separate_tanks: true,
            auto_eject_brains: false,
            predict_pads: false,
            predict_shots: false,
            replay: false,
            spawn_spheres: Vec::new(),
            flags: Vec::new(),
            gravity: 1.0,
            terrain,
            statics: Vec::new(),
            mission_area: None,
            floor: -100.0,
            markers: HashMap::new(),
            tanks: Vec::new(),
            projectiles: Vec::new(),
            triggers: Vec::new(),
            rng: Rng::default(),
            math_rng: Rng::math_utils(),
            lag_comp: Default::default(),
            next_id: 1,
            events: Vec::new(),
        }
    }

    pub(crate) fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn add_static(&mut self, shape: Arc<StaticShape>, transform: Affine3A) -> usize {
        self.statics.push(StaticObject::new(shape, transform));
        self.statics.len() - 1
    }

    /// Whether a static is solid right now. A PowerUp/Boost/Volcano's
    /// collision meshes only exist while it is on: its `buildPolyList` and
    /// `castRay` test the on flag first, so a picked-up power-up is not an
    /// obstacle until it respawns.
    pub fn static_solid(&self, s: &StaticObject) -> bool {
        s.trigger.is_none_or(|id| self.trigger(id).is_some_and(|t| t.on))
    }

    /// Whether a static blocks tanks (`checkCollisions`). As
    /// [`World::static_solid`], except that tanks drive through weapon and
    /// health pick-ups ([`PowerUpParams::is_pickup`]) and collect them; the
    /// Reload arch, volcano and other PowerUps keep their collision.
    pub fn static_blocks_tanks(&self, s: &StaticObject) -> bool {
        self.static_solid(s) && !s.trigger.and_then(|id| self.trigger(id)).is_some_and(|t| matches!(&t.kind, TriggerKind::PowerUp(d) if d.is_pickup()))
    }

    pub fn add_marker(&mut self, name: &str, world_box: Aabb) {
        self.markers.insert(name.to_ascii_lowercase(), world_box);
    }

    /// Spawns a tank at `pos` facing `yaw`, snapped to the ground like the
    /// engine's spawn. Call [`World::set_projectile`] with the default
    /// projectile afterwards, as the game scripts do.
    pub fn add_tank(&mut self, data: Arc<TankParams>, pos: Vec3, yaw: f32) -> TankId {
        let id = TankId(self.alloc_id());
        let mut t = Tank::new(id, data, pos, crate::geom::rot_z(yaw));
        tank::snap_to_ground(&mut t, self.terrain.as_ref(), self.gravity);
        self.tanks.push(t);
        id
    }

    /// Adds a tank exactly at `pos`/`rot` (no ground snap; e.g. dropped from the air).
    pub fn add_tank_at(&mut self, data: Arc<TankParams>, pos: Vec3, rot: Mat3) -> TankId {
        let id = TankId(self.alloc_id());
        self.tanks.push(Tank::new(id, data, pos, rot));
        id
    }

    /// Container search for tanks: every tank whose world box overlaps
    /// `area`, corpses included, `exclude` left out.
    pub fn tanks_in_box(&self, area: &Aabb, exclude: Option<TankId>) -> Vec<TankId> {
        self.tanks.iter().filter(|t| !t.deleted && Some(t.id) != exclude && t.world_box().overlaps(area)).map(|t| t.id).collect()
    }

    /// Only tanks that are themselves in the air near the drop point (e.g.
    /// spawned moments before at the same point) are found; a tank sitting on
    /// the ground there is 10 m below the first test cube, and the new tank
    /// lands on it from 15 m (tank–tank bounce, unstuck kick).
    pub fn spawn_clearance(&mut self, pos: Vec3, exclude: Option<TankId>) -> Vec3 {
        const DROP: f32 = 15.0;
        const HALF: f32 = 5.0;
        const STEP_Z: f32 = 10.0;
        let mut p = pos;
        p.z += DROP;
        // Bounded only to keep a bad mission from hanging.
        for _ in 0..64 {
            let cube = Aabb::new(p - Vec3::splat(HALF), p + Vec3::splat(HALF));
            if self.tanks_in_box(&cube, exclude).is_empty() {
                break;
            }
            // fild(randI) · 2^-31 · 4 − 2 + pos, on the x87 stack, then stored.
            let dx = self.rng.rand_i() as f64 * (1.0 / 2_147_483_648.0) * 4.0 - 2.0;
            p.x = (p.x as f64 + dx) as f32;
            let dy = self.rng.rand_i() as f64 * (1.0 / 2_147_483_648.0) * 4.0 - 2.0;
            p.y = (p.y as f64 + dy) as f32;
            p.z += STEP_Z;
        }
        p
    }

    /// The heading is kept and flattened (forward = the matrix's Y column
    /// without z; straight up/down gives +Y), the wheel springs are zeroed,
    /// the tank is dropped to the terrain and fitted to it, and on the server
    /// it is then raised by [`World::spawn_clearance`]. `rot = None` keeps
    /// the current rotation.
    pub fn set_tank_transform(&mut self, id: TankId, pos: Vec3, rot: Option<Mat3>) {
        let Some(i) = self.tank_index(id) else { return };
        let t = &mut self.tanks[i];
        let f = rot.unwrap_or(t.rot).y_axis;
        let f = if f.z * f.z > 0.9 { Vec3::Y } else { Vec3::new(f.x, f.y, 0.0).normalize_or(Vec3::Y) };
        t.rot = Mat3::from_cols(Vec3::new(f.y, -f.x, 0.0), f, Vec3::Z);
        t.pos = pos;
        t.wheel_vel = [0.0; 4];
        tank::snap_to_ground(t, self.terrain.as_ref(), self.gravity);
        if self.server {
            let p = self.tanks[i].pos;
            self.tanks[i].pos = self.spawn_clearance(p, Some(id));
        }
    }

    pub fn tank(&self, id: TankId) -> Option<&Tank> {
        self.tanks.iter().find(|t| t.id == id && !t.deleted)
    }

    pub fn tank_mut(&mut self, id: TankId) -> Option<&mut Tank> {
        self.tanks.iter_mut().find(|t| t.id == id && !t.deleted)
    }

    fn tank_index(&self, id: TankId) -> Option<usize> {
        self.tanks.iter().position(|t| t.id == id && !t.deleted)
    }

    pub fn set_move(&mut self, id: TankId, mv: Move) {
        if let Some(t) = self.tank_mut(id) {
            t.input = mv;
        }
    }

    pub fn projectile(&self, id: ProjectileId) -> Option<&Projectile> {
        self.projectiles.iter().find(|p| p.id == id && !p.deleted)
    }

    pub fn trigger(&self, id: TriggerId) -> Option<&Trigger> {
        self.triggers.iter().find(|t| t.id == id)
    }

    /// Console `setProjectile` (§5.3).
    pub fn set_projectile(&mut self, id: TankId, p: Arc<ProjectileParams>) -> bool {
        let now = self.time_ms;
        let Some(t) = self.tank_mut(id) else { return false };
        set_projectile(t, p, now);
        true
    }

    /// `applyDamage(amount, source)` (§5.4).
    pub fn apply_damage(&mut self, id: TankId, amount: f32, source: Option<TankId>) {
        if let Some(i) = self.tank_index(id) {
            let ev = apply_damage(&mut self.tanks[i], amount, source);
            self.events.push(ev);
        }
    }

    /// Console `hurtMe(x)`: `applyDamage(atoi(x), the tank's own id)` — the
    /// tank becomes its own last damage source.
    pub fn hurt_me(&mut self, id: TankId, amount: &str) {
        self.apply_damage(id, crate::fields::atoi(amount) as f32, Some(id));
    }

    /// Marks a tank as driven by a client connection. Such a tank's corpse
    /// is never deleted by the simulation: it stays until the scripts
    /// respawn the player and delete it ([`World::remove_tank`]). Bots and
    /// other uncontrolled tanks are deleted after corpse_fade_time +
    /// corpse_fade_speed.
    pub fn set_controlling_client(&mut self, id: TankId, controlled: bool) {
        if let Some(t) = self.tank_mut(id) {
            t.controlled = controlled;
        }
    }

    /// Deletes a tank (script `delete()`); drops the ball it carries.
    pub fn remove_tank(&mut self, id: TankId) {
        if let Some(i) = self.tank_index(id) {
            let mut t = self.tanks[i].clone();
            self.drop_flag(&mut t);
            self.tanks[i].carried_flag = None;
            self.tanks[i].deleted = true;
            self.events.push(Event::TankRemoved { tank: id });
        }
    }

    fn add_trigger(&mut self, name: &str, transform: Affine3A, shape: TriggerShape, kind: TriggerKind) -> TriggerId {
        let id = TriggerId(self.alloc_id());
        if let Some(col) = shape.collision.clone() {
            let i = self.add_static(col, transform);
            self.statics[i].trigger = Some(id);
        }
        let mut t = Trigger { id, name: name.to_string(), transform, shape, on: true, off_until: 0, kind, thread: None };
        if self.server {
            let start_on = t.power_up_params().starts_on;
            if t.set_on(start_on, self.time_ms, &mut self.rng) {
                self.events.push(Event::PowerUpSwitched { trigger: id, on: t.on });
            }
        }
        self.triggers.push(t);
        id
    }

    pub fn add_power_up(&mut self, name: &str, data: Arc<PowerUpParams>, shape: TriggerShape, transform: Affine3A) -> TriggerId {
        self.add_trigger(name, transform, shape, TriggerKind::PowerUp(data))
    }

    pub fn add_boost(&mut self, name: &str, data: Arc<BoostParams>, shape: TriggerShape, transform: Affine3A) -> TriggerId {
        // onNewDef: state = −1, then setState(0).
        let mut st = BoostState { data, state: usize::MAX, next_state_time: None };
        st.set_state(0, self.time_ms, &mut self.rng);
        self.add_trigger(name, transform, shape, TriggerKind::Boost(st))
    }

    pub fn add_volcano(&mut self, name: &str, data: Arc<VolcanoParams>, shape: TriggerShape, transform: Affine3A) -> TriggerId {
        // onNewDef: focusZone = 0, then schedule: nextAttack = now +
        // randI(min_attack_delay, max_attack_delay).
        let delay = self.rng.rand_range(data.min_attack_delay, data.max_attack_delay) as i64;
        let st = VolcanoState { data, rocks_left: 0, next_attack: self.time_ms + delay, focus_zone: 0 };
        self.add_trigger(name, transform, shape, TriggerKind::Volcano(st))
    }

    pub fn spawn_projectile(&mut self, data: Arc<ProjectileParams>, pos: Vec3, vel: Vec3, source: Option<TankId>, damage_scale: f32) -> ProjectileId {
        let id = ProjectileId(self.alloc_id());
        let splash = data.blast_radius;
        let born_ms = self.time_ms;
        self.projectiles.push(Projectile { id, data, pos, prev_pos: pos, vel, source, damage_scale, bounces: 0, exploded_at: None, cosmetic_since: None, born_ms, deleted: false });
        if !self.lag_comp.view_delay.is_empty() {
            let now = self.time_ms;
            self.lag_comp.fired(id, source, splash, now);
        }
        id
    }

    /// `setTeam`.
    pub fn set_team(&mut self, id: TankId, team: i32) {
        if let Some(t) = self.tank_mut(id) {
            t.team = team;
        }
    }

    /// `setPlayerName`.
    pub fn set_player_name(&mut self, id: TankId, name: &str) {
        if let Some(t) = self.tank_mut(id) {
            t.name = name.to_string();
        }
    }

    /// `isCorpse`.
    pub fn is_corpse(&self, id: TankId) -> bool {
        self.tank(id).is_some_and(|t| t.is_corpse)
    }

    /// `setRandomSeed(seed)`.
    pub fn set_random_seed(&mut self, seed: u32) {
        self.rng.set_seed(seed);
    }

    /// Client death animation: once corpseTime ≥ corpse_shake_time a random
    /// `death%i` sequence starts (randI over the shape's death sequences). The
    /// brain is ejected as it starts, before the tank flips over. A shape
    /// without death sequences never ejects its brain.
    fn advance_death_sequence(&mut self, t: &mut Tank) {
        let shake = t.data.fx.corpse_shake_time;
        if t.corpse_timer < shake {
            return;
        }
        if t.death_sequence.is_none() {
            let n = t.data.death_sequences.len();
            if n == 0 {
                return;
            }
            let k = self.rng.rand_range(0, n as i32 - 1) as usize;
            t.death_sequence = Some((k, t.data.death_sequences[k]));
        }
        self.eject_brain_of(t);
    }

    fn eject_brain_of(&mut self, t: &mut Tank) -> Option<ProjectileId> {
        if !t.is_corpse || t.brain_ejected {
            return None;
        }
        t.brain_ejected = true;
        let g = &t.data.geometry;
        let seat = g.seat_node.map_or(Affine3A::IDENTITY, |n| g.posed_node(n, t.yaw_n, t.pitch_n, &t.data));
        let m = Affine3A::from_mat3_translation(t.rot, t.pos) * seat;
        let up: Vec3 = m.matrix3.z_axis.into();
        let pos: Vec3 = m.translation.into();
        let projectile = t.data.fx.brain_projectile.clone().map(|bp| {
            let vel = t.vel * bp.inherit_velocity + up * bp.velocity;
            // The corpse is the source, so the jar does not hit it on the way out.
            let id = self.spawn_projectile(bp, pos, vel, Some(t.id), 0.0);
            let now = self.time_ms;
            if let Some(p) = self.projectiles.iter_mut().find(|p| p.id == id) {
                p.cosmetic_since = Some(now);
            }
            id
        });
        self.events.push(Event::BrainEjected { tank: t.id, projectile, shape: t.data.brain_shape.clone() });
        projectile
    }

    /// Plays the def's user sequence `slot` on the object's shape once, from the
    /// start (or, with `reset` off while that sequence is already playing, only
    /// rescaled). Returns false (and does nothing) for an unknown trigger or an
    /// empty slot.
    pub fn play_user_thread(&mut self, id: TriggerId, slot: u32, scale: f32, reset: bool) -> bool {
        let now = self.time_ms;
        let Some(t) = self.triggers.iter_mut().find(|t| t.id == id) else { return false };
        let Some(seq) = t.power_up_params().user_seq.get(slot as usize).filter(|s| !s.is_empty()).cloned() else { return false };
        match &mut t.thread {
            Some(th) if !reset && th.slot == slot => th.scale = scale,
            th => *th = Some(crate::pads::UserThread { slot, seq, start_ms: now, scale }),
        }
        self.events.push(Event::UserThreadPlayed { trigger: id, slot, scale });
        true
    }

    /// Queues an event as if the world had produced it (a network client
    /// replays the server's cosmetic events through the normal handlers).
    pub fn push_event(&mut self, e: Event) {
        self.events.push(e);
    }

    /// Takes the events queued outside a tick (damage from `hurt_me` etc.).
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Advances the simulation by one 32 ms tick and returns its events.
    // SPEC-GAP: the engine's process-list order is not in the spec; tanks
    // tick first (in creation order), then projectiles (including the ones
    // fired this tick), then scrum balls, then power-ups/pads/volcanoes.
    pub fn tick(&mut self) -> Vec<Event> {
        for i in 0..self.tanks.len() {
            if self.tanks[i].deleted || self.tanks[i].puppet {
                continue;
            }
            self.tick_tank_at(i);
        }
        let now = self.time_ms;
        self.lag_comp.record(now, self.tanks.iter().filter(|t| !t.deleted).map(|t| (t.id, t.pos, t.rot)));
        self.tick_projectiles();
        self.tick_flags();
        self.tick_triggers();
        self.tanks.retain(|t| !t.deleted);
        self.projectiles.retain(|p| !p.deleted);
        if !self.lag_comp.shots.is_empty() {
            let live: std::collections::HashSet<ProjectileId> = self.projectiles.iter().filter(|p| p.is_live()).map(|p| p.id).collect();
            self.lag_comp.retain_shots(|id| live.contains(&id));
        }
        self.time_ms += TICK_MS;
        std::mem::take(&mut self.events)
    }

    /// Steps tank `i` (taken out of the list so it does not collide with itself).
    fn tick_tank_at(&mut self, i: usize) {
        let mut placeholder = self.tanks[i].clone();
        placeholder.deleted = true;
        let mut t = std::mem::replace(&mut self.tanks[i], placeholder);
        self.tick_tank(&mut t);
        self.tanks[i] = t;
    }

    /// Steps **one** tank by one tick with move `mv` (client-side prediction
    /// and reconciliation, docs/netcode.md §2.2): exactly the tank part of
    /// [`World::tick`] — other tanks (puppets included) are obstacles,
    /// projectiles, flags and triggers are not stepped, and `time_ms` is
    /// not advanced (the caller owns the clock). `mv` is stored as the
    /// tank's input, so it goes through the same quantisation as a tick.
    /// Steps puppets too (the caller asked for this tank). Returns the
    /// events of the step; a tank that ends up deleted is removed.
    ///
    /// With `server = false` it draws nothing from `rng`/`math_rng`.
    pub fn step_tank(&mut self, id: TankId, mv: Move) -> Vec<Event> {
        let Some(i) = self.tank_index(id) else { return Vec::new() };
        if self.tanks[i].deleted {
            return Vec::new();
        }
        let queued = std::mem::take(&mut self.events);
        self.tanks[i].input = mv;
        self.tick_tank_at(i);
        self.tanks.retain(|t| !t.deleted);
        std::mem::replace(&mut self.events, queued)
    }

    // ---------------------------------------------------------------- tanks

    fn tick_tank(&mut self, t: &mut Tank) {
        let mv: Option<Move> = if t.is_bot() {
            // Bots run on the server only (`move = ai.getMove()`).
            self.server.then(|| self.bot_move(t))
        } else if self.ai_control_mode {
            None
        } else if self.quantize_moves {
            Some(crate::netmove::server_move(&t.input))
        } else {
            Some(t.input)
        };
        t.prev_vel = t.vel;
        if self.separate_tanks && t.corpse_timer <= t.data.fx.corpse_fade_time {
            let push = self.separation_push(t);
            t.pos += push;
        }
        let (start_pos, start_rot) = (t.pos, t.rot);
        let misc = t.data.misc.clone();

        if t.boost_counter != 0 {
            if t.boost_counter <= misc.boost_ticks {
                t.vel += t.boost_impulse;
            }
            t.boost_counter -= 1;
        }

        if t.any_contact || t.vel.length_squared() >= 0.1 || t.is_corpse {
            t.stuck_ticks = 0;
        } else {
            t.stuck_ticks += 1;
            if t.stuck_ticks * TICK_MS as i32 > misc.unstuck_time {
                t.wheel_vel = [0.0; 4];
                if self.server {
                    // randomDir((0,0,1), 0°..90°, 0°..360°) from the math-utils generator.
                    let dir = crate::rng::random_dir(&mut self.math_rng, Vec3::Z, 0.0, 90.0, 0.0, 360.0);
                    t.vel += dir * misc.unstuck_boost;
                }
            }
        }

        let drive = if t.is_corpse { Move::NULL } else { mv.unwrap_or(Move::NULL) };
        tank::update_move(t, &drive, self.terrain.as_ref(), self.gravity);
        if t.corpse_timer <= t.data.fx.corpse_fade_time && !self.check_collisions(t, start_pos, start_rot) {
            t.pos = start_pos + t.vel * DT;
            if !self.check_collisions(t, start_pos, start_rot) {
                t.vel = Vec3::ZERO;
                t.wheel_vel = [0.0; 4];
            }
        }

        self.handle_triggers(t);
        self.flag_triggers(t);

        if let (Some(mv), false) = (mv, t.is_corpse) {
            tank::update_turret(t, &mv, start_rot);
            if self.server || self.predict_shots {
                self.update_fire(t, &mv);
            }
        }

        if self.server && !t.is_corpse && self.death_check(t) {
            t.is_corpse = true;
            self.drop_flag(t);
        }
        if self.server && t.is_corpse {
            t.corpse_timer += DT;
        }
        if self.auto_eject_brains && t.is_corpse && !t.brain_ejected {
            self.advance_death_sequence(t);
        }
        if self.server && !t.controlled && t.corpse_timer > t.data.fx.corpse_fade_time + t.data.fx.corpse_fade_speed {
            t.deleted = true;
            self.events.push(Event::TankRemoved { tank: t.id });
        }
    }

    /// Spawns no longer start that way, but crowds and landings still can. This pushes the
    /// tank half the remaining overlap (at most 0.25 m per tick) horizontally away from each
    /// overlapping tank; both tanks do it, so they part within a few ticks. Exactly coincident
    /// centres separate along a direction derived from the tank ids.
    fn separation_push(&self, t: &Tank) -> Vec3 {
        let c = t.center();
        let r = t.data.geometry.small_radius();
        let mut push = Vec3::ZERO;
        for o in &self.tanks {
            if o.deleted || o.id == t.id || (o.is_corpse && o.corpse_timer > o.data.fx.corpse_fade_time) {
                continue;
            }
            let d = c - o.center();
            let depth = r + o.data.geometry.small_radius() - d.length();
            if depth <= 0.01 {
                continue;
            }
            let mut dir = Vec3::new(d.x, d.y, 0.0);
            if dir.length_squared() < 1e-6 {
                let a = (t.id.0.min(o.id.0) as f32) * 2.399_963;
                let s = if t.id.0 < o.id.0 { 1.0 } else { -1.0 };
                dir = Vec3::new(a.cos(), a.sin(), 0.0) * s;
            }
            push += dir.normalize() * (0.5 * depth).min(0.25);
        }
        push
    }

    /// §3.5 swept collision against other tanks and statics. On a hit the
    /// tank is moved to the last clear eighth of the move and the velocities
    /// get the restitution impulse. Returns false only when the tank could
    /// not move at all (clear fraction 0), as the engine does.
    fn check_collisions(&mut self, t: &mut Tank, start_pos: Vec3, start_rot: Mat3) -> bool {
        let g = &t.data.geometry;
        let big_r = g.big_radius();
        let r_self = g.small_radius();
        let bounds = g.bounds;
        let start_box = bounds.transformed(&Affine3A::from_mat3_translation(start_rot, start_pos)).grow(big_r);
        let d = t.pos - start_pos;
        let mut swept = start_box;
        for i in 0..3 {
            if d[i] >= 0.0 {
                swept.max[i] += d[i];
            } else {
                swept.min[i] += d[i];
            }
        }

        let others: Vec<usize> = (0..self.tanks.len())
            .filter(|&j| {
                let o = &self.tanks[j];
                !o.deleted && o.id != t.id && !(o.is_corpse && o.corpse_timer > o.data.fx.corpse_fade_time) && o.world_box().overlaps(&swept)
            })
            .collect();
        let mut tris: Vec<([Vec3; 3], Vec3, usize)> = Vec::new();
        for (si, s) in self.statics.iter().enumerate() {
            if !s.world_box.overlaps(&swept) || !self.static_blocks_tanks(s) {
                continue;
            }
            for (tri, n) in s.triangles.iter().zip(&s.normals) {
                if Aabb::from_points(tri.iter().copied()).overlaps(&swept) {
                    tris.push((*tri, *n, si));
                }
            }
        }
        if others.is_empty() && tris.is_empty() {
            return true;
        }

        let (end_pos, end_rot) = (t.pos, t.rot);
        let (q0, q1) = (glam::Quat::from_mat3(&start_rot).normalize(), glam::Quat::from_mat3(&end_rot).normalize());
        let pose = |k: u32| {
            let f = k as f32 * 0.125;
            (start_pos + (end_pos - start_pos) * f, Mat3::from_quat(q0.slerp(q1, f)))
        };
        let vel_dir = t.vel.normalize_or_zero();
        let half = bounds.half_extents();
        let sample = |k: u32| -> Option<Contact> {
            let (p, r) = pose(k);
            let c = p + r * bounds.center();
            let mut best: Option<(f32, Contact)> = None;
            for &j in &others {
                let o = &self.tanks[j];
                let oc = o.center();
                let depth = r_self + o.data.geometry.small_radius() - (c - oc).length();
                if depth > 0.0 && best.is_none_or(|(bd, _)| depth > bd) {
                    let normal = (c - oc).normalize_or(Vec3::Z);
                    best = Some((depth, Contact { normal, other: HitObject::Tank(o.id), other_index: Some(j) }));
                }
            }
            if let Some((_, ct)) = best {
                return Some(ct);
            }
            let inv = r.transpose();
            let mut best: Option<(f32, Contact)> = None;
            for (tri, n, si) in &tris {
                let local = tri.map(|v| inv * (v - c));
                if !tri_box_overlap(half, local) {
                    continue;
                }
                let dot = n.dot(vel_dir);
                if dot <= 0.0 && best.is_none_or(|(bd, _)| dot < bd) {
                    best = Some((dot, Contact { normal: *n, other: HitObject::Static(*si), other_index: None }));
                }
            }
            best.map(|(_, c)| c)
        };

        // 3-bit binary search over k/8 (bit pattern of the engine loop).
        let mut results: [Option<Contact>; 9] = [None; 9];
        let mut k: u32 = 8;
        let mut step: u32 = 8;
        loop {
            let hit = sample(k);
            results[k as usize] = hit;
            if hit.is_none() {
                if k == 8 {
                    return true;
                }
            } else {
                k ^= step;
            }
            step >>= 1;
            k |= step;
            if step == 0 {
                break;
            }
        }
        if k == 0 {
            if let Some(ai) = &mut t.ai {
                ai.bumped = true;
            }
        }
        let (p, r) = pose(k);
        t.pos = p;
        t.rot = r;
        let Some(contact) = results[k as usize + 1] else { return k != 0 };

        let n = contact.normal;
        let vs = n * t.vel.dot(n);
        let (vo, bounce) = match contact.other_index {
            Some(j) => (n * self.tanks[j].vel.dot(n), t.data.misc.bounce_tank),
            None => (-vs, t.data.misc.bounce_static),
        };
        let b = (bounce + 1.0) * 0.5;
        let j_imp = (vs - vo) * b;
        let speed = (vs - vo).length();
        t.vel -= j_imp;
        if let Some(j) = contact.other_index {
            // A puppet's state comes from snapshots: it pushes back but takes
            // no impulse locally (the server's collision is authoritative).
            if !self.tanks[j].puppet {
                self.tanks[j].vel += j_imp;
            }
            // Knocks the ball loose when the impulse (the closing speed
            // along the normal times (1 + bounce_tank)/2), not the closing
            // speed, beats the carrier's hardTankCollVel.
            let hard = |c: &Tank| j_imp.length() > c.data.fx.hard_tank_coll_vel;
            if self.server && t.carried_flag.is_some() {
                if hard(t) {
                    self.drop_flag(t);
                }
            } else if self.server && hard(&self.tanks[j]) {
                if let Some(flag) = self.tanks[j].carried_flag.take() {
                    self.release_flag(flag, self.tanks[j].id);
                }
            }
        }
        self.events.push(Event::Collided { tank: t.id, other: contact.other, speed });
        k != 0
    }

    /// §6.1: PowerUp/Boost/Volcano overlap, onEnter once per entry, onLeave
    /// when nothing overlaps any more.
    fn handle_triggers(&mut self, t: &mut Tank) {
        let r_tank = t.data.geometry.big_radius();
        let now = self.time_ms;
        let mut any = false;
        for ti in 0..self.triggers.len() {
            if !self.triggers[ti].overlaps(t.pos, r_tank) {
                continue;
            }
            any = true;
            if self.triggers[ti].on && t.current_trigger.is_none() {
                self.trigger_enter(ti, t, now);
                t.current_trigger = Some(self.triggers[ti].id);
            }
        }
        if !any {
            if let Some(id) = t.current_trigger.take() {
                if let Some(tr) = self.trigger(id) {
                    let def = tr.def_name().to_string();
                    if self.server && !matches!(tr.kind, TriggerKind::Boost(_)) {
                        self.events.push(Event::PowerUpLeft { trigger: id, tank: t.id, def });
                    }
                }
            }
        }
    }

    fn trigger_enter(&mut self, ti: usize, t: &mut Tank, now: i64) {
        if !self.server {
            // Pad prediction: the same kick, no state change, no event.
            if let (true, TriggerKind::Boost(st)) = (self.predict_pads, &self.triggers[ti].kind) {
                let d = self.triggers[ti].boost_direction();
                let k = d * st.velocity();
                let dv = match st.data.mode {
                    BoostMode::Override => k - t.vel,
                    BoostMode::OverrideDir => k - d * t.vel.dot(d),
                    BoostMode::Additive => k,
                };
                apply_boost(t, dv);
            }
            return;
        }
        let tr = &mut self.triggers[ti];
        let id = tr.id;
        if let TriggerKind::Boost(st) = &tr.kind {
            let d = tr.boost_direction();
            let k = d * st.velocity();
            let dv = match st.data.mode {
                BoostMode::Override => k - t.vel,
                BoostMode::OverrideDir => k - d * t.vel.dot(d),
                BoostMode::Additive => k,
            };
            apply_boost(t, dv);
            if let TriggerKind::Boost(st) = &mut tr.kind {
                if st.data.advance_delay_on_enter != 0 {
                    st.next_state_time = Some(now + st.data.advance_delay_on_enter as i64);
                }
            }
            self.events.push(Event::PadUsed { trigger: id, tank: t.id, dv });
        } else {
            let def = tr.def_name().to_string();
            self.events.push(Event::PowerUpEntered { trigger: id, tank: t.id, def });
            if tr.set_on(false, now, &mut self.rng) {
                self.events.push(Event::PowerUpSwitched { trigger: id, on: false });
            }
        }
    }

    /// §5.2 firing, server only.
    fn update_fire(&mut self, t: &mut Tank, mv: &Move) {
        let now = self.time_ms;
        let default = t.data.default_projectile.clone();
        if !mv.trigger[0] {
            t.trigger_released = true;
        } else {
            let mut rate = t.data.fire_scale;
            if let Some(target) = self.bot_target_pos(t) {
                let d = (target - t.pos).abs();
                let d = d.x + d.y + d.z;
                rate *= if d > 400.0 { 10.0 } else { 1.0 + 3.0 * d / 400.0 };
            }
            let p = if t.ammo > 0 { t.current.clone() } else { default.clone() };
            if let Some(p) = p {
                if t.trigger_released && (now as f32) > t.last_fire as f32 + p.volley_delay_ms as f32 * rate {
                    t.burst_left = p.volley_size;
                }
                if t.burst_left > 0 && now > t.last_fire + (p.reload_ms as f32 * rate) as i64 {
                    t.trigger_released = false;
                    t.burst_left -= 1;
                    t.last_fire = now;
                    let effects = self.server || !self.replay;
                    if t.ammo == 0 {
                        t.dry_fire_count += 1;
                        if effects {
                            self.events.push(Event::DryFired { tank: t.id });
                        }
                    } else {
                        t.ammo -= 1;
                        t.fire_count += 1;
                        if effects {
                            let m = t.muzzle();
                            // One shell along the barrel; the scatter shot's pellets and
                            // the mortar's high angle (`ProjectileParams::launch_dirs`,
                            // its speed `launch_speed`). One `Fired` per shot (its first
                            // projectile).
                            let mut first = None;
                            let speed = p.launch_speed(t.pitch_n, self.gravity);
                            for fwd in p.launch_dirs(&m, t.pitch_n, t.fire_count) {
                                let vel = fwd * speed + t.vel * p.inherit_velocity;
                                let id = self.spawn_projectile(p.clone(), m.translation.into(), vel, Some(t.id), t.data.damage_scale);
                                first.get_or_insert(id);
                            }
                            if let Some(id) = first {
                                self.events.push(Event::Fired { tank: t.id, projectile: id, def: p.name.clone() });
                            }
                        }
                    }
                }
            }
        }
        if t.ammo == 0 {
            if self.server || self.predict_shots {
                t.current = t.saved.take();
                t.ammo = t.saved_ammo;
                t.saved_ammo = 0;
            }
            if t.ammo == 0 {
                t.current = default.clone();
                if t.is_bot() {
                    t.ammo = default.as_ref().map_or(0, |d| d.rounds);
                }
            }
        }
    }

    /// §5.5 death / out of bounds / repair. Returns true when destroyed.
    fn death_check(&mut self, t: &mut Tank) -> bool {
        let a = t.data.clone();
        let mut destroyed = false;
        if t.damage > a.damage_amount - 0.01 {
            self.events.push(Event::Killed { tank: t.id, killer: t.last_damage_source });
            destroyed = true;
        }
        let outside = self.mission_area.is_some_and(|m| {
            t.pos.x < m.x - 5.0 || t.pos.x > m.x + m.w + 5.0 || t.pos.y < m.y - 5.0 || t.pos.y > m.y + m.h + 5.0
        });
        if (t.pos.z <= self.floor || outside) && !a.name.eq_ignore_ascii_case("BossTank") {
            t.damage = (t.damage + 0.1).min(a.damage_amount);
        }
        if t.damage > 0.0 {
            t.damage = (t.damage - a.misc.repair_rate * DT).max(0.0);
        }
        destroyed
    }

    // ---------------------------------------------------------- projectiles

    /// Nearest point where the segment a→b hits the terrain, a tank (but
    /// `ignore`) or a solid static.
    pub fn segment_hit_point(&self, a: Vec3, b: Vec3, ignore: Option<TankId>) -> Option<Vec3> {
        self.ray_cast(a, b, ignore).map(|h| h.1)
    }

    /// Nearest hit of the segment a→b against terrain, tanks and statics.
    pub(crate) fn ray_cast(&self, a: Vec3, b: Vec3, ignore: Option<TankId>) -> Option<(f32, Vec3, Vec3, HitObject)> {
        self.ray_cast_back(a, b, ignore, 0.0)
    }

    /// [`World::ray_cast`] with the tanks where they were `back_ms` ago
    /// (lag compensation, [`crate::lagcomp`]); 0 = now.
    fn ray_cast_back(&self, a: Vec3, b: Vec3, ignore: Option<TankId>, back_ms: f32) -> Option<(f32, Vec3, Vec3, HitObject)> {
        let mut best: Option<(f32, Vec3, Vec3, HitObject)> = None;
        let mut consider = |t: f32, n: Vec3, obj: HitObject| {
            if best.is_none_or(|(bt, ..)| t < bt) {
                best = Some((t, a + (b - a) * t, n, obj));
            }
        };
        if let Some(ter) = &self.terrain {
            if let Some((t, _, n)) = ter.segment_hit(a, b) {
                consider(t, n, HitObject::Terrain);
            }
        }
        let seg_box = Aabb::from_points([a, b]);
        for tk in &self.tanks {
            if tk.deleted || Some(tk.id) == ignore {
                continue;
            }
            let past = if back_ms > 0.0 { self.lag_comp.pose_at(tk.id, self.time_ms as f32 - back_ms) } else { None };
            match past {
                None => {
                    if !tk.world_box().overlaps(&seg_box) {
                        continue;
                    }
                    if let Some((t, n)) = segment_tank(a, b, tk) {
                        consider(t, n, HitObject::Tank(tk.id));
                    }
                }
                Some((pos, rot)) => {
                    let bx = tk.data.geometry.bounds;
                    if !bx.transformed(&glam::Affine3A::from_mat3_translation(rot, pos)).overlaps(&seg_box) {
                        continue;
                    }
                    let inv = rot.transpose();
                    if let Some((t, n)) = crate::geom::segment_aabb(inv * (a - pos), inv * (b - pos), &bx) {
                        consider(t, rot * n, HitObject::Tank(tk.id));
                    }
                }
            }
        }
        let dir = b - a;
        for (si, s) in self.statics.iter().enumerate() {
            if !s.world_box.overlaps(&seg_box) || !self.static_solid(s) {
                continue;
            }
            for (tri, n) in s.triangles.iter().zip(&s.normals) {
                if let Some(t) = segment_triangle(a, b, *tri) {
                    let n = if n.dot(dir) > 0.0 { -*n } else { *n };
                    consider(t, n, HitObject::Static(si));
                }
            }
        }
        best
    }

    fn tick_projectiles(&mut self) {
        let now = self.time_ms;
        for i in 0..self.projectiles.len() {
            let p = &self.projectiles[i];
            if p.deleted {
                continue;
            }
            if p.cosmetic_since.is_some_and(|t| now > t + 5000) {
                let id = p.id;
                self.projectiles[i].deleted = true;
                self.events.push(Event::ProjectileRemoved { projectile: id });
                continue;
            }
            if let Some(t) = p.exploded_at {
                let id = p.id;
                // Deleted once now > explosion time + 1000. A network client
                // deletes its own copies the same way.
                if now > t + 1000 {
                    self.projectiles[i].deleted = true;
                    self.events.push(Event::ProjectileRemoved { projectile: id });
                }
                continue;
            }
            // `max_life_ms`: the scatter shot's pellets run out (no
            // explosion, no damage; every copy removes its own).
            if p.data.max_life_ms > 0 && now >= p.born_ms + p.data.max_life_ms as i64 {
                let id = p.id;
                self.projectiles[i].deleted = true;
                self.events.push(Event::ProjectileRemoved { projectile: id });
                continue;
            }
            let data = p.data.clone();
            let r = data.radius;
            let (pos, mut vel) = (p.pos, p.vel);
            // The end point uses the old velocity, then vel.z −=
            // gravity_scale·0.32, then the ray cast and bounce work on that
            // post-gravity velocity.
            let end = pos + vel * DT;
            vel.z -= data.fall(self.gravity) * DT;
            // Ray lowered by r on z: 0.5·sprite_scale for sprites, half the
            // shape height for.dts.
            let off = Vec3::new(0.0, 0.0, r);
            // The source tank is ignored until the first bounce. Here the
            // source is skipped and the ray may still hit what lies behind
            // it in the same tick.
            let ignore = if p.bounces == 0 { p.source } else { None };
            // Lag compensation: tanks as the (remote) shooter saw them.
            let back = self.lag_comp.rewind_ms(p.id, now);
            let hit = self.ray_cast_back(pos - off, end - off, ignore, back);
            let (id, source, scale, bounces) = (p.id, p.source, p.damage_scale, p.bounces);
            let mut new_pos = end;
            if let Some((_, h, n, obj)) = hit {
                let splash = data.blast_radius;
                let proximity = splash >= 0.1
                    && self.tanks.iter().any(|tk| !tk.deleted && (tk.pos - h).length() < 0.7 * tk.data.geometry.big_radius() + splash);
                let cosmetic = self.projectiles[i].cosmetic_since.is_some();
                let explode = !cosmetic && (bounces == data.bounces || matches!(obj, HitObject::Tank(_)) || proximity);
                if explode {
                    if self.server {
                        let amount = data.damage * scale;
                        if splash >= 0.1 {
                            let cube = Aabb::new(h - Vec3::splat(splash), h + Vec3::splat(splash));
                            for ti in 0..self.tanks.len() {
                                if !self.tanks[ti].deleted && self.tanks[ti].world_box().overlaps(&cube) {
                                    let ev = apply_damage(&mut self.tanks[ti], amount, source);
                                    self.events.push(ev);
                                }
                            }
                        } else if let HitObject::Tank(tid) = obj {
                            if let Some(ti) = self.tank_index(tid) {
                                let ev = apply_damage(&mut self.tanks[ti], amount, source);
                                self.events.push(ev);
                            }
                        }
                    }
                    let p = &mut self.projectiles[i];
                    p.pos = h + off;
                    p.vel = Vec3::ZERO;
                    p.exploded_at = Some(now);
                    self.events.push(Event::ProjectileExploded { projectile: id, pos: h, normal: n, hit: obj, def: data.name.clone(), source });
                    continue;
                }
                // Bounce. Back-off along the ray: the step is normalised only
                // when |d|² > 0.01, so a slow projectile (< 3.125 m/s) backs
                // off by 0.05·d instead.
                let d = end - pos;
                let dir = if d.length_squared() > 0.01 { d.normalize() } else { d };
                new_pos = h - dir * 0.05 + off;
                // Vel −= (bounce_factor + 1)·(n·vel)·n on the post-gravity
                // velocity: restitution = bounce_factor on the normal part,
                // tangential part kept, no friction, no minimum speed. The rest
                // of the tick is not re-flown.
                vel -= n * ((1.0 + data.bounce_factor) * vel.dot(n));
                self.projectiles[i].bounces += 1;
                self.events.push(Event::ProjectileBounced { projectile: id, pos: h, normal: n, hit: obj });
            }
            let p = &mut self.projectiles[i];
            p.prev_pos = p.pos;
            p.pos = new_pos;
            p.vel = vel;
            if p.pos.abs().max_element() > 10000.0 {
                p.deleted = true;
                self.events.push(Event::ProjectileRemoved { projectile: id });
            }
        }
    }

    // ------------------------------------------------------------- triggers

    fn tick_triggers(&mut self) {
        if !self.server {
            return;
        }
        let now = self.time_ms;
        for i in 0..self.triggers.len() {
            let tr = &mut self.triggers[i];
            let id = tr.id;
            if !tr.on && tr.off_until < now && tr.set_on(true, now, &mut self.rng) {
                self.events.push(Event::PowerUpSwitched { trigger: id, on: true });
            }
            match &mut tr.kind {
                TriggerKind::Boost(st) => {
                    // server, on, now > nextStateTime (−1 = never).
                    if tr.on && st.next_state_time.is_some_and(|t| now > t) {
                        let next = (st.state + 1) % st.data.states();
                        if st.set_state(next, now, &mut self.rng) {
                            let name = st.data.state_name[st.state.min(crate::data::MAX_STATES - 1)].clone();
                            self.events.push(Event::PadStateChanged { trigger: id, state: st.state, name });
                        }
                    }
                }
                TriggerKind::Volcano(_) => self.tick_volcano(i, now),
                TriggerKind::PowerUp(_) => {}
            }
        }
    }

    /// Volcano tick.
    fn tick_volcano(&mut self, i: usize, now: i64) {
        let on = self.triggers[i].on;
        let TriggerKind::Volcano(st) = &mut self.triggers[i].kind else { return };
        if st.rocks_left == 0 {
            // A new attack starts; the first rocks fly next tick.
            if on && now > st.next_attack {
                st.rocks_left = self.rng.rand_range(st.data.min_attack_rocks, st.data.max_attack_rocks);
            }
            return;
        }
        // n = min(1 + randI(0, 1), rocksLeft): ftol(2.0) − 1 = 1 is the span.
        let n = (1 + self.rng.rand_range(0, 1)).min(st.rocks_left);
        self.erupt(i, n);
        let TriggerKind::Volcano(st) = &mut self.triggers[i].kind else { return };
        if st.rocks_left == 0 {
            let zones = st.data.num_zones.max(1) as usize;
            st.focus_zone = (st.focus_zone + 1) % zones;
            st.next_attack = now + self.rng.rand_range(st.data.min_attack_delay, st.data.max_attack_delay) as i64;
        }
    }

    fn erupt(&mut self, i: usize, n: i32) {
        let tr = &self.triggers[i];
        let TriggerKind::Volcano(st) = &tr.kind else { return };
        let st = st.clone();
        let id = tr.id;
        let origin = tr.volcano_origin();
        if let TriggerKind::Volcano(s) = &mut self.triggers[i].kind {
            s.rocks_left -= n;
        }
        let Some(rock) = st.data.projectile.clone() else { return };
        // The box is grown by the *focus* zone's radius (as coded), whichever zone is picked.
        let radius = st.data.zones[st.focus_zone.min(crate::data::MAX_STATES - 1)].radius;
        let mut fired = 0;
        for _ in 0..n {
            let Some(z) = st.pick_zone(&mut self.rng) else { break };
            let Some(bx) = self.markers.get(&st.data.zones[z].name.to_ascii_lowercase()).copied() else { break };
            let bx = bx.grow(radius);
            let r = Vec3::new(self.rng.rand_f(), self.rng.rand_f(), self.rng.rand_f());
            let target = bx.min + (bx.max - bx.min) * r;
            let vel = ballistic(origin, target, rock.velocity, rock.fall(self.gravity), &mut self.rng);
            // No shooter: damage_scale keeps the Projectile constructor's 1.0.
            self.spawn_projectile(rock.clone(), origin, vel, None, 1.0);
            fired += 1;
        }
        self.events.push(Event::VolcanoErupted { trigger: id, rocks: fired });
    }
}

/// §5.3 `setProjectile`.
pub fn set_projectile(t: &mut Tank, p: Arc<ProjectileParams>, now: i64) {
    let same = |a: &Option<Arc<ProjectileParams>>| a.as_ref().is_some_and(|a| a.name.eq_ignore_ascii_case(&p.name));
    if same(&t.saved) && !same(&t.current) {
        t.saved_ammo = p.rounds;
        return;
    }
    if p.keep_previous_weapon {
        if let Some(cur) = &t.current {
            if cur.can_be_kept && !same(&t.current) {
                t.saved = Some(cur.clone());
                t.saved_ammo = t.ammo;
            }
        }
    } else {
        t.saved = None;
        t.saved_ammo = 0;
    }
    t.ammo = p.rounds;
    t.current = Some(p);
    t.last_fire = now;
    t.trigger_released = true;
    t.burst_left = 0;
}

/// §5.4 `applyDamage`. The bot hit counters follow: with a source, the
/// same-attacker streak grows on a repeated damaging hit and resets
/// otherwise, and every damaging hit adds to the recent-hit counter.
pub fn apply_damage(t: &mut Tank, amount: f32, source: Option<TankId>) -> Event {
    let scaled = amount * t.data.armor_scale;
    t.damage = (t.damage + scaled).min(t.data.damage_amount);
    if t.damage <= 0.0 {
        t.damage = 0.0;
    }
    if source.is_some() {
        let repeat = t.last_damage_source == source && amount > 0.0;
        if let Some(bot) = &mut t.ai {
            if repeat {
                bot.streak += 1.0;
            } else {
                bot.streak = 0.0;
            }
            if amount > 0.0 {
                bot.hits += 1.0;
            }
        }
    }
    t.last_damage_source = source;
    Event::Damaged { tank: t.id, amount: scaled, source }
}

/// `applyBoost(dv)`: ignored while a boost is running.
pub fn apply_boost(t: &mut Tank, dv: Vec3) {
    if t.boost_counter != 0 {
        return;
    }
    let ticks = t.data.misc.boost_ticks.max(1);
    t.boost_impulse = dv / ticks as f32;
    t.boost_counter = 2 * ticks;
    t.wheel_vel = [0.0; 4];
}

//! The game's entities in the world: triggers, tanks, balls, their defs
//! and transforms.

use super::{fields_transform, object_transform, parse_transform, set_transform_fields, tank_script_transform, GameSim};
use glam::{Affine3A, Mat3, Quat, Vec3};
use std::sync::Arc;
use tt_assets::mission::SceneObject;
use tt_game::fmt::format_g;
use tt_game::{ProjectileParams, TankId, TankParams, TriggerShape};
use tt_gameplay::{EntityId, EntityKind};

impl GameSim {
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

    /// `setTransform` on a tank ([`World::set_tank_transform`](tt_game::World::set_tank_transform)), a ball or
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
}

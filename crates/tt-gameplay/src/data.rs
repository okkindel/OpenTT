//! The game's data, typed ([`GameData`]): every def the data files
//! (and a mission) declared, as a definition of its kind with the values
//! the game uses and its references to other definitions resolved to
//! [`Ref`]s when it is built. A reference to a name that is not declared
//! (of the right kind) is a [`DataError`]; the reference is then unset.
//!
//! It is built from the declarations after their parents were copied in
//! ([`tt_assets::data::defs::DefSet`]), so a definition holds
//! the values the simulation gets too. [`GameData::fingerprint`] is a
//! canonical hash of every gameplay definition (sounds left out: they are
//! presentation and a dedicated server has no sound channel for them),
//! what a network client compares with the server's.

use std::collections::HashMap;
use std::marker::PhantomData;

use tt_assets::mission::SceneObject;
use tt_game::fmt::atof;

/// A definition of kind `T`: its index in [`GameData`]'s table of `T`.
pub struct Ref<T> {
    index: u16,
    _kind: PhantomData<fn() -> T>,
}

impl<T> Ref<T> {
    fn new(index: usize) -> Ref<T> {
        Ref { index: index as u16, _kind: PhantomData }
    }

    pub fn index(self) -> usize {
        self.index as usize
    }
}

impl<T> Clone for Ref<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Ref<T> {}
impl<T> PartialEq for Ref<T> {
    fn eq(&self, o: &Self) -> bool {
        self.index == o.index
    }
}
impl<T> Eq for Ref<T> {}
impl<T> std::hash::Hash for Ref<T> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.index.hash(h)
    }
}
impl<T> std::fmt::Debug for Ref<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ref({})", self.index)
    }
}

/// A definition's name.
pub trait Named {
    fn name(&self) -> &str;
}

/// The definitions of one kind, by name (any case).
#[derive(Debug, Clone)]
pub struct Table<T> {
    items: Vec<T>,
    by_name: HashMap<String, usize>,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Table { items: Vec::new(), by_name: HashMap::new() }
    }
}

impl<T: Named> Table<T> {
    fn push(&mut self, t: T) {
        let key = t.name().to_ascii_lowercase();
        match self.by_name.get(&key) {
            // A later declaration of a name replaces the earlier.
            Some(&i) => self.items[i] = t,
            None => {
                self.by_name.insert(key, self.items.len());
                self.items.push(t);
            }
        }
    }

    /// The definition of that name.
    pub fn find(&self, name: &str) -> Option<Ref<T>> {
        if name.is_empty() {
            return None;
        }
        self.by_name.get(&name.to_ascii_lowercase()).map(|&i| Ref::new(i))
    }

    pub fn get(&self, r: Ref<T>) -> &T {
        &self.items[r.index()]
    }

    /// The definition of that name, if any.
    pub fn named(&self, name: &str) -> Option<&T> {
        self.find(name).map(|r| self.get(r))
    }

    pub fn iter(&self) -> impl Iterator<Item = (Ref<T>, &T)> {
        self.items.iter().enumerate().map(|(i, t)| (Ref::new(i), t))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T> std::ops::Index<Ref<T>> for Table<T> {
    type Output = T;
    fn index(&self, r: Ref<T>) -> &T {
        &self.items[r.index()]
    }
}

macro_rules! named {
    ($($t:ty),*) => {$(
        impl Named for $t {
            fn name(&self) -> &str {
                &self.name
            }
        }
    )*};
}

/// A sound's playback settings.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundDescDef {
    pub name: String,
    pub volume: f32,
    pub looping: bool,
    pub is_3d: bool,
}

/// A sound.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundDef {
    pub name: String,
    pub file: String,
    pub description: Option<Ref<SoundDescDef>>,
}

/// An explosion: its shape and sound.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplosionDef {
    pub name: String,
    pub resource: String,
    pub sound: Option<Ref<SoundDef>>,
}

/// A particle's look.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleDef {
    pub name: String,
    pub texture: String,
}

/// A particle emitter.
#[derive(Debug, Clone, PartialEq)]
pub struct EmitterDef {
    pub name: String,
    pub particles: Vec<Ref<ParticleDef>>,
}

/// An aiming reticle.
#[derive(Debug, Clone, PartialEq)]
pub struct ReticleDef {
    pub name: String,
    pub bitmap: String,
}

/// A projectile (a cartridge's shot).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileDef {
    pub name: String,
    pub damage: f32,
    pub rounds: i32,
    pub tank_explosion: Option<Ref<ExplosionDef>>,
    pub own_tank_explosion: Option<Ref<ExplosionDef>>,
    pub bounce_explosion: Option<Ref<ExplosionDef>>,
    pub other_explosion: Option<Ref<ExplosionDef>>,
    pub reticle: Option<Ref<ReticleDef>>,
    pub emitter: Option<Ref<EmitterDef>>,
    pub fire_sound: Option<Ref<SoundDef>>,
}

/// What a power-up does when a tank drives into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupKind {
    /// A random power-up weapon.
    Weapon,
    /// Repairs the tank.
    Health,
    /// The reload arch: the tank's own cartridge back.
    Ammo,
    /// The scrum goal.
    Goal,
    /// A solo brain.
    Brain,
    /// Power-up Test: the pick-up's own cartridge.
    TestCartridge,
    /// Power-up Test: a random cartridge.
    TestRandom,
    /// Power-up Test: full health.
    TestHeal,
    None,
}

impl PickupKind {
    /// From a power-up's `behavior` and `kind` fields.
    pub fn of(behavior: &str, kind: &str) -> PickupKind {
        match behavior.to_ascii_lowercase().as_str() {
            "poweruptestcartridge" => return PickupKind::TestCartridge,
            "poweruptestrandom" => return PickupKind::TestRandom,
            "poweruptestheal" => return PickupKind::TestHeal,
            _ => {}
        }
        match kind.to_ascii_lowercase().as_str() {
            "weapon" => PickupKind::Weapon,
            "health" => PickupKind::Health,
            "ammo" => PickupKind::Ammo,
            "goal" => PickupKind::Goal,
            "brain" => PickupKind::Brain,
            _ => PickupKind::None,
        }
    }
}

/// A power-up.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupDef {
    pub name: String,
    pub kind: PickupKind,
    pub sound: Option<Ref<SoundDef>>,
    /// The sound when it switches off (the reload arch's zap).
    pub sound_off: Option<Ref<SoundDef>>,
    /// Power-up Test: the cartridge it gives and its sound.
    pub projectile: Option<Ref<ProjectileDef>>,
    pub pickup_sound: Option<Ref<SoundDef>>,
}

/// A boost or jump pad.
#[derive(Debug, Clone, PartialEq)]
pub struct PadDef {
    pub name: String,
    pub sound: Option<Ref<SoundDef>>,
}

/// A volcano.
#[derive(Debug, Clone, PartialEq)]
pub struct VolcanoDef {
    pub name: String,
    pub projectile: Option<Ref<ProjectileDef>>,
    pub sound: Option<Ref<SoundDef>>,
}

/// A ball (the scrum ball, the practice ball).
#[derive(Debug, Clone, PartialEq)]
pub struct BallDef {
    pub name: String,
    /// A practice ball: the goal sends it back to its spot, no score.
    pub practice: bool,
    pub hard_sound: Option<Ref<SoundDef>>,
    pub soft_sound: Option<Ref<SoundDef>>,
    pub snag_sound: Option<Ref<SoundDef>>,
}

/// A bot driver.
#[derive(Debug, Clone, PartialEq)]
pub struct AiDef {
    pub name: String,
}

/// A tank's sounds and effects.
#[derive(Debug, Clone, PartialEq)]
pub struct TankFxDef {
    pub name: String,
    pub sounds: Vec<Ref<SoundDef>>,
    pub damage_emitter: Option<Ref<EmitterDef>>,
    pub brain_projectile: Option<Ref<ProjectileDef>>,
}

/// Tank physics shared by the tanks.
#[derive(Debug, Clone, PartialEq)]
pub struct TankMiscDef {
    pub name: String,
}

/// The chase camera.
#[derive(Debug, Clone, PartialEq)]
pub struct TankCameraDef {
    pub name: String,
}

/// The observer camera.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraDef {
    pub name: String,
}

/// A mission editor marker.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkerDef {
    pub name: String,
}

/// A solo campaign tank's tier (by its name: `BronzeLightTank`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
    Boss,
}

/// A tank.
#[derive(Debug, Clone, PartialEq)]
pub struct TankDef {
    pub name: String,
    pub default_projectile: Option<Ref<ProjectileDef>>,
    /// Its driver when a bot drives it.
    pub ai: Option<Ref<AiDef>>,
    pub fx: Option<Ref<TankFxDef>>,
    pub misc: Option<Ref<TankMiscDef>>,
    pub camera: Option<Ref<TankCameraDef>>,
    /// Damage dealt, armour and fire rate scales.
    pub damage_scale: f32,
    pub armor_scale: f32,
    pub fire_scale: f32,
    /// The solo campaign's points class.
    pub tier: Option<Tier>,
}

named!(SoundDescDef, SoundDef, ExplosionDef, ParticleDef, EmitterDef, ReticleDef, ProjectileDef, PickupDef, PadDef, VolcanoDef, BallDef, AiDef, TankFxDef, TankMiscDef, TankCameraDef, CameraDef, MarkerDef, TankDef);

/// A power-up weapon: the projectile a tank gets and the sound its player
/// hears.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weapon {
    pub projectile: Ref<ProjectileDef>,
    pub sound: Option<Ref<SoundDef>>,
}

/// A reference that names nothing of its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataError {
    /// The definition with the reference.
    pub owner: String,
    pub field: &'static str,
    /// The name it gives.
    pub name: String,
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} names {:?}, which is not declared", self.owner, self.field, self.name)
    }
}

/// The game's data (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct GameData {
    pub sound_descs: Table<SoundDescDef>,
    pub sounds: Table<SoundDef>,
    pub explosions: Table<ExplosionDef>,
    pub particles: Table<ParticleDef>,
    pub emitters: Table<EmitterDef>,
    pub reticles: Table<ReticleDef>,
    pub projectiles: Table<ProjectileDef>,
    pub pickups: Table<PickupDef>,
    pub pads: Table<PadDef>,
    pub volcanoes: Table<VolcanoDef>,
    pub balls: Table<BallDef>,
    pub ai: Table<AiDef>,
    pub tank_fx: Table<TankFxDef>,
    pub tank_misc: Table<TankMiscDef>,
    pub tank_cameras: Table<TankCameraDef>,
    pub cameras: Table<CameraDef>,
    pub markers: Table<MarkerDef>,
    pub tanks: Table<TankDef>,
    /// The power-up weapons, each as often as its share of the random pick.
    pub weapons: Vec<Weapon>,
    /// The references that name nothing.
    pub errors: Vec<DataError>,
    fingerprint: (u64, u32),
}

/// The classes that are sounds (not in the fingerprint).
const SOUND_CLASSES: [&str; 2] = ["SoundDescDef", "SoundDef"];

/// A power-up weapon as the data file lists it: projectile, sound and
/// how many slots of the pick it fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponEntry {
    pub projectile: String,
    pub sound: String,
    pub weight: u32,
}

impl GameData {
    /// The definitions of `objects` (declaration order, parents copied in)
    /// and the power-up weapons of `weapons`.
    pub fn build<'a>(objects: impl IntoIterator<Item = &'a SceneObject>, weapons: &[WeaponEntry]) -> GameData {
        let all: Vec<&SceneObject> = objects.into_iter().collect();
        let mut d = GameData { fingerprint: fingerprint(&all), ..Default::default() };
        let objects: Vec<&SceneObject> = all.into_iter().filter(|o| !o.name.is_empty()).collect();
        // First every name of every kind, then the references between them.
        let class = |o: &SceneObject, c: &str| o.class.eq_ignore_ascii_case(c);
        let text = |o: &SceneObject, f: &str| o.get(f).unwrap_or("").trim().to_string();
        let num = |o: &SceneObject, f: &str, default: f32| o.get(f).map_or(default, |v| atof(v) as f32);
        for o in &objects {
            let name = o.name.clone();
            if class(o, "SoundDescDef") {
                d.sound_descs.push(SoundDescDef { name, volume: num(o, "volume", 1.0), looping: num(o, "is_looping", 0.0) != 0.0, is_3d: num(o, "is_3d", 0.0) != 0.0 });
            } else if class(o, "SoundDef") {
                d.sounds.push(SoundDef { name, file: text(o, "file"), description: None });
            } else if class(o, "ExplosionDef") {
                d.explosions.push(ExplosionDef { name, resource: text(o, "resource"), sound: None });
            } else if class(o, "ParticleDef") {
                d.particles.push(ParticleDef { name, texture: text(o, "texture") });
            } else if class(o, "EmitterDef") {
                d.emitters.push(EmitterDef { name, particles: Vec::new() });
            } else if class(o, "ReticleDef") {
                d.reticles.push(ReticleDef { name, bitmap: text(o, "image") });
            } else if class(o, "ProjectileDef") {
                d.projectiles.push(ProjectileDef { name, damage: num(o, "damage", 0.0), rounds: num(o, "rounds", 0.0) as i32, tank_explosion: None, own_tank_explosion: None, bounce_explosion: None, other_explosion: None, reticle: None, emitter: None, fire_sound: None });
            } else if class(o, "PowerUpDef") {
                d.pickups.push(PickupDef { name, kind: PickupKind::of(&text(o, "behavior"), &text(o, "kind")), sound: None, sound_off: None, projectile: None, pickup_sound: None });
            } else if class(o, "BoostDef") {
                d.pads.push(PadDef { name, sound: None });
            } else if class(o, "VolcanoDef") {
                d.volcanoes.push(VolcanoDef { name, projectile: None, sound: None });
            } else if class(o, "FlagDef") {
                d.balls.push(BallDef { name, practice: num(o, "practice", 0.0) != 0.0, hard_sound: None, soft_sound: None, snag_sound: None });
            } else if class(o, "TankAiDef") {
                d.ai.push(AiDef { name });
            } else if class(o, "TankFxDef") {
                d.tank_fx.push(TankFxDef { name, sounds: Vec::new(), damage_emitter: None, brain_projectile: None });
            } else if class(o, "TankMiscDef") {
                d.tank_misc.push(TankMiscDef { name });
            } else if class(o, "TankCameraDef") {
                d.tank_cameras.push(TankCameraDef { name });
            } else if class(o, "CameraDef") {
                d.cameras.push(CameraDef { name });
            } else if class(o, "MarkerDef") {
                d.markers.push(MarkerDef { name });
            } else if class(o, "TankDef") {
                let tier = [("Bronze", Tier::Bronze), ("Silver", Tier::Silver), ("Gold", Tier::Gold), ("Boss", Tier::Boss)].into_iter().find(|(t, _)| name.contains(t)).map(|(_, t)| t);
                d.tanks.push(TankDef {
                    name,
                    default_projectile: None,
                    ai: None,
                    fx: None,
                    misc: None,
                    camera: None,
                    damage_scale: num(o, "damage_scale", 0.0),
                    armor_scale: num(o, "armor_scale", 0.0),
                    fire_scale: num(o, "fire_scale", 0.0),
                    tier,
                });
            }
        }
        let mut errors = Vec::new();
        for o in &objects {
            let mut r = Resolver { owner: &o.name, o, errors: &mut errors };
            if class(o, "SoundDef") {
                let v = r.one(&d.sound_descs, "description");
                set(&mut d.sounds, &o.name, |s| s.description = v);
            } else if class(o, "ExplosionDef") {
                let v = r.one(&d.sounds, "sound");
                set(&mut d.explosions, &o.name, |s| s.sound = v);
            } else if class(o, "EmitterDef") {
                let v = r.list(&d.particles, "particles");
                set(&mut d.emitters, &o.name, |s| s.particles = v);
            } else if class(o, "ProjectileDef") {
                let v = (r.one(&d.explosions, "tank_explosion"), r.one(&d.explosions, "own_tank_explosion"), r.one(&d.explosions, "bounce_explosion"), r.one(&d.explosions, "other_explosion"));
                let (reticle, emitter, fire) = (r.one(&d.reticles, "reticle"), r.one(&d.emitters, "emitter"), r.one(&d.sounds, "fire_sound"));
                set(&mut d.projectiles, &o.name, |p| {
                    (p.tank_explosion, p.own_tank_explosion, p.bounce_explosion, p.other_explosion) = v;
                    (p.reticle, p.emitter, p.fire_sound) = (reticle, emitter, fire);
                });
            } else if class(o, "PowerUpDef") {
                let v = (r.one(&d.sounds, "sound"), r.one(&d.sounds, "sound_off"), r.one(&d.projectiles, "test_projectile"), r.one(&d.sounds, "test_sound"));
                set(&mut d.pickups, &o.name, |p| (p.sound, p.sound_off, p.projectile, p.pickup_sound) = v);
            } else if class(o, "BoostDef") {
                let v = r.one(&d.sounds, "sound");
                set(&mut d.pads, &o.name, |p| p.sound = v);
            } else if class(o, "VolcanoDef") {
                let v = (r.one(&d.projectiles, "Projectile"), r.one(&d.sounds, "sound"));
                set(&mut d.volcanoes, &o.name, |p| (p.projectile, p.sound) = v);
            } else if class(o, "FlagDef") {
                let v = (r.one(&d.sounds, "hard_collision_sound"), r.one(&d.sounds, "soft_collision_sound"), r.one(&d.sounds, "snag_sound"));
                set(&mut d.balls, &o.name, |b| (b.hard_sound, b.soft_sound, b.snag_sound) = v);
            } else if class(o, "TankFxDef") {
                let fields = ["hard_tank_collision_sound", "soft_tank_collision_sound", "hard_static_collision_sound", "soft_static_collision_sound", "hard_ground_collision_sound", "soft_ground_collision_sound", "loose_head_sound", "fire_sound", "dry_fire_sound", "engine_knock_sound", "engine_sound_a", "engine_sound_b"];
                let sounds: Vec<Ref<SoundDef>> = fields.iter().filter_map(|f| r.one(&d.sounds, f)).collect();
                let v = (r.one(&d.emitters, "damage_emitter"), r.one(&d.projectiles, "brain_projectile"));
                set(&mut d.tank_fx, &o.name, |x| {
                    x.sounds = sounds;
                    (x.damage_emitter, x.brain_projectile) = v;
                });
            } else if class(o, "TankDef") {
                let v = (r.one(&d.projectiles, "default_projectile"), r.one(&d.ai, "ai"), r.one(&d.tank_fx, "fx"), r.one(&d.tank_misc, "misc"), r.one(&d.tank_cameras, "Camera"));
                set(&mut d.tanks, &o.name, |t| (t.default_projectile, t.ai, t.fx, t.misc, t.camera) = v);
            }
        }
        for w in weapons {
            match d.projectiles.find(&w.projectile) {
                Some(p) => {
                    let sound = d.sounds.find(&w.sound);
                    if sound.is_none() && !w.sound.is_empty() {
                        errors.push(DataError { owner: "power-up weapons".into(), field: "sound", name: w.sound.clone() });
                    }
                    for _ in 0..w.weight {
                        d.weapons.push(Weapon { projectile: p, sound });
                    }
                }
                None => errors.push(DataError { owner: "power-up weapons".into(), field: "projectile", name: w.projectile.clone() }),
            }
        }
        d.errors = errors;
        d
    }

    /// The canonical hash of the gameplay definitions and their count.
    pub fn fingerprint(&self) -> (u64, u32) {
        self.fingerprint
    }
}

/// Changes the definition `name` of `t`.
fn set<T: Named>(t: &mut Table<T>, name: &str, f: impl FnOnce(&mut T)) {
    if let Some(r) = t.find(name) {
        f(&mut t.items[r.index()]);
    }
}

/// Resolves one definition's references.
struct Resolver<'a> {
    owner: &'a str,
    o: &'a SceneObject,
    errors: &'a mut Vec<DataError>,
}

impl Resolver<'_> {
    /// The definition `field` names (None: unset, or an error).
    fn one<T: Named>(&mut self, t: &Table<T>, field: &'static str) -> Option<Ref<T>> {
        let name = self.o.get(field).unwrap_or("").trim();
        if name.is_empty() {
            return None;
        }
        let r = t.find(name);
        if r.is_none() {
            self.errors.push(DataError { owner: self.owner.to_string(), field, name: name.to_string() });
        }
        r
    }

    /// The definitions a tab- or space-separated list names.
    fn list<T: Named>(&mut self, t: &Table<T>, field: &'static str) -> Vec<Ref<T>> {
        let text = self.o.get(field).unwrap_or("").to_string();
        let mut out = Vec::new();
        for name in text.split(['\t', ' ']).filter(|n| !n.is_empty()) {
            match t.find(name) {
                Some(r) => out.push(r),
                None => self.errors.push(DataError { owner: self.owner.to_string(), field, name: name.to_string() }),
            }
        }
        out
    }
}

/// The canonical hash of the non-sound definitions: each as its name,
/// class and lower-case field / value pairs, sorted.
fn fingerprint(objects: &[&SceneObject]) -> (u64, u32) {
    let mut entries: Vec<String> = Vec::new();
    for o in objects {
        if SOUND_CLASSES.iter().any(|c| o.class.eq_ignore_ascii_case(c)) {
            continue;
        }
        let mut s = format!("{}|{}", o.name.to_ascii_lowercase(), o.class.to_ascii_lowercase());
        let mut fields: Vec<(String, String)> = o.fields.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.to_ascii_lowercase())).collect();
        fields.sort();
        for (k, v) in fields {
            s.push('|');
            s.push_str(&k);
            s.push('=');
            s.push_str(&v);
        }
        entries.push(s);
    }
    entries.sort();
    let mut hash = tt_assets::FNV_START;
    for e in &entries {
        tt_assets::fnv1a(&mut hash, e.as_bytes());
        tt_assets::fnv1a(&mut hash, &[0]);
    }
    (hash, entries.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(class: &str, name: &str, fields: &[(&str, &str)]) -> SceneObject {
        SceneObject { class: class.into(), name: name.into(), def: true, parent: None, fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), children: Vec::new() }
    }

    #[test]
    fn references_resolve_or_err() {
        let objects = [
            obj("TankAiDef", "GoldAI", &[]),
            obj("ProjectileDef", "Shell", &[("damage", "1.5")]),
            obj("TankDef", "GoldLightTank", &[("ai", "goldai"), ("default_projectile", "Shell"), ("damage_scale", "2"), ("fx", "NoFx")]),
        ];
        let d = GameData::build(&objects, &[WeaponEntry { projectile: "Shell".into(), sound: String::new(), weight: 2 }]);
        let t = d.tanks.named("goldlighttank").unwrap();
        assert_eq!(t.ai.map(|a| d.ai[a].name.as_str()), Some("GoldAI"));
        assert_eq!(t.default_projectile.map(|p| d.projectiles[p].damage), Some(1.5));
        assert_eq!((t.damage_scale, t.tier), (2.0, Some(Tier::Gold)));
        assert_eq!(t.fx, None);
        assert_eq!(d.errors, vec![DataError { owner: "GoldLightTank".into(), field: "fx", name: "NoFx".into() }]);
        assert_eq!(d.weapons.len(), 2);
        assert_eq!(d.fingerprint().1, 3);
    }

    /// The fingerprint a client compares with the server's: the same for
    /// the same defs whatever the order of declarations and fields or the
    /// case, different when a value (or a field's name) differs.
    #[test]
    fn fingerprint_is_deterministic() {
        let a = [obj("ProjectileDef", "Shell", &[("damage", "1.5"), ("blast_radius", "11")]), obj("TankDef", "Light", &[("max_speed", "12")])];
        let b = [obj("tankdef", "LIGHT", &[("MAX_SPEED", "12")]), obj("ProjectileDef", "shell", &[("blast_radius", "11"), ("damage", "1.5")])];
        let fp = |o: &[SceneObject]| GameData::build(o, &[]).fingerprint();
        assert_eq!(fp(&a), fp(&b));
        assert_eq!(fp(&a), fp(&a.clone()));
        let other_value = [obj("ProjectileDef", "Shell", &[("damage", "1.5"), ("blast_radius", "12")]), a[1].clone()];
        let old_name = [obj("ProjectileDef", "Shell", &[("damage", "1.5"), ("splash_area", "11")]), a[1].clone()];
        assert_ne!(fp(&a), fp(&other_value));
        assert_ne!(fp(&a), fp(&old_name));
        assert_eq!(fp(&a).1, 2);
    }
}

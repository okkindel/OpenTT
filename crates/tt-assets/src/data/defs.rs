//! The game's data files (`assets/data/`), by domain: `sounds.ron`,
//! `cameras.ron`, `markers.ron`, `effects.ron`, `weapons.ron`,
//! `pickups.ron`, `map_objects.ron`, `ai.ron`, `tanks.ron` (the defs,
//! [`SETS`] in the order they are declared), and `campaign.ron` (the solo
//! level table).
//!
//! The schemas list every field of a class; a def's fields keep their
//! exact text ([`super::Num`]) and reach the game under their RON names.

use super::Num;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The def files (`data/<set>.ron`), in the order the server declares
/// them (a file's defs may name earlier ones as parents).
pub const SETS: &[&str] = &["sounds", "cameras", "markers", "effects", "weapons", "pickups", "map_objects", "ai", "tanks"];

/// `data/<stem>.ron` under the asset directory `root`.
pub fn file_path(root: &Path, stem: &str) -> PathBuf {
    root.join("data").join(format!("{stem}.ron"))
}

/// `data/campaign.ron` under the asset directory `root`.
pub fn campaign_path(root: &Path) -> PathBuf {
    file_path(root, "campaign")
}

/// A def file: the declarations in order.
pub type DefFile = Vec<Def>;

/// `sounds.ron`: the sound descriptions and the sounds of the game world
/// (defs), the interface sounds and the music (the client's).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundsFile {
    pub game: Vec<Def>,
    pub interface: Vec<ClientObject>,
    pub music: Vec<ClientObject>,
}

/// `pickups.ron`: the power-ups and the power-up weapons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickupsFile {
    pub pickups: Vec<Def>,
    pub weapons: Vec<PowerUpWeapon>,
}

/// The defs of set `stem` ([`SETS`]).
pub fn load_file(root: &Path, stem: &str) -> Result<DefFile, String> {
    let path = file_path(root, stem);
    match stem {
        "sounds" => super::load::<SoundsFile>(&path).map(|f| f.game),
        "pickups" => super::load::<PickupsFile>(&path).map(|f| f.pickups),
        _ => super::load(&path),
    }
}

/// The power-up weapons of `pickups.ron`.
pub fn load_weapons(root: &Path) -> Result<Vec<PowerUpWeapon>, String> {
    super::load::<PickupsFile>(&file_path(root, "pickups")).map(|f| f.weapons)
}

/// The solo campaign's levels: a named group of levels, each with an
/// optional parent level it copies the fields of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpLevels {
    /// The group's name (`SPLevels`).
    pub group: String,
    pub levels: Vec<SpLevel>,
}

/// One power-up weapon: a slot of the random roll at a weapon pick-up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerUpWeapon {
    /// The projectile def the tank gets.
    pub projectile: String,
    /// The sound the player hears a second later.
    pub sound: String,
    /// Slots of the roll it fills (its share of the random pick); 1 when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<u32>,
}

impl PowerUpWeapon {
    /// The roll table: each weapon `weight` times, in order.
    pub fn table(weapons: &[PowerUpWeapon]) -> Vec<&PowerUpWeapon> {
        weapons.iter().flat_map(|w| std::iter::repeat_n(w, w.weight.unwrap_or(1) as usize)).collect()
    }
}

crate::object_enum!(
    /// One def of a data file.
    Def {
        SoundDescDef,
        SoundDef,
        CameraDef,
        MarkerDef,
        ExplosionDef,
        ParticleDef,
        EmitterDef,
        ReticleDef,
        ProjectileDef,
        PowerUpDef,
        BoostDef,
        VolcanoDef,
        FlagDef,
        TankAiDef,
        TankDef,
        TankCameraDef,
        TankFxDef,
        TankMiscDef,
    }
);

crate::schema! {
    SoundDescDef = "SoundDescDef", true {
        /// The mixer channel whose volume applies: `"sim"` (the game
        /// world's sounds; see [`ClientSoundDesc`] for the others).
        channel: String,
        volume: Num,
        is_looping: Num,
        is_3d: Num,
        reference_distance: Num,
        max_distance: Num,
    }
}

crate::object_enum!(
    /// One of the client's sound objects (interface sounds, music):
    /// plain objects, not defs.
    ClientObject {
        ClientSoundDesc,
        ClientSound,
        MusicDef,
    }
);

/// The client's sounds (interface, then music) of `sounds.ron`.
pub type ClientFile = Vec<ClientObject>;

/// The client's interface sound descriptions and sounds of `sounds.ron`.
pub fn load_interface_sounds(root: &Path) -> Result<ClientFile, String> {
    super::load::<SoundsFile>(&file_path(root, "sounds")).map(|f| f.interface)
}

/// The music tracks of `sounds.ron`.
pub fn load_music(root: &Path) -> Result<ClientFile, String> {
    super::load::<SoundsFile>(&file_path(root, "sounds")).map(|f| f.music)
}

/// The client's part of `sounds.ron`: the interface sounds.
pub const CLIENT_AUDIO: &str = "interface";
/// The client's part of `sounds.ron`: the music.
pub const MUSIC: &str = "music";

/// A client part of `sounds.ron` ([`CLIENT_AUDIO`] or [`MUSIC`]).
pub fn load_client_file(root: &Path, part: &str) -> Result<ClientFile, String> {
    if part == MUSIC {
        load_music(root)
    } else {
        load_interface_sounds(root)
    }
}

crate::schema! {
    ClientSoundDesc = "SoundDescDef", false {
        volume: Num,
        is_looping: Num,
        is_streaming: Num,
        is_3d: Num,
        /// The mixer channel: `"gui"`, `"message"` or `"music"`.
        channel: String,
    }
}

crate::schema! {
    ClientSound = "SoundDef", false {
        file: String,
        description: String,
        preload: Num,
    }
}

crate::schema! {
    /// A music track: its moods' files and the sequences they play in.
    MusicDef = "MusicDef", false {
        num_moods: Num,
        k1: Num,
        k2: Num,
        k3: Num,
        smooth1: Num,
        smooth2: Num,
        smooth3: Num,
        mood_smooth: Num,
        mood_scale: Num,
    } [
        mood1: String,
        sequences1: String,
    ]
}

crate::schema! {
    CameraDef = "CameraDef", true {
        mode: String,
    }
}

crate::schema! {
    MarkerDef = "MarkerDef", true {
        category: String,
        shape: String,
    }
}

crate::schema! {
    SoundDef = "SoundDef", true {
        file: String,
        description: String,
        preload: Num,
    }
}

crate::schema! {
    ExplosionDef = "ExplosionDef", true {
        start_size: Num,
        end_size: Num,
        duration_ms: Num,
        resource: String,
        sound: String,
    }
}

crate::schema! {
    ParticleDef = "ParticleDef", true {
        texture: String,
        drag_coefficient: Num,
        gravity_coefficient: Num,
        inherit_velocity: Num,
        constant_acceleration: Num,
        lifetime_ms: Num,
        lifetime_variance_ms: Num,
        inverse_alpha: Num,
        spin_random_min: Num,
        spin_random_max: Num,
    } [
        colors: [Num; 4],
        sizes: Num,
        times: Num,
    ]
}

crate::schema! {
    EmitterDef = "EmitterDef", true {
        ejection_period_ms: Num,
        period_variance_ms: Num,
        ejection_velocity: Num,
        velocity_variance: Num,
        ejection_offset: Num,
        theta_min: Num,
        theta_max: Num,
        phi_reference_vel: Num,
        phi_variance: Num,
        override_advances: Num,
        particles: String,
    }
}

crate::schema! {
    ReticleDef = "ReticleDef", true {
        image: String,
        size: Num,
        start_time: Num,
        time_step: Num,
        tick_count: Num,
        spacing_exponent: Num,
        smooth_near: Num,
        smooth_far: Num,
        smooth_exponent: Num,
        animate: Num,
        anim_image: String,
        anim_time: Num,
        anim_size: Num,
        show_spot: Num,
        spot_image: String,
        spot_size_near: Num,
        spot_size_far: Num,
        /// Flat ring texture drawn on the surface at each impact spot,
        /// as wide as the cartridge's splash (the mortar's).
        ring_image: String,
    }
}

crate::schema! {
    ProjectileDef = "ProjectileDef", true {
        velocity: Num,
        inherit_velocity: Num,
        bounces: Num,
        blast_radius: Num,
        rounds: Num,
        keep_previous_weapon: Num,
        can_be_kept: Num,
        reload_ms: Num,
        volley_delay_ms: Num,
        volley_size: Num,
        sync_ms: Num,
        sync_lead_ms: Num,
        damage: Num,
        gravity_scale: Num,
        sprite_scale: Num,
        resource: String,
        tank_explosion: String,
        own_tank_explosion: String,
        bounce_explosion: String,
        other_explosion: String,
        reticle: String,
        bounce_factor: Num,
        emitter: String,
        max_rotation: Num,
        /// Projectiles per shot (scatter shot): a fan or a cone.
        pellet_count: Num,
        /// Total fan (or ring) width of the pellets, degrees.
        spread_angle: Num,
        /// Pellet spacing: 1 = even, above 1 packed towards the middle.
        spread_exponent: Num,
        /// 1 = a cone (one pellet in the middle, the rest on a ring
        /// `spread_angle` wide) instead of a horizontal fan.
        spread_ring: Num,
        /// Per-shot jitter of the ring (0..1), from the shot number.
        spread_jitter: Num,
        /// A cone's every other ring pellet at this fraction of the ring.
        spread_inner: Num,
        /// Tracer streak behind a sprite projectile (m; 0 = none).
        trail_length: Num,
        /// High-angle launch (mortar): the elevation above the horizon,
        /// from `launch_pitch_max` (barrel lowest) to
        /// `launch_pitch_min` (barrel highest), degrees.
        launch_pitch_min: Num,
        launch_pitch_max: Num,
        /// High-angle reach on the flat (m) with the barrel lowest /
        /// highest; the launch speed follows from it (0 = `velocity`).
        launch_range_min: Num,
        launch_range_max: Num,
        /// Flight time limit (ms; 0 = none): the projectile is removed
        /// when it runs out.
        max_life_ms: Num,
        /// Fire sound (SoundProfile) instead of the tank's `fireSound`.
        fire_sound: String,
        /// A `.dts` drawn instead of a sprite projectile's image, nose
        /// (+Y) along its flight (looks only; the image stays the
        /// projectile's resource).
        display_shape: String,
        /// Additive glow sprite (an exhaust flame) and its size (m),
        /// `exhaust_offset` m behind the projectile's centre, where its
        /// `emitter` trail starts too.
        exhaust_glow: String,
        exhaust_glow_size: Num,
        exhaust_offset: Num,
    }
}

crate::schema! {
    PowerUpDef = "PowerUpDef", true {
        category: String,
        shape: String,
        kind: String,
        shadow: Num,
        shadow_animation: Num,
        starts_on: Num,
        min_off_ms: Num,
        max_off_ms: Num,
        sound: String,
        sound_off: String,
        ai_pickup: Num,
        activate_sequence: String,
        static_shadow: Num,
        behavior: String,
        test_projectile: String,
        test_sound: String,
    }
}

crate::schema! {
    BoostDef = "BoostDef", true {
        category: String,
        shape: String,
        kind: String,
        shadow: Num,
        advance_delay_on_enter: Num,
        num_states: Num,
        state_name1: String,
        velocity1: Num,
        min_duration1: Num,
        state_name2: String,
        velocity2: Num,
        min_duration2: Num,
        state_name3: String,
        velocity3: Num,
        min_duration3: Num,
        state_name4: String,
        velocity4: Num,
        min_duration4: Num,
        sound: String,
    }
}

crate::schema! {
    VolcanoDef = "VolcanoDef", true {
        category: String,
        shadow: Num,
        static_shadow: Num,
        shape: String,
        num_zones: Num,
        min_attack_delay: Num,
        max_attack_delay: Num,
        min_attack_rocks: Num,
        max_attack_rocks: Num,
        zone_name1: String,
        zone_weight1: Num,
        zone_focus_weight1: Num,
        zone_radius1: Num,
        zone_name2: String,
        zone_weight2: Num,
        zone_focus_weight2: Num,
        zone_radius2: Num,
        zone_name3: String,
        zone_weight3: Num,
        zone_focus_weight3: Num,
        zone_radius3: Num,
        zone_name4: String,
        zone_weight4: Num,
        zone_focus_weight4: Num,
        zone_radius4: Num,
        zone_name5: String,
        zone_weight5: Num,
        zone_focus_weight5: Num,
        zone_radius5: Num,
        projectile: String,
        sound: String,
    }
}

crate::schema! {
    FlagDef = "FlagDef", true {
        category: String,
        shape: String,
        bounce_terrain: Num,
        bounce_static: Num,
        friction_terrain: Num,
        unstuck_time: Num,
        unstuck_boost: Num,
        spring_range: Num,
        spring_vel_scale: Num,
        spring_correct_scale1: Num,
        spring_correct_scale2: Num,
        hard_collision_sound: String,
        hard_collision_speed: Num,
        soft_collision_sound: String,
        soft_collision_speed: Num,
        collision_interval_ms: Num,
        snag_sound: String,
        no_pickup: Num,
    }
}

crate::schema! {
    TankAiDef = "TankAiDef", false {
        home_range: Num,
        target_range: Num,
        disengage_range: Num,
        aim: Num,
        aim_speed: Num,
        trigger_happy: Num,
        long_shot_scale: Num,
        flee_damage: Num,
        flee_on_time: Num,
        flee_off_time: Num,
        scan_delay: Num,
        follow: Num,
        snipe_weight: Num,
        ram_weight: Num,
        flank_weight: Num,
        back_weight: Num,
        oc_weight: Num,
        revenge_weight: Num,
    }
}

crate::schema! {
    TankDef = "TankDef", true {
        category: String,
        shape: String,
        brain_shape: String,
        camera: String,
        fx: String,
        misc: String,
        turn_rate: Num,
        accel_rate: Num,
        decel_rate: Num,
        coast_decel_rate: Num,
        anti_slide_rate: Num,
        max_speed: Num,
        turret_pitch_center: Num,
        turret_yaw_range: Num,
        turret_pitch_range: Num,
        turret_auto_level: Num,
        suspension_range: Num,
        spring_range_x: Num,
        spring_range_y: Num,
        spring_vel_scale: Num,
        spring_correct_scale1: Num,
        spring_correct_scale2: Num,
        damage_amount: Num,
        damage_scale: Num,
        armor_scale: Num,
        fire_scale: Num,
        default_projectile: String,
        ai: String,
    }
}

crate::schema! {
    TankCameraDef = "TankCameraDef", true {
        cam_focus_dist: Num,
        cam_focus_up_dist: Num,
        cam_backup_dist: Num,
        cam_float_height: Num,
        cam_smooth: Num,
        cam_smooth_turn: Num,
        cam_smooth_up: Num,
        cam_fov: Num,
        cam_near: Num,
    }
}

crate::schema! {
    TankFxDef = "TankFxDef", true {
        corpse_rotate_rate: Num,
        corpse_shake_time: Num,
        corpse_shake_speed: Num,
        corpse_shake_amp: Num,
        corpse_jump_height: Num,
        corpse_flip_speed: Num,
        corpse_smoke_time: Num,
        corpse_fade_time: Num,
        corpse_fade_speed: Num,
        hard_tank_collision_sound: String,
        hard_tank_collision_speed: Num,
        soft_tank_collision_sound: String,
        soft_tank_collision_speed: Num,
        tank_collision_interval_ms: Num,
        hard_static_collision_sound: String,
        hard_static_collision_speed: Num,
        soft_static_collision_sound: String,
        soft_static_collision_speed: Num,
        static_collision_interval_ms: Num,
        hard_ground_collision_sound: String,
        hard_ground_collision_speed: Num,
        soft_ground_collision_sound: String,
        soft_ground_collision_speed: Num,
        ground_collision_interval_ms: Num,
        loose_head_sound: String,
        fire_sound: String,
        dry_fire_sound: String,
        engine_knock_sound: String,
        engine_knock_start: Num,
        engine_knock_min: Num,
        engine_knock_max: Num,
        engine_sound_a: String,
        engine_pitch_a0: Num,
        engine_pitch_a1: Num,
        engine_vol_a0: Num,
        engine_vol_a1: Num,
        engine_sound_b: String,
        engine_pitch_b0: Num,
        engine_pitch_b1: Num,
        engine_vol_b0: Num,
        engine_vol_b1: Num,
        damage_emitter_start: Num,
        damage_emitter: String,
        damage_emitter_node: String,
        brain_projectile: String,
    }
}

crate::schema! {
    TankMiscDef = "TankMiscDef", true {
        boost_ticks: Num,
        bounce_static: Num,
        bounce_tank: Num,
        unstuck_time: Num,
        unstuck_boost: Num,
        repair_rate: Num,
    }
}

crate::schema! {
    /// One solo level (its `parent`: the level it copies the fields of).
    SpLevel = "Level", false {
        damage_scale: Num,
        armor_scale: Num,
        fire_scale: Num,
        start_bots: Num,
        max_live_bots: Num,
        max_spawn_bots: Num,
        end_bots: Num,
        min_bot_ticks: Num,
        max_bot_ticks: Num,
        max_mission_ticks: Num,
        quick_shot_time: Num,
        bronze: Num,
        silver: Num,
        gold: Num,
        light: Num,
        medium: Num,
        heavy: Num,
        bronze_pts: Num,
        silver_pts: Num,
        gold_pts: Num,
        boss_pts: Num,
        brain_pts: Num,
        level_type: String,
        mission: String,
        mission_briefing: String,
        goto_level: Num,
        add_damage_scale: Num,
        add_armor_scale: Num,
        add_fire_scale: Num,
        add_bronze_pts: Num,
        add_silver_pts: Num,
        add_gold_pts: Num,
        add_boss_pts: Num,
        add_brain_pts: Num,
    }
}

/// The defs as a sequence of declarations leaves them:
///
/// * a def of a name already declared reuses that def: its fields are set
///   again, nothing is copied (a different class is an error, the
///   declaration does nothing);
/// * otherwise a new def gets a copy of its `parent`'s fields *as they
///   are now* (later changes of the parent do not reach it), then its own;
/// * a plain object of a def class ([`Object::DEF`] false, `TankAiDef`)
///   is always a new object; a later one of the same name hides the
///   earlier;
/// * setting a field to the empty string removes it.
///
/// [`DefSet::objects`] lists them flat (fields merged, no parent),
/// in declaration order, ready for `tt_game::fields::Defs`.
#[derive(Debug, Clone, Default)]
pub struct DefSet {
    /// (def, survives [`DefSet::delete_defs`]).
    blocks: Vec<(crate::mission::SceneObject, bool)>,
}

impl DefSet {
    pub fn new() -> DefSet {
        DefSet::default()
    }

    /// Whether a declaration makes a def: declared as one, or a plain
    /// object of a def class (the classes all end in `Def`).
    pub fn is_def(class: &str, def: bool) -> bool {
        def || class.len() > 3 && class.is_char_boundary(class.len() - 3) && class[class.len() - 3..].eq_ignore_ascii_case("def")
    }

    /// The def `name` (the last declared of that name).
    pub fn get(&self, name: &str) -> Option<&crate::mission::SceneObject> {
        self.blocks.iter().rev().map(|(o, _)| o).find(|o| o.name.eq_ignore_ascii_case(name))
    }

    fn set(o: &mut crate::mission::SceneObject, key: &str, value: &str) {
        match o.fields.iter().position(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(i) if value.is_empty() => {
                o.fields.remove(i);
            }
            Some(i) => o.fields[i].1 = value.to_string(),
            None if value.is_empty() => {}
            None => o.fields.push((key.to_string(), value.to_string())),
        }
    }

    /// One declaration with its fields `(name + index, value)`. False if
    /// it made no def.
    pub fn declare(&mut self, class: &str, name: &str, parent: Option<&str>, def: bool, fields: &[(String, String)]) -> bool {
        if !Self::is_def(class, def) {
            return false;
        }
        if def && !name.is_empty() {
            if let Some((o, _)) = self.blocks.iter_mut().find(|(o, _)| o.name.eq_ignore_ascii_case(name)) {
                if !o.class.eq_ignore_ascii_case(class) {
                    return false;
                }
                for (k, v) in fields {
                    Self::set(o, k, v);
                }
                return true;
            }
        }
        let copied = parent.filter(|p| !p.is_empty()).and_then(|p| self.get(p)).map(|p| p.fields.clone()).unwrap_or_default();
        let mut o = crate::mission::SceneObject { class: class.to_string(), name: name.to_string(), def: true, parent: None, fields: copied, children: Vec::new() };
        for (k, v) in fields {
            Self::set(&mut o, k, v);
        }
        self.blocks.push((o, false));
        true
    }

    /// Declares a def of a data file.
    pub fn declare_def(&mut self, d: &Def) -> bool {
        let fields: Vec<(String, String)> = d.to_fields().iter().map(|f| (f.key(), f.value.clone())).collect();
        self.declare(d.class(), d.name(), d.parent(), d.is_def(), &fields)
    }

    /// A field written later (e.g. the solo tank scales); false if there
    /// is no such def.
    pub fn set_field(&mut self, name: &str, key: &str, value: &str) -> bool {
        match self.blocks.iter_mut().rev().find(|(o, _)| o.name.eq_ignore_ascii_case(name)) {
            Some((o, _)) => {
                Self::set(o, key, value);
                true
            }
            None => false,
        }
    }

    /// Everything declared so far survives [`DefSet::delete_defs`]
    /// (the client's own defs, declared at start-up).
    pub fn mark_persistent(&mut self) {
        for b in &mut self.blocks {
            b.1 = true;
        }
    }

    /// Deletes all but the persistent defs.
    pub fn delete_defs(&mut self) {
        self.blocks.retain(|b| b.1);
    }

    /// The defs in declaration order, fields merged.
    pub fn objects(&self) -> impl Iterator<Item = &crate::mission::SceneObject> {
        self.blocks.iter().map(|(o, _)| o)
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn field<'a>(s: &'a DefSet, name: &str, key: &str) -> Option<&'a str> {
        s.get(name).and_then(|o| o.get(key))
    }

    #[test]
    fn declarations_copy_the_parent_at_declaration_time() {
        let mut s = DefSet::new();
        assert!(s.declare("TankDef", "Base", None, true, &f(&[("max_speed", "10"), ("turn_rate", "60")])));
        assert!(s.declare("TankDef", "Child", Some("base"), true, &f(&[("max_speed", "12")])));
        // Redeclaring the parent later does not reach the child (copy semantics).
        assert!(s.declare("TankDef", "BASE", None, true, &f(&[("turn_rate", "70"), ("armor", "2")])));
        assert_eq!(field(&s, "child", "turn_rate"), Some("60"));
        assert_eq!(field(&s, "child", "MAX_SPEED"), Some("12"));
        assert_eq!(field(&s, "child", "armor"), None);
        // The reused def kept its old fields and its place.
        assert_eq!(field(&s, "base", "max_speed"), Some("10"));
        assert_eq!(field(&s, "base", "turn_rate"), Some("70"));
        assert_eq!(s.objects().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["Base", "Child"]);
        // A different class cannot redeclare it.
        assert!(!s.declare("ProjectileDef", "Base", None, true, &f(&[("velocity", "5")])));
        assert_eq!(field(&s, "base", "velocity"), None);
        // An empty value removes the field; a missing parent copies nothing.
        assert!(s.declare("TankDef", "Child", None, true, &f(&[("max_speed", "")])));
        assert_eq!(field(&s, "child", "max_speed"), None);
        assert!(s.declare("TankDef", "Orphan", Some("Nobody"), true, &f(&[("a", "1")])));
        assert_eq!(s.get("orphan").unwrap().fields, f(&[("a", "1")]));
    }

    #[test]
    fn new_objects_and_runtime_writes() {
        let mut s = DefSet::new();
        assert!(!s.declare("Level", "SPLevel1", None, false, &f(&[("x", "1")])));
        assert!(s.declare("TankAiDef", "Ai", None, false, &f(&[("a", "1"), ("b", "2")])));
        assert!(s.declare("TankAiDef", "Ai", None, false, &f(&[("a", "3")])));
        assert_eq!(s.len(), 2);
        assert_eq!(field(&s, "ai", "a"), Some("3"));
        assert_eq!(field(&s, "ai", "b"), None);
        assert!(s.set_field("AI", "b", "5"));
        assert_eq!(field(&s, "ai", "b"), Some("5"));
        assert!(!s.set_field("none", "b", "5"));
        s.mark_persistent();
        assert!(s.declare("SoundDef", "Snd", None, true, &f(&[("file", "x.wav")])));
        s.delete_defs();
        assert!(s.get("snd").is_none() && s.get("ai").is_some());
    }
}

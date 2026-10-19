//! Mission files in RON (`assets/missions/<id>.ron`; the importer
//! converts the maps of the game install, our own maps are written as RON).
//!
//! A mission is a tree of objects: the `MissionGroup` [`Group`] with the
//! mission's objects nested in groups (object order is kept: the spawn
//! point choice indexes it), optionally preceded by defs of the mission
//! (`PowerUpDef(name: "PT_Heal", parent: "Heal", ...)`, the Power-up Test)
//! and followed by objects added under a condition ([`Conditional`]).
//! Building the parts in order creates the mission's objects.

use super::defs::Def;
use super::{Extra, Field, Num, Object};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A mission file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mission {
    /// Defs the mission declares before its objects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defs: Vec<Def>,
    /// The top-level objects (normally the one `MissionGroup`).
    pub objects: Vec<MissionObject>,
    /// Objects added after the mission's objects under a condition.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditional: Vec<Conditional>,
}

/// Objects added to a group unless a game variable has a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditional {
    /// (variable, value): the objects are added unless the variable
    /// equals the value (case-insensitive string comparison).
    pub unless: (String, String),
    /// The group the objects are added to.
    pub add_to: String,
    pub objects: Vec<MissionObject>,
}

/// Loads a mission RON file.
pub fn load(path: &Path) -> Result<Mission, String> {
    super::load(path)
}

/// Reads a mission from its RON text.
pub fn parse(text: &str) -> Result<Mission, String> {
    super::from_ron(text)
}

/// A group of objects (`MissionGroup`, the spawn point groups, ...).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The mission's music (on `MissionGroup`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_track: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Extra,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<MissionObject>,
}

impl Object for Group {
    const CLASS: &'static str = "Group";
    const DEF: bool = false;

    fn name(&self) -> &str {
        &self.name
    }

    fn parent(&self) -> Option<&str> {
        None
    }

    fn to_fields(&self) -> Vec<Field> {
        let mut out = Vec::new();
        if let Some(m) = &self.music_track {
            out.push(Field::new("music_track", "", m.clone()));
        }
        out.extend(self.extra.iter().map(|(k, v)| Field::new(k.clone(), "", v.clone())));
        out
    }

    /// The fields only; the children are added by the caller.
    fn from_fields(name: &str, _parent: Option<&str>, fields: &[Field]) -> Self {
        let mut g = Group { name: name.to_string(), ..Default::default() };
        for f in fields {
            if f.index.is_empty() && f.name.eq_ignore_ascii_case("music_track") {
                g.music_track = Some(f.value.clone());
                super::clear_extra(&mut g.extra, &f.key());
            } else {
                super::set_extra(&mut g.extra, f.key(), f.value.clone());
            }
        }
        g
    }
}

crate::schema! {
    /// Information: `MissionInfo`, the mission's name and description (or
    /// a named place: the practice ball's start).
    Info = "Info", false {
        display_name: String,
        desc0: String,
        desc1: String,
        desc2: String,
    }
}

crate::schema! {
    MissionArea = "MissionArea", false {
        area: [Num; 4],
        flight_ceiling: Num,
        flight_ceiling_range: Num,
        floor: Num,
        locked: String,
    }
}

crate::schema! {
    Sky = "Sky", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        material_list: String,
        visible_distance: Num,
        use_sky_textures: Num,
        render_bottom_texture: Num,
        sky_solid_color: [Num; 4],
        fog_distance: Num,
        fog_color: [Num; 4],
        fog_volume1: [Num; 3],
        fog_volume2: [Num; 3],
        fog_volume3: [Num; 3],
        fog_volume_color1: [Num; 4],
        fog_volume_color2: [Num; 4],
        fog_volume_color3: [Num; 4],
        wind_velocity: [Num; 3],
        wind_effect_precipitation: Num,
        no_render_bans: Num,
        cloud_height_per0: Num,
        cloud_height_per1: Num,
        cloud_height_per2: Num,
        cloud_speed1: Num,
        cloud_speed2: Num,
        cloud_speed3: Num,
        locked: String,
    }
}

crate::schema! {
    Sun = "Sun", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        direction: [Num; 3],
        color: [Num; 4],
        ambient: [Num; 4],
        locked: String,
    }
}

crate::schema! {
    Terrain = "Terrain", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        terrain_file: String,
        detail_texture: String,
        square_size: Num,
        tile: Num,
        block_shift: Num,
        locked: String,
    }
}

crate::schema! {
    SpawnPoint = "SpawnPoint", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        def: String,
        radius: Num,
        sphere_weight: Num,
        indoor_weight: Num,
        outdoor_weight: Num,
        lock_count: Num,
        homing_count: Num,
        locked: String,
    }
}

crate::schema! {
    Prop = "Prop", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        shape: String,
        tree_lighting: Num,
        light_boost: Num,
    }
}

crate::schema! {
    PowerUp = "PowerUp", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        def: String,
        light_boost: Num,
    }
}

crate::schema! {
    Boost = "Boost", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        def: String,
        light_boost: Num,
    }
}

crate::schema! {
    Volcano = "Volcano", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        def: String,
        light_boost: Num,
    }
}

crate::schema! {
    Flag = "Flag", false {
        position: [Num; 3],
        rotation: [Num; 4],
        scale: [Num; 3],
        def: String,
    }
}

crate::object_enum!(
    /// One object of a mission.
    MissionObject {
        Group,
        Info,
        MissionArea,
        Sky,
        Sun,
        Terrain,
        SpawnPoint,
        Prop,
        PowerUp,
        Boost,
        Volcano,
        Flag,
    }
);

impl MissionObject {
    /// The objects of a group (none for other objects).
    pub fn children(&self) -> &[MissionObject] {
        match self {
            MissionObject::Group(g) => &g.children,
            _ => &[],
        }
    }

    /// As a flat [`crate::mission::SceneObject`] tree.
    pub fn to_scene_object(&self) -> crate::mission::SceneObject {
        crate::mission::SceneObject {
            class: self.class().to_string(),
            name: self.name().to_string(),
            def: self.is_def(),
            parent: self.parent().map(str::to_string),
            fields: self.to_fields().iter().map(|f| (f.key(), f.value.clone())).collect(),
            children: self.children().iter().map(MissionObject::to_scene_object).collect(),
        }
    }
}

impl Mission {
    /// The mission's objects as [`crate::mission::SceneObject`]s: the
    /// top-level objects, then the conditional ones as members of their
    /// group when `added` says they are added. Mission defs are left out.
    pub fn scene_objects(&self, added: impl Fn(&Conditional) -> bool) -> Vec<crate::mission::SceneObject> {
        let mut out: Vec<crate::mission::SceneObject> = self.objects.iter().map(MissionObject::to_scene_object).collect();
        for c in self.conditional.iter().filter(|c| added(c)) {
            let objs = c.objects.iter().map(MissionObject::to_scene_object);
            match out.iter_mut().find_map(|o| scene_find(o, &c.add_to)) {
                Some(g) => g.children.extend(objs),
                None => out.extend(objs),
            }
        }
        out
    }
}

impl Mission {
    /// The mission's own defs as [`crate::mission::SceneObject`]s,
    /// each with the fields of its parent among them merged in
    /// ([`crate::mission::inherit`]).
    pub fn def_objects(&self) -> Vec<crate::mission::SceneObject> {
        let mut out: Vec<crate::mission::SceneObject> = Vec::new();
        for d in &self.defs {
            let mut o = crate::mission::SceneObject {
                class: d.class().to_string(),
                name: d.name().to_string(),
                def: true,
                parent: d.parent().map(str::to_string),
                fields: d.to_fields().iter().map(|f| (f.key(), f.value.clone())).collect(),
                children: Vec::new(),
            };
            crate::mission::inherit(&mut o, &out);
            out.push(o);
        }
        out
    }
}

fn scene_find<'a>(o: &'a mut crate::mission::SceneObject, name: &str) -> Option<&'a mut crate::mission::SceneObject> {
    if o.name.eq_ignore_ascii_case(name) {
        return Some(o);
    }
    o.children.iter_mut().find_map(|c| scene_find(c, name))
}

//! Missions: [`exec_mission`] loads a mission's objects from its RON file
//! (`assets/missions/<id>.ron`, [`tt_assets::data::mission`]) into
//! [`crate::game::MissionLoad`] (what the world and the scene are built
//! from); missions are named by that file (`missions/tt1_1.ron`,
//! [`tt_assets::layout::mission`]). Also the scene of the loaded mission
//! for the renderer, the `--mission` developer path and the mission file
//! helpers the server uses (existence, CRC, file search).

use crate::engine::Engine;
use crate::game::{MissionLoad, MissionNode};
use crate::host::EngineHost;
use std::path::{Path, PathBuf};
use tt_assets::data::mission as m;
use tt_assets::mission::SceneObject;
use tt_render::Scene;

/// The mission file `path` (`missions/X.ron`), if it exists.
pub fn mission_ron(h: &EngineHost, path: &str) -> Option<String> {
    h.fs.exists(path).then(|| path.to_string())
}

/// The mission file `path` exists.
pub fn mission_exists(h: &EngineHost, path: &str) -> bool {
    mission_ron(h, path).is_some()
}

fn parse_ron(h: &mut EngineHost, path: &str) -> Option<m::Mission> {
    let Some(ron) = mission_ron(h, path) else {
        h.log.error(&format!("Missing file: {path}!"));
        return None;
    };
    let bytes = h.fs.read(&ron)?;
    match std::str::from_utf8(&bytes).map_err(|e| e.to_string()).and_then(tt_assets::data::from_ron::<m::Mission>) {
        Ok(v) => Some(v),
        Err(e) => {
            h.log.error(&format!("{ron}: {e}"));
            None
        }
    }
}

/// The `MissionInfo` name of a mission ("" if it has none or no RON file).
pub fn mission_info_name(h: &mut EngineHost, path: &str) -> String {
    mission_meta(h, path).map(|m| m.name).unwrap_or_default()
}

/// What mission file `path` says about itself for the map list: the
/// `MissionInfo` name and `world` field, the `MissionGroup` music track
/// (`None` without a readable RON file).
pub fn mission_meta(h: &EngineHost, path: &str) -> Option<tt_shell::catalog::MissionMeta> {
    let mission = mission_ron(h, path).and_then(|r| h.fs.read(&r)).and_then(|b| String::from_utf8(b).ok()).and_then(|t| m::parse(&t).ok())?;
    let objects = mission.scene_objects(|_| false);
    let all: Vec<&SceneObject> = objects.iter().flat_map(|o| o.walk()).collect();
    let info = all.iter().find(|o| o.class.eq_ignore_ascii_case("Info") && o.name.eq_ignore_ascii_case("MissionInfo"));
    let group = all.iter().find(|o| o.class.eq_ignore_ascii_case("Group") && o.name.eq_ignore_ascii_case("MissionGroup"));
    Some(tt_shell::catalog::MissionMeta {
        id: tt_assets::layout::mission_id(path),
        name: info.and_then(|o| o.get("display_name")).unwrap_or_default().to_string(),
        music: group.and_then(|o| o.get("music_track")).unwrap_or_default().to_string(),
        world: info.and_then(|o| o.get("world")).and_then(|w| w.trim().parse().ok()),
    })
}

/// Loads a mission: its objects from its RON file (`path`,
/// `missions/X.ron`) in order — the mission's defs, the object tree, then
/// the conditional objects. The `MissionGroup` tree is
/// [`EngineHost::mission_load`]. False (with the "Missing file" error)
/// without a RON file.
pub fn exec_mission(h: &mut EngineHost, path: &str) -> bool {
    let Some(mission) = parse_ron(h, path) else { return false };
    declare_defs(h, path, &mission);
    let mut next = 0u32;
    let mut top: Vec<MissionNode> = mission.objects.iter().map(|o| build(o, &mut next)).collect();
    for c in &mission.conditional {
        // No game variable has a value here: the objects are added
        // unless the condition's value is empty.
        if c.unless.1.is_empty() {
            continue;
        }
        for o in &c.objects {
            let node = build(o, &mut next);
            // `Group.add(obj)`: into the group of that name in this mission.
            if let Some(g) = top.iter_mut().rev().find_map(|t| t.find_named_mut(&c.add_to)) {
                g.children.push(node);
            }
        }
    }
    h.mission_loads += 1;
    let id = h.mission_loads;
    h.mission_load = top.iter().rev().find(|n| n.object.name.eq_ignore_ascii_case("MissionGroup")).map(|n| MissionLoad::from_tree(id, n));
    h.scene_dirty = true;
    true
}

/// The mission's own defs (`BoostDef(name: "JumpBoost", parent:
/// "Boost1", ...)`, its `defs`). They are server defs like those of the
/// data scripts, so the mission goes into the def manifest: a remote
/// client declares them from its own copy of the mission
/// ([`declare_mission_defs`]) before it compares the def hash,
/// on this mission and on every later one (defs outlive the mission).
fn declare_defs(h: &mut EngineHost, path: &str, mission: &m::Mission) {
    if mission.defs.is_empty() {
        return;
    }
    crate::netgame::manifest_entry(h, path);
    for d in &mission.defs {
        crate::gameplay::defs::declare_def(h, d);
    }
    crate::gameplay::defs::refresh(h);
}

/// A remote client, for a mission in the server's def manifest:
/// declares that mission's defs (not its objects). False if `path`
/// is no mission this client has.
pub fn declare_mission_defs(h: &mut EngineHost, path: &str) -> bool {
    if !path.to_ascii_lowercase().ends_with(".ron") || !mission_exists(h, path) {
        return false;
    }
    let Some(mission) = parse_ron(h, path) else { return false };
    declare_defs(h, path, &mission);
    true
}

/// A mission object and its children as declared.
fn build(o: &m::MissionObject, next: &mut u32) -> MissionNode {
    *next += 1;
    let id = *next;
    let fields: Vec<(String, String)> = o.to_fields().into_iter().map(|f| (f.key(), f.value)).collect();
    let object = SceneObject { class: o.class().to_string(), name: o.name().to_string(), def: o.is_def(), parent: o.parent().map(str::to_string), fields, children: Vec::new() };
    let children = o.children().iter().map(|c| build(c, next)).collect();
    MissionNode { id, object, children }
}

/// `MissionGroup.delete()`: the loaded mission goes.
pub fn delete_mission(h: &mut EngineHost) {
    if h.mission_load.take().is_some() {
        h.scene_dirty = true;
    }
}

/// The objects of the loaded mission (flattened, fields as declared, the
/// last value of a field winning) and the defs, or `None` if no
/// mission is loaded.
pub fn mission_objects(h: &EngineHost) -> Option<(Vec<SceneObject>, Vec<SceneObject>)> {
    let load = h.mission_load.as_ref()?;
    let objects = load
        .nodes
        .iter()
        .map(|(_, o)| {
            let mut fields: Vec<(String, String)> = Vec::new();
            for (k, v) in &o.fields {
                fields.retain(|(x, _)| !x.eq_ignore_ascii_case(k));
                if !v.is_empty() {
                    fields.push((k.clone(), v.clone()));
                }
            }
            SceneObject { fields, children: Vec::new(), ..o.clone() }
        })
        .collect();
    let defs = h.defs.objects().cloned().collect();
    Some((objects, defs))
}

/// Directory `./` paths in the mission resolve against.
pub fn mission_dir(h: &EngineHost) -> PathBuf {
    let mut f = h.srv.get().map(|s| s.session.mission_file.clone()).unwrap_or_default();
    if f.is_empty() {
        // A remote client has no server: the mission it was sent.
        f = h.cl.mission_file.clone();
    }
    let root = h.fs.root();
    let rel = Path::new(&f).parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| tt_assets::layout::MISSIONS.into());
    root.join(&rel)
}

/// Builds the render scene of the loaded mission. With `dynamic_triggers`
/// power-ups and boost pads are left out (the game draws them animated as
/// moving shapes).
pub fn build_scene(h: &EngineHost, root: &Path, dynamic_triggers: bool) -> Option<Result<Scene, String>> {
    let (objects, defs) = mission_objects(h)?;
    let refs: Vec<&SceneObject> = objects.iter().collect();
    // Balls (`Flag`) are world entities, drawn where the simulation has them.
    let keep = |o: &SceneObject| !dynamic_triggers || !["PowerUp", "Boost", "Flag"].iter().any(|c| o.class.eq_ignore_ascii_case(c));
    Some(Scene::from_objects_filtered(root, &mission_dir(h), &refs, &defs, &keep))
}

pub fn game_relative(root: &Path, mission: &Path) -> Result<String, String> {
    let s = mission.to_string_lossy().replace('\\', "/");
    let rel_of = |file: &Path| -> Option<String> {
        let root = root.canonicalize().ok()?;
        let rel = tt_assets::find_ci(file)?.canonicalize().ok()?.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
        rel.to_ascii_lowercase().ends_with(".ron").then_some(rel)
    };
    let id = (!s.contains('/') && !s.contains('.')).then(|| tt_assets::layout::mission(&s));
    let candidates = [id, tt_assets::layout::migrate_mission(&s), Some(s.clone())];
    for c in candidates.into_iter().flatten() {
        if let Some(rel) = rel_of(&root.join(&c)) {
            return Ok(rel);
        }
    }
    rel_of(mission).ok_or_else(|| format!("{}: not a mission of the asset directory {}", mission.display(), root.display()))
}

/// `--mission`: loads the server defs and the mission directly (no
/// server, no client connection). Needs a booted engine.
pub fn load_dev_mission(engine: &mut Engine, rel: &str) -> Result<(), String> {
    let h = &mut engine.host;
    crate::gameplay::defs::load_server_data(h);
    delete_mission(h);
    h.srv.state_mut().session.mission_file = rel.to_string();
    if !exec_mission(h, rel) {
        return Err(format!("could not load {rel}"));
    }
    if h.mission_load.is_none() {
        return Err(format!("{rel} has no MissionGroup"));
    }
    h.scene_dirty = true;
    Ok(())
}

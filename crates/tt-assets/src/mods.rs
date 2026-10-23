//! Mods (`docs/modding.md`): folders that mirror the asset directory
//! (`models/`, `missions/`, `sound/`, `ui/`, `sky/`, `terrain/`, `data/`,
//! ...). An enabled mod overlays the game data: a file of the mod replaces
//! the file at the same relative path (case-insensitive, like every asset
//! path); of several mods the later one in the load order wins. Files at a
//! mod's top level (`mod.ron`, a readme, a picture) are not assets.
//!
//! The overlay is applied in one place for every loader: [`Overlay`] is
//! installed for an asset directory ([`install`]) and [`redirect`] maps a
//! path under that directory (or under one of the mods) to the file that
//! wins. [`crate::find_ci`], [`crate::resolve`] and [`crate::data::load`]
//! call it, so textures, shapes, sounds, terrains, missions and the data
//! files all see the mods; the engine's file index (`tt_engine::fs`)
//! lists the same files.
//!
//! A data file (`data/**/*.ron`) of a mod replaces the whole file (no
//! merging of single defs).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::Deserialize;

/// The optional description file at a mod's top level.
pub const MOD_FILE: &str = "mod.ron";

/// `mod.ron`: every field is optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModFile {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// Declared: `Some(true)` marks a mod as changing the game rules even
    /// without `data/` or `missions/` files. `Some(false)` cannot hide such
    /// files (they are detected: [`is_gameplay_file`]).
    pub affects_gameplay: Option<bool>,
    /// One of the example mods that ship with the game (`mods/` of the
    /// repository): shown with an "Example" tag. A copy made to start a
    /// mod of one's own drops it.
    pub example: bool,
}

/// An installed mod.
#[derive(Debug, Clone, PartialEq)]
pub struct ModInfo {
    /// The folder name: what the settings' `mods.enabled` lists.
    pub id: String,
    pub dir: PathBuf,
    /// `mod.ron`'s name, else the id.
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// Changes the game itself (`data/` or `missions/` files, or declared):
    /// network games need every player to have it ([`gameplay_hash`](Self::gameplay_hash)).
    pub affects_gameplay: bool,
    /// FNV-1a 64 of the mod's gameplay files (paths and contents); 0 when it
    /// has none.
    pub gameplay_hash: u64,
    /// (asset path as on disk, file) of every asset of the mod, sorted.
    pub files: Vec<(String, PathBuf)>,
    /// Problems found reading it (an unreadable `mod.ron`, ...).
    pub warnings: Vec<String>,
    /// `mod.ron` marks it as an example mod ([`ModFile::example`]).
    pub example: bool,
}

impl ModInfo {
    /// "name version" for messages.
    pub fn label(&self) -> String {
        if self.version.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.version)
        }
    }
}

/// Whether an asset path changes the game rather than its looks: the data
/// files (`data/`) and the missions' object files and terrains
/// (`missions/*.ron`, `missions/*.ter`). Textures, sounds, models, skies,
/// the menu art and the mission pictures are cosmetic.
pub fn is_gameplay_file(rel: &str) -> bool {
    let l = rel.replace('\\', "/").to_ascii_lowercase();
    l.starts_with("data/") || (l.starts_with("missions/") && (l.ends_with(".ron") || l.ends_with(".ter")))
}

/// Reads the mod in folder `dir` (`None` when it is not a folder).
pub fn read_mod(dir: &Path) -> Option<ModInfo> {
    if !dir.is_dir() {
        return None;
    }
    let id = dir.file_name()?.to_string_lossy().into_owned();
    let mut warnings = Vec::new();
    let mut desc = ModFile::default();
    let mut files = Vec::new();
    for (rel, path) in crate::layout::files(dir) {
        if !rel.contains('/') {
            if rel.eq_ignore_ascii_case(MOD_FILE) {
                match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| crate::data::from_ron::<ModFile>(&t)) {
                    Ok(d) => desc = d,
                    Err(e) => warnings.push(format!("{}: {e}", path.display())),
                }
            }
            continue;
        }
        files.push((rel, path));
    }
    let mut gameplay: Vec<&(String, PathBuf)> = files.iter().filter(|(r, _)| is_gameplay_file(r)).collect();
    gameplay.sort_by_key(|(r, _)| r.to_ascii_lowercase());
    let mut gameplay_hash = 0;
    if !gameplay.is_empty() {
        gameplay_hash = crate::FNV_START;
        for (r, p) in &gameplay {
            crate::fnv1a(&mut gameplay_hash, r.to_ascii_lowercase().as_bytes());
            crate::fnv1a(&mut gameplay_hash, &[0]);
            crate::fnv1a(&mut gameplay_hash, &std::fs::read(p).unwrap_or_default());
            crate::fnv1a(&mut gameplay_hash, &[0]);
        }
    }
    if desc.affects_gameplay == Some(false) && !gameplay.is_empty() {
        warnings.push(format!("{id}: affects_gameplay is false, but the mod has data or mission files ({}): it affects the game", gameplay[0].0));
    }
    let affects_gameplay = !gameplay.is_empty() || desc.affects_gameplay == Some(true);
    if affects_gameplay && gameplay_hash == 0 {
        // Declared without files: identified by its name and version.
        gameplay_hash = crate::FNV_START;
        crate::fnv1a(&mut gameplay_hash, format!("{}\0{}", desc.name, desc.version).as_bytes());
    }
    let name = if desc.name.trim().is_empty() { id.clone() } else { desc.name.trim().to_string() };
    Some(ModInfo { id, dir: dir.to_path_buf(), name, version: desc.version.trim().into(), author: desc.author.trim().into(), description: desc.description.trim().into(), affects_gameplay, gameplay_hash, files, warnings, example: desc.example })
}

/// Every mod in the folders `dirs` ([`crate::layout::mod_dirs`]): each
/// sub-folder is a mod, its folder name the id; an id found again in a
/// later folder is skipped. Sorted by id (case-insensitive).
pub fn discover(dirs: &[PathBuf]) -> Vec<ModInfo> {
    let mut out: Vec<ModInfo> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        subs.sort();
        for s in subs {
            let hidden = s.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if hidden {
                continue;
            }
            if let Some(m) = read_mod(&s) {
                if !out.iter().any(|o| o.id.eq_ignore_ascii_case(&m.id)) {
                    out.push(m);
                }
            }
        }
    }
    out.sort_by_key(|m| m.id.to_ascii_lowercase());
    out
}

/// The enabled mods in load order: the ids of `enabled` (the settings'
/// `mods.enabled`, first = lowest priority) found in `installed`; unknown
/// ids are returned second.
pub fn enabled<'a>(installed: &'a [ModInfo], enabled: &[String]) -> (Vec<&'a ModInfo>, Vec<String>) {
    let mut on = Vec::new();
    let mut missing = Vec::new();
    for id in enabled {
        match installed.iter().find(|m| m.id.eq_ignore_ascii_case(id)) {
            Some(m) if !on.iter().any(|o: &&ModInfo| o.id == m.id) => on.push(m),
            Some(_) => {}
            None => missing.push(id.clone()),
        }
    }
    (on, missing)
}

/// The files the enabled mods put over an asset directory.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    root: PathBuf,
    /// The mods' folders, in load order.
    layers: Vec<PathBuf>,
    /// lowercase asset path -> (asset path as in the mod, file); the last
    /// mod having a path wins.
    files: HashMap<String, (String, PathBuf)>,
}

impl Overlay {
    /// The overlay of `mods` (load order: later ones win) over the asset
    /// directory `root`.
    pub fn new(root: &Path, mods: &[&ModInfo]) -> Overlay {
        let mut files = HashMap::new();
        for m in mods {
            for (rel, p) in &m.files {
                files.insert(rel.to_ascii_lowercase(), (rel.clone(), p.clone()));
            }
        }
        Overlay { root: root.to_path_buf(), layers: mods.iter().map(|m| m.dir.clone()).collect(), files }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// The mod file for asset path `rel`, if a mod has one.
    pub fn get(&self, rel: &str) -> Option<&Path> {
        self.files.get(&rel.replace('\\', "/").to_ascii_lowercase()).map(|(_, p)| p.as_path())
    }

    /// (asset path, file) of every file the mods provide.
    pub fn files(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.files.values().map(|(r, p)| (r.as_str(), p.as_path()))
    }

    /// The file `path` means with the mods applied, `None` when it is not
    /// under the asset directory or a mod, or no mod changes it.
    pub fn redirect(&self, path: &Path) -> Option<PathBuf> {
        if let Some(rel) = rel_under(path, &self.root) {
            return self.get(&rel).map(Path::to_path_buf);
        }
        let rel = self.layers.iter().find_map(|l| rel_under(path, l))?;
        // A file named relative to a mod's file (`./x` of a mod mission, a
        // texture next to a mod's shape): the winning file, else the game's.
        Some(self.get(&rel).map_or_else(|| self.root.join(&rel), Path::to_path_buf))
    }
}

/// `path` relative to `dir`, `/`-separated; `None` outside it (or with `..`).
fn rel_under(path: &Path, dir: &Path) -> Option<String> {
    let rest = path.strip_prefix(dir).ok()?;
    let mut parts = Vec::new();
    for c in rest.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static REGISTRY: RwLock<Vec<Arc<Overlay>>> = RwLock::new(Vec::new());
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Makes `overlay` the one of its asset directory for every loader of this
/// process (an empty overlay removes the directory's); a change moves
/// [`generation`] on.
pub fn install(overlay: Overlay) {
    let mut r = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    let before = r.iter().find(|o| o.root == overlay.root).map(|o| (**o).clone()).unwrap_or_else(|| Overlay { root: overlay.root.clone(), ..Default::default() });
    if before == overlay {
        return;
    }
    r.retain(|o| o.root != overlay.root);
    if !overlay.is_empty() {
        r.push(Arc::new(overlay));
    }
    ACTIVE.store(!r.is_empty(), Ordering::Release);
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Changes whenever [`install`] changes what a path means: a process-wide
/// cache of files by path (the parsed shapes) drops what it holds when
/// this differs from its value at the last lookup (a game restarted with
/// other mods, in the same process).
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// The overlay installed for the asset directory `root`, if any.
pub fn installed(root: &Path) -> Option<Arc<Overlay>> {
    if !ACTIVE.load(Ordering::Acquire) {
        return None;
    }
    REGISTRY.read().unwrap_or_else(|e| e.into_inner()).iter().find(|o| o.root == root).cloned()
}

/// The file `path` means with the installed mods applied ([`Overlay::redirect`]);
/// `None`: the path itself.
pub fn redirect(path: &Path) -> Option<PathBuf> {
    if !ACTIVE.load(Ordering::Acquire) {
        return None;
    }
    REGISTRY.read().unwrap_or_else(|e| e.into_inner()).iter().find_map(|o| o.redirect(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn reads_mods_and_detects_gameplay() {
        let base = std::env::temp_dir().join(format!("tt-assets-mods-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        put(&mods.join("skins/models/tanks/tank01.png"), "skin");
        put(&mods.join("skins/mod.ron"), "(name: \"Skins\", version: \"1.2\", author: \"Me\", description: \"Shiny\", example: true)");
        put(&mods.join("skins/README.md"), "not an asset");
        put(&mods.join("rules/data/tanks.ron"), "[]");
        put(&mods.join("rules/mod.ron"), "(affects_gameplay: Some(false))");
        put(&mods.join("maps/missions/Mine.ron"), "()");
        put(&mods.join("maps/missions/Mine.png"), "png");
        let other = base.join("other");
        put(&other.join("Skins/ui/x.png"), "shadowed by the first folder's skins");
        let all = discover(&[mods.clone(), other]);
        let ids: Vec<&str> = all.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["maps", "rules", "skins"]);
        let skins = &all[2];
        assert_eq!((skins.name.as_str(), skins.version.as_str(), skins.author.as_str()), ("Skins", "1.2", "Me"));
        assert!(!skins.affects_gameplay && skins.example);
        assert_eq!(skins.gameplay_hash, 0);
        assert_eq!(skins.files.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>(), ["models/tanks/tank01.png"]);
        let rules = &all[1];
        assert!(rules.affects_gameplay && rules.gameplay_hash != 0);
        assert_eq!(rules.name, "rules");
        assert!(!rules.example);
        assert_eq!(rules.warnings.len(), 1, "{:?}", rules.warnings);
        assert!(all[0].affects_gameplay);
        // The hash follows the contents.
        let h = all[0].gameplay_hash;
        put(&mods.join("maps/missions/Mine.png"), "another picture");
        assert_eq!(read_mod(&mods.join("maps")).unwrap().gameplay_hash, h);
        put(&mods.join("maps/missions/Mine.ron"), "(objects: [])");
        assert_ne!(read_mod(&mods.join("maps")).unwrap().gameplay_hash, h);
        let (on, missing) = enabled(&all, &["skins".into(), "gone".into(), "MAPS".into()]);
        assert_eq!(on.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["skins", "maps"]);
        assert_eq!(missing, ["gone"]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn overlay_resolution_order() {
        let base = std::env::temp_dir().join(format!("tt-assets-mods-overlay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("assets");
        put(&root.join("models/tanks/Tank01.png"), "game");
        put(&root.join("models/tanks/tank01.dts"), "game shape");
        put(&root.join("sound/boom.ogg"), "game");
        put(&root.join("data/pickups.ron"), "game");
        let mods = base.join("mods");
        put(&mods.join("a/models/tanks/tank01.png"), "a");
        put(&mods.join("a/sound/boom.ogg"), "a");
        put(&mods.join("a/models/tanks/tank01.dts"), "a shape");
        put(&mods.join("b/MODELS/tanks/TANK01.png"), "b");
        put(&mods.join("off/data/pickups.ron"), "disabled mod");
        let all = discover(std::slice::from_ref(&mods));
        let (on, _) = enabled(&all, &["a".into(), "b".into()]);
        install(Overlay::new(&root, &on));
        let read = |p: &Path| std::fs::read_to_string(crate::find_ci(p).unwrap()).unwrap();
        // b is later in the load order: it wins over a; both over the game.
        assert_eq!(read(&root.join("models/tanks/tank01.png")), "b");
        assert_eq!(read(&root.join("sound/boom.ogg")), "a");
        assert_eq!(std::fs::read_to_string(crate::resolve(&root, &root, "sound/boom.ogg")).unwrap(), "a");
        assert_eq!(crate::find_texture(&root.join("models/tanks/tank01")).map(|p| std::fs::read_to_string(p).unwrap()).as_deref(), Some("b"));
        // A disabled mod is ignored.
        assert_eq!(read(&root.join("data/pickups.ron")), "game");
        // Next to a mod's shape: the winning texture, else the game's file.
        let shape = crate::find_ci(&root.join("models/tanks/tank01.dts")).unwrap();
        assert!(shape.starts_with(mods.join("a")));
        assert_eq!(read(&shape.with_file_name("tank01.png")), "b");
        put(&root.join("models/tanks/tread.png"), "game tread");
        assert_eq!(read(&shape.with_file_name("tread.png")), "game tread");
        // Another asset directory is not affected.
        let other = base.join("other");
        put(&other.join("sound/boom.ogg"), "other");
        assert_eq!(read(&other.join("sound/boom.ogg")), "other");
        // Swapping the order; then no mods at all.
        let (on, _) = enabled(&all, &["b".into(), "a".into()]);
        install(Overlay::new(&root, &on));
        assert_eq!(read(&root.join("models/tanks/tank01.png")), "a");
        install(Overlay::new(&root, &[]));
        assert_eq!(read(&root.join("models/tanks/tank01.png")), "game");
        assert!(installed(&root).is_none());
        std::fs::remove_dir_all(&base).unwrap();
    }
}

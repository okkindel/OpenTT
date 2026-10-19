//! Resource file system: reads the game's data from the asset directory
//! (`assets/`, [`tt_assets::layout`]), writes to a per-user data directory.
//! Files written there shadow asset files of the same name. The saves
//! (`settings.ron`, `players.ron`, ..., `crate::shell`) are read from the
//! data dir only ([`GameFs::read_data`]).
//!
//! The enabled mods ([`tt_assets::mods`], `docs/modding.md`) overlay the
//! asset directory: [`GameFs::with_mods`] lists their files in place of the
//! game's and installs the same overlay for the loaders that open files by
//! path (`tt_assets::find_ci` / `resolve` / `data::load`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct GameFs {
    assets: PathBuf,
    data: PathBuf,
    /// lowercase asset path -> (asset path, file) in the asset directory,
    /// the mods' files in place of the game's.
    files: HashMap<String, (String, PathBuf)>,
    /// The asset directory's own files (no mods).
    game: HashMap<String, (String, PathBuf)>,
    /// The enabled mods, in load order.
    mods: Vec<tt_assets::mods::ModInfo>,
    /// lowercase relative path -> relative path as on disk (data dir).
    user: HashMap<String, String>,
}

/// Normalises a script path: `\` -> `/`, drops `./` and leading `/`.
/// `None` for paths escaping the root (`..`) or empty ones.
pub fn normalize(path: &str) -> Option<String> {
    let p = path.replace('\\', "/");
    let mut parts = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            ".." => return None,
            c if c.contains(':') => return None,
            c => parts.push(c),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The files under `root` by lowercase relative path (`/`-separated), each
/// to its path as on disk; top-level entries named in `skip_top` (any
/// case) are left out.
pub(crate) fn index_dir(root: &Path, skip_top: &[&str]) -> HashMap<String, String> {
    fn walk(root: &Path, dir: &Path, skip_top: &[&str], out: &mut HashMap<String, String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if dir == root && skip_top.iter().any(|d| e.file_name().to_string_lossy().eq_ignore_ascii_case(d)) {
                continue;
            }
            if p.is_dir() {
                walk(root, &p, skip_top, out);
            } else if let Ok(rel) = p.strip_prefix(root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                out.insert(rel.to_ascii_lowercase(), rel);
            }
        }
    }
    let mut out = HashMap::new();
    walk(root, root, skip_top, &mut out);
    out
}

impl GameFs {
    /// The asset directory `assets` and the user data directory `data`, no mods.
    pub fn new(assets: &Path, data: &Path) -> GameFs {
        GameFs::with_mods(assets, data, Vec::new())
    }

    /// With the enabled `mods` (load order: later ones win) over the asset
    /// directory; installs their overlay for the asset directory
    /// ([`tt_assets::mods::install`], replacing an earlier one).
    pub fn with_mods(assets: &Path, data: &Path, mods: Vec<tt_assets::mods::ModInfo>) -> GameFs {
        let game: HashMap<String, (String, PathBuf)> = tt_assets::layout::files(assets).into_iter().map(|(g, p)| (g.to_ascii_lowercase(), (g, p))).collect();
        let overlay = tt_assets::mods::Overlay::new(assets, &mods.iter().collect::<Vec<_>>());
        let mut files = game.clone();
        for (rel, p) in overlay.files() {
            files.insert(rel.to_ascii_lowercase(), (rel.to_string(), p.to_path_buf()));
        }
        tt_assets::mods::install(overlay);
        // The mods and an import there are not the player's files.
        let user = index_dir(data, &[tt_assets::layout::MODS, tt_assets::layout::IMPORTED]);
        GameFs { assets: assets.to_path_buf(), data: data.to_path_buf(), files, game, mods, user }
    }

    /// The enabled mods, in load order.
    pub fn mods(&self) -> &[tt_assets::mods::ModInfo] {
        &self.mods
    }

    /// Every version of an asset file, lowest priority first: the game's,
    /// then each enabled mod's (for files merged rather than replaced:
    /// `missions/catalog.ron`).
    pub fn read_layers(&self, path: &str) -> Vec<Vec<u8>> {
        let Some(key) = normalize(path).map(|k| k.to_ascii_lowercase()) else { return Vec::new() };
        let mut out: Vec<Vec<u8>> = self.game.get(&key).and_then(|(_, p)| std::fs::read(p).ok()).into_iter().collect();
        for m in &self.mods {
            if let Some((_, p)) = m.files.iter().find(|(r, _)| r.eq_ignore_ascii_case(&key)) {
                out.extend(std::fs::read(p).ok());
            }
        }
        out
    }

    /// The asset directory.
    pub fn root(&self) -> &Path {
        &self.assets
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    /// Absolute path of a file (data dir, then assets).
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let key = normalize(path)?.to_ascii_lowercase();
        if let Some(rel) = self.user.get(&key) {
            return Some(self.data.join(rel));
        }
        self.files.get(&key).map(|(_, p)| p.clone())
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.resolve(path)?).ok()
    }

    /// A file of the data dir only (the saves; never an asset).
    pub fn read_data(&self, path: &str) -> Option<Vec<u8>> {
        let key = normalize(path)?.to_ascii_lowercase();
        std::fs::read(self.data.join(self.user.get(&key)?)).ok()
    }

    pub fn exists(&self, path: &str) -> bool {
        normalize(path).map(|k| k.to_ascii_lowercase()).is_some_and(|k| self.user.contains_key(&k) || self.files.contains_key(&k))
    }

    /// Writes (or appends to) a file in the data dir. Never touches the assets.
    pub fn write(&mut self, path: &str, data: &[u8], append: bool) -> std::io::Result<PathBuf> {
        let rel = normalize(path).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("bad path {path:?}")))?;
        let key = rel.to_ascii_lowercase();
        // Reuse the spelling of an existing data file, else the game file's.
        let rel = self.user.get(&key).or_else(|| self.files.get(&key).map(|(r, _)| r)).cloned().unwrap_or(rel);
        let full = self.data.join(&rel);
        if let Some(dir) = full.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if append {
            use std::io::Write;
            std::fs::OpenOptions::new().create(true).append(true).open(&full)?.write_all(data)?;
        } else {
            std::fs::write(&full, data)?;
        }
        self.user.insert(key, rel);
        Ok(full)
    }

    /// All visible files (relative paths, data dir spelling preferred).
    pub fn list(&self) -> Vec<String> {
        let mut out: HashMap<&String, &String> = HashMap::new();
        out.extend(self.files.iter().map(|(k, (rel, _))| (k, rel)));
        out.extend(self.user.iter());
        let mut v: Vec<String> = out.into_values().cloned().collect();
        v.sort();
        v
    }

    /// The visible files matching `pattern` (`*`, `?`; case-insensitive), sorted:
    /// what `findFirstFile` / `findNextFile` went through.
    pub fn find_files(&self, pattern: &str) -> Vec<String> {
        self.list().into_iter().filter(|f| wildcard_match(pattern, f)).collect()
    }
}

pub fn wildcard_match(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.chars().map(|c| c.to_ascii_lowercase()).collect();
    let t: Vec<char> = s.chars().map(|c| c.to_ascii_lowercase()).collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Default user data directory: `$XDG_DATA_HOME/opentt` or
/// `~/.local/share/opentt`
/// ([`tt_assets::layout::user_data_dir`]).
pub fn default_data_dir() -> PathBuf {
    tt_assets::layout::user_data_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize("./save/client/prefs.txt").as_deref(), Some("save/client/prefs.txt"));
        assert_eq!(normalize("save\\client\\x.txt").as_deref(), Some("save/client/x.txt"));
        assert_eq!(normalize("/a//b").as_deref(), Some("a/b"));
        assert_eq!(normalize("../etc/passwd"), None);
        assert_eq!(normalize("c:/x"), None);
        assert_eq!(normalize(""), None);
    }

    #[test]
    fn reads_assets_writes_data_dir() {
        let base = std::env::temp_dir().join(format!("tt-engine-fs-{}", std::process::id()));
        let (assets, data) = (base.join("assets"), base.join("data"));
        std::fs::create_dir_all(assets.join("missions")).unwrap();
        std::fs::create_dir_all(assets.join("ui")).unwrap();
        std::fs::write(assets.join("missions/A.ron"), b"assets").unwrap();
        std::fs::write(assets.join("ui/credits.txt"), b"c").unwrap();
        let mut fs = GameFs::new(&assets, &data);
        assert_eq!(fs.read("missions/a.RON").unwrap(), b"assets");
        assert_eq!(fs.read("./ui/CREDITS.txt").unwrap(), b"c");
        assert!(fs.exists("MISSIONS/A.ron"));
        assert!(fs.read("save/client/init.txt").is_none());
        assert_eq!(fs.list(), vec!["missions/A.ron".to_string(), "ui/credits.txt".to_string()]);
        // Writes go to the data dir and shadow the asset; the asset is never written.
        fs.write("missions/a.ron", b"user", false).unwrap();
        fs.write("missions/a.ron", b"!", true).unwrap();
        assert_eq!(fs.read("missions/a.ron").unwrap(), b"user!");
        assert_eq!(std::fs::read(assets.join("missions/A.ron")).unwrap(), b"assets");
        assert!(data.join("missions/A.ron").exists());
        assert!(fs.write("../escape.txt", b"x", false).is_err());
        assert_eq!(fs.find_files("missions/*.RON"), vec!["missions/A.ron".to_string()]);
        assert!(wildcard_match("missions/TT*_*.ron", "missions/tt1_2.ron"));
        assert!(!wildcard_match("missions/TT*_*.ron", "missions/target.ron"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}

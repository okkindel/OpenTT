//! The user data directory's layout (docs/layout.md § User data) and the
//! one-time migration of older layouts into it, run by [`crate::Engine::new`]
//! before anything reads or writes there.
//!
//! ```text
//! <data>/settings.ron  players.ron  hiscores.ron  progress.ron
//!        binds.ron     banlist.ron
//!        logs/console.log
//!        screenshots/screenshot_00001.png ...
//!        legacy/       files of an older layout, kept as backups (never read)
//! ```
//!
//! The import of the game's saves, step by step (each only when its new file is
//! missing):
//!
//! 4. every old file whose new file now exists is moved under `legacy/`
//!    (same relative path; `.1`, `.2`, ... appended if that name is taken):
//!    when both exist, the new file wins and the old one is archived, never
//!    merged;
//! 5. the old directories (`game/client`, `game/server`, `game`,
//!    `common/server`, `common`) are removed when empty.
//!
//! Nothing is overwritten or deleted: files are only created, renamed into
//! free names or (directories) removed when empty. A file that cannot be
//! converted stays where it is (reported), so the next start tries again.
//! A second run finds nothing to do. Paths are matched case-insensitively,
//! like the game's file system ([`crate::fs`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tt_gameplay::client::binds::Binds;
use tt_shell::banlist::BanList;
use tt_shell::save::{self, SaveStore};

/// What [`migrate`] did: one line per step taken, one per problem.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Migration {
    pub done: Vec<String>,
    pub errors: Vec<String>,
}

const SAVES: &[(&str, &[&str])] = &[
    (save::PLAYERS, &[save::LEGACY_PLAYERS]),
    (save::HISCORES, &[save::LEGACY_HISCORES]),
    (save::PROGRESS, &[save::LEGACY_PROGRESS]),
    (save::SETTINGS, &[save::LEGACY_CLIENT_PREFS, save::LEGACY_SERVER_PREFS]),
];

/// Old file -> the new file that supersedes it (archived once that exists).
const SUPERSEDED: &[(&str, &str)] = &[
    (save::LEGACY_PLAYERS, save::PLAYERS),
    (save::LEGACY_HISCORES, save::HISCORES),
    (save::LEGACY_PROGRESS, save::PROGRESS),
    (save::LEGACY_CLIENT_PREFS, save::SETTINGS),
    (save::LEGACY_SERVER_PREFS, save::SETTINGS),
    (save::LEGACY_CONFIG, save::BINDS),
    (save::LEGACY_BANLISTS[0], save::BANLIST),
    (save::LEGACY_BANLISTS[1], save::BANLIST),
];

/// The old layout's directories, removed when empty (deepest first).
const OLD_DIRS: &[&str] = &["game/client", "game/server", "game", "common/server", "common"];

/// The data directory's files by lowercase relative path.
struct Dir {
    root: PathBuf,
    files: HashMap<String, String>,
}

impl Dir {
    fn new(root: &Path) -> Dir {
        Dir { root: root.to_path_buf(), files: crate::fs::index_dir(root, &[]) }
    }

    /// The file's relative path as on disk.
    fn find(&self, rel: &str) -> Option<&str> {
        self.files.get(&rel.to_ascii_lowercase()).map(String::as_str)
    }

    fn has(&self, rel: &str) -> bool {
        self.find(rel).is_some()
    }

    fn bytes(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(self.find(rel)?)).ok()
    }

    /// A new file (never over an existing one).
    fn create(&mut self, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
        if self.has(rel) {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{rel} exists")));
        }
        let p = self.root.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::OpenOptions::new().write(true).create_new(true).open(&p).and_then(|mut f| std::io::Write::write_all(&mut f, bytes))?;
        self.files.insert(rel.to_ascii_lowercase(), rel.to_string());
        Ok(())
    }

    /// Renames the file `from` to `to` (a free name).
    fn rename(&mut self, from: &str, to: &str) -> std::io::Result<()> {
        let from = self.find(from).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, from.to_string()))?.to_string();
        if self.has(to) || self.root.join(to).exists() {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{to} exists")));
        }
        let dest = self.root.join(to);
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::rename(self.root.join(&from), &dest)?;
        self.files.remove(&from.to_ascii_lowercase());
        self.files.insert(to.to_ascii_lowercase(), to.to_string());
        Ok(())
    }

    /// A free name: `rel`, else `rel.1`, `rel.2`, ...
    fn free(&self, rel: &str) -> String {
        std::iter::once(rel.to_string()).chain((1..).map(|n| format!("{rel}.{n}"))).find(|c| !self.has(c) && !self.root.join(c).exists()).unwrap()
    }

    /// Moves `rel` under `legacy/`; the new relative path.
    fn archive(&mut self, rel: &str) -> std::io::Result<String> {
        let on_disk = self.find(rel).unwrap_or(rel).to_string();
        let to = self.free(&format!("{}/{on_disk}", save::LEGACY_DIR));
        self.rename(&on_disk, &to)?;
        Ok(to)
    }

    /// A directory's path as on disk (components matched case-insensitively).
    fn find_dir(&self, rel: &str) -> Option<PathBuf> {
        let mut p = self.root.clone();
        for c in rel.split('/') {
            let hit = std::fs::read_dir(&p).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(c) && e.path().is_dir())?;
            p = hit.path();
        }
        Some(p)
    }
}

impl SaveStore for Dir {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        self.bytes(rel)
    }
    fn write(&mut self, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.create(rel, bytes)
    }
}

/// Moves an older layout of the data directory `data` into the current one
/// (the module docs). `assets`: the asset directory. A missing `data` is a
/// fresh install: nothing is created.
pub fn migrate(data: &Path, assets: &Path) -> Migration {
    let mut m = Migration::default();
    if !data.is_dir() {
        return m;
    }
    let mut d = Dir::new(data);
    import_saves(&mut d, &mut m);
    migrate_binds(&mut d, assets, &mut m);
    migrate_banlist(&mut d, &mut m);
    for (old, new) in SUPERSEDED {
        if d.has(old) && d.has(new) {
            match d.archive(old) {
                Ok(to) => m.done.push(format!("moved {old} to {to} ({new} supersedes it)")),
                Err(e) => m.errors.push(format!("{old}: {e} (left in place)")),
            }
        }
    }
    for dir in OLD_DIRS {
        let Some(p) = d.find_dir(dir) else { continue };
        match std::fs::remove_dir(&p) {
            Ok(()) => m.done.push(format!("removed the empty directory {dir}")),
            Err(_) => m.done.push(format!("kept the directory {dir}: it holds files the game does not know")),
        }
    }
    m
}

fn import_saves(d: &mut Dir, m: &mut Migration) {
    if !SAVES.iter().any(|(new, old)| !d.has(new) && old.iter().any(|o| d.has(o))) {
        return;
    }
    let (_, r) = save::load(d);
    for (from, to) in r.imported {
        m.done.push(format!("imported {} into {to}", from.join(" + ")));
    }
    m.errors.extend(r.errors);
}

fn migrate_binds(d: &mut Dir, assets: &Path, m: &mut Migration) {
    if d.has(save::BINDS) {
        return;
    }
    let Some(config) = d.bytes(save::LEGACY_CONFIG) else { return };
    let defaults_path = assets.join("data/binds.ron");
    let mut b = match std::fs::read_to_string(&defaults_path).map_err(|e| e.to_string()).and_then(|t| Binds::parse(&t)) {
        Ok(b) => b,
        Err(e) => {
            m.errors.push(format!("{}: {e}: {} not imported (left in place)", defaults_path.display(), save::LEGACY_CONFIG));
            return;
        }
    };
    for l in b.import_config_cs(&tt_game::fmt::latin1(&config)) {
        // `ActionMap::save` recreates the map first; the bindings replace the defaults' anyway.
        let recreate = l.starts_with("new ActionMap(") || (l.starts_with("if (isObject(") && l.ends_with(".delete();"));
        if !recreate {
            m.errors.push(format!("{}: not imported: {l}", save::LEGACY_CONFIG));
        }
    }
    match d.create(save::BINDS, b.to_ron().as_bytes()) {
        Ok(()) => m.done.push(format!("imported {} into {}", save::LEGACY_CONFIG, save::BINDS)),
        Err(e) => m.errors.push(format!("{}: {e}", save::BINDS)),
    }
}

fn migrate_banlist(d: &mut Dir, m: &mut Migration) {
    if d.has(save::BANLIST) {
        return;
    }
    let sources: Vec<&str> = save::LEGACY_BANLISTS.iter().copied().filter(|f| d.has(f)).collect();
    if sources.is_empty() {
        return;
    }
    let mut list = BanList::default();
    for f in &sources {
        tt_shell::legacy::import_banlist(&mut list, &tt_game::fmt::latin1(&d.bytes(f).unwrap_or_default()));
    }
    match d.create(save::BANLIST, list.to_ron().as_bytes()) {
        Ok(()) => m.done.push(format!("converted {} into {} ({} bans)", sources.join(" + "), save::BANLIST, list.bans.len())),
        Err(e) => m.errors.push(format!("{}: {e}", save::BANLIST)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tt_shell::{HiScores, Players, Prefs};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tt-userdata-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    /// Every file (relative path -> content).
    fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        Dir::new(dir).files.into_values().map(|r| (r.clone(), std::fs::read(dir.join(&r)).unwrap())).collect()
    }

    fn assets() -> PathBuf {
        tt_assets::layout::default_dir()
    }

    fn text(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    /// Where the migration archives the old file `rel`.
    fn legacy(rel: &str) -> String {
        format!("{}/{rel}", save::LEGACY_DIR)
    }

    /// Old ban list files of one ban each.
    fn ban_a() -> String {
        tt_shell::legacy::banlist_file(&[tt_shell::banlist::Ban { guid: "0".into(), ip: "10.0.0.2".into(), until: 1800001825 }])
    }

    fn original_saves(dir: &Path) -> (Players, HiScores) {
        let mut prefs = Prefs::defaults();
        let mut p = Players::new();
        p.create("Bait", &mut prefs).unwrap();
        p.record_mp_game(2.0, 3.0);
        p.prepare_save();
        let mut h = HiScores::default();
        h.insert("Bait", 100000.0, 4);
        put(dir, save::LEGACY_PLAYERS, p.to_legacy().as_bytes());
        put(dir, save::LEGACY_HISCORES, h.to_legacy().as_bytes());
        put(dir, save::LEGACY_CLIENT_PREFS, tt_shell::legacy::prefs_file(&[("player.name", "Bait"), ("server.port", "28000")]).as_bytes());
        put(dir, save::LEGACY_SERVER_PREFS, tt_shell::legacy::prefs_file(&[("server.port", "28005")]).as_bytes());
        put(dir, save::LEGACY_CONFIG, b"if (isObject(moveMap)) moveMap.delete();\r\nnew ActionMap(moveMap);\r\nmoveMap.bind(keyboard, \"f9\", moveforward);\r\n");
        put(dir, save::LEGACY_BANLISTS[0], ban_a().as_bytes());
        put(dir, save::LEGACY_BANLISTS[1], ban_a().as_bytes());
        (p, h)
    }

    fn assert_new_layout_only(dir: &Path) {
        for d in OLD_DIRS {
            assert!(!dir.join(d).exists(), "{d} removed");
        }
        for f in tree(dir).keys() {
            let top_ok = ["settings.ron", "players.ron", "hiscores.ron", "progress.ron", "binds.ron", "banlist.ron"].contains(&f.as_str());
            assert!(top_ok || f.starts_with("legacy/") || f.starts_with("logs/") || f.starts_with("screenshots/"), "{f} in the new layout");
        }
    }

    #[test]
    fn fresh_install_is_left_alone() {
        let missing = std::env::temp_dir().join(format!("tt-userdata-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        assert_eq!(migrate(&missing, &assets()), Migration::default());
        assert!(!missing.exists(), "nothing created");
        let empty = temp("empty");
        assert_eq!(migrate(&empty, &assets()), Migration::default());
        assert!(tree(&empty).is_empty());
        std::fs::remove_dir_all(&empty).unwrap();
    }

    #[test]
    fn original_cs_saves_land_in_the_new_layout() {
        tt_assets::require_game_data!();
        let dir = temp("cs");
        let (p, h) = original_saves(&dir);
        let before = tree(&dir);
        let m = migrate(&dir, &assets());
        assert!(m.errors.is_empty(), "{:?}", m.errors);
        let s = save::read_dir(&dir);
        assert_eq!(s.players.vars.to_text(""), p.vars.to_text(""));
        assert_eq!(s.hiscores.entries(), h.entries());
        let st = s.settings.unwrap();
        assert_eq!((st.player.name.as_deref(), st.server.port), (Some("Bait"), Some(28005)));
        let b = Binds::parse(&text(&dir, "binds.ron")).unwrap();
        let mv = b.map("MoveMap").unwrap();
        assert!(mv.binds.iter().any(|x| x.key == "f9" && x.action.function() == "moveforward"));
        assert!(mv.binds.iter().any(|x| x.key == "w"), "the defaults stay");
        // The two ban lists, merged.
        let bans = BanList::from_ron(&text(&dir, "banlist.ron")).unwrap();
        assert_eq!(bans.bans.len(), 1);
        assert_eq!((bans.bans[0].ip.as_str(), bans.bans[0].until), ("10.0.0.2", 1800001825));
        let after = tree(&dir);
        for (f, bytes) in &before {
            assert_eq!(after.get(&format!("legacy/{f}")), Some(bytes), "{f} archived");
        }
        assert_new_layout_only(&dir);
        assert!(m.done.iter().any(|l| *l == format!("imported {} + {} into settings.ron", save::LEGACY_CLIENT_PREFS, save::LEGACY_SERVER_PREFS)), "{:?}", m.done);
        // Idempotent.
        assert_eq!(migrate(&dir, &assets()), Migration::default());
        assert_eq!(tree(&dir), after);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn new_files_win_and_old_ones_are_archived() {
        let dir = temp("both");
        put(&dir, "players.ron", b"new players");
        put(&dir, save::LEGACY_PLAYERS, b"old players");
        put(&dir, "binds.ron", b"(maps: [])");
        put(&dir, save::LEGACY_CONFIG, b"moveMap.bind(keyboard, \"f9\", moveforward);\n");
        put(&dir, "banlist.ron", b"BanList(bans: [])");
        put(&dir, save::LEGACY_BANLISTS[1], ban_a().as_bytes());
        // An earlier migration's archive: the name is taken.
        put(&dir, &legacy(save::LEGACY_PLAYERS), b"older players");
        // A file the game does not know keeps its directory.
        put(&dir, "game/server/notes.txt", b"mine");
        let m = migrate(&dir, &assets());
        assert!(m.errors.is_empty(), "{:?}", m.errors);
        assert_eq!(text(&dir, "players.ron"), "new players");
        assert_eq!(text(&dir, "binds.ron"), "(maps: [])");
        assert_eq!(text(&dir, "banlist.ron"), "BanList(bans: [])");
        assert_eq!(text(&dir, &legacy(save::LEGACY_PLAYERS)), "older players");
        assert_eq!(text(&dir, &format!("{}.1", legacy(save::LEGACY_PLAYERS))), "old players");
        assert!(dir.join(legacy(save::LEGACY_CONFIG)).exists());
        assert_eq!(text(&dir, &legacy(save::LEGACY_BANLISTS[1])), ban_a());
        assert_eq!(text(&dir, "game/server/notes.txt"), "mine");
        assert!(!dir.join("game/client").exists() && !dir.join("common").exists());
        assert!(m.done.contains(&"kept the directory game/server: it holds files the game does not know".to_string()), "{:?}", m.done);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unconvertible_files_stay() {
        tt_assets::require_game_data!();
        let dir = temp("bad");
        put(&dir, save::LEGACY_CONFIG, b"moveMap.bind(keyboard, \"f9\", moveforward);\n");
        let no_assets = dir.join("no-assets");
        let m = migrate(&dir, &no_assets);
        assert_eq!(m.errors.len(), 1, "{:?}", m.errors);
        assert!(dir.join(save::LEGACY_CONFIG).exists() && !dir.join("binds.ron").exists());
        // The next start (with the defaults) imports it.
        assert!(migrate(&dir, &assets()).errors.is_empty());
        assert!(dir.join("binds.ron").exists() && !dir.join("game").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

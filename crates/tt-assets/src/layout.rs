//! `docs/layout.md` describes the directories and the naming rules.
//!
//! Data and code name files by their path relative to the asset directory,
//! exactly as on disk (`models/tanks/tank01.dts`, `missions/tt1_1.ron`);
//! [`crate::resolve`] turns such a reference into a file. Missions are
//! named by id (`tt1_1`) where a name is enough ([`mission`]).
//!
//! The game's paths (`game/data/missions/TT1_1.mis`) are gone;
//! [`migrate_mission`] reads the ones a save of an older build may hold.
//!
//! The asset directory is `$TT_ASSETS`, else the imported game data
//! (`gamedata/`, [`imported_candidates`]), else `./assets` when it holds
//! the game data, else the repository's `assets/` (compile-time path):
//! [`default_dir`]. The game imports into [`import_target`].

use std::path::{Path, PathBuf};

/// Directory of the missions (`<id>.ron`, `<id>.ter`, `<id>.png`).
pub const MISSIONS: &str = "missions";

/// The mission file of mission `id` (`tt1_1` -> `missions/tt1_1.ron`).
/// File names are lowercase (`docs/layout.md` § Naming), so is the id:
/// `TT1_1` names the same mission.
pub fn mission(id: &str) -> String {
    format!("{MISSIONS}/{}.ron", id.to_ascii_lowercase())
}

/// The id of a mission file (`missions/tt1_1.ron` -> `tt1_1`), lowercase
/// (game data imported by an older build, or a mod, may name the file
/// `TT1_1.ron`).
pub fn mission_id(file: &str) -> String {
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem).to_ascii_lowercase()
}

/// A mission named the way an older build saved it (the game's path,
/// `game/data/missions/TT1_1.mis`, or `~/data/missions/TT1_1.mis`) as a
/// mission file of this layout (`missions/tt1_1.ron`); `None` if `old` is
/// not such a path.
pub fn migrate_mission(old: &str) -> Option<String> {
    let p = old.trim().replace('\\', "/");
    let p = p.trim_start_matches("./").trim_start_matches('/');
    let lower = p.to_ascii_lowercase();
    let rest = ["game/data/missions/", "~/data/missions/"].iter().find_map(|pre| lower.strip_prefix(pre).map(|_| &p[pre.len()..]))?;
    let id = rest.strip_suffix(".mis").or_else(|| rest.strip_suffix(".MIS")).unwrap_or(rest);
    (!id.is_empty() && !id.contains('/')).then(|| mission(id))
}

/// (asset-relative path, file) of every file under the asset directory
/// `root`, sorted.
pub fn files(root: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let r = format!("{rel}{}", e.file_name().to_string_lossy());
            if p.is_dir() {
                walk(&p, &format!("{r}/"), out);
            } else {
                out.push((r, p));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "", &mut out);
    out.sort();
    out
}

/// Directory name of the imported game data (never in git).
pub const IMPORTED: &str = "gamedata";

/// The source checkout this was built in (the compile-time path), when it
/// still exists: a game run from its checkout.
pub fn source_checkout() -> Option<PathBuf> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    repo.join("crates").is_dir().then_some(repo)
}

/// The user data directory (settings, players, logs, mods, the game's own
/// import; `docs/layout.md` § User data), per platform:
///
/// | OS | directory |
/// |---|---|
/// | Linux (and other Unix) | `$XDG_DATA_HOME/<DATA_DIR>`, else `~/.local/share/<DATA_DIR>` |
/// | Windows | `%APPDATA%\<DATA_DIR>` |
/// | macOS | `~/Library/Application Support/<DATA_DIR>` |
///
/// `<DATA_DIR>` is [`crate::product::DATA_DIR`] ("opentt"). When the
/// platform's variables are unset: `./<DATA_DIR>-data`.
pub fn user_data_dir() -> PathBuf {
    user_data_base().map(|b| b.join(crate::product::DATA_DIR)).unwrap_or_else(|| PathBuf::from(format!("{}-data", crate::product::DATA_DIR)))
}

/// Where the user data was under the product's earlier name
/// ([`crate::product::OLD_DATA_DIR`] next to [`user_data_dir`]); `None`
/// without a platform data dir.
pub fn old_user_data_dir() -> Option<PathBuf> {
    user_data_base().map(|b| b.join(crate::product::OLD_DATA_DIR))
}

/// The platform data dir (see [`user_data_dir`]).
fn user_data_base() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))
    }
}

/// Directory name of the mods (`docs/modding.md`).
pub const MODS: &str = "mods";

/// The directories mods are looked for in, in this order (a mod id found
/// twice is taken from the first): `<data dir>/mods` (`data_dir`: the
/// engine's user data directory, [`user_data_dir`] by default), `mods/` of
/// the source checkout (development; the example mods), then the mods that
/// ship with an install ([`installed_mod_dirs`] of the executable's
/// directory). A directory may not exist; duplicates are dropped.
pub fn mod_dirs(data_dir: Option<&Path>) -> Vec<PathBuf> {
    mod_dirs_for(&data_dir.map_or_else(user_data_dir, Path::to_path_buf), source_checkout().as_deref(), exe_dir().as_deref())
}

/// [`mod_dirs`] for the user data directory `data_dir`, the source
/// checkout `checkout` and the executable's directory `bin_dir`.
pub fn mod_dirs_for(data_dir: &Path, checkout: Option<&Path>, bin_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut v = vec![data_dir.join(MODS)];
    if let Some(r) = checkout {
        v.push(r.join(MODS));
    }
    if let Some(e) = bin_dir {
        v.extend(installed_mod_dirs(e));
    }
    let mut seen = Vec::new();
    v.retain(|d| {
        let key = d.canonicalize().unwrap_or_else(|_| d.clone());
        let new = !seen.contains(&key);
        seen.push(key);
        new
    });
    v
}

/// Where an install keeps the mods that ship with it (the example mods),
/// for the executable's directory `bin_dir`:
///
/// | Install | Executable | Mods |
/// |---|---|---|
/// | `.tar.gz`, `.zip` (portable) | `<dir>/<game>` | `<dir>/mods` |
/// | `.deb`, AppImage | `<prefix>/bin/<game>` | `<prefix>/share/<DATA_DIR>/mods` |
/// | macOS `.app` | `X.app/Contents/MacOS/<game>` | `X.app/Contents/Resources/mods` |
///
/// plus `/usr/share/<DATA_DIR>/mods` on Linux and other Unix (a system
/// package whose binary is elsewhere). `<DATA_DIR>` is
/// [`crate::product::DATA_DIR`].
pub fn installed_mod_dirs(bin_dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![bin_dir.join(MODS)];
    if let Some(up) = bin_dir.parent() {
        v.push(up.join("share").join(crate::product::DATA_DIR).join(MODS));
        v.push(up.join("Resources").join(MODS));
    }
    if cfg!(unix) && !cfg!(target_os = "macos") {
        v.push(Path::new("/usr/share").join(crate::product::DATA_DIR).join(MODS));
    }
    v
}

/// The directory of the running executable.
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf))
}

/// Whether the executable's directory `dir` may hold the imported game
/// data: not inside a macOS application bundle (`X.app/Contents/MacOS`:
/// writing there breaks the bundle's signature) and writable ([`writable`];
/// not so under `Program Files` or `/usr` for a normal user).
fn portable_dir(dir: &Path) -> bool {
    let in_bundle = dir.components().any(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase().ends_with(".app"));
    !in_bundle && writable(dir)
}

/// Whether files can be created in `dir` (tried with a probe file).
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".tt-write-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// The places the imported game data ([`IMPORTED`]) may be, in the order
/// [`default_dir`] looks: the working directory, the source checkout, next
/// to the executable, the user data directory.
pub fn imported_candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from(IMPORTED)];
    if let Some(r) = source_checkout() {
        v.push(r.join(IMPORTED));
    }
    if let Some(e) = exe_dir() {
        v.push(e.join(IMPORTED));
    }
    v.push(user_data_dir().join(IMPORTED));
    v
}

/// Where the game's own import goes (the first-run screen, Settings →
/// Game data): the source checkout's `gamedata/` when the game runs from
/// its checkout; else `gamedata/` next to the executable when that
/// directory is writable and not inside a macOS `.app` bundle (a portable
/// install); else `gamedata/` in the user data directory (an install in a
/// read-only place: `Program Files`, `/Applications`, a system package).
/// [`default_dir`] finds each of them.
pub fn import_target() -> PathBuf {
    if let Some(r) = source_checkout() {
        return r.join(IMPORTED);
    }
    if let Some(e) = exe_dir().filter(|e| portable_dir(e)) {
        return e.join(IMPORTED);
    }
    user_data_dir().join(IMPORTED)
}

/// The asset directory: `$TT_ASSETS`, else the first of
/// [`imported_candidates`] (`tt-import --out gamedata`, or the game's own
/// import, [`import_target`]) that holds the game data
/// ([`crate::testdata::present_in`]), else `./assets` when it holds the
/// game data (`missions/`), else the repository's. The game shows its
/// first-run screen when the directory found has no game data.
pub fn default_dir() -> PathBuf {
    if let Some(v) = std::env::var_os("TT_ASSETS").filter(|v| !v.is_empty()) {
        return PathBuf::from(v);
    }
    if let Some(d) = imported_candidates().into_iter().find(|d| crate::testdata::present_in(d)) {
        return d;
    }
    let cwd = PathBuf::from("assets");
    if cwd.join(MISSIONS).is_dir() {
        return cwd;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missions_by_id() {
        assert_eq!(mission("tt1_1"), "missions/tt1_1.ron");
        assert_eq!(mission_id("missions/tt1_1.ron"), "tt1_1");
        assert_eq!(mission_id("tt2_3"), "tt2_3");
        assert_eq!(mission("TT1_1"), "missions/tt1_1.ron");
        assert_eq!(mission_id(&format!("missions/{}.ron", "TT2_3")), "tt2_3");
    }

    #[test]
    fn migrates_original_mission_paths() {
        assert_eq!(migrate_mission("game/data/missions/TT2_1.mis").as_deref(), Some("missions/tt2_1.ron"));
        assert_eq!(migrate_mission("./Game/Data/Missions/target.MIS").as_deref(), Some("missions/target.ron"));
        assert_eq!(migrate_mission("~/data/missions/TT1_5.mis").as_deref(), Some("missions/tt1_5.ron"));
        assert_eq!(migrate_mission("missions/tt1_1.ron"), None);
        assert_eq!(migrate_mission("game/server/banlist.txt"), None);
    }

    #[test]
    fn user_dirs() {
        // An app bundle's executable directory never holds the import.
        let base = std::env::temp_dir().join(format!("tt-assets-layout-bundle-{}", std::process::id()));
        let bundle = base.join("Game.app/Contents/MacOS");
        std::fs::create_dir_all(&bundle).unwrap();
        assert!(!portable_dir(&bundle));
        assert!(portable_dir(&base));
        std::fs::remove_dir_all(&base).unwrap();
        assert!(user_data_dir().ends_with(crate::product::DATA_DIR));
        let dirs = mod_dirs(Some(Path::new("/data")));
        assert_eq!(dirs[0], Path::new("/data/mods"));
    }

    #[test]
    fn lists_files_by_asset_path() {
        let base = std::env::temp_dir().join(format!("tt-assets-layout-{}", std::process::id()));
        std::fs::create_dir_all(base.join("missions")).unwrap();
        std::fs::create_dir_all(base.join("ui/icons")).unwrap();
        std::fs::write(base.join("missions/Foo.ron"), b"x").unwrap();
        std::fs::write(base.join("ui/icons/lives.png"), b"x").unwrap();
        let names: Vec<String> = files(&base).into_iter().map(|(g, _)| g).collect();
        assert_eq!(names, vec!["missions/Foo.ron", "ui/icons/lives.png"]);
        std::fs::remove_dir_all(&base).unwrap();
    }
}

//! Finding the Think Tanks install in a folder the player chose (the
//! game's first-run screen): the install itself, a folder above it or a
//! folder inside it, checked for every file the import reads.

use crate::import::is_install;
use crate::mapping::Mapping;
use std::path::{Path, PathBuf};

/// How many folders up from the chosen one an install is looked for (the
/// player picked `game/`, `game/data` or `game/data/missions`).
const UP: usize = 3;

/// The install root for the folder `chosen` (the root, the folder that
/// holds it, its `game/` folder or a folder below that, or a file in
/// it), complete with every file the import needs; otherwise a message
/// for the player saying what is missing.
pub fn resolve(chosen: &Path) -> Result<PathBuf, String> {
    let root = find_root(chosen)?;
    let missing = missing_files(&root, &Mapping::builtin());
    if missing.is_empty() {
        return Ok(root);
    }
    let shown: Vec<&str> = missing.iter().take(3).map(String::as_str).collect();
    let more = if missing.len() > shown.len() { format!(" and {} more", missing.len() - shown.len()) } else { String::new() };
    Err(format!(
        "The Think Tanks installation in {} is incomplete: {} of its files {} missing ({}{more}). Reinstall Think Tanks or choose another copy.",
        root.display(),
        missing.len(),
        if missing.len() == 1 { "is" } else { "are" },
        shown.join(", ")
    ))
}

/// The install root for `chosen`, not checked for completeness.
pub fn find_root(chosen: &Path) -> Result<PathBuf, String> {
    let text = chosen.as_os_str().to_string_lossy();
    if text.trim().is_empty() {
        return Err("Choose the folder where Think Tanks is installed.".into());
    }
    let chosen = expand_home(text.trim());
    if !chosen.exists() {
        return Err(format!("{}: there is no such folder.", chosen.display()));
    }
    // The installer itself is not the game: it has to be installed (or
    // unpacked) first.
    if chosen.is_file() && chosen.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) && !is_install(chosen.parent().unwrap_or(Path::new(""))) {
        return Err(format!("{} is a Windows program, not the installed game. Install Think Tanks first (on Windows, or with Wine), or unpack the installer (for example with innoextract), then choose the folder that holds the \"game\" folder.", chosen.display()));
    }
    let dir = if chosen.is_dir() { chosen.clone() } else { chosen.parent().map(Path::to_path_buf).unwrap_or_default() };
    // The folder itself or one above it.
    for d in dir.ancestors().take(UP + 1) {
        if is_install(d) {
            return Ok(d.to_path_buf());
        }
    }
    // A folder inside it (the folder that holds the install, such as the
    // games folder), one or two levels down.
    let mut level = vec![dir.clone()];
    for _ in 0..2 {
        let mut next = Vec::new();
        for d in &level {
            for sub in subdirs(d) {
                if is_install(&sub) {
                    return Ok(sub);
                }
                next.push(sub);
            }
        }
        level = next;
        if level.len() > 200 {
            break;
        }
    }
    Err(diagnose(&dir))
}

/// What is wrong with `dir` as an install root, for the player.
fn diagnose(dir: &Path) -> String {
    let at = dir.display();
    if tt_assets::find_ci(&dir.join("game")).is_none() {
        return format!("No Think Tanks installation in {at}: it has no \"game\" folder. Choose the folder the Windows version of Think Tanks was installed or unpacked to (it holds the \"game\" and \"common\" folders; the installer .exe itself won't do).");
    }
    let need = ["game/data/missions", "game/server/scripts"];
    let missing: Vec<&str> = need.into_iter().filter(|n| tt_assets::find_ci(&dir.join(n)).is_none()).collect();
    format!("No complete Think Tanks installation in {at}: {} {} missing.", missing.join(" and "), if missing.len() == 1 { "is" } else { "are" })
}

/// The sub-folders of `dir`, sorted.
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    v.sort();
    v
}

/// `~/x` as the home directory's `x`.
fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(h)) => PathBuf::from(h).join(rest),
        _ => PathBuf::from(p),
    }
}

/// The files the import reads (`mapping`) that `root` lacks, as paths
/// relative to it (case is ignored, as the game did).
pub fn missing_files(root: &Path, mapping: &Mapping) -> Vec<String> {
    let mut v: Vec<String> = mapping.files.iter().flat_map(|e| e.how.game_files()).filter(|f| tt_assets::find_ci(&root.join(f)).is_none()).collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake install under a fresh temporary directory: the two folders
    /// [`is_install`] looks for, and every file of the mapping when `full`.
    fn fake(name: &str, full: bool) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("tt-locate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("Games").join("Think Tanks");
        std::fs::create_dir_all(root.join("game/data/missions")).unwrap();
        std::fs::create_dir_all(root.join("game/server/scripts")).unwrap();
        std::fs::create_dir_all(root.join("common")).unwrap();
        if full {
            for f in Mapping::builtin().files.iter().flat_map(|e| e.how.game_files()) {
                let p = root.join(&f);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, b"").unwrap();
            }
        }
        (base, root)
    }

    #[test]
    fn finds_the_root_from_around_it() {
        let (base, root) = fake("around", false);
        assert_eq!(find_root(&root).unwrap(), root);
        // The folder that holds it, and the one above that.
        assert_eq!(find_root(&base.join("Games")).unwrap(), root);
        assert_eq!(find_root(&base).unwrap(), root);
        // Its game/ folder and folders inside it.
        assert_eq!(find_root(&root.join("game")).unwrap(), root);
        assert_eq!(find_root(&root.join("game/data/missions")).unwrap(), root);
        // A file in it.
        std::fs::write(root.join("ThinkTanks.bin"), b"").unwrap();
        assert_eq!(find_root(&root.join("ThinkTanks.bin")).unwrap(), root);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn says_what_is_missing() {
        let (base, root) = fake("missing", false);
        assert!(find_root(Path::new("")).unwrap_err().contains("Choose the folder"));
        assert!(find_root(&base.join("nowhere")).unwrap_err().contains("no such folder"));
        let empty = base.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(find_root(&empty).unwrap_err().contains("no \"game\" folder"), "{}", find_root(&empty).unwrap_err());
        let exe = empty.join("Think Tanks.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        assert!(find_root(&exe).unwrap_err().contains("not the installed game"), "{}", find_root(&exe).unwrap_err());
        // A game/ folder without the server scripts.
        let half = base.join("half");
        std::fs::create_dir_all(half.join("game/data/missions")).unwrap();
        let e = find_root(&half).unwrap_err();
        assert!(e.contains("game/server/scripts is missing"), "{e}");
        // The root exists but the files are not there.
        let e = resolve(&root).unwrap_err();
        assert!(e.contains("incomplete") && e.contains("more"), "{e}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn accepts_a_complete_install() {
        let (base, root) = fake("full", true);
        assert_eq!(resolve(&base.join("Games")).unwrap(), root);
        assert_eq!(resolve(&root.join("game")).unwrap(), root);
        // One file gone: named in the message.
        let f = Mapping::builtin().files[0].how.game_files()[0].clone();
        std::fs::remove_file(root.join(&f)).unwrap();
        let e = resolve(&root).unwrap_err();
        assert!(e.contains(&f) && e.contains("1 of its files is missing"), "{e}");
        let _ = std::fs::remove_dir_all(&base);
    }
}

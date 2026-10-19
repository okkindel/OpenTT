//! The mods of the running game (`docs/modding.md`, [`tt_assets::mods`]):
//! which are enabled (the setting `mods.enabled`, read before the asset
//! file index is built: [`enabled_ids`]), the Options → Mods screen's
//! commands, and the network rule: every player of a game needs the same
//! mods that change the game ([`check_server_mods`]); the cosmetic ones
//! may differ.

use crate::host::EngineHost;
use std::path::Path;
use tt_assets::mods::ModInfo;
use tt_net::ModRef;

/// The setting holding the enabled mods' folder names, in load order.
pub const ENABLED_KEY: &str = "mods.enabled";

/// The enabled mods of the settings file in the user data directory
/// `data` (none without one).
pub fn enabled_ids(data: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(data.join(tt_shell::save::SETTINGS)) else { return Vec::new() };
    tt_shell::Settings::from_ron(&text).ok().and_then(|s| s.mods.enabled).unwrap_or_default()
}

/// The installed mods (every folder of [`tt_assets::layout::mod_dirs`] for
/// the user data directory `data`).
pub fn installed_in(data: &Path) -> Vec<ModInfo> {
    tt_assets::mods::discover(&tt_assets::layout::mod_dirs(Some(data)))
}

/// The installed mods of this game's user data directory.
pub fn installed(h: &EngineHost) -> Vec<ModInfo> {
    installed_in(h.fs.data_dir())
}

/// The enabled mods for user data directory `data`, in load order, for
/// [`crate::fs::GameFs::with_mods`]; also the log lines about them.
pub fn load(data: &Path) -> (Vec<ModInfo>, Vec<String>) {
    let installed = installed_in(data);
    let ids = enabled_ids(data);
    let (on, missing) = tt_assets::mods::enabled(&installed, &ids);
    let mut log = Vec::new();
    for m in &on {
        log.push(format!("mod: {} ({}, {}, {} files)", m.label(), m.id, if m.affects_gameplay { "changes the game" } else { "cosmetic" }, m.files.len()));
        log.extend(m.warnings.iter().map(|w| format!("mod: {w}")));
    }
    for id in missing {
        log.push(format!("mod: {id} is enabled but not installed"));
    }
    (on.into_iter().cloned().collect(), log)
}

/// The enabled mods setting now (what the next start uses).
pub fn enabled_setting(h: &EngineHost) -> Vec<String> {
    let v = h.shell.prefs.get(ENABLED_KEY);
    v.split('\t').filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Saves the enabled mods (load order); they apply on the next start.
pub fn set_enabled(h: &mut EngineHost, ids: &[String]) {
    crate::shell::set_pref(h, ENABLED_KEY, &ids.join("\t"));
    crate::shell::save_prefs(h);
}

/// The running game's mods that change the game, as a server announces them.
pub fn gameplay_refs(h: &EngineHost) -> Vec<ModRef> {
    h.fs.mods().iter().filter(|m| m.affects_gameplay).map(|m| ModRef { name: m.name.clone(), version: m.version.clone(), hash: m.gameplay_hash }).collect()
}

/// A joining client's check of the server's gameplay mods (`server`)
/// against its own: `Err` with a message naming each mod to install,
/// enable, update or disable. Cosmetic mods are not compared.
pub fn check_server_mods(h: &EngineHost, server: &[ModRef]) -> Result<(), String> {
    let mine = gameplay_refs(h);
    let installed = installed(h);
    let mut problems = Vec::new();
    for s in server {
        if mine.iter().any(|m| m.hash == s.hash) {
            continue;
        }
        let same_name = |m: &&ModInfo| m.name.eq_ignore_ascii_case(&s.name) || m.id.eq_ignore_ascii_case(&s.name);
        if installed.iter().any(|m| m.gameplay_hash == s.hash) {
            problems.push(format!("enable the mod \"{}\"", s.label()));
        } else if let Some(m) = installed.iter().find(same_name) {
            problems.push(format!("your mod \"{}\" differs from the server's \"{}\"", m.label(), s.label()));
        } else {
            problems.push(format!("the mod \"{}\" is missing", s.label()));
        }
    }
    for m in &mine {
        if !server.iter().any(|s| s.hash == m.hash) {
            problems.push(format!("disable the mod \"{}\" (the server does not use it)", m.label()));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(format!("This server's game is modded differently: {}. Change your mods in Options > Mods and restart.", problems.join("; ")))
}

/// Opens the user's mods folder in the file manager (best effort; created
/// first).
pub fn open_folder(h: &mut EngineHost) {
    let dir = h.fs.data_dir().join(tt_assets::layout::MODS);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        h.log.warn(&format!("mods: {}: {e}", dir.display()));
        return;
    }
    match crate::gamedata::open_folder(&dir) {
        Ok(()) => h.log.info(&format!("mods: opened {}", dir.display())),
        Err(e) => h.log.warn(&format!("mods: {e}")),
    }
}

//! [`declare`] makes one declaration with `CREATE_OBJECT`'s semantics
//! ([`tt_assets::data::defs::DefSet`]: reuse of an existing
//! def, a copy of the parent's fields); a sound def also goes
//! to the sound database. [`load_server_set`] loads one of the server's
//! data sets (the server's start, and a remote client loading the
//! server's def manifest).

use crate::host::EngineHost;
use std::path::Path;
use tt_assets::data::defs::{self as db, Def, SpLevel, SpLevels};
use tt_assets::data::Object;
use tt_gameplay::data::WeaponEntry;

/// The server's data sets, in the order the server declares them (the
/// def manifest a remote client gets names them): `data/<set>.ron`.
pub const SERVER_SETS: &[&str] = db::SETS;

/// Declares one def, fields `(name + index, value)`.
pub fn declare(h: &mut EngineHost, class: &str, name: &str, parent: Option<&str>, def: bool, fields: &[(String, String)]) {
    if !h.defs.declare(class, name, parent, def, fields) {
        return;
    }
    // A sound def: the sound database follows (its merged fields).
    if crate::audio::AUDIO_CLASSES.iter().any(|c| c.eq_ignore_ascii_case(class)) {
        if let Some(o) = h.defs.get(name) {
            let (class, name, fields) = (o.class.clone(), o.name.clone(), o.fields.clone());
            h.audio.set_object(&class, &name, &fields);
        }
    }
}

/// Declares one def of a data file.
pub fn declare_def(h: &mut EngineHost, d: &Def) {
    let fields: Vec<(String, String)> = d.to_fields().into_iter().map(|f| (f.key(), f.value)).collect();
    declare(h, d.class(), d.name(), d.parent(), d.is_def(), &fields);
}

/// Logs the deprecation notes of the data files read since the last call
/// (names of earlier versions a mod's file still uses,
/// [`tt_assets::data::take_notes`]).
pub fn log_data_notes(h: &mut EngineHost) {
    for n in tt_assets::data::take_notes() {
        h.log.warn(&format!("data file: deprecated name, still read: {n}"));
    }
}

/// Declares every def of `data/<stem>.ron`, in order.
pub fn declare_file(h: &mut EngineHost, stem: &str) {
    let file = match db::load_file(h.fs.root(), stem) {
        Ok(f) => f,
        Err(e) => {
            h.log.error(&format!("data file: {e}"));
            return;
        }
    };
    log_data_notes(h);
    for d in &file {
        declare_def(h, d);
    }
}

/// Loads one of the [`SERVER_SETS`] (any case); false if `set` is none
/// of them.
pub fn load_server_set(h: &mut EngineHost, set: &str) -> bool {
    let set = set.to_ascii_lowercase();
    match SERVER_SETS.iter().find(|s| **s == set) {
        Some(s) => {
            declare_file(h, s);
            true
        }
        None => false,
    }
}

/// The power-up weapons of the data files.
pub fn load_weapons(h: &mut EngineHost) {
    let weapons = match db::load_weapons(h.fs.root()) {
        Ok(w) => {
            log_data_notes(h);
            w
        }
        Err(e) => {
            h.log.error(&format!("data file: {e}"));
            Vec::new()
        }
    };
    h.weapons = weapons.iter().map(|w| WeaponEntry { projectile: w.projectile.clone(), sound: w.sound.clone(), weight: w.weight.unwrap_or(1) }).collect();
}

/// The server's data: every one of the [`SERVER_SETS`], in order, and the
/// power-up weapons; the typed data rebuilt.
pub fn load_server_data(h: &mut EngineHost) {
    for s in SERVER_SETS {
        crate::netgame::manifest_entry(h, s);
        load_server_set(h, s);
    }
    load_weapons(h);
    refresh(h);
}

/// The typed data ([`EngineHost::data`]) after the defs changed;
/// references that name nothing are logged.
pub fn refresh(h: &mut EngineHost) {
    let data = tt_gameplay::data::GameData::build(h.defs.objects(), &h.weapons);
    for e in data.errors.iter().filter(|e| !h.data.errors.contains(e)) {
        h.log.error(&format!("data: {e}"));
    }
    h.data = data;
}

/// The solo level table for tt-shell (`SPLevel1..N`), from
/// `data/campaign.ron` under the asset directory `root`: each level's
/// fields after the copy of its parent's. Empty if the file cannot be
/// read.
pub fn splevels_table(root: &Path) -> tt_shell::LevelTable {
    let Ok(table) = tt_assets::data::load::<SpLevels>(&db::campaign_path(root)) else { return tt_shell::LevelTable { levels: Vec::new() } };
    fn fields(table: &SpLevels, l: &SpLevel, depth: usize) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(p) = l.parent.as_deref().filter(|_| depth < 32) {
            if let Some(pl) = table.levels.iter().find(|o| o.name.eq_ignore_ascii_case(p)) {
                out = fields(table, pl, depth + 1);
            }
        }
        for f in l.to_fields() {
            let v = f.value.clone();
            let name = f.key().to_ascii_lowercase();
            out.retain(|(k, _)| *k != name);
            out.push((name, v));
        }
        out
    }
    let mut levels = Vec::new();
    for n in 1u32.. {
        let name = format!("SPLevel{n}");
        let Some(l) = table.levels.iter().find(|o| o.name.eq_ignore_ascii_case(&name)) else { break };
        let f = tt_game::fields::Fields::new(&l.name, SpLevel::CLASS, fields(&table, l, 0));
        levels.push(tt_shell::solo::SpLevel::from_fields(n, &f));
    }
    tt_shell::LevelTable { levels }
}

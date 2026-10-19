//! Helpers shared by the integration tests (each test uses some of them).
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use tt_assets::data::defs::{load_file, Def, DefFile, DefSet, SETS};
use tt_game::data::TankParams;
use tt_game::fields::Defs;
use tt_game::mission::load_shape;
use tt_game::Move;

pub fn assets() -> PathBuf {
    tt_assets::layout::default_dir()
}

/// The game's defs (`data/*.ron`, in declaration order), a set's file
/// replaced where `replace` gives one, then the defs `own` declares.
pub fn defs_with(replace: impl Fn(&str) -> Option<DefFile>, own: &[Def]) -> Defs {
    let mut set = DefSet::new();
    for stem in SETS {
        let file = replace(stem).unwrap_or_else(|| load_file(&assets(), stem).unwrap());
        for d in file {
            set.declare_def(&d);
        }
    }
    for d in own {
        set.declare_def(d);
    }
    let mut db = Defs::new();
    db.add(set.objects().filter(|o| !o.name.is_empty()).cloned());
    db
}

/// The game's defs.
pub fn defs() -> Defs {
    defs_with(|_| None, &[])
}

/// A tank def with its shape (hit box) loaded.
pub fn tank(db: &Defs, name: &str) -> Arc<TankParams> {
    let mut td = db.tank(name).unwrap_or_else(|| panic!("{name}"));
    td.set_shape(&load_shape(&assets(), &assets(), &td.shape_file).unwrap());
    Arc::new(td)
}

/// The primary trigger held.
pub const FIRE: Move = Move { trigger: [true, false, false, false, false, false], ..Move::NULL };

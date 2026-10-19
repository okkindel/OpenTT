//! Gating of the tests that need the game data.
//!
//! The data under [`crate::layout::default_dir`] is built from a user's own
//! game install (`tt-import`); a checkout without it (a fresh clone of the
//! public repository) still builds and runs every test that does not need
//! it. Tests that read the game data start with
//! [`require_game_data!`](crate::require_game_data), which returns early
//! (the test passes, with a note on stderr) when the data is not there.
//!
//! `OPENTT_ASSETS` overrides the presence check:
//!
//! * unset or `auto`: run the data tests when the data is present;
//! * `require` (or `1`): the data must be present, a missing file fails
//!   the test (for runs that are meant to have the data);
//! * `skip` (or `0`): skip the data tests even when the data is present.

use std::path::Path;

/// Files that only a full asset directory has (built from the game files,
/// not part of the repository's own assets).
pub const MARKERS: [&str; 3] = ["missions/tt1_1.ron", "data/tanks.ron", "models/tanks/tank01.dts"];

/// The `OPENTT_ASSETS` setting (see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Require,
    Skip,
}

/// The mode `OPENTT_ASSETS` selects (unknown values are `Auto`).
pub fn mode() -> Mode {
    match std::env::var("OPENTT_ASSETS").unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "require" | "1" | "yes" | "true" => Mode::Require,
        "skip" | "0" | "no" | "false" => Mode::Skip,
        _ => Mode::Auto,
    }
}

/// Whether the asset directory `root` holds the game data ([`MARKERS`]).
pub fn present_in(root: &Path) -> bool {
    MARKERS.iter().all(|m| crate::find_ci(&root.join(m)).is_some())
}

/// Whether the game data is in [`crate::layout::default_dir`].
pub fn present() -> bool {
    present_in(&crate::layout::default_dir())
}

/// Whether a test named `test` that needs the game data should run: `true`
/// when the data is present (and not skipped by `OPENTT_ASSETS=skip`);
/// otherwise prints a note and returns `false`. Panics when
/// `OPENTT_ASSETS=require` and the data is missing.
pub fn require(test: &str) -> bool {
    let dir = crate::layout::default_dir();
    require_path(test, &dir, present_in(&dir))
}

/// [`require`] for a fixture other than the asset directory (recorded
/// traces, ...): whether the file or directory `path` exists, with the same
/// `OPENTT_ASSETS` handling.
pub fn require_fixture(test: &str, path: &Path) -> bool {
    require_path(test, path, path.exists())
}

fn require_path(test: &str, path: &Path, present: bool) -> bool {
    let mode = mode();
    if mode == Mode::Skip {
        eprintln!("{test}: skipped (OPENTT_ASSETS=skip)");
        return false;
    }
    if present {
        return true;
    }
    if mode == Mode::Require {
        panic!("{test}: the game data is missing from {} (OPENTT_ASSETS=require); build it with tt-import", path.display());
    }
    eprintln!("{test}: skipped, no game data in {} (build it with tt-import; OPENTT_ASSETS=require makes this an error)", path.display());
    false
}

/// Returns from the enclosing test (which then passes) when the game data
/// is not available: see [`testdata::require`](crate::testdata::require).
/// `require_game_data!(value)` returns `value` instead of `()`.
#[macro_export]
macro_rules! require_game_data {
    () => {
        if !$crate::testdata::require(module_path!()) {
            return;
        }
    };
    ($ret:expr) => {
        if !$crate::testdata::require(module_path!()) {
            return $ret;
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_need_every_file() {
        let dir = std::env::temp_dir().join(format!("tt-testdata-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("missions")).unwrap();
        std::fs::write(dir.join("missions/tt1_5.ron"), b"").unwrap();
        assert!(!present_in(&dir));
        std::fs::write(dir.join("missions/tt1_1.ron"), b"").unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/tanks.ron"), b"").unwrap();
        std::fs::create_dir_all(dir.join("models/tanks")).unwrap();
        assert!(!present_in(&dir));
        std::fs::write(dir.join("models/tanks/tank01.dts"), b"").unwrap();
        assert!(present_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

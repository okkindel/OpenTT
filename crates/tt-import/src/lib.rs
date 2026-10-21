//! The asset importer: builds the game's asset directory from the player's
//! ThinkTanks install plus OpenTT's own files (`docs/import.md`).
//!
//! * [`mapping`] – every asset file made from the game install (where from,
//!   how) and OpenTT's own files (`mapping.ron`), among them the game's
//!   data tables (`data/`), built into the program ([`OURS`]);
//! * [`import`] – the import itself ([`import::import`]);
//! * [`convert`] – missions to RON (with [`mis`], the mission file parser,
//!   and [`decl`]);
//! * [`images`] – images, masks, recolouring, the upscaler hook;
//! * [`verify`] – comparison of an import with the repository's `assets/`;
//! * [`locate`] – finding the install in a folder the player chose;
//! * [`sound`] – the built-in mixer for our sounds when ffmpeg is missing.
//!
//! The game's first-run screen runs [`install`] (all or nothing, with
//! [`Options::progress`] and [`Options::cancel`]) on a background thread;
//! the `tt-import` command runs [`import`].

pub mod convert;
pub mod decl;
pub mod images;
pub mod import;
pub mod locate;
pub mod mapping;
pub mod mis;
pub mod names;
pub mod sound;
pub mod verify;

pub use import::{import, install, InstallError, Options, Phase, PictureFn, PictureJob, Progress, ProgressFn, refresh_ours, Report, Skipped, CANCELLED, OURS_VERSION, PICTURE_SIZE};

include!(concat!(env!("OUT_DIR"), "/ours.rs"));

pub fn original_dir() -> Option<std::path::PathBuf> {
    let env = ["TT_ORIGINAL_DIR", "TT_ORIGINAL"].iter().find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()));
    let dir = match env {
        Some(v) => std::path::PathBuf::from(v),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original"),
    };
    import::is_install(&dir).then_some(dir)
}

/// The repository's `assets/` (the compile-time path).
pub fn repo_assets() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

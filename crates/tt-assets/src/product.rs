//! The product name shown to the player (window title, dialogs, version
//! line) and the name of its user data directory.
//!
//! Set at build time by `TT_PRODUCT_NAME` / `TT_PRODUCT_DATA_DIR` (for
//! example from the `[env]` table of `.cargo/config.toml`); "OpenTT" and
//! "opentt" when unset.

/// The product name: `TT_PRODUCT_NAME` at build time, else "OpenTT".
pub const NAME: &str = match option_env!("TT_PRODUCT_NAME") {
    Some(name) => name,
    None => "OpenTT",
};

/// The game window's title: the product name.
pub const WINDOW_TITLE: &str = NAME;

/// Directory name of the user data (settings, saves, imported game data)
/// under the platform data dir: `TT_PRODUCT_DATA_DIR` at build time, else
/// "opentt".
pub const DATA_DIR: &str = match option_env!("TT_PRODUCT_DATA_DIR") {
    Some(dir) => dir,
    None => "opentt",
};

/// The user data directory's earlier name (before OpenTT): renamed
/// to [`DATA_DIR`] once at startup (`tt_engine::userdata`).
pub const OLD_DATA_DIR: &str = "thinktanks-rs";

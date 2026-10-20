//! Native ThinkTanks shell (menu) logic, save compatible with the game's
//! client scripts:
//!
//! - [`solo`]: the campaign level table and sequence, run state and the
//!   post-level tally;
//! - [`catalog`]: tanks, mission names, the Quick Play pool;

pub mod banlist;
pub mod catalog;
pub mod hiscores;
pub mod legacy;
pub mod players;
pub mod netprefs;
pub mod prefs;
pub mod progress;
pub mod save;
pub mod settings;
pub mod solo;
pub mod vars;

pub use catalog::{MissionInfo, TankChoice, TANKS};
pub use hiscores::{HiScore, HiScores};
pub use players::{PlayerError, PlayerStats, Players};
pub use prefs::{Prefs, WindowMode};
pub use progress::Progress;
pub use save::{LoadReport, SaveStore, Saves};
pub use settings::Settings;
pub use solo::{campaign_levels, CampaignLevel, LevelTable, SoloRun, SpLevel, Tally, TallyRow};
pub use vars::{VarFile, VarValue};

/// The salt of every obfuscation key.
pub use tt_game::obfuscation::IDLE_TEXT;


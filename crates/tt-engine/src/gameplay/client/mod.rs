//! The client of the game, native.
//!
//! Its state is [`tt_gameplay::ClientState`] (`EngineHost::client`: the HUD
//! model, the player list, the chat input, the player's move) and
//! [`ClientGlue`] (`EngineHost::cl`: the connection flow and the client's
//! settings that were console globals).
//!
//! * [`recv`] – the server's messages to their handlers.
//! * [`connection`] – the connection and loading flow, the client's
//!   connection callbacks.
//! * [`hud`] – the in-game HUD's client logic: message lines, chat input,
//!   player list, `PlayGui`'s wake.

pub mod connection;
pub mod game;
pub mod hud;
pub mod recv;

use crate::canvas::Ctrl;
use tt_gameplay::client::value::Val;

/// The client's connection flow and settings (the client scripts' globals).
#[derive(Debug, Clone, Default)]
pub struct ClientGlue {
    /// The screen a disconnect returns to (None: the network browser,
    /// `joinGameGui`).
    pub disconnect_gui: Option<Ctrl>,
    /// As stored (unset: 1; the pause and target-practice dialogs set
    /// 0.00001).
    pub time_scale: Option<Val>,
    /// `synch_time`: the round's first clock sync (the end-of-round
    /// stats key).
    pub synch_time: Option<Val>,
    /// The password of the server being joined.
    pub password: String,
    /// The mission the client loaded.
    pub mission_file: String,
    /// `$MSeq`: the mission sequence of the download.
    pub mission_seq: String,
    /// The lighting phase runs.
    pub lighting_mission: bool,
    pub replica_total: u32,
    pub replicas_received: f32,
    pub camera_fov: Option<Val>,
    /// The client's HUD exists (`initClient` ran: `PlayGui`, the solo
    /// widgets; none on a dedicated server).
    pub hud: bool,
    /// The client's `MusicPlayer` exists.
    pub music_player: bool,
    /// The cursor package locks the mouse.
    pub cursor_controlled: bool,
    pub screenshot_number: f32,
    pub join_address: String,
    pub mission_arg: String,
}

impl ClientGlue {
    /// As text ("" unset).
    pub fn time_scale_text(&self) -> String {
        self.time_scale.as_ref().map(Val::string).unwrap_or_default()
    }

    /// `synch_time` as text ("" unset).
    pub fn synch_time_text(&self) -> String {
        self.synch_time.as_ref().map(Val::string).unwrap_or_default()
    }

    /// Read as a number.
    pub fn camera_fov_num(&self) -> f64 {
        self.camera_fov.as_ref().map(Val::float).unwrap_or(0.0)
    }
}

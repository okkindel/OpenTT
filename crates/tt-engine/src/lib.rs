//! The engine: the game (window, input, rendering, audio, network
//! play, menus) around the game's rules and client (`tt-gameplay`) and the
//! simulation (`tt-game`).
//!
//! Module boundaries:
//! * [`host`] – [`host::EngineHost`], the engine's state (files, log,
//!   video, canvas, audio, input, connections, server, client, shell);
//! * [`engine`] – the engine and the 32 ms sim tick; [`sched`] – the
//!   simulation clock and the game's timed events; [`boot`] – the boot
//!   sequence (command line, defaults, saves, client/server start);
//! * [`canvas`] – the screens and dialogs on the canvas, the cursor;
//!   [`video`] – the window's video mode;
//! * [`audio`] – **audio plug-in point**: the [`audio::Audio`] trait,
//!   implemented on the tt-audio mixer;
//! * [`input`] – action maps and dispatch;
//! * [`net`] – the local client/server connection, the typed messages and
//!   commands, the mission download phases; [`netgame`] – network play;
//! * [`game`] – the tt-game simulation and the game's entities (moves,
//!   events, chase camera);
//! * [`gameplay`] – the engine side of the game server, the client, the
//!   data declarations and the golden behaviour traces;
//! * [`fs`] – asset dir (read-only) + user data dir (writes) file system;
//! * [`crash`] – fatal errors made visible (crash log, message box,
//!   console attach on Windows);
//! * [`console`] – the dedicated server's typed console; [`mission`] –
//!   missions (RON) and the render scene; [`app`] – window and main loop;
//!   [`gfx`] – GPU helpers and offscreen screenshots; [`hud`] – the native
//!   HUD's model;
//! * [`gamedata`] – the game data's presence and its import (first-run
//!   screen, Settings → Game data); [`launcher`] – the window's two
//!   phases: that screen, then the game;
//! * [`menu`] – the menus (tt-menu); [`shell`] – their shell logic
//!   (tt-shell); [`menu_gpu`] – egui drawing and the tank preview texture.

pub mod allocstats;
pub mod app;
pub mod audio;
pub mod boot;
pub mod canvas;
pub mod console;
pub mod crash;
pub mod engine;
#[cfg(feature = "dev")]
pub mod framestats;
#[cfg(not(feature = "dev"))]
#[path = "framestats_off.rs"]
pub mod framestats;
pub mod frontend;
pub mod dynlights;
pub mod fps;
pub mod fs;
pub mod game;
pub mod gamedata;
pub mod gameplay;
pub mod gfx;
pub mod gui_gpu;
pub mod host;
pub mod hud;
pub mod icon;
pub mod input;
pub mod launcher;
pub mod log;
pub mod menu;
pub mod menu_gpu;
pub mod mission;
pub mod mission_picture;
pub mod mods;
pub mod net;
pub mod netgame;
pub mod sched;
pub mod shell;
pub mod treads;
pub mod userdata;
pub mod video;

pub use engine::{Config, Engine, TICK_MS};

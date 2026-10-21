//! `tt-gameplay`: the ThinkTanks game logic without engine or I/O: the game
//! server's rules ([`server`], behind the [`server::host::ServerHost`] the
//! engine implements), the game data they use ([`data`]), the typed messages
//! between the server and its clients ([`msg`]) and their wire codec ([`wire`]),
//! the world's game entities and events ([`entity`]), and the client's state
//! ([`client`]: the HUD model, the player list, the chat input, key bindings and
//! the player's move).

pub mod client;
pub mod data;
pub mod entity;
pub mod msg;
pub mod server;
pub mod wire;

pub use client::ClientState;
pub use entity::{Entity, EntityId, EntityKind, EntityTable, GameEvent};
pub use msg::{ClientCmd, Command, DebugCmd, HudMsg, MsgKind, Received, ServerMsg, Value};

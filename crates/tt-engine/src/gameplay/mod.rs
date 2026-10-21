//! The game's rules and the client, native:
//!
//! * the game server is `tt_gameplay::server`: [`server_host`] is its engine
//!   side; [`health_packs`] and [`scoreboard`] OpenTT's additions on the
//!   engine side;
//! * [`client`] – the client (connection flow, HUD, bindings, the server's
//!   messages);
//! * [`defs`] – the data declarations (`assets/data/`);
//! * [`golden`] – the golden behaviour traces.

pub mod client;
pub mod defs;
#[cfg(feature = "dev")]
pub mod golden;
#[cfg(not(feature = "dev"))]
#[path = "golden_off.rs"]
pub mod golden;
pub mod health_packs;
pub mod scoreboard;
pub mod server_host;

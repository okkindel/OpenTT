//! `tt-net`: the network transport.
//!
//! * [`Datagram`]: the socket trait, with [`UdpTransport`] (std, non-blocking)
//!   and [`MemNet`]/[`MemLink`] (in-memory, deterministic [`LinkConditions`]
//!   on a virtual clock).
//! * [`Server`] / [`Client`]: stateless-cookie handshake, version check,
//!   sessions, keepalives, timeouts, graceful disconnect, LAN info queries.
//! * Per connection: packet acks (seq + 32-bit ack field) driving RTT, loss,
//!   reliable resends and (later) snapshot baselines; three [`Lane`]s:
//!   reliable ordered (fragmented up to 32 KB), reliable unordered events
//!   (1 s lifetime) and unreliable.
//! * [`lan`]: LAN discovery ([`LanBrowser`], [`ServerInfo`]).
//! * [`input`]: move redundancy helper. [`bits`]: bit packing.
//!
//! Time is always passed in as a [`Duration`](std::time::Duration) since an
//! arbitrary epoch, so tests run on virtual time. The transport never
//! blocks or spawns threads (only [`portmap`]'s router mapping runs on a
//! thread of its own); the engine calls `update(now)` before its tick and
//! `flush(now)` after it.

pub mod auth;
pub mod bits;
pub mod client;
pub mod conn;
pub mod error;
pub mod input;
pub mod lan;
pub mod master;
pub mod msg;
pub mod packet;
pub mod portmap;
pub mod reliable;
pub mod rng;
pub mod server;
pub mod simlink;
pub mod stats;
pub mod transport;

pub use client::{Client, ClientConfig, ClientEvent, ClientState};
pub use conn::{ConnConfig, DisconnectReason, Lane};
pub use error::{DecodeError, SendError};
pub use lan::{LanBrowser, LanServer, ModRef, ServerInfo};
pub use master::{MasterPacket, MasterStatus};
pub use packet::MAX_PACKET_SIZE;
pub use server::{ConnId, EndpointStats, PendingId, Server, ServerConfig, ServerEvent};
pub use stats::ConnStats;
pub use simlink::{SimHandle, SimLink};
pub use transport::{Datagram, LinkConditions, LinkStats, MemLink, MemNet, RestrictedNat, UdpTransport};

/// Wire protocol version; a mismatch is rejected with `CR_WRONGVERSION`. 3: typed
/// gameplay messages (`Game`/`Cmd` instead of `TagDef`/`Command`). 4: multi-pellet shots
/// and the cartridge's own fire sound (`Fired` with a def). 5: power-up user animation
/// threads (`UserThread`: the Reload arch's lightning). 6: mission files
/// (`MissionPhase*`, `MissionInfo`, the LAN info's `mission_file`) are asset paths
/// (`missions/tt1_1.ron`), not the game's `game/data/missions/TT1_1.mis`. 7: the def
/// manifest names the server's data sets (`tanks`, `special`, ...) instead of script
/// files. 8: mods (docs/modding.md): the server's gameplay mods in the LAN info
/// (`ServerInfo` format 3) and in a `ModList` message before the def manifest. 9: the
/// def manifest names the data files by domain (`sounds`, `effects`, `weapons`,
/// `pickups`, `tanks`, ...). 10: asset file names are lowercase (docs/layout.md §
/// Naming): the mission files the messages carry are `missions/tt1_1.ron`, not
/// `missions/TT1_1.ron`. 11: OpenTT's own data vocabulary: the def fingerprint hashes the
/// defs under their new class and field names (`ProjectileDef`, `blast_radius`, ...),
/// the messages are `DefManifest`/`DefsDone`/`ReplicasDone`/ `ReplicaReset` (same tags),
/// mission objects are `Prop`, `Group`, ...
pub const PROTOCOL_VERSION: u16 = 11;
/// This build's game version ("0.1.2"), as servers announce it.
pub const GAME_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The game type a server announces (LAN answers, the master list): the
/// same for every build of this code base, whatever its product name, so
/// they list and join each other's games.
pub const GAME_TYPE: &str = "OpenTT";

/// The product name for the network's own texts (the router's port-mapping
/// description): `TT_PRODUCT_NAME` at build time, else "OpenTT" (the
/// rule of `tt_assets::product::NAME`).
pub const PRODUCT_NAME: &str = match option_env!("TT_PRODUCT_NAME") {
    Some(name) => name,
    None => "OpenTT",
};

/// Build id checked with the protocol (both sides must run the same sim):
/// from the crate version's `major.minor`, so the patch releases of one
/// minor version play together (a patch release changes no protocol,
/// simulation, data or map). The engine may pass a stronger one.
pub const BUILD_ID: u64 = {
    // FNV-1a of the version up to its second '.', at compile time.
    let s = env!("CARGO_PKG_VERSION").as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let (mut i, mut dots) = (0, 0);
    while i < s.len() {
        if s[i] == b'.' {
            dots += 1;
            if dots == 2 {
                break;
            }
        }
        h ^= s[i] as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    h
};
pub const DEFAULT_PORT: u16 = 28000;

#[cfg(test)]
mod tests {
    #[test]
    fn build_id_is_the_minor_version() {
        let v = env!("CARGO_PKG_VERSION");
        let minor = v.splitn(3, '.').take(2).collect::<Vec<_>>().join(".");
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in minor.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        assert_eq!(super::BUILD_ID, h, "{v} -> {minor}");
    }
}

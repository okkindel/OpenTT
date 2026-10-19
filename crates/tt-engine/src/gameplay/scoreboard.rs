//! Kills / deaths / assists for the player list and the round results.
//!
//! * Counted by the server (`tt_gameplay::server::combat`) from the world's
//!   damage and kill events ([`track`]); an assist scores a point in
//!   Deathmatch, a kill two.
//! * Always on (user decision D4: OpenTT's rules are the only ones).

use crate::host::EngineHost;
use tt_gameplay::server::combat::KdaEvent;

pub use tt_gameplay::server::Kda;

/// Server, every authority tick: the world's damage and kill events (in
/// entity terms); a local game's own list follows at once (a listen
/// server updates it with the list it sends, see netgame).
pub fn track(h: &mut EngineHost, events: &[KdaEvent], now: i64) {
    crate::gameplay::server_host::with_server(h, |s, b| s.count_kda(b, events, now));
    if h.net.client.is_some() && !h.netgame.serving() && h.srv.get().is_some_and(|s| s.kda.unsent) {
        let entries = take_unsent(h);
        apply(h, &entries, true);
    }
}

/// The K/D/A list by client id.
pub fn entries(h: &EngineHost) -> Vec<(u32, Kda)> {
    h.srv.get().map(|s| s.kda.entries().into_iter().map(|(c, k)| (c.0, k)).collect()).unwrap_or_default()
}

/// Changed since the lists were sent?
pub fn unsent(h: &EngineHost) -> bool {
    h.srv.get().is_some_and(|s| s.kda.unsent)
}

/// The list, marked sent.
pub fn take_unsent(h: &mut EngineHost) -> Vec<(u32, Kda)> {
    if let Some(s) = h.srv.server.as_mut() {
        s.kda.unsent = false;
    }
    entries(h)
}

/// Client (and the host's own list): the K/D/A of `entries` into the
/// client's player list (`EngineHost::client.players`); `full` = the whole
/// list (others are zero).
pub fn apply(h: &mut EngineHost, entries: &[(u32, Kda)], full: bool) {
    let e: Vec<(u32, tt_gameplay::client::Kda)> = entries.iter().map(|(id, k)| (*id, tt_gameplay::client::Kda { kills: k.kills, deaths: k.deaths, assists: k.assists })).collect();
    h.client.players.set_kda(&e, full);
}

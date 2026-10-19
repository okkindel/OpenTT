//! Client side of the server browser: what the menu uses ([`query_lan`],
//! [`query_master`], [`query_address`], [`rows`], [`status`]).
//!
//! A LAN query opens its own socket (a [`tt_net::LanBrowser`]; through the
//! netgame transport factory in tests), sends `InfoQuery` to the broadcast
//! address and to loopback on `port..port+10`, resends twice, and finishes
//! after [`LAN_QUERY_TIME`]. An internet query asks our master server for
//! its list.

use std::net::SocketAddr;
use std::time::Duration;

use super::{address_string, now, open, parse_address, NetGame};
use crate::host::EngineHost;
use tt_net::{Datagram, LanBrowser, LanServer};

/// How long a LAN query collects replies.
pub const LAN_QUERY_TIME: Duration = Duration::from_millis(1500);
/// Resends after the first query (lost broadcasts).
const RESENDS: [Duration; 2] = [Duration::from_millis(300), Duration::from_millis(700)];
/// How long an internet query collects replies (list, punches, info).
pub const MASTER_QUERY_TIME: Duration = Duration::from_millis(3000);
/// Info query resends of an internet query (the first one to a server
/// behind NAT is usually dropped before its punch arrives).
const MASTER_RESENDS: [Duration; 4] = [Duration::from_millis(400), Duration::from_millis(900), Duration::from_millis(1600), Duration::from_millis(2300)];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueryStatus {
    #[default]
    Idle,
    /// Collecting replies.
    Querying,
    /// The last query finished (or was cancelled).
    Done,
}

/// Browser state (in [`NetGame`]).
#[derive(Default)]
pub struct Query {
    browser: Option<LanBrowser<Box<dyn Datagram>>>,
    status: QueryStatus,
    started: Duration,
    resends: usize,
    port: u16,
    /// The query of the master's list (not the LAN broadcast).
    master: bool,
    /// What the Online menu lists (its LAN / Internet switch).
    pub source: tt_menu::ServerSource,
    /// The last status message ("Found 2 servers", ...).
    pub message: String,
}

/// One row of the server list, as the join screen shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerRow {
    /// Index for [`set_server_info`] / `setServerInfo`.
    pub index: usize,
    /// `IP:a.b.c.d:port` (what `connect` takes).
    pub address: String,
    pub name: String,
    /// The mission's display name.
    pub map: String,
    pub mission_file: String,
    /// "Battlemode", "Team Scrum", ...
    pub mode: String,
    pub players: u32,
    pub max_players: u32,
    pub bots: u32,
    pub ping_ms: u32,
    pub password: bool,
    pub dedicated: bool,
    /// Same protocol and build: joining can work.
    pub compatible: bool,
    /// The server runs mods that change the game; [`Self::mods`]: those it
    /// listed ("name version").
    pub modded: bool,
    pub mods: Vec<String>,
    pub platform: String,
    pub world: String,
    pub cpu_mhz: u32,
    /// (team letter, name) per player.
    pub player_list: Vec<(String, String)>,
    /// Score per player of [`Self::player_list`] (0 where the server sent none).
    pub player_scores: Vec<i32>,
    /// `getServerScore()`: > 0 is the green dot.
    pub score: f64,
}

/// `getServerScore()` of `joinGameGui.gui` for one server.
pub fn server_score(s: &LanServer) -> f64 {
    let mut score = s.info.cpu_mhz as f64 * 0.05 - s.ping.as_millis() as f64 + 65.0;
    if s.info.dedicated {
        score += if s.info.info.eq_ignore_ascii_case("Linux") { 50.0 } else { 25.0 };
    }
    score
}

fn row(index: usize, s: &LanServer) -> ServerRow {
    let i = &s.info;
    ServerRow {
        index,
        address: address_string(s.addr),
        name: i.name.clone(),
        map: i.map().to_string(),
        mission_file: i.mission_file.clone(),
        mode: i.mission_type.clone(),
        players: i.player_count as u32,
        max_players: i.max_players as u32,
        bots: i.bot_count as u32,
        ping_ms: s.ping.as_millis().min(u32::MAX as u128) as u32,
        password: i.password,
        dedicated: i.dedicated,
        compatible: s.compatible(),
        modded: i.modded || !i.mods.is_empty(),
        mods: i.mods.iter().map(|m| m.label()).collect(),
        platform: i.platform().to_string(),
        world: i.world().to_string(),
        cpu_mhz: i.cpu_mhz as u32,
        player_list: i.player_list(),
        player_scores: i.player_scores().into_iter().map(|p| p.2).collect(),
        score: server_score(s),
    }
}

fn browser(h: &mut EngineHost) -> Option<&mut LanBrowser<Box<dyn Datagram>>> {
    if h.netgame.query.browser.is_none() && !h.offline() {
        let ng: &mut NetGame = &mut h.netgame;
        let b = if ng.factory.is_some() {
            open(ng, 0).map(|t| LanBrowser::new(t, tt_net::rng::Rng::from_entropy().next_u64()))
        } else {
            tt_net::lan::udp_transport().map(|t| {
                let mut b = LanBrowser::new(Box::new(t) as Box<dyn Datagram>, tt_net::rng::Rng::from_entropy().next_u64());
                b.extra_broadcast.extend(tt_net::lan::guess_subnet_broadcast());
                b
            })
        };
        match b {
            Ok(b) => h.netgame.query.browser = Some(b),
            Err(e) => h.log.warn(&format!("server query: no socket: {e}")),
        }
    }
    h.netgame.query.browser.as_mut()
}

// ---- Rust API (the menu) ----------------------------------------------------------

/// Starts a LAN query for servers on `port..port+10` (0 = 28000); `clear`
/// empties the list first. Ends with `onServerQueryStatus("done")`.
pub fn query_lan(h: &mut EngineHost, port: u16, clear: bool) {
    let t = now(h);
    let port = if port == 0 { super::DEFAULT_PORT } else { port };
    h.netgame.query.message = "Querying LAN servers".into();
    let q = &mut h.netgame.query;
    q.status = QueryStatus::Querying;
    q.started = t;
    q.resends = 0;
    q.port = port;
    q.master = false;
    if let Some(b) = browser(h) {
        if clear {
            b.clear();
        }
        b.query_lan(port, t);
    }
}

/// Queries one address (`host[:port]` or `IP:a.b.c.d:port`): the "Join by
/// address" preview. The answer joins the list. False if it does not resolve.
pub fn query_address(h: &mut EngineHost, address: &str) -> bool {
    let Some(addr) = parse_address(address) else { return false };
    let t = now(h);
    match browser(h) {
        Some(b) => {
            b.query(&[addr], t);
            true
        }
        None => false,
    }
}

/// Starts an internet query: the master server's list, then every listed
/// server's info (with introductions for servers behind NAT). Ends with
/// `onServerQueryStatus("done")` after [`MASTER_QUERY_TIME`].
pub fn query_master(h: &mut EngineHost, master: SocketAddr, clear: bool) {
    let t = now(h);
    h.netgame.query.message = "Querying master server".into();
    let q = &mut h.netgame.query;
    q.status = QueryStatus::Querying;
    q.started = t;
    q.resends = 0;
    q.master = true;
    if let Some(b) = browser(h) {
        if clear {
            b.clear();
        }
        b.query_master(master, t);
    }
}

/// Empties the server list.
pub fn clear(h: &mut EngineHost) {
    if let Some(b) = h.netgame.query.browser.as_mut() {
        b.clear();
    }
}

/// `cancelServerQuery()`: stops collecting replies; the list stays.
pub fn cancel(h: &mut EngineHost) {
    if let Some(b) = h.netgame.query.browser.as_mut() {
        b.cancel();
    }
    if h.netgame.query.status == QueryStatus::Querying {
        h.netgame.query.status = QueryStatus::Done;
    }
}

pub fn status(h: &EngineHost) -> QueryStatus {
    h.netgame.query.status
}

/// The servers found so far.
pub fn servers(h: &EngineHost) -> &[LanServer] {
    h.netgame.query.browser.as_ref().map(|b| b.servers()).unwrap_or(&[])
}

/// The server list as join-screen rows.
pub fn rows(h: &EngineHost) -> Vec<ServerRow> {
    servers(h).iter().enumerate().map(|(i, s)| row(i, s)).collect()
}

/// Each tick (from `netgame::poll`): replies, resends, the end of the query.
pub fn poll(h: &mut EngineHost) {
    let t = now(h);
    let q = &mut h.netgame.query;
    if let Some(b) = q.browser.as_mut() {
        b.update(t);
        if q.status == QueryStatus::Querying {
            let age = t.saturating_sub(q.started);
            let sched: &[Duration] = if q.master { &MASTER_RESENDS } else { &RESENDS };
            if q.resends < sched.len() && age >= sched[q.resends] {
                q.resends += 1;
                if q.master {
                    b.resend_master(t);
                } else {
                    b.query_lan(q.port, t);
                }
            }
            if age >= if q.master { MASTER_QUERY_TIME } else { LAN_QUERY_TIME } {
                q.status = QueryStatus::Done;
                let n = b.servers().len();
                q.message = format!("Found {n} server{}", if n == 1 { "" } else { "s" });
            }
        }
    } else if q.status == QueryStatus::Querying {
        // No socket (or a deterministic run): nothing can answer.
        q.status = QueryStatus::Done;
        q.message = "No servers found".into();
    }
}


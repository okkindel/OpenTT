//! Network play over tt-net.
//!
//! * **Server** (dedicated or listen): `allowConnections(1)` opens a
//!   [`tt_net::Server`]. Each accepted peer is a connection id of its own
//!   (`crate::net::Conn`) and a client of the server
//!   (`gameplay::server_host`: connect request, connect, drop, the mission
//!   phases), like the local connection. The engine steps of the phases:
//!   `transmitDefs` sends the def manifest (the server's data
//!   files by domain and a hash of their defs), `startReplication`
//!   the mission name and hash; snapshots replicate the world
//!   (`replicate`).
//! * **Client**: `crate::net::connect_to` (the menu's Join, `-connect`)
//!   opens a [`tt_net::Client`] for the client's connection. The client
//!   (`gameplay::client::connection`) gets the connection and download
//!   callbacks as from the local connection.
//! * Messages are typed (protocol 3): the server's [`tt_gameplay::ServerMsg`]
//!   goes on the wire as it is ([`Msg::Game`]) and the client hands it to its
//!   handler (`gameplay::client::recv`); the client's
//!   [`tt_gameplay::ClientCmd`] ([`Msg::Cmd`]) goes to the server's
//!   `client_cmd`.
//!
//! Deterministic runs (the golden harness) never open sockets.

pub mod admin;
#[cfg(test)]
mod fuzz;
pub mod info;
pub mod predict;
pub mod proto;
pub mod query;
pub mod rep;
pub mod replicate;
pub mod sim;
pub mod snapshot;

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

use crate::host::EngineHost;
use crate::net::{ClientCb, Conn, Deferred};
use proto::{ConnectArgs, Msg};
use tt_gameplay::{ClientCmd, ServerMsg};
use tt_net::msg::{TextRx, TextTx};
use tt_net::{Client, ClientConfig, ClientEvent, ConnId, Datagram, DisconnectReason, Lane, Server, ServerConfig, ServerEvent, UdpTransport};

/// Opens a transport on a port (0 = ephemeral). Tests plug in `MemNet` links.
pub type TransportFactory = Box<dyn FnMut(u16) -> std::io::Result<Box<dyn Datagram>>>;

/// Transports on `net` at `ip` (port 0 = the next free one from 40000):
/// the in-memory network of the golden runs and the tests.
pub fn mem_factory(net: &tt_net::MemNet, ip: [u8; 4]) -> TransportFactory {
    let net = net.clone();
    let mut next = 40000u16;
    Box::new(move |port| {
        let port = if port == 0 {
            next += 1;
            next
        } else {
            port
        };
        Ok(Box::new(net.endpoint(std::net::SocketAddr::from((ip, port)))) as Box<dyn Datagram>)
    })
}

pub const DEFAULT_PORT: u16 = tt_net::DEFAULT_PORT;
/// Ports tried upwards from the requested one (`portInit`).
const PORT_TRIES: u16 = 10;
/// Connection timeout while loading (§5.3); 10 s once in the game.
const LOADING_TIMEOUT: Duration = Duration::from_secs(15);
const GAME_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest string argument from a client (§6).
const MAX_CLIENT_ARG: usize = 255;

/// May a remote client run `cmd`? The game's commands (§6: mission phase
/// acks, respawn, chat, admin login) always. Those commands are
/// `isRelease()`-gated in the scripts as well.
pub fn remote_command_allowed(cmd: &ClientCmd, test_cheats: bool, debug_build: bool) -> bool {
    !cmd.is_debug() || (debug_build && test_cheats)
}

/// No forged tags from clients; bounded length (§6).
fn sanitize(s: &mut String) {
    s.retain(|c| c != '\u{1}');
    if s.len() > MAX_CLIENT_ARG {
        let mut end = MAX_CLIENT_ARG;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
}

/// A remote client of this server (keyed by its connection id,
/// `crate::net::Conn`).
struct Remote {
    /// None while the connect request is decided, or once tt-net dropped it.
    conn: Option<ConnId>,
    addr: SocketAddr,
    /// Tagged texts this client has.
    texts: TextTx,
    /// tt-net ended the connection (timeout, the client's disconnect).
    dropped_by_net: bool,
    rep: replicate::ServerRep,
}

/// The client side: our connection to a remote server.
struct ClientSide {
    obj: Conn,
    server: SocketAddr,
    /// Tagged texts the server defined.
    texts: TextRx,
    rep: replicate::ClientRep,
}

#[derive(Default)]
pub struct NetGame {
    /// Transport factory; default: real UDP (`UdpTransport::bind_any`).
    pub factory: Option<TransportFactory>,
    /// Replaces the port the scripts ask for (`--port`; 0 = ephemeral).
    pub port_override: Option<u16>,
    /// `setNetPort`'s port; the socket is opened by `allowConnections(1)`.
    port: Option<u16>,
    server: Option<Server<Box<dyn Datagram>>>,
    accepting: bool,
    remotes: HashMap<Conn, Remote>,
    by_conn: HashMap<ConnId, Conn>,
    client: Option<Client<Box<dyn Datagram>>>,
    client_side: Option<ClientSide>,
    /// Def scripts loaded since the last `deleteDefs` (the
    /// manifest; lowercase game paths).
    manifest: Vec<String>,
    /// `--net-stats`: print connection stats this often.
    pub stats_every: Option<Duration>,
    stats_last: Duration,
    shared: replicate::ServerShared,
    /// `BanList` (loaded from `banlist.ron` when a server opens).
    pub bans: Vec<admin::Ban>,
    pub bans_loaded: bool,
    /// Last ping list sent (server).
    ping_last: Duration,
    kda_last: Duration,
    /// Lag icon shown (client).
    lagging: bool,
    /// Engine time of the last snapshot (client).
    last_snapshot: Duration,
    /// A server refused us for a missing or wrong password (`CHR_PASSWORD`):
    /// its address, for the menu's password prompt.
    pub password_request: Option<String>,
    /// The server browser (`queryLanServers`, `query::rows`).
    pub query: query::Query,
    /// `startHeartbeat()` was called (and no `stopHeartbeat()` since): list
    /// the open server on the master server.
    heartbeat: bool,
    /// Last logged master registrations (master, registered, public address).
    master_logged: Vec<(SocketAddr, bool, Option<SocketAddr>)>,
    /// The master list as resolved for the server's heartbeats ([`MasterDns`]).
    master_dns: MasterDns,
    /// The router port mapping of the open server.
    portmap: Option<tt_net::portmap::PortMapper>,
    portmap_logged: Option<tt_net::portmap::PortMapStatus>,
    /// (default on): rewind targets for remote players' direct-fire
    /// shots (§5.13). Read every tick.
    pub lag_compensation: bool,
    /// Replaces the build id in the handshake (tests of the wrong-version path).
    pub build_override: Option<u64>,
    /// `setSimulatedNetParams` state of the server and client sockets.
    pub sim: sim::Sims,
}

impl NetGame {
    pub fn client_rep_mut(&mut self) -> Option<&mut replicate::ClientRep> {
        self.client_side.as_mut().map(|c| &mut c.rep)
    }

    pub fn client_rep(&self) -> Option<&replicate::ClientRep> {
        self.client_side.as_ref().map(|c| &c.rep)
    }

    /// Our connection to a remote server.
    pub fn client_obj(&self) -> Option<Conn> {
        self.client_side.as_ref().map(|c| c.obj)
    }

    /// Remote clients known to the server.
    pub fn remotes_len(&self) -> usize {
        self.remotes.len()
    }

    /// Any remote client receiving snapshots.
    pub fn serving(&self) -> bool {
        self.remotes.values().any(|r| r.rep.in_game)
    }
}

/// The control object (our tank's entity) of a remote client.
pub fn control_object(h: &EngineHost) -> Option<tt_gameplay::EntityId> {
    h.netgame.client_rep().and_then(|r| r.control_obj)
}

/// A remote client's world was rebuilt: its puppets went with the old one
/// (they are created again as the snapshots bring them).
pub fn world_rebuilt(ng: &mut NetGame) {
    if let Some(r) = ng.client_rep_mut() {
        r.world_rebuilt();
    }
}

/// Server, before the world tick: every remote client's next move.
pub fn apply_remote_moves(ng: &mut NetGame, net: &crate::net::Net, g: &mut crate::game::GameSim) {
    // Lag compensation (§5.13): this tick's view delay of every remote shooter.
    g.world.lag_comp.view_delay.clear();
    for (obj, r) in &mut ng.remotes {
        let tank = net.control.get(obj).and_then(|o| g.tank_of(*o));
        replicate::apply_moves(&mut r.rep, tank, &mut g.world);
        if let (true, Some(t), Some(v)) = (ng.lag_compensation, tank, r.rep.view_delay) {
            g.world.lag_comp.view_delay.insert(t, v.min(tt_game::lagcomp::MAX_REWIND_TICKS));
        }
    }
}

/// Server: each remote client's estimated view delay V (ticks), for tests and stats.
pub fn view_delays(h: &EngineHost) -> Vec<(Conn, f32)> {
    let mut v: Vec<(Conn, f32)> = h.netgame.remotes.iter().filter_map(|(o, r)| r.rep.view_delay.map(|d| (*o, d))).collect();
    v.sort_by_key(|e| e.0);
    v
}

/// Server, after the world tick: what the clients must learn about it.
pub fn capture(ng: &mut NetGame, g: &crate::game::GameSim, events: &[tt_game::Event]) {
    if ng.serving() {
        replicate::capture(&mut ng.shared, &g.world, events);
    }
}

/// Client: this tick's move for the server (instead of the local world).
pub fn push_move(ng: &mut NetGame, mv: tt_game::Move) {
    if let Some(r) = ng.client_rep_mut() {
        // Clock sync: 1 ± 5 % moves per tick (§2.5).
        let n = r.pred.moves_this_tick(r.delay > 3.0);
        for _ in 0..n {
            let seq = r.input.push(snapshot::NetMove(mv));
            r.pred.sent(seq, &mv);
        }
    }
}

/// Prediction statistics of our connection.
pub fn predict_stats(h: &EngineHost) -> Option<predict::PredictStats> {
    h.netgame.client_rep().map(|r| r.pred.stats.clone())
}

/// Client, inside `game::tick`: puppets to the render time.
pub fn apply_states(ng: &mut NetGame, g: &mut crate::game::GameSim) {
    if let Some(r) = ng.client_rep_mut() {
        replicate::apply_states(r, g);
        replicate::predict(r, g);
    }
}

/// Collection sizes of every replication state (soak tests).
pub fn debug_sizes(h: &EngineHost) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    if let Some(r) = h.netgame.client_rep() {
        out.extend(r.debug_sizes().into_iter().map(|(k, v)| (format!("client {k}"), v)));
    }
    for r in h.netgame.remotes.values() {
        for (k, v) in r.rep.debug_sizes() {
            out.push((format!("server {k}"), v));
        }
    }
    out.push(("server known projectiles".into(), h.netgame.shared.known_count()));
    out
}

/// Per remote client: (connection object, stats, average snapshot bytes).
pub fn server_stats(h: &mut EngineHost) -> Vec<(Conn, tt_net::ConnStats, f64)> {
    let t = now(h);
    let ng = &mut h.netgame;
    let Some(s) = ng.server.as_mut() else { return Vec::new() };
    let mut out: Vec<_> = ng
        .remotes
        .iter()
        .filter_map(|(o, r)| {
            let st = s.stats(r.conn?, t)?;
            let avg = if r.rep.snapshots > 0 { r.rep.snapshot_bytes as f64 / r.rep.snapshots as f64 } else { 0.0 };
            Some((*o, st, avg))
        })
        .collect();
    out.sort_by_key(|e| e.0);
    out
}

fn now(h: &EngineHost) -> Duration {
    Duration::from_secs_f64(h.now_ms_f64().max(0.0) / 1000.0)
}

fn open(ng: &mut NetGame, port: u16) -> std::io::Result<Box<dyn Datagram>> {
    match ng.factory.as_mut() {
        Some(f) => f(port),
        None => UdpTransport::bind_any(port).map(|t| Box::new(t) as Box<dyn Datagram>),
    }
}


/// 64-bit FNV-1a of the mission's RON file (what builds its objects), the
/// terrain file and every prop shape of the loaded mission (the
/// collision world, §5.3).
pub fn mission_hash(h: &EngineHost, path: &str) -> u64 {
    let mut hash = tt_assets::FNV_START;
    tt_assets::fnv1a(&mut hash, &h.fs.read(path).unwrap_or_default());
    let root = h.fs.root();
    let dir = root.join(std::path::Path::new(path).parent().unwrap_or(std::path::Path::new("")));
    let mut files: Vec<String> = Vec::new();
    let last = |o: &tt_assets::mission::SceneObject, f: &str| o.fields.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(f)).map(|(_, v)| v.clone()).unwrap_or_default();
    for (_, o) in h.mission_load.as_ref().map(|l| l.nodes.as_slice()).unwrap_or(&[]) {
        if o.class.eq_ignore_ascii_case("Terrain") {
            files.push(last(o, "terrain_file"));
        } else if o.class.eq_ignore_ascii_case("Prop") {
            files.push(last(o, "shape"));
        }
    }
    files.sort();
    files.dedup();
    for f in files {
        tt_assets::fnv1a(&mut hash, f.to_ascii_lowercase().as_bytes());
        tt_assets::fnv1a(&mut hash, &tt_assets::find_ci(&tt_assets::resolve(root, &dir, &f)).and_then(|p| std::fs::read(p).ok()).unwrap_or_default());
    }
    hash
}

pub fn parse_address(s: &str) -> Option<SocketAddr> {
    parse_address_port(s, DEFAULT_PORT)
}

/// The master servers; empty when unset. Unresolvable entries and
/// repeats are left out.
pub fn master_addrs(prefs: &tt_shell::Prefs) -> Vec<SocketAddr> {
    parse_master_list(&prefs.get("net.master_server"))
}

/// [`master_addrs`] of a pref value.
pub fn parse_master_list(s: &str) -> Vec<SocketAddr> {
    let mut out: Vec<SocketAddr> = Vec::new();
    for a in s.split(',').filter(|e| !e.trim().is_empty()).filter_map(|e| parse_address_port(e, tt_net::master::DEFAULT_MASTER_PORT)) {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// [`parse_address`] with another default port.
pub fn parse_address_port(s: &str, default_port: u16) -> Option<SocketAddr> {
    let s = s.trim();
    let s = s.strip_prefix("IP:").or_else(|| s.strip_prefix("ip:")).unwrap_or(s);
    if let Ok(a) = s.parse::<SocketAddr>() {
        return Some(a);
    }
    let with_port = if s.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) { s.to_string() } else { format!("{s}:{default_port}") };
    let addrs: Vec<SocketAddr> = with_port.to_socket_addrs().ok()?.collect();
    addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()).copied()
}

/// The resolved master servers of a pref value, looked up on a thread (a
/// DNS lookup can take long; the server ticks must not wait for it) when
/// the value changes and every [`MASTER_DNS_EVERY`].
#[derive(Default)]
struct MasterDns {
    value: String,
    addrs: Vec<SocketAddr>,
    at: Duration,
    pending: Option<std::sync::mpsc::Receiver<Vec<SocketAddr>>>,
}

/// How often the master servers' names are looked up again.
const MASTER_DNS_EVERY: Duration = Duration::from_secs(300);

impl MasterDns {
    fn get(&mut self, value: &str, t: Duration) -> &[SocketAddr] {
        if let Some(addrs) = self.pending.as_ref().and_then(|r| r.try_recv().ok()) {
            self.addrs = addrs;
            self.pending = None;
        }
        // A changed value supersedes a pending lookup (its receiver is
        // dropped, so a late result of the old value is discarded).
        if value != self.value || (self.pending.is_none() && t.saturating_sub(self.at) >= MASTER_DNS_EVERY) {
            if value != self.value {
                self.addrs.clear();
            }
            self.value = value.to_string();
            self.at = t;
            let (tx, rx) = std::sync::mpsc::channel();
            let v = value.to_string();
            std::thread::spawn(move || tx.send(parse_master_list(&v)));
            self.pending = Some(rx);
        }
        &self.addrs
    }
}

fn address_string(a: SocketAddr) -> String {
    format!("IP:{}:{}", a.ip(), a.port())
}

// ---- the def manifest -------------------------------------------------------

/// A server data script is loaded (`crate::gameplay::defs`): into the
/// manifest, once.
pub fn manifest_entry(h: &mut EngineHost, entry: &str) {
    let lower = entry.trim_start_matches("./").to_ascii_lowercase();
    let ng = &mut h.netgame;
    if !ng.manifest.contains(&lower) {
        ng.manifest.push(lower);
    }
}

/// `deleteDefs`: the next server starts a new manifest.
pub fn defs_deleted(h: &mut EngineHost) {
    h.netgame.manifest.clear();
}

// ---- connections ------------------------------------------------------------------

/// `setNetPort(port)`: remembers the port (the socket opens with
/// `allowConnections(1)`, trying `PORT_TRIES` ports upwards).
pub fn set_net_port(h: &mut EngineHost, port: u16) {
    h.netgame.port = Some(port);
}

/// `allowConnections(on)`.
pub fn allow_connections(h: &mut EngineHost, on: bool) {
    h.netgame.accepting = on;
    if !on || h.netgame.server.is_some() {
        return;
    }
    admin::load(h);
    let want = h.netgame.port_override.or(h.netgame.port).unwrap_or(DEFAULT_PORT);
    let tries = if want == 0 { 1 } else { PORT_TRIES };
    let mut last_err = None;
    for i in 0..tries {
        match open(&mut h.netgame, want.wrapping_add(i)) {
            Ok(t) => {
                let addr = t.local_addr();
                let (t, sim) = tt_net::SimLink::new(t, tt_net::rng::Rng::from_entropy().next_u64());
                h.netgame.sim.server = Some(sim);
                let cfg = ServerConfig { build: h.netgame.build_override.unwrap_or(tt_net::BUILD_ID), ..ServerConfig::default() };
                h.netgame.server = Some(Server::new(Box::new(t) as Box<dyn Datagram>, cfg));
                h.log.info(&format!("Server listening on {addr}"));
                return;
            }
            Err(e) => last_err = Some(e),
        }
    }
    h.log.warn(&format!("allowConnections: could not open UDP port {want}: {}", last_err.map(|e| e.to_string()).unwrap_or_default()));
}

/// The server's bound address (tests, the Host screen).
pub fn server_addr(h: &EngineHost) -> Option<SocketAddr> {
    h.netgame.server.as_ref().map(|s| s.local_addr())
}

/// Is `conn` a remote client of this server?
pub fn is_remote(h: &EngineHost, conn: Conn) -> bool {
    h.netgame.remotes.contains_key(&conn)
}

/// Is `conn` our connection to a remote server?
pub fn is_remote_server(h: &EngineHost, conn: Conn) -> bool {
    h.netgame.client_side.as_ref().is_some_and(|c| c.obj == conn)
}

/// Connected (or connecting) to a remote server.
pub fn is_remote_client(h: &EngineHost) -> bool {
    h.netgame.client_side.is_some()
}

/// `getAddress()` of a remote connection.
pub fn address(h: &EngineHost, conn: Conn) -> Option<String> {
    if let Some(r) = h.netgame.remotes.get(&conn) {
        return Some(address_string(r.addr));
    }
    h.netgame.client_side.as_ref().filter(|c| c.obj == conn).map(|c| address_string(c.server))
}

/// `getPing()` / `getPacketLoss()` of a remote connection: (ms, percent).
pub fn ping_loss(h: &mut EngineHost, conn: Conn) -> Option<(f64, f64)> {
    let t = now(h);
    let ng = &mut h.netgame;
    let st = if let Some(id) = ng.remotes.get(&conn).and_then(|r| r.conn) {
        ng.server.as_mut()?.stats(id, t)?
    } else if ng.client_side.as_ref().is_some_and(|c| c.obj == conn) {
        ng.client.as_mut()?.stats(t)?
    } else {
        return None;
    };
    Some((st.rtt.as_secs_f64() * 1000.0, st.loss as f64 * 100.0))
}

/// `GameConnection::connect(address)` of the client's connection `obj`.
pub fn connect(h: &mut EngineHost, obj: Conn, address: &str, args: Vec<String>, password: &str) -> bool {
    let Some(addr) = parse_address(address) else {
        h.log.warn(&format!("connect: bad address {address:?}"));
        h.net.client_cb(Some(obj), ClientCb::ConnectRequestTimedOut);
        return false;
    };
    if let Some(old) = h.netgame.client_side.take() {
        let t = now(h);
        if let Some(c) = h.netgame.client.as_mut() {
            c.disconnect("reconnect", t);
        }
        h.netgame.client = None;
        h.net.forget(old.obj);
    }
    let transport = match open(&mut h.netgame, 0) {
        Ok(t) => t,
        Err(e) => {
            h.log.warn(&format!("connect: no socket: {e}"));
            h.net.client_cb(Some(obj), ClientCb::ConnectRequestTimedOut);
            return false;
        }
    };
    let args = ConnectArgs { args };
    let (transport, sim) = tt_net::SimLink::new(transport, tt_net::rng::Rng::from_entropy().next_u64());
    h.netgame.sim.client = Some(sim);
    let cfg = ClientConfig { build: h.netgame.build_override.unwrap_or(tt_net::BUILD_ID), ..ClientConfig::default() };
    let mut client = Client::new(Box::new(transport) as Box<dyn Datagram>, cfg);
    client.set_auth_secret(tt_net::auth::secret(password));
    // Through the master (hole punching) for internet addresses, when one is set.
    let masters = if tt_net::master::is_public(addr.ip()) { master_addrs(&h.shell.prefs) } else { Vec::new() };
    client.set_introducer(&masters);
    client.set_timeout(LOADING_TIMEOUT);
    let t = now(h);
    if client.connect(addr, &args.encode(), t).is_err() {
        h.net.client_cb(Some(obj), ClientCb::ConnectRequestRejected("CR_INVALID_CONNECT_PACKET".into()));
        return false;
    }
    h.log.info(&format!("Connecting to {addr}"));
    h.netgame.client = Some(client);
    h.netgame.client_side = Some(ClientSide { obj, server: addr, texts: TextRx::default(), rep: Default::default() });
    true
}

// ---- sending ---------------------------------------------------------------------

fn send_server(h: &mut EngineHost, conn: Conn, msgs: &[Msg]) {
    let t = now(h);
    let ng = &mut h.netgame;
    let (Some(r), Some(server)) = (ng.remotes.get_mut(&conn), ng.server.as_mut()) else { return };
    let Some(id) = r.conn else { return };
    for m in msgs {
        let b = m.encode_on(&mut r.texts);
        if server.send(id, Lane::Reliable, &b, t).is_err() {
            return;
        }
    }
}

fn send_client(h: &mut EngineHost, msgs: &[Msg]) {
    let t = now(h);
    let Some(c) = h.netgame.client.as_mut() else { return };
    for m in msgs {
        let b = m.encode();
        if c.send(Lane::Reliable, &b, t).is_err() {
            return;
        }
    }
}

/// A server message to a remote client.
pub fn send_game(h: &mut EngineHost, conn: Conn, msg: &ServerMsg) {
    *h.net.sent.entry(msg.lower().name.to_string()).or_default() += 1;
    send_server(h, conn, &[Msg::Game(msg.clone())]);
}

/// A client command to our remote server (nothing unless connected).
pub fn send_cmd(h: &mut EngineHost, cmd: ClientCmd) {
    if h.netgame.client.as_ref().is_none_or(|c| !c.is_connected()) || h.netgame.client_side.is_none() {
        return;
    }
    *h.net.sent.entry(cmd.lower().0.to_string()).or_default() += 1;
    send_client(h, &[Msg::Cmd(cmd)]);
}

/// `transmitDefs(seq)` for a remote client: the manifest.
pub fn transmit_defs(h: &mut EngineHost, conn: Conn, seq: &str) {
    let (hash, count) = h.data.fingerprint();
    let files = h.netgame.manifest.clone();
    // The mods first: a client without them is told which (the def
    // hash would only say that the data differs).
    let mods = crate::mods::gameplay_refs(h);
    send_server(h, conn, &[Msg::ModList { mods }, Msg::DefManifest { seq: seq.to_string(), files, hash, count }]);
}

/// `startReplication()` for a remote client: the mission to load.
pub fn start_replication(h: &mut EngineHost, conn: Conn, mission_file: &str) {
    let path = mission_file.to_string();
    let hash = mission_hash(h, &path);
    send_server(h, conn, &[Msg::MissionInfo { path, hash }]);
}

pub fn reset_replication(h: &mut EngineHost, conn: Conn) {
    send_server(h, conn, &[Msg::ReplicaReset]);
    if let Some(r) = h.netgame.remotes.get_mut(&conn) {
        r.rep = Default::default();
    }
}

pub fn play_sound(h: &mut EngineHost, conn: Conn, profile: &str, pos: Option<[f32; 3]>) {
    send_server(h, conn, &[Msg::Sound { profile: profile.to_string(), pos }]);
}

// ---- connection objects going away -----------------------------------------------

/// A remote client goes (dropped by tt-net, kicked or banned by the
/// server, `why` the reason the client is told, empty for the default):
/// the link is closed, the server drops its client.
pub fn remove_remote(h: &mut EngineHost, conn: Conn, why: &str) {
    if on_connection_removed(h, conn, why) {
        crate::gameplay::server_host::connection_removed(h, conn);
    }
}

/// The remote clients of this server (connection order).
pub fn remote_conns(h: &EngineHost) -> Vec<crate::net::Conn> {
    let mut v: Vec<crate::net::Conn> = h.netgame.remotes.keys().copied().collect();
    v.sort();
    v
}

/// A remote client of this server or our connection to a remote server
/// goes (`why`: see [`remove_remote`]). Returns false if `conn` is neither.
pub fn on_connection_removed(h: &mut EngineHost, conn: Conn, why: &str) -> bool {
    let t = now(h);
    if let Some(r) = h.netgame.remotes.get_mut(&conn) {
        let (id, dropped_by_net) = (r.conn.take(), std::mem::take(&mut r.dropped_by_net));
        let why = if why.is_empty() { "Server dropped the connection" } else { why };
        if let Some(id) = id {
            h.netgame.by_conn.remove(&id);
            if let Some(s) = h.netgame.server.as_mut() {
                s.disconnect(id, why, t);
                let _ = s.drain_events();
            }
        }
        // The record stays during the drop (address); sends are no-ops now.
        if id.is_some() || dropped_by_net {
            crate::gameplay::server_host::dropped(h, conn);
        }
        h.netgame.remotes.remove(&conn);
        h.net.forget(conn);
        return true;
    }
    if h.netgame.client_side.as_ref().is_some_and(|c| c.obj == conn) {
        h.netgame.client_side = None;
        if let Some(mut c) = h.netgame.client.take() {
            c.disconnect("disconnect", t);
        }
        h.net.forget(conn);
        return true;
    }
    false
}

/// Shuts every connection (engine exit).
pub fn shutdown(h: &mut EngineHost) {
    let t = now(h);
    if let Some(mut c) = h.netgame.client.take() {
        c.disconnect("quit", t);
    }
    if let Some(s) = h.netgame.server.as_mut() {
        let ids: Vec<ConnId> = s.connections().collect();
        for id in ids {
            s.disconnect(id, "Server shut down", t);
        }
        // Off the master servers' lists at once.
        s.set_master(&[], 0, t);
    }
    h.netgame.server = None;
    h.netgame.portmap = None;
}

// ---- the tick ----------------------------------------------------------------------

/// Receives packets and turns them into deferred calls (queued on `h.net`
/// like the local "packets", so they run in the same delivery round). Runs
/// at the start of every engine tick, before `net::deliver`.
pub fn poll(h: &mut EngineHost) {
    let t = now(h);
    h.netgame.sim.set_now(t);
    info::refresh(h);
    query::poll(h);
    let sev = match h.netgame.server.as_mut() {
        Some(s) => {
            s.update(t);
            s.drain_events()
        }
        None => Vec::new(),
    };
    for e in sev {
        server_event(h, e, t);
    }
    let cev = match h.netgame.client.as_mut() {
        Some(c) => {
            c.update(t);
            c.drain_events()
        }
        None => Vec::new(),
    };
    for e in cev {
        client_event(h, e);
    }
    ping_list(h, t);
    kda_list(h, t);
    lag_icon(h, t);
    master_tick(h, t);
    let lc = h.shell.prefs.get("server.lag_compensation");
    h.netgame.lag_compensation = lc.trim().is_empty() || tt_game::fmt::atoi(&lc) != 0;
    if let Some(every) = h.netgame.stats_every {
        if t.saturating_sub(h.netgame.stats_last) >= every {
            h.netgame.stats_last = t;
            for line in stats_lines(h) {
                h.log.info(&line);
            }
        }
    }
}

/// The remote clients with a tt-net connection, in connection order.
fn linked_remotes(ng: &NetGame) -> Vec<(Conn, ConnId)> {
    let mut ids: Vec<(Conn, ConnId)> = ng.remotes.iter().filter_map(|(o, r)| r.conn.map(|c| (*o, c))).collect();
    ids.sort();
    ids
}

/// `msg` (reliable) to those of `ids` that are in the game.
fn send_in_game(ng: &mut NetGame, ids: &[(Conn, ConnId)], msg: &[u8], t: Duration) {
    let Some(server) = ng.server.as_mut() else { return };
    for (o, c) in ids {
        if ng.remotes.get(o).is_some_and(|r| r.rep.in_game) {
            let _ = server.send(*c, Lane::Reliable, msg, t);
        }
    }
}

/// Server: every 2 s, the ping of every connection to every remote client
/// (and to the listen host's own player list).
fn ping_list(h: &mut EngineHost, t: Duration) {
    if h.netgame.server.is_none() || t.saturating_sub(h.netgame.ping_last) < Duration::from_secs(2) {
        return;
    }
    h.netgame.ping_last = t;
    // By the server's client ids (the ids of the player list rows).
    let client = |h: &EngineHost, o: Conn| h.srv.conn_client(o).map(|c| c.0);
    let mut entries: Vec<(u32, u32)> = h.net.server.and_then(|o| client(h, o)).map(|c| (c, 0)).into_iter().collect();
    let ids = linked_remotes(&h.netgame);
    let clients: Vec<Option<u32>> = ids.iter().map(|(o, _)| client(h, *o)).collect();
    let ng = &mut h.netgame;
    let Some(server) = ng.server.as_mut() else { return };
    for ((_, c), cl) in ids.iter().zip(&clients) {
        if let (Some(st), Some(cl)) = (server.stats(*c, t), cl) {
            entries.push((*cl, st.rtt.as_millis().min(9999) as u32));
        }
    }
    send_in_game(ng, &ids, &Msg::PingList { entries: entries.clone() }.encode(), t);
    if h.net.client.is_some() {
        admin::apply_pings(h, &entries);
    }
}

/// Server: the K/D/A list (`gameplay::scoreboard`) to every remote client
/// in game when it changed and every 2 s (late joiners), and to the
/// listen host's own player list.
fn kda_list(h: &mut EngineHost, t: Duration) {
    if h.netgame.server.is_none() {
        return;
    }
    let periodic = t.saturating_sub(h.netgame.kda_last) >= Duration::from_secs(2);
    if !crate::gameplay::scoreboard::unsent(h) && !periodic {
        return;
    }
    h.netgame.kda_last = t;
    let entries = crate::gameplay::scoreboard::take_unsent(h);
    let msg = Msg::KdaList { entries: entries.iter().map(|(o, k)| (*o, k.kills, k.deaths, k.assists)).collect() }.encode();
    let ids = linked_remotes(&h.netgame);
    send_in_game(&mut h.netgame, &ids, &msg, t);
    if h.net.client.is_some() {
        crate::gameplay::scoreboard::apply(h, &entries, true);
    }
}

/// `startHeartbeat()` / `stopHeartbeat()`.
pub fn set_heartbeat(h: &mut EngineHost, on: bool) {
    h.netgame.heartbeat = on;
}

/// The open server's registration with the master server.
pub fn master_status(h: &EngineHost) -> Option<tt_net::MasterStatus> {
    h.netgame.server.as_ref().map(|s| s.master_status().clone())
}

/// Server: heartbeats to the master server while the scripts want it and
/// connections are allowed. Both are logged when they change.
fn master_tick(h: &mut EngineHost, t: Duration) {
    let Some(port) = h.netgame.server.as_ref().map(|s| s.local_addr().port()) else {
        h.netgame.portmap = None;
        return;
    };
    let listed = h.netgame.heartbeat && h.netgame.accepting && !h.shell.prefs.get("server.display_on_master").eq_ignore_ascii_case("Never");
    let value = if listed { h.shell.prefs.get("net.master_server") } else { String::new() };
    let masters = h.netgame.master_dns.get(&value, t).to_vec();
    let id = info::server_id(h);
    let Some(s) = h.netgame.server.as_mut() else { return };
    s.set_master(&masters, id, t);
    let now: Vec<_> = s.master_statuses().filter_map(|st| Some((st.master?, st.registered, st.public_addr))).collect();
    if now != h.netgame.master_logged {
        let old = std::mem::replace(&mut h.netgame.master_logged, now.clone());
        let gone = old.iter().filter(|o| o.1 && !now.iter().any(|n| n.0 == o.0)).map(|o| (o.0, false, None));
        let changed: Vec<_> = now.iter().copied().filter(|n| old.iter().find(|o| o.0 == n.0).map_or((false, None), |o| (o.1, o.2)) != (n.1, n.2)).chain(gone).collect();
        for (m, registered, public) in changed {
            match (registered, public) {
                (true, Some(a)) => h.log.info(&format!("Master server {m}: listed; this server's internet address is {a}")),
                _ => h.log.info(&format!("Master server {m}: not listed")),
            }
        }
    }
    // The router port mapping (a background thread; the factory = tests: none).
    let want_map = h.netgame.accepting && h.netgame.factory.is_none() && tt_game::fmt::atoi(&h.shell.prefs.get("net.port_mapping")) != 0;
    if !want_map {
        h.netgame.portmap = None;
        h.netgame.portmap_logged = None;
        return;
    }
    if h.netgame.portmap.is_none() {
        h.netgame.portmap = Some(tt_net::portmap::PortMapper::start(port, Default::default()));
    }
    let st = h.netgame.portmap.as_ref().map(|p| p.status());
    if st != h.netgame.portmap_logged {
        match &st {
            Some(tt_net::portmap::PortMapStatus::Mapped { method, external_ip, external_port }) => {
                let ip = external_ip.map(|i| i.to_string()).unwrap_or_else(|| "?".into());
                h.log.info(&format!("Router port mapping ({method}): {ip}:{external_port} -> UDP {port}"))
            }
            Some(tt_net::portmap::PortMapStatus::Failed(e)) => h.log.warn(&format!("Router port mapping failed ({e}); forward UDP {port} by hand for internet players")),
            _ => {}
        }
        h.netgame.portmap_logged = st;
    }
}

fn lag_icon(h: &mut EngineHost, t: Duration) {
    let Some(obj) = h.netgame.client_obj() else {
        h.netgame.lagging = false;
        return;
    };
    let in_game = h.netgame.client_rep().is_some_and(|r| r.snapshots > 0);
    let threshold = tt_game::fmt::atof(&h.shell.prefs.get("net.lag_threshold"));
    let threshold = Duration::from_millis(if threshold > 0.0 { threshold as u64 } else { 400 });
    let lag = in_game && t.saturating_sub(h.netgame.last_snapshot) > threshold;
    if lag != h.netgame.lagging {
        h.netgame.lagging = lag;
        crate::gameplay::client::connection::set_lag_icon(h, obj, lag);
    }
}

/// Sends what this tick produced. Runs at the end of every engine tick.
pub fn flush(h: &mut EngineHost) {
    let t = now(h);
    h.netgame.sim.set_now(t);
    replicate_out(h, t);
    if let Some(s) = h.netgame.server.as_mut() {
        s.flush(t);
    }
    let ng = &mut h.netgame;
    if let (Some(c), Some(cs)) = (ng.client.as_mut(), ng.client_side.as_mut()) {
        if c.is_connected() {
            let seq = c.next_seq();
            if let Some(m) = replicate::input_message(&mut cs.rep, seq) {
                let _ = c.send(Lane::Unreliable, &m, t);
            }
            for s in c.take_acked() {
                cs.rep.input.packet_acked(s);
            }
        }
        c.flush(t);
    }
}

/// Server: snapshots, objects and events for every client in the game.
fn replicate_out(h: &mut EngineHost, t: Duration) {
    let ng = &mut h.netgame;
    let Some(server) = ng.server.as_mut() else { return };
    let Some(g) = h.game.as_ref() else {
        ng.shared.tick_reps.clear();
        return;
    };
    for (obj, r) in &mut ng.remotes {
        let Some(id) = r.conn.filter(|_| r.rep.in_game) else { continue };
        let control = h.net.control.get(obj).and_then(|o| g.tank_of(*o));
        let seq = server.next_seq(id).unwrap_or(0);
        let acked = server.take_acked(id);
        let (reliable, events, snap) = replicate::build(&mut r.rep, g, control, &ng.shared, seq, &acked);
        let tick = (g.world.time_ms / tt_game::TICK_MS) as u32;
        for rep in reliable {
            let _ = server.send(id, Lane::Reliable, &Msg::Rep { tick, rep }.encode(), t);
        }
        for (k, rep) in events {
            let lane = if rep.reliable() { Lane::Reliable } else { Lane::Event };
            let _ = server.send(id, lane, &Msg::Rep { tick: k, rep }.encode(), t);
        }
        let _ = server.send(id, Lane::Unreliable, &snap, t);
    }
    ng.shared.tick_reps.clear();
}

/// `--net-stats` lines: RTT, loss, bytes/s per connection.
pub fn stats_lines(h: &mut EngineHost) -> Vec<String> {
    let t = now(h);
    let mut out = Vec::new();
    let fmt = |who: String, s: tt_net::ConnStats| {
        format!("[net] {who}: rtt {:.0} ms ±{:.0}, loss {:.1} %, in {:.0} B/s, out {:.0} B/s, resends {}", s.rtt.as_secs_f64() * 1000.0, s.rtt_var.as_secs_f64() * 1000.0, s.loss * 100.0, s.bytes_in_per_sec, s.bytes_out_per_sec, s.resends)
    };
    let ng = &mut h.netgame;
    if let Some(s) = ng.server.as_mut() {
        let mut ids: Vec<ConnId> = s.connections().collect();
        ids.sort();
        for id in ids {
            let addr = s.addr(id).map(|a| a.to_string()).unwrap_or_default();
            if let Some(st) = s.stats(id, t) {
                let snap = ng.by_conn.get(&id).and_then(|o| ng.remotes.get(o)).filter(|r| r.rep.snapshots > 0).map(|r| r.rep.snapshot_bytes as f64 / r.rep.snapshots as f64).unwrap_or(0.0);
                out.push(format!("{}, snapshot avg {snap:.0} B", fmt(format!("client {} {addr}", id.0), st)));
            }
        }
    }
    if let (Some(c), Some(cs)) = (ng.client.as_mut(), ng.client_side.as_ref()) {
        if let Some(st) = c.stats(t) {
            out.push(fmt(format!("server {}", cs.server), st));
            let p = &cs.rep.pred.stats;
            let mut e = p.errors.clone();
            e.sort_by(f32::total_cmp);
            let p99 = e.get(((e.len() as f64 - 1.0).max(0.0) * 0.99) as usize).copied().unwrap_or(0.0);
            let depth = p.depth.iter().rev().take(150).map(|d| *d as f64).sum::<f64>() / p.depth.len().clamp(1, 150) as f64;
            out.push(format!(
                "[net] prediction: {} of {} reconciles mispredicted, corrections p99 {:.2} m, {} snaps; shots {} predicted, {} confirmed, {} faded; server queue {:.1}; delay {:.1} ticks",
                p.mispredicted, p.compared, p99, p.snaps, p.shots_predicted, p.shots_confirmed, p.shots_unconfirmed, depth, cs.rep.delay
            ));
        }
    }
    out
}

fn server_event(h: &mut EngineHost, e: ServerEvent, t: Duration) {
    match e {
        ServerEvent::ConnectRequest { pending, addr, payload, handshake } => connect_request(h, pending, addr, &payload, handshake, t),
        ServerEvent::Connected { .. } => {}
        ServerEvent::Disconnected { conn, reason, .. } => {
            let Some(obj) = h.netgame.by_conn.remove(&conn) else { return };
            h.log.info(&format!("Client {obj} dropped: {}", reason.text()));
            if let Some(r) = h.netgame.remotes.get_mut(&obj) {
                r.conn = None;
                r.dropped_by_net = true;
            }
            remove_remote(h, obj, "");
        }
        ServerEvent::Message { conn, data, lane } => {
            let Some(&obj) = h.netgame.by_conn.get(&conn) else { return };
            if lane == Lane::Unreliable {
                if data.first() == Some(&snapshot::MSG_INPUT) {
                    if let Some(r) = h.netgame.remotes.get_mut(&obj) {
                        r.rep.receive_input(&data[1..]);
                    }
                }
                return;
            }
            let Ok(msg) = Msg::decode(&data) else {
                h.log.warn(&format!("Client {obj}: bad message dropped"));
                return;
            };
            server_message(h, obj, conn, msg);
        }
    }
}

fn connect_request(h: &mut EngineHost, pending: tt_net::PendingId, addr: SocketAddr, payload: &[u8], handshake: tt_net::auth::Handshake, t: Duration) {
    let reject = |h: &mut EngineHost, reason: &str| {
        h.log.info(&format!("Connect request from {addr} rejected: {reason}"));
        if let Some(s) = h.netgame.server.as_mut() {
            s.reject(pending, reason, t);
        }
    };
    if !h.netgame.accepting {
        return reject(h, "CR_SERVERCLOSED");
    }
    let Ok(args) = ConnectArgs::decode(payload) else { return reject(h, "CR_INVALID_CONNECT_PACKET") };
    if admin::is_banned(h, &addr.ip().to_string()) {
        return reject(h, "CR_YOUAREBANNED");
    }
    let pw = h.shell.prefs.get("server.password");
    if !handshake.verify(&pw) {
        return reject(h, "CHR_PASSWORD");
    }
    let obj = h.net.new_remote();
    h.netgame.remotes.insert(obj, Remote { conn: None, addr, texts: TextTx::default(), dropped_by_net: false, rep: Default::default() });
    let reply = crate::gameplay::server_host::connect_request(h, &address_string(addr));
    if !reply.is_empty() {
        reject(h, &reply);
        h.netgame.remotes.remove(&obj);
        h.net.forget(obj);
        return;
    }
    let id = h.netgame.server.as_mut().and_then(|s| s.accept(pending, t));
    let Some(id) = id else {
        h.netgame.remotes.remove(&obj);
        h.net.forget(obj);
        return;
    };
    if let Some(s) = h.netgame.server.as_mut() {
        s.set_timeout(id, LOADING_TIMEOUT);
        let _ = s.drain_events(); // OpenTT's own Connected
    }
    if let Some(r) = h.netgame.remotes.get_mut(&obj) {
        r.conn = Some(id);
    }
    h.netgame.by_conn.insert(id, obj);
    h.log.info(&format!("Connection established: {obj} from {addr}"));
    crate::gameplay::server_host::connected(h, obj, &args.args);
}

fn server_message(h: &mut EngineHost, obj: Conn, conn: ConnId, msg: Msg) {
    match msg {
        Msg::Cmd(mut cmd) => {
            if !remote_command_allowed(&cmd, h.srv.get().is_some_and(|s| s.setup.test_cheats), cfg!(debug_assertions)) {
                h.log.warn(&format!("Client {obj}: command {} refused", cmd.lower().0));
                return;
            }
            for s in cmd.strings_mut() {
                sanitize(s);
            }
            h.net.defer(Some(obj), Deferred::ServerCmd(obj, cmd));
        }
        Msg::DefsDone { seq } => h.net.defer(Some(obj), Deferred::DefsDone(obj, seq)),
        Msg::ReplicasDone => {
            if let Some(s) = h.netgame.server.as_mut() {
                s.set_timeout(conn, GAME_TIMEOUT);
            }
            if let Some(r) = h.netgame.remotes.get_mut(&obj) {
                r.rep.in_game = true;
            }
            h.net.defer(Some(obj), Deferred::ReplicasDone(obj));
        }
        _ => h.log.warn(&format!("Client {obj}: unexpected message")),
    }
}

fn client_event(h: &mut EngineHost, e: ClientEvent) {
    let Some(obj) = h.netgame.client_side.as_ref().map(|c| c.obj) else { return };
    match e {
        ClientEvent::Connected => {
            h.log.info("Connection accepted");
            h.net.client_cb(Some(obj), ClientCb::ConnectionAccepted);
        }
        ClientEvent::Disconnected(reason) => {
            let cb = match &reason {
                DisconnectReason::Rejected(r) if r == tt_net::server::CR_WRONGVERSION => Some(ClientCb::ConnectRequestRejected("CR_INVALID_PROTOCOL_VERSION".into())),
                DisconnectReason::Rejected(r) => {
                    if r == "CHR_PASSWORD" {
                        h.netgame.password_request = h.netgame.client_side.as_ref().map(|c| address_string(c.server));
                    }
                    Some(ClientCb::ConnectRequestRejected(r.clone()))
                }
                DisconnectReason::ConnectTimeout => Some(ClientCb::ConnectRequestTimedOut),
                DisconnectReason::Timeout => Some(ClientCb::ConnectionTimedOut),
                DisconnectReason::Remote(r) => Some(ClientCb::ConnectionDropped(r.clone())),
                DisconnectReason::Local(_) => None,
                r => Some(ClientCb::ConnectionError(r.text().to_string())),
            };
            h.log.info(&format!("Connection ended: {}", reason.text()));
            h.netgame.client = None;
            h.netgame.client_side = None;
            h.net.forget(obj);
            if let Some(cb) = cb {
                h.net.client_cb(Some(obj), cb);
            }
            h.net.defer(Some(obj), Deferred::DeleteClientConn(obj));
        }
        ClientEvent::Message { data, lane: Lane::Unreliable } => {
            if data.first() == Some(&snapshot::MSG_SNAPSHOT) {
                if let Some(r) = h.netgame.client_rep_mut() {
                    r.receive_snapshot(&data);
                    h.netgame.last_snapshot = now(h);
                }
            }
        }
        ClientEvent::Message { data, .. } => match h.netgame.client_side.as_mut().map(|c| Msg::decode_on(&data, &mut c.texts)).unwrap_or(Err(tt_net::DecodeError::Truncated)) {
            Ok(Msg::Rep { tick, rep }) => {
                if let Some(r) = h.netgame.client_rep_mut() {
                    r.push(tick, rep);
                }
            }
            Ok(msg) => h.net.defer(Some(obj), Deferred::NetMsg(obj, msg)),
            Err(_) => h.log.warn("Server sent a bad message"),
        },
        ClientEvent::Info { .. } => {}
    }
}

/// Drops the connection from the client side with an error for the scripts.
fn client_fail(h: &mut EngineHost, obj: Conn, why: &str) {
    h.log.warn(why);
    let t = now(h);
    if let Some(mut c) = h.netgame.client.take() {
        c.disconnect(why, t);
        c.flush(t);
    }
    h.netgame.client_side = None;
    h.net.forget(obj);
    h.net.client_cb(Some(obj), ClientCb::ConnectionDropped(why.to_string()));
    h.net.defer(Some(obj), Deferred::DeleteClientConn(obj));
}

/// Applies a server message on the client, in the delivery round.
pub fn apply_client_msg(h: &mut EngineHost, obj: Conn, msg: Msg) {
    if !is_remote_server(h, obj) {
        return;
    }
    match msg {
        Msg::Game(m) => {
            if h.golden.is_some() {
                crate::gameplay::golden::record::on_received(h, &m);
            }
            crate::gameplay::client::recv::apply(h, &m);
        }
        Msg::DefManifest { seq, files, hash, count } => {
            // A later mission on the same connection sends the manifest
            // again: the sets already loaded are not loaded twice, which
            // would duplicate unnamed defs and fail the hash.
            for f in &files {
                if !h.netgame.manifest.iter().any(|e| e.eq_ignore_ascii_case(f)) {
                    manifest_entry(h, f);
                    // Or a mission with defs of its own (tt1_5's LeapBoost).
                    if !crate::gameplay::defs::load_server_set(h, f) {
                        crate::mission::declare_mission_defs(h, f);
                    }
                }
            }
            crate::gameplay::defs::refresh(h);
            let (mine, n) = h.data.fingerprint();
            if mine != hash {
                return client_fail(h, obj, &format!("Game data differs from the server's ({n} defs here, {count} there)"));
            }
            let n = n.max(1);
            for i in 1..=n {
                h.net.client_cb(None, ClientCb::DefReceived(i, n));
            }
            send_client(h, &[Msg::DefsDone { seq }]);
        }
        Msg::ModList { mods } => {
            if let Err(e) = crate::mods::check_server_mods(h, &mods) {
                client_fail(h, obj, &e);
            }
        }
        Msg::MissionInfo { path, hash } => {
            crate::mission::delete_mission(h);
            if !crate::mission::mission_exists(h, &path) {
                let id = tt_assets::layout::mission_id(&path);
                return client_fail(h, obj, &format!("You don't have the map \"{id}\" ({path}) the server plays: install the mod that has it, or copy the map into your mods folder"));
            }
            h.cl.mission_file = path.clone();
            crate::mission::exec_mission(h, &path);
            if mission_hash(h, &path) != hash || h.mission_load.is_none() {
                return client_fail(h, obj, "You don't have this mission (or it differs)");
            }
            let n = crate::net::replica_count(h);
            h.net.client_cb(None, ClientCb::ReplicasStarted(n as u32));
            for _ in 0..n {
                h.net.client_cb(None, ClientCb::ReplicaReceived);
            }
            send_client(h, &[Msg::ReplicasDone]);
            if let Some(c) = h.netgame.client.as_mut() {
                c.set_timeout(GAME_TIMEOUT);
            }
        }
        Msg::ReplicaReset => {
            replicate::reset_client(h);
            crate::mission::delete_mission(h);
        }
        Msg::Sound { profile, pos } => {
            if h.audio.profile_name(&profile).is_some() {
                h.audio.play_profile(&profile, pos);
            }
        }
        Msg::PingList { entries } => {
            if h.golden.is_some() {
                let e: Vec<(u32, String)> = entries.iter().map(|&(id, ms)| (id, ms.to_string())).collect();
                crate::gameplay::golden::record::on_received_list(h, "Pings", &e);
            }
            admin::apply_pings(h, &entries)
        }
        Msg::KdaList { entries } => {
            if h.golden.is_some() {
                let e: Vec<(u32, String)> = entries.iter().map(|&(id, k, d, a)| (id, format!("{k}/{d}/{a}"))).collect();
                crate::gameplay::golden::record::on_received_list(h, "Kda", &e);
            }
            let e: Vec<(u32, crate::gameplay::scoreboard::Kda)> = entries.iter().map(|&(id, kills, deaths, assists)| (id, crate::gameplay::scoreboard::Kda { kills, deaths, assists })).collect();
            crate::gameplay::scoreboard::apply(h, &e, true);
        }
        Msg::DefsDone { .. } | Msg::ReplicasDone | Msg::Rep { .. } | Msg::Cmd(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse_address("127.0.0.1:28001"), Some("127.0.0.1:28001".parse().unwrap()));
        assert_eq!(parse_address("IP:10.0.0.2:5000"), Some("10.0.0.2:5000".parse().unwrap()));
        assert_eq!(parse_address("127.0.0.1"), Some("127.0.0.1:28000".parse().unwrap()));
        assert_eq!(parse_address("[::1]:7"), Some("[::1]:7".parse().unwrap()));
        assert_eq!(address_string("10.0.0.2:5000".parse().unwrap()), "IP:10.0.0.2:5000");
        // The master list: comma-separated, port 28100 by default, spaces,
        // empty entries and repeats dropped, `IP:` and host names.
        let l = |s: &str| parse_master_list(s).iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(l("10.0.0.50"), ["10.0.0.50:28100"]);
        assert_eq!(l(" 10.0.0.50 , IP:10.0.0.51:9,, localhost:28101 ,"), ["10.0.0.50:28100", "10.0.0.51:9", "127.0.0.1:28101"]);
        assert_eq!(l("10.0.0.50,10.0.0.50:28100, ip:10.0.0.50"), ["10.0.0.50:28100"]);
        assert!(l("").is_empty() && l(" , ,").is_empty());
        assert_eq!(parse_address_port("10.0.0.50", 28100), Some("10.0.0.50:28100".parse().unwrap()));
        assert_eq!(parse_address_port("10.0.0.50:9", 28100), Some("10.0.0.50:9".parse().unwrap()));
    }

    #[test]
    fn command_whitelist() {
        use tt_gameplay::DebugCmd;
        for c in [ClientCmd::SpawnPlayer, ClientCmd::MessageSent { text: "hi".into() }, ClientCmd::PhaseAck { phase: 2, seq: "1".into() }, ClientCmd::MapList] {
            assert!(remote_command_allowed(&c, false, false), "{c:?}");
        }
        for d in [DebugCmd::ToggleCamera, DebugCmd::CycleTank, DebugCmd::TogglePlayerAI, DebugCmd::SelectObject, DebugCmd::DropPlayerAtCamera] {
            let c = ClientCmd::Debug { cmd: d, args: vec![] };
            assert!(!remote_command_allowed(&c, true, false), "{c:?}");
            assert!(!remote_command_allowed(&c, false, true), "{c:?}");
            assert!(remote_command_allowed(&c, true, true), "{c:?}");
        }
        // Client strings cannot carry tags and are bounded.
        let mut s = format!("\u{1}5 x{}", "y".repeat(300));
        sanitize(&mut s);
        assert_eq!(s.len(), MAX_CLIENT_ARG);
        assert!(s.starts_with("5 xy"));
    }

    #[test]
    fn messages_cross_the_wire_typed() {
        let m = ServerMsg::Message {
            kind: tt_gameplay::MsgKind::ClientJoin { name: tt_gameplay::Value::tag("Bait"), id: "2000".into(), team: String::new(), guid: String::new(), score: String::new(), cum: String::new(), ai: String::new(), admin: String::new(), super_admin: String::new() },
            text: tt_gameplay::Value::tag("\u{2}%1 joined the game."),
        };
        // Over the wire twice: the second time the texts go by number.
        let mut tx = TextTx::default();
        let mut rx = TextRx::default();
        let first = Msg::Game(m.clone()).encode_on(&mut tx);
        let second = Msg::Game(m.clone()).encode_on(&mut tx);
        assert!(second.len() + 20 < first.len(), "{} {}", first.len(), second.len());
        for b in [first, second] {
            assert_eq!(Msg::decode_on(&b, &mut rx).unwrap(), Msg::Game(m.clone()));
        }
        // What the client's handler gets: the template expanded.
        let r = m.received();
        assert_eq!(r[1].text, "\u{2}Bait joined the game.");
        assert_eq!(r[3].text, "2000");
    }
}

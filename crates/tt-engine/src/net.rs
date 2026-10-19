//! In-process "network": the local client/server connection of a single-player
//! or listen-server game, the typed messages between them ([`ServerMsg`] to
//! the client, [`ClientCmd`] to the server: [`send_to_client`], [`send_cmd`]),
//! the mission download phases (defs, always-replicated objects, control
//! object) and a queue of deferred calls standing in for network packets.
//!
//! A connection is a number ([`Conn`]): the client's end (`ServerConnection`,
//! to the local server or a remote one) and the local server's end of the
//! local connection (`LocalClientConnection`) count from 1, a remote client
//! of this server from [`REMOTE_BASE`]. Replication is a direct mapping: the
//! client sees the server's world itself.
//!
//! Messages, commands and the client's connection callbacks are delivered
//! on the next sim tick (like packets), in order.

use crate::host::EngineHost;
use std::collections::{HashMap, HashSet, VecDeque};
use tt_gameplay::{ClientCmd, ServerMsg};

/// A connection (see the module docs).
pub type Conn = u32;

/// The first remote connection id.
pub const REMOTE_BASE: Conn = 0x4000_0000;

/// A callback of the client's connection.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientCb {
    ConnectionAccepted,
    InitialControlSet,
    ConnectionDropped(String),
    ConnectionTimedOut,
    ConnectionError(String),
    ConnectRequestRejected(String),
    ConnectRequestTimedOut,
    /// Def `i` of `n` arrived.
    DefReceived(u32, u32),
    /// The mission's objects start (their number).
    ReplicasStarted(u32),
    ReplicaReceived,
    /// The mission lighting is done (`lightScene`'s callback).
    SceneLightingComplete,
}

/// A call made later, when the "packet" arrives.
#[derive(Debug, Clone, PartialEq)]
pub enum Deferred {
    /// A callback of the client's connection.
    Client(ClientCb),
    /// Deletes the client's connection (a dropped network connection,
    /// after its callback).
    DeleteClientConn(Conn),
    /// A message from our remote server (`crate::netgame`).
    NetMsg(Conn, crate::netgame::proto::Msg),
    /// A server message for the local client.
    ServerMsg(ServerMsg),
    /// A client's command for the server (the client's connection on the server).
    ServerCmd(Conn, ClientCmd),
    /// The client of a server connection has the defs (sequence).
    DefsDone(Conn, String),
    /// The client of a server connection has the mission objects.
    ReplicasDone(Conn),
}

#[derive(Debug, Clone)]
pub struct Queued {
    /// Dropped if this connection is gone at delivery time.
    pub requires: Option<Conn>,
    pub call: Deferred,
}

#[derive(Default)]
pub struct Net {
    /// Client side of the local connection (`ServerConnection`).
    pub client: Option<Conn>,
    /// Server side of the local connection (`LocalClientConnection`).
    pub server: Option<Conn>,
    /// The client's connection object (`ServerConnection`, to the local
    /// server or a remote one; the remote link itself is `crate::netgame`'s).
    /// It stays until it is deleted, after its link dropped too.
    pub server_conn: Option<Conn>,
    /// Connections that exist (both ends of the local one, the client's
    /// connection to a remote server until it is deleted).
    pub alive: HashSet<Conn>,
    /// Server connection -> control object (the player's tank entity).
    pub control: HashMap<Conn, tt_gameplay::EntityId>,
    /// Server connections with replication active.
    pub replication: HashSet<Conn>,
    /// `initialControlSet` already sent for this server connection.
    pub control_sent: HashSet<Conn>,
    pub queue: VecDeque<Queued>,
    /// The next local connection id.
    pub next_local: u32,
    /// The next remote connection id ([`REMOTE_BASE`] + n).
    pub next_remote: u32,
    /// Messages and commands sent, for reports: name -> count.
    pub sent: HashMap<String, usize>,
}

impl Net {
    pub fn defer(&mut self, requires: Option<Conn>, call: Deferred) {
        self.queue.push_back(Queued { requires, call });
    }

    /// A callback of the client's connection `conn` (dropped if it is gone).
    pub fn client_cb(&mut self, conn: Option<Conn>, cb: ClientCb) {
        self.defer(conn, Deferred::Client(cb));
    }

    /// A new connection of this process (local ends, the client's connection).
    pub fn new_conn(&mut self) -> Conn {
        self.next_local += 1;
        let c = self.next_local;
        self.alive.insert(c);
        c
    }

    /// A new remote connection id.
    pub fn new_remote(&mut self) -> Conn {
        let c = REMOTE_BASE + self.next_remote;
        self.next_remote += 1;
        c
    }

    /// The other end of a local connection.
    pub fn peer(&self, conn: Conn) -> Option<Conn> {
        if Some(conn) == self.client {
            self.server
        } else if Some(conn) == self.server {
            self.client
        } else {
            None
        }
    }

    /// The client's connection (`ServerConnection`, local or remote).
    pub fn server_connection(&self) -> Option<Conn> {
        self.server_conn
    }

    /// Control object as seen from either end of the connection.
    pub fn control_object(&self, conn: Conn) -> Option<tt_gameplay::EntityId> {
        let server = if Some(conn) == self.client { self.server? } else { conn };
        self.control.get(&server).copied()
    }

    pub fn forget(&mut self, conn: Conn) {
        if self.client == Some(conn) {
            self.client = None;
        }
        if self.server == Some(conn) {
            self.server = None;
        }
        self.control.remove(&conn);
        self.replication.remove(&conn);
        self.control_sent.remove(&conn);
    }
}

/// Is `conn` there (a connection of this process, or a remote client)?
pub fn alive(h: &EngineHost, conn: Conn) -> bool {
    h.net.alive.contains(&conn) || crate::netgame::is_remote(h, conn)
}

/// `getAddress()` of a connection ("local" for the local connection).
pub fn address(h: &EngineHost, conn: Conn) -> String {
    crate::netgame::address(h, conn).unwrap_or_else(|| "local".into())
}

/// Objects of the loaded mission a client receives as always replicated
/// objects (the scene objects; groups and script objects are not).
pub fn replica_count(h: &EngineHost) -> usize {
    const NET_OBJECTS: &[&str] = &["Terrain", "Sky", "Sun", "MissionArea", "Prop", "MissionMarker", "SpawnPoint", "WayPoint", "PowerUp", "Boost", "Volcano", "Flag", "Tank", "Camera", "Projectile"];
    h.mission_load.as_ref().map_or(0, |l| l.nodes.iter().filter(|(_, o)| NET_OBJECTS.iter().any(|c| c.eq_ignore_ascii_case(&o.class))).count())
}

/// The local connection a launcher asked for (`new GameConnection(ServerConnection)`,
/// its connect arguments, `connectLocal`): creates the server side, runs the connect
/// request and establishes both ends.
pub fn connect_local(h: &mut EngineHost, args: Vec<String>) {
    let client = h.net.new_conn();
    h.net.server_conn = Some(client);
    if let Some(old) = h.net.server.filter(|s| h.net.alive.contains(s)) {
        delete_conn(h, old);
    }
    let server = h.net.new_conn();
    h.net.client = Some(client);
    h.net.server = Some(server);
    let reply = crate::gameplay::server_host::connect_request(h, "local");
    if !reply.is_empty() {
        h.log.info(&format!("Local connection rejected: {reply}"));
        crate::gameplay::client::connection::on_connect_request_rejected(h, &reply);
        h.net.server = None;
        delete_conn(h, server);
        return;
    }
    // onConnectionEstablished: client first (onConnectionAccepted), then server (onConnect).
    crate::gameplay::client::connection::on_connection_accepted(h);
    crate::gameplay::server_host::connected(h, server, &args);
    h.log.info("Local connection established");
}

/// Joins a remote server (`-connect`, the menu's Join): a new client
/// connection with the player's name, team (the prefs' when empty) and
/// tank, connecting to `address`.
pub fn connect_to(h: &mut EngineHost, address: &str, password: &str, name: &str, team: &str) -> bool {
    if let Some(old) = h.net.server_connection() {
        delete_conn(h, old);
    }
    let conn = h.net.new_conn();
    let prefs = &h.shell.prefs;
    let pick = |v: &str, pref: &str| if v.is_empty() { prefs.get(pref) } else { v.to_string() };
    let args = vec![pick(name, "player.name"), pick(team, "player.team"), prefs.get("player.tank")];
    h.net.server_conn = Some(conn);
    crate::netgame::connect(h, conn, address, args, password)
}

/// A client command to the server: the local server gets it as the next
/// "packet", a remote one over the network (nothing without a connection).
pub fn send_cmd(h: &mut EngineHost, cmd: ClientCmd) {
    let (Some(_), Some(server)) = (h.net.client, h.net.server) else {
        crate::netgame::send_cmd(h, cmd);
        return;
    };
    *h.net.sent.entry(cmd.lower().0.to_string()).or_default() += 1;
    h.net.defer(Some(server), Deferred::ServerCmd(server, cmd));
}

/// A server message to connection `target`: a remote client gets it over
/// the network, the local client as the next "packet" (bots and test
/// clients have no connection).
pub fn send_to_client(h: &mut EngineHost, target: Conn, msg: &ServerMsg) {
    if crate::netgame::is_remote(h, target) {
        crate::netgame::send_game(h, target, msg);
        return;
    }
    let (Some(server), Some(client)) = (h.net.server, h.net.client) else { return };
    if target != server {
        return;
    }
    *h.net.sent.entry(msg.lower().name.to_string()).or_default() += 1;
    h.net.defer(Some(client), Deferred::ServerMsg(msg.clone()));
}

/// `conn.transmitDefs(seq)`.
pub fn transmit_defs(h: &mut EngineHost, conn: Conn, seq: &str) {
    if crate::netgame::is_remote(h, conn) {
        crate::netgame::transmit_defs(h, conn, seq);
        return;
    }
    let client = h.net.peer(conn);
    let n = h.defs.len().max(1);
    for i in 1..=n {
        h.net.client_cb(client, ClientCb::DefReceived(i as u32, n as u32));
    }
    h.net.defer(Some(conn), Deferred::DefsDone(conn, seq.to_string()));
}

/// `conn.startReplication()`.
pub fn start_replication(h: &mut EngineHost, conn: Conn, mission_file: &str) {
    h.net.replication.insert(conn);
    if crate::netgame::is_remote(h, conn) {
        crate::netgame::start_replication(h, conn, mission_file);
    } else {
        let client = h.net.peer(conn);
        let n = replica_count(h);
        h.net.client_cb(client, ClientCb::ReplicasStarted(n as u32));
        for _ in 0..n {
            h.net.client_cb(client, ClientCb::ReplicaReceived);
        }
        h.net.defer(Some(conn), Deferred::ReplicasDone(conn));
    }
    send_control(h, conn);
}

/// `conn.resetReplication()`.
pub fn reset_replication(h: &mut EngineHost, conn: Conn) {
    h.net.replication.remove(&conn);
    h.net.control_sent.remove(&conn);
    if crate::netgame::is_remote(h, conn) {
        crate::netgame::reset_replication(h, conn);
    }
}

/// `conn.setControlObject(tank)`.
pub fn set_control(h: &mut EngineHost, conn: Conn, tank: tt_gameplay::EntityId) {
    h.net.control.insert(conn, tank);
    send_control(h, conn);
}

/// The client learns its control object once replication runs (`initialControlSet`).
fn send_control(h: &mut EngineHost, server: Conn) {
    if !h.net.replication.contains(&server) || !h.net.control.contains_key(&server) || h.net.control_sent.contains(&server) {
        return;
    }
    if crate::netgame::is_remote(h, server) {
        // It learns it from the next snapshot (whose header names its tank).
        h.net.control_sent.insert(server);
    } else if let Some(client) = h.net.peer(server) {
        h.net.control_sent.insert(server);
        h.net.client_cb(Some(client), ClientCb::InitialControlSet);
    }
}

/// The control object of the client's connection (the player's tank
/// entity), if it is in the world (`ServerConnection.getControlObject()`).
pub fn client_control_object(h: &EngineHost) -> Option<tt_gameplay::EntityId> {
    let local = h.net.client.and_then(|c| h.net.control_object(c));
    local.or_else(|| crate::netgame::control_object(h)).filter(|e| h.game.as_ref().is_some_and(|g| g.entities.get(*e).is_some()))
}

/// `lightScene(callback)`: the renderer lights the scene; the callback
/// runs on the next tick.
pub fn light_scene(h: &mut EngineHost) {
    h.net.client_cb(None, ClientCb::SceneLightingComplete);
}

/// `deleteDefs()`: every def but the client's own.
pub fn delete_defs(h: &mut EngineHost) {
    h.defs.delete_defs();
    h.weapons.clear();
    crate::gameplay::defs::refresh(h);
    crate::netgame::defs_deleted(h);
}

/// Delivers queued calls (one "packet round"); calls queued meanwhile wait for
/// the next round.
pub fn deliver(h: &mut EngineHost) {
    let batch: Vec<Queued> = h.net.queue.drain(..).collect();
    for q in batch {
        if q.requires.is_some_and(|c| !alive(h, c)) {
            continue;
        }
        match q.call {
            Deferred::Client(cb) => crate::gameplay::client::connection::callback(h, cb),
            Deferred::DeleteClientConn(c) => delete_conn(h, c),
            Deferred::NetMsg(conn, msg) => crate::netgame::apply_client_msg(h, conn, msg),
            Deferred::ServerMsg(msg) => crate::gameplay::client::recv::apply(h, &msg),
            Deferred::ServerCmd(conn, cmd) => crate::gameplay::server_host::client_cmd_from(h, conn, &cmd),
            Deferred::DefsDone(conn, seq) => crate::gameplay::server_host::defs_done(h, conn, &seq),
            Deferred::ReplicasDone(conn) => crate::gameplay::server_host::replicas_done(h, conn),
        }
    }
}

/// Deletes connection `conn` (a local end or the client's connection to a
/// remote server; remote clients go with `crate::netgame::remove_remote`):
/// the other end drops like a disconnect.
pub fn delete_conn(h: &mut EngineHost, conn: Conn) {
    if !h.net.alive.remove(&conn) {
        return;
    }
    if h.net.server_conn == Some(conn) {
        h.net.server_conn = None;
    }
    let local = h.net.client == Some(conn) || h.net.server == Some(conn);
    if local {
        on_connection_removed(h, conn);
    } else if crate::netgame::is_remote_server(h, conn) {
        crate::netgame::on_connection_removed(h, conn, "");
    }
    h.net.forget(conn);
    crate::gameplay::server_host::connection_removed(h, conn);
}

/// An end of the local connection is going away: drop the other end like a disconnect.
fn on_connection_removed(h: &mut EngineHost, conn: Conn) {
    let peer = h.net.peer(conn);
    let was_server = h.net.server == Some(conn);
    h.net.forget(conn);
    if was_server {
        crate::gameplay::server_host::dropped(h, conn);
    }
    if let Some(p) = peer.filter(|p| h.net.alive.contains(p)) {
        if was_server {
            // Server dropped the client: the client's connection gets onConnectionDropped.
            h.net.forget(p);
            crate::gameplay::client::connection::callback(h, ClientCb::ConnectionDropped("Server shut down".into()));
        } else {
            // Client disconnected: the server side is deleted (its onDrop runs).
            delete_conn(h, p);
        }
    }
}

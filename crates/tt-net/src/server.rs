//! Server endpoint: stateless cookie handshake, connection table, LAN info
//! replies (docs/netcode.md §5.3, §5.6, §6).
//!
//! ```text
//! C→S ConnectRequest{magic, protocol, build, nonce}        padded to 256 B
//! S→C Challenge{nonce, cookie} | Reject{nonce, "CR_WRONGVERSION"}   ≤ request, no state
//! C→S ChallengeResponse{…, nonce, cookie, connect args}   padded to ≥ 256 B
//!       cookie checked → ServerEvent::ConnectRequest → app calls accept / reject
//! S→C Accept{nonce, session} | Reject{nonce, reason}       ≤ request
//! ```

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::conn::{ConnConfig, Connection, DisconnectReason, Lane};
use crate::master::{MasterPacket, MasterStatus, HEARTBEAT_INTERVAL, MASTER_VERSION, PUNCHES};
use crate::packet::{DataBody, Packet, MAX_INFO_PAYLOAD, MAX_PACKET_SIZE};
use crate::rng::{siphash24, Rng};
use crate::stats::ConnStats;
use crate::transport::Datagram;
use crate::{SendError, BUILD_ID, PROTOCOL_VERSION};

/// Cookie validity bucket; a cookie is accepted in its bucket and the next
/// (10–20 s lifetime).
pub const COOKIE_BUCKET: Duration = Duration::from_secs(10);
/// Reject reason for a protocol/build mismatch.
pub const CR_WRONGVERSION: &str = "CR_WRONGVERSION";
/// Reject reason when `max_clients` connections exist.
pub const CR_SERVERFULL: &str = "CR_SERVERFULL";

/// Server settings.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub protocol: u16,
    pub build: u64,
    /// Hard cap on connections (the protocol allows 64 tank slots).
    pub max_clients: usize,
    pub conn: ConnConfig,
    /// Pending (cookie-verified, undecided) connections per IP.
    pub max_pending_per_ip: usize,
    /// Pending connections in total.
    pub max_pending: usize,
    /// The app must accept/reject a pending connection within this.
    pub pending_timeout: Duration,
    /// `ConnectRequest`s processed per second, globally (token bucket, burst = rate).
    pub handshake_rate: u32,
    /// Packets per second accepted from one address (token bucket, burst =
    /// rate); more are dropped unread (§6 flood limit).
    pub max_packets_per_sec: u32,
    /// Undecodable (or wrong-session) packets per second from a connected
    /// peer's address before it is kicked (§6).
    pub max_malformed_per_sec: u32,
    /// Payload of `Info` replies to `InfoQuery` (≤ [`MAX_INFO_PAYLOAD`]);
    /// `None` = do not answer queries.
    pub info: Option<Vec<u8>>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            protocol: PROTOCOL_VERSION,
            build: BUILD_ID,
            max_clients: 64,
            conn: ConnConfig::default(),
            max_pending_per_ip: 4,
            max_pending: 64,
            pending_timeout: Duration::from_secs(5),
            handshake_rate: 10,
            max_packets_per_sec: 64,
            max_malformed_per_sec: 100,
            info: None,
        }
    }
}

/// Server-side connection handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnId(pub u32);

/// A verified connection attempt waiting for [`Server::accept`] / [`Server::reject`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PendingId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    /// A client proved its address; `payload` is its connect args.
    /// `handshake`: its password proof ([`crate::auth::Handshake::verify`]).
    ConnectRequest { pending: PendingId, addr: SocketAddr, payload: Vec<u8>, handshake: crate::auth::Handshake },
    Connected { conn: ConnId, addr: SocketAddr },
    Disconnected { conn: ConnId, addr: SocketAddr, reason: DisconnectReason },
    Message { conn: ConnId, lane: Lane, data: Vec<u8> },
}

/// Endpoint-wide counters (packets not attributable to a connection).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointStats {
    pub malformed: u64,
    pub unknown_source: u64,
    pub bad_session: u64,
    pub bad_cookie: u64,
    pub rate_limited: u64,
    pub version_rejects: u64,
    pub replays: u64,
    /// Packets dropped by the per-address flood limit.
    pub flood_dropped: u64,
    /// Connections kicked for malformed packets.
    pub malformed_kicks: u64,
}

#[derive(Debug)]
struct Pending {
    addr: SocketAddr,
    nonce: u64,
    cookie: u64,
    created: Duration,
}

#[derive(Debug, Clone)]
enum CookieUse {
    Pending,
    Accepted(ConnId, u32),
    Rejected(String),
}

pub struct Server<T: Datagram> {
    transport: T,
    cfg: ServerConfig,
    key: (u64, u64),
    rng: Rng,
    conns: HashMap<ConnId, Connection>,
    by_addr: HashMap<SocketAddr, ConnId>,
    next_conn: u32,
    pending: HashMap<u64, Pending>,
    next_pending: u64,
    used_cookies: HashMap<u64, (Duration, CookieUse)>,
    tokens: f64,
    tokens_at: Duration,
    events: VecDeque<ServerEvent>,
    stats: EndpointStats,
    /// Per-address token buckets: (tokens, last refill).
    flood: HashMap<SocketAddr, (f64, Duration)>,
    /// Per-connection malformed packets in the current second: (window start, count).
    malformed: HashMap<ConnId, (Duration, u32)>,
    master: MasterLink,
}

/// Registration with a master server (see [`crate::master`]).
#[derive(Debug, Default)]
struct MasterLink {
    status: MasterStatus,
    server_id: u64,
    nonce: u64,
    cookie: u64,
    last_sent: Option<Duration>,
    last_ack: Option<Duration>,
    /// Punch budget (token bucket, [`PUNCH_RATE`]/s): (tokens, last refill).
    punch: (f64, Duration),
}

/// Introductions answered per second at most.
const PUNCH_RATE: f64 = 20.0;
/// Heartbeat retry while not registered.
const HEARTBEAT_RETRY: Duration = Duration::from_secs(2);

impl<T: Datagram> Server<T> {
    /// A server with a random cookie key and session ids.
    pub fn new(transport: T, cfg: ServerConfig) -> Self {
        Self::with_rng(transport, cfg, Rng::from_entropy())
    }

    /// Deterministic variant for tests.
    pub fn with_seed(transport: T, cfg: ServerConfig, seed: u64) -> Self {
        Self::with_rng(transport, cfg, Rng::new(seed))
    }

    fn with_rng(transport: T, cfg: ServerConfig, mut rng: Rng) -> Self {
        let key = (rng.next_u64(), rng.next_u64());
        let tokens = cfg.handshake_rate as f64;
        Server {
            transport,
            cfg,
            key,
            rng,
            conns: HashMap::new(),
            by_addr: HashMap::new(),
            next_conn: 0,
            pending: HashMap::new(),
            next_pending: 0,
            used_cookies: HashMap::new(),
            tokens,
            tokens_at: Duration::ZERO,
            events: VecDeque::new(),
            stats: EndpointStats::default(),
            flood: HashMap::new(),
            malformed: HashMap::new(),
            master: MasterLink::default(),
        }
    }

    /// Registers with (heartbeats to) a master server, or stops (`None`:
    /// a closing heartbeat takes this server off the old master's list).
    /// `server_id` is the instance id of the `Info` replies.
    pub fn set_master(&mut self, master: Option<SocketAddr>, server_id: u64, now: Duration) {
        if self.master.status.master == master {
            self.master.server_id = server_id;
            return;
        }
        if let Some(old) = self.master.status.master {
            if self.master.status.registered {
                self.send_heartbeat(old, true);
            }
        }
        let nonce = self.rng.next_u64();
        self.master = MasterLink { status: MasterStatus { master, ..Default::default() }, server_id, nonce, punch: (PUNCH_RATE, now), ..Default::default() };
    }

    /// Registration state with the master server.
    pub fn master_status(&self) -> &MasterStatus {
        &self.master.status
    }

    fn send_heartbeat(&mut self, to: SocketAddr, closing: bool) {
        let m = &self.master;
        let p = MasterPacket::Heartbeat { version: MASTER_VERSION, nonce: m.nonce, cookie: m.cookie, server_id: m.server_id, closing };
        let _ = self.transport.send_to(&p.encode(), to);
    }

    fn master_timers(&mut self, now: Duration) {
        let Some(to) = self.master.status.master else { return };
        let m = &mut self.master;
        if m.status.registered && m.last_ack.is_some_and(|a| now.saturating_sub(a) > crate::master::ENTRY_LIFETIME) {
            m.status.registered = false;
            m.cookie = 0;
        }
        let every = if m.status.registered { HEARTBEAT_INTERVAL } else { HEARTBEAT_RETRY };
        if m.last_sent.is_none_or(|t| now.saturating_sub(t) >= every) {
            m.last_sent = Some(now);
            self.send_heartbeat(to, false);
        }
    }

    fn on_master_packet(&mut self, data: &[u8], from: SocketAddr, now: Duration) {
        let Ok(p) = MasterPacket::decode(data) else {
            self.stats.malformed += 1;
            return;
        };
        if Some(from) != self.master.status.master {
            // Punches from introduced peers (and anything else) are ignored.
            return;
        }
        match p {
            MasterPacket::HeartbeatChallenge { nonce, cookie } if nonce == self.master.nonce => {
                self.master.cookie = cookie;
                self.master.last_sent = Some(now);
                self.send_heartbeat(from, false);
            }
            MasterPacket::HeartbeatAck { nonce, public } if nonce == self.master.nonce => {
                self.master.status.registered = true;
                self.master.status.public_addr = Some(public);
                self.master.last_ack = Some(now);
            }
            MasterPacket::Introduce { client } => {
                let (tokens, at) = self.master.punch;
                let tokens = (tokens + now.saturating_sub(at).as_secs_f64() * PUNCH_RATE).min(PUNCH_RATE);
                if tokens < 1.0 {
                    self.master.punch = (tokens, now);
                    return;
                }
                self.master.punch = (tokens - 1.0, now);
                self.master.status.introductions += 1;
                let punch = MasterPacket::Punch { nonce: self.rng.next_u64() }.encode();
                for _ in 0..PUNCHES {
                    let _ = self.transport.send_to(&punch, client);
                }
            }
            _ => {}
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.transport.local_addr()
    }

    /// Replaces the `Info` payload for LAN queries.
    pub fn set_info(&mut self, info: Option<Vec<u8>>) {
        self.cfg.info = info;
    }

    pub fn endpoint_stats(&self) -> &EndpointStats {
        &self.stats
    }

    pub fn connections(&self) -> impl Iterator<Item = ConnId> + '_ {
        self.conns.keys().copied()
    }

    pub fn connection_count(&self) -> usize {
        self.conns.len()
    }

    pub fn addr(&self, conn: ConnId) -> Option<SocketAddr> {
        self.conns.get(&conn).map(|c| c.addr)
    }

    pub fn stats(&mut self, conn: ConnId, now: Duration) -> Option<ConnStats> {
        self.conns.get_mut(&conn).map(|c| c.stats(now))
    }

    /// Packet seqs the client acked since the last call.
    pub fn take_acked(&mut self, conn: ConnId) -> Vec<u16> {
        self.conns.get_mut(&conn).map(|c| c.take_acked()).unwrap_or_default()
    }

    /// Unacked reliable bytes for `conn` (back off before 64 KB).
    pub fn reliable_backlog(&self, conn: ConnId) -> Option<usize> {
        self.conns.get(&conn).map(|c| c.reliable_backlog())
    }

    /// Sequence number of the next packet to `conn`.
    pub fn next_seq(&self, conn: ConnId) -> Option<u16> {
        self.conns.get(&conn).map(|c| c.next_seq())
    }

    /// Per-connection timeout (e.g. 15 s while loading).
    pub fn set_timeout(&mut self, conn: ConnId, timeout: Duration) {
        if let Some(c) = self.conns.get_mut(&conn) {
            c.cfg.timeout = timeout;
        }
    }

    pub fn drain_events(&mut self) -> Vec<ServerEvent> {
        self.events.drain(..).collect()
    }

    fn cookie(&self, addr: SocketAddr, nonce: u64, bucket: u64) -> u64 {
        let mut b = [0u8; 34];
        let ip = match addr.ip() {
            IpAddr::V4(v) => v.to_ipv6_mapped().octets(),
            IpAddr::V6(v) => v.octets(),
        };
        b[..16].copy_from_slice(&ip);
        b[16..18].copy_from_slice(&addr.port().to_le_bytes());
        b[18..26].copy_from_slice(&nonce.to_le_bytes());
        b[26..34].copy_from_slice(&bucket.to_le_bytes());
        siphash24(self.key.0, self.key.1, &b)
    }

    fn bucket(now: Duration) -> u64 {
        now.as_secs() / COOKIE_BUCKET.as_secs()
    }

    fn cookie_valid(&self, addr: SocketAddr, nonce: u64, cookie: u64, now: Duration) -> bool {
        let b = Self::bucket(now);
        cookie == self.cookie(addr, nonce, b) || (b > 0 && cookie == self.cookie(addr, nonce, b - 1))
    }

    fn take_token(&mut self, now: Duration) -> bool {
        let rate = self.cfg.handshake_rate as f64;
        let dt = now.saturating_sub(self.tokens_at).as_secs_f64();
        self.tokens_at = now;
        self.tokens = (self.tokens + dt * rate).min(rate);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            self.stats.rate_limited += 1;
            false
        }
    }

    fn send_raw(&mut self, p: &Packet, to: SocketAddr) {
        let _ = self.transport.send_to(&p.encode(), to);
    }

    /// Receives everything pending, runs timers and emits events.
    pub fn update(&mut self, now: Duration) {
        let mut buf = [0u8; MAX_PACKET_SIZE + 1];
        while let Some((n, from)) = self.transport.recv_from(&mut buf) {
            if !self.allow_packet(from, now) {
                self.stats.flood_dropped += 1;
                continue;
            }
            let data = buf[..n].to_vec();
            self.handle(&data, from, now);
        }
        self.flood.retain(|_, (_, last)| now.saturating_sub(*last) < Duration::from_secs(5));
        let live: std::collections::HashSet<ConnId> = self.conns.keys().copied().collect();
        self.malformed.retain(|id, _| live.contains(id));
        let mut dead = Vec::new();
        for (&id, c) in &mut self.conns {
            if c.update(now) {
                dead.push(id);
            }
        }
        dead.sort();
        for id in dead {
            if let Some(mut c) = self.conns.remove(&id) {
                c.send_disconnect("timeout", now, &mut self.transport);
                self.by_addr.remove(&c.addr);
                self.events.push_back(ServerEvent::Disconnected {
                    conn: id,
                    addr: c.addr,
                    reason: DisconnectReason::Timeout,
                });
            }
        }
        self.master_timers(now);
        let pt = self.cfg.pending_timeout;
        self.pending.retain(|_, p| now.saturating_sub(p.created) <= pt);
        self.used_cookies.retain(|_, (expiry, _)| *expiry > now);
    }

    /// Sends due packets to every connection.
    pub fn flush(&mut self, now: Duration) {
        let mut ids: Vec<ConnId> = self.conns.keys().copied().collect();
        ids.sort();
        for id in ids {
            if let Some(c) = self.conns.get_mut(&id) {
                c.flush(now, &mut self.transport);
            }
        }
    }

    /// Queues a message for `conn`. On [`SendError::Overflow`] the connection
    /// is dropped (event `Disconnected{Overflow}`).
    pub fn send(&mut self, conn: ConnId, lane: Lane, data: &[u8], now: Duration) -> Result<(), SendError> {
        let c = self.conns.get_mut(&conn).ok_or(SendError::NotConnected)?;
        let r = c.send(lane, data, now);
        if r == Err(SendError::Overflow) {
            self.drop_conn(conn, DisconnectReason::Overflow, now);
        }
        r
    }

    /// Closes a connection gracefully (`Disconnect{reason}` ×3).
    pub fn disconnect(&mut self, conn: ConnId, reason: &str, now: Duration) {
        self.drop_conn(conn, DisconnectReason::Local(reason.to_string()), now);
    }

    fn drop_conn(&mut self, conn: ConnId, reason: DisconnectReason, now: Duration) {
        if let Some(mut c) = self.conns.remove(&conn) {
            c.send_disconnect(reason.text(), now, &mut self.transport);
            self.by_addr.remove(&c.addr);
            self.events.push_back(ServerEvent::Disconnected { conn, addr: c.addr, reason });
        }
    }

    /// Accepts a pending connection; sends `Accept`.
    pub fn accept(&mut self, pending: PendingId, now: Duration) -> Option<ConnId> {
        let p = self.pending.remove(&pending.0)?;
        if self.conns.len() >= self.cfg.max_clients {
            self.reject_pending(p, CR_SERVERFULL, now);
            return None;
        }
        if let Some(&old) = self.by_addr.get(&p.addr) {
            self.drop_conn(old, DisconnectReason::Replaced, now);
        }
        let session = loop {
            let s = self.rng.next_u32();
            if s != 0 {
                break s;
            }
        };
        let id = ConnId(self.next_conn);
        self.next_conn = self.next_conn.wrapping_add(1);
        let conn = Connection::new(p.addr, session, self.cfg.conn.clone(), now, Duration::from_millis(100));
        self.conns.insert(id, conn);
        self.by_addr.insert(p.addr, id);
        self.used_cookies.insert(p.cookie, (now + COOKIE_BUCKET * 3, CookieUse::Accepted(id, session)));
        self.send_raw(&Packet::Accept { nonce: p.nonce, session }, p.addr);
        self.events.push_back(ServerEvent::Connected { conn: id, addr: p.addr });
        Some(id)
    }

    /// Refuses a pending connection with `reason` (e.g. `onConnectRequest`'s).
    pub fn reject(&mut self, pending: PendingId, reason: &str, now: Duration) {
        if let Some(p) = self.pending.remove(&pending.0) {
            self.reject_pending(p, reason, now);
        }
    }

    fn reject_pending(&mut self, p: Pending, reason: &str, now: Duration) {
        self.used_cookies.insert(p.cookie, (now + COOKIE_BUCKET * 3, CookieUse::Rejected(reason.to_string())));
        self.send_raw(&Packet::Reject { nonce: p.nonce, reason: reason.to_string() }, p.addr);
    }

    /// Per-address flood limit: a token bucket of `max_packets_per_sec`.
    fn allow_packet(&mut self, from: SocketAddr, now: Duration) -> bool {
        let rate = self.cfg.max_packets_per_sec as f64;
        let e = self.flood.entry(from).or_insert((rate, now));
        e.0 = (e.0 + now.saturating_sub(e.1).as_secs_f64() * rate).min(rate);
        e.1 = now;
        if e.0 >= 1.0 {
            e.0 -= 1.0;
            true
        } else {
            false
        }
    }

    /// A bad packet from a connected peer's address; too many per second kick it.
    fn malformed_from(&mut self, id: ConnId, now: Duration) {
        if let Some(c) = self.conns.get_mut(&id) {
            c.note_malformed();
        }
        let e = self.malformed.entry(id).or_insert((now, 0));
        if now.saturating_sub(e.0) >= Duration::from_secs(1) {
            *e = (now, 0);
        }
        e.1 += 1;
        if e.1 > self.cfg.max_malformed_per_sec {
            self.malformed.remove(&id);
            self.stats.malformed_kicks += 1;
            self.drop_conn(id, DisconnectReason::Protocol("too many malformed packets"), now);
        }
    }

    fn handle(&mut self, data: &[u8], from: SocketAddr, now: Duration) {
        if crate::master::is_master_packet(data) {
            return self.on_master_packet(data, from, now);
        }
        let pkt = match Packet::decode(data) {
            Ok(p) => p,
            Err(_) => {
                self.stats.malformed += 1;
                if let Some(&id) = self.by_addr.get(&from) {
                    self.malformed_from(id, now);
                }
                return;
            }
        };
        match pkt {
            Packet::ConnectRequest { protocol, build, nonce } => {
                if !self.take_token(now) {
                    return;
                }
                if protocol != self.cfg.protocol || build != self.cfg.build {
                    self.stats.version_rejects += 1;
                    self.send_raw(&Packet::Reject { nonce, reason: CR_WRONGVERSION.into() }, from);
                    return;
                }
                let cookie = self.cookie(from, nonce, Self::bucket(now));
                self.send_raw(&Packet::Challenge { nonce, cookie }, from);
            }
            Packet::ChallengeResponse { protocol, build, nonce, cookie, proof, payload } => {
                self.on_challenge_response(protocol, build, nonce, cookie, proof, payload, from, now)
            }
            Packet::Data { header, body } => {
                let Some(id) = self.conn_for(from, header.session) else { return };
                let body = match DataBody::decode(&body) {
                    Ok(b) => b,
                    Err(_) => {
                        self.stats.malformed += 1;
                        self.malformed_from(id, now);
                        return;
                    }
                };
                self.on_conn_packet(id, &header, Some(body), data.len(), now);
            }
            Packet::Keepalive { header } => {
                if let Some(id) = self.conn_for(from, header.session) {
                    self.on_conn_packet(id, &header, None, data.len(), now);
                }
            }
            Packet::Disconnect { header, reason } => {
                if let Some(id) = self.conn_for(from, header.session) {
                    if let Some(c) = self.conns.remove(&id) {
                        self.by_addr.remove(&c.addr);
                        self.events.push_back(ServerEvent::Disconnected {
                            conn: id,
                            addr: from,
                            reason: DisconnectReason::Remote(reason),
                        });
                    }
                }
            }
            Packet::InfoQuery { protocol: _, nonce } => {
                // Stateless and never larger than the query: no rate limit needed here.
                if let Some(info) = &self.cfg.info {
                    let payload = info[..info.len().min(MAX_INFO_PAYLOAD)].to_vec();
                    let p = Packet::Info { protocol: self.cfg.protocol, nonce, payload };
                    self.send_raw(&p, from);
                }
            }
            Packet::Challenge { .. } | Packet::Accept { .. } | Packet::Reject { .. } | Packet::Info { .. } => {
                self.stats.unknown_source += 1;
            }
        }
    }

    fn conn_for(&mut self, from: SocketAddr, session: u32) -> Option<ConnId> {
        let Some(&id) = self.by_addr.get(&from) else {
            self.stats.unknown_source += 1;
            return None;
        };
        let c = self.conns.get_mut(&id)?;
        if c.session != session {
            self.stats.bad_session += 1;
            c.note_malformed();
            return None;
        }
        Some(id)
    }

    fn on_conn_packet(&mut self, id: ConnId, h: &crate::packet::Header, body: Option<DataBody>, size: usize, now: Duration) {
        let Some(c) = self.conns.get_mut(&id) else { return };
        if let Err(e) = c.on_packet(h, body, size, now) {
            self.drop_conn(id, DisconnectReason::Protocol(e), now);
            return;
        }
        while let Some((lane, data)) = c.inbox.pop_front() {
            self.events.push_back(ServerEvent::Message { conn: id, lane, data });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_challenge_response(
        &mut self,
        protocol: u16,
        build: u64,
        nonce: u64,
        cookie: u64,
        proof: u64,
        payload: Vec<u8>,
        from: SocketAddr,
        now: Duration,
    ) {
        if !self.cookie_valid(from, nonce, cookie, now) {
            self.stats.bad_cookie += 1;
            return;
        }
        if let Some((_, used)) = self.used_cookies.get(&cookie).cloned() {
            match used {
                CookieUse::Pending => {}
                CookieUse::Accepted(id, session) => {
                    if self.by_addr.get(&from) == Some(&id) {
                        // Our Accept was lost: repeat it.
                        self.send_raw(&Packet::Accept { nonce, session }, from);
                    } else {
                        self.stats.replays += 1;
                    }
                }
                CookieUse::Rejected(reason) => self.send_raw(&Packet::Reject { nonce, reason }, from),
            }
            return;
        }
        if protocol != self.cfg.protocol || build != self.cfg.build {
            self.stats.version_rejects += 1;
            self.send_raw(&Packet::Reject { nonce, reason: CR_WRONGVERSION.into() }, from);
            return;
        }
        if self.conns.len() >= self.cfg.max_clients {
            let p = Pending { addr: from, nonce, cookie, created: now };
            self.reject_pending(p, CR_SERVERFULL, now);
            return;
        }
        let same_ip = self.pending.values().filter(|p| p.addr.ip() == from.ip()).count();
        if same_ip >= self.cfg.max_pending_per_ip || self.pending.len() >= self.cfg.max_pending {
            self.stats.rate_limited += 1;
            return;
        }
        let id = self.next_pending;
        self.next_pending += 1;
        self.pending.insert(id, Pending { addr: from, nonce, cookie, created: now });
        self.used_cookies.insert(cookie, (now + COOKIE_BUCKET * 3, CookieUse::Pending));
        let handshake = crate::auth::Handshake { cookie, nonce, proof };
        self.events.push_back(ServerEvent::ConnectRequest { pending: PendingId(id), addr: from, payload, handshake });
    }
}

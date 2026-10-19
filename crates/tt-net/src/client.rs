//! Client endpoint: handshake state machine, one connection, LAN queries.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;

use crate::conn::{ConnConfig, Connection, DisconnectReason, Lane};
use crate::packet::{DataBody, Packet, MAX_CONNECT_PAYLOAD, MAX_PACKET_SIZE};
use crate::rng::Rng;
use crate::stats::ConnStats;
use crate::transport::Datagram;
use crate::{SendError, BUILD_ID, PROTOCOL_VERSION};

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub protocol: u16,
    pub build: u64,
    pub conn: ConnConfig,
    /// Give up connecting after this.
    pub connect_timeout: Duration,
    /// Resend `ConnectRequest`/`ChallengeResponse` this often.
    pub retry_interval: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            protocol: PROTOCOL_VERSION,
            build: BUILD_ID,
            conn: ConnConfig::default(),
            connect_timeout: Duration::from_secs(5),
            retry_interval: Duration::from_millis(250),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    Disconnected,
    /// Sent `ConnectRequest`, waiting for `Challenge`.
    Requesting,
    /// Sent `ChallengeResponse`, waiting for `Accept`.
    Responding,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    Connected,
    Disconnected(DisconnectReason),
    Message { lane: Lane, data: Vec<u8> },
    /// Answer to [`Client::query_info`].
    Info { from: SocketAddr, payload: Vec<u8>, rtt: Duration },
}

pub struct Client<T: Datagram> {
    transport: T,
    cfg: ClientConfig,
    rng: Rng,
    state: ClientState,
    server: Option<SocketAddr>,
    nonce: u64,
    cookie: u64,
    payload: Vec<u8>,
    started: Duration,
    last_try: Duration,
    conn: Option<Connection>,
    events: VecDeque<ClientEvent>,
    queries: Vec<(u64, Duration)>,
    malformed: u64,
    /// Master server that introduces us to the server (hole punching).
    introducer: Option<SocketAddr>,
    last_intro: Option<Duration>,
    /// Secret for the password proof (0: none), see [`crate::auth`].
    auth_secret: u64,
}

/// Introduction requests while connecting are resent this often.
const INTRO_INTERVAL: Duration = Duration::from_secs(1);

impl<T: Datagram> Client<T> {
    pub fn new(transport: T, cfg: ClientConfig) -> Self {
        Self::with_rng(transport, cfg, Rng::from_entropy())
    }

    pub fn with_seed(transport: T, cfg: ClientConfig, seed: u64) -> Self {
        Self::with_rng(transport, cfg, Rng::new(seed))
    }

    fn with_rng(transport: T, cfg: ClientConfig, rng: Rng) -> Self {
        Client {
            transport,
            cfg,
            rng,
            state: ClientState::Disconnected,
            server: None,
            nonce: 0,
            cookie: 0,
            payload: Vec::new(),
            started: Duration::ZERO,
            last_try: Duration::ZERO,
            conn: None,
            events: VecDeque::new(),
            queries: Vec::new(),
            malformed: 0,
            introducer: None,
            last_intro: None,
            auth_secret: 0,
        }
    }

    /// While connecting, ask this master server to introduce us to the
    /// server (UDP hole punching through both NATs, [`crate::master`]).
    /// Set before [`connect`](Self::connect); `None` connects directly.
    pub fn set_introducer(&mut self, master: Option<SocketAddr>) {
        self.introducer = master;
        self.last_intro = None;
    }

    /// The password secret ([`crate::auth::secret`]) proven in the
    /// handshake: the `ChallengeResponse` carries `proof(secret, cookie,
    /// nonce)`, bound to this handshake, never the password or the secret.
    pub fn set_auth_secret(&mut self, secret: u64) {
        self.auth_secret = secret;
    }

    pub fn is_connected(&self) -> bool {
        self.state == ClientState::Connected
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.transport.local_addr()
    }

    pub fn stats(&mut self, now: Duration) -> Option<ConnStats> {
        self.conn.as_mut().map(|c| c.stats(now))
    }

    pub fn take_acked(&mut self) -> Vec<u16> {
        self.conn.as_mut().map(|c| c.take_acked()).unwrap_or_default()
    }

    /// Unacked reliable bytes (back off before 64 KB).
    pub fn reliable_backlog(&self) -> usize {
        self.conn.as_ref().map_or(0, |c| c.reliable_backlog())
    }

    pub fn next_seq(&self) -> Option<u16> {
        self.conn.as_ref().map(|c| c.next_seq())
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.cfg.conn.timeout = timeout;
        if let Some(c) = self.conn.as_mut() {
            c.cfg.timeout = timeout;
        }
    }

    pub fn drain_events(&mut self) -> Vec<ClientEvent> {
        self.events.drain(..).collect()
    }

    /// Starts connecting; `payload` = connect args (≤ 900 B). Drops any
    /// current connection.
    pub fn connect(&mut self, server: SocketAddr, payload: &[u8], now: Duration) -> Result<(), SendError> {
        if payload.len() > MAX_CONNECT_PAYLOAD {
            return Err(SendError::TooLarge);
        }
        if self.conn.is_some() {
            self.disconnect("reconnect", now);
        }
        self.server = Some(server);
        self.nonce = self.rng.next_u64();
        self.payload = payload.to_vec();
        self.state = ClientState::Requesting;
        self.last_intro = None;
        self.started = now;
        self.send_handshake(now);
        Ok(())
    }

    fn send_raw(&mut self, p: &Packet) {
        if let Some(s) = self.server {
            let _ = self.transport.send_to(&p.encode(), s);
        }
    }

    fn send_handshake(&mut self, now: Duration) {
        self.last_try = now;
        let (protocol, build, nonce) = (self.cfg.protocol, self.cfg.build, self.nonce);
        match self.state {
            ClientState::Requesting => {
                self.send_raw(&Packet::ConnectRequest { protocol, build, nonce });
                // Through NAT: the master asks the server to punch towards us.
                if let (Some(m), Some(s)) = (self.introducer, self.server) {
                    if self.last_intro.is_none_or(|t| now.saturating_sub(t) >= INTRO_INTERVAL) {
                        self.last_intro = Some(now);
                        let p = crate::master::MasterPacket::IntroduceRequest { version: crate::master::MASTER_VERSION, nonce, server: s };
                        let _ = self.transport.send_to(&p.encode(), m);
                    }
                }
            }
            ClientState::Responding => {
                let proof = crate::auth::proof(self.auth_secret, self.cookie, nonce);
                let p = Packet::ChallengeResponse { protocol, build, nonce, cookie: self.cookie, proof, payload: self.payload.clone() };
                self.send_raw(&p)
            }
            _ => {}
        }
    }

    /// Sends an `InfoQuery` (LAN browser / ping before joining). Answers
    /// arrive as [`ClientEvent::Info`].
    pub fn query_info(&mut self, addr: SocketAddr, now: Duration) {
        let nonce = self.rng.next_u64();
        self.queries.retain(|(_, t)| now.saturating_sub(*t) < Duration::from_secs(5));
        self.queries.push((nonce, now));
        let p = Packet::InfoQuery { protocol: self.cfg.protocol, nonce };
        let _ = self.transport.send_to(&p.encode(), addr);
    }

    /// Queues a message to the server.
    pub fn send(&mut self, lane: Lane, data: &[u8], now: Duration) -> Result<(), SendError> {
        let c = self.conn.as_mut().ok_or(SendError::NotConnected)?;
        let r = c.send(lane, data, now);
        if r == Err(SendError::Overflow) {
            self.close(DisconnectReason::Overflow, true, now);
        }
        r
    }

    /// Graceful disconnect (`Disconnect{reason}` ×3). Emits `Disconnected(Local)`.
    pub fn disconnect(&mut self, reason: &str, now: Duration) {
        if self.state != ClientState::Disconnected {
            self.close(DisconnectReason::Local(reason.to_string()), true, now);
        }
    }

    fn close(&mut self, reason: DisconnectReason, notify_peer: bool, now: Duration) {
        if let Some(mut c) = self.conn.take() {
            if notify_peer {
                c.send_disconnect(reason.text(), now, &mut self.transport);
            }
        }
        self.state = ClientState::Disconnected;
        self.events.push_back(ClientEvent::Disconnected(reason));
    }

    /// Receives everything pending and runs handshake/connection timers.
    pub fn update(&mut self, now: Duration) {
        let mut buf = [0u8; MAX_PACKET_SIZE + 1];
        while let Some((n, from)) = self.transport.recv_from(&mut buf) {
            let data = buf[..n].to_vec();
            self.handle(&data, from, now);
        }
        match self.state {
            ClientState::Requesting | ClientState::Responding => {
                if now.saturating_sub(self.started) > self.cfg.connect_timeout {
                    self.close(DisconnectReason::ConnectTimeout, false, now);
                } else if now.saturating_sub(self.last_try) >= self.cfg.retry_interval {
                    self.send_handshake(now);
                }
            }
            ClientState::Connected => {
                if let Some(c) = self.conn.as_mut() {
                    if c.update(now) {
                        self.close(DisconnectReason::Timeout, true, now);
                    }
                }
            }
            ClientState::Disconnected => {}
        }
    }

    /// Sends due packets to the server; returns the number sent.
    pub fn flush(&mut self, now: Duration) -> usize {
        match self.conn.as_mut() {
            Some(c) => c.flush(now, &mut self.transport),
            None => 0,
        }
    }

    fn handle(&mut self, data: &[u8], from: SocketAddr, now: Duration) {
        if crate::master::is_master_packet(data) {
            // Punches: they only opened our NAT.
            return;
        }
        let pkt = match Packet::decode(data) {
            Ok(p) => p,
            Err(_) => {
                self.malformed += 1;
                return;
            }
        };
        if let Packet::Info { nonce, payload, .. } = &pkt {
            if let Some(i) = self.queries.iter().position(|(n, _)| n == nonce) {
                let (_, sent) = self.queries.swap_remove(i);
                self.events.push_back(ClientEvent::Info { from, payload: payload.clone(), rtt: now.saturating_sub(sent) });
            }
            return;
        }
        if Some(from) != self.server {
            return;
        }
        match pkt {
            Packet::Challenge { nonce, cookie } if self.state == ClientState::Requesting && nonce == self.nonce => {
                self.cookie = cookie;
                self.state = ClientState::Responding;
                self.send_handshake(now);
            }
            Packet::Reject { nonce, reason }
                if nonce == self.nonce && matches!(self.state, ClientState::Requesting | ClientState::Responding) =>
            {
                self.close(DisconnectReason::Rejected(reason), false, now);
            }
            Packet::Accept { nonce, session } if self.state == ClientState::Responding && nonce == self.nonce => {
                // RTT seed: time since the last ChallengeResponse.
                let rtt = now.saturating_sub(self.last_try).max(Duration::from_millis(1));
                self.conn = Some(Connection::new(from, session, self.cfg.conn.clone(), now, rtt));
                self.state = ClientState::Connected;
                self.events.push_back(ClientEvent::Connected);
            }
            Packet::Data { header, body } => {
                let Some(c) = self.conn.as_mut().filter(|c| c.session == header.session) else { return };
                let Ok(body) = DataBody::decode(&body) else {
                    self.malformed += 1;
                    c.note_malformed();
                    return;
                };
                if let Err(e) = c.on_packet(&header, Some(body), data.len(), now) {
                    self.close(DisconnectReason::Protocol(e), true, now);
                    return;
                }
                self.drain_inbox();
            }
            Packet::Keepalive { header } => {
                if let Some(c) = self.conn.as_mut().filter(|c| c.session == header.session) {
                    let _ = c.on_packet(&header, None, data.len(), now);
                }
            }
            Packet::Disconnect { header, reason }
                if self.conn.as_ref().is_some_and(|c| c.session == header.session) => {
                    self.close(DisconnectReason::Remote(reason), false, now);
                }
            _ => {}
        }
    }

    fn drain_inbox(&mut self) {
        if let Some(c) = self.conn.as_mut() {
            while let Some((lane, data)) = c.inbox.pop_front() {
                self.events.push_back(ClientEvent::Message { lane, data });
            }
        }
    }
}

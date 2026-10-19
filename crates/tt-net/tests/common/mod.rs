//! Test harness: one server and N clients on a `MemNet`, stepped on virtual time.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::time::Duration;

use tt_net::*;

pub const TICK: Duration = Duration::from_millis(16);

pub fn addr(host: u8, port: u16) -> SocketAddr {
    SocketAddr::from(([10, 0, 0, host], port))
}

pub fn server_addr() -> SocketAddr {
    addr(1, 28000)
}

pub struct Sim {
    pub net: MemNet,
    pub server: Server<MemLink>,
    pub clients: Vec<Client<MemLink>>,
    pub sev: Vec<ServerEvent>,
    pub cev: Vec<Vec<ClientEvent>>,
    /// Accept every ConnectRequest automatically.
    pub auto_accept: bool,
    /// `Some(reason)`: reject every ConnectRequest instead.
    pub auto_reject: Option<String>,
}

impl Sim {
    pub fn new(seed: u64, cond: LinkConditions) -> Sim {
        Self::with_config(seed, cond, ServerConfig::default())
    }

    pub fn with_config(seed: u64, cond: LinkConditions, cfg: ServerConfig) -> Sim {
        let net = MemNet::new(seed);
        net.set_conditions(cond);
        let server = Server::with_seed(net.endpoint(server_addr()), cfg, seed ^ 0x5eed);
        Sim { net, server, clients: Vec::new(), sev: Vec::new(), cev: Vec::new(), auto_accept: true, auto_reject: None }
    }

    pub fn now(&self) -> Duration {
        self.net.now()
    }

    pub fn add_client(&mut self, at: SocketAddr, cfg: ClientConfig) -> usize {
        let seed = self.clients.len() as u64 * 7919 + 1;
        self.clients.push(Client::with_seed(self.net.endpoint(at), cfg, seed));
        self.cev.push(Vec::new());
        self.clients.len() - 1
    }

    pub fn connect(&mut self, i: usize) {
        let now = self.now();
        self.clients[i].connect(server_addr(), b"name=tester", now).unwrap();
    }

    /// One step: advance, receive, handle handshakes, flush.
    pub fn step(&mut self, dt: Duration) {
        let now = self.net.advance(dt);
        self.server.update(now);
        for e in self.server.drain_events() {
            if let ServerEvent::ConnectRequest { pending, .. } = &e {
                if let Some(r) = &self.auto_reject {
                    let r = r.clone();
                    self.server.reject(*pending, &r, now);
                } else if self.auto_accept {
                    self.server.accept(*pending, now);
                }
            }
            self.sev.push(e);
        }
        for e in self.server.drain_events() {
            self.sev.push(e);
        }
        for (i, c) in self.clients.iter_mut().enumerate() {
            c.update(now);
            self.cev[i].extend(c.drain_events());
        }
        self.server.flush(now);
        for c in &mut self.clients {
            c.flush(now);
        }
    }

    pub fn run(&mut self, d: Duration) {
        let end = self.now() + d;
        while self.now() < end {
            self.step(TICK);
        }
    }

    /// Steps until `f` holds (or panics after `limit`).
    pub fn run_until(&mut self, limit: Duration, mut f: impl FnMut(&Sim) -> bool) {
        let end = self.now() + limit;
        while !f(self) {
            assert!(self.now() < end, "condition not reached within {limit:?}");
            self.step(TICK);
        }
    }

    pub fn server_conns(&self) -> Vec<ConnId> {
        let mut v: Vec<_> = self.server.connections().collect();
        v.sort();
        v
    }

    pub fn connect_all(&mut self) {
        for i in 0..self.clients.len() {
            self.connect(i);
        }
        let n = self.clients.len();
        self.run_until(Duration::from_secs(5), |s| s.clients.iter().all(|c| c.is_connected()) && s.server.connection_count() == n);
    }

    pub fn server_messages(&mut self, conn: ConnId, lane: Lane) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        self.sev.retain(|e| match e {
            ServerEvent::Message { conn: c, lane: l, data } if *c == conn && *l == lane => {
                out.push(data.clone());
                false
            }
            _ => true,
        });
        out
    }

    pub fn client_messages(&mut self, i: usize, lane: Lane) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        self.cev[i].retain(|e| match e {
            ClientEvent::Message { lane: l, data } if *l == lane => {
                out.push(data.clone());
                false
            }
            _ => true,
        });
        out
    }

    pub fn client_disconnect_reason(&self, i: usize) -> Option<DisconnectReason> {
        self.cev[i].iter().find_map(|e| match e {
            ClientEvent::Disconnected(r) => Some(r.clone()),
            _ => None,
        })
    }

    pub fn server_disconnects(&self) -> Vec<(ConnId, DisconnectReason)> {
        self.sev
            .iter()
            .filter_map(|e| match e {
                ServerEvent::Disconnected { conn, reason, .. } => Some((*conn, reason.clone())),
                _ => None,
            })
            .collect()
    }
}

/// Deterministic message payload `k` of length 1..=`max_len`.
pub fn payload(k: u32, max_len: u32) -> Vec<u8> {
    let mut x = k.wrapping_mul(2_654_435_761).wrapping_add(12345);
    let len = 1 + (x % max_len) as usize;
    let mut v = Vec::with_capacity(len + 4);
    v.extend_from_slice(&k.to_le_bytes());
    while v.len() < len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        v.push(x as u8);
    }
    v
}

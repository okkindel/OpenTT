//! The master server (docs/netcode.md §5.11; protocol in
//! [`tt_net::master`]). This one is small enough to run anywhere (one UDP
//! socket, no state on disk):
//!
//! * game servers register with heartbeats (address proven by a cookie,
//!   [`ENTRY_LIFETIME`](tt_net::master::ENTRY_LIFETIME) without one drops
//!   them, a closing heartbeat drops them at once);
//! * browsers fetch the list in pages and ask the servers for their info
//!   themselves;
//! * clients ask for an introduction to a listed server, which the master
//!   forwards so the server punches a hole towards the client.
//!
//! Limits against abuse: a token bucket per source IP for all packets,
//! at most [`Limits::servers_per_ip`] registrations per IP and
//! [`Limits::max_servers`] in total, introductions per target server per
//! second, and every reply no larger than its request.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tt_net::master::{MasterPacket, ENTRY_LIFETIME, LIST_PAGE, MASTER_VERSION};
use tt_net::rng::{siphash24, Rng};
use tt_net::Datagram;

/// Abuse limits.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Packets per second per source IP (token bucket, burst = rate).
    pub packets_per_ip: f64,
    pub servers_per_ip: usize,
    pub max_servers: usize,
    /// Introductions forwarded per target server per second.
    pub introductions_per_server: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { packets_per_ip: 40.0, servers_per_ip: 16, max_servers: 4096, introductions_per_server: 10.0 }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    server_id: u64,
    last_seen: Duration,
    /// Introduction budget: (tokens, last refill).
    intro: (f64, Duration),
}

/// Counters (printed by the binary).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MasterStats {
    pub heartbeats: u64,
    pub challenges: u64,
    pub registrations: u64,
    pub list_requests: u64,
    pub introductions: u64,
    pub dropped: u64,
    pub malformed: u64,
}

/// The master server state on one socket.
pub struct Master<T: Datagram> {
    transport: T,
    key: (u64, u64),
    limits: Limits,
    servers: HashMap<SocketAddr, Entry>,
    flood: HashMap<IpAddr, (f64, Duration)>,
    pub stats: MasterStats,
}

/// Cookie validity bucket (a cookie is accepted in its bucket and the next).
const COOKIE_BUCKET: u64 = 30;

impl<T: Datagram> Master<T> {
    pub fn new(transport: T, limits: Limits) -> Self {
        let mut rng = Rng::from_entropy();
        Master { transport, key: (rng.next_u64(), rng.next_u64()), limits, servers: HashMap::new(), flood: HashMap::new(), stats: MasterStats::default() }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.transport.local_addr()
    }

    /// Registered servers, sorted (tests, the status line).
    pub fn servers(&self) -> Vec<SocketAddr> {
        let mut v: Vec<SocketAddr> = self.servers.keys().copied().collect();
        v.sort();
        v
    }

    fn cookie(&self, addr: SocketAddr, bucket: u64) -> u64 {
        let mut b = Vec::with_capacity(26);
        match addr.ip() {
            IpAddr::V4(v) => b.extend_from_slice(&v.to_ipv6_mapped().octets()),
            IpAddr::V6(v) => b.extend_from_slice(&v.octets()),
        }
        b.extend_from_slice(&addr.port().to_le_bytes());
        b.extend_from_slice(&bucket.to_le_bytes());
        siphash24(self.key.0, self.key.1, &b) | 1
    }

    fn cookie_ok(&self, addr: SocketAddr, cookie: u64, now: Duration) -> bool {
        let b = now.as_secs() / COOKIE_BUCKET;
        cookie == self.cookie(addr, b) || (b > 0 && cookie == self.cookie(addr, b - 1))
    }

    fn allow(&mut self, ip: IpAddr, now: Duration) -> bool {
        let rate = self.limits.packets_per_ip;
        let e = self.flood.entry(ip).or_insert((rate, now));
        e.0 = (e.0 + now.saturating_sub(e.1).as_secs_f64() * rate).min(rate);
        e.1 = now;
        if e.0 >= 1.0 {
            e.0 -= 1.0;
            true
        } else {
            false
        }
    }

    fn send(&mut self, p: &MasterPacket, to: SocketAddr) {
        let _ = self.transport.send_to(&p.encode(), to);
    }

    /// Receives and answers everything pending; expires old entries.
    pub fn update(&mut self, now: Duration) {
        let mut buf = [0u8; tt_net::MAX_PACKET_SIZE + 1];
        while let Some((n, from)) = self.transport.recv_from(&mut buf) {
            if !self.allow(from.ip(), now) {
                self.stats.dropped += 1;
                continue;
            }
            match MasterPacket::decode(&buf[..n]) {
                Ok(p) => self.handle(p, from, now),
                Err(_) => self.stats.malformed += 1,
            }
        }
        self.servers.retain(|_, e| now.saturating_sub(e.last_seen) <= ENTRY_LIFETIME);
        self.flood.retain(|_, (_, t)| now.saturating_sub(*t) < Duration::from_secs(10));
    }

    fn handle(&mut self, p: MasterPacket, from: SocketAddr, now: Duration) {
        match p {
            MasterPacket::Heartbeat { version, nonce, cookie, server_id, closing } => {
                if version != MASTER_VERSION {
                    return;
                }
                self.stats.heartbeats += 1;
                if !self.cookie_ok(from, cookie, now) {
                    self.stats.challenges += 1;
                    let cookie = self.cookie(from, now.as_secs() / COOKIE_BUCKET);
                    return self.send(&MasterPacket::HeartbeatChallenge { nonce, cookie }, from);
                }
                if closing {
                    self.servers.remove(&from);
                    return;
                }
                if !self.servers.contains_key(&from) {
                    let same_ip = self.servers.keys().filter(|a| a.ip() == from.ip()).count();
                    if same_ip >= self.limits.servers_per_ip || self.servers.len() >= self.limits.max_servers {
                        self.stats.dropped += 1;
                        return;
                    }
                    self.stats.registrations += 1;
                }
                let rate = self.limits.introductions_per_server;
                let e = self.servers.entry(from).or_insert(Entry { server_id, last_seen: now, intro: (rate, now) });
                e.server_id = server_id;
                e.last_seen = now;
                self.send(&MasterPacket::HeartbeatAck { nonce, public: from }, from);
            }
            MasterPacket::ListRequest { version, nonce, page } => {
                if version != MASTER_VERSION {
                    return;
                }
                self.stats.list_requests += 1;
                let all = self.servers();
                let pages = all.len().div_ceil(LIST_PAGE).clamp(1, 255);
                let page = (page as usize).min(pages - 1);
                let servers: Vec<SocketAddr> = all.iter().skip(page * LIST_PAGE).take(LIST_PAGE).copied().collect();
                self.send(&MasterPacket::ListReply { nonce, page: page as u8, pages: pages as u8, servers }, from);
            }
            MasterPacket::IntroduceRequest { version, server, .. } => {
                if version != MASTER_VERSION {
                    return;
                }
                let rate = self.limits.introductions_per_server;
                let Some(e) = self.servers.get_mut(&server) else { return };
                let tokens = (e.intro.0 + now.saturating_sub(e.intro.1).as_secs_f64() * rate).min(rate);
                if tokens < 1.0 {
                    e.intro = (tokens, now);
                    self.stats.dropped += 1;
                    return;
                }
                e.intro = (tokens - 1.0, now);
                self.stats.introductions += 1;
                self.send(&MasterPacket::Introduce { client: from }, server);
            }
            // Replies and punches are not for us.
            _ => {}
        }
    }
}

/// Command-line options of the binary (`tt-master`, `opentt --master`).
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub port: u16,
    /// Print a status line this often (0 = never).
    pub status_every: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options { port: tt_net::master::DEFAULT_MASTER_PORT, status_every: Duration::from_secs(60) }
    }
}

pub const USAGE: &str = "usage: tt-master [--port N] [--status SECONDS]   (default port 28100)";

/// Parses `--port N`, `--status SECONDS`.
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut num = |what: &str| -> Result<u64, String> { it.next().and_then(|v| v.parse().ok()).ok_or_else(|| format!("{what} needs a number\n{USAGE}")) };
        match a.as_str() {
            "--port" => o.port = num("--port")?.clamp(1, 65535) as u16,
            "--status" => o.status_every = Duration::from_secs(num("--status")?),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(o)
}

/// Runs a master server on a real UDP socket until the process is killed.
pub fn run(o: &Options) -> std::io::Result<()> {
    let t = tt_net::UdpTransport::bind_any(o.port)?;
    let addr = t.local_addr();
    let mut m = Master::new(t, Limits::default());
    println!("{} master server on UDP {addr}", tt_net::PRODUCT_NAME);
    let start = std::time::Instant::now();
    let mut last_status = Duration::ZERO;
    loop {
        let now = start.elapsed();
        m.update(now);
        if !o.status_every.is_zero() && now.saturating_sub(last_status) >= o.status_every {
            last_status = now;
            let s = &m.stats;
            println!("[master] {} servers; {} heartbeats, {} registrations, {} lists, {} introductions, {} dropped, {} malformed", m.servers.len(), s.heartbeats, s.registrations, s.list_requests, s.introductions, s.dropped, s.malformed);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

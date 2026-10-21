//! The master server. This one is small enough to run anywhere (one UDP
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
//!
//! With `--stats-port` the master also asks the listed servers for their
//! info ([`Master::query_info`]) and serves `GET /stats` (JSON, optionally
//! behind HTTP Basic auth) on that TCP port ([`spawn_stats`]).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tt_net::master::{MasterPacket, ENTRY_LIFETIME, LIST_PAGE, MASTER_VERSION};
use tt_net::packet::Packet;
use tt_net::rng::{siphash24, Rng};
use tt_net::{Datagram, ServerInfo};

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
    /// The real source address (replies, introductions); the key is the listed one.
    reply: SocketAddr,
    last_seen: Duration,
    /// Introduction budget: (tokens, last refill).
    intro: (f64, Duration),
    /// The last info query (nonce, sent) and the server's reply (received).
    query: Option<(u64, Duration)>,
    info: Option<(ServerInfo, Duration)>,
}

/// With [`Master::query_info`], each listed server is asked for its info this often.
pub const INFO_INTERVAL: Duration = Duration::from_secs(30);
/// An info reply older than this is dropped (the server stopped answering).
const INFO_LIFETIME: Duration = Duration::from_secs(90);
/// Info queries per second at most (burst = a tenth): a long list is spread out.
const INFO_QUERY_RATE: f64 = 200.0;

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
    pub info_queries: u64,
    pub info_replies: u64,
}

/// The master server state on one socket.
pub struct Master<T: Datagram> {
    transport: T,
    key: (u64, u64),
    limits: Limits,
    servers: HashMap<SocketAddr, Entry>,
    flood: HashMap<IpAddr, (f64, Duration)>,
    pub stats: MasterStats,
    /// List changes not yet printed (servers added, restarted, dropped).
    log: Vec<String>,
    /// `--local-address`: lists servers registering from a non-public
    /// source (same host, Docker bridge) under this IP and their port.
    pub local_address: Option<IpAddr>,
    /// Ask the listed servers for their info every [`INFO_INTERVAL`] (for
    /// [`Master::stats_json`]); off by default.
    pub query_info: bool,
    /// Info query budget: (tokens, last refill).
    info_budget: (f64, Duration),
    nonces: Rng,
}

/// Cookie validity bucket (a cookie is accepted in its bucket and the next).
const COOKIE_BUCKET: u64 = 30;

impl<T: Datagram> Master<T> {
    pub fn new(transport: T, limits: Limits) -> Self {
        let mut rng = Rng::from_entropy();
        let key = (rng.next_u64(), rng.next_u64());
        Master { transport, key, limits, servers: HashMap::new(), flood: HashMap::new(), stats: MasterStats::default(), log: Vec::new(), local_address: None, query_info: false, info_budget: (0.0, Duration::ZERO), nonces: rng }
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

    /// The list changes since the last call (one line each, no timestamp).
    pub fn drain_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// The address a server registering from `from` is listed under.
    fn listed(&self, from: SocketAddr) -> SocketAddr {
        match self.local_address {
            Some(ip) if !tt_net::master::is_public(from.ip()) => SocketAddr::new(ip, from.port()),
            _ => from,
        }
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
                Err(_) if self.info_reply(&buf[..n], from, now) => {}
                Err(_) => self.stats.malformed += 1,
            }
        }
        let log = &mut self.log;
        self.servers.retain(|a, e| {
            let keep = now.saturating_sub(e.last_seen) <= ENTRY_LIFETIME;
            if !keep {
                log.push(format!("dropped {a} (id {:016x}): no heartbeat for {}s", e.server_id, ENTRY_LIFETIME.as_secs()));
            }
            e.info = e.info.take().filter(|(_, t)| now.saturating_sub(*t) <= INFO_LIFETIME);
            keep
        });
        self.flood.retain(|_, (_, t)| now.saturating_sub(*t) < Duration::from_secs(10));
        if self.query_info {
            self.send_info_queries(now);
        }
    }

    /// The game browser's `InfoQuery` to the servers due, at most the
    /// budget's worth, at their real source address (its NAT mapping is
    /// open towards the master).
    fn send_info_queries(&mut self, now: Duration) {
        let b = &mut self.info_budget;
        b.0 = (b.0 + now.saturating_sub(b.1).as_secs_f64() * INFO_QUERY_RATE).min(INFO_QUERY_RATE / 10.0);
        b.1 = now;
        if b.0 < 1.0 {
            return;
        }
        let due = |e: &Entry| e.query.is_none_or(|(_, t)| now.saturating_sub(t) >= INFO_INTERVAL);
        let mut sent = Vec::new();
        for e in self.servers.values_mut().filter(|e| due(e)).take(self.info_budget.0 as usize) {
            let nonce = self.nonces.next_u64();
            e.query = Some((nonce, now));
            sent.push((nonce, e.reply));
        }
        self.info_budget.0 -= sent.len() as f64;
        self.stats.info_queries += sent.len() as u64;
        for (nonce, to) in sent {
            let _ = self.transport.send_to(&Packet::InfoQuery { protocol: tt_net::PROTOCOL_VERSION, nonce }.encode(), to);
        }
    }

    /// An `Info` reply to one of our queries: kept with the server's entry.
    fn info_reply(&mut self, b: &[u8], from: SocketAddr, now: Duration) -> bool {
        let Ok(Packet::Info { nonce, payload, .. }) = Packet::decode(b) else { return false };
        let Some(e) = self.servers.values_mut().find(|e| e.reply == from && e.query.is_some_and(|(n, _)| n == nonce)) else { return false };
        let Ok(info) = ServerInfo::decode(&payload) else { return false };
        e.info = Some((info, now));
        self.stats.info_replies += 1;
        true
    }

    /// `GET /stats`: the master's version, uptime (`now`), counters and the
    /// listed servers with their last info reply (`null` before one).
    pub fn stats_json(&self, now: Duration) -> String {
        let s = &self.stats;
        let mut out = format!(
            "{{\"master\":{{\"product\":{},\"version\":{},\"uptime\":{}}},\"stats\":{{\"servers\":{},\"heartbeats\":{},\"challenges\":{},\"registrations\":{},\"list_requests\":{},\"introductions\":{},\"dropped\":{},\"malformed\":{},\"info_queries\":{},\"info_replies\":{}}},\"servers\":[",
            json_str(tt_net::PRODUCT_NAME),
            json_str(env!("CARGO_PKG_VERSION")),
            now.as_secs(),
            self.servers.len(),
            s.heartbeats,
            s.challenges,
            s.registrations,
            s.list_requests,
            s.introductions,
            s.dropped,
            s.malformed,
            s.info_queries,
            s.info_replies
        );
        for (i, a) in self.servers().iter().enumerate() {
            let e = &self.servers[a];
            let info = match &e.info {
                None => "null".to_string(),
                Some((f, t)) => format!(
                    "{{\"name\":{},\"game_type\":{},\"mission_type\":{},\"mission\":{},\"players\":{},\"bots\":{},\"humans\":{},\"max_players\":{},\"password\":{},\"dedicated\":{},\"version\":{},\"build\":\"{:016x}\",\"age\":{}}}",
                    json_str(&f.name),
                    json_str(&f.game_type),
                    json_str(&f.mission_type),
                    json_str(f.map()),
                    f.player_count,
                    f.bot_count,
                    f.player_count.saturating_sub(f.bot_count),
                    f.max_players,
                    f.password,
                    f.dedicated,
                    json_str(&f.version),
                    f.build,
                    now.saturating_sub(*t).as_secs()
                ),
            };
            let comma = if i > 0 { "," } else { "" };
            out += &format!("{comma}{{\"address\":\"{a}\",\"last_seen\":{},\"info\":{info}}}", now.saturating_sub(e.last_seen).as_secs());
        }
        out + "]}"
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
                let listed = self.listed(from);
                if closing {
                    if let Some(e) = self.servers.remove(&listed) {
                        self.log.push(format!("dropped {listed} (id {:016x}): server closed", e.server_id));
                    }
                    return;
                }
                if !self.servers.contains_key(&listed) {
                    let same_ip = self.servers.keys().filter(|a| a.ip() == listed.ip()).count();
                    if same_ip >= self.limits.servers_per_ip || self.servers.len() >= self.limits.max_servers {
                        self.stats.dropped += 1;
                        return;
                    }
                    self.stats.registrations += 1;
                    self.log.push(format!("added {listed} (id {server_id:016x})"));
                } else if self.servers[&listed].server_id != server_id {
                    self.log.push(format!("restarted {listed} (id {server_id:016x})"));
                }
                let rate = self.limits.introductions_per_server;
                let e = self.servers.entry(listed).or_insert(Entry { server_id, reply: from, last_seen: now, intro: (rate, now), query: None, info: None });
                e.server_id = server_id;
                e.last_seen = now;
                e.reply = from;
                self.send(&MasterPacket::HeartbeatAck { nonce, public: listed }, from);
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
                let to = e.reply;
                self.stats.introductions += 1;
                self.send(&MasterPacket::Introduce { client: from }, to);
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
    /// `--local-address IP`: see [`Master::local_address`].
    pub local_address: Option<IpAddr>,
    /// `--stats-port N`: serve `GET /stats` on this TCP port.
    pub stats_port: Option<u16>,
    /// `--stats-user NAME`: `/stats` needs HTTP Basic auth as this user,
    /// with the password from [`STATS_PASSWORD_ENV`].
    pub stats_user: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options { port: tt_net::master::DEFAULT_MASTER_PORT, status_every: Duration::from_secs(60), local_address: None, stats_port: None, stats_user: None }
    }
}

/// The environment variable with the `/stats` password (`--stats-user`).
pub const STATS_PASSWORD_ENV: &str = "OPENTT_MASTER_STATS_PASSWORD";

pub const USAGE: &str = "usage: tt-master [--port N] [--status SECONDS] [--local-address IP] [--stats-port N [--stats-user NAME]]   (default port 28100)
  --stats-port N     serve GET /stats (JSON) on TCP port N
  --stats-user NAME  require HTTP Basic auth as NAME, password from OPENTT_MASTER_STATS_PASSWORD";

/// Parses `--port N`, `--status SECONDS`, `--local-address IP`,
/// `--stats-port N`, `--stats-user NAME`.
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut num = |what: &str| -> Result<u64, String> { it.next().and_then(|v| v.parse().ok()).ok_or_else(|| format!("{what} needs a number\n{USAGE}")) };
        match a.as_str() {
            "--port" => o.port = num("--port")?.clamp(1, 65535) as u16,
            "--status" => o.status_every = Duration::from_secs(num("--status")?),
            "--stats-port" => o.stats_port = Some(num("--stats-port")?.clamp(1, 65535) as u16),
            "--local-address" => o.local_address = Some(it.next().and_then(|v| v.parse().ok()).ok_or_else(|| format!("--local-address needs an IP address\n{USAGE}"))?),
            "--stats-user" => o.stats_user = Some(it.next().filter(|v| !v.is_empty() && !v.contains(':')).ok_or_else(|| format!("--stats-user needs a name (without ':')\n{USAGE}"))?.clone()),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    if o.stats_user.is_some() && o.stats_port.is_none() {
        return Err(format!("--stats-user needs --stats-port\n{USAGE}"));
    }
    Ok(o)
}

/// Connections `/stats` serves at once (more are closed at once).
const STATS_CONNECTIONS: usize = 8;
/// A request's header is at most this long.
const STATS_REQUEST_MAX: usize = 8192;

/// Serves `GET /stats` with `page` (the JSON) on `listener` from a thread,
/// each connection on its own (at most [`STATS_CONNECTIONS`]): HTTP/1.1,
/// one response per connection. With `auth` (user, password) a request
/// needs HTTP Basic auth.
pub fn spawn_stats(listener: TcpListener, auth: Option<(String, String)>, page: Arc<Mutex<String>>) {
    let token = auth.map(|(u, p)| base64(format!("{u}:{p}").as_bytes()));
    let open = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            if open.fetch_add(1, Ordering::SeqCst) >= STATS_CONNECTIONS {
                open.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let (token, page, open) = (token.clone(), page.clone(), open.clone());
            std::thread::spawn(move || {
                let _ = stats_reply(s, token.as_deref(), &page);
                open.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
}

/// Reads one request (header only, within 3 s and [`STATS_REQUEST_MAX`]) and answers it.
fn stats_reply(mut s: TcpStream, token: Option<&str>, page: &Mutex<String>) -> std::io::Result<()> {
    let start = Instant::now();
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut req = Vec::new();
    let mut buf = [0u8; 2048];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
        if req.len() > STATS_REQUEST_MAX || start.elapsed() > Duration::from_secs(3) {
            return Ok(());
        }
        let n = s.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        req.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&req);
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let authorized = token.is_none_or(|want| {
        lines.filter_map(|l| l.split_once(':')).any(|(k, v)| {
            k.trim().eq_ignore_ascii_case("authorization") && v.trim().split_once(' ').is_some_and(|(scheme, t)| scheme.eq_ignore_ascii_case("basic") && same_bytes(t.trim().as_bytes(), want.as_bytes()))
        })
    });
    let (status, extra, body) = if method != "GET" {
        ("405 Method Not Allowed", "Allow: GET\r\n".to_string(), "{\"error\":\"method not allowed\"}".to_string())
    } else if !authorized {
        ("401 Unauthorized", format!("WWW-Authenticate: Basic realm=\"{} master\"\r\n", tt_net::PRODUCT_NAME), "{\"error\":\"unauthorized\"}".to_string())
    } else if path.split('?').next() != Some("/stats") {
        ("404 Not Found", String::new(), "{\"error\":\"not found\"}".to_string())
    } else {
        ("200 OK", String::new(), page.lock().map(|p| p.clone()).unwrap_or_default())
    };
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n", body.len());
    s.write_all(head.as_bytes())?;
    s.write_all(body.as_bytes())?;
    s.flush()
}

/// Constant-time equality (for equal lengths; the length is not secret).
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Standard base64 with padding.
fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A JSON string literal.
fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out + "\""
}

/// TCP listeners on `port` like [`tt_net::UdpTransport::bind_any`]: `[::]`
/// and, where that is IPv6-only, `0.0.0.0`.
fn bind_tcp(port: u16) -> std::io::Result<Vec<TcpListener>> {
    let v6 = TcpListener::bind((std::net::Ipv6Addr::UNSPECIFIED, port));
    let v4 = TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port));
    match (v6, v4) {
        (Err(e), Err(_)) => Err(e),
        (a, b) => Ok(a.into_iter().chain(b).collect()),
    }
}

/// Runs a master server on a real UDP socket until the process is killed.
pub fn run(o: &Options) -> std::io::Result<()> {
    let auth = match &o.stats_user {
        Some(u) => match std::env::var(STATS_PASSWORD_ENV) {
            Ok(p) if !p.is_empty() => Some((u.clone(), p)),
            _ => return Err(std::io::Error::other(format!("--stats-user needs the password in {STATS_PASSWORD_ENV}"))),
        },
        None => None,
    };
    let t = tt_net::UdpTransport::bind_any(o.port)?;
    let addr = t.local_addr();
    let mut m = Master::new(t, Limits::default());
    m.local_address = o.local_address;
    println!("{} master server on UDP {addr}", tt_net::PRODUCT_NAME);
    // The first page before the listeners start (never an empty body).
    let page = Arc::new(Mutex::new(m.stats_json(Duration::ZERO)));
    if let Some(port) = o.stats_port {
        let ls = bind_tcp(port)?;
        let at: Vec<String> = ls.iter().filter_map(|l| l.local_addr().ok()).map(|a| a.to_string()).collect();
        println!("stats on http://{}/stats ({})", at.join(", http://"), if auth.is_some() { "Basic auth" } else { "open" });
        for l in ls {
            spawn_stats(l, auth.clone(), page.clone());
        }
        m.query_info = true;
    }
    let start = std::time::Instant::now();
    let stamp = || utc_stamp(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()));
    let mut last_status = Duration::ZERO;
    let mut last_page = None;
    loop {
        let now = start.elapsed();
        m.update(now);
        if m.query_info && last_page.is_none_or(|t: Duration| now.saturating_sub(t) >= Duration::from_secs(1)) {
            last_page = Some(now);
            let json = m.stats_json(now);
            if let Ok(mut p) = page.lock() {
                *p = json;
            }
        }
        for l in m.drain_log() {
            println!("{} [master] {l}", stamp());
        }
        if !o.status_every.is_zero() && now.saturating_sub(last_status) >= o.status_every {
            last_status = now;
            let s = &m.stats;
            println!("{} [master] {} servers; {} heartbeats, {} registrations, {} lists, {} introductions, {} dropped, {} malformed", stamp(), m.servers.len(), s.heartbeats, s.registrations, s.list_requests, s.introductions, s.dropped, s.malformed);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `YYYY-MM-DD HH:MM:SS UTC` of a Unix time (H. Hinnant's days-to-civil).
pub fn utc_stamp(secs: u64) -> String {
    let (days, rem) = ((secs / 86400) as i64 + 719_468, secs % 86400);
    let (era, doe) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rem / 3600, rem % 3600 / 60, rem % 60)
}

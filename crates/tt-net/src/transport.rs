//! Datagram transports (docs/netcode.md §3.1): the [`Datagram`] trait, the
//! real [`UdpTransport`] and the in-memory [`MemNet`]/[`MemLink`] with a
//! deterministic [`LinkConditions`] simulator on a virtual clock.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::rng::Rng;

/// An unreliable datagram socket.
pub trait Datagram {
    /// Sends one datagram. Errors are reported but endpoints ignore them
    /// (UDP is lossy anyway).
    fn send_to(&mut self, data: &[u8], addr: SocketAddr) -> io::Result<()>;
    /// Receives one pending datagram without blocking. Datagrams longer than
    /// `buf` are truncated (as with UDP).
    fn recv_from(&mut self, buf: &mut [u8]) -> Option<(usize, SocketAddr)>;
    fn local_addr(&self) -> SocketAddr;
}

impl<T: Datagram + ?Sized> Datagram for Box<T> {
    fn send_to(&mut self, data: &[u8], addr: SocketAddr) -> io::Result<()> {
        (**self).send_to(data, addr)
    }
    fn recv_from(&mut self, buf: &mut [u8]) -> Option<(usize, SocketAddr)> {
        (**self).recv_from(buf)
    }
    fn local_addr(&self) -> SocketAddr {
        (**self).local_addr()
    }
}

// ---------------------------------------------------------------------------
// UDP

/// Non-blocking std UDP socket(s).
///
/// [`bind_any`](Self::bind_any) binds `[::]:port` and, where that socket is
/// IPv6-only (Windows default), also `0.0.0.0:port`; std cannot clear
/// `IPV6_V6ONLY` without extra dependencies, so two sockets give dual stack
/// everywhere. IPv4-mapped peer addresses are reported as plain IPv4.
#[derive(Debug)]
pub struct UdpTransport {
    v4: Option<UdpSocket>,
    v6: Option<UdpSocket>,
    local: SocketAddr,
}

fn normalize(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V6(a) => match a.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(IpAddr::V4(v4), a.port()),
            None => addr,
        },
        v4 => v4,
    }
}

impl UdpTransport {
    /// Binds exactly `addr` (port 0 = ephemeral).
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {
        let s = UdpSocket::bind(addr)?;
        s.set_nonblocking(true)?;
        let local = s.local_addr()?;
        Ok(if addr.is_ipv4() {
            UdpTransport { v4: Some(s), v6: None, local }
        } else {
            UdpTransport { v4: None, v6: Some(s), local }
        })
    }

    /// Binds all interfaces on `port` (0 = ephemeral), dual stack when possible.
    pub fn bind_any(port: u16) -> io::Result<Self> {
        let v6 = UdpSocket::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)).ok();
        let port = match &v6 {
            Some(s) => s.local_addr()?.port(),
            None => port,
        };
        // Fails on Linux/macOS where `[::]` is already dual stack; that is fine.
        let v4 = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port));
        let v4 = match (v4, &v6) {
            (Ok(s), _) => Some(s),
            (Err(_), Some(_)) => None,
            (Err(e), None) => return Err(e),
        };
        for s in v4.iter().chain(v6.iter()) {
            s.set_nonblocking(true)?;
        }
        let local = match (&v4, &v6) {
            (Some(s), _) | (None, Some(s)) => s.local_addr()?,
            (None, None) => unreachable!("one socket is bound"),
        };
        Ok(UdpTransport { v4, v6, local })
    }

    /// Enables sending to broadcast addresses (LAN discovery).
    pub fn set_broadcast(&self, on: bool) -> io::Result<()> {
        if let Some(s) = &self.v4 {
            s.set_broadcast(on)?;
        }
        Ok(())
    }
}

impl Datagram for UdpTransport {
    fn send_to(&mut self, data: &[u8], addr: SocketAddr) -> io::Result<()> {
        match (addr, &self.v4, &self.v6) {
            (SocketAddr::V4(_), Some(s), _) => s.send_to(data, addr).map(|_| ()),
            (SocketAddr::V4(a), None, Some(s)) => {
                let mapped = SocketAddr::new(IpAddr::V6(a.ip().to_ipv6_mapped()), a.port());
                s.send_to(data, mapped).map(|_| ())
            }
            (SocketAddr::V6(_), _, Some(s)) => s.send_to(data, addr).map(|_| ()),
            _ => Err(io::Error::new(io::ErrorKind::Unsupported, "no socket for this address family")),
        }
    }

    fn recv_from(&mut self, buf: &mut [u8]) -> Option<(usize, SocketAddr)> {
        for s in self.v4.iter().chain(self.v6.iter()) {
            // Errors other than WouldBlock (e.g. Windows ICMP "connection
            // reset") are per datagram: skip them and keep reading.
            for _ in 0..64 {
                match s.recv_from(buf) {
                    Ok((n, from)) => return Some((n, normalize(from))),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => continue,
                }
            }
        }
        None
    }

    fn local_addr(&self) -> SocketAddr {
        self.local
    }
}

// ---------------------------------------------------------------------------
// In-memory network + link simulator

/// Conditions of one direction of a link (all zero = perfect, instant).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkConditions {
    /// One-way delay.
    pub latency: Duration,
    /// Uniform extra delay in `[-jitter, +jitter]` (clamped at 0 total).
    /// Jitter alone never reorders (packets queue behind earlier ones).
    pub jitter: Duration,
    /// Drop probability.
    pub loss: f64,
    /// Probability a packet is delivered twice.
    pub duplicate: f64,
    /// Probability a packet is held back by `reorder_delay`, letting later
    /// packets overtake it.
    pub reorder: f64,
    pub reorder_delay: Duration,
    /// Bytes per second; `None` = unlimited.
    pub bandwidth: Option<u64>,
    /// With a bandwidth cap: packets that would queue longer than this are dropped.
    pub queue_limit: Duration,
}

impl LinkConditions {
    /// LAN: 1 ms.
    pub fn lan() -> Self {
        LinkConditions { latency: Duration::from_millis(1), ..Default::default() }
    }
    /// DSL: 60 ms ±5, 0.5 % loss.
    pub fn dsl() -> Self {
        LinkConditions {
            latency: Duration::from_millis(60),
            jitter: Duration::from_millis(5),
            loss: 0.005,
            ..Default::default()
        }
    }
    /// Bad wifi: 150 ms ±30, 3 % loss, 1 % reorder.
    pub fn bad_wifi() -> Self {
        LinkConditions {
            latency: Duration::from_millis(150),
            jitter: Duration::from_millis(30),
            loss: 0.03,
            reorder: 0.01,
            reorder_delay: Duration::from_millis(40),
            ..Default::default()
        }
    }
    /// Awful: 300 ms ±80, 10 % loss.
    pub fn awful() -> Self {
        LinkConditions {
            latency: Duration::from_millis(300),
            jitter: Duration::from_millis(80),
            loss: 0.10,
            ..Default::default()
        }
    }
}

/// Counters of one link direction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub sent: u64,
    pub delivered: u64,
    pub lost: u64,
    pub duplicated: u64,
    pub reordered: u64,
    pub queue_dropped: u64,
    pub bytes_sent: u64,
}

#[derive(Debug, Default)]
struct LinkState {
    cond: Option<LinkConditions>,
    busy_until: Duration,
    last_arrival: Duration,
    stats: LinkStats,
}

type Queued = Reverse<(Duration, u64, SocketAddr, Vec<u8>)>;

#[derive(Debug)]
struct NetInner {
    now: Duration,
    rng: Rng,
    default: LinkConditions,
    links: HashMap<(SocketAddr, SocketAddr), LinkState>,
    inboxes: HashMap<SocketAddr, BinaryHeap<Queued>>,
    counter: u64,
    capture: Option<Vec<(SocketAddr, SocketAddr, Vec<u8>)>>,
}

/// A deterministic in-process network: every [`MemLink`] created from it
/// is an endpoint with its own address. Datagrams to `255.255.255.255:p`
/// (or `a.b.c.255:p`) reach every endpoint on port `p` (in that /24),
/// like a LAN broadcast. Time only moves through
/// [`advance`](Self::advance)/[`set_now`](Self::set_now); all randomness
/// comes from the seed. Cloning shares the network.
#[derive(Debug, Clone)]
pub struct MemNet {
    inner: Arc<Mutex<NetInner>>,
}

impl MemNet {
    pub fn new(seed: u64) -> Self {
        MemNet {
            inner: Arc::new(Mutex::new(NetInner {
                now: Duration::ZERO,
                rng: Rng::new(seed),
                default: LinkConditions::default(),
                links: HashMap::new(),
                inboxes: HashMap::new(),
                counter: 0,
                capture: None,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, NetInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Creates an endpoint at `addr`.
    pub fn endpoint(&self, addr: SocketAddr) -> MemLink {
        self.lock().inboxes.entry(addr).or_default();
        MemLink { net: self.clone(), addr }
    }

    /// Conditions for every link without its own.
    pub fn set_conditions(&self, c: LinkConditions) {
        self.lock().default = c;
    }

    /// Conditions for the `from → to` direction.
    pub fn set_link(&self, from: SocketAddr, to: SocketAddr, c: LinkConditions) {
        self.lock().links.entry((from, to)).or_default().cond = Some(c);
    }

    pub fn now(&self) -> Duration {
        self.lock().now
    }

    pub fn set_now(&self, t: Duration) {
        self.lock().now = t;
    }

    pub fn advance(&self, dt: Duration) -> Duration {
        let mut n = self.lock();
        n.now += dt;
        n.now
    }

    pub fn link_stats(&self, from: SocketAddr, to: SocketAddr) -> LinkStats {
        self.lock().links.get(&(from, to)).map(|l| l.stats.clone()).unwrap_or_default()
    }

    /// Starts/stops recording every submitted datagram (for fuzz corpora).
    pub fn set_capture(&self, on: bool) {
        self.lock().capture = if on { Some(Vec::new()) } else { None };
    }

    pub fn take_captured(&self) -> Vec<(SocketAddr, SocketAddr, Vec<u8>)> {
        self.lock().capture.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Sends a datagram with an arbitrary (spoofed) source address.
    pub fn inject(&self, from: SocketAddr, to: SocketAddr, data: &[u8]) {
        self.lock().submit(from, to, data);
    }
}

/// Is `ep` reached by a datagram to the broadcast address `to`
/// (255.255.255.255, or the /24 directed broadcast `a.b.c.255`)?
fn broadcast_reaches(to: SocketAddr, ep: SocketAddr) -> bool {
    match (to.ip(), ep.ip()) {
        (IpAddr::V4(t), IpAddr::V4(e)) if to.port() == ep.port() => t.is_broadcast() || (t.octets()[3] == 255 && t.octets()[..3] == e.octets()[..3]),
        _ => false,
    }
}

fn is_broadcast(to: SocketAddr) -> bool {
    matches!(to.ip(), IpAddr::V4(t) if t.is_broadcast() || t.octets()[3] == 255)
}

impl NetInner {
    fn submit(&mut self, from: SocketAddr, to: SocketAddr, data: &[u8]) {
        if is_broadcast(to) {
            // Broadcast: one copy to every endpoint on that port (not the sender).
            let mut eps: Vec<SocketAddr> = self.inboxes.keys().copied().filter(|&e| e != from && broadcast_reaches(to, e)).collect();
            eps.sort();
            for e in eps {
                self.submit_one(from, e, data);
            }
            return;
        }
        self.submit_one(from, to, data);
    }

    fn submit_one(&mut self, from: SocketAddr, to: SocketAddr, data: &[u8]) {
        if let Some(c) = self.capture.as_mut() {
            c.push((from, to, data.to_vec()));
        }
        let now = self.now;
        let default = self.default.clone();
        let link = self.links.entry((from, to)).or_default();
        let cond = link.cond.clone().unwrap_or(default);
        link.stats.sent += 1;
        link.stats.bytes_sent += data.len() as u64;
        if self.rng.chance(cond.loss) {
            link.stats.lost += 1;
            return;
        }
        let copies = if self.rng.chance(cond.duplicate) {
            link.stats.duplicated += 1;
            2
        } else {
            1
        };
        for _ in 0..copies {
            let mut depart = now;
            if let Some(bw) = cond.bandwidth.filter(|&b| b > 0) {
                let start = link.busy_until.max(now);
                if start - now > cond.queue_limit {
                    link.stats.queue_dropped += 1;
                    continue;
                }
                let tx = Duration::from_secs_f64(data.len() as f64 / bw as f64);
                link.busy_until = start + tx;
                depart = start + tx;
            }
            let j = cond.jitter.as_secs_f64();
            let jitter = (self.rng.next_f64() * 2.0 - 1.0) * j;
            let delay = (cond.latency.as_secs_f64() + jitter).max(0.0);
            let mut arrival = depart + Duration::from_secs_f64(delay);
            if self.rng.chance(cond.reorder) {
                link.stats.reordered += 1;
                arrival += cond.reorder_delay;
            } else {
                arrival = arrival.max(link.last_arrival);
                link.last_arrival = arrival;
            }
            self.counter += 1;
            let order = self.counter;
            self.inboxes.entry(to).or_default().push(Reverse((arrival, order, from, data.to_vec())));
        }
    }
}

/// One endpoint of a [`MemNet`].
#[derive(Debug, Clone)]
pub struct MemLink {
    net: MemNet,
    addr: SocketAddr,
}

impl MemLink {
}

impl Datagram for MemLink {
    fn send_to(&mut self, data: &[u8], addr: SocketAddr) -> io::Result<()> {
        self.net.lock().submit(self.addr, addr, data);
        Ok(())
    }

    fn recv_from(&mut self, buf: &mut [u8]) -> Option<(usize, SocketAddr)> {
        let mut n = self.net.lock();
        let now = n.now;
        let inbox = n.inboxes.get_mut(&self.addr)?;
        if inbox.peek().is_none_or(|Reverse((t, ..))| *t > now) {
            return None;
        }
        let Reverse((_, _, from, data)) = inbox.pop()?;
        if let Some(l) = n.links.get_mut(&(from, self.addr)) {
            l.stats.delivered += 1;
        }
        let len = data.len().min(buf.len());
        buf[..len].copy_from_slice(&data[..len]);
        Some((len, from))
    }

    fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(p: u16) -> SocketAddr {
        SocketAddr::from(([10, 0, 0, 1], p))
    }

    fn drain(l: &mut MemLink) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        while let Some((n, _)) = l.recv_from(&mut buf) {
            assert_eq!(n, 1);
            out.push(buf[0]);
        }
        out
    }

    #[test]
    fn latency_and_order() {
        let net = MemNet::new(1);
        net.set_conditions(LinkConditions {
            latency: Duration::from_millis(50),
            jitter: Duration::from_millis(20),
            ..Default::default()
        });
        let mut x = net.endpoint(a(1));
        let mut y = net.endpoint(a(2));
        for i in 0..100u8 {
            x.send_to(&[i], a(2)).unwrap();
        }
        net.advance(Duration::from_millis(29));
        assert!(drain(&mut y).is_empty());
        net.advance(Duration::from_millis(100));
        assert_eq!(drain(&mut y), (0..100).collect::<Vec<_>>()); // jitter does not reorder
    }

    #[test]
    fn loss_dup_reorder_are_seeded() {
        let run = |seed| {
            let net = MemNet::new(seed);
            net.set_conditions(LinkConditions {
                latency: Duration::from_millis(10),
                loss: 0.2,
                duplicate: 0.1,
                reorder: 0.1,
                reorder_delay: Duration::from_millis(30),
                ..Default::default()
            });
            let mut x = net.endpoint(a(1));
            let mut y = net.endpoint(a(2));
            let mut got = Vec::new();
            for i in 0..200u8 {
                x.send_to(&[i], a(2)).unwrap();
                net.advance(Duration::from_millis(5));
                got.extend(drain(&mut y));
            }
            net.advance(Duration::from_secs(1));
            got.extend(drain(&mut y));
            (got, net.link_stats(a(1), a(2)))
        };
        let (g1, s1) = run(9);
        let (g2, s2) = run(9);
        assert_eq!(g1, g2);
        assert_eq!(s1, s2);
        assert!((25..=60).contains(&s1.lost), "{s1:?}");
        assert!(s1.duplicated > 5 && s1.reordered > 5);
        assert_eq!(s1.delivered as usize, g1.len());
        assert!(g1.windows(2).any(|w| w[0] > w[1]), "some reordering");
    }

    #[test]
    fn bandwidth_cap() {
        let net = MemNet::new(1);
        net.set_conditions(LinkConditions {
            bandwidth: Some(10_000),
            queue_limit: Duration::from_millis(500),
            ..Default::default()
        });
        let mut x = net.endpoint(a(1));
        let mut y = net.endpoint(a(2));
        for _ in 0..20 {
            x.send_to(&[0u8; 1000], a(2)).unwrap(); // 100 ms each
        }
        let s = net.link_stats(a(1), a(2));
        assert_eq!(s.queue_dropped, 14); // 6 fit the 500 ms queue
        let mut buf = [0u8; 2000];
        net.advance(Duration::from_millis(250));
        let mut n = 0;
        while y.recv_from(&mut buf).is_some() {
            n += 1;
        }
        assert_eq!(n, 2);
    }

    #[test]
    fn udp_loopback_basic() {
        let mut s = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let mut c = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let sa = s.local_addr();
        c.send_to(b"hi", sa).unwrap();
        let mut buf = [0u8; 16];
        let start = std::time::Instant::now();
        loop {
            if let Some((n, from)) = s.recv_from(&mut buf) {
                assert_eq!(&buf[..n], b"hi");
                assert_eq!(from, c.local_addr());
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

// ---------------------------------------------------------------------------
// NAT simulation

/// A port-restricted cone NAT in front of an endpoint (tests of the master
/// server's hole punching): the endpoint's address stands for the NAT's
/// public address, and a datagram from outside is delivered only from an
/// address the endpoint has sent to before (the usual home-router filter).
/// The mapping is permanent (no expiry).
#[derive(Debug)]
pub struct RestrictedNat<T: Datagram> {
    inner: T,
    opened: std::collections::HashSet<SocketAddr>,
    /// Datagrams the filter dropped.
    pub filtered: u64,
}

impl<T: Datagram> RestrictedNat<T> {
    pub fn new(inner: T) -> Self {
        RestrictedNat { inner, opened: Default::default(), filtered: 0 }
    }
}

impl<T: Datagram> Datagram for RestrictedNat<T> {
    fn send_to(&mut self, data: &[u8], addr: SocketAddr) -> io::Result<()> {
        self.opened.insert(addr);
        self.inner.send_to(data, addr)
    }

    fn recv_from(&mut self, buf: &mut [u8]) -> Option<(usize, SocketAddr)> {
        loop {
            let (n, from) = self.inner.recv_from(buf)?;
            if self.opened.contains(&from) {
                return Some((n, from));
            }
            self.filtered += 1;
        }
    }

    fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr()
    }
}

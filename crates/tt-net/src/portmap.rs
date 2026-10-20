//! Automatic port forwarding: ask the home router to forward a UDP port to
//! this computer, so a game hosted behind a NAT is reachable from the
//! internet without the player opening the port by hand.
//!
//! Two protocols are tried, in this order:
//!
//! * **NAT-PMP** (RFC 6886): two tiny UDP requests to `gateway:5351`
//!   (external address, op 0; UDP mapping, op 1). Apple routers, many
//!   OpenWrt/pfSense/miniupnpd setups. A mapping is deleted by requesting it
//!   again with lifetime 0 (and external port 0).
//! * **UPnP IGD** (UPnP InternetGatewayDevice:1 and :2): SSDP `M-SEARCH`
//!   to 239.255.255.250:1900, an HTTP GET of the device description named by
//!   the `LOCATION` header, then SOAP `AddPortMapping`,
//!   `GetExternalIPAddress` and `DeletePortMapping` on the
//!   `WANIPConnection:1`/`:2` or `WANPPPConnection:1` service. The XML is
//!   scanned with a small tolerant string matcher (namespace prefixes and tag
//!   case ignored); there is no XML crate, `tt-net` stays dependency-free.
//!
//! **This module blocks.** Unlike the rest of `tt-net` (non-blocking, driven
//! by `update(now)`), every function here does blocking std socket I/O with
//! timeouts of up to [`PortMapConfig::timeout`] per method (and per HTTP
//! request). Call it from a background thread; [`PortMapper`] is exactly
//! that: it maps at start, renews at half the lifetime and removes the
//! mapping when stopped or dropped.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// NAT-PMP server port on the gateway (RFC 6886 §3).
pub const NATPMP_PORT: u16 = 5351;
/// SSDP multicast group and port (UPnP Device Architecture §1.3).
pub const SSDP_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900));
/// Lifetime [`PortMapper`] requests over NAT-PMP (RFC 6886 recommends 2 h).
pub const DEFAULT_NATPMP_LIFETIME: Duration = Duration::from_secs(2 * 3600);
/// Lease [`PortMapper`] requests over UPnP.
pub const DEFAULT_UPNP_LEASE: Duration = Duration::from_secs(3600);
/// `NewPortMappingDescription`, shown in the router's web UI.
pub const MAPPING_DESCRIPTION: &str = crate::PRODUCT_NAME;

/// SSDP search targets: the root device and the three usable services.
const SEARCH_TARGETS: [&str; 4] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];
/// Services that carry `AddPortMapping`, in order of preference.
const WAN_SERVICES: [&str; 3] = [
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];
/// First NAT-PMP retransmission interval (RFC 6886 §3.1), doubled each try.
const NATPMP_FIRST_RETRY: Duration = Duration::from_millis(250);
/// Largest HTTP response accepted from the router.
const MAX_HTTP_RESPONSE: usize = 1 << 20;
/// How long [`PortMapper`] waits before trying again after a failure.
const RETRY_INTERVAL: Duration = Duration::from_secs(60);

/// Which protocol made a mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    NatPmp,
    Upnp,
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Method::NatPmp => "NAT-PMP",
            Method::Upnp => "UPnP",
        })
    }
}

/// An active port mapping on the gateway.
#[derive(Debug, Clone)]
pub struct Mapping {
    pub method: Method,
    pub internal_port: u16,
    /// Port opened on the router's public side (may differ from
    /// `internal_port` with NAT-PMP if the gateway picked another one).
    pub external_port: u16,
    /// The router's public address, if it told us.
    pub external_ip: Option<Ipv4Addr>,
    /// Granted lifetime; zero for a permanent UPnP lease.
    pub lifetime: Duration,
    /// Lifetime asked for (renewals ask for it again).
    requested: Duration,
    /// NAT-PMP server address.
    gateway: Option<SocketAddrV4>,
    /// UPnP control point.
    upnp: Option<UpnpTarget>,
}

/// Where and how to reach the router.
#[derive(Debug, Clone)]
pub struct PortMapConfig {
    /// Per method (NAT-PMP retries, SSDP search) and per HTTP request.
    pub timeout: Duration,
    /// NAT-PMP gateway; `None` = [`default_gateway`].
    pub gateway: Option<Ipv4Addr>,
    /// NAT-PMP port, [`NATPMP_PORT`]; tests override it.
    pub natpmp_port: u16,
    /// SSDP destination, [`SSDP_ADDR`]; tests point it at a loopback fake.
    pub ssdp_addr: SocketAddr,
    pub try_natpmp: bool,
    pub try_upnp: bool,
}

impl Default for PortMapConfig {
    fn default() -> Self {
        PortMapConfig {
            timeout: Duration::from_secs(2),
            gateway: None,
            natpmp_port: NATPMP_PORT,
            ssdp_addr: SSDP_ADDR,
            try_natpmp: true,
            try_upnp: true,
        }
    }
}

#[derive(Debug)]
pub enum PortMapError {
    /// No default gateway could be determined (NAT-PMP only).
    NoGateway,
    /// Nothing answered in time.
    Timeout,
    /// The gateway answered but said no (NAT-PMP result code, SOAP fault).
    Refused(String),
    Io(io::Error),
    /// Malformed or unexpected answer; also the combined report when both
    /// methods failed. The message names the protocol.
    Protocol(String),
}

impl fmt::Display for PortMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortMapError::NoGateway => f.write_str("no default gateway found"),
            PortMapError::Timeout => f.write_str("no answer from the router (timed out)"),
            PortMapError::Refused(s) => write!(f, "refused by the router: {s}"),
            PortMapError::Io(e) => write!(f, "network error: {e}"),
            PortMapError::Protocol(s) => f.write_str(s),
        }
    }
}

impl From<io::Error> for PortMapError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => PortMapError::Timeout,
            _ => PortMapError::Io(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Removes a mapping made by [`map_with`] / [`renew`].
pub fn unmap(m: &Mapping, cfg: &PortMapConfig) -> Result<(), PortMapError> {
    match m.method {
        Method::NatPmp => {
            let gw = m.gateway.ok_or(PortMapError::NoGateway)?;
            natpmp_map(gw, m.internal_port, 0, 0, cfg.timeout)?;
            Ok(())
        }
        Method::Upnp => {
            let t = m.upnp.as_ref().ok_or(PortMapError::NoGateway)?;
            let args = [
                ("NewRemoteHost", String::new()),
                ("NewExternalPort", m.external_port.to_string()),
                ("NewProtocol", "UDP".to_string()),
            ];
            match soap_call(t, "DeletePortMapping", &args, cfg.timeout) {
                // 714 NoSuchEntryInArray: already gone.
                Ok(_) | Err(SoapError::Fault(714, _)) => Ok(()),
                Err(e) => Err(e.into()),
            }
        }
    }
}

/// Refreshes a mapping before its lifetime ends (same ports, same gateway).
pub fn renew(m: &Mapping, cfg: &PortMapConfig) -> Result<Mapping, PortMapError> {
    match m.method {
        Method::NatPmp => {
            let gw = m.gateway.ok_or(PortMapError::NoGateway)?;
            let (_, external, lifetime) = natpmp_map(
                gw,
                m.internal_port,
                m.external_port,
                natpmp_secs(m.requested),
                cfg.timeout,
            )?;
            let external_ip = natpmp_external_ip(gw, cfg.timeout).ok().or(m.external_ip);
            Ok(Mapping {
                external_port: external,
                external_ip,
                lifetime: Duration::from_secs(lifetime.into()),
                ..m.clone()
            })
        }
        Method::Upnp => {
            let t = m.upnp.as_ref().ok_or(PortMapError::NoGateway)?;
            let lease = upnp_add(
                t,
                m.internal_port,
                m.external_port,
                m.requested,
                cfg.timeout,
            )?;
            let external_ip = upnp_external_ip(t, cfg.timeout)
                .ok()
                .flatten()
                .or(m.external_ip);
            Ok(Mapping {
                external_ip,
                lifetime: lease,
                ..m.clone()
            })
        }
    }
}

/// The IPv4 default gateway. Linux: the default route of `/proc/net/route`.
/// Elsewhere (or if that fails): a guess, the primary interface address with
/// the last octet set to 1.
pub fn default_gateway() -> Option<Ipv4Addr> {
    #[cfg(target_os = "linux")]
    {
        if let Some(gw) = std::fs::read_to_string("/proc/net/route")
            .ok()
            .and_then(|t| parse_proc_net_route(&t))
        {
            return Some(gw);
        }
    }
    let ip = local_ipv4_towards(Ipv4Addr::new(192, 0, 2, 1)).ok()?;
    if ip.is_loopback() || ip.is_unspecified() {
        return None;
    }
    let [a, b, c, _] = ip.octets();
    Some(Ipv4Addr::new(a, b, c, 1))
}

fn map_with(
    internal_port: u16,
    natpmp_lifetime: Duration,
    upnp_lease: Duration,
    cfg: &PortMapConfig,
) -> Result<Mapping, PortMapError> {
    let mut errors = Vec::new();
    if cfg.try_natpmp {
        match natpmp_map_new(internal_port, natpmp_lifetime, cfg) {
            Ok(m) => return Ok(m),
            Err(e) => errors.push((Method::NatPmp, e)),
        }
    }
    if cfg.try_upnp {
        match upnp_map_new(internal_port, upnp_lease, cfg) {
            Ok(m) => return Ok(m),
            Err(e) => errors.push((Method::Upnp, e)),
        }
    }
    match errors.len() {
        0 => Err(PortMapError::Protocol(
            "port mapping: no method enabled".into(),
        )),
        1 => Err(errors.pop().unwrap().1),
        _ => Err(PortMapError::Protocol(
            errors
                .iter()
                .map(|(m, e)| format!("{m}: {e}"))
                .collect::<Vec<_>>()
                .join("; "),
        )),
    }
}

/// Local address of a UDP socket connected to `peer`: the address of the
/// interface that routes there. Connecting a UDP socket sends nothing.
fn local_ipv4_towards(peer: Ipv4Addr) -> io::Result<Ipv4Addr> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.connect((peer, 9))?;
    match sock.local_addr()? {
        SocketAddr::V4(a) => Ok(*a.ip()),
        SocketAddr::V6(_) => Err(io::Error::other("not IPv4")),
    }
}

/// Default route from the text of `/proc/net/route` (fields are hex, the
/// address in host = little-endian byte order); lowest metric wins.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_proc_net_route(text: &str) -> Option<Ipv4Addr> {
    const RTF_UP_GATEWAY: u32 = 0x1 | 0x2;
    let mut best: Option<(u32, Ipv4Addr)> = None;
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 {
            continue;
        }
        let hex = |s: &str| u32::from_str_radix(s, 16).ok();
        let (Some(dest), Some(gw), Some(flags), Some(mask)) =
            (hex(f[1]), hex(f[2]), hex(f[3]), hex(f[7]))
        else {
            continue;
        };
        if dest != 0 || mask != 0 || gw == 0 || flags & RTF_UP_GATEWAY != RTF_UP_GATEWAY {
            continue;
        }
        let metric = f[6].parse().unwrap_or(u32::MAX);
        let ip = Ipv4Addr::from(gw.to_le_bytes());
        if best.is_none_or(|(m, _)| metric < m) {
            best = Some((metric, ip));
        }
    }
    best.map(|(_, ip)| ip)
}

// ---------------------------------------------------------------------------
// NAT-PMP (RFC 6886)
// ---------------------------------------------------------------------------

const NATPMP_OP_EXTERNAL: u8 = 0;
const NATPMP_OP_MAP_UDP: u8 = 1;

fn natpmp_secs(d: Duration) -> u32 {
    d.as_secs().clamp(1, u32::MAX.into()) as u32
}

fn natpmp_map_new(
    internal_port: u16,
    lifetime: Duration,
    cfg: &PortMapConfig,
) -> Result<Mapping, PortMapError> {
    let ip = cfg
        .gateway
        .or_else(default_gateway)
        .ok_or(PortMapError::NoGateway)?;
    let gw = SocketAddrV4::new(ip, cfg.natpmp_port);
    // Op 0 first: cheap, and tells quickly whether NAT-PMP is there at all.
    let external_ip = natpmp_external_ip(gw, cfg.timeout)?;
    let (_, external, granted) = natpmp_map(
        gw,
        internal_port,
        internal_port,
        natpmp_secs(lifetime),
        cfg.timeout,
    )?;
    Ok(Mapping {
        method: Method::NatPmp,
        internal_port,
        external_port: external,
        external_ip: (!external_ip.is_unspecified()).then_some(external_ip),
        lifetime: Duration::from_secs(granted.into()),
        requested: lifetime,
        gateway: Some(gw),
        upnp: None,
    })
}

/// Op 0: the gateway's public address.
fn natpmp_external_ip(gw: SocketAddrV4, timeout: Duration) -> Result<Ipv4Addr, PortMapError> {
    let r = natpmp_request(gw, &[0, NATPMP_OP_EXTERNAL], timeout)?;
    natpmp_check(&r, 12)?;
    Ok(Ipv4Addr::new(r[8], r[9], r[10], r[11]))
}

/// Op 1: maps UDP `internal` (suggesting `external`) for `lifetime` seconds;
/// lifetime 0 with external 0 deletes. Returns (internal, external, lifetime).
fn natpmp_map(
    gw: SocketAddrV4,
    internal: u16,
    external: u16,
    lifetime: u32,
    timeout: Duration,
) -> Result<(u16, u16, u32), PortMapError> {
    let mut req = [0u8; 12];
    req[1] = NATPMP_OP_MAP_UDP;
    req[4..6].copy_from_slice(&internal.to_be_bytes());
    req[6..8].copy_from_slice(&external.to_be_bytes());
    req[8..12].copy_from_slice(&lifetime.to_be_bytes());
    let r = natpmp_request(gw, &req, timeout)?;
    natpmp_check(&r, 16)?;
    let got_internal = u16::from_be_bytes([r[8], r[9]]);
    if got_internal != internal {
        return Err(PortMapError::Protocol(format!(
            "NAT-PMP: answer for port {got_internal}, asked for {internal}"
        )));
    }
    Ok((
        got_internal,
        u16::from_be_bytes([r[10], r[11]]),
        u32::from_be_bytes([r[12], r[13], r[14], r[15]]),
    ))
}

/// Result code and length of a response (version 0, op 128 + n, result
/// code, seconds since epoch, ...).
fn natpmp_check(r: &[u8], len: usize) -> Result<(), PortMapError> {
    let code = u16::from_be_bytes([r[2], r[3]]);
    if code != 0 {
        return Err(PortMapError::Refused(format!(
            "NAT-PMP result {code} ({})",
            natpmp_result_text(code)
        )));
    }
    if r.len() < len {
        return Err(PortMapError::Protocol(format!(
            "NAT-PMP: short response ({} bytes)",
            r.len()
        )));
    }
    Ok(())
}

/// RFC 6886 §3.5.
fn natpmp_result_text(code: u16) -> &'static str {
    match code {
        1 => "unsupported version",
        2 => "not authorized / refused, NAT-PMP may be disabled on the router",
        3 => "network failure, the router has no public address yet",
        4 => "out of resources",
        5 => "unsupported opcode",
        _ => "unknown error",
    }
}

/// Sends `req` and waits for the matching response (op + 128), resending at
/// 250 ms, 500 ms, 1 s, ... (RFC 6886 §3.1) until `timeout` is used up.
fn natpmp_request(
    gw: SocketAddrV4,
    req: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, PortMapError> {
    let closed = || PortMapError::Refused("NAT-PMP port closed on the router".into());
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    // Connected: only the gateway's answers are received (RFC 6886 §3.1).
    sock.connect(gw)?;
    let want_op = 128 + req[1];
    let deadline = Instant::now() + timeout;
    let mut delay = NATPMP_FIRST_RETRY;
    let mut buf = [0u8; 64];
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(PortMapError::Timeout);
        }
        match sock.send(req) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => return Err(closed()),
            Err(e) => return Err(e.into()),
        }
        let attempt_end = (now + delay).min(deadline);
        delay *= 2;
        loop {
            let left = attempt_end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            sock.set_read_timeout(Some(left))?;
            match sock.recv(&mut buf) {
                Ok(n) if n >= 4 && buf[0] == 0 && buf[1] == want_op => return Ok(buf[..n].to_vec()),
                Ok(_) => {} // stray packet
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break
                }
                // ICMP port unreachable: no NAT-PMP server, no point waiting.
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => return Err(closed()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// UPnP IGD:1 / IGD:2
// ---------------------------------------------------------------------------

/// A WAN*Connection service control point.
#[derive(Debug, Clone)]
struct UpnpTarget {
    control_url: Url,
    service_type: String,
    /// Our LAN address, `NewInternalClient`.
    internal_client: Ipv4Addr,
}

fn upnp_map_new(
    internal_port: u16,
    lease: Duration,
    cfg: &PortMapConfig,
) -> Result<Mapping, PortMapError> {
    let (control_url, service_type) = ssdp_discover(cfg)?;
    let internal_client = local_ipv4_towards(resolve_ipv4(&control_url)?)?;
    let target = UpnpTarget {
        control_url,
        service_type,
        internal_client,
    };
    let granted = upnp_add(&target, internal_port, internal_port, lease, cfg.timeout)?;
    let external_ip = upnp_external_ip(&target, cfg.timeout).ok().flatten();
    Ok(Mapping {
        method: Method::Upnp,
        internal_port,
        external_port: internal_port,
        external_ip,
        lifetime: granted,
        requested: lease,
        gateway: None,
        upnp: Some(target),
    })
}

/// `AddPortMapping`; on 725 (OnlyPermanentLeasesSupported, IGD:1 routers)
/// retries with lease 0. Returns the lease in effect (zero = permanent).
fn upnp_add(
    t: &UpnpTarget,
    internal: u16,
    external: u16,
    lease: Duration,
    timeout: Duration,
) -> Result<Duration, PortMapError> {
    let add = |secs: u64| {
        let args = [
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", external.to_string()),
            ("NewProtocol", "UDP".to_string()),
            ("NewInternalPort", internal.to_string()),
            ("NewInternalClient", t.internal_client.to_string()),
            ("NewEnabled", "1".to_string()),
            ("NewPortMappingDescription", MAPPING_DESCRIPTION.to_string()),
            ("NewLeaseDuration", secs.to_string()),
        ];
        soap_call(t, "AddPortMapping", &args, timeout)
    };
    let secs = lease.as_secs().min(u32::MAX.into());
    match add(secs) {
        Ok(_) => Ok(Duration::from_secs(secs)),
        Err(SoapError::Fault(725, _)) if secs != 0 => {
            add(0)?;
            Ok(Duration::ZERO)
        }
        Err(e) => Err(e.into()),
    }
}

/// `GetExternalIPAddress`; `None` if the router has no (valid) one.
fn upnp_external_ip(t: &UpnpTarget, timeout: Duration) -> Result<Option<Ipv4Addr>, PortMapError> {
    let body = soap_call(t, "GetExternalIPAddress", &[], timeout)?;
    Ok(tag_text(&body, "NewExternalIPAddress")
        .and_then(|s| s.parse::<Ipv4Addr>().ok())
        .filter(|ip| !ip.is_unspecified()))
}

/// SSDP search; returns the control URL and type of the first usable WAN
/// connection service found in a responding device's description.
fn ssdp_discover(cfg: &PortMapConfig) -> Result<(Url, String), PortMapError> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    if let SocketAddr::V4(a) = cfg.ssdp_addr {
        if a.ip().is_multicast() {
            let _ = sock.set_multicast_ttl_v4(2);
        }
    }
    let search = |sock: &UdpSocket| {
        for st in SEARCH_TARGETS {
            let msg = format!(
                "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\n\
                 MAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {st}\r\n\r\n"
            );
            let _ = sock.send_to(msg.as_bytes(), cfg.ssdp_addr);
        }
    };
    let start = Instant::now();
    let deadline = start + cfg.timeout;
    let resend_at = start + cfg.timeout / 2;
    let mut resent = false;
    search(&sock);

    let mut seen: Vec<String> = Vec::new();
    let mut last_err = None;
    let mut buf = [0u8; 2048];
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(last_err.unwrap_or(PortMapError::Timeout));
        }
        if !resent && now >= resend_at {
            search(&sock); // UDP may lose the first burst
            resent = true;
        }
        let until = if resent { deadline } else { resend_at };
        sock.set_read_timeout(Some(
            until
                .saturating_duration_since(now)
                .max(Duration::from_millis(1)),
        ))?;
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let Some(loc) = header_value(&text, "location") else {
                    continue;
                };
                if seen.iter().any(|s| s == loc) {
                    continue;
                }
                seen.push(loc.to_string());
                match describe(loc, cfg.timeout) {
                    Ok(found) => return Ok(found),
                    Err(e) => last_err = Some(e),
                }
            }
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock
                | io::ErrorKind::TimedOut
                | io::ErrorKind::Interrupted
                | io::ErrorKind::ConnectionRefused
                | io::ErrorKind::ConnectionReset => {}
                _ => return Err(e.into()),
            },
        }
    }
}

/// Fetches a device description and picks the WAN connection service.
fn describe(location: &str, timeout: Duration) -> Result<(Url, String), PortMapError> {
    let url = Url::parse(location)?;
    let (status, xml) = http_request(&url, "GET", &[], "", timeout)?;
    if status != 200 {
        return Err(PortMapError::Protocol(format!(
            "UPnP: device description {location} returned HTTP {status}"
        )));
    }
    let (service_type, control) = find_wan_service(&xml).ok_or_else(|| {
        PortMapError::Protocol(format!("UPnP: no WAN connection service in {location}"))
    })?;
    // controlURL is relative to URLBase (IGD:1) or else to the description.
    let base = match tag_text(&xml, "URLBase").filter(|s| !s.is_empty()) {
        Some(b) => Url::parse(&xml_unescape(b))?,
        None => url,
    };
    Ok((base.join(&control)?, service_type))
}

/// `(serviceType, controlURL)` of the most preferred WAN service.
fn find_wan_service(xml: &str) -> Option<(String, String)> {
    let mut services = Vec::new();
    let mut pos = 0;
    while let Some((a, b, end)) = find_element(xml, "service", pos) {
        let block = &xml[a..b];
        if let (Some(st), Some(ctl)) = (
            tag_text(block, "serviceType"),
            tag_text(block, "controlURL"),
        ) {
            services.push((xml_unescape(st), xml_unescape(ctl)));
        }
        pos = end;
    }
    WAN_SERVICES.iter().find_map(|want| {
        services
            .iter()
            .find(|(st, ctl)| st.eq_ignore_ascii_case(want) && !ctl.is_empty())
            .cloned()
    })
}

/// A failed SOAP action.
enum SoapError {
    /// UPnP error code and description.
    Fault(u16, String),
    Other(PortMapError),
}

impl From<PortMapError> for SoapError {
    fn from(e: PortMapError) -> Self {
        SoapError::Other(e)
    }
}

impl From<SoapError> for PortMapError {
    fn from(e: SoapError) -> Self {
        match e {
            SoapError::Fault(code, desc) if desc.is_empty() => PortMapError::Refused(format!("UPnP error {code}")),
            SoapError::Fault(code, desc) => PortMapError::Refused(format!("UPnP error {code} {desc}")),
            SoapError::Other(e) => e,
        }
    }
}

/// POSTs a SOAP action; returns the response body.
fn soap_call(
    t: &UpnpTarget,
    action: &str,
    args: &[(&str, String)],
    timeout: Duration,
) -> Result<String, SoapError> {
    let mut inner = String::new();
    for (k, v) in args {
        inner.push_str(&format!("<{k}>{}</{k}>", xml_escape(v)));
    }
    let body = format!(
        "<?xml version=\"1.0\"?>\r\n\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body><u:{action} xmlns:u=\"{st}\">{inner}</u:{action}></s:Body></s:Envelope>\r\n",
        st = t.service_type
    );
    let headers = [
        ("Content-Type", "text/xml; charset=\"utf-8\"".to_string()),
        ("SOAPAction", format!("\"{}#{action}\"", t.service_type)),
    ];
    let (status, text) = http_request(&t.control_url, "POST", &headers, &body, timeout)?;
    if status == 200 {
        return Ok(text);
    }
    match tag_text(&text, "errorCode").and_then(|c| c.parse().ok()) {
        Some(code) => Err(SoapError::Fault(
            code,
            tag_text(&text, "errorDescription")
                .map(xml_unescape)
                .unwrap_or_default(),
        )),
        None => Err(SoapError::Other(PortMapError::Protocol(format!(
            "UPnP: {action} failed with HTTP {status}"
        )))),
    }
}

// ---------------------------------------------------------------------------
// Minimal HTTP/1.1 client
// ---------------------------------------------------------------------------

/// `http://host[:port]/path`; the only form routers use.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Url {
    host: String,
    port: u16,
    path: String,
}

impl Url {
    fn parse(s: &str) -> Result<Url, PortMapError> {
        let s = s.trim();
        let bad = || PortMapError::Protocol(format!("UPnP: unsupported URL {s:?}"));
        let rest = strip_prefix_ci(s, "http://").ok_or_else(bad)?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let authority = authority.rsplit('@').next().unwrap_or(authority);
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().map_err(|_| bad())?),
            None => (authority, 80),
        };
        if host.is_empty() {
            return Err(bad());
        }
        Ok(Url {
            host: host.to_string(),
            port,
            path: path.to_string(),
        })
    }

    /// Resolves a reference against this URL (absolute, absolute path, or
    /// relative to the directory of this URL's path).
    fn join(&self, reference: &str) -> Result<Url, PortMapError> {
        let r = reference.trim();
        if strip_prefix_ci(r, "http://").is_some() {
            return Url::parse(r);
        }
        let path = if r.starts_with('/') {
            r.to_string()
        } else {
            let base = self.path.split('?').next().unwrap_or("/");
            let dir = &base[..base.rfind('/').map_or(0, |i| i + 1)];
            let dir = if dir.is_empty() { "/" } else { dir };
            format!("{dir}{r}")
        };
        Ok(Url {
            path,
            ..self.clone()
        })
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "http://{}:{}{}", self.host, self.port, self.path)
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn resolve_ipv4(url: &Url) -> Result<Ipv4Addr, PortMapError> {
    if let Ok(ip) = url.host.parse() {
        return Ok(ip);
    }
    (url.host.as_str(), url.port)
        .to_socket_addrs()?
        .find_map(|a| match a {
            SocketAddr::V4(v4) => Some(*v4.ip()),
            SocketAddr::V6(_) => None,
        })
        .ok_or_else(|| PortMapError::Protocol(format!("UPnP: cannot resolve {}", url.host)))
}

/// One request on a fresh connection (`Connection: close`); returns the
/// status code and body. Handles Content-Length, chunked and read-to-EOF
/// bodies.
fn http_request(
    url: &Url,
    method: &str,
    headers: &[(&str, String)],
    body: &str,
    timeout: Duration,
) -> Result<(u16, String), PortMapError> {
    let deadline = Instant::now() + timeout;
    let addr = SocketAddr::from((resolve_ipv4(url)?, url.port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_write_timeout(Some(timeout))?;
    let mut req = format!(
        "{method} {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\
         User-Agent: {}/1.0 UPnP/1.1\r\n",
        url.path, url.host, url.port, crate::GAME_TYPE
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if method != "GET" {
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream.write_all(req.as_bytes())?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(r) = parse_http_response(&buf, false) {
            return r;
        }
        if buf.len() > MAX_HTTP_RESPONSE {
            return Err(PortMapError::Protocol(
                "UPnP: HTTP response too large".into(),
            ));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(PortMapError::Timeout);
        }
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut chunk) {
            Ok(0) => {
                return parse_http_response(&buf, true).unwrap_or_else(|| {
                    Err(PortMapError::Protocol(
                        "UPnP: truncated HTTP response".into(),
                    ))
                })
            }
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
}

/// `None` while the response is incomplete (and not at EOF).
fn parse_http_response(buf: &[u8], eof: bool) -> Option<Result<(u16, String), PortMapError>> {
    let proto = |m: &str| Some(Err(PortMapError::Protocol(format!("UPnP: {m}"))));
    let Some(hdr_end) = find_bytes(buf, b"\r\n\r\n") else {
        return if eof {
            proto("HTTP response without headers")
        } else {
            None
        };
    };
    let head = String::from_utf8_lossy(&buf[..hdr_end]);
    let status = head
        .lines()
        .next()
        .filter(|l| l.starts_with("HTTP/"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok());
    let Some(status) = status else {
        return proto("bad HTTP status line");
    };
    let raw = &buf[hdr_end + 4..];
    let chunked = header_value(&head, "transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let body = if chunked {
        match decode_chunked(raw) {
            Some(Ok(b)) => b,
            Some(Err(e)) => return Some(Err(e)),
            None if eof => return proto("truncated chunked body"),
            None => return None,
        }
    } else if let Some(len) =
        header_value(&head, "content-length").and_then(|v| v.parse::<usize>().ok())
    {
        if raw.len() >= len {
            raw[..len].to_vec()
        } else if eof {
            return proto("truncated HTTP body");
        } else {
            return None;
        }
    } else if eof {
        raw.to_vec()
    } else {
        return None;
    };
    Some(Ok((status, String::from_utf8_lossy(&body).into_owned())))
}

/// Chunked transfer coding; `None` if incomplete.
fn decode_chunked(mut raw: &[u8]) -> Option<Result<Vec<u8>, PortMapError>> {
    let mut out = Vec::new();
    loop {
        let line_end = find_bytes(raw, b"\r\n")?;
        let line = String::from_utf8_lossy(&raw[..line_end]);
        let size_str = line.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_str, 16) else {
            return Some(Err(PortMapError::Protocol(format!(
                "UPnP: bad chunk size {size_str:?}"
            ))));
        };
        raw = &raw[line_end + 2..];
        if size == 0 {
            return Some(Ok(out)); // trailers ignored
        }
        if raw.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&raw[..size]);
        raw = &raw[size + 2..];
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Value of an HTTP-style header (name ASCII case-insensitive), skipping the
/// first (request/status) line.
fn header_value<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

// ---------------------------------------------------------------------------
// Tolerant XML scanning
// ---------------------------------------------------------------------------

/// Next element whose local name is `name` (prefix ignored, ASCII
/// case-insensitive) at or after byte `from`: (content start, content end,
/// end of the closing tag).
fn find_element(s: &str, name: &str, from: usize) -> Option<(usize, usize, usize)> {
    let local = |n: &str| n.rsplit(':').next().unwrap_or("").to_string();
    let mut pos = from;
    loop {
        let lt = pos + s.get(pos..)?.find('<')?;
        let gt = lt + s[lt..].find('>')?;
        let tag = &s[lt + 1..gt];
        pos = gt + 1;
        if tag.starts_with(['/', '?', '!']) {
            continue;
        }
        let tag_name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        if !local(tag_name).eq_ignore_ascii_case(name) {
            continue;
        }
        if tag.ends_with('/') {
            return Some((pos, pos, pos));
        }
        let mut p = pos;
        loop {
            let c = p + s[p..].find("</")?;
            let cg = c + s[c..].find('>')?;
            if local(s[c + 2..cg].trim()).eq_ignore_ascii_case(name) {
                return Some((pos, c, cg + 1));
            }
            p = cg + 1;
        }
    }
}

/// Trimmed text of the first element `name`.
fn tag_text<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    find_element(s, name, 0).map(|(a, b, _)| s[a..b].trim())
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Background mapper
// ---------------------------------------------------------------------------

/// What a [`PortMapper`] has achieved so far.
#[derive(Debug, Clone, PartialEq)]
pub enum PortMapStatus {
    /// First attempt still running.
    Pending,
    Mapped {
        method: Method,
        external_ip: Option<Ipv4Addr>,
        external_port: u16,
    },
    /// Last attempt failed (retried every minute).
    Failed(String),
}

/// Keeps a UDP port mapped from a background thread: maps at start
/// ([`DEFAULT_NATPMP_LIFETIME`] / [`DEFAULT_UPNP_LEASE`]), renews at half
/// the granted lifetime (a permanent UPnP lease is not renewed), retries
/// after failures and unmaps when dropped.
pub struct PortMapper {
    /// Dropping it (disconnect) tells the thread to unmap and exit; the
    /// drop does not wait for that.
    _stop: Sender<()>,
    status: Arc<Mutex<PortMapStatus>>,
}

impl PortMapper {
    pub fn start(internal_port: u16, cfg: PortMapConfig) -> PortMapper {
        let status = Arc::new(Mutex::new(PortMapStatus::Pending));
        let (tx, rx) = mpsc::channel();
        let shared = Arc::clone(&status);
        if let Err(e) = thread::Builder::new()
            .name("tt-portmap".into())
            .spawn(move || mapper_thread(internal_port, cfg, rx, shared))
        {
            set_status(&status, PortMapStatus::Failed(format!("cannot start thread: {e}")));
        }
        PortMapper { _stop: tx, status }
    }

    pub fn status(&self) -> PortMapStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

fn set_status(status: &Mutex<PortMapStatus>, s: PortMapStatus) {
    *status.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// When to renew; `None` = never (permanent lease).
fn renew_interval(m: &Mapping) -> Option<Duration> {
    if m.lifetime.is_zero() {
        // UPnP lease 0 is permanent; a NAT-PMP lifetime of 0 is bogus, re-ask.
        return (m.method == Method::NatPmp).then_some(RETRY_INTERVAL);
    }
    Some((m.lifetime / 2).max(Duration::from_millis(100)))
}

fn mapper_thread(
    port: u16,
    cfg: PortMapConfig,
    stop: Receiver<()>,
    status: Arc<Mutex<PortMapStatus>>,
) {
    let mut mapping: Option<Mapping> = None;
    let mut wait = Some(Duration::ZERO);
    loop {
        let r = match wait {
            Some(d) => stop.recv_timeout(d),
            None => stop.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        if r != Err(RecvTimeoutError::Timeout) {
            break; // stop message or sender dropped
        }
        let result = match &mapping {
            Some(m) => renew(m, &cfg),
            None => map_with(port, DEFAULT_NATPMP_LIFETIME, DEFAULT_UPNP_LEASE, &cfg),
        };
        match result {
            Ok(m) => {
                set_status(
                    &status,
                    PortMapStatus::Mapped {
                        method: m.method,
                        external_ip: m.external_ip,
                        external_port: m.external_port,
                    },
                );
                wait = renew_interval(&m);
                mapping = Some(m);
            }
            Err(e) => {
                set_status(&status, PortMapStatus::Failed(e.to_string()));
                // Start over (fresh discovery) next time.
                mapping = None;
                wait = Some(RETRY_INTERVAL);
            }
        }
    }
    if let Some(m) = mapping {
        let _ = unmap(&m, &cfg);
    }
}

// ---------------------------------------------------------------------------
// Tests (fake gateways on loopback)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    // ---- fake NAT-PMP gateway ------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct PmpReq {
        op: u8,
        internal: u16,
        external: u16,
        lifetime: u32,
    }

    struct FakePmp {
        port: u16,
        reqs: Arc<Mutex<Vec<PmpReq>>>,
    }

    impl FakePmp {
        fn reqs(&self) -> Vec<PmpReq> {
            self.reqs.lock().unwrap().clone()
        }
    }

    /// Answers op 0 with 203.0.113.7 and op 1 with `map_result`, granting at
    /// most `max_lifetime` seconds.
    fn fake_natpmp(map_result: u16, max_lifetime: u32) -> FakePmp {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = sock.local_addr().unwrap().port();
        let reqs = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&reqs);
        thread::spawn(move || {
            let mut buf = [0u8; 64];
            loop {
                let Ok((n, from)) = sock.recv_from(&mut buf) else {
                    continue;
                };
                if n < 2 || buf[0] != 0 {
                    continue;
                }
                let epoch = 1234u32.to_be_bytes();
                match buf[1] {
                    0 => {
                        log.lock().unwrap().push(PmpReq {
                            op: 0,
                            internal: 0,
                            external: 0,
                            lifetime: 0,
                        });
                        let mut r = vec![0, 128, 0, 0];
                        r.extend_from_slice(&epoch);
                        r.extend_from_slice(&[203, 0, 113, 7]);
                        let _ = sock.send_to(&r, from);
                    }
                    1 if n >= 12 => {
                        let internal = u16::from_be_bytes([buf[4], buf[5]]);
                        let external = u16::from_be_bytes([buf[6], buf[7]]);
                        let lifetime = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
                        log.lock().unwrap().push(PmpReq {
                            op: 1,
                            internal,
                            external,
                            lifetime,
                        });
                        let (ext, life) = if lifetime == 0 {
                            (0, 0)
                        } else if external == 0 {
                            (internal, lifetime.min(max_lifetime))
                        } else {
                            (external, lifetime.min(max_lifetime))
                        };
                        let mut r = vec![0, 129];
                        r.extend_from_slice(&map_result.to_be_bytes());
                        r.extend_from_slice(&epoch);
                        r.extend_from_slice(&internal.to_be_bytes());
                        r.extend_from_slice(&ext.to_be_bytes());
                        r.extend_from_slice(&life.to_be_bytes());
                        let _ = sock.send_to(&r, from);
                    }
                    _ => {}
                }
            }
        });
        FakePmp { port, reqs }
    }

    fn natpmp_cfg(port: u16) -> PortMapConfig {
        PortMapConfig {
            timeout: Duration::from_millis(1000),
            gateway: Some(Ipv4Addr::LOCALHOST),
            natpmp_port: port,
            try_upnp: false,
            ..PortMapConfig::default()
        }
    }

    /// A bound UDP socket that never answers (no ICMP refusal either).
    fn silent_udp() -> UdpSocket {
        UdpSocket::bind("127.0.0.1:0").unwrap()
    }

    // ---- fake UPnP IGD -------------------------------------------------

    #[derive(Clone, Default)]
    struct IgdOpts {
        url_base: bool,
        chunked: bool,
        add_fault: Option<u16>,
    }

    #[derive(Debug, Clone)]
    struct HttpReq {
        method: String,
        path: String,
        action: String,
        body: String,
    }

    struct FakeIgd {
        ssdp: SocketAddr,
        reqs: Arc<Mutex<Vec<HttpReq>>>,
    }

    impl FakeIgd {
        fn soap(&self, action: &str) -> Vec<HttpReq> {
            self.reqs
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.action == action)
                .cloned()
                .collect()
        }
    }

    const EXT_IP: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 9);

    fn fake_igd(opts: IgdOpts) -> FakeIgd {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let http_port = listener.local_addr().unwrap().port();
        let (url_base, control, control_path) = if opts.url_base {
            (
                format!("<URLBase>http://127.0.0.1:{http_port}/igd/</URLBase>"),
                "ctl/IPConn",
                "/igd/ctl/IPConn",
            )
        } else {
            (String::new(), "/ctl/IPConn", "/ctl/IPConn")
        };
        let desc = format!(
            "<?xml version=\"1.0\"?>\n<root xmlns=\"urn:schemas-upnp-org:device-1-0\">\
             <specVersion><major>1</major><minor>0</minor></specVersion>{url_base}\
             <device><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>\
             <serviceList><service>\
             <serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType>\
             <controlURL>/l3f</controlURL></service></serviceList>\
             <deviceList><device><deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType>\
             <deviceList><device>\
             <deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType>\
             <serviceList><service>\
             <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>\
             <serviceId>urn:upnp-org:serviceId:WANIPConn1</serviceId>\
             <controlURL>{control}</controlURL><eventSubURL>/evt</eventSubURL>\
             <SCPDURL>/scpd.xml</SCPDURL></service></serviceList>\
             </device></deviceList></device></deviceList></device></root>\n"
        );

        let reqs = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&reqs);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Some(req) = read_request(&mut stream) else {
                    continue;
                };
                log.lock().unwrap().push(req.clone());
                let (status, body) = if req.method == "GET" && req.path == "/rootDesc.xml" {
                    (200, desc.clone())
                } else if req.method == "POST" && req.path == control_path {
                    match req.action.as_str() {
                        "AddPortMapping" => match opts.add_fault {
                            Some(725) if req.body.contains("<NewLeaseDuration>0<") => {
                                (200, soap_ok("AddPortMapping", ""))
                            }
                            Some(code) => (500, soap_fault(code)),
                            None => (200, soap_ok("AddPortMapping", "")),
                        },
                        "GetExternalIPAddress" => (
                            200,
                            soap_ok(
                                "GetExternalIPAddress",
                                &format!("<NewExternalIPAddress>{EXT_IP}</NewExternalIPAddress>"),
                            ),
                        ),
                        "DeletePortMapping" => (200, soap_ok("DeletePortMapping", "")),
                        _ => (500, soap_fault(401)),
                    }
                } else {
                    (404, String::new())
                };
                let mut resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: text/xml\r\nConnection: close\r\n"
                );
                if opts.chunked {
                    resp.push_str("Transfer-Encoding: chunked\r\n\r\n");
                    for part in body.as_bytes().chunks(37) {
                        resp.push_str(&format!("{:x};ext=1\r\n", part.len()));
                        resp.push_str(std::str::from_utf8(part).unwrap());
                        resp.push_str("\r\n");
                    }
                    resp.push_str("0\r\n\r\n");
                } else {
                    resp.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
                }
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let ssdp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let ssdp_addr = ssdp.local_addr().unwrap();
        thread::spawn(move || {
            let mut buf = [0u8; 2048];
            loop {
                let Ok((n, from)) = ssdp.recv_from(&mut buf) else {
                    continue;
                };
                if !buf[..n].starts_with(b"M-SEARCH * HTTP/1.1") {
                    continue;
                }
                let st = header_value(std::str::from_utf8(&buf[..n]).unwrap(), "st")
                    .unwrap_or("")
                    .to_string();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nst: {st}\r\n\
                     location: http://127.0.0.1:{http_port}/rootDesc.xml\r\nSERVER: fake\r\n\r\n"
                );
                let _ = ssdp.send_to(reply.as_bytes(), from);
            }
        });
        FakeIgd {
            ssdp: ssdp_addr,
            reqs,
        }
    }

    fn read_request(stream: &mut TcpStream) -> Option<HttpReq> {
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let hdr_end = loop {
            if let Some(i) = find_bytes(&buf, b"\r\n\r\n") {
                break i;
            }
            let n = stream.read(&mut chunk).ok().filter(|&n| n > 0)?;
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..hdr_end]).into_owned();
        let len: usize = header_value(&head, "content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        while buf.len() < hdr_end + 4 + len {
            let n = stream.read(&mut chunk).ok().filter(|&n| n > 0)?;
            buf.extend_from_slice(&chunk[..n]);
        }
        let mut first = head.lines().next()?.split_whitespace();
        let method = first.next()?.to_string();
        let path = first.next()?.to_string();
        let action = header_value(&head, "soapaction")
            .and_then(|v| v.trim_matches('"').rsplit('#').next())
            .unwrap_or("")
            .to_string();
        let body = String::from_utf8_lossy(&buf[hdr_end + 4..hdr_end + 4 + len]).into_owned();
        Some(HttpReq {
            method,
            path,
            action,
            body,
        })
    }

    fn soap_ok(action: &str, inner: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
             <s:Body><u:{action}Response xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">\
             {inner}</u:{action}Response></s:Body></s:Envelope>"
        )
    }

    fn soap_fault(code: u16) -> String {
        format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
             <s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring>\
             <detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\">\
             <errorCode>{code}</errorCode><errorDescription>{}</errorDescription>\
             </UPnPError></detail></s:Fault></s:Body></s:Envelope>",
            if code == 718 { "ConflictInMappingEntry" } else { "" }
        )
    }

    fn upnp_cfg(igd: &FakeIgd) -> PortMapConfig {
        PortMapConfig {
            timeout: Duration::from_millis(1000),
            ssdp_addr: igd.ssdp,
            try_natpmp: false,
            ..PortMapConfig::default()
        }
    }

    // ---- NAT-PMP -------------------------------------------------------

    #[test]
    fn natpmp_map_renew_unmap() {
        let gw = fake_natpmp(0, u32::MAX);
        let cfg = natpmp_cfg(gw.port);
        let m = map_with(28000, Duration::from_secs(7200), Duration::from_secs(7200), &cfg).unwrap();
        assert_eq!(m.method, Method::NatPmp);
        assert_eq!(m.internal_port, 28000);
        assert_eq!(m.external_port, 28000);
        assert_eq!(m.external_ip, Some(Ipv4Addr::new(203, 0, 113, 7)));
        assert_eq!(m.lifetime, Duration::from_secs(7200));
        let reqs = gw.reqs();
        assert_eq!(reqs[0].op, 0);
        assert_eq!(
            reqs[1],
            PmpReq {
                op: 1,
                internal: 28000,
                external: 28000,
                lifetime: 7200
            }
        );

        let r = renew(&m, &cfg).unwrap();
        assert_eq!(r.external_port, 28000);
        assert!(
            gw.reqs()
                .iter()
                .filter(|q| q.op == 1 && q.lifetime == 7200)
                .count()
                >= 2
        );

        unmap(&r, &cfg).unwrap();
        assert_eq!(
            *gw.reqs().last().unwrap(),
            PmpReq {
                op: 1,
                internal: 28000,
                external: 0,
                lifetime: 0
            }
        );
    }

    #[test]
    fn natpmp_result_code_is_refused() {
        let gw = fake_natpmp(2, u32::MAX);
        let err = map_with(28000, Duration::from_secs(60), Duration::from_secs(60), &natpmp_cfg(gw.port)).unwrap_err();
        match &err {
            PortMapError::Refused(msg) => assert!(msg.contains("result 2"), "{msg}"),
            e => panic!("unexpected {e:?}"),
        }
        assert!(err.to_string().contains("not authorized"));
    }

    #[test]
    fn nothing_answering_times_out() {
        let pmp = silent_udp();
        let ssdp = silent_udp();
        let cfg = PortMapConfig {
            timeout: Duration::from_millis(300),
            gateway: Some(Ipv4Addr::LOCALHOST),
            natpmp_port: pmp.local_addr().unwrap().port(),
            ssdp_addr: ssdp.local_addr().unwrap(),
            ..PortMapConfig::default()
        };
        let t0 = Instant::now();
        let err = map_with(28000, Duration::from_secs(60), Duration::from_secs(60), &cfg).unwrap_err();
        let elapsed = t0.elapsed();
        // The timeout applies per method: at most 2 x 300 ms here.
        assert!(elapsed < Duration::from_millis(1000), "{elapsed:?}");
        let msg = err.to_string();
        assert!(msg.contains("NAT-PMP") && msg.contains("UPnP"), "{msg}");

        // NAT-PMP alone: exactly its timeout.
        let t0 = Instant::now();
        let err = map_with(
            28000,
            Duration::from_secs(60),
            Duration::from_secs(60),
            &PortMapConfig {
                try_upnp: false,
                ..cfg
            },
        )
        .unwrap_err();
        assert!(matches!(err, PortMapError::Timeout), "{err:?}");
        assert!(t0.elapsed() < Duration::from_millis(600));
    }

    // ---- UPnP ----------------------------------------------------------

    fn check_add_body(body: &str, lease: u32) {
        for want in [
            "<NewRemoteHost></NewRemoteHost>".to_string(),
            "<NewExternalPort>28000</NewExternalPort>".to_string(),
            "<NewProtocol>UDP</NewProtocol>".to_string(),
            "<NewInternalPort>28000</NewInternalPort>".to_string(),
            "<NewInternalClient>127.0.0.1</NewInternalClient>".to_string(),
            "<NewEnabled>1</NewEnabled>".to_string(),
            format!("<NewPortMappingDescription>{MAPPING_DESCRIPTION}</NewPortMappingDescription>"),
            format!("<NewLeaseDuration>{lease}</NewLeaseDuration>"),
            "xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\"".to_string(),
        ] {
            assert!(body.contains(&want), "missing {want} in {body}");
        }
    }

    fn upnp_roundtrip(opts: IgdOpts) {
        let igd = fake_igd(opts);
        let cfg = upnp_cfg(&igd);
        let m = map_with(28000, Duration::from_secs(3600), Duration::from_secs(3600), &cfg).unwrap();
        assert_eq!(m.method, Method::Upnp);
        assert_eq!(m.external_port, 28000);
        assert_eq!(m.external_ip, Some(EXT_IP));
        assert_eq!(m.lifetime, Duration::from_secs(3600));
        let adds = igd.soap("AddPortMapping");
        assert_eq!(adds.len(), 1);
        check_add_body(&adds[0].body, 3600);
        assert_eq!(igd.soap("GetExternalIPAddress").len(), 1);

        unmap(&m, &cfg).unwrap();
        let dels = igd.soap("DeletePortMapping");
        assert_eq!(dels.len(), 1);
        assert!(dels[0]
            .body
            .contains("<NewExternalPort>28000</NewExternalPort>"));
        assert!(dels[0].body.contains("<NewProtocol>UDP</NewProtocol>"));
    }

    #[test]
    fn upnp_absolute_control_path() {
        upnp_roundtrip(IgdOpts::default());
    }

    #[test]
    fn upnp_relative_control_url_with_url_base() {
        upnp_roundtrip(IgdOpts {
            url_base: true,
            ..IgdOpts::default()
        });
    }

    #[test]
    fn upnp_chunked_responses() {
        upnp_roundtrip(IgdOpts {
            chunked: true,
            ..IgdOpts::default()
        });
    }

    #[test]
    fn upnp_conflict_fault_is_refused() {
        let igd = fake_igd(IgdOpts {
            add_fault: Some(718),
            ..IgdOpts::default()
        });
        let err = map_with(28000, Duration::from_secs(3600), Duration::from_secs(3600), &upnp_cfg(&igd)).unwrap_err();
        match &err {
            PortMapError::Refused(msg) => {
                assert!(msg.contains("718") && msg.contains("Conflict"), "{msg}")
            }
            e => panic!("unexpected {e:?}"),
        }
    }

    #[test]
    fn upnp_permanent_lease_retry() {
        let igd = fake_igd(IgdOpts {
            add_fault: Some(725),
            ..IgdOpts::default()
        });
        let m = map_with(28000, Duration::from_secs(3600), Duration::from_secs(3600), &upnp_cfg(&igd)).unwrap();
        assert_eq!(m.lifetime, Duration::ZERO);
        assert_eq!(renew_interval(&m), None);
        let adds = igd.soap("AddPortMapping");
        assert_eq!(adds.len(), 2);
        check_add_body(&adds[0].body, 3600);
        check_add_body(&adds[1].body, 0);
    }

    #[test]
    fn falls_back_from_natpmp_to_upnp() {
        let pmp = silent_udp();
        let igd = fake_igd(IgdOpts::default());
        let cfg = PortMapConfig {
            timeout: Duration::from_millis(300),
            gateway: Some(Ipv4Addr::LOCALHOST),
            natpmp_port: pmp.local_addr().unwrap().port(),
            ..upnp_cfg(&igd)
        };
        let cfg = PortMapConfig {
            try_natpmp: true,
            ..cfg
        };
        let m = map_with(28000, Duration::from_secs(3600), Duration::from_secs(3600), &cfg).unwrap();
        assert_eq!(m.method, Method::Upnp);
    }

    // ---- PortMapper ----------------------------------------------------

    fn wait_for(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + limit;
        while Instant::now() < end {
            if f() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        f()
    }

    #[test]
    fn port_mapper_maps_renews_and_unmaps() {
        // The gateway grants 1 s, so the renew comes after ~0.5 s.
        let gw = fake_natpmp(0, 1);
        let mapper = PortMapper::start(28001, natpmp_cfg(gw.port));
        assert!(wait_for(Duration::from_secs(1), || matches!(
            mapper.status(),
            PortMapStatus::Mapped { .. }
        )));
        assert_eq!(
            mapper.status(),
            PortMapStatus::Mapped {
                method: Method::NatPmp,
                external_ip: Some(Ipv4Addr::new(203, 0, 113, 7)),
                external_port: 28001,
            }
        );
        let maps = |gw: &FakePmp| {
            gw.reqs()
                .iter()
                .filter(|r| r.op == 1 && r.lifetime > 0)
                .count()
        };
        assert!(
            wait_for(Duration::from_millis(1500), || maps(&gw) >= 2),
            "no renew"
        );
        drop(mapper);
        let unmap = PmpReq { op: 1, internal: 28001, external: 0, lifetime: 0 };
        assert!(wait_for(Duration::from_secs(1), || gw.reqs().last() == Some(&unmap)), "no unmap");
    }

    // ---- helpers -------------------------------------------------------

    #[test]
    fn proc_net_route_default() {
        let text =
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                    wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                    eth0\t00000000\t01000A0A\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                    eth0\t00000A0A\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(
            parse_proc_net_route(text),
            Some(Ipv4Addr::new(10, 10, 0, 1))
        );
        assert_eq!(parse_proc_net_route("Iface\n"), None);
    }

    #[test]
    fn chunked_and_lengths() {
        let resp = b"HTTP/1.1 200 OK\r\ntransfer-encoding: Chunked\r\n\r\n5\r\nhello\r\n6;x=y\r\n world\r\n0\r\n\r\n";
        let (s, b) = parse_http_response(resp, false).unwrap().unwrap();
        assert_eq!((s, b.as_str()), (200, "hello world"));
        assert!(parse_http_response(&resp[..resp.len() - 8], false).is_none());
        let resp = b"HTTP/1.0 500 Internal\r\nContent-Length: 3\r\n\r\nabcdef";
        assert_eq!(
            parse_http_response(resp, false).unwrap().unwrap(),
            (500, "abc".into())
        );
        let resp = b"HTTP/1.1 200 OK\r\n\r\nuntil eof";
        assert!(parse_http_response(resp, false).is_none());
        assert_eq!(
            parse_http_response(resp, true).unwrap().unwrap().1,
            "until eof"
        );
    }

    #[test]
    fn url_join() {
        let base = Url::parse("http://192.168.1.1:5000/rootDesc.xml").unwrap();
        assert_eq!(
            base.join("/ctl/IPConn").unwrap().to_string(),
            "http://192.168.1.1:5000/ctl/IPConn"
        );
        assert_eq!(
            base.join("ctl/IPConn").unwrap().to_string(),
            "http://192.168.1.1:5000/ctl/IPConn"
        );
        let base = Url::parse("HTTP://10.0.0.1:80/igd/desc.xml").unwrap();
        assert_eq!(base.join("upnp/c").unwrap().path, "/igd/upnp/c");
        assert_eq!(base.join("http://10.0.0.2:81/x").unwrap().port, 81);
        let bare = Url::parse("http://10.0.0.1").unwrap();
        assert_eq!((bare.port, bare.path.as_str()), (80, "/"));
        assert_eq!(bare.join("ctl").unwrap().path, "/ctl");
        assert!(Url::parse("https://10.0.0.1/").is_err());
    }

    #[test]
    fn xml_scanning() {
        let xml = "<a:root><A:Service><serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType>\
                   <controlURL>/ppp?a=1&amp;b=2</controlURL></A:Service><serviceList/>\
                   <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:2</serviceType>\
                   <controlURL>/ip2</controlURL></service></a:root>";
        let (st, ctl) = find_wan_service(xml).unwrap();
        assert_eq!(st, "urn:schemas-upnp-org:service:WANIPConnection:2");
        assert_eq!(ctl, "/ip2");
        let ppp_only = &xml[..xml.find("<service>").unwrap()];
        assert_eq!(find_wan_service(ppp_only).unwrap().1, "/ppp?a=1&b=2");
        assert_eq!(
            header_value("HTTP/1.1 200 OK\r\nLocation: http://x/\r\n", "LOCATION"),
            Some("http://x/")
        );
    }
}

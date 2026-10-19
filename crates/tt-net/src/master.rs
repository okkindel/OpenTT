//! Master (lobby) server protocol: internet server lists and NAT
//! introductions (docs/netcode.md §5.11).
//!
//! It does three things:
//!
//! * **Registration.** A game server sends [`MasterPacket::Heartbeat`] from
//!   its game socket every [`HEARTBEAT_INTERVAL`]. The first one gets a
//!   [`MasterPacket::HeartbeatChallenge`] with a cookie bound to the
//!   source address (no registration for spoofed sources); heartbeats with
//!   the cookie are acknowledged with the address the master saw
//!   ([`MasterPacket::HeartbeatAck`]: the server's public address). Entries
//!   expire after [`ENTRY_LIFETIME`] without a heartbeat. The interval is
//!   short on purpose: it keeps the server's NAT mapping towards the master
//!   open, which the introductions need.
//! * **Lists.** A browser sends [`MasterPacket::ListRequest`] for a page and
//!   gets [`MasterPacket::ListReply`] with up to [`LIST_PAGE`] addresses.
//! * **Introductions (UDP hole punching).** A client behind NAT that wants
//!   to reach a server behind NAT sends [`MasterPacket::IntroduceRequest`]
//!   (the server's public address) to the master; the master forwards
//!   [`MasterPacket::Introduce`] (the client's public address) to that
//!   server over its open mapping, and the server sends a few
//!   [`MasterPacket::Punch`] packets to the client. Those open the server's
//!   NAT towards the client, the client's own packets (its info query or
//!   connect request, resent every 250 ms) open the client's, and the normal
//!   handshake gets through. This works for the common "cone" NATs, not for
//!   symmetric ones (the port the master saw is not the one the client is
//!   reached on); there, port forwarding is still needed.
//!
//! Every request is padded so that no reply is larger than the request it
//! answers (no amplification); `Introduce` goes to a third party but is
//! smaller than the request and rate-limited per target by the master.
//! Packets start with a kind byte in `16..=23` (the game packets use
//! `0..=9`) and the magic `TTMS`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use crate::error::DecodeError;
use crate::packet::ByteReader;

/// Master protocol version (checked by the master).
pub const MASTER_VERSION: u16 = 1;
/// Magic after the kind byte.
pub const MASTER_MAGIC: [u8; 4] = *b"TTMS";
/// The master's default UDP port (the game uses 28000–28009).
pub const DEFAULT_MASTER_PORT: u16 = 28100;
/// Heartbeats, registrations and introduction requests are padded to this.
pub const SMALL_REQUEST_SIZE: usize = 64;
/// List requests are padded to this (a full list page is not larger).
pub const LIST_REQUEST_SIZE: usize = crate::packet::MAX_PACKET_SIZE;
/// Addresses per list page (19 B each worst case: 60 × 19 + 24 < 1200).
pub const LIST_PAGE: usize = 60;
/// A registered server heartbeats this often (also keeps its NAT open).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
/// Without a heartbeat for this long a server leaves the list.
pub const ENTRY_LIFETIME: Duration = Duration::from_secs(70);
/// Punch packets a server sends per introduction.
pub const PUNCHES: usize = 3;

const KIND_HEARTBEAT: u8 = 16;
const KIND_CHALLENGE: u8 = 17;
const KIND_ACK: u8 = 18;
const KIND_LIST_REQUEST: u8 = 19;
const KIND_LIST_REPLY: u8 = 20;
const KIND_INTRODUCE_REQUEST: u8 = 21;
const KIND_INTRODUCE: u8 = 22;
const KIND_PUNCH: u8 = 23;

/// A master-protocol datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MasterPacket {
    /// Server → master. `cookie` = 0 until challenged; `server_id` is the
    /// server's random instance id (the `ServerInfo` one); `closing` takes it
    /// off the list at once.
    Heartbeat { version: u16, nonce: u64, cookie: u64, server_id: u64, closing: bool },
    /// Master → server: repeat the heartbeat with this cookie.
    HeartbeatChallenge { nonce: u64, cookie: u64 },
    /// Master → server: registered; `public` is the source address seen.
    HeartbeatAck { nonce: u64, public: SocketAddr },
    /// Browser → master: page `page` of the list.
    ListRequest { version: u16, nonce: u64, page: u8 },
    /// Master → browser.
    ListReply { nonce: u64, page: u8, pages: u8, servers: Vec<SocketAddr> },
    /// Client → master: introduce me to `server` (a listed address).
    IntroduceRequest { version: u16, nonce: u64, server: SocketAddr },
    /// Master → server: `client` wants to reach you; punch towards it.
    Introduce { client: SocketAddr },
    /// Server → client (or anyone): opens a NAT mapping; ignored on receipt.
    Punch { nonce: u64 },
}

/// True if `data` is a master-protocol packet (kind byte and magic).
pub fn is_master_packet(data: &[u8]) -> bool {
    data.len() >= 5 && (KIND_HEARTBEAT..=KIND_PUNCH).contains(&data[0]) && data[1..5] == MASTER_MAGIC
}

fn put_addr(out: &mut Vec<u8>, a: &SocketAddr) {
    match a.ip() {
        IpAddr::V4(ip) => {
            out.push(4);
            out.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            out.push(6);
            out.extend_from_slice(&ip.octets());
        }
    }
    out.extend_from_slice(&a.port().to_le_bytes());
}

impl ByteReader<'_> {
    fn addr(&mut self) -> Result<SocketAddr, DecodeError> {
        let ip = match self.u8()? {
            4 => {
                let b = self.take(4)?;
                IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3]))
            }
            6 => {
                let mut a = [0u8; 16];
                a.copy_from_slice(self.take(16)?);
                IpAddr::V6(Ipv6Addr::from(a))
            }
            _ => return Err(DecodeError::Invalid("address family")),
        };
        Ok(SocketAddr::new(ip, self.u16()?))
    }
}

impl MasterPacket {
    fn kind(&self) -> u8 {
        match self {
            MasterPacket::Heartbeat { .. } => KIND_HEARTBEAT,
            MasterPacket::HeartbeatChallenge { .. } => KIND_CHALLENGE,
            MasterPacket::HeartbeatAck { .. } => KIND_ACK,
            MasterPacket::ListRequest { .. } => KIND_LIST_REQUEST,
            MasterPacket::ListReply { .. } => KIND_LIST_REPLY,
            MasterPacket::IntroduceRequest { .. } => KIND_INTRODUCE_REQUEST,
            MasterPacket::Introduce { .. } => KIND_INTRODUCE,
            MasterPacket::Punch { .. } => KIND_PUNCH,
        }
    }

    /// The datagram (requests padded, list replies at most [`LIST_PAGE`] entries).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.kind()];
        out.extend_from_slice(&MASTER_MAGIC);
        match self {
            MasterPacket::Heartbeat { version, nonce, cookie, server_id, closing } => {
                out.extend_from_slice(&version.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&cookie.to_le_bytes());
                out.extend_from_slice(&server_id.to_le_bytes());
                out.push(*closing as u8);
                out.resize(SMALL_REQUEST_SIZE, 0);
            }
            MasterPacket::HeartbeatChallenge { nonce, cookie } => {
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&cookie.to_le_bytes());
            }
            MasterPacket::HeartbeatAck { nonce, public } => {
                out.extend_from_slice(&nonce.to_le_bytes());
                put_addr(&mut out, public);
            }
            MasterPacket::ListRequest { version, nonce, page } => {
                out.extend_from_slice(&version.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
                out.push(*page);
                out.resize(LIST_REQUEST_SIZE, 0);
            }
            MasterPacket::ListReply { nonce, page, pages, servers } => {
                out.extend_from_slice(&nonce.to_le_bytes());
                out.push(*page);
                out.push(*pages);
                let n = servers.len().min(LIST_PAGE);
                out.push(n as u8);
                for a in &servers[..n] {
                    put_addr(&mut out, a);
                }
            }
            MasterPacket::IntroduceRequest { version, nonce, server } => {
                out.extend_from_slice(&version.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
                put_addr(&mut out, server);
                out.resize(SMALL_REQUEST_SIZE, 0);
            }
            MasterPacket::Introduce { client } => put_addr(&mut out, client),
            MasterPacket::Punch { nonce } => out.extend_from_slice(&nonce.to_le_bytes()),
        }
        out
    }

    /// Decodes a datagram; never panics. Requests that lack their padding are refused.
    pub fn decode(data: &[u8]) -> Result<MasterPacket, DecodeError> {
        if data.len() > crate::packet::MAX_PACKET_SIZE {
            return Err(DecodeError::TooLarge);
        }
        if !is_master_packet(data) {
            return Err(DecodeError::BadMagic);
        }
        let kind = data[0];
        let need = match kind {
            KIND_HEARTBEAT | KIND_INTRODUCE_REQUEST => SMALL_REQUEST_SIZE,
            KIND_LIST_REQUEST => LIST_REQUEST_SIZE,
            _ => 0,
        };
        if data.len() < need {
            return Err(DecodeError::TooSmall);
        }
        let mut r = ByteReader::new(data, 5);
        Ok(match kind {
            KIND_HEARTBEAT => MasterPacket::Heartbeat { version: r.u16()?, nonce: r.u64()?, cookie: r.u64()?, server_id: r.u64()?, closing: r.u8()? != 0 },
            KIND_CHALLENGE => MasterPacket::HeartbeatChallenge { nonce: r.u64()?, cookie: r.u64()? },
            KIND_ACK => MasterPacket::HeartbeatAck { nonce: r.u64()?, public: r.addr()? },
            KIND_LIST_REQUEST => MasterPacket::ListRequest { version: r.u16()?, nonce: r.u64()?, page: r.u8()? },
            KIND_LIST_REPLY => {
                let nonce = r.u64()?;
                let page = r.u8()?;
                let pages = r.u8()?;
                let n = r.u8()? as usize;
                if n > LIST_PAGE {
                    return Err(DecodeError::TooLarge);
                }
                let mut servers = Vec::with_capacity(n);
                for _ in 0..n {
                    servers.push(r.addr()?);
                }
                MasterPacket::ListReply { nonce, page, pages, servers }
            }
            KIND_INTRODUCE_REQUEST => MasterPacket::IntroduceRequest { version: r.u16()?, nonce: r.u64()?, server: r.addr()? },
            KIND_INTRODUCE => MasterPacket::Introduce { client: r.addr()? },
            KIND_PUNCH => MasterPacket::Punch { nonce: r.u64()? },
            k => return Err(DecodeError::BadKind(k)),
        })
    }
}

/// Is `ip` a public internet address (worth an introduction through the
/// master)? Loopback, private, link-local, CGNAT-shared and unspecified
/// addresses are reached directly.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_loopback() || v.is_private() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || (o[0] == 100 && (o[1] & 0xc0) == 64))
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            !(v.is_loopback() || v.is_unspecified() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80)
        }
    }
}

/// Registration state of a game server with its master (kept by [`crate::Server`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MasterStatus {
    pub master: Option<SocketAddr>,
    /// The master acknowledged a heartbeat.
    pub registered: bool,
    /// Our address as the master sees it (the public one behind NAT).
    pub public_addr: Option<SocketAddr>,
    /// Introductions received (clients that punched through).
    pub introductions: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_padding() {
        let v4: SocketAddr = "203.0.113.7:28000".parse().unwrap();
        let v6: SocketAddr = "[2001:db8::1]:28001".parse().unwrap();
        let all = [
            MasterPacket::Heartbeat { version: 1, nonce: 2, cookie: 3, server_id: 4, closing: true },
            MasterPacket::HeartbeatChallenge { nonce: 5, cookie: 6 },
            MasterPacket::HeartbeatAck { nonce: 7, public: v6 },
            MasterPacket::ListRequest { version: 1, nonce: 8, page: 2 },
            MasterPacket::ListReply { nonce: 9, page: 0, pages: 1, servers: vec![v4, v6] },
            MasterPacket::IntroduceRequest { version: 1, nonce: 10, server: v4 },
            MasterPacket::Introduce { client: v6 },
            MasterPacket::Punch { nonce: 11 },
        ];
        for p in &all {
            let b = p.encode();
            assert!(is_master_packet(&b));
            assert_eq!(&MasterPacket::decode(&b).unwrap(), p);
            for n in 0..b.len() {
                let _ = MasterPacket::decode(&b[..n]);
            }
        }
        // Replies never exceed their requests.
        let full = MasterPacket::ListReply { nonce: 1, page: 0, pages: 1, servers: vec![v6; LIST_PAGE] }.encode();
        assert!(full.len() <= LIST_REQUEST_SIZE, "{}", full.len());
        assert!(MasterPacket::HeartbeatAck { nonce: 1, public: v6 }.encode().len() <= SMALL_REQUEST_SIZE);
        assert!(MasterPacket::Introduce { client: v6 }.encode().len() < SMALL_REQUEST_SIZE);
        // Unpadded requests are refused.
        let mut hb = all[0].encode();
        hb.truncate(40);
        assert_eq!(MasterPacket::decode(&hb), Err(DecodeError::TooSmall));
        assert!(!is_master_packet(&crate::packet::Packet::InfoQuery { protocol: 1, nonce: 1 }.encode()));
    }

    #[test]
    fn public_addresses() {
        for (a, p) in [("8.8.8.8", true), ("192.168.1.2", false), ("10.0.0.1", false), ("127.0.0.1", false), ("100.64.1.1", false), ("172.16.0.1", false), ("2001:db8::1", true), ("fe80::1", false), ("fd00::1", false), ("::1", false)] {
            assert_eq!(is_public(a.parse().unwrap()), p, "{a}");
        }
    }
}

//! LAN server discovery: the server-info payload of `Info` replies ([`ServerInfo`])
//! and the querying side ([`LanBrowser`]).
//!
//! A browser sends `InfoQuery` packets (padded to
//! [`INFO_QUERY_SIZE`](crate::packet::INFO_QUERY_SIZE), so no reply is ever
//! larger) to the broadcast address and to loopback on every port of the
//! server range (`28000..=28009`, what `portInit` tries), plus to any unicast
//! address the player typed. Servers answer with an `Info` packet carrying
//! [`ServerInfo::encode`]; [`crate::Server`] does that statelessly from
//! [`crate::Server::set_info`]. Broadcast is not available everywhere
//! (containers, some CI networks, Windows firewalls); the loopback and
//! unicast targets still work there.
//!
//! The same server can answer twice (to the broadcast from its LAN address
//! and to the loopback query from 127.0.0.1); replies are merged by the
//! random [`ServerInfo::server_id`], preferring the non-loopback address.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use crate::error::DecodeError;
use crate::master::{MasterPacket, MASTER_VERSION};
use crate::packet::{Packet, MAX_INFO_PAYLOAD, MAX_PACKET_SIZE};
use crate::rng::Rng;
use crate::transport::{Datagram, UdpTransport};
use crate::{BUILD_ID, DEFAULT_PORT, PROTOCOL_VERSION};

/// Ports a server may have bound (`portInit` tries 10 upwards).
pub const PORT_RANGE: u16 = 10;
/// Version of the [`ServerInfo`] layout (first payload byte). Format 2
/// appends the player scores, format 3 the gameplay mods (before the
/// player list); format 1 and 2 replies still decode. A format 3 reply may
/// end with the server's version and protocol range (older browsers ignore
/// the trailing bytes, older servers do not send them).
pub const INFO_FORMAT: u8 = 3;

/// A mod that changes the game, as a server announces it
/// (docs/modding.md § Multiplayer): every player needs the same one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModRef {
    pub name: String,
    pub version: String,
    /// The mod's gameplay files' hash (`tt_assets::mods::ModInfo::gameplay_hash`).
    pub hash: u64,
}

impl ModRef {
    /// "name version" (or the name alone).
    pub fn label(&self) -> String {
        if self.version.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.version)
        }
    }
}

/// Most bytes of an `Info` reply the mod list takes (the rest is the
/// player list's).
const MAX_MODS_BYTES: usize = 140;

/// What a server tells a browser.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerInfo {
    /// Random per server instance; merges replies that came over several routes.
    pub server_id: u64,
    /// The server's build id ([`BUILD_ID`]); joining needs the same.
    pub build: u64,
    pub game_type: String,
    pub name: String,
    /// "Battlemode", "Team Scrum"....
    pub mission_type: String,
    /// Platform TAB world TAB mission name TAB 0.
    pub info: String,
    pub mission_file: String,
    /// Team letter TAB name pairs, one per player.
    pub players: String,
    /// Humans plus bots.
    pub player_count: u8,
    pub max_players: u8,
    pub bot_count: u8,
    /// A join password is set.
    pub password: bool,
    pub dedicated: bool,
    /// MHz; `getServerScore` uses it.
    pub cpu_mhz: u16,
    /// Score of each listed player, in [`Self::players`] order (format 2;
    /// may be shorter than the list: missing scores are 0).
    pub scores: Vec<i16>,
    /// The server runs mods that change the game (format 3).
    pub modded: bool,
    /// Those mods, as many as fit the reply (format 3; joining checks them
    /// all, `docs/modding.md`).
    pub mods: Vec<ModRef>,
    /// The server's game version ([`crate::GAME_VERSION`]; empty from
    /// servers that do not send it).
    pub version: String,
    /// The wire protocols it accepts (lowest, highest); (0, 0) when not
    /// sent: then only the `Info` header's protocol.
    pub protocols: (u16, u16),
}

fn field(s: &str, i: usize) -> &str {
    s.split('\t').nth(i).unwrap_or("")
}

impl ServerInfo {
    /// Platform ("Windows", "Linux", ...): field 0 of [`Self::info`].
    pub fn platform(&self) -> &str {
        field(&self.info, 0)
    }

    /// World number (1–3, 0 for custom maps): field 1 of [`Self::info`].
    pub fn world(&self) -> &str {
        field(&self.info, 1)
    }

    /// The mission's display name: field 2 of [`Self::info`].
    pub fn map(&self) -> &str {
        field(&self.info, 2)
    }

    /// (team letter, name) of every listed player.
    pub fn player_list(&self) -> Vec<(String, String)> {
        if self.players.is_empty() {
            return Vec::new();
        }
        let f: Vec<&str> = self.players.split('\t').collect();
        f.chunks(2).map(|c| (c[0].to_string(), c.get(1).copied().unwrap_or("").to_string())).collect()
    }

    /// (team letter, name, score) of every listed player.
    pub fn player_scores(&self) -> Vec<(String, String, i32)> {
        self.player_list().into_iter().enumerate().map(|(i, (t, n))| (t, n, self.scores.get(i).copied().unwrap_or(0) as i32)).collect()
    }

    /// The bytes of an `Info` reply (at most [`MAX_INFO_PAYLOAD`]): strings
    /// are clipped, and the player list keeps only whole pairs that fit.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![INFO_FORMAT];
        out.extend_from_slice(&self.server_id.to_le_bytes());
        out.extend_from_slice(&self.build.to_le_bytes());
        let modded = self.modded || !self.mods.is_empty();
        out.extend_from_slice(&[self.player_count, self.max_players, self.bot_count, self.password as u8 | (self.dedicated as u8) << 1 | (modded as u8) << 2]);
        out.extend_from_slice(&self.cpu_mhz.to_le_bytes());
        for (s, max) in [(&self.game_type, 24), (&self.name, 48), (&self.mission_type, 32), (&self.info, 64), (&self.mission_file, 64)] {
            put_str(&mut out, s, max);
        }
        // The mods that fit their share: count byte, then name, version, hash.
        let count_at = out.len();
        out.push(0);
        let mut n = 0u8;
        for m in &self.mods {
            let mut one = Vec::new();
            put_str(&mut one, &m.name, 24);
            put_str(&mut one, &m.version, 12);
            one.extend_from_slice(&m.hash.to_le_bytes());
            if out.len() - count_at + one.len() > MAX_MODS_BYTES || n == u8::MAX {
                break;
            }
            out.extend_from_slice(&one);
            n += 1;
        }
        out[count_at] = n;
        // The version tail goes last (when known); the list's length byte,
        // then the score count byte.
        let mut tail = Vec::new();
        if !self.version.is_empty() || self.protocols != (0, 0) {
            put_str(&mut tail, &self.version, 16);
            tail.extend_from_slice(&self.protocols.0.to_le_bytes());
            tail.extend_from_slice(&self.protocols.1.to_le_bytes());
        }
        let room = MAX_INFO_PAYLOAD.saturating_sub(out.len() + 2 + tail.len());
        let mut players = String::new();
        let mut kept = 0usize;
        for (team, name) in self.player_list() {
            let pair = format!("{}{team}\t{name}", if players.is_empty() { "" } else { "\t" });
            // Each kept pair also costs its 2-byte score.
            if players.len() + pair.len() > 255 || players.len() + pair.len() + 2 * (kept + 1) > room {
                break;
            }
            players.push_str(&pair);
            kept += 1;
        }
        put_str(&mut out, &players, 255);
        out.push(kept as u8);
        for i in 0..kept {
            out.extend_from_slice(&self.scores.get(i).copied().unwrap_or(0).to_le_bytes());
        }
        out.extend_from_slice(&tail);
        debug_assert!(out.len() <= MAX_INFO_PAYLOAD);
        out
    }

    pub fn decode(b: &[u8]) -> Result<ServerInfo, DecodeError> {
        let mut r = crate::packet::ByteReader::new(b, 0);
        let format = r.u8()?;
        if !(1..=INFO_FORMAT).contains(&format) {
            return Err(DecodeError::Invalid("server info format"));
        }
        let server_id = r.u64()?;
        let build = r.u64()?;
        let (player_count, max_players, bot_count, flags) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let cpu_mhz = r.u16()?;
        let mut info = ServerInfo {
            server_id,
            build,
            player_count,
            max_players,
            bot_count,
            password: flags & 1 != 0,
            dedicated: flags & 2 != 0,
            modded: flags & 4 != 0,
            cpu_mhz,
            game_type: r.str()?,
            name: r.str()?,
            mission_type: r.str()?,
            info: r.str()?,
            mission_file: r.str()?,
            ..Default::default()
        };
        if format >= 3 {
            let n = r.u8()?;
            for _ in 0..n {
                info.mods.push(ModRef { name: r.str()?, version: r.str()?, hash: r.u64()? });
            }
        }
        info.players = r.str()?;
        if format >= 2 {
            let n = r.u8()? as usize;
            for _ in 0..n {
                info.scores.push(r.u16()? as i16);
            }
        }
        if !r.rest().is_empty() {
            info.version = r.str()?;
            info.protocols = (r.u16()?, r.u16()?);
        }
        Ok(info)
    }
}

/// A string clipped to `max` bytes (at a char boundary), u8 length prefix.
fn put_str(out: &mut Vec<u8>, s: &str, max: usize) {
    let mut end = s.len().min(max).min(255);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    out.push(end as u8);
    out.extend_from_slice(&s.as_bytes()[..end]);
}

/// One server found by a [`LanBrowser`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanServer {
    /// Where to connect (the address the reply came from).
    pub addr: SocketAddr,
    pub info: ServerInfo,
    /// Wire protocol of the server (`Info` header).
    pub protocol: u16,
    /// Query round trip (the best seen).
    pub ping: Duration,
    /// When the last reply arrived.
    pub seen: Duration,
}

impl LanServer {
    /// The server accepts our protocol and runs our build: joining can work.
    pub fn compatible(&self) -> bool {
        let (lo, hi) = self.info.protocols;
        let protocol = if hi == 0 { self.protocol == PROTOCOL_VERSION } else { (lo..=hi).contains(&PROTOCOL_VERSION) };
        protocol && self.info.build == BUILD_ID
    }
}

/// The querying side of LAN discovery, on its own socket (no connection).
pub struct LanBrowser<T: Datagram> {
    transport: T,
    rng: Rng,
    /// (nonce, sent at) of queries that may still be answered.
    pending: Vec<(u64, Duration)>,
    servers: Vec<LanServer>,
    /// Extra broadcast addresses (directed subnet broadcasts) to query.
    pub extra_broadcast: Vec<Ipv4Addr>,
    /// Query 127.0.0.1 as well (servers on this machine without broadcast).
    pub loopback: bool,
    /// The master servers of the current internet query: (address, list
    /// nonce, list pages received).
    masters: Vec<(SocketAddr, u64, u8)>,
    /// Servers the masters listed, without duplicates (queried for their info).
    master_list: Vec<SocketAddr>,
}

/// How long a query nonce is accepted.
const QUERY_LIFETIME: Duration = Duration::from_secs(5);

impl LanBrowser<UdpTransport> {
    /// A real UDP browser: an ephemeral IPv4 socket with broadcast on (when
    /// the OS allows it) and the guessed directed broadcast of the primary
    /// interface as an extra target.
    pub fn udp() -> std::io::Result<Self> {
        let mut b = LanBrowser::new(udp_transport()?, Rng::from_entropy().next_u64());
        b.extra_broadcast.extend(guess_subnet_broadcast());
        Ok(b)
    }
}

/// The browser socket: ephemeral IPv4 (broadcast needs an IPv4 socket),
/// `SO_BROADCAST` on where the OS allows it.
pub fn udp_transport() -> std::io::Result<UdpTransport> {
    let t = UdpTransport::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))?;
    let _ = t.set_broadcast(true);
    Ok(t)
}

/// The /24 broadcast of the interface that routes to the internet, found
/// without sending anything (a connected UDP socket's local address); std
/// cannot enumerate interfaces or their netmasks.
pub fn guess_subnet_broadcast() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => {
            let o = ip.octets();
            Some(Ipv4Addr::new(o[0], o[1], o[2], 255))
        }
        _ => None,
    }
}

impl<T: Datagram> LanBrowser<T> {
    pub fn new(transport: T, seed: u64) -> Self {
        LanBrowser { transport, rng: Rng::new(seed), pending: Vec::new(), servers: Vec::new(), extra_broadcast: Vec::new(), loopback: true, masters: Vec::new(), master_list: Vec::new() }
    }

    /// Asks master servers for their lists (page by page); every listed
    /// server (once, however many masters list it) is then queried for its
    /// info, with an introduction through the master that listed it so
    /// servers behind NAT can answer (hole punching). A master that does
    /// not answer only lists nothing.
    pub fn query_master(&mut self, masters: &[SocketAddr], now: Duration) {
        self.masters = masters.iter().map(|&m| (m, self.rng.next_u64(), 0)).collect();
        self.master_list.clear();
        for i in 0..self.masters.len() {
            self.request_page(i, 0);
        }
        let _ = now;
    }

    fn request_page(&mut self, i: usize, page: u8) {
        let (m, nonce, _) = self.masters[i];
        let p = MasterPacket::ListRequest { version: MASTER_VERSION, nonce, page };
        let _ = self.transport.send_to(&p.encode(), m);
    }

    /// Repeats the info queries (and the list request while nothing came)
    /// of the current internet query: lost packets, and the first query to
    /// a server behind NAT is often dropped before its punch arrives.
    pub fn resend_master(&mut self, now: Duration) {
        if self.masters.is_empty() {
            return;
        }
        for i in 0..self.masters.len() {
            if self.masters[i].2 == 0 {
                self.request_page(i, 0);
            }
        }
        let list = self.master_list.clone();
        if !list.is_empty() {
            self.query(&list, now);
        }
    }

    /// Servers the masters listed in the current internet query (each once).
    pub fn master_list(&self) -> &[SocketAddr] {
        &self.master_list
    }

    fn on_master(&mut self, data: &[u8], from: SocketAddr, now: Duration) {
        let Ok(MasterPacket::ListReply { nonce: n, page, pages, servers }) = MasterPacket::decode(data) else { return };
        let Some(i) = self.masters.iter().position(|&(m, nonce, _)| m == from && nonce == n) else { return };
        self.masters[i].2 = self.masters[i].2.max(page.saturating_add(1));
        let new: Vec<SocketAddr> = servers.into_iter().filter(|a| !self.master_list.contains(a)).collect();
        self.master_list.extend(new.iter().copied());
        for &s in &new {
            if crate::master::is_public(s.ip()) {
                let p = MasterPacket::IntroduceRequest { version: MASTER_VERSION, nonce: self.rng.next_u64(), server: s };
                let _ = self.transport.send_to(&p.encode(), from);
            }
        }
        if !new.is_empty() {
            self.query(&new, now);
        }
        if page.saturating_add(1) < pages {
            self.request_page(i, page + 1);
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.transport.local_addr()
    }

    /// Where a LAN query for servers on `port..port+PORT_RANGE` goes.
    pub fn lan_targets(&self, port: u16) -> Vec<SocketAddr> {
        let port = if port == 0 { DEFAULT_PORT } else { port };
        let mut ips = vec![Ipv4Addr::BROADCAST];
        for &ip in &self.extra_broadcast {
            if !ips.contains(&ip) {
                ips.push(ip);
            }
        }
        if self.loopback {
            ips.push(Ipv4Addr::LOCALHOST);
        }
        let mut out = Vec::new();
        for ip in ips {
            for p in port..port.saturating_add(PORT_RANGE) {
                out.push(SocketAddr::new(IpAddr::V4(ip), p));
            }
        }
        out
    }

    /// Queries the LAN (broadcast + loopback on the port range).
    pub fn query_lan(&mut self, port: u16, now: Duration) {
        let targets = self.lan_targets(port);
        self.query(&targets, now);
    }

    /// Sends one `InfoQuery` (one nonce) to each of `targets`. Send errors
    /// (e.g. broadcast not permitted) are ignored.
    pub fn query(&mut self, targets: &[SocketAddr], now: Duration) {
        let nonce = self.rng.next_u64();
        self.pending.retain(|(_, t)| now.saturating_sub(*t) < QUERY_LIFETIME);
        self.pending.push((nonce, now));
        let bytes = Packet::InfoQuery { protocol: PROTOCOL_VERSION, nonce }.encode();
        for &a in targets {
            let _ = self.transport.send_to(&bytes, a);
        }
    }

    /// Receives replies; returns the number of new or updated entries.
    pub fn update(&mut self, now: Duration) -> usize {
        let mut buf = [0u8; MAX_PACKET_SIZE + 1];
        let mut n = 0;
        while let Some((len, from)) = self.transport.recv_from(&mut buf) {
            if crate::master::is_master_packet(&buf[..len]) {
                let data = buf[..len].to_vec();
                self.on_master(&data, from, now);
                continue;
            }
            let Ok(Packet::Info { protocol, nonce, payload }) = Packet::decode(&buf[..len]) else { continue };
            let Some(&(_, sent)) = self.pending.iter().find(|(q, _)| *q == nonce) else { continue };
            let Ok(info) = ServerInfo::decode(&payload) else { continue };
            let ping = now.saturating_sub(sent);
            self.add(LanServer { addr: from, info, protocol, ping, seen: now });
            n += 1;
        }
        n
    }

    fn add(&mut self, s: LanServer) {
        let same = |e: &LanServer| e.addr == s.addr || (s.info.server_id != 0 && e.info.server_id == s.info.server_id);
        match self.servers.iter_mut().find(|e| same(e)) {
            Some(e) => {
                let ping = e.ping.min(s.ping);
                // A LAN address is more useful than loopback (others can use it too).
                let addr = if is_loopback(e.addr) && !is_loopback(s.addr) { s.addr } else { e.addr };
                *e = LanServer { addr, ping, ..s };
            }
            None => self.servers.push(s),
        }
    }

    /// Servers found since the last [`clear`](Self::clear), in reply order.
    pub fn servers(&self) -> &[LanServer] {
        &self.servers
    }

    pub fn clear(&mut self) {
        self.servers.clear();
    }

    /// Stops accepting replies to queries sent so far.
    pub fn cancel(&mut self) {
        self.pending.clear();
        self.masters.clear();
    }
}

fn is_loopback(a: SocketAddr) -> bool {
    a.ip().is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ServerInfo {
        ServerInfo {
            server_id: 42,
            build: BUILD_ID,
            game_type: crate::GAME_TYPE.into(),
            name: "Test Server".into(),
            mission_type: "Team Battlemode".into(),
            info: "Linux\t1\tSpider Valley\t0".into(),
            mission_file: "missions/tt1_1.ron".into(),
            players: "G\tBait\tB\tGrub [Bot]".into(),
            player_count: 2,
            max_players: 10,
            bot_count: 1,
            password: true,
            dedicated: true,
            cpu_mhz: 2400,
            scores: vec![12, -3],
            modded: true,
            mods: vec![ModRef { name: "Big Maps".into(), version: "1.0".into(), hash: 0x1234_5678_9abc_def0 }],
            ..Default::default()
        }
    }

    #[test]
    fn info_round_trip() {
        let i = sample();
        let b = i.encode();
        assert!(b.len() <= MAX_INFO_PAYLOAD);
        assert_eq!(ServerInfo::decode(&b).unwrap(), i);
        assert_eq!(i.map(), "Spider Valley");
        assert_eq!(i.world(), "1");
        assert_eq!(i.platform(), "Linux");
        assert_eq!(i.player_list(), vec![("G".into(), "Bait".into()), ("B".into(), "Grub [Bot]".into())]);
        assert_eq!(i.player_scores(), vec![("G".into(), "Bait".into(), 12), ("B".into(), "Grub [Bot]".into(), -3)]);
        // Format 1 (no scores) and 2 (no mods) replies still decode.
        let mut old = i.clone();
        old.scores.clear();
        old.mods.clear();
        old.modded = false;
        let b3 = old.encode();
        // Without the (empty) mod list: after the five strings.
        let mut at = 1 + 8 + 8 + 4 + 2;
        for _ in 0..5 {
            at += 1 + b3[at] as usize;
        }
        assert_eq!(b3[at], 0);
        let mut b2 = b3.clone();
        b2.remove(at);
        b2[0] = 2;
        // (Every listed player gets a score: 0 when none was given.)
        assert_eq!(ServerInfo::decode(&b2).unwrap(), ServerInfo { scores: vec![0, 0], ..old.clone() });
        // Format 1 ends with the player list.
        let mut b1 = b2.clone();
        b1.truncate(b1.len() - 1 - 2 * 2);
        b1[0] = 1;
        assert_eq!(ServerInfo::decode(&b1).unwrap(), old);
        // Many mods: as many as fit, the reply stays small enough.
        let mut many = i.clone();
        many.mods = (0..20).map(|k| ModRef { name: format!("A mod with a long name {k}"), version: "10.20.30".into(), hash: k }).collect();
        let enc = many.encode();
        assert!(enc.len() <= MAX_INFO_PAYLOAD);
        let d = ServerInfo::decode(&enc).unwrap();
        assert!(d.modded && d.mods.len() >= 2 && d.mods.len() < 20, "{}", d.mods.len());
        assert_eq!(d.mods[1].name, "A mod with a long name 1");
        assert!(ServerInfo::decode(&b[..b.len() - 1]).is_err());
        assert!(ServerInfo::decode(&[9]).is_err());
    }

    #[test]
    fn version_tail_is_optional_and_checked() {
        let i = ServerInfo { version: "0.1.2".into(), protocols: (10, 11), ..sample() };
        let b = i.encode();
        assert_eq!(ServerInfo::decode(&b).unwrap(), i);
        // Without the tail (older servers): the same bytes before it, no version, no range.
        let old = ServerInfo { version: String::new(), protocols: (0, 0), ..i.clone() };
        assert_eq!(b[..b.len() - 10], old.encode()[..]);
        assert_eq!(ServerInfo::decode(&b[..b.len() - 10]).unwrap(), old);
        // A full player list leaves room for it.
        let full = ServerInfo { players: (0..40).map(|k| format!("G\tA rather long player name {k}")).collect::<Vec<_>>().join("\t"), scores: (0..40).collect(), ..i.clone() };
        let enc = full.encode();
        assert!(enc.len() <= MAX_INFO_PAYLOAD);
        let d = ServerInfo::decode(&enc).unwrap();
        assert_eq!((d.version.as_str(), d.protocols), ("0.1.2", (10, 11)));
        // Joinable: our protocol in the announced range, else the header's protocol.
        let server = |protocol, protocols| LanServer { addr: "10.0.0.1:28000".parse().unwrap(), info: ServerInfo { build: BUILD_ID, protocols, ..Default::default() }, protocol, ping: Duration::ZERO, seen: Duration::ZERO };
        assert!(server(PROTOCOL_VERSION, (0, 0)).compatible());
        assert!(!server(PROTOCOL_VERSION + 1, (0, 0)).compatible());
        assert!(server(PROTOCOL_VERSION + 1, (PROTOCOL_VERSION - 1, PROTOCOL_VERSION + 1)).compatible());
        assert!(!server(PROTOCOL_VERSION, (PROTOCOL_VERSION + 1, PROTOCOL_VERSION + 2)).compatible());
    }

    #[test]
    fn long_player_lists_are_clipped_to_whole_pairs() {
        let mut i = sample();
        i.players = (0..40).map(|k| format!("G\tA rather long player name {k}")).collect::<Vec<_>>().join("\t");
        i.name = "N".repeat(300);
        i.scores = (0..40).collect();
        let b = i.encode();
        assert!(b.len() <= MAX_INFO_PAYLOAD);
        let d = ServerInfo::decode(&b).unwrap();
        assert_eq!(d.name.len(), 48);
        let list = d.player_list();
        assert!(list.len() >= 5 && list.len() < 40, "{}", list.len());
        assert!(list.iter().all(|(t, n)| t == "G" && n.starts_with("A rather long player name")));
        assert_eq!(d.scores.len(), list.len(), "a score per kept player");
        assert_eq!(d.player_scores()[3].2, 3);
        // Fits a whole Info packet.
        let p = Packet::Info { protocol: 1, nonce: 1, payload: b };
        assert!(p.encode().len() <= crate::packet::INFO_QUERY_SIZE);
    }

    #[test]
    fn targets_cover_the_port_range() {
        let net = crate::MemNet::new(1);
        let mut b = LanBrowser::new(net.endpoint("10.0.0.5:1000".parse().unwrap()), 1);
        b.extra_broadcast.push(Ipv4Addr::new(10, 0, 0, 255));
        let t = b.lan_targets(28000);
        assert_eq!(t.len(), 30);
        assert!(t.contains(&"255.255.255.255:28009".parse().unwrap()));
        assert!(t.contains(&"10.0.0.255:28000".parse().unwrap()));
        assert!(t.contains(&"127.0.0.1:28005".parse().unwrap()));
        b.loopback = false;
        assert_eq!(b.lan_targets(0).len(), 20);
    }
}

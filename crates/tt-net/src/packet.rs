//! Wire format: packet kinds, the 13-byte connection header and the
//! bit-packed data body.
//!
//! Handshake and LAN packets (`ConnectRequest`, `Challenge`,
//! `ChallengeResponse`, `Accept`, `Reject`, `InfoQuery`, `Info`) are a kind
//! byte plus fixed little-endian fields. Connection packets (`Data`,
//! `Keepalive`, `Disconnect`) carry the [`Header`] (kind, session, seq, ack,
//! ack_bits = 13 B) and then their body.

use crate::bits::{varint_bits, BitReader, BitWriter};
use crate::error::DecodeError;

/// Maximum datagram payload (never fragments on IPv4/IPv6, §3.2).
pub const MAX_PACKET_SIZE: usize = 1200;
/// Kind + session + seq + ack + ack_bits.
pub const HEADER_SIZE: usize = 13;
/// `ConnectRequest` and `ChallengeResponse` are padded to at least this, so no
/// handshake reply is larger than the request it answers.
pub const CONNECT_REQUEST_SIZE: usize = 256;
/// `InfoQuery` padding; `Info` replies are never larger. 384 B leaves room
/// for the server info with its player list (`lan::ServerInfo`).
pub const INFO_QUERY_SIZE: usize = 384;
/// Protocol magic in `ConnectRequest`, `ChallengeResponse` and `InfoQuery`.
pub const MAGIC: [u8; 4] = *b"TTRS";
/// Longest reason string in `Reject` / `Disconnect`.
pub const MAX_REASON_LEN: usize = 200;
/// Longest connect-args payload in `ChallengeResponse`.
pub const MAX_CONNECT_PAYLOAD: usize = 900;
/// Longest `Info` payload (fits in an `InfoQuery`-sized reply).
pub const MAX_INFO_PAYLOAD: usize = INFO_QUERY_SIZE - INFO_FIXED;

const INFO_FIXED: usize = 1 + 2 + 8 + 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketKind {
    ConnectRequest = 0,
    Challenge = 1,
    ChallengeResponse = 2,
    Accept = 3,
    Reject = 4,
    Data = 5,
    Disconnect = 6,
    InfoQuery = 7,
    Info = 8,
    Keepalive = 9,
}

impl PacketKind {
    pub fn from_u8(b: u8) -> Result<Self, DecodeError> {
        use PacketKind::*;
        Ok(match b {
            0 => ConnectRequest,
            1 => Challenge,
            2 => ChallengeResponse,
            3 => Accept,
            4 => Reject,
            5 => Data,
            6 => Disconnect,
            7 => InfoQuery,
            8 => Info,
            9 => Keepalive,
            k => return Err(DecodeError::BadKind(k)),
        })
    }
}

/// Connection packet header (after the kind byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Header {
    /// Random per connection, assigned in `Accept`.
    pub session: u32,
    /// This packet's sequence number.
    pub seq: u16,
    /// Newest packet sequence received from the peer.
    pub ack: u16,
    /// Bit `i` set: packet `ack - 1 - i` was received too.
    pub ack_bits: u32,
}

/// A decoded datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    ConnectRequest { protocol: u16, build: u64, nonce: u64 },
    Challenge { nonce: u64, cookie: u64 },
    /// `proof`: the password proof bound to this handshake ([`crate::auth`]), 0 without one.
    ChallengeResponse { protocol: u16, build: u64, nonce: u64, cookie: u64, proof: u64, payload: Vec<u8> },
    Accept { nonce: u64, session: u32 },
    Reject { nonce: u64, reason: String },
    /// `body` is the raw bit-packed [`DataBody`]; decoded by the connection.
    Data { header: Header, body: Vec<u8> },
    Disconnect { header: Header, reason: String },
    InfoQuery { protocol: u16, nonce: u64 },
    Info { protocol: u16, nonce: u64, payload: Vec<u8> },
    Keepalive { header: Header },
}

/// Little-endian reader of the byte formats (packets, LAN info, master
/// datagrams); running out of bytes is [`DecodeError::Truncated`].
pub(crate) struct ByteReader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> ByteReader<'a> {
    /// Reads `d` from offset `p`.
    pub(crate) fn new(d: &'a [u8], p: usize) -> ByteReader<'a> {
        ByteReader { d, p }
    }
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.d.len() - self.p < n {
            return Err(DecodeError::Truncated);
        }
        let s = &self.d[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }
    pub(crate) fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16, DecodeError> {
        self.array().map(u16::from_le_bytes)
    }
    pub(crate) fn u32(&mut self) -> Result<u32, DecodeError> {
        self.array().map(u32::from_le_bytes)
    }
    pub(crate) fn u64(&mut self) -> Result<u64, DecodeError> {
        self.array().map(u64::from_le_bytes)
    }
    /// A string with a u8 length prefix (invalid UTF-8 replaced).
    pub(crate) fn str(&mut self) -> Result<String, DecodeError> {
        let n = self.u8()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn magic(&mut self) -> Result<(), DecodeError> {
        if self.take(4)? != MAGIC {
            return Err(DecodeError::BadMagic);
        }
        Ok(())
    }
    fn header(&mut self) -> Result<Header, DecodeError> {
        Ok(Header { session: self.u32()?, seq: self.u16()?, ack: self.u16()?, ack_bits: self.u32()? })
    }
    fn short_string(&mut self) -> Result<String, DecodeError> {
        let n = self.u8()? as usize;
        if n > MAX_REASON_LEN {
            return Err(DecodeError::TooLarge);
        }
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    pub(crate) fn rest(&self) -> &'a [u8] {
        &self.d[self.p..]
    }
}

fn put_header(out: &mut Vec<u8>, h: &Header) {
    out.extend_from_slice(&h.session.to_le_bytes());
    out.extend_from_slice(&h.seq.to_le_bytes());
    out.extend_from_slice(&h.ack.to_le_bytes());
    out.extend_from_slice(&h.ack_bits.to_le_bytes());
}

/// Truncates to [`MAX_REASON_LEN`] bytes on a char boundary.
pub fn clip_reason(s: &str) -> &str {
    if s.len() <= MAX_REASON_LEN {
        return s;
    }
    let mut end = MAX_REASON_LEN;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn put_reason(out: &mut Vec<u8>, s: &str) {
    let s = clip_reason(s);
    out.push(s.len() as u8);
    out.extend_from_slice(s.as_bytes());
}

impl Packet {
    pub fn kind(&self) -> PacketKind {
        match self {
            Packet::ConnectRequest { .. } => PacketKind::ConnectRequest,
            Packet::Challenge { .. } => PacketKind::Challenge,
            Packet::ChallengeResponse { .. } => PacketKind::ChallengeResponse,
            Packet::Accept { .. } => PacketKind::Accept,
            Packet::Reject { .. } => PacketKind::Reject,
            Packet::Data { .. } => PacketKind::Data,
            Packet::Disconnect { .. } => PacketKind::Disconnect,
            Packet::InfoQuery { .. } => PacketKind::InfoQuery,
            Packet::Info { .. } => PacketKind::Info,
            Packet::Keepalive { .. } => PacketKind::Keepalive,
        }
    }

    /// Encodes the datagram. Oversized payloads are clipped to their limits
    /// (callers check limits up front); the result never exceeds
    /// [`MAX_PACKET_SIZE`] for bodies built by this crate.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.push(self.kind() as u8);
        match self {
            Packet::ConnectRequest { protocol, build, nonce } => {
                out.extend_from_slice(&MAGIC);
                out.extend_from_slice(&protocol.to_le_bytes());
                out.extend_from_slice(&build.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
                out.resize(CONNECT_REQUEST_SIZE, 0);
            }
            Packet::Challenge { nonce, cookie } => {
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&cookie.to_le_bytes());
            }
            Packet::ChallengeResponse { protocol, build, nonce, cookie, proof, payload } => {
                let payload = &payload[..payload.len().min(MAX_CONNECT_PAYLOAD)];
                out.extend_from_slice(&MAGIC);
                out.extend_from_slice(&protocol.to_le_bytes());
                out.extend_from_slice(&build.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&cookie.to_le_bytes());
                out.extend_from_slice(&proof.to_le_bytes());
                out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                out.extend_from_slice(payload);
                if out.len() < CONNECT_REQUEST_SIZE {
                    out.resize(CONNECT_REQUEST_SIZE, 0);
                }
            }
            Packet::Accept { nonce, session } => {
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&session.to_le_bytes());
            }
            Packet::Reject { nonce, reason } => {
                out.extend_from_slice(&nonce.to_le_bytes());
                put_reason(&mut out, reason);
            }
            Packet::Data { header, body } => {
                put_header(&mut out, header);
                out.extend_from_slice(body);
            }
            Packet::Disconnect { header, reason } => {
                put_header(&mut out, header);
                put_reason(&mut out, reason);
            }
            Packet::InfoQuery { protocol, nonce } => {
                out.extend_from_slice(&MAGIC);
                out.extend_from_slice(&protocol.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
                out.resize(INFO_QUERY_SIZE, 0);
            }
            Packet::Info { protocol, nonce, payload } => {
                let payload = &payload[..payload.len().min(MAX_INFO_PAYLOAD)];
                out.extend_from_slice(&protocol.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                out.extend_from_slice(payload);
            }
            Packet::Keepalive { header } => put_header(&mut out, header),
        }
        out
    }

    /// Decodes a datagram. Never panics; any inconsistency is an error.
    pub fn decode(data: &[u8]) -> Result<Packet, DecodeError> {
        if data.len() > MAX_PACKET_SIZE {
            return Err(DecodeError::TooLarge);
        }
        let mut r = ByteReader::new(data, 0);
        let kind = PacketKind::from_u8(r.u8()?)?;
        Ok(match kind {
            PacketKind::ConnectRequest => {
                if data.len() < CONNECT_REQUEST_SIZE {
                    return Err(DecodeError::TooSmall);
                }
                r.magic()?;
                Packet::ConnectRequest { protocol: r.u16()?, build: r.u64()?, nonce: r.u64()? }
            }
            PacketKind::Challenge => Packet::Challenge { nonce: r.u64()?, cookie: r.u64()? },
            PacketKind::ChallengeResponse => {
                if data.len() < CONNECT_REQUEST_SIZE {
                    return Err(DecodeError::TooSmall);
                }
                r.magic()?;
                let protocol = r.u16()?;
                let build = r.u64()?;
                let nonce = r.u64()?;
                let cookie = r.u64()?;
                let proof = r.u64()?;
                let n = r.u16()? as usize;
                if n > MAX_CONNECT_PAYLOAD {
                    return Err(DecodeError::TooLarge);
                }
                let payload = r.take(n)?.to_vec();
                Packet::ChallengeResponse { protocol, build, nonce, cookie, proof, payload }
            }
            PacketKind::Accept => Packet::Accept { nonce: r.u64()?, session: r.u32()? },
            PacketKind::Reject => Packet::Reject { nonce: r.u64()?, reason: r.short_string()? },
            PacketKind::Data => {
                let header = r.header()?;
                Packet::Data { header, body: r.rest().to_vec() }
            }
            PacketKind::Disconnect => {
                let header = r.header()?;
                Packet::Disconnect { header, reason: r.short_string()? }
            }
            PacketKind::InfoQuery => {
                if data.len() < INFO_QUERY_SIZE {
                    return Err(DecodeError::TooSmall);
                }
                r.magic()?;
                Packet::InfoQuery { protocol: r.u16()?, nonce: r.u64()? }
            }
            PacketKind::Info => {
                let protocol = r.u16()?;
                let nonce = r.u64()?;
                let n = r.u16()? as usize;
                if n > MAX_INFO_PAYLOAD {
                    return Err(DecodeError::TooLarge);
                }
                Packet::Info { protocol, nonce, payload: r.take(n)?.to_vec() }
            }
            PacketKind::Keepalive => Packet::Keepalive { header: r.header()? },
        })
    }
}

/// One chunk of the ordered reliable stream on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireChunk {
    pub id: u16,
    /// Last chunk of its message.
    pub last: bool,
    pub data: Vec<u8>,
}

/// One unordered reliable message on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireEvent {
    pub id: u16,
    pub data: Vec<u8>,
}

/// The bit-packed body of a `Data` packet: three present-flagged sections.
///
/// ```text
/// bit  has_ordered   [varint count, u16 first id, per chunk: (varint id-prev-1 after the first),
///                     bit last, varint len, bytes]
/// bit  has_unordered [varint count, u16 first id, per msg: (varint id-prev-1), varint len, bytes]
/// bit  has_unreliable[varint count, per msg: varint len, bytes]
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DataBody {
    pub ordered: Vec<WireChunk>,
    pub unordered: Vec<WireEvent>,
    pub unreliable: Vec<Vec<u8>>,
}

/// Bits reserved for section flags, counts and first ids.
pub const BODY_OVERHEAD_BITS: usize = 3 + 2 * (16 + 16) + 16;

/// Wire cost of an ordered chunk / unordered message (`delta` = id gap − 1,
/// `None` for the first in its section).
pub fn reliable_cost_bits(len: usize, delta: Option<u16>, ordered: bool) -> usize {
    delta.map_or(0, |d| varint_bits(d as u64)) + ordered as usize + varint_bits(len as u64) + 8 * len
}

/// Wire cost of an unreliable message.
pub fn unreliable_cost_bits(len: usize) -> usize {
    varint_bits(len as u64) + 8 * len
}

impl DataBody {
    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty() && self.unordered.is_empty() && self.unreliable.is_empty()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = BitWriter::with_capacity(256);
        w.write_bool(!self.ordered.is_empty());
        if let Some(first) = self.ordered.first() {
            w.write_varint(self.ordered.len() as u64);
            w.write_bits(first.id as u64, 16);
            let mut prev = first.id;
            for (i, c) in self.ordered.iter().enumerate() {
                if i > 0 {
                    w.write_varint(c.id.wrapping_sub(prev).wrapping_sub(1) as u64);
                    prev = c.id;
                }
                w.write_bool(c.last);
                w.write_varint(c.data.len() as u64);
                w.write_bytes(&c.data);
            }
        }
        w.write_bool(!self.unordered.is_empty());
        if let Some(first) = self.unordered.first() {
            w.write_varint(self.unordered.len() as u64);
            w.write_bits(first.id as u64, 16);
            let mut prev = first.id;
            for (i, m) in self.unordered.iter().enumerate() {
                if i > 0 {
                    w.write_varint(m.id.wrapping_sub(prev).wrapping_sub(1) as u64);
                    prev = m.id;
                }
                w.write_varint(m.data.len() as u64);
                w.write_bytes(&m.data);
            }
        }
        w.write_bool(!self.unreliable.is_empty());
        if !self.unreliable.is_empty() {
            w.write_varint(self.unreliable.len() as u64);
            for m in &self.unreliable {
                w.write_varint(m.len() as u64);
                w.write_bytes(m);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<DataBody, DecodeError> {
        let mut r = BitReader::new(data);
        let mut body = DataBody::default();
        // Every entry costs at least 9 bits, so `count` is bounded by the input.
        let read_count = |r: &mut BitReader| -> Result<usize, DecodeError> {
            let n = r.read_varint()?;
            if n == 0 || n > (r.remaining_bits() / 9) as u64 {
                return Err(DecodeError::Invalid("section count"));
            }
            Ok(n as usize)
        };
        let read_len = |r: &mut BitReader| -> Result<usize, DecodeError> {
            let n = r.read_varint()?;
            if n > MAX_PACKET_SIZE as u64 {
                return Err(DecodeError::TooLarge);
            }
            Ok(n as usize)
        };
        let read_delta = |r: &mut BitReader| -> Result<u16, DecodeError> {
            let d = r.read_varint()?;
            if d >= u16::MAX as u64 {
                return Err(DecodeError::Invalid("id delta"));
            }
            Ok(d as u16)
        };
        if r.read_bool()? {
            let n = read_count(&mut r)?;
            let mut id = r.read_bits(16)? as u16;
            for i in 0..n {
                if i > 0 {
                    id = id.wrapping_add(read_delta(&mut r)?).wrapping_add(1);
                }
                let last = r.read_bool()?;
                let len = read_len(&mut r)?;
                body.ordered.push(WireChunk { id, last, data: r.read_bytes(len)? });
            }
        }
        if r.read_bool()? {
            let n = read_count(&mut r)?;
            let mut id = r.read_bits(16)? as u16;
            for i in 0..n {
                if i > 0 {
                    id = id.wrapping_add(read_delta(&mut r)?).wrapping_add(1);
                }
                let len = read_len(&mut r)?;
                body.unordered.push(WireEvent { id, data: r.read_bytes(len)? });
            }
        }
        if r.read_bool()? {
            let n = read_count(&mut r)?;
            for _ in 0..n {
                let len = read_len(&mut r)?;
                body.unreliable.push(r.read_bytes(len)?);
            }
        }
        if r.remaining_bits() >= 8 {
            return Err(DecodeError::Invalid("trailing bytes"));
        }
        Ok(body)
    }
}

/// `a` is newer than `b` in u16 sequence space.
pub fn seq_newer(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

/// Widens a u16 sequence to u32 using the nearest value to `reference`.
pub fn widen_u16(reference: u32, raw: u16) -> u32 {
    let diff = raw.wrapping_sub(reference as u16) as i16 as i64;
    (reference as i64 + diff).max(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_packets() -> Vec<Packet> {
        let h = Header { session: 0xdead_beef, seq: 65535, ack: 3, ack_bits: 0x8000_0001 };
        vec![
            Packet::ConnectRequest { protocol: 1, build: 99, nonce: 7 },
            Packet::Challenge { nonce: 7, cookie: 0x1234 },
            Packet::ChallengeResponse { protocol: 1, build: 99, nonce: 7, cookie: 5, proof: 77, payload: vec![1, 2, 3] },
            Packet::ChallengeResponse { protocol: 1, build: 99, nonce: 7, cookie: 5, proof: 77, payload: vec![9; 800] },
            Packet::Accept { nonce: 7, session: 77 },
            Packet::Reject { nonce: 7, reason: "CR_SERVERFULL".into() },
            Packet::Data { header: h, body: vec![1, 2, 3] },
            Packet::Disconnect { header: h, reason: "bye".into() },
            Packet::InfoQuery { protocol: 1, nonce: 8 },
            Packet::Info { protocol: 1, nonce: 8, payload: b"name".to_vec() },
            Packet::Keepalive { header: h },
        ]
    }

    #[test]
    fn packets_round_trip() {
        for p in all_packets() {
            let bytes = p.encode();
            assert!(bytes.len() <= MAX_PACKET_SIZE);
            assert_eq!(Packet::decode(&bytes).unwrap(), p);
        }
        assert_eq!(Packet::Keepalive { header: Header::default() }.encode().len(), HEADER_SIZE);
    }

    #[test]
    fn handshake_replies_never_exceed_requests() {
        let req = Packet::ConnectRequest { protocol: 1, build: 1, nonce: 1 }.encode().len();
        let ch = Packet::Challenge { nonce: 1, cookie: 1 }.encode().len();
        let rej = Packet::Reject { nonce: 1, reason: "x".repeat(1000) }.encode().len();
        let resp = Packet::ChallengeResponse { protocol: 1, build: 1, nonce: 1, cookie: 1, proof: 0, payload: vec![] }
            .encode()
            .len();
        let acc = Packet::Accept { nonce: 1, session: 1 }.encode().len();
        assert_eq!(req, CONNECT_REQUEST_SIZE);
        assert!(ch <= req && rej <= req && rej <= resp && acc <= resp);
        let q = Packet::InfoQuery { protocol: 1, nonce: 1 }.encode().len();
        let info = Packet::Info { protocol: 1, nonce: 1, payload: vec![0; 500] }.encode().len();
        assert!(info <= q);
    }

    #[test]
    fn short_requests_are_refused() {
        let mut b = Packet::ConnectRequest { protocol: 1, build: 1, nonce: 1 }.encode();
        b.truncate(100);
        assert_eq!(Packet::decode(&b), Err(DecodeError::TooSmall));
        let mut b = Packet::InfoQuery { protocol: 1, nonce: 1 }.encode();
        b.truncate(INFO_QUERY_SIZE - 1);
        assert_eq!(Packet::decode(&b), Err(DecodeError::TooSmall));
        assert_eq!(Packet::decode(&[42]), Err(DecodeError::BadKind(42)));
        assert_eq!(Packet::decode(&[]), Err(DecodeError::Truncated));
        assert_eq!(Packet::decode(&[5; 1201]), Err(DecodeError::TooLarge));
    }

    #[test]
    fn body_round_trip() {
        let body = DataBody {
            ordered: vec![
                WireChunk { id: 65534, last: false, data: vec![1; 300] },
                WireChunk { id: 65535, last: true, data: vec![] },
                WireChunk { id: 3, last: true, data: vec![7] },
            ],
            unordered: vec![WireEvent { id: 10, data: vec![2; 5] }, WireEvent { id: 12, data: vec![3] }],
            unreliable: vec![vec![], vec![4; 20]],
        };
        let enc = body.encode();
        assert_eq!(DataBody::decode(&enc).unwrap(), body);
        assert_eq!(DataBody::decode(&DataBody::default().encode()).unwrap(), DataBody::default());
    }

    #[test]
    fn seq_helpers() {
        assert!(seq_newer(1, 0));
        assert!(seq_newer(0, 65535));
        assert!(!seq_newer(65535, 0));
        assert!(!seq_newer(5, 5));
        assert_eq!(widen_u16(65530, 2), 65538);
        assert_eq!(widen_u16(65540, 65535), 65535);
        assert_eq!(widen_u16(0, 65535), 0);
        assert_eq!(widen_u16(100, 90), 90);
    }
}

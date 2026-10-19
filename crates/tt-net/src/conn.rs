//! One established connection: sequence numbers and acks, RTT/loss, the
//! three lanes, keepalives and timeouts. Shared by [`Server`](crate::Server)
//! and [`Client`](crate::Client); the handshake lives there.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;

use crate::packet::{
    seq_newer, unreliable_cost_bits, DataBody, Header, Packet, BODY_OVERHEAD_BITS, HEADER_SIZE,
    MAX_PACKET_SIZE,
};
use crate::reliable::{OrderedReceiver, OrderedSender, PushError, UnorderedReceiver, UnorderedSender};
use crate::stats::{ConnStats, LossWindow, RateMeter, Rtt};
use crate::transport::Datagram;
use crate::SendError;

/// Message lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    /// Reliable, ordered (remote commands, tags, mission phases).
    Reliable,
    /// Reliable, unordered, dropped after 1 s (projectile/trigger events).
    Event,
    /// Unreliable (inputs, snapshots): sent in the next packet or dropped.
    Unreliable,
}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Nothing received for the timeout (10 s default).
    Timeout,
    /// The client gave up connecting (no answer to the handshake).
    ConnectTimeout,
    /// The peer sent `Disconnect{reason}`.
    Remote(String),
    /// We called `disconnect(reason)`.
    Local(String),
    /// The server refused the connection (`Reject{reason}`).
    Rejected(String),
    /// Unacked reliable data passed the cap.
    Overflow,
    /// The peer violated the protocol.
    Protocol(&'static str),
    /// A new handshake from the same address replaced the connection.
    Replaced,
}

impl DisconnectReason {
    /// Short text, as sent on the wire / passed to `onDrop`.
    pub fn text(&self) -> &str {
        match self {
            DisconnectReason::Timeout => "timeout",
            DisconnectReason::ConnectTimeout => "connect timeout",
            DisconnectReason::Remote(s) | DisconnectReason::Local(s) | DisconnectReason::Rejected(s) => s,
            DisconnectReason::Overflow => "overflow",
            DisconnectReason::Protocol(s) => s,
            DisconnectReason::Replaced => "replaced",
        }
    }
}

/// Timer and size settings of a connection.
#[derive(Debug, Clone)]
pub struct ConnConfig {
    /// Drop after this long without a packet from the peer (§5.3: 10 s; 15 s while loading).
    pub timeout: Duration,
    /// Send a keepalive after this long without sending (§3.1: 250 ms).
    pub keepalive: Duration,
    /// Reliable bytes a packet may carry when it also carries unreliable data (§3.2).
    pub reliable_budget_with_unreliable: usize,
    /// Most packets one `flush` sends to a connection.
    pub max_packets_per_flush: usize,
}

impl Default for ConnConfig {
    fn default() -> Self {
        ConnConfig {
            timeout: Duration::from_secs(10),
            keepalive: Duration::from_millis(250),
            reliable_budget_with_unreliable: 400,
            max_packets_per_flush: 4,
        }
    }
}

const SENT_BUF: usize = 1024;
const RECV_WINDOW: usize = 256;
/// Send an ack-only packet after receiving this many packets without sending.
const ACK_PRESSURE: u32 = 16;
/// A packet not acked after this long counts as lost (statistics).
const LOSS_TIMEOUT: Duration = Duration::from_secs(1);
/// Bits in the ack field beyond which a packet's ack can no longer arrive.
const ACK_SPAN: u16 = 33;
/// A packet whose bit is still clear this many packets after it is nacked.
const NACK_DISTANCE: u16 = 3;
const MAX_ACK_LOG: usize = 256;

#[derive(Debug)]
struct SentPacket {
    seq: u16,
    time: Duration,
    ordered: Vec<u64>,
    events: Vec<u64>,
    acked: bool,
    nacked: bool,
    /// Not a pure ack: the peer acks it at its next flush, so its ack gives a
    /// clean RTT sample (pure acks are acked lazily).
    eliciting: bool,
    /// Counted in the loss statistic already.
    decided: bool,
}

/// Connection state (after the handshake).
#[derive(Debug)]
pub struct Connection {
    pub(crate) addr: SocketAddr,
    pub(crate) session: u32,
    pub(crate) cfg: ConnConfig,
    local_seq: u16,
    sent: Vec<Option<SentPacket>>,
    undecided: VecDeque<u16>,
    peer_newest_ack: Option<u16>,
    remote_newest: Option<u16>,
    recv_ring: Vec<u32>,
    ordered_tx: OrderedSender,
    ordered_rx: OrderedReceiver,
    events_tx: UnorderedSender,
    events_rx: UnorderedReceiver,
    unreliable_tx: VecDeque<Vec<u8>>,
    pub(crate) inbox: VecDeque<(Lane, Vec<u8>)>,
    acked_log: VecDeque<u16>,
    rtt: Rtt,
    loss: LossWindow,
    rate_in: RateMeter,
    rate_out: RateMeter,
    stats: ConnStats,
    pub(crate) last_recv: Duration,
    last_send: Duration,
    ack_pending: bool,
    recv_since_send: u32,
}

impl Connection {
    pub(crate) fn new(addr: SocketAddr, session: u32, cfg: ConnConfig, now: Duration, initial_rtt: Duration) -> Self {
        Connection {
            addr,
            session,
            cfg,
            // Seq 0 is skipped at the start: a peer that has received nothing
            // sends ack = 0 / ack_bits = 0, which must not ack a real packet.
            local_seq: 1,
            sent: (0..SENT_BUF).map(|_| None).collect(),
            undecided: VecDeque::new(),
            peer_newest_ack: None,
            remote_newest: None,
            recv_ring: vec![0; RECV_WINDOW],
            ordered_tx: OrderedSender::default(),
            ordered_rx: OrderedReceiver::default(),
            events_tx: UnorderedSender::default(),
            events_rx: UnorderedReceiver::default(),
            unreliable_tx: VecDeque::new(),
            inbox: VecDeque::new(),
            acked_log: VecDeque::new(),
            rtt: Rtt::new(initial_rtt),
            loss: LossWindow::default(),
            rate_in: RateMeter::default(),
            rate_out: RateMeter::default(),
            stats: ConnStats::default(),
            last_recv: now,
            last_send: now,
            ack_pending: false,
            recv_since_send: 0,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Sequence number the next packet will carry.
    pub fn next_seq(&self) -> u16 {
        self.local_seq
    }

    /// Packet sequences the peer acked since the last call (oldest first,
    /// at most 256 kept). Drives snapshot delta baselines.
    pub fn take_acked(&mut self) -> Vec<u16> {
        self.acked_log.drain(..).collect()
    }

    pub fn stats(&mut self, now: Duration) -> ConnStats {
        let mut s = self.stats.clone();
        s.rtt = self.rtt.srtt();
        s.rtt_var = self.rtt.var();
        s.loss = self.loss.loss();
        s.bytes_in_per_sec = self.rate_in.per_sec(now);
        s.bytes_out_per_sec = self.rate_out.per_sec(now);
        s.resends = self.ordered_tx.resends + self.events_tx.resends;
        s.events_expired = self.events_tx.expired;
        s
    }

    /// Reliable bytes (both lanes) queued or in flight, not yet acked. Sends
    /// past [`MAX_UNACKED_BYTES`](crate::reliable::MAX_UNACKED_BYTES) overflow.
    pub fn reliable_backlog(&self) -> usize {
        self.ordered_tx.unacked_bytes() + self.events_tx.unacked_bytes()
    }

    pub(crate) fn note_malformed(&mut self) {
        self.stats.malformed += 1;
    }

    /// Queues a message.
    pub(crate) fn send(&mut self, lane: Lane, data: &[u8], now: Duration) -> Result<(), SendError> {
        let r = match lane {
            Lane::Reliable => self.ordered_tx.push(data, self.events_tx.unacked_bytes()),
            Lane::Event => self.events_tx.push(data, now, self.ordered_tx.unacked_bytes()),
            Lane::Unreliable => {
                if unreliable_cost_bits(data.len()) + BODY_OVERHEAD_BITS > (MAX_PACKET_SIZE - HEADER_SIZE) * 8 {
                    return Err(SendError::TooLarge);
                }
                self.unreliable_tx.push_back(data.to_vec());
                return Ok(());
            }
        };
        r.map_err(|e| match e {
            PushError::TooLarge => SendError::TooLarge,
            PushError::Overflow => SendError::Overflow,
        })
    }

    fn header(&mut self) -> Header {
        let (ack, ack_bits) = match self.remote_newest {
            None => (0, 0),
            Some(newest) => {
                let mut bits = 0u32;
                for i in 1..=32u16 {
                    let s = newest.wrapping_sub(i);
                    if self.recv_ring[s as usize % RECV_WINDOW] == s as u32 + 1 {
                        bits |= 1 << (i - 1);
                    }
                }
                (newest, bits)
            }
        };
        let h = Header { session: self.session, seq: self.local_seq, ack, ack_bits };
        self.local_seq = self.local_seq.wrapping_add(1);
        h
    }

    fn wants_send(&self, now: Duration) -> bool {
        let rto = self.rtt.rto();
        !self.unreliable_tx.is_empty()
            || self.ordered_tx.has_due(now, rto)
            || self.events_tx.has_due(now, rto)
            || self.ack_pending
            || self.recv_since_send >= ACK_PRESSURE
            || now.saturating_sub(self.last_send) >= self.cfg.keepalive
    }

    fn record_sent(&mut self, seq: u16, now: Duration, ordered: Vec<u64>, events: Vec<u64>, size: usize, eliciting: bool) {
        let slot = seq as usize % SENT_BUF;
        if let Some(old) = self.sent[slot].take() {
            if !old.acked && !old.decided {
                self.loss.record(true);
                self.stats.lost += 1;
            }
        }
        self.sent[slot] = Some(SentPacket { seq, time: now, ordered, events, acked: false, nacked: false, eliciting, decided: false });
        self.undecided.push_back(seq);
        self.last_send = now;
        self.ack_pending = false;
        self.recv_since_send = 0;
        self.stats.packets_out += 1;
        self.stats.bytes_out += size as u64;
        self.rate_out.record(now, size);
    }

    /// Builds the next packet, or `None` if there is nothing to send.
    fn build(&mut self, now: Duration) -> Option<Vec<u8>> {
        self.events_tx.expire(now);
        if !self.wants_send(now) {
            return None;
        }
        let rto = self.rtt.rto();
        let total = (MAX_PACKET_SIZE - HEADER_SIZE) * 8 - BODY_OVERHEAD_BITS;
        // Unreliable data (snapshots, inputs) has priority for its space; the
        // reliable lanes then get at most 400 B of what is left, or the whole
        // packet when there is no unreliable data. Events go before the
        // ordered stream so a long transfer cannot starve them.
        let reliable_budget = match self.unreliable_tx.front() {
            None => total,
            Some(m) => total
                .saturating_sub(unreliable_cost_bits(m.len()))
                .min(self.cfg.reliable_budget_with_unreliable * 8),
        };
        let mut body = DataBody::default();
        let (mut ordered_ids, mut event_ids) = (Vec::new(), Vec::new());
        let mut used = self.events_tx.collect(now, rto, reliable_budget, &mut body.unordered, &mut event_ids);
        used += self.ordered_tx.collect(now, rto, reliable_budget - used, &mut body.ordered, &mut ordered_ids);
        let mut left = total - used;
        while let Some(m) = self.unreliable_tx.front() {
            let cost = unreliable_cost_bits(m.len());
            if cost > left {
                break;
            }
            left -= cost;
            body.unreliable.push(self.unreliable_tx.pop_front().unwrap_or_default());
        }
        let ack_only = self.ack_pending || self.recv_since_send >= ACK_PRESSURE;
        let header = self.header();
        // `Keepalive` asks the peer for an immediate ack (clean RTT samples
        // on idle links); a pure ack is an empty `Data`, which asks nothing,
        // so the two never ping-pong.
        let eliciting = !(body.is_empty() && ack_only);
        let bytes = if body.is_empty() && !ack_only {
            Packet::Keepalive { header }.encode()
        } else {
            Packet::Data { header, body: body.encode() }.encode()
        };
        debug_assert!(bytes.len() <= MAX_PACKET_SIZE);
        self.record_sent(header.seq, now, ordered_ids, event_ids, bytes.len(), eliciting);
        Some(bytes)
    }

    /// Sends whatever is due (up to `max_packets_per_flush` packets).
    /// Unreliable messages that did not fit are dropped.
    pub(crate) fn flush(&mut self, now: Duration, t: &mut dyn Datagram) -> usize {
        let mut n = 0;
        while n < self.cfg.max_packets_per_flush {
            let Some(bytes) = self.build(now) else { break };
            let _ = t.send_to(&bytes, self.addr);
            n += 1;
        }
        self.stats.unreliable_dropped += self.unreliable_tx.len() as u64;
        self.unreliable_tx.clear();
        n
    }

    /// Sends `Disconnect{reason}` three times (§5.3).
    pub(crate) fn send_disconnect(&mut self, reason: &str, now: Duration, t: &mut dyn Datagram) {
        for _ in 0..3 {
            let header = self.header();
            let bytes = Packet::Disconnect { header, reason: reason.to_string() }.encode();
            let _ = t.send_to(&bytes, self.addr);
            self.stats.packets_out += 1;
            self.stats.bytes_out += bytes.len() as u64;
            self.rate_out.record(now, bytes.len());
        }
    }

    /// Records an incoming packet's sequence; `false` for duplicates / too old.
    fn accept_seq(&mut self, seq: u16) -> bool {
        let slot = seq as usize % RECV_WINDOW;
        match self.remote_newest {
            None => self.remote_newest = Some(seq),
            Some(newest) => {
                let diff = seq.wrapping_sub(newest) as i16;
                if diff == 0 || (diff < 0 && (-(diff as i32)) as usize >= RECV_WINDOW) {
                    return false;
                }
                if diff < 0 && self.recv_ring[slot] == seq as u32 + 1 {
                    return false;
                }
                if diff > 0 {
                    self.remote_newest = Some(seq);
                }
            }
        }
        self.recv_ring[slot] = seq as u32 + 1;
        true
    }

    fn on_ack(&mut self, seq: u16, now: Duration, newest: bool) {
        let slot = seq as usize % SENT_BUF;
        let Some(p) = self.sent[slot].as_mut() else { return };
        if p.seq != seq || p.acked {
            return;
        }
        p.acked = true;
        let time = p.time;
        let newest = newest && p.eliciting;
        let ordered = std::mem::take(&mut p.ordered);
        let events = std::mem::take(&mut p.events);
        for id in ordered {
            self.ordered_tx.ack(id);
        }
        for id in events {
            self.events_tx.ack(id, now);
        }
        if newest {
            self.rtt.sample(now.saturating_sub(time));
        }
        if self.acked_log.len() >= MAX_ACK_LOG {
            self.acked_log.pop_front();
        }
        self.acked_log.push_back(seq);
    }

    fn on_nack(&mut self, seq: u16) {
        let slot = seq as usize % SENT_BUF;
        let Some(p) = self.sent[slot].as_mut() else { return };
        if p.seq != seq || p.acked || p.nacked {
            return;
        }
        p.nacked = true;
        for &id in &p.ordered {
            self.ordered_tx.nack(id);
        }
        for &id in &p.events {
            self.events_tx.nack(id);
        }
    }

    fn process_acks(&mut self, h: &Header, now: Duration) {
        // Ignore acks for packets we have not sent yet (forged or corrupt).
        if !seq_newer(self.local_seq, h.ack) {
            return;
        }
        self.on_ack(h.ack, now, true);
        for i in 1..=32u16 {
            let s = h.ack.wrapping_sub(i);
            if h.ack_bits & (1 << (i - 1)) != 0 {
                self.on_ack(s, now, false);
            } else if i >= NACK_DISTANCE {
                self.on_nack(s);
            }
        }
        if self.peer_newest_ack.is_none_or(|p| seq_newer(h.ack, p)) {
            self.peer_newest_ack = Some(h.ack);
        }
    }

    /// Handles a decoded `Data`/`Keepalive` from the peer (session already checked).
    pub(crate) fn on_packet(
        &mut self,
        h: &Header,
        body: Option<DataBody>,
        size: usize,
        now: Duration,
    ) -> Result<(), &'static str> {
        self.stats.packets_in += 1;
        self.stats.bytes_in += size as u64;
        self.rate_in.record(now, size);
        if !self.accept_seq(h.seq) {
            self.stats.duplicates += 1;
            return Ok(());
        }
        self.last_recv = now;
        self.recv_since_send += 1;
        self.process_acks(h, now);
        let Some(body) = body else {
            self.ack_pending = true; // a Keepalive: answer with an ack
            return Ok(());
        };
        if !body.is_empty() {
            self.ack_pending = true; // everything but a pure ack gets acked promptly
        }
        let mut done = VecDeque::new();
        for c in body.ordered {
            self.ordered_rx.receive(c, &mut done)?;
        }
        for m in done {
            self.inbox.push_back((Lane::Reliable, m));
        }
        for e in body.unordered {
            if let Some(m) = self.events_rx.receive(e) {
                self.inbox.push_back((Lane::Event, m));
            }
        }
        for m in body.unreliable {
            self.inbox.push_back((Lane::Unreliable, m));
        }
        Ok(())
    }

    /// Decides the fate of old packets for the loss statistic. Returns
    /// `true` when the connection timed out.
    pub(crate) fn update(&mut self, now: Duration) -> bool {
        while let Some(&seq) = self.undecided.front() {
            let slot = seq as usize % SENT_BUF;
            let fate = match self.sent[slot].as_ref() {
                Some(p) if p.seq == seq && !p.decided => {
                    if p.acked {
                        Some(false)
                    } else if now.saturating_sub(p.time) > LOSS_TIMEOUT
                        || self.peer_newest_ack.is_some_and(|a| seq_newer(a, seq) && a.wrapping_sub(seq) >= ACK_SPAN)
                    {
                        Some(true)
                    } else {
                        None
                    }
                }
                _ => {
                    self.undecided.pop_front();
                    continue;
                }
            };
            let Some(lost) = fate else { break };
            self.loss.record(lost);
            if lost {
                self.stats.lost += 1;
                // Make sure its reliable content goes out again.
                self.on_nack(seq);
            } else {
                self.stats.acked += 1;
            }
            self.undecided.pop_front();
            // A late ack still acks its content but is not counted again.
            if let Some(p) = self.sent[slot].as_mut() {
                p.decided = true;
            }
        }
        now.saturating_sub(self.last_recv) > self.cfg.timeout
    }
}

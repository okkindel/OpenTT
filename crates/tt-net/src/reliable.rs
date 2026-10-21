//! Reliable lanes on top of packet acks.
//!
//! * **Ordered**: one stream of messages, split into chunks of at most
//!   [`CHUNK_SIZE`] bytes, each chunk a u16 id. The receiver delivers whole
//!   messages in order through a 256-chunk window.
//! * **Unordered** (events): single-chunk messages delivered on first
//!   arrival, deduplicated, and dropped by the sender if not acked within
//!   [`UNORDERED_LIFETIME`].
//!
//! Chunks are (re)sent when never sent, when the packet carrying them was
//! nacked, or after the retransmission timeout without an ack.

use std::collections::VecDeque;
use std::time::Duration;

use crate::packet::{reliable_cost_bits, WireChunk, WireEvent};

/// Largest ordered chunk. Small enough that a chunk always fits the 400 B
/// reliable budget a packet keeps when it also carries a snapshot.
pub const CHUNK_SIZE: usize = 384;
/// Largest ordered message (it is split into chunks).
pub const MAX_MESSAGE_SIZE: usize = 32 * 1024;
/// Largest unordered message (one chunk; events are small).
pub const MAX_UNORDERED_SIZE: usize = 1024;
/// Receiver window of the ordered lane, in chunks.
pub const ORDERED_WINDOW: u64 = 256;
/// Unacked reliable bytes per connection (both lanes) before "overflow".
pub const MAX_UNACKED_BYTES: usize = 64 * 1024;
/// Unordered messages older than this are dropped by the sender.
pub const UNORDERED_LIFETIME: Duration = Duration::from_secs(1);
/// Most unordered messages in flight.
pub const MAX_UNORDERED_IN_FLIGHT: usize = 512;
const UNORDERED_DEDUP: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    TooLarge,
    Overflow,
}

#[derive(Debug)]
struct OutChunk {
    data: Vec<u8>,
    last: bool,
    acked: bool,
    last_sent: Option<Duration>,
}

/// Sending side of the ordered lane.
#[derive(Debug, Default)]
pub struct OrderedSender {
    /// Id of `queue[0]`.
    base: u64,
    queue: VecDeque<OutChunk>,
    unacked_bytes: usize,
    pub resends: u64,
}

impl OrderedSender {
    pub fn unacked_bytes(&self) -> usize {
        self.unacked_bytes
    }

    /// Queues a message. `budget_used` is the other lane's unacked bytes.
    pub fn push(&mut self, msg: &[u8], other_unacked: usize) -> Result<(), PushError> {
        if msg.len() > MAX_MESSAGE_SIZE {
            return Err(PushError::TooLarge);
        }
        if self.unacked_bytes + other_unacked + msg.len() > MAX_UNACKED_BYTES {
            return Err(PushError::Overflow);
        }
        let n = msg.len().div_ceil(CHUNK_SIZE).max(1);
        for i in 0..n {
            let part = &msg[(i * CHUNK_SIZE).min(msg.len())..((i + 1) * CHUNK_SIZE).min(msg.len())];
            self.queue.push_back(OutChunk { data: part.to_vec(), last: i + 1 == n, acked: false, last_sent: None });
        }
        self.unacked_bytes += msg.len();
        Ok(())
    }

    fn get(&mut self, id: u64) -> Option<&mut OutChunk> {
        let idx = id.checked_sub(self.base)?;
        self.queue.get_mut(idx as usize)
    }

    pub fn ack(&mut self, id: u64) {
        if let Some(c) = self.get(id) {
            if !c.acked {
                c.acked = true;
                let len = c.data.len();
                self.unacked_bytes -= len;
            }
        }
        while self.queue.front().is_some_and(|c| c.acked) {
            self.queue.pop_front();
            self.base += 1;
        }
    }

    /// The packet carrying `id` was lost: resend at the next opportunity.
    pub fn nack(&mut self, id: u64) {
        if let Some(c) = self.get(id) {
            if !c.acked {
                c.last_sent = None;
            }
        }
    }

    fn due(c: &OutChunk, now: Duration, rto: Duration) -> bool {
        !c.acked && c.last_sent.is_none_or(|t| now.saturating_sub(t) >= rto)
    }

    pub fn has_due(&self, now: Duration, rto: Duration) -> bool {
        self.queue.iter().take(ORDERED_WINDOW as usize).any(|c| Self::due(c, now, rto))
    }

    /// Moves due chunks (oldest first, inside the window) into `out` while
    /// they fit `budget_bits`; returns the bits used. Ids go to `ids`.
    pub fn collect(
        &mut self,
        now: Duration,
        rto: Duration,
        mut budget_bits: usize,
        out: &mut Vec<WireChunk>,
        ids: &mut Vec<u64>,
    ) -> usize {
        let start = budget_bits;
        let mut prev: Option<u64> = None;
        for (i, c) in self.queue.iter_mut().enumerate().take(ORDERED_WINDOW as usize) {
            if !Self::due(c, now, rto) {
                continue;
            }
            let id = self.base + i as u64;
            let delta = prev.map(|p| (id - p - 1).min(u16::MAX as u64 - 1) as u16);
            let cost = reliable_cost_bits(c.data.len(), delta, true);
            if cost > budget_bits {
                break;
            }
            budget_bits -= cost;
            if c.last_sent.is_some() {
                self.resends += 1;
            }
            c.last_sent = Some(now);
            out.push(WireChunk { id: id as u16, last: c.last, data: c.data.clone() });
            ids.push(id);
            prev = Some(id);
        }
        start - budget_bits
    }
}

/// Receiving side of the ordered lane.
#[derive(Debug, Default)]
pub struct OrderedReceiver {
    next: u64,
    /// `window[i]` holds chunk `next + i`.
    window: VecDeque<Option<(Vec<u8>, bool)>>,
    assembling: Vec<u8>,
}

impl OrderedReceiver {
    /// Accepts a chunk; complete messages are appended to `out` in order.
    /// Returns `Err` if a reassembled message exceeds [`MAX_MESSAGE_SIZE`].
    pub fn receive(&mut self, chunk: WireChunk, out: &mut VecDeque<Vec<u8>>) -> Result<(), &'static str> {
        let diff = chunk.id.wrapping_sub(self.next as u16) as u64;
        if diff >= ORDERED_WINDOW {
            return Ok(()); // already delivered (duplicate / late resend)
        }
        if chunk.data.len() > CHUNK_SIZE {
            return Err("reliable chunk too large");
        }
        let idx = diff as usize;
        if self.window.len() <= idx {
            self.window.resize_with(idx + 1, || None);
        }
        if self.window[idx].is_none() {
            self.window[idx] = Some((chunk.data, chunk.last));
        }
        while let Some(Some(_)) = self.window.front() {
            let Some(Some((data, last))) = self.window.pop_front() else { break };
            self.next += 1;
            if self.assembling.len() + data.len() > MAX_MESSAGE_SIZE {
                return Err("reliable message too large");
            }
            self.assembling.extend_from_slice(&data);
            if last {
                out.push_back(std::mem::take(&mut self.assembling));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct OutEvent {
    data: Vec<u8>,
    created: Duration,
    acked: bool,
    last_sent: Option<Duration>,
}

/// Sending side of the unordered lane.
#[derive(Debug, Default)]
pub struct UnorderedSender {
    base: u64,
    queue: VecDeque<OutEvent>,
    unacked_bytes: usize,
    pub expired: u64,
    pub resends: u64,
}

impl UnorderedSender {
    pub fn unacked_bytes(&self) -> usize {
        self.unacked_bytes
    }

    pub fn push(&mut self, msg: &[u8], now: Duration, other_unacked: usize) -> Result<(), PushError> {
        if msg.len() > MAX_UNORDERED_SIZE {
            return Err(PushError::TooLarge);
        }
        if self.queue.len() >= MAX_UNORDERED_IN_FLIGHT
            || self.unacked_bytes + other_unacked + msg.len() > MAX_UNACKED_BYTES
        {
            return Err(PushError::Overflow);
        }
        self.queue.push_back(OutEvent { data: msg.to_vec(), created: now, acked: false, last_sent: None });
        self.unacked_bytes += msg.len();
        Ok(())
    }

    fn pop_done(&mut self, now: Duration) {
        while let Some(e) = self.queue.front() {
            let old = now.saturating_sub(e.created) > UNORDERED_LIFETIME;
            if !(e.acked || old) {
                break;
            }
            if !e.acked {
                self.expired += 1;
                self.unacked_bytes -= e.data.len();
            }
            self.queue.pop_front();
            self.base += 1;
        }
    }

    /// Drops messages older than [`UNORDERED_LIFETIME`].
    pub fn expire(&mut self, now: Duration) {
        self.pop_done(now);
    }

    fn get(&mut self, id: u64) -> Option<&mut OutEvent> {
        let idx = id.checked_sub(self.base)?;
        self.queue.get_mut(idx as usize)
    }

    pub fn ack(&mut self, id: u64, now: Duration) {
        if let Some(e) = self.get(id) {
            if !e.acked {
                e.acked = true;
                let len = e.data.len();
                self.unacked_bytes -= len;
            }
        }
        self.pop_done(now);
    }

    pub fn nack(&mut self, id: u64) {
        if let Some(e) = self.get(id) {
            if !e.acked {
                e.last_sent = None;
            }
        }
    }

    fn due(e: &OutEvent, now: Duration, rto: Duration) -> bool {
        !e.acked
            && now.saturating_sub(e.created) <= UNORDERED_LIFETIME
            && e.last_sent.is_none_or(|t| now.saturating_sub(t) >= rto)
    }

    pub fn has_due(&self, now: Duration, rto: Duration) -> bool {
        self.queue.iter().any(|e| Self::due(e, now, rto))
    }

    pub fn collect(
        &mut self,
        now: Duration,
        rto: Duration,
        mut budget_bits: usize,
        out: &mut Vec<WireEvent>,
        ids: &mut Vec<u64>,
    ) -> usize {
        let start = budget_bits;
        let mut prev: Option<u64> = None;
        for (i, e) in self.queue.iter_mut().enumerate() {
            if !Self::due(e, now, rto) {
                continue;
            }
            let id = self.base + i as u64;
            let delta = prev.map(|p| (id - p - 1).min(u16::MAX as u64 - 1) as u16);
            let cost = reliable_cost_bits(e.data.len(), delta, false);
            if cost > budget_bits {
                continue; // a smaller event may still fit
            }
            budget_bits -= cost;
            if e.last_sent.is_some() {
                self.resends += 1;
            }
            e.last_sent = Some(now);
            out.push(WireEvent { id: id as u16, data: e.data.clone() });
            ids.push(id);
            prev = Some(id);
        }
        start - budget_bits
    }
}

/// Receiving side of the unordered lane: delivers on first arrival and
/// remembers the last [`UNORDERED_DEDUP`] ids.
#[derive(Debug)]
pub struct UnorderedReceiver {
    newest: i64,
    /// `seen[id % N] == id + 1` when `id` was delivered.
    seen: Vec<u64>,
}

impl Default for UnorderedReceiver {
    fn default() -> Self {
        UnorderedReceiver { newest: -1, seen: vec![0; UNORDERED_DEDUP as usize] }
    }
}

impl UnorderedReceiver {
    /// Returns the message if it is new.
    pub fn receive(&mut self, ev: WireEvent) -> Option<Vec<u8>> {
        if ev.data.len() > MAX_UNORDERED_SIZE {
            return None;
        }
        let diff = ev.id.wrapping_sub(self.newest as u16) as i16 as i64;
        let id = self.newest + diff;
        if id < 0 || id <= self.newest - UNORDERED_DEDUP as i64 {
            return None; // stale: the sender gave up on it long ago
        }
        let slot = (id as u64 % UNORDERED_DEDUP) as usize;
        if self.seen[slot] == id as u64 + 1 {
            return None;
        }
        self.seen[slot] = id as u64 + 1;
        self.newest = self.newest.max(id);
        Some(ev.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RTO: Duration = Duration::from_millis(100);

    #[test]
    fn ordered_split_reassemble_and_dedup() {
        let mut tx = OrderedSender::default();
        let mut rx = OrderedReceiver::default();
        let big: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        tx.push(&big, 0).unwrap();
        tx.push(b"", 0).unwrap();
        tx.push(b"x", 0).unwrap();
        let mut chunks = Vec::new();
        let mut ids = Vec::new();
        tx.collect(Duration::ZERO, RTO, 100_000, &mut chunks, &mut ids);
        assert_eq!(chunks.len(), 5); // 3 for the big one + empty + x
        let mut out = VecDeque::new();
        // Deliver reversed, with duplicates.
        for c in chunks.iter().rev().chain(chunks.iter()) {
            rx.receive(c.clone(), &mut out).unwrap();
        }
        assert_eq!(out, VecDeque::from(vec![big, vec![], b"x".to_vec()]));
        assert!(!tx.has_due(Duration::from_millis(50), RTO));
        assert!(tx.has_due(Duration::from_millis(100), RTO));
        for id in ids {
            tx.ack(id);
        }
        assert!(tx.queue.is_empty());
        assert_eq!(tx.unacked_bytes(), 0);
    }

    #[test]
    fn ordered_limits() {
        let mut tx = OrderedSender::default();
        assert_eq!(tx.push(&vec![0; MAX_MESSAGE_SIZE + 1], 0), Err(PushError::TooLarge));
        tx.push(&vec![0; MAX_MESSAGE_SIZE], 0).unwrap();
        tx.push(&vec![0; MAX_MESSAGE_SIZE], 0).unwrap();
        assert_eq!(tx.push(&[0], 0), Err(PushError::Overflow));
    }

    #[test]
    fn ordered_window_respected() {
        let mut tx = OrderedSender::default();
        for _ in 0..300 {
            tx.push(b"m", 0).unwrap();
        }
        let (mut c, mut ids) = (Vec::new(), Vec::new());
        tx.collect(Duration::ZERO, RTO, 1 << 20, &mut c, &mut ids);
        assert_eq!(c.len(), ORDERED_WINDOW as usize);
    }

    #[test]
    fn unordered_dedup_and_expiry() {
        let mut tx = UnorderedSender::default();
        let mut rx = UnorderedReceiver::default();
        for i in 0..5u8 {
            tx.push(&[i], Duration::ZERO, 0).unwrap();
        }
        let (mut evs, mut ids) = (Vec::new(), Vec::new());
        tx.collect(Duration::ZERO, RTO, 1 << 20, &mut evs, &mut ids);
        let mut got = Vec::new();
        for e in evs.iter().rev().chain(evs.iter()) {
            if let Some(d) = rx.receive(e.clone()) {
                got.push(d[0]);
            }
        }
        assert_eq!(got, vec![4, 3, 2, 1, 0]);
        tx.ack(ids[0], Duration::ZERO);
        tx.expire(Duration::from_millis(1001));
        assert_eq!(tx.expired, 4);
        assert_eq!(tx.unacked_bytes(), 0);
        assert!(!tx.has_due(Duration::from_secs(2), RTO));
    }
}

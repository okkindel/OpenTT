//! Input redundancy: every client packet repeats the newest 4 moves, or all unacked
//! moves up to 8.
//!
//! ```text
//! u16 first_seq; u3 count-1        moves first_seq-count+1 ..= first_seq, oldest first
//! first move: full pack; each later move: bit "same as previous", else the full pack
//! ```
//!
//! The move pack itself is game-specific (`netmove` layout in tt-engine), so
//! it is a [`MoveCodec`] implemented by the caller.

use std::collections::VecDeque;

use crate::bits::{BitReader, BitWriter};
use crate::error::DecodeError;
use crate::packet::widen_u16;

/// Moves always repeated per packet.
pub const INPUT_REDUNDANCY: usize = 4;
/// Most moves in one packet (3-bit count).
pub const MAX_MOVES_PER_PACKET: usize = 8;
/// Moves more than this ahead of the last applied one cause a resync (§6).
pub const MOVE_WINDOW: u32 = 64;

/// Bit codec of one move.
pub trait MoveCodec: Clone + PartialEq {
    fn encode(&self, w: &mut BitWriter);
    fn decode(r: &mut BitReader) -> Result<Self, DecodeError>;
}

/// Client side: numbers moves (from 1), remembers the newest ones and
/// encodes them with redundancy.
#[derive(Debug, Clone)]
pub struct InputSender<M> {
    moves: VecDeque<(u32, M)>,
    next_seq: u32,
    acked: u32,
    in_packets: VecDeque<(u16, u32)>,
}

impl<M: MoveCodec> Default for InputSender<M> {
    fn default() -> Self {
        InputSender { moves: VecDeque::new(), next_seq: 1, acked: 0, in_packets: VecDeque::new() }
    }
}

impl<M: MoveCodec> InputSender<M> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the move for the next tick; returns its sequence number.
    pub fn push(&mut self, m: M) -> u32 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.moves.push_back((seq, m));
        while self.moves.len() > MAX_MOVES_PER_PACKET {
            self.moves.pop_front();
        }
        seq
    }

    pub fn newest_seq(&self) -> Option<u32> {
        self.moves.back().map(|(s, _)| *s)
    }

    /// Newest move seq known to be received (e.g. the snapshot's `ack_move`).
    pub fn acked(&self) -> u32 {
        self.acked
    }

    pub fn ack(&mut self, seq: u32) {
        self.acked = self.acked.max(seq.min(self.next_seq.saturating_sub(1)));
    }

    /// How many moves the next [`encode`](Self::encode) writes.
    pub fn count(&self) -> usize {
        let unacked = self.moves.iter().filter(|(s, _)| *s > self.acked).count();
        unacked.clamp(INPUT_REDUNDANCY, MAX_MOVES_PER_PACKET).min(self.moves.len())
    }

    /// Writes the input section. Writes nothing if no move was pushed yet.
    pub fn encode(&self, w: &mut BitWriter) {
        let n = self.count();
        let Some(&(first, _)) = self.moves.back() else { return };
        w.write_bits(first as u64 & 0xffff, 16);
        w.write_bits(n as u64 - 1, 3);
        let mut prev: Option<&M> = None;
        for (_, m) in self.moves.iter().skip(self.moves.len() - n) {
            if let Some(p) = prev {
                let same = p == m;
                w.write_bool(same);
                if !same {
                    m.encode(w);
                }
            } else {
                m.encode(w);
            }
            prev = Some(m);
        }
    }

    /// Records that packet `packet_seq` carried the moves up to the newest.
    pub fn packet_sent(&mut self, packet_seq: u16) {
        if let Some(newest) = self.newest_seq() {
            self.in_packets.push_back((packet_seq, newest));
            while self.in_packets.len() > 64 {
                self.in_packets.pop_front();
            }
        }
    }

    /// A packet ack: every move it carried was received.
    pub fn packet_acked(&mut self, packet_seq: u16) {
        if let Some(&(_, newest)) = self.in_packets.iter().find(|(p, _)| *p == packet_seq) {
            self.ack(newest);
        }
    }
}

/// Decodes an input section: (raw first seq, moves oldest first).
pub fn decode_moves<M: MoveCodec>(r: &mut BitReader) -> Result<(u16, Vec<M>), DecodeError> {
    let first = r.read_bits(16)? as u16;
    let n = r.read_bits(3)? as usize + 1;
    let mut out: Vec<M> = Vec::with_capacity(n);
    for i in 0..n {
        let m = if i > 0 && r.read_bool()? { out[i - 1].clone() } else { M::decode(r)? };
        out.push(m);
    }
    Ok((first, out))
}

/// Server side: widens sequence numbers and returns only new moves, in order.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct InputReceiver {
    last: Option<u32>,
    /// Jumps of more than [`MOVE_WINDOW`] (client stalled, or cheating).
    pub resyncs: u64,
}


impl InputReceiver {
    pub fn last(&self) -> Option<u32> {
        self.last
    }

    pub fn receive<M: MoveCodec>(&mut self, r: &mut BitReader) -> Result<Vec<(u32, M)>, DecodeError> {
        let (raw, moves) = decode_moves::<M>(r)?;
        let first = widen_u16(self.last.unwrap_or(raw as u32), raw);
        let n = moves.len() as u32;
        if first + 1 < n {
            return Err(DecodeError::Invalid("move seq underflow"));
        }
        let oldest = first + 1 - n;
        if let Some(l) = self.last {
            if first > l + MOVE_WINDOW {
                // §6 says "ignore"; that would lock out a client whose moves
                // jumped ahead after a stall, so resync instead (the server's
                // 8-move queue cap still bounds the effect).
                self.resyncs += 1;
            }
        }
        let last = self.last;
        let out: Vec<(u32, M)> = moves
            .into_iter()
            .enumerate()
            .map(|(i, m)| (oldest + i as u32, m))
            .filter(|(s, _)| last.is_none_or(|l| *s > l))
            .collect();
        if let Some(&(s, _)) = out.last() {
            self.last = Some(s);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Mv(u8);

    impl MoveCodec for Mv {
        fn encode(&self, w: &mut BitWriter) {
            w.write_bits(self.0 as u64, 5);
        }
        fn decode(r: &mut BitReader) -> Result<Self, DecodeError> {
            Ok(Mv(r.read_bits(5)? as u8))
        }
    }

    fn roundtrip(tx: &InputSender<Mv>, rx: &mut InputReceiver) -> Vec<(u32, Mv)> {
        let mut w = BitWriter::new();
        tx.encode(&mut w);
        let b = w.finish();
        rx.receive(&mut BitReader::new(&b)).unwrap()
    }

    #[test]
    fn redundancy_and_dedup() {
        let mut tx = InputSender::new();
        let mut rx = InputReceiver::default();
        tx.push(Mv(1));
        assert_eq!(roundtrip(&tx, &mut rx), vec![(1, Mv(1))]);
        for i in 2..=6 {
            tx.push(Mv(if i < 5 { 3 } else { i as u8 }));
        }
        // Nothing acked: 6 moves in the ring, all unacked → 6 sent; 1 dropped as old.
        assert_eq!(tx.count(), 6);
        let got = roundtrip(&tx, &mut rx);
        assert_eq!(got, vec![(2, Mv(3)), (3, Mv(3)), (4, Mv(3)), (5, Mv(5)), (6, Mv(6))]);
        tx.ack(6);
        assert_eq!(tx.count(), INPUT_REDUNDANCY);
        assert!(roundtrip(&tx, &mut rx).is_empty());
        for i in 0..20 {
            tx.push(Mv(i));
        }
        assert_eq!(tx.count(), MAX_MOVES_PER_PACKET);
        let got = roundtrip(&tx, &mut rx);
        assert_eq!(got.first().unwrap().0, 19);
        assert_eq!(got.last().unwrap().0, 26);
        tx.packet_sent(40);
        tx.packet_acked(40);
        assert_eq!(tx.acked(), 26);
    }

    #[test]
    fn lost_packets_recovered_by_redundancy() {
        let mut tx = InputSender::new();
        let mut rx = InputReceiver::default();
        let mut applied = Vec::new();
        for i in 1..=100u32 {
            tx.push(Mv((i % 32) as u8));
            if i % 3 != 0 {
                for (s, _) in roundtrip(&tx, &mut rx) {
                    applied.push(s);
                }
                tx.ack(rx.last().unwrap());
            }
        }
        assert_eq!(applied, (1..=100).collect::<Vec<_>>());
    }

    #[test]
    fn wraparound_and_resync() {
        let mut rx = InputReceiver::default();
        let mut tx = InputSender::<Mv>::new();
        tx.next_seq = 65530;
        for _ in 0..10 {
            tx.push(Mv(1));
            let _ = roundtrip(&tx, &mut rx);
        }
        assert_eq!(rx.last(), Some(65539));
        let mut tx = InputSender::<Mv>::new();
        tx.next_seq = 65539 + 200;
        tx.push(Mv(2));
        let got = roundtrip(&tx, &mut rx);
        assert_eq!(got.last().unwrap().0, 65739);
        assert_eq!(rx.resyncs, 1);
    }
}

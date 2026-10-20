//! Delayed datagrams go out on the next `send_to`/`recv_from` once due,
//! so the resolution is the caller's tick. Without conditions a
//! [`SimLink`] is a plain pass-through.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::rng::Rng;
use crate::transport::{Datagram, LinkConditions};

type Queued = Reverse<(Duration, u64, SocketAddr, Vec<u8>)>;

#[derive(Debug)]
struct SimState {
    now: Duration,
    /// Conditions for every destination without its own.
    default: Option<LinkConditions>,
    per_addr: HashMap<SocketAddr, LinkConditions>,
    rng: Rng,
    /// Delayed datagrams: (due, order, to, bytes).
    queue: BinaryHeap<Queued>,
    counter: u64,
    /// Last due time per destination (jitter never reorders).
    last_due: HashMap<SocketAddr, Duration>,
    dropped: u64,
}

/// Shared control of a [`SimLink`]: its clock and conditions. The engine
/// keeps it while the link itself sits inside a `Server`/`Client`.
#[derive(Debug, Clone)]
pub struct SimHandle(Arc<Mutex<SimState>>);

impl SimHandle {
    fn lock(&self) -> MutexGuard<'_, SimState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The time queued datagrams are released against (set before
    /// `update`/`flush`).
    pub fn set_now(&self, now: Duration) {
        self.lock().now = now;
    }

    /// Conditions for datagrams to `to`, or to every destination (`None`);
    /// `c = None` removes them.
    pub fn set_conditions(&self, to: Option<SocketAddr>, c: Option<LinkConditions>) {
        let mut s = self.lock();
        match (to, c) {
            (None, c) => s.default = c,
            (Some(a), Some(c)) => {
                s.per_addr.insert(a, c);
            }
            (Some(a), None) => {
                s.per_addr.remove(&a);
            }
        }
    }

    /// Any conditions set.
    pub fn active(&self) -> bool {
        let s = self.lock();
        s.default.is_some() || !s.per_addr.is_empty()
    }

    /// Datagrams dropped by the simulated loss.
    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }

}

/// See the module docs.
#[derive(Debug)]
pub struct SimLink<T: Datagram> {
    inner: T,
    state: SimHandle,
}

impl<T: Datagram> SimLink<T> {
    pub fn new(inner: T, seed: u64) -> (Self, SimHandle) {
        let state = SimHandle(Arc::new(Mutex::new(SimState {
            now: Duration::ZERO,
            default: None,
            per_addr: HashMap::new(),
            rng: Rng::new(seed),
            queue: BinaryHeap::new(),
            counter: 0,
            last_due: HashMap::new(),
            dropped: 0,
        })));
        (SimLink { inner, state: state.clone() }, state)
    }

    fn release(&mut self) {
        let due: Vec<(SocketAddr, Vec<u8>)> = {
            let mut s = self.state.lock();
            let now = s.now;
            let mut out = Vec::new();
            while s.queue.peek().is_some_and(|Reverse((t, ..))| *t <= now) {
                if let Some(Reverse((_, _, to, d))) = s.queue.pop() {
                    out.push((to, d));
                }
            }
            out
        };
        for (to, d) in due {
            let _ = self.inner.send_to(&d, to);
        }
    }
}

impl<T: Datagram> Datagram for SimLink<T> {
    fn send_to(&mut self, data: &[u8], addr: SocketAddr) -> io::Result<()> {
        self.release();
        let queued = {
            let mut s = self.state.lock();
            match s.per_addr.get(&addr).cloned().or_else(|| s.default.clone()) {
                None => false,
                Some(c) => {
                    if s.rng.chance(c.loss) {
                        s.dropped += 1;
                        return Ok(());
                    }
                    let j = c.jitter.as_secs_f64();
                    let extra = if j > 0.0 { (s.rng.next_f64() * 2.0 - 1.0) * j } else { 0.0 };
                    let mut due = s.now + Duration::from_secs_f64((c.latency.as_secs_f64() + extra).max(0.0));
                    if let Some(&last) = s.last_due.get(&addr) {
                        due = due.max(last);
                    }
                    s.last_due.insert(addr, due);
                    if due <= s.now {
                        false
                    } else {
                        s.counter += 1;
                        let k = s.counter;
                        s.queue.push(Reverse((due, k, addr, data.to_vec())));
                        true
                    }
                }
            }
        };
        if queued {
            Ok(())
        } else {
            self.inner.send_to(data, addr)
        }
    }

    fn recv_from(&mut self, buf: &mut [u8]) -> Option<(usize, SocketAddr)> {
        self.release();
        self.inner.recv_from(buf)
    }

    fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemNet;

    #[test]
    fn delays_and_drops_outgoing_only() {
        let net = MemNet::new(1);
        let a: SocketAddr = "10.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "10.0.0.2:2".parse().unwrap();
        let (mut sa, h) = SimLink::new(net.endpoint(a), 7);
        let mut eb = net.endpoint(b);
        let mut buf = [0u8; 64];
        // Pass-through.
        sa.send_to(b"x", b).unwrap();
        assert_eq!(eb.recv_from(&mut buf).map(|r| r.0), Some(1));
        // 100 ms delay to b.
        h.set_conditions(Some(b), Some(LinkConditions { latency: Duration::from_millis(100), ..Default::default() }));
        sa.send_to(b"y", b).unwrap();
        assert!(eb.recv_from(&mut buf).is_none());
        h.set_now(Duration::from_millis(99));
        sa.recv_from(&mut buf);
        assert!(eb.recv_from(&mut buf).is_none());
        h.set_now(Duration::from_millis(100));
        sa.recv_from(&mut buf);
        assert_eq!(eb.recv_from(&mut buf).map(|r| r.0), Some(1));
        // Incoming is not affected.
        eb.send_to(b"z", a).unwrap();
        assert!(sa.recv_from(&mut buf).is_some());
        // 50 % loss for everyone.
        h.set_conditions(Some(b), None);
        h.set_conditions(None, Some(LinkConditions { loss: 0.5, ..Default::default() }));
        for _ in 0..1000 {
            sa.send_to(b"w", b).unwrap();
        }
        let mut got = 0;
        while eb.recv_from(&mut buf).is_some() {
            got += 1;
        }
        assert!((400..600).contains(&got), "{got}");
        assert_eq!(h.dropped(), 1000 - got);
        assert!(h.active());
    }
}

//! Per-connection statistics (for `--net-stats`, `getPing`, `getPacketLoss`).

use std::collections::VecDeque;
use std::time::Duration;

/// Window for the bytes/s rates.
pub const RATE_WINDOW: Duration = Duration::from_secs(1);
/// Loss is the fraction of lost packets among the last this many decided.
pub const LOSS_WINDOW: usize = 64;

/// A snapshot of a connection's counters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnStats {
    /// Smoothed round-trip time (EWMA of send→ack, α = 1/8).
    pub rtt: Duration,
    /// Smoothed RTT deviation (jitter estimate).
    pub rtt_var: Duration,
    /// Packet loss over the last [`LOSS_WINDOW`] decided packets (0..=1).
    pub loss: f32,
    /// Bytes (UDP payload) per second, over the last [`RATE_WINDOW`].
    pub bytes_in_per_sec: f64,
    pub bytes_out_per_sec: f64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Reliable chunks/events sent again.
    pub resends: u64,
    /// Unreliable messages that did not fit in a packet.
    pub unreliable_dropped: u64,
    /// Unordered reliable events dropped after their lifetime.
    pub events_expired: u64,
    /// Duplicate or too-old packets ignored.
    pub duplicates: u64,
    /// Packets from the peer's address that failed to decode or had a wrong session.
    pub malformed: u64,
    /// Packets acked by the peer / declared lost (totals).
    pub acked: u64,
    pub lost: u64,
}

/// Sliding-window byte counter.
#[derive(Debug, Default)]
pub struct RateMeter {
    samples: VecDeque<(Duration, u32)>,
    sum: u64,
}

impl RateMeter {
    pub fn record(&mut self, now: Duration, bytes: usize) {
        self.samples.push_back((now, bytes as u32));
        self.sum += bytes as u64;
        self.prune(now);
    }

    fn prune(&mut self, now: Duration) {
        while let Some(&(t, b)) = self.samples.front() {
            if now.saturating_sub(t) < RATE_WINDOW {
                break;
            }
            self.sum -= b as u64;
            self.samples.pop_front();
        }
    }

    pub fn per_sec(&mut self, now: Duration) -> f64 {
        self.prune(now);
        self.sum as f64 / RATE_WINDOW.as_secs_f64()
    }
}

/// RTT estimator (RFC 6298 style smoothing).
#[derive(Debug, Clone)]
pub struct Rtt {
    srtt: f64,
    var: f64,
    sampled: bool,
}

impl Rtt {
    pub fn new(initial: Duration) -> Self {
        let s = initial.as_secs_f64();
        Rtt { srtt: s, var: s / 2.0, sampled: false }
    }

    pub fn sample(&mut self, rtt: Duration) {
        let s = rtt.as_secs_f64();
        if !self.sampled {
            self.srtt = s;
            self.var = s / 2.0;
            self.sampled = true;
        } else {
            self.var = 0.75 * self.var + 0.25 * (self.srtt - s).abs();
            self.srtt = 0.875 * self.srtt + 0.125 * s;
        }
    }

    pub fn srtt(&self) -> Duration {
        Duration::from_secs_f64(self.srtt)
    }

    pub fn var(&self) -> Duration {
        Duration::from_secs_f64(self.var)
    }

    /// Reliable resend timeout: `1.25·RTT + 32 ms`, capped at 1 s.
    pub fn rto(&self) -> Duration {
        Duration::from_secs_f64((1.25 * self.srtt + 0.032).min(1.0))
    }
}

/// Packet fates over the last [`LOSS_WINDOW`].
#[derive(Debug, Default)]
pub struct LossWindow {
    fates: VecDeque<bool>,
    lost: usize,
}

impl LossWindow {
    pub fn record(&mut self, lost: bool) {
        self.fates.push_back(lost);
        self.lost += lost as usize;
        if self.fates.len() > LOSS_WINDOW
            && self.fates.pop_front() == Some(true) {
                self.lost -= 1;
            }
    }

    pub fn loss(&self) -> f32 {
        if self.fates.is_empty() {
            0.0
        } else {
            self.lost as f32 / self.fates.len() as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_rtt_loss() {
        let mut m = RateMeter::default();
        for i in 0..20 {
            m.record(Duration::from_millis(i * 100), 100);
        }
        // Samples at 1.0..=1.9 s are inside the window at 1.95 s.
        assert_eq!(m.per_sec(Duration::from_millis(1950)), 1000.0);
        assert_eq!(m.per_sec(Duration::from_secs(10)), 0.0);

        let mut r = Rtt::new(Duration::from_millis(100));
        r.sample(Duration::from_millis(40));
        assert_eq!(r.srtt(), Duration::from_millis(40));
        for _ in 0..100 {
            r.sample(Duration::from_millis(80));
        }
        assert!((r.srtt().as_secs_f64() - 0.08).abs() < 0.001);
        assert!(r.rto() >= Duration::from_millis(131));

        let mut l = LossWindow::default();
        for i in 0..200 {
            l.record(i % 10 == 0);
        }
        assert!((l.loss() - 0.1).abs() < 0.03);
    }
}

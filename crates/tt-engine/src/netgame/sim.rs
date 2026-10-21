//! On real connections: the server and client sockets sit behind a
//! [`tt_net::SimLink`], a pass-through until conditions are set. The game's
//! `simLag(loss, lag)` sets half the lag on both ends. A developer feature:
//! compiled into debug builds and with the `net-sim` cargo feature; elsewhere the
//! call only logs.

use std::time::Duration;

use super::now;
use crate::host::EngineHost;
use tt_net::{LinkConditions, SimHandle};
use crate::net::Conn;

/// Simulated link controls of the open sockets.
#[derive(Default)]
pub struct Sims {
    pub server: Option<SimHandle>,
    pub client: Option<SimHandle>,
}

impl Sims {
    /// Advances both clocks (before receiving and before sending).
    pub fn set_now(&self, t: Duration) {
        for h in self.server.iter().chain(self.client.iter()) {
            h.set_now(t);
        }
    }
}

/// Compiled in (debug builds, `--features net-sim`).
pub const AVAILABLE: bool = cfg!(any(debug_assertions, feature = "net-sim"));

/// The conditions of `(packetLoss, delay ms)`; `None` for a perfect link.
pub fn conditions(loss: f64, delay_ms: f64) -> Option<LinkConditions> {
    let loss = if loss.is_finite() { loss.clamp(0.0, 1.0) } else { 0.0 };
    let delay = if delay_ms.is_finite() { delay_ms.clamp(0.0, 10_000.0) } else { 0.0 };
    (loss > 0.0 || delay > 0.0).then(|| LinkConditions { loss, latency: Duration::from_secs_f64(delay / 1000.0), ..Default::default() })
}

/// `conn.setSimulatedNetParams(loss, delay)`: true if `conn` is a network
/// connection (ours to a server, or a remote client's).
pub fn set_params(h: &mut EngineHost, conn: Conn, loss: f64, delay_ms: f64) -> bool {
    let c = conditions(loss, delay_ms);
    let (handle, to) = if let Some(r) = h.netgame.remotes.get(&conn) {
        (h.netgame.sim.server.clone(), Some(r.addr))
    } else if super::is_remote_server(h, conn) {
        (h.netgame.sim.client.clone(), None)
    } else {
        return false;
    };
    if !AVAILABLE {
        h.log.warn("setSimulatedNetParams: not in this build (debug builds or --features net-sim)");
        return true;
    }
    let Some(handle) = handle else { return true };
    handle.set_now(now(h));
    h.log.info(&format!("Simulated network on connection {conn}: {:.0} % loss, {delay_ms:.0} ms delay", c.as_ref().map_or(0.0, |c| c.loss * 100.0)));
    handle.set_conditions(to, c);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params() {
        assert_eq!(conditions(0.0, 0.0), None);
        let c = conditions(0.25, 120.0).unwrap();
        assert_eq!((c.loss, c.latency), (0.25, Duration::from_millis(120)));
        assert_eq!(conditions(7.0, -1.0).unwrap().loss, 1.0);
        assert_eq!(conditions(f64::NAN, 0.0), None);
    }
}

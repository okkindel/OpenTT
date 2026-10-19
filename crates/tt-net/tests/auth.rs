//! The join password proof in the handshake: right and wrong passwords,
//! and a recorded `ChallengeResponse` replayed from another address.

use std::net::SocketAddr;
use std::time::Duration;
use tt_net::packet::Packet;
use tt_net::*;

fn a(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

/// Runs a connect attempt; the server accepts when `verify(password)` holds.
fn attempt(net: &MemNet, t: &mut Duration, server: &mut Server<MemLink>, client_addr: &str, secret: u64, password: &str) -> (bool, Vec<auth::Handshake>) {
    let mut c = Client::with_seed(net.endpoint(a(client_addr)), ClientConfig::default(), t.as_millis() as u64 + 1);
    c.set_auth_secret(secret);
    c.connect(a("10.0.0.1:28000"), b"args", *t).unwrap();
    let mut seen = Vec::new();
    for _ in 0..100 {
        *t += Duration::from_millis(20);
        net.set_now(*t);
        server.update(*t);
        for e in server.drain_events() {
            if let ServerEvent::ConnectRequest { pending, handshake, .. } = e {
                seen.push(handshake);
                if handshake.verify(password) {
                    server.accept(pending, *t);
                } else {
                    server.reject(pending, "CHR_PASSWORD", *t);
                }
            }
        }
        server.flush(*t);
        c.update(*t);
        c.flush(*t);
        for e in c.drain_events() {
            match e {
                ClientEvent::Connected => return (true, seen),
                ClientEvent::Disconnected(_) => return (false, seen),
                _ => {}
            }
        }
    }
    (false, seen)
}

#[test]
fn password_proof_is_bound_to_the_handshake() {
    let net = MemNet::new(2);
    net.set_capture(true);
    let mut t = Duration::ZERO;
    let mut server = Server::with_seed(net.endpoint(a("10.0.0.1:28000")), ServerConfig::default(), 3);
    let (ok, h1) = attempt(&net, &mut t, &mut server, "10.0.0.2:5000", auth::secret("sesame"), "sesame");
    assert!(ok);
    let (ok, _) = attempt(&net, &mut t, &mut server, "10.0.0.3:5000", auth::secret("guess"), "sesame");
    assert!(!ok, "wrong password");
    let (ok, _) = attempt(&net, &mut t, &mut server, "10.0.0.4:5000", 0, "sesame");
    assert!(!ok, "no password");
    let (ok, h2) = attempt(&net, &mut t, &mut server, "10.0.0.5:5000", auth::secret("sesame"), "sesame");
    assert!(ok);
    assert_ne!(h1[0].proof, h2[0].proof, "a new proof per handshake");
    // Neither the password nor its secret is on the wire.
    let sent = net.take_captured();
    let secret = auth::secret("sesame").to_le_bytes();
    assert!(sent.iter().all(|(_, _, d)| !d.windows(6).any(|w| w == b"sesame") && !d.windows(8).any(|w| w == secret)));
    // The good ChallengeResponse replayed from another address: no new request.
    let resp = sent.iter().find(|(from, _, d)| *from == a("10.0.0.2:5000") && matches!(Packet::decode(d), Ok(Packet::ChallengeResponse { .. }))).unwrap().2.clone();
    net.inject(a("10.0.0.9:5000"), a("10.0.0.1:28000"), &resp);
    t += Duration::from_millis(20);
    net.set_now(t);
    server.update(t);
    assert!(!server.drain_events().iter().any(|e| matches!(e, ServerEvent::ConnectRequest { .. })));
    assert!(server.endpoint_stats().bad_cookie >= 1);
}

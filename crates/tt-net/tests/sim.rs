//! Integration tests over the deterministic link simulator.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::*;
use tt_net::packet::{Packet, CONNECT_REQUEST_SIZE, INFO_QUERY_SIZE};
use tt_net::*;

fn lossy(loss: f64) -> LinkConditions {
    LinkConditions {
        latency: Duration::from_millis(50),
        jitter: Duration::from_millis(20),
        loss,
        duplicate: 0.05,
        reorder: 0.1,
        reorder_delay: Duration::from_millis(40),
        ..Default::default()
    }
}

#[test]
fn connect_and_exchange() {
    let mut s = Sim::new(1, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    assert_eq!(s.cev[0][0], ClientEvent::Connected);
    let conn = s.server_conns()[0];
    assert!(s.sev.iter().any(|e| matches!(e, ServerEvent::ConnectRequest { payload, .. } if payload == b"name=tester")));
    assert!(s.sev.iter().any(|e| matches!(e, ServerEvent::Connected { conn: c, addr: a } if *c == conn && *a == addr(2, 5000))));
    let now = s.now();
    s.server.send(conn, Lane::Reliable, b"MissionStartPhase1", now).unwrap();
    s.server.send(conn, Lane::Event, b"ProjectileSpawn", now).unwrap();
    s.server.send(conn, Lane::Unreliable, b"snapshot", now).unwrap();
    s.clients[0].send(Lane::Reliable, b"MissionStartPhase1Ack", now).unwrap();
    s.run(Duration::from_millis(100));
    assert_eq!(s.client_messages(0, Lane::Reliable), vec![b"MissionStartPhase1".to_vec()]);
    assert_eq!(s.client_messages(0, Lane::Event), vec![b"ProjectileSpawn".to_vec()]);
    assert_eq!(s.client_messages(0, Lane::Unreliable), vec![b"snapshot".to_vec()]);
    assert_eq!(s.server_messages(conn, Lane::Reliable), vec![b"MissionStartPhase1Ack".to_vec()]);
    // Packet acks are reported for baselines.
    assert!(!s.server.take_acked(conn).is_empty());
    assert!(!s.clients[0].take_acked().is_empty());
}

/// §7: many messages under heavy loss, reordering and duplication arrive
/// exactly once and in order, in both directions, fragmented ones included.
fn reliable_stress(seed: u64, loss: f64, count: u32, max_len: u32) {
    let mut s = Sim::new(seed, lossy(loss));
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let conn = s.server_conns()[0];
    let (mut sent_down, mut sent_up) = (0u32, 0u32);
    let (mut got_down, mut got_up) = (Vec::new(), Vec::new());
    let mut events = HashSet::new();
    let mut event_dups = 0;
    let mut events_sent = 0usize;
    let mut ticks = 0u32;
    while (got_down.len() as u32) < count || (got_up.len() as u32) < count {
        ticks += 1;
        assert!(ticks < 200_000, "stalled: down {} up {}", got_down.len(), got_up.len());
        let now = s.now();
        // Feed while staying below the unacked cap (send() past it drops the peer).
        for _ in 0..8 {
            let room = |backlog: usize| backlog + max_len as usize + 4 + 2048 < 64 * 1024;
            if sent_down < count && room(s.server.reliable_backlog(conn).unwrap()) {
                s.server.send(conn, Lane::Reliable, &payload(sent_down, max_len), now).unwrap();
                sent_down += 1;
            }
            if sent_up < count && room(s.clients[0].reliable_backlog()) {
                s.clients[0].send(Lane::Reliable, &payload(sent_up, max_len), now).unwrap();
                sent_up += 1;
            }
        }
        if ticks.is_multiple_of(4) {
            s.server.send(conn, Lane::Event, &(ticks / 4).to_le_bytes(), now).unwrap();
            events_sent += 1;
        }
        s.server.send(conn, Lane::Unreliable, &[0u8; 100], now).unwrap();
        s.step(TICK);
        got_down.extend(s.client_messages(0, Lane::Reliable));
        got_up.extend(s.server_messages(conn, Lane::Reliable));
        for e in s.client_messages(0, Lane::Event) {
            if !events.insert(e) {
                event_dups += 1;
            }
        }
        // The unacked cap must make the feeder back off, never drop.
        assert!(s.clients[0].is_connected(), "{:?}", s.client_disconnect_reason(0));
    }
    s.run(Duration::from_secs(2));
    got_down.extend(s.client_messages(0, Lane::Reliable));
    got_up.extend(s.server_messages(conn, Lane::Reliable));
    for e in s.client_messages(0, Lane::Event) {
        if !events.insert(e) {
            event_dups += 1;
        }
    }
    let expect: Vec<Vec<u8>> = (0..count).map(|k| payload(k, max_len)).collect();
    assert!(got_down == expect, "downstream differs");
    assert!(got_up == expect, "upstream differs");
    assert_eq!(event_dups, 0);
    // Events: at most once; with 1 s lifetime nearly all make it.
    assert!(events.len() * 100 >= events_sent * 99, "events delivered: {} of {events_sent}", events.len());
    let now = s.now();
    let st = s.server.stats(conn, now).unwrap();
    assert!(st.resends > 0 && st.duplicates > 0, "{st:?}");
}

#[test]
fn reliable_10k_messages_30pct_loss() {
    reliable_stress(11, 0.30, 10_000, 40);
}

#[test]
fn reliable_fragmented_20pct_loss() {
    reliable_stress(12, 0.20, 600, 3000);
}

#[test]
fn unreliable_delivery_and_stats() {
    let mut cond = LinkConditions { latency: Duration::from_millis(40), loss: 0.10, ..Default::default() };
    let mut s = Sim::new(3, cond.clone());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let conn = s.server_conns()[0];
    let mut got = 0;
    for i in 0..1000u32 {
        let now = s.now();
        s.server.send(conn, Lane::Unreliable, &i.to_le_bytes(), now).unwrap();
        s.clients[0].send(Lane::Unreliable, &[1; 10], now).unwrap();
        s.step(Duration::from_millis(32));
        got += s.client_messages(0, Lane::Unreliable).len();
    }
    s.run(Duration::from_secs(2));
    got += s.client_messages(0, Lane::Unreliable).len();
    assert!((850..=950).contains(&got), "delivered {got}");
    let now = s.now();
    let st = s.server.stats(conn, now).unwrap();
    // RTT = 80 ms + the 32 ms step quantisation on each side.
    assert!(st.rtt >= Duration::from_millis(75) && st.rtt <= Duration::from_millis(150), "{st:?}");
    assert!(st.loss > 0.03 && st.loss < 0.25, "{st:?}");
    assert!(st.lost > 50 && st.acked > 700, "{st:?}");
    let ls = s.net.link_stats(server_addr(), addr(2, 5000));
    assert!(ls.lost > 50);
    // Rates over the last second while idle (keepalives only) are small but nonzero.
    assert!(st.bytes_out_per_sec > 0.0 && st.bytes_out_per_sec < 200.0, "{st:?}");
    // While streaming: ≈ 31.25 packets/s × (13 header + ~7 body).
    cond.loss = 0.0;
    s.net.set_conditions(cond);
    for i in 0..100u32 {
        let now = s.now();
        s.server.send(conn, Lane::Unreliable, &i.to_le_bytes(), now).unwrap();
        s.clients[0].send(Lane::Unreliable, &[1; 10], now).unwrap();
        s.step(Duration::from_millis(32));
    }
    let now = s.now();
    let st = s.server.stats(conn, now).unwrap();
    assert!(st.bytes_out_per_sec > 500.0 && st.bytes_out_per_sec < 800.0, "{st:?}");
    assert!(st.bytes_in_per_sec > 600.0 && st.bytes_in_per_sec < 1000.0, "{st:?}");
    let cst = s.clients[0].stats(now).unwrap();
    assert!(cst.packets_in > 900 && cst.bytes_in > 0);
}

#[test]
fn idle_connection_stays_up_with_keepalives() {
    let mut s = Sim::new(4, LinkConditions::dsl());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    s.run(Duration::from_secs(30));
    assert!(s.clients[0].is_connected());
    assert_eq!(s.server.connection_count(), 1);
    // About 4 keepalives (or their acks) per second each way, not more.
    let now = s.now();
    let st = s.clients[0].stats(now).unwrap();
    assert!(st.packets_out >= 110 && st.packets_out <= 170, "{st:?}");
    // RTT from keepalive → immediate ack: 120 ms + ≤ 2 steps of 16 ms.
    assert!(st.rtt >= Duration::from_millis(110) && st.rtt <= Duration::from_millis(160), "{st:?}");
}

#[test]
fn timeouts_on_both_sides() {
    let mut s = Sim::new(5, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    s.net.set_conditions(LinkConditions { loss: 1.0, ..Default::default() });
    s.run(Duration::from_millis(9900));
    assert!(s.clients[0].is_connected());
    s.run(Duration::from_millis(300));
    assert_eq!(s.client_disconnect_reason(0), Some(DisconnectReason::Timeout));
    assert_eq!(s.server_disconnects(), vec![(ConnId(0), DisconnectReason::Timeout)]);
    assert_eq!(s.server.connection_count(), 0);
}

#[test]
fn loading_timeout_is_configurable() {
    let mut s = Sim::new(5, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let conn = s.server_conns()[0];
    s.server.set_timeout(conn, Duration::from_secs(15));
    s.clients[0].set_timeout(Duration::from_secs(15));
    s.net.set_conditions(LinkConditions { loss: 1.0, ..Default::default() });
    s.run(Duration::from_secs(12));
    assert!(s.clients[0].is_connected() && s.server.connection_count() == 1);
    s.run(Duration::from_secs(4));
    assert!(!s.clients[0].is_connected() && s.server.connection_count() == 0);
}

#[test]
fn graceful_disconnect_both_directions() {
    let mut s = Sim::new(6, LinkConditions::dsl());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.add_client(addr(3, 5000), ClientConfig::default());
    s.connect_all();
    let mut conns = s.server_conns();
    conns.sort_by_key(|c| s.server.addr(*c));
    let now = s.now();
    s.clients[0].disconnect("quit", now);
    assert_eq!(s.client_disconnect_reason(0), None); // events are drained in step
    s.run(Duration::from_millis(200));
    assert_eq!(s.client_disconnect_reason(0), Some(DisconnectReason::Local("quit".into())));
    assert_eq!(s.server_disconnects(), vec![(conns[0], DisconnectReason::Remote("quit".into()))]);
    let now = s.now();
    s.server.disconnect(conns[1], "kicked", now);
    s.run(Duration::from_millis(200));
    assert_eq!(s.client_disconnect_reason(1), Some(DisconnectReason::Remote("kicked".into())));
    assert_eq!(s.server.connection_count(), 0);
    // Late packets from the old connection are ignored.
    s.run(Duration::from_secs(1));
    assert_eq!(s.server_disconnects().len(), 2);
}

#[test]
fn connect_timeout_without_server() {
    let net = MemNet::new(1);
    let mut c = Client::with_seed(net.endpoint(addr(2, 5000)), ClientConfig::default(), 1);
    c.connect(addr(9, 28000), b"", net.now()).unwrap();
    let mut events = Vec::new();
    while net.now() < Duration::from_secs(6) {
        let now = net.advance(TICK);
        c.update(now);
        events.extend(c.drain_events());
    }
    assert_eq!(events, vec![ClientEvent::Disconnected(DisconnectReason::ConnectTimeout)]);
    // Retries every 250 ms.
    let st = net.link_stats(addr(2, 5000), addr(9, 28000));
    assert!((19..=22).contains(&st.sent), "{st:?}");
}

#[test]
fn version_mismatch_rejected() {
    let mut s = Sim::new(7, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig { protocol: PROTOCOL_VERSION + 1, ..Default::default() });
    s.add_client(addr(3, 5000), ClientConfig { build: BUILD_ID ^ 1, ..Default::default() });
    s.connect(0);
    s.connect(1);
    s.run(Duration::from_millis(100));
    for i in 0..2 {
        assert_eq!(s.client_disconnect_reason(i), Some(DisconnectReason::Rejected("CR_WRONGVERSION".into())));
    }
    assert!(s.sev.is_empty(), "no server state or events: {:?}", s.sev);
    assert_eq!(s.server.endpoint_stats().version_rejects, 2);
}

#[test]
fn server_full_and_app_reject() {
    let cfg = ServerConfig { max_clients: 1, ..Default::default() };
    let mut s = Sim::with_config(8, LinkConditions::lan(), cfg);
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.add_client(addr(3, 5000), ClientConfig::default());
    s.connect(0);
    s.run(Duration::from_millis(100));
    s.connect(1);
    s.run(Duration::from_millis(100));
    assert!(s.clients[0].is_connected());
    assert_eq!(s.client_disconnect_reason(1), Some(DisconnectReason::Rejected("CR_SERVERFULL".into())));

    let mut s = Sim::new(9, LinkConditions::lan());
    s.auto_reject = Some("CR_INVALID_PASSWORD".into());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect(0);
    s.run(Duration::from_millis(100));
    assert_eq!(s.client_disconnect_reason(0), Some(DisconnectReason::Rejected("CR_INVALID_PASSWORD".into())));
    assert_eq!(s.server.connection_count(), 0);
}

#[test]
fn handshake_survives_loss_and_accept_loss() {
    let mut s = Sim::new(10, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    // Server → client dead for 600 ms: Challenge and later Accept get lost.
    s.net.set_link(server_addr(), addr(2, 5000), LinkConditions { loss: 1.0, ..Default::default() });
    s.connect(0);
    s.run(Duration::from_millis(600));
    s.net.set_link(server_addr(), addr(2, 5000), LinkConditions::lan());
    s.run_until(Duration::from_secs(3), |s| s.clients[0].is_connected());
    s.run(Duration::from_secs(1));
    let connected = s.sev.iter().filter(|e| matches!(e, ServerEvent::Connected { .. })).count();
    assert_eq!(connected, 1);
    assert_eq!(s.server.connection_count(), 1);

    // Same, but Accept itself is lost: the server must repeat it.
    let mut s = Sim::new(10, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.auto_accept = false;
    s.connect(0);
    s.run(Duration::from_millis(100));
    let pending = s
        .sev
        .iter()
        .find_map(|e| match e {
            ServerEvent::ConnectRequest { pending, .. } => Some(*pending),
            _ => None,
        })
        .unwrap();
    // The client keeps resending ChallengeResponse while the app decides:
    // only one ConnectRequest event.
    s.run(Duration::from_millis(1000));
    assert_eq!(s.sev.iter().filter(|e| matches!(e, ServerEvent::ConnectRequest { .. })).count(), 1);
    s.net.set_link(server_addr(), addr(2, 5000), LinkConditions { loss: 1.0, ..Default::default() });
    let now = s.now();
    s.server.accept(pending, now).unwrap();
    s.run(Duration::from_millis(300));
    s.net.set_link(server_addr(), addr(2, 5000), LinkConditions::lan());
    s.run_until(Duration::from_secs(2), |s| s.clients[0].is_connected());
    assert_eq!(s.server.connection_count(), 1);
}

/// Every server datagram sent before an address is verified is no larger
/// than the datagram that triggered it; spoofed / forged handshakes create
/// no state.
#[test]
fn handshake_spoofing_and_amplification() {
    let mut s = Sim::new(13, LinkConditions::lan());
    s.server.set_info(Some(vec![7; 500]));
    s.net.set_capture(true);
    let victim = addr(66, 6666);
    let _victim_ep = s.net.endpoint(victim);
    // Spoofed ConnectRequests and InfoQueries "from" the victim.
    for n in 0..5u64 {
        let req = Packet::ConnectRequest { protocol: PROTOCOL_VERSION, build: BUILD_ID, nonce: n }.encode();
        s.net.inject(victim, server_addr(), &req);
        let q = Packet::InfoQuery { protocol: PROTOCOL_VERSION, nonce: n }.encode();
        s.net.inject(victim, server_addr(), &q);
        // Unpadded request: must get no answer at all.
        s.net.inject(victim, server_addr(), &req[..64]);
        // Forged ChallengeResponse (guessed cookie): no state.
        let forged = Packet::ChallengeResponse {
            protocol: PROTOCOL_VERSION,
            build: BUILD_ID,
            nonce: n,
            cookie: 0xdead_beef ^ n,
            proof: 0,
            payload: vec![],
        }
        .encode();
        s.net.inject(victim, server_addr(), &forged);
    }
    s.run(Duration::from_millis(100));
    let traffic = s.net.take_captured();
    let to_victim: Vec<_> = traffic.iter().filter(|(f, t, _)| *f == server_addr() && *t == victim).collect();
    assert_eq!(to_victim.len(), 10, "5 challenges + 5 infos");
    for (_, _, d) in &to_victim {
        match Packet::decode(d).unwrap() {
            Packet::Challenge { .. } => assert!(d.len() <= CONNECT_REQUEST_SIZE),
            Packet::Info { payload, .. } => {
                assert!(d.len() <= INFO_QUERY_SIZE);
                assert!(payload.len() < 500); // clipped
            }
            p => panic!("unexpected reply {p:?}"),
        }
    }
    let total_in: usize = traffic.iter().filter(|(_, t, _)| *t == server_addr()).map(|(_, _, d)| d.len()).sum();
    let total_out: usize = to_victim.iter().map(|(_, _, d)| d.len()).sum();
    // Every reply is smaller than its request; the Info share (≤ the 384 B
    // query) dominates the outgoing bytes.
    assert!(total_out < total_in / 2, "amplification: {total_out} out for {total_in} in");
    assert!(s.sev.is_empty());
    assert_eq!(s.server.endpoint_stats().bad_cookie, 5);
    assert_eq!(s.server.endpoint_stats().malformed, 5);

    // A real client; then an off-path attacker spoofs its address with a
    // wrong session: ignored, the connection is unaffected.
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let conn = s.server_conns()[0];
    for seq in 0..50u16 {
        let h = tt_net::packet::Header { session: 12345, seq, ack: 0, ack_bits: 0 };
        let d = Packet::Disconnect { header: h, reason: "spoof".into() }.encode();
        s.net.inject(addr(2, 5000), server_addr(), &d);
        let d = Packet::Disconnect { header: h, reason: "spoof".into() }.encode();
        s.net.inject(server_addr(), addr(2, 5000), &d);
    }
    s.run(Duration::from_millis(200));
    assert!(s.clients[0].is_connected());
    assert_eq!(s.server.connection_count(), 1);
    assert_eq!(s.server.endpoint_stats().bad_session, 50);
    let now = s.now();
    s.server.send(conn, Lane::Reliable, b"still here", now).unwrap();
    s.run(Duration::from_millis(100));
    assert_eq!(s.client_messages(0, Lane::Reliable), vec![b"still here".to_vec()]);
}

#[test]
fn cookie_replay_and_expiry() {
    let mut s = Sim::new(14, LinkConditions::lan());
    s.net.set_capture(true);
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let response = s
        .net
        .take_captured()
        .into_iter()
        .find(|(_, _, d)| matches!(Packet::decode(d), Ok(Packet::ChallengeResponse { .. })))
        .unwrap();
    // Disconnect, then replay the old ChallengeResponse from the same address.
    let now = s.now();
    s.clients[0].disconnect("bye", now);
    s.run(Duration::from_millis(100));
    assert_eq!(s.server.connection_count(), 0);
    s.net.inject(response.0, response.1, &response.2);
    s.run(Duration::from_millis(100));
    assert_eq!(s.server.connection_count(), 0);
    assert_eq!(s.server.endpoint_stats().replays, 1);
    // Much later the cookie has expired (10–20 s lifetime).
    s.run(Duration::from_secs(31));
    s.net.inject(response.0, response.1, &response.2);
    s.run(Duration::from_millis(100));
    assert_eq!(s.server.connection_count(), 0);
    assert_eq!(s.server.endpoint_stats().bad_cookie, 1);
    assert_eq!(s.sev.iter().filter(|e| matches!(e, ServerEvent::ConnectRequest { .. })).count(), 1);
}

#[test]
fn pending_limits_and_rate() {
    let mut s = Sim::new(15, LinkConditions::lan());
    s.auto_accept = false;
    for p in 0..6 {
        s.add_client(addr(2, 5000 + p), ClientConfig::default());
    }
    for i in 0..6 {
        s.connect(i);
    }
    s.run(Duration::from_millis(500));
    let reqs = s.sev.iter().filter(|e| matches!(e, ServerEvent::ConnectRequest { .. })).count();
    assert_eq!(reqs, 4, "at most 4 pending per IP");
    // Global ConnectRequest rate: a burst of 100 from many IPs gets ~10 challenges.
    let mut s = Sim::new(16, LinkConditions::lan());
    s.net.set_capture(true);
    for n in 0..100u8 {
        let req = Packet::ConnectRequest { protocol: PROTOCOL_VERSION, build: BUILD_ID, nonce: n as u64 }.encode();
        s.net.inject(addr(100 + n % 100, 1), server_addr(), &req);
    }
    s.step(TICK);
    let challenges = s.net.take_captured().iter().filter(|(f, ..)| *f == server_addr()).count();
    assert!((10..=11).contains(&challenges), "{challenges}");
    assert!(s.server.endpoint_stats().rate_limited >= 89);
}

#[test]
fn reconnect_from_same_address_replaces() {
    let mut s = Sim::new(17, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let old = s.server_conns()[0];
    // The client process restarts on the same port (no Disconnect sent).
    s.clients[0] = Client::with_seed(s.net.endpoint(addr(2, 5000)), ClientConfig::default(), 999);
    s.connect(0);
    s.run_until(Duration::from_secs(2), |s| s.clients[0].is_connected());
    assert_eq!(s.server.connection_count(), 1);
    assert_ne!(s.server_conns()[0], old);
    assert!(s.server_disconnects().contains(&(old, DisconnectReason::Replaced)));
}

#[test]
fn overflow_drops_the_connection() {
    let mut s = Sim::new(18, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let conn = s.server_conns()[0];
    s.net.set_conditions(LinkConditions { loss: 1.0, ..Default::default() });
    let now = s.now();
    let mut result = Ok(());
    for _ in 0..100 {
        result = s.server.send(conn, Lane::Reliable, &[0; 1000], now);
        if result.is_err() {
            break;
        }
    }
    assert_eq!(result, Err(SendError::Overflow));
    assert_eq!(s.server.connection_count(), 0);
    assert_eq!(s.server.send(conn, Lane::Reliable, b"x", now), Err(SendError::NotConnected));
    s.step(TICK);
    assert!(s.server_disconnects().contains(&(conn, DisconnectReason::Overflow)));
    assert_eq!(s.server.send(conn, Lane::Unreliable, &[0; 1200], now), Err(SendError::NotConnected));
}

#[test]
fn size_limits() {
    let mut s = Sim::new(19, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let now = s.now();
    let c = &mut s.clients[0];
    assert_eq!(c.send(Lane::Unreliable, &[0; 1190], now), Err(SendError::TooLarge));
    assert_eq!(c.send(Lane::Event, &[0; 1025], now), Err(SendError::TooLarge));
    assert_eq!(c.send(Lane::Reliable, &[0; 32 * 1024 + 1], now), Err(SendError::TooLarge));
    c.send(Lane::Unreliable, &[0; 1150], now).unwrap();
    c.send(Lane::Reliable, &[1; 32 * 1024], now).unwrap();
    s.net.set_capture(true);
    s.run(Duration::from_millis(500));
    let conn = s.server_conns()[0];
    assert_eq!(s.server_messages(conn, Lane::Reliable), vec![vec![1; 32 * 1024]]);
    assert_eq!(s.server_messages(conn, Lane::Unreliable).len(), 1);
    for (_, _, d) in s.net.take_captured() {
        assert!(d.len() <= MAX_PACKET_SIZE);
    }
}

#[test]
fn bandwidth_capped_link() {
    let cond = LinkConditions {
        latency: Duration::from_millis(30),
        bandwidth: Some(16_000),
        queue_limit: Duration::from_millis(200),
        ..Default::default()
    };
    let mut s = Sim::new(20, cond);
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let conn = s.server_conns()[0];
    let mut sent = 0u32;
    let mut got = Vec::new();
    while got.len() < 300 {
        let now = s.now();
        while sent < 300 && s.server.reliable_backlog(conn).unwrap() < 32 * 1024 {
            s.server.send(conn, Lane::Reliable, &payload(sent, 1000), now).unwrap();
            sent += 1;
        }
        s.step(TICK);
        got.extend(s.client_messages(0, Lane::Reliable));
        assert!(s.now() < Duration::from_secs(120));
    }
    assert_eq!(got, (0..300).map(|k| payload(k, 1000)).collect::<Vec<_>>());
    assert!(s.net.link_stats(server_addr(), addr(2, 5000)).queue_dropped > 0);
}

#[test]
fn lan_info_query() {
    let mut s = Sim::new(21, LinkConditions::dsl());
    s.server.set_info(Some(b"Test server|Spider|Brawl|3/16".to_vec()));
    s.add_client(addr(2, 5000), ClientConfig::default());
    let now = s.now();
    s.clients[0].query_info(server_addr(), now);
    s.run(Duration::from_millis(300));
    let info = s.cev[0]
        .iter()
        .find_map(|e| match e {
            ClientEvent::Info { from, payload, rtt } => Some((*from, payload.clone(), *rtt)),
            _ => None,
        })
        .unwrap();
    assert_eq!(info.0, server_addr());
    assert_eq!(info.1, b"Test server|Spider|Brawl|3/16");
    assert!(info.2 >= Duration::from_millis(110) && info.2 <= Duration::from_millis(150), "{:?}", info.2);
}

#[test]
fn simulation_is_deterministic() {
    let run = || {
        let mut s = Sim::new(22, LinkConditions::bad_wifi());
        s.add_client(addr(2, 5000), ClientConfig::default());
        s.add_client(addr(3, 5000), ClientConfig::default());
        s.connect_all();
        for i in 0..300u32 {
            let now = s.now();
            for c in s.server_conns() {
                s.server.send(c, Lane::Reliable, &payload(i, 200), now).unwrap();
                s.server.send(c, Lane::Unreliable, &payload(i, 50), now).unwrap();
            }
            s.clients[1].send(Lane::Event, &i.to_le_bytes(), now).unwrap();
            s.step(Duration::from_millis(32));
        }
        s.run(Duration::from_secs(2));
        let now = s.now();
        let st = s.server.stats(ConnId(0), now).unwrap();
        (format!("{:?}", s.sev), format!("{:?}", s.cev), st)
    };
    assert_eq!(run(), run());
}

#[test]
fn many_clients() {
    let mut s = Sim::new(23, LinkConditions::dsl());
    for i in 0..16 {
        s.add_client(addr(10 + i, 5000), ClientConfig::default());
    }
    s.connect_all();
    assert_eq!(s.server.connection_count(), 16);
    for k in 0..50u32 {
        let now = s.now();
        for c in s.server_conns() {
            s.server.send(c, Lane::Reliable, &payload(k, 100), now).unwrap();
        }
        s.step(Duration::from_millis(32));
    }
    s.run(Duration::from_secs(1));
    for i in 0..16 {
        assert_eq!(s.client_messages(i, Lane::Reliable), (0..50).map(|k| payload(k, 100)).collect::<Vec<_>>());
    }
}

/// §6 flood limit: more than 64 packets/s from one address are dropped
/// unread; other clients are not disturbed.
#[test]
fn per_address_flood_limit() {
    let mut s = Sim::new(30, LinkConditions::lan());
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let flooder = addr(77, 1);
    let junk = Packet::InfoQuery { protocol: PROTOCOL_VERSION, nonce: 1 }.encode();
    for _ in 0..1000 {
        s.net.inject(flooder, server_addr(), &junk);
    }
    s.step(TICK);
    let st = s.server.endpoint_stats().clone();
    assert!(st.flood_dropped >= 900, "{st:?}");
    // A second later the bucket has refilled (≤ 64 more).
    s.run(Duration::from_secs(1));
    for _ in 0..1000 {
        s.net.inject(flooder, server_addr(), &junk);
    }
    s.step(TICK);
    let st2 = s.server.endpoint_stats().clone();
    assert!(st2.flood_dropped - st.flood_dropped >= 930, "{st2:?}");
    let conn = s.server_conns()[0];
    let now = s.now();
    s.server.send(conn, Lane::Reliable, b"fine", now).unwrap();
    s.run(Duration::from_millis(200));
    assert_eq!(s.client_messages(0, Lane::Reliable), vec![b"fine".to_vec()]);
}

/// §6: a connected peer sending more than 100 undecodable packets per
/// second is kicked.
#[test]
fn malformed_flood_kicks() {
    let cfg = ServerConfig { max_packets_per_sec: 1000, ..Default::default() };
    let mut s = Sim::with_config(31, LinkConditions::lan(), cfg);
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.add_client(addr(3, 5000), ClientConfig::default());
    s.connect_all();
    let conns = s.server_conns();
    let bad = conns.iter().copied().find(|c| s.server.addr(*c) == Some(addr(2, 5000))).unwrap();
    // 50 per tick for a few ticks from the first client's address.
    for _ in 0..4 {
        for _ in 0..50 {
            s.net.inject(addr(2, 5000), server_addr(), &[42, 1, 2, 3]);
        }
        s.step(TICK);
    }
    assert!(s.server_disconnects().contains(&(bad, DisconnectReason::Protocol("too many malformed packets"))), "{:?}", s.server_disconnects());
    assert_eq!(s.server.endpoint_stats().malformed_kicks, 1);
    assert_eq!(s.server.connection_count(), 1);
    // A few bad packets per second are only counted.
    let good = s.server_conns()[0];
    for _ in 0..30 {
        s.net.inject(addr(3, 5000), server_addr(), &[42]);
        s.step(Duration::from_millis(100));
    }
    assert_eq!(s.server_conns(), vec![good]);
}

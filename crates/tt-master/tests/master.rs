//! The master server with game servers, browsers and clients on a `MemNet`,
//! servers and clients behind simulated restricted-cone NATs.

use std::net::SocketAddr;
use std::time::Duration;

use tt_master::{Limits, Master};
use tt_net::master::{MasterPacket, ENTRY_LIFETIME, MASTER_VERSION};
use tt_net::*;

fn a(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

const MASTER: &str = "198.51.100.1:28100";
const SERVER: &str = "203.0.113.10:28000";

struct World {
    net: MemNet,
    t: Duration,
    master: Master<MemLink>,
    server: Server<RestrictedNat<MemLink>>,
}

impl World {
    fn new() -> World {
        let net = MemNet::new(3);
        net.set_conditions(LinkConditions::dsl());
        let master = Master::new(net.endpoint(a(MASTER)), Limits::default());
        let cfg = ServerConfig { info: Some(ServerInfo { server_id: 77, build: BUILD_ID, name: "Behind NAT".into(), ..Default::default() }.encode()), ..Default::default() };
        let server = Server::with_seed(RestrictedNat::new(net.endpoint(a(SERVER))), cfg, 1);
        World { net, t: Duration::ZERO, master, server }
    }

    fn step(&mut self, f: &mut dyn FnMut(&mut Server<RestrictedNat<MemLink>>, Duration)) {
        self.t += Duration::from_millis(32);
        self.net.set_now(self.t);
        self.master.update(self.t);
        self.server.update(self.t);
        f(&mut self.server, self.t);
        self.server.flush(self.t);
    }

    fn run(&mut self, ms: u64, f: &mut dyn FnMut(&mut Server<RestrictedNat<MemLink>>, Duration)) {
        for _ in 0..ms / 32 {
            self.step(f);
        }
    }
}

/// Accepts every connect request.
fn accept_all(s: &mut Server<RestrictedNat<MemLink>>, t: Duration) {
    for e in s.drain_events() {
        if let ServerEvent::ConnectRequest { pending, .. } = e {
            s.accept(pending, t);
        }
    }
}

#[test]
fn register_list_punch_and_connect_through_nat() {
    let mut w = World::new();
    w.server.set_master(&[a(MASTER)], 77, w.t);
    w.run(1500, &mut accept_all);
    let st = w.server.master_status().clone();
    assert!(st.registered, "{st:?}");
    assert_eq!(st.public_addr, Some(a(SERVER)));
    assert_eq!(w.master.servers(), vec![a(SERVER)]);
    assert_eq!(w.master.stats.challenges, 1, "one cookie round trip");
    assert_eq!(w.master.drain_log(), ["added 203.0.113.10:28000 (id 000000000000004d)"], "refreshes are not logged");

    // A browser behind its own NAT: the master's list, then the info query
    // gets through thanks to the introduction.
    let mut b = LanBrowser::new(RestrictedNat::new(w.net.endpoint(a("192.0.2.50:40001"))), 9);
    b.loopback = false;
    b.query_master(&[a(MASTER)], w.t);
    for _ in 0..60 {
        w.step(&mut accept_all);
        b.update(w.t);
        if w.t.as_millis().is_multiple_of(320) {
            b.resend_master(w.t);
        }
    }
    assert_eq!(b.master_list(), &[a(SERVER)]);
    assert_eq!(b.servers().len(), 1, "info through the punched hole");
    assert_eq!(b.servers()[0].info.name, "Behind NAT");
    assert!(w.server.master_status().introductions >= 1);

    // Without an introduction the server's NAT drops the client: timeout.
    let mut direct = Client::with_seed(RestrictedNat::new(w.net.endpoint(a("192.0.2.60:40002"))), ClientConfig::default(), 4);
    direct.connect(a(SERVER), b"x", w.t).unwrap();
    let mut ended = None;
    for _ in 0..250 {
        w.step(&mut accept_all);
        direct.update(w.t);
        direct.flush(w.t);
        for e in direct.drain_events() {
            ended = Some(e);
        }
        if ended.is_some() {
            break;
        }
    }
    assert_eq!(ended, Some(ClientEvent::Disconnected(DisconnectReason::ConnectTimeout)));

    // With the master introducing: connected.
    let mut c = Client::with_seed(RestrictedNat::new(w.net.endpoint(a("192.0.2.61:40003"))), ClientConfig::default(), 5);
    c.set_introducer(&[a(MASTER)]);
    c.connect(a(SERVER), b"x", w.t).unwrap();
    let mut connected = false;
    for _ in 0..150 {
        w.step(&mut accept_all);
        c.update(w.t);
        c.flush(w.t);
        connected |= c.drain_events().contains(&ClientEvent::Connected);
        if connected {
            break;
        }
    }
    assert!(connected, "hole punched");
    assert_eq!(w.server.connection_count(), 1);

    // A closing heartbeat takes the server off the list at once.
    w.server.set_master(&[], 77, w.t);
    w.run(500, &mut accept_all);
    assert!(w.master.servers().is_empty());
    assert_eq!(w.master.drain_log(), ["dropped 203.0.113.10:28000 (id 000000000000004d): server closed"]);
}

#[test]
fn entries_expire_and_spoofed_heartbeats_do_not_register() {
    let mut w = World::new();
    w.server.set_master(&[a(MASTER)], 1, w.t);
    w.run(1000, &mut accept_all);
    assert_eq!(w.master.servers().len(), 1);
    // A heartbeat from a forged source: challenged (to the forged address), not listed.
    let hb = MasterPacket::Heartbeat { version: MASTER_VERSION, nonce: 1, cookie: 0, server_id: 2, closing: false }.encode();
    w.net.inject(a("203.0.113.99:28000"), a(MASTER), &hb);
    let forged_cookie = MasterPacket::Heartbeat { version: MASTER_VERSION, nonce: 1, cookie: 12345, server_id: 2, closing: false }.encode();
    w.net.inject(a("203.0.113.98:28000"), a(MASTER), &forged_cookie);
    w.run(500, &mut accept_all);
    assert_eq!(w.master.servers(), vec![a(SERVER)]);
    // Heartbeats keep it listed for a long time.
    w.run(150_000, &mut accept_all);
    assert_eq!(w.master.servers(), vec![a(SERVER)]);
    assert_eq!(w.master.drain_log().len(), 1, "only the first registration");
    // A restarted server: closed, then added again with its new id.
    w.server.set_master(&[], 1, w.t);
    w.server.set_master(&[a(MASTER)], 3, w.t);
    w.run(1000, &mut accept_all);
    assert_eq!(w.master.drain_log(), ["dropped 203.0.113.10:28000 (id 0000000000000001): server closed", "added 203.0.113.10:28000 (id 0000000000000003)"]);
    // A server that stops heartbeating expires.
    let t0 = w.t;
    while w.t < t0 + ENTRY_LIFETIME + Duration::from_secs(2) {
        w.t += Duration::from_millis(100);
        w.net.set_now(w.t);
        w.master.update(w.t);
    }
    assert!(w.master.servers().is_empty());
    assert_eq!(w.master.drain_log(), ["dropped 203.0.113.10:28000 (id 0000000000000003): no heartbeat for 70s"]);
}

/// Several masters: the server registers with each (its own challenge),
/// the browser merges their lists (the server once; a dead master only
/// lists nothing), and leaving one out takes the server off that one only.
#[test]
fn several_masters_register_and_merge() {
    const MASTER2: &str = "198.51.100.2:28100";
    const DEAD: &str = "198.51.100.3:28100";
    let mut w = World::new();
    let mut m2 = Master::new(w.net.endpoint(a(MASTER2)), Limits::default());
    w.server.set_master(&[a(MASTER), a(MASTER2)], 77, w.t);
    let step = |w: &mut World, m2: &mut Master<MemLink>| {
        w.step(&mut accept_all);
        m2.update(w.t);
    };
    for _ in 0..50 {
        step(&mut w, &mut m2);
    }
    assert!(w.server.master_statuses().all(|s| s.registered), "{:?}", w.server.master_statuses().collect::<Vec<_>>());
    assert_eq!((w.master.servers(), m2.servers()), (vec![a(SERVER)], vec![a(SERVER)]));
    assert_eq!((w.master.stats.challenges, m2.stats.challenges), (1, 1));

    let mut b = LanBrowser::new(RestrictedNat::new(w.net.endpoint(a("192.0.2.50:40001"))), 9);
    b.loopback = false;
    b.query_master(&[a(DEAD), a(MASTER), a(MASTER2)], w.t);
    for _ in 0..60 {
        step(&mut w, &mut m2);
        b.update(w.t);
        if w.t.as_millis().is_multiple_of(320) {
            b.resend_master(w.t);
        }
    }
    assert_eq!(b.master_list(), &[a(SERVER)], "listed by two masters, kept once");
    assert_eq!(b.servers().len(), 1);
    assert_eq!(b.servers()[0].info.name, "Behind NAT");

    w.server.set_master(&[a(MASTER2)], 77, w.t);
    for _ in 0..20 {
        step(&mut w, &mut m2);
    }
    assert!(w.master.servers().is_empty());
    assert_eq!(m2.servers(), vec![a(SERVER)]);
    assert_eq!(w.server.master_status().master, Some(a(MASTER2)));
}

#[test]
fn utc_stamps() {
    assert_eq!(tt_master::utc_stamp(0), "1970-01-01 00:00:00 UTC");
    assert_eq!(tt_master::utc_stamp(1_792_491_303), "2026-10-20 10:15:03 UTC");
    assert_eq!(tt_master::utc_stamp(951_782_400), "2000-02-29 00:00:00 UTC");
}

#[test]
fn lists_are_paged_and_limits_hold() {
    let net = MemNet::new(8);
    let mut m = Master::new(net.endpoint(a(MASTER)), Limits { servers_per_ip: 2, ..Default::default() });
    let mut t = Duration::ZERO;
    // 150 servers on 75 IPs (two each), plus a third on one IP (refused).
    let mut eps: Vec<MemLink> = (0..150).map(|i| net.endpoint(a(&format!("10.1.{}.{}:{}", i / 2 / 250, (i / 2) % 250 + 1, 28000 + i % 2)))).collect();
    eps.push(net.endpoint(a("10.1.0.1:28009")));
    let mut buf = [0u8; 1500];
    for round in 0..3 {
        for (i, e) in eps.iter_mut().enumerate() {
            // Answer the challenge with its cookie.
            let mut cookie = 0;
            while let Some((n, _)) = e.recv_from(&mut buf) {
                if let Ok(MasterPacket::HeartbeatChallenge { cookie: c, .. }) = MasterPacket::decode(&buf[..n]) {
                    cookie = c;
                }
            }
            if round == 0 || cookie != 0 {
                let p = MasterPacket::Heartbeat { version: MASTER_VERSION, nonce: i as u64, cookie, server_id: i as u64, closing: false };
                e.send_to(&p.encode(), a(MASTER)).unwrap();
            }
        }
        t += Duration::from_millis(100);
        net.set_now(t);
        m.update(t);
    }
    assert_eq!(m.servers().len(), 150, "the third server on one IP is refused");
    let mut b = LanBrowser::new(net.endpoint(a("192.0.2.9:5000")), 1);
    b.query_master(&[a(MASTER)], t);
    for _ in 0..10 {
        t += Duration::from_millis(50);
        net.set_now(t);
        m.update(t);
        b.update(t);
    }
    assert_eq!(b.master_list().len(), 150, "three pages");
    // Flooding from one IP is cut by the token bucket.
    let before = m.stats.list_requests;
    let req = MasterPacket::ListRequest { version: MASTER_VERSION, nonce: 5, page: 0 }.encode();
    for _ in 0..500 {
        net.inject(a("192.0.2.77:1"), a(MASTER), &req);
    }
    t += Duration::from_millis(50);
    net.set_now(t);
    m.update(t);
    assert!(m.stats.list_requests - before <= 41, "{}", m.stats.list_requests - before);
    assert!(m.stats.dropped >= 400);
}

#[test]
fn args() {
    let o = tt_master::parse_args(&["--port".into(), "29000".into(), "--status".into(), "0".into()]).unwrap();
    assert_eq!((o.port, o.status_every), (29000, Duration::ZERO));
    assert_eq!(tt_master::parse_args(&[]).unwrap().port, 28100);
    assert!(tt_master::parse_args(&["--bogus".into()]).is_err());
    assert_eq!(tt_master::parse_args(&["--local-address".into(), "203.0.113.5".into()]).unwrap().local_address, Some("203.0.113.5".parse().unwrap()));
    assert!(tt_master::parse_args(&["--local-address".into(), "nope".into()]).is_err());
}

/// `--local-address`: a server on the master's host (Docker bridge,
/// loopback) is listed and acked under the master's public IP, heartbeats
/// keep refreshing it, and introductions still go to its real source.
#[test]
fn local_address_lists_same_host_servers_under_the_public_ip() {
    const LOCAL: &str = "127.0.0.1:28000";
    const LISTED: &str = "203.0.113.5:28000";
    let net = MemNet::new(5);
    let mut m = Master::new(net.endpoint(a(MASTER)), Limits::default());
    m.local_address = Some("203.0.113.5".parse().unwrap());
    let mut s = Server::with_seed(net.endpoint(a(LOCAL)), ServerConfig::default(), 2);
    let mut t = Duration::ZERO;
    s.set_master(&[a(MASTER)], 9, t);
    let step = |m: &mut Master<MemLink>, s: &mut Server<MemLink>, t: &mut Duration| {
        *t += Duration::from_millis(32);
        net.set_now(*t);
        m.update(*t);
        s.update(*t);
        s.flush(*t);
    };
    for _ in 0..40 {
        step(&mut m, &mut s, &mut t);
    }
    assert!(s.master_status().registered);
    assert_eq!(s.master_status().public_addr, Some(a(LISTED)), "the ack names the listed address");
    assert_eq!(m.servers(), vec![a(LISTED)]);
    assert_eq!(m.drain_log(), ["added 203.0.113.5:28000 (id 0000000000000009)"]);
    for _ in 0..100_000 / 32 {
        step(&mut m, &mut s, &mut t);
    }
    assert_eq!(m.servers(), vec![a(LISTED)], "heartbeats refresh it");
    assert!(m.drain_log().is_empty());

    // An introduction to the listed address reaches the server, which punches the client.
    let mut c = net.endpoint(a("192.0.2.70:40000"));
    c.send_to(&MasterPacket::IntroduceRequest { version: MASTER_VERSION, nonce: 3, server: a(LISTED) }.encode(), a(MASTER)).unwrap();
    for _ in 0..20 {
        step(&mut m, &mut s, &mut t);
    }
    assert_eq!((m.stats.introductions, s.master_status().introductions), (1, 1));
    let mut buf = [0u8; 1500];
    let mut punched = false;
    while let Some((n, from)) = c.recv_from(&mut buf) {
        punched |= from == a(LOCAL) && matches!(MasterPacket::decode(&buf[..n]), Ok(MasterPacket::Punch { .. }));
    }
    assert!(punched, "punched from the real address");

    s.set_master(&[], 9, t);
    for _ in 0..20 {
        step(&mut m, &mut s, &mut t);
    }
    assert_eq!(m.drain_log(), ["dropped 203.0.113.5:28000 (id 0000000000000009): server closed"]);
}

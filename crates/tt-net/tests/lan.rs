//! LAN discovery (docs/netcode.md §5.6): a [`LanBrowser`] finds servers by
//! broadcast on a `MemNet` and over real UDP on loopback (where broadcast
//! may be unavailable, the 127.0.0.1 query must still find the server).

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tt_net::*;

fn info(name: &str, id: u64) -> ServerInfo {
    ServerInfo {
        server_id: id,
        build: BUILD_ID,
        game_type: tt_net::GAME_TYPE.into(),
        name: name.into(),
        mission_type: "Battlemode".into(),
        info: "Linux\t1\tSpider Valley\t0".into(),
        mission_file: "missions/tt1_1.ron".into(),
        players: "\tBait".into(),
        player_count: 3,
        max_players: 10,
        bot_count: 2,
        password: false,
        dedicated: true,
        cpu_mhz: 2000,
        scores: vec![7],
        ..Default::default()
    }
}

fn a(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

#[test]
fn broadcast_finds_servers_on_memnet() {
    let net = MemNet::new(5);
    net.set_conditions(LinkConditions { latency: Duration::from_millis(3), ..Default::default() });
    // Two servers on the LAN (ports 28000 and 28001), one not accepting
    // (no info: no reply), one on another port range.
    let mut servers: Vec<Server<MemLink>> = [("10.0.0.1:28000", Some("Alpha")), ("10.0.0.2:28001", Some("Beta")), ("10.0.0.3:28000", None), ("10.0.0.4:29000", Some("Other"))]
        .iter()
        .enumerate()
        .map(|(i, (addr, name))| {
            let mut s = Server::new(net.endpoint(a(addr)), ServerConfig::default());
            s.set_info(name.map(|n| info(n, i as u64 + 1).encode()));
            s
        })
        .collect();
    let mut b = LanBrowser::new(net.endpoint(a("10.0.0.9:40000")), 1);
    b.loopback = false;
    b.query_lan(28000, net.now());
    for _ in 0..20 {
        let now = net.advance(Duration::from_millis(1));
        for s in &mut servers {
            s.update(now);
        }
        b.update(now);
    }
    let mut found: Vec<(String, SocketAddr)> = b.servers().iter().map(|s| (s.info.name.clone(), s.addr)).collect();
    found.sort();
    assert_eq!(found, vec![("Alpha".into(), a("10.0.0.1:28000")), ("Beta".into(), a("10.0.0.2:28001"))]);
    let alpha = b.servers().iter().find(|s| s.info.name == "Alpha").unwrap();
    assert!(alpha.compatible());
    assert_eq!(alpha.info.map(), "Spider Valley");
    assert_eq!((alpha.info.player_count, alpha.info.max_players, alpha.info.bot_count), (3, 10, 2));
    assert!(alpha.ping >= Duration::from_millis(6) && alpha.ping <= Duration::from_millis(8), "{:?}", alpha.ping);
    // A second round updates instead of duplicating; a unicast query
    // reaches a server outside the broadcast range.
    b.query_lan(28000, net.now());
    b.query(&[a("10.0.0.4:29000")], net.now());
    for _ in 0..20 {
        let now = net.advance(Duration::from_millis(1));
        for s in &mut servers {
            s.update(now);
        }
        b.update(now);
    }
    assert_eq!(b.servers().len(), 3);
    // Replies to a cancelled query are ignored.
    b.clear();
    b.query_lan(28000, net.now());
    b.cancel();
    for _ in 0..20 {
        let now = net.advance(Duration::from_millis(1));
        for s in &mut servers {
            s.update(now);
        }
        b.update(now);
    }
    assert!(b.servers().is_empty());
}

#[test]
fn replies_over_two_routes_are_merged() {
    // The same server answers from its LAN address and from loopback.
    let net = MemNet::new(6);
    let lan = a("10.0.0.1:28000");
    let lo = a("127.0.0.1:28000");
    let mut s1 = Server::new(net.endpoint(lan), ServerConfig::default());
    let mut s2 = Server::new(net.endpoint(lo), ServerConfig::default());
    s1.set_info(Some(info("Same", 77).encode()));
    s2.set_info(Some(info("Same", 77).encode()));
    let mut b = LanBrowser::new(net.endpoint(a("10.0.0.9:40000")), 2);
    b.query(&[lo, lan], net.now());
    for _ in 0..5 {
        let now = net.advance(Duration::from_millis(1));
        s1.update(now);
        s2.update(now);
        b.update(now);
    }
    assert_eq!(b.servers().len(), 1);
    assert_eq!(b.servers()[0].addr, lan);
}

/// Real sockets: a server on an ephemeral port of the wildcard address and a
/// browser on its own socket. The LAN query (broadcast + loopback over the
/// port range starting at the server's port) must find it, whether or not
/// this environment lets broadcasts through.
#[test]
fn udp_loopback_discovery() {
    let t = UdpTransport::bind_any(0).unwrap();
    let port = t.local_addr().port();
    let mut server = Server::new(t, ServerConfig::default());
    server.set_info(Some(info("Loopback", 9).encode()));
    let mut b = LanBrowser::udp().unwrap();
    let start = Instant::now();
    b.query_lan(port, start.elapsed());
    let mut resent = false;
    while b.servers().is_empty() && start.elapsed() < Duration::from_secs(5) {
        let now = start.elapsed();
        server.update(now);
        b.update(now);
        if !resent && now > Duration::from_millis(500) {
            b.query_lan(port, now);
            resent = true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let s = b.servers().first().expect("server found over UDP");
    assert_eq!(s.info.name, "Loopback");
    assert_eq!(s.addr.port(), port);
    assert!(s.ping < Duration::from_secs(1));
    // Unicast to 127.0.0.1 directly (the "Join by address" preview).
    b.clear();
    let now = start.elapsed();
    b.query(&[SocketAddr::from(([127, 0, 0, 1], port))], now);
    let t0 = Instant::now();
    while b.servers().is_empty() && t0.elapsed() < Duration::from_secs(5) {
        let now = start.elapsed();
        server.update(now);
        b.update(now);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(b.servers().len(), 1);
    assert!(b.servers()[0].addr.ip().is_loopback());
}

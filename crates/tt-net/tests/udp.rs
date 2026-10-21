//! Real UDP over loopback (127.0.0.1, ephemeral ports), on real time.

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tt_net::*;

const MESSAGES: u32 = 300;

fn msg(k: u32) -> Vec<u8> {
    let mut v = k.to_le_bytes().to_vec();
    v.resize(4 + (k as usize * 37) % 2000, k as u8);
    v
}

/// Runs a server that echoes every reliable message back, until its client
/// disconnects or `limit` passes.
fn bind_server() -> Server<UdpTransport> {
    let t = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    // The test loops every millisecond (far more packets than a game tick
    // rate): no per-address flood limit here.
    Server::new(t, ServerConfig { max_packets_per_sec: 1_000_000, ..ServerConfig::default() })
}

fn echo_server(mut server: Server<UdpTransport>, limit: Duration) -> Result<(), String> {
    let start = Instant::now();
    let mut served = false;
    while start.elapsed() < limit {
        let now = start.elapsed();
        server.update(now);
        for e in server.drain_events() {
            match e {
                ServerEvent::ConnectRequest { pending, .. } => {
                    server.accept(pending, now);
                }
                ServerEvent::Message { conn, lane: Lane::Reliable, data } => {
                    server.send(conn, Lane::Reliable, &data, now).map_err(|e| e.to_string())?;
                }
                ServerEvent::Disconnected { reason, .. } => {
                    if reason != DisconnectReason::Remote("done".into()) {
                        return Err(format!("unexpected disconnect {reason:?}"));
                    }
                    served = true;
                }
                _ => {}
            }
        }
        server.flush(now);
        if served {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err("timed out".into())
}

/// Connects, sends [`MESSAGES`] reliable messages, checks the echoes arrive
/// once and in order, then disconnects.
fn echo_client(server_addr: SocketAddr) {
    let t = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let mut client = Client::new(t, ClientConfig::default());
    let start = Instant::now();
    client.connect(server_addr, b"loopback", start.elapsed()).unwrap();
    let mut sent = 0;
    let mut echoed = Vec::new();
    while echoed.len() < MESSAGES as usize {
        assert!(start.elapsed() < Duration::from_secs(20), "loopback stalled at {}", echoed.len());
        let now = start.elapsed();
        client.update(now);
        for e in client.drain_events() {
            match e {
                ClientEvent::Message { lane: Lane::Reliable, data } => echoed.push(data),
                ClientEvent::Disconnected(r) => panic!("disconnected: {r:?}"),
                _ => {}
            }
        }
        while client.is_connected() && sent < MESSAGES && client.reliable_backlog() < 32 * 1024 {
            client.send(Lane::Reliable, &msg(sent), now).unwrap();
            sent += 1;
        }
        client.flush(now);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(echoed, (0..MESSAGES).map(msg).collect::<Vec<_>>());
    let st = client.stats(start.elapsed()).unwrap();
    assert!(st.rtt < Duration::from_millis(200), "{st:?}");
    client.disconnect("done", start.elapsed());
}

#[test]
fn loopback_same_process() {
    let server = bind_server();
    let addr = server.local_addr();
    let srv = std::thread::spawn(move || echo_server(server, Duration::from_secs(30)));
    echo_client(addr);
    srv.join().unwrap().unwrap();
}

/// Child-process entry point for [`loopback_two_processes`]; does nothing
/// unless spawned by it.
#[test]
fn child_echo_server() {
    if std::env::var_os("TT_NET_CHILD").is_none() {
        return;
    }
    let server = bind_server();
    println!("PORT={}", server.local_addr().port());
    if let Err(e) = echo_server(server, Duration::from_secs(30)) {
        panic!("child server: {e}");
    }
}

/// Two processes exchange reliable messages over loopback UDP.
#[test]
fn loopback_two_processes() {
    let bin = std::env::current_exe().unwrap();
    let mut child = Command::new(bin)
        .args(["--exact", "child_echo_server", "--nocapture", "--test-threads=1"])
        .env("TT_NET_CHILD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut port = None;
    let mut lines = BufReader::new(stdout).lines();
    for line in lines.by_ref() {
        let line = line.unwrap();
        // The harness may print "test child_echo_server ... " on the same line.
        if let Some(i) = line.find("PORT=") {
            let digits: String = line[i + 5..].chars().take_while(|c| c.is_ascii_digit()).collect();
            port = Some(digits.parse::<u16>().unwrap());
            break;
        }
    }
    let port = port.expect("child printed its port");
    // Keep draining the child's stdout so it never hits a broken pipe.
    let drain = std::thread::spawn(move || lines.for_each(drop));
    echo_client(SocketAddr::from(([127, 0, 0, 1], port)));
    let status = child.wait().unwrap();
    drain.join().unwrap();
    assert!(status.success(), "child server failed: {status}");
}

#[test]
fn bind_any_dual_stack_smoke() {
    // Some sandboxes have no IPv6; bind_any then falls back to 0.0.0.0.
    let mut a = UdpTransport::bind_any(0).unwrap();
    let mut b = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = a.local_addr().port();
    b.send_to(b"ping", SocketAddr::from(([127, 0, 0, 1], port))).unwrap();
    let start = Instant::now();
    let mut buf = [0u8; 64];
    let from = loop {
        if let Some((n, from)) = a.recv_from(&mut buf) {
            assert_eq!(&buf[..n], b"ping");
            break from;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    };
    // IPv4-mapped peers are reported as IPv4, and replies reach them.
    assert_eq!(from, b.local_addr());
    a.send_to(b"pong", from).unwrap();
    let start = Instant::now();
    loop {
        if let Some((n, _)) = b.recv_from(&mut buf) {
            assert_eq!(&buf[..n], b"pong");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
}

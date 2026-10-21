//! Network play, stage 2: a dedicated server and a client engine complete the
//! handshake, load the same mission through the script phases, reach the game, chat,
//! and disconnect. In-process over `MemNet` (virtual time, lossy link) and over real
//! UDP against the `opentt --dedicated` executable.

use std::net::SocketAddr;
use std::time::Duration;
use tt_engine::Engine;
use tt_net::{LinkConditions, MemNet};

mod common;
use common::*;

const MISSION: &str = "missions/tt1_1.ron";

/// The chat lines on the client's HUD.
fn chat_log(c: &Engine) -> String {
    c.host.client.messages.lines.iter().filter(|l| l.chat).map(|l| l.text.as_str()).collect::<Vec<_>>().join("|")
}

/// The client says `text`.
fn say(c: &mut Engine, text: &str) {
    tt_engine::net::send_cmd(&mut c.host, tt_gameplay::ClientCmd::MessageSent { text: text.into() });
}

/// The server's client of a connection.
fn client_of(s: &Engine, conn: tt_engine::net::Conn) -> Option<tt_gameplay::server::ClientId> {
    tt_engine::gameplay::server_host::client_of_conn(&s.host, conn)
}

fn connect(e: &mut Engine, address: &str, password: &str, name: &str) {
    tt_engine::net::connect_to(&mut e.host, address, password, name, "");
}

fn disconnect(e: &mut Engine) {
    tt_engine::gameplay::client::connection::disconnect(&mut e.host);
}

/// Drives both engines through join → play → chat → disconnect. `step`
/// advances the clocks by one tick and ticks both.
fn scenario(server: &mut Engine, client: &mut Engine, mut step: impl FnMut(&mut Engine, &mut Engine), connect_to: &str) {
    let mut run = |s: &mut Engine, c: &mut Engine, ticks: u32, until: &mut dyn FnMut(&mut Engine, &mut Engine) -> bool, what: &str| {
        for _ in 0..ticks {
            if until(s, c) {
                return;
            }
            step(s, c);
        }
        dump_log(s, "server");
        dump_log(c, "client");
        panic!("not reached: {what}");
    };
    connect(client, connect_to, "", "Tester");
    // Handshake, then the three phases: the server marks the client phase 3,
    // the client has the mission loaded and its control object.
    run(server, client, 600, &mut |s, _| remote_clients(s).len() == 1, "server accepts the connection");
    run(
        server,
        client,
        1500,
        &mut |s, c| {
            let cl = remote_clients(s);
            !cl.is_empty() && client_of(s, cl[0]).is_some_and(|c| tt_engine::gameplay::server_host::client_phase(&s.host, c) == Some(tt_gameplay::server::DownloadPhase::InGame)) && log_has(c, "*** Initial Control Object")
        },
        "both reach the play phase",
    );
    for line in ["*** Phase 1: Download Definitions", "*** Phase 2: Download Mission Objects", "*** Phase 3: Mission Lighting", "Mission lighting done"] {
        assert!(log_has(client, line), "client: {line}");
    }
    // The same mission, as a static world on the client (role Client).
    run(server, client, 50, &mut |_, c| c.host.game.is_some(), "client builds the mission world");
    let (sg, cg) = (server.host.game.as_ref().unwrap(), client.host.game.as_ref().unwrap());
    assert_eq!(cg.role, tt_engine::game::Role::Client);
    assert_eq!(sg.role, tt_engine::game::Role::Dedicated);
    assert!(!cg.world.server);
    assert_eq!(cg.world.statics.len(), sg.world.statics.len());
    assert_eq!(cg.world.triggers.len(), sg.world.triggers.len());
    assert_eq!(client.host.cl.mission_file, MISSION);
    // The player list came over with translated tags (MsgClientJoin).
    assert!(log_has(server, "Connection established"));
    let cl = remote_clients(server)[0];
    assert!(tt_engine::netgame::address(&server.host, cl).unwrap().starts_with("IP:"));
    assert_eq!(tt_engine::gameplay::server_host::record(&server.host, client_of(server, cl).unwrap()).unwrap().name(), "Tester");
    say(client, "hello from the client");
    run(server, client, 200, &mut |_, c| chat_log(c).contains("hello from the client"), "chat round trip");
    assert!(chat_log(client).contains("Tester: hello"), "{}", chat_log(client));
    // Clean disconnect: the server runs onDrop and forgets the connection.
    disconnect(client);
    run(server, client, 200, &mut |s, _| remote_clients(s).is_empty(), "server drops the client");
    assert!(log_has(server, "dropped: disconnect"));
    assert!(client.host.net.server_conn.is_none());
    assert!(!tt_engine::netgame::is_remote_client(&client.host));
}

#[test]
fn dedicated_and_client_over_memnet() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(7);
        // A lossy internet-like link: 40 ms ±10, 5 % loss, some reordering.
        net.set_conditions(LinkConditions { latency: Duration::from_millis(40), jitter: Duration::from_millis(10), loss: 0.05, reorder: 0.02, reorder_delay: Duration::from_millis(20), ..Default::default() });
        let mut server = engine(&root, "mem-server", &["-dedicated", "-mission", MISSION]);
        server.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 1]));
        assert!(server.boot());
        assert_eq!(tt_engine::netgame::server_addr(&server.host), Some(SocketAddr::from(([10, 0, 0, 1], 28000))));
        let mut client = client_engine(&root, "mem-client");
        client.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 2]));
        let mut t = 0u64;
        let step = |s: &mut Engine, c: &mut Engine| {
            t += 32;
            net.set_now(Duration::from_millis(t));
            for e in [&mut *s, &mut *c] {
                e.host.virtual_clock_ms = Some(t as f64);
                e.tick();
            }
        };
        scenario(&mut server, &mut client, step, "10.0.0.1:28000");
    });
}

#[test]
fn rejections_and_timeouts() {
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(3);
        let mut t = 0u64;
        let mut client = client_engine(&root, "mem-errors");
        client.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 2]));
        // Nobody there: the handshake times out (5 s).
        connect(&mut client, "10.0.0.9:28000", "", "");
        for _ in 0..200 {
            step_all(&net, &mut t, &mut [&mut client]);
        }
        assert_eq!(conn_err(&client), "Your connection to the server timed out.");
        assert!(client.host.net.server_conn.is_none());

        let mut server = engine(&root, "mem-errors-server", &["-dedicated", "-mission", MISSION]);
        server.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 1]));
        assert!(server.boot());
        let attempt = |server: &mut Engine, client: &mut Engine, t: &mut u64, password: &str| {
            client.host.shell.message = None;
            client.host.cl.password = password.to_string();
            connect(client, "10.0.0.1", password, "Tester");
            for _ in 0..60 {
                step_all(&net, t, &mut [&mut *server, &mut *client]);
            }
            conn_err(client)
        };
        // Full (onConnectRequest's reason goes back to the client).
        server.set_var("server.max_players", "0");
        assert_eq!(attempt(&mut server, &mut client, &mut t, ""), "This server is full.");
        server.set_var("server.max_players", "16");
        // Password (only its digest travels).
        server.set_var("server.password", "secret");
        assert_eq!(attempt(&mut server, &mut client, &mut t, "wrong"), "That password is incorrect.");
        assert_eq!(attempt(&mut server, &mut client, &mut t, "secret"), "");
        assert_eq!(remote_clients(&server).len(), 1);
        // The link dies: both sides time out (15 s while loading at most).
        net.set_conditions(LinkConditions { loss: 1.0, ..Default::default() });
        for _ in 0..(16_000 / 32) {
            step_all(&net, &mut t, &mut [&mut server, &mut client]);
        }
        assert!(remote_clients(&server).is_empty());
        assert!(log_has(&server, "dropped: timeout"));
        assert_eq!(conn_err(&client), "The server connection has timed out.");
        assert!(client.host.net.server_conn.is_none());
    });
}

/// Real UDP on loopback: the `opentt --dedicated` executable as the
/// server process, this process as the client.
#[test]
fn dedicated_executable_and_client_over_udp() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        let data = std::env::temp_dir().join(format!("tt-netplay-udp-server-{}", std::process::id()));
        let mut child = Command::new(game_exe())
            .args(["--assets", root.to_str().unwrap(), "--data", data.to_str().unwrap(), "--dedicated", "--mission", MISSION, "--port", "0", "--no-sound", "--quiet"])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut port = None;
        for line in lines.by_ref() {
            let line = line.unwrap();
            if let Some(a) = line.strip_prefix("dedicated server listening on ") {
                port = a.trim().parse::<SocketAddr>().ok().map(|a| a.port());
                break;
            }
        }
        let drain = std::thread::spawn(move || lines.for_each(drop));
        let port = port.expect("server address on stdout");
        // A server stand-in for `scenario`'s server-side checks would need
        // the other process; here the client side is checked, and the
        // server's view through what it sends back (chat, drop).
        let mut client = client_engine(&root, "udp-client");
        connect(&mut client, &format!("127.0.0.1:{port}"), "", "Tester");
        let start = std::time::Instant::now();
        let mut tick = |c: &mut Engine| {
            c.host.virtual_clock_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
            c.tick();
            std::thread::sleep(Duration::from_millis(4));
        };
        let wait = |c: &mut Engine, tick: &mut dyn FnMut(&mut Engine), what: &str, f: &dyn Fn(&mut Engine) -> bool| {
            while !f(c) {
                if start.elapsed() > Duration::from_secs(60) {
                    dump_log(c, "client");
                    panic!("not reached: {what}");
                }
                tick(c);
            }
        };
        wait(&mut client, &mut tick, "play phase", &|c| log_has(c, "*** Initial Control Object") && log_has(c, "Mission lighting done"));
        wait(&mut client, &mut tick, "mission world", &|c| c.host.game.is_some());
        assert!(!client.host.game.as_ref().unwrap().world.statics.is_empty());
        // Replication (stages 3–4): the server's bots appear as puppets and
        // move; OpenTT's own tank follows our moves (driving forward).
        wait(&mut client, &mut tick, "replicated tanks", &|c| c.host.netgame.client_rep().is_some_and(|r| r.tanks.len() >= 2) && tt_engine::netgame::control_object(&c.host).is_some());
        let positions = |c: &Engine| -> Vec<(u32, glam::Vec3)> {
            let rep = c.host.netgame.client_rep().unwrap();
            let g = c.host.game.as_ref().unwrap();
            rep.tanks.iter().filter_map(|(n, (_, t))| g.world.tank(*t).map(|t| (*n, t.pos))).collect()
        };
        let before = positions(&client);
        let own = client.host.netgame.client_rep().unwrap().control_net;
        client.host.client.moves.set_drive(1.0, 0.0);
        let t0 = start.elapsed();
        while start.elapsed() < t0 + Duration::from_secs(3) {
            tick(&mut client);
        }
        let after = positions(&client);
        let moved = |n: u32| before.iter().zip(&after).any(|((a, p), (b, q))| *a == n && *b == n && (*p - *q).length() > 2.0);
        assert!(moved(own), "own tank drives: {before:?} -> {after:?}");
        assert!(before.iter().any(|(n, _)| *n != own && moved(*n)), "a bot moves");
        client.host.client.moves.set_drive(0.0, 0.0);
        say(&mut client, "over udp");
        wait(&mut client, &mut tick, "chat round trip", &|c| chat_log(c).contains("over udp"));
        let conn = client.host.net.server_conn.unwrap();
        let (ping, _) = tt_engine::netgame::ping_loss(&mut client.host, conn).unwrap();
        assert!(ping < 1000.0, "ping {ping}");
        disconnect(&mut client);
        for _ in 0..10 {
            tick(&mut client);
        }
        assert!(client.host.net.server_conn.is_none());
        let _ = child.kill();
        let _ = child.wait();
        drain.join().unwrap();
        let _ = std::fs::remove_dir_all(&data);
    });
}

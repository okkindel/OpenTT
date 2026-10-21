
use std::path::Path;
use std::time::{Duration, Instant};
use tt_engine::console::execute;
use tt_engine::Engine;
use tt_net::{LinkConditions, MemNet};

mod common;
use common::*;

const MISSION: &str = "missions/tt1_1.ron";

fn query_done(e: &Engine) -> bool {
    tt_engine::netgame::query::status(&e.host) == tt_engine::netgame::query::QueryStatus::Done
}

fn mem_server(root: &Path, net: &MemNet, tag: &str) -> Engine {
    let mut server = engine(root, tag, &["-dedicated", "-mission", MISSION]);
    server.host.netgame.factory = Some(mem_factory(net, [10, 0, 0, 1]));
    assert!(server.boot());
    server
}

/// Runs the query the way joinGameGui does and waits for "done".
fn query(net: &MemNet, t: &mut u64, server: &mut Engine, client: &mut Engine) {
    tt_engine::netgame::query::query_lan(&mut client.host, 28000, true);
    assert_eq!(tt_engine::netgame::query::status(&client.host), tt_engine::netgame::query::QueryStatus::Querying);
    for _ in 0..200 {
        if query_done(client) {
            return;
        }
        step_all(net, t, &mut [&mut *server, &mut *client]);
    }
    panic!("query not done: {}", client.host.netgame.query.message);
}

#[test]
fn lan_discovery_over_memnet() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(11);
        net.set_conditions(LinkConditions { latency: Duration::from_millis(20), ..Default::default() });
        let mut server = mem_server(&root, &net, "mem-server");
        let mut client = client_engine(&root, "mem-client");
        client.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 2]));
        let mut t = 0u64;
        // Let the server start its round.
        for _ in 0..30 {
            step_all(&net, &mut t, &mut [&mut server]);
        }
        query(&net, &mut t, &mut server, &mut client);
        assert_eq!(client.host.netgame.query.message, "Found 1 server");
        // What the menu lists.
        let rows = tt_engine::netgame::query::rows(&client.host);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.address, "IP:10.0.0.1:28000");
        assert_eq!(r.name, server.var("server.name"));
        assert_eq!(r.mode, server.server().session.mode_label);
        assert!(!r.mode.is_empty());
        assert_eq!(r.max_players.to_string(), server.var("server.max_players"));
        assert!(!r.password && r.dedicated && r.compatible);
        assert!((40..=80).contains(&r.ping_ms), "ping {}", r.ping_ms);
        let address = r.address.clone();
        let bots = r.bots;
        assert_eq!(r.players, bots, "only bots so far");
        assert_eq!(r.player_list.len() as u32, r.players);
        assert!(!r.map.is_empty() && r.mission_file.ends_with("tt1_1.ron"), "{r:?}");
        assert!(r.score > 0.0);
        // Join the found server by its address (joinGameGui::onJoin), then
        // the next query counts us.
        tt_engine::net::connect_to(&mut client.host, &address, "", "Tester", "");
        for _ in 0..600 {
            if remote_clients(&server).len() == 1 && log_has(&client, "*** Initial Control Object") {
                break;
            }
            step_all(&net, &mut t, &mut [&mut server, &mut client]);
        }
        if remote_clients(&server).len() != 1 {
            dump_log(&server, "server");
            dump_log(&client, "client");
            panic!("joined the found server");
        }
        for _ in 0..10 {
            step_all(&net, &mut t, &mut [&mut server, &mut client]);
        }
        query(&net, &mut t, &mut server, &mut client);
        let r = &tt_engine::netgame::query::rows(&client.host)[0];
        assert_eq!(r.players, r.bots + 1, "{r:?}");
        assert!(r.player_list.iter().any(|(_, n)| n == "Tester"), "{:?}", r.player_list);
        // A password shows as the lock flag; a closed server disappears.
        server.set_var("server.password", "secret");
        query(&net, &mut t, &mut server, &mut client);
        assert!(tt_engine::netgame::query::rows(&client.host)[0].password);
        tt_engine::netgame::allow_connections(&mut server.host, false);
        query(&net, &mut t, &mut server, &mut client);
        assert!(tt_engine::netgame::query::rows(&client.host).is_empty());
        // The master server is gone: queryMasterServer completes at once.
        let masters = tt_engine::netgame::master_addrs(&client.host.shell.prefs);
        if !masters.is_empty() {
            tt_engine::netgame::query::query_master(&mut client.host, &masters, false);
        }
        for _ in 0..(tt_engine::netgame::query::MASTER_QUERY_TIME.as_millis() as u32 / 32 + 2) {
            step_all(&net, &mut t, &mut [&mut server, &mut client]);
        }
        assert!(query_done(&client));
    });
}

/// Real UDP: an in-process dedicated server on an ephemeral port and a client
/// whose query goes to broadcast and to 127.0.0.1 (broadcast may be blocked
/// here; loopback must find it either way).
#[test]
fn lan_discovery_over_udp_loopback() {
    let root = assets();
    big_stack(move || {
        let mut server = engine(&root, "udp-server", &["-dedicated", "-mission", MISSION]);
        server.host.netgame.port_override = Some(0);
        assert!(server.boot());
        let port = tt_engine::netgame::server_addr(&server.host).expect("listening").port();
        let mut client = client_engine(&root, "udp-client");
        let start = Instant::now();
        let tick = |e: &mut Engine| {
            e.host.virtual_clock_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
            e.tick();
        };
        tt_engine::netgame::query::query_lan(&mut client.host, port, true);
        while tt_engine::netgame::query::status(&client.host) != tt_engine::netgame::query::QueryStatus::Done {
            assert!(start.elapsed() < Duration::from_secs(20), "query timed out");
            tick(&mut server);
            tick(&mut client);
            std::thread::sleep(Duration::from_millis(2));
        }
        // The query covers a port range from `port`: a server of another test
        // running at the same time may answer too (seen on Windows). Ours:
        let rows = tt_engine::netgame::query::rows(&client.host);
        let ours: Vec<_> = rows.iter().filter(|r| r.address.ends_with(&format!(":{port}"))).collect();
        assert_eq!(ours.len(), 1, "{rows:?}");
        assert!(ours[0].compatible);
        assert!(query_done(&client));
        // Unicast preview of a typed address.
        assert!(tt_engine::netgame::query::query_address(&mut client.host, &format!("127.0.0.1:{port}")));
        assert!(!tt_engine::netgame::query::query_address(&mut client.host, "no such host name.invalid"));
    });
}

#[test]
fn console_commands_on_dedicated_server() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(12);
        let mut server = mem_server(&root, &net, "con-server");
        let mut client = client_engine(&root, "con-client");
        client.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 2]));
        let mut t = 0u64;
        tt_engine::net::connect_to(&mut client.host, "10.0.0.1:28000", "", "Tester", "");
        for _ in 0..800 {
            if remote_clients(&server).len() == 1 && log_has(&client, "*** Initial Control Object") {
                break;
            }
            step_all(&net, &mut t, &mut [&mut server, &mut client]);
        }
        assert_eq!(remote_clients(&server).len(), 1);
        let id = tt_engine::gameplay::server_host::client_of_conn(&server.host, remote_clients(&server)[0]).unwrap().0;

        let help = execute(&mut server, "help");
        assert!(help[0].contains("kick"));
        let st = execute(&mut server, "status").join("\n");
        assert!(st.contains("Tester") && st.contains(&id.to_string()) && st.contains("IP:10.0.0.2:"), "{st}");
        assert!(st.contains("tt1_1.ron"), "{st}");

        // say -> a server message to every client.
        let before = server.host.net.sent.get("ServerMessage").copied().unwrap_or(0);
        assert!(execute(&mut server, "say hello everybody").is_empty());
        assert_eq!(server.host.net.sent.get("ServerMessage").copied().unwrap_or(0), before + 1);
        assert!(log_has(&server, "Server: hello everybody"));

        // The typed settings commands (and the assignment as an alias).
        assert!(execute(&mut server, "echo console works").is_empty());
        assert!(log_has(&server, "console works"));
        execute(&mut server, "server.max_players = 12");
        assert_eq!(server.var("server.max_players"), "12");
        assert_eq!(execute(&mut server, "set server.name \"Console Test\""), vec!["server.name = Console Test".to_string()]);
        assert_eq!(server.host.shell.prefs.server_name(), "Console Test");
        assert_eq!(execute(&mut server, "get server.max_players"), vec!["server.max_players = 12".to_string()]);
        assert!(execute(&mut server, "get server.*").len() > 5);
        assert!(execute(&mut server, "getServerCount")[0].starts_with("unknown command"));
        assert!(execute(&mut server, "session.status = 1;")[0].contains("not a setting"));

        // map: another mission (by name); unknown names are refused.
        assert_eq!(execute(&mut server, "map nosuchmission"), vec!["no mission \"nosuchmission\"".to_string()]);
        assert_eq!(execute(&mut server, "map tt1_2"), vec!["loading missions/tt1_2.ron".to_string()]);
        assert_eq!(server.server().session.mission_file.clone(), "missions/tt1_2.ron");
        for _ in 0..900 {
            if client.host.cl.mission_file.ends_with("tt1_2.ron") {
                break;
            }
            step_all(&net, &mut t, &mut [&mut server, &mut client]);
        }
        if !client.host.cl.mission_file.ends_with("tt1_2.ron") {
            dump_log(&server, "server");
            dump_log(&client, "client");
        }
        assert!(client.host.cl.mission_file.ends_with("tt1_2.ron"), "client follows: {}", client.host.cl.mission_file);

        // kick by (a prefix of the) name: the client is dropped.
        assert_eq!(execute(&mut server, "kick nobody"), vec!["no player \"nobody\" (status lists them)".to_string()]);
        assert_eq!(execute(&mut server, "kick test"), vec![format!("kicked Tester ({id})")]);
        for _ in 0..100 {
            step_all(&net, &mut t, &mut [&mut server, &mut client]);
        }
        assert!(remote_clients(&server).is_empty());
        assert!(client.host.net.server_conn.is_none(), "client dropped ({:?})", client.host.shell.message.as_ref().map(|m| &m.text));

        assert_eq!(execute(&mut server, "quit"), vec!["shutting down".to_string()]);
        assert!(server.host.quit_requested);
    });
}

/// `opentt --dedicated` on a free port with its data in `data`, its
/// stdin piped and its stdout lines on a channel; returns once it
/// listens.
struct Dedicated {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    rx: std::sync::mpsc::Receiver<String>,
    reader: std::thread::JoinHandle<()>,
}

impl Dedicated {
    fn start(data: &Path) -> Dedicated {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        let root = assets();
        let mut child = Command::new(game_exe())
            .args(["--assets", root.to_str().unwrap(), "--data", data.to_str().unwrap(), "--dedicated", "--mission", MISSION, "--port", "0", "--no-sound", "--quiet"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for l in lines.map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        let d = Dedicated { child, stdin, rx, reader };
        d.wait_for(&|l| l.starts_with("dedicated server listening on "), "listening line");
        d
    }

    fn wait_for(&self, pred: &dyn Fn(&str) -> bool, what: &str) -> String {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(60) {
            if let Ok(l) = self.rx.recv_timeout(Duration::from_millis(100)) {
                if pred(&l) {
                    return l;
                }
            }
        }
        panic!("no {what} on stdout");
    }

    /// Waits for the exit (30 s at most) and asserts it was a clean one.
    fn exits(mut self) {
        let start = Instant::now();
        let status = loop {
            if let Some(s) = self.child.try_wait().unwrap() {
                break s;
            }
            if start.elapsed() > Duration::from_secs(30) {
                let _ = self.child.kill();
                panic!("server did not quit");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(status.success(), "{status}");
        drop(self.stdin);
        self.reader.join().unwrap();
    }
}

/// The executable: commands typed on stdin while it serves.
#[test]
fn dedicated_executable_reads_console_from_stdin() {
    use std::io::Write;
    let data = std::env::temp_dir().join(format!("tt-lan-stdin-{}", std::process::id()));
    let mut d = Dedicated::start(&data);
    writeln!(d.stdin, "status").unwrap();
    let l = d.wait_for(&|l| l.starts_with("mission "), "status output");
    assert!(l.contains("tt1_1.ron"), "{l}");
    writeln!(d.stdin, "echo from stdin").unwrap();
    d.wait_for(&|l| l == "from stdin", "echo output (--quiet prints the command's log lines)");
    writeln!(d.stdin, "quit").unwrap();
    d.exits();
    let _ = std::fs::remove_dir_all(&data);
}

/// SIGTERM (`docker stop`) shuts the executable down like `quit`: the
/// settings changed in the console are saved.
#[cfg(unix)]
#[test]
fn dedicated_executable_saves_on_sigterm() {
    use std::io::Write;
    let data = std::env::temp_dir().join(format!("tt-lan-sigterm-{}", std::process::id()));
    let mut d = Dedicated::start(&data);
    writeln!(d.stdin, "set server.name SignalSaved").unwrap();
    writeln!(d.stdin, "get server.name").unwrap();
    d.wait_for(&|l| l.contains("SignalSaved"), "the new name");
    let kill = std::process::Command::new("kill").args(["-TERM", &d.child.id().to_string()]).status().unwrap();
    assert!(kill.success());
    d.exits();
    let settings = std::fs::read_to_string(data.join("settings.ron")).unwrap();
    assert!(settings.contains("SignalSaved"), "{settings}");
    let _ = std::fs::remove_dir_all(&data);
}

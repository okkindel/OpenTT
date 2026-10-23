//! The Online menus driven through the menu actions: an engine hosts a listen server
//! (the host plays on it), a second engine finds it in the LAN list over `MemNet` and
//! joins; the password prompt; the error dialogs; leaving.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tt_engine::menu::Mode;
use tt_engine::{Config, Engine};
use tt_menu::{BotSetup, BotSkill, CustomGame, HostGame, MenuAction, MenuBackend, Screen, Team};
use tt_net::MemNet;

mod common;
use common::*;

fn engine(root: &Path, net: &MemNet, tag: &str, ip: [u8; 4], player: &str) -> Engine {
    let data = std::env::temp_dir().join(format!("tt-menu-online-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let cfg = Config { assets: root.to_path_buf(), data, args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    e.host.virtual_clock_ms = Some(0.0);
    e.host.netgame.factory = Some(mem_factory(net, ip));
    assert!(e.boot());
    new_player(&mut e, player);
    e
}

struct World {
    net: MemNet,
    t: u64,
}

impl World {
    fn step(&mut self, engines: &mut [&mut Engine]) {
        self.t += 32;
        self.net.set_now(Duration::from_millis(self.t));
        for e in engines.iter_mut() {
            e.host.virtual_clock_ms = Some(self.t as f64);
            e.tick();
            // What the frame loop does: the menu follows the game flow.
            let Engine { host, menu, .. } = &mut **e;
            menu.as_mut().unwrap().sync(host);
        }
    }

    fn run_until(&mut self, engines: &mut [&mut Engine], ticks: u32, what: &str, f: impl Fn(&[&mut Engine]) -> bool) {
        for _ in 0..ticks {
            if f(engines) {
                return;
            }
            self.step(engines);
        }
        for e in engines.iter() {
            for (_, m) in e.host.log.history.iter().rev().take(25).collect::<Vec<_>>().into_iter().rev() {
                eprintln!("log: {m}");
            }
        }
        panic!("not reached: {what}");
    }
}

fn in_game(e: &Engine) -> bool {
    e.menu.as_ref().unwrap().data.mode == Mode::InGame
}

fn screen(e: &Engine) -> Screen {
    e.menu.as_ref().unwrap().state.screen()
}

fn message(e: &mut Engine) -> Option<String> {
    backend_do(e, |b| b.script_message().map(|m| m.text))
}

fn humans(e: &Engine) -> usize {
    e.host.net.server.iter().count() + tt_engine::netgame::remote_conns(&e.host).len()
}

fn host_game(name: &str, password: &str, max_players: u32) -> HostGame {
    HostGame {
        game: CustomGame {
            mission: "missions/tt1_1.ron".into(),
            mode: 0,
            bots: vec![BotSetup { name: "Hum".into(), skill: BotSkill::Bronze, tank: 0, team: Team::Auto }],
            player_tank: 1,
            player_team: Team::Auto,
            minutes: 15,
            replace_bots: true,
        },
        name: name.into(),
        max_players,
        password: password.into(),
        port: 28000,
        port_mapping: false,
        public: false,
        rotation: vec!["missions/tt1_1.ron".into(), "missions/tt2_3.ron".into()],
        rotation_random: true,
    }
}

/// Host → LAN list → join → play → the player list's ping → leave; the
/// host settings and the recent address are remembered.
#[test]
fn host_and_join_through_the_menus() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(5);
        let mut w = World { net: net.clone(), t: 0 };
        let mut host = engine(&root, &net, "host", [10, 0, 0, 1], "Hoster");
        let mut client = engine(&root, &net, "client", [10, 0, 0, 2], "Joiner");
        apply(&mut host, MenuAction::Host(host_game("Hoster's arena", "", 16)));
        assert_eq!(screen(&host), Screen::Loading);
        w.run_until(&mut [&mut host], 600, "the host plays", |e| in_game(e[0]));
        assert_eq!(tt_engine::netgame::server_addr(&host.host), Some(SocketAddr::from(([10, 0, 0, 1], 28000))));
        // The Host screen's rotation is the server's (kept in the settings).
        assert_eq!(host.server().setup.rotation, vec!["missions/tt1_1.ron".to_string(), "missions/tt2_3.ron".to_string()]);
        assert_eq!(host.host.shell.prefs.map_rotation(), vec!["tt1_1".to_string(), "tt2_3".to_string()]);
        assert!(host.server().setup.rotation_random, "the random order reaches the server");
        assert!(host.host.shell.prefs.map_rotation_random());
        // The client opens Online/Join: the LAN query finds the server.
        backend_do(&mut client, |b| {
            b.set_server_source(tt_menu::ServerSource::Lan);
            b.refresh_servers()
        });
        w.run_until(&mut [&mut host, &mut client], 200, "LAN list", |e| !tt_engine::netgame::query::rows(&e[1].host).is_empty());
        let servers = backend_do(&mut client, |b| b.lan_servers());
        assert_eq!(servers.len(), 1, "{servers:?}");
        let s = &servers[0];
        assert_eq!((s.name.as_str(), s.map.as_str(), s.password, s.compatible, s.dedicated), ("Hoster's arena", "Medulla Mesa", false, true, false));
        assert!(s.players >= 2 && s.max_players == 16 && s.bots >= 1, "{s:?}");
        apply(&mut client, MenuAction::Join { address: s.address.clone(), password: String::new(), team: Team::Auto });
        assert_eq!(screen(&client), Screen::Loading);
        assert!(backend_do(&mut client, |b| b.loading()).is_some_and(|l| l.phase.contains("Connecting to 10.0.0.1:28000")));
        w.run_until(&mut [&mut host, &mut client], 900, "the client plays", |e| in_game(e[1]));
        assert_eq!(humans(&host), 2);
        // The ping column (2 s ping list) on both sides.
        for _ in 0..80 {
            w.step(&mut [&mut host, &mut client]);
        }
        let remote = tt_engine::netgame::remote_conns(&host.host)[0];
        let remote = tt_engine::gameplay::server_host::client_of_conn(&host.host, remote).unwrap().0;
        for e in [&mut host, &mut client] {
            assert!(e.host.client.players.pings.get(&(remote as i32)).is_some_and(|p| !p.is_empty()), "ping in the player list");
        }
        // Leave the game from the pause menu: back to Online, the host goes on alone.
        apply(&mut client, MenuAction::LeaveGame);
        w.run_until(&mut [&mut host, &mut client], 200, "client left", |e| humans(e[0]) == 1 && !in_game(e[1]));
        assert_eq!(screen(&client), Screen::Online);
        assert!(in_game(&host));
        // Remembered: the host settings and the address joined.
        let hs = backend_do(&mut host, |b| b.host_settings()).unwrap();
        assert_eq!((hs.name.as_str(), hs.max_players, hs.port, hs.game.bots.len()), ("Hoster's arena", 16, 28000, 1));
        assert_eq!(hs.rotation, host_game("", "", 16).rotation);
        assert!(hs.rotation_random, "the random order is remembered");
        assert_eq!(backend_do(&mut client, |b| b.recent_addresses()).first().map(String::as_str), Some("10.0.0.1:28000"));
        // The host leaves: the server closes.
        apply(&mut host, MenuAction::LeaveGame);
        w.run_until(&mut [&mut host, &mut client], 100, "host left", |e| !in_game(e[0]));
        assert_eq!(screen(&host), Screen::Online);
    });
}

/// Password prompt and the error dialogs (wrong password, full, banned, no answer).
#[test]
fn password_and_error_dialogs() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(6);
        let mut w = World { net: net.clone(), t: 0 };
        let mut host = engine(&root, &net, "pwhost", [10, 0, 0, 1], "Hoster");
        let mut client = engine(&root, &net, "pwclient", [10, 0, 0, 2], "Joiner");
        apply(&mut host, MenuAction::Host(host_game("Locked", "secret", 16)));
        w.run_until(&mut [&mut host], 600, "host plays", |e| in_game(e[0]));
        backend_do(&mut client, |b| {
            b.set_server_source(tt_menu::ServerSource::Lan);
            b.refresh_servers()
        });
        w.run_until(&mut [&mut host, &mut client], 200, "LAN list", |e| !tt_engine::netgame::query::rows(&e[1].host).is_empty());
        let s = backend_do(&mut client, |b| b.lan_servers()).remove(0);
        assert!(s.password, "the lock icon");
        // Joining by address without a password: refused, the menu asks for one.
        let addr = "10.0.0.1".to_string();
        let attempt = |w: &mut World, host: &mut Engine, client: &mut Engine, pw: &str| -> Option<String> {
            apply(client, MenuAction::Join { address: addr.clone(), password: pw.into(), team: Team::Auto });
            for _ in 0..200 {
                w.step(&mut [host, client]);
                if in_game(client) {
                    return None;
                }
                if let Some(m) = message(client) {
                    backend_do(client, |b| b.answer_script_message(true));
                    for _ in 0..5 {
                        w.step(&mut [host, client]);
                    }
                    return Some(m);
                }
            }
            panic!("no answer");
        };
        assert_eq!(attempt(&mut w, &mut host, &mut client, "").as_deref(), Some("That server requires a password."));
        assert_eq!(backend_do(&mut client, |b| b.take_password_request()).as_deref(), Some("IP:10.0.0.1:28000"));
        assert_eq!(screen(&client), Screen::Online, "back on the join screen after the dialog");
        assert_eq!(attempt(&mut w, &mut host, &mut client, "guess").as_deref(), Some("That password is incorrect."));
        assert_eq!(attempt(&mut w, &mut host, &mut client, "secret"), None);
        w.run_until(&mut [&mut host, &mut client], 900, "client plays", |e| in_game(e[1]));
        apply(&mut client, MenuAction::LeaveGame);
        w.run_until(&mut [&mut host, &mut client], 200, "left", |e| humans(e[0]) == 1);
        // Banned.
        tt_engine::netgame::admin::add(&mut host.host, "0", "IP:10.0.0.2:40000", 600);
        tt_engine::netgame::admin::persist(&mut host.host);
        // Saved at once (not only on a clean exit); the ban times have defaults.
        let saved = String::from_utf8(host.host.fs.read(tt_engine::netgame::admin::BAN_FILE).unwrap_or_default()).unwrap();
        assert!(saved.contains("guid: \"0\"") && saved.contains("ip: \"10.0.0.2\""), "{saved}");
        assert_eq!((host.var("server.kick_ban_time").as_str(), host.var("server.ban_time").as_str()), ("300", "1800"));
        assert_eq!(attempt(&mut w, &mut host, &mut client, "secret").as_deref(), Some("You are not allowed to play on this server."));
        host.host.netgame.bans.clear();
        // Full (the host itself is one of the players).
        host.set_var("server.max_players", "1");
        assert_eq!(attempt(&mut w, &mut host, &mut client, "secret").as_deref(), Some("This server is full."));
        // Nobody there.
        apply(&mut client, MenuAction::Join { address: "10.0.0.77".into(), password: String::new(), team: Team::Auto });
        let mut text = None;
        for _ in 0..300 {
            w.step(&mut [&mut host, &mut client]);
            if let Some(m) = message(&mut client) {
                text = Some(m);
                break;
            }
        }
        assert_eq!(text.as_deref(), Some("Your connection to the server timed out."));
    });
}

fn remote_client(host: &Engine) -> Option<u32> {
    tt_engine::netgame::remote_conns(&host.host).first().copied()
}

/// Joins `address` and answers the first dialog; `None` when the game started.
fn join_and_answer(w: &mut World, host: &mut Engine, client: &mut Engine, address: &str, team: Team) -> Option<String> {
    apply(client, MenuAction::Join { address: address.into(), password: String::new(), team });
    for _ in 0..400 {
        w.step(&mut [host, client]);
        if in_game(client) {
            return None;
        }
        if let Some(m) = message(client) {
            backend_do(client, |b| b.answer_script_message(true));
            for _ in 0..5 {
                w.step(&mut [host, client]);
            }
            return Some(m);
        }
    }
    panic!("no answer joining {address}");
}

/// A team game: the server details list the players with teams and scores,
/// the team asked for when joining is the one the server gives; the
/// wrong-version and "game data differs" errors reach the menu.
#[test]
fn team_choice_player_list_and_version_errors() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(7);
        let mut w = World { net: net.clone(), t: 0 };
        let mut host = engine(&root, &net, "teamhost", [10, 0, 0, 1], "Hoster");
        let mut client = engine(&root, &net, "teamclient", [10, 0, 0, 2], "Joiner");
        let mut g = host_game("Teams", "", 16);
        g.game.mode = 1;
        g.game.bots.push(BotSetup { name: "Tom".into(), skill: BotSkill::Silver, tank: 1, team: Team::Green });
        apply(&mut host, MenuAction::Host(g));
        w.run_until(&mut [&mut host], 600, "host plays", |e| in_game(e[0]));
        // Some score on the host so the list shows one.
        let hc = tt_engine::gameplay::server_host::local_client(&host.host).unwrap();
        tt_engine::gameplay::server_host::with_record(&mut host.host, hc, |r| r.score = Some(7.0));
        tt_engine::gameplay::server_host::with_server(&mut host.host, |s, b| s.refresh_player_list(b));
        backend_do(&mut client, |b| {
            b.set_server_source(tt_menu::ServerSource::Lan);
            b.refresh_servers()
        });
        w.run_until(&mut [&mut host, &mut client], 200, "LAN list", |e| !tt_engine::netgame::query::rows(&e[1].host).is_empty());
        let s = backend_do(&mut client, |b| b.lan_servers()).remove(0);
        assert!(s.team_game(), "{}", s.mode);
        assert!(s.player_list.len() >= 3, "{:?}", s.player_list);
        let me = s.player_list.iter().find(|p| p.name == "Hoster").unwrap_or_else(|| panic!("{:?}", s.player_list));
        assert_eq!(me.score, 7);
        assert!(s.player_list.iter().all(|p| p.team != Team::Auto), "everyone is on a team: {:?}", s.player_list);
        // Join on Blue: the server puts us there.
        assert_eq!(join_and_answer(&mut w, &mut host, &mut client, &s.address, Team::Blue), None);
        w.run_until(&mut [&mut host, &mut client], 900, "client plays", |e| in_game(e[1]));
        let rc = remote_client(&host).unwrap();
        assert_eq!(team_of(&host, rc), "BlueTeam");
        apply(&mut client, MenuAction::LeaveGame);
        w.run_until(&mut [&mut host, &mut client], 200, "left", |e| humans(e[0]) == 1);
        // And on Green.
        assert_eq!(join_and_answer(&mut w, &mut host, &mut client, &s.address, Team::Green), None);
        w.run_until(&mut [&mut host, &mut client], 900, "client plays", |e| in_game(e[1]));
        let rc = remote_client(&host).unwrap();
        assert_eq!(team_of(&host, rc), "GreenTeam");
        apply(&mut client, MenuAction::LeaveGame);
        w.run_until(&mut [&mut host, &mut client], 200, "left", |e| humans(e[0]) == 1);
        // Another build on the server: the list says so, joining is refused.
        // (The info reply reports the server's; the open socket keeps its handshake
        // build, so the joining client runs the other build instead.)
        host.host.netgame.build_override = Some(0x1234);
        client.host.netgame.build_override = Some(0x1234);
        backend_do(&mut client, |b| {
            b.set_server_source(tt_menu::ServerSource::Lan);
            b.refresh_servers()
        });
        w.run_until(&mut [&mut host, &mut client], 200, "LAN list", |e| tt_engine::netgame::query::rows(&e[1].host).first().is_some_and(|r| !r.compatible));
        assert!(!backend_do(&mut client, |b| b.lan_servers())[0].compatible);
        assert_eq!(join_and_answer(&mut w, &mut host, &mut client, "10.0.0.1", Team::Auto).as_deref(), Some("Incompatible protocol version: Your game version is not compatible with this server."));
        assert_eq!(screen(&client), Screen::Online);
        host.host.netgame.build_override = None;
        client.host.netgame.build_override = None;
        // The server's defs differ (a changed tank): the client gives up.
        assert!(host.host.defs.set_field("MediumTank", "max_speed", "99"));
        tt_engine::gameplay::defs::refresh(&mut host.host);
        let m = join_and_answer(&mut w, &mut host, &mut client, "10.0.0.1", Team::Auto).unwrap();
        assert!(m.contains("Game data differs from the server's"), "{m}");
        assert_eq!(screen(&client), Screen::Online);
        w.run_until(&mut [&mut host, &mut client], 200, "dropped on the host", |e| humans(e[0]) == 1);
    });
}

/// The Internet list through a master server, and `simLag` /
/// `setSimulatedNetParams` on the real connection.
#[test]
fn internet_list_through_the_master_and_simulated_lag() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(9);
        let mut w = World { net: net.clone(), t: 0 };
        let mut master = tt_master::Master::new(net.endpoint(SocketAddr::from(([10, 0, 0, 50], 28100))), Default::default());
        let mut host = engine(&root, &net, "mshost", [10, 0, 0, 1], "Hoster");
        let mut client = engine(&root, &net, "msclient", [10, 0, 0, 2], "Joiner");
        backend_do(&mut host, |b| b.set_master_server("10.0.0.50"));
        let mut g = host_game("Listed", "", 16);
        g.public = true;
        apply(&mut host, MenuAction::Host(g));
        let mut step = |w: &mut World, host: &mut Engine, client: &mut Engine| {
            w.step(&mut [host, client]);
            master.update(Duration::from_millis(w.t));
        };
        for _ in 0..700 {
            step(&mut w, &mut host, &mut client);
            if in_game(&host) && tt_engine::netgame::master_status(&host.host).is_some_and(|s| s.registered) {
                break;
            }
        }
        let st = tt_engine::netgame::master_status(&host.host).unwrap();
        assert!(st.registered, "{st:?}");
        assert_eq!(st.public_addr, Some(SocketAddr::from(([10, 0, 0, 1], 28000))));
        // The client's Internet source.
        backend_do(&mut client, |b| {
            b.set_master_server("10.0.0.50:28100");
            b.set_server_source(tt_menu::ServerSource::Internet);
            b.refresh_servers();
        });
        for _ in 0..120 {
            step(&mut w, &mut host, &mut client);
        }
        let list = backend_do(&mut client, |b| b.lan_servers());
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].name, "Listed");
        assert_eq!(client.var("net.master_server"), "10.0.0.50:28100");
        // Join from the internet list and play.
        apply(&mut client, MenuAction::Join { address: list[0].address.clone(), password: String::new(), team: Team::Auto });
        for _ in 0..900 {
            step(&mut w, &mut host, &mut client);
            if in_game(&client) {
                break;
            }
        }
        assert!(in_game(&client));
        // simLag: the round trip grows by the simulated delay, then recovers
        // (a developer feature: debug builds and --features net-sim).
        if tt_engine::netgame::sim::AVAILABLE {
            let sc = client.host.net.server_conn.unwrap();
            let rtt = |c: &mut Engine| tt_engine::netgame::ping_loss(&mut c.host, sc).unwrap().0;
            for _ in 0..60 {
                step(&mut w, &mut host, &mut client);
            }
            let base = rtt(&mut client);
            tt_engine::netgame::sim::set_params(&mut client.host, sc, 0.0, 200.0);
            for _ in 0..200 {
                step(&mut w, &mut host, &mut client);
            }
            let lagged = rtt(&mut client);
            assert!(lagged > base + 150.0, "rtt {base:.0} -> {lagged:.0} ms");
            // Loss: about half the client's packets are dropped.
            tt_engine::netgame::sim::set_params(&mut client.host, sc, 0.5, 0.0);
            for _ in 0..200 {
                step(&mut w, &mut host, &mut client);
            }
            assert!(client.host.netgame.sim.client.as_ref().unwrap().dropped() > 50);
            tt_engine::netgame::sim::set_params(&mut client.host, sc, 0.0, 0.0);
            for _ in 0..300 {
                step(&mut w, &mut host, &mut client);
            }
            let after = rtt(&mut client);
            assert!(after < base + 60.0, "rtt back to {after:.0} ms (base {base:.0})");
            assert!(in_game(&client), "the connection survived");
        } else {
            // Release builds without --features net-sim: the call only logs.
            let sc = client.host.net.server_conn.unwrap();
            tt_engine::netgame::sim::set_params(&mut client.host, sc, 0.5, 200.0);
            assert!(!client.host.netgame.sim.client.as_ref().unwrap().active());
        }
        // The host closes: off the master's list.
        apply(&mut host, MenuAction::LeaveGame);
        for _ in 0..100 {
            step(&mut w, &mut host, &mut client);
        }
        assert!(master.servers().is_empty(), "{:?}", master.servers());
    });
}

/// A connection's client's team (the server's record).
fn team_of(e: &Engine, conn: tt_engine::net::Conn) -> &'static str {
    let c = tt_engine::gameplay::server_host::client_of_conn(&e.host, conn).unwrap();
    tt_engine::gameplay::server_host::team_of(&e.host, c).map_or("", |t| t.name())
}

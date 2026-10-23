//! Map rotation and voting on a headless dedicated server with two remote
//! clients over `MemNet`: the console's rotation commands, F10's map list,
//! a vote called from the chat (`/vote`), the other player's F12, the
//! round ending and the voted map starting, then the rotation going on.

use std::time::Duration;
use tt_engine::console::execute;
use tt_engine::Engine;
use tt_net::{LinkConditions, MemNet};

mod common;
use common::*;

const MISSION: &str = "missions/tt1_1.ron";

fn step(net: &MemNet, t: &mut u64, server: &mut Engine, clients: &mut [Engine]) {
    let mut all: Vec<&mut Engine> = vec![server];
    all.extend(clients.iter_mut());
    step_all(net, t, &mut all);
}

fn run_until(net: &MemNet, t: &mut u64, server: &mut Engine, clients: &mut [Engine], max: usize, what: &str, done: impl Fn(&Engine, &[Engine]) -> bool) {
    for _ in 0..max {
        if done(server, clients) {
            return;
        }
        step(net, t, server, clients);
    }
    dump_log(server, "server");
    panic!("{what}: not reached");
}

/// A bound action's key press (F10, F12 by default: the test's game data
/// may be an import of an older version, whose bindings predate them).
fn press(e: &mut Engine, a: tt_gameplay::client::BindAction) {
    tt_engine::gameplay::client::game::bind_action(&mut e.host, a, "1");
    tt_engine::gameplay::client::game::bind_action(&mut e.host, a, "0");
}

#[test]
fn rotation_and_vote_over_the_network() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(17);
        net.set_conditions(LinkConditions { latency: Duration::from_millis(20), ..Default::default() });
        let mut server = engine(&root, "vote-server", &["-dedicated", "-mission", MISSION]);
        server.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 1]));
        assert!(server.boot());
        server.host.srv.state_mut().setup.vote_wait_ms = 2_000;
        // The console: the rotation is the mission alone, then three maps.
        let out = execute(&mut server, "rotation");
        assert_eq!(out[0], "rotation (1 map):");
        assert!(out[1].contains("tt1_1") && out[1].ends_with("<- now"), "{out:?}");
        execute(&mut server, "rotation set tt1_1 tt1_3");
        execute(&mut server, "rotation add tt1_2");
        let out = execute(&mut server, "rotation add tt1_4");
        assert_eq!(out[0], "rotation (4 maps):");
        let out = execute(&mut server, "rotation remove tt1_4");
        assert_eq!(out.len(), 4, "{out:?}");
        assert_eq!(execute(&mut server, "rotation add tt9_9"), vec!["no mission \"tt9_9\"".to_string()]);
        assert_eq!(execute(&mut server, "rotation remove tt1_1 tt1_3 tt1_2"), vec!["the rotation needs a map".to_string()]);
        assert_eq!(server.server().setup.rotation, ["tt1_1", "tt1_3", "tt1_2"].map(tt_assets::layout::mission).to_vec());
        assert_eq!(server.var("server.map_rotation"), "tt1_1\ttt1_3\ttt1_2");
        let mut clients: Vec<Engine> = (0..2)
            .map(|i| {
                let mut c = client_engine(&root, &format!("vote-client{i}"));
                c.host.netgame.factory = Some(mem_factory(&net, [10, 0, 1, i as u8 + 1]));
                tt_engine::net::connect_to(&mut c.host, "10.0.0.1:28000", "", &format!("Player{i}"), "");
                c
            })
            .collect();
        let mut t = 0u64;
        run_until(&net, &mut t, &mut server, &mut clients, 3000, "both clients in play", |_, c| c.iter().all(|c| tt_engine::netgame::control_object(&c.host).is_some()));
        for _ in 0..(2_500 / 32) {
            step(&net, &mut t, &mut server, &mut clients);
        }
        // F10: the rotation arrives, the vote menu opens.
        press(&mut clients[0], tt_gameplay::client::BindAction::VoteMenu);
        run_until(&net, &mut t, &mut server, &mut clients, 100, "map list", |_, c| c[0].host.client.vote_maps.len() == 3);
        assert_eq!(clients[0].host.client.vote_maps[1], ("tt1_3".to_string(), tt_engine::mission::mission_info_name(&mut server.host, "missions/tt1_3.ron")));
        let menu = |c: &Engine| c.menu.as_ref().unwrap().pause.as_ref().map(|p| p.screen());
        {
            // A frame of the menu (the window's frontend runs it).
            let Engine { host, menu, .. } = &mut clients[0];
            menu.as_mut().unwrap().sync(host);
        }
        assert_eq!(menu(&clients[0]), Some(tt_menu::Screen::VoteMap));
        apply(&mut clients[0], tt_menu::MenuAction::Resume);
        assert_eq!(menu(&clients[0]), None);
        // `/vote` in the chat: the vote starts, both see it (bots never count).
        tt_engine::gameplay::client::hud::send_chat(&mut clients[0].host, "/vote tt1_2");
        run_until(&net, &mut t, &mut server, &mut clients, 100, "vote prompt", |_, c| c.iter().all(|c| c.host.client.vote.as_ref().is_some_and(|v| v.players == 2 && v.yes == 1)));
        assert_eq!(clients[0].host.client.vote.as_ref().unwrap().mine, Some(true));
        assert_eq!(clients[1].host.client.vote.as_ref().unwrap().mine, None);
        // A second call is refused while it runs.
        tt_engine::net::send_cmd(&mut clients[1].host, tt_gameplay::ClientCmd::CallVote { map: "tt1_3".into() });
        // F12 on the other client: 2 of 2, passed.
        press(&mut clients[1], tt_gameplay::client::BindAction::VoteYes);
        run_until(&net, &mut t, &mut server, &mut clients, 100, "vote over", |_, c| c.iter().all(|c| c.host.client.vote.is_none()));
        let said = |c: &Engine, s: &str| c.host.client.messages.lines.iter().any(|l| l.text.contains(s));
        run_until(&net, &mut t, &mut server, &mut clients, 50, "the announcements", |_, c| said(&c[0], "Vote passed: switching to") && said(&c[1], "Vote passed: switching to"));
        assert!(said(&clients[1], "Player0 called a vote: switch to"));
        assert!(said(&clients[1], "A vote is already running."));
        assert!(!said(&clients[0], "A vote is already running."), "the refusal goes to its caller only");
        // The round ends (results), the voted map loads on the clients.
        run_until(&net, &mut t, &mut server, &mut clients, 1500, "the voted map", |s, c| s.server().session.mission_file == "missions/tt1_2.ron" && c.iter().all(|c| c.host.cl.mission_file.ends_with("tt1_2.ron")));
        // The rotation goes on after it: `next` ends the round, the first map comes.
        run_until(&net, &mut t, &mut server, &mut clients, 1500, "the round on the voted map", |s, _| s.server().round.running());
        let out = execute(&mut server, "next");
        assert_eq!(out, vec!["round over, next missions/tt1_1.ron".to_string()]);
        run_until(&net, &mut t, &mut server, &mut clients, 1500, "the next map", |s, _| s.server().session.mission_file == MISSION);
    });
}

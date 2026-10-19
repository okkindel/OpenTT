//! Offscreen in-game HUD screenshots (solo, team, scrum, a network
//! client, the chat input, crowded name tags) for visual review. Not a
//! check: run on demand with
//! `TT_HUD_SHOTS=out/hud-native/after cargo test --release -p tt-engine --test hud_shots -- --ignored --test-threads=1`
//! (files `<scenario>_<W>x<H>.png` in that directory).

use std::path::PathBuf;
use std::time::Duration;
use glam::Vec3;
use tt_engine::Engine;
use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, Team};
use tt_net::MemNet;

mod common;
use common::*;

fn out_dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("TT_HUD_SHOTS")?);
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

/// The local player says `text` (to the team).
fn chat(e: &mut Engine, text: &str, team: bool) {
    let text = text.to_string();
    let cmd = if team { tt_gameplay::ClientCmd::TeamMessageSent { text } } else { tt_gameplay::ClientCmd::MessageSent { text } };
    tt_engine::net::send_cmd(&mut e.host, cmd);
}

fn ticks(e: &mut Engine, ms: u32) {
    for _ in 0..ms / tt_engine::TICK_MS {
        tick(e);
    }
}

fn until_play(e: &mut Engine) {
    for _ in 0..60_000 / tt_engine::TICK_MS {
        if content(e) == "PlayGui" && e.control_object().is_some() {
            return;
        }
        tick(e);
    }
    panic!("no game: {}", content(e));
}

/// Renders the engine's frame at each size into `<dir>/<name>_<W>x<H>.png`.
fn shot(e: &mut Engine, name: &str, sizes: &[(u32, u32)]) {
    let Some(dir) = out_dir() else { return };
    for &size in sizes {
        let off = tt_engine::gfx::Offscreen::new(size).unwrap();
        let mut fe = tt_engine::frontend::Frontend::new(&off.device, &off.queue, off.format, assets(), false);
        e.host.scene_dirty = true;
        fe.step(e, &off.device, &off.queue, 0.0);
        // A few frames: menu/HUD layers settle (fonts, textures).
        for _ in 0..3 {
            fe.draw(e, &off.device, &off.queue, &off.view, off.size, 16.0);
        }
        let path = dir.join(format!("{name}_{}x{}.png", size.0, size.1));
        off.save(&path).unwrap();
        eprintln!("saved {}", path.display());
    }
}

const SIZES: &[(u32, u32)] = &[(1280, 720), (1920, 1080)];

fn bot(name: &str, team: Team, tank: usize) -> BotSetup {
    BotSetup { name: name.into(), skill: BotSkill::Silver, tank, team }
}

fn custom(mission: &str, mode: u32) -> CustomGame {
    let bots = vec![bot("Hum", Team::Blue, 2), bot("Gum", Team::Green, 1), bot("Tom", Team::Green, 0), bot("Mal", Team::Blue, 1), bot("Ri", Team::Green, 2)];
    CustomGame { mission: tt_assets::layout::mission(mission), mode, bots, player_tank: 1, player_team: Team::Blue, minutes: 5, replace_bots: true }
}

/// The player's tank (pos, forward).
fn me(e: &Engine) -> Option<(tt_game::TankId, Vec3, Vec3)> {
    let g = e.host.game.as_ref()?;
    let id = g.tank_of(e.control_object()?)?;
    let t = g.world.tank(id)?;
    Some((id, t.pos, t.rot.y_axis))
}

/// Puts the first three other living tanks nearly in a line `dist` ahead
/// of the player and holds them there (x, y) for `ms` while they drop to
/// the ground: their name labels land on top of each other.
fn crowd(e: &mut Engine, dist: f32, ms: u32) {
    let Some((mine, pos, fwd)) = me(e) else { return };
    let side = Vec3::new(fwd.y, -fwd.x, 0.0).normalize_or_zero();
    let g = e.host.game.as_mut().unwrap();
    let others: Vec<_> = g.world.tanks.iter().filter(|t| t.id != mine && !t.deleted && !t.is_corpse).map(|t| t.id).take(3).collect();
    // 6 m apart along the view: outside each other's spawn clearance box.
    let spots: Vec<(tt_game::TankId, Vec3)> = others.into_iter().enumerate().map(|(i, id)| (id, pos + fwd * (dist + i as f32 * 6.0) + side * [0.0, 0.9, -0.9][i])).collect();
    for &(id, p) in &spots {
        g.world.set_tank_transform(id, p, None);
    }
    for _ in 0..ms / tt_engine::TICK_MS {
        let g = e.host.game.as_mut().unwrap();
        for &(id, p) in &spots {
            if let Some(t) = g.world.tank_mut(id) {
                t.pos.x = p.x;
                t.pos.y = p.y;
                t.vel.x = 0.0;
                t.vel.y = 0.0;
            }
        }
        tick(e);
    }
}

#[test]
#[ignore]
fn solo() {
    let root = assets();
    big_stack(move || {
        let mut e = engine(&root, "solo", &[]);
        assert!(e.boot());
        new_player(&mut e, "Tester");
        apply(&mut e, MenuAction::StartSolo);
        until_play(&mut e);
        ticks(&mut e, 4000);
        shot(&mut e, "solo_start", SIZES);
        {
            use tt_gameplay::{ServerMsg, Value};
            let host = &mut e.host;
            let p = |t: &str| (Value::str(t), "5".to_string(), "1".to_string());
            let (text, secs, lines) = p("Destroy all enemy brains!");
            tt_engine::gameplay::client::recv::apply(host, &ServerMsg::CenterPrint { text, secs, lines, lc: false });
            let (text, secs, lines) = p("Watch out for the turrets");
            tt_engine::gameplay::client::recv::apply(host, &ServerMsg::BottomPrint { text, secs, lines, lc: false });
        }
        ticks(&mut e, 300);
        shot(&mut e, "solo_prints", &[(1280, 720)]);
    });
}

/// A brain level (campaign position 5, greenbrain): the brain counter;
/// with a score, extra lives and the clock as the scripts set them.
#[test]
#[ignore]
fn solo_brains() {
    let root = assets();
    big_stack(move || {
        let mut e = engine(&root, "solobrains", &[]);
        assert!(e.boot());
        new_player(&mut e, "Tester");
        apply(&mut e, MenuAction::StartSoloAt(5));
        until_play(&mut e);
        ticks(&mut e, 5000);
        let now = e.host.sim_time() as u64;
        let c = &mut e.host.client;
        c.solo.set_score("48250");
        c.solo.show_lives("7");
        c.clock.set_time("42", now);
        c.clock.visible = true;
        ticks(&mut e, 200);
        shot(&mut e, "solo_brains_greenbrain", SIZES);
    });
}

#[test]
#[ignore]
fn team() {
    let root = assets();
    big_stack(move || {
        let mut e = engine(&root, "team", &[]);
        assert!(e.boot());
        new_player(&mut e, "Tester");
        apply(&mut e, MenuAction::StartCustom(custom("tt1_1", 1)));
        until_play(&mut e);
        ticks(&mut e, 3000);
        chat(&mut e, "gg everyone, nice shot", false);
        chat(&mut e, "cover me, going for the ammo", true);
        ticks(&mut e, 25_000);
        chat(&mut e, "This is a rather long chat line that has to wrap onto a second line in the HUD", false);
        ticks(&mut e, 600);
        shot(&mut e, "team_tt1_1", SIZES);
        // The chat input open with text typed.
        tt_engine::gameplay::client::hud::message_hud_key(&mut e.host, "1", true);
        for c in "on my way".chars() {
            let k = if c == ' ' { "space".to_string() } else { c.to_string() };
            e.key(&k, true, Some(&c.to_string()));
            e.key(&k, false, None);
        }
        ticks(&mut e, 100);
        shot(&mut e, "team_chat_input", &[(1280, 720)]);
        tt_engine::gameplay::client::hud::close_chat(&mut e.host);
        // Tab: the player list off.
        tt_engine::gameplay::client::hud::toggle_player_list(&mut e.host);
        ticks(&mut e, 100);
        shot(&mut e, "team_list_hidden", &[(1280, 720)]);
        tt_engine::gameplay::client::hud::toggle_player_list(&mut e.host);
        // Crowded name tags.
        crowd(&mut e, 40.0, 3000);
        shot(&mut e, "team_name_tags", SIZES);
    });
}

#[test]
#[ignore]
fn scrum() {
    let root = assets();
    big_stack(move || {
        let mut e = engine(&root, "scrum", &[]);
        assert!(e.boot());
        new_player(&mut e, "Tester");
        apply(&mut e, MenuAction::StartCustom(custom("tt2_3", 2)));
        until_play(&mut e);
        ticks(&mut e, 20_000);
        shot(&mut e, "scrum_tt2_3", SIZES);
    });
}

#[test]
#[ignore]
fn network_client() {
    let root = assets();
    big_stack(move || {
        let net = MemNet::new(11);
        let mut server = engine(&root, "net-server", &["-dedicated", "-mission", "missions/tt2_1.ron"]);
        server.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 1]));
        assert!(server.boot());
        let mut c = engine(&root, "net-client", &[]);
        assert!(c.boot());
        new_player(&mut c, "Remote");
        c.host.netgame.factory = Some(mem_factory(&net, [10, 0, 1, 1]));
        tt_engine::net::connect_to(&mut c.host, "10.0.0.1:28000", "", "Remote", "");
        let mut t = 0u64;
        let mut step = |server: &mut Engine, c: &mut Engine| {
            t += 32;
            net.set_now(Duration::from_millis(t));
            for e in [&mut *server, &mut *c] {
                e.host.virtual_clock_ms = Some(t as f64);
                e.tick();
            }
        };
        for _ in 0..(40_000 / 32) {
            step(&mut server, &mut c);
            if tt_engine::netgame::control_object(&c.host).is_some() && content(&c) == "PlayGui" {
                break;
            }
        }
        assert!(tt_engine::netgame::control_object(&c.host).is_some(), "client not in play");
        for _ in 0..(20_000 / 32) {
            step(&mut server, &mut c);
        }
        {
            let (kind, text) = tt_gameplay::server::Server::message_of("MsgChat", "\\c2Server: welcome to the LAN game", &[]);
            tt_engine::gameplay::server_host::with_server(&mut server.host, |s, b| s.message_all(b, &kind, &text));
        }
        // The lag icon as `setLagIcon` shows it when snapshots stop.
        c.host.client.lag = true;
        for _ in 0..20 {
            step(&mut server, &mut c);
        }
        shot(&mut c, "netclient_tt2_1", SIZES);
    });
}

//! Network play, stages 3–4: a dedicated server with two remote clients over `MemNet`
//! (LAN and DSL profiles). Each client sees the other tank move and shoot; tank
//! poses, corpses, power-ups and the scrum ball agree with the server's history at
//! the client's render time; scores agree; bandwidth per client is measured with 2
//! and 8 tanks.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use glam::Vec3;
use tt_engine::Engine;
use tt_net::{LinkConditions, MemNet};

mod common;
use common::*;

const MISSION: &str = "missions/tt1_1.ron";

/// Server state per tick, for comparing with what the clients show.
#[derive(Default)]
struct History {
    tanks: HashMap<u32, HashMap<u32, (Vec3, bool)>>,
    triggers: HashMap<u32, HashMap<u32, bool>>,
    flags: HashMap<u32, HashMap<u32, (Vec3, u32)>>,
}

struct Game {
    net: MemNet,
    server: Engine,
    clients: Vec<Engine>,
    t: u64,
    hist: History,
}

impl Game {
    fn new(root: &Path, tag: &str, cond: LinkConditions, server_args: &[&str], clients: usize, bots: usize) -> Game {
        let net = MemNet::new(11);
        net.set_conditions(cond);
        let mut args = vec!["-dedicated", "-mission", MISSION];
        args.extend_from_slice(server_args);
        let mut server = engine(root, &format!("{tag}-server"), &args);
        server.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 1]));
        assert!(server.boot());
        // The dedicated server starts with its default 6 bots; `bots == 0`
        // drops them (a bot-free game).
        if bots == 0 {
            server.host.srv.state_mut().bot_slots.clear();
            for b in tt_engine::gameplay::server_host::bots(&server.host) {
                tt_engine::gameplay::server_host::with_server(&mut server.host, |s, h| s.drop_bot(h, b));
            }
        } else {
            // One bot slot more than bots wanted: the server drops one bot
            // per human beyond the first (updateTeamBotCount).
            {
                let slots = &mut server.host.srv.state_mut().bot_slots;
                slots.resize(7, Default::default());
                slots[6] = tt_gameplay::server::BotSlot::new("SilverMediumTank", "Extra [Bot]", "BlueTeam");
            }
            tt_engine::gameplay::server_host::with_server(&mut server.host, |s, b| s.balance_bots(b));
        }
        let mut cl = Vec::new();
        for i in 0..clients {
            let mut c = engine(root, &format!("{tag}-client{i}"), &[]);
            assert!(c.boot());
            new_player(&mut c, &format!("Player{i}"));
            c.host.netgame.factory = Some(mem_factory(&net, [10, 0, 1, i as u8 + 1]));
            tt_engine::net::connect_to(&mut c.host, "10.0.0.1:28000", "", &format!("Player{i}"), "");
            cl.push(c);
        }
        Game { net, server, clients: cl, t: 0, hist: History::default() }
    }

    fn step(&mut self) {
        self.t += 32;
        self.net.set_now(Duration::from_millis(self.t));
        self.server.host.virtual_clock_ms = Some(self.t as f64);
        self.server.tick();
        if let Some(g) = self.server.host.game.as_ref() {
            let tick = (g.world.time_ms / 32) as u32;
            self.hist.tanks.insert(tick, g.world.tanks.iter().filter(|t| !t.deleted).map(|t| (t.id.0, (t.pos, t.is_corpse))).collect());
            self.hist.triggers.insert(tick, g.world.triggers.iter().map(|t| (t.id.0, t.on)).collect());
            self.hist.flags.insert(tick, g.world.flags.iter().map(|f| (f.id.0, (f.pos, f.carrier.map_or(0, |c| c.0)))).collect());
        }
        for c in &mut self.clients {
            c.host.virtual_clock_ms = Some(self.t as f64);
            c.tick();
        }
    }

    fn in_play(&self) -> bool {
        self.clients.iter().all(|c| tt_engine::netgame::control_object(&c.host).is_some())
    }

    /// Server tank id of each remote client's control tank.
    fn control_nets(&self) -> Vec<u32> {
        let g = self.server.host.game.as_ref().unwrap();
        let mut conns: Vec<_> = self.server.host.net.control.iter().filter(|(c, _)| tt_engine::netgame::is_remote(&self.server.host, **c)).collect();
        conns.sort();
        conns.into_iter().filter_map(|(_, o)| g.tank_of(*o)).map(|t| t.0).collect()
    }
}

/// Pose agreement of client `c` with the server history at its render time:
/// (samples, within 0.25 m, worst error).
fn compare_tanks(g: &Game, c: &Engine) -> (usize, usize, f32) {
    let rep = c.host.netgame.client_rep().unwrap();
    let r = rep.render_tick;
    let k = r.floor() as u32;
    let f = (r - r.floor()) as f32;
    let (Some(a), Some(b)) = (g.hist.tanks.get(&k), g.hist.tanks.get(&(k + 1))) else { return (0, 0, 0.0) };
    let cg = c.host.game.as_ref().unwrap();
    let (mut n, mut ok, mut worst) = (0, 0, 0f32);
    for (net, (_, tid)) in &rep.tanks {
        // The own tank is predicted (ahead of the render time).
        if rep.pred.predicting(*net) {
            continue;
        }
        let (Some(pa), Some(pb), Some(t)) = (a.get(net), b.get(net), cg.world.tank(*tid)) else { continue };
        let want = pa.0.lerp(pb.0, f);
        let err = (t.pos - want).length();
        n += 1;
        // Teleports (respawn) inside the interval are not interpolable.
        if err < 0.25 || (pa.0 - pb.0).length() > 5.0 {
            ok += 1;
        }
        worst = worst.max(err);
    }
    (n, ok, worst)
}

fn drive(g: &mut Game, tick: u64) {
    for (i, c) in g.clients.iter_mut().enumerate() {
        let turn = i32::from((tick / 90 + i as u64).is_multiple_of(2));
        c.host.client.moves.set_drive(1.0, turn as f32);
        if tick % 30 == i as u64 * 7 {
            c.host.client.moves.trigger(0);
        }
        if tick % 125 == 3 {
            tt_engine::net::send_cmd(&mut c.host, tt_gameplay::ClientCmd::SpawnPlayer);
        }
    }
}

fn play(root: &Path, tag: &str, cond: LinkConditions, server_args: &[&str], bots: usize, seconds: u64) -> Game {
    let mut g = Game::new(root, tag, cond, server_args, 2, bots);
    let mut ticks = 0;
    while !g.in_play() {
        g.step();
        ticks += 1;
        if ticks > 3000 {
            dump_log(&g.server, "server");
            for c in &g.clients {
                dump_log(c, "client");
            }
            panic!("clients did not reach the game");
        }
    }
    for tick in 0..seconds * 1000 / 32 {
        drive(&mut g, tick);
        g.step();
    }
    g
}

fn check_game(g: &mut Game, tag: &str) {
    let nets = g.control_nets();
    assert_eq!(nets.len(), 2, "{tag}: both clients have a tank on the server");
    for (i, c) in g.clients.iter().enumerate() {
        let rep = c.host.netgame.client_rep().unwrap();
        let other = nets[1 - i];
        assert!(rep.tanks.contains_key(&other) || g.hist.tanks.values().any(|m| !m.contains_key(&other)), "{tag}: client {i} sees the other tank");
    }
}

/// The full acceptance run on one link profile.
fn acceptance(root: &Path, tag: &str, cond: LinkConditions) {
    let mut g = Game::new(root, tag, cond, &[], 2, 6);
    let mut ticks = 0;
    while !g.in_play() {
        g.step();
        ticks += 1;
        assert!(ticks < 3000, "{tag}: clients did not reach the game");
    }
    let nets = g.control_nets();
    assert_eq!(nets.len(), 2);
    // Each client: the other's tank moving, its shots, corpses, and samples.
    let mut seen_other_moving = [false; 2];
    let mut seen_other_shots = [false; 2];
    let mut first_pos: [Option<Vec3>; 2] = [None, None];
    let mut corpses = [false; 2];
    let (mut samples, mut good, mut worst) = (0usize, 0usize, 0f32);
    let (mut trig_samples, mut trig_good) = (0usize, 0usize);
    for tick in 0..(75_000 / 32) {
        drive(&mut g, tick);
        g.step();
        let nets = g.control_nets();
        for i in 0..2 {
            let c = &g.clients[i];
            let rep = c.host.netgame.client_rep().unwrap();
            let cg = c.host.game.as_ref().unwrap();
            let Some(&other) = nets.get(1 - i) else { continue };
            if let Some((_, tid)) = rep.tanks.get(&other) {
                if let Some(t) = cg.world.tank(*tid) {
                    let p0 = *first_pos[i].get_or_insert(t.pos);
                    if (t.pos - p0).length() > 10.0 {
                        seen_other_moving[i] = true;
                    }
                }
                if cg.world.projectiles.iter().any(|p| p.source == Some(*tid)) {
                    seen_other_shots[i] = true;
                }
            }
            if rep.tanks.values().any(|(_, t)| cg.world.tank(*t).is_some_and(|t| t.is_corpse)) {
                corpses[i] = true;
            }
            if tick % 8 == 0 && tick > 100 {
                let (n, ok, w) = compare_tanks(&g, c);
                samples += n;
                good += ok;
                worst = worst.max(w);
                // Power-up states at the render time (±1 tick).
                let k = rep.render_tick.floor() as u32;
                if let (Some(a), Some(b)) = (g.hist.triggers.get(&k), g.hist.triggers.get(&(k + 1))) {
                    for (net, tid) in &rep.triggers {
                        let Some(t) = cg.world.triggers.iter().find(|t| t.id == *tid) else { continue };
                        let (Some(&sa), Some(&sb)) = (a.get(net), b.get(net)) else { continue };
                        trig_samples += 1;
                        if t.on == sa || t.on == sb {
                            trig_good += 1;
                        }
                    }
                }
            }
        }
    }
    let sg = g.server.host.game.as_ref().unwrap();
    let kills = sg.event_counts.get("Killed").copied().unwrap_or(0);
    let pickups = sg.event_counts.get("PowerUpSwitched").copied().unwrap_or(0);
    let fired = sg.event_counts.get("Fired").copied().unwrap_or(0);
    println!("{tag}: {} tanks, fired {fired}, kills {kills}, power-up switches {pickups}; poses {good}/{samples} within 0.25 m (worst {worst:.2} m); power-ups {trig_good}/{trig_samples}", sg.world.tanks.len());
    for (i, c) in g.clients.iter().enumerate() {
        let rep = c.host.netgame.client_rep().unwrap();
        println!("{tag}: client {i}: delay {:.1} ticks, {} snapshots ({} undecodable), {} tanks", rep.delay, rep.snapshots, rep.bad_snapshots, rep.tanks.len());
    }
    for (o, st, snap) in tt_engine::netgame::server_stats(&mut g.server.host) {
        println!("{tag}: to client {o}: {:.0} B/s down, {:.0} B/s up, rtt {:.0} ms, loss {:.1} %, snapshot avg {snap:.0} B", st.bytes_out_per_sec, st.bytes_in_per_sec, st.rtt.as_secs_f64() * 1000.0, st.loss * 100.0);
    }
    for i in 0..2 {
        assert!(seen_other_moving[i], "{tag}: client {i} sees the other tank move");
        assert!(seen_other_shots[i], "{tag}: client {i} sees the other tank's shots");
    }
    assert!(fired > 20);
    assert!(samples > 500 && good * 100 >= samples * 97, "{tag}: poses {good}/{samples}, worst {worst}");
    assert!(trig_samples > 100 && trig_good * 100 >= trig_samples * 98, "{tag}: power-ups {trig_good}/{trig_samples}");
    if kills > 0 {
        assert!(corpses.iter().any(|c| *c), "{tag}: corpses shown");
    }
    // Scores: every client's player list matches the server's records.
    for _ in 0..60 {
        g.step();
    }
    let ids: Vec<u32> = g.server.host.net.server.into_iter().chain(tt_engine::netgame::remote_conns(&g.server.host)).collect();
    let mut scored = 0;
    for conn in ids {
        let Some(c) = tt_engine::gameplay::server_host::client_of_conn(&g.server.host, conn) else { continue };
        let id = c.0;
        let want = tt_engine::gameplay::server_host::record(&g.server.host, c).map(|r| r.score_text()).unwrap_or_default();
        for (i, c) in g.clients.iter_mut().enumerate() {
            // The score field of the client's player list row.
            let got = c.host.client.players.players.text(&id.to_string()).map(|r| tt_gameplay::client::text::get_field(r, 2)).unwrap_or_default();
            if got.is_empty() {
                assert!(want.is_empty() || want == "0", "{tag}: client {i} never got score {want} of {id}");
                continue;
            }
            assert_eq!(got, want, "{tag}: client {i} score of {id}");
            scored += 1;
        }
    }
    println!("{tag}: {scored} score entries agree");
}

#[test]
fn two_clients_lan() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || acceptance(&root, "lan", LinkConditions::lan()));
}

#[test]
fn two_clients_dsl() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || acceptance(&root, "dsl", LinkConditions::dsl()));
}

#[test]
fn scrum_ball_replicates() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut g = play(&root, "scrum", LinkConditions::dsl(), &["-game", "S"], 2, 5);
        check_game(&mut g, "scrum");
        let (mut n, mut ok, mut carried) = (0, 0, 0);
        for tick in 0..(40_000 / 32) {
            drive(&mut g, tick);
            g.step();
            for c in &g.clients {
                let rep = c.host.netgame.client_rep().unwrap();
                let cg = c.host.game.as_ref().unwrap();
                let k = rep.render_tick.floor() as u32;
                let fr = (rep.render_tick - rep.render_tick.floor()) as f32;
                let (Some(a), Some(b)) = (g.hist.flags.get(&k), g.hist.flags.get(&(k + 1))) else { continue };
                for (net, (_, fid)) in &rep.flags {
                    let (Some(fa), Some(fb), Some(f)) = (a.get(net), b.get(net), cg.world.flag(*fid)) else { continue };
                    n += 1;
                    if fa.1 != 0 {
                        carried += 1;
                    }
                    if (f.pos - fa.0.lerp(fb.0, fr)).length() < 0.5 || (fa.0 - fb.0).length() > 5.0 {
                        ok += 1;
                    }
                }
            }
        }
        println!("scrum: ball samples {ok}/{n} within 0.5 m, {carried} carried");
        assert!(n > 500 && ok * 100 >= n * 95, "ball {ok}/{n}");
    });
}

/// Bytes per second per client with 2 and 8 tanks (§4.3 estimates ~9.5 KB/s
/// with 16 tanks of which 10 move, 31.25 Hz).
#[test]
fn bandwidth_2_and_8_tanks() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        // No bots (2 tanks), and the default 6 bots (8 tanks).
        for bots in [0usize, 6] {
            let mut g = play(&root, &format!("bw{bots}"), LinkConditions::lan(), &[], bots, 30);
            let w = &g.server.host.game.as_ref().unwrap().world;
            let tanks = 2 + tt_engine::gameplay::server_host::bots(&g.server.host).len();
            let moving = w.tanks.iter().filter(|t| t.vel.length() > 0.5).count();
            let mut ev: Vec<_> = g.server.host.game.as_ref().unwrap().event_counts.iter().map(|(k, v)| format!("{k} {v}")).collect();
            ev.sort();
            println!("bandwidth: {tanks} tanks ({} objects incl. corpses), {moving} moving; events in 30 s: {}", w.tanks.len(), ev.join(", "));
            for (o, st, snap) in tt_engine::netgame::server_stats(&mut g.server.host) {
                println!("bandwidth: {tanks} tanks: client {o}: {:.0} B/s down ({:.1} KB/s), {:.0} B/s up, snapshot avg {snap:.0} B", st.bytes_out_per_sec, st.bytes_out_per_sec / 1024.0, st.bytes_in_per_sec);
                assert!(st.bytes_out_per_sec < 12.0 * 1024.0, "down {:.0} B/s", st.bytes_out_per_sec);
                assert!(st.bytes_in_per_sec < 2.5 * 1024.0, "up {:.0} B/s", st.bytes_in_per_sec);
            }
        }
    });
}

// ---- stage 5: prediction -----------------------------------------------------------

fn own_tank(c: &Engine) -> Option<(u32, tt_game::TankId)> {
    let rep = c.host.netgame.client_rep()?;
    rep.tanks.get(&rep.control_net).map(|(_, t)| (rep.control_net, *t))
}

fn predict_report(tag: &str, c: &Engine) -> tt_engine::netgame::predict::PredictStats {
    let st = tt_engine::netgame::predict_stats(&c.host).unwrap();
    let mut e = st.errors.clone();
    e.sort_by(f32::total_cmp);
    let pct = |p: f64| e.get(((e.len() as f64 - 1.0).max(0.0) * p).round() as usize).copied().unwrap_or(0.0);
    let d = &st.depth;
    let mean = d.iter().map(|x| *x as f64).sum::<f64>() / d.len().max(1) as f64;
    let in_band = d.iter().filter(|x| (0..=2).contains(*x)).count() as f64 / d.len().max(1) as f64;
    println!(
        "{tag}: {} reconciles, {} mispredicted ({:.2} %), error mean {:.3} m p50 {:.3} p99 {:.3} max {:.3}, {} snaps; {} moves in {} ticks; server queue depth mean {mean:.2}, {:.0} % in 0..=2",
        st.compared,
        st.mispredicted,
        st.mispredicted as f64 * 100.0 / st.compared.max(1) as f64,
        e.iter().sum::<f32>() / e.len().max(1) as f32,
        pct(0.5),
        pct(0.99),
        e.last().copied().unwrap_or(0.0),
        st.snaps,
        st.moves,
        st.ticks,
        in_band * 100.0
    );
    if !st.fields.is_empty() {
        println!("{tag}: first differing owner words {:?}", st.fields);
    }
    println!("{tag}: own shots: {} predicted, {} confirmed by the server, {} faded, {} server shots not predicted; {} spawns within 5 cm of ours (max {:.3} m)", st.shots_predicted, st.shots_confirmed, st.shots_unconfirmed, st.shots_unpredicted, st.shots_exact, st.shot_error_max);
    st
}

fn drive_varied(c: &mut Engine, tick: u64) {
    let phase = (tick / 60) % 5;
    let (fwd, right) = [(1, 0), (1, 1), (1, 0), (0, 0), (1, -1)][phase as usize];
    let (r, l) = if right > 0 { (1, 0) } else if right < 0 { (0, 1) } else { (0, 0) };
    let yaw = ((tick as f32) * 0.05).sin() * 0.02;
    {
        use tt_gameplay::client::binds::MoveAxis;
        let m = &mut c.host.client.moves;
        m.set_axis(MoveAxis::Forward, &fwd.to_string());
        m.set_axis(MoveAxis::Right, &r.to_string());
        m.set_axis(MoveAxis::Left, &l.to_string());
        m.yaw = tt_gameplay::client::text::SVal::Float(yaw);
    }
    if tick % 125 == 3 {
        tt_engine::net::send_cmd(&mut c.host, tt_gameplay::ClientCmd::SpawnPlayer);
    }
    if tick % 40 == 7 {
        c.host.client.moves.trigger(0);
    }
}

/// Alone on the map at 150 ms RTT: the own tank answers the next tick and
/// the prediction never differs from the server (§7).
#[test]
fn prediction_alone_is_exact() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let cond = LinkConditions { latency: Duration::from_millis(75), jitter: Duration::from_millis(5), ..Default::default() };
        let mut g = Game::new(&root, "alone", cond, &[], 1, 0);
        let mut ticks = 0;
        while !g.in_play() || g.clients[0].host.netgame.client_rep().unwrap().pred.tank.is_none() {
            g.step();
            ticks += 1;
            assert!(ticks < 3000, "not in play");
        }
        for _ in 0..60 {
            g.step();
        }
        // Responsiveness: forward is pressed, the tank moves on the next tick.
        let (_, tid) = own_tank(&g.clients[0]).unwrap();
        let p0 = g.clients[0].host.game.as_ref().unwrap().world.tank(tid).unwrap().pos;
        g.clients[0].host.client.moves.set_drive(1.0, 0.0);
        g.step();
        let p1 = g.clients[0].host.game.as_ref().unwrap().world.tank(tid).unwrap().pos;
        println!("alone: the own tank moved {:.3} m on the tick forward was pressed (RTT 150 ms)", (p1 - p0).length());
        assert!((p1 - p0).length() > 0.005, "moved {} m on the first tick", (p1 - p0).length());
        // Five minutes (§7), driving around (pads included when it meets them).
        for tick in 0..(300_000 / 32) {
            drive_varied(&mut g.clients[0], tick);
            g.step();
        }
        let sg = g.server.host.game.as_ref().unwrap();
        println!("alone: server events: pads {}, deaths {}", sg.event_counts.get("PadUsed").copied().unwrap_or(0), sg.event_counts.get("Killed").copied().unwrap_or(0));
        let st = predict_report("alone 150 ms", &g.clients[0]);
        assert!(st.compared > 500, "{} reconciles", st.compared);
        assert_eq!(st.mispredicted, 0, "errors {:?}", &st.errors[..st.errors.len().min(10)]);
        // Own shots: every one shown at once and confirmed (§7 projectiles).
        assert!(st.shots_predicted > 50, "{} shots", st.shots_predicted);
        assert!(st.shots_confirmed + 2 >= st.shots_predicted && st.shots_unconfirmed <= 2 && st.shots_unpredicted == 0, "shots {} {} {} {}", st.shots_predicted, st.shots_confirmed, st.shots_unconfirmed, st.shots_unpredicted);
        // The others exploded in their first tick (the server spawn is the hit point).
        assert!(st.shots_exact * 100 >= st.shots_confirmed * 75, "{} of {} spawns exact", st.shots_exact, st.shots_confirmed);
        let d = &st.depth[st.depth.len() / 4..];
        let mean = d.iter().map(|x| *x as f64).sum::<f64>() / d.len() as f64;
        assert!((0.5..=2.5).contains(&mean), "queue depth {mean}");
    });
}

/// With bots around: misprediction rate and correction sizes per profile.
#[test]
fn prediction_with_bots() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        for (tag, cond) in [("dsl", LinkConditions::dsl()), ("bad_wifi", LinkConditions::bad_wifi())] {
            let mut g = Game::new(&root, &format!("pred-{tag}"), cond, &[], 2, 6);
            let mut ticks = 0;
            while !g.in_play() {
                g.step();
                ticks += 1;
                assert!(ticks < 3000, "{tag}: not in play");
            }
            for tick in 0..(60_000 / 32) {
                drive(&mut g, tick);
                g.step();
            }
            for (i, c) in g.clients.iter().enumerate() {
                let st = predict_report(&format!("{tag} client {i}"), c);
                assert!(st.compared > 300, "{tag}: {} reconciles", st.compared);
            }
        }
    });
}

// ---- WP-D: administration and status ----------------------------------------------

fn play_until_in(g: &mut Game) {
    let mut ticks = 0;
    while !g.in_play() {
        g.step();
        ticks += 1;
        assert!(ticks < 3000, "not in play");
    }
}

/// Server connection objects of the remote clients, in client order.
fn remote_conns(g: &Game) -> Vec<u32> {
    let mut v: Vec<(String, u32)> = Vec::new();
    for m in tt_engine::netgame::remote_conns(&g.server.host) {
        let a = tt_engine::netgame::address(&g.server.host, m).unwrap();
        v.push((a, m));
    }
    v.sort();
    v.into_iter().map(|(_, m)| m).collect()
}

#[test]
fn ping_column_lag_icon_kick_and_ban() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut g = Game::new(&root, "admin", LinkConditions::dsl(), &[], 2, 0);
        play_until_in(&mut g);
        for _ in 0..120 {
            g.step();
        }
        let conns = remote_conns(&g);
        assert_eq!(conns.len(), 2);
        // The player list is keyed by the server's client ids.
        let ids: Vec<u32> = conns.iter().map(|&o| tt_engine::gameplay::server_host::client_of_conn(&g.server.host, o).unwrap().0).collect();
        // Ping column: every client has every remote client's ping, in the row.
        for c in &mut g.clients {
            for id in &ids {
                let p = c.host.client.players.pings.get(&(*id as i32)).cloned().unwrap_or_default();
                assert!(p.parse::<u32>().is_ok_and(|p| (60..400).contains(&p)), "ping of {id}: {p:?}");
                let row = c.host.client.players.players.text(&id.to_string()).unwrap_or_default().to_string();
                // After the K/D/A columns.
                let at = tt_gameplay::client::players::fields(true).0;
                assert_eq!(row.split('\t').nth(at), Some(p.as_str()), "row {row:?}");
            }
        }
        // Lag icon: the link dies for a second, then comes back.
        assert!(!g.clients[0].host.client.lag);
        g.net.set_conditions(LinkConditions { loss: 1.0, ..Default::default() });
        for _ in 0..30 {
            g.step();
        }
        assert!(g.clients[0].host.client.lag);
        g.net.set_conditions(LinkConditions::dsl());
        for _ in 0..30 {
            g.step();
        }
        assert!(!g.clients[0].host.client.lag);
        // Ban the second client: it gets the reason, its address is banned,
        // and it cannot come back. The game defines no ban durations: 0 s,
        // no lasting ban, unless the prefs are set.
        g.server.set_var("server.ban_time", "600");
        let c = tt_engine::gameplay::server_host::client_of_conn(&g.server.host, conns[1]).unwrap();
        tt_engine::gameplay::server_host::with_server(&mut g.server.host, |s, b| s.ban(b, c));
        for _ in 0..30 {
            g.step();
        }
        let msg = conn_err(&g.clients[1]);
        assert!(msg.contains("kicked from this server") || msg.contains("banned from this server"), "{msg:?}");
        assert_eq!(remote_conns(&g).len(), 1);
        g.clients[1].host.shell.message = None;
        tt_engine::net::connect_to(&mut g.clients[1].host, "10.0.0.1:28000", "", "Player1", "");
        for _ in 0..60 {
            g.step();
        }
        assert_eq!(conn_err(&g.clients[1]), "You are not allowed to play on this server.");
        // The ban list survives in banlist.ron.
        let file = tt_engine::netgame::admin::BAN_FILE;
        tt_engine::netgame::admin::export(&mut g.server.host, file);
        let text = String::from_utf8(g.server.host.fs.read(file).unwrap()).unwrap();
        assert!(text.contains("BanList(") && text.contains("ip: \"10.0.1.2\""), "{text}");
        g.server.host.netgame.bans.clear();
        let banned_before = tt_engine::netgame::admin::is_banned(&g.server.host, "10.0.1.2");
        g.server.host.netgame.bans = Vec::new();
        g.server.host.netgame.bans_loaded = false;
        tt_engine::netgame::admin::load(&mut g.server.host);
        assert!(!banned_before && tt_engine::netgame::admin::is_banned(&g.server.host, "10.0.1.2"));
        // The first client was never disturbed.
        assert!(g.clients[0].host.netgame.client_rep().is_some());
    });
}

// ---- soak ----------------------------------------------------------------------------

/// `TT_SOAK_SECONDS` / `TT_SOAK_CLIENTS` override the soak length / size.
fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// 10 minutes, 8 clients and bots on a lossy link, rounds of 60 s with map
/// rotation: every client keeps up (poses agree with the server, few
/// mispredictions), rejoins every new map, and nothing grows without bound.
#[test]
fn soak_eight_clients_bad_wifi_rotation() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let seconds = env_num("TT_SOAK_SECONDS", 600);
        let clients = env_num("TT_SOAK_CLIENTS", 8) as usize;
        let mut g = Game::new(&root, "soak", LinkConditions::bad_wifi(), &["-mission", "GREEN"], clients, 6);
        // Short rounds so the maps rotate (GREEN = the TT1_* maps).
        g.server.host.srv.state_mut().setup.round_secs = 60.0;
        play_until_in(&mut g);
        let start_ms = g.t;
        let (mut samples, mut good) = (0usize, 0usize);
        let mut missions = std::collections::BTreeSet::new();
        let mut loads = 0;
        let mut mid: Option<Vec<Vec<(String, usize)>>> = None;
        let mut peak_projectiles = 0usize;
        let ticks = seconds * 1000 / 32;
        for tick in 0..ticks {
            drive(&mut g, tick);
            g.step();
            let m = g.server.server().session.mission_file.clone();
            if missions.insert(m.clone()) || tick % 32 == 0 {
                loads = g.server.host.log.history.iter().filter(|(_, l)| l.starts_with("*** LOADING MISSION")).count();
            }
            if tick % 16 == 0 {
                for c in &g.clients {
                    if c.host.game.is_none() || c.host.netgame.client_rep().is_none_or(|r| r.tanks.is_empty()) {
                        continue;
                    }
                    let (n, ok, _) = compare_tanks(&g, c);
                    samples += n;
                    good += ok;
                    peak_projectiles = peak_projectiles.max(c.host.game.as_ref().unwrap().world.projectiles.len());
                }
            }
            if tick == ticks / 2 {
                mid = Some(g.clients.iter().map(|c| tt_engine::netgame::debug_sizes(&c.host)).collect());
            }
        }
        // Everyone is (back) in the game on the current map.
        for _ in 0..600 {
            if g.in_play() {
                break;
            }
            g.step();
        }
        let minutes = (g.t - start_ms) as f64 / 60_000.0;
        println!("soak: {minutes:.1} min, {clients} clients + bots, bad_wifi; maps {missions:?} ({loads} mission loads); poses {good}/{samples} within 0.25 m; peak projectiles in a client world {peak_projectiles}");
        let mut total = (0u64, 0u64);
        for (i, c) in g.clients.iter().enumerate() {
            let st = predict_report(&format!("soak client {i}"), c);
            total.0 += st.compared;
            total.1 += st.mispredicted;
        }
        for (o, st, snap) in tt_engine::netgame::server_stats(&mut g.server.host) {
            println!("soak: to client {o}: {:.0} B/s down, {:.0} B/s up, rtt {:.0} ms, loss {:.1} %, snapshot avg {snap:.0} B", st.bytes_out_per_sec, st.bytes_in_per_sec, st.rtt.as_secs_f64() * 1000.0, st.loss * 100.0);
        }
        let end: Vec<Vec<(String, usize)>> = g.clients.iter().map(|c| tt_engine::netgame::debug_sizes(&c.host)).collect();
        println!("soak: client 0 sizes mid {:?}", mid.as_ref().map(|m| &m[0]));
        println!("soak: client 0 sizes end {:?}", end[0]);
        assert!(g.in_play(), "every client back in the game");
        assert!(loads >= 3 && missions.len() >= 2, "maps rotated: {loads} loads, {missions:?}");
        assert!(samples > 1000 && good * 100 >= samples * 95, "poses {good}/{samples}");
        assert!(total.0 > 1000 && total.1 * 100 <= total.0 * 5, "mispredicted {}/{}", total.1, total.0);
        assert!(peak_projectiles < 500, "projectiles {peak_projectiles}");
        // Bounded: no collection of the replication state keeps growing.
        for c in &end {
            for (k, v) in c {
                assert!(*v < 5000, "{k} = {v}");
            }
        }
        assert_eq!(g.server.host.netgame.remotes_len(), clients, "no stale server-side connections");
    });
}

/// A mission with defs of its own (tt1_5's `LeapBoost`): the mission
/// is in the def manifest, so a remote client joining on it declares
/// them before the hash check (it was dropped with "Game data
/// differs"), and has the def its pads use.
#[test]
fn client_joins_mission_with_own_defs() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut g = Game::new(&root, "missiondb", LinkConditions::default(), &["-mission", "missions/tt1_5.ron"], 1, 0);
        play_until_in(&mut g);
        assert_eq!(g.server.server().session.mission_file.clone(), "missions/tt1_5.ron");
        assert!(g.clients[0].host.defs.get("LeapBoost").is_some());
    });
}

/// K/D/A over the network: the server counts a scripted fight between
/// the two remote players (a hit, a kill), the clients' player lists
/// show the server's numbers.
#[test]
fn kda_reaches_network_clients() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut g = Game::new(&root, "kda", LinkConditions::lan(), &[], 2, 0);
        play_until_in(&mut g);
        for _ in 0..60 {
            g.step();
        }
        let conns = remote_conns(&g);
        let tank_of = |g: &Game, conn: u32| {
            let o = g.server.host.net.control.get(&conn).copied().unwrap();
            g.server.host.game.as_ref().unwrap().tank_of(o).unwrap()
        };
        let (t0, t1) = (tank_of(&g, conns[0]), tank_of(&g, conns[1]));
        let client = |g: &Game, conn: u32| tt_engine::gameplay::server_host::client_of_conn(&g.server.host, conn).unwrap();
        let before = |g: &Game, c: u32| tt_engine::gameplay::server_host::kda(&g.server.host, client(g, c));
        let (k0, d1) = (before(&g, conns[0]).kills, before(&g, conns[1]).deaths);
        {
            let w = &mut g.server.host.game.as_mut().unwrap().world;
            w.apply_damage(t1, 1.0, Some(t0));
            w.apply_damage(t1, 1000.0, Some(t0));
        }
        for _ in 0..90 {
            g.step();
        }
        let sb = |g: &Game, c: u32| tt_engine::gameplay::server_host::kda(&g.server.host, client(g, c));
        assert_eq!(sb(&g, conns[0]).kills, k0 + 1);
        assert_eq!(sb(&g, conns[1]).deaths, d1 + 1);
        let want: Vec<(u32, tt_engine::gameplay::scoreboard::Kda)> = conns.iter().map(|&conn| (client(&g, conn).0, tt_engine::gameplay::server_host::kda(&g.server.host, client(&g, conn)))).collect();
        for (i, c) in g.clients.iter_mut().enumerate() {
            for &(id, k) in &want {
                let row = c.host.client.players.players.text(&id.to_string()).unwrap_or_default().to_string();
                let f: Vec<&str> = row.split('\t').collect();
                assert_eq!(&f[3..6], &[k.kills.to_string(), k.deaths.to_string(), k.assists.to_string()], "client {i}, row of {id}: {row:?}");
                let v = c.host.client.players.kda.get(&id).copied().unwrap_or_default();
                assert_eq!((v.kills, v.deaths, v.assists), (k.kills, k.deaths, k.assists));
            }
        }
    });
}

/// The Mortar and Scatter shot over the network: the shooter's own scatter
/// shot shows all seven pellets at once (predicted) and the server
/// confirms each; the other client sees the seven pellets and the mortar
/// shell (spawn events) and the shell's blast; the shooter draws the new
/// reticles (the mortar's with its splash ring).
#[test]
fn power_up_shots_reach_network_clients() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let cond = LinkConditions { latency: Duration::from_millis(40), jitter: Duration::from_millis(2), ..Default::default() };
        let mut g = Game::new(&root, "pups", cond, &[], 2, 0);
        let mut ticks = 0;
        while !g.in_play() || g.clients.iter().any(|c| c.host.netgame.client_rep().unwrap().pred.tank.is_none()) {
            g.step();
            ticks += 1;
            assert!(ticks < 3000, "not in play");
        }
        for _ in 0..60 {
            g.step();
        }
        let (shooter, _) = own_tank(&g.clients[0]).unwrap();
        // Projectiles of `name` the client has (had) in its world: (ids, how many live now).
        // (With each one's flight direction when first seen.)
        type Seen = std::collections::HashMap<u32, glam::Vec3>;
        let seen = |c: &Engine, name: &str, ids: &mut Seen| {
            for p in c.host.game.as_ref().unwrap().world.projectiles.iter().filter(|p| p.data.name == name) {
                // (Zero once it has exploded.)
                let d = p.vel.normalize_or_zero();
                ids.entry(p.id.0).and_modify(|o| if *o == glam::Vec3::ZERO { *o = d }).or_insert(d);
            }
        };
        // The projectiles the shooter, the other client and the server had.
        let fire = |g: &mut Game, name: &str| -> [Seen; 3] {
            {
                let sg = g.server.host.game.as_mut().unwrap();
                let p = sg.projectile(name).unwrap();
                sg.world.set_projectile(tt_game::TankId(shooter), p);
            }
            // The cartridge reaches the shooter (owner state) and its first
            // burst delay passes (1.8 s for the mortar), then a trigger pull.
            for _ in 0..70 {
                g.step();
            }
            let cur = |c: &Engine| own_tank(c).and_then(|(_, t)| c.host.game.as_ref().unwrap().world.tank(t)?.current.as_ref().map(|p| p.name.clone()));
            assert_eq!(cur(&g.clients[0]).as_deref(), Some(name), "the shooter's cartridge");
            assert!(!g.clients[0].reticle_sprites().is_empty(), "{name}: the reticle");
            g.clients[0].host.client.moves.trigger(0);
            let (mut mine, mut theirs, mut server) = (Seen::new(), Seen::new(), Seen::new());
            for _ in 0..30 {
                g.step();
                seen(&g.clients[0], name, &mut mine);
                seen(&g.clients[1], name, &mut theirs);
                seen(&g.server, name, &mut server);
            }
            g.clients[0].host.client.moves.trigger(0);
            [mine, theirs, server]
        };
        let st0 = tt_engine::netgame::predict_stats(&g.clients[0].host).unwrap();
        let [mine, theirs, server] = fire(&mut g, "ScatterProjectile");
        assert_eq!((mine.len(), theirs.len()), (7, 7), "scatter shot pellets: shooter {}, other client {}", mine.len(), theirs.len());
        // The cone (jittered per shot from the shot number): the shooter's
        // prediction and the other client fly every pellet the server's way.
        assert_eq!(server.len(), 7);
        // (The other client may see pellets that hit something point blank
        // only once they have exploded: their direction is unknown there.)
        for (who, c) in [("shooter", &mine), ("other client", &theirs)] {
            let flying: Vec<_> = c.values().filter(|d| **d != glam::Vec3::ZERO).collect();
            assert!(if who == "shooter" { flying.len() == 7 } else { !flying.is_empty() }, "{who}: {} pellets in flight", flying.len());
            for d in flying {
                let off = server.values().map(|s| s.angle_between(*d).to_degrees()).fold(f32::MAX, f32::min);
                assert!(off < 0.1, "{who}: a pellet {off} deg off the server's");
            }
        }
        for _ in 0..60 {
            g.step();
        }
        let st = tt_engine::netgame::predict_stats(&g.clients[0].host).unwrap();
        assert_eq!(st.shots_predicted - st0.shots_predicted, 7, "seven pellets predicted");
        assert_eq!(st.shots_confirmed - st0.shots_confirmed, 7, "each confirmed by its server spawn");
        assert_eq!((st.shots_unpredicted, st.shots_unconfirmed), (st0.shots_unpredicted, st0.shots_unconfirmed));
        // The mortar: one shell, seen by the other client, its blast too.
        let blasts = |c: &Engine| c.host.game.as_ref().unwrap().event_counts.get("ProjectileExploded").copied().unwrap_or(0);
        let b0 = blasts(&g.clients[1]);
        let [mine, theirs, _] = fire(&mut g, "MortarProjectile");
        let (mine, theirs) = (mine.len(), theirs.len());
        assert_eq!((mine, theirs), (1, 1), "mortar shell: shooter {mine}, other client {theirs}");
        {
            let c = &mut g.clients[0];
            let ring = c.reticle_sprites().iter().any(|s| matches!(s.orient, tt_render::sprites::SpriteOrient::Plane { .. }));
            assert!(ring, "the mortar reticle's splash ring");
        }
        for _ in 0..200 {
            g.step();
        }
        assert!(blasts(&g.clients[1]) > b0, "the other client saw the blast");
    });
}

/// The Reload arch's lightning (`playUserThread`) over the network: the
/// server starts the arch's user thread, both clients play it from the
/// start on their copy of the arch (one `UserThread` event each).
#[test]
fn reload_arch_thread_reaches_network_clients() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut g = Game::new(&root, "arch", LinkConditions::lan(), &[], 2, 0);
        play_until_in(&mut g);
        for _ in 0..30 {
            g.step();
        }
        let (tid, pos) = {
            let w = &mut g.server.host.game.as_mut().unwrap().world;
            let t = w.triggers.iter().find(|t| t.def_name().eq_ignore_ascii_case("Reload")).expect("tt1_1 has a Reload arch");
            let (tid, pos) = (t.id, glam::Vec3::from(t.transform.translation));
            assert!(w.play_user_thread(tid, 1, 1.0, true));
            (tid, pos)
        };
        assert!(!g.server.host.game.as_mut().unwrap().world.play_user_thread(tid, 2, 1.0, true), "no sequence in slot 2");
        let arch = |c: &Engine| -> Option<tt_game::Trigger> {
            let w = &c.host.game.as_ref()?.world;
            w.triggers.iter().find(|t| t.def_name().eq_ignore_ascii_case("Reload") && (glam::Vec3::from(t.transform.translation) - pos).length() < 0.1).cloned()
        };
        for c in &g.clients {
            assert!(arch(c).expect("the client's arch").thread.is_none(), "idle before");
        }
        for _ in 0..30 {
            g.step();
        }
        for (i, c) in g.clients.iter().enumerate() {
            let th = arch(c).unwrap().thread.unwrap_or_else(|| panic!("client {i}: no thread on the arch"));
            assert_eq!((th.slot, th.seq.as_str(), th.scale), (1, "activate", 1.0), "client {i}");
            let now = c.host.game.as_ref().unwrap().world.time_ms;
            assert!(now - th.start_ms > 0 && now - th.start_ms < 30 * 32, "client {i}: started {} ms ago", now - th.start_ms);
            assert_eq!(c.host.game.as_ref().unwrap().event_counts.get("UserThreadPlayed").copied().unwrap_or(0), 1, "client {i}: one thread event");
        }
    });
}

/// Whether network client `c` shows a message line containing `text`.
fn shows_line(c: &Engine, text: &str) -> bool {
    c.host.client.messages.lines.iter().any(|l| l.text.contains(text))
}

/// Team chat in a team game over the network: the line reaches the
/// sender and its team mate, not the other team; global chat reaches all.
#[test]
fn team_chat_reaches_only_the_team() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut g = Game::new(&root, "teamchat", LinkConditions::lan(), &["-game", "TB"], 3, 0);
        play_until_in(&mut g);
        for _ in 0..30 {
            g.step();
        }
        // Clients in address order, as `remote_conns` sorts them.
        let teams: Vec<_> = remote_conns(&g)
            .iter()
            .map(|&conn| {
                let c = tt_engine::gameplay::server_host::client_of_conn(&g.server.host, conn).unwrap();
                tt_engine::gameplay::server_host::team_of(&g.server.host, c).expect("a team in a team game")
            })
            .collect();
        let sender = (0..3).find(|&i| (0..3).any(|j| j != i && teams[j] == teams[i])).expect("two on one team");
        let mate = (0..3).find(|&j| j != sender && teams[j] == teams[sender]).unwrap();
        let other = (0..3).find(|&j| teams[j] != teams[sender]).expect("one on the other team");
        tt_engine::net::send_cmd(&mut g.clients[sender].host, tt_gameplay::ClientCmd::TeamMessageSent { text: "team-only-line".into() });
        for _ in 0..30 {
            g.step();
        }
        assert!(shows_line(&g.clients[sender], "team-only-line"), "the sender sees its team line");
        assert!(shows_line(&g.clients[mate], "team-only-line"), "the team mate sees the team line");
        assert!(!shows_line(&g.clients[other], "team-only-line"), "the other team does not");
        tt_engine::net::send_cmd(&mut g.clients[other].host, tt_gameplay::ClientCmd::MessageSent { text: "global-line".into() });
        for _ in 0..30 {
            g.step();
        }
        for (i, c) in g.clients.iter().enumerate() {
            assert!(shows_line(c, "global-line"), "client {i} sees the global line");
        }
    });
}

/// What client `c` draws of the tank of server id `net`: (corpse, brain
/// ejected, death sequence, tilt of the drawn transform from the hull's up).
fn drawn_corpse(c: &Engine, root: &Path, net: u32) -> Option<(bool, bool, Option<usize>, f32)> {
    let rep = c.host.netgame.client_rep()?;
    let tid = rep.tanks.get(&net)?.1;
    let g = c.host.game.as_ref()?;
    let t = g.world.tank(tid)?;
    let shape = tt_render::pose::shape(&root.join(&t.data.shape_file))?;
    let mut inst = tt_render::DynamicInstance { transform: glam::Mat4::from_mat3(t.rot), ..Default::default() };
    tt_engine::game::pose_tank(g, t, 1.0, &shape, &mut inst);
    let tilt = inst.transform.transform_vector3(Vec3::Z).angle_between(t.rot.z_axis);
    Some((t.is_corpse, t.brain_ejected, t.death_sequence.map(|d| d.0), tilt))
}

/// The own tank's death on a network client plays like any other tank's:
/// it shakes, the brain jar pops out and the tank flips over (as the
/// other client draws the same tank); the chase camera frames it like a
/// local game's; after the respawn the new tank is whole and upright.
#[test]
fn own_tank_death_animation_on_a_client() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut g = Game::new(&root, "own-death", LinkConditions::lan(), &[], 2, 0);
        play_until_in(&mut g);
        for _ in 0..60 {
            g.step();
        }
        let conns = remote_conns(&g);
        let tank_of = |g: &Game, conn: u32| {
            let o = g.server.host.net.control.get(&conn).copied().unwrap();
            g.server.host.game.as_ref().unwrap().tank_of(o).unwrap()
        };
        let (t0, t1) = (tank_of(&g, conns[0]), tank_of(&g, conns[1]));
        let net = t1.0;
        assert_eq!(own_tank(&g.clients[1]).map(|o| o.0), Some(net), "client 1 drives tank {net}");
        g.server.host.game.as_mut().unwrap().world.apply_damage(t1, 1000.0, Some(t0));
        // The chase camera a local game would show of the server's corpse.
        let mut local_cam = tt_engine::game::ChaseCam::default();
        let (mut own_tilt, mut other_tilt, mut cam_err) = (0f32, 0f32, 0f32);
        for i in 0..150 {
            g.step();
            let own = drawn_corpse(&g.clients[1], &root, net).unwrap();
            let other = drawn_corpse(&g.clients[0], &root, net).unwrap();
            own_tilt = own_tilt.max(own.3);
            other_tilt = other_tilt.max(other.3);
            let eye = g.clients[1].view_camera().unwrap().pos;
            let sg = g.server.host.game.as_ref().unwrap();
            let st = sg.world.tank(t1).unwrap();
            let terrain = sg.world.terrain.clone();
            let want: Vec3 = local_cam.update(g.t as f64, st.pos, st.rot, &sg.camera_data(t1), |x, y| terrain.as_ref().map(|h| h.height(x, y))).translation.into();
            if i > 10 {
                cam_err = cam_err.max((eye - want).length());
            }
        }
        println!("own-death: drawn tilt {own_tilt:.2} (other client {other_tilt:.2}); camera within {cam_err:.2} m of the local one");
        // The camera frames the corpse like a local game's (the client's pose is a tick or two behind).
        assert!(cam_err < 0.5, "camera off by {cam_err} m");
        let st = g.server.host.game.as_ref().unwrap().world.tank(t1).unwrap();
        let seq = st.death_sequence.map(|d| d.0);
        assert!(st.is_corpse && st.brain_ejected && seq.is_some(), "the server played the death");
        let own = drawn_corpse(&g.clients[1], &root, net).unwrap();
        let other = drawn_corpse(&g.clients[0], &root, net).unwrap();
        assert_eq!((other.0, other.1, other.2), (true, true, seq), "the other client's view of it");
        assert_eq!((own.0, own.1, own.2), (true, true, seq), "the own corpse: brain out, the server's death sequence");
        assert!(other_tilt > 1.5, "the other client sees it flip: {other_tilt}");
        assert!(own_tilt > 1.5, "the own tank flips over: {own_tilt}");
        // SPACE: a new tank, whole and upright.
        tt_engine::net::send_cmd(&mut g.clients[1].host, tt_gameplay::ClientCmd::SpawnPlayer);
        let mut ticks = 0;
        while own_tank(&g.clients[1]).is_none_or(|(n, _)| n == net) || g.clients[1].host.netgame.client_rep().unwrap().pred.tank.is_none() {
            g.step();
            ticks += 1;
            assert!(ticks < 300, "no respawn");
        }
        for _ in 0..10 {
            g.step();
        }
        let (new_net, _) = own_tank(&g.clients[1]).unwrap();
        let fresh = drawn_corpse(&g.clients[1], &root, new_net).unwrap();
        assert_eq!((fresh.0, fresh.1, fresh.2), (false, false, None), "the respawned tank");
        assert!(fresh.3 < 0.01, "upright: {}", fresh.3);
        // The old corpse, no longer predicted, still shows its death.
        if let Some(old) = drawn_corpse(&g.clients[1], &root, net) {
            assert_eq!((old.0, old.1, old.2), (true, true, seq), "the old corpse after the respawn");
        }
    });
}

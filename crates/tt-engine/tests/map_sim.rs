//! A map's bot-match numbers (a design tool, ignored by default): team games of bots only (the local
//! player's tank dies at the start and stays down; `SIM_MODE` the game type, default 1 Team Battlemode,
//! 3 Team Scrum) on each map of `MAP_SIM` (default `tt1_1,tt1_5`), `SIM_RUNS` runs of `SIM_MIN` minutes
//! each. Prints per map: the time to the first hit and the first kill, kills per minute, the teams'
//! points per minute (Scrum: goals), the hit distances, how spread out the fights are (the hits'
//! effective 48 m cells), the share of bot time stuck (under 4 m moved in 10 s) and the deaths without
//! a killer (falls, out of bounds).
//!
//! ```sh
//! MAP_SIM=tt1_1,tt1_3,tt1_5 SIM_RUNS=3 cargo test -p tt-engine --release --test map_sim -- --ignored --nocapture
//! ```

use std::collections::HashMap;

use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, Team};

mod common;
use common::*;

#[derive(Default)]
struct Run {
    first_hit: Option<f32>,
    first_kill: Option<f32>,
    kills: u32,
    falls: u32,
    hit_dist: Vec<f32>,
    hits: Vec<(f32, f32)>,
    alive_samples: u32,
    stuck_samples: u32,
    stuck_at: Vec<(f32, f32)>,
    minutes: f32,
    team_points: f64,
}

fn run(map: &str, k: u32, minutes: u32, mode: u32) -> Run {
    let root = assets();
    let mut e = engine(&root, &format!("sim-{map}-{k}"), &[]);
    assert!(e.boot());
    new_player(&mut e, "Watcher");
    let tanks = [0usize, 1, 2, 1];
    let skills = [BotSkill::Silver, BotSkill::Gold, BotSkill::Silver, BotSkill::Bronze];
    let mut bots = Vec::new();
    for i in 0..4 {
        for (team, tag) in [(Team::Green, "G"), (Team::Blue, "B")] {
            let j = (i + k as usize + if tag == "B" { 1 } else { 0 }) % 4;
            bots.push(BotSetup { name: format!("{tag}{i}"), skill: skills[j], tank: tanks[j], team });
        }
    }
    let g = CustomGame { mission: format!("missions/{map}.ron"), mode, bots, player_tank: 1, player_team: Team::Green, minutes: minutes + 2, replace_bots: false };
    apply(&mut e, MenuAction::StartCustom(g));
    until_spawned(&mut e, tick);
    let (_, me) = player(&e).unwrap();
    let w = &mut e.host.game.as_mut().unwrap().world;
    w.rng.set_seed(0x5eed + 7919 * k);
    w.hurt_me(me, "10000");
    let start = w.time_ms;
    let mut r = Run { minutes: minutes as f32, ..Run::default() };
    // Per tank id: (damage, corpse); trail of (time, pos) every 2 s.
    let mut seen: HashMap<u32, (f32, bool)> = HashMap::new();
    let mut trail: HashMap<u32, Vec<glam::Vec3>> = HashMap::new();
    let ticks = minutes * 60_000 / tt_engine::TICK_MS;
    for n in 0..ticks {
        tick(&mut e);
        let w = &e.host.game.as_ref().unwrap().world;
        let t_s = (w.time_ms - start) as f32 / 1000.0;
        for t in w.tanks.iter().filter(|t| !t.deleted && t.ai.is_some()) {
            let id = t.id.0;
            let (dmg0, corpse0) = seen.get(&id).copied().unwrap_or((t.damage, t.is_corpse));
            if t.damage > dmg0 + 0.01 {
                if let Some(src) = t.last_damage_source.and_then(|s| w.tank(s)).filter(|s| s.id != t.id) {
                    r.first_hit.get_or_insert(t_s);
                    r.hit_dist.push((src.pos - t.pos).length());
                    r.hits.push((t.pos.x, t.pos.y));
                }
            }
            if t.is_corpse && !corpse0 {
                if t.last_damage_source.is_some_and(|s| s != t.id) {
                    r.kills += 1;
                    r.first_kill.get_or_insert(t_s);
                } else {
                    r.falls += 1;
                    eprintln!("  {map}: {} died without a killer at {:.0} {:.0} {:.0}", t.name, t.pos.x, t.pos.y, t.pos.z);
                }
            }
            seen.insert(id, (t.damage, t.is_corpse));
            if n % (2000 / tt_engine::TICK_MS) == 0 {
                let tr = trail.entry(id).or_default();
                if t.is_corpse {
                    tr.clear();
                } else {
                    tr.push(t.pos);
                    if tr.len() > 5 {
                        r.alive_samples += 1;
                        if (tr[tr.len() - 1] - tr[tr.len() - 6]).length() < 4.0 {
                            r.stuck_samples += 1;
                            r.stuck_at.push((t.pos.x, t.pos.y));
                        }
                    }
                }
            }
        }
    }
    r.team_points = e.host.srv.get().map_or(0.0, |s| s.teams.iter().flatten().map(|t| t.rec.score).sum());
    let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    r
}

fn pct(v: &mut [f32], p: f32) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f32 * p).round() as usize]
}

#[test]
#[ignore]
fn map_sim() {
    tt_assets::require_game_data!();
    let maps = std::env::var("MAP_SIM").unwrap_or("tt1_1,tt1_5".into());
    let runs: u32 = std::env::var("SIM_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let minutes: u32 = std::env::var("SIM_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let mode: u32 = std::env::var("SIM_MODE").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    for map in maps.split(',') {
        let map = map.to_string();
        let rs: Vec<Run> = (0..runs).map(|k| {
            let map = map.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            big_stack(move || tx.send(run(&map, k, minutes, mode)).unwrap());
            rx.recv().unwrap()
        }).collect();
        let mut fh: Vec<f32> = rs.iter().map(|r| r.first_hit.unwrap_or(f32::INFINITY)).collect();
        let mut fk: Vec<f32> = rs.iter().map(|r| r.first_kill.unwrap_or(f32::INFINITY)).collect();
        let kills: u32 = rs.iter().map(|r| r.kills).sum();
        let falls: u32 = rs.iter().map(|r| r.falls).sum();
        let mins: f32 = rs.iter().map(|r| r.minutes).sum();
        let points: f64 = rs.iter().map(|r| r.team_points).sum();
        let mut dist: Vec<f32> = rs.iter().flat_map(|r| r.hit_dist.clone()).collect();
        let hits: Vec<(f32, f32)> = rs.iter().flat_map(|r| r.hits.clone()).collect();
        let mut cells: HashMap<(i32, i32), u32> = HashMap::new();
        for (x, y) in &hits {
            *cells.entry(((x / 48.0).floor() as i32, (y / 48.0).floor() as i32)).or_default() += 1;
        }
        let n = hits.len().max(1) as f32;
        let entropy: f32 = cells.values().map(|&c| { let p = c as f32 / n; -p * p.ln() }).sum();
        let r_mean = hits.iter().map(|(x, y)| x.hypot(*y)).sum::<f32>() / n;
        let stuck: f32 = rs.iter().map(|r| r.stuck_samples).sum::<u32>() as f32 / rs.iter().map(|r| r.alive_samples).sum::<u32>().max(1) as f32;
        let mut spots: HashMap<(i32, i32), u32> = HashMap::new();
        for (x, y) in rs.iter().flat_map(|r| r.stuck_at.clone()) {
            *spots.entry(((x / 24.0).round() as i32 * 24, (y / 24.0).round() as i32 * 24)).or_default() += 1;
        }
        let mut spots: Vec<_> = spots.into_iter().collect();
        spots.sort_by_key(|s| std::cmp::Reverse(s.1));
        let spots: Vec<String> = spots.iter().take(8).map(|((x, y), n)| format!("({x},{y})x{n}")).collect();
        let per_run: Vec<String> = rs.iter().map(|r| format!("{}k/{}h", r.kills, r.hit_dist.len())).collect();
        println!(
            "SIM {map} (mode {mode}): {runs} x {minutes} min | first hit median {:.0} s, first kill median {:.0} s | kills/min {:.2} | team points/min {:.2} | hits/min {:.1} | hit dist p25/50/75 {:.0}/{:.0}/{:.0} m | fight cells (48 m) {:.1} eff., mean r {:.0} m | stuck {:.1}% | no-killer deaths {falls} | runs {}",
            pct(&mut fh, 0.5),
            pct(&mut fk, 0.5),
            kills as f32 / mins,
            points as f32 / mins,
            hits.len() as f32 / mins,
            pct(&mut dist, 0.25),
            pct(&mut dist, 0.5),
            pct(&mut dist, 0.75),
            entropy.exp(),
            r_mean,
            stuck * 100.0,
            per_run.join(" ")
        );
        println!("  {map} stuck spots (24 m): {}", spots.join(" "));
    }
}

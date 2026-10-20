//! Respawn overlap soak: a free-for-all with many bots on a map with few
//! spawn points, run headless for several minutes of sim time, counting
//! spawns on top of another tank and pairs of tanks locked together.
//! The long run is `#[ignore]`d
//! (`cargo test -p tt-engine --release --test spawn_overlap -- --ignored --nocapture`).

use glam::Vec3;
use std::collections::{HashMap, HashSet};
use tt_engine::{Config, Engine};
use tt_game::TankId;
use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, Team};

mod common;
use common::*;

#[derive(Default, Debug)]
struct Stats {
    spawns: usize,
    /// Spawns whose collision sphere overlaps a live tank's (3D) on the first tick.
    spawn_overlap_3d: usize,
    /// Spawns within the sum of the collision radii (XY) of a live tank.
    spawn_overlap_xy: usize,
    /// Pairs of live tanks whose spheres overlapped for more than 2 s on end.
    locked_pairs: usize,
    /// Longest continuous overlap of a pair, in ticks.
    longest_overlap: u32,
    /// Tank-ticks spent overlapping another live tank.
    overlap_ticks: u64,
    /// Live tanks that moved < 0.5 m in 2 s while touching another tank
    /// (live or corpse), counted once per episode.
    stuck_touching: usize,
}

fn soak(mission: &str, bots: usize, minutes: u32) -> Stats {
    let root = assets();
    let data = std::env::temp_dir().join(format!("tt-engine-test-spawn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let cfg = Config { assets: root.clone(), data: data.clone(), args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    assert!(e.boot());
    for _ in 0..30 {
        e.tick();
    }
    new_player(&mut e, "Tester");
    let names = ["Ann", "Bob", "Cid", "Dee", "Eve", "Fay", "Gus", "Hal"];
    let g = CustomGame {
        mission: mission.into(),
        mode: 0,
        bots: (0..bots).map(|i| BotSetup { name: names[i % names.len()].into(), skill: BotSkill::Gold, tank: i % 3, team: Team::Auto }).collect(),
        player_tank: 0,
        player_team: Team::Auto,
        minutes: minutes + 1,
        replace_bots: true,
    };
    apply(&mut e, MenuAction::StartCustom(g));
    let mut st = Stats::default();
    let mut known: HashSet<TankId> = HashSet::new();
    let mut pair_ticks: HashMap<(TankId, TankId), u32> = HashMap::new();
    let ticks = minutes as u64 * 60_000 / tt_engine::TICK_MS as u64;
    let mut started = false;
    let mut hist: HashMap<TankId, (Vec3, u32, bool)> = HashMap::new();
    for i in 0..ticks + 2_000 {
        e.tick();
        if i % 90 == 0 {
            // Respawn the player when dead (SPACE), like an idle human.
            e.key("space", true, None);
            e.key("space", false, None);
        }
        let Some(gs) = e.host.game.as_ref() else { continue };
        let w = &gs.world;
        if !started {
            started = w.tanks.len() > bots;
            if !started {
                continue;
            }
        }
        let live: Vec<_> = w.tanks.iter().filter(|t| !t.deleted && !t.is_corpse).collect();
        let sphere = |t: &tt_game::Tank| (t.center(), t.data.geometry.small_radius());
        for t in &live {
            if known.insert(t.id) {
                st.spawns += 1;
                let (c, r) = sphere(t);
                let (mut o3, mut oxy) = (false, false);
                for o in &live {
                    if o.id == t.id {
                        continue;
                    }
                    let (oc, or) = sphere(o);
                    o3 |= (c - oc).length() < r + or;
                    oxy |= (c - oc).truncate().length() < r + or;
                }
                st.spawn_overlap_3d += o3 as usize;
                st.spawn_overlap_xy += oxy as usize;
            }
        }
        let mut now: HashSet<(TankId, TankId)> = HashSet::new();
        for (ai, a) in live.iter().enumerate() {
            for b in &live[ai + 1..] {
                let ((ca, ra), (cb, rb)) = (sphere(a), sphere(b));
                if (ca - cb).length() < ra + rb - 0.05 {
                    let k = if a.id.0 < b.id.0 { (a.id, b.id) } else { (b.id, a.id) };
                    now.insert(k);
                    st.overlap_ticks += 2;
                    let n = pair_ticks.entry(k).or_insert(0);
                    *n += 1;
                    st.longest_overlap = st.longest_overlap.max(*n);
                    if *n == 2_000 / tt_engine::TICK_MS {
                        st.locked_pairs += 1;
                        eprintln!("locked pair {k:?} at {:?} / {:?}", ca, cb);
                    }
                }
            }
        }
        pair_ticks.retain(|k, _| now.contains(k));
        let all: Vec<_> = w.tanks.iter().filter(|t| !t.deleted && !(t.is_corpse && t.corpse_timer > t.data.fx.corpse_fade_time)).collect();
        for t in &live {
            let (c, r) = sphere(t);
            let touching = all.iter().any(|o| o.id != t.id && (c - sphere(o).0).length() < r + sphere(o).1 + 0.2);
            let h = hist.entry(t.id).or_insert((t.pos, 0, false));
            if !touching || (t.pos - h.0).length() > 0.5 {
                *h = (t.pos, 0, false);
                continue;
            }
            h.1 += 1;
            if h.1 >= 2_000 / tt_engine::TICK_MS && !h.2 {
                h.2 = true;
                st.stuck_touching += 1;
                eprintln!("stuck {:?} bot={} at {:?}", t.id, t.ai.is_some(), t.pos);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&data);
    st
}

/// Two minutes of the soak: no spawn lands inside another tank's sphere and
/// no pair stays overlapped.
#[test]
fn respawns_do_not_overlap() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let st = soak("missions/tt2_4.ron", 7, 2);
        println!("{st:?}");
        assert!(st.spawns > 20, "{st:?}");
        assert_eq!((st.spawn_overlap_3d, st.locked_pairs), (0, 0), "{st:?}");
    });
}

#[test]
#[ignore]
fn respawn_overlap_soak() {
    big_stack(|| {
        let minutes: u32 = std::env::var("TT_SOAK_MINUTES").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
        let mission = std::env::var("TT_SOAK_MISSION").unwrap_or_else(|_| "missions/tt2_4.ron".into());
        let st = soak(&mission, 7, minutes);
        println!("{mission} 7 bots {minutes} min: {st:?}");
    });
}

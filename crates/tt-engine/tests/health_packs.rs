//! (`tt_engine::gameplay::health_packs`): bot free-for-alls run headless for
//! several minutes of sim time, counting the health power-ups that are on
//! (available) every tick.
//!
//! - With the pref at 2 (the default) the count never drops below
//!   min(2, the map's cap) once the first respawn cycle is over.
//!
//! `HEALTH_LOG=1` prints the per-map numbers.

use tt_engine::{Config, Engine};
use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, Team};

mod common;
use common::*;

#[derive(Debug, Default, Clone)]
struct Stats {
    /// Weapon / health pick-up spots of the map.
    spots: usize,
    /// Ticks measured (after the warm-up).
    ticks: u64,
    min: usize,
    max: usize,
    sum: u64,
    /// Ticks with no health power-up on.
    zero_ticks: u64,
    pickups: u32,
    /// Fewest weapon-typed spots at once (one must stay a weapon).
    min_weapon_typed: usize,
    /// Ticks below the floor (`cap(min, spots)`).
    below: u64,
    /// Ticks below the floor while a weapon pick-up was on (one the rule
    /// could have turned): must stay 0.
    below_with_weapon_on: u64,
    /// Every power-up state change: (sim ms, def, on).
    changes: Vec<(i64, String, bool)>,
}

impl Stats {
    fn avg(&self) -> f64 {
        self.sum as f64 / self.ticks.max(1) as f64
    }
}

/// `minutes` of a bot free-for-all on `mission` with the pref at `min`;
/// counts from `warmup_s` on (all pick-ups start off: weapons respawn in
/// 60–90 s).
fn soak(mission: &str, min: u32, minutes: u32, warmup_s: u32, seed: u32) -> Stats {
    let root = assets();
    let data = std::env::temp_dir().join(format!("tt-engine-test-health-{}-{}-{min}-{}", mission, seed, std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let cfg = Config { assets: root, data: data.clone(), args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    e.host.shell.rng = tt_game::rng::Rng::new(seed | 1);
    e.host.virtual_clock_ms = Some(0.0);
    assert!(e.boot());
    for _ in 0..30 {
        e.tick();
    }
    new_player(&mut e, "Tester");
    e.set_var("server.min_health_packs", min.to_string().as_str());
    let names = ["Ann", "Bob", "Cid", "Dee", "Eve", "Fay"];
    let g = CustomGame {
        mission: tt_assets::layout::mission(mission),
        mode: 0,
        bots: names.iter().enumerate().map(|(i, n)| BotSetup { name: (*n).into(), skill: BotSkill::ALL[i % 3], tank: i % 3, team: Team::Auto }).collect(),
        player_tank: 0,
        player_team: Team::Auto,
        minutes: minutes + 2,
        replace_bots: true,
    };
    apply(&mut e, MenuAction::StartCustom(g));
    let mut st = Stats { min: usize::MAX, min_weapon_typed: usize::MAX, ..Default::default() };
    // Until the mission is in (the world's clock starts with it).
    let mut start = None;
    let total = minutes as u64 * 60_000 / tt_engine::TICK_MS as u64;
    let mut i = 0u64;
    let mut last_pickups = 0u32;
    let mut last_state: Vec<(String, bool)> = Vec::new();
    while i < total + 3_000 {
        e.tick();
        i += 1;
        e.host.virtual_clock_ms = Some((i * tt_engine::TICK_MS as u64) as f64);
        if i.is_multiple_of(90) {
            e.key("space", true, None);
            e.key("space", false, None);
        }
        let Some(gs) = e.host.game.as_ref() else { continue };
        let w = &gs.world;
        let pick: Vec<&tt_game::PowerUpParams> = w.triggers.iter().filter_map(|t| if let tt_game::TriggerKind::PowerUp(d) = &t.kind { Some(&**d) } else { None }).filter(|d| d.is_pickup()).collect();
        if pick.is_empty() {
            continue;
        }
        let t0 = *start.get_or_insert(w.time_ms);
        let ms = w.time_ms - t0;
        if ms > minutes as i64 * 60_000 {
            break;
        }
        let state: Vec<(String, bool)> = w.triggers.iter().filter_map(|t| if let tt_game::TriggerKind::PowerUp(d) = &t.kind { Some((d.name.clone(), t.on)) } else { None }).collect();
        if state != last_state {
            st.changes.extend(state.iter().filter(|x| !last_state.contains(x)).map(|(n, on)| (ms, n.clone(), *on)));
            last_state = state;
        }
        let n = gs.event_counts.get("PowerUpEntered").copied().unwrap_or(0) as u32;
        st.pickups += n - last_pickups.min(n);
        last_pickups = n;
        if ms < warmup_s as i64 * 1000 {
            continue;
        }
        st.spots = pick.len();
        let on_health = w.triggers.iter().filter(|t| t.on && matches!(&t.kind, tt_game::TriggerKind::PowerUp(d) if d.kind.eq_ignore_ascii_case("health"))).count();
        let weapon_typed = pick.iter().filter(|d| d.kind.eq_ignore_ascii_case("weapon")).count();
        let weapon_on = w.triggers.iter().any(|t| t.on && matches!(&t.kind, tt_game::TriggerKind::PowerUp(d) if d.kind.eq_ignore_ascii_case("weapon")));
        if on_health < cap(min as usize, pick.len()) {
            st.below += 1;
            st.below_with_weapon_on += weapon_on as u64;
        }
        st.ticks += 1;
        st.min = st.min.min(on_health);
        st.max = st.max.max(on_health);
        st.sum += on_health as u64;
        st.zero_ticks += (on_health == 0) as u64;
        st.min_weapon_typed = st.min_weapon_typed.min(weapon_typed);
    }
    let _ = std::fs::remove_dir_all(&data);
    if std::env::var_os("HEALTH_LOG").is_some() {
        eprintln!(
            "{mission} pref={min} seed={seed}: spots {} | on health avg {:.2} min {} max {} | none {:.1} % | below the floor {:.1} % (with a weapon on: {}) | pick-ups {} | weapon-typed min {}",
            st.spots,
            st.avg(),
            st.min,
            st.max,
            100.0 * st.zero_ticks as f64 / st.ticks.max(1) as f64,
            100.0 * st.below as f64 / st.ticks.max(1) as f64,
            st.below_with_weapon_on,
            st.pickups,
            st.min_weapon_typed
        );
    }
    st
}

/// The cap for `n` wanted on a map with `spots` pick-ups (keeps one weapon
/// when there are 3 or more).
fn cap(n: usize, spots: usize) -> usize {
    if spots <= 2 {
        return 0;
    }
    n.min(spots - 1)
}

/// After the first respawn cycle, health power-ups never drop below the
/// floor while there is a weapon pick-up to turn into one (only when the
/// bots have taken every weapon can a taken health pack leave a gap until
/// the next respawn: the timing stays the game's, e.g. ~20 % of the time
/// on tt3_4 with seed 7), on three maps (tt1_1, tt2_2, tt3_4), five
/// minutes each.
#[test]
fn min_health_packs_floor() {
    tt_assets::require_game_data!();
    big_stack(|| {
        for (map, seed) in [("tt1_1", 3), ("tt2_2", 5), ("tt3_4", 7)] {
            let st = soak(map, 2, 5, 125, seed);
            assert!(st.ticks > 1000, "{map}: not measured ({st:?})");
            assert!(st.pickups >= 3, "{map}: the bots took no power-ups ({st:?})");
            assert_eq!(st.below_with_weapon_on, 0, "{map}: below the floor with a weapon pick-up on: {st:?}");
            assert!(st.avg() >= cap(2, st.spots) as f64 - 0.5, "{map}: too few health packs on average: {st:?}");
            assert!(st.spots < 3 || st.min_weapon_typed >= 1, "{map}: no weapon spot left: {st:?}");
        }
    });
}

/// The before / after numbers over more maps (`--ignored --nocapture`,
/// `HEALTH_LOG=1`).
#[test]
#[ignore]
fn min_health_packs_report() {
    big_stack(|| {
        for map in ["tt1_1", "tt1_3", "tt2_2", "tt2_4", "tt3_2", "tt3_4"] {
            for min in [0, 1, 2] {
                soak(map, min, 6, 125, 7);
            }
        }
    });
}

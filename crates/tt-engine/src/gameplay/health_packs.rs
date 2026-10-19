//! The health-pack minimum on the world's power-ups.

use tt_game::{TriggerId, TriggerKind};
use tt_gameplay::server::health_floor::{self as health, Spot};

use crate::game::GameSim;

pub use health::{MAX, PREF};

/// The pref (empty: the default), clamped to 0..=3.
pub fn wanted(prefs: &tt_shell::Prefs) -> u32 {
    health::wanted(&prefs.get(PREF))
}

/// The rule for this tick (server, after the world tick, before the
/// network capture; multiplayer only: not `single_player`). `respawned`:
/// pick-ups that switched on this tick.
pub fn apply(prefs: &tt_shell::Prefs, single_player: bool, g: &mut GameSim, respawned: &[TriggerId]) {
    if !g.role.authority() || single_player {
        return;
    }
    let n = wanted(prefs);
    if n == 0 {
        return;
    }
    let spots: Vec<Spot> = g
        .world
        .triggers
        .iter()
        .filter_map(|t| {
            let TriggerKind::PowerUp(d) = &t.kind else { return None };
            if !(d.name.eq_ignore_ascii_case("TestPowerup") || d.name.eq_ignore_ascii_case("Heal")) {
                return None;
            }
            let id = g.entities.of_trigger(t.id)?.id;
            let p = t.transform.translation;
            Some(Spot { id, pos: [p.x, p.y, p.z], on: t.on, health: d.kind.eq_ignore_ascii_case("health"), respawned: respawned.contains(&t.id) })
        })
        .collect();
    for (e, db) in health::rule(n, spots) {
        g.set_def(e, db);
    }
}

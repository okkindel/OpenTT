//! A floor on the health power-ups available at once.
//!
//! A health power-up only appears when a weapon pick-up rolls `getRandom(10)
//! <= 2` and becomes `Heal` 5 s later; a taken `Heal` goes back to
//! `TestPowerup` 5 s later. The def swap keeps the object's on/off state and
//! respawn time.
//!
//! The rule, over the `TestPowerup` / `Heal` pick-ups ("spots"), every
//! server tick of a multiplayer game: while fewer than `cap(N, spots)`
//! health pick-ups are on, an available weapon pick-up becomes `Heal`. The
//! one that respawned this tick comes first; otherwise the available weapon
//! pick-up farthest from the other health spots. On larger maps `cap` keeps
//! one weapon spot (and an off `Heal` goes back to a weapon when none is
//! left).

use crate::entity::EntityId;

pub const PREF: &str = "server.min_health_packs";
pub const DEFAULT: u32 = 2;
pub const MAX: u32 = 3;

/// The pref's text (empty: the default), clamped to 0..=3.
pub fn wanted(pref: &str) -> u32 {
    if pref.trim().is_empty() {
        DEFAULT
    } else {
        tt_game::fmt::atoi(pref).clamp(0, MAX as i32) as u32
    }
}

/// How many of `n` wanted a map with `spots` pick-ups gets.
pub fn cap(n: usize, spots: usize) -> usize {
    if spots <= 2 {
        return 0;
    }
    n.min(spots - 1)
}

/// A `TestPowerup` / `Heal` pick-up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub id: EntityId,
    pub pos: [f32; 3],
    pub on: bool,
    /// Its def's type is `health`.
    pub health: bool,
    /// It switched on this tick.
    pub respawned: bool,
}

/// The def changes of this tick: (pick-up, `TestPowerup` or `Heal`),
/// in order.
pub fn rule(n: u32, mut s: Vec<Spot>) -> Vec<(EntityId, &'static str)> {
    let mut out = Vec::new();
    let target = cap(n as usize, s.len());
    if target == 0 {
        return out;
    }
    if s.len() >= 3 && s.iter().all(|p| p.health) {
        if let Some(i) = s.iter().position(|p| !p.on) {
            out.push((s[i].id, "TestPowerup"));
            s[i].health = false;
        }
    }
    loop {
        let on_health = s.iter().filter(|p| p.on && p.health).count();
        let weapons = s.iter().filter(|p| !p.health).count();
        if on_health >= target || (s.len() >= 3 && weapons < 2) {
            return out;
        }
        let health: Vec<[f32; 3]> = s.iter().filter(|p| p.health).map(|p| p.pos).collect();
        let dist = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        let far = |p: &Spot| health.iter().map(|h| dist(p.pos, *h)).fold(f32::INFINITY, f32::min);
        let pick = s
            .iter()
            .enumerate()
            .filter(|(_, p)| p.on && !p.health)
            .max_by(|(_, a), (_, b)| a.respawned.cmp(&b.respawned).then(far(a).total_cmp(&far(b))))
            .map(|(i, _)| i);
        let Some(i) = pick else { return out };
        out.push((s[i].id, "Heal"));
        s[i].health = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps() {
        assert_eq!(cap(2, 0), 0);
        assert_eq!(cap(2, 2), 0);
        assert_eq!(cap(3, 3), 2);
        assert_eq!(cap(2, 6), 2);
        assert_eq!(cap(0, 6), 0);
        assert_eq!(wanted(""), 2);
        assert_eq!(wanted("7"), 3);
    }

    #[test]
    fn respawned_spot_first_then_farthest() {
        let spot = |i: u32, x: f32, on, health, respawned| Spot { id: EntityId(i), pos: [x, 0.0, 0.0], on, health, respawned };
        let s = vec![spot(1, 0.0, true, true, false), spot(2, 10.0, true, false, false), spot(3, 50.0, true, false, false), spot(4, 5.0, true, false, true)];
        assert_eq!(rule(2, s.clone()), vec![(EntityId(4), "Heal")]);
        let mut s2 = s;
        s2[3].respawned = false;
        assert_eq!(rule(2, s2), vec![(EntityId(3), "Heal")]);
    }
}

//! Simulation time and the timed events of the game (what `schedule()` and
//! the engine's event queue were): every timer of the server and the
//! client, in one queue ordered by (due time, id) with ids from one counter,
//! so events due at the same time fire in the order they were set — the
//! order the game's behaviour depends on (docs/rust-native-plan.md risk R3).

use std::collections::{BTreeMap, HashMap};

/// A timed event.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A timer of the game server (`tt_gameplay::server::Server::fire`).
    ServerTimer,
    /// The mission lighting progress on the loading screen
    /// (`updateLightingProgress`, every millisecond while lighting).
    UpdateLightingProgress,
    /// The solo game over (`soloPlayGui.endGame`).
    SoloEndGame,
}

/// The queue and the simulation clock.
#[derive(Debug, Clone)]
pub struct Scheduler {
    now: u32,
    next_id: u32,
    events: BTreeMap<(u32, u32), Event>,
    times: HashMap<u32, u32>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler { now: 0, next_id: 1, events: BTreeMap::new(), times: HashMap::new() }
    }
}

impl Scheduler {
    /// The simulation time (ms).
    pub fn now(&self) -> u32 {
        self.now
    }

    /// `ev` at time `at` (not before now); its id.
    pub fn post_at(&mut self, at: u32, ev: Event) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let at = at.max(self.now);
        self.events.insert((at, id), ev);
        self.times.insert(id, at);
        id
    }

    /// `ev` in `delay` ms.
    pub fn post(&mut self, delay: u32, ev: Event) -> u32 {
        let at = self.now.wrapping_add(delay);
        self.post_at(at, ev)
    }

    /// Cancels event `id` (nothing if it is not pending).
    pub fn cancel(&mut self, id: u32) {
        if let Some(t) = self.times.remove(&id) {
            self.events.remove(&(t, id));
        }
    }

    pub fn is_pending(&self, id: u32) -> bool {
        self.times.contains_key(&id)
    }

    pub fn pending(&self) -> usize {
        self.events.len()
    }

    /// The next event due at or before `until`, removed, with the clock set
    /// to its time; None when there is none (the clock is left).
    pub fn pop_due(&mut self, until: u32) -> Option<(u32, Event)> {
        let (&(t, id), _) = self.events.iter().next()?;
        if t > until {
            return None;
        }
        let ev = self.events.remove(&(t, id))?;
        self.times.remove(&id);
        self.now = t;
        Some((id, ev))
    }

    /// Sets the clock (after the events up to it ran).
    pub fn set_now(&mut self, t: u32) {
        self.now = t;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_fire_in_time_then_id_order() {
        let mut s = Scheduler::default();
        let a = s.post(64, Event::ServerTimer);
        let b = s.post(32, Event::UpdateLightingProgress);
        let c = s.post(32, Event::SoloEndGame);
        s.cancel(c);
        assert!(s.is_pending(a) && !s.is_pending(c));
        let target = s.now() + 64;
        assert_eq!(s.pop_due(target), Some((b, Event::UpdateLightingProgress)));
        assert_eq!(s.now(), 32);
        // Posted while running: after the ones already due at that time.
        let d = s.post(32, Event::ServerTimer);
        assert_eq!(s.pop_due(target), Some((a, Event::ServerTimer)));
        assert_eq!(s.pop_due(target), Some((d, Event::ServerTimer)));
        assert_eq!(s.pop_due(target), None);
        s.set_now(target);
        assert_eq!(s.now(), 64);
    }
}

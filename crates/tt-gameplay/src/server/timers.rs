//! The server's timers: typed events due after a delay in simulation
//! milliseconds. The host keeps the one queue of the game ordered by due
//! time, then by the order the timers were set (server and client timers
//! alike, so events due at the same time keep their order); the server
//! keeps what each timer does by its id.

use std::collections::BTreeMap;

use super::{ClientId, ServerHost};
use crate::data::{PickupDef, Ref, SoundDef};
use crate::entity::EntityId;

/// A timer's id in the host's queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub u32);

/// What a timer does when it is due.
#[derive(Debug, Clone, PartialEq)]
pub enum Timer {
    /// 35 s before the round's end: the clients' clocks count down.
    RoundWarning,
    /// The round's time is up.
    RoundEnd,
    /// The round ends; the pause before the next one.
    EndRound,
    /// The pause is over: the next round (or mission).
    NextRound,
    /// A solo level's 5 s step (the end, the next bot).
    SoloTick,
    /// The solo level ends.
    SoloLevelEnd,
    /// Target practice is won.
    TargetPracticeDone,
    /// Bots leave or come back for the players.
    BalanceBots,
    /// The server list's player list.
    RefreshPlayerList,
    /// A dedicated server left to its bots starts its mission over.
    RestartMission,
    /// Announce the server to the master server.
    StartHeartbeat,
    /// A multiplayer mission load's second stage (the objects).
    LoadMissionObjects,
    /// A dead player's new tank.
    Respawn(ClientId),
    /// A sound for one player.
    PlaySound(ClientId, Ref<SoundDef>),
    /// A power-up becomes another.
    SwapPickup(EntityId, Ref<PickupDef>),
    /// A flood-protected client may chat again.
    Unmute(ClientId),
    /// A chat message no longer counts for the flood protection.
    ForgetMessage(ClientId),
}

impl Timer {
    /// The client the timer is for.
    pub fn client(&self) -> Option<ClientId> {
        match self {
            Timer::Respawn(c) | Timer::PlaySound(c, _) | Timer::Unmute(c) | Timer::ForgetMessage(c) => Some(*c),
            _ => None,
        }
    }

    /// The entity the timer is for.
    pub fn entity(&self) -> Option<EntityId> {
        match self {
            Timer::SwapPickup(e, _) => Some(*e),
            _ => None,
        }
    }
}

/// The pending timers.
#[derive(Debug, Clone, Default)]
pub struct Timers {
    pending: BTreeMap<TimerId, Timer>,
}

impl Timers {
    /// `t` in `delay_ms` (0: after what is due now).
    pub fn schedule(&mut self, h: &mut dyn ServerHost, delay_ms: u32, t: Timer) -> TimerId {
        let id = h.schedule(delay_ms);
        self.pending.insert(id, t);
        id
    }

    /// Cancels a timer (nothing if it is not pending).
    pub fn cancel(&mut self, h: &mut dyn ServerHost, id: TimerId) {
        self.pending.remove(&id);
        h.cancel(id);
    }

    /// Cancels every timer `gone` says goes.
    pub fn cancel_where(&mut self, h: &mut dyn ServerHost, gone: impl Fn(&Timer) -> bool) {
        let ids: Vec<TimerId> = self.pending.iter().filter(|(_, t)| gone(t)).map(|(id, _)| *id).collect();
        for id in ids {
            self.cancel(h, id);
        }
    }

    /// The timer that is due (it leaves the queue).
    pub fn take(&mut self, id: TimerId) -> Option<Timer> {
        self.pending.remove(&id)
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

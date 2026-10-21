//! The world's game entities: tanks, scrum balls and triggers (power-ups,
//! boost/jump pads, volcanoes, the scrum goal) live in [`tt_game::World`]; the
//! [`EntityTable`] gives each one its game identity — name, def, team and
//! owning client — and the [`GameEvent`]s the server reacts to are expressed
//! in [`EntityId`]s instead of script callbacks by name.
//!
//! Ids are allocated monotonically in creation order and stay unique across
//! missions (the engine carries the counter from one world to the next), so
//! the server can hold an entity id longer than the entity lives.

use std::collections::{BTreeMap, HashMap};
use tt_game::{Event, FlagId, TankId, TriggerId};

/// A game entity's id (stable for its lifetime, never reused).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub u32);

/// What the entity is in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKind {
    Tank(TankId),
    Flag(FlagId),
    /// A power-up, boost/jump pad or volcano.
    Trigger(TriggerId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    /// The object's name (`Goal`, a pad's name; empty for most).
    pub name: String,
    /// Def name (as declared; `setDef` changes it).
    pub def: String,
    /// Tank team.
    pub team: Option<i32>,
    /// The client it belongs to (a player's or bot's tank: the server's
    /// client id).
    pub owner: Option<u32>,
}

/// All entities of a mission, by id and by world id.
#[derive(Debug, Clone, Default)]
pub struct EntityTable {
    next: u32,
    entities: BTreeMap<EntityId, Entity>,
    by_kind: HashMap<EntityKind, EntityId>,
}

impl EntityTable {
    pub fn new() -> EntityTable {
        EntityTable { next: 1, ..Default::default() }
    }

    /// A table whose first id is `first`.
    pub fn starting_at(first: u32) -> EntityTable {
        EntityTable { next: first.max(1), ..Default::default() }
    }

    /// The id the next entity gets.
    pub fn next_id(&self) -> u32 {
        self.next.max(1)
    }

    /// Adds an entity (a world id already in the table is replaced).
    pub fn insert(&mut self, kind: EntityKind, name: &str, def: &str) -> EntityId {
        let id = EntityId(self.next.max(1));
        self.insert_id(id, kind, name, def);
        id
    }

    /// Adds an entity with an id given out earlier (an entity the server
    /// created before the world existed); later ids follow it.
    pub fn insert_id(&mut self, id: EntityId, kind: EntityKind, name: &str, def: &str) {
        if let Some(old) = self.by_kind.get(&kind).copied() {
            self.remove(old);
        }
        self.next = self.next.max(id.0 + 1);
        self.entities.insert(id, Entity { id, kind, name: name.to_string(), def: def.to_string(), team: None, owner: None });
        self.by_kind.insert(kind, id);
    }

    pub fn remove(&mut self, id: EntityId) -> Option<Entity> {
        let e = self.entities.remove(&id)?;
        self.by_kind.remove(&e.kind);
        Some(e)
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
    }

    /// In creation order.
    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    pub fn of_kind(&self, kind: EntityKind) -> Option<&Entity> {
        self.by_kind.get(&kind).and_then(|id| self.entities.get(id))
    }

    pub fn of_tank(&self, t: TankId) -> Option<&Entity> {
        self.of_kind(EntityKind::Tank(t))
    }

    pub fn of_flag(&self, f: FlagId) -> Option<&Entity> {
        self.of_kind(EntityKind::Flag(f))
    }

    pub fn of_trigger(&self, t: TriggerId) -> Option<&Entity> {
        self.of_kind(EntityKind::Trigger(t))
    }

    /// The world tank of an entity.
    pub fn tank(&self, e: EntityId) -> Option<TankId> {
        match self.get(e)?.kind {
            EntityKind::Tank(t) => Some(t),
            _ => None,
        }
    }

    pub fn flag(&self, e: EntityId) -> Option<FlagId> {
        match self.get(e)?.kind {
            EntityKind::Flag(f) => Some(f),
            _ => None,
        }
    }

    pub fn trigger(&self, e: EntityId) -> Option<TriggerId> {
        match self.get(e)?.kind {
            EntityKind::Trigger(t) => Some(t),
            _ => None,
        }
    }

    /// An entity by object name (any case), the first created.
    pub fn named(&self, name: &str) -> Option<EntityId> {
        self.entities.values().find(|e| !e.name.is_empty() && e.name.eq_ignore_ascii_case(name)).map(|e| e.id)
    }

    /// Number of tank entities.
    pub fn tanks(&self) -> usize {
        self.entities.values().filter(|e| matches!(e.kind, EntityKind::Tank(_))).count()
    }
}

/// What happened in the world that the game rules react to (was the
/// engine's `db.onDestroyed`/`onEnter`/`onLeave`/`onOutOfBounds` calls by
/// name and the corpse deletion). Stage 4 adds the client events
/// (`ClientConnected`, `ClientDropped`, `ClientCmd`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameEvent {
    /// The tank was destroyed (now a corpse); `killer` is the tank that
    /// dealt the last damage.
    TankDestroyed { tank: EntityId, killer: Option<EntityId> },
    /// The corpse timer ran out: the tank is gone from the world.
    TankRemoved { tank: EntityId },
    /// A tank entered / left a power-up (or pad, goal) trigger;
    /// `def` is the trigger's power-up def as the world has it.
    PowerUpEnter { pup: EntityId, tank: EntityId, def: String },
    PowerUpLeave { pup: EntityId, tank: EntityId, def: String },
    /// The scrum ball left the mission area.
    FlagOutOfBounds { flag: EntityId },
}

impl GameEvent {
    /// The game event of a world event, in the entities' terms (`None`
    /// for world events the rules do not see, or for entities not in the
    /// table).
    pub fn of_world(entities: &EntityTable, e: &Event) -> Option<GameEvent> {
        let tank = |t: &TankId| entities.of_tank(*t).map(|e| e.id);
        Some(match e {
            Event::Killed { tank: t, killer } => GameEvent::TankDestroyed { tank: tank(t)?, killer: killer.as_ref().and_then(tank) },
            Event::TankRemoved { tank: t } => GameEvent::TankRemoved { tank: tank(t)? },
            Event::PowerUpEntered { trigger, tank: t, def } => GameEvent::PowerUpEnter { pup: entities.of_trigger(*trigger)?.id, tank: tank(t)?, def: def.clone() },
            Event::PowerUpLeft { trigger, tank: t, def } => GameEvent::PowerUpLeave { pup: entities.of_trigger(*trigger)?.id, tank: tank(t)?, def: def.clone() },
            Event::FlagOutOfBounds { flag } => GameEvent::FlagOutOfBounds { flag: entities.of_flag(*flag)?.id },
            _ => return None,
        })
    }

    /// The game events of a tick's world events, in order.
    pub fn of_world_events(entities: &EntityTable, events: &[Event]) -> Vec<GameEvent> {
        events.iter().filter_map(|e| GameEvent::of_world(entities, e)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_monotonic_and_lookups_follow_removal() {
        let mut t = EntityTable::new();
        let a = t.insert(EntityKind::Tank(TankId(7)), "", "MediumTank");
        let b = t.insert(EntityKind::Trigger(TriggerId(0)), "Goal", "TestPowerup");
        assert!(a < b);
        assert_eq!(t.tank(a), Some(TankId(7)));
        assert_eq!(t.trigger(b), Some(TriggerId(0)));
        assert_eq!(t.named("goal"), Some(b));
        t.remove(a);
        assert_eq!(t.tank(a), None);
        let c = t.insert(EntityKind::Tank(TankId(8)), "", "LightTank");
        assert!(c > b);
        assert_eq!(t.iter().map(|e| e.id).collect::<Vec<_>>(), [b, c]);
        // A world id bound again replaces the old entity.
        let d = t.insert(EntityKind::Tank(TankId(8)), "", "LightTank");
        assert_eq!(t.iter().count(), 2);
        assert_eq!(t.of_tank(TankId(8)).map(|e| e.id), Some(d));
        // Ids given out earlier, and a table continuing a counter.
        t.insert_id(EntityId(40), EntityKind::Flag(FlagId(1)), "", "DefaultFlag");
        assert_eq!(t.next_id(), 41);
        assert_eq!(EntityTable::starting_at(9).insert(EntityKind::Tank(TankId(1)), "", "x"), EntityId(9));
    }

    #[test]
    fn world_events_become_game_events() {
        let mut t = EntityTable::new();
        let v = t.insert(EntityKind::Tank(TankId(1)), "", "MediumTank");
        let k = t.insert(EntityKind::Tank(TankId(2)), "", "MediumTank");
        let p = t.insert(EntityKind::Trigger(TriggerId(3)), "", "Heal");
        let f = t.insert(EntityKind::Flag(FlagId(0)), "", "DefaultFlag");
        let ev = [
            Event::DryFired { tank: TankId(1) },
            Event::Killed { tank: TankId(1), killer: Some(TankId(2)) },
            Event::Killed { tank: TankId(2), killer: Some(TankId(9)) },
            Event::PowerUpEntered { trigger: TriggerId(3), tank: TankId(2), def: "Heal".into() },
            Event::PowerUpLeft { trigger: TriggerId(4), tank: TankId(2), def: "Heal".into() },
            Event::FlagOutOfBounds { flag: FlagId(0) },
            Event::TankRemoved { tank: TankId(1) },
        ];
        assert_eq!(
            GameEvent::of_world_events(&t, &ev),
            [
                GameEvent::TankDestroyed { tank: v, killer: Some(k) },
                GameEvent::TankDestroyed { tank: k, killer: None },
                GameEvent::PowerUpEnter { pup: p, tank: k, def: "Heal".into() },
                GameEvent::FlagOutOfBounds { flag: f },
                GameEvent::TankRemoved { tank: v },
            ]
        );
    }
}

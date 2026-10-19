//! ThinkTanks gameplay simulation: a deterministic, engine-independent port of the tank,
//! projectile, pad, power-up and volcano logic of the 2003 game, following
//! `docs/tank-physics.md`.
//!
//! The engine drives it at 32 ms ticks:
//!
//! ```no_run
//! use std::sync::Arc;
//! use glam::Vec3;
//! use tt_game::{HeightField, Move, TankParams, World, fields::Fields};
//!
//! let mut world = World::new(Some(HeightField::flat(0.0)));
//! let data = Arc::new(TankParams::from_fields(&Fields::from_pairs("T", "TankDef", &[]), None));
//! let tank = world.add_tank(data.clone(), Vec3::ZERO, 0.0);
//! world.set_move(tank, Move { y: 1.0, ..Move::NULL });
//! for _ in 0..100 {
//!     for event in world.tick() {
//!         println!("{event:?}");
//!     }
//! }
//! println!("{:?}", world.tank(tank).unwrap().pos);
//! ```
//!
//! Places where the spec is silent are marked `SPEC-GAP` in the code.

pub mod ai;
pub mod data;
pub mod fields;
pub mod flag;
pub mod fmt;
pub mod geom;
pub mod lagcomp;
pub mod mission;
pub mod netmove;
pub mod obfuscation;
pub mod pads;
pub mod projectile;
pub mod reticle;
pub mod rng;
pub mod statics;
pub mod tank;
pub mod terrain;
pub mod world;

pub use data::{BoostParams, BoostMode, FlagParams, PowerUpParams, ProjectileParams, TankParams, TankFxParams, TankGeometry, TankMiscParams, TriggerShape, VolcanoParams};
pub use fields::Defs;
pub use geom::Aabb;
pub use pads::{Trigger, TriggerKind, UserThread};
pub use projectile::{HitObject, Projectile};
pub use reticle::{Reticle, ReticleParams, ReticleSprite, ReticleState};
pub use statics::{StaticObject, StaticShape};
pub use ai::{Bot, BotTarget, TankAiParams};
pub use flag::Flag;
pub use tank::{Move, Tank};
pub use terrain::HeightField;
pub use world::{Event, MissionArea, World};

/// Tick length in seconds.
pub const DT: f32 = 0.032;
/// Tick length in milliseconds.
pub const TICK_MS: i64 = 32;
/// Gravity, m/s² (z component).
pub const GRAVITY: f32 = -10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TankId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectileId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TriggerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FlagId(pub u32);

#[cfg(test)]
mod tests;

//! Power-ups: a tank driving into one (power-up weapons, health, the
//! reload arch, the scrum goal, solo brains, the Power-up Test arena's
//! pick-ups), and balls out of bounds.

use super::state::*;
use super::*;
use crate::data::*;
use crate::entity::EntityId;
use crate::msg::*;

/// The practice ball's def (a ball the mission declares with
/// `practice = 1`).
pub const PRACTICE_BALL_DATA: &str = "PracticeBallData";

/// The mission group holding the practice ball's start spot (one object
/// with a `position`).
const PRACTICE_BALL_GROUP: &str = "PracticeBall";

impl Server {
    /// Where the mission's practice ball starts (the Power-up Test, outside
    /// Scrum): the first object of its `PracticeBall` group; `None` on
    /// missions without one or without the ball's def.
    pub fn practice_ball_start(&mut self, h: &mut dyn ServerHost) -> Option<String> {
        h.data().balls.find(PRACTICE_BALL_DATA)?;
        match h.group(PRACTICE_BALL_GROUP)?.into_iter().next()? {
            super::GroupMember::Object(t) => Some(t),
            super::GroupMember::Group { .. } => None,
        }
    }

    /// Whether `flag` is a practice ball.
    pub(crate) fn is_practice_ball(&self, h: &mut dyn ServerHost, flag: EntityId) -> bool {
        h.ball_def(flag).is_some_and(|b| h.data().balls[b].practice)
    }

    /// The practice ball back to its start spot (the goal stays).
    pub(crate) fn reset_practice_ball(&mut self, h: &mut dyn ServerHost, flag: EntityId) {
        h.clear_mount(flag);
        if let Some(start) = self.practice_ball_start(h) {
            h.set_transform(flag, &start);
        }
    }

    /// A sound for the tank's player a second later (bots hear nothing).
    pub(crate) fn play_later(&mut self, h: &mut dyn ServerHost, tank: EntityId, sound: Option<Ref<SoundDef>>) {
        let (Some(c), Some(sound)) = (self.tank_client(tank), sound) else { return };
        if self.kind(c).is_some_and(|k| k.network() || k == super::ClientKind::Test) {
            self.timers.schedule(h, 1000, Timer::PlaySound(c, sound));
        }
    }

    /// The sound of that name.
    pub(crate) fn sound(h: &dyn ServerHost, name: &str) -> Option<Ref<SoundDef>> {
        h.data().sounds.find(name)
    }

    /// The power-up becomes the one named `name` in 5 s.
    pub(crate) fn swap_later(&mut self, h: &mut dyn ServerHost, pup: EntityId, name: &str) {
        if let Some(def) = h.data().pickups.find(name) {
            self.timers.schedule(h, 5000, Timer::SwapPickup(pup, def));
        }
    }

    /// The scrum ball back to a random place, with a new goal.
    pub(crate) fn reset_ball(&mut self, h: &mut dyn ServerHost, flag: EntityId) {
        h.clear_mount(flag);
        let p = self.spawn_point(h, "");
        let pos = super::respawn::vector_add(&p, "0 0 10");
        h.set_transform(flag, &pos);
        if let Some(goal) = self.place_goal(h) {
            h.set_goal(flag, goal);
        }
    }

    /// A power-up weapon at random.
    pub(crate) fn random_weapon(&mut self, h: &dyn ServerHost) -> Option<Weapon> {
        let weapons = &h.data().weapons;
        let i = self.rng.get_random_max(weapons.len() as i32 - 1);
        weapons.get(i as usize).copied()
    }

    /// `tank` entered power-up `this` (a `def`).
    pub fn pickup_entered(&mut self, h: &mut dyn ServerHost, def: Ref<PickupDef>, this: EntityId, tank: EntityId) {
        let pickup = h.data().pickups[def].clone();
        match pickup.kind {
            PickupKind::Weapon => {
                let w = self.random_weapon(h);
                if let Some(w) = w {
                    h.set_projectile(tank, w.projectile);
                }
                // Now and then the pick-up turns into a health pack.
                if (self.rng.get_random_max(10) as f64) <= 2.0 && !self.setup.single_player() {
                    self.swap_later(h, this, "Heal");
                }
                self.play_later(h, tank, w.and_then(|w| w.sound));
            }
            PickupKind::Health => {
                h.hurt(tank, -100.0);
                self.swap_later(h, this, "TestPowerup");
                let sound = Self::sound(h, "ActivateHeal");
                self.play_later(h, tank, sound);
            }
            PickupKind::Ammo => {
                // The arch's lightning (its first user animation, once).
                h.play_user_thread(this, 1, 1.0, true);
                // With its zap (`sound_off`, ReloadSound): the arch never
                // switches off, so the switch sound alone never played.
                if let Some(zap) = h.pickup_def(this).and_then(|p| h.data().pickups[p].sound_off) {
                    let t = h.transform(this);
                    self.server_play_3d(h, zap, &t);
                }
                if let Some(p) = h.tank_def(tank).and_then(|t| h.data().tanks[t].default_projectile) {
                    h.set_projectile(tank, p);
                }
                let sound = Self::sound(h, "ActivateReload");
                self.play_later(h, tank, sound);
            }
            PickupKind::Goal => {
                if self.setup.mission_type == MissionType::Scrum {
                    if let Some(flag) = h.flag_of(tank) {
                        let t = h.transform(this);
                        if let Some(snd) = Self::sound(h, "PupOffSound") {
                            self.server_play_3d(h, snd, &t);
                        }
                        self.reset_ball(h, flag);
                        let client = self.tank_client(tank);
                        let name = self.rec(client, |r| r.name().to_string());
                        let text = if self.setup.team_game {
                            let team = client.and_then(|c| self.team_of(c));
                            format!("{name} scored a goal for the {} team!", self.team_short(team))
                        } else {
                            format!("{name} scored a goal!")
                        };
                        self.bottom_print_all(h, &text, "4", "2");
                        self.score_client(h, client, 1.0, 1.0);
                    }
                } else if let Some(flag) = h.flag_of(tank).filter(|&f| self.is_practice_ball(h, f)) {
                    // A practice goal (battle modes): the effect and the ball back
                    // to its spot; no score.
                    let t = h.transform(this);
                    if let Some(snd) = Self::sound(h, "PupOffSound") {
                        self.server_play_3d(h, snd, &t);
                    }
                    self.reset_practice_ball(h, flag);
                    let client = self.tank_client(tank);
                    self.center_print(h, client, &crate::msg::Value::str("Practice goal!"), "2", "1");
                }
            }
            PickupKind::Brain => {
                if self.setup.single_player() {
                    let local = self.local();
                    let player = local.and_then(|l| self.clients[&l].player);
                    if player == Some(tank) && h.entity_exists(tank) {
                        let client = self.tank_client(tank);
                        let start = self.solo_score(client);
                        let score = start + self.campaign.level().brain_points as f64;
                        self.set_solo_score(h, client, score);
                        self.with_record(local, |r| r.brains = Some(r.brains.unwrap_or(0) + 1));
                        let brains = self.rec(local, |r| r.brains.map(|b| b.to_string()).unwrap_or_default());
                        h.hud(client, &HudMsg::BrainsCount(brains));
                        let now = h.sim_time() as i32 as i64;
                        self.with_record(local, |r| r.last_kill_time = Some(now));
                        self.free_lives(h, client, start, score);
                    }
                }
            }
            PickupKind::TestCartridge => {
                if let Some(p) = pickup.projectile {
                    h.set_projectile(tank, p);
                }
                self.play_later(h, tank, pickup.pickup_sound);
            }
            PickupKind::TestRandom => {
                let w = self.random_weapon(h);
                if let Some(w) = w {
                    h.set_projectile(tank, w.projectile);
                }
                self.play_later(h, tank, w.and_then(|w| w.sound));
            }
            PickupKind::TestHeal => {
                h.hurt(tank, -100.0);
                let sound = Self::sound(h, "ActivateHeal");
                self.play_later(h, tank, sound);
            }
            PickupKind::None => {}
        }
    }

    /// A ball fell out of the world: back to a start.
    pub fn ball_out_of_bounds(&mut self, h: &mut dyn ServerHost, flag: EntityId) {
        if self.is_practice_ball(h, flag) {
            self.reset_practice_ball(h, flag);
            return;
        }
        self.reset_ball(h, flag);
    }
}

//! Client-side prediction, reconciliation and clock sync.
//!
//! The client steps its own tank at once with exactly the move the server
//! will apply (the move as it decodes from the wire). Each snapshot carries
//! the tank's exact state after the newest move the server applied
//! (`ack_move`); the client overwrites its tank with it and re-runs the
//! moves sent since. A correction is shown as a visual offset that decays
//! with τ = 100 ms (snapped beyond 2 m); the simulation never sees it.
//!
//! Clock sync: the server reports its move queue depth; the client produces
//! 1 ± 5 % moves per tick (a phase accumulator: now and then 0 or 2) to hold
//! the depth at 1 (2 on jittery links), so its moves arrive just in time.

use std::collections::VecDeque;

use glam::{Quat, Vec3};
use tt_game::{Move, TankId};
use tt_net::bits::{BitReader, BitWriter};
use tt_net::input::MoveCodec;

use std::collections::HashMap;

use super::snapshot::{apply_owner, owner_words, NetMove, OWNER_CURRENT, OWNER_SAVED};
use crate::game::GameSim;

/// Moves kept for replay (4 s).
const MAX_MOVES: usize = 128;
/// Replays longer than this are cut (§2.2).
const MAX_REPLAY: usize = 32;
/// Visual error decay per 32 ms tick: exp(−32/100).
const DECAY: f32 = 0.726_149;
/// Beyond this the correction snaps.
const SNAP: f32 = 2.0;

/// Prediction statistics (`--net-stats`, tests).
#[derive(Debug, Clone, Default)]
pub struct PredictStats {
    /// Owner states compared with the prediction of the same move.
    pub compared: u64,
    /// ... that differed in any bit.
    pub mispredicted: u64,
    /// Position error of each misprediction (m).
    pub errors: Vec<f32>,
    pub snaps: u64,
    /// Moves produced (with clock-sync dilation) and ticks.
    pub moves: u64,
    pub ticks: u64,
    /// Server queue depth samples.
    pub depth: Vec<i32>,
    /// Owner-state word indices that differed (per misprediction, first one).
    pub fields: std::collections::BTreeMap<usize, u64>,
    /// Own shots: shown at once, confirmed by the server's spawn, never
    /// confirmed (faded), server spawns of ours that were not predicted.
    pub shots_predicted: u64,
    pub shots_confirmed: u64,
    pub shots_unconfirmed: u64,
    pub shots_unpredicted: u64,
    /// Distance between a confirmed shot's server spawn and our copy's
    /// first-tick position (m), max; and how many were within 5 cm (the
    /// rest exploded in their first tick: the server spawn is the hit point).
    pub shot_error_max: f32,
    pub shots_exact: u64,
}

/// A predicted own shot: our projectile, its first-tick position, age in ticks.
struct Shot {
    pid: tt_game::ProjectileId,
    pos1: Vec3,
    age: u32,
}

/// Predicted shots not confirmed within this many ticks (≈ RTT + margin) fade.
const SHOT_TIMEOUT: u32 = 40;

#[derive(Default)]
pub struct Predict {
    /// (seq, move as the server decodes it) not yet acked.
    moves: VecDeque<(u32, Move)>,
    /// Predicted owner state after each move.
    predicted: VecDeque<(u32, Vec<u32>)>,
    /// Newest owner state: (ack_move, server tank id, words, server time of move `ack_move`).
    pending: Option<(u32, u32, Vec<u32>, i64)>,
    /// Server time (ms) the next predicted move will run at (§2.5: client
    /// tick ≈ server tick); the firing logic compares it with `last_fire`.
    clock: i64,
    /// Predicted own shots by the tank's `fire_count`: one projectile, or a
    /// scatter shot's pellets (the server spawns share the number).
    shots: HashMap<u32, Vec<Shot>>,
    /// Moves produced this tick, to step.
    new_moves: Vec<(u32, Move)>,
    /// The tank being predicted: (server id, our id); None until its first owner state.
    pub tank: Option<(u32, TankId)>,
    /// Visual correction (render only).
    pub offset: (Vec3, Quat),
    phase: f64,
    depth_avg: f64,
    pub stats: PredictStats,
}

/// Owner-state words the client cannot predict by design: damage and the
/// corpse timer (server-only health).
fn server_only(i: usize) -> bool {
    matches!(i, 33 | 34)
}

/// Owner states equal in everything but server-only health (word 37 bit 2
/// is death). Firing state counts: own shots are predicted.
fn same_motion(a: &[u32], b: &[u32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).enumerate().all(|(i, (x, y))| server_only(i) || if i == 37 { (x & !4) == (y & !4) } else { x == y })
}

/// What the server will apply for `raw` (encode + decode of the wire move).
pub fn wire_move(raw: &Move) -> Move {
    let mut w = BitWriter::new();
    NetMove(*raw).encode(&mut w);
    let b = w.finish();
    NetMove::decode(&mut BitReader::new(&b)).map(|m| m.0).unwrap_or(Move::NULL)
}

impl Predict {
    /// A snapshot's owner state and queue depth.
    pub fn owner_state(&mut self, ack: u32, net: u32, words: Vec<u32>, depth: i32, server_ms: i64) {
        self.pending = Some((ack, net, words, server_ms));
        self.depth_avg = 0.9 * self.depth_avg + 0.1 * depth as f64;
        self.stats.depth.push(depth);
        if self.stats.depth.len() > 100_000 {
            self.stats.depth.drain(..50_000);
        }
    }

    /// How many moves to produce this tick (clock sync).
    pub fn debug_size(&self) -> usize {
        self.moves.len() + self.predicted.len() + self.shots.values().map(Vec::len).sum::<usize>()
    }

    pub fn moves_this_tick(&mut self, jittery: bool) -> usize {
        let target = if jittery { 2.0 } else { 1.0 };
        let adj = ((target - self.depth_avg) * 0.02).clamp(-0.05, 0.05);
        self.phase += 1.0 + adj;
        let n = self.phase.floor().max(0.0);
        self.phase -= n;
        self.stats.ticks += 1;
        n as usize
    }

    /// A move was sent (sequence `seq`).
    pub fn sent(&mut self, seq: u32, raw: &Move) {
        let m = wire_move(raw);
        self.moves.push_back((seq, m));
        while self.moves.len() > MAX_MOVES {
            self.moves.pop_front();
        }
        self.new_moves.push((seq, m));
        self.stats.moves += 1;
    }

    /// Inside the client's `game::tick` (after the previous poses are
    /// saved): reconcile with the newest owner state, then step this tick's
    /// moves. `own` = (server id, our id) of the control tank.
    /// The server spawned our shot `fire` at `pos` (after its first tick):
    /// returns our predicted copy, now confirmed.
    pub fn confirm_shot(&mut self, fire: u32, pos: Vec3) -> Option<tt_game::ProjectileId> {
        let pellets = self.shots.get_mut(&fire)?;
        // A scatter shot's pellets: the one that should be nearest.
        let i = (0..pellets.len()).min_by(|&a, &b| (pellets[a].pos1 - pos).length_squared().total_cmp(&(pellets[b].pos1 - pos).length_squared()))?;
        let s = pellets.swap_remove(i);
        if pellets.is_empty() {
            self.shots.remove(&fire);
        }
        self.stats.shots_confirmed += 1;
        let e = (s.pos1 - pos).length();
        self.stats.shot_error_max = self.stats.shot_error_max.max(e);
        if e < 0.05 {
            self.stats.shots_exact += 1;
        }
        Some(s.pid)
    }

    /// One predicted step of our tank at the predicted server time; this
    /// tick's (not replayed) steps keep their shots and effects.
    fn step(&mut self, g: &mut GameSim, tid: TankId, m: Move, replay: bool) -> Vec<tt_game::Event> {
        self.clock += tt_game::TICK_MS;
        let keep = g.world.time_ms;
        g.world.time_ms = self.clock;
        g.world.replay = replay;
        let events = g.world.step_tank(tid, m);
        g.world.replay = false;
        g.world.time_ms = keep;
        events
    }

    pub fn tick(&mut self, g: &mut GameSim, own: Option<(u32, TankId)>, map_trigger: impl Fn(u32) -> Option<tt_game::TriggerId>, map_flag: impl Fn(u32) -> Option<tt_game::FlagId>, names: &HashMap<u32, String>) {
        let new_moves = std::mem::take(&mut self.new_moves);
        // Shots the server never confirmed fade (misprediction, e.g. no ammo left).
        let mut faded = Vec::new();
        self.shots.retain(|_, pellets| {
            pellets.retain_mut(|s| {
                s.age += 1;
                let keep = s.age <= SHOT_TIMEOUT;
                if !keep {
                    faded.push(s.pid);
                }
                keep
            });
            !pellets.is_empty()
        });
        for pid in faded {
            self.stats.shots_unconfirmed += 1;
            if let Some(p) = g.world.projectiles.iter_mut().find(|p| p.id == pid) {
                p.deleted = true;
            }
        }
        let Some((net, tid)) = own else {
            self.tank = None;
            return;
        };
        if self.tank.is_some_and(|t| t != (net, tid)) {
            // A new tank (respawn): start over from its first owner state.
            self.tank = None;
            self.predicted.clear();
            self.shots.clear();
            self.offset = (Vec3::ZERO, Quat::IDENTITY);
            g.render_offset.remove(&tid);
        }
        // Decay the visual correction.
        self.offset.0 *= DECAY;
        self.offset.1 = Quat::IDENTITY.slerp(self.offset.1, DECAY);
        if let Some((ack, _, words, server_ms)) = self.pending.take().filter(|p| p.1 == net) {
            let proj = |g: &mut GameSim, h: u32| names.get(&h).cloned().and_then(|n| g.projectile(&n));
            let (current, saved) = (words.get(OWNER_CURRENT).and_then(|h| proj(g, *h)), words.get(OWNER_SAVED).and_then(|h| proj(g, *h)));
            let Some(t) = g.world.tank_mut(tid) else { return };
            let (p_old, r_old) = (t.pos, Quat::from_mat3(&t.rot));
            // Compare with what we predicted for the same move.
            while self.predicted.front().is_some_and(|(s, _)| *s < ack) {
                self.predicted.pop_front();
            }
            if self.tank.is_some() {
                if let Some((s, w)) = self.predicted.front() {
                    if *s == ack {
                        self.stats.compared += 1;
                        if !same_motion(w, &words) {
                            self.stats.mispredicted += 1;
                            if let Some(i) = (0..w.len().min(words.len())).find(|&i| !server_only(i) && w[i] != words[i]) {
                                *self.stats.fields.entry(i).or_default() += 1;
                                if std::env::var_os("TT_NET_DEBUG").is_some() {
                                    eprintln!("[predict] ack {ack}: word {i}: predicted {:#x} server {:#x}; replayed {} moves", w[i], words[i], self.moves.len());
                                }
                            }
                            let pos = |w: &[u32]| Vec3::new(f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]));
                            self.stats.errors.push((pos(w) - pos(&words)).length());
                        }
                    }
                }
            }
            self.predicted.clear();
            while self.moves.front().is_some_and(|(s, _)| *s <= ack) {
                self.moves.pop_front();
            }
            apply_owner(t, &words, &map_trigger, &map_flag, current, saved);
            self.clock = server_ms;
            let corpse = t.is_corpse;
            let first = self.tank.is_none();
            self.tank = Some((net, tid));
            if !corpse {
                // Re-run the moves the server has not applied yet (this
                // tick's are stepped below).
                let replay: Vec<(u32, Move)> = self.moves.iter().filter(|(s, _)| !new_moves.iter().any(|(n, _)| n == s)).copied().collect();
                let skip = replay.len().saturating_sub(MAX_REPLAY);
                for (seq, m) in replay.into_iter().skip(skip) {
                    self.step(g, tid, m, true);
                    if let Some(t) = g.world.tank(tid) {
                        self.predicted.push_back((seq, owner_words(t)));
                    }
                }
            }
            if let Some(t) = g.world.tank(tid) {
                let (p_new, r_new) = (t.pos, Quat::from_mat3(&t.rot));
                let e = p_old - p_new;
                if first || corpse || e.length() > SNAP {
                    if !first && e.length() > SNAP {
                        self.stats.snaps += 1;
                    }
                    self.offset = (Vec3::ZERO, Quat::IDENTITY);
                } else {
                    self.offset.0 += e;
                    self.offset.1 = (self.offset.1 * r_old * r_new.inverse()).normalize();
                }
                // Drawn from the reconciled pose, plus the offset.
                g.prev_pose.insert(tid, (p_new, t.rot));
            }
        }
        if self.tank.is_none() {
            return;
        }
        if g.world.tank(tid).is_some_and(|t| !t.is_corpse) {
            for (seq, m) in new_moves {
                let before = g.world.projectiles.len();
                let events = self.step(g, tid, m, false);
                if let Some(t) = g.world.tank(tid) {
                    self.predicted.push_back((seq, owner_words(t)));
                    let fire = t.fire_count;
                    for e in events {
                        // Our shot, shown now; matched later by the server's spawn.
                        if let tt_game::Event::Fired { .. } = &e {
                            // Every projectile of the shot (one, or a
                            // scatter shot's pellets) spawned by this step.
                            let pellets: Vec<Shot> = g.world.projectiles[before.min(g.world.projectiles.len())..]
                                .iter()
                                .filter(|p| p.source == Some(tid) && p.cosmetic_since.is_none())
                                .map(|p| Shot { pid: p.id, pos1: p.pos + p.vel * tt_game::DT, age: 0 })
                                .collect();
                            self.stats.shots_predicted += pellets.len() as u64;
                            if !pellets.is_empty() {
                                self.shots.insert(fire, pellets);
                            }
                        }
                        g.world.push_event(e);
                    }
                }
            }
        }
        while self.predicted.len() > MAX_MOVES {
            self.predicted.pop_front();
        }
        if self.offset.0.length() < 1e-4 && self.offset.1.angle_between(Quat::IDENTITY) < 1e-4 {
            g.render_offset.remove(&tid);
        } else {
            g.render_offset.insert(tid, self.offset);
        }
    }

    /// The control tank is predicted (not interpolated).
    pub fn predicting(&self, net: u32) -> bool {
        self.tank.is_some_and(|(n, _)| n == net)
    }
}

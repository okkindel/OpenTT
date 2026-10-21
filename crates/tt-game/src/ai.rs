//! Bots have no path finding: they steer through a local potential field towards a
//! destination that their mode and tactic choose.

use std::f32::consts::PI;
use std::sync::Arc;

use glam::{Mat3, Vec3};

use crate::fields::Fields;
use crate::geom::Aabb;
use crate::tank::{Move, Tank};
use crate::world::World;
use crate::{FlagId, TankId, TriggerId};

const DEG: f32 = PI / 180.0;

#[derive(Debug, Clone, PartialEq)]
pub struct TankAiParams {
    pub name: String,
    pub home_range: f32,
    pub target_range: f32,
    pub disengage_range: f32,
    /// Aim error amplitude, degrees.
    pub aim: f32,
    pub aim_speed: f32,
    /// Fire only within this many degrees of the computed aim.
    pub trigger_happy: f32,
    pub long_shot_scale: f32,
    pub flee_damage: f32,
    /// Milliseconds.
    pub flee_on_time: i32,
    pub flee_off_time: i32,
    pub scan_delay: i32,
    pub follow: bool,
    pub snipe_weight: f32,
    pub ram_weight: f32,
    pub flank_weight: f32,
    pub back_weight: f32,
    pub oc_weight: f32,
    pub revenge_weight: f32,
}

impl Default for TankAiParams {
    fn default() -> Self {
        TankAiParams::from_fields(&Fields::from_pairs("", "TankAiDef", &[]))
    }
}

impl TankAiParams {
    pub fn from_fields(f: &Fields) -> TankAiParams {
        TankAiParams {
            name: f.name.clone(),
            home_range: f.f32_or("home_range", 1000.0),
            target_range: f.f32_or("target_range", 200.0),
            disengage_range: f.f32_or("disengage_range", 400.0),
            aim: f.f32_or("aim", 0.5),
            aim_speed: f.f32_or("aim_speed", 0.5),
            trigger_happy: f.f32_or("trigger_happy", 0.8),
            long_shot_scale: f.f32_or("long_shot_scale", 4.0),
            flee_damage: f.f32_or("flee_damage", 0.5),
            flee_on_time: f.i32_or("flee_on_time", 8000),
            flee_off_time: f.i32_or("flee_off_time", 10000),
            scan_delay: f.i32_or("scan_delay", 1500),
            follow: f.bool_or("follow", true),
            snipe_weight: f.f32_or("snipe_weight", 1.0),
            ram_weight: f.f32_or("ram_weight", 1.0),
            flank_weight: f.f32_or("flank_weight", 1.0),
            back_weight: f.f32_or("back_weight", 1.0),
            oc_weight: f.f32_or("oc_weight", 2.0),
            revenge_weight: f.f32_or("revenge_weight", 1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Attack = 0,
    Flee = 1,
    Roam = 2,
    RoamReset = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tactic {
    Snipe = 0,
    Ram = 1,
    FlankLeft = 2,
    FlankRight = 3,
    BackOff = 4,
    Choose = 5,
}

const TACTICS: [Tactic; 5] = [Tactic::Snipe, Tactic::Ram, Tactic::FlankLeft, Tactic::FlankRight, Tactic::BackOff];

/// What a bot is after: a tank, or in scrum the goal or the free ball.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotTarget {
    Tank(TankId),
    Goal(TriggerId),
    Flag(FlagId),
}

/// Per-bot state.
#[derive(Debug, Clone, PartialEq)]
pub struct Bot {
    pub data: Arc<TankAiParams>,
    /// Position at `setAI`.
    pub home: Vec3,
    pub mode: Mode,
    pub tactic: Tactic,
    pub tactic_time: i64,
    pub ticks_in_tactic: u32,
    pub ticks_on_target: u32,
    pub los_countdown: i32,
    pub unstuck: i32,
    pub clear_shot: bool,
    /// Set by the collision code when the swept move made no progress.
    pub bumped: bool,
    /// Recent damaging hits (decays ×0.9885 per tick).
    pub hits: f32,
    pub streak: f32,
    pub stuck: f32,
    pub flee_deadline: i64,
    pub next_scan: i64,
    pub dest: Vec3,
    pub target: Option<BotTarget>,
    /// Aim wobble random walks.
    pub wobble: [f32; 2],
}

/// What the AI needs to know about a target object.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TargetInfo {
    pub pos: Vec3,
    pub center: Vec3,
    pub vel: Vec3,
    pub rot: Mat3,
    pub sphere_radius: f32,
    /// Tank id, team and corpse flag when the target is a tank.
    pub tank: Option<(TankId, i32, bool)>,
}

fn within(a: Vec3, b: Vec3, d: f32) -> bool {
    (a - b).length_squared() < d * d
}

/// Reflected random walk, step ±`step`, mirrored at ±1, pulled back by
/// `pull` beyond ±`soft`.
pub fn wobble_step(v: &mut f32, step: f32, soft: f32, pull: f32, rng: &mut crate::rng::Rng) {
    let mut x = (step + step) * rng.rand_f() - step + *v;
    if x > 1.0 {
        x = 2.0 - x;
    } else if x < -1.0 {
        x = -2.0 - x;
    }
    if x > soft {
        x -= pull;
    } else if x < -soft {
        x += pull;
    }
    *v = x;
}

/// Ballistic lead, in the bot's body frame: four iterations of `t = |aim|/v; n =
/// floor(t·31.25)+1; aim = rel + relV·t + (0,0,g·0.01024·n(n+1)/2)`. Returns the unit aim
/// direction, or zero.
pub fn lead_aim(rel: Vec3, rel_v: Vec3, velocity: f32, gravity_scale: f32) -> Vec3 {
    if rel.length_squared() < 0.1 || velocity == 0.0 {
        return Vec3::ZERO;
    }
    let inv_v = 1.0 / velocity;
    let mut aim = rel;
    for _ in 0..4 {
        let t = aim.normalize_or_zero().dot(aim) * inv_v;
        let n = ((t * 31.25) as f64).floor() as i64 + 1;
        let ticks = ((n + 1) * n) >> 1;
        if ticks < 0 {
            return Vec3::ZERO;
        }
        aim = rel + rel_v * t + Vec3::new(0.0, 0.0, gravity_scale * 0.01024 * ticks as f32);
    }
    if aim.length_squared() < 0.01 {
        Vec3::ZERO
    } else {
        aim.normalize()
    }
}

/// Aim direction (body frame) → (yaw, pitch) in turret angle units:
/// yaw = atan2(x, y), pitch = −asin(z) (+down).
pub fn aim_angles(aim: Vec3) -> (f32, f32) {
    if aim.length_squared() < 0.9 {
        return (0.0, 0.0);
    }
    if aim.z.abs() > 0.99 {
        return (0.0, -aim.z.clamp(-1.0, 1.0).asin());
    }
    (aim.x.atan2(aim.y), -aim.z.clamp(-1.0, 1.0).asin())
}

impl World {
    /// `setAI(def)` / `setAI(0)`.
    pub fn set_ai(&mut self, id: crate::TankId, data: Option<Arc<TankAiParams>>) {
        let Some(i) = self.tanks.iter().position(|t| t.id == id && !t.deleted) else { return };
        match data {
            None => self.tanks[i].ai = None,
            Some(d) => {
                if self.tanks[i].ai.as_ref().is_some_and(|b| Arc::ptr_eq(&b.data, &d) || *b.data == *d) {
                    return;
                }
                let home = self.tanks[i].pos;
                let w0 = self.rng.rand_f();
                let w1 = self.rng.rand_f();
                self.tanks[i].ai = Some(Box::new(Bot {
                    data: d,
                    home,
                    mode: Mode::RoamReset,
                    tactic: Tactic::Choose,
                    tactic_time: 0,
                    ticks_in_tactic: 0,
                    ticks_on_target: 0,
                    los_countdown: 15,
                    unstuck: 0,
                    clear_shot: true,
                    bumped: false,
                    hits: 0.0,
                    streak: 0.0,
                    stuck: 0.0,
                    flee_deadline: 0,
                    next_scan: 0,
                    dest: home,
                    target: None,
                    wobble: [w0, w1],
                }));
            }
        }
    }

    /// `a` and `b` are enemies.
    pub fn enemies(&self, a_team: i32, b_team: i32) -> bool {
        a_team != b_team || (!self.team_game && b_team == 0)
    }

    pub(crate) fn target_info(&self, t: BotTarget) -> Option<TargetInfo> {
        match t {
            BotTarget::Tank(id) => {
                let o = self.tank(id)?;
                let wb = o.world_box();
                Some(TargetInfo {
                    pos: o.pos,
                    center: wb.center(),
                    vel: o.vel,
                    rot: o.rot,
                    sphere_radius: 0.5 * (wb.max - wb.min).length(),
                    tank: Some((o.id, o.team, o.is_corpse)),
                })
            }
            BotTarget::Goal(id) => {
                let tr = self.trigger(id)?;
                let wb = tr.shape.bounds.transformed(&tr.transform);
                Some(TargetInfo {
                    pos: tr.transform.translation.into(),
                    center: wb.center(),
                    vel: Vec3::ZERO,
                    rot: tr.transform.matrix3.into(),
                    sphere_radius: 0.5 * (wb.max - wb.min).length(),
                    tank: None,
                })
            }
            BotTarget::Flag(id) => {
                let f = self.flag(id)?;
                let wb = f.world_box();
                Some(TargetInfo { pos: f.pos, center: wb.center(), vel: f.vel, rot: Mat3::IDENTITY, sphere_radius: 0.5 * (wb.max - wb.min).length(), tank: None })
            }
        }
    }

    /// Position of a bot's current target (for the fire-rate scaling).
    pub fn bot_target_pos(&self, t: &Tank) -> Option<Vec3> {
        let target = t.ai.as_ref()?.target?;
        self.target_info(target).map(|i| i.pos)
    }

    /// The ball the bots play scrum with (a practice ball is left alone).
    fn scrum_ball(&self) -> Option<&crate::flag::Flag> {
        self.flags.first().filter(|f| !f.data.practice)
    }

    /// Target choice.
    pub(crate) fn pick_target(&self, t: &Tank) -> Option<BotTarget> {
        let bot = t.ai.as_ref()?;
        let d = &bot.data;
        if !d.follow && !within(bot.home, t.pos, d.home_range) {
            return None;
        }
        let score_mods = |cand: &Tank, mut score: f32| {
            if bot.target == Some(BotTarget::Tank(cand.id)) {
                score *= d.oc_weight;
            }
            if t.last_damage_source == Some(cand.id) {
                score *= d.revenge_weight * bot.streak + 1.0;
            }
            score
        };
        let candidates = || self.tanks.iter().filter(|c| !c.deleted && c.id != t.id && self.enemies(t.team, c.team) && !c.is_corpse);
        if let Some(flag) = self.scrum_ball() {
            let Some(carrier_id) = flag.carrier else { return Some(BotTarget::Flag(flag.id)) };
            if carrier_id == t.id {
                return flag.goal.map(BotTarget::Goal);
            }
            let carrier = self.tank(carrier_id)?;
            if self.enemies(t.team, carrier.team) {
                return Some(BotTarget::Tank(carrier_id));
            }
            let mut best: Option<(f32, TankId)> = None;
            for c in candidates() {
                let dm = (c.pos - t.pos).length_squared();
                let dc = (c.pos - carrier.pos).length_squared();
                let v = 2.25e6 - (dm * 0.2 + dc * 0.8);
                let score = score_mods(c, if v > 0.0 { v.sqrt() } else { 0.0 });
                if score > best.map_or(0.0, |b| b.0) {
                    best = Some((score, c.id));
                }
            }
            return best.map(|b| BotTarget::Tank(b.1));
        }
        let (tr, dr) = (d.target_range, d.disengage_range);
        let mut best: Option<(f32, TankId)> = None;
        for c in candidates() {
            let d2 = (c.pos - t.pos).length_squared();
            if d2 <= dr * dr && (d2 <= tr * tr || bot.target == Some(BotTarget::Tank(c.id))) {
                let v = dr * dr - d2;
                let score = score_mods(c, if v > 0.0 { v.sqrt() } else { 0.0 });
                if score > best.map_or(0.0, |b| b.0) {
                    best = Some((score, c.id));
                }
            }
        }
        best.map(|b| BotTarget::Tank(b.1))
    }

    /// `randomSpawnPoint`, including its in-place shuffling of the global
    /// spawn point list.
    pub(crate) fn random_spawn_point(&mut self, t: &Tank) -> Vec3 {
        let bot = t.ai.as_ref().expect("bot");
        let r2 = bot.data.home_range * bot.data.home_range;
        let n = self.spawn_spheres.len();
        let mut count = 0usize;
        for i in 0..n {
            if (self.spawn_spheres[i] - t.pos).length_squared() <= r2 {
                count += 1;
            } else {
                self.spawn_spheres.swap(i, n - 1);
            }
        }
        if count == 0 {
            return bot.home;
        }
        let mut r = self.rng.rand_i();
        if count - 1 < 0x10000 {
            r >>= 16;
        }
        self.spawn_spheres[r as usize % count]
    }

    pub(crate) fn set_mode(&mut self, t: &mut Tank, m: Mode) {
        let now = self.time_ms;
        let bot = t.ai.as_mut().expect("bot");
        if bot.mode == Mode::Attack && m != Mode::Attack {
            bot.target = None;
        }
        match m {
            Mode::Attack => {
                if bot.mode != Mode::Attack {
                    bot.flee_deadline = now + bot.data.flee_off_time as i64;
                    bot.tactic = Tactic::Choose;
                    bot.clear_shot = true;
                    bot.los_countdown = 0;
                }
            }
            Mode::Flee => {
                let dest = self.random_spawn_point(t);
                let bot = t.ai.as_mut().expect("bot");
                bot.dest = dest;
                bot.flee_deadline = now + bot.data.flee_on_time as i64;
            }
            Mode::Roam => {
                let dest = self.random_spawn_point(t);
                t.ai.as_mut().expect("bot").dest = dest;
            }
            Mode::RoamReset => {}
        }
        t.ai.as_mut().expect("bot").mode = m;
    }

    /// Potential-field steering.
    pub(crate) fn steer(&self, t: &Tank, dest: Vec3, direct: bool) -> Vec3 {
        let bot = t.ai.as_ref().expect("bot");
        let rel = dest - t.pos;
        if rel.length_squared() < 0.1 {
            return rel;
        }
        let mut desired = rel.normalize();
        if direct {
            return desired;
        }
        let b = Aabb::from_points([t.pos, dest]).grow(10.0);
        let r_me = t.data.geometry.big_radius();
        let w = bot.stuck + 1.0;
        let mut repulse = Vec3::ZERO;
        for s in &self.statics {
            if !s.world_box.overlaps(&b) {
                continue;
            }
            let c = s.world_box.center();
            let half = s.shape.bounds.half_extents();
            let rel = t.pos - c;
            let m = s.transform.matrix3;
            let mut p = 0.0;
            for (axis, h) in [(Vec3::from(m.x_axis), half.x), (Vec3::from(m.y_axis), half.y), (Vec3::from(m.z_axis), half.z)] {
                let e = rel.dot(axis).abs() - h - r_me;
                if e > 0.0 {
                    p += e;
                }
            }
            let k = w / (p + 0.5);
            if rel.length_squared() > 0.1 {
                repulse += rel.normalize() * k;
            }
        }
        for o in &self.tanks {
            if o.deleted || o.id == t.id || !o.world_box().overlaps(&b) {
                continue;
            }
            let rel = t.pos - o.world_box().center();
            let rs = o.data.geometry.big_radius() + r_me;
            let d2 = rel.length_squared();
            if rs * rs < d2 {
                let d = d2.sqrt();
                let k = w / ((d + 1.0) - rs);
                repulse += rel * (k / d);
            }
        }
        for tr in &self.triggers {
            if !tr.on || !tr.power_up_params().ai_pickup || !tr.shape.bounds.transformed(&tr.transform).overlaps(&b) {
                continue;
            }
            let rel = Vec3::from(tr.transform.translation) - t.pos;
            let d = rel.length();
            if d < 50.0 {
                if d < 10.0 {
                    if d > 0.0 {
                        desired = rel / d;
                    }
                } else {
                    repulse += rel * 20.0;
                }
            }
        }
        let sum = repulse + desired;
        if sum.length_squared() > 0.01 {
            sum.normalize()
        } else {
            desired
        }
    }

    // Whether the bot changes its tactic: 70 % when hits > 3; snipe: no clear
    // shot, or hits > 1.5 and 70 %, or the target closing faster than 5 m/s
    // and 35 %; ram 20 %; flank when the target's velocity towards us is
    // ≤ 5 m/s; back off when d² ≤ 2500 and stuck ≤ 0.5.
    fn should_change(&mut self, bot: &Bot, d2: f32, target_vel: f32) -> bool {
        if bot.hits > 3.0 && self.rng.rand_f() < 0.7 {
            return true;
        }
        match bot.tactic {
            Tactic::Choose => true,
            Tactic::Snipe => {
                if !bot.clear_shot || (bot.hits > 1.5 && self.rng.rand_f() < 0.7) {
                    return true;
                }
                target_vel < -5.0 && self.rng.rand_f() < 0.35
            }
            Tactic::Ram => self.rng.rand_f() < 0.2,
            Tactic::FlankLeft | Tactic::FlankRight => target_vel <= 5.0,
            Tactic::BackOff => bot.clear_shot && d2 <= 2500.0 && bot.stuck <= 0.5,
        }
    }

    /// Tactic re-pick and destination.
    fn choose_tactic(&mut self, t: &mut Tank, target: &TargetInfo, target_is_object: bool) {
        let now = self.time_ms;
        let bot = t.ai.as_ref().expect("bot").clone();
        let old = bot.tactic;
        let rel = target.pos - t.pos;
        let dir = rel.normalize_or_zero();
        let d2 = rel.length_squared();
        let (t_right, t_fwd) = (target.rot.x_axis, target.rot.y_axis);
        let closing = target.vel.dot(dir);
        let mut tactic = bot.tactic;
        if (now > bot.tactic_time || bot.tactic == Tactic::Choose) && self.should_change(&bot, d2, closing) {
            // {snipe, ram, flank L, flank R, back}
            let mut w = [1.0f32, 1.0, 1.0, 1.0, 1.0];
            if dir.dot(t_right) >= 0.0 {
                w[3] = 0.0;
            } else {
                w[2] = 0.0;
            }
            // Target facing away from us: flanking is pointless.
            if dir.dot(t_fwd) > 0.0 {
                w[2] = 0.1;
                w[3] = 0.1;
                w[4] = 0.1;
            }
            // Target driving away.
            if closing > 1.0 {
                w[2] = 0.01;
                w[3] = 0.01;
            }
            if closing < -5.0 {
                w[0] = 0.5;
                w[4] = 1.5;
            }
            if d2 > 900.0 {
                w[4] = 0.0;
            }
            if bot.hits > 0.5 {
                w[0] *= 0.5 / bot.hits;
            }
            if !bot.clear_shot {
                w[0] = 0.0;
                w[4] = 0.0;
            }
            if bot.stuck > 0.5 {
                w[4] *= 0.25;
            }
            if target_is_object {
                w[0] = 0.0;
                w[2] = 0.0;
                w[3] = 0.0;
                w[4] = 0.0;
            }
            let carrier_target = matches!((self.scrum_ball(), target.tank), (Some(f), Some((id, ..))) if f.carrier == Some(id));
            if carrier_target {
                w[1] = 5.0;
            }
            let d = &bot.data;
            w[0] *= d.snipe_weight;
            w[1] *= d.ram_weight;
            w[2] *= d.flank_weight;
            w[3] *= d.flank_weight;
            w[4] *= d.back_weight;
            let total: f32 = w.iter().sum();
            let mut r = self.rng.rand_f() * total;
            tactic = Tactic::Ram;
            for (i, &wi) in w.iter().enumerate() {
                if r < wi {
                    tactic = TACTICS[i];
                    break;
                }
                r -= wi;
            }
            t.ai.as_mut().expect("bot").tactic_time = now + 1000;
        }
        let dest = match tactic {
            // Snipe: one metre towards the target. With direct steering the
            // bot turns to face it and, the destination being within (−1, 5]
            // m ahead, holds still.
            Tactic::Snipe => t.pos + dir,
            Tactic::Choose => t.pos,
            Tactic::Ram => target.pos + dir * 15.0,
            Tactic::FlankLeft | Tactic::FlankRight => {
                let side = if tactic == Tactic::FlankLeft { t_right } else { -t_right };
                let mut dest = target.pos - side * 10.0 - t_fwd * 10.0;
                let f = (-rel).dot(t_fwd) * 0.1;
                if f < 1.0 {
                    let b = if f <= 0.0 { 0.0 } else { ((f + 1.0) * 0.5).clamp(0.5, 1.0) };
                    dest = dest * b + target.pos * (1.0 - b);
                }
                dest
            }
            Tactic::BackOff => t.pos * 2.0 - target.pos,
        };
        let bot = t.ai.as_mut().expect("bot");
        bot.tactic = tactic;
        bot.dest = dest;
        if old != tactic {
            bot.ticks_in_tactic = 0;
        }
        bot.ticks_in_tactic += 1;
    }

    /// The bot's move for this tick.
    pub(crate) fn bot_move(&mut self, t: &mut Tank) -> Move {
        let now = self.time_ms;
        let (right, fwd) = (t.rot.x_axis, t.rot.y_axis);
        {
            let bot = t.ai.as_mut().expect("bot");
            if bot.bumped && bot.unstuck == 0 && !(bot.mode == Mode::Attack && bot.tactic == Tactic::BackOff) {
                bot.unstuck = 30;
            }
            if bot.bumped {
                bot.stuck += 1.0;
            }
            if bot.unstuck != 0 {
                bot.unstuck -= 1;
            }
        }

        // Drop a dead or out-of-bounds target.
        if let Some(target) = t.ai.as_ref().expect("bot").target {
            let valid = self.target_info(target).is_some_and(|i| {
                let alive = !i.tank.is_some_and(|(_, _, corpse)| corpse);
                let inside = self.mission_area.is_none_or(|m| {
                    let (x, y) = (i.pos.x - m.x, i.pos.y - m.y);
                    x < m.w + 5.0 && y < m.h + 5.0 && x > -5.0 && y > -5.0
                });
                alive && i.pos.z > self.floor && inside
            });
            if !valid {
                let nt = self.pick_target(t);
                let bot = t.ai.as_mut().expect("bot");
                bot.target = nt;
                bot.ticks_on_target = 0;
            }
        }

        // Periodic scan.
        let (next_scan, mode) = {
            let b = t.ai.as_ref().expect("bot");
            (b.next_scan, b.mode)
        };
        if now > next_scan && mode != Mode::Flee {
            let nt = self.pick_target(t);
            let bot = t.ai.as_mut().expect("bot");
            if bot.target != nt {
                bot.ticks_on_target = 0;
            }
            bot.target = nt;
            bot.next_scan = now + bot.data.scan_delay as i64;
            if nt.is_some() {
                self.set_mode(t, Mode::Attack);
            }
        }

        // Mode switch.
        let mut rel = Vec3::ZERO;
        let mut rel_v = Vec3::ZERO;
        let mut fire = false;
        let a = t.data.clone();
        let is_default_weapon = match (&t.current, &a.default_projectile) {
            (Some(c), Some(d)) => c.name.eq_ignore_ascii_case(&d.name),
            _ => false,
        };
        let weapon_changed = t.current.is_some() && !is_default_weapon;
        let mode = t.ai.as_ref().expect("bot").mode;
        match mode {
            Mode::Attack => {
                let bot = t.ai.as_ref().expect("bot");
                let flee = now > bot.flee_deadline && t.damage > a.damage_amount * bot.data.flee_damage && is_default_weapon && self.scrum_ball().is_none();
                let target = bot.target;
                if flee {
                    self.set_mode(t, Mode::Flee);
                } else if let Some(info) = target.and_then(|tg| self.target_info(tg)) {
                    self.choose_tactic(t, &info, info.tank.is_none());
                    let dir = (info.pos - t.pos).normalize_or_zero();
                    let dest = t.ai.as_ref().expect("bot").dest;
                    if within(dest, t.pos, 300.0) && dir.dot(fwd) > 0.7 {
                        let my_center = t.world_box().center();
                        let splash = t.current.as_ref().is_some_and(|p| p.blast_radius > 0.1);
                        let r = if !splash {
                            info.center - my_center
                        } else {
                            // Splash weapons aim at the ground short of the target.
                            let r = info.pos - my_center;
                            r - r.normalize_or_zero() * (info.sphere_radius * 1.5) + Vec3::Z
                        };
                        let inv = t.rot.transpose();
                        rel = inv * r;
                        rel_v = inv * (info.vel - t.vel);
                        if t.burst_left != 0 || t.trigger_released {
                            fire = true;
                        }
                    }
                } else {
                    self.set_mode(t, Mode::Roam);
                }
            }
            Mode::Flee => {
                let bot = t.ai.as_ref().expect("bot");
                let (flee_damage, dest, deadline) = (bot.data.flee_damage, bot.dest, bot.flee_deadline);
                if t.damage < a.damage_amount * flee_damage * 0.5 || weapon_changed {
                    self.set_mode(t, Mode::RoamReset);
                } else if within(dest, t.pos, 11.0) {
                    if now < deadline {
                        let d = self.random_spawn_point(t);
                        t.ai.as_mut().expect("bot").dest = d;
                    } else {
                        self.set_mode(t, Mode::RoamReset);
                    }
                }
            }
            Mode::Roam | Mode::RoamReset => {
                if mode == Mode::RoamReset {
                    self.set_mode(t, Mode::Roam);
                }
                if within(t.ai.as_ref().expect("bot").dest, t.pos, 11.0) {
                    let d = self.random_spawn_point(t);
                    t.ai.as_mut().expect("bot").dest = d;
                }
            }
        }

        if let Some(info) = t.ai.as_ref().expect("bot").target.and_then(|tg| self.target_info(tg)) {
            if info.vel.length_squared() < 1.0 {
                t.ai.as_mut().expect("bot").ticks_on_target += 1;
            }
        }
        if t.ai.as_ref().expect("bot").unstuck != 0 {
            t.ai.as_mut().expect("bot").dest = t.pos - fwd * 50.0;
        }
        let (dest, direct) = {
            let b = t.ai.as_ref().expect("bot");
            (b.dest, b.mode == Mode::Attack && b.tactic == Tactic::Snipe)
        };
        let steer = self.steer(t, dest, direct);

        // Aim.
        let yaw_cur = a.turret_yaw_range * DEG * t.yaw_n;
        let pitch_cur = a.turret_pitch_range * DEG * t.pitch_n - a.turret_pitch_center * DEG;
        let (vel, gs) = t.current.as_ref().map_or((0.0, 0.0), |p| (p.velocity, p.gravity_scale * self.gravity));
        let aim = if t.current.is_some() { lead_aim(rel, rel_v, vel, gs) } else { Vec3::ZERO };
        let (aim_yaw, mut aim_pitch) = aim_angles(aim);
        // A high-angle cartridge (the mortar): the barrel pitch whose shell
        // comes down on the target (`rel` is in the hull's frame, from the
        // hull's centre).
        let high_angle = t.current.as_ref().filter(|p| p.high_angle() && aim != Vec3::ZERO).cloned();
        if let Some(p) = &high_angle {
            let pn = t.high_angle_pitch_n(p, t.yaw_n, t.world_box().center() + t.rot * rel, self.gravity);
            aim_pitch = a.turret_pitch_range * DEG * pn - a.turret_pitch_center * DEG;
        }
        {
            let bot = t.ai.as_mut().expect("bot");
            let (mut w0, mut w1) = (bot.wobble[0], bot.wobble[1]);
            wobble_step(&mut w0, 0.1, 0.25, 0.05, &mut self.rng);
            wobble_step(&mut w1, 0.1, 0.25, 0.05, &mut self.rng);
            bot.wobble = [w0, w1];
        }
        let bot = t.ai.as_mut().expect("bot");
        let mut acc = 1.0f32;
        if bot.mode == Mode::Attack {
            if bot.tactic == Tactic::Snipe {
                acc = (30.0 - bot.ticks_in_tactic.min(30) as f32) / 60.0 + 0.5;
            }
            acc *= (60.0 - bot.ticks_on_target.min(60) as f32) / 120.0 + 0.5;
        }
        let d = bot.data.clone();
        let cmd_yaw = (1.0 - d.aim_speed) * yaw_cur + (d.aim * DEG * bot.wobble[0] * acc + aim_yaw) * d.aim_speed;
        let cmd_pitch = (1.0 - d.aim_speed) * pitch_cur + (d.aim * DEG * bot.wobble[1] * acc + aim_pitch) * d.aim_speed;
        bot.clear_shot = true;
        let th = d.trigger_happy * DEG;
        if (cmd_yaw - aim_yaw).abs() > th {
            fire = false;
        } else if th * 0.5 < (cmd_pitch - aim_pitch).abs() {
            fire = false;
            if a.turret_pitch_range * DEG < (aim_pitch + a.turret_pitch_center * DEG).abs() {
                bot.clear_shot = false;
            }
        }

        // Line of sight, every 16 qualifying ticks.
        let has_target = bot.target.is_some();
        // (The mortar lobs over what is in between: no line-of-sight test.)
        if bot.clear_shot && fire && t.current.is_some() && has_target && high_angle.is_none() {
            let c = bot.los_countdown;
            bot.los_countdown -= 1;
            if c == 0 {
                bot.los_countdown = 15;
                let target = bot.target.and_then(|tg| self.target_info(tg));
                if let Some(info) = target {
                    let clear = self.line_of_sight(t, aim, rel, rel_v, vel, &info);
                    if !clear {
                        t.ai.as_mut().expect("bot").clear_shot = false;
                    }
                }
            }
        }
        let bot = t.ai.as_mut().expect("bot");
        if !bot.clear_shot {
            fire = false;
        }

        // Drive.
        let s = steer.dot(right);
        let f = steer.dot(fwd);
        let mut x = if s < -0.1 {
            -1.0
        } else if s > 0.1 {
            1.0
        } else if f < 0.0 {
            if s >= 0.0 {
                1.0
            } else {
                -1.0
            }
        } else {
            0.0
        };
        let g = (bot.dest - t.pos).dot(fwd);
        let mut y = if !(-1.0..=5.0).contains(&g) { 1.0 } else { 0.0 };
        if bot.mode == Mode::Attack && bot.tactic == Tactic::BackOff {
            x = -x;
            y = -y;
        }
        if bot.unstuck != 0 {
            y = -1.0;
        }
        let decay = |v: f32| (v as f64 * 0.9885) as f32;
        bot.hits = decay(bot.hits);
        bot.streak = decay(bot.streak);
        bot.stuck = decay(bot.stuck);
        bot.bumped = false;

        let mut mv = Move::NULL;
        mv.x = x;
        mv.y = y;
        mv.yaw = cmd_yaw - yaw_cur;
        mv.pitch = cmd_pitch - pitch_cur;
        mv.trigger[0] = fire;
        mv
    }

    /// Two-segment arc check muzzle → apex → target (0x46ef..).
    // SPEC-GAP: the apex-time selection was partly mangled; I use
    // t_apex = vz/10 for an upward shot (t/2 otherwise), halved when it
    // exceeds the flight time t. Triggers are not part of the ray mask here.
    fn line_of_sight(&self, t: &Tank, aim: Vec3, rel: Vec3, rel_v: Vec3, v: f32, info: &TargetInfo) -> bool {
        let closing = rel_v - aim * v;
        let c2 = closing.length_squared();
        let time = if c2 > 0.0 {
            let q = rel.length_squared() / c2;
            if q <= 0.0 {
                0.0
            } else {
                q.sqrt()
            }
        } else {
            0.0
        };
        let launch = t.rot * aim * v + t.vel;
        let mut t_apex = if launch.z <= 0.0 { time * 0.5 } else { launch.z * 0.1 };
        if t_apex > time {
            t_apex *= 0.5;
        }
        let start = t.world_box().center();
        let apex = start + launch * t_apex - Vec3::new(0.0, 0.0, t_apex * t_apex * 5.0);
        let end = info.center;
        let mut clear = true;
        let mut hit_tank = false;
        for (a, b) in [(start, apex), (apex, end)] {
            if let Some((_, _, _, obj)) = self.ray_cast(a, b, Some(t.id)) {
                match obj {
                    crate::HitObject::Tank(id) => {
                        hit_tank = true;
                        let teammate = self.tank(id).is_some_and(|o| o.team == t.team && (self.team_game || o.team != 0));
                        if id == t.id || teammate {
                            clear = false;
                        }
                    }
                    _ => clear = false,
                }
            }
        }
        if info.tank.is_none() && !hit_tank {
            clear = false;
        }
        clear
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lead_matches_projectile_drop() {
        // Static target 100 m ahead, 85 m/s, gravity_scale 0.4.
        let aim = lead_aim(Vec3::new(0.0, 100.0, 0.0), Vec3::ZERO, 85.0, 0.4);
        let t = 100.0f32 / 85.0;
        let n = (t * 31.25).floor() + 1.0;
        let drop = 0.4 * 0.01024 * n * (n + 1.0) / 2.0;
        assert!((aim.z / aim.y - drop / 100.0).abs() < 0.01, "{aim}");
        // Moving target: lead in the direction of motion.
        let aim = lead_aim(Vec3::new(0.0, 100.0, 0.0), Vec3::new(10.0, 0.0, 0.0), 85.0, 0.4);
        assert!(aim.x > 0.1);
        assert_eq!(lead_aim(Vec3::new(0.1, 0.1, 0.0), Vec3::ZERO, 85.0, 0.4), Vec3::ZERO);
    }

    #[test]
    fn angles_and_wobble() {
        let (y, p) = aim_angles(Vec3::new(1.0, 1.0, 0.0).normalize());
        assert!((y - PI / 4.0).abs() < 1e-5 && p.abs() < 1e-6);
        let (_, p) = aim_angles(Vec3::new(0.0, 1.0, -0.5).normalize());
        assert!(p > 0.0, "down is positive pitch");
        let mut rng = crate::rng::Rng::new(5);
        let mut w = 0.9;
        for _ in 0..10000 {
            wobble_step(&mut w, 0.1, 0.25, 0.05, &mut rng);
            assert!((-1.0..=1.0).contains(&w));
        }
    }
}

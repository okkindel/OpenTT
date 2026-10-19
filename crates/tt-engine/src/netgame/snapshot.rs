//! Snapshots (docs/netcode.md §4): quantised tank and ball states, delta
//! coded against a baseline the client acked, packed into one unreliable
//! message that never exceeds the budget; and the input (move) codec.

use std::collections::HashMap;

use glam::{Mat3, Quat, Vec3};
use tt_game::{Move, Tank};
use tt_net::bits::{BitReader, BitWriter};
use tt_net::input::MoveCodec;
use tt_net::DecodeError;

/// Unreliable message kinds.
pub const MSG_SNAPSHOT: u8 = 1;
pub const MSG_INPUT: u8 = 2;

/// 1/64 m, ±4096 m.
const POS_SCALE: f32 = 64.0;
const POS_BITS: u32 = 19;
/// 1/32 m/s, ±128 m/s.
const VEL_SCALE: f32 = 32.0;
const VEL_BITS: u32 = 13;
const AIM_BITS: u32 = 11;
const POS_SMALL: u32 = 10;
const VEL_SMALL: u32 = 7;
/// Corpse timer: 1/32 s, 10 bits (32 s).
const CORPSE_SCALE: f32 = 32.0;

const G_POS: u8 = 1;
const G_ROT: u8 = 2;
const G_VEL: u8 = 4;
const G_AIM: u8 = 8;
const G_DMG: u8 = 16;
const G_BITS: u8 = 32;

const B_CORPSE: u8 = 1;
const B_ANY: u8 = 2;
const B_ALL: u8 = 4;
const B_BRAIN: u8 = 8;
const B_SEQ: u8 = 16;

/// A tank as the client gets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QTank {
    pub pos: [i32; 3],
    pub rot: u32,
    pub vel: [i32; 3],
    pub aim: [u16; 2],
    /// `damage / damage_amount` in 1/255.
    pub damage: u8,
    pub bits: u8,
    pub corpse_t: u16,
    pub death_seq: u8,
}

/// A scrum ball: carried by a tank, or loose at `pos` with `vel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QFlag {
    pub carrier: u32,
    pub pos: [i32; 3],
    pub vel: [i32; 3],
}

fn q(v: f32, scale: f32, bits: u32) -> i32 {
    let lim = (1i32 << (bits - 1)) - 1;
    if v.is_nan() {
        0
    } else {
        ((v * scale).round() as i64).clamp(-(lim as i64), lim as i64) as i32
    }
}

fn q3(v: Vec3, scale: f32, bits: u32) -> [i32; 3] {
    [q(v.x, scale, bits), q(v.y, scale, bits), q(v.z, scale, bits)]
}

fn dq3(v: [i32; 3], scale: f32) -> Vec3 {
    Vec3::new(v[0] as f32 / scale, v[1] as f32 / scale, v[2] as f32 / scale)
}

fn q_unit(v: f32) -> u16 {
    let steps = ((1u32 << AIM_BITS) - 1) as f32;
    (((v.clamp(-1.0, 1.0) + 1.0) * 0.5 * steps).round()) as u16
}

fn dq_unit(v: u16) -> f32 {
    let steps = ((1u32 << AIM_BITS) - 1) as f32;
    v as f32 / steps * 2.0 - 1.0
}

/// Quaternion "smallest three": 2-bit index of the largest component, the
/// other three in 10 bits each over ±1/√2 (≤ 0.15° error).
pub fn pack_quat(q: Quat) -> u32 {
    let q = q.normalize();
    let a = [q.x, q.y, q.z, q.w];
    let mut big = 0;
    for i in 1..4 {
        if a[i].abs() > a[big].abs() {
            big = i;
        }
    }
    let sign = if a[big] < 0.0 { -1.0 } else { 1.0 };
    let mut out = (big as u32) << 30;
    let mut shift = 20;
    for (i, &c) in a.iter().enumerate() {
        if i == big {
            continue;
        }
        let v = (c * sign * std::f32::consts::SQRT_2).clamp(-1.0, 1.0);
        let n = ((v + 1.0) * 0.5 * 1023.0).round() as u32;
        out |= n << shift;
        shift -= 10;
    }
    out
}

pub fn unpack_quat(p: u32) -> Quat {
    let big = (p >> 30) as usize;
    let mut a = [0f32; 4];
    let mut shift = 20;
    let mut sum = 0.0;
    for (i, c) in a.iter_mut().enumerate() {
        if i == big {
            continue;
        }
        let n = (p >> shift) & 1023;
        *c = (n as f32 / 1023.0 * 2.0 - 1.0) / std::f32::consts::SQRT_2;
        sum += *c * *c;
        shift -= 10;
    }
    a[big] = (1.0 - sum).max(0.0).sqrt();
    Quat::from_xyzw(a[0], a[1], a[2], a[3]).normalize()
}

impl QTank {
    pub fn of(t: &Tank) -> QTank {
        let mut bits = 0;
        if t.is_corpse {
            bits |= B_CORPSE;
        }
        if t.any_contact {
            bits |= B_ANY;
        }
        if t.all_contact {
            bits |= B_ALL;
        }
        if t.brain_ejected {
            bits |= B_BRAIN;
        }
        let mut death_seq = 0;
        if let Some((k, _)) = t.death_sequence {
            bits |= B_SEQ;
            death_seq = k.min(15) as u8;
        }
        let dmg = if t.data.damage_amount > 0.0 { t.damage / t.data.damage_amount } else { 0.0 };
        QTank {
            pos: q3(t.pos, POS_SCALE, POS_BITS),
            rot: pack_quat(Quat::from_mat3(&t.rot)),
            vel: q3(t.vel, VEL_SCALE, VEL_BITS),
            aim: [q_unit(t.yaw_n), q_unit(t.pitch_n)],
            damage: (dmg.clamp(0.0, 1.0) * 255.0).round() as u8,
            bits,
            corpse_t: if t.is_corpse { (t.corpse_timer * CORPSE_SCALE).clamp(0.0, 1023.0) as u16 } else { 0 },
            death_seq,
        }
    }

    pub fn pos(&self) -> Vec3 {
        dq3(self.pos, POS_SCALE)
    }
    pub fn vel(&self) -> Vec3 {
        dq3(self.vel, VEL_SCALE)
    }
    pub fn rot(&self) -> Quat {
        unpack_quat(self.rot)
    }
    pub fn yaw_n(&self) -> f32 {
        dq_unit(self.aim[0])
    }
    pub fn pitch_n(&self) -> f32 {
        dq_unit(self.aim[1])
    }
    pub fn corpse(&self) -> bool {
        self.bits & B_CORPSE != 0
    }

    /// Writes this state into a puppet tank.
    pub fn apply(&self, t: &mut Tank, pos: Vec3, rot: Quat, vel: Vec3, yaw_n: f32, pitch_n: f32) {
        t.prev_vel = t.vel;
        t.pos = pos;
        t.rot = Mat3::from_quat(rot);
        t.vel = vel;
        t.yaw_n = yaw_n;
        t.pitch_n = pitch_n;
        t.damage = self.damage as f32 / 255.0 * t.data.damage_amount;
        t.is_corpse = self.bits & B_CORPSE != 0;
        t.any_contact = self.bits & B_ANY != 0;
        t.all_contact = self.bits & B_ALL != 0;
        t.brain_ejected = self.bits & B_BRAIN != 0;
        t.corpse_timer = self.corpse_t as f32 / CORPSE_SCALE;
        t.death_sequence = if self.bits & B_SEQ != 0 {
            let k = self.death_seq as usize;
            t.data.death_sequences.get(k).map(|d| (k, *d))
        } else {
            None
        };
    }
}

impl QFlag {
    pub fn of(f: &tt_game::Flag) -> QFlag {
        QFlag { carrier: f.carrier.map_or(0, |c| c.0), pos: q3(f.pos, POS_SCALE, POS_BITS), vel: q3(f.vel, VEL_SCALE, VEL_BITS) }
    }
    pub fn pos(&self) -> Vec3 {
        dq3(self.pos, POS_SCALE)
    }
    pub fn vel(&self) -> Vec3 {
        dq3(self.vel, VEL_SCALE)
    }
}

fn put_axis(w: &mut BitWriter, v: i32, base: i32, small: u32, full: u32) {
    let d = v as i64 - base as i64;
    let lim = 1i64 << (small - 1);
    if -lim < d && d < lim {
        w.write_bool(true);
        w.write_signed(d, small);
    } else {
        w.write_bool(false);
        w.write_signed(v as i64, full);
    }
}

fn get_axis(r: &mut BitReader, base: i32, small: u32, full: u32) -> Result<i32, DecodeError> {
    if r.read_bool()? {
        Ok((base as i64 + r.read_signed(small)?) as i32)
    } else {
        Ok(r.read_signed(full)? as i32)
    }
}

fn put_tank(w: &mut BitWriter, net: u32, t: &QTank, base: &QTank) {
    let mut mask = 0;
    if t.pos != base.pos {
        mask |= G_POS;
    }
    if t.rot != base.rot {
        mask |= G_ROT;
    }
    if t.vel != base.vel {
        mask |= G_VEL;
    }
    if t.aim != base.aim {
        mask |= G_AIM;
    }
    if t.damage != base.damage {
        mask |= G_DMG;
    }
    if t.bits != base.bits || t.corpse_t != base.corpse_t || t.death_seq != base.death_seq {
        mask |= G_BITS;
    }
    w.write_varint(net as u64);
    w.write_bits(mask as u64, 6);
    if mask & G_POS != 0 {
        for i in 0..3 {
            put_axis(w, t.pos[i], base.pos[i], POS_SMALL, POS_BITS);
        }
    }
    if mask & G_ROT != 0 {
        w.write_bits(t.rot as u64, 32);
    }
    if mask & G_VEL != 0 {
        for i in 0..3 {
            put_axis(w, t.vel[i], base.vel[i], VEL_SMALL, VEL_BITS);
        }
    }
    if mask & G_AIM != 0 {
        w.write_bits(t.aim[0] as u64, AIM_BITS);
        w.write_bits(t.aim[1] as u64, AIM_BITS);
    }
    if mask & G_DMG != 0 {
        w.write_bits(t.damage as u64, 8);
    }
    if mask & G_BITS != 0 {
        w.write_bits(t.bits as u64, 5);
        if t.bits & B_CORPSE != 0 {
            w.write_bits(t.corpse_t as u64, 10);
        }
        if t.bits & B_SEQ != 0 {
            w.write_bits(t.death_seq as u64, 4);
        }
    }
}

fn get_tank(r: &mut BitReader, base: &QTank) -> Result<QTank, DecodeError> {
    let mask = r.read_bits(6)? as u8;
    let mut t = *base;
    if mask & G_POS != 0 {
        for i in 0..3 {
            t.pos[i] = get_axis(r, base.pos[i], POS_SMALL, POS_BITS)?;
        }
    }
    if mask & G_ROT != 0 {
        t.rot = r.read_bits(32)? as u32;
    }
    if mask & G_VEL != 0 {
        for i in 0..3 {
            t.vel[i] = get_axis(r, base.vel[i], VEL_SMALL, VEL_BITS)?;
        }
    }
    if mask & G_AIM != 0 {
        t.aim = [r.read_bits(AIM_BITS)? as u16, r.read_bits(AIM_BITS)? as u16];
    }
    if mask & G_DMG != 0 {
        t.damage = r.read_bits(8)? as u8;
    }
    if mask & G_BITS != 0 {
        t.bits = r.read_bits(5)? as u8;
        t.corpse_t = if t.bits & B_CORPSE != 0 { r.read_bits(10)? as u16 } else { 0 };
        t.death_seq = if t.bits & B_SEQ != 0 { r.read_bits(4)? as u8 } else { 0 };
    }
    Ok(t)
}

/// The snapshot header (§4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Header {
    /// Snapshot id (from 1) and the id of the baseline it is coded against (0 = none).
    pub id: u32,
    pub baseline: u32,
    /// Server tick the state is from (after it).
    pub tick: u32,
    /// Newest client move the server applied.
    pub ack_move: u32,
    /// Moves waiting in the server queue (negative: starved ticks).
    pub depth: i32,
    /// Control tank (0 = none).
    pub control: u32,
    pub ammo: i32,
}

/// A decoded snapshot: the full state (baseline + changes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub tanks: HashMap<u32, QTank>,
    pub flags: HashMap<u32, QFlag>,
    /// The control tank's exact predicting state ([`owner_words`]).
    pub owner: Option<(u32, Vec<u32>)>,
}

/// Owner state (§2.2): every field the tank step reads or writes, as exact
/// f32/i32 bits, so the client's prediction from it is bit-identical.
pub fn owner_words(t: &Tank) -> Vec<u32> {
    let mut w: Vec<u32> = Vec::with_capacity(48);
    let f = |w: &mut Vec<u32>, v: f32| w.push(v.to_bits());
    for v in [t.pos, t.vel, t.prev_vel, t.boost_impulse] {
        for c in v.to_array() {
            f(&mut w, c);
        }
    }
    for c in t.rot.to_cols_array() {
        f(&mut w, c);
    }
    for c in t.wheel_vel.iter().chain(t.susp_display.iter()) {
        f(&mut w, *c);
    }
    for c in [t.turn_in, t.throttle, t.yaw_n, t.pitch_n, t.damage, t.corpse_timer] {
        f(&mut w, c);
    }
    let flags = t.any_contact as u32 | (t.all_contact as u32) << 1 | (t.is_corpse as u32) << 2 | (t.trigger_released as u32) << 3;
    w.extend([t.boost_counter as u32, t.stuck_ticks as u32, flags, t.ammo as u32, t.burst_left as u32, t.fire_count, t.current_trigger.map_or(0, |x| x.0 + 1), t.carried_flag.map_or(0, |x| x.0 + 1)]);
    // Firing (stage 6): the last shot's server time, the cartridges.
    let hash = |p: &Option<std::sync::Arc<tt_game::ProjectileParams>>| p.as_ref().map_or(0, |p| super::rep::db_hash(&p.name));
    w.extend([t.last_fire as u64 as u32, (t.last_fire as u64 >> 32) as u32, hash(&t.current), hash(&t.saved), t.saved_ammo as u32, t.dry_fire_count]);
    w
}

/// Owner words of the projectile defs (current, saved), as [`super::rep::db_hash`].
pub const OWNER_CURRENT: usize = 45;
pub const OWNER_SAVED: usize = 46;

/// Number of words of [`owner_words`].
pub const OWNER_WORDS: usize = 4 * 3 + 9 + 8 + 6 + 8 + 6;

/// Writes an owner state into the client's tank. Trigger and ball ids are
/// the server's: `map_trigger`/`map_flag` translate them.
pub fn apply_owner(t: &mut Tank, w: &[u32], map_trigger: impl Fn(u32) -> Option<tt_game::TriggerId>, map_flag: impl Fn(u32) -> Option<tt_game::FlagId>, current: Option<std::sync::Arc<tt_game::ProjectileParams>>, saved: Option<std::sync::Arc<tt_game::ProjectileParams>>) {
    if w.len() != OWNER_WORDS {
        return;
    }
    let f = |i: usize| f32::from_bits(w[i]);
    let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
    t.pos = v(0);
    t.vel = v(3);
    t.prev_vel = v(6);
    t.boost_impulse = v(9);
    let mut m = [0f32; 9];
    for (k, x) in m.iter_mut().enumerate() {
        *x = f(12 + k);
    }
    t.rot = Mat3::from_cols_array(&m);
    for k in 0..4 {
        t.wheel_vel[k] = f(21 + k);
        t.susp_display[k] = f(25 + k);
    }
    t.turn_in = f(29);
    t.throttle = f(30);
    t.yaw_n = f(31);
    t.pitch_n = f(32);
    t.damage = f(33);
    t.corpse_timer = f(34);
    t.boost_counter = w[35] as i32;
    t.stuck_ticks = w[36] as i32;
    let flags = w[37];
    t.any_contact = flags & 1 != 0;
    t.all_contact = flags & 2 != 0;
    t.is_corpse = flags & 4 != 0;
    t.trigger_released = flags & 8 != 0;
    t.ammo = w[38] as i32;
    t.burst_left = w[39] as i32;
    t.fire_count = w[40];
    t.current_trigger = w[41].checked_sub(1).and_then(&map_trigger);
    t.carried_flag = w[42].checked_sub(1).and_then(&map_flag);
    t.last_fire = (w[43] as u64 | (w[44] as u64) << 32) as i64;
    if w[OWNER_CURRENT] == 0 || current.is_some() {
        t.current = current;
    }
    if w[OWNER_SAVED] == 0 || saved.is_some() {
        t.saved = saved;
    }
    t.saved_ammo = w[47] as i32;
    t.dry_fire_count = w[48];
}

/// Encodes `cur` against `base`, tanks in `order` (priority) while they fit
/// `budget` bytes. Returns the bytes and the state the client will hold
/// (tanks that did not fit keep their baseline state).
pub fn encode(h: &Header, cur: &State, base: &State, order: &[u32], budget: usize) -> (Vec<u8>, State, Vec<u32>) {
    let mut w = BitWriter::with_capacity(256);
    w.write_bits(MSG_SNAPSHOT as u64, 8);
    w.write_varint(h.id as u64);
    w.write_varint(h.baseline as u64);
    w.write_varint(h.tick as u64);
    w.write_varint(h.ack_move as u64);
    w.write_signed(h.depth.clamp(-128, 127) as i64, 8);
    w.write_varint(h.control as u64);
    w.write_varint(h.ammo.max(0) as u64);
    // The owner state: XOR against the baseline's, one "unchanged" bit per word.
    w.write_bool(cur.owner.is_some());
    if let Some((net, words)) = &cur.owner {
        let zero = vec![0u32; words.len()];
        let bw = base.owner.as_ref().filter(|(n, b)| n == net && b.len() == words.len()).map_or(&zero, |(_, b)| b);
        w.write_varint(*net as u64);
        w.write_varint(words.len() as u64);
        for (a, b) in words.iter().zip(bw) {
            // Changed: the XOR's significant bits (small for small changes).
            w.write_bool(a != b);
            if a != b {
                let x = a ^ b;
                let n = 32 - x.leading_zeros();
                w.write_bits((n - 1) as u64, 5);
                w.write_bits(x as u64, n);
            }
        }
    }
    // Balls first (rare, small), then removals, then tanks by priority.
    let flags: Vec<(&u32, &QFlag)> = {
        let mut v: Vec<_> = cur.flags.iter().filter(|(n, f)| base.flags.get(n) != Some(f)).collect();
        v.sort_by_key(|(n, _)| **n);
        v
    };
    w.write_varint(flags.len() as u64);
    for (n, f) in &flags {
        w.write_varint(**n as u64);
        w.write_varint(f.carrier as u64);
        if f.carrier == 0 {
            for i in 0..3 {
                w.write_signed(f.pos[i] as i64, POS_BITS);
            }
            for i in 0..3 {
                w.write_signed(f.vel[i] as i64, VEL_BITS);
            }
        }
    }
    let mut gone: Vec<u32> = base.tanks.keys().filter(|n| !cur.tanks.contains_key(n)).copied().collect();
    gone.extend(base.flags.keys().filter(|n| !cur.flags.contains_key(n)).copied());
    gone.sort();
    w.write_varint(gone.len() as u64);
    for n in &gone {
        w.write_varint(*n as u64);
    }
    let mut held = State { tanks: HashMap::new(), flags: cur.flags.clone(), owner: cur.owner.clone() };
    let mut sent = Vec::new();
    let mut body = BitWriter::new();
    let mut count = 0u32;
    let fixed = w.byte_len() + 3;
    for n in order {
        let Some(t) = cur.tanks.get(n) else { continue };
        let b = base.tanks.get(n);
        if b == Some(t) {
            held.tanks.insert(*n, *t);
            continue;
        }
        let mut one = BitWriter::new();
        put_tank(&mut one, *n, t, b.unwrap_or(&QTank::default()));
        if fixed + (body.bit_len() + one.bit_len()).div_ceil(8) > budget {
            if let Some(b) = b {
                held.tanks.insert(*n, *b);
            }
            continue;
        }
        put_tank(&mut body, *n, t, b.unwrap_or(&QTank::default()));
        held.tanks.insert(*n, *t);
        sent.push(*n);
        count += 1;
    }
    // Tanks not in `order` keep the baseline.
    for (n, b) in &base.tanks {
        if cur.tanks.contains_key(n) && !held.tanks.contains_key(n) {
            held.tanks.insert(*n, *b);
        }
    }
    w.write_varint(count as u64);
    let body = body.finish();
    // `body` is bit-packed from bit 0; append it bit by bit-exactly.
    let mut r = BitReader::new(&body);
    let mut left = body.len() * 8;
    while left > 0 {
        let n = left.min(32) as u32;
        w.write_bits(r.read_bits(n).unwrap_or(0), n);
        left -= n as usize;
    }
    (w.finish(), held, sent)
}

/// Decodes a snapshot's header (to find its baseline).
pub fn decode_header(b: &[u8]) -> Result<Header, DecodeError> {
    let mut r = BitReader::new(b);
    read_header(&mut r)
}

fn read_header(r: &mut BitReader) -> Result<Header, DecodeError> {
    if r.read_bits(8)? as u8 != MSG_SNAPSHOT {
        return Err(DecodeError::Invalid("not a snapshot"));
    }
    let id = r.read_varint()? as u32;
    let baseline = r.read_varint()? as u32;
    let tick = r.read_varint()? as u32;
    let ack_move = r.read_varint()? as u32;
    let depth = r.read_signed(8)? as i32;
    let control = r.read_varint()? as u32;
    let ammo = r.read_varint()?.min(i32::MAX as u64) as i32;
    Ok(Header { id, baseline, tick, ack_move, depth, control, ammo })
}

/// Decodes a snapshot against `base` (the state of its baseline id).
pub fn decode(b: &[u8], base: &State) -> Result<(Header, State), DecodeError> {
    let mut r = BitReader::new(b);
    let h = read_header(&mut r)?;
    let mut st = base.clone();
    let count = |r: &mut BitReader| -> Result<usize, DecodeError> {
        let n = r.read_varint()?;
        if n > r.remaining_bits() as u64 {
            return Err(DecodeError::Invalid("count"));
        }
        Ok(n as usize)
    };
    st.owner = None;
    if r.read_bool()? {
        let net = r.read_varint()? as u32;
        let n = count(&mut r)?.min(256);
        let zero = vec![0u32; n];
        let bw = base.owner.as_ref().filter(|(bn, b)| *bn == net && b.len() == n).map_or(&zero, |(_, b)| b);
        let mut words = Vec::with_capacity(n);
        for b in bw.iter().take(n) {
            words.push(if r.read_bool()? {
                let n = r.read_bits(5)? as u32 + 1;
                b ^ r.read_bits(n)? as u32
            } else {
                *b
            });
        }
        st.owner = Some((net, words));
    }
    for _ in 0..count(&mut r)? {
        let n = r.read_varint()? as u32;
        let carrier = r.read_varint()? as u32;
        let mut f = QFlag { carrier, ..Default::default() };
        if carrier == 0 {
            for i in 0..3 {
                f.pos[i] = r.read_signed(POS_BITS)? as i32;
            }
            for i in 0..3 {
                f.vel[i] = r.read_signed(VEL_BITS)? as i32;
            }
        }
        st.flags.insert(n, f);
    }
    for _ in 0..count(&mut r)? {
        let n = r.read_varint()? as u32;
        st.tanks.remove(&n);
        st.flags.remove(&n);
    }
    for _ in 0..count(&mut r)? {
        let n = r.read_varint()? as u32;
        let b = st.tanks.get(&n).copied().unwrap_or_default();
        let t = get_tank(&mut r, &b)?;
        st.tanks.insert(n, t);
    }
    Ok((h, st))
}

/// A move on the wire (§3.3): x/y as −1/0/+1, yaw/pitch as the 16-bit
/// angles of `Move::clamp`, trigger 0. Holds the raw move; what the server
/// gets decodes to the same `netmove::server_move` as the raw one.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NetMove(pub Move);

impl MoveCodec for NetMove {
    fn encode(&self, w: &mut BitWriter) {
        let axis = |v: f32| match tt_game::netmove::axis_to_u5(v) {
            16 => 1,
            p if p > 16 => 2,
            _ => 0,
        };
        w.write_bits(axis(self.0.x), 2);
        w.write_bits(axis(self.0.y), 2);
        for a in [self.0.yaw, self.0.pitch] {
            let u = tt_game::netmove::angle_to_u16(a);
            w.write_bool(u != 0);
            if u != 0 {
                w.write_bits(u as u64, 16);
            }
        }
        w.write_bool(self.0.trigger[0]);
    }

    fn decode(r: &mut BitReader) -> Result<Self, DecodeError> {
        let axis = |v: u64| match v {
            0 => -1.0,
            1 => 0.0,
            2 => 1.0,
            _ => 0.0,
        };
        let mut m = Move::NULL;
        m.x = axis(r.read_bits(2)?);
        m.y = axis(r.read_bits(2)?);
        let mut ang = [0f32; 2];
        for a in &mut ang {
            if r.read_bool()? {
                // A quarter step away from zero, so the server's own
                // truncation (`server_move`) lands on the same 16-bit value.
                let u = r.read_bits(16)? as u16 as i16 as f32;
                *a = (u + 0.25 * u.signum()) * (2.0 * std::f32::consts::PI / 65536.0);
            }
        }
        m.yaw = ang[0];
        m.pitch = ang[1];
        m.trigger[0] = r.read_bool()?;
        Ok(NetMove(m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tank(i: i32) -> QTank {
        QTank { pos: [i * 100, -i * 7, 500], rot: pack_quat(Quat::from_rotation_z(i as f32 * 0.3)), vel: [i, 2, -3], aim: [1023, 1024 + i as u16], damage: (i * 10) as u8, bits: B_ANY | B_ALL, corpse_t: 0, death_seq: 0 }
    }

    #[test]
    fn quaternion_error_is_small() {
        for i in 0..200 {
            let a = i as f32 * 0.137;
            let q = Quat::from_euler(glam::EulerRot::ZXY, a, (a * 0.7).sin() * 0.5, (a * 1.3).cos() * 0.3);
            let u = unpack_quat(pack_quat(q));
            assert!(q.angle_between(u).to_degrees() < 0.2, "{}", q.angle_between(u).to_degrees());
        }
    }

    #[test]
    fn delta_round_trip_and_budget() {
        let mut base = State::default();
        for i in 1..=10 {
            base.tanks.insert(i, tank(i as i32));
        }
        base.flags.insert(50, QFlag { carrier: 3, ..Default::default() });
        let mut cur = base.clone();
        cur.tanks.get_mut(&2).unwrap().pos[0] += 5; // small delta
        cur.tanks.get_mut(&3).unwrap().pos[1] += 100_000; // full
        cur.tanks.get_mut(&4).unwrap().bits |= B_CORPSE;
        cur.tanks.get_mut(&4).unwrap().corpse_t = 77;
        cur.tanks.remove(&5);
        cur.tanks.insert(11, tank(11));
        cur.flags.insert(50, QFlag { carrier: 0, pos: [1, 2, 3], vel: [4, 5, 6] });
        let order: Vec<u32> = (1..=11).collect();
        let h = Header { id: 2, baseline: 1, tick: 99, ack_move: 7, depth: -2, control: 11, ammo: 40 };
        let (bytes, held, sent) = encode(&h, &cur, &base, &order, 1000);
        assert_eq!(held, cur);
        assert_eq!(sent, vec![2, 3, 4, 11]);
        let (h2, st) = decode(&bytes, &base).unwrap();
        assert_eq!(h2, h);
        assert_eq!(st, cur);
        // Unchanged: just the header.
        let (small, _, _) = encode(&h, &cur, &cur, &order, 1000);
        assert!(small.len() < 16, "{}", small.len());
        // Over budget: the rest keep the baseline; the client agrees.
        let (bytes, held, sent) = encode(&h, &cur, &base, &order, 34);
        assert!(bytes.len() <= 34 && !sent.is_empty() && sent.len() < 4, "{} {sent:?}", bytes.len());
        let (_, st) = decode(&bytes, &base).unwrap();
        assert_eq!(st, held);
    }

    #[test]
    fn moves_round_trip() {
        use tt_game::netmove::server_move;
        for i in 0..2000 {
            let f = i as f32 * 0.37;
            let raw = Move { x: f.sin(), y: (f * 1.3).cos(), yaw: f.sin() * 0.4, pitch: -(f * 0.7).cos() * 0.05, trigger: [i % 3 == 0, true, false, false, false, false], ..Move::NULL };
            let mut w = BitWriter::new();
            NetMove(raw).encode(&mut w);
            let b = w.finish();
            let got = NetMove::decode(&mut BitReader::new(&b)).unwrap().0;
            assert_eq!(server_move(&got), server_move(&raw), "{raw:?}");
        }
    }
}

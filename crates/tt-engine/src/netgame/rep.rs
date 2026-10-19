//! Replication messages (stage 3): objects that appear and go (reliable,
//! ordered) and the world's events (reliable, unordered "event" lane, 1 s
//! lifetime). Both carry the server tick they belong to; the client applies
//! them when its render timeline reaches that tick.

use tt_net::bits::{BitReader, BitWriter};
use tt_net::DecodeError;

/// What a projectile or tank hit, with server ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetHit {
    Terrain,
    Tank(u32),
    Static(u32),
}

/// Ids are the server's world ids (`TankId.0`, `ProjectileId.0`,
/// `TriggerId.0`, `FlagId.0`).
#[derive(Debug, Clone, PartialEq)]
pub enum Rep {
    /// A tank appeared or changed def, name or team.
    TankInfo { net: u32, def: String, name: String, team: i32 },
    TankRemove { net: u32 },
    FlagAdd { net: u32, def: String },
    FlagRemove { net: u32 },
    /// A power-up, pad or volcano: known, moved, switched, re-declared.
    /// `xf` = the 3×3 matrix columns, then the translation.
    TriggerSet { net: u32, class: String, def: String, xf: [f32; 12], on: bool, state: u8 },
    TriggerRemove { net: u32 },
    /// The control tank's current projectile (reticle, HUD).
    Cartridge { net: u32, projectile: String },
    /// A projectile appeared (its state after its first tick); brain jars
    /// carry the jar shape.
    /// `fire` = the shooter's `fire_count` after the shot (0 if not a shot):
    /// the client matches its predicted shot by (shooter, fire).
    Spawn { net: u32, def: String, pos: [f32; 3], vel: [f32; 3], source: u32, scale: f32, brain: String, fire: u32 },
    Bounce { net: u32, pos: [f32; 3], vel: [f32; 3] },
    Explode { net: u32, def: String, pos: [f32; 3], normal: [f32; 3], hit: NetHit, source: u32 },
    /// A shot; `def` () names a cartridge with its own fire sound
    /// (mortar, scatter shot), "" for the tank's `fire_sound`.
    Fired { tank: u32, def: String },
    DryFired { tank: u32 },
    Collided { tank: u32, other: NetHit, speed: f32 },
    PowerUpSwitched { trigger: u32, on: bool },
    PadUsed { trigger: u32, tank: u32 },
    Volcano { trigger: u32, rocks: u32 },
    /// A power-up's user animation thread (`playUserThread`): slot and
    /// time scale; the client plays it from the start.
    UserThread { trigger: u32, slot: u32, scale: f32 },
}

impl Rep {
    /// Goes on the reliable ordered lane (state), else the event lane.
    pub fn reliable(&self) -> bool {
        matches!(self, Rep::TankInfo { .. } | Rep::TankRemove { .. } | Rep::FlagAdd { .. } | Rep::FlagRemove { .. } | Rep::TriggerSet { .. } | Rep::TriggerRemove { .. } | Rep::Cartridge { .. })
    }
}

fn put_str(w: &mut BitWriter, s: &str) {
    w.write_varint(s.len() as u64);
    w.write_bytes(s.as_bytes());
}

fn get_str(r: &mut BitReader) -> Result<String, DecodeError> {
    let n = r.read_varint()? as usize;
    Ok(String::from_utf8_lossy(&r.read_bytes(n)?).into_owned())
}

fn put_f(w: &mut BitWriter, v: f32) {
    w.write_bits(v.to_bits() as u64, 32);
}

fn get_f(r: &mut BitReader) -> Result<f32, DecodeError> {
    Ok(f32::from_bits(r.read_bits(32)? as u32))
}

/// Projectile positions: 1/64 m over ±16384 m; velocities: 1/32 m/s over ±256 m/s.
fn put_q(w: &mut BitWriter, v: &[f32; 3], scale: f32, bits: u32) {
    let lim = ((1i64 << (bits - 1)) - 1) as f32;
    for x in v {
        let q = if x.is_nan() { 0.0 } else { (x * scale).round().clamp(-lim, lim) };
        w.write_signed(q as i64, bits);
    }
}

fn get_q(r: &mut BitReader, scale: f32, bits: u32) -> Result<[f32; 3], DecodeError> {
    Ok([r.read_signed(bits)? as f32 / scale, r.read_signed(bits)? as f32 / scale, r.read_signed(bits)? as f32 / scale])
}

fn put_pos(w: &mut BitWriter, v: &[f32; 3]) {
    put_q(w, v, 64.0, 21);
}

fn get_pos(r: &mut BitReader) -> Result<[f32; 3], DecodeError> {
    get_q(r, 64.0, 21)
}

fn put_vel(w: &mut BitWriter, v: &[f32; 3]) {
    put_q(w, v, 32.0, 14);
}

fn get_vel(r: &mut BitReader) -> Result<[f32; 3], DecodeError> {
    get_q(r, 32.0, 14)
}

/// Def names travel as a 32-bit hash ([`db_hash`]); the decoded name
/// is `#xxxxxxxx`, resolved by the client against its own defs.
pub fn db_hash(name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.to_ascii_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn put_db(w: &mut BitWriter, name: &str) {
    let h = match name.strip_prefix('#').and_then(|x| u32::from_str_radix(x, 16).ok()) {
        Some(h) => h,
        None => db_hash(name),
    };
    w.write_bits(h as u64, 32);
}

fn get_db(r: &mut BitReader) -> Result<String, DecodeError> {
    Ok(format!("#{:08x}", r.read_bits(32)? as u32))
}

fn put_hit(w: &mut BitWriter, h: &NetHit) {
    match h {
        NetHit::Terrain => w.write_bits(0, 2),
        NetHit::Tank(t) => {
            w.write_bits(1, 2);
            w.write_varint(*t as u64);
        }
        NetHit::Static(s) => {
            w.write_bits(2, 2);
            w.write_varint(*s as u64);
        }
    }
}

fn get_hit(r: &mut BitReader) -> Result<NetHit, DecodeError> {
    Ok(match r.read_bits(2)? {
        0 => NetHit::Terrain,
        1 => NetHit::Tank(id(r)?),
        2 => NetHit::Static(id(r)?),
        _ => return Err(DecodeError::Invalid("hit kind")),
    })
}

fn id(r: &mut BitReader) -> Result<u32, DecodeError> {
    let v = r.read_varint()?;
    u32::try_from(v).map_err(|_| DecodeError::Invalid("id"))
}

fn v(w: &mut BitWriter, x: u32) {
    w.write_varint(x as u64);
}

impl Rep {
    pub fn encode(&self, w: &mut BitWriter) {
        match self {
            Rep::TankInfo { net, def, name, team } => {
                w.write_bits(0, 8);
                v(w, *net);
                put_str(w, def);
                put_str(w, name);
                w.write_bits(*team as u32 as u64, 32);
            }
            Rep::TankRemove { net } => {
                w.write_bits(1, 8);
                v(w, *net);
            }
            Rep::FlagAdd { net, def } => {
                w.write_bits(2, 8);
                v(w, *net);
                put_str(w, def);
            }
            Rep::FlagRemove { net } => {
                w.write_bits(3, 8);
                v(w, *net);
            }
            Rep::TriggerSet { net, class, def, xf, on, state } => {
                w.write_bits(4, 8);
                v(w, *net);
                put_str(w, class);
                put_str(w, def);
                for x in xf {
                    put_f(w, *x);
                }
                w.write_bool(*on);
                w.write_bits(*state as u64, 8);
            }
            Rep::TriggerRemove { net } => {
                w.write_bits(5, 8);
                v(w, *net);
            }
            Rep::Cartridge { net, projectile } => {
                w.write_bits(6, 8);
                v(w, *net);
                put_str(w, projectile);
            }
            Rep::Spawn { net, def, pos, vel, source, scale: _, brain, fire } => {
                w.write_bits(7, 8);
                v(w, *net);
                put_db(w, def);
                put_pos(w, pos);
                put_vel(w, vel);
                v(w, *source);
                v(w, *fire);
                w.write_bool(!brain.is_empty());
                if !brain.is_empty() {
                    put_str(w, brain);
                }
            }
            Rep::Bounce { net, pos, vel } => {
                w.write_bits(8, 8);
                v(w, *net);
                put_pos(w, pos);
                put_vel(w, vel);
            }
            Rep::Explode { net, def, pos, normal, hit, source } => {
                w.write_bits(9, 8);
                v(w, *net);
                put_db(w, def);
                put_pos(w, pos);
                for c in normal {
                    w.write_signed((c.clamp(-1.0, 1.0) * 127.0).round() as i64, 8);
                }
                put_hit(w, hit);
                v(w, *source);
            }
            Rep::Fired { tank, def } if def.is_empty() => {
                w.write_bits(10, 8);
                v(w, *tank);
            }
            Rep::Fired { tank, def } => {
                w.write_bits(16, 8);
                v(w, *tank);
                put_db(w, def);
            }
            Rep::DryFired { tank } => {
                w.write_bits(11, 8);
                v(w, *tank);
            }
            Rep::Collided { tank, other, speed } => {
                w.write_bits(12, 8);
                v(w, *tank);
                put_hit(w, other);
                w.write_bits((speed.clamp(0.0, 255.0) * 4.0).round() as u64, 10);
            }
            Rep::PowerUpSwitched { trigger, on } => {
                w.write_bits(13, 8);
                v(w, *trigger);
                w.write_bool(*on);
            }
            Rep::PadUsed { trigger, tank } => {
                w.write_bits(14, 8);
                v(w, *trigger);
                v(w, *tank);
            }
            Rep::Volcano { trigger, rocks } => {
                w.write_bits(15, 8);
                v(w, *trigger);
                v(w, *rocks);
            }
            Rep::UserThread { trigger, slot, scale } => {
                w.write_bits(17, 8);
                v(w, *trigger);
                w.write_bits((*slot).min(255) as u64, 8);
                put_f(w, *scale);
            }
        }
    }

    pub fn decode(r: &mut BitReader) -> Result<Rep, DecodeError> {
        Ok(match r.read_bits(8)? {
            0 => Rep::TankInfo { net: id(r)?, def: get_str(r)?, name: get_str(r)?, team: r.read_bits(32)? as u32 as i32 },
            1 => Rep::TankRemove { net: id(r)? },
            2 => Rep::FlagAdd { net: id(r)?, def: get_str(r)? },
            3 => Rep::FlagRemove { net: id(r)? },
            4 => {
                let (net, class, def) = (id(r)?, get_str(r)?, get_str(r)?);
                let mut xf = [0f32; 12];
                for x in &mut xf {
                    *x = get_f(r)?;
                }
                Rep::TriggerSet { net, class, def, xf, on: r.read_bool()?, state: r.read_bits(8)? as u8 }
            }
            5 => Rep::TriggerRemove { net: id(r)? },
            6 => Rep::Cartridge { net: id(r)?, projectile: get_str(r)? },
            7 => {
                let (net, def, pos, vel, source, fire) = (id(r)?, get_db(r)?, get_pos(r)?, get_vel(r)?, id(r)?, id(r)?);
                let brain = if r.read_bool()? { get_str(r)? } else { String::new() };
                Rep::Spawn { net, def, pos, vel, source, scale: 1.0, brain, fire }
            }
            8 => Rep::Bounce { net: id(r)?, pos: get_pos(r)?, vel: get_vel(r)? },
            9 => {
                let (net, def, pos) = (id(r)?, get_db(r)?, get_pos(r)?);
                let mut normal = [0f32; 3];
                for c in &mut normal {
                    *c = r.read_signed(8)? as f32 / 127.0;
                }
                Rep::Explode { net, def, pos, normal, hit: get_hit(r)?, source: id(r)? }
            }
            10 => Rep::Fired { tank: id(r)?, def: String::new() },
            16 => Rep::Fired { tank: id(r)?, def: get_db(r)? },
            11 => Rep::DryFired { tank: id(r)? },
            12 => Rep::Collided { tank: id(r)?, other: get_hit(r)?, speed: r.read_bits(10)? as f32 / 4.0 },
            13 => Rep::PowerUpSwitched { trigger: id(r)?, on: r.read_bool()? },
            14 => Rep::PadUsed { trigger: id(r)?, tank: id(r)? },
            15 => Rep::Volcano { trigger: id(r)?, rocks: id(r)? },
            17 => Rep::UserThread { trigger: id(r)?, slot: r.read_bits(8)? as u32, scale: get_f(r)? },
            k => return Err(DecodeError::BadKind(k as u8)),
        })
    }
}

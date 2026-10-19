//! The engine's random number generator, bit-exact (docs/engine-notes.md §6).
//!
//! `gRandGen` is MINSTD (×16807 mod 2³¹−1) computed with 32-bit wrapping
//! arithmetic. The stored state is the plain MINSTD value; every output is
//! XOR-ed with 0x075BD924.
//!
//! [`World`](crate::World) keeps both.

use glam::{Mat3, Vec3};

/// Initial `sSeed` before the static constructor runs.
const S_SEED_INIT: u32 = 0x5208_DD0D;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    state: i32,
}

impl Default for Rng {
    /// `gRandGen` as constructed at start-up (state 29377), before any
    /// `setRandomSeed`.
    fn default() -> Rng {
        Rng::new(((S_SEED_INIT.wrapping_mul(0x15A_4E35).wrapping_add(1)) >> 16) & 0x7FFF)
    }
}

impl Rng {
    /// `setSeed(seed)`: the state is the seed as is.
    pub fn new(seed: u32) -> Rng {
        Rng { state: seed as i32 }
    }

    pub fn math_utils() -> Rng {
        Rng::new(0xDEAD_BEEF)
    }

    /// `setRandomSeed(seed)`.
    pub fn set_seed(&mut self, seed: u32) {
        self.state = seed as i32;
    }

    /// `getRandomSeed()`: the current state.
    pub fn seed(&self) -> u32 {
        self.state as u32
    }

    /// `randI()`.
    pub fn rand_i(&mut self) -> u32 {
        let s = self.state;
        let q = s / 127_773; // truncating, like the magic-number division
        let mut n = s.wrapping_mul(16807).wrapping_sub(q.wrapping_mul(0x7FFF_FFFF));
        if n < 0 {
            n = n.wrapping_add(0x7FFF_FFFF);
        }
        self.state = n;
        (n as u32) ^ 0x075B_D924
    }

    /// `randF()`: `randI() · 4.656613e−10` as a float, in [0, 1).
    pub fn rand_f(&mut self) -> f32 {
        // Fild of the (zero-extended) value, fmul by the f32 constant
        // 2^-31, stored as f32.
        (self.rand_i() as f64 * (1.0 / 2_147_483_648.0)) as f32
    }

    /// `randI(min, max)` / `getRandom(min, max)`: inclusive range; for spans
    /// below 0x10000 the high 16 bits are used.
    pub fn rand_range(&mut self, min: i32, max: i32) -> i32 {
        let span = max.wrapping_sub(min);
        let mut r = self.rand_i();
        if span < 0x10000 {
            r >>= 16;
        }
        let m = (span as u32).wrapping_add(1);
        if m == 0 {
            return min.wrapping_add(r as i32);
        }
        min.wrapping_add((r % m) as i32)
    }

    /// `rand % (span + 1)` with the engine's small-span rule.
    pub fn below_inclusive(&mut self, span: u32) -> u32 {
        self.rand_range(0, span as i32) as u32
    }

}

/// x87-style `ftol` of `a·f + b` (a, b integers, f a float): the engine's
/// "random extra milliseconds" formula (`fild; fmul; fiadd; ftol`).
pub fn ftol_scaled(a: i32, f: f32, b: i64) -> i64 {
    (a as f64 * f as f64 + b as f64).trunc() as i64
}

// With phi uniform over 360° the distribution is the same either way.
pub fn random_dir(rng: &mut Rng, axis: Vec3, theta_min: f32, theta_max: f32, phi_min: f32, phi_max: f32) -> Vec3 {
    let theta = (theta_max - theta_min) * rng.rand_f() + theta_min;
    let phi = (phi_max - phi_min) * rng.rand_f() + phi_min;
    // createOrientFromDir: i = normalize(axis × Z), or (0, −1, 0) when parallel.
    let mut i = axis.cross(Vec3::Z);
    if i.length_squared() == 0.0 {
        i = Vec3::new(0.0, -1.0, 0.0);
    }
    let i = i.normalize();
    let theta_rot = Mat3::from_axis_angle(i, -theta.to_radians());
    let phi_rot = Mat3::from_axis_angle(axis.normalize_or(Vec3::Z), -phi.to_radians());
    phi_rot * (theta_rot * axis)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference MINSTD in 64-bit arithmetic (no wrapping needed).
    fn minstd(s: i64) -> i64 {
        s * 16807 % 0x7FFF_FFFF
    }

    #[test]
    fn matches_minstd_for_valid_states() {
        let mut r = Rng::new(29377);
        let mut s = 29377i64;
        for _ in 0..10000 {
            s = minstd(s);
            assert_eq!(r.rand_i(), (s as u32) ^ 0x075B_D924);
            assert_eq!(r.seed() as i64, s);
        }
    }

    #[test]
    fn startup_seed_and_known_values() {
        assert_eq!(Rng::default().seed(), 29377);
        let mut r = Rng::new(1);
        assert_eq!(r.rand_i(), 16807 ^ 0x075B_D924);
        assert_eq!(r.rand_i(), 282_475_249 ^ 0x075B_D924);
        // A negative seed (0xDEADBEEF) goes through the 32-bit wrap path.
        let mut m = Rng::math_utils();
        let s = 0xDEAD_BEEFu32 as i32;
        let q = s / 127_773;
        let mut n = s.wrapping_mul(16807).wrapping_sub(q.wrapping_mul(0x7FFF_FFFF));
        if n < 0 {
            n = n.wrapping_add(0x7FFF_FFFF);
        }
        assert_eq!(m.rand_i(), n as u32 ^ 0x075B_D924);
    }

    #[test]
    fn ranges_and_floats() {
        let mut r = Rng::new(12345);
        for _ in 0..1000 {
            let f = r.rand_f();
            assert!((0.0..1.0).contains(&f));
            let v = r.rand_range(3, 20);
            assert!((3..=20).contains(&v));
        }
        // Small spans use the high bits.
        let mut a = Rng::new(777);
        let mut b = Rng::new(777);
        assert_eq!(a.rand_range(0, 1), ((b.rand_i() >> 16) % 2) as i32);
        assert_eq!(a.rand_range(0, 0x20000), (b.rand_i() % 0x20001) as i32);
    }

    #[test]
    fn random_dir_in_cone() {
        let mut r = Rng::math_utils();
        for _ in 0..500 {
            let d = random_dir(&mut r, Vec3::Z, 0.0, 90.0, 0.0, 360.0);
            assert!((d.length() - 1.0).abs() < 1e-4 && d.z >= -1e-4);
            let d = random_dir(&mut r, Vec3::Z, 0.0, 45.0, 0.0, 360.0);
            assert!(d.z >= 45f32.to_radians().cos() - 1e-4);
        }
    }
}

/// The game logic draws from it in the order the scripts did (docs/rust-native-plan.md risk R1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineRng {
    seed: i32,
}

const S32_MAX: i64 = i32::MAX as i64;

impl Default for EngineRng {
    /// The interpreter's start state.
    fn default() -> EngineRng {
        EngineRng { seed: 1_376_312_589 }
    }
}

impl EngineRng {
    pub fn new() -> EngineRng {
        EngineRng::default()
    }

    /// `setRandomSeed(seed)` (0 becomes 1; reduced mod 2^31-1, at least 1).
    pub fn set_seed(&mut self, seed: i32) {
        self.seed = if seed == 0 { 1 } else { (seed as i64).rem_euclid(S32_MAX).max(1) as i32 };
    }

    /// `getRandomSeed()`.
    pub fn seed(&self) -> i32 {
        self.seed
    }

    /// The next state (also the raw output).
    pub fn next_state(&mut self) -> i32 {
        let mut n = (self.seed as i64 * 16807) % S32_MAX;
        if n <= 0 {
            n += S32_MAX - 1;
        }
        self.seed = n as i32;
        self.seed
    }

    /// `[lo, hi]` inclusive (`lo + next % span`).
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi as i64 - lo as i64 + 1).max(1);
        (lo as i64 + self.next_state() as i64 % span) as i32
    }

    /// `getRandom(max)`: `[0, max]` (a negative max as 0).
    pub fn get_random_max(&mut self, max: i32) -> i32 {
        self.range(0, max.max(0))
    }
}

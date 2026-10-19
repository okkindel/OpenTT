//! Small deterministic RNG (SplitMix64) and the handshake cookie hash
//! (SipHash-2-4). Both are here to keep `tt-net` dependency-free.

use std::hash::{BuildHasher, Hasher};

/// SplitMix64: fast, seedable, good enough for nonces, session ids and the
/// link simulator. Not a CSPRNG.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng { state: seed }
    }

    /// Seeded from std's per-process random hasher keys and the clock.
    pub fn from_entropy() -> Self {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        h.write_u128(t);
        h.write_usize(&h as *const _ as usize);
        Rng::new(h.finish())
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        p > 0.0 && self.next_f64() < p
    }

    /// Uniform in `0..n` (`n` > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}

/// SipHash-2-4 keyed with (`k0`, `k1`).
pub fn siphash24(k0: u64, k1: u64, data: &[u8]) -> u64 {
    let mut v0 = 0x736f_6d65_7073_6575 ^ k0;
    let mut v1 = 0x646f_7261_6e64_6f6d ^ k1;
    let mut v2 = 0x6c79_6765_6e65_7261 ^ k0;
    let mut v3 = 0x7465_6462_7974_6573 ^ k1;
    macro_rules! round {
        () => {
            v0 = v0.wrapping_add(v1);
            v1 = v1.rotate_left(13);
            v1 ^= v0;
            v0 = v0.rotate_left(32);
            v2 = v2.wrapping_add(v3);
            v3 = v3.rotate_left(16);
            v3 ^= v2;
            v0 = v0.wrapping_add(v3);
            v3 = v3.rotate_left(21);
            v3 ^= v0;
            v2 = v2.wrapping_add(v1);
            v1 = v1.rotate_left(17);
            v1 ^= v2;
            v2 = v2.rotate_left(32);
        };
    }
    let (chunks, rest) = data.as_chunks::<8>();
    for c in chunks {
        let m = u64::from_le_bytes(*c);
        v3 ^= m;
        round!();
        round!();
        v0 ^= m;
    }
    let mut b = (data.len() as u64) << 56;
    for (i, &byte) in rest.iter().enumerate() {
        b |= (byte as u64) << (8 * i);
    }
    v3 ^= b;
    round!();
    round!();
    v0 ^= b;
    v2 ^= 0xff;
    round!();
    round!();
    round!();
    round!();
    v0 ^ v1 ^ v2 ^ v3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siphash_reference_vectors() {
        // Reference key 00..0f, messages 00..(n-1) (Aumasson & Bernstein).
        let k0 = u64::from_le_bytes([0, 1, 2, 3, 4, 5, 6, 7]);
        let k1 = u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(siphash24(k0, k1, &[]), 0x726f_db47_dd0e_0e31);
        let msg: Vec<u8> = (0..15).collect();
        assert_eq!(siphash24(k0, k1, &msg), 0xa129_ca61_49be_45e5);
    }

    #[test]
    fn rng_is_deterministic_and_spread() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut hits = 0;
        for _ in 0..10_000 {
            if a.chance(0.2) {
                hits += 1;
            }
        }
        assert!((1800..2200).contains(&hits), "{hits}");
    }
}

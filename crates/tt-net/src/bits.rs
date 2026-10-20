//! Bit-packed writer/reader (docs/netcode.md §3.2).
//!
//! Bits are packed LSB-first into little-endian bytes through a u64
//! accumulator. Every read is bounds-checked and returns
//! [`DecodeError::Truncated`] instead of panicking.

use crate::error::DecodeError;

/// Appends bit fields to a byte buffer.
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    buf: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(bytes: usize) -> Self {
        BitWriter { buf: Vec::with_capacity(bytes), acc: 0, nbits: 0 }
    }

    /// Number of bits written so far.
    pub fn bit_len(&self) -> usize {
        self.buf.len() * 8 + self.nbits as usize
    }

    /// Number of bytes [`finish`](Self::finish) will return.
    pub fn byte_len(&self) -> usize {
        self.bit_len().div_ceil(8)
    }

    /// Writes the low `bits` bits of `value` (`bits` ≤ 64).
    pub fn write_bits(&mut self, value: u64, bits: u32) {
        debug_assert!(bits <= 64);
        if bits == 0 {
            return;
        }
        if bits > 32 {
            self.write_bits(value & 0xffff_ffff, 32);
            self.write_bits(value >> 32, bits - 32);
            return;
        }
        let v = value & ((1u64 << bits) - 1);
        self.acc |= v << self.nbits;
        self.nbits += bits;
        while self.nbits >= 8 {
            self.buf.push(self.acc as u8);
            self.acc >>= 8;
            self.nbits -= 8;
        }
    }

    pub fn write_bool(&mut self, b: bool) {
        self.write_bits(b as u64, 1);
    }

    /// Two's complement in `bits` bits. Values outside the range wrap.
    pub fn write_signed(&mut self, v: i64, bits: u32) {
        self.write_bits(v as u64, bits);
    }

    /// LEB128-style: 7 data bits + 1 continuation bit per group.
    pub fn write_varint(&mut self, mut v: u64) {
        loop {
            let group = v & 0x7f;
            v >>= 7;
            if v == 0 {
                self.write_bits(group, 8);
                return;
            }
            self.write_bits(group | 0x80, 8);
        }
    }

    pub fn write_bytes(&mut self, data: &[u8]) {
        if self.nbits == 0 {
            self.buf.extend_from_slice(data);
        } else {
            for &b in data {
                self.write_bits(b as u64, 8);
            }
        }
    }

    /// Pads with zero bits to the next byte boundary.
    pub fn align(&mut self) {
        if self.nbits > 0 {
            self.write_bits(0, 8 - self.nbits);
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.align();
        self.buf
    }
}

/// Bits a [`BitWriter::write_varint`] of `v` takes.
pub fn varint_bits(v: u64) -> usize {
    let significant = 64 - v.leading_zeros() as usize;
    8 * significant.div_ceil(7).max(1)
}

/// Reads bit fields written by [`BitWriter`].
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0 }
    }

    pub fn remaining_bits(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    pub fn read_bits(&mut self, bits: u32) -> Result<u64, DecodeError> {
        if bits > 64 {
            return Err(DecodeError::Invalid("bit count > 64"));
        }
        if bits as usize > self.remaining_bits() {
            return Err(DecodeError::Truncated);
        }
        let mut out = 0u64;
        let mut got = 0u32;
        while got < bits {
            let byte = self.data[self.pos / 8] as u32;
            let off = (self.pos % 8) as u32;
            let take = (8 - off).min(bits - got);
            let v = (byte >> off) & ((1u32 << take) - 1);
            out |= (v as u64) << got;
            got += take;
            self.pos += take as usize;
        }
        Ok(out)
    }

    pub fn read_bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.read_bits(1)? != 0)
    }

    pub fn read_signed(&mut self, bits: u32) -> Result<i64, DecodeError> {
        let v = self.read_bits(bits)?;
        if bits == 0 || bits == 64 {
            return Ok(v as i64);
        }
        let shift = 64 - bits;
        Ok(((v << shift) as i64) >> shift)
    }

    pub fn read_varint(&mut self) -> Result<u64, DecodeError> {
        let mut out = 0u64;
        for i in 0..10 {
            let g = self.read_bits(8)?;
            let shift = 7 * i;
            let data = g & 0x7f;
            if shift == 63 && data > 1 {
                return Err(DecodeError::Invalid("varint overflow"));
            }
            out |= data << shift;
            if g & 0x80 == 0 {
                return Ok(out);
            }
        }
        Err(DecodeError::Invalid("varint too long"))
    }

    /// Reads `n` bytes; checks the length before allocating.
    pub fn read_bytes(&mut self, n: usize) -> Result<Vec<u8>, DecodeError> {
        if n.saturating_mul(8) > self.remaining_bits() {
            return Err(DecodeError::Truncated);
        }
        if self.pos.is_multiple_of(8) {
            let start = self.pos / 8;
            self.pos += n * 8;
            return Ok(self.data[start..start + n].to_vec());
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.read_bits(8)? as u8);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn round_trip_random_fields() {
        let mut rng = Rng::new(7);
        for _ in 0..200 {
            let mut w = BitWriter::new();
            let mut fields = Vec::new();
            for _ in 0..50 {
                let bits = rng.next_u32() % 64 + 1;
                let v = rng.next_u64() & if bits == 64 { u64::MAX } else { (1 << bits) - 1 };
                w.write_bits(v, bits);
                fields.push((v, bits));
            }
            let expect_bits = w.bit_len();
            let bytes = w.finish();
            assert_eq!(bytes.len(), expect_bits.div_ceil(8));
            let mut r = BitReader::new(&bytes);
            for (v, bits) in fields {
                assert_eq!(r.read_bits(bits).unwrap(), v);
            }
        }
    }

    #[test]
    fn signed_varint_bytes() {
        let mut w = BitWriter::new();
        w.write_bool(true);
        w.write_signed(-5, 7);
        w.write_signed(63, 7);
        for v in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            w.write_varint(v);
            assert_eq!(varint_bits(v), {
                let mut t = BitWriter::new();
                t.write_varint(v);
                t.bit_len()
            });
        }
        w.write_bytes(b"hello");
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert!(r.read_bool().unwrap());
        assert_eq!(r.read_signed(7).unwrap(), -5);
        assert_eq!(r.read_signed(7).unwrap(), 63);
        for v in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            assert_eq!(r.read_varint().unwrap(), v);
        }
        assert_eq!(r.read_bytes(5).unwrap(), b"hello");
    }

    #[test]
    fn truncated_reads_fail() {
        let mut r = BitReader::new(&[0xff]);
        assert_eq!(r.read_bits(9), Err(DecodeError::Truncated));
        assert_eq!(r.read_bits(8).unwrap(), 0xff);
        assert_eq!(r.read_bits(1), Err(DecodeError::Truncated));
        let mut r = BitReader::new(&[0xff; 11]);
        assert!(r.read_varint().is_err());
        let mut r = BitReader::new(&[1, 2]);
        assert_eq!(r.read_bytes(usize::MAX), Err(DecodeError::Truncated));
    }
}

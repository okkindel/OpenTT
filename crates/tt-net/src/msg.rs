//! Wire primitives of the game messages on the reliable lane
//! (`tt_gameplay::wire`, the engine's netgame messages).
//!
//! A *text* (a script value: a number, a name, a file path) and a *tagged
//! text* (a string the server sends many times: message templates, player
//! names) share one varint header `h`:
//!
//! | `h & 3` | meaning | follows |
//! |---|---|---|
//! | 0 | plain string of `h >> 2` bytes | the bytes |
//! | 1 | integer, zigzag `h >> 2` (only for the canonical decimal text) | – |
//! | 2 | tagged text number `h >> 2`, defined earlier on this connection | – |
//! | 3 | new tagged text of `h >> 2` bytes (gets the next number) | the bytes |
//!
//! So `"0"`, `""` and a known tag are one byte each; a tag's text crosses
//! the wire once per connection ([`TextTx`] / [`TextRx`], the replacement of
//! the old per-connection `TagDef` table). The reliable lane is ordered, so
//! both tables advance in step.

use std::collections::HashMap;

use crate::bits::{BitReader, BitWriter};
use crate::DecodeError;

/// Most tagged texts one connection may define (bounds the receiver's table).
pub const MAX_TEXTS: usize = 8192;

pub fn put_str(w: &mut BitWriter, s: &str) {
    w.write_varint(s.len() as u64);
    w.write_bytes(s.as_bytes());
}

pub fn get_str(r: &mut BitReader) -> Result<String, DecodeError> {
    let n = r.read_varint()?;
    get_bytes(r, n)
}

fn get_bytes(r: &mut BitReader, n: u64) -> Result<String, DecodeError> {
    if n > r.remaining_bits() as u64 / 8 {
        return Err(DecodeError::Truncated);
    }
    Ok(String::from_utf8_lossy(&r.read_bytes(n as usize)?).into_owned())
}

/// A count of following items, bounded by the bytes left (each item takes one at least).
pub fn get_count(r: &mut BitReader) -> Result<usize, DecodeError> {
    let n = r.read_varint()?;
    if n > r.remaining_bits() as u64 / 8 {
        return Err(DecodeError::Invalid("count"));
    }
    Ok(n as usize)
}

/// `s` as an integer when it is the canonical decimal text of one.
fn int_text(s: &str) -> Option<i64> {
    if s.len() > 19 {
        return None;
    }
    let i: i64 = s.parse().ok()?;
    (i.to_string() == s).then_some(i)
}

fn zigzag(i: i64) -> u64 {
    ((i << 1) ^ (i >> 63)) as u64
}

fn unzigzag(u: u64) -> i64 {
    ((u >> 1) as i64) ^ -((u & 1) as i64)
}

/// A plain text (modes 0 and 1).
pub fn put_text(w: &mut BitWriter, s: &str) {
    match int_text(s).map(zigzag).filter(|z| *z < 1 << 61) {
        Some(z) => w.write_varint(z << 2 | 1),
        None => {
            w.write_varint((s.len() as u64) << 2);
            w.write_bytes(s.as_bytes());
        }
    }
}

/// The sender's table of tagged texts.
#[derive(Debug, Default, Clone)]
pub struct TextTx {
    ids: HashMap<String, u64>,
}

impl TextTx {
    /// A tagged text (modes 2 and 3).
    pub fn put(&mut self, w: &mut BitWriter, s: &str) {
        match self.ids.get(s) {
            Some(id) => w.write_varint(id << 2 | 2),
            None => {
                let id = self.ids.len() as u64;
                self.ids.insert(s.to_string(), id);
                w.write_varint((s.len() as u64) << 2 | 3);
                w.write_bytes(s.as_bytes());
            }
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// The receiver's table of tagged texts.
#[derive(Debug, Default, Clone)]
pub struct TextRx {
    texts: Vec<String>,
}

impl TextRx {
    /// Any text: (tagged, text).
    pub fn get(&mut self, r: &mut BitReader) -> Result<(bool, String), DecodeError> {
        let h = r.read_varint()?;
        let v = h >> 2;
        Ok(match h & 3 {
            0 => (false, get_bytes(r, v)?),
            1 => (false, unzigzag(v).to_string()),
            2 => (true, self.texts.get(v as usize).cloned().ok_or(DecodeError::Invalid("unknown text"))?),
            _ => {
                if self.texts.len() >= MAX_TEXTS {
                    return Err(DecodeError::TooLarge);
                }
                let s = get_bytes(r, v)?;
                self.texts.push(s.clone());
                (true, s)
            }
        })
    }

    /// A plain text (a tagged one is an error).
    pub fn get_text(&mut self, r: &mut BitReader) -> Result<String, DecodeError> {
        match self.get(r)? {
            (false, s) => Ok(s),
            _ => Err(DecodeError::Invalid("tagged text")),
        }
    }

    pub fn len(&self) -> usize {
        self.texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }
}

/// A plain text without a table (client → server).
pub fn get_text(r: &mut BitReader) -> Result<String, DecodeError> {
    TextRx::default().get_text(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_round_trip_compactly() {
        let mut tx = TextTx::default();
        let mut w = BitWriter::new();
        let plain = ["0", "", "-1", "1250", "899.936", "01", "-0", "+3", "9223372036854775807", "-9223372036854775808", "Bait", "missions/tt1_1.ron"];
        for p in plain {
            put_text(&mut w, p);
        }
        for t in ["MsgClientJoin", "\u{2}%1 joined the game.", "MsgClientJoin", ""] {
            tx.put(&mut w, t);
        }
        let b = w.finish();
        let mut r = BitReader::new(&b);
        let mut rx = TextRx::default();
        for p in plain {
            assert_eq!(rx.get(&mut r).unwrap(), (false, p.to_string()));
        }
        for t in ["MsgClientJoin", "\u{2}%1 joined the game.", "MsgClientJoin", ""] {
            assert_eq!(rx.get(&mut r).unwrap(), (true, t.to_string()));
        }
        assert_eq!(rx.len(), 3);
        // One byte each: small integers, the empty string, a known tag.
        for s in ["0", "", "-1", "15"] {
            let mut w = BitWriter::new();
            put_text(&mut w, s);
            assert_eq!(w.finish().len(), 1, "{s:?}");
        }
        let mut w = BitWriter::new();
        tx.put(&mut w, "MsgClientJoin");
        assert_eq!(w.finish().len(), 1);
    }

    #[test]
    fn bad_input_is_an_error() {
        let mut rx = TextRx::default();
        assert!(rx.get(&mut BitReader::new(&[2 | 5 << 2])).is_err());
        assert!(rx.get(&mut BitReader::new(&[40 << 2, b'x'])).is_err());
        assert!(rx.get_text(&mut BitReader::new(&[1 << 2 | 3, b'x'])).is_err());
        assert!(get_count(&mut BitReader::new(&[100])).is_err());
    }
}

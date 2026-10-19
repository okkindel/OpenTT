//! `strswiz` / `strfrip` / `strfrap`: the save/score obfuscation (spec §7).
//!
//! Strings are treated as Latin-1 bytes, like the engine's `char*`.

/// The salt of every obfuscation key (saves, scores, round stats).
pub const IDLE_TEXT: &str = "refer";

fn bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

/// `strswiz(s, n)`: alternates last, first, second-to-last, second … for
/// `min(strlen(s), atoi(n))` characters.
pub fn strswiz(s: &str, n: &str) -> String {
    let b = bytes(s);
    let l = b.len() as i64;
    let m = l.min(crate::fields::atoi(n) as i64).max(0) as usize;
    let out: Vec<u8> = (0..m).map(|i| if i % 2 == 0 { b[b.len() - 1 - i / 2] } else { b[i / 2] }).collect();
    tt_assets::latin1(&out)
}

/// The four-register keystream.
#[derive(Debug, Clone, Copy)]
struct Keystream {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

impl Keystream {
    fn seed(key: &[u8]) -> Keystream {
        let mut k = Keystream { a: 1, b: 0, c: 0, d: 0 };
        for &byte in key {
            let v = byte as i8 as i32 as u32;
            k.a = k.a.wrapping_mul(v.wrapping_add(1));
            k.b = k.b.wrapping_add(v).wrapping_add(1);
            k.c = k.c.wrapping_add(k.b.wrapping_mul(k.a));
            k.d = (k.a >> 2).wrapping_add(v.wrapping_add(0x21).wrapping_mul(k.d));
        }
        k
    }

    fn next(&mut self) -> u8 {
        let mut x = self.a;
        x ^= x << 17;
        x ^= x >> 13;
        x ^= x << 5;
        self.c = (self.c & 0xFFFF).wrapping_mul(36969).wrapping_add(self.c >> 16);
        self.d = (self.d & 0xFFFF).wrapping_mul(18000).wrapping_add(self.d >> 16);
        self.b = self.b.wrapping_mul(69069).wrapping_add(1234567);
        self.a = x;
        (((self.b ^ self.d) & 0xFF) + (self.a & 0xFF)) as u8
    }

    fn feedback(&mut self, c: u8) {
        let c = c as u32;
        self.a = self.a.wrapping_add(c << 12);
        self.b = self.b.wrapping_sub(c << 13);
        self.c = self.c.wrapping_add(c << 14);
        self.d = self.d.wrapping_sub(c << 15);
    }
}

/// `strfrip(key, value)`: encrypts `value`, 0, `key`, 0 and writes every
/// cipher byte as `%2X` (uppercase hex, space padded).
pub fn strfrip(key: &str, value: &str) -> String {
    let key = bytes(key);
    let mut ks = Keystream::seed(&key);
    let mut out = String::new();
    let plain = bytes(value).into_iter().chain([0]).chain(key.iter().copied()).chain([0]);
    for p in plain {
        let c = p ^ ks.next();
        ks.feedback(c);
        out.push_str(&format!("{c:2X}"));
    }
    out
}

/// `sscanf("%X")` of a two-character field (leading spaces allowed).
fn parse_hex2(s: &[u8]) -> u8 {
    let t: String = tt_assets::latin1(s);
    let t = t.trim_start();
    let digits: String = t.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    u32::from_str_radix(&digits, 16).map_or(0, |v| v as u8)
}

/// Decodes `cipher` with `key`; `Some(value)` when the trailing key check passes.
pub fn strfrap_decode(key: &str, cipher: &str) -> Option<String> {
    let key = bytes(key);
    let cipher = bytes(cipher);
    let mut ks = Keystream::seed(&key);
    let mut pos = 0usize;
    let mut next_plain = |ks: &mut Keystream| -> Option<u8> {
        // SPEC-GAP: the spec says fewer than 2 remaining characters give the
        // byte 0; decoding past the end of the cipher text is treated as a
        // failed check instead of looping (the engine reads into the
        // terminator, which cannot produce a valid text either).
        if pos >= cipher.len() {
            return None;
        }
        let c = if cipher.len() - pos < 2 { 0 } else { parse_hex2(&cipher[pos..pos + 2]) };
        pos += 2;
        let p = c ^ ks.next();
        ks.feedback(c);
        Some(p)
    };
    let mut value = Vec::new();
    loop {
        match next_plain(&mut ks)? {
            0 => break,
            p => value.push(p),
        }
    }
    for &expect in key.iter().chain(&[0]) {
        if next_plain(&mut ks)? != expect {
            return None;
        }
    }
    Some(tt_assets::latin1(&value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worked_example() {
        let key = strswiz("referclient", "12");
        assert_eq!(key, "trneefielrc");
        let c = strfrip(&key, "12345");
        assert_eq!(c, "96A720E111A859C045CA1B5BAD6BE339 FEE");
        assert_eq!(c.len(), 2 * (5 + key.len() + 2));
        assert_eq!(strfrap_decode(&key, &c).as_deref(), Some("12345"));
        assert_eq!(strfrip(&key, "9000"), "9E354740AA86E2FCB031CEE2901DC7B2FC");
        assert_eq!(strfrip("refer", "0"), " B63AD64BC 1B76F");
    }

    #[test]
    fn invalid_and_edge_cases() {
        assert_eq!(strfrap_decode("trneefielrc", "96A720E111A859C045CA1B5BAD6BE339 FEF"), None);
        assert_eq!(strfrap_decode("trneefielrc", "lalala"), None);
        assert_eq!(strfrap_decode("0xACDCABBA", ""), None);
        assert_eq!(strswiz("abc", "-1"), "");
        assert_eq!(strswiz("abcd", "2"), "da");
        let c = strfrip("0xBAADF00D", "abba");
        assert_eq!(strfrap_decode("0xBAADF00D", &c).as_deref(), Some("abba"));
        // Non-ASCII key bytes are signed in the seed.
        let c = strfrip("\u{e9}t\u{e9}", "x");
        assert_eq!(strfrap_decode("\u{e9}t\u{e9}", &c).as_deref(), Some("x"));
    }
}

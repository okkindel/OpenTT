//! The game's string conversions, bit-exact.

use std::borrow::Cow;

fn c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

pub use tt_assets::latin1;

/// `strToPlayerName(s)`: at most 16 characters, no control characters or
/// `,.'``, runs of spaces / underscores collapsed, trimmed.
pub fn str_to_player_name(s: &str) -> String {
    const MAX: usize = 16;
    let src = s.trim_start_matches([' ', '_']);
    let mut out = String::new();
    let mut space = false;
    for ch in src.chars() {
        if out.chars().count() >= MAX {
            break;
        }
        if (ch as u32) < 32 || matches!(ch, ',' | '.' | '\'' | '`') {
            continue;
        }
        if ch == ' ' || ch == '_' {
            if space {
                continue;
            }
            space = true;
        } else {
            space = false;
        }
        out.push(ch);
    }
    out.trim_end_matches([' ', '_']).to_string()
}

/// C `atoi` (MSVC: no overflow detection, wraps).
pub fn atoi(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && c_space(b[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut n: i32 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        n = n.wrapping_mul(10).wrapping_add((b[i] - b'0') as i32);
        i += 1;
    }
    if neg {
        n.wrapping_neg()
    } else {
        n
    }
}

/// C `atof`: optional whitespace, sign, digits, fraction, exponent.
pub fn atof(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && c_space(b[i]) {
        i += 1;
    }
    let start = i;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut ndigits = i - digits_start;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let f = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        ndigits += i - f;
    }
    if ndigits == 0 {
        return 0.0;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let mut text = &s[start..i];
    if text.ends_with('.') && !text.contains(['e', 'E']) {
        text = &text[..text.len() - 1];
    }
    let text = text.strip_prefix('+').unwrap_or(text);
    let text = if text.starts_with('.') || text.starts_with("-.") { Cow::Owned(text.replacen('.', "0.", 1)) } else { Cow::Borrowed(text) };
    text.parse::<f64>().unwrap_or(0.0)
}

/// `%g` (precision 6) as printed by the MSVC 7.1 C runtime.
pub fn format_g(v: f64) -> String {
    format_g_prec(v, 6)
}

/// `%.<prec>g` as printed by the MSVC 7.1 C runtime.
pub fn format_g_prec(v: f64, prec: usize) -> String {
    let prec = prec.max(1);
    if v.is_nan() {
        return if v.is_sign_negative() { "-1.#IND".into() } else { "1.#QNAN".into() };
    }
    if v.is_infinite() {
        return if v < 0.0 { "-1.#INF".into() } else { "1.#INF".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let s = format!("{:.*e}", prec - 1, v);
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if exp < -4 || exp >= prec as i32 {
        let mant = strip_zeros(mant);
        format!("{mant}e{}{:03}", if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        let decimals = (prec as i32 - 1 - exp).max(0) as usize;
        strip_zeros(&format!("{:.*}", decimals, v)).to_string()
    }
}

fn strip_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// An engine `F32` result as the console prints it (`%g` of the float).
pub fn f32_str(f: f64) -> String {
    format_g(f as f32 as f64)
}

/// `dStricmp` result sign (-1, 0, 1).
pub fn stricmp(a: &str, b: &str) -> i32 {
    let mut x = a.chars().map(|c| c.to_ascii_lowercase());
    let mut y = b.chars().map(|c| c.to_ascii_lowercase());
    loop {
        match (x.next(), y.next()) {
            (None, None) => return 0,
            (None, Some(_)) => return -1,
            (Some(_), None) => return 1,
            (Some(p), Some(q)) if p != q => return if p < q { -1 } else { 1 },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(atof("  -3.5abc"), -3.5);
        assert_eq!(atof(".5"), 0.5);
        assert_eq!(atof("-.5"), -0.5);
        assert_eq!(atof("1e"), 1.0);
        assert_eq!(atof("5."), 5.0);
        assert_eq!(atof("."), 0.0);
        assert_eq!(atoi(" -12x"), -12);
        assert_eq!(format_g(0.1), "0.1");
        assert_eq!(format_g(1234567.0), "1.23457e+006");
        assert_eq!(format_g(0.00001), "1e-005");
        assert_eq!(format_g(-0.0), "-0");
        assert_eq!(format_g(100.0), "100");
        assert_eq!(f32_str(0.1), "0.1");
        assert_eq!(stricmp("abc", "ABD"), -1);
        assert_eq!(stricmp("Tank", "tank"), 0);
    }
}


pub use tt_game::fmt::{atof, atoi, format_g, stricmp};

pub use crate::client::value::{eq, f32r, f32s, float_to_u32, fs, lf, lu, Val as SVal};

/// A script truth value: a number other than 0, or "true".
pub fn truthy(v: &str) -> bool {
    atof(v) != 0.0 || v.eq_ignore_ascii_case("true")
}

/// `strlen`.
pub fn strlen(s: &str) -> usize {
    s.chars().count()
}

/// `getSubStr(s, start, n)` (negative arguments give "").
pub fn sub_str(s: &str, start: &str, n: &str) -> String {
    let (start, n) = (atoi(start), atoi(n));
    if start < 0 || n < 0 {
        return String::new();
    }
    s.chars().skip(start as usize).take(n as usize).collect()
}

/// `strstr(s, needle)`: character index or -1.
pub fn strstr(s: &str, needle: &str) -> i64 {
    match s.find(needle) {
        Some(p) => s[..p].chars().count() as i64,
        None => -1,
    }
}

/// `strrchr(s, c)`: from the last `c` to the end, or "".
pub fn strrchr(s: &str, c: char) -> String {
    s.rfind(c).map(|p| s[p..].to_string()).unwrap_or_default()
}

/// `stripChars(s, set)`.
pub fn strip_chars(s: &str, set: &str) -> String {
    s.chars().filter(|c| !set.contains(*c)).collect()
}

/// `trim`.
pub fn trim(s: &str) -> String {
    s.trim_matches([' ', '\t', '\n']).to_string()
}

/// `StripMLControlChars`: drops every `<...>` markup tag.
pub fn strip_ml(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('<') {
        out.push_str(&rest[..p]);
        match rest[p..].find('>') {
            Some(q) => rest = &rest[p + q + 1..],
            None => {
                rest = &rest[p..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `detag`: the text of an expanded tagged string (`"\x01<id> text"`).
pub fn detag(s: &str) -> String {
    match s.strip_prefix('\u{1}') {
        Some(rest) => rest.find(' ').map(|p| rest[p + 1..].to_string()).unwrap_or_default(),
        None => s.to_string(),
    }
}

/// Byte ranges of the units of `s` split at every char of `set`.
fn units(s: &str, set: &str) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        if set.contains(ch) {
            v.push((start, i));
            start = i + ch.len_utf8();
        }
    }
    v.push((start, s.len()));
    v
}

const FIELD: &str = "\t\n";

/// `setField(s, index, value)`: past the end the field list is padded.
pub fn set_field(s: &str, index: usize, value: &str) -> String {
    let u = units(s, FIELD);
    if index < u.len() {
        let (a, b) = u[index];
        format!("{}{value}{}", &s[..a], &s[b..])
    } else {
        let pad: String = std::iter::repeat_n('\t', index - (u.len() - 1)).collect();
        format!("{s}{pad}{value}")
    }
}

/// `getField(s, index)`.
pub fn get_field(s: &str, index: usize) -> String {
    units(s, FIELD).get(index).map(|&(a, b)| s[a..b].to_string()).unwrap_or_default()
}

/// Text without control bytes (colour codes, tagged-string marks), as the
/// HUD shows it.
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_like_the_builtins() {
        assert_eq!(sub_str("hello world", "6", "1000"), "world");
        assert_eq!(sub_str("hello", "-1", "2"), "");
        assert_eq!(strstr("ab~wx", "~w"), 2);
        assert_eq!(strstr("ab", "~w"), -1);
        assert_eq!(strrchr("a b c", ' '), " c");
        assert_eq!(strrchr("abc", ' '), "");
        assert_eq!(strip_ml("<just:center>hi <b>there"), "hi there");
        assert_eq!(detag("\u{1}12 Blue Team"), "Blue Team");
        assert_eq!(detag("plain"), "plain");
        assert_eq!(set_field("a\tb\tc", 1, "x"), "a\tx\tc");
        assert_eq!(set_field("a\tb", 3, "x"), "a\tb\t\tx");
        assert_eq!(get_field("a\tb", 1), "b");
        assert_eq!(lf("1.5"), 1.5);
        assert_eq!(f32s(0.1), "0.1");
    }
}

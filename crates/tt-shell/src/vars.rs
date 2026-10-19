//! A store of named values ([`VarFile`]): the settings, the player list,
//! the high scores, kept in memory by key and saved as RON; the escapes of
//! quoted text; number formatting helpers. The old save files' format is
//! [`crate::legacy`]'s.

use tt_game::fmt::format_g;

/// A stored console value, typed the way the interpreter stores it.
#[derive(Debug, Clone, PartialEq)]
pub enum VarValue {
    Int(i32),
    Float(f32),
    Str(String),
}

impl VarValue {
    pub fn as_string(&self) -> String {
        match self {
            VarValue::Int(i) => i.to_string(),
            VarValue::Float(f) => format_g(*f as f64),
            VarValue::Str(s) => s.clone(),
        }
    }

    /// Numeric value with C `atof` semantics.
    pub fn as_f64(&self) -> f64 {
        match self {
            VarValue::Int(i) => *i as f64,
            VarValue::Float(f) => *f as f64,
            VarValue::Str(s) => tt_game::fmt::atof(s),
        }
    }
}

/// An ordered set of console variables (names keep their case; lookups are
/// case-insensitive like the engine's dictionary).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VarFile {
    vars: Vec<(String, VarValue)>,
}

impl VarFile {
    pub fn new() -> VarFile {
        VarFile::default()
    }

    /// Every variable whose name starts with `prefix` (case-insensitive;
    /// e.g. `"players."`, all when empty) as `name = value;` lines sorted by
    /// name (for logs and comparisons).
    pub fn to_text(&self, prefix: &str) -> String {
        let p = prefix.to_ascii_lowercase();
        let mut v: Vec<&(String, VarValue)> = self.vars.iter().filter(|(n, _)| n.to_ascii_lowercase().starts_with(&p)).collect();
        v.sort_by_key(|(n, _)| n.to_ascii_lowercase());
        let mut out = String::new();
        for (name, value) in v {
            match value {
                VarValue::Int(i) => out.push_str(&format!("{name} = {i};")),
                VarValue::Float(f) => out.push_str(&format!("{name} = {};", format_g(*f as f64))),
                VarValue::Str(s) => out.push_str(&format!("{name} = \"{}\";", expand_escape(s))),
            }
            out.push_str("\r\n");
        }
        out
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.vars.iter().position(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    pub fn value(&self, name: &str) -> Option<&VarValue> {
        self.index(name).map(|i| &self.vars[i].1)
    }

    /// The variable as a string ("" when unset, like an unset global).
    pub fn get(&self, name: &str) -> String {
        self.value(name).map(VarValue::as_string).unwrap_or_default()
    }

    pub fn get_f64(&self, name: &str) -> f64 {
        self.value(name).map_or(0.0, VarValue::as_f64)
    }

    pub fn is_set(&self, name: &str) -> bool {
        self.index(name).is_some()
    }

    /// Sets a variable, keeping the case of an existing name.
    pub fn set_value(&mut self, name: &str, value: VarValue) {
        match self.index(name) {
            Some(i) => self.vars[i].1 = value,
            None => self.vars.push((name.to_string(), value)),
        }
    }

    /// What a script assignment of a string stores.
    pub fn set(&mut self, name: &str, value: &str) {
        self.set_value(name, VarValue::Str(value.to_string()));
    }

    pub fn remove(&mut self, name: &str) {
        if let Some(i) = self.index(name) {
            self.vars.remove(i);
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.vars.iter().map(|(n, _)| n.as_str())
    }

    /// Copies every variable of `other` into this set.
    pub fn merge(&mut self, other: &VarFile) {
        for (n, v) in &other.vars {
            self.set_value(n, v.clone());
        }
    }
}

pub fn expand_escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\'' => out.push_str("\\'"),
            c if (c as u32) < 32 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub fn collapse_escape(s: &str) -> String {
    const COLORS: [u8; 10] = [0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0xb, 0xc, 0xe, 0xf];
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] != '\\' || i + 1 >= c.len() {
            out.push(c[i]);
            i += 1;
            continue;
        }
        let e = c[i + 1];
        i += 2;
        match e {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'x' => {
                let hex: String = c[i..].iter().take(2).collect();
                match u8::from_str_radix(&hex, 16) {
                    Ok(v) if hex.len() == 2 => {
                        out.push(v as char);
                        i += 2;
                    }
                    _ => out.push('x'),
                }
            }
            'c' if i < c.len() && c[i].is_ascii_digit() => {
                out.push(COLORS[c[i] as usize - '0' as usize] as char);
                i += 1;
            }
            other => out.push(other),
        }
    }
    out
}

pub use tt_game::fmt::f32_str as num;

/// C `atof` of a script string.
pub use tt_game::fmt::atof;

/// `getWord(text, i)` (words split on space, tab and newline).
pub fn get_word(s: &str, i: usize) -> String {
    s.split([' ', '\t', '\n']).nth(i).unwrap_or("").to_string()
}

pub fn word_count(s: &str) -> usize {
    let delims = s.chars().filter(|c| [' ', '\t', '\n'].contains(c)).count();
    delims + usize::from(s.chars().last().is_some_and(|c| ![' ', '\t', '\n'].contains(&c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_text() {
        let mut f = VarFile::new();
        f.set("players.name0", "Tester");
        f.set_value("players.count", VarValue::Int(1));
        f.set_value("audio.music_volume", VarValue::Float(0.8));
        f.set("misc.q", "a\"b\\c\n");
        assert_eq!(f.get("PLAYERS.NAME0"), "Tester");
        assert_eq!(f.get("audio.music_volume"), "0.8");
        assert_eq!(f.to_text("players."), "players.count = 1;\r\nplayers.name0 = \"Tester\";\r\n");
        assert_eq!(f.to_text("misc."), "misc.q = \"a\\\"b\\\\c\\n\";\r\n");
    }

    #[test]
    fn words() {
        assert_eq!(word_count("--- "), 1);
        assert_eq!(word_count("5 3"), 2);
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("---"), 1);
        assert_eq!(get_word("5 3", 1), "3");
        assert_eq!(get_word("---", 1), "");
        assert_eq!(num(1234567.0), "1.23457e+006");
        assert_eq!(num(75.0), "75");
    }
}

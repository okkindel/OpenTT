//! The engine converts them with the C runtime (`atof`, `atoi`, `dAtob`);
//! the helpers here reproduce those conversions, including the leniency
//! (leading numeric prefix, 0 for garbage).

use std::collections::HashMap;

use tt_assets::mission::SceneObject;

/// C `atof` as an engine `F32` ([`crate::fmt::atof`], then `as f32`).
pub fn atof(s: &str) -> f32 {
    crate::fmt::atof(s) as f32
}

pub use crate::fmt::atoi;

pub fn atob(s: &str) -> bool {
    s.trim().eq_ignore_ascii_case("true") || atof(s) != 0.0
}

/// Case-insensitive field map of one def (inherited fields merged).
#[derive(Debug, Clone, Default)]
pub struct Fields {
    pub name: String,
    pub class: String,
    map: HashMap<String, String>,
}

impl Fields {
    pub fn new(name: &str, class: &str, fields: impl IntoIterator<Item = (String, String)>) -> Fields {
        Fields {
            name: name.to_string(),
            class: class.to_string(),
            map: fields.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect(),
        }
    }

    /// Builds a field map from `(name, value)` string pairs (handy for tests).
    pub fn from_pairs(name: &str, class: &str, pairs: &[(&str, &str)]) -> Fields {
        Fields::new(name, class, pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
    }

    pub fn get(&self, field: &str) -> Option<&str> {
        self.map.get(&field.to_ascii_lowercase()).map(String::as_str)
    }

    pub fn f32_or(&self, field: &str, default: f32) -> f32 {
        self.get(field).map_or(default, atof)
    }

    pub fn i32_or(&self, field: &str, default: i32) -> i32 {
        self.get(field).map_or(default, atoi)
    }

    pub fn bool_or(&self, field: &str, default: bool) -> bool {
        self.get(field).map_or(default, atob)
    }

    pub fn str_or<'a>(&'a self, field: &str, default: &'a str) -> &'a str {
        self.get(field).unwrap_or(default)
    }

    /// Def reference: `None` when absent or empty.
    pub fn reference(&self, field: &str) -> Option<&str> {
        self.get(field).map(str::trim).filter(|s| !s.is_empty())
    }
}

/// All defs from one or more compiled scripts, with parent
/// (`def X(Name : Parent)`) inheritance resolved across files.
#[derive(Debug, Default, Clone)]
pub struct Defs {
    blocks: Vec<SceneObject>,
    by_name: HashMap<String, usize>,
}

impl Defs {
    pub fn new() -> Defs {
        Defs::default()
    }

    /// Adds defs.
    pub fn add(&mut self, objects: impl IntoIterator<Item = SceneObject>) {
        for o in objects {
            let key = o.name.to_ascii_lowercase();
            match self.by_name.get(&key) {
                Some(&i) => self.blocks[i] = o,
                None => {
                    self.by_name.insert(key, self.blocks.len());
                    self.blocks.push(o);
                }
            }
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.blocks.iter().map(|b| b.name.as_str())
    }

    pub fn object(&self, name: &str) -> Option<&SceneObject> {
        self.by_name.get(&name.to_ascii_lowercase()).map(|&i| &self.blocks[i])
    }

    /// One field of `name` as [`Defs::fields`] would give it (the
    /// nearest ancestor's that has it, its last assignment), without
    /// copying the field map.
    pub fn field(&self, name: &str, field: &str) -> Option<&str> {
        let mut cur = self.object(name)?;
        for _ in 0..64 {
            if let Some((_, v)) = cur.fields.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(field)) {
                return Some(v);
            }
            match cur.parent.as_deref().and_then(|p| self.object(p)) {
                Some(p) if !std::ptr::eq(p, cur) => cur = p,
                _ => return None,
            }
        }
        None
    }

    /// Fields of `name` with every ancestor's fields underneath (child wins).
    pub fn fields(&self, name: &str) -> Option<Fields> {
        let mut chain = Vec::new();
        let mut cur = self.object(name)?;
        loop {
            chain.push(cur);
            let parent = cur.parent.as_deref().and_then(|p| self.object(p));
            match parent {
                Some(p) if chain.len() < 64 && !std::ptr::eq(p, cur) => cur = p,
                _ => break,
            }
        }
        let leaf = chain[0];
        let mut merged = Fields::new(&leaf.name, &leaf.class, std::iter::empty());
        for o in chain.iter().rev() {
            for (k, v) in &o.fields {
                merged.map.insert(k.to_ascii_lowercase(), v.clone());
            }
        }
        Some(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_is_the_merged_fields_value() {
        let obj = |name: &str, parent: Option<&str>, fields: &[(&str, &str)]| SceneObject {
            class: "TankFxDef".into(),
            name: name.into(),
            def: true,
            parent: parent.map(String::from),
            fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            children: Vec::new(),
        };
        let mut db = Defs::new();
        db.add([obj("Base", None, &[("a", "1"), ("b", "2"), ("A", "3")]), obj("Child", Some("base"), &[("b", "4"), ("c", "5")])]);
        for name in ["Base", "child"] {
            let merged = db.fields(name).unwrap();
            for f in ["a", "B", "c", "d"] {
                assert_eq!(db.field(name, f), merged.get(f), "{name}.{f}");
            }
        }
        assert_eq!(db.field("nothing", "a"), None);
    }

    #[test]
    fn c_conversions() {
        assert_eq!(atof("11.9"), 11.9);
        assert_eq!(atof("  -3.5abc"), -3.5);
        assert_eq!(atof("1e2"), 100.0);
        assert_eq!(atof("1e"), 1.0);
        assert_eq!(atof("abc"), 0.0);
        assert_eq!(atof(".5"), 0.5);
        assert_eq!(atof("."), 0.0);
        assert_eq!(atoi("-100.0"), -100);
        assert_eq!(atoi("42x"), 42);
        assert_eq!(atoi("x"), 0);
        assert!(atob("TRUE") && atob("1") && atob("0.5"));
        assert!(!atob("0") && !atob("false") && !atob(""));
    }

    #[test]
    fn inheritance_across_files() {
        let mk = |name: &str, parent: Option<&str>, f: &[(&str, &str)]| SceneObject {
            class: "ProjectileDef".into(),
            name: name.into(),
            def: true,
            parent: parent.map(String::from),
            fields: f.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            children: vec![],
        };
        let mut db = Defs::new();
        db.add([mk("Zero", Some("Base"), &[("rounds", "0")])]);
        db.add([mk("base", None, &[("rounds", "145"), ("velocity", "85")])]);
        let f = db.fields("ZERO").unwrap();
        assert_eq!(f.get("Rounds"), Some("0"));
        assert_eq!(f.get("velocity"), Some("85"));
        assert_eq!(f.name, "Zero");
    }
}

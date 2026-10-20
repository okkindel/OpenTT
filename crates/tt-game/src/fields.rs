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

/// Case-insensitive field map of one def.
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

/// All defs by name (case-insensitive). The defs come with their fields
/// already merged (`tt_assets::data::defs::DefSet`, which copies a
/// parent's fields when a def is declared).
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

    /// One field of `name` (its last assignment), without copying the
    /// field map.
    pub fn field(&self, name: &str, field: &str) -> Option<&str> {
        self.object(name)?.fields.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(field)).map(|(_, v)| v.as_str())
    }

    /// The fields of `name`.
    pub fn fields(&self, name: &str) -> Option<Fields> {
        let o = self.object(name)?;
        Some(Fields::new(&o.name, &o.class, o.fields.iter().cloned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_is_the_fields_value() {
        let mut db = Defs::new();
        let fields = [("a", "1"), ("b", "2"), ("A", "3")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        db.add([SceneObject { class: "TankFxDef".into(), name: "Base".into(), def: true, parent: None, fields, children: Vec::new() }]);
        let merged = db.fields("base").unwrap();
        for f in ["a", "B", "c"] {
            assert_eq!(db.field("BASE", f), merged.get(f), "{f}");
        }
        assert_eq!(merged.get("a"), Some("3"));
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
}

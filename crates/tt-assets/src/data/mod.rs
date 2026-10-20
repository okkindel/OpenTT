//! The game's data files in RON (`assets/data/`, `assets/missions/*.ron`):
//! typed declarations of the game's definitions ("defs": tanks, weapons,
//! effects, sounds, ...) and of the objects of a mission.
//!
//! Every declaration keeps the exact text of each field (numbers as
//! written, [`Num`]), and becomes the flat field list the game reads
//! (`(name, value)`, [`Object::to_fields`]): a field's name there is its
//! RON name. Fields a schema does not know (or whose text does not fit its
//! type) are kept in `extra`, so nothing is lost.
//!
//! * [`defs`] – the game's defs, the solo level table and the power-up
//!   weapons;
//! * [`mission`] – mission files (object trees).
//!
//! The importer (`tt-import`) writes the files of the maps it converts.

pub mod defs;
pub mod mission;

use ron::value::RawValue;
pub use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One field of a declaration: `name` (with `index` for an entry of a
/// list field, else empty) and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub index: String,
    pub value: String,
}

impl Field {
    pub fn new(name: impl Into<String>, index: impl Into<String>, value: impl Into<String>) -> Field {
        Field { name: name.into(), index: index.into(), value: value.into() }
    }

    /// The name the object stores the field under (`name` + `index`).
    pub fn key(&self) -> String {
        format!("{}{}", self.name, self.index)
    }
}

/// A number with the exact text the data file has (`"0.547000"`, `"12"`,
/// `"-1e+38"`): written to RON as that number literal, so loading gives the
/// same string (and `atof` of it the same bits).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Num(String);

impl Num {
    /// `None` unless `text` is a plain decimal number (`-?d+(.d+)?(e[+-]?d+)?`)
    /// that RON reads back as the same token.
    pub fn parse(text: &str) -> Option<Num> {
        let b = text.as_bytes();
        let mut i = 0;
        if b.first() == Some(&b'-') {
            i += 1;
        }
        let digits = |i: &mut usize| {
            let s = *i;
            while *i < b.len() && b[*i].is_ascii_digit() {
                *i += 1;
            }
            *i > s
        };
        if !digits(&mut i) {
            return None;
        }
        let mut fraction = false;
        if i < b.len() && b[i] == b'.' {
            i += 1;
            if !digits(&mut i) {
                return None;
            }
            fraction = true;
        }
        let mut exponent = false;
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            if !digits(&mut i) {
                return None;
            }
            exponent = true;
        }
        if i != b.len() {
            return None;
        }
        // An integer literal must fit RON's integers; leading zeros are
        // not RON syntax.
        let int_part = text.trim_start_matches('-').split(['.', 'e', 'E']).next().unwrap_or("");
        if int_part.len() > 1 && int_part.starts_with('0') {
            return None;
        }
        if !fraction && !exponent && text.parse::<i64>().is_err() {
            return None;
        }
        RawValue::from_ron(text).ok()?;
        Some(Num(text.to_string()))
    }

    /// The text as written.
    pub fn text(&self) -> &str {
        &self.0
    }

    /// The value (as C `atof` reads it).
    pub fn value(&self) -> f64 {
        self.0.parse().unwrap_or(0.0)
    }
}

impl std::fmt::Display for Num {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        RawValue::from_ron(&self.0).map_err(serde::ser::Error::custom)?.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw: Box<RawValue> = Deserialize::deserialize(d)?;
        let text = raw.get_ron().trim();
        Num::parse(text).ok_or_else(|| serde::de::Error::custom(format!("not a number: {text}")))
    }
}

/// A typed field's conversion from and to its text.
pub trait FieldType: Sized {
    fn to_text(&self) -> String;
    /// `None` when the text does not fit the type (the field then stays
    /// in `extra`).
    fn from_text(v: &str) -> Option<Self>;
}

impl FieldType for String {
    fn to_text(&self) -> String {
        self.clone()
    }
    fn from_text(v: &str) -> Option<Self> {
        Some(v.to_string())
    }
}

impl FieldType for Num {
    fn to_text(&self) -> String {
        self.0.clone()
    }
    fn from_text(v: &str) -> Option<Self> {
        Num::parse(v)
    }
}

/// A vector (`"0.57735 0.57735 -0.57735"`): components separated by one space.
impl<const N: usize> FieldType for [Num; N] {
    fn to_text(&self) -> String {
        self.iter().map(Num::text).collect::<Vec<_>>().join(" ")
    }
    fn from_text(v: &str) -> Option<Self> {
        let parts: Vec<Num> = v.split(' ').map(Num::parse).collect::<Option<_>>()?;
        parts.try_into().ok()
    }
}

/// A typed declaration of one def or mission object.
pub trait Object: Sized {
    /// The class (the RON variant's name, but for the client's sounds).
    const CLASS: &'static str;
    /// A def (true): declaring the name again reuses it (its fields are
    /// set again); false: a plain object, a later one of the same name
    /// hides the earlier.
    const DEF: bool;
    fn name(&self) -> &str;
    fn parent(&self) -> Option<&str>;
    /// The fields: typed fields in schema order, then the indexed ones,
    /// then `extra`.
    fn to_fields(&self) -> Vec<Field>;
    /// From fields in order (a later value of the same field replaces an
    /// earlier one, as in the object).
    fn from_fields(name: &str, parent: Option<&str>, fields: &[Field]) -> Self;
}

/// Serializes a present optional field as its value (RON's `implicit_some`
/// would still write `Some(..)` around a raw [`Num`]).
#[doc(hidden)]
pub fn ser_option<T: Serialize, S: Serializer>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => v.serialize(s),
        None => s.serialize_none(),
    }
}

/// Fields outside a schema: (field name with its index, value).
pub type Extra = Vec<(String, String)>;

/// Sets `key` in `extra` (replacing an earlier value of the same field).
pub fn set_extra(extra: &mut Extra, key: String, value: String) {
    extra.retain(|(k, _)| !k.eq_ignore_ascii_case(&key));
    extra.push((key, value));
}

/// Removes `key` from `extra` (a typed field took it).
pub fn clear_extra(extra: &mut Extra, key: &str) {
    extra.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
}

/// Declares a typed object schema: `Name = "Class", def { field: Type,
/// ... } [ indexed: Type, ... ]` (indexed fields `name0`, `name1`, … are
/// lists).
#[macro_export]
macro_rules! schema {
    (
        $(#[$m:meta])*
        $ty:ident = $class:literal, $def:literal {
            $( $(#[$fm:meta])* $f:ident : $t:ty ),* $(,)?
        }
        $( [ $( $(#[$im:meta])* $fi:ident : $ti:ty ),* $(,)? ] )?
    ) => {
        $(#[$m])*
        #[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $ty {
            #[serde(default, skip_serializing_if = "String::is_empty")]
            pub name: String,
            /// The def the fields are copied from first.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub parent: Option<String>,
            $(
                $(#[$fm])*
                #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "crate::data::ser_option")]
                pub $f: Option<$t>,
            )*
            $($(
                $(#[$im])*
                /// A list: entry `i` is the field `<name><i>`.
                #[serde(default, skip_serializing_if = "Vec::is_empty")]
                pub $fi: Vec<$ti>,
            )*)?
            /// Fields outside the schema.
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub extra: $crate::data::Extra,
        }

        impl $crate::data::Object for $ty {
            const CLASS: &'static str = $class;
            const DEF: bool = $def;

            fn name(&self) -> &str {
                &self.name
            }

            fn parent(&self) -> Option<&str> {
                self.parent.as_deref()
            }

            fn to_fields(&self) -> Vec<$crate::data::Field> {
                #[allow(unused_imports)]
                use $crate::data::FieldType;
                #[allow(unused_mut)]
                let mut out = Vec::new();
                $(
                    if let Some(v) = &self.$f {
                        out.push($crate::data::Field::new(stringify!($f), "", v.to_text()));
                    }
                )*
                $($(
                    for (i, v) in self.$fi.iter().enumerate() {
                        out.push($crate::data::Field::new(stringify!($fi), i.to_string(), v.to_text()));
                    }
                )*)?
                for (k, v) in &self.extra {
                    out.push($crate::data::Field::new(k.clone(), "", v.clone()));
                }
                out
            }

            fn from_fields(name: &str, parent: Option<&str>, fields: &[$crate::data::Field]) -> Self {
                #[allow(unused_imports)]
                use $crate::data::FieldType;
                let mut o = $ty { name: name.to_string(), parent: parent.map(str::to_string), ..Default::default() };
                for f in fields {
                    let key = f.key();
                    $(
                        if f.index.is_empty() && f.name.eq_ignore_ascii_case(stringify!($f)) {
                            match <$t>::from_text(&f.value) {
                                Some(v) => {
                                    o.$f = Some(v);
                                    $crate::data::clear_extra(&mut o.extra, &key);
                                }
                                None => {
                                    o.$f = None;
                                    $crate::data::set_extra(&mut o.extra, key, f.value.clone());
                                }
                            }
                            continue;
                        }
                    )*
                    $($(
                        if f.name.eq_ignore_ascii_case(stringify!($fi)) {
                            let base = stringify!($fi);
                            let in_extra = o.extra.iter().any(|(k, _)| k.len() > base.len() && k.get(..base.len()).is_some_and(|p| p.eq_ignore_ascii_case(base)));
                            if let (Ok(i), Some(v)) = (f.index.parse::<usize>(), <$ti>::from_text(&f.value)) {
                                if i.to_string() == f.index && i <= o.$fi.len() && !in_extra {
                                    if i == o.$fi.len() {
                                        o.$fi.push(v);
                                    } else {
                                        o.$fi[i] = v;
                                    }
                                    continue;
                                }
                            }
                            // Not representable as a list: every entry goes to `extra`.
                            for (i, v) in std::mem::take(&mut o.$fi).iter().enumerate() {
                                $crate::data::set_extra(&mut o.extra, format!("{base}{i}"), v.to_text());
                            }
                            $crate::data::set_extra(&mut o.extra, key, f.value.clone());
                            continue;
                        }
                    )*)?
                    $crate::data::set_extra(&mut o.extra, key, f.value.clone());
                }
                o
            }
        }
    };
}

/// An object of a class outside the schemas.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generic {
    pub class: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub def: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Extra,
}

impl Generic {
    pub fn from_fields(class: &str, name: &str, parent: Option<&str>, def: bool, fields: &[Field]) -> Generic {
        let mut extra = Extra::new();
        for f in fields {
            set_extra(&mut extra, f.key(), f.value.clone());
        }
        Generic { class: class.to_string(), name: name.to_string(), parent: parent.map(str::to_string), def, fields: extra }
    }

    pub fn to_fields(&self) -> Vec<Field> {
        self.fields.iter().map(|(k, v)| Field::new(k.clone(), "", v.clone())).collect()
    }
}

/// Declares an enum of [`Object`] schemas plus [`Generic`] (`Other`), with
/// the class-independent accessors.
#[macro_export]
macro_rules! object_enum {
    ($(#[$m:meta])* $enum:ident { $($v:ident),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        #[allow(clippy::large_enum_variant)]
        pub enum $enum {
            $($v($v),)*
            Other($crate::data::Generic),
        }

        #[allow(dead_code)]
        impl $enum {
            /// The class.
            pub fn class(&self) -> &str {
                use $crate::data::Object;
                match self {
                    $($enum::$v(_) => <$v as Object>::CLASS,)*
                    $enum::Other(g) => &g.class,
                }
            }

            /// A def ([`Object::DEF`]).
            pub fn is_def(&self) -> bool {
                use $crate::data::Object;
                match self {
                    $($enum::$v(_) => <$v as Object>::DEF,)*
                    $enum::Other(g) => g.def,
                }
            }

            pub fn name(&self) -> &str {
                use $crate::data::Object;
                match self {
                    $($enum::$v(o) => o.name(),)*
                    $enum::Other(g) => &g.name,
                }
            }

            pub fn parent(&self) -> Option<&str> {
                use $crate::data::Object;
                match self {
                    $($enum::$v(o) => o.parent(),)*
                    $enum::Other(g) => g.parent.as_deref(),
                }
            }

            pub fn to_fields(&self) -> Vec<$crate::data::Field> {
                use $crate::data::Object;
                match self {
                    $($enum::$v(o) => o.to_fields(),)*
                    $enum::Other(g) => g.to_fields(),
                }
            }

            /// The schema of `class` (case-insensitive) if there is one and
            /// it is declared the same way, else [`Generic`].
            pub fn from_fields(class: &str, name: &str, parent: Option<&str>, def: bool, fields: &[$crate::data::Field]) -> Self {
                use $crate::data::Object;
                $(
                    if class.eq_ignore_ascii_case(<$v as Object>::CLASS) && def == <$v as Object>::DEF {
                        return $enum::$v(<$v as Object>::from_fields(name, parent, fields));
                    }
                )*
                $enum::Other($crate::data::Generic::from_fields(class, name, parent, def, fields))
            }
        }
    };
}

/// RON extensions the data files use (written as the file's
/// `#![enable(...)]` line).
fn extensions() -> ron::extensions::Extensions {
    ron::extensions::Extensions::IMPLICIT_SOME | ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES
}

/// The canonical text of a data file: `header` (comment lines, each
/// prefixed with `// `), then the value as pretty RON.
pub fn to_ron<T: Serialize>(header: &str, value: &T) -> Result<String, String> {
    let cfg = ron::ser::PrettyConfig::new().struct_names(false).extensions(extensions()).indentor("  ".to_string());
    let body = ron::ser::to_string_pretty(value, cfg).map_err(|e| e.to_string())?;
    let mut out = String::new();
    for l in header.lines() {
        out.push_str(if l.is_empty() { "//" } else { "// " });
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&body);
    out.push('\n');
    Ok(out)
}

/// Reads a data file's text.
pub fn from_ron<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    ron::Options::default().with_default_extension(extensions()).from_str(text).map_err(|e| e.to_string())
}

/// Reads the data file `path` (a mod's file when an enabled mod replaces
/// it, [`crate::mods`]: the whole file).
pub fn load<T: DeserializeOwned>(path: &std::path::Path) -> Result<T, String> {
    let redirected = crate::mods::redirect(path);
    let path = redirected.as_deref().unwrap_or(path);
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    from_ron(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The header comment lines of a data file (what [`to_ron`] wrote).
#[cfg(test)]
pub fn header_of(text: &str) -> String {
    let mut out = Vec::new();
    for l in text.lines() {
        let Some(rest) = l.strip_prefix("//") else { break };
        out.push(rest.strip_prefix(' ').unwrap_or(rest));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_keep_their_text() {
        for t in ["0", "-0", "12", "0.547000", "1.0", "-222768174765569860000000000000000000000.000000", "1e+38", "2.5E-3"] {
            assert_eq!(Num::parse(t).unwrap().text(), t);
        }
        for t in ["", "-", ".5", "5.", "1.0.0", "abc", "0x10", "+3", "007", "1.#INF", "99999999999999999999", " 1", "1 "] {
            assert!(Num::parse(t).is_none(), "{t}");
        }
        let v = <[Num; 3]>::from_text("0.57735 0.57735 -0.57735").unwrap();
        assert_eq!(v.to_text(), "0.57735 0.57735 -0.57735");
        assert!(<[Num; 3]>::from_text("1  2 3").is_none());
        assert!(<[Num; 3]>::from_text("1 2 3 4").is_none());
    }

    schema! {
        Sample = "SampleDef", true {
            shape: String,
            speed: Num,
            dir: [Num; 3],
        } [
            sizes: Num,
        ]
    }

    object_enum!(SampleEnum { Sample });

    #[test]
    fn schema_round_trip() {
        let f = |n: &str, i: &str, v: &str| Field::new(n, i, v);
        let fields = vec![f("shape", "", "models/x.dts"), f("SPEED", "", "11.10"), f("dir", "", "1 2"), f("sizes", "0", "1"), f("sizes", "1", "2.5"), f("other", "", "x"), f("speed", "", "12")];
        let s = Sample::from_fields("A", Some("B"), &fields);
        assert_eq!(s.speed.as_ref().unwrap().text(), "12");
        assert!(s.dir.is_none());
        let t = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(s.extra, vec![t("dir", "1 2"), t("other", "x")]);
        let e = SampleEnum::Sample(s.clone());
        let text = to_ron("test", &vec![e.clone()]).unwrap();
        assert!(text.starts_with("// test\n"), "{text}");
        let back: Vec<SampleEnum> = from_ron(&text).unwrap();
        assert_eq!(back, vec![e], "{text}");
        assert_eq!(header_of(&text), "test");
        let sf: Vec<String> = s.to_fields().iter().map(|f| format!("{}[{}]={}", f.name, f.index, f.value)).collect();
        assert_eq!(sf, ["shape[]=models/x.dts", "speed[]=12", "sizes[0]=1", "sizes[1]=2.5", "dir[]=1 2", "other[]=x"]);
        // A gap in an indexed field: all of it in `extra`.
        let g = Sample::from_fields("A", None, &[f("sizes", "0", "1"), f("sizes", "2", "3")]);
        assert!(g.sizes.is_empty());
        assert_eq!(g.extra, vec![t("sizes0", "1"), t("sizes2", "3")]);
        // Unknown class / declaration kind: generic.
        assert!(matches!(SampleEnum::from_fields("SampleDef", "x", None, false, &fields), SampleEnum::Other(_)));
    }
}

//! The rules keep them where they change a value or a text the players see
//! (scores, timers, counters); everything else is plain Rust.
//!
//! * [`Val`] – a console variable as the engine stored it: `U32`, `F32`
//!   or a string ([`Val::float`]: an integer or string read as a number is
//!   an `F32`);
//! * [`fs`] – `FLT_TO_STR` (`%g` of the double), [`sf`] – `atof` of a
//!   string;
//! * [`lf`] – a string local read as a number (`LOADVAR_FLT`: `F32`),
//!   [`f32r`] / [`f32s`] – a value stored as `F32` read as a number / text.

use tt_game::fmt::{atof, atoi, format_g, stricmp};

/// A value stored as `F32`, read back as text.
pub use tt_game::fmt::f32_str as f32s;

/// `FLT_TO_STR`: `%g` of the double.
pub fn fs(v: f64) -> String {
    format_g(v)
}

/// `STR_TO_FLT`.
pub fn sf(s: &str) -> f64 {
    atof(s)
}

/// A string read as a number through a local (`LOADVAR_FLT`: `F32`).
pub fn lf(s: &str) -> f64 {
    Val::Str(s.to_string()).float()
}

/// A string read through a local with `LOADVAR_UINT`.
pub fn lu(s: &str) -> u32 {
    Val::Str(s.to_string()).int()
}

/// A value stored as `F32`, read back as a number.
pub fn f32r(v: f64) -> f64 {
    v as f32 as f64
}

/// String equality of `COMPARE_STR` (case-insensitive).
pub fn eq(a: &str, b: &str) -> bool {
    stricmp(a, b) == 0
}

/// `(U32)double` as MSVC compiled it (64-bit truncation, low 32 bits).
pub fn float_to_u32(f: f64) -> u32 {
    if f.is_nan() {
        return 0;
    }
    f as i64 as u32
}

/// A console variable's value (`Dictionary::Entry`): integers as `U32`,
/// floats as `F32`, else the string.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Int(u32),
    Float(f32),
    Str(String),
}

impl Default for Val {
    fn default() -> Val {
        Val::Str(String::new())
    }
}

impl Val {
    /// `LOADVAR_UINT`.
    pub fn int(&self) -> u32 {
        match self {
            Val::Int(i) => *i,
            Val::Float(f) => float_to_u32(*f as f64),
            Val::Str(s) if s.len() < 256 => atoi(s) as u32,
            Val::Str(_) => 0,
        }
    }

    /// `LOADVAR_FLT`.
    pub fn float(&self) -> f64 {
        match self {
            Val::Int(i) => *i as f32 as f64,
            Val::Float(f) => *f as f64,
            Val::Str(s) if s.len() < 256 => atof(s) as f32 as f64,
            Val::Str(_) => 0.0,
        }
    }

    /// `LOADVAR_STR`.
    pub fn string(&self) -> String {
        match self {
            Val::Int(i) => (*i as i32).to_string(),
            Val::Float(f) => format_g(*f as f64),
            Val::Str(s) => s.clone(),
        }
    }

    /// `SAVEVAR_FLT` of a float expression.
    pub fn f(v: f64) -> Val {
        Val::Float(v as f32)
    }

    pub fn s(v: impl Into<String>) -> Val {
        Val::Str(v.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_values() {
        assert_eq!(Val::f(0.1).float(), 0.1f32 as f64);
        assert_eq!(Val::s("0.1").float(), 0.1f32 as f64);
        assert_eq!(Val::Int(7).string(), "7");
        assert_eq!(Val::f(899.936).string(), "899.936");
        assert_eq!(Val::s("12abc").int(), 12);
        assert_eq!(lf("2.5"), 2.5);
        assert_eq!(f32s(3.0), "3");
    }
}

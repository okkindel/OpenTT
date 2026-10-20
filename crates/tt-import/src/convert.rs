//! The game's mission files (`.mis`) → the game's RON mission files
//! (`missions/<id>.ron`).
//!
//! The declarations are read from the parsed `.mis` text ([`crate::mis`]),
//! the values kept as the text has them, the class and field names
//! translated to ours ([`crate::names`]); file references
//! (`~/data/shapes/tanks/tank01.dts`, `./TT1_1.ter`) become asset paths
//! through the importer's mapping ([`Paths`]).

use crate::decl::{self, Decl, ScriptValue};
use crate::mis::ast::{CallKind, Expr, Stmt, StmtKind, UnOp};
use crate::names;
use tt_assets::data;
use tt_assets::data::defs::Def;
use tt_assets::data::mission::{Conditional, Mission, MissionObject};

/// Turns a reference of the game's data into an asset path: `(value,
/// directory of the referencing file in the game install)` → the asset
/// path, `None` if the value is not a reference to an imported file.
pub type Paths<'a> = &'a dyn Fn(&str, &str) -> Option<String>;

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Rewrites the file references among the fields of `d` (and its children).
fn rewrite(d: &mut Decl, dir: &str, paths: Paths) {
    for f in &mut d.fields {
        if let ScriptValue::Text(t) = &f.value {
            if let Some(p) = paths(t, dir) {
                f.value = ScriptValue::Text(p);
            }
        }
    }
    for c in &mut d.children {
        rewrite(c, dir, paths);
    }
}

// --- missions -----------------------------------------------------------

/// The parts of a mission file as declarations: defs, objects and
/// the conditional blocks; plus the functions it defines.
#[derive(Default)]
pub struct Source {
    pub defs: Vec<Decl>,
    pub objects: Vec<Decl>,
    pub conditional: Vec<CondSource>,
    pub functions: Vec<String>,
}

pub struct CondSource {
    pub global: String,
    pub value: String,
    pub group: String,
    pub objects: Vec<Decl>,
}

fn order_error(what: &str) -> String {
    format!("{what} after the mission objects / conditional blocks is not supported")
}

/// Reads a mission file's statements (`text` decoded 1:1 from its bytes).
pub fn read_source(text: &str) -> Result<Source, String> {
    let stmts = crate::mis::parser::parse(text)?;
    let mut s = Source::default();
    for st in &stmts {
        match &st.kind {
            StmtKind::Comment(_) => {}
            StmtKind::Function { ns, name, .. } => s.functions.push(match ns {
                Some(ns) => format!("{ns}::{name}"),
                None => name.clone(),
            }),
            StmtKind::Expr(e) => {
                let d = decl::declared(e).ok_or_else(|| format!("unsupported statement: {e:?}"))?;
                let d = decl::from_ast(d)?;
                if d.def {
                    if !s.objects.is_empty() || !s.conditional.is_empty() {
                        return Err(order_error("a def"));
                    }
                    if !d.children.is_empty() {
                        return Err(format!("def {}: child objects", d.name));
                    }
                    s.defs.push(d);
                } else {
                    if !s.conditional.is_empty() {
                        return Err(order_error("an object"));
                    }
                    s.objects.push(d);
                }
            }
            StmtKind::If { cond, then, els } if els.is_empty() => {
                let (global, value) = condition(cond)?;
                let (group, objects) = group_adds(then)?;
                s.conditional.push(CondSource { global, value, group, objects });
            }
            other => return Err(format!("unsupported statement: {other:?}")),
        }
    }
    Ok(s)
}

/// → (global, value).
fn condition(e: &Expr) -> Result<(String, String), String> {
    if let Expr::Unary(UnOp::Not, inner) = e {
        if let Expr::StrEq(a, b) = inner.as_ref() {
            if let (Expr::Var(g), Ok(v)) = (a.as_ref(), decl::constant(b)) {
                if g.starts_with('$') {
                    return Ok((g.clone(), v));
                }
            }
        }
    }
    Err(format!("unsupported condition (only `$global !$= \"value\"`): {e:?}"))
}

/// `{ Group.add(new ...); ... }` → (group, objects).
fn group_adds(body: &[Stmt]) -> Result<(String, Vec<Decl>), String> {
    let mut group: Option<String> = None;
    let mut out = Vec::new();
    for st in body {
        match &st.kind {
            StmtKind::Expr(Expr::Call { kind: CallKind::Method, name, args, .. }) if name.eq_ignore_ascii_case("add") && args.len() == 2 => {
                let g = decl::constant(&args[0])?;
                let Expr::New(d) = &args[1] else { return Err(format!("{g}.add(): not a `new` object")) };
                if group.as_ref().is_some_and(|x| !x.eq_ignore_ascii_case(&g)) {
                    return Err("objects added to different groups in one block".into());
                }
                group = Some(g);
                out.push(decl::from_ast(d)?);
            }
            StmtKind::Comment(_) => {}
            other => return Err(format!("unsupported statement in an `if` block (only `Group.add(new ...);`): {other:?}")),
        }
    }
    Ok((group.ok_or("empty `if` block")?, out))
}

/// A mission object (with its children) from its declaration.
pub fn object_of(d: &Decl) -> Result<MissionObject, String> {
    let (class, fields) = names::translate(&d.class, &d.fields)?;
    let mut o = MissionObject::from_fields(&class, &d.name, d.parent.as_deref(), d.def, &fields);
    match &mut o {
        MissionObject::Group(g) => g.children = d.children.iter().map(object_of).collect::<Result<_, _>>()?,
        _ if !d.children.is_empty() => return Err(format!("{}({}): child objects of an object that is no group", d.class, d.name)),
        _ => {}
    }
    Ok(o)
}

/// Converts a mission file's text (references rewritten by `paths`
/// relative to `dir`, the mission's directory in the game install);
/// also returns the functions it defines (not converted).
pub fn convert_mission_text(text: &str, dir: &str, paths: Paths) -> Result<(Mission, Vec<String>), String> {
    let mut s = read_source(text)?;
    for d in s.defs.iter_mut().chain(s.objects.iter_mut()) {
        rewrite(d, dir, paths);
    }
    for c in &mut s.conditional {
        for d in &mut c.objects {
            rewrite(d, dir, paths);
        }
    }
    let mut defs = Vec::new();
    for d in &s.defs {
        let (class, fields) = names::translate(&d.class, &d.fields)?;
        defs.push(Def::from_fields(&class, &d.name, d.parent.as_deref(), true, &fields));
    }
    let objects = s.objects.iter().map(object_of).collect::<Result<_, _>>()?;
    let mut conditional = Vec::new();
    for c in s.conditional {
        conditional.push(Conditional { unless: (c.global, c.value), add_to: c.group, objects: c.objects.iter().map(object_of).collect::<Result<_, _>>()? });
    }
    Ok((Mission { defs, objects, conditional }, s.functions))
}

fn mission_header(file: &str, functions: &[String]) -> String {
    let mut h = format!("Mission {file}, converted by `tt-convert missions`; values are the .mis text.\nSee tt_assets::data::mission.");
    if !functions.is_empty() {
        h.push_str(&format!("\nNot converted (the game implements them natively): {}.", functions.join(", ")));
    }
    h
}

/// The line after which the editor's mission files end.
const END_MARKER: &str = "//--- OBJECT WRITE END ---";

/// The text of a mission file's bytes, as the game read it: UTF-16 (LE or
/// BE, with a byte order mark or recognised by its NUL bytes) decoded, a
/// UTF-8 byte order mark dropped, else one character per byte (Latin-1);
/// NUL characters removed (padding: the game's reader stopped at them).
/// The notes say what was done to the bytes, for the import report.
pub fn mission_text(bytes: &[u8]) -> (String, Vec<String>) {
    let mut notes = Vec::new();
    let utf16 = |b: &[u8], le: bool| -> String {
        let units: Vec<u16> = b.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&units)
    };
    // Without a byte order mark: UTF-16 when nearly all of the odd (LE) or
    // even (BE) bytes of the start are NUL and few of the others are.
    let head = &bytes[..bytes.len().min(1024) & !1];
    let nul_at = |parity: usize| head.iter().skip(parity).step_by(2).filter(|&&b| b == 0).count();
    let half = head.len() / 2;
    let mostly = |n: usize| half >= 8 && n * 10 >= half * 9;
    let rarely = |n: usize| n * 10 <= half;
    let mut text = if let Some(b) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        notes.push("decoded from UTF-16 (little-endian)".to_string());
        utf16(b, true)
    } else if let Some(b) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        notes.push("decoded from UTF-16 (big-endian)".to_string());
        utf16(b, false)
    } else if let Some(b) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(b).into_owned()
    } else if mostly(nul_at(1)) && rarely(nul_at(0)) {
        notes.push("decoded from UTF-16 (little-endian)".to_string());
        utf16(bytes, true)
    } else if mostly(nul_at(0)) && rarely(nul_at(1)) {
        notes.push("decoded from UTF-16 (big-endian)".to_string());
        utf16(bytes, false)
    } else {
        tt_assets::latin1(bytes)
    };
    let nuls = text.chars().filter(|&c| c == '\0').count();
    if nuls > 0 {
        text.retain(|c| c != '\0');
        notes.push(format!("{nuls} NUL bytes removed"));
    }
    (text, notes)
}

/// `missions/<id>.ron` from the `.mis` file `mis` and its bytes
/// ([`mission_text`]); when the whole text does not parse, what comes
/// after the editor's end line (`//--- OBJECT WRITE END ---`) is left
/// out. Also returns the notes of what was done to the bytes.
pub fn mission_file(mis: &str, bytes: &[u8], paths: Paths) -> Result<(String, Vec<String>), String> {
    let (text, mut notes) = mission_text(bytes);
    let dir = dir_of(mis);
    let converted = match convert_mission_text(&text, dir, paths) {
        Ok(m) => m,
        Err(e) => {
            let end = text.find(END_MARKER).map(|i| i + END_MARKER.len());
            match end.filter(|&i| !text[i..].trim().is_empty()) {
                Some(i) => {
                    let m = convert_mission_text(&text[..i], dir, paths).map_err(|_| format!("{mis}: {e}"))?;
                    notes.push(format!("{} characters after the end of the mission ignored", text[i..].trim().chars().count()));
                    m
                }
                None => return Err(format!("{mis}: {e}")),
            }
        }
    };
    let (m, functions) = converted;
    if m.objects.is_empty() {
        return Err(format!("{mis}: no mission objects in it (an empty or zero-filled file?)"));
    }
    let name = mis.rsplit('/').next().unwrap_or(mis);
    Ok((data::to_ron(&mission_header(name, &functions), &m)?, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIS: &str = "//--- OBJECT WRITE BEGIN ---\r\nnew SimGroup(MissionGroup) {\r\n      musicTrack = \"Lush\";\r\n   new ScriptObject(MissionInfo) {\r\n         name = \"Test\";\r\n   };\r\n};\r\n//--- OBJECT WRITE END ---\r\n";

    fn convert(bytes: &[u8]) -> Result<(String, Vec<String>), String> {
        mission_file("game/data/missions/TT3_2.mis", bytes, &|_: &str, _: &str| None)
    }

    fn utf16(text: &str, le: bool, bom: bool) -> Vec<u8> {
        let mut b = Vec::new();
        if bom {
            b.extend(if le { [0xFF, 0xFE] } else { [0xFE, 0xFF] });
        }
        for u in text.encode_utf16() {
            b.extend(if le { u.to_le_bytes() } else { u.to_be_bytes() });
        }
        b
    }

    #[test]
    fn mission_files_in_other_encodings() {
        let (plain, notes) = convert(MIS.as_bytes()).unwrap();
        assert!(notes.is_empty() && plain.contains("Test"), "{notes:?}");
        for (le, bom) in [(true, false), (true, true), (false, false), (false, true)] {
            let (t, notes) = convert(&utf16(MIS, le, bom)).unwrap_or_else(|e| panic!("le {le} bom {bom}: {e}"));
            assert_eq!(t, plain);
            assert!(notes[0].contains(if le { "little-endian" } else { "big-endian" }), "{notes:?}");
        }
        // NUL padding after the end (a file zero-filled to its size).
        let mut padded = MIS.as_bytes().to_vec();
        padded.extend([0u8; 3000]);
        let (t, notes) = convert(&padded).unwrap();
        assert_eq!(t, plain);
        assert!(notes.iter().any(|n| n.contains("3000 NUL")), "{notes:?}");
        // Junk after the end line.
        let junk = format!("{MIS}\u{1}\u{2}garbage {{ (( ;;\n");
        let (t, notes) = convert(junk.as_bytes()).unwrap();
        assert_eq!(t, plain);
        assert!(notes.iter().any(|n| n.contains("after the end")), "{notes:?}");
        // A zero-filled file, a truncated one: errors (the import leaves the mission out).
        assert!(convert(&[0u8; 4096]).unwrap_err().contains("no mission objects"));
        assert!(convert(&MIS.as_bytes()[..60]).is_err());
    }
}

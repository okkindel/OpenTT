//! Object declarations of a parsed mission file (`.mis`).

use crate::mis::ast::{Expr, ObjectDecl, Sep, UnOp};

/// What a mission file stores in a field: a text, or a variable's
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptValue {
    Text(String),
    Variable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptField {
    pub name: String,
    pub index: String,
    pub value: ScriptValue,
}

impl ScriptField {
    /// `name[index] = "text";`.
    pub fn new(name: impl Into<String>, index: impl Into<String>, text: impl Into<String>) -> ScriptField {
        ScriptField { name: name.into(), index: index.into(), value: ScriptValue::Text(text.into()) }
    }

    pub fn variable(name: impl Into<String>, index: impl Into<String>, variable: impl Into<String>) -> ScriptField {
        ScriptField { name: name.into(), index: index.into(), value: ScriptValue::Variable(variable.into()) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decl {
    pub class: String,
    pub name: String,
    pub parent: Option<String>,
    pub def: bool,
    /// In script order (a field may repeat; the last value counts).
    pub fields: Vec<ScriptField>,
    pub children: Vec<Decl>,
}

/// The string a constant expression evaluates to (an integer as a signed
/// 32-bit number, a float as `%g` of the double).
pub fn constant(e: &Expr) -> Result<String, String> {
    Ok(match e {
        Expr::Str(s) | Expr::Ident(s) => s.clone(),
        Expr::Uint(n) => (*n as i32).to_string(),
        Expr::Float(f) => tt_game::fmt::format_g(*f),
        Expr::Unary(UnOp::Neg, inner) if matches!(inner.as_ref(), Expr::Float(_)) => {
            let Expr::Float(f) = inner.as_ref() else { unreachable!() };
            tt_game::fmt::format_g(-*f)
        }
        Expr::Concat(l, sep, r) => {
            let sep = match sep {
                Sep::Cat => String::new(),
                Sep::Char(c) => char::from(*c).to_string(),
                Sep::Comma => "_".into(),
            };
            format!("{}{sep}{}", constant(l)?, constant(r)?)
        }
        other => return Err(format!("not a constant string: {other:?}")),
    })
}

/// The declaration an object expression makes.
pub fn from_ast(d: &ObjectDecl) -> Result<Decl, String> {
    let class = constant(&d.class)?;
    let name = if matches!(d.name, Expr::Str(ref s) if s.is_empty()) { String::new() } else { constant(&d.name)? };
    if !d.args.is_empty() {
        return Err(format!("{class}({name}): constructor arguments are not supported"));
    }
    let mut fields = Vec::new();
    for f in &d.fields {
        let index = f.index.iter().map(constant).collect::<Result<Vec<_>, _>>()?.join("_");
        let field = match &f.value {
            // The variable's value then.
            Expr::Var(g) if g.starts_with('$') => ScriptField::variable(f.name.clone(), index, g.clone()),
            v => ScriptField::new(f.name.clone(), index, constant(v).map_err(|e| format!("{class}({name}).{}: {e}", f.name))?),
        };
        fields.push(field);
    }
    let children = d.children.iter().map(from_ast).collect::<Result<Vec<_>, _>>()?;
    Ok(Decl { class, name, parent: d.parent.clone(), def: d.datablock, fields, children })
}

/// The object a top-level statement expression declares (`new ...;`,
/// `%x = new ...;`), if any.
pub fn declared(e: &Expr) -> Option<&ObjectDecl> {
    match e {
        Expr::New(d) => Some(d),
        Expr::Assign(_, rhs) => declared(rhs),
        _ => None,
    }
}

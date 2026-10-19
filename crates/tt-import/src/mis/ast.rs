//! The statement tree of a parsed mission file ([`super::parser`]).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    Xor,
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    BitNot,
}

/// Separator between two pieces of a concatenation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sep {
    /// `@`
    Cat,
    /// `SPC`, `TAB`, `NL` or another single character.
    Char(u8),
    /// `,` inside an array index (stored as `_`).
    Comma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    Function,
    Method,
    Parent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Str(String),
    Tagged(String),
    Ident(String),
    Uint(u32),
    Float(f64),
    Var(String),
    ArrayVar(String, Vec<Expr>),
    Field { obj: Box<Expr>, name: String, index: Vec<Expr> },
    Assign(Box<Expr>, Box<Expr>),
    /// `lhs op= rhs` (also `++`/`--`).
    AssignOp(BinOp, Box<Expr>, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Concat(Box<Expr>, Sep, Box<Expr>),
    StrEq(Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call { kind: CallKind, ns: Option<String>, name: String, args: Vec<Expr> },
    New(Box<ObjectDecl>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectDecl {
    pub datablock: bool,
    pub class: Expr,
    pub name: Expr,
    pub parent: Option<String>,
    pub args: Vec<Expr>,
    pub fields: Vec<Field>,
    pub children: Vec<ObjectDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub index: Vec<Expr>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Expr(Expr),
    Return(Option<Expr>),
    Break,
    Continue,
    If { cond: Expr, then: Vec<Stmt>, els: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    DoWhile { body: Vec<Stmt>, cond: Expr },
    For { init: Option<Expr>, cond: Expr, step: Vec<Expr>, body: Vec<Stmt> },
    Function { ns: Option<String>, name: String, package: Option<String>, args: Vec<String>, body: Vec<Stmt> },
    Comment(String),
}

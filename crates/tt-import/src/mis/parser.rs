
use crate::mis::ast::*;
use crate::mis::lexer::{tokenize, Tok, Token};

pub fn parse(src: &str) -> Result<Vec<Stmt>, String> {
    let mut p = Parser { toks: tokenize(src)?, pos: 0 };
    let mut out = Vec::new();
    while p.peek() != &Tok::Eof {
        p.top_level(&mut out)?;
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

type R<T> = Result<T, String>;

fn stmt(kind: StmtKind) -> Stmt {
    Stmt { kind }
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err<T>(&self, msg: &str) -> R<T> {
        Err(format!("line {}: {msg}, found {:?}", self.toks[self.pos].line, self.peek()))
    }

    fn is(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sym(x) if *x == s)
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> R<()> {
        if self.eat(s) {
            Ok(())
        } else {
            self.err(&format!("expected `{s}`"))
        }
    }

    fn ident(&mut self) -> R<String> {
        match self.next() {
            Tok::Ident(s) => Ok(s),
            // Keywords are valid field / function names in some positions.
            Tok::Sym(s) if s.chars().all(|c| c.is_ascii_alphabetic()) => Ok(s.to_string()),
            _ => {
                self.pos -= 1;
                self.err("expected identifier")
            }
        }
    }

    // ---- statements ----------------------------------------------------

    fn top_level(&mut self, out: &mut Vec<Stmt>) -> R<()> {
        if self.eat("package") {
            let name = self.ident()?;
            self.expect("{")?;
            while !self.eat("}") {
                let f = self.function(Some(name.clone()))?;
                out.push(f);
            }
            self.expect(";")?;
            return Ok(());
        }
        if self.is("function") {
            let f = self.function(None)?;
            out.push(f);
            return Ok(());
        }
        let s = self.statement()?;
        out.push(s);
        Ok(())
    }

    fn function(&mut self, package: Option<String>) -> R<Stmt> {
        self.expect("function")?;
        let mut name = self.ident()?;
        let mut ns = None;
        if self.eat("::") {
            ns = Some(name);
            name = self.ident()?;
        }
        self.expect("(")?;
        let mut args = Vec::new();
        while !self.eat(")") {
            match self.next() {
                Tok::Var(v) => args.push(v),
                _ => return self.err("expected argument"),
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        let body = self.block()?;
        Ok(stmt(StmtKind::Function { ns, name, package, args, body }))
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.expect("{")?;
        let mut out = Vec::new();
        while !self.eat("}") {
            out.push(self.statement()?);
        }
        Ok(out)
    }

    /// Statement or `{ ... }` block.
    fn body(&mut self) -> R<Vec<Stmt>> {
        if self.is("{") {
            self.block()
        } else {
            Ok(vec![self.statement()?])
        }
    }

    fn statement(&mut self) -> R<Stmt> {
        let kind = match self.peek().clone() {
            Tok::Sym("if") => {
                self.next();
                self.expect("(")?;
                let cond = self.expr()?;
                self.expect(")")?;
                let then = self.body()?;
                let els = if self.eat("else") { self.body()? } else { Vec::new() };
                StmtKind::If { cond, then, els }
            }
            Tok::Sym("while") => {
                self.next();
                self.expect("(")?;
                let cond = self.expr()?;
                self.expect(")")?;
                let body = self.body()?;
                StmtKind::While { cond, body }
            }
            Tok::Sym("do") => {
                self.next();
                let body = self.body()?;
                self.expect("while")?;
                self.expect("(")?;
                let cond = self.expr()?;
                self.expect(")")?;
                self.expect(";")?;
                StmtKind::DoWhile { body, cond }
            }
            Tok::Sym("for") => {
                self.next();
                self.expect("(")?;
                let init = if self.is(";") { None } else { Some(self.expr()?) };
                self.expect(";")?;
                let cond = self.expr()?;
                self.expect(";")?;
                let mut step = Vec::new();
                if !self.is(")") {
                    step.push(self.expr()?);
                    while self.eat(",") {
                        step.push(self.expr()?);
                    }
                }
                self.expect(")")?;
                let body = self.body()?;
                StmtKind::For { init, cond, step, body }
            }
            Tok::Sym(kw @ ("switch" | "switch$")) => {
                self.next();
                return self.switch(kw == "switch$");
            }
            Tok::Sym("return") => {
                self.next();
                let v = if self.is(";") { None } else { Some(self.expr()?) };
                self.expect(";")?;
                StmtKind::Return(v)
            }
            Tok::Sym("break") => {
                self.next();
                self.expect(";")?;
                StmtKind::Break
            }
            Tok::Sym("continue") => {
                self.next();
                self.expect(";")?;
                StmtKind::Continue
            }
            Tok::Sym("datablock") => {
                let e = self.object_decl()?;
                self.expect(";")?;
                StmtKind::Expr(e)
            }
            _ => {
                let e = self.expr()?;
                self.expect(";")?;
                StmtKind::Expr(e)
            }
        };
        Ok(stmt(kind))
    }

    /// `switch` is sugar for an if/else chain re-evaluating the operand.
    fn switch(&mut self, strings: bool) -> R<Stmt> {
        self.expect("(")?;
        let value = self.expr()?;
        self.expect(")")?;
        self.expect("{")?;
        let mut cases: Vec<(Expr, Vec<Stmt>)> = Vec::new();
        let mut default = Vec::new();
        while !self.eat("}") {
            if self.eat("default") {
                self.expect(":")?;
                while !self.is("case") && !self.is("default") && !self.is("}") {
                    default.push(self.statement()?);
                }
                continue;
            }
            self.expect("case")?;
            let mut test: Option<Expr> = None;
            loop {
                let v = self.ternary()?;
                let t = if strings {
                    Expr::StrEq(Box::new(value.clone()), Box::new(v))
                } else {
                    Expr::Binary(BinOp::Eq, Box::new(value.clone()), Box::new(v))
                };
                test = Some(match test {
                    None => t,
                    Some(prev) => Expr::Binary(BinOp::Or, Box::new(prev), Box::new(t)),
                });
                if !self.eat("or") {
                    break;
                }
            }
            self.expect(":")?;
            let mut body = Vec::new();
            while !self.is("case") && !self.is("default") && !self.is("}") {
                body.push(self.statement()?);
            }
            cases.push((test.unwrap(), body));
        }
        let mut els = default;
        for (cond, then) in cases.into_iter().rev() {
            els = vec![stmt(StmtKind::If { cond, then, els })];
        }
        Ok(els.pop().unwrap_or_else(|| stmt(StmtKind::Expr(value))))
    }

    // ---- expressions ---------------------------------------------------

    pub fn expr(&mut self) -> R<Expr> {
        let lhs = self.ternary()?;
        const ASSIGN_OPS: &[(&str, BinOp)] = &[
            ("+=", BinOp::Add),
            ("-=", BinOp::Sub),
            ("*=", BinOp::Mul),
            ("/=", BinOp::Div),
            ("%=", BinOp::Mod),
            ("&=", BinOp::BitAnd),
            ("|=", BinOp::BitOr),
            ("^=", BinOp::Xor),
            ("<<=", BinOp::Shl),
            (">>=", BinOp::Shr),
        ];
        if self.is("=") {
            self.check_lvalue(&lhs)?;
            self.next();
            let rhs = self.expr()?;
            return Ok(Expr::Assign(Box::new(lhs), Box::new(rhs)));
        }
        for (sym, op) in ASSIGN_OPS {
            if self.is(sym) {
                self.check_lvalue(&lhs)?;
                self.next();
                let rhs = self.expr()?;
                return Ok(Expr::AssignOp(*op, Box::new(lhs), Box::new(rhs)));
            }
        }
        Ok(lhs)
    }

    fn check_lvalue(&self, e: &Expr) -> R<()> {
        match e {
            Expr::Var(_) | Expr::ArrayVar(..) | Expr::Field { .. } => Ok(()),
            _ => self.err("assignment to a non-variable"),
        }
    }

    fn ternary(&mut self) -> R<Expr> {
        let c = self.binary(0)?;
        if self.eat("?") {
            let t = self.expr()?;
            self.expect(":")?;
            let f = self.ternary()?;
            return Ok(Expr::Ternary(Box::new(c), Box::new(t), Box::new(f)));
        }
        Ok(c)
    }

    /// Binary operator levels, loosest first.
    fn binary(&mut self, level: usize) -> R<Expr> {
        const LEVELS: &[&[&str]] = &[
            &["||"],
            &["&&"],
            &["|"],
            &["^"],
            &["&"],
            &["==", "!=", "$=", "!$="],
            &["<", "<=", ">", ">="],
            &["@", "SPC", "TAB", "NL"],
            &["<<", ">>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        while let Some(op) = LEVELS[level].iter().find(|s| self.is(s)).copied() {
            self.next();
            let rhs = self.binary(level + 1)?;
            let (l, r) = (Box::new(lhs), Box::new(rhs));
            lhs = match op {
                "$=" => Expr::StrEq(l, r),
                "!$=" => Expr::Unary(UnOp::Not, Box::new(Expr::StrEq(l, r))),
                "@" => Expr::Concat(l, Sep::Cat, r),
                "SPC" => Expr::Concat(l, Sep::Char(b' '), r),
                "TAB" => Expr::Concat(l, Sep::Char(b'\t'), r),
                "NL" => Expr::Concat(l, Sep::Char(b'\n'), r),
                _ => {
                    let op = match op {
                        "||" => BinOp::Or,
                        "&&" => BinOp::And,
                        "|" => BinOp::BitOr,
                        "^" => BinOp::Xor,
                        "&" => BinOp::BitAnd,
                        "==" => BinOp::Eq,
                        "!=" => BinOp::Ne,
                        "<" => BinOp::Lt,
                        "<=" => BinOp::Le,
                        ">" => BinOp::Gt,
                        ">=" => BinOp::Ge,
                        "<<" => BinOp::Shl,
                        ">>" => BinOp::Shr,
                        "+" => BinOp::Add,
                        "-" => BinOp::Sub,
                        "*" => BinOp::Mul,
                        "/" => BinOp::Div,
                        _ => BinOp::Mod,
                    };
                    Expr::Binary(op, l, r)
                }
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> R<Expr> {
        let op = if self.eat("!") {
            UnOp::Not
        } else if self.eat("-") {
            UnOp::Neg
        } else if self.eat("~") {
            UnOp::BitNot
        } else {
            return self.postfix();
        };
        let e = self.unary()?;
        Ok(Expr::Unary(op, Box::new(e)))
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat(".") {
                let name = self.ident()?;
                if self.eat("(") {
                    let mut args = vec![e];
                    args.extend(self.args(")")?);
                    e = Expr::Call { kind: CallKind::Method, ns: None, name, args };
                } else if self.eat("[") {
                    let index = self.args("]")?;
                    e = Expr::Field { obj: Box::new(e), name, index };
                } else {
                    e = Expr::Field { obj: Box::new(e), name, index: Vec::new() };
                }
            } else if self.is("++") || self.is("--") {
                let op = if self.next() == Tok::Sym("++") { BinOp::Add } else { BinOp::Sub };
                self.check_lvalue(&e)?;
                e = Expr::AssignOp(op, Box::new(e), Box::new(Expr::Float(1.0)));
            } else {
                return Ok(e);
            }
        }
    }

    fn args(&mut self, close: &str) -> R<Vec<Expr>> {
        let mut v = Vec::new();
        while !self.eat(close) {
            v.push(self.expr()?);
            if !self.is(close) {
                self.expect(",")?;
            }
        }
        Ok(v)
    }

    fn primary(&mut self) -> R<Expr> {
        Ok(match self.next() {
            Tok::Int(n) => Expr::Uint(n),
            Tok::Float(f) => Expr::Float(f),
            Tok::Str(s) => Expr::Str(s),
            Tok::Tagged(s) => Expr::Tagged(s),
            Tok::Sym("true") => Expr::Uint(1),
            Tok::Sym("false") => Expr::Uint(0),
            Tok::Var(v) => {
                if self.eat("[") {
                    let idx = self.args("]")?;
                    Expr::ArrayVar(v, idx)
                } else {
                    Expr::Var(v)
                }
            }
            Tok::Sym("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                e
            }
            Tok::Sym("new") | Tok::Sym("datablock") => {
                self.pos -= 1;
                self.object_decl()?
            }
            Tok::Ident(name) => {
                if self.eat("::") {
                    let f = self.ident()?;
                    self.expect("(")?;
                    let args = self.args(")")?;
                    let kind = if name.eq_ignore_ascii_case("Parent") { CallKind::Parent } else { CallKind::Function };
                    Expr::Call { kind, ns: Some(name), name: f, args }
                } else if self.eat("(") {
                    let args = self.args(")")?;
                    Expr::Call { kind: CallKind::Function, ns: None, name, args }
                } else {
                    Expr::Ident(name)
                }
            }
            _ => {
                self.pos -= 1;
                return self.err("expected expression");
            }
        })
    }

    fn object_decl(&mut self) -> R<Expr> {
        let datablock = self.next() == Tok::Sym("datablock");
        let class = if self.eat("(") {
            let e = self.expr()?;
            self.expect(")")?;
            e
        } else {
            Expr::Ident(self.ident()?)
        };
        self.expect("(")?;
        let name = if self.is(")") || self.is(":") || self.is(",") { Expr::Str(String::new()) } else { self.ternary()? };
        let parent = if self.eat(":") { Some(self.ident()?) } else { None };
        let mut args = Vec::new();
        while self.eat(",") {
            args.push(self.expr()?);
        }
        self.expect(")")?;
        let mut decl = ObjectDecl { datablock, class, name, parent, args, fields: Vec::new(), children: Vec::new() };
        if self.eat("{") {
            while !self.eat("}") {
                if (self.is("new") || self.is("datablock")) && self.peek_at(1) != &Tok::Sym("=") {
                    match self.object_decl()? {
                        Expr::New(child) => decl.children.push(*child),
                        _ => unreachable!(),
                    }
                    self.expect(";")?;
                    continue;
                }
                let name = self.ident()?;
                let index = if self.eat("[") { self.args("]")? } else { Vec::new() };
                self.expect("=")?;
                let value = self.expr()?;
                self.expect(";")?;
                decl.fields.push(Field { name, index, value });
            }
        }
        Ok(Expr::New(Box::new(decl)))
    }
}

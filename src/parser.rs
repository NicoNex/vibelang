//! Parser. Layout is column-based: declarations start at column 0, match arms
//! belong to the match whose first `|` sits in the same column.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{Tok, Token};

pub struct Parser {
    pub toks: Vec<Token>,
    pub i: usize,
    /// Inside (), [] or {} newlines are insignificant.
    pub depth: usize,
    /// The name on the `mod` line, stamped onto every declaration below it.
    pub home: String,
}

type P<T> = Result<T, Diag>;

/// The name `a ; b` binds its left side to. Not writable: an identifier starts
/// with a lower-case letter or `_`, never with `;`.
pub const SEQ: &str = ";seq";

pub fn parse(toks: Vec<Token>) -> P<Module> {
    Parser {
        toks,
        i: 0,
        depth: 0,
        home: String::new(),
    }
    .module()
}

impl Parser {
    // ---- token helpers ----

    fn cur(&self) -> &Token {
        &self.toks[self.i.min(self.toks.len() - 1)]
    }
    fn span(&self) -> Span {
        self.cur().span
    }
    fn at_newline(&self) -> bool {
        matches!(self.cur().tok, Tok::Newline)
    }
    fn at_eof(&self) -> bool {
        matches!(self.cur().tok, Tok::Eof)
    }

    /// Advance past newlines that are insignificant at the current nesting depth.
    fn sync(&mut self) {
        if self.depth > 0 {
            while self.at_newline() {
                self.i += 1;
            }
        }
    }
    fn peek(&mut self) -> Tok {
        self.sync();
        self.cur().tok.clone()
    }
    fn peek_at(&self, n: usize) -> Tok {
        let mut j = self.i;
        let mut k = 0;
        loop {
            if j >= self.toks.len() {
                return Tok::Eof;
            }
            if self.depth > 0 && matches!(self.toks[j].tok, Tok::Newline) {
                j += 1;
                continue;
            }
            if k == n {
                return self.toks[j].tok.clone();
            }
            k += 1;
            j += 1;
        }
    }
    fn bump(&mut self) -> Token {
        self.sync();
        let t = self.cur().clone();
        if !self.at_eof() {
            self.i += 1;
        }
        t
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        self.sync();
        if self.cur().is_sym(s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    /// `c` in `ext c` / `exp c` is a keyword in that one position only. Making
    /// it a global keyword would take a single-letter name that §2.3's grammar
    /// allows and §2.2's keyword set does not claim.
    fn eat_c(&mut self) -> bool {
        self.sync();
        if matches!(&self.cur().tok, Tok::Name(n) if n == "c") {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, s: &str) -> bool {
        self.sync();
        if self.cur().is_kw(s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, s: &str) -> bool {
        self.sync();
        if matches!(&self.cur().tok, Tok::Name(n) if n == s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect_sym(&mut self, s: &str) -> P<Span> {
        let sp = self.span();
        if self.eat_sym(s) {
            Ok(sp)
        } else {
            Err(self.err(
                "parse.expected",
                &format!("expected `{}`, found {}", s, self.describe()),
            ))
        }
    }
    fn describe(&self) -> String {
        match &self.cur().tok {
            Tok::Int(n) => format!("`{}`", n),
            Tok::Float(x) => format!("`{}`", x),
            Tok::Str(_) => "a string literal".into(),
            Tok::Char(_) => "a character literal".into(),
            Tok::Name(n) | Tok::Ctor(n) => format!("`{}`", n),
            Tok::Kw(k) => format!("keyword `{}`", k),
            Tok::Sym(s) => format!("`{}`", s),
            Tok::Newline => "end of line".into(),
            Tok::Eof => "end of file".into(),
        }
    }
    fn err(&self, code: &str, msg: &str) -> Diag {
        Diag::error(self.span(), code, msg)
    }
    fn name(&mut self) -> P<(String, Span)> {
        self.sync();
        let t = self.cur().clone();
        match t.tok {
            Tok::Name(n) => {
                self.i += 1;
                Ok((n, t.span))
            }
            _ => Err(self.err(
                "parse.name",
                &format!("expected a name, found {}", self.describe()),
            )),
        }
    }
    /// A capitalised name, with its module qualifier if it has one. `Json.Value`
    /// is one name — the same spelling the flattened program uses for it — so
    /// types, patterns and constructor expressions all read a qualifier without
    /// any of them knowing what a module is. A lower-case name after the dot is
    /// a field access and is left alone.
    fn ctor_name(&mut self) -> (String, Span) {
        let t = self.cur().clone();
        let Tok::Ctor(n) = t.tok.clone() else {
            return (String::new(), t.span);
        };
        self.i += 1;
        if self.cur().is_sym(".") {
            if let Tok::Ctor(c) = self
                .toks
                .get(self.i + 1)
                .map(|t| t.tok.clone())
                .unwrap_or(Tok::Eof)
            {
                self.i += 2;
                return (format!("{n}.{c}"), t.span);
            }
        }
        (n, t.span)
    }

    fn ctor(&mut self) -> P<(String, Span)> {
        self.sync();
        let t = self.cur().clone();
        match t.tok {
            Tok::Ctor(_) => {
                let (n, sp) = self.ctor_name();
                Ok((n, sp))
            }
            _ => Err(self.err(
                "parse.ctor",
                &format!("expected a capitalised name, found {}", self.describe()),
            )),
        }
    }
    fn skip_newlines(&mut self) {
        while self.at_newline() {
            self.i += 1;
        }
    }
    fn end_of_line(&mut self) -> P<()> {
        if self.at_newline() || self.at_eof() {
            self.skip_newlines();
            Ok(())
        } else {
            Err(self.err(
                "parse.trailing",
                &format!("unexpected {} at end of declaration", self.describe()),
            ))
        }
    }

    // ---- module ----

    fn module(&mut self) -> P<Module> {
        self.skip_newlines();
        let span = self.span();
        if !self.eat_kw("mod") {
            return Err(self
                .err("parse.mod", "a module must start with `mod <Name>`")
                .with_fix("add `mod MyModule` as the first line"));
        }
        let (name, _) = self.ctor()?;
        self.home = name.clone();
        self.end_of_line()?;
        let mut decls = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_eof() {
                break;
            }
            decls.push(self.decl()?);
        }
        Ok(Module { name, decls, span })
    }

    fn decl(&mut self) -> P<Decl> {
        if self.cur().is_kw("type") {
            return Ok(Decl::Type(self.typedecl()?));
        }
        if self.cur().is_kw("ext") {
            return Ok(Decl::Ext(self.extblock()?));
        }
        if self.cur().is_kw("exp") {
            return self.expdecl();
        }
        let ghost = self.eat_kw("ghost");
        Ok(Decl::Fun(self.fundecl(ghost)?))
    }

    fn expdecl(&mut self) -> P<Decl> {
        let span = self.span();
        self.eat_kw("exp");
        if !self.eat_c() {
            return Err(self.err("parse.exp", "expected `exp c <name>, <name>`"));
        }
        let mut names = Vec::new();
        loop {
            names.push(self.name()?.0);
            if !self.eat_sym(",") {
                break;
            }
        }
        self.end_of_line()?;
        Ok(Decl::Exp(names, span))
    }

    // ---- types ----

    fn typedecl(&mut self) -> P<TypeDecl> {
        let span = self.span();
        self.eat_kw("type");
        let (name, _) = self.ctor()?;
        let mut params = Vec::new();
        while let Tok::Name(_) = self.peek() {
            params.push(self.name()?.0);
        }
        if !self.eat_sym("=") {
            // `type Window` inside an `ext c` block: opaque C type.
            self.end_of_line()?;
            return Ok(TypeDecl {
                name,
                home: self.home.clone(),
                params,
                body: TypeBody::Opaque,
                span,
            });
        }
        let body = if self.cur().is_sym("{") {
            TypeBody::Record(self.record_def()?)
        } else {
            let mut vs = Vec::new();
            loop {
                let (cn, _) = self.ctor()?;
                let mut args = Vec::new();
                while self.starts_type_atom() {
                    args.push(self.type_atom()?);
                }
                vs.push(VariantDef { name: cn, args });
                if !self.eat_sym("|") {
                    break;
                }
            }
            TypeBody::Variants(vs)
        };
        self.end_of_line()?;
        Ok(TypeDecl {
            name,
            home: self.home.clone(),
            params,
            body,
            span,
        })
    }

    fn record_def(&mut self) -> P<RecordDef> {
        self.expect_sym("{")?;
        self.depth += 1;
        let mut fields = Vec::new();
        let mut refines = Vec::new();
        loop {
            if self.cur().is_sym("}") {
                break;
            }
            // A field is `name : type`; anything else is a refinement predicate.
            let is_field =
                matches!(self.peek(), Tok::Name(_)) && matches!(self.peek_at(1), Tok::Sym(":"));
            if is_field {
                let (n, _) = self.name()?;
                self.expect_sym(":")?;
                fields.push((n, self.ty()?));
            } else {
                refines.push(self.expr()?);
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        self.depth -= 1;
        self.expect_sym("}")?;
        Ok(RecordDef { fields, refines })
    }

    fn starts_type_atom(&mut self) -> bool {
        matches!(self.peek(), Tok::Ctor(_) | Tok::Name(_))
            || self.cur().is_sym("(")
            || self.cur().is_sym("&")
            || self.cur().is_sym("E!")
    }

    pub fn ty(&mut self) -> P<Ty> {
        let lhs = self.ty_app()?;
        if self.eat_sym("->") {
            let rhs = self.ty()?;
            return Ok(Ty::Fun(Box::new(lhs), Box::new(rhs)));
        }
        Ok(lhs)
    }

    fn ty_app(&mut self) -> P<Ty> {
        if self.eat_sym("&") {
            return Ok(Ty::Ref(Box::new(self.ty_app()?)));
        }
        if self.eat_sym("E!") {
            return Ok(Ty::Eff(Box::new(self.ty_app()?)));
        }
        self.sync();
        if let Tok::Ctor(_) = self.cur().tok {
            let (n, _) = self.ctor()?;
            let mut args = Vec::new();
            while self.starts_type_atom() {
                args.push(self.type_atom()?);
            }
            return Ok(Ty::Con(n, args));
        }
        self.type_atom()
    }

    fn type_atom(&mut self) -> P<Ty> {
        if self.eat_sym("&") {
            return Ok(Ty::Ref(Box::new(self.type_atom()?)));
        }
        if self.eat_sym("E!") {
            return Ok(Ty::Eff(Box::new(self.type_atom()?)));
        }
        self.sync();
        match self.cur().tok.clone() {
            Tok::Ctor(_) => {
                let (n, _) = self.ctor_name();
                Ok(Ty::Con(n, vec![]))
            }
            Tok::Name(n) => {
                self.i += 1;
                Ok(Ty::Var(n))
            }
            Tok::Sym("(") => {
                self.i += 1;
                self.depth += 1;
                if self.cur().is_sym(")") {
                    self.depth -= 1;
                    self.i += 1;
                    return Ok(Ty::unit());
                }
                let mut parts = vec![self.ty()?];
                while self.eat_sym(",") {
                    parts.push(self.ty()?);
                }
                self.depth -= 1;
                self.expect_sym(")")?;
                Ok(if parts.len() == 1 {
                    parts.pop().unwrap()
                } else {
                    Ty::Tuple(parts)
                })
            }
            _ => Err(self.err(
                "parse.type",
                &format!("expected a type, found {}", self.describe()),
            )),
        }
    }

    // ---- ext / exp ----

    fn extblock(&mut self) -> P<ExtBlock> {
        let span = self.span();
        self.eat_kw("ext");
        if !self.eat_c() {
            return Err(self
                .err("parse.ext", "expected `ext c \"header.h\"`")
                .with_fix("write `ext c \"stdio.h\"`"));
        }
        let header = match self.bump().tok {
            Tok::Str(s) => s,
            _ => return Err(self.err("parse.ext", "expected a header name in quotes")),
        };
        let mut links = Vec::new();
        let mut pkgs = Vec::new();
        loop {
            if self.eat_word("link") {
                match self.bump().tok {
                    Tok::Str(s) => links.push(s),
                    _ => return Err(self.err("parse.ext", "expected a library name after `link`")),
                }
            } else if self.eat_word("pkg") {
                match self.bump().tok {
                    Tok::Str(s) => pkgs.push(s),
                    _ => return Err(self.err("parse.ext", "expected a package name after `pkg`")),
                }
            } else {
                break;
            }
        }
        self.end_of_line()?;
        let mut sigs = Vec::new();
        let mut types = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat_kw("end") {
                break;
            }
            if self.at_eof() {
                return Err(self
                    .err("parse.ext", "expected `end` to close the `ext c` block")
                    .with_fix("add `end` after the last signature"));
            }
            if self.cur().is_kw("type") {
                types.push(self.typedecl()?);
                continue;
            }
            let (name, sp) = self.name()?;
            let symbol = if self.eat_sym("=") {
                match self.bump().tok {
                    Tok::Str(s) => s,
                    _ => return Err(self.err("parse.ext", "expected the C symbol name in quotes")),
                }
            } else {
                name.clone()
            };
            self.expect_sym(":")?;
            let ty = self.ty()?;
            let mut refines = Vec::new();
            while self.eat_sym(",") {
                refines.push(self.expr()?);
            }
            self.end_of_line()?;
            sigs.push(ExtSig {
                name,
                symbol,
                ty,
                refines,
                span: sp,
            });
        }
        Ok(ExtBlock {
            header,
            links,
            pkgs,
            sigs,
            types,
            span,
        })
    }

    // ---- functions ----

    fn fundecl(&mut self, ghost: bool) -> P<FunDecl> {
        let (name, span) = self.name()?;
        let mut params = Vec::new();
        loop {
            self.sync();
            match self.cur().tok.clone() {
                Tok::Name(n) => {
                    let sp = self.span();
                    self.i += 1;
                    params.push(Param {
                        names: vec![n],
                        ty: None,
                        refines: vec![],
                        span: sp,
                    });
                }
                Tok::Sym("(") => {
                    let sp = self.span();
                    self.i += 1;
                    self.depth += 1;
                    let mut names = Vec::new();
                    while let Tok::Name(_) = self.peek() {
                        names.push(self.name()?.0);
                    }
                    if names.is_empty() {
                        return Err(self.err("parse.param", "expected parameter names before `:`"));
                    }
                    self.expect_sym(":")?;
                    let ty = self.ty()?;
                    let mut refines = Vec::new();
                    while self.eat_sym(",") {
                        refines.push(self.expr()?);
                    }
                    self.depth -= 1;
                    self.expect_sym(")")?;
                    params.push(Param {
                        names,
                        ty: Some(ty),
                        refines,
                        span: sp,
                    });
                }
                _ => break,
            }
        }
        let ret = if self.eat_sym(":") {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect_sym("=")?;
        let body = self.body()?;
        let measure = if self.eat_sym("%") {
            Some(self.expr()?)
        } else {
            None
        };
        self.end_of_line()?;
        Ok(FunDecl {
            name,
            home: self.home.clone(),
            ghost,
            params,
            ret,
            body,
            measure,
            span,
        })
    }

    /// A function body: a chain of `<-` binds / `let ... in` followed by an expression.
    /// A body: `name <- value ;` binds, `let name = value in` binds, `;`
    /// sequences, and anything else is an expression.
    ///
    /// `a ; b` is `_ <- a ; b`: the value is still bound, to a name nothing can
    /// read, so sequencing needs no second rule in the effect checker.
    fn body(&mut self) -> P<Expr> {
        self.skip_newlines();
        let span = self.span();
        if matches!(self.peek(), Tok::Name(_)) && matches!(self.peek_at(1), Tok::Sym("<-")) {
            let (n, _) = self.name()?;
            self.expect_sym("<-")?;
            let val = self.expr()?;
            if !self.eat_sym(";") {
                return Err(self
                    .err("parse.bind", "expected `;` after a `<-` binding")
                    .with_fix("write `name <- value ;` and continue on the next line"));
            }
            let rest = self.body()?;
            return Ok(Expr::new(
                ExprKind::Bind(n, Box::new(val), Box::new(rest)),
                span,
            ));
        }
        if self.cur().is_kw("let") {
            self.i += 1;
            let (n, _) = self.name()?;
            self.expect_sym("=")?;
            let val = self.expr()?;
            if !self.eat_kw("in") {
                return Err(self.err("parse.let", "expected `in` after a `let` binding"));
            }
            let rest = self.body()?;
            return Ok(Expr::new(
                ExprKind::Let(n, Box::new(val), Box::new(rest)),
                span,
            ));
        }
        let e = self.expr()?;
        if self.eat_sym(";") {
            let rest = self.body()?;
            return Ok(Expr::new(
                ExprKind::Bind(SEQ.into(), Box::new(e), Box::new(rest)),
                span,
            ));
        }
        Ok(e)
    }

    // ---- expressions ----

    pub fn expr(&mut self) -> P<Expr> {
        self.pipe()
    }

    fn pipe(&mut self) -> P<Expr> {
        let mut lhs = self.binop(1)?;
        while self.cur().is_sym("|>") {
            let span = self.span();
            self.i += 1;
            let rhs = self.binop(1)?;
            lhs = match rhs.kind {
                // `x |> f a` is `f a x`: the pipe feeds the last argument.
                ExprKind::App(f, mut args) => {
                    args.push(lhs);
                    Expr::new(ExprKind::App(f, args), span)
                }
                _ => Expr::new(ExprKind::App(Box::new(rhs), vec![lhs]), span),
            };
        }
        Ok(lhs)
    }

    fn binop(&mut self, min_prec: u8) -> P<Expr> {
        let mut lhs = self.unary()?;
        loop {
            self.sync();
            let op = match &self.cur().tok {
                Tok::Sym(s) => *s,
                _ => break,
            };
            let prec = match op {
                "||" => 1,
                "&&" => 2,
                "==" | "!=" | "<" | "<=" | ">" | ">=" => 3,
                "++" => 4,
                "+" | "-" => 5,
                "*" | "/" => 6,
                _ => break,
            };
            if prec < min_prec {
                break;
            }
            let span = self.span();
            self.i += 1;
            let rhs = self.binop(prec + 1)?;
            lhs = Expr::new(
                ExprKind::Binop(op.to_string(), Box::new(lhs), Box::new(rhs)),
                span,
            );
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> P<Expr> {
        self.sync();
        let span = self.span();
        if self.cur().is_sym("-") {
            self.i += 1;
            return Ok(Expr::new(ExprKind::Neg(Box::new(self.unary()?)), span));
        }
        if self.cur().is_sym("!") {
            self.i += 1;
            return Ok(Expr::new(ExprKind::Not(Box::new(self.unary()?)), span));
        }
        self.app()
    }

    fn app(&mut self) -> P<Expr> {
        let head = self.postfix()?;
        let mut args = Vec::new();
        while self.starts_atom() {
            args.push(self.postfix()?);
        }
        if args.is_empty() {
            Ok(head)
        } else {
            let span = head.span;
            Ok(Expr::new(ExprKind::App(Box::new(head), args), span))
        }
    }

    fn postfix(&mut self) -> P<Expr> {
        self.sync();
        // `&x` binds like an atom, so it can appear as a bare argument.
        if self.cur().is_sym("&") {
            let span = self.span();
            self.i += 1;
            return Ok(Expr::new(ExprKind::Borrow(Box::new(self.postfix()?)), span));
        }
        let mut e = self.atom()?;
        loop {
            // `.` binds tighter than application, and must not be preceded by a space.
            if self.cur().is_sym(".") {
                let span = self.span();
                self.i += 1;
                let (f, _) = self.name()?;
                e = Expr::new(ExprKind::Field(Box::new(e), f), span);
            } else {
                return Ok(e);
            }
        }
    }

    fn starts_atom(&mut self) -> bool {
        if self.depth == 0 && self.at_newline() {
            return false;
        }
        self.sync();
        match &self.cur().tok {
            Tok::Int(_)
            | Tok::Float(_)
            | Tok::Str(_)
            | Tok::Char(_)
            | Tok::Name(_)
            | Tok::Ctor(_) => true,
            Tok::Kw(k) => matches!(*k, "True" | "False" | "arena"),
            Tok::Sym(s) => matches!(*s, "(" | "[" | "{" | "&" | "?" | "\\"),
            _ => false,
        }
    }

    fn atom(&mut self) -> P<Expr> {
        self.sync();
        let span = self.span();
        let t = self.cur().tok.clone();
        match t {
            Tok::Int(n) => {
                self.i += 1;
                Ok(Expr::new(ExprKind::Int(n), span))
            }
            Tok::Float(x) => {
                self.i += 1;
                Ok(Expr::new(ExprKind::Float(x), span))
            }
            Tok::Str(s) => {
                self.i += 1;
                Ok(Expr::new(ExprKind::Str(s), span))
            }
            Tok::Char(c) => {
                self.i += 1;
                Ok(Expr::new(ExprKind::Char(c), span))
            }
            Tok::Name(n) => {
                self.i += 1;
                Ok(Expr::new(ExprKind::Var(n), span))
            }
            Tok::Ctor(_) => {
                let (n, _) = self.ctor_name();
                Ok(Expr::new(ExprKind::Ctor(n), span))
            }
            Tok::Kw("True") => {
                self.i += 1;
                Ok(Expr::new(ExprKind::Bool(true), span))
            }
            Tok::Kw("False") => {
                self.i += 1;
                Ok(Expr::new(ExprKind::Bool(false), span))
            }
            Tok::Kw("let") => {
                self.i += 1;
                let (n, _) = self.name()?;
                self.expect_sym("=")?;
                let val = self.expr()?;
                if !self.eat_kw("in") {
                    return Err(self.err("parse.let", "expected `in` after a `let` binding"));
                }
                let body = self.body()?;
                Ok(Expr::new(
                    ExprKind::Let(n, Box::new(val), Box::new(body)),
                    span,
                ))
            }
            Tok::Kw("arena") => {
                self.i += 1;
                let (n, _) = self.name()?;
                if !self.eat_kw("in") {
                    return Err(self.err("parse.arena", "expected `in` after the arena name"));
                }
                let body = self.body()?;
                Ok(Expr::new(ExprKind::Arena(n, Box::new(body)), span))
            }
            Tok::Sym("\\") => {
                self.i += 1;
                let mut ps = Vec::new();
                while let Tok::Name(_) = self.peek() {
                    ps.push(self.name()?.0);
                }
                self.expect_sym("->")?;
                let body = self.expr()?;
                Ok(Expr::new(ExprKind::Lambda(ps, Box::new(body)), span))
            }
            Tok::Sym("?") => {
                self.i += 1;
                self.match_expr(span)
            }
            Tok::Sym("(") => {
                self.i += 1;
                self.depth += 1;
                if self.cur().is_sym(")") {
                    self.depth -= 1;
                    self.i += 1;
                    return Ok(Expr::new(ExprKind::Unit, span));
                }
                let mut parts = vec![self.expr()?];
                while self.eat_sym(",") {
                    parts.push(self.expr()?);
                }
                self.depth -= 1;
                self.expect_sym(")")?;
                if parts.len() == 1 {
                    let inner = parts.pop().unwrap();
                    // P1: parentheses around something that needs none are not canonical.
                    if matches!(
                        inner.kind,
                        ExprKind::Int(_)
                            | ExprKind::Float(_)
                            | ExprKind::Str(_)
                            | ExprKind::Char(_)
                            | ExprKind::Bool(_)
                            | ExprKind::Var(_)
                            | ExprKind::Ctor(_)
                    ) {
                        return Err(Diag::error(span, "canon.parens", "redundant parentheses")
                            .with_fix("remove the parentheses"));
                    }
                    Ok(inner)
                } else {
                    Ok(Expr::new(ExprKind::Tuple(parts), span))
                }
            }
            Tok::Sym("[") => {
                self.i += 1;
                self.depth += 1;
                let mut items = Vec::new();
                if !self.cur().is_sym("]") {
                    items.push(self.expr()?);
                    while self.eat_sym(",") {
                        items.push(self.expr()?);
                    }
                }
                self.depth -= 1;
                self.expect_sym("]")?;
                Ok(Expr::new(ExprKind::List(items), span))
            }
            Tok::Sym("{") => {
                self.i += 1;
                self.depth += 1;
                let base = if matches!(self.peek(), Tok::Name(_))
                    && matches!(self.peek_at(1), Tok::Kw("with"))
                {
                    let (n, sp) = self.name()?;
                    self.eat_kw("with");
                    Some(Box::new(Expr::new(ExprKind::Var(n), sp)))
                } else {
                    None
                };
                let mut fields = Vec::new();
                if !self.cur().is_sym("}") {
                    loop {
                        let (f, _) = self.name()?;
                        self.expect_sym("=")?;
                        fields.push((f, self.expr()?));
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                }
                self.depth -= 1;
                self.expect_sym("}")?;
                Ok(Expr::new(ExprKind::Record(base, fields), span))
            }
            _ => Err(self.err(
                "parse.expr",
                &format!("expected an expression, found {}", self.describe()),
            )),
        }
    }

    /// `? scrutinee { "|" pattern "->" expr } "end"`.
    ///
    /// `end` is what makes a nested match unambiguous without layout: the inner
    /// one closes before the outer one's next `|` is read, so no arm has to be
    /// aligned with anything.
    fn match_expr(&mut self, span: Span) -> P<Expr> {
        let scrut = self.expr()?;
        if !self.cur().is_sym("|") {
            return Err(self
                .err("parse.match", "a `?` match needs at least one `|` arm")
                .with_fix("add `|_ -> ...`"));
        }
        let mut arms = Vec::new();
        while self.eat_sym("|") {
            let pat = self.pattern()?;
            self.expect_sym("->")?;
            arms.push((pat, self.body()?));
        }
        if !self.eat_kw("end") {
            return Err(self
                .err(
                    "parse.match",
                    "expected another `|` arm or `end` to close the match",
                )
                .with_fix("add `end` after the last arm"));
        }
        Ok(Expr::new(ExprKind::Match(Box::new(scrut), arms), span))
    }

    fn pattern(&mut self) -> P<Pat> {
        self.sync();
        if let Tok::Ctor(_) = self.cur().tok.clone() {
            let (n, _) = self.ctor_name();
            let mut args = Vec::new();
            while self.starts_pattern_atom() {
                args.push(self.pattern_atom()?);
            }
            return Ok(Pat::Ctor(n, args));
        }
        self.pattern_atom()
    }

    fn starts_pattern_atom(&mut self) -> bool {
        self.sync();
        match &self.cur().tok {
            Tok::Int(_)
            | Tok::Float(_)
            | Tok::Str(_)
            | Tok::Char(_)
            | Tok::Name(_)
            | Tok::Ctor(_) => true,
            Tok::Kw(k) => matches!(*k, "True" | "False"),
            Tok::Sym(s) => matches!(*s, "(" | "[" | "_"),
            _ => false,
        }
    }

    fn pattern_atom(&mut self) -> P<Pat> {
        self.sync();
        let t = self.cur().tok.clone();
        match t {
            Tok::Sym("_") => {
                self.i += 1;
                Ok(Pat::Wild)
            }
            Tok::Int(n) => {
                self.i += 1;
                Ok(Pat::Int(n))
            }
            Tok::Float(x) => {
                self.i += 1;
                Ok(Pat::Float(x))
            }
            Tok::Str(s) => {
                self.i += 1;
                Ok(Pat::Str(s))
            }
            Tok::Char(c) => {
                self.i += 1;
                Ok(Pat::Char(c))
            }
            Tok::Kw("True") => {
                self.i += 1;
                Ok(Pat::Bool(true))
            }
            Tok::Kw("False") => {
                self.i += 1;
                Ok(Pat::Bool(false))
            }
            Tok::Name(n) => {
                self.i += 1;
                Ok(Pat::Var(n))
            }
            Tok::Ctor(_) => {
                let (n, _) = self.ctor_name();
                Ok(Pat::Ctor(n, vec![]))
            }
            Tok::Sym("[") => {
                self.i += 1;
                self.depth += 1;
                let mut ps = Vec::new();
                if !self.cur().is_sym("]") {
                    ps.push(self.pattern()?);
                    while self.eat_sym(",") {
                        ps.push(self.pattern()?);
                    }
                }
                self.depth -= 1;
                self.expect_sym("]")?;
                Ok(Pat::List(ps))
            }
            Tok::Sym("(") => {
                self.i += 1;
                self.depth += 1;
                let mut ps = vec![self.pattern()?];
                while self.eat_sym(",") {
                    ps.push(self.pattern()?);
                }
                self.depth -= 1;
                self.expect_sym(")")?;
                Ok(if ps.len() == 1 {
                    ps.pop().unwrap()
                } else {
                    Pat::Tuple(ps)
                })
            }
            _ => Err(self.err(
                "parse.pattern",
                &format!("expected a pattern, found {}", self.describe()),
            )),
        }
    }
}

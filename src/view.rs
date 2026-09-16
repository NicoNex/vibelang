//! Projection views (spec §13.1). The printer reads only the AST, never the
//! source text, so `vibe view` round-tripping a file is a real test of the
//! canonical form (P1).
//!
//! ponytail: layout that the AST cannot carry is recovered from spans, not from
//! new AST nodes — blank lines and the column alignment of adjacent one-line
//! declarations come from `span.line` (see `groups`). Comments are lost: the
//! lexer drops `;;`, so a file with comments does not round-trip. The upgrade
//! path is a trivia list on `Module`, once the AST is allowed to change.
//!
//! ponytail: every `ExprKind` has an arm here on purpose — when a new node
//! lands (`arena`, say) the compiler points at `raw`/`lay` instead of silently
//! printing something that does not re-parse.

use crate::ast::*;
use crate::diag::Diag;
use crate::infer::Checked;
use crate::lexer::{self, Comment};

const WIDTH: usize = 80;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Canon,
    SigOnly,
    Explicit,
    Flow,
    /// The drop table: one line per owned value, at the point it dies.
    Drops,
}

pub fn render(m: &Module, ck: &Checked, mode: Mode) -> String {
    if mode == Mode::Drops {
        return drops(m, ck);
    }
    let mut out = String::new();
    out.push_str(&format!("mod {}\n", m.name));
    for g in groups(&m.decls, mode) {
        out.push('\n');
        decl_group(&mut out, ck, mode, &g);
    }
    out
}

/// The drop table as text, one site a line, in source order. This is a reading
/// tool for work in progress: the backend does not act on it yet
/// (docs/static-drop-roadmap.md).
fn drops(m: &Module, ck: &Checked) -> String {
    let mut out: String = crate::own::drop_points(m, ck)
        .iter()
        .map(|d| match d.when {
            crate::own::DropWhen::ScopeEnd => format!("{}.body: drop {}\n", d.path, d.name),
            crate::own::DropWhen::BackEdge => {
                format!("{}.body: drop {} before the back edge\n", d.path, d.name)
            }
        })
        .collect();
    // The count is the point: it is how much of the program's memory the
    // compiler declines to free because it cannot prove nobody else holds it.
    let n = crate::own::shared_suppressed(m, ck);
    if n > 0 {
        out.push_str(&format!(
            "{n} drop(s) suppressed: the value may alias one the caller owns\n"
        ));
    }
    out
}

// ------------------------------------------------------------------ grouping

/// Declarations that sit on adjacent source lines and print on one line each
/// form an aligned group; every other boundary gets one blank line.
fn groups(decls: &[Decl], mode: Mode) -> Vec<Vec<&Decl>> {
    let mut gs: Vec<Vec<&Decl>> = Vec::new();
    for d in decls {
        let joins = match (gs.last().and_then(|g| g.last()), d) {
            (Some(Decl::Type(p)), Decl::Type(t)) => t.span.line == p.span.line + 1,
            (Some(Decl::Fun(p)), Decl::Fun(f)) => {
                f.span.line == p.span.line + 1 && oneline(p, mode) && oneline(f, mode)
            }
            _ => false,
        };
        if joins {
            gs.last_mut().unwrap().push(d);
        } else {
            gs.push(vec![d]);
        }
    }
    gs
}

/// A function prints on one line when it has parameters, a body that is not a
/// block form, and the whole thing fits.
fn oneline(f: &FunDecl, mode: Mode) -> bool {
    if mode == Mode::SigOnly {
        return true;
    }
    if f.params.is_empty() || f.measure.is_some() {
        return false;
    }
    if matches!(
        f.body.kind,
        ExprKind::Match(..) | ExprKind::Bind(..) | ExprKind::Let(..)
    ) {
        return false;
    }
    head(f, 0, 0, 0).len() + 1 + P.flat(&f.body, 0).len() <= WIDTH
}

fn decl_group(out: &mut String, ck: &Checked, mode: Mode, g: &[&Decl]) {
    match g[0] {
        Decl::Type(_) => {
            let w = g
                .iter()
                .filter_map(|d| match d {
                    Decl::Type(t) => Some(type_head(t).len()),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            for d in g {
                if let Decl::Type(t) = d {
                    out.push_str(&typedecl(t, w));
                }
            }
        }
        Decl::Fun(_) => {
            let fs: Vec<&FunDecl> = g
                .iter()
                .filter_map(|d| match d {
                    Decl::Fun(f) => Some(f),
                    _ => None,
                })
                .collect();
            let (mut nw, mut pw, mut rw) = (0, 0, 0);
            if fs.len() > 1 {
                for f in &fs {
                    nw = nw.max(f.name.len());
                    pw = pw.max(params(f).len());
                    rw = rw.max(f.ret.as_ref().map(|t| ty(t).len()).unwrap_or(0));
                }
            }
            for f in fs {
                fundecl(out, ck, mode, f, nw, pw, rw);
            }
        }
        Decl::Ext(_) => {
            for d in g {
                if let Decl::Ext(e) = d {
                    extblock(out, e);
                }
            }
        }
        Decl::Exp(..) => {
            for d in g {
                if let Decl::Exp(names, _) = d {
                    out.push_str(&format!("exp c {}\n", names.join(", ")));
                }
            }
        }
    }
}

// --------------------------------------------------------------- declarations

fn type_head(t: &TypeDecl) -> String {
    let mut s = t.name.clone();
    for p in &t.params {
        s.push(' ');
        s.push_str(p);
    }
    s
}

fn typedecl(t: &TypeDecl, w: usize) -> String {
    let h = pad(&type_head(t), w);
    match &t.body {
        TypeBody::Opaque => format!("type {}\n", h.trim_end()),
        TypeBody::Record(r) => {
            let mut parts: Vec<String> = r
                .fields
                .iter()
                .map(|(n, ft)| format!("{n}:{}", ty(ft)))
                .collect();
            parts.extend(r.refines.iter().map(|e| P.flat(e, 0)));
            format!("type {h} = {{ {} }}\n", parts.join(", "))
        }
        TypeBody::Variants(vs) => {
            let body: Vec<String> = vs
                .iter()
                .map(|v| {
                    let mut s = v.name.clone();
                    for a in &v.args {
                        s.push(' ');
                        s.push_str(&ty_atom(a));
                    }
                    s
                })
                .collect();
            format!("type {h} = {}\n", body.join(" | "))
        }
    }
}

fn extblock(out: &mut String, e: &ExtBlock) {
    let mut h = format!("ext c \"{}\"", e.header);
    for l in &e.links {
        h.push_str(&format!(" link \"{l}\""));
    }
    for p in &e.pkgs {
        h.push_str(&format!(" pkg \"{p}\""));
    }
    out.push_str(&h);
    out.push('\n');
    for t in &e.types {
        out.push_str("  ");
        out.push_str(&typedecl(t, 0));
    }
    let w = e.sigs.iter().map(|s| sig_head(s).len()).max().unwrap_or(0);
    for s in &e.sigs {
        let mut line = format!("  {} : {}", pad(&sig_head(s), w), ty(&s.ty));
        for r in &s.refines {
            line.push_str(&format!(", {}", P.flat(r, 0)));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("end\n");
}

fn sig_head(s: &ExtSig) -> String {
    if s.symbol == s.name {
        s.name.clone()
    } else {
        format!("{} = \"{}\"", s.name, s.symbol)
    }
}

fn params(f: &FunDecl) -> String {
    let mut parts: Vec<String> = Vec::new();
    for p in &f.params {
        match &p.ty {
            None => parts.push(p.names.join(" ")),
            Some(t) => {
                let mut s = format!("({}:{}", p.names.join(" "), ty(t));
                for r in &p.refines {
                    s.push_str(&format!(", {}", P.flat(r, 0)));
                }
                s.push(')');
                parts.push(s);
            }
        }
    }
    parts.join(" ")
}

/// `name params : ret =`, with each column padded to the group's width.
fn head(f: &FunDecl, nw: usize, pw: usize, rw: usize) -> String {
    let mut s = String::new();
    if f.ghost {
        s.push_str("ghost ");
    }
    s.push_str(&pad(&f.name, nw));
    let ps = params(f);
    if !ps.is_empty() {
        s.push(' ');
        s.push_str(&pad(&ps, pw));
    }
    if let Some(r) = &f.ret {
        s.push_str(" : ");
        s.push_str(&pad(&ty(r), rw));
    }
    s.push_str(" =");
    s
}

fn fundecl(
    out: &mut String,
    ck: &Checked,
    mode: Mode,
    f: &FunDecl,
    nw: usize,
    pw: usize,
    rw: usize,
) {
    if mode == Mode::Explicit {
        for l in explicit_notes(ck, f) {
            out.push_str(&format!(";; {l}\n"));
        }
    }
    let h = head(f, nw, pw, rw);
    if mode == Mode::SigOnly {
        out.push_str(h.trim_end_matches(" =").trim_end());
        out.push('\n');
        return;
    }
    let body = if mode == Mode::Flow {
        flow_body(&f.body, &mut 0)
    } else {
        f.body.clone()
    };
    if oneline(f, mode) && !matches!(body.kind, ExprKind::Let(..)) {
        out.push_str(&format!("{h} {}\n", P.flat(&body, 0)));
    } else {
        out.push_str(&format!("{h}\n  {}\n", P.lay(&body, 2)));
    }
    if let Some(ms) = &f.measure {
        out.push_str(&format!("  %{}\n", P.flat(ms, 10)));
    }
}

/// What the compiler actually knows today: the inferred type, and the fact that
/// refinements and measures are *not* discharged yet (see types.rs).
/// ponytail: no borrow, copy or proof information exists in the bootstrap, so
/// `--explicit` does not invent any. It grows when infer starts recording it.
fn explicit_notes(ck: &Checked, f: &FunDecl) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(s) = ck.sigs.get(&f.name) {
        v.push(format!("{}.{} : {}", f.home, f.name, s.ty.show()));
    }
    for p in &f.params {
        for r in &p.refines {
            v.push(format!("|- {} [checked at run time]", P.flat(r, 0)));
        }
    }
    if let Some(ms) = &f.measure {
        v.push(format!(
            "%{} [measure parsed, not discharged]",
            P.flat(ms, 10)
        ));
    }
    v
}

// -------------------------------------------------------------------- layout

/// The printer is stateless; `P` just namespaces the methods.
struct Printer;
const P: Printer = Printer;

impl Printer {
    /// Print `e` starting at column `col`, breaking lines when it does not fit.
    fn lay(&self, e: &Expr, col: usize) -> String {
        match &e.kind {
            ExprKind::Match(s, arms) => self.lay_match(s, arms, col),
            ExprKind::Bind(n, v, rest) if n == crate::parser::SEQ => {
                format!("{} ;\n{}{}", self.flat(v, 0), sp(col), self.lay(rest, col))
            }
            ExprKind::Bind(n, v, rest) => {
                format!(
                    "{n} <- {} ;\n{}{}",
                    self.flat(v, 0),
                    sp(col),
                    self.lay(rest, col)
                )
            }
            ExprKind::Let(n, v, rest) => {
                format!(
                    "let {n} = {} in\n{}{}",
                    self.flat(v, 0),
                    sp(col),
                    self.lay(rest, col)
                )
            }
            // The block body is indented one step in, as the parser expects.
            ExprKind::Arena(a, body) => {
                format!("arena {a} in\n{}{}", sp(col + 2), self.lay(body, col + 2))
            }
            ExprKind::App(f, args) if !is_pipe(e) => {
                let flat = self.flat(e, 0);
                if col + flat.len() <= WIDTH {
                    return flat;
                }
                // Head and first argument stay together; the rest go one line
                // below, aligned under the first argument.
                let h = self.flat(f, 10);
                let acol = col + h.len() + 1;
                let mut s = format!("{h} {}", self.lay_arg(&args[0], acol));
                if args.len() > 1 {
                    let rest: Vec<String> = args[1..].iter().map(|a| self.flat(a, 10)).collect();
                    s.push_str(&format!("\n{}{}", sp(acol), rest.join(" ")));
                }
                s
            }
            _ => self.flat(e, 0),
        }
    }

    fn lay_arg(&self, e: &Expr, col: usize) -> String {
        if power(&e.kind) < 10 {
            format!("({})", self.lay(e, col + 1))
        } else {
            self.lay(e, col)
        }
    }

    /// `?scrut` with the arms one column to its right; a bare variable keeps its
    /// first arm on the same line.
    fn lay_match(&self, scrut: &Expr, arms: &[(Pat, Expr)], col: usize) -> String {
        let s = self.scrut(scrut);
        let inline = matches!(scrut.kind, ExprKind::Var(_));
        let acol = if inline {
            col + 1 + s.len() + 1
        } else {
            col + 1
        };
        let w = arms.iter().map(|(p, _)| pat(p).len()).max().unwrap_or(0);
        let mut out = format!("?{s}");
        for (i, (p, b)) in arms.iter().enumerate() {
            if i == 0 && inline {
                out.push(' ');
            } else {
                out.push_str(&format!("\n{}", sp(acol)));
            }
            let lead = format!("|{} -> ", pad(&pat(p), w));
            out.push_str(&lead);
            out.push_str(&self.lay(b, acol + lead.len()));
        }
        // `end` closes the match; it is what lets a nested one sit inside an
        // arm without any alignment.
        out.push_str(&format!("\n{}end", sp(col)));
        out
    }

    /// A match scrutinee needs parentheses only when it is not an application
    /// or an atom — `?(n>0)`, but `?split ',' ln` and `?txt |> lines`.
    fn scrut(&self, e: &Expr) -> String {
        let bare = matches!(
            e.kind,
            ExprKind::App(..)
                | ExprKind::Var(_)
                | ExprKind::Ctor(_)
                | ExprKind::Field(..)
                | ExprKind::Tuple(_)
                | ExprKind::List(_)
                | ExprKind::Record(..)
                | ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Str(_)
                | ExprKind::Char(_)
                | ExprKind::Bool(_)
                | ExprKind::Unit
                | ExprKind::Borrow(_)
        );
        let s = self.flat(e, 0);
        if bare {
            s
        } else {
            format!("({s})")
        }
    }

    // ---------------------------------------------------------------- flat

    /// One-line rendering. `need` is the binding power the context demands.
    fn flat(&self, e: &Expr, need: u8) -> String {
        let s = self.raw(e);
        if power(&e.kind) < need {
            format!("({s})")
        } else {
            s
        }
    }

    fn raw(&self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(n) => n.to_string(),
            ExprKind::Float(x) => num(*x),
            ExprKind::Str(s) => quote(s),
            ExprKind::Char(c) => format!("'{}'", esc(*c)),
            ExprKind::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            ExprKind::Unit => "()".into(),
            ExprKind::Var(n) | ExprKind::Ctor(n) => n.clone(),
            ExprKind::App(f, args) if is_pipe(e) => {
                let (last, init) = args.split_last().expect("a pipe feeds one argument");
                let mut rhs = self.flat(f, 10);
                for a in init {
                    rhs.push_str(&format!(" {}", self.flat(a, 10)));
                }
                format!("{} |> {rhs}", self.flat(last, 0))
            }
            ExprKind::App(f, args) => {
                let mut s = self.flat(f, 10);
                for a in args {
                    s.push_str(&format!(" {}", self.flat(a, 10)));
                }
                s
            }
            ExprKind::Binop(op, l, r) => {
                let p = prec(op);
                // ponytail: comparisons print tight (`n>0`), everything else
                // spaced — the rule examples/ledger.vibe is written in.
                let sep = if matches!(op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=") {
                    op.clone()
                } else {
                    format!(" {op} ")
                };
                format!("{}{sep}{}", self.flat(l, p), self.flat(r, p + 1))
            }
            ExprKind::Neg(x) => format!("-{}", self.flat(x, 10)),
            ExprKind::Not(x) => format!("!{}", self.flat(x, 10)),
            ExprKind::Borrow(x) => format!("&{}", self.flat(x, 10)),
            ExprKind::Arena(a, body) => format!("arena {a} in {}", self.flat(body, 0)),
            ExprKind::Field(x, f) => format!("{}.{f}", self.flat(x, 10)),
            ExprKind::Tuple(xs) => {
                format!(
                    "({})",
                    xs.iter()
                        .map(|x| self.flat(x, 0))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ExprKind::List(xs) => {
                format!(
                    "[{}]",
                    xs.iter()
                        .map(|x| self.flat(x, 0))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ExprKind::Record(base, fs) => {
                let b = base
                    .as_ref()
                    .map(|b| format!("{} with ", self.flat(b, 10)))
                    .unwrap_or_default();
                let fs: Vec<String> = fs
                    .iter()
                    .map(|(n, v)| format!("{n}={}", self.flat(v, 0)))
                    .collect();
                format!("{{{b}{}}}", fs.join(", "))
            }
            ExprKind::Lambda(ps, b) => format!("\\{} -> {}", ps.join(" "), self.flat(b, 0)),
            // ponytail: block forms have no faithful one-line form; `lay` always
            // breaks them, so these only appear in width measurements.
            ExprKind::Match(s, arms) => {
                let arms: Vec<String> = arms
                    .iter()
                    .map(|(p, b)| format!("|{} -> {}", pat(p), self.flat(b, 0)))
                    .collect();
                format!("?{} {} end", self.scrut(s), arms.join(" "))
            }
            ExprKind::Bind(n, v, rest) if n == crate::parser::SEQ => {
                format!("{} ; {}", self.flat(v, 0), self.flat(rest, 0))
            }
            ExprKind::Bind(n, v, rest) => {
                format!("{n} <- {} ; {}", self.flat(v, 0), self.flat(rest, 0))
            }
            ExprKind::Let(n, v, rest) => {
                format!("let {n} = {} in {}", self.flat(v, 0), self.flat(rest, 0))
            }
        }
    }
}

/// Binding power: 10 atom, 7 application, 1..6 operators, 0 block forms.
fn power(k: &ExprKind) -> u8 {
    match k {
        ExprKind::App(..) => 7,
        ExprKind::Binop(op, ..) => prec(op),
        ExprKind::Match(..) | ExprKind::Bind(..) | ExprKind::Let(..) | ExprKind::Lambda(..) => 0,
        _ => 10,
    }
}

fn prec(op: &str) -> u8 {
    match op {
        "||" => 1,
        "&&" => 2,
        "==" | "!=" | "<" | "<=" | ">" | ">=" => 3,
        "++" => 4,
        "+" | "-" => 5,
        _ => 6,
    }
}

/// `x |> f a` desugars to `App(f, [a, x])` at parse time; the only surviving
/// trace is that the node's span is the `|>` token, not the head's.
/// ponytail: a `Pipe` node in the AST would be the honest fix.
fn is_pipe(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::App(f, args) => !args.is_empty() && e.span != f.span,
        _ => false,
    }
}

// ------------------------------------------------------------------ patterns

fn pat(p: &Pat) -> String {
    match p {
        Pat::Wild => "_".into(),
        Pat::Var(n) => n.clone(),
        Pat::Int(n) => n.to_string(),
        Pat::Float(x) => num(*x),
        Pat::Str(s) => quote(s),
        Pat::Char(c) => format!("'{}'", esc(*c)),
        Pat::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        Pat::Ctor(n, args) if args.is_empty() => n.clone(),
        Pat::Ctor(n, args) => {
            format!(
                "{n} {}",
                args.iter().map(pat_atom).collect::<Vec<_>>().join(" ")
            )
        }
        Pat::List(ps) => format!("[{}]", ps.iter().map(pat).collect::<Vec<_>>().join(",")),
        Pat::Tuple(ps) => format!("({})", ps.iter().map(pat).collect::<Vec<_>>().join(", ")),
    }
}

fn pat_atom(p: &Pat) -> String {
    match p {
        Pat::Ctor(_, args) if !args.is_empty() => format!("({})", pat(p)),
        _ => pat(p),
    }
}

// --------------------------------------------------------------------- types

fn ty(t: &Ty) -> String {
    match t {
        Ty::Fun(a, b) => format!("{} -> {}", ty_app(a), ty(b)),
        _ => ty_app(t),
    }
}

fn ty_app(t: &Ty) -> String {
    match t {
        Ty::Con(n, args) if args.is_empty() => n.clone(),
        Ty::Con(n, args) => {
            format!(
                "{n} {}",
                args.iter().map(ty_atom).collect::<Vec<_>>().join(" ")
            )
        }
        Ty::Var(v) => v.clone(),
        Ty::Ref(x) => format!("&{}", ty_app(x)),
        Ty::Eff(x) => format!("E! {}", ty_app(x)),
        Ty::Tuple(ts) => format!("({})", ts.iter().map(ty).collect::<Vec<_>>().join(", ")),
        Ty::Fun(..) => format!("({})", ty(t)),
    }
}

fn ty_atom(t: &Ty) -> String {
    match t {
        Ty::Con(_, args) if !args.is_empty() => format!("({})", ty(t)),
        Ty::Fun(..) | Ty::Eff(_) => format!("({})", ty(t)),
        Ty::Ref(x) => format!("&{}", ty_atom(x)),
        _ => ty_app(t),
    }
}

// ---------------------------------------------------------------------- flow

/// `--flow`: rewrite every pipeline into named bindings, then print normally.
/// ponytail: the generated names are `p1..pn` with no capture check; a real
/// implementation asks the checker for a fresh name.
fn flow_body(e: &Expr, n: &mut usize) -> Expr {
    match &e.kind {
        ExprKind::Bind(name, v, rest) => Expr::new(
            ExprKind::Bind(
                name.clone(),
                Box::new(hoisted(v, n)),
                Box::new(flow_body(rest, n)),
            ),
            e.span,
        ),
        ExprKind::Let(name, v, rest) => Expr::new(
            ExprKind::Let(
                name.clone(),
                Box::new(hoisted(v, n)),
                Box::new(flow_body(rest, n)),
            ),
            e.span,
        ),
        ExprKind::Lambda(ps, b) => Expr::new(
            ExprKind::Lambda(ps.clone(), Box::new(flow_body(b, n))),
            e.span,
        ),
        ExprKind::Match(s, arms) => {
            let mut lets = Vec::new();
            let scrut = hoist(s, n, &mut lets);
            let arms = arms
                .iter()
                .map(|(p, b)| (p.clone(), flow_body(b, n)))
                .collect();
            wrap(
                lets,
                Expr::new(ExprKind::Match(Box::new(scrut), arms), e.span),
            )
        }
        _ => hoisted(e, n),
    }
}

fn hoisted(e: &Expr, n: &mut usize) -> Expr {
    let mut lets = Vec::new();
    let r = hoist(e, n, &mut lets);
    wrap(lets, r)
}

fn wrap(lets: Vec<(String, Expr)>, body: Expr) -> Expr {
    lets.into_iter().rev().fold(body, |acc, (name, v)| {
        let span = v.span;
        Expr::new(ExprKind::Let(name, Box::new(v), Box::new(acc)), span)
    })
}

/// Replace each pipeline stage by a fresh variable bound in `lets`.
fn hoist(e: &Expr, n: &mut usize, lets: &mut Vec<(String, Expr)>) -> Expr {
    let kind = match &e.kind {
        ExprKind::App(f, args) => ExprKind::App(
            Box::new(hoist(f, n, lets)),
            args.iter().map(|a| hoist(a, n, lets)).collect(),
        ),
        ExprKind::Binop(op, a, b) => ExprKind::Binop(
            op.clone(),
            Box::new(hoist(a, n, lets)),
            Box::new(hoist(b, n, lets)),
        ),
        ExprKind::Neg(x) => ExprKind::Neg(Box::new(hoist(x, n, lets))),
        ExprKind::Not(x) => ExprKind::Not(Box::new(hoist(x, n, lets))),
        ExprKind::Borrow(x) => ExprKind::Borrow(Box::new(hoist(x, n, lets))),
        ExprKind::Arena(a, body) => ExprKind::Arena(a.clone(), Box::new(hoist(body, n, lets))),
        ExprKind::Field(x, f) => ExprKind::Field(Box::new(hoist(x, n, lets)), f.clone()),
        ExprKind::Tuple(xs) => ExprKind::Tuple(xs.iter().map(|x| hoist(x, n, lets)).collect()),
        ExprKind::List(xs) => ExprKind::List(xs.iter().map(|x| hoist(x, n, lets)).collect()),
        ExprKind::Record(b, fs) => ExprKind::Record(
            b.as_ref().map(|b| Box::new(hoist(b, n, lets))),
            fs.iter()
                .map(|(k, v)| (k.clone(), hoist(v, n, lets)))
                .collect(),
        ),
        // A binding cannot be hoisted out of a nested block form; leaves have
        // nothing to hoist.
        ExprKind::Match(..) | ExprKind::Bind(..) | ExprKind::Let(..) | ExprKind::Lambda(..) => {
            return flow_body(e, n)
        }
        _ => return e.clone(),
    };
    let out = Expr::new(kind, e.span);
    if is_pipe(e) {
        *n += 1;
        let name = format!("p{n}");
        // Re-span the call on its head, so it prints as an application and not
        // as the pipeline it came from.
        let val = match out.kind {
            ExprKind::App(f, args) => {
                let sp = f.span;
                Expr::new(ExprKind::App(f, args), sp)
            }
            k => Expr::new(k, e.span),
        };
        lets.push((name.clone(), val));
        return Expr::new(ExprKind::Var(name), e.span);
    }
    out
}

// -------------------------------------------------------------------- pieces

fn sp(n: usize) -> String {
    " ".repeat(n)
}

fn pad(s: &str, w: usize) -> String {
    format!("{s}{}", sp(w.saturating_sub(s.len())))
}

fn num(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

fn esc(c: char) -> String {
    match c {
        '\n' => "\\n".into(),
        '\t' => "\\t".into(),
        '\r' => "\\r".into(),
        '\0' => "\\0".into(),
        '\\' => "\\\\".into(),
        '"' => "\\\"".into(),
        c => c.to_string(),
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.chars().map(esc).collect::<String>())
}

// --------------------------------------------------------------- comments

/// Put the comments back into a rendering.
///
/// The lexer throws comments away — they carry no meaning — but it records how
/// many real tokens preceded each one. Newlines are not real tokens, so that
/// index means the same thing in the file and in any re-rendering of it, and
/// the comment can be placed again without either side agreeing on layout.
///
/// That matters more than it used to. Whitespace is free now, so `vibe view`
/// is routinely asked to reformat a file whose layout is nothing like the
/// canonical one. An earlier version lined the two up by line number and gave
/// up when the counts disagreed, which quietly deleted every comment in exactly
/// that case.
///
/// ponytail: an own-line comment is placed before the token it preceded and a
/// trailing one after the line holding the token before it, which is the whole
/// model. A comment written in the middle of an expression that the projection
/// then rewraps can move to the start of that line; it is never lost.
pub fn reattach(rendered: &str, comments: &[Comment]) -> String {
    if comments.is_empty() {
        return rendered.to_string();
    }
    let Ok(toks) = lexer::lex(rendered, 0) else {
        return rendered.to_string();
    };
    // line number of each real token, in the same counting the lexer used
    let lines: Vec<usize> = toks
        .iter()
        .filter(|t| !matches!(t.tok, lexer::Tok::Newline | lexer::Tok::Eof))
        .map(|t| t.span.line)
        .collect();
    let body: Vec<&str> = rendered.lines().collect();
    let mut before: Vec<Vec<&str>> = vec![Vec::new(); body.len() + 1];
    let mut after: Vec<Vec<&str>> = vec![Vec::new(); body.len() + 1];
    for c in comments {
        if c.own_line {
            let at = lines.get(c.after).copied().unwrap_or(body.len() + 1);
            before[(at - 1).min(body.len())].push(&c.text);
        } else {
            let at = c
                .after
                .checked_sub(1)
                .and_then(|i| lines.get(i).copied())
                .unwrap_or(1);
            after[(at - 1).min(body.len().saturating_sub(1))].push(&c.text);
        }
    }
    let mut out: Vec<String> = Vec::with_capacity(body.len() + comments.len());
    for (i, line) in body.iter().enumerate() {
        let indent = line.len() - line.trim_start().len();
        for c in &before[i] {
            out.push(format!("{}{}", sp(indent), c));
        }
        let mut l = line.to_string();
        for c in &after[i] {
            l = format!("{} {}", l.trim_end(), c);
        }
        out.push(l);
    }
    for c in before.last().into_iter().flatten() {
        out.push(c.to_string());
    }
    let mut s = out.join("\n");
    if rendered.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Canonicity (spec §3.1), as one rule instead of a list.
///
/// Whitespace is not part of it. A generator that miscounts spaces must still
/// produce a program that compiles, so indentation, blank lines and the column
/// a declaration starts in are all free. What canonicity still means is that
/// the projection and the parser agree on the program: printing a file and
/// lexing the result must give back the same tokens.
///
/// In practice that leaves this check guarding the compiler rather than the
/// user — the parser already rejects the structural variants on its own, and a
/// difference here means `vibe view` would have changed the program. It is
/// cheap, and the day it fires it will have caught something worth catching.
pub fn canon(m: &Module, ck: &Checked, src: &str, _comments: &[Comment], file: usize) -> Vec<Diag> {
    let rendered = render(m, ck, Mode::Canon);
    let (Ok(want), Ok(got)) = (lexer::lex(&rendered, file), lexer::lex(src, file)) else {
        return Vec::new(); // the source already failed to lex, or the rendering did
    };
    let n = want.len().min(got.len());
    let at = (0..n).find(|&i| want[i].tok != got[i].tok);
    let i = match at {
        Some(i) => i,
        None if want.len() == got.len() => return Vec::new(),
        None => n.saturating_sub(1),
    };
    let span = got.get(i).map(|t| t.span).unwrap_or_default();
    vec![Diag::error(
        span,
        "canon.form",
        "this is not the canonical form of the program",
    )
    .with_path(&m.name)
    .with_fix("run `vibe view` on the file and write back what it prints")]
}

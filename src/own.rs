//! Ownership (spec §4): affine use of owned values.
//!
//! The one rule: `&`-prefixed parameters are borrows for the duration of the
//! call, everything else is owned, and an owned value is consumed by its first
//! use. Using it twice is an error with a mechanical fix (`&x`, or `dup x`).
//!
//! Parameters carry their own type; `let`, `<-` and pattern binders do not, so
//! their affinity comes from inference, which records it under the span of the
//! expression each binder scopes over (`Checked::affine`).
//!
//! A closure that outlives the call owns what it captured (spec §4.6), so a
//! capture inside an escaping lambda is a move; one inside a lambda that is
//! consumed during the call — the argument to `map`, say — is a read.
//!
//! ponytail: "outlives the call" is read as "its value reaches the result",
//! through match arms, `let` and `<-` bodies, and any tuple, list or record
//! built in that position. A lambda stored in a structure that a callee then
//! returns is missed. §4.6 asks for an error demanding an explicit `move` when
//! the analysis cannot decide; deciding conservatively costs no soundness here,
//! because the direction it errs in is "read", and reads are already checked.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::infer::Checked;
use crate::types::is_num;
use std::collections::{HashMap, HashSet};

pub fn check(m: &Module, ck: &Checked) -> Vec<Diag> {
    run(m, ck).0
}

/// Spans of `{r with ...}` updates whose base is a uniquely owned value, so
/// codegen can mutate in place instead of copying (spec §4.3).
pub fn inplace_updates(m: &Module, ck: &Checked) -> HashSet<(usize, usize, usize)> {
    run(m, ck).1
}

fn run(m: &Module, ck: &Checked) -> (Vec<Diag>, HashSet<(usize, usize, usize)>) {
    let mut out = Vec::new();
    let mut inplace = HashSet::new();
    for f in m.funs().filter(|f| !f.ghost) {
        let mut escaping = HashSet::new();
        escapes(&f.body, &mut escaping);
        let mut st = State {
            moved: HashMap::new(),
            errors: Vec::new(),
            inplace: HashSet::new(),
            path: format!("{}.{}", f.home, f.name),
            ck,
            escaping,
        };
        let mut owned: Vec<&str> = Vec::new();
        for p in &f.params {
            if p.ty.as_ref().is_some_and(is_affine) {
                owned.extend(p.names.iter().map(|s| s.as_str()));
            }
        }
        st.walk(&f.body, Mode::Own, &owned);
        out.append(&mut st.errors);
        inplace.extend(st.inplace.drain());
    }
    (out, inplace)
}

/// Scalars are copied, not moved. Everything with a payload is affine.
fn is_affine(t: &Ty) -> bool {
    match t {
        Ty::Ref(_) => false,
        Ty::Con(n, _) => {
            !(is_num(n)
                || matches!(
                    n.as_str(),
                    "Bool" | "Char" | "Unit" | "Nat" | "Size" | "CStr" | "Ptr"
                ))
        }
        Ty::Var(_) => true,
        Ty::Tuple(ts) => ts.iter().any(is_affine),
        Ty::Fun(..) | Ty::Eff(_) => false,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// The value is consumed by this position.
    Own,
    /// The value is only read for the duration of the expression.
    Borrow,
}

struct State<'a> {
    moved: HashMap<String, Span>,
    errors: Vec<Diag>,
    inplace: HashSet<(usize, usize, usize)>,
    path: String,
    ck: &'a Checked,
    /// Lambdas whose value reaches the function's result, so their captures
    /// outlive the call.
    escaping: HashSet<(usize, usize, usize)>,
}

impl State<'_> {
    /// The names a binder adds to the owned set: those inference typed as
    /// affine. A name that shadows an owned one always leaves the outer set,
    /// affine or not, because from here on it means something else.
    fn scope<'n>(&self, outer: &[&'n str], bound: &'n [String], scope: Span) -> Vec<&'n str> {
        let mut v: Vec<&str> = outer
            .iter()
            .copied()
            .filter(|x| !bound.iter().any(|b| b == x))
            .collect();
        for n in bound {
            let k = (scope.file, scope.line, scope.col, n.clone());
            if self.ck.affine.get(&k).copied().unwrap_or(false) {
                v.push(n);
            }
        }
        v
    }

    fn use_var(&mut self, n: &str, span: Span, mode: Mode, owned: &[&str]) {
        if mode == Mode::Borrow || !owned.contains(&n) {
            return;
        }
        if let Some(first) = self.moved.get(n) {
            self.errors.push(
                Diag::error(
                    span,
                    "own.use_after_move",
                    &format!("`{}` was already moved", n),
                )
                .with_path(&self.path)
                .with_witness(&format!(
                    "first moved at line {}, column {}",
                    first.line,
                    first.col + 1
                ))
                .with_fix(&format!(
                    "borrow it here with `&{}`, or copy it with `dup {}`",
                    n, n
                )),
            );
        } else {
            self.moved.insert(n.to_string(), span);
        }
    }

    fn walk(&mut self, e: &Expr, mode: Mode, owned: &[&str]) {
        use ExprKind::*;
        match &e.kind {
            Var(n) => self.use_var(n, e.span, mode, owned),
            Borrow(x) => self.walk(x, Mode::Borrow, owned),
            // Reading a field reads through the value; it does not consume it.
            Field(x, _) => self.walk(x, Mode::Borrow, owned),
            App(h, args) => {
                self.walk(h, Mode::Borrow, owned);
                for a in args {
                    self.walk(a, mode, owned);
                }
            }
            Binop(_, a, b) => {
                self.walk(a, mode, owned);
                self.walk(b, mode, owned);
            }
            Neg(x) | Not(x) => self.walk(x, mode, owned),
            Arena(_, body) => self.walk(body, mode, owned),
            Tuple(xs) | List(xs) => {
                for x in xs {
                    self.walk(x, mode, owned);
                }
            }
            Record(base, fields) => {
                if let Some(b) = base {
                    // `{r with f = v}` on a value we own and have not moved yet
                    // is a mutation, not a copy.
                    if let Var(n) = &b.kind {
                        if mode == Mode::Own
                            && owned.contains(&n.as_str())
                            && !self.moved.contains_key(n)
                        {
                            self.inplace.insert((e.span.file, e.span.line, e.span.col));
                        }
                    }
                    self.walk(b, mode, owned);
                }
                for (_, v) in fields {
                    self.walk(v, mode, owned);
                }
            }
            Let(n, v, body) | Bind(n, v, body) => {
                self.walk(v, mode, owned);
                let bound = [n.clone()];
                let inner = self.scope(owned, &bound, body.span);
                self.walk(body, mode, &inner);
            }
            Lambda(_, body) => {
                let key = (e.span.file, e.span.line, e.span.col);
                // An escaping closure owns its captures; one that dies with the
                // call only reads them (spec §4.6).
                let m = if self.escaping.contains(&key) {
                    Mode::Own
                } else {
                    Mode::Borrow
                };
                self.walk(body, m, owned)
            }
            Match(scrut, arms) => {
                self.walk(scrut, Mode::Borrow, owned);
                // Arms are alternatives: each starts from the state before the
                // match, and a value moved in any arm is moved after it.
                let before = self.moved.clone();
                let mut after = before.clone();
                for (p, body) in arms {
                    self.moved = before.clone();
                    let bound = pat_names(p);
                    let visible = self.scope(owned, &bound, body.span);
                    self.walk(body, mode, &visible);
                    for (k, v) in self.moved.drain() {
                        after.entry(k).or_insert(v);
                    }
                }
                self.moved = after;
            }
            Int(_) | Float(_) | Str(_) | Char(_) | Bool(_) | Unit | Ctor(_) => {}
        }
    }
}

/// Lambdas in result position: the value of the expression, or of an arm, or a
/// component of a structure built there.
fn escapes(e: &Expr, out: &mut HashSet<(usize, usize, usize)>) {
    use ExprKind::*;
    match &e.kind {
        // an inner lambda's captures are the outer lambda's problem, not ours
        Lambda(..) => {
            out.insert((e.span.file, e.span.line, e.span.col));
        }
        Match(_, arms) => arms.iter().for_each(|(_, b)| escapes(b, out)),
        Let(_, _, b) | Bind(_, _, b) | Arena(_, b) | Borrow(b) => escapes(b, out),
        Tuple(xs) | List(xs) => xs.iter().for_each(|x| escapes(x, out)),
        Record(_, fields) => fields.iter().for_each(|(_, v)| escapes(v, out)),
        _ => {}
    }
}

fn pat_names(p: &Pat) -> Vec<String> {
    match p {
        Pat::Var(n) => vec![n.clone()],
        Pat::Ctor(_, ps) | Pat::List(ps) | Pat::Tuple(ps) => {
            ps.iter().flat_map(pat_names).collect()
        }
        _ => Vec::new(),
    }
}

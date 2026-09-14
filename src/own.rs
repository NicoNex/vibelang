//! Ownership (spec §4): affine use of owned values.
//!
//! The one rule: `&`-prefixed parameters are borrows for the duration of the
//! call, everything else is owned, and an owned value is consumed by its first
//! use. Using it twice is an error with a mechanical fix (`&x`, or `dup x`).
//!
//! ponytail: only parameters with a written type are tracked — `let`/`<-`
//! bindings and pattern variables have no declared type here, so tracking them
//! would need the inferred type of every subexpression. Widen this by threading
//! `infer::Checked` through once the checker records per-expression types.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::types::is_num;
use std::collections::HashMap;

pub fn check(m: &Module) -> Vec<Diag> {
    let mut out = Vec::new();
    for f in m.funs().filter(|f| !f.ghost) {
        let mut st = State { moved: HashMap::new(), errors: Vec::new(), path: format!("{}.{}", m.name, f.name) };
        let mut owned: Vec<&str> = Vec::new();
        for p in &f.params {
            if p.ty.as_ref().is_some_and(is_affine) {
                owned.extend(p.names.iter().map(|s| s.as_str()));
            }
        }
        if owned.is_empty() {
            continue;
        }
        st.walk(&f.body, Mode::Own, &owned);
        out.append(&mut st.errors);
    }
    out
}

/// Scalars are copied, not moved. Everything with a payload is affine.
fn is_affine(t: &Ty) -> bool {
    match t {
        Ty::Ref(_) => false,
        Ty::Con(n, _) => !(is_num(n) || matches!(n.as_str(), "Bool" | "Char" | "Unit" | "Size" | "CStr" | "Ptr")),
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

struct State {
    moved: HashMap<String, Span>,
    errors: Vec<Diag>,
    path: String,
}

impl State {
    fn use_var(&mut self, n: &str, span: Span, mode: Mode, owned: &[&str]) {
        if mode == Mode::Borrow || !owned.contains(&n) {
            return;
        }
        if let Some(first) = self.moved.get(n) {
            self.errors.push(
                Diag::error(span, "own.use_after_move", &format!("`{}` was already moved", n))
                    .with_path(&self.path)
                    .with_witness(&format!("first moved at line {}, column {}", first.line, first.col + 1))
                    .with_fix(&format!("borrow it here with `&{}`, or copy it with `dup {}`", n, n)),
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
            Tuple(xs) | List(xs) => {
                for x in xs {
                    self.walk(x, mode, owned);
                }
            }
            Record(base, fields) => {
                if let Some(b) = base {
                    self.walk(b, mode, owned);
                }
                for (_, v) in fields {
                    self.walk(v, mode, owned);
                }
            }
            Let(n, v, body) | Bind(n, v, body) => {
                self.walk(v, mode, owned);
                // A binding of the same name shadows the parameter from here on.
                let shadowed: Vec<&str> = owned.iter().copied().filter(|x| x != n).collect();
                self.walk(body, mode, &shadowed);
            }
            // ponytail: captures are treated as reads. Escape analysis (spec §4.6)
            // is what decides whether a closure owns them; it does not exist yet.
            Lambda(_, body) => self.walk(body, Mode::Borrow, owned),
            Match(scrut, arms) => {
                self.walk(scrut, Mode::Borrow, owned);
                // Arms are alternatives: each starts from the state before the
                // match, and a value moved in any arm is moved after it.
                let before = self.moved.clone();
                let mut after = before.clone();
                for (p, body) in arms {
                    self.moved = before.clone();
                    let bound = pat_names(p);
                    let visible: Vec<&str> =
                        owned.iter().copied().filter(|x| !bound.iter().any(|b| b == x)).collect();
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

fn pat_names(p: &Pat) -> Vec<String> {
    match p {
        Pat::Var(n) => vec![n.clone()],
        Pat::Ctor(_, ps) | Pat::List(ps) | Pat::Tuple(ps) => ps.iter().flat_map(pat_names).collect(),
        _ => Vec::new(),
    }
}

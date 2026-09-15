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
use crate::types::{is_num, is_special, PRELUDE_SIGS};
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
    let sigs = borrowed_params(m);
    let mut out = Vec::new();
    let mut inplace = HashSet::new();
    for f in m.funs().filter(|f| !f.ghost) {
        // A borrow lives for the call that lent it (spec §4.4), so returning
        // one hands the caller a pointer into a value it may already have
        // dropped. `&` is erased by `types::lower_ty`, so nothing downstream
        // notices: the result arrives owned and gets freed a second time.
        if let Some(Ty::Ref(_)) = &f.ret {
            out.push(
                Diag::error(
                    f.span,
                    "own.borrow_escapes",
                    &format!("`{}` returns a borrow", f.name),
                )
                .with_path(&format!("{}.{}", f.home, f.name))
                .with_witness("a borrow lives only for the call that lent it")
                .with_fix("return an owned value, or the index of the element instead"),
            );
        }
        // The other half of the same rule: `&` is erased, so a body whose tail
        // is a borrowed parameter launders it into an owned result without the
        // declared type ever saying `&`.
        let borrows: Vec<&str> = f
            .params
            .iter()
            .filter(|p| matches!(p.ty, Some(Ty::Ref(_))))
            .flat_map(|p| p.names.iter().map(|s| s.as_str()))
            .collect();
        if !borrows.is_empty() && f.ret.as_ref().is_some_and(|t| !matches!(t, Ty::Ref(_))) {
            let mut tails = Vec::new();
            returned(&f.body, &mut tails);
            for (n, span) in tails {
                if borrows.contains(&n) {
                    out.push(
                        Diag::error(
                            span,
                            "own.borrow_escapes",
                            &format!("`{}` is a borrow and is returned as owned", n),
                        )
                        .with_path(&format!("{}.{}", f.home, f.name))
                        .with_witness("a borrow lives only for the call that lent it")
                        .with_fix(&format!("copy it with `dup {}`, or return an index", n)),
                    );
                }
            }
        }
        let mut escaping = HashSet::new();
        escapes(&f.body, &mut escaping);
        let mut st = State {
            moved: HashMap::new(),
            mutated: HashMap::new(),
            errors: Vec::new(),
            inplace: HashSet::new(),
            path: format!("{}.{}", f.home, f.name),
            ck,
            sigs: &sigs,
            escaping,
        };
        let mut owned: Vec<(&str, bool)> = Vec::new();
        for p in &f.params {
            if p.ty.as_ref().is_some_and(is_affine) {
                let is_str = matches!(&p.ty, Some(Ty::Con(n, _)) if n == "Str");
                owned.extend(p.names.iter().map(|s| (s.as_str(), is_str)));
            }
        }
        st.walk(&f.body, Mode::Own, &owned);
        out.append(&mut st.errors);
        inplace.extend(st.inplace.drain());
    }
    (out, inplace)
}

/// Argument positions the callee declared `&`, by callee name. Passing an
/// owned value into a borrowed position is a read, not a move (spec §4.2):
/// `len ts` leaves `ts` usable, which is what makes `(len ts) (total &ts)` in
/// `examples/ledger.vibe` a correct program.
fn borrowed_params(m: &Module) -> HashMap<String, Vec<bool>> {
    let mut map: HashMap<String, Vec<bool>> = PRELUDE_SIGS
        .iter()
        .map(|(n, sig)| ((*n).to_string(), sig_borrows(sig)))
        .collect();
    for e in m.exts() {
        for s in &e.sigs {
            map.insert(s.name.clone(), ty_borrows(&s.ty));
        }
    }
    // Declarations win over the prelude: a module may shadow a prelude name.
    for f in m.funs() {
        map.insert(
            f.name.clone(),
            f.params
                .iter()
                .flat_map(|p| {
                    let borrowed = matches!(p.ty, Some(Ty::Ref(_)));
                    p.names.iter().map(move |_| borrowed)
                })
                .collect(),
        );
    }
    map
}

/// The parameters of an arrow type, each as "is it a borrow".
fn ty_borrows(t: &Ty) -> Vec<bool> {
    let mut v = Vec::new();
    let mut cur = t;
    while let Ty::Fun(a, b) = cur {
        v.push(matches!(**a, Ty::Ref(_)));
        cur = b;
    }
    v
}

/// The same, over a written prelude signature: split on top-level `->` and
/// drop the result.
fn sig_borrows(sig: &str) -> Vec<bool> {
    let b = sig.as_bytes();
    let (mut depth, mut start, mut i) = (0usize, 0usize, 0usize);
    let mut out = Vec::new();
    while i < b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'-' if depth == 0 && b.get(i + 1) == Some(&b'>') => {
                out.push(sig[start..i].trim_start().starts_with('&'));
                i += 1;
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Every name a body can yield as its result, with the span it sits at.
fn returned<'a>(e: &'a Expr, out: &mut Vec<(&'a str, Span)>) {
    use ExprKind::*;
    match &e.kind {
        Var(n) => out.push((n.as_str(), e.span)),
        Let(_, _, b) | Bind(_, _, b) | Arena(_, b) => returned(b, out),
        Match(_, arms) => arms.iter().for_each(|(_, a)| returned(a, out)),
        _ => {}
    }
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
    /// Bases of a `{r with ...}` that codegen mutates in place.
    mutated: HashMap<String, Span>,
    errors: Vec<Diag>,
    inplace: HashSet<(usize, usize, usize)>,
    path: String,
    ck: &'a Checked,
    /// Which argument positions each callee borrows.
    sigs: &'a HashMap<String, Vec<bool>>,
    /// Lambdas whose value reaches the function's result, so their captures
    /// outlive the call.
    escaping: HashSet<(usize, usize, usize)>,
}

impl State<'_> {
    /// The names a binder adds to the owned set, each with whether its type is
    /// `Str`: those inference typed as affine. A name that shadows an owned one
    /// always leaves the outer set, affine or not, because from here on it
    /// means something else.
    fn scope<'n>(
        &self,
        outer: &[(&'n str, bool)],
        bound: &'n [String],
        scope: Span,
    ) -> Vec<(&'n str, bool)> {
        let mut v: Vec<(&str, bool)> = outer
            .iter()
            .copied()
            .filter(|(x, _)| !bound.iter().any(|b| b == x))
            .collect();
        for n in bound {
            let k = (scope.file, scope.line, scope.col, n.clone());
            if let Some(base) = self.ck.affine.get(&k) {
                v.push((n, base == "Str"));
            }
        }
        v
    }

    fn use_var(&mut self, n: &str, span: Span, mode: Mode, owned: &[(&str, bool)]) {
        let Some((_, is_str)) = owned.iter().find(|(x, _)| *x == n) else {
            return;
        };
        // A borrow does not consume, but it does read, and the value has to
        // still be there to read. Two ways it is not: the base of a
        // `{r with ...}` that codegen mutated in place — the read returns the
        // update rather than the original, silently and wrongly — and a value
        // already moved, whose lifetime now belongs to whoever took it
        // (docs/aliasing-audit.md gap 5).
        if mode == Mode::Borrow {
            if let Some(first) = self.mutated.get(n) {
                self.errors.push(
                    Diag::error(
                        span,
                        "own.use_after_update",
                        &format!("`{}` was updated in place", n),
                    )
                    .with_path(&self.path)
                    .with_witness(&format!(
                        "updated at line {}, column {}",
                        first.line,
                        first.col + 1
                    ))
                    .with_fix(&format!("read `{}` before the update", n)),
                );
            } else if let Some(first) = self.moved.get(n) {
                self.errors.push(
                    Diag::error(
                        span,
                        "own.borrow_after_move",
                        &format!("`{}` was already moved", n),
                    )
                    .with_path(&self.path)
                    .with_witness(&format!(
                        "first moved at line {}, column {}",
                        first.line,
                        first.col + 1
                    ))
                    // `&x` is the fix for a second *move*; here the borrow is
                    // the second use, so the move is what has to give way.
                    .with_fix(&if *is_str {
                        format!("read `{}` before the move, or copy it with `dup {}`", n, n)
                    } else {
                        format!("read `{}` before the move", n)
                    }),
                );
            }
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
                // `dup` is `&Str -> Str`, so offering it on anything else
                // would be a fix that does not type-check.
                .with_fix(&if *is_str {
                    format!("borrow it here with `&{}`, or copy it with `dup {}`", n, n)
                } else {
                    format!("borrow it here with `&{}`", n)
                }),
            );
        } else {
            self.moved.insert(n.to_string(), span);
        }
    }

    fn walk(&mut self, e: &Expr, mode: Mode, owned: &[(&str, bool)]) {
        use ExprKind::*;
        match &e.kind {
            Var(n) => self.use_var(n, e.span, mode, owned),
            Borrow(x) => self.walk(x, Mode::Borrow, owned),
            // Reading a field reads through the value; it does not consume it.
            Field(x, _) => self.walk(x, Mode::Borrow, owned),
            App(h, args) => {
                self.walk(h, Mode::Borrow, owned);
                // The callee's signature decides each argument: a position it
                // declared `&` is read for the duration of the call, so what is
                // passed there is not moved, whether or not the call site wrote
                // the `&`. Anything the signature does not cover — a local
                // function value, an over-applied result — stays a move.
                let sigs = self.sigs;
                let (reads_all, borrows) = match &h.kind {
                    // `len`, `fmt` and `show` are typed by a rule rather than a
                    // signature (`types::is_special`), and all three only read.
                    Var(n) => (is_special(n), sigs.get(n.as_str())),
                    _ => (false, None),
                };
                for (i, a) in args.iter().enumerate() {
                    let m = if reads_all || borrows.is_some_and(|b| *b.get(i).unwrap_or(&false)) {
                        Mode::Borrow
                    } else {
                        mode
                    };
                    self.walk(a, m, owned);
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
                // Fields first: `{r with f = r.f+1}` computes the update from
                // the value the base still holds, so that read comes before the
                // base is moved and before it is overwritten, not after either.
                for (_, v) in fields {
                    self.walk(v, mode, owned);
                }
                let mut updated = None;
                if let Some(b) = base {
                    // `{r with f = v}` on a value we own and have not moved yet
                    // is a mutation, not a copy.
                    if let Var(n) = &b.kind {
                        if mode == Mode::Own
                            && owned.iter().any(|(x, _)| x == n)
                            && !self.moved.contains_key(n)
                        {
                            self.inplace.insert((e.span.file, e.span.line, e.span.col));
                            updated = Some(n.clone());
                        }
                    }
                    self.walk(b, mode, owned);
                }
                if let Some(n) = updated {
                    self.mutated.insert(n, e.span);
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

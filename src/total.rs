//! Termination (spec §6). A *pure* recursive function needs a measure that
//! decreases on a well-founded order at each recursive call. The measure is
//! inferred when a parameter decreases syntactically everywhere (§6.2), and
//! otherwise written by hand with `%`.
//!
//! Divergence is an effect. A function marked `E!` may recurse for ever, and is
//! exempt: an event loop or a server is supposed not to terminate, and the
//! alternative was a `while` construct, which is one more thing for a generator
//! to choose wrong. Purity is what the static reasoning rests on, and purity is
//! exactly what this still enforces.
//!
//! A measure is a lexicographic tuple of linear expressions over the parameters
//! (§6.4): `%(n, k)` compares `n` first and only looks at `k` when `n` is level,
//! which is what a mutually recursive group needs when the component that
//! shrinks changes from one member to the next. A measure that is not written as
//! a tuple is a tuple of one.
//!
//! ponytail: a tuple measure is written, never inferred, and the members of a
//! group must all write tuples of the same width. Upgrade path: pad a short
//! tuple on the right, and try parameter positions across the group at once.

use crate::ast::*;
use crate::diag::{Diag, Span};
use std::collections::HashMap;

/// Whether this function is allowed not to terminate: its declared result is
/// effectful, so divergence is among the effects it announces.
fn diverges(f: &FunDecl) -> bool {
    fn effectful(t: &Ty) -> bool {
        match t {
            Ty::Eff(_) => true,
            Ty::Fun(_, r) => effectful(r),
            _ => false,
        }
    }
    f.ret.as_ref().is_some_and(effectful)
}

pub fn check(m: &Module) -> Vec<Diag> {
    let funs: Vec<&FunDecl> = m.funs().collect();
    let index: HashMap<&str, usize> = funs
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), i))
        .collect();
    let params: Vec<Vec<String>> = funs.iter().map(|f| param_names(f)).collect();
    let calls: Vec<Vec<Call>> = funs.iter().map(|f| calls_in(&f.body, &index)).collect();

    // Reachability closure over the call graph; `i` is recursive when it reaches itself.
    let n = funs.len();
    let mut reach = vec![vec![false; n]; n];
    for (i, cs) in calls.iter().enumerate() {
        for c in cs {
            reach[i][c.callee] = true;
        }
    }
    for k in 0..n {
        for i in 0..n {
            for j in 0..n {
                if reach[i][k] && reach[k][j] {
                    reach[i][j] = true;
                }
            }
        }
    }
    let same_group = |i: usize, j: usize| i == j || (reach[i][j] && reach[j][i]);

    let mut ds = Vec::new();
    // Pass 1: a measure for every recursive function.
    let mut measures: Vec<Option<Vec<Lin>>> = vec![None; n];
    for i in 0..n {
        if !reach[i][i] {
            continue; // not recursive: nothing to prove
        }
        if diverges(funs[i]) {
            continue; // effectful: non-termination is one of the effects (§6.1)
        }
        let f = funs[i];
        let path = format!("{}.{}", f.home, f.name);
        match &f.measure {
            Some(e) => {
                let parts: Vec<Lin> = components(e).into_iter().map(lin).collect();
                if parts
                    .iter()
                    .all(|l| l.terms.keys().all(|k| params[i].contains(k)))
                {
                    measures[i] = Some(parts);
                } else {
                    ds.push(
                        Diag::error(
                            e.span,
                            "total.no_measure",
                            &format!(
                                "the measure of `{}` is not arithmetic over its parameters",
                                f.name
                            ),
                        )
                        .with_path(&path)
                        .with_witness(&show(e))
                        .with_fix(
                            "write the measure with `+`/`-` over scalar parameters, e.g. `%(n-k)`, or as a tuple, e.g. `%(n, k)`",
                        ),
                    );
                }
            }
            // ponytail: inference only for direct self-recursion; a mutual group
            // must say what decreases. Upgrade path: try each parameter position
            // across the whole group at once.
            None => {
                let solo = calls[i].iter().all(|c| c.callee == i);
                let inferred = params[i].iter().find(|p| {
                    let cand = Lin::var(p);
                    solo && calls[i]
                        .iter()
                        .all(|c| delta(&cand, &cand, &params[i], &c.args).is_some_and(|d| d > 0))
                });
                match inferred {
                    Some(p) => measures[i] = Some(vec![Lin::var(p)]),
                    None => ds.push(
                        Diag::error(
                            f.span,
                            "total.no_measure",
                            &if solo {
                                format!(
                                    "`{}` is recursive and no parameter decreases at every call",
                                    f.name
                                )
                            } else {
                                format!(
                                    "`{}` is mutually recursive, so its measure is not inferred",
                                    f.name
                                )
                            },
                        )
                        .with_path(&path)
                        .with_fix(if solo {
                            "add a measure after the body, e.g. `%(n-k)`"
                        } else {
                            "give every member of the group a measure, as a lexicographic tuple if the component that shrinks changes, e.g. `%(n, k)`"
                        }),
                    ),
                }
            }
        }
    }

    // Pass 2: every call inside the group must drop the measure.
    for i in 0..n {
        let Some(mi) = &measures[i] else { continue };
        for c in &calls[i] {
            if !same_group(i, c.callee) {
                continue;
            }
            let Some(mj) = &measures[c.callee] else {
                continue;
            };
            if lex_decreases(mi, mj, &params[c.callee], &c.args) {
                continue;
            }
            ds.push(
                Diag::error(
                    c.span,
                    "total.not_decreasing",
                    &format!(
                        "the measure of `{}` does not decrease at this call to `{}`",
                        funs[i].name, funs[c.callee].name
                    ),
                )
                .with_path(&format!("{}.{}", funs[i].home, funs[i].name))
                .with_witness(&format!("measure {}", show_lex(mi)))
                .with_fix("make an argument shrink, or give a measure that does, e.g. `%(n-k)`"),
            );
        }
    }
    ds
}

struct Call {
    callee: usize,
    args: Vec<Expr>,
    span: Span,
}

fn param_names(f: &FunDecl) -> Vec<String> {
    f.params
        .iter()
        .flat_map(|p| p.names.iter().cloned())
        .collect()
}

/// Every way this body can reach a module function: a direct call, with its
/// arguments, and a mention of the name as a *value*, with none.
///
/// The second kind is what `map f xs` does, and it is an edge for the same
/// reason the first is — `sum &(map loopy &(single n))` recurses. Its arguments
/// are unknown, so no measure can be shown to decrease at it, and a pure
/// function that recurses this way is rejected until it stops. That is the
/// conservative direction, and purity is what every static guarantee in the
/// language rests on (§6.1).
fn calls_in(body: &Expr, index: &HashMap<&str, usize>) -> Vec<Call> {
    let mut out = Vec::new();
    scan(body, index, &mut out);
    out
}

fn scan(e: &Expr, index: &HashMap<&str, usize>, out: &mut Vec<Call>) {
    if let ExprKind::App(h, args) = &e.kind {
        if let ExprKind::Var(name) = &h.kind {
            if let Some(&i) = index.get(name.as_str()) {
                // The head of a saturated call, with its arguments. Counting it
                // again below as a bare mention would be the same call twice,
                // once with the arguments and once without.
                out.push(Call {
                    callee: i,
                    args: args.clone(),
                    span: e.span,
                });
                for a in args {
                    scan(a, index, out);
                }
                return;
            }
        }
    }
    if let ExprKind::Var(name) = &e.kind {
        if let Some(&i) = index.get(name.as_str()) {
            out.push(Call {
                callee: i,
                args: Vec::new(),
                span: e.span,
            });
        }
    }
    children(e, &mut |c| scan(c, index, out));
}

/// The immediate sub-expressions, in evaluation order.
fn children(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    use ExprKind::*;
    match &e.kind {
        App(h, args) => {
            f(h);
            args.iter().for_each(&mut *f);
        }
        Binop(_, a, b) | Bind(_, a, b) | Let(_, a, b) => {
            f(a);
            f(b);
        }
        Neg(a) | Not(a) | Borrow(a) | Field(a, _) | Lambda(_, a) | Arena(_, a) => f(a),
        Match(s, arms) => {
            f(s);
            arms.iter().for_each(|(_, b)| f(b));
        }
        Record(base, fields) => {
            if let Some(b) = base {
                f(b);
            }
            fields.iter().for_each(|(_, v)| f(v));
        }
        Tuple(xs) | List(xs) => xs.iter().for_each(f),
        Int(_) | Float(_) | Str(_) | Char(_) | Bool(_) | Unit | Var(_) | Ctor(_) => {}
    }
}

/// A measure as `sum(coefficient * atom) + constant`. Atoms are parameters, or
/// opaque sub-expressions keyed by how they are written.
#[derive(Clone, Default)]
struct Lin {
    terms: HashMap<String, i64>,
    c: i64,
}

impl Lin {
    fn var(name: &str) -> Lin {
        let mut l = Lin::default();
        l.terms.insert(name.to_string(), 1);
        l
    }
    fn add_scaled(&mut self, o: &Lin, k: i64) {
        self.c += o.c * k;
        for (a, v) in &o.terms {
            let e = self.terms.entry(a.clone()).or_insert(0);
            *e += v * k;
            if *e == 0 {
                self.terms.remove(a);
            }
        }
    }
    fn show(&self) -> String {
        let mut parts: Vec<String> = self
            .terms
            .iter()
            .map(|(a, v)| format!("{}*{}", v, a))
            .collect();
        parts.sort();
        if self.c != 0 || parts.is_empty() {
            parts.push(self.c.to_string());
        }
        parts.join(" + ")
    }
}

fn lin(e: &Expr) -> Lin {
    let mut l = Lin::default();
    match &e.kind {
        ExprKind::Int(n) => l.c = *n,
        ExprKind::Var(n) => return Lin::var(n),
        ExprKind::Neg(a) => l.add_scaled(&lin(a), -1),
        ExprKind::Binop(op, a, b) if op == "+" || op == "-" => {
            l.add_scaled(&lin(a), 1);
            l.add_scaled(&lin(b), if op == "+" { 1 } else { -1 });
        }
        ExprKind::Binop(op, a, b) if op == "*" => match (&a.kind, &b.kind) {
            (ExprKind::Int(k), _) => l.add_scaled(&lin(b), *k),
            (_, ExprKind::Int(k)) => l.add_scaled(&lin(a), *k),
            _ => return Lin::var(&show(e)),
        },
        _ => return Lin::var(&show(e)),
    }
    l
}

/// The components of a measure. A tuple is lexicographic, outermost first;
/// anything else is a tuple of one.
fn components(e: &Expr) -> Vec<&Expr> {
    match &e.kind {
        ExprKind::Tuple(xs) => xs.iter().collect(),
        _ => vec![e],
    }
}

fn show_lex(m: &[Lin]) -> String {
    m.iter().map(Lin::show).collect::<Vec<_>>().join(", ")
}

/// How far the callee's component sits below the caller's, once the call
/// arguments stand in for the callee's parameters. `None` when the other atoms
/// do not cancel exactly, because then no gap holds for every value they take.
fn delta(caller: &Lin, callee: &Lin, callee_params: &[String], args: &[Expr]) -> Option<i64> {
    if args.len() != callee_params.len() {
        return None; // partial application: nothing to substitute into
    }
    let mut d = caller.clone();
    for (atom, k) in &callee.terms {
        let i = callee_params.iter().position(|p| p == atom)?;
        d.add_scaled(&lin(&args[i]), -k);
    }
    d.c -= callee.c;
    d.terms.is_empty().then_some(d.c)
}

/// Lexicographic order: one component drops by a positive constant and no
/// component before it grows. Tuples of different widths are not comparable.
fn lex_decreases(caller: &[Lin], callee: &[Lin], callee_params: &[String], args: &[Expr]) -> bool {
    if caller.len() != callee.len() {
        return false;
    }
    for (a, b) in caller.iter().zip(callee) {
        match delta(a, b, callee_params, args) {
            Some(d) if d > 0 => return true,
            Some(0) => continue,
            _ => return false, // grows, or does not cancel
        }
    }
    false // every component level: the order is not well founded
}

/// Enough of an expression printer for diagnostics, and for keying atoms: two
/// sub-expressions share an atom only when they are written the same way.
fn show(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(n) => n.to_string(),
        ExprKind::Var(n) | ExprKind::Ctor(n) => n.clone(),
        ExprKind::Neg(a) => format!("-{}", show(a)),
        ExprKind::Binop(op, a, b) => format!("({} {} {})", show(a), op, show(b)),
        ExprKind::Field(a, f) => format!("{}.{}", show(a), f),
        ExprKind::Borrow(a) => format!("&{}", show(a)),
        ExprKind::App(h, args) => {
            let a: Vec<String> = args.iter().map(show).collect();
            format!("({} {})", show(h), a.join(" "))
        }
        k => format!("{:?}", k),
    }
}

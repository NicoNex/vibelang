//! Termination (spec §6). Every recursive function needs a measure that
//! decreases on a well-founded order at each recursive call. The measure is
//! inferred when a parameter decreases syntactically everywhere (§6.2), and
//! otherwise written by hand with `%`.
//!
//! ponytail: the order is a single linear expression over the parameters, not a
//! lexicographic tuple (§6.4). A mutually recursive group is accepted when every
//! member has a measure that drops at every call inside the group — the first
//! component of that tuple. Upgrade path: keep a Vec<Lin> per function and
//! compare lexicographically.

use crate::ast::*;
use crate::diag::{Diag, Span};
use std::collections::HashMap;

pub fn check(m: &Module) -> Vec<Diag> {
    let funs: Vec<&FunDecl> = m.funs().collect();
    let index: HashMap<&str, usize> =
        funs.iter().enumerate().map(|(i, f)| (f.name.as_str(), i)).collect();
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
    let mut measures: Vec<Option<Lin>> = vec![None; n];
    for i in 0..n {
        if !reach[i][i] {
            continue; // not recursive: nothing to prove
        }
        let f = funs[i];
        let path = format!("{}.{}", m.name, f.name);
        match &f.measure {
            Some(e) => {
                let l = lin(e);
                if l.terms.keys().all(|k| params[i].contains(k)) {
                    measures[i] = Some(l);
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
                        .with_fix("write the measure with `+`/`-` over scalar parameters, e.g. `%(n-k)`"),
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
                    solo && calls[i].iter().all(|c| decreases(&cand, &cand, &params[i], &c.args))
                });
                match inferred {
                    Some(p) => measures[i] = Some(Lin::var(p)),
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
                                format!("`{}` is mutually recursive, so its measure is not inferred", f.name)
                            },
                        )
                        .with_path(&path)
                        .with_fix("add a measure after the body, e.g. `%(n-k)`"),
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
            let Some(mj) = &measures[c.callee] else { continue };
            if decreases(mi, mj, &params[c.callee], &c.args) {
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
                .with_path(&format!("{}.{}", m.name, funs[i].name))
                .with_witness(&format!("measure {}", mi.show()))
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
    f.params.iter().flat_map(|p| p.names.iter().cloned()).collect()
}

/// Direct calls to module functions. ponytail: a recursive name used as a value
/// (`map f xs`) is not an edge, so recursion through higher-order code is not
/// seen. Upgrade path: treat any occurrence of the name as an edge with unknown
/// arguments.
fn calls_in(body: &Expr, index: &HashMap<&str, usize>) -> Vec<Call> {
    let mut out = Vec::new();
    walk(body, &mut |e| {
        if let ExprKind::App(h, args) = &e.kind {
            if let ExprKind::Var(name) = &h.kind {
                if let Some(&i) = index.get(name.as_str()) {
                    out.push(Call { callee: i, args: args.clone(), span: e.span });
                }
            }
        }
    });
    out
}

fn walk(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match &e.kind {
        ExprKind::App(h, args) => {
            walk(h, f);
            for a in args {
                walk(a, f);
            }
        }
        ExprKind::Binop(_, a, b) | ExprKind::Bind(_, a, b) | ExprKind::Let(_, a, b) => {
            walk(a, f);
            walk(b, f);
        }
        ExprKind::Neg(a) | ExprKind::Not(a) | ExprKind::Borrow(a) | ExprKind::Field(a, _) => {
            walk(a, f)
        }
        ExprKind::Lambda(_, b) => walk(b, f),
        ExprKind::Match(s, arms) => {
            walk(s, f);
            for (_, b) in arms {
                walk(b, f);
            }
        }
        ExprKind::Record(base, fields) => {
            if let Some(b) = base {
                walk(b, f);
            }
            for (_, v) in fields {
                walk(v, f);
            }
        }
        ExprKind::Tuple(xs) | ExprKind::List(xs) => {
            for x in xs {
                walk(x, f);
            }
        }
        ExprKind::Arena(_, b) => walk(b, f),
        _ => {}
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
        let mut parts: Vec<String> = self.terms.iter().map(|(a, v)| format!("{}*{}", v, a)).collect();
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

/// True when the callee's measure, with the call arguments in place of its
/// parameters, is below the caller's measure by a positive constant — the other
/// atoms must cancel exactly, so the drop holds for every value they take.
fn decreases(caller: &Lin, callee: &Lin, callee_params: &[String], args: &[Expr]) -> bool {
    if args.len() != callee_params.len() {
        return false; // partial application: nothing to substitute into
    }
    let mut d = caller.clone();
    for (atom, k) in &callee.terms {
        match callee_params.iter().position(|p| p == atom) {
            Some(i) => d.add_scaled(&lin(&args[i]), -k),
            None => return false,
        }
    }
    d.c -= callee.c;
    d.terms.is_empty() && d.c > 0
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

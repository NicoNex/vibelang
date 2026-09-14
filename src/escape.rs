//! Escape analysis, in the one form the bootstrap runtime can act on (spec
//! §4.6): which functions may release everything they allocated when they
//! return.
//!
//! The runtime already refuses to release when the result itself is heap
//! allocated — `vb_release` cancels on a string, object, vector or closure —
//! so a value that escapes through the return is handled dynamically and for
//! free. What it cannot see is a pointer that escaped some other way, and in a
//! language with no globals and no mutation of borrowed values there is exactly
//! one such way: handing the pointer to C, which may keep it for as long as it
//! likes.
//!
//! So a function may release iff neither it nor anything it can reach calls an
//! `ext c` symbol. The taint propagates to callers because the release happens
//! at the outermost frame: if `f` releases and `g` below it gave a pointer to
//! C, `f`'s release frees what C is still holding.
//!
//! ponytail: the ceiling is the whole-function granularity. A tail-recursive
//! loop marks once and releases once, so its iterations still accumulate; the
//! upgrade path is a mark per loop iteration, which needs to know which values
//! cross the back edge.

use crate::ast::*;
use crate::infer::Checked;
use std::collections::{HashMap, HashSet};

/// Names of the functions whose body may be bracketed by `vb_mark` /
/// `vb_release`.
pub fn releasable(m: &Module, ck: &Checked) -> HashSet<String> {
    let mut calls: HashMap<String, Vec<String>> = HashMap::new();
    let mut tainted: HashSet<String> = HashSet::new();
    for f in m.funs() {
        let mut named = Vec::new();
        collect(&f.body, &mut named);
        if named.iter().any(|n| ck.ext.contains_key(n)) {
            tainted.insert(f.name.clone());
        }
        calls.insert(f.name.clone(), named);
    }
    // A caller of a tainted function is tainted: the release would happen above
    // the frame that handed the pointer out.
    loop {
        let grown: Vec<String> = calls
            .iter()
            .filter(|(n, cs)| !tainted.contains(*n) && cs.iter().any(|c| tainted.contains(c)))
            .map(|(n, _)| n.clone())
            .collect();
        if grown.is_empty() {
            break;
        }
        tainted.extend(grown);
    }
    m.funs()
        .filter(|f| !f.ghost && !tainted.contains(&f.name))
        .map(|f| f.name.clone())
        .collect()
}

/// Every name an expression mentions. Over-approximate on purpose: a name that
/// is not a call costs one comparison and never costs correctness.
fn collect(e: &Expr, out: &mut Vec<String>) {
    use ExprKind::*;
    if let Var(n) | Ctor(n) = &e.kind {
        out.push(n.clone());
    }
    match &e.kind {
        Int(_) | Float(_) | Str(_) | Char(_) | Bool(_) | Unit | Var(_) | Ctor(_) => {}
        App(h, args) => {
            collect(h, out);
            args.iter().for_each(|a| collect(a, out));
        }
        Binop(_, a, b) | Bind(_, a, b) | Let(_, a, b) => {
            collect(a, out);
            collect(b, out);
        }
        Neg(i) | Not(i) | Borrow(i) | Field(i, _) | Lambda(_, i) | Arena(_, i) => collect(i, out),
        Match(s, arms) => {
            collect(s, out);
            arms.iter().for_each(|(_, b)| collect(b, out));
        }
        Record(base, fields) => {
            base.iter().for_each(|b| collect(b, out));
            fields.iter().for_each(|(_, v)| collect(v, out));
        }
        Tuple(xs) | List(xs) => xs.iter().for_each(|x| collect(x, out)),
    }
}

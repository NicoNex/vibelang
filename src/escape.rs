//! Escape analysis (spec §4.6): which functions can be trusted to free what
//! they allocated.
//!
//! The bulk release this used to gate is gone — there is no region and no
//! frontier, every value is freed where its owner dies. The question underneath
//! it survives unchanged, because it was never about regions: in a language
//! with no globals and no mutation of borrowed values there is exactly one way
//! a pointer escapes without the compiler seeing it, and that is handing it to
//! C, which may keep it for as long as it likes.
//!
//! So a function is trustworthy iff neither it nor anything it can reach calls
//! an `ext c` symbol, and the taint propagates to callers: if `f` frees a value
//! it lent to `g`, and `g` gave a pointer into it to C, `f` freed what C is
//! still holding. `own.rs` reads this set and refuses to place a drop on
//! anything a tainted call was given.

use crate::ast::*;
use crate::infer::Checked;
use std::collections::{HashMap, HashSet};

/// Names of the functions that cannot have handed a pointer to C.
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

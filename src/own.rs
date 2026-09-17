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
use crate::types::{is_num, is_special, PRELUDE_SHARES, PRELUDE_SIGS};
use std::collections::{HashMap, HashSet};

pub fn check(m: &Module, ck: &Checked) -> Vec<Diag> {
    run(m, ck).0
}

/// Spans of `{r with ...}` updates whose base is a uniquely owned value, so
/// codegen can mutate in place instead of copying (spec §4.3).
pub fn inplace_updates(m: &Module, ck: &Checked) -> HashSet<(usize, usize, usize)> {
    run(m, ck).1
}

/// One place an owned value stops being its scope's to free: the binder's name,
/// the span the drop goes after, and the function it is in
/// (docs/static-drop-roadmap.md).
#[derive(Clone, Debug)]
pub struct DropSite {
    pub name: String,
    pub at: Span,
    pub path: String,
    pub when: DropWhen,
}

/// Where the free goes relative to the expression at `at`. The back edge is the
/// case the backend cannot guess: `spin (k-1) (concat &s "x")` *reads* `s` to
/// build the new argument, so the free belongs between the argument and the
/// assignment that overwrites the parameter with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropWhen {
    /// after the expression at `at`, which is the end of the value's scope
    ScopeEnd,
    /// before the parameter assignments of the self-tail-call at `at`
    BackEdge,
    /// after the call that borrowed the unnamed value computed at `at`. A
    /// borrow lives only for the call that lent it (§4.4), so the temporary is
    /// dead the moment the call returns, and no name ever held it.
    Temp,
}

/// Where every owned value dies. The same traversal that reports affine misuse
/// computes it, so the two cannot disagree. Codegen emits a free at each one.
pub fn drop_points(m: &Module, ck: &Checked) -> Vec<DropSite> {
    let mut v = run(m, ck).2;
    v.sort_by_key(|d| (d.at.file, d.at.line, d.at.col, d.name.clone()));
    v
}

/// How many drops the maybe-shared bit suppressed. The count is the distance
/// between what this frees and what it could free if aliasing were checked
/// rather than assumed (docs/aliasing-audit.md).
pub fn shared_suppressed(m: &Module, ck: &Checked) -> usize {
    run(m, ck).3
}

type Analysis = (
    Vec<Diag>,
    HashSet<(usize, usize, usize)>,
    Vec<DropSite>,
    usize,
);

fn run(m: &Module, ck: &Checked) -> Analysis {
    let sigs = borrowed_params(m);
    // `escape::releasable` answers whether a call may put a pointer somewhere
    // this frame cannot see; a value that reaches one is not freed here.
    let releasable = crate::escape::releasable(m, ck);
    let reaches_c: HashSet<String> = m
        .exts()
        .flat_map(|e| e.sigs.iter().map(|s| s.name.clone()))
        .chain(
            m.funs()
                .filter(|f| !releasable.contains(&f.name))
                .map(|f| f.name.clone()),
        )
        .collect();
    let mut out = Vec::new();
    let mut inplace = HashSet::new();
    let mut drops = Vec::new();
    let mut suppressed = 0usize;
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
                .with_path(&crate::ast::path(&f.home, &f.name))
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
        // A scalar result is a copy and cannot point at anything, so only an
        // affine return type can carry a piece of a borrow back out.
        if !borrows.is_empty()
            && f.ret
                .as_ref()
                .is_some_and(|t| !matches!(t, Ty::Ref(_)) && is_affine(t))
        {
            let mut tails = Vec::new();
            borrow_out(&f.body, &borrows, &mut tails);
            for (n, span, how) in tails {
                out.push(
                    Diag::error(
                        span,
                        "own.borrow_escapes",
                        &format!("this result is {} `{}`, which is a borrow", how, n),
                    )
                    .with_path(&crate::ast::path(&f.home, &f.name))
                    .with_witness("a borrow lives only for the call that lent it")
                    .with_fix(&format!(
                        "copy it with `dup &(...)`, or return an index into `{}`",
                        n
                    )),
                );
            }
        }
        let mut escaping = HashSet::new();
        escapes(&f.body, &mut escaping);
        let mut leaves = HashSet::new();
        reaches_result(&f.body, &mut leaves);
        let mut st = State {
            moved: HashMap::new(),
            drops: Vec::new(),
            mutated: HashMap::new(),
            errors: Vec::new(),
            inplace: HashSet::new(),
            path: crate::ast::path(&f.home, &f.name),
            ck,
            sigs: &sigs,
            escaping,
            leaves,
            exts: reaches_c.clone(),
            fun: f.name.clone(),
            arity: f.arity(),
            shared: borrows.iter().map(|b| (*b).to_string()).collect(),
            suppressed: 0,
        };
        let mut owned: Vec<&str> = Vec::new();
        for p in &f.params {
            if p.ty.as_ref().is_some_and(is_affine) {
                owned.extend(p.names.iter().map(|s| s.as_str()));
            }
        }
        st.walk(&f.body, Mode::Own, &owned);
        // A parameter is owned by the frame that received it. One the body
        // never hands on dies with that frame.
        for n in &owned {
            if !st.moved.contains_key(*n) && !st.leaves.contains(*n) {
                st.drop_site(n, f.body.span, DropWhen::ScopeEnd);
            }
        }
        out.append(&mut st.errors);
        inplace.extend(st.inplace.drain());
        drops.append(&mut st.drops);
        suppressed += st.suppressed;
    }
    (out, inplace, drops, suppressed)
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
fn borrow_out<'a>(e: &'a Expr, borrows: &[&str], out: &mut Vec<(&'a str, Span, &'static str)>) {
    use ExprKind::*;
    let go = |x: &'a Expr, out: &mut Vec<_>| borrow_out(x, borrows, out);
    match &e.kind {
        Var(n) => {
            if borrows.contains(&n.as_str()) {
                out.push((n.as_str(), e.span, "the borrow"));
            }
        }
        // `r.f` is a pointer into `r`; `&x` in a result position is `x`.
        Field(b, _) | Borrow(b) => {
            let mut inner = Vec::new();
            borrow_out(b, borrows, &mut inner);
            out.extend(inner.into_iter().map(|(n, _, _)| (n, e.span, "a part of")));
        }
        Let(_, _, b) | Bind(_, _, b) | Arena(_, b) => go(b, out),
        Match(_, arms) => arms.iter().for_each(|(_, a)| go(a, out)),
        Tuple(xs) | List(xs) => xs.iter().for_each(|x| go(x, out)),
        Record(base, fields) => {
            if let Some(b) = base {
                go(b, out);
            }
            fields.iter().for_each(|(_, v)| go(v, out));
        }
        // A prelude function that hands out an element of what it was given
        // passes the provenance through with it: `max_by amt ts` is one of
        // `ts`, and `get ts i` is another. A module function does not, because
        // this check is what makes that true.
        //
        // `fold` is in `PRELUDE_SHARES` and is not one of these. Its result is
        // the accumulator, which is an element of the vector only when the
        // function it was given returns its own argument — `fold (\a x -> x) z
        // v`. So the question goes to the function, and `fold insert dict ws`,
        // which builds something new out of every element, is not an escape.
        App(h, xs) => {
            let shares = match &h.kind {
                Var(n) if n == "fold" => xs.iter().any(lambda_returns_its_argument),
                Var(n) => PRELUDE_SHARES.contains(&n.as_str()),
                _ => false,
            };
            if shares {
                let mut inner = Vec::new();
                for x in xs {
                    borrow_out(x, borrows, &mut inner);
                }
                out.extend(inner.into_iter().map(|(n, _, _)| (n, e.span, "a part of")));
            }
        }
        _ => {}
    }
}

/// Names whose value can leave through the result: the tail expression itself,
/// a component of a structure built there, a base whose field is read there —
/// `r.f` is a pointer into `r` — or anything a closure returned from there can
/// read. A value the caller receives is the caller's to free.
///
/// This is the one place where erring wide is the safe direction: suppressing a
/// drop that was not needed leaks, and missing one frees memory the caller is
/// about to read.
///
/// ponytail: names, not spans, so an inner binder that shadows one of these is
/// suppressed with it — a leak, never a use-after-free. Upgrade path: key on
/// the binder's span once drops are emitted.
fn reaches_result(e: &Expr, out: &mut HashSet<String>) {
    use ExprKind::*;
    match &e.kind {
        Var(n) => {
            out.insert(n.clone());
        }
        // `r.f` hands out a pointer into `r`, so `r` leaves with it.
        Field(b, _) | Borrow(b) => reaches_result(b, out),
        Let(_, _, b) | Bind(_, _, b) | Arena(_, b) => reaches_result(b, out),
        Match(_, arms) => arms.iter().for_each(|(_, a)| reaches_result(a, out)),
        Tuple(xs) | List(xs) => xs.iter().for_each(|x| reaches_result(x, out)),
        Record(base, fields) => {
            if let Some(b) = base {
                reaches_result(b, out);
            }
            fields.iter().for_each(|(_, v)| reaches_result(v, out));
        }
        // Everything a returned closure reads, it may carry out with it (§4.6).
        Lambda(_, b) => names_in(b, out),
        // A call's result is the callee's value; what was passed to it was
        // moved there, and the move already suppressed the drop.
        _ => {}
    }
}

/// Every name the expression mentions, at any depth.
fn names_in(e: &Expr, out: &mut HashSet<String>) {
    if let ExprKind::Var(n) = &e.kind {
        out.insert(n.clone());
    }
    e.children(&mut |c| names_in(c, out));
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
    /// Binders still owned when their scope ends: the drop table.
    drops: Vec<DropSite>,
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
    /// Names whose value can leave through the result: never dropped here.
    leaves: HashSet<String>,
    /// Names that may point into a value the caller owns, so this frame must
    /// not free them (docs/aliasing-audit.md). Seeded with the borrowed
    /// parameters and grown by `maybe_shared`.
    shared: HashSet<String>,
    /// How many drops that bit suppressed: the distance between what is freed
    /// and what could be.
    suppressed: usize,
    /// Callees that may hand a pointer to C — an `ext c` symbol, or a function
    /// that reaches one (`escape::releasable`). C keeps what it likes for as
    /// long as it likes, so nothing this frame lent to such a call is freed
    /// here (spec §4.6, §10.1).
    exts: HashSet<String>,
    /// This function's name and how many arguments a saturated call takes: a
    /// call matching both is the back edge codegen compiles to `continue`.
    fun: String,
    arity: usize,
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
            if self.ck.affine.contains_key(&k) {
                v.push(n);
            }
        }
        v
    }

    /// Whether this expression can evaluate to a pointer into a value some
    /// other name still owns. A prelude call that hands out an interior pointer
    /// is one (`PRELUDE_SHARES`); a field read is one; anything built out of a
    /// shared name is one.
    ///
    /// ponytail: a call to a *module* function counts as fresh. A user function
    /// that returns a value derived from a `&` parameter would break that, and
    /// `own.borrow_escapes` is what keeps the shape out of the language —
    /// deciding it in general is region inference (docs/aliasing-audit.md,
    /// "the part that really is bigger than the plan").
    fn maybe_shared(&self, e: &Expr) -> bool {
        use ExprKind::*;
        match &e.kind {
            Var(n) => self.shared.contains(n),
            // `r.f` is a pointer into `r`, whoever owns `r`.
            Field(..) => true,
            Borrow(x) | Neg(x) | Not(x) | Arena(_, x) => self.maybe_shared(x),
            Let(_, _, b) | Bind(_, _, b) => self.maybe_shared(b),
            Match(_, arms) => arms.iter().any(|(_, a)| self.maybe_shared(a)),
            Tuple(xs) | List(xs) => xs.iter().any(|x| self.maybe_shared(x)),
            Record(base, fields) => {
                base.as_ref().is_some_and(|b| self.maybe_shared(b))
                    || fields.iter().any(|(_, v)| self.maybe_shared(v))
            }
            // A prelude name that hands out an interior pointer, or a call
            // given a lambda that returns a piece of its own argument. The
            // second is `map (\x -> x) v` and `map (\x -> x.s) v`: the elements
            // the caller gets back are the ones it lent. Rejecting the shape
            // would need the lambda's result type, which inference does not
            // record for a lambda; suppressing the drop needs nothing and is
            // the trade this file makes everywhere else — a leak, never a
            // use-after-free (docs/aliasing-audit.md, gap 7).
            App(h, xs) => {
                matches!(&h.kind, Var(n) if PRELUDE_SHARES.contains(&n.as_str()))
                    || xs.iter().any(lambda_returns_its_argument)
            }
            _ => false,
        }
    }

    /// Record a drop, unless the value may belong to someone else as well.
    fn drop_site(&mut self, name: &str, at: Span, when: DropWhen) {
        if self.shared.contains(name) {
            self.suppressed += 1;
            return;
        }
        self.drops.push(DropSite {
            name: name.to_string(),
            at,
            path: self.path.clone(),
            when,
        });
    }

    fn use_var(&mut self, n: &str, span: Span, mode: Mode, owned: &[&str]) {
        if !owned.contains(&n) {
            return;
        }
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
                    .with_fix(&format!(
                        "read `{}` before the move, or copy it with `dup {}`",
                        n, n
                    )),
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
                // `dup` is `&a -> a` and copies in depth, so it type-checks
                // and produces an independently owned value for any type.
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
                // A saturated self-call is not a call: codegen overwrites the
                // parameters and loops (§11.2). A parameter still owned here is
                // about to be unreachable, so it dies on this edge — after the
                // new argument has read it, before the assignment.
                let back_edge = matches!(&h.kind, Var(n)
                    if *n == self.fun && args.len() == self.arity && !owned.contains(&n.as_str()));
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
                // A closure handed to a prelude function is consumed by the
                // call — `map`, `filter`, `fold`, `each`, `sort_by` all call it
                // and drop it. A closure handed to a module function may be
                // stored and returned, and §4.6 says outright that the analysis
                // cannot decide that case. The spec's answer is an error
                // demanding an explicit `move`; the answer here is the same one
                // the may-alias family gets, because the direction that matters
                // is the same: what the closure captured may outlive this
                // frame, so this frame does not free it.
                let keeps_closures = !matches!(&h.kind, Var(n)
                    if PRELUDE_SIGS.iter().any(|(p, _)| p == n));
                for (i, a) in args.iter().enumerate() {
                    // The callee's own signature, not the position this call
                    // sits in: `out (id "ok")` is a borrow position for `out`,
                    // and says nothing about what `id` does with its argument.
                    let lent = reads_all || borrows.is_some_and(|b| *b.get(i).unwrap_or(&false));
                    // Nor does that position decide what the callee takes: in
                    // `fmt "{}" (fold f acc v)` the fold is read, but `acc` is
                    // still handed to it.
                    let m = if lent { Mode::Borrow } else { Mode::Own };
                    if matches!(a.kind, Lambda(..)) {
                        if keeps_closures {
                            let mut captured = HashSet::new();
                            names_in(a, &mut captured);
                            self.shared.extend(captured);
                        } else {
                            // A closure a prelude function is given is called
                            // and finished with during the call, and no name
                            // ever held it: `map (\x -> x+1) &v` inside a loop
                            // allocates one per iteration.
                            self.drops.push(DropSite {
                                name: String::new(),
                                at: a.span,
                                path: self.path.clone(),
                                when: DropWhen::Temp,
                            });
                        }
                    }
                    // `len &(range 0 n)` allocates a vector no name ever holds.
                    // The callee borrowed it, so it is dead when the call
                    // returns, and it is the shape `examples/churn.vibe` is
                    // about: without this the value would live to the end of
                    // the frame, or, inside a loop, for ever.
                    // A callee whose result can point into what it was lent
                    // keeps the value alive past its own return, and so does C.
                    let lends_onward = matches!(&h.kind, Var(n)
                        if PRELUDE_SHARES.contains(&n.as_str()) || self.exts.contains(n));
                    // What went to C is C's for as long as it likes: no name
                    // mentioned in that argument is this frame's to free.
                    if matches!(&h.kind, Var(n) if self.exts.contains(n)) {
                        let mut given = HashSet::new();
                        names_in(a, &mut given);
                        self.shared.extend(given);
                    }
                    if lent && !lends_onward {
                        let inner = match &a.kind {
                            Borrow(x) => x,
                            _ => a,
                        };
                        // Whether the source wrote the `&` does not matter:
                        // the position is a borrow either way.
                        // A name is somebody's and a scalar literal is nobody's
                        // allocation, so neither is a temporary to free.
                        let computed = !matches!(
                            inner.kind,
                            Var(_)
                                | Field(..)
                                | Borrow(_)
                                | Int(_)
                                | Float(_)
                                | Bool(_)
                                | Char(_)
                                | Unit
                        );
                        if computed && !self.maybe_shared(inner) {
                            self.drops.push(DropSite {
                                name: String::new(),
                                at: inner.span,
                                path: self.path.clone(),
                                when: DropWhen::Temp,
                            });
                        }
                    }
                    self.walk(a, m, owned);
                }
                if back_edge {
                    for n in owned {
                        if !self.moved.contains_key(*n) && !self.leaves.contains(*n) {
                            self.drop_site(n, e.span, DropWhen::BackEdge);
                        }
                    }
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
                            && owned.contains(&n.as_str())
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
                let was = self.shared.contains(n);
                if self.maybe_shared(v) {
                    self.shared.insert(n.clone());
                }
                let bound = [n.clone()];
                let inner = self.scope(owned, &bound, body.span);
                self.walk(body, mode, &inner);
                // Affine, and nothing took it: the value is still this scope's
                // when the scope ends, so this is where it dies.
                if inner.contains(&n.as_str())
                    && !self.moved.contains_key(n)
                    && !self.leaves.contains(n)
                {
                    self.drop_site(n, body.span, DropWhen::ScopeEnd);
                }
                // The name leaves scope here: a later binder of the same name
                // is a different value, and neither its move nor its sharing is
                // this one's.
                self.moved.remove(n);
                if !was {
                    self.shared.remove(n);
                }
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
                // An in-place update in one arm is not seen by its siblings.
                let updated_before = self.mutated.clone();
                let mut updated_after = updated_before.clone();
                let mut kept: Vec<(String, Span)> = Vec::new();
                for (p, body) in arms {
                    self.moved = before.clone();
                    self.mutated = updated_before.clone();
                    let bound = p.names();
                    // The scrutinee is read, not consumed, so a payload the
                    // pattern names is a pointer into a value someone else
                    // still owns (docs/aliasing-audit.md gap 4).
                    let mut fresh: Vec<String> = Vec::new();
                    for n in &bound {
                        if self.shared.insert(n.clone()) {
                            fresh.push(n.clone());
                        }
                    }
                    // A binder is a new name: an outer one of the same name
                    // moved earlier says nothing about it inside the arm.
                    let shadowed: Vec<(String, Span)> = bound
                        .iter()
                        .filter_map(|n| self.moved.remove(n).map(|at| (n.clone(), at)))
                        .collect();
                    let visible = self.scope(owned, &bound, body.span);
                    self.walk(body, mode, &visible);
                    // A payload handed on takes the scrutinee's memory with it,
                    // so the scrutinee is consumed on this path; freeing it would
                    // free what the arm just gave away.
                    // ponytail: the scrutinee's outer cell leaks on that path; a
                    // shell-only free needs Task 8's per-type `_Drop_T`.
                    if let ExprKind::Var(sv) = &scrut.kind {
                        if owned.contains(&sv.as_str()) && !self.moved.contains_key(sv) {
                            if let Some(at) = bound.iter().find_map(|n| self.moved.get(n).copied())
                            {
                                self.moved.insert(sv.clone(), at);
                            }
                        }
                    }
                    for n in &fresh {
                        self.shared.remove(n);
                    }
                    for n in &bound {
                        self.moved.remove(n);
                    }
                    self.moved.extend(shadowed);
                    for n in &visible {
                        if !self.moved.contains_key(*n) && !self.leaves.contains(*n) {
                            kept.push(((*n).to_string(), body.span));
                        }
                    }
                    for (k, v) in self.moved.drain() {
                        after.entry(k).or_insert(v);
                    }
                    for (k, v) in self.mutated.drain() {
                        updated_after.entry(k).or_insert(v);
                    }
                }
                // An arm that still owns a value another arm consumed frees it
                // where the arm ends: after the match, the consuming path would
                // free it a second time. A value no arm consumes is nobody's
                // business here — the scope that binds it drops it.
                //
                // ponytail: a payload the pattern binds is part of the
                // scrutinee, so it is freed with the scrutinee and never here.
                // Upgrade path: drop a payload separately once a value can be
                // taken apart, which needs Task 8's per-type `_Drop_T`.
                for (n, at) in kept {
                    if after.contains_key(&n) {
                        self.drop_site(&n, at, DropWhen::ScopeEnd);
                    }
                }
                self.moved = after;
                self.mutated = updated_after;
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

/// Whether this argument is a lambda whose result is its own parameter, or a
/// piece of one. A lambda handed to `map`, `filter` or `sort_by` is called on
/// the elements of a borrowed vector, so such a result is interior to a value
/// the caller still owns.
fn lambda_returns_its_argument(e: &Expr) -> bool {
    let ExprKind::Lambda(ps, body) = &e.kind else {
        return false;
    };
    let ps: Vec<&str> = ps.iter().map(|s| s.as_str()).collect();
    let mut out = Vec::new();
    borrow_out(body, &ps, &mut out);
    !out.is_empty()
}

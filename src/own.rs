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
}

/// Where every owned value dies. The same traversal that reports affine misuse
/// computes it, so the two cannot disagree. Nothing consumes this yet: the
/// frontend records the points and the backend still brackets whole frames.
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
        let mut leaves = HashSet::new();
        reaches_result(&f.body, &mut leaves);
        let mut st = State {
            moved: HashMap::new(),
            drops: Vec::new(),
            mutated: HashMap::new(),
            errors: Vec::new(),
            inplace: HashSet::new(),
            path: format!("{}.{}", f.home, f.name),
            ck,
            sigs: &sigs,
            escaping,
            leaves,
            fun: f.name.clone(),
            arity: f.params.iter().map(|p| p.names.len()).sum(),
            shared: borrows.iter().map(|b| (*b).to_string()).collect(),
            suppressed: 0,
        };
        let mut owned: Vec<(&str, bool)> = Vec::new();
        for p in &f.params {
            if p.ty.as_ref().is_some_and(is_affine) {
                let is_str = matches!(&p.ty, Some(Ty::Con(n, _)) if n == "Str");
                owned.extend(p.names.iter().map(|s| (s.as_str(), is_str)));
            }
        }
        st.walk(&f.body, Mode::Own, &owned);
        // A parameter is owned by the frame that received it. One the body
        // never hands on dies with that frame.
        for (n, _) in &owned {
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
fn returned<'a>(e: &'a Expr, out: &mut Vec<(&'a str, Span)>) {
    use ExprKind::*;
    match &e.kind {
        Var(n) => out.push((n.as_str(), e.span)),
        Let(_, _, b) | Bind(_, _, b) | Arena(_, b) => returned(b, out),
        Match(_, arms) => arms.iter().for_each(|(_, a)| returned(a, out)),
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
    use ExprKind::*;
    match &e.kind {
        Var(n) => {
            out.insert(n.clone());
        }
        App(h, xs) => {
            names_in(h, out);
            xs.iter().for_each(|x| names_in(x, out));
        }
        Binop(_, a, b) | Let(_, a, b) | Bind(_, a, b) => {
            names_in(a, out);
            names_in(b, out);
        }
        Neg(x) | Not(x) | Borrow(x) | Field(x, _) | Lambda(_, x) | Arena(_, x) => names_in(x, out),
        Tuple(xs) | List(xs) => xs.iter().for_each(|x| names_in(x, out)),
        Record(base, fields) => {
            if let Some(b) = base {
                names_in(b, out);
            }
            fields.iter().for_each(|(_, v)| names_in(v, out));
        }
        Match(s, arms) => {
            names_in(s, out);
            arms.iter().for_each(|(_, a)| names_in(a, out));
        }
        Int(_) | Float(_) | Str(_) | Char(_) | Bool(_) | Unit | Ctor(_) => {}
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
    /// This function's name and how many arguments a saturated call takes: a
    /// call matching both is the back edge codegen compiles to `continue`.
    fun: String,
    arity: usize,
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
            App(h, _) => matches!(&h.kind, Var(n) if PRELUDE_SHARES.contains(&n.as_str())),
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
                // A saturated self-call is not a call: codegen overwrites the
                // parameters and loops (§11.2). A parameter still owned here is
                // about to be unreachable, so it dies on this edge — after the
                // new argument has read it, before the assignment.
                let back_edge = matches!(&h.kind, Var(n)
                    if *n == self.fun && args.len() == self.arity && !owned.iter().any(|(x, _)| *x == n));
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
                    let m = if reads_all || borrows.is_some_and(|b| *b.get(i).unwrap_or(&false)) {
                        Mode::Borrow
                    } else {
                        mode
                    };
                    if keeps_closures && matches!(a.kind, Lambda(..)) {
                        let mut captured = HashSet::new();
                        names_in(a, &mut captured);
                        self.shared.extend(captured);
                    }
                    self.walk(a, m, owned);
                }
                if back_edge {
                    for (n, _) in owned {
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
                let was = self.shared.contains(n);
                if self.maybe_shared(v) {
                    self.shared.insert(n.clone());
                }
                let bound = [n.clone()];
                let inner = self.scope(owned, &bound, body.span);
                self.walk(body, mode, &inner);
                // Affine, and nothing took it: the value is still this scope's
                // when the scope ends, so this is where it dies.
                if inner.iter().any(|(x, _)| *x == n.as_str())
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
                let mut kept: Vec<(String, Span)> = Vec::new();
                for (p, body) in arms {
                    self.moved = before.clone();
                    let bound = pat_names(p);
                    // The scrutinee is read, not consumed, so a payload the
                    // pattern names is a pointer into a value someone else
                    // still owns (docs/aliasing-audit.md gap 4).
                    let mut fresh: Vec<String> = Vec::new();
                    for n in &bound {
                        if self.shared.insert(n.clone()) {
                            fresh.push(n.clone());
                        }
                    }
                    let visible = self.scope(owned, &bound, body.span);
                    self.walk(body, mode, &visible);
                    for n in &fresh {
                        self.shared.remove(n);
                    }
                    for (n, _) in &visible {
                        if !self.moved.contains_key(*n) && !self.leaves.contains(*n) {
                            kept.push(((*n).to_string(), body.span));
                        }
                    }
                    for (k, v) in self.moved.drain() {
                        after.entry(k).or_insert(v);
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

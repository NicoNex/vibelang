//! Types, unification, the prelude, and inference.
//!
//! Bootstrap simplifications, all deliberate:
//!   * `&T` is erased — affine use is checked in `own`, but there is no full
//!     borrow checker (spec fase 4).
//!   * refinements are checked at run time, not discharged to SMT (spec fase 6).

use crate::ast::*;
use crate::diag::{Diag, Span};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub enum T {
    Var(usize),
    Con(String, Vec<T>),
    Fun(Box<T>, Box<T>),
    Tuple(Vec<T>),
    Eff(Box<T>),
}

impl T {
    pub fn con(n: &str) -> T {
        T::Con(n.into(), vec![])
    }
    pub fn unit() -> T {
        T::con("Unit")
    }
    pub fn show(&self) -> String {
        match self {
            T::Var(i) => format!("t{}", i),
            T::Con(n, a) if a.is_empty() => n.clone(),
            T::Con(n, a) => format!(
                "{} {}",
                n,
                a.iter()
                    .map(|t| t.show_atom())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            T::Fun(a, b) => format!("{} -> {}", a.show_atom(), b.show()),
            T::Tuple(ts) => format!(
                "({})",
                ts.iter().map(|t| t.show()).collect::<Vec<_>>().join(", ")
            ),
            T::Eff(t) => format!("E! {}", t.show_atom()),
        }
    }
    fn show_atom(&self) -> String {
        match self {
            T::Con(_, a) if !a.is_empty() => format!("({})", self.show()),
            T::Fun(_, _) | T::Eff(_) => format!("({})", self.show()),
            _ => self.show(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Scheme {
    pub vars: Vec<usize>,
    pub ty: T,
}

impl Scheme {
    pub fn mono(t: T) -> Scheme {
        Scheme {
            vars: vec![],
            ty: t,
        }
    }
}

/// Numeric constraints on literal type variables, resolved by defaulting.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Any,
    Num,
    Float,
}

const INT_TYPES: &[&str] = &["U8", "U16", "U32", "U64", "I8", "I16", "I32", "I64"];
const FLOAT_TYPES: &[&str] = &["F32", "F64"];

pub fn is_num(n: &str) -> bool {
    INT_TYPES.contains(&n) || FLOAT_TYPES.contains(&n)
}

// ---------------------------------------------------------------- data decls

#[derive(Clone, Debug)]
pub struct RecordInfo {
    pub name: String,
    pub fields: Vec<(String, Ty)>,
    pub refines: Vec<Expr>,
}

#[derive(Clone, Debug)]
pub struct CtorInfo {
    pub owner: String,
    pub params: Vec<String>,
    pub args: Vec<Ty>,
    pub tag: usize,
}

#[derive(Default)]
pub struct Data {
    pub records: HashMap<String, RecordInfo>,
    pub ctors: HashMap<String, CtorInfo>,
    /// type name -> constructor names, in declaration order.
    pub variants: HashMap<String, Vec<String>>,
    pub opaque: Vec<String>,
    /// field name -> owning record type, for the one case a field has to be
    /// resolved by its name alone: a record literal or a `.field` read whose
    /// base type inference never pinned down. It holds the *first* record to
    /// declare the name, and `field_ambiguous` says when that answer is a
    /// guess rather than the answer.
    pub field_owner: HashMap<String, String>,
    /// Field names more than one record declares. Resolving one of these by
    /// name is not allowed: `Checked::field_of` carries what inference decided
    /// at each use site, and a site inference could not decide is an error
    /// rather than a coin toss.
    pub field_ambiguous: HashSet<String>,
}

pub const PRELUDE_TYPES: &str = "\
mod Prelude
type Res e t = Ok t | Er e
type Opt t = Some t | None
type Fault = Overflow | DivZero | OutOfBounds | BadParse
";

/// Prelude signatures. `name : type`, one per line. Everything here is
/// implemented by the C runtime in runtime/vibert.c.
/// Prelude names whose result *is* part of an argument rather than a fresh
/// value: `get` returns the element itself (`return s->a[k]`), `to_cstr` points
/// into a string's bytes (docs/aliasing-audit.md, gaps 2 and 8). A binder
/// holding one of these carries the *maybe-shared* bit in `own.rs`, and nothing
/// carrying it is freed by the frame that named it.
///
/// The structural vector operations — `rev`, `filter`, `push`, `map`, … — are
/// deliberately absent. They allocate a fresh spine and copy the same element
/// pointers into it (gap 3), and a drop is shallow: freeing the spine cannot
/// free an element twice. A deep drop would have to put them back.
///
/// The table is the compiler's own knowledge of its runtime, written by a human
/// once; §4.2's ban is on lifetime annotations in the source language, which an
/// entry in the compiler's builtin table is not.
pub const PRELUDE_SHARES: &[&str] = &["get", "max_by", "min_by", "sum", "fold", "seq", "to_cstr"];

pub const PRELUDE_SIGS: &[(&str, &str)] = &[
    // Copy. Deep, and generic: the copy owns everything it points at, so it is
    // the "return a copy" of spec §4.4 — one of the three answers to the absence
    // of lifetimes — for every type rather than for `Str` alone.
    ("dup", "&a -> a"),
    // Vec
    ("empty", "Vec a"),
    ("single", "a -> Vec a"),
    ("push", "Vec a -> a -> Vec a"),
    ("get", "&Vec a -> Size -> a"),
    ("set", "Vec a -> Size -> a -> Vec a"),
    ("map", "(a -> b) -> &Vec a -> Vec b"),
    ("filter", "(a -> Bool) -> &Vec a -> Vec a"),
    ("fold", "(b -> a -> b) -> b -> &Vec a -> b"),
    ("each", "(a -> E! Unit) -> &Vec a -> E! Unit"),
    ("sum", "&Vec a -> a"),
    ("max_by", "(a -> b) -> &Vec a -> a"),
    ("min_by", "(a -> b) -> &Vec a -> a"),
    ("sort_by", "(a -> b) -> &Vec a -> Vec a"),
    ("rev", "&Vec a -> Vec a"),
    ("concat_vec", "&Vec a -> &Vec a -> Vec a"),
    ("seq", "Vec (Res e t) -> Res e (Vec t)"),
    ("range", "Size -> Size -> Vec Size"),
    ("take", "Size -> &Vec a -> Vec a"),
    ("drop", "Size -> &Vec a -> Vec a"),
    // Dict. An association from a key to a value, and the one prelude type
    // with no literal syntax: `dict` is the empty one and `insert` grows it.
    // `len` and `show` work on it because they work on anything, and a
    // dictionary is a vector of pairs underneath (runtime/vibert.c).
    // A set is a `Dict k Unit`; it does not need names of its own.
    ("dict", "Dict k v"),
    ("insert", "Dict k v -> k -> v -> Dict k v"),
    ("lookup", "&Dict k v -> &k -> Opt v"),
    ("remove", "Dict k v -> &k -> Dict k v"),
    ("keys", "&Dict k v -> Vec k"),
    // Str
    ("split", "Char -> &Str -> Vec Str"),
    ("lines", "&Str -> Vec Str"),
    ("concat", "&Str -> &Str -> Str"),
    ("trim", "&Str -> Str"),
    ("starts_with", "&Str -> &Str -> Bool"),
    ("contains", "&Str -> &Str -> Bool"),
    ("to_cstr", "&Str -> CStr"),
    ("from_cstr", "CStr -> Str"),
    ("chr", "Char -> Str"),
    // `slice` is total by returning `Opt`: the bootstrap cannot yet phrase
    // `i <= j <= len s` as a refinement on a prelude name (spec §14).
    ("slice", "Size -> Size -> &Str -> Opt Str"),
    ("index_of", "&Str -> &Str -> Opt Size"),
    ("replace", "&Str -> &Str -> &Str -> Str"),
    ("lower", "&Str -> Str"),
    // conversions (no overloading: one name, one meaning)
    ("f32", "a -> F32"),
    ("f64", "a -> F64"),
    ("i8", "a -> I8"),
    ("i16", "a -> I16"),
    ("i32", "a -> I32"),
    ("i64", "a -> I64"),
    ("u8", "a -> U8"),
    ("u16", "a -> U16"),
    ("u32", "a -> U32"),
    ("u64", "a -> U64"),
    ("size", "a -> Size"),
    ("parse_i64", "&Str -> Opt I64"),
    ("parse_u32", "&Str -> Opt U32"),
    ("parse_u64", "&Str -> Opt U64"),
    ("parse_f64", "&Str -> Opt F64"),
    // Bits. Functions, not operators: `&` is the borrow sigil and `|` separates
    // match arms, and inventing symbols for the rest would add a precedence
    // table — the one where `a & b == c` means `a & (b == c)` in C. A call has
    // no precedence. `shr` is arithmetic on a signed value and logical on an
    // unsigned one, which is what the value's own type already says.
    ("band", "a -> a -> a"),
    ("bor", "a -> a -> a"),
    ("bxor", "a -> a -> a"),
    ("bnot", "a -> a"),
    ("shl", "a -> Size -> a"),
    ("shr", "a -> Size -> a"),
    ("ord", "Char -> U32"),
    // Math
    ("abs", "a -> a"),
    ("min", "a -> a -> a"),
    ("max", "a -> a -> a"),
    ("sqrt", "F64 -> F64"),
    ("pow", "F64 -> F64 -> F64"),
    ("floor", "F64 -> F64"),
    // IO (all effectful)
    ("read", "&Str -> E! Str"),
    ("write", "&Str -> &Str -> E! Unit"),
    ("out", "&Str -> E! Unit"),
    ("warn", "&Str -> E! Unit"),
    ("argv", "E! Vec Str"),
    ("read_stdin", "E! Str"),
    ("exit", "I32 -> E! Unit"),
    // Checked: moves the obligation to run time (spec §7.5)
    ("add_checked", "a -> a -> Res Fault a"),
    ("sub_checked", "a -> a -> Res Fault a"),
    ("mul_checked", "a -> a -> Res Fault a"),
    ("div_checked", "a -> a -> Res Fault a"),
    ("get_checked", "&Vec a -> Size -> Res Fault a"),
];

/// Names typed by a rule rather than by a signature.
pub fn is_special(name: &str) -> bool {
    matches!(name, "fmt" | "show" | "len")
}

// ------------------------------------------------------------------ checker

pub struct Checker {
    pub subst: Vec<Option<T>>,
    pub kinds: Vec<Kind>,
    pub env: Vec<HashMap<String, Scheme>>,
    pub data: Data,
    pub sigs: HashMap<String, Scheme>,
    pub errors: Vec<Diag>,
    /// Every `let`, `<-` and pattern binder, keyed by the span of the
    /// expression it scopes over plus its name, with the type it was given.
    /// Ownership needs this: a binder has no written type, so the only place
    /// its affinity can come from is inference (spec §4.2).
    pub binds: Vec<(Span, String, T)>,
    /// The record every `.field` read and every record literal resolved to,
    /// keyed by the span of that expression. See `Checked::field_of`.
    pub field_of: HashMap<(usize, usize, usize), String>,
    pending_matches: Vec<(Span, T, Vec<Pat>, String)>,
}

type R<X> = Result<X, Diag>;

impl Checker {
    pub fn new() -> Checker {
        Checker {
            subst: Vec::new(),
            kinds: Vec::new(),
            env: vec![HashMap::new()],
            data: Data::default(),
            sigs: HashMap::new(),
            errors: Vec::new(),
            binds: Vec::new(),
            field_of: HashMap::new(),
            pending_matches: Vec::new(),
        }
    }

    pub fn fresh(&mut self, k: Kind) -> T {
        self.subst.push(None);
        self.kinds.push(k);
        T::Var(self.subst.len() - 1)
    }

    pub fn resolve(&self, t: &T) -> T {
        match t {
            T::Var(i) => match &self.subst[*i] {
                Some(u) => self.resolve(u),
                None => t.clone(),
            },
            T::Con(n, a) => T::Con(n.clone(), a.iter().map(|x| self.resolve(x)).collect()),
            T::Fun(a, b) => T::Fun(Box::new(self.resolve(a)), Box::new(self.resolve(b))),
            T::Tuple(ts) => T::Tuple(ts.iter().map(|x| self.resolve(x)).collect()),
            T::Eff(x) => T::Eff(Box::new(self.resolve(x))),
        }
    }

    fn occurs(&self, v: usize, t: &T) -> bool {
        match self.resolve(t) {
            T::Var(i) => i == v,
            T::Con(_, a) => a.iter().any(|x| self.occurs(v, x)),
            T::Fun(a, b) => self.occurs(v, &a) || self.occurs(v, &b),
            T::Tuple(ts) => ts.iter().any(|x| self.occurs(v, x)),
            T::Eff(x) => self.occurs(v, &x),
        }
    }

    pub fn unify(&mut self, a: &T, b: &T, span: Span, ctx: &str) -> R<()> {
        let (a, b) = (self.resolve(a), self.resolve(b));
        match (&a, &b) {
            (T::Var(i), T::Var(j)) if i == j => Ok(()),
            (T::Var(i), _) => self.bind_var(*i, &b, span, ctx),
            (_, T::Var(j)) => self.bind_var(*j, &a, span, ctx),
            (T::Con(n1, a1), T::Con(n2, a2)) if n1 == n2 && a1.len() == a2.len() => {
                for (x, y) in a1.iter().zip(a2) {
                    self.unify(x, y, span, ctx)?;
                }
                Ok(())
            }
            (T::Fun(p1, r1), T::Fun(p2, r2)) => {
                self.unify(p1, p2, span, ctx)?;
                self.unify(r1, r2, span, ctx)
            }
            (T::Tuple(t1), T::Tuple(t2)) if t1.len() == t2.len() => {
                for (x, y) in t1.iter().zip(t2) {
                    self.unify(x, y, span, ctx)?;
                }
                Ok(())
            }
            (T::Eff(x), T::Eff(y)) => self.unify(x, y, span, ctx),
            (T::Eff(_), _) => Err(Diag::error(
                span,
                "effect.leak",
                &format!(
                    "this is an effectful value (`{}`) used where a pure `{}` is expected",
                    a.show(),
                    b.show()
                ),
            )
            .with_fix("bind it first with `name <- ...`, or mark the enclosing function `E!`")),
            (_, T::Eff(_)) => Err(Diag::error(
                span,
                "effect.missing",
                &format!(
                    "expected an effectful `{}` but found the pure `{}`",
                    b.show(),
                    a.show()
                ),
            )),
            _ => Err(Diag::error(
                span,
                "type.mismatch",
                &format!("expected `{}`, found `{}`{}", b.show(), a.show(), ctx),
            )),
        }
    }

    fn bind_var(&mut self, v: usize, t: &T, span: Span, ctx: &str) -> R<()> {
        if self.occurs(v, t) {
            return Err(Diag::error(
                span,
                "type.infinite",
                "this expression would have an infinite type",
            ));
        }
        match (self.kinds[v], t) {
            (Kind::Num, T::Con(n, _)) if !is_num(n) => {
                return Err(Diag::error(
                    span,
                    "type.mismatch",
                    &format!("`{}` is not a number{}", n, ctx),
                ))
            }
            (Kind::Float, T::Con(n, _)) if !FLOAT_TYPES.contains(&n.as_str()) => {
                return Err(Diag::error(
                    span,
                    "type.mismatch",
                    &format!("a decimal literal cannot have type `{}`{}", n, ctx),
                )
                .with_fix("drop the decimal point, or use `f64 x` to convert"))
            }
            (k, T::Var(j)) => {
                // Keep the stronger constraint.
                let merged = match (k, self.kinds[*j]) {
                    (Kind::Float, _) | (_, Kind::Float) => Kind::Float,
                    (Kind::Num, _) | (_, Kind::Num) => Kind::Num,
                    _ => Kind::Any,
                };
                self.kinds[*j] = merged;
            }
            _ => {}
        }
        self.subst[v] = Some(t.clone());
        Ok(())
    }

    /// Unresolved literal variables become I64 / F64.
    pub fn default_numerics(&mut self) {
        for i in 0..self.subst.len() {
            if self.subst[i].is_none() {
                match self.kinds[i] {
                    Kind::Num => self.subst[i] = Some(T::con("I64")),
                    Kind::Float => self.subst[i] = Some(T::con("F64")),
                    Kind::Any => {}
                }
            }
        }
    }

    // ---- environment ----

    pub fn push_scope(&mut self) {
        self.env.push(HashMap::new());
    }
    pub fn pop_scope(&mut self) {
        self.env.pop();
    }
    pub fn define(&mut self, n: &str, s: Scheme) {
        self.env
            .last_mut()
            .expect("a scope is open")
            .insert(n.to_string(), s);
    }

    /// Record a binder for the ownership pass. `scope` is the span of the
    /// expression the name is visible in, which is the only identity a binder
    /// has: patterns carry no spans of their own.
    pub fn bound(&mut self, scope: Span, n: &str, t: &T) {
        self.binds.push((scope, n.to_string(), t.clone()));
    }
    pub fn lookup(&self, n: &str) -> Option<Scheme> {
        for scope in self.env.iter().rev() {
            if let Some(s) = scope.get(n) {
                return Some(s.clone());
            }
        }
        self.sigs.get(n).cloned()
    }

    pub fn instantiate(&mut self, s: &Scheme) -> T {
        if s.vars.is_empty() {
            return s.ty.clone();
        }
        let map: HashMap<usize, T> = s.vars.iter().map(|v| (*v, self.fresh(Kind::Any))).collect();
        fn go(t: &T, m: &HashMap<usize, T>) -> T {
            match t {
                T::Var(i) => m.get(i).cloned().unwrap_or(T::Var(*i)),
                T::Con(n, a) => T::Con(n.clone(), a.iter().map(|x| go(x, m)).collect()),
                T::Fun(a, b) => T::Fun(Box::new(go(a, m)), Box::new(go(b, m))),
                T::Tuple(ts) => T::Tuple(ts.iter().map(|x| go(x, m)).collect()),
                T::Eff(x) => T::Eff(Box::new(go(x, m))),
            }
        }
        go(&s.ty, &map)
    }

    /// Convert a surface type to an inference type. `&T` is erased, aliases are
    /// normalised, and named type variables map through `vars`.
    /// Lower a written type into an inference type, sharing one variable per
    /// name in `vars`.
    pub fn lower_ty(&mut self, t: &Ty, vars: &mut HashMap<String, T>) -> T {
        match t {
            Ty::Ref(inner) => self.lower_ty(inner, vars),
            Ty::Eff(inner) => T::Eff(Box::new(self.lower_ty(inner, vars))),
            Ty::Fun(a, b) => {
                let a = self.lower_ty(a, vars);
                let b = self.lower_ty(b, vars);
                T::Fun(Box::new(a), Box::new(b))
            }
            Ty::Tuple(ts) => T::Tuple(ts.iter().map(|x| self.lower_ty(x, vars)).collect()),
            Ty::Var(n) => vars
                .entry(n.clone())
                .or_insert_with(|| {
                    self.subst.push(None);
                    self.kinds.push(Kind::Any);
                    T::Var(self.subst.len() - 1)
                })
                .clone(),
            Ty::Con(n, args) => {
                let n = normalise_type_name(n);
                T::Con(n, args.iter().map(|x| self.lower_ty(x, vars)).collect())
            }
        }
    }

    pub fn generalise(&self, t: &T) -> Scheme {
        let t = self.resolve(t);
        let mut vars = Vec::new();
        collect_vars(&t, &mut vars);
        Scheme { vars, ty: t }
    }
}

pub fn normalise_type_name(n: &str) -> String {
    match n {
        "Nat" | "Size" => "U64".to_string(),
        other => other.to_string(),
    }
}

fn collect_vars(t: &T, out: &mut Vec<usize>) {
    match t {
        T::Var(i) => {
            if !out.contains(i) {
                out.push(*i)
            }
        }
        T::Con(_, a) => a.iter().for_each(|x| collect_vars(x, out)),
        T::Fun(a, b) => {
            collect_vars(a, out);
            collect_vars(b, out)
        }
        T::Tuple(ts) => ts.iter().for_each(|x| collect_vars(x, out)),
        T::Eff(x) => collect_vars(x, out),
    }
}

/// Levenshtein distance, for `did you mean` suggestions.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn suggest<'a>(name: &str, candidates: impl Iterator<Item = &'a String>) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for c in candidates {
        let d = edit_distance(name, c);
        if d <= 2.max(name.len() / 3) && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, c.clone()));
        }
    }
    best.map(|(_, n)| n)
}

impl Checker {
    pub fn record_match(&mut self, span: Span, scrut: T, pats: Vec<Pat>, path: String) {
        self.pending_matches.push((span, scrut, pats, path));
    }

    /// Exhaustiveness, run after inference so scrutinee types are known (spec §8.2).
    pub fn check_exhaustiveness(&mut self) {
        let pending = std::mem::take(&mut self.pending_matches);
        for (span, scrut, pats, path) in pending {
            let ty = self.resolve(&scrut);
            let has_catchall = pats.iter().any(|p| matches!(p, Pat::Wild | Pat::Var(_)));
            if has_catchall {
                continue;
            }
            let missing: Vec<String> = match &ty {
                T::Con(n, _) if n == "Bool" => {
                    let mut m = Vec::new();
                    let covered = |b: bool| {
                        pats.iter().any(|p| matches!(p, Pat::Bool(x) if *x == b)
                            || matches!(p, Pat::Ctor(c, _) if (c == "True") == b && (c == "True" || c == "False")))
                    };
                    if !covered(true) {
                        m.push("True".into());
                    }
                    if !covered(false) {
                        m.push("False".into());
                    }
                    m
                }
                T::Con(n, _) if self.data.variants.contains_key(n) => {
                    let all = self.data.variants[n].clone();
                    all.into_iter()
                        .filter(|c| {
                            !pats
                                .iter()
                                .any(|p| matches!(p, Pat::Ctor(pc, _) if pc == c))
                        })
                        .collect()
                }
                T::Con(n, _) if n == "Vec" => {
                    let lens: Vec<usize> = pats
                        .iter()
                        .filter_map(|p| match p {
                            Pat::List(xs) => Some(xs.len()),
                            _ => None,
                        })
                        .collect();
                    let next = (0..).find(|k| !lens.contains(k)).unwrap();
                    vec![format!("a list of length {}", next)]
                }
                T::Tuple(_) => vec!["the remaining combinations".into()],
                _ => vec!["any other value".into()],
            };
            if !missing.is_empty() {
                let list = missing.join(", ");
                self.errors.push(
                    Diag::error(
                        span,
                        "match.nonexhaustive",
                        &format!("this match does not cover {}", list),
                    )
                    .with_path(&format!("{}.match", path))
                    .with_witness(&format!("missing {}", list))
                    .with_fix(&format!(
                        "add `|{} -> ...`",
                        missing
                            .first()
                            .map(|m| if m.contains(' ') {
                                "_".to_string()
                            } else {
                                m.clone()
                            })
                            .unwrap()
                    )),
                );
            }
        }
    }
}

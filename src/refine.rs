//! Phase 6: refinement obligations and SMT discharge (spec §7).
//!
//! Every construct of §7.2 that could fail at run time becomes a proof
//! obligation. Obligations are turned into SMT-LIB 2 text (`smt`, a pure
//! function, so it is testable without a solver) and discharged by piping that
//! text to `z3 -in`. Without `--prove` nothing is discharged and the run is
//! exactly what it was before this module existed.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::infer::Checked;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sort {
    Int,
    Real,
    Bool,
}

impl Sort {
    fn smt(self) -> &'static str {
        match self {
            Sort::Int => "Int",
            Sort::Real => "Real",
            Sort::Bool => "Bool",
        }
    }
}

/// One proof obligation: prove `goal` under `hyps`, with `vars` free.
#[derive(Clone, Debug)]
pub struct Ob {
    pub span: Span,
    pub path: String,
    pub code: String,
    pub msg: String,
    pub fix: Option<String>,
    pub vars: Vec<(String, Sort)>,
    pub hyps: Vec<String>,
    pub goal: String,
}

/// SMT-LIB 2 text for one obligation: assert the hypotheses and the negated
/// goal, then ask. `unsat` means proved; `sat` yields the counterexample.
pub fn smt(o: &Ob) -> String {
    let mut s = String::from("(set-logic ALL)\n");
    for (n, k) in &o.vars {
        s.push_str(&format!("(declare-const {} {})\n", n, k.smt()));
    }
    for h in &o.hyps {
        s.push_str(&format!("(assert {})\n", h));
    }
    s.push_str(&format!(
        "(assert (not {}))\n(check-sat)\n(get-model)\n",
        o.goal
    ));
    s
}

/// What to do with the obligations of a run.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// discharge them with z3; an open obligation is an error
    Prove,
    /// leave them, and say once on stderr how many are open (`vibe check`)
    Report,
    /// leave them silently (`vibe build`/`run`, whose output belongs to the program)
    Silent,
}

pub fn check(m: &Module, ck: &Checked, mode: Mode, cache: &mut Cache) -> Vec<Diag> {
    let obs = obligations(m, ck);
    if obs.is_empty() || mode == Mode::Silent {
        return Vec::new();
    }
    if mode == Mode::Report {
        eprintln!(
            "note: {} refinement obligation(s) not discharged; run with --prove",
            obs.len()
        );
        return Vec::new();
    }
    // A fully cached run needs no solver at all, so the missing-solver error
    // must come after the cache has had its say (§16.5).
    if obs.iter().all(|o| cache.hit(o)) {
        return Vec::new();
    }
    if !have_z3() {
        return vec![Diag::error(
            m.span,
            "refine.no-solver",
            "--prove needs the `z3` binary on PATH",
        )
        .with_path(&m.name)
        .with_fix("install z3, or drop --prove to leave obligations undischarged")];
    }
    let out: Vec<Diag> = obs.iter().filter_map(|o| discharge(o, cache)).collect();
    cache.flush();
    out
}

fn have_z3() -> bool {
    Command::new("z3")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Whether z3 closes this obligation on its own. A missing solver is reported
/// as "not proved" rather than as an error, so `vibe proof --prove` degrades to
/// listing everything instead of claiming a proof it did not get.
pub fn proved(o: &Ob) -> bool {
    discharge(o, &mut Cache::off()).is_none()
}

/// Proof certificates, keyed by the hash of the SMT text (spec §16.5). The text
/// is the whole question — hypotheses, goal, sorts — so a hit means the same
/// question was already answered `unsat`, and nothing else can collide with it.
///
/// Only proofs are cached. A failure may be a timeout, or a program that has
/// since changed around it, and re-asking costs one solver call.
pub struct Cache {
    path: Option<PathBuf>,
    proved: HashSet<String>,
    added: bool,
}

impl Cache {
    pub fn off() -> Cache {
        Cache {
            path: None,
            proved: HashSet::new(),
            added: false,
        }
    }

    /// The cache for a source file: a sibling `.vibe-proofs`, one hash a line.
    pub fn beside(src: &Path) -> Cache {
        let path = src.with_file_name(".vibe-proofs");
        let proved = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        Cache {
            path: Some(path),
            proved,
            added: false,
        }
    }

    /// Whether this obligation was already proved, by this cache or an earlier
    /// run.
    pub fn hit(&self, o: &Ob) -> bool {
        self.proved.contains(&crate::patch::hash(&smt(o)))
    }

    /// ponytail: rewrites the whole file rather than appending, and never
    /// evicts. A stale hash costs one line; the day that matters, sort by
    /// mtime and truncate.
    pub fn flush(&self) {
        if !self.added {
            return;
        }
        if let Some(p) = &self.path {
            let mut lines: Vec<&str> = self.proved.iter().map(|s| s.as_str()).collect();
            lines.sort_unstable();
            let _ = std::fs::write(p, format!("{}\n", lines.join("\n")));
        }
    }
}

fn discharge(o: &Ob, cache: &mut Cache) -> Option<Diag> {
    let text = smt(o);
    let key = crate::patch::hash(&text);
    if cache.proved.contains(&key) {
        return None;
    }
    let out = match z3(&text) {
        Ok(s) => s,
        Err(e) => {
            return Some(
                Diag::error(o.span, "refine.no-solver", &format!("z3 failed: {e}"))
                    .with_path(&o.path),
            )
        }
    };
    if out.starts_with("unsat") {
        cache.proved.insert(key);
        cache.added = true;
        return None;
    }
    // `unknown` is not a counterexample: the solver ran out of budget, and
    // saying so is more useful than a refutation nobody can read (§16.5).
    if out.starts_with("unknown") || out.starts_with("timeout") {
        return Some(
            Diag::error(
                o.span,
                "refine.budget",
                &format!("the solver gave up on: {}", o.msg),
            )
            .with_path(&o.path)
            .with_fix(
                "raise --prove-timeout, or add a ghost function that makes the step explicit",
            ),
        );
    }
    let mut d = Diag::error(o.span, &o.code, &o.msg).with_path(&o.path);
    if let Some(w) = model(&out) {
        d = d.with_witness(&w);
    }
    if let Some(f) = &o.fix {
        d = d.with_fix(f);
    }
    Some(d)
}

/// Seconds the solver gets per obligation. A budget is what keeps one hard
/// obligation from becoming the compile time (§16.5).
static BUDGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(5);

pub fn set_budget(secs: u64) {
    BUDGET.store(secs.max(1), std::sync::atomic::Ordering::Relaxed);
}

fn z3(text: &str) -> Result<String, String> {
    let t = BUDGET.load(std::sync::atomic::Ordering::Relaxed);
    let mut ch = Command::new("z3")
        .arg(format!("-T:{t}"))
        .arg("-in")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    ch.stdin
        .as_mut()
        .ok_or("no stdin")?
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = ch.wait_with_output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `(define-fun x () Int 3)` -> `x=3`, joined. ponytail: a scanner, not an
/// s-expression parser; enough for the shape z3 prints for a flat model.
fn model(out: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for chunk in out.split("(define-fun ").skip(1) {
        let name = chunk.split_whitespace().next()?.to_string();
        let rest = chunk.split_once(')')?.1; // past the empty parameter list
        let rest = rest.trim_start();
        let rest = rest.strip_prefix(rest.split_whitespace().next()?)?; // past the sort
        parts.push(format!("{}={}", name, value(rest.trim_start())?));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// The first balanced term of `s`, with `(- 3)` flattened to `-3`.
fn value(s: &str) -> Option<String> {
    if !s.starts_with('(') {
        let a = s.split(|c: char| c.is_whitespace() || c == ')').next()?;
        return if a.is_empty() {
            None
        } else {
            Some(a.to_string())
        };
    }
    let mut depth = 0usize;
    let mut end = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = s[1..end.checked_sub(1)?].trim();
    let toks: Vec<&str> = inner.split_whitespace().collect();
    Some(match toks.as_slice() {
        ["-", n] => format!("-{n}"),
        ["/", a, b] => format!("{a}/{b}"),
        _ => s[..end].split_whitespace().collect::<Vec<_>>().join(" "),
    })
}

// ------------------------------------------------------------- generation

struct Gen<'a> {
    m: &'a Module,
    ck: &'a Checked,
    /// declared types of names in scope, when a signature gives one
    tys: HashMap<String, String>,
    syms: BTreeMap<String, Sort>,
    hyps: Vec<String>,
    obs: Vec<Ob>,
    fun: String,
    /// names the enclosing signature binds — a goal over only these is fixable
    params: Vec<String>,
    /// Source name -> the symbol standing for it. A binder that reuses a name
    /// already in scope gets a symbol of its own, because inheriting the
    /// symbol would inherit the hypotheses: with `(n:U64, n>0)` in the
    /// signature, an `|Ok n ->` arm would discharge `100 / n` for an `n` the
    /// pattern rebound to anything at all. That is not imprecision, it is a
    /// false proof.
    renames: HashMap<String, String>,
    /// every name bound so far in this function, so shadowing is detectable
    seen: HashSet<String>,
    shadows: usize,
}

pub fn obligations(m: &Module, ck: &Checked) -> Vec<Ob> {
    let mut obs = Vec::new();
    for f in m.funs() {
        if f.ghost {
            continue; // ghost functions exist only for the proofs (§7.4)
        }
        let mut g = Gen {
            m,
            ck,
            tys: HashMap::new(),
            syms: BTreeMap::new(),
            hyps: Vec::new(),
            obs: Vec::new(),
            fun: format!("{}.{}", f.home, f.name),
            params: Vec::new(),
            renames: HashMap::new(),
            seen: HashSet::new(),
            shadows: 0,
        };
        for p in &f.params {
            for n in &p.names {
                if let Some(t) = p.ty.as_ref().and_then(base_name) {
                    g.tys.insert(n.clone(), t);
                }
                g.params.push(n.clone());
                g.seen.insert(n.clone());
            }
        }
        // the signature's own refinements are hypotheses inside the body
        for p in &f.params {
            for r in &p.refines {
                if let Some((t, Sort::Bool)) = g.term(r) {
                    g.hyps.push(t);
                }
            }
        }
        for n in g.params.clone() {
            g.range_hyp(&n);
            g.inv_hyps(&n);
        }
        g.expr(&f.body);
        obs.extend(g.obs);
    }
    obs
}

fn base_name(t: &Ty) -> Option<String> {
    match t {
        Ty::Con(n, _) => Some(n.clone()),
        Ty::Ref(i) | Ty::Eff(i) => base_name(i),
        _ => None,
    }
}

fn sort_of(ty: &str) -> Sort {
    match ty {
        "F32" | "F64" => Sort::Real,
        "Bool" => Sort::Bool,
        _ => Sort::Int,
    }
}

/// `(min, max)` for the machine integer types. `Nat` is mathematical, so it has
/// a floor but no ceiling and never overflows.
fn range(ty: &str) -> Option<(i128, Option<i128>)> {
    let u = |b: u32| Some((0i128, Some((1i128 << b) - 1)));
    let s = |b: u32| Some((-(1i128 << (b - 1)), Some((1i128 << (b - 1)) - 1)));
    match ty {
        "Nat" => Some((0, None)),
        "U8" => u(8),
        "U16" => u(16),
        "U32" => u(32),
        "U64" | "Size" => u(64),
        "I8" => s(8),
        "I16" => s(16),
        "I32" => s(32),
        "I64" => s(64),
        _ => None,
    }
}

fn is_container(ty: &str) -> bool {
    matches!(ty, "Vec" | "Str" | "List" | "Slice")
}

/// The sort of a type whose values the solver can talk about directly. `None`
/// means the value is opaque, so only its length and its invariant travel.
fn scalar_sort(ty: &str) -> Option<Sort> {
    match ty {
        "F32" | "F64" => Some(Sort::Real),
        "Bool" => Some(Sort::Bool),
        _ if range(ty).is_some() => Some(Sort::Int),
        _ => None,
    }
}

fn conj(mut parts: Vec<String>) -> String {
    if parts.len() == 1 {
        parts.pop().expect("checked non-empty")
    } else {
        format!("(and {})", parts.join(" "))
    }
}

fn field_tys(info: &crate::types::RecordInfo) -> HashMap<String, String> {
    info.fields
        .iter()
        .filter_map(|(n, t)| base_name(t).map(|b| (n.clone(), b)))
        .collect()
}

fn is_unsigned(ty: &str) -> bool {
    matches!(ty, "Nat" | "U8" | "U16" | "U32" | "U64" | "Size")
}

/// Conversions are the identity on the value, only the sort changes.
fn conv(name: &str) -> Option<&'static str> {
    match name {
        "f32" | "f64" => Some("F64"),
        "i8" | "i16" | "i32" | "i64" => Some("I64"),
        "u8" | "u16" | "u32" | "u64" | "size" => Some("U64"),
        _ => None,
    }
}

fn is_checked(name: &str) -> bool {
    name.ends_with("_checked")
}

impl<'a> Gen<'a> {
    /// The symbol standing for a source name here.
    fn var(&self, n: &str) -> String {
        self.renames
            .get(n)
            .cloned()
            .unwrap_or_else(|| n.to_string())
    }

    /// Introduce `n`, giving it a fresh symbol if the name is already taken.
    fn fresh_name(&mut self, n: &str) -> String {
        if self.seen.insert(n.to_string()) {
            self.renames.remove(n);
            return n.to_string();
        }
        self.shadows += 1;
        // `.` is a legal SMT-LIB simple-symbol character and `#` is not, and a
        // Vibelang identifier is `[a-z][a-zA-Z0-9_]*`, so this cannot collide
        // with a name the program wrote.
        let sym = format!("{n}.{}", self.shadows);
        self.renames.insert(n.to_string(), sym.clone());
        sym
    }

    fn sym(&mut self, name: String, k: Sort) -> (String, Sort) {
        let k = *self.syms.entry(name.clone()).or_insert(k);
        (name, k)
    }

    /// Unsigned and machine-integer names carry their range into the context.
    fn range_hyp(&mut self, n: &str) {
        let ty = match self.tys.get(n) {
            Some(t) => t.clone(),
            None => return,
        };
        if let Some((lo, hi)) = range(&ty) {
            let (s, _) = self.sym(n.to_string(), Sort::Int);
            self.hyps.push(format!("(>= {s} {lo})"));
            if let Some(hi) = hi {
                self.hyps.push(format!("(<= {s} {hi})"));
            }
        }
    }

    /// A value of a record type satisfies that record's invariant, so the
    /// invariant of every parameter enters the context as `param_field` facts.
    fn inv_hyps(&mut self, var: &str) {
        let Some(rec) = self.tys.get(var).cloned() else {
            return;
        };
        let Some(info) = self.ck.data.records.get(&rec).cloned() else {
            return;
        };
        if info.refines.is_empty() {
            return;
        }
        let keep: Vec<String> = self.syms.keys().cloned().collect();
        let saved = std::mem::replace(&mut self.tys, field_tys(&info));
        let mut out: Vec<String> = Vec::new();
        for r in &info.refines {
            if let Some((t, Sort::Bool)) = self.term(r) {
                out.push(t);
            }
        }
        self.tys = saved;
        for (n, ty) in &info.fields {
            let k = base_name(ty).as_deref().map(sort_of).unwrap_or(Sort::Int);
            let (sym, _) = self.sym(format!("{var}_{n}"), k);
            for t in out.iter_mut() {
                *t = replace_sym(t, n, &sym);
            }
        }
        let pre = format!("{var}_");
        self.syms
            .retain(|k, _| keep.contains(k) || k.starts_with(&pre));
        self.hyps.extend(out);
    }

    /// Translate a Vibelang expression into an SMT term. `None` means "outside
    /// the fragment"; an obligation that mentions such a term is dropped.
    /// ponytail: the ceiling is linear integer/real arithmetic over named
    /// values, `len xs` and `x.f` as opaque symbols. Upgrade path: give the
    /// solver the real datatypes (sequences for Vec, algebraic datatypes for
    /// ADTs) instead of flattening them to symbols.
    fn term(&mut self, e: &Expr) -> Option<(String, Sort)> {
        match &e.kind {
            ExprKind::Int(n) => Some((int_lit(*n as i128), Sort::Int)),
            ExprKind::Float(f) => Some((real_lit(*f), Sort::Real)),
            ExprKind::Bool(b) => Some((b.to_string(), Sort::Bool)),
            ExprKind::Var(n) => {
                let v = self.var(n);
                let k = self.tys.get(&v).map(|t| sort_of(t)).unwrap_or(Sort::Int);
                Some(self.sym(v, k))
            }
            ExprKind::Borrow(i) => self.term(i),
            ExprKind::Field(b, f) => {
                let base = as_name(b)?;
                let ty = self
                    .ck
                    .data
                    .field_owner
                    .get(f)
                    .and_then(|r| self.ck.data.records.get(r))
                    .and_then(|r| r.fields.iter().find(|(n, _)| n == f))
                    .and_then(|(_, t)| base_name(t));
                let k = ty.as_deref().map(sort_of).unwrap_or(Sort::Int);
                Some(self.sym(format!("{base}_{f}"), k))
            }
            ExprKind::Neg(i) => {
                let (t, k) = self.term(i)?;
                Some((format!("(- {t})"), k))
            }
            ExprKind::Not(i) => {
                let (t, _) = self.term(i)?;
                Some((format!("(not {t})"), Sort::Bool))
            }
            ExprKind::Binop(op, a, b) => {
                let (ta, ka) = self.term(a)?;
                let (tb, kb) = self.term(b)?;
                let k = if ka == Sort::Real || kb == Sort::Real {
                    Sort::Real
                } else {
                    ka
                };
                let (ta, tb) = (cast(ta, ka, k), cast(tb, kb, k));
                match op.as_str() {
                    "==" => Some((format!("(= {ta} {tb})"), Sort::Bool)),
                    "!=" => Some((format!("(not (= {ta} {tb}))"), Sort::Bool)),
                    "<" | "<=" | ">" | ">=" => Some((format!("({op} {ta} {tb})"), Sort::Bool)),
                    "&&" => Some((format!("(and {ta} {tb})"), Sort::Bool)),
                    "||" => Some((format!("(or {ta} {tb})"), Sort::Bool)),
                    "+" | "-" | "*" => Some((format!("({op} {ta} {tb})"), k)),
                    "/" if k == Sort::Real => Some((format!("(/ {ta} {tb})"), k)),
                    "/" => Some((format!("(div {ta} {tb})"), k)),
                    "%" => Some((format!("(mod {ta} {tb})"), k)),
                    _ => None,
                }
            }
            ExprKind::App(_, _) => {
                let (f, args) = flatten(e);
                let name = as_name(f)?;
                if name == "len" && args.len() == 1 {
                    let v = self.var(&as_name(strip(args[0]))?);
                    return Some(self.sym(format!("len_{v}"), Sort::Int));
                }
                if let (Some(to), 1) = (conv(&name), args.len()) {
                    let (t, k) = self.term(args[0])?;
                    let want = sort_of(to);
                    return Some((cast(t, k, want), want));
                }
                None
            }
            _ => None,
        }
    }

    /// The static type name of an expression, when the signature makes it
    /// obvious. Drives the overflow rules; `None` means "do not guess".
    fn ty_of(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Var(n) => self.tys.get(&self.var(n)).cloned(),
            ExprKind::Borrow(i) | ExprKind::Neg(i) => self.ty_of(i),
            ExprKind::Field(_, f) => self
                .ck
                .data
                .field_owner
                .get(f)
                .and_then(|r| self.ck.data.records.get(r))
                .and_then(|r| r.fields.iter().find(|(n, _)| n == f))
                .and_then(|(_, t)| base_name(t)),
            ExprKind::Binop(op, a, b) if matches!(op.as_str(), "+" | "-" | "*" | "/" | "%") => {
                self.ty_of(a).or_else(|| self.ty_of(b))
            }
            ExprKind::App(_, _) => {
                let (f, args) = flatten(e);
                let name = as_name(f)?;
                if name == "len" {
                    return Some("Size".into());
                }
                if args.len() == 1 {
                    if let Some(t) = conv(&name) {
                        return Some(t.into());
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn push(&mut self, span: Span, code: &str, msg: &str, pretty: String, goal: String) {
        let path = format!("{}.body/{}", self.fun, self.obs.len());
        // A goal that only mentions the parameters can be repaired by adding it
        // to the signature (spec §12.2).
        let fixable = self
            .syms
            .keys()
            .filter(|s| goal.contains(s.as_str()))
            .all(|s| {
                self.params
                    .iter()
                    .any(|p| p == s || format!("len_{p}") == *s)
            });
        let fix = if fixable {
            Some(format!("{}.sig += {}", self.fun, pretty))
        } else {
            None
        };
        let vars = self.syms.iter().map(|(n, k)| (n.clone(), *k)).collect();
        self.obs.push(Ob {
            span,
            path,
            code: code.into(),
            msg: msg.into(),
            fix,
            vars,
            hyps: self.hyps.clone(),
            goal,
        });
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Binop(op, a, b) => {
                self.expr(a);
                self.expr(b);
                self.arith(e, op, a, b);
            }
            ExprKind::App(_, _) => {
                let (f, args) = flatten(e);
                for a in &args {
                    self.expr(a);
                }
                if let Some(name) = as_name(f) {
                    self.call(e, &name, &args);
                }
            }
            ExprKind::Record(base, fields) => {
                for (_, v) in fields {
                    self.expr(v);
                }
                if let Some(b) = base {
                    self.expr(b);
                }
                self.record(e, base.as_deref(), fields);
            }
            ExprKind::Neg(i) | ExprKind::Not(i) | ExprKind::Borrow(i) | ExprKind::Field(i, _) => {
                self.expr(i)
            }
            ExprKind::Tuple(xs) | ExprKind::List(xs) => {
                for x in xs {
                    self.expr(x)
                }
            }
            ExprKind::Lambda(_, b) => self.expr(b),
            ExprKind::Bind(n, v, rest) => {
                self.expr(v);
                let saved = self.renames.clone();
                self.fresh_name(n);
                self.expr(rest);
                self.renames = saved;
            }
            ExprKind::Let(n, v, rest) => {
                self.expr(v);
                let k = self.hyps.len();
                // the value is translated before the name is introduced, so a
                // `let x = x + 1` still reads the outer `x`
                let val = self.term(v);
                let ty = self.ty_of(v);
                let saved = self.renames.clone();
                let sym = self.fresh_name(n);
                if let Some((t, s)) = val {
                    let (sym, _) = self.sym(sym.clone(), s);
                    self.hyps.push(format!("(= {sym} {t})"));
                }
                if let Some(t) = ty {
                    self.tys.insert(sym, t);
                }
                self.expr(rest);
                self.hyps.truncate(k);
                self.renames = saved;
            }
            ExprKind::Match(scrut, arms) => {
                self.expr(scrut);
                let s = self.term(scrut);
                let mut seen: Vec<String> = Vec::new();
                for (p, body) in arms {
                    let k = self.hyps.len();
                    let tys = self.tys.clone();
                    for n in &seen {
                        self.hyps.push(n.clone());
                    }
                    if let Some(h) = self.arm_hyp(&s, scrut, p) {
                        self.hyps.push(h.clone());
                        seen.push(format!("(not {h})"));
                    }
                    self.expr(body);
                    self.hyps.truncate(k);
                    self.tys = tys;
                }
            }
            _ => {}
        }
    }

    /// What a branch teaches the solver (spec §8.3). Literal, boolean and
    /// list-length patterns speak about the scrutinee's own term; a constructor
    /// pattern goes through `pat_facts`, which names the payload positionally,
    /// so a refinement of the payload survives the binding.
    fn arm_hyp(&mut self, s: &Option<(String, Sort)>, scrut: &Expr, p: &Pat) -> Option<String> {
        if let Pat::Ctor(..) = p {
            let v = self.var(&as_name(strip(scrut))?);
            let ty = self.ty_of(scrut);
            return self.pat_facts(&v, ty, p);
        }
        let (t, k) = s.clone()?;
        match p {
            Pat::Int(n) if k == Sort::Int => Some(format!("(= {t} {})", int_lit(*n as i128))),
            Pat::Bool(b) if k == Sort::Bool => Some(if *b { t } else { format!("(not {t})") }),
            Pat::List(ps) => {
                let v = self.var(&as_name(strip(scrut))?);
                let (l, _) = self.sym(format!("len_{v}"), Sort::Int);
                Some(format!("(= {l} {})", ps.len()))
            }
            _ => None,
        }
    }

    /// Facts about the value at SMT path `v` (declared type `ty`) when pattern
    /// `p` matches it. The result is the arm's *guard*: it is negated into the
    /// arms below, so a later arm learns that an earlier one did not fire.
    /// Bindings the pattern introduces are pushed straight onto `hyps` instead,
    /// because they hold only inside this arm and must never be negated.
    ///
    /// ponytail: a constructor is a `tag_v` integer plus positional payload
    /// paths, not a real SMT datatype. That carries tag exclusivity and payload
    /// refinements, which is what §8.3 asks for. Upgrade path: emit
    /// `declare-datatypes` if a proof ever needs to reason about a constructor
    /// no arm names.
    fn pat_facts(&mut self, v: &str, ty: Option<String>, p: &Pat) -> Option<String> {
        match p {
            Pat::Var(n) => {
                self.bind(n, v, ty);
                None
            }
            Pat::Int(i) => {
                let (s, _) = self.sym(v.to_string(), Sort::Int);
                Some(format!("(= {s} {})", int_lit(*i as i128)))
            }
            Pat::Bool(b) => {
                let (s, _) = self.sym(v.to_string(), Sort::Bool);
                Some(if *b { s } else { format!("(not {s})") })
            }
            Pat::List(ps) => {
                let l = self.len_sym(v);
                Some(format!("(= {l} {})", ps.len()))
            }
            Pat::Ctor(c, args) => {
                let info = self.ck.data.ctors.get(c)?.clone();
                let (tag, _) = self.sym(format!("tag_{v}"), Sort::Int);
                let mut parts = vec![format!("(= {tag} {})", info.tag)];
                for (i, sub) in args.iter().enumerate() {
                    let path = format!("{v}_{i}");
                    let aty = info.args.get(i).and_then(base_name);
                    if let Some(g) = self.pat_facts(&path, aty, sub) {
                        parts.push(g);
                    }
                }
                Some(conj(parts))
            }
            _ => None,
        }
    }

    /// `n` names the value at path `v`. Alias the scalar term when the type has
    /// one, alias the length for a container (`len` is an opaque symbol keyed on
    /// the name, so the alias is the only thing that carries `len ts > 0` across
    /// the binding), and give `n` the context any parameter of that type gets.
    fn bind(&mut self, n: &str, v: &str, ty: Option<String>) {
        let sym = self.fresh_name(n);
        if sym == v {
            return;
        }
        if let Some(t) = &ty {
            self.tys.insert(sym.clone(), t.clone());
            if let Some(k) = scalar_sort(t) {
                let (a, _) = self.sym(sym.clone(), k);
                let (b, _) = self.sym(v.to_string(), k);
                self.hyps.push(format!("(= {a} {b})"));
            }
        }
        if ty.as_deref().is_none_or(is_container) {
            let ln = self.len_sym(&sym);
            let lv = self.len_sym(v);
            self.hyps.push(format!("(= {ln} {lv})"));
            self.hyps.push(format!("(>= {lv} 0)"));
        }
        self.range_hyp(&sym);
        self.inv_hyps(&sym);
    }

    fn len_sym(&mut self, v: &str) -> String {
        self.sym(format!("len_{v}"), Sort::Int).0
    }

    fn arith(&mut self, e: &Expr, op: &str, a: &Expr, b: &Expr) {
        match op {
            "/" | "%" => {
                let Some((tb, kb)) = self.term(b) else { return };
                let zero = if kb == Sort::Real { "0.0" } else { "0" };
                self.push(
                    e.span,
                    "div0",
                    "cannot prove the divisor is non-zero",
                    format!("{} != {}", show(b), zero),
                    format!("(not (= {tb} {zero}))"),
                );
            }
            "+" | "*" => {
                let ty = self.ty_of(a).or_else(|| self.ty_of(b));
                let Some(ty) = ty else { return };
                let Some((lo, Some(hi))) = range(&ty) else {
                    return;
                };
                let (Some((ta, _)), Some((tb, _))) = (self.term(a), self.term(b)) else {
                    return;
                };
                let t = format!("({op} {ta} {tb})");
                self.push(
                    e.span,
                    "overflow",
                    &format!(
                        "cannot prove `{} {} {}` stays in {}",
                        show(a),
                        op,
                        show(b),
                        ty
                    ),
                    format!("{} {} {} <= {}", show(a), op, show(b), hi),
                    format!("(and (<= {t} {hi}) (>= {t} {lo}))"),
                );
            }
            "-" => {
                let ty = self.ty_of(a).or_else(|| self.ty_of(b));
                let Some(ty) = ty else { return };
                if !is_unsigned(&ty) {
                    return;
                }
                let (Some((ta, _)), Some((tb, _))) = (self.term(a), self.term(b)) else {
                    return;
                };
                self.push(
                    e.span,
                    "underflow",
                    &format!("cannot prove `{} >= {}` on {}", show(a), show(b), ty),
                    format!("{} >= {}", show(a), show(b)),
                    format!("(>= {ta} {tb})"),
                );
            }
            _ => {}
        }
    }

    fn call(&mut self, e: &Expr, name: &str, args: &[&Expr]) {
        if is_checked(name) {
            return; // §7.5: the checked forms discharge the obligation at run time
        }
        if name == "get" && args.len() == 2 {
            if let (Some(v), Some((ti, _))) = (as_name(strip(args[0])), self.term(args[1])) {
                let v = self.var(&v);
                let (l, _) = self.sym(format!("len_{v}"), Sort::Int);
                self.hyps.push(format!("(>= {l} 0)"));
                self.push(
                    e.span,
                    "bounds",
                    &format!("cannot prove the index is inside `{v}`"),
                    format!("{} < len {v}", show(args[1])),
                    format!("(and (< {ti} {l}) (>= {ti} 0))"),
                );
                self.hyps.pop();
            }
            return;
        }
        // precondition of a declared function or of an `ext c` signature (§7.2)
        let (params, refines): (Vec<String>, Vec<Expr>) = match self.sig(name) {
            Some(x) => x,
            None => return,
        };
        let mut sub: HashMap<String, (String, Sort)> = HashMap::new();
        let mut len_sub: HashMap<String, String> = HashMap::new();
        for (i, p) in params.iter().enumerate() {
            let Some(a) = args.get(i) else { return };
            if let Some(t) = self.term(a) {
                sub.insert(p.clone(), t);
            }
            if let Some(v) = as_name(strip(a)) {
                let v = self.var(&v);
                let (l, _) = self.sym(format!("len_{v}"), Sort::Int);
                len_sub.insert(format!("len_{p}"), l);
            }
        }
        for r in &refines {
            // translate the refinement in the callee's names, then rename
            let saved = std::mem::take(&mut self.tys);
            let got = self.term(r);
            self.tys = saved;
            let Some((mut t, Sort::Bool)) = got else {
                continue;
            };
            for (from, to) in &len_sub {
                t = replace_sym(&t, from, to);
            }
            for (from, (to, _)) in &sub {
                if !len_sub.contains_key(&format!("len_{from}"))
                    || !t.contains(&format!("len_{from}"))
                {
                    t = replace_sym(&t, from, to);
                }
            }
            let pretty = show(r);
            self.push(
                e.span,
                "refine.unproven",
                &format!("cannot prove the precondition of `{name}`: {pretty}"),
                pretty,
                t,
            );
        }
    }

    /// Parameter names and refinements of a callee, module function or `ext c`.
    fn sig(&self, name: &str) -> Option<(Vec<String>, Vec<Expr>)> {
        if let Some(f) = self.m.funs().find(|f| f.name == name) {
            let mut ps = Vec::new();
            let mut rs = Vec::new();
            for p in &f.params {
                ps.extend(p.names.iter().cloned());
                rs.extend(p.refines.iter().cloned());
            }
            if rs.is_empty() {
                return None;
            }
            return Some((ps, rs));
        }
        let x = self.ck.ext.get(name)?;
        if x.refines.is_empty() {
            return None;
        }
        // ponytail: `ext` signatures are types, not named parameters, so their
        // refinements can only mention names the call site also uses.
        Some((Vec::new(), x.refines.clone()))
    }

    fn record(&mut self, e: &Expr, base: Option<&Expr>, fields: &[(String, Expr)]) {
        let Some((f0, _)) = fields.first() else {
            return;
        };
        let Some(rec) = self.ck.data.field_owner.get(f0).cloned() else {
            return;
        };
        let Some(info) = self.ck.data.records.get(&rec).cloned() else {
            return;
        };
        if info.refines.is_empty() {
            return;
        }
        let mut sub: HashMap<String, (String, Sort)> = HashMap::new();
        for (n, v) in fields {
            if let Some(t) = self.term(v) {
                sub.insert(n.clone(), t);
            }
        }
        // an update keeps the untouched fields of the base record
        if let Some(b) = base {
            if let Some(bn) = as_name(strip(b)) {
                for (n, ty) in &info.fields {
                    if !sub.contains_key(n) {
                        let k = base_name(ty).as_deref().map(sort_of).unwrap_or(Sort::Int);
                        sub.insert(n.clone(), self.sym(format!("{bn}_{n}"), k));
                    }
                }
            }
        }
        for r in &info.refines {
            let saved = std::mem::replace(&mut self.tys, field_tys(&info));
            let got = self.term(r);
            self.tys = saved;
            let Some((mut t, Sort::Bool)) = got else {
                continue;
            };
            for (from, (to, _)) in &sub {
                t = replace_sym(&t, from, to);
            }
            let pretty = show(r);
            self.push(
                e.span,
                "refine.unproven",
                &format!("cannot prove the invariant of `{rec}`: {pretty}"),
                pretty,
                t,
            );
        }
    }
}

// ------------------------------------------------------------------ helpers

fn int_lit(n: i128) -> String {
    if n < 0 {
        format!("(- {})", -n)
    } else {
        n.to_string()
    }
}

fn real_lit(f: f64) -> String {
    let s = if f.fract() == 0.0 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    };
    if f < 0.0 {
        format!("(- {})", &s[1..])
    } else {
        s
    }
}

fn cast(t: String, from: Sort, to: Sort) -> String {
    match (from, to) {
        (Sort::Int, Sort::Real) => format!("(to_real {t})"),
        (Sort::Real, Sort::Int) => format!("(to_int {t})"),
        _ => t,
    }
}

/// Replace whole-token occurrences of `from` in an SMT term.
fn replace_sym(t: &str, from: &str, to: &str) -> String {
    let ok = |c: Option<char>| !matches!(c, Some(c) if c.is_alphanumeric() || c == '_');
    let mut out = String::new();
    let b: Vec<char> = t.chars().collect();
    let f: Vec<char> = from.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(&f[..])
            && ok(if i == 0 { None } else { Some(b[i - 1]) })
            && ok(b.get(i + f.len()).copied())
        {
            out.push_str(to);
            i += f.len();
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

fn strip(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Borrow(i) => strip(i),
        _ => e,
    }
}

fn as_name(e: &Expr) -> Option<String> {
    match &strip(e).kind {
        ExprKind::Var(n) | ExprKind::Ctor(n) => Some(n.clone()),
        _ => None,
    }
}

fn flatten(e: &Expr) -> (&Expr, Vec<&Expr>) {
    match &e.kind {
        ExprKind::App(f, args) => {
            let (h, mut acc) = flatten(f);
            acc.extend(args.iter());
            (h, acc)
        }
        _ => (e, Vec::new()),
    }
}

/// Expressions back in Vibelang syntax, for the `fix` field.
fn show(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(n) => n.to_string(),
        ExprKind::Float(f) => format!("{f}"),
        ExprKind::Str(s) => format!("{s:?}"),
        ExprKind::Char(c) => format!("'{c}'"),
        ExprKind::Bool(b) => if *b { "True" } else { "False" }.into(),
        ExprKind::Unit => "()".into(),
        ExprKind::Var(n) | ExprKind::Ctor(n) => n.clone(),
        ExprKind::Borrow(i) => format!("&{}", show(i)),
        ExprKind::Neg(i) => format!("-{}", show(i)),
        ExprKind::Not(i) => format!("!{}", show(i)),
        ExprKind::Field(b, f) => format!("{}.{}", show(b), f),
        ExprKind::Binop(op, a, b) => format!("{} {} {}", show(a), op, show(b)),
        ExprKind::App(_, _) => {
            let (f, args) = flatten(e);
            let mut s = show(f);
            for a in args {
                let t = show(a);
                s.push(' ');
                if t.contains(' ') {
                    s.push_str(&format!("({t})"));
                } else {
                    s.push_str(&t);
                }
            }
            s
        }
        _ => "_".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{infer, lexer, parser};

    fn obs_of(src: &str) -> Vec<Ob> {
        let toks = lexer::lex(src, 0).expect("lexes");
        let m = parser::parse(toks).expect("parses");
        let ck = infer::check(&m).expect("checks");
        obligations(&m, &ck)
    }

    #[test]
    fn division_emits_a_div0_obligation_in_smt() {
        let o = obs_of("mod T\n\nf (a:U64) (b:U64) : U64 = a / b\n");
        assert_eq!(o.len(), 1, "{o:#?}");
        assert_eq!(o[0].code, "div0");
        assert_eq!(o[0].goal, "(not (= b 0))");
        let text = smt(&o[0]);
        assert!(text.starts_with("(set-logic ALL)\n"), "{text}");
        assert!(text.contains("(declare-const b Int)"), "{text}");
        assert!(text.contains("(assert (>= b 0))"), "{text}");
        assert!(text.contains("(assert (not (not (= b 0))))"), "{text}");
        assert!(text.ends_with("(check-sat)\n(get-model)\n"), "{text}");
        assert_eq!(o[0].fix.as_deref(), Some("T.f.sig += b != 0"));
    }

    #[test]
    fn a_refinement_on_the_signature_becomes_a_hypothesis() {
        let o = obs_of("mod T\n\nf (a:U64) (b:U64, b != 0) : U64 = a / b\n");
        assert_eq!(o.len(), 1);
        assert!(
            o[0].hyps.contains(&"(not (= b 0))".to_string()),
            "{:?}",
            o[0].hyps
        );
    }

    #[test]
    fn checked_operations_generate_nothing() {
        let o = obs_of("mod T\n\nf (a:U64) (b:U64) : Res Fault U64 = div_checked a b\n");
        assert!(o.is_empty(), "{o:#?}");
        let o = obs_of("mod T\n\nf (v:&Vec U64) (i:Size) : Res Fault U64 = get_checked v i\n");
        assert!(o.is_empty(), "{o:#?}");
    }

    #[test]
    fn indexing_emits_a_bounds_obligation() {
        let o = obs_of("mod T\n\nf (v:&Vec U64) (i:Size) : U64 = get v i\n");
        assert_eq!(o.len(), 1, "{o:#?}");
        assert_eq!(o[0].code, "bounds");
        assert_eq!(o[0].goal, "(and (< i len_v) (>= i 0))");
        assert!(smt(&o[0]).contains("(declare-const len_v Int)"));
    }

    #[test]
    fn machine_addition_emits_an_overflow_obligation() {
        let o = obs_of("mod T\n\nf (a:U8) (b:U8) : U8 = a + b\n");
        assert_eq!(o.len(), 1, "{o:#?}");
        assert_eq!(o[0].code, "overflow");
        assert_eq!(o[0].goal, "(and (<= (+ a b) 255) (>= (+ a b) 0))");
    }

    #[test]
    fn unsigned_subtraction_emits_an_underflow_obligation() {
        let o = obs_of("mod T\n\nf (a:Nat) (b:Nat) : Nat = a - b\n");
        assert_eq!(o.len(), 1, "{o:#?}");
        assert_eq!(o[0].code, "underflow");
        assert_eq!(o[0].goal, "(>= a b)");
    }

    #[test]
    fn a_record_invariant_is_checked_at_construction() {
        let src = "mod T\n\ntype R = { q:U32, q>0 }\n\nf (n:U32) : R = {q=n}\n";
        let o = obs_of(src);
        assert_eq!(o.len(), 1, "{o:#?}");
        assert_eq!(o[0].code, "refine.unproven");
        assert_eq!(o[0].goal, "(> n 0)");
    }

    #[test]
    fn a_callee_precondition_lands_at_the_call_site() {
        let src = "mod T\n\ng (n:U32, n>0) : U32 = n\n\nf (k:U32) : U32 = g k\n";
        let o = obs_of(src);
        assert_eq!(o.len(), 1, "{o:#?}");
        assert_eq!(o[0].code, "refine.unproven");
        assert_eq!(o[0].goal, "(> k 0)");
        assert_eq!(o[0].path, "T.f.body/0");
    }

    #[test]
    fn a_match_arm_teaches_the_solver() {
        let src = "mod T\n\nf (a:U64) (b:U64) : U64 =\n  ?b |0 -> 0 |_ -> a / b end\n";
        let o = obs_of(src);
        assert_eq!(o.len(), 1, "{o:#?}");
        assert!(
            o[0].hyps.iter().any(|h| h == "(not (= b 0))"),
            "{:?}",
            o[0].hyps
        );
    }

    #[test]
    fn model_parsing_yields_a_counterexample() {
        let out = "sat\n(\n  (define-fun b () Int 0)\n  (define-fun a () Int (- 3))\n)";
        assert_eq!(model(out).as_deref(), Some("b=0, a=-3"));
    }
}

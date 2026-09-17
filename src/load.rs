//! Loading a program (spec §9): one file, one module, and qualification is the
//! only import syntax there is. `Ledger.total` names a function in `Ledger`,
//! and `Ledger` is the file `Ledger.vibe` sitting next to the one that names
//! it. No `import` line, no alias, no search path — one construct fewer, and
//! one fewer place for a generator to choose wrong.
//!
//! Modules are then flattened into a single unit and handed to the checker
//! unchanged. §9 gives v0.1 no visibility system at all — everything declared
//! is visible to an importer — so a flat namespace loses nothing that exists
//! yet, and a name declared twice is reported rather than silently shadowed.
//!
//! ponytail: the ceiling is exactly that flat namespace. Two modules cannot
//! both declare `parse`, and a type cannot be written `Ledger.Tx` because after
//! flattening there is only `Tx`. §9 already says this design has to be
//! revisited before v1; the upgrade path is per-module name resolution in
//! `infer`, at which point this pass keeps the loading and drops the renaming.

use crate::ast::*;
use crate::diag::{Diag, Files, Span};
use crate::lexer::{self, Comment};
use crate::parser;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One file, as it was written. Modules keep their qualifiers here: only
/// `Program::flat` is rewritten, because a projection has to print the program
/// the way the file spells it.
pub struct Unit {
    pub module: Module,
    pub src: String,
    pub comments: Vec<Comment>,
    pub file: usize,
}

/// The program as the rest of the compiler wants it.
pub struct Program {
    /// Every module's declarations in one unit, dependencies first. This is
    /// what gets checked, proved and compiled.
    pub flat: Module,
    /// Every file that was read, dependencies first, the root last. Canonicity
    /// is per file, so it is checked here rather than on the flattened whole.
    pub units: Vec<Unit>,
}

impl Program {
    /// The file named on the command line. Projections and patches address one
    /// file, never the flattened whole.
    pub fn root(&self) -> &Unit {
        self.units.last().expect("a program has at least its root")
    }
}

/// Read `path` and everything it names, transitively.
pub fn program(path: &Path, files: &mut Files) -> Result<Program, Vec<Diag>> {
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut l = Loader {
        dir,
        files,
        loaded: HashMap::new(),
        order: Vec::new(),
    };
    let root = l.read(path, None)?;
    l.follow(&root.module)?;

    let units: Vec<Unit> = l.order.drain(..).chain(std::iter::once(root)).collect();
    let table: Table = units
        .iter()
        .map(|u| (u.module.name.clone(), syms_of(&u.module)))
        .collect();

    // Two kinds of name stay global because the rest of the compiler resolves
    // them without a module in hand: a record field, which the checker finds by
    // name alone, and an `ext c` symbol, which is C's name and not ours.
    let mut errors = global_clashes(&units);

    let mut decls = Vec::new();
    for u in &units {
        let mut ds = u.module.decls.clone();
        let mut mg = Mangler {
            home: u.module.name.clone(),
            syms: &table[&u.module.name],
            table: &table,
            scopes: Vec::new(),
            errors: Vec::new(),
        };
        for d in &mut ds {
            mg.decl(d);
        }
        errors.append(&mut mg.errors);
        decls.extend(ds);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let root = units.last().expect("the root was pushed last");
    let flat = Module {
        name: root.module.name.clone(),
        decls,
        span: root.module.span,
    };
    Ok(Program { flat, units })
}

/// What a module declares, by kind. The three namespaces are separate because
/// the grammar keeps them separate: a lower-case name is a value, a capitalised
/// one in type position is a type, and one in a pattern is a constructor.
#[derive(Default)]
struct Syms {
    vals: HashSet<String>,
    types: HashSet<String>,
    ctors: HashSet<String>,
}

type Table = HashMap<String, Syms>;

fn syms_of(m: &Module) -> Syms {
    let mut s = Syms::default();
    for d in &m.decls {
        match d {
            Decl::Fun(f) => {
                s.vals.insert(f.name.clone());
            }
            Decl::Type(t) => {
                s.types.insert(t.name.clone());
                if let TypeBody::Variants(vs) = &t.body {
                    s.ctors.extend(vs.iter().map(|v| v.name.clone()));
                }
            }
            // `ext c` names are C's, and stay global.
            Decl::Ext(_) | Decl::Exp(..) => {}
        }
    }
    s
}

/// Names that two modules may not both declare, because nothing downstream
/// carries a module with them.
///
/// Only `ext c` symbols are left here, and even they are not a clash by being
/// declared twice: the symbol belongs to C, and two modules naming the same one
/// are talking about the same function. What they may not do is disagree about
/// its type — C will link whichever it likes and the checker will have proved
/// something about the other.
///
/// Record fields used to be on this list. They are resolved per use site now
/// (`Checked::field_of`), so two modules may both declare `qty`.
fn global_clashes(units: &[Unit]) -> Vec<Diag> {
    let mut seen: HashMap<String, (String, String)> = HashMap::new();
    let mut out = Vec::new();
    for u in units {
        let m = &u.module;
        for d in &m.decls {
            let Decl::Ext(e) = d else { continue };
            for sig in &e.sigs {
                let shape = format!("{} : {}", sig.symbol, crate::view::ty(&sig.ty));
                match seen.get(&sig.name) {
                    Some((first, was)) if first != &m.name && was != &shape => out.push(
                        Diag::error(
                            sig.span,
                            "mod.duplicate",
                            &format!(
                                "the `ext c` symbol `{}` is declared with two different types, in `{first}` and `{}`",
                                sig.name, m.name
                            ),
                        )
                        .with_path(&format!("{}.{}", m.name, sig.name))
                        .with_witness(&format!("`{first}` declares {was}; `{}` declares {shape}", m.name))
                        .with_fix("make the two agree: a C symbol has one type, and no module to hide behind"),
                    ),
                    _ => {
                        seen.insert(sig.name.clone(), (m.name.clone(), shape));
                    }
                }
            }
        }
    }
    out
}

/// Rewrites one module's declarations into the flattened program's spelling:
/// every name a module declares becomes `Mod.name`, which is exactly what a
/// qualified reference was already written as. So `Csv.parse` and `Json.parse`
/// are two names, a module may declare `take` without fighting the prelude, and
/// a bare name means "mine, else the prelude's".
///
/// The per-file `Unit` keeps the source spelling: `vibe view` and `vibe patch`
/// work on that, and neither has to know any of this happened.
struct Mangler<'a> {
    home: String,
    syms: &'a Syms,
    table: &'a Table,
    /// names bound by a parameter, a `let`, a `<-`, a lambda or a pattern —
    /// they shadow the module's own, so they are not rewritten
    scopes: Vec<HashSet<String>>,
    errors: Vec<Diag>,
}

impl Mangler<'_> {
    fn mine(&self, n: &str) -> String {
        format!("{}.{}", self.home, n)
    }

    fn shadowed(&self, n: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(n))
    }

    /// A qualified name is checked against the module it names, so a typo is a
    /// diagnostic here rather than an unbound-name error further down.
    fn qualified(&mut self, n: &str, span: Span, kind: &str) {
        let Some((m, base)) = n.split_once('.') else {
            return;
        };
        let Some(syms) = self.table.get(m) else {
            return;
        };
        let known = match kind {
            "value" => &syms.vals,
            "type" => &syms.types,
            _ => &syms.ctors,
        };
        if !known.contains(base) {
            self.errors.push(
                Diag::error(
                    span,
                    "mod.no_name",
                    &format!("`{m}` declares no {kind} `{base}`"),
                )
                .with_path(&format!("{}.{}", self.home, base))
                .with_fix(&format!(
                    "check the spelling, or declare `{base}` in {m}.vibe"
                )),
            );
        }
    }

    fn decl(&mut self, d: &mut Decl) {
        match d {
            Decl::Fun(f) => {
                let mut bound = HashSet::new();
                for p in &mut f.params {
                    if let Some(t) = &mut p.ty {
                        self.ty(t);
                    }
                    bound.extend(p.names.iter().cloned());
                }
                self.scopes.push(bound);
                for p in &mut f.params {
                    for r in &mut p.refines {
                        self.expr(r);
                    }
                }
                if let Some(r) = &mut f.ret {
                    self.ty(r);
                }
                self.expr(&mut f.body);
                if let Some(ms) = &mut f.measure {
                    self.expr(ms);
                }
                self.scopes.pop();
                f.name = self.mine(&f.name);
            }
            Decl::Type(t) => {
                match &mut t.body {
                    TypeBody::Variants(vs) => {
                        for v in vs.iter_mut() {
                            for a in &mut v.args {
                                self.ty(a);
                            }
                            v.name = self.mine(&v.name);
                        }
                    }
                    TypeBody::Record(r) => {
                        let fields: HashSet<String> =
                            r.fields.iter().map(|(n, _)| n.clone()).collect();
                        for (_, ft) in r.fields.iter_mut() {
                            self.ty(ft);
                        }
                        self.scopes.push(fields);
                        for e in &mut r.refines {
                            self.expr(e);
                        }
                        self.scopes.pop();
                    }
                    TypeBody::Opaque => {}
                }
                t.name = self.mine(&t.name);
            }
            Decl::Ext(e) => {
                for s in &mut e.sigs {
                    self.ty(&mut s.ty);
                }
                for t in &mut e.types {
                    t.name = self.mine(&t.name);
                }
            }
            Decl::Exp(names, _) => {
                for n in names.iter_mut() {
                    if self.syms.vals.contains(n) {
                        *n = self.mine(n);
                    }
                }
            }
        }
    }

    fn ty(&mut self, t: &mut Ty) {
        match t {
            Ty::Con(n, args) => {
                if n.contains('.') {
                    let span = Span::default();
                    self.qualified(&n.clone(), span, "type");
                } else if self.syms.types.contains(n) {
                    *n = self.mine(n);
                }
                for a in args {
                    self.ty(a);
                }
            }
            Ty::Ref(i) | Ty::Eff(i) => self.ty(i),
            Ty::Fun(a, b) => {
                self.ty(a);
                self.ty(b);
            }
            Ty::Tuple(xs) => xs.iter_mut().for_each(|x| self.ty(x)),
            Ty::Var(_) => {}
        }
    }

    fn pat(&mut self, p: &mut Pat, bound: &mut HashSet<String>) {
        match p {
            Pat::Var(n) => {
                bound.insert(n.clone());
            }
            Pat::Ctor(c, ps) => {
                if c.contains('.') {
                    let c = c.clone();
                    self.qualified(&c, Span::default(), "constructor");
                } else if self.syms.ctors.contains(c) {
                    *c = self.mine(c);
                }
                for sp in ps {
                    self.pat(sp, bound);
                }
            }
            Pat::Tuple(ps) | Pat::List(ps) => {
                for sp in ps {
                    self.pat(sp, bound);
                }
            }
            _ => {}
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        let span = e.span;
        match &mut e.kind {
            ExprKind::Field(base, name) => {
                // `Csv.parse`: a qualifier is written exactly like a field
                // access on a constructor, and this is where the two part ways.
                if let ExprKind::Ctor(m) = &base.kind {
                    if let Some(syms) = self.table.get(m) {
                        if syms.vals.contains(name) {
                            e.kind = ExprKind::Var(format!("{m}.{name}"));
                            return;
                        }
                        self.errors.push(
                            Diag::error(
                                span,
                                "mod.no_name",
                                &format!("`{m}` declares no value `{name}`"),
                            )
                            .with_path(&format!("{}.{}", self.home, name))
                            .with_fix(&format!(
                                "check the spelling, or declare `{name}` in {m}.vibe"
                            )),
                        );
                        return;
                    }
                }
                self.expr(base);
            }
            ExprKind::Var(n) => {
                if !self.shadowed(n) && self.syms.vals.contains(n) {
                    *n = self.mine(n);
                }
            }
            ExprKind::Ctor(c) => {
                if c.contains('.') {
                    let c = c.clone();
                    self.qualified(&c, span, "constructor");
                } else if self.syms.ctors.contains(c) {
                    *c = self.mine(c);
                }
            }
            ExprKind::Let(n, v, body) | ExprKind::Bind(n, v, body) => {
                self.expr(v);
                self.scopes.push(HashSet::from([n.clone()]));
                self.expr(body);
                self.scopes.pop();
            }
            ExprKind::Lambda(ps, body) => {
                self.scopes.push(ps.iter().cloned().collect());
                self.expr(body);
                self.scopes.pop();
            }
            ExprKind::Match(scrut, arms) => {
                self.expr(scrut);
                for (p, body) in arms.iter_mut() {
                    let mut bound = HashSet::new();
                    self.pat(p, &mut bound);
                    self.scopes.push(bound);
                    self.expr(body);
                    self.scopes.pop();
                }
            }
            ExprKind::Arena(n, body) => {
                self.scopes.push(HashSet::from([n.clone()]));
                self.expr(body);
                self.scopes.pop();
            }
            ExprKind::App(h, args) => {
                self.expr(h);
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Binop(_, a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Neg(x) | ExprKind::Not(x) | ExprKind::Borrow(x) => self.expr(x),
            ExprKind::Record(base, fields) => {
                if let Some(b) = base {
                    self.expr(b);
                }
                for (_, v) in fields.iter_mut() {
                    self.expr(v);
                }
            }
            ExprKind::Tuple(xs) | ExprKind::List(xs) => {
                for x in xs {
                    self.expr(x);
                }
            }
            _ => {}
        }
    }
}

struct Loader<'a> {
    dir: PathBuf,
    files: &'a mut Files,
    /// module name -> the file it came from, so a second reference is free and
    /// a cycle terminates
    loaded: HashMap<String, PathBuf>,
    order: Vec<Unit>,
}

/// Where a module may live, in order: beside the file that names it, then each
/// entry of `VIBE_PATH`, then the `lib` directory shipped with the compiler.
///
/// No manifest, no lockfile, no versions. A name maps to a file mechanically,
/// in both directions, which is the property that lets a generator write
/// `Json.parse` without being told where `Json` came from.
fn search_path(dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![dir.to_path_buf()];
    if let Ok(p) = std::env::var("VIBE_PATH") {
        v.extend(p.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            v.push(d.join("lib"));
            v.push(d.join("../lib"));
        }
    }
    v.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib"));
    v
}

impl Loader<'_> {
    /// The file a module name resolves to. Case and underscores are the only
    /// things allowed to differ, so `MoneyBox` is `MoneyBox.vibe` or
    /// `money_box.vibe` and nothing else.
    fn find(&self, name: &str) -> Option<PathBuf> {
        for d in search_path(&self.dir) {
            for stem in [name.to_string(), snake_case(name)] {
                let p = d.join(format!("{stem}.vibe"));
                if p.exists() {
                    return Some(p);
                }
            }
        }
        None
    }

    fn read(&mut self, path: &Path, at: Option<Span>) -> Result<Unit, Vec<Diag>> {
        let src = std::fs::read_to_string(path).map_err(|e| {
            vec![Diag::error(
                at.unwrap_or_default(),
                "mod.unreadable",
                &format!("cannot read {}: {e}", path.display()),
            )]
        })?;
        let fid = self.files.add(&path.display().to_string(), &src);
        let (toks, comments) = lexer::lex_full(&src, fid).map_err(|d| vec![d])?;
        let m = parser::parse(toks).map_err(|d| vec![d])?;
        // A module name is a constructor name and a file name is a file name,
        // so `TotalOk` lives in `total_ok.vibe` as readily as in `TotalOk.vibe`.
        // Case and underscores are the only things allowed to differ: the
        // mapping from a qualified name to a file has to stay mechanical.
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if squash(&m.name) != squash(&stem) {
            return Err(vec![Diag::error(
                m.span,
                "mod.name",
                &format!(
                    "`mod {}` does not match the file name `{stem}.vibe`",
                    m.name
                ),
            )
            .with_fix(&format!(
                "rename the file to {}.vibe, or the module to {stem}",
                m.name
            ))]);
        }
        self.loaded.insert(m.name.clone(), path.to_path_buf());
        Ok(Unit {
            module: m,
            src,
            comments,
            file: fid,
        })
    }

    /// Load every module `m` names, depth first, so `order` ends up with
    /// dependencies before the modules that use them.
    fn follow(&mut self, m: &Module) -> Result<(), Vec<Diag>> {
        for (name, span) in qualifiers(m) {
            if self.loaded.contains_key(&name) {
                continue; // already loaded, or being loaded: a cycle stops here
            }
            let Some(p) = self.find(&name) else {
                let looked: Vec<String> = search_path(&self.dir)
                    .iter()
                    .map(|d| d.join(format!("{name}.vibe")).display().to_string())
                    .collect();
                return Err(vec![Diag::error(
                    span,
                    "mod.missing",
                    &format!("no module `{name}`: looked in {}", looked.join(", ")),
                )
                .with_fix(&format!(
                    "create {name}.vibe next to this file, or put it on VIBE_PATH"
                ))]);
            };
            let dep = self.read(&p, Some(span))?;
            self.follow(&dep.module)?;
            self.order.push(dep);
        }
        Ok(())
    }
}

/// Case and underscores carry no information in the file-name mapping.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// `TotalOk` -> `total_ok`: the spelling a file is most likely to use.
fn snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.char_indices() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// Every `Mod.name` in the module, with the span to blame if `Mod` is missing.
/// A qualifier is written exactly like a field access on a constructor, so the
/// two are told apart by case and by whether the file exists — that check is
/// the caller's.
fn qualifiers(m: &Module) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for d in &m.decls {
        match d {
            Decl::Fun(f) => {
                for p in &f.params {
                    if let Some(t) = &p.ty {
                        ty_qualifiers(t, f.span, &mut out);
                    }
                }
                if let Some(r) = &f.ret {
                    ty_qualifiers(r, f.span, &mut out);
                }
                find_qualifiers(&f.body, &mut out);
            }
            Decl::Type(t) => match &t.body {
                TypeBody::Variants(vs) => {
                    for v in vs {
                        for a in &v.args {
                            ty_qualifiers(a, t.span, &mut out);
                        }
                    }
                }
                TypeBody::Record(r) => {
                    for (_, ft) in &r.fields {
                        ty_qualifiers(ft, t.span, &mut out);
                    }
                }
                TypeBody::Opaque => {}
            },
            Decl::Ext(e) => {
                for sg in &e.sigs {
                    ty_qualifiers(&sg.ty, e.span, &mut out);
                }
            }
            Decl::Exp(..) => {}
        }
    }
    out
}

/// The module in a dotted name, if it has one. `Shape.Kind` is written as one
/// name by the parser, so a module can be named by a type or a constructor and
/// never by a call — and that module still has to be loaded.
fn dotted(n: &str) -> Option<String> {
    n.split_once('.').map(|(m, _)| m.to_string())
}

fn ty_qualifiers(t: &Ty, span: Span, out: &mut Vec<(String, Span)>) {
    match t {
        Ty::Con(n, args) => {
            if let Some(m) = dotted(n) {
                out.push((m, span));
            }
            for a in args {
                ty_qualifiers(a, span, out);
            }
        }
        Ty::Ref(i) | Ty::Eff(i) => ty_qualifiers(i, span, out),
        Ty::Fun(a, b) => {
            ty_qualifiers(a, span, out);
            ty_qualifiers(b, span, out);
        }
        Ty::Tuple(xs) => xs.iter().for_each(|x| ty_qualifiers(x, span, out)),
        Ty::Var(_) => {}
    }
}

fn pat_qualifiers(p: &Pat, span: Span, out: &mut Vec<(String, Span)>) {
    match p {
        Pat::Ctor(c, ps) => {
            if let Some(m) = dotted(c) {
                out.push((m, span));
            }
            ps.iter().for_each(|sp| pat_qualifiers(sp, span, out));
        }
        Pat::Tuple(ps) | Pat::List(ps) => ps.iter().for_each(|sp| pat_qualifiers(sp, span, out)),
        _ => {}
    }
}

fn find_qualifiers(e: &Expr, out: &mut Vec<(String, Span)>) {
    match &e.kind {
        ExprKind::Field(base, _) => {
            if let ExprKind::Ctor(mo) = &base.kind {
                out.push((mo.clone(), e.span));
            }
        }
        ExprKind::Ctor(c) => {
            if let Some(m) = dotted(c) {
                out.push((m, e.span));
            }
        }
        ExprKind::Match(_, arms) => {
            for (p, _) in arms {
                pat_qualifiers(p, e.span, out);
            }
        }
        _ => {}
    }
    e.children(&mut |k| find_qualifiers(k, out));
}

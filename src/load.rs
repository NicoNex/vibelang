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
use std::collections::HashMap;
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
    let mut l = Loader { dir, files, loaded: HashMap::new(), order: Vec::new() };
    let root = l.read(path, None)?;
    l.follow(&root.module)?;

    let mut decls = Vec::new();
    let mut home: HashMap<String, String> = HashMap::new();
    let mut errors = Vec::new();
    // dependencies first, so a reader of the flattened unit sees a definition
    // before its use even though the checker does not require it
    let units: Vec<Unit> = l.order.drain(..).chain(std::iter::once(root)).collect();
    for u in &units {
        let m = &u.module;
        for d in m.decls.iter().cloned() {
            for n in declared(&d) {
                if let Some(first) = home.get(&n) {
                    errors.push(
                        Diag::error(
                            decl_span(&d),
                            "mod.duplicate",
                            &format!("`{n}` is declared in both `{first}` and `{}`", m.name),
                        )
                        .with_path(&format!("{}.{n}", m.name))
                        .with_fix("v0.1 has one flat namespace (spec §9): rename one of them"),
                    );
                } else {
                    home.insert(n, m.name.clone());
                }
            }
            decls.push(d);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let root = units.last().expect("the root was pushed last");
    let mut flat =
        Module { name: root.module.name.clone(), decls, span: root.module.span };
    let known: Vec<String> = home.values().cloned().collect();
    // Only the flattened unit is resolved: a projection has to show the program
    // as it is written, qualifiers included, or `vibe view` would edit the file
    // every time it printed it.
    resolve(&mut flat, &known);
    Ok(Program { flat, units })
}

struct Loader<'a> {
    dir: PathBuf,
    files: &'a mut Files,
    /// module name -> the file it came from, so a second reference is free and
    /// a cycle terminates
    loaded: HashMap<String, PathBuf>,
    order: Vec<Unit>,
}

impl Loader<'_> {
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
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if squash(&m.name) != squash(&stem) {
            return Err(vec![Diag::error(
                m.span,
                "mod.name",
                &format!("`mod {}` does not match the file name `{stem}.vibe`", m.name),
            )
            .with_fix(&format!("rename the file to {}.vibe, or the module to {stem}", m.name))]);
        }
        self.loaded.insert(m.name.clone(), path.to_path_buf());
        Ok(Unit { module: m, src, comments, file: fid })
    }

    /// Load every module `m` names, depth first, so `order` ends up with
    /// dependencies before the modules that use them.
    fn follow(&mut self, m: &Module) -> Result<(), Vec<Diag>> {
        for (name, span) in qualifiers(m) {
            if self.loaded.contains_key(&name) {
                continue; // already loaded, or being loaded: a cycle stops here
            }
            let exact = self.dir.join(format!("{name}.vibe"));
            let snake = self.dir.join(format!("{}.vibe", snake_case(&name)));
            let Some(p) = [exact.clone(), snake].into_iter().find(|p| p.exists()) else {
                return Err(vec![Diag::error(
                    span,
                    "mod.missing",
                    &format!("no module `{name}`: expected {}", exact.display()),
                )
                .with_fix(&format!("create {name}.vibe, or fix the qualified name"))]);
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
    s.chars().filter(|c| *c != '_').flat_map(|c| c.to_lowercase()).collect()
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

fn decl_span(d: &Decl) -> Span {
    match d {
        Decl::Type(t) => t.span,
        Decl::Fun(f) => f.span,
        Decl::Ext(e) => e.span,
        Decl::Exp(_, s) => *s,
    }
}

/// The top-level names a declaration introduces. Constructors count, and so do
/// record field names: the checker resolves a field to its owning record by
/// name alone, so two modules that both spell a field `qty` would otherwise
/// meet as a type error somewhere else entirely.
fn declared(d: &Decl) -> Vec<String> {
    match d {
        Decl::Fun(f) => vec![f.name.clone()],
        Decl::Type(t) => {
            let mut v = vec![t.name.clone()];
            match &t.body {
                TypeBody::Variants(vs) => v.extend(vs.iter().map(|x| x.name.clone())),
                TypeBody::Record(r) => v.extend(r.fields.iter().map(|(n, _)| n.clone())),
                TypeBody::Opaque => {}
            }
            v
        }
        Decl::Ext(e) => e.sigs.iter().map(|s| s.name.clone()).collect(),
        Decl::Exp(..) => Vec::new(),
    }
}

/// Every `Mod.name` in the module, with the span to blame if `Mod` is missing.
/// A qualifier is written exactly like a field access on a constructor, so the
/// two are told apart by case and by whether the file exists — that check is
/// the caller's.
fn qualifiers(m: &Module) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for f in m.funs() {
        find_qualifiers(&f.body, &mut out);
    }
    out
}

fn find_qualifiers(e: &Expr, out: &mut Vec<(String, Span)>) {
    if let ExprKind::Field(base, _) = &e.kind {
        if let ExprKind::Ctor(mo) = &base.kind {
            out.push((mo.clone(), e.span));
        }
    }
    children(e, &mut |k| find_qualifiers(k, out));
}

/// Rewrite `Mod.name` to `name` once `Mod` is known to be a module. Anything
/// qualified by something that is not a loaded module is left alone, so a real
/// field access on a constructor still fails where it always did.
fn resolve(m: &mut Module, modules: &[String]) {
    for d in &mut m.decls {
        if let Decl::Fun(f) = d {
            rewrite(&mut f.body, modules);
        }
    }
}

fn rewrite(e: &mut Expr, modules: &[String]) {
    if let ExprKind::Field(base, name) = &e.kind {
        if let ExprKind::Ctor(mo) = &base.kind {
            if modules.iter().any(|x| x == mo) {
                e.kind = ExprKind::Var(name.clone());
                return;
            }
        }
    }
    children_mut(e, &mut |k| rewrite(k, modules));
}

/// Apply `f` to every direct subexpression.
fn children(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    use ExprKind::*;
    match &e.kind {
        Int(_) | Float(_) | Str(_) | Char(_) | Bool(_) | Unit | Var(_) | Ctor(_) => {}
        App(h, args) => {
            f(h);
            args.iter().for_each(&mut *f);
        }
        Binop(_, a, b) | Bind(_, a, b) | Let(_, a, b) => {
            f(a);
            f(b);
        }
        Neg(i) | Not(i) | Borrow(i) | Field(i, _) | Lambda(_, i) | Arena(_, i) => f(i),
        Match(s, arms) => {
            f(s);
            arms.iter().for_each(|(_, b)| f(b));
        }
        Record(base, fields) => {
            base.iter().for_each(|b| f(b));
            fields.iter().for_each(|(_, v)| f(v));
        }
        Tuple(xs) | List(xs) => xs.iter().for_each(&mut *f),
    }
}

fn children_mut(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    use ExprKind::*;
    match &mut e.kind {
        Int(_) | Float(_) | Str(_) | Char(_) | Bool(_) | Unit | Var(_) | Ctor(_) => {}
        App(h, args) => {
            f(h);
            args.iter_mut().for_each(&mut *f);
        }
        Binop(_, a, b) | Bind(_, a, b) | Let(_, a, b) => {
            f(a);
            f(b);
        }
        Neg(i) | Not(i) | Borrow(i) | Field(i, _) | Lambda(_, i) | Arena(_, i) => f(i),
        Match(s, arms) => {
            f(s);
            arms.iter_mut().for_each(|(_, b)| f(b));
        }
        Record(base, fields) => {
            base.iter_mut().for_each(|b| f(b));
            fields.iter_mut().for_each(|(_, v)| f(v));
        }
        Tuple(xs) | List(xs) => xs.iter_mut().for_each(&mut *f),
    }
}

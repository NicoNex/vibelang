//! Whole-module checking: name resolution, Hindley-Milner inference, effect
//! propagation, exhaustiveness. Errors are collected per declaration so one run
//! reports everything that is wrong, not just the first thing.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer;
use crate::parser::Parser;
use crate::types::*;
use std::collections::HashMap;

type R<X> = Result<X, Diag>;

pub struct Checked {
    pub data: Data,
    pub ext: HashMap<String, ExtSig>,
    pub sigs: HashMap<String, Scheme>,
    /// Which `let`, `<-` and pattern binders hold an affine value, keyed by the
    /// span of the expression they scope over and their name. Ownership has no
    /// other way to know: a binder has no written type (spec §4.2).
    pub affine: HashMap<(usize, usize, usize, String), bool>,
}

pub fn parse_type(src: &str) -> Ty {
    let toks = lexer::lex(src, usize::MAX).expect("prelude type lexes");
    let mut p = Parser { toks, i: 0, depth: 1, home: "Prelude".into() };
    p.ty().expect("prelude type parses")
}

pub fn prelude_module() -> Module {
    let toks = lexer::lex(PRELUDE_TYPES, usize::MAX).expect("prelude lexes");
    crate::parser::parse(toks).expect("prelude parses")
}

pub fn check(m: &Module) -> Result<Checked, Vec<Diag>> {
    let mut c = Checker::new();
    let mut ext: HashMap<String, ExtSig> = HashMap::new();

    // 1. Data declarations: prelude first, then the module.
    let prelude = prelude_module();
    for t in prelude.types() {
        collect_type(&mut c, t);
    }
    for t in m.types() {
        if c.data.records.contains_key(&t.name) || c.data.variants.contains_key(&t.name) {
            c.errors.push(
                Diag::error(t.span, "name.duplicate", &format!("type `{}` is already defined", t.name))
                    .with_path(&format!("{}.{}", t.home, t.name)),
            );
        }
        collect_type(&mut c, t);
    }

    // 2. Prelude values.
    for (name, sig) in PRELUDE_SIGS {
        let ty = parse_type(sig);
        let mut vars = HashMap::new();
        let t = c.lower_ty(&ty, &mut vars);
        let s = c.generalise(&t);
        c.sigs.insert(name.to_string(), s);
    }

    // 3. Constructors.
    let ctor_names: Vec<String> = c.data.ctors.keys().cloned().collect();
    for name in ctor_names {
        let info = c.data.ctors[&name].clone();
        let mut vars: HashMap<String, T> = HashMap::new();
        let owner_args: Vec<T> =
            info.params.iter().map(|p| c.lower_ty(&Ty::Var(p.clone()), &mut vars)).collect();
        let mut ty = T::Con(info.owner.clone(), owner_args);
        for a in info.args.iter().rev() {
            let at = c.lower_ty(a, &mut vars);
            ty = T::Fun(Box::new(at), Box::new(ty));
        }
        let s = c.generalise(&ty);
        c.sigs.insert(name, s);
    }

    // 4. `ext c` signatures. Everything crossing the C boundary is `E!` (spec §5.2).
    for block in m.exts() {
        for sig in &block.sigs {
            let mut vars = HashMap::new();
            let t = c.lower_ty(&sig.ty, &mut vars);
            if !returns_eff(&t) {
                c.errors.push(
                    Diag::error(
                        sig.span,
                        "ext.pure",
                        &format!("`{}` comes from C, so its result must be `E!`", sig.name),
                    )
                    .with_path(&format!("{}.{}", m.name, sig.name))
                    .with_fix(&format!("write `{} : ... -> E! {}`", sig.name, result_name(&t))),
                );
            }
            let s = c.generalise(&t);
            c.sigs.insert(sig.name.clone(), s);
            ext.insert(sig.name.clone(), sig.clone());
        }
    }

    // 5. Function signatures, before any body, so recursion and forward use work.
    for f in m.funs() {
        if c.sigs.contains_key(&f.name) && !ext.contains_key(&f.name) {
            c.errors.push(
                Diag::error(f.span, "name.duplicate", &format!("`{}` is already defined", f.name))
                    .with_path(&format!("{}.{}", f.home, f.name)),
            );
        }
        let mut vars = HashMap::new();
        let fully_annotated = f.ret.is_some() && f.params.iter().all(|p| p.ty.is_some());
        let mut ty = match &f.ret {
            Some(r) => c.lower_ty(r, &mut vars),
            None => c.fresh(Kind::Any),
        };
        for p in f.params.iter().rev() {
            for _ in 0..p.names.len() {
                let pt = match &p.ty {
                    Some(t) => c.lower_ty(t, &mut vars),
                    None => c.fresh(Kind::Any),
                };
                ty = T::Fun(Box::new(pt), Box::new(ty));
            }
        }
        let s = if fully_annotated { c.generalise(&ty) } else { Scheme::mono(ty) };
        c.sigs.insert(f.name.clone(), s);
    }

    // 6. Bodies.
    for f in m.funs() {
        let path = format!("{}.{}", f.home, f.name);
        if let Err(e) = check_fun(&mut c, f, &path) {
            c.errors.push(e.at_path(&path));
        }
    }

    // 7. Record invariants type-check as Bool in the scope of their own fields.
    let recs: Vec<RecordInfo> = c.data.records.values().cloned().collect();
    for r in recs {
        if r.refines.is_empty() {
            continue;
        }
        c.push_scope();
        for (fname, fty) in &r.fields {
            let mut vars = HashMap::new();
            let t = c.lower_ty(fty, &mut vars);
            c.define(fname, Scheme::mono(t));
        }
        for pred in &r.refines {
            let path = format!("{}.{}.invariant", m.name, r.name);
            if let Err(e) = infer_bool(&mut c, pred, &path) {
                c.errors.push(e.at_path(&path));
            }
        }
        c.pop_scope();
    }

    // 8. `exp c` must name something that exists.
    for d in &m.decls {
        if let Decl::Exp(names, span) = d {
            for n in names {
                if !m.funs().any(|f| &f.name == n) {
                    let sug = suggest(n, m.funs().map(|f| &f.name).collect::<Vec<_>>().into_iter());
                    let mut e = Diag::error(
                        *span,
                        "export.unknown",
                        &format!("`{}` is exported but not defined in this module", n),
                    );
                    if let Some(s) = sug {
                        e = e.with_fix(&format!("did you mean `exp c {}`?", s));
                    }
                    c.errors.push(e);
                }
            }
        }
    }

    c.default_numerics();
    c.check_exhaustiveness();

    if c.errors.is_empty() {
        let affine = c
            .binds
            .iter()
            .map(|(sp, n, t)| {
                let k = (sp.file, sp.line, sp.col, n.clone());
                (k, is_affine_t(&c.resolve(t)))
            })
            .collect();
        Ok(Checked { data: c.data, ext, sigs: c.sigs, affine })
    } else {
        c.errors.sort_by_key(|d| (d.span.file, d.span.line, d.span.col));
        Err(c.errors)
    }
}

fn returns_eff(t: &T) -> bool {
    match t {
        T::Fun(_, r) => returns_eff(r),
        T::Eff(_) => true,
        _ => false,
    }
}

fn result_name(t: &T) -> String {
    match t {
        T::Fun(_, r) => result_name(r),
        other => other.show(),
    }
}

fn collect_type(c: &mut Checker, t: &TypeDecl) {
    match &t.body {
        TypeBody::Record(r) => {
            for (f, _) in &r.fields {
                c.data.field_owner.insert(f.clone(), t.name.clone());
            }
            c.data.records.insert(
                t.name.clone(),
                RecordInfo { name: t.name.clone(), fields: r.fields.clone(), refines: r.refines.clone() },
            );
        }
        TypeBody::Variants(vs) => {
            let names: Vec<String> = vs.iter().map(|v| v.name.clone()).collect();
            for (i, v) in vs.iter().enumerate() {
                c.data.ctors.insert(
                    v.name.clone(),
                    CtorInfo {
                        owner: t.name.clone(),
                        params: t.params.clone(),
                        args: v.args.clone(),
                        tag: i,
                    },
                );
            }
            c.data.variants.insert(t.name.clone(), names);
        }
        TypeBody::Opaque => c.data.opaque.push(t.name.clone()),
    }
}

fn check_fun(c: &mut Checker, f: &FunDecl, path: &str) -> R<()> {
    let scheme = c.sigs[&f.name].clone();
    let sig = c.instantiate(&scheme);
    c.push_scope();
    let mut cur = sig.clone();
    for p in &f.params {
        for n in &p.names {
            match cur {
                T::Fun(a, b) => {
                    c.define(n, Scheme::mono(*a));
                    cur = *b;
                }
                _ => {
                    return Err(Diag::error(
                        p.span,
                        "arity.excess",
                        &format!("`{}` has more parameters than its type allows", f.name),
                    ))
                }
            }
        }
    }
    // Refinements are Bool expressions over the parameters.
    for p in &f.params {
        for pred in &p.refines {
            infer_bool(c, pred, path)?;
        }
    }
    if let Some(m) = &f.measure {
        infer(c, m, path)?;
    }

    let declared = cur;
    let body_ty = infer(c, &f.body, path)?;
    let bt = c.resolve(&body_ty);
    match (&declared, &bt) {
        (T::Eff(want), T::Eff(_)) => {
            c.unify(&bt, &T::Eff(want.clone()), f.body.span, "")?;
        }
        // A pure body in an `E!` function is lifted silently: purity is a subtype of effectful.
        (T::Eff(want), _) => {
            c.unify(&bt, want, f.body.span, "")?;
        }
        (_, T::Eff(inner)) => {
            let want = c.resolve(&declared);
            return Err(Diag::error(
                f.body.span,
                "effect.missing",
                &format!("`{}` performs effects but its result type is pure `{}`", f.name, want.show()),
            )
            .with_fix(&format!("change the result type to `E! {}`", inner.show())));
        }
        _ => {
            c.unify(&bt, &declared, f.body.span, "")?;
        }
    }
    c.pop_scope();
    Ok(())
}

fn infer_bool(c: &mut Checker, e: &Expr, path: &str) -> R<()> {
    let t = infer(c, e, path)?;
    c.unify(&t, &T::con("Bool"), e.span, " (a refinement must be a Bool)")
}

fn effect_of(t: &T) -> Option<T> {
    match t {
        T::Eff(x) => Some((**x).clone()),
        _ => None,
    }
}

pub fn infer(c: &mut Checker, e: &Expr, path: &str) -> R<T> {
    match &e.kind {
        ExprKind::Int(_) => Ok(c.fresh(Kind::Num)),
        ExprKind::Float(_) => Ok(c.fresh(Kind::Float)),
        ExprKind::Str(_) => Ok(T::con("Str")),
        ExprKind::Char(_) => Ok(T::con("Char")),
        ExprKind::Bool(_) => Ok(T::con("Bool")),
        ExprKind::Unit => Ok(T::unit()),
        ExprKind::Borrow(inner) => infer(c, inner, path),
        // The arena name is a scope marker, not a value: nothing can refer to it yet.
        ExprKind::Arena(_, body) => infer(c, body, path),

        ExprKind::Var(n) => {
            if is_special(n) {
                return Ok(match n.as_str() {
                    "len" => {
                        let a = c.fresh(Kind::Any);
                        T::Fun(Box::new(a), Box::new(T::con("U64")))
                    }
                    "show" => {
                        let a = c.fresh(Kind::Any);
                        T::Fun(Box::new(a), Box::new(T::con("Str")))
                    }
                    _ => T::Fun(Box::new(T::con("Str")), Box::new(T::con("Str"))),
                });
            }
            match c.lookup(n) {
                Some(s) => Ok(c.instantiate(&s)),
                None => {
                    let mut names: Vec<String> = c.sigs.keys().cloned().collect();
                    for scope in &c.env {
                        names.extend(scope.keys().cloned());
                    }
                    let mut d = Diag::error(e.span, "name.unbound", &format!("`{}` is not defined", n));
                    if let Some(s) = suggest(n, names.iter()) {
                        d = d.with_fix(&format!("did you mean `{}`?", s));
                    }
                    Err(d.at_path(path))
                }
            }
        }

        ExprKind::Ctor(n) => match c.lookup(n) {
            Some(s) => Ok(c.instantiate(&s)),
            None => {
                let names: Vec<String> = c.data.ctors.keys().cloned().collect();
                let mut d =
                    Diag::error(e.span, "name.unbound", &format!("constructor `{}` is not defined", n));
                if let Some(s) = suggest(n, names.iter()) {
                    d = d.with_fix(&format!("did you mean `{}`?", s));
                }
                Err(d.at_path(path))
            }
        },

        ExprKind::App(head, args) => {
            if let ExprKind::Var(n) = &head.kind {
                if n == "fmt" {
                    if args.is_empty() {
                        return Err(Diag::error(e.span, "fmt.args", "`fmt` needs a format string"));
                    }
                    let f = infer(c, &args[0], path)?;
                    c.unify(&f, &T::con("Str"), args[0].span, " (the first argument of `fmt`)")?;
                    for a in &args[1..] {
                        infer(c, a, path)?;
                    }
                    return Ok(T::con("Str"));
                }
                if n == "len" || n == "show" {
                    if args.len() != 1 {
                        return Err(Diag::error(
                            e.span,
                            "arity.mismatch",
                            &format!("`{}` takes exactly one argument", n),
                        ));
                    }
                    let a = infer(c, &args[0], path)?;
                    if n == "len" {
                        if let T::Con(tn, _) = c.resolve(&a) {
                            if tn != "Vec" && tn != "Str" {
                                return Err(Diag::error(
                                    args[0].span,
                                    "type.mismatch",
                                    &format!("`len` works on `Vec a` or `Str`, not `{}`", tn),
                                ));
                            }
                        }
                        return Ok(T::con("U64"));
                    }
                    return Ok(T::con("Str"));
                }
            }
            let mut ft = infer(c, head, path)?;
            for (i, a) in args.iter().enumerate() {
                let at = infer(c, a, path)?;
                let rt = c.fresh(Kind::Any);
                let want = T::Fun(Box::new(at), Box::new(rt.clone()));
                let resolved = c.resolve(&ft);
                if !matches!(resolved, T::Fun(_, _) | T::Var(_)) {
                    return Err(Diag::error(
                        a.span,
                        "arity.excess",
                        &format!(
                            "too many arguments: this call already produced `{}` after {} argument{}",
                            resolved.show(),
                            i,
                            if i == 1 { "" } else { "s" }
                        ),
                    )
                    .with_fix("remove the extra arguments")
                    .at_path(path));
                }
                c.unify(&ft, &want, a.span, &format!(" (argument {})", i + 1))?;
                ft = rt;
            }
            Ok(ft)
        }

        ExprKind::Binop(op, a, b) => {
            let ta = infer(c, a, path)?;
            let tb = infer(c, b, path)?;
            match op.as_str() {
                "&&" | "||" => {
                    c.unify(&ta, &T::con("Bool"), a.span, " (left of a logical operator)")?;
                    c.unify(&tb, &T::con("Bool"), b.span, " (right of a logical operator)")?;
                    Ok(T::con("Bool"))
                }
                "==" | "!=" => {
                    c.unify(&ta, &tb, e.span, " (both sides of a comparison)")?;
                    Ok(T::con("Bool"))
                }
                "<" | "<=" | ">" | ">=" => {
                    let n = c.fresh(Kind::Num);
                    c.unify(&ta, &n, a.span, " (left of a comparison)")?;
                    c.unify(&tb, &n, b.span, " (right of a comparison)")?;
                    Ok(T::con("Bool"))
                }
                "++" => {
                    c.unify(&ta, &T::con("Str"), a.span, " (left of `++`)")?;
                    c.unify(&tb, &T::con("Str"), b.span, " (right of `++`)")?;
                    Ok(T::con("Str"))
                }
                _ => {
                    let n = c.fresh(Kind::Num);
                    c.unify(&ta, &n, a.span, &format!(" (left of `{}`)", op))?;
                    c.unify(&tb, &n, b.span, &format!(" (right of `{}`)", op))?;
                    Ok(n)
                }
            }
        }

        ExprKind::Neg(x) => {
            let t = infer(c, x, path)?;
            let n = c.fresh(Kind::Num);
            c.unify(&t, &n, x.span, " (negation)")?;
            Ok(n)
        }
        ExprKind::Not(x) => {
            let t = infer(c, x, path)?;
            c.unify(&t, &T::con("Bool"), x.span, " (`!` needs a Bool)")?;
            Ok(T::con("Bool"))
        }

        ExprKind::Tuple(xs) => {
            let mut ts = Vec::new();
            for x in xs {
                ts.push(infer(c, x, path)?);
            }
            Ok(T::Tuple(ts))
        }

        ExprKind::List(xs) => {
            let el = c.fresh(Kind::Any);
            for x in xs {
                let t = infer(c, x, path)?;
                c.unify(&t, &el, x.span, " (list elements must share one type)")?;
            }
            Ok(T::Con("Vec".into(), vec![el]))
        }

        ExprKind::Lambda(ps, body) => {
            c.push_scope();
            let mut pts = Vec::new();
            for p in ps {
                let t = c.fresh(Kind::Any);
                pts.push(t.clone());
                c.define(p, Scheme::mono(t));
            }
            let bt = infer(c, body, path)?;
            c.pop_scope();
            let mut ty = bt;
            for p in pts.into_iter().rev() {
                ty = T::Fun(Box::new(p), Box::new(ty));
            }
            Ok(ty)
        }

        ExprKind::Let(n, val, body) => {
            let vt = infer(c, val, path)?;
            c.push_scope();
            c.bound(body.span, n, &vt);
            c.define(n, Scheme::mono(vt));
            let bt = infer(c, body, path)?;
            c.pop_scope();
            Ok(bt)
        }

        ExprKind::Bind(n, val, body) => {
            let vt = infer(c, val, path)?;
            let inner = match effect_of(&c.resolve(&vt)) {
                Some(t) => t,
                None => {
                    let r = c.resolve(&vt);
                    if let T::Var(_) = r {
                        let fresh = c.fresh(Kind::Any);
                        c.unify(&vt, &T::Eff(Box::new(fresh.clone())), val.span, "")?;
                        fresh
                    } else {
                        return Err(Diag::error(
                            val.span,
                            "effect.pure_bind",
                            &format!("`<-` needs an effectful value, but this is a pure `{}`", r.show()),
                        )
                        .with_fix(&format!("use `let {} = ... in` instead", n))
                        .at_path(path));
                    }
                }
            };
            c.push_scope();
            c.bound(body.span, n, &inner);
            c.define(n, Scheme::mono(inner));
            let bt = infer(c, body, path)?;
            c.pop_scope();
            Ok(match effect_of(&c.resolve(&bt)) {
                Some(_) => bt,
                None => T::Eff(Box::new(bt)),
            })
        }

        ExprKind::Match(scrut, arms) => {
            let st = infer(c, scrut, path)?;
            let res = c.fresh(Kind::Any);
            let mut effectful = false;
            for (p, body) in arms {
                c.push_scope();
                bind_pattern(c, p, &st, e.span, path)?;
                record_pattern(c, p, body.span);
                let bt = infer(c, body, path)?;
                let bt_r = c.resolve(&bt);
                match effect_of(&bt_r) {
                    Some(inner) => {
                        effectful = true;
                        c.unify(&inner, &res, body.span, " (all match arms must agree)")?;
                    }
                    None => {
                        c.unify(&bt, &res, body.span, " (all match arms must agree)")?;
                    }
                }
                c.pop_scope();
            }
            c.record_match(e.span, st, arms.iter().map(|(p, _)| p.clone()).collect(), path.to_string());
            Ok(if effectful { T::Eff(Box::new(res)) } else { res })
        }

        ExprKind::Field(base, f) => {
            let bt = infer(c, base, path)?;
            let owner = match c.resolve(&bt) {
                T::Con(n, _) if c.data.records.contains_key(&n) => n,
                T::Var(_) => match c.data.field_owner.get(f) {
                    Some(o) => {
                        let o = o.clone();
                        c.unify(&bt, &T::Con(o.clone(), vec![]), base.span, "")?;
                        o
                    }
                    None => {
                        let mut d = Diag::error(
                            e.span,
                            "field.unknown",
                            &format!("no record type has a field `{}`", f),
                        );
                        if let Some(s) = suggest(f, c.data.field_owner.keys()) {
                            d = d.with_fix(&format!("did you mean `.{}`?", s));
                        }
                        return Err(d.at_path(path));
                    }
                },
                other => {
                    return Err(Diag::error(
                        e.span,
                        "field.not_record",
                        &format!("`{}` is not a record, so it has no field `{}`", other.show(), f),
                    )
                    .at_path(path))
                }
            };
            let rec = c.data.records[&owner].clone();
            match rec.fields.iter().find(|(n, _)| n == f) {
                Some((_, ty)) => {
                    let mut vars = HashMap::new();
                    Ok(c.lower_ty(ty, &mut vars))
                }
                None => {
                    let mut d = Diag::error(
                        e.span,
                        "field.unknown",
                        &format!("`{}` has no field `{}`", owner, f),
                    );
                    if let Some(s) = suggest(f, rec.fields.iter().map(|(n, _)| n)) {
                        d = d.with_fix(&format!("did you mean `.{}`?", s));
                    }
                    Err(d.at_path(path))
                }
            }
        }

        ExprKind::Record(base, fields) => {
            let given: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
            let owner = match base {
                Some(b) => {
                    let bt = infer(c, b, path)?;
                    match c.resolve(&bt) {
                        T::Con(n, _) if c.data.records.contains_key(&n) => n,
                        T::Var(_) => match given.first().and_then(|f| c.data.field_owner.get(f)) {
                            Some(o) => o.clone(),
                            None => {
                                return Err(Diag::error(e.span, "record.unknown", "cannot tell which record type this update builds")
                                    .at_path(path))
                            }
                        },
                        other => {
                            return Err(Diag::error(
                                e.span,
                                "record.not_record",
                                &format!("`{}` is not a record, so `with` does not apply", other.show()),
                            )
                            .at_path(path))
                        }
                    }
                }
                None => {
                    let mut found = None;
                    for (name, r) in &c.data.records {
                        let mut want: Vec<String> = r.fields.iter().map(|(n, _)| n.clone()).collect();
                        let mut have = given.clone();
                        want.sort();
                        have.sort();
                        if want == have {
                            found = Some(name.clone());
                            break;
                        }
                    }
                    match found {
                        Some(n) => n,
                        None => {
                            let hint = given
                                .first()
                                .and_then(|f| c.data.field_owner.get(f))
                                .map(|o| {
                                    let r = &c.data.records[o];
                                    let want: Vec<&str> =
                                        r.fields.iter().map(|(n, _)| n.as_str()).collect();
                                    format!(
                                        "`{}` has fields {}; this literal gives {}",
                                        o,
                                        want.join(", "),
                                        given.join(", ")
                                    )
                                })
                                .unwrap_or_else(|| format!("fields given: {}", given.join(", ")));
                            return Err(Diag::error(
                                e.span,
                                "record.nomatch",
                                "no record type has exactly these fields",
                            )
                            .with_witness(&hint)
                            .at_path(path));
                        }
                    }
                }
            };
            let rec = c.data.records[&owner].clone();
            for (fname, fexpr) in fields {
                match rec.fields.iter().find(|(n, _)| n == fname) {
                    Some((_, fty)) => {
                        let mut vars = HashMap::new();
                        let want = c.lower_ty(fty, &mut vars);
                        let got = infer(c, fexpr, path)?;
                        c.unify(&got, &want, fexpr.span, &format!(" (field `{}`)", fname))?;
                    }
                    None => {
                        return Err(Diag::error(
                            fexpr.span,
                            "field.unknown",
                            &format!("`{}` has no field `{}`", owner, fname),
                        )
                        .at_path(path))
                    }
                }
            }
            Ok(T::Con(owner, vec![]))
        }
    }
}

fn bind_pattern(c: &mut Checker, p: &Pat, expected: &T, span: Span, path: &str) -> R<()> {
    match p {
        Pat::Wild => Ok(()),
        Pat::Var(n) => {
            c.define(n, Scheme::mono(expected.clone()));
            Ok(())
        }
        Pat::Int(_) => {
            let n = c.fresh(Kind::Num);
            c.unify(expected, &n, span, " (integer pattern)")
        }
        Pat::Float(_) => {
            let n = c.fresh(Kind::Float);
            c.unify(expected, &n, span, " (decimal pattern)")
        }
        Pat::Str(_) => c.unify(expected, &T::con("Str"), span, " (string pattern)"),
        Pat::Char(_) => c.unify(expected, &T::con("Char"), span, " (character pattern)"),
        Pat::Bool(_) => c.unify(expected, &T::con("Bool"), span, " (boolean pattern)"),
        Pat::Tuple(ps) => {
            let mut ts = Vec::new();
            for _ in ps {
                ts.push(c.fresh(Kind::Any));
            }
            c.unify(expected, &T::Tuple(ts.clone()), span, " (tuple pattern)")?;
            for (sp, st) in ps.iter().zip(ts.iter()) {
                bind_pattern(c, sp, st, span, path)?;
            }
            Ok(())
        }
        Pat::List(ps) => {
            let el = c.fresh(Kind::Any);
            c.unify(expected, &T::Con("Vec".into(), vec![el.clone()]), span, " (list pattern)")?;
            for sp in ps {
                bind_pattern(c, sp, &el, span, path)?;
            }
            Ok(())
        }
        Pat::Ctor(name, args) => {
            if name == "True" || name == "False" {
                return c.unify(expected, &T::con("Bool"), span, " (boolean pattern)");
            }
            let info = match c.data.ctors.get(name) {
                Some(i) => i.clone(),
                None => {
                    let names: Vec<String> = c.data.ctors.keys().cloned().collect();
                    let mut d = Diag::error(
                        span,
                        "name.unbound",
                        &format!("constructor `{}` is not defined", name),
                    );
                    if let Some(s) = suggest(name, names.iter()) {
                        d = d.with_fix(&format!("did you mean `{}`?", s));
                    }
                    return Err(d.at_path(path));
                }
            };
            if args.len() != info.args.len() {
                return Err(Diag::error(
                    span,
                    "pattern.arity",
                    &format!(
                        "`{}` carries {} value{}, but the pattern binds {}",
                        name,
                        info.args.len(),
                        if info.args.len() == 1 { "" } else { "s" },
                        args.len()
                    ),
                )
                .with_fix(&format!(
                    "write `|{}{} -> ...`",
                    name,
                    (0..info.args.len()).map(|i| format!(" x{}", i)).collect::<String>()
                ))
                .at_path(path));
            }
            let mut vars: HashMap<String, T> = HashMap::new();
            let owner_args: Vec<T> =
                info.params.iter().map(|q| c.lower_ty(&Ty::Var(q.clone()), &mut vars)).collect();
            c.unify(expected, &T::Con(info.owner.clone(), owner_args), span, " (constructor pattern)")?;
            for (sp, aty) in args.iter().zip(info.args.iter()) {
                let at = c.lower_ty(aty, &mut vars);
                bind_pattern(c, sp, &at, span, path)?;
            }
            Ok(())
        }
    }
}

/// After `bind_pattern` has typed a pattern's names, hand them to the
/// ownership pass under the span of the arm they scope over — the only stable
/// identity a pattern binder has, since `Pat` carries no spans.
fn record_pattern(c: &mut Checker, p: &Pat, scope: Span) {
    for n in pat_names(p) {
        if let Some(s) = c.lookup(&n) {
            c.bound(scope, &n, &s.ty.clone());
        }
    }
}

fn pat_names(p: &Pat) -> Vec<String> {
    match p {
        Pat::Var(n) => vec![n.clone()],
        Pat::Ctor(_, ps) | Pat::List(ps) | Pat::Tuple(ps) => ps.iter().flat_map(pat_names).collect(),
        _ => Vec::new(),
    }
}

/// The inferred-type twin of `own::is_affine`: scalars are copied, everything
/// with a payload is moved. An unresolved variable is treated as affine, so an
/// unknown is reported rather than waved through.
fn is_affine_t(t: &T) -> bool {
    match t {
        T::Con(n, _) => !(crate::types::is_num(n)
            || matches!(n.as_str(), "Bool" | "Char" | "Unit" | "Size" | "CStr" | "Ptr")),
        T::Var(_) => true,
        T::Tuple(ts) => ts.iter().any(is_affine_t),
        T::Fun(..) | T::Eff(_) => false,
    }
}

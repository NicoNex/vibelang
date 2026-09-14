//! C emission (spec §11). Portable C99: no statement expressions, no VLAs, no
//! GNU extensions, so gcc / clang / MSVC all accept the output.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::infer::Checked;
use crate::types::{normalise_type_name, Scheme, T};
use std::collections::{HashMap, HashSet};

pub struct Gen<'a> {
    m: &'a Module,
    ck: &'a Checked,
    file: String,
    /// Arity of every top-level function, by name.
    arity: HashMap<String, usize>,
    /// Record type -> field order; used for `.field` and record literals.
    field_index: HashMap<String, usize>,
    scopes: Vec<HashMap<String, String>>,
    tmp: usize,
    lifted: String,
    /// Builtins / constructors / ext functions used as first-class values.
    need_wrapper: HashSet<String>,
    errors: Vec<Diag>,
    /// Name of the function being emitted, for self-tail-call detection.
    cur_fn: String,
    cur_params: Vec<String>,
    cur_path: String,
}

type G<X> = Result<X, Diag>;

/// name -> (arity, C call template). `$0`..`$n` are the arguments, `$P` the
/// semantic path used in run-time obligation messages.
fn builtin(name: &str) -> Option<(usize, String)> {
    let t = |n: usize, s: &str| Some((n, s.to_string()));
    match name {
        "empty" => t(0, "vb_vec_new()"),
        "single" => t(1, "vb_single($0)"),
        "push" => t(2, "vb_push($0, $1)"),
        "get" => t(2, "vb_get($0, $1, $P)"),
        "set" => t(3, "vb_set($0, $1, $2, $P)"),
        "map" => t(2, "vb_map($0, $1)"),
        "filter" => t(2, "vb_filter($0, $1)"),
        "fold" => t(3, "vb_fold($0, $1, $2)"),
        "each" => t(2, "vb_each($0, $1)"),
        "sum" => t(1, "vb_sum($0)"),
        "max_by" => t(2, "vb_max_by($0, $1, $P)"),
        "min_by" => t(2, "vb_min_by($0, $1, $P)"),
        "sort_by" => t(2, "vb_sort_by($0, $1)"),
        "rev" => t(1, "vb_rev($0)"),
        "concat_vec" => t(2, "vb_concat_vec($0, $1)"),
        "seq" => t(1, "vb_seq($0)"),
        "range" => t(2, "vb_range($0, $1)"),
        "split" => t(2, "vb_split($0, $1)"),
        "lines" => t(1, "vb_lines($0)"),
        "dup" => t(1, "vb_dup($0)"),
        "concat" => t(2, "vb_concat($0, $1)"),
        "trim" => t(1, "vb_trim($0)"),
        "starts_with" => t(2, "vb_starts_with($0, $1)"),
        "contains" => t(2, "vb_contains($0, $1)"),
        "to_cstr" => t(1, "vb_to_cstr($0)"),
        "from_cstr" => t(1, "vb_from_cstr($0)"),
        "chr" => t(1, "vb_chr($0)"),
        "f32" => t(1, "vb_to_f32($0)"),
        "f64" => t(1, "vb_to_f64($0)"),
        "i8" => t(1, "vb_to_signed($0, 8)"),
        "i16" => t(1, "vb_to_signed($0, 16)"),
        "i32" => t(1, "vb_to_signed($0, 32)"),
        "i64" => t(1, "vb_to_signed($0, 64)"),
        "u8" => t(1, "vb_to_unsigned($0, 8)"),
        "u16" => t(1, "vb_to_unsigned($0, 16)"),
        "u32" => t(1, "vb_to_unsigned($0, 32)"),
        "u64" | "size" => t(1, "vb_to_unsigned($0, 64)"),
        "parse_i64" => t(1, "vb_parse_int($0, 1)"),
        "parse_u32" | "parse_u64" => t(1, "vb_parse_int($0, 0)"),
        "parse_f64" => t(1, "vb_parse_f64($0)"),
        "abs" => t(1, "vb_abs($0)"),
        "min" => t(2, "vb_min($0, $1)"),
        "max" => t(2, "vb_max($0, $1)"),
        "sqrt" => t(1, "vb_sqrt($0)"),
        "pow" => t(2, "vb_pow($0, $1)"),
        "floor" => t(1, "vb_floor($0)"),
        "read" => t(1, "vb_read($0)"),
        "write" => t(2, "vb_write($0, $1)"),
        "out" => t(1, "vb_out($0)"),
        "warn" => t(1, "vb_warn($0)"),
        "argv" => t(0, "vb_argv()"),
        "exit" => t(1, "vb_exit($0)"),
        "add_checked" => t(2, "vb_add_checked($0, $1)"),
        "sub_checked" => t(2, "vb_sub_checked($0, $1)"),
        "mul_checked" => t(2, "vb_mul_checked($0, $1)"),
        "div_checked" => t(2, "vb_div_checked($0, $1)"),
        "get_checked" => t(2, "vb_get_checked($0, $1)"),
        "len" => t(1, "vb_len($0)"),
        "show" => t(1, "vb_show($0)"),
        _ => None,
    }
}

fn cname(n: &str) -> String {
    n.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

fn cstring(s: &str) -> String {
    let mut o = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => o.push_str("\\\""),
            b'\\' => o.push_str("\\\\"),
            b'\n' => o.push_str("\\n"),
            b'\t' => o.push_str("\\t"),
            b'\r' => o.push_str("\\r"),
            0x20..=0x7e => o.push(b as char),
            _ => o.push_str(&format!("\\x{:02x}\"\"", b)),
        }
    }
    o.push('"');
    o
}

pub fn generate(m: &Module, ck: &Checked, file: &str) -> Result<String, Vec<Diag>> {
    let mut arity = HashMap::new();
    for f in m.funs() {
        arity.insert(f.name.clone(), f.params.iter().map(|p| p.names.len()).sum());
    }
    let mut g = Gen {
        m,
        ck,
        file: file.to_string(),
        arity,
        field_index: HashMap::new(),
        scopes: vec![HashMap::new()],
        tmp: 0,
        lifted: String::new(),
        need_wrapper: HashSet::new(),
        errors: Vec::new(),
        cur_fn: String::new(),
        cur_params: Vec::new(),
        cur_path: String::new(),
    };
    for (rn, r) in &ck.data.records {
        for (i, (fname, _)) in r.fields.iter().enumerate() {
            g.field_index.insert(format!("{}#{}", rn, fname), i);
        }
    }
    let out = g.module();
    if g.errors.is_empty() {
        Ok(out)
    } else {
        Err(g.errors)
    }
}

impl<'a> Gen<'a> {
    fn fresh(&mut self) -> String {
        self.tmp += 1;
        format!("t{}", self.tmp)
    }
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    fn bind(&mut self, n: &str) -> String {
        self.tmp += 1;
        let c = format!("v_{}_{}", cname(n), self.tmp);
        self.scopes.last_mut().unwrap().insert(n.to_string(), c.clone());
        c
    }
    fn lookup(&self, n: &str) -> Option<String> {
        self.scopes.iter().rev().find_map(|s| s.get(n).cloned())
    }
    fn line(&self, span: Span) -> String {
        format!("#line {} {}\n", span.line, cstring(&self.file))
    }

    // ------------------------------------------------------------- module

    fn module(&mut self) -> String {
        let mut o = String::new();
        o.push_str("/* generated by vibec — do not edit; edit the .vibe source */\n");
        o.push_str("#include \"vibert.h\"\n");
        for e in self.m.exts() {
            let h = &e.header;
            if h.starts_with('.') || h.starts_with('/') {
                o.push_str(&format!("#include {}\n", cstring(h)));
            } else {
                o.push_str(&format!("#include <{}>\n", h));
            }
        }
        o.push_str("\nstatic const VbInfo vb_info_tuple = {\"tuple\", 0, 0};\n");

        // Static descriptors for records and variants.
        for (rn, r) in &self.ck.data.records {
            let names: Vec<String> = r.fields.iter().map(|(n, _)| cstring(n)).collect();
            o.push_str(&format!(
                "static const char *const vbfl_{}[] = {{{}}};\n",
                cname(rn),
                if names.is_empty() { "0".to_string() } else { names.join(", ") }
            ));
            o.push_str(&format!(
                "static const VbInfo vbi_{} = {{{}, {}, vbfl_{}}};\n",
                cname(rn),
                cstring(rn),
                r.fields.len(),
                cname(rn)
            ));
        }
        for (cn, ci) in &self.ck.data.ctors {
            o.push_str(&format!(
                "static const VbInfo vbi_{} = {{{}, {}, 0}};\n",
                cname(cn),
                cstring(cn),
                ci.args.len()
            ));
        }

        // Forward declarations: order of definition never matters.
        o.push('\n');
        for f in self.m.funs().filter(|f| !f.ghost) {
            o.push_str(&format!("static VbVal vbf_{}(VbVal *a);\n", cname(&f.name)));
        }
        o.push('\n');

        let mut bodies = String::new();
        for f in self.m.funs() {
            if f.ghost {
                // Ghost functions exist only for proofs (spec §7.4): never emitted.
                continue;
            }
            bodies.push_str(&self.function(f));
        }

        let wrappers = self.wrappers();
        o.push_str(&wrappers);
        o.push_str(&std::mem::take(&mut self.lifted));
        o.push_str(&bodies);
        o.push_str(&self.exports());
        if self.arity.contains_key("main") {
            o.push_str(
                "\nint main(int argc, char **argv) {\n  vb_init();\n  vb_set_args(argc, argv);\n  vbf_main(0);\n  return 0;\n}\n",
            );
        }
        o
    }

    fn wrappers(&mut self) -> String {
        // Two passes: emitting bodies discovers which wrappers are needed, so the
        // caller emits this block after the bodies are built but places it first.
        let mut o = String::new();
        let names: Vec<String> = {
            let mut v: Vec<String> = self.need_wrapper.iter().cloned().collect();
            v.sort();
            v
        };
        for n in names {
            if let Some((ar, tpl)) = builtin(&n) {
                let args: Vec<String> = (0..ar).map(|i| format!("a[{}]", i)).collect();
                let mut body = tpl.clone();
                for (i, a) in args.iter().enumerate() {
                    body = body.replace(&format!("${}", i), a);
                }
                body = body.replace("$P", &cstring(&format!("Prelude.{}", n)));
                o.push_str(&format!(
                    "static VbVal vbw_{}(VbVal *a) {{ (void)a; return {}; }}\n",
                    cname(&n),
                    body
                ));
            } else if let Some(ci) = self.ck.data.ctors.get(&n) {
                let args: Vec<String> = (0..ci.args.len()).map(|i| format!("a[{}]", i)).collect();
                o.push_str(&format!(
                    "static VbVal vbw_{}(VbVal *a) {{ (void)a; return vb_obj(&vbi_{}, {}, {}{}{}); }}\n",
                    cname(&n),
                    cname(&n),
                    ci.tag,
                    ci.args.len(),
                    if args.is_empty() { "" } else { ", " },
                    args.join(", ")
                ));
            }
        }
        if !o.is_empty() {
            o.push('\n');
        }
        o
    }

    fn function(&mut self, f: &FunDecl) -> String {
        let path = format!("{}.{}", self.m.name, f.name);
        self.cur_fn = f.name.clone();
        self.cur_path = path.clone();
        self.push_scope();
        let mut params = Vec::new();
        let mut head = String::new();
        let mut i = 0;
        for p in &f.params {
            for n in &p.names {
                let c = self.bind(n);
                head.push_str(&format!("  VbVal {} = a[{}];\n", c, i));
                params.push(c);
                i += 1;
            }
        }
        self.cur_params = params.clone();

        // Refinements are run-time obligations in the bootstrap (spec fase 6
        // replaces them with SMT discharge).
        let mut pre = String::new();
        for p in &f.params {
            for r in &p.refines {
                let mut s = String::new();
                let v = self.ex(r, &mut s);
                pre.push_str(&s);
                pre.push_str(&format!(
                    "  vb_require(vb_as_bool({}), {}, {});\n",
                    v,
                    cstring(&format!("{}.pre", path)),
                    cstring(&expr_text(r))
                ));
            }
        }

        let mut body = String::new();
        self.tail(&f.body, &mut body);
        self.pop_scope();

        let uses_params = if params.is_empty() { String::new() } else { String::new() };
        format!(
            "{}static VbVal vbf_{}(VbVal *a) {{\n  (void)a;\n{}{}{}  VbVal vbret = vb_unit();\n  for (;;) {{\n{}  }}\n  return vbret;\n}}\n\n",
            self.line(f.span),
            cname(&f.name),
            head,
            uses_params,
            pre,
            indent(&body, 4)
        )
    }

    fn exports(&mut self) -> String {
        let names = self.m.exports();
        if names.is_empty() {
            return String::new();
        }
        let mut o = String::from("\n/* exp c — C ABI surface (spec §10.2) */\n");
        for n in &names {
            let ar = *self.arity.get(n).unwrap_or(&0);
            let sig = self.ck.sigs.get(n).cloned().unwrap_or(Scheme::mono(T::unit()));
            let (ps, ret) = split_fn(&sig.ty, ar);
            let cargs: Vec<String> =
                ps.iter().enumerate().map(|(i, t)| format!("{} x{}", c_type(t), i)).collect();
            let boxed: Vec<String> = ps.iter().enumerate().map(|(i, t)| box_expr(t, &format!("x{}", i))).collect();
            o.push_str(&format!(
                "{} {}_{}({}) {{\n  vb_init();\n  VbVal a[{}];\n{}  VbVal r = vbf_{}(a);\n  {}\n}}\n",
                c_type(&ret),
                cname(&self.m.name),
                cname(n),
                if cargs.is_empty() { "void".to_string() } else { cargs.join(", ") },
                ar.max(1),
                boxed.iter().enumerate().map(|(i, b)| format!("  a[{}] = {};\n", i, b)).collect::<String>(),
                cname(n),
                unbox_return(&ret)
            ));
        }
        o
    }

    // --------------------------------------------------------- tail position

    /// Emit `e` in tail position: either assign `vbret` and break, or, for a
    /// saturated self-call, rebind the parameters and loop (spec §11.2).
    fn tail(&mut self, e: &Expr, out: &mut String) {
        match &e.kind {
            ExprKind::Let(n, val, body) | ExprKind::Bind(n, val, body) => {
                out.push_str(&self.line(e.span));
                let v = self.ex(val, out);
                self.push_scope();
                let c = self.bind(n);
                out.push_str(&format!("VbVal {} = {};\n", c, v));
                self.tail(body, out);
                self.pop_scope();
            }
            ExprKind::Match(scrut, arms) => {
                out.push_str(&self.line(e.span));
                let s = self.ex(scrut, out);
                self.match_chain(e, &s, arms, out, None);
            }
            ExprKind::App(head, args) => {
                if let ExprKind::Var(n) = &head.kind {
                    if n == &self.cur_fn
                        && self.lookup(n).is_none()
                        && self.arity.get(n) == Some(&args.len())
                    {
                        let mut vals = Vec::new();
                        for a in args {
                            vals.push(self.ex(a, out));
                        }
                        let tmps: Vec<String> = (0..vals.len()).map(|_| self.fresh()).collect();
                        for (t, v) in tmps.iter().zip(vals.iter()) {
                            out.push_str(&format!("VbVal {} = {};\n", t, v));
                        }
                        for (p, t) in self.cur_params.clone().iter().zip(tmps.iter()) {
                            out.push_str(&format!("{} = {};\n", p, t));
                        }
                        out.push_str("continue;\n");
                        return;
                    }
                }
                let v = self.ex(e, out);
                out.push_str(&format!("vbret = {};\nbreak;\n", v));
            }
            _ => {
                let v = self.ex(e, out);
                out.push_str(&format!("vbret = {};\nbreak;\n", v));
            }
        }
    }

    /// Shared by tail and value position. `dest` is `None` in tail position.
    fn match_chain(
        &mut self,
        e: &Expr,
        scrut: &str,
        arms: &[(Pat, Expr)],
        out: &mut String,
        dest: Option<&str>,
    ) {
        let sv = self.fresh();
        out.push_str(&format!("VbVal {} = {};\n", sv, scrut));
        for (i, (p, body)) in arms.iter().enumerate() {
            let cond = self.pat_cond(p, &sv);
            out.push_str(&format!("{}if ({}) {{\n", if i == 0 { "" } else { "else " }, cond));
            let mut inner = String::new();
            self.push_scope();
            self.pat_bind(p, &sv, &mut inner);
            match dest {
                Some(d) => {
                    let v = self.ex(body, &mut inner);
                    inner.push_str(&format!("{} = {};\n", d, v));
                }
                None => self.tail(body, &mut inner),
            }
            self.pop_scope();
            out.push_str(&indent(&inner, 2));
            out.push_str("}\n");
        }
        out.push_str(&format!(
            "else {{ vb_fail({}, \"no match arm applied\"); }}\n",
            cstring(&format!("{}.match", self.cur_path))
        ));
        let _ = e;
    }

    // ------------------------------------------------------------- patterns

    fn pat_cond(&mut self, p: &Pat, s: &str) -> String {
        match p {
            Pat::Wild | Pat::Var(_) => "1".to_string(),
            Pat::Int(n) => format!("vb_eq({}, vb_int({}))", s, n),
            Pat::Float(x) => format!("vb_eq({}, vb_float({:?}))", s, x),
            Pat::Str(t) => format!("vb_eq({}, vb_strz({}))", s, cstring(t)),
            Pat::Char(c) => format!("vb_eq({}, vb_char({}))", s, *c as u32),
            Pat::Bool(b) => format!("vb_eq({}, vb_bool({}))", s, if *b { "true" } else { "false" }),
            Pat::Tuple(ps) => {
                let mut cs = vec!["1".to_string()];
                for (i, sp) in ps.iter().enumerate() {
                    cs.push(self.pat_cond(sp, &format!("vb_field({}, {})", s, i)));
                }
                cs.retain(|c| c != "1");
                if cs.is_empty() { "1".into() } else { cs.join(" && ") }
            }
            Pat::List(ps) => {
                let mut cs = vec![format!("vb_as_vec({})->n == {}", s, ps.len())];
                for (i, sp) in ps.iter().enumerate() {
                    let c = self.pat_cond(sp, &format!("vb_as_vec({})->a[{}]", s, i));
                    if c != "1" {
                        cs.push(c);
                    }
                }
                cs.join(" && ")
            }
            Pat::Ctor(n, args) => {
                if n == "True" || n == "False" {
                    return format!("vb_eq({}, vb_bool({}))", s, if n == "True" { "true" } else { "false" });
                }
                let tag = self.ck.data.ctors.get(n).map(|c| c.tag).unwrap_or(0);
                let mut cs = vec![format!("vb_tag({}) == {}", s, tag)];
                for (i, sp) in args.iter().enumerate() {
                    let c = self.pat_cond(sp, &format!("vb_field({}, {})", s, i));
                    if c != "1" {
                        cs.push(c);
                    }
                }
                cs.join(" && ")
            }
        }
    }

    fn pat_bind(&mut self, p: &Pat, s: &str, out: &mut String) {
        match p {
            Pat::Wild | Pat::Int(_) | Pat::Float(_) | Pat::Str(_) | Pat::Char(_) | Pat::Bool(_) => {}
            Pat::Var(n) => {
                let c = self.bind(n);
                out.push_str(&format!("VbVal {} = {};\n", c, s));
            }
            Pat::Tuple(ps) => {
                for (i, sp) in ps.iter().enumerate() {
                    self.pat_bind(sp, &format!("vb_field({}, {})", s, i), out);
                }
            }
            Pat::List(ps) => {
                for (i, sp) in ps.iter().enumerate() {
                    self.pat_bind(sp, &format!("vb_as_vec({})->a[{}]", s, i), out);
                }
            }
            Pat::Ctor(n, args) => {
                if n == "True" || n == "False" {
                    return;
                }
                for (i, sp) in args.iter().enumerate() {
                    self.pat_bind(sp, &format!("vb_field({}, {})", s, i), out);
                }
            }
        }
    }

    // ---------------------------------------------------------- expressions

    fn ex(&mut self, e: &Expr, out: &mut String) -> String {
        match &e.kind {
            ExprKind::Int(n) => format!("vb_int({})", n),
            ExprKind::Float(x) => format!("vb_float({:?})", x),
            ExprKind::Str(s) => format!("vb_strz({})", cstring(s)),
            ExprKind::Char(c) => format!("vb_char({})", *c as u32),
            ExprKind::Bool(b) => format!("vb_bool({})", if *b { "true" } else { "false" }),
            ExprKind::Unit => "vb_unit()".to_string(),
            ExprKind::Borrow(inner) => self.ex(inner, out),
            ExprKind::Neg(x) => {
                let v = self.ex(x, out);
                format!("vb_neg({})", v)
            }
            ExprKind::Not(x) => {
                let v = self.ex(x, out);
                format!("vb_bool(!vb_as_bool({}))", v)
            }
            ExprKind::Var(n) => self.value_ref(n, e.span, out),
            ExprKind::Ctor(n) => {
                if let Some(ci) = self.ck.data.ctors.get(n) {
                    if ci.args.is_empty() {
                        return format!("vb_obj(&vbi_{}, {}, 0)", cname(n), ci.tag);
                    }
                    let (tag, ar) = (ci.tag, ci.args.len());
                    self.need_wrapper.insert(n.clone());
                    let _ = tag;
                    return format!("vb_clos(vbw_{}, {}, {})", cname(n), cstring(n), ar);
                }
                self.errors.push(Diag::error(e.span, "codegen.ctor", &format!("unknown constructor `{}`", n)));
                "vb_unit()".into()
            }
            ExprKind::Tuple(xs) => {
                let vs: Vec<String> = xs.iter().map(|x| self.ex(x, out)).collect();
                format!("vb_obj(&vb_info_tuple, 0, {}, {})", vs.len(), vs.join(", "))
            }
            ExprKind::List(xs) => {
                if xs.is_empty() {
                    return "vb_vec_new()".into();
                }
                let vs: Vec<String> = xs.iter().map(|x| self.ex(x, out)).collect();
                format!("vb_vec_lit({}, {})", vs.len(), vs.join(", "))
            }
            ExprKind::Binop(op, a, b) => self.binop(op, a, b, e.span, out),
            ExprKind::Field(base, f) => {
                let b = self.ex(base, out);
                let owner = self.ck.data.field_owner.get(f).cloned();
                match owner.and_then(|o| self.field_index.get(&format!("{}#{}", o, f)).copied()) {
                    Some(i) => format!("vb_field({}, {})", b, i),
                    None => {
                        self.errors
                            .push(Diag::error(e.span, "codegen.field", &format!("unknown field `{}`", f)));
                        "vb_unit()".into()
                    }
                }
            }
            ExprKind::Record(base, fields) => self.record(base, fields, e.span, out),
            ExprKind::Let(n, val, body) | ExprKind::Bind(n, val, body) => {
                let v = self.ex(val, out);
                self.push_scope();
                let c = self.bind(n);
                out.push_str(&format!("VbVal {} = {};\n", c, v));
                let r = self.ex(body, out);
                self.pop_scope();
                r
            }
            ExprKind::Match(scrut, arms) => {
                let s = self.ex(scrut, out);
                let d = self.fresh();
                out.push_str(&format!("VbVal {} = vb_unit();\n", d));
                let dc = d.clone();
                self.match_chain(e, &s, arms, out, Some(&dc));
                d
            }
            ExprKind::Lambda(ps, body) => self.lambda(ps, body, e.span, out),
            ExprKind::App(head, args) => self.app(head, args, e.span, out),
        }
    }

    fn value_ref(&mut self, n: &str, span: Span, _out: &mut String) -> String {
        if let Some(c) = self.lookup(n) {
            return c;
        }
        if let Some(ar) = self.arity.get(n).copied() {
            return format!("vb_clos(vbf_{}, {}, {})", cname(n), cstring(n), ar.max(1));
        }
        if let Some(sig) = self.ck.ext.get(n) {
            let ar = fn_arity(&sig.ty);
            self.need_wrapper.insert(n.to_string());
            return format!("vb_clos(vbe_{}, {}, {})", cname(n), cstring(n), ar.max(1));
        }
        if let Some((ar, tpl)) = builtin(n) {
            if ar == 0 {
                let mut b = tpl;
                b = b.replace("$P", &cstring(&format!("Prelude.{}", n)));
                return b;
            }
            self.need_wrapper.insert(n.to_string());
            return format!("vb_clos(vbw_{}, {}, {})", cname(n), cstring(n), ar);
        }
        self.errors.push(Diag::error(span, "codegen.unbound", &format!("`{}` has no definition to emit", n)));
        "vb_unit()".into()
    }

    fn binop(&mut self, op: &str, a: &Expr, b: &Expr, span: Span, out: &mut String) -> String {
        if op == "&&" || op == "||" {
            let va = self.ex(a, out);
            let d = self.fresh();
            out.push_str(&format!("VbVal {} = vb_bool({});\n", d, if op == "&&" { "false" } else { "true" }));
            let keep = if op == "&&" { format!("vb_as_bool({})", va) } else { format!("!vb_as_bool({})", va) };
            out.push_str(&format!("if ({}) {{\n", keep));
            let mut inner = String::new();
            let vb = self.ex(b, &mut inner);
            inner.push_str(&format!("{} = {};\n", d, vb));
            out.push_str(&indent(&inner, 2));
            out.push_str("}\n");
            return d;
        }
        let va = self.ex(a, out);
        let vb = self.ex(b, out);
        match op {
            "+" => format!("vb_add({}, {})", va, vb),
            "-" => format!("vb_sub({}, {})", va, vb),
            "*" => format!("vb_mul({}, {})", va, vb),
            "/" => format!("vb_div({}, {}, {})", va, vb, cstring(&format!("{}.div", self.cur_path))),
            "++" => format!("vb_concat({}, {})", va, vb),
            "==" => format!("vb_bool(vb_eq({}, {}))", va, vb),
            "!=" => format!("vb_bool(!vb_eq({}, {}))", va, vb),
            "<" => format!("vb_bool(vb_cmp({}, {}) < 0)", va, vb),
            "<=" => format!("vb_bool(vb_cmp({}, {}) <= 0)", va, vb),
            ">" => format!("vb_bool(vb_cmp({}, {}) > 0)", va, vb),
            ">=" => format!("vb_bool(vb_cmp({}, {}) >= 0)", va, vb),
            _ => {
                self.errors.push(Diag::error(span, "codegen.binop", &format!("unknown operator `{}`", op)));
                "vb_unit()".into()
            }
        }
    }

    fn record(
        &mut self,
        base: &Option<Box<Expr>>,
        fields: &[(String, Expr)],
        span: Span,
        out: &mut String,
    ) -> String {
        let owner = match fields.first().and_then(|(f, _)| self.ck.data.field_owner.get(f)).cloned() {
            Some(o) => o,
            None => {
                self.errors.push(Diag::error(span, "codegen.record", "cannot resolve this record type"));
                return "vb_unit()".into();
            }
        };
        let rec = self.ck.data.records[&owner].clone();
        let d = self.fresh();
        match base {
            Some(b) => {
                let bv = self.ex(b, out);
                let mut idx = Vec::new();
                let mut vals = Vec::new();
                for (f, x) in fields {
                    let i = self.field_index[&format!("{}#{}", owner, f)];
                    idx.push(i.to_string());
                    vals.push(self.ex(x, out));
                }
                out.push_str(&format!(
                    "static const uint32_t {}_ix[] = {{{}}};\n",
                    d,
                    idx.join(", ")
                ));
                out.push_str(&format!("VbVal {}_vs[] = {{{}}};\n", d, vals.join(", ")));
                out.push_str(&format!(
                    "VbVal {} = vb_with({}, {}, {}_ix, {}_vs);\n",
                    d,
                    bv,
                    idx.len(),
                    d,
                    d
                ));
            }
            None => {
                let mut slots: Vec<String> = vec!["vb_unit()".into(); rec.fields.len()];
                for (f, x) in fields {
                    let v = self.ex(x, out);
                    let i = self.field_index[&format!("{}#{}", owner, f)];
                    slots[i] = v;
                }
                out.push_str(&format!(
                    "VbVal {} = vb_obj(&vbi_{}, 0, {}{}{});\n",
                    d,
                    cname(&owner),
                    rec.fields.len(),
                    if slots.is_empty() { "" } else { ", " },
                    slots.join(", ")
                ));
            }
        }
        // Record invariants are obligations at every construction (spec §7.2).
        if !rec.refines.is_empty() {
            let mut inner = String::new();
            self.push_scope();
            for (i, (fname, _)) in rec.fields.iter().enumerate() {
                let c = self.bind(fname);
                inner.push_str(&format!("VbVal {} = vb_field({}, {});\n", c, d, i));
            }
            for r in &rec.refines {
                let v = self.ex(r, &mut inner);
                inner.push_str(&format!(
                    "vb_require(vb_as_bool({}), {}, {});\n",
                    v,
                    cstring(&format!("{}.invariant", owner)),
                    cstring(&expr_text(r))
                ));
            }
            self.pop_scope();
            out.push_str("{\n");
            out.push_str(&indent(&inner, 2));
            out.push_str("}\n");
        }
        d
    }

    fn lambda(&mut self, ps: &[String], body: &Expr, span: Span, out: &mut String) -> String {
        let mut bound: HashSet<String> = ps.iter().cloned().collect();
        let mut free = Vec::new();
        free_vars(body, &mut bound, &mut free);
        let captured: Vec<(String, String)> =
            free.into_iter().filter_map(|n| self.lookup(&n).map(|c| (n, c))).collect();

        self.tmp += 1;
        let fname = format!("vbl_{}", self.tmp);
        let saved = std::mem::take(&mut self.scopes);
        self.scopes = vec![HashMap::new()];
        let mut head = String::new();
        for (i, (n, _)) in captured.iter().enumerate() {
            let c = self.bind(n);
            head.push_str(&format!("  VbVal {} = a[{}];\n", c, i));
        }
        for (i, p) in ps.iter().enumerate() {
            let c = self.bind(p);
            head.push_str(&format!("  VbVal {} = a[{}];\n", c, captured.len() + i));
        }
        let mut inner = String::new();
        let rv = self.ex(body, &mut inner);
        inner.push_str(&format!("return {};\n", rv));
        self.scopes = saved;

        let lifted = format!(
            "{}static VbVal {}(VbVal *a) {{\n  (void)a;\n{}{}}}\n\n",
            self.line(span),
            fname,
            head,
            indent(&inner, 2)
        );
        self.lifted.push_str(&lifted);

        let d = self.fresh();
        out.push_str(&format!(
            "VbVal {} = vb_clos({}, \"<lambda>\", {});\n",
            d,
            fname,
            captured.len() + ps.len()
        ));
        for (_, c) in &captured {
            out.push_str(&format!("{} = vb_apply1({}, {});\n", d, d, c));
        }
        d
    }

    fn app(&mut self, head: &Expr, args: &[Expr], span: Span, out: &mut String) -> String {
        if let ExprKind::Var(n) = &head.kind {
            if self.lookup(n).is_none() {
                // fmt is variadic by rule, not by signature.
                if n == "fmt" {
                    let vs: Vec<String> = args.iter().map(|a| self.ex(a, out)).collect();
                    let rest = &vs[1..];
                    return format!(
                        "vb_fmt({}, {}{}{})",
                        vs[0],
                        rest.len(),
                        if rest.is_empty() { "" } else { ", " },
                        rest.join(", ")
                    );
                }
                if let Some(ci) = self.ck.data.ctors.get(n).cloned() {
                    if args.len() == ci.args.len() {
                        let vs: Vec<String> = args.iter().map(|a| self.ex(a, out)).collect();
                        return format!(
                            "vb_obj(&vbi_{}, {}, {}{}{})",
                            cname(n),
                            ci.tag,
                            vs.len(),
                            if vs.is_empty() { "" } else { ", " },
                            vs.join(", ")
                        );
                    }
                }
                if let Some(sig) = self.ck.ext.get(n).cloned() {
                    if args.len() == fn_arity(&sig.ty) {
                        return self.ext_call(&sig, args, out);
                    }
                }
                if let Some(ar) = self.arity.get(n).copied() {
                    if args.len() == ar {
                        let vs: Vec<String> = args.iter().map(|a| self.ex(a, out)).collect();
                        let d = self.fresh();
                        out.push_str(&format!("VbVal {}_a[] = {{{}}};\n", d, vs.join(", ")));
                        out.push_str(&format!("VbVal {} = vbf_{}({}_a);\n", d, cname(n), d));
                        return d;
                    }
                }
                if let Some((ar, tpl)) = builtin(n) {
                    if args.len() == ar {
                        let vs: Vec<String> = args.iter().map(|a| self.ex(a, out)).collect();
                        let mut b = tpl;
                        for (i, v) in vs.iter().enumerate() {
                            b = b.replace(&format!("${}", i), v);
                        }
                        b = b.replace("$P", &cstring(&format!("{}.{}", self.cur_path, n)));
                        return b;
                    }
                }
            }
        }
        // Everything else: build the callee, then apply one argument at a time.
        let f = self.ex(head, out);
        let mut cur = f;
        for a in args {
            let v = self.ex(a, out);
            let d = self.fresh();
            out.push_str(&format!("VbVal {} = vb_apply1({}, {});\n", d, cur, v));
            cur = d;
        }
        let _ = span;
        cur
    }

    fn ext_call(&mut self, sig: &ExtSig, args: &[Expr], out: &mut String) -> String {
        let (ps, ret) = flatten_fn(&sig.ty);
        let mut cargs = Vec::new();
        for (a, t) in args.iter().zip(ps.iter()) {
            let v = self.ex(a, out);
            match c_unbox(t, &v) {
                Some(s) => cargs.push(s),
                None => {
                    self.errors.push(
                        Diag::error(
                            a.span,
                            "ffi.type",
                            &format!("`{}` cannot cross the C boundary", ty_show(t)),
                        )
                        .with_fix("use a scalar, Bool, Char, Str, CStr or `Ptr a` at the C boundary"),
                    );
                    cargs.push("0".into());
                }
            }
        }
        // Refinements on an `ext` are assumed past the boundary, checked here (spec §10.1).
        for r in &sig.refines {
            let v = self.ex(r, out);
            out.push_str(&format!(
                "vb_require(vb_as_bool({}), {}, {});\n",
                v,
                cstring(&format!("{}.pre", sig.name)),
                cstring(&expr_text(r))
            ));
        }
        let call = format!("{}({})", sig.symbol, cargs.join(", "));
        let d = self.fresh();
        let inner = strip_eff(&ret);
        if is_unit(&inner) {
            out.push_str(&format!("{};\n", call));
            out.push_str(&format!("VbVal {} = vb_unit();\n", d));
        } else {
            match c_box(&inner, &call) {
                Some(b) => out.push_str(&format!("VbVal {} = {};\n", d, b)),
                None => {
                    self.errors.push(
                        Diag::error(
                            sig.span,
                            "ffi.type",
                            &format!("`{}` cannot come back from C", ty_show(&inner)),
                        )
                        .with_fix("return a scalar, Bool, Char, CStr or `Ptr a`"),
                    );
                    out.push_str(&format!("VbVal {} = vb_unit();\n", d));
                }
            }
        }
        d
    }
}

// --------------------------------------------------------------- helpers

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines().map(|l| if l.is_empty() { String::from("\n") } else { format!("{}{}\n", pad, l) }).collect()
}

fn free_vars(e: &Expr, bound: &mut HashSet<String>, out: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Var(n) => {
            if !bound.contains(n) && !out.contains(n) {
                out.push(n.clone());
            }
        }
        ExprKind::App(h, args) => {
            free_vars(h, bound, out);
            args.iter().for_each(|a| free_vars(a, bound, out));
        }
        ExprKind::Binop(_, a, b) => {
            free_vars(a, bound, out);
            free_vars(b, bound, out);
        }
        ExprKind::Neg(x) | ExprKind::Not(x) | ExprKind::Borrow(x) | ExprKind::Field(x, _) => {
            free_vars(x, bound, out)
        }
        ExprKind::Tuple(xs) | ExprKind::List(xs) => xs.iter().for_each(|x| free_vars(x, bound, out)),
        ExprKind::Record(b, fs) => {
            if let Some(b) = b {
                free_vars(b, bound, out);
            }
            fs.iter().for_each(|(_, x)| free_vars(x, bound, out));
        }
        ExprKind::Lambda(ps, body) => {
            let added: Vec<String> = ps.iter().filter(|p| bound.insert((*p).clone())).cloned().collect();
            free_vars(body, bound, out);
            for a in added {
                bound.remove(&a);
            }
        }
        ExprKind::Let(n, v, b) | ExprKind::Bind(n, v, b) => {
            free_vars(v, bound, out);
            let added = bound.insert(n.clone());
            free_vars(b, bound, out);
            if added {
                bound.remove(n);
            }
        }
        ExprKind::Match(s, arms) => {
            free_vars(s, bound, out);
            for (p, body) in arms {
                let mut added = Vec::new();
                pat_vars(p, &mut added);
                let fresh: Vec<String> = added.into_iter().filter(|a| bound.insert(a.clone())).collect();
                free_vars(body, bound, out);
                for a in fresh {
                    bound.remove(&a);
                }
            }
        }
        _ => {}
    }
}

fn pat_vars(p: &Pat, out: &mut Vec<String>) {
    match p {
        Pat::Var(n) => out.push(n.clone()),
        Pat::Ctor(_, ps) | Pat::List(ps) | Pat::Tuple(ps) => ps.iter().for_each(|x| pat_vars(x, out)),
        _ => {}
    }
}

/// A best-effort source rendering of a refinement, for run-time messages.
fn expr_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(n) => n.to_string(),
        ExprKind::Float(x) => format!("{}", x),
        ExprKind::Str(s) => format!("{:?}", s),
        ExprKind::Char(c) => format!("'{}'", c),
        ExprKind::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        ExprKind::Unit => "()".into(),
        ExprKind::Var(n) => n.clone(),
        ExprKind::Ctor(n) => n.clone(),
        ExprKind::Borrow(x) => format!("&{}", expr_text(x)),
        ExprKind::Neg(x) => format!("-{}", expr_text(x)),
        ExprKind::Not(x) => format!("!{}", expr_text(x)),
        ExprKind::Field(x, f) => format!("{}.{}", expr_text(x), f),
        ExprKind::Binop(o, a, b) => format!("{} {} {}", expr_text(a), o, expr_text(b)),
        ExprKind::App(h, args) => {
            let mut s = expr_text(h);
            for a in args {
                s.push(' ');
                s.push_str(&expr_text(a));
            }
            s
        }
        _ => "<expr>".into(),
    }
}

fn fn_arity(t: &Ty) -> usize {
    match t {
        Ty::Fun(_, r) => 1 + fn_arity(r),
        _ => 0,
    }
}

fn flatten_fn(t: &Ty) -> (Vec<Ty>, Ty) {
    match t {
        Ty::Fun(a, r) => {
            let (mut ps, ret) = flatten_fn(r);
            ps.insert(0, (**a).clone());
            (ps, ret)
        }
        other => (vec![], other.clone()),
    }
}

fn strip_eff(t: &Ty) -> Ty {
    match t {
        Ty::Eff(x) => strip_eff(x),
        Ty::Ref(x) => strip_eff(x),
        other => other.clone(),
    }
}

fn is_unit(t: &Ty) -> bool {
    matches!(t, Ty::Con(n, _) if n == "Unit")
}

fn ty_show(t: &Ty) -> String {
    match t {
        Ty::Con(n, a) if a.is_empty() => n.clone(),
        Ty::Con(n, a) => format!("{} {}", n, a.iter().map(ty_show).collect::<Vec<_>>().join(" ")),
        Ty::Var(n) => n.clone(),
        Ty::Ref(x) => format!("&{}", ty_show(x)),
        Ty::Eff(x) => format!("E! {}", ty_show(x)),
        Ty::Fun(a, b) => format!("{} -> {}", ty_show(a), ty_show(b)),
        Ty::Tuple(ts) => format!("({})", ts.iter().map(ty_show).collect::<Vec<_>>().join(", ")),
    }
}

/// VbVal -> C, for arguments handed to a C function.
fn c_unbox(t: &Ty, v: &str) -> Option<String> {
    let t = strip_eff(t);
    match &t {
        Ty::Con(n, args) => {
            let n = normalise_type_name(n);
            Some(match n.as_str() {
                "U8" => format!("(uint8_t)vb_as_uint({})", v),
                "U16" => format!("(uint16_t)vb_as_uint({})", v),
                "U32" => format!("(uint32_t)vb_as_uint({})", v),
                "U64" => format!("(uint64_t)vb_as_uint({})", v),
                "I8" => format!("(int8_t)vb_as_int({})", v),
                "I16" => format!("(int16_t)vb_as_int({})", v),
                "I32" => format!("(int32_t)vb_as_int({})", v),
                "I64" => format!("(int64_t)vb_as_int({})", v),
                "F32" => format!("(float)vb_as_float({})", v),
                "F64" => format!("(double)vb_as_float({})", v),
                "Bool" => format!("vb_as_bool({})", v),
                "Char" => format!("(char)vb_as_int({})", v),
                "CStr" | "Str" => format!("vb_as_cstr({})", v),
                "Ptr" => {
                    let _ = args;
                    format!("vb_as_ptr({})", v)
                }
                "Unit" => return None,
                _ => return None,
            })
        }
        _ => None,
    }
}

/// C -> VbVal, for values coming back from C.
fn c_box(t: &Ty, expr: &str) -> Option<String> {
    let t = strip_eff(t);
    match &t {
        Ty::Con(n, _) => {
            let n = normalise_type_name(n);
            Some(match n.as_str() {
                "U8" | "U16" | "U32" | "U64" => format!("vb_uint((uint64_t)({}))", expr),
                "I8" | "I16" | "I32" | "I64" => format!("vb_int((int64_t)({}))", expr),
                "F32" | "F64" => format!("vb_float((double)({}))", expr),
                "Bool" => format!("vb_bool(({}) ? true : false)", expr),
                "Char" => format!("vb_char((uint32_t)({}))", expr),
                "CStr" => format!("vb_cstr_val((const char *)({}))", expr),
                "Str" => format!("vb_strz((const char *)({}))", expr),
                "Ptr" => format!("vb_ptr((void *)({}))", expr),
                _ => return None,
            })
        }
        _ => None,
    }
}

// Exported C ABI: scalars are unboxed, everything else stays a VbVal.

fn c_type(t: &T) -> String {
    match t {
        T::Con(n, _) => match n.as_str() {
            "U8" => "uint8_t".into(),
            "U16" => "uint16_t".into(),
            "U32" => "uint32_t".into(),
            "U64" => "uint64_t".into(),
            "I8" => "int8_t".into(),
            "I16" => "int16_t".into(),
            "I32" => "int32_t".into(),
            "I64" => "int64_t".into(),
            "F32" => "float".into(),
            "F64" => "double".into(),
            "Bool" => "bool".into(),
            "Char" => "char".into(),
            "CStr" => "const char *".into(),
            "Unit" => "void".into(),
            _ => "VbVal".into(),
        },
        T::Eff(x) => c_type(x),
        _ => "VbVal".into(),
    }
}

fn box_expr(t: &T, v: &str) -> String {
    match t {
        T::Con(n, _) => match n.as_str() {
            "U8" | "U16" | "U32" | "U64" => format!("vb_uint((uint64_t){})", v),
            "I8" | "I16" | "I32" | "I64" => format!("vb_int((int64_t){})", v),
            "F32" | "F64" => format!("vb_float((double){})", v),
            "Bool" => format!("vb_bool({})", v),
            "Char" => format!("vb_char((uint32_t){})", v),
            "CStr" => format!("vb_cstr_val({})", v),
            _ => v.to_string(),
        },
        T::Eff(x) => box_expr(x, v),
        _ => v.to_string(),
    }
}

fn unbox_return(t: &T) -> String {
    match t {
        T::Eff(x) => unbox_return(x),
        T::Con(n, _) => match n.as_str() {
            "Unit" => "(void)r;".into(),
            "U8" | "U16" | "U32" | "U64" => format!("return ({})vb_as_uint(r);", c_type(t)),
            "I8" | "I16" | "I32" | "I64" | "Char" => format!("return ({})vb_as_int(r);", c_type(t)),
            "F32" | "F64" => format!("return ({})vb_as_float(r);", c_type(t)),
            "Bool" => "return vb_as_bool(r);".into(),
            "CStr" => "return vb_as_cstr(r);".into(),
            _ => "return r;".into(),
        },
        _ => "return r;".into(),
    }
}

fn split_fn(t: &T, n: usize) -> (Vec<T>, T) {
    let mut ps = Vec::new();
    let mut cur = t.clone();
    for _ in 0..n {
        match cur {
            T::Fun(a, r) => {
                ps.push(*a);
                cur = *r;
            }
            other => {
                cur = other;
                break;
            }
        }
    }
    (ps, cur)
}

/// The C header for `exp c` (spec §10.2). Preconditions are carried as comments
/// with an explicit warning: nothing verifies them past the boundary.
pub fn header(m: &Module, ck: &Checked) -> String {
    let names = m.exports();
    let mut o = format!(
        "/* {}.h — generated by vibec */\n#ifndef {}_H\n#define {}_H\n\n#include \"vibert.h\"\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n",
        m.name,
        cname(&m.name).to_uppercase(),
        cname(&m.name).to_uppercase()
    );
    for n in &names {
        let f = match m.funs().find(|f| &f.name == n) {
            Some(f) => f,
            None => continue,
        };
        let ar: usize = f.params.iter().map(|p| p.names.len()).sum();
        let sig = ck.sigs.get(n).cloned().unwrap_or(Scheme::mono(T::unit()));
        let (ps, ret) = split_fn(&sig.ty, ar);
        let pres: Vec<String> =
            f.params.iter().flat_map(|p| p.refines.iter().map(expr_text)).collect();
        if !pres.is_empty() {
            o.push_str(&format!(
                "/* {} — pre: {}   NOT VERIFIED ACROSS THE BOUNDARY */\n",
                n,
                pres.join(" && ")
            ));
        }
        let args: Vec<String> =
            ps.iter().enumerate().map(|(i, t)| format!("{} x{}", c_type(t), i)).collect();
        o.push_str(&format!(
            "{} {}_{}({});\n",
            c_type(&ret),
            cname(&m.name),
            cname(n),
            if args.is_empty() { "void".into() } else { args.join(", ") }
        ));
    }
    o.push_str("\n#ifdef __cplusplus\n}\n#endif\n#endif\n");
    o
}

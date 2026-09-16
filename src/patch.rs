//! Structured edit and the two read-only queries that go with it (spec §13.2,
//! §13.3): `patch`, `deps`, `proof`.
//!
//! Addressing is by semantic path (`Ledger.mean.body`), never by index, so it
//! survives insertion and reordering. The extent of a node is computed
//! textually rather than re-rendered, because §13.2 makes the file the source
//! of truth and the AST a derived cache: replacing a byte range keeps every
//! byte the patch did not name, which is what makes the textual diff minimal.
//! Canonical form is what makes that safe — a declaration starts in column 1
//! and everything belonging to it is indented under it, so "where does this
//! node end" has one answer.

use crate::ast::*;
use crate::diag::Span;

/// An addressable node: a semantic path and the byte range it occupies.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub path: String,
    pub start: usize,
    pub end: usize,
}

impl Node {
    pub fn text<'s>(&self, src: &'s str) -> &'s str {
        &src[self.start..self.end]
    }
}

/// FNV-1a over the node's bytes, as 16 hex digits.
///
/// ponytail: not a cryptographic hash, and it does not need to be — it is
/// optimistic concurrency, guarding against a stale view of the file, not
/// against an adversary. Swap in a real digest if a patch ever crosses a trust
/// boundary.
pub fn hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Byte offset of the start of every 1-based line, plus a final sentinel at the
/// end of the source. `starts[n]` is where line `n + 1` begins.
fn line_starts(src: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(src.match_indices('\n').map(|(i, _)| i + 1));
    v.push(src.len());
    v
}

/// Byte offset of a span's first character. `Span::col` is a 0-based byte
/// offset inside its line, so this is addition, not a scan.
fn offset(starts: &[usize], s: Span) -> usize {
    starts.get(s.line - 1).map_or(0, |b| b + s.col)
}

/// Where the declaration starting at `line` ends: the byte before the next
/// declaration, with the blank lines between them left outside.
fn decl_end(src: &str, starts: &[usize], next_line: Option<usize>) -> usize {
    let mut end = match next_line {
        Some(l) => *starts.get(l - 1).unwrap_or(&src.len()),
        None => src.len(),
    };
    while end > 0 && src[..end].ends_with('\n') {
        end -= 1;
    }
    end
}

fn decl_span(d: &Decl) -> Span {
    match d {
        Decl::Type(t) => t.span,
        Decl::Fun(f) => f.span,
        Decl::Ext(e) => e.span,
        Decl::Exp(_, s) => *s,
    }
}

/// Every addressable node of a module, in declaration order. A function
/// contributes three: the whole declaration, its signature, and its body.
pub fn nodes(m: &Module, src: &str) -> Vec<Node> {
    let starts = line_starts(src);
    let mut out = Vec::new();
    for (i, d) in m.decls.iter().enumerate() {
        let start = offset(&starts, decl_span(d));
        let next = m.decls.get(i + 1).map(|n| decl_span(n).line);
        let end = decl_end(src, &starts, next);
        if end <= start {
            continue; // a span the parser could not place; not addressable
        }
        let base = match d {
            Decl::Type(t) => crate::ast::path(&t.home, &t.name),
            Decl::Fun(f) => crate::ast::path(&f.home, &f.name),
            Decl::Ext(e) => format!("{}.ext.{}", m.name, ext_key(&e.header)),
            Decl::Exp(_, _) => format!("{}.exp", m.name),
        };
        if let Decl::Fun(_) = d {
            if let Some((sig_end, body)) = split_head(src, start, end) {
                out.push(Node {
                    path: format!("{base}.sig"),
                    start,
                    end: sig_end,
                });
                out.push(Node {
                    path: format!("{base}.body"),
                    start: body,
                    end,
                });
            }
        }
        out.push(Node {
            path: base,
            start,
            end,
        });
    }
    out
}

/// Split a function declaration at the `=` that introduces its body, returning
/// `(end of signature, start of body)` with the separator and the spaces around
/// it in neither. The body's own span is no use here: it belongs to whichever
/// token the parser chose to hang the node on, which for `a * b` is the `*`.
///
/// The scan looks for a bare `=`: `==`, `<=`, `>=` and `!=` can all appear in a
/// parameter refinement, and nothing else in a signature spells `=`.
fn split_head(src: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let seg = &src[start..end];
    let b = seg.as_bytes();
    let i = (0..b.len()).find(|&i| {
        b[i] == b'='
            && b.get(i + 1) != Some(&b'=')
            && !matches!(
                i.checked_sub(1).map(|p| b[p]),
                Some(b'<' | b'>' | b'!' | b'=')
            )
    })?;
    let body = seg[i + 1..].len() - seg[i + 1..].trim_start_matches([' ', '\t']).len();
    Some((start + seg[..i].trim_end().len(), start + i + 1 + body))
}

/// `"stdio.h"` addresses better than `ext/0` and is as stable as the header is.
fn ext_key(header: &str) -> String {
    header.trim_end_matches(".h").replace(['/', '.'], "_")
}

pub fn find<'n>(nodes: &'n [Node], path: &str) -> Option<&'n Node> {
    nodes.iter().find(|n| n.path == path)
}

/// The source with `n` replaced by `new`. The caller re-checks the result
/// before writing it: a patch that does not parse is refused, not saved.
pub fn apply(src: &str, n: &Node, new: &str) -> String {
    let mut out = String::with_capacity(src.len() + new.len());
    out.push_str(&src[..n.start]);
    out.push_str(new.trim_end_matches('\n'));
    out.push_str(&src[n.end..]);
    out
}

// ------------------------------------------------------------------ deps

/// Callees of every function, and the callers implied by them (§13.3). Names
/// are resolved against the module's own declarations; a qualified name is
/// reported as `M.n`, because the loader has already resolved it; anything
/// else is either the prelude or an `ext c` symbol and is dropped.
pub fn deps(m: &Module) -> String {
    let mut out = String::new();
    let known: Vec<&str> = m.funs().map(|f| f.name.as_str()).collect();
    let mut callers: Vec<(String, String)> = Vec::new();
    for f in m.funs() {
        let mut calls: Vec<String> = Vec::new();
        collect_calls(&f.body, &mut calls);
        calls.retain(|c| known.contains(&c.as_str()) || c.contains('.'));
        calls.sort();
        calls.dedup();
        for c in &calls {
            callers.push((c.clone(), f.name.clone()));
        }
        out.push_str(&format!(
            "{}.{} -> {}\n",
            f.home,
            f.name,
            join(&calls, &m.name)
        ));
    }
    callers.sort();
    callers.dedup();
    for f in m.funs() {
        let up: Vec<String> = callers
            .iter()
            .filter(|(c, _)| *c == f.name)
            .map(|(_, u)| u.clone())
            .collect();
        out.push_str(&format!(
            "{}.{} <- {}\n",
            f.home,
            f.name,
            join(&up, &m.name)
        ));
    }
    out
}

fn join(names: &[String], m: &str) -> String {
    if names.is_empty() {
        "-".to_string()
    } else {
        names
            .iter()
            .map(|n| {
                if n.contains('.') {
                    n.clone()
                } else {
                    format!("{m}.{n}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn collect_calls(e: &Expr, out: &mut Vec<String>) {
    // `Money.vat` parses as a field of a constructor (§9). It is a call into
    // another module, not a record field, so report it qualified and stop:
    // recursing would report the module name `Money` as a callee of its own.
    if let ExprKind::Field(b, n) = &e.kind {
        if let ExprKind::Ctor(m) = &b.kind {
            out.push(format!("{m}.{n}"));
            return;
        }
    }
    if let ExprKind::Var(n) | ExprKind::Ctor(n) = &e.kind {
        out.push(n.clone());
    }
    walk(e, &mut |k| collect_calls(k, out));
}

/// Apply `f` to every direct subexpression. One place to teach the AST shape,
/// so a new `ExprKind` breaks the build instead of silently going unvisited.
fn walk(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    match &e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Char(_)
        | ExprKind::Bool(_)
        | ExprKind::Unit
        | ExprKind::Var(_)
        | ExprKind::Ctor(_) => {}
        ExprKind::App(h, args) => {
            f(h);
            args.iter().for_each(&mut *f);
        }
        ExprKind::Binop(_, a, b) => {
            f(a);
            f(b);
        }
        ExprKind::Neg(i) | ExprKind::Not(i) | ExprKind::Borrow(i) | ExprKind::Field(i, _) => f(i),
        ExprKind::Match(s, arms) => {
            f(s);
            arms.iter().for_each(|(_, b)| f(b));
        }
        ExprKind::Bind(_, v, r) | ExprKind::Let(_, v, r) => {
            f(v);
            f(r);
        }
        ExprKind::Lambda(_, b) | ExprKind::Arena(_, b) => f(b),
        ExprKind::Record(base, fields) => {
            base.iter().for_each(|b| f(b));
            fields.iter().for_each(|(_, v)| f(v));
        }
        ExprKind::Tuple(xs) | ExprKind::List(xs) => xs.iter().for_each(&mut *f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_separates_what_it_must() {
        assert_eq!(hash("t.price * f64 t.qty"), hash("t.price * f64 t.qty"));
        assert_ne!(hash("a"), hash("b"));
        assert_ne!(hash("ab"), hash("ba"), "order has to matter");
        assert_eq!(hash("").len(), 16);
    }

    #[test]
    fn a_head_splits_at_the_bare_equals() {
        let src = "f (n:U64, n >= 1) : U64 = n + 1";
        let (sig, body) = split_head(src, 0, src.len()).expect("a head with a body");
        assert_eq!(
            &src[..sig],
            "f (n:U64, n >= 1) : U64",
            "`>=` is not the separator"
        );
        assert_eq!(&src[body..], "n + 1");
    }

    #[test]
    fn a_head_with_no_body_splits_nowhere() {
        let src = "type Err = Bad Str";
        // `=` is there, but nodes() only asks this of a function declaration
        assert!(
            split_head(src, 0, 0).is_none(),
            "an empty extent has no head"
        );
    }
}

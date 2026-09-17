# Remaining work

What is not done. Read this before starting anything large.

Traps that bite while *writing* Vibelang live in
`.claude/skills/vibelang/references/sharp-edges.md`; this file is about the
compiler. Two bodies of work have their own plans:
[`static-drop-roadmap.md`](static-drop-roadmap.md) and
[`backend-roadmap.md`](backend-roadmap.md).

## Where the compiler is today

2026-09-17. `cargo test -- --test-threads=1`: 168 tests across 12 binaries,
green. `cargo clippy --all-targets -- -D warnings`: clean.

A `.vibe` file goes to a native executable through C99. Working: HM inference
with ADTs, records and exhaustive matching; effects (`E!`); affine ownership
with implicit deep drop and no GC; termination checking for the pure fragment;
refinement obligations discharged by z3 and cached; `ext c` / `exp c`;
multi-file programs with one namespace per module and fields resolved per use
site; `fmt`, the four projections, and the agent surface (`patch`, `deps`,
`proof`, `--diag=struct|json`); a standard library in `lib/`, 20 modules, each
passing `--prove`, each with a runnable check in `lib/check/`.

## Open, by value over cost

1. **A value that may alias is not freed at all**, and a closure's captures are
   freed spine-only. `own.rs::escapes` knows which closures own their captures;
   the runtime does not. `vibe view --drops` counts both. The general case is
   region inference — [`aliasing-audit.md`](aliasing-audit.md), where five of
   eight gaps are still open. Gaps 2 and 3 are the prelude handing out interior
   pointers as owned values: the fix is in the signatures and the runtime, not
   in `own.rs`.
2. **A call's result read only by a match scrutinee is never freed.**
   `?f x |(a, b) -> …` leaks what `f` built. `Regex.decode` packs a pair into
   one `Size` to route around it.
3. **Integer width is not a runtime fact.** Every integer is 64 bits, so
   `bnot (u8 0)` is 2^64-1 typed `U8`, `shl (i32 1) 31` overflows unseen, and
   `add_checked` on two `I32` never reports `Overflow` at 32 bits. The width has
   to reach `vb_bnot`, `vb_shl` and the `_checked` forms from codegen.
4. **`sum` of an empty vector is `vb_int(0)`** whatever the element type — the
   wrong tag for `U64`/`F64`, a crash for `Vec Str`.
5. **Keys and losers are leaked**: the keys `sort_by`/`max_by`/`min_by` compute,
   the argument `min`/`max` drop, the slot `set_owned` and `{r with}` overwrite.
   Each may still be read by a live name, so this needs the may-alias bit (1).
6. **A lambda frees no parameter it owns**, so `fold (\acc x -> …)` over strings
   leaks each accumulator it replaces unless the body updates in place.
7. **The measure checker does not read guards.** `go (next s k)` under
   `?(next s k>k)` is still `total.no_measure`, so parsers thread `fuel`.
   Passing the path condition to the z3 query the refinements already use
   retires it.
8. **Inference is not bidirectional.** A lambda passed to a parameter of written
   type takes no argument type from it, so `\r -> r.qty` is `field.ambiguous`
   whenever two records share `qty`. Correct programs are refused.
9. **A nullary declaration is not a constant** — it compiles to an unapplied
   closure — and a record one aborts at run time when read through
   (`mk.qty`, "expected a record or variant"). Not diagnosed.
10. **A nullary `ext c` binding is never called.** `abort : E! Unit` then
    `abort ;` compiles and does nothing.
11. **Builtin refinements are hardcoded** in `src/refine.rs` (`get`'s bounds is
    the only one), so a new prelude function cannot carry a static refinement.
    `slice` and `index_of` return `Opt` because of this. Fix: declare the
    refinement alongside the signature in `PRELUDE_SIGS`.
12. **A type argument is kept one level deep.** `Ok t` on `Res Err Tx` carries
    `Tx`'s invariant; the `Tx` in `Res Err (Vec Tx)` is lost, and a `<-` binder
    has no type. Both end at keeping the `Ty`, not its base name.
13. **A tuple measure is written, never inferred**, and members of a group must
    write tuples of the same width — a short one is not padded.
14. **`match.nonexhaustive` does not combine nested patterns**:
    `|Some (Ok x) |Some (Er e) |None` still needs `|_`. Conservative, not
    unsound.
15. **A guard's `False` arm learns nothing** when the guard is partly outside
    the solver's fragment: the negation of a conjunction is a disjunction.
16. **Every closure is on the heap** (spec §4.6 asks for zero): `map (\x -> …)`
    costs a `malloc`/`free` pair per call plus one per element in `vb_apply1`.
    A cost, no longer a leak.
17. **`VbVal` is dynamically tagged** and resolved types are not threaded into
    codegen. Any performance claim made before this is a claim about a boxed
    interpreter. Several items above get cheaper afterwards.
18. **A float proof is a proof about a mathematical real.** `--prove` says so in
    a note; saying so is not being right. SMT-LIB `FloatingPoint` is the fix,
    unpriced.
19. **`Regex` has no Unicode tables**: `\p{…}` and `\pL` are refused, `(?i)`
    folds ASCII only. One engine, on the boxed runtime — microseconds a byte
    where Go's costs nanoseconds.
20. **`Json`, `Csv`, `Encoding` still build strings with `concat`** (O(n²)).
    `push_str` exists; each carries its `ponytail:` note. There is no
    `U32 -> Char`.
21. **Mutual tail calls are not guaranteed.** `[[gnu::musttail]]` (spec §11.2)
    cannot be emitted as the calling convention stands — the argument block
    lives in the frame a musttail call replaces. A convention decision.
22. **Cranelift backend** → [`backend-roadmap.md`](backend-roadmap.md). Note its
    own finding: Cranelift alone does not remove the C dependency, because
    `runtime/vibert.c` is still compiled alongside and something must link the
    object.
23. `src/codegen.rs` reaches into `own::inplace_updates` and
    `escape::releasable` — a layering violation the moment there is a second
    backend.
24. **`let … in` where a top-level binding would do** (spec §3.1) — the one
    §3.1 rule never implemented; `view::canon` compares token streams and makes
    no such judgement.
25. `lib/check/io_check.vibe` writes to a `/tmp/claude-1000/…` path from the
    machine it was written on and fails anywhere else.
26. **The skill's token protocol is untested**: no A/B run of the same tasks
    with and without `.claude/skills/vibelang/SKILL.md`, counting tokens and
    whether `--prove` passes.

## Spec §16, still open

- **§16.1 lifetimes** — absent by design; the cost is still unmeasured.
- **§16.2 overflow invariants** — partly relieved by `refine::range_hyp`; a
  `ghost` function is still the escape hatch and still costs more tokens than
  the function it is about. `ghost` parses and is excluded from codegen *and*
  from obligation generation, so it teaches the solver nothing yet.
- **§16.3 modules** — answered without a construct: a declared name carries its
  module. Left: no `import`, aliases or visibility; `ext c` symbols are still a
  global namespace; module names do not nest (`Std.Json` is not a path).
- **§16.4 effect granularity** — one `E!`, undivided.
- **§16.5 solver time** — certificates cached by SMT-text hash in
  `.vibe-proofs`, `--prove-timeout=` budgets each obligation. Left: one edit
  re-asks every question whose text changed.
- **§16.6 cyclic structures**, **§16.7 concurrency** — out of scope for v0.1.
- **§16.8 postconditions** — inferred, not declared: `refine::postconditions`
  derives per-constructor payload facts. One predicate, `len payload >= k`;
  widening it is the next question.

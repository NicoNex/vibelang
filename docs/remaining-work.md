# Remaining work

What is not done, as of the last commit on `master` that touched it. This is the
list to read first in a new session: it is the union of the spec's own open
questions, the gaps the README's Status section names, and the defects found
while building the rest — several of which nobody has written down anywhere else.

Two larger bodies of work have their own plans and are only referenced here:
[`static-drop-roadmap.md`](static-drop-roadmap.md) and
[`backend-roadmap.md`](backend-roadmap.md).

Ordering is by value over cost, not by section number. Each item says where the
code is, so the first step is never a search.

---

## Small, and worth doing first

These are hours, not days, and every one of them is a real defect rather than a
missing feature.

### 1. `own.use_after_move` suggests a `fix` that does not apply

`src/own.rs::use_var` offers ``borrow it here with `&x`, or copy it with `dup x` ``
for every affine value. But `dup` is typed `&Str -> Str` (`src/types.rs`,
`PRELUDE_SIGS`), so on anything other than a string the suggested edit produces a
`type.mismatch` — a second retry for a generator that did exactly what the
diagnostic told it to.

The `fix` field is the project's headline claim: *not advice, an edit*. A `fix`
that does not apply is worse than no `fix`.

What it needs: `own.rs` knows the declared type of a parameter (`Param::ty`) but
only a `bool` for `let`, `<-` and pattern binders — `Checked::affine` in
`src/infer.rs` stores affinity and discards the type. Widen that map's value from
`bool` to the resolved base type name, then suggest `dup` only when the type is
`Str`, and `&x` alone otherwise.

### 2. Signed subtraction generates no overflow obligation

`f (a:I64) (b:I64) : I64 = a - b` produces no obligation; the unsigned version
produces one. See `src/refine.rs::arith`, which handles `-` only for the `Nat` /
unsigned underflow case. Signed overflow is just as real and the machinery is
already there.

### 3. An obligation the solver cannot express is dropped silently

`src/refine.rs::term` returns `None` for anything outside linear integer/real
arithmetic, and `push` then skips the obligation entirely. So a program can pass
`vibe check --prove` because the compiler could not phrase the question, not
because the answer was yes.

That is the one failure mode a prover must not have quietly. It does not need the
fragment widened — it needs the skip counted and reported, so `vibe proof` can
say *"3 obligations could not be expressed"* instead of nothing.

### 4. `vibe deps` stops at the module boundary

`App.gross` calls `Money.vat` and `Money.cents`; `vibe deps examples/shop/app.vibe`
prints `App.gross -> -`. `src/patch.rs::deps` filters callees against the current
module's own functions and drops qualified names on the floor. The loader already
resolves them, so the fix is to report `Field(Ctor(M), n)` as `M.n` rather than
discarding it.

### 5. `vibe patch` addresses the root file only

`src/main.rs::do_patch` builds its node table from `prog.root()`. In a multi-file
program every other module is unaddressable. `load::Program::units` already
carries each file with its source and its file id, so this is plumbing rather than
design.

### 6. Floats go to the solver as mathematical reals

`src/refine.rs::Sort::Real` maps `F32`/`F64` to SMT `Real`, which says nothing
about rounding, precision or NaN. A proof about a float is therefore a proof
about a number the program does not have. Either move to SMT-LIB floating point
or say plainly in the diagnostic which obligations are approximated.

---

## Medium

### 7. Postconditions (spec §7.1, §16.8)

The spec puts refinements on parameters only, so a function cannot state anything
about its result. This is the single reason the reference program still has two
open obligations:

```console
$ vibe proof --prove examples/ledger.vibe
Ledger.main.body/0	refine.unproven	cannot prove the precondition of `mean`: len ts > 0
Ledger.main.body/1	refine.unproven	cannot prove the precondition of `top`: len ts > 0
```

The fact that rules out the empty case lives in `load`, whose `Ok []` arm returns
`Er Void`. §8.3's mechanism already carries that across arms of *one* match — see
`src/refine.rs::pat_facts` — but not across a call.

Two routes, neither chosen:

- **Declared.** Add syntax to §7.1 for a refinement on the result, and check it
  at every `return` position. Explicit, costs the generator tokens on every
  signature that needs one.
- **Inferred.** Derive a constructor-conditioned postcondition from the body —
  for `load`, *result is `Ok x` implies `len x != 0`* — and assume it at call
  sites. Costs no tokens and is where the project's whole argument points, but it
  is real work and the inference has to be sound or every proof built on it is
  worthless.

Prefer the second, and only after item 3: an inferred postcondition that is
silently dropped when it cannot be phrased is exactly the failure mode above,
with more at stake.

### 8. A tail-recursive loop still accumulates

Per-frame release (`src/escape.rs`) marks once at function entry and releases once
on return, but a self-tail-call compiles to a `for (;;)` with the release
*after* the loop (`src/codegen.rs::function`). So iterations pile up until the
function returns, and a function that never returns — now legal, since divergence
is an effect — never releases at all. `arena a in ...` is the manual override.

A mark per iteration needs to know which values cross the back edge. This is the
same analysis [`static-drop-roadmap.md`](static-drop-roadmap.md) needs; do it
there rather than twice.

### 9. Generic constructor payloads lose their type

`src/refine.rs::pat_facts` reads a payload's declared type from `CtorInfo::args`.
For a monomorphic ADT that gives a concrete type, so a record invariant flows
through `B t`. For `Res e t` the argument is a type variable, so `Ok ts` learns
the payload's *length* but not its record invariant. Instantiating the
constructor's type from the scrutinee's would fix it; `ty_of` currently returns a
base name and throws the arguments away.

### 10. Mutual recursion gets a single linear measure

`src/total.rs` compares one linear expression per function, not a lexicographic
tuple, so a mutually recursive group where the measure shifts between components
is rejected. Spec §6.4. The upgrade path is written in the module's own header
comment.

### 11. `let ... in` where a top-level binding would do (spec §3.1)

The one §3.1 rule never implemented. `view::canon` compares token streams, and
the projection prints a `let` exactly as written, so this needs a judgement the
renderer does not make. Low value on its own; worth doing if §3.1 is ever
audited as a whole.

---

## Large, with plans already written

### 12. Implicit drop → [`static-drop-roadmap.md`](static-drop-roadmap.md)

Delete the bump allocator; deallocate at the point the owner dies. The ownership
checker already computes that point and discards it. Read the plan's preconditions
before starting: two properties the current design is safe to get wrong become
use-after-free and double-free once drops are real.

### 13. Cranelift backend → [`backend-roadmap.md`](backend-roadmap.md)

A native backend so a pure Vibelang program needs no C toolchain, behind a
`CodegenBackend` trait, with today's emission moved into `CBackend`. C is not
deprecated by it: `--emit-c` and `exp c` header generation are why it stays.

Note the plan's own finding — Cranelift alone does not remove the C dependency.
`cranelift-object` emits a relocatable object, something still has to link it,
and `runtime/vibert.c` is C source compiled alongside the generated program. That
is an open question in the plan, not a decision.

### 14. Stack allocation for a closure that does not escape (spec §4.6)

The ownership half of §4.6 is enforced — an escaping closure owns its captures
(`src/own.rs::escapes`). The allocation half is not: every closure is
heap-allocated. §4.6 asks for zero cost when the closure does not escape, and the
analysis that decides it already exists.

---

## Design questions the spec itself leaves open

Carried from §16, with what has changed since:

- **§16.1 No lifetimes.** Cost still unmeasured; nobody has written zero-copy C
  interop against it yet.
- **§16.2 Overflow invariants.** Partly relieved by the range hypotheses in
  `src/refine.rs::range_hyp`, but a ghost function is still the escape hatch and
  still costs more tokens than the function it is about.
- **§16.3 Module system.** Implemented as specified (`src/load.rs`) and, as the
  spec predicted, the flat namespace does not scale: two modules cannot both
  declare `parse`, and a type cannot be written `Ledger.Tx`. The constraint is
  that a fix must not reintroduce `import`, aliases and visibility — four
  constructs to replace one.
- **§16.4 Effect granularity.** One `E!`, undivided. Unchanged.
- **§16.5 Solver time.** Partly addressed: certificates are cached by the hash of
  the SMT text in a sibling `.vibe-proofs`, and `--prove-timeout=` budgets each
  obligation. What is left is granularity — the cache is per obligation, so one
  edit anywhere re-asks every question whose text changed.
- **§16.6 Cyclic structures.** Unchanged.
- **§16.7 Concurrency.** Out of scope for v0.1. Unchanged.
- **§16.8 Postconditions.** Item 7 above.

---

## Things that are true and easy to forget

- `VbVal` is dynamically tagged, and resolved types are not threaded into
  codegen. `runtime/vibert.h` says so in its own header comment. Several of the
  items above would be cheaper afterwards, and any performance claim made before
  it is a claim about a boxed interpreter.
- Mutual tail calls are not guaranteed. Spec §11.2 asks for `[[gnu::musttail]]`
  so an unrealisable one fails visibly; neither backend does this.
- `src/codegen.rs` reaches into `own::inplace_updates` and `escape::releasable`
  from inside `generate()`. Harmless today, a layering violation the moment there
  is a second backend — [`backend-roadmap.md`](backend-roadmap.md) turns it into
  a task.
- An `ext c` refinement cannot name a parameter, because an `ext` signature is a
  type and has no parameter names. `README.md` and spec §10.1 both say so now;
  the spec draft's `(n:Size, n>0) -> E! I32` was never valid syntax.

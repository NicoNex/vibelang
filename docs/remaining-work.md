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

## Medium

### 1. A tail-recursive loop still accumulates

Per-frame release (`src/escape.rs`) marks once at function entry and releases once
on return, but a self-tail-call compiles to a `for (;;)` with the release
*after* the loop (`src/codegen.rs::function`). So iterations pile up until the
function returns, and a function that never returns — now legal, since divergence
is an effect — never releases at all. `arena a in ...` is the manual override.

A mark per iteration needs to know which values cross the back edge. This is the
same analysis [`static-drop-roadmap.md`](static-drop-roadmap.md) needs; do it
there rather than twice.

### 2. Generic constructor payloads lose their type

`src/refine.rs::pat_facts` reads a payload's declared type from `CtorInfo::args`.
For a monomorphic ADT that gives a concrete type, so a record invariant flows
through `B t`. For `Res e t` the argument is a type variable, so `Ok ts` learns
the payload's *length* but not its record invariant. Instantiating the
constructor's type from the scrutinee's would fix it; `ty_of` currently returns a
base name and throws the arguments away.

### 3. Mutual recursion gets a single linear measure

`src/total.rs` compares one linear expression per function, not a lexicographic
tuple, so a mutually recursive group where the measure shifts between components
is rejected. Spec §6.4. The upgrade path is written in the module's own header
comment.

### 4. `let ... in` where a top-level binding would do (spec §3.1)

The one §3.1 rule never implemented. `view::canon` compares token streams, and
the projection prints a `let` exactly as written, so this needs a judgement the
renderer does not make. Low value on its own; worth doing if §3.1 is ever
audited as a whole.

---

## Large, with plans already written

### 5. Implicit drop → [`static-drop-roadmap.md`](static-drop-roadmap.md)

Delete the bump allocator; deallocate at the point the owner dies. The ownership
checker already computes that point and discards it. Read the plan's preconditions
before starting: two properties the current design is safe to get wrong become
use-after-free and double-free once drops are real.

### 6. Cranelift backend → [`backend-roadmap.md`](backend-roadmap.md)

A native backend so a pure Vibelang program needs no C toolchain, behind a
`CodegenBackend` trait, with today's emission moved into `CBackend`. C is not
deprecated by it: `--emit-c` and `exp c` header generation are why it stays.

Note the plan's own finding — Cranelift alone does not remove the C dependency.
`cranelift-object` emits a relocatable object, something still has to link it,
and `runtime/vibert.c` is C source compiled alongside the generated program. That
is an open question in the plan, not a decision.

### 7. Stack allocation for a closure that does not escape (spec §4.6)

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
- **§16.8 Postconditions.** Inferred, not declared: `refine::postconditions`
  derives per-constructor payload facts from a body and assumes them at call
  sites. The reference program has no open obligation. The vocabulary is one
  predicate, `len payload >= k`; widening it is the next question.

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
- A proof about an `F32`/`F64` is a proof about a mathematical real, not about
  the machine float the program has: `refine::Sort::Real` says nothing about
  rounding, precision or NaN. The diagnostic now says so — obligations carrying a
  real are reported as approximated — but saying so is not the same as being
  right. SMT-LIB `FloatingPoint` is the fix, and nobody has priced it.
- Borrow-after-move is not checked at all, and the reference program relies on
  it: `len ts` moves `ts`, so the `&ts` after it is a read of a moved value.
  Only `{r with ...}` bases are checked (`own.use_after_update`). See
  [`aliasing-audit.md`](aliasing-audit.md) gap 5.
- `vibe deps` reports callees across module boundaries but callers (`<-`) only
  within the root module, because that direction would mean walking every unit.
- An `ext c` refinement cannot name a parameter, because an `ext` signature is a
  type and has no parameter names. `README.md` and spec §10.1 both say so now;
  the spec draft's `(n:Size, n>0) -> E! I32` was never valid syntax.

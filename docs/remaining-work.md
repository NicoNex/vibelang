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

### 1. A drop is shallow, and what may be shared is not freed at all

Done: the bump allocator is gone and every value is freed where its owner dies
([`static-drop-roadmap.md`](static-drop-roadmap.md), all nine tasks). What is
left is the two deliberate retreats that made it safe.

`vb_dispose` frees a vector's spine and not its elements, because the structural
operations copy element pointers between vectors and a deep free would free one
twice. And anything that *may* alias — a prelude result that points into an
argument, a capture of a closure a callee may keep, a pointer given to C — is
not freed at all; `vibe view --drops` counts those suppressions. Both give up
memory to avoid a use-after-free, and both come back the same way: knowing,
per value, that nothing else points at it.

### 2. A type argument is kept one level deep

`src/refine.rs::pat_facts` instantiates a constructor's payload type from the
scrutinee's type arguments, so `Ok t` on a `Res Err Tx` carries `Tx`'s record
invariant (`tests/refine_ctor.vibe`). What is left: `ty_args` keeps the *head*
of each argument, so the `Tx` in `Res Err (Vec Tx)` is still lost, and a binder
introduced by `<-` still has no type at all. Both end at the same fix — keep the
`Ty`, not its base name.

### 3. A tuple measure is written, never inferred

`src/total.rs` compares measures lexicographically, so a mutually recursive
group whose shrinking component changes is accepted when each member writes
`%(n, k)` (§6.4, `tests/total_lex.vibe`). What is left: inference still only
handles direct self-recursion over one parameter, and the members of a group
must write tuples of the same width — a short one is not padded.

### 4. `let ... in` where a top-level binding would do (spec §3.1)

The one §3.1 rule never implemented. `view::canon` compares token streams, and
the projection prints a `let` exactly as written, so this needs a judgement the
renderer does not make. Low value on its own; worth doing if §3.1 is ever
audited as a whole.

---

## Large, with plans already written

### 5. Implicit drop — done

[`static-drop-roadmap.md`](static-drop-roadmap.md) is executed end to end. The
numbers it was written for: `examples/loop.vibe` — two million iterations, an
allocation in each, nothing kept — is flat at 1.4 MB where the bump allocator
grew to 197 MB, and `examples/churn.vibe` is unchanged at 1.5 MB. Every example
runs clean under `-fsanitize=address,undefined`.

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
heap-allocated, and now that allocation is `malloc` rather than a bump pointer,
`map (\x -> ...)` costs a `malloc`/`free` pair per call plus one per element
inside `vb_apply1`. It is no longer a leak — that was fixed — it is a cost, and
§4.6 asks for it to be zero.

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
- Mutual tail calls are not guaranteed, and `[[gnu::musttail]]` (spec §11.2)
  cannot be emitted as the calling convention stands: a function takes
  `VbVal *a`, and the array it would point at lives in the caller's frame, which
  a musttail call replaces. The attribute needs the argument block to live
  somewhere the callee can still reach — a decision about the convention, not a
  line of codegen.
- `src/codegen.rs` reaches into `own::inplace_updates` and `escape::releasable`
  from inside `generate()`. Harmless today, a layering violation the moment there
  is a second backend — [`backend-roadmap.md`](backend-roadmap.md) turns it into
  a task.
- A proof about an `F32`/`F64` is a proof about a mathematical real, not about
  the machine float the program has: `refine::Sort::Real` says nothing about
  rounding, precision or NaN. The diagnostic now says so — obligations carrying a
  real are reported as approximated — but saying so is not the same as being
  right. SMT-LIB `FloatingPoint` is the fix, and nobody has priced it.
- Five of the eight gaps in [`aliasing-audit.md`](aliasing-audit.md) are still
  open. Gaps 2 and 3 are the prelude handing out interior pointers as owned
  values, and no check on user code can reach them: the fix is in the
  signatures and the runtime, not in `own.rs`.
- Builtin refinements are hardcoded in `src/refine.rs` — `get`'s bounds
  obligation is the only one. So a new prelude function cannot carry a static
  refinement; `slice` and `index_of` return `Opt` because of this, not because
  `Opt` was the better design. Making the refinement declarable alongside the
  signature in `PRELUDE_SIGS` is the fix.
- The integration tests share a build directory and race when run in parallel;
  `cargo test -- --test-threads=1` is the reliable invocation, and CI pins it.
- `vibe deps` reports callees across module boundaries but callers (`<-`) only
  within the root module, because that direction would mean walking every unit.
- An `ext c` refinement cannot name a parameter, because an `ext` signature is a
  type and has no parameter names. `README.md` and spec §10.1 both say so now;
  the spec draft's `(n:Size, n>0) -> E! I32` was never valid syntax.


---

## Before a standard library can be written

Not a list of missing functions. These are the three things that make a library
of more than one module impossible to write today, each confirmed against the
compiler rather than read off the spec.

1. **A module has to be a sibling file.** `Json.parse` resolves to `Json.vibe`
   next to the file that names it (`src/load.rs`): no search path, no
   directories, no place for a shipped library to live. `error[mod.missing]`
   is what a `lib/` subdirectory gets.
2. **One flat namespace, so two modules cannot both declare `parse`.**
   `error[mod.duplicate]`, and the fix it prints is "rename one of them". A
   library of ten modules has to make every name in it globally unique, and a
   type still cannot be written `Json.Value` because flattening keeps only
   `Value`. Spec §16.3 predicted exactly this.
3. **The prelude owns its names outright.** A module declaring `take`, `get` or
   `map` gets `error[name.duplicate]` — the names a collections library most
   wants are the ones it cannot have.

Three and two are the same fix: per-module name resolution in `src/infer.rs`,
after which `load.rs` keeps the loading and drops the renaming, and the prelude
becomes one more module rather than a reserved word list. One is small and
independent: a search path, defaulting to a directory shipped with the compiler.

Worth knowing before starting, none of them blocking:

- Recursive helpers need a measure unless one parameter shrinks syntactically at
  every self-call; a mutually recursive pair needs the lexicographic tuple
  written out (`%(n, k)`).
- `dup` is `&Str -> Str` and nothing else. Copying an aggregate has no answer
  yet — deep copy, shared immutable value, or a diagnostic that stops offering
  it (`static-drop-roadmap.md`, Open decisions 4).
- A library that builds nested structures leaks the inner ones: drops are
  shallow (item 1 above).

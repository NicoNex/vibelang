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

## Where the compiler is today

2026-09-16. `cargo test -- --test-threads=1`: 150 tests across 12 binaries,
green. `cargo clippy --all-targets -- -D warnings`: clean.

A `.vibe` file goes to a native executable through C. What stands between the
two: Hindley–Milner inference with ADTs, records and exhaustive matching;
effects (`E!`); affine ownership with implicit drop and no allocator to rewind;
termination checking for the pure fragment; refinement obligations discharged
with z3 and cached; `ext c` / `exp c` both ways; multi-file programs with one
namespace per module; the projections, `vibe fmt`, and the agent surface of
§13.3 (`patch`, `deps`, `proof`).

Shipped since this list was first written, each with the item it closed:

- **Implicit drop, and the bump allocator deleted** (item 5). Every value is
  freed where its owner dies. `examples/loop.vibe` — two million iterations, an
  allocation in each, nothing kept — is flat at 1.4 MB where the bump allocator
  grew to 197 MB. Every example runs clean under `-fsanitize=address,undefined`.
- **A module is a namespace** (§16.3). A declared name carries the module that
  declared it, so two modules may declare `parse`, a module may declare `take`
  against the prelude, types and constructors qualify, and `VIBE_PATH` is where
  a library lives. Still no `import`, no aliases, no visibility.
- **Bit operations** — `band`, `bor`, `bxor`, `bnot`, `shl`, `shr`, `ord` — as
  prelude functions rather than operators, because `&` and `|` are taken and a
  call has no precedence to get wrong.
- **A dictionary.** `dict`, `insert`, `lookup`, `remove`, `keys`, and `len`
  which already worked on anything. It needed no runtime *tag*: a `Dict k v` is
  a vector of two-field objects, so the deep drop, `vb_dup` and `show` were
  already right for it, and the representation can change without any of them
  knowing. Keys compare with `vb_eq`, the equality the language already has. A
  set is a `Dict k Unit` and gets no names of its own. Lookup is a linear scan
  — see the ceiling below.
- **A drop is deep.** `vb_dispose` recurses into a vector's elements and an
  object's fields. `examples/nested.vibe` — 200,000 iterations, eight fresh
  strings in each, nothing kept — is flat at 1.5 MB where the shallow drop grew
  to 52 MB. Three things had to be true first: the `&Vec` operations copy each
  element rather than its pointer, a function may no longer return a piece of a
  borrowed parameter, and a value that may alias is not dropped at all.
- **A borrow no longer escapes through a derivation.** `own.borrow_escapes`
  asked whether the tail *was* a borrowed name; it asks now whether the result
  is made of one, so `pick (r:&R) : Str = r.s` and `ts |> max_by amt` are
  refused with `dup` as the fix. This is gap 1 of `aliasing-audit.md`, and
  closing it is what the audit said it would be: not the code, but
  `examples/ledger.vibe::top`, now `dup &(ts |> max_by amt)`.
- **`dup` copies in depth, and copies anything** (`static-drop-roadmap.md`,
  Open decision 4, answered). `&a -> a`: the copy owns what it points at, so it
  survives the original and can be freed on its own. Spec §4.4 offers "return a
  copy" as one of three answers to the absence of lifetimes, and it existed for
  `Str` alone until now. `VB_CSTR` and `VB_PTR` stay shallow — C owns that
  memory and its extent is not known here — so a copy holding one still aliases,
  which is the suppression deep drop has to keep making.
- **A drop no longer lands ahead of the expression that reads it.** `ex` returns
  an expression *string*, so a `vb_dispose` pushed after it was emitted *before*
  it: `vbret = vb_len(v_s_5)` after `vb_dispose(v_s_5)`. Three sites in
  `src/codegen.rs` violated `flush`'s own stated contract. `settle` names the
  result first, and only where the result is a compound expression, so the
  snapshots in `tests/drop.rs` did not move.
- **A record field is resolved per use site, not by its name.** Inference
  already knew which record every `.field` and every literal meant; it writes
  that down in `Checked::field_of`, keyed by span, and codegen reads it instead
  of guessing from `Data::field_owner`. Two records may now declare `val` — in
  one module or in two — and a site inference could not pin down is
  `field.ambiguous` rather than a coin toss. This was silently wrong before, not
  merely restricted: `tests/field_share.vibe` printed the tag instead of the
  value, with no diagnostic.
- **An `ext c` symbol is C's, so two modules may declare it.** What they may not
  do is give it two types, which is the only thing `global_clashes` still
  checks. Record fields left that list.
- **`vibe fmt`**, which writes the canonical form back to the file; `--check`
  reports and fails instead.
- **A measure may be a lexicographic tuple** (`%(n, k)`), so a mutually
  recursive group whose shrinking component changes has something to write.
- **A generic payload keeps the invariant of what it holds**, so `Ok t` on a
  `Res Err Tx` discharges what `Tx` declares.
- **A pure function can no longer diverge through a higher-order call.** A
  recursive name used as a value is an edge in the call graph.

---

## Medium

### 1. A closure's captures are not freed, and what may alias is not freed at all

The first of the two retreats is gone: `vb_dispose` recurses into a vector's
elements and an object's fields. `examples/nested.vibe` builds eight fresh
strings an iteration and keeps none — 1.5 MB, where the shallow drop grew to
52 MB.

Two suppressions remain.

A **closure** is still freed spine-only. Its captures belong to the frame that
built it unless the closure escapes, and `own.rs::escapes` knows which while the
runtime does not; freeing them in `vb_dispose` would free the frame's values
under it. The fix is the escape bit reaching the runtime, or codegen emitting
the deep free for the closures it already knows own their captures.

Anything that **may alias** — a prelude result that points into an argument, a
`map` given a lambda that returns a piece of its own element, a pointer handed
to C — is not freed at all, and `vibe view --drops` counts those. That one comes
back the same way it always would: knowing, per value, that nothing else points
at it.

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

### 5. A value that may alias, and a closure's captures

See item 1. Deep drops landed; what is left of that plan is the two suppressions
above, and the general case of aliasing — knowing, per value, that nothing else
points at it — which is region inference
([`aliasing-audit.md`](aliasing-audit.md), "the part that really is bigger than
the plan").

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
- **§16.3 Module system.** Answered without adding a construct. A declared name
  carries its module in the flattened program — `Csv.parse`, which is the
  spelling a qualified reference already had — so two modules may declare
  `parse`, a module may declare `take` against the prelude, and `Shape.Kind` and
  `Shape.Line` qualify a type and a constructor. Still no `import`, no aliases,
  no visibility: qualification at the use site is the whole system, and
  `VIBE_PATH` says where a library lives. What is left is the two namespaces
  that stayed global — record fields and `ext c` symbols — and module names that
  cannot nest (`Std.Json` is not a path).
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

- A dictionary lookup is a linear scan, so holding n entries costs O(n) per
  lookup and building one costs O(n²). The upgrade path is a hash table behind
  the same five names, and it needs a hash for every tag `vb_eq` compares.
  Nothing has been slow yet.
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
- **Gap 1 is closed for the name and open for every derivation of it**, which is
  the prerequisite a deep drop actually has. `own.borrow_escapes` rejects
  `launder (s:&Str) : Str = s`, because `own::returned` collects names in tail
  position. `pick (r:&R) : Str = r.s` is not a name, checks clean, and hands the
  caller a pointer into a record the caller still owns. So does
  `map (\x -> x) v`, and so does `map (\x -> x.s) v`. `own.rs::maybe_shared`
  assumes in writing that this shape is not in the language; it is. Shallow
  drops pay nothing for it, and it is the first thing a deep drop frees twice.
- Builtin refinements are hardcoded in `src/refine.rs` — `get`'s bounds
  obligation is the only one. So a new prelude function cannot carry a static
  refinement; `slice` and `index_of` return `Opt` because of this, not because
  `Opt` was the better design. Making the refinement declarable alongside the
  signature in `PRELUDE_SIGS` is the fix.
- The integration tests share a build directory and race when run in parallel;
  `cargo test -- --test-threads=1` is the reliable invocation, and CI pins it.
- A recursive name used as a *value* is an edge in the totality call graph, so
  `sum &(map loopy &(single n))` is rejected: the call has unknown arguments and
  no measure can be shown to decrease at it. The conservative direction — a
  terminating program written that way is rejected too, and the answer is to
  call the function rather than pass it.
- `vibe deps` reports callees across module boundaries but callers (`<-`) only
  within the root module, because that direction would mean walking every unit.
- An `ext c` refinement cannot name a parameter, because an `ext` signature is a
  type and has no parameter names. `README.md` and spec §10.1 both say so now;
  the spec draft's `(n:Size, n>0) -> E! I32` was never valid syntax.
- **`a % b` is not modulo, and nothing says so.** `%` introduces a termination
  measure, so `f (a:U64) (b:U64) : U64 = a % b` parses as the body `a` with the
  measure `%b`, checks clean, and `f 7 3` returns 7. There is no modulo operator
  in the prelude; the shape that asks for one is accepted and silently means
  something else. A body followed by a measure on the same line is the ambiguity.
- **A negative literal cannot be a bare argument.** `f (-2.5)` is refused as
  `canon.form`, because the projection renders it `f -2.5`, which re-parses as
  the subtraction `f - 2.5`. `(0.0 - 2.5)` is the spelling that works. This is
  the one case where `canon.form` fires on source a person would write rather
  than guarding the compiler against itself.
- **A nullary declaration whose value is a record cannot be read through.**
  `mk : Ta = {note="x", qty=7}` followed by `mk.qty` aborts at run time with
  "expected a record or variant", in one module or across two. Found while
  writing `tests/modules.rs`; the fixture takes a parameter to route around it.
  Not diagnosed, so it is a run-time failure rather than a compile error.
- **Inference is not bidirectional.** A lambda passed to a parameter of written
  type does not take its argument type from that signature, so `\r -> r.qty`
  leaves `r` a variable. It used to resolve by field name and be right by luck;
  it is now `field.ambiguous` whenever two records share the field. The program
  is correct and is refused, and the fix in the diagnostic — write the type — is
  the honest one until checking is bidirectional.


---

## Before a standard library can be written

Not a list of missing functions. What is left after the module system landed
(`src/load.rs`: a name carries the module that declared it, so `Csv.parse` and
`Json.parse` are two names, a module may declare `take`, types and constructors
qualify, and `VIBE_PATH` plus a `lib` directory beside the compiler are on the
search path).


Worth knowing before starting, none of them blocking:

- Recursive helpers need a measure unless one parameter shrinks syntactically at
  every self-call; a mutually recursive pair needs the lexicographic tuple
  written out (`%(n, k)`); and a recursive name passed as a *value* is a call
  with unknown arguments, so it is rejected.
- A library that builds nested structures leaks the inner ones: drops are
  shallow (item 1 above).

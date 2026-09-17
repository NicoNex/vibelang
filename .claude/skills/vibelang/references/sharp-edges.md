# Sharp edges

What the compiler does not do yet, and what to do instead. Each item is either verified
against the compiler in this tree or sourced from `docs/remaining-work.md`.

**`docs/remaining-work.md` is the authority and this file is a snapshot.** The compiler is
under active development and items move from one list to the next quickly — the module and
field namespacing in particular. Re-read `docs/remaining-work.md` before starting anything
large, and when it disagrees with this file, believe it. When a claim here matters to a
decision, spend thirty seconds confirming it against the compiler rather than trusting the
snapshot.

Contents:
1. [Missing capabilities](#missing-capabilities)
2. [Things that parse but do not mean what they look like](#things-that-parse-but-do-not-mean-what-they-look-like)
3. [Where the proofs are weaker than they read](#where-the-proofs-are-weaker-than-they-read)
4. [Memory](#memory)
5. [Modules](#modules)
6. [Working on the compiler itself](#working-on-the-compiler-itself)

---

## Missing capabilities

**`remove` on a dictionary is O(n).** `Dict k v` (`dict insert lookup remove keys`) is a hash
map in insertion order: `insert` and `lookup` are O(1), `remove` shifts the entries after
it. A set is a `Dict k Unit`.

**Grow a string with `push_str`, not `concat`.** `concat` copies both sides, so building a
string a piece at a time with it is O(n²). `push_str acc &piece` consumes `acc` and appends
in place when the frame holds it alone — a tail-call accumulator, a `fold` lambda's
accumulator — which is linear. A byte is `push_str acc &(byte_str b)`.

**No `U32 -> Char`.** `chr` takes a `Char` and `ord` gives a `U32`, but nothing goes back.
Use `byte_str : U8 -> Str` to make a byte, and `byte_at : &Str -> Size -> U8` to read one.

**No record pattern.** The grammar lists one, the parser rejects it with `parse.pattern`.
Match on ADTs; project records with `.`.

```
;; parse error
?t |{sku=s, qty=q} -> fmt "{} x{}" s q end

;; what to write
label (t:&Tx) : Str = fmt "{} x{}" t.sku t.qty
```

**No modulo operator, no remainder.** See below — `%` means something else.

**No postcondition syntax.** A refinement constrains parameters only, so a fact established
inside a function does not leave it. One narrow exception is inferred, not declared:
`refine::postconditions` derives per-constructor payload facts of the form
`len payload >= k` from a body and assumes them at call sites. That vocabulary is one
predicate wide; anything else has to be re-established at the call site, usually with a
match arm.

**`ghost` teaches the solver nothing.** `ghost` declarations parse, and `src/refine.rs`
skips them when generating obligations just as `src/codegen.rs` skips them when emitting
code. They are inert today. The spec offers them as the escape hatch for overflow
invariants; that is not yet true.

**No `import`, alias or visibility**, by design. Qualification at the use site is the whole
module system.

**No native backend.** Everything goes through C, so a C toolchain is required to build.

---

## Things that parse but do not mean what they look like

**`%` is a termination measure, not modulo.**

```
f (a:U64) (b:U64) : U64 = a % b
```

compiles, with body `a` and measure `b`. `vibe view` shows it:

```
f (a:U64) (b:U64) : U64 =
  a
  %b
```

**A nullary top-level declaration is not a constant.** `limit : U64 = 42` compiles to an
unapplied closure; referring to `limit` yields the closure, and `out (show limit)` prints
`<Mod.limit>`. A `Vec`-typed one fails at run time with `expected a Vec`. Only `main` works
nullary, because the driver calls it. Put constants inside the function that uses them, or
give the declaration a parameter. `vibe check` does not catch this.

**A negative literal cannot be a bare argument.** `f (-2.5)` is `canon.form`, because the
canonical rendering `f -2.5` re-parses as the subtraction `f - 2.5`. Write `f (0.0 - 2.5)`,
or bind the value with `let`.

**A recursive name passed as a value is rejected.** It is an edge in the totality call graph
with unknown arguments, so no measure can decrease at it:

```
loopy (n:U64) : U64 = sum &(map loopy &(single n))   ;; total.no_measure
```

The direction is conservative on purpose — a terminating program written that way is
rejected too. Call the function instead of passing it.

**A file named `proofs.vibe` collides with the proof cache.** The build directory for
`x.vibe` is `.vibe-x`, and the proof cache beside a source file is `.vibe-proofs`, so a
module named `Proofs` fails with `cannot create .vibe-proofs: File exists`. Name it
anything else.

---

## Where the proofs are weaker than they read

**`vibe check` and `vibe check --prove` are different bars.** Without `--prove`, refinement
obligations are left open (a note says how many) and asserted at run time. With `--prove`,
z3 discharges them — and `--prove` also raises overflow obligations on every `+`, `-`, `*`
over a machine integer, which the bound has to come from a signature to satisfy. Several
files in this repository pass `check` and fail `check --prove` for exactly that reason.
Always say which one you ran.

**A proof about an `F32`/`F64` is a proof about a mathematical real.** `refine::Sort::Real`
says nothing about rounding, precision or NaN. `--prove` prints a note when an obligation
carries a real, but saying so is not the same as being right. Never present a float proof
as a guarantee about the machine float.

**Some obligations are silently not checked.** `note: N obligation(s) could not be expressed
in the solver's fragment and were not proved` means the generator could not phrase the
goal. `--prove` exiting 0 alongside that note is weaker than it looks — read the note.

**Builtin refinements are hardcoded**, and `get`'s bounds obligation is the only one. A new
prelude function cannot carry a static refinement; `slice` and `index_of` return `Opt`
because of this, not because `Opt` was the better design.

**A type argument is only kept one level deep.** `Ok t` on a `Res Err Tx` carries `Tx`'s
record invariant into the arm, but the `Tx` in `Res Err (Vec Tx)` is lost, and a binder
introduced by `<-` has no type at all for the solver. Expect a proof that "obviously"
follows to fail when the fact has to travel through two type constructors.

**Measure inference is narrow, and the measure checker does not read guards.** Inference
only handles direct self-recursion over one scalar parameter that shrinks syntactically at
every call (a call to a helper outside the recursive group does not get in the way). A
written measure may name only scalar parameters — `%(len s - i)` is refused, so pin the
length: `(n:Size, n==len s)` and `%(n - i)`. And whether `n - i` decreases is decided without
the guards around the call, so the argument must shrink syntactically (`i + 1`); a position
returned by a callee needs a `fuel:U64` parameter measured `%fuel`.

**What a guard lends the solver.** A guard's `True` arm learns each conjunct the solver can
phrase, even when others (a call, a string comparison) are out of its fragment; the `False`
arm of such a guard learns nothing. The left side of `&&` is known while checking the right.
`let x = call …` keeps the numeric range of the call's result type; the same call inline
inside a conversion (`u64 (byte_at s i)`) does not.

**A nullary `ext c` binding is never called.** `abort : E! Unit` followed by `abort ;`
compiles and does nothing, like a nullary top-level declaration. Give the binding a
parameter.

**The proof cache is keyed on SMT text**, not on the subtree, so an unrelated edit to the
context re-asks every question whose text changed. A slow `--prove` after a small edit is
expected, not a bug.

---

## Memory

**Drops are deep.** Freeing a vector frees its elements, and every example runs clean under
AddressSanitizer. A crash under `vibe run` (`error[run.signal]`) is a compiler defect to
reduce and report, not a flake: the ones found writing `lib/` were a payload moved out of a
match, a self-tail-call inside a nested match, and an argument of a call sitting in a borrow
position, each freed twice.

**Anything that may alias is not freed at all** — a prelude result that points into an
argument (`get`, `max_by`, `min_by`, `sum`, `fold`, `seq`, `to_cstr`), a capture a callee
may keep, a pointer handed to C. `vibe view --drops` counts the suppressions.

**Handing a pointer to C taints the function and every caller**, because the release happens
at the outermost frame.

**Every closure is on the heap.** `map (\x -> …)` costs a `malloc`/`free` pair per call plus
one per element. Not a leak — a cost. Stack allocation for a non-escaping closure is
planned, not done.

**`arena a in expr`** releases everything the block allocated when it ends. It is the manual
override for a tail-recursive loop, whose iterations otherwise accumulate until the frame
returns. An arena block does not make a non-decreasing recursion total.

**Values are dynamically tagged** and resolved types are not threaded into codegen, so any
performance claim today is a claim about a boxed interpreter. `runtime/vibert.h` says so in
its own header comment.

**Mutual tail calls are not guaranteed.** `[[gnu::musttail]]` cannot be emitted as the
calling convention stands. Self-tail-recursion *is* turned into a loop directly by the
compiler.

---

## Modules

**A field name shared by two records needs the base's type to be inferable.** Fields are
resolved per use site from the base's type, so two modules *may* both declare `qty` — but a
site where inference could not decide is `field.ambiguous`, not a coin toss. Write the type
on the parameter or the signature.

**`ext c` symbols are still a shared namespace.** Two modules may name the same C function
— it is the same function — but they may not give it two types, because the linker takes
one and the checker proved something about the other.

**Module names do not nest.** `Std.Json` is not a path.

**The file name must match the module name** up to case and underscores, and nothing else —
`TotalOk` may live in `total_ok.vibe`, and that is the full extent of the tolerance.

**`vibe deps` reports callees across module boundaries but callers only within the root
module**, because the other direction would mean walking every unit.

---

## Working on the compiler itself

**`cargo test -- --test-threads=1`.** The integration tests share a build directory and
race when run in parallel. CI pins the single-threaded invocation; anything else produces
failures that are not real.

`cargo clippy --all-targets -- -D warnings` is expected to be clean.

`src/codegen.rs` reaches into `own::inplace_updates` and `escape::releasable` from inside
`generate()`. Harmless today, a layering violation the moment there is a second backend.

The `.vibe` corpus under `tests/` is walked by `tests/canon.rs`, which asserts every file
checks — except an explicit list of files that exist in order to be *rejected*. Adding a
`.vibe` file there joins that corpus.

---

## Where to look next

- `docs/remaining-work.md` — the authoritative list, ordered by value over cost, with a
  file and a line for every item. Read it before starting anything large.
- `docs/aliasing-audit.md` — the eight aliasing gaps and what closing each would cost.
- `docs/static-drop-roadmap.md` — how the garbage collector never happened.
- `docs/backend-roadmap.md` — the Cranelift plan, and its own finding that Cranelift alone
  does not remove the C dependency.
- `vibelang-spec.md` §16 — the design questions the spec itself leaves open.

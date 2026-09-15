![Vibelang](assets/banner.svg)

# Vibelang

### The compiler is the code review.

Vibelang is a statically typed **programming language designed for LLMs to write** — it compiles to portable C99, and its compiler proves what a reviewer would otherwise have to read. Refinement types discharged by an SMT solver, affine ownership with no garbage collector and no lifetime annotations, mandatory termination, one canonical spelling per program.

Every pure function must prove that it terminates. Every division, index and overflow that could go wrong is discharged by z3 before the binary exists. You are still the reviewer — the compiler just got there first, and it does not get tired at four in the afternoon.

Early, and honest about it: the [Status](#status) section is three short lists.

---

## The part that should not compile

**Two signatures, zero runtime checks. The empty vector was eliminated three lines earlier, and the solver noticed.**

```
mean (ts:&Vec Tx, len ts>0) : F64 = total ts / f64 (len ts)
top  (ts:&Vec Tx, len ts>0) : Tx  = ts |> max_by amt

load (p:&Str) : E! Res Err (Vec Tx) =
  txt <- read p ;
  ?txt |> lines |> map parse |> seq
   |Er e  -> Er e
   |Ok [] -> Er Void
   |Ok ts -> Ok ts
  end
```

`len ts>0` is part of the type. The `|Ok []` arm is not defensive programming — it is the proof step: it tells the solver that `ts` in the arm below it cannot be empty, so `mean ts` needs no check, emits no check, and cannot divide by zero.

```console
$ vibe check --prove examples/ledger.vibe ; echo $?
0
$ vibe run examples/ledger.vibe
n=3 tot=20.75 avg=6.91667 top=b
```

Forty-five lines for the whole program: a CSV parser, an error type, record invariants, a proof, and a C ABI surface. It is [`examples/ledger.vibe`](examples/ledger.vibe). Read it — it is shorter than this README.

## What it refuses to compile

Five classes of wrong, all caught before the binary exists:

- **`match.nonexhaustive`** — a `match` with a hole in it, with the missing constructor named.
- **`own.use_after_move`** — a value used twice when it was only ever yours once.
- **`total.no_measure`** — a *pure* function that has not proved it terminates. Looping forever is an effect; only `E!` may do it.
- **`refine.unproven`** — a precondition the solver could not discharge, with a counterexample.
- **`canon.parens` / `canon.form`** — source that is not in the one canonical spelling. There is no style debate because there is no style.

Here is the fourth one in full, because the shape of the answer is the point:

```console
$ vibe check --prove avg.vibe
error[refine.unproven]: cannot prove the precondition of `mean`: len xs > 0
  --> avg.vibe:5:30
   |
   | report (xs:&Vec F64) : F64 = mean xs
   |                              ^^^^
   = counterexample: len_xs=0, xs=0
   = fix: Avg.report.sig += len xs > 0
```

## 60 seconds

No dependencies: the compiler is a single Rust crate with an empty dependency graph. You need a Rust toolchain, a C compiler, and `z3` on PATH if you want proofs.

```bash
git clone https://github.com/NicoNex/vibelang && cd vibelang
cargo build
./target/debug/vibe run examples/ledger.vibe
```

```
vibe check   file.vibe [--prove]   types, ownership, totality, refinements
vibe build   file.vibe [--lib]     a native executable, or a static library plus a C header
vibe run     file.vibe -- args     build and run
vibe view    file.vibe [--drops]   the canonical form, or where every value dies
vibe fmt     file.vibe [--check]   write the canonical form back; --check fails instead
vibe proof   file.vibe             open obligations, by semantic path
vibe deps    file.vibe             callers and callees
vibe patch   file.vibe <path>      hash-guarded structural edit, refused if it stops compiling
```

## Errors addressed to a machine

**That is not an error message. That is the patch, addressed to something that does not need to be persuaded.**

```console
$ vibe check --diag=json move.vibe
[{"path":"Move.twice","code":"own.use_after_move","msg":"`s` was already moved",
  "file":"move.vibe","line":5,"col":30,"witness":"first moved at line 5, column 28",
  "fix":"borrow it here with `&s`, or copy it with `dup s`"}]
```

Every diagnostic carries three things: a **path** that names the node semantically instead of by line number, a **witness** that says why, and a **fix** mechanical enough to apply without understanding it. An agent does not need prose. It needs the next edit.

## Memory

**Two million allocations. Nothing retained. 1.4 MB peak, no GC, no refcount, and not one `free` in the source.**

```console
$ /usr/bin/time -l ./loop
4000000
        1474560  maximum resident set size
```

[`examples/loop.vibe`](examples/loop.vibe) allocates on every iteration of a loop that never returns. The ownership checker already knows where each value stops being yours, and that point is where the `free` goes: end of scope, end of the arm that still held it, or on the back edge of the loop just before the parameter is overwritten. The same program, before implicit drops landed, peaked at 197 MB.

What is *not* freed is anything that might be someone else's — a pointer into an argument, a capture a callee kept, anything handed to C. `vibe view --drops` prints the table and counts every suppression, because that count is the distance between what gets freed and what could be.

## C, both directions

C interop is not an escape hatch bolted on afterwards; it is how Vibelang inherits forty years of libraries.

```
ext c "stdio.h"
  puts : &CStr -> E! I32
end

exp c mean, total
```

`ext c` calls C. `exp c` emits the C ABI surface and a header, so C calls you — and the header says out loud where the guarantees stop:

```c
/* mean — pre: len ts > 0   NOT VERIFIED ACROSS THE BOUNDARY */
double Ledger_mean(VbVal x0);
```

## Status

**Works** — Hindley–Milner inference, ADTs, records, exhaustive matching, effects (`E!`), affine ownership, termination checking, refinement obligations discharged with z3 and cached, implicit drop with no GC, bit operations, a deep and generic `dup`, C emission and its runtime, `ext c` / `exp c`, multi-file programs where every module is its own namespace, four projection views, the whole CLI above. 145 tests, green.

**Partial** — refinements on prelude builtins are hardcoded; a proof about a float is a proof about a mathematical real; drops are shallow, so nested structures leak their interior; every closure is on the heap; values are dynamically tagged, so any performance claim today is a claim about a boxed interpreter.

**Not yet** — no map and no set, so nothing associates a key with a value; record fields and `ext c` symbols are the one namespace still global; a native backend.

The full list, with a file and a line for every item, is [`docs/remaining-work.md`](docs/remaining-work.md). Moving a line from one list to the next is how this section gets edited.

## Read more

- [`vibelang-spec.md`](vibelang-spec.md) — the normative specification. Also in [Italian](vibelang-spec.it.md).
- [`docs/remaining-work.md`](docs/remaining-work.md) — what is not done, ordered by value over cost.
- [`docs/static-drop-roadmap.md`](docs/static-drop-roadmap.md) — how the garbage collector never happened.
- [`examples/`](examples) — the reference program, a grep, a loop, a C binding.

## License

GPL-3.0. See [LICENSE](LICENSE).

![Vibelang](assets/banner.svg)

# Vibelang

<sub>Italiano: [README.it.md](README.it.md)</sub>

**A programming language that is not for you.**

Vibelang is not designed to be pleasant to write. It is designed to be *generated*: one canonical form per program, no syntactic sugar, a type checker that rejects what it cannot account for, and one metric the rest of language design has never optimized for — the total tokens burned, generation plus diagnostics plus retries, before a program is correct.

The target audience is a statistical model. It has no opinions about brace placement, never opens an issue about the ternary operator, and does not need onboarding. Designing for it is, in that narrow sense, a relief.

You are the reviewer now, not the author. If that sounds like a demotion, you have understood the project correctly.

```
mod Ledger

parse (ln:&Str) : Res Err Tx =
  ?split ',' ln
   |[s,q,p] -> ?(parse_u32 q, parse_f64 p)
                |(Some n, Some v) -> mk (dup s) n v
                |_                -> Er (Num (dup ln))
               end
   |_       -> Er (Bad (dup ln))
  end
```

This is not C with the sugar removed. It is the lowest-variance syntax possible for a statistical generator, bolted to a compiler that turns a `.vibe` file into a native executable through C. That compiler exists, and it runs:

```console
$ vibe run examples/ledger.vibe
n=3 tot=20.75 avg=6.91667 top=b
```

There is no model in this repository. No API key, no inference, no chat window, nothing that autocompletes. Vibelang is the unglamorous half of the arrangement: the part that reads what the machine wrote and tells it, in a stable machine-readable format, that it is wrong.

It is early. The [Status](#status) section says exactly how early, in the only useful form: a list of what works, a list of what half-works, and a list of what is still paper.

---

## If none of that meant anything

No prerequisites below. Nothing is simplified either — it is only unpacked.

A **programming language** is the notation instructions are written in. A **compiler** is the program that turns that notation into something a processor can actually run, and — the half that matters here — refuses to do so when the instructions do not add up. Most of a compiler's working life is spent saying no. Vibelang's is spent saying no more often, on purpose.

**The premise.** Every language you have heard of was designed around a person writing it — decades of research into which notation humans find readable, memorable, forgiving. Vibelang assumes the first draft is written by a machine, and read by a person who then approves it or sends it back. Every choice on this page is that one inversion, followed to the end.

**One way to write each thing.** Most languages let you spell the same idea three or four ways, and taste picks between them. A generator has no taste; it has a probability distribution. Two equally valid spellings is a coin flip, and a coin flip in the middle of a program is a bug waiting for its turn. So the parser rejects the variants instead of accepting them and tidying up afterwards: parentheses you did not need are an error, a match that is not closed is an error. Other languages ship a formatter, which is a tool for forgiving you. Vibelang's formatter is the error message.

The one thing the parser is *not* strict about is whitespace, and that is the same argument running the other way. Indentation is not structure here: a match ends at `end`, a binding ends at `;`, and any layout that puts the tokens in that order is the same program. A model that miscounts spaces has not made a mistake about the program, so it should not pay a retry for one.

**Four things it will not compile.** In plain terms:

- **A choice with a case missing.** The code handles red and green; blue also exists. Most languages will let that ship, and fall over the first time blue turns up. Vibelang refuses the file and names blue.
- **Using a value after you gave it away.** Every value has exactly one owner. Pass it on and you no longer hold it; using it again is selling the same car twice, and the compiler is standing there at the second sale.
- **A loop that might never stop.** Anything *pure* that calls itself has to exhibit some quantity that strictly shrinks at every call, so the bottom is reachable. A function marked as touching the outside world is exempt, because a server is supposed to run for ever — the signature says which kind you are looking at.
- **Dividing by something that might be zero.** The checked operations hand back a result that is either a number or a failure, and there is no way to look away from the failure.

The load-bearing word is **before**. None of these are run-time checks that fire once the bad case finally arrives. They are settled while nothing is running, about every run that could ever happen. The industry default is to learn the same facts at three in the morning, from a customer, or from neither — and a test only covers the cases somebody thought to write down.

(The strongest form of the fourth one — proving the divisor is never zero, so that no check is needed at all — is done by an external solver and is opt-in today. [Status](#status) is exact about which guarantees are which, which is the entire reason that section exists.)

**Errors addressed to a machine.** An ordinary compiler error is a complaint: here is what is wrong, good luck. A Vibelang diagnostic carries a `fix` field, which is not advice but the edit — the exact text to add, and where it goes. The difference is about who is reading. A person bridges the gap between "this match is not exhaustive" and the repair using judgement, and barely notices doing it. A generator pays for that gap with an entire new attempt: regenerate, recompile, re-read the complaint, hope. A `fix` closes the loop inside one diagnostic instead. For the same reason each error is addressed by semantic path (`Color.show_c.match`) rather than by line number — the generator's own last edit has already moved the lines.

**Why it compiles to C.** Vibelang does not emit machine code itself. It emits C, and hands that to the C compiler already sitting on your machine. C is the front door of nearly every library that exists — image decoders, databases, cryptography, the operating system underneath all of them — so going through C inherits sixty years of other people's work instead of spending its first decade rebuilding it badly. The cost is a dependency on a C compiler. The alternative was a language that can do arithmetic and nothing else.

None of these choices makes the language nicer to type. That is not an oversight. It is the design, and you are on the other side of it.

---

## Why it exists

The languages we use have been optimized for sixty years for one thing: being pleasant for a human to write and read. Syntactic sugar, several ways to say the same thing, conventions inferred from context. An experienced programmer loves all of it. A language model pays for all of it — every syntactic ambiguity is a fork where the generator can take the wrong turn, and every wrong turn is another retry, which is more tokens, more latency, more money.

All of that research is hospitality, and the guest has stopped turning up to write the first draft.

So Vibelang asks a different question. Not "how convenient is this to write", but **how many tokens does it take, end to end, to arrive at a correct program.** Not source brevity. The cost of the generate-compile-fix loop.

Every design decision falls out of that:

- **exactly one valid form per program** — the parser *rejects* variants instead of normalizing them, because if there are two ways to write a thing, the generator has to choose, and choosing is where errors come from;
- **no contextual rules** — what a token means does not depend on where it sits in the file;
- **failure at compile time, not at three in the morning** — a non-exhaustive `match`, a use-after-move, an unproven precondition are errors, not surprises;
- **diagnostics with a mechanically applicable `fix` field** — so the next attempt is an edit, not a guess.

The style guide is therefore empty, and the formatter is the parser refusing the file. The bikeshed is a parse error.

Human readability is not ignored. It is a *derived* goal, to be served by projection tools rather than carved into the syntax. You read the rendering; the generator writes the canonical form. (Those tools are `vibe view`. See [Status](#status).)

---

## What the compiler enforces today

This is the section where a language README usually lists adjectives. Here it is a list of things that will stop you, all of them checked by the code in `src/` and covered by `cargo test`.

- **Canonical structure, enforced not normalized.** Redundant parentheses are an error; an unclosed `?` match or `ext c` block is an error; a `<-` binding without its `;` is an error. The parser rejects and hands back a `fix`; it never quietly reformats. Layout is deliberately excluded: whitespace does not reach the AST, so indentation, blank lines and the column a declaration starts in are all free.
- **Hindley–Milner inference.** The classical algorithm that deduces every type from how a value is used instead of being told. Full inference over ADTs — *algebraic data types*, a type declared as a fixed list of alternatives, each allowed to carry data — with payloads, records, tuples and lists. Signatures are declared; bodies are inferred.
- **Exhaustive pattern matching.** A missing constructor is a compile error that names the constructor and the arm to add.
- **Effects, propagated not inferred.** A function is pure until it is marked `E!`. Calling something effectful from a pure function is an error; the compiler will not quietly promote you.
- **Termination, for the pure fragment.** Every recursive *pure* function needs a measure that decreases at each call. The compiler infers it when a parameter decreases syntactically, and asks for `%expr` when it cannot. Divergence is an effect: an `E!` function may recurse for ever, which is how an event loop is written without the language growing a `while`.
- **Affine use of owned values.** *Affine* means a value may be used once and not twice. `&` parameters are borrows — lent for the duration of the call and still yours afterwards; everything else is owned and consumed by its first use. Using it twice is an error whose `fix` is `&x` or `dup x`. A `{r with ...}` update on a uniquely owned record mutates in place instead of copying.
- **Arithmetic and indexing that can fail in the type system.** `add_checked`, `sub_checked`, `mul_checked`, `div_checked` and `get_checked` return `Res Fault a`, so overflow, division by zero and an out-of-range index are values you have to match on. (Bare `+` is still unchecked; the spec's plan is for the proof solver described below to discharge these obligations instead.)
- **Machine-first diagnostics.** `--diag=prose` for you, `--diag=struct` and `--diag=json` for whatever is generating the code.
- **A native binary.** Codegen emits C, `cc` links it against a small C runtime, and you get an executable. No GC, no VM, nothing on the target beyond libc and libm.
- **An explicit C boundary.** `ext c` imports C declarations with `link` / `pkg-config` wiring; `exp c` emits C-ABI symbols and a header.

### Designed, not yet enforced

The specification describes more than the compiler currently proves. Every project has this list; most call it the roadmap, phrase it in the future tense and move it to the bottom of the page. Here it sits directly under the feature list, because the distance between the two is what a type checker is for:

- **Refinement types** — a type carrying a condition it has to satisfy, so `mean (ts:&Vec Tx, len ts>0)` reads "a vector of transactions, and it is not empty". They parse and type-check as boolean expressions in the parameter scope, and are **asserted at run time** unless you ask for proof.
- **Refinement obligations** are generated for division, indexing, overflow, record invariants and call-site preconditions, and `vibe check --prove` discharges them with `z3` — an SMT solver, which is a program that decides whether a set of arithmetic and logical facts can all hold at once, and therefore whether a condition already follows from what is known. Without `--prove`, `vibe check` only counts what is left open. A constructor pattern carries its payload into the solver, so an earlier `Ok [] ->` arm is what discharges a later `len ts > 0` — no explicit check, which is the point of the reference program.
- **Memory.** Allocation is a bump allocator — a pointer that walks forward and never back — and most of what it hands out now comes back on its own ([Memory](#memory) explains how). The half §4.6 asks for and does not get is the free one: a closure that cannot outlive the call that made it should live on the stack, and today every closure is on the heap. Nor is there a full borrow checker — affine use is checked, the aliasing rules beyond it are not.

---

## Diagnostics

Most compilers are written as if apologizing to a human. Vibelang files a report.

Prose by default:

```console
$ vibe check color.vibe
vibe: error[match.nonexhaustive]: this match does not cover Blue
  --> color.vibe:6:3
   |
   |   ?k |Red   -> dup "red"
   |   ^
   = counterexample: missing Blue
   = fix: add `|Blue -> ...`
```

The same content as a term, keyed by a stable semantic path instead of a line number:

```console
$ vibe check color.vibe --diag=struct
✗ Color.show_c.match match.nonexhaustive ⊨ missing Blue
  at: color.vibe:6:3
  msg: this match does not cover Blue
  fix: add `|Blue -> ...`
```

And as JSON, for whatever wrote the file:

```console
$ vibe check color.vibe --diag=json
vibe: [{"path":"Color.show_c.match","code":"match.nonexhaustive","msg":"this match does not cover Blue","file":"color.vibe","line":6,"col":3,"witness":"missing Blue","fix":"add `|Blue -> ...`"}]
```

Three fields carry the design:

- `path` — a semantic address (`Color.show_c.match`) rather than a coordinate, so a generator does not have to re-anchor on line numbers its own last edit invalidated;
- `witness` — the concrete thing that went wrong, never a category;
- `fix` — present whenever the repair is mechanical, so the loop closes inside one diagnostic instead of a blind retry.

Ownership errors have the same shape:

```console
$ vibe check tests/move.vibe --diag=struct
vibe: ✗ Move.twice own.use_after_move ⊨ first moved at line 5, column 28
  at: tests/move.vibe:5:30
  msg: `s` was already moved
  fix: borrow it here with `&s`, or copy it with `dup s`
```

---

## Status

Honest state of `main` today. Moving a line from one list to the next is the intended way to edit this section. No percentage complete, no progress bar, no quarters.

**Works**

- lexer with no offside rule — whitespace does not reach the AST; parser enforcing the structural half of canonicity; AST; name resolution
- Hindley–Milner type checker; ADTs; records; exhaustive pattern matching
- effect propagation (`E!`)
- structured diagnostics (`--diag=prose|struct|json`) with semantic path, witness and mechanical `fix`
- C code generation, and a small C runtime
- `ext c` FFI with `link` / `pkg-config`, and `exp c` export with a generated header
- `vibe build --lib`: a static archive plus that header, linkable from C with no initialisation call
- the `vibe` CLI: `check`, `build`, `run`, `view` — a `.vibe` file to a native executable via C
- the agent surface of §13.3: `vibe patch` (semantic path, hash-guarded, refused unless the result still compiles), `vibe deps` (callers and callees), `vibe proof` (open obligations by path)
- termination checking for the pure fragment: inferred measures, `%expr` when inference gives up, and `E!` functions exempt because divergence is an effect (§6.1)
- refinement obligations generated for §7.2 and discharged with `vibe check --prove` (needs `z3` on PATH), with proved obligations cached in a sibling `.vibe-proofs` by the hash of the question asked, and a per-obligation solver budget (`--prove-timeout=`, 5 seconds by default) that reports giving up instead of pretending to refute (§16.5)
- the projection views: `vibe view` (canonical form, byte-identical on every `.vibe` file in the repository), `--sig-only`, `--explicit`, `--flow`. Comments are anchored to tokens, so reformatting a file laid out any other way keeps them
- escape analysis for closures — deciding which values outlive the call that built them: a closure whose value reaches the result owns its captures, one consumed during the call reads them
- `arena a in ...` blocks: a named region you allocate into and discard whole, which releases everything it allocated when it ends
- automatic release per frame: a function that cannot hand a pointer to C releases everything it allocated when it returns, and the runtime cancels the release when the result is itself heap-allocated
- refinements of values bound by a constructor pattern: an arm learns the constructor tag, the payload's length, and the payload's record invariant
- multi-file programs: `Money.cents` is the whole import system, resolved by loading `money.vibe` next to the file that names it, with a flat namespace and a clash reported rather than shadowed (§9)
- the spec's Appendix A reference program compiles and runs

**Partial**

- **ownership** — affine use checking of every owned name: parameters, `let` and `<-` binders, names a pattern binds, and the captures of a closure that outlives its call. In-place update of uniquely owned records. What is missing is the allocation half of §4.6: a non-escaping closure should live on the stack, and today every closure is heap-allocated.
- **refinements** — discharged with `vibe check --prove`, asserted at run time otherwise. `--prove` is opt-in rather than the default, and it needs `z3` on PATH.

**Not yet**

- stack allocation for a closure that does not escape (§4.6 wants zero cost; the ownership half of that rule is done, the allocation half is not)
- a postcondition on a function's result. §7.1 puts refinements on parameters only, which is why the reference program's `mean &ts` in `main` is still an open obligation: the fact that rules out the empty case lives in `load`, and there is no way to say so (§16.8)
- implicit drop — see [`docs/static-drop-roadmap.md`](docs/static-drop-roadmap.md) — and a native backend that needs no C toolchain, see [`docs/backend-roadmap.md`](docs/backend-roadmap.md)

Several of these are being worked on in parallel, so this list moves faster than the prose above it.

The compiler is blunt about its own maturity at run time, too, which is more than most of us manage:

```console
$ vibe run refine.vibe
✗ Refine.half.pre refuted ⊨ n > 0
  this obligation is checked at run time in the bootstrap; fase 6 discharges it statically
```

---

## Try it in 60 seconds

No dependencies — the compiler is a single Rust crate with an empty dependency graph. You need a Rust toolchain and a C compiler.

```bash
git clone <this-repo> vibelang
cd vibelang
cargo build
```

Run the reference program. It reads `ledger.csv` from the working directory, so run it from the repo root:

```console
$ ./target/debug/vibe run examples/ledger.vibe
n=3 tot=20.75 avg=6.91667 top=b
```

Now watch the loop close. Write a `match` with a hole in it:

```bash
cat > color.vibe <<'EOF'
mod Color

type Color = Red | Green | Blue

show_c (k:&Color) : Str =
  ?k |Red   -> dup "red"
     |Green -> dup "green"
  end

main : E! Unit =
  out (show_c &Red)
EOF
```

```console
$ ./target/debug/vibe check color.vibe --diag=struct
vibe: ✗ Color.show_c.match match.nonexhaustive ⊨ missing Blue
  at: color.vibe:6:3
  msg: this match does not cover Blue
  fix: add `|Blue -> ...`
```

The `fix` field is the whole product. It is not advice, it is an edit, and it is addressed to something that does not need to be persuaded.

Apply that `fix` — add `|Blue  -> dup "blue"` under the other arms — and the same file builds and runs:

```console
$ ./target/debug/vibe run color.vibe
red
```

Then take an executable away with you, and keep the C if you want to read it:

```console
$ ./target/debug/vibe build examples/ledger.vibe -o ledger --emit-c
$ ./ledger
n=3 tot=20.75 avg=6.91667 top=b
```

The whole CLI is seven commands, three of which exist for the generator rather than for you. No dashboard, no plugin directory, nothing to log into:

```
vibe check <file.vibe>            check only; silent on success
vibe build <file.vibe> [-o out]   emit C, compile, link
vibe run   <file.vibe> [-- args]  build and execute
vibe view  <file.vibe>            print the canonical form, or a projection of it
vibe deps  <file.vibe>            callers and callees, one line each
vibe proof <file.vibe>            the open refinement obligations, by semantic path
vibe patch <file.vibe> <path> [<hash> <node>]
                                  read a node, or replace it if the hash still matches

  --diag=prose|struct|json        diagnostic rendering (default: prose)
  --prove                         discharge obligations with z3 (§7.3)
  --prove-timeout=<secs>          solver budget per obligation (default 5)
  --sig-only|--explicit|--flow    which projection to print (view)
  --lib                           build a static library and its C header
  --emit-c                        keep the generated C next to the output
```

`cargo test` runs the end-to-end suite, which is the most reliable description of what the compiler actually does: every example must check, the reference program must produce exactly the output above, a type error must report `type.mismatch`, a double move must report `own.use_after_move`, an owned `{r with ...}` must not copy, a frame that cannot leak must release what it allocated, and a C program must link the library `--lib` builds.

---

## The reference program

Appendix A of the specification, and the program every implementation phase has to keep compiling. It reads a CSV of transactions, validates each row, and prints count, total, mean and the largest row — with no explicit empty-list check in `mean` or `top`.

```
mod Ledger

ext c "stdio.h"
  puts : &CStr -> E! I32
end

type Tx  = { sku:Str, qty:U32, price:F64, qty>0, price>0.0 }
type Err = Bad Str | Num Str | Void

parse (ln:&Str) : Res Err Tx =
  ?split ',' ln
   |[s,q,p] -> ?(parse_u32 q, parse_f64 p)
                |(Some n, Some v) -> mk (dup s) n v
                |_                -> Er (Num (dup ln))
               end
   |_       -> Er (Bad (dup ln))
  end

mk (s:Str) (n:U32) (v:F64) : Res Err Tx =
  ?(n>0 && v>0.0)
   |True  -> Ok {sku=s, qty=n, price=v}
   |False -> Er (Bad s)
  end

amt   (t:&Tx)      : F64 = t.price * f64 t.qty
total (ts:&Vec Tx) : F64 = ts |> map amt |> sum

mean (ts:&Vec Tx, len ts>0) : F64 = total ts / f64 (len ts)
top  (ts:&Vec Tx, len ts>0) : &Tx = ts |> max_by amt

load (p:&Str) : E! Res Err (Vec Tx) =
  txt <- read p ;
  ?txt |> lines |> map parse |> seq
   |Er e  -> Er e
   |Ok [] -> Er Void
   |Ok ts -> Ok ts
  end

main : E! Unit =
  r <- load "ledger.csv" ;
  ?r |Er e  -> warn (show e)
     |Ok ts -> out (fmt "n={} tot={} avg={} top={}"
                        (len ts) (total &ts) (mean &ts) (top &ts).sku)
  end

exp c mean, total
```

`mean` and `top` demand `len ts > 0` in their signatures, and nothing in `main` checks it. The design intent is that `load` has already ruled out `Ok []`, that this fact enters the solver's context on the `Ok ts` branch, and that a redundant check would itself be a dead-branch error.

Be clear about how much of that is real *today*: `vibe check --prove` discharges most of it, and the two obligations in `main` are still open, because proving them needs `main` to know what `load`'s `Ok []` branch already ruled out — a postcondition, which spec §7.1 has no syntax for. This program is the target that keeps the pipeline honest, and the [Status](#status) section says exactly where it stands.

One deviation from the published spec: the draft writes `u32` and `f64` as parsing conversions overloaded on strings. Vibelang has no overloading — a language principle, not an implementation gap — so string parsing is `parse_u32` / `parse_f64` : `&Str -> Opt U32` / `&Str -> Opt F64`. The code above is the corrected form.

---

## Modules

One file, one module, and the name is the import:

```
mod App

gross (euro:F64) : I64 = Money.vat (Money.cents euro) 22
```

`Money.vat` resolves by loading `money.vibe` from the same directory. There is no `import` line, no alias and no search path, which removes a construct and, more to the point, a place for a generator to guess. Spec §9 gives v0.1 no visibility system at all, so the loaded modules are flattened into one unit and a name declared twice is a `mod.duplicate` error rather than a silent shadow. Diagnostics keep the module that owns the code, so an obligation raised inside `Money` is still addressed as `Money.half.body/0` from a run rooted at `App`.

This does not scale past a small project, and the spec says so itself. It is the smallest thing that makes two files work.

---

## Memory

Allocation is a bump allocator. Freeing is not reference counting and not a garbage collector; it is two questions about whether anything a frame allocated got out of it.

The first is asked by the runtime, dynamically and for free: a frame refuses to release when its own result is a string, object, vector or closure, because that value is exactly what escaped. The second is asked statically, and there is only one thing to ask, because Vibelang has no globals and no mutation of borrowed values: *did this frame hand a pointer to C?* C may keep it for as long as it likes. That taint travels to callers, since the release happens at the outermost frame.

Everything else releases on return. `examples/churn.vibe` builds and discards a five-thousand-element vector two thousand times:

```console
$ /usr/bin/time -l ./churn
10000000
        1556480  maximum resident set size
```

The same program with the release suppressed peaks at 325 MB. The number that matters is not the ratio, it is that it is flat: the loop no longer grows.

The ceiling is whole-function granularity. A tail-recursive loop marks once and releases once, so its own iterations still accumulate until it returns; `arena a in ...` is the manual override for that case, and a mark per iteration is the upgrade path.

---

## The C boundary

The C boundary is deliberately the one place where the guarantees stop, and it has to be visible at a glance rather than buried in a generated binding.

```
ext c "sqlite3.h" link "sqlite3"
  sqlite3_open : &CStr -> Ptr Db -> E! I32
  sqlite3_exec : Ptr Db -> &CStr -> E! I32
end
```

`link` names a library for the linker; `pkg` resolves one through `pkg-config`. Every `ext c` signature is mandatorily `E!`: the checker cannot know what a C function does, so it assumes the worst by construction.

An `ext` signature is a *type*, not a parameter list — there are no names to the left of the arrows, so a refinement written on one has nothing to refer to. The spec draft wrote `(n:Size, n>0) -> E! I32`; that is not a type, and the parser says so. A precondition on an imported C function is therefore still only expressible at the Vibelang call site.

Going the other way:

```
exp c mean, total
```

emits C-ABI symbols and a header. Preconditions travel into the header as a comment, with the warning attached:

```c
/* mean — pre: len ts > 0   NOT VERIFIED ACROSS THE BOUNDARY */
double Ledger_mean(VbVal x0);
```

Shouting it in a comment is not verification. It is the most a header can do, and pretending otherwise is how guarantees leak out of a project.

A module with no `main` is a library, and `vibe build --lib` says so out loud:

```console
$ vibe build --lib examples/mathlib.vibe
libmathlib.a
$ cc -std=c11 -o use use.c -L. -lmathlib -lm
```

The archive arrives with its generated header beside it. The exported wrappers initialise the runtime themselves, so there is no `vibe_init()` for the C side to forget. This is the direction the project cares about most: C is not an escape hatch bolted on at the end, it is how sixty years of existing libraries stay reachable without anyone rewriting them.

The exported signature currently passes the uniform runtime value `VbVal`. The spec's `own T` / `ref T` ownership qualifiers, and the generated `<name>_free`, are not implemented yet.

---

## Further reading

- [`vibelang-spec.md`](vibelang-spec.md) — the normative specification: goals and non-goals (§0), normative principles (§1), grammar and canonicity rules (§3), ownership (§4), effects (§5), totality (§6), refinements (§7), C interop (§10), implementation phases (§15). Also in Italian: [`vibelang-spec.it.md`](vibelang-spec.it.md).
- [`docs/backend-roadmap.md`](docs/backend-roadmap.md) — the plan for a Cranelift backend beside the C one, so a pure Vibelang program needs no C toolchain. C is not deprecated by it: `exp c` headers and `--emit-c` are the reason it stays.
- [`docs/static-drop-roadmap.md`](docs/static-drop-roadmap.md) — the plan for deleting the bump allocator. The ownership checker already knows where every value dies; Static Drop is the work of emitting that knowledge instead of discarding it.
- [`README.it.md`](README.it.md) — this page in Italian.
- `examples/` — the reference program and a hello world. `tests/` — the end-to-end suite, which is also the most reliable description of what the compiler actually does.

---

## License

GPL-3.0-or-later — see [LICENSE](LICENSE). The language surface can still change; the license will not.

---

*The text editor is obsolete. You are the reviewer. The compiler cuts neither of you any slack.*

---
name: vibelang
description: >-
  Write, fix and verify Vibelang — the statically typed language in this repository that
  compiles to portable C99 with refinement types discharged by z3, affine ownership, and
  mandatory termination proofs. Use this skill whenever a `.vibe` file or the `vibe` CLI is
  involved: writing or editing any Vibelang source, adding an example or a test program,
  designing a signature, or fixing any compiler diagnostic (`refine.unproven`,
  `own.use_after_move`, `total.no_measure`, `match.nonexhaustive`, `canon.parens`,
  `effect.missing`, `ffi.type`, `mod.missing`). Also use it for `ext c` / `exp c` bindings,
  for `vibe check --prove` / `run` / `fmt` / `view` / `proof` / `deps` / `patch`, and when
  reviewing or explaining Vibelang someone else wrote. Vibelang looks like ML or Rust and is
  not either of them — consult this before writing even three lines, because guessing the
  syntax from the family resemblance reliably produces code that does not compile.
---

# Vibelang

Vibelang is designed to be written by a machine and checked by a compiler. The refusals are
the feature: a program that compiles has already been proved total, memory-safe without a
GC, exhaustive, and free of the division, index and overflow faults a solver could find.

`vibelang-spec.md` at the repo root is normative. This skill is the working subset, plus
the places where today's compiler is narrower than the spec. Everything here compiles
against the compiler in this tree.

## The loop — never hand over unverified Vibelang

```bash
cargo build                        # produces ./target/debug/vibe
vibe fmt    f.vibe                 # write the canonical form back to the file
vibe check --prove f.vibe          # types, ownership, totality, refinements
vibe run    f.vibe -- args         # build and run
```

`vibe check` on its own leaves refinement obligations open (it prints how many) and turns
them into run-time asserts. **`--prove` is the real check, and it is stricter than you
expect**: it also demands that every `+`, `-`, `*` on a machine integer stays in range.
Several files in this repository's own test corpus pass `vibe check` and fail
`vibe check --prove` for exactly that reason. Decide which bar the task wants, and say
which one you actually met.

`z3` must be on `PATH` for `--prove`. Proof certificates are cached in a sibling
`.vibe-proofs` file.

## Shape of a file

There is **no offside rule**. Indentation, blank lines and the column a declaration starts
in carry no meaning — a generator that miscounts spaces still gets a program. What closes
things is delimiters: `end` closes a `match` and an `ext c` block, `;` terminates a `<-`
bind. The one surviving layout rule is that a newline ends a top-level declaration, so two
declarations may not share a line.

```
mod Shape

;; `;;` runs to the end of the line. An ADT: constructors are capitalised.
type Kind = Dot | Line F64

;; A record, and the trailing expression is its invariant.
type Seg = { a:F64, b:F64, b>a }

;; `&` borrows for the duration of the call; everything else moves.
span (s:&Seg) : F64 = s.b - s.a

;; `?` opens a match, `|` opens an arm, `end` closes the whole thing.
size (k:&Kind) : F64 =
  ?k |Dot    -> 0.0
     |Line n -> n
  end

;; `E!` marks a function that touches the world.
main : E! Unit =
  out (fmt "{} {}\n" (size &(Line 3.0)) (span &{a=1.0, b=4.0}))
```

**One canonical spelling.** The parser rejects redundant parentheses around an atom
(`f (n:U64) : U64 = (n)` is `canon.parens`) and a `match` or `ext c` without `end`.
`canon.form` fires when the file is not what `vibe view` would render — mostly a compiler
self-check, but it does catch real source, `f (-2.5)` being the one you will meet. Layout
the parser does *not* reject; `vibe fmt` settles it and `vibe fmt --check` fails on
anything else.

Practical rule: **write it, then run `vibe fmt`, then read back what it wrote.** That is
the canonical form, there is no style debate to have, and reading it back is also how you
catch a silent misparse.

Names: `[a-z][a-zA-Z0-9_]*` for values, `[A-Z][a-zA-Z0-9_]*` for types and constructors.
Keep them descriptive — that is a deliberate design choice, not a preference. Reserved
words that cannot be used as names: `mod ext exp type ghost let in own ref arena with end
True False` (`c` is a keyword only right after `ext`/`exp`).

Full grammar notes, literals, operator table, and the complete prelude with every
signature: **`references/prelude.md`**.

## Types

Hindley–Milner inference: annotate the signature, never the body. `let x = e in body` binds
locally; `\x -> e` is a lambda; `xs |> f a` is `f a xs` (the pipe feeds the *last*
argument).

Numerics are `U8…U64`, `I8…I64`, `F32`, `F64`, plus `Nat` and `Size` (both normalise to
`U64`). `Nat` and `Size`, like every number, `Bool`, `Char`, `CStr` and `Ptr`, are scalars:
they are copied, not moved. `Str`, `Vec a`, records and ADTs are affine.

Prelude data: `Res e t = Ok t | Er e`, `Opt t = Some t | None`,
`Fault = Overflow | DivZero | OutOfBounds | BadParse`.

Matching is exhaustive or it is `match.nonexhaustive`, with the missing constructor named.
Patterns available: literals, names, `_`, `Ctor pat…`, list patterns `[]` / `[a,b,c]`, and
tuples. **There is no record pattern** — the grammar mentions one, the parser rejects it.
Project record fields with `.` instead.

```
mod Rec

;; The trailing expressions are the record's invariant: it is proved at every
;; construction and every update.
type Tx = { sku:Str, qty:U32, qty>0 }

;; `{r with f=v}` on a uniquely owned record mutates in place, no allocation.
restock (t:Tx) : Tx = {t with qty=12}

;; Fields are read with `.`. There is no record pattern: match on ADTs, project
;; records.
label (t:&Tx) : Str = fmt "{} x{}" t.sku t.qty

main : E! Unit =
  out (concat &(label &(restock {sku=dup "bolt", qty=1})) "\n")
```

## Refinements — the point of the language

A refinement lives in the signature, after the type, separated by a comma. It constrains
*parameters*. There is no postcondition syntax.

```
mean (xs:&Vec F64, len xs>0) : F64 = sum xs / f64 (len xs)
head (v:&Vec U64, len v>0) : U64 = get v 0
gap  (a:Nat) (b:Nat, a>=b) : Nat = a - b
safe_div (a:U64) (b:U64, b!=0) : U64 = a / b
```

The compiler generates obligations nobody wrote for: `get xs i` ⊨ `i < len xs`; `a / b` ⊨
`b != 0`; `a + b`, `a - b`, `a * b` ⊨ stays in the machine type; every record construction
and `{r with …}` ⊨ the invariant; every call to a refined function ⊨ its precondition.

**Match arms are proof steps.** What an arm learns — the tag, a list pattern's length, the
payload's record invariant, and that the arms above it did not fire — enters the solver's
context for that arm. This is the mechanism that removes run-time checks:

```
mod RefineDemo

type Err = Empty

mean (xs:&Vec F64, len xs>0) : F64 = sum xs / f64 (len xs)

;; The `Ok []` arm is the proof step: it removes the empty case, so the arm
;; below it discharges `len xs > 0` with no run-time check.
report (r:&Res Err (Vec F64)) : F64 =
  ?r |Er e  -> 0.0
     |Ok [] -> 0.0
     |Ok xs -> mean xs
  end
```

A failure reads like this, and the `fix` line is meant to be applied without deliberation:

```
error[refine.unproven]: cannot prove the precondition of `Avg.mean`: len xs > 0
  --> avg.vibe:5:30
   | report (xs:&Vec F64) : F64 = mean xs
   |                              ^^^^
   = counterexample: len_xs=0, xs=0
   = fix: Avg.report.sig += len xs > 0
```

The counterexample is a model: `len_xs=0` is the assignment that breaks it. Three ways out,
in order of preference: **(1)** add the precondition to your own signature and push the
obligation to the caller — that is what the `fix` says; **(2)** eliminate the case with a
match arm above the use, as `report` does; **(3)** when the value comes from outside the
program and no bound is derivable, use the checked forms (`div_checked`, `get_checked`,
`add_checked`, `sub_checked`, `mul_checked`) which return `Res Fault a`. Choosing (3) when
(1) was available is the single most common generation mistake — use (3) only for untrusted
external input.

Overflow is where `--prove` bites. The bound has to come from the signature, because nothing
else constrains a parameter:

```
step (a:U32, a<1000) (b:U32, b<1000) : U32 = a + b
```

A **guard** is the other source of bounds, and what the solver takes from it is exact:

- `?(i<len s)` bounds `i + 1` in the `True` arm, because a length is a `Size`.
- `?(i<n && byte_at s i==34)` lends the `True` arm every conjunct it can phrase (`i<n`) even
  though it cannot phrase the call. The `False` arm of such a guard learns **nothing**: a
  fact you need there goes in a match of its own.
- The left side of `&&` is known while checking the right: `i<=len s && len s - i>2`.
- `let b = byte_at s i` keeps `b`'s `U8` range; `u64 (byte_at s i)` inline does not, because
  the call is outside the solver's fragment. Bind first, then convert.
- Arithmetic that overflows on purpose (hashes, generators) is `wrap_add` / `wrap_mul`, not
  `+` / `*`.

`ghost` declarations parse and are excluded from codegen, but today they are also excluded
from obligation generation, so they teach the solver nothing yet. Do not reach for one.

## Termination

Divergence is an effect. Every **pure** recursive function must prove it terminates; every
`E!` function is exempt and may loop for ever.

```
mod Totality

;; Inferred: `k` shrinks syntactically at the only recursive call.
countdown (k:U64) (acc:U64) : U64 =
  ?k |0 -> acc
     |_ -> countdown (k - 1) acc
  end

;; Written with `%`: `k` grows towards `n`, so what shrinks is `n - k`.
count_up (k:U64) (n:U64, k<=n) : U64 =
  ?(k==n)
   |True  -> k
   |False -> count_up (k + 1) n
  end
  %(n - k)

;; Mutual recursion: every member writes the same lexicographic tuple.
ping (n:U64) (k:U64) : U64 =
  ?(k==0)
   |True  -> n
   |False -> pong n (k - 1)
  end
  %(n, k)

pong (n:U64) (k:U64) : U64 =
  ?(n==0)
   |True  -> k
   |False -> ping (n - 1) k
  end
  %(n, k)

;; `E!` is exempt: divergence is one of the effects it announces.
serve (port:U16) : E! Unit =
  out (show port) ;
  serve port
```

The measure is **inferred** only for direct self-recursion where one scalar parameter
shrinks syntactically at every call. Anything else — a parameter growing towards a bound,
mutual recursion — needs `%` written after the body. There is no increasing form: convert
mechanically, `k` rising towards `n` is `%(n - k)`. Members of a mutually recursive group
must write tuples of the same width; a short one is not padded.

**The trap:** a recursive name passed as a *value* is an edge in the call graph with unknown
arguments, so no measure can decrease at it and the function is rejected. `sum &(map loopy
&(single n))` inside `loopy` is `total.no_measure`. The fix is to call the function rather
than pass it.

**Walking a string or vector by index** is the loop you will write most. A measure may name
only scalar parameters — not `len s` — so pin the length to a parameter with a refinement:

```
skip_spaces (s:&Str) (n:Size, n==len s) (i:Size) : Size =
  ?(i<n)
   |False -> i
   |True  -> ?(byte_at s i==32)
              |True  -> skip_spaces s n (i + 1)
              |False -> i
             end
  end
  %(n - i)
```

The measure checker does **not** read guards: the step has to be syntactic (`i + 1`, `i + 2`
under a guard that keeps it in range). When the next position comes back from a callee — a
recursive-descent parser — nothing proves it moved, so thread a `fuel:U64` that every call
decrements and write `%fuel` on every member of the group; start it at a bound the input
cannot exceed. A self-call inside a nested match is still turned into a loop.

```
;; `item` returns where it stopped; nothing proves that moved, so fuel pays for each step.
items (s:&Str) (i:Size) (acc:Vec Str) (fuel:U64) : Res Str (Vec Str) =
  ?fuel
   |0 -> Er "input too long"
   |_ -> ?item s i
          |Er e      -> Er e
          |Ok (w, k) -> ?(k<len s)
                         |True  -> items s (k + 1) (push acc (dup &w)) (fuel - 1)
                         |False -> Ok (push acc (dup &w))
                        end
         end
  end
  %fuel

all_items (s:&Str) : Res Str (Vec Str) =
  ?(len s<1000000000000)
   |True  -> items s 0 empty (len s + 1)
   |False -> Er "input too long"
  end
```

`Ok (w, k)` binds into the result: a tuple pattern reads the fields rather than taking them,
hence `dup &w`.

**The other trap:** `%` is not modulo. `f (a:U64) (b:U64) : U64 = a % b` parses silently as
body `a` with measure `b`. There is no remainder operator in the language.

## Ownership

Affine. One owner per value, move by default, `&` borrows for the duration of the call, the
return value is always owned. No lifetime annotations exist.

```
mod Ownership

;; `&` borrows for the duration of the call; everything else is moved.
size_of (v:&Vec Str) : Size = len v

;; Two uses are fine when neither of them takes the value.
report (v:Vec Str) : Size = size_of &v + size_of &v

;; A value handed on is gone: `eat` owns it, so `s` cannot be read afterwards.
eat (s:Str) : Size = len &s

use_once (s:Str) : Size = eat s

;; `dup` is the copy: `&a -> a`, deep, so the copy owns what it points at and
;; the original stays usable. This is how one value becomes two owners.
grow (v:Vec Str) : Size =
  let d = dup &v in
  let w = push v "d" in
  size_of &w + size_of &d
```

Drops are implicit and there is no GC: the checker knows where each value stops being
yours, and that is where the `free` goes. Nothing is written by hand; `vibe view --drops`
prints the table if you need to see it.

`own.use_after_move` names the first move and offers `&x` or `dup x` — pick the borrow
whenever a read is all you need, because `dup` is a deep copy and costs what a deep copy
costs. Two exceptions to `dup`'s depth: `CStr` and `Ptr` stay shallow, because C owns that
memory and its extent is not knowable here, so a copy holding one still aliases.

Related refusals: `own.borrow_escapes` (returning a `&` parameter as owned — a borrow lives
only for the call that lent it, so return an index or a `dup`), `own.borrow_after_move`,
`own.use_after_update` (reading a record after `{r with …}` mutated it in place).

## Effects

Pure by default. `E! T` is a computation producing `T` while touching the world. A function
that calls an `E!` function must be `E!` — no inference, it is `effect.missing` with the
fix. Everything from `ext c` is `E!` by construction.

`<-` is the bind and is only legal in an `E!` body; it is terminated by `;`. `a ; b` is
`_ <- a ; b`.

A whole pipe filter — `vibe run examples/grep.vibe -- error < log.txt`:

```
mod Grep

hit (pat:&Str) (ln:&Str) : Bool = contains (lower ln) (lower pat)

main : E! Unit =
  args <- argv ;
  txt <- read_stdin ;
  ?drop 1 args
   |[p] -> each out (filter (hit &p) (lines &txt))
   |_   -> warn "usage: grep PATTERN < input"
  end
```

An `E!` value cannot be used as an argument directly — `out (read "x")` is
`effect.missing`; bind it first with `t <- read "x" ; out &t`.

## Modules

One file, one module, `mod Name` on the first line. The file name must match the module
name up to case and underscores (`TotalOk` may live in `total_ok.vibe`). **There is no
`import`, no alias, no visibility.** A qualified name at the use site is the entire system,
and it loads the file:

```
;; csv.vibe
mod Csv

;; One file, one module. Nothing is exported and nothing is imported.
type Row = { key:Str, val:Str }

parse (ln:&Str) : Opt Row =
  ?split '=' ln
   |[k,v] -> Some {key=trim &k, val=trim &v}
   |_     -> None
  end
```

```
;; app.vibe
mod App

;; `Csv.parse` is the whole import system: the name says which module it is in.
show_row (r:&Csv.Row) : Str = fmt "{} -> {}" r.key r.val

main : E! Unit =
  ?Csv.parse "host = localhost"
   |Some r -> out (concat &(show_row &r) "\n")
   |None   -> warn "no\n"
  end
```

Types and constructors qualify the same way (`Shape.Kind`, `Shape.Line`). A declared name
carries its module, so two modules may both declare `parse`, and a module may declare a name
the prelude already has — inside that module the declaration wins. Modules are looked up
next to the naming file, then on `VIBE_PATH`, then in `lib/` beside the compiler. Module
names do not nest: `Std.Json` is not a path.

Record fields are resolved per use site from the *base's* inferred type, so two modules may
both declare a field `qty`. What that costs: when inference cannot pin the base's type and
the field name belongs to more than one record, it is `field.ambiguous`, and the fix is to
write the type down. `ext c` symbols remain a shared namespace — two modules may name the
same C function, since it is the same function, but they may not give it two types.
(Module and field namespacing is the part of the compiler most actively in motion; if
something here disagrees with `docs/remaining-work.md` or `tests/modules.rs`, those win.)

## The standard library

`lib/` beside the compiler is on the search path, so these are one qualified name away
(`Json.parse`, `Json.JNum`, `Rand.Rng`). Each passes `--prove`. Every signature, with its doc
comment, is in **`references/stdlib.md`** — read the module's section there before calling
into it.

| module | what it has |
|---|---|
| `Json` | `Json` ADT, `parse : &Str -> Res Str Json`, `render`, `field`, `index`, `as_*` |
| `Csv` | RFC 4180 `parse : &Str -> Res Str (Vec (Vec Str))`, `render` |
| `Path` | POSIX text: `join basename dirname extension stem normalize segments` |
| `Encoding` | `hex unhex base64 unbase64` over bytes |
| `Hash` | `fnv1a`, `mix` (splitmix64 finaliser), `combine` — not cryptographic |
| `Rand` | `Rng`, `seed next below unit` — splitmix64, deterministic, not for secrets |
| `Args` | `parse` into flags / `--k=v` options / positionals, `has_flag`, `option` |
| `Text` | character classes, `join words pad_left repeat upper reverse …` |
| `List`, `DictX`, `Set`, `Opt`, `Res` | combinators beyond the prelude |
| `Math`, `Stats`, `Bits`, `Search`, `Time` | numeric helpers, statistics, binary search, calendar |
| `Io` | file, argument and environment conveniences |

Before writing a helper, check whether one of these already has it. A new module goes in
`lib/<name>.vibe` with its check beside the others, and should pass `--prove` too.

## Bit operations are functions

`band bor bxor bnot shl shr ord` are prelude **functions**, not operators, because `&` is
the borrow sigil and `|` separates match arms — and a call has no precedence to get wrong.
`shr` is arithmetic on a signed value and logical on an unsigned one.

```
low_byte (x:U64) : U64 = band x 255
set_flag (x:U64) (bit:Size) : U64 = bor x (shl 1 bit)
```

## C interop, both directions

First-class, not an escape hatch. `ext c "header.h" … end` calls C; `exp c name, …` emits
a C-ABI symbol plus a header so C calls you.

```
ext c "stdio.h"
  puts : &CStr -> E! I32
end

scale (x:F64) (k:F64, k>0.0) : F64 = x * k

exp c scale
```

Every `ext` signature must be `E!`. Only scalars, `Bool`, `Char`, `Str`, `CStr` and `Ptr a`
cross the boundary — anything else is `ffi.type` at `vibe check` time. And the documented
limitation: **an `ext c` refinement cannot name a parameter**, because an `ext` signature is
a type with no binders; the `(n:Size, n>0)` parameter form is a parse error there.

Full treatment — the type mapping, the generated header, `vibe build --lib`, what `own`/`ref`
do and do not do today: **`references/c-interop.md`**.

## The CLI

| command | what it is for |
|---|---|
| `vibe check f.vibe [--prove]` | the gate. `--prove` discharges refinements with z3 and adds overflow obligations |
| `vibe run f.vibe -- args` | build and run; refinements not proved become run-time asserts |
| `vibe build f.vibe [-o out] [--lib] [--emit-c]` | native executable, or a static library plus its C header |
| `vibe fmt f.vibe [--check]` | write the canonical form back; `--check` reports and fails instead |
| `vibe view f.vibe [--sig-only\|--explicit\|--flow\|--drops]` | canonical text; signatures only (cheap context); inferred types and whether each refinement is discharged; pipelines expanded; where every value dies |
| `vibe proof f.vibe [--prove]` | open obligations, one per line, by semantic path |
| `vibe deps f.vibe` | callees (`->`) and callers (`<-`) |
| `vibe patch f.vibe <path> [<hash> <node>]` | print a node and its hash; replace it if the hash still matches — the result is re-parsed and re-checked, and refused if it stops compiling |
| `--diag=prose\|struct\|json` | diagnostic rendering; `json` when you are going to parse it |

`vibe view --sig-only` is the cheap way to load a module into context. `vibe proof` is how
you see what is still open without reading the whole error list. `vibe patch` is the safe
structural edit when you know the semantic path (`Ledger.amt.body`) and want the compiler to
refuse a bad rewrite before it touches the file.

## When something is rejected

Read the `code`, the `counterexample`/`witness`, and the `fix`. The `fix` is designed to be
applied mechanically. The full catalogue — every code, what it means, and the repair —
is **`references/errors.md`**.

The five refusals that define the language: `match.nonexhaustive`, `own.use_after_move`,
`total.no_measure`, `refine.unproven`, `canon.parens` / `canon.form`.

## Sharp edges — read before designing anything

These are real limitations of the compiler as it stands, and each one has silently wasted
someone's time. Full list with sources: **`references/sharp-edges.md`**.

- **`remove` on a dictionary is O(n).** `Dict k v` (`dict insert lookup remove keys`) is a
  hash map in insertion order: `insert` and `lookup` are O(1), `remove` shifts the entries.
- **Anything that may alias is not freed at all.** Drops are deep, but a value a prelude
  call may hand out a piece of (`get`, `fold`, `max_by`, …) or that went to C is left alone.
- **Building a string byte by byte is O(n²).** There is no byte buffer; every `concat`
  copies. Slice whole ranges out of the input where you can.
- **A nullary `ext c` function is never called.** `abort : E! Unit` then `abort ;` compiles
  and does nothing. Give the binding a parameter.
- **A program that crashes** under `vibe run` prints `error[run.signal]`; a double free or a
  failed run-time assert is a compiler bug worth reducing, not a flaky test.
- **A proof about an `F32`/`F64` is a proof about a mathematical real** — nothing about
  rounding, precision or NaN. `--prove` says so in a note. Do not present a float proof as
  a guarantee about the machine float.
- **A nullary top-level declaration is not a constant.** `limit : U64 = 42` compiles to an
  unapplied closure and prints `<Mod.limit>` at run time; only `main` works nullary. Put
  constants inside the function that uses them, or give the declaration a parameter.
- **No record pattern, no modulo operator, no postcondition syntax.**
- **A negative literal cannot be a bare argument.** `f (-2.5)` is `canon.form` and `f -2.5`
  is the subtraction `f - 2.5`. Write `f (0.0 - 2.5)`, or bind the value first.
- **Integration tests race in parallel:** `cargo test -- --test-threads=1` is the reliable
  invocation, and CI pins it.

## References

- `references/prelude.md` — every prelude signature, the operator table, literals, the
  specials (`len`, `show`, `fmt`), and the grammar in brief.
- `references/errors.md` — the diagnostic catalogue, with real compiler output and the
  repair for each.
- `references/c-interop.md` — `ext c` and `exp c` in full, the boundary type mapping, and
  building a library for C to link.
- `references/sharp-edges.md` — what does not work yet, and what to do instead.
- `references/stdlib.md` — every `lib/` module's signatures and doc comments.

In the repository itself: `README.md` (the tour), `vibelang-spec.md` (normative),
`docs/remaining-work.md` (read before starting anything large), `examples/ledger.vibe` (the
reference program — 45 lines containing a CSV parser, an error type, record invariants, a
proof and a C ABI surface), and `tests/*.vibe`, where the files that exist to be *rejected*
are as instructive as the ones that compile.

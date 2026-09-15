# Vibelang — Language Specification

<sub>Italiano: [vibelang-spec.it.md](vibelang-spec.it.md)</sub>

> File extension: `.vibe` — command: `vibe`
> Document version: 0.1 — design draft, not normative
> Status: partly implemented. Each section states what the compiler does today and what is still design.

---

## 0. Goals and non-goals

### Goals, in priority order

1. **Minimize the total tokens spent reaching a correct program.** Not source length: generation + diagnostics + retries. Every design decision is judged against this metric.
2. **Minimize the error probability of a generating LLM.** One form per thing; syntax with a high prior in training data (ML/Rust family); no contextual rules.
3. **Maximum static correctness.** Pure, total, static ownership, refinement types with SMT discharge.
4. **Zero runtime, zero GC.** The binary depends on nothing beyond libc.
5. **Bidirectional C interoperability** with a minimal syntactic surface.
6. **Human readability as a derived goal**, obtained through projection tools, not through syntax design.

### Explicit non-goals

- It is not a general-purpose language for humans. Human ergonomics are delegated to the renderer.
- It does not support cyclic structures or arbitrary sharing without arenas.
- It does not support polymorphic lifetimes (see §4.4 for the cost of this choice).
- It has no macros, no overloading, no inheritance, no reflection.
- It does not aim to beat C on performance; it aims to match it.

---

## 1. Normative principles

These principles resolve design ambiguities. Where a specific rule conflicts with a principle, the principle wins.

**P1 — Uniqueness of form.** For every program there is exactly one valid textual representation. The parser rejects non-canonical forms. There is no optional formatter: canonicity is part of the grammar.

**P2 — No contextual rules.** The meaning of a token does not depend on its position in the file, on parser state, or on preceding declarations other than name binding.

**P3 — Demand-driven annotation.** No annotation is mandatory when it can be inferred. The compiler asks for one only when inference fails, naming the exact point.

**P4 — High prior on symbols.** Only ASCII symbols already frequent in existing code are used, with the meaning they already have there. No Unicode glyphs, no reinterpretation of a known symbol.

**P5 — Failure at compile time, never at run time.** Every condition that in a traditional language would be a panic (out-of-range index, division by zero, overflow, non-exhaustive match) is a proof obligation. If it does not discharge, it does not compile.

**P6 — The C boundary is the only place where the guarantees end,** and it must be syntactically visible.

---

## 2. Lexicon

### 2.1 Symbol table

Every symbol is chosen to be a single token in common tokenizers and not to collide with its own dominant prior.

| Symbol | Meaning | Reference prior |
|---|---|---|
| `:` | type ascription | ML, Rust |
| `->` | arrow (function type, match arm) | ML, Rust |
| `=` | definition | universal |
| `?` | introduces a match scrutinee | new, unambiguous position |
| `\|` | match arm / ADT alternative | ML, Haskell |
| `_` | wildcard | universal |
| `\|>` | pipe forward | F#, Elixir |
| `&` | borrow | Rust, C |
| `%` | termination measure | arithmetic, never structural in ML |
| `<-` | monadic bind in an effectful block | Haskell, Rust |
| `;` | sequencing: terminates a bind, chains two expressions | C, Rust |
| `{ }` | record literal and update | universal |
| `[ ]` | list literal and list pattern | universal |
| `( )` | grouping, tuple, parameter signature | universal |
| `,` | separator | universal |
| `.` | field access; also a first-class accessor | universal |
| `--` | **reserved, unused** | collides with the Haskell/Lua comment |
| `#` | **reserved, unused** | collides with the Python/shell comment |

Comments: `;;` to end of line. Chosen because it collides with no mainstream language's comment and is not an operator in the ML family.

### 2.2 Keywords

The complete set. Every keyword is a common English word (high prior, 1-2 tokens).

```
mod  ext  exp  type  ghost  let  in  end  E!  own  ref  arena  with
True False
```

Note: `own` and `ref` appear only in `exp c` signatures. Inside the language, ownership uses `&` and the default.

### 2.3 Identifiers

`[a-z][a-zA-Z0-9_]*` for values and functions, `[A-Z][a-zA-Z0-9_]*` for types and constructors.

**Names stay descriptive.** This is a deliberate deviation from compression: names carry semantics that guides generation. `total_amount` costs as much as `t` in tokens but produces fewer errors.

---

## 3. Grammar

EBNF. **Indentation is not significant**: no offside rule, no
`INDENT`/`DEDENT` token, no counting of spaces. A generator that miscounts
spaces must still produce a program that compiles, because a parse error costs
a whole retry and retries are the metric this language is optimized against.

Multi-arm constructs close with `end`, monadic sequencing uses `;`. One
significant `NL` survives, and it is not a counting rule: a newline ends a
top-level declaration. Without it juxtaposition swallows the next
declaration's name, and `f : U64 = 1` followed by `g : U64 = 2` parses as
`1 g` — a silent misparse, not an error. It applies only outside every bracket
and every `end`, only when the tokens so far already form an expression, and
only if the next token could begin one: a newline before a binary operator,
before a `|` in a type declaration, or before a `%` measure is a continuation.

One asymmetry follows from that rule, and it is the only place where the
position of a line break still matters: `-` is also unary negation, and the
lexer cannot tell the two apart, so it counts as something that can begin an
expression. `a\n- b` therefore ends the declaration, while `a -\nb` does not.

```ebnf
module      = "mod" ModName NL { decl } ;

decl        = typedecl | fundecl | extblock | expdecl | ghostdecl ;

typedecl    = "type" TypeName "=" typebody ;
typebody    = record | variants ;
record      = "{" field { "," field } [ "," refine ] "}" ;
field       = name ":" type ;
variants    = variant { "|" variant } ;
variant     = CtorName { type } ;

fundecl     = name { param } [ ":" type ] "=" expr [ measure ] ;
param       = name
            | "(" name { name } ":" type [ "," refine ] ")" ;
measure     = "%" expr ;
refine      = expr ;                    (* decidable boolean expression *)

ghostdecl   = "ghost" fundecl ;

expr        = app | match | bind | letexpr | lambda | literal | record
            | expr binop expr | expr "|>" expr ;
app         = atom { atom } ;           (* juxtaposition, curried *)
match       = "?" expr arm { arm } "end" ;
arm         = "|" pattern "->" expr ;
bind        = name "<-" expr ";" expr ; (* only in an E! context *)
seq         = expr ";" expr ;           (* `a ; b` is `_ <- a ; b` *)
letexpr     = "let" name "=" expr "in" expr ;
lambda      = "\\" name { name } "->" expr ;

pattern     = literal | name | "_" | CtorName { pattern }
            | "[" [ pattern { "," pattern } ] "]"
            | "(" pattern { "," pattern } ")"
            | record ;

type        = TypeName { type }
            | type "->" type
            | "&" type
            | "E!" type
            | "(" type { "," type } ")" ;

extblock    = "ext" "c" StringLit { extsig } "end" ;
extsig      = name ":" type [ "," refine ] NL ;
expdecl     = "exp" "c" name { "," name } ;
```

### 3.1 Canonicity rules

Canonicity is about **structure**, not layout. Whitespace does not reach the
AST: indentation, blank lines and the column a declaration starts in are free.
Forcing the generator to count them produced parse errors on otherwise correct
programs, which is exactly the cost P5 exists to avoid.

The parser **rejects**, it does not normalize:

- redundant parentheses;
- `let ... in` where a top-level binding would be equivalent;
- a `match` or an `ext c` block not closed by `end`;
- a `<-` binding not terminated by `;`.

The motivation is unchanged: if the parser normalized structure, several valid
forms of the same program would exist, violating P1 and introducing choice
points for the generator. Layout is not a choice point, because it does not
change the program: `vibe view` prints one, and nobody is obliged to write it
by hand.

---

## 4. Ownership

### 4.1 Model

Affine types. Every value has exactly one owner. Passing is **move by default**.

### 4.2 The one rule

> **Parameters prefixed with `&` are borrows valid for the duration of the call. Everything else is owned. The return value is always owned.**

There are no lifetime annotations. If a value has to outlive the call, it is either owned or it is a compile error.

```
amt   (t:&Tx)      : F64 = t.price * f64 t.qty      ;; borrows
consume (t:Tx)     : F64 = t.price                  ;; consumes, t no longer usable
```

Affine use is checked today over every owned name: parameters, `let` and `<-`
binders, the names a pattern binds, and the captures of a closure whose value
reaches the function's result — such a closure owns what it captured (§4.6),
while one consumed during the call, the argument to `map` say, only reads it.
A second use is an error whose `fix` is `&x` or `dup x`.

### 4.3 In-place reuse

`{ r with f = v }` on a uniquely owned `r` compiles to in-place mutation, zero allocations. On a borrowed or shared `r` it copies — and the compiler reports it as an informational diagnostic (not an error), because it is the main source of hidden cost.

### 4.4 Accepted cost of this choice

Functions returning a reference derived from a parameter (`fn first(v: &Vec<T>) -> &T` in Rust) are not expressible. In their place: return an index, return a copy, or use an arena.

**This limitation has to be validated on real code before the design is settled.** If zero-copy C APIs turn out to be systematically inexpressible, a minimal lifetime mechanism is needed — preferably inferred, not written.

### 4.5 Arenas

An escape valve for structures that linear ownership does not express (graphs, arbitrary sharing).

```
graph_size (n:Size) : Size = arena a in
  len (rev &(range 0 n))   ;; everything allocated in `a` lives until the end of the block
```

Bulk deallocation at end of scope. No reference counting, no run-time tracking.

Beside arenas there is a second, automatic release, at whole-frame
granularity. Allocation is a bump pointer; a frame that cannot hand a pointer
to C releases everything it allocated when it returns. Whether it can is
decided statically, and in a language with no globals and no mutation of
borrowed values there is exactly one way out: calling an `ext c` symbol, which
taints the function and every caller, because the release happens at the
outermost frame. The complementary question — did the result itself escape? —
is answered by the runtime for free: `vb_release` cancels entirely when the
returned value is a string, object, vector, closure, C string or pointer. The
ceiling is the granularity: a tail-recursive loop marks once and releases once,
so its own iterations still accumulate until it returns, and `arena` is the
manual override for that case.

### 4.6 Closures

Escape analysis is static and inferred:
- a closure that does not escape its scope → stack, zero cost;
- a closure that escapes → owns its captures, allocated by the caller.

No annotation. If the analysis cannot decide, it is an error demanding an explicit `move`.

The ownership half of this rule is enforced today; the allocation half is not —
every closure is heap-allocated, and the analysis decides conservatively rather
than demanding a `move`. Implicit drop and stack allocation for a non-escaping
closure are planned in [`docs/static-drop-roadmap.md`](docs/static-drop-roadmap.md), which also
describes deleting the bump allocator that the release of §4.5 rests on.

---

## 5. Effects

### 5.1 Rule

Every function is **pure by default**. The type `E! T` marks a computation producing `T` while touching the outside world.

```
amt  (t:&Tx) : F64        ;; pure
read (p:&Str) : E! Str    ;; effectful
```

### 5.2 Propagation

- A function calling an `E!` function must be `E!`. There is no silent inference: it is an error with a suggested fix.
- `<-` is the bind, usable only in an `E!` body.
- **Everything coming from `ext c` is `E!` by construction**, without exception. The typechecker cannot know what a C function does.

### 5.3 Granularity

v0.1 has a single effect (`E!`, undifferentiated). Splitting it (`IO`, `Alloc`, `Panic`) is deferred: it grows the syntactic surface and the benefit has to be measured.

---

## 6. Totality

### 6.1 Requirement

**Divergence is an effect.** Two rules follow from this, and no new construct:

1. **Pure functions must be total.** Every recursive pure function needs a
   measure decreasing on a well-founded order, inferred (§6.2) or written with
   `%`. This is what all the static reasoning rests on: a solver reasoning
   about a function that might not terminate is proving nothing.
2. **Effectful (`E!`) functions are exempt.** They may omit the measure and
   recurse for ever. An event loop or a server is designed not to terminate,
   and the signature already declares it: non-termination is one of the effects
   `E!` announces.

The alternative was adding `while` or `loop`, that is, one more construct and
one more choice point for the generator — exactly what Goal 1 forbids. The tail
recursion that already exists is enough:

```
serve (port:U16) : E! Unit =
  req <- wait_request port ;
  handle_request req ;
  serve port    ;; infinite recursion allowed: the function is E!
```

The boundary is sharp and legible in the signature. `serve` does not terminate
and says so; `amt (t:&Tx) : F64` terminates and says so.

### 6.2 Inference

The measure is **inferred** when a scalar parameter decreases syntactically in every recursive call. This covers the large majority of cases.

```
go (k:Nat) (a b:U64) : U64 =
  ?k |0 -> a
     |_ -> go (k-1) b (a+b)        ;; measure inferred: k
  end
```

When inference fails, the compiler asks for it and it is written with `%`:

```
sum_to (k:Nat) (n:Nat, k<=n) (acc:U64) : U64 =
  ?(k==n) |True  -> acc
          |False -> sum_to (k+1) n (acc+k)
  end
  %(n-k)
```

### 6.3 Decreasing measures only

There is no "increasing" form. Every increasing measure with a bound converts mechanically: `k` increasing towards `n` is `%(n-k)`. Two forms for the same concept would violate P1 with no gain in expressiveness.

### 6.4 Mutual recursion

Allowed. The measure is a tuple in lexicographic order over the group of mutually recursive functions.

---

## 7. Refinement types

### 7.1 Syntax

Refinements live in the signature, after the type, separated by a comma. There is no separate `where` block.

```
fib (n:Nat, n<=93) : U64
mean (ts:&Vec Tx, len ts>0) : F64
```

On record types they are invariants, verified at every construction and update:

```
type Tx = { sku:Str, qty:U32, price:F64, qty>0, price>0.0 }
```

There is **no syntax for a postcondition**: a refinement constrains the
parameters, never the result. A fact established inside a function therefore
does not leave it, which is the single largest open gap in this section — see
§16.8 and the obligations left open by the reference program of Appendix A.

### 7.2 Automatically generated obligations (P5)

The compiler generates a proof obligation, with nobody writing it, for:

| construct | obligation |
|---|---|
| `xs[i]` / `get xs i` | `i < len xs` |
| `a / b` | `b != 0` |
| `a + b` on machine integers | `a + b < MAX` |
| `a - b` on `Nat`/unsigned | `a >= b` |
| record construction with an invariant | the invariant |
| call to a function with a refinement | the precondition at the call site |

### 7.3 Discharge

Every obligation goes to an SMT solver (reference: Z3). An undischarged obligation is a **compile error** with a concrete counterexample, never a warning.

This is implemented and opt-in: `vibe check --prove` turns the obligations into
SMT-LIB 2 and discharges them with the `z3` binary on `PATH`. Without `--prove`,
`vibe check` only reports how many are open, and the refinement is asserted at
run time instead. Proved obligations are cached by the hash of the SMT text in
a sibling `.vibe-proofs`, and each obligation gets a solver budget
(`--prove-timeout=`, 5 seconds by default) after which the compiler reports
giving up rather than reporting a refutation (§16.5).

Two rules govern the context an obligation is proved in:

- a binder that shadows a refined name gets its own symbol, because inheriting
  the outer name's facts would prove something the program does not say;
- what a constructor pattern teaches enters its arm's context — the tag, the
  payload's length, and the payload's record invariant. That is §8.3's
  mechanism, and it is what discharges `len ts > 0` on an `Ok ts` arm below an
  `Ok []` arm.

### 7.4 Ghost functions

For invariants the solver does not derive locally. They are not compiled; they exist only for proofs.

```
ghost fib_spec (n:Nat) : Nat =
  ?n |0 -> 0
     |1 -> 1
     |_ -> fib_spec (n-1) + fib_spec (n-2)
  end
```

### 7.5 Escape hatch

Where the proof costs more than the benefit, the checked operations move the problem to run time and zero out the obligation:

```
add_checked : U64 -> U64 -> Res Overflow U64
get_checked : &Vec a -> Size -> Res OutOfBounds &a
```

**Note for the agent's system prompt.** The choice between proving statically and using the checked version is the decision a generator gets wrong most easily. The rule to state explicitly: prove statically when the bound derives from an already constrained input; use checked when the value comes from untrusted external input.

---

## 8. Pattern matching

### 8.1 Form

```
?scrutinee
 |pat1 -> expr1
 |pat2 -> expr2
end
```

### 8.2 Exhaustiveness

Mandatory. A non-exhaustive match is a compile error listing the missing cases.

### 8.3 Interaction with refinements (central mechanism)

The information gained on a branch enters the solver's context for that branch. This is the mechanism that eliminates redundant checks:

```
?load path
 |Er e  -> warn (show e)
 |Ok [] -> Er Void
 |Ok ts -> mean &ts        ;; len ts > 0 already proved: the Er and Ok [] arms are excluded
end
```

No explicit empty-list check, and `mean` is safe anyway.

This is implemented for constructor, literal, boolean and list patterns: an arm
learns the scrutinee's tag, the length of a list pattern's payload, and the
record invariant of whatever the payload binds. An arm below also learns that
the arms above it did not fire. What does not cross a function boundary is a
fact established in a *callee* — that needs a postcondition, and §7.1 has no
syntax for one.

---

## 9. Modules

One file, one module. `mod Name` on the first line. No visibility system in v0.1: everything declared is visible to importing modules, except `ghost`.

Import is implicit through qualification: `Ledger.total`. No `import` keyword, no alias — one construct fewer and one choice point fewer.

This is implemented as specified. `Ledger.total` loads the module `Ledger` from
a file sitting next to the one that names it; the file name must match the
module name up to case and underscores, so `TotalOk` may live in `total_ok.vibe`
as readily as in `TotalOk.vibe`, and nothing else is allowed to differ — the
mapping from a qualified name to a file stays mechanical. The loaded modules are
then flattened into a single unit, and a name declared twice is a `mod.duplicate`
error rather than a silent shadow.

*(Open: this does not scale beyond small projects. To be revisited before v1.)*

---

## 10. C interoperability

### 10.1 C → Vibelang

```
ext c "stdio.h"
  puts   : &CStr -> E! I32
  malloc : Size -> E! Ptr Byte
end
```

Rules:
- `E!` mandatory on every declaration (§5.2);
- refinements on an `ext` are **assumed, not proved**: verified at the Vibelang call sites, assumed beyond the boundary;
- an `ext` signature is a type, not a parameter list: the refinement is written
  after the type, `name : type, refine` (§3), and can only name what the call
  site itself names — there is no binder for the argument. The `(n:Size, n>0)`
  parameter form of a function declaration is not accepted here;
- the compiler emits the corresponding `#include` in the generated C.

### 10.2 Vibelang → C

```
exp c mean, total
```

Generates C-ABI symbols and the header. Ownership at the boundary uses just two qualifiers in the exported signature:

| qualifier | meaning |
|---|---|
| `own T` | the C caller takes ownership; `<name>_free` is exported as well |
| `ref T` | a borrow valid only for the duration of the call |

Preconditions end up in the header as a comment, with an explicit warning that they are not verified:

```c
/* Ledger.h */
/* mean — pre: len(ts) > 0   NOT VERIFIED ACROSS THE BOUNDARY */
double Ledger_mean(const Ledger_Vec_Tx *ts);
```

### 10.3 Type mapping at the boundary

| Vibelang | C |
|---|---|
| `U8 U16 U32 U64` | `uint8_t` … `uint64_t` |
| `I8 … I64` | `int8_t` … `int64_t` |
| `F32 F64` | `float`, `double` |
| `Bool` | `bool` |
| `Str` | `struct { const char *p; size_t n; }` |
| `CStr` | `const char *` (NUL-terminated) |
| `&T` | `const T*` |
| `own T` | `T*` |
| record | `struct` with the same field order |
| ADT | `struct { uint8_t tag; union {...} v; }` |
| `Res E T` | `struct { bool ok; union { E e; T t; }; }` |

`Str` and `CStr` are distinct types: conversion is explicit in both directions and allocates.

---

## 11. Backend

### 11.1 Strategy: C emission

Not a GCC frontend. Rationale:

- GCC has no plugin API for frontends; a frontend has to live in-tree and the process is largely undocumented. That means shipping a patched GCC.
- With no GC and no unwinding, Vibelang's IR is nearly isomorphic to C: the semantic gap a native frontend would close is minimal.
- Emitting C gives GCC, clang, MSVC and every embedded toolchain with no extra work, and debugging works through `#line` without generating DWARF.

C emission is what ships. A second, native backend is planned in
[`docs/backend-roadmap.md`](docs/backend-roadmap.md): Cranelift beside the C
backend, so that a pure Vibelang program needs no C toolchain on the machine
that builds it. The rule that plan is written under is that **the C backend is
not deprecated by it** — `exp c` headers, `--emit-c` and every embedded
toolchain are the reason C stays a fully supported target.

Future option: libgccjit (which despite the name also does AOT via `compile_to_file`), or LLVM. To be considered only if optimizations emerge that C cannot express.

### 11.2 Requirements on the generated C

- `#line` back to the `.vibe` source on every statement;
- self-tail-recursion → loop, emitted directly by the compiler, without relying on the optimizer;
- mutual tail calls → `[[gnu::musttail]]` (GCC/clang: an error when it cannot be realized, so the failure is visible at compile time);
- no `setjmp`/`longjmp`, no unwind tables;
- no dependency beyond libc.

### 11.3 Runtime

A minimal static library. It contains: the arena allocator, the closure representation, `Str`/`CStr` conversions, `Vec`. **No GC, no refcount, no signal handler.**

---

## 12. Diagnostics

### 12.1 Two formats, same content

The compiler emits prose by default and structured terms with `--diag=struct`.

### 12.2 Structured format

```
✗ <path> <code> ⊨ <counterexample>
  fix: <suggested patch>
```

Examples:

```
✗ Ledger.mean.body/2 div0 ⊨ len ts == 0
  fix: Ledger.mean.sig += len ts > 0

✗ Fib.go.arm[1] overflow:U64 ⊨ a=2^63, b=2^63
  need: invariant on (a, b)

✗ Ledger.parse.match nonexhaustive ⊨ missing [_, _]
  fix: add arm |_ -> Er (Bad (dup ln))
```

### 12.3 Requirements

- Every diagnostic carries a **semantic path**, not a line number (stable under insertion).
- Every refinement failure carries a **concrete counterexample** from the SMT model.
- Where the repair is mechanical (ownership failures, missing preconditions, missing arms), the `fix` field holds the applicable patch.

The `fix` field is the highest-yield point of the whole design against the metric of §0: it closes the correction loop inside a single diagnostic instead of a retry.

---

## 13. Tooling

### 13.1 Views

```
vibe view <path>              ;; canonical form (identical to the file)
vibe view <path> --explicit   ;; inferred types, borrows, discharged proofs, implicit copies
vibe view <path> --sig-only   ;; signatures only
vibe view <path> --flow       ;; pipelines expanded into named bindings
```

`--explicit` is the answer to the requirement "a human must be able to see what is in the code": it shows everything the language omits.

```
go : (k:Nat) -> (a:U64) -> (b:U64) -> U64
go k a b =
  ?k                             ;; exhaustive {0, _}
   |0 -> a
   |_ -> go (k-1) b (a+b)        ;; |- k-1 : Nat   (k != 0)
                                 ;; |- a+b < 2^64  [inv, from n<=93]
  end
  %k                             ;; |- k-1 < k     [inferred]
```

`--sig-only` is the cheap view for the agent's context: signatures for the whole module, bodies only for what is being modified.

All four views exist. The canonical projection is byte-identical on every
`.vibe` file in the repository, comments included — the comments are put back
from the source, so only the canonical view can carry them; the other three
rewrite the program and drop them. `--explicit` is narrower than the sample
above: it prints the inferred signature of each declaration and, for a refined
one, whether the refinement is discharged or checked at run time. Exhaustiveness
annotations and per-arm proof terms are not emitted.

### 13.2 Structured edit

```
vibe patch <path> <hash> <new-node>
```

Addressing is by **semantic path** (`Ledger.mean.body`), not by index — stable under insertion and reordering. The hash of the expected subtree acts as optimistic concurrency: if it does not match, the patch is rejected instead of being applied in the wrong place.

Files remain text and are the source of truth. The AST is a derived cache. The canonical form (P1) guarantees that a structured patch produces a minimal textual diff, so git, grep and code review keep working.

This is implemented. `vibe patch <file> <path>` with no hash prints the node and
its hash; with a hash and a new node it replaces the node only if the hash still
matches. Before anything is written, the resulting file is re-lexed, re-parsed
and re-checked, and the patch is refused with the diagnostics — leaving the
original untouched — if the result is not a program.

### 13.3 Surface to expose to the agent

| operation | purpose |
|---|---|
| `view --sig-only` | cheap context |
| `view --explicit` | inspection |
| `patch` | structured edit |
| `check --diag=struct` | diagnostics as terms |
| `deps` | callers and callees |
| `proof` | open SMT obligations on a node |

Every row exists. `vibe deps` reports callers and callees, `vibe proof` lists
the open obligations one per line under the same semantic path the diagnostics
use, and with `--prove` the ones z3 closes are marked as closed.

---

## 14. Minimal standard library (v0.1)

Only what is needed to write the compiler itself and test programs.

```
Prelude   Nat U8..U64 I8..I64 F32 F64 Bool Unit Str CStr Ptr Size
          Res e t = Ok t | Er e
          Opt t   = Some t | None
Vec       new push get set len map filter fold each sum max_by min_by
          sort_by rev concat_vec seq range take drop
Str       split lines dup len concat fmt trim starts_with contains slice
          index_of replace lower chr to_cstr from_cstr
Math      abs min max
IO        read read_stdin write out warn argv exit    ;; all E!
Checked   add_checked sub_checked mul_checked get_checked div_checked
```

Every stdlib function carries its own refinements:
```
get  (v:&Vec a) (i:Size, i < len v) : &a
div  (a:F64) (b:F64, b != 0.0) : F64
```
A partial function that the bootstrap cannot yet phrase a refinement for returns
`Opt` instead — `slice` and `index_of` do.

---

## 15. Implementation roadmap

An order designed to surface the real risks early.

**Phase 0 — Validating the hypothesis (before writing the compiler).**
Take 50 representative tasks, generate them with an LLM in Rust/Haskell and in mocked Vibelang syntax (no compiler, manual correction). Measure **tokens to a correct program**, not source length. If the delta is under 20%, the design has to be rethought before an implementation exists to make that expensive.

**Phase 1 — Frontend.** Lexer, parser with canonicity checking, AST, name resolution. No semantics.

**Phase 2 — Typechecker.** Hindley-Milner inference, ADTs, exhaustiveness, effects. At this point the language is already useful and checkable.

**Phase 3 — C emission.** Without ownership: allocate and never free. It validates the type mapping, the FFI and the minimal runtime on real programs.

**Phase 4 — Ownership.** Borrow checker, in-place reuse, escape analysis. Here the generated C starts freeing memory.

**Phase 5 — Totality.** Measure inference, termination checking.

**Phase 6 — Refinements.** Obligation generation, Z3 integration, counterexamples, structured diagnostics with a `fix` field.

**Phase 7 — Tooling.** Views, structured patches, agent surface.

Phases 1–3 produce a usable language already. Phases 4–6 are independent of each other and can be ordered differently.

---

## 16. Open questions

Points where the design is unresolved and the choice has to be made with data, not a priori.

1. **Absence of lifetimes (§4.4).** The real cost is unknown until code is written that interoperates with zero-copy C APIs. It might require a minimal mechanism, preferably inferred.

2. **Overflow invariants.** On arithmetic near the limit of the representation the solver often asks for a ghost function, which costs more tokens than the whole function. To be evaluated: an integrated range analysis for the common cases, so that not everything is delegated to SMT.

3. **Module system.** Implicit qualification (§9) does not scale. A solution is needed that does not reintroduce `import`/alias/visibility — that is, four new constructs.

4. **Effect granularity (§5.3).** A single `E!` is probably too coarse for real code; splitting it grows the surface. To be decided on real code.

5. **Solver time.** Partly addressed. Certificates are cached by the hash of
   the SMT text in a sibling `.vibe-proofs`, so a run whose obligations are all
   cached needs no solver at all, and each obligation gets a budget
   (`--prove-timeout=`, 5 seconds by default) after which the compiler reports
   `refine.budget` — it says it gave up, rather than presenting the absence of
   a proof as a refutation. What is open is the granularity: the cache is keyed
   on the text of the question, not on the subtree, so an unrelated edit to the
   context invalidates it.

6. **Cyclic structures.** Arenas cover many cases but not all. It is unclear whether an additional mechanism is needed or whether the constraint is acceptable.

7. **Concurrency.** Entirely out of scope for v0.1. Immutability and linear ownership are a good basis, but the design has not been considered.

8. **Postconditions.** §7.1 has syntax for a precondition and none for a
   postcondition, so a fact established inside a function does not leave it.
   This is what leaves the calls to `mean` and `top` in Appendix A's `main`
   open: `load` has already excluded `Ok []`, and the caller cannot see it. Any
   syntax for it is a new construct and a new choice point for the generator,
   which is why it is a question and not a feature.

---

## Appendix A — Reference program

The file every implementation phase has to keep compiling.

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

Properties this program exercises: FFI in both directions, ADTs with payloads, records with an invariant, nested and exhaustive pattern matching, effect propagation, multiple borrows of the same value, explicit copy from a borrowed slice, and — the central point — `mean` and `top` requiring `len ts > 0` with no explicit check, because the `Ok []` branch was already consumed in `load`.

It compiles and runs. Under `--prove` most of its obligations are discharged;
the two in `main` are not, for the reason §16.8 gives.

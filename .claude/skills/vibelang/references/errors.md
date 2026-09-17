# Diagnostic catalogue

Every diagnostic carries a **semantic path** (`Ledger.mean.body/2`), a **witness** or
counterexample saying why, and — where the repair is mechanical — a **`fix`** meant to be
applied without understanding it. Apply the `fix` first; it is right far more often than
a rewrite you invent.

Three renderings of the same content:

```bash
vibe check f.vibe                  # prose (default)
vibe check f.vibe --diag=struct    # ✗ <path> <code> ⊨ <counterexample> / fix: …
vibe check f.vibe --diag=json      # a JSON array — use this when you will parse it
```

```
[{"path":"Move.twice","code":"own.use_after_move","msg":"`s` was already moved",
  "file":"move.vibe","line":5,"col":30,"witness":"first moved at line 5, column 28",
  "fix":"borrow it here with `&s`, or copy it with `dup s`"}]
```

Contents:
1. [The five that define the language](#the-five-that-define-the-language)
2. [Ownership](#ownership)
3. [Totality](#totality)
4. [Refinements](#refinements)
5. [Effects](#effects)
6. [Types and names](#types-and-names)
7. [Parsing and canonicity](#parsing-and-canonicity)
8. [Modules](#modules)
9. [FFI](#ffi)
10. [Notes that are not errors](#notes-that-are-not-errors)

---

## The five that define the language

`match.nonexhaustive` · `own.use_after_move` · `total.no_measure` · `refine.unproven` ·
`canon.parens` / `canon.form`. Each is detailed below.

---

## Ownership

### `own.use_after_move`

```
error[own.use_after_move]: `s` was already moved
  --> tests/move.vibe:5:30
   | twice (s:Str) : Str = join s s
   |                              ^
   = counterexample: first moved at line 5, column 28
   = fix: borrow it here with `&s`, or copy it with `dup s`
```

A value used twice when it was only ever yours once. Checked over every owned name:
parameters, `let` and `<-` binders, pattern binders, and the captures of a closure whose
value reaches the function's result (an escaping closure owns what it captured; one
consumed during the call, the argument to `map` say, only reads it).

**Fix, in order:** borrow with `&x` when the second use is only a read — that is free.
`dup x` when you genuinely need two owners; it is a deep copy and costs one. Restructuring
so the value is produced twice is the last resort.

### `own.borrow_escapes`

```
error[own.borrow_escapes]: `s` is a borrow and is returned as owned
   | launder (s:&Str) : Str = s
   = counterexample: a borrow lives only for the call that lent it
   = fix: copy it with `dup s`, or return an index
```

There are no lifetimes, so a function cannot return a reference derived from a parameter.
Return a `dup`, return an index, or use an `arena`.

### `own.borrow_after_move`

```
   | after (v:Vec Str) : Vec (Vec Str) = push (single v) (rev &v)
   = fix: read `v` before the move
```

Argument evaluation order made the borrow land after the move. Reorder.

### `own.use_after_update`

```
   | let u = {t with qty=9} in
   |   push (single u.qty) t.qty
   = fix: read `t` before the update
```

`{r with f=v}` on a uniquely owned `r` mutates in place, so the base is no longer readable.
An update in one match arm is not seen by the other arms: `?b |True -> {c with x=1} |False
-> {c with y=c.x}` is fine.

---

## Totality

### `total.no_measure`

```
error[total.no_measure]: `TotalNoMeasure.spin` is recursive and no parameter decreases at every call
   | spin (n:U64) (acc:U64) : U64 =
   = fix: add a measure after the body, e.g. `%(n-k)`
```

A pure recursive function with nothing shrinking. Either write the measure with `%` after
the body, or make the function `E!` if it genuinely is not meant to terminate.

**The value trap.** A recursive name passed as a *value* is an edge in the call graph with
unknown arguments, so nothing can be shown to decrease:

```
loopy (n:U64) : U64 = sum &(map loopy &(single n))   ;; total.no_measure
```

The direction is deliberately conservative — a terminating program written that way is
rejected too. The answer is to call the function rather than pass it.

### `total.not_decreasing`

```
error[total.not_decreasing]: the measure of `TotalGrowing.up` does not decrease at this call
   |    |False -> up (k + 1) n
   = counterexample: measure -1*n + 1*k
   = fix: make an argument shrink, or give a measure that does, e.g. `%(n-k)`
```

The measure you wrote grows. There is no increasing form; convert mechanically: `k` rising
towards `n` is `%(n - k)`.

The same error when the step is not syntactic. The checker does not use the guards around
the call, so `go (k + 1)` decreases `%(n - k)` but `go (next_index s k)` does not, whatever
`next_index` guarantees. Thread a `fuel:U64` that each call decrements and measure `%fuel`.
And `%(len s - i)` is refused outright (`the measure … is not arithmetic over its
parameters`): add `(n:Size, n==len s)` and write `%(n - i)`.

For a mutually recursive group, write the same lexicographic tuple on every member —
`%(n, k)` — and note the members must use tuples of the same width, because a short one is
not padded.

---

## Refinements

### `refine.unproven`

```
error[refine.unproven]: cannot prove the precondition of `Avg.mean`: len xs > 0
  --> avg.vibe:5:30
   | report (xs:&Vec F64) : F64 = mean xs
   |                              ^^^^
   = counterexample: len_xs=0, xs=0
   = fix: Avg.report.sig += len xs > 0
```

**Reading the counterexample.** It is the solver's model: the assignment of every symbol in
the obligation's context that makes the goal false. `len_xs=0` is the one that matters
here — `xs=0` is the opaque handle and carries no information. Symbols are named after the
program's names, with `len_` prefixes for lengths and `_0`/`_1` suffixes where a name was
rebound.

**Three repairs, in order of preference:**

1. **Push it to the caller.** Add the precondition to your own signature — that is
   literally what `fix` says. `Avg.report.sig += len xs > 0`.
2. **Eliminate the case with a match arm.** What an arm learns enters the solver's context
   for that arm: the tag, a list pattern's length, the payload's record invariant, and the
   fact that the arms above did not fire. An `|Ok [] -> …` arm above `|Ok xs -> mean xs`
   discharges `len xs > 0`.
3. **Move it to run time** with a checked form (`div_checked`, `get_checked`,
   `add_checked`, `sub_checked`, `mul_checked` → `Res Fault a`). Correct only when the
   value comes from untrusted external input and no bound is derivable.

The same code covers `div0` (`cannot prove the divisor is non-zero`) and `overflow`
(`cannot prove 'a + b' stays in U64`), which are generated automatically.

**Overflow is the one that surprises.** `vibe check --prove` demands that every `+`, `-`,
`*` on a machine integer stays in range, and the bound must come from a signature because
nothing else constrains a parameter:

```
step (a:U32, a<1000) (b:U32, b<1000) : U32 = a + b
```

Several files in this repository pass `vibe check` and fail `vibe check --prove` for
exactly this reason. Say which bar you met.

Shadowing is deliberate: a binder that rebinds a refined name gets its own solver symbol,
because inheriting the outer name's facts would prove something the program does not say.

### `refine.budget`

The solver hit its per-obligation budget (`--prove-timeout=`, 5 seconds by default). The
compiler reports **giving up**, not a refutation. Raise the timeout, or simplify the
obligation.

### `match.nonexhaustive`

```
error[match.nonexhaustive]: this match does not cover Holes.Blue
   |   ?c |Red   -> dup "r"
   = counterexample: missing Holes.Blue
   = fix: add `|Holes.Blue -> ...`
```

The missing constructor is named. Add the arm, or a `|_ -> …` wildcard — but prefer the
named arm, because an arm is also a proof step and a wildcard teaches the solver nothing.

A constructor counts as covered only by an arm whose payload patterns are all names, `_`, or
tuples of those: `|Some 3 -> … |None -> …` names `Some` as missing. Nested constructor
patterns are not combined either, so `|Some (Ok x) |Some (Er e) |None` needs a `|_`. A
tuple of names, `|(a, b) -> …`, covers its type on its own.

---

## Effects

### `effect.missing`

Two shapes. A body that performs effects under a pure signature:

```
error[effect.missing]: `Eff2.f` performs effects but its result type is pure `Str`
   |   t <- read p ;
   = fix: change the result type to `E! Str`
```

Or an effectful value used where a pure one is expected:

```
error[effect.missing]: expected an effectful `E! Str` but found the pure `Str`
   |   out (read "x")
```

Bind it first: `t <- read "x" ; out &t`.

### `effect.leak`

```
= fix: bind it first with `name <- ...`, or mark the enclosing function `E!`
```

### `effect.pure_bind`

`<-` used outside an `E!` body.

### `ext.pure`

An `ext c` signature without `E!`. Everything coming from C is `E!` by construction — the
typechecker cannot know what a C function does.

---

## Types and names

| code | meaning |
|---|---|
| `type.mismatch` | unification failed; the message names both sides and the argument position |
| `type.infinite` | occurs check — a type would contain itself |
| `name.unbound` | undefined name or constructor; carries a `did you mean` suggestion |
| `name.duplicate` | two declarations of one name in one module |
| `arity.excess` / `arity.mismatch` | too many arguments; `len` and `show` take exactly one |
| `field.unknown` / `field.not_record` | `.f` on something without that field |
| `record.unknown` / `record.not_record` / `record.nomatch` | a record literal that names no declared record, or the wrong field set |
| `pattern.arity` | a constructor pattern with the wrong number of payload patterns |
| `fmt.args` | `fmt` with no format string |

---

## Parsing and canonicity

### `canon.parens`

```
error[canon.parens]: redundant parentheses
   | f (n:U64) : U64 = (n)
   = fix: remove the parentheses
```

Parentheses around a bare atom — an integer, float, string, char, bool, name or
constructor — are not canonical. `(f x)` is fine; `(x)` is not.

### `canon.form`

The file is not the canonical form: `vibe view` would have rendered it differently.
Structurally it is mostly the compiler guarding itself, since the parser already rejects
the structural variants — but it does fire on real source. The one you will meet:

```
error[canon.form]: this is not the canonical form of the program
   |   d <- fabs (-2.5) ;
   |             ^
   = fix: run `vibe view` on the file and write back what it prints
```

`vibe view` prints `fabs -2.5`, which re-parses as the *subtraction* `fabs - 2.5`. A
negative literal is not expressible as a bare argument; write `fabs (0.0 - 2.5)`, or bind
the value first. The fix line is still the right procedure — run `vibe view`, read what it
prints, and if what it prints is not the program you meant, rewrite the expression.

**Layout is not what these check.** Indentation, blank lines and starting column are free
and `vibe check` accepts any of them. `vibe fmt --check` is the tool that enforces the
canonical *text*. Run `vibe fmt` before handing anything over.

### `parse.*`

| code | trigger |
|---|---|
| `parse.match` | a `match` not closed by `end` — `fix: add \`end\`` |
| `parse.ext` | an `ext c` block not closed by `end` |
| `parse.bind` | a `<-` binding not terminated by `;` |
| `parse.let` | a malformed `let … in` |
| `parse.pattern` | e.g. a record pattern, which is not implemented |
| `parse.param`, `parse.type`, `parse.expr`, `parse.ctor`, `parse.name`, `parse.mod`, `parse.exp`, `parse.arena`, `parse.expected`, `parse.trailing` | the rest |
| `lex.char`, `lex.string`, `lex.int`, `lex.escape` | malformed literals |

Two silent misparses to watch for, because neither is an error:

- `f a b = a % b` — `%` starts a **measure**, not a modulo. Body `a`, measure `b`.
- Two declarations on one line — `f : U64 = 1 g : U64 = 2` parses as `1 g`. This one *is*
  caught, but as a downstream type error rather than a layout complaint.

`vibe view` is the cheap way to see what the parser actually built.

---

## Modules

| code | meaning |
|---|---|
| `mod.no_name` | a qualified name the named module does not declare (`Money.centz`) — caught by name, not as an unbound-name error somewhere downstream |
| `parse.mod` | the file does not start with `mod <Name>` |
| `mod.name` | `mod Foo` in a file not named `foo.vibe` / `Foo.vibe`. The mapping is mechanical: case and underscores are the only tolerated difference |
| `mod.missing` | the file for a qualified name was not found; the error lists every directory it looked in |
| `mod.duplicate` | one name declared in two modules where the namespace is still shared — `ext c` symbols, which may be named by two modules but not given two types |
| `field.ambiguous` | a `.field` read or record literal whose base type inference could not pin down, where that field name belongs to more than one record. `fix: write the type: a parameter or a signature says which record this is` |
| `mod.unreadable` | the file exists in a qualified name but cannot be read |

---

## FFI

### `ffi.type`

```
error[ffi.type]: `&Vec U64` cannot cross the C boundary
   |   takes : &Vec U64 -> E! Unit
   = fix: use a scalar, Bool, Char, Str, CStr or `Ptr a` at the C boundary
```

Raised by `vibe check`, deliberately — a checker that passes a program `vibe build` then
refuses costs a whole retry for nothing.

`export.unknown` — `exp c` naming something that is not declared.

`codegen.*` (`codegen.unbound`, `codegen.record`, `codegen.field`, `codegen.ctor`,
`codegen.binop`) are internal invariants. If one fires, it is a compiler bug, not a program
error.

---

## Running

### `run.signal`

```
error[run.signal]: the program was killed by signal 6
```

The program crashed; `vibe run` exits 128 + the signal. Signal 6 (`SIGABRT`) from the C
allocator is a double free or a bad pointer, which a checked program should never do: reduce
it to a few lines and treat it as a compiler defect. A run-time refinement failure is not a
signal; it prints `✗ <path> ⊨ <predicate>` and exits 70.

---

## Notes that are not errors

```
note: 1 obligation(s) are about F32/F64 and are approximated as mathematical reals,
      not machine floats
note: N refinement obligation(s) not discharged; run with --prove
note: N obligation(s) could not be expressed in the solver's fragment and were not proved
```

The first is a real limitation: a float proof says nothing about rounding, precision or
NaN. Do not report it as a guarantee about the machine float. The second means you ran
`vibe check` without `--prove` and the obligations became run-time asserts. The third means
the obligation generator could not phrase the goal at all — those obligations are silently
*not* checked, so `--prove` exiting 0 with that note is weaker than it looks.

(The spec promises an informational diagnostic when `{r with f=v}` copies instead of
mutating in place. It is not emitted today — `vibe view --drops` and reading `own.rs` are
the only ways to see that cost.)

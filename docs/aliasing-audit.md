# Aliasing audit

[`static-drop-roadmap.md`](static-drop-roadmap.md) §3 names a precondition it did
not schedule:

> Either the aliasing rules get checked, or Task 8 is not safe to enable by
> default. This is not scheduled below; it is a precondition, and it may be
> larger than this plan.

Nobody had measured it, so item 6 of [`remaining-work.md`](remaining-work.md) had
no estimate. This is the measurement. Eight gaps, each with a program that the
compiler on `master` accepts, each run. What each costs today, what it costs
after Task 8, and what checking it would cost.

The verdict is at the end: **comparable to the drop plan, not larger**, on one
condition the plan has already accepted in principle.

---

## What the affine checker guarantees

Stated as a property. For a function `f` and a name `n` that `f` binds — a
parameter whose written type `own.rs:67`'s `is_affine` calls affine, or a `let`,
`<-` or pattern binder that inference recorded in `Checked::affine`:

> **`n` occurs in at most one consuming position along any one path through `f`'s
> body.**

Consuming is every position except five, which `State::walk` visits in
`Mode::Borrow`: under `&` (`own.rs:165`), the object of a `.field`
(`own.rs:166`), the head of an application (`own.rs:168`), the scrutinee of a
match (`own.rs:220`), and the body of a lambda that `escapes` did not mark
(`own.rs:206-214`). Match arms are alternatives — each restarts from the state
before the match and a name consumed in any arm is consumed after it
(`own.rs:221-238`).

That is the whole guarantee, and it is a statement about *names*. It says nothing
about the heap. In particular it does not say:

- that two owned names denote different allocations;
- that a name's consuming use is its last use — `use_var` returns early on
  `Mode::Borrow` (`own.rs:128`), so borrows leave no trace at all;
- that what is reachable from `n` is reachable from nothing else;
- that `&` constrains what the callee does with the pointer, or how long it keeps
  it.

The last one is structural rather than an omission. `Ty::Ref` never reaches the
type checker: `types.rs:414` lowers `Ty::Ref(inner)` to `inner`. `&T` and `T` are
one type. `&` is a lexical marker that `own.rs` reads and nothing else does.

Everything below follows from that. The checker counts moves of names; aliasing
is about allocations, and allocations are not what it counts.

---

## The gaps

All eight were compiled with `cargo build --release` at `354ea43` and run.
Nothing I expected to be accepted was rejected; the double-move shapes that
`tests/move.vibe`, `tests/move_let.vibe`, `tests/move_pat.vibe`,
`tests/move_vec.vibe` and `tests/capture.vibe` cover are all still caught.

### 1. `&` is erased, so a borrow can be laundered into an owned value

```
mod A12

launder (s:&Str) : Str = s

main : E! Unit = out (launder (concat "a" "b"))
```

```console
$ vibe check a12.vibe ; echo $?
0
$ vibe run a12.vibe
ab
```

`types.rs:414` throws the `&` away, so the body type-checks, and `own.rs:67`
returns `false` for `Ty::Ref`, so `s` is not in the owned set and no move is
recorded. The caller's value comes back out as an owned result. Every other gap
in the "needs provenance" group below can be written as a special case of this
one, and this is the version that fits on one line.

**Today:** nothing. The value is bump-allocated and `vb_release` cancels on a
`VB_STR` result anyway.
**Under exact drop:** double free. The caller drops the argument; the callee's
caller drops the result; they are one object.

### 2. Prelude functions hand out interior pointers as owned values

```
mod A1

grab (v:Vec Str) : Vec (Vec Str) = push (single (single (get &v 0))) v

main : E! Unit = out (fmt "{}\n" (show (grab (push (single (dup "a")) (dup "b")))))
```

```console
$ vibe check a1.vibe ; echo $?
note: 1 refinement obligation(s) not discharged; run with --prove
0
$ vibe run a1.vibe
[[a], [a, b]]
```

`get : &Vec a -> Size -> a` (`types.rs:129`) is implemented as `return s->a[k]`
(`vibert.c:335`): the element pointer itself, not a copy. Its result type is
owned, and `v` still owns the same object. `max_by`, `min_by`, `sum` and `fold`
have the same shape — `vb_max_by` returns `s->a[best]` (`vibert.c:384`).

The two-owned-names version needs no second name at all:

```
mod A2

twin (v:&Vec Str) : Vec Str = push (single (get v 0)) (get v 0)
```

```console
$ vibe check a2.vibe ; echo $?
note: 2 refinement obligation(s) not discharged; run with --prove
0
$ vibe run a2.vibe
[a, a]
```

Two slots of one vector hold one `Str`.

**Today:** nothing.
**Under exact drop:** double free.

### 3. Structural vector operations copy the spine and share every element

```
mod A3

both (v:Vec Str) : Vec (Vec Str) = push (single (rev &v)) v

main : E! Unit = out (fmt "{}\n" (show (both (push (single (dup "a")) (dup "b")))))
```

```console
$ vibe check a3.vibe ; echo $?
0
$ vibe run a3.vibe
[[b, a], [a, b]]
```

`vb_rev` (`vibert.c:418`), `vb_push` (`vibert.c:327`), `vb_set`
(`vibert.c:341`), `vb_filter`, `vb_sort_by` and `vb_concat_vec` all allocate a
fresh `VbVec` and copy `VbVal`s into it. A `VbVal` of tag `VB_STR` is a pointer.
So the two vectors in the result above are distinct spines over one set of
elements, and both are owned.

This is worse than gap 2 in degree: it is not one aliased element, it is all of
them, and the checker sees two names of type `Vec Str` neither of which is moved
twice.

**Today:** nothing.
**Under exact drop:** double free of every element. It also forces a runtime
decision the drop plan has not made — dropping a vector has to be able to be
shallow.

### 4. A match binds the scrutinee's payload without consuming the scrutinee

```
mod A11

type Err = Void

keep (r:Res Err Str) : Vec (Res Err Str) =
  let s = ?r |Er e -> dup "x"
             |Ok x -> x
         end in
  push (single (Ok s)) r

main : E! Unit = out (fmt "{}\n" (len &(keep (Ok (dup "a")))))
```

```console
$ vibe check a11.vibe ; echo $?
0
$ vibe run a11.vibe
2
```

`own.rs:220` walks the scrutinee in `Mode::Borrow`, deliberately: a match is a
read. But the arm binders are owned (`pat_names`, `own.rs:258`), and `x` *is*
`r`'s payload. So `s` and `r` are both live, both owned, and `s` is reachable
from `r`. The vector above holds the payload twice.

**Today:** nothing.
**Under exact drop:** double free, if the drop of an `Ok` is deep. If it is
shallow, gap 3's problem arrives instead.

### 5. A borrow after a move is not checked

```
mod A7

after (v:Vec Str) : Vec (Vec Str) = push (single v) (rev &v)

main : E! Unit = out (fmt "{}\n" (show (after (push (single (dup "a")) (dup "b")))))
```

```console
$ vibe check a7.vibe ; echo $?
0
$ vibe run a7.vibe
[[a, b], [b, a]]
```

`v` is moved into `single`, then read through `&v`. `use_var` returns on
`Mode::Borrow` before it ever looks at `self.moved` (`own.rs:127-131`), so the
second use is not compared against the first. The diagnostic `own.use_after_move`
offers `&x` as its mechanical fix, and `&x` is precisely the thing the checker
stops looking at.

**Today:** nothing. The bump allocator has not reused the memory.
**Under exact drop:** use-after-free. The move handed the value's lifetime to the
callee; the borrow reads it afterwards, and the drop the callee's frame emits may
already have run. C's unspecified argument evaluation order decides which, per
call site and per compiler.

### 6. `{r with ...}` mutates in place while the base is still readable

This is the only one that is wrong today.

```
mod R4b

type Tx = { sku:Str, qty:U32 }

upd (t:Tx) : Vec U32 =
  let u = {t with qty=9} in
  push (single u.qty) t.qty

main : E! Unit =
  let t = {sku=dup "a", qty=1} in
  out (fmt "{}\n" (show (upd t)))
```

```console
$ vibe check r4b.vibe ; echo $?
0
$ vibe run r4b.vibe
[9, 9]
```

The answer is `[9, 1]`. `u.qty` is 9 and `t.qty` is 1 — `t` was never moved, and
§4.3 promises in-place reuse only on a *uniquely owned* base.

`own.rs:188-193` records the update as in-place when the base is an owned name
that has not been moved yet. "Not moved yet" is not the same as "not used
again": `t.qty` is a `Field`, which `own.rs:166` walks in `Mode::Borrow`, so it
is not a move and does not disturb the condition. Codegen then emits
`vb_set_fields` (`codegen.rs:804-807`), which writes through the object and
returns the same pointer (`vibert.c:278-282`):

```c
VbVal t2 = vb_set_fields(v_t_1, 1, t2_ix, t2_vs);
VbVal v_u_3 = t2;
vbret = vb_push(vb_single(vb_field(v_u_3, 1)), vb_field(v_t_1, 1));
```

Writing the same function so the update is under a `.field` — which puts it in
`Mode::Borrow` and suppresses the in-place decision — emits `vb_with` and prints
`[9, 1]`. The two spellings disagree.

**Today:** a silent wrong answer, with no diagnostic and no memory error.
**Under exact drop:** the same wrong answer, plus a double free — `u` and `t` are
the same pointer and both are owned names.

### 7. An escaping closure may capture a borrowed parameter

```
mod A5

hold (s:&Str) : Unit -> Size = \u -> len s

main : E! Unit = out (fmt "{}\n" ((hold (concat "abc" "def")) ()))
```

```console
$ vibe check a5.vibe ; echo $?
0
$ vibe run a5.vibe
6
```

The emitted C stores the borrowed pointer in the closure environment:

```c
VbVal t5 = vb_clos(vbl_2, "<lambda>", 2);
t5 = vb_apply1(t5, v_s_1);
```

`escapes` (`own.rs:243`) correctly marks the lambda, so its captures are walked
in `Mode::Own` — but `s` is a `&` parameter, `is_affine(Ty::Ref)` is `false`
(`own.rs:70`), so it is not in the owned set and `use_var` does nothing. §4.2's
"if a value has to outlive the call, it is either owned or it is a compile error"
is not enforced for the capture case.

The declared-`&`-return version of the same thing is in the repository already —
`examples/ledger.vibe:29`, `top (ts:&Vec Tx, len ts>0) : &Tx`, which §4.4 says is
not expressible:

```
mod A6

type Tx = { sku:Str, qty:U32 }

first (ts:&Vec Tx) : &Tx = ts |> max_by (\t -> t.qty)
```

```console
$ vibe check a6.vibe ; echo $?
0
```

**Today:** benign. `vb_release` cancels on `VB_CLOS`, and the taint in
`escape.rs` covers the `ext c` direction.
**Under exact drop:** use-after-free when the closure is called, or double free
when the `&Tx` result is dropped as owned per §4.2.

### 8. `to_cstr` aliases a `Str` into a type the checker does not track

```
mod A8

peek (s:&Str) : E! Unit =
  let p = to_cstr (concat s s) in
  out (from_cstr p)

main : E! Unit = peek "hi"
```

```console
$ vibe check a8.vibe ; echo $?
0
$ vibe run a8.vibe
hihi
```

`to_cstr : &Str -> CStr` points into the string's bytes, and `CStr` is on
`is_affine`'s scalar list (`own.rs:74`), so `p` is invisible to the checker while
the `concat` result it points into is an anonymous temporary with no name to own
it. The same alias crosses the C boundary:

```
mod A10

ext c "stdio.h"
  puts : &CStr -> E! I32
end

give (s:Str) : E! I32 = puts (to_cstr &s)
```

```console
$ vibe check a10.vibe ; echo $?
0
$ vibe run a10.vibe
hello C
8
```

**Today:** handled, bluntly. `tests/dangle.vibe` is this exact shape, and
`escape.rs`'s taint stops the frame releasing under the pointer.
**Under exact drop:** use-after-free. There is no bracket left to suppress, and
the roadmap's Task 9 step 5 is the note that this becomes per-value.

---

## What checking them would cost

The eight split three ways, and the split is not the one the roadmap's wording
suggests.

### Local checks in `own.rs`, and they are mandatory

Gaps 1, 5 and 6 are not may-alias problems. They are places where the existing
rule is stated but not applied, and conservative drop suppression cannot cover
them because in each case the aliased value belongs to the *caller*, which this
function's drop pass cannot see.

- **Gap 5** is the smallest: `use_var` (`own.rs:127-128`) returns on
  `Mode::Borrow` before consulting `self.moved`. Consult it and emit
  `own.borrow_after_move` with the same witness/fix shape. Under twenty lines.
  Expect it to fire on existing code; re-run `examples/` before believing it.
- **Gap 6** needs the in-place condition (`own.rs:188-193`) to read "owned, not
  moved, *and not read again in this scope*". The last clause is a last-use
  question, and last use is exactly what Task 1 of the drop plan computes, so
  after Task 1 this is one extra condition. Before Task 1 the conservative
  version is one line: delete the `inplace` set and always emit `vb_with`. That
  gives up §4.3's optimisation and fixes a wrong answer, which is the right trade
  to make today rather than after the drop work.
- **Gap 1** needs `Ty::Ref` rejected in a declared return type, plus the same
  rejection for a lambda whose body returns a `&` parameter (gap 7's first half).
  One condition in the signature check. The cost is not the code: it is
  `examples/ledger.vibe::top`, which has to be rewritten to return an index or a
  copy — which is what §4.4 has always said to do, and which makes the example an
  honest demonstration of the language instead of a counterexample to its own
  spec.

Call this a day each including fallout, not a research project.

### Conservative suppression, which the roadmap already prefers

Gaps 2, 3, 4 and 8 are one shape: a value that *may* alias something the caller
owns. The roadmap's stated preference — count the leak rather than take the
unsoundness — covers all four, and covering them does not need a borrow checker.

It needs one forward, syntactic bit on each binder: *maybe-shared*. Set it when
the binder's defining expression is a call to a prelude function whose result is
drawn from a `&` argument (`get`, `max_by`, `min_by`, `sum`, `fold`, `rev`,
`filter`, `sort_by`, `push`, `set`, `concat_vec`), a pattern binder off a
scrutinee that was not consumed, a `.field` read, or anything derived from a `&`
parameter. Propagate it through `let`, `<-`, match arms and the structures built
from them. Suppress the drop of anything carrying it, and count the
suppressions — the roadmap's "How to tell it is working" already asks for that
counter.

This is the traversal `own.rs` already runs, plus a flag. Call it eighty lines.
The prelude half of the relation has to be written down somewhere, and the honest
place is `types.rs::PRELUDE_SIGS` — seventy lines, written once, by a human, not
by the generator. §4.2's ban is on lifetime annotations in the *source language*;
annotating the compiler's own builtin table is not that.

What it costs is leaks proportional to how much of a function's data came from
outside it. It does not leak in the shape the drop plan exists for: `serve`'s
per-iteration allocations are built fresh in the loop body and never carry the
bit. Gap 3 additionally forces a runtime decision — dropping a vector must be
able to be shallow — which belongs in Task 7 and is currently unstated there.

### The part that really is bigger than the plan

Gap 7's second half and the general case of gap 1 — a user-written function that
returns a value derived from a borrowed parameter, or stores one in an escaping
closure — cannot be caught by a table of prelude signatures. Catching them means
inferring, through arbitrary user code, which argument a result borrows from.
That is region inference, and it is the "minimal lifetime mechanism, preferably
inferred, not written" that §4.4 names as the thing to build if §16.1's cost
turns out to be real.

Nothing in the drop plan needs it, provided gap 1's signature-level rejection
lands. Rejecting the shape outright is what §4.4 already prescribes, and it is a
condition, not an analysis.

---

## Verdict

**Comparable to the drop plan, not larger** — three local checks that are each
about a day, one flag threaded through a traversal that already exists, and one
runtime decision about shallow drops. That is smaller than the roadmap's nine
tasks, and half of it (last use, for gap 6) is work Task 1 does anyway.

It becomes larger than the plan only under one reading: that "the aliasing rules
get checked" means actually checked, by inference, through user code. That
reading requires region inference, and §3's precondition does not require it as
long as the language keeps §4.4's promise that a borrow-derived return is a
compile error rather than a value. Gap 1 is the one place where the compiler
currently breaks that promise, and closing it is a condition on a signature, not
an analysis.

Two findings that change the order of work rather than its size:

- **Gap 6 is a bug on `master`**, not a latent one. `r4b.vibe` prints `[9, 9]`
  where `[9, 1]` is correct, silently, and the one-line conservative fix is
  available now. It should not wait for the drop plan.
- **Gap 1 is the cheapest and the most load-bearing.** A five-line `launder`
  turns any borrow into an owned value, and the repository's own reference
  program relies on the same shape. Every other gap in the provenance group
  becomes a special case of it. Schedule it before anything in the drop plan
  after Task 1.

# Static Drop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the per-frame bump allocator with exact, deterministic deallocation at the point the owner dies, so that a program which never returns does not grow.

**Architecture:** The frontend already computes, and then discards, the point in the AST where each owned value stops being owned. That information becomes a drop table, the C backend learns to act on it, and `vb_mark` / `vb_release` / the bump allocator are deleted once it does. The frontend work lands first and emits nothing, so the first six tasks cannot regress a running program.

**Tech Stack:** Rust (no dependencies — `Cargo.toml` has an empty dependency graph), C99 runtime in `runtime/vibert.c`, `cargo test` as the end-to-end suite.

**Spec:** [`vibelang-spec.md`](../vibelang-spec.md) §4 (Ownership), in particular §4.2 (the one rule), §4.5 (arenas), §4.6 (closures and escape analysis); §11 (backend); §16.1 and §16.6 (open questions this work touches).

**Companion document:** [`backend-roadmap.md`](backend-roadmap.md) — the Cranelift/native backend plan, written in parallel. This document does not describe that backend; it describes what Static Drop needs from it, in Task 9 and in "Open decisions".

## Global Constraints

- No dependencies. The compiler is a single Rust crate with an empty dependency graph; keep it that way (§11.3: "Nessun GC, nessun refcount, nessun handler di segnale").
- Generated C stays portable C99: no statement expressions, no VLAs, no GNU extensions (`src/codegen.rs` header comment).
- No lifetime annotations enter the language. §4.2: "Non esistono annotazioni di lifetime." Any design that requires the generator to write an annotation has failed the objective this work exists to serve.
- Every diagnostic keeps the existing shape: semantic `path`, `witness`, mechanical `fix` (README, *Diagnostics*).
- `cargo test` is green before and after every task. The suite asserts on emitted C (`tests/escape.rs`), on ownership diagnostics (`tests/own.rs`), and on the byte-identical canonical projection (`tests/view.rs`).
- Each task ends in a commit. Do not batch.

---

## 1. The problem

Allocation today is a bump pointer. `vb_alloc` (`runtime/vibert.c:29`) rounds the request to 16 bytes, hands out the next slice of a 1 MiB chunk, and never walks backwards. There is no `free` for a single object; the only way memory returns is `vb_release` (`runtime/vibert.c:54`), which frees whole chunks down to a mark and rewinds `used`.

Two things decide when that happens, and both are decided per *frame*:

- `escape::releasable` (`src/escape.rs`) asks, statically, whether a function or anything it can reach calls an `ext c` symbol. If it does, the function is tainted and brackets nothing, because C may keep the pointer for as long as it likes, and the taint propagates to callers since the release happens at the outermost frame.
- If the function is not tainted, `src/codegen.rs:335` wraps its body: `VbMark vbm = vb_mark();` before, `vb_release(vbm, vbret);` after. At run time `vb_release` looks at the result's tag and **cancels entirely** if it is `VB_STR`, `VB_OBJ`, `VB_VEC`, `VB_CLOS`, `VB_CSTR` or `VB_PTR`, because that value is what escaped.

This works, and `examples/churn.vibe` is the demonstration. `work` builds a five-thousand-element vector, reverses it, and returns its length; `spin` calls it two thousand times. `work` returns a `Size`, so nothing cancels its release, and the frame gives back everything it allocated on the way out:

```console
$ /usr/bin/time -l ./churn
10000000
        1556480  maximum resident set size
```

The README records that the same program with the release suppressed peaks at 325 MB. The number that matters is not the ratio; it is that the figure is flat.

It is flat *because `work` returns*. Look at what `spin` compiles to. A self-tail-call is not a call: `src/codegen.rs:399-418` recognises it, evaluates the arguments into temporaries, assigns them over the parameters, and emits `continue` inside the `for (;;)` that wraps every function body. So `spin`'s `vb_mark()` runs once, at entry, and its `vb_release(vbm, vbret)` sits textually *after* the loop. Two thousand iterations happen between those two statements. Nothing `spin` itself allocates is released until `spin` returns, and `churn` survives only because the interesting allocation was pushed down into a callee whose frame does return.

Now take the loop that does not return — an event loop, an accept loop, a supervisor:

```
serve (s:Socket) : E! Unit =
  c <- accept &s ;
  handle c ;
  serve s
```

The release is emitted at the frame's single exit point, and control never reaches it. Every iteration's allocations accumulate in chunks that `vb_alloc` keeps `malloc`-ing, and the process grows without bound until it is killed. Per-frame release is not a weak answer for this shape; it is not an answer at all, because the frame is the thing that never ends.

Three workarounds exist today, and each costs something the project is trying not to spend:

- **`arena a in ...`** (§4.5) inside the loop body. It works — `src/codegen.rs:550` lowers it to a mark/release pair around the block — but it is a manual annotation the generator has to know to write, which is exactly the token cost Objective 1 exists to avoid. It also cannot hold a value that must survive one iteration and be handed to the next, and `vb_release` cancels on any heap-tagged result, so an arena whose block yields a `Str` releases nothing at all.
- **Push the work into a callee that returns a scalar**, the `churn` shape. This is not a general transformation. A handler that returns a record, a string, or a closure cancels its own release by construction.
- **Tail recursion**, which does not help: it is precisely what turns the loop into a frame that never returns.

`src/escape.rs` states the ceiling in its own words: "a tail-recursive loop marks once and releases once, so its iterations still accumulate; the upgrade path is a mark per loop iteration, which needs to know which values cross the back edge." Static Drop is that upgrade path, generalised: instead of a coarser bracket, no bracket at all, and a free at the exact point each value dies.

## 2. The advantage that is already paid for

`src/own.rs` is a full affine-use checker, and the information Static Drop needs is the information it computes in order to emit an error.

What it knows, concretely:

- **Which values are affine.** `is_affine` (`src/own.rs`) treats `Ty::Ref` and the scalar `Ty::Con`s (`is_num`, `Bool`, `Char`, `Unit`, `Size`, `CStr`, `Ptr`) as copied, and everything else as owned. Function parameters carry a written type, so the owned set is seeded from the signature.
- **Which binders are affine, without a written type.** `let`, `<-` and pattern binders have no annotation (§4.2), so inference records their affinity in `Checked::affine`, keyed by `(file, line, col, name)` where the span is that of the expression the binder scopes over. `State::scope` reads that map to extend the owned set, and drops from the outer set any name a binder shadows.
- **Where each value is consumed.** `State::walk` carries a `Mode` — `Own` at a consuming position, `Borrow` under `&`, under `.field`, and at a call head. `use_var` records the consuming use in `moved: HashMap<String, Span>`, or, if the name is already there, emits `own.use_after_move` naming the first move as the witness and `&x` / `dup x` as the fix.
- **How alternatives combine.** `Match` arms each restart from the state before the match and union afterwards: a value moved in any arm is moved after it.
- **Which closure captures are moves.** `escapes` collects the spans of lambdas whose value reaches the function's result — through match arms, `let` and `<-` bodies, and any tuple, list or record built in that position — and a capture inside one of those is walked in `Mode::Own` rather than `Mode::Borrow` (§4.6).
- **Which record updates are in place.** `{r with f = v}` on a name that is owned and not yet moved is recorded in `inplace`.

What it does with all of that: `check` returns `Vec<Diag>` and `inplace_updates` returns the `{r with ...}` spans. `run` builds one `State` per function, and at the end of each function the `moved` map — a name-to-span record of exactly where each owned value stopped being owned — goes out of scope and is dropped on the floor. The analysis that would place every `free` in the program already runs on every build, and its result is thrown away.

Two honest gaps, because the map is not literally the drop table:

- `moved` records the **first consuming use**, which is the point after which the value is no longer this scope's to free — so a moved value needs *no* drop here, and the map is a suppression list rather than a placement list. The placement a drop needs is the **last use of a value that is never moved**: a binder read only through borrows, or never read at all, must be freed at the end of the scope that binds it. Nothing records that today.
- The traversal visits borrows, but `use_var` returns early on `Mode::Borrow` and on names outside the owned set, so a borrow-only last use leaves no trace. Task 1 adds the recording; the traversal itself does not change shape.

## 3. Before this work can start

- `cargo test` green on `master`, and the `examples/churn.vibe` figure reproduced locally. It is the before-picture, and the after-picture has to be compared against a number measured on the same machine.
- **§4.6 implemented as an error, not as a guess.** `src/own.rs` says outright that a lambda stored in a structure that a callee then returns is missed, and that deciding conservatively costs no soundness "because the direction it errs in is *read*, and reads are already checked." That sentence stops being true the moment drops are emitted: under-approximating escape then means freeing something that escaped. §4.6 already asks for an error demanding an explicit `move` when the analysis cannot decide. That error must exist and fire before Task 8, and it is the gate on the whole second half of this plan.
- **A decision about aliasing.** The README states that there is no full borrow checker: affine use is checked, the aliasing rules beyond it are not. Under a bump allocator an unsound alias costs nothing. Under exact drop it is a double free. Either the aliasing rules get checked, or Task 8 is not safe to enable by default. This is not scheduled below; it is a precondition, and it may be larger than this plan.
- **An allocator that can free one object.** `vb_alloc` cannot, by construction. Task 7 is that work and everything after it depends on it.
- The backend roadmap's IR decision, or an explicit agreement to defer it. See "Open decisions".

---

## File structure

- `src/own.rs` — gains a drop table beside the diagnostics. The affine traversal stays one traversal; do not fork a second walker, the two analyses must agree by construction.
- `src/view.rs` — gains a projection that prints the drop table, so the frontend phase is observable and testable with no backend work. This follows the existing projection pattern (`Mode::Canon | SigOnly | Explicit | Flow`).
- `src/main.rs` — CLI wiring for that projection, alongside `--flow`.
- `src/codegen.rs` — consumes the drop table exactly as it already consumes `inplace` and `releasable`: a side table keyed by span, threaded through `Gen`.
- `runtime/vibert.c`, `runtime/vibert.h` — the allocator swap and, later, the removal of `vb_mark` / `vb_release`.
- `tests/drop.rs` — new; asserts on the printed drop table and, from Task 8, on emitted C. Model it on `tests/escape.rs`, which asserts against the body of one named generated function so a claim about `vbf_work` cannot be satisfied by something in `vbf_main`.
- `examples/loop.vibe` — new; the non-returning loop that is the point of the exercise.

---

## Task 1: The drop table, for the easy case

The easy case is a `let`-bound affine value that is never moved: it dies at the end of the body it scopes over.

**Files:**
- Modify: `src/own.rs`
- Modify: `src/view.rs`, `src/main.rs`
- Create: `tests/drop.rs`
- Create: `tests/drop_let.vibe`

**Interfaces:**
- Produces: `pub struct DropSite { pub name: String, pub at: Span, pub path: String }` and `pub fn drop_points(m: &Module, ck: &Checked) -> Vec<DropSite>` in `src/own.rs`; `pub fn drops(m: &Module, ck: &Checked, sites: &[DropSite]) -> String` in `src/view.rs`. `at` is the span the drop is emitted *after* — for this task, the span of the `let` body.
- Consumes: `Checked::affine`, already populated by inference.

- [x] **Step 1: Write the failing test**

```rust
// tests/drop.rs
use std::process::Command;

const VIBE: &str = env!("CARGO_BIN_EXE_vibe");

fn drops(file: &str) -> String {
    let o = Command::new(VIBE)
        .args(["view", file, "--drops"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).expect("utf-8")
}

#[test]
fn a_let_bound_value_nothing_takes_is_dropped_at_the_end_of_its_scope() {
    let d = drops("tests/drop_let.vibe");
    assert!(d.contains("Drop.keep.body: drop s"), "{d}");
}
```

```
;; tests/drop_let.vibe
mod Drop

keep (n:U32) : U32 =
  s = concat "a" "b"
  n + u32 (len &s)
```

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test --test drop`
Expected: FAIL — `--drops` is not a recognised option.

- [x] **Step 3: Record the binders that reach the end of their scope**

In `State`, add `drops: Vec<DropSite>`. In `walk`, the `Let(n, v, body)` arm already computes `inner` via `scope`; after walking the body, any name in `inner` that is not in `self.moved` is still owned at that point:

```rust
Let(n, v, body) | Bind(n, v, body) => {
    self.walk(v, mode, owned);
    let bound = [n.clone()];
    let inner = self.scope(owned, &bound, body.span);
    self.walk(body, mode, &inner);
    if inner.contains(&n.as_str()) && !self.moved.contains_key(n) {
        self.drops.push(DropSite {
            name: n.clone(),
            at: body.span,
            path: self.path.clone(),
        });
    }
    self.moved.remove(n);
}
```

The `remove` matters: the name leaves scope here, and a later binder of the same name is a different value.

- [x] **Step 4: Expose it**

`run` returns the drops alongside the diagnostics and the in-place set; `drop_points` is the third accessor over the same `run`. Add `Mode::Drops` handling in `src/main.rs` next to `"--flow" => o.view = view::Mode::Flow`, and a `view::drops` that prints one line per site, `<path>.body: drop <name>`, sorted by span so the output is deterministic.

- [x] **Step 5: Run the tests**

Run: `cargo test`
Expected: PASS, and every pre-existing test still passes — nothing in codegen has changed.

- [x] **Step 6: Commit**

```bash
git add src/own.rs src/view.rs src/main.rs tests/drop.rs tests/drop_let.vibe
git commit -m "own: record drop points for let-bound values"
```

---

## Task 2: A move suppresses the drop

**Files:**
- Modify: `src/own.rs`
- Modify: `tests/drop.rs`
- Create: `tests/drop_move.vibe`

**Interfaces:** unchanged from Task 1.

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn a_value_given_away_is_not_dropped_by_the_giver() {
    let d = drops("tests/drop_move.vibe");
    assert!(!d.contains("drop s"), "`s` belongs to `take` now:\n{d}");
    assert!(d.contains("Drop.take"), "`take` owns its parameter and drops it:\n{d}");
}
```

```
;; tests/drop_move.vibe
mod Drop

take (s:Str) : U32 = u32 (len &s)

give (n:U32) : U32 =
  s = concat "a" "b"
  n + take s
```

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test --test drop`
Expected: FAIL — `s` is still reported as dropped in `give`, and `take` reports nothing.

- [x] **Step 3: Implement**

The suppression in `give` is the `!self.moved.contains_key(n)` guard from Task 1; if it does not already hold, the bug is that the call argument was walked in `Mode::Borrow`. Check `App`: the head is borrowed, the arguments inherit `mode`.

The other half is new. `run` seeds `owned` from the affine parameters and never revisits them; a parameter not moved by the body dies when the frame does. After `st.walk(&f.body, Mode::Own, &owned)`, push a `DropSite` for every name in `owned` absent from `st.moved`, with `at` the span of `f.body` — with one exception, handled in Task 4: a parameter whose value reaches the result must not be dropped.

- [x] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/own.rs tests/drop.rs tests/drop_move.vibe
git commit -m "own: a moved value is dropped by its new owner, not its old one"
```

---

## Task 3: Match arms drop separately

This is the first genuinely hard case. `walk` unions the arms — a value moved in *any* arm is moved after the match — which is right for diagnostics and wrong for drops. If `s` is consumed in the first arm and merely read in the second, then on the second path it is still alive when the arm ends, and it must be freed **there**, at the end of that arm, not after the match where the first path would free it twice.

Placement is therefore per arm, on the join edge. This is the general shape of conditional ownership — a value owned on one path and merely borrowed on another — and the usual answer to it is a run-time drop flag: a hidden boolean, set where the value is consumed, tested where it would be freed. Vibelang should not need one. Every arm has exactly one exit, `match_chain` already emits it (`src/codegen.rs:614-620` assigns each arm's value into one destination temporary), and there is no other way for control to leave an arm: no early return, no `break`, no exceptions, no unwinding (§11.2). Placement is therefore decidable statically for every path. If a case is found where it is not, that case is a candidate for the §4.6 error rather than for a flag — adding one would put a branch back into the "zero cost" this plan is for.

**Files:**
- Modify: `src/own.rs`
- Modify: `tests/drop.rs`
- Create: `tests/drop_arm.vibe`

**Interfaces:** `DropSite.at` for an arm drop is the span of that arm's body, so the backend can attach the free to the statement that assigns the arm's result.

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn an_arm_that_keeps_the_value_drops_it_and_the_arm_that_gives_it_away_does_not() {
    let d = drops("tests/drop_arm.vibe");
    let lines: Vec<&str> = d.lines().filter(|l| l.contains("drop s")).collect();
    assert_eq!(lines.len(), 1, "exactly one arm still owns `s`:\n{d}");
}
```

```
;; tests/drop_arm.vibe
mod Drop

take (s:Str) : U32 = u32 (len &s)

pick (b:Bool) (s:Str) : U32 =
  ?b |True  -> take s
     |False -> u32 (len &s)
  end
```

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test --test drop`
Expected: FAIL — either no drop or two.

- [x] **Step 3: Implement**

In the `Match` arm of `walk`, the loop already restarts `self.moved` from `before` for each arm. Inside that loop, after walking the arm body, record a drop for every name that is in `visible`, absent from `self.moved`, and **present in the union `after`** — that last condition is what distinguishes "this arm kept it" from "no arm ever consumed it", the second case belonging to the enclosing scope's drop, not to the arms.

Because `after` is only complete once every arm has been walked, this needs two passes over the arms, or a deferred fixup after the loop. Two passes is clearer; the traversal is not hot.

- [x] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/own.rs tests/drop.rs tests/drop_arm.vibe
git commit -m "own: place drops per match arm, not after the match"
```

---

## Task 4: Nothing that escapes is dropped

A value in result position belongs to the caller. So does a value reachable from the result: a component of a returned tuple, list or record, a capture of a returned closure. `src/escape.rs::escapes` already computes an approximation of exactly this relation for lambdas, and `src/codegen.rs::tail` already knows which positions are tail positions.

The approximation is the danger. Today it errs toward "read" and that costs nothing. Here, missing an escape means emitting a free for memory the caller is about to use. Do not widen the existing approximation; narrow it, and where it cannot decide, raise the §4.6 error demanding an explicit `move`.

**Files:**
- Modify: `src/own.rs`
- Modify: `tests/drop.rs`
- Create: `tests/drop_escape.vibe`

**Interfaces:** `own::drop_points` gains no new signature; it gains the escape set as an input, computed the same way `run` already computes `escaping`.

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn a_value_that_leaves_through_the_return_is_never_dropped() {
    let d = drops("tests/drop_escape.vibe");
    assert!(!d.contains("drop s"), "`s` is the result:\n{d}");
    assert!(!d.contains("drop t"), "`t` is inside the result:\n{d}");
}
```

```
;; tests/drop_escape.vibe
mod Drop

direct (a:Str) : Str =
  s = concat &a "!"
  s

nested (a:Str) : (Str, U32) =
  t = concat &a "?"
  (t, 1)
```

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test --test drop`
Expected: FAIL — both are reported as dropped at the end of their scope.

- [x] **Step 3: Implement**

Add a result-reachability set over the same shape as `escapes`: the spans of `Var` nodes in result position, through `Let` / `Bind` bodies, match arms, and any tuple, list or record built there. A `DropSite` is suppressed when the binder's name is read at one of those spans. Where the expression in result position is a call whose argument was this value, the value has been moved already and Task 2's suppression covers it.

- [ ] **Step 4: Decide the undecidable case out loud**

Where reachability cannot be decided — a value stored into a structure passed to a callee that may return it — emit the §4.6 error: `own.escape_unknown`, witness naming the value and the call, fix `move`. Do not emit a drop and do not silently suppress one. A missing drop leaks and is recoverable; a wrong drop is a use-after-free.

Deferred, deliberately, to the Task 8 gate. `move` is spec'd (§4.6) and does not
exist in the language, so the error has no fix to name, and every lambda handed
to `map`/`filter`/`fold` would raise it — the reference program included —
while drops are still inert. What landed instead is the suppression: a name that
can leave through the result is never a drop site, including the base of a field
read in result position, which is `r.f` handing out a pointer into `r`. That
errs toward the leak. The error, and the `move` that answers it, are part of the
precondition list above and stay the gate on Task 8.

- [x] **Step 5: Run the tests**

Run: `cargo test`
Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add src/own.rs tests/drop.rs tests/drop_escape.vibe
git commit -m "own: suppress drops for values reachable from the result"
```

---

## Task 5: The back edge

This is the task that fixes the event loop, and it is the reason the whole plan exists.

`src/codegen.rs:399-418` compiles a self-tail-call by evaluating the arguments into temporaries, assigning them over the parameter variables, and issuing `continue`. The old parameter values are overwritten in place. Under Static Drop, a parameter that is still owned at the back edge, and whose value was not itself moved into one of the new argument expressions, must be dropped **before** the assignment, once per iteration. That is the "mark per loop iteration" `src/escape.rs` names as the upgrade path, except that it frees a value rather than rewinding a region.

**Files:**
- Modify: `src/own.rs`
- Modify: `tests/drop.rs`
- Create: `tests/drop_loop.vibe`

**Interfaces:** `DropSite.at` for a back-edge drop is the span of the self-tail-call expression. The backend must emit the free *before* the parameter assignments it precedes, not after.

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn a_parameter_replaced_on_the_back_edge_is_dropped_each_iteration() {
    let d = drops("tests/drop_loop.vibe");
    assert!(d.contains("Drop.spin"), "the loop body drops its own garbage:\n{d}");
}
```

```
;; tests/drop_loop.vibe
mod Drop

spin (k:U64) (s:Str) : U32 =
  ?k |0 -> u32 (len &s)
     |_ -> spin (k - 1) (concat &s "x")
  end
  %k
```

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test --test drop`
Expected: FAIL — no drop, because the arm neither moves `s` nor returns it.

- [x] **Step 3: Implement**

Recognise the self-tail-call in `walk` with the same test codegen uses (`ExprKind::App` whose head is `Var(n)` with `n == f.name`, not shadowed, arity matching). Walk the argument expressions first, then record a drop at that span for every parameter still owned and not moved by them.

Note the ordering hazard: `concat &s "x"` *borrows* `s` to build the new value, so the drop must be sequenced after the new argument has been computed and before the assignment overwrites the parameter. Encode that in the site, not in a convention the backend has to remember.

- [x] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/own.rs tests/drop.rs tests/drop_loop.vibe
git commit -m "own: drop parameters replaced across a self-tail-call"
```

---

## Task 6: The C backend ignores all of it

The point of this task is a commit where the entire frontend analysis is in the tree, tested, and provably inert. It makes the first half of the plan landable on `master` months before the second half is ready.

**Files:**
- Modify: `src/codegen.rs`
- Modify: `tests/drop.rs`

**Interfaces:** `Gen` gains `drops: HashMap<(usize, usize, usize), Vec<DropSite>>`, populated in the constructor beside `inplace: crate::own::inplace_updates(m, ck)` and `releasable: crate::escape::releasable(m, ck)`. Keying by `(file, line, col)` matches the existing convention for `inplace`.

- [x] **Step 1: Write the failing test**

```rust
/// Copied from the helper at the top of `tests/escape.rs`: runs
/// `vibe build --emit-c` into a temp directory and reads the `.c` back.
fn emit_c(file: &str, stem: &str) -> String {
    let dir = std::env::temp_dir().join("vibe-drop-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out = dir.join(stem);
    let o = Command::new(VIBE)
        .args(["build", "--emit-c", file, "-o"])
        .arg(&out)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    std::fs::read_to_string(out.with_extension("c")).expect("the emitted C")
}

fn snapshot(stem: &str) -> String {
    let p = format!("{}/tests/snap/{stem}.c", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("no snapshot at {p}"))
}

#[test]
fn drop_points_change_no_emitted_c_yet() {
    // The generated C for every example is identical to the committed snapshot.
    // Update the snapshot only in Task 8, deliberately.
    for (file, stem) in
        [("examples/churn.vibe", "churn"), ("examples/ledger.vibe", "ledger"), ("examples/hello.vibe", "hello")]
    {
        assert_eq!(emit_c(file, stem), snapshot(stem), "codegen must not move yet: {file}");
    }
}
```

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test --test drop`
Expected: FAIL — no snapshots exist yet.

- [x] **Step 3: Generate the snapshots from `master`'s output, then thread the table through**

`Gen` holds the drop table and does nothing with it. `#[allow(dead_code)]` on the field is acceptable here and should be removed in Task 8.

Generating the snapshots found a defect this task exists to catch: the record
and constructor descriptors were emitted in `HashMap` order, so the same source
produced different C on different runs. Both tables are sorted by name now, and
the snapshots are what makes that stay true.

- [x] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS, including the byte-identical C.

- [x] **Step 5: Commit**

```bash
git add src/codegen.rs tests/drop.rs tests/snap
git commit -m "codegen: carry the drop table without acting on it"
```

---

## Task 7: An allocator that can free one object

`vb_alloc` cannot free a single object; nothing built on it can implement a drop. This task replaces the allocator and keeps the bump path alive behind a flag until Task 9 retires it.

**Files:**
- Modify: `runtime/vibert.c`, `runtime/vibert.h`
- Modify: `tests/e2e.rs`

**Interfaces:** `void *vb_alloc(size_t n)` keeps its signature. New: `void vb_free(void *p)`. `vb_mark` / `vb_release` keep working unchanged while the bump path is still selected.

The allocator itself is **undecided**. `malloc`/`free` is the honest first move: it is in libc, it adds no dependency, and it makes the plan measurable. A size-classed free list is the obvious follow-up if measurement asks for it, and a measurement that does not ask for it is the reason not to write one. Decide with `examples/churn.vibe` in hand, not before.

- [x] **Step 1: Write the failing test**

Correctness first; the memory assertion is Task 8's. In `tests/e2e.rs`, beside the existing example tests:

```rust
#[test]
fn churn_still_computes_the_same_answer_without_the_bump_allocator() {
    let bin = build_with(["--alloc=exact"], "examples/churn.vibe", "churn-exact");
    let o = Command::new(&bin).output().expect("the program runs");
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "10000000");
}
```

`build_with` runs `vibe build` with the extra flags and returns the path of the produced binary; write it next to the existing build helper in that file. The flag sets `-DVB_EXACT_DROP` on the `cc` invocation.

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test --test e2e`
Expected: FAIL — the flag does not exist.

- [x] **Step 3: Implement**

Header-level selection (`#ifdef VB_EXACT_DROP`) keeps both paths in one file and lets the C compiler delete the one not chosen. Under `VB_EXACT_DROP`, `vb_alloc` is `calloc` — the bump path `memset`s to zero and code downstream relies on it — and `vb_free` is `free`. Every runtime constructor (`vb_str`, `vb_obj`, `vb_clos`, `vb_vec_new`, …) allocates through `vb_alloc` already, so no call site changes.

- [x] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS. Programs leak everything under the new flag; that is expected until Task 8.

Measured on the machine that will measure the after-picture, `examples/churn.vibe`:

```console
$ /usr/bin/time -l ./churn            # bump
        1556480  maximum resident set size
$ /usr/bin/time -l ./churn-exact      # --alloc=exact, no drops emitted yet
      330498048  maximum resident set size
```

330 MB is the whole working set of the program with nothing freed, and it is
what Task 8 has to bring back down to the flat figure.

- [x] **Step 5: Commit**

```bash
git add runtime/vibert.c runtime/vibert.h tests/e2e.rs
git commit -m "runtime: an allocation path that can free one object"
```

---

## Task 8: `_Drop_T`, and drops emitted

**Files:**
- Modify: `src/codegen.rs`
- Modify: `tests/drop.rs`
- Create: `examples/loop.vibe`

**Interfaces:** for each record and variant type in `Checked::data`, codegen emits the type's drop function — `_Drop_T` in the design's terms, spelled `static void vbd_<T>(VbVal v);` to match the `vbf_` / `vbi_` / `vbe_` prefixes `src/codegen.rs` already uses. It frees the nested strings and vectors the value owns, then the value itself. At each drop site, `vbd_<T>(x);` is emitted for the statically known `T`.

- [ ] **Step 1: Write the failing test**

```rust
/// Peak resident set of a run, in bytes, read from `/usr/bin/time -l` — the
/// same tool the README's `churn` figure was measured with. The crate has no
/// dependencies, so there is no `libc::getrusage` to call; the platform gate is
/// the price of not adding one.
#[cfg(target_os = "macos")]
fn peak_rss(bin: &std::path::Path) -> u64 {
    let o = Command::new("/usr/bin/time").arg("-l").arg(bin).output().expect("time runs");
    let err = String::from_utf8_lossy(&o.stderr);
    let line = err
        .lines()
        .find(|l| l.contains("maximum resident set size"))
        .unwrap_or_else(|| panic!("no rss line in:\n{err}"));
    line.split_whitespace().next().expect("a number").parse().expect("a number")
}

#[cfg(target_os = "macos")]
#[test]
fn a_loop_that_never_returns_does_not_grow() {
    // examples/loop.vibe allocates per iteration, keeps nothing, uses no arena.
    let bin = build_with([], "examples/loop.vibe", "loop");
    let rss = peak_rss(&bin);
    assert!(rss < 8 * 1024 * 1024, "the loop grew to {rss} bytes");
}
```

```
;; examples/loop.vibe
mod Loop

step (k:U64) (acc:U32) : U32 =
  ?k |0 -> acc
     |_ -> step (k - 1) (acc + u32 (len &(concat "x" "y")))
  end
  %k

main : E! Unit =
  out (show (step 2000000 0))
```

The threshold is a ceiling, not a benchmark: the assertion is that the figure is flat, in the same sense the README's `churn` figure is flat. Do not tune it into a performance claim.

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test --test drop`
Expected: FAIL — the loop grows without bound.

- [ ] **Step 3: Emit the drop functions**

Walk `Checked::data` to generate one `vbd_<T>` per type, recursing into fields whose type is itself heap-carrying and calling `vb_free` on the payload pointers. `VbStr` owns `p`; `VbVec` owns `a` and each element; `VbObj` owns `f` and each field; `VbClos` owns `args`.

- [ ] **Step 4: Emit the drops**

At each site in the table, emit the call. Respect the ordering contract from Task 5: back-edge drops precede the parameter assignments.

**The zero-cost claim depends on a debt already recorded elsewhere.** `runtime/vibert.h` opens with it: values are dynamically tagged, arithmetic dispatches on the tag, and the upgrade path is to thread resolved types into codegen. Until that lands, a drop cannot always name a static `T` and must dispatch on the tag at run time — which is a call and a branch, not zero cost. Emit the static call where the type is known and fall back to a generic `vb_drop(VbVal)` where it is not; count the fallbacks and report the count, because that number is the remaining distance to the objective.

- [ ] **Step 5: Run the tests under a checker**

Run: `cargo test`, then rebuild the examples with `-fsanitize=address` and run them.
Expected: PASS, no double free, no use-after-free. Leaks are now visible for the first time and are findings, not noise.

- [ ] **Step 6: Commit**

```bash
git add src/codegen.rs tests/drop.rs examples/loop.vibe
git commit -m "codegen: emit exact drops at the point the owner dies"
```

---

## Task 9: Delete the bump allocator

Do this only once Task 8 has been the default for long enough to trust, and only with the escape-analysis precondition from §3 actually satisfied.

**Files:**
- Modify: `runtime/vibert.c`, `runtime/vibert.h`, `src/codegen.rs`, `src/escape.rs`
- Modify: `tests/escape.rs`, `README.md`

- [ ] **Step 1: Make the exact path the default and delete the flag.**
- [ ] **Step 2: Delete `vb_mark`, `vb_release`, `VbMark`, and the chunk list.**
- [ ] **Step 3: Delete the mark/release bracket in `src/codegen.rs:335` and the `Arena` lowering at `src/codegen.rs:550`** — see "Open decisions" for what happens to the `arena` *construct*, which is a spec question and not a codegen one.
- [ ] **Step 4: Rewrite `tests/escape.rs`.** Every assertion in it is about `vb_mark()` and `vb_release(vbm, vbret)` appearing or not appearing in a named function body. Those strings will not exist. The tests are not obsolete — the questions they ask about the C boundary still matter — but they must be rewritten against drop emission. Expect this to be the most annoying hour in the plan, and do not skip it by deleting the file.
- [ ] **Step 5: `src/escape.rs` loses its job as a memory mechanism but keeps its question.** Whether a pointer was handed to C still decides whether *we* may free it. Fold the taint into drop suppression rather than deleting the module.
- [ ] **Step 6: Update `README.md`** — the *Memory* section, the `churn` figures, the "Designed, not yet enforced" entry, and the Status lists. Move lines between lists rather than rewriting the section, as that section asks.
- [ ] **Step 7: Commit.**

---

## The Cranelift backend

Not a task in this plan. [`backend-roadmap.md`](backend-roadmap.md) owns it. What Static Drop needs from it is one thing: a drop must lower to instructions, not to a call. The C backend cannot do that — a `vbd_<T>` call is a call, and the optimiser may or may not inline it across a translation unit — so on the C path exact drop is cheap but not free. A native backend translates a drop site directly into the stores and frees it implies, at the point the owner dies, with no function-call boundary and no tag dispatch. That is where the "zero-cost" in "zero-cost memory abstraction" is actually collected.

The dependency runs the other way too, and it is the one cross-document decision: if the backend roadmap introduces an intermediate representation, drop points should be nodes in it — an `IR_Drop(Value)` — rather than a side table keyed by span. This plan deliberately uses the side table, because that is the pattern `src/codegen.rs` already uses twice (`inplace`, `releasable`) and it does not require an IR to exist. Converting a span-keyed table into IR nodes is mechanical; the reverse is not. Settle this with the backend document before Task 6, not after.

---

## The end goal

No garbage collector, and no annotation for the generator to write.

Every other language pays for memory safety in one of those two currencies. A collector costs run time, a resident set larger than the live set, and pauses at times nobody chose. Annotations cost tokens: Rust's lifetimes are the most expressive answer anyone has shipped, and they are also a notation a generator has to get right, on the first try, in a language it is being measured on.

Vibelang's bet is that §4.2's single rule — `&` parameters are borrows for the duration of the call, everything else is owned, the return value is always owned — already determines where every free goes. If that is true, the compiler can place them all, the generator writes nothing, and the binary contains no collector. The measurement that matters is the one §15 fase 0 names: **tokens until a correct program**, not source length, not benchmark throughput. An `arena` that a human has to remember to write is a token cost; so is a lifetime; so is a retry after a leak nobody caught. Static Drop removes all three, or it has not paid for itself.

---

## How to tell it is working

- `examples/loop.vibe` (Task 8) runs to completion with a flat resident set, with no `arena` in the source and no callee introduced to give the frame something to return.
- `examples/churn.vibe` still prints `10000000`, and its peak resident set is compared against the current committed figure — 1,556,480 bytes — measured on the same machine. A regression there is a real finding: the bump allocator's bulk chunk free is genuinely fast for short frames, and per-value frees may lose on that shape. Record whatever the number is; do not tune the example until it flatters the change.
- `grep -rn "arena" examples/ tests/` trends toward zero as arenas stop being the workaround for a language limitation. Arenas that remain should remain for the reason §4.5 gives — structures linear ownership does not express — and not because a loop would otherwise grow.
- No `vb_mark` or `vb_release` in emitted C (Task 9). `tests/escape.rs`, rewritten, is the fence.
- Examples built with `-fsanitize=address` and with valgrind: no double free, no use-after-free, and a leak report that is now meaningful. Under a bump allocator every leak was invisible by construction; the first honest leak numbers this project has ever had will arrive here, and some of them will be old bugs rather than new ones.
- The count of drop sites that fall back to run-time tag dispatch (Task 8, step 4) goes down over time. While it is above zero, "zero-cost" is a goal and not a claim, and the README should say so in the register it already uses for this kind of thing.

---

## What could go wrong

**An under-approximated escape becomes a use-after-free.** This is the one that matters. Today `src/own.rs` errs toward "read" and the comment says plainly that this "costs no soundness here". After Task 8 the same error frees live memory, silently, on a path a test may not take. Mitigation: Task 4's `own.escape_unknown` error, and the refusal to guess. If the error turns out to fire constantly on real code, that is data about §16.1 — the open question asking whether the absence of lifetimes has a cost nobody has measured — and it should be reported as such rather than worked around by widening the analysis.

**Aliasing that the affine checker does not track.** The README is explicit: "affine use is checked, the aliasing rules beyond it are not." Every alias the checker misses is a double free once drops are real. Listed as a precondition in §3 because it may be a larger piece of work than this plan, and discovering that late would be expensive.

**`dup` does not mean what the diagnostic implies.** `own.use_after_move` offers `dup x` as a mechanical fix, but `vb_dup` (`runtime/vibert.c:465`) is `vb_str(x->p, x->n)` — a string copy, and nothing else. On a record or a vector it aborts in `vb_as_str`. Under a bump allocator the gap between the promised fix and the implemented primitive is a run-time error in an uncommon case; under Static Drop, whatever `dup` becomes has to produce a value with its own independent ownership, or the copy and the original will both be dropped. Settle what `dup` means before Task 8.

**Ownership across the C boundary.** The README notes that the exported signature passes the uniform `VbVal`, and that "the spec's `own T` / `ref T` ownership qualifiers, and the generated `<name>_free`, are not implemented yet." With per-frame release, the taint in `src/escape.rs` is a sufficient blunt answer: touch C anywhere and release nothing anywhere above. With exact drops there is no bracket to suppress, and the question becomes per value: did this pointer's ownership go to C? Task 9 step 5 folds the taint into drop suppression, which is conservative and leaks; `own T` / `ref T` is the real answer and is out of scope here.

**Performance regression on the shape the current design is good at.** A bump pointer plus a bulk chunk free is close to the cheapest thing that can be done for a frame that allocates a lot and returns a scalar. That is `churn`. Exact drop wins the case `churn` cannot express and may lose the case it does. Measure both; report both; do not pick the flattering one.

**Scope creep into the backend.** Tasks 1-6 are frontend-only and inert. Tasks 7-9 touch the runtime. None of them require the native backend, and doing this work at the same time as the backend swap means that when the resident set moves nobody will know which change moved it. Land this against the C backend first.

---

## Open decisions

These are undecided, not omitted. Deciding them silently while executing a task is the failure mode this section exists to prevent.

1. **Side table or IR node.** This plan uses a span-keyed side table because no IR exists today — `src/codegen.rs` walks the AST directly. If [`backend-roadmap.md`](backend-roadmap.md) introduces one, `IR_Drop(Value)` is the better home. Settle before Task 6.
2. **What replaces the bump allocator.** `malloc`/`free` is the honest starting point. Size classes, pools, or a free list are follow-ups that measurement may or may not justify. Unresolved until Task 7 has numbers.
3. **Whether `arena` survives.** §4.5 puts arenas in the language for structures linear ownership does not express — graphs, arbitrary sharing — and §16.6 records the open question of whether they are sufficient. Static Drop removes arenas' *other* role, as the manual workaround for loop growth. Whether the construct stays for its original purpose is a spec question, and this plan does not answer it. Task 9 deletes the mark/release lowering, not the keyword.
4. **What `dup` means on an aggregate.** See "What could go wrong". Deep copy, shared immutable value, or a diagnostic that stops offering `dup` for non-strings — undecided.
5. **Whether drops can be elided into reuse.** A value dropped immediately before an allocation of the same shape could feed that allocation instead. This is a real optimisation and a real source of bugs; it is not part of this plan and should not be smuggled into Task 8.

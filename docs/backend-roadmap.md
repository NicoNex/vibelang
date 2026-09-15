# Backend Roadmap: Cranelift as the Primary Native Backend

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Cranelift backend that turns a checked Vibelang module into native object code without a C compiler on the user's machine, while keeping the existing C backend as a fully supported, permanently maintained second target.

**Architecture:** One typed, validated IR — today's `ast::Module` plus `infer::Checked` and the side tables computed from them — is handed to a `CodegenBackend` implementation. Today's C emission becomes `CBackend`; `CraneliftBackend` is added beside it. Neither backend typechecks, infers, or decides ownership: everything they need has already been decided by the time they are called.

**Tech Stack:** Rust 2021 (`vibec`, currently zero dependencies), `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-object`, `cranelift-native`; the existing C runtime in `runtime/vibert.c`; `z3` (external binary, already used by `src/refine.rs`).

**Spec:** [`vibelang-spec.md`](../vibelang-spec.md) §10 (C interop) and §11 (Backend), both normative and both in Italian. [`README.md`](../README.md) §Memory and §The C boundary describe the shipped behaviour. Implicit drop is *not* in this plan: it belongs to `docs/static-drop-roadmap.md`, written in parallel; this plan depends on it and links to it rather than duplicating it.

---

## Global Constraints

Copied from the spec and the README. Every task's requirements implicitly include this section.

- **The generated program depends on nothing beyond libc.** Spec §11.2: *"nessuna dipendenza oltre libc"*. No GC, no refcount, no signal handler (§11.3).
- **No `setjmp`/`longjmp`, no unwind tables.** Spec §11.2.
- **Self-tail-recursion becomes a loop, emitted by the compiler, not left to an optimiser.** Spec §11.2. This is already true of the C backend (`Gen::tail` emits `for (;;)` and rebinds the parameters).
- **A mutual tail call that cannot be realised must fail visibly at compile time.** Spec §11.2 asks for `[[gnu::musttail]]` in C. Not implemented in either place today.
- **The C type mapping in spec §10.3 is normative** for `ext c` and `exp c` in *both* backends: `U8..U64` → `uint8_t..uint64_t`, `I8..I64` → `int8_t..int64_t`, `F32`/`F64` → `float`/`double`, `Bool` → `bool`, `Str` → `struct { const char *p; size_t n; }`, `CStr` → `const char *`, `&T` → `const T*`, `own T` → `T*`, record → `struct` in field order, ADT → `struct { uint8_t tag; union {...} v; }`, `Res E T` → `struct { bool ok; union { E e; T t; }; }`.
- **`ext c` refinements are assumed, not proved** — checked at the Vibelang call sites and not one step beyond (§10.1). Every `ext c` declaration is mandatorily `E!` (§5.2).
- **`exp c` preconditions travel into the header as a comment carrying the explicit warning that they are not verified** (§10.2). The current wording is `NOT VERIFIED ACROSS THE BOUNDARY`; keep it byte-identical unless the spec changes.
- **Diagnostics keep their contract**: stable `code`, semantic `path`, concrete `witness`, mechanical `fix`; `--diag=prose|struct|json`. A backend that invents a new diagnostic code adds it to the same table everything else uses.
- **Licence**: the project is GPL-3.0-or-later. The Cranelift crates are Apache-2.0 WITH LLVM-exception, which is one-way compatible into GPLv3. Any further dependency must be checked the same way before it is added.
- **`cargo build --locked` and `cargo test --locked` are the CI gates** (`.github/workflows`). Anything added to `Cargo.toml` must land with its `Cargo.lock` in the same commit.

---

## Where the code stands today

Surveyed by reading the code at commit `c0604fa`, not by reading the README. Line numbers rot; the function and file names are the durable part. The surface syntax changed immediately after this survey — whitespace is no longer significant, a match closes with `end`, a `<-` binding is terminated by `;`, and an `ext c` block closes with `end` — so quote [`vibelang-spec.md`](../vibelang-spec.md) §3, not the examples, when you need the grammar. Nothing in this plan depends on the surface syntax: it all sits behind the parser.

### The pipeline

`src/main.rs::run` is the whole driver, in this order:

1. `load::program` — resolves `Money.cents`-style qualified names by loading sibling files, and hands back both the per-file units and one flattened `ast::Module` (§9).
2. `infer::check` — Hindley–Milner over the flattened module. Returns `infer::Checked { data, ext, sigs, affine }`. **This is the typed IR.** There is no separate IR data structure; the AST plus these side tables is it.
3. `view::canon` — canonicity, per file (§3.1).
4. `own::check` — affine use.
5. `total::check` — termination measures.
6. `refine::check` — obligations, in one of three modes.
7. `codegen::generate` — C text.
8. `write` the C, the runtime (`RT_C`/`RT_H`, `include_str!`-ed into the binary), and the generated header into a sibling `.vibe-<stem>/` directory.
9. `cc()` or `archive()` — invoke `$CC` and `$AR` as subprocesses.

Steps 7–9 are what Phase 2 puts behind a trait. Steps 1–6 are the middle end and are never entered by a backend.

### Static ownership: what exists

- `src/own.rs::check` enforces affine use of owned values: parameters carrying an affine type, `let` and `<-` binders, names bound by a pattern, and captures of a closure whose value reaches the function's result. `is_affine` treats `&T`, numerics, `Bool`, `Char`, `Unit`, `Size`, `CStr`, `Ptr`, functions and effects as copyable; everything else is affine. The diagnostic is `own.use_after_move` with a `fix` of `&x` or `dup x`.
- `src/own.rs::inplace_updates` returns the spans of `{r with ...}` updates whose base is uniquely owned, so the C backend can mutate rather than copy (§4.3). It is a second return value of the same walk.
- `src/escape.rs::releasable` computes which functions may bracket their body with `vb_mark`/`vb_release`: a function may release iff neither it nor anything reachable from it calls an `ext c` symbol, because C may retain a pointer indefinitely. The taint propagates to callers by fixpoint.
- The runtime's half: `vb_release` refuses to release when the returned `VbVal` is itself heap-allocated (string, object, vector, closure), so escape through the return is handled dynamically and for free.

**What remains is implicit drop.** It is deliberately not planned here. See [`static-drop-roadmap.md`](static-drop-roadmap.md). Two things this plan needs from it, and nothing else:

- the *emission contract*: what a backend must emit at a drop point, in terms the IR can express (a call to a per-type drop function? an inline sequence? a runtime call taking a type descriptor?);
- whether drop points are computed in the middle end and attached to the IR, or recomputed per backend. Only the first answer is acceptable under the rules below, so if the drop roadmap chooses the second, that conflict must be resolved before Phase 2 starts.

### Refinements and Z3: what exists

`src/refine.rs` generates obligations (`obligations`), renders each to SMT-LIB 2 (`smt`, a pure function and therefore unit-testable without a solver), and discharges them by piping to `z3 -in`. Proved obligations are cached in a sibling `.vibe-proofs` keyed by the hash of the question asked; each obligation gets a wall-clock budget (`--prove-timeout=`, 5s default) and a timeout is reported as `refine.budget` — "the solver gave up" — never as a refutation.

**Generated today** (all of spec §7.2 except where noted):

| site | code | goal |
|---|---|---|
| `a / b`, `a % b` | `div0` | divisor ≠ 0 |
| `a + b`, `a * b` on a sized integer type | `overflow` | result within `[min, max]` of that type |
| `a - b` on an **unsigned** type | `underflow` | `a >= b` |
| `get xs i` | `bounds` | `0 <= i < len xs` |
| record literal or update with an invariant | `refine.unproven` | the invariant, with the base record's untouched fields carried through |
| call to a function or `ext c` symbol with parameter refinements | `refine.unproven` | the precondition, renamed into the caller's terms |

Hypotheses in scope: the function's own parameter refinements; the machine range of every parameter with a sized type; record invariants of parameters; `len` of containers, asserted non-negative; and the facts a constructor pattern teaches an arm — the tag, the payload's length, the payload's record invariant.

**Not generated, and worth knowing before you trust a green run:**

- **No postconditions of any kind.** Nothing a callee establishes is visible to its caller.
- **Signed subtraction is unchecked.** `arith` returns early for `-` unless the type is unsigned, so signed overflow on `-` raises no obligation.
- **An obligation whose type cannot be resolved is silently skipped.** `arith` bails when `ty_of` yields `None`; `call` bails when an argument has no SMT term. A construct that generates nothing looks exactly like a construct that was proved.
- **Floats go to `Sort::Real`.** SMT reals are exact; `F32`/`F64` are not. A proof about float arithmetic says nothing about rounding, overflow, or NaN.
- **Non-scalar values are opaque.** Only their `len` and their record invariant enter the solver.
- **`ext c` refinements name no parameters** (`Gen::sig` returns an empty parameter list for them), so an `ext` refinement can only mention names the call site itself uses.
- **Ghost functions (§7.4) are skipped entirely**, which is correct, but means nothing checks that a ghost function is itself consistent.

**The known gap, exactly.** On the reference program:

```console
$ vibe proof examples/ledger.vibe
Ledger.mk.body/0	refine.unproven	cannot prove the invariant of `Tx`: qty > 0
Ledger.mk.body/1	refine.unproven	cannot prove the invariant of `Tx`: price > 0
Ledger.mean.body/0	div0	cannot prove the divisor is non-zero
Ledger.main.body/0	refine.unproven	cannot prove the precondition of `mean`: len ts > 0
Ledger.main.body/1	refine.unproven	cannot prove the precondition of `top`: len ts > 0

$ vibe proof examples/ledger.vibe --prove
Ledger.main.body/0	refine.unproven	cannot prove the precondition of `mean`: len ts > 0
Ledger.main.body/1	refine.unproven	cannot prove the precondition of `top`: len ts > 0
```

Three of five close. The two that remain are the same fact twice: on the `Ok ts` arm of the match on `load`'s result, `len ts > 0` holds because `load` answers `Er Void` when the list is empty — and nothing carries that fact across the call. This needs an **interprocedural postcondition**, and **spec §7.1 has no syntax for one**. §7.1 says refinements live in the signature, after the type, separated by commas, and that there is no separate `where` block; every example puts them on a parameter. The fact required here is also constructor-conditioned — "*if* the result is `Ok ts`, then `len ts > 0`" — so whatever syntax is chosen must be able to mention the result and discriminate on its constructor.

### The C backend: what it actually does

`src/codegen.rs` emits portable C99. Everything is one uniform dynamically tagged value:

```c
typedef struct VbVal { uint8_t tag; union { int64_t i; uint64_t u; double f; bool b; uint32_t c; void *p; } v; } VbVal;
```

Every Vibelang function becomes `static VbVal vbf_<name>(VbVal *a)` — arguments in an array, one return value, no static types in the generated code at all. `runtime/vibert.h` says so in its own header comment and calls it debt: *"values are dynamically tagged and arithmetic dispatches on the tag. The type checker already knows every static type, so the upgrade path is to thread resolved types into codegen and emit native C operators."*

That is the single most important fact for this roadmap, and it is stated again under the rules below: **the boxed representation is a property of the IR contract, not of the C backend.** A Cranelift backend that reproduces it will be exactly as slow as the C one. Unboxing is a separate project that benefits both backends and is out of scope here.

What `codegen::generate` reaches into today:

- `ck.data.records`, `ck.data.ctors`, `ck.data.field_owner` — to emit `VbInfo` descriptors, field indices and constructor tags;
- `ck.sigs` — to type the `exp c` wrappers and the header;
- `ck.ext` — to build `ext c` calls and to recognise ext symbols used as values;
- `crate::own::inplace_updates(m, ck)` and `crate::escape::releasable(m, ck)` — **called from inside `generate()`**, which is the layering violation Phase 2 fixes.

It also *produces diagnostics*: `ffi.type` when a value cannot cross the C boundary, and `codegen.unbound` when a name has no definition to emit. `ffi.type` is a semantic check living in a backend; a second backend would have to reimplement it to get the same errors. It also means the error is not reported by `vibe check`, only by `vibe build`.

Two incidental findings from the survey, both real, neither fixed here:

- `Gen::value_ref` emits `vb_clos(vbe_<name>, ...)` for an `ext c` symbol used as a first-class value, and inserts the name into `need_wrapper` — but `Gen::wrappers` only emits wrappers for builtins and constructors. `grep -rn "vbe_" src/ runtime/` finds exactly one hit, the reference. An `ext` function passed as a value therefore emits C that does not compile. File it as a bug against the C backend; do not fix it inside a backend-extraction commit.
- `#line` directives are emitted per statement, so a C-built program is debuggable at Vibelang source level for free. Cranelift emits none without deliberate DWARF work. This is a real, permanent advantage of the C backend, and one of the reasons it is not deprecated.

### The build path

`src/main.rs::cc` writes `vibert.h`/`vibert.c`/`<stem>.h`/`<stem>.c` into `.vibe-<stem>/`, then runs `$CC -std=c11 -O2 -o <exe> <stem>.c vibert.c -I<dir>`, appending `pkg-config --cflags --libs` output for every `pkg` and `-l<lib>` for every `link` named by an `ext c` block, plus `-lm`. `src/main.rs::archive` compiles `<stem>.c` and `vibert.c` to objects, runs `$AR rcs`, and copies both headers next to the archive; it refuses when the module declares no `exp c`.

The runtime uses roughly eighteen libc/libm symbols: `malloc`, `free`, `memcpy`, `memset`, `strlen`, `snprintf`, `fopen`, `fclose`, `fread`, `fwrite`, `fputs`, `exit`, `strtoll`, `strtod`, `sqrt`, `pow`, `floor`, `fabs`. Allocation is a global chunk list (`static VbChunk *g_chunk`) bumped by `vb_alloc`; `vb_mark` captures `(chunk, used)` and `vb_release` frees every chunk newer than the mark and rewinds `used`, unless the result value is heap-allocated.

---

## Architectural rules

These are not preferences. A change that breaks one of them is wrong even if it passes the tests.

### C is not deprecated

`vibe build --emit-c` must keep working, on every platform, for every program the Cranelift backend accepts. Concretely:

- it is how `exp c` headers are produced, and §10.2 is not an afterthought — this repository treats C interop as a first-class goal. The README says it in as many words: *"C is not an escape hatch bolted on at the end, it is how sixty years of existing libraries stay reachable without anyone rewriting them."*
- it is the fallback for every platform Cranelift does not cover. Cranelift's backends are x86-64, aarch64, riscv64 and s390x; everything else — and, until proven otherwise, Windows, whose x64 ABI returns a 16-byte struct through a hidden pointer rather than in a register pair — goes through C.
- it is the readable artifact. A generated C file plus `#line` is a debugger, a second opinion, and a bug report. Cranelift IR is none of those for a user.
- it is the differential oracle. The equivalence gate in Task 3.10 exists because two independent lowerings of the same IR that agree are evidence; one lowering is not.

A commit that makes the C backend worse in order to make Cranelift simpler is to be rejected on sight.

### No external runtime dependency

A user compiling a pure Vibelang program — one with no `ext c` block — must not need a C toolchain installed. That is the entire point of the Cranelift work; nothing else it buys is worth this much effort at this stage.

**Say plainly what this implies for `runtime/vibert.c`: it is the hardest open question in this plan, and it is not settled here.** Today the runtime is C source, `include_str!`-ed into the `vibe` binary, written out at build time and compiled by the user's `cc` alongside the generated program. Cranelift emits an object file for the *generated* code only. It does not compile C, and it does not link. So two dependencies survive naively:

1. **the runtime**, which is C;
2. **the linker**, because `cranelift-object` produces a relocatable object, not an executable — and on most Unix systems the ordinary way to reach a linker is `cc`.

Options for (1) are laid out in [The open question](#the-open-question-the-runtime-without-a-c-toolchain) below, with what each costs and what evidence would settle it. Do not pick one from the armchair. Task 3.0b builds the measurements.

Option (2) is smaller but must not be forgotten: it is a real decision between invoking the platform linker directly (`ld`/`ld64`/`link.exe`, none of which are a C *compiler*, so the constraint is arguably met), shipping a linker (`rust-lld` ships with the Rust toolchain; `wild` and `mold` are separate binaries), or emitting a static archive and telling the user to link it — which fails the constraint outright.

### One IR for both backends

No typechecking and no ownership logic inside either backend. Mechanically:

- a backend may **read** `ast::Module`, `infer::Checked`, and the precomputed side tables handed to it;
- a backend may **not** call `infer::`, `own::`, `escape::`, `total::` or `refine::` functions, nor reimplement what they decide;
- a backend may **not** emit a diagnostic that another backend would not emit for the same input. If a check belongs to the language, it belongs to the middle end. `ffi.type` is the existing violation and Task 2.2 moves it.
- anything a backend needs to know that is not already in `Checked` gets computed once, in the middle end, and attached to the `Unit` handed to the backend.

Task 2.6 turns this rule into a test rather than a convention.

---

## Phase ordering and dependencies

```
Phase 1 (middle end)                 Phase 2 (trait)              Phase 3 (Cranelift)
────────────────────                 ───────────────              ───────────────────
1.1 lock obligation surface ─┐
1.2 decide postcondition     │
1.3 parse it                 ├──► gate A ──► 2.1 cabi.rs ──┐
1.4 generate the hypothesis  │               2.2 ffi.type  │
1.5 close the ledger's two  ─┘               2.3 link.rs   ├──► gate B ──► 3.0 deps
                                             2.4 trait     │                3.1 hello object
docs/static-drop-roadmap.md ─► gate A        2.5 Unit      │                3.2 VbVal ABI
  (emission contract frozen)                 2.6 layering ─┘                3.3 type mapping
                                                                            3.4 bodies
                                                                            3.5 closures/variadics
                                                                            3.6 drop functions
                                                                            3.7 allocator
                                                                            3.8 ext c
                                                                            3.9 exp c + archive
                                                                            3.10 equivalence gate
```

**Gate A — Phase 1 is done.** All of:

- `vibe proof examples/ledger.vibe --prove` prints nothing;
- `cargo test` is green;
- `docs/static-drop-roadmap.md` has a frozen emission contract, in the sense defined above.

Why this gate exists: every change to what must be emitted has to be made twice once there are two backends. Implicit drop changes what must be emitted. Doing it before the split costs one implementation; doing it after costs two and an equivalence argument.

**Gate B — Phase 2 is done.** All of:

- `cargo test` is green and the e2e outputs are byte-identical to before Phase 2;
- `src/codegen.rs` contains no reference to `crate::own`, `crate::escape`, `crate::infer::check`, `crate::refine` or `crate::total` (enforced by `tests/layering.rs`);
- `vibe build --backend=c` and plain `vibe build` produce identical binaries.

Phase 3 tasks 3.0–3.4 are strictly ordered; 3.5–3.9 may be taken in any order once 3.4 lands; 3.10 is last.

**Every step of Phase 1 must remain testable on the existing C backend.** There is no second backend to test against yet, and the reference program's output (`n=3 tot=20.75 avg=6.91667 top=b`) is the invariant that survives all three phases.

---

## File structure

The repository keeps `src/` flat — one file per concern, no module directories. Follow that.

| file | status | responsibility |
|---|---|---|
| `src/cabi.rs` | **new** (Task 2.1) | the §10.3 C-ABI type mapping and the `exp c` header. Backend-neutral: both backends need it, and it depends only on `ast` + `types` + `Checked`. Absorbs `c_type`, `c_unbox`, `box_expr`, `unbox_return`, `split_fn`, `flatten_fn`, `header` from `codegen.rs`. |
| `src/link.rs` | **new** (Task 2.3) | subprocess invocation: `cc`, `ar`, `pkg-config`, and the `-l`/`--cflags` computation from `ext c` blocks. Absorbs `cc` and `archive` from `main.rs`. |
| `src/backend.rs` | **new** (Task 2.4) | the `CodegenBackend` trait, the `Unit` it consumes, `BuildError`, and `select()`. |
| `src/codegen.rs` | modified | C emission only. Gains `pub struct CBackend`. Loses the C-ABI helpers, the `ffi.type` check, and the calls into `own`/`escape`. |
| `src/clif.rs` | **new** (Task 3.1) | the Cranelift backend. One file until it exceeds roughly a thousand lines, at which point split by concern (`clif_abi.rs`, `clif_expr.rs`), never by "layer". |
| `src/main.rs` | modified | argument parsing and pipeline order. Loses the toolchain invocation; gains `--backend=`. |
| `tests/layering.rs` | **new** (Task 2.6) | the architectural rules as assertions. |
| `tests/backends.rs` | **new** (Task 3.10) | the differential gate: the e2e suite run under both backends. |

---

# Phase 1 — Finish the middle end

Nothing touches the backends until the semantic layer is stable.

### Task 1.1: Lock the obligation surface with a test

The refactors in Phases 2 and 3 must not be able to change which obligations exist without a test noticing. `src/refine.rs` already has unit tests; this adds an end-to-end assertion keyed to the reference program.

**Files:**
- Test: `tests/refine.rs` (append)

**Interfaces:**
- Consumes: the `vibe proof` output format — three tab-separated columns, `path\tcode\tmsg`, one line per open obligation (`main.rs::proof`). `tests/refine.rs` already defines `fn vibe(args: &[&str]) -> (bool, String)` (stdout then stderr, run from the manifest directory) and `fn have_z3() -> bool`; reuse both, do not add a second copy.
- Produces: nothing other code calls.

- [ ] **Step 1: Write the failing test**

```rust
// tests/refine.rs — appended; `vibe` and `have_z3` already exist at the top of this file.

/// The obligations the reference program raises, exactly. A refactor that
/// drops one is indistinguishable from a refactor that proves one, so the
/// list is pinned rather than counted.
#[test]
fn ledger_obligations_are_exactly_these() {
    let (ok, out) = vibe(&["proof", "examples/ledger.vibe"]);
    assert!(ok, "vibe proof failed:\n{out}");
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec![
            "Ledger.mk.body/0\trefine.unproven\tcannot prove the invariant of `Tx`: qty > 0",
            "Ledger.mk.body/1\trefine.unproven\tcannot prove the invariant of `Tx`: price > 0",
            "Ledger.mean.body/0\tdiv0\tcannot prove the divisor is non-zero",
            "Ledger.main.body/0\trefine.unproven\tcannot prove the precondition of `mean`: len ts > 0",
            "Ledger.main.body/1\trefine.unproven\tcannot prove the precondition of `top`: len ts > 0",
        ]
    );
}
```

- [ ] **Step 2: Run it**

Run: `cargo test --test refine ledger_obligations_are_exactly_these`
Expected: PASS today. If it fails, the survey above is stale — re-run `vibe proof examples/ledger.vibe`, update the expected list in this test *and* the table in this document, and note the commit you were on.

- [ ] **Step 3: Commit**

```bash
git add tests/refine.rs
git commit -m "test(refine): pin the reference program's obligation list"
```

---

### Task 1.2: Decide the postcondition syntax, and amend the spec

**This is a language design decision, not an implementation task.** It must go through the spec (`vibelang-spec.md` §7.1) before any parser changes, because §7.1 is normative and currently has no such construct. Use `superpowers:brainstorming` with the repository owner rather than choosing alone.

**Files:**
- Modify: `vibelang-spec.md` §7.1 and §7.2

**Interfaces:**
- Consumes: the gap documented above — `Ledger.main.body/0` and `/1`.
- Produces: the surface form Task 1.3 parses and the semantics Task 1.4 encodes.

- [ ] **Step 1: Write down the requirement before the syntax**

The construct must be able to express, for `load (p:&Str) : E! Res Err (Vec Tx)`, the fact *"when the result is `Ok ts`, `len ts > 0`"*. Therefore it must (a) name the result, (b) discriminate on the result's constructor, (c) be checkable at the callee's return sites and assumable at the caller's call sites. A postcondition that cannot do (b) does not close the reference program, which is the only test that matters here.

- [ ] **Step 2: Record the two routes, and choose between them explicitly**

*Route A — a postcondition in the signature.* Consistent with §7.1's existing rule ("refinements live in the signature, after the type, separated by commas; there is no separate `where` block"), so the return position is the only place the rule has not yet been applied. Costs: a grammar change, a canonicity rule (§3.1) for it, a `view` projection update, and a new obligation kind at every `return` site of the annotated function. Buys: a stated, checkable contract at the boundary, the same thing `exp c` headers already advertise as a comment, and a story that works across `ext c` (where the postcondition is *assumed*, exactly as §10.1 already does for preconditions).

*Route B — inline the callee's body into the caller's hypotheses.* No language change at all. §9 already flattens every module into one unit, so `refine.rs` has every callee body in hand; it could emit the callee's body as an SMT definition and let Z3 derive `len ts > 0` itself. Costs: obligation size grows with call depth and the solver budget is already 5 seconds; recursion needs a depth cut-off, which is a soundness-neutral but completeness-destroying knob; it stops dead at `ext c`, where there is no body; and the resulting failure mode — "the solver gave up" — is exactly the diagnostic the project says it will not hide behind.

Route A is the one consistent with the rest of the language and the only one that survives the C boundary. Route B is cheaper and may be worth it as an interim. **Choose one, in writing, in the spec.** If Route A is chosen, the remaining tasks change only in which token the parser accepts.

- [ ] **Step 3: Amend §7.1 and §7.2**

Add the chosen form to §7.1 with at least the `load` example spelled out, and add a row to the §7.2 table for the new obligation: *"return site of a function with a postcondition → the postcondition, in the callee"*. State, next to it, that a postcondition on an `ext c` declaration is assumed and not proved, for the same reason §10.1 gives for preconditions.

- [ ] **Step 4: Commit**

```bash
git add vibelang-spec.md
git commit -m "spec(7.1): postconditions in the signature"
```

---

### Task 1.3: Parse the postcondition

**Files:**
- Modify: `src/parser.rs` (the function-declaration path — today refinements are attached only to `Param` and to `RecordDef`; `ExtSig` carries a `refines: Vec<Expr>` already)
- Modify: `src/ast.rs` (`FunDecl`)
- Modify: `src/view.rs` (the canonical projection must round-trip the new syntax byte-identically)
- Test: `tests/refine.rs`

**Interfaces:**
- Consumes: the syntax fixed by Task 1.2.
- Produces: `FunDecl::post: Vec<Expr>` — the postcondition expressions, in a scope where the result is nameable. Tasks 1.4 and 1.5 read this field.

- [ ] **Step 1: Write the failing test**

```rust
// tests/refine.rs
#[test]
fn a_postcondition_parses_and_round_trips() {
    // tests/post.vibe declares one function with a postcondition.
    let (ok, out) = vibe(&["check", "tests/post.vibe"]);
    assert!(ok, "post.vibe must check:\n{out}");
    // §3.1: the canonical projection is byte-identical to the source.
    let (ok, rendered) = vibe(&["view", "tests/post.vibe"]);
    assert!(ok, "view failed:\n{rendered}");
    let src = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/post.vibe")
    ).expect("post.vibe exists");
    assert_eq!(rendered, src, "the canonical projection must be byte-identical");
}
```

Write `tests/post.vibe` in the syntax Task 1.2 chose, containing one function whose postcondition is constructor-conditioned, plus a `main` that calls it.

- [ ] **Step 2: Run it**

Run: `cargo test --test refine a_postcondition_parses_and_round_trips`
Expected: FAIL at parse, with `parse.trailing` or similar.

- [ ] **Step 3: Add the field**

```rust
// src/ast.rs, in FunDecl
/// Postconditions (spec §7.1): proved at the function's return sites,
/// assumed at its call sites. Empty for every function that declares none.
pub post: Vec<Expr>,
```

Every `FunDecl` construction site in `src/parser.rs` gains `post: Vec::new()`; the compiler will list them.

- [ ] **Step 4: Parse and project**

Accept the new form in the signature parser and emit it again in `view::canon`'s signature rendering, in the same position, with the same spacing. The projection defines canonicity in this repository, so a postcondition the projection does not print is a postcondition that cannot be written.

- [ ] **Step 5: Run the whole suite**

Run: `cargo test`
Expected: PASS, including the byte-identical `view` check over every `.vibe` in the repository.

- [ ] **Step 6: Commit**

```bash
git add src/ast.rs src/parser.rs src/view.rs tests/post.vibe tests/refine.rs
git commit -m "feat(parse): postconditions in the signature (spec §7.1)"
```

---

### Task 1.4: Prove postconditions at return sites, assume them at call sites

**Files:**
- Modify: `src/refine.rs` (`obligations`, and `Gen::call`)
- Test: `src/refine.rs` unit tests (the `smt` function is pure; test the generated text, not the solver)

**Interfaces:**
- Consumes: `FunDecl::post` from Task 1.3; the existing `Gen` walk, its `hyps: Vec<String>` stack, and `Gen::push(span, code, msg, pretty, goal)`. `src/refine.rs`'s `#[cfg(test)] mod tests` already defines `fn obs_of(src: &str) -> Vec<Ob>` (lex, parse, check, generate); reuse it.
- Produces: a new obligation code `post.unproven` at return sites; new hypotheses at call sites. No new public function.

- [ ] **Step 1: Write the failing unit tests**

Both tests are written against the module below. Only the marked line depends on Task 1.2's choice of syntax; everything else is the language as it stands.

```rust
// src/refine.rs, in the existing #[cfg(test)] module

/// A callee whose result is non-empty by construction, and a caller that
/// relies on it. Mirrors `load` / `mean` in the reference program.
const POST_SRC: &str = "\
mod T

half (n:U64, n>0) : U64 = 100 / n

pick (n:U64) : U64 = ...   /* <- the postcondition goes here (Task 1.2): result > 0 */

use_it (n:U64) : U64 = half (pick n)
";

#[test]
fn a_postcondition_becomes_an_obligation_in_the_callee() {
    let obs = obs_of(POST_SRC);
    assert!(
        obs.iter().any(|o| o.code == "post.unproven" && o.path.starts_with("T.pick")),
        "the callee must be asked to prove its own postcondition: {:?}",
        obs.iter().map(|o| (&o.path, &o.code)).collect::<Vec<_>>()
    );
}

#[test]
fn a_postcondition_is_a_hypothesis_at_the_call_site() {
    let obs = obs_of(POST_SRC);
    let pre = obs
        .iter()
        .find(|o| o.msg.contains("precondition of `half`"))
        .expect("the precondition obligation still exists");
    assert!(
        pre.hyps.iter().any(|h| h.contains("> 0") || h.contains("(> ")),
        "the callee's postcondition must be in scope as a hypothesis: {:?}",
        pre.hyps
    );
}
```

Replace the `...` with the real body and postcondition once Task 1.2 has fixed the syntax; the body only has to make the postcondition true (`n + 1` does, for a `U64` that the range hypothesis already bounds).

- [ ] **Step 2: Run them**

Run: `cargo test --lib refine`
Expected: FAIL — `post.unproven` is not a code that exists.

- [ ] **Step 3: Generate the callee-side obligation**

In `obligations`, after walking `f.body`, raise one obligation per entry in `f.post`, translated in the callee's scope with the result bound to the body's SMT term. The path follows the existing convention (`<Home>.<fn>.post/<i>`), the code is `post.unproven`, and the message names the postcondition as written, via the existing `show` helper. A constructor-conditioned postcondition becomes an implication whose antecedent is the tag equality the arm already teaches — reuse the machinery that gives a match arm its constructor facts rather than writing a second one.

- [ ] **Step 4: Assume it at the call site**

In `Gen::call`, after the precondition obligations are pushed, push the callee's postconditions onto `self.hyps`, renamed into the caller's terms with the same `sub`/`len_sub` substitution the precondition path already builds. They must stay on the stack for the remainder of the enclosing scope, not just for one obligation — this is the one place where the existing push/pop discipline (`self.hyps.push(...)` then `self.hyps.pop()` around a single `push`) does not apply.

For an `ext c` declaration, assume the postcondition without ever proving it, exactly as §10.1 does for preconditions, and say so in the code comment.

- [ ] **Step 5: Run the tests**

Run: `cargo test`
Expected: PASS, except `ledger_obligations_are_exactly_these` from Task 1.1, which now fails because the two `main` obligations may still be open but new `post.unproven` entries have appeared. Update that list in the same commit and say why in the message.

- [ ] **Step 6: Commit**

```bash
git add src/refine.rs tests/refine.rs
git commit -m "feat(refine): prove postconditions in the callee, assume them at the call site"
```

---

### Task 1.5: Close the reference program

**Files:**
- Modify: `examples/ledger.vibe` (add the postcondition to `load`)
- Modify: `README.md` (the Status lists), `vibelang-spec.md` Appendix A if the reference program text is duplicated there
- Test: `tests/refine.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: the Gate A condition.

- [ ] **Step 1: Write the failing test**

```rust
// tests/refine.rs
#[test]
fn the_reference_program_proves() {
    if !have_z3() {
        eprintln!("skipped: z3 is not on PATH");
        return;
    }
    let (ok, out) = vibe(&["proof", "examples/ledger.vibe", "--prove"]);
    assert!(ok, "vibe proof failed:\n{out}");
    assert_eq!(out, "", "no obligation may remain open:\n{out}");
}
```

- [ ] **Step 2: Run it**

Run: `cargo test --test refine the_reference_program_proves`
Expected: FAIL, listing the two `Ledger.main.body` obligations. The `have_z3` guard is the pattern the other discharge tests in this file already use: skip loudly, never pass silently, because a test that quietly succeeds when the solver is absent is worse than no test.

- [ ] **Step 3: Annotate `load`**

Add the postcondition to `load`'s signature in `examples/ledger.vibe`: when the result is `Ok ts`, `len ts > 0`. This is a claim the body already establishes — the `Ok [] -> Er Void` arm is exactly what makes it true — so it should discharge with no other change.

- [ ] **Step 4: Run it**

Run: `cargo test`
Expected: PASS, including `reference_program_runs` in `tests/e2e.rs` still printing `n=3 tot=20.75 avg=6.91667 top=b`, and the byte-identical `view` check over `examples/ledger.vibe`.

- [ ] **Step 5: Move the README lines**

In `README.md`, `refinements` moves out of **Partial**; the reference program section's paragraph beginning *"Be clear about why it compiles today"* is now false and must be rewritten to say what is actually true after this task. Moving a line from one list to the next is the documented way to edit that section.

- [ ] **Step 6: Commit**

```bash
git add examples/ledger.vibe README.md tests/refine.rs
git commit -m "feat(refine): the reference program proves with no open obligations"
```

---

# Phase 2 — Isolate codegen behind a trait

No behaviour changes in this phase. Every commit must leave `cargo test` green and the reference program's output byte-identical. If a commit here changes a single byte of generated C, it is wrong.

### Task 2.1: Extract the C-ABI mapping into `src/cabi.rs`

**Files:**
- Create: `src/cabi.rs`
- Modify: `src/codegen.rs` (delete the moved functions, import them)
- Modify: `src/main.rs` (`mod cabi;`, and `codegen::header` becomes `cabi::header`)
- Test: `tests/e2e.rs` (append a golden-header test)

**Interfaces:**
- Consumes: `ast::Module`, `infer::Checked`, `types::T`.
- Produces:
  - `pub fn c_type(t: &T) -> String`
  - `pub fn c_unbox(t: &T, v: &str) -> Option<String>`
  - `pub fn box_expr(t: &T, v: &str) -> String`
  - `pub fn unbox_return(t: &T) -> String`
  - `pub fn split_fn(t: &T, n: usize) -> (Vec<T>, T)`
  - `pub fn flatten_fn(t: &Ty) -> (Vec<Ty>, Ty)`
  - `pub fn header(m: &Module, ck: &Checked) -> String`

  Task 3.8 and 3.9 call `split_fn`, `flatten_fn` and `header`; `c_unbox` returning `None` is the signal that a type cannot cross the boundary, which Task 2.2 turns into the diagnostic.

- [ ] **Step 1: Write the failing test**

```rust
// tests/e2e.rs
#[test]
fn the_exported_header_is_stable() {
    let (ok, out) = vibe(&["build", "examples/ledger.vibe"]);
    assert!(ok, "build failed:\n{out}");
    let h = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/.vibe-ledger/ledger.h")
    ).expect("the generated header is written beside the build");
    assert!(h.contains("/* mean — pre: len ts > 0   NOT VERIFIED ACROSS THE BOUNDARY */"));
    assert!(h.contains("double Ledger_mean("));
    assert!(h.contains("double Ledger_total("));
}
```

- [ ] **Step 2: Run it**

Run: `cargo test --test e2e the_exported_header_is_stable`
Expected: PASS today — this is a characterisation test written *before* the move so that the move is provably behaviour-free. If the path or the wording differs, fix the test to match reality first, in its own commit.

- [ ] **Step 3: Move the functions**

`git mv` is not available for a partial file move; cut the listed functions from `src/codegen.rs` into a new `src/cabi.rs` unchanged — same bodies, same names, same doc comments — add `pub` where they were private, and add `use crate::cabi::{...};` at the top of `codegen.rs`.

- [ ] **Step 4: Run the suite**

Run: `cargo test`
Expected: PASS, unchanged.

- [ ] **Step 5: Commit**

```bash
git add src/cabi.rs src/codegen.rs src/main.rs tests/e2e.rs
git commit -m "refactor(cabi): the C-ABI mapping is backend-neutral, not C-backend-private"
```

---

### Task 2.2: Move the `ffi.type` check out of the backend

A value that cannot cross the C boundary is a language error, not a C-emission error. Today it is reported by `vibe build` and not by `vibe check`, and a second backend would have to reimplement it.

**Files:**
- Modify: `src/cabi.rs` (add `check`)
- Modify: `src/codegen.rs` (`ext_call` stops pushing the diagnostic; it may `expect` instead, since the check has already run)
- Modify: `src/main.rs` (call `cabi::check` in the `semantic` block, beside `own::check` and `total::check`)
- Test: `tests/e2e.rs`

**Interfaces:**
- Consumes: `c_unbox` from Task 2.1.
- Produces: `pub fn check(m: &Module, ck: &Checked) -> Vec<Diag>` — the same `ffi.type` diagnostic, same code, same `fix` text (`"use a scalar, Bool, Char, Str, CStr or `Ptr a` at the C boundary"`), now raised at `vibe check` time.

- [ ] **Step 1: Write the failing test**

```rust
// tests/e2e.rs
#[test]
fn a_bad_ffi_type_is_a_check_error_not_a_build_error() {
    // tests/ffi_bad.vibe passes a Vec to an `ext c` function.
    let (ok, out) = vibe(&["check", "tests/ffi_bad.vibe", "--diag=struct"]);
    assert!(!ok, "check must reject it");
    assert!(out.contains("ffi.type"), "expected ffi.type, got:\n{out}");
}
```

Write `tests/ffi_bad.vibe`: one `ext c` block declaring a function taking `&CStr`, and a call passing something `c_unbox` refuses.

- [ ] **Step 2: Run it**

Run: `cargo test --test e2e a_bad_ffi_type_is_a_check_error_not_a_build_error`
Expected: FAIL — `vibe check` succeeds today, because the check lives in codegen.

- [ ] **Step 3: Implement `cabi::check`**

Walk the module's expressions; at every application of a name in `ck.ext`, zip the arguments against `flatten_fn(&sig.ty).0` and push the `ffi.type` diagnostic for every parameter whose `c_unbox` is `None`. Also check the return type, which `ext_call` checks today via `c_box`'s equivalent path.

- [ ] **Step 4: Run the suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/cabi.rs src/codegen.rs src/main.rs tests/ffi_bad.vibe tests/e2e.rs
git commit -m "fix(cabi): `ffi.type` is a semantic error, reported by `vibe check`"
```

---

### Task 2.3: Extract the toolchain invocation into `src/link.rs`

**Files:**
- Create: `src/link.rs`
- Modify: `src/main.rs` (delete `cc` and `archive`, call into `link`)
- Test: covered by the existing `tests/e2e.rs`

**Interfaces:**
- Consumes: `ast::Module` (for `ext c` `link`/`pkg` entries).
- Produces:
  - `pub fn ext_flags(m: &Module) -> Result<Vec<String>, String>` — the `pkg-config` output plus `-l<lib>` for every `link`, in that order, ending with `-lm`. Extracted verbatim from today's `cc`, whitespace-splitting caveat and all; carry the existing `ponytail:` comment with it.
  - `pub fn cc_exe(objs: &[&Path], out: &Path, include: &Path, flags: &[String]) -> Result<(), String>`
  - `pub fn cc_obj(src: &Path, out: &Path, include: &Path) -> Result<(), String>`
  - `pub fn ar(objs: &[PathBuf], out: &Path) -> Result<(), String>`

  Task 3.1 calls `cc_exe` to link Cranelift's object against the C runtime; Task 3.9 calls `ar`.

- [ ] **Step 1: Move the code**

Cut `cc` and `archive` out of `src/main.rs` into `src/link.rs`, splitting `archive` so that the *policy* (refuse a `--lib` build of a module with no `exp c`; copy the headers next to the archive) stays in `main.rs` and only the *subprocess* part moves.

- [ ] **Step 2: Run the suite**

Run: `cargo test`
Expected: PASS, unchanged. In particular `exported_functions_get_a_header` and `c_ffi_calls_libc`.

- [ ] **Step 3: Verify by hand that a linked binary still runs**

Run: `cargo run -- run examples/ledger.vibe`
Expected: `n=3 tot=20.75 avg=6.91667 top=b`

- [ ] **Step 4: Commit**

```bash
git add src/link.rs src/main.rs
git commit -m "refactor(link): toolchain invocation out of the driver"
```

---

### Task 2.4: Define the trait

**Files:**
- Create: `src/backend.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `ast::Module`, `infer::Checked`, `own::inplace_updates`, `escape::releasable`, `diag::Diag`.
- Produces: the trait and its `Unit`, both named below. Everything in Phase 3 implements against this exact signature.

- [ ] **Step 1: Write the trait**

```rust
//! The backend boundary (spec §11).
//!
//! A backend is handed a module that has already been parsed, typed, checked
//! for canonicity, ownership, termination and refinements. It decides how to
//! turn that into machine code and nothing else: no inference, no ownership
//! analysis, no diagnostics that another backend would not also emit.

use crate::ast::Module;
use crate::diag::Diag;
use crate::infer::Checked;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Everything a backend is allowed to see. Built once, in the driver, after
/// every semantic pass has run and before any backend is chosen.
pub struct Unit<'a> {
    pub module: &'a Module,
    pub checked: &'a Checked,
    /// `{r with ...}` sites whose base is uniquely owned (spec §4.3), from
    /// `own::inplace_updates`. Precomputed: a backend may not ask again.
    pub inplace: HashSet<(usize, usize, usize)>,
    /// Functions whose body may be bracketed by mark/release (spec §4.6),
    /// from `escape::releasable`.
    pub releasable: HashSet<String>,
    /// File stem: the C file's name, the object's name, the header's prefix.
    pub stem: String,
    /// The source path, for `#line` and for any future debug info.
    pub source: PathBuf,
    /// Scratch directory the backend may write into. Created by the driver.
    pub work: PathBuf,
}

/// What a backend can fail with. `Diags` is the language talking; `Driver` is
/// the environment; `Unsupported` is a backend declining a request that
/// another backend can satisfy — never an error in the user's program.
pub enum BuildError {
    Diags(Vec<Diag>),
    Driver(String),
    Unsupported(&'static str),
}

/// A `--lib` build produces both, and the header is the point of the exercise.
pub struct Lib {
    pub archive: PathBuf,
    pub header: PathBuf,
}

pub trait CodegenBackend {
    /// The name `--backend=` selects, and the name diagnostics use.
    fn name(&self) -> &'static str;

    /// Produce a native executable at `out`.
    fn build_exe(&mut self, u: &Unit, out: &Path) -> Result<(), BuildError>;

    /// Produce a static library plus its C header (spec §10.2).
    fn build_lib(&mut self, u: &Unit, out: &Path) -> Result<Lib, BuildError>;

    /// Produce the backend's source form, when it has one. The C backend
    /// returns the generated C, which is what `--emit-c` writes. A backend
    /// with no source form returns `Unsupported`.
    fn emit_source(&mut self, u: &Unit) -> Result<String, BuildError>;
}

pub fn select(name: &str) -> Result<Box<dyn CodegenBackend>, String> {
    match name {
        "c" => Ok(Box::new(crate::codegen::CBackend::new())),
        other => Err(format!("unknown backend `{other}`; known: c")),
    }
}
```

Three notes on this signature, each deliberate:

- **`Unit` carries precomputed tables rather than a `&Checked` alone.** That is the mechanical form of the "one IR" rule: if the tables are arguments, a backend physically cannot recompute them differently.
- **`emit_source` is on the trait rather than on `CBackend` only.** `--emit-c` is a user-facing flag that must keep working regardless of which backend is selected; the driver satisfies it by asking for the C backend's source explicitly. Keeping the method on the trait means a future backend with a readable intermediate form (Cranelift IR text, say) can offer it without a second flag.
- **`BuildError::Unsupported` is not a program error.** A `--lib` request to a backend that cannot archive yet must be reported as a limitation of the backend, with the C backend named as the fallback — never as though the user's code were wrong.

- [ ] **Step 2: Implement `CBackend`**

In `src/codegen.rs`, add a struct whose three methods are today's `main.rs` build path verbatim: write the runtime and header into `u.work`, call `generate`, write the C, then `link::cc_exe` or `link::ar`.

```rust
pub struct CBackend;

impl CBackend {
    pub fn new() -> CBackend { CBackend }
}

impl crate::backend::CodegenBackend for CBackend {
    fn name(&self) -> &'static str { "c" }
    // build_exe / build_lib / emit_source as described
}
```

- [ ] **Step 3: Switch the driver**

`main.rs` parses `--backend=<name>` (default `c`), builds the `Unit`, and calls `select(&name)?`. The `--emit-c` path asks the C backend for `emit_source` regardless of the selected backend.

- [ ] **Step 4: Run the suite**

Run: `cargo test`
Expected: PASS, unchanged.

- [ ] **Step 5: Verify the two paths agree**

```bash
cargo run -- build examples/ledger.vibe -o /tmp/a
cargo run -- build examples/ledger.vibe -o /tmp/b --backend=c
cmp /tmp/a /tmp/b && echo identical
```

- [ ] **Step 6: Commit**

```bash
git add src/backend.rs src/codegen.rs src/main.rs
git commit -m "feat(backend): codegen behind a trait, C as the first implementation"
```

---

### Task 2.5: Stop `codegen` reaching into the middle end

**Files:**
- Modify: `src/codegen.rs` (`generate` takes the tables instead of computing them)
- Modify: `src/backend.rs` / `src/main.rs` (compute them once)

**Interfaces:**
- Consumes: `Unit::inplace`, `Unit::releasable`.
- Produces: `pub fn generate(u: &Unit) -> Result<String, Vec<Diag>>` — the old `(m, ck, file)` triple plus the two tables, all of which `Unit` already holds.

- [ ] **Step 1: Change the signature**

Delete these two lines from `generate`:

```rust
inplace: crate::own::inplace_updates(m, ck),
releasable: crate::escape::releasable(m, ck),
```

and take them from the `Unit` instead. Delete the now-unused `use` lines.

- [ ] **Step 2: Run the suite**

Run: `cargo test`
Expected: PASS, unchanged.

- [ ] **Step 3: Commit**

```bash
git add src/codegen.rs src/backend.rs src/main.rs
git commit -m "refactor(codegen): the backend is handed its tables, it does not ask for them"
```

---

### Task 2.6: Make the architectural rules a test

A rule nobody can violate accidentally is worth more than a rule in a document. This test is crude on purpose: it reads the backend source files as text and asserts what they may not mention. It costs nothing and it fails loudly on exactly the mistake this plan exists to prevent.

**Files:**
- Create: `tests/layering.rs`

**Interfaces:**
- Consumes: nothing. Reads `src/*.rs` as text.
- Produces: the Gate B condition.

- [ ] **Step 1: Write the test**

```rust
//! The rules from docs/backend-roadmap.md, as assertions.
//!
//! A backend may read the typed IR. It may not run a semantic pass, and it may
//! not decide anything the middle end is supposed to have decided.

const BACKENDS: &[&str] = &["src/codegen.rs", "src/clif.rs"];

const FORBIDDEN: &[&str] = &[
    "crate::own",
    "crate::escape",
    "crate::total",
    "crate::refine",
    "infer::check",
];

#[test]
fn no_backend_runs_a_semantic_pass() {
    for f in BACKENDS {
        let path = format!("{}/{f}", env!("CARGO_MANIFEST_DIR"));
        let Ok(src) = std::fs::read_to_string(&path) else { continue }; // not written yet
        for bad in FORBIDDEN {
            assert!(
                !src.contains(bad),
                "{f} mentions `{bad}`: a backend must be handed what the middle end decided, \
                 not recompute it. See docs/backend-roadmap.md, \"One IR for both backends\"."
            );
        }
    }
}
```

- [ ] **Step 2: Run it**

Run: `cargo test --test layering`
Expected: PASS, because Task 2.5 removed the last references. If it fails, Task 2.5 is not finished.

- [ ] **Step 3: Commit**

```bash
git add tests/layering.rs
git commit -m "test(layering): a backend may not run a semantic pass"
```

---

# Phase 3 — Cranelift

Read before starting: the Cranelift API changes shape between minor releases. Everything below was written against the crate as of this document's date and must be re-checked against the version you pin in Task 3.0. Task 3.1 exists specifically so that the first thing you find out is whether the API still looks like this, before anything depends on it.

### Task 3.0a: Decide the dependency posture, and pin the versions

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`
- Modify: `README.md`

**Interfaces:**
- Produces: the dependency set every later task builds on.

- [ ] **Step 1: Face the cost honestly**

`vibec` has **no dependencies at all** today — `Cargo.lock` is 149 bytes, and the README advertises *"the compiler is a single Rust crate with an empty dependency graph"* as a feature. The Cranelift crates bring a transitive tree (regalloc2, cranelift-entity, cranelift-bforest, cranelift-bitset, cranelift-control, target-lexicon, smallvec, hashbrown, bumpalo, log, object, anyhow, gimli and others). That claim in the README becomes false the moment this lands and must be edited in the same commit, not later.

- [ ] **Step 2: Choose whether Cranelift is a default feature**

Behind a non-default cargo feature: a plain `cargo build` stays dependency-free, the claim survives in qualified form, and a distributor who wants the small compiler can have it. Against: CI must build and test both configurations or the feature rots, `#[cfg(feature = ...)]` spreads into `backend::select`, and the *default* build is then the one that needs a C toolchain — which is the opposite of the project's stated goal.

Recommendation: start behind `feature = "cranelift"`, non-default, so Phase 3 can land incrementally without changing what a default build means; flip it to default in Task 3.10, once the equivalence gate passes. This is reversible in one commit either way, which is why it is a recommendation and not a gate.

- [ ] **Step 3: Add the dependencies**

Pin all Cranelift crates to the same exact version — they release in lockstep and mixing them does not work.

```toml
[features]
default = []
cranelift = ["dep:cranelift-codegen", "dep:cranelift-frontend",
             "dep:cranelift-module", "dep:cranelift-object", "dep:cranelift-native"]

[dependencies]
cranelift-codegen  = { version = "=X.Y.Z", optional = true }
cranelift-frontend = { version = "=X.Y.Z", optional = true }
cranelift-module   = { version = "=X.Y.Z", optional = true }
cranelift-object   = { version = "=X.Y.Z", optional = true }
cranelift-native   = { version = "=X.Y.Z", optional = true }
```

Look up the current version on crates.io rather than copying one from here; write the one you chose into this document's Tech Stack line so the next reader knows what the code below was written against. Note that the task brief named four crates; `cranelift-frontend` (which provides `FunctionBuilder`, without which you are emitting CLIF by hand) and `cranelift-native` (host ISA detection) are needed too. The `cranelift` umbrella crate re-exports codegen and frontend; depending on the components directly is clearer about what is actually used.

- [ ] **Step 4: Update CI**

`.github/workflows` runs `cargo build --locked` and `cargo test --locked`. Add a second job, or two extra steps, with `--features cranelift`. A feature that CI does not build is a feature that is already broken.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock .github/workflows README.md
git commit -m "build: cranelift behind a feature; the dependency graph is no longer empty"
```

---

### Task 3.0b: Settle the runtime question by measurement, not by argument

This is the gate on everything after Task 3.4, and it must not be decided from the armchair. See [The open question](#the-open-question-the-runtime-without-a-c-toolchain) for the options and what each costs.

**Files:**
- Create: nothing in `src/`. Prototypes live in a scratch worktree and are thrown away.
- Modify: this document — record the decision and the numbers in the section below.

- [ ] **Step 1: Establish the baseline**

```bash
cargo run -- build examples/ledger.vibe -o /tmp/ledger
ls -l /tmp/ledger
otool -L /tmp/ledger   # or: ldd /tmp/ledger
```

Record the binary size and the shared-library dependencies. This is what any option has to be compared against.

- [ ] **Step 2: Establish what the runtime actually needs**

```bash
grep -oE '\b(malloc|free|memcpy|memset|strlen|snprintf|fopen|fclose|fread|fwrite|fputs|exit|strtoll|strtod|sqrt|pow|floor|fabs)\b' runtime/vibert.c | sort | uniq -c
```

Eighteen symbols at the time of writing. Any "rewrite the runtime" option has to supply all of them; any "ship a prebuilt runtime" option has to supply them per target triple.

- [ ] **Step 3: Prototype the linking question in isolation**

Before any runtime decision: does a Cranelift-produced object link into an executable *without* invoking `cc`? Try, on each host you care about, `ld`/`ld64` directly, and `rust-lld` (shipped inside the Rust toolchain, path from `rustc --print sysroot`). Record for each: whether it worked, what flags were needed, whether the C runtime startup files (`crt1.o` and friends) had to be located by hand, and whether locating them needed a C toolchain anyway. **That last answer may make the whole runtime question moot in one direction or the other**, which is why it comes first.

- [ ] **Step 4: Write the decision into this document**

Replace the options list in [The open question](#the-open-question-the-runtime-without-a-c-toolchain) with the decision, the numbers behind it, and the date. Leave the rejected options in place with the reason each was rejected — the next person to reopen the question deserves to know what was already tried.

- [ ] **Step 5: Commit**

```bash
git add docs/backend-roadmap.md
git commit -m "docs(backend): the runtime question, decided"
```

---

### Task 3.1: A Cranelift object that links and runs

The smallest possible end-to-end path: emit a `main` that calls nothing but `vb_init` and returns 0, link it against the C runtime with `cc`, run it. This deliberately does *not* solve the runtime question — it isolates the Cranelift wiring so that everything after it has a working baseline.

**Files:**
- Create: `src/clif.rs`
- Modify: `src/backend.rs` (`select` learns `"cranelift"`), `src/main.rs` (`mod clif;` behind the feature)
- Test: `tests/backends.rs`

**Interfaces:**
- Consumes: `backend::Unit`, `backend::CodegenBackend`, `link::cc_exe`.
- Produces: `pub struct CraneliftBackend` implementing `CodegenBackend` with `name() == "cranelift"`; `build_lib` and `emit_source` return `BuildError::Unsupported` for now.

- [ ] **Step 1: Write the failing test**

```rust
// tests/backends.rs
use std::process::Command;

const VIBE: &str = env!("CARGO_BIN_EXE_vibe");

fn vibe(args: &[&str]) -> (bool, String) {
    let o = Command::new(VIBE)
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.success(), s)
}

#[test]
#[cfg(feature = "cranelift")]
fn cranelift_builds_and_runs_hello() {
    let (ok, out) = vibe(&["run", "examples/hello.vibe", "--backend=cranelift"]);
    assert!(ok, "cranelift build failed:\n{out}");
}
```

- [ ] **Step 2: Run it**

Run: `cargo test --features cranelift --test backends`
Expected: FAIL with `unknown backend \`cranelift\``.

- [ ] **Step 3: Write the module skeleton**

```rust
//! The Cranelift backend (spec §11, "Opzione futura"). Emits an object file
//! for the generated code; the runtime and the link step are separate
//! questions, see docs/backend-roadmap.md.

use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

fn host_module(stem: &str) -> Result<ObjectModule, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    // Spec §11.2: no unwind tables.
    flags.set("unwind_info", "false").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| format!("cranelift does not support this host: {e}"))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let b = ObjectBuilder::new(isa, stem.to_string(), default_libcall_names())
        .map_err(|e| e.to_string())?;
    Ok(ObjectModule::new(b))
}
```

- [ ] **Step 4: Emit `main`**

```rust
fn emit_main(m: &mut ObjectModule) -> Result<(), String> {
    // void vb_init(void) — an empty signature is exactly that
    let init_sig = m.make_signature();
    let init = m
        .declare_function("vb_init", Linkage::Import, &init_sig)
        .map_err(|e| e.to_string())?;

    // int main(int argc, char **argv)
    let ptr = m.target_config().pointer_type();
    let mut sig = m.make_signature();
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ptr));
    sig.returns.push(AbiParam::new(types::I32));
    let id = m.declare_function("main", Linkage::Export, &sig).map_err(|e| e.to_string())?;

    let mut ctx = m.make_context();
    ctx.func.signature = sig;
    let mut fcx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fcx);
        let blk = b.create_block();
        b.append_block_params_for_function_params(blk);
        b.switch_to_block(blk);
        b.seal_block(blk);
        let init_ref = m.declare_func_in_func(init, b.func);
        b.ins().call(init_ref, &[]);
        let zero = b.ins().iconst(types::I32, 0);
        b.ins().return_(&[zero]);
        b.finalize();
    }
    m.define_function(id, &mut ctx).map_err(|e| e.to_string())?;
    m.clear_context(&mut ctx);
    Ok(())
}
```

- [ ] **Step 5: Write the object and link it**

```rust
fn write_object(m: ObjectModule, path: &std::path::Path) -> Result<(), String> {
    let bytes = m.finish().emit().map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}
```

`build_exe` writes `runtime/vibert.c` into `u.work` (the same `RT_C` constant the driver already has — move it to `link.rs` so both backends read it from one place), compiles it with `link::cc_obj`, and calls `link::cc_exe` with both objects. **This still needs a C compiler.** That is the point of this task: it proves the Cranelift half works before the runtime question is answered.

- [ ] **Step 6: Run the test**

Run: `cargo test --features cranelift --test backends`
Expected: PASS. `examples/hello.vibe` will not *do* anything yet — its body is not compiled — so keep the assertion at "builds and runs with exit status 0" until Task 3.4.

- [ ] **Step 7: Commit**

```bash
git add src/clif.rs src/backend.rs src/main.rs tests/backends.rs
git commit -m "feat(clif): a Cranelift object that links against the C runtime and runs"
```

---

### Task 3.2: The `VbVal` calling convention

Every runtime entry point takes and returns `VbVal` **by value**: a 16-byte struct, one `uint8_t` tag and an 8-byte union, alignment 8. Cranelift has no aggregate types; a struct passed by value has to be lowered to registers by hand, per target ABI. This is the first genuinely hard piece of Phase 3 and everything from Task 3.3 on depends on it.

**Files:**
- Modify: `src/clif.rs`
- Test: `tests/backends.rs`

**Interfaces:**
- Produces:
  - `const VBVAL_PARTS: usize = 2;`
  - `fn vbval_params(sig: &mut Signature)` / `fn vbval_returns(sig: &mut Signature)` — append the lowered representation of one `VbVal`
  - `fn vbval_of(b: &mut FunctionBuilder, tag: u8, payload: Value) -> [Value; 2]`
  - `fn vbval_tag(b: &mut FunctionBuilder, v: [Value; 2]) -> Value` and `fn vbval_payload(...) -> Value`

  Tasks 3.4–3.9 build every call out of these.

- [ ] **Step 1: Write the failing test first, as a program**

The only honest test of an ABI assumption is a program that crosses the boundary and comes back with the right number. Add to `tests/backends.rs` a test that builds a Vibelang module through the Cranelift backend whose `main` calls `vb_int(42)` and then `vb_as_int`, and prints the result through the runtime's own `vb_out`. Assert the output is `42`. A wrong ABI gives garbage or a crash, not a wrong-but-plausible number.

- [ ] **Step 2: Choose the lowering, and write down why**

Three options, in descending order of preference:

*Two `I64` values.* Declare each `VbVal` parameter as two consecutive `AbiParam::new(types::I64)` and rely on the platform ABI assigning them the same registers the C compiler would. On x86-64 System V the struct is two eightbytes, both classified INTEGER because a union containing any integer member classifies as INTEGER, so it travels in two general-purpose registers and returns in `rax:rdx`. On AArch64 AAPCS it is a 16-byte composite with no floating-point-only members, so it travels in two X registers and returns in `x0:x1`. **Both of those sentences are claims about an ABI document, not observations**; Step 1's test is what turns them into observations, and it must run on both architectures before this is believed.

*Pass by pointer.* Change the runtime so every generated-code-facing entry point takes `VbVal*`. Mechanical but large, it touches `runtime/vibert.h` and `.c` throughout, and it makes the **C backend** slower and uglier for the Cranelift backend's convenience — which the rules above forbid. Only acceptable if it is done as a *second* ABI surface that the C backend does not use, and that duplication has its own cost.

*Cranelift's `StructArgument`.* `ArgumentPurpose::StructArgument` exists but implements a stack-passing convention for the Wasmtime use case rather than the platform's register classification. It does not solve this problem.

Take the first. Record in a comment in `src/clif.rs` that it is an assumption held up by the test in Step 1, and name the test.

- [ ] **Step 3: Note the platform where this breaks**

Windows x64 returns a 16-byte struct through a hidden pointer (`sret`), not in a register pair. That is a divergence to be handled when Windows is supported, and until then it is a reason the C backend remains the fallback. Write that in the comment too.

- [ ] **Step 4: Run the test on every host you have**

Run: `cargo test --features cranelift --test backends`
Expected: PASS on x86-64 and on aarch64. If you only have one, say so in the commit message — an ABI verified on one architecture is verified on one architecture.

- [ ] **Step 5: Commit**

```bash
git add src/clif.rs tests/backends.rs
git commit -m "feat(clif): VbVal lowered to an integer pair, asserted by a round trip"
```

---

### Task 3.3: The primitive type mapping

**Files:**
- Modify: `src/clif.rs`
- Test: `src/clif.rs` unit tests (the mapping is a pure function)

**Interfaces:**
- Produces: `pub fn clif_type(t: &T, ptr: Type) -> Option<Type>` — `None` for a type with no direct machine representation.

- [ ] **Step 1: Write the table**

Cranelift integer types carry no signedness; the *instruction* does. `U32` and `I32` are both `types::I32`, and the difference shows up as `udiv` vs `sdiv`, `uextend` vs `sextend`, and the `IntCC` variant chosen for a comparison. Getting that wrong produces a program that is correct on small values and wrong on large ones, which is the worst kind of wrong.

| Vibelang | Cranelift | notes |
|---|---|---|
| `U8` / `I8` | `types::I8` | signedness is in the instruction |
| `U16` / `I16` | `types::I16` | |
| `U32` / `I32` | `types::I32` | |
| `U64` / `I64` | `types::I64` | |
| `Size` | `types::I64` | `size_t`, 64-bit targets only |
| `Nat` | `types::I64` | mathematical in the solver; a machine word here |
| `F32` | `types::F32` | |
| `F64` | `types::F64` | |
| `Bool` | `types::I8` | 0 or 1; Cranelift has no boolean type |
| `Char` | `types::I32` | the runtime stores it as `uint32_t` |
| `CStr` | `ptr` | `pointer_type()` from the target config |
| `Ptr a` | `ptr` | |
| `Str`, `Vec a`, record, ADT, closure | `ptr` | a pointer into the bump arena; the boxed `VbVal` carries it in its payload |
| `Unit` | — | no value; a signature with a `Unit` return has no return parameter |
| `Res E T`, `Opt a` | — | ADTs; see the row above, they are heap objects |
| `VbVal` | — | not a Cranelift type; two `I64`s, see Task 3.2 |

- [ ] **Step 2: Write it as code and as a unit test**

One `match` on the type constructor name, mirroring `cabi::c_type`'s structure so the two stay comparable. Test every row: a mapping table is exactly the kind of code that is wrong in one row and right in the rest.

- [ ] **Step 3: Run**

Run: `cargo test --features cranelift --lib clif`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/clif.rs
git commit -m "feat(clif): the Vibelang-to-Cranelift type mapping"
```

**Note on what this table is for.** Nothing in the current IR is unboxed, so at the end of Phase 3 almost every value is still a `VbVal` and this table is used mainly at the `ext c` and `exp c` boundaries, where §10.3 already demands real machine types. The table is the foundation for unboxing when unboxing comes; it is not itself unboxing, and Phase 3 does not deliver a faster program. Say that out loud in any status update, because "we added a native backend" invites the opposite assumption.

---

### Task 3.4: Function bodies

The bulk of the work: lower each `FunDecl` body into Cranelift IR, mirroring `codegen.rs`'s structure — expression by expression, one `VbVal` pair per value, with `vb_*` runtime calls where the C backend emits `vb_*` calls.

**Files:**
- Modify: `src/clif.rs`
- Test: `tests/backends.rs`

**Interfaces:**
- Consumes: `clif_type`, the Task 3.2 helpers, `Unit::inplace`, `Unit::releasable`.
- Produces: `fn function(&mut self, f: &FunDecl) -> Result<(), String>` — defines `vbf_<name>` with the same C-visible signature the C backend gives it (`VbVal (*)(VbVal *)`), so that the two backends' objects are interchangeable at link time. That interchangeability is worth preserving deliberately: it makes a mixed build possible as a debugging tool.

- [ ] **Step 1: Mirror the C backend's control flow, do not invent a new one**

`codegen.rs` emits every function as `for (;;) { ... }` with `vbret` assigned and `break` in tail position, so that a saturated self-call becomes a parameter rebind and a `continue`. In Cranelift that is a loop header block with block parameters: one per Vibelang parameter. A self-tail-call jumps back to the header with new arguments; every other tail position jumps to an exit block carrying the result. Spec §11.2 requires this to be emitted by the compiler, not left to an optimiser — the same requirement, the same shape, in a different notation.

- [ ] **Step 2: Note the one place Cranelift is better, and use it**

Spec §11.2 wants mutual tail calls to be guaranteed or to fail visibly, and in C that needs `[[gnu::musttail]]`, which MSVC does not have and which the C backend does not emit today. Cranelift has `return_call` and `return_call_indirect`, which are guaranteed tail calls in the IR, subject to the callee using the `tail` calling convention. Use them. Record in the commit message that this is a capability the C backend does not have, so that the §11.2 gap is closed on one backend and still open on the other rather than being quietly forgotten on both.

- [ ] **Step 3: Match lowering**

`match_chain` in `codegen.rs` emits a cascade of tag comparisons with field extraction in each arm. Lower the same cascade with `brif` / `br_table`; the arms are blocks and the join is a block with one parameter, the arm's value. Exhaustiveness is already guaranteed by the middle end, so the fall-through case is unreachable — emit `trap` with a distinctive code rather than a silent fall-through, so that a bug in this lowering is loud.

- [ ] **Step 4: Test against the C backend's answer**

Every test in this task takes the form: build the same `.vibe` file with both backends and compare stdout. That is the only assertion that scales, and it is what Task 3.10 generalises.

- [ ] **Step 5: Commit incrementally**

One commit per expression family (literals, `let`/`<-`, application, `match`, records, lists, lambdas), each with its differential test. A single "implement codegen" commit is not reviewable.

---

### Task 3.5: Closures, and the variadic runtime functions

**Files:**
- Modify: `src/clif.rs`, `runtime/vibert.h`, `runtime/vibert.c`
- Test: `tests/backends.rs`

**Interfaces:**
- Produces: non-variadic runtime entry points, used by **both** backends.

- [ ] **Step 1: Face the problem**

Three runtime functions are C variadics: `vb_obj(const VbInfo*, uint32_t tag, uint32_t n, ...)`, `vb_vec_lit(uint32_t n, ...)` and `vb_fmt(VbVal f, uint32_t n, ...)`. Calling a C variadic from Cranelift is not portable: the variadic ABI differs from the fixed one (on AArch64 Apple platforms variadic arguments go on the stack regardless of register availability), and Cranelift does not model it.

- [ ] **Step 2: Fix it in the runtime, for both backends**

Add a non-variadic form of each — `vb_obj_n(const VbInfo*, uint32_t tag, uint32_t n, const VbVal *fields)` and the same shape for the other two — implemented as the real body, with the variadic version kept as a thin wrapper that packs its `va_list` into an array and calls it. Then switch the C backend to the array form as well, so there is one code path through the runtime and the two backends are testing the same thing. The C backend's output changes in this commit; that is expected and the e2e outputs must not.

- [ ] **Step 3: Closures**

`VbClos { VbFn fn; const char *name; uint32_t arity, nargs; VbVal *args; }` with `VbFn = VbVal (*)(VbVal *)`. Emitting one from Cranelift is `vb_clos(func_addr, name_ptr, arity)`, where `func_addr` comes from `declare_func_in_func` plus `func_addr`. Partial application already lives in `vb_apply1`/`vb_applyn`; do not reimplement it in the backend.

Note while you are here: the C backend has a latent bug in this area — `Gen::value_ref` references a `vbe_<name>` wrapper for an `ext c` symbol used as a value, and no such wrapper is ever emitted. Fix it in the C backend in its own commit before mirroring the behaviour, or you will faithfully reproduce a bug.

- [ ] **Step 4: Test and commit**

Differential test on a program that builds a closure, partially applies it, and passes it to `map`.

---

### Task 3.6: Structural drop functions as Cranelift instructions

**Blocked on** `docs/static-drop-roadmap.md`'s emission contract. Do not start until Gate A.

**Files:**
- Modify: `src/clif.rs`
- Test: `tests/backends.rs`

- [ ] **Step 1: Read the contract, then decide where the function lives**

If the drop roadmap's contract is "one drop function per type, called at each drop point", generate those functions as Cranelift functions with `Linkage::Local`, one per record and ADT, each walking its fields and calling the field type's drop function. The walk is driven by `ck.data.records` and `ck.data.ctors` — the same tables the C backend uses to emit `VbInfo` descriptors — so it is a structural recursion over data already in the IR, with no analysis in the backend.

- [ ] **Step 2: Watch for the cycle**

A recursive type gives a recursive drop function; declare every drop function before defining any of them, exactly as the C backend forward-declares every `vbf_` before emitting any body.

- [ ] **Step 3: Test differentially, then by resident set**

`examples/churn.vibe` is the existing memory test: it builds and discards a five-thousand-element vector two thousand times and must stay flat. Run it under both backends and compare peak RSS with `/usr/bin/time -l` (macOS) or `-v` (GNU). Do not record a number in this document from one machine and call it a benchmark; record *flat versus growing*, which is the property that matters and the one the README already claims.

---

### Task 3.7: The bump allocator

**Files:**
- Modify: `src/clif.rs`
- Test: `tests/backends.rs`

- [ ] **Step 1: Start with calls, not with inlining**

`vb_alloc`, `vb_mark` and `vb_release` are ordinary C functions; call them. `Unit::releasable` already says which functions may be bracketed — the same set the C backend uses — so the emission is: at function entry, if `u.releasable.contains(&f.name)`, call `vb_mark` and keep the returned `VbMark` (itself a two-word struct: a pointer and a `size_t`, so the Task 3.2 lowering applies); before every return, call `vb_release(mark, result)`.

- [ ] **Step 2: Only then consider inlining the fast path**

`vb_alloc`'s hot path is three loads, a comparison and a store against the file-static `g_chunk`. Inlining it means referencing that symbol from generated code, which means it stops being `static` — a runtime ABI change affecting both backends, for a benefit nobody has measured on a program whose values are all boxed anyway. Defer it. If it is ever done, it is a runtime change first and a backend change second.

- [ ] **Step 3: Test and commit**

The `churn` test from Task 3.6 covers this; add a differential test for `arena a in ...` blocks.

---

### Task 3.8: `ext c` calls without a header

**Files:**
- Modify: `src/clif.rs`
- Test: `tests/backends.rs`

- [ ] **Step 1: Note what changes**

The C backend emits `#include <stdio.h>` and writes `puts(x)`, and the C compiler supplies the prototype. Cranelift has no headers: the call signature has to be *constructed* from the `ExtSig`'s Vibelang type, through `cabi::flatten_fn` and the Task 3.3 mapping. The `ExtSig::symbol` field already carries the C symbol name when it differs from the Vibelang one.

- [ ] **Step 2: Declare and call**

For each `ext c` symbol used, build a `Signature` from its parameter and return types, `declare_function(symbol, Linkage::Import, &sig)`, and call it. Unbox each `VbVal` argument to its machine type first — `cabi::c_unbox`'s Cranelift twin — and box the result. `cabi::check` from Task 2.2 has already rejected the types that cannot cross, so a `None` here is an internal error, not a user error: `unreachable!` with a message naming Task 2.2.

- [ ] **Step 3: The `-l` flags still apply**

`link::ext_flags` is backend-neutral and must be passed to whatever performs the link. An `ext c` program built through Cranelift still needs the library on the link line; what it does not need is a C *compiler* to produce the caller's code.

- [ ] **Step 4: Test and commit**

`examples/ffi.vibe` under both backends, asserting the same output. Note that a program with an `ext c` block is precisely the case where the user has a C toolchain anyway, so this task is about correctness and uniformity, not about the toolchain goal.

---

### Task 3.9: `exp c` exports and `--lib`

**Files:**
- Modify: `src/clif.rs`
- Test: `tests/e2e.rs` (the existing `exported_functions_get_a_header` test, extended)

- [ ] **Step 1: Emit the wrappers**

`Gen::exports` builds, for each exported name, a C-ABI function that boxes its arguments into a `VbVal[]`, calls `vbf_<name>`, and unboxes the result — plus a `vb_init()` call so the C caller has nothing to initialise. Mirror it exactly: same symbol name (`<Module>_<name>`), same signature from `cabi::split_fn` and the §10.3 mapping, same `vb_init` call, `Linkage::Export`.

- [ ] **Step 2: The header comes from `cabi`, not from the backend**

`cabi::header` is already backend-neutral after Task 2.1. `build_lib` writes it beside the archive, identically to the C backend, and the `NOT VERIFIED ACROSS THE BOUNDARY` comment is byte-identical. The existing header test must pass unchanged under both backends.

- [ ] **Step 3: Archive**

`link::ar` over the generated object plus the runtime object. Whether the runtime object comes from `cc` or from whatever Task 3.0b decided is that decision's business, not this task's.

- [ ] **Step 4: Test and commit**

The `--lib` path, built under Cranelift, linked from a C program with `cc -std=c11 -o use use.c -L. -lmathlib -lm`, producing the same output as the C-built archive.

---

### Task 3.10: The equivalence gate

**Files:**
- Modify: `tests/backends.rs`
- Modify: `Cargo.toml` (make `cranelift` a default feature, if Task 3.0a's recommendation is being followed)
- Modify: `README.md` (Status)

**Interfaces:**
- Produces: the property the whole plan exists to establish.

- [ ] **Step 1: Write the test**

```rust
// tests/backends.rs
/// Two independent lowerings of one IR that agree are evidence. One lowering
/// is not. Every example must produce byte-identical output under both.
#[test]
#[cfg(feature = "cranelift")]
fn the_backends_agree() {
    for e in ["examples/hello.vibe", "examples/ledger.vibe", "examples/churn.vibe"] {
        let (ok_c, out_c) = vibe(&["run", e, "--backend=c"]);
        let (ok_k, out_k) = vibe(&["run", e, "--backend=cranelift"]);
        assert_eq!(ok_c, ok_k, "{e}: backends disagree on success");
        assert_eq!(out_c, out_k, "{e}: backends disagree on output");
    }
}
```

`examples/ledger.vibe` reads `ledger.csv` from the working directory, so the helper's `current_dir` must stay the manifest directory.

- [ ] **Step 2: Run it**

Run: `cargo test --features cranelift`
Expected: PASS. Every disagreement is a bug in one of the two, and the C backend is the older and better-tested of them — start by assuming Cranelift is wrong.

- [ ] **Step 3: Move the README lines**

Add the Cranelift backend to **Works**, with the exact statement of what it does and does not remove — in particular, whether a C toolchain is still needed, which depends entirely on Task 3.0b's answer. Do not write "no C compiler required" unless a machine with no C compiler has built a program.

- [ ] **Step 4: Commit**

```bash
git add tests/backends.rs Cargo.toml Cargo.lock README.md
git commit -m "test(backends): the two backends agree on every example"
```

---

## The open question: the runtime without a C toolchain

`runtime/vibert.c` is 746 lines of C, `include_str!`-ed into the compiler and compiled by the user's `cc` on every build. Cranelift removes the C compiler from the *generated code* path and leaves it in the *runtime* path. Nothing in this plan pretends otherwise, and nothing below is decided. Task 3.0b is where it gets decided, with measurements.

**Option A — ship prebuilt runtime objects.** Compile `vibert.c` once per target triple at release time, embed the resulting archives in the `vibe` binary, write the right one out at build time.
*For*: the runtime stays one C file, one implementation, readable, debuggable, and identical for both backends. No rewrite, no second source of truth.
*Against*: `vibe` becomes per-target rather than portable; a new target needs a release, not a rebuild; the binary grows by one archive per supported triple; cross-compilation needs the target's archive to exist; and building the release artifacts needs a C toolchain matrix in CI. Also: the archives are binary artifacts, which either go in git or come from a release pipeline that must itself be trustworthy.

**Option B — rewrite the runtime in Rust.** `runtime/vibert.c` becomes a Rust crate compiled as part of, or alongside, `vibec`, with `#[no_mangle] extern "C"` entry points so both backends call it identically.
*For*: one toolchain (the one the user already needs to have built `vibe`), cross-compiles wherever Rust does, and the ~18 libc functions the runtime uses are all available through Rust's `std`. It also puts the runtime under `cargo test`, which it is not today.
*Against*: a rewrite of the one component whose correctness is least covered by tests; the `exp c` story needs the runtime to be linkable into a C program, so it has to be built as a `staticlib` and the resulting `.a` embedded and written out — which is Option A's distribution problem wearing a different hat, only with `rustc` instead of `cc`; and it deletes the property that a user can read `vibert.c` next to their generated C and understand the whole program.

**Option C — emit the runtime from `vibec` as Cranelift IR.** The compiler generates the runtime's functions itself.
*For*: no external anything.
*Against*: two implementations of the runtime to keep in step, in two notations, one of which is not readable. It also violates the "no logic in the backend" rule about as directly as anything could. Listed for completeness and for the record that it was considered.

**Option D — write the runtime in Vibelang.** The self-hosting answer.
*For*: the honest endpoint, and it would exercise the language hard.
*Against*: Vibelang has no raw memory primitives, and the allocator is the runtime; this is a multi-year item and not a Phase 3 answer. It does not stop being interesting for that reason.

**And the linker, which is a separate question with the same shape.** `cranelift-object` emits a relocatable object. Something must still link it, and on a typical Unix the usual route to a linker is `cc`. Sub-options: invoke `ld`/`ld64` directly, which is not a C compiler but does need the C runtime startup objects located; use `rust-lld` from the Rust toolchain the user already has; or ship a linker. Task 3.0b Step 3 tests this *before* the runtime question, because if the linker path drags in a C toolchain regardless, the runtime options are being compared on a false premise.

**Provisional reading, with the evidence it rests on stated plainly**: Option B plus `rust-lld` is the only combination that plausibly reaches "no C toolchain on the user's machine" without shipping per-triple binaries, and it has the largest rewrite cost of the four. That is a hypothesis, not a decision, and the only evidence behind it is a reading of what each option needs — no prototype has been built. Do not treat it as settled.

---

## What would signal the design is going wrong

Concrete, observable, and each one a reason to stop and reconsider rather than push through:

- **`tests/layering.rs` fails, and the fix is to edit the test.** The rule is the point; a backend that needs to run a semantic pass has found a gap in the IR, and the gap is what should be fixed.
- **A `match` on which backend is active appears anywhere outside `backend::select` and the driver's flag handling.** Backend-specific behaviour belongs behind the trait or nowhere.
- **The two backends disagree on an example and the resolution is to skip the example.** `the_backends_agree` is the whole value of keeping two backends; a skipped case is a lie in the test suite.
- **A trait method appears that only one backend can implement meaningfully.** Either it is not a backend concern, or the trait is the wrong shape. `emit_source` returning `Unsupported` is fine — a source form genuinely is optional. A method called `emit_c_specific_thing` is not.
- **The C backend gets worse.** Slower, uglier, less portable, or losing `#line`, in order to make Cranelift's job easier. This is explicitly forbidden above and is the most likely way the design fails, because it always looks locally reasonable.
- **A second runtime appears.** `vibert_clif.c`, or a Rust runtime that diverges from the C one, or a runtime function whose behaviour depends on which backend called it. One runtime, one behaviour, or the equivalence test is meaningless.
- **An IR change lands whose only motivation is Cranelift.** The IR serves the language; if Cranelift wants something the language does not need, Cranelift can compute it locally from what it is given, or the request is wrong.
- **Phase 3 starts before Gate A.** Every emission-contract change then has to be made twice and argued about once.
- **Someone reports a performance win from Cranelift.** Nothing in Phase 3 makes anything faster — every value is still a boxed `VbVal` and every operation still dispatches on a tag. A measured win means something else changed, and it is worth finding out what before believing it.
- **The `.vibe-<stem>` working directory grows backend-specific subdirectories that the other backend cannot read.** The build tree is a debugging artifact; keeping it legible is cheap and keeping it comparable across backends is most of how Task 3.10's failures get diagnosed.

---

## Explicitly out of scope

None of these are bad ideas. They are not this plan, and folding any of them in is how this plan stops finishing.

- **Unboxing / type-directed representation.** The `vibec debt` note at the top of `runtime/vibert.h` names it, and it is the single largest performance item in the project. It belongs to the IR and benefits both backends equally; it is a separate plan, and doing it first would have been defensible. Doing it *during* Phase 3 would make every differential test failure ambiguous.
- **Optimisation passes of any kind.** Cranelift's own `opt_level` is a flag; anything beyond that is a separate project.
- **Debug info.** The C backend emits `#line` and gets source-level debugging free. Cranelift emits none without deliberate DWARF work through `gimli`. Phase 3 ships a Cranelift backend whose output is not source-debuggable, and that is a stated limitation, not an oversight.
- **JIT.** `cranelift-jit` exists and `vibe run` could use it to skip the object-and-link round trip entirely. Attractive, genuinely useful for the check-fix loop the project is built around, and a different plan with different failure modes.
- **Cross-compilation.** Cranelift can target a triple other than the host; nothing in this plan exposes that, and the runtime question has to be answered before it can be.
- **Windows and MSVC.** The `VbVal` return ABI differs (Task 3.2, Step 3), and the linker story differs. The C backend covers Windows; Cranelift does not, in this plan.
- **LLVM or libgccjit.** Spec §11.1 keeps both as future options *"da valutare solo se emergono ottimizzazioni non esprimibili in C"* — to be evaluated only if optimisations arise that C cannot express. That condition has not arisen, and Cranelift is being added for toolchain independence, not for optimisation, which is a different reason than §11.1 contemplates. Worth an explicit note in §11.1 when Phase 3 lands.
- **`own T` / `ref T` at the export boundary, and the generated `<name>_free`.** Spec §10.2 asks for them and the compiler passes uniform `VbVal` instead. That is §10 work, not backend work, and both backends inherit whatever it decides.
- **Self-hosting.** Option D above.
- **Threads, signals, unwinding.** Spec §11.3 rules all three out of the runtime, and nothing here changes that.

---

## Appendix: commands used in this survey

Reproducible, so the next reader can check whether this document has rotted.

```bash
vibe proof examples/ledger.vibe              # five open obligations
vibe proof examples/ledger.vibe --prove      # two remain; needs z3 on PATH
grep -rn "vbe_" src/ runtime/                # one hit: the reference, never the definition
grep -n '\.\.\.' runtime/vibert.h            # three variadic runtime functions
wc -l src/*.rs runtime/*                     # codegen.rs ~1264, refine.rs ~1212, vibert.c ~746
```

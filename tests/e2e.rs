//! One end-to-end check: every example must check, and the reference program
//! must produce the documented output. If codegen or the runtime breaks, this
//! fails. ponytail: no harness, no fixtures — the compiler is the fixture.

use std::io::Write;
use std::process::{Command, Stdio};

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

/// Same, with something on standard input. `read_stdin` cannot be tested any
/// other way: the point of it is the pipe.
fn vibe_piped(args: &[&str], input: &str) -> (bool, String) {
    let mut ch = Command::new(VIBE)
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("vibe runs");
    ch.stdin
        .as_mut()
        .expect("a pipe")
        .write_all(input.as_bytes())
        .expect("the child takes its input");
    let o = ch.wait_with_output().expect("the child finishes");
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.success(), s)
}

#[test]
fn examples_check() {
    for e in [
        "examples/hello.vibe",
        "examples/ledger.vibe",
        "examples/grep.vibe",
        "examples/nested.vibe",
    ] {
        let (ok, out) = vibe(&["check", e]);
        assert!(ok, "{e} failed to check:\n{out}");
    }
}

#[test]
fn reference_program_runs() {
    let (ok, out) = vibe(&["run", "examples/ledger.vibe"]);
    assert!(ok, "ledger failed to build or run:\n{out}");
    assert_eq!(
        out.trim(),
        "n=3 tot=20.75 avg=6.91667 top=b",
        "unexpected output"
    );
}

#[test]
fn bad_program_reports_a_diagnostic() {
    let (ok, out) = vibe(&["check", "tests/bad.vibe", "--diag=struct"]);
    assert!(!ok, "a type error must fail the check");
    assert!(
        out.contains("type.mismatch"),
        "expected type.mismatch, got:\n{out}"
    );
}

#[test]
fn c_ffi_calls_libc() {
    let (ok, out) = vibe(&["run", "examples/ffi.vibe"]);
    assert!(ok, "ext c example failed:\n{out}");
    assert!(out.contains("through libc"), "libc puts must run:\n{out}");
    // C says only that `puts` returns a nonnegative value on success; the exact
    // number is the libc's business (glibc gives the byte count, macOS does not)
    let n: i32 = out
        .split("puts returned ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| panic!("its result must come back:\n{out}"));
    assert!(n >= 0, "puts reported failure: {n}\n{out}");
}

#[test]
fn exported_functions_get_a_header() {
    let (ok, out) = vibe(&["build", "examples/ledger.vibe"]);
    assert!(ok, "build failed:\n{out}");
    let h = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/.vibe-ledger/ledger.h"),
    )
    .expect("a header is generated for `exp c`");
    assert!(
        h.contains("double Ledger_mean(VbVal x0);"),
        "mean must be exported:\n{h}"
    );
    assert!(
        h.contains("NOT VERIFIED ACROSS THE BOUNDARY"),
        "the precondition must be flagged:\n{h}"
    );
}

#[test]
fn json_diagnostics_are_json() {
    let (ok, out) = vibe(&["check", "tests/bad.vibe", "--diag=json"]);
    assert!(!ok);
    assert!(
        out.trim_start().starts_with("[{"),
        "expected a JSON array, got:\n{out}"
    );
    assert!(
        out.contains("\"code\""),
        "expected a code field, got:\n{out}"
    );
}

#[test]
fn emit_c_keeps_the_generated_source() {
    let (ok, out) = vibe(&["build", "examples/hello.vibe", "--emit-c"]);
    assert!(ok, "build --emit-c failed:\n{out}");
    let c = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello.c");
    assert!(c.exists(), "--emit-c must leave the C next to the output");
    std::fs::remove_file(c).ok();
}

/// Blank lines used to be a canonicity error. Whitespace no longer reaches the
/// AST, so the file that exists to prove it now has to compile.
#[test]
fn blank_lines_carry_no_meaning() {
    let (ok, out) = vibe(&["check", "tests/blank.vibe"]);
    assert!(ok, "a run of blank lines is whitespace:\n{out}");
}

#[test]
fn a_library_links_into_a_c_program() {
    let dir = std::env::temp_dir().join("vibe-lib-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let archive = dir.join("libmathlib.a");
    let (ok, out) = vibe(&[
        "build",
        "--lib",
        "examples/mathlib.vibe",
        "-o",
        archive.to_str().expect("utf-8"),
    ]);
    assert!(ok, "building a library failed:\n{out}");
    assert!(archive.exists(), "no archive at {}", archive.display());
    assert!(
        dir.join("mathlib.h").exists(),
        "the header must sit next to the archive"
    );
    assert!(
        dir.join("vibert.h").exists(),
        "and so must the runtime header it includes"
    );

    let c = dir.join("use.c");
    std::fs::write(
        &c,
        "#include <stdio.h>\n#include \"mathlib.h\"\n\
         int main(void) {\n  printf(\"%g %g\\n\", MathLib_area(3.0, 4.0), MathLib_perimeter(1.0, 2.0));\n  return 0;\n}\n",
    )
    .expect("write the C caller");
    let exe = dir.join("use");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let st = Command::new(&cc)
        .current_dir(&dir)
        .args(["-std=c11", "-o"])
        .arg(&exe)
        .arg(&c)
        .args(["-L.", "-lmathlib", "-lm"])
        .status()
        .expect("a C compiler");
    assert!(st.success(), "C could not link the library");
    let run = Command::new(&exe)
        .output()
        .expect("the linked program runs");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "12 6");
}

#[test]
fn a_library_with_no_exports_is_refused() {
    let (ok, out) = vibe(&["build", "--lib", "examples/hello.vibe"]);
    assert!(!ok, "a library with no `exp c` has no symbols:\n{out}");
    assert!(out.contains("no `exp c`"), "{out}");
}

/// An `ext c` symbol used as a value, not called directly. The compiler used to
/// emit C that referenced a wrapper it never defined, so the error arrived from
/// `cc`, about a generated file, which is the one place a compiler error must
/// never come from.
#[test]
fn an_ext_symbol_can_be_passed_as_a_value() {
    let dir = std::env::temp_dir().join("vibe-extvalue-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let exe = dir.join("ext_value");
    let (ok, out) = vibe(&[
        "build",
        "--emit-c",
        "tests/ext_value.vibe",
        "-o",
        exe.to_str().expect("utf-8"),
    ]);
    assert!(ok, "the generated C must compile:\n{out}");
    let c = std::fs::read_to_string(exe.with_extension("c")).expect("the emitted C");
    assert!(
        c.contains("static VbVal vbe_perror("),
        "the wrapper must be defined:\n{c}"
    );
    assert!(
        c.contains("perror(vb_as_cstr(a[0]))"),
        "and it must call the C function:\n{c}"
    );
}

/// The C boundary is a language rule (§10.3), so `vibe check` has to be the one
/// that says no. It used to pass, and `cc` found out later.
#[test]
fn a_type_that_cannot_cross_the_c_boundary_fails_at_check() {
    let (ok, out) = vibe(&["check", "tests/ffi_bad.vibe", "--diag=struct"]);
    assert!(!ok, "`check` must not pass what `build` refuses:\n{out}");
    assert!(out.contains("ffi.type"), "{out}");
    assert!(
        out.contains("FfiBad.takes"),
        "blame the signature, not a call site:\n{out}"
    );
}

/// Every prelude function added for stdin-driven programs, run once, including
/// both `None` branches of `slice` and `index_of`.
#[test]
fn the_added_prelude_functions_run() {
    let (ok, out) = vibe_piped(
        &["run", "tests/prelude_ops.vibe"],
        "ALPHA=1\nBeta=22\ngamma=333\nhi\n",
    );
    assert!(ok, "prelude_ops failed to build or run:\n{out}");
    assert_eq!(
        out.trim_end(),
        "alpha\nbeta\ngamma\n?\n\
         ALPHA\nBeta=\ngamma\nshort\n\
         ALPHA is 1\nBeta is 22\ngamma is 333\nhi",
        "unexpected output"
    );
}

/// The whole point of `read_stdin`: a filter in a pipe, which no program could
/// be before it existed.
#[test]
fn a_filter_reads_its_input_from_a_pipe() {
    let (ok, out) = vibe_piped(
        &["run", "examples/grep.vibe", "--", "ta"],
        "ALPHA=1\nBeta=22\ngamma=333\nhi\n",
    );
    assert!(ok, "grep failed to build or run:\n{out}");
    assert_eq!(out.trim_end(), "Beta=22", "only the matching line survives");
}

#[test]
fn a_filter_without_a_pattern_says_so() {
    let (ok, out) = vibe_piped(&["run", "examples/grep.vibe"], "anything\n");
    assert!(ok, "the usage path is not a failure:\n{out}");
    assert!(
        out.contains("usage:"),
        "expected the usage line, got:\n{out}"
    );
}

/// Generated headers are named after their module and land in the build dir,
/// which used to be on `-I`, so a module named after a libc header shadowed it
/// and the C compiler failed with a wall of missing declarations. `-iquote`
/// leaves `#include <...>` alone.
#[test]
fn a_module_named_after_a_libc_header_still_builds() {
    let (ok, out) = vibe(&["run", "tests/stdio.vibe"]);
    assert!(ok, "a module may be named `Stdio`:\n{out}");
    assert!(out.contains("ok"), "{out}");
}

/// The bump allocator is gone (docs/static-drop-roadmap.md, Task 9): every
/// value is freed where its owner dies. The answer is what must not change.
#[test]
fn churn_computes_the_same_answer_without_the_bump_allocator() {
    let dir = std::env::temp_dir().join("vibe-alloc-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let bin = dir.join("churn-exact");
    let (ok, out) = vibe(&[
        "build",
        "examples/churn.vibe",
        "-o",
        bin.to_str().expect("utf-8"),
    ]);
    assert!(ok, "building churn failed:\n{out}");
    let o = Command::new(&bin).output().expect("the program runs");
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "10000000");
}

/// Bit operations are prelude functions, so there is no precedence to get
/// wrong. The two that C leaves undefined are pinned here: a shift wider than
/// the word, and a signed right shift, which keeps the sign.
#[test]
fn the_bit_operations_agree_with_the_hardware() {
    let (ok, out) = vibe(&["run", "tests/bits.vibe"]);
    assert!(ok, "bits.vibe failed to build or run:\n{out}");
    assert_eq!(out.trim(), "255 7 6 1024 128 15 65 -4 0");
}

/// A bit pattern is an integer. A float is a mistake, not a value to truncate.
#[test]
fn a_bit_operation_refuses_a_float() {
    let dir = std::env::temp_dir().join("vibe-bits-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let src = dir.join("Half.vibe");
    std::fs::write(
        &src,
        "mod Half\n\nmain : E! Unit = out (show (band 1.5 3))\n",
    )
    .expect("fixture");
    let o = Command::new(VIBE)
        .arg("run")
        .arg(&src)
        .output()
        .expect("vibe runs");
    assert!(!o.status.success(), "a float is not a bit pattern");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("band needs an integer"), "{err}");
}

/// A value dies at the end of the body that binds it, and the body reads it, so
/// the free has to follow the read. `ex` returns an expression rather than a
/// statement, so a `vb_dispose` pushed after it landed ahead of it in the
/// emitted C and this read a freed string header.
#[test]
fn a_drop_follows_the_expression_that_reads_it() {
    let (ok, out) = vibe(&["run", "tests/drop_order.vibe"]);
    assert!(ok, "drop_order.vibe failed to build or run:\n{out}");
    assert_eq!(out.trim(), "5");
}

/// `dup` is `&a -> a` and copies in depth, so a `Vec` or a record can be the
/// second owned value a frame needs. Spec §4.4 offers "return a copy" as one of
/// three answers to the absence of lifetimes; it used to exist for `Str` alone.
#[test]
fn dup_copies_an_aggregate_and_leaves_the_original_owned() {
    let (ok, out) = vibe(&["run", "tests/dup_deep.vibe"]);
    assert!(ok, "dup_deep.vibe failed to build or run:\n{out}");
    assert_eq!(out.trim(), "7 ada");
}

/// Two records in one module may declare the same field name. Inference knows
/// which record each use site means and writes it down; codegen used to resolve
/// the field by its bare name, which picked whichever record was collected last
/// and emitted the wrong `vb_field` index — wrong output, no diagnostic.
#[test]
fn a_field_two_records_declare_resolves_to_the_right_one() {
    let (ok, out) = vibe(&["run", "tests/field_share.vibe"]);
    assert!(ok, "field_share.vibe failed to build or run:\n{out}");
    assert_eq!(out.trim(), "99 2");
}

/// A drop is deep, so a structure built and thrown away in a loop comes back
/// whole. `examples/nested.vibe` splits a string into eight fresh ones every
/// iteration and keeps none of them: 1.5 MB here, 52 MB when `vb_dispose` freed
/// the spine and left the elements.
#[test]
fn a_nested_structure_is_freed_to_the_bottom() {
    let (ok, out) = vibe(&["run", "examples/nested.vibe"]);
    assert!(ok, "nested.vibe failed to build or run:\n{out}");
    assert_eq!(out.trim(), "1600000");
}

/// A dictionary associates a key with a value, which nothing in the prelude did
/// before. It is a vector of two-field objects underneath, so a drop already
/// frees it to the bottom and `len` already counted it; `insert` replaces a live
/// key rather than growing, and `lookup` copies the value out because handing
/// out the entry's own pointer would be interior to a dictionary the caller
/// still owns.
#[test]
fn a_dictionary_associates_a_key_with_a_value() {
    let (ok, out) = vibe(&["run", "tests/dict.vibe"]);
    assert!(ok, "dict.vibe failed to build or run:\n{out}");
    assert_eq!(out.trim(), "2 1 20 0 2 2");
}

/// Two drops the checker used to place twice: a payload moved out of an owned
/// scrutinee was freed again with the scrutinee, and a self-tail-call inside a
/// nested match freed its locals both at the back edge and at the scope end.
#[test]
fn a_moved_payload_and_a_nested_back_edge_are_freed_once() {
    let (ok, out) = vibe(&["run", "tests/payload_move.vibe"]);
    assert!(ok, "payload_move.vibe failed to build or run:\n{out}");
    assert_eq!(
        out.split_whitespace().collect::<Vec<_>>(),
        ["kept", "a-a-a-", "x+++"]
    );
}

/// A string literal is UTF-8 bytes, not one char per byte, and `byte_at` /
/// `byte_str` reach those bytes: 0xC3 ^ 0xA9 is 106.
#[test]
fn a_string_is_bytes_and_bytes_are_reachable() {
    let (ok, out) = vibe(&["run", "tests/bytes.vibe"]);
    assert!(ok, "bytes.vibe failed to build or run:\n{out}");
    assert!(out.starts_with("106 A "), "{out}");
    assert!(
        out.contains(" 18446744073709551615"),
        "wrapping arithmetic:\n{out}"
    );
}

/// Three refusals of programs that were fine: a tuple of names as the only
/// pattern, a recursive function calling a helper outside its group, and an
/// integer literal above `i64::MAX`.
#[test]
fn a_tuple_pattern_a_helper_call_and_a_u64_literal_are_accepted() {
    let (ok, out) = vibe(&["run", "tests/tuple_total.vibe"]);
    assert!(ok, "tuple_total.vibe failed to build or run:\n{out}");
    assert_eq!(out.trim(), "18446744073709551613");
}

/// A program killed by a signal used to leave `vibe run` exiting 1 with nothing
/// said, which is how a double free looked from outside.
#[test]
fn a_signal_is_reported() {
    let (ok, out) = vibe(&["run", "tests/signal.vibe"]);
    assert!(!ok);
    assert!(out.contains("run.signal"), "{out}");
}

/// A constructor is covered only by an arm that takes every payload: `|Some 3`
/// with `|None` used to check and then fail at run time on `Some 4`.
#[test]
fn a_refutable_payload_does_not_cover_its_constructor() {
    let dir = std::env::temp_dir().join("vibe-exhaustive-test");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("ex.vibe");
    std::fs::write(
        &f,
        "mod Ex\n\nf (o:Opt U64) : U64 =\n  ?o |Some 3 -> 1\n     |None   -> 0\n  end\n",
    )
    .unwrap();
    let (ok, out) = vibe(&["check", f.to_str().unwrap()]);
    assert!(!ok && out.contains("match.nonexhaustive"), "{out}");
    // Arms that cover a constructor together still cover it.
    std::fs::write(
        &f,
        "mod Ex\n\nf (o:Opt (Opt U64)) : U64 =\n  ?o |Some (Some x) -> x\n     |Some None -> 0\n     |None -> 1\n  end\n\ng (p:(Bool, Bool)) : U64 =\n  ?p |(True, _) -> 1\n     |(False, True) -> 2\n     |(False, False) -> 3\n  end\n",
    )
    .unwrap();
    let (ok, out) = vibe(&["check", f.to_str().unwrap()]);
    assert!(ok, "{out}");
    // A literal past i64::MAX is a U64, never a signed value.
    std::fs::write(
        &f,
        "mod Ex\n\nf (x:I64) : Bool = x < 0\n\ng : Bool = f 18446744073709551615\n",
    )
    .unwrap();
    let (ok, out) = vibe(&["check", f.to_str().unwrap()]);
    assert!(!ok && out.contains("type.mismatch"), "{out}");
}

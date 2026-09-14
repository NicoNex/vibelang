//! One end-to-end check: every example must check, and the reference program
//! must produce the documented output. If codegen or the runtime breaks, this
//! fails. ponytail: no harness, no fixtures — the compiler is the fixture.

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
fn examples_check() {
    for e in ["examples/hello.vibe", "examples/ledger.vibe"] {
        let (ok, out) = vibe(&["check", e]);
        assert!(ok, "{e} failed to check:\n{out}");
    }
}

#[test]
fn reference_program_runs() {
    let (ok, out) = vibe(&["run", "examples/ledger.vibe"]);
    assert!(ok, "ledger failed to build or run:\n{out}");
    assert_eq!(out.trim(), "n=3 tot=20.75 avg=6.91667 top=b", "unexpected output");
}

#[test]
fn bad_program_reports_a_diagnostic() {
    let (ok, out) = vibe(&["check", "tests/bad.vibe", "--diag=struct"]);
    assert!(!ok, "a type error must fail the check");
    assert!(out.contains("type.mismatch"), "expected type.mismatch, got:\n{out}");
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
    assert!(h.contains("double Ledger_mean(VbVal x0);"), "mean must be exported:\n{h}");
    assert!(h.contains("NOT VERIFIED ACROSS THE BOUNDARY"), "the precondition must be flagged:\n{h}");
}

#[test]
fn json_diagnostics_are_json() {
    let (ok, out) = vibe(&["check", "tests/bad.vibe", "--diag=json"]);
    assert!(!ok);
    assert!(out.trim_start().starts_with("[{"), "expected a JSON array, got:\n{out}");
    assert!(out.contains("\"code\""), "expected a code field, got:\n{out}");
}

#[test]
fn emit_c_keeps_the_generated_source() {
    let (ok, out) = vibe(&["build", "examples/hello.vibe", "--emit-c"]);
    assert!(ok, "build --emit-c failed:\n{out}");
    let c = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello.c");
    assert!(c.exists(), "--emit-c must leave the C next to the output");
    std::fs::remove_file(c).ok();
}

#[test]
fn two_blank_lines_are_not_canonical() {
    let (ok, out) = vibe(&["check", "tests/blank.vibe", "--diag=struct"]);
    assert!(!ok, "two blank lines in a row must be rejected");
    assert!(out.contains("canon.blankline"), "expected canon.blankline, got:\n{out}");
}

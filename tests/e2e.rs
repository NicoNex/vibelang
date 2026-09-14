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

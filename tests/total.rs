//! Termination checking (spec §6), through the driver. ponytail: the fixtures
//! are four small .vibe files, one per outcome.

use std::process::Command;

const VIBE: &str = env!("CARGO_BIN_EXE_vibe");

fn check(file: &str) -> (bool, String) {
    let o = Command::new(VIBE)
        .args(["check", file, "--diag=struct"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.success(), s)
}

#[test]
fn inferred_and_explicit_measures_are_accepted() {
    let (ok, out) = check("tests/total_ok.vibe");
    assert!(ok, "total_ok.vibe must check:\n{out}");
}

#[test]
fn missing_measure_is_rejected() {
    let (ok, out) = check("tests/total_no_measure.vibe");
    assert!(!ok, "a recursive function with no measure must fail");
    assert!(out.contains("total.no_measure"), "expected total.no_measure, got:\n{out}");
    assert!(out.contains("TotalNoMeasure.spin"), "expected a semantic path, got:\n{out}");
}

#[test]
fn growing_measure_is_rejected() {
    let (ok, out) = check("tests/total_growing.vibe");
    assert!(!ok, "a measure that grows must fail");
    assert!(out.contains("total.not_decreasing"), "expected total.not_decreasing, got:\n{out}");
}

#[test]
fn examples_still_terminate() {
    for e in ["examples/hello.vibe", "examples/ledger.vibe"] {
        let (ok, out) = check(e);
        assert!(ok, "{e} failed to check:\n{out}");
    }
}

#[test]
fn recursion_inside_an_arena_is_still_recursion() {
    let (ok, out) = check("tests/total_arena.vibe");
    assert!(!ok, "a recursive call inside an `arena` block must still need a measure");
    assert!(out.contains("total.no_measure"), "expected total.no_measure, got:\n{out}");
}

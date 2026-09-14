//! Refinement obligations end to end (spec §7). The generated SMT-LIB text is
//! asserted on in `src/refine.rs`; here we only drive the binary. The discharge
//! assertions run only where z3 exists — detected, never assumed.

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

fn have_z3() -> bool {
    Command::new("z3")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[test]
fn obligations_are_counted_but_not_discharged_by_default() {
    let (ok, out) = vibe(&["check", "tests/refine_ok.vibe"]);
    assert!(ok, "check must keep working without a solver:\n{out}");
    assert!(out.contains("4 refinement obligation(s) not discharged"), "{out}");
}

#[test]
fn the_checked_forms_generate_no_obligation() {
    let (ok, out) = vibe(&["check", "tests/refine_checked.vibe"]);
    assert!(ok, "{out}");
    assert!(!out.contains("obligation"), "§7.5 must leave nothing to prove:\n{out}");
}

#[test]
fn prove_without_z3_says_so_clearly() {
    if have_z3() {
        return;
    }
    let (ok, out) = vibe(&["check", "--prove", "tests/refine_ok.vibe", "--diag=struct"]);
    assert!(!ok, "--prove cannot silently succeed without a solver:\n{out}");
    assert!(out.contains("refine.no-solver"), "{out}");
    assert!(out.contains("z3"), "{out}");
}

#[test]
fn provable_obligations_discharge() {
    if !have_z3() {
        return; // no solver here: nothing to discharge
    }
    let (ok, out) = vibe(&["check", "--prove", "tests/refine_ok.vibe"]);
    assert!(ok, "every obligation follows from the signature:\n{out}");
}

#[test]
fn an_open_obligation_is_an_error_with_a_counterexample() {
    if !have_z3() {
        return;
    }
    let (ok, out) = vibe(&["check", "--prove", "tests/refine_bad.vibe", "--diag=struct"]);
    assert!(!ok, "an undischarged obligation is an error, not a warning:\n{out}");
    assert!(out.contains("div0"), "{out}");
    assert!(out.contains("RefineBad.div_any.body/0"), "semantic path missing:\n{out}");
    assert!(out.contains("\u{22a8}"), "counterexample missing:\n{out}");
    assert!(out.contains("fix: RefineBad.div_any.sig += b != 0"), "{out}");
}

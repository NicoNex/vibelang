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
    assert!(
        out.contains("4 refinement obligation(s) not discharged"),
        "{out}"
    );
}

#[test]
fn the_checked_forms_generate_no_obligation() {
    let (ok, out) = vibe(&["check", "tests/refine_checked.vibe"]);
    assert!(ok, "{out}");
    assert!(
        !out.contains("obligation"),
        "§7.5 must leave nothing to prove:\n{out}"
    );
}

#[test]
fn prove_without_z3_says_so_clearly() {
    if have_z3() {
        return;
    }
    let (ok, out) = vibe(&["check", "--prove", "tests/refine_ok.vibe", "--diag=struct"]);
    assert!(
        !ok,
        "--prove cannot silently succeed without a solver:\n{out}"
    );
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
    assert!(
        !ok,
        "an undischarged obligation is an error, not a warning:\n{out}"
    );
    assert!(out.contains("div0"), "{out}");
    assert!(
        out.contains("RefineBad.div_any.body/0"),
        "semantic path missing:\n{out}"
    );
    assert!(out.contains("\u{22a8}"), "counterexample missing:\n{out}");
    assert!(
        out.contains("fix: RefineBad.div_any.sig += b != 0"),
        "{out}"
    );
}

#[test]
fn a_refinement_survives_a_constructor_pattern() {
    if !have_z3() {
        return;
    }
    let (ok, out) = vibe(&["check", "--prove", "tests/refine_ctor.vibe"]);
    assert!(
        ok,
        "the `Ok []` arm and the record invariant discharge these:\n{out}"
    );
}

#[test]
fn without_the_empty_arm_the_same_call_is_open() {
    if !have_z3() {
        return;
    }
    // the arm above is what teaches the solver `len xs != 0`; drop it and the
    // proof must fail, otherwise the previous test proves nothing
    let src = "mod Neg\n\
               head (v:&Vec U64, len v>0) : U64 = get v 0\n\
               pick (r:&Res Unit (Vec U64)) : U64 =\n  \
                 ?r |Er e  -> 0\n     \
                    |Ok xs -> head xs\n  end\n";
    let dir = std::env::temp_dir().join("vibe-refine-ctor");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let f = dir.join("neg.vibe");
    std::fs::write(&f, src).expect("write");
    let (ok, out) = vibe(&["check", "--prove", f.to_str().expect("utf-8")]);
    assert!(!ok, "`len xs > 0` does not follow from nothing:\n{out}");
    assert!(out.contains("refine.unproven"), "{out}");
}

/// Certificate caching (spec §16.5). The cache is keyed by the SMT text, which
/// is the entire question, so a hit cannot be a hit on a different question.
#[test]
fn a_proved_obligation_is_not_asked_twice() {
    if !have_z3() {
        return;
    }
    let dir = std::env::temp_dir().join("vibe-cache-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let f = dir.join("refine_ok.vibe");
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/refine_ok.vibe");
    std::fs::copy(&src, &f).expect("copy");
    let p = f.to_str().expect("utf-8");

    let (ok, out) = vibe(&["check", "--prove", p]);
    assert!(ok, "{out}");
    let cache = std::fs::read_to_string(dir.join(".vibe-proofs")).expect("a cache file");
    assert_eq!(
        cache.lines().count(),
        4,
        "one certificate per obligation:\n{cache}"
    );

    // The strongest statement the cache can make: with no solver on PATH the
    // same file still proves, so nothing was asked again.
    let o = Command::new(VIBE)
        .args(["check", "--prove", p])
        .env("PATH", "/nonexistent")
        .output()
        .expect("vibe runs");
    assert!(
        o.status.success(),
        "a fully cached run must need no solver:\n{}",
        String::from_utf8_lossy(&o.stderr)
    );

    // and an empty cache in the same place does need one
    std::fs::remove_file(dir.join(".vibe-proofs")).expect("drop the cache");
    let o = Command::new(VIBE)
        .args(["check", "--prove", p])
        .env("PATH", "/nonexistent")
        .output()
        .expect("vibe runs");
    assert!(
        !o.status.success(),
        "without a cache the solver is required"
    );
    assert!(String::from_utf8_lossy(&o.stderr).contains("refine.no-solver"));
}

#[test]
fn the_solver_budget_is_a_number_of_seconds() {
    let (ok, out) = vibe(&["check", "--prove-timeout=nope", "tests/refine_ok.vibe"]);
    assert!(!ok, "{out}");
    assert!(out.contains("not a number of seconds"), "{out}");
}

/// A binder that shadows a refined name must not inherit its hypotheses. This
/// is the one failure mode that matters more than any missed proof: the wrong
/// answer here is `unsat` on something false.
#[test]
fn a_shadowing_binder_does_not_inherit_a_refinement() {
    if !have_z3() {
        return;
    }
    let (ok, out) = vibe(&["check", "--prove", "tests/shadow.vibe", "--diag=struct"]);
    assert!(
        !ok,
        "`n>0` is about the parameter, not about whatever `Ok n` bound:\n{out}"
    );
    assert!(out.contains("div0"), "{out}");
    assert!(
        out.contains("n.1=0"),
        "the counterexample must be about the inner `n`:\n{out}"
    );
}

/// and the outer name still works where nothing shadows it
#[test]
fn an_unshadowed_refinement_still_discharges() {
    if !have_z3() {
        return;
    }
    let (ok, out) = vibe(&["check", "--prove", "tests/refine_ok.vibe"]);
    assert!(ok, "{out}");
}

/// Writes `src` to its own file under the temp dir and returns the path.
fn scratch(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join("vibe-refine-scratch");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let f = dir.join(format!("{name}.vibe"));
    std::fs::write(&f, src).expect("write");
    f.to_str().expect("utf-8").to_string()
}

/// Signed subtraction overflows as readily as unsigned subtraction underflows,
/// so it asks the same question.
#[test]
fn signed_subtraction_generates_an_overflow_obligation() {
    let f = scratch("Sub", "mod Sub\nf (a:I64) (b:I64) : I64 = a - b\n");
    let (ok, out) = vibe(&["proof", &f]);
    assert!(ok, "{out}");
    assert!(
        out.contains("Sub.f.body/0\toverflow"),
        "`a - b` on I64 must be an obligation:\n{out}"
    );
}

/// An obligation the fragment cannot phrase is counted, not dropped: a run that
/// says nothing is indistinguishable from a proof.
#[test]
fn an_inexpressible_obligation_is_counted() {
    // `get v 0` is not a term the solver has, so the divisor obligation about
    // it cannot be asked at all.
    let f = scratch(
        "Opaque",
        "mod Opaque\nd (a:U64) (v:&Vec U64) : U64 = a / (get v 0)\n",
    );
    let (ok, out) = vibe(&["proof", &f]);
    assert!(ok, "{out}");
    assert!(
        out.contains("1 obligation(s) could not be expressed"),
        "a question never asked must be reported:\n{out}"
    );
}

/// A float obligation is never reported as an exact proof: the solver reasons
/// over reals, the program does not.
#[test]
fn a_float_obligation_is_marked_approximated() {
    let f = scratch("Flt", "mod Flt\nd (a:F64) (b:F64) : F64 = a / b\n");
    let (ok, out) = vibe(&["proof", &f]);
    assert!(ok, "{out}");
    assert!(out.contains("approximated as mathematical reals"), "{out}");
}

/// The acceptance test for inferred postconditions: the reference program's
/// last two obligations close because `load`'s `Ok []` arm is carried across
/// the call into `main`.
#[test]
fn the_reference_program_has_no_open_obligation() {
    if !have_z3() {
        return;
    }
    let (ok, out) = vibe(&["check", "--prove", "examples/ledger.vibe"]);
    assert!(ok, "`len ts > 0` must follow from `load`:\n{out}");
    assert!(!out.contains("refine.unproven"), "{out}");
}

/// and the inference must abstain where the fact is not established on every
/// return path yielding that constructor. Same program, minus the one arm that
/// rules the empty case out: the obligation has to stay open.
#[test]
fn a_postcondition_is_not_inferred_without_the_arm_that_proves_it() {
    if !have_z3() {
        return;
    }
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ledger.vibe"),
    )
    .expect("the reference program");
    let cut: String = src
        .lines()
        .filter(|l| !l.contains("Ok [] -> Er Void"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_ne!(cut, src, "the arm this test removes must exist");
    let f = scratch("Ledger", &cut);
    let (ok, out) = vibe(&["check", "--prove", &f]);
    assert!(!ok, "nothing rules out `Ok []` here:\n{out}");
    assert!(out.contains("refine.unproven"), "{out}");
}

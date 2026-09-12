//! Ownership: an owned value may be used once. The reference program, which
//! borrows the same vector four times, must keep checking.

use std::process::Command;

fn vibe(args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.success(), s)
}

#[test]
fn double_move_is_rejected() {
    let (ok, out) = vibe(&["check", "tests/move.vibe", "--diag=struct"]);
    assert!(!ok, "using an owned parameter twice must fail");
    assert!(out.contains("own.use_after_move"), "expected own.use_after_move, got:\n{out}");
    assert!(out.contains("dup s"), "the fix must name the mechanical repair, got:\n{out}");
}

#[test]
fn repeated_borrows_are_fine() {
    let (ok, out) = vibe(&["check", "examples/ledger.vibe"]);
    assert!(ok, "borrowing the same value repeatedly must be allowed:\n{out}");
}

#[test]
fn unique_update_mutates_in_place() {
    let (ok, out) = vibe(&["run", "tests/inplace.vibe"]);
    assert!(ok, "in-place example failed:\n{out}");
    assert_eq!(out.trim(), "2");
    let c = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/.vibe-inplace/inplace.c"),
    )
    .expect("generated C is kept next to the example");
    assert!(c.contains("vb_set_fields"), "an owned `{{r with ...}}` must not copy:\n{c}");
}

#[test]
fn arena_block_releases_in_bulk() {
    let (ok, out) = vibe(&["run", "tests/arena.vibe"]);
    assert!(ok, "arena example failed:\n{out}");
    assert_eq!(out.trim(), "1000");
    let c = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/.vibe-arena/arena.c"),
    )
    .expect("generated C is kept next to the example");
    assert!(c.contains("vb_mark") && c.contains("vb_release"), "arena must mark and release:\n{c}");
}

#[test]
fn consecutive_comment_lines_are_not_blank_lines() {
    let (ok, out) = vibe(&["check", "tests/arena.vibe"]);
    assert!(ok, "two comment lines in a row must be canonical:\n{out}");
}

#[test]
fn a_let_binder_is_affine_too() {
    let (ok, out) = vibe(&["check", "tests/move_let.vibe", "--diag=struct"]);
    assert!(!ok, "a `let` name with no written type is still owned:\n{out}");
    assert!(out.contains("own.use_after_move"), "{out}");
    assert!(out.contains("MoveLet.f"), "the semantic path must name the function:\n{out}");
}

#[test]
fn a_pattern_binder_is_affine_too() {
    let (ok, out) = vibe(&["check", "tests/move_pat.vibe", "--diag=struct"]);
    assert!(!ok, "a name a pattern binds is owned:\n{out}");
    assert!(out.contains("own.use_after_move"), "{out}");
}

#[test]
fn a_scalar_binder_is_copied() {
    let (ok, out) = vibe(&["check", "tests/own_let.vibe"]);
    assert!(ok, "scalars are copied, so using one twice is not a move:\n{out}");
}

#[test]
fn an_escaping_closure_owns_its_captures() {
    let (ok, out) = vibe(&["check", "tests/capture.vibe", "--diag=struct"]);
    assert!(!ok, "a closure that outlives the call moves what it captured:\n{out}");
    assert!(out.contains("own.use_after_move"), "{out}");
    assert!(out.contains("Capture.grab"), "{out}");
}

#[test]
fn a_closure_that_dies_with_the_call_only_reads() {
    let (ok, out) = vibe(&["check", "tests/capture_ok.vibe"]);
    assert!(ok, "the argument to `map` is consumed during the call:\n{out}");
}

/// `Nat` was missing from the scalar list, so a natural number was treated as
/// an affine value and using it twice was a move error. The spec's own §6.2
/// example did not compile.
#[test]
fn a_nat_is_a_scalar_not_an_affine_value() {
    let (ok, out) = vibe(&["check", "tests/nat_scalar.vibe"]);
    assert!(ok, "a number is copied, not moved:\n{out}");
}

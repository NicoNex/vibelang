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

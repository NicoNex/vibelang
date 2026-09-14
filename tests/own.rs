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

//! The drop table (docs/static-drop-roadmap.md): where each owned value stops
//! being this scope's to free. The frontend prints it; nothing is emitted yet.

use std::process::Command;

const VIBE: &str = env!("CARGO_BIN_EXE_vibe");

fn drops(file: &str) -> String {
    let o = Command::new(VIBE)
        .args(["view", file, "--drops"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).expect("utf-8")
}

#[test]
fn a_let_bound_value_nothing_takes_is_dropped_at_the_end_of_its_scope() {
    let d = drops("tests/drop_let.vibe");
    assert!(d.contains("DropLet.keep.body: drop s"), "{d}");
}

#[test]
fn a_value_given_away_is_not_dropped_by_the_giver() {
    let d = drops("tests/drop_move.vibe");
    assert!(
        !d.contains("DropMove.give"),
        "`s` belongs to `eat` now, so `give` frees nothing:\n{d}"
    );
    assert!(
        d.contains("DropMove.eat"),
        "`eat` owns its parameter and drops it:\n{d}"
    );
}

#[test]
fn an_arm_that_keeps_the_value_drops_it_and_the_arm_that_gives_it_away_does_not() {
    let d = drops("tests/drop_arm.vibe");
    let lines: Vec<&str> = d
        .lines()
        .filter(|l| l.starts_with("DropArm.pick") && l.contains("drop s"))
        .collect();
    assert_eq!(lines.len(), 1, "exactly one arm still owns `s`:\n{d}");
}

#[test]
fn a_value_that_leaves_through_the_return_is_never_dropped() {
    let d = drops("tests/drop_escape.vibe");
    assert!(!d.contains("drop s"), "`s` is the result:\n{d}");
    assert!(!d.contains("drop t"), "`t` is inside the result:\n{d}");
    assert!(
        !d.contains("drop r"),
        "`r.s` is a pointer into `r`, so `r` outlives the frame too:\n{d}"
    );
}

#[test]
fn a_parameter_replaced_on_the_back_edge_is_dropped_each_iteration() {
    let d = drops("tests/drop_loop.vibe");
    assert!(
        d.contains("DropLoop.spin.body: drop s before the back edge"),
        "the old `s` dies on the edge, not when the frame finally returns:\n{d}"
    );
}

/// Runs `vibe build --emit-c` into a temp directory and reads the `.c` back —
/// the helper at the top of `tests/escape.rs`, which asserts on the same output.
fn emit_c(file: &str, stem: &str) -> String {
    let dir = std::env::temp_dir().join("vibe-drop-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out = dir.join(stem);
    let o = Command::new(VIBE)
        .args(["build", "--emit-c", file, "-o"])
        .arg(&out)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    std::fs::read_to_string(out.with_extension("c")).expect("the emitted C")
}

fn snapshot(stem: &str) -> String {
    let p = format!("{}/tests/snap/{stem}.c", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("no snapshot at {p}"))
}

/// The frontend knows where every value dies and the backend still does not act
/// on it. Update these snapshots in Task 8 of docs/static-drop-roadmap.md,
/// deliberately, and never to make a test pass.
#[test]
fn drop_points_change_no_emitted_c_yet() {
    for (file, stem) in [
        ("examples/churn.vibe", "churn"),
        ("examples/ledger.vibe", "ledger"),
        ("examples/hello.vibe", "hello"),
    ] {
        assert_eq!(
            emit_c(file, stem),
            snapshot(stem),
            "codegen must not move yet: {file}"
        );
    }
}

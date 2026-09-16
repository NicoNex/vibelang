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
        !d.lines().any(|l| l == "DropMove.give.body: drop s"),
        "`s` belongs to `eat` now, so `give` does not free it:\n{d}"
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
    let drops_name = |n: &str| d.lines().any(|l| l.ends_with(&format!(": drop {n}")));
    assert!(!drops_name("s"), "`s` is the result:\n{d}");
    assert!(!drops_name("t"), "`t` is inside the result:\n{d}");
    assert!(
        !drops_name("r"),
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

#[test]
fn a_value_that_may_alias_another_is_not_dropped_here() {
    let d = drops("tests/drop_alias.vibe");
    assert!(d.contains("drop v"), "the vector is this frame's:\n{d}");
    assert!(
        !d.contains("drop x"),
        "`x` is an element of `v`, not a value of its own:\n{d}"
    );
    assert!(
        d.contains("1 drop(s) suppressed"),
        "and the count says so:\n{d}"
    );
}

/// Peak resident set of a run, in bytes, read from `/usr/bin/time -l` — the
/// tool the README's `churn` figure was measured with. The crate has no
/// dependencies, so there is no `getrusage` to call; the platform gate is the
/// price of not adding one.
#[cfg(target_os = "macos")]
fn peak_rss(bin: &std::path::Path) -> u64 {
    let o = Command::new("/usr/bin/time")
        .arg("-l")
        .arg(bin)
        .output()
        .expect("time runs");
    let err = String::from_utf8_lossy(&o.stderr);
    let line = err
        .lines()
        .find(|l| l.contains("maximum resident set size"))
        .unwrap_or_else(|| panic!("no rss line in:\n{err}"));
    line.split_whitespace()
        .next()
        .expect("a number")
        .parse()
        .expect("a number")
}

/// The point of the whole exercise: a loop that allocates every iteration and
/// keeps nothing does not grow. The threshold is a ceiling, not a benchmark —
/// the claim is that the figure is flat, not that it is any particular number.
#[cfg(target_os = "macos")]
#[test]
fn a_loop_that_keeps_nothing_does_not_grow() {
    let dir = std::env::temp_dir().join("vibe-drop-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let bin = dir.join("loop-exact");
    let o = Command::new(VIBE)
        .args(["build", "--alloc=exact", "examples/loop.vibe", "-o"])
        .arg(&bin)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let rss = peak_rss(&bin);
    assert!(rss < 8 * 1024 * 1024, "the loop grew to {rss} bytes");
}

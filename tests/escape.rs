//! Escape analysis (spec §4.6). The question it used to answer — may this frame
//! release in bulk on the way out — no longer exists: there is no bulk release,
//! every value is freed where its owner dies. The question underneath it does:
//! a pointer handed to C is C's for as long as C likes, and nothing this frame
//! lent to it is this frame's to free.
//!
//! The generated C is still the artefact worth asserting on — it is
//! deterministic, and it fails the moment a frame frees something it should
//! not, or stops freeing something it should.

use std::process::Command;

const VIBE: &str = env!("CARGO_BIN_EXE_vibe");

fn emit_c(file: &str, stem: &str) -> String {
    let dir = std::env::temp_dir().join("vibe-escape-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out = dir.join(stem);
    let o = Command::new(VIBE)
        .args(["build", "--emit-c", file, "-o"])
        .arg(&out)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(
        o.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    std::fs::read_to_string(out.with_extension("c")).expect("the emitted C")
}

/// The body of one generated function, so a claim about `vbf_low` cannot be
/// satisfied by something `vbf_main` happens to contain.
fn body<'c>(c: &'c str, name: &str) -> &'c str {
    let head = format!("static VbVal vbf_{name}(VbVal *a) {{");
    let start = c
        .find(&head)
        .unwrap_or_else(|| panic!("no definition of {name} in:\n{c}"));
    let rest = &c[start + head.len()..];
    let end = rest.find("\nstatic VbVal vbf_").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn a_frame_frees_what_it_allocated_and_nobody_kept() {
    let c = emit_c("examples/churn.vibe", "churn");
    let work = body(&c, "work");
    assert_eq!(
        work.matches("vb_dispose(").count(),
        2,
        "`work` builds a vector, reverses it, and keeps neither:\n{work}"
    );
    assert!(
        !work.contains("vb_mark()") && !work.contains("vb_release("),
        "the bump allocator is gone:\n{work}"
    );
}

#[test]
fn handing_a_pointer_to_c_stops_the_free() {
    let c = emit_c("tests/taint_own.vibe", "taint_own");
    let hand = body(&c, "hand");
    assert!(
        !hand.contains("vb_dispose("),
        "C may keep the pointer `hand` gave it:\n{hand}"
    );
}

#[test]
fn the_taint_reaches_the_caller() {
    let c = emit_c("tests/taint_own.vibe", "taint_own");
    let top = body(&c, "top");
    assert!(
        !top.contains("vb_dispose("),
        "`top` owns the string C is holding below it:\n{top}"
    );
    // and it stops at values that never reach C
    let m = body(&c, "main");
    assert!(
        m.contains("vb_dispose("),
        "the taint must not spread to everything:\n{m}"
    );
}

#[test]
fn freeing_does_not_change_the_answer() {
    let dir = std::env::temp_dir().join("vibe-escape-tests");
    let exe = dir.join("churn");
    emit_c("examples/churn.vibe", "churn");
    let o = Command::new(&exe).output().expect("the built program runs");
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "10000000");
}

/// A result that carries a pointer — `CStr` and `Ptr`, not only the boxed
/// types — points into a value the frame must not have freed.
#[test]
fn a_returned_pointer_is_not_freed_under_the_caller() {
    let dir = std::env::temp_dir().join("vibe-escape-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let exe = dir.join("dangle");
    let o = Command::new(VIBE)
        .args(["run", "tests/dangle.vibe", "-o"])
        .arg(&exe)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).trim(),
        "hello there world",
        "the frame freed memory its own result still points into"
    );
}

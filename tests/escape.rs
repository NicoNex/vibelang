//! Escape analysis (spec §4.6): which frames may release what they allocated.
//! The generated C is the artefact worth asserting on — it is deterministic,
//! and it fails the moment a frame stops releasing.

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
    assert!(o.status.success(), "build failed:\n{}", String::from_utf8_lossy(&o.stderr));
    std::fs::read_to_string(out.with_extension("c")).expect("the emitted C")
}

/// The body of one generated function, so a claim about `vbf_low` cannot be
/// satisfied by something `vbf_main` happens to contain.
fn body<'c>(c: &'c str, name: &str) -> &'c str {
    let head = format!("static VbVal vbf_{name}(VbVal *a) {{");
    let start = c.find(&head).unwrap_or_else(|| panic!("no definition of {name} in:\n{c}"));
    let rest = &c[start + head.len()..];
    let end = rest.find("\nstatic VbVal vbf_").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn a_frame_that_cannot_leak_releases_what_it_allocated() {
    let c = emit_c("examples/churn.vibe", "churn");
    let work = body(&c, "work");
    assert!(work.contains("vb_mark()"), "`work` allocates a vector nothing keeps:\n{work}");
    assert!(work.contains("vb_release(vbm, vbret)"), "{work}");
}

#[test]
fn handing_a_pointer_to_c_stops_the_release() {
    let c = emit_c("tests/taint.vibe", "taint");
    let low = body(&c, "low");
    assert!(!low.contains("vb_mark()"), "C may keep the pointer `low` gave it:\n{low}");
}

#[test]
fn the_taint_reaches_the_caller() {
    let c = emit_c("tests/taint.vibe", "taint");
    let high = body(&c, "high");
    assert!(
        !high.contains("vb_mark()"),
        "releasing in `high` would free what C is holding below it:\n{high}"
    );
    // and it stops at functions that cannot reach C
    let clean = body(&c, "clean");
    assert!(clean.contains("vb_mark()"), "the taint must not spread to everything:\n{clean}");
}

#[test]
fn releasing_does_not_change_the_answer() {
    let dir = std::env::temp_dir().join("vibe-escape-tests");
    let exe = dir.join("churn");
    emit_c("examples/churn.vibe", "churn");
    let o = Command::new(&exe).output().expect("the built program runs");
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "10000000");
}

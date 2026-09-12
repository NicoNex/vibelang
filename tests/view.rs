//! The canonical form is a test, not a claim: `vibe view` prints from the AST
//! only, so its output must come back byte-identical for a canonical file.
//! ponytail: the examples are the fixtures.

use std::process::Command;

const VIBE: &str = env!("CARGO_BIN_EXE_vibe");

fn view(args: &[&str]) -> String {
    let o = Command::new(VIBE)
        .arg("view")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "vibe view {args:?} failed:\n{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn source(path: &str) -> String {
    std::fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR"))).expect("example exists")
}

#[test]
fn canonical_form_round_trips() {
    for e in ["examples/hello.vibe", "examples/ledger.vibe"] {
        assert_eq!(view(&[e]), source(e), "{e} is not reproduced byte for byte");
    }
}

#[test]
fn sig_only_drops_bodies() {
    let out = view(&["examples/ledger.vibe", "--sig-only"]);
    assert!(out.contains("mean (ts:&Vec Tx, len ts>0) : F64\n"), "missing signature:\n{out}");
    assert!(!out.contains("total ts /"), "bodies must not be printed:\n{out}");
    // Types, externs and exports are signatures: they survive.
    assert!(out.contains("type Err = Bad Str | Num Str | Void"), "missing type:\n{out}");
    assert!(out.contains("exp c mean, total"), "missing exports:\n{out}");
}

#[test]
fn flow_expands_pipelines() {
    let out = view(&["examples/ledger.vibe", "--flow"]);
    assert!(!out.contains("|>"), "no pipeline may survive:\n{out}");
    assert!(out.contains("let p1 = map amt ts in\n"), "missing named stage:\n{out}");
}

#[test]
fn explicit_shows_inferred_types() {
    let out = view(&["examples/ledger.vibe", "--explicit"]);
    assert!(out.contains(";; Ledger.total : (Vec Tx) -> F64"), "missing inferred type:\n{out}");
    assert!(out.contains(";; |- len ts>0 [checked at run time]"), "missing refinement:\n{out}");
}

/// Every `.vibe` file in the repository that is meant to parse is already in
/// canonical form, comments included. This is the property that makes `view` a
/// projection rather than a formatter: reading a file and printing it back is
/// the identity, so a structured edit is the only thing that changes a file.
#[test]
fn every_file_round_trips_byte_for_byte() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // these two exist to be rejected, so they never reach the projection
    let broken = ["bad.vibe", "blank.vibe"];
    let mut seen = 0;
    for dir in ["examples", "examples/shop", "tests"] {
        for e in std::fs::read_dir(root.join(dir)).expect("the directory exists") {
            let p = e.expect("readable entry").path();
            if p.extension().is_none_or(|x| x != "vibe") {
                continue;
            }
            let name = p.file_name().expect("a file name").to_string_lossy().to_string();
            if broken.contains(&name.as_str()) {
                continue;
            }
            let rel = format!("{dir}/{name}");
            assert_eq!(view(&[&rel]), source(&rel), "{rel} is not in canonical form");
            seen += 1;
        }
    }
    assert!(seen >= 10, "expected the whole corpus, walked {seen} files");
}

/// The lexer throws comments away, so the only reason they come back is that
/// `view` puts them back (§13.1).
#[test]
fn comments_survive_the_round_trip() {
    let out = view(&["tests/total_ok.vibe"]);
    assert!(out.contains(";; measure inferred: k decreases at the only recursive call"), "{out}");
    assert!(out.contains(";; not recursive: nothing to prove"), "{out}");
}

/// Comments are anchored to tokens, not to lines, so reformatting a file whose
/// layout is nothing like the canonical one keeps them. Losing them here would
/// be data loss with no error, which is the worst shape a bug can take.
#[test]
fn reformatting_a_ragged_file_keeps_its_comments() {
    let dir = std::env::temp_dir().join("vibe-view-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let f = dir.join("wide.vibe");
    std::fs::write(
        &f,
        "mod Wide\n\n;; keep me\nf (n:U64) : U64 =\n  n\n  +\n  1   ;; and me\n\ng (n:U64) : U64 = n\n",
    )
    .expect("write");
    let o = std::process::Command::new(VIBE)
        .args(["view", f.to_str().expect("utf-8")])
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    // the body collapses onto one line, so the two files do not agree on how
    // many lines there are — which is exactly the case that used to drop them
    assert!(out.contains("f (n:U64) : U64 = n + 1 ;; and me"), "{out}");
    assert!(out.contains(";; keep me"), "{out}");
}

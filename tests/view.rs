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

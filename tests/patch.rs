//! The agent-facing surface of §13.3: `deps`, `proof`, `patch`.

use std::path::PathBuf;
use std::process::Command;

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

/// A private copy of the reference program, so a patch test never edits the
/// example the other tests read. The copy keeps the name `ledger.vibe`: a file
/// name has to match its `mod` line (spec §9), which is what makes a qualified
/// name resolvable without a search path.
fn scratch(name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ledger.vibe");
    let dir = std::env::temp_dir().join("vibe-patch-tests").join(name);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let p = dir.join("ledger.vibe");
    std::fs::copy(&src, &p).expect("copy the reference program");
    p
}

fn node(path: &str, file: &str) -> (String, String) {
    let (ok, out) = vibe(&["patch", file, path]);
    assert!(ok, "reading `{path}` failed:\n{out}");
    let (head, text) = out.split_once('\n').expect("a header line and the node");
    let (p, h) = head.split_once('\t').expect("path and hash, tab separated");
    assert_eq!(p, path);
    (h.to_string(), text.trim_end().to_string())
}

#[test]
fn deps_reports_both_directions() {
    let (ok, out) = vibe(&["deps", "examples/ledger.vibe"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("Ledger.total -> Ledger.amt"),
        "callees missing:\n{out}"
    );
    assert!(
        out.contains("Ledger.amt <- Ledger.top Ledger.total"),
        "callers missing:\n{out}"
    );
    assert!(
        out.contains("Ledger.main <- -"),
        "a root must say so:\n{out}"
    );
}

#[test]
fn proof_addresses_obligations_by_semantic_path() {
    let (ok, out) = vibe(&["proof", "examples/ledger.vibe"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("Ledger.mean.body/0\tdiv0"),
        "path and code expected:\n{out}"
    );
}

#[test]
fn a_node_reads_back_with_its_hash() {
    let (h, text) = node("Ledger.amt.body", "examples/ledger.vibe");
    assert_eq!(text, "t.price * f64 t.qty");
    assert_eq!(h.len(), 16, "the hash is 16 hex digits: {h}");
    // the signature stops at the `=`, so the two nodes do not overlap
    let (_, sig) = node("Ledger.amt.sig", "examples/ledger.vibe");
    assert!(
        sig.ends_with(": F64"),
        "the separator belongs to neither node: {sig:?}"
    );
}

#[test]
fn a_multi_line_body_keeps_its_indentation() {
    let (_, text) = node("Ledger.parse.body", "examples/ledger.vibe");
    assert!(text.starts_with("\n  ?split ',' ln"), "{text:?}");
    assert!(
        text.ends_with("|_       -> Er (Bad (dup ln))\n  end"),
        "{text:?}"
    );
}

#[test]
fn a_stale_hash_is_refused() {
    let f = scratch("stale");
    let before = std::fs::read_to_string(&f).expect("read");
    let (ok, out) = vibe(&[
        "patch",
        f.to_str().expect("utf-8"),
        "Ledger.amt.body",
        "0",
        "1.0",
    ]);
    assert!(
        !ok,
        "a patch against a hash nobody has must not apply:\n{out}"
    );
    assert!(out.contains("stale patch"), "{out}");
    assert_eq!(
        std::fs::read_to_string(&f).expect("read"),
        before,
        "the file must be untouched"
    );
}

#[test]
fn a_patch_that_does_not_compile_is_refused() {
    let f = scratch("broken");
    let p = f.to_str().expect("utf-8");
    let before = std::fs::read_to_string(&f).expect("read");
    let (h, _) = node("Ledger.amt.body", p);
    let (ok, out) = vibe(&["patch", p, "Ledger.amt.body", &h, "no_such_fn t"]);
    assert!(!ok, "an unbound name must not reach the file:\n{out}");
    assert!(out.contains("patch refused"), "{out}");
    assert!(out.contains("name.unbound"), "{out}");
    assert_eq!(
        std::fs::read_to_string(&f).expect("read"),
        before,
        "the file must be untouched"
    );
}

#[test]
fn a_good_patch_applies_and_the_program_still_runs() {
    let f = scratch("good");
    let p = f.to_str().expect("utf-8");
    let (h, _) = node("Ledger.amt.body", p);
    let (ok, out) = vibe(&["patch", p, "Ledger.amt.body", &h, "f64 t.qty * t.price"]);
    assert!(ok, "{out}");
    let after = std::fs::read_to_string(&f).expect("read");
    assert!(after.contains("F64 = f64 t.qty * t.price"), "{after}");
    // everything the patch did not name is byte-identical
    let original = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ledger.vibe"),
    )
    .expect("read");
    assert_eq!(
        after
            .lines()
            .filter(|l| !l.starts_with("amt"))
            .collect::<Vec<_>>(),
        original
            .lines()
            .filter(|l| !l.starts_with("amt"))
            .collect::<Vec<_>>(),
        "a structured edit must produce a minimal textual diff"
    );
    let (ok, out) = vibe(&["check", p]);
    assert!(ok, "the patched file must still check:\n{out}");
}

#[test]
fn an_unknown_path_lists_what_exists() {
    let (ok, out) = vibe(&["patch", "examples/ledger.vibe", "Ledger.nope"]);
    assert!(!ok, "{out}");
    assert!(out.contains("no node at `Ledger.nope`"), "{out}");
    assert!(
        out.contains("Ledger.mean.body"),
        "the error must name the alternatives:\n{out}"
    );
}

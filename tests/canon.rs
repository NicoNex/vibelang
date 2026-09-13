//! Canonicity (spec §3.1). The rule is the projection: a file is canonical
//! exactly when printing it gives it back, so the fix is the line to write.

use std::path::PathBuf;
use std::process::Command;

fn check(name: &str, src: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join("vibe-canon-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let p: PathBuf = dir.join(format!("{name}.vibe"));
    std::fs::write(&p, src).expect("write");
    let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(["check", p.to_str().expect("utf-8"), "--diag=struct"])
        .output()
        .expect("vibe runs");
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.success(), s)
}

#[test]
fn spacing_around_a_binary_operator_is_not_free() {
    let (ok, out) = check("loose", "mod Loose\n\nf (a:U64) (b:U64, b != 0) : U64 = a/b\n");
    assert!(!ok, "two spellings of one program is exactly what P1 forbids:\n{out}");
    assert!(out.contains("canon.form"), "{out}");
    assert!(
        out.contains("f (a:U64) (b:U64, b!=0) : U64 = a / b"),
        "the fix must be the line to write, not advice:\n{out}"
    );
}

#[test]
fn a_block_body_that_fits_on_one_line_must_be_on_one_line() {
    let (ok, out) = check("wide", "mod Wide\n\nf (n:U64) : U64 =\n  n + 1\n");
    assert!(!ok, "{out}");
    assert!(out.contains("f (n:U64) : U64 = n + 1"), "{out}");
}

#[test]
fn a_canonical_file_passes() {
    let (ok, out) = check("tight", "mod Tight\n\nf (a:U64) (b:U64, b!=0) : U64 = a / b\n");
    assert!(ok, "the canonical spelling must be accepted:\n{out}");
}

/// The lexer's own rules fire earlier and say more, so they stay where they are.
#[test]
fn the_lexer_still_owns_the_rules_it_can_see_first() {
    let (ok, out) = check("blanks", "mod Blanks\n\n\n\nf (n:U64) : U64 = n\n");
    assert!(!ok, "{out}");
    assert!(out.contains("canon.blankline"), "{out}");
}

#[test]
fn redundant_parentheses_are_rejected() {
    let (ok, out) = check("parens", "mod Parens\n\nf (n:U64) : U64 = (n)\n");
    assert!(!ok, "{out}");
    assert!(out.contains("canon."), "some canonicity rule must catch it:\n{out}");
}

/// Every file the project ships is canonical, and `vibe check` is now what says
/// so — the round-trip test asserts the same property through `vibe view`.
#[test]
fn the_whole_corpus_checks() {
    for dir in ["examples", "examples/shop", "tests"] {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
        for e in std::fs::read_dir(&root).expect("the directory exists") {
            let p = e.expect("entry").path();
            if p.extension().is_none_or(|x| x != "vibe") {
                continue;
            }
            let name = p.file_name().expect("a name").to_string_lossy().to_string();
            // these exist to be rejected, for reasons of their own
            if ["bad.vibe", "blank.vibe", "move.vibe", "capture.vibe", "move_let.vibe",
                "move_pat.vibe", "refine_bad.vibe", "total_growing.vibe",
                "total_no_measure.vibe", "total_arena.vibe"]
                .contains(&name.as_str())
            {
                continue;
            }
            let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
                .args(["check", p.to_str().expect("utf-8")])
                .output()
                .expect("vibe runs");
            assert!(
                o.status.success(),
                "{name} does not check:\n{}",
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }
}

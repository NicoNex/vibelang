//! `vibe fmt` (§13.1): the canonical projection, written back to the file. The
//! property under test is that it agrees with `vibe view` and with itself, so a
//! formatted file is a fixed point and `--check` never disagrees with `fmt`.
//! Fixtures are written into a temp directory: a `.vibe` file under `tests/`
//! would join the corpus that `tests/view.rs` walks.

use std::path::{Path, PathBuf};
use std::process::Command;

const VIBE: &str = env!("CARGO_BIN_EXE_vibe");

/// Ragged on purpose: the body spans three lines and the module is padded, so
/// nothing about it is already canonical.
const RAGGED: &str = "mod Ragged\n\n\n;; keep me\nf (n:U64) : U64 =\n  n\n  +\n  1   ;; and me\n\ng (n:U64) : U64 = n\n";

fn fixture(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("vibe-fmt-tests").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let f = dir.join("ragged.vibe");
    std::fs::write(&f, body).expect("write the fixture");
    f
}

/// `(exit ok, stdout + stderr)`.
fn vibe(args: &[&str]) -> (bool, String) {
    let o = Command::new(VIBE).args(args).output().expect("vibe runs");
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.success(), s)
}

fn fmt(f: &Path, args: &[&str]) -> (bool, String) {
    let mut argv = vec!["fmt", f.to_str().expect("utf-8")];
    argv.extend_from_slice(args);
    vibe(&argv)
}

fn read(f: &Path) -> String {
    std::fs::read_to_string(f).expect("the fixture is still there")
}

#[test]
fn formats_a_ragged_file_in_place() {
    let f = fixture("ragged", RAGGED);
    let (ok, out) = fmt(&f, &[]);
    assert!(ok, "{out}");
    assert!(
        out.starts_with("formatted "),
        "no report of the write:\n{out}"
    );
    let got = read(&f);
    assert!(
        got.contains("f (n:U64) : U64 = n + 1 ;; and me\n"),
        "the body did not collapse:\n{got}"
    );
    assert!(got.contains(";; keep me\n"), "a comment was lost:\n{got}");
    assert!(!got.contains("\n\n\n"), "blank runs survived:\n{got}");
    // What `fmt` writes is what `vibe view` prints: one canonical form, not two.
    let o = Command::new(VIBE)
        .args(["view", f.to_str().expect("utf-8")])
        .output()
        .expect("vibe runs");
    assert_eq!(String::from_utf8_lossy(&o.stdout), got);
}

#[test]
fn formatting_is_idempotent() {
    let f = fixture("idempotent", RAGGED);
    assert!(fmt(&f, &[]).0);
    let once = read(&f);
    let (ok, out) = fmt(&f, &[]);
    assert!(ok, "{out}");
    assert_eq!(out, "", "a settled file must be formatted silently");
    assert_eq!(read(&f), once, "the second pass moved bytes");
}

#[test]
fn an_already_canonical_file_is_not_rewritten() {
    let f = fixture("untouched", RAGGED);
    assert!(fmt(&f, &[]).0);
    let before = std::fs::metadata(&f).expect("metadata").modified().ok();
    let body = read(&f);
    assert!(fmt(&f, &[]).0);
    assert_eq!(read(&f), body);
    // Not writing at all is the point: a formatter that rewrites identical
    // bytes churns mtimes and re-triggers every watcher downstream.
    assert_eq!(
        std::fs::metadata(&f).expect("metadata").modified().ok(),
        before,
        "the file was rewritten with its own contents"
    );
}

#[test]
fn check_refuses_ragged_input_without_touching_it() {
    let f = fixture("check-bad", RAGGED);
    let (ok, out) = fmt(&f, &["--check"]);
    assert!(!ok, "--check must fail on a file that is not canonical");
    assert!(
        out.contains("ragged.vibe"),
        "--check must name the file:\n{out}"
    );
    assert_eq!(read(&f), RAGGED, "--check wrote to the file");
}

#[test]
fn check_accepts_a_canonical_file() {
    let f = fixture("check-good", RAGGED);
    assert!(fmt(&f, &[]).0);
    let (ok, out) = fmt(&f, &["--check"]);
    assert!(ok, "--check must pass what `fmt` just wrote:\n{out}");
    assert_eq!(out, "");
}

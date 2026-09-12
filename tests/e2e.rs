//! One end-to-end check: every example must check, and the reference program
//! must produce the documented output. If codegen or the runtime breaks, this
//! fails. ponytail: no harness, no fixtures — the compiler is the fixture.

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

#[test]
fn examples_check() {
    for e in ["examples/hello.vibe", "examples/ledger.vibe"] {
        let (ok, out) = vibe(&["check", e]);
        assert!(ok, "{e} failed to check:\n{out}");
    }
}

#[test]
fn reference_program_runs() {
    let (ok, out) = vibe(&["run", "examples/ledger.vibe"]);
    assert!(ok, "ledger failed to build or run:\n{out}");
    assert_eq!(out.trim(), "n=3 tot=20.75 avg=6.91667 top=b", "unexpected output");
}

#[test]
fn bad_program_reports_a_diagnostic() {
    let (ok, out) = vibe(&["check", "tests/bad.vibe", "--diag=struct"]);
    assert!(!ok, "a type error must fail the check");
    assert!(out.contains("type.mismatch"), "expected type.mismatch, got:\n{out}");
}

#[test]
fn c_ffi_calls_libc() {
    let (ok, out) = vibe(&["run", "examples/ffi.vibe"]);
    assert!(ok, "ext c example failed:\n{out}");
    assert!(out.contains("through libc"), "libc puts must run:\n{out}");
    // C says only that `puts` returns a nonnegative value on success; the exact
    // number is the libc's business (glibc gives the byte count, macOS does not)
    let n: i32 = out
        .split("puts returned ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| panic!("its result must come back:\n{out}"));
    assert!(n >= 0, "puts reported failure: {n}\n{out}");
}

#[test]
fn exported_functions_get_a_header() {
    let (ok, out) = vibe(&["build", "examples/ledger.vibe"]);
    assert!(ok, "build failed:\n{out}");
    let h = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/.vibe-ledger/ledger.h"),
    )
    .expect("a header is generated for `exp c`");
    assert!(h.contains("double Ledger_mean(VbVal x0);"), "mean must be exported:\n{h}");
    assert!(h.contains("NOT VERIFIED ACROSS THE BOUNDARY"), "the precondition must be flagged:\n{h}");
}

#[test]
fn json_diagnostics_are_json() {
    let (ok, out) = vibe(&["check", "tests/bad.vibe", "--diag=json"]);
    assert!(!ok);
    assert!(out.trim_start().starts_with("[{"), "expected a JSON array, got:\n{out}");
    assert!(out.contains("\"code\""), "expected a code field, got:\n{out}");
}

#[test]
fn emit_c_keeps_the_generated_source() {
    let (ok, out) = vibe(&["build", "examples/hello.vibe", "--emit-c"]);
    assert!(ok, "build --emit-c failed:\n{out}");
    let c = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello.c");
    assert!(c.exists(), "--emit-c must leave the C next to the output");
    std::fs::remove_file(c).ok();
}

#[test]
fn two_blank_lines_are_not_canonical() {
    let (ok, out) = vibe(&["check", "tests/blank.vibe", "--diag=struct"]);
    assert!(!ok, "two blank lines in a row must be rejected");
    assert!(out.contains("canon.blankline"), "expected canon.blankline, got:\n{out}");
}

/// `--lib` is only worth anything if a C compiler can actually consume what it
/// produces, so this test builds the archive and links a real C program
/// against it (spec §10.2).
#[test]
fn a_library_links_into_a_c_program() {
    let dir = std::env::temp_dir().join("vibe-lib-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let archive = dir.join("libmathlib.a");
    let (ok, out) = vibe(&[
        "build",
        "--lib",
        "examples/mathlib.vibe",
        "-o",
        archive.to_str().expect("utf-8"),
    ]);
    assert!(ok, "building a library failed:\n{out}");
    assert!(archive.exists(), "no archive at {}", archive.display());
    assert!(dir.join("mathlib.h").exists(), "the header must sit next to the archive");
    assert!(dir.join("vibert.h").exists(), "and so must the runtime header it includes");

    let c = dir.join("use.c");
    std::fs::write(
        &c,
        "#include <stdio.h>\n#include \"mathlib.h\"\n\
         int main(void) {\n  printf(\"%g %g\\n\", MathLib_area(3.0, 4.0), MathLib_perimeter(1.0, 2.0));\n  return 0;\n}\n",
    )
    .expect("write the C caller");
    let exe = dir.join("use");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let st = Command::new(&cc)
        .current_dir(&dir)
        .args(["-std=c11", "-o"])
        .arg(&exe)
        .arg(&c)
        .args(["-L.", "-lmathlib", "-lm"])
        .status()
        .expect("a C compiler");
    assert!(st.success(), "C could not link the library");
    let run = Command::new(&exe).output().expect("the linked program runs");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "12 6");
}

#[test]
fn a_library_with_no_exports_is_refused() {
    let (ok, out) = vibe(&["build", "--lib", "examples/hello.vibe"]);
    assert!(!ok, "a library with no `exp c` has no symbols:\n{out}");
    assert!(out.contains("no `exp c`"), "{out}");
}

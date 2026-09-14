//! Modules (spec §9): one file, one module, and a qualified name is the whole
//! import system. Everything here works on temporary directories, because what
//! is being tested is how files find each other.

use std::path::{Path, PathBuf};
use std::process::Command;

fn vibe_in(dir: &Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("vibe runs");
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.success(), s)
}

/// A fresh directory holding the given `(file name, contents)` pairs.
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join("vibe-module-tests").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    for (f, body) in files {
        std::fs::write(dir.join(f), body).expect("write a module");
    }
    dir
}

const MONEY: &str = "mod Money\n\ncents (euro:F64) : I64 = i64 (euro * 100.0)\n";

#[test]
fn a_qualified_name_loads_the_file_it_names() {
    let (ok, out) = vibe_in(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &["run", "examples/shop/app.vibe"],
    );
    assert!(ok, "a two-module program must run:\n{out}");
    assert_eq!(out.trim(), "gross=1220");
}

#[test]
fn a_missing_module_names_the_file_it_wanted() {
    let dir = project(
        "missing",
        &[("app.vibe", "mod App\n\nmain : E! Unit =\n  out (show (Money.cents 1.0))\n")],
    );
    let (ok, out) = vibe_in(&dir, &["check", "app.vibe", "--diag=struct"]);
    assert!(!ok, "{out}");
    assert!(out.contains("mod.missing"), "{out}");
    assert!(out.contains("Money.vibe"), "the error must name the file it looked for:\n{out}");
}

#[test]
fn a_module_must_match_its_file_name() {
    let dir = project("misnamed", &[("app.vibe", "mod Elsewhere\n\nmain : E! Unit =\n  out \"x\"\n")]);
    let (ok, out) = vibe_in(&dir, &["check", "app.vibe", "--diag=struct"]);
    assert!(!ok, "the mapping from a qualified name to a file must stay mechanical:\n{out}");
    assert!(out.contains("mod.name"), "{out}");
}

/// `total_ok.vibe` holds `mod TotalOk`: underscores and case are the only
/// difference the mapping tolerates.
#[test]
fn snake_case_files_hold_pascal_case_modules() {
    let dir = project(
        "snake",
        &[
            ("money_box.vibe", "mod MoneyBox\n\ntwice (n:I64) : I64 = n * 2\n"),
            ("app.vibe", "mod App\n\nmain : E! Unit =\n  out (show (MoneyBox.twice 21))\n"),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["run", "app.vibe"]);
    assert!(ok, "{out}");
    assert_eq!(out.trim(), "42");
}

#[test]
fn one_name_declared_twice_is_reported_not_shadowed() {
    let dir = project(
        "clash",
        &[
            ("money.vibe", MONEY),
            (
                "app.vibe",
                "mod App\n\ncents (n:F64) : I64 = 0\n\nmain : E! Unit =\n  out (show (Money.cents 1.0))\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["check", "app.vibe", "--diag=struct"]);
    assert!(!ok, "v0.1 has one flat namespace, so a clash must be an error:\n{out}");
    assert!(out.contains("mod.duplicate"), "{out}");
    assert!(out.contains("rename"), "the fix must be mechanical:\n{out}");
}

#[test]
fn a_cycle_terminates() {
    let dir = project(
        "cycle",
        &[
            ("a.vibe", "mod A\n\nup (n:I64) : I64 = B.down n\n"),
            ("b.vibe", "mod B\n\ndown (n:I64) : I64 = n - 1\n\nround (n:I64) : I64 = A.up n\n"),
        ],
    );
    // mutual reference is not mutual recursion: loading must not spin, whatever
    // the checker then makes of the program
    let (_, out) = vibe_in(&dir, &["check", "a.vibe"]);
    assert!(!out.contains("mod.missing"), "both modules must be found:\n{out}");
}

#[test]
fn a_diagnostic_names_the_module_that_owns_the_code() {
    let dir = project(
        "paths",
        &[
            ("money.vibe", "mod Money\n\nhalf (n:I64) (d:I64) : I64 = n / d\n"),
            ("app.vibe", "mod App\n\nmain : E! Unit =\n  out (show (Money.half 4 2))\n"),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["proof", "app.vibe"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("Money.half.body/0"),
        "an obligation from an imported module must keep that module's path:\n{out}"
    );
}

/// A projection shows the program as written: resolving `Money.cents` to
/// `cents` in the output would make `vibe view` an editor.
#[test]
fn a_projection_keeps_the_qualifier() {
    let (ok, out) =
        vibe_in(Path::new(env!("CARGO_MANIFEST_DIR")), &["view", "examples/shop/app.vibe"]);
    assert!(ok, "{out}");
    assert!(out.contains("Money.vat (Money.cents euro)"), "{out}");
}

/// Fields resolve to their record by name alone, so two modules spelling one
/// the same way must be caught here rather than surfacing as a type error in a
/// third place.
#[test]
fn a_record_field_declared_twice_is_a_clash_too() {
    let dir = project(
        "fields",
        &[
            ("a.vibe", "mod A\n\ntype Ta = { qty:U32 }\n\nmk : Ta = {qty=1}\n"),
            (
                "b.vibe",
                "mod B\n\ntype Tb = { qty:F64 }\n\nmain : E! Unit =\n  out (show (A.mk.qty))\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["check", "b.vibe", "--diag=struct"]);
    assert!(!ok, "{out}");
    assert!(out.contains("mod.duplicate"), "{out}");
    assert!(out.contains("`qty` is declared in both"), "{out}");
}

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

/// Same, with `VIBE_PATH` set: where a shipped library would live.
fn vibe_with_path(dir: &Path, path: &Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(args)
        .current_dir(dir)
        .env("VIBE_PATH", path)
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
        &[(
            "app.vibe",
            "mod App\n\nmain : E! Unit =\n  out (show (Money.cents 1.0))\n",
        )],
    );
    let (ok, out) = vibe_in(&dir, &["check", "app.vibe", "--diag=struct"]);
    assert!(!ok, "{out}");
    assert!(out.contains("mod.missing"), "{out}");
    assert!(
        out.contains("Money.vibe"),
        "the error must name the file it looked for:\n{out}"
    );
}

#[test]
fn a_module_must_match_its_file_name() {
    let dir = project(
        "misnamed",
        &[(
            "app.vibe",
            "mod Elsewhere\n\nmain : E! Unit =\n  out \"x\"\n",
        )],
    );
    let (ok, out) = vibe_in(&dir, &["check", "app.vibe", "--diag=struct"]);
    assert!(
        !ok,
        "the mapping from a qualified name to a file must stay mechanical:\n{out}"
    );
    assert!(out.contains("mod.name"), "{out}");
}

/// `total_ok.vibe` holds `mod TotalOk`: underscores and case are the only
/// difference the mapping tolerates.
#[test]
fn snake_case_files_hold_pascal_case_modules() {
    let dir = project(
        "snake",
        &[
            (
                "money_box.vibe",
                "mod MoneyBox\n\ntwice (n:I64) : I64 = n * 2\n",
            ),
            (
                "app.vibe",
                "mod App\n\nmain : E! Unit =\n  out (show (MoneyBox.twice 21))\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["run", "app.vibe"]);
    assert!(ok, "{out}");
    assert_eq!(out.trim(), "42");
}

/// Two modules may declare the same name. The qualified spelling is what the
/// flattened program uses, so `Money.cents` and `App.cents` are two names and
/// neither shadows the other — and a bare `cents` inside `App` means App's.
#[test]
fn the_same_name_in_two_modules_is_two_names() {
    let dir = project(
        "clash",
        &[
            ("money.vibe", MONEY),
            (
                "app.vibe",
                "mod App\n\ncents (n:I64) : I64 = n + 1\n\nmain : E! Unit =\n  \
                 out (show (Money.cents 1.0 + cents 5))\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["run", "app.vibe"]);
    assert!(ok, "same name, different modules, no clash:\n{out}");
    assert_eq!(out.trim(), "106", "100 from Money, 6 from App");
}

/// A module may declare a name the prelude already has. Inside that module the
/// declaration wins; every other module still gets the prelude's.
#[test]
fn a_module_may_shadow_a_prelude_name() {
    let dir = project(
        "shadow",
        &[(
            "app.vibe",
            "mod App\n\ntake (n:I64) : I64 = n * 2\n\nmain : E! Unit =\n  \
             out (show (take 21))\n",
        )],
    );
    let (ok, out) = vibe_in(&dir, &["run", "app.vibe"]);
    assert!(ok, "`take` is the prelude's name, not its property:\n{out}");
    assert_eq!(out.trim(), "42");
}

/// A qualified name that the module does not declare is caught here, by name,
/// rather than as an unbound-name error somewhere downstream.
#[test]
fn a_qualified_name_that_is_not_there_says_so() {
    let dir = project(
        "typo",
        &[
            ("money.vibe", MONEY),
            (
                "app.vibe",
                "mod App\n\nmain : E! Unit =\n  out (show (Money.centz 1.0))\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["check", "app.vibe", "--diag=struct"]);
    assert!(!ok, "{out}");
    assert!(out.contains("mod.no_name"), "{out}");
    assert!(out.contains("centz"), "the error must name it:\n{out}");
}

#[test]
fn a_cycle_terminates() {
    let dir = project(
        "cycle",
        &[
            ("a.vibe", "mod A\n\nup (n:I64) : I64 = B.down n\n"),
            (
                "b.vibe",
                "mod B\n\ndown (n:I64) : I64 = n - 1\n\nround (n:I64) : I64 = A.up n\n",
            ),
        ],
    );
    // mutual reference is not mutual recursion: loading must not spin, whatever
    // the checker then makes of the program
    let (_, out) = vibe_in(&dir, &["check", "a.vibe"]);
    assert!(
        !out.contains("mod.missing"),
        "both modules must be found:\n{out}"
    );
}

#[test]
fn a_diagnostic_names_the_module_that_owns_the_code() {
    let dir = project(
        "paths",
        &[
            (
                "money.vibe",
                "mod Money\n\nhalf (n:I64) (d:I64) : I64 = n / d\n",
            ),
            (
                "app.vibe",
                "mod App\n\nmain : E! Unit =\n  out (show (Money.half 4 2))\n",
            ),
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
    let (ok, out) = vibe_in(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &["view", "examples/shop/app.vibe"],
    );
    assert!(ok, "{out}");
    assert!(out.contains("Money.vat (Money.cents euro)"), "{out}");
}

/// Two modules may both declare a field `qty`. Inference resolves each use site
/// from the base's type and writes the answer down (`Checked::field_of`), so
/// codegen no longer picks a record by the bare field name.
///
/// `qty` sits at a different index in each record here on purpose: resolving it
/// the old way emitted `vb_field(x, 0)` and this printed `x` instead of 7.
#[test]
fn two_modules_may_declare_the_same_field() {
    let dir = project(
        "fields",
        &[
            (
                "a.vibe",
                "mod A\n\ntype Ta = { note:Str, qty:U32 }\n\nmk (n:U32) : Ta = {note=\"x\", qty=n}\n",
            ),
            (
                "b.vibe",
                "mod B\n\ntype Tb = { qty:F64, extra:U32 }\n\nmain : E! Unit =\n  out (show (A.mk 7).qty)\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["run", "b.vibe"]);
    assert!(ok, "two modules may name a field the same way:\n{out}");
    assert_eq!(
        out.trim(),
        "7",
        "the field resolved to the wrong record:\n{out}"
    );
}

/// An `ext c` symbol is C's, so two modules naming it are naming one function
/// and that is not a clash. Disagreeing about its type is: the linker takes one
/// and the checker proved something about the other.
#[test]
fn two_modules_may_declare_the_same_c_symbol_with_one_type() {
    let dir = project(
        "csym",
        &[
            (
                "a.vibe",
                "mod A\n\next c \"stdio.h\"\n  puts : &CStr -> E! I32\nend\n\nsay (s:&Str) : E! I32 = puts &(to_cstr s)\n",
            ),
            (
                "b.vibe",
                "mod B\n\next c \"stdio.h\"\n  puts : &CStr -> E! I32\nend\n\nmain : E! Unit =\n  n <- A.say \"hi\" ;\n  out (show n)\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["check", "b.vibe"]);
    assert!(ok, "one C symbol, one type, two modules:\n{out}");
}

#[test]
fn two_modules_may_not_give_one_c_symbol_two_types() {
    let dir = project(
        "csym2",
        &[
            (
                "a.vibe",
                "mod A\n\next c \"stdio.h\"\n  puts : &CStr -> E! I32\nend\n\nsay (s:&Str) : E! I32 = puts &(to_cstr s)\n",
            ),
            (
                "b.vibe",
                "mod B\n\next c \"stdio.h\"\n  puts : &CStr -> E! U64\nend\n\nmain : E! Unit =\n  n <- A.say \"hi\" ;\n  out (show n)\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["check", "b.vibe", "--diag=struct"]);
    assert!(!ok, "{out}");
    assert!(out.contains("mod.duplicate"), "{out}");
    assert!(out.contains("two different types"), "{out}");
}

/// A module does not have to sit next to the file that names it: `VIBE_PATH` is
/// where a library lives, and the name still maps to the file mechanically.
#[test]
fn a_module_is_found_on_the_search_path() {
    let lib = project(
        "lib",
        &[(
            "greet.vibe",
            "mod Greet\n\nhi (n:&Str) : Str = concat \"hi \" n\n",
        )],
    );
    let app = project(
        "path_app",
        &[(
            "prog.vibe",
            "mod Prog\n\nmain : E! Unit =\n  out (Greet.hi \"there\")\n",
        )],
    );
    let (ok, out) = vibe_with_path(&app, &lib, &["run", "prog.vibe"]);
    assert!(ok, "VIBE_PATH must be searched:\n{out}");
    assert_eq!(out.trim(), "hi there");

    // and without it, the error says every place it looked
    let (ok, out) = vibe_in(&app, &["check", "prog.vibe"]);
    assert!(!ok, "{out}");
    assert!(out.contains("looked in"), "{out}");
}

/// A type and a constructor can be qualified too, and a module named only that
/// way still has to be loaded.
#[test]
fn a_qualified_type_loads_its_module() {
    let dir = project(
        "qual_ty",
        &[
            ("shape.vibe", "mod Shape\n\ntype Kind = Dot | Line F64\n"),
            (
                "app.vibe",
                "mod App\n\nflat (k:&Shape.Kind) : F64 =\n  \
                 ?k |Shape.Dot    -> 0.0\n     \
                    |Shape.Line n -> n\n  end\n\n\
                 main : E! Unit = out (show (flat &(Shape.Line 3.0)))\n",
            ),
        ],
    );
    let (ok, out) = vibe_in(&dir, &["run", "app.vibe"]);
    assert!(ok, "a module named only by a type and a pattern:\n{out}");
    assert_eq!(out.trim(), "3");
}

/// The installed standard library lives under the user's home, like `~/.cargo`,
/// so installing one needs no root: `~/.vibe/lib` is on the search path.
#[test]
fn a_module_is_found_under_the_home_directory() {
    let home = project("home", &[]);
    let lib = home.join(".vibe").join("lib");
    std::fs::create_dir_all(&lib).expect("a home library");
    std::fs::write(
        lib.join("greet.vibe"),
        "mod Greet\n\nhi (n:&Str) : Str = concat \"hi \" n\n",
    )
    .expect("write a module");
    let app = project(
        "home_app",
        &[(
            "prog.vibe",
            "mod Prog\n\nmain : E! Unit =\n  out (Greet.hi \"there\")\n",
        )],
    );
    let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(["run", "prog.vibe"])
        .current_dir(&app)
        .env("HOME", &home)
        .env_remove("VIBE_PATH")
        .output()
        .expect("vibe runs");
    let mut out = String::from_utf8_lossy(&o.stdout).to_string();
    out.push_str(&String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "~/.vibe/lib must be searched:\n{out}");
    assert_eq!(out.trim(), "hi there");
}

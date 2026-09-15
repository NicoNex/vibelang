//! Layout (spec §3.1). Whitespace carries no meaning: the same program written
//! with any indentation, or none, must compile to the same thing. A generator
//! that miscounts spaces should not pay for a retry.

use std::path::PathBuf;
use std::process::Command;

fn path(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("vibe-canon-tests")
        .join(format!("{name}.vibe"))
}

/// The canonical projection of a file written earlier by `check`.
fn view(name: &str) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(["view", path(name).to_str().expect("utf-8")])
        .output()
        .expect("vibe runs");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn check(name: &str, src: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join("vibe-canon-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let p: PathBuf = path(name);
    std::fs::write(&p, src).expect("write");
    let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(["check", p.to_str().expect("utf-8"), "--diag=struct"])
        .output()
        .expect("vibe runs");
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.success(), s)
}

/// The same program, written two ways no formatter would produce. Both must
/// compile, and both must project to the same canonical text — which is the
/// operational meaning of "whitespace does not reach the AST".
#[test]
fn indentation_is_free() {
    // one module name, two files, so only the layout differs
    let tidy = "mod Tidy\n\nf (n:U64) : U64 =\n  ?n |0 -> 1\n     |_ -> n\n  end\n";
    let ragged = "mod Ragged\n\n      f (n:U64) : U64 =\n?n\n|0 -> 1\n            |_ -> n\nend\n";
    let (ok, out) = check("tidy", tidy);
    assert!(ok, "{out}");
    let (ok, out) = check("ragged", ragged);
    assert!(
        ok,
        "indentation must not be able to break a program:\n{out}"
    );
    assert_eq!(
        view("tidy").replace("Tidy", "M"),
        view("ragged").replace("Ragged", "M"),
        "the two must project to the same program"
    );
}

#[test]
fn blank_lines_are_free() {
    let src = "mod Blanks\n\n\n\n\nf (n:U64) : U64 = n\n\n\ng (n:U64) : U64 = n\n";
    let (ok, out) = check("blanks", src);
    assert!(
        ok,
        "a run of blank lines is whitespace, not an error:\n{out}"
    );
}

/// A newline still ends a declaration, and that is the only layout rule left.
/// Without it juxtaposition swallows the next declaration's name.
#[test]
fn a_newline_still_separates_declarations() {
    let (ok, out) = check("joined", "mod Joined\n\nf : U64 = 1 g : U64 = 2\n");
    assert!(
        !ok,
        "`1 g` is an application, and that is the ambiguity:\n{out}"
    );
}

/// A newline inside a declaration is a continuation wherever the line so far
/// is not yet an expression, and inside anything `end` or a bracket closes.
#[test]
fn a_newline_inside_a_declaration_is_whitespace() {
    let src = "mod Split\n\nf (n:U64) : U64 =\n  n\n  +\n  1\n";
    let (ok, out) = check("split", src);
    assert!(
        ok,
        "a line break around an operator must not end the declaration:\n{out}"
    );
}

#[test]
fn redundant_parentheses_are_still_rejected() {
    let (ok, out) = check("parens", "mod Parens\n\nf (n:U64) : U64 = (n)\n");
    assert!(
        !ok,
        "structure is still canonical even though layout is not:\n{out}"
    );
    assert!(out.contains("canon."), "{out}");
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
            if [
                "bad.vibe",
                "move.vibe",
                "move_vec.vibe",
                "with_alias.vibe",
                "capture.vibe",
                "move_let.vibe",
                "move_pat.vibe",
                "refine_bad.vibe",
                "total_growing.vibe",
                "total_no_measure.vibe",
                "total_arena.vibe",
                "diverge.vibe",
                "ffi_bad.vibe",
            ]
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

/// `c` is a keyword after `ext` and `exp` and nowhere else: §2.2 lists the
/// complete keyword set and does not claim it, and §2.3's identifier grammar
/// allows any lower-case word.
#[test]
fn c_is_a_name_everywhere_but_after_ext_and_exp() {
    let o = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(["check", "tests/ident_c.vibe"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vibe runs");
    assert!(
        o.status.success(),
        "a file that both declares `ext c` and binds `c`:\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// `;` sequences two effectful expressions. It is the same construct as `<-`
/// with the name left off, so the effect checker needs no second rule.
#[test]
fn a_semicolon_sequences_without_binding() {
    let src = "mod Seq\n\nmain : E! Unit =\n  out \"a\\n\" ;\n  out \"b\\n\"\n";
    let (ok, out) = check("seq", src);
    assert!(ok, "{out}");
}

/// `;;` is a comment and `;` is the sequencer. The lexer takes the longer one
/// first, so a trailing comment after a `;` is still a comment.
#[test]
fn a_double_semicolon_is_still_a_comment() {
    let src = "mod Comment\n\nmain : E! Unit =\n  out \"a\\n\" ;   ;; and then\n  out \"b\\n\"\n";
    let (ok, out) = check("comment", src);
    assert!(ok, "{out}");
}

/// Nested matches are why `end` exists: without it the inner arms and the outer
/// ones would have to be told apart by column.
#[test]
fn end_closes_the_inner_match_before_the_outer_one() {
    let src = "mod Nest\n\nf (a:U64) (b:U64) : U64 =\n\
               ?a |0 -> ?b |0 -> 1 |_ -> 2 end\n\
                  |_ -> 3\n\
               end\n";
    let (ok, out) = check("nest", src);
    assert!(ok, "{out}");
    assert!(
        view("nest").contains("end"),
        "the projection must print it back"
    );
}

#[test]
fn a_match_without_end_says_so() {
    let src = "mod Unclosed\n\nf (n:U64) : U64 = ?n |0 -> 1 |_ -> 2\n";
    let (ok, out) = check("unclosed", src);
    assert!(!ok, "{out}");
    assert!(out.contains("parse.match"), "{out}");
    assert!(
        out.contains("add `end`"),
        "the fix must be mechanical:\n{out}"
    );
}

#[test]
fn an_ext_block_without_end_says_so() {
    let src = "mod Unended\n\next c \"stdio.h\"\n  puts : &CStr -> E! I32\n";
    let (ok, out) = check("unended", src);
    assert!(!ok, "{out}");
    assert!(out.contains("parse.ext"), "{out}");
}

#[test]
fn a_bind_without_a_semicolon_says_so() {
    let src = "mod NoSemi\n\nmain : E! Unit =\n  s <- read \"f\"\n  out &s\n";
    let (ok, out) = check("nosemi", src);
    assert!(!ok, "{out}");
    assert!(out.contains("parse.bind"), "{out}");
    assert!(out.contains("`;`"), "{out}");
}

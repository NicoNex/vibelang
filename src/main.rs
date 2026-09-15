//! Driver: .vibe -> C -> native executable.
//!
//! vibe check <file> [--diag=prose|struct|json]
//! vibe build <file> [-o out] [--lib] [--emit-c] [--diag=...]
//! vibe run   <file> [--diag=...] [-- args...]
//! vibe view  <file> [--sig-only|--explicit|--flow]
//! vibe deps  <file>
//! vibe proof <file> [--prove]
//! vibe patch <file> <path> [<hash> <new-node>]

// A `Diag` is a few hundred bytes and every fallible function returns one. The
// error path of a compiler is cold by definition, so paying an allocation to
// shrink it would buy nothing and cost a `Box` at every construction site.
#![allow(clippy::result_large_err)]

mod ast;
mod codegen;
mod diag;
mod escape;
mod infer;
mod lexer;
mod load;
mod own;
mod parser;
mod patch;
mod refine;
mod total;
mod types;
mod view;

use diag::{Diag, DiagFormat, Files};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const RT_C: &str = include_str!("../runtime/vibert.c");
const RT_H: &str = include_str!("../runtime/vibert.h");

const USAGE: &str = "\
usage: vibe <check|build|run|view|deps|proof|patch> <file.vibe> [options]
  patch <file> <path>                 print the node's hash and current text
  patch <file> <path> <hash> <node>   replace it, if the hash still matches
  --diag=prose|struct|json   diagnostic rendering (default: prose)
  --prove                    discharge refinement obligations with z3 (§7.3)
  --prove-timeout=<secs>     solver budget per obligation (default 5)
  --sig-only|--explicit|--flow   projection to print (view; default: canonical)
  -o <path>                  output executable (build)
  --lib                      build a static library plus its C header (build)
  --emit-c                   also keep the generated C next to the output
  -- <args...>               arguments passed to the program (run)
";

/// Driver failures get a `vibe:` prefix; diagnostics are printed verbatim, so
/// `--diag=json` output stays machine-readable.
enum Fail {
    Diags(String),
    Driver(String),
}

impl From<String> for Fail {
    fn from(s: String) -> Fail {
        Fail::Driver(s)
    }
}

impl From<&str> for Fail {
    fn from(s: &str) -> Fail {
        Fail::Driver(s.to_string())
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(code) => code,
        Err(Fail::Diags(s)) => {
            eprintln!("{s}");
            ExitCode::FAILURE
        }
        Err(Fail::Driver(e)) => {
            eprintln!("vibe: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Opts {
    cmd: String,
    file: PathBuf,
    out: Option<PathBuf>,
    fmt: DiagFormat,
    emit_c: bool,
    lib: bool,
    prove: bool,
    view: view::Mode,
    prog_args: Vec<String>,
    /// positionals after the file: the `patch` path, hash and new node
    rest: Vec<String>,
}

fn parse_args(argv: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        cmd: String::new(),
        file: PathBuf::new(),
        out: None,
        fmt: DiagFormat::Prose,
        emit_c: false,
        lib: false,
        prove: false,
        view: view::Mode::Canon,
        prog_args: Vec::new(),
        rest: Vec::new(),
    };
    let mut i = 0;
    let mut positional: Vec<String> = Vec::new();
    while i < argv.len() {
        let a = &argv[i];
        match a.as_str() {
            "--" => {
                o.prog_args = argv[i + 1..].to_vec();
                break;
            }
            "--emit-c" => o.emit_c = true,
            "--lib" => o.lib = true,
            "--prove" => o.prove = true,
            _ if a.starts_with("--prove-timeout=") => {
                let v = &a["--prove-timeout=".len()..];
                refine::set_budget(
                    v.parse()
                        .map_err(|_| format!("`{v}` is not a number of seconds"))?,
                );
            }
            "--sig-only" => o.view = view::Mode::SigOnly,
            "--explicit" => o.view = view::Mode::Explicit,
            "--flow" => o.view = view::Mode::Flow,
            "-h" | "--help" => return Err(USAGE.to_string()),
            "-o" => {
                i += 1;
                o.out = Some(PathBuf::from(argv.get(i).ok_or("-o needs a path")?));
            }
            _ if a.starts_with("--diag=") => {
                o.fmt = match &a["--diag=".len()..] {
                    "prose" => DiagFormat::Prose,
                    "struct" => DiagFormat::Struct,
                    "json" => DiagFormat::Json,
                    x => return Err(format!("unknown diagnostic format `{x}`")),
                }
            }
            _ if a.starts_with('-') => return Err(format!("unknown option `{a}`")),
            _ => positional.push(a.clone()),
        }
        i += 1;
    }
    if positional.len() < 2 {
        return Err(USAGE.to_string());
    }
    o.cmd = positional[0].clone();
    o.file = PathBuf::from(&positional[1]);
    o.rest = positional[2..].to_vec();
    if o.cmd != "patch" && !o.rest.is_empty() {
        return Err(format!("`{}` takes one file", o.cmd));
    }
    Ok(o)
}

fn run(argv: &[String]) -> Result<ExitCode, Fail> {
    let o = parse_args(argv)?;
    if !matches!(
        o.cmd.as_str(),
        "check" | "build" | "run" | "view" | "deps" | "proof" | "patch"
    ) {
        return Err(Fail::Driver(USAGE.to_string()));
    }

    let mut files = Files::new();
    // `Ledger.total` is the whole import system (§9): loading follows the
    // qualified names and hands the checker one flattened unit.
    let prog =
        load::program(&o.file, &mut files).map_err(|ds| Fail::Diags(diags(&ds, &files, o.fmt)))?;
    let root = prog.root();
    let comments = root.comments.clone();
    let root = root.module.clone();
    let module = prog.flat.clone();
    let checked = infer::check(&module).map_err(|ds| Fail::Diags(diags(&ds, &files, o.fmt)))?;
    // A projection is a reading tool: it works on code that does not yet prove.
    if o.cmd == "view" {
        let r = view::render(&root, &checked, o.view);
        // Only the canonical projection lines up with the file, so only it can
        // put the comments back; the others rewrite the program (§13.1).
        let r = match o.view {
            view::Mode::Canon => view::reattach(&r, &comments),
            _ => r,
        };
        print!("{r}");
        return Ok(ExitCode::SUCCESS);
    }
    if o.cmd == "deps" {
        print!("{}", patch::deps(&root));
        return Ok(ExitCode::SUCCESS);
    }
    if o.cmd == "proof" {
        print!("{}", proof(&module, &checked, o.prove));
        return Ok(ExitCode::SUCCESS);
    }
    if o.cmd == "patch" {
        return do_patch(&o, &prog, &files);
    }
    // P1 is the project's whole premise, so canonicity is checked on the same
    // footing as types (§3.1). The projection defines it: see `view::canon`.
    // Canonicity is per file, so every file that was read is checked, not just
    // the one that was named (§3.1).
    let mut semantic: Vec<Diag> = prog
        .units
        .iter()
        .flat_map(|u| view::canon(&u.module, &checked, &u.src, &u.comments, u.file))
        .collect();
    semantic.append(&mut own::check(&module, &checked));
    // The C boundary is a language rule, not a backend detail (§10.3).
    semantic.append(&mut codegen::boundary_errors(&module));
    semantic.append(&mut total::check(&module));
    // Refinements: proved on demand, counted on `check`, quiet on build/run so
    // the program's own output stays clean.
    let mode = match (o.prove, o.cmd.as_str()) {
        (true, _) => refine::Mode::Prove,
        (false, "check") => refine::Mode::Report,
        _ => refine::Mode::Silent,
    };
    let mut cache = refine::Cache::beside(&o.file);
    semantic.append(&mut refine::check(&module, &checked, mode, &mut cache));
    if !semantic.is_empty() {
        return Err(Fail::Diags(diags(&semantic, &files, o.fmt)));
    }
    if o.cmd == "check" {
        return Ok(ExitCode::SUCCESS);
    }

    let stem = o
        .file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or("out".into());
    let c_src = codegen::generate(&module, &checked, &stem)
        .map_err(|ds| Fail::Diags(diags(&ds, &files, o.fmt)))?;
    let exe = o.out.clone().unwrap_or_else(|| o.file.with_extension(""));

    // ponytail: build in a sibling directory, no temp-dir crate, no cleanup thread.
    let dir = exe
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".vibe-{stem}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    write(&dir.join("vibert.h"), RT_H)?;
    write(&dir.join("vibert.c"), RT_C)?;
    write(
        &dir.join(format!("{stem}.h")),
        &codegen::header(&module, &checked),
    )?;
    let c_path = dir.join(format!("{stem}.c"));
    write(&c_path, &c_src)?;
    if o.emit_c {
        write(&exe.with_extension("c"), &c_src)?;
    }

    if o.lib {
        archive(&o, &dir, &c_path, &stem, &module)?;
        return Ok(ExitCode::SUCCESS);
    }

    cc(&dir, &c_path, &exe, &module)?;

    if o.cmd == "run" {
        let st = Command::new(exe.canonicalize().unwrap_or(exe.clone()))
            .args(&o.prog_args)
            .status()
            .map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
        return Ok(ExitCode::from(st.code().unwrap_or(1).clamp(0, 255) as u8));
    }
    Ok(ExitCode::SUCCESS)
}

/// `vibe proof` (§13.3): the open obligations, one line each, addressed by the
/// same semantic path the diagnostics use. With `--prove` the ones z3 closes
/// are dropped, so what is left is exactly the work remaining.
fn proof(m: &ast::Module, ck: &infer::Checked, prove: bool) -> String {
    let (obs, skipped) = refine::obligations(m, ck);
    let mut out = refine::caveats(&obs, skipped);
    for o in &obs {
        if prove && refine::proved(o) {
            continue;
        }
        out.push_str(&format!("{}\t{}\t{}\n", o.path, o.code, o.msg));
    }
    out
}

/// `vibe patch` (§13.2). With no hash: report the node so the caller can name
/// it back. With one: replace the node, but only if the file still holds what
/// the caller last saw, and only if the result still compiles.
fn do_patch(o: &Opts, prog: &load::Program, files: &Files) -> Result<ExitCode, Fail> {
    // Every file the loader read is addressable, not just the one named on the
    // command line: `App.gross` and `Money.vat` are one program (§9), and a
    // semantic path already says which module it belongs to.
    let units: Vec<(&load::Unit, Vec<patch::Node>)> = prog
        .units
        .iter()
        .map(|u| (u, patch::nodes(&u.module, &u.src)))
        .collect();
    let path = o.rest.first().ok_or("patch needs a semantic path")?;
    let found = units
        .iter()
        .find_map(|(u, ns)| patch::find(ns, path).map(|n| (*u, n)));
    let Some((unit, node)) = found else {
        let known: Vec<&str> = units
            .iter()
            .flat_map(|(_, ns)| ns.iter().map(|n| n.path.as_str()))
            .collect();
        return Err(Fail::Driver(format!(
            "no node at `{path}`; this program has: {}",
            known.join(" ")
        )));
    };
    let src = unit.src.as_str();
    // The unit's file id is how the loader names the file it came from: the
    // edit lands there, not in the file that happened to be on the command
    // line.
    let file = Path::new(&files.names[unit.file]);
    let text = node.text(src);
    if o.rest.len() == 1 {
        print!("{}\t{}\n{text}\n", node.path, patch::hash(text));
        return Ok(ExitCode::SUCCESS);
    }
    let (want, new) = match (o.rest.get(1), o.rest.get(2)) {
        (Some(h), Some(n)) => (h, n),
        _ => {
            return Err(Fail::Driver(
                "patch needs both a hash and a new node".into(),
            ))
        }
    };
    let got = patch::hash(text);
    if *want != got {
        return Err(Fail::Driver(format!(
            "stale patch: `{path}` hashes {got}, not {want}; re-read the node and retry"
        )));
    }
    let patched = patch::apply(src, node, new);
    // The patch is not applied unless the result is a program: canonicity,
    // types and effects are all re-checked against the file that would be
    // written, and the original is left alone if any of them refuses.
    let mut fresh = Files::new();
    let fid = fresh.add(&files.names[unit.file], &patched);
    let verdict = lexer::lex(&patched, fid)
        .and_then(parser::parse)
        .map_err(|d| vec![d])
        .and_then(|pm| infer::check(&pm).map(|_| ()));
    if let Err(ds) = verdict {
        return Err(Fail::Diags(format!(
            "vibe: patch refused, `{}` is unchanged\n{}",
            file.display(),
            diags(&ds, &fresh, o.fmt)
        )));
    }
    write(file, &patched)?;
    println!("{path}\t{}", patch::hash(new.trim_end_matches('\n')));
    Ok(ExitCode::SUCCESS)
}

/// `vibe build --lib`: a static archive plus the generated header, so a C
/// project links a Vibelang module the way it links any other library (§10.2).
/// The export wrappers call `vb_init` themselves, so there is nothing for the
/// caller to initialise.
fn archive(o: &Opts, dir: &Path, c_path: &Path, stem: &str, m: &ast::Module) -> Result<(), String> {
    if m.exports().is_empty() {
        return Err(format!(
            "{} declares no `exp c`, so a library built from it would have no symbols",
            o.file.display()
        ));
    }
    let out = o
        .out
        .clone()
        .unwrap_or_else(|| o.file.with_file_name(format!("lib{stem}.a")));
    let outdir = out.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut objs = Vec::new();
    for (src, name) in [
        (c_path.to_path_buf(), stem),
        (dir.join("vibert.c"), "vibert"),
    ] {
        let obj = dir.join(format!("{name}.o"));
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
        let st = Command::new(&cc)
            .arg("-std=c11")
            .arg("-O2")
            .arg("-c")
            .arg(&src)
            .arg("-o")
            .arg(&obj)
            .arg(format!("-iquote{}", dir.display()))
            .status()
            .map_err(|e| format!("cannot run {cc}: {e}"))?;
        if !st.success() {
            return Err(format!("{cc} failed on {}", src.display()));
        }
        objs.push(obj);
    }
    let ar = std::env::var("AR").unwrap_or_else(|_| "ar".into());
    let st = Command::new(&ar)
        .arg("rcs")
        .arg(&out)
        .args(&objs)
        .status()
        .map_err(|e| format!("cannot run {ar}: {e}"))?;
    if !st.success() {
        return Err(format!("{ar} failed on {}", out.display()));
    }
    // The header is the point of the exercise: copy it, and the runtime header
    // it includes, next to the archive.
    for h in [format!("{stem}.h"), "vibert.h".to_string()] {
        let from = dir.join(&h);
        std::fs::copy(&from, outdir.join(&h))
            .map_err(|e| format!("cannot place {h} next to the archive: {e}"))?;
    }
    println!("{}", out.display());
    Ok(())
}

fn write(p: &Path, s: &str) -> Result<(), String> {
    std::fs::write(p, s).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

/// Compile the generated C plus the runtime, honouring `ext` link requirements.
fn cc(dir: &Path, c_path: &Path, exe: &Path, m: &ast::Module) -> Result<(), String> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = Command::new(&cc);
    cmd.arg("-std=c11")
        .arg("-O2")
        .arg("-o")
        .arg(exe)
        .arg(c_path)
        .arg(dir.join("vibert.c"));
    cmd.arg(format!("-iquote{}", dir.display()));

    let mut links: Vec<String> = Vec::new();
    let mut pkgs: Vec<String> = Vec::new();
    for d in &m.decls {
        if let ast::Decl::Ext(b) = d {
            links.extend(b.links.iter().cloned());
            pkgs.extend(b.pkgs.iter().cloned());
        }
    }
    if !pkgs.is_empty() {
        let out = Command::new("pkg-config")
            .arg("--cflags")
            .arg("--libs")
            .args(&pkgs)
            .output()
            .map_err(|e| format!("pkg-config failed: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "pkg-config: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        // ponytail: whitespace split, so a flag containing a space (`-I/opt/a b`)
        // breaks. Parse quoting if a real package ever needs it.
        cmd.args(
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .map(String::from),
        );
    }
    for l in &links {
        cmd.arg(format!("-l{l}"));
    }
    cmd.arg("-lm");

    let st = cmd.status().map_err(|e| format!("cannot run {cc}: {e}"))?;
    if !st.success() {
        return Err(format!("{cc} failed on generated C ({})", c_path.display()));
    }
    Ok(())
}

fn diags(ds: &[Diag], files: &Files, fmt: DiagFormat) -> String {
    let body: Vec<String> = ds.iter().map(|d| diag::render(d, files, fmt)).collect();
    match fmt {
        DiagFormat::Json => format!("[{}]", body.join(",")),
        _ => body.join("\n"),
    }
}

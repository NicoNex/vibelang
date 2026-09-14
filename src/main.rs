//! Driver: .vibe -> C -> native executable.
//!
//! vibe check <file> [--diag=prose|struct|json]
//! vibe build <file> [-o out] [--emit-c] [--diag=...]
//! vibe run   <file> [--diag=...] [-- args...]
//! vibe view  <file> [--sig-only|--explicit|--flow]

mod ast;
mod codegen;
mod diag;
mod infer;
mod lexer;
mod parser;
mod types;
mod view;

use diag::{DiagFormat, Diag, Files};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const RT_C: &str = include_str!("../runtime/vibert.c");
const RT_H: &str = include_str!("../runtime/vibert.h");

const USAGE: &str = "\
usage: vibe <check|build|run|view> <file.vibe> [options]
  --sig-only|--explicit|--flow  projection to print (view; default: canonical)
  --diag=prose|struct|json   diagnostic rendering (default: prose)
  -o <path>                  output executable (build)
  --emit-c                   also keep the generated C next to the output
  -- <args...>               arguments passed to the program (run)
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(code) => code,
        Err(e) => {
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
    prog_args: Vec<String>,
    view: view::Mode,
}

fn parse_args(argv: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        cmd: String::new(),
        file: PathBuf::new(),
        out: None,
        fmt: DiagFormat::Prose,
        emit_c: false,
        prog_args: Vec::new(),
        view: view::Mode::Canon,
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
    if positional.len() != 2 {
        return Err(USAGE.to_string());
    }
    o.cmd = positional[0].clone();
    o.file = PathBuf::from(&positional[1]);
    Ok(o)
}

fn run(argv: &[String]) -> Result<ExitCode, String> {
    let o = parse_args(argv)?;
    if !matches!(o.cmd.as_str(), "check" | "build" | "run" | "view") {
        return Err(USAGE.to_string());
    }

    let src = std::fs::read_to_string(&o.file)
        .map_err(|e| format!("cannot read {}: {e}", o.file.display()))?;
    let mut files = Files::new();
    let fid = files.add(&o.file.display().to_string(), &src);

    let toks = lexer::lex(&src, fid).map_err(|d| report(&[d], &files, o.fmt))?;
    let module = parser::parse(toks).map_err(|d| report(&[d], &files, o.fmt))?;
    let checked = infer::check(&module).map_err(|ds| report(&ds, &files, o.fmt))?;
    if o.cmd == "check" {
        return Ok(ExitCode::SUCCESS);
    }
    if o.cmd == "view" {
        print!("{}", view::render(&module, &checked, o.view));
        return Ok(ExitCode::SUCCESS);
    }

    let stem = o.file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("out".into());
    let c_src = codegen::generate(&module, &checked, &stem).map_err(|ds| report(&ds, &files, o.fmt))?;
    let exe = o.out.clone().unwrap_or_else(|| o.file.with_extension(""));

    // ponytail: build in a sibling directory, no temp-dir crate, no cleanup thread.
    let dir = exe.parent().unwrap_or(Path::new(".")).join(format!(".vibe-{stem}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    write(&dir.join("vibert.h"), RT_H)?;
    write(&dir.join("vibert.c"), RT_C)?;
    write(&dir.join(format!("{stem}.h")), &codegen::header(&module, &checked))?;
    let c_path = dir.join(format!("{stem}.c"));
    write(&c_path, &c_src)?;
    if o.emit_c {
        write(&exe.with_extension("c"), &c_src)?;
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

fn write(p: &Path, s: &str) -> Result<(), String> {
    std::fs::write(p, s).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

/// Compile the generated C plus the runtime, honouring `ext` link requirements.
fn cc(dir: &Path, c_path: &Path, exe: &Path, m: &ast::Module) -> Result<(), String> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = Command::new(&cc);
    cmd.arg("-std=c11").arg("-O2").arg("-o").arg(exe).arg(c_path).arg(dir.join("vibert.c"));
    cmd.arg(format!("-I{}", dir.display()));

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
            return Err(format!("pkg-config: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        cmd.args(String::from_utf8_lossy(&out.stdout).split_whitespace().map(String::from));
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

fn report(ds: &[Diag], files: &Files, fmt: DiagFormat) -> String {
    let body: Vec<String> = ds.iter().map(|d| diag::render(d, files, fmt)).collect();
    match fmt {
        DiagFormat::Json => format!("[{}]", body.join(",")),
        _ => body.join("\n"),
    }
}

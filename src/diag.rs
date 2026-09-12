//! Diagnostics. Two renderings of the same content (spec §12):
//! prose by default, structured terms with `--diag=struct`, JSON with `--diag=json`.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Span {
    pub file: usize,
    pub line: usize,
    pub col: usize,
    pub len: usize,
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub span: Span,
    /// Stable code, e.g. `type.mismatch`, `match.nonexhaustive`.
    pub code: String,
    pub msg: String,
    /// Semantic path (`Ledger.mean.body`), stable under insertion.
    pub path: Option<String>,
    /// Concrete counterexample / witness, when there is one.
    pub witness: Option<String>,
    /// Mechanical repair, when the repair is mechanical.
    pub fix: Option<String>,
}

impl Diag {
    pub fn error(span: Span, code: &str, msg: &str) -> Diag {
        Diag {
            span,
            code: code.into(),
            msg: msg.into(),
            path: None,
            witness: None,
            fix: None,
        }
    }
    pub fn with_fix(mut self, fix: &str) -> Diag {
        self.fix = Some(fix.into());
        self
    }
    pub fn with_witness(mut self, w: &str) -> Diag {
        self.witness = Some(w.into());
        self
    }
    pub fn with_path(mut self, p: &str) -> Diag {
        self.path = Some(p.into());
        self
    }
    pub fn at_path(mut self, p: &str) -> Diag {
        if self.path.is_none() {
            self.path = Some(p.into());
        }
        self
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum DiagFormat {
    Prose,
    Struct,
    Json,
}

pub struct Files {
    pub names: Vec<String>,
    pub texts: Vec<String>,
}

impl Files {
    pub fn new() -> Files {
        Files {
            names: Vec::new(),
            texts: Vec::new(),
        }
    }
    pub fn add(&mut self, name: &str, text: &str) -> usize {
        self.names.push(name.to_string());
        self.texts.push(text.to_string());
        self.names.len() - 1
    }
    pub fn line(&self, span: Span) -> String {
        self.texts
            .get(span.file)
            .and_then(|t| t.lines().nth(span.line.saturating_sub(1)))
            .unwrap_or("")
            .to_string()
    }
    pub fn name(&self, span: Span) -> &str {
        self.names
            .get(span.file)
            .map(|s| s.as_str())
            .unwrap_or("<input>")
    }
}

fn json_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

pub fn render(d: &Diag, files: &Files, fmt: DiagFormat) -> String {
    let path = d
        .path
        .clone()
        .unwrap_or_else(|| format!("{}:{}:{}", files.name(d.span), d.span.line, d.span.col + 1));
    match fmt {
        DiagFormat::Struct => {
            let mut s = format!("\u{2717} {} {}", path, d.code);
            if let Some(w) = &d.witness {
                s.push_str(&format!(" \u{22a8} {}", w));
            }
            s.push_str(&format!(
                "\n  at: {}:{}:{}",
                files.name(d.span),
                d.span.line,
                d.span.col + 1
            ));
            s.push_str(&format!("\n  msg: {}", d.msg));
            if let Some(f) = &d.fix {
                s.push_str(&format!("\n  fix: {}", f));
            }
            s
        }
        DiagFormat::Json => {
            let mut s = format!(
                "{{\"path\":\"{}\",\"code\":\"{}\",\"msg\":\"{}\",\"file\":\"{}\",\"line\":{},\"col\":{}",
                json_escape(&path),
                json_escape(&d.code),
                json_escape(&d.msg),
                json_escape(files.name(d.span)),
                d.span.line,
                d.span.col + 1
            );
            if let Some(w) = &d.witness {
                s.push_str(&format!(",\"witness\":\"{}\"", json_escape(w)));
            }
            if let Some(f) = &d.fix {
                s.push_str(&format!(",\"fix\":\"{}\"", json_escape(f)));
            }
            s.push('}');
            s
        }
        DiagFormat::Prose => {
            let src = files.line(d.span);
            let caret = format!(
                "{}{}",
                " ".repeat(d.span.col),
                "^".repeat(d.span.len.max(1))
            );
            let mut s = format!(
                "error[{}]: {}\n  --> {}:{}:{}\n   |\n   | {}\n   | {}",
                d.code,
                d.msg,
                files.name(d.span),
                d.span.line,
                d.span.col + 1,
                src,
                caret
            );
            if let Some(w) = &d.witness {
                s.push_str(&format!("\n   = counterexample: {}", w));
            }
            if let Some(f) = &d.fix {
                s.push_str(&format!("\n   = fix: {}", f));
            }
            s
        }
    }
}

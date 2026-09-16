//! Lexer for Vibelang. Whitespace carries no meaning.
//!
//! There is no offside rule, no INDENT/DEDENT, and no column tracking: a
//! generator that miscounts spaces should produce a program that still parses,
//! because a parse error costs a whole retry and retries are the metric this
//! language is optimised against.
//!
//! One layout rule survives, and it is not a counting rule: a newline ends a
//! top-level declaration. It fires only when the line so far is a complete
//! expression — the last token was a name, a literal, a closing bracket or
//! `end` — and only outside every bracketed or `end`-terminated construct.
//! Without it juxtaposition would swallow the next declaration's name, and
//! `f : U64 = 1` followed by `g : U64 = 2` would parse as `1 g`. Everywhere
//! else a newline is whitespace, so a continuation line may sit at any
//! indentation at all, including none.

use crate::diag::{Diag, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    Name(String), // lower-case identifier
    Ctor(String), // upper-case identifier
    Kw(&'static str),
    Sym(&'static str),
    Newline,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

impl Token {
    pub fn is_sym(&self, s: &str) -> bool {
        matches!(&self.tok, Tok::Sym(x) if *x == s)
    }
    pub fn is_kw(&self, s: &str) -> bool {
        matches!(&self.tok, Tok::Kw(x) if *x == s)
    }
}

const KEYWORDS: &[&str] = &[
    "mod", "ext", "exp", "type", "ghost", "let", "in", "own", "ref", "arena", "with", "end",
    "True", "False",
];

// Longest first: the matcher takes the first hit.
const SYMBOLS: &[&str] = &[
    "|>", "<-", "->", "==", "!=", "<=", ">=", "&&", "||", "++", "E!", ":", "=", "?", "|",
    "&", "%", "{", "}", "[", "]", "(", ")", ",", ".", "\\", "+", "-", "*", "/", "<", ">", "!", ";",
];

/// Whether a token can be the last one of an expression. A newline after
/// anything else is a continuation, never a terminator — which is what lets a
/// declaration be laid out however the generator felt like laying it out.
fn ends_expr(t: &Tok) -> bool {
    match t {
        Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::Char(_) | Tok::Name(_) | Tok::Ctor(_) => {
            true
        }
        Tok::Kw(k) => matches!(*k, "end" | "True" | "False"),
        Tok::Sym(s) => matches!(*s, ")" | "]" | "}"),
        _ => false,
    }
}

/// Whether a token can be the first one of an expression. A newline in front
/// of something that cannot start one is a continuation: it is what lets a line
/// break sit before a binary operator, before a `|` in a type declaration, or
/// before a `%` measure.
///
/// `end` counts as a starter so the newline in front of it survives, which is
/// what terminates the last signature of an `ext c` block.
///
/// `-` counts as a starter, because it is also unary negation and the lexer
/// cannot tell the two apart. So `a\n- b` ends the declaration while `a -\nb`
/// does not — the one case where where the break goes still matters.
fn starts_expr(t: &Tok) -> bool {
    match t {
        Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::Char(_) | Tok::Name(_) | Tok::Ctor(_) => {
            true
        }
        Tok::Kw(k) => !matches!(*k, "in" | "with"),
        Tok::Sym(s) => matches!(*s, "(" | "[" | "{" | "?" | "\\" | "&" | "!" | "-"),
        Tok::Eof => true,
        Tok::Newline => true,
    }
}

/// How a token changes the nesting depth. `?` opens a match, closed by `end`,
/// so a match needs no layout at all.
///
/// `ext c` is deliberately not on this list. Its signatures are `name : type`,
/// and a type extends greedily, so `puts : &CStr -> E! I32` followed by
/// `fputs : ...` would read `I32 fputs` as a type application. A newline
/// separates them, for the same reason one separates declarations.
fn nesting(t: &Tok) -> i32 {
    match t {
        Tok::Sym("(") | Tok::Sym("[") | Tok::Sym("{") | Tok::Sym("?") => 1,
        Tok::Sym(")") | Tok::Sym("]") | Tok::Sym("}") | Tok::Kw("end") => -1,
        _ => 0,
    }
}

pub struct Lexer<'a> {
    src: &'a [u8],
    file: usize,
    pos: usize,
    line: usize,
    line_start: usize,
    comments: Vec<Comment>,
}

/// A `;;` comment and where it sat. The lexer throws comments away — they carry
/// no meaning — but `vibe view` needs them back to round-trip a file, so it
/// keeps them on the side rather than in the token stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    /// How many real tokens precede it. Newlines do not count, so this index
    /// means the same thing in the file and in any re-rendering of it — which
    /// is what lets `vibe view` put the comment back without depending on
    /// either layout.
    pub after: usize,
    /// `true` when nothing but whitespace precedes it on its line.
    pub own_line: bool,
    /// The comment itself, `;;` included, newline excluded.
    pub text: String,
}

pub fn lex(src: &str, file: usize) -> Result<Vec<Token>, Diag> {
    lex_full(src, file).map(|(t, _)| t)
}

/// `lex`, plus the comments it discarded.
pub fn lex_full(src: &str, file: usize) -> Result<(Vec<Token>, Vec<Comment>), Diag> {
    let mut l = Lexer {
        src: src.as_bytes(),
        file,
        pos: 0,
        line: 1,
        line_start: 0,
        comments: Vec::new(),
    };
    let mut toks = l.run()?;
    // A newline in front of something that cannot begin an expression was never
    // a separator. Dropping those here, with one token of lookahead, is what
    // the emitting loop could not do with none.
    let mut keep = Vec::with_capacity(toks.len());
    for i in 0..toks.len() {
        if toks[i].tok == Tok::Newline && !toks.get(i + 1).is_some_and(|t| starts_expr(&t.tok)) {
            continue;
        }
        keep.push(toks[i].clone());
    }
    toks = keep;
    Ok((toks, l.comments))
}

impl<'a> Lexer<'a> {
    fn span(&self, start: usize) -> Span {
        Span {
            file: self.file,
            line: self.line,
            col: start - self.line_start,
            len: self.pos - start,
        }
    }
    fn peek(&self) -> u8 {
        *self.src.get(self.pos).unwrap_or(&0)
    }
    fn at(&self, off: usize) -> u8 {
        *self.src.get(self.pos + off).unwrap_or(&0)
    }

    fn run(&mut self) -> Result<Vec<Token>, Diag> {
        let mut out: Vec<Token> = Vec::new();
        let mut depth: i32 = 0;
        loop {
            // Horizontal whitespace and comments.
            loop {
                match self.peek() {
                    b' ' | b'\t' | b'\r' => self.pos += 1,
                    b';' if self.at(1) == b';' => {
                        let start = self.pos;
                        while self.peek() != b'\n' && self.pos < self.src.len() {
                            self.pos += 1;
                        }
                        self.comments.push(Comment {
                            after: out.iter().filter(|t| t.tok != Tok::Newline).count(),
                            own_line: self.src[self.line_start..start]
                                .iter()
                                .all(|b| b.is_ascii_whitespace()),
                            text: String::from_utf8_lossy(&self.src[start..self.pos])
                                .trim_end()
                                .to_string(),
                        });
                    }
                    _ => break,
                }
            }
            if self.pos >= self.src.len() {
                out.push(Token {
                    tok: Tok::Eof,
                    span: self.span(self.pos),
                });
                return Ok(out);
            }
            if self.peek() == b'\n' {
                // A newline separates declarations, and nothing else. It counts
                // only outside every bracket and every `end`, and only when the
                // tokens so far already form an expression; anywhere else it is
                // a continuation, so blank lines and stray indentation vanish
                // here rather than becoming errors later.
                let ends = out.last().is_some_and(|t| ends_expr(&t.tok));
                if depth == 0 && ends {
                    out.push(Token {
                        tok: Tok::Newline,
                        span: self.span(self.pos),
                    });
                }
                self.pos += 1;
                self.line += 1;
                self.line_start = self.pos;
                continue;
            }
            let start = self.pos;
            let c = self.peek();
            let tok = if c.is_ascii_digit() {
                self.number()?
            } else if c == b'"' {
                self.string()?
            } else if c == b'\'' {
                self.character()?
            } else if c.is_ascii_alphabetic() || c == b'_' {
                self.word()
            } else {
                self.symbol()?
            };
            let span = self.span(start);
            depth = (depth + nesting(&tok)).max(0);
            out.push(Token { tok, span });
        }
    }

    fn number(&mut self) -> Result<Tok, Diag> {
        let start = self.pos;
        while self.peek().is_ascii_digit() || self.peek() == b'_' {
            self.pos += 1;
        }
        let mut is_float = false;
        if self.peek() == b'.' && self.at(1).is_ascii_digit() {
            is_float = true;
            self.pos += 1;
            while self.peek().is_ascii_digit() || self.peek() == b'_' {
                self.pos += 1;
            }
        }
        let text: String = std::str::from_utf8(&self.src[start..self.pos])
            .unwrap()
            .chars()
            .filter(|c| *c != '_')
            .collect();
        if is_float {
            Ok(Tok::Float(text.parse().unwrap()))
        } else {
            text.parse::<i64>().map(Tok::Int).map_err(|_| {
                Diag::error(
                    self.span(start),
                    "lex.int",
                    "integer literal does not fit in 64 bits",
                )
            })
        }
    }

    fn escape(&mut self) -> Result<char, Diag> {
        let start = self.pos;
        self.pos += 1; // backslash
        let c = self.peek();
        self.pos += 1;
        Ok(match c {
            b'n' => '\n',
            b't' => '\t',
            b'r' => '\r',
            b'0' => '\0',
            b'\\' => '\\',
            b'"' => '"',
            b'\'' => '\'',
            _ => {
                return Err(Diag::error(
                    self.span(start),
                    "lex.escape",
                    &format!("unknown escape `\\{}`", c as char),
                ))
            }
        })
    }

    fn string(&mut self) -> Result<Tok, Diag> {
        let start = self.pos;
        self.pos += 1;
        let mut s = String::new();
        loop {
            match self.peek() {
                b'"' => {
                    self.pos += 1;
                    return Ok(Tok::Str(s));
                }
                b'\\' => s.push(self.escape()?),
                0 | b'\n' => {
                    return Err(Diag::error(
                        self.span(start),
                        "lex.string",
                        "unterminated string",
                    ))
                }
                c => {
                    s.push(c as char);
                    self.pos += 1;
                }
            }
        }
    }

    fn character(&mut self) -> Result<Tok, Diag> {
        let start = self.pos;
        self.pos += 1;
        let c = if self.peek() == b'\\' {
            self.escape()?
        } else {
            let c = self.peek() as char;
            self.pos += 1;
            c
        };
        if self.peek() != b'\'' {
            return Err(Diag::error(
                self.span(start),
                "lex.char",
                "unterminated character literal",
            ));
        }
        self.pos += 1;
        Ok(Tok::Char(c))
    }

    fn word(&mut self) -> Tok {
        let start = self.pos;
        while self.peek().is_ascii_alphanumeric() || self.peek() == b'_' {
            self.pos += 1;
        }
        // `E!` is the effect marker, lexed as one token.
        if &self.src[start..self.pos] == b"E" && self.peek() == b'!' {
            self.pos += 1;
            return Tok::Sym("E!");
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        if let Some(k) = KEYWORDS.iter().find(|k| **k == text) {
            return Tok::Kw(k);
        }
        if text.starts_with(|c: char| c.is_ascii_uppercase()) {
            Tok::Ctor(text.to_string())
        } else {
            Tok::Name(text.to_string())
        }
    }

    fn symbol(&mut self) -> Result<Tok, Diag> {
        let rest = &self.src[self.pos..];
        for s in SYMBOLS {
            if rest.starts_with(s.as_bytes()) {
                self.pos += s.len();
                return Ok(Tok::Sym(s));
            }
        }
        let start = self.pos;
        self.pos += 1;
        Err(Diag::error(
            self.span(start),
            "lex.char",
            &format!("unexpected character `{}`", rest[0] as char),
        ))
    }
}

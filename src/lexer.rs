//! Lexer for Vibelang. Newline-significant, column-tracking.
//!
//! No INDENT/DEDENT tokens: the parser uses column numbers directly (match arms
//! align on their `|`, declarations start at column 0). That is enough layout
//! for the whole grammar and keeps the lexer a flat loop.

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
    pub col: usize,
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
    "mod", "ext", "exp", "type", "ghost", "let", "in", "own", "ref", "arena", "with", "c", "True",
    "False",
];

// Longest first: the matcher takes the first hit.
const SYMBOLS: &[&str] = &[
    "|>", "<-", "->", "==", "!=", "<=", ">=", "&&", "||", "++", "E!", "::", ":", "=", "?", "|",
    "&", "%", "{", "}", "[", "]", "(", ")", ",", ".", "\\", "+", "-", "*", "/", "<", ">", "!",
];

pub struct Lexer<'a> {
    src: &'a [u8],
    file: usize,
    pos: usize,
    line: usize,
    line_start: usize,
}

pub fn lex(src: &str, file: usize) -> Result<Vec<Token>, Diag> {
    Lexer { src: src.as_bytes(), file, pos: 0, line: 1, line_start: 0 }.run()
}

impl<'a> Lexer<'a> {
    fn col(&self) -> usize {
        self.pos - self.line_start
    }
    fn span(&self, start: usize) -> Span {
        Span { file: self.file, line: self.line, col: start - self.line_start, len: self.pos - start }
    }
    fn peek(&self) -> u8 {
        *self.src.get(self.pos).unwrap_or(&0)
    }
    fn at(&self, off: usize) -> u8 {
        *self.src.get(self.pos + off).unwrap_or(&0)
    }

    fn run(mut self) -> Result<Vec<Token>, Diag> {
        let mut out: Vec<Token> = Vec::new();
        let mut blank_run = 0usize;
        loop {
            // Horizontal whitespace and comments.
            loop {
                match self.peek() {
                    b' ' | b'\t' | b'\r' => self.pos += 1,
                    b';' if self.at(1) == b';' => {
                        while self.peek() != b'\n' && self.pos < self.src.len() {
                            self.pos += 1;
                        }
                    }
                    _ => break,
                }
            }
            if self.pos >= self.src.len() {
                out.push(Token { tok: Tok::Eof, span: self.span(self.pos), col: 0 });
                return Ok(out);
            }
            if self.peek() == b'\n' {
                let blank = matches!(out.last(), None | Some(Token { tok: Tok::Newline, .. }));
                if blank {
                    blank_run += 1;
                    // P1: one blank line separates declarations, never two.
                    if blank_run > 2 {
                        return Err(Diag::error(
                            self.span(self.pos),
                            "canon.blankline",
                            "more than one blank line in a row",
                        )
                        .with_fix("delete the extra blank lines"));
                    }
                } else {
                    blank_run = 0;
                    out.push(Token { tok: Tok::Newline, span: self.span(self.pos), col: self.col() });
                }
                self.pos += 1;
                self.line += 1;
                self.line_start = self.pos;
                continue;
            }
            let start = self.pos;
            let col = self.col();
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
            out.push(Token { tok, span, col });
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
        let text: String =
            std::str::from_utf8(&self.src[start..self.pos]).unwrap().chars().filter(|c| *c != '_').collect();
        if is_float {
            Ok(Tok::Float(text.parse().unwrap()))
        } else {
            text.parse::<i64>().map(Tok::Int).map_err(|_| {
                Diag::error(self.span(start), "lex.int", "integer literal does not fit in 64 bits")
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
                    return Err(Diag::error(self.span(start), "lex.string", "unterminated string"))
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
        let c = if self.peek() == b'\\' { self.escape()? } else {
            let c = self.peek() as char;
            self.pos += 1;
            c
        };
        if self.peek() != b'\'' {
            return Err(Diag::error(self.span(start), "lex.char", "unterminated character literal"));
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

//! Abstract syntax. Mirrors spec §3, plus the `ext c` link/pkg clauses that make
//! real C libraries usable without a build system.

use crate::diag::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    /// `TypeName arg arg` — also the 0-ary case (`U64`, `Str`, a user ADT).
    Con(String, Vec<Ty>),
    /// Lower-case type variable in a signature (`a`, `b`).
    Var(String),
    Fun(Box<Ty>, Box<Ty>),
    /// `&T`. Erased by the bootstrap type checker (see ownership debt in the spec).
    Ref(Box<Ty>),
    /// `E! T`.
    Eff(Box<Ty>),
    Tuple(Vec<Ty>),
}

impl Ty {
    pub fn unit() -> Ty {
        Ty::Con("Unit".into(), vec![])
    }
    pub fn con(n: &str) -> Ty {
        Ty::Con(n.into(), vec![])
    }
}

#[derive(Clone, Debug)]
pub enum Pat {
    Wild,
    Var(String),
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    Bool(bool),
    Ctor(String, Vec<Pat>),
    List(Vec<Pat>),
    Tuple(Vec<Pat>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    Bool(bool),
    Unit,
    Var(String),
    Ctor(String),
    App(Box<Expr>, Vec<Expr>),
    Binop(String, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Match(Box<Expr>, Vec<(Pat, Expr)>),
    /// `name <- value` followed by the rest of the effectful block.
    Bind(String, Box<Expr>, Box<Expr>),
    Let(String, Box<Expr>, Box<Expr>),
    Lambda(Vec<String>, Box<Expr>),
    /// `{f = v, ...}` or `{base with f = v, ...}`.
    Record(Option<Box<Expr>>, Vec<(String, Expr)>),
    Field(Box<Expr>, String),
    Borrow(Box<Expr>),
    /// `arena a in body` — everything the body allocates dies with the block.
    Arena(String, Box<Expr>),
    Tuple(Vec<Expr>),
    List(Vec<Expr>),
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        Expr { kind, span }
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub names: Vec<String>,
    pub ty: Option<Ty>,
    pub refines: Vec<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FunDecl {
    pub name: String,
    pub ghost: bool,
    pub params: Vec<Param>,
    pub ret: Option<Ty>,
    pub body: Expr,
    pub measure: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RecordDef {
    pub fields: Vec<(String, Ty)>,
    pub refines: Vec<Expr>,
}

#[derive(Clone, Debug)]
pub struct VariantDef {
    pub name: String,
    pub args: Vec<Ty>,
}

#[derive(Clone, Debug)]
pub enum TypeBody {
    Record(RecordDef),
    Variants(Vec<VariantDef>),
    /// `type Window` inside an `ext c` block: an opaque C type, only ever seen
    /// behind `Ptr`.
    Opaque,
}

#[derive(Clone, Debug)]
pub struct TypeDecl {
    pub name: String,
    pub params: Vec<String>,
    pub body: TypeBody,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ExtSig {
    pub name: String,
    /// C symbol, when it differs from the Vibelang name (`name = "c_symbol"`).
    pub symbol: String,
    pub ty: Ty,
    pub refines: Vec<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ExtBlock {
    pub header: String,
    /// `-l` names.
    pub links: Vec<String>,
    /// pkg-config package names.
    pub pkgs: Vec<String>,
    pub sigs: Vec<ExtSig>,
    pub types: Vec<TypeDecl>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Decl {
    Type(TypeDecl),
    Fun(FunDecl),
    Ext(ExtBlock),
    Exp(Vec<String>, Span),
}

#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    pub decls: Vec<Decl>,
    pub span: Span,
}

impl Module {
    pub fn funs(&self) -> impl Iterator<Item = &FunDecl> {
        self.decls.iter().filter_map(|d| match d {
            Decl::Fun(f) => Some(f),
            _ => None,
        })
    }
    pub fn types(&self) -> Vec<&TypeDecl> {
        let mut v: Vec<&TypeDecl> = Vec::new();
        for d in &self.decls {
            match d {
                Decl::Type(t) => v.push(t),
                Decl::Ext(e) => v.extend(e.types.iter()),
                _ => {}
            }
        }
        v
    }
    pub fn exts(&self) -> impl Iterator<Item = &ExtBlock> {
        self.decls.iter().filter_map(|d| match d {
            Decl::Ext(e) => Some(e),
            _ => None,
        })
    }
    pub fn exports(&self) -> Vec<String> {
        let mut v = Vec::new();
        for d in &self.decls {
            if let Decl::Exp(names, _) = d {
                v.extend(names.iter().cloned());
            }
        }
        v
    }
}

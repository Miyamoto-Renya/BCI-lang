use bci_schema::{number::Rational, types::Span, unit::Unit};

#[derive(Debug, Clone)]
pub(crate) struct Program {
    pub language: String,
    pub module: String,
    pub imports: Vec<(String, String, Span)>,
    pub bindings: Vec<Binding>,
    pub graphs: Vec<Graph>,
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone)]
pub(crate) struct Binding {
    pub name: String,
    pub kind: String,
    pub annotation: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub(crate) struct Graph {
    pub name: String,
    pub params: Vec<(String, TypeExpr)>,
    pub output: TypeExpr,
    pub mode: String,
    pub body: Vec<Statement>,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub(crate) struct Run {
    pub name: String,
    pub body: Vec<Statement>,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub(crate) enum Statement {
    Let(Box<Binding>),
    Expression(Expr),
    Return(Expr),
}
#[derive(Debug, Clone)]
pub(crate) struct TypeExpr {
    pub name: String,
    pub args: Vec<TypeExpr>,
    pub literal: Option<(Rational, Unit)>,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub(crate) struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub(crate) struct Argument {
    pub name: Option<String>,
    pub value: Expr,
}
#[derive(Debug, Clone)]
pub(crate) enum ExprKind {
    Number(Rational, Unit),
    Text(String),
    Bool(bool),
    Path(String),
    Array(Vec<Expr>),
    Tuple(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Call(Box<Expr>, Vec<Argument>),
    Field(Box<Expr>, String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
}

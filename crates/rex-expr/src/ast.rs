//! Spanned abstract syntax trees for the rexlang expression sublanguage.
//!
//! Every node carries a [`Span`] (a byte range into the expression source).
//! The shapes mirror `docs/EXPRESSIONS.md`; see that document for the normative
//! grammar and semantics.

use chumsky::span::SimpleSpan;
use std::fmt;

/// Byte-offset span into the expression source text.
pub type Span = SimpleSpan<usize>;

/// A value paired with the span it was written at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Span of the value in the source.
    pub span: Span,
}

/// A parsed expression: a kind plus the span it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// The expression shape.
    pub kind: ExprKind,
    /// Span covering the whole expression.
    pub span: Span,
}

impl Expr {
    /// Builds an expression of the given kind and span.
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { kind, span }
    }
}

/// The shape of an expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// An integer literal. The token grammar admits non-negative decimals; a
    /// leading `-` is [`ExprKind::Unary`] with [`UnOp::Neg`]. Literals are
    /// typed `int` by default, `long` in a long context (spec rule L1).
    Int(i64),
    /// A string literal, unescaped.
    String(String),
    /// A boolean literal (`true`/`false`).
    Bool(bool),
    /// The `null` literal, typed [`crate::Ty::Null`] (spec R2/R3).
    Null,
    /// The `date("YYYY-MM-DD")` constructor: a calendar-date constant
    /// (issue #9). The argument must be a string literal — the parser
    /// enforces that shape; the checker validates the calendar date and
    /// rejects malformed text with the literal's span (spec R6). There is no
    /// runtime constructor in the language: the value is a constant.
    Date {
        /// The unescaped literal text, e.g. `"2026-09-17"`.
        text: String,
        /// Span of the string literal itself (R6's error span).
        literal_span: Span,
    },
    /// A variable reference: a `let` binding or lambda parameter.
    Name(String),
    /// Feature access `receiver.name` or safe navigation `receiver?.name`.
    FeatureAccess {
        /// The object being accessed.
        receiver: Box<Expr>,
        /// The feature name as written.
        name: Spanned<String>,
        /// `true` for `?.` (safe navigation): the receiver may be `None`, and
        /// the access result is wrapped in `Option` unless it already is one.
        optional_safe: bool,
    },
    /// Coalescing `value ?: default` (spec R3): `default` is evaluated when
    /// `value` is `None`.
    Coalesce {
        /// The optional value.
        value: Box<Expr>,
        /// The fallback.
        default: Box<Expr>,
    },
    /// An operation call `receiver.name(args)` on a model-declared `op`.
    Call {
        /// The object the operation is invoked on.
        receiver: Box<Expr>,
        /// The operation name as written.
        name: Spanned<String>,
        /// Arguments in source order.
        args: Vec<Expr>,
        /// `true` when invoked through `?.`.
        optional_safe: bool,
    },
    /// The fixed collection algebra: `receiver.kind(lambda?)` (spec A1–A6).
    Algebra {
        /// The collection being folded over.
        receiver: Box<Expr>,
        /// Which of the six algebra operations.
        kind: AlgebraKind,
        /// The lambda `(param, body)`, where present (`first` may omit it).
        lambda: Option<(String, Box<Expr>)>,
        /// `true` when invoked through `?.`.
        optional_safe: bool,
    },
    /// A binary operation `lhs op rhs`.
    Binary {
        /// The operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// A unary operation `op expr`.
    Unary {
        /// The operator.
        op: UnOp,
        /// The operand.
        expr: Box<Expr>,
    },
    /// `if cond { then } else { else_ }` (spec U1).
    If {
        /// The condition; must be `boolean`.
        cond: Box<Expr>,
        /// The then-branch.
        then: Box<Expr>,
        /// The else-branch.
        else_: Box<Expr>,
    },
    /// `let name = init; body` (shadowing allowed).
    Let {
        /// The bound name as written.
        name: Spanned<String>,
        /// The initializer.
        init: Box<Expr>,
        /// The body, in which `name` is bound.
        body: Box<Expr>,
    },
    /// A single-parameter lambda `param => body` with an inferred parameter
    /// type (inferred only inside a collection algebra call).
    Lambda {
        /// The parameter name.
        param: String,
        /// The body, in which `param` is bound.
        body: Box<Expr>,
    },
    /// A list literal `[a, b, c]` or the empty `[]`.
    ListLiteral(Vec<Expr>),
}

/// One of the six fixed collection algebra operations (spec A1–A6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlgebraKind {
    /// `first` — first element, or first matching the optional lambda.
    First,
    /// `filter` — keep the elements whose lambda is `true`.
    Filter,
    /// `map` — transform each element.
    Map,
    /// `any` — whether any element's lambda is `true`.
    Any,
    /// `size` — the element count.
    Size,
    /// `sum` — the numeric sum of the elements.
    Sum,
}

impl AlgebraKind {
    /// The algebra operation named by `name` in call position, or `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "first" => Some(AlgebraKind::First),
            "filter" => Some(AlgebraKind::Filter),
            "map" => Some(AlgebraKind::Map),
            "any" => Some(AlgebraKind::Any),
            "size" => Some(AlgebraKind::Size),
            "sum" => Some(AlgebraKind::Sum),
            _ => None,
        }
    }
}

impl fmt::Display for AlgebraKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AlgebraKind::First => "first",
            AlgebraKind::Filter => "filter",
            AlgebraKind::Map => "map",
            AlgebraKind::Any => "any",
            AlgebraKind::Size => "size",
            AlgebraKind::Sum => "sum",
        })
    }
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `==` (or the accepted `=` alias; spec R2).
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `&&`.
    And,
    /// `||`.
    Or,
}

impl BinOp {
    /// Whether this operator is `+`, `-`, `*`, or `/`.
    pub fn is_arithmetic(self) -> bool {
        matches!(self, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div)
    }

    /// Whether this operator is `<`, `<=`, `>`, or `>=`.
    pub fn is_relational(self) -> bool {
        matches!(self, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }

    /// Whether this operator is `==` or `!=`.
    pub fn is_equality(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne)
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::And => "&&",
            BinOp::Or => "||",
        })
    }
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `!` — logical negation.
    Not,
    /// `-` — arithmetic negation.
    Neg,
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UnOp::Not => "!",
            UnOp::Neg => "-",
        })
    }
}

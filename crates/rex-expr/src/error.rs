//! Expression-language errors and the parse result.

use crate::ast::{Expr, Span};

/// A single error produced while parsing or type-checking an expression.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message} at byte offsets {span}")]
pub struct ExprError {
    /// Human-readable description of the error; semantic errors cite the
    /// governing spec rule (`R1`–`R4`) by number.
    pub message: String,
    /// Byte range the error refers to.
    pub span: Span,
}

impl ExprError {
    /// Builds an error with the given message and span.
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        ExprError {
            message: message.into(),
            span,
        }
    }
}

/// The outcome of parsing an expression: as much of the tree as could be
/// recovered, plus all errors. Parsing never panics.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseResult {
    /// The recovered expression, or `None` if no expression could be parsed.
    pub ast: Option<Expr>,
    /// All errors encountered while lexing and parsing.
    pub errors: Vec<ExprError>,
}

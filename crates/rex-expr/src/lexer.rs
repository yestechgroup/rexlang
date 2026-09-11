//! Token definitions and the lexer entry point for expressions.
//!
//! Lexing is total: characters that match no pattern (and integer literals
//! that do not fit `i64`) become [`Token::Error`] tokens carrying the
//! offending span, which the parser then reports. It never fails and never
//! panics.

use logos::Logos;

use crate::ast::Span;

/// A lexical token. String/identifier payloads borrow from the source text.
#[derive(Debug, Clone, PartialEq, Logos)]
pub enum Token<'src> {
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("let")]
    Let,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("null")]
    Null,

    /// A decimal integer literal (non-negative; `-` is the unary operator).
    /// Literals that do not fit `i64` fail the callback and become
    /// [`Token::Error`] (spec: lexical rules).
    #[regex("[0-9]+", |lexer| lexer.slice().parse::<i64>().map_err(|_| ()))]
    Int(i64),
    /// A string literal payload: the raw text between the quotes, with
    /// escapes left intact (unescaping happens in the parser).
    #[regex(r#""([^"\\\n\r]|\\.)*""#, |lexer| {
        let slice = lexer.slice();
        &slice[1..slice.len() - 1]
    })]
    Str(&'src str),
    /// An identifier: `[A-Za-z_][A-Za-z0-9_]*`.
    #[regex("[A-Za-z_][A-Za-z0-9_]*", |lexer| lexer.slice())]
    Ident(&'src str),

    #[token(".")]
    Dot,
    #[token("?.")]
    QuestionDot,
    #[token("?:")]
    QuestionColon,
    #[token(",")]
    Comma,
    #[token(";")]
    Semi,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("=>")]
    Arrow,
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<=")]
    Le,
    #[token(">=")]
    Ge,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("&&")]
    AmpAmp,
    #[token("||")]
    PipePipe,
    #[token("!")]
    Bang,
    #[token("=")]
    Eq,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,

    // Trivia: whitespace is skipped outright, as are `//` line and `/* */`
    // block comments (spec: lexical rules).
    #[regex(r"[ \t\r\n\f]+", logos::skip, priority = 1)]
    Whitespace,
    #[regex(r"//[^\r\n]*", logos::skip, priority = 1)]
    LineComment,
    #[regex(r"/\*[^*]*\*+([^/*][^*]*\*+)*/", logos::skip, priority = 1)]
    BlockComment,

    /// Emitted for input that matched no pattern (unknown characters,
    /// out-of-range integer literals). The parser reports these as errors.
    Error,
}

impl std::fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::If => f.write_str("`if`"),
            Token::Else => f.write_str("`else`"),
            Token::Let => f.write_str("`let`"),
            Token::True => f.write_str("`true`"),
            Token::False => f.write_str("`false`"),
            Token::Null => f.write_str("`null`"),
            Token::Int(value) => write!(f, "`{value}`"),
            Token::Str(_) => f.write_str("string literal"),
            Token::Ident(text) => write!(f, "`{text}`"),
            Token::Dot => f.write_str("`.`"),
            Token::QuestionDot => f.write_str("`?.`"),
            Token::QuestionColon => f.write_str("`?:`"),
            Token::Comma => f.write_str("`,`"),
            Token::Semi => f.write_str("`;`"),
            Token::LParen => f.write_str("`(`"),
            Token::RParen => f.write_str("`)`"),
            Token::LBrace => f.write_str("`{`"),
            Token::RBrace => f.write_str("`}`"),
            Token::LBracket => f.write_str("`[`"),
            Token::RBracket => f.write_str("`]`"),
            Token::Arrow => f.write_str("`=>`"),
            Token::EqEq => f.write_str("`==`"),
            Token::NotEq => f.write_str("`!=`"),
            Token::Le => f.write_str("`<=`"),
            Token::Ge => f.write_str("`>=`"),
            Token::Lt => f.write_str("`<`"),
            Token::Gt => f.write_str("`>`"),
            Token::AmpAmp => f.write_str("`&&`"),
            Token::PipePipe => f.write_str("`||`"),
            Token::Bang => f.write_str("`!`"),
            Token::Eq => f.write_str("`=`"),
            Token::Plus => f.write_str("`+`"),
            Token::Minus => f.write_str("`-`"),
            Token::Star => f.write_str("`*`"),
            Token::Slash => f.write_str("`/`"),
            // Skipped trivia: never reaches the parser, so never displayed.
            Token::Whitespace | Token::LineComment | Token::BlockComment => {
                f.write_str("whitespace")
            }
            Token::Error => f.write_str("invalid token"),
        }
    }
}

/// Tokenize an expression source text.
///
/// Whitespace and comments are skipped; input that matches no pattern becomes
/// [`Token::Error`] tokens, so this function is total.
pub fn lex(source: &str) -> Vec<(Token<'_>, Span)> {
    Token::lexer(source)
        .spanned()
        .map(|(result, range)| {
            let token = result.unwrap_or(Token::Error);
            (token, range.into())
        })
        .collect()
}

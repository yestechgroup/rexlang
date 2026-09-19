//! A fault-tolerant chumsky parser for the expression sublanguage.
//!
//! The grammar and precedence table are normative in `docs/EXPRESSIONS.md`.
//! Precedence is implemented with explicit chained levels (loosest first):
//! `?:` → `||` → `&&` → comparisons → `+ -` → `* /` → unary → postfix, all
//! left-associative; `let`/lambda are prefix forms at the top.
//!
//! Recovery strategy: the top level parses one expression; any leftover input
//! is consumed by a junk-region parser that emits a single error, so trailing
//! garbage never discards a successfully parsed expression. Input that cannot
//! start an expression yields `None` plus the parser's errors. Parsing never
//! panics.

use chumsky::input::{Stream, ValueInput};
use chumsky::prelude::*;

use crate::ast::{AlgebraKind, BinOp, Expr, ExprKind, Span, Spanned, UnOp};
use crate::error::{ExprError, ParseResult};
use crate::lexer::{lex, Token};

type ExExtra<'src> = extra::Err<Rich<'src, Token<'src>>>;

/// Lex and parse an expression source text.
///
/// This function never panics and always recovers as much of the expression
/// as possible; check [`ParseResult::errors`] for syntax problems.
pub fn parse(source: &str) -> ParseResult {
    let tokens = lex(source);
    let eoi: Span = (source.len()..source.len()).into();
    let stream = Stream::from_iter(tokens).map(eoi, |(token, span)| (token, span));

    let (ast, errors) = expr_parser()
        .then_ignore(junk_tail())
        .parse(stream)
        .into_output_errors();

    ParseResult {
        ast,
        errors: errors
            .into_iter()
            .map(|error| ExprError::new(error.to_string(), *error.span()))
            .collect(),
    }
}

/// The `date("YYYY-MM-DD")` primary form: the contextual word `date`
/// followed by a parenthesized string literal (spec R6). `date` is not a
/// keyword — a bare `date` with no string-literal argument still parses as
/// an ordinary name. When the parenthesized argument is not a string
/// literal, one clear error is emitted and an empty date literal is
/// recovered (the checker is never reached with parse errors in the wired
/// pipeline, so the placeholder cannot leak into lowering).
fn date_literal<'src, I, P>(expr: &P) -> impl Parser<'src, I, Expr, ExExtra<'src>> + Clone
where
    I: ValueInput<'src, Token = Token<'src>, Span = Span>,
    P: Parser<'src, I, Expr, ExExtra<'src>> + Clone + 'src,
{
    select! { Token::Ident(text) = e if text == "date" => e.span() }
        .ignore_then(
            expr.clone()
                .map(|inner| match inner.kind {
                    ExprKind::String(text) => Ok(text),
                    _ => Err(inner.span),
                })
                .validate(|arg, _e, emitter| match arg {
                    Ok(text) => text,
                    Err(span) => {
                        emitter.emit(Rich::custom(
                            span,
                            "the `date` constructor takes a string literal \
                                 (`date(\"YYYY-MM-DD\")`)",
                        ));
                        String::new()
                    }
                })
                .map_with(|text, e| {
                    let literal_span: Span = e.span();
                    (text, literal_span)
                })
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .map_with(|(text, literal_span), e| {
            Expr::new(ExprKind::Date { text, literal_span }, e.span())
        })
        .boxed()
}

/// Consumes every token after a successfully parsed expression, emitting one
/// error for the skipped region. Consuming zero tokens emits no error, so a
/// clean parse stays clean.
fn junk_tail<'src, I>() -> impl Parser<'src, I, (), ExExtra<'src>> + Clone
where
    I: ValueInput<'src, Token = Token<'src>, Span = Span>,
{
    any()
        .repeated()
        .collect::<Vec<_>>()
        .validate(|junk, e, emitter| {
            if !junk.is_empty() {
                emitter.emit(Rich::custom(e.span(), "unexpected trailing input"));
            }
        })
        .ignored()
}

fn name<'src, I>() -> impl Parser<'src, I, Spanned<String>, ExExtra<'src>> + Clone
where
    I: ValueInput<'src, Token = Token<'src>, Span = Span>,
{
    select! {
        Token::Ident(text) = e => Spanned { value: text.to_string(), span: e.span() },
    }
}

fn int_lit<'src, I>() -> impl Parser<'src, I, i64, ExExtra<'src>> + Clone
where
    I: ValueInput<'src, Token = Token<'src>, Span = Span>,
{
    select! { Token::Int(value) => value }
}

fn string_lit<'src, I>() -> impl Parser<'src, I, String, ExExtra<'src>> + Clone
where
    I: ValueInput<'src, Token = Token<'src>, Span = Span>,
{
    select! { Token::Str(raw) => unescape_string(raw) }
}

fn unescape_string(raw: &str) -> String {
    let mut value = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => value.push('"'),
                Some('\\') => value.push('\\'),
                // Unknown escapes are kept verbatim rather than rejected.
                Some(other) => {
                    value.push('\\');
                    value.push(other);
                }
                None => value.push('\\'),
            }
        } else {
            value.push(c);
        }
    }
    value
}

/// One postfix step: `.name` or `?.name`, with an optional parenthesized
/// argument list. The algebra argument shapes (spec A1–A6) are validated
/// here, at parse time. The step span covers `.`/`?.` through the member
/// name and (when present) its argument list, closing paren included.
fn member<'src, I, P>(
    expr: &P,
) -> impl Parser<'src, I, (bool, Spanned<String>, Option<Vec<Expr>>, Span), ExExtra<'src>> + Clone
where
    I: ValueInput<'src, Token = Token<'src>, Span = Span>,
    P: Parser<'src, I, Expr, ExExtra<'src>> + Clone,
{
    let args = expr
        .clone()
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen));

    choice((
        just(Token::Dot).to(false),
        just(Token::QuestionDot).to(true),
    ))
    .then(name())
    .then(args.or_not())
    .validate(|((optional_safe, member_name), args), _e, emitter| {
        if let Some(args) = &args {
            if let Some(algebra) = AlgebraKind::from_name(&member_name.value) {
                let shape_ok = match algebra {
                    AlgebraKind::Size | AlgebraKind::Sum => args.is_empty(),
                    AlgebraKind::First => args.len() <= 1 && args.first().is_none_or(is_lambda),
                    AlgebraKind::Filter | AlgebraKind::Map | AlgebraKind::Any => {
                        args.len() == 1 && is_lambda(&args[0])
                    }
                };
                if !shape_ok {
                    let message = match algebra {
                        AlgebraKind::Size | AlgebraKind::Sum => {
                            format!("collection algebra `{algebra}` takes no arguments")
                        }
                        _ => format!(
                            "collection algebra `{algebra}` requires exactly one lambda \
                                 (`name => expr`); `first` also accepts none"
                        ),
                    };
                    emitter.emit(Rich::custom(member_name.span, message));
                }
            }
        }
        ((optional_safe, member_name), args)
    })
    .map_with(|((optional_safe, member_name), args), e| {
        let step_span: Span = e.span();
        (optional_safe, member_name, args, step_span)
    })
}

fn is_lambda(expr: &Expr) -> bool {
    matches!(expr.kind, ExprKind::Lambda { .. })
}

/// Extracts the `(param, body)` lambda from an argument list, for algebra
/// nodes.
fn lambda_of(args: &[Expr]) -> Option<(String, Box<Expr>)> {
    match args.first().map(|expr| &expr.kind) {
        Some(ExprKind::Lambda { param, body }) => Some((param.clone(), body.clone())),
        _ => None,
    }
}

/// Builds the node a postfix step denotes: a feature access (no arguments),
/// a collection algebra operation (algebra name + parentheses), or an
/// operation call (any other name + parentheses).
fn apply_member(receiver: Expr, step: (bool, Spanned<String>, Option<Vec<Expr>>, Span)) -> Expr {
    let (optional_safe, member_name, args, step_span) = step;
    let span = (receiver.span.start..step_span.end).into();
    match args {
        None => Expr::new(
            ExprKind::FeatureAccess {
                receiver: Box::new(receiver),
                name: member_name,
                optional_safe,
            },
            span,
        ),
        Some(args) => match AlgebraKind::from_name(&member_name.value) {
            Some(algebra) => {
                let lambda = match algebra {
                    AlgebraKind::Size | AlgebraKind::Sum => None,
                    _ => lambda_of(&args),
                };
                Expr::new(
                    ExprKind::Algebra {
                        receiver: Box::new(receiver),
                        kind: algebra,
                        lambda,
                        optional_safe,
                    },
                    span,
                )
            }
            None => Expr::new(
                ExprKind::Call {
                    receiver: Box::new(receiver),
                    name: member_name,
                    args,
                    optional_safe,
                },
                span,
            ),
        },
    }
}

/// Folds `operand (op operand)*` into a left-associative binary tree.
fn chain<'src, I>(
    operand: impl Parser<'src, I, Expr, ExExtra<'src>> + Clone,
    op: impl Parser<'src, I, BinOp, ExExtra<'src>> + Clone,
) -> impl Parser<'src, I, Expr, ExExtra<'src>> + Clone
where
    I: ValueInput<'src, Token = Token<'src>, Span = Span>,
{
    operand
        .clone()
        .then(op.then(operand).repeated().collect::<Vec<_>>())
        .map(|(first, rest)| {
            rest.into_iter().fold(first, |lhs, (op, rhs)| {
                let span = (lhs.span.start..rhs.span.end).into();
                Expr::new(
                    ExprKind::Binary {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    span,
                )
            })
        })
}

/// One date-literal parser, wired into the primary `choice` below.
///
/// The parser for one expression, level by level. Every level is
/// [`Parser::boxed`]: the fully inlined combinator tower would be so deeply
/// typed that merely constructing (or dropping) it can exhaust a small
/// thread's stack; boxing flattens the tower to constant depth.
fn expr_parser<'src, I>() -> impl Parser<'src, I, Expr, ExExtra<'src>> + Clone
where
    I: ValueInput<'src, Token = Token<'src>, Span = Span>,
{
    recursive(|expr| {
        let atom = choice((
            int_lit().map_with(|value, e| Expr::new(ExprKind::Int(value), e.span())),
            string_lit().map_with(|value, e| Expr::new(ExprKind::String(value), e.span())),
            date_literal(&expr),
            just(Token::True).map_with(|_, e| Expr::new(ExprKind::Bool(true), e.span())),
            just(Token::False).map_with(|_, e| Expr::new(ExprKind::Bool(false), e.span())),
            just(Token::Null).map_with(|_, e| Expr::new(ExprKind::Null, e.span())),
            name().map_with(|n, e| Expr::new(ExprKind::Name(n.value), e.span())),
            expr.clone()
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .boxed(),
            just(Token::LBracket)
                .ignore_then(
                    expr.clone()
                        .separated_by(just(Token::Comma))
                        .collect::<Vec<_>>(),
                )
                .then_ignore(just(Token::RBracket))
                .map_with(|items, e| Expr::new(ExprKind::ListLiteral(items), e.span()))
                .boxed(),
            just(Token::If)
                .ignore_then(expr.clone())
                .then_ignore(just(Token::LBrace))
                .then(expr.clone())
                .then_ignore(just(Token::RBrace))
                .then_ignore(just(Token::Else))
                .then_ignore(just(Token::LBrace))
                .then(expr.clone())
                .then_ignore(just(Token::RBrace))
                .map_with(|((cond, then), else_), e| {
                    Expr::new(
                        ExprKind::If {
                            cond: Box::new(cond),
                            then: Box::new(then),
                            else_: Box::new(else_),
                        },
                        e.span(),
                    )
                })
                .boxed(),
        ))
        .boxed();

        // postfix: primary (("." | "?.") member)*
        let postfix = atom
            .then(member(&expr).repeated().collect::<Vec<_>>())
            .map(|(first, steps)| steps.into_iter().fold(first, apply_member))
            .boxed();

        // unary: ("!" | "-")* postfix, folded right-to-left
        let prefixes = choice((
            just(Token::Bang).map_with(|_, e| {
                let s: Span = e.span();
                (UnOp::Not, s)
            }),
            just(Token::Minus).map_with(|_, e| {
                let s: Span = e.span();
                (UnOp::Neg, s)
            }),
        ))
        .repeated()
        .collect::<Vec<_>>();
        let unary = prefixes
            .then(postfix)
            .map_with(|(prefixes, mut expr), e| {
                let whole: Span = e.span();
                let end = whole.end;
                for (op, span) in prefixes.into_iter().rev() {
                    expr = Expr::new(
                        ExprKind::Unary {
                            op,
                            expr: Box::new(expr),
                        },
                        (span.start..end).into(),
                    );
                }
                expr
            })
            .boxed();

        let mul = chain(
            unary,
            choice((
                just(Token::Star).to(BinOp::Mul),
                just(Token::Slash).to(BinOp::Div),
            )),
        )
        .boxed();
        let add = chain(
            mul,
            choice((
                just(Token::Plus).to(BinOp::Add),
                just(Token::Minus).to(BinOp::Sub),
            )),
        )
        .boxed();
        let comparison = chain(
            add,
            choice((
                // `==` must be tried before the `=` alias.
                just(Token::EqEq).to(BinOp::Eq),
                just(Token::Eq).to(BinOp::Eq),
                just(Token::NotEq).to(BinOp::Ne),
                just(Token::Le).to(BinOp::Le),
                just(Token::Ge).to(BinOp::Ge),
                just(Token::Lt).to(BinOp::Lt),
                just(Token::Gt).to(BinOp::Gt),
            )),
        )
        .boxed();
        let logic_and = chain(comparison, just(Token::AmpAmp).to(BinOp::And)).boxed();
        let logic_or = chain(logic_and, just(Token::PipePipe).to(BinOp::Or)).boxed();

        // coalesce: logic_or ("?:" logic_or)* — the loosest binary level
        let coalesce = logic_or
            .clone()
            .then(
                just(Token::QuestionColon)
                    .ignore_then(logic_or)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |value, default| {
                    let span = (value.span.start..default.span.end).into();
                    Expr::new(
                        ExprKind::Coalesce {
                            value: Box::new(value),
                            default: Box::new(default),
                        },
                        span,
                    )
                })
            })
            .boxed();

        // prefix forms at the top: let, lambda, then the operator levels
        let let_expr = just(Token::Let)
            .ignore_then(name())
            .then_ignore(just(Token::Eq))
            .then(expr.clone())
            .then_ignore(just(Token::Semi))
            .then(expr.clone())
            .map_with(|((bound_name, init), body), e| {
                Expr::new(
                    ExprKind::Let {
                        name: bound_name,
                        init: Box::new(init),
                        body: Box::new(body),
                    },
                    e.span(),
                )
            })
            .boxed();
        let lambda = name()
            .then_ignore(just(Token::Arrow))
            .then(expr.clone())
            .map_with(|(param, body), e| {
                Expr::new(
                    ExprKind::Lambda {
                        param: param.value,
                        body: Box::new(body),
                    },
                    e.span(),
                )
            })
            .boxed();

        choice((let_expr, lambda, coalesce))
    })
}

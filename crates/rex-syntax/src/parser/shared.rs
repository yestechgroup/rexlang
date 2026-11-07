//! Shared lexical combinators: names, string literals with escape
//! decoding, qualified names, type references, multiplicities,
//! defaults, target/op bodies, and the constraint blocks every
//! surface's grammar reuses.

use std::iter::once;

use crate::lexer::Token;

use super::*;
/// Consumes a run of tokens that cannot start or continue a feature,
/// stopping before the class body's closing `}` and before tokens that could
/// begin the next feature. Used as a recovery alternative: it always consumes
/// at least one token (guaranteeing progress) and emits an error pointing at
/// the skipped region.
pub(crate) fn junk_feature<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(
            t,
            Token::RBrace
                | Token::Ident(_)
                | Token::IdentEscaped(_)
                | Token::Contains
                | Token::Refers
                | Token::Container
                | Token::Op
                | Token::Derived
        ) =>
        ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected a feature declaration"));
        })
}

pub(crate) fn name<'src>() -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    select! {
        Token::Ident(text) = e => Name { text: text.to_string(), span: e.span(), escaped: false },
        Token::IdentEscaped(text) = e => Name { text: text.to_string(), span: e.span(), escaped: true },
    }
}

pub(crate) fn string_lit<'src>() -> impl Parser<'src, Tokens<'src>, String, MoxExtra<'src>> + Clone
{
    select! { Token::Str(raw) => unescape_string(raw) }
}

pub(crate) fn unescape_string(raw: &str) -> String {
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

pub(crate) fn int_lit<'src>() -> impl Parser<'src, Tokens<'src>, i64, MoxExtra<'src>> + Clone {
    select! { Token::Int(value) => value }
}

/// A qualified-name segment: an identifier or — exceptionally — a keyword
/// token (e.g. the trailing `actors` in `package rex.conformance.actors`).
/// Keywords only introduce grammar constructs in statement position; inside
/// a dotted reference they are just names. Escaped forms are handled by
/// [`name`].
pub(crate) fn qname_segment<'src>() -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone
{
    name().or(
        any().try_map(|token: Token<'src>, span| match token.keyword() {
            Some(keyword) => Ok(Name {
                text: keyword.to_string(),
                span,
                escaped: false,
            }),
            None => Err(Rich::custom(span, "expected a name")),
        }),
    )
}

pub(crate) fn qname<'src>() -> impl Parser<'src, Tokens<'src>, QualifiedName, MoxExtra<'src>> + Clone
{
    qname_segment()
        .then(
            kw(Token::Dot)
                .ignore_then(qname_segment())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map_with(|(first, rest), e| QualifiedName {
            segments: once(first).chain(rest).collect(),
            span: e.span(),
        })
}

pub(crate) fn tref<'src>() -> impl Parser<'src, Tokens<'src>, TypeRef, MoxExtra<'src>> + Clone {
    qname().map_with(|name, e| TypeRef {
        name,
        span: e.span(),
    })
}

pub(crate) fn multiplicity<'src>(
) -> impl Parser<'src, Tokens<'src>, Multiplicity, MoxExtra<'src>> + Clone {
    let bound = int_lit()
        .map(MultBound::Int)
        .or(kw(Token::Star).map(|_| MultBound::Star));
    let inner = int_lit()
        .or_not()
        .then(
            kw(Token::Dot)
                .ignore_then(kw(Token::Dot))
                .ignore_then(bound)
                .or_not(),
        )
        .map(|(lower, upper)| match (lower, upper) {
            (None, _) => MultiplicityKind::Unbounded,
            (Some(lower), None) => MultiplicityKind::Exact(lower),
            (Some(lower), Some(upper)) => MultiplicityKind::Range(lower, upper),
        });
    kw(Token::LBracket)
        .ignore_then(inner)
        .then_ignore(kw(Token::RBracket))
        .map_with(|kind, e| Multiplicity {
            kind,
            span: e.span(),
        })
}

pub(crate) fn default_value<'src>(
) -> impl Parser<'src, Tokens<'src>, DefaultValue, MoxExtra<'src>> + Clone {
    choice((
        string_lit().map_with(|value, e| DefaultValue::Str {
            value,
            span: e.span(),
        }),
        int_lit().map_with(|value, e| DefaultValue::Int {
            value,
            span: e.span(),
        }),
        kw(Token::True).map_with(|_, e| DefaultValue::Bool {
            value: true,
            span: e.span(),
        }),
        kw(Token::False).map_with(|_, e| DefaultValue::Bool {
            value: false,
            span: e.span(),
        }),
        name().map(DefaultValue::Name),
    ))
}

pub(crate) fn opposite<'src>() -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    kw(Token::Opposite).ignore_then(name())
}

/// Parses the contextual `id`/`readonly` modifiers that may precede any
/// feature. Both keywords lex as ordinary identifiers; they act as modifiers
/// only in this leading position, and the first token that is neither starts
/// the feature itself. Escaped forms (`^id`, `^readonly`) are different token
/// variants and are never modifiers. Repeats are idempotent.
pub(crate) fn modifiers<'src>() -> impl Parser<'src, Tokens<'src>, Modifiers, MoxExtra<'src>> + Clone
{
    select! {
        Token::Ident("id") = e => (ModifierKind::Id, e.span()),
        Token::Ident("readonly") = e => (ModifierKind::ReadOnly, e.span()),
    }
    .repeated()
    .collect::<Vec<_>>()
    .map(|pairs| {
        let mut modifiers = Modifiers::default();
        for (kind, span) in pairs {
            modifiers.push(kind, span);
        }
        modifiers
    })
}

/// Scans a raw `{ ... }` body with balanced braces and returns the span
/// covering everything from the opening to the closing brace, inclusive. The
/// contents are deliberately not parsed.
pub(crate) fn raw_body<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    let balanced = recursive(|body| {
        let atom = select! { t if !matches!(t, Token::LBrace | Token::RBrace) => () };
        atom.or(kw(Token::LBrace)
            .ignore_then(body)
            .then_ignore(kw(Token::RBrace)))
            .repeated()
            .ignored()
    });
    kw(Token::LBrace)
        .ignore_then(balanced)
        .then_ignore(kw(Token::RBrace))
        .map_with(|(), e| e.span())
}

/// Scans a raw `(...)` region with balanced parens and returns the span
/// covering everything from the opening to the closing paren, inclusive
/// (mirrors [`raw_body`], which is braces inclusive). The contents are
/// deliberately not parsed; the driver slices the condition text strictly
/// inside these bounds, same convention as [`TargetBody`].
pub(crate) fn raw_parens<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    let balanced = recursive(|body| {
        let atom = select! { t if !matches!(t, Token::LParen | Token::RParen) => () };
        atom.or(kw(Token::LParen)
            .ignore_then(body)
            .then_ignore(kw(Token::RParen)))
            .repeated()
            .ignored()
    });
    kw(Token::LParen)
        .ignore_then(balanced)
        .then_ignore(kw(Token::RParen))
        .map_with(|(), e| e.span())
}

/// One `<target> { ... }` body block inside an operation body or a datatype
/// `create`/`convert` block.
pub(crate) fn target_body<'src>(
) -> impl Parser<'src, Tokens<'src>, TargetBody, MoxExtra<'src>> + Clone {
    name()
        .then(raw_body())
        .map_with(|(target, span), _| TargetBody { target, span })
}

/// The body of an `op` declaration: either a bare balanced `{ ... }` block
/// (Tier 1 rejects it downstream) or one or more `<target> { ... }` blocks
/// inside an outer brace pair. Returns the outer span (when present) and the
/// target-tagged bodies (empty for a bare body).
pub(crate) fn op_body<'src>(
) -> impl Parser<'src, Tokens<'src>, (Option<Span>, Vec<TargetBody>), MoxExtra<'src>> + Clone {
    let tagged = kw(Token::LBrace)
        .ignore_then(target_body().repeated().at_least(1).collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|bodies, e| (Some(e.span()), bodies));
    let bare = raw_body().map(|span| (Some(span), Vec::new()));
    tagged
        .or(bare)
        .or_not()
        .map(|body| body.unwrap_or((None, Vec::new())))
}

pub(crate) fn params<'src>() -> impl Parser<'src, Tokens<'src>, Vec<Param>, MoxExtra<'src>> + Clone
{
    let param = tref().then(name()).map_with(|(type_ref, name), e| Param {
        type_ref,
        name,
        span: e.span(),
    });
    let list = param
        .clone()
        .then(
            kw(Token::Comma)
                .ignore_then(param)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| once(first).chain(rest).collect::<Vec<_>>());
    kw(Token::LParen)
        .ignore_then(list.or_not())
        .then_ignore(kw(Token::RParen))
        .map(|list| list.unwrap_or_default())
}

/// The closed set of value-taking constraint keywords allowed inside an
/// attribute's constraint block, plus the value-less `unique` (handled by
/// the flag arm below). They are contextual: ordinary identifiers everywhere
/// else, matching the `create`/`convert` precedent.
pub(crate) const VALUED_CONSTRAINT_KEYWORDS: [&str; 5] =
    ["pattern", "minLength", "maxLength", "minimum", "maximum"];

/// One `keyword value` entry inside an attribute's constraint block.
/// Value-less `unique` takes no value: a literal after it is a syntax error
/// (with recovery that keeps the block parseable).
pub(crate) fn constraint_entry<'src>(
) -> impl Parser<'src, Tokens<'src>, Constraint, MoxExtra<'src>> + Clone {
    let valued = select! {
        Token::Ident(text) = e if VALUED_CONSTRAINT_KEYWORDS.contains(&text) => (text, e.span()),
    }
    .then(
        string_lit()
            .map_with(|value, e| ConstraintValue::Str {
                value,
                span: e.span(),
            })
            .or(int_lit().map_with(|value, e| ConstraintValue::Int {
                value,
                span: e.span(),
            })),
    )
    .map_with(|((text, name_span), value), e| Constraint {
        name: Name {
            text: text.to_string(),
            span: name_span,
            escaped: false,
        },
        value,
        span: e.span(),
    });
    let flag = select! { Token::Ident("unique") = e => e.span() }
        .then(
            string_lit()
                .map_with(|_, e| e.span())
                .or(int_lit().map_with(|_, e| e.span()))
                .or_not(),
        )
        .validate(|(name_span, value), _e, emitter| {
            if let Some(span) = value {
                emitter.emit(Rich::custom(span, "constraint `unique` takes no value"));
            }
            name_span
        })
        .map_with(|name_span, e| Constraint {
            name: Name {
                text: "unique".to_string(),
                span: name_span,
                escaped: false,
            },
            value: ConstraintValue::Flag { span: e.span() },
            span: e.span(),
        });
    valued.or(flag)
}

/// An attribute's optional `{ pattern "..." minLength 3 }` constraint block:
/// a closed keyword set with literal values, each at most once (a repeat is
/// a syntax error, mirroring the datatype `create`/`convert` duplicate rule).
pub(crate) fn constraint_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<Constraint>, MoxExtra<'src>> + Clone {
    kw(Token::LBrace)
        .ignore_then(constraint_entry().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .validate(|entries, _e, emitter| {
            for (index, entry) in entries.iter().enumerate() {
                if entries[..index]
                    .iter()
                    .any(|prior| prior.name.text == entry.name.text)
                {
                    emitter.emit(Rich::custom(
                        entry.name.span,
                        format!("duplicate constraint `{}`", entry.name.text),
                    ));
                }
            }
            entries
        })
}

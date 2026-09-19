//! A fault-tolerant parser built with chumsky, producing a spanned AST.
//!
//! Recovery strategy:
//! * top-level: when the input at a position cannot start a declaration, a
//!   junk-region parser consumes tokens up to the next top-level declaration
//!   keyword (or end of input) and emits a single error for the region;
//! * features inside a class body: same approach, but the junk region stops at
//!   the body's closing `}` and at tokens that could begin the next feature.
//!
//! Both junk parsers always consume at least one token, which guarantees
//! forward progress (and therefore termination).

use std::iter::once;
use std::ops::{Range, RangeFrom};

use chumsky::input::{ExactSizeInput, ValueInput};
use chumsky::prelude::*;

use crate::ast::*;
use crate::lexer::{lex, lex_with_comments, Token};

/// Parser input: a token slice paired with a custom [`chumsky::Input`]
/// implementation that reports **byte-offset** spans (chumsky's built-in slice
/// input would report token indices instead).
#[derive(Clone, Copy)]
struct Tokens<'src> {
    tokens: &'src [(Token<'src>, Span)],
}

impl<'src> Tokens<'src> {
    fn new(tokens: &'src [(Token<'src>, Span)]) -> Self {
        Tokens { tokens }
    }

    /// Byte offset just past the end of the last token (0 for empty input).
    fn eof_offset(&self) -> usize {
        self.tokens.last().map(|(_, span)| span.end).unwrap_or(0)
    }

    /// Convert a token-index range into a byte span. An empty range maps to an
    /// empty span at the start offset of the token at `start` (or the end of
    /// the input when at EOF).
    fn byte_span(&self, start: usize, end: usize) -> Span {
        let start_byte = match self.tokens.get(start) {
            Some((_, span)) => span.start,
            None => self.eof_offset(),
        };
        let end_byte = if end > start {
            self.tokens
                .get(end - 1)
                .map(|(_, span)| span.end)
                .unwrap_or_else(|| self.eof_offset())
        } else {
            start_byte
        };
        (start_byte..end_byte).into()
    }
}

impl<'src> Input<'src> for Tokens<'src> {
    type Cursor = usize;
    type Span = Span;
    type Token = Token<'src>;
    type MaybeToken = Token<'src>;
    type Cache = Self;

    fn begin(self) -> (Self::Cursor, Self::Cache) {
        (0, self)
    }

    fn cursor_location(cursor: &Self::Cursor) -> usize {
        *cursor
    }

    unsafe fn next_maybe(
        this: &mut Self::Cache,
        cursor: &mut Self::Cursor,
    ) -> Option<Self::MaybeToken> {
        let (token, _) = this.tokens.get(*cursor)?;
        *cursor += 1;
        Some(token.clone())
    }

    unsafe fn span(this: &mut Self::Cache, range: Range<&Self::Cursor>) -> Self::Span {
        this.byte_span(*range.start, *range.end)
    }
}

impl<'src> ValueInput<'src> for Tokens<'src> {
    unsafe fn next(this: &mut Self::Cache, cursor: &mut Self::Cursor) -> Option<Self::Token> {
        Self::next_maybe(this, cursor)
    }
}

impl<'src> ExactSizeInput<'src> for Tokens<'src> {
    unsafe fn span_from(this: &mut Self::Cache, range: RangeFrom<&Self::Cursor>) -> Self::Span {
        let start_byte = match this.tokens.get(*range.start) {
            Some((_, span)) => span.start,
            None => this.eof_offset(),
        };
        (start_byte..this.eof_offset()).into()
    }
}

type MoxError<'src> = Rich<'src, Token<'src>, Span>;
type MoxExtra<'src> = extra::Err<MoxError<'src>>;

/// A parse error with a span and a rendered message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message} at byte offsets {span}")]
pub struct ParseError {
    /// Human-readable description of the error.
    pub message: String,
    /// Byte range the error refers to.
    pub span: Span,
}

/// The outcome of parsing: as much of the AST as could be recovered, plus all
/// encountered errors. Parsing never panics and always produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseResult {
    /// The recovered model, or `None` if no output could be produced at all.
    pub ast: Option<Model>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

fn kw<'src>(token: Token<'src>) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { t = e if t == token => e.span() }
}

/// Consumes a run of tokens that cannot start or continue a feature,
/// stopping before the class body's closing `}` and before tokens that could
/// begin the next feature. Used as a recovery alternative: it always consumes
/// at least one token (guaranteeing progress) and emits an error pointing at
/// the skipped region.
fn junk_feature<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
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

fn name<'src>() -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    select! {
        Token::Ident(text) = e => Name { text: text.to_string(), span: e.span(), escaped: false },
        Token::IdentEscaped(text) = e => Name { text: text.to_string(), span: e.span(), escaped: true },
    }
}

fn string_lit<'src>() -> impl Parser<'src, Tokens<'src>, String, MoxExtra<'src>> + Clone {
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

fn int_lit<'src>() -> impl Parser<'src, Tokens<'src>, i64, MoxExtra<'src>> + Clone {
    select! { Token::Int(value) => value }
}

/// A qualified-name segment: an identifier or — exceptionally — a keyword
/// token (e.g. the trailing `actors` in `package rex.conformance.actors`).
/// Keywords only introduce grammar constructs in statement position; inside
/// a dotted reference they are just names. Escaped forms are handled by
/// [`name`].
fn qname_segment<'src>() -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
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

fn qname<'src>() -> impl Parser<'src, Tokens<'src>, QualifiedName, MoxExtra<'src>> + Clone {
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

fn tref<'src>() -> impl Parser<'src, Tokens<'src>, TypeRef, MoxExtra<'src>> + Clone {
    qname().map_with(|name, e| TypeRef {
        name,
        span: e.span(),
    })
}

fn multiplicity<'src>() -> impl Parser<'src, Tokens<'src>, Multiplicity, MoxExtra<'src>> + Clone {
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

fn default_value<'src>() -> impl Parser<'src, Tokens<'src>, DefaultValue, MoxExtra<'src>> + Clone {
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

fn opposite<'src>() -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    kw(Token::Opposite).ignore_then(name())
}

/// Parses the contextual `id`/`readonly` modifiers that may precede any
/// feature. Both keywords lex as ordinary identifiers; they act as modifiers
/// only in this leading position, and the first token that is neither starts
/// the feature itself. Escaped forms (`^id`, `^readonly`) are different token
/// variants and are never modifiers. Repeats are idempotent.
fn modifiers<'src>() -> impl Parser<'src, Tokens<'src>, Modifiers, MoxExtra<'src>> + Clone {
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
fn raw_body<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
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
fn raw_parens<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
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
fn target_body<'src>() -> impl Parser<'src, Tokens<'src>, TargetBody, MoxExtra<'src>> + Clone {
    name()
        .then(raw_body())
        .map_with(|(target, span), _| TargetBody { target, span })
}

/// The body of an `op` declaration: either a bare balanced `{ ... }` block
/// (Tier 1 rejects it downstream) or one or more `<target> { ... }` blocks
/// inside an outer brace pair. Returns the outer span (when present) and the
/// target-tagged bodies (empty for a bare body).
fn op_body<'src>(
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

fn params<'src>() -> impl Parser<'src, Tokens<'src>, Vec<Param>, MoxExtra<'src>> + Clone {
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
const VALUED_CONSTRAINT_KEYWORDS: [&str; 5] =
    ["pattern", "minLength", "maxLength", "minimum", "maximum"];

/// One `keyword value` entry inside an attribute's constraint block.
/// Value-less `unique` takes no value: a literal after it is a syntax error
/// (with recovery that keeps the block parseable).
fn constraint_entry<'src>() -> impl Parser<'src, Tokens<'src>, Constraint, MoxExtra<'src>> + Clone {
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
fn constraint_block<'src>(
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

fn attribute<'src>() -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
    tref()
        .then(multiplicity().or_not())
        .then(name())
        .then(kw(Token::Eq).ignore_then(default_value()).or_not())
        .then(constraint_block().or_not())
        .map_with(
            |((((type_ref, multiplicity), name), default), constraints), e| {
                FeatureDecl::Attribute {
                    modifiers: Modifiers::default(),
                    doc: None,
                    type_ref,
                    multiplicity,
                    name,
                    default,
                    constraints: constraints.unwrap_or_default(),
                    span: e.span(),
                }
            },
        )
}

fn containment<'src>() -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
    kw(Token::Contains)
        .ignore_then(tref())
        .then(multiplicity().or_not())
        .then(name())
        .then(opposite().or_not())
        .map_with(
            |(((type_ref, multiplicity), name), opposite), e| FeatureDecl::Containment {
                modifiers: Modifiers::default(),
                doc: None,
                type_ref,
                multiplicity,
                name,
                opposite,
                span: e.span(),
            },
        )
}

fn reference<'src>() -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
    kw(Token::Refers)
        .ignore_then(tref())
        .then(multiplicity().or_not())
        .then(name())
        .then(opposite().or_not())
        .map_with(
            |(((type_ref, multiplicity), name), opposite), e| FeatureDecl::Reference {
                modifiers: Modifiers::default(),
                doc: None,
                type_ref,
                multiplicity,
                name,
                opposite,
                span: e.span(),
            },
        )
}

fn container<'src>() -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
    kw(Token::Container)
        .ignore_then(tref())
        .then(name())
        .then(opposite().or_not())
        .map_with(|((type_ref, name), opposite), e| FeatureDecl::Container {
            modifiers: Modifiers::default(),
            doc: None,
            type_ref,
            name,
            opposite,
            span: e.span(),
        })
}

fn op_decl<'src>() -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
    kw(Token::Op)
        .ignore_then(tref())
        .then(name())
        .then(params())
        .then(op_body())
        .map_with(
            |(((return_type, name), params), (body, bodies)), e| FeatureDecl::Op {
                modifiers: Modifiers::default(),
                doc: None,
                return_type,
                name,
                params,
                body,
                bodies,
                span: e.span(),
            },
        )
}

fn derived_decl<'src>() -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
    kw(Token::Derived)
        .ignore_then(tref())
        .then(multiplicity().or_not())
        .then(name())
        // Tier 2: a derived body is the same shape as an operation body —
        // target-tagged blocks (the neutral expression lives in a single
        // `expr { ... }` block) or a bare `{ ... }` the driver rejects.
        .then(op_body())
        .map_with(
            |(((type_ref, multiplicity), name), (body, bodies)), e| FeatureDecl::Derived {
                modifiers: Modifiers::default(),
                doc: None,
                type_ref,
                multiplicity,
                name,
                body,
                bodies,
                span: e.span(),
            },
        )
}

fn feature<'src>() -> impl Parser<'src, Tokens<'src>, Option<FeatureDecl>, MoxExtra<'src>> + Clone {
    let any_feature = modifiers()
        .then(choice((
            containment(),
            reference(),
            container(),
            op_decl(),
            derived_decl(),
            attribute(),
        )))
        .map_with(|(modifiers, mut feature), e| {
            feature.set_modifiers(modifiers);
            // The whole-declaration span includes the leading modifiers.
            feature.set_span(e.span());
            feature
        })
        .map(Some);
    any_feature.or(junk_feature().to(None))
}

fn package_decl<'src>() -> impl Parser<'src, Tokens<'src>, PackageDecl, MoxExtra<'src>> + Clone {
    kw(Token::Package)
        .ignore_then(qname())
        .map_with(|name, e| PackageDecl {
            name,
            doc: None,
            span: e.span(),
        })
}

fn annotation_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    kw(Token::Annotation)
        .ignore_then(string_lit())
        .then(kw(Token::As).ignore_then(name()).or_not())
        .map_with(|(value, name), e| {
            Decl::Annotation(AnnotationDecl {
                value,
                name,
                span: e.span(),
            })
        })
}

fn class_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    let extends = {
        let rest = kw(Token::Comma)
            .ignore_then(tref())
            .repeated()
            .collect::<Vec<_>>();
        kw(Token::Extends)
            .ignore_then(tref())
            .then(rest)
            .map(|(first, rest)| once(first).chain(rest).collect::<Vec<_>>())
    };
    kw(Token::Class)
        .ignore_then(name())
        .then(extends.or_not())
        .then_ignore(kw(Token::LBrace))
        .then(feature().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|((name, extends), features), e| {
            Decl::Class(ClassDecl {
                name,
                doc: None,
                extends: extends.unwrap_or_default(),
                features: features.into_iter().flatten().collect(),
                span: e.span(),
            })
        })
}

fn binding_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<BindingEntry>, MoxExtra<'src>> + Clone {
    kw(Token::LBrace)
        .ignore_then(binding_entry().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
}

fn binding_entry<'src>() -> impl Parser<'src, Tokens<'src>, BindingEntry, MoxExtra<'src>> + Clone {
    name()
        .then(string_lit())
        .map_with(|(key, value), e| BindingEntry {
            key,
            value,
            span: e.span(),
        })
}

/// A `create { <target-body>* }` or `convert { <target-body>* }` block named
/// by a contextual keyword (both lex as ordinary identifiers).
fn named_target_block<'src>(
    keyword: &'static str,
) -> impl Parser<'src, Tokens<'src>, Vec<TargetBody>, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) if text == keyword => () }
        .ignore_then(kw(Token::LBrace))
        .ignore_then(target_body().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
}

/// The reserved `format "…"` entry of a datatype's `{ ... }` block: the
/// unescaped key `format` followed by a string literal. It never creates a
/// target binding.
fn format_entry<'src>() -> impl Parser<'src, Tokens<'src>, String, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) if text == "format" => () }.ignore_then(string_lit())
}

/// One entry of a datatype's `{ ... }` block.
enum DatatypeEntry {
    Binding(BindingEntry),
    Format(String),
    Create(Vec<TargetBody>),
    Convert(Vec<TargetBody>),
}

/// The `{ ... }` block of a datatype: target-binding entries, the (at most
/// one) reserved `format "…"` entry, and (at most one each) `create`/`convert`
/// body blocks, in any order. A second `create` (or `convert`, or `format`)
/// is a syntax error, as is a binding target literally named `format` (only
/// writable escaped as `^format` — the unescaped key declares the format).
fn datatype_block<'src>() -> impl Parser<
    'src,
    Tokens<'src>,
    (
        Vec<BindingEntry>,
        Option<String>,
        Vec<TargetBody>,
        Vec<TargetBody>,
    ),
    MoxExtra<'src>,
> + Clone {
    let entry = choice((
        named_target_block("create").map(DatatypeEntry::Create),
        named_target_block("convert").map(DatatypeEntry::Convert),
        format_entry().map(DatatypeEntry::Format),
        binding_entry().map(DatatypeEntry::Binding),
    ));
    kw(Token::LBrace)
        .ignore_then(
            entry
                .repeated()
                .collect::<Vec<_>>()
                .map(|entries| {
                    let mut bindings = Vec::new();
                    let mut format: Option<String> = None;
                    let mut create: Option<Vec<TargetBody>> = None;
                    let mut convert: Option<Vec<TargetBody>> = None;
                    // `Some(keyword)` when a second block/entry of that kind appears.
                    let mut duplicate: Option<&'static str> = None;
                    for entry in entries {
                        match entry {
                            DatatypeEntry::Binding(binding) => bindings.push(binding),
                            DatatypeEntry::Format(value) if format.is_none() => {
                                format = Some(value)
                            }
                            DatatypeEntry::Format(_) if duplicate.is_none() => {
                                duplicate = Some("format")
                            }
                            DatatypeEntry::Format(_) => {}
                            DatatypeEntry::Create(bodies) if create.is_none() => {
                                create = Some(bodies)
                            }
                            DatatypeEntry::Create(_) if duplicate.is_none() => {
                                duplicate = Some("create")
                            }
                            DatatypeEntry::Create(_) => {}
                            DatatypeEntry::Convert(bodies) if convert.is_none() => {
                                convert = Some(bodies)
                            }
                            DatatypeEntry::Convert(_) if duplicate.is_none() => {
                                duplicate = Some("convert")
                            }
                            DatatypeEntry::Convert(_) => {}
                        }
                    }
                    (
                        (
                            bindings,
                            format,
                            create.unwrap_or_default(),
                            convert.unwrap_or_default(),
                        ),
                        duplicate,
                    )
                })
                .validate(|(block, duplicate), e, emitter| {
                    if let Some(keyword) = duplicate {
                        let noun = if keyword == "format" { "entry" } else { "block" };
                        emitter.emit(Rich::custom(
                            e.span(),
                            format!("duplicate `{keyword}` {noun} in datatype declaration"),
                        ));
                    }
                    for binding in &block.0 {
                        if binding.key.text == "format" {
                            emitter.emit(Rich::custom(
                                binding.key.span,
                                "`format` is a reserved key in datatype declarations: the unescaped key declares the datatype's format (`format \"…\"`)".to_string(),
                            ));
                        }
                    }
                    block
                }),
        )
        .then_ignore(kw(Token::RBrace))
}

fn interface_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    kw(Token::Interface)
        .ignore_then(name())
        .then(binding_block())
        .map_with(|(name, bindings), e| {
            Decl::Interface(InterfaceDecl {
                name,
                doc: None,
                bindings,
                span: e.span(),
            })
        })
}

fn enum_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    let literal = name()
        .then(kw(Token::As).ignore_then(string_lit()).or_not())
        .then(kw(Token::Eq).ignore_then(int_lit()).or_not())
        .map_with(|((name, label), value), e| EnumLiteral {
            name,
            doc: None,
            label,
            value,
            span: e.span(),
        });
    kw(Token::Enum)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(literal.repeated().at_least(1).collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, literals), e| {
            Decl::Enum(EnumDecl {
                name,
                doc: None,
                literals,
                span: e.span(),
            })
        })
}

fn datatype_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    let wraps_target = kw(Token::Opaque)
        .map(Wraps::Opaque)
        .or(qname().map(Wraps::Named));
    kw(Token::Type)
        .ignore_then(name())
        .then_ignore(kw(Token::Wraps))
        .then(wraps_target.or_not())
        .then(datatype_block().or_not())
        .map_with(|((name, wraps), block), e| {
            let (bindings, format, create, convert) = block.unwrap_or_default();
            Decl::Datatype(DatatypeDecl {
                name,
                doc: None,
                wraps,
                bindings,
                format,
                create,
                convert,
                span: e.span(),
            })
        })
}

/// One body item of a `vocabulary` declaration.
enum VocabItem {
    Version(String),
    Key(Name),
    Facet(VocabularyFacetDecl),
}

fn vocabulary_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    let facet_decl =
        kw(Token::Facet)
            .ignore_then(tref())
            .then(name())
            .map_with(|(type_ref, name), e| VocabularyFacetDecl {
                type_ref,
                name,
                span: e.span(),
            });
    let item = choice((
        kw(Token::Version)
            .ignore_then(string_lit())
            .map(VocabItem::Version),
        kw(Token::Key).ignore_then(name()).map(VocabItem::Key),
        facet_decl.map(VocabItem::Facet),
    ));
    kw(Token::Vocabulary)
        .ignore_then(name())
        .then_ignore(kw(Token::From))
        .then(string_lit())
        .then_ignore(kw(Token::LBrace))
        .then(item.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|((name, source), items), e| {
            let mut version = None;
            let mut key = None;
            let mut facets = Vec::new();
            for item in items {
                match item {
                    VocabItem::Version(value) => version = Some(value),
                    VocabItem::Key(key_name) => key = Some(key_name),
                    VocabItem::Facet(facet) => facets.push(facet),
                }
            }
            Decl::Vocabulary(VocabularyDecl {
                name,
                doc: None,
                source,
                version,
                key,
                facets,
                span: e.span(),
            })
        })
}

/// One body item of an `actors` declaration.
enum ActorsItem {
    Actor(ActorDecl),
    Capability(CapabilityDecl),
    Purpose(PurposeDecl),
    Grant(GrantDecl),
    Delegation(DelegationDecl),
    NeverBoth(NeverBothDecl),
}

/// One body item of a `delegation` declaration.
enum DelegationItem {
    From(Name),
    To(Name),
    Purpose(Name),
    Entry(GrantEntryDecl),
}

/// The `actors <name> { ... }` block grammar, shared verbatim by the inline
/// `.mox` declaration (see [`actors_decl`]) and standalone `.actor` files.
fn actors_block<'src>() -> impl Parser<'src, Tokens<'src>, ActorsDecl, MoxExtra<'src>> + Clone {
    let actor_decl = kw(Token::Actor)
        .ignore_then(name())
        .then(kw(Token::Extends).ignore_then(name()).or_not())
        .map_with(|(name, extends), e| ActorDecl {
            kind: ActorKind::Human,
            name,
            extends,
            span: e.span(),
        });
    let agent_decl = kw(Token::Agent)
        .ignore_then(name())
        .then(kw(Token::Extends).ignore_then(name()).or_not())
        .map_with(|(name, extends), e| ActorDecl {
            kind: ActorKind::Agent,
            name,
            extends,
            span: e.span(),
        });
    let capability_decl = kw(Token::Capability)
        .ignore_then(name())
        .then_ignore(kw(Token::On))
        .then(tref())
        .map_with(|(name, class), e| CapabilityDecl {
            name,
            class,
            span: e.span(),
        });
    let purpose_decl = kw(Token::Purpose)
        .ignore_then(name())
        .map_with(|name, e| PurposeDecl {
            name,
            span: e.span(),
        });
    let never_both_decl = kw(Token::NeverBoth)
        .ignore_then(kw(Token::LBrace))
        .ignore_then(
            name()
                .then(
                    kw(Token::Comma)
                        .ignore_then(name())
                        .repeated()
                        .collect::<Vec<_>>(),
                )
                .map(|(first, rest)| once(first).chain(rest).collect::<Vec<_>>()),
        )
        .then_ignore(kw(Token::RBrace))
        .map_with(|capabilities, e| NeverBothDecl {
            capabilities,
            span: e.span(),
        })
        .validate(|decl, e, emitter| {
            if decl.capabilities.len() < 2 {
                emitter.emit(Rich::custom(
                    e.span(),
                    "`never_both` requires at least two capability names",
                ));
            }
            decl
        });
    let obligation = kw(Token::Obligation).ignore_then(name());
    let effect = kw(Token::Permit)
        .to(Effect::Permit)
        .or(kw(Token::Forbid).to(Effect::Forbid));
    let effect_entry = effect
        .then(name())
        .then(kw(Token::When).ignore_then(raw_parens()).or_not())
        .then(obligation.repeated().collect::<Vec<_>>())
        .map_with(|(((effect, capability), when), obligations), e| {
            GrantEntryDecl::Effect(GrantEffectDecl {
                effect,
                capability,
                when,
                obligations,
                span: e.span(),
            })
        });
    let cedar_entry = kw(Token::Cedar)
        .then(raw_body())
        .map_with(|(target_span, span), _| {
            GrantEntryDecl::Cedar(TargetBody {
                target: Name {
                    text: "cedar".to_string(),
                    span: target_span,
                    escaped: false,
                },
                span,
            })
        });
    let entry = choice((effect_entry, cedar_entry));
    let grant_decl = kw(Token::Grant)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(entry.clone().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(actor, entries), e| GrantDecl {
            actor,
            entries,
            span: e.span(),
        });
    let from_item = kw(Token::From)
        .ignore_then(name())
        .map(DelegationItem::From);
    let to_item = select! { Token::Ident(text) if text == "to" => () }
        .ignore_then(name())
        .map(DelegationItem::To);
    let purpose_item = kw(Token::Purpose)
        .ignore_then(name())
        .map(DelegationItem::Purpose);
    let delegation_item = choice((
        from_item,
        to_item,
        purpose_item,
        entry.map(DelegationItem::Entry),
    ));
    let delegation_decl = kw(Token::Delegation)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(delegation_item.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, items), e| {
            let (mut decl, problems) = fold_delegation(name, items);
            decl.span = e.span();
            (decl, problems)
        })
        .validate(|(decl, problems), _, emitter| {
            for (span, message) in problems {
                emitter.emit(Rich::custom(span, message));
            }
            decl
        });
    let item = choice((
        actor_decl.map(ActorsItem::Actor),
        agent_decl.map(ActorsItem::Actor),
        capability_decl.map(ActorsItem::Capability),
        purpose_decl.map(ActorsItem::Purpose),
        grant_decl.map(ActorsItem::Grant),
        delegation_decl.map(ActorsItem::Delegation),
        never_both_decl.map(ActorsItem::NeverBoth),
    ));
    kw(Token::Actors)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(item.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, items), e| {
            let mut actors = Vec::new();
            let mut capabilities = Vec::new();
            let mut purposes = Vec::new();
            let mut grants = Vec::new();
            let mut delegations = Vec::new();
            let mut never_both = Vec::new();
            for item in items {
                match item {
                    ActorsItem::Actor(decl) => actors.push(decl),
                    ActorsItem::Capability(decl) => capabilities.push(decl),
                    ActorsItem::Purpose(decl) => purposes.push(decl),
                    ActorsItem::Grant(decl) => grants.push(decl),
                    ActorsItem::Delegation(decl) => delegations.push(decl),
                    ActorsItem::NeverBoth(decl) => never_both.push(decl),
                }
            }
            ActorsDecl {
                name,
                actors,
                capabilities,
                purposes,
                grants,
                delegations,
                never_both,
                span: e.span(),
            }
        })
}

/// The inline `actors { ... }` declaration of a `.mox` source: the shared
/// [`actors_block`] grammar wrapped as a top-level declaration.
fn actors_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    actors_block().map(Decl::Actors)
}

/// Folds the body items of a delegation into a [`DelegationDecl`], enforcing
/// the required `from` → `to` → `purpose` ordering and rejecting `cedar`
/// entries. The returned problems are (span, message) pairs the caller emits
/// as errors; the AST recovers as much as possible regardless.
fn fold_delegation(
    name: Name,
    items: Vec<DelegationItem>,
) -> (DelegationDecl, Vec<(Span, String)>) {
    let mut from: Option<Name> = None;
    let mut to: Option<Name> = None;
    let mut purpose: Option<Name> = None;
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    // 0 = expecting `from`, 1 = expecting `to`, 2 = collecting entries,
    // 3 = collecting entries after the (optional) `purpose` line.
    let mut stage = 0;
    for item in items {
        match item {
            DelegationItem::From(actor) => {
                if stage == 0 {
                    from = Some(actor);
                    stage = 1;
                } else {
                    problems.push((actor.span, "duplicate `from` in delegation".to_string()));
                }
            }
            DelegationItem::To(actor) => match stage {
                1 => {
                    to = Some(actor);
                    stage = 2;
                }
                0 => problems.push((actor.span, "delegation requires `from`".to_string())),
                _ => problems.push((actor.span, "duplicate `to` in delegation".to_string())),
            },
            DelegationItem::Purpose(name) => match stage {
                2 => {
                    purpose = Some(name);
                    stage = 3;
                }
                0 => problems.push((name.span, "delegation requires `from`".to_string())),
                1 => problems.push((name.span, "delegation requires `to`".to_string())),
                _ => problems.push((name.span, "duplicate `purpose` in delegation".to_string())),
            },
            DelegationItem::Entry(GrantEntryDecl::Cedar(body)) => {
                problems.push((
                    body.target.span,
                    "`cedar` entries are not allowed inside a delegation".to_string(),
                ));
            }
            DelegationItem::Entry(GrantEntryDecl::Effect(effect)) => {
                match stage {
                    0 => problems.push((effect.span, "delegation requires `from`".to_string())),
                    1 => problems.push((effect.span, "delegation requires `to`".to_string())),
                    _ => {}
                }
                entries.push(effect);
            }
        }
    }
    match stage {
        0 => problems.push((name.span, "delegation requires `from`".to_string())),
        1 => problems.push((name.span, "delegation requires `to`".to_string())),
        _ => {}
    }
    let missing = Name {
        text: String::new(),
        span: name.span,
        escaped: false,
    };
    (
        DelegationDecl {
            name,
            from: from.unwrap_or_else(|| missing.clone()),
            to: to.unwrap_or_else(|| missing.clone()),
            purpose,
            entries,
            span: (0..0).into(),
        },
        problems,
    )
}

/// An `import "path"` declaration of an `.actor` file. The path is a string
/// literal with the standard escapes.
fn import_decl<'src>() -> impl Parser<'src, Tokens<'src>, ImportDecl, MoxExtra<'src>> + Clone {
    kw(Token::Import)
        .ignore_then(string_lit())
        .map_with(|path, e| ImportDecl {
            path,
            span: e.span(),
        })
}

/// The contextual `schema` word of an `import schema` declaration: an
/// ordinary identifier that is special only directly after the `import`
/// keyword of a `.mox` file (the `format`-entry precedent). Escaped
/// `^schema` never matches.
fn schema_keyword<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == "schema" => e.span() }
}

/// An `import schema "<path>" (as <name>)?` declaration of a `.mox` source.
/// Once the contextual `schema` word matched, the declaration is committed:
/// a missing path literal or a missing alias name is a syntax error, but the
/// AST still recovers a declaration so the surrounding model parses on.
/// `import` without `schema` is not this declaration (the alternative fails
/// and declaration-level recovery reports the region).
fn import_schema_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    kw(Token::Import)
        .ignore_then(schema_keyword())
        .then(string_lit().or_not())
        .then(
            kw(Token::As)
                .or_not()
                .then(name().or_not())
                .map(|(as_kw, alias)| (as_kw.is_some(), alias)),
        )
        .map_with(|((schema_span, path), (has_as, alias)), e| {
            (schema_span, path, has_as, alias, e.span())
        })
        .validate(|(schema_span, path, has_as, alias, span), _e, emitter| {
            if path.is_none() {
                emitter.emit(Rich::custom(
                    schema_span,
                    "`import schema` requires a string literal path",
                ));
            }
            if has_as && alias.is_none() {
                emitter.emit(Rich::custom(schema_span, "expected a name after `as`"));
            }
            Decl::ImportSchema(ImportSchemaDecl {
                path: path.unwrap_or_default(),
                alias,
                span,
            })
        })
}

#[derive(Clone)]
enum Item {
    Package(PackageDecl),
    Decl(Decl),
    Junk,
}

/// Consumes a run of tokens up to the next top-level declaration keyword
/// (or end of input). Declaration-level recovery: always consumes at least
/// one token (guaranteeing progress) and emits an error for the skipped
/// region.
fn junk_decl<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let rest = select! {
        t if !matches!(
            t,
            Token::Package | Token::Annotation | Token::Class | Token::Interface | Token::Enum | Token::Type | Token::Vocabulary | Token::Actors | Token::Import
        ) =>
        ()
    };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected a declaration"));
        })
}

fn fold_model(items: Vec<Item>) -> Model {
    let mut package = None;
    let mut declarations = Vec::new();
    for item in items {
        match item {
            Item::Package(decl) if package.is_none() => package = Some(decl),
            Item::Package(_) | Item::Junk => {}
            Item::Decl(decl) => declarations.push(decl),
        }
    }
    Model {
        package,
        declarations,
    }
}

fn model<'src>() -> impl Parser<'src, Tokens<'src>, Model, MoxExtra<'src>> + Clone {
    let item = choice((
        package_decl().map(Item::Package),
        annotation_decl().map(Item::Decl),
        class_decl().map(Item::Decl),
        interface_decl().map(Item::Decl),
        enum_decl().map(Item::Decl),
        datatype_decl().map(Item::Decl),
        vocabulary_decl().map(Item::Decl),
        actors_decl().map(Item::Decl),
        import_schema_decl().map(Item::Decl),
    ))
    .or(junk_decl().to(Item::Junk));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map(fold_model)
}

/// Lex and parse a `.mox` source text.
///
/// This function never panics and always recovers as much of the AST as
/// possible; check [`ParseResult::errors`] for syntax problems. Doc comments
/// (`///`, `/** ... */`) on their own lines directly above the package
/// declaration, a declaration, feature, or enum literal are attached to it as
/// its `doc` description.
pub fn parse(source: &str) -> ParseResult {
    let (tokens, comments) = match lex_with_comments(source) {
        Ok(result) => result,
        Err(error) => {
            return ParseResult {
                ast: None,
                errors: vec![ParseError {
                    message: error.to_string(),
                    span: error.span,
                }],
            }
        }
    };
    let (mut ast, errors) = model().parse(Tokens::new(&tokens)).into_output_errors();
    if let Some(model) = ast.as_mut() {
        attach_docs(model, &comments, source);
    }
    ParseResult {
        ast,
        errors: errors
            .into_iter()
            .map(|error| ParseError {
                message: error.to_string(),
                span: *error.span(),
            })
            .collect(),
    }
}

/// One precomputed comment fact used by doc attachment.
struct DocComment {
    content: Option<String>,
    start_line: usize,
    end_line: usize,
    /// Whether only whitespace precedes the comment on its first line.
    begins_line: bool,
}

/// Attaches doc comments to the model's package declaration, declarations,
/// features, and enum literals. A doc run is a maximal sequence of doc
/// comments, each on its own line, each starting on the line directly after
/// the previous one ends, whose last comment ends on the line directly above
/// the declaration's first line.
fn attach_docs(model: &mut Model, comments: &[crate::lexer::Comment<'_>], source: &str) {
    let line_starts: Vec<usize> = {
        let mut starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        starts
    };
    let line_of = |byte: usize| -> usize {
        line_starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    };
    let docs: Vec<DocComment> = comments
        .iter()
        .map(|comment| DocComment {
            content: comment.doc_content().filter(|content| !content.is_empty()),
            start_line: line_of(comment.span.start),
            end_line: line_of(comment.span.end.saturating_sub(1)),
            begins_line: source[line_starts[line_of(comment.span.start)]..comment.span.start]
                .bytes()
                .all(|byte| byte.is_ascii_whitespace()),
        })
        .collect();

    /// The joined description of the doc run ending directly above
    /// `start_line`, or `None`.
    fn run_above(docs: &[DocComment], start_line: usize) -> Option<String> {
        let (last_index, _) =
            docs.iter().enumerate().rev().find(|(_, comment)| {
                comment.end_line + 1 == start_line && comment.content.is_some()
            })?;
        if !docs[last_index].begins_line {
            return None;
        }
        // Walk back through the contiguous doc run above the last comment.
        let mut first = last_index;
        while first > 0 {
            let previous = &docs[first - 1];
            let current = &docs[first];
            let contiguous = previous.end_line + 1 == current.start_line;
            if !(contiguous && previous.content.is_some() && previous.begins_line) {
                break;
            }
            first -= 1;
        }
        let joined = docs[first..=last_index]
            .iter()
            .map(|comment| {
                comment
                    .content
                    .as_deref()
                    .expect("run members are doc comments")
            })
            .collect::<Vec<_>>()
            .join("\n");
        (!joined.is_empty()).then_some(joined)
    }

    if let Some(package) = &mut model.package {
        let start_line = line_of(package.span.start);
        package.doc = run_above(&docs, start_line);
    }

    for decl in &mut model.declarations {
        let start_line = line_of(decl.span().start);
        match decl {
            Decl::Class(decl) => {
                decl.doc = run_above(&docs, start_line);
                for feature in &mut decl.features {
                    let feature_line = line_of(feature.span().start);
                    if feature.doc().is_none() {
                        feature.set_doc(run_above(&docs, feature_line));
                    }
                }
            }
            Decl::Interface(decl) => decl.doc = run_above(&docs, start_line),
            Decl::Enum(decl) => {
                decl.doc = run_above(&docs, start_line);
                for literal in &mut decl.literals {
                    let literal_line = line_of(literal.span.start);
                    if literal.doc.is_none() {
                        literal.doc = run_above(&docs, literal_line);
                    }
                }
            }
            Decl::Datatype(decl) => decl.doc = run_above(&docs, start_line),
            Decl::Vocabulary(decl) => decl.doc = run_above(&docs, start_line),
            Decl::Annotation(_) | Decl::Actors(_) | Decl::ImportSchema(_) => {}
        }
    }
}

#[derive(Clone)]
enum ActorFileItem {
    Import(ImportDecl),
    Block(ActorsDecl),
    Junk,
}

/// Consumes a run of tokens up to the next `import`/`actors` keyword (or end
/// of input). Declaration-level recovery for `.actor` files: always consumes
/// at least one token (guaranteeing progress) and emits an error for the
/// skipped region.
fn junk_actor_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let rest = select! { t if !matches!(t, Token::Import | Token::Actors) => () };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected an import or actors block"));
        })
}

fn fold_actor_file(items: Vec<ActorFileItem>) -> (ActorFile, Option<Span>) {
    let mut imports = Vec::new();
    let mut blocks = Vec::new();
    // Span of the first `import` that follows an actors block (a syntax
    // error reported by the caller's `validate`).
    let mut late_import = None;
    for item in items {
        match item {
            ActorFileItem::Import(decl) => {
                if blocks.is_empty() {
                    imports.push(decl);
                } else if late_import.is_none() {
                    late_import = Some(decl.span);
                }
            }
            ActorFileItem::Block(decl) => blocks.push(decl),
            ActorFileItem::Junk => {}
        }
    }
    (ActorFile { imports, blocks }, late_import)
}

fn actor_file<'src>() -> impl Parser<'src, Tokens<'src>, ActorFile, MoxExtra<'src>> + Clone {
    let item = choice((
        import_decl().map(ActorFileItem::Import),
        actors_block().map(ActorFileItem::Block),
    ))
    .or(junk_actor_file().to(ActorFileItem::Junk));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map(fold_actor_file)
        .validate(|(file, late_import), _, emitter| {
            if let Some(span) = late_import {
                emitter.emit(Rich::custom(span, "`import` after an actors block"));
            }
            file
        })
}

/// The outcome of parsing an `.actor` source: as much of the file as could be
/// recovered, plus all encountered errors. Parsing never panics and always
/// produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorsParseResult {
    /// The recovered actor file, or `None` if no output could be produced at all.
    pub ast: Option<ActorFile>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

/// Lex and parse an `.actor` source text: `import` declarations followed by
/// `actors` blocks, the block grammar being identical to the inline actors
/// declarations of `.mox` sources.
///
/// This function never panics and always recovers as much of the AST as
/// possible; check [`ActorsParseResult::errors`] for syntax problems. An
/// `import` after an actors block is a syntax error; empty import and block
/// lists are legal, and so are duplicate imports — those are semantic
/// questions for the driver.
pub fn parse_actors(source: &str) -> ActorsParseResult {
    let tokens = match lex(source) {
        Ok(tokens) => tokens,
        Err(error) => {
            return ActorsParseResult {
                ast: None,
                errors: vec![ParseError {
                    message: error.to_string(),
                    span: error.span,
                }],
            }
        }
    };
    let (ast, errors) = actor_file()
        .parse(Tokens::new(&tokens))
        .into_output_errors();
    ActorsParseResult {
        ast,
        errors: errors
            .into_iter()
            .map(|error| ParseError {
                message: error.to_string(),
                span: *error.span(),
            })
            .collect(),
    }
}

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
use crate::lexer::{lex, Token};

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

fn qname<'src>() -> impl Parser<'src, Tokens<'src>, QualifiedName, MoxExtra<'src>> + Clone {
    name()
        .then(
            kw(Token::Dot)
                .ignore_then(name())
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

fn attribute<'src>() -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
    tref()
        .then(multiplicity().or_not())
        .then(name())
        .then(kw(Token::Eq).ignore_then(default_value()).or_not())
        .map_with(
            |(((type_ref, multiplicity), name), default), e| FeatureDecl::Attribute {
                modifiers: Modifiers::default(),
                type_ref,
                multiplicity,
                name,
                default,
                span: e.span(),
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
        .then(raw_body().or_not())
        .map_with(
            |(((type_ref, multiplicity), name), body), e| FeatureDecl::Derived {
                modifiers: Modifiers::default(),
                type_ref,
                multiplicity,
                name,
                body,
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

fn package_decl<'src>() -> impl Parser<'src, Tokens<'src>, QualifiedName, MoxExtra<'src>> + Clone {
    kw(Token::Package).ignore_then(qname())
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

/// One entry of a datatype's `{ ... }` block.
enum DatatypeEntry {
    Binding(BindingEntry),
    Create(Vec<TargetBody>),
    Convert(Vec<TargetBody>),
}

/// The `{ ... }` block of a datatype: target-binding entries and (at most one
/// each) `create`/`convert` body blocks, in any order. A second `create` (or
/// `convert`) block is a syntax error.
fn datatype_block<'src>() -> impl Parser<
    'src,
    Tokens<'src>,
    (Vec<BindingEntry>, Vec<TargetBody>, Vec<TargetBody>),
    MoxExtra<'src>,
> + Clone {
    let entry = choice((
        named_target_block("create").map(DatatypeEntry::Create),
        named_target_block("convert").map(DatatypeEntry::Convert),
        binding_entry().map(DatatypeEntry::Binding),
    ));
    kw(Token::LBrace)
        .ignore_then(
            entry
                .repeated()
                .collect::<Vec<_>>()
                .map(|entries| {
                    let mut bindings = Vec::new();
                    let mut create: Option<Vec<TargetBody>> = None;
                    let mut convert: Option<Vec<TargetBody>> = None;
                    // `Some(keyword)` when a second block of that kind appears.
                    let mut duplicate: Option<&'static str> = None;
                    for entry in entries {
                        match entry {
                            DatatypeEntry::Binding(binding) => bindings.push(binding),
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
                            create.unwrap_or_default(),
                            convert.unwrap_or_default(),
                        ),
                        duplicate,
                    )
                })
                .validate(|(block, duplicate), e, emitter| {
                    if let Some(keyword) = duplicate {
                        emitter.emit(Rich::custom(
                            e.span(),
                            format!("duplicate `{keyword}` block in datatype declaration"),
                        ));
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
            let (bindings, create, convert) = block.unwrap_or_default();
            Decl::Datatype(DatatypeDecl {
                name,
                wraps,
                bindings,
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
                source,
                version,
                key,
                facets,
                span: e.span(),
            })
        })
}

#[derive(Clone)]
enum Item {
    Package(QualifiedName),
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
            Token::Package | Token::Annotation | Token::Class | Token::Interface | Token::Enum | Token::Type | Token::Vocabulary
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
            Item::Package(qualified) if package.is_none() => package = Some(qualified),
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
/// possible; check [`ParseResult::errors`] for syntax problems.
pub fn parse(source: &str) -> ParseResult {
    let tokens = match lex(source) {
        Ok(tokens) => tokens,
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
    let (ast, errors) = model().parse(Tokens::new(&tokens)).into_output_errors();
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

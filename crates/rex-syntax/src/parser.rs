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
//!
//! # The normative grammar
//!
//! This module is the **grammar authority** for the three chumsky surfaces:
//! the EBNF below is derived from the parsers in this file, and the docs
//! (`docs/LANGUAGE.md`, `docs/DDD.md`) link here instead of restating it.
//! When any prose and this EBNF disagree, the parsers in this file win;
//! change both in the same commit. The grammar of the fourth surface,
//! `.ifml`, is normatively defined by its Pest grammar file
//! (`crates/rex-ifml/src/grammar/ifml.pest`), which is machine-checked at
//! build time — it is not duplicated here.
//!
//! The unit tests at the bottom of this file spot-check the EBNF against the
//! lexer and the `.ddd` keyword tables (every real keyword must appear as a
//! terminal, every terminal must be a known keyword or contextual word), so
//! adding or renaming a keyword without updating the grammar fails a test.
//! Structural drift (a changed repetition or optionality) is *not* caught
//! mechanically; keep this block in the parser's change.
//!
//! Notation shared by all three surfaces:
//!
//! * Terminals are quoted (`"class"`); everything else is a metavariable.
//!   `name` is an identifier ([`Token::Ident`], or an escaped keyword,
//!   [`Token::IdentEscaped`]); `qualified_name` is `name ("." segment)*`
//!   where a *segment* may exceptionally be any keyword token — keywords
//!   introduce constructs only in statement position, never inside a dotted
//!   reference. `type_ref` is a `qualified_name`; `string` and `int` are the
//!   string and integer literals.
//! * Real keywords are fixed lexer tokens, escapable with `^` — the list is
//!   the [`Token`] enum. *Contextual* words are ordinary identifiers that are
//!   special only in the grammar position noted: `id`/`readonly`
//!   (feature modifiers), `schema`/`sigil` (only directly after `import`),
//!   `to` (delegation line), `create`/`convert`/`format` (datatype blocks),
//!   the constraint keywords, and every `.ddd` word — the `.ddd` word tables
//!   in `crate::ddd` are that surface's keyword authority and are not
//!   restated here.
//! * Doc comments (`///`, `/** ... */`) attach to the element whose first
//!   line they end on, by position. They are not part of any production.
//! * `raw_body` is a balanced `{ ... }` scan whose contents are deliberately
//!   not parsed; `raw_parens` is the same over `(...)`. Both yield a span the
//!   driver slices (op bodies, `when` conditions, document expressions).
//! * `multiplicity` is `"[" (int (".." (int | "*"))?)? "]"`: `[]` is
//!   unbounded, `[n]` exact, `[n..m]` and `[n..*]` ranges.
//! * Expressions (`when` conditions, `expr` op bodies, `.ddd` document
//!   entries) are captured raw here and parsed/typed later by rex-expr; the
//!   expression grammar is normative in `docs/EXPRESSIONS.md` and is not
//!   duplicated below.
//!
//! ## `.mox` model sources ([`parse`])
//!
//! ```text
//! model            := item*
//! item             := package_decl | annotation_decl | import_schema_decl
//!                   | import_sigil_decl | class_decl | interface_decl
//!                   | enum_decl | datatype_decl | vocabulary_decl
//!                   | actors_block
//! package_decl     := "package" qualified_name
//! annotation_decl  := "annotation" string ("as" name)?
//! import_schema_decl := "import" "schema" string ("as" name)?
//! import_sigil_decl  := "import" "sigil" string
//! class_decl       := "class" name ("extends" type_ref ("," type_ref)*)?
//!                     "{" feature* "}"
//! feature          := modifier* ( containment | reference | container
//!                               | op_decl | derived_decl | attribute )
//! modifier         := "id" | "readonly"
//! containment      := "contains" type_ref multiplicity? name
//!                     ("opposite" name)?
//! reference        := "refers" type_ref multiplicity? name
//!                     ("opposite" name)?
//! container        := "container" type_ref name ("opposite" name)?
//! op_decl          := "op" type_ref name "(" params? ")" op_body?
//! derived_decl     := "derived" type_ref multiplicity? name op_body?
//! attribute        := type_ref multiplicity? name ("=" default)?
//!                     constraint_block?
//! params           := param ("," param)*
//! param            := type_ref name
//! default          := string | int | "true" | "false" | name
//! op_body          := "{" target_body+ "}" | raw_body
//! target_body      := name raw_body
//! constraint_block := "{" constraint_entry* "}"
//! constraint_entry := constraint_keyword (string | int) | "unique"
//! constraint_keyword := "pattern" | "minLength" | "maxLength"
//!                     | "minimum" | "maximum"
//! enum_decl        := "enum" name "{" literal+ "}"
//! literal          := name ("as" string)? ("=" int)?
//! datatype_decl    := "type" name "wraps" ("opaque" | qualified_name)?
//!                     datatype_block?
//! datatype_block   := "{" datatype_entry* "}"
//! datatype_entry   := binding_entry | "format" string
//!                   | "create" target_bodies | "convert" target_bodies
//! binding_entry    := name string
//! interface_decl   := "interface" name "{" binding_entry* "}"
//! vocabulary_decl  := "vocabulary" name "from" string "{" vocab_item* "}"
//! vocab_item       := "version" string | "key" name | "facet" type_ref name
//! ```
//!
//! Grammar-position notes the AST or driver enforces (a violated note is a
//! reported error, not a silent acceptance):
//!
//! * At most one `package` declaration is kept — the first; any later one is
//!   dropped silently. `annotation` declarations are top-level only.
//! * An `import schema` is committed once `"schema"` matched: a missing path
//!   or a missing name after `as` is an error (recovered, so the model
//!   parses on). `import sigil` likewise, and a present `as` clause is an
//!   error — a whole namespace set is imported, never one type.
//! * `op_body`'s bare `raw_body` alternative parses but is rejected by the
//!   driver; a `derived` body must be a single `expr` target body (the
//!   neutral expression language, `docs/EXPRESSIONS.md`).
//! * Constraint keywords each at most once per block; `unique` takes no
//!   value (a literal after it is an error) and is the collection-level
//!   constraint.
//! * In a `datatype_block`, `format`, `create`, and `convert` each appear at
//!   most once, and a binding target literally named `format` is an error —
//!   the unescaped key declares the format (only writable escaped,
//!   `^format`).
//!
//! ## The shared `actors_block`
//!
//! One grammar, two homes: the inline `.mox` declaration (as an `item`
//! above) and the standalone `.actor` file.
//!
//! ```text
//! actors_block    := "actors" name "{" actors_item* "}"
//! actors_item     := actor_decl | capability_decl | purpose_decl
//!                  | grant_decl | delegation_decl | never_both_decl
//! actor_decl      := ("actor" | "agent") name ("extends" name)?
//! capability_decl := "capability" name "on" type_ref
//! purpose_decl    := "purpose" name
//! grant_decl      := "grant" name "{" grant_entry* "}"
//! grant_entry     := effect_entry | cedar_entry
//! effect_entry    := ("permit" | "forbid") name ("when" raw_parens)?
//!                    obligation*
//! obligation      := "obligation" name
//! cedar_entry     := "cedar" raw_body
//! delegation_decl := "delegation" name "{" "from" name "to" name
//!                     ("purpose" name)? delegation_entry* "}"
//! delegation_entry := effect_entry
//! never_both_decl := "never_both" "{" name "," name ("," name)* "}"
//! ```
//!
//! * The body items may be interleaved in any order; the AST folds them into
//!   per-kind lists.
//! * The delegation shape above is the only *valid* one; the parser accepts
//!   any item order and reports misplaced/duplicate `from`/`to`/`purpose`
//!   lines and `cedar` entries (not allowed inside a delegation) as errors.
//! * `never_both` requires at least two capability names (an error,
//!   recovered).
//! * `when` conditions are `raw_parens` spans: the driver slices the
//!   condition text and type-checks it against the capability's class via
//!   rex-expr.
//!
//! ## `.actor` policy files ([`parse_actors`])
//!
//! ```text
//! actor_file  := actor_file_item*
//! actor_file_item := import_decl | actors_block
//! import_decl := "import" string
//! ```
//!
//! * An empty file is legal; duplicate imports are a driver concern, not a
//!   syntax error. An `import` after an `actors` block is an error
//!   (recovered): imports must precede every block.
//! * Unlike the `.ddd` import (below), an `.actor` import takes no trailing
//!   `;`.
//!
//! ## `.ddd` design sources ([`parse_ddd`])
//!
//! ```text
//! ddd_file          := ddd_file_item*
//! ddd_file_item     := ddd_import | application_decl
//! ddd_import        := "import" string ";"?
//! application_decl  := "application" name "{" application_item* "}"
//! application_item  := base_decl | module_decl
//! base_decl         := "base" qualified_name
//! module_decl       := "module" name "{" module_member* "}"
//! module_member     := service_decl | design_decl | search_decl
//! service_decl      := "service" name "{" service_member* "}"
//! service_member    := service_op | inject_decl
//! service_op        := signature capability_clause? ";"
//!                    | name "=>" qualified_name capability_clause? ";"
//! inject_decl       := "inject" name ";"
//! signature         := type_ref multiplicity? name
//!                      "(" (param ("," param)*)? ")"
//! param             := type_ref multiplicity? name
//! capability_clause := "capability" name ("," name)*
//! design_decl       := "abstract"? stereotype name design_flag*
//!                      repository_decl?
//! stereotype        := "entity" | "value" | "dto"
//! design_flag       := "scaffold" | "auditable" | "optimisticLocking"
//!                    | "nonPersistent" | "cache"
//! repository_decl   := "repository" name "{" repository_op* "}"
//! repository_op     := repository_builtin ";" | signature ";"
//! repository_builtin := "findById" | "findAll" | "save" | "delete"
//! search_decl       := "search" name "{" search_member* "}"
//! search_member     := "entity" qualified_name
//!                    | "text" "{" search_field* "}"
//!                    | "filters" "{" qualified_name* "}"
//!                    | "sort" "{" qualified_name* "}"
//!                    | "document" "{" document_entry* "}"
//!                    | "ranking" ("bm25" | "tfIdf" | "exact"
//!                                | "custom" string)
//!                    | "analyzer" string
//!                    | "pagination" "{" pagination_member* "}"
//!                    | "capability" name
//! search_field      := qualified_name ("boost" int)? ("analyzer" string)?
//! document_entry    := name "=" raw_expr ";"
//! pagination_member := "limit" int | "max" int | "cursor"
//! ```
//!
//! * Every `.ddd` word above is a **contextual identifier** (the `id`/
//!   `readonly` precedent), escapable with `^`; the tables in `crate::ddd`
//!   are the keyword authority, and the stereotype/flag/builtin/search-member
//!   alternatives follow the table orders (the formatter's canonical order).
//! * Imports must precede the `application` declaration and there is exactly
//!   one application per file (violations are reported errors); the
//!   delegated-operation `qualified_name` is `target.operation`, split at its
//!   last segment (a bare name recovers with an error).
//! * Declared repository operations take no `capability` clause (the wire
//!   artifact carries none, so accepting one would silently drop data); a
//!   delegated op may be named `save` etc. — contextual words.
//! * `design_flag`s and the single-member search lines (`entity`, `ranking`,
//!   `analyzer`, `pagination`) are idempotent on repeat — the first
//!   declaration wins; repeated `text`/`filters`/`sort`/`document` clauses
//!   merge.
//! * `base` must be the application's first member; a duplicate or late
//!   `base` is an error.
//! * `document_entry`'s `raw_expr` spans balanced `()`/`[]`/`{}` nesting up
//!   to the terminating `;` (string literals are single tokens), and is
//!   stored verbatim; parsing/typing it is the driver's job.

use std::iter::once;
use std::ops::{Range, RangeFrom};

use chumsky::input::{ExactSizeInput, ValueInput};
use chumsky::prelude::*;

use crate::ast::*;
use crate::ddd;
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
                kind: ImportKind::Schema,
                path: path.unwrap_or_default(),
                alias,
                span,
            })
        })
}

/// The contextual `sigil` word of an `import sigil` declaration: an
/// ordinary identifier that is special only directly after the `import`
/// keyword of a `.mox` file (the `schema`-entry precedent). Escaped
/// `^sigil` never matches.
fn sigil_keyword<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == "sigil" => e.span() }
}

/// An `import sigil "<path>"` declaration of a `.mox` source: a Rune DSL
/// (`.rosetta`) namespace set imported into the package's namespace. A whole
/// namespace set is imported, not one type, so the declaration takes no `as`
/// alias. Once the contextual `sigil` word matched, the declaration is
/// committed: a missing path literal or a present `as` clause is a syntax
/// error, but the AST still recovers a declaration so the surrounding model
/// parses on. `import` without `sigil` is not this declaration (the
/// alternative fails and declaration-level recovery reports the region).
fn import_sigil_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    kw(Token::Import)
        .ignore_then(sigil_keyword())
        .then(string_lit().or_not())
        .then(
            kw(Token::As)
                .map_with(|_, e| e.span())
                .or_not()
                .then(name().or_not()),
        )
        .map_with(|((sigil_span, path), (as_kw, _alias)), e| (sigil_span, path, as_kw, e.span()))
        .validate(|(sigil_span, path, as_kw, span), _e, emitter| {
            if path.is_none() {
                emitter.emit(Rich::custom(
                    sigil_span,
                    "`import sigil` requires a string literal path",
                ));
            }
            if let Some(as_span) = as_kw {
                emitter.emit(Rich::custom(
                    as_span,
                    "`import sigil` does not take an alias",
                ));
            }
            Decl::ImportSchema(ImportSchemaDecl {
                kind: ImportKind::Sigil,
                path: path.unwrap_or_default(),
                alias: None,
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
        import_sigil_decl().map(Item::Decl),
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

/// The byte offsets at which each line of `source` starts.
fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

/// The 0-based line containing the given byte offset.
fn line_of(byte: usize, starts: &[usize]) -> usize {
    starts
        .partition_point(|&start| start <= byte)
        .saturating_sub(1)
}

/// Precomputes the doc-comment facts of a comment list against its source.
fn collect_docs(comments: &[crate::lexer::Comment<'_>], source: &str) -> Vec<DocComment> {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    comments
        .iter()
        .map(|comment| DocComment {
            content: comment.doc_content().filter(|content| !content.is_empty()),
            start_line: line_of(comment.span.start),
            end_line: line_of(comment.span.end.saturating_sub(1)),
            begins_line: source[starts[line_of(comment.span.start)]..comment.span.start]
                .bytes()
                .all(|byte| byte.is_ascii_whitespace()),
        })
        .collect()
}

/// The joined description of the doc run ending directly above `start_line`,
/// or `None`. A doc run is a maximal sequence of doc comments, each on its
/// own line, each starting on the line directly after the previous one ends,
/// whose last comment ends on the line directly above the declaration's
/// first line.
fn doc_run_above(docs: &[DocComment], start_line: usize) -> Option<String> {
    let (last_index, _) = docs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, comment)| comment.end_line + 1 == start_line && comment.content.is_some())?;
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

/// Attaches doc comments to the model's package declaration, declarations,
/// features, and enum literals.
fn attach_docs(model: &mut Model, comments: &[crate::lexer::Comment<'_>], source: &str) {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    let docs = collect_docs(comments, source);

    if let Some(package) = &mut model.package {
        let start_line = line_of(package.span.start);
        package.doc = doc_run_above(&docs, start_line);
    }

    for decl in &mut model.declarations {
        let start_line = line_of(decl.span().start);
        match decl {
            Decl::Class(decl) => {
                decl.doc = doc_run_above(&docs, start_line);
                for feature in &mut decl.features {
                    let feature_line = line_of(feature.span().start);
                    if feature.doc().is_none() {
                        feature.set_doc(doc_run_above(&docs, feature_line));
                    }
                }
            }
            Decl::Interface(decl) => decl.doc = doc_run_above(&docs, start_line),
            Decl::Enum(decl) => {
                decl.doc = doc_run_above(&docs, start_line);
                for literal in &mut decl.literals {
                    let literal_line = line_of(literal.span.start);
                    if literal.doc.is_none() {
                        literal.doc = doc_run_above(&docs, literal_line);
                    }
                }
            }
            Decl::Datatype(decl) => decl.doc = doc_run_above(&docs, start_line),
            Decl::Vocabulary(decl) => decl.doc = doc_run_above(&docs, start_line),
            Decl::Annotation(_) | Decl::Actors(_) | Decl::ImportSchema(_) => {}
        }
    }
}

/// Attaches doc comments to a `.ddd` file's services and searches. The
/// contiguous `///` run is joined with newlines into a single description —
/// the same semantic as the `.mox` doc collection ([`doc_run_above`] is
/// shared; no re-joining happens here).
fn attach_ddd_docs(file: &mut DddFile, comments: &[crate::lexer::Comment<'_>], source: &str) {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    let docs = collect_docs(comments, source);
    if let Some(application) = &mut file.application {
        for module in &mut application.modules {
            for service in &mut module.services {
                let start_line = line_of(service.span.start);
                service.doc = doc_run_above(&docs, start_line);
            }
            for search in &mut module.searches {
                let start_line = line_of(search.span.start);
                search.doc = doc_run_above(&docs, start_line);
            }
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

// --- .ddd files ---------------------------------------------------------------

/// A contextual `.ddd` keyword: an ordinary identifier that is special only
/// in the grammar position it is parsed here in (the `schema`/`to`
/// precedent). Escaped forms (`^service`) are different token variants and
/// never match.
fn ddd_keyword<'src>(
    keyword: &'static str,
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == keyword => e.span() }
}

/// An `import "<path>"` declaration of a `.ddd` file. Unlike the `.actor`
/// production this tolerates a trailing `;`, which the canonical `.ddd`
/// formatting emits on every import line.
fn ddd_import_decl<'src>() -> impl Parser<'src, Tokens<'src>, ImportDecl, MoxExtra<'src>> + Clone {
    kw(Token::Import)
        .ignore_then(string_lit())
        .then_ignore(kw(Token::Other(';')).or_not())
        .map_with(|path, e| ImportDecl {
            path,
            span: e.span(),
        })
}

/// The `capability <name> (, <name>)*` clause of a service operation. The
/// clause is committed once the `capability` keyword matched: a missing name
/// list is a syntax error (with recovery), not a failed alternative.
fn ddd_capabilities<'src>() -> impl Parser<'src, Tokens<'src>, Vec<Name>, MoxExtra<'src>> + Clone {
    kw(Token::Capability)
        .ignore_then(name().or_not())
        .then(
            kw(Token::Comma)
                .ignore_then(name())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map_with(|(first, rest), e| (first, rest, e.span()))
        .validate(|(first, rest, span), _e, emitter| {
            if first.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "`capability` requires at least one capability name",
                ));
            }
            first.into_iter().chain(rest).collect::<Vec<_>>()
        })
}

/// A `type_ref multiplicity? name "(" params? ")"` operation signature,
/// shared verbatim by service and repository operations.
fn ddd_signature<'src>() -> impl Parser<
    'src,
    Tokens<'src>,
    (TypeRef, Option<Multiplicity>, Name, Vec<DddParam>),
    MoxExtra<'src>,
> + Clone {
    let param = tref().then(multiplicity().or_not()).then(name()).map_with(
        |((type_ref, multiplicity), name), e| DddParam {
            type_ref,
            multiplicity,
            name,
            span: e.span(),
        },
    );
    let list = param
        .clone()
        .then(
            kw(Token::Comma)
                .ignore_then(param)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| once(first).chain(rest).collect::<Vec<_>>());
    let params = kw(Token::LParen)
        .ignore_then(list.or_not())
        .then_ignore(kw(Token::RParen))
        .map(|list| list.unwrap_or_default());
    tref()
        .then(multiplicity().or_not())
        .then(name())
        .then(params)
        .map(|(((type_ref, multiplicity), name), params)| (type_ref, multiplicity, name, params))
}

/// The `<target>.<operation>` delegation target of a delegated service
/// operation. [`qname`] cannot know where the dependency name ends (a
/// following `.` might continue a package-qualified target), so the dotted
/// pair is parsed as one qualified name and split at its last segment. Once
/// the `=>` matched the operation is committed: a bare single-segment name
/// (no `.`) recovers with an empty target and a reported error.
fn ddd_delegation<'src>() -> impl Parser<'src, Tokens<'src>, DddDelegation, MoxExtra<'src>> + Clone
{
    qname().map_with(|qname, e| (qname, e.span())).validate(
        |(qname, span), _e, emitter| match qname.segments.split_last() {
            Some((operation, target_segments)) if !target_segments.is_empty() => DddDelegation {
                target: QualifiedName {
                    segments: target_segments.to_vec(),
                    span: (qname.span.start..operation.span.start).into(),
                },
                operation: operation.clone(),
                span: qname.span,
            },
            _ => {
                emitter.emit(Rich::custom(span, "expected `dependency.operation`"));
                DddDelegation {
                    target: QualifiedName {
                        segments: Vec::new(),
                        span: (span.end..span.end).into(),
                    },
                    operation: Name {
                        text: String::new(),
                        span,
                        escaped: false,
                    },
                    span,
                }
            }
        },
    )
}

/// One operation of a `service` body: a declared signature
/// (`type_ref multiplicity? name "(" params? ")" capability*? ";"`) or a
/// delegation (`name "=>" target.operation capability*? ";"`). The two
/// alternatives are distinguished by the `=>` after the leading name;
/// chumsky backtracks, so a delegated op may be named `save` and a declared
/// op's type may be any contextual keyword.
fn ddd_service_op<'src>() -> impl Parser<'src, Tokens<'src>, DddServiceOp, MoxExtra<'src>> + Clone {
    let declared = ddd_signature()
        .then(ddd_capabilities().or_not())
        .then_ignore(kw(Token::Other(';')))
        .map_with(
            |((type_ref, multiplicity, name, params), capabilities), e| DddServiceOp {
                name,
                return_type: Some(type_ref),
                multiplicity,
                params,
                delegation: None,
                capabilities: capabilities.unwrap_or_default(),
                span: e.span(),
            },
        );
    let delegated = name()
        .then_ignore(kw(Token::FatArrow))
        .then(ddd_delegation())
        .then(ddd_capabilities().or_not())
        .then_ignore(kw(Token::Other(';')))
        .map_with(|((name, delegation), capabilities), e| DddServiceOp {
            name,
            return_type: None,
            multiplicity: None,
            params: Vec::new(),
            delegation: Some(delegation),
            capabilities: capabilities.unwrap_or_default(),
            span: e.span(),
        });
    delegated.or(declared)
}

/// An `inject <name>;` line of a `service` body.
fn ddd_inject_decl<'src>() -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::INJECT)
        .ignore_then(name())
        .then_ignore(kw(Token::Other(';')))
}

/// One member of a `service` body.
#[derive(Clone)]
enum DddServiceMember {
    Operation(Box<DddServiceOp>),
    Inject(Name),
}

/// Consumes a run of tokens that cannot start or continue a service member,
/// stopping before the body's closing `}` and before identifiers (which
/// could begin the next operation or `inject` line). Always consumes at
/// least one token.
fn junk_ddd_service_member<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(t, Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)) => ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected a service member"));
        })
}

/// One member of a `service` body with junk recovery.
fn ddd_service_member<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<DddServiceMember>, MoxExtra<'src>> + Clone {
    choice((
        ddd_service_op().map(|operation| DddServiceMember::Operation(Box::new(operation))),
        ddd_inject_decl().map(DddServiceMember::Inject),
    ))
    .map(Some)
    .or(junk_ddd_service_member().to(None))
}

/// A `service <name> { ... }` declaration of a `module`. Doc comments (`///`
/// runs) directly above it become the service description after parsing.
fn ddd_service_decl<'src>() -> impl Parser<'src, Tokens<'src>, DddService, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::SERVICE)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(ddd_service_member().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, members), e| {
            let mut operations = Vec::new();
            let mut dependencies = Vec::new();
            for member in members {
                match member {
                    Some(DddServiceMember::Operation(operation)) => {
                        operations.push(*operation);
                    }
                    Some(DddServiceMember::Inject(name)) => dependencies.push(name),
                    None => {}
                }
            }
            DddService {
                name,
                doc: None,
                operations,
                dependencies,
                span: e.span(),
            }
        })
}

/// The `("abstract")? stereotype` head of a design declaration.
fn ddd_design_head<'src>(
) -> impl Parser<'src, Tokens<'src>, (Option<Span>, DddStereotype), MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::ABSTRACT).or_not().then(ddd_stereotype())
}

/// The stereotype keyword of a design head: the `choice` alternatives come
/// from [`ddd::STEREOTYPES`] in the table's order, so the parse order and
/// the table order (the formatter's dispatch set) agree by construction.
fn ddd_stereotype<'src>() -> impl Parser<'src, Tokens<'src>, DddStereotype, MoxExtra<'src>> + Clone
{
    choice(ddd::STEREOTYPES.map(|(keyword, stereotype)| ddd_keyword(keyword).to(stereotype)))
}

/// The design flags of a design declaration: any subset of the
/// [`ddd::DESIGN_FLAGS`] keywords in any order (the canonical order is the
/// table's, a formatter concern). Repeats are idempotent.
fn ddd_flags<'src>() -> impl Parser<'src, Tokens<'src>, DddFlags, MoxExtra<'src>> + Clone {
    select! {
        // chumsky's `select!` takes a boolean guard expression, so the
        // lookup runs twice; the `unwrap` is the guard's `Some`.
        Token::Ident(text) = e if ddd::flag_kind(text).is_some() => {
            (ddd::flag_kind(text).unwrap(), e.span())
        }
    }
    .repeated()
    .collect::<Vec<_>>()
    .map(|pairs| {
        let mut flags = DddFlags::default();
        for (kind, span) in pairs {
            flags.push(kind, span);
        }
        flags
    })
}

/// A design declaration: `("abstract")? stereotype name flag*
/// ("repository" ...)?`.
fn ddd_design_decl<'src>() -> impl Parser<'src, Tokens<'src>, DddDesign, MoxExtra<'src>> + Clone {
    ddd_design_head()
        .then(name())
        .then(ddd_flags())
        .then(ddd_repository_decl().or_not())
        .map_with(
            |((((abstract_span, stereotype), class), flags), repository), e| DddDesign {
                is_abstract: abstract_span.is_some(),
                stereotype,
                class,
                flags,
                repository,
                span: e.span(),
            },
        )
}

/// One operation of a `repository` body: a built-in (`findById`, `findAll`,
/// `save`, `delete` — recognized by their contextual keywords, no signature
/// of their own) or a declared operation with an explicit signature. Both
/// forms end in `;`. A declared repository operation takes no `capability`
/// clause: the wire artifact's repository operations carry no capabilities,
/// so accepting one would silently drop data.
fn ddd_repository_op<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<DddRepositoryOp>, MoxExtra<'src>> + Clone {
    let builtin = select! {
        // chumsky's `select!` takes a boolean guard expression, so the
        // lookup runs twice; the `unwrap` is the guard's `Some`.
        Token::Ident(text) = e if ddd::builtin_from_keyword(text).is_some() => {
            (e.span(), ddd::builtin_from_keyword(text).unwrap())
        }
    }
    .then_ignore(kw(Token::Other(';')))
    .map_with(|(keyword_span, builtin), e| DddRepositoryOp {
        name: Name {
            text: builtin.keyword().to_string(),
            span: keyword_span,
            escaped: false,
        },
        builtin: Some(builtin),
        return_type: None,
        multiplicity: None,
        params: Vec::new(),
        span: e.span(),
    });
    let declared = ddd_signature().then_ignore(kw(Token::Other(';'))).map_with(
        |(type_ref, multiplicity, name, params), e| DddRepositoryOp {
            name,
            builtin: None,
            return_type: Some(type_ref),
            multiplicity,
            params,
            span: e.span(),
        },
    );
    builtin
        .map(Some)
        .or(declared.map(Some))
        .or(junk_ddd_repository_op().to(None))
}

/// Consumes a run of tokens that cannot start or continue a repository
/// operation, stopping before the body's closing `}` and before identifiers.
fn junk_ddd_repository_op<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(t, Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)) => ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected a repository operation"));
        })
}

/// A `repository <name> { ... }` block of a design declaration.
fn ddd_repository_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddRepository, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::REPOSITORY)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(ddd_repository_op().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, operations), e| DddRepository {
            name,
            operations: operations.into_iter().flatten().collect(),
            span: e.span(),
        })
}

/// Scans a raw expression: tokens up to the terminating `;`, tracking the
/// nesting depth of `()`, `[]` and `{}` so an `if ... { ... } else { ... }`
/// expression's braces stay inside the capture while the terminator must sit
/// at depth 0. String literals are single tokens, so a `;` inside one never
/// terminates. The returned span slices the expression source strictly
/// between the `=` and the `;` (the `;` excluded), the `.mox` op-body
/// convention; at least one token is required, so a bare `name = ;` fails.
fn ddd_raw_expr<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    let balanced = recursive(|body| {
        let atom = select! {
            t if !matches!(
                t,
                Token::LParen
                    | Token::RParen
                    | Token::LBracket
                    | Token::RBracket
                    | Token::LBrace
                    | Token::RBrace
                    | Token::Other(';')
            ) =>
                ()
        };
        let parens = kw(Token::LParen)
            .ignore_then(body.clone())
            .then_ignore(kw(Token::RParen));
        let brackets = kw(Token::LBracket)
            .ignore_then(body.clone())
            .then_ignore(kw(Token::RBracket));
        let braces = kw(Token::LBrace)
            .ignore_then(body)
            .then_ignore(kw(Token::RBrace));
        atom.or(parens)
            .or(brackets)
            .or(braces)
            .repeated()
            .at_least(1)
    });
    balanced
        .map_with(|(), e| e.span())
        .then_ignore(kw(Token::Other(';')))
}

/// One `name = <expr>;` entry of a `document { ... }` clause.
fn ddd_document_entry<'src>(
) -> impl Parser<'src, Tokens<'src>, DddDocumentEntry, MoxExtra<'src>> + Clone {
    name()
        .then_ignore(kw(Token::Eq))
        .then(ddd_raw_expr())
        .map(|(name, expr)| DddDocumentEntry { name, expr })
}

/// One `text { ... }` field: `<property> (boost <int>)? (analyzer "<...>")?`.
fn ddd_search_field<'src>(
) -> impl Parser<'src, Tokens<'src>, DddSearchField, MoxExtra<'src>> + Clone {
    qname()
        .then(ddd_keyword(ddd::BOOST).ignore_then(int_lit()).or_not())
        .then(
            ddd_keyword(ddd::ANALYZER)
                .ignore_then(string_lit())
                .or_not(),
        )
        .map_with(|((property, boost), analyzer), e| DddSearchField {
            property,
            boost,
            analyzer,
            span: e.span(),
        })
}

/// The `ranking (bm25 | tfIdf | exact | custom "<...>")` line.
fn ddd_ranking<'src>() -> impl Parser<'src, Tokens<'src>, DddRanking, MoxExtra<'src>> + Clone {
    ddd_keyword("ranking").ignore_then(choice((
        ddd_keyword("bm25").to(DddRanking::Bm25),
        ddd_keyword("tfIdf").to(DddRanking::TfIdf),
        ddd_keyword("exact").to(DddRanking::Exact),
        ddd_keyword("custom")
            .ignore_then(string_lit())
            .map(DddRanking::Custom),
    )))
}

/// The `pagination { (limit <int>)? (max <int>)? (cursor)? }` block. An
/// empty block is legal at the syntax layer (the driver judges it).
fn ddd_pagination<'src>() -> impl Parser<'src, Tokens<'src>, DddPagination, MoxExtra<'src>> + Clone
{
    let member = choice((
        ddd_keyword(ddd::LIMIT)
            .ignore_then(int_lit())
            .map(|value| (Some(value), None, false)),
        ddd_keyword(ddd::MAX)
            .ignore_then(int_lit())
            .map(|value| (None, Some(value), false)),
        ddd_keyword(ddd::CURSOR).to((None, None, true)),
    ));
    ddd_keyword("pagination")
        .ignore_then(kw(Token::LBrace))
        .ignore_then(member.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|members, e| DddPagination {
            limit: members.iter().find_map(|(limit, _, _)| *limit),
            max: members.iter().find_map(|(_, max, _)| *max),
            cursor: members.iter().any(|(_, _, cursor)| *cursor),
            span: e.span(),
        })
}

/// One member of a `search` body.
#[derive(Clone)]
enum DddSearchMember {
    Entity(QualifiedName),
    Text(DddSearchField),
    Filter(QualifiedName),
    Sort(QualifiedName),
    Document(DddDocumentEntry),
    Ranking(DddRanking),
    Analyzer(String),
    Pagination(DddPagination),
    Capability(Name),
}

/// Consumes a run of tokens that cannot start or continue a search member,
/// stopping before the body's closing `}` and before identifiers (which
/// could begin the next member). Always consumes at least one token.
fn junk_ddd_search_member<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(t, Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)) => ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected a search member"));
        })
}

/// The leading keyword of a [`ddd::SEARCH_MEMBERS`] row. Only the
/// `capability` clause — led by the real `capability` token, not an
/// identifier — has no keyword, and its arm never calls this.
fn member_keyword(keyword: Option<&'static str>) -> &'static str {
    keyword.expect("only the `capability` search member lacks a keyword")
}

/// One member of a `search` body with junk recovery. The alternatives come
/// from [`ddd::SEARCH_MEMBERS`] in the table's order — the same order the
/// formatter canonicalizes a search body to — so the parse order and the
/// canonical order cannot drift apart. Repeated `text`, `filters`, `sort`,
/// and `document` clauses merge into their lists; `entity`, `ranking`,
/// `analyzer`, and `pagination` are single — a repeat is idempotent, the
/// first declaration wins (the design-flags precedent).
fn ddd_search_member<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<DddSearchMember>, MoxExtra<'src>> + Clone {
    let alternatives: Vec<Boxed<'src, 'src, Tokens<'src>, Vec<DddSearchMember>, MoxExtra<'src>>> =
        ddd::SEARCH_MEMBERS
            .iter()
            .map(|(kind, keyword)| match kind {
                ddd::SearchMemberKind::Entity => ddd_keyword(member_keyword(*keyword))
                    .ignore_then(qname())
                    .map(|qname| vec![DddSearchMember::Entity(qname)])
                    .boxed(),
                ddd::SearchMemberKind::Text => ddd_keyword(member_keyword(*keyword))
                    .ignore_then(kw(Token::LBrace))
                    .ignore_then(
                        ddd_search_field()
                            .map(Some)
                            .or(junk_ddd_search_member().to(None))
                            .repeated()
                            .collect::<Vec<_>>(),
                    )
                    .then_ignore(kw(Token::RBrace))
                    .map(|fields| {
                        fields
                            .into_iter()
                            .flatten()
                            .map(DddSearchMember::Text)
                            .collect()
                    })
                    .boxed(),
                ddd::SearchMemberKind::Filters => {
                    ddd_search_names(member_keyword(*keyword), DddSearchMember::Filter).boxed()
                }
                ddd::SearchMemberKind::Sort => {
                    ddd_search_names(member_keyword(*keyword), DddSearchMember::Sort).boxed()
                }
                ddd::SearchMemberKind::Document => ddd_keyword(member_keyword(*keyword))
                    .ignore_then(kw(Token::LBrace))
                    .ignore_then(
                        ddd_document_entry()
                            .map(Some)
                            .or(junk_ddd_search_member().to(None))
                            .repeated()
                            .collect::<Vec<_>>(),
                    )
                    .then_ignore(kw(Token::RBrace))
                    .map(|entries| {
                        entries
                            .into_iter()
                            .flatten()
                            .map(DddSearchMember::Document)
                            .collect::<Vec<_>>()
                    })
                    .boxed(),
                ddd::SearchMemberKind::Ranking => ddd_ranking()
                    .map(|ranking| vec![DddSearchMember::Ranking(ranking)])
                    .boxed(),
                ddd::SearchMemberKind::Analyzer => ddd_keyword(member_keyword(*keyword))
                    .ignore_then(string_lit())
                    .map(|analyzer| vec![DddSearchMember::Analyzer(analyzer)])
                    .boxed(),
                ddd::SearchMemberKind::Pagination => ddd_pagination()
                    .map(|pagination| vec![DddSearchMember::Pagination(pagination)])
                    .boxed(),
                ddd::SearchMemberKind::Capability => kw(Token::Capability)
                    .ignore_then(name())
                    .map(|capability| vec![DddSearchMember::Capability(capability)])
                    .boxed(),
            })
            .collect();
    choice(alternatives).or(junk_ddd_search_member().to(Vec::new()))
}

/// The `filters { ... }` / `sort { ... }` bodies: newline-separated
/// qualified names, one per line, with junk recovery between them.
fn ddd_search_names<'src>(
    keyword: &'static str,
    wrap: fn(QualifiedName) -> DddSearchMember,
) -> impl Parser<'src, Tokens<'src>, Vec<DddSearchMember>, MoxExtra<'src>> + Clone {
    ddd_keyword(keyword)
        .ignore_then(kw(Token::LBrace))
        .ignore_then(
            qname()
                .map(Some)
                .or(junk_ddd_search_member().to(None))
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then_ignore(kw(Token::RBrace))
        .map(move |names| names.into_iter().flatten().map(wrap).collect())
}

/// A `search <name> { ... }` projection of a `module`. Doc comments (`///`
/// runs) directly above it become the search description after parsing.
fn ddd_search_decl<'src>() -> impl Parser<'src, Tokens<'src>, DddSearch, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::SEARCH)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(ddd_search_member().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, members), e| {
            let mut search = DddSearch {
                name,
                doc: None,
                entity: None,
                text: Vec::new(),
                filters: Vec::new(),
                sort: Vec::new(),
                document: Vec::new(),
                ranking: None,
                analyzer: None,
                pagination: None,
                capabilities: Vec::new(),
                span: e.span(),
            };
            for member in members.into_iter().flatten() {
                match member {
                    DddSearchMember::Entity(entity) => {
                        if search.entity.is_none() {
                            search.entity = Some(entity);
                        }
                    }
                    DddSearchMember::Text(field) => search.text.push(field),
                    DddSearchMember::Filter(name) => search.filters.push(name),
                    DddSearchMember::Sort(name) => search.sort.push(name),
                    DddSearchMember::Document(entry) => search.document.push(entry),
                    DddSearchMember::Ranking(ranking) => {
                        if search.ranking.is_none() {
                            search.ranking = Some(ranking);
                        }
                    }
                    DddSearchMember::Analyzer(analyzer) => {
                        if search.analyzer.is_none() {
                            search.analyzer = Some(analyzer);
                        }
                    }
                    DddSearchMember::Pagination(pagination) => {
                        if search.pagination.is_none() {
                            search.pagination = Some(pagination);
                        }
                    }
                    DddSearchMember::Capability(name) => search.capabilities.push(name),
                }
            }
            search
        })
}

/// One member of a `module` body.
#[derive(Clone)]
enum DddModuleMember {
    Service(DddService),
    Design(DddDesign),
    Search(DddSearch),
}

/// Consumes a run of tokens that cannot start or continue a module member,
/// stopping before the body's closing `}` and before identifiers (which
/// could begin the next `service` or design).
fn junk_ddd_module_member<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(t, Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)) => ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected a module member"));
        })
}

/// One member of a `module` body with junk recovery.
fn ddd_module_member<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<DddModuleMember>, MoxExtra<'src>> + Clone {
    choice((
        ddd_service_decl().map(DddModuleMember::Service),
        ddd_design_decl().map(DddModuleMember::Design),
        ddd_search_decl().map(DddModuleMember::Search),
    ))
    .map(Some)
    .or(junk_ddd_module_member().to(None))
}

/// A `module <name> { ... }` declaration of the application.
fn ddd_module_decl<'src>() -> impl Parser<'src, Tokens<'src>, DddModule, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::MODULE)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(ddd_module_member().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, members), e| {
            let mut services = Vec::new();
            let mut designs = Vec::new();
            let mut searches = Vec::new();
            for member in members {
                match member {
                    Some(DddModuleMember::Service(service)) => services.push(service),
                    Some(DddModuleMember::Design(design)) => designs.push(design),
                    Some(DddModuleMember::Search(search)) => searches.push(search),
                    None => {}
                }
            }
            DddModule {
                name,
                services,
                designs,
                searches,
                span: e.span(),
            }
        })
}

/// The `base <qualified-name>` member of the application body.
fn ddd_base_decl<'src>() -> impl Parser<'src, Tokens<'src>, DddBase, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::BASE)
        .ignore_then(qname())
        .map_with(|package, e| DddBase {
            package,
            span: e.span(),
        })
}

/// One member of an `application` body.
#[derive(Clone)]
enum DddApplicationItem {
    Base(DddBase),
    Module(DddModule),
}

/// Consumes a run of tokens that cannot start or continue an application
/// member, stopping before the body's closing `}` and before identifiers.
fn junk_ddd_application_item<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone
{
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(t, Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)) => ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected `base` or `module`"));
        })
}

/// One member of an `application` body with junk recovery.
fn ddd_application_item<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<DddApplicationItem>, MoxExtra<'src>> + Clone {
    choice((
        ddd_base_decl().map(DddApplicationItem::Base),
        ddd_module_decl().map(DddApplicationItem::Module),
    ))
    .map(Some)
    .or(junk_ddd_application_item().to(None))
}

/// The `application <name> { ... }` declaration: at most one `base` (which
/// the grammar pins to the first member position) followed by modules.
fn ddd_application_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddApplication, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::APPLICATION)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(ddd_application_item().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, items), e| {
            let mut base = None;
            let mut modules = Vec::new();
            let mut problems = Vec::new();
            for item in items {
                match item {
                    Some(DddApplicationItem::Base(decl))
                        if base.is_none() && modules.is_empty() =>
                    {
                        base = Some(decl);
                    }
                    Some(DddApplicationItem::Base(decl)) if base.is_some() => {
                        problems.push((decl.span, "duplicate `base` declaration".to_string()))
                    }
                    Some(DddApplicationItem::Base(decl)) => problems.push((
                        decl.span,
                        "`base` must be the first member of the application".to_string(),
                    )),
                    Some(DddApplicationItem::Module(module)) => modules.push(module),
                    None => {}
                }
            }
            (
                DddApplication {
                    name,
                    base,
                    modules,
                    span: e.span(),
                },
                problems,
            )
        })
        .validate(|(decl, problems), _, emitter| {
            for (span, message) in problems {
                emitter.emit(Rich::custom(span, message));
            }
            decl
        })
}

/// One top-level item of a `.ddd` file.
#[derive(Clone)]
enum DddFileItem {
    Import(ImportDecl),
    Application(DddApplication),
    Junk,
}

/// Consumes a run of tokens up to the next `import` keyword or the
/// contextual `application` keyword (or end of input). File-level recovery:
/// always consumes at least one token (guaranteeing progress) and emits an
/// error for the skipped region.
fn junk_ddd_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let rest = select! {
        t if !matches!(t, Token::Import | Token::Ident(ddd::APPLICATION)) => ()
    };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(
                e.span(),
                "expected an import or `application` declaration",
            ));
        })
}

/// Folds the top-level items of a `.ddd` file: imports must precede the
/// application and exactly one application is allowed. Violations are
/// reported as (span, message) problems; the AST recovers the first
/// application and drops the offending items regardless.
fn fold_ddd_file(items: Vec<DddFileItem>) -> (DddFile, Vec<(Span, String)>) {
    let mut imports = Vec::new();
    let mut application = None;
    let mut problems = Vec::new();
    for item in items {
        match item {
            DddFileItem::Import(decl) => {
                if application.is_none() {
                    imports.push(decl);
                } else {
                    problems.push((
                        decl.span,
                        "`import` after the `application` declaration".to_string(),
                    ));
                }
            }
            DddFileItem::Application(decl) => {
                if application.is_none() {
                    application = Some(decl);
                } else {
                    problems.push((decl.span, "duplicate `application` declaration".to_string()));
                }
            }
            DddFileItem::Junk => {}
        }
    }
    (
        DddFile {
            imports,
            application,
        },
        problems,
    )
}

fn ddd_file<'src>() -> impl Parser<'src, Tokens<'src>, DddFile, MoxExtra<'src>> + Clone {
    let item = choice((
        ddd_import_decl().map(DddFileItem::Import),
        ddd_application_decl().map(DddFileItem::Application),
    ))
    .or(junk_ddd_file().to(DddFileItem::Junk));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map_with(|items, e| (fold_ddd_file(items), e.span()))
        .validate(|((file, problems), span), _, emitter| {
            for (span, message) in problems {
                emitter.emit(Rich::custom(span, message));
            }
            if file.application.is_none() {
                emitter.emit(Rich::custom(
                    Span::from(span.end..span.end),
                    "expected an `application` declaration",
                ));
            }
            file
        })
}

/// The outcome of parsing a `.ddd` source: as much of the file as could be
/// recovered, plus all encountered errors. Parsing never panics and always
/// produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct DddParseResult {
    /// The recovered design file, or `None` if no output could be produced
    /// at all.
    pub ast: Option<DddFile>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

/// Lex and parse a `.ddd` source text: `import` declarations followed by
/// exactly one `application` declaration of modules, application services,
/// and class designs.
///
/// This function never panics and always recovers as much of the AST as
/// possible; check [`DddParseResult::errors`] for syntax problems. Doc
/// comment runs (`///`) directly above a `service` declaration become its
/// description. A file without an `application` declaration is a syntax
/// error (recovered as `application: None`).
pub fn parse_ddd(source: &str) -> DddParseResult {
    let (tokens, comments) = match lex_with_comments(source) {
        Ok(result) => result,
        Err(error) => {
            return DddParseResult {
                ast: None,
                errors: vec![ParseError {
                    message: error.to_string(),
                    span: error.span,
                }],
            }
        }
    };
    let (mut ast, errors) = ddd_file().parse(Tokens::new(&tokens)).into_output_errors();
    if let Some(file) = ast.as_mut() {
        attach_ddd_docs(file, &comments, source);
    }
    DddParseResult {
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

/// Keeps the normative grammar in this module's docs in sync with the code.
///
/// The grammar authority lives as EBNF blocks in the `//!` docs above (see
/// "The normative grammar"). Full grammar-from-docs validation is out of
/// scope; these tests are the cheap, honest tripwire: every *keyword
/// terminal* of the EBNF must be a word the lexer/parser actually knows, and
/// every keyword the lexer knows must appear in the EBNF — so adding,
/// renaming, or removing a keyword without updating the grammar fails here.
/// Structural drift (repetition/optionality changes) is not caught
/// mechanically.
#[cfg(test)]
mod grammar_doc_tests {
    use crate::ddd;
    use crate::lexer::lex;

    /// This file's source; the leading `//!` lines are the module docs that
    /// carry the grammar blocks.
    const PARSER_SOURCE: &str = include_str!("parser.rs");

    /// The module docs (the leading `//!` line run), comment markers stripped.
    fn module_docs() -> String {
        let mut docs = String::new();
        for line in PARSER_SOURCE.lines() {
            let Some(rest) = line.strip_prefix("//!") else {
                break;
            };
            docs.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            docs.push('\n');
        }
        docs
    }

    /// The fenced ```text blocks of the docs: the EBNF grammar blocks.
    fn grammar_blocks() -> Vec<String> {
        let mut blocks = Vec::new();
        let mut current: Option<Vec<String>> = None;
        for line in module_docs().lines() {
            match (current.as_mut(), line.trim()) {
                (None, "```text") => current = Some(Vec::new()),
                (Some(block), "```") => {
                    blocks.push(block.join("\n"));
                    current = None;
                }
                (Some(block), line) => block.push(line.to_string()),
                (None, _) => {}
            }
        }
        assert!(current.is_none(), "unterminated grammar block");
        blocks
    }

    /// The quoted EBNF terminals of all grammar blocks that are shaped like
    /// words (punctuation terminals such as `"{"` are skipped).
    fn grammar_words() -> Vec<String> {
        let mut words = Vec::new();
        for block in grammar_blocks() {
            for terminal in block.split('"').skip(1).step_by(2) {
                if !terminal.is_empty()
                    && terminal
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    words.push(terminal.to_string());
                }
            }
        }
        words
    }

    /// Contextual words the grammar quotes that are *not* lexer keywords and
    /// not covered by a table below: feature modifiers, the import sigils,
    /// the delegation `to`, the datatype block words, `unique`, and the
    /// parser-only `ranking` values (`crate::ddd` documents them as
    /// parser-only).
    const CONTEXTUAL_WORDS: [&str; 15] = [
        "id", "readonly", "schema", "sigil", "to", "create", "convert", "format", "unique", "bm25",
        "tfIdf", "exact", "custom", "entity", "value",
    ];

    /// Every keyword of the lexer. Each entry is checked against the lexer
    /// itself, so this list and `Token` cannot drift apart silently.
    const KEYWORDS: [&str; 38] = [
        "package",
        "annotation",
        "as",
        "class",
        "extends",
        "interface",
        "enum",
        "type",
        "wraps",
        "opaque",
        "contains",
        "refers",
        "container",
        "opposite",
        "op",
        "derived",
        "true",
        "false",
        "vocabulary",
        "from",
        "version",
        "key",
        "facet",
        "actors",
        "actor",
        "agent",
        "capability",
        "grant",
        "permit",
        "forbid",
        "when",
        "obligation",
        "on",
        "never_both",
        "delegation",
        "purpose",
        "cedar",
        "import",
    ];

    /// Every word the grammar is allowed to quote as a terminal.
    fn known_words() -> Vec<&'static str> {
        let mut words: Vec<&'static str> = KEYWORDS.to_vec();
        words.extend(CONTEXTUAL_WORDS);
        words.extend(super::VALUED_CONSTRAINT_KEYWORDS);
        words.extend([
            ddd::APPLICATION,
            ddd::BASE,
            ddd::MODULE,
            ddd::SERVICE,
            ddd::SEARCH,
            ddd::ABSTRACT,
            ddd::REPOSITORY,
            ddd::INJECT,
            ddd::BOOST,
            ddd::ANALYZER,
            ddd::LIMIT,
            ddd::MAX,
            ddd::CURSOR,
        ]);
        for (keyword, _) in ddd::STEREOTYPES {
            words.push(keyword);
        }
        for (keyword, _) in ddd::DESIGN_FLAGS {
            words.push(keyword);
        }
        for (keyword, _) in ddd::REPOSITORY_BUILTINS {
            words.push(keyword);
        }
        for (kind, keyword) in ddd::SEARCH_MEMBERS {
            let _ = kind;
            if let Some(keyword) = keyword {
                words.push(keyword);
            }
        }
        words
    }

    /// The grammar blocks exist and every word terminal is a known keyword or
    /// contextual word — quoting a word the parser would not recognize fails.
    #[test]
    fn grammar_terminals_are_known_words() {
        let blocks = grammar_blocks();
        assert!(
            blocks.len() >= 4,
            "expected the .mox, actors, .actor, and .ddd grammar blocks, found {}",
            blocks.len()
        );
        let known = known_words();
        for word in grammar_words() {
            let word = word.as_str();
            assert!(
                known.contains(&word),
                "the grammar quotes `{word}`, which is neither a lexer keyword \
                 nor a known contextual word — update the grammar or the word \
                 lists in this test"
            );
        }
    }

    /// Every lexer keyword appears in the grammar — adding or renaming a
    /// keyword in [`Token`] without updating the grammar fails.
    #[test]
    fn every_lexed_keyword_appears_in_the_grammar() {
        let words = grammar_words();
        for keyword in KEYWORDS {
            // The list itself must stay honest: each entry really is a
            // keyword token of the lexer.
            let tokens = lex(keyword).unwrap();
            assert_eq!(
                tokens.len(),
                1,
                "`{keyword}` must be exactly one token to be a keyword"
            );
            assert_eq!(
                tokens[0].0.keyword(),
                Some(keyword),
                "`{keyword}` is listed as a keyword but lexes otherwise — \
                 update KEYWORDS in this test"
            );
            assert!(
                words.iter().any(|word| word == keyword),
                "the lexer keyword `{keyword}` is missing from the normative \
                 grammar blocks in this module's docs"
            );
        }
    }

    /// Every `.ddd` table keyword appears in the grammar — extending a
    /// `crate::ddd` table (the keyword authority #36 single-sourced) without
    /// updating the grammar fails.
    #[test]
    fn every_ddd_table_keyword_appears_in_the_grammar() {
        let words = grammar_words();
        let check = |keyword: &'static str| {
            assert!(
                words.iter().any(|word| word == keyword),
                "the .ddd keyword `{keyword}` is missing from the normative \
                 grammar blocks in this module's docs"
            );
        };
        for (keyword, _) in ddd::STEREOTYPES {
            check(keyword);
        }
        for (keyword, _) in ddd::DESIGN_FLAGS {
            check(keyword);
        }
        for (keyword, _) in ddd::REPOSITORY_BUILTINS {
            check(keyword);
        }
        for (_, keyword) in ddd::SEARCH_MEMBERS {
            if let Some(keyword) = keyword {
                check(keyword);
            }
        }
    }
}

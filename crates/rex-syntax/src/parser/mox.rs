//! The `.mox` grammar: features, declarations, datatypes, and
//! vocabularies, plus the fault-tolerant `model` parser and the
//! plain `parse` entry point.

use std::iter::once;

use crate::lexer::{lex_with_comments, Token};

use super::actor::actors_decl;
use super::docs::attach_docs;
use super::imports::{import_schema_decl, import_sigil_decl};
use super::*;
pub(crate) fn kw<'src>(
    token: Token<'src>,
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { t = e if t == token => e.span() }
}

pub(crate) fn attribute<'src>(
) -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
    attribute_with(tref())
}

/// The `attribute` production over a caller-chosen type parser. The class
/// body uses the shared [`tref`]; the register body narrows it (see
/// [`register_type_ref`]) — the shape is otherwise identical.
pub(crate) fn attribute_with<'src, P>(
    type_parser: P,
) -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone
where
    P: Parser<'src, Tokens<'src>, TypeRef, MoxExtra<'src>> + Clone,
{
    type_parser
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

/// A register body attribute's type: the shared `type_ref`, except that a
/// *single-segment keyword* reference is not accepted. A bare keyword can
/// never name a declared type — every `.mox` declaration names itself with
/// an identifier — so accepting one would only ever mis-parse class-only
/// grammar (`extends Base` silently reading as an attribute typed
/// `extends`, issue rexlang#181). Keyword-led *qualified* references keep
/// parsing: a package's last segment may be a keyword
/// (`package rex.conformance.actors`), so `actors.Thing` stays a valid
/// attribute type inside a register body.
pub(crate) fn register_type_ref<'src>(
) -> impl Parser<'src, Tokens<'src>, TypeRef, MoxExtra<'src>> + Clone {
    let ident_led = name().then(
        kw(Token::Dot)
            .ignore_then(qname_segment())
            .repeated()
            .collect::<Vec<_>>(),
    );
    let keyword_led = qname_segment().then(
        kw(Token::Dot)
            .ignore_then(qname_segment())
            .repeated()
            .at_least(1)
            .collect::<Vec<_>>(),
    );
    ident_led
        .or(keyword_led)
        .map_with(|(first, rest), e| TypeRef {
            name: QualifiedName {
                segments: once(first).chain(rest).collect(),
                span: e.span(),
            },
            span: e.span(),
        })
}

pub(crate) fn containment<'src>(
) -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
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

pub(crate) fn reference<'src>(
) -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
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

pub(crate) fn container<'src>(
) -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
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

pub(crate) fn op_decl<'src>() -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone
{
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

pub(crate) fn derived_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, FeatureDecl, MoxExtra<'src>> + Clone {
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

pub(crate) fn feature<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<FeatureDecl>, MoxExtra<'src>> + Clone {
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

pub(crate) fn package_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, PackageDecl, MoxExtra<'src>> + Clone {
    kw(Token::Package)
        .ignore_then(qname())
        .map_with(|name, e| PackageDecl {
            name,
            doc: None,
            span: e.span(),
        })
}

pub(crate) fn annotation_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
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

/// The contextual `table` annotation of a class declaration: the ordinary
/// identifier `table` directly followed by a string literal, in the one
/// grammar position after the class name (`table` stays usable as a class,
/// feature, or enum name everywhere else — the `id`/`readonly` contextual
/// identifier precedent).
pub(crate) fn table_annotation<'src>(
) -> impl Parser<'src, Tokens<'src>, String, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) if text == "table" => () }.ignore_then(string_lit())
}

pub(crate) fn class_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
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
        .then(table_annotation().or_not())
        .then(extends.or_not())
        .then_ignore(kw(Token::LBrace))
        .then(feature().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(((name, table), extends), features), e| {
            Decl::Class(ClassDecl {
                name,
                doc: None,
                table,
                extends: extends.unwrap_or_default(),
                features: features.into_iter().flatten().collect(),
                span: e.span(),
            })
        })
}

/// The contextual `register` declaration (issue rexlang#181): the ordinary
/// identifier `register` at the top-level item position, followed by a name
/// and a body of the existing `attribute` production. Everywhere else —
/// class bodies, feature, enum, and class names — `register` stays an
/// ordinary identifier (the `table`-annotation contextual precedent). The
/// grammar admits no `extends`, no `table` annotation, and no relationship
/// features in the body: they simply do not parse here.
pub(crate) fn register_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone
{
    select! { Token::Ident(text) if text == "register" => () }
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(
            attribute_with(register_type_ref())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, attributes), e| {
            Decl::Register(RegisterDecl {
                name,
                doc: None,
                attributes,
                span: e.span(),
            })
        })
}

pub(crate) fn binding_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<BindingEntry>, MoxExtra<'src>> + Clone {
    kw(Token::LBrace)
        .ignore_then(binding_entry().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
}

pub(crate) fn binding_entry<'src>(
) -> impl Parser<'src, Tokens<'src>, BindingEntry, MoxExtra<'src>> + Clone {
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
pub(crate) fn named_target_block<'src>(
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
pub(crate) fn format_entry<'src>() -> impl Parser<'src, Tokens<'src>, String, MoxExtra<'src>> + Clone
{
    select! { Token::Ident(text) if text == "format" => () }.ignore_then(string_lit())
}

/// One entry of a datatype's `{ ... }` block.
pub(crate) enum DatatypeEntry {
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
pub(crate) fn datatype_block<'src>() -> impl Parser<
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

pub(crate) fn interface_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone
{
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

pub(crate) fn enum_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
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

pub(crate) fn datatype_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone
{
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
pub(crate) enum VocabItem {
    Version(String),
    Key(Name),
    Facet(VocabularyFacetDecl),
}

pub(crate) fn vocabulary_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
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

#[derive(Clone)]
pub(crate) enum Item {
    Package(PackageDecl),
    Decl(Decl),
    Junk,
}

/// Consumes a run of tokens up to the next top-level declaration keyword
/// (or end of input). Declaration-level recovery: always consumes at least
/// one token (guaranteeing progress) and emits an error for the skipped
/// region.
pub(crate) fn junk_decl<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
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

pub(crate) fn fold_model(items: Vec<Item>) -> Model {
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

pub(crate) fn model<'src>() -> impl Parser<'src, Tokens<'src>, Model, MoxExtra<'src>> + Clone {
    let item = choice((
        package_decl().map(Item::Package),
        annotation_decl().map(Item::Decl),
        class_decl().map(Item::Decl),
        register_decl().map(Item::Decl),
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

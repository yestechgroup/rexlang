//! Import declarations: the plain `import "path"` shape shared by
//! `.actor`/`.evt`/`.deploy`, plus the `.mox` `import schema` and
//! `import sigil` forms.

use crate::lexer::Token;

use super::*;
/// An `import "path"` declaration without an `as` clause (`.actor`,
/// `.evt`, `.deploy`). The path is a string literal with the standard
/// escapes.
pub(crate) fn import_decl<'src>() -> impl Parser<'src, Tokens<'src>, Import, MoxExtra<'src>> + Clone
{
    kw(Token::Import)
        .ignore_then(string_lit())
        .map_with(|path, e| Import {
            path,
            alias: None,
            span: e.span(),
        })
}

/// The contextual `schema` word of an `import schema` declaration: an
/// ordinary identifier that is special only directly after the `import`
/// keyword of a `.mox` file (the `format`-entry precedent). Escaped
/// `^schema` never matches.
pub(crate) fn schema_keyword<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone
{
    select! { Token::Ident(text) = e if text == "schema" => e.span() }
}

/// An `import schema "<path>" (as <name>)?` declaration of a `.mox` source.
/// Once the contextual `schema` word matched, the declaration is committed:
/// a missing path literal or a missing alias name is a syntax error, but the
/// AST still recovers a declaration so the surrounding model parses on.
/// `import` without `schema` is not this declaration (the alternative fails
/// and declaration-level recovery reports the region).
pub(crate) fn import_schema_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
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
pub(crate) fn sigil_keyword<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone
{
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
pub(crate) fn import_sigil_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
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

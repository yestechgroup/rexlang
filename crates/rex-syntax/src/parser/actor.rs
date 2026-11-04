//! The shared `actors_block` grammar (inline `.mox` blocks and
//! `.actor` files alike) and the standalone `.actor` surface.

use std::iter::once;

use crate::lexer::{lex, Token};

use super::imports::import_decl;
use super::*;
/// One body item of an `actors` declaration.
pub(crate) enum ActorsItem {
    Actor(ActorDecl),
    Capability(CapabilityDecl),
    Purpose(PurposeDecl),
    Obligation(ObligationDecl),
    Grant(GrantDecl),
    Delegation(DelegationDecl),
    NeverBoth(NeverBothDecl),
}

/// One body item of a `delegation` declaration.
pub(crate) enum DelegationItem {
    From(Name),
    To(Name),
    Purpose(Name),
    Entry(GrantEntryDecl),
}

/// The `actors <name> { ... }` block grammar, shared verbatim by the inline
/// `.mox` declaration (see [`actors_decl`]) and standalone `.actor` files.
pub(crate) fn actors_block<'src>(
) -> impl Parser<'src, Tokens<'src>, ActorsDecl, MoxExtra<'src>> + Clone {
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
    let obligation_decl = kw(Token::Obligation)
        .ignore_then(name())
        .map_with(|name, e| ObligationDecl {
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
        obligation_decl.map(ActorsItem::Obligation),
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
            let mut obligations = Vec::new();
            let mut grants = Vec::new();
            let mut delegations = Vec::new();
            let mut never_both = Vec::new();
            for item in items {
                match item {
                    ActorsItem::Actor(decl) => actors.push(decl),
                    ActorsItem::Capability(decl) => capabilities.push(decl),
                    ActorsItem::Purpose(decl) => purposes.push(decl),
                    ActorsItem::Obligation(decl) => obligations.push(decl),
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
                obligations,
                grants,
                delegations,
                never_both,
                span: e.span(),
            }
        })
}

/// The inline `actors { ... }` declaration of a `.mox` source: the shared
/// [`actors_block`] grammar wrapped as a top-level declaration.
pub(crate) fn actors_decl<'src>() -> impl Parser<'src, Tokens<'src>, Decl, MoxExtra<'src>> + Clone {
    actors_block().map(Decl::Actors)
}

/// Folds the body items of a delegation into a [`DelegationDecl`], enforcing
/// the required `from` → `to` → `purpose` ordering and rejecting `cedar`
/// entries. The returned problems are (span, message) pairs the caller emits
/// as errors; the AST recovers as much as possible regardless.
pub(crate) fn fold_delegation(
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

#[derive(Clone)]
pub(crate) enum ActorFileItem {
    Import(ImportDecl),
    Block(ActorsDecl),
    Junk,
}

/// Consumes a run of tokens up to the next `import`/`actors` keyword (or end
/// of input). Declaration-level recovery for `.actor` files: always consumes
/// at least one token (guaranteeing progress) and emits an error for the
/// skipped region.
pub(crate) fn junk_actor_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone
{
    let rest = select! { t if !matches!(t, Token::Import | Token::Actors) => () };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected an import or actors block"));
        })
}

pub(crate) fn fold_actor_file(items: Vec<ActorFileItem>) -> (ActorFile, Option<Span>) {
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

pub(crate) fn actor_file<'src>(
) -> impl Parser<'src, Tokens<'src>, ActorFile, MoxExtra<'src>> + Clone {
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

// --- .evt files ---------------------------------------------------------------

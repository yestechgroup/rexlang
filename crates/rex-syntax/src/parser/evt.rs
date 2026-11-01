//! The `.evt` event-contract grammar: events, channels,
//! subscriptions, and the fault-tolerant file parser.

use crate::lexer::{lex, Token};

use super::imports::import_decl;
use super::*;
/// One top-level item of an `.evt` file.
#[derive(Clone)]
pub(crate) enum EvtFileItem {
    Import(ImportDecl),
    Event(EventDecl),
    Channel(ChannelDecl),
    Subscription(SubscriptionDecl),
    Junk,
}

/// One `name: type_ref;` payload field of an [`EventDecl`]. Committed once
/// the field name matched: a missing `:`, type, or `;` is a reported error
/// and the field still recovers (with an empty type when absent), so one
/// malformed field never loses the surrounding event.
pub(crate) fn event_field<'src>(
) -> impl Parser<'src, Tokens<'src>, EventFieldDecl, MoxExtra<'src>> + Clone {
    name()
        .then(kw(Token::Other(':')).or_not())
        .then(tref().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(((name, colon), ty), semi), e| (name, colon, ty, semi, e.span()))
        .validate(|(name, colon, ty, semi, span), _e, emitter| {
            if colon.is_none() {
                emitter.emit(Rich::custom(name.span, "expected `:` after the field name"));
            }
            if ty.is_none() {
                emitter.emit(Rich::custom(name.span, "expected a field type"));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the field"));
            }
            EventFieldDecl {
                name,
                ty: ty.unwrap_or_else(|| TypeRef {
                    name: QualifiedName {
                        segments: Vec::new(),
                        span: (span.end..span.end).into(),
                    },
                    span: (span.end..span.end).into(),
                }),
                span,
            }
        })
}

/// An `event <name> ("version" string)? { field* }` declaration. The
/// `version` clause is committed once the keyword matched: a missing version
/// string is a reported error and the event still recovers.
pub(crate) fn event_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, EventDecl, MoxExtra<'src>> + Clone {
    kw(Token::Event)
        .ignore_then(name())
        .then(kw(Token::Version).or_not().then(string_lit().or_not()))
        .then_ignore(kw(Token::LBrace))
        .then(event_field().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|((name, (version_kw, version)), fields), e| {
            (name, version_kw, version, fields, e.span())
        })
        .validate(|(name, version_kw, version, fields, span), _e, emitter| {
            if version_kw.is_some() && version.is_none() {
                emitter.emit(Rich::custom(
                    name.span,
                    "expected a version string after `version`",
                ));
            }
            EventDecl {
                name,
                version,
                fields,
                span,
            }
        })
}

/// A `publishes <event>;` entry of a channel body. Committed once the
/// `publishes` keyword matched: a missing event name or `;` is a reported
/// error and the entry still recovers.
pub(crate) fn publishes_entry<'src>(
) -> impl Parser<'src, Tokens<'src>, PublishesDecl, MoxExtra<'src>> + Clone {
    kw(Token::Publishes)
        .ignore_then(name().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(event, semi), e| (event, semi, e.span()))
        .validate(|(event, semi, span), _e, emitter| {
            let missing = Name {
                text: String::new(),
                span,
                escaped: false,
            };
            if event.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected an event name after `publishes`",
                ));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `;` after the `publishes` entry",
                ));
            }
            PublishesDecl {
                event: event.unwrap_or(missing),
                span,
            }
        })
}

/// A `channel <name> { publishes_entry* }` declaration.
pub(crate) fn channel_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, ChannelDecl, MoxExtra<'src>> + Clone {
    kw(Token::Channel)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(publishes_entry().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, publishes), e| ChannelDecl {
            name,
            publishes,
            span: e.span(),
        })
}

/// The contextual `events` word of a subscription body: an ordinary
/// identifier that is special only in that grammar position (the
/// delegation-`to` precedent). Escaped `^events` never matches.
pub(crate) fn evt_events_keyword<'src>(
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == "events" => e.span() }
}

/// The contextual `consumer` word of a subscription body (same discipline as
/// [`evt_events_keyword`]).
pub(crate) fn evt_consumer_keyword<'src>(
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == "consumer" => e.span() }
}

/// A `subscription <name> { events [ <event>* ] consumer <name> }`
/// declaration. Once the body's `{` matched the declaration is committed:
/// missing `events`, list brackets, `consumer`, or the consumer name are
/// each reported errors and the subscription still recovers. Inside the
/// `[...]` list the contextual `consumer` word ends the list (the word
/// introducing the next clause), so a missing `]` never swallows the
/// consumer line.
pub(crate) fn subscription_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, SubscriptionDecl, MoxExtra<'src>> + Clone {
    let event_name = select! {
        Token::Ident(text) = e if text != "consumer" => Name {
            text: text.to_string(),
            span: e.span(),
            escaped: false,
        },
        Token::IdentEscaped(text) = e => Name {
            text: text.to_string(),
            span: e.span(),
            escaped: true,
        },
    };
    kw(Token::Subscription)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(evt_events_keyword().or_not())
        .then(
            kw(Token::LBracket)
                .ignore_then(event_name.repeated().collect::<Vec<_>>())
                .then(kw(Token::RBracket).map_with(|_, e| e.span()).or_not())
                .map_with(|(events, close), e| (events, close, e.span()))
                .or_not(),
        )
        .then(evt_consumer_keyword().or_not())
        .then(name().or_not())
        .then_ignore(kw(Token::RBrace).or_not())
        .map_with(|((((name, events_kw), list), consumer_kw), consumer), e| {
            (name, events_kw, list, consumer_kw, consumer, e.span())
        })
        .validate(
            |(name, events_kw, list, consumer_kw, consumer, span), _e, emitter| {
                if events_kw.is_none() {
                    emitter.emit(Rich::custom(span, "expected `events`"));
                }
                let (events, close, list_span) = match list {
                    Some(list) => list,
                    None => {
                        if events_kw.is_some() {
                            emitter.emit(Rich::custom(span, "expected `[...]` after `events`"));
                        }
                        (Vec::new(), None, span)
                    }
                };
                if events_kw.is_some() && close.is_none() {
                    emitter.emit(Rich::custom(list_span, "expected `]` after the event list"));
                }
                if consumer_kw.is_none() {
                    emitter.emit(Rich::custom(span, "expected `consumer`"));
                }
                if consumer_kw.is_some() && consumer.is_none() {
                    emitter.emit(Rich::custom(
                        span,
                        "expected a consumer name after `consumer`",
                    ));
                }
                SubscriptionDecl {
                    name,
                    events,
                    consumer: consumer.unwrap_or_else(|| Name {
                        text: String::new(),
                        span,
                        escaped: false,
                    }),
                    span,
                }
            },
        )
}

/// Consumes a run of tokens up to the next `import`/`event`/`channel`/
/// `subscription` keyword (or end of input). Declaration-level recovery for
/// `.evt` files: always consumes at least one token (guaranteeing progress)
/// and emits an error for the skipped region.
pub(crate) fn junk_evt_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let rest = select! {
        t if !matches!(
            t,
            Token::Import | Token::Event | Token::Channel | Token::Subscription
        ) =>
        ()
    };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(
                e.span(),
                "expected an import, event, channel, or subscription declaration",
            ));
        })
}

pub(crate) fn fold_evt_file(items: Vec<EvtFileItem>) -> (EvtFile, Option<Span>) {
    let mut imports = Vec::new();
    let mut events = Vec::new();
    let mut channels = Vec::new();
    let mut subscriptions = Vec::new();
    // Span of the first `import` that follows a declaration (a syntax error
    // reported by the caller's `validate`).
    let mut late_import = None;
    for item in items {
        match item {
            EvtFileItem::Import(decl) => {
                if events.is_empty() && channels.is_empty() && subscriptions.is_empty() {
                    imports.push(decl);
                } else if late_import.is_none() {
                    late_import = Some(decl.span);
                }
            }
            EvtFileItem::Event(decl) => events.push(decl),
            EvtFileItem::Channel(decl) => channels.push(decl),
            EvtFileItem::Subscription(decl) => subscriptions.push(decl),
            EvtFileItem::Junk => {}
        }
    }
    (
        EvtFile {
            imports,
            events,
            channels,
            subscriptions,
        },
        late_import,
    )
}

pub(crate) fn evt_file<'src>() -> impl Parser<'src, Tokens<'src>, EvtFile, MoxExtra<'src>> + Clone {
    let item = choice((
        import_decl().map(EvtFileItem::Import),
        event_decl().map(EvtFileItem::Event),
        channel_decl().map(EvtFileItem::Channel),
        subscription_decl().map(EvtFileItem::Subscription),
    ))
    .or(junk_evt_file().to(EvtFileItem::Junk));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map(fold_evt_file)
        .validate(|(file, late_import), _, emitter| {
            if let Some(span) = late_import {
                emitter.emit(Rich::custom(
                    span,
                    "`import` after an event, channel, or subscription declaration",
                ));
            }
            file
        })
}

/// The outcome of parsing an `.evt` source: as much of the file as could be
/// recovered, plus all encountered errors. Parsing never panics and always
/// produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct EvtParseResult {
    /// The recovered event-contract file, or `None` if no output could be
    /// produced at all.
    pub ast: Option<EvtFile>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

/// Lex and parse an `.evt` source text: `import` declarations followed by
/// `event`, `channel`, and `subscription` declarations (which may be
/// interleaved; imports must come first).
///
/// This function never panics and always recovers as much of the AST as
/// possible; check [`EvtParseResult::errors`] for syntax problems.
pub fn parse_evt(source: &str) -> EvtParseResult {
    let tokens = match lex(source) {
        Ok(tokens) => tokens,
        Err(error) => {
            return EvtParseResult {
                ast: None,
                errors: vec![ParseError {
                    message: error.to_string(),
                    span: error.span,
                }],
            }
        }
    };
    let (ast, errors) = evt_file().parse(Tokens::new(&tokens)).into_output_errors();
    EvtParseResult {
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

//! The `.evt` event-contract grammar: events, channels,
//! subscriptions, and the fault-tolerant file parser.

use crate::lexer::{lex, Token};

use super::imports::import_decl;
use super::*;
/// One top-level item of an `.evt` file.
#[derive(Clone)]
pub(crate) enum EvtFileItem {
    Import(Import),
    Event(EventDecl),
    Channel(ChannelDecl),
    Subscription(SubscriptionDecl),
    Junk,
}

/// One member of an event body: a payload field, a governance line, or the
/// `cloudEvents` block. Members may appear in any source order; the fold
/// below groups them — repeated governance lines and `cloudEvents` blocks
/// overwrite (the last wins), `pii` entries append.
#[derive(Clone)]
pub(crate) enum EvtEventMember {
    Field(EventFieldDecl),
    Owner(Name),
    Classification(Name),
    Retention(String),
    Pii(Vec<Name>),
    CloudEvents(CloudEventsDecl),
}

/// One `name: type_ref;` payload field of an [`EventDecl`]. Committed once
/// the field name matched: a missing `:`, type, or `;` is a reported error
/// and the field still recovers (with an empty type when absent), so one
/// malformed field never loses the surrounding event. The field name is
/// ordinarily a name — plus the `type` keyword itself (the qname-segment
/// rule: `type` is special only inside the `cloudEvents` block, so it
/// remains usable as a payload field name without an escape).
pub(crate) fn evt_field_name<'src>() -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone
{
    name().or(any().try_map(|token: Token<'src>, span| match token {
        Token::Type => Ok(Name {
            text: "type".to_string(),
            span,
            escaped: false,
        }),
        _ => Err(Rich::custom(span, "expected a field name")),
    }))
}

pub(crate) fn event_field<'src>(
) -> impl Parser<'src, Tokens<'src>, EventFieldDecl, MoxExtra<'src>> + Clone {
    evt_field_name()
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

/// The contextual `word` of an `.evt` body: an ordinary identifier that is
/// special only in the grammar position its clause parser commits to (the
/// delegation-`to` precedent). Escaped `^word` never matches.
pub(crate) fn evt_word<'src>(
    word: &'static str,
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == word => e.span() }
}

/// One committed `owner <name>;` / `classification <word>;` governance line
/// of an event body: once the contextual word matched, a missing value or
/// `;` is a reported error and the line still recovers.
fn evt_name_line<'src>(
    word: &'static str,
) -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    evt_word(word)
        .ignore_then(name().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(value, semi), e| (value, semi, e.span()))
        .validate(move |(value, semi, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    format!("expected a name after `{word}`"),
                ));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    format!("expected `;` after the `{word}` line"),
                ));
            }
            value.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            })
        })
}

/// The `retention "<period>";` governance line of an event body, committed
/// like [`evt_name_line`] (the string value is kept verbatim).
fn evt_retention_line<'src>() -> impl Parser<'src, Tokens<'src>, String, MoxExtra<'src>> + Clone {
    evt_word("retention")
        .ignore_then(string_lit().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(value, semi), e| (value, semi, e.span()))
        .validate(|(value, semi, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected a retention string after `retention`",
                ));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `;` after the `retention` line",
                ));
            }
            value.unwrap_or_default()
        })
}

/// The `pii [ <field>* ];` governance line of an event body, committed once
/// the contextual word matched: missing brackets or `;` are reported errors
/// and the list still recovers.
fn evt_pii_line<'src>() -> impl Parser<'src, Tokens<'src>, Vec<Name>, MoxExtra<'src>> + Clone {
    evt_word("pii")
        .ignore_then(
            kw(Token::LBracket)
                .ignore_then(name().repeated().collect::<Vec<_>>())
                .then(kw(Token::RBracket).map_with(|_, e| e.span()).or_not())
                .map_with(|(fields, close), e| (fields, close, e.span()))
                .map(Some)
                .or_not(),
        )
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(list, semi), e| (list, semi, e.span()))
        .validate(|(list, semi, span), _e, emitter| {
            // The inner layer is always `Some` (the `.map(Some)` follows a
            // complete parse), so flattening keeps exactly the "no `[`"
            // case as `None`.
            let (fields, close, _list_span) = match list.flatten() {
                Some(list) => list,
                None => {
                    emitter.emit(Rich::custom(span, "expected `[` after `pii`"));
                    (Vec::new(), None, span)
                }
            };
            if close.is_none() {
                emitter.emit(Rich::custom(span, "expected `]` after the `pii` list"));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the `pii` list"));
            }
            fields
        })
}

/// One committed `type`/`source`/`subject` string entry of a `cloudEvents`
/// block: once the word matched, a missing string is a reported error and
/// the entry still recovers. The outer `None` means the word never matched
/// (an entry required by the block is missing); `Some(None)` means the word
/// matched but its string was missing (already diagnosed). `type` is the
/// real lexer keyword (the `.mox` datatype word); `source`/`subject` are
/// contextual identifiers.
fn evt_ce_entry<'src, P>(
    word: P,
    missing_value: &'static str,
) -> impl Parser<'src, Tokens<'src>, Option<Option<String>>, MoxExtra<'src>> + Clone
where
    P: Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone,
{
    word.ignore_then(string_lit().or_not())
        .map_with(|value, e| (value, e.span()))
        .validate(move |(value, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(span, missing_value));
            }
            value
        })
        .map(Some)
        .or_not()
        .map(Option::flatten)
}

/// The `cloudEvents { type "..." source "..." ("subject" "...")? }` block of
/// an event body. Committed once the contextual word matched: a missing
/// `{`, `type`, or `source` entry is a reported error (both attributes are
/// required once the block is present) and the block still recovers. With
/// the `{` missing the parser commits to the recovery consuming only the
/// word, so the event's own closing `}` is never swallowed. The entries
/// carry no `;` — the block's grammar is bracket-only.
pub(crate) fn cloud_events_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, CloudEventsDecl, MoxExtra<'src>> + Clone {
    let block = kw(Token::LBrace)
        .ignore_then(evt_ce_entry(
            kw(Token::Type),
            "expected a type string after `type`",
        ))
        .then(evt_ce_entry(
            evt_word("source"),
            "expected a source string after `source`",
        ))
        .then(evt_ce_entry(
            evt_word("subject"),
            "expected a subject string after `subject`",
        ))
        .then(kw(Token::RBrace).map_with(|_, e| e.span()).or_not())
        .map_with(|(((ty, source), subject), close), _| (ty, source, subject, close));
    evt_word("cloudEvents")
        .ignore_then(block.or_not())
        .map_with(|block, e| (block, e.span()))
        .validate(|(block, span), _e, emitter| {
            let Some((ty, source, subject, close)) = block else {
                emitter.emit(Rich::custom(span, "expected `{` after `cloudEvents`"));
                return CloudEventsDecl {
                    r#type: String::new(),
                    source: String::new(),
                    subject: None,
                    span,
                };
            };
            if ty.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected the `type` entry in the `cloudEvents` block",
                ));
            }
            if source.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected the `source` entry in the `cloudEvents` block",
                ));
            }
            if close.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `}` after the `cloudEvents` block",
                ));
            }
            CloudEventsDecl {
                r#type: ty.flatten().unwrap_or_default(),
                source: source.flatten().unwrap_or_default(),
                subject: subject.flatten(),
                span,
            }
        })
}

/// One member of an event body (any source order): a governance line, the
/// `cloudEvents` block, or a payload field. The governance words and
/// `cloudEvents` are contextual — special only as member introducers, so a
/// payload field named `type` (the keyword lookalike, see
/// [`evt_field_name`]) still parses as a field.
pub(crate) fn event_body_member<'src>(
) -> impl Parser<'src, Tokens<'src>, EvtEventMember, MoxExtra<'src>> + Clone {
    choice((
        evt_name_line("owner").map(EvtEventMember::Owner),
        evt_name_line("classification").map(EvtEventMember::Classification),
        evt_retention_line().map(EvtEventMember::Retention),
        evt_pii_line().map(EvtEventMember::Pii),
        cloud_events_decl().map(EvtEventMember::CloudEvents),
        event_field().map(EvtEventMember::Field),
    ))
}

/// An `event <name> ("version" string)? { member* }` declaration. The
/// `version` clause is committed once the keyword matched: a missing version
/// string is a reported error and the event still recovers. Members may
/// appear in any source order; the fold groups them into the canonical
/// shape (fields, then governance, then CloudEvents) — repeated governance
/// lines and `cloudEvents` blocks overwrite (the last wins), `pii` appends.
pub(crate) fn event_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, EventDecl, MoxExtra<'src>> + Clone {
    kw(Token::Event)
        .ignore_then(name())
        .then(kw(Token::Version).or_not().then(string_lit().or_not()))
        .then_ignore(kw(Token::LBrace))
        .then(event_body_member().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|((name, (version_kw, version)), members), e| {
            (name, version_kw, version, members, e.span())
        })
        .validate(|(name, version_kw, version, members, span), _e, emitter| {
            if version_kw.is_some() && version.is_none() {
                emitter.emit(Rich::custom(
                    name.span,
                    "expected a version string after `version`",
                ));
            }
            let mut decl = EventDecl {
                name,
                version,
                fields: Vec::new(),
                owner: None,
                classification: None,
                retention: None,
                pii: Vec::new(),
                cloud_events: None,
                span,
            };
            for member in members {
                match member {
                    EvtEventMember::Field(field) => decl.fields.push(field),
                    EvtEventMember::Owner(owner) => decl.owner = Some(owner),
                    EvtEventMember::Classification(classification) => {
                        decl.classification = Some(classification)
                    }
                    EvtEventMember::Retention(retention) => decl.retention = Some(retention),
                    EvtEventMember::Pii(pii) => decl.pii.extend(pii),
                    EvtEventMember::CloudEvents(cloud_events) => {
                        decl.cloud_events = Some(cloud_events)
                    }
                }
            }
            decl
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

/// The contextual `of` word of a channel header: an ordinary identifier
/// that is special only immediately after the channel's name (the
/// delegation-`to` precedent). Escaped `^of` never matches.
pub(crate) fn evt_of_keyword<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone
{
    select! { Token::Ident(text) = e if text == "of" => e.span() }
}

/// The contextual `scope` word of a subscription body (same discipline as
/// [`evt_consumer_keyword`]).
pub(crate) fn evt_scope_keyword<'src>(
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == "scope" => e.span() }
}

/// A `channel <name> ("of" <scope>)? { publishes_entry* }` declaration. The
/// `of` clause is committed once the contextual word matched: a missing
/// scope name is a reported error and the channel still recovers. A stray
/// name without the leading `of` is reported too (after the channel name
/// only `of` or `{` is legal), so nothing is silently swallowed.
pub(crate) fn channel_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, ChannelDecl, MoxExtra<'src>> + Clone {
    kw(Token::Channel)
        .ignore_then(name())
        .then(evt_of_keyword().or_not())
        .then(name().or_not())
        .then_ignore(kw(Token::LBrace))
        .then(publishes_entry().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(((name, of_kw), scope), publishes), e| {
            (name, of_kw, scope, publishes, e.span())
        })
        .validate(|(name, of_kw, scope, publishes, span), _e, emitter| {
            if of_kw.is_some() && scope.is_none() {
                emitter.emit(Rich::custom(span, "expected a scope name after `of`"));
            }
            if of_kw.is_none() {
                if let Some(scope) = &scope {
                    emitter.emit(Rich::custom(
                        scope.span,
                        "expected `of` or `{` after the channel name",
                    ));
                }
            }
            ChannelDecl {
                name,
                scope,
                publishes,
                span,
            }
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

/// One committed `mode <word>;` / `maxAttempts <int>;` / `backoffSeconds
/// <int>;` / `timeoutSeconds <int>;` / `deadLetter <channel>;` entry of a
/// `delivery` block: once the contextual word matched, a missing value or
/// `;` is a reported error and the entry still recovers. The outer `None`
/// means the word never matched; `Some(None)` means the word matched but
/// its value was missing (already diagnosed).
fn evt_delivery_entry<'src, V, W, P>(
    word: W,
    missing_value: &'static str,
    entry: &'static str,
    value: P,
) -> impl Parser<'src, Tokens<'src>, Option<Option<V>>, MoxExtra<'src>> + Clone
where
    V: Clone + 'static,
    W: Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone,
    P: Parser<'src, Tokens<'src>, V, MoxExtra<'src>> + Clone,
{
    word.ignore_then(value.or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(move |(value, semi), e| (value, semi, e.span()))
        .validate(move |(value, semi, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(span, missing_value));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    format!("expected `;` after the `{entry}` entry"),
                ));
            }
            value
        })
        .map(Some)
        .or_not()
        .map(Option::flatten)
}

/// The `delivery { mode <word>; ... }` block of a subscription body.
/// Committed once the contextual word matched: a missing `{`, the required
/// `mode` entry, or the closing `}` is a reported error and the block still
/// recovers. With the `{` missing the parser commits to the recovery
/// consuming only the word, so the subscription's own closing `}` is never
/// swallowed. The entries are a fixed chain in canonical order — `mode`
/// first, then the optional tuning knobs.
pub(crate) fn delivery_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DeliveryDecl, MoxExtra<'src>> + Clone {
    let mode_entry = evt_delivery_entry(
        evt_word("mode"),
        "expected a delivery mode after `mode`",
        "mode",
        name(),
    );
    let attempts_entry = evt_delivery_entry(
        evt_word("maxAttempts"),
        "expected an int after `maxAttempts`",
        "maxAttempts",
        int_lit(),
    );
    let backoff_entry = evt_delivery_entry(
        evt_word("backoffSeconds"),
        "expected an int after `backoffSeconds`",
        "backoffSeconds",
        int_lit(),
    );
    let timeout_entry = evt_delivery_entry(
        evt_word("timeoutSeconds"),
        "expected an int after `timeoutSeconds`",
        "timeoutSeconds",
        int_lit(),
    );
    let dead_letter_entry = evt_delivery_entry(
        evt_word("deadLetter"),
        "expected a channel name after `deadLetter`",
        "deadLetter",
        name(),
    );
    let block = kw(Token::LBrace)
        .ignore_then(mode_entry)
        .then(attempts_entry)
        .then(backoff_entry)
        .then(timeout_entry)
        .then(dead_letter_entry)
        .then(kw(Token::RBrace).map_with(|_, e| e.span()).or_not())
        .map_with(
            |(((((mode, attempts), backoff), timeout), dead_letter), close), _| {
                (mode, attempts, backoff, timeout, dead_letter, close)
            },
        );
    evt_word("delivery")
        .ignore_then(block.or_not())
        .map_with(|block, e| (block, e.span()))
        .validate(|(block, span), _e, emitter| {
            let Some((mode, attempts, backoff, timeout, dead_letter, close)) = block else {
                emitter.emit(Rich::custom(span, "expected `{` after `delivery`"));
                return DeliveryDecl {
                    mode: None,
                    max_attempts: None,
                    backoff_seconds: None,
                    timeout_seconds: None,
                    dead_letter: None,
                    span,
                };
            };
            if mode.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected a `mode` entry in the `delivery` block",
                ));
            }
            if close.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `}` after the `delivery` block",
                ));
            }
            DeliveryDecl {
                mode: mode.flatten(),
                max_attempts: attempts.flatten(),
                backoff_seconds: backoff.flatten(),
                timeout_seconds: timeout.flatten(),
                dead_letter: dead_letter.flatten(),
                span,
            }
        })
}

/// The `when (...)` filter clause of a subscription body — the real lexer
/// keyword (escapable like every keyword). Committed once the keyword
/// matched: a missing parenthesized condition is a reported error and the
/// clause still recovers. The parens are a [`raw_parens`] span: the driver
/// slices the interior strictly inside and types it.
pub(crate) fn evt_when_clause<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<Span>, MoxExtra<'src>> + Clone {
    kw(Token::When)
        .ignore_then(raw_parens().map(Some).or_not())
        .map_with(|parens, e| (parens, e.span()))
        .validate(|(parens, span), _e, emitter| {
            if parens.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected a `(...)` condition after `when`",
                ));
            }
            parens
        })
        .map(Option::flatten)
}

/// A `subscription <name> { events [ <event>* ] consumer <name>
/// ("scope" <name>)? ("when" (...))? ("delivery" {...})? }` declaration.
/// Once the body's `{` matched the declaration is committed: missing
/// `events`, list brackets, `consumer`, the consumer name, the scope name,
/// the `when` condition, or any `delivery` piece are each reported errors
/// and the subscription still recovers. Inside the `[...]` list the
/// contextual `consumer` word ends the list (the word introducing the next
/// clause), so a missing `]` never swallows the consumer line; after the
/// consumer name the contextual `delivery` word is likewise reserved (the
/// next clause's introducer — `scope ^delivery` escapes). A stray name
/// after the consumer without the leading `scope` is reported too. The
/// clauses follow the canonical order `events` → `consumer` → `scope` →
/// `when` → `delivery`.
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
    // The scope-name / stray-name slot: any name except the unescaped
    // `delivery` word, which introduces the next clause (the
    // `consumer`-ends-the-list precedent; `^delivery` and `scope ^delivery`
    // stay legal names).
    let scope_name = select! {
        Token::Ident(text) = e if text != "delivery" => Name {
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
        .then(evt_scope_keyword().or_not())
        .then(scope_name.or_not())
        .then(evt_when_clause().or_not())
        .then(delivery_decl().or_not())
        .then_ignore(kw(Token::RBrace).or_not())
        .map_with(
            |(
                (((((((name, events_kw), list), consumer_kw), consumer), scope_kw), scope), when),
                delivery,
            ),
             e| {
                (
                    name,
                    events_kw,
                    list,
                    consumer_kw,
                    consumer,
                    scope_kw,
                    scope,
                    when,
                    delivery,
                    e.span(),
                )
            },
        )
        .validate(
            |(
                name,
                events_kw,
                list,
                consumer_kw,
                consumer,
                scope_kw,
                scope,
                when,
                delivery,
                span,
            ),
             _e,
             emitter| {
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
                if scope_kw.is_some() && scope.is_none() {
                    emitter.emit(Rich::custom(span, "expected a scope name after `scope`"));
                }
                if scope_kw.is_none() {
                    if let Some(scope) = &scope {
                        emitter.emit(Rich::custom(
                            scope.span,
                            "expected `scope`, `when`, `delivery`, or `}` after the consumer name",
                        ));
                    }
                }
                SubscriptionDecl {
                    name,
                    events,
                    consumer: consumer.unwrap_or_else(|| Name {
                        text: String::new(),
                        span,
                        escaped: false,
                    }),
                    scope,
                    when: when.flatten(),
                    delivery,
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

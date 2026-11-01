//! The `.uc` use case grammar: participants, occurrences, steps,
//! messages, successions, realizations, and the file parser.

use crate::lexer::{lex_with_comments, Token};
use crate::usecases;

use super::docs::attach_uc_docs;
use super::*;
/// A contextual `.uc` keyword: an ordinary identifier that is special only
/// in the grammar position it is parsed here in (the `.deploy` discipline).
/// Escaped forms (`^usecase`) are different token variants and never match.
pub(crate) fn uc_keyword<'src>(
    keyword: &'static str,
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == keyword => e.span() }
}

/// An `import "<path>" (as <name>)?` declaration of a `.uc` file: the
/// `.actor` import production plus the optional `as` alias. Committed once
/// the `as` keyword matched: a missing alias name is a reported error.
/// Without `as` nothing else is consumed (the next word may be a usecase).
pub(crate) fn uc_import_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, UcImport, MoxExtra<'src>> + Clone {
    kw(Token::Import)
        .ignore_then(string_lit())
        .then(
            // Committed once `as` matched: a missing alias name is a
            // reported error and the clause still recovers (the `as`
            // stays consumed). Without `as` nothing else is consumed —
            // the next word may begin a usecase.
            kw(Token::As)
                .ignore_then(name().map(Some).or_not().map(Some))
                .or_not(),
        )
        .map_with(|(path, alias), e| (path, alias, e.span()))
        .validate(|(path, alias, span), _e, emitter| {
            if matches!(&alias, Some(Some(None))) {
                emitter.emit(Rich::custom(span, "expected a name after `as`"));
            }
            UcImport {
                path,
                alias: match alias {
                    Some(Some(Some(Some(alias)))) => Some(alias.text),
                    _ => None,
                },
                span,
            }
        })
}

/// One member of a participant body.
#[derive(Clone)]
pub(crate) enum UcBodyMember {
    Occurrence(UcOccurrence),
    Step(UcStep),
}

/// Consumes a run of tokens that cannot start or continue a body member,
/// stopping before the body's closing `}` and before identifiers and the
/// `actor` keyword (which could begin the next member). Always consumes at
/// least one token.
pub(crate) fn junk_uc_member<'src>(
    expected: &'static str,
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(
            t,
            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_) | Token::Actor
        ) =>
        ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(move |(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), expected));
        })
}

/// An `occurrence <name> ["." ("source"|"target")] ";"` line of a
/// participant body. Committed once `occurrence` matched: a missing name
/// or `;` is a reported error; a `.` tail must be the contextual
/// `source`/`target` word (anything else recovers as a standalone
/// occurrence with a reported error).
pub(crate) fn uc_occurrence_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, UcOccurrence, MoxExtra<'src>> + Clone {
    uc_keyword(usecases::OCCURRENCE)
        .ignore_then(name().or_not())
        .then(
            // `tail`: `Some(Some(target))` = `.` + source/target;
            // `Some(None)` = `.` with a bad tail; `None` = no `.` at all.
            kw(Token::Dot)
                .ignore_then(
                    uc_keyword(usecases::SOURCE)
                        .to(false)
                        .or(uc_keyword(usecases::TARGET).to(true))
                        .or_not(),
                )
                .or_not(),
        )
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((name, tail), semi), e| (name, tail, semi, e.span()))
        .validate(|(name, tail, semi, span), _e, emitter| {
            let name = name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if name.text.is_empty() {
                emitter.emit(Rich::custom(
                    span,
                    "expected an occurrence name after `occurrence`",
                ));
            }
            let occurrence = match tail {
                Some(Some(target)) => UcOccurrence::MessageEnd {
                    message: name.text,
                    target,
                },
                Some(None) => {
                    emitter.emit(Rich::custom(
                        span,
                        "expected `source` or `target` after `.`",
                    ));
                    UcOccurrence::Named(name.text)
                }
                None => UcOccurrence::Named(name.text),
            };
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the occurrence"));
            }
            occurrence
        })
}

/// One capability name of a `requires [...]` list: any identifier except
/// the two words that can begin the next member, so a missing `]` never
/// swallows it (the `.evt` events-list rule). Escaped forms are names.
pub(crate) fn uc_req_name<'src>() -> impl Parser<'src, Tokens<'src>, String, MoxExtra<'src>> + Clone
{
    select! {
        Token::Ident(text) = e if text != usecases::OCCURRENCE && text != usecases::PERFORM => {
            let _ = e;
            text.to_string()
        },
        Token::IdentEscaped(text) = e => {
            let _ = e;
            text.to_string()
        },
    }
}

/// A `perform <name> [":" <op>] ["requires" "[" <name>* "]"] ";"` step of
/// a participant body. The bare form names the operation after the step;
/// the `:`/`requires` clauses are committed once their leading token
/// matched (missing pieces are reported errors and the step recovers).
pub(crate) fn uc_perform_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, UcStep, MoxExtra<'src>> + Clone {
    // The named form `: <op>` is committed once the colon matched; without
    // a colon nothing is consumed (a bare step's next word may be
    // `requires`).
    let operation = kw(Token::Other(':'))
        .ignore_then(name().map(Some))
        .map(|op| (op, true))
        .or_not();
    let requires = uc_keyword(usecases::REQUIRES)
        .ignore_then(kw(Token::LBracket).or_not())
        .then(uc_req_name().repeated().collect::<Vec<_>>())
        .then(kw(Token::RBracket).or_not())
        .map_with(|((bracket, names), close), e| (bracket, names, close, e.span()));
    uc_keyword(usecases::PERFORM)
        .ignore_then(name().or_not())
        .then(operation)
        .then(requires.or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(((name, operation), requires), semi), e| {
            (name, operation, requires, semi, e.span())
        })
        .validate(|(name, operation, requires, semi, span), _e, emitter| {
            let name = name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if name.text.is_empty() {
                emitter.emit(Rich::custom(span, "expected a step name after `perform`"));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the perform step"));
            }
            let (op, colon_seen) = operation.unwrap_or((None, false));
            if colon_seen && op.is_none() {
                emitter.emit(Rich::custom(
                    name.span,
                    "expected an operation name after `:`",
                ));
            }
            let mut requires_names = Vec::new();
            if let Some((bracket, names, close, list_span)) = requires {
                if bracket.is_none() {
                    emitter.emit(Rich::custom(list_span, "expected `[` after `requires`"));
                }
                if close.is_none() {
                    emitter.emit(Rich::custom(
                        list_span,
                        "expected `]` after the requires list",
                    ));
                }
                requires_names = names;
            }
            UcStep {
                operation: if colon_seen {
                    // Recovered with an empty name when `:` had none.
                    op.map(|op| op.text).unwrap_or_default()
                } else {
                    // The bare form: the step names its own operation.
                    name.text.clone()
                },
                name: name.text,
                requires: requires_names,
                span,
            }
        })
}

/// A `participant.anchor` endpoint pair of a message clause. Committed once
/// the participant name matched: a missing `.` or anchor is a reported
/// error and the endpoint recovers with an empty anchor.
pub(crate) fn uc_endpoint<'src>(
) -> impl Parser<'src, Tokens<'src>, UcEndpoint, MoxExtra<'src>> + Clone {
    name()
        .then(kw(Token::Dot).or_not())
        .then(name().or_not())
        .map_with(|((participant, dot), anchor), e| (participant, dot, anchor, e.span()))
        .validate(|(participant, dot, anchor, span), _e, emitter| {
            if dot.is_none() {
                emitter.emit(Rich::custom(
                    participant.span,
                    "expected `participant.anchor`",
                ));
            } else if anchor.is_none() {
                emitter.emit(Rich::custom(
                    participant.span,
                    "expected an anchor name after `.`",
                ));
            }
            UcEndpoint {
                participant: participant.text,
                anchor: anchor.map(|anchor| anchor.text).unwrap_or_default(),
                span,
            }
        })
}

/// A `message <name> ["of" <type>] ["from" <endpoint>] ["to" <endpoint>]
/// ";"` declaration. Committed once `message` matched: a missing name or
/// `;` is a reported error; each clause is committed once its preposition
/// matched.
pub(crate) fn uc_message_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, UcMessage, MoxExtra<'src>> + Clone {
    uc_keyword(usecases::MESSAGE)
        .ignore_then(name().or_not())
        .then(uc_keyword(usecases::OF).ignore_then(tref()).or_not())
        .then(kw(Token::From).ignore_then(uc_endpoint()).or_not())
        .then(uc_keyword(usecases::TO).ignore_then(uc_endpoint()).or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((((name, payload), source), target), semi), e| {
            (name, payload, source, target, semi, e.span())
        })
        .validate(|(name, payload, source, target, semi, span), _e, emitter| {
            let name = name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if name.text.is_empty() {
                emitter.emit(Rich::custom(
                    span,
                    "expected a message name after `message`",
                ));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the message"));
            }
            UcMessage {
                name: name.text,
                payload,
                source,
                target,
                span,
            }
        })
}

/// One node reference of a succession: the contextual `start`/`done`
/// words, a bare name, or a `participant.anchor` pair (a qualified
/// reference; a message-end pair reads the same). The qualified
/// alternative is tried first, so `start.anchor` is qualified while a
/// bare `start` is the initial node.
pub(crate) fn uc_node_ref<'src>(
) -> impl Parser<'src, Tokens<'src>, UcNodeRef, MoxExtra<'src>> + Clone {
    let qualified = name()
        .then_ignore(kw(Token::Dot))
        .then(name())
        .map(|(participant, anchor)| UcNodeRef::Qualified(participant.text, anchor.text));
    let start = uc_keyword(usecases::START).to(UcNodeRef::Start);
    let done = uc_keyword(usecases::DONE).to(UcNodeRef::Done);
    let bare = name().map(|name| UcNodeRef::Bare(name.text));
    choice((qualified, start, done, bare))
}

/// A `first <node> then <node>;` statement. Committed once `first` and the
/// first node matched: a missing `then` half or `;` is a reported error.
pub(crate) fn uc_succession_stmt<'src>(
) -> impl Parser<'src, Tokens<'src>, UcSuccession, MoxExtra<'src>> + Clone {
    uc_keyword(usecases::FIRST)
        .ignore_then(uc_node_ref())
        .then(
            // `Some(node)` = `then` + node; `None` = no (complete) `then`
            // half. A bare `then` whose node never came rolls the whole
            // half back, so its error surfaces below.
            uc_keyword(usecases::THEN)
                .ignore_then(uc_node_ref())
                .or_not(),
        )
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((first, then_half), semi), e| (first, then_half, semi, e.span()))
        .validate(|(first, then_half, semi, span), _e, emitter| {
            let then = then_half.unwrap_or_else(|| {
                emitter.emit(Rich::custom(
                    span,
                    "expected `then <node>` after the first node reference",
                ));
                UcNodeRef::Bare(String::new())
            });
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the succession"));
            }
            UcSuccession { first, then, span }
        })
}

/// A `realizes <alias>.<Service>.<operation>;` binding. Committed once
/// `realizes` matched: a reference that is not exactly three segments (the
/// `alias.Service.operation` shape) or a missing `;` is a reported error,
/// and the binding recovers with the segments it has.
pub(crate) fn uc_realizes_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, UcRealizes, MoxExtra<'src>> + Clone {
    uc_keyword(usecases::REALIZES)
        .ignore_then(qname())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(name, semi), e| (name, semi, e.span()))
        .validate(|(name, semi, span), _e, emitter| {
            if semi.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `;` after the realizes declaration",
                ));
            }
            let segment = |index: usize| {
                name.segments
                    .get(index)
                    .map(|s| s.text.clone())
                    .unwrap_or_default()
            };
            let (alias, service, operation) = match name.segments.as_slice() {
                [alias, service, operation] => (
                    alias.text.clone(),
                    service.text.clone(),
                    operation.text.clone(),
                ),
                _ => {
                    emitter.emit(Rich::custom(
                        name.span,
                        "expected `alias.Service.operation`",
                    ));
                    (segment(0), segment(1), segment(2))
                }
            };
            UcRealizes {
                alias,
                service,
                operation,
                span,
            }
        })
}

/// A `subject`/`actor` participant declaration with its optional
/// `occurrence`/`perform` body. A `subject` requires its `: <type>` clause
/// (a reported error when missing); an `actor` may be bare — a policy
/// actor with no type.
pub(crate) fn uc_participant_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, (bool, UcParticipant), MoxExtra<'src>> + Clone {
    let head = uc_keyword(usecases::SUBJECT)
        .map(|span| (true, span))
        .or(kw(Token::Actor).map(|span| (false, span)));
    let typed = kw(Token::Other(':'))
        .ignore_then(tref().map(Some))
        .map(|ty| (ty, true))
        .or_not();
    let body = kw(Token::LBrace)
        .ignore_then(
            choice((
                uc_occurrence_decl().map(UcBodyMember::Occurrence),
                uc_perform_decl().map(UcBodyMember::Step),
            ))
            .map(Some)
            .or(junk_uc_member("expected an occurrence or perform declaration").to(None))
            .repeated()
            .collect::<Vec<_>>(),
        )
        .then_ignore(kw(Token::RBrace))
        .or_not();
    head.then(name())
        .then(typed)
        .then(body)
        .map_with(|(((is_subject, name), typed), members), e| {
            (is_subject, name, typed, members, e.span())
        })
        .validate(|(is_subject, name, typed, members, span), _e, emitter| {
            let head_span = is_subject.1;
            let is_subject = is_subject.0;
            let (ty, colon_seen) = typed.unwrap_or((None, false));
            if colon_seen && ty.is_none() {
                emitter.emit(Rich::custom(head_span, "expected a type after `:`"));
            }
            if is_subject && !colon_seen {
                emitter.emit(Rich::custom(
                    head_span,
                    "expected `: <type>` after the subject name",
                ));
            }
            let mut occurrences = Vec::new();
            let mut steps = Vec::new();
            for member in members.into_iter().flatten().flatten() {
                match member {
                    UcBodyMember::Occurrence(occurrence) => occurrences.push(occurrence),
                    UcBodyMember::Step(step) => steps.push(step),
                }
            }
            (
                is_subject,
                UcParticipant {
                    name: name.text,
                    type_name: ty.map(|ty| ty.name.full_name()),
                    occurrences,
                    steps,
                    span,
                },
            )
        })
}

/// One member of a usecase body.
#[derive(Clone)]
pub(crate) enum UcItem {
    Subject(UcParticipant),
    Actor(UcParticipant),
    Message(UcMessage),
    Succession(UcSuccession),
    Realizes(UcRealizes),
}

/// A `usecase <name> { ... }` declaration. The four member kinds may be
/// interleaved in any order; the AST folds them into per-kind lists, with
/// a second `subject` line reported as an error (the first wins).
pub(crate) fn usecase_decl<'src>() -> impl Parser<'src, Tokens<'src>, UcDecl, MoxExtra<'src>> + Clone
{
    let item = choice((
        uc_participant_decl().map(|(is_subject, participant)| {
            if is_subject {
                UcItem::Subject(participant)
            } else {
                UcItem::Actor(participant)
            }
        }),
        uc_message_decl().map(UcItem::Message),
        uc_succession_stmt().map(UcItem::Succession),
        uc_realizes_decl().map(UcItem::Realizes),
    ));
    let member = item
        .map(Some)
        .or(junk_uc_member("expected a usecase member").to(None));
    uc_keyword(usecases::USECASE)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(member.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, members), e| {
            let mut decl = UcDecl {
                name: name.text,
                description: None,
                subject: None,
                actors: Vec::new(),
                messages: Vec::new(),
                successions: Vec::new(),
                realizes: Vec::new(),
                span: e.span(),
            };
            let mut problems = Vec::new();
            for member in members.into_iter().flatten() {
                match member {
                    UcItem::Subject(participant) => {
                        if decl.subject.is_none() {
                            decl.subject = Some(participant);
                        } else {
                            problems.push((
                                participant.span,
                                "duplicate `subject` declaration".to_string(),
                            ));
                        }
                    }
                    UcItem::Actor(participant) => decl.actors.push(participant),
                    UcItem::Message(message) => decl.messages.push(message),
                    UcItem::Succession(succession) => decl.successions.push(succession),
                    UcItem::Realizes(realizes) => decl.realizes.push(realizes),
                }
            }
            (decl, problems)
        })
        .validate(|(decl, problems), _, emitter| {
            for (span, message) in problems {
                emitter.emit(Rich::custom(span, message));
            }
            decl
        })
}

/// Consumes a run of tokens up to the next `import` keyword or the
/// contextual `usecase` word (or end of input). File-level recovery for
/// `.uc` files: always consumes at least one token (guaranteeing progress)
/// and emits an error for the skipped region.
pub(crate) fn junk_uc_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let rest = select! {
        t if !matches!(t, Token::Import | Token::Ident(usecases::USECASE)) => ()
    };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(
                e.span(),
                "expected an import or usecase declaration",
            ));
        })
}

/// One top-level item of a `.uc` file.
#[derive(Clone)]
pub(crate) enum UcFileItem {
    Import(UcImport),
    UseCase(Box<UcDecl>),
    Junk,
}

pub(crate) fn fold_uc_file(items: Vec<UcFileItem>) -> (UcFile, Option<Span>) {
    let mut imports = Vec::new();
    let mut use_cases = Vec::new();
    // Span of the first `import` that follows a usecase (a syntax error
    // reported by the caller's `validate`).
    let mut late_import = None;
    for item in items {
        match item {
            UcFileItem::Import(decl) => {
                if use_cases.is_empty() {
                    imports.push(decl);
                } else if late_import.is_none() {
                    late_import = Some(decl.span);
                }
            }
            UcFileItem::UseCase(decl) => use_cases.push(*decl),
            UcFileItem::Junk => {}
        }
    }
    (UcFile { imports, use_cases }, late_import)
}

pub(crate) fn uc_file<'src>() -> impl Parser<'src, Tokens<'src>, UcFile, MoxExtra<'src>> + Clone {
    let item = choice((
        uc_import_decl().map(UcFileItem::Import),
        usecase_decl().map(|decl| UcFileItem::UseCase(Box::new(decl))),
    ))
    .or(junk_uc_file().to(UcFileItem::Junk));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map(fold_uc_file)
        .validate(|(file, late_import), _, emitter| {
            if let Some(span) = late_import {
                emitter.emit(Rich::custom(span, "`import` after a usecase declaration"));
            }
            file
        })
}

/// The outcome of parsing a `.uc` source: as much of the file as could be
/// recovered, plus all encountered errors. Parsing never panics and always
/// produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct UcParseResult {
    /// The recovered use case file, or `None` if no output could be
    /// produced at all.
    pub ast: Option<UcFile>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

/// Lex and parse a `.uc` source text: `import` declarations followed by
/// `usecase` declarations (imports must come first).
///
/// This function never panics and always recovers as much of the AST as
/// possible; check [`UcParseResult::errors`] for syntax problems. Doc
/// comment runs (`///`) directly above a `usecase` declaration become its
/// description.
pub fn parse_usecases(source: &str) -> UcParseResult {
    let (tokens, comments) = match lex_with_comments(source) {
        Ok(result) => result,
        Err(error) => {
            return UcParseResult {
                ast: None,
                errors: vec![ParseError {
                    message: error.to_string(),
                    span: error.span,
                }],
            }
        }
    };
    let (mut ast, errors) = uc_file().parse(Tokens::new(&tokens)).into_output_errors();
    if let Some(file) = ast.as_mut() {
        attach_uc_docs(file, &comments, source);
    }
    UcParseResult {
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

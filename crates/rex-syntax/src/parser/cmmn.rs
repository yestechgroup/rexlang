//! The `.cmmn` case-management grammar: cases, roles, case-file
//! items, plan items, and the file parser.

use crate::cmmn;
use crate::lexer::{lex_with_comments, Token};

use super::dmn::{dmn_bare_name, keyword_block};
use super::docs::{collect_docs, doc_run_above, line_of, line_starts};
use super::*;
pub(crate) fn cmmn_keyword<'src>(
    keyword: &'static str,
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == keyword => e.span() }
}

/// An `import "<path>" (as <name>)?` declaration of a `.cmmn` file: the
/// `.uc` import production. Committed once the `as` keyword matched: a
/// missing alias name is a reported error. Without `as` nothing else is
/// consumed (the next word may begin a case).
pub(crate) fn cmmn_import_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, Import, MoxExtra<'src>> + Clone {
    kw(Token::Import)
        .ignore_then(string_lit())
        .then(
            kw(Token::As)
                .ignore_then(name().map(Some).or_not().map(Some))
                .or_not(),
        )
        .map_with(|(path, alias), e| (path, alias, e.span()))
        .validate(|(path, alias, span), _e, emitter| {
            if matches!(&alias, Some(Some(None))) {
                emitter.emit(Rich::custom(span, "expected a name after `as`"));
            }
            Import {
                path,
                alias: match alias {
                    Some(Some(Some(Some(alias)))) => Some(alias.text),
                    _ => None,
                },
                span,
            }
        })
}

/// Consumes a run of tokens that cannot start or continue a body member,
/// stopping before the body's closing `}` and before identifiers (which
/// could begin the next member). Always consumes at least one token.
pub(crate) fn junk_cmmn_member<'src>(
    expected: &'static str,
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(
            t,
            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
        ) =>
        ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(move |(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), expected));
        })
}

/// A `role <name> (as <actor>)?;` line of a roles block. Committed once
/// `role` matched: a missing name, alias, or `;` is a reported error.
pub(crate) fn role_decl<'src>() -> impl Parser<'src, Tokens<'src>, CmmnRole, MoxExtra<'src>> + Clone
{
    cmmn_keyword(cmmn::ROLE)
        .ignore_then(name().or_not())
        .then(
            kw(Token::As)
                .ignore_then(name().map(Some).or_not().map(Some))
                .or_not(),
        )
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((name, actor), semi), e| (name, actor, semi, e.span()))
        .validate(|(name, actor, semi, span), _e, emitter| {
            let name = name.or_else(|| {
                emitter.emit(Rich::custom(span, "expected a role name after `role`"));
                None
            });
            if matches!(&actor, Some(Some(None))) {
                emitter.emit(Rich::custom(span, "expected an actor name after `as`"));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the role"));
            }
            let actor = match actor {
                Some(Some(Some(Some(actor)))) => Some(actor),
                _ => None,
            };
            CmmnRole {
                name: name.as_ref().map(|n| n.text.clone()).unwrap_or_default(),
                name_span: name.as_ref().map(|n| n.span).unwrap_or(span),
                actor: actor.as_ref().map(|actor| actor.text.clone()),
                actor_span: actor.as_ref().map(|actor| actor.span),
                span,
            }
        })
}

/// A `roles { ... }` block of a case body. A member that cannot start a
/// role is junk (recovered, reported).
pub(crate) fn roles_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<CmmnRole>, MoxExtra<'src>> + Clone {
    cmmn_keyword(cmmn::ROLES)
        .ignore_then(
            kw(Token::LBrace)
                .ignore_then(
                    role_decl()
                        .map(Some)
                        .or(junk_cmmn_member("expected a role declaration").to(None))
                        .repeated()
                        .collect::<Vec<_>>(),
                )
                .then_ignore(kw(Token::RBrace)),
        )
        .map(|roles| roles.into_iter().flatten().collect())
}

/// A case-file item declaration: the `.mox` attribute production
/// (`Type[mult]? name;`). Committed once the type matched: a missing name
/// or `;` is a reported error.
pub(crate) fn case_file_item<'src>(
) -> impl Parser<'src, Tokens<'src>, CmmnFileItem, MoxExtra<'src>> + Clone {
    tref()
        .then(multiplicity().or_not())
        .then(name().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(((type_ref, mult), name), semi), e| (type_ref, mult, name, semi, e.span()))
        .validate(|(type_ref, multiplicity, name, semi, span), _e, emitter| {
            let name = name.or_else(|| {
                emitter.emit(Rich::custom(span, "expected an item name after the type"));
                None
            });
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the case-file item"));
            }
            CmmnFileItem {
                name: name.as_ref().map(|n| n.text.clone()).unwrap_or_default(),
                name_span: name.as_ref().map(|n| n.span).unwrap_or(span),
                type_ref,
                multiplicity,
                description: None,
                span,
            }
        })
}

/// A `case_file { ... }` block of a case body.
pub(crate) fn case_file_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<CmmnFileItem>, MoxExtra<'src>> + Clone {
    cmmn_keyword(cmmn::CASE_FILE)
        .ignore_then(
            kw(Token::LBrace)
                .ignore_then(
                    case_file_item()
                        .map(Some)
                        .or(junk_cmmn_member("expected a case-file item declaration").to(None))
                        .repeated()
                        .collect::<Vec<_>>(),
                )
                .then_ignore(kw(Token::RBrace)),
        )
        .map(|items| items.into_iter().flatten().collect())
}

/// One on-part of a criterion: `name` (a domain event) or `name.tail` (a
/// plan item event or case-file transition — the driver validates the
/// tail). Committed once the name matched: a missing tail word is a
/// reported error.
pub(crate) fn cmmn_on_part<'src>(
) -> impl Parser<'src, Tokens<'src>, CmmnOnPart, MoxExtra<'src>> + Clone {
    name()
        .then(
            kw(Token::Dot)
                .ignore_then(name().map(Some).or_not().map(Some))
                .or_not(),
        )
        .map_with(|(source, tail), e| (source, tail, e.span()))
        .validate(|(source, tail, span), _e, emitter| {
            if matches!(&tail, Some(Some(None))) {
                emitter.emit(Rich::custom(span, "expected a transition word after `.`"));
            }
            CmmnOnPart {
                source: source.text.clone(),
                source_span: source.span,
                event: match tail {
                    Some(Some(Some(Some(word)))) => Some(word.text),
                    _ => None,
                },
                span,
            }
        })
}

/// An `entry`/`exit` criterion of a plan item body:
/// `entry on <part> (, <part>)* (when (...))?;`. Committed once the
/// `entry`/`exit` word matched: a missing `on`, on-part, `when` parens,
/// or `;` is a reported error.
pub(crate) fn criterion_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, (bool, CmmnCriterion), MoxExtra<'src>> + Clone {
    let head = cmmn_keyword(cmmn::ENTRY)
        .to(false)
        .or(cmmn_keyword(cmmn::EXIT).to(true));
    let parts = cmmn_on_part()
        .then(
            kw(Token::Comma)
                .ignore_then(cmmn_on_part())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| {
            let mut parts = Vec::with_capacity(rest.len() + 1);
            parts.push(first);
            parts.extend(rest);
            parts
        });
    let when = kw(Token::When)
        .ignore_then(raw_parens().map(Some).or_not().map(Some))
        .or_not();
    head.then(kw(Token::On).map(Some).or_not())
        .then(parts.or_not())
        .then(when)
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((((is_exit, on), parts), when), semi), e| {
            (is_exit, on, parts, when, semi, e.span())
        })
        .validate(|(is_exit, on, parts, when, semi, span), _e, emitter| {
            let head_word = if is_exit { "exit" } else { "entry" };
            if on.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    format!("expected `on` after `{head_word}`"),
                ));
            }
            if matches!(&when, Some(Some(None))) {
                emitter.emit(Rich::custom(span, "expected `(...)` after `when`"));
            }
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the criterion"));
            }
            let parts = parts.unwrap_or_default();
            if on.is_some() && parts.is_empty() {
                emitter.emit(Rich::custom(span, "expected an on-part after `on`"));
            }
            (
                is_exit,
                CmmnCriterion {
                    on_parts: parts,
                    when: match when {
                        Some(Some(Some(Some(parens)))) => Some(parens),
                        _ => None,
                    },
                    span,
                },
            )
        })
}

/// A `performer: <role>;` clause of a plan item body. Committed once
/// `performer` matched.
///
/// The tuple is `(role, clause span, role-name span)`: the clause span
/// carries the diagnostics, the name span is the mention site
/// ([`CmmnItemDecl::performer_span`]). A recovered clause without a role
/// name falls back to the clause span for both.
pub(crate) fn performer_clause<'src>(
) -> impl Parser<'src, Tokens<'src>, (String, Span, Span), MoxExtra<'src>> + Clone {
    cmmn_keyword(cmmn::PERFORMER)
        .ignore_then(
            kw(Token::Other(':'))
                .ignore_then(name().map(Some).or_not().map(Some))
                .or_not(),
        )
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(role, semi), e| (role, semi, e.span()))
        .validate(|(role, semi, span), _e, emitter| {
            let role = match role {
                Some(Some(Some(Some(name)))) => Some(name),
                Some(Some(Some(None))) | Some(Some(None)) | Some(None) => {
                    emitter.emit(Rich::custom(span, "expected a role name after `:`"));
                    None
                }
                None => {
                    emitter.emit(Rich::custom(span, "expected `:` after `performer`"));
                    None
                }
            };
            if semi.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `;` after the performer clause",
                ));
            }
            let name_span = role.as_ref().map(|name| name.span).unwrap_or(span);
            (
                role.map(|name| name.text).unwrap_or_default(),
                span,
                name_span,
            )
        })
}

/// An `execute <alias>.<Service>.<operation>;` clause of a service body.
/// Committed once `execute` matched: a reference that is not exactly three
/// segments or a missing `;` is a reported error, and the clause recovers
/// with the segments it has.
pub(crate) fn execute_clause<'src>(
) -> impl Parser<'src, Tokens<'src>, CmmnExecute, MoxExtra<'src>> + Clone {
    cmmn_keyword(cmmn::EXECUTE)
        .ignore_then(qname())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(name, semi), e| (name, semi, e.span()))
        .validate(|(name, semi, span), _e, emitter| {
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the execute clause"));
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
            // The mention the graph resolves is `Service.operation` — the
            // alias names the imported file, not part of the target's name.
            let reference_span = match name.segments.as_slice() {
                [_, service, operation, ..] => (service.span.start..operation.span.end).into(),
                _ => name.span,
            };
            CmmnExecute {
                alias,
                service,
                operation,
                reference_span,
                span,
            }
        })
}

/// An `invokes <qualified-name> [version <pin>];` clause of a decision
/// body. Committed once `invokes` matched: a missing `;` is a reported
/// error. The qualified name lowers **as written** (joined segments); the
/// optional `version` pin (rexlang#154) lowers with it — the same shape
/// the `.ddd` decision references take.
pub(crate) fn invoke_clause<'src>(
) -> impl Parser<'src, Tokens<'src>, CmmnInvoke, MoxExtra<'src>> + Clone {
    let pin = kw(Token::Version)
        .ignore_then(
            choice((
                cmmn_keyword(cmmn::COMPATIBLE).to(DddVersionPin::Compatible),
                string_lit().map(DddVersionPin::Exact),
            ))
            .or_not(),
        )
        .map_with(|pin, e| (pin, e.span()))
        .validate(|(pin, span), _e, emitter| {
            pin.unwrap_or_else(|| {
                emitter.emit(Rich::custom(
                    span,
                    "`version` requires `compatible` or an \"X.Y.Z\" version string",
                ));
                DddVersionPin::Compatible
            })
        });
    cmmn_keyword(cmmn::INVOKES)
        .ignore_then(qname())
        .then(pin.or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((name, pin), semi), e| (name, pin, semi, e.span()))
        .validate(|(name, pin, semi, span), _e, emitter| {
            if semi.is_none() {
                emitter.emit(Rich::custom(span, "expected `;` after the invokes clause"));
            }
            let segments: Vec<&str> = name
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect();
            CmmnInvoke {
                service: segments.join("."),
                name_span: name.span,
                pin,
                span,
            }
        })
}

/// The words that end a decision body's bare name lists: the words that
/// can begin the next plan-item body member (the `.dmn` service-list
/// rule).
const DECISION_NAME_LIST_STOPS: &[&str] = &[
    cmmn::PERFORMER,
    cmmn::EXECUTE,
    cmmn::INVOKES,
    cmmn::INPUTS,
    cmmn::OUTPUTS,
    cmmn::ENTRY,
    cmmn::EXIT,
    cmmn::REQUIRED,
    cmmn::REPEATS,
    cmmn::AUTO,
];

/// An `inputs { <name>* }` / `outputs { <name>* }` bare name list of a
/// decision body: the `.dmn` service name-list shape, reused verbatim.
pub(crate) fn decision_name_list<'src>(
    keyword: &'static str,
) -> impl Parser<'src, Tokens<'src>, Vec<Name>, MoxExtra<'src>> + Clone {
    keyword_block(keyword, dmn_bare_name(DECISION_NAME_LIST_STOPS).map(Some))
}

/// A control flag: `required;`, `repeats;`, or `auto;`.
pub(crate) fn flag_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, (Span, &'static str), MoxExtra<'src>> + Clone {
    choice((
        cmmn_keyword(cmmn::REQUIRED).to(cmmn::REQUIRED),
        cmmn_keyword(cmmn::REPEATS).to(cmmn::REPEATS),
        cmmn_keyword(cmmn::AUTO).to(cmmn::AUTO),
    ))
    .then(kw(Token::Other(';')).or_not())
    .map_with(|(word, semi), e| (word, semi, e.span()))
    .validate(|(word, semi, span), _e, emitter| {
        if semi.is_none() {
            emitter.emit(Rich::custom(span, "expected `;` after the flag"));
        }
        (span, word)
    })
}

/// One member of a plan item body: a `performer` clause, an `execute`
/// clause, an `invokes` clause, an `inputs`/`outputs` name list, a
/// criterion, a control flag, or — inside a stage — a nested plan item.
#[derive(Clone)]
pub(crate) enum CmmnBodyMember {
    Performer((String, Span, Span)),
    Execute(CmmnExecute),
    Invoke(CmmnInvoke),
    Inputs(Vec<Name>),
    Outputs(Vec<Name>),
    Criterion {
        is_exit: bool,
        criterion: CmmnCriterion,
    },
    Flag(Span, &'static str),
    Item(Box<CmmnItemDecl>),
}

/// The folded members of a plan item body.
#[derive(Default)]
pub(crate) struct CmmnBody {
    performer: Option<(String, Span, Span)>,
    execute: Option<CmmnExecute>,
    invokes: Option<CmmnInvoke>,
    inputs: Vec<Name>,
    outputs: Vec<Name>,
    entry: Vec<CmmnCriterion>,
    exit: Vec<CmmnCriterion>,
    required: bool,
    auto: bool,
    repeats: bool,
    problems: Vec<(Span, String)>,
}

impl CmmnBody {
    fn fold(&mut self, member: CmmnBodyMember) {
        match member {
            CmmnBodyMember::Performer(performer) => {
                if self.performer.is_none() {
                    self.performer = Some(performer);
                } else {
                    self.problems
                        .push((performer.1, "duplicate `performer` clause".to_string()));
                }
            }
            CmmnBodyMember::Execute(execute) => {
                if self.execute.is_none() {
                    self.execute = Some(execute);
                } else {
                    self.problems
                        .push((execute.span, "duplicate `execute` clause".to_string()));
                }
            }
            CmmnBodyMember::Invoke(invoke) => {
                if self.invokes.is_none() {
                    self.invokes = Some(invoke);
                } else {
                    self.problems
                        .push((invoke.span, "duplicate `invokes` clause".to_string()));
                }
            }
            CmmnBodyMember::Inputs(names) => self.inputs.extend(names),
            CmmnBodyMember::Outputs(names) => self.outputs.extend(names),
            CmmnBodyMember::Criterion { is_exit, criterion } => {
                if is_exit {
                    self.exit.push(criterion);
                } else {
                    self.entry.push(criterion);
                }
            }
            CmmnBodyMember::Flag(span, word) => match word {
                w if w == cmmn::REQUIRED => self.required = true,
                w if w == cmmn::AUTO => self.auto = true,
                w if w == cmmn::REPEATS => self.repeats = true,
                _ => self.problems.push((span, format!("unknown flag `{word}`"))),
            },
            CmmnBodyMember::Item(_) => {}
        }
    }
}

/// The `{ ... }` body of a plan item: its members, folded. `nested`
/// selects whether nested plan items are grammatical inside (stages only);
/// `expected` is the junk-member error message.
pub(crate) fn plan_item_body<'src>(
    nested: bool,
    decl: impl Parser<'src, Tokens<'src>, CmmnItemDecl, MoxExtra<'src>> + Clone + 'src,
) -> impl Parser<'src, Tokens<'src>, (CmmnBody, Vec<CmmnItemDecl>), MoxExtra<'src>> + Clone {
    let plain = choice((
        performer_clause().map(CmmnBodyMember::Performer),
        execute_clause().map(CmmnBodyMember::Execute),
        invoke_clause().map(CmmnBodyMember::Invoke),
        decision_name_list(cmmn::INPUTS).map(CmmnBodyMember::Inputs),
        decision_name_list(cmmn::OUTPUTS).map(CmmnBodyMember::Outputs),
        criterion_decl()
            .map(|(is_exit, criterion)| CmmnBodyMember::Criterion { is_exit, criterion }),
        flag_decl().map(|(span, word)| CmmnBodyMember::Flag(span, word)),
    ));
    let member: Boxed<'src, 'src, Tokens<'src>, CmmnBodyMember, MoxExtra<'src>> = if nested {
        choice((plain, decl.map(|item| CmmnBodyMember::Item(Box::new(item))))).boxed()
    } else {
        plain.boxed()
    };
    kw(Token::LBrace)
        .ignore_then(
            member
                .map(Some)
                .or(junk_cmmn_member("expected a plan item member").to(None))
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then_ignore(kw(Token::RBrace))
        .map(|members| {
            let mut body = CmmnBody::default();
            let mut items = Vec::new();
            for member in members.into_iter().flatten() {
                match member {
                    CmmnBodyMember::Item(item) => items.push(*item),
                    other => body.fold(other),
                }
            }
            (body, items)
        })
}

/// A plan item declaration: a `task`, `service`, `milestone`, or `stage`
/// (optionally prefixed `discretionary`), with its optional body. Stages
/// may nest plan items; the other kinds may not.
pub(crate) fn plan_item_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, CmmnItemDecl, MoxExtra<'src>> + Clone {
    recursive(|decl| {
        let simple = |word: &'static str| {
            cmmn_keyword(word)
                .ignore_then(name().or_not())
                .then(plan_item_body(false, decl.clone()).or_not())
                .map_with(move |(name, body), e| (word, name, body, e.span()))
                .validate(|(word, name, body, span), _e, emitter| {
                    let name = name.or_else(|| {
                        emitter.emit(Rich::custom(span, "expected a plan item name"));
                        None
                    });
                    let (body, _) = body.unwrap_or_default();
                    for (problem_span, message) in &body.problems {
                        emitter.emit(Rich::custom(*problem_span, message.clone()));
                    }
                    let kind = if word == cmmn::TASK {
                        CmmnItemKind::Task
                    } else if word == cmmn::SERVICE {
                        CmmnItemKind::Service
                    } else if word == cmmn::DECISION {
                        CmmnItemKind::Decision
                    } else {
                        CmmnItemKind::Milestone
                    };
                    CmmnItemDecl {
                        name: name.as_ref().map(|n| n.text.clone()).unwrap_or_default(),
                        name_span: name.as_ref().map(|n| n.span).unwrap_or(span),
                        description: None,
                        discretionary: false,
                        kind,
                        performer: body
                            .performer
                            .as_ref()
                            .map(|(role, clause, _)| (role.clone(), *clause)),
                        performer_span: body.performer.as_ref().map(|(_, _, name)| *name),
                        execute: body.execute,
                        invokes: body.invokes,
                        inputs: body.inputs,
                        outputs: body.outputs,
                        entry: body.entry,
                        exit: body.exit,
                        required: body.required,
                        auto: body.auto,
                        repeats: body.repeats,
                        span,
                    }
                })
        };
        let discretionary = cmmn_keyword(cmmn::DISCRETIONARY)
            .ignore_then(choice((
                simple(cmmn::TASK),
                simple(cmmn::SERVICE),
                simple(cmmn::DECISION),
                simple(cmmn::MILESTONE),
            )))
            .validate(|mut item, _e, _emitter| {
                item.discretionary = true;
                item
            });
        let stage = cmmn_keyword(cmmn::STAGE)
            .ignore_then(name().or_not())
            .then(plan_item_body(true, decl.clone()).or_not())
            .map_with(|(name, body), e| (name, body, e.span()))
            .validate(|(name, body, span), _e, emitter| {
                let name = name.or_else(|| {
                    emitter.emit(Rich::custom(span, "expected a plan item name"));
                    None
                });
                let (body, items) = body.unwrap_or_default();
                for (problem_span, message) in &body.problems {
                    emitter.emit(Rich::custom(*problem_span, message.clone()));
                }
                CmmnItemDecl {
                    name: name.as_ref().map(|n| n.text.clone()).unwrap_or_default(),
                    name_span: name.as_ref().map(|n| n.span).unwrap_or(span),
                    description: None,
                    discretionary: false,
                    kind: CmmnItemKind::Stage { items },
                    performer: body
                        .performer
                        .as_ref()
                        .map(|(role, clause, _)| (role.clone(), *clause)),
                    performer_span: body.performer.as_ref().map(|(_, _, name)| *name),
                    execute: body.execute,
                    invokes: body.invokes,
                    inputs: body.inputs,
                    outputs: body.outputs,
                    entry: body.entry,
                    exit: body.exit,
                    required: body.required,
                    auto: body.auto,
                    repeats: body.repeats,
                    span,
                }
            });
        choice((
            discretionary,
            simple(cmmn::TASK),
            simple(cmmn::SERVICE),
            simple(cmmn::DECISION),
            simple(cmmn::MILESTONE),
            stage,
        ))
    })
}

/// One member of a case body.
#[derive(Clone)]
pub(crate) enum CmmnCaseMember {
    Roles(Vec<CmmnRole>),
    CaseFile(Vec<CmmnFileItem>),
    Item(Box<CmmnItemDecl>),
}

/// A `case <name> { ... }` declaration. The three member kinds may be
/// interleaved in any order; the AST folds them into per-kind lists.
pub(crate) fn case_decl<'src>() -> impl Parser<'src, Tokens<'src>, CmmnCase, MoxExtra<'src>> + Clone
{
    let member = choice((
        roles_block().map(CmmnCaseMember::Roles),
        case_file_block().map(CmmnCaseMember::CaseFile),
        plan_item_decl().map(|item| CmmnCaseMember::Item(Box::new(item))),
    ));
    cmmn_keyword(cmmn::CASE)
        .ignore_then(name().or_not())
        .then_ignore(kw(Token::LBrace))
        .then(
            member
                .map(Some)
                .or(junk_cmmn_member("expected a case member").to(None))
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, members), e| (name, members, e.span()))
        .validate(|(name, members, span), _e, emitter| {
            let name = name.or_else(|| {
                emitter.emit(Rich::custom(span, "expected a case name after `case`"));
                None
            });
            let mut case = CmmnCase {
                name: name.as_ref().map(|n| n.text.clone()).unwrap_or_default(),
                name_span: name.as_ref().map(|n| n.span).unwrap_or(span),
                description: None,
                roles: Vec::new(),
                case_file: Vec::new(),
                plan_items: Vec::new(),
                span,
            };
            for member in members.into_iter().flatten() {
                match member {
                    CmmnCaseMember::Roles(roles) => case.roles.extend(roles),
                    CmmnCaseMember::CaseFile(items) => case.case_file.extend(items),
                    CmmnCaseMember::Item(item) => case.plan_items.push(*item),
                }
            }
            case
        })
}

/// Consumes a run of tokens up to the next `import` keyword or the
/// contextual `case` word (or end of input). File-level recovery for
/// `.cmmn` files: always consumes at least one token (guaranteeing
/// progress) and emits an error for the skipped region.
pub(crate) fn junk_cmmn_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone
{
    let rest = select! {
        t if !matches!(t, Token::Import | Token::Ident(cmmn::CASE)) => ()
    };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(
                e.span(),
                "expected an import or case declaration",
            ));
        })
}

/// One top-level item of a `.cmmn` file.
#[derive(Clone)]
pub(crate) enum CmmnFileItemKind {
    Import(Import),
    Case(Box<CmmnCase>),
    Junk,
}

pub(crate) fn fold_cmmn_file(items: Vec<CmmnFileItemKind>) -> (CmmnFile, Option<Span>) {
    let mut imports = Vec::new();
    let mut cases = Vec::new();
    // Span of the first `import` that follows a case (a syntax error
    // reported by the caller's `validate`).
    let mut late_import = None;
    for item in items {
        match item {
            CmmnFileItemKind::Import(decl) => {
                if cases.is_empty() {
                    imports.push(decl);
                } else if late_import.is_none() {
                    late_import = Some(decl.span);
                }
            }
            CmmnFileItemKind::Case(decl) => cases.push(*decl),
            CmmnFileItemKind::Junk => {}
        }
    }
    (CmmnFile { imports, cases }, late_import)
}

pub(crate) fn cmmn_file<'src>() -> impl Parser<'src, Tokens<'src>, CmmnFile, MoxExtra<'src>> + Clone
{
    let item = choice((
        cmmn_import_decl().map(CmmnFileItemKind::Import),
        case_decl().map(|decl| CmmnFileItemKind::Case(Box::new(decl))),
    ))
    .or(junk_cmmn_file().to(CmmnFileItemKind::Junk));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map(fold_cmmn_file)
        .validate(|(file, late_import), _, emitter| {
            if let Some(span) = late_import {
                emitter.emit(Rich::custom(span, "`import` after a case declaration"));
            }
            file
        })
}

/// Attaches doc comments to a `.cmmn` file's cases, case-file items, and
/// plan items (recursively through stages). The contiguous `///` run is
/// joined with newlines into a single description — the same semantic as
/// the `.mox` doc collection ([`doc_run_above`] is shared).
pub(crate) fn attach_cmmn_docs(
    file: &mut CmmnFile,
    comments: &[crate::lexer::Comment<'_>],
    source: &str,
) {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    let docs = collect_docs(comments, source);
    let description = |span: Span| doc_run_above(&docs, line_of(span.start));
    for case in &mut file.cases {
        case.description = description(case.span);
        for item in &mut case.case_file {
            item.description = description(item.span);
        }
        attach_item_docs(&mut case.plan_items, &description);
    }
}

pub(crate) fn attach_item_docs(
    items: &mut Vec<CmmnItemDecl>,
    description: &impl Fn(Span) -> Option<String>,
) {
    for item in items {
        item.description = description(item.span);
        if let CmmnItemKind::Stage { items } = &mut item.kind {
            attach_item_docs(items, description);
        }
    }
}

/// The outcome of parsing a `.cmmn` source: as much of the file as could be
/// recovered, plus all encountered errors. Parsing never panics and always
/// produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnParseResult {
    /// The recovered case file, or `None` if no output could be produced
    /// at all.
    pub ast: Option<CmmnFile>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

/// Lex and parse a `.cmmn` source text: `import` declarations followed by
/// `case` declarations (imports must come first).
///
/// This function never panics and always recovers as much of the AST as
/// possible; check [`CmmnParseResult::errors`] for syntax problems. Doc
/// comment runs (`///`) directly above a `case`, a plan item, or a
/// case-file item become its description.
pub fn parse_cmmn(source: &str) -> CmmnParseResult {
    let (tokens, comments) = match lex_with_comments(source) {
        Ok(result) => result,
        Err(error) => {
            return CmmnParseResult {
                ast: None,
                errors: vec![ParseError {
                    message: error.to_string(),
                    span: error.span,
                }],
            }
        }
    };
    let (mut ast, errors) = cmmn_file().parse(Tokens::new(&tokens)).into_output_errors();
    if let Some(file) = ast.as_mut() {
        attach_cmmn_docs(file, &comments, source);
    }
    CmmnParseResult {
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

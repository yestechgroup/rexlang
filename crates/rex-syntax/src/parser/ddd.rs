//! The `.ddd` design grammar: services, designs, repositories,
//! machines, searches, modules, and the application file parser.

use std::iter::once;

use crate::ddd;
use crate::lexer::{lex_with_comments, Token};

use super::docs::attach_ddd_docs;
use super::*;
/// A contextual `.ddd` keyword: an ordinary identifier that is special only
/// in the grammar position it is parsed here in (the `schema`/`to`
/// precedent). Escaped forms (`^service`) are different token variants and
/// never match.
pub(crate) fn ddd_keyword<'src>(
    keyword: &'static str,
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == keyword => e.span() }
}

/// An `import "<path>"` declaration of a `.ddd` file. Unlike the `.actor`
/// production this tolerates a trailing `;`, which the canonical `.ddd`
/// formatting emits on every import line.
pub(crate) fn ddd_import_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, Import, MoxExtra<'src>> + Clone {
    kw(Token::Import)
        .ignore_then(string_lit())
        .then_ignore(kw(Token::Other(';')).or_not())
        .map_with(|path, e| Import {
            path,
            alias: None,
            span: e.span(),
        })
}

/// The `capability <name> (, <name>)*` clause of a service operation. The
/// clause is committed once the `capability` keyword matched: a missing name
/// list is a syntax error (with recovery), not a failed alternative.
pub(crate) fn ddd_capabilities<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<Name>, MoxExtra<'src>> + Clone {
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

/// The `emits <Event> (, <Event>)*` clause of a service operation: the
/// events the operation produces. The `emits` keyword is contextual (the
/// [`ddd::EMITS`] identifier, not a lexer token), so an operation may be
/// named `emits` and an emitted event may be named `emits` — the keyword
/// position is unambiguous. The clause is committed once the keyword
/// matched: a missing name list is a syntax error (with recovery), not a
/// failed alternative. Names are loosely coupled: the `.ddd` compile has
/// no `.evt` bucket, so existence/coverage resolve in the union integrity
/// pass.
pub(crate) fn ddd_emits<'src>() -> impl Parser<'src, Tokens<'src>, Vec<Name>, MoxExtra<'src>> + Clone
{
    ddd_keyword(ddd::EMITS)
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
                    "`emits` requires at least one event name",
                ));
            }
            first.into_iter().chain(rest).collect::<Vec<_>>()
        })
}

/// The `invokes <DecisionService> (, <DecisionService>)*` clause of a
/// service operation: the decision services the operation invokes. The
/// `invokes` keyword is contextual (the [`ddd::INVOKES`] identifier, not a
/// lexer token), so an operation may be named `invokes` — the keyword
/// position is unambiguous. The clause is committed once the keyword
/// matched: a missing name list is a syntax error (with recovery), not a
/// failed alternative. Names are loosely coupled: the `.ddd` compile
/// resolves them against the provided `.dmn` artifacts only (the decisions
/// entry point), never at the wire layer.
pub(crate) fn ddd_invokes<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<QualifiedName>, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::INVOKES)
        .ignore_then(qname().or_not())
        .then(
            kw(Token::Comma)
                .ignore_then(qname())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map_with(|(first, rest), e| (first, rest, e.span()))
        .validate(|(first, rest, span), _e, emitter| {
            if first.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "`invokes` requires at least one decision service name",
                ));
            }
            first.into_iter().chain(rest).collect::<Vec<_>>()
        })
}

/// A `type_ref multiplicity? name "(" params? ")"` operation signature,
/// shared verbatim by service and repository operations.
pub(crate) fn ddd_signature<'src>() -> impl Parser<
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
pub(crate) fn ddd_delegation<'src>(
) -> impl Parser<'src, Tokens<'src>, DddDelegation, MoxExtra<'src>> + Clone {
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

/// One operation of a `service` body: an optional `protected` modifier, then
/// a declared signature
/// (`type_ref multiplicity? name "(" params? ")" capability*? invokes? emits? ";"`)
/// or a delegation (`name "=>" target.operation capability*? invokes? emits? ";"`).
/// The two alternatives are distinguished by the `=>` after the leading name;
/// chumsky backtracks, so a delegated op may be named `save` and a declared
/// op's type may be any contextual keyword. Both forms carry the clause
/// sequence `capability? invokes? emits? when?` (the canonical clause order
/// the formatter emits); repository operations take none of the
/// `capability`/`invokes`/`emits` clauses — the wire artifact's repository
/// operations carry neither, so accepting one would silently drop data.
pub(crate) fn ddd_service_op<'src>(
) -> impl Parser<'src, Tokens<'src>, DddServiceOp, MoxExtra<'src>> + Clone {
    let declared = ddd_signature()
        .then(ddd_capabilities().or_not())
        .then(ddd_invokes().or_not())
        .then(ddd_emits().or_not())
        .then(ddd_guard_clause().or_not())
        .then_ignore(kw(Token::Other(';')))
        .map_with(
            |(
                ((((type_ref, multiplicity, name, params), capabilities), invokes), emits),
                condition,
            ),
             e| {
                DddServiceOp {
                    name,
                    return_type: Some(type_ref),
                    multiplicity,
                    params,
                    delegation: None,
                    is_protected: false,
                    capabilities: capabilities.unwrap_or_default(),
                    invokes: invokes.unwrap_or_default(),
                    emits: emits.unwrap_or_default(),
                    condition,
                    span: e.span(),
                }
            },
        );
    let delegated = name()
        .then_ignore(kw(Token::FatArrow))
        .then(ddd_delegation())
        .then(ddd_capabilities().or_not())
        .then(ddd_invokes().or_not())
        .then(ddd_emits().or_not())
        .then(ddd_guard_clause().or_not())
        .then_ignore(kw(Token::Other(';')))
        .map_with(
            |(((((name, delegation), capabilities), invokes), emits), condition), e| DddServiceOp {
                name,
                return_type: None,
                multiplicity: None,
                params: Vec::new(),
                delegation: Some(delegation),
                is_protected: false,
                capabilities: capabilities.unwrap_or_default(),
                invokes: invokes.unwrap_or_default(),
                emits: emits.unwrap_or_default(),
                condition,
                span: e.span(),
            },
        );
    ddd_keyword(ddd::PROTECTED)
        .or_not()
        .then(delegated.or(declared))
        .map_with(|(protected, mut op), e| {
            op.is_protected = protected.is_some();
            // The span covers the whole operation, modifier included.
            op.span = e.span();
            op
        })
}

/// An `inject <name>;` or `inject decision <qualified-name>;` line of a
/// `service` body. After the `inject` keyword the parser attempts the
/// contextual `decision` discriminator ([`ddd::DECISION`]): present, the
/// rest of the line is a qualified decision-service name (the `.dmn`
/// dimension); absent, the plain dependency-name form. Chumsky backtracks,
/// so `inject decision;` — a plain dependency literally named `decision` —
/// still parses as the plain form (the decision form requires a name
/// behind the discriminator).
pub(crate) fn ddd_inject_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddServiceMember, MoxExtra<'src>> + Clone {
    let decision = ddd_keyword(ddd::DECISION)
        .ignore_then(qname())
        .map(DddServiceMember::InjectDecision);
    let plain = name().map(DddServiceMember::Inject);
    ddd_keyword(ddd::INJECT)
        .ignore_then(decision.or(plain))
        .then_ignore(kw(Token::Other(';')))
}

/// One member of a `service` body.
#[derive(Clone)]
pub(crate) enum DddServiceMember {
    Operation(Box<DddServiceOp>),
    Inject(Name),
    InjectDecision(QualifiedName),
}

/// Consumes a run of tokens that cannot start or continue a service member,
/// stopping before the body's closing `}` and before identifiers (which
/// could begin the next operation or `inject` line). Always consumes at
/// least one token.
pub(crate) fn junk_ddd_service_member<'src>(
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
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
pub(crate) fn ddd_service_member<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<DddServiceMember>, MoxExtra<'src>> + Clone {
    choice((
        ddd_service_op().map(|operation| DddServiceMember::Operation(Box::new(operation))),
        ddd_inject_decl(),
    ))
    .map(Some)
    .or(junk_ddd_service_member().to(None))
}

/// A `service <name> { ... }` declaration of a `module`. Doc comments (`///`
/// runs) directly above it become the service description after parsing.
pub(crate) fn ddd_service_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddService, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::SERVICE)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(ddd_service_member().repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, members), e| {
            let mut operations = Vec::new();
            let mut dependencies = Vec::new();
            let mut decision_dependencies = Vec::new();
            for member in members {
                match member {
                    Some(DddServiceMember::Operation(operation)) => {
                        operations.push(*operation);
                    }
                    Some(DddServiceMember::Inject(name)) => dependencies.push(name),
                    Some(DddServiceMember::InjectDecision(name)) => {
                        decision_dependencies.push(name);
                    }
                    None => {}
                }
            }
            DddService {
                name,
                doc: None,
                operations,
                dependencies,
                decision_dependencies,
                span: e.span(),
            }
        })
}

/// The `("abstract")? stereotype` head of a design declaration.
pub(crate) fn ddd_design_head<'src>(
) -> impl Parser<'src, Tokens<'src>, (Option<Span>, DddStereotype), MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::ABSTRACT).or_not().then(ddd_stereotype())
}

/// The stereotype keyword of a design head: the `choice` alternatives come
/// from [`ddd::STEREOTYPES`] in the table's order, so the parse order and
/// the table order (the formatter's dispatch set) agree by construction.
pub(crate) fn ddd_stereotype<'src>(
) -> impl Parser<'src, Tokens<'src>, DddStereotype, MoxExtra<'src>> + Clone {
    choice(ddd::STEREOTYPES.map(|(keyword, stereotype)| ddd_keyword(keyword).to(stereotype)))
}

/// The design flags of a design declaration: any subset of the
/// [`ddd::DESIGN_FLAGS`] keywords in any order, each optionally negated
/// (`!auditable` — the opt-out from a flag that defaults on; the canonical
/// order is the table's, a formatter concern). Repeats are idempotent.
pub(crate) fn ddd_flags<'src>() -> impl Parser<'src, Tokens<'src>, DddFlags, MoxExtra<'src>> + Clone
{
    kw(Token::Other('!'))
        .or_not()
        .then(select! {
            // chumsky's `select!` takes a boolean guard expression, so the
            // lookup runs twice; the `unwrap` is the guard's `Some`.
            Token::Ident(text) = e if ddd::flag_kind(text).is_some() => {
                (ddd::flag_kind(text).unwrap(), e.span())
            }
        })
        .repeated()
        .collect::<Vec<_>>()
        .map(|pairs| {
            let mut flags = DddFlags::default();
            for (bang, (kind, span)) in pairs {
                // A negated occurrence spans from the `!` through the
                // keyword, so diagnostics point at the whole `!flag` form.
                let span = match bang {
                    Some(bang) => (bang.start..span.end).into(),
                    None => span,
                };
                flags.push(
                    kind,
                    DddFlagUse {
                        negated: bang.is_some(),
                        span,
                    },
                );
            }
            flags
        })
}

/// A design declaration: `("abstract")? stereotype name flag*
/// ("repository" ...)? ("machine" ...)?`.
pub(crate) fn ddd_design_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddDesign, MoxExtra<'src>> + Clone {
    ddd_design_head()
        .then(name())
        .then(ddd_flags())
        .then(ddd_repository_decl().or_not())
        .then(ddd_machine_decl().or_not())
        .map_with(
            |(((((abstract_span, stereotype), class), flags), repository), machine), e| DddDesign {
                is_abstract: abstract_span.is_some(),
                stereotype,
                class,
                flags,
                repository,
                machine,
                span: e.span(),
            },
        )
}

/// One operation of a `repository` body: an optional `protected` modifier,
/// then a built-in (`findById`, `findAll`, `findByExample`, `findByKeys`,
/// `save`, `delete` — recognized by their contextual keywords, no signature
/// of their own) or a declared operation with an explicit signature. Both
/// forms end in `;`. A declared repository operation takes no `capability`
/// clause: the wire artifact's repository operations carry no capabilities,
/// so accepting one would silently drop data.
pub(crate) fn ddd_repository_op<'src>(
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
        is_protected: false,
        condition: None,
        span: e.span(),
    });
    let declared = ddd_signature()
        .then(ddd_guard_clause().or_not())
        .then_ignore(kw(Token::Other(';')))
        .map_with(
            |((type_ref, multiplicity, name, params), condition), e| DddRepositoryOp {
                name,
                builtin: None,
                return_type: Some(type_ref),
                multiplicity,
                params,
                is_protected: false,
                condition,
                span: e.span(),
            },
        );
    ddd_keyword(ddd::PROTECTED)
        .or_not()
        .then(
            builtin
                .map(Some)
                .or(declared.map(Some))
                .or(junk_ddd_repository_op().to(None)),
        )
        .map(|(protected, op)| {
            op.map(|mut op| {
                op.is_protected = protected.is_some();
                op
            })
        })
}

/// Consumes a run of tokens that cannot start or continue a repository
/// operation, stopping before the body's closing `}` and before identifiers.
pub(crate) fn junk_ddd_repository_op<'src>(
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
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
pub(crate) fn ddd_repository_decl<'src>(
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

/// The `when ( ... )` guard clause shared by machine transitions and service
/// operations: the real `when` keyword followed by a [`raw_parens`] span the
/// driver slices and types (the `.actor` condition convention).
pub(crate) fn ddd_guard_clause<'src>(
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    kw(Token::When).ignore_then(raw_parens())
}

/// One `from -> to (via <Trigger>)? (when (...))?;` transition of a machine
/// body. The arrow is the real [`Token::Arrow`]; a transition takes exactly
/// one target — a `,` after the target cannot start any machine member, so a
/// multi-target arrow fails into the member junk path (the deliberate
/// exclusion noted in the grammar).
pub(crate) fn ddd_machine_transition<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<DddMachineTransition>, MoxExtra<'src>> + Clone {
    let transition = name()
        .then_ignore(kw(Token::Arrow))
        .then(name())
        .then(ddd_keyword(ddd::VIA).ignore_then(name()).or_not())
        .then(ddd_guard_clause().or_not())
        .then_ignore(kw(Token::Other(';')))
        .map_with(|(((from, to), trigger), guard), e| DddMachineTransition {
            from,
            to,
            trigger,
            guard,
            span: e.span(),
        });
    transition.map(Some).or(junk_ddd_machine_member().to(None))
}

/// Consumes a run of tokens that cannot start or continue a machine member,
/// stopping before the body's closing `}` and before identifiers (which
/// could begin the next member). Always consumes at least one token.
pub(crate) fn junk_ddd_machine_member<'src>(
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(t, Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)) => ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), "expected a machine member"));
        })
}

/// A `machine <field>: <Enum> { ... }` block of a design declaration. Body
/// members are `initial`/`terminal` lines and transitions; the per-kind
/// lists fold exactly like the search members.
pub(crate) fn ddd_machine_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddMachine, MoxExtra<'src>> + Clone {
    let initial = ddd_keyword(ddd::INITIAL)
        .ignore_then(name())
        .then_ignore(kw(Token::Other(';')));
    let terminal = ddd_keyword(ddd::TERMINAL)
        .ignore_then(name())
        .then_ignore(kw(Token::Other(';')));
    let member = choice((
        initial.map(DddMachineMember::Initial),
        terminal.map(DddMachineMember::Terminal),
        ddd_machine_transition().map(DddMachineMember::Transition),
    ));
    ddd_keyword(ddd::MACHINE)
        .ignore_then(name())
        .then_ignore(kw(Token::Other(':')))
        .then(tref())
        .then_ignore(kw(Token::LBrace))
        .then(member.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|((field, type_), members), e| {
            let mut machine = DddMachine {
                field,
                type_,
                initial: None,
                terminals: Vec::new(),
                transitions: Vec::new(),
                span: e.span(),
            };
            for member in members {
                match member {
                    DddMachineMember::Initial(name) => machine.initial = Some(name),
                    DddMachineMember::Terminal(name) => machine.terminals.push(name),
                    DddMachineMember::Transition(Some(transition)) => {
                        machine.transitions.push(transition)
                    }
                    DddMachineMember::Transition(None) => {}
                }
            }
            machine
        })
}

/// One member of a `machine` body.
#[derive(Clone)]
pub(crate) enum DddMachineMember {
    Initial(Name),
    Terminal(Name),
    Transition(Option<DddMachineTransition>),
}

/// Scans a raw expression: tokens up to the terminating `;`, tracking the
/// nesting depth of `()`, `[]` and `{}` so an `if ... { ... } else { ... }`
/// expression's braces stay inside the capture while the terminator must sit
/// at depth 0. String literals are single tokens, so a `;` inside one never
/// terminates. The returned span slices the expression source strictly
/// between the `=` and the `;` (the `;` excluded), the `.mox` op-body
/// convention; at least one token is required, so a bare `name = ;` fails.
pub(crate) fn ddd_raw_expr<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone
{
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
pub(crate) fn ddd_document_entry<'src>(
) -> impl Parser<'src, Tokens<'src>, DddDocumentEntry, MoxExtra<'src>> + Clone {
    name()
        .then_ignore(kw(Token::Eq))
        .then(ddd_raw_expr())
        .map(|(name, expr)| DddDocumentEntry { name, expr })
}

/// One `text { ... }` field: `<property> (boost <int>)? (analyzer "<...>")?`.
pub(crate) fn ddd_search_field<'src>(
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
pub(crate) fn ddd_ranking<'src>(
) -> impl Parser<'src, Tokens<'src>, DddRanking, MoxExtra<'src>> + Clone {
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
pub(crate) fn ddd_pagination<'src>(
) -> impl Parser<'src, Tokens<'src>, DddPagination, MoxExtra<'src>> + Clone {
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
pub(crate) enum DddSearchMember {
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
pub(crate) fn junk_ddd_search_member<'src>(
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
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
pub(crate) fn member_keyword(keyword: Option<&'static str>) -> &'static str {
    keyword.expect("only the `capability` search member lacks a keyword")
}

/// One member of a `search` body with junk recovery. The alternatives come
/// from [`ddd::SEARCH_MEMBERS`] in the table's order — the same order the
/// formatter canonicalizes a search body to — so the parse order and the
/// canonical order cannot drift apart. Repeated `text`, `filters`, `sort`,
/// and `document` clauses merge into their lists; `entity`, `ranking`,
/// `analyzer`, and `pagination` are single — a repeat is idempotent, the
/// first declaration wins (the design-flags precedent).
pub(crate) fn ddd_search_member<'src>(
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
pub(crate) fn ddd_search_names<'src>(
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
pub(crate) fn ddd_search_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddSearch, MoxExtra<'src>> + Clone {
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
pub(crate) enum DddModuleMember {
    Service(DddService),
    Design(DddDesign),
    Search(DddSearch),
}

/// Consumes a run of tokens that cannot start or continue a module member,
/// stopping before the body's closing `}` and before identifiers (which
/// could begin the next `service` or design).
pub(crate) fn junk_ddd_module_member<'src>(
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
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
pub(crate) fn ddd_module_member<'src>(
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
pub(crate) fn ddd_module_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddModule, MoxExtra<'src>> + Clone {
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
pub(crate) fn ddd_base_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DddBase, MoxExtra<'src>> + Clone {
    ddd_keyword(ddd::BASE)
        .ignore_then(qname())
        .map_with(|package, e| DddBase {
            package,
            span: e.span(),
        })
}

/// One member of an `application` body.
#[derive(Clone)]
pub(crate) enum DddApplicationItem {
    Base(DddBase),
    Module(DddModule),
}

/// Consumes a run of tokens that cannot start or continue an application
/// member, stopping before the body's closing `}` and before identifiers.
pub(crate) fn junk_ddd_application_item<'src>(
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
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
pub(crate) fn ddd_application_item<'src>(
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
pub(crate) fn ddd_application_decl<'src>(
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
pub(crate) enum DddFileItem {
    Import(Import),
    Application(DddApplication),
    Junk,
}

/// Consumes a run of tokens up to the next `import` keyword or the
/// contextual `application` keyword (or end of input). File-level recovery:
/// always consumes at least one token (guaranteeing progress) and emits an
/// error for the skipped region.
pub(crate) fn junk_ddd_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
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
pub(crate) fn fold_ddd_file(items: Vec<DddFileItem>) -> (DddFile, Vec<(Span, String)>) {
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

pub(crate) fn ddd_file<'src>() -> impl Parser<'src, Tokens<'src>, DddFile, MoxExtra<'src>> + Clone {
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

// --- .deploy files ------------------------------------------------------------

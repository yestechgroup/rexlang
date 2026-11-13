//! The `.dmn` decision model grammar: metadata, inputs, knowledge sources
//! and models, decisions, requirements, services, views, and the file
//! parser.

use crate::dmn;
use crate::lexer::{lex_with_comments, Token};

use super::docs::attach_dmn_docs;
use super::*;

/// A contextual `.dmn` keyword: an ordinary identifier that is special only
/// in the grammar position it is parsed here in (the `.deploy` discipline).
/// Escaped forms (`^decision`) are different token variants and never match.
pub(crate) fn dmn_keyword<'src>(
    keyword: &'static str,
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == keyword => e.span() }
}

/// An `import "<path>";` declaration of a `.dmn` file: the `.ddd` import
/// production verbatim — the trailing `;` is canonical but optional.
pub(crate) fn dmn_import_decl<'src>(
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

/// One foldable metadata line of a decision model body: the `namespace`
/// qualified-name line or a string-valued metadata entry.
#[derive(Clone)]
pub(crate) enum DmnModelMeta {
    Namespace(QualifiedName),
    Metadata(DmnMetadata),
}

/// The `namespace <qname>;` line of a decision model body. Committed once
/// `namespace` matched: a missing qualified name is a reported error and
/// the line recovers with an empty qualified name. The trailing `;` is
/// tolerated when absent.
pub(crate) fn dmn_namespace_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnModelMeta, MoxExtra<'src>> + Clone {
    dmn_keyword(dmn::NAMESPACE)
        .ignore_then(qname().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(namespace, semi), e| (namespace, semi, e.span()))
        .validate(|(namespace, _semi, span), _e, emitter| {
            if namespace.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected a qualified name after `namespace`",
                ));
            }
            DmnModelMeta::Namespace(namespace.unwrap_or_else(|| QualifiedName {
                segments: Vec::new(),
                span,
            }))
        })
}

/// One string-valued metadata line (`id`/`effective`/`jurisdiction`).
/// Committed once the contextual word matched: a missing value is a
/// reported error and the line recovers with an empty value. The trailing
/// `;` is tolerated when absent. `version` is
/// not handled here — it is a real lexer keyword (see
/// [`dmn_version_decl`]).
fn dmn_string_meta<'src>(
    keyword: &'static str,
    kind: DmnMetadataKind,
) -> impl Parser<'src, Tokens<'src>, DmnModelMeta, MoxExtra<'src>> + Clone {
    dmn_keyword(keyword)
        .ignore_then(string_lit().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(move |(value, semi), e| (value, semi, e.span()))
        .validate(move |(value, _semi, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    format!("expected a string after `{keyword}`"),
                ));
            }
            DmnModelMeta::Metadata(DmnMetadata {
                kind,
                value: value.unwrap_or_default(),
                span,
            })
        })
}

/// The `version "<value>";` line of a decision model body. `version` is a
/// real lexer keyword ([`Token::Version`] — the `.mox` vocabulary
/// precedent), so the keyword token is matched positionally; `^version`
/// stays usable as a name. Committed like the other metadata lines.
pub(crate) fn dmn_version_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnModelMeta, MoxExtra<'src>> + Clone {
    kw(Token::Version)
        .ignore_then(string_lit().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(value, semi), e| (value, semi, e.span()))
        .validate(|(value, _semi, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(span, "expected a string after `version`"));
            }
            DmnModelMeta::Metadata(DmnMetadata {
                kind: DmnMetadataKind::Version,
                value: value.unwrap_or_default(),
                span,
            })
        })
}

/// Any one metadata line of a decision model body.
pub(crate) fn dmn_metadata_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnModelMeta, MoxExtra<'src>> + Clone {
    choice((
        dmn_string_meta(dmn::ID, DmnMetadataKind::Id),
        dmn_version_decl(),
        dmn_string_meta(dmn::EFFECTIVE, DmnMetadataKind::Effective),
        dmn_string_meta(dmn::JURISDICTION, DmnMetadataKind::Jurisdiction),
    ))
}

/// One `name ":" type_ref ";"` entry of an `inputs`, `parameters`, or
/// `outputs` block (three blocks, one production). Committed once the name
/// matched: a missing `:`/type is a reported error and the entry
/// recovers with an empty type reference.
pub(crate) fn dmn_typed_entry<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnInput, MoxExtra<'src>> + Clone {
    name()
        .then(
            kw(Token::Other(':'))
                .ignore_then(tref().map(Some))
                .map(|ty| (ty, true))
                .or_not(),
        )
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((entry_name, typed), semi), e| (entry_name, typed, semi, e.span()))
        .validate(|(entry_name, typed, _semi, span), _e, emitter| {
            let (ty, colon_seen) = typed.unwrap_or((None, false));
            if colon_seen && ty.is_none() {
                emitter.emit(Rich::custom(span, "expected a type after `:`"));
            }
            if !colon_seen {
                emitter.emit(Rich::custom(
                    entry_name.span,
                    "expected `: <type>` after the name",
                ));
            }
            DmnInput {
                name: entry_name,
                type_ref: ty.unwrap_or_else(|| TypeRef {
                    name: QualifiedName {
                        segments: Vec::new(),
                        span,
                    },
                    span,
                }),
                span,
            }
        })
}

/// Consumes a run of tokens that cannot start or continue a body member,
/// stopping before the body's closing `}` and before identifiers and the
/// `import` keyword (which could begin the next member or a misplaced
/// import). Always consumes at least one token.
pub(crate) fn junk_dmn_member<'src>(
    expected: &'static str,
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(
            t,
            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_) | Token::Import
        ) =>
        ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(move |(), e, emitter| {
            emitter.emit(Rich::custom(e.span(), expected));
        })
}

/// A `{ items }` body committed to its brace: `None` when the `{` is
/// missing, in which case nothing beyond the introducing word is consumed
/// (the `.evt` recovery — the enclosing body's `}` is never swallowed).
fn braced_items<'src, T>(
    item: impl Parser<'src, Tokens<'src>, Option<T>, MoxExtra<'src>> + Clone,
) -> impl Parser<'src, Tokens<'src>, Option<Vec<T>>, MoxExtra<'src>> + Clone {
    kw(Token::LBrace)
        .ignore_then(item.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .or_not()
        .map(|items| items.map(|list| list.into_iter().flatten().collect::<Vec<T>>()))
}

/// A `<keyword> { items }` block whose brace is committed: a missing `{` is
/// a reported error and only the introducing word is consumed. Shared with
/// the `.cmmn` decision name lists (the same committed-brace shape).
pub(crate) fn keyword_block<'src, T>(
    keyword: &'static str,
    item: impl Parser<'src, Tokens<'src>, Option<T>, MoxExtra<'src>> + Clone,
) -> impl Parser<'src, Tokens<'src>, Vec<T>, MoxExtra<'src>> + Clone {
    dmn_keyword(keyword)
        .ignore_then(braced_items(item))
        .map_with(move |items, e| (items, e.span()))
        .validate(move |(items, span), _e, emitter| {
            if items.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    format!("expected `{{` after `{keyword}`"),
                ));
            }
            items.into_iter().flatten().collect::<Vec<_>>()
        })
}

/// A `<keyword> "<value>";` body line (the service/view `description`).
/// Committed once the word matched: a missing value is a reported
/// error.
fn dmn_description_item<'src>() -> impl Parser<'src, Tokens<'src>, String, MoxExtra<'src>> + Clone {
    dmn_keyword(dmn::DESCRIPTION)
        .ignore_then(string_lit().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(value, semi), e| (value, semi, e.span()))
        .validate(|(value, _semi, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(span, "expected a string after `description`"));
            }
            value.unwrap_or_default()
        })
}

/// One foldable member of a service body.
#[derive(Clone)]
enum DmnServiceItem {
    Description(String),
    Inputs(Vec<Name>),
    Outputs(Vec<Name>),
    Encapsulates(Vec<Name>),
}

/// One foldable member of a view body.
#[derive(Clone)]
enum DmnViewItem {
    Description(String),
    Show(Vec<Name>),
    Direction(Name),
}

/// One foldable member of a model (BKM) body.
#[derive(Clone)]
enum DmnBkmItem {
    Parameters(Vec<DmnInput>),
    Output(TypeRef),
    Logic(DmnLogic),
}

/// One foldable member of a decision body.
#[derive(Clone)]
enum DmnDecisionItem {
    Output(DmnOutput),
    Outputs(Vec<DmnOutput>),
    Logic(DmnLogic),
    Emits(Vec<DmnEmits>),
}

/// One foldable item of a knowledge block.
#[derive(Clone)]
enum DmnKnowledgeItem {
    Source(DmnKnowledgeSource),
    Model(DmnBkm),
}

/// Which requirement group a line folds into.
#[derive(Clone, Copy)]
pub(crate) enum DmnReqGroup {
    Information,
    Knowledge,
    Authority,
}

/// One foldable member of a decision model body.
#[derive(Clone)]
pub(crate) enum DmnModelMember {
    Meta(DmnModelMeta),
    Inputs(Vec<DmnInput>),
    Knowledge(Vec<DmnKnowledgeSource>, Vec<DmnBkm>),
    Decisions(Vec<DmnDecision>),
    Requirements(
        Vec<DmnRequirement>,
        Vec<DmnRequirement>,
        Vec<DmnRequirement>,
    ),
    Services(Vec<DmnService>),
    Views(Vec<DmnView>),
}

/// One name of a bare `{ name* }` list: any identifier except the words
/// that can begin the next list or member, so a missing `}` never swallows
/// it (the `.evt` events-list rule). Escaped forms are names, always.
pub(crate) fn dmn_bare_name<'src>(
    stops: &'static [&'static str],
) -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    select! {
        Token::Ident(text) = e if !stops.contains(&text) => {
            Name { text: text.to_string(), span: e.span(), escaped: false }
        },
        Token::IdentEscaped(text) = e => {
            Name { text: text.to_string(), span: e.span(), escaped: true }
        },
    }
}

/// The words that end a service body's bare name lists: the words that can
/// begin the next service member.
const SERVICE_NAME_STOPS: &[&str] = &[
    dmn::DESCRIPTION,
    dmn::INPUTS,
    dmn::OUTPUTS,
    dmn::ENCAPSULATES,
];

/// The words that end a view body's bare name lists: the words that can
/// begin the next view member.
const VIEW_NAME_STOPS: &[&str] = &[dmn::DESCRIPTION, dmn::SHOW, dmn::DIRECTION];

/// A `<keyword> { <name>* }` bare name list of a service or view body. The
/// brace is committed like every block's.
fn dmn_name_list<'src>(
    keyword: &'static str,
    stops: &'static [&'static str],
) -> impl Parser<'src, Tokens<'src>, Vec<Name>, MoxExtra<'src>> + Clone {
    keyword_block(keyword, dmn_bare_name(stops).map(Some))
}

/// A knowledge source property: the display `name` or the `reference`, each
/// a string with its canonical `;`. Committed once the word matched.
fn dmn_source_prop<'src>(
    keyword: &'static str,
    display_name: bool,
) -> impl Parser<'src, Tokens<'src>, (bool, String), MoxExtra<'src>> + Clone {
    dmn_keyword(keyword)
        .ignore_then(string_lit().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(move |(value, semi), e| (value, semi, e.span()))
        .validate(move |(value, _semi, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    format!("expected a string after `{keyword}`"),
                ));
            }
            (display_name, value.unwrap_or_default())
        })
}

/// A `source <name> { ... }` declaration of a knowledge block. Committed
/// once `source` matched: a missing name or `{` is a
/// reported error and the declaration recovers with what it has. A repeated
/// property is idempotent — the first wins (the `.ddd` flag rule).
pub(crate) fn dmn_source_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnKnowledgeSource, MoxExtra<'src>> + Clone {
    let prop = choice((
        dmn_source_prop(dmn::NAME, true),
        dmn_source_prop(dmn::REFERENCE, false),
    ))
    .map(Some)
    .or(junk_dmn_member("expected a source property").to(None));
    dmn_keyword(dmn::SOURCE)
        .ignore_then(name().or_not())
        .then(braced_items(prop))
        .map_with(|(decl_name, props), e| (decl_name, props, e.span()))
        .validate(|(decl_name, props, span), _e, emitter| {
            let decl_name = decl_name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if decl_name.text.is_empty() {
                emitter.emit(Rich::custom(span, "expected a source name after `source`"));
            }
            if props.is_none() {
                emitter.emit(Rich::custom(span, "expected `{` after the source name"));
            }
            let mut display_name = None;
            let mut reference = None;
            for (is_display_name, value) in props.into_iter().flatten() {
                if is_display_name {
                    if display_name.is_none() {
                        display_name = Some(value);
                    }
                } else if reference.is_none() {
                    reference = Some(value);
                }
            }
            DmnKnowledgeSource {
                name: decl_name,
                description: None,
                display_name,
                reference,
                span,
            }
        })
}

/// The `output <type>;` line: the model (BKM) body's required output and
/// the decision body's anonymous single-output form (an output with an
/// empty name). Committed once `output` matched: a missing type is a
/// reported error and the line recovers with an empty type reference.
pub(crate) fn dmn_output_line<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnOutput, MoxExtra<'src>> + Clone {
    dmn_keyword(dmn::OUTPUT)
        .ignore_then(tref().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(type_ref, semi), e| (type_ref, semi, e.span()))
        .validate(|(type_ref, _semi, span), _e, emitter| {
            if type_ref.is_none() {
                emitter.emit(Rich::custom(span, "expected a type after `output`"));
            }
            DmnOutput {
                name: Name {
                    text: String::new(),
                    span,
                    escaped: false,
                },
                type_ref: type_ref.unwrap_or_else(|| TypeRef {
                    name: QualifiedName {
                        segments: Vec::new(),
                        span,
                    },
                    span,
                }),
                span,
            }
        })
}

/// One `name ":" "<expression>";` binding of an invoke body. Committed once
/// the name matched: a missing `:`/string is a reported error and the
/// binding recovers with an empty expression.
pub(crate) fn dmn_binding_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnBinding, MoxExtra<'src>> + Clone {
    name()
        .then(
            kw(Token::Other(':'))
                .ignore_then(string_lit().map(Some))
                .map(|value| (value, true))
                .or_not(),
        )
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((binding_name, value), semi), e| (binding_name, value, semi, e.span()))
        .validate(|(binding_name, value, _semi, span), _e, emitter| {
            let (expression, colon_seen) = value.unwrap_or((None, false));
            if colon_seen && expression.is_none() {
                emitter.emit(Rich::custom(span, "expected a string after `:`"));
            }
            if !colon_seen {
                emitter.emit(Rich::custom(
                    binding_name.span,
                    "expected `: \"<expression>\"` after the binding name",
                ));
            }
            DmnBinding {
                name: binding_name,
                expression: expression.unwrap_or_default(),
                span,
            }
        })
}

/// A `logic invoke <model> { ... }` kind: the invoked model's name and its
/// argument bindings. Committed once `invoke` matched: a missing name or
/// `{` is a reported error.
pub(crate) fn dmn_invoke_logic<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnLogic, MoxExtra<'src>> + Clone {
    dmn_keyword(dmn::INVOKE)
        .ignore_then(name().or_not())
        .then(braced_items(dmn_binding_decl().map(Some)))
        .map_with(|(model, bindings), e| (model, bindings, e.span()))
        .validate(|(model, bindings, span), _e, emitter| {
            let model = model.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if model.text.is_empty() {
                emitter.emit(Rich::custom(span, "expected a model name after `invoke`"));
            }
            if bindings.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `{` after the invoke model name",
                ));
            }
            DmnLogic::Invoke {
                model,
                bindings: bindings.unwrap_or_default(),
                span,
            }
        })
}

/// One string-valued `logic` kind (`table`/`expression`). Committed once
/// the kind word matched: a missing string is a reported error.
fn dmn_string_logic<'src>(
    keyword: &'static str,
    table: bool,
) -> impl Parser<'src, Tokens<'src>, DmnLogic, MoxExtra<'src>> + Clone {
    dmn_keyword(keyword)
        .ignore_then(string_lit().or_not())
        .map_with(move |value, e| (value, e.span()))
        .validate(move |(value, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    format!("expected a string after `{keyword}`"),
                ));
            }
            if table {
                DmnLogic::Table {
                    source: value.unwrap_or_default(),
                    span,
                }
            } else {
                DmnLogic::Expression {
                    text: value.unwrap_or_default(),
                    span,
                }
            }
        })
}

/// A `logic <kind>` clause of a model (BKM) or decision body. Committed
/// once `logic` matched: an unrecognized kind is a reported error and the
/// clause recovers as empty expression logic (the caller decides whether a
/// missing clause is tolerated). A `logic` clause carries no `;` — it ends
/// at its string or brace block.
pub(crate) fn dmn_logic_clause<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnLogic, MoxExtra<'src>> + Clone {
    dmn_keyword(dmn::LOGIC)
        .ignore_then(
            choice((
                dmn_string_logic(dmn::TABLE, true),
                dmn_string_logic(dmn::EXPRESSION, false),
                dmn_invoke_logic(),
            ))
            .or_not(),
        )
        .map_with(|kind, e| (kind, e.span()))
        .validate(|(kind, span), _e, emitter| match kind {
            Some(logic) => logic,
            None => {
                emitter.emit(Rich::custom(
                    span,
                    "expected `table`, `expression`, or `invoke` after `logic`",
                ));
                DmnLogic::Expression {
                    text: String::new(),
                    span,
                }
            }
        })
}

/// The span of a [`DmnLogic`] clause.
fn dmn_logic_span(logic: &DmnLogic) -> Span {
    match logic {
        DmnLogic::Table { span, .. }
        | DmnLogic::Expression { span, .. }
        | DmnLogic::Invoke { span, .. } => *span,
    }
}

/// A `model <name> { ... }` declaration of a knowledge block: one business
/// knowledge model. The three clause kinds (`parameters`, `output`,
/// `logic`) may appear in any source order and fold into the fixed shape;
/// a repeated `output`/`logic` clause is a reported error (the first
/// wins) and a missing `logic` clause is a reported error — the grammar
/// requires it, unlike a decision's.
pub(crate) fn dmn_bkm_decl<'src>() -> impl Parser<'src, Tokens<'src>, DmnBkm, MoxExtra<'src>> + Clone
{
    let item = choice((
        keyword_block(dmn::PARAMETERS, dmn_typed_entry().map(Some)).map(DmnBkmItem::Parameters),
        dmn_output_line().map(|out| DmnBkmItem::Output(out.type_ref)),
        dmn_logic_clause().map(DmnBkmItem::Logic),
    ))
    .map(Some)
    .or(junk_dmn_member("expected a model member").to(None));
    dmn_keyword(dmn::MODEL)
        .ignore_then(name().or_not())
        .then(braced_items(item))
        .map_with(|(decl_name, items), e| (decl_name, items, e.span()))
        .validate(|(decl_name, items, span), _e, emitter| {
            let decl_name = decl_name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if decl_name.text.is_empty() {
                emitter.emit(Rich::custom(span, "expected a model name after `model`"));
            }
            if items.is_none() {
                emitter.emit(Rich::custom(span, "expected `{` after the model name"));
            }
            let mut parameters = Vec::new();
            let mut output = None;
            let mut logic = None;
            for entry in items.into_iter().flatten() {
                match entry {
                    DmnBkmItem::Parameters(entries) => parameters.extend(entries),
                    DmnBkmItem::Output(type_ref) => {
                        if output.is_none() {
                            output = Some(type_ref);
                        } else {
                            emitter.emit(Rich::custom(type_ref.span, "duplicate `output` clause"));
                        }
                    }
                    DmnBkmItem::Logic(entry_logic) => {
                        if logic.is_none() {
                            logic = Some(entry_logic);
                        } else {
                            emitter.emit(Rich::custom(
                                dmn_logic_span(&entry_logic),
                                "duplicate `logic` clause",
                            ));
                        }
                    }
                }
            }
            if logic.is_none() {
                emitter.emit(Rich::custom(span, "expected `logic` in the model body"));
            }
            DmnBkm {
                name: decl_name,
                description: None,
                parameters,
                output: output.unwrap_or_else(|| TypeRef {
                    name: QualifiedName {
                        segments: Vec::new(),
                        span,
                    },
                    span,
                }),
                logic: logic.unwrap_or(DmnLogic::Expression {
                    text: String::new(),
                    span,
                }),
                span,
            }
        })
}

/// One `name ("when" "<FEEL>")? ";"` entry of a decision's `emits` block.
/// Committed once the name matched: a `when` clause without its string is
/// a reported error and the entry recovers without a condition.
pub(crate) fn dmn_emits_entry<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnEmits, MoxExtra<'src>> + Clone {
    let guard = kw(Token::When)
        .ignore_then(string_lit().or_not())
        .map_with(|when, e| (when, e.span()))
        .validate(|(when, span), _e, emitter| {
            if when.is_none() {
                emitter.emit(Rich::custom(span, "expected a string after `when`"));
            }
            when.unwrap_or_default()
        });
    name()
        .then(guard.map(Some).or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|((event, when), _semi), e| (event, when, e.span()))
        .validate(|(event, when, span), _e, _emitter| DmnEmits {
            event,
            when: when.unwrap_or_default(),
            span,
        })
}

/// The `emits { ... }` block of a decision body: the declared events the
/// decision's outcome publishes.
fn dmn_emits_block<'src>() -> impl Parser<'src, Tokens<'src>, Vec<DmnEmits>, MoxExtra<'src>> + Clone
{
    keyword_block(dmn::EMITS, dmn_emits_entry().map(Some))
}

/// A `decision <name> { ... }` declaration of a decisions block. The
/// `output`/`outputs` members, the optional `logic` clause, and the
/// optional `emits` block may be interleaved in any order; a repeated
/// `logic` clause is a reported error (the first wins). Unlike a model
/// (BKM), a decision may declare only its contract — a missing `logic` is
/// not an error.
pub(crate) fn dmn_decision_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnDecision, MoxExtra<'src>> + Clone {
    let item = choice((
        dmn_output_line().map(DmnDecisionItem::Output),
        keyword_block(dmn::OUTPUTS, dmn_typed_entry().map(Some)).map(|entries| {
            DmnDecisionItem::Outputs(
                entries
                    .into_iter()
                    .map(|entry| DmnOutput {
                        name: entry.name,
                        type_ref: entry.type_ref,
                        span: entry.span,
                    })
                    .collect(),
            )
        }),
        dmn_logic_clause().map(DmnDecisionItem::Logic),
        dmn_emits_block().map(DmnDecisionItem::Emits),
    ))
    .map(Some)
    .or(junk_dmn_member("expected a decision member").to(None));
    dmn_keyword(dmn::DECISION)
        .ignore_then(name().or_not())
        .then(braced_items(item))
        .map_with(|(decl_name, items), e| (decl_name, items, e.span()))
        .validate(|(decl_name, items, span), _e, emitter| {
            let decl_name = decl_name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if decl_name.text.is_empty() {
                emitter.emit(Rich::custom(
                    span,
                    "expected a decision name after `decision`",
                ));
            }
            if items.is_none() {
                emitter.emit(Rich::custom(span, "expected `{` after the decision name"));
            }
            let mut outputs = Vec::new();
            let mut logic = None;
            let mut emits = Vec::new();
            for entry in items.into_iter().flatten() {
                match entry {
                    DmnDecisionItem::Output(out) => outputs.push(out),
                    DmnDecisionItem::Outputs(named) => outputs.extend(named),
                    DmnDecisionItem::Logic(entry_logic) => {
                        if logic.is_none() {
                            logic = Some(entry_logic);
                        } else {
                            emitter.emit(Rich::custom(
                                dmn_logic_span(&entry_logic),
                                "duplicate `logic` clause",
                            ));
                        }
                    }
                    DmnDecisionItem::Emits(entries) => emits.extend(entries),
                }
            }
            DmnDecision {
                name: decl_name,
                description: None,
                outputs,
                logic,
                emits,
                span,
            }
        })
}

/// One `<name> <connective> <name>;` line of a requirement group. Committed
/// once the left name matched: a missing connective or target is a
/// reported error and the line recovers with an empty target.
pub(crate) fn dmn_requirement_line<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnRequirement, MoxExtra<'src>> + Clone {
    let connective = choice((
        dmn_keyword(dmn::REQUIRES).to(dmn::REQUIRES),
        dmn_keyword(dmn::INVOKES).to(dmn::INVOKES),
        dmn_keyword(dmn::GOVERNED_BY).to(dmn::GOVERNED_BY),
    ));
    name()
        .then(connective.or_not())
        .then(name().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(((line_name, connective), target), semi), e| {
            (line_name, connective, target, semi, e.span())
        })
        .validate(
            |(line_name, connective, target, _semi, span), _e, emitter| {
                if let Some(word) = connective {
                    if target.is_none() {
                        emitter.emit(Rich::custom(
                            span,
                            format!("expected a name after `{word}`"),
                        ));
                    }
                } else {
                    emitter.emit(Rich::custom(
                        line_name.span,
                        "expected `requires`, `invokes`, or `governed_by` after the name",
                    ));
                }
                DmnRequirement {
                    requiring: line_name,
                    required: target.unwrap_or_else(|| Name {
                        text: String::new(),
                        span,
                        escaped: false,
                    }),
                    span,
                }
            },
        )
}

/// One `information`/`knowledge`/`authority` requirement group: the word, a
/// committed `{`, and the group's lines.
pub(crate) fn dmn_requirement_group<'src>(
    keyword: &'static str,
    group: DmnReqGroup,
) -> impl Parser<'src, Tokens<'src>, (DmnReqGroup, Vec<DmnRequirement>), MoxExtra<'src>> + Clone {
    let line = dmn_requirement_line()
        .map(Some)
        .or(junk_dmn_member("expected a requirement line").to(None));
    keyword_block(keyword, line).map(move |lines| (group, lines))
}

/// The `requirements { ... }` block of a decision model body: the three
/// group kinds may be interleaved and repeated; lines fold into per-group
/// lists in source order.
pub(crate) fn dmn_requirements_block<'src>() -> impl Parser<
    'src,
    Tokens<'src>,
    (
        Vec<DmnRequirement>,
        Vec<DmnRequirement>,
        Vec<DmnRequirement>,
    ),
    MoxExtra<'src>,
> + Clone {
    let group = choice((
        dmn_requirement_group(dmn::INFORMATION, DmnReqGroup::Information),
        dmn_requirement_group(dmn::KNOWLEDGE, DmnReqGroup::Knowledge),
        dmn_requirement_group(dmn::AUTHORITY, DmnReqGroup::Authority),
    ))
    .map(Some)
    .or(junk_dmn_member("expected a requirement group").to(None));
    keyword_block(dmn::REQUIREMENTS, group).validate(|groups, _e, _emitter| {
        let mut information = Vec::new();
        let mut knowledge = Vec::new();
        let mut authority = Vec::new();
        for (group, lines) in groups {
            match group {
                DmnReqGroup::Information => information.extend(lines),
                DmnReqGroup::Knowledge => knowledge.extend(lines),
                DmnReqGroup::Authority => authority.extend(lines),
            }
        }
        (information, knowledge, authority)
    })
}

/// A `service <name> { ... }` declaration of a services block. The body
/// members may be interleaved in any order; a repeated `description` line
/// is idempotent (the first wins), and repeated `inputs`/`outputs`/
/// `encapsulates` lists merge in source order.
pub(crate) fn dmn_service_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnService, MoxExtra<'src>> + Clone {
    let item = choice((
        dmn_description_item().map(DmnServiceItem::Description),
        dmn_name_list(dmn::INPUTS, SERVICE_NAME_STOPS).map(DmnServiceItem::Inputs),
        dmn_name_list(dmn::OUTPUTS, SERVICE_NAME_STOPS).map(DmnServiceItem::Outputs),
        dmn_name_list(dmn::ENCAPSULATES, SERVICE_NAME_STOPS).map(DmnServiceItem::Encapsulates),
    ))
    .map(Some)
    .or(junk_dmn_member("expected a service member").to(None));
    dmn_keyword(dmn::SERVICE)
        .ignore_then(name().or_not())
        .then(braced_items(item))
        .map_with(|(decl_name, items), e| (decl_name, items, e.span()))
        .validate(|(decl_name, items, span), _e, emitter| {
            let decl_name = decl_name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if decl_name.text.is_empty() {
                emitter.emit(Rich::custom(
                    span,
                    "expected a service name after `service`",
                ));
            }
            if items.is_none() {
                emitter.emit(Rich::custom(span, "expected `{` after the service name"));
            }
            let mut description = None;
            let mut inputs = Vec::new();
            let mut outputs = Vec::new();
            let mut encapsulates = Vec::new();
            for entry in items.into_iter().flatten() {
                match entry {
                    DmnServiceItem::Description(value) => {
                        if description.is_none() {
                            description = Some(value);
                        }
                    }
                    DmnServiceItem::Inputs(names) => inputs.extend(names),
                    DmnServiceItem::Outputs(names) => outputs.extend(names),
                    DmnServiceItem::Encapsulates(names) => encapsulates.extend(names),
                }
            }
            DmnService {
                name: decl_name,
                description,
                inputs,
                outputs,
                encapsulates,
                span,
            }
        })
}

/// The `direction <name>;` line of a view body. Committed once `direction`
/// matched: a missing name is a reported error.
pub(crate) fn dmn_direction_item<'src>(
) -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    dmn_keyword(dmn::DIRECTION)
        .ignore_then(name().or_not())
        .then(kw(Token::Other(';')).or_not())
        .map_with(|(target, semi), e| (target, semi, e.span()))
        .validate(|(target, _semi, span), _e, emitter| {
            let target = target.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if target.text.is_empty() {
                emitter.emit(Rich::custom(span, "expected a name after `direction`"));
            }
            target
        })
}

/// A `view <name> { ... }` declaration of a views block. The body members
/// may be interleaved in any order; a repeated `description` line is
/// idempotent (the first wins), and repeated `show` lists merge in source
/// order. The `direction` line is optional.
pub(crate) fn dmn_view_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnView, MoxExtra<'src>> + Clone {
    let item = choice((
        dmn_description_item().map(DmnViewItem::Description),
        dmn_name_list(dmn::SHOW, VIEW_NAME_STOPS).map(DmnViewItem::Show),
        dmn_direction_item().map(DmnViewItem::Direction),
    ))
    .map(Some)
    .or(junk_dmn_member("expected a view member").to(None));
    dmn_keyword(dmn::VIEW)
        .ignore_then(name().or_not())
        .then(braced_items(item))
        .map_with(|(decl_name, items), e| (decl_name, items, e.span()))
        .validate(|(decl_name, items, span), _e, emitter| {
            let decl_name = decl_name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if decl_name.text.is_empty() {
                emitter.emit(Rich::custom(span, "expected a view name after `view`"));
            }
            if items.is_none() {
                emitter.emit(Rich::custom(span, "expected `{` after the view name"));
            }
            let mut description = None;
            let mut show = Vec::new();
            let mut direction = None;
            for entry in items.into_iter().flatten() {
                match entry {
                    DmnViewItem::Description(value) => {
                        if description.is_none() {
                            description = Some(value);
                        }
                    }
                    DmnViewItem::Show(names) => show.extend(names),
                    DmnViewItem::Direction(name) => {
                        if direction.is_none() {
                            direction = Some(name);
                        } else {
                            emitter.emit(Rich::custom(name.span, "duplicate `direction` line"));
                        }
                    }
                }
            }
            DmnView {
                name: decl_name,
                description,
                show,
                direction,
                span,
            }
        })
}

/// An `inputs { ... }` block of a decision model body: typed entries.
pub(crate) fn dmn_inputs_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<DmnInput>, MoxExtra<'src>> + Clone {
    keyword_block(dmn::INPUTS, dmn_typed_entry().map(Some))
}

/// A `decisions { ... }` block of a decision model body.
pub(crate) fn dmn_decisions_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<DmnDecision>, MoxExtra<'src>> + Clone {
    keyword_block(dmn::DECISIONS, dmn_decision_decl().map(Some))
}

/// A `knowledge { ... }` block of a decision model body: sources and
/// models (BKMs) interleaved in any order, folded into per-kind lists.
pub(crate) fn dmn_knowledge_block<'src>(
) -> impl Parser<'src, Tokens<'src>, (Vec<DmnKnowledgeSource>, Vec<DmnBkm>), MoxExtra<'src>> + Clone
{
    let item = choice((
        dmn_source_decl().map(DmnKnowledgeItem::Source),
        dmn_bkm_decl().map(DmnKnowledgeItem::Model),
    ))
    .map(Some)
    .or(junk_dmn_member("expected a knowledge source or model").to(None));
    keyword_block(dmn::KNOWLEDGE, item).validate(|items, _e, _emitter| {
        let mut sources = Vec::new();
        let mut models = Vec::new();
        for entry in items {
            match entry {
                DmnKnowledgeItem::Source(source) => sources.push(source),
                DmnKnowledgeItem::Model(model) => models.push(model),
            }
        }
        (sources, models)
    })
}

/// A `services { ... }` block of a decision model body.
pub(crate) fn dmn_services_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<DmnService>, MoxExtra<'src>> + Clone {
    keyword_block(dmn::SERVICES, dmn_service_decl().map(Some))
}

/// A `views { ... }` block of a decision model body.
pub(crate) fn dmn_views_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<DmnView>, MoxExtra<'src>> + Clone {
    keyword_block(dmn::VIEWS, dmn_view_decl().map(Some))
}

/// One member of a `decision_model` body: metadata lines and the six
/// blocks, interleaved in any source order. The junk recovery is the last
/// alternative, so it never competes with a real member.
pub(crate) fn dmn_model_member<'src>(
) -> impl Parser<'src, Tokens<'src>, Option<DmnModelMember>, MoxExtra<'src>> + Clone {
    choice((
        dmn_namespace_decl().map(DmnModelMember::Meta),
        dmn_metadata_decl().map(DmnModelMember::Meta),
        dmn_inputs_block().map(DmnModelMember::Inputs),
        dmn_knowledge_block().map(|(sources, models)| DmnModelMember::Knowledge(sources, models)),
        dmn_decisions_block().map(DmnModelMember::Decisions),
        dmn_requirements_block().map(|(information, knowledge, authority)| {
            DmnModelMember::Requirements(information, knowledge, authority)
        }),
        dmn_services_block().map(DmnModelMember::Services),
        dmn_views_block().map(DmnModelMember::Views),
    ))
    .map(Some)
    .or(junk_dmn_member("expected a decision model member").to(None))
}

/// A `decision_model <name> { ... }` declaration. The member kinds may be
/// interleaved in any order; the AST folds them into per-kind lists. A
/// repeated `namespace` line is a reported error (the first wins);
/// repeated metadata lines are a driver concern.
pub(crate) fn decision_model_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DmnDecisionModel, MoxExtra<'src>> + Clone {
    dmn_keyword(dmn::DECISION_MODEL)
        .ignore_then(name().or_not())
        .then(braced_items(dmn_model_member()))
        .map_with(|(decl_name, members), e| (decl_name, members, e.span()))
        .validate(|(decl_name, members, span), _e, emitter| {
            let decl_name = decl_name.unwrap_or_else(|| Name {
                text: String::new(),
                span,
                escaped: false,
            });
            if decl_name.text.is_empty() {
                emitter.emit(Rich::custom(span, "expected a name after `decision_model`"));
            }
            if members.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `{` after the decision model name",
                ));
            }
            let mut namespace = None;
            let mut metadata = Vec::new();
            let mut inputs = Vec::new();
            let mut knowledge_sources = Vec::new();
            let mut models = Vec::new();
            let mut decisions = Vec::new();
            let mut information_requirements = Vec::new();
            let mut knowledge_requirements = Vec::new();
            let mut authority_requirements = Vec::new();
            let mut services = Vec::new();
            let mut views = Vec::new();
            for member in members.into_iter().flatten() {
                match member {
                    DmnModelMember::Meta(DmnModelMeta::Namespace(qname)) => {
                        if namespace.is_none() {
                            namespace = Some(qname);
                        } else {
                            emitter.emit(Rich::custom(qname.span, "duplicate `namespace` line"));
                        }
                    }
                    DmnModelMember::Meta(DmnModelMeta::Metadata(entry)) => metadata.push(entry),
                    DmnModelMember::Inputs(entries) => inputs.extend(entries),
                    DmnModelMember::Knowledge(sources, bkm_models) => {
                        knowledge_sources.extend(sources);
                        models.extend(bkm_models);
                    }
                    DmnModelMember::Decisions(decls) => decisions.extend(decls),
                    DmnModelMember::Requirements(information, knowledge, authority) => {
                        information_requirements.extend(information);
                        knowledge_requirements.extend(knowledge);
                        authority_requirements.extend(authority);
                    }
                    DmnModelMember::Services(decls) => services.extend(decls),
                    DmnModelMember::Views(decls) => views.extend(decls),
                }
            }
            DmnDecisionModel {
                name: decl_name,
                namespace,
                metadata,
                inputs,
                knowledge_sources,
                models,
                decisions,
                information_requirements,
                knowledge_requirements,
                authority_requirements,
                services,
                views,
                span,
            }
        })
}

/// One top-level item of a `.dmn` file.
#[derive(Clone)]
pub(crate) enum DmnFileItem {
    Import(Import),
    Model(Box<DmnDecisionModel>),
    Junk,
}

/// Consumes a run of tokens up to the next `import` keyword or the
/// contextual `decision_model` word (or end of input). File-level recovery
/// for `.dmn` files: always consumes at least one token (guaranteeing
/// progress) and emits an error for the skipped region.
pub(crate) fn junk_dmn_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let rest = select! {
        t if !matches!(t, Token::Import | Token::Ident(dmn::DECISION_MODEL)) => ()
    };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(
                e.span(),
                "expected an import or `decision_model` declaration",
            ));
        })
}

/// Folds the top-level items of a `.dmn` file: imports must precede the
/// model and there is exactly one model. Violations are reported as
/// (span, message) problems; the AST recovers the first model and drops
/// the offending items regardless.
pub(crate) fn fold_dmn_file(items: Vec<DmnFileItem>) -> (DmnFile, Vec<(Span, String)>) {
    let mut imports = Vec::new();
    let mut model = None;
    let mut problems = Vec::new();
    for item in items {
        match item {
            DmnFileItem::Import(decl) => {
                if model.is_none() {
                    imports.push(decl);
                } else {
                    problems.push((
                        decl.span,
                        "`import` after the `decision_model` declaration".to_string(),
                    ));
                }
            }
            DmnFileItem::Model(decl) => {
                if model.is_none() {
                    model = Some(*decl);
                } else {
                    problems.push((
                        decl.span,
                        "duplicate `decision_model` declaration".to_string(),
                    ));
                }
            }
            DmnFileItem::Junk => {}
        }
    }
    (DmnFile { imports, model }, problems)
}

pub(crate) fn dmn_file<'src>() -> impl Parser<'src, Tokens<'src>, DmnFile, MoxExtra<'src>> + Clone {
    let item = choice((
        dmn_import_decl().map(DmnFileItem::Import),
        decision_model_decl().map(|decl| DmnFileItem::Model(Box::new(decl))),
    ))
    .or(junk_dmn_file().to(DmnFileItem::Junk));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map_with(|items, e| (fold_dmn_file(items), e.span()))
        .validate(|((file, problems), span), _, emitter| {
            for (span, message) in problems {
                emitter.emit(Rich::custom(span, message));
            }
            if file.model.is_none() {
                emitter.emit(Rich::custom(
                    Span::from(span.end..span.end),
                    "expected a `decision_model` declaration",
                ));
            }
            file
        })
}

/// The outcome of parsing a `.dmn` source: as much of the file as could be
/// recovered, plus all encountered errors. Parsing never panics and always
/// produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnParseResult {
    /// The recovered decision model file, or `None` if no output could be
    /// produced at all.
    pub ast: Option<DmnFile>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

/// Lex and parse a `.dmn` source text: `import` declarations followed by
/// the `decision_model` declaration (imports must come first).
///
/// This function never panics and always recovers as much of the AST as
/// possible; check [`DmnParseResult::errors`] for syntax problems. Doc
/// comment runs (`///`) directly above a `decision`, `model`, `source`,
/// `service`, or `view` declaration become its description.
pub fn parse_dmn(source: &str) -> DmnParseResult {
    let (tokens, comments) = match lex_with_comments(source) {
        Ok(result) => result,
        Err(error) => {
            return DmnParseResult {
                ast: None,
                errors: vec![ParseError {
                    message: error.to_string(),
                    span: error.span,
                }],
            }
        }
    };
    let (mut ast, errors) = dmn_file().parse(Tokens::new(&tokens)).into_output_errors();
    if let Some(file) = ast.as_mut() {
        attach_dmn_docs(file, &comments, source);
    }
    DmnParseResult {
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

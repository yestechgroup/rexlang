//! The `.evt` entry points: event-contract compilation against the
//! imported domain union.

use salsa::Database as Db;

use crate::compilation::EventCompilation;
use crate::entry::finish_surface;
use crate::events;
use crate::imports::DomainImports;
use crate::session::{parse_evt_query, Database, SourceFile};
use crate::union::{domain_files, lower_domains};

/// Compiles an `.evt` event-contract file against its imported domain models
/// to the standalone [`rex_ir::events::EventModel`] (memoized by salsa):
/// parse the contract file, lower every named domain through the ordinary
/// domain pipeline (the same union-namespace rule
/// [`compile_actors_str`](crate::compile_actors_str) uses), then lower and
/// validate the contract against the resolved domain namespaces.
#[salsa::tracked]
pub fn compile_evt(
    db: &dyn Db,
    contract: SourceFile,
    domains: Vec<SourceFile>,
) -> EventCompilation {
    compile_evt_with(db, contract, domains, &DomainImports::default())
}

/// The body of [`compile_evt`], parameterized over the provided import
/// content for the domain files (see [`compile_file`] for the shape).
fn compile_evt_with(
    db: &dyn Db,
    contract: SourceFile,
    domains: Vec<SourceFile>,
    imports: &DomainImports,
) -> EventCompilation {
    let contract_path = contract.path(db);
    let parsed = parse_evt_query(db, contract);
    let domains = lower_domains(
        db,
        &contract_path,
        parsed
            .ast
            .as_ref()
            .map(|ast| ast.imports.as_slice())
            .unwrap_or_default(),
        &domains,
        imports,
    );

    let (model, diagnostics) = events::compile_evt_file(
        &contract_path,
        &contract.text(db),
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domains.domain_units,
    );
    finish_surface(domains, model, diagnostics, EventCompilation::assemble)
}
/// Compiles an `.evt` event-contract file against its imported domain models
/// in one call.
///
/// Each domain is a `(path, source)` pair; a contract import resolves by
/// exact path match with a lexical resolution relative to the contract
/// file's directory as the fallback (the same rule
/// [`compile_actors_str`](crate::compile_actors_str) applies). Extra pairs
/// no import names are ignored; an import no provided pair names is the
/// error `imported file "<path>" was not provided`. The result carries
/// diagnostics from every file involved, each tagged with its path — the
/// same shape [`compile_actors_str`](crate::compile_actors_str) returns, so
/// a host renders both with ariadne identically.
///
/// ```
/// let domains = [(
///     "orders.mox".to_string(),
///     "package nz.example.orders\n\nclass Order { String orderId }".to_string(),
/// )];
/// let source = concat!(
///     "import \"orders.mox\"\n",
///     "\n",
///     "event OrderPlaced version \"1.0.0\" {\n",
///     "    orderId: String;\n",
///     "}\n",
///     "\n",
///     "channel orders { publishes OrderPlaced; }\n",
/// );
/// let compilation = rex_driver::compile_evt_str(
///     "orders.evt",
///     source,
///     &domains,
///     &rex_driver::DomainImports::default(),
/// );
/// assert!(compilation.diagnostics.is_empty());
/// let model = compilation.model.unwrap();
/// assert_eq!(model.events.len(), 1);
/// assert_eq!(model.channels[0].publishes.len(), 1);
/// ```
///
/// # Normative contract
///
/// ## Entry points
///
/// - [`compile_evt_str`] — parse, domain lowering, contract lowering and
///   validation, in one call.
/// - [`compile_evt`] / [`parse_evt_query`] — the salsa-tracked queries the
///   one-shot wrapper runs on a fresh [`Database`].
///
/// ## Resolution semantics
///
/// Event payload field types resolve against the **union namespace of the
/// imported domains** (the same rule multi-file `.mox` compiles follow)
/// through the shared helper `resolve_ddd_type`: the `rex-syntax`
/// primitives resolve everywhere; every other reference resolves with the
/// `.mox` cross-package rules (a bare single-segment name must be unique
/// across all domain packages — ambiguity, unknown names, and
/// actors-block-as-type each report through the same diagnostics the `.mox`
/// and `.ddd` surfaces use). Any declared kind resolves: class, enum,
/// datatype, interface, vocabulary.
///
/// ## Validation rules
///
/// Any error-severity diagnostic anywhere (the contract file or an imported
/// domain) drops the artifact; warnings do not.
///
/// 1. **Syntax** — parse errors are reported against the `.evt` file.
/// 2. **Imports** — every `import "<path>"` must name a provided domain,
///    else `imported file "<path>" was not provided`.
/// 3. **Field types** — an event payload field type that is neither a
///    primitive nor resolvable in the domain namespaces errors, naming the
///    type (`unknown type 'X'`).
/// 4. **Publications** — every `publishes X` entry of a channel must name
///    an event declared in the same file (`unknown event 'X' on channel
///    'C'`).
/// 5. **Subscription events** — every entry of a subscription's
///    `events [...]` list must name an event declared in the same file
///    (`unknown event 'X' in subscription 'S'`).
/// 6. **Uniqueness** — event, channel, and subscription names are unique
///    within the file, and payload field names are unique within their
///    event.
/// 7. **Subscription shape** — an empty `events [...]` list or an empty
///    consumer name errors (`subscription 'S' subscribes to no events` /
///    `subscription 'S' names an empty consumer`; the empty consumer is
///    only reachable through parser recovery, whose syntax error fires
///    alongside).
/// 8. **CloudEvents attributes** — once an event carries a `cloudEvents`
///    block, its `type` and `source` must be non-empty (`cloudEvents
///    type/source on event 'X' must not be empty`); when present they are
///    authoritative for a downstream generator, overriding any derived
///    mapping.
/// 9. **Governance `pii`** — every entry of a `pii [ ... ]` line must name
///    a payload field of the same event (`unknown payload field 'x' in pii
///    of event 'Y'`). The other governance lines (`owner`,
///    `classification`, `retention`) are verbatim labels the driver never
///    interprets.
/// 10. **Delivery mode** — the `mode` entry of a `delivery` block must be
///     one of `at_most_once`, `at_least_once`, `effectively_once`
///     (`unknown delivery mode 'x' on subscription 'S'`, with help listing
///     them).
/// 11. **Delivery tuning knobs** — `maxAttempts`, `backoffSeconds`, and
///     `timeoutSeconds` must be positive (`'maxAttempts' on subscription
///     'S' must be positive`).
/// 12. **Dead-letter channel** — the `deadLetter` entry must name a channel
///     declared in the same file (`unknown dead-letter channel 'x' on
///     subscription 'S'`).
/// 13. **Subscription `when`** — the filter's interior (sliced strictly
///     inside the parens, trimmed — the `when`-condition convention) must
///     parse as a rex-expr condition and type `boolean` against **every**
///     event in the subscription's `events [...]` list: the expression is
///     typed with a synthetic per-event class (the subscribed event's
///     payload fields as the implicit `self`, over the imported domains'
///     class union — `DomainTypes::insert_class`, never a fake `Model`).
///     A field absent from any subscribed event is an error **by design** —
///     a filter is a routing decision per event, so an event without the
///     field cannot be routed (the help names the subscription and, for
///     per-event failures, the event). A parse failure leaves the filter
///     out of the artifact; a type failure still lowers the verbatim
///     interior (the `.cmmn` precedent) — the artifact drops on any error
///     either way.
///
/// Scope labels are **never** validated: a channel's `of <scope>` clause
/// and a subscription's `scope <name>` binding are opaque topic-template
/// labels carried verbatim into the artifact (`ChannelDef::scope` /
/// `SubscriptionDef::scope`) — no resolution, no uniqueness, no
/// cross-checks; interpretation is a consumer concern.
///
/// Diagnostics are ordered deterministically: the contract file's parse
/// diagnostics first, then the import errors, then each domain's own
/// diagnostics (import order), then the contract's semantic diagnostics
/// grouped by declaration kind — events, then channels, then subscriptions
/// — each group in source order, matching the artifact's declaration
/// order. Within an event: its fields (source order), then `pii`, then
/// `cloudEvents`; within a subscription: its `events [...]` list, then
/// `when`, then `delivery`. Every diagnostic is tagged with its file's
/// path, so hosts (and the CLI) render them grouped per file. On any error
/// the result's `model` is `None`; `domains_model` still carries the
/// imported domains when they lowered.
pub fn compile_evt_str(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
) -> EventCompilation {
    let db = Database::new();
    let contract = SourceFile::new(&db, path.to_string(), source.to_string());
    let files = domain_files(&db, domains);
    compile_evt_with(&db, contract, files, imports)
}

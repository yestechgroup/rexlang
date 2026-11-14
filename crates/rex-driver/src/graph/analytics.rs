//! Cross-policy analytics over the semantic graph artifact (issue
//! rexlang#246): separation of duty, monotonicity, dead capabilities, and
//! reachability.
//!
//! Every analytic runs on a [`GraphIndex`] — the artifact alone, no sources,
//! no recompile — and answers in the same versioned envelope as the
//! structural queries ([`super::query::GRAPH_QUERY_FORMAT_VERSION`]):
//! `formatVersion`, the camelCase `query` name, the parameters it was given
//! (echoed as resolved ids), and `results`.
//!
//! # Two rules make the answers trustworthy
//!
//! 1. **Per application.** Capability names pool only inside one
//!    import-connected application, and the object graph never crosses an
//!    application boundary. An analytic never joins two applications'
//!    policies: an actor in one application is invisible to a capability in
//!    another, and a capability pair split across applications has no shared
//!    policy scope at all.
//! 2. **Conservative may-analysis with an explicit `unknown`.** A `when`
//!    guard is treated as satisfiable (may-analysis — the tool never claims
//!    a guarded grant can never fire), and a bare `cedar { … }` entry is
//!    opaque: it never contributes a permit, and any question it could
//!    change the answer to is reported as `unknown`, never as a definite
//!    yes or no.
//!
//! # Shared terms (mirroring `crate::lower::actors`)
//!
//! These are the compiler's own definitions, rederived from the artifact:
//!
//! - [`lineage`] — the actor plus every transitive `extends` ancestor
//!   (cycle-guarded, the compiler's walk).
//! - [`permits`] — every `permit` grant entry of any lineage member plus
//!   every permit an inbound delegation hands a lineage member (delegation
//!   permits count for descendants — the compiler's `effective_permits`
//!   rule). `cedar` entries never contribute.
//! - [`forbids`] — the same walk for forbid entries (grants and inbound
//!   delegations).
//! - [`open`] — `permits \ forbids`.
//! - [`unknown`] — a lineage member carries an `opaqueGrant` edge, so some
//!   authority over this actor is not interpretable.
//!
//! The optional session filter narrows both permits and forbids: an edge
//! applies to a `human` (or `agent`) session when its `sessionKind` is that
//! kind or the unmodified default (`any`).
//!
//! # Findings
//!
//! Every result type implements [`Finding`], so a caller can gate CI with
//! `--fail-on-findings` without turning a policy finding into a language
//! error. A finding is a **definite** negative answer only:
//!
//! | query | finding |
//! |---|---|
//! | `separation-of-duty` | an actor holds both capabilities (declared or not) |
//! | `monotonicity` | a definite gap — `missing` non-empty and neither side opaque |
//! | `dead-capabilities` | the status is `dead` (`unknown` and `latent` are not findings) |
//! | `reachability` | the target class is unreachable (the thing asked for is not there) |

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rex_ir::graph::{kinds, GraphEdge, GraphNode};
use rex_ir::{GrantEffect, SessionKind};
use serde::{Deserialize, Serialize};

use super::index::{GraphIndex, ResolveError};
use super::query::GRAPH_QUERY_FORMAT_VERSION;

// ---------------------------------------------------------------------------
// Shared terms
// ---------------------------------------------------------------------------

/// The `lineage` of an actor: the actor itself plus every transitive
/// `extends` ancestor, cycle-guarded exactly like the compiler's
/// `actor_lineage`.
///
/// Only `extends` edges inside the actor's application are followed, so an
/// artifact that (invalidly) draws an edge across applications cannot leak
/// one application's policy into another.
pub fn lineage(index: &GraphIndex<'_>, actor: &str) -> BTreeSet<String> {
    let mut lineage: BTreeSet<String> = BTreeSet::new();
    let mut current = Some(actor.to_string());
    while let Some(name) = current {
        if !lineage.insert(name.clone()) {
            break;
        }
        current = index
            .edges_from(&name)
            .iter()
            .filter(|edge| edge.kind == kinds::EXTENDS)
            .find(|edge| same_application(index, &name, &edge.to))
            .map(|edge| edge.to.clone());
    }
    lineage
}

/// The capability ids the actor may exercise: every `permit` grant entry of
/// any lineage member plus every permit an inbound `delegates` edge hands a
/// lineage member.
///
/// Mirrors `effective_permits` in `crate::lower::actors`: a `delegates` edge
/// counts only when its effect is `permit` (an inbound `forbid` delegation is
/// not authority), and a `cedar` entry contributes nothing — its opaque
/// authority is [`unknown`], never a permit.
pub fn permits(
    index: &GraphIndex<'_>,
    actor: &str,
    session: Option<SessionKind>,
) -> BTreeSet<String> {
    let mut permits = BTreeSet::new();
    for member in lineage(index, actor) {
        for edge in index.edges_from(&member) {
            let counts = match edge.kind.as_str() {
                kinds::PERMITS => edge.effect != Some(GrantEffect::Forbid),
                kinds::DELEGATES => edge.effect == Some(GrantEffect::Permit),
                _ => false,
            };
            if counts
                && applies_to(edge, session)
                && is_capability(index, &edge.to)
                && same_application(index, actor, &edge.to)
            {
                permits.insert(edge.to.clone());
            }
        }
    }
    permits
}

/// The capability ids the actor is forbidden: every `forbid` grant entry of
/// any lineage member plus every inbound `delegates` forbid.
pub fn forbids(
    index: &GraphIndex<'_>,
    actor: &str,
    session: Option<SessionKind>,
) -> BTreeSet<String> {
    let mut forbids = BTreeSet::new();
    for member in lineage(index, actor) {
        for edge in index.edges_from(&member) {
            let counts = match edge.kind.as_str() {
                kinds::FORBIDS => edge.effect != Some(GrantEffect::Permit),
                kinds::DELEGATES => edge.effect == Some(GrantEffect::Forbid),
                _ => false,
            };
            if counts
                && applies_to(edge, session)
                && is_capability(index, &edge.to)
                && same_application(index, actor, &edge.to)
            {
                forbids.insert(edge.to.clone());
            }
        }
    }
    forbids
}

/// `open(A) = permits(A) \ forbids(A)`: the authority the actor may actually
/// exercise. A `when` guard is treated as satisfiable (may-analysis), so a
/// guarded permit counts.
pub fn open(index: &GraphIndex<'_>, actor: &str, session: Option<SessionKind>) -> BTreeSet<String> {
    let mut open = permits(index, actor, session);
    let forbidden = forbids(index, actor, session);
    open.retain(|capability| !forbidden.contains(capability));
    open
}

/// Whether the actor's authority is partly uninterpretable: any lineage
/// member carries an `opaqueGrant` edge (a bare `cedar { … }` entry).
pub fn unknown(index: &GraphIndex<'_>, actor: &str) -> bool {
    lineage(index, actor).iter().any(|member| {
        index.edges_from(member).iter().any(|edge| {
            edge.kind == kinds::OPAQUE_GRANT && same_application(index, member, &edge.to)
        })
    })
}

/// Whether a grant edge applies to the requested session kind: unmodified
/// (`any`) entries apply to every session; a restricted entry applies only to
/// its own kind. `None` asks for every session, so every edge applies.
fn applies_to(edge: &GraphEdge, session: Option<SessionKind>) -> bool {
    match session {
        None => true,
        Some(requested) => edge.session_kind.is_any() || edge.session_kind == requested,
    }
}

/// Whether two node ids name nodes of the same application (an unknown id is
/// never in the same application).
fn same_application(index: &GraphIndex<'_>, left: &str, right: &str) -> bool {
    match (index.node(left), index.node(right)) {
        (Some(left), Some(right)) => left.application == right.application,
        _ => false,
    }
}

fn is_capability(index: &GraphIndex<'_>, id: &str) -> bool {
    index
        .node(id)
        .is_some_and(|node| node.kind == kinds::CAPABILITY)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why an analytic could not answer.
///
/// Reference resolution failures carry the index's sorted candidates, exactly
/// like the structural queries; a resolved reference of the wrong kind is its
/// own variant so the CLI can say precisely what was expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalyticsError {
    /// The reference did not resolve ([`ResolveError`]).
    Resolve(ResolveError),
    /// The reference resolved to something that is not an actor.
    NotAnActor {
        /// The reference as the user wrote it.
        reference: String,
        /// The kind the reference resolved to.
        kind: String,
    },
    /// The reference resolved to something that is not a capability.
    NotACapability {
        /// The reference as the user wrote it.
        reference: String,
        /// The kind the reference resolved to.
        kind: String,
    },
    /// The reference resolved to something that is not a class or feature.
    NotAClass {
        /// The reference as the user wrote it.
        reference: String,
        /// The kind the reference resolved to.
        kind: String,
    },
    /// Reachability needs exactly one start: `--from-actor` or
    /// `--from-capability`.
    MissingStart,
}

impl AnalyticsError {
    /// The sorted candidate node ids for a resolution failure (empty for
    /// every other variant).
    pub fn candidates(&self) -> &[String] {
        match self {
            AnalyticsError::Resolve(error) => error.candidates(),
            _ => &[],
        }
    }
}

impl std::fmt::Display for AnalyticsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyticsError::Resolve(error) => write!(formatter, "{error}"),
            AnalyticsError::NotAnActor { reference, kind } => {
                write!(formatter, "{reference:?} is a {kind}, not an actor")
            }
            AnalyticsError::NotACapability { reference, kind } => {
                write!(formatter, "{reference:?} is a {kind}, not a capability")
            }
            AnalyticsError::NotAClass { reference, kind } => {
                write!(
                    formatter,
                    "{reference:?} is a {kind}, not a class or feature"
                )
            }
            AnalyticsError::MissingStart => formatter
                .write_str("reachability needs exactly one of --from-actor or --from-capability"),
        }
    }
}

impl std::error::Error for AnalyticsError {}

impl From<ResolveError> for AnalyticsError {
    fn from(error: ResolveError) -> Self {
        AnalyticsError::Resolve(error)
    }
}

fn resolve_actor<'a>(
    index: &GraphIndex<'a>,
    reference: &str,
) -> Result<&'a GraphNode, AnalyticsError> {
    let node = index.resolve(reference)?;
    if node.kind != kinds::ACTOR {
        return Err(AnalyticsError::NotAnActor {
            reference: reference.to_string(),
            kind: node.kind.clone(),
        });
    }
    Ok(node)
}

fn resolve_capability<'a>(
    index: &GraphIndex<'a>,
    reference: &str,
) -> Result<&'a GraphNode, AnalyticsError> {
    let node = index.resolve(reference)?;
    if node.kind != kinds::CAPABILITY {
        return Err(AnalyticsError::NotACapability {
            reference: reference.to_string(),
            kind: node.kind.clone(),
        });
    }
    Ok(node)
}

// ---------------------------------------------------------------------------
// The envelope
// ---------------------------------------------------------------------------

/// The versioned envelope every analytic answers in — the structural query
/// envelope's shape ([`super::query::QueryEnvelope`]) with analytic result
/// records.
///
/// `formatVersion` is [`GRAPH_QUERY_FORMAT_VERSION`]; `query` is the
/// camelCase analytic name; the parameter echoes appear only when the query
/// took that parameter, as resolved node ids; `results` is the analytic's own
/// record type. Absent fields are omitted rather than written as `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsEnvelope<R> {
    /// The envelope format version ([`GRAPH_QUERY_FORMAT_VERSION`]).
    pub format_version: u32,
    /// The analytic name (`separationOfDuty`, `monotonicity`,
    /// `deadCapabilities`, `reachability`).
    pub query: String,
    /// The `--pair` echo: the two resolved capability ids, sorted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<Vec<String>>,
    /// The `--actor` echo: the resolved actor id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// The `--session` echo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionKind>,
    /// The `--from-actor` echo: the resolved actor id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The `--from-capability` echo: the resolved capability id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_capability: Option<String>,
    /// The `--to` echo: the resolved target id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// The analytic's result records, in the query's documented order.
    pub results: Vec<R>,
}

impl<R: Serialize> AnalyticsEnvelope<R> {
    /// Serializes the envelope compactly, or pretty-printed (2-space) when
    /// `pretty` is set. Whitespace is the only difference.
    pub fn to_json(&self, pretty: bool) -> Result<String, serde_json::Error> {
        if pretty {
            serde_json::to_string_pretty(self)
        } else {
            serde_json::to_string(self)
        }
    }
}

impl<R: Finding> AnalyticsEnvelope<R> {
    /// The number of definite findings in the results.
    pub fn finding_count(&self) -> usize {
        self.results
            .iter()
            .filter(|result| result.is_finding())
            .count()
    }

    /// Whether any result is a definite finding (`--fail-on-findings`).
    pub fn has_findings(&self) -> bool {
        self.finding_count() > 0
    }
}

/// A type-erased analytic answer: a host can print and gate on a
/// heterogeneous set of answers without naming every result type.
pub trait AnalyticAnswer {
    /// Serializes the envelope, pretty-printed (2-space) when `pretty`.
    fn render_json(&self, pretty: bool) -> Result<String, serde_json::Error>;
    /// Whether the answer contains a definite finding.
    fn has_findings(&self) -> bool;
    /// The number of definite findings.
    fn finding_count(&self) -> usize;
}

impl<R: Finding + Serialize> AnalyticAnswer for AnalyticsEnvelope<R> {
    fn render_json(&self, pretty: bool) -> Result<String, serde_json::Error> {
        self.to_json(pretty)
    }

    fn has_findings(&self) -> bool {
        AnalyticsEnvelope::has_findings(self)
    }

    fn finding_count(&self) -> usize {
        AnalyticsEnvelope::finding_count(self)
    }
}

/// A result record that can be a **definite** finding.
///
/// An uncertain answer (`unknown`) is never a finding: gating CI must not
/// turn "this artifact holds a construct the tool cannot interpret" into a
/// failure the author cannot fix in the model.
pub trait Finding {
    /// Whether this record is a definite finding.
    fn is_finding(&self) -> bool;
}

fn envelope<R>(query: &str) -> AnalyticsEnvelope<R> {
    AnalyticsEnvelope {
        format_version: GRAPH_QUERY_FORMAT_VERSION,
        query: query.to_string(),
        pair: None,
        actor: None,
        session: None,
        from: None,
        from_capability: None,
        to: None,
        results: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 1. separation-of-duty
// ---------------------------------------------------------------------------

/// The `separation-of-duty` options.
#[derive(Debug, Clone, Copy, Default)]
pub struct SeparationOfDutyOptions<'a> {
    /// Ask the question for this capability pair — declared or not. `None`
    /// answers every declared `never_both` pair.
    pub pair: Option<(&'a str, &'a str)>,
    /// Restrict the actors examined and reported to this one.
    pub actor: Option<&'a str>,
    /// Refilter the grants by session kind.
    pub session: Option<SessionKind>,
}

/// One capability of a pair and the actors that hold it in `open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityHolders {
    /// The capability id.
    pub capability: String,
    /// The actors holding it (`open`), sorted by id. Empty when nobody does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<String>,
}

/// One separation-of-duty violation: an actor holding both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyViolation {
    /// The actor id holding both capabilities.
    pub actor: String,
    /// The granting edges that make up the finding, as `<kind> <capability
    /// id>` strings (e.g. `permits capability:app.actor#App.RaiseRefund`),
    /// sorted and deduplicated.
    pub evidence: Vec<String>,
}

/// One capability pair's separation-of-duty answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeparationOfDutyResult {
    /// The application the pair lives in.
    pub application: String,
    /// The two capability ids, sorted.
    pub pair: Vec<String>,
    /// Whether the pair is declared by a `never_both` constraint (`excludes`
    /// edge). `--pair` answers questions about undeclared pairs too.
    pub declared: bool,
    /// The holders of each side, in pair order.
    pub holders: Vec<CapabilityHolders>,
    /// The actors holding both sides, sorted by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<PolicyViolation>,
    /// Actors whose authority is partly opaque (an `opaqueGrant` edge), so
    /// the pair cannot be judged for them. Disjoint from `violations`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_actors: Vec<String>,
}

impl Finding for SeparationOfDutyResult {
    fn is_finding(&self) -> bool {
        !self.violations.is_empty()
    }
}

/// `separation-of-duty [--pair A B] [--actor a] [--session human|agent]`.
///
/// Without `--pair`, answers every declared `never_both` pair per
/// application, in `(application, pair)` order. With `--pair`, answers that
/// pair — declared or not — which is the cross-policy value: an undeclared
/// pair nobody checked is exactly the violation worth catching. A pair whose
/// capabilities live in different applications has no shared policy scope
/// and produces no result.
pub fn separation_of_duty(
    index: &GraphIndex<'_>,
    options: &SeparationOfDutyOptions<'_>,
) -> Result<AnalyticsEnvelope<SeparationOfDutyResult>, AnalyticsError> {
    let mut envelope = envelope("separationOfDuty");
    let actor = options
        .actor
        .map(|reference| resolve_actor(index, reference))
        .transpose()?
        .map(|node| node.id.clone());
    envelope.actor = actor.clone();
    envelope.session = options.session;

    let pairs = match options.pair {
        Some((left, right)) => {
            let left = resolve_capability(index, left)?;
            let right = resolve_capability(index, right)?;
            let (first, second) = sorted_pair(&left.id, &right.id);
            envelope.pair = Some(vec![first.clone(), second.clone()]);
            if left.application != right.application {
                // No shared policy scope: nothing to answer in.
                Vec::new()
            } else {
                let declared = declared_pair(index, &first, &second);
                vec![(left.application.clone(), first, second, declared)]
            }
        }
        None => declared_pairs(index),
    };

    for (application, first, second, declared) in pairs {
        if let Some(result) = separation_of_duty_record(
            index,
            &application,
            first,
            second,
            declared,
            actor.as_deref(),
            options.session,
        ) {
            envelope.results.push(result);
        }
    }
    Ok(envelope)
}

/// The sorted pair of two capability ids.
fn sorted_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    }
}

/// Whether an `excludes` edge joins the two capabilities (either direction:
/// the emitter writes both, a hand-written artifact may write one).
fn declared_pair(index: &GraphIndex<'_>, first: &str, second: &str) -> bool {
    let joins = |from: &str, to: &str| {
        index
            .edges_from(from)
            .iter()
            .any(|edge| edge.kind == kinds::EXCLUDES && edge.to == to)
    };
    joins(first, second) || joins(second, first)
}

/// Every declared `never_both` pair, per application, deduplicated (the
/// artifact emits both directions) and sorted by `(application, first,
/// second)`.
fn declared_pairs(index: &GraphIndex<'_>) -> Vec<(String, String, String, bool)> {
    let mut pairs: BTreeSet<(String, String, String)> = BTreeSet::new();
    for edge in &index.model().edges {
        if edge.kind != kinds::EXCLUDES {
            continue;
        }
        let (Some(from), Some(to)) = (index.node(&edge.from), index.node(&edge.to)) else {
            continue;
        };
        if from.kind != kinds::CAPABILITY
            || to.kind != kinds::CAPABILITY
            || from.application != to.application
        {
            continue;
        }
        let (first, second) = sorted_pair(&from.id, &to.id);
        pairs.insert((from.application.clone(), first, second));
    }
    pairs
        .into_iter()
        .map(|(application, first, second)| (application, first, second, true))
        .collect()
}

fn separation_of_duty_record(
    index: &GraphIndex<'_>,
    application: &str,
    first: String,
    second: String,
    declared: bool,
    actor: Option<&str>,
    session: Option<SessionKind>,
) -> Option<SeparationOfDutyResult> {
    let actors: Vec<&GraphNode> = actors_in(index, application)
        .into_iter()
        .filter(|candidate| actor.is_none_or(|actor| candidate.id == actor))
        .collect();
    // A filtered actor in another application leaves the pair unanswered.
    if actor.is_some() && actors.is_empty() {
        return None;
    }

    let mut holders: Vec<CapabilityHolders> = Vec::with_capacity(2);
    let mut violations = Vec::new();
    let mut unknown_actors = Vec::new();
    for capability in [&first, &second] {
        let held: Vec<String> = actors
            .iter()
            .filter(|candidate| open(index, &candidate.id, session).contains(capability))
            .map(|candidate| candidate.id.clone())
            .collect();
        holders.push(CapabilityHolders {
            capability: capability.clone(),
            actors: held,
        });
    }
    for candidate in &actors {
        let authority = open(index, &candidate.id, session);
        if authority.contains(&first) && authority.contains(&second) {
            let mut evidence = grant_evidence(index, &candidate.id, &first, session);
            evidence.extend(grant_evidence(index, &candidate.id, &second, session));
            evidence.sort();
            evidence.dedup();
            violations.push(PolicyViolation {
                actor: candidate.id.clone(),
                evidence,
            });
        } else if unknown(index, &candidate.id) {
            unknown_actors.push(candidate.id.clone());
        }
    }
    Some(SeparationOfDutyResult {
        application: application.to_string(),
        pair: vec![first, second],
        declared,
        holders,
        violations,
        unknown_actors,
    })
}

/// The `<kind> <capability id>` evidence strings for one actor's authority
/// over one capability: every lineage member's granting edge that applies.
fn grant_evidence(
    index: &GraphIndex<'_>,
    actor: &str,
    capability: &str,
    session: Option<SessionKind>,
) -> Vec<String> {
    let mut evidence = Vec::new();
    for member in lineage(index, actor) {
        for edge in index.edges_from(&member) {
            if edge.to != capability
                || !applies_to(edge, session)
                || !same_application(index, actor, &edge.to)
            {
                continue;
            }
            let granting = match edge.kind.as_str() {
                kinds::PERMITS => edge.effect != Some(GrantEffect::Forbid),
                kinds::DELEGATES => edge.effect == Some(GrantEffect::Permit),
                _ => false,
            };
            if granting {
                evidence.push(format!("{} {}", edge.kind, capability));
            }
        }
    }
    evidence.sort();
    evidence.dedup();
    evidence
}

/// The actor nodes of one application, sorted by id.
fn actors_in<'a>(index: &GraphIndex<'a>, application: &str) -> Vec<&'a GraphNode> {
    index
        .nodes()
        .into_iter()
        .filter(|node| node.kind == kinds::ACTOR && node.application == application)
        .collect()
}

/// The actor nodes with an `opaqueGrant` edge, sorted by id.
fn opaque_actors_in<'a>(index: &GraphIndex<'a>, application: &str) -> Vec<&'a GraphNode> {
    actors_in(index, application)
        .into_iter()
        .filter(|actor| unknown(index, &actor.id))
        .collect()
}

// ---------------------------------------------------------------------------
// 2. monotonicity
// ---------------------------------------------------------------------------

/// The `monotonicity` options.
#[derive(Debug, Clone, Copy, Default)]
pub struct MonotonicityOptions<'a> {
    /// Restrict the answer to the `extends` edge whose child is this actor.
    pub actor: Option<&'a str>,
}

/// One `extends` edge's monotonicity answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonotonicityResult {
    /// The application the edge lives in.
    pub application: String,
    /// The extending actor.
    pub child: String,
    /// The parent actor.
    pub parent: String,
    /// `open(parent) \ open(child)` — the capabilities the parent may
    /// exercise and the child may not, sorted by id. A `forbid` on the child
    /// is the realistic gap ("Manager extends Agent but cannot do what Agent
    /// can").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// Whether the child may do everything the parent may (`missing` empty).
    /// Under `unknown` this is the best-effort comparison, not a definite
    /// answer.
    pub dominates: bool,
    /// Whether the child's authority is a **strict** superset of the
    /// parent's (it gained authority beyond what it inherited). Best-effort
    /// under `unknown`, like [`Self::dominates`].
    pub strict: bool,
    /// Whether either side carries an `opaqueGrant` edge: the comparison is
    /// then incomplete, so it is never a definite finding and never a
    /// definite domination.
    pub unknown: bool,
}

impl Finding for MonotonicityResult {
    fn is_finding(&self) -> bool {
        !self.dominates && !self.unknown
    }
}

/// `monotonicity [--actor a]`: for every actor `extends` edge
/// `child → parent`, compare the two open sets.
///
/// Results follow the artifact's `extends`-edge order (the artifact is sorted
/// by `(from, kind, to, …)`), so the answer is deterministic without
/// re-sorting. Class inheritance edges are not policy edges and never
/// appear.
pub fn monotonicity(
    index: &GraphIndex<'_>,
    options: &MonotonicityOptions<'_>,
) -> Result<AnalyticsEnvelope<MonotonicityResult>, AnalyticsError> {
    let mut envelope = envelope("monotonicity");
    let actor = options
        .actor
        .map(|reference| resolve_actor(index, reference))
        .transpose()?
        .map(|node| node.id.clone());
    envelope.actor = actor.clone();

    for edge in &index.model().edges {
        if edge.kind != kinds::EXTENDS {
            continue;
        }
        let (Some(child), Some(parent)) = (index.node(&edge.from), index.node(&edge.to)) else {
            continue;
        };
        if child.kind != kinds::ACTOR
            || parent.kind != kinds::ACTOR
            || child.application != parent.application
        {
            continue;
        }
        if actor.as_deref().is_some_and(|actor| actor != child.id) {
            continue;
        }
        let open_child = open(index, &child.id, None);
        let open_parent = open(index, &parent.id, None);
        let missing: Vec<String> = open_parent.difference(&open_child).cloned().collect();
        let strict = open_child != open_parent && open_child.is_superset(&open_parent);
        envelope.results.push(MonotonicityResult {
            application: child.application.clone(),
            child: child.id.clone(),
            parent: parent.id.clone(),
            missing,
            dominates: open_parent.is_subset(&open_child),
            strict,
            unknown: unknown(index, &child.id) || unknown(index, &parent.id),
        });
    }
    Ok(envelope)
}

// ---------------------------------------------------------------------------
// 3. dead-capabilities
// ---------------------------------------------------------------------------

/// The four outcomes a declared capability can have — the compiler's
/// vocabulary (`latent` is its word for declared-but-ungranted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityStatus {
    /// Some actor holds it in `open(A)`.
    Alive,
    /// Permitted somewhere, but in no `open(A)`: every holder also forbids
    /// it.
    Dead,
    /// No permit edge at all — declared but ungranted. Deliberately not
    /// reported as dead.
    Latent,
    /// No open holder, and some actor of the application carries an opaque
    /// grant: the artifact cannot decide.
    Unknown,
}

/// One declared capability's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadCapabilityResult {
    /// The application the capability is declared in.
    pub application: String,
    /// The capability id.
    pub capability: String,
    /// The capability name.
    pub name: String,
    /// The outcome.
    pub status: CapabilityStatus,
    /// The actors holding it in `open(A)`, sorted (`alive`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holders: Vec<String>,
    /// The actors that permit and also forbid it, sorted (`dead`): the
    /// evidence that every holder blocks it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// The actors carrying an opaque grant, sorted (`unknown`): why the
    /// artifact cannot decide.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_actors: Vec<String>,
}

impl Finding for DeadCapabilityResult {
    fn is_finding(&self) -> bool {
        self.status == CapabilityStatus::Dead
    }
}

/// `dead-capabilities [--session human|agent]`: each declared capability's
/// outcome, sorted by capability id.
///
/// The precedence is the conservative one — a definite answer is only given
/// when the artifact supports it:
///
/// 1. `alive` — an open holder exists.
/// 2. `unknown` — no open holder and some actor of the application carries
///    an opaque grant (nothing can be concluded).
/// 3. `dead` — no open holder, at least one permit edge, and every
///    permitting actor also forbids it.
/// 4. `latent` — no permit edge at all.
pub fn dead_capabilities(
    index: &GraphIndex<'_>,
    session: Option<SessionKind>,
) -> AnalyticsEnvelope<DeadCapabilityResult> {
    let mut envelope = envelope("deadCapabilities");
    envelope.session = session;

    let mut capabilities: Vec<&GraphNode> = index
        .nodes()
        .into_iter()
        .filter(|node| node.kind == kinds::CAPABILITY)
        .collect();
    capabilities.sort_by(|left, right| left.id.cmp(&right.id));

    let mut opaque_by_application: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for capability in &capabilities {
        let opaque = opaque_by_application
            .entry(capability.application.clone())
            .or_insert_with(|| {
                opaque_actors_in(index, &capability.application)
                    .into_iter()
                    .map(|actor| actor.id.clone())
                    .collect()
            });
        let actors = actors_in(index, &capability.application);
        let mut holders = Vec::new();
        let mut permitting = Vec::new();
        let mut blocked_by = Vec::new();
        for actor in &actors {
            let permitted = permits(index, &actor.id, session);
            if open(index, &actor.id, session).contains(&capability.id) {
                holders.push(actor.id.clone());
            }
            if permitted.contains(&capability.id) {
                permitting.push(actor.id.clone());
                if forbids(index, &actor.id, session).contains(&capability.id) {
                    blocked_by.push(actor.id.clone());
                }
            }
        }
        let (status, holders, blocked_by, unknown_actors) = if !holders.is_empty() {
            (CapabilityStatus::Alive, holders, Vec::new(), Vec::new())
        } else if !opaque.is_empty() {
            (
                CapabilityStatus::Unknown,
                Vec::new(),
                Vec::new(),
                opaque.clone(),
            )
        } else if !permitting.is_empty() {
            (CapabilityStatus::Dead, Vec::new(), blocked_by, Vec::new())
        } else {
            (CapabilityStatus::Latent, Vec::new(), Vec::new(), Vec::new())
        };
        envelope.results.push(DeadCapabilityResult {
            application: capability.application.clone(),
            capability: capability.id.clone(),
            name: capability.name.clone(),
            status,
            holders,
            blocked_by,
            unknown_actors,
        });
    }
    envelope
}

// ---------------------------------------------------------------------------
// 4. reachability
// ---------------------------------------------------------------------------

/// The `reachability` options: exactly one start, and a target.
#[derive(Debug, Clone, Copy)]
pub struct ReachabilityOptions<'a> {
    /// Start from the classes of the capabilities this actor holds (their
    /// `on` targets).
    pub from_actor: Option<&'a str>,
    /// Start from this capability's class.
    pub from_capability: Option<&'a str>,
    /// The target: a class, or a feature of a class (its owning class is the
    /// destination).
    pub to: &'a str,
}

/// One hop of a witness path: the node reached, the edge kind that reached
/// it, and the object-graph feature that edge carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReachabilityStep {
    /// The node id.
    pub node: String,
    /// The edge kind walked into this node (absent at the start).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    /// The `feature` (or `field`) attribute of that edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
}

/// One reachability answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReachabilityResult {
    /// The application the walk ran in.
    pub application: String,
    /// Whether the target class was reached.
    pub reachable: bool,
    /// The starting capabilities, sorted by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starts: Vec<String>,
    /// The starting classes (the starts' `on` targets), sorted by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub start_classes: Vec<String>,
    /// The destination class (a feature target resolves to its owning class).
    pub target: String,
    /// The shortest witness path from a start class to the target, each hop
    /// naming the edge kind and feature it walked. Empty when unreachable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<ReachabilityStep>,
}

impl Finding for ReachabilityResult {
    fn is_finding(&self) -> bool {
        !self.reachable
    }
}

/// `reachability --from-actor A --to Class.feature` (or
/// `--from-capability C`): can the authority reach the target's class
/// through the domain object graph?
///
/// Starts are the classes on which a capability the actor holds (`open`,
/// unguarded by session kind) is `on`; with `--from-capability` the starts
/// are that capability's classes. The walk follows `contains`, `refers`, and
/// `opposite` **undirected**: they describe one object graph, and the
/// question "can this authority reach that data" is about connectivity, not
/// navigation direction. Containment leaves the container and reaches the
/// contained class; a reference reaches its target class; `opposite` links
/// the two ends of a bidirectional association. A `file`, `service`, or any
/// other semantic edge is never walked.
///
/// A feature target reaches when its owning class is reachable (a feature's
/// owning class is only knowable from the feature's qualified name — the
/// artifact has no `declares` edges). BFS visits neighbours in lexicographic
/// id order and stops at the first discovery, so the witness path is the
/// shortest one with the lexicographically smallest chain.
pub fn reachability(
    index: &GraphIndex<'_>,
    options: &ReachabilityOptions<'_>,
) -> Result<AnalyticsEnvelope<ReachabilityResult>, AnalyticsError> {
    let mut envelope = envelope("reachability");
    envelope.to = Some(index.resolve(options.to)?.id.clone());

    let (starts, start_classes) = match (options.from_actor, options.from_capability) {
        (Some(_), Some(_)) | (None, None) => return Err(AnalyticsError::MissingStart),
        (Some(reference), None) => {
            let actor = resolve_actor(index, reference)?;
            envelope.from = Some(actor.id.clone());
            let starts: Vec<String> = open(index, &actor.id, None).into_iter().collect();
            let classes = capability_classes(index, &starts);
            (starts, classes)
        }
        (None, Some(reference)) => {
            let capability = resolve_capability(index, reference)?;
            envelope.from_capability = Some(capability.id.clone());
            let starts = vec![capability.id.clone()];
            let classes = capability_classes(index, &starts);
            (starts, classes)
        }
    };

    let target_node = index.resolve(options.to)?;
    let target = target_class(index, target_node)?;
    let application = target.application.clone();

    // Starts outside the destination's application cannot reach it: the
    // object graph never crosses an application.
    let mut start_classes: Vec<String> = start_classes
        .into_iter()
        .filter(|class| {
            index
                .node(class)
                .is_some_and(|node| node.application == application)
        })
        .collect();
    start_classes.sort();
    start_classes.dedup();

    let mut seen: BTreeMap<String, Discovery> = BTreeMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for class in &start_classes {
        seen.insert(class.clone(), (None, None, None));
        queue.push_back(class.clone());
    }
    while let Some(current) = queue.pop_front() {
        if current == target.id {
            break;
        }
        for (neighbour, edge) in object_neighbours(index, &current, &application) {
            if seen.contains_key(&neighbour) {
                continue;
            }
            let feature = edge
                .attributes
                .get("feature")
                .or_else(|| edge.attributes.get("field"))
                .cloned();
            seen.insert(
                neighbour.clone(),
                (Some(current.clone()), Some(edge.kind.clone()), feature),
            );
            queue.push_back(neighbour);
        }
    }

    let path = if seen.contains_key(&target.id) {
        witness_path(&target.id, &seen)
    } else {
        Vec::new()
    };
    envelope.results.push(ReachabilityResult {
        application,
        reachable: !path.is_empty(),
        starts,
        start_classes,
        target: target.id.clone(),
        path,
    });
    Ok(envelope)
}

/// The classes the given capabilities are `on`, sorted and deduplicated.
fn capability_classes(index: &GraphIndex<'_>, capabilities: &[String]) -> Vec<String> {
    let mut classes: BTreeSet<String> = BTreeSet::new();
    for capability in capabilities {
        for edge in index.edges_from(capability) {
            if edge.kind == kinds::ON {
                if let Some(node) = index.node(&edge.to) {
                    if node.kind == kinds::CLASS {
                        classes.insert(node.id.clone());
                    }
                }
            }
        }
    }
    classes.into_iter().collect()
}

/// The destination class of a `--to` reference: a class node as itself, a
/// feature node as its owning class.
fn target_class<'a>(
    index: &GraphIndex<'a>,
    node: &'a GraphNode,
) -> Result<&'a GraphNode, AnalyticsError> {
    match node.kind.as_str() {
        kinds::CLASS => Ok(node),
        kinds::FEATURE => {
            let Some((class_name, _)) = node.name.rsplit_once('.') else {
                return Err(AnalyticsError::NotAClass {
                    reference: node.id.clone(),
                    kind: node.kind.clone(),
                });
            };
            let candidates: Vec<&GraphNode> = index
                .nodes_by_kind_name(kinds::CLASS, class_name)
                .iter()
                .copied()
                .filter(|candidate| candidate.application == node.application)
                .collect();
            match candidates.first() {
                Some(class) => Ok(class),
                None => Err(AnalyticsError::NotAClass {
                    reference: node.id.clone(),
                    kind: "feature with no owning class".to_string(),
                }),
            }
        }
        _ => Err(AnalyticsError::NotAClass {
            reference: node.id.clone(),
            kind: node.kind.clone(),
        }),
    }
}

/// Whether an edge kind belongs to the domain object graph.
fn object_kind(kind: &str) -> bool {
    matches!(kind, kinds::CONTAINS | kinds::REFERS | kinds::OPPOSITE)
}

/// The undirected object-graph neighbours of one node inside one
/// application, as `(neighbour id, edge)` pairs sorted by neighbour id then
/// edge kind, first pair per neighbour.
fn object_neighbours<'a>(
    index: &GraphIndex<'a>,
    id: &str,
    application: &str,
) -> Vec<(String, &'a GraphEdge)> {
    let in_application = |other: &str| {
        index
            .node(other)
            .is_some_and(|node| node.application == application)
    };
    let mut found: Vec<(String, &'a GraphEdge)> = Vec::new();
    for edge in index.edges_from(id) {
        if object_kind(&edge.kind) && in_application(&edge.to) {
            found.push((edge.to.clone(), edge));
        }
    }
    for edge in index.edges_to(id) {
        if object_kind(&edge.kind) && in_application(&edge.from) {
            found.push((edge.from.clone(), edge));
        }
    }
    found.sort_by(|(left_id, left_edge), (right_id, right_edge)| {
        (left_id, &left_edge.kind).cmp(&(right_id, &right_edge.kind))
    });
    found.dedup_by(|(left_id, _), (right_id, _)| left_id == right_id);
    found
}

/// One BFS discovery: the parent node, the edge kind walked, and the
/// object-graph feature that edge carries.
type Discovery = (Option<String>, Option<String>, Option<String>);

/// Reconstructs the witness path from a BFS discovery table.
fn witness_path(target: &str, seen: &BTreeMap<String, Discovery>) -> Vec<ReachabilityStep> {
    let mut steps: Vec<ReachabilityStep> = Vec::new();
    let mut current = Some(target.to_string());
    while let Some(id) = current {
        let Some((parent, edge, feature)) = seen.get(&id) else {
            break;
        };
        steps.push(ReachabilityStep {
            node: id,
            edge: edge.clone(),
            feature: feature.clone(),
        });
        current = parent.clone();
    }
    steps.reverse();
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_ir::graph::{GraphApplication, GraphModel, GRAPH_MODEL_FORMAT_VERSION};

    const APP: &str = "app.actor";
    const CUSTOMER: &str = "actor:app.actor#App.Customer";
    const AGENT: &str = "actor:app.actor#App.Agent";
    const MANAGER: &str = "actor:app.actor#App.Manager";
    const TICKET: &str = "class:app.mox#app.Ticket";
    const NOTE: &str = "class:app.mox#app.Note";
    const TICKET_NOTES: &str = "feature:app.mox#app.Ticket.notes";
    const TICKET_TITLE: &str = "feature:app.mox#app.Ticket.title";
    const NOTE_BODY: &str = "feature:app.mox#app.Note.body";
    const INVOICE: &str = "class:app.mox#app.Invoice";
    const INVOICE_TOTAL: &str = "feature:app.mox#app.Invoice.total";
    const READ: &str = "capability:app.actor#App.ReadTicket";
    const UPDATE: &str = "capability:app.actor#App.UpdateTicket";
    const ESCALATE: &str = "capability:app.actor#App.EscalateTicket";

    fn node(id: &str, kind: &str, name: &str) -> GraphNode {
        GraphNode::new(id, kind, name, "app.actor", APP)
    }

    fn domain_node(id: &str, kind: &str, name: &str) -> GraphNode {
        GraphNode::new(id, kind, name, "app.mox", APP)
    }

    /// A small policy + object graph: `Manager extends Agent extends
    /// Customer`; `UpdateTicket` is permitted by `Agent` and forbidden by
    /// `Manager`; `EscalateTicket` is granted to nobody; `ReadTicket` is
    /// granted with a `human` session modifier; `Customer` carries an opaque
    /// `cedar` entry in the "no opaque" variant. `Ticket` contains `Note`.
    fn model(opaque: bool) -> GraphModel {
        let file = GraphNode::new(
            "file:app.actor#app.actor",
            kinds::FILE,
            "app.actor",
            "app.actor",
            APP,
        );
        let mut nodes = vec![
            file,
            node(CUSTOMER, kinds::ACTOR, "App.Customer"),
            node(AGENT, kinds::ACTOR, "App.Agent"),
            node(MANAGER, kinds::ACTOR, "App.Manager"),
            node(READ, kinds::CAPABILITY, "App.ReadTicket"),
            node(UPDATE, kinds::CAPABILITY, "App.UpdateTicket"),
            node(ESCALATE, kinds::CAPABILITY, "App.EscalateTicket"),
            domain_node(TICKET, kinds::CLASS, "app.Ticket"),
            domain_node(NOTE, kinds::CLASS, "app.Note"),
            domain_node(TICKET_NOTES, kinds::FEATURE, "app.Ticket.notes"),
            domain_node(TICKET_TITLE, kinds::FEATURE, "app.Ticket.title"),
            domain_node(NOTE_BODY, kinds::FEATURE, "app.Note.body"),
            domain_node(INVOICE, kinds::CLASS, "app.Invoice"),
            domain_node(INVOICE_TOTAL, kinds::FEATURE, "app.Invoice.total"),
        ];
        let mut edges = vec![
            GraphEdge::new(AGENT, CUSTOMER, kinds::EXTENDS),
            GraphEdge::new(MANAGER, AGENT, kinds::EXTENDS),
            GraphEdge::new(CUSTOMER, READ, kinds::PERMITS)
                .effect(GrantEffect::Permit)
                .session_kind(SessionKind::Human),
            GraphEdge::new(AGENT, UPDATE, kinds::PERMITS).effect(GrantEffect::Permit),
            GraphEdge::new(MANAGER, UPDATE, kinds::FORBIDS).effect(GrantEffect::Forbid),
            GraphEdge::new(READ, TICKET, kinds::ON),
            GraphEdge::new(UPDATE, TICKET, kinds::ON),
            GraphEdge::new(ESCALATE, TICKET, kinds::ON),
            GraphEdge::new(TICKET, NOTE, kinds::CONTAINS).attribute("feature", "notes"),
            GraphEdge::new(TICKET_NOTES, TICKET, kinds::MEMBER_OF),
            GraphEdge::new(TICKET_TITLE, TICKET, kinds::MEMBER_OF),
            GraphEdge::new(NOTE_BODY, NOTE, kinds::MEMBER_OF),
            GraphEdge::new(INVOICE_TOTAL, INVOICE, kinds::MEMBER_OF),
        ];
        if opaque {
            let file_id = "file:app.actor#app.actor".to_string();
            edges.push(
                GraphEdge::new(MANAGER, file_id, kinds::OPAQUE_GRANT).effect(GrantEffect::Permit),
            );
        }
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        edges.sort_by(|left, right| {
            (&left.from, &left.kind, &left.to).cmp(&(&right.from, &right.kind, &right.to))
        });
        GraphModel {
            format_version: GRAPH_MODEL_FORMAT_VERSION,
            applications: vec![GraphApplication {
                id: APP.to_string(),
                files: vec!["app.actor".to_string(), "app.mox".to_string()],
                packages: Vec::new(),
            }],
            nodes,
            edges,
        }
    }

    /// `lineage` walks `extends` transitively and survives a cycle.
    #[test]
    fn lineage_is_transitive_and_cycle_safe() {
        let graph = model(false);
        let index = GraphIndex::new(&graph);
        assert_eq!(
            lineage(&index, MANAGER),
            [MANAGER.to_string(), AGENT.to_string(), CUSTOMER.to_string()]
                .into_iter()
                .collect()
        );

        // A cycle: Customer extends Manager closes the ring.
        let mut cyclic = model(false);
        cyclic
            .edges
            .push(GraphEdge::new(CUSTOMER, MANAGER, kinds::EXTENDS));
        let index = GraphIndex::new(&cyclic);
        assert_eq!(lineage(&index, MANAGER).len(), 3, "the walk terminates");
    }

    /// `permits` mirrors `lower/actors.rs`: inherited grant permits count,
    /// inbound delegation permits count for the receiver, and a `cedar` entry
    /// never contributes.
    #[test]
    fn effective_permits_mirror_the_driver() {
        let graph = model(false);
        let index = GraphIndex::new(&graph);
        assert!(permits(&index, MANAGER, None).contains(UPDATE));
        assert!(!open(&index, MANAGER, None).contains(UPDATE));
        assert!(open(&index, AGENT, None).contains(UPDATE));

        // A delegation handing `EscalateTicket` to the manager counts; a
        // forbid delegation does not.
        let mut delegated = model(false);
        delegated.edges.push(
            GraphEdge::new(MANAGER, ESCALATE, kinds::DELEGATES)
                .effect(GrantEffect::Permit)
                .attribute("fromActor", "App.Finance"),
        );
        let index = GraphIndex::new(&delegated);
        assert!(open(&index, MANAGER, None).contains(ESCALATE));

        let mut forbidden = model(false);
        forbidden.edges.push(
            GraphEdge::new(AGENT, UPDATE, kinds::DELEGATES)
                .effect(GrantEffect::Forbid)
                .attribute("fromActor", "App.Finance"),
        );
        let index = GraphIndex::new(&forbidden);
        assert!(forbids(&index, AGENT, None).contains(UPDATE));
        assert!(!open(&index, AGENT, None).contains(UPDATE));

        // With an opaque grant the actor is `unknown`, and the opaque entry
        // still contributes no permit.
        let graph = model(true);
        let index = GraphIndex::new(&graph);
        assert!(unknown(&index, MANAGER));
        assert!(!permits(&index, MANAGER, None).contains(ESCALATE));
    }

    /// An opaque `cedar` entry makes the actor unknown but never grants.
    #[test]
    fn opaque_cedar_is_unknown_never_permit() {
        let graph = model(true);
        let index = GraphIndex::new(&graph);
        assert!(unknown(&index, MANAGER));
        assert!(!unknown(&index, AGENT));
        assert!(!permits(&index, MANAGER, None).contains(ESCALATE));
    }

    /// The session filter narrows both permits and forbids: `ReadTicket` is
    /// human-only, so an agent session does not hold it.
    #[test]
    fn session_filter_narrows_permits() {
        let graph = model(false);
        let index = GraphIndex::new(&graph);
        assert!(open(&index, CUSTOMER, Some(SessionKind::Human)).contains(READ));
        assert!(!open(&index, CUSTOMER, Some(SessionKind::Agent)).contains(READ));
        assert!(open(&index, CUSTOMER, None).contains(READ));
    }

    /// `dead-capabilities` distinguishes alive, dead, latent, and unknown.
    #[test]
    fn dead_latent_alive_and_unknown_are_distinguished() {
        let graph = model(false);
        let index = GraphIndex::new(&graph);
        let answer = dead_capabilities(&index, None);
        let status = |id: &str| {
            answer
                .results
                .iter()
                .find(|result| result.capability == id)
                .map(|result| result.status)
                .expect("declared capability")
        };
        assert_eq!(status(UPDATE), CapabilityStatus::Alive);
        assert_eq!(status(ESCALATE), CapabilityStatus::Latent);
        assert!(!answer.has_findings());

        let graph = model(true);
        let index = GraphIndex::new(&graph);
        let answer = dead_capabilities(&index, None);
        let status = |id: &str| {
            answer
                .results
                .iter()
                .find(|result| result.capability == id)
                .map(|result| result.status)
                .expect("declared capability")
        };
        assert_eq!(status(ESCALATE), CapabilityStatus::Unknown);
        assert_eq!(status(UPDATE), CapabilityStatus::Alive);
    }

    /// Monotonicity reports the capability the child forbade, and the pair
    /// query catches an undeclared pair held through inheritance.
    #[test]
    fn monotonicity_and_undeclared_pairs_report_their_findings() {
        let graph = model(false);
        let index = GraphIndex::new(&graph);
        let answer = monotonicity(&index, &MonotonicityOptions::default()).expect("resolves");
        let manager = answer
            .results
            .iter()
            .find(|result| result.child == MANAGER)
            .expect("the manager extends the agent");
        assert_eq!(manager.missing, [UPDATE.to_string()]);
        assert!(!manager.dominates);
        assert!(manager.is_finding());

        let answer = separation_of_duty(
            &index,
            &SeparationOfDutyOptions {
                pair: Some((UPDATE, READ)),
                actor: Some(AGENT),
                session: None,
            },
        )
        .expect("resolves");
        let result = &answer.results[0];
        assert!(!result.declared);
        assert_eq!(result.violations.len(), 1, "the agent holds both");
        assert_eq!(result.violations[0].actor, AGENT);
        assert!(result.violations[0]
            .evidence
            .contains(&format!("permits {UPDATE}")));
    }

    /// Reachability walks containment and refuses an unrelated class.
    #[test]
    fn reachability_walks_the_object_graph() {
        let graph = model(false);
        let index = GraphIndex::new(&graph);
        let options = ReachabilityOptions {
            from_actor: Some(AGENT),
            from_capability: None,
            to: "Note.body",
        };
        let answer = reachability(&index, &options).expect("resolves");
        let result = &answer.results[0];
        assert!(result.reachable);
        assert_eq!(result.path[0].node, TICKET);
        assert_eq!(result.path[1].node, NOTE);
        assert_eq!(result.path[1].edge.as_deref(), Some(kinds::CONTAINS));
        assert_eq!(result.path[1].feature.as_deref(), Some("notes"));

        // A feature of the start class is reachable at depth 0.
        let options = ReachabilityOptions {
            from_actor: Some(AGENT),
            from_capability: None,
            to: "Ticket.title",
        };
        let answer = reachability(&index, &options).expect("resolves");
        assert!(answer.results[0].reachable);
        assert_eq!(answer.results[0].path.len(), 1);

        // An unrelated rooted class is not reachable.
        let options = ReachabilityOptions {
            from_actor: Some(AGENT),
            from_capability: None,
            to: "Invoice.total",
        };
        let answer = reachability(&index, &options).expect("resolves");
        assert!(!answer.results[0].reachable);
        assert!(answer.results[0].is_finding());
    }

    /// The envelope is the query envelope's shape: versioned, camelCase, and
    /// with absent echoes omitted.
    #[test]
    fn envelopes_are_versioned_and_omit_absent_echoes() {
        let graph = model(false);
        let index = GraphIndex::new(&graph);
        let answer = dead_capabilities(&index, Some(SessionKind::Agent));
        let json = answer.to_json(false).expect("serialize");
        assert!(json.contains("\"formatVersion\":1"));
        assert!(json.contains("\"query\":\"deadCapabilities\""));
        assert!(json.contains("\"session\":\"agent\""));
        assert!(!json.contains("format_version"));
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert!(parsed.get("actor").is_none());
        assert!(parsed.get("pair").is_none());
    }
}

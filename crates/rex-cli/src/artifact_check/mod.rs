//! Wire-format validation for serialized rexlang artifacts — the portable
//! test kit behind `rexlang artifact check`.
//!
//! Third-party backends consume [`rex_ir::Model`] / [`rex_ir::ActorModel`]
//! / [`rex_ir::ddd::DddModel`] / [`rex_ir::events::EventModel`]
//! / [`rex_ir::deploy::DeployModel`] / [`rex_ir::usecases::UseCaseModel`]
//! (or read the serialized artifact directly) and emit canonical instance
//! JSON. This module checks the *format* invariants of those documents
//! without needing the originating model:
//!
//! * the root shape matches a known artifact kind (IR artifact, standalone
//!   actor-policy artifact, DDD design artifact, event-contract artifact,
//!   deployment artifact, use-case artifact, decision-model artifact,
//!   case-management artifact, interaction-flow artifact, or canonical
//!   instance),
//! * `formatVersion` is present and supported — a higher version is rejected
//!   with an error naming it, mirroring the readers' version gate
//!   ([`rex_ir::IrError::UnsupportedFormatVersion`]),
//! * keys on known objects are camelCase (snake_case spellings of known
//!   structural keys are called out with the correct spelling),
//! * instance documents are sane: `$id`s are well-formed (`<class>/<n>`)
//!   and unique, `$type` is present, and every `$ref` resolves within the
//!   document.
//!
//! The wire-format contract itself is normative in the `rex-ir` crate docs;
//! `docs/BACKENDS.md` is the backend-author guide.
//!
//! Classification is **shape-sniffing**, not self-declared: no artifact kind
//! except instances carries a root `$type` discriminator. The precedence is
//! documented on [`check`], and
//! [`tests::classification_table_over_all_artifact_kinds`] pins it for every
//! kind — including the two shapes an [`rex_ir::ifml::IfmlModel`] artifact
//! collides with (the empty form and the module-only form; see the table).

use serde_json::Value;

mod instances;
mod keys;
mod kinds;

use instances::check_instance;
use kinds::{
    check_actor_model, check_cmmn_model, check_ddd_model, check_deploy_model, check_dmn_model,
    check_event_model, check_ifml_model, check_ir_model, check_usecase_model,
};

/// The instance `formatVersion` this tool reads
/// ([`rex_runtime::INSTANCE_FORMAT_VERSION`]).
pub const SUPPORTED_INSTANCE_FORMAT_VERSION: u32 = rex_runtime::INSTANCE_FORMAT_VERSION;

/// The artifact kinds a document can be classified as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// A domain IR artifact (`rex_ir::Model`): root carries `packages`.
    IrModel,
    /// A standalone actor-policy artifact (`rex_ir::ActorModel`): root
    /// carries `blocks` (omitted when empty).
    ActorModel,
    /// A standalone DDD design artifact (`rex_ir::ddd::DddModel`): root
    /// carries an `application` object or a `modules` array (both omitted
    /// when empty).
    DddModel,
    /// A standalone event-contract artifact
    /// (`rex_ir::events::EventModel`): root carries an `events`,
    /// `channels`, or `subscriptions` array (all omitted when empty).
    EventModel,
    /// A standalone deployment artifact (`rex_ir::deploy::DeployModel`):
    /// root carries an `applications`, `profiles`, or `deployments` array
    /// (all omitted when empty).
    DeployModel,
    /// A standalone use-case artifact
    /// (`rex_ir::usecases::UseCaseModel`): root carries a `useCases`
    /// array (omitted when empty).
    UseCaseModel,
    /// A standalone decision-model artifact
    /// (`rex_ir::dmn::DmnModel`): root carries a `decisions` array
    /// (omitted when empty).
    DmnModel,
    /// A standalone case-management artifact (`rex_ir::cmmn::CmmnModel`):
    /// root carries a `cases` array (omitted when empty).
    CmmnModel,
    /// A standalone interaction-flow artifact
    /// (`rex_ir::ifml::IfmlModel`): root carries a `views`, `domains`,
    /// `actions`, `commands`, or `imports` entry (all omitted when
    /// empty). The `modules` key alone is shape-ambiguous with the DDD
    /// design artifact — a module-only IFML model classifies as a
    /// DddModel, the documented limitation.
    IfmlModel,
    /// A canonical instance document: root `$type` is `rex.instance`.
    Instance,
}

/// Validates one artifact document, returning its kind or every violation
/// found. `path` prefixes each error line (the file the document came from).
pub fn check_str(path: &str, text: &str) -> Result<ArtifactKind, Vec<String>> {
    match check(text) {
        Ok(kind) => Ok(kind),
        Err(errors) => Err(errors
            .into_iter()
            .map(|error| format!("{path}: {error}"))
            .collect()),
    }
}

/// Validates one artifact document without path prefixing.
///
/// Classification precedence — the first matching rule wins:
///
/// 1. **`$type` (instances).** A root `$type` selects the instance path:
///    `rex.instance` validates as an [`ArtifactKind::Instance`]; any other
///    string is rejected (IR/actor/DDD/event artifacts carry no root
///    `$type`).
/// 2. **`packages`/`rexVersion` → IR artifact.** A [`rex_ir::Model`] always
///    emits `packages` (and stamps `rexVersion` provenance), so either key
///    claims the document for [`ArtifactKind::IrModel`].
/// 3. **`decisions` → decision-model artifact.** A `formatVersion`-bearing
///    root that carries that array validates as an
///    [`ArtifactKind::DmnModel`]. This check runs *before* the IFML shape:
///    a decision-model root commonly also carries a `views` array (DMN's
///    decision views), which the IFML rule would claim — but `decisions`
///    exists on no other artifact root, so a decision-shaped document must
///    classify as a DmnModel (the mirror image of the views-before-DDD
///    rule below).
/// 4. **`views`/`domains`/`actions`/`commands`/`imports` → interaction
///    flow.** A `formatVersion`-bearing root carrying any of those entries
///    validates as an [`ArtifactKind::IfmlModel`]. This check runs *before*
///    the DDD shape so an IFML model that also carries `modules` classifies
///    as an IfmlModel; the keys exist on no other artifact, and a
///    module-only IFML model falls through to rule 5 — the documented
///    shape collision.
/// 5. **`application`/`modules` → DDD design.** A `formatVersion`-bearing
///    root without `blocks` that carries an `application` object or a
///    `modules` array validates as a [`ArtifactKind::DddModel`]. This check
///    runs *before* the actor fallback: those keys exist on no other
///    artifact, so design-shaped documents must be reported as designs even
///    when malformed (the tightened-fallback regression).
/// 6. **`events`/`channels`/`subscriptions` → event contract.** A
///    `formatVersion`-bearing root that carries any of those arrays
///    validates as an [`ArtifactKind::EventModel`]. Like the DDD rule this
///    runs *before* the actor fallback (the keys exist on no other
///    artifact), and after the DDD rule only for documentation symmetry —
///    the key sets are disjoint, so rules 5 and 6 can never both match.
/// 7. **`applications`/`profiles`/`deployments` → deployment artifact.** A
///    `formatVersion`-bearing root that carries any of those arrays
///    validates as an [`ArtifactKind::DeployModel`]. The keys are disjoint
///    from every earlier rule (`application` on a DDD design is a singular
///    object, not these plural arrays), so the ordering is documentation
///    symmetry again.
/// 8. **`useCases` → use-case artifact.** A `formatVersion`-bearing root
///    that carries that array validates as an
///    [`ArtifactKind::UseCaseModel`]. The key exists on no other artifact,
///    so the ordering is documentation symmetry once more.
/// 9. **`cases` → case artifact.** A `formatVersion`-bearing root that
///    carries that array validates as an [`ArtifactKind::CmmnModel`] —
///    same disjointness, same symmetry.
/// 10. **`blocks` or exactly `{"formatVersion": N}` → actor artifact.** The
///     canonical block-less [`rex_ir::ActorModel`] is exactly the version
///     marker, and the marker alone can only be an ActorModel (a Model always
///     emits `packages`; a DddModel always carries the design keys), so the
///     minimal-empty document falls here.
/// 11. **Anything else** is rejected as `unrecognized artifact`.
///
/// Known limitations, same root cause: the minimal-empty artifact is exactly
/// `{"formatVersion":1}` for several kinds, and shape-sniffing cannot tell
/// them apart.
///
/// - [`rex_ir::events::EventModel`] is empty-form-collided: an empty event
///   artifact serializes as exactly `{"formatVersion":1}` and classifies as
///   an ActorModel via rule 10 (only a non-empty one reaches rule 6).
/// - [`rex_ir::deploy::DeployModel`] is empty-form-collided the same way
///   (rule 10; only a non-empty one reaches rule 7).
/// - [`rex_ir::usecases::UseCaseModel`] is empty-form-collided the same way
///   (rule 10; only a non-empty one reaches rule 8).
/// - [`rex_ir::dmn::DmnModel`] is empty-form-collided the same way (rule
///   10; only a non-empty one reaches rule 3).
/// - [`rex_ir::cmmn::CmmnModel`] is empty-form-collided the same way (rule
///   10; only a non-empty one reaches rule 9's `cases` check first).
/// - [`rex_ir::ifml::IfmlModel`] is empty-form-collided (rule 10; only a
///   non-empty one reaches rule 4) and module-only-collided: an IFML model
///   carrying *only* `modules` shape-matches the DDD design keys and
///   classifies as a DddModel (rule 5). Any other IFML key claims it.
///
/// A root `$type` discriminator on all artifact kinds would resolve both, at
/// the cost of a wire-format change (golden churn, version bump) — see the
/// classification table test for the pinned behavior.
pub fn check(text: &str) -> Result<ArtifactKind, Vec<String>> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => return Err(vec![format!("not valid JSON: {error}")]),
    };
    let Some(root) = value.as_object() else {
        return Err(vec!["artifact root must be a JSON object".to_string()]);
    };
    if let Some(tag) = root.get("$type") {
        return match tag.as_str() {
            Some(t) if t == rex_runtime::INSTANCE_TYPE => check_instance(root),
            Some(other) => Err(vec![format!(
                "unknown root $type {other:?}: instance documents declare $type {:?}; \
                 IR and actor artifacts carry no root $type",
                rex_runtime::INSTANCE_TYPE
            )]),
            None => Err(vec![format!("root $type must be a string, found {tag}")]),
        };
    }
    if root.contains_key("packages") || root.contains_key("rexVersion") {
        return check_ir_model(root);
    }
    // A decision-model artifact carries a `decisions` array — a key no
    // other artifact root has — so its check runs before the IFML shape: a
    // decision-model root commonly also carries a `views` array (DMN's
    // decision views), which the IFML rule below would claim. The empty
    // decision artifact serializes as exactly `{"formatVersion":1}` and
    // falls through to the actor fallback like the other empty forms.
    if root.get("formatVersion").is_some_and(Value::is_u64)
        && root.get("decisions").is_some_and(Value::is_array)
    {
        return check_dmn_model(root);
    }
    // An IFML interaction-flow artifact carries a `views`, `domains`,
    // `actions`, `commands`, or `imports` entry — keys no other artifact
    // has — so its check runs before the DDD shape: an IFML model that
    // also carries `modules` must classify as an IfmlModel, never the
    // DddModel design shape. A module-only IFML model (no other IFML key)
    // falls through to the DDD rule — the documented shape collision,
    // shared root cause with the empty-form limitation below.
    if root.get("formatVersion").is_some_and(Value::is_u64)
        && (root.get("views").is_some_and(Value::is_array)
            || root.get("domains").is_some_and(Value::is_array)
            || root.get("actions").is_some_and(Value::is_array)
            || root.get("commands").is_some_and(Value::is_array)
            || root.get("imports").is_some_and(Value::is_array))
    {
        return check_ifml_model(root);
    }
    // A DDD design artifact carries an `application` object or a `modules`
    // array — keys no other artifact has — so its check runs before the
    // actor fallback: anything with the design shape must validate as a
    // DddModel, never as a (block-less) ActorModel.
    if root.get("formatVersion").is_some_and(Value::is_u64)
        && !root.contains_key("blocks")
        && (root.get("application").is_some_and(Value::is_object)
            || root.get("modules").is_some_and(Value::is_array))
    {
        return check_ddd_model(root);
    }
    // An event-contract artifact carries an `events`, `channels`, or
    // `subscriptions` array — keys no other artifact has — so its check
    // runs before the actor fallback: anything with the event shape must
    // validate as an EventModel, never as a (block-less) ActorModel. The
    // empty event artifact serializes as exactly `{"formatVersion":1}` and
    // therefore falls through to the actor fallback (the documented
    // empty-form limitation, shared with IFML and deployments).
    if root.get("formatVersion").is_some_and(Value::is_u64)
        && (root.get("events").is_some_and(Value::is_array)
            || root.get("channels").is_some_and(Value::is_array)
            || root.get("subscriptions").is_some_and(Value::is_array))
    {
        return check_event_model(root);
    }
    // A deployment artifact carries an `applications`, `profiles`, or
    // `deployments` array — keys no other artifact has (`application` on a
    // DDD design is a singular *object*, not these plural arrays) — so its
    // check runs before the actor fallback. The empty deployment artifact
    // serializes as exactly `{"formatVersion":1}` and falls through to the
    // actor fallback like the event/IFML empty forms.
    if root.get("formatVersion").is_some_and(Value::is_u64)
        && (root.get("applications").is_some_and(Value::is_array)
            || root.get("profiles").is_some_and(Value::is_array)
            || root.get("deployments").is_some_and(Value::is_array))
    {
        return check_deploy_model(root);
    }
    // A use-case artifact carries a `useCases` array — a key no other
    // artifact has — so its check runs before the actor fallback. The empty
    // use-case artifact serializes as exactly `{"formatVersion":1}` and
    // falls through to the actor fallback like the event/deployment/IFML
    // empty forms.
    if root.get("formatVersion").is_some_and(Value::is_u64)
        && root.get("useCases").is_some_and(Value::is_array)
    {
        return check_usecase_model(root);
    }
    // A case artifact carries a `cases` array — likewise a key no other
    // artifact has. The empty case artifact serializes as exactly
    // `{"formatVersion":1}` and falls through to the actor fallback like
    // the other empty forms.
    if root.get("formatVersion").is_some_and(Value::is_u64)
        && root.get("cases").is_some_and(Value::is_array)
    {
        return check_cmmn_model(root);
    }
    // No `packages`/`rexVersion`, no decision-model shape, no IFML shape,
    // no DDD design shape, no
    // event-contract shape, no deployment shape, no use-case shape: the
    // canonical block-less ActorModel is exactly `{"formatVersion": N}` —
    // a Model always emits `packages`, a DddModel always carries the
    // design keys, an EventModel always carries one of the event arrays, a
    // DeployModel always carries one of the deployment arrays, a
    // UseCaseModel always carries `useCases`, a DmnModel always
    // carries `decisions`, an IfmlModel always carries one of the IFML
    // keys. Anything else has no known root shape.
    if root.contains_key("blocks") || (root.len() == 1 && root.contains_key("formatVersion")) {
        return check_actor_model(root);
    }
    Err(vec![format!(
        "unrecognized artifact: no known root shape (IR artifacts carry \"packages\", \
         actor artifacts \"blocks\", interaction-flow artifacts \
         \"views\"/\"domains\"/\"actions\"/\"commands\", \
         DDD design artifacts \"application\"/\"modules\", \
         event artifacts \"events\"/\"channels\"/\"subscriptions\", \
         deployment artifacts \"applications\"/\"profiles\"/\"deployments\", \
         use-case artifacts \"useCases\", \
         decision artifacts \"decisions\", \
         instances declare $type {:?})",
        rex_runtime::INSTANCE_TYPE
    )])
}

#[cfg(test)]
mod tests;

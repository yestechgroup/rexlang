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

use std::collections::BTreeSet;

use serde_json::Value;

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

/// Validates one artifact root against its kind's spec: the version gate
/// mirrors the root's `from_json` constant, each named key must be an
/// array of named objects, any extra per-kind shape checks run, and the
/// whole document is scanned for snake_case keys.
fn check_kind(
    root: &serde_json::Map<String, Value>,
    kind: ArtifactKind,
    version: u32,
    label: &str,
    keys: &[&str],
    extra: fn(&serde_json::Map<String, Value>, &mut Vec<String>),
) -> Result<ArtifactKind, Vec<String>> {
    let mut errors = Vec::new();
    if let Some(error) = format_version_error(root, version, label) {
        errors.push(error);
    }
    extra(root, &mut errors);
    for key in keys {
        if let Some(array) = root.get(*key) {
            check_named_array(array, key, &mut errors);
        }
    }
    scan_snake_case_keys(&Value::Object(root.clone()), &mut errors);
    finish(kind, errors)
}

/// The common shape has no checks beyond the version gate and the named
/// arrays.
fn no_extra(_: &serde_json::Map<String, Value>, _: &mut Vec<String>) {}

/// The DDD design root's `application` object must carry a non-empty
/// string `name`.
fn check_ddd_application(root: &serde_json::Map<String, Value>, errors: &mut Vec<String>) {
    let Some(application) = root.get("application") else {
        return;
    };
    match application.as_object() {
        None => errors.push(format!(
            "\"application\" must be an object, found {application}"
        )),
        Some(object) => match object.get("name") {
            None => errors.push("the DDD design \"application\" is missing \"name\"".to_string()),
            Some(name) if name.as_str().is_none_or(str::is_empty) => errors.push(format!(
                "the DDD design \"application\" \"name\" must be a non-empty \
                 string, found {name}"
            )),
            _ => {}
        },
    }
}

/// Validates an interaction-flow artifact: the version gate mirrors
/// [`rex_ir::ifml::IfmlModel::from_json`], and `views`, `domains`,
/// `actions`, `modules`, `actors`, and `commands` must be arrays of named
/// objects (`imports` is an array of strings — excluded from the named
/// check).
fn check_ifml_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::IfmlModel,
        rex_ir::ifml::IFML_MODEL_FORMAT_VERSION,
        "interaction-flow artifact",
        &[
            "views", "domains", "actions", "modules", "actors", "commands",
        ],
        no_extra,
    )
}

fn check_ir_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::IrModel,
        rex_ir::FORMAT_VERSION,
        "IR artifact",
        &["packages"],
        no_extra,
    )
}

fn check_actor_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::ActorModel,
        rex_ir::ACTOR_MODEL_FORMAT_VERSION,
        "actor artifact",
        &["blocks"],
        no_extra,
    )
}

/// Validates a DDD design artifact: the version gate mirrors
/// [`rex_ir::ddd::DddModel::from_json`], the `application` object must
/// carry a non-empty string `name`, and `modules` must be an array of
/// named objects.
fn check_ddd_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::DddModel,
        rex_ir::ddd::DDD_MODEL_FORMAT_VERSION,
        "DDD design artifact",
        &["modules"],
        check_ddd_application,
    )
}

/// Validates an event-contract artifact: the version gate mirrors
/// [`rex_ir::events::EventModel::from_json`], and `events`, `channels`,
/// and `subscriptions` must be arrays of named objects.
fn check_event_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::EventModel,
        rex_ir::events::EVENT_MODEL_FORMAT_VERSION,
        "event artifact",
        &["events", "channels", "subscriptions"],
        no_extra,
    )
}

/// Validates a deployment artifact: the version gate mirrors
/// [`rex_ir::deploy::DeployModel::from_json`], and `applications`,
/// `profiles`, and `deployments` must be arrays of named objects.
fn check_deploy_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::DeployModel,
        rex_ir::deploy::DEPLOY_MODEL_FORMAT_VERSION,
        "deployment artifact",
        &["applications", "profiles", "deployments"],
        no_extra,
    )
}

/// Validates a use-case artifact: the version gate mirrors
/// [`rex_ir::usecases::UseCaseModel::from_json`], and `useCases` must be
/// an array of named objects.
fn check_usecase_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::UseCaseModel,
        rex_ir::usecases::USE_CASE_MODEL_FORMAT_VERSION,
        "use-case artifact",
        &["useCases"],
        no_extra,
    )
}

/// Validates a case artifact: the version gate mirrors
/// [`rex_ir::cmmn::CmmnModel::from_json`], and `cases` must be an array
/// of named objects.
fn check_cmmn_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::CmmnModel,
        rex_ir::cmmn::CMMN_MODEL_FORMAT_VERSION,
        "case artifact",
        &["cases"],
        no_extra,
    )
}

/// Validates a decision-model artifact: the version gate mirrors
/// [`rex_ir::dmn::DmnModel::from_json`], and `decisions` must be an array
/// of named objects.
fn check_dmn_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    check_kind(
        root,
        ArtifactKind::DmnModel,
        rex_ir::dmn::DMN_MODEL_FORMAT_VERSION,
        "decision artifact",
        &["decisions"],
        no_extra,
    )
}

fn check_instance(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    let mut errors = Vec::new();
    if let Some(error) = format_version_error(root, SUPPORTED_INSTANCE_FORMAT_VERSION, "instance") {
        errors.push(error);
    }
    match root.get("model") {
        None => errors.push("instance is missing \"model\"".to_string()),
        Some(model) if model.as_str().is_none_or(str::is_empty) => errors.push(format!(
            "instance \"model\" must be a non-empty string, found {model}"
        )),
        _ => {}
    }
    let Some(objects) = root.get("objects") else {
        errors.push("instance is missing \"objects\"".to_string());
        return finish(ArtifactKind::Instance, errors);
    };
    let Some(objects) = objects.as_array() else {
        errors.push(format!(
            "instance \"objects\" must be an array, found {objects}"
        ));
        return finish(ArtifactKind::Instance, errors);
    };
    for (index, object) in objects.iter().enumerate() {
        if !object.is_object() {
            errors.push(format!(
                "objects[{index}] must be an object, found {object}"
            ));
        }
    }
    let mut ids = BTreeSet::new();
    register_objects(objects, &mut ids, &mut errors);
    resolve_refs(objects, &ids, &mut errors);
    finish(ArtifactKind::Instance, errors)
}

fn finish(kind: ArtifactKind, errors: Vec<String>) -> Result<ArtifactKind, Vec<String>> {
    if errors.is_empty() {
        Ok(kind)
    } else {
        Err(errors)
    }
}

/// The readers' version gate: only the exact supported version passes; a
/// different version (including a higher one) is rejected with an error
/// naming both versions, like [`rex_ir::IrError::UnsupportedFormatVersion`].
fn format_version_error(
    root: &serde_json::Map<String, Value>,
    supported: u32,
    what: &str,
) -> Option<String> {
    match root.get("formatVersion") {
        None => Some(format!(
            "missing {what} formatVersion: this tool reads formatVersion {supported}"
        )),
        Some(found) => match found.as_u64() {
            Some(found) if found == u64::from(supported) => None,
            Some(found) => Some(format!(
                "unsupported {what} formatVersion {found}: this tool reads formatVersion \
                 {supported} — regenerate the artifact with a matching rexlang version"
            )),
            None => Some(format!(
                "{what} formatVersion must be an unsigned integer, found {found}"
            )),
        },
    }
}

/// `packages`/`blocks` sanity: an array of objects each carrying a string
/// `name`.
fn check_named_array(value: &Value, key: &str, errors: &mut Vec<String>) {
    let Some(items) = value.as_array() else {
        errors.push(format!("{key:?} must be an array, found {value}"));
        return;
    };
    for (index, item) in items.iter().enumerate() {
        match item.as_object() {
            None => errors.push(format!("{key}[{index}] must be an object, found {item}")),
            Some(object) => match object.get("name") {
                None => errors.push(format!("{key}[{index}] is missing \"name\"")),
                Some(name) if name.as_str().is_none_or(str::is_empty) => errors.push(format!(
                    "{key}[{index}] \"name\" must be a non-empty string, found {name}"
                )),
                _ => {}
            },
        }
    }
}

/// Every structural (non model-defined) key of the IR, actor, DDD design,
/// event, deployment, and use-case wire formats, from the `rex-ir` serde
/// shapes — the camelCase spellings a snake_case key is compared against.
/// Sorted for binary search.
const STRUCTURAL_KEYS: &[&str] = &[
    "actors",
    "anchor",
    "annotations",
    "applicabilityRules",
    "application",
    "authorizedRoles",
    "autoComplete",
    "base",
    "blocks",
    "bodies",
    "builtin",
    "cache",
    "capabilities",
    "capability",
    "caseFile",
    "cases",
    "cedar",
    "channels",
    "children",
    "class",
    "classes",
    "componentType",
    "composition",
    "constraints",
    "consumer",
    "context",
    "convert",
    "create",
    "datatypes",
    "default",
    "definitionRef",
    "delegations",
    "dependencies",
    "description",
    "designs",
    "detailFields",
    "details",
    "effect",
    "end",
    "entries",
    "enums",
    "eventType",
    "events",
    "extends",
    "facets",
    "features",
    "finite",
    "first",
    "flags",
    "format",
    "formatVersion",
    "from",
    "functions",
    "grants",
    "hierarchyField",
    "id",
    "ifPart",
    "inputParams",
    "interfaces",
    "isDefault",
    "isDerived",
    "isId",
    "isLandmark",
    "isModal",
    "isReadOnly",
    "isXor",
    "key",
    "kind",
    "label",
    "labelField",
    "labelFields",
    "literals",
    "lower",
    "manualActivation",
    "manualActivationRule",
    "maxDepth",
    "maxLength",
    "maximum",
    "message",
    "messages",
    "minLength",
    "minimum",
    "moduleUses",
    "modules",
    "multiplicity",
    "name",
    "neverBoth",
    "nonPersistent",
    "objects",
    "obligations",
    "occurrences",
    "onParts",
    "operation",
    "operations",
    "opposite",
    "optimisticLocking",
    "outputParams",
    "packages",
    "params",
    "participant",
    "pattern",
    "payload",
    "planItems",
    "planningMode",
    "planningTables",
    "platform",
    "processRef",
    "publishes",
    "purposes",
    "realizes",
    "repetitionRule",
    "repository",
    "requiredRole",
    "requiredRule",
    "requires",
    "returnMultiplicity",
    "returnType",
    "rexVersion",
    "schemaName",
    "sentryRef",
    "services",
    "source",
    "steps",
    "stereotype",
    "subcommands",
    "subject",
    "subscriptions",
    "successions",
    "target",
    "targetBindings",
    "taskType",
    "then",
    "to",
    "type",
    "typeName",
    "unique",
    "upper",
    "useCases",
    "value",
    "valueFields",
    "version",
    "vocabularies",
    "when",
];

/// Reports every key whose spelling is snake_case but whose camelCase form
/// is a known structural key — the readers ignore unknown fields (forward
/// compatibility), so such artifacts load but silently drop the value.
///
/// Runs on IR, actor, DDD design, event, deployment, and use-case artifacts
/// only: their keys are entirely structural. Instance objects' feature keys
/// are model-defined names, so instance documents are exempt.
fn scan_snake_case_keys(value: &Value, errors: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key.contains('_') {
                    let camel_case = camel_case(key);
                    if STRUCTURAL_KEYS.binary_search(&camel_case.as_str()).is_ok() {
                        errors.push(format!(
                            "key {key:?} is not wire format: the camelCase key is \
                             {camel_case:?}"
                        ));
                    }
                }
                scan_snake_case_keys(child, errors);
            }
        }
        Value::Array(items) => {
            for item in items {
                scan_snake_case_keys(item, errors);
            }
        }
        _ => {}
    }
}

/// `format_version` -> `formatVersion` (first segment lowercase, the rest
/// capitalized).
fn camel_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for (index, segment) in key
        .split('_')
        .filter(|segment| !segment.is_empty())
        .enumerate()
    {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            if index == 0 {
                out.extend(first.to_lowercase());
            } else {
                out.extend(first.to_uppercase());
            }
            out.push_str(chars.as_str());
        }
    }
    out
}

/// Pass one over the instance object tree: register every `$id` (checking
/// well-formedness and uniqueness) and flag malformed objects.
fn register_objects(items: &[Value], ids: &mut BTreeSet<String>, errors: &mut Vec<String>) {
    for item in items {
        register_value(item, ids, errors);
    }
}

fn register_value(value: &Value, ids: &mut BTreeSet<String>, errors: &mut Vec<String>) {
    match value {
        Value::Array(items) => register_objects(items, ids, errors),
        Value::Object(map) => {
            let id = map.get("$id");
            if let Some(id) = id {
                match id.as_str() {
                    Some(id) => {
                        if let Some(problem) = malformed_id(id) {
                            errors.push(problem);
                        } else if !ids.insert(id.to_string()) {
                            errors.push(format!(
                                "duplicate $id {id:?}: object ids must be unique within \
                                 the document"
                            ));
                        }
                    }
                    None => errors.push(format!("object $id must be a string, found {id}")),
                }
                match map.get("$type") {
                    None => errors.push(format!("object with $id {id:?} is missing $type")),
                    Some(type_) if type_.as_str().is_none() => {
                        errors.push(format!("object $type must be a string, found {type_}"))
                    }
                    _ => {}
                }
            } else if let Some(type_) = map.get("$type") {
                errors.push(format!(
                    "object with $type {type_:?} has neither $id nor $ref: instance \
                     objects are either registered ($id) or links ($ref)"
                ));
            }
            if let Some(reference) = map.get("$ref") {
                let extra: Vec<&String> = map.keys().filter(|key| **key != "$ref").collect();
                if !extra.is_empty() {
                    errors.push(format!(
                        "a $ref object must contain only \"$ref\" (found {reference}), \
                         with keys {}",
                        quoted_list(&extra),
                    ));
                }
            }
            for child in map.values() {
                register_value(child, ids, errors);
            }
        }
        _ => {}
    }
}

/// Pass two: every `$ref` must resolve to an `$id` collected anywhere in the
/// document (refs may point forward or up through containment).
fn resolve_refs(items: &[Value], ids: &BTreeSet<String>, errors: &mut Vec<String>) {
    for item in items {
        resolve_value(item, ids, errors);
    }
}

fn resolve_value(value: &Value, ids: &BTreeSet<String>, errors: &mut Vec<String>) {
    match value {
        Value::Array(items) => resolve_refs(items, ids, errors),
        Value::Object(map) => {
            if let Some(target) = map
                .get("$ref")
                .and_then(Value::as_str)
                .filter(|target| !ids.contains(*target))
            {
                errors.push(format!(
                    "unresolved $ref {target:?}: no object in this document has that $id"
                ));
            }
            for child in map.values() {
                resolve_value(child, ids, errors);
            }
        }
        _ => {}
    }
}

/// Canonical object ids are `<class>/<ordinal>` — the shape the generated
/// loaders and sort keys rely on (`rsplit_once('/')` + numeric ordinal).
fn malformed_id(id: &str) -> Option<String> {
    let Some((prefix, ordinal)) = id.rsplit_once('/') else {
        return Some(format!(
            "object $id {id:?} is malformed: canonical ids are \"<class>/<ordinal>\""
        ));
    };
    if prefix.is_empty() {
        return Some(format!(
            "object $id {id:?} is malformed: the class prefix before '/' is empty"
        ));
    }
    if ordinal.is_empty() || ordinal.parse::<u64>().is_err() {
        return Some(format!(
            "object $id {id:?} is malformed: the ordinal after '/' must be a \
             non-negative integer"
        ));
    }
    None
}

fn quoted_list(keys: &[&String]) -> String {
    let mut out = String::new();
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(key);
        out.push('"');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One row of the classification table: a document and the outcome
    /// `check` must produce. `Err` rows carry a fragment every error line
    /// must contain (the kind whose check claimed the document).
    struct ClassificationCase {
        name: &'static str,
        json: String,
        expected: Result<ArtifactKind, &'static str>,
    }

    #[test]
    fn classification_table_over_all_artifact_kinds() {
        // Round-trip rows serialize the real struct kinds (rex_ir types),
        // so the table pins the classification of actual emitter output,
        // not hand-written approximations.
        let block_less_actor = rex_ir::ActorModel::new();
        let block_less_actor_json =
            serde_json::to_string(&block_less_actor).expect("serialize ActorModel");
        let empty_ifml = rex_ir::ifml::IfmlModel::default();
        let empty_ifml_json = serde_json::to_string(&empty_ifml).expect("serialize IfmlModel");
        let empty_event = rex_ir::events::EventModel::new();
        let empty_event_json = serde_json::to_string(&empty_event).expect("serialize EventModel");
        let empty_usecase = rex_ir::usecases::UseCaseModel::new();
        let empty_usecase_json =
            serde_json::to_string(&empty_usecase).expect("serialize UseCaseModel");
        let empty_dmn = rex_ir::dmn::DmnModel::new();
        let empty_dmn_json = serde_json::to_string(&empty_dmn).expect("serialize DmnModel");

        let cases = vec![
            ClassificationCase {
                name: "domain IR artifact (serialized rex_ir::Model)",
                json: serde_json::to_string(&rex_ir::Model::new()).expect("serialize Model"),
                expected: Ok(ArtifactKind::IrModel),
            },
            ClassificationCase {
                name: "actor-policy artifact with a block (serialized ActorModel)",
                json: serde_json::to_string(
                    &rex_ir::ActorModel::new().block(rex_ir::ActorsDef::new("Support")),
                )
                .expect("serialize ActorModel"),
                expected: Ok(ArtifactKind::ActorModel),
            },
            ClassificationCase {
                name: "minimal-empty artifact is the block-less ActorModel",
                json: block_less_actor_json.clone(),
                expected: Ok(ArtifactKind::ActorModel),
            },
            ClassificationCase {
                name: "DDD design artifact (serialized DddModel)",
                json: serde_json::to_string(
                    &rex_ir::ddd::DddModel::new()
                        .application(rex_ir::ddd::Application::new("Library"))
                        .module(rex_ir::ddd::Module::new("core")),
                )
                .expect("serialize DddModel"),
                expected: Ok(ArtifactKind::DddModel),
            },
            ClassificationCase {
                name: "event-contract artifact (serialized EventModel)",
                json: serde_json::to_string(
                    &rex_ir::events::EventModel::new()
                        .event(rex_ir::events::EventDef::new("Ping"))
                        .channel(rex_ir::events::ChannelDef::new("pings").publishes("Ping"))
                        .subscription(
                            rex_ir::events::SubscriptionDef::new("watch")
                                .event("Ping")
                                .consumer("watcher"),
                        ),
                )
                .expect("serialize EventModel"),
                expected: Ok(ArtifactKind::EventModel),
            },
            ClassificationCase {
                name: "deployment artifact (serialized DeployModel)",
                json: serde_json::to_string(
                    &rex_ir::deploy::DeployModel::new()
                        .application(rex_ir::deploy::DeployApplication::new("Svc").component(
                            rex_ir::deploy::DeployComponent::new(
                                "db",
                                rex_ir::deploy::ComponentKind::Database,
                            ),
                        ))
                        .profile(rex_ir::deploy::DeployProfile::new(
                            "edge",
                            rex_ir::deploy::DeployTarget::CloudflareWorkers,
                        ))
                        .deployment(rex_ir::deploy::DeployDeployment::new("prod", "Svc", "edge")),
                )
                .expect("serialize DeployModel"),
                expected: Ok(ArtifactKind::DeployModel),
            },
            ClassificationCase {
                name: "use-case artifact (serialized UseCaseModel)",
                json: serde_json::to_string(&{
                    let mut model = rex_ir::usecases::UseCaseModel::new();
                    model.use_cases.push(rex_ir::usecases::UseCaseDef {
                        name: "HandOver".to_string(),
                        description: None,
                        subject: None,
                        actors: vec![rex_ir::usecases::Participant {
                            name: "Courier".to_string(),
                            type_name: None,
                            occurrences: Vec::new(),
                            steps: Vec::new(),
                        }],
                        messages: Vec::new(),
                        successions: Vec::new(),
                        realizes: Vec::new(),
                    });
                    model
                })
                .expect("serialize UseCaseModel"),
                expected: Ok(ArtifactKind::UseCaseModel),
            },
            // Misclassification guard: a malformed use-case-shaped document
            // must be reported by the UseCaseModel check, never fall through
            // to the actor fallback.
            ClassificationCase {
                name: "malformed use-case shape is reported as a UseCaseModel, not an ActorModel",
                json: r#"{"formatVersion": 1, "useCases": [{"oops": true}]}"#.to_string(),
                expected: Err("useCases[0]"),
            },
            // Misclassification guard: a malformed deployment-shaped
            // document must be reported by the DeployModel check, never
            // fall through to the actor fallback.
            ClassificationCase {
                name: "malformed deployment shape is reported as a DeployModel, not an ActorModel",
                json: r#"{"formatVersion": 1, "applications": [{"oops": true}]}"#.to_string(),
                expected: Err("applications[0]"),
            },
            // Misclassification guard: a malformed event-shaped document
            // must be reported by the EventModel check, never fall through
            // to the actor fallback (whose "missing formatVersion"-style
            // errors would misattribute the artifact kind).
            ClassificationCase {
                name: "malformed event shape is reported as an EventModel, not an ActorModel",
                json: r#"{"formatVersion": 1, "events": [{"oops": true}]}"#.to_string(),
                expected: Err("events[0]"),
            },
            ClassificationCase {
                name: "decision-model artifact (serialized DmnModel)",
                json: serde_json::to_string(&{
                    let mut model = rex_ir::dmn::DmnModel::new();
                    model.decisions.push(rex_ir::dmn::Decision {
                        name: "Assess".to_string(),
                        description: None,
                        outputs: Vec::new(),
                        logic: Some(rex_ir::dmn::DecisionLogic::Expression {
                            text: "1".to_string(),
                        }),
                    });
                    model
                })
                .expect("serialize DmnModel"),
                expected: Ok(ArtifactKind::DmnModel),
            },
            // Precedence case: a decision model's root commonly carries a
            // `views` array too — the IFML rule's key — but the unique
            // `decisions` key claims the document first.
            ClassificationCase {
                name: "a decisions-and-views DmnModel classifies as a DmnModel, \
                       never the IFML shape",
                json: serde_json::to_string(&{
                    let mut model = rex_ir::dmn::DmnModel::new();
                    model.decisions.push(rex_ir::dmn::Decision {
                        name: "Assess".to_string(),
                        description: None,
                        outputs: Vec::new(),
                        logic: None,
                    });
                    model.views.push(rex_ir::dmn::DecisionView {
                        name: "overview".to_string(),
                        description: None,
                        show: vec!["Assess".to_string()],
                        direction: None,
                    });
                    model
                })
                .expect("serialize DmnModel"),
                expected: Ok(ArtifactKind::DmnModel),
            },
            // Misclassification guard: a malformed decision-shaped document
            // must be reported by the DmnModel check, never fall through
            // to the actor fallback.
            ClassificationCase {
                name: "malformed decision shape is reported as a DmnModel, not an ActorModel",
                json: r#"{"formatVersion": 1, "decisions": [{"oops": true}]}"#.to_string(),
                expected: Err("decisions[0]"),
            },
            ClassificationCase {
                name: "unknown root shape is unrecognized",
                json: r#"{"formatVersion": 1, "nobodyKnowsMe": true}"#.to_string(),
                expected: Err("unrecognized artifact"),
            },
            // Empty-form limitation, shared with the empty IfmlModel: an
            // empty EventModel serializes as exactly `{"formatVersion":1}`,
            // so shape-sniffing classifies it as the block-less ActorModel.
            // Pinned here so any future $type discriminator must update the
            // three kinds consciously.
            ClassificationCase {
                name: "empty event artifact serializes as the same minimal-empty bytes",
                json: {
                    assert_eq!(
                        empty_event_json, block_less_actor_json,
                        "an empty EventModel and an empty ActorModel are \
                         byte-identical on the wire"
                    );
                    empty_event_json.clone()
                },
                expected: Ok(ArtifactKind::ActorModel),
            },
            // Empty-form limitation, shared with the empty EventModel and
            // IfmlModel: an empty DeployModel serializes as exactly
            // `{"formatVersion":1}`, so shape-sniffing classifies it as the
            // block-less ActorModel. Pinned here so any future $type
            // discriminator must update the four kinds consciously.
            ClassificationCase {
                name: "empty deployment artifact serializes as the same minimal-empty bytes",
                json: {
                    let empty_deploy_json =
                        serde_json::to_string(&rex_ir::deploy::DeployModel::new())
                            .expect("serialize DeployModel");
                    assert_eq!(
                        empty_deploy_json, block_less_actor_json,
                        "an empty DeployModel and an empty ActorModel are \
                         byte-identical on the wire"
                    );
                    empty_deploy_json
                },
                expected: Ok(ArtifactKind::ActorModel),
            },
            // Empty-form limitation, shared with the empty EventModel,
            // DeployModel, and IfmlModel: an empty UseCaseModel serializes
            // as exactly `{"formatVersion":1}`, so shape-sniffing classifies
            // it as the block-less ActorModel. Pinned here so any future
            // $type discriminator must update the five kinds consciously.
            ClassificationCase {
                name: "empty use-case artifact serializes as the same minimal-empty bytes",
                json: {
                    assert_eq!(
                        empty_usecase_json, block_less_actor_json,
                        "an empty UseCaseModel and an empty ActorModel are \
                         byte-identical on the wire"
                    );
                    empty_usecase_json
                },
                expected: Ok(ArtifactKind::ActorModel),
            },
            // Empty-form limitation, shared with the empty EventModel,
            // DeployModel, UseCaseModel, and IfmlModel: an empty DmnModel
            // serializes as exactly `{"formatVersion":1}`, so shape-sniffing
            // classifies it as the block-less ActorModel. Pinned here so any
            // future $type discriminator must update the six kinds
            // consciously.
            ClassificationCase {
                name: "empty decision artifact serializes as the same minimal-empty bytes",
                json: {
                    assert_eq!(
                        empty_dmn_json, block_less_actor_json,
                        "an empty DmnModel and an empty ActorModel are \
                         byte-identical on the wire"
                    );
                    empty_dmn_json
                },
                expected: Ok(ArtifactKind::ActorModel),
            },
            // IfmlModel is a classified kind; the two unfixable shape
            // collisions are pinned here so any future $type discriminator
            // must update them consciously.
            ClassificationCase {
                name: "empty IFML artifact serializes as the same minimal-empty bytes",
                json: {
                    assert_eq!(
                        empty_ifml_json, block_less_actor_json,
                        "an empty IfmlModel and an empty ActorModel are \
                         byte-identical on the wire"
                    );
                    empty_ifml_json.clone()
                },
                expected: Ok(ArtifactKind::ActorModel),
            },
            ClassificationCase {
                name: "module-only IFML artifact shape-matches the DDD design keys \
                       (the documented limitation)",
                json: serde_json::to_string(&rex_ir::ifml::IfmlModel {
                    modules: vec![rex_ir::ifml::ModuleDeclaration {
                        name: "Pagination".to_string(),
                        input_params: Vec::new(),
                        output_params: Vec::new(),
                        properties: Vec::new(),
                        containers: Vec::new(),
                        components: Vec::new(),
                        events: Vec::new(),
                        module_uses: Vec::new(),
                    }],
                    ..rex_ir::ifml::IfmlModel::default()
                })
                .expect("serialize IfmlModel"),
                expected: Ok(ArtifactKind::DddModel),
            },
            ClassificationCase {
                name: "view-bearing IFML artifact classifies as an IfmlModel",
                json: serde_json::to_string(&rex_ir::ifml::IfmlModel {
                    views: vec![rex_ir::ifml::ViewDeclaration {
                        name: "Home".to_string(),
                        label: None,
                        is_landmark: false,
                        is_xor: false,
                        is_modal: false,
                        params: Vec::new(),
                        properties: Vec::new(),
                        containers: Vec::new(),
                        components: Vec::new(),
                        events: Vec::new(),
                        module_uses: Vec::new(),
                        roles: Vec::new(),
                        requires: Vec::new(),
                        condition: None,
                        position: None,
                    }],
                    ..rex_ir::ifml::IfmlModel::default()
                })
                .expect("serialize IfmlModel"),
                expected: Ok(ArtifactKind::IfmlModel),
            },
            ClassificationCase {
                name: "a view-and-module IFML artifact classifies as an IfmlModel, \
                       never the DDD shape",
                json: serde_json::to_string(&rex_ir::ifml::IfmlModel {
                    views: vec![rex_ir::ifml::ViewDeclaration {
                        name: "Home".to_string(),
                        label: None,
                        is_landmark: false,
                        is_xor: false,
                        is_modal: false,
                        params: Vec::new(),
                        properties: Vec::new(),
                        containers: Vec::new(),
                        components: Vec::new(),
                        events: Vec::new(),
                        module_uses: Vec::new(),
                        roles: Vec::new(),
                        requires: Vec::new(),
                        condition: None,
                        position: None,
                    }],
                    modules: vec![rex_ir::ifml::ModuleDeclaration {
                        name: "Pagination".to_string(),
                        input_params: Vec::new(),
                        output_params: Vec::new(),
                        properties: Vec::new(),
                        containers: Vec::new(),
                        components: Vec::new(),
                        events: Vec::new(),
                        module_uses: Vec::new(),
                    }],
                    ..rex_ir::ifml::IfmlModel::default()
                })
                .expect("serialize IfmlModel"),
                expected: Ok(ArtifactKind::IfmlModel),
            },
            // Misclassification guard: a malformed IFML-shaped document is
            // reported by the IfmlModel check, never the actor fallback.
            ClassificationCase {
                name: "malformed IFML shape is reported as an IfmlModel, not an ActorModel",
                json: r#"{"formatVersion": 1, "views": [{"oops": true}]}"#.to_string(),
                expected: Err("views[0]"),
            },
        ];

        for case in &cases {
            match (&case.expected, check(&case.json)) {
                (Ok(kind), Ok(found)) => assert_eq!(
                    found, *kind,
                    "{}: classified as {found:?}, expected {kind:?}",
                    case.name
                ),
                (Err(fragment), Err(errors)) => assert!(
                    errors.iter().any(|error| error.contains(fragment)),
                    "{}: errors {errors:?} must mention {fragment:?}",
                    case.name
                ),
                (expected, found) => {
                    panic!("{}: expected {expected:?}, found {found:?}", case.name)
                }
            }
        }
    }

    #[test]
    fn bare_format_version_classifies_as_block_less_actor_model() {
        // A Model always emits `packages`; the only artifact whose canonical
        // form is just the version marker is the empty ActorModel.
        assert_eq!(
            check(r#"{"formatVersion": 1}"#),
            Ok(ArtifactKind::ActorModel)
        );
    }

    #[test]
    fn packages_key_classifies_as_ir_model() {
        assert_eq!(
            check(r#"{"formatVersion": 1, "rexVersion": "0.1.0", "packages": []}"#),
            Ok(ArtifactKind::IrModel)
        );
    }

    #[test]
    fn format_version_gate_names_both_versions() {
        let errors = check(r#"{"formatVersion": 2, "packages": []}"#).unwrap_err();
        assert!(
            errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn snake_case_keys_are_reported_with_the_camel_case_spelling() {
        let errors =
            check(r#"{"formatVersion": 1, "format_version": 1, "packages": []}"#).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("format_version") && error.contains("formatVersion")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn instance_ids_must_be_well_formed_unique_and_resolved() {
        let errors = check(
            r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [
                    {"$id": "book/0", "$type": "Book", "other": {"$ref": "writer/0"}}
                ]
            }"#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("unresolved $ref \"writer/0\"")),
            "errors: {errors:?}"
        );

        let errors = check(
            r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [
                    {"$id": "book/0", "$type": "Book"},
                    {"$id": "book/0", "$type": "Book"}
                ]
            }"#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("duplicate $id \"book/0\"")),
            "errors: {errors:?}"
        );

        let errors = check(
            r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [{"$id": "book", "$type": "Book"}]
            }"#,
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("malformed")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn unknown_root_type_is_rejected() {
        let errors = check(r#"{"$type": "something.else", "formatVersion": 1}"#).unwrap_err();
        assert!(
            errors[0].contains("something.else") && errors[0].contains("rex.instance"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn application_or_modules_keys_classify_as_ddd_model() {
        assert_eq!(
            check(r#"{"formatVersion": 1, "application": {"name": "Library"}, "modules": []}"#),
            Ok(ArtifactKind::DddModel)
        );
        assert_eq!(
            check(r#"{"formatVersion": 1, "application": {"name": "Library"}}"#),
            Ok(ArtifactKind::DddModel)
        );
        assert_eq!(
            check(r#"{"formatVersion": 1, "modules": [{"name": "core"}]}"#),
            Ok(ArtifactKind::DddModel)
        );
    }

    #[test]
    fn a_ddd_artifact_does_not_fall_through_to_the_actor_fallback() {
        // Regression for the fallback order: before the DddModel check
        // existed, the actor fallback claimed any versioned document
        // without `packages`/`$type` — a malformed DDD design was reported
        // as a (block-less) ActorModel missing its formatVersion.
        let errors = check(
            r#"{"formatVersion": 1, "application": {"name": "X"}, "modules": [{"oops": true}]}"#,
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("modules[0]")),
            "must validate as a DddModel, not an ActorModel: {errors:?}"
        );
    }

    #[test]
    fn the_dddd_golden_artifact_classifies_as_ddd_model() {
        // The real emitter output (`rexlang ir` on the canonical fixture):
        // it carries no `packages`/`blocks`/`$type`, so only the DddModel
        // check may claim it.
        let golden = include_str!("../../../tests/conformance/ddd/library.ddd.json");
        assert_eq!(check(golden), Ok(ArtifactKind::DddModel));
    }

    #[test]
    fn the_ifml_golden_artifact_classifies_as_ifml_model() {
        // The real emitter output (`rexlang ir` on the canonical fixture):
        // view-bearing, so the IFML rule claims it before the DDD shape
        // (the fixture carries `modules` too — the precedence case).
        let golden = include_str!("../../../tests/conformance/ifml/app.ifml.json");
        assert_eq!(check(golden), Ok(ArtifactKind::IfmlModel));
    }

    #[test]
    fn an_ifml_artifact_does_not_fall_through_to_the_actor_fallback() {
        // Regression for the fallback order: before the IfmlModel check
        // existed, only a view-bearing artifact was rejected outright and
        // a module-bearing one was claimed by the DDD shape. Every
        // IFML-exclusive root key claims the document.
        assert_eq!(
            check(r#"{"formatVersion": 1, "views": [{"name": "Home"}]}"#),
            Ok(ArtifactKind::IfmlModel)
        );
        assert_eq!(
            check(r#"{"formatVersion": 1, "domains": [{"name": "sales"}]}"#),
            Ok(ArtifactKind::IfmlModel)
        );
        assert_eq!(
            check(r#"{"formatVersion": 1, "actions": [{"name": "Update"}]}"#),
            Ok(ArtifactKind::IfmlModel)
        );
        assert_eq!(
            check(r#"{"formatVersion": 1, "commands": [{"name": "deploy"}]}"#),
            Ok(ArtifactKind::IfmlModel)
        );
        assert_eq!(
            check(r#"{"formatVersion": 1, "imports": ["./patterns.ifml"]}"#),
            Ok(ArtifactKind::IfmlModel)
        );
    }

    #[test]
    fn ifml_format_version_gate_names_both_versions() {
        let errors = check(r#"{"formatVersion": 2, "views": [{"name": "Home"}]}"#).unwrap_err();
        assert!(
            errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn ddd_format_version_gate_names_both_versions() {
        let errors = check(r#"{"formatVersion": 2, "application": {"name": "X"}}"#).unwrap_err();
        assert!(
            errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn an_event_artifact_does_not_fall_through_to_the_actor_fallback() {
        // Regression for the fallback order: before the EventModel check
        // existed, the actor fallback claimed any versioned document
        // without `packages`/`application`/`modules`/`blocks`.
        assert_eq!(
            check(r#"{"formatVersion": 1, "events": [{"name": "Ping"}]}"#),
            Ok(ArtifactKind::EventModel)
        );
        assert_eq!(
            check(r#"{"formatVersion": 1, "channels": [{"name": "pings"}]}"#),
            Ok(ArtifactKind::EventModel)
        );
        assert_eq!(
            check(r#"{"formatVersion": 1, "subscriptions": [{"name": "w", "consumer": "c"}]}"#,),
            Ok(ArtifactKind::EventModel)
        );
    }

    #[test]
    fn event_format_version_gate_names_both_versions() {
        let errors = check(r#"{"formatVersion": 2, "events": []}"#).unwrap_err();
        assert!(
            errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn event_arrays_must_be_arrays_of_named_objects() {
        let errors = check(
            r#"{"formatVersion": 1, "events": [42], "channels": [{}], "subscriptions": [7]}"#,
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("events[0]")),
            "errors: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("channels[0] is missing \"name\"")),
            "errors: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("subscriptions[0]")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn snake_case_event_keys_are_reported_with_the_camel_case_spelling() {
        let errors =
            check(r#"{"formatVersion": 1, "events": [{"name": "Ping", "format_version": 1}]}"#)
                .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("format_version") && error.contains("formatVersion")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn ddd_application_must_be_a_named_object_and_modules_named_objects() {
        let errors = check(r#"{"formatVersion": 1, "application": "Library", "modules": [42]}"#)
            .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("\"application\" must be an object")),
            "errors: {errors:?}"
        );
        assert!(
            errors.iter().any(|error| error.contains("modules[0]")),
            "errors: {errors:?}"
        );

        let errors = check(r#"{"formatVersion": 1, "application": {"name": ""}}"#).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("non-empty string")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn unknown_root_shape_is_reported_as_unrecognized() {
        // The tightened fallback: a truly unknown v1 artifact is no longer
        // misclassified as a block-less ActorModel.
        let errors = check(r#"{"formatVersion": 1, "nobodyKnowsMe": true}"#).unwrap_err();
        assert!(
            errors[0].contains("unrecognized artifact"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn a_usecase_artifact_does_not_fall_through_to_the_actor_fallback() {
        // Regression for the fallback order: before the UseCaseModel check
        // existed, the actor fallback claimed any versioned document
        // without `packages`/`application`/`modules`/`events`/`blocks`.
        assert_eq!(
            check(r#"{"formatVersion": 1, "useCases": [{"name": "HandOver"}]}"#),
            Ok(ArtifactKind::UseCaseModel)
        );
    }

    #[test]
    fn usecase_format_version_gate_names_both_versions() {
        let errors = check(r#"{"formatVersion": 2, "useCases": []}"#).unwrap_err();
        assert!(
            errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn usecase_arrays_must_be_arrays_of_named_objects() {
        let errors = check(r#"{"formatVersion": 1, "useCases": [42]}"#).unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("useCases[0]")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn snake_case_usecase_keys_are_reported_with_the_camel_case_spelling() {
        let errors = check(
            r#"{"formatVersion": 1, "useCases": [{"name": "HandOver", "type_name": "demo.Drone"}]}"#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("type_name") && error.contains("typeName")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn the_usecase_golden_artifact_classifies_as_usecase_model() {
        // The real emitter output (`rexlang ir` on the canonical fixture):
        // it carries no `packages`/`blocks`/`$type`, so only the UseCaseModel
        // check may claim it.
        let golden = include_str!("../../../tests/conformance/usecases/delivery.uc.json");
        assert_eq!(check(golden), Ok(ArtifactKind::UseCaseModel));
    }

    #[test]
    fn a_cmmn_artifact_does_not_fall_through_to_the_actor_fallback() {
        assert_eq!(
            check(r#"{"formatVersion": 1, "cases": [{"name": "Loan"}]}"#),
            Ok(ArtifactKind::CmmnModel)
        );
    }

    #[test]
    fn cmmn_format_version_gate_names_both_versions() {
        let errors = check(r#"{"formatVersion": 2, "cases": []}"#).unwrap_err();
        assert!(
            errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn cmmn_arrays_must_be_arrays_of_named_objects() {
        let errors = check(r#"{"formatVersion": 1, "cases": [42]}"#).unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("cases[0]")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn snake_case_cmmn_keys_are_reported_with_the_camel_case_spelling() {
        let errors = check(r#"{"formatVersion": 1, "cases": [{"name": "Loan", "case_file": []}]}"#)
            .unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("caseFile")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn the_cmmn_golden_artifact_classifies_as_cmmn_model() {
        // The real emitter output (`rexlang ir` on the canonical fixture).
        let golden = include_str!("../../../tests/conformance/cmmn/lending.cmmn.json");
        assert_eq!(check(golden), Ok(ArtifactKind::CmmnModel));
    }

    #[test]
    fn the_cmmn_engine_golden_does_not_classify_as_a_rexlang_artifact() {
        // The cmmn-rs CaseDefinition JSON is a DIFFERENT wire format
        // (snake_case, no formatVersion): the checker must reject it rather
        // than guess.
        let golden = include_str!("../../../tests/conformance/cmmn/lending.case.json");
        assert!(check(golden).is_err());
    }

    #[test]
    fn a_dmn_artifact_does_not_fall_through_to_the_actor_fallback() {
        // Regression for the fallback order: before the DmnModel check
        // existed, the actor fallback claimed any versioned document
        // without `packages`/`application`/`modules`/`events`/`useCases`/
        // `cases`/`blocks`.
        assert_eq!(
            check(r#"{"formatVersion": 1, "decisions": [{"name": "Assess"}]}"#),
            Ok(ArtifactKind::DmnModel)
        );
    }

    #[test]
    fn dmn_format_version_gate_names_both_versions() {
        let errors = check(r#"{"formatVersion": 2, "decisions": []}"#).unwrap_err();
        assert!(
            errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn dmn_arrays_must_be_arrays_of_named_objects() {
        let errors = check(r#"{"formatVersion": 1, "decisions": [42]}"#).unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("decisions[0]")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn snake_case_dmn_keys_are_reported_with_the_camel_case_spelling() {
        let errors = check(
            r#"{"formatVersion": 1, "decisions": [{"name": "Assess", "format_version": 1}]}"#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("format_version") && error.contains("formatVersion")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn the_dmn_golden_artifact_classifies_as_dmn_model() {
        // The real emitter output (`rexlang ir` on the canonical fixture):
        // it carries no `packages`/`blocks`/`$type`, so only the DmnModel
        // check may claim it.
        let golden = include_str!("../../../tests/conformance/dmn/kiwisaver.dmn.json");
        assert_eq!(check(golden), Ok(ArtifactKind::DmnModel));
    }
}
